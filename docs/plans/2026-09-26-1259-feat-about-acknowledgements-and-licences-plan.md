---
title: About Pane with Acknowledgements and Licences - Plan
type: feat
date: 2026-09-26
topic: about-acknowledgements-and-licences
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# About Pane with Acknowledgements and Licences - Plan

## Goal Capsule

- **Objective:** A person who opens Longhand's settings can see which projects Longhand is built on, read a personal thank-you to anarlog and Handy, and read the licence of every component that Longhand ships.
- **Product authority:** This Product Contract. The personal texts in R3 and R4 are the author's words and change only with the author's approval.
- **Open blockers:** None.

---

## Product Contract

### Summary

The Licences pane becomes an About pane with three layers. Acknowledgements thanks the projects that Longhand is built on, with a personal entry each for anarlog and Handy. Licences lists the models and a small set of major components. A last row opens a separate window that shows the full licence text of every component the app ships, as plain scrolling text.

### Problem Frame

The Licences pane lists only the downloaded models. Longhand also ships code that it copied from anarlog and Handy, two inference engines, an app framework, an editor and hundreds of smaller crates and packages. MIT, BSD and Apache licences ask that the copyright and permission text go with the app, so a list of models alone does not meet that duty. The pane also gives no credit to the two projects that Longhand grew from.

Neither reference app has a full licence screen. anarlog's `LICENSING.md` says to generate a distribution's third-party notices from the provenance records. Handy's About pane has an Acknowledgments group with one entry, which thanks transcribe.cpp, ggml and Georgi Gerganov.

### Key Decisions

- **Three layers, not one list.** Thanks, a curated licence list and the full text each have a different reader, and one long list would hide all three. Governs R1, R5, R9. (session-settled: user-directed — chosen over a curated list with a generated notice file: the user wanted the full list inside the app, not in a file.)
- **The full list is plain scrolling text in its own window.** A formatted list of hundreds of rows would cost effort and help nobody. Governs R9, R10. (session-settled: user-directed — chosen over a formatted row per component in the pane.)
- **anarlog and Handy get personal entries in the author's voice.** Longhand exists because of these two projects, and this credit is a courtesy, not a legal duty. Governs R3, R4. (session-settled: user-directed — chosen over a single technical thanks line for each.)
- **Technical acknowledgements go only to components that are large, complex and central to what Longhand does.** Common building blocks such as React, ProseMirror, cpal and Opus appear in the curated licence list but not in the thanks. Governs R5, R7. (session-settled: user-approved — the user asked for this judgment and accepted the proposed set.)
- **The pane is renamed About.** "Licences" does not describe a pane that also holds thanks, and Handy uses About for the same content. Governs R1. (session-settled: user-approved — proposed as a call-out and confirmed.)
- **The full list comes from the build, not from a hand-kept file.** Nobody can keep hundreds of entries current by hand. Governs R11, R12.

### Requirements

**Pane**

- R1. The settings pane named "Licences" is renamed "About". It shows, in this order: Acknowledgements, Licences, and a row that opens the full list.
- R2. The pane text follows the UI language rules: short, plain words, and the term "note", not "session", when it speaks about recordings.

**Acknowledgements**

- R3. An anarlog entry comes first and carries this text, with only small changes to wording: "anarlog inspired me to make Longhand. The work its team is doing is outstanding. I wanted something smaller and simpler for my own use, and I started from what they built." A short clause then says which parts of Longhand come from anarlog.
- R4. A Handy entry comes next and carries this text, with only small changes to wording: "Thank you for an amazing local-only dictation tool that I still use every day." A short clause then says which parts of Longhand come from Handy.
- R5. A separate, shorter group of technical thanks follows the personal entries. It names whisper.cpp, ggml and transcribe.cpp (Georgi Gerganov and contributors), ONNX Runtime, pyannote and Tauri, with one line each on what it does for Longhand.
- R6. Each acknowledgement names its project and is a link that opens the project's page in the default browser.

**Curated licences**

- R7. The Licences group lists every model, as it does now, plus one row for each major component: ggml, transcribe.cpp, ONNX Runtime, Tauri, ProseMirror, React, cpal, Opus, the code copied from anarlog, and the code copied from Handy. whisper.cpp gets no row, because Longhand does not contain its code: transcribe.cpp only reads whisper.cpp's model file format.
- R8. A component row behaves as a model row does: a click shows its licence name, attribution, licence address and source.

**Full list**

- R9. The last row of the pane opens a separate window that shows the licence of every component the app ships.
- R10. The window shows plain scrolling text. Each distinct licence body appears once, followed by the components that use it, each with its own copyright line. The text can be selected and copied.
- R11. For each platform that Longhand ships (macOS and Windows), the full list covers every Rust crate linked into that platform's binary, every JavaScript package bundled into the interface, and the code copied from anarlog and Handy. It also covers the native code that crates bundle or link, with its own licence and notice texts: ggml and miniz inside transcribe-cpp-sys, kaldi-native-fbank inside knf-rs-sys, and the ONNX Runtime library that ort-sys links, with its third-party notices. It leaves out tools used only to build or test the app. Each build shows the list for its own platform.
- R12. A check that runs with the local gates and in CI fails when the full list for any shipped platform does not match the components that platform ships, including the native code in R11.

### Acceptance Examples

- AE1. **Covers R12.** **Given** a change that adds a new crate to the binary, **when** the local gates run, **then** the check fails until the full list includes that crate and its licence text.
- AE2. **Covers R11.** **Given** a crate used only by a test or a build script, **when** the full list is produced, **then** that crate does not appear in it.
- AE3. **Covers R10.** **Given** 40 crates under the same MIT text, **when** the window opens, **then** that MIT text appears once, with the 40 crate names listed with it, each with its own copyright line.

### Scope Boundaries

- No change to how model rows work or to the model data behind them.
- No app version display in the About pane. It is a natural later addition, but this work does not need it.
- No licences for tools that are used only to build, test or develop the app.

### Outstanding Questions

**Deferred to Planning**

- The clauses in R3 and R4 that say what came from each project must match the `PROVENANCE.md` records. The records show that anarlog supplied recording, audio, diarization, embedding and the editor, and that Handy supplied the model catalog and downloads, speech recognition, voice detection and parts of the pipeline.
- Which tool produces the full list for Rust and for JavaScript, and in what form the app carries it.
- How the native licence and notice texts in R11 are collected. Crate-level licence tools read only each crate's top-level licence, so they miss licence files deeper in a crate and the ONNX Runtime download.
- Whether R12's check uses the same tool as R11 or compares against the existing `cargo deny` licence data.
- Whether the app carries a stored list that R12 compares with freshly generated output, or a list made during each build. With a list made during each build, the check compares the list with itself, and AE1 cannot fail.
- How the JavaScript list is taken from the bundled interface output. `package.json` lists some build tools, such as Tailwind, as app dependencies, and AE2 says build tools are left out.

### Sources / Research

- `src/settings/Licences.tsx` and `src/settings/SettingsWindow.tsx`: the current pane and its place in the sidebar.
- `deny.toml`: the licence allow-list and the MPL-2.0 exceptions for Tauri's crates, which the full list must include.
- `LICENSES/anarlog-MIT.txt`, `LICENSES/Handy-MIT.txt` and the `PROVENANCE.md` files beside the copied code.
- `reference-source/anarlog/LICENSING.md`, "Third-party provenance": notices are generated from the provenance records.
- `reference-source/Handy/src/components/settings/about/AboutSettings.tsx` and the `settings.about.acknowledgments` strings: Handy's acknowledgement group.
