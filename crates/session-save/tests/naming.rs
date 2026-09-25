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
