//! Folder rename (KTD8, R9, R31): a saved, idle session's home folder
//! takes the name its title gives. The rename is a step-then-record
//! operation under the store lock:
//! 1. record the intended move (`Store::set_rename_intent`);
//! 2. rename the folder with no-replace semantics, then fsync the parent
//!    (`session_save::rename_home::rename`);
//! 3. write the title into `session.md` in the new folder, and then into
//!    `longhand.json` as the last-written title (KTD3, KTD5);
//! 4. set `home_path`, then clear the intent.
//!
//! At launch, [`resolve_intents`] settles each intent that is left from
//! disk. After the library reconcile, [`reconcile_saved`] runs the
//! reconcile for every saved session. A rename that was deferred, lost to
//! a crash or failed then completes.

use std::path::{Path, PathBuf};

use library::scan::{READ_TIMEOUT, read_note_files};
use session_core::folder::ResultFlag;
use session_core::jobs::busy;
use session_core::note_folder::{Layout, NoteFiles, SESSION_FILE};
use session_core::{RenameIntent, SessionRecord, SessionState, Store, TitleSource};
use session_save::Fs;
use session_save::note_files;
use session_save::rename_home::{self, Move, RenameError, Resolved};

use crate::save::note_started;
use crate::{SharedStore, StageError, lock};

/// What a reconcile did to a session's home folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FolderRename {
    /// The folder already has the name its title gives.
    Unchanged,
    /// The session is busy: it records, is queued, processes, regenerates
    /// its summary, is deleted or moves to a new storage folder. The folder
    /// takes the name when that ends. Also for a note that another
    /// computer records into and has not saved: that computer renames the
    /// folder after its Save (KTD9).
    Later,
    /// The folder has the new name at this path.
    Moved(PathBuf),
    /// The rename failed and the folder keeps its old name. The title
    /// stays. The text says why.
    Kept(String),
}

/// The user's rename (R9, R35): bring the row up to date from its folder
/// (KTD3), store `title` as the user's title, which changes it at once,
/// then run the reconcile. `claimed` tells whether a summary regeneration
/// or a delete holds a claim on the session now.
pub fn rename_session(
    store: &SharedStore,
    session_id: &str,
    title: &str,
    claimed: &dyn Fn(&str) -> bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    rename_locked(&mut lock(store), session_id, title, claimed, fs)
}

/// [`rename_session`] for a caller that holds the store lock: the runner
/// takes it when the Move does not hold the note
/// ([`crate::move_notes::lock_unheld`]).
pub fn rename_locked(
    guard: &mut Store,
    session_id: &str,
    title: &str,
    claimed: &dyn Fn(&str) -> bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    let renamed = retitle_locked(guard, session_id, title, TitleSource::User, claimed, fs)?;
    Ok(renamed.expect("a user title always replaces the stored title"))
}

/// The title job's proposal (KTD13, R13): bring the row up to date from
/// its folder (KTD3), store `title` as a proposed title, then run the
/// reconcile, as [`rename_locked`] does for the user's title. `None` when
/// the row keeps its title: the title is empty, or the user set a title,
/// also on another computer. The caller holds the store lock.
pub fn propose_locked(
    guard: &mut Store,
    session_id: &str,
    title: &str,
    claimed: &dyn Fn(&str) -> bool,
    fs: &dyn Fs,
) -> Result<Option<FolderRename>, StageError> {
    retitle_locked(guard, session_id, title, TitleSource::Proposed, claimed, fs)
}

/// Bring the row up to date from its folder (KTD3), store `title` with
/// `source`, then run the reconcile. `None` when the row keeps its title.
fn retitle_locked(
    guard: &mut Store,
    session_id: &str,
    title: &str,
    source: TitleSource,
    claimed: &dyn Fn(&str) -> bool,
    fs: &dyn Fs,
) -> Result<Option<FolderRename>, StageError> {
    let claimed = claimed(session_id);
    crate::follow::follow_folder(guard, session_id, claimed)?;
    // A title that a person or an agent wrote into `session.md` is not
    // replaced by a proposal, also before the row takes it (R5, KTD5).
    if source == TitleSource::Proposed
        && let Some(home) = guard.session(session_id)?.and_then(|r| r.home_path)
        && read_files(Path::new(&home)).is_some_and(|files| files.title_changed_outside())
    {
        return Ok(None);
    }
    if !guard.set_title(session_id, title, source)? {
        return Ok(None);
    }
    // Recorded before the move, so a crash before the reconcile ends leaves
    // the rename to the next launch.
    guard.set_rename_owed(session_id, true)?;
    let renamed = reconcile(guard, session_id, claimed, true, fs)?;
    // A folder that moves later, or that keeps its name because the move
    // failed, gets the title in its `session.md` now. Otherwise the next
    // library refresh reads the old title and puts it back in the row.
    if matches!(renamed, FolderRename::Later | FolderRename::Kept(_)) {
        write_title(guard, session_id);
    }
    Ok(Some(renamed))
}

/// Write the stored title into the `session.md` of the session's folder
/// where it is now: a busy session's folder takes its new name later, and
/// a folder whose move failed keeps its name. A writer or a library
/// refresh that follows the folder in the meantime (KTD3) then reads this
/// title, not the old one. A folder that is not there, or a `session.md`
/// that cannot be read or written, is logged: the reconcile after the
/// work writes the document.
fn write_title(store: &Store, session_id: &str) {
    let Ok(Some(record)) = store.session(session_id) else {
        return;
    };
    let Some(home) = record.home_path.as_deref().map(Path::new) else {
        return;
    };
    write_title_in(&record, home);
}

/// Write the title of `record` into the note folder `home`, and change
/// nothing else (R4, KTD3). In the new layout, the patch changes only the
/// title of `session.md`, and then `longhand.json` records it as the
/// last-written title. Nothing is written when the row has the title that
/// Longhand last wrote: a `session.md` title that differs from it then is
/// a change from outside, which this write must not undo (KTD5). In the
/// old layout, the patch changes the title and its source in `session.md`.
fn write_title_in(record: &SessionRecord, home: &Path) {
    if let Err(e) = try_write_title_in(record, home) {
        log::warn!(
            "could not write the title into {SESSION_FILE} in {}: {e}",
            home.display()
        );
    }
}

fn try_write_title_in(record: &SessionRecord, home: &Path) -> Result<(), StageError> {
    let Some(files) = read_files(home) else {
        return Ok(());
    };
    let title = record.title.as_ref().map(|t| t.text.as_str());
    match files.layout {
        Layout::New => {
            if files.record.written_title == record.title {
                return Ok(());
            }
            note_files::patch_session_md(home, &note_files::title_patch(title))?;
            let mut written = files.record;
            written.written_title = record.title.clone();
            note_files::write_record(home, &written)?;
        }
        Layout::Old => {
            let source = record.title.as_ref().map(|t| t.source);
            if files.doc.title.as_deref() == title && files.doc.title_source == source {
                return Ok(());
            }
            let patch = match source {
                Some(source) => note_files::title_patch(title).set("title_source", source),
                None => note_files::title_patch(title).remove("title_source"),
            };
            note_files::patch_session_md(home, &patch)?;
        }
    }
    Ok(())
}

/// The note files in `folder`, or `None` when they cannot be read or do
/// not give a note. The read stops after [`READ_TIMEOUT`].
fn read_files(folder: &Path) -> Option<NoteFiles> {
    read_note_files(folder, READ_TIMEOUT)
}

/// Compare a saved, idle session's folder name with the name its title
/// gives, and rename the folder when they differ. With `check_document`,
/// a folder that keeps its name gets `session.md` rewritten when its title
/// is not the stored one. The caller holds the store lock for the whole
/// call, so no other writer sees the folder between two steps.
///
/// A result of `Later` or `Kept` records that this computer owes the
/// folder a rename, and the launch reconcile tries again. `Unchanged` or
/// `Moved` clears the record.
pub fn reconcile(
    store: &Store,
    session_id: &str,
    claimed: bool,
    check_document: bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    if not_saved_elsewhere(store, session_id, claimed)? {
        // The computer that records into the note renames the folder after
        // its Save, with the title that it takes from `session.md` (KTD9).
        // This computer owes no rename.
        store.set_rename_owed(session_id, false)?;
        return Ok(FolderRename::Later);
    }
    let result = reconcile_once(store, session_id, claimed, check_document, fs)?;
    let owed = matches!(result, FolderRename::Later | FolderRename::Kept(_));
    store.set_rename_owed(session_id, owed)?;
    Ok(result)
}

fn reconcile_once(
    store: &Store,
    session_id: &str,
    claimed: bool,
    check_document: bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    let record = require(store, session_id)?;
    // The Move puts the folder in the new storage folder first (KTD9).
    let moving = !matches!(
        store.move_step(session_id)?,
        None | Some(session_core::MoveStep::Done)
    );
    if busy(&record, claimed) || moving {
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
    // The note's start time names the folder, also for a note with no
    // segment (KTD9).
    let title = record.title.as_ref().map(|t| t.text.as_str());
    let name = session_save::naming::folder_name(note_started(&record, &segments), title);
    let current = home
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if rename_home::has_name(current, &name) {
        if check_document {
            write_title_in(&record, &home);
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
                write_title_in(&record, &home);
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
    claimed: bool,
    fs: &dyn Fs,
) -> Result<FolderRename, StageError> {
    reconcile(&lock(store), session_id, claimed, true, fs)
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

/// At launch, after [`resolve_intents`] and the library reconcile, and
/// before the processing queue starts: run the reconcile for each saved
/// session whose folder this computer owes a rename (KTD9). A folder that
/// another computer renamed is not in that list, so the reconcile does not
/// rename it back when its new `session.md` has not synced yet. Nothing
/// regenerates at launch. A failure on one session does not stop the
/// others.
pub fn reconcile_saved(store: &SharedStore, fs: &dyn Fs) -> Result<(), StageError> {
    let guard = lock(store);
    let saved = SessionState::Saved.name();
    for id in guard.owed_renames()? {
        if require(&guard, &id)?.state.name() != saved {
            continue;
        }
        match reconcile(&guard, &id, false, false, fs) {
            Ok(FolderRename::Kept(why)) => {
                log::warn!("the folder of {id} keeps its name: {why}")
            }
            Ok(_) => {}
            Err(e) => log::warn!("could not reconcile the folder of {id}: {e}"),
        }
    }
    Ok(())
}

/// Settle a recorded intent from disk. Returns the folder's new path when
/// the move happened; then `session.md` and `home_path` follow it. The
/// intent is cleared in every case but an error.
pub(crate) fn settle(
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
            // The move did not happen, so this computer still owes it.
            store.set_rename_owed(&record.id, true)?;
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

/// Steps 3 and 4 of the rename: write the title into the folder at
/// `path`, set `home_path`, then clear the intent. Each step can run
/// again. A title that cannot be written does not stop the rename, because
/// the folder has moved: the next reconcile with the document check writes
/// it. The other records stay as `longhand.json` has them (KTD6 of the
/// result copies plan).
fn finish(store: &Store, record: &SessionRecord, path: &Path) -> Result<(), StageError> {
    write_title_in(record, path);
    store.set_home_path(&record.id, &path.to_string_lossy())?;
    store.clear_rename_intent(&record.id)?;
    Ok(())
}

/// Whether another computer records into the note `session_id` and has not
/// saved the recording (KTD9): this computer does no work on the note (its
/// row is Saved and idle), and the records in the home folder say
/// "recording not saved". A folder whose files cannot be read gives
/// `false`, and the rename goes on as before.
fn not_saved_elsewhere(store: &Store, session_id: &str, claimed: bool) -> Result<bool, StageError> {
    let record = require(store, session_id)?;
    if busy(&record, claimed) {
        return Ok(false);
    }
    Ok(record
        .home_path
        .as_deref()
        .and_then(|home| read_files(Path::new(home)))
        .is_some_and(|files| files.doc.result_flag() == Some(ResultFlag::NotSaved)))
}

fn require(store: &Store, session_id: &str) -> Result<SessionRecord, StageError> {
    store
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))
}

pub(crate) fn to_intent(m: &Move) -> RenameIntent {
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
