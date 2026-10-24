# The layout of a storage folder

```text
<storage folder>/
  .longhand/
    library                      the library marker
    settings.json                the shared settings
    background.md                the Background
    prompts/
      Weekly review.md           one file for each summary prompt
    imports/
      <sha256>/
        <computer id>.json       the import record of one audio file
  2026-10-01 0900 Budget review/ a note of the type General
    session.md
    notes.md
    transcript.md
    summary.md
    longhand.json
    recognition.jsonl
    audio/
      segment-01.opus
  Meetings/                      a type folder: the note type "Meetings"
    2026-10-02 1400 Hiring sync/ a note of the type Meetings
      ...
```

## Note folders

- Each note has one note folder. A note folder has a `session.md` or a `longhand.json` file.
- A note folder directly in the storage folder is a note of the type General.
- A note folder in a type folder is a note of that type. A type folder has the name of the note type. It has no `session.md`.
- Longhand reads note folders only at these two levels. A note folder at a lower level is not a note.
- Longhand does not read hidden names (names that start with `.`).
- The name of a note folder is `YYYY-MM-DD HHMM <title>`. The time is the start time of the note in its own time zone. `folder_name.py` gives the name. Two notes with one name get `-2`, `-3` and so on.

## The files of a note folder

| File | Content | Who writes it |
|---|---|---|
| `session.md` | A YAML frontmatter with `title` and `longhand_tags`. Other keys and text after the frontmatter are the user's. | The user and Longhand |
| `notes.md` | The notes that the user typed. | The user and Longhand |
| `transcript.md` | The transcript. | Longhand, then the user |
| `summary.md` | The summary. | Longhand, then the user |
| `longhand.json` | Longhand's records: the layout number, the note id, the start time, the segments, the fingerprints, the result state, the summary data and, for an imported note, the source file. See [The layouts of `longhand.json`](#the-layouts-of-longhandjson). | Longhand only |
| `recognition.jsonl` | The recognition data that Transcribe Again and Resume use. | Longhand only |
| `audio/` | The audio of each segment, and a `.removed` file for a segment whose audio Longhand removed. | Longhand only |
| `.longhand-in-use-<run>` | An in-use marker: a computer records or processes the note now. | Longhand only |

A note that has never recorded has only `session.md`, `notes.md` and `longhand.json`.

A note can also have conflict copies from a sync service, for example `notes (1).md`. Longhand shows a notice for them. Do not merge or delete them unless the user asks.

## The layouts of `longhand.json`

The `layout` key of `longhand.json` tells which keys the file has. The scripts know two layouts. The two layouts have the same keys, except the key of the note id and the `import` key.

| Layout | Key of the note id | `import` key | Written by |
|---|---|---|---|
| 1 | `session_id` | No | Older Longhand versions |
| 2 | `note_id` | Optional | Current Longhand versions |

An imported note has the `import` key: the audio file that Longhand imported, as `source_name` (the file name) and `sha256` (the SHA-256 of the file). The key is optional. A note that Longhand recorded does not have it. An older Longhand version removes the key when it writes the records of the note, so a note can lose it. To find which files Longhand imported, do not use this key. Read the import record (see [The `.longhand` folder](#the-longhand-folder)).

A storage folder can have notes in the two layouts. Longhand changes a note to layout 2 when it next writes the note's records. `list_notes.py --json` gives the note id of each layout as `note_id`.

A file with a larger layout number, or with a key that its layout does not have, comes from a Longhand version that this skill does not know. A check reports such a note as not checked. Tell the user to update the Longhand plugin.

## The `.longhand` folder

| File | Content |
|---|---|
| `library` | An empty file. It tells Longhand that the folder is a storage folder. |
| `settings.json` | The shared settings: one JSON object. See `settings.md`. |
| `background.md` | The Background: plain text that each summary uses. See `settings.md`. |
| `prompts/*.md` | One file for each custom summary prompt. See `prompts.md`. |
| `imports/<sha256>/<computer id>.json` | The import record: one folder for each audio file that a computer imported, skipped or did not decode, and in it one file for each computer. Only Longhand writes it. |

Each computer that uses the storage folder reads these files. A change in them has an effect on every computer.

The import record is Longhand's own. Never edit, move or delete a file in `imports/`. You can read it. The `state` of an entry is `imported`, `skipped` or `failed`. Each entry also has `source_name`, `size`, `computer_id` and `time`. A `failed` entry also has `longhand_version`, `decoder_name` and `decoder_version`. When the entries of one file have different states, `imported` has priority over `skipped`, and `skipped` has priority over `failed`.
