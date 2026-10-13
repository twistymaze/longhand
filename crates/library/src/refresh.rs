//! The refresh step (KTD3): bring one row up to date from its note folder.
//!
//! The folder is the authority for a saved note on every computer (KD4).
//! The refresh reads `longhand.json`, `session.md`, `notes.md` and the fingerprints of
//! `transcript.md` and `summary.md`, and asks whether each segment's
//! removal marker exists. It updates the title, title source, recognition
//! model, speakers found, speaker count, segments, removed flags, notes,
//! tags and pending type of the row. It never writes to the folder. It is idempotent: a row that
//! already matches its folder changes nothing.
//!
//! A `transcript.md` with another fingerprint than the one the row stores
//! was written by another computer. The refresh then removes this
//! computer's stored recognition results for the note (KTD12, R18).
//!
//! The row takes the result snapshot of the records only when both
//! documents have the fingerprints that the records hold (KTD10 of the
//! result copies plan). While the sync delivers the documents, the row
//! keeps its snapshot.
//!
//! The refresh does not check claims: the library reconcile refreshes every
//! part only for a Saved row that no stage has claimed, and only the notes
//! ([`Scope::Notes`]) for any other row with a home folder (KTD6). A writer
//! calls it on the row that it holds.

use std::path::{Path, PathBuf};
use std::time::Duration;

use session_core::folder::{PendingType, SessionDoc, fingerprint};
use session_core::note_folder::Layout;
use session_core::{Segment, SpeakerCount, Store, StoreError, Title, TitleSource};

use crate::scan::{NoteFolder, NoteRead, Read, read_note, read_notes, read_timed, removed_audio};

/// What the refresh reads from a note folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderContents {
    /// The records and the user fields of the note
    /// ([`session_core::note_folder::NoteFiles::doc`]).
    pub doc: SessionDoc,
    pub layout: Layout,
    /// The title that Longhand last wrote into `session.md` (KTD5). `None`
    /// in the old layout.
    pub written_title: Option<Title>,
    /// The title of `session.md` has differed from `written_title` for
    /// about one minute: it is a user title (KTD5). Until then, the row
    /// keeps its title. The reconcile sets it ([`crate::reconcile`]).
    pub title_settled: bool,
    /// The text of `notes.md`. `None` while the sync has not delivered it:
    /// the row keeps its notes.
    pub notes: Option<String>,
    /// The fingerprint of `transcript.md`. `None` while the sync has not
    /// delivered it.
    pub transcript_fingerprint: Option<String>,
    /// The fingerprint of `summary.md`. `None` while the sync has not
    /// delivered it.
    pub summary_fingerprint: Option<String>,
    /// The audio files of the segments whose removal marker is in the
    /// folder, sorted (KTD9 of the audio removal plan).
    pub removed_audio: Vec<String>,
}

impl FolderContents {
    /// Read the documents of the note folder `folder`. Each read stops
    /// after `timeout`.
    pub fn read(folder: &Path, timeout: Duration) -> Result<Self, RefreshError> {
        let files = match read_note(folder, timeout) {
            NoteRead::Note(files) => *files,
            NoteRead::NotANote => return Err(RefreshError::NoNote(folder.to_path_buf())),
            NoteRead::WaitingForRecord => {
                return Err(RefreshError::WaitingForRecord(folder.to_path_buf()));
            }
            NoteRead::Unreadable(reason) => {
                return Err(RefreshError::Unreadable {
                    folder: folder.to_path_buf(),
                    reason,
                });
            }
            NoteRead::Invalid(invalid) => {
                return Err(RefreshError::Invalid {
                    folder: folder.to_path_buf(),
                    reason: invalid.reason,
                });
            }
        };
        let layout = files.layout;
        let written_title = files.written_title().cloned();
        let doc = files.doc;
        let document_fingerprint = |name: &str| match read_timed(folder.join(name), timeout) {
            Read::Bytes(bytes) => Some(fingerprint(&bytes)),
            Read::Missing | Read::Failed(_) => None,
        };
        Ok(Self {
            removed_audio: removed_audio(folder, &doc),
            notes: read_notes(folder, timeout),
            transcript_fingerprint: document_fingerprint("transcript.md"),
            summary_fingerprint: document_fingerprint("summary.md"),
            doc,
            layout,
            written_title,
            title_settled: false,
        })
    }

    /// The contents that a scan read from the note's folder.
    pub fn from_scan(note: &NoteFolder) -> Self {
        Self {
            doc: note.doc.clone(),
            layout: note.layout,
            written_title: note.written_title.clone(),
            title_settled: false,
            notes: note.notes.clone(),
            transcript_fingerprint: note.transcript_fingerprint.clone(),
            summary_fingerprint: note.summary_fingerprint.clone(),
            removed_audio: note.removed_audio.clone(),
        }
    }

    /// Whether `transcript.md` and `summary.md` exist and have the
    /// fingerprints that `session.md` records. A `session.md` that records
    /// no fingerprint accepts any file, as the scan does.
    fn result_matches(&self) -> bool {
        let matches = |recorded: &Option<String>, actual: &Option<String>| {
            actual
                .as_ref()
                .is_some_and(|actual| recorded.as_ref().is_none_or(|r| r == actual))
        };
        matches(
            &self.doc.transcript_fingerprint,
            &self.transcript_fingerprint,
        ) && matches(&self.doc.summary_fingerprint, &self.summary_fingerprint)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("the folder {} has no session.md and no longhand.json", .0.display())]
    NoNote(PathBuf),
    #[error("the folder {} waits for its longhand.json", .0.display())]
    WaitingForRecord(PathBuf),
    #[error("the note files in {} cannot be read: {reason}", folder.display())]
    Unreadable { folder: PathBuf, reason: String },
    #[error("the note files in {} are not valid: {reason}", folder.display())]
    Invalid { folder: PathBuf, reason: String },
    #[error("the folder holds note {found}, not note {expected}")]
    OtherNote { expected: String, found: String },
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// What a refresh changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Refresh {
    /// The row changed in a way that Home or the note page shows.
    pub changed: bool,
    /// Another computer wrote `transcript.md`, so this computer's stored
    /// recognition results for the note are removed (KTD12). A later Resume
    /// takes the earlier segments' text from `transcript.md`.
    pub cleared_recognition_results: bool,
}

/// Which parts of the row a refresh takes from the folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every part, and the transcript fingerprint (KTD12). For a Saved row
    /// that no stage has claimed.
    All,
    /// Only a title that a user set, the notes and the user fields (the
    /// tags and the pending type, KTD2). For a writer inside
    /// its own run (Resume, Transcribe Again): the run's recognition
    /// results, model, speakers found, segments and title proposal belong
    /// to the run, and its new transcript and summary replace the folder's
    /// (KTD3).
    TitleAndNotes,
    /// Only the notes and the user fields. For a row that records,
    /// processes or waits on this computer, so that a notes, tag or type
    /// edit from another computer shows during that work (R22, KTD6, R18).
    /// The work owns the rest of the row.
    Notes,
}

/// Read the note folder `folder` and bring the row `id` up to date from it.
pub fn refresh(
    store: &mut Store,
    id: &str,
    folder: &Path,
    timeout: Duration,
) -> Result<Refresh, RefreshError> {
    refresh_scoped(store, id, folder, timeout, Scope::All)
}

/// [`refresh`] of the parts of the row that `scope` names.
pub fn refresh_scoped(
    store: &mut Store,
    id: &str,
    folder: &Path,
    timeout: Duration,
    scope: Scope,
) -> Result<Refresh, RefreshError> {
    let contents = FolderContents::read(folder, timeout)?;
    apply_scoped(store, id, &contents, scope)
}

/// Bring the row `id` up to date from `contents`, which were read from its
/// folder.
pub fn apply(
    store: &mut Store,
    id: &str,
    contents: &FolderContents,
) -> Result<Refresh, RefreshError> {
    apply_scoped(store, id, contents, Scope::All)
}

/// [`apply`] to the parts of the row that `scope` names.
pub fn apply_scoped(
    store: &mut Store,
    id: &str,
    contents: &FolderContents,
    scope: Scope,
) -> Result<Refresh, RefreshError> {
    if contents.doc.session_id != id {
        return Err(RefreshError::OtherNote {
            expected: id.to_string(),
            found: contents.doc.session_id.clone(),
        });
    }
    // While the note waits for the sync, the speaker data of a new
    // `session.md` must not join an old transcript (KTD10).
    if scope == Scope::All && contents.result_matches() {
        store.promote_result_snapshot(id, contents.doc.recognition_snapshot.as_ref())?;
    }
    let updates = scoped(updates(store, id, contents)?, scope);
    Ok(write(store, id, updates)?)
}

/// The part of `updates` that `scope` names.
fn scoped(updates: Updates, scope: Scope) -> Updates {
    match scope {
        Scope::All => updates,
        // The run proposes its own title with its new summary, so only a
        // title that a user set in the folder replaces the row's.
        Scope::TitleAndNotes => Updates {
            title: updates.title.filter(|title| {
                title
                    .as_ref()
                    .is_some_and(|title| title.source == TitleSource::User)
            }),
            notes: updates.notes,
            tags: updates.tags,
            pending_type: updates.pending_type,
            ..Updates::default()
        },
        Scope::Notes => Updates {
            notes: updates.notes,
            tags: updates.tags,
            pending_type: updates.pending_type,
            ..Updates::default()
        },
    }
}

/// Whether [`apply_scoped`] with `contents` and `scope` would write to the
/// row `id`.
pub(crate) fn differs(
    store: &Store,
    id: &str,
    contents: &FolderContents,
    scope: Scope,
) -> Result<bool, StoreError> {
    Ok(!scoped(updates(store, id, contents)?, scope).is_empty())
}

/// How the stored transcript fingerprint changes.
enum TranscriptChange {
    /// The row has no fingerprint: record the one of the folder. The row
    /// was saved before fingerprints were stored, or had no transcript.
    Record(String),
    /// Another computer wrote `transcript.md`.
    Replace(String),
}

/// The writes that bring a row up to date with its folder. Each `Some` is
/// a value that differs from the stored one.
#[derive(Default)]
struct Updates {
    title: Option<Option<Title>>,
    speaker_count: Option<Option<SpeakerCount>>,
    asr_model: Option<String>,
    found_speakers: Option<u32>,
    segments: Vec<Segment>,
    stereo_from: Option<u32>,
    /// The removed flags of the segments, when the folder's removal
    /// markers differ from them.
    removed_audio: Option<Vec<String>>,
    notes: Option<String>,
    tags: Option<Vec<String>>,
    pending_type: Option<Option<PendingType>>,
    transcript: Option<TranscriptChange>,
}

impl Updates {
    fn is_empty(&self) -> bool {
        self.transcript.is_none() && !self.changes_row()
    }

    /// Whether the updates change what Home or the note page shows.
    fn changes_row(&self) -> bool {
        self.title.is_some()
            || self.speaker_count.is_some()
            || self.asr_model.is_some()
            || self.found_speakers.is_some()
            || !self.segments.is_empty()
            || self.stereo_from.is_some()
            || self.removed_audio.is_some()
            || self.notes.is_some()
            || self.tags.is_some()
            || self.pending_type.is_some()
            || matches!(self.transcript, Some(TranscriptChange::Replace(_)))
    }
}

fn updates(store: &Store, id: &str, contents: &FolderContents) -> Result<Updates, StoreError> {
    let record = store
        .session(id)?
        .ok_or_else(|| StoreError::NoSuchSession(id.to_string()))?;
    let doc = &contents.doc;
    let mut updates = Updates::default();

    let title = folder_title(contents, record.title.as_ref());
    if title != record.title {
        updates.title = Some(title);
    }
    if doc.speaker_count != record.speaker_count {
        updates.speaker_count = Some(doc.speaker_count);
    }
    // A `session.md` from an older Longhand has no model and no speakers
    // found: the row keeps its values.
    if let Some(model) = &doc.asr_model
        && record.asr_model.as_ref() != Some(model)
    {
        updates.asr_model = Some(model.clone());
    }
    if let Some(found) = doc.found_speakers
        && record.found_speakers != Some(found)
    {
        updates.found_speakers = Some(found);
    }

    // A segment that the row has and the folder does not stays: a Resume
    // on this computer adds its segment to the row before it saves.
    let stored = store.segments(id)?;
    updates.segments = doc
        .timeline_segments()
        .into_iter()
        .filter(|segment| !stored.contains(segment))
        .collect();
    if let Some(index) = doc.stereo_from()
        && store.stereo_from(id)?.is_none()
    {
        updates.stereo_from = Some(index);
    }
    // The flags follow the folder's removal markers, so the row changes
    // when a marker arrives, and an open note page reads the note again.
    if contents.removed_audio != store.removed_audio(id)? {
        updates.removed_audio = Some(contents.removed_audio.clone());
    }

    if let Some(notes) = &contents.notes
        && *notes != store.notes(id)?
    {
        updates.notes = Some(notes.clone());
    }
    if doc.tags != record.tags {
        updates.tags = Some(doc.tags.clone());
    }
    if doc.pending_type != record.pending_type {
        updates.pending_type = Some(doc.pending_type.clone());
    }

    if let Some(actual) = &contents.transcript_fingerprint {
        updates.transcript = match store.transcript_fingerprint(id)? {
            None => Some(TranscriptChange::Record(actual.clone())),
            Some(stored) if stored != *actual => Some(TranscriptChange::Replace(actual.clone())),
            Some(_) => None,
        };
    }
    Ok(updates)
}

/// The title that the folder gives.
///
/// In the new layout, a `session.md` title that equals the title that
/// Longhand last wrote has the source of that title. A title that differs
/// is the user's once it has differed for about one minute
/// ([`FolderContents::title_settled`], KTD5). Until then, a sync can still
/// deliver the other file of a title write, and the row keeps its title.
///
/// In the old layout, a `session.md` from an older Longhand can have a
/// title and no source: when the title is the stored one, the row keeps
/// its source (KTD3). Otherwise a title with no source is the user's (R5).
fn folder_title(contents: &FolderContents, stored: Option<&Title>) -> Option<Title> {
    let doc = &contents.doc;
    // The store keeps a title trimmed, and keeps no empty title.
    let text = doc
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if contents.layout == Layout::New {
        let written = contents.written_title.as_ref();
        if doc.title.as_deref() == written.map(|w| w.text.as_str()) {
            return text.map(|text| Title {
                text: text.to_string(),
                source: written.map_or(TitleSource::User, |w| w.source),
            });
        }
        if !contents.title_settled {
            return stored.cloned();
        }
        return text.map(|text| Title {
            text: text.to_string(),
            source: TitleSource::User,
        });
    }
    let text = text?;
    let source = match (doc.title_source, stored) {
        (Some(source), _) => source,
        (None, Some(stored)) if stored.text == text => stored.source,
        (None, _) => TitleSource::User,
    };
    Some(Title {
        text: text.to_string(),
        source,
    })
}

fn write(store: &mut Store, id: &str, updates: Updates) -> Result<Refresh, StoreError> {
    let changed = updates.changes_row();
    if let Some(title) = updates.title {
        write_title(store, id, title)?;
    }
    if let Some(count) = updates.speaker_count {
        store.set_speaker_count(id, count)?;
    }
    if let Some(model) = &updates.asr_model {
        store.set_asr_model(id, model)?;
    }
    if let Some(found) = updates.found_speakers {
        store.set_found_speakers(id, Some(found))?;
    }
    for segment in &updates.segments {
        store.put_segment(id, segment)?;
    }
    if let Some(index) = updates.stereo_from {
        store.mark_stereo_from(id, index)?;
    }
    // After the segments, so that a new segment gets its flag.
    if let Some(audio) = &updates.removed_audio {
        store.set_removed_audio(id, audio)?;
    }
    if let Some(notes) = &updates.notes {
        store.set_notes(id, notes)?;
    }
    if let Some(tags) = &updates.tags {
        store.set_tags(id, tags)?;
    }
    if let Some(pending) = &updates.pending_type {
        store.set_pending_type(id, pending.as_ref())?;
    }
    let mut cleared_recognition_results = false;
    match &updates.transcript {
        None => {}
        Some(TranscriptChange::Record(fingerprint)) => {
            store.set_transcript_fingerprint(id, Some(fingerprint))?;
        }
        Some(TranscriptChange::Replace(fingerprint)) => {
            log::info!("library: another computer wrote the transcript of {id}");
            store.clear_recognition_results(id)?;
            store.set_transcript_fingerprint(id, Some(fingerprint))?;
            cleared_recognition_results = true;
        }
    }
    Ok(Refresh {
        changed,
        cleared_recognition_results,
    })
}

/// Store `title` with its source as it is. The store refuses a proposed
/// title over the user's title (R31), so that title is cleared first: the
/// folder is the authority (KD4).
fn write_title(store: &Store, id: &str, title: Option<Title>) -> Result<(), StoreError> {
    match title {
        None => {
            store.set_title(id, "", TitleSource::User)?;
        }
        Some(Title {
            text,
            source: TitleSource::User,
        }) => {
            store.set_title(id, &text, TitleSource::User)?;
        }
        Some(Title {
            text,
            source: TitleSource::Proposed,
        }) => {
            store.set_title(id, "", TitleSource::User)?;
            store.set_title(id, &text, TitleSource::Proposed)?;
        }
    }
    Ok(())
}
