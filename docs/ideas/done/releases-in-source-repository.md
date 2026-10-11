---
title: Releases in the source repository
description: "Publish a numbered GitHub Release in twistymaze/longhand for each push to main that changes the app, in place of preview builds in longhand-builds"
area: distribution
added: 2026-10-11
source: "conversation after the repository became public"
related: [preview-builds-and-updates, windows-preview-builds-and-updates, sccache-for-hosted-builds]
plan: docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md
---

# Releases in the source repository

## Idea

Drop the separate `twistymaze/longhand-builds` repository. Each push to `main` that changes the shipped app publishes a numbered release on the Releases page of `twistymaze/longhand`. git-cliff chooses the version from the Conventional Commit types since the latest release tag, starting at 1.0.0. The Windows installer is attached to each release.

## Why

The repository became public, so the separate builds repository was no longer needed. The preview stream had become the release stream.

## Notes / Current state

Shipped in PR twistymaze/longhand#2. `docs/releases.md` describes the flow, the version rules, the one-time setup, and the failure cases.
