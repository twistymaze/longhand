//! `longhand.json`: Longhand's records for a note (R1, KTD1).
//!
//! The file is a JSON object with a `layout` number. The `.md` files of the
//! note folder hold only user data and text that the user may edit. The
//! output is deterministic: sorted keys, LF line ends and no write time, so
//! two computers that write the same records write the same bytes.

use std::collections::BTreeMap;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::folder::{
    FolderError, PendingType, ResultFlag, SegmentRecord, SpeakerCount, SummaryMeta,
};
use crate::jobs::Title;
use crate::recognition::ArtifactReference;

/// The name of the file in each note folder. It has no leading dot,
/// because the folder watcher and the scan skip such names.
pub const NOTE_RECORD_FILE: &str = "longhand.json";

/// The layout that this build writes.
pub const LAYOUT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRecord {
    /// The layout of the file. A reader that finds a larger number does
    /// not use the records (see [`NoteRecordRead::Newer`]).
    pub layout: u32,
    /// The folder identity of the note.
    pub session_id: String,
    pub started: DateTime<FixedOffset>,
    /// The title that Longhand last wrote into `session.md`, with its
    /// source (R5, KTD5). A `session.md` title that differs from it is the
    /// user's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub written_title: Option<Title>,
    /// A type change that waits for its move (KTD2, KTD3 of the note types
    /// and tags plan).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_type: Option<PendingType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_count: Option<SpeakerCount>,
    /// The recognition model that made the transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_artifact: Option<ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_audio: Option<BTreeMap<String, String>>,
    /// The number of speakers that processing found.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_speakers: Option<u32>,
    /// The fingerprint of `transcript.md` when this file was written (R2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_fingerprint: Option<String>,
    /// The fingerprint of `summary.md` when this file was written (R2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_fingerprint: Option<String>,
    /// Why the note has no result yet. `None` once a result is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultFlag>,
    /// Empty for a note with no audio.
    #[serde(default)]
    pub segments: Vec<SegmentRecord>,
    /// The data of the summary: its status, endpoint host, model and
    /// prompt. `None` until a summary is saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<SummaryMeta>,
}

/// The result of a parse of `longhand.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteRecordRead {
    /// A file in [`LAYOUT`].
    Current(Box<NoteRecord>),
    /// A file that a newer build wrote. This build does not read its
    /// records and does not write the file.
    Newer { layout: u32 },
}

/// The part of the file that tells its layout.
#[derive(Deserialize)]
struct LayoutOnly {
    layout: u32,
}

impl NoteRecord {
    /// The records of a note with only the required fields.
    pub fn new(session_id: impl Into<String>, started: DateTime<FixedOffset>) -> Self {
        Self {
            layout: LAYOUT,
            session_id: session_id.into(),
            started,
            written_title: None,
            pending_type: None,
            speaker_count: None,
            asr_model: None,
            recognition_snapshot: None,
            recognition_artifact: None,
            recognition_audio: None,
            found_speakers: None,
            transcript_fingerprint: None,
            summary_fingerprint: None,
            result: None,
            segments: Vec::new(),
            summary: None,
        }
    }

    /// The bytes of the file as text: keys sorted at each level, two
    /// spaces of indent, LF line ends and a final LF.
    pub fn render(&self) -> String {
        let value = serde_json::to_value(self).expect("note record serializes");
        let mut out = serde_json::to_string_pretty(&sorted(value)).expect("note record serializes");
        out.push('\n');
        out
    }

    /// Parse the text of a `longhand.json`. An error gives the line and the
    /// column. A file with a layout above [`LAYOUT`] gives
    /// [`NoteRecordRead::Newer`].
    pub fn parse(text: &str) -> Result<NoteRecordRead, FolderError> {
        let error = match serde_json::from_str::<NoteRecord>(text) {
            Ok(record) if record.layout > LAYOUT => {
                return Ok(NoteRecordRead::Newer {
                    layout: record.layout,
                });
            }
            Ok(record) => {
                check_session_id(NOTE_RECORD_FILE, &record.session_id)?;
                return Ok(NoteRecordRead::Current(Box::new(record)));
            }
            Err(e) => e,
        };
        // A newer layout can have other fields. A file that has no valid
        // layout gives the error of the layout.
        let LayoutOnly { layout } = serde_json::from_str(text).map_err(json_error)?;
        if layout > LAYOUT {
            return Ok(NoteRecordRead::Newer { layout });
        }
        Err(json_error(error))
    }
}

impl NoteRecordRead {
    /// The records of a file in [`LAYOUT`]. `None` for a file that a newer
    /// build wrote.
    pub fn current(self) -> Option<NoteRecord> {
        match self {
            Self::Current(record) => Some(*record),
            Self::Newer { .. } => None,
        }
    }
}

/// The `session_id` in the `longhand.json` text `text`, when the file
/// parses. A file that a newer build wrote gives its `session_id` when it
/// has one as text.
pub fn session_id_of(text: &str) -> Option<String> {
    match NoteRecord::parse(text).ok()? {
        NoteRecordRead::Current(record) => Some(record.session_id),
        NoteRecordRead::Newer { .. } => {
            let value: serde_json::Value = serde_json::from_str(text).ok()?;
            value
                .get("session_id")?
                .as_str()
                .filter(|id| valid_session_id(id))
                .map(str::to_string)
        }
    }
}

/// Whether `id` is a note id in the form that Longhand writes: 1 to 64
/// characters, each a lowercase ASCII letter, a digit, `-` or `_`. The id
/// comes from a synced file, and the app uses it as a folder name under
/// the working root. So a path, `..`, an empty id and uppercase (a file
/// system that ignores case gives two such ids one folder) are refused.
pub fn valid_session_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// An error for the file `file` when `id` is not a valid note id
/// ([`valid_session_id`]).
pub(crate) fn check_session_id(file: &'static str, id: &str) -> Result<(), FolderError> {
    if valid_session_id(id) {
        return Ok(());
    }
    // `Yaml` is the error with a free text. It also serves a JSON file.
    Err(FolderError::Yaml {
        file,
        message: format!("session_id {id:?} is not a valid note id"),
    })
}

fn json_error(e: serde_json::Error) -> FolderError {
    FolderError::At {
        file: NOTE_RECORD_FILE,
        line: e.line(),
        column: e.column(),
        message: e.to_string(),
    }
}

/// `value` with the keys of each object in sorted order. The order does not
/// depend on the `preserve_order` feature of `serde_json`.
fn sorted(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<_> = map.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(entries.into_iter().map(|(k, v)| (k, sorted(v))).collect())
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sorted).collect())
        }
        other => other,
    }
}
