# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Saving a session

### Working location
The app's own storage, where a session lives while it records and processes: its audio, the processing copies and every intermediate file. The user never browses it.

Some of what it holds never leaves it: the processing audio and the raw recovery chunks stay behind when the session moves to its Destination.

### Destination
The folder the user chose to receive finished sessions, possibly on another volume or in a synced cloud folder that several computers share. Nothing partial ever appears in it.

### Save
The step that moves a finished, verified session from the Working location into the Destination under its final folder name. Its end state is Saved. At every moment, exactly one location holds a complete copy. After a crash at any point, Save continues from what is on disk. It does not start again and it does not lose the session.
*Avoid in all text (code, docs and UI):* "land", "lands", "landed" and "landing". The step is Save and its end state is Saved. The UI says saved, or shows a saved session as Ready.

Save does one step at a time. It records each step durably before it does the next step. Thus each step must recognise, from disk alone, a step that completed just before a crash. The app removes the Working-location copy only after it records the session under its final name.

### Staging
The hidden, session-specific folder inside the Destination. Save assembles the session in this folder before the final rename. The folder is next to the final name, so the last step of every Save is a same-volume rename. This is also true when the Working location is on another volume and Save had to copy the session into Staging.

A staging folder is removed at launch only when its session is recorded as completed: until then it may hold the only complete copy.

### Home folder
The folder that Save put a session into. A session keeps its home folder for life. When a saved session is resumed, recorded into and processed again, the app updates its home folder in place (it adds new audio next to the old audio and replaces the documents). It does not save a second copy.
