---
title: What the full licence list's tools cannot see, and the checks that cover each gap
date: 2026-09-26
category: workflow-issues
module: Full licence list generator (scripts/licences/generate.ts, scripts/licences/group.ts, licences/manifest.json, scripts/check-licences.sh)
problem_type: workflow_issue
component: development_workflow
related_components:
  - tooling
  - frontend
severity: high
applies_when:
  - "A dependency, a bundled model or any other file that ships in the app is added, removed or updated"
  - "A crate in the Longhand workspace starts to compile in a file with include_bytes!"
  - "scripts/check-licences.sh passes and you want to know what it did not check"
  - "cargo-about or rollup-plugin-license is upgraded"
symptoms:
  - "The DTLN-aec models (crates/aec, MIT) shipped in the app with no entry in the full licence list, and the drift check passed"
  - "The @tauri-apps/plugin-opener entry showed SPDX metadata in place of its MIT and Apache-2.0 texts"
  - "A changed clarify checksum in about.toml gave only a coloured [WARN] line, and cargo-about exited 0"
root_cause: missing_tooling
resolution_type: tooling_addition
tags:
  - licences
  - cargo-about
  - rollup-plugin-license
  - include-bytes
  - drift-check
  - notices
---

# What the full licence list's tools cannot see, and the checks that cover each gap

## Context

The About pane's full licence list (archived PR 13) is stored per platform in `licences/generated/`. `scripts/check-licences.sh` makes it again from three sources and fails when the result differs from the stored text:

- cargo-about 0.9.2, once for each target, for the Rust crates;
- rollup-plugin-license in `vite.config.ts`, for the JavaScript packages in the bundle;
- the hand-kept `licences/manifest.json`, for everything else.

A drift check only proves that the list matches what the tools see. Four gaps showed up in this work where a tool sees nothing, or sees the wrong thing, and the check still passes. Three have a check now. One does not.

## Guidance

**1. cargo-about reports a changed licence file only as a warning.** A `[[<crate>.clarify]]` entry in `about.toml` records a checksum for each vendored licence file. When the file changes, cargo-about 0.9.2 prints a `[WARN]` line (with colour codes, even when stderr is not a terminal), uses the files it finds itself, and exits 0. The generator strips the colour codes and fails on any `[WARN]` or `[ERROR]` line: `cargoAboutProblems` at `scripts/licences/group.ts:341`, called at `scripts/licences/generate.ts:86`. Tests in `group.test.ts` use the real line format. During the review, a checksum was changed by hand and cargo-about was run: the check caught the warning.

**2. cargo-about does not see Longhand's own crates.** It lists third-party crates only. A file that a workspace crate compiles in with `include_bytes!` is invisible to it. In archived PR 12, `crates/aec/src/onnx/model.rs:3-4` started to compile in the two DTLN-aec models (MIT, Nils L. Westhausen, recorded in `crates/aec/PROVENANCE.md`). The list gained no entry, and nothing failed. The gap was found only by reading the rebase diff. The fix was a hand-kept native entry (`licences/manifest.json:69`) and a curated About row. **No check covers this gap yet.** When a workspace crate adds a bundled model, weights or another vendored file, add a native entry. `PROVENANCE.md` already names the licence, so copy it from there. (`src-tauri/src/tray.rs:17` also uses `include_bytes!`, for Longhand's own tray icon, which needs no entry.)

**3. An npm package can ship only SPDX metadata.** `@tauri-apps/plugin-opener` includes `LICENSE.spdx` and no licence text. rollup-plugin-license passes that file as `licenseText`, so the list showed an SPDX record where the terms belong. `isSpdxMetadata` (`group.ts:285`) finds such a text. `packageLicenceTexts` (`group.ts:295`) swaps in the files that `packageLicences` in the manifest names, and fails when there is no replacement or when its version does not match. For the opener, the replacement files come from the `tauri-plugin-opener` crate of the same version and repository.

**4. A hand-kept version does not follow a lock file update.** The Tailwind base styles entry named 4.3.3 while `package.json` allows `^4.1.16`, so a lock update would leave a stale entry that the drift check makes again unchanged. A native entry with `npmPackage` is now compared with `node_modules/<pkg>/package.json`: `checkNpmVersion` at `group.ts:325`, called from `generate.ts:144`. The ONNX Runtime entry has the same kind of check against ort-sys's `dist.txt` (`checkOnnxRuntimeVersion`, `group.ts:352`).

## Why This Matters

Each gap produces a list that looks complete and passes CI, but omits a notice or prints the wrong terms. Gap 2 is the one most likely to recur. Crates such as `aec`, `audio-system` and `audio-sync` arrive through other branches, and the drift check shows only third-party crates (`cidre`, `wasapi`), never the vendored data those crates compile in.

## When to Apply

- Reviewing any diff that adds a file under a workspace crate's `data/` folder, or a new `include_bytes!`/`include_str!` of third-party material.
- Rebasing a licence-related branch on a main that added crates: read the added `PROVENANCE.md` records, not only the list diff.
- Upgrading cargo-about: confirm that the warning format still matches `cargoAboutProblems`, using the tests in `scripts/licences/group.test.ts`.

## Examples

A bundled model with no native entry passes today:

```
crates/aec/src/onnx/model.rs:3  include_bytes!("../../data/model_128_1.onnx")
licences/manifest.json   (no DTLN-aec entry)
scripts/check-licences.sh       licences ok
```

The native entry that closes it:

```json
{
  "name": "DTLN-aec echo cancellation model",
  "version": "128-unit",
  "part": "native",
  "note": "compiled into the app",
  "platforms": ["macos", "windows"],
  "files": ["licences/sources/DTLN-aec-MIT.txt"],
  "source": "https://github.com/breizhn/DTLN-aec"
}
```

A possible check for gap 2, not built: list the targets of `include_bytes!` in `crates/` and `src-tauri/src/`, and fail when a target outside Longhand's own assets has no native entry.

## Related

- `docs/plans/2026-09-26-1259-feat-about-acknowledgements-and-licences-plan.md` (KTD3 and KTD4)
- `docs/solutions/workflow-issues/parallel-branches-bump-same-schema-version.md`, another gap that only a rebase on main exposes
