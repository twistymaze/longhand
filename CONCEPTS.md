# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Saving a session

### Working location
The app's own storage, where a session lives while it records and processes: its audio, the processing copies and every intermediate file. The user never browses it.

Some of what it holds never leaves it: the processing audio and the raw recovery chunks stay behind when the session moves to its Destination.

### Destination
The folder the user chose to receive finished sessions, possibly on another volume or in a synced cloud folder that several computers share. Nothing partial ever appears in it.

### Landing
Moving a finished, verified session from the Working location into the Destination under its final folder name. At every moment exactly one location holds a complete copy, and a crash at any point resumes from what is on disk rather than starting over or losing the session.
*Avoid in user-facing text:* "landed" — the app says saved, or shows a landed session as Ready.

Landing advances one step at a time and records each step durably before taking the next, so every step must recognise, from disk alone, a step that completed just before a crash. The Working-location copy is removed only after the session is recorded under its final name.

### Staging
The hidden, session-specific folder inside the Destination where a landing session is assembled before its final rename. Because it sits beside the final name, the last step of every Landing is a same-volume rename, even when the Working location is on another volume and the session had to be copied into Staging.

A staging folder is removed at launch only when its session is recorded as completed: until then it may hold the only complete copy.

### Home folder
The folder a session was landed into. A session keeps its home folder for life: when a saved session is resumed, recorded into and processed again, its home folder is updated in place (new audio added beside the old, the documents replaced) instead of Landing a second copy.
