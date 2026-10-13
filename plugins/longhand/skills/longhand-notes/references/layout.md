# The layout of a storage folder

```text
<storage folder>/
  .longhand/
    library                      the library marker
    settings.json                the shared settings
    background.md                the Background
    prompts/
      Weekly review.md           one file for each summary prompt
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
| `longhand.json` | Longhand's records: the note id (`session_id`), the start time, the segments, the fingerprints, the result state and the summary data. | Longhand only |
| `recognition.jsonl` | The recognition data that Transcribe Again and Resume use. | Longhand only |
| `audio/` | The audio of each segment, and a `.removed` file for a segment whose audio Longhand removed. | Longhand only |
| `.longhand-in-use-<run>` | An in-use marker: a computer records or processes the note now. | Longhand only |

A note that has never recorded has only `session.md`, `notes.md` and `longhand.json`.

A note can also have conflict copies from a sync service, for example `notes (1).md`. Longhand shows a notice for them. Do not merge or delete them unless the user asks.

## The `.longhand` folder

| File | Content |
|---|---|
| `library` | An empty file. It tells Longhand that the folder is a storage folder. |
| `settings.json` | The shared settings: one JSON object. See `settings.md`. |
| `background.md` | The Background: plain text that each summary uses. See `settings.md`. |
| `prompts/*.md` | One file for each custom summary prompt. See `prompts.md`. |

Each computer that uses the storage folder reads these files. A change in them has an effect on every computer.
