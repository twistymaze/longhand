//! Publication of a new result into a note folder (KTD1 to KTD5 of the
//! result copies plan). The new result is complete in a local staging
//! folder first. The owner then writes a small record into that staging
//! folder, copies the staged files that differ into the note folder, and
//! writes `session.md` last. The note folder keeps no copies of earlier
//! results.
//!
//! The record never syncs. Its presence means "unfinished publication":
//! after a crash, a rerun reads it and only rolls forward. The rerun
//! writes the staged files that differ and then `session.md`, whatever the
//! note folder holds, unless the result fields of `session.md` changed on
//! another computer. Then the owner drops its result (KTD4).
use crate::home_write;
use serde::{Deserialize, Serialize};
use session_core::SpeakerCount;
use session_core::folder::{SessionDoc, fingerprint};
use session_core::recognition::{ARTIFACT_FILE, ArtifactReference};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// The name of the record in the local staging folder.
pub const RECORD: &str = "publication.json";

/// The files that a publication writes into the note folder before
/// `session.md` (KTD5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteSet {
    /// Save, Resume and Transcribe Again: `recognition.jsonl` when the
    /// result has one, `transcript.md` and `summary.md`, in that order.
    /// Most sync clients upload in write order, so `recognition.jsonl`
    /// does not arrive after `session.md`.
    Result,
    /// Regenerate Summary: `summary.md` only. The recognition references
    /// in `session.md` come from the note folder at write time, and go
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

/// The fields of `session.md` that tell which result the note folder
/// holds (KTD2). A publication compares only these fields, so a title
/// write or a rename between two writes is not a conflict.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResultFields {
    pub session_id: String,
    pub transcript_fingerprint: Option<String>,
    pub summary_fingerprint: Option<String>,
    pub recognition_artifact: Option<ArtifactReference>,
    pub recognition_audio: Option<BTreeMap<String, String>>,
    pub speaker_count: Option<SpeakerCount>,
    pub recognition_snapshot: Option<serde_json::Value>,
}

impl ResultFields {
    pub fn of(doc: &SessionDoc) -> Self {
        Self {
            session_id: doc.session_id.clone(),
            transcript_fingerprint: doc.transcript_fingerprint.clone(),
            summary_fingerprint: doc.summary_fingerprint.clone(),
            recognition_artifact: doc.recognition_artifact.clone(),
            recognition_audio: doc.recognition_audio.clone(),
            speaker_count: doc.speaker_count,
            recognition_snapshot: doc.recognition_snapshot.clone(),
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
    /// The result fields of `session.md` equal neither the fields before
    /// the first write nor the staged fields: another computer published
    /// a result. This computer writes nothing after it finds the conflict
    /// (KTD4). The files it wrote before that stay, and the note waits for
    /// a sync until the mismatch counts as a hand edit.
    Conflict,
}

/// The record of one publication (KTD1). It holds no copy of the result:
/// the staged files are beside it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    /// The result fields of the note folder's `session.md` before the first
    /// write.
    pub before: ResultFields,
    /// The staged `session.md`.
    pub after: SessionDoc,
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
    /// `home` holds now as the fields before the first write.
    pub fn new(home: &Path, after: SessionDoc, set: WriteSet) -> io::Result<Self> {
        let before = ResultFields::of(&SessionDoc::read(home)?);
        if before.session_id != after.session_id {
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
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
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

    /// [`Publication::publish_with`] with no hook.
    pub fn publish(&self, home: &Path, local: &Path) -> io::Result<Outcome> {
        self.publish_with(home, local, &mut |_| Ok(()))
    }

    /// Write the staged files in `local` that differ from the files in
    /// `home`, then `session.md`. Each step can run again: a rerun from the
    /// record after any write reaches the same end state. `after_write`
    /// runs after each write with the name of the file. An error from it
    /// stops the publication, as a crash does.
    pub fn publish_with(
        &self,
        home: &Path,
        local: &Path,
        after_write: &mut dyn FnMut(&str) -> io::Result<()>,
    ) -> io::Result<Outcome> {
        let before = self.before.clone().compared(self.set);
        let after = ResultFields::of(&self.after).compared(self.set);
        let fields = || -> io::Result<ResultFields> {
            Ok(ResultFields::of(&SessionDoc::read(home)?).compared(self.set))
        };
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
            // Another computer's `session.md` can arrive during the
            // writes. Check before each write, so that at most the file in
            // progress replaces a file of the other result.
            if fields()? != before {
                return Ok(Outcome::Conflict);
            }
            home_write::copy_atomic(home, name, &local.join(name))?;
            after_write(name)?;
        }
        // Read `session.md` again just before the write, so that a title
        // written in the meantime stays.
        let current = SessionDoc::read(home)?;
        if ResultFields::of(&current).compared(self.set) != before {
            return Ok(Outcome::Conflict);
        }
        let doc = self.patch(home, &current);
        home_write::write_atomic(home, "session.md", doc.render().as_bytes())?;
        after_write("session.md")?;
        Ok(Outcome::Committed)
    }

    /// The staged `session.md` with the title, the tags and the pending
    /// type of `current`: the title belongs to the title and rename writers
    /// (KTD2), and the tags and the pending type are user fields (KTD2 of
    /// the note types and tags plan). With [`WriteSet::Summary`], the
    /// recognition references also come from `current` (KTD5).
    fn patch(&self, home: &Path, current: &SessionDoc) -> SessionDoc {
        let mut doc = self.after.clone();
        doc.title = current.title.clone();
        doc.title_source = current.title_source;
        doc.tags = current.tags.clone();
        doc.pending_type = current.pending_type.clone();
        if self.set == WriteSet::Summary {
            let present = home.join(ARTIFACT_FILE).is_file();
            doc.recognition_artifact = current.recognition_artifact.clone().filter(|_| present);
            doc.recognition_audio = current.recognition_audio.clone().filter(|_| present);
        }
        doc
    }
}
