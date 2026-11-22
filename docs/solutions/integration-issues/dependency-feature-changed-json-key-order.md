---
title: A dependency feature changed the key order of JSON that Longhand writes
date: 2026-10-18
last_updated: 2026-11-21
category: integration-issues
module: Shared settings file (crates/shared-settings/src/file.rs), summary requests (crates/summarize)
problem_type: integration_issue
component: service_layer
severity: low
symptoms:
  - test_storage_folder failed with "testdata/storage-folder/.longhand/settings.json differs from what the writers make"
  - The regenerated settings.json had the same keys and values in a different order
  - "After the update to rig-core 0.47, all tests stayed green, but each serde_json::Map would serialize in sorted key order"
root_cause: feature_unification
resolution_type: code_fix
tags:
  - serde_json
  - preserve_order
  - cargo-features
  - shared-settings
  - rig-core
---

# A dependency feature changed the key order of JSON that Longhand writes

## Problem

The `preserve_order` feature of `serde_json` decides the key order of each `serde_json::Map`. With the feature, a map keeps the order of insertion. Without it, a map keeps its keys in sorted order. Cargo joins the features of a crate for the whole build, so one dependency can turn the feature on for all of Longhand.

This occurred two times:

1. **A dependency turned the feature on (PR #39).** `rig-core` 0.44 enabled `preserve_order`. After `rig-core` was added, `test_storage_folder` failed: the shared `settings.json` had the same keys and values, but the keys were not in sorted order.
2. **A dependency turned the feature off (PR #107).** `rig-core` 0.47 no longer enables `preserve_order`. Without a fix, the provider request bodies and the files that `local_store` writes would change to sorted key order. No test failed, because no test compared those bytes.

## Cause

Longhand used a behaviour that came from a feature of another crate, and did not declare that feature itself. A change in key order can make a sync service copy a file again, and two versions of Longhand can change the order back and forth.

## Fix

- `settings_bytes` in `crates/shared-settings/src/file.rs` sorts the keys at each level before it serializes the map. The settings file gives the same bytes with or without the feature.
- The workspace `Cargo.toml` now enables `preserve_order` for `serde_json` itself (PR #107). The key order no longer depends on a dependency.

## Prevention

- When you add, update or remove a dependency, run `cargo tree -e features -i serde_json` before and after, and compare the `preserve_order` lines.
- Declare each feature that Longhand depends on in its own `Cargo.toml`, also when a dependency already enables it.
- Do not let the bytes of a shared file depend on the order of a `serde_json::Map`. Serialize a struct, or sort the keys.
- These tests compare bytes and fail when the key order changes:
  - `test_storage_folder` (the shared settings file).
  - `crates/summarize/tests/request_bodies.rs` (each preset's request body against fixtures captured on `rig-core` 0.44).
  - `a_write_keeps_the_keys_in_the_order_of_the_map` in `src-tauri/src/managers/local_store.rs`.
- On a dependency update, read the release notes and the changelog, not only the API diff. The `rig-core` 0.47 update also changed which Gemini header holds the key, how an error code in a 2xx reply is read, and the catalog output limits.
