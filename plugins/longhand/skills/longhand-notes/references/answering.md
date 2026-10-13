# Find notes and answer questions about them

Reading is always safe. You do not need a snapshot to read.

## Find notes

- Use `list_notes.py`. It gives the folder, the start time, the type, the title, the tags and the state of each note.
- Use the filters to find fewer notes: `--type`, `--tag`, `--since YYYY-MM-DD`, `--until YYYY-MM-DD` and `--title`.
- `--tag "Person::Nathan"` finds the tag Person::Nathan in any case and with any spaces around `::`. `--tag "Person::"` finds each tag with the scope Person.
- Use `--json` when you must read the output with a script.
- To search the text of the notes, search `notes.md`, `transcript.md` and `summary.md` with your own search tools. Do not open the files in `audio/`.

## Read a note

| To find | Read |
|---|---|
| What the user typed during the note | `notes.md` |
| What the people said | `transcript.md` |
| The main points, decisions and next steps | `summary.md` |
| The title and the tags | `session.md` (or the output of `list_notes.py`) |
| The type | The folder that holds the note folder (General when there is none) |

## The transcript

`transcript.md` starts with `# Transcript`. Each turn is one line:

```text
- [00:00:09] Speaker 2: The travel line is lower than last quarter.
```

- The time is the position in the recording (hours, minutes, seconds).
- "Speaker 1", "Speaker 2" and so on are labels that speaker recognition gave. They do not identify a person. The same label in two notes is not the same person. Do not guess a name from a label. Use a name only when the transcript, the notes or the Background make clear who the person is.
- A line such as `--- silence 00:00:12 ---`, `--- pause 00:05:00 ---` or `--- gap 00:01:30 ---` marks time with no speech, a pause by the user, or time between two recordings.
- A line in another form is text that a person or an agent added.

## Answer

- Give the date and the title of each note that you use, so the user can find it in Longhand.
- Say when the transcript is not clear. Recognition can write words that are wrong, most often names and special terms.
- A note that `list_notes.py` shows as "not readable" can have text that you can read, but its title or tags can be out of date.
