# Reference source reviews

This file records the upstream commit up to which each clone in `reference-source/` was reviewed. Review each clone again about every two weeks.

## Review status

| Clone | Upstream | Reviewed through | Review date |
| --- | --- | --- | --- |
| anarlog | https://github.com/fastrepl/anarlog | `b02b58de` | 2026-09-29 |
| Handy | https://github.com/cjpais/Handy | `29bd2c0` | 2026-09-29 |
| shorthand | https://github.com/mshish/shorthand | `90b8d40` | 2026-09-29 |

## Procedure

1. Fetch each clone. Do not pull yet. The working tree must stay at the reviewed commit until the review is complete.

   ```sh
   for d in anarlog Handy shorthand; do
     git -C reference-source/$d fetch origin
     git -C reference-source/$d log --oneline HEAD..origin/main | wc -l
   done
   ```

2. Review the new commits with `git log` and `git diff <reviewed>..origin/main` only. For each clone, sort each commit into one of these groups:
   - **Act now**: a fix to code that `PROVENANCE.md` records as borrowed, or a defect that Longhand also has.
   - **Idea**: a change worth a file in `docs/ideas/`. Create it with the procedure in `docs/ideas/README.md`.
   - **Note only**: a verified fact worth keeping. Add it to `docs/reference/borrowing-notes.md`.
   - **Not relevant**: web, billing, marketing, cloud-only features, and similar changes.
3. Fast-forward each clone to the reviewed commit: `git -C reference-source/<clone> merge --ff-only <commit>`.
4. Update the table above and add an entry to the log below. Commit this file on `main`.

## Log

### 2026-09-29

Ranges: anarlog `39613e5..b02b58de` (108 commits, about 44 relevant), Handy `8f9cf53..29bd2c0` (6 commits), and a first full review of shorthand at `90b8d40`. Shorthand last merged Handy at `05e0aed` (release 0.9.7).

**Act now** (done in PR #25. The 0.2.4 runtime uses 30-39 MB more peak memory on the same audio. This cost is accepted.)
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
