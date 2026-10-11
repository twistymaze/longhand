---
title: A golden test file failed on Windows CI because the checkout changed its line ends to CRLF
date: 2026-10-05
category: test-failures
module: Summary prompt golden tests (crates/summarize/tests/background.rs, crates/summarize/tests/golden/, .gitattributes)
problem_type: test_failure
component: testing_framework
severity: medium
symptoms:
  - "Test an_empty_or_blank_background_gives_the_same_bytes_as_before failed only on Windows CI with \"assertion left == right failed\""
  - The same test passed on macOS CI and in local runs
  - scripts/check-windows.sh did not catch the failure, because it lints and runs no tests
root_cause: config_error
resolution_type: test_fix
tags:
  - windows-ci
  - golden-file
  - gitattributes
  - crlf
  - line-endings
  - include-str
---

## Problem

A golden-file test passed on macOS and failed on Windows CI. Git checked out the golden file with CRLF line ends on Windows. The code under test produced LF line ends. The byte-for-byte comparison failed.

## Symptoms

- macOS CI and all local runs passed. Windows CI failed.
- Test `an_empty_or_blank_background_gives_the_same_bytes_as_before` panicked at `crates\summarize\tests\background.rs:96:9`:
  `assertion left == right failed: ""`
  The message text is the Debug form of the empty Background input (`{background:?}`). It does not show the byte difference.
- The test reads `crates/summarize/tests/golden/no_background.txt` with `include_str!` (`crates/summarize/tests/background.rs:94`). It then compares `render_all(background)` with that text (line 96).
- Archived PR 38 introduced the file and the test.

## What Didn't Work

- Local runs on macOS. Git writes LF on macOS, so the file and the output matched.
- `scripts/check-windows.sh`. It checks Windows builds from macOS with a clippy cross-check. It runs no tests. A line-end difference in test data cannot show there.
- Result: the failure appeared only in the Windows CI run. This cost one Windows CI cycle of about 17 to 20 minutes.

## Solution

Add a `.gitattributes` rule for golden files (`.gitattributes:5-7`, added in archived PR 38):

```
# Golden test files keep LF line ends, so the tests compare the same bytes
# on Windows as on macOS.
crates/*/tests/golden/** text eol=lf
```

The file already had the same kind of rule for `licences/**` (`.gitattributes:1-3`). The licence drift check, `scripts/check-licences.sh`, needs the same bytes on every platform.

Check the rule with:

```
git check-attr eol crates/summarize/tests/golden/no_background.txt
```

Expected output: `eol: lf`. The next Windows CI run passed.

## Why This Works

With `core.autocrlf` set, Git on the Windows runner changes LF to CRLF in text files at checkout. The rule `text eol=lf` overrides that setting for the matched paths. The file on disk keeps LF. `include_str!` then embeds the same bytes that `render_all` produces.

## Prevention

- A test that compares text from a tracked file byte for byte needs one of two things:
  - an `eol=lf` rule for that path in `.gitattributes`, or
  - code that normalizes line ends before the comparison.
  This applies to `include_str!`, `fs::read_to_string`, and Bun file reads.
- Put golden files under `crates/<crate>/tests/golden/`. The existing rule then covers them with no extra change.
- Add the rule in the same commit that adds the file. A local macOS run cannot reproduce the failure, and `scripts/check-windows.sh` cannot catch it.
- Run `git check-attr eol <path>` for each new golden file. The output must be `eol: lf`.
- Files that the code parses are not affected. Examples: `crates/pipeline/tests/fixtures/paragraph-rule.json` (read with `include_str!`) and the SQL migrations in `crates/session-core`. The parsers accept CRLF. Do not add rules for these files without a failing test.

## Related

- [Full licence list blind spots](../workflow-issues/full-licence-list-blind-spots.md): the `licences/** text eol=lf` rule exists for the same reason, a byte comparison in `scripts/check-licences.sh`.
- [Windows Rust selects the Git linker after MSVC setup](../build-errors/windows-rust-selects-git-linker-after-msvc-setup.md): another failure that only Windows CI shows.
