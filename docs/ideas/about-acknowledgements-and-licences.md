---
title: Acknowledgements and full licences in an About pane
status: in-progress
area: interface
added: 2026-09-26
source: "a ce-brainstorm session, 2026-09-26"
related: [recognition-model-picker]
---

# Acknowledgements and full licences in an About pane

## Idea
The Licences pane becomes About. It thanks anarlog and Handy in the author's own words, thanks the core engines (whisper.cpp, ggml, transcribe.cpp, ONNX Runtime, pyannote, Tauri), lists the models and major components with their licences, and opens a window with the full licence text of every component the app ships.

## Why
The pane lists only the models. The copied code, the engines and hundreds of smaller crates and packages also need their notices, and the two projects that Longhand grew from get no credit.

## Notes / Current state
Requirements: `docs/plans/2026-09-26-1259-feat-about-acknowledgements-and-licences-plan.md`. An app version line in About is a possible later addition.
