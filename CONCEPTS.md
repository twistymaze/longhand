# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Notes

### Note
One note and everything in it: the manual notes and, when it records, the audio, the transcript and the summary. A note can have no recording: see Note with no audio. Code, docs and the UI all use "note" ("New Note", the Notes list). Older code uses "session" for a note (for example `SessionState` and the `session-core` crate) and "notes" for the manual notes; new code uses "note" and "manual notes".

### Note id
The identity of a note: 1 to 64 lowercase ASCII letters, digits, `-` and `_`, and not a Windows device name. Longhand makes it as a random UUID and uses it as a folder name, so it is not a secret. The note record file holds it as `note_id` (layout 2). Layout 1 of that file and the old layout of `session.md` hold it as `session_id`. Write `note_id`, not `session_id`, in new code: the CodeQL rule `rust/cleartext-logging` treats a name like `session_id` as sensitive data.

### Manual notes
The text that the user types into a note, in `notes.md`. The UI shows it in the My Notes tab. Older code calls it "notes".

### Note with no audio
A note that has never recorded, or whose only recording was discarded because every segment was shorter than one second. It has manual notes and a title, and no transcript and no summary. The user opens one with New Note when the Recording setting says that New Note does not record, or with the other choice of the New Note menu. Its Record button records into it; its first recording makes the transcript and summary through the usual processing. An empty note with no audio (no manual notes and no title from the user) is removed when the user leaves it, but only by the computer that started it, and only when no other computer can have used it.

### Result flag
The `result` field of the note record file for a note that has no result yet. `none` means the note has never recorded. `pending` means a recording exists that is not saved yet, on the computer that records it. The first result removes the field. Every computer reads the flag and does not infer the state from missing files, so a note with the flag is not waiting for a sync. Records with no field, no segments and no result fingerprints read as `none`.

### Note record file
The file `longhand.json` in each home folder. It holds Longhand's records of the note: the note id, the start time, the segments, the result flag, the recognition data, the fingerprints of `transcript.md` and `summary.md`, the summary data, the pending type, the title that Longhand last wrote and, for an imported note, the source file (`import`: its name and its SHA-256). The `import` field is optional, and an older Longhand removes it when it writes the file. It is a JSON object with a `layout` number. Longhand writes it with sorted keys, LF line ends and no write time, so two computers that write the same records write the same bytes. It is the last file of each write into a home folder. The `.md` files of a home folder belong to the user, and a person or an agent can edit them. Only Longhand writes the note record file. A `layout` that is larger than this build knows is from a newer Longhand: this build does not use or write the records of that note. `session.md` holds only the title and the tags, and Longhand changes only the lines of the keys that it changes. A folder whose note id has another form (see Note id) is a folder with a problem, not a note, because Longhand uses the id as a folder name.

### Old layout
The layout of a home folder that an older Longhand saved: `session.md` holds `session_id` and all the records, and `summary.md` holds the summary data in its frontmatter. A `session.md` that holds `session_id` is the old layout, also when the folder has a `longhand.json`. Longhand reads an old note, and the user can change its title, tags and manual notes. Transcribe Again, Regenerate Summary, Resume and a type change wait until the conversion. The conversion moves the records into the note record file. Only the user starts it, with Convert in the notice on Home. The notice shows only while the storage folder has notes in the old layout. The conversion skips a note that a computer records or processes, and the user can select Convert again later.

### Not finished on another computer
The state of a note whose result flag is `pending` when no computer's in-use marker in its home folder is fresh and the note record file is older than 30 minutes: the computer that recorded it stopped before it saved the recording. Other computers let the user edit its manual notes and delete it after a confirmation. Nothing removes it automatically. When the recording computer later recovers the note and finds its home folder deleted, the delete wins and the recording is discarded.

## Saving a note

### Working location
The app's own storage, where a note lives while it records and processes: its audio, the processing copies and every intermediate file. The user never browses it. A computer that has not recorded can have no Working location yet, also when it shows and changes notes from a shared storage folder.

Some of what it holds never leaves it: the processing audio and the raw recovery chunks stay behind when the note moves to the storage folder.

### Unregistered audio
Recovery chunks of a note that no segment row includes: a closed chunk whose segment index has no row, or any `.part` file that recovery could not repair. While a note has unregistered audio, Longhand keeps its folder in the Working location, keeps the note Interrupted, and refuses to save or resume it. Longhand first tries to add the chunks as segments, at each Stop, Start, Resume, processing claim and launch. A `.part` no longer than a WAV header holds no audio and is removed.

### Storage folder
The one folder that holds all saved notes of the library and the shared settings. It can be on another volume or in a synced cloud folder that several computers share. A new install uses a default folder in the local app data folder, so a new install records with no setup. A new note appears in it at once with its notes, `session.md` and `longhand.json`; audio and results appear in it only when they are complete. New Note is unavailable while the storage folder is unavailable. The user changes it in Settings > Storage. The UI calls it "Storage". "Destination" is the older term for the same folder, in older code and docs. The storage folder has a hidden `.longhand` folder for the library marker and the shared settings.

### Save
The step that puts the first result of a finished, verified note into the storage folder. A note that started after the early home folder existed already has its home folder, so Save writes the result into it with the Publication rules. A note from before that change has no home folder, so Save moves it from the Working location into the storage folder under its final folder name. Its end state is Saved. At every moment, exactly one location holds a complete copy. After a crash at any point, Save continues from what is on disk. It does not start again and it does not lose the session.
*Avoid in all text (code, docs and UI):* "land", "lands", "landed" and "landing". The step is Save and its end state is Saved. The UI says saved, or shows a saved note as Ready.

Save does one step at a time. It records each step durably before it does the next step. Thus each step must recognise, from disk alone, a step that completed just before a crash. The app removes the Working-location copy only after it records the note under its final name.

### Staging
The hidden, note-specific folder inside the storage folder. The early write assembles a new note's first files in this folder before it renames the folder to the note's home folder name. Save of a note from before the early home folder assembles the note in this folder before the final rename. The folder is next to the final name, so the last step of every Save is a same-volume rename. This is also true when the Working location is on another volume and Save had to copy the note into Staging.

A staging folder is removed at launch only when its note is recorded as completed: until then it may hold the only complete copy.

### Home folder
The folder of a note in the storage folder. The early write makes it when the note starts, recording or not, and the recording never waits for that write. A note from before that change got its home folder from Save. A note keeps its home folder for life. A rename of the note changes the folder's name, not the folder: the note still has one home folder. When a saved note is resumed, recorded into and processed again, the app updates its home folder in place (it adds new audio next to the old audio and replaces the documents). It does not save a second copy.

### Publication
The step that writes a new result of a saved note into its home folder. The result is complete in local staging first. Then the publication writes the result files and writes the note record file last. A small record on this computer, which never syncs, lets a publication that a crash stopped roll forward at the next launch. Only the computer that started a publication finishes it.

Before each write, the publication checks whether another computer published a result for the note. If it did, this computer drops its unfinished result and writes nothing more: the last change wins. A publication changes only the result fields of the note record file, and never the title or the tags in `session.md`, so a title change or a rename is not a conflict.

### Title
The name of a note, shown on Home, on the note page and in the home folder's name. Processing proposes it. The user can rename it at any time, and a title the user set is never replaced automatically. A note with no title yet shows its start time: the time the note opened, also when it records later.

## The library

### Library
Every saved note in the storage folder. After a change of storage folder, the library forgets the notes of the old folder and leaves their folders on disk. Several computers of one person can share a storage folder through a sync service (OneDrive, Google Drive, Dropbox, iCloud). Each computer lists the whole library, including the notes that other computers recorded. The home folders are the authority: each computer keeps its store in step with them through a library scan at launch and a folder watcher while Longhand runs.

A computer that only receives a sync changes no home folder. An automatic step, such as a launch reconcile, writes into a home folder only for work that this computer started and recorded. Audio removal is the one exception. A partial sync can make the files of one home folder disagree for a time, so the shared files cannot tell which computer made a change.

### Waiting for a sync
The state of a saved note whose `transcript.md` or `summary.md` does not match the fingerprint that the note record file records for it. Longhand writes a new result as `transcript.md` and `summary.md` first and `longhand.json` last, so a computer that receives part of a sync sees this state for a few seconds. The note shows its files as they are, and Transcribe Again, Regenerate Summary and Resume wait. A difference that lasts about one minute is a hand edit: the note stops waiting, and the actions run again and use or replace the edited file. A file that cannot be read keeps the note waiting.

A folder with a `session.md` in the new layout and no `longhand.json` also waits for a sync: the sync delivered `session.md` first. Longhand does not adopt the folder and writes nothing into it. The folder can be a home folder that a rename on another computer moved, so while it waits, this computer forgets no note of that storage folder. When `longhand.json` has not arrived after about one minute, the notice on Home names the note and `longhand.json`, and tells the user to update Longhand on the other computer if the file does not sync.

The same rule applies to a title. A title change writes `session.md` and `longhand.json`, and a sync can deliver them in either order. A `session.md` title that differs from the title that Longhand last wrote is the user's title only when the difference lasts about one minute. Until then, Longhand keeps the title source that it has and writes nothing for that note.

### Adopt
The step that makes a Saved row in this computer's store from a home folder that another computer saved. The row is built from the note record file, `session.md` and `notes.md`. Its opposite, forget, removes a row from the store and does not touch any folder: a computer forgets a note when its home folder is deleted from an available storage folder.

### Database rebuild
The step at launch that replaces this computer's store when another version of Longhand owns it: a newer version, or a build whose migrations differ. Longhand moves the store and the Working location aside together as one database backup, makes a new store, and the library scan then adopts every saved note again.

The home folders hold every saved note, so a rebuild loses no saved note. Unfinished work exists only in the old store and the Working location: a note that is not saved yet, a running transcription or summary, an open Move, a rename or a publication. When the old store or the Working location holds unfinished work, or Longhand cannot read the store, Longhand asks the user to choose Quit or Rebuild first. Quit changes no file. Longhand keeps the newest automatic backups and every backup that the user chose in that alert.

### Move
The step that copies every saved note from the old storage folder to a new one, when the user changes the storage folder and chooses Move. Move works on one note at a time and records each step. A crash, a full disk or an unavailable folder pauses it, and it continues from what is on disk after a launch or a Retry. A note leaves the Move without a copy only when its source is gone from an old folder that is still available; when the whole old folder goes away, even in the middle of a Move, the Move pauses. At every step, one complete copy of each note exists. The other choice, Leave them, changes the storage folder and leaves the old notes where they are; the library then forgets them. The user cannot change the storage folder while a Move is open.

### Audio removal
The step that permanently deletes all audio of a saved note and keeps its transcript, summary, notes and title. A shared setting, on by default at 30 days, removes the audio that many days after the note last recorded; the Remove Audio action on one note removes it at once. Any computer that uses the storage folder can remove old audio, but not from a note that it can see another computer records into or processes. A computer that records into or processes a note shows this with an in-use marker, a file named `.longhand-in-use-<run ID>` in the note's home folder. The computer refreshes the marker while it works and deletes it after. A marker that is older than 30 minutes has no effect. A quit or a crash can leave a fresh marker of an earlier launch; the computer that wrote it counts it as its own work, not as work on another computer. Before a computer deletes a segment, it writes a removal marker (`segment-01.opus.removed`) next to it, so every computer knows that the audio of the segment is gone, also before the delete syncs. While the shared settings cannot be read, no audio is removed automatically. When the last audio of a note is removed, `recognition.jsonl` goes too, because it has no use without the audio. The same pass also deletes the result copies that older builds kept in `.longhand-results/`. A note with no audio cannot use Transcribe Again or Resume.

### Note type
The one kind that a note can have, for example "Pathfinder" or "One on One". A note has one note type. The note types are a list in the shared settings. A note type is also a type folder: a folder in the storage folder with the type's name, which holds the home folders of the notes of that type. General is the built-in note type: a note of General has its home folder directly in the storage folder, and has no type folder. General is always in the type list, first, and the user cannot rename or delete it. No type folder can have the name "General". In code, the type `None` is General. At rest, the position of the home folder decides the type. A type change for a note that a computer records or processes waits: the note record file holds the pending type, and the computer where the user made the change owes the move of the folder (an owed type move). The effective type list is the shared list plus every type folder that holds a note.

### Tag
A free text label on a note. A note has zero or more tags. When the text contains `::`, the part before it is the scope and the part after it is the value, for example `Person::Nathan`. The scope changes only how the tag looks: one note can have several tags with the same scope. The tag list is every tag that is on at least one note.

### Conflict copy
A second copy of a note's file that a sync service makes when two computers change the file before either change syncs, for example `notes (1).md`. Longhand does not merge conflict copies. It shows a notice that names them. When a notes save finds that `notes.md` changed since the notes were loaded, Longhand keeps `notes.md` and saves the edit as its own conflict copy, `notes (Longhand conflict YYYY-MM-DD HHMM).md`.

## Recording

### Microphone choice
The input that a note records from. The user chooses the usual microphone in Settings, or "System default", and can change it for one note on the recording page. The choice is saved by the device ID, with the name as a label. A chosen microphone that is missing or disconnects is replaced by the system default, and the choice stays. When the chosen microphone is back for two polls in a row, the note moves back to it. Each change of input is a segment boundary.

### App audio
The audio of the apps in a note's app set, recorded with the microphone. The app list in Settings is empty on a new install, and the user adds only running apps. Each new note copies the app list into its own app set; the note menu changes the app set of that note only. An app is recorded while it plays and is ignored at other times. A note saves only the microphone until the first app sound above a low level; from then on it saves the microphone on the left channel and the app audio on the right. The microphone is the clock: the app audio is padded or trimmed to match it. Processing and the live transcript use a plain mix of the two channels. On macOS the capture is a process tap of the app set's processes and needs the Screen & System Audio Recording permission. On Windows 11 it is one process loopback client for each app. The client also records the processes that the app started, also an app that is not in the app set; Windows gives no way to exclude them. While Teams plays, it records the whole output device of Teams, because Windows gives silence for Teams. Code calls the capture "system audio" in some names.
*Avoid:* computer audio (the removed whole-computer feature)

### App set
The apps whose audio one note records: a copy of the Settings app list at the note's start, changed only from that note's menu. Continue and Resume use it.

### Pause
The state of a recording note that captures nothing for a time, by the user's choice. The microphone and the app audio are closed, and no audio, not even recovery data, goes to disk. Continue records into the same note; the UI does not call it Resume, because Resume records into a stopped note. A paused note is still open: processing starts only at Stop. The transcript and the summary input mark each pause with its duration. A pause is not a gap: gap fill does not transcribe it.
*Avoid:* "pause" for a silence in speech (say silence)

### Restart budget
The few attempts that Longhand makes to reopen a lost microphone during a note, with a longer wait before each attempt, before it interrupts the note and shows the alert. Only failed reopens in a row count, however long each attempt takes. The budget is full again only after capture has recorded for a set time since it last came back, so a microphone that comes back and drops again soon after gets no new attempts. A move to the system default when the chosen microphone goes away also counts as a comeback. A wake from sleep, a returned microphone and a resume by the user start a new budget.

### Recording widget
A small window above all other windows that shows the open note's state, time and Pause, Continue and Stop while a note is recording, paused or interrupted, so the user does not forget a note that still records or is paused. It never takes the focus from the app in front and is hidden from screen capture where the system allows. A Settings switch sets whether each new note shows it; the user can hide or show it for the open note only, and that choice ends when the note stops.
*Avoid:* pill, overlay (the code calls the shape a pill; say recording widget)

### Import
The step that makes a new note from an audio file that another device recorded, for example a phone. The user drops or chooses the file (a manual import), or the watched folder brings it in. A drop from an app that gives only a promise of a file, for example Voice Memos, is also a manual import: Longhand first asks that app to write the file into a local folder. An imported note is a normal note: it processes on the computer that imported it, and its audio becomes the normal audio of a note. A manual import always makes a new note, also for a file that was imported before.

### Watched folder
A folder that the user chooses on one computer, where each new audio file becomes a note through an import with no user action. It is a setting of that computer only, not a shared setting. Longhand never changes, moves or deletes a file in it. It cannot be the storage folder, be inside it or contain it.

### Import record
The plain files, in the hidden `.longhand/imports/` folder of the storage folder, that record each file that Longhand imported (with a manual import or from a watched folder), skipped, or could not decode. A file's identity is its content: the SHA-256 of the file. Each identity has one folder, `.longhand/imports/<sha256>/`, and in it each computer writes only its own entry, `<computer_id>.json`, so two computers never write the same file. An entry has the state `imported`, `skipped` or `failed`. A `failed` entry also holds the Longhand version and the decoder name and version that refused the file; only a newer Longhand version tries that file again. When the entries of one identity have different states, `imported` has priority over `skipped`, and `skipped` has priority over `failed`. The import record decides whether a file was imported: the `import` field of the note record file is optional, and an older Longhand removes it. The watched folder never imports a file that the import record holds, also after its note is deleted. A manual import also writes an entry. Only Longhand writes the import record; an agent must never edit it. A Move takes it to the new storage folder with the notes.

### Importing
The state of a note while the import copies and converts its audio file. Home shows it as "Preparing". Only the import and a user delete change the note. When the conversion is complete, the note goes into the processing queue. When the conversion fails, or the file is a duplicate, Longhand removes the note. An older Longhand cannot read this state, so the import raises the database schema version.

## Transcription

### Recognition model
The model that turns speech into text. The user selects a transcription model and compatible processing choices. A note records its configuration in use and exact model artifacts when recording starts. A later change to the defaults does not change that note. Longhand selects the runtime internally.

### Speaker recognition
The user-facing term for diarization: assigning anonymous speaker labels to speech. It does not identify named people, enroll voices, or retain voiceprints. A separate speaker model supplies a timeline, and Longhand joins it with the text. Transcription models do not supply the labels. Speaker recognition can also be off. The selected artifacts determine valid processing times and speaker limits.

After Stop, the Finding speakers step shows one percent for the whole note. During one attempt, the percent does not go back, and it stays below 100 until the step is complete. Work that cannot be measured shows activity, not a percent. Saved work of an earlier attempt counts at the start of a new attempt, but the percent of the earlier attempt does not show.

### Speaker analysis window
A span of audio that the speaker model analyzes on its own, with its own speaker labels. A window ends at a fixed maximum length or at the end of its segment, and the next window starts there. The labels restart in each window, and no rule links the labels of two windows, so two labels can be the same person. The live transcript shows each restart as a "Speaker labels restart" divider. Code calls a window an epoch.
*Avoid:* epoch (in docs and UI)

### Live transcript
The text that a streaming recognition model makes during recording. It shows in the Transcript tab when the selected model supports live transcription and the note uses that mode. Live text and speaker labels can be provisional. The runtime can revise them before finalization. Compatible durable results are reused after Stop; missing or invalid results require processing. Live speaker analysis is a separate choice and can run with an offline transcription model.

### Part
A span of a live transcript that the app saves as one checkpoint. A part ends at the first silence of at least 600 ms after 4 minutes, and always at 8 minutes. The live stream then starts again for the next part.

### Gap
Audio of a note that has no live transcript: a span that the live transcript missed because it fell behind, failed or stopped, or a part that did not save before a crash. After Stop, recognition transcribes only the gaps in batch ("gap fill").

### Transcribe Again
The menu action that makes the transcript and summary of a saved note again. It asks for one confirmation and offers no choices. The run uses the models, speaker source, and language in Settings, always with After Stop timing, and the speaker count of the note, including "auto". When the saved timed text is compatible, the run uses it again. The recording, notes, and title stay. A note with no title gets a proposed title. During the run, the note shows progress in place of the transcript and summary. A failed run keeps the previous result.

### Retry
The action that puts a note back in line after its processing failed or its save found no storage folder. The new attempt does not start the processing again from the beginning. It skips each step whose saved work is still valid, so it continues at the step that failed, and it uses the saved work of that step again where it can. The note shows only the progress of the new attempt. When a Retry fails again at the same transcription or speaker step, the next Retry asks to start over: after confirmation, it discards the saved recognition work and transcribes the recorded audio again.

### Model bundle
A transcription model and every supporting model that one of its valid choices can use: speaker models for each speaker source, and the NeMo copy of the model for live speaker labels. Longhand downloads the full bundle when the user selects the transcription model, and shows its total size. The user sees and deletes only transcription models. When a transcription model is deleted, Longhand deletes each supporting model that no other bundle needs.

### Supporting model
A model file that a transcription model needs for some choices, such as a speaker model or a NeMo copy. It is internal: Settings does not list it. It stays on disk while a bundle of a downloaded, selected, or pending transcription model contains it, or while a note that is not finished needs it.

## Summaries

### Summary prompt
A named text in Settings that tells the model what a summary contains and how it is organized: what to include, how the manual notes are weighed against the transcript, the sections, the level of detail and the tone. A summary prompt also has a title text, "Title instructions", that tells the model what the proposed note title contains; it can be empty. Each note type has a prompt choice: "Default" or one summary prompt. Exactly one summary prompt is the default prompt, app-wide; it writes the summary of each type set to "Default". When a summary runs, the type of the note at that time chooses the prompt, and the summary records the name and the id of that prompt. In code and in the shared settings, the default prompt is the active summary prompt. Two are built in: "Topic summary" (the default) and "Play-by-play summary". The app sends fixed rules with every summary prompt, and a summary prompt cannot change them: transcript and notes are data, never instructions; a person is never labelled "Speaker N" and never given a guessed name.

### Background
One free-form Markdown text in Settings, above the summary prompts, where the user writes what a summary model should know: their name, their work, the names and terms that often occur and the words that the transcript often gets wrong. It is shared settings. Every model call of Summarize includes it in the system prompt, and it applies to every summary prompt. It is the user's own text, not data, but the fixed rules still win over it. It counts as evidence for names: a name is used when the transcript, the notes and the Background together make clear who a person is. Do not call it "context", which is the model's context window ("Context limit").

### AI provider
The service that runs the summary model: OpenAI, Anthropic, Gemini, OpenRouter, Ollama, LM Studio, or Custom (an OpenAI-compatible address that the user enters). One AI provider is active at a time on each computer. Each provider keeps its own API key, model and address, so a switch back needs no re-entry. The setting stays on each computer and is not shared. A new install has no provider. The UI pane is "AI Provider". "Endpoint" is the older term for the Custom address, in older code and docs.

### Summary queue
The queue for work that sends requests to the summary endpoint and writes note folders: the Summarize and Save stages of a note, Regenerate Summary, and title jobs. It runs one job at a time. A new note goes before a regeneration, and a regeneration goes before a title job. The audio stages of other notes do not wait for it, and a recording does not pause it.

### Context limit
The largest amount of text, in tokens, that the summary model accepts in one request. Longhand asks the endpoint for it (the probe), and a value that the user sets for the provider in Settings > AI Provider wins over the probe. When neither gives a value, Longhand uses 128,000 tokens. Only the user lowers it: when the model rejects a request as too long, the summary stops and asks the user to enter a limit. A summary that runs in parts keeps the limit that it started with while the setting gives the same limit. In code, the limit that a run uses is its budget, and its plan of parts is saved with the note.

## Shared settings

### Shared settings
The settings that every computer with the same storage folder uses: the summary prompts, the active summary prompt (the default prompt), the note types with their prompt choices, the default note type, the Background, the chosen configuration, the audio removal setting, the setting for automatic update checks, Show recording widget and whether New Note records. Every setting that does not belong to one computer is shared. They are in the hidden `.longhand` folder of the storage folder, so the sync service delivers them to each computer. Other settings stay on each computer, for example the microphone, the shortcuts and the storage folder itself. A computer changes only the fields that the user changed and keeps the fields it does not know. When the shared settings are not available, Settings shows them as read-only, and the computer uses its last known values.

### Chosen configuration
The recognition choices that the user made in Settings and that every computer shares: the transcription model, live transcript, speaker source, speaker processing mode, language and default speaker count. It is what the user wants, not always what a computer can use now. Live transcript, speaker processing mode and language are preferences: each one applies only when the model and the speaker source support it, and otherwise a note uses After Stop or automatic language. A change of the model does not change them. Compare Configuration in use.

### Configuration in use
The recognition choices that this computer uses for a new note. It equals the chosen configuration when this computer has every model that the choice needs. Until then, it stays on the last configuration whose models are on disk, and Longhand downloads the missing models. A new note that starts in this time uses the configuration in use, or the chosen configuration with live transcript off when its models are on disk. When the download ends, the configuration in use follows the chosen configuration. It is local to each computer.

## Agents

### Agent skill
The `longhand-notes` skill, which tells an AI agent how to read and change the notes and the shared settings in a storage folder. It is a plugin in `plugins/longhand/` of the Longhand repository, and the repository is its plugin marketplace. The plugin updates from `main`, not from a release. Its scripts use Python and its standard library only, write nothing into the storage folder, and never look for the app.

### Managed block
The agent instructions that Longhand writes between two markers into `AGENTS.md` and `CLAUDE.md` in the root of the storage folder. Longhand writes the first block only when the user asks for it in Settings > Storage. The block has a version. When the block in a file is older than the block of this build, Longhand replaces it at the next write into the storage folder that a user change on this computer causes, never at launch or at a read. Longhand never replaces a block that a newer Longhand wrote. Longhand changes only the text between the markers, and keeps the other text of the file, its line ends and its byte order mark. When the markers are damaged (a begin marker with no end marker, or two blocks), Longhand writes nothing and shows a notice that names the file. When `CLAUDE.md` is a link to `AGENTS.md`, or holds only `@AGENTS.md`, Longhand writes the block one time.

## Shortcuts

### Shortcut
A global key combination that does one Longhand action from any app. The recording shortcut starts and stops a recording; the show shortcut shows the main window, or hides it when it is in front; the Pause shortcut pauses and continues the open note; and the widget shortcut shows or hides the recording widget for the open note. The user can clear each one; a cleared recording shortcut is a choice, not a fault, and New Note still starts a recording.

### Chord
The keys of a shortcut: one or more modifier keys and one other key. A shortcut is the action; its chord is the keys the user sets for it. No two shortcuts can have the same chord, and a chord that another app holds is refused.

### Key capture
The state in which a Settings row waits for the user to press a new chord. It is distinct from audio capture, which records the microphone. During key capture all shortcuts are paused, so their chords start nothing, and only one row waits at a time. Escape, Tab or a click elsewhere ends key capture with no change.

## About

### Full licence list
The full licence text of every open source component that the app ships on one platform, direct and indirect, shown in its own window from Settings > About. Each platform has its own list, stored in the repository. A check makes the list again from the dependencies and fails when the result differs from the stored list. The UI calls it "All open source licences".

### Native entry
A hand-kept record in the full licence list for a shipped component that no licence tool can read, such as a runtime library or a model that one of Longhand's own crates compiles in. Each native entry names its licence files and its source. Nothing fails when a native entry is missing, so a change that ships a new file of this kind must add one.

## Updates

### Release
A numbered GitHub Release in `twistymaze/longhand`, which CI publishes from a push to main whose commits change the version. Its version is `<major>.<minor>.<patch>`, and git-cliff and the release script compute it from the PR titles since the latest release: a patch by default, a minor when the user chooses one with `+`, and a major when the user asks for one with `!`. It holds the signed, notarized macOS DMG, the updater archive and its signature, the update manifest, the release notes and, after a later workflow run, the signed Windows installer, its updater signature and the Windows update manifest `latest-windows.json`. Only a release build checks for updates: the macOS or Windows app of a release, or a local test build made with the release flag set at build time. Other local builds get no updates.

### Changelog
The list of the changes in each release since 1.0.0, grouped by calendar month (UTC) and not by version. Its lines and headings are the same as in the release notes, because one script writes both from git-cliff's result. Each build contains the changelog: What's new in Settings > About shows the recent months, and the update window shows the changes that an offered release adds. The full changelog is a file of each release and a page at https://longhand.md/changelog/. Nobody edits it, and it is not a file in the repository.

### Update manifest
The file that identifies the newest release: its version, date, archive address and signature. The app reads only this file when it checks for updates. The macOS app reads `latest.json` of the latest release in `twistymaze/longhand`, and the Windows app reads `latest-windows.json` of that release.

## Flagged ambiguities

- "Gap" has two senses. A Gap (above) is audio with no live transcript, which gap fill transcribes. The gap line in a finished transcript is the time between two segments, for example after a Stop and a Resume: no audio exists there, and gap fill does not transcribe it. The time between two segments that a Pause and a Continue made is a pause line, never a gap line.
