---
title: Rules for settings that several computers share through a synced folder
date: 2026-09-29
category: integration-issues
module: "Shared settings (crates/shared-settings/src/file.rs, prompts.rs, view.rs, join.rs; src-tauri/src/managers/shared.rs)"
problem_type: integration_issue
component: service_layer
severity: high
applies_when:
  - "A file in the storage folder is written by more than one computer, and a sync service delivers it"
  - "A newer version of Longhand can add fields, values or a schema version that an older version does not know"
  - "A new automatic step reads or writes the `.longhand` folder"
symptoms:
  - "A read of the shared file that writes back can replace a change that another computer made"
  - "An older version that saves the whole file removes fields that a newer version added"
  - "A sync service makes copies of the file with names such as `settings 2.json`, and a computer uses the wrong one"
resolution_type: design_rule
tags: [sync, storage-folder, shared-settings, conflict-copy, schema-version, prompts, icloud, one-library]
related_components: [library, summarize]
---

# Rules for settings that several computers share through a synced folder

## Problem

Longhand keeps the summary prompts and the chosen configuration in the hidden `.longhand` folder of the storage folder. Each computer with the same storage folder reads and writes these files. A sync service copies the files between computers, but it is slow, can deliver files in any order, and can make conflict copies. A computer can also run an older or a newer version of Longhand.

These rules come from building this feature. Each rule guards against a fault that a simple read-modify-write cycle causes.

## Rules

1. **Never write only because of a read.** A read of the shared files never changes them. Launch, the folder watcher, the 60 s re-read and a view refresh only update this computer's cache (`read_settings` and `SharedSettingsManager::reread` in `crates/shared-settings/src/file.rs` and `src-tauri/src/managers/shared.rs`). A write happens only for a change that the user made on this computer, or for the one fill-in join when the file is missing. `SameFolder` join never merges prompts into a folder that already has settings (`crates/shared-settings/src/join.rs`). This is the rule from [receiving-computer-renamed-synced-folder-back](../logic-errors/receiving-computer-renamed-synced-folder-back.md), applied to settings: a computer that only receives a sync writes nothing.

2. **Patch only this save's fields, and keep everything else.** `save_settings` reads the file again at save time. It applies only the `SharedValue` items of this save, and it keeps every other field, including fields with unknown names and values with unknown types (`Field::Unknown` in `file.rs`). The app must not save its own full copy of the settings, because the copy is out of date and lacks the fields of a newer version. A shared change while the file is read-only returns an error.

3. **Never lower the schema version.** The file saves the higher of its own `schema_version` and the version of this build (`apply_patch` in `file.rs`). An older build that saves the file does not mark it as older. A file with a newer version than this build knows is still read and patched, and the UI shows a notice (`newer_schema`). Unknown values show in the UI and stay in the file.

4. **A file that exists but does not parse is not available. Never overwrite it.** `read_settings` returns `NotAvailable` with a cause (`FolderMissing`, `Unreadable` or `DoesNotParse`) in these cases, and `view.rs` shows the settings as read-only from the cache. A half-synced or damaged file often does not parse for a short time. A write at that time would replace the good content that arrives later. Only a file that is missing (`Missing`) can be filled, and only by the join.

5. **Read only the exact file name `settings.json`.** Sync services make copies with other names: `settings 2.json`, `settings (X's conflicted copy DATE).json`, `settings-HOST.json`. A scan by pattern would make a computer choose a copy that another computer chose differently. The exact name gives every computer the same file. A conflict copy of the settings file has no effect (`SETTINGS_FILE` in `file.rs`).

6. **Find prompt conflict copies by the id in the file, and choose ids only from the shared files.** A prompt is a markdown file in `.longhand/prompts/` with an `id` in its front matter. A conflict copy keeps the id of its original, so two files with one id and different text are a conflict. `scan_prompts` (`prompts.rs`) gives each such copy its own id. It makes that id with a name-based UUID from the id of the original and the file name (`copy_id`), and it makes the id of a file without an id from its file name (`file_name_id`). Every computer that reads the same files then computes the same ids without a write, so "the active prompt" means the same file on each computer. A random id, or an id that a computer saves, would differ between computers. Files with one id and the same text show as one prompt. The list shows conflicted prompts (`PromptScan::conflicted`), and the user resolves them.

7. **Accepted risk: a save made before another computer's change has synced can undo that change.** Computer A saves before the change of computer B arrives. A then writes a whole file that lacks the change of B, and the sync service can make A's file the winner. Rule 2 does not prevent this, because A's copy of the other fields is old. Only a person's edits write this file, so the risk needs edits on two computers within the sync delay. The user chose to accept it over one settings file for each computer (plan R15, AE12 and the Risks table). Do not add a per-computer file or a "not yet synced" wait without a new decision.

## Also

- Reads have no timeout. An online-only iCloud file can block a read. Run every read off the main thread. Do not hold the manager lock during a read (`reread` takes the lock only to start and to finish).
- `SharedSettingsManager::change` and `join_locked` still hold the manager lock during file writes. A slow save can make `snapshot` wait.
- After a change of storage folder, the join uses `ChangeOfFolder`. It can fill the new folder from the cache of the old folder only when the new folder has no settings.

## Prevention

- Each new field in the shared file needs a `Field` value, a `KEY_` name equal to the local key, and a `SharedValue` variant. A test must show that a save of another field keeps it.
- Each new automatic reader of `.longhand` needs a test with a file that does not parse, a file with a newer `schema_version`, and a conflict copy such as `settings 2.json`. The test must assert that nothing is written.
- Test two computers on a real synced folder before a release. The manual test in the plan (Verification Contract) does this on two Macs with one iCloud Drive folder.

## Related Issues

- [receiving-computer-renamed-synced-folder-back](../logic-errors/receiving-computer-renamed-synced-folder-back.md): the same rule for note folders.
- [parallel-branches-bump-same-schema-version](../workflow-issues/parallel-branches-bump-same-schema-version.md): the local settings schema is version 6 in this work.
- Plan: `docs/plans/2026-09-28-2343-feat-one-storage-folder-and-shared-settings-plan.md` (KTD1, KTD2, KTD5, R15, R17, R21).
