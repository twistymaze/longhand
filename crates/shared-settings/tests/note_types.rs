//! The effective note type list (U4 of the note types plan): the shared
//! list plus each type folder that holds a note (R2, R3, R7, R34, KTD9).

use session_save::naming::TypeNameError;
use shared_settings::note_types::{
    NoteType, NoteTypeError, Renamed, TypeFolder, add_type, default_after, default_to_save,
    delete_type, effective_default, effective_types, list_after_edit, list_to_save, rename_type,
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
                name: "Customer Calls".into(),
                valid: true,
                shared: false,
                notes: 2,
            },
            NoteType {
                name: "One on One".into(),
                valid: true,
                shared: true,
                notes: 0,
            },
            NoteType {
                name: "Pathfinder".into(),
                valid: true,
                shared: true,
                notes: 3,
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
fn a_shared_name_that_fails_the_check_or_repeats_is_not_listed() {
    let types = effective_types(
        &shared(&["Pathfinder", "1:1s", "PATHFINDER", " ", "General"]),
        &[],
    );
    assert_eq!(names(&types), ["Pathfinder"]);
}

#[test]
fn shared_and_folder_only_types_are_in_one_name_order_that_ignores_case() {
    let types = effective_types(
        &shared(&["Zeta", "alpha"]),
        &[folder("customer calls", 1), folder("Board", 1)],
    );
    assert_eq!(names(&types), ["alpha", "Board", "customer calls", "Zeta"]);
}

#[test]
fn a_type_added_later_with_an_earlier_name_is_listed_first() {
    // The list is in name order, not in the order of creation.
    let (list, _) = add_type(&shared(&["Pathfinder"]), &[], &[], "Board").unwrap();
    assert_eq!(list, ["Board", "Pathfinder"]);
    assert_eq!(names(&effective_types(&list, &[])), ["Board", "Pathfinder"]);
    // A shared list in the order of creation is listed in name order too.
    assert_eq!(
        names(&effective_types(&shared(&["Pathfinder", "Board"]), &[])),
        ["Board", "Pathfinder"]
    );
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
    assert_eq!(list, ["Board", "Pathfinder"]);
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
    assert_eq!(list, ["Board", "One on One"]);
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
    assert_eq!(list, ["Board", "Customer Calls", "Pathfinder"]);
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
        add_type(&[], &[], &[], "general"),
        Err(NoteTypeError::Name(TypeNameError::General))
    );
    assert_eq!(
        add_type(&[], &[], &[], "General").unwrap_err().to_string(),
        "General is a built-in type."
    );
}

#[test]
fn a_rename_to_general_is_refused() {
    // General is the built-in type of the notes at the top of the storage
    // folder. No type folder can have its name.
    assert_eq!(
        rename_type(
            &shared(&["Pathfinder"]),
            &[],
            &[],
            "Pathfinder",
            " GENERAL "
        ),
        Err(NoteTypeError::Name(TypeNameError::General))
    );
}

#[test]
fn a_folder_named_general_is_listed_but_not_valid_and_can_be_renamed() {
    // A folder that a user made in Finder. Its notes keep their folder, and
    // Settings offers a rename to a valid name or a delete (KTD9).
    let folders = [folder("general", 2)];
    let types = effective_types(&shared(&["General"]), &folders);
    assert_eq!(names(&types), ["general"]);
    assert!(!types[0].valid);
    assert_eq!(types[0].notes, 2);
    assert!(list_to_save(&shared(&["General"]), &folders).is_empty());
    let renamed = rename_type(&[], &folders, &[], "general", "Team").unwrap();
    assert_eq!(renamed.list, shared(&["Team"]));
    assert_eq!(renamed.from, "general");
}

#[test]
fn a_default_of_general_is_no_type_folder() {
    // The default `None` is General.
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &[], Some(" general ")),
        Ok((None, None))
    );
    let types = effective_types(&shared(&["Pathfinder"]), &[]);
    assert_eq!(effective_default(Some("General"), &types), None);
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
fn the_default_is_the_spelling_of_the_list_or_general() {
    // R7, R23.
    let types = effective_types(&shared(&["Pathfinder"]), &[]);
    assert_eq!(
        effective_default(Some("pathfinder"), &types).as_deref(),
        Some("Pathfinder")
    );
    assert_eq!(effective_default(None, &types), None);
    // A default that names no type is General.
    assert_eq!(effective_default(Some("Board"), &types), None);
}

#[test]
fn a_default_of_a_folder_only_type_also_writes_the_list() {
    let folders = [folder("Customer Calls", 1)];
    assert_eq!(
        default_to_save(&shared(&["Pathfinder"]), &folders, Some("customer calls")),
        Ok((
            Some("Customer Calls".to_string()),
            Some(shared(&["Customer Calls", "Pathfinder"]))
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

#[test]
fn a_rename_moves_the_type_to_the_name_order_place_of_its_new_name() {
    // R6.
    let renamed = rename_type(
        &shared(&["One on One", "Pathfinder"]),
        &[folder("Pathfinder", 3)],
        &[],
        "pathfinder",
        " D&D ",
    )
    .unwrap();
    assert_eq!(
        renamed,
        Renamed {
            list: shared(&["D&D", "One on One"]),
            from: "Pathfinder".into(),
            to: "D&D".into(),
            merged: false,
        }
    );
}

#[test]
fn a_rename_to_an_existing_type_merges_into_its_spelling() {
    // R6: "1 on 1" merges into "One on One".
    let renamed = rename_type(
        &shared(&["One on One", "1 on 1"]),
        &[folder("1 on 1", 2), folder("One on One", 1)],
        &[],
        "1 on 1",
        "one on one",
    )
    .unwrap();
    assert_eq!(
        renamed,
        Renamed {
            list: shared(&["One on One"]),
            from: "1 on 1".into(),
            to: "One on One".into(),
            merged: true,
        }
    );
}

#[test]
fn a_rename_that_changes_only_the_case_is_not_a_merge() {
    let renamed = rename_type(
        &shared(&["pathfinder"]),
        &[folder("pathfinder", 1)],
        &[],
        "pathfinder",
        "Pathfinder",
    )
    .unwrap();
    assert_eq!(renamed.list, shared(&["Pathfinder"]));
    assert!(!renamed.merged);
    assert_eq!(
        (renamed.from.as_str(), renamed.to.as_str()),
        ("pathfinder", "Pathfinder")
    );
}

#[test]
fn a_rename_is_refused_for_a_bad_name_a_home_folder_or_an_unknown_type() {
    let list = shared(&["Pathfinder"]);
    assert_eq!(
        rename_type(&list, &[], &[], "Pathfinder", "1:1s"),
        Err(NoteTypeError::Name(TypeNameError::ForbiddenCharacter(':')))
    );
    assert_eq!(
        rename_type(&list, &[], &shared(&["Board"]), "Pathfinder", "board"),
        Err(NoteTypeError::HomeFolder)
    );
    assert_eq!(
        rename_type(&list, &[], &[], "Board", "Games"),
        Err(NoteTypeError::NoSuchType)
    );
}

#[test]
fn a_folder_only_type_with_a_bad_name_can_be_renamed_to_a_valid_name() {
    // KTD9: Settings offers a rename to a valid name.
    let renamed = rename_type(
        &shared(&["Pathfinder"]),
        &[folder("con", 2)],
        &[],
        "con",
        "Console",
    )
    .unwrap();
    assert_eq!(renamed.list, shared(&["Console", "Pathfinder"]));
    assert_eq!(renamed.from, "con");
}

#[test]
fn a_delete_removes_the_type_and_writes_the_folder_only_types() {
    // R6.
    let (list, name) = delete_type(
        &shared(&["Pathfinder", "One on One"]),
        &[folder("Pathfinder", 30), folder("Board", 1)],
        "pathfinder",
    )
    .unwrap();
    assert_eq!(list, shared(&["Board", "One on One"]));
    assert_eq!(name, "Pathfinder");
    assert_eq!(
        delete_type(&shared(&["One on One"]), &[], "Pathfinder"),
        Err(NoteTypeError::NoSuchType)
    );
}

#[test]
fn the_default_follows_a_rename_and_is_general_after_a_delete() {
    // R7.
    assert_eq!(
        default_after(Some("pathfinder"), "Pathfinder", Some("D&D")),
        Some(Some("D&D".to_string()))
    );
    assert_eq!(
        default_after(Some("Pathfinder"), "Pathfinder", None),
        Some(None)
    );
    assert_eq!(default_after(Some("Board"), "Pathfinder", None), None);
    assert_eq!(default_after(None, "Pathfinder", None), None);
}

#[test]
fn a_list_edit_that_is_continued_changes_only_what_is_missing() {
    // KTD7: a launch continues an edit whose list save was lost.
    assert_eq!(
        list_after_edit(&shared(&["Pathfinder", "Board"]), "pathfinder", Some("D&D")),
        Some(shared(&["D&D", "Board"]))
    );
    assert_eq!(
        list_after_edit(&shared(&["D&D", "Board"]), "Pathfinder", Some("D&D")),
        None
    );
    assert_eq!(
        list_after_edit(
            &shared(&["One on One", "1 on 1"]),
            "1 on 1",
            Some("One on One")
        ),
        Some(shared(&["One on One"]))
    );
    assert_eq!(
        list_after_edit(&shared(&["Pathfinder"]), "Pathfinder", None),
        Some(shared(&[]))
    );
    assert_eq!(
        list_after_edit(&shared(&["Board"]), "Pathfinder", None),
        None
    );
    assert_eq!(
        list_after_edit(&shared(&["pathfinder"]), "pathfinder", Some("Pathfinder")),
        Some(shared(&["Pathfinder"]))
    );
}

#[test]
fn an_add_of_a_leaving_name_makes_a_type_with_no_notes() {
    // The notes of the leaving folder still move to their new type.
    let folders = [leaving("Pathfinder", 2)];
    let (list, _) = add_type(&shared(&["D&D"]), &folders, &[], "Pathfinder").unwrap();
    let types = effective_types(&list, &folders);
    assert_eq!(names(&types), ["D&D", "Pathfinder"]);
    assert_eq!(types[1].notes, 0);
}
