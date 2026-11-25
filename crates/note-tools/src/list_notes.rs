//! List notes (R2, R3): the notes of one storage folder, with filters by
//! note type, tags, date range, title, busy state and keyword.

use std::path::Path;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use session_core::folder::Summary;

use crate::storage::{
    GENERAL, NOTES_FILE, Note, ProblemFolder, SUMMARY_FILE, Storage, TRANSCRIPT_FILE,
};
use crate::{Tool, ToolError};

/// The number of notes in one result when the input sets no limit.
pub const DEFAULT_LIMIT: u32 = 50;
/// The largest limit that the input can set.
pub const MAX_LIMIT: u32 = 200;

/// The input of list notes. Every filter is optional, and a note must
/// match all the filters that the input gives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListNotesInput {
    /// Only notes of this note type. "General" gives the notes that are not
    /// in a type folder. Case does not count.
    #[serde(default)]
    pub note_type: Option<String>,
    /// Only notes with each of these tags. Case and the spaces around `::`
    /// do not count. A tag that ends in `::`, for example "Person::",
    /// matches each tag with that scope.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Only notes that started on this day or later, as YYYY-MM-DD.
    #[serde(default)]
    pub since: Option<String>,
    /// Only notes that started on this day or earlier, as YYYY-MM-DD.
    #[serde(default)]
    pub until: Option<String>,
    /// Only notes whose title contains this text. Case does not count.
    #[serde(default)]
    pub title: Option<String>,
    /// Only notes that contain this text in the title, the tags, the manual
    /// notes, the summary or the transcript. Case does not count. Each
    /// result says where the text is.
    #[serde(default)]
    pub keyword: Option<String>,
    /// Only busy notes (true) or only notes that are not busy (false). A
    /// note is busy when a computer records or processes it, when its
    /// recording is not saved yet, or when a move to another type waits.
    #[serde(default)]
    pub busy: Option<bool>,
    /// The most notes in the result, from 1 to 200. The default is 50. The
    /// newest notes come first.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// A part of a note where the keyword is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchPart {
    Title,
    Tags,
    ManualNotes,
    Summary,
    Transcript,
}

/// One note in the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct NoteItem {
    /// The id to give to get note.
    pub note_id: String,
    pub title: Option<String>,
    pub note_type: String,
    pub tags: Vec<String>,
    /// The start time, as RFC 3339.
    pub started: String,
    /// The start day, as YYYY-MM-DD.
    pub date: String,
    pub busy: bool,
    /// Why the note is busy.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub busy_reasons: Vec<String>,
    /// Other folders with the same note id. Get note refuses such an id.
    #[serde(skip_serializing_if = "is_zero")]
    pub duplicate_folders: usize,
    /// Where the keyword is. Only with a keyword.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keyword_in: Option<Vec<MatchPart>>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

/// The result of list notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ListNotesOutput {
    pub notes: Vec<NoteItem>,
    /// The number of notes that match, also those over the limit.
    pub matching: usize,
    /// True when more notes match than the result holds.
    pub truncated: bool,
    /// True when the keyword search stopped at its time limit, so that
    /// some notes were not searched.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub keyword_search_incomplete: bool,
    /// Folders that look like note folders but give no note, for example a
    /// note that a newer Longhand wrote.
    pub problems: Vec<ProblemFolder>,
}

/// The list notes tool.
#[derive(Debug, Clone)]
pub struct ListNotes {
    /// The longest time that one keyword search reads files.
    pub keyword_time: Duration,
}

impl Default for ListNotes {
    fn default() -> Self {
        Self {
            keyword_time: Duration::from_secs(20),
        }
    }
}

/// The key of a tag: two tags with one key are one tag.
pub fn tag_key(text: &str) -> String {
    text.split("::")
        .map(str::trim)
        .collect::<Vec<_>>()
        .join("::")
        .to_lowercase()
}

fn has_tag(tags: &[String], wanted: &str) -> bool {
    let key = tag_key(wanted);
    if key.ends_with("::") {
        tags.iter().any(|tag| tag_key(tag).starts_with(&key))
    } else {
        tags.iter().any(|tag| tag_key(tag) == key)
    }
}

fn day(text: &Option<String>, name: &str) -> Result<Option<NaiveDate>, ToolError> {
    text.as_deref()
        .map(|t| {
            NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d")
                .map_err(|_| ToolError::invalid(format!("{name} must be a day as YYYY-MM-DD")))
        })
        .transpose()
}

/// The parts of `note` that contain `needle`, which is lowercase.
fn keyword_parts(root: &Path, note: &Note, needle: &str) -> Vec<MatchPart> {
    let has = |text: &str| text.to_lowercase().contains(needle);
    let mut parts = Vec::new();
    if note.doc.title.as_deref().is_some_and(has) {
        parts.push(MatchPart::Title);
    }
    if note.doc.tags.iter().any(|tag| has(tag)) {
        parts.push(MatchPart::Tags);
    }
    if let (_, Some(text)) = note.read_part(root, NOTES_FILE)
        && has(&text)
    {
        parts.push(MatchPart::ManualNotes);
    }
    if let (_, Some(text)) = note.read_part(root, SUMMARY_FILE)
        && has(Summary::body_of(&text))
    {
        parts.push(MatchPart::Summary);
    }
    if let (_, Some(text)) = note.read_part(root, TRANSCRIPT_FILE)
        && has(&text)
    {
        parts.push(MatchPart::Transcript);
    }
    parts
}

fn item(note: &Note, keyword_in: Option<Vec<MatchPart>>) -> NoteItem {
    NoteItem {
        note_id: note.id.clone(),
        title: note.doc.title.clone(),
        note_type: note.type_name(),
        tags: note.doc.tags.clone(),
        started: note.started().to_rfc3339(),
        date: note.started().date_naive().format("%Y-%m-%d").to_string(),
        busy: !note.busy.is_empty(),
        busy_reasons: note.busy.clone(),
        duplicate_folders: note.duplicates,
        keyword_in,
    }
}

impl Tool for ListNotes {
    type Input = ListNotesInput;
    type Output = ListNotesOutput;

    const NAME: &'static str = "list_notes";

    fn description(&self) -> String {
        "List the notes in the Longhand storage folder, newest first. Filter by note type, \
         tags, a date range, title text, busy state or a keyword. A keyword result says \
         whether the keyword is in the title, tags, manual notes, summary or transcript. \
         Use get_note with a note_id to read a note."
            .into()
    }

    fn call(&self, root: &Path, input: ListNotesInput) -> Result<ListNotesOutput, ToolError> {
        let limit = input.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(ToolError::invalid(format!(
                "limit must be from 1 to {MAX_LIMIT}"
            )));
        }
        let since = day(&input.since, "since")?;
        let until = day(&input.until, "until")?;
        let keyword = input
            .keyword
            .as_deref()
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty());
        let title = input.title.as_deref().map(str::to_lowercase);
        let storage = Storage::open(root)?;

        let started = Instant::now();
        let mut incomplete = false;
        let mut notes = Vec::new();
        let mut matching = 0;
        for note in &storage.notes {
            if let Some(wanted) = &input.note_type
                && !note
                    .note_type
                    .as_deref()
                    .unwrap_or(GENERAL)
                    .trim()
                    .eq_ignore_ascii_case(wanted.trim())
            {
                continue;
            }
            if !input.tags.iter().all(|tag| has_tag(&note.doc.tags, tag)) {
                continue;
            }
            let date = note.started().date_naive();
            if since.is_some_and(|since| date < since) || until.is_some_and(|until| date > until) {
                continue;
            }
            if let Some(title) = &title
                && !note
                    .doc
                    .title
                    .as_deref()
                    .is_some_and(|t| t.to_lowercase().contains(title.as_str()))
            {
                continue;
            }
            if let Some(busy) = input.busy
                && busy == note.busy.is_empty()
            {
                continue;
            }
            let keyword_in = match &keyword {
                None => None,
                Some(_) if incomplete => continue,
                Some(needle) => {
                    if started.elapsed() > self.keyword_time {
                        incomplete = true;
                        continue;
                    }
                    let parts = keyword_parts(&storage.root, note, needle);
                    if parts.is_empty() {
                        continue;
                    }
                    Some(parts)
                }
            };
            matching += 1;
            if notes.len() < limit as usize {
                notes.push(item(note, keyword_in));
            }
        }
        Ok(ListNotesOutput {
            truncated: matching > notes.len(),
            notes,
            matching,
            keyword_search_incomplete: incomplete,
            problems: storage.problems,
        })
    }
}
