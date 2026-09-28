//! Owner-local recovery for replacement of a complete result.
use crate::{fsync::sync_dir, home_write};
use serde::{Deserialize, Serialize};
use session_core::{
    folder::{SessionDoc, fingerprint},
    generation::{self, Generation, Transition},
};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

pub const JOURNAL: &str = "result-publication.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Publication {
    pub home: PathBuf,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub initial: Option<String>,
    pub notes: Option<String>,
    pub previous: Generation,
    pub next: Generation,
}

fn conflict() -> io::Error {
    io::Error::other("the note changed during processing; previous and staged results were kept")
}

fn hash_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(fingerprint(&bytes))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Stage all result files and flush them before returning their identity.
pub fn stage(home: &Path, source: &Path) -> io::Result<Generation> {
    let mut files = BTreeMap::new();
    for name in ["transcript.md", "summary.md", "recognition.jsonl"] {
        if name == "recognition.jsonl" && !source.join(name).exists() {
            continue;
        }
        files.insert(
            name.to_string(),
            session_core::recognition::file_digest(&source.join(name))?,
        );
    }
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = (
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos(),
        SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    );
    let id = fingerprint(&serde_json::to_vec(&(&files, nonce))?);
    let generation = Generation { id, files };
    let dir = generation.directory(home)?;
    std::fs::create_dir_all(&dir)?;
    for (name, hash) in &generation.files {
        if dir.join(name).exists() {
            if session_core::recognition::file_digest(&dir.join(name))? != *hash {
                return Err(conflict());
            }
        } else {
            home_write::copy_atomic(&dir, name, &source.join(name))?;
        }
    }
    sync_dir(&home.join(generation::ROOT))?;
    sync_dir(home)?;
    generation.verify(home)?;
    Ok(generation)
}

/// The local journal exists before the first canonical result write.
pub fn prepare(
    home: &Path,
    source: &Path,
    local: &Path,
    mut next_doc: SessionDoc,
    expected: &str,
) -> io::Result<Publication> {
    if local.join(JOURNAL).exists() {
        let publication = load(local)?;
        if !publication.committed() {
            publication.write_transition()?;
        }
        return Ok(publication);
    }
    let mut before = std::fs::read_to_string(home.join("session.md"))?;
    if fingerprint(before.as_bytes()) != expected {
        return Err(conflict());
    }
    let mut old_doc = SessionDoc::parse(&before).map_err(io::Error::other)?;
    let initial = old_doc.result_generation.is_none().then(|| before.clone());
    if old_doc.session_id != next_doc.session_id {
        return Err(conflict());
    }
    // Legacy folders gain an immutable copy before any result is replaced.
    let previous = match old_doc.result_generation.clone() {
        Some(g) => {
            g.verify(home)?;
            g
        }
        None => {
            let g = stage(home, home)?;
            old_doc.result_generation = Some(g.clone());
            before = old_doc.render();
            g
        }
    };
    for name in previous.files.keys() {
        generation::read(home, &old_doc, name)?;
    }
    let next = stage(home, source)?;
    next_doc.result_generation = Some(next.clone());
    let publication = Publication {
        home: home.to_path_buf(),
        before,
        after: next_doc.render(),
        initial,
        notes: hash_optional(&home.join("notes.md"))?,
        previous,
        next,
    };
    home_write::write_atomic(local, JOURNAL, &serde_json::to_vec(&publication)?)?;
    publication.write_transition()?;
    Ok(publication)
}

pub fn load(local: &Path) -> io::Result<Publication> {
    Ok(serde_json::from_slice(&std::fs::read(
        local.join(JOURNAL),
    )?)?)
}

impl Publication {
    fn write_transition(&self) -> io::Result<()> {
        let files = self
            .next
            .files
            .iter()
            .map(|(name, hash)| {
                (
                    name.clone(),
                    (self.previous.files.get(name).cloned(), hash.clone()),
                )
            })
            .collect();
        let transition = Transition {
            before: fingerprint(self.before.as_bytes()),
            after: fingerprint(self.after.as_bytes()),
            files,
        };
        if let Ok(bytes) = std::fs::read(self.home.join(generation::TRANSITION)) {
            let current: Transition = serde_json::from_slice(&bytes)?;
            if current.before == transition.before && current.after != transition.after {
                return Err(conflict());
            }
        }
        home_write::write_atomic(
            &self.home,
            generation::TRANSITION,
            &serde_json::to_vec(&transition)?,
        )
    }
    pub fn committed(&self) -> bool {
        let expected = SessionDoc::parse(&self.after);
        generation::manifest(&self.home).is_ok_and(|doc| {
            expected.is_ok_and(|expected| {
                doc.session_id == expected.session_id
                    && doc.result_generation.as_ref() == Some(&self.next)
            })
        })
    }

    /// Advance one idempotent effect. The manifest is the commit record.
    pub fn step(&self) -> io::Result<bool> {
        if self.committed() {
            self.next.verify(&self.home)?;
            return Ok(true);
        }
        if let Some(initial) = &self.initial
            && std::fs::read(self.home.join("session.md"))? == initial.as_bytes()
        {
            home_write::write_atomic(&self.home, "session.md", self.before.as_bytes())?;
            return Ok(false);
        }
        if std::fs::read(self.home.join("session.md"))? != self.before.as_bytes()
            || hash_optional(&self.home.join("notes.md"))? != self.notes
        {
            return Err(conflict());
        }
        self.next.verify(&self.home)?;
        for (name, expected) in &self.next.files {
            let actual = hash_optional(&self.home.join(name))?;
            if actual.as_ref() != Some(expected) && actual.as_ref() != self.previous.files.get(name)
            {
                return Err(conflict());
            }
        }
        for (name, expected) in &self.next.files {
            let actual = hash_optional(&self.home.join(name))?;
            if actual.as_ref() == Some(expected) {
                continue;
            }
            if actual.as_ref() != self.previous.files.get(name) {
                return Err(conflict());
            }
            home_write::copy_atomic(
                &self.home,
                name,
                &self.next.directory(&self.home)?.join(name),
            )?;
            return Ok(false);
        }
        home_write::write_atomic(&self.home, "session.md", self.after.as_bytes())?;
        Ok(true)
    }

    /// Restore only bytes written by this publication. Keep recovery files.
    pub fn rollback(&self) -> io::Result<()> {
        if self.committed() {
            return Ok(());
        }
        if self.initial.as_ref().is_some_and(|initial| {
            std::fs::read(self.home.join("session.md"))
                .is_ok_and(|bytes| bytes == initial.as_bytes())
        }) {
            return Ok(());
        }
        if std::fs::read(self.home.join("session.md"))? != self.before.as_bytes() {
            return Err(conflict());
        }
        let mut external_edit = false;
        for (name, written) in &self.next.files {
            if hash_optional(&self.home.join(name))?.as_ref() == Some(written) {
                if self.previous.files.contains_key(name) {
                    home_write::copy_atomic(
                        &self.home,
                        name,
                        &self.previous.directory(&self.home)?.join(name),
                    )?;
                } else {
                    std::fs::remove_file(self.home.join(name))?;
                    sync_dir(&self.home)?;
                }
            } else if hash_optional(&self.home.join(name))?.as_ref()
                != self.previous.files.get(name)
            {
                external_edit = true;
            }
        }
        if external_edit {
            Err(conflict())
        } else {
            Ok(())
        }
    }

    /// Restore an uncommitted publication and let a retry prepare new inputs.
    /// Keep the journal and staged generations if rollback detects a conflict.
    pub fn abort(&self, local: &Path) -> io::Result<()> {
        if self.committed() {
            return Err(conflict());
        }
        self.rollback()?;
        if let Some(initial) = &self.initial
            && std::fs::read(self.home.join("session.md"))? == self.before.as_bytes()
        {
            home_write::write_atomic(&self.home, "session.md", initial.as_bytes())?;
        }
        let transition_path = self.home.join(generation::TRANSITION);
        if let Ok(bytes) = std::fs::read(&transition_path) {
            let transition: Transition = serde_json::from_slice(&bytes)?;
            if transition.before == fingerprint(self.before.as_bytes())
                && transition.after == fingerprint(self.after.as_bytes())
            {
                std::fs::remove_file(transition_path)?;
                sync_dir(&self.home)?;
            }
        }
        std::fs::remove_file(local.join(JOURNAL))?;
        sync_dir(local)
    }

    /// Call only after local configuration and state promotion.
    pub fn acknowledge(&self, local: &Path) -> io::Result<()> {
        if !self.committed() {
            return Err(conflict());
        }
        std::fs::remove_file(local.join(JOURNAL))?;
        sync_dir(local)
    }
}
