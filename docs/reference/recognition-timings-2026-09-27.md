# Recognition timing reference

Measured on 2026-09-27 during the flexible speaker-recognition implementation. These are local engineering measurements. They do not rank transcription or speaker-label accuracy.

## Conditions and limits

- Host: Apple M3 Ultra, 512 GiB memory, 32 logical CPUs, macOS 26.6.2, AC power. This is not the intended laptop acceptance result.
- Input: a five-minute, mono, 16 kHz excerpt from the existing 176.2-minute RPG recording. The recording and raw transcripts remain local.
- Build: Rust debug test harness, native libraries built with the configured acceleration. The route matrix enabled `diarize/coreml`; NeMo selected GPU device 0 (Metal). transcribe.cpp used its normal engine defaults. This is not a CPU-only comparison.
- Other builds and tests ran during these measurements. Each case has one observation. Cases ran in sequence, with a new test process per case. Filesystem caches were not cleared. Do not treat small differences as significant or extrapolate these times to three hours.
- The matrix used a frozen test executable from the implementation before code-review fixes. It includes the U11 runtime policy change at `9ab0495`. The exact full source tree was not recorded at build time; the binary hash below identifies this run. Later batching, recovery, and reuse fixes can change these results.
- All 49 route cases passed. A pass means the production recognition stages and their verification completed; it does not establish word accuracy, correct real speaker identities, or signed-app recording behavior.

## Five-minute after-Stop route matrix

Times are seconds for the entire test process: fixture setup, model loading, Assemble, recognition, Join, and Verify. Compilation and the matrix's preliminary model-hash checks are outside the timer. Live capture, summary generation, and Save/publication are excluded.

The first table uses transcribe.cpp for transcription. Separate speaker options use pyannote/WeSpeaker or the NeMo helper as indicated. “Off” has no speaker analysis.

| Transcription model | Off | pyannote/WeSpeaker | Sortformer | Nemotron 3 Diarization |
|---|---:|---:|---:|---:|
| Parakeet Unified EN 0.6B Q8_0 | 32.19 | 40.83 | 31.93 | 31.28 |
| Nemotron Streaming EN 0.6B Q8_0 | 23.47 | 41.53 | 31.76 | 31.52 |
| Nemotron 3.5 Streaming 0.6B Q8_0 | 24.49 | 42.88 | 33.47 | 34.29 |
| Multitalker Parakeet 0.6B Q8_0 | 28.70 | 46.55 | 33.09 | 32.72 |
| Parakeet TDT 0.6B v2 Q8_0 | 23.82 | 51.37 | 33.19 | 32.63 |
| Parakeet TDT 0.6B v3 Q8_0 | 24.30 | 43.19 | 32.92 | 32.94 |
| Parakeet TDT 1.1B Q5_K_M | 24.53 | 42.94 | 33.31 | 32.79 |
| Whisper large-v3-turbo Q5_K_M | 26.58 | 50.22 | 47.28 | 47.78 |
| MOSS Transcribe Diarize Q8_0 | 35.07 | 57.05 | 51.68 | 51.78 |
| Granite Speech 4.1 2B Plus Q5_K_M | 31.51 | 51.04 | 43.20 | 42.97 |

### Labels from the transcription model

These routes use the speaker-capable artifact through transcribe.cpp. No separate speaker model is loaded.

| Model | Seconds |
|---|---:|
| MOSS Transcribe Diarize Q8_0 | 35.44 |
| Granite Speech 4.1 2B Plus Q5_K_M | 32.64 |
| Multitalker Parakeet 0.6B Q8_0 | 40.22 |

Granite's integrated output has source-window timing, not word alignment. The working Multitalker bundle supports this offline case; this test does not establish live attributed output.

### Combined NeMo transcription and speaker recognition

These explicit comparison routes run both models through the same helper. The two streaming ASR families use live-capable native models, but this table feeds the full excerpt after Stop. It does not measure the advantage of doing work during recording.

| Transcription model | Sortformer | Nemotron 3 Diarization |
|---|---:|---:|
| Nemotron Streaming EN 0.6B Q8_0 | 47.34 | 46.36 |
| Nemotron 3.5 Streaming 0.6B Q8_0 | 47.35 | 48.69 |
| Parakeet TDT 0.6B v3 Q8_0 | 27.45 | 26.84 |

## Live processing

The following five-minute tests feed one second of audio per second. “Stop to Verify” starts when input ends and includes remaining live finalization, Assemble, recognition, Join, and Verify. It excludes summary generation and Save. These test the production durable-input path, not a real microphone or the signed UI.

| ASR | Speaker model | Stop to Verify (seconds) | Final rows | Updates before Stop |
|---|---|---:|---:|---:|
| Nemotron 3.5 Streaming 0.6B Q8_0 | Sortformer | 16.739 | 780 | 550 |
| Nemotron 3.5 Streaming 0.6B Q8_0 | Nemotron 3 Diarization | 16.746 | 780 | 550 |

### Existing speaker pipeline

On a five-minute excerpt, standalone analysis took 64.200 seconds on CPU and 9.144 seconds with CoreML; load times were 0.031 and 0.482 seconds respectively. This measures the same speaker-analysis workload, not transcription or summary generation. It is about a 7x difference in this observation, not a general hardware guarantee.

In paced five-minute live feature tests, CPU and CoreML completed 145 windows before Stop and 146 in total. Remaining final clustering took 9.983 seconds on CPU and 9.639 seconds with CoreML. The test asserted that final clustering did not repeat native feature inference. These are separate speaker-pipeline tests, not full ASR routes. Live pyannote capability was not enabled in the catalog at this measurement point.

## Longer runs and recording positions

| Workload | Input pace | Result | Measured completion |
|---|---|---|---|
| 22 minutes of real audio, combined Nemotron EN + Nemotron speaker model | 4x recording speed | 2 native epochs, 3,363 rows, 11,508 updates before Stop | 72.998 seconds from Stop through Verify |
| 125 minutes made by repeating the five-minute excerpt | 100x recording speed | 7 epochs, 20,158 rows, 5,301 updates before Stop | 1,483.623 seconds from Stop through Verify |
| First five minutes, combined Nemotron EN + Nemotron | After Stop | 829 rows, all tagged, 5 anonymous labels | 53.118 seconds processing; 61.37 seconds total harness |
| Last five minutes, same route | After Stop | 663 rows, all tagged, 4 anonymous labels | 51.982 seconds processing; 60.23 seconds total harness |

The end excerpt contains 299.9935 seconds because it ends at the recording boundary. Different observed label counts are not a correctness score.

The 125-minute run deliberately created a backlog. It is a duration, restart, and completion test, not a real-time benchmark. Live finalization took 1,083.138 seconds. Cumulative Stop through Assemble was 1,269.432 seconds; Transcribe took 33.813, Join 21.345, and Verify 158.328 seconds. The first attempt was interrupted because the test's finalization deadline was too short; the completed attempt used a 3,600-second test deadline.

A memory sampler recorded 119 observations at ten-second intervals during that run. The highest sum of parent and helper RSS was 1,275,664 KiB (about 1.22 GiB). Sampling began after launch; this is a sampled process RSS maximum, not exact peak allocation or total host memory pressure.

## What remains unmeasured

- A full 176.2-minute recording paced at recording speed on the intended laptop.
- Stop through a usable, saved transcript **and summary**, including summary endpoint latency and Save.
- Controlled release-build repeats, isolated host load, and a separate cold-load result.
- Native Windows timings.

The preferred two-minute completion target remains open. Do not present these measurements as that acceptance result or use them to replace Handy's published catalog assessments.

## Repeating the checks

Keep these tests local and opt-in. See [Local recognition checks](../development-workflow.md#local-recognition-checks). Run affected model/route combinations after processing changes; do not add this matrix or paced long runs to normal CI.

From the implementation checkout:

```sh
LONGHAND_ROUTE_MATRIX_SECONDS=300 bun scripts/validate-recognition/route-matrix.ts \
  /absolute/path/model-paths.json \
  /absolute/path/excerpt.wav \
  /absolute/path/installed/longhand-nemo-helper \
  /absolute/path/private-results
```

The local model-paths file contains artifact `id`, `path`, and `sha256` entries. The script checks those hashes, builds one test executable, and uses that executable for all cases. Output includes a `summary.json`, per-case logs, and private result JSON. Retain the summary and environment metadata for comparison; do not commit raw result JSON or audio.

For future measurements, also record the exact commit and dirty-tree state before compilation, binary hash, build profile, model hashes, backend, input duration and pace, competing workloads, and whether the timer includes load, summary, and Save. Use repeated runs before changing defaults based on small differences.

## Evidence identity

Original local artifacts: `u11-route-matrix-300/summary.json`, `u11-live-35-sortformer-300.log`, `u11-live-35-nemotron-300.log`, `u11-pyannote-live-300.log`, `u11-coreml-live-300.log`, `u8-live-22min.log`, `u11-live-synthetic-125min-final.log`, and `u11-long-memory.jsonl`. These names identify the local measurements; they are not repository dependencies.

- Frozen route test executable SHA-256: `9ea0c0ae69ffe67e3552ee0426f52d73c92b3f8b031bdfb46c251e7c1c642e7c`.
- NeMo-Speech.cpp revision: `97a15afa5caa9bce5baaa86c1184103877af4101`.
- transcribe.cpp package: `0.2.3`.

### Model artifact hashes

The local matrix verified these inputs before execution. File paths are omitted.

| Artifact ID | SHA-256 |
|---|---|
| `parakeet-unified-en-0.6b-q80` | `4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795` |
| `nemotron-speech-streaming-en-0.6b-q80` | `90d8c89714cd31efc88be62a40c6b2bea57e0cc2063af1ffe2c28f1a228ca110` |
| `nemotron-3.5-asr-streaming-0.6b-q80` | `b94545b313b3223fda7b2857a52681da813935c2127643d1e9ff0c23d988089c` |
| `multitalker-parakeet-streaming-0.6b-v1-q80` | `847df2b5b7576ee69801d74d60f5863df1503d09a790a57f0609323c15f26d64` |
| `parakeet-tdt-0.6b-v2-q80` | `f0d0e99cebb6d3b83f1f7069b82b5d3c2e39a54545b0da039cb4bafd9c4e5caa` |
| `parakeet-tdt-0.6b-v3-q80` | `5859f77944efcd8eafa23a6350731960b2b55b2203df51f319665c807d802cc7` |
| `parakeet-tdt-1.1b-q5km` | `18c22888fed10676fde72fdd5b833fab9175a6abf4e0cb390eb0a59b0b107cd3` |
| `whisper-large-v3-turbo-q5km` | `977b5db4e004349dffd1ab9caa10ba5aaba3fc3edd3ba72cadb84328a3203e36` |
| `moss-transcribe-diarize-q80` | `64ec654dc6ffcfdfe180422dffce1d33422b0c30959b7edfd131bad77ee35039` |
| `granite-speech-4.1-2b-plus-q5km` | `3415f2c2d5979ded038dc012e5a875d77602fa5a0699cc75ee7fa47991f42530` |
| `nemo-nemotron-speech-streaming-en-0.6b-q80` | `d9a01898d2a611c8764e23a1c2f45e70bbd5a425dc4de93692ac951dd603812d` |
| `nemo-nemotron-3.5-asr-streaming-0.6b-q80` | `3fc991d3badad7277c11030a7519832cddaf2057aafed6d4b25147e953a070b1` |
| `nemo-parakeet-tdt-0.6b-v3-q80` | `e3880d0aaaaf2c308ea2c35016b2b895c423eb3fda924c1b463d1c19b7f4d32e` |
| `sortformer-v2-q80` | `0679cfeb1ce356d0dea9470b31274f4bfc7eb927497d82005483770666da998a` |
| `nemotron-3-diarization-q80` | `08456d9e22cd9a323c0364d98375f3746d6e68507ebb705cd46438c534c7a3a1` |
| `pyannote-segmentation-3.0` | `057ee564753071c0b09b5b611648b50ac188d50846bff5f01e9f7bbf1591ea25` |
| `wespeaker-voxceleb-resnet34-lm` | `7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068` |
