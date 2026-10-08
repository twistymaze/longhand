//! Tag edits and the tag list (U6 of the note types and tags plan, R9, R10,
//! R12, R13, R15, R18, R30, KTD10, KTD11).
//!
//! A tag edit is a read-modify-write of `longhand_tags` in the note's
//! `session.md`, under the store lock, followed by the row. It does not
//! wait for a busy note: every writer of `session.md` copies the tags from
//! the file that it replaces (KTD2), and a run on another computer takes
//! them from the synced file. Two tags are the same tag when their keys are
//! equal ([`tag_key`]). A note never keeps two tags with one key.
//!
//! A Settings rename or delete of a tag is a [`BulkEdit`]. The caller
//! records it before the first note changes, and each launch and scan runs
//! [`apply_edit`] again until every note is done (KTD11).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, FixedOffset};
use library::scan::{READ_TIMEOUT, Read, read_timed};
use session_core::folder::SessionDoc;
use session_core::jobs::Claims;
use session_core::tags::tag_key;
use session_core::{BulkEdit, SessionState, Store};
use session_save::home_write;

use crate::early_home::{self, EarlyState};
use crate::save::SESSION_FILE;
use crate::{SharedStore, StageError, lock};

/// The error text for a tag that is empty after trimming.
pub const EMPTY_TAG: &str = "A tag can't be empty.";

/// The error text for a tag with a line break, a tab or another control
/// character.
pub const CONTROL_IN_TAG: &str = "A tag can't contain a line break or a tab.";

/// The error text when the note's `session.md` cannot be read or written
/// now. The edit did not change the note.
pub const NOTE_UNAVAILABLE: &str =
    "The note's folder isn't available now, so its tags didn't change. Try again later.";

/// Check the text of a tag that the user typed. Returns the text with the
/// spaces at its ends removed. The text inside stays as the user typed it
/// (R11).
pub fn check_tag(text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err(EMPTY_TAG.into());
    }
    if text.chars().any(char::is_control) {
        return Err(CONTROL_IN_TAG.into());
    }
    Ok(text.to_string())
}

/// `tags` with one tag for each key: the first tag with a key stays.
fn unique(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut out = Vec::new();
    for tag in tags {
        let key = tag_key(&tag);
        if !keys.contains(&key) {
            keys.push(key);
            out.push(tag);
        }
    }
    out
}

/// `tags` without the tags that have a key of `remove`, then with each tag
/// of `add` whose key is not there yet (R15, R30).
pub fn changed(tags: &[String], add: &[String], remove: &[String]) -> Vec<String> {
    let removed: Vec<String> = remove.iter().map(|t| tag_key(t)).collect();
    unique(
        tags.iter()
            .filter(|t| !removed.contains(&tag_key(t)))
            .chain(add)
            .cloned(),
    )
}

/// `tags` after the bulk edit `edit` (KTD11). A rename puts `to` in the
/// place of each tag with the key of `from`. A tag with the key of `to`
/// then merges with it: the note keeps one of the two. `None` for an edit
/// that is not a tag edit.
pub fn edited(tags: &[String], edit: &BulkEdit) -> Option<Vec<String>> {
    match edit {
        BulkEdit::RenameTag { from, to } => {
            let from = tag_key(from);
            Some(unique(tags.iter().map(|t| {
                if tag_key(t) == from {
                    to.clone()
                } else {
                    t.clone()
                }
            })))
        }
        BulkEdit::DeleteTag { name } => Some(changed(tags, &[], std::slice::from_ref(name))),
        BulkEdit::RenameType { .. } | BulkEdit::DeleteType { .. } => None,
    }
}

/// The user adds the tags `add` to the note `session_id` and removes the
/// tags `remove` (R10). This works during a recording and during
/// processing. The tags of `add` must have passed [`check_tag`]. Returns
/// the tags of the note after the edit.
///
/// The edit reads the tags of `session.md` at the moment of the write, so
/// a tag edit that another computer made and that has synced stays (R18).
/// The edit waits while the Move holds the note.
pub fn set_note_tags(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    add: &[String],
    remove: &[String],
) -> Result<Vec<String>, StageError> {
    write_tags(store, claims, session_id, &|tags| {
        changed(tags, add, remove)
    })
}

/// Apply `edit` to the tags of the note `session_id`, in `session.md` and
/// in the row. A note whose early write is not done has its tags in the row
/// only, and the early write puts them into `session.md`. Under the note
/// guard of the early write, so the write and the edit do not overlap.
fn write_tags(
    store: &SharedStore,
    claims: &Claims,
    session_id: &str,
    edit: &dyn Fn(&[String]) -> Vec<String>,
) -> Result<Vec<String>, StageError> {
    let has_home = lock(store)
        .session(session_id)?
        .ok_or_else(|| no_session(session_id))?
        .home_path
        .is_some();
    // A note with a home folder needs no guard: the early write is done,
    // and each later writer reads the tags of the file under the store
    // lock.
    let _guard = (!has_home).then(|| early_home::guard(session_id));
    let guard = crate::move_notes::lock_unheld(store, claims, session_id);
    write_locked(&guard, session_id, edit)
}

/// [`write_tags`] with the store lock held.
fn write_locked(
    store: &Store,
    session_id: &str,
    edit: &dyn Fn(&[String]) -> Vec<String>,
) -> Result<Vec<String>, StageError> {
    let record = store
        .session(session_id)?
        .ok_or_else(|| no_session(session_id))?;
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        let tags = edit(&record.tags);
        // A staging folder that is complete becomes the home folder with
        // its `session.md` as it is: that file gets the tags too.
        if let Some(EarlyState::Filled { root, .. }) = early_home::state(store, session_id)? {
            let staging = root.join(session_save::naming::staging_name(session_id));
            write_file(&staging, session_id, &|_| tags.clone())?;
        }
        if tags != record.tags {
            store.set_tags(session_id, &tags)?;
        }
        return Ok(tags);
    };
    let tags = write_file(&home, session_id, edit)?;
    if tags != record.tags {
        store.set_tags(session_id, &tags)?;
    }
    Ok(tags)
}

/// Read `session.md` in `folder`, apply `edit` to its tags and write it
/// when the tags change. Returns the tags of the file after the edit.
fn write_file(
    folder: &Path,
    session_id: &str,
    edit: &dyn Fn(&[String]) -> Vec<String>,
) -> Result<Vec<String>, StageError> {
    let unavailable = |why: String| {
        log::warn!(
            "could not change the tags in {SESSION_FILE} in {}: {why}",
            folder.display()
        );
        StageError::NoDestination(NOTE_UNAVAILABLE.into())
    };
    let mut doc = match read_timed(folder.join(SESSION_FILE), READ_TIMEOUT) {
        Read::Bytes(bytes) => std::str::from_utf8(&bytes)
            .ok()
            .and_then(|text| SessionDoc::parse(text).ok())
            .ok_or_else(|| unavailable("the file is not valid".into()))?,
        Read::Missing => return Err(unavailable("the file is missing".into())),
        Read::Failed(why) => return Err(unavailable(why)),
    };
    if doc.session_id != session_id {
        return Err(unavailable(format!(
            "the file is of the note {}",
            doc.session_id
        )));
    }
    let tags = edit(&doc.tags);
    if tags != doc.tags {
        doc.tags = tags.clone();
        home_write::write_atomic(folder, SESSION_FILE, doc.render().as_bytes())
            .map_err(|e| unavailable(e.to_string()))?;
    }
    Ok(tags)
}

/// What one pass of a tag rename or delete did (KTD11).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TagPass {
    /// The number of notes that had the tag at the start of the pass (R31).
    pub matched: usize,
    /// The notes that the pass changed.
    pub changed: Vec<String>,
    /// Every note that had the tag is changed: the edit is done.
    pub done: bool,
}

/// One pass of the tag rename or delete `edit` over every note of the
/// store (KTD11, R13). A note that another computer records or processes
/// changes too. A note whose `session.md` cannot be read or written now
/// stays, and the pass is not done: the caller runs it again later. A pass
/// can run again: a note that has the result already is not in it.
pub fn apply_edit(
    store: &SharedStore,
    claims: &Claims,
    edit: &BulkEdit,
) -> Result<TagPass, StageError> {
    if !edit.is_tag_edit() {
        return Err(StageError::Failed(format!("{edit:?} is not a tag edit")));
    }
    let apply = |tags: &[String]| edited(tags, edit).unwrap_or_else(|| tags.to_vec());
    let ids: Vec<String> = lock(store)
        .sessions()?
        .into_iter()
        .filter(|r| r.state != SessionState::Discarded && apply(&r.tags) != r.tags)
        .map(|r| r.id)
        .collect();
    let mut pass = TagPass {
        matched: ids.len(),
        changed: Vec::new(),
        done: true,
    };
    for id in ids {
        match write_tags(store, claims, &id, &apply) {
            Ok(_) => pass.changed.push(id),
            Err(e) => {
                log::warn!("the tag edit of {id} continues later: {e}");
                pass.done = false;
            }
        }
    }
    Ok(pass)
}

/// One tag of the tag list (KTD10, R12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCount {
    /// The spelling on the note with the earliest start that has the tag
    /// (R30).
    pub text: String,
    /// The key of the tag ([`tag_key`]).
    pub key: String,
    /// The number of notes with the tag.
    pub count: usize,
}

/// The tag list (R12): each tag that is on at least one note that is not
/// discarded, sorted by its text without regard to case.
pub fn tag_list(store: &Store) -> Result<Vec<TagCount>, StageError> {
    let mut found: HashMap<String, (TagCount, DateTime<FixedOffset>)> = HashMap::new();
    for record in store.sessions()? {
        if record.state == SessionState::Discarded || record.tags.is_empty() {
            continue;
        }
        let started = match record.started_at {
            Some(at) => at,
            None => record.start(&store.segments(&record.id)?),
        };
        for text in unique(record.tags.iter().cloned()) {
            let key = tag_key(&text);
            let entry = found.entry(key.clone()).or_insert_with(|| {
                (
                    TagCount {
                        text: text.clone(),
                        key,
                        count: 0,
                    },
                    started,
                )
            });
            entry.0.count += 1;
            if started < entry.1 {
                entry.0.text = text;
                entry.1 = started;
            }
        }
    }
    let mut list: Vec<TagCount> = found.into_values().map(|(tag, _)| tag).collect();
    list.sort_by(|a, b| {
        a.text
            .to_lowercase()
            .cmp(&b.text.to_lowercase())
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(list)
}

fn no_session(session_id: &str) -> StageError {
    StageError::Failed(format!("no session {session_id}"))
}
