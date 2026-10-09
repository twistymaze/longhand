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
//! General is the built-in type of the notes whose home folder is directly
//! in the storage folder. It has no type folder, so it is not in this list,
//! and the type name check refuses its name. A folder named "General" is
//! therefore a type that is not valid.
//!
//! Nothing here reads a file. The caller gives the type folders from the
//! store.

use std::collections::BTreeMap;

use session_save::naming::{GENERAL, TypeNameError, check_type_name, same_type_name};

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

/// The effective type list: the valid names of the shared list and the
/// folder-only types, in name order that ignores case. Two names that
/// differ only in case are one type, with the spelling of the shared list.
pub fn effective_types(shared: &[String], folders: &[TypeFolder]) -> Vec<NoteType> {
    // The notes of a leaving folder have another type now.
    let notes_in = |name: &str| {
        folders
            .iter()
            .filter(|f| !f.leaving && same_type_name(&f.name, name))
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
    types.extend(folder_only);
    // The list is in name order, so a type is easy to find.
    types.sort_by_cached_key(|t| t.name.to_lowercase());
    types
}

/// Put the names `list` in name order that ignores case: the order of the
/// effective list.
fn sort_names(list: &mut [String]) {
    list.sort_by_cached_key(|name| name.to_lowercase());
}

/// The shared list that a change of the list writes: each valid type of
/// the effective list, in name order. Folder-only types go into the list
/// here (KTD9).
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
    sort_names(&mut list);
    Ok((list, name))
}

/// What a type rename writes (R6, KTD7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renamed {
    /// The shared list after the rename.
    pub list: Vec<String>,
    /// The renamed type, with the spelling of the effective list.
    pub from: String,
    /// The type that the notes get: the new name, or the type that the
    /// renamed type merges into, with the spelling of the list.
    pub to: String,
    /// `to` is another type: the two types merge.
    pub merged: bool,
}

/// The type `name` of the effective list, valid or not.
fn find_type(shared: &[String], folders: &[TypeFolder], name: &str) -> Option<NoteType> {
    effective_types(shared, folders)
        .into_iter()
        .find(|t| same_type_name(&t.name, name))
}

/// The shared list after the user renames the type `old` to `new` (R6).
/// The new name goes through the type name check (KTD8). A new name of
/// another type merges the two types. A new name that differs only in case
/// is not a merge. `homes` are the names of the home folders directly in
/// the storage folder.
pub fn rename_type(
    shared: &[String],
    folders: &[TypeFolder],
    homes: &[String],
    old: &str,
    new: &str,
) -> Result<Renamed, NoteTypeError> {
    let from = find_type(shared, folders, old)
        .ok_or(NoteTypeError::NoSuchType)?
        .name;
    let new = check_type_name(new)?;
    let mut list = list_to_save(shared, folders);
    let place = list.iter().position(|t| same_type_name(t, &from));
    if !same_type_name(&from, &new)
        && let Some(other) = effective_types(shared, folders)
            .into_iter()
            .find(|t| t.valid && same_type_name(&t.name, &new))
    {
        list.retain(|t| !same_type_name(t, &from));
        return Ok(Renamed {
            list,
            from,
            to: other.name,
            merged: true,
        });
    }
    if !same_type_name(&from, &new) && homes.iter().any(|home| same_type_name(home, &new)) {
        return Err(NoteTypeError::HomeFolder);
    }
    match place {
        Some(i) => list[i] = new.clone(),
        None => list.push(new.clone()),
    }
    sort_names(&mut list);
    Ok(Renamed {
        list,
        from,
        to: new,
        merged: false,
    })
}

/// The shared list after the user deletes the type `name` (R6), and the
/// name with the spelling of the effective list.
pub fn delete_type(
    shared: &[String],
    folders: &[TypeFolder],
    name: &str,
) -> Result<(Vec<String>, String), NoteTypeError> {
    let name = find_type(shared, folders, name)
        .ok_or(NoteTypeError::NoSuchType)?
        .name;
    let mut list = list_to_save(shared, folders);
    list.retain(|t| !same_type_name(t, &name));
    Ok((list, name))
}

/// The default type after a rename of `old` to `new`, or a delete of `old`
/// for `None` (R7): `Some` when the default names `old`, else `None`.
pub fn default_after(
    default: Option<&str>,
    old: &str,
    new: Option<&str>,
) -> Option<Option<String>> {
    default
        .is_some_and(|default| same_type_name(default, old))
        .then(|| new.map(str::to_string))
}

/// The id of the summary prompt that the type `note_type` chooses in the
/// map `prompts`, or `None` for "Default" (R1, R18, KTD1). `None` is
/// General. `valid` tells that the name of the type passes the type name
/// check, and it has no effect for General. A type that is not valid has
/// no choice, so a folder named "General" does not get the choice of the
/// built-in General. The names compare without case.
pub fn prompt_choice<'a>(
    prompts: &'a BTreeMap<String, String>,
    note_type: Option<&str>,
    valid: bool,
) -> Option<&'a str> {
    let name = match note_type {
        None => GENERAL,
        Some(name) if valid => name,
        Some(_) => return None,
    };
    prompts
        .get(name)
        .or_else(|| {
            prompts
                .iter()
                .find(|(key, _)| same_type_name(key, name))
                .map(|(_, id)| id)
        })
        .map(String::as_str)
}

/// Set the choice of the type `name` in the map `prompts` to the prompt
/// `prompt_id`, or to "Default" for `None` (R1, KTD6). An entry of each
/// other spelling of the name goes.
pub fn set_prompt_choice(
    prompts: &mut BTreeMap<String, String>,
    name: &str,
    prompt_id: Option<&str>,
) {
    prompts.retain(|key, _| !same_type_name(key, name));
    if let Some(id) = prompt_id {
        prompts.insert(name.to_string(), id.to_string());
    }
}

/// The map `prompts` after the edit of the type `from` (R6, KTD6), or
/// `None` when the edit changes nothing. A rename to `to` moves the entry
/// of `from` to `to`. A merge into `to` (`merged`) keeps the entry of
/// `to` and removes the entry of `from`. A delete (`to` is `None`) removes
/// the entry of `from`. An add of a type uses a delete of its name, to
/// remove an old entry with that name. A type with no entry changes
/// nothing, so a launch can continue an edit whose map is saved already.
pub fn prompts_after_edit(
    prompts: &BTreeMap<String, String>,
    from: &str,
    to: Option<&str>,
    merged: bool,
) -> Option<BTreeMap<String, String>> {
    let mut map = prompts.clone();
    let mut id = None;
    map.retain(|key, value| {
        let keep = !same_type_name(key, from);
        if !keep && (id.is_none() || key == from) {
            id = Some(value.clone());
        }
        keep
    });
    let id = id?;
    if let Some(to) = to.filter(|_| !merged) {
        set_prompt_choice(&mut map, to, Some(&id));
    }
    Some(map)
}

/// The map `prompts` without each entry that chooses the prompt
/// `prompt_id`, or `None` when no entry chooses it (R5, KTD6). The types
/// of those entries then use the default prompt.
pub fn prompts_without(
    prompts: &BTreeMap<String, String>,
    prompt_id: &str,
) -> Option<BTreeMap<String, String>> {
    let mut map = prompts.clone();
    map.retain(|_, id| id != prompt_id);
    (map.len() != prompts.len()).then_some(map)
}

/// The shared list `shared` with the edit of `from` to `to` (a rename or a
/// merge), or with `from` removed for `None`. `None` when the list has the
/// edit. A launch uses this to continue an edit whose save was lost
/// (KTD7), so it refuses nothing.
pub fn list_after_edit(shared: &[String], from: &str, to: Option<&str>) -> Option<Vec<String>> {
    let mut list = shared.to_vec();
    let place = list.iter().position(|t| same_type_name(t, from));
    let has_to = |list: &[String]| to.is_none_or(|to| list.iter().any(|t| t == to));
    match (place, to) {
        (None, _) if has_to(&list) => return None,
        (None, Some(to)) => list.push(to.to_string()),
        (None, None) => return None,
        (Some(i), Some(to)) if !list.iter().any(|t| t == to) => list[i] = to.to_string(),
        (Some(i), _) => {
            if list[i] == to.unwrap_or_default() {
                return None;
            }
            list.remove(i);
        }
    }
    Some(list)
}

/// The type that a new note gets from the shared default (R7, R23): the
/// spelling of the effective list, or `None` for General. A default that
/// names no valid type of the effective list is General.
pub fn effective_default(default: Option<&str>, types: &[NoteType]) -> Option<String> {
    let default = default?;
    types
        .iter()
        .find(|t| t.valid && same_type_name(&t.name, default))
        .map(|t| t.name.clone())
}

/// What a change of the default type to `name` writes (R7): the default,
/// with the spelling of the effective list, and the shared list when the
/// type is not in it yet (a folder-only type). `None` and the name
/// "General" are General: the default is no type folder.
pub fn default_to_save(
    shared: &[String],
    folders: &[TypeFolder],
    name: Option<&str>,
) -> Result<(Option<String>, Option<Vec<String>>), NoteTypeError> {
    let Some(name) = name.filter(|name| !same_type_name(name, GENERAL)) else {
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
