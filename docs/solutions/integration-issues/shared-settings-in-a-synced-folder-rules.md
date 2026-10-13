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

1. **Never write only because of a read.** A read of the shared files never changes them. Launch, the folder watcher, the 60 s re-read and a view refresh only update this computer's cache (`read_settings` and `SharedSettingsManager::reread` in `crates/shared-settings/src/file.rs` and `src-tauri/src/managers/shared.rs`). A write happens only for a change that the user made on this computer, or for the join. The join fills a missing file, or adds this computer's prompts that the folder does not have (R22). When the join could not read the folder, the first read that finds the folder runs the join (the cache then did not come from this folder). A read with a cache from this folder only reads. `SameFolder` join never merges prompts into a folder that already has settings (`crates/shared-settings/src/join.rs`). This is the rule from [receiving-computer-renamed-synced-folder-back](../logic-errors/receiving-computer-renamed-synced-folder-back.md), applied to settings: a computer that only receives a sync writes nothing.

2. **Patch only this save's fields, and keep everything else.** `save_settings` reads the file again at save time. It applies only the `SharedValue` items of this save, and it keeps every other field, including fields with unknown names and values with unknown types (`Field::Unknown` in `file.rs`). The app must not save its own full copy of the settings, because the copy is out of date and lacks the fields of a newer version. A shared change while the file is read-only returns an error.

3. **Never lower the schema version.** The file saves the higher of its own `schema_version` and the version of this build (`apply_patch` in `file.rs`). An older build that saves the file does not mark it as older. A file with a newer version than this build knows is still read and patched, and the UI shows a notice (`newer_schema`). Unknown values show in the UI and stay in the file.

4. **A file that exists but does not parse is not available. Never overwrite it.** `read_settings` returns `NotAvailable` with a cause (`FolderMissing`, `Unreadable` or `DoesNotParse`) in these cases. A half-synced or damaged file often does not parse for a short time. A write at that time would replace the good content that arrives later. Only a file that is missing (`Missing`) can be filled, and only by the join. When the folder is missing or cannot be read, `view.rs` shows all the shared settings as read-only from the cache. When only `settings.json` does not parse, the settings in it come from the cache and are read-only, but the prompts and the Background come from the folder and can change (`files_writable` in `view.rs` and `SharedSnapshot::files_writable` in `shared.rs`). A change of a prompt or of the Background does not write `settings.json`. A delete of a prompt then changes no setting that names it. When `settings.json` still does not parse after about one minute (`MISSING_PROMPT_WAIT`), it is not a sync in progress: the cause becomes `SettingsFileDamaged`, and Home and Settings > Storage show a notice with the path of the file and the line of the error (`DamagedFile`), because the user must fix the file. Before that, Settings > Storage says only that Longhand uses the last settings it read and that the prompts and the Background can still change.

5. **Read only the exact file name `settings.json`.** Sync services make copies with other names: `settings 2.json`, `settings (X's conflicted copy DATE).json`, `settings-HOST.json`. A scan by pattern would make a computer choose a copy that another computer chose differently. The exact name gives every computer the same file. A conflict copy of the settings file has no effect (`SETTINGS_FILE` in `file.rs`).

6. **Find prompt conflict copies by the id in the file, and choose ids only from the shared files.** A prompt is a markdown file in `.longhand/prompts/` with an `id` in its front matter. A conflict copy keeps the id of its original, so two files with one id and different text are a conflict. `scan_prompts` (`prompts.rs`) gives each such copy its own id. It makes that id with a name-based UUID from the id of the original and the file name (`copy_id`), and it makes the id of a file without an id from its file name (`file_name_id`). Every computer that reads the same files then computes the same ids without a write, so "the active prompt" means the same file on each computer. A random id, or an id that a computer saves, would differ between computers. Files with one id and the same text show as one prompt. The list shows conflicted prompts (`PromptScan::conflicted`), and the user resolves them.

   A person or an agent can add, copy or rename a prompt file. `repair_prompts` (`prompts.rs`) repairs these edits, but only at a write that a user change on this computer causes, never at a read (rule 1). First, a file with no id gets its `file_name_id`, before any rename in the same save, so the settings that name the prompt keep it. Then a file whose name is not the file name of its `name` gets that file name, and its id stays. A prompt with a conflict copy or a duplicate file changes in neither step: the user resolves the conflict.

7. **Accepted risk: a save made before another computer's change has synced can undo that change.** Computer A saves before the change of computer B arrives. A then writes a whole file that lacks the change of B, and the sync service can make A's file the winner. Rule 2 does not prevent this, because A's copy of the other fields is old. Only a person's edits write this file, so the risk needs edits on two computers within the sync delay. The user chose to accept it over one settings file for each computer (plan R15, AE12 and the Risks table). Do not add a per-computer file or a "not yet synced" wait without a new decision.

8. **The last change wins, and conflicts belong to the sync service.** When computers change or copy settings, the last file that the sync service keeps applies to every computer. Longhand does not try to predict what the sync service will do: no "has this synced yet" guesses, no field that one path copies and another path leaves out, and no special handling for one setting. Every shared setting follows the same default and copy rules, including a setting whose value deletes data, such as audio removal. The rules above still apply: a file that cannot be read is not overwritten (rule 4), and only the exact name counts (rule 5). The user set this rule on 2026-09-30, during the plan for removing old audio (`docs/plans/2026-09-30-0357-feat-remove-old-audio-plan.md`).

9. **The cache copy stands in only for a prompt file that exists and cannot be used now. A missing file is missing at once.** A prompt file can exist and still not be usable for a time: a sync delivered only a part of it (a front matter block with no closing line), it cannot be read, it has no prompt text, or it is not UTF-8. The scan puts its id in `PromptScan::unusable` (the id in the file, or the `file_name_id` when the file has no id), so the prompt is not missing. A file whose id the scan cannot read sets `unknown_unusable`, and then no custom prompt is missing. For such a prompt, `resolve` in `view.rs` uses the last copy in the cache. A prompt whose file is not in the folder has no cache stand-in: a summary uses the default prompt at once. The settings that name a missing prompt (the active prompt and the type prompt choices) clear only at a write that a user change on this computer causes, after the prompt has been missing for about one minute (`MISSING_PROMPT_WAIT`), and only for a prompt that this computer read before. The wait keeps the settings while a sync service delivers a rename as a delete and then a create. The clear needs a `settings.json` that parses.

   A prompt over the size limit has two cases. A file over `MAX_FILE_BYTES` is too large for the scan: the scan reads only its front matter, Settings lists it with no text and an over-limit flag, and a summary uses the default prompt. The file exists, so the prompt is not missing and the settings that name it stay. A file under that size whose prompt text or title text is over its character limit, for example after an edit outside Longhand, is listed with an over-limit flag, and a run uses its whole text (`PromptFile::over_limit`).

## Also

- Reads have no timeout. An online-only iCloud file can block a read. Run every read off the main thread. Do not hold the manager lock during a read (`reread` takes the lock only to start and to finish).
- Writes (`SharedSettingsManager::change` and `join`) take the `writer` lock, not the `inner` lock. They lock `inner` only for short steps, so a slow save does not make `snapshot` wait. A slow save can make other shared changes wait.
- After a change of storage folder, the join uses `ChangeOfFolder`. It can fill the new folder from the cache of the old folder only when the new folder has no settings.

## Prevention

- Each new field in the shared file needs a `Field` value, a `KEY_` name equal to the local key, and a `SharedValue` variant. A test must show that a save of another field keeps it.
- Each new automatic reader of `.longhand` needs a test with a file that does not parse, a file with a newer `schema_version`, and a conflict copy such as `settings 2.json`. The test must assert that nothing is written.
- Test two computers on a real synced folder before a release. The manual test in the plan (Verification Contract) does this on two Macs with one iCloud Drive folder.

## Related Issues

- [receiving-computer-renamed-synced-folder-back](../logic-errors/receiving-computer-renamed-synced-folder-back.md): the same rule for note folders.
- [parallel-branches-bump-same-schema-version](../workflow-issues/parallel-branches-bump-same-schema-version.md): the local settings schema is version 6 in this work.
- Plan: `docs/plans/2026-09-28-2343-feat-one-storage-folder-and-shared-settings-plan.md` (KTD1, KTD2, KTD5, R15, R17, R21).
- [app-owned-note-record-file](app-owned-note-record-file.md): the same rules for note folders, and the files that a person or an agent can edit.
- Plan: `docs/plans/2026-10-12-1827-feat-safe-agent-edits-plan.md` (KTD8, R18 to R23): outside edits of the prompts and of `settings.json`.
