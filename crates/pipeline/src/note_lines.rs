//! The line form of notes from before the notes document (R26): each note is
//! one `- [hh:mm:ss] text`, `- [hh:mm:ss ~gap] text` or `- text` line.
//!
//! Until the summarizer reads the notes document whole (U5), it reads the
//! document in this form, one line for each note.

use session_core::Anchor;
use session_core::folder::parse_position;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteLine {
    pub anchor: Anchor,
    pub text: String,
}

/// Read one line of the document as a note. `None` for a blank line. A line
/// that is not a list item is a note with no position and the whole line as
/// its text. A note with empty text keeps its position.
pub fn parse(line: &str) -> Option<NoteLine> {
    if line.trim().is_empty() {
        return None;
    }
    let rest = line
        .strip_prefix("- ")
        .or_else(|| (line.trim_end() == "-").then_some(""));
    let Some(rest) = rest else {
        return Some(NoteLine {
            anchor: Anchor::Unanchored,
            text: line.to_string(),
        });
    };
    let anchored = rest
        .strip_prefix('[')
        .and_then(|r| r.split_once("] "))
        .and_then(|(tag, text)| {
            let (time, gap_anchored) = match tag.strip_suffix(" ~gap") {
                Some(time) => (time, true),
                None => (tag, false),
            };
            parse_position(time).map(|position_ms| NoteLine {
                anchor: Anchor::At {
                    position_ms,
                    gap_anchored,
                },
                text: text.to_string(),
            })
        });
    Some(anchored.unwrap_or_else(|| NoteLine {
        anchor: Anchor::Unanchored,
        text: rest.to_string(),
    }))
}

/// Every note in the document, in its order.
pub fn parse_all(document: &str) -> Vec<NoteLine> {
    document.lines().filter_map(parse).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_form_is_read() {
        for (line, anchor) in [
            ("- [bracketed] thought", Anchor::Unanchored),
            (
                "- [00:12:40] [bracketed] thought",
                Anchor::At {
                    position_ms: 760_000,
                    gap_anchored: false,
                },
            ),
            (
                "- [01:02:03 ~gap] [bracketed] thought",
                Anchor::At {
                    position_ms: 3_723_000,
                    gap_anchored: true,
                },
            ),
        ] {
            assert_eq!(
                parse(line),
                Some(NoteLine {
                    anchor,
                    text: "[bracketed] thought".into()
                })
            );
        }
        assert_eq!(parse("   "), None);
    }

    #[test]
    fn an_emptied_note_keeps_its_position() {
        let anchor = Anchor::At {
            position_ms: 1_000,
            gap_anchored: false,
        };
        let empty = NoteLine {
            anchor,
            text: String::new(),
        };
        assert_eq!(parse("- [00:00:01] "), Some(empty));
        assert_eq!(parse("-").unwrap().text, "");
    }
}
