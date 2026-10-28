//! The changelog in the binary (R12, KTD2, KTD4 of the monthly changelog
//! plan). `build.rs` puts a `changelog.json` file into `OUT_DIR`, and the
//! binary contains it, so Recent changes needs no network. A file that this
//! build cannot read gives the "not available" changelog.

use std::collections::HashSet;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use specta::Type;

/// The format number of `changelog.json` that this build can read.
const FORMAT: u64 = 1;

/// The `changelog.json` text that `build.rs` staged for this build.
const BUNDLED_TEXT: &str = include_str!(concat!(env!("OUT_DIR"), "/changelog.json"));

/// One change in the changelog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ChangelogEntry {
    /// The full commit id on main.
    pub id: String,
    /// The commit time on main, in seconds since 1970 (UTC).
    pub time: i64,
    /// The month of the change, `YYYY-MM`, in UTC.
    pub month: String,
    /// The heading of the group, for example "Fixes".
    pub group: String,
    /// The position of the group heading: 0 is the first heading.
    pub rank: u32,
    /// The line text, the same as in the release notes.
    pub line: String,
    /// The PR number of the line, or none.
    pub pr: Option<u32>,
}

/// The commit that the changelog was made from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ChangelogHead {
    /// The full commit id.
    pub id: String,
    /// The commit time, in seconds since 1970 (UTC).
    pub time: i64,
}

/// A changelog, or "not available" (`available` is false, with no head and
/// no entries).
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct Changelog {
    pub available: bool,
    pub head: Option<ChangelogHead>,
    /// The entries in the order of the file (the order of main).
    pub entries: Vec<ChangelogEntry>,
}

impl Changelog {
    fn not_available() -> Self {
        Changelog {
            available: false,
            head: None,
            entries: Vec::new(),
        }
    }
}

/// The fields of a `changelog.json` file with format 1.
#[derive(Deserialize)]
struct File {
    available: bool,
    head: Option<ChangelogHead>,
    #[serde(default)]
    entries: Vec<ChangelogEntry>,
}

/// Read a `changelog.json` text. A parse error, a format other than 1,
/// `available: false`, or an available file with no head gives "not
/// available".
pub fn parse(text: &str) -> Changelog {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return Changelog::not_available();
    };
    // Read the format number first: a file with another format can have
    // other fields.
    if value.get("format").and_then(serde_json::Value::as_u64) != Some(FORMAT) {
        return Changelog::not_available();
    }
    match serde_json::from_value::<File>(value) {
        Ok(File {
            available: true,
            head: Some(head),
            entries,
        }) => Changelog {
            available: true,
            head: Some(head),
            entries,
        },
        _ => Changelog::not_available(),
    }
}

/// The commit ids of the entries in a changelog.
pub(crate) fn ids(changelog: &Changelog) -> HashSet<String> {
    changelog
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect()
}

/// The bundled changelog and its commit ids, parsed one time.
struct Bundled {
    changelog: Changelog,
    ids: HashSet<String>,
}

static BUNDLED: LazyLock<Bundled> = LazyLock::new(|| {
    let changelog = parse(BUNDLED_TEXT);
    let ids = ids(&changelog);
    Bundled { changelog, ids }
});

/// The time of the commit that the bundled changelog was made from. None
/// when the bundled changelog is not available.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "U6 of the monthly changelog plan reads it")
)]
pub(crate) fn bundled_head_time() -> Option<i64> {
    BUNDLED.changelog.head.as_ref().map(|head| head.time)
}

/// True when the bundled changelog has an entry with this full commit id.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "U6 of the monthly changelog plan reads it")
)]
pub(crate) fn bundled_contains(id: &str) -> bool {
    BUNDLED.ids.contains(id)
}

/// Return the changelog that this build contains.
#[tauri::command]
#[specta::specta]
pub fn changelog() -> Changelog {
    BUNDLED.changelog.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"{"format":1,"available":true,
        "head":{"id":"cccccccccccccccccccccccccccccccccccccccc","time":1761600000},
        "entries":[
          {"id":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","time":1761500000,"month":"2025-10",
           "group":"Fixes","rank":1,"line":"Fix the timer (#12)","pr":12},
          {"id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","time":1761400000,"month":"2025-10",
           "group":"New","rank":0,"line":"Add tags","pr":null}
        ]}"#;

    fn not_available() -> Changelog {
        Changelog::not_available()
    }

    #[test]
    fn a_valid_file_gives_its_head_and_its_entries_in_file_order() {
        let changelog = parse(VALID);
        assert!(changelog.available);
        assert_eq!(
            changelog.head,
            Some(ChangelogHead {
                id: "c".repeat(40),
                time: 1761600000
            })
        );
        let lines: Vec<&str> = changelog.entries.iter().map(|e| e.line.as_str()).collect();
        assert_eq!(lines, ["Fix the timer (#12)", "Add tags"]);
        assert_eq!(changelog.entries[0].pr, Some(12));
        assert_eq!(changelog.entries[1].pr, None);
        assert_eq!(changelog.entries[0].month, "2025-10");
        assert_eq!(changelog.entries[1].rank, 0);
    }

    #[test]
    fn an_unknown_format_number_gives_not_available() {
        let text = VALID.replacen(r#""format":1"#, r#""format":2"#, 1);
        assert_eq!(parse(&text), not_available());
        assert_eq!(
            parse(r#"{"available":true,"head":{"id":"c","time":1},"entries":[]}"#),
            not_available()
        );
    }

    #[test]
    fn empty_or_broken_text_gives_not_available() {
        assert_eq!(parse(""), not_available());
        assert_eq!(parse("{\"format\":1,"), not_available());
        assert_eq!(parse("not json"), not_available());
    }

    #[test]
    fn available_false_gives_not_available() {
        assert_eq!(parse(r#"{"format":1,"available":false}"#), not_available());
    }

    #[test]
    fn the_id_check_finds_each_entry_id_and_no_other_id() {
        let ids = ids(&parse(VALID));
        assert!(ids.contains(&"a".repeat(40)));
        assert!(ids.contains(&"b".repeat(40)));
        assert!(!ids.contains(&"c".repeat(40)));
        assert!(!ids.contains("aaaaaaa"));
    }

    #[test]
    fn the_bundled_changelog_reads() {
        let changelog = changelog();
        assert_eq!(bundled_head_time(), changelog.head.map(|head| head.time));
        for entry in &changelog.entries {
            assert!(bundled_contains(&entry.id));
        }
        assert!(!bundled_contains("not a commit id"));
    }
}
