---
title: Custom vocabulary
description: "The user keeps a list of names and special words, and transcription spells them correctly"
area: processing
added: 2026-10-15
source: "Handy review, 2026-10-15: commit 4150a4a; walking-skeleton plan, deferred scope"
related: [transcribe-cpp-update]
---

# Custom vocabulary

## Idea
The user keeps a list of names and special words. Longhand gives the list to the recognition model through transcribe-cpp's `RunOptions::vocabulary` when the model supports it (`Feature::Vocabulary`). For other models, it corrects close matches in the text after recognition, as Handy's `text.rs` does with soundex and n-gram keys.

## Why
Names of people, products and vendors are the words that a summary needs most, and recognition models often spell them wrong. The walking-skeleton plan left custom vocabulary out on purpose, "so its absence can be felt before it is specified".

## Notes / Current state
- Needs transcribe-cpp 0.4 (`transcribe-cpp-update`).
- Of Longhand's catalog models, probably only Whisper takes a vocabulary (inferred, not checked).
- Tags of the Person and Vendor scopes could seed the list.
