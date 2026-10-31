---
title: Processing checkpoints as files in the Working location
description: "Move the recognition checkpoints of a note that records or processes from the database to files in the Working location, so the database is only an index"
area: processing
added: 2026-10-31
source: "the user, during the plan of docs/plans/2026-10-30-1438-feat-remove-old-transcripts-plan.md"
related: [remove-old-transcripts]
---

# Processing checkpoints as files in the Working location

## Idea
While a note records or processes, its recognition checkpoints (Transcribe window rows, live parts, Diarize, Assemble, `recognition_output`) live in the database. Move them to files in the note's Working location, and keep in the database only an index of them, such as the queue.

## Why
Files are the source of truth, and the database is an index for bulk reads. The transcript removal plan clears these rows when processing ends; this idea removes them from the database during processing too.

## Notes / Current state
- A review on 2026-10-31 found the database convenient, not needed, for these rows. Live parts during a recording have the strongest reason to stay: the live worker writes them while the UI reads them, and WAL gives safe concurrent reads and crash safety. Files need an append log and reader coordination.
- `recognition_output` uses atomic batch commits with revision and coverage, and an index swap in one transaction (`crates/session-core/src/recognition.rs`). A per-generation append file with an atomic index rename can do the same.
- The walking-skeleton plan (KTD10) chose SQLite checkpoints over a job-queue framework; no file-based design was compared.
