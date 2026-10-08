//! The effective note type list (R2, R34, KTD9): the shared `note_types`
//! plus each type folder that holds a note.
//!
//! A read of the list never writes the shared list (rule 1 of the shared
//! settings rules). A folder-only type goes into the shared list when the
//! user next changes the list or the default type, with two exceptions:
//!
//! - A type that is leaving: each note in its folder has a
//!   `longhand_pending_type` with that folder as `from`. It is not in the
//!   effective list. Every computer applies this rule, because the pending
//!   field syncs with the note.
//! - A folder whose name fails the type name check. It is in the effective
//!   list, so its notes keep their type and Home can filter them, but it is
//!   not offered for a new note or a type change.
//!
//! Nothing here reads a file. The caller gives the type folders from the
//! store.

use session_save::naming::{TypeNameError, check_type_name, same_type_name};

/// A type folder in the storage folder: a folder that holds at least one
/// home folder (KTD4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeFolder {
    /// The folder name, as the disk gives it.
    pub name: String,
    /// The number of notes in the folder.
    pub notes: usize,
    /// Each note in the folder has a `longhand_pending_type` with this
    /// folder as `from`.
    pub leaving: bool,
}

/// One entry of the effective type list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteType {
    pub name: String,
    /// The name passes the type name check. Only a valid type is offered
    /// for a new note or a type change, and only a valid type goes into the
    /// shared list.
    pub valid: bool,
    /// The shared list has the type. False for a folder-only type.
    pub shared: bool,
    /// The number of notes in the type folder.
    pub notes: usize,
}

/// Why a type name or a default type is refused. The text is for the UI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NoteTypeError {
    #[error(transparent)]
    Name(#[from] TypeNameError),
    #[error("A type with this name already exists.")]
    Duplicate,
    #[error("A note folder in the storage folder has this name.")]
    HomeFolder,
    #[error("That type no longer exists.")]
    NoSuchType,
}

/// The effective type list: the valid names of the shared list in their
/// order, then the folder-only types in name order. Two names that differ
/// only in case are one type, with the spelling of the shared list.
pub fn effective_types(shared: &[String], folders: &[TypeFolder]) -> Vec<NoteType> {
    let notes_in = |name: &str| {
        folders
            .iter()
            .filter(|f| same_type_name(&f.name, name))
            .map(|f| f.notes)
            .sum()
    };
    let mut types: Vec<NoteType> = Vec::new();
    for name in shared {
        let Ok(name) = check_type_name(name) else {
            continue;
        };
        if types.iter().any(|t| same_type_name(&t.name, &name)) {
            continue;
        }
        types.push(NoteType {
            notes: notes_in(&name),
            name,
            valid: true,
            shared: true,
        });
    }
    let mut folder_only: Vec<NoteType> = Vec::new();
    for folder in folders {
        if folder.leaving
            || types
                .iter()
                .chain(&folder_only)
                .any(|t| same_type_name(&t.name, &folder.name))
        {
            continue;
        }
        folder_only.push(NoteType {
            name: folder.name.clone(),
            valid: check_type_name(&folder.name).as_deref() == Ok(folder.name.as_str()),
            shared: false,
            notes: notes_in(&folder.name),
        });
    }
    folder_only.sort_by_key(|t| t.name.to_lowercase());
    types.extend(folder_only);
    types
}

/// The shared list that a change of the list writes: each valid type of
/// the effective list. Folder-only types go into the list here (KTD9).
pub fn list_to_save(shared: &[String], folders: &[TypeFolder]) -> Vec<String> {
    effective_types(shared, folders)
        .into_iter()
        .filter(|t| t.valid)
        .map(|t| t.name)
        .collect()
}

/// The shared list after the user adds the type `name` (R3), and the name
/// with the spaces at its ends removed. `homes` are the names of the home
/// folders directly in the storage folder: a type folder cannot have such a
/// name (KTD8).
pub fn add_type(
    shared: &[String],
    folders: &[TypeFolder],
    homes: &[String],
    name: &str,
) -> Result<(Vec<String>, String), NoteTypeError> {
    let name = check_type_name(name)?;
    if effective_types(shared, folders)
        .iter()
        .any(|t| same_type_name(&t.name, &name))
    {
        return Err(NoteTypeError::Duplicate);
    }
    if homes.iter().any(|home| same_type_name(home, &name)) {
        return Err(NoteTypeError::HomeFolder);
    }
    let mut list = list_to_save(shared, folders);
    list.push(name.clone());
    Ok((list, name))
}

/// The type that a new note gets from the shared default (R7, R23): the
/// spelling of the effective list, or `None` for "No type". A default that
/// names no valid type of the effective list is "No type".
pub fn effective_default(default: Option<&str>, types: &[NoteType]) -> Option<String> {
    let default = default?;
    types
        .iter()
        .find(|t| t.valid && same_type_name(&t.name, default))
        .map(|t| t.name.clone())
}

/// What a change of the default type to `name` writes (R7): the default,
/// with the spelling of the effective list, and the shared list when the
/// type is not in it yet (a folder-only type). `None` is "No type".
pub fn default_to_save(
    shared: &[String],
    folders: &[TypeFolder],
    name: Option<&str>,
) -> Result<(Option<String>, Option<Vec<String>>), NoteTypeError> {
    let Some(name) = name else {
        return Ok((None, None));
    };
    let types = effective_types(shared, folders);
    let chosen = types
        .iter()
        .find(|t| t.valid && same_type_name(&t.name, name))
        .ok_or(NoteTypeError::NoSuchType)?;
    let list = (!chosen.shared).then(|| list_to_save(shared, folders));
    Ok((Some(chosen.name.clone()), list))
}
