---
title: Update rusqlite_migration past 2.5.0
description: "Remove the 2.5.0 pin of rusqlite_migration now that the toolchain is Rust 1.95"
area: distribution
added: 2026-10-18
source: "docs/plans/2026-10-18-0955-feat-rig-engine-and-provider-presets-plan.md, Deferred to Follow-Up Work"
related: [ai-provider-presets]
---

# Update rusqlite_migration past 2.5.0

## Idea
`crates/session-core/Cargo.toml` pins `rusqlite_migration = "=2.5.0"`, because 2.6 needs Rust 1.95. The Rig engine plan moves the toolchain to 1.95, so the pin can go.

## Why
Stay on supported versions of the migration crate.

## Notes / Current state
Check the changelog for changes to migration behaviour, and run the session-core migration tests.

## Outcome
Shipped in twistymaze/longhand#50: rusqlite_migration 2.6.0, together with rusqlite 0.40.2. The exact pin stays on purpose, because each rusqlite_migration release accepts one rusqlite minor version. The comment in `crates/session-core/Cargo.toml` gives this reason.
