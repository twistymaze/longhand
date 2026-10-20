# NeMo helper

The helper links the pinned SDK. It does not copy or modify its source. The C ABI supplies standalone speaker analysis and offline ASR. A narrow C++ adapter supplies combined streaming word corrections.

Build with `scripts/build-nemo.sh`. Set `LONGHAND_NEMO_BACKEND` to `cpu`, `metal`, or `vulkan`. Windows builds require an MSVC developer shell, Ninja, Git Bash, and the Vulkan SDK for Vulkan. The script builds pinned static SentencePiece. It writes `build-output.json` with `helper` and `sdkBin` paths. The app packaging script must copy and verify the full private dependency closure. The CMake install target installs only the executable; it does not claim to make a complete app package.

## Protocol version 1

Each frame has two little-endian u32 lengths, JSON then PCM bytes. JSON is at most 1 MiB. PCM is at most 16,000 finite little-endian f32 samples. Input is mono at 16 kHz. No result frame has PCM. Each request includes `version`, `generation`, `epoch`, and a contiguous `seq` starting at zero. Every response repeats these fields. Paths are local model artifacts already validated by the caller.

A `start` request includes `route`, `speaker_model` when required, `asr_model` when required, `gpu` (-1 for CPU), `language` (optional), `diar_preset` (optional), `epoch_start`, and `max_epoch_samples`. The maximum epoch is one hour. Callers normally select a shorter bound from measured memory and replay cost. The helper emits `hello` with its runtime revision and routes before model load, then `ready` with the selected backend. An unavailable requested GPU fails; the caller can explicitly retry on CPU.

A `push` request includes `offset` and PCM. Offsets must be contiguous. `ack.accepted_end` means accepted input, not committed result coverage. The caller must persist input before queueing it. A full queue returns an error to the caller; recording must continue and record a backlog.

`finish` has no PCM. `finished` reports complete `coverage_start` and `coverage_end`, including silence. Only that terminal event proves the complete native result. An `error` event is structured failure, never complete coverage. Pipe EOF without `finished` is failure.

`combined_stream` emits:

- `interim`: replace the current provisional hypothesis. Both text and speakers are provisional.
- `text`: finalized utterance text, keyed by `result_id`.
- `words`: patches keyed by `result_id` and word `index`. Each word has canonical sample `start` and `end`, a native one-based `speaker` (zero means unknown), and separate `speaker_final`. Each patch has an increasing `revision` and `text_final`.
- `punctuation`: append late endpoint punctuation to the indicated result. This is an explicit text revision from the SDK.

The helper refreshes buffered word labels before each push and after decode steps. It emits and removes stable prefixes. It retains only the revisable final-word suffix. Native millisecond word offsets are converted once. Native frame padding is clamped to the accepted end.

`separate` emits transactional timeline snapshots. `segments_begin` starts a `revision`, `segments` carries bounded batches, and `segments_end` completes it. A caller stages the batches and replaces the epoch timeline only after the matching end event. A speaker namespace is `(generation, epoch, native speaker)`. No namespace survives a native reset without caller-validated continuity evidence. Probability compaction supplies a conservative stable boundary. Until finish, later segments remain provisional.

`combined_offline` runs offline ASR in bounded 30-second input blocks and speaker analysis in an independent stateful stream. This supports offline TDT without passing a long recording to a full-attention call. It emits text and word timing plus the same speaker timeline snapshots. Words use `speaker_source: timeline`; the caller joins them with the complete timeline. No native speaker tag is invented. The diarizer stays active across ASR blocks. Hard ASR block boundaries need excerpt and long-session quality tests; this adapter does not claim seamless text at those boundaries.

`asr_only` requires `asr_model` and a boolean `streaming`. It does not load a speaker model. Set `streaming` to true for streaming ASR or false for offline ASR in bounded 30-second blocks. It emits text and words with speaker zero and no speaker timeline. The caller can run speaker recognition at an independent time.

## Ownership and memory

The supervisor drains stdout and stderr on separate threads. Input and output queues are bounded. Diagnostics retain only the last 16 KiB. A full result queue is a terminal error; the supervisor does not discard output and then claim success. Cancellation kills and reaps the child before joining pipe threads. A Windows kill-on-close job owns the process. A native watchdog exits after parent death, including while inference or pipe output is blocked.

The pinned SDK keeps compacted speaker segments. Its probability compaction needs an all-speaker silence gap; otherwise the probability history can grow. See `src/asr/diar/diar_pipeline.cpp`, `maybe_compact`, and `frozen_segs_` in its header. The helper therefore enforces the caller's epoch sample budget. It never silently resets a stream. The parent must finish the epoch, persist output, and explicitly start a new namespace. This bounds a process by the selected epoch; it does not claim constant memory for an unlimited native stream. U11 must measure memory, replay time, and attribution across epochs.

## Checks

- `cargo test -p nemo-runtime --features test-helper` runs framing and deterministic child-process tests. Normal workspace tests run the framing tests. Add `--features nemo-runtime/test-helper` to the workspace invocation for process tests.
- `node scripts/test-nemo.ts --helper PATH` checks native malformed-version rejection without models.
- Add `--speaker PATH --route separate --wav PATH --gpu -1` for local CPU inference. The WAV must be mono 16 kHz PCM16 or float32. Add `--asr PATH --route combined_stream` or `combined_offline` for those routes.
- Run the same smoke command against the installed helper with developer search paths absent. A successful source-tree run does not verify packaging, signatures, Windows, or acceleration.

The native smoke runner accepts `--sample-start`, `--sample-count`, and `--epoch-start` to test separate parts of a local WAV without making audio copies. `--timeout-ms` bounds the whole run. For `asr_only`, pass `--asr PATH --route asr_only --streaming true|false` without `--speaker`. It requires nonempty text and words, zero speaker tags, and no speaker timeline. For speaker routes with `--wav`, it requires nonempty speaker output, and combined routes require nonempty words and text. Combined streaming also requires nonzero word speaker tags. Metrics include load, processing, and finish times, coverage, unique speakers, attributed words, and revisions. `--output PATH` writes the final local result for inspection; use a private, ignored path. It never prints transcript text.

`node scripts/package-nemo.ts BUILD-OUTPUT.json [OUTPUT-DIR]` stages the full private dependency closure. macOS dependencies must resolve inside the pinned SDK bin directory or system library directories. The script removes build RPATHs and rewrites private references to `@loader_path`. It signs dylibs first and the helper last, then hashes the signed bytes. Signing uses `LONGHAND_NEMO_SIGNING_IDENTITY`, then `APPLE_SIGNING_IDENTITY`, then an ad hoc signature. A non-ad-hoc identity uses hardened runtime and timestamping. Windows requires `dumpbin`; VC runtime and Vulkan DLLs must resolve from explicit SDK directories. Missing dependencies fail staging.

`--parent-death` checks native cleanup while the helper waits on its input pipe. `--cancel-stage load|push|finish` uses the supplied models with synthetic audio and kills the helper at that native phase. It requires process reaping within two seconds and fails if a terminal result arrived before cancellation. `--cancel-delay-ms` controls the delay after the phase starts. These checks complement the Rust supervisor's bounded-pipe cancellation tests.
