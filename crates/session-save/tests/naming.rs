//! Folder names built from the session start and its title (KTD7, R30).

use session_save::naming::{MAX_TITLE_CHARS, folder_name};

fn started() -> chrono::DateTime<chrono::FixedOffset> {
    chrono::DateTime::parse_from_rfc3339("2026-09-24T14:05:09-05:00").unwrap()
}

const NO_TITLE: &str = "2026-09-24 1405 Longhand session";

fn title_part(name: &str) -> &str {
    name.strip_prefix("2026-09-24 1405 ").expect(name)
}

#[test]
fn a_title_follows_the_date_and_time() {
    assert_eq!(
        folder_name(started(), Some("Budget review")),
        "2026-09-24 1405 Budget review"
    );
}

#[test]
fn forbidden_characters_are_removed_and_spaces_collapsed() {
    assert_eq!(
        folder_name(started(), Some("Budget: Q3 / review?")),
        "2026-09-24 1405 Budget Q3 review"
    );
    assert_eq!(
        folder_name(started(), Some("a\"b*c<d>e|f\\g")),
        "2026-09-24 1405 abcdefg"
    );
    assert_eq!(
        folder_name(started(), Some("tab\there\nnew\u{7}line")),
        "2026-09-24 1405 tab here newline"
    );
}

#[test]
fn no_title_or_an_unusable_title_gives_the_no_title_name() {
    assert_eq!(folder_name(started(), None), NO_TITLE);
    assert_eq!(folder_name(started(), Some("")), NO_TITLE);
    assert_eq!(folder_name(started(), Some(" . .. . ")), NO_TITLE);
    assert_eq!(folder_name(started(), Some("?/:*")), NO_TITLE);
}

#[test]
fn spaces_and_dots_at_either_end_and_a_leading_dot_are_removed() {
    assert_eq!(
        folder_name(started(), Some("  .hidden plan.  ")),
        "2026-09-24 1405 hidden plan"
    );
    assert_eq!(
        folder_name(started(), Some("...v2 notes...")),
        "2026-09-24 1405 v2 notes"
    );
}

#[test]
fn a_long_title_is_cut_without_a_trailing_space_or_dot() {
    let spaced = format!("{} {}", "a".repeat(MAX_TITLE_CHARS - 1), "b".repeat(180));
    let name = folder_name(started(), Some(&spaced));
    assert_eq!(title_part(&name), "a".repeat(MAX_TITLE_CHARS - 1));

    let dotted = format!("{}.{}", "a".repeat(MAX_TITLE_CHARS - 1), "b".repeat(180));
    let name = folder_name(started(), Some(&dotted));
    assert_eq!(title_part(&name), "a".repeat(MAX_TITLE_CHARS - 1));

    let plain = "c".repeat(300);
    let name = folder_name(started(), Some(&plain));
    assert_eq!(title_part(&name).chars().count(), MAX_TITLE_CHARS);
}

#[test]
fn a_long_title_is_cut_by_characters_not_bytes() {
    let title = "é".repeat(300);
    let name = folder_name(started(), Some(&title));
    assert_eq!(title_part(&name), "é".repeat(MAX_TITLE_CHARS));
}

#[test]
fn a_reserved_title_gives_a_safe_name() {
    for title in ["CON", "con:", "Nul", "LPT1", "com9.txt", "desktop.ini"] {
        let name = folder_name(started(), Some(title));
        let part = title_part(&name);
        assert!(!part.is_empty(), "{title}");
        assert!(
            !session_save::naming::is_reserved(part),
            "{title} gave {name}"
        );
    }
    assert_eq!(folder_name(started(), Some("CON")), "2026-09-24 1405 CON_");
    // A reserved word inside a longer title is a normal title.
    assert_eq!(
        folder_name(started(), Some("Con planning")),
        "2026-09-24 1405 Con planning"
    );
}

/// Type names (R28, KTD8). A type name becomes a folder name, so the check
/// refuses each hostile name with a reason.
mod type_names {
    use session_save::naming::{MAX_TITLE_CHARS, TypeNameError, check_type_name, same_type_name};

    #[test]
    fn usual_names_pass() {
        for name in [
            "Pathfinder",
            "One on One",
            "Customer calls",
            "Café 2026",
            "1-1s",
            // Only "General" is kept for the built-in type.
            "No type",
            "General Meetings",
        ] {
            assert_eq!(check_type_name(name).as_deref(), Ok(name), "{name}");
        }
    }

    #[test]
    fn spaces_at_the_ends_are_removed() {
        assert_eq!(
            check_type_name("  Pathfinder ").as_deref(),
            Ok("Pathfinder")
        );
    }

    #[test]
    fn unusable_names_are_refused_with_a_reason() {
        let table: &[(&str, TypeNameError)] = &[
            ("", TypeNameError::Empty),
            ("   ", TypeNameError::Empty),
            ("\t\n", TypeNameError::Empty),
            ("1:1s", TypeNameError::ForbiddenCharacter(':')),
            ("a/b", TypeNameError::ForbiddenCharacter('/')),
            ("a\\b", TypeNameError::ForbiddenCharacter('\\')),
            ("a|b?", TypeNameError::ForbiddenCharacter('|')),
            (".", TypeNameError::StartsWithDot),
            ("..", TypeNameError::StartsWithDot),
            ("../x", TypeNameError::StartsWithDot),
            (".hidden", TypeNameError::StartsWithDot),
            (".longhand", TypeNameError::StartsWithDot),
            (".LONGHAND", TypeNameError::StartsWithDot),
            ("Notes.", TypeNameError::EndsWithDot),
            ("a\u{0}b", TypeNameError::ControlCharacter),
            ("a\tb", TypeNameError::ControlCharacter),
            ("con", TypeNameError::Reserved),
            ("CON", TypeNameError::Reserved),
            ("com1.txt", TypeNameError::Reserved),
            ("desktop.ini", TypeNameError::Reserved),
            ("General", TypeNameError::General),
            ("general", TypeNameError::General),
            (" GENERAL ", TypeNameError::General),
        ];
        for (name, expected) in table {
            let refused = check_type_name(name).unwrap_err();
            assert_eq!(&refused, expected, "{name:?}");
            assert!(!refused.to_string().is_empty(), "{name:?}");
        }
    }

    #[test]
    fn a_name_longer_than_the_title_limit_is_refused() {
        let at_limit = "é".repeat(MAX_TITLE_CHARS);
        assert_eq!(check_type_name(&at_limit).as_deref(), Ok(at_limit.as_str()));
        let over = "é".repeat(MAX_TITLE_CHARS + 1);
        assert_eq!(check_type_name(&over), Err(TypeNameError::TooLong));
    }

    #[test]
    fn names_compare_without_regard_to_case() {
        assert!(same_type_name("pathfinder", "Pathfinder"));
        assert!(same_type_name(" One on One", "ONE ON ONE "));
        assert!(same_type_name("Café", "CAFÉ"));
        assert!(!same_type_name("Pathfinder", "Pathfinders"));
    }
}
