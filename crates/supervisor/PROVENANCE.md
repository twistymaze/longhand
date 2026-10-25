# Provenance

Borrowed material in this crate, one record per file (R27). `supervisor.rs` and `dynamic.rs` (the `ractor` actor tree) are not taken (KTD5).

## `src/restart.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 693500220d8e253b175e7a5b88fd831c48b4583b
- Source paths: `crates/supervisor/src/restart.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — `new_at`, `record_restart_at` and `maybe_reset_at` added so budgets are testable at their real durations; upstream methods unchanged; keeps the four tests that upstream removed in 8860cd65 (`new_tracker_starts_at_zero`, `record_restart_increments_within_budget`, `record_restart_at_exact_boundary`, `maybe_reset_noop_when_none`)

## `src/retry.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/supervisor/src/retry.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — the backoff loop is generic over any fallible future instead of spawning a `ractor` actor; tests rewritten without actors

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: `crates/listener-core/src/actors/session/supervisor/children.rs` (budget constants, `source_restart_delay`, its two tests)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: modified — constants and delay function moved here and made public

