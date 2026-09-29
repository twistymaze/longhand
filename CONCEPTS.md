# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Sessions

### Session
One recording and everything made from it: the audio, the user's notes, the transcript and the summary. The UI calls a session a "note" ("New Note", the Notes list); "session" is the term in code and docs only.

## Saving a session

### Working location
The app's own storage, where a session lives while it records and processes: its audio, the processing copies and every intermediate file. The user never browses it.

Some of what it holds never leaves it: the processing audio and the raw recovery chunks stay behind when the session moves to the storage folder.

### Storage folder
The one folder that holds all saved sessions of the library and the shared settings. It can be on another volume or in a synced cloud folder that several computers share. A new install uses a default folder in the local app data folder, so a new install records with no setup. Nothing partial ever appears in it. The user changes it in Settings > Storage. The UI calls it "Storage". "Destination" is the older term for the same folder, in older code and docs. The storage folder has a hidden `.longhand` folder for the library marker and the shared settings.

### Save
The step that moves a finished, verified session from the Working location into the storage folder under its final folder name. Its end state is Saved. At every moment, exactly one location holds a complete copy. After a crash at any point, Save continues from what is on disk. It does not start again and it does not lose the session.
*Avoid in all text (code, docs and UI):* "land", "lands", "landed" and "landing". The step is Save and its end state is Saved. The UI says saved, or shows a saved session as Ready.

Save does one step at a time. It records each step durably before it does the next step. Thus each step must recognise, from disk alone, a step that completed just before a crash. The app removes the Working-location copy only after it records the session under its final name.

### Staging
The hidden, session-specific folder inside the storage folder. Save assembles the session in this folder before the final rename. The folder is next to the final name, so the last step of every Save is a same-volume rename. This is also true when the Working location is on another volume and Save had to copy the session into Staging.

A staging folder is removed at launch only when its session is recorded as completed: until then it may hold the only complete copy.

### Home folder
The folder that Save put a session into. A session keeps its home folder for life. A rename of the session changes the folder's name, not the folder: the session still has one home folder. When a saved session is resumed, recorded into and processed again, the app updates its home folder in place (it adds new audio next to the old audio and replaces the documents). It does not save a second copy.

### Title
The name of a session, shown on Home, on the session page and in the home folder's name. Processing proposes it. The user can rename it at any time, and a title the user set is never replaced automatically. A session with no title yet shows its start time.

## The library

### Library
Every saved session in the storage folder. After a change of storage folder, the library forgets the sessions of the old folder and leaves their folders on disk. Several computers of one person can share a storage folder through a sync service (OneDrive, Google Drive, Dropbox, iCloud). Each computer lists the whole library, including the sessions that other computers recorded. The home folders are the authority: each computer keeps its store in step with them through a library scan at launch and a folder watcher while Longhand runs.

A computer that only receives a sync changes no home folder. An automatic step, such as a launch reconcile, writes into a home folder only for work that this computer started and recorded. A partial sync can make the files of one home folder disagree for a time, so the shared files cannot tell which computer made a change.

### Adopt
The step that makes a Saved row in this computer's store from a home folder that another computer saved. The row is built from `session.md` and `notes.md`. Its opposite, forget, removes a row from the store and does not touch any folder: a computer forgets a session when its home folder is deleted from an available storage folder.

### Move
The step that copies every saved session from the old storage folder to a new one, when the user changes the storage folder and chooses Move. Move works on one note at a time and records each step. A crash, a full disk or an unavailable folder pauses it, and it continues from what is on disk after a launch or a Retry. A note leaves the Move without a copy only when its source is gone from an old folder that is still available; when the whole old folder goes away, even in the middle of a Move, the Move pauses. At every step, one complete copy of each note exists. The other choice, Leave them, changes the storage folder and leaves the old notes where they are; the library then forgets them. The user cannot change the storage folder while a Move is open.

### Conflict copy
A second copy of a session's file that a sync service makes when two computers change the file before either change syncs, for example `notes (1).md`. Longhand does not merge conflict copies. It shows a notice that names them. When a notes save finds that `notes.md` changed since the notes were loaded, Longhand keeps `notes.md` and saves the edit as its own conflict copy, `notes (Longhand conflict YYYY-MM-DD HHMM).md`.

## Recording

### Microphone choice
The input that a note records from. The user chooses the usual microphone in Settings, or "System default", and can change it for one note on the recording page. The choice is saved by the device ID, with the name as a label. A chosen microphone that is missing or disconnects is replaced by the system default, and the choice stays. When the chosen microphone is back for two polls in a row, the note moves back to it. Each change of input is a segment boundary.

### System audio
All audio that the computer plays, from any app. A note records it only while its Computer audio switch is on. The UI calls it "Computer audio"; "system audio" is the term in code and docs only. A note with system audio saves the raw microphone on the left channel and the raw system audio on the right. The microphone is the clock: the system audio is padded or trimmed to match it. On macOS the capture is a global process tap that leaves out Longhand's own sounds and the known mixer apps (apps that play the audio of other apps again, often with the microphone in it), and needs the Screen & System Audio Recording permission. On Windows it is a loopback of one output device, which follows the device where another app plays.

### Echo removal
The processing step after Stop that removes the system audio that the microphone heard from the speakers. It changes only the audio that processing uses, never the saved recording. It is distinct from the echo gate, which only leaves out microphone chunks that are mostly echo from the live mix during recording.

### Echo span
A part of a stereo segment where the microphone holds echo of the system audio. Assemble finds echo spans by the delay of the system audio in the microphone channel, and runs echo removal only inside them. Transcribe treats each echo span as a gap: it removes the live words inside the span and recognizes the span again from the cleaned mix.

## Transcription

### Recognition model
The model that turns speech into text. The user selects a transcription model and compatible processing choices. A note records its configuration in use and exact model artifacts when recording starts. A later change to the defaults does not change that note. Longhand selects the runtime internally.

### Speaker recognition
The user-facing term for diarization: assigning anonymous speaker labels to speech. It does not identify named people, enroll voices, or retain voiceprints. A separate speaker model supplies a timeline, and Longhand joins it with the text. Transcription models do not supply the labels. Speaker recognition can also be off. The selected artifacts determine valid processing times and speaker limits.

### Live transcript
The text that a streaming recognition model makes during recording. It shows in the Transcript tab when the selected model supports live transcription and the note uses that mode. Live text and speaker labels can be provisional. The runtime can revise them before finalization. Compatible durable results are reused after Stop; missing or invalid results require processing. Live speaker analysis is a separate choice and can run with an offline transcription model.

### Part
A span of a live transcript that the app saves as one checkpoint. A part ends at the first pause of at least 600 ms after 4 minutes, and always at 8 minutes. The live stream then starts again for the next part.

### Gap
Audio of a session that has no live transcript: a span that the live transcript missed because it fell behind, failed or stopped, or a part that did not save before a crash. After Stop, recognition transcribes only the gaps in batch ("gap fill").

### Transcribe Again
The menu action that makes the transcript and summary of a saved note again. It asks for one confirmation and offers no choices. The run uses the models, speaker source, and language in Settings, always with After Stop timing, and the speaker count of the note, including "auto". When the saved timed text is compatible, the run uses it again. The recording, notes, and title stay. During the run, the note shows progress in place of the transcript and summary. A failed run keeps the previous result.

### Model bundle
A transcription model and every supporting model that one of its valid choices can use: speaker models for each speaker source, and the NeMo copy of the model for live speaker labels. Longhand downloads the full bundle when the user selects the transcription model, and shows its total size. The user sees and deletes only transcription models. When a transcription model is deleted, Longhand deletes each supporting model that no other bundle needs.

### Supporting model
A model file that a transcription model needs for some choices, such as a speaker model or a NeMo copy. It is internal: Settings does not list it. It stays on disk while a bundle of a downloaded, selected, or pending transcription model contains it, or while a note that is not finished needs it.

## Summaries

### Summary prompt
A named text in Settings that tells the model what a summary contains and how it is organized: what to include, how the user's notes are weighed against the transcript, the sections, the level of detail and the tone. Exactly one summary prompt is active, app-wide. Two are built in: "Topic summary" (the default) and "Play-by-play summary". The app sends fixed rules with every summary prompt, and a summary prompt cannot change them: transcript and notes are data, never instructions; a person is never labelled "Speaker N" and never given a guessed name.

## Shared settings

### Shared settings
The settings that every computer with the same storage folder uses: the summary prompts, the active summary prompt and the chosen configuration, and the setting for automatic update checks. They are in the hidden `.longhand` folder of the storage folder, so the sync service delivers them to each computer. Other settings stay on each computer, for example the microphone, the shortcuts, the storage folder itself and the track. A computer changes only the fields that the user changed and keeps the fields it does not know. When the shared settings are not available, Settings shows them as read-only, and the computer uses its last known values.

### Chosen configuration
The recognition choices that the user made in Settings and that every computer shares: the transcription model, live transcript, speaker source, speaker processing mode, language and default speaker count. It is what the user wants, not always what a computer can use now. Compare Configuration in use.

### Configuration in use
The recognition choices that this computer uses for a new note. It equals the chosen configuration when this computer has every model that the choice needs. Until then, it stays on the last configuration whose models are on disk, and Longhand downloads the missing models. A new note that starts in this time uses the configuration in use, or the chosen configuration with live transcript off when its models are on disk. When the download ends, the configuration in use follows the chosen configuration. It is local to each computer.

## Shortcuts

### Shortcut
A global key combination that does one Longhand action from any app. There are two: the recording shortcut starts and stops a recording, and the show shortcut shows the main window, or hides it when it is in front. The user can clear each one; a cleared recording shortcut is a choice, not a fault, and New Note still starts a recording.

### Chord
The keys of a shortcut: one or more modifier keys and one other key. A shortcut is the action; its chord is the keys the user sets for it. The two shortcuts cannot have the same chord, and a chord that another app holds is refused.

### Key capture
The state in which a Settings row waits for the user to press a new chord. It is distinct from audio capture, which records the microphone. During key capture both shortcuts are paused, so their chords start nothing, and only one row waits at a time. Escape, Tab or a click elsewhere ends key capture with no change.

## About

### Full licence list
The full licence text of every open source component that the app ships on one platform, direct and indirect, shown in its own window from Settings > About. Each platform has its own list, stored in the repository. A check makes the list again from the dependencies and fails when the result differs from the stored list. The UI calls it "All open source licences".

### Native entry
A hand-kept record in the full licence list for a shipped component that no licence tool can read, such as a runtime library or a model that one of Longhand's own crates compiles in. Each native entry names its licence files and its source. Nothing fails when a native entry is missing, so a change that ships a new file of this kind must add one.

## Updates

### Track
The stream of builds that the updater offers to an installed app. A build records its track, and the track is a setting of the one app: switching tracks needs no reinstall. Only the preview track exists now. A stable track comes later.

### Preview build
A signed, notarized macOS build that CI makes from each push to main that passes CI. Its version is `<next stable version>-preview.<UTC commit time as YYYYMMDDHHMMSS>`, so a later commit on main has a higher version.

### Update manifest
The file that identifies the newest build of a track: its version, date, archive address and signature. The app reads only this file when it checks for updates. For the preview track, it is `preview/latest.json` in the public builds repository.
