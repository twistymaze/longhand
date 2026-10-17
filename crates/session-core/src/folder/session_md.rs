//! The user fields of a `session.md`: its title and its tags (KTD3).

use serde::Deserialize;

use super::{FolderError, frontmatter_bounds, parse_mapping};
use crate::note_folder::SESSION_FILE;

/// The user fields of a `session.md`: its title and its tags. The read
/// accepts any other key, because the file is the user's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionMd {
    /// A number or a boolean reads as its text.
    pub title: Option<String>,
    /// The `longhand_tags` list. One text value reads as one tag.
    pub tags: Vec<String>,
    /// `longhand_tags` holds an item that is not text: null, a list or a
    /// mapping, or `longhand_tags` is a mapping. [`SessionMd::tags`] does
    /// not hold such an item, so a write of the tags would remove it.
    pub tags_not_text: bool,
}

impl SessionMd {
    /// Parse the frontmatter of a `session.md`. An error gives the line in
    /// the file.
    pub fn parse(text: &str) -> Result<Self, FolderError> {
        let (start, end, _) = frontmatter_bounds(text)
            .ok_or(FolderError::MissingFrontmatter { file: SESSION_FILE })?;
        let yaml = &text[start..end];
        let map = parse_mapping(SESSION_FILE, yaml)?;
        // The parsed values give the kind of each value. The text of a
        // scalar comes from a second read as text, so that `1.50` stays
        // `1.50` and does not become `1.5`.
        let mut tags_not_text = false;
        let tags = match map.get("longhand_tags") {
            Some(serde_yaml::Value::Sequence(items)) => {
                if items.iter().all(|item| scalar_text(item).is_some()) {
                    serde_yaml::from_str::<TagTexts>(yaml)
                        .map(|t| t.longhand_tags)
                        .unwrap_or_else(|_| items.iter().filter_map(scalar_text).collect())
                } else {
                    tags_not_text = true;
                    items.iter().filter_map(scalar_text).collect()
                }
            }
            Some(serde_yaml::Value::Null) | None => Vec::new(),
            Some(value) => match scalar_text(value) {
                Some(text) => vec![
                    serde_yaml::from_str::<TagText>(yaml)
                        .map(|t| t.longhand_tags)
                        .unwrap_or(text),
                ],
                None => {
                    tags_not_text = true;
                    Vec::new()
                }
            },
        };
        let title = map.get("title").and_then(scalar_text).map(|text| {
            serde_yaml::from_str::<TitleText>(yaml)
                .map(|t| t.title)
                .unwrap_or(text)
        });
        Ok(Self {
            title,
            tags,
            tags_not_text,
        })
    }
}

/// The `title` of a frontmatter, read as the text in the file.
#[derive(Deserialize)]
struct TitleText {
    title: String,
}

/// One text value of `longhand_tags`, read as the text in the file.
#[derive(Deserialize)]
struct TagText {
    longhand_tags: String,
}

/// The items of `longhand_tags`, read as the text in the file.
#[derive(Deserialize)]
struct TagTexts {
    longhand_tags: Vec<String>,
}

fn scalar_text(value: &serde_yaml::Value) -> Option<String> {
    match value {
        serde_yaml::Value::String(text) => Some(text.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
