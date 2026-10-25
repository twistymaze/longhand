# Files that you can edit, and files that you must not edit

`check.py` uses the same lists. A test of the skill checks that this file names each pattern.

## Never edit these files

Do not edit, move, copy, rename or delete these files. Do not make new files with these names.

| Pattern | Where | Why |
|---|---|---|
| `longhand.json` | In each note folder | Longhand's records of the note. A change can stop Transcribe Again, Regenerate Summary and Resume, or hide the note. |
| `recognition.jsonl` | In each note folder | The recognition data. It must agree with the audio and the records. |
| `audio/*` | In each note folder | The audio and the removal files. A sync service can keep audio online only. Do not open it. |
| `.longhand-in-use-*` | In each note folder | In-use markers. They tell each computer that a computer works on the note. |
| `.longhand/library` | In the storage folder | The library marker. Without it, Longhand sees the storage folder as not available. |
| `.longhand/imports/*` | In the storage folder | The import record: which audio files Longhand imported, skipped or did not decode. Longhand uses it to import each file one time. Longhand writes it at any time, so `check.py` does not compare it, and an edit there is not found. |

Also do not touch the hidden folders that Longhand makes for a short time, such as `.longhand-staging-*` and `.longhand-rename-*`.

## Files that you can edit

| Pattern | Where | Permitted edits | Steps |
|---|---|---|---|
| `session.md` | In each note folder | The title and the tags. Other keys and text are the user's. | `editing.md`, `tags.md` |
| `notes.md` | In each note folder | Any edit of the text. | `editing.md` |
| `transcript.md` | In each note folder | Any edit of the text. | `editing.md` |
| `summary.md` | In each note folder | Any edit of the text. | `editing.md` |
| `.longhand/settings.json` | In the storage folder | The values of the shared settings. | `settings.md` |
| `.longhand/background.md` | In the storage folder | Any edit of the text, up to 4,000 characters. | `settings.md` |
| `.longhand/prompts/*.md` | In the storage folder | New prompts, text edits, renames and deletes. | `prompts.md` |

You can also move a note folder to another type folder (`types.md`) and rename a note folder after a title change (`editing.md`).

## Before each edit

- Do not edit a busy note (see `SKILL.md`).
- Keep the line ends of the file (LF or CRLF) and its encoding (UTF-8).
- Keep each key and each line that the user did not ask you to change.
