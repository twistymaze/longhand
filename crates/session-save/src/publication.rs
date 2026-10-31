//! Publication of a new result into a note folder (KTD1 to KTD5 of the
//! result copies plan). The new result is complete in a local staging
//! folder first. The owner then writes a small record into that staging
//! folder, copies the staged files that differ into the note folder, and
//! writes `longhand.json` last (KTD2 of the safe agent edits plan). The
//! note folder keeps no copies of earlier results.
//!
//! The record never syncs. Its presence means "unfinished publication":
//! after a crash, a rerun reads it and only rolls forward. The rerun
//! writes the staged files that differ and then `longhand.json`, whatever
//! the note folder holds, unless the result fields of the note changed on
//! another computer. Then the owner drops its result (KTD4).
//!
//! A publication never changes the title or the tags in `session.md`: the
//! title and tag writers own them. It makes `session.md` only when the
//! folder has none, before `longhand.json`. A note in the old layout gets
//! no publication (KTD6): its records are in `session.md` until the
//! conversion.
use crate::home_write;
use crate::note_files;
use serde::{Deserialize, Serialize};
use session_core::SpeakerCount;
use session_core::folder::{self, fingerprint};
use session_core::note_folder::SESSION_FILE;
use session_core::note_record::{LAYOUT, NOTE_RECORD_FILE, NoteRecord};
use session_core::recognition::{ARTIFACT_FILE, ArtifactReference};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// The name of the record in the local staging folder.
pub const RECORD: &str = "publication.json";

/// The error text for an action that changes the records of a note in the
/// old layout (KTD6).
pub const OLD_LAYOUT: &str =
    "An older version of Longhand saved this note. Convert your notes to use this action.";

/// The files that a publication writes into the note folder before
/// `longhand.json` (KTD5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteSet {
    /// Save, Resume and Transcribe Again: `recognition.jsonl` when the
    /// result has one, `transcript.md` and `summary.md`, in that order.
    /// Most sync clients upload in write order, so `recognition.jsonl`
    /// does not arrive after `longhand.json`.
    Result,
    /// Regenerate Summary: `summary.md` only. The recognition references
    /// in `longhand.json` come from the note folder at write time, and go
    /// when its `recognition.jsonl` is not there. The publication never
    /// writes `recognition.jsonl` back.
    Summary,
}

impl WriteSet {
    fn files(self) -> &'static [&'static str] {
        match self {
            Self::Result => &[ARTIFACT_FILE, "transcript.md", "summary.md"],
            Self::Summary => &["summary.md"],
        }
    }
}

/// The records that tell which result the note folder holds (KTD2). A
/// publication compares only these fields, so a title write or a rename
/// between two writes is not a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultFields {
    /// An older build wrote the key `session_id`.
    #[serde(alias = "session_id")]
    pub note_id: String,
    pub transcript_fingerprint: Option<String>,
    pub summary_fingerprint: Option<String>,
    pub recognition_artifact: Option<ArtifactReference>,
    pub recognition_audio: Option<BTreeMap<String, String>>,
    pub speaker_count: Option<SpeakerCount>,
    pub recognition_snapshot: Option<serde_json::Value>,
}

impl ResultFields {
    pub fn of(record: &NoteRecord) -> Self {
        Self {
            note_id: record.note_id.clone(),
            transcript_fingerprint: record.transcript_fingerprint.clone(),
            summary_fingerprint: record.summary_fingerprint.clone(),
            recognition_artifact: record.recognition_artifact.clone(),
            recognition_audio: record.recognition_audio.clone(),
            speaker_count: record.speaker_count,
            recognition_snapshot: record.recognition_snapshot.clone(),
        }
    }

    /// The fields that `set` sets. [`WriteSet::Summary`] takes the
    /// recognition references from the note folder, so they are not
    /// compared.
    fn compared(mut self, set: WriteSet) -> Self {
        if set == WriteSet::Summary {
            self.recognition_artifact = None;
            self.recognition_audio = None;
        }
        self
    }
}

/// What a publication did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The note folder holds the staged result.
    Committed,
    /// The result fields of the note equal neither the fields before the
    /// first write nor the staged fields: another computer published a
    /// result. Or the note folder has the transcript marker: the
    /// transcript was removed (KTD10 of the transcript removal plan). This
    /// computer writes nothing after it finds the conflict (KTD4). The
    /// files it wrote before that stay, and the note waits for a sync
    /// until the mismatch counts as a hand edit.
    Conflict,
}

/// The record of one publication (KTD1). It holds no copy of the result:
/// the staged files are beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    /// The result fields of the note folder before the first write.
    pub before: ResultFields,
    /// The staged `longhand.json`.
    pub after: NoteRecord,
    pub set: WriteSet,
}

fn conflict() -> io::Error {
    io::Error::other("the note belongs to another session")
}

fn digest(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(fingerprint(&bytes))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

impl Publication {
    /// A publication of `after` into `home`, with the result fields that
    /// `home` holds now as the fields before the first write. A note in the
    /// old layout gives an error, and nothing is written.
    pub fn new(home: &Path, after: NoteRecord, set: WriteSet) -> io::Result<Self> {
        let before = ResultFields::of(&note_files::new_layout_record(home)?);
        if before.note_id != after.note_id {
            return Err(conflict());
        }
        Ok(Self { before, after, set })
    }

    /// Write the record into `local`. From then on, the publication only
    /// rolls forward.
    pub fn record(&self, local: &Path) -> io::Result<()> {
        home_write::write_atomic(local, RECORD, &serde_json::to_vec(self)?)
    }

    /// The record in `local`, or `None` when `local` has none.
    pub fn load(local: &Path) -> io::Result<Option<Self>> {
        match std::fs::read(local.join(RECORD)) {
            Ok(bytes) => {
                let mut publication: Self = serde_json::from_slice(&bytes)?;
                // An older build recorded its layout. The roll-forward
                // writes `note_id`, so it writes this layout (KTD1).
                publication.after.layout = publication.after.layout.max(LAYOUT);
                Ok(Some(publication))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Delete the record in `local`. A record that is not there is not an
    /// error.
    pub fn remove(local: &Path) -> io::Result<()> {
        match std::fs::remove_file(local.join(RECORD)) {
            Ok(()) => crate::fsync::sync_dir(local),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// [`Publication::publish_with`] with no `after_write` hook.
    pub fn publish<G>(
        &self,
        home: &Path,
        local: &Path,
        hold: impl FnOnce() -> G,
    ) -> io::Result<Outcome> {
        self.publish_with(home, local, hold, &mut |_| Ok(()))
    }

    /// Write the staged files in `local` that differ from the files in
    /// `home`, then `longhand.json`. Each step can run again: a rerun from
    /// the record after any write reaches the same end state. `after_write`
    /// runs after each write with the name of the file. An error from it
    /// stops the publication, as a crash does.
    ///
    /// The staged files are written without a lock. `hold` gives the guard
    /// of the lock that the other writers of `longhand.json` hold (the store
    /// lock of the app): the publication holds it from the last read of
    /// the records to the write of `longhand.json`, so a title or a type
    /// written by another writer is not undone. A caller that holds the lock
    /// already gives a function that returns `()`.
    pub fn publish_with<G>(
        &self,
        home: &Path,
        local: &Path,
        hold: impl FnOnce() -> G,
        after_write: &mut dyn FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Outcome> {
        let before = self.before.clone().compared(self.set);
        let after = ResultFields::of(&self.after).compared(self.set);
        let fields = || -> io::Result<ResultFields> {
            Ok(ResultFields::of(&note_files::new_layout_record(home)?).compared(self.set))
        };
        // No write puts a removed transcript back (KTD10 of the transcript
        // removal plan). The marker stops each write, also a roll-forward.
        let removed = || folder::transcript_removed(home);
        let now = fields()?;
        if now == after && now != before {
            return Ok(Outcome::Committed);
        }
        if now != before {
            return Ok(Outcome::Conflict);
        }
        for name in self.set.files() {
            let Some(staged) = digest(&local.join(name))? else {
                // The result has no `recognition.jsonl`.
                continue;
            };
            if digest(&home.join(name))?.as_ref() == Some(&staged) {
                continue;
            }
            // Another computer's `longhand.json` can arrive during the
            // writes. Check before each write, so that at most the file in
            // progress replaces a file of the other result.
            if fields()? != before || removed() {
                return Ok(Outcome::Conflict);
            }
            home_write::copy_atomic(home, name, &local.join(name))?;
            after_write(name)?;
        }
        // Read the records again under the lock, just before the write, so
        // that a title or a pending type written in the meantime stays. The
        // read has a time limit, because the lock stops the other writers:
        // after a time-out, the publication stops as a crash does, and a
        // rerun from the record rolls forward.
        let _guard = hold();
        let current = note_files::new_layout_record(home)?;
        if ResultFields::of(&current).compared(self.set) != before || removed() {
            return Ok(Outcome::Conflict);
        }
        // An agent can delete `session.md`. The note gets it back with the
        // title that Longhand last wrote.
        if !home.join(SESSION_FILE).exists() {
            let title = current.written_title.as_ref().map(|t| t.text.as_str());
            home_write::write_atomic(
                home,
                SESSION_FILE,
                note_files::new_session_md(title, &[]).as_bytes(),
            )?;
            after_write(SESSION_FILE)?;
        }
        let record = self.patch(home, &current);
        note_files::write_record(home, &record)?;
        after_write(NOTE_RECORD_FILE)?;
        Ok(Outcome::Committed)
    }

    /// The staged records with the last-written title and the pending type
    /// of `current`: the title belongs to the title and rename writers
    /// (KTD2), and the pending type to the type move. With
    /// [`WriteSet::Summary`], the recognition references also come from
    /// `current` (KTD5).
    fn patch(&self, home: &Path, current: &NoteRecord) -> NoteRecord {
        let mut record = self.after.clone();
        record.written_title = current.written_title.clone();
        record.pending_type = current.pending_type.clone();
        if self.set == WriteSet::Summary {
            let present = home.join(ARTIFACT_FILE).is_file();
            record.recognition_artifact = current.recognition_artifact.clone().filter(|_| present);
            record.recognition_audio = current.recognition_audio.clone().filter(|_| present);
        }
        record
    }
}
