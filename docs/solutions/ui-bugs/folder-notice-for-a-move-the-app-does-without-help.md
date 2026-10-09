---
title: A folder notice showed for a move that the app does without help
date: 2026-10-09
category: ui-bugs
module: "Note page type and title notices (src/session/TypeAndTags.tsx, src/session/TitleField.tsx)"
problem_type: ui_bug
component: frontend
symptoms:
  - "A type change during a recording showed \"The type changed. The folder moves when the note is free.\""
  - "A type move or folder rename that failed now showed a caution with the reason, but the app tries again without help"
  - "The type button had a tooltip about a folder move that waits"
root_cause: logic_error
resolution_type: code_fix
severity: low
tags: [notice, message-rule, note-type, folder-rename, type-move, attention]
---

# A folder notice showed for a move that the app does without help

## Problem

The note page showed a caution notice when the folder of a note moved later or had to try again. The user had nothing to do: the pipeline records the owed move or rename and does it later. The notice took the attention of the user for normal behaviour.

## Symptoms

- The user started a new note without a type and chose the type at once. The note recorded, so the folder move waited, and the page showed "The type changed. The folder moves when the note is free." This is the most frequent path, not an edge case.
- A type move or folder rename that failed now (for example, a file in the folder was open in another app) showed a caution with the reason.
- The type button showed the same "moves when the note is free" text as a tooltip.

## What Didn't Work

- The note types and tags plan (`docs/plans/2026-10-07-1940-feat-note-types-and-tags-plan.md`, U10, Approach step 3) specified the "moves later" notice, and a test required it. The plan designed the notice from the internal state of the folder (where the folder is now), not from what the user must do. Review of the plan and the code did not find the problem, because the notice was correct. It was only not necessary.
- Keeping the "kept" notice because it gives a reason was considered. The reason does not help: the app tries again, and the user cannot see whether the next try succeeded.

## Solution

PR #49 removed the notices. `typeNotice` in `src/session/TypeAndTags.tsx` and `commitTitle` in `src/session/TitleField.tsx` now return a notice only for an error:

```ts
function typeNotice(result: Awaited<ReturnType<typeof commands.setNoteType>>): Notice | null {
  if (result.status === "error") return { tone: "negative", text: sentence(result.error) };
  return null;
}
```

The type button tooltip went too. Its only data source, `SessionEntry.type_move_waits` in `src-tauri/src/commands/session.rs`, had no other reader and was removed. `pipeline::type_move::shown_type` still returns the pending state for the pipeline.

The general rule went into the UI design section of `CLAUDE.md`: show a notice, warning, or error only when the user must do something, or when something abnormal occurred that is likely to be important to the user. Do not show a message for normal behaviour or for a problem that the app corrects without help, unless the app's tries to correct it do not succeed.

## Why This Works

The pipeline already does the work that the notice described:

- `TypeChange::Later` and `TypeChange::Kept` in `crates/pipeline/src/type_move.rs` both mean "the folder moves later"; the note shows the new type at once.
- `reconcile` in `crates/pipeline/src/folder_rename.rs` records that this computer owes a rename for `Later` and `Kept`, and the launch reconcile tries again.

So the notice reported an internal step, not a state that the user must know about. Removing it removes noise without hiding a problem the user can fix.

## Prevention

- Before you add a notice, a warning, or a tooltip, ask: what must the user do after they read it? If the answer is "nothing, the app does it", do not add it.
- Do not design a message from internal state (where a file is, which step is pending). Design it from the action or the abnormal result.
- A failure that the app retries gets no message at the time. If it can stay failed, report it only after the tries do not succeed. That report does not exist yet: `docs/ideas/someday/failed-folder-move-needs-attention.md`.
- When a plan lists a notice in its approach or test scenarios, check it against the rule in `CLAUDE.md` during plan review.
- A test that requires a notice for a normal result is a sign of this problem. Test that the normal result shows no notice, and that an error still shows.

## Related Issues

- PR #49 (merged): the fix.
- `docs/ideas/done/no-folder-notices-on-the-note-page.md`: the idea for this fix.
- `docs/ideas/someday/failed-folder-move-needs-attention.md`: the report for a move that keeps failing.
- `docs/solutions/logic-errors/receiving-computer-renamed-synced-folder-back.md`: the `Later` and `Kept` rename results and the retry at launch.
- `docs/solutions/logic-errors/verify-stopped-on-detected-sound-without-words.md`: an earlier case of the same rule. The note shows nothing for short wordless sound.
