---
title: A dependency feature changed the key order of the shared settings file
date: 2026-10-18
category: integration-issues
module: Shared settings file (crates/shared-settings/src/file.rs)
problem_type: integration_issue
component: service_layer
severity: low
symptoms:
  - test_storage_folder failed with "testdata/storage-folder/.longhand/settings.json differs from what the writers make"
  - The regenerated settings.json had the same keys and values in a different order
root_cause: feature_unification
resolution_type: code_fix
tags:
  - serde_json
  - preserve_order
  - cargo-features
  - shared-settings
---

# A dependency feature changed the key order of the shared settings file

## Problem

After `rig-core` was added, `test_storage_folder` failed. The shared `settings.json` that the writers made had the same keys and values, but the keys were not in sorted order.

## Cause

`rig-core` enables the `preserve_order` feature of `serde_json`. Cargo joins the features of a crate for the whole build, so each `serde_json::Map` in Longhand keeps the order of insertion and not the sorted order. `write_settings` serialized a `Map`, so the order of the keys in the file came from the order of the merge.

A change in key order makes a sync service copy the file again, and two versions of Longhand can change the order back and forth.

## Fix

`settings_bytes` sorts the keys at each level before it serializes the map (`sorted` in `file.rs`). The same settings always give the same bytes, with or without the feature.

## Prevention

- When you add a dependency, run `cargo tree -e features -i serde_json` and look for `preserve_order`.
- Do not let the bytes of a file that Longhand writes depend on the order of a `serde_json::Map`. Serialize a struct, or sort the keys.
- `test_storage_folder` compares the checked-in folder with the bytes that the writers make, and it found this change.
