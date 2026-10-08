//! The type move (U5 of the note types and tags plan, R5, R6, R27, R29,
//! R35): a type change puts the home folder in the folder of the new type.
//!
//! The move follows the owed-rename recipe (KTD5):
//! - The computer where the user changes the type records a `type_moves`
//!   row before the move. The row holds no path: the move finds the home
//!   folder by session id at move time.
//! - When no computer works on the note (KTD6), the folder moves at once.
//!   Otherwise the change writes `longhand_pending_type` into `session.md`,
//!   so every computer shows the new type, and the move waits.
//! - Each library scan and each launch runs the owed moves again. A move
//!   is done when the folder is at the target, found there by session id.
//! - Only the computer with the row moves the folder. Another computer
//!   takes over a move only after 7 days (KTD13).
//!
//! At rest, the folder decides the type (KTD3): when the folder is not in
//! the `from` type folder any more, the move is done or a person moved the
//! folder, and the row and the pending type go.
//!
//! A type rename, merge or delete is a set of per-note moves (KTD7), never
//! a rename of the type folder. Only a rename that changes the case alone
//! renames the type folder, because a case-insensitive volume sees the two
//! names as one folder.

use std::io;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, FixedOffset, TimeDelta};
use library::scan::{self, Place};
use session_core::folder::{PendingType, ResultFlag, SessionDoc};
use session_core::jobs::{Claims, busy};
use session_core::{BulkEdit, MoveStep, SessionRecord, SessionState, Store, TypeMove};
use session_save::naming::{check_type_name, same_type_name};
use session_save::rename_home::{self, RenameError};
use session_save::{Fs, home_write};

use crate::save::SESSION_FILE;
use crate::{SharedStore, StageError, folder_rename, lock};

/// A pending type this old can be taken over by any computer (KTD13, R35).
pub const TAKEOVER_AFTER: TimeDelta = TimeDelta::days(7);

/// A computer that took over a move waits this long, then moves the folder
/// only when the pending type still names it (KTD13).
pub const TAKEOVER_WAIT: TimeDelta = TimeDelta::minutes(10);

/// The files that do not keep an old type folder (KTD7).
pub const SYSTEM_FILES: [&str; 2] = [".DS_Store", "Thumbs.db"];

/// What a type move reads from this computer.
pub struct Context<'a> {
    /// This computer's runs: their in-use markers are this computer's own
    /// work (KTD5 of the notes without a recording plan).
    pub own_runs: &'a [String],
    /// The work on a note that its state does not show (KTD6).
    pub claims: &'a Claims,
    pub now: DateTime<FixedOffset>,
    pub fs: &'a dyn Fs,
}

/// What a type change did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeChange {
    /// The note has this type already. A waiting change back to the type
    /// of the folder is cleared.
    Unchanged,
    /// The home folder is not written yet. The row has the type, and the
    /// home folder is made in the folder of the type (U7).
    NoFolder,
    /// The home folder is in the folder of the new type at this path.
    Moved(PathBuf),
    /// A computer works on the note, or the storage folder is not
    /// available: the folder moves later. The note shows the new type now.
    Later,
    /// The move failed, and the folder moves later. The note shows the new
    /// type now. The text says why.
    Kept(String),
}

/// The type that the app shows for the note of `record` (KTD3), and
/// whether a move waits: the pending type while the home folder is still
/// in the `from` folder, else the type of the folder.
pub fn shown_type(record: &SessionRecord) -> (Option<String>, bool) {
    match &record.pending_type {
        Some(pending) if same_type(&pending.from, &record.note_type) => (pending.to.clone(), true),
        _ => (record.note_type.clone(), false),
    }
}

/// Whether two types are one type. `None` is "No type".
fn same_type(a: &Option<String>, b: &Option<String>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same_type_name(a, b),
        _ => false,
    }
}

/// The user changes the type of the note `session_id` to `to`, or to "No
/// type" for `None` (R5). The caller checks that `to` is a type of the
/// effective list, and holds the store lock while the Move does not hold
/// the note ([`crate::move_notes::lock_unheld`]). `remove_empty_from` is
/// for a type rename, merge or delete: the old type folder goes when the
/// last note leaves it (KTD7).
pub fn change_type_locked(
    store: &mut Store,
    session_id: &str,
    to: Option<&str>,
    remove_empty_from: bool,
    ctx: &Context,
) -> Result<TypeChange, StageError> {
    let to = to
        .map(check_type_name)
        .transpose()
        .map_err(|e| StageError::Failed(e.to_string()))?;
    let claimed = ctx.claims.busy(session_id);
    crate::follow::follow_folder(store, session_id, claimed)?;
    let record = require(store, session_id)?;
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        // A type change before the early write is done changes only the
        // row (U7).
        store.set_note_type(session_id, to.as_deref())?;
        return Ok(TypeChange::NoFolder);
    };
    // The type of the folder now. While the storage folder is not
    // available, the row has the type of the last scan.
    let from = scan::storage_place(&home).map_or_else(|| record.note_type.clone(), |p| p.note_type);
    if same_type(&to, &from) {
        // The note has the type, or a change goes back to the type of the
        // folder: the waiting change goes (KTD3).
        store.clear_type_move(session_id)?;
        if record.pending_type.is_some() {
            clear_pending(store, session_id, &home)?;
        }
        return Ok(TypeChange::Unchanged);
    }
    store.set_type_move(
        session_id,
        &TypeMove {
            to,
            from,
            changed_at: ctx.now,
            taken_over_at: None,
            remove_empty_from,
            field_written: false,
        },
    )?;
    settle(store, session_id, ctx, true)
}

/// On each launch and each library scan: run each move that this computer
/// owes, take over the moves that waited for 7 days (KTD13), and clear a
/// pending type of this computer that no move needs (KTD3). Returns the
/// ids of the notes that changed. A failure on one note does not stop the
/// others. Only with the instance lock held, so no other run on this
/// computer moves the same folders.
pub fn run_owed(store: &SharedStore, ctx: &Context) -> Result<Vec<String>, StageError> {
    let (owed, me) = {
        let guard = lock(store);
        (guard.type_moves()?, guard.computer_id()?)
    };
    let mut changed = Vec::new();
    for (id, _) in owed {
        let mut guard = crate::move_notes::lock_unheld(store, ctx.claims, &id);
        match settle(&mut guard, &id, ctx, false) {
            Ok(TypeChange::Moved(_)) => changed.push(id),
            Ok(TypeChange::Kept(why)) => {
                log::warn!("the folder of {id} stays in its old type folder: {why}")
            }
            Ok(_) => {}
            Err(e) => log::warn!("could not run the type move of {id}: {e}"),
        }
    }
    let waiting: Vec<SessionRecord> = lock(store)
        .sessions()?
        .into_iter()
        .filter(|r| r.pending_type.is_some())
        .collect();
    for record in waiting {
        let id = record.id.clone();
        let mut guard = crate::move_notes::lock_unheld(store, ctx.claims, &id);
        let result = if record.pending_type.as_ref().is_some_and(|p| p.by == me) {
            own_field(&mut guard, &id, ctx)
        } else {
            take_over(&mut guard, &id, ctx)
        };
        match result {
            Ok(true) => changed.push(id),
            Ok(false) => {}
            Err(e) => log::warn!("could not check the pending type of {id}: {e}"),
        }
    }
    changed.sort();
    changed.dedup();
    Ok(changed)
}

/// Run the type move that this computer owes for `session_id`, when no
/// computer works on the note. With `claim`, the user changed the type on
/// this computer now: the pending type names this computer, also when it
/// named another computer.
fn settle(
    store: &mut Store,
    session_id: &str,
    ctx: &Context,
    claim: bool,
) -> Result<TypeChange, StageError> {
    let Some(mut owed) = store.type_move(session_id)? else {
        return Ok(TypeChange::Unchanged);
    };
    let Some(mut record) = store.session(session_id)? else {
        return Ok(TypeChange::Unchanged);
    };
    // A move that a crash stopped is settled first, so the move starts
    // from where the folder is.
    if let Some(intent) = store.rename_intent(session_id)? {
        folder_rename::settle(store, &record, &intent, ctx.fs)?;
        record = require(store, session_id)?;
    }
    let me = store.computer_id()?;
    let pending = PendingType {
        to: owed.to.clone(),
        from: owed.from.clone(),
        by: me.clone(),
        at: owed.taken_over_at.unwrap_or(owed.changed_at),
    };
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        return Ok(TypeChange::Later);
    };
    let Some(place) = scan::storage_place(&home) else {
        // The storage folder is not available: the move waits (KTD6). The
        // app shows the new type.
        if claim {
            store.set_pending_type(session_id, Some(&pending))?;
        }
        return Ok(TypeChange::Later);
    };
    if !rename_home::exists_exactly(&home) {
        log::warn!(
            "the folder of {session_id} is gone, so its type move is dropped: {}",
            home.display()
        );
        store.clear_type_move(session_id)?;
        return Ok(TypeChange::Unchanged);
    }
    let Some(doc) = read_document(&home) else {
        return Ok(TypeChange::Later);
    };
    if !claim {
        match &doc.pending_type {
            Some(field) if field.by != me => {
                log::info!(
                    "the type move of {session_id} is now the move of computer {}",
                    field.by
                );
                store.clear_type_move(session_id)?;
                return Ok(TypeChange::Unchanged);
            }
            None if owed.field_written => {
                log::info!("the type change of {session_id} was cleared on another computer");
                store.clear_type_move(session_id)?;
                store.set_pending_type(session_id, None)?;
                return Ok(TypeChange::Unchanged);
            }
            _ => {}
        }
    }
    if !same_type(&place.note_type, &owed.from) {
        // The move is done, or a person moved the folder: the folder wins
        // (KTD3, R29).
        if doc.pending_type.is_some() {
            clear_pending(store, session_id, &home)?;
        }
        store.set_pending_type(session_id, None)?;
        store.set_note_type(session_id, place.note_type.as_deref())?;
        store.clear_type_move(session_id)?;
        if same_type(&place.note_type, &owed.to) {
            return Ok(TypeChange::Moved(home));
        }
        log::info!(
            "the folder of {session_id} is in another type folder now, so its type move is dropped"
        );
        return Ok(TypeChange::Unchanged);
    }
    if let Some(at) = owed.taken_over_at
        && ctx.now.signed_duration_since(at) < TAKEOVER_WAIT
    {
        return Ok(TypeChange::Later);
    }
    if !free(store, &record, &home, &doc, ctx)? {
        write_pending(store, session_id, &home, doc, &pending, &mut owed)?;
        return Ok(TypeChange::Later);
    }
    let parent = match &owed.to {
        None => Ok(place.root.clone()),
        Some(name) => type_folder(&place.root, name),
    };
    let moved = parent.and_then(|parent| {
        let name = home
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| RenameError::Missing(home.clone()))?;
        rename_home::rename_into(&home, &parent, name, session_id, ctx.fs, &mut |m| {
            store
                .set_rename_intent(session_id, &folder_rename::to_intent(m))
                .map_err(|e| RenameError::Record(e.to_string()))
        })
    });
    match moved {
        Ok(path) if path == home => {
            store.clear_rename_intent(session_id)?;
            Ok(TypeChange::Unchanged)
        }
        Ok(path) => {
            finish(store, session_id, &path, &place, &owed)?;
            log::info!(
                "moved the folder of {session_id} from {} to {}",
                home.display(),
                path.display()
            );
            Ok(TypeChange::Moved(path))
        }
        Err(e) => {
            // The error can come after the move, for example from the
            // fsync of the parent. The recorded intent tells where the
            // folder is.
            if let Some(intent) = store.rename_intent(session_id)?
                && let Some(path) = folder_rename::settle(store, &record, &intent, ctx.fs)?
                && path != home
            {
                finish(store, session_id, &path, &place, &owed)?;
                return Ok(TypeChange::Moved(path));
            }
            log::warn!("could not move the folder of {session_id}: {e}");
            if let Some(doc) = read_document(&home) {
                write_pending(store, session_id, &home, doc, &pending, &mut owed)?;
            }
            Ok(TypeChange::Kept(e.to_string()))
        }
    }
}

/// The move is done: the row follows the folder at `path`, and the pending
/// type and the row of the move go. The old type folder goes when the move
/// is part of a type rename, merge or delete and the folder holds nothing
/// else (KTD7).
fn finish(
    store: &Store,
    session_id: &str,
    path: &Path,
    place: &Place,
    owed: &TypeMove,
) -> Result<(), StageError> {
    store.set_home_path(session_id, &path.to_string_lossy())?;
    store.clear_rename_intent(session_id)?;
    let note_type = scan::storage_place(path).and_then(|p| p.note_type);
    store.set_note_type(session_id, note_type.as_deref())?;
    clear_pending(store, session_id, path)?;
    store.clear_type_move(session_id)?;
    if owed.remove_empty_from
        && let Some(from) = &owed.from
    {
        remove_empty_type_folder(&place.root, from);
    }
    Ok(())
}

/// Whether no computer works on the note (KTD6): no fresh in-use marker of
/// another computer's run, no `result: pending` in `session.md`, and this
/// computer does not record, queue, process, resume, regenerate, remove
/// audio from or Move the note. A folder whose markers cannot be read
/// counts as in use.
fn free(
    store: &Store,
    record: &SessionRecord,
    home: &Path,
    doc: &SessionDoc,
    ctx: &Context,
) -> Result<bool, StageError> {
    let moving = !matches!(store.move_step(&record.id)?, None | Some(MoveStep::Done));
    if busy(record, ctx.claims.busy(&record.id)) || moving {
        return Ok(false);
    }
    if doc.result_flag() == Some(ResultFlag::NotSaved) {
        return Ok(false);
    }
    Ok(!crate::in_use::in_use_except(home, ctx.own_runs, ctx.now).unwrap_or(true))
}

/// Write `pending` into `session.md` in `home` with a read-modify-write of
/// `doc`, so every computer shows the new type, and into the row. A write
/// that fails is logged, and the next scan tries again.
fn write_pending(
    store: &Store,
    session_id: &str,
    home: &Path,
    mut doc: SessionDoc,
    pending: &PendingType,
    owed: &mut TypeMove,
) -> Result<(), StageError> {
    store.set_pending_type(session_id, Some(pending))?;
    if doc.pending_type.as_ref() != Some(pending) {
        doc.pending_type = Some(pending.clone());
        if let Err(e) = home_write::write_atomic(home, SESSION_FILE, doc.render().as_bytes()) {
            log::warn!(
                "could not write the pending type into {SESSION_FILE} in {}: {e}",
                home.display()
            );
            return Ok(());
        }
    }
    if !owed.field_written {
        owed.field_written = true;
        store.set_type_move(session_id, owed)?;
    }
    Ok(())
}

/// Remove the pending type from `session.md` in `home` and from the row. A
/// write that fails is logged: the folder decides the type (KTD3).
fn clear_pending(store: &Store, session_id: &str, home: &Path) -> Result<(), StageError> {
    store.set_pending_type(session_id, None)?;
    let Some(mut doc) = read_document(home) else {
        return Ok(());
    };
    if doc.pending_type.take().is_none() {
        return Ok(());
    }
    if let Err(e) = home_write::write_atomic(home, SESSION_FILE, doc.render().as_bytes()) {
        log::warn!(
            "could not remove the pending type from {SESSION_FILE} in {}: {e}",
            home.display()
        );
    }
    Ok(())
}

/// A pending type that names this computer and has no row (KTD3): a move
/// that this computer owes when the folder is still in `from`, so the row
/// comes back. Otherwise the pending type is stale and goes. Returns
/// whether the row of the note changed.
fn own_field(store: &mut Store, session_id: &str, ctx: &Context) -> Result<bool, StageError> {
    if store.type_move(session_id)?.is_some() {
        return Ok(false);
    }
    let Some(record) = store.session(session_id)? else {
        return Ok(false);
    };
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        return Ok(false);
    };
    let Some(place) = scan::storage_place(&home) else {
        return Ok(false);
    };
    let Some(field) = read_document(&home).and_then(|doc| doc.pending_type) else {
        return Ok(false);
    };
    if field.by != store.computer_id()? {
        return Ok(false);
    }
    if same_type(&place.note_type, &field.from) {
        log::info!("this computer owes the type move of {session_id} again");
        store.set_type_move(
            session_id,
            &TypeMove {
                to: field.to,
                from: field.from,
                changed_at: field.at,
                taken_over_at: None,
                remove_empty_from: false,
                field_written: true,
            },
        )?;
        let moved = settle(store, session_id, ctx, false)?;
        return Ok(matches!(moved, TypeChange::Moved(_)));
    }
    clear_pending(store, session_id, &home)?;
    Ok(true)
}

/// Take over the move of another computer when its pending type is 7 days
/// old and no computer works on the note (KTD13, R35): write this computer
/// into `by` and record the move. The move itself waits for
/// [`TAKEOVER_WAIT`]. Returns whether the row of the note changed.
fn take_over(store: &mut Store, session_id: &str, ctx: &Context) -> Result<bool, StageError> {
    if store.type_move(session_id)?.is_some() {
        return Ok(false);
    }
    let Some(record) = store.session(session_id)? else {
        return Ok(false);
    };
    let Some(seen) = record.pending_type.clone() else {
        return Ok(false);
    };
    if ctx.now.signed_duration_since(seen.at) < TAKEOVER_AFTER {
        return Ok(false);
    }
    let Some(home) = record.home_path.as_deref().map(PathBuf::from) else {
        return Ok(false);
    };
    let Some(place) = scan::storage_place(&home) else {
        return Ok(false);
    };
    if !same_type(&place.note_type, &seen.from) {
        return Ok(false);
    }
    let Some(mut doc) = read_document(&home) else {
        return Ok(false);
    };
    // The file must still have the field that the row has.
    if doc.pending_type.as_ref() != Some(&seen) {
        return Ok(false);
    }
    // The target comes from a synced file: it goes through the type name
    // check (KTD8).
    if let Some(to) = &seen.to
        && check_type_name(to).as_deref() != Ok(to.as_str())
    {
        log::warn!(
            "the pending type of {session_id} is not a valid type name, so it is not taken over"
        );
        return Ok(false);
    }
    if !free(store, &record, &home, &doc, ctx)? {
        return Ok(false);
    }
    let me = store.computer_id()?;
    let taken = PendingType {
        by: me,
        at: ctx.now,
        ..seen.clone()
    };
    // The row comes first: a crash after it finds another computer in
    // `by` and drops the row.
    store.set_type_move(
        session_id,
        &TypeMove {
            to: seen.to.clone(),
            from: seen.from.clone(),
            changed_at: ctx.now,
            taken_over_at: Some(ctx.now),
            remove_empty_from: false,
            field_written: true,
        },
    )?;
    doc.pending_type = Some(taken.clone());
    if let Err(e) = home_write::write_atomic(&home, SESSION_FILE, doc.render().as_bytes()) {
        log::warn!("could not take over the type move of {session_id}: {e}");
        store.clear_type_move(session_id)?;
        return Ok(false);
    }
    store.set_pending_type(session_id, Some(&taken))?;
    log::info!(
        "took over the type move of {session_id} from computer {}",
        seen.by
    );
    Ok(true)
}

/// The folder of the type `name` in `root`: the folder whose name is the
/// same type, with the spelling of the disk, or a new folder `name`. The
/// name goes through the type name check (KTD8).
fn type_folder(root: &Path, name: &str) -> Result<PathBuf, RenameError> {
    let refused = |why: String| RenameError::Io(io::Error::new(io::ErrorKind::InvalidInput, why));
    let checked = check_type_name(name).map_err(|e| refused(e.to_string()))?;
    if checked != name {
        return Err(refused(format!("\"{name}\" is not a valid type name")));
    }
    for entry in std::fs::read_dir(root)?.flatten() {
        let found = entry.file_name();
        let Some(found) = found.to_str() else {
            continue;
        };
        if same_type_name(found, name) {
            return if entry.file_type()?.is_dir() {
                Ok(entry.path())
            } else {
                Err(refused(format!(
                    "a file has the name of the type folder {found}"
                )))
            };
        }
    }
    let folder = root.join(name);
    match std::fs::create_dir(&folder) {
        Ok(()) => {
            session_save::fsync::sync_dir(root)?;
            Ok(folder)
        }
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(folder),
        Err(e) => Err(e.into()),
    }
}

/// Remove the type folder `name` in `root` when it holds nothing except
/// system files (KTD7). Returns whether the folder went. A name that is
/// not one plain folder name, and a folder with anything else in it, stay.
pub fn remove_empty_type_folder(root: &Path, name: &str) -> bool {
    let mut parts = Path::new(name).components();
    if !matches!(
        (parts.next(), parts.next()),
        (Some(Component::Normal(_)), None)
    ) || name.starts_with('.')
    {
        return false;
    }
    let folder = root.join(name);
    if !rename_home::exists_exactly(&folder) {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return false;
    };
    let mut system = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else {
            return false;
        };
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        let named = entry
            .file_name()
            .to_str()
            .is_some_and(|n| SYSTEM_FILES.contains(&n));
        if !is_file || !named {
            return false;
        }
        system.push(entry.path());
    }
    for file in system {
        if let Err(e) = std::fs::remove_file(&file) {
            log::warn!("could not remove {}: {e}", file.display());
            return false;
        }
    }
    match std::fs::remove_dir(&folder) {
        Ok(()) => {
            log::info!("removed the empty type folder {}", folder.display());
            true
        }
        Err(e) => {
            log::warn!("could not remove the type folder {}: {e}", folder.display());
            false
        }
    }
}

/// What one pass of a type rename, merge or delete did (KTD7).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EditPass {
    /// The notes that the pass changed.
    pub changed: Vec<String>,
    /// Every note of the old type has moved, or has its pending type and
    /// its row: the edit is done.
    pub done: bool,
}

/// The per-note part of a type rename, merge or delete in the storage
/// folder `root` (KTD7, R6): each note that shows the old type changes to
/// the new type, or to "No type" for a delete, as a user's type change
/// does. The shared list changes before this. A pass can run again: a note
/// that shows the new type is done. `edit_id` names the temporary folder of
/// a rename that changes the case alone.
pub fn apply_edit(
    store: &SharedStore,
    root: &Path,
    edit_id: i64,
    edit: &BulkEdit,
    ctx: &Context,
) -> Result<EditPass, StageError> {
    let (from, to) = match edit {
        BulkEdit::RenameType { from, to } => (from.as_str(), Some(to.as_str())),
        BulkEdit::DeleteType { name } => (name.as_str(), None),
        BulkEdit::RenameTag { .. } | BulkEdit::DeleteTag { .. } => {
            return Err(StageError::Failed(format!("{edit:?} is not a type edit")));
        }
    };
    if let Some(to) = to
        && same_type_name(from, to)
    {
        return rename_case(store, root, edit_id, from, to, ctx);
    }
    let old = Some(from.to_string());
    let roots = [root.to_path_buf()];
    let shows_old = |record: &SessionRecord| {
        record.state != SessionState::Discarded
            && record
                .home_path
                .as_deref()
                .is_some_and(|home| scan::place_in(Path::new(home), &roots).is_some())
            && same_type(&shown_type(record).0, &old)
    };
    let ids: Vec<String> = lock(store)
        .sessions()?
        .into_iter()
        .filter(|r| shows_old(r))
        .map(|r| r.id)
        .collect();
    let mut pass = EditPass {
        done: true,
        ..EditPass::default()
    };
    for id in ids {
        let mut guard = crate::move_notes::lock_unheld(store, ctx.claims, &id);
        // The row can change before the lock.
        if !guard.session(&id)?.is_some_and(|r| shows_old(&r)) {
            continue;
        }
        match change_type_locked(&mut guard, &id, to, true, ctx) {
            Ok(change) => {
                pass.changed.push(id.clone());
                let waits = guard.type_move(&id)?;
                if matches!(change, TypeChange::Later | TypeChange::Kept(_))
                    && !waits.is_some_and(|w| w.field_written)
                {
                    pass.done = false;
                }
            }
            Err(e) => {
                log::warn!("could not change the type of {id}: {e}");
                pass.done = false;
            }
        }
    }
    remove_empty_type_folder(root, from);
    Ok(pass)
}

/// A type rename that changes the case alone (KTD7): the type folder takes
/// the new name through a temporary name, because a case-insensitive
/// volume sees the two names as one folder. The rename waits while a
/// computer works on a note in the folder (KTD6).
fn rename_case(
    store: &SharedStore,
    root: &Path,
    edit_id: i64,
    from: &str,
    to: &str,
    ctx: &Context,
) -> Result<EditPass, StageError> {
    let temp = root.join(rename_home::temp_name(&format!("type-{edit_id}")));
    let target = root.join(to);
    let guard = lock(store);
    let found = if rename_home::exists_exactly(&temp) {
        // A crash stopped the rename at the temporary name.
        Some(temp.clone())
    } else {
        std::fs::read_dir(root)?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| same_type_name(n, from) && n != to)
            })
    };
    let Some(folder) = found else {
        return Ok(EditPass {
            changed: Vec::new(),
            done: true,
        });
    };
    let notes: Vec<SessionRecord> = guard
        .sessions()?
        .into_iter()
        .filter(|r| {
            r.home_path
                .as_deref()
                .is_some_and(|home| Path::new(home).parent() == Some(folder.as_path()))
        })
        .collect();
    if folder != temp {
        for record in &notes {
            let home = Path::new(record.home_path.as_deref().unwrap_or_default());
            let free = match read_document(home) {
                Some(doc) => free(&guard, record, home, &doc, ctx)?,
                None => false,
            };
            if !free {
                return Ok(EditPass {
                    changed: Vec::new(),
                    done: false,
                });
            }
        }
        ctx.fs.rename_new(&folder, &temp)?;
        session_save::fsync::sync_dir(root)?;
    }
    if let Err(e) = ctx.fs.rename_new(&temp, &target) {
        // The folder never stays under the hidden name.
        ctx.fs.rename_new(&temp, &folder)?;
        return Err(e.into());
    }
    session_save::fsync::sync_dir(root)?;
    let mut changed = Vec::new();
    for record in notes {
        let Some(name) = record
            .home_path
            .as_deref()
            .and_then(|home| Path::new(home).file_name())
        else {
            continue;
        };
        guard.set_home_path(&record.id, &target.join(name).to_string_lossy())?;
        guard.set_note_type(&record.id, Some(to))?;
        changed.push(record.id);
    }
    drop(guard);
    Ok(EditPass {
        changed,
        done: true,
    })
}

fn read_document(folder: &Path) -> Option<SessionDoc> {
    std::fs::read_to_string(folder.join(SESSION_FILE))
        .ok()
        .and_then(|text| SessionDoc::parse(&text).ok())
}

fn require(store: &Store, session_id: &str) -> Result<SessionRecord, StageError> {
    store
        .session(session_id)?
        .ok_or_else(|| StageError::Failed(format!("no session {session_id}")))
}
