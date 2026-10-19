# Reference source reviews

The weekly upkeep routine reviews the upstream projects of Longhand. It keeps the review baselines in the maintainer's private upkeep repository, and writes its findings in the upkeep tracking issue (label `upkeep`). See `docs/development-workflow.md`, "Dependency and upstream upkeep".

A baseline is the last reviewed commit for a repository, or the last reviewed release version for a crate.

Handy keeps its recorder and resampler in the crates `handy-recorder` and `handy-cpal` (since a42131d). Longhand copied files from both crates. A change to them does not show in Handy's history, so a review reads the source of the crate releases after the baseline and compares it with the files that Longhand copied. `PROVENANCE.md` lists those files.

## Manual procedure

Use this procedure when the maintainer starts a review by hand. It starts from the baselines in the private upkeep repository, and it uses the local clones in `reference-source/`. Do not run it at the same time as the routine.

1. Read the baseline of each project in the private upkeep repository. Fetch each clone. Do not pull yet. The working tree must stay at the reviewed commit until the review is complete.

   ```sh
   for d in anarlog Handy shorthand; do
     git -C reference-source/$d fetch origin
     git -C reference-source/$d log --oneline HEAD..origin/main | wc -l
   done
   ```

2. Review the commits after the baseline with `git log` and `git diff <baseline>..origin/main` only. Sort each change into one of these groups:
   - **Act now**: a fix to code that `PROVENANCE.md` records as borrowed, or a defect that Longhand also has.
   - **Idea**: a change worth a feature idea. Write it with the procedure in `docs/ideas/README.md`.
   - **Note only**: a verified fact worth keeping. Add it to `docs/reference/borrowing-notes.md`.
   - **Not relevant**: web, billing, marketing, cloud-only features, and similar changes.
3. For NeMo-Speech.cpp, transcribe-cpp, `handy-recorder` and `handy-cpal`, clone the repository in a scratch folder outside the checkout and read the changes after the baseline.
4. Fast-forward each local clone to the reviewed commit: `git -C reference-source/<clone> merge --ff-only <commit>`.
5. Write the new baselines and the review date in the private upkeep repository, and commit that change there.

## Log

The log ends on 2026-10-15. Later reviews write their open findings in the upkeep tracking issue, and record their baselines in the private upkeep repository.

### 2026-10-15

Ranges: anarlog `b02b58de..afb35650` (217 commits, about 20 relevant), Handy `29bd2c0..9e57a0d` (28 commits, release v0.9.8), and shorthand `90b8d40..567640c` (59 commits, releases 0.6.1 to 0.7.0, with 17 commits from Handy).

**Act now** (plan: `docs/plans/2026-10-15-1157-fix-reference-review-fixes-plan.md`)
- anarlog `69350022`, `9a9ed32f`: the microphone start waits with no time limit while the capture manager holds its lock (`crates/audio-cpal/src/lib.rs`). Add a limit and a guard against a second start.
- Shorthand `19029f4`, `22ef0ed`: on Windows with cpal 0.18.2, a failed open of the default microphone is retried by device ID.
- anarlog `a84a8a5d`: `getChangedRanges` maps each range to the final document. Longhand's copy in `src/editor/plugins/changed-ranges.ts` is the old code.
- anarlog `a112ca8a`: prosemirror-view 1.41.7 to 1.42.3 (Dependabot security group).
- Handy `7b2a28a`: the `SmoothedVad` pre-roll sent frames twice. Latent in Longhand: no production code calls it.

**Ideas** (files in `docs/ideas/someday/`)
- New: `transcribe-cpp-update`, `handy-recorder-crates`, `custom-vocabulary`, `recognition-in-worker-process`, `summary-endpoint-anthropic-format` (Handy); `summary-language`, `keep-chosen-tab-after-processing`, `buffer-audio-during-storage-errors`, `summary-error-source` (anarlog).
- Changed: `prewarm-gpu-shaders`, `webview-crash-recovery`, `skip-summary-for-short-transcript`.

**Note only:** the facts are in `docs/reference/borrowing-notes.md`, section "Findings from the 2026-10-15 review".

**Not relevant:** about 197 anarlog commits (web, billing, cloud sync, mobile, calendar, chat, cloud speech-to-text, releases), 16 Handy commits, and 25 shorthand commits (dictation hold mode, telemetry, branding, docs, releases).

### 2026-09-29

Ranges: anarlog `39613e5..b02b58de` (108 commits, about 44 relevant), Handy `8f9cf53..29bd2c0` (6 commits), and a first full review of shorthand at `90b8d40`. Shorthand last merged Handy at `05e0aed` (release 0.9.7).

**Act now** (done in archived PR 25. The 0.2.4 runtime uses 30-39 MB more peak memory on the same audio. This cost is accepted.)
- Handy `2d526b1`: transcribe-cpp 0.2.3 to 0.2.4. Parakeet uses less memory on long audio (#161), and Whisper failures report the real error code (#171). The decode-budget change (#165) does not affect Longhand's catalog models. Update `TRANSCRIBE_REVISION` in `crates/models/src/recognition.rs` with the bump.
- Shorthand `a5da3f0`: on cpal 0.18, `Device::to_string()` panics when `description()` fails. Longhand calls it in `crates/audio-cpal/src/config.rs:19`. Use `device.description()` with a fallback.
- anarlog `7b57ea48`, `5338e3c7`: hide the format toolbar during IME composition. Longhand's borrowed `src/editor/widgets/format-toolbar.tsx` does not have the fix. Update `src/editor/PROVENANCE.md` after the port.

**Ideas** (files in `docs/ideas/someday/`)
- `live-speaker-labels`: Shorthand's live labels come from the capture lane ("Me" and "Them"), not from voices. Longhand can show live labels on the recommended route if it joins the live text to the live speaker turns.
- `webview-crash-recovery` (anarlog), `skip-summary-for-short-transcript` (anarlog), `prewarm-gpu-shaders` (Handy `29bd2c0`), `header-inset-under-zoom` (anarlog).
- `system-audio-capture` (parked): Shorthand's separate lanes support per-channel processing. Its capture method is weaker than Longhand's parked capture.

**Note only**
- Shorthand `a5da3f0`: cpal 0.18 changed how default formats rank. Longhand's `preferred_config` can select a format that it cannot decode, and the open then fails cleanly.
- anarlog `5a32275b`, `5cae98b9`: anarlog added, then removed, a pause of other apps' media at recording start. macOS has no public API for it.
- anarlog `d9a37afa`, `89362b67`, `77c1eaf6`: playback streams and waveform peaks come from native code, because WebKit used too much memory to decode a full recording. Use this design if Longhand adds playback.
- anarlog `4feda5cb`: Windows Credential Manager holds at most 2560 bytes for each secret.
- The changes to borrowed `recorder/chunks.rs`, `stream.rs`, `children.rs`, and `note/keymap.ts` do not apply to Longhand.
