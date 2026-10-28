# Shared settings and the Background

The shared settings are the settings that every computer with the same storage folder uses. They are in `.longhand/settings.json`, one JSON object. Each computer reads a change in the file within a few seconds.

## Rules

- Change only the values that the user asked for. Keep every key, also a key that this file does not name.
- Do not change `schema_version`.
- Write valid JSON. Longhand does not use a file that does not parse, and after about one minute it shows a notice for it.
- Keep the format of the file: two spaces of indent and the keys in their order.

## The keys

| Key | Value | Meaning |
|---|---|---|
| `schema_version` | A number | The version of the file. Do not change it. |
| `active_summary_prompt` | A prompt id | The default prompt. `topic-summary`, `play-by-play`, or the `id` of a file in `.longhand/prompts/`. |
| `note_types` | A list of type names | The note types, in the order that Settings shows them. See `types.md`. |
| `default_note_type` | A type name, or `null` | The type of a new note. `null` is General. |
| `note_type_prompts` | An object: type name to prompt id | The prompt of each note type. "General" is the key for General. A type with no key uses the default prompt. |
| `show_recording_widget` | `true` or `false` | A new note shows the recording widget. |
| `new_note_records` | `true` or `false` | New Note and the recording shortcut start a note that records. With `false`, they start a note with no audio. |
| `check_updates_automatically` | `true` or `false` | Longhand checks for updates automatically. |
| `audio_removal` | `"off"`, or whole days from 1 to 3650 | Longhand removes the audio of a saved note that many days after the note last recorded. |
| `live_transcript` | `true` or `false` | The transcript shows during the recording, when the model supports it. With a model that cannot, Longhand transcribes after Stop and keeps the value. |
| `speaker_source` | `"off"`, `"pyannote"`, `"sortformer"` or `"nemotron"` | The speaker recognition model. |
| `speaker_processing_mode` | `"during_recording"` or `"after_stop"` | When speaker recognition runs. With a speaker source that cannot run during the recording, Longhand uses `"after_stop"` and keeps the value. |
| `default_speaker_count` | `null`, `{"kind": "exact", "count": N}` or `{"kind": "range", "min": N, "max": M}` | The number of speakers for a new note. `null` finds the number. N is 1 or more, and N is not more than M. |
| `recognition_language` | A language code, or `null` | The language of the transcript. `null` finds the language. With a model that does not support the language, Longhand finds the language and keeps the value. |
| `asr_model` | A model id | The transcription model. Change it in Longhand Settings: this skill cannot check the list of models. |

A change to a recognition setting (`asr_model`, `live_transcript`, `speaker_source`, `speaker_processing_mode`, `default_speaker_count`, `recognition_language`) can make each computer download models.

## Settings that are not in the file

Some settings belong to one computer, and are not shared: the shortcuts, the interface zoom, the Computer audio default, Hide from Dock, the alert sound, the summary endpoint and its key, the microphone and the storage folder. Tell the user to change them in Longhand Settings on that computer.

## The Background

`.longhand/background.md` is the Background: plain text that each summary uses. It tells the summary model what it must know: the user's name, the user's work, names and terms that often occur, and words that the transcript often gets wrong.

- The file has no frontmatter. Write plain text.
- At most 4,000 characters.
- To clear the Background, make the file empty. Do not delete it.
- The Background counts as evidence for names, but the fixed rules of each summary are more important than the Background.
