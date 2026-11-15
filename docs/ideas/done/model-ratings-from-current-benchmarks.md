---
title: Model ratings from current benchmarks
description: "Make the speed and accuracy ratings in the model picker from Handy's current benchmark results, not the frozen 0-100 scores"
area: interface
added: 2026-11-15
source: "docs/plans/2026-11-15-1436-chore-refresh-catalog-schema-2-plan.md (KTD6)"
related: []
plan: docs/plans/2026-11-15-1436-chore-refresh-catalog-schema-2-plan.md
---

# Model ratings from current benchmarks

## Idea
The model picker shows a speed and an accuracy rating for each recognition model. The values are 0-100 scores that Handy's old generator made, frozen at Handy `8f9cf53`. Handy's catalog schema 2 has no scores. It has FLEURS error percent for each language and real-time factors on CPU and GPU. Longhand could make its ratings from these results, for example the error for the note's language and the CPU speed.

## Why
The frozen scores do not follow new benchmark data, and one accuracy score hides large differences between languages. A change moves the ratings that users see, so it needs a product decision.

## Outcome
Shipped in twistymaze/longhand#94. The ratings use the formulas of Handy's model list on the package's FLEURS English error and GPU real-time factor.
