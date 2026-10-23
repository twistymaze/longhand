---
title: CodeQL failed the PR because a test panic printed a parse error that holds the session id
date: 2026-10-23
category: test-failures
module: session-core tests (crates/session-core/tests/note_record.rs, crates/session-core/tests/note_folder.rs)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "The CodeQL check failed on PR #61 with a high alert of the rule `rust/cleartext-logging` (Cleartext logging of sensitive information)"
  - "The alert text: \"This operation writes check_session_id(...) to a log file\", on a `panic!(\"{audio:?} gave {other:?}\")` line in a test"
  - All Rust tests, clippy and fmt passed locally and in the other CI jobs
root_cause: wrong_api
resolution_type: test_fix
tags:
  - codeql
  - code-scanning
  - cleartext-logging
  - session-id
  - panic
  - tests
---

## Problem

CodeQL treats a `session_id` value as sensitive data. A new test printed a `FolderError` with `{other:?}` in a `panic!`. That error can hold the session id, so CodeQL reported cleartext logging. The alert has high severity, so the CodeQL check failed and the PR could not merge.

## Symptoms

- On PR #61, the CodeQL check failed. The `github-advanced-security` bot opened a review thread on `crates/session-core/tests/note_record.rs`.
- The test passed. Nothing logs data in production. The flagged line was only the failure branch of a `match` in a test.
- Older tests in the same crate also print errors with `{other:?}` and are not flagged. CodeQL flags only a value that it can trace from `check_session_id` to the print.

## What Didn't Work

- Nothing local finds this. `cargo test`, `cargo clippy -D warnings` and `scripts/check-windows.sh` all passed. Only the CodeQL analysis in CI ran the rule, about 13 minutes after the push.

## Solution

Check the error kind with `matches!`, and print only the test input:

```rust
// Before: the panic prints the whole error, which can hold the session id.
match NoteRecord::parse(&text) {
    Err(FolderError::Yaml { file, .. }) => assert_eq!(file, NOTE_RECORD_FILE),
    other => panic!("{audio:?} gave {other:?}"),
}

// After: the assert does not print the result.
assert!(
    matches!(
        NoteRecord::parse(&text),
        Err(FolderError::Yaml { file, .. }) if file == NOTE_RECORD_FILE
    ),
    "{audio:?}"
);
```

The PR changed the same pattern in `crates/session-core/tests/note_folder.rs` at the same time, before CodeQL flagged it. The next CodeQL run passed.

## Why This Works

`check_session_id` (`crates/session-core/src/note_record.rs:216`) writes the id into the error text: `format!("session_id {id:?} is not a valid note id")`. CodeQL follows that value into each formatting call. A `panic!` message counts as a log write. When the test prints only its own input, no traced value reaches the print, and the rule has nothing to report.

## Prevention

- In a test that can get a `FolderError` from note parsing (`NoteRecord::parse`, `NoteFiles::parse`, `note_id_of`), do not print the error with `{:?}` or `{}`. Use `assert!(matches!(..), "<input>")`.
- If a test needs the error text for debugging, print the variant name or the `file` field, not the `message`.
- The same applies to any new error that carries a value with a name like `session`, `token`, `password` or `secret`. In this case CodeQL treated a note id as sensitive, which suggests that it uses the name of the value, not whether the value is a real secret.

## Update: the rename removed the cause

On 2026-10-23 the note id rename (`docs/plans/2026-10-23-1220-refactor-note-id-and-note-terminology-plan.md`) changed each code name of the id from `session_id` to `note_id`. CodeQL's shared name rule (`SensitiveDataHeuristics.qll`, `maybeAccountInfo`) matches `session.?(id|key)` and names that contain `uid`, but not `note_id`. So a log line or a test message with the note id no longer gives an alert. Write `note_id`, not `session_id`, in new code (CONCEPTS.md, Note id).

The test rule above still applies to other values whose names the rule matches, for example `secret`, `token`, `password`, `user_id` or `account`.
