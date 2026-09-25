---
title: Parallel worktree PRs that each raise a schema version must renumber at merge
date: 2026-09-25
category: workflow-issues
module: Settings store migrations (src-tauri/src/managers/settings.rs apply_settings_migrations) and session store migrations (crates/session-core/src/schema.rs)
problem_type: workflow_issue
component: development_workflow
related_components:
  - database
  - frontend
  - testing_framework
severity: high
applies_when:
  - "Two or more branches in separate worktrees each add a settings migration or raise CURRENT_SETTINGS_SCHEMA_VERSION"
  - "Two or more branches each add a numbered SQL migration to crates/session-core"
  - "A plan says to use the next unused version number"
  - "Main is merged into a branch that changes the Settings type or the IPC commands that the Settings window calls"
  - "Builds of an unmerged branch are installed on real machines"
symptoms:
  - "Git conflict in the version constant and the migration block of apply_settings_migrations"
  - "Tests that assert a literal schema version pass or fail for the wrong reason after the merge"
  - "tsc error in bun run build because literal Settings objects in tests lack the other branch's new fields"
  - "Settings-window test fails at run time with No mock for command speaker_models_status"
root_cause: missing_workflow_step
resolution_type: workflow_improvement
tags:
  - schema-version
  - settings-migration
  - sql-migrations
  - parallel-worktrees
  - merge-conflict
  - migration-numbering
  - test-fixtures
  - tauri-settings
---

# Parallel worktree PRs that each raise a schema version must renumber at merge

## Context

PR #6 (summary prompts) and PR #8 (recognition models, live transcript) were developed at the same time in separate worktrees. Each PR raised `CURRENT_SETTINGS_SCHEMA_VERSION` from 2 to 3 and added its own `if stored_version < 3` block to `apply_settings_migrations`:

- PR #6 sets `active_summary_prompt` to the Topic summary built-in for every store below version 3.
- PR #8 keeps `PREVIOUS_DEFAULT_ASR_MODEL` (Whisper) for a store of version 1 or more that has no `asr_model` key, because the default recognition model changed.

PR #6 merged first. When main was merged into PR #8, git reported a conflict in those lines. Notarized builds of both branches were already installed on the user's machines, so version-3 stores from both builds exist.

Each branch chose "the next unused number" correctly for its own base. The rule fails because each branch computes the number against the main it branched from, not against main at merge time. A third branch (summary model picker) started from main after PR #6 merged and before PR #8 merged. Its plan also says "Bump the schema version to the next unused number", which from that base is 4, the number PR #8 now uses (session history).

## Guidance

**1. At merge, the branch that merges second takes the next version.** Main's migration keeps its number. The second branch's migration moves to the next number, and the constant follows it. The merged result in PR #8:

Before (PR #8 branch):

```rust
pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 3;
// ...
if stored_version < 3 {
    if stored_version >= 1 && stored.get("asr_model").is_none() {
        settings.asr_model = PREVIOUS_DEFAULT_ASR_MODEL.to_string();
    }
    updated = true;
}
```

After (main now, `src-tauri/src/managers/settings.rs:22` and the migration block from `:362`):

```rust
pub const CURRENT_SETTINGS_SCHEMA_VERSION: u32 = 4;
// ...
if stored_version < 3 {
    // Every existing install changes to the new default prompt (KTD6).
    settings.active_summary_prompt = TOPIC_SUMMARY_ID.to_string();
    updated = true;
}
if stored_version < 4 {
    if stored_version >= 1 && stored.get("asr_model").is_none() {
        settings.asr_model = PREVIOUS_DEFAULT_ASR_MODEL.to_string();
    }
    updated = true;
}
```

**2. Guard a migration on field presence, not only on the version.** The model migration runs only when `asr_model` is missing (`settings.rs:387`). Every build writes `asr_model`, so a version-3 store from either build keeps its model. A version-3 store from the PR #8 build has no prompt fields, and serde defaults fill them. Thus a version-3 store from either build migrates correctly to version 4.

**3. Key tests on `CURRENT_SETTINGS_SCHEMA_VERSION`, not on a literal.** After the renumber, a test such as "a store at the current version is not updated" that used a literal 3 became wrong without an error. Two prompt tests that asserted `updated` passed only because of the version stamp, not because of the prompt logic. These tests now use the constant.

**4. Add a test for each store that an installed build can write.** `a_version_3_store_keeps_its_model_and_its_prompt` (`settings.rs:603`) loads a version-3 store from each build. It checks that the model, `live_transcript` and the active prompt are kept, and that the store from the PR #8 build gets the Topic summary default.

**5. After a merge of main, run the frontend gates, not only cargo.** Run `bun run build` (it runs `tsc`) and `bun run test`. The same merge broke:

- Literal `Settings` objects in `src/settings/Models.test.tsx` and `src/settings/SummaryPrompts.test.tsx` lacked the other branch's new fields (a `tsc` error).
- Main's new Settings-window test failed with "No mock for command speaker_models_status". The other branch's Settings window calls that command when it opens.

**6. The same rule applies to SQL migrations in the session store.** `crates/session-core/src/schema.rs` registers the migrations in a list for `rusqlite_migration`, which records the applied version in SQLite's `user_version` pragma. The position in the list sets the version, not the file name. PR #8 added 0004 and 0005. If a parallel branch also adds a migration, the branch that merges second moves its file to the next number and to the end of the list.

## Why This Matters

A textual conflict is the good case: it forces a decision. If the two migrations had been in different places or files, both PRs would merge without a conflict and both would claim version 3. An install that already reached version 3 with one build would never run the other build's migration. The result is a store that is marked current but is missing a change, with no error. For the session store, the same failure leaves a database at `user_version` N without the columns or tables of the other branch's migration N.

## When to Apply

- Before you merge a PR that adds a settings migration or a SQL migration: merge main into it, then compare its version number with main's.
- When two open worktree branches both change `apply_settings_migrations` or `crates/session-core/migrations/`.
- When a plan says "the next unused number": read that as "the next unused number on main at merge time".
- When builds of an unmerged branch are installed on real machines, because their stores then exist at that branch's version.
- After any merge of main into a branch that changes the `Settings` type or the IPC commands that the Settings window calls.

## Examples

Checklist for the branch that merges second:

1. Run `git merge origin/main` and resolve the version conflict. When there is no conflict, look for the collision: compare `CURRENT_SETTINGS_SCHEMA_VERSION` and the last entry in `schema.rs` on both sides.
2. Keep main's migration at its number. Move this branch's migration to the next number and raise the constant.
3. Make the moved migration safe for stores from both builds: guard it on the presence of the field it repairs.
4. Replace literal version numbers in tests with `CURRENT_SETTINGS_SCHEMA_VERSION`.
5. Add a test for a store at the old shared version, written by each build.
6. Run `cargo test --workspace`, `bun run build` and `bun run test`.
