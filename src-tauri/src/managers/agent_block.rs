//! The managed block in `AGENTS.md` and `CLAUDE.md` of the storage folder
//! (R29, R30, KTD12): a short text between two markers that tells an AI
//! agent that the folder is a Longhand storage folder, gives the install
//! steps of the `longhand-notes` skill, and gives the main rules.
//!
//! Longhand writes the first block only when the user asks in Settings >
//! Storage. After that, a change that the user makes on this computer
//! replaces a block that an older Longhand wrote. Longhand never replaces
//! a block that a newer Longhand wrote, so computers with different
//! versions do not write the block in turns. A launch, a read, a scan or a
//! file event writes nothing. Longhand changes only the text between its
//! markers, and keeps the rest of each file, its line ends and its byte
//! order mark.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use specta::Type;

use crate::managers::destination::{self, DestinationStatus};

/// The version of the block text. Increase it at each change of
/// [`BLOCK_LINES`], so that a newer Longhand replaces the old block (R30).
pub const BLOCK_VERSION: u32 = 1;

/// The file that every agent reads.
pub const AGENTS_FILE: &str = "AGENTS.md";
/// The file that Claude Code reads.
pub const CLAUDE_FILE: &str = "CLAUDE.md";

/// The start of the begin marker. The version follows it.
const BEGIN_PREFIX: &str = "<!-- BEGIN:longhand-agent-instructions";
const END_MARKER: &str = "<!-- END:longhand-agent-instructions -->";

/// The text between the markers. Keep it the same as the skill
/// (`plugins/longhand/skills/longhand-notes/SKILL.md`) and the install
/// steps in `README.md`.
const BLOCK_LINES: &[&str] = &[
    "## Longhand storage folder",
    "",
    "This folder is a Longhand storage folder. Longhand keeps each note as plain files in a note folder here.",
    "",
    "Before you read or change a note or a setting here, use the `longhand-notes` skill. If the skill is not installed, tell the user to install it:",
    "",
    "- Claude Code: `/plugin marketplace add twistymaze/longhand`, then `/plugin install longhand@longhand`.",
    "- Codex: `codex plugin marketplace add twistymaze/longhand`, then `codex plugin add longhand@longhand`.",
    "- Other agents: `npx skills add twistymaze/longhand --skill longhand-notes -g`.",
    "",
    "The full install steps are in https://github.com/twistymaze/longhand#use-your-notes-with-an-ai-agent. Do not install software without the user's permission.",
    "",
    "Main rules:",
    "",
    "1. You can edit `session.md`, `notes.md`, `transcript.md` and `summary.md` in a note folder, and `settings.json`, `background.md` and the prompt files in `.longhand/`. Never edit, move or delete `longhand.json`, `recognition.jsonl`, the files in `audio/`, a `.longhand-in-use-*` file or `.longhand/library`.",
    "2. Do not edit a note that a computer records or processes.",
    "3. Do not make, copy or delete a note folder. Do not write other files into this folder.",
    "4. Do not look for, start or control the Longhand app.",
    "",
    "Longhand writes the text between these markers and can replace it. Write your own text outside the markers.",
];

/// The state of the managed block, as Settings > Storage shows it.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentBlockStatus {
    /// At least one of the files has no block. The button adds it.
    NotAdded,
    /// Each file has a block.
    Added,
    /// Longhand cannot write the files now. `reason` tells why.
    Unavailable { reason: String },
    /// The markers in `file` are damaged: a begin marker with no end
    /// marker, an end marker with no begin marker, two blocks, or a begin
    /// marker with no version. Longhand writes nothing until the user
    /// fixes the file.
    Damaged { file: String },
}

/// The block in the text of one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    /// The text has no marker.
    None,
    /// One block, from byte `start` to byte `end`, of version `version`.
    Block {
        start: usize,
        end: usize,
        version: u32,
    },
    Damaged,
}

/// Find the block in `text`.
fn find(text: &str) -> Found {
    let begins: Vec<usize> = text.match_indices(BEGIN_PREFIX).map(|(i, _)| i).collect();
    let ends: Vec<usize> = text.match_indices(END_MARKER).map(|(i, _)| i).collect();
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Found::None,
        ([start], [end]) if start < end => {
            let rest = &text[start + BEGIN_PREFIX.len()..];
            let version = rest
                .strip_prefix(" version=")
                .and_then(|r| r.split_once(" -->"))
                .and_then(|(v, _)| v.parse::<u32>().ok());
            match version {
                Some(version) => Found::Block {
                    start: *start,
                    end: end + END_MARKER.len(),
                    version,
                },
                None => Found::Damaged,
            }
        }
        _ => Found::Damaged,
    }
}

/// The block of [`BLOCK_VERSION`], with `eol` as the line end and no line
/// end after the end marker.
fn block_text(eol: &str) -> String {
    let mut lines = vec![format!("{BEGIN_PREFIX} version={BLOCK_VERSION} -->")];
    lines.extend(BLOCK_LINES.iter().map(|l| (*l).to_string()));
    lines.push(END_MARKER.to_string());
    lines.join(eol)
}

const BOM: &str = "\u{feff}";

/// One file that gets the block.
struct Target {
    /// The file name, as the messages show it.
    name: &'static str,
    /// The path to write. For a symlink, the file that it points to.
    path: PathBuf,
    /// The text of the file, with no byte order mark, or `None` when the
    /// file does not exist.
    text: Option<String>,
    bom: bool,
    found: Found,
}

impl Target {
    /// Whether the file has a block that an older Longhand wrote.
    fn has_older_block(&self) -> bool {
        matches!(self.found, Found::Block { version, .. } if version < BLOCK_VERSION)
    }

    /// The text of the file with `block` in place of the old block, or
    /// after the old text when the file has no block.
    fn with_block(&self) -> String {
        let text = self.text.as_deref().unwrap_or("");
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let block = block_text(eol);
        let body = match self.found {
            Found::Block { start, end, .. } => {
                format!("{}{block}{}", &text[..start], &text[end..])
            }
            _ if text.is_empty() => format!("{block}{eol}"),
            _ => {
                let sep = if text.ends_with('\n') { "" } else { eol };
                format!("{text}{sep}{eol}{block}{eol}")
            }
        };
        if self.bom {
            format!("{BOM}{body}")
        } else {
            body
        }
    }

    fn write(&self) -> Result<(), String> {
        let fail = |e: io::Error| format!("Longhand can’t write {}: {e}", self.name);
        let dir = self
            .path
            .parent()
            .ok_or_else(|| fail(io::ErrorKind::NotFound.into()))?;
        let file_name = self
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| fail(io::ErrorKind::InvalidInput.into()))?;
        session_save::home_write::write_atomic(dir, file_name, self.with_block().as_bytes())
            .map_err(fail)
    }
}

/// Read the file `name` in `root`. A symlink is followed.
fn read_target(root: &Path, name: &'static str) -> Result<Target, String> {
    let link = root.join(name);
    let path = match std::fs::symlink_metadata(&link) {
        Ok(meta) if meta.file_type().is_symlink() => link.canonicalize().unwrap_or(link),
        _ => link,
    };
    let (text, bom) = match std::fs::read(&path) {
        Ok(bytes) => {
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("Longhand can’t read {name}: the file is not UTF-8 text."))?;
            match text.strip_prefix(BOM) {
                Some(rest) => (Some(rest.to_string()), true),
                None => (Some(text), false),
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (None, false),
        Err(e) => return Err(format!("Longhand can’t read {name}: {e}")),
    };
    let found = text.as_deref().map_or(Found::None, find);
    Ok(Target {
        name,
        path,
        text,
        bom,
        found,
    })
}

/// Whether `CLAUDE.md` in `root` is a symlink to `AGENTS.md` in `root`.
fn claude_links_to_agents(root: &Path) -> bool {
    let claude = root.join(CLAUDE_FILE);
    let Ok(target) = std::fs::read_link(&claude) else {
        return false;
    };
    let agents = root.join(AGENTS_FILE);
    if let (Ok(a), Ok(b)) = (claude.canonicalize(), agents.canonicalize()) {
        return a == b;
    }
    // A link to an `AGENTS.md` that does not exist yet.
    let target = target
        .strip_prefix(".")
        .map(Path::to_path_buf)
        .unwrap_or(target);
    target == Path::new(AGENTS_FILE) || target == agents
}

/// Whether the text of `CLAUDE.md` imports `AGENTS.md`: a line that is
/// `@AGENTS.md`.
fn imports_agents(text: &str) -> bool {
    text.lines().any(|l| {
        let l = l.trim();
        l == "@AGENTS.md" || l == "@./AGENTS.md"
    })
}

/// The files that get the block. `CLAUDE.md` gets no block of its own
/// when it is a symlink to `AGENTS.md`, or when it imports `AGENTS.md` and
/// has no block: Claude Code then reads the block in `AGENTS.md`.
fn targets(root: &Path) -> Result<Vec<Target>, String> {
    let mut targets = vec![read_target(root, AGENTS_FILE)?];
    if !claude_links_to_agents(root) {
        let claude = read_target(root, CLAUDE_FILE)?;
        let covered =
            claude.found == Found::None && claude.text.as_deref().is_some_and(imports_agents);
        if !covered {
            targets.push(claude);
        }
    }
    Ok(targets)
}

/// The first file with damaged markers, as an error for the user.
fn damaged(targets: &[Target]) -> Option<AgentBlockStatus> {
    targets
        .iter()
        .find(|t| t.found == Found::Damaged)
        .map(|t| AgentBlockStatus::Damaged {
            file: t.name.to_string(),
        })
}

/// The storage folder `root` when Longhand can write it, or the reason
/// why it cannot.
fn writable_root(root: Option<&Path>) -> Result<&Path, String> {
    let Some(root) = root else {
        return Err("No storage folder is set.".into());
    };
    match destination::check(root, 0) {
        DestinationStatus::Ready { .. } | DestinationStatus::LowSpace { .. } => Ok(root),
        DestinationStatus::Missing { .. } | DestinationStatus::NotADirectory { .. } => {
            Err("The storage folder is not available.".into())
        }
        DestinationStatus::NotWritable { message, .. } => Err(format!(
            "Longhand can’t write to the storage folder: {message}"
        )),
    }
}

/// The state of the block in the storage folder `root`. This writes
/// nothing into the files.
pub fn status(root: Option<&Path>) -> AgentBlockStatus {
    let root = match writable_root(root) {
        Ok(root) => root,
        Err(reason) => return AgentBlockStatus::Unavailable { reason },
    };
    match targets(root) {
        Err(reason) => AgentBlockStatus::Unavailable { reason },
        Ok(targets) => damaged(&targets).unwrap_or_else(|| {
            if targets
                .iter()
                .all(|t| matches!(t.found, Found::Block { .. }))
            {
                AgentBlockStatus::Added
            } else {
                AgentBlockStatus::NotAdded
            }
        }),
    }
}

/// Add the block to each file in `root` that has none, and replace an
/// older block (R29). The user asked for this in Settings. A file with
/// damaged markers stops the write of both files. Returns the state after
/// the write, or an error that names the file.
pub fn add(root: Option<&Path>) -> Result<AgentBlockStatus, String> {
    let root = writable_root(root)?;
    let targets = targets(root)?;
    if let Some(damaged) = damaged(&targets) {
        return Ok(damaged);
    }
    for target in &targets {
        if target.found == Found::None || target.has_older_block() {
            target.write()?;
        }
    }
    // Each target now has a block of this version or of a newer version.
    Ok(AgentBlockStatus::Added)
}

/// Replace each block in `root` that an older Longhand wrote (R30, KTD12).
/// Call this only at a write into the storage folder that a change of the
/// user on this computer causes. This never adds a block, never changes a
/// block of this version or of a newer version, and writes nothing when a
/// file has damaged markers. Returns the names of the files that it wrote.
pub fn update(root: &Path) -> Result<Vec<&'static str>, String> {
    let targets = targets(root)?;
    if damaged(&targets).is_some() {
        return Ok(Vec::new());
    }
    let mut written = Vec::new();
    for target in &targets {
        if target.has_older_block() {
            target.write()?;
            written.push(target.name);
        }
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn read(root: &Path, name: &str) -> String {
        std::fs::read_to_string(root.join(name)).unwrap()
    }

    fn write(root: &Path, name: &str, text: &str) {
        std::fs::write(root.join(name), text).unwrap();
    }

    /// A block of `version` with `body` between the markers.
    fn old_block(version: u32, body: &str) -> String {
        format!("{BEGIN_PREFIX} version={version} -->\n{body}\n{END_MARKER}")
    }

    fn count(text: &str, pattern: &str) -> usize {
        text.matches(pattern).count()
    }

    #[test]
    fn the_button_adds_a_block_and_keeps_the_user_text() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, AGENTS_FILE, "# My rules\n\nUse short answers.\n");
        assert_eq!(status(Some(root)), AgentBlockStatus::NotAdded);

        assert_eq!(add(Some(root)), Ok(AgentBlockStatus::Added));

        let agents = read(root, AGENTS_FILE);
        assert!(agents.starts_with("# My rules\n\nUse short answers.\n"));
        assert_eq!(count(&agents, BEGIN_PREFIX), 1);
        assert_eq!(count(&agents, END_MARKER), 1);
        assert!(agents.contains("version=1 -->"));
        assert!(agents.contains("`longhand-notes` skill"));
        assert!(agents.contains("/plugin marketplace add twistymaze/longhand"));
        assert!(agents.ends_with(&format!("{END_MARKER}\n")));
        // CLAUDE.md is made as a real file with one block.
        let claude_meta = std::fs::symlink_metadata(root.join(CLAUDE_FILE)).unwrap();
        assert!(claude_meta.file_type().is_file());
        let claude = read(root, CLAUDE_FILE);
        assert_eq!(count(&claude, BEGIN_PREFIX), 1);
        assert!(claude.starts_with(BEGIN_PREFIX));
    }

    #[test]
    fn a_second_add_changes_nothing() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        add(Some(root)).unwrap();
        let before = read(root, AGENTS_FILE);
        add(Some(root)).unwrap();
        assert_eq!(read(root, AGENTS_FILE), before);
    }

    #[test]
    fn an_update_replaces_an_older_block_and_keeps_the_text_around_it() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let text = format!(
            "Top text.\n\n{}\n\nBottom text.\n",
            old_block(0, "Old rules.")
        );
        write(root, AGENTS_FILE, &text);
        write(root, CLAUDE_FILE, "Claude text.\n");

        assert_eq!(update(root), Ok(vec![AGENTS_FILE]));

        let agents = read(root, AGENTS_FILE);
        assert!(
            agents
                .starts_with("Top text.\n\n<!-- BEGIN:longhand-agent-instructions version=1 -->\n")
        );
        assert!(agents.ends_with(&format!("{END_MARKER}\n\nBottom text.\n")));
        assert!(!agents.contains("Old rules."));
        // The update never adds a block to a file that has none.
        assert_eq!(read(root, CLAUDE_FILE), "Claude text.\n");
    }

    #[test]
    fn a_block_of_a_newer_version_is_not_touched() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let newer = format!("{}\n", old_block(BLOCK_VERSION + 1, "Newer rules."));
        write(root, AGENTS_FILE, &newer);
        write(root, CLAUDE_FILE, &newer);

        assert_eq!(update(root), Ok(vec![]));
        assert_eq!(add(Some(root)), Ok(AgentBlockStatus::Added));

        assert_eq!(read(root, AGENTS_FILE), newer);
        assert_eq!(read(root, CLAUDE_FILE), newer);
    }

    #[test]
    fn with_no_block_an_update_writes_nothing() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, AGENTS_FILE, "User text.\n");

        assert_eq!(update(root), Ok(vec![]));

        assert_eq!(read(root, AGENTS_FILE), "User text.\n");
        assert!(!root.join(CLAUDE_FILE).exists());
    }

    #[test]
    fn a_begin_marker_with_no_end_marker_stops_each_write_and_names_the_file() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let agents = format!("{}\n", old_block(0, "Old rules."));
        let claude = format!("{BEGIN_PREFIX} version=0 -->\nOld rules.\n");
        write(root, AGENTS_FILE, &agents);
        write(root, CLAUDE_FILE, &claude);
        let damaged = AgentBlockStatus::Damaged {
            file: CLAUDE_FILE.into(),
        };

        assert_eq!(status(Some(root)), damaged);
        assert_eq!(add(Some(root)), Ok(damaged));
        assert_eq!(update(root), Ok(vec![]));

        assert_eq!(read(root, AGENTS_FILE), agents);
        assert_eq!(read(root, CLAUDE_FILE), claude);
    }

    #[test]
    fn two_blocks_in_one_file_are_damaged() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let text = format!("{}\n{}\n", old_block(0, "A."), old_block(0, "B."));
        write(root, AGENTS_FILE, &text);

        assert_eq!(
            add(Some(root)),
            Ok(AgentBlockStatus::Damaged {
                file: AGENTS_FILE.into()
            })
        );
        assert_eq!(read(root, AGENTS_FILE), text);
        assert!(!root.join(CLAUDE_FILE).exists());
    }

    #[test]
    fn crlf_and_a_byte_order_mark_stay() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, AGENTS_FILE, "\u{feff}Line one.\r\nLine two.\r\n");

        add(Some(root)).unwrap();

        let agents = read(root, AGENTS_FILE);
        assert!(agents.starts_with("\u{feff}Line one.\r\nLine two.\r\n\r\n<!-- BEGIN"));
        assert_eq!(count(&agents, "\u{feff}"), 1);
        // Every line end is CRLF.
        assert_eq!(count(&agents, "\n"), count(&agents, "\r\n"));
        assert!(agents.ends_with(&format!("{END_MARKER}\r\n")));

        // An update keeps them too.
        let old = agents.replace("version=1 -->", "version=0 -->");
        write(root, AGENTS_FILE, &old);
        update(root).unwrap();
        assert_eq!(read(root, AGENTS_FILE), agents);
    }

    #[test]
    fn a_file_with_no_line_end_at_the_end_gets_one_before_the_block() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, AGENTS_FILE, "No line end");
        add(Some(root)).unwrap();
        assert!(read(root, AGENTS_FILE).starts_with("No line end\n\n<!-- BEGIN"));
    }

    #[cfg(unix)]
    #[test]
    fn a_claude_md_that_links_to_agents_md_gets_one_block() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, AGENTS_FILE, "User text.\n");
        std::os::unix::fs::symlink(AGENTS_FILE, root.join(CLAUDE_FILE)).unwrap();

        assert_eq!(add(Some(root)), Ok(AgentBlockStatus::Added));

        assert!(
            std::fs::symlink_metadata(root.join(CLAUDE_FILE))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(count(&read(root, AGENTS_FILE), BEGIN_PREFIX), 1);
    }

    #[test]
    fn a_claude_md_that_imports_agents_md_gets_no_block() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(root, CLAUDE_FILE, "@AGENTS.md\n");

        assert_eq!(add(Some(root)), Ok(AgentBlockStatus::Added));

        assert_eq!(read(root, CLAUDE_FILE), "@AGENTS.md\n");
        assert_eq!(count(&read(root, AGENTS_FILE), BEGIN_PREFIX), 1);
    }

    #[test]
    fn a_folder_that_is_not_available_is_refused_with_the_reason() {
        let dir = TempDir::new().unwrap();
        let gone = dir.path().join("gone");
        let unavailable = AgentBlockStatus::Unavailable {
            reason: "The storage folder is not available.".into(),
        };
        assert_eq!(status(Some(&gone)), unavailable);
        assert_eq!(
            add(Some(&gone)),
            Err("The storage folder is not available.".into())
        );
        assert_eq!(
            status(None),
            AgentBlockStatus::Unavailable {
                reason: "No storage folder is set.".into()
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_write_names_the_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        // CLAUDE.md links to a file in a folder that Longhand cannot write.
        let locked = TempDir::new().unwrap();
        write(locked.path(), "rules.md", "Text.\n");
        std::os::unix::fs::symlink(locked.path().join("rules.md"), root.join(CLAUDE_FILE)).unwrap();
        std::fs::set_permissions(locked.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

        let result = add(Some(root));

        std::fs::set_permissions(locked.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
        let error = result.unwrap_err();
        assert!(
            error.starts_with("Longhand can’t write CLAUDE.md"),
            "{error}"
        );
    }

    #[test]
    fn the_block_names_the_skill_and_the_install_steps_of_the_readme() {
        let readme = include_str!("../../../README.md");
        let block = block_text("\n");
        for step in [
            "/plugin marketplace add twistymaze/longhand",
            "/plugin install longhand@longhand",
            "codex plugin marketplace add twistymaze/longhand",
            "codex plugin add longhand@longhand",
            "npx skills add twistymaze/longhand --skill longhand-notes -g",
        ] {
            assert!(block.contains(step), "block: {step}");
            assert!(readme.contains(step), "README: {step}");
        }
        assert!(readme.contains("## Use your notes with an AI agent"));
        let skill = include_str!("../../../plugins/longhand/skills/longhand-notes/SKILL.md");
        assert!(skill.contains("name: longhand-notes"));
    }
}
