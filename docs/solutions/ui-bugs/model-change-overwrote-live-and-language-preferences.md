---
title: A model change overwrote the live and language preferences
date: 2026-10-28
category: ui-bugs
module: "Recognition settings (src/settings/RecognitionOptions.tsx; src-tauri/src/managers/settings.rs RecognitionChoice)"
problem_type: ui_bug
component: frontend
symptoms:
  - "A test of a model without live transcription saved After Stop, and a change back to a live model kept After Stop"
  - "A model without the chosen language saved Automatic, and the language did not come back"
  - "The timing and language controls changed when the user selected a different model"
root_cause: logic_error
resolution_type: code_fix
severity: medium
tags: [recognition, preferences, live-transcript, speaker-timing, language, older-builds, settings-window]
related_components: [service_layer]
---

# A model change overwrote the live and language preferences

## Problem

The Settings window resolved the best configuration that the selected model could use, and saved that configuration. So a model without live transcription saved `live_transcript: false` and After Stop speaker timing. A model without the chosen language saved Automatic language. A change back to a capable model kept the reduced values. Fixed in PR #73.

## Solution

Keep two values apart:

- **The preference.** `chosen_recognition` (a `RecognitionChoice` in the shared settings file) stores transcript timing, speaker timing and language as the user chose them. Only a change of that control changes them. A model change never changes them.
- **The effective configuration.** `RecognitionChoice::config()` resolves, for each model, what the model can use. It uses After Stop when the model does not stream or when the speaker source has no live mode (`SpeakerSource::has_live_mode`), and automatic language when the model does not list the language.

The configuration in use on each computer stores only `usable()` values (`set_in_use` calls `choice.usable()`), and comparisons use `same_use()`. Reason: an older build on the same computer reads that local store with no fallback. With the raw preference there, the older build could not start a recording (code-review P1 on PR #73). The shared file can hold a preference that the model cannot use. An older build on another computer can then show an error and keep its previous model. This was accepted in planning.

## UI rules from the user's manual test

The user rejected two designs before the accepted one:

1. **A control shows the preference, never the effective value.** The first version showed "After Stop" in the timing control when Whisper Turbo was selected. The user: "the option is the option and is never influenced by model choice. It's just a representation of the user preference." Do not disable options that the model cannot use. Every option can be chosen with every model.
2. **One line under the controls gives what will happen and why.** The line uses `text-caution`, not `text-label-secondary`, because grey text blended in. It names the cause. Use this sentence pattern from the user:
   - "Transcription will run after Stop when using Whisper Turbo because it does not support live transcription."
   - "Speaker recognition will run after Stop when using pyannote / WeSpeaker because it does not support live speaker recognition."
   - "Speaker recognition will run after Stop with 9 expected speakers because live speaker recognition does not support that many speakers."
   - "The language will be detected automatically when using Parakeet because it does not support German."

   The user rejected these versions: "with these settings" (vague), "only supports … after Stop" (does not say what will happen), and "Your setting is kept …" (a user expects a setting to stay; this text made the setting look invalid).
3. **Name the speaker count only when it is the cause.** The window resolves the live speaker models again with no count (`setCountLimits`). The count is the cause only when one of them then resolves. Otherwise the line names the speaker model override or the transcription model. When the transcription model causes both fallbacks, one sentence names it once.

## Prevention

- When a setting depends on another selection, store the user's value and resolve the effective value where it is used. Do not save the resolved value back over the user's value.
- A local store that older builds read must hold only values that those builds can use.
- Test the round trip: choose a preference, select a model that cannot use it, select a capable model again, and check the control and the saved choice.

## Related

- [Rules for settings that several computers share through a synced folder](../integration-issues/shared-settings-in-a-synced-folder-rules.md)
- [A notice for a folder move that the app does without help](folder-notice-for-a-move-the-app-does-without-help.md)
