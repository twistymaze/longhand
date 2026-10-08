//! The effective note type list (U4 of the note types plan): the shared
//! list plus each type folder that holds a note (R2, R3, R7, R34, KTD9).

use session_save::naming::TypeNameError;
use shared_settings::note_types::{
    NoteType, NoteTypeError, TypeFolder, add_type, default_to_save, effective_default,
    effective_types, list_to_save,
};

fn shared(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn folder(name: &str, notes: usize) -> TypeFolder {
    TypeFolder {
        name: name.to_string(),
        notes,
        leaving: false,
    }
}

fn leaving(name: &str, notes: usize) -> TypeFolder {
    TypeFolder {
        leaving: true,
        ..folder(name, notes)
    }
}

fn names(types: &[NoteType]) -> Vec<&str> {
    types.iter().map(|t| t.name.as_str()).collect()
}

#[test]
fn the_list_holds_a_shared_type_with_no_folder_and_a_folder_type_not_in_the_shared_list() {
    // R2, R34, KTD9.
    let types = effective_types(
        &shared(&["Pathfinder", "One on One"]),
        &[folder("Pathfinder", 3), folder("Customer Calls", 2)],
    );
    assert_eq!(
        types,
        [
            NoteType {
                name: "Pathfinder".into(),
                valid: true,
                shared: true,
                notes: 3,
            },
            NoteType {
                name: "One on One".into(),
                valid: true,
                shared: true,
                notes: 0,
            },
            NoteType {
                name: "Customer Calls".into(),
                valid: true,
                shared: false,
                notes: 2,
            },
        ]
    );
}

#[test]
fn a_folder_that_differs_only_in_case_is_the_shared_type_with_the_shared_spelling() {
    let types = effective_types(&shared(&["Pathfinder"]), &[folder("pathfinder", 4)]);
    assert_eq!(names(&types), ["Pathfinder"]);
    assert_eq!(types[0].notes, 4);
}

#[test]
fn a_shared_name_that_fails_the_check_or_repeats_gives_no_type() {
    let types = effective_types(
        &shared(&["Pathfinder", "1:1s", "PATHFINDER", " ", "No type"]),
        &[],
    );
    assert_eq!(names(&types), ["Pathfinder"]);
}

#[test]
fn folder_only_types_follow_the_shared_list_in_name_order() {
    let types = effective_types(
        &shared(&["Zeta"]),
        &[folder("customer calls", 1), folder("Board", 1)],
    );
    assert_eq!(names(&types), ["Zeta", "Board", "customer calls"]);
}

#[test]
fn a_folder_named_con_is_listed_but_not_valid_and_never_goes_into_the_shared_list() {
    // KTD9: Home can filter it, but a new note cannot use it.
    let folders = [folder("con", 2), folder("Pathfinder", 1)];
    let types = effective_types(&[], &folders);
    let con = types.iter().find(|t| t.name == "con").unwrap();
    assert!(!con.valid);
    assert_eq!(con.notes, 2);
    assert_eq!(list_to_save(&[], &folders), ["Pathfinder"]);
    let (list, _) = add_type(&[], &folders, &[], "Board").unwrap();
    assert_eq!(list, ["Pathfinder", "Board"]);
    assert_eq!(
        default_to_save(&[], &folders, Some("con")),
        Err(NoteTypeError::NoSuchType)
    );
    assert_eq!(effective_default(Some("con"), &types), None);
}

#[test]
fn a_leaving_type_is_not_listed_and_does_not_come_back_after_a_list_edit() {
    // KTD9: B deleted "Pathfinder" while one of its notes was busy on A.
    // The note keeps its pending type with "Pathfinder" as `from`. A third
    // computer adds a type and must not write "Pathfinder" back.
    let folders = [leaving("Pathfinder", 1), folder("One on One", 1)];
    let after_delete_on_b = shared(&["One on One"]);
    assert_eq!(
        names(&effective_types(&after_delete_on_b, &folders)),
        ["One on One"]
    );
    let (list, name) = add_type(&after_delete_on_b, &folders, &[], "Board").unwrap();
    assert_eq!(name, "Board");
    assert_eq!(list, ["One on One", "Board"]);
}

#[test]
fn a_leaving_folder_of_a_shared_type_keeps_the_shared_type() {
    let types = effective_types(&shared(&["Pathfinder"]), &[leaving("Pathfinder", 1)]);
    assert_eq!(names(&types), ["Pathfinder"]);
}

#[test]
fn an_add_puts_the_folder_only_types_into_the_shared_list() {
    // KTD9: the first edit of the list adds the folder-only types.
    let (list, name) = add_type(
        &shared(&["Pathfinder"]),
        &[folder("Customer Calls", 1)],
        &[],
        "  Board  ",
    )
    .unwrap();
    assert_eq!(name, "Board");
    assert_eq!(list, ["Pathfinder", "Customer Calls", "Board"]);
}

#[test]
fn an_add_of_1_1s_is_refused_with_the_reason_of_the_name_check() {
    assert_eq!(
        add_type(&[], &[], &[], "1:1s"),
        Err(NoteTypeError::Name(TypeNameError::ForbiddenCharacter(':')))
    );
    assert_eq!(
        add_type(&[], &[], &[], "1:1s").unwrap_err().to_string(),
        TypeNameError::ForbiddenCharacter(':').to_string()
    );
    assert_eq!(
        add_type(&[], &[], &[], "no TYPE"),
        Err(NoteTypeError::Name(TypeNameError::NoType))
    );
}

#[test]
fn an_add_of_pathfinder_when_pathfinder_exists_is_a_duplicate() {
    assert_eq!(
        add_type(&shared(&["Pathfinder"]), &[], &[], "pathfinder"),
        Err(NoteTypeError::Duplicate)
    );
    // A folder-only type counts too.
    assert_eq!(
        add_type(&[], &[folder("Pathfinder", 1)], &[], "PATHFINDER"),
        Err(NoteTypeError::Duplicate)
    );
}

#[test]
fn an_add_with_the_name_of_a_home_folder_in_the_storage_folder_is_refused() {
    // KTD8.
    assert_eq!(
        add_type(
            &[],
            &[],
            &shared(&["2026-10-07 0900 Standup"]),
            "2026-10-07 0900 standup"
        ),
        Err(NoteTypeError::HomeFolder)
    );
}

#[test]
fn the_default_is_the_spelling_of_the_list_or_no_type() {
    // R7, R23.
    let types = effective_types(&shared(&["Pathfinder"]), &[]);
    assert_eq!(
        effective_default(Some("pathfinder"), &types).as_deref(),
        Some("Pathfinder")
    );
    assert_eq!(effective_default(None, &types), None);
    // A default that names no type is "No type".
    assert_eq!(effective_default(Some("Board"), &types), None);
}

#[test]
fn a_default_of_a_folder_only_type_also_writes_the_list() {
    let folders = [folder("Customer Calls", 1)];
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &folders, Some("customer calls")),
        Ok((
            Some("Customer Calls".to_string()),
            Some(shared(&["Pathfinder", "Customer Calls"]))
        ))
    );
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &folders, Some("Pathfinder")),
        Ok((Some("Pathfinder".to_string()), None))
    );
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &folders, None),
        Ok((None, None))
    );
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &folders, Some("Board")),
        Err(NoteTypeError::NoSuchType)
    );
}
