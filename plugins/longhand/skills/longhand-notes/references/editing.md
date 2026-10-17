# Edit a title, the notes, the transcript or the summary

Do each edit in the edit loop of `SKILL.md`: a snapshot, the edit, then a compare. Do not edit a busy note.

## The frontmatter of `session.md`

`session.md` starts with a YAML frontmatter between two `---` lines:

```yaml
---
title: Budget review
longhand_tags:
- Finance
- Team::Planning
---
```

- Change only the lines of `title` and `longhand_tags`. Keep other keys, comments, the key order, the line ends and the text after the frontmatter.
- Write each value on one line. Put a value in quotes when it has `: `, ` #`, or starts with a character such as `[`, `{`, `'`, `"`, `&`, `*`, `!`, `|`, `>`, `%`, `@` or `` ` ``. Also put a value in quotes when YAML can read it as a number, `true`, `false` or `null`.
- Use single quotes (`'It''s here'`) or double quotes (`"A \"quoted\" title"`).
- Do not use block values (`|` or `>`), anchors or nested keys. `check.py` cannot read them and reports the file as not checked.
- Save the file as UTF-8 with no byte order mark. Longhand cannot read a `session.md` that starts with one.
- A frontmatter that does not parse does not hide the note, but after about one minute Longhand shows a notice for it. Correct it at once.

## Change the title

A title change also renames the note folder, as a title change in Longhand does.

1. Make sure that the note is not busy, and make a snapshot.
2. Change `title` in `session.md`.
3. Get the new folder name:
   `python3 SKILL_DIR/scripts/folder_name.py "<note folder>" "<new title>"`
4. If the name is different from the current folder name, rename the note folder to that name. Keep it in the same parent folder.
5. Compare. `check.py` reads the renamed folder as a move of the same note.

Longhand keeps a title that the user or an agent set. Processing does not replace it.

## Edit `notes.md`

`notes.md` is the text that the user typed during the note. You can change any part of it. Keep the user's words unless the user asks for a change.

## Edit `transcript.md` or `summary.md`

- You can correct words, names and speaker labels, or change the text in other ways.
- Keep the line form of the transcript (`- [00:00:09] Speaker 2: text`). Longhand shows a line in another form as plain text with no time.
- For about one minute after the edit, Longhand shows the note as waiting for a sync. Then it accepts the edit.
- Transcribe Again replaces `transcript.md` and `summary.md`, and Regenerate Summary replaces `summary.md`. Tell the user that these actions remove the edit.
- Do not add a frontmatter to `summary.md`. Longhand does not show it or use it.

## Do not

- Do not edit `longhand.json`, `recognition.jsonl` or the files in `audio/`.
- Do not add new files to a note folder, also not a backup copy.
