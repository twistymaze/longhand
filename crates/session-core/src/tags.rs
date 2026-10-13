//! Tag text rules (R9, R30, KTD10). A tag is free text. When the text
//! contains `::`, the part before the first `::` is the scope and the part
//! after it is the value.

/// The separator between the scope and the value of a tag.
pub const SCOPE_SEPARATOR: &str = "::";

/// The key of a tag (KTD10). Two tags are the same tag when their keys are
/// equal (R30). The key is the text with the spaces at its ends and the
/// spaces around each `::` removed, in lower case. Matching, the filter,
/// merge and rename use the key. The UI shows the text, not the key.
pub fn tag_key(text: &str) -> String {
    text.split(SCOPE_SEPARATOR)
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(SCOPE_SEPARATOR)
        .to_lowercase()
}

/// `tags` with one tag for each key: the first tag with a key stays (R8).
pub fn unique(tags: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut keys = Vec::new();
    let mut out = Vec::new();
    for tag in tags {
        let key = tag_key(&tag);
        if !keys.contains(&key) {
            keys.push(key);
            out.push(tag);
        }
    }
    out
}

/// The scope and the value of a scoped tag, with the spaces at their ends
/// removed. The split is at the first `::`, so the value can contain `::`.
/// A tag with no `::`, or with an empty scope or an empty value, has no
/// scope: it is a one-part tag, and this function gives `None`.
pub fn scope_and_value(text: &str) -> Option<(&str, &str)> {
    let (scope, value) = text.split_once(SCOPE_SEPARATOR)?;
    let (scope, value) = (scope.trim(), value.trim());
    (!scope.is_empty() && !value.is_empty()).then_some((scope, value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spellings_of_one_tag_give_one_key() {
        let key = tag_key("Person::Nathan");
        for text in [
            "person::nathan",
            " Person :: Nathan ",
            "PERSON::NATHAN",
            "\tPerson\t::  Nathan\n",
        ] {
            assert_eq!(tag_key(text), key, "{text:?}");
        }
        assert_eq!(key, "person::nathan");
    }

    #[test]
    fn spaces_inside_a_part_stay_in_the_key() {
        assert_eq!(tag_key("Customer :: Mayo Clinic"), "customer::mayo clinic");
        assert_ne!(tag_key("Mayo Clinic"), tag_key("MayoClinic"));
    }

    #[test]
    fn spaces_around_each_separator_are_removed() {
        assert_eq!(
            tag_key(" Topic :: Security :: Old "),
            tag_key("topic::security::old")
        );
    }

    #[test]
    fn different_tags_give_different_keys() {
        assert_ne!(tag_key("Person::Nathan"), tag_key("Person::Sarah"));
        assert_ne!(tag_key("Person::Nathan"), tag_key("Nathan"));
        assert_ne!(tag_key("follow-up"), tag_key("follow up"));
    }

    #[test]
    fn a_tag_without_a_separator_has_no_scope() {
        assert_eq!(scope_and_value("follow-up"), None);
        assert_eq!(scope_and_value("a:b"), None);
        assert_eq!(tag_key(" Follow-Up "), "follow-up");
    }

    #[test]
    fn a_scoped_tag_splits_at_the_first_separator() {
        assert_eq!(
            scope_and_value("Person::Nathan"),
            Some(("Person", "Nathan"))
        );
        assert_eq!(
            scope_and_value(" Customer :: Mayo Clinic "),
            Some(("Customer", "Mayo Clinic"))
        );
        assert_eq!(
            scope_and_value("Topic::Security::Old"),
            Some(("Topic", "Security::Old"))
        );
    }

    #[test]
    fn an_empty_scope_or_value_gives_a_one_part_tag() {
        for text in ["::x", "Person::", " :: x", "x ::  ", "::", "  ::  "] {
            assert_eq!(scope_and_value(text), None, "{text:?}");
        }
    }

    #[test]
    fn the_split_keeps_the_case_of_the_text() {
        assert_eq!(
            scope_and_value("person::NATHAN"),
            Some(("person", "NATHAN"))
        );
    }
}
