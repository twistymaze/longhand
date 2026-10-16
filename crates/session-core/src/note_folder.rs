//! The read of a note folder in each layout (KTD1, KTD6).
//!
//! In the new layout, `longhand.json` holds Longhand's records and the
//! folder identity, and `session.md` holds only the title and the tags.
//! In the old layout, `session.md` holds `session_id` and all the records.
//! A `session.md` that holds `session_id` is the old layout, also when a
//! `longhand.json` is in the folder: an older build wrote the records
//! there last. Before the conversion, the records of an old note come from
//! `session.md`, and its summary data from the `summary.md` frontmatter.

use std::path::Path;

use crate::folder::{FolderError, SessionDoc, SessionMd, fingerprint};
use crate::jobs::{Title, TitleSource};
use crate::note_record::{
    LAYOUT, NOTE_RECORD_FILE, NoteRecord, NoteRecordRead, check_session_id,
};

/// The name of the user file of a note folder.
pub const SESSION_FILE: &str = "session.md";

/// The layout of a note folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `longhand.json` holds the records.
    New,
    /// `session.md` holds `session_id` and the records (KTD6).
    Old,
}

/// What a note folder holds, in either layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteFiles {
    pub layout: Layout,
    /// The records and the user fields as one document. The title and the
    /// tags come from `session.md`. A tag key that is in the file two times
    /// counts one time (R8). In the new layout, the title has the source of
    /// the last-written title when the texts are equal, and is the user's
    /// title when they differ.
    pub doc: SessionDoc,
    /// The records. In the old layout, they come from `session.md`, with
    /// its title as the last-written title and no summary data.
    pub record: NoteRecord,
}

/// Why a folder gives no [`NoteFiles`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteFilesError {
    /// The folder has no `session.md` and no `longhand.json`.
    #[error("the folder is not a note folder")]
    NotANote,
    /// A new-layout `session.md` and no `longhand.json`: the sync has not
    /// delivered the records yet (KTD6).
    #[error("{NOTE_RECORD_FILE} is not in the folder yet")]
    WaitingForRecord,
    /// A file does not parse.
    #[error("{0}")]
    Invalid(FolderError),
    /// A newer build wrote `longhand.json`. This build does not use or
    /// write its records.
    #[error(
        "{NOTE_RECORD_FILE} has layout {layout}, and this version of Longhand reads layout {LAYOUT}"
    )]
    Newer { layout: u32 },
}

impl NoteFiles {
    /// Read the texts of `session.md` and `longhand.json` of one folder.
    /// `None` is a file that is not there.
    pub fn parse(session_md: Option<&str>, record: Option<&str>) -> Result<Self, NoteFilesError> {
        if let Some(text) = session_md
            && holds_session_id(text)
        {
            // The parse of the user fields gives the line of a YAML error.
            let doc = SessionDoc::parse(text)
                .map_err(|e| NoteFilesError::Invalid(SessionMd::parse(text).err().unwrap_or(e)))?;
            check_session_id(SESSION_FILE, &doc.session_id).map_err(NoteFilesError::Invalid)?;
            return Ok(Self::old(doc));
        }
        let Some(record) = record else {
            return Err(match session_md {
                // A file that does not parse is a problem, whatever its
                // layout.
                Some(text) => match SessionMd::parse(text) {
                    Ok(_) => NoteFilesError::WaitingForRecord,
                    Err(e) => NoteFilesError::Invalid(e),
                },
                None => NoteFilesError::NotANote,
            });
        };
        let record = match NoteRecord::parse(record).map_err(NoteFilesError::Invalid)? {
            NoteRecordRead::Current(record) => *record,
            NoteRecordRead::Newer { layout } => return Err(NoteFilesError::Newer { layout }),
        };
        let user = match session_md {
            Some(text) => SessionMd::parse(text).map_err(NoteFilesError::Invalid)?,
            // An agent can delete `session.md`: the last-written title
            // shows.
            None => SessionMd {
                title: record.written_title.as_ref().map(|t| t.text.clone()),
                tags: Vec::new(),
            },
        };
        Ok(Self::new(record, user))
    }

    /// Read the note folder `folder`. A folder that gives no note gives an
    /// error of the kind [`std::io::ErrorKind::Other`] with the reason, and
    /// a file that cannot be read gives its error.
    pub fn read(folder: &Path) -> std::io::Result<Self> {
        let session_md = read_optional(&folder.join(SESSION_FILE))?;
        let record = read_optional(&folder.join(NOTE_RECORD_FILE))?;
        Self::parse(session_md.as_deref(), record.as_deref())
            .map_err(|e| std::io::Error::other(e.to_string()))
    }

    /// The files of a new-layout note.
    pub fn new(record: NoteRecord, user: SessionMd) -> Self {
        let title_source = user
            .title
            .as_ref()
            .map(|title| match &record.written_title {
                Some(written) if written.text == *title => written.source,
                _ => TitleSource::User,
            });
        let doc = SessionDoc {
            session_id: record.session_id.clone(),
            started: record.started,
            title: user.title,
            title_source,
            tags: crate::tags::unique(user.tags),
            pending_type: record.pending_type.clone(),
            speaker_count: record.speaker_count,
            asr_model: record.asr_model.clone(),
            recognition_snapshot: record.recognition_snapshot.clone(),
            recognition_artifact: record.recognition_artifact.clone(),
            recognition_audio: record.recognition_audio.clone(),
            found_speakers: record.found_speakers,
            transcript_fingerprint: record.transcript_fingerprint.clone(),
            summary_fingerprint: record.summary_fingerprint.clone(),
            result: record.result,
            segments: record.segments.clone(),
        };
        Self {
            layout: Layout::New,
            doc,
            record,
        }
    }

    /// The files of an old-layout note, from its `session.md`.
    pub fn old(mut doc: SessionDoc) -> Self {
        doc.tags = crate::tags::unique(std::mem::take(&mut doc.tags));
        Self {
            layout: Layout::Old,
            record: NoteRecord::from_doc(&doc),
            doc,
        }
    }

    /// The title that Longhand last wrote. `None` in the old layout.
    pub fn written_title(&self) -> Option<&Title> {
        match self.layout {
            Layout::New => self.record.written_title.as_ref(),
            Layout::Old => None,
        }
    }

    /// Whether the title of `session.md` differs from the title that
    /// Longhand last wrote (KTD5). Always false in the old layout.
    pub fn title_changed_outside(&self) -> bool {
        self.layout == Layout::New
            && self.doc.title.as_deref()
                != self.record.written_title.as_ref().map(|t| t.text.as_str())
    }
}

/// The text of the file at `path`, or `None` when it is not there.
pub fn read_optional(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Whether the frontmatter of `text` has the key `session_id`: the file is
/// an old-layout `session.md` (KTD6). A file that does not parse does not
/// have it.
fn holds_session_id(text: &str) -> bool {
    let Some(rest) = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
    else {
        return false;
    };
    for line in rest.lines() {
        let line = line.trim_end_matches('\r');
        if line == "---" {
            return false;
        }
        if let Some(key) = line.split_once(':').map(|(key, _)| key.trim_end())
            && matches!(key, "session_id" | "\"session_id\"" | "'session_id'")
        {
            return true;
        }
    }
    false
}

impl NoteRecord {
    /// The records of an old-layout `session.md`: its title is the
    /// last-written title, and it has no summary data.
    pub fn from_doc(doc: &SessionDoc) -> Self {
        Self {
            written_title: doc.title_and_source(),
            pending_type: doc.pending_type.clone(),
            speaker_count: doc.speaker_count,
            asr_model: doc.asr_model.clone(),
            recognition_snapshot: doc.recognition_snapshot.clone(),
            recognition_artifact: doc.recognition_artifact.clone(),
            recognition_audio: doc.recognition_audio.clone(),
            found_speakers: doc.found_speakers,
            transcript_fingerprint: doc.transcript_fingerprint.clone(),
            summary_fingerprint: doc.summary_fingerprint.clone(),
            result: doc.result,
            segments: doc.segments.clone(),
            ..Self::new(doc.session_id.clone(), doc.started)
        }
    }

    /// The [`fingerprint`] of the records that a run compares (KTD4): all
    /// the records but the last-written title, the pending type and the
    /// layout. A title proposal or a type move during a run then does not
    /// make the run refuse. The value comes from the parsed records, so the
    /// text style of the file does not count.
    pub fn run_fingerprint(&self) -> String {
        let compared = Self {
            layout: 0,
            written_title: None,
            pending_type: None,
            ..self.clone()
        };
        fingerprint(compared.render().as_bytes())
    }
}

/// The value that the store records as the files of a note that this
/// computer last wrote: the fingerprints of `session.md` and
/// `longhand.json` (KTD8 of the notes without a recording plan).
pub fn written_fingerprint(session_md: &[u8], record: &[u8]) -> String {
    format!("{}+{}", fingerprint(session_md), fingerprint(record))
}
