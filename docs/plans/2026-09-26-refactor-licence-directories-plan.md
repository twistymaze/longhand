---
title: Consolidate licence source files
date: 2026-09-26
type: refactor
---

# Consolidate licence source files

## Goal

Use one directory for upstream licence texts and notices. Preserve every text and the generated platform lists.

## Agreed scope

- Move the files in `LICENSES/` and the text files in `licences/native/` to `licences/sources/`.
- Move `licences/native/manifest.json` to `licences/manifest.json`.
- Update repository path references in provenance records, scripts, tests and current maintenance guidance. Preserve upstream URLs and historical plans and ideas.
- Add `licences/README.md` with instructions for source files, the manifest and generated files.
- Do not change dependencies, licence coverage, licence text or app behavior.

## Implementation

One mechanical unit: move the files, update consumers and document the layout. Use native inline execution. Commit this plan on main and push it before implementation. Create a managed worktree from origin/main for the implementation.

## Verification

- Compare every moved licence text with its original Git blob.
- Run the provenance check and its self-test.
- Run the licence check and its self-test, including the existing generator tests and frontend build.
- Verify that all files in `licences/generated/`, including the macOS and Windows lists, remain byte-for-byte identical to the base commit.
- Run the required Windows lint check.
- Check that old local paths remain only in historical documents. Preserve remote source URLs.
- Run `git diff --check` and review the final diff.

## Completion

The new layout is documented, all active references resolve, and required local checks pass. Commit the implementation, open a pull request and wait for CI. Do not merge without authorization.
