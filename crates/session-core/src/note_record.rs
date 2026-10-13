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
        let LayoutOnly { layout } = serde_json::from_str(text).map_err(json_error)?;
        if layout > LAYOUT {
            return Ok(NoteRecordRead::Newer { layout });
        }
        serde_json::from_str(text)
            .map(|record| NoteRecordRead::Current(Box::new(record)))
            .map_err(json_error)
    }
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
