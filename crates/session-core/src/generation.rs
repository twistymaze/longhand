//! A manifest names one complete immutable result.
use crate::folder::{SessionDoc, fingerprint};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    path::{Path, PathBuf},
};

pub const ROOT: &str = ".longhand-results";
pub const TRANSITION: &str = ".longhand-result-transition.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generation {
    pub id: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transition {
    pub before: String,
    pub after: String,
    pub files: BTreeMap<String, (Option<String>, String)>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::other(message)
}

impl Generation {
    pub fn directory(&self, home: &Path) -> io::Result<PathBuf> {
        if self.id.is_empty()
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        {
            return Err(invalid("invalid result generation id"));
        }
        Ok(home.join(ROOT).join(&self.id))
    }

    pub fn verify(&self, home: &Path) -> io::Result<()> {
        let dir = self.directory(home)?;
        for required in ["transcript.md", "summary.md"] {
            if !self.files.contains_key(required) {
                return Err(invalid("incomplete result generation"));
            }
        }
        for (name, hash) in &self.files {
            if !matches!(
                name.as_str(),
                "transcript.md" | "summary.md" | "recognition.jsonl"
            ) {
                return Err(invalid("invalid result file"));
            }
            if crate::recognition::file_digest(&dir.join(name))? != *hash {
                return Err(invalid("result generation hash mismatch"));
            }
        }
        Ok(())
    }
}

pub fn manifest(home: &Path) -> io::Result<SessionDoc> {
    SessionDoc::parse(&std::fs::read_to_string(home.join("session.md"))?)
        .map_err(|e| invalid(&e.to_string()))
}

pub struct Resolved {
    pub doc: SessionDoc,
    pub transcript: Vec<u8>,
    pub summary: Vec<u8>,
}

/// A receiver retains its last verified result while new files arrive.
pub fn resolve(home: &Path, previous: Option<&SessionDoc>) -> io::Result<Resolved> {
    let current = manifest(home)?;
    let pending = current
        .result_generation
        .as_ref()
        .is_some_and(|g| g.verify(home).is_err());
    let doc = if pending {
        previous
            .filter(|p| p.session_id == current.session_id)
            .ok_or_else(|| invalid("waiting for result files to sync"))?
    } else {
        &current
    };
    let read_one = |name: &str| {
        if pending {
            let generation = doc
                .result_generation
                .as_ref()
                .ok_or_else(|| invalid("waiting for result files to sync"))?;
            generation.verify(home)?;
            if let Ok(bytes) = std::fs::read(home.join(name)) {
                let hash = fingerprint(&bytes);
                if generation.files.get(name) != Some(&hash)
                    && current
                        .result_generation
                        .as_ref()
                        .and_then(|g| g.files.get(name))
                        != Some(&hash)
                {
                    return Err(invalid("result file changed outside Longhand"));
                }
            }
            let bytes = std::fs::read(generation.directory(home)?.join(name))?;
            if generation.files.get(name) != Some(&fingerprint(&bytes)) {
                return Err(invalid("result file changed during read"));
            }
            Ok(bytes)
        } else {
            read(home, doc, name)
        }
    };
    let transcript = read_one("transcript.md")?;
    let summary = read_one("summary.md")?;
    if manifest(home)? != current {
        return Err(invalid("the result changed while reading; read it again"));
    }
    Ok(Resolved {
        doc: doc.clone(),
        transcript,
        summary,
    })
}

/// Read only a complete generation. Unexpected canonical edits are conflicts.
pub fn read(home: &Path, doc: &SessionDoc, name: &str) -> io::Result<Vec<u8>> {
    let Some(generation) = &doc.result_generation else {
        return std::fs::read(home.join(name));
    };
    generation.verify(home)?;
    let expected = generation
        .files
        .get(name)
        .ok_or_else(|| invalid("result file is absent"))?;
    if let Ok(canonical) = std::fs::read(home.join(name)) {
        let hash = fingerprint(&canonical);
        if hash != *expected {
            let transition: Option<Transition> = std::fs::read(home.join(TRANSITION))
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok());
            let manifest_hash = fingerprint(&std::fs::read(home.join("session.md"))?);
            let own_write = transition.is_some_and(|t| {
                (t.before == manifest_hash || t.after == manifest_hash)
                    && t.files
                        .get(name)
                        .is_some_and(|(old, new)| old.as_ref() == Some(&hash) || *new == hash)
            });
            if !own_write {
                return Err(invalid(
                    "result file changed outside Longhand; preserve the external edit",
                ));
            }
        }
    }
    let bytes = std::fs::read(generation.directory(home)?.join(name))?;
    if fingerprint(&bytes) != *expected {
        return Err(invalid("result file changed during read"));
    }
    Ok(bytes)
}
