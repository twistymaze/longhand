# Provenance

Borrowed material in this directory, one record per file (R27). The other files here are written for Longhand.

## `ShortcutField.tsx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `apps/desktop/src/settings/dictation-shortcut.tsx` (the key-down capture: a chord from the modifier flags and `KeyboardEvent.code`, complete at the first key that is not a modifier; Escape cancels, Tab and blur end capture; keys accepted only after the prepare step; an attempt counter against a stale result)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — the pure key handling moved to `chord.ts`; the prepare step pauses Longhand's own shortcuts, through one pause that both rows share; a settings row with Reset and Clear; the Fn and Right Command buttons and the translations dropped

## `chord.ts`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `apps/desktop/src/settings/dictation-shortcut.tsx` (the `onKeyDown` handler: modifier keys ignored, modifiers from the event flags, the key from `event.code`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — a pure function that returns what the key-down means, so it can be tested
