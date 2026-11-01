---
title: A Code Quality finding in generated bindings needs a change to the generated text, not a CodeQL path exclusion
date: 2026-11-01
category: tooling-decisions
module: TypeScript bindings export (src-tauri/src/lib.rs export_bindings, src/bindings.ts)
problem_type: tooling_decision
component: development_workflow
severity: low
applies_when:
  - GitHub Code Quality reports a finding in a generated file, for example src/bindings.ts
  - CodeQL code scanning uses default setup
symptoms:
  - "Code Quality reported `js/unused-local-variable` on the import `Channel as TAURI_CHANNEL` in src/bindings.ts"
root_cause: missing_tooling
resolution_type: code_fix
retire_when: "tauri-specta writes the channel import only when a command has a channel argument, or GitHub Code Quality gets a path or generated-file exclusion; check the crate's src/lang/globals.ts template and the Code Quality settings"
tags:
  - code-quality
  - codeql
  - generated-code
  - tauri-specta
  - bindings
---

# A Code Quality finding in generated bindings needs a change to the generated text, not a CodeQL path exclusion

## Context

GitHub Code Quality reported an unused import in `src/bindings.ts`. tauri-specta 2.0.0-rc.21 writes this file. Its fixed template (`src/lang/globals.ts` in the crate) always writes `Channel as TAURI_CHANNEL`, also when no command has a channel argument. The crate has no option that removes the import.

Code scanning in this repository uses CodeQL default setup. Code Quality is a separate run (actor `github-code-quality`). It has no setting that excludes a path or generated files. Thus these changes do not remove the finding:

- A CodeQL configuration file with `paths-ignore`, also after a switch to advanced setup.
- A dismissal on GitHub. The next generated file gives the same finding again.

## Guidance

Change the generated text in the shared export step. `export_bindings` in `src-tauri/src/lib.rs` writes the file, then removes the line `\tChannel as TAURI_CHANNEL,` only when the text `TAURI_CHANNEL` occurs nowhere else (PR #91):

```rust
fn without_unused_channel_import(bindings: &str) -> String {
    if bindings.contains(CHANNEL_IMPORT) && bindings.matches("TAURI_CHANNEL").count() == 1 {
        bindings.replacen(CHANNEL_IMPORT, "", 1)
    } else {
        bindings.to_string()
    }
}
```

Put the change in the function that both `cargo test -p longhand export_bindings` and a debug run of the app call. Then the two write the same file, and the CI step "Bindings are current" (`git diff --exit-code src/bindings.ts`) stays green.

## Why This Matters

`src/bindings.ts` starts with `// @ts-nocheck`. If a later command gets a channel argument and the import is removed by mistake, `npm run typecheck` does not find the missing import. Only unit tests on the pure text function find it. The tests in `lib.rs` cover three cases: the import is removed, the import is kept when it is used, and text without the import does not change.

## When to Apply

- Code Quality reports a finding in a file that a tool writes.
- First look for a generator option. If there is none, change the text in the export step and test the function on text.

## Related

- `docs/solutions/test-failures/codeql-cleartext-logging-from-a-test-panic.md` (a CodeQL code-scanning alert, which is a different run)
