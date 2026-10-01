//! The custom summary prompts: one markdown file for each prompt in
//! `.longhand/prompts/` (R14, KTD4, KTD5).
//!
//! The file name is the prompt name, made safe for macOS and Windows. A
//! short front matter block at the top of the file holds the `id` and the
//! `name`, and the body is the prompt text:
//!
//! ```text
//! ---
//! id: 0b8f…
//! name: "1:1 notes"
//! ---
//!
//! The prompt text.
//! <!-- longhand: title instructions -->
//! The title text.
//! ```
//!
//! The marker line and the title text follow the summary text only when
//! the title text is not empty. A file with no marker line has an empty
//! title text (KTD8).
//!
//! A markdown file with no front matter is a prompt too: its id comes from
//! its file name, and its name is its file name. A sync conflict copy of a
//! prompt file keeps the id of its original. The scan gives each such
//! copy an id of its own, made only from the shared files, so every
//! computer gives each file the same id without a write. A scan never
//! writes.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::file::{
    FileRead, SettingsFile, Unavailable, check_root, longhand_folder, read_settings, write_new,
};

/// The folder in the hidden Longhand folder that holds the prompt files.
pub const PROMPTS_FOLDER: &str = "prompts";

const EXTENSION: &str = ".md";
const FRONT_MATTER_LINE: &str = "---";
const KEY_ID: &str = "id";
const KEY_NAME: &str = "name";

/// The name that a file name gets when a prompt name has no character
/// that a file name can use.
const FALLBACK_STEM: &str = "Prompt";

/// The most bytes in the part of a file name before `.md`. A free name
/// adds a number and the atomic write adds a temporary prefix and suffix,
/// and the total must stay below the 255-byte limit of macOS and Windows.
const MAX_STEM_BYTES: usize = 197;

/// The line in the body of a prompt file between the summary text and
/// the title text (KTD8). A summary text cannot hold this line.
pub const TITLE_MARKER: &str = "<!-- longhand: title instructions -->";

/// The largest prompt file that the scan reads. A summary text of
/// [`summarize::prompts::PROMPT_TEXT_LIMIT`] characters and a title text
/// of [`summarize::prompts::TITLE_TEXT_LIMIT`] characters, of four bytes
/// each, fit with the marker line and space for the front matter.
const MAX_FILE_BYTES: u64 = summarize::prompts::PROMPT_TEXT_LIMIT as u64 * 4
    + summarize::prompts::TITLE_TEXT_LIMIT as u64 * 4
    + TITLE_MARKER.len() as u64
    + 2
    + 4096;

/// The namespace of the ids that the scan makes (UUID v5). It never
/// changes: every computer and every version must make the same ids.
const ID_NAMESPACE: Uuid = Uuid::from_u128(0x6c6f_6e67_6861_5e64_8072_6f6d_7074_7331);

/// Windows refuses these file names, with or without an extension.
const WINDOWS_RESERVED: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM1", "COM2", "COM3", "COM4", "COM5",
    "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8",
    "LPT9",
];

/// The characters that a macOS or Windows file name cannot hold.
const REFUSED_CHARACTERS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// The prompt folder of the storage folder `root`.
pub fn prompts_folder(root: &Path) -> PathBuf {
    longhand_folder(root).join(PROMPTS_FOLDER)
}

/// One custom prompt as the storage folder holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptFile {
    pub id: String,
    /// The name that Settings shows. It is [`Self::stored_name`], or a
    /// free name when another prompt shows that name already.
    pub name: String,
    /// The name in the file's front matter, or the file name when the file
    /// has no name.
    pub stored_name: String,
    pub text: String,
    /// The text after the marker line, or empty when the file has no
    /// marker line.
    pub title_text: String,
    pub file_name: String,
    /// The other files with the same id, the same text and the same title
    /// text. They show as this one prompt.
    pub duplicates: Vec<String>,
    /// The id that the file holds. It differs from [`Self::id`] for a
    /// conflict copy, and it is `None` for a file with no id.
    pub id_in_file: Option<String>,
}

impl From<&PromptFile> for summarize::Prompt {
    fn from(prompt: &PromptFile) -> Self {
        Self {
            id: prompt.id.clone(),
            name: prompt.name.clone(),
            text: prompt.text.clone(),
            title_text: prompt.title_text.clone(),
        }
    }
}

/// The custom prompts of a storage folder, in the order that Settings
/// shows them.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PromptScan {
    pub prompts: Vec<PromptFile>,
    /// The ids that more than one file with different texts or different
    /// title texts holds. Such an
    /// id has a conflict copy (KTD5).
    pub conflicted: BTreeSet<String>,
}

impl PromptScan {
    /// The prompt with `id`.
    pub fn get(&self, id: &str) -> Option<&PromptFile> {
        self.prompts.iter().find(|p| p.id == id)
    }
}

/// Why a change of a prompt failed. The messages are the UI text of the
/// Settings window.
#[derive(Debug, thiserror::Error)]
pub enum PromptError {
    #[error("That prompt no longer exists.")]
    NoSuchPrompt,
    #[error("A built-in prompt can't be changed. Duplicate it to make your own.")]
    BuiltIn,
    #[error("Enter a name for the prompt.")]
    EmptyName,
    #[error("A prompt with that name already exists.")]
    NameTaken,
    #[error("Enter the text for the prompt.")]
    EmptyText,
    #[error(
        "The prompt text is too long. Use {} characters or fewer.",
        format_count(summarize::prompts::PROMPT_TEXT_LIMIT)
    )]
    TextTooLong,
    #[error("The prompt text can't contain the line {TITLE_MARKER}.")]
    TextHasMarker,
    #[error(
        "The title instructions are too long. Use {} characters or fewer.",
        format_count(summarize::prompts::TITLE_TEXT_LIMIT)
    )]
    TitleTextTooLong,
    /// The shared settings are read-only. The change wrote nothing.
    #[error("{0}")]
    NotAvailable(Unavailable),
    /// The storage folder has no shared settings file. The join fills it
    /// first (KTD8).
    #[error("The storage folder has no shared settings yet.")]
    NoSettingsFile,
    #[error("The prompt file cannot be written: {0}")]
    Io(#[from] io::Error),
}

/// Names are compared trimmed and without case.
pub fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// `base`, or `base 2`, `base 3`, … when a name in `taken` is the same
/// name.
pub fn free_prompt_name<'a>(base: &str, taken: impl Iterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = taken.collect();
    let is_free = |candidate: &str| !taken.iter().any(|t| same_name(t, candidate));
    if is_free(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| is_free(candidate))
        .expect("an unbounded range always gives a free name")
}

/// The file name of a prompt named `name`: the characters that macOS or
/// Windows refuse become `-`, a Windows reserved name gets a `_` prefix,
/// and leading dots, trailing dots and spaces go.
pub fn safe_file_name(name: &str) -> String {
    let replaced: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || REFUSED_CHARACTERS.contains(&c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    // A name that starts with `.` or `~$` is a name that the scan skips.
    let mut stem = replaced.trim_start_matches(['.', ' ']).to_string();
    if stem.starts_with("~$") {
        stem.replace_range(1..2, "-");
    }
    let mut end = stem.len().min(MAX_STEM_BYTES);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    stem.truncate(end);
    let mut stem = stem.trim_end_matches(['.', ' ']).to_string();
    if stem.is_empty() {
        stem = FALLBACK_STEM.to_string();
    }
    let first = stem.split('.').next().unwrap_or("").trim_end();
    if WINDOWS_RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(first))
    {
        stem.insert(0, '_');
    }
    format!("{stem}{EXTENSION}")
}

/// The text of a prompt file with `id`, `name` and `text`.
pub fn render_prompt_file(id: &str, name: &str, text: &str) -> String {
    render(id, name, &[], text, "")
}

/// The text of a prompt file with `id`, `name`, `text` and `title_text`.
pub fn render_prompt_file_with_title(id: &str, name: &str, text: &str, title_text: &str) -> String {
    render(id, name, &[], text, title_text)
}

fn render(id: &str, name: &str, extra: &[String], text: &str, title_text: &str) -> String {
    let name = serde_json::to_string(name).expect("a string serializes");
    let mut out = format!("{FRONT_MATTER_LINE}\n{KEY_ID}: {id}\n{KEY_NAME}: {name}\n");
    for line in extra {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(FRONT_MATTER_LINE);
    out.push_str("\n\n");
    out.push_str(text);
    out.push('\n');
    if !title_text.is_empty() {
        out.push_str(TITLE_MARKER);
        out.push('\n');
        out.push_str(title_text);
        out.push('\n');
    }
    out
}

/// Whether `line`, without its line end, is the marker line.
fn is_marker_line(line: &str) -> bool {
    line.trim() == TITLE_MARKER
}

/// The summary text and the title text of the body of a prompt file. The
/// first marker line splits them. The render adds one line end after each
/// text, and this removes it.
fn split_body(body: &str) -> (String, String) {
    let mut start = 0;
    for line in body.split_inclusive('\n') {
        if is_marker_line(line) {
            let text = strip_final_newline(&body[..start]);
            let title = strip_final_newline(&body[start + line.len()..]);
            return (text.to_string(), title.to_string());
        }
        start += line.len();
    }
    (strip_final_newline(body).to_string(), String::new())
}

/// The parts of a prompt file.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    id: Option<String>,
    name: Option<String>,
    /// The front matter lines that this build does not know. A write
    /// keeps them.
    extra: Vec<String>,
    text: String,
    title_text: String,
}

fn parse_prompt_file(contents: &str) -> Parsed {
    let contents = contents.strip_prefix('\u{feff}').unwrap_or(contents);
    front_matter(contents).unwrap_or_else(|| {
        let (text, title_text) = split_body(contents);
        Parsed {
            id: None,
            name: None,
            extra: Vec::new(),
            text,
            title_text,
        }
    })
}

/// The front matter and the body of `contents`, or `None` when the file
/// has no complete front matter block.
fn front_matter(contents: &str) -> Option<Parsed> {
    let mut lines = contents.split_inclusive('\n');
    if lines.next()?.trim_end() != FRONT_MATTER_LINE {
        return None;
    }
    let mut parsed = Parsed {
        id: None,
        name: None,
        extra: Vec::new(),
        text: String::new(),
        title_text: String::new(),
    };
    let mut consumed = contents.split_inclusive('\n').next()?.len();
    for line in lines {
        consumed += line.len();
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim_end() == FRONT_MATTER_LINE {
            let body = &contents[consumed..];
            let body = body
                .strip_prefix("\r\n")
                .or_else(|| body.strip_prefix('\n'))
                .unwrap_or(body);
            (parsed.text, parsed.title_text) = split_body(body);
            return Some(parsed);
        }
        match line.split_once(':') {
            Some((key, value)) if key.trim() == KEY_ID => {
                parsed.id = Some(front_matter_value(value)).filter(|v| !v.is_empty());
            }
            Some((key, value)) if key.trim() == KEY_NAME => {
                parsed.name = Some(front_matter_value(value)).filter(|v| !v.trim().is_empty());
            }
            _ => parsed.extra.push(line.to_string()),
        }
    }
    None
}

/// A front matter value: a JSON string in quotes, or the text as it is.
fn front_matter_value(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"')
        && let Ok(text) = serde_json::from_str::<String>(value)
    {
        return text;
    }
    value.to_string()
}

fn strip_final_newline(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

/// The id of a file that holds no id, made from its file name.
fn file_name_id(file_name: &str) -> String {
    Uuid::new_v5(&ID_NAMESPACE, format!("file:{file_name}").as_bytes()).to_string()
}

/// The id of a conflict copy, made from the id of its original and its
/// file name.
fn copy_id(id: &str, file_name: &str) -> String {
    Uuid::new_v5(&ID_NAMESPACE, format!("copy:{id}/{file_name}").as_bytes()).to_string()
}

/// The id of a prompt that a join adds when the folder has its id with
/// another text. The same id on each run lets a join that runs again find
/// the prompt that it added.
pub(crate) fn joined_id(id: &str) -> String {
    Uuid::new_v5(&ID_NAMESPACE, format!("join:{id}").as_bytes()).to_string()
}

pub(crate) fn is_built_in(id: &str) -> bool {
    summarize::prompts::built_in(id).is_some()
}

/// The names of the built-in prompts.
pub(crate) fn built_in_names() -> impl Iterator<Item = String> {
    summarize::prompts::built_ins().into_iter().map(|p| p.name)
}

/// One prompt file that the scan read.
struct Entry {
    file_name: String,
    id_in_file: Option<String>,
    stored_name: String,
    text: String,
    title_text: String,
}

impl Entry {
    /// Two files are the same prompt only when both texts are the same
    /// (KTD9).
    fn same_texts(&self, other: &Entry) -> bool {
        self.text == other.text && self.title_text == other.title_text
    }

    /// The file name is the safe file name of the prompt's own name.
    fn qualifies(&self) -> bool {
        self.file_name == safe_file_name(&self.stored_name)
    }
}

/// Read the prompt files of the storage folder `root`. This never writes.
/// A folder with no prompt folder has no custom prompts.
pub fn scan_prompts(root: &Path) -> Result<PromptScan, Unavailable> {
    check_root(root)?;
    let folder = prompts_folder(root);
    let dir = match std::fs::read_dir(&folder) {
        Ok(dir) => dir,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(PromptScan::default()),
        Err(e) => return Err(Unavailable::Unreadable(e.to_string())),
    };
    let mut entries = Vec::new();
    for item in dir {
        let item = item.map_err(|e| Unavailable::Unreadable(e.to_string()))?;
        let Ok(file_name) = item.file_name().into_string() else {
            continue;
        };
        if let Some(entry) = read_entry(&folder, file_name) {
            entries.push(entry);
        }
    }
    Ok(resolve(entries))
}

/// The prompt in `folder/file_name`, or `None` when that file is not a
/// prompt: a hidden or temporary name, a name that is not `.md`, a file
/// over the text limit, or a file with no text.
fn read_entry(folder: &Path, file_name: String) -> Option<Entry> {
    if file_name.starts_with('.') || file_name.starts_with("~$") {
        return None;
    }
    let stem_end = file_name.len().checked_sub(EXTENSION.len())?;
    if stem_end == 0
        || !file_name.is_char_boundary(stem_end)
        || !file_name[stem_end..].eq_ignore_ascii_case(EXTENSION)
    {
        return None;
    }
    let path = folder.join(&file_name);
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(e) => {
            log::warn!(
                "shared settings: cannot read the prompt file {}: {e}",
                path.display()
            );
            return None;
        }
    };
    let parsed = parse_prompt_file(&contents);
    if parsed.text.trim().is_empty()
        || parsed.text.chars().count() > summarize::prompts::PROMPT_TEXT_LIMIT
        || parsed.title_text.chars().count() > summarize::prompts::TITLE_TEXT_LIMIT
    {
        return None;
    }
    // A file cannot take the id of a built-in prompt.
    let id_in_file = parsed.id.filter(|id| !is_built_in(id));
    let stored_name = parsed
        .name
        .map(|name| name.trim().to_string())
        .unwrap_or_else(|| file_name[..stem_end].to_string());
    Some(Entry {
        file_name,
        id_in_file,
        stored_name,
        text: parsed.text,
        title_text: parsed.title_text,
    })
}

/// Give each file its id (KTD5), join the files with one id and the same
/// texts (KTD9), and give each prompt the name that Settings shows.
fn resolve(entries: Vec<Entry>) -> PromptScan {
    let mut groups: BTreeMap<String, Vec<Entry>> = BTreeMap::new();
    for entry in entries {
        let id = entry
            .id_in_file
            .clone()
            .unwrap_or_else(|| file_name_id(&entry.file_name));
        groups.entry(id).or_default().push(entry);
    }
    let mut conflicted = BTreeSet::new();
    // Each prompt, and whether its file name matches its name.
    let mut prompts: Vec<(PromptFile, bool)> = Vec::new();
    for (id, mut files) in groups {
        files.sort_by(|a, b| a.file_name.as_bytes().cmp(b.file_name.as_bytes()));
        // The files with the same texts, in file name order.
        let mut texts: Vec<Vec<Entry>> = Vec::new();
        for file in files {
            match texts.iter_mut().find(|group| group[0].same_texts(&file)) {
                Some(group) => group.push(file),
                None => texts.push(vec![file]),
            }
        }
        let keeper = if texts.len() == 1 {
            0
        } else {
            conflicted.insert(id.clone());
            let qualifying: Vec<usize> = texts
                .iter()
                .enumerate()
                .flat_map(|(i, group)| group.iter().filter(|f| f.qualifies()).map(move |_| i))
                .collect();
            match qualifying.as_slice() {
                [only] => *only,
                // The first file name in byte order is in the first group.
                _ => 0,
            }
        };
        for (i, mut group) in texts.into_iter().enumerate() {
            // The file whose name matches its name shows the prompt.
            let shown = group.iter().position(Entry::qualifies).unwrap_or(0);
            let entry = group.remove(shown);
            let prompt_id = if i == keeper {
                id.clone()
            } else {
                copy_id(&id, &entry.file_name)
            };
            let qualifies = entry.qualifies();
            prompts.push((
                PromptFile {
                    id: prompt_id,
                    name: entry.stored_name.clone(),
                    stored_name: entry.stored_name,
                    text: entry.text,
                    title_text: entry.title_text,
                    file_name: entry.file_name,
                    duplicates: group.into_iter().map(|f| f.file_name).collect(),
                    id_in_file: entry.id_in_file,
                },
                qualifies,
            ));
        }
    }
    // A shown name that another prompt shows already gets a free name.
    // Files whose name matches their file name keep their names first.
    prompts.sort_by(|(a, a_qualifies), (b, b_qualifies)| {
        b_qualifies
            .cmp(a_qualifies)
            .then_with(|| a.file_name.as_bytes().cmp(b.file_name.as_bytes()))
    });
    let mut taken: Vec<String> = built_in_names().collect();
    let mut prompts: Vec<PromptFile> = prompts
        .into_iter()
        .map(|(mut prompt, _)| {
            prompt.name = free_prompt_name(&prompt.stored_name, taken.iter().map(String::as_str));
            taken.push(prompt.name.clone());
            prompt
        })
        .collect();
    prompts.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.file_name.as_bytes().cmp(b.file_name.as_bytes()))
    });
    PromptScan {
        prompts,
        conflicted,
    }
}

/// Refuse a change while the shared settings are read-only (R16, KTD3).
/// Returns the settings file that it read.
fn ensure_writable(root: &Path) -> Result<SettingsFile, PromptError> {
    match read_settings(root) {
        FileRead::Parsed(settings) => Ok(settings),
        FileRead::Missing => Err(PromptError::NoSettingsFile),
        FileRead::NotAvailable(cause) => Err(PromptError::NotAvailable(cause)),
    }
}

fn check_text(text: &str) -> Result<(), PromptError> {
    if text.trim().is_empty() {
        return Err(PromptError::EmptyText);
    }
    if text.chars().count() > summarize::prompts::PROMPT_TEXT_LIMIT {
        return Err(PromptError::TextTooLong);
    }
    // The file must parse back to the same summary text (KTD8).
    if text.split('\n').any(is_marker_line) {
        return Err(PromptError::TextHasMarker);
    }
    Ok(())
}

/// An empty title text is valid (R4).
fn check_title_text(title_text: &str) -> Result<(), PromptError> {
    if title_text.chars().count() > summarize::prompts::TITLE_TEXT_LIMIT {
        return Err(PromptError::TitleTextTooLong);
    }
    Ok(())
}

fn check_custom(id: &str) -> Result<(), PromptError> {
    if is_built_in(id) {
        return Err(PromptError::BuiltIn);
    }
    Ok(())
}

fn scan_for_change(root: &Path) -> Result<PromptScan, PromptError> {
    scan_prompts(root).map_err(PromptError::NotAvailable)
}

/// The names of the built-in prompts and of the prompts in `scan`, except
/// the prompt `except`.
fn taken_names<'a>(
    scan: &'a PromptScan,
    except: Option<&'a str>,
) -> impl Iterator<Item = String> + 'a {
    built_in_names().chain(
        scan.prompts
            .iter()
            .filter(move |p| Some(p.id.as_str()) != except)
            .map(|p| p.name.clone()),
    )
}

/// The names in the prompt folder, in lower case, so that a free file name
/// is free on a file system that ignores case too.
fn names_in_folder(folder: &Path) -> io::Result<BTreeSet<String>> {
    match std::fs::read_dir(folder) {
        Ok(dir) => dir
            .map(|item| item.map(|i| i.file_name().to_string_lossy().to_lowercase()))
            .collect(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(BTreeSet::new()),
        Err(e) => Err(e),
    }
}

/// `safe`, or `stem 2.md`, `stem 3.md`, … when that file name is taken.
fn free_file_name(safe: &str, taken: &BTreeSet<String>) -> String {
    if !taken.contains(&safe.to_lowercase()) {
        return safe.to_string();
    }
    let stem = &safe[..safe.len() - EXTENSION.len()];
    (2..)
        .map(|n| format!("{stem} {n}{EXTENSION}"))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("an unbounded range always gives a free name")
}

/// Write a new prompt file for `id`, `name`, `text` and `title_text` under
/// a free file name. A file that appears at that name in the meantime
/// stays. Returns the file name.
pub(crate) fn write_new_prompt(
    root: &Path,
    id: &str,
    name: &str,
    text: &str,
    title_text: &str,
) -> io::Result<String> {
    let folder = prompts_folder(root);
    std::fs::create_dir_all(&folder)?;
    let contents = render(id, name, &[], text, title_text);
    let mut taken = names_in_folder(&folder)?;
    for _ in 0..NAME_ATTEMPTS {
        let file_name = free_file_name(&safe_file_name(name), &taken);
        if write_new(&folder, &file_name, contents.as_bytes())? {
            return Ok(file_name);
        }
        taken.insert(file_name.to_lowercase());
    }
    Err(names_taken())
}

/// The most file names that a write tries when other files take them in
/// the meantime.
const NAME_ATTEMPTS: usize = 100;

fn names_taken() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "other files took each free file name",
    )
}

/// Add a prompt with `text` and an empty title text, named `base` or a
/// free name made from it. Returns the new prompt.
pub fn create_prompt(root: &Path, base: &str, text: &str) -> Result<PromptFile, PromptError> {
    create_prompt_with_title(root, base, text, "")
}

/// Add a prompt with `text` and `title_text`, named `base` or a free name
/// made from it. Returns the new prompt.
pub fn create_prompt_with_title(
    root: &Path,
    base: &str,
    text: &str,
    title_text: &str,
) -> Result<PromptFile, PromptError> {
    check_text(text)?;
    check_title_text(title_text)?;
    let base = base.trim();
    if base.is_empty() {
        return Err(PromptError::EmptyName);
    }
    ensure_writable(root)?;
    let scan = scan_for_change(root)?;
    let taken: Vec<String> = taken_names(&scan, None).collect();
    let name = free_prompt_name(base, taken.iter().map(String::as_str));
    // A random id never matches a built-in id, which is a fixed word.
    let id = Uuid::new_v4().to_string();
    let file_name = write_new_prompt(root, &id, &name, text, title_text)?;
    Ok(PromptFile {
        id: id.clone(),
        name: name.clone(),
        stored_name: name,
        text: text.to_string(),
        title_text: title_text.to_string(),
        file_name,
        duplicates: Vec::new(),
        id_in_file: Some(id),
    })
}

/// Give the prompt `id` a new name (KTD4). The new name goes into the
/// existing file first, and then the file gets the new file name, so a
/// crash between the two steps leaves one file with the id.
pub fn rename_prompt(root: &Path, id: &str, name: &str) -> Result<PromptFile, PromptError> {
    check_custom(id)?;
    let name = name.trim();
    if name.is_empty() {
        return Err(PromptError::EmptyName);
    }
    ensure_writable(root)?;
    let scan = scan_for_change(root)?;
    let prompt = scan.get(id).ok_or(PromptError::NoSuchPrompt)?.clone();
    if taken_names(&scan, Some(id)).any(|taken| same_name(&taken, name)) {
        return Err(PromptError::NameTaken);
    }
    settle_copies(root, &scan, id)?;
    let folder = prompts_folder(root);
    rewrite(&folder, &prompt, name, &prompt.text, &prompt.title_text)?;
    let file_name = move_to_name(&folder, &prompt.file_name, name)?;
    Ok(PromptFile {
        name: name.to_string(),
        stored_name: name.to_string(),
        file_name,
        duplicates: Vec::new(),
        id_in_file: Some(prompt.id.clone()),
        ..prompt
    })
}

/// Give the prompt `id` a new text. This writes one file.
pub fn set_prompt_text(root: &Path, id: &str, text: &str) -> Result<PromptFile, PromptError> {
    check_custom(id)?;
    check_text(text)?;
    ensure_writable(root)?;
    let scan = scan_for_change(root)?;
    let prompt = scan.get(id).ok_or(PromptError::NoSuchPrompt)?.clone();
    settle_copies(root, &scan, id)?;
    rewrite(
        &prompts_folder(root),
        &prompt,
        &prompt.stored_name,
        text,
        &prompt.title_text,
    )?;
    Ok(PromptFile {
        text: text.to_string(),
        duplicates: Vec::new(),
        id_in_file: Some(prompt.id.clone()),
        ..prompt
    })
}

/// Give the prompt `id` a new title text. This writes one file, and keeps
/// the summary text. An empty title text removes the marker line.
pub fn set_prompt_title_text(
    root: &Path,
    id: &str,
    title_text: &str,
) -> Result<PromptFile, PromptError> {
    check_custom(id)?;
    check_title_text(title_text)?;
    ensure_writable(root)?;
    let scan = scan_for_change(root)?;
    let prompt = scan.get(id).ok_or(PromptError::NoSuchPrompt)?.clone();
    settle_copies(root, &scan, id)?;
    rewrite(
        &prompts_folder(root),
        &prompt,
        &prompt.stored_name,
        &prompt.text,
        title_text,
    )?;
    Ok(PromptFile {
        title_text: title_text.to_string(),
        duplicates: Vec::new(),
        id_in_file: Some(prompt.id.clone()),
        ..prompt
    })
}

/// Remove each file of the prompt `id`. Returns the shared settings file
/// as it was before the delete.
pub fn delete_prompt(root: &Path, id: &str) -> Result<SettingsFile, PromptError> {
    check_custom(id)?;
    let settings = ensure_writable(root)?;
    let scan = scan_for_change(root)?;
    let prompt = scan.get(id).ok_or(PromptError::NoSuchPrompt)?;
    settle_copies(root, &scan, id)?;
    let folder = prompts_folder(root);
    for file_name in std::iter::once(&prompt.file_name).chain(&prompt.duplicates) {
        remove_if_present(&folder.join(file_name))?;
    }
    session_save::fsync::sync_dir(&folder)?;
    Ok(settings)
}

/// Write the made id into each conflict copy of `id` before Longhand
/// changes the prompt `id` (KTD5). Without this step, a change of the
/// original could give its id to a copy.
fn settle_copies(root: &Path, scan: &PromptScan, id: &str) -> Result<(), PromptError> {
    if !scan.conflicted.contains(id) {
        return Ok(());
    }
    let folder = prompts_folder(root);
    for copy in scan
        .prompts
        .iter()
        .filter(|p| p.id != id && p.id_in_file.as_deref() == Some(id))
    {
        rewrite(
            &folder,
            copy,
            &copy.stored_name,
            &copy.text,
            &copy.title_text,
        )?;
    }
    Ok(())
}

/// Write the file of `prompt` with its id, `name`, `text` and
/// `title_text`, and remove its duplicates, which have the old texts.
fn rewrite(
    folder: &Path,
    prompt: &PromptFile,
    name: &str,
    text: &str,
    title_text: &str,
) -> Result<(), PromptError> {
    let path = folder.join(&prompt.file_name);
    let extra = match std::fs::read_to_string(&path) {
        Ok(contents) => parse_prompt_file(&contents).extra,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(PromptError::NoSuchPrompt),
        Err(e) => return Err(e.into()),
    };
    let contents = render(&prompt.id, name, &extra, text, title_text);
    session_save::home_write::write_atomic(folder, &prompt.file_name, contents.as_bytes())?;
    for duplicate in &prompt.duplicates {
        remove_if_present(&folder.join(duplicate))?;
    }
    Ok(())
}

/// Give the file `current` the file name of `name`. Returns the file name.
fn move_to_name(folder: &Path, current: &str, name: &str) -> Result<String, PromptError> {
    let safe = safe_file_name(name);
    if safe == current {
        return Ok(safe);
    }
    let from = folder.join(current);
    if safe.to_lowercase() == current.to_lowercase() && !has_exact_name(folder, &safe)? {
        // Only the case changes. The target is the same file on a file
        // system that ignores case, so a rename that refuses a taken name
        // cannot do this.
        std::fs::rename(&from, folder.join(&safe))?;
        session_save::fsync::sync_dir(folder)?;
        return Ok(safe);
    }
    let mut taken = names_in_folder(folder)?;
    taken.remove(&current.to_lowercase());
    for _ in 0..NAME_ATTEMPTS {
        let file_name = free_file_name(&safe, &taken);
        if file_name == current {
            return Ok(file_name);
        }
        match session_save::rename::rename_noreplace(&from, &folder.join(&file_name)) {
            Ok(()) => {
                session_save::fsync::sync_dir(folder)?;
                return Ok(file_name);
            }
            Err(e) if session_save::rename::is_name_taken(&e) => {
                taken.insert(file_name.to_lowercase());
            }
            Err(e) => return Err(e.into()),
        }
    }
    Err(names_taken().into())
}

/// Whether the folder has an entry with exactly `name`, case included.
fn has_exact_name(folder: &Path, name: &str) -> io::Result<bool> {
    for item in std::fs::read_dir(folder)? {
        if item?.file_name() == name {
            return Ok(true);
        }
    }
    Ok(false)
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `4000` as "4,000", for UI text.
fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rendered_file_parses_back() {
        let text = "Line one.\n\nLine two.";
        let parsed = parse_prompt_file(&render("abc", "1:1 \"notes\"", &[], text, ""));
        assert_eq!(parsed.id.as_deref(), Some("abc"));
        assert_eq!(parsed.name.as_deref(), Some("1:1 \"notes\""));
        assert_eq!(parsed.text, text);
        assert_eq!(parsed.title_text, "");
        assert!(parsed.extra.is_empty());
    }

    #[test]
    fn each_pair_of_texts_parses_back() {
        let texts = ["One.", "\nOne.\n\n", "One.\r\nTwo."];
        let titles = [
            "",
            "Title.",
            "\n",
            "\n# Title\n\n",
            "A\n<!-- longhand: title instructions -->\nB",
        ];
        for text in texts {
            for title in titles {
                let parsed = parse_prompt_file(&render("abc", "N", &[], text, title));
                assert_eq!(
                    (parsed.text.as_str(), parsed.title_text.as_str()),
                    (text, title)
                );
            }
        }
    }

    #[test]
    fn a_file_with_no_front_matter_splits_at_the_marker() {
        let parsed = parse_prompt_file(&format!("Summary.\n{TITLE_MARKER}\nTitle."));
        assert_eq!(parsed.text, "Summary.");
        assert_eq!(parsed.title_text, "Title.");
    }

    #[test]
    fn the_largest_file_fits_the_byte_limit() {
        let text = "😀".repeat(summarize::prompts::PROMPT_TEXT_LIMIT);
        let title = "😀".repeat(summarize::prompts::TITLE_TEXT_LIMIT);
        let id = Uuid::new_v4().to_string();
        let contents = render(&id, &"n".repeat(MAX_STEM_BYTES), &[], &text, &title);
        assert!(contents.len() as u64 <= MAX_FILE_BYTES);
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_front_matter() {
        let parsed = parse_prompt_file("\u{feff}---\nid: abc\n---\nText.");
        assert_eq!(parsed.id.as_deref(), Some("abc"));
        assert_eq!(parsed.text, "Text.");
    }

    #[test]
    fn a_name_without_quotes_is_taken_as_it_is() {
        let parsed = parse_prompt_file("---\nname:  Weekly: review \n---\nText.");
        assert_eq!(parsed.name.as_deref(), Some("Weekly: review"));
    }

    #[test]
    fn made_ids_do_not_change() {
        // Every computer and every version must make these same ids.
        assert_eq!(file_name_id("Recap.md"), file_name_id("Recap.md"));
        assert_ne!(file_name_id("Recap.md"), file_name_id("Recap 2.md"));
        assert_ne!(copy_id("a", "Recap.md"), copy_id("b", "Recap.md"));
        assert_eq!(
            Uuid::parse_str(&file_name_id("Recap.md"))
                .unwrap()
                .get_version_num(),
            5
        );
    }
}
