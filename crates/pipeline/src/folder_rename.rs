//! Folder rename (KTD8, R9, R31): a saved, idle session's home folder
//! takes the name its title gives. The rename is a step-then-record
//! operation under the store lock:
//! 1. record the intended move (`Store::set_rename_intent`);
//! 2. rename the folder with no-replace semantics, then fsync the parent
//!    (`session_save::rename_home::rename`);
//! 3. rewrite `session.md` in the new folder with the title;
//! 4. set `home_path`, then clear the intent.
//!
//! At launch, [`recover`] settles each intent that is left from disk, then
//! runs the reconcile for every saved session. A rename that was deferred,
//! lost to a crash or failed then completes.

use std::path::{Path, PathBuf};

use session_core::folder::SessionDoc;
use session_core::{RenameIntent, Segment, SessionRecord, SessionState, Store, TitleSource};
use session_save::rename_home::{self, Move, RenameError, Resolved};
use session_save::{Fs, home_write};

use crate::save::{SESSION_FILE, session_doc};
use crate::{SharedStore, StageError, lock};

/// What a reconcile did to a session's home folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderRename {
    /// The folder already has the name its title gives.
    Unchanged,
    /// The session is busy: it records, is queued, processes or
    /// regenerates its summary. The folder takes the name when that ends.
    Later,
    /// The folder has the new name at this path.
    Moved(PathBuf),
    /// The rename failed and the folder keeps its old name. The title
    /// stays. The text says why.
    Kept(String),
}

/// Whether a session is too busy for a folder rename (KTD8). This is the
/// one busy check: every reconcile calls it. A session that records, is
/// queued or processes is not in the saved state. `regenerating` is true
/// while the session's summary is regenerated; the caller reads it under
/// the store lock, so a regeneration that starts later reads the renamed
/// folder.
pub fn busy(record: &SessionRecord, regenerating: bool) -> bool {
    record.state != SessionState::Saved || record.home_path.is_none() || regenerating
}

/// The user's rename (R9, R35): store `title` as the user's title, which
/// changes it at once, then run the reconcile. `regenerating` tells
/// whether the session's summary is regenerated now.
pub fn rename_session(
    store: &SharedStore,
    session_id: &str,
    title: &str,
    regenerating: &dyn Fn(&str) -> bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    let guard = lock(store);
    guard.set_title(session_id, title, TitleSource::User)?;
    let regenerating = regenerating(session_id);
    reconcile(&guard, session_id, regenerating, true, fs)
}

/// Compare a saved, idle session's folder name with the name its title
/// gives, and rename the folder when they differ. With `check_document`,
/// a folder that keeps its name gets `session.md` rewritten when its title
/// is not the stored one. The caller holds the store lock for the whole
/// call, so no other writer sees the folder between two steps.
pub fn reconcile(
    store: &Store,
    session_id: &str,
    regenerating: bool,
    check_document: bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    let record = require(store, session_id)?;
    if busy(&record, regenerating) {
        return Ok(FolderRename::Later);
    }
    // An intent that an earlier failure left is settled first, so the
    // rename starts from where the folder is.
    let record = match store.rename_intent(session_id)? {
        Some(intent) => {
            settle(store, &record, &intent, fs)?;
            require(store, session_id)?
        }
        None => record,
    };
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        return Ok(FolderRename::Later);
    };
    let segments = store.segments(session_id)?;
    let Some(first) = segments.first() else {
        return Ok(FolderRename::Unchanged);
    };
    let title = record.title.as_ref().map(|t| t.text.as_str());
    let name = session_save::naming::folder_name(first.wall_start, title);
    let current = home
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if rename_home::has_name(current, &name) {
        if check_document {
            refresh_document(&record, &segments, &home);
        }
        return Ok(FolderRename::Unchanged);
    }

    let renamed = rename_home::rename(&home, &name, session_id, fs, &mut |m| {
        store
            .set_rename_intent(session_id, &to_intent(m))
            .map_err(|e| RenameError::Record(e.to_string()))
    });
    match renamed {
        Ok(path) if path == home => {
            if check_document {
                refresh_document(&record, &segments, &home);
            }
            Ok(FolderRename::Unchanged)
        }
        Ok(path) => {
            finish(store, &record, &path)?;
            log::info!(
                "renamed the folder of {session_id} from {} to {}",
                home.display(),
                path.display()
            );
            Ok(FolderRename::Moved(path))
        }
        Err(e) => {
            log::warn!("could not rename the folder of {session_id}: {e}");
            // The error can come after the move, for example from the
            // fsync of the parent. The recorded intent tells where the
            // folder is.
            if let Some(intent) = store.rename_intent(session_id)?
                && let Some(path) = settle(store, &record, &intent, fs)?
                && path != home
            {
                return Ok(FolderRename::Moved(path));
            }
            Ok(FolderRename::Kept(e.to_string()))
        }
    }
}

/// [`reconcile`] for a caller that does not hold the store lock.
pub fn reconcile_shared(
    store: &SharedStore,
    session_id: &str,
    regenerating: bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    reconcile(&lock(store), session_id, regenerating, true, fs)
}

/// At launch, settle every rename intent that a crash left. Returns the
/// ids of the sessions whose intents were settled.
pub fn resolve_intents(store: &SharedStore, fs: &dyn Fs) -> Result<Vec<String>, StageError> {
    let guard = lock(store);
    let mut settled = Vec::new();
    for (id, intent) in guard.rename_intents()? {
        let Some(record) = guard.session(&id)? else {
            guard.clear_rename_intent(&id)?;
            continue;
        };
        match settle(&guard, &record, &intent, fs) {
            Ok(_) => settled.push(id),
            Err(e) => log::warn!("could not settle the folder rename of {id}: {e}"),
        }
    }
    Ok(settled)
}

/// At launch, before the processing queue starts: settle the intents that
/// are left, then run the reconcile for every saved session. Nothing
/// regenerates at launch. A failure on one session does not stop the
/// others.
pub fn recover(store: &SharedStore, fs: &dyn Fs) -> Result<(), StageError> {
    resolve_intents(store, fs)?;
    let guard = lock(store);
    for record in guard.sessions_in(SessionState::Saved.name())? {
        match reconcile(&guard, &record.id, false, false, fs) {
            Ok(FolderRename::Kept(why)) => {
                log::warn!("the folder of {} keeps its name: {why}", record.id)
            }
            Ok(_) => {}
            Err(e) => log::warn!("could not reconcile the folder of {}: {e}", record.id),
        }
    }
    Ok(())
}

/// Settle a recorded intent from disk. Returns the folder's new path when
/// the move happened; then `session.md` and `home_path` follow it. The
/// intent is cleared in every case but an error.
fn settle(
    store: &Store,
    record: &SessionRecord,
    intent: &RenameIntent,
    fs: &dyn Fs,
) -> Result<Option<PathBuf>, StageError> {
    let recorded = from_intent(intent);
    let resolved = rename_home::resolve(&recorded, fs)
        .map_err(|e| StageError::Failed(format!("the folder rename cannot be settled: {e}")))?;
    match resolved {
        Resolved::Moved(path) => {
            finish(store, record, &path)?;
            Ok(Some(path))
        }
        Resolved::NotMoved => {
            store.clear_rename_intent(&record.id)?;
            Ok(None)
        }
        Resolved::Missing => {
            log::warn!(
                "the folder of {} is at none of {}, {} or the temporary name",
                record.id,
                recorded.from.display(),
                recorded.to.display()
            );
            store.clear_rename_intent(&record.id)?;
            Ok(None)
        }
    }
}

/// Steps 3 and 4 of the rename: rewrite `session.md` in the folder at
/// `path`, set `home_path`, then clear the intent. Each step can run
/// again. A `session.md` that cannot be written does not stop the rename,
/// because the folder has moved: the next reconcile with the document
/// check writes it.
fn finish(store: &Store, record: &SessionRecord, path: &Path) -> Result<(), StageError> {
    let segments = store.segments(&record.id)?;
    if let Err(e) = write_document(record, &segments, path) {
        log::warn!(
            "could not rewrite {SESSION_FILE} in {}: {e}",
            path.display()
        );
    }
    store.set_home_path(&record.id, &path.to_string_lossy())?;
    store.clear_rename_intent(&record.id)?;
    Ok(())
}

/// Rewrite `session.md` in `folder` when its title is not the stored one.
fn refresh_document(record: &SessionRecord, segments: &[Segment], folder: &Path) {
    let stored = record.title.as_ref().map(|t| t.text.as_str());
    let written = std::fs::read_to_string(folder.join(SESSION_FILE))
        .ok()
        .and_then(|text| SessionDoc::parse(&text).ok());
    if let Some(doc) = written
        && doc.title.as_deref() != stored
        && let Err(e) = write_document(record, segments, folder)
    {
        log::warn!(
            "could not rewrite {SESSION_FILE} in {}: {e}",
            folder.display()
        );
    }
}

fn write_document(
    record: &SessionRecord,
    segments: &[Segment],
    folder: &Path,
) -> Result<(), StageError> {
    let doc = session_doc(record, segments)?;
    home_write::write_atomic(folder, SESSION_FILE, doc.render().as_bytes())?;
    Ok(())
}

fn require(store: &Store, session_id: &str) -> Result<SessionRecord, StageError> {
    store
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))
}

fn to_intent(m: &Move) -> RenameIntent {
    let text = |p: &Path| p.to_string_lossy().into_owned();
    RenameIntent {
        from: text(&m.from),
        to: text(&m.to),
        temp: m.temp.as_deref().map(text),
    }
}

fn from_intent(intent: &RenameIntent) -> Move {
    Move {
        from: PathBuf::from(&intent.from),
        to: PathBuf::from(&intent.to),
        temp: intent.temp.as_ref().map(PathBuf::from),
    }
}
