---
name: longhand-notes
description: Read and change the notes and shared settings in a Longhand storage folder. Use when the current folder, or a folder that the user names, has a `.longhand/library` file, or when the user asks about their Longhand notes, transcripts, summaries, tags, note types, summary prompts or Longhand settings.
---

# Longhand notes

Longhand records a meeting or a conversation and saves each note as plain files in a storage folder. This skill tells you the layout of that folder, the files that you can edit, and the steps for each permitted edit.

A storage folder has the file `.longhand/library`. Start in the root of the storage folder. If the folder has no `.longhand/library` file, it is not a storage folder: stop, and ask the user for the path. One task needs no storage folder: to read the text of a built-in prompt, read `references/built-in-prompts.md`.

## Rules

1. **The `.md` files are the user's. Longhand's records are not.** You can edit `session.md`, `notes.md`, `transcript.md` and `summary.md` in a note folder, and `settings.json`, `background.md` and the prompt files in `.longhand/`. Never edit, move or delete `longhand.json`, `recognition.jsonl`, the files in `audio/`, a `.longhand-in-use-*` file or `.longhand/library`. `references/files.md` gives the full list.
2. **Do not edit a busy note.** A note is busy when a computer records or processes it, when its recording is not saved yet, or when a move to another type waits. `list_notes.py` shows the busy notes. A busy note can change on another computer at any time, and your edit can go.
3. **Make a snapshot before an edit, and compare after it.** The steps are below. Do not tell the user that an edit is complete until the compare passes.
4. **Do not make, copy or delete a note folder.** Longhand makes notes. To remove a note, the user deletes it in Longhand.
5. **Do not write other files into the storage folder.** Keep drafts, scripts and output in a temporary folder or in the conversation. The scripts of this skill write nothing into the storage folder.
6. **Do not look for, start or control the Longhand app.** Everything in this skill works on the files only. Longhand reads each change from the files, and the sync service copies it to the other computers.
7. **Edit only what the user asked for.** Keep all other text, keys, comments and line ends in each file as they are.

## Python

The scripts in the `scripts/` folder next to this file need Python 3.9 or later, with no packages. Before the first edit, find a Python. Try these commands in this order, and use the first one that works:

- `python3 --version`
- `python --version`
- `py -3 --version` (Windows)
- `uv run --no-project python --version`

Python is technically optional. If no command works, tell the user: "Python is not available, so I continue without Longhand's checks. Proceed with caution, and check the result in Longhand." Then do the edit with the steps in the references, with extra care. Do not install software without the user's permission.

In the commands below, `python3` is the command that you found, and `SKILL_DIR` is the folder of this file.

## The edit loop

1. List the notes, and find the notes to change:
   `python3 SKILL_DIR/scripts/list_notes.py` (add `--json`, `--tag`, `--type`, `--since`, `--until`, `--title` or `--not-busy` as necessary).
2. Make a snapshot:
   `python3 SKILL_DIR/scripts/check.py snapshot`
   The script prints the path of the snapshot. The snapshot is in the temporary folder of the computer, not in the storage folder.
3. Make the edit with your own file tools. Follow the reference file for the task.
4. Compare:
   `python3 SKILL_DIR/scripts/check.py compare SNAPSHOT_PATH`
5. Read the result:
   - Exit code 0: the check passed. Tell the user which notes and files changed.
   - Exit code 1: a problem. Correct each problem, or undo the edit, and compare again with the same snapshot.
   - Exit code 2: a part of the change was not checked. This is not a pass. Tell the user what was not checked, and why.

A script that prints "Stopped:" did no work. Read the reason, and correct it.

## Tasks

| Task | Read |
|---|---|
| The layout of the storage folder and of a note folder | `references/layout.md` |
| Which files you can edit, and which files you must not edit | `references/files.md` |
| Find notes and answer questions about them | `references/answering.md` |
| Edit a title, `notes.md`, `transcript.md` or `summary.md` | `references/editing.md` |
| Add, remove or change tags | `references/tags.md` |
| Change the type of a note, or add, rename or remove a note type | `references/types.md` |
| Make, change, rename or delete a summary prompt | `references/prompts.md` |
| Read the text of a built-in prompt | `references/built-in-prompts.md` |
| Change a shared setting or the Background | `references/settings.md` |

## Scripts

| Script | Use |
|---|---|
| `list_notes.py` | List the notes, with filters and the busy state. |
| `check.py snapshot` and `check.py compare SNAPSHOT` | Prove what an edit changed. |
| `new_id.py` | Print a new id for a new prompt file. |
| `folder_name.py NOTE_FOLDER "Title"` | Print the folder name that Longhand gives a note with that title. |
