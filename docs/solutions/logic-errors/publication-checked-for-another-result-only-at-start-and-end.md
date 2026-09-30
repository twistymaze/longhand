---
title: A result that another computer published during the writes left the note folder with mixed files
date: 2026-09-30
category: logic-errors
module: Result publication into a note folder (crates/session-save/src/publication.rs and the publication callers in crates/pipeline/src/save.rs)
problem_type: logic_error
component: service_layer
severity: high
symptoms:
  - "A session.md from another computer that arrived during the writes did not stop the writes: this computer still replaced transcript.md and summary.md"
  - "publish_with then returned Outcome::Conflict, whose doc comment said \"Nothing was written\""
  - "drop_publication treated the conflict as a clean drop, so session.md (other computer) and transcript.md / summary.md (this computer) stayed mismatched with no repair"
  - "The conflict test passed, because it wrote the other session.md only before the rerun started"
root_cause: concurrency
resolution_type: code_fix
framework_version: rust 1.94.0
related_components:
  - library
  - background_job
  - testing_framework
tags:
  - storage-folder
  - sync
  - multi-file-publication
  - roll-forward
  - idempotent-steps
  - conflict-detection
  - crash-recovery
  - note-folder
---

# A result that another computer published during the writes left the note folder with mixed files

## Problem

A note has several result files in its home folder: `recognition.jsonl`, `transcript.md`, `summary.md` and `session.md`. The storage folder can be in a synced cloud folder that several computers share. PR #29 removed the old result copies in `.longhand-results/` and replaced them with a publication: direct writes into the home folder, with `session.md` last. This doc describes that publication pattern and a defect that code review of PR #29 found in it.

### How a publication works

1. **The result is complete in local staging first (R3).** Save, Resume and Transcribe Again stage the result in `working_root/<id>/`. Regenerate Summary stages it in its `.longhand-summary-*` folder. A failed run changes no file in the home folder.
2. **A local record.** `Publication::new` (`crates/session-save/src/publication.rs:130-136`) reads the result fields of the home `session.md` as `before`. The staged `session.md` is `after`. `Publication::record` (`publication.rs:140-142`) writes both into `publication.json` in the local staging folder (`RECORD`, `publication.rs:23`). The record never syncs. Its presence means "unfinished publication" (KTD1).
3. **Write order (KTD5).** `WriteSet::Result` writes `recognition.jsonl`, `transcript.md`, `summary.md`, in that order (`publication.rs:43-48`). `WriteSet::Summary` writes only `summary.md`. `session.md` is always last (`publication.rs:208-216`). Most sync clients upload in write order, so a receiving computer gets `session.md` after the files that it describes.
4. **Result fields, not whole files (KTD2).** `ResultFields` (`publication.rs:54-63`) are `session_id`, `transcript_fingerprint`, `summary_fingerprint`, `recognition_artifact`, `recognition_audio`, `speaker_count` and `recognition_snapshot`. `Publication::patch` (`publication.rs:224-234`) writes `session.md` as a patch: it keeps the title and title source of the home file. A title change or a rename between two steps is not a conflict. For `WriteSet::Summary`, `compared` (`publication.rs:81-87`) leaves out the recognition references, because the patch takes them from the home folder at write time.
5. **Commit is detected from disk alone.** "Committed" means the home result fields equal the staged result fields (`publication.rs:185-187`). No record says "committed".
6. **Each step can run again (KTD3).** A file whose digest already equals the staged file is skipped (`publication.rs:196-198`). Other files are replaced with `home_write::copy_atomic` (`crates/session-save/src/home_write.rs:36`), so each target is the old file or the complete new file. With a record present, a rerun only rolls forward: it skips `render`, the notes check and the `replacement-base` check (`crates/pipeline/src/save.rs:416-435`).
7. **Conflict (KTD4).** A conflict is home result fields that equal neither `before` nor `after`: another computer published a result. This computer drops its unfinished result and writes nothing more. `drop_publication` (`save.rs:576-591`) deletes the record and the checkpoints and ends the job with `CHANGED_ELSEWHERE` (`save.rs:48-49`): "This note changed on another computer. Longhand kept that version." The summary paths do the same (`save.rs:110-114`, `save.rs:1211-1217`).

### The defect

Before the fix, `publish_with` read the home result fields only twice: once before the first write, and once before `session.md`. The loop that writes the other files did not check. A `session.md` from another computer can arrive through the sync at any time. When it arrived during the loop, this computer:

1. Overwrote `transcript.md` and `summary.md` with its own result.
2. Found the conflict only before `session.md`, and returned `Outcome::Conflict`.
3. The doc comment of `Outcome::Conflict` said "Nothing was written (KTD4)". `drop_publication` trusted it and did a clean drop.

The home folder then held the other computer's `session.md` and this computer's `transcript.md` and `summary.md`. Nothing repaired it. The reliability reviewer and a Codex cross-model pass of the PR #29 code review found this independently.

## Symptoms

No user saw these symptoms. Code review found the defect before the merge. The review showed these results:

- A note with a `session.md` from one computer and a `transcript.md` / `summary.md` from another, after a Transcribe Again or a Resume on one computer and a publication on another computer at almost the same time.
- The job failed with "This note changed on another computer. Longhand kept that version." But Longhand did not keep that version: the files of the other version were already replaced.
- The note then waited for a sync (its fingerprints did not match), and after about one minute (`HAND_EDIT_WAIT`, `crates/library/src/reconcile.rs:46`) it counted as a hand edit.
- `another_result_between_two_writes_is_a_conflict_and_nothing_is_written` (`crates/session-save/tests/publication.rs:180-204`) passed. It writes the other `session.md` after a crash and before the rerun, so only the first check ever saw a conflict.

## What Didn't Work

- **Result copies in `.longhand-results/<generation>/`.** The earlier design wrote a complete copy of each result into the note folder and let a receiving computer show the last complete copy. The copies were never deleted, synced to every computer, and kept old versions of the transcript, a privacy problem. The user found them in the storage folder. The plan (`docs/plans/2026-09-30-0850-refactor-remove-result-copies-plan.md`, Problem Frame and Key Decisions) chose to remove them: a few seconds of mismatch on a receiving computer is acceptable, and the last change wins (rule 8 of `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`).
- **A marker in the note folder in place of a local record.** Nothing in a synced folder can tell a crashed publication from a hand edit (KTD1). Any file that this computer writes there also syncs to other computers and can arrive in any order. The record stays in local staging.
- **A per-step record, as in the save state machine.** See "When to use which pattern" below.
- **Two checks (before the first write and before `session.md`).** These were the defect. A check only at the two ends leaves the whole loop open to a `session.md` that arrives during it.
- **A conflict test that changes `session.md` between two runs.** It proved the first check only. The window was inside one run.

## Solution

Read the home result fields again before each `copy_atomic`, and stop at the first difference from `before`. Correct the doc comment of `Outcome::Conflict`. Both changes are in PR #29.

Before (`crates/session-save/src/publication.rs` in PR #29 unit U3):

```rust
let now = ResultFields::of(&SessionDoc::read(home)?).compared(self.set);
if now == after && now != before {
    return Ok(Outcome::Committed);
}
if now != before {
    return Ok(Outcome::Conflict);
}
for name in self.set.files() {
    let Some(staged) = digest(&local.join(name))? else { continue };
    if digest(&home.join(name))?.as_ref() == Some(&staged) {
        continue;
    }
    home_write::copy_atomic(home, name, &local.join(name))?;
    after_write(name)?;
}
```

After (`publication.rs:179-207`):

```rust
let fields = || -> io::Result<ResultFields> {
    Ok(ResultFields::of(&SessionDoc::read(home)?).compared(self.set))
};
let now = fields()?;
if now == after && now != before {
    return Ok(Outcome::Committed);
}
if now != before {
    return Ok(Outcome::Conflict);
}
for name in self.set.files() {
    let Some(staged) = digest(&local.join(name))? else { continue };
    if digest(&home.join(name))?.as_ref() == Some(&staged) {
        continue;
    }
    // Another computer's `session.md` can arrive during the
    // writes. Check before each write, so that at most the file in
    // progress replaces a file of the other result.
    if fields()? != before {
        return Ok(Outcome::Conflict);
    }
    home_write::copy_atomic(home, name, &local.join(name))?;
    after_write(name)?;
}
```

The doc comment of `Outcome::Conflict` (`publication.rs:95-100`) changed from "Nothing was written (KTD4)" to: "This computer writes nothing after it finds the conflict (KTD4). The files it wrote before that stay, and the note waits for a sync until the mismatch counts as a hand edit."

The new test `another_result_during_the_writes_stops_the_next_write` (`crates/session-save/tests/publication.rs:206-226`) writes the other computer's `session.md` from the `after_write` hook, after the first write. It expects `Outcome::Conflict`, one write (`recognition.jsonl`), and the old `transcript.md` and `summary.md`.

## Why This Works

- The check is now next to each write. The window for a `session.md` that arrives from another computer is the one file in progress, not the whole loop.
- The check compares result fields only, so a title change or a rename by this computer or by another computer between two writes does not stop the publication (KTD2).
- The check before `session.md` stays (`publication.rs:210-213`). It also reads the current file for the title patch.
- The loop needs no other change for a rerun: a stop at a conflict is the same as a stop at a crash, and a rerun from the record reaches the same end state.

**Residual risk (accepted).** The file in progress when the other `session.md` arrives can still be this computer's. The note then waits for a sync, because a fingerprint does not match, and after about one minute it counts as a hand edit (R8, KTD7). The next Transcribe Again or Regenerate Summary replaces or accepts the file. Rule 8 of `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md` applies: the last change wins, conflicts belong to the sync service, and Longhand does not add a lock or a "has this synced yet" guess to close this window.

## When to use which pattern

`docs/solutions/logic-errors/save-state-machine-crash-window-recovery.md` describes the save state machine (`SaveState`). The two patterns solve different problems.

| | Save state machine (`SaveState`) | Publication (`publication.json`) |
|---|---|---|
| Target | A new folder name in the storage folder that no other writer uses yet | An existing home folder that other computers and the user can change |
| Record | One durable state per step in SQLite, written after the effect of each step | One local record, written once before the first write. No record per step |
| Recovery | Each step recognizes its own completed effect from evidence on disk (the ownership marker, the candidate names) and returns the same next state | Each write compares content (file digests, result fields) and skips what is already done. "Committed" is home fields == staged fields |
| Effects | Moves and renames, which are not idempotent by content and can produce a duplicate (`NAME-2`) | Atomic file replacements, which are idempotent by content |
| Evidence in the target | A marker file in the folder, which is safe because the folder is not shared until the final rename | None. Any file in a synced folder syncs to other computers, and a crash cannot be told from a hand edit |
| Other writers | None during the steps | Expected. Check for them before each write |

Use the state machine when the steps are moves or renames into a location that only this computer writes, and the effect of a step can be recognized by evidence that this computer placed. Use the publication pattern when the target is a shared, synced folder, the effects are whole-file replacements that can be compared by content, and another writer can change the target between two steps.

## Prevention

- In a multi-file write into a synced folder, check for another writer immediately before each write, not only at the start and the end.
- Write the doc comment of a conflict outcome to say what was written before the conflict. A caller that drops state trusts that comment.
- For each check in a loop of writes, add a test that injects the other writer from the hook after each write, not only between two runs. The hook in `publish_with` exists for this.
- Keep all evidence of an unfinished publication out of the note folder (KTD1).
- Compare only the fields that a publication sets (KTD2), so that a title change or a rename is not a conflict.

Test sketch (one test per write boundary):

```rust
#[test]
fn another_result_after_write_n_stops_write_n_plus_1() {
    for stop in 1..=3 {
        let (_root, home, local, publication) = fixture(WriteSet::Result);
        publication.record(&local).unwrap();
        let before: Vec<String> = ["transcript.md", "summary.md"]
            .iter().map(|n| read(&home, n)).collect();
        let other_home = home.clone();
        let mut written = Vec::new();
        let outcome = publication
            .publish_with(&home, &local, &mut |name| {
                written.push(name.to_string());
                if written.len() == stop {
                    // The other computer's `session.md` arrives now.
                    let mut other = SessionDoc::read(&other_home)?;
                    other.transcript_fingerprint = Some(fingerprint(b"other"));
                    std::fs::write(other_home.join("session.md"), other.render())?;
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(outcome, Outcome::Conflict);
        assert_eq!(written.len(), stop, "no write after the conflict");
        // Files after the boundary keep the other computer's content.
        // `session.md` keeps the other computer's result fields.
    }
}
```

Also test the caller: after `Outcome::Conflict`, `drop_publication` deletes the record, and the next run does not write into the home folder.

## Related Issues

- PR #29: the removal of the result copies, the publication, and this fix.
- `docs/plans/2026-09-30-0850-refactor-remove-result-copies-plan.md`: R2 to R5 and KTD1 to KTD5 (the publication), KTD7 (the one-minute wait), and the state diagram "Publication on the owner".
- `docs/solutions/logic-errors/save-state-machine-crash-window-recovery.md`: the step-then-record state machine of Save.
- `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`: rules 2 (patch only this save's fields) and 8 (the last change wins).
- `CONCEPTS.md`: storage folder, home folder, waiting for a sync.
