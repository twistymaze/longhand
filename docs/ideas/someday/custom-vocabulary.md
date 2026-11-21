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
- Added 2026-11-15 (upkeep review, Handy vocabulary through transcribe-cpp): Handy passes the user's word list through the transcribe-cpp vocabulary API. A later fix (1e10e71, #2506) drops a transcript that is only the word list read back: Qwen3-ASR takes the words as a prompt and repeats them on a recording with no speech. Add the same guard for silent windows.
- The Background (Settings, Summaries) already lists names, terms and common misspellings for each summary and title call. A design can take the names and terms from the Background, or keep one list for both, so that the user does not keep two lists.
- transcribe-cpp 0.4.2 removes the glossary from the custom-word prompt. Check which catalog models and the NeMo helper accept a vocabulary before a plan.
- Added 2026-11-21 (upkeep review, transcribe.cpp 35d86179, #221, on main and not in a release after 0.4.2): the Parakeet TDT and RNN-T greedy decoders use `run_params.vocabulary`. The terms boost tokens: the model still decides between blank and a token, and the vocabulary changes only which token it writes. With no terms, the output does not change. The default bonus is 8.0 (`TRANSCRIBE_VOCABULARY_BOOST` changes it). Thus the two offline Parakeet models (parakeet-unified-en-0.6b and parakeet-tdt-0.6b-v3) can take the vocabulary, not only Whisper. Streaming and CTC heads ignore the terms, so the live previews do not use them. Plan this for the first transcribe-cpp release that contains the commit.
