# Recognition timing reference

Measured on 2026-09-27 during the flexible speaker-recognition implementation. These are local engineering measurements. They do not rank transcription or speaker-label accuracy.

## Choosing by the wait after a meeting

**Status, 2026-09-28:** The extrapolations in this section are historical engineering estimates from a debug harness. Use the signed-app observations below when comparing measured waits. The first full-length native test exposed a live-view failure and required recovery; it did not establish the normal three-hour completion time. Do not publish the estimates as expected product performance. Granite's word-timing fix also changes its processing path, so the earlier Granite estimates need replacement measurements.

**For each 30 minutes you record, budget the extra processing time below after you press Stop.** These are early ballpark estimates for the tested M3 Ultra. Summary generation adds a separate wait that is not measured here. Other computers can take substantially longer.

The estimates multiply a five-minute observation by six and round outward to whole-minute ranges. They are extrapolations, not measured 30-minute results. Live finalization includes fixed work and any backlog, so its time may not grow in proportion to recording length. Use these estimates for planning and comparison; verify longer runs before publishing them as expected product performance.

### Choose when processing happens

These rows use the labels in Settings. The example transcription model is **Nemotron Speech Streaming EN 0.6B**, with the catalog-selected **Nemotron 3** speaker model.

| Transcription timing | Speaker recognition | Extra processing per 30 minutes recorded | What happens after Stop |
|---|---|---|---|
| During recording | During recording | **About 1–2 minutes** | Finish the work already in progress and prepare the final transcript. |
| During recording | After Stop | **About 2–3 minutes** | Keep the completed text, then finish speaker recognition and join the labels. |
| After Stop | During recording | **About 4–5 minutes** | Transcribe the recording and join the speaker work completed during recording. |
| After Stop | After Stop | **About 3–4 minutes** | Transcribe and identify speakers after recording ends. |

The live rows come from paced five-minute tests: 16.612, 25.763, and 44.465 seconds after Stop respectively. The last row uses the normal after-Stop choice in the 49-case matrix: 31.519 seconds. These are different processing paths, not a controlled experiment that changes only one switch. In this implementation, live speaker work alone did not reduce the observed finish time. Do not promise that every live setting is faster.

With both timings set to **During recording**, **Sortformer** gave the same rough **1–2 minute** estimate. **Nemotron 3.5 ASR Streaming 0.6B** also gave **1–2 minutes** with either **Sortformer** or **Nemotron 3**. Other live model/timing combinations do not yet have a comparable estimate. Unavailable choices remain disabled in the app.

### Choose a model with processing after recording

Set **Transcription timing** to **After Stop**. Set **Speaker recognition** to **Off** or **After Stop**. The catalog selects the speaker model; this choice does not require an engine selection.

Each cell is **extra minutes per 30 minutes recorded**, excluding the summary. These estimates use automatic speaker selection with no expected speaker count. A count above the selected model's limit can require a different speaker model and more time.

| Transcription model | Speaker recognition Off | Speaker recognition After Stop |
|---|---:|---:|
| Parakeet Unified EN 0.6B | 3–4 min | 3–4 min |
| Nemotron Speech Streaming EN 0.6B | 2–3 min | 3–4 min |
| Nemotron 3.5 ASR Streaming 0.6B | 2–3 min | 3–4 min |
| Multitalker Parakeet Streaming 0.6B | 2–3 min | 3–4 min |
| Parakeet TDT 0.6B v2 | 2–3 min | 3–4 min |
| Parakeet TDT 0.6B v3 | 2–3 min | 3–4 min |
| Parakeet TDT 1.1B | 2–3 min | 3–4 min |
| Whisper large-v3-turbo | 2–3 min | 4–5 min |
| MOSS Transcribe Diarize | 3–4 min | 5–6 min |
| Granite Speech 4.1 2B Plus | 3–4 min | 4–5 min |

For example, a 90-minute meeting with Whisper and automatic speaker recognition suggests **12–15 extra minutes**, plus summary generation. This extrapolates from the short test; it is not a measured 90-minute result.

### Advanced speaker-model overrides

The following table applies only when the user changes **Speaker model override** under **Advanced options**. Both processing timings are after recording. Each cell is **extra minutes per 30 minutes recorded**, excluding the summary. A dash means the transcription model does not provide that speaker-label option.

| Transcription model | Off | Transcription model labels | pyannote / WeSpeaker | Sortformer | Nemotron 3 |
|---|---:|---:|---:|---:|---:|
| Parakeet Unified EN 0.6B | 3–4 min | — | 4–5 min | 3–4 min | 3–4 min |
| Nemotron Speech Streaming EN 0.6B | 2–3 min | — | 4–5 min | 3–4 min | 3–4 min |
| Nemotron 3.5 ASR Streaming 0.6B | 2–3 min | — | 4–5 min | 3–4 min | 3–4 min |
| Multitalker Parakeet Streaming 0.6B | 2–3 min | 4–5 min | 4–5 min | 3–4 min | 3–4 min |
| Parakeet TDT 0.6B v2 | 2–3 min | — | 5–6 min | 3–4 min | 3–4 min |
| Parakeet TDT 0.6B v3 | 2–3 min | — | 4–5 min | 3–4 min | 3–4 min |
| Parakeet TDT 1.1B | 2–3 min | — | 4–5 min | 3–4 min | 3–4 min |
| Whisper large-v3-turbo | 2–3 min | — | 5–6 min | 4–5 min | 4–5 min |
| MOSS Transcribe Diarize | 3–4 min | 3–4 min | 5–6 min | 5–6 min | 5–6 min |
| Granite Speech 4.1 2B Plus | 3–4 min | 3–4 min | 5–6 min | 4–5 min | 4–5 min |

These cells cover the advanced speaker-model overrides. The alternative internal-runtime comparisons below are engineering evidence; users do not choose a runtime. Quantization and exact artifacts are recorded in the technical tables and hashes.

**Example:** For a 90-minute meeting using Whisper with **pyannote / WeSpeaker**, the table suggests roughly **15–18 extra minutes**, plus summary generation. This is a linear planning estimate from the short test, not a measured 90-minute result.

## Conditions and limits

- Host: Apple M3 Ultra, 512 GiB memory, 32 logical CPUs, macOS 26.6.2, AC power. This is not the intended laptop acceptance result.
- Input: a five-minute, mono, 16 kHz excerpt from the existing 176.2-minute RPG recording. The recording and raw transcripts remain local.
- Build: Rust debug test harness, native libraries built with the configured acceleration. The route matrix enabled `diarize/coreml`; NeMo selected GPU device 0 (Metal). transcribe.cpp used its normal engine defaults. This is not a CPU-only comparison.
- Other builds and tests ran during these measurements. Each case has one observation. Cases ran in sequence, with a new test process per case. Filesystem caches were not cleared. Do not treat small differences as significant. The rounded extrapolations above are planning estimates; they do not establish three-hour performance.
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

## Later verification on the same host

A second five-minute route matrix passed all 49 combinations. It used source revision `d8e2fd3` and frozen executable SHA-256 `594c00fd1a5f5996015aaf3a8ec404b4ff8fb5854022dbe136af53b0b8d285c9`. The local evidence is `post-review-route-matrix-300/summary.json`. Builds and other tests ran on the host during this matrix, so these results verify execution and coverage; they do not replace the timing estimates above.

A separate five-minute replay, paced at recording speed, completed with combined Nemotron Speech Streaming EN and Nemotron 3 Diarization. It produced 575 updates and 802 final rows. Stop through Verify took 16.710 seconds. This excludes summary generation and Save. Evidence: `post-review-live-300.log`.

A signed-app check then played 30 seconds of recorded conversation through system audio into the selected virtual microphone. Parakeet Unified produced live text, speaker recognition completed afterwards, and the app saved the transcript and summary. Recording and processing each held a sleep assertion, which was released at completion. This short test establishes the audio route and complete app flow; it is not a performance benchmark or a substitute for the longer UI tests.

These checks preceded the live paragraph-rendering fix and the correction that excludes ASR request padding from the speech-coverage denominator. Those changes require their own current-build checks.

## Signed-app recording checks, 2026-09-28

All ten transcription models completed a new recording through the configured virtual microphone. The input was conversation audio played through system output at normal speed. Each case saved both a transcript and a summary. Only the local manual-test folder received new notes.

These checks used the Developer ID signed release bundle from `e14d651`. Its executable SHA-256 was `5fe0c0cdf2d75a0eae5c4d21427cd022bf92672a353f99098090b3b4a1b79680`. The host was the M3 Ultra described above. Other local validation ran at the same time. These are single observations, not isolated benchmarks or intended-laptop acceptance results.

Unlike the earlier harness timings, the following waits include summary generation and saving. The configured summary endpoint used `gpt-6-luna`. Recording durations include a short setup or silence interval in addition to the listed playback duration. Times are rounded from the Stop action and the saved-result log.

| Transcription model | Transcription timing | Speaker recognition | Speaker model used | Conversation playback | Stop to saved transcript and summary |
|---|---|---|---|---:|---:|
| Nemotron Speech Streaming EN 0.6B | During recording | During recording | Nemotron 3 | 5 min | About 14 s |
| Parakeet Unified EN 0.6B | During recording | After Stop | Nemotron 3 | 5 min | About 25 s |
| Granite Speech 4.1 2B Plus | After Stop | After Stop | Transcription model labels | 5 min | About 28 s |
| Multitalker Parakeet Streaming 0.6B | During recording | After Stop | Transcription model labels | 5 min, split by Stop and Resume | About 34 s after the final Stop |
| Nemotron 3.5 ASR Streaming 0.6B | During recording | During recording | Sortformer | 2 min | About 12 s |
| Whisper large-v3-turbo | After Stop | After Stop | pyannote / WeSpeaker | 2 min | About 20 s |
| Parakeet TDT 0.6B v2 | After Stop | During recording | Nemotron 3 | 2 min | About 12 s |
| Parakeet TDT 0.6B v3 | After Stop | During recording | Nemotron 3 | 2 min | About 14 s |
| Parakeet TDT 1.1B | After Stop | Off | None | 2 min | About 13 s |
| MOSS Transcribe Diarize | After Stop | After Stop | Transcription model labels | 2 min | About 14 s |

The integrated-label tests completed, but labels restarted across processing sections. The five-minute Granite and Multitalker tests produced 30 and 38 distinct section-scoped labels. The two-minute MOSS test reached 14. These are not estimates of the number of participants. Later signed-app comparisons used separate Nemotron speaker analysis. Multitalker retained five labels and saved about 16 seconds after Stop with both tasks during recording. Granite retained five labels and saved about 42 seconds after Stop with both tasks afterwards. These comparisons support separate analysis for label continuity; they do not establish speaker accuracy. The automatic after-Stop order for these three families is now Nemotron, pyannote, then integrated labels. An expected count above eight selects pyannote. Explicit saved choices and Advanced integrated labels remain available. The normal-choice timing table above now uses these defaults; the advanced table retains both routes.

The tests also found two display defects: a completed model download could retain a stale downloading notice, and a transcript with speaker recognition Off could show a false failure notice. Both have regression fixes after this bundle. The saved Off transcript already had the correct metadata. A new signed build and native checks must verify the display fixes.

### Current coverage check

A third five-minute backend matrix passed all 49 combinations with the corrected speech-coverage denominator. Source: `041e146`. Frozen executable SHA-256: `6095c9ff2651362aab468b3547deb929c5a452d3a409c403f7ef7ce1ab29c646`. Local evidence: `vad-fixed-route-matrix-300/summary.json`. The sum of case times was 1,816.954 seconds with concurrent native tests, downloads, and other checks. This confirms execution and coverage; it does not replace the earlier timing estimates or prove speaker accuracy.


## Full-length validation and corrections, 2026-09-28

The first 176.2-minute native playback used signed release source `3058b96` with Nemotron Speech Streaming EN and Nemotron 3, both during recording. Audio capture continued, but the live transcript view became unresponsive as output accumulated. The renderer reached about 5.0 GiB RSS in the final memory samples. Earlier backend-plus-helper memory samples did not include that renderer and therefore did not establish total app memory use.

Stop did not complete through the unresponsive view. After checking that durable audio covered the full playback, the app was closed through its native menu and restarted. The recovered note saved its transcript and summary about 21 minutes 20 seconds after the recovered Stop. It reprocessed the verified recording because the interrupted live output could not pass provenance checks. Summary generation began about 24 seconds before Save. This run failed the normal live-view and Stop interaction checks. Recovery completion is a separate measurement and must not replace that result.

Source `d031ca2` indexes row comparison and speaker projection and batches incoming updates. A synthetic state with 40,000 words, separate speaker snapshots, and 1,000 additional patches took about 25.5 ms after the fix, compared with about 27.7 seconds before it. This is reducer evidence, not a native rendering measurement. The native long-note Resume check remains pending.

Recovery also exposed a five-second gap in idle-sleep protection during audio preparation between processing stages. Source `76c1326` starts protection before that work. The regression failed before the change and passed after it. All 133 scheduler tests and pipeline lint passed. Native verification of the new bundle remains pending.

The full-length Granite plus Nemotron speaker-analysis harness passed after the word-timing fix. It produced 26,773 text rows, including 22,710 with a speaker assignment, and reached the source tail. Anonymous labels remain scoped to processing epochs; the 49 observed labels do not establish 49 people. The debug harness took 3,609.94 seconds in total, with concurrent native processing on the host. A process sample showed Verify executing the debug-build voice detector. This elapsed time is functional-test evidence, not a release-app estimate. Local evidence: `granite-word-timing/full.log` and `full.json`.

### Signed release follow-up, source `55ec7cc`

The app was rebuilt, verified with its Developer ID signature, and restarted before these tests. The executable SHA-256 was `aabb17f4fea8ffe48cba5c1519b693c76a8dfc63a8bbf4dcf95a4c723446a792`.

- **Long-note Resume:** Five more minutes of conversation were recorded into the new full-length test note. Stop responded and processing completed. Historical words appeared more than once in the live view, so this remains a failed display check. The renderer reached about 3.7 GiB during this observation. Save occurred about 25 minutes 21 seconds after Stop, including pauses while other test notes recorded. Historical audio was processed again. This is not a normal live-finalization benchmark.
- **Granite with automatic Nemotron speakers:** A fresh five-minute playback completed with both processing choices after recording. It waited about 13 minutes 54 seconds behind other notes, then took about 1 minute 52 seconds from processing start through the saved transcript and summary. The total wait after Stop was about 15 minutes 47 seconds. This test used the first five minutes of the full recording; do not treat differences from earlier excerpts as a controlled model comparison. The transcript reached the tail with four anonymous labels. Some paragraphs were short and speaker changes were frequent; completion does not establish label quality. The coverage check found 230,690 ms covered out of 232,448 ms voiced, with a longest uncovered span of 592 ms.
- **Speaker-only reprocessing:** On the new speaker-Off test note, retaining Parakeet TDT 1.1B and enabling speaker recognition reused all 249 saved text rows with identical timings. Of those rows, 213 received labels. The replacement transcript and summary saved about 11 seconds after the action. The title and folder stayed unchanged. A separate cancellation through Resume had already left the previous result files unchanged.
- **Native settings and notices:** Choosing offline Granite disabled live transcription and saved the choice without Apply. Changing next-note settings did not change the active recording's snapshot. The speaker-Off notice displayed correctly. During recording, the recording sleep assertion remained active while processing jobs waited; those jobs did not retain a processing assertion.

The long-note display check exposed a separate history-selection defect after the batching fix. Source `5a7ca68` corrects it. A new signed build then completed another five-minute Resume check on that same new test note. The visible history appeared once, new words and labels continued, and Stop responded within about 1.3 seconds. The renderer's highest sampled RSS was 772,800 KiB, about 0.74 GiB, across 29 samples at ten-second intervals. This is a short check with a long existing transcript, not another full 176-minute live run or an exact allocation peak. Local evidence: `native-dedup-resume-memory.jsonl`.

That build still recognizes saved historical audio after Resume. Appending a segment changes the whole-recording audio identity; decoding the saved Opus archive can also change the processing-PCM digest. The conservative checks then invalidate earlier recognition. A verified saved-prefix reuse path is being tested separately. Until it passes a native check, do not describe Resume as processing only new audio. The intended-laptop target and native Windows runtime checks remain open.
