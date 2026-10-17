---
title: A YAML parse changed the text of session.md tags and titles, and a tag write removed items that were not text
date: 2026-10-17
category: logic-errors
module: session.md reader (crates/session-core/src/folder/session_md.rs, crates/session-save/src/note_files.rs, crates/pipeline/src/tags.rs)
problem_type: logic_error
component: service_layer
severity: medium
symptoms:
  - "A tag `1.50` in session.md showed as `1.5`, and a tag edit wrote `1.5` back"
  - "A title or tag `yes` or `true` was read as a boolean"
  - "A tag edit removed an item of longhand_tags that was null, a list or a mapping"
root_cause: wrong_api
resolution_type: code_fix
framework_version: rust 1.94.0
tags:
  - session-md
  - yaml
  - serde-yaml
  - tags
  - agent-edited-files
---

# A YAML parse changed the text of session.md tags and titles, and a tag write removed items that were not text

## Problem

`session.md` is the user's file. A person or an agent can type `longhand_tags: [1.50, yes]` with no quotes. The reader parsed the frontmatter into `serde_yaml::Value` and took the text of each scalar from that value. `serde_yaml` 0.9 reads an unquoted `1.50` as the number 1.5 and `yes`/`true` as booleans, so the text of the tag changed. A tag edit then wrote the changed text back into the file (PR #30).

An item that is not text (null, a list or a mapping) gave no tag. A tag edit wrote the list of tags that the reader found, so the edit removed that item from the file.

## Root cause

A `Value` keeps the parsed value of a scalar, not its text in the file. The number `1.50` and the number `1.5` are the same `Value`.

## Solution

Use the `Value` only to find the kind of each item. Read the text of the scalars with a second, typed parse into `String` fields. `serde_yaml` gives the source text of a plain scalar when the target type is `String`:

```rust
#[derive(Deserialize)]
struct TagTexts {
    longhand_tags: Vec<String>,
}

// The parsed values give the kind of each value. The text of a scalar
// comes from a second read as text, so that `1.50` stays `1.50`.
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
```

`TitleText` and `TagText` do the same for `title` and for a `longhand_tags` that is one scalar.

When `longhand_tags` has an item that is not text, `SessionMd::tags_not_text` is true. A tag edit then writes nothing. `patch_session_md_tags` returns `TagsNotText`, and the tag command shows `TAGS_NOT_TEXT`: the user must fix the file, because Longhand cannot keep that item and also write the new tags.

## Prevention

When Longhand reads a value from a file that a person or an agent can edit, and it can write that value back, read the source text. Do not render the text again from a parsed value. Test with unquoted values that YAML reads as other types: `1.50`, `007`, `yes`, `true`, `null`.
