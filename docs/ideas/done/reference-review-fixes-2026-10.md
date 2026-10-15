---
title: Reference review fixes, October 2026
description: "Four fixes from the 2026-10-15 reference review: microphone start limit and Windows retry by ID, editor changed ranges, prosemirror-view 1.42.3, voice detector pre-roll"
area: capture
added: 2026-10-15
source: "reference source review, 2026-10-15"
related: []
plan: docs/plans/2026-10-15-1157-fix-reference-review-fixes-plan.md
---

# Reference review fixes, October 2026

## Idea
Port four upstream fixes for defects that Longhand also has. The plan gives the requirements.

## Why
The reference source review of 2026-10-15 found them. The user chose all four.

## Shipped

Shipped in [#22](https://github.com/twistymaze/longhand/pull/22) on 2026-10-15, as release v1.1.7. The PR also fixed a timing race in the runner test `a_title_job_does_not_wait_for_the_audio_stages_of_another_note`.
