# Borrowing notes: anarlog and Handy

Verified findings about the two reference projects in `reference-source/`, kept because they cost real effort to establish and because several of them are traps that would otherwise be rediscovered during implementation.

Both clones are read-only upstream source. They are never edited, and `reference-source/` is not tracked by this repository. Claims below were verified directly against the code on 2026-09-21; where a claim is inference rather than observation, it says so.

## Licensing

**anarlog** is MIT except `enterprise/**`, which is commercially licensed. `reference-source/anarlog/LICENSING.md` states that the nearest license file controls and that *a file does not keep its previous license merely because it was moved* — so the boundary is checked at the source path, never at the destination. That file also explicitly names "local meeting capture, transcription, storage, export, and sync clients" as part of the MIT layer, which covers the recorder, the diarization crates, and the transcript model.

**Handy** is MIT throughout, with `license = "MIT"` declared in `reference-source/Handy/src-tauri/Cargo.toml`.

**transcribe-cpp** (the GGUF transcription runtime Handy uses) is MIT on crates.io, from `handy-computer/transcribe.cpp`. It is a standalone crate, not coupled to Handy.

**Model weights carry their own licenses**, which is where the real constraint sits. Among the models relevant here: MOSS is Apache-2.0; Parakeet is CC-BY-4.0 and requires attribution in a licenses screen; the multitalker and Sortformer GGUFs are published under the NVIDIA Open Model License, which obliges shipping the agreement to recipients. The Sortformer model card and NVIDIA's own card disagree about its license — resolve before distributing anything that includes it.

anarlog's own `LICENSING.md` requires a provenance record — upstream repository, immutable revision, file paths, copyright holder, license, and whether material was copied, modified, or used only as reference — kept beside the consuming code. That practice is worth adopting wholesale, and see the provenance gap below for why.

## Traps

**The committed ONNX blob has no provenance.** `reference-source/anarlog/crates/pyannote-local/src/data/segmentation.onnx` is a ~6 MB binary compiled in via `include_bytes!`, with no NOTICE, PROVENANCE, README, or LICENSE file anywhere in the crate. This contradicts anarlog's own third-party provenance policy. Download the weights directly from upstream under terms this project accepts rather than inheriting an unrecorded blob.

**`pyannote-local` ships only in-memory audio sources.** Its `AudioSource` trait is documented as random-access so that long recordings need not be resident, but the crate itself implements the trait only for `&[f32]` and `Vec<f32>`. The file-backed implementation lives in `crates/listener2-core/`, which drags in thirteen sibling crates and is not worth taking. A file-backed implementation has to be written here.

**MOSS silently truncates long audio.** Its upstream card documents `max_new_tokens` defaulting to 5120, far below what a multi-hour multi-speaker transcript needs, with no error on overflow. Anything built on MOSS must set this explicitly and verify the runtime exposes the knob.

**Parakeet's "no length limit" is about API acceptance, not feasibility.** transcribe.cpp reports `max_audio_ms == 0` for the Parakeet family, but NVIDIA's own model card states full attention handles roughly 24 minutes on an A100 80GB. Attention cost is quadratic in audio length. Do not read that constant as permission to submit four hours; measure before designing around it.

**Most diarizing models cap at four speakers.** The multitalker Parakeet bundle and Sortformer are both architecturally limited to four, which is below a typical table. MOSS is the only open-vocabulary speaker model in that catalog, and it carries no published diarization error rate at any speaker count. `pyannote-local` has no fixed ceiling, which is the main reason to prefer it.

**Models that emit no timestamps cannot be joined to a separate diarizer.** Canary, Cohere, and Qwen3-ASR produce excellent word error rates and `timestamps: "none"`, which disqualifies them from any architecture that aligns a speaker track against a text track — regardless of their accuracy.

**Two copies of ggml in one binary is a real risk.** whisper-rs statically links its own ggml, and transcribe-cpp brings another. anarlog already has code reaching into `whisper_rs_sys` for backend enumeration that assumes a single global backend registry. If both runtimes are ever linked together, resolve this before writing anything else; running one engine out of process is the escape hatch.

**transcribe-cpp has real build friction, already solved upstream.** Its backend libraries reach the consumer through `DEP_TRANSCRIBE_CPP_*` metadata that must be forwarded across cargo's one-hop `links` boundary, plus Linux rpath handling, Windows MSVC runtime staging, and a macOS Metal residency workaround. `reference-source/Handy/src-tauri/build.rs` solves all of it and should be read rather than rediscovered. The runtime also permits only one compute run per process.

## What each project actually does

**anarlog's recorder** (`crates/listener-core/src/actors/recorder/chunks.rs`) streams to disk in roughly 60-second MP3 chunks, calls `sync_all()` as each chunk closes, enforces a 256 MB free-space floor before writing, and recovers partial chunks at startup. Nothing accumulates in memory beyond a short lookback ring. This is the right shape for multi-hour capture and the reference implementation to follow.

**anarlog ends the session on sleep.** `apps/desktop/src/stt/detect-events.ts` handles `sleepStateChanged` by calling `stop()` unconditionally — no capability check, no preference, no resume. A laptop sleeping mid-session ends an anarlog recording deliberately.

**anarlog does have resume**, through a context-menu item whose label flips to "Resume listening" (`apps/desktop/src/session/components/outer-header/overflow/listening.tsx`), separate from the automatic crash-recovery path in `apps/desktop/src/stt/live-capture-recovery.tsx`. Both continue the existing session, and `capture-lifecycle.ts` backfills the gap from saved audio.

**anarlog's source supervision** allows three restarts of a failing audio source within a 22-second window — 15 seconds base plus a 7-second allowance so backoff sleep does not consume the budget — with 1s/2s/4s capped backoff.

**anarlog has no hierarchical summarization.** `crates/template-app/assets/_macros.jinja` renders the transcript by flat iteration into a single prompt, and the desktop workflow makes one generation call. There is no chunking, no map-reduce, and no token accounting anywhere in the path. A long transcript either fits the model's context or fails.

**anarlog drops timestamps at prompt-render time.** The renderer produces per-segment and per-word timings, and the mapping into the template type keeps only speaker and text. The summarizer therefore cannot refer to when anything happened. Worth fixing in anything derived from this.

**anarlog's diarization gate depends on a participant list.** Whether the microphone channel is diarized at all hinges on a speaker count derived solely from session participants, with no user-facing speaker-count input. On a stereo recording with no participants added, the microphone channel is never diarized. A rendered-label cap and a consolidation pass further collapse distinct speakers toward the participant count. All of this sits above the diarization engine, not inside it.

**Handy accumulates the whole recording in memory.** `src-tauri/src/audio_toolkit/audio/recorder.rs` grows an unbounded `Vec<f32>` returned at stop, with no maximum-duration constant or auto-stop anywhere. Roughly 920 MB for four hours, held twice because the WAV writer and the transcriber each own a copy. Correct for ten-second dictation, unusable for sessions.

**Handy discards structure it already asked for.** Its transcription entry point returns a plain `String`, and every engine path maps the richer result to `.text`. It explicitly requests segment timestamps from Parakeet and then drops them one line later.

**Handy excludes diarization deliberately.** Its catalog generator copies four capability keys from each model card and silently drops `diarize`; the capability struct has no field for it; inference never requests it; and a test asserts that pure diarization models stay out of the catalog. The capability exists in the models and is removed at three layers.

**Handy's `src-tauri/src/audio_toolkit/text.rs`** provides fuzzy custom-word correction using soundex and n-gram keys — directly applicable to proper nouns and domain jargon, and borrowable as-is.

## Settings and secrets

The two projects solve different halves of this and both halves are worth taking.

**Handy holds ordinary settings** in a `tauri-plugin-store` JSON file (`src-tauri/src/settings.rs`), whose path is resolved through `src-tauri/src/portable.rs` to an application data directory — or, in portable mode, to a directory beside the executable. Settings never sit in user content.

**anarlog holds secrets** in the OS keyring, wrapped by `plugins/store2/` over the `keyring` crate. Its value beyond the crate itself is the failure handling: it distinguishes a locked login keyring from an entirely absent Secret Service on Linux and says which is which, rather than surfacing a generic platform error. That distinction is the difference between a user who can fix the problem and one who files a bug.

Use both: the store for configuration, the keyring for credentials. Neither belongs anywhere a vault sync or a CLI would read.
