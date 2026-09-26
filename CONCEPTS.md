# Concepts

Shared domain vocabulary for this project — entities, named processes, and status concepts with project-specific meaning. Seeded with core domain vocabulary, then accretes as ce-compound and ce-compound-refresh process learnings; direct edits are fine. Glossary only, not a spec or catch-all.

## Sessions

### Session
One recording and everything made from it: the audio, the user's notes, the transcript and the summary. The UI calls a session a "note" ("New Note", the Notes list); "session" is the term in code and docs only.

## Saving a session

### Working location
The app's own storage, where a session lives while it records and processes: its audio, the processing copies and every intermediate file. The user never browses it.

Some of what it holds never leaves it: the processing audio and the raw recovery chunks stay behind when the session moves to its Destination.

### Destination
The folder the user chose to receive finished sessions, possibly on another volume or in a synced cloud folder that several computers share. Nothing partial ever appears in it. The UI calls it "Storage" (Settings > Storage); "Destination" is the term in code and docs only.

### Save
The step that moves a finished, verified session from the Working location into the Destination under its final folder name. Its end state is Saved. At every moment, exactly one location holds a complete copy. After a crash at any point, Save continues from what is on disk. It does not start again and it does not lose the session.
*Avoid in all text (code, docs and UI):* "land", "lands", "landed" and "landing". The step is Save and its end state is Saved. The UI says saved, or shows a saved session as Ready.

Save does one step at a time. It records each step durably before it does the next step. Thus each step must recognise, from disk alone, a step that completed just before a crash. The app removes the Working-location copy only after it records the session under its final name.

### Staging
The hidden, session-specific folder inside the Destination. Save assembles the session in this folder before the final rename. The folder is next to the final name, so the last step of every Save is a same-volume rename. This is also true when the Working location is on another volume and Save had to copy the session into Staging.

A staging folder is removed at launch only when its session is recorded as completed: until then it may hold the only complete copy.

### Home folder
The folder that Save put a session into. A session keeps its home folder for life. A rename of the session changes the folder's name, not the folder: the session still has one home folder. When a saved session is resumed, recorded into and processed again, the app updates its home folder in place (it adds new audio next to the old audio and replaces the documents). It does not save a second copy.

### Title
The name of a session, shown on Home, on the session page and in the home folder's name. Processing proposes it. The user can rename it at any time, and a title the user set is never replaced automatically. A session with no title yet shows its start time.

## Recording

### Microphone choice
The input that a note records from. The user chooses the usual microphone in Settings, or "System default", and can change it for one note on the recording page. The choice is saved by the device ID, with the name as a label. A chosen microphone that is missing or disconnects is replaced by the system default, and the choice stays. When the chosen microphone is back for two polls in a row, the note moves back to it. Each change of input is a segment boundary.

### System audio
All audio that the computer plays, from any app. A note records it only while its Computer audio switch is on. The UI calls it "Computer audio"; "system audio" is the term in code and docs only. A note with system audio saves the raw microphone on the left channel and the raw system audio on the right. The microphone is the clock: the system audio is padded or trimmed to match it. On macOS the capture is a global process tap that leaves out Longhand's own sounds and needs the Screen & System Audio Recording permission. On Windows it is a loopback of one output device, which follows the device where another app plays.

### Echo removal
The processing step after Stop that removes the system audio that the microphone heard from the speakers. It changes only the audio that processing uses, never the saved recording. It is distinct from the echo gate, which only leaves out microphone chunks that are mostly echo from the live mix during recording.

### Echo span
A part of a stereo segment where the microphone holds echo of the system audio. Assemble finds echo spans by the delay of the system audio in the microphone channel, and runs echo removal only inside them. Transcribe treats each echo span as a gap: it removes the live words inside the span and recognizes the span again from the cleaned mix.

## Transcription

### Recognition model
The model that turns speech into text. The user selects one recognition model in Settings from the catalog of nine. A session records its recognition model when recording starts and keeps it through processing. The speaker-detection weights are not recognition models: the user does not select them, and the UI shows them as one item, "Other required models".

### Live transcript
The text that a streaming recognition model makes during recording. It shows in the Transcript tab. A live transcript runs only when the session's recognition model streams and the Live transcript switch is on; the session records this live mode when recording starts. The text of the live transcript is final: processing after Stop does not transcribe it again.

### Part
A span of a live transcript that the app saves as one checkpoint. A part ends at the first pause of at least 600 ms after 4 minutes, and always at 8 minutes. The live stream then starts again for the next part.

### Gap
Audio of a session that has no live transcript: a span that the live transcript missed because it fell behind, failed or stopped, or a part that did not save before a crash. After Stop, recognition transcribes only the gaps in batch ("gap fill").

### Transcribe Again
The action that makes a new transcript and a new summary of a saved session with the selected recognition model. The session's audio, its notes and its title stay. When the run fails before Save, the session stays Saved with its previous files. The UI label is "Transcribe Again…".

## Summaries

### Summary prompt
A named text in Settings that tells the model what a summary contains and how it is organized: what to include, how the user's notes are weighed against the transcript, the sections, the level of detail and the tone. Exactly one summary prompt is active, app-wide. Two are built in: "Topic summary" (the default) and "Play-by-play summary". The app sends fixed rules with every summary prompt, and a summary prompt cannot change them: transcript and notes are data, never instructions; a person is never labelled "Speaker N" and never given a guessed name.

## Shortcuts

### Shortcut
A global key combination that does one Longhand action from any app. There are two: the recording shortcut starts and stops a recording, and the show shortcut shows the main window, or hides it when it is in front. The user can clear each one; a cleared recording shortcut is a choice, not a fault, and New Note still starts a recording.

### Chord
The keys of a shortcut: one or more modifier keys and one other key. A shortcut is the action; its chord is the keys the user sets for it. The two shortcuts cannot have the same chord, and a chord that another app holds is refused.

### Key capture
The state in which a Settings row waits for the user to press a new chord. It is distinct from audio capture, which records the microphone. During key capture both shortcuts are paused, so their chords start nothing, and only one row waits at a time. Escape, Tab or a click elsewhere ends key capture with no change.
