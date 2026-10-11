---
title: NeMo CPU inference was 30 times slower on a runner with fewer cores than SDK threads
date: 2026-10-11
category: performance-issues
module: NeMo helper build (scripts/build-nemo.sh, scripts/patch-nemo.ts) and its CI smoke test
problem_type: performance_issue
component: service_layer
symptoms:
  - "Installed NeMo helper CPU inference (macOS)" timed out after 180000 ms on GitHub's macos-latest runner, with no output after the model download
  - The same test took about 4 s on the self-hosted Mac runner
  - The helper wrote nothing to stderr, so the log did not show if it was slow or stopped
root_cause: concurrency
resolution_type: code_fix
severity: medium
retire_when: "The NeMo pin moves to a revision whose diarizer sets its CPU thread count from the hardware thread count; check src/asr/diar/diarizer.cpp at the new pin. scripts/patch-nemo.ts refuses a source that does not match, so a pin bump also forces this check."
related_components:
  - infrastructure
  - testing_framework
tags:
  - nemo-helper
  - ggml-threads
  - cpu-inference
  - github-actions
  - hosted-runner
  - slow-or-hung
---

# NeMo CPU inference was 30 times slower on a runner with fewer cores than SDK threads

## Problem

The pinned NeMo SDK computes each CPU graph with 4 threads. On a machine with fewer than 4 hardware threads, speaker analysis on the CPU is about 30 times slower. On GitHub's macOS runner this made the installed-helper smoke test exceed its time limit. Users with CPU-only processing on a low-core machine had the same slowdown.

## Symptoms

- The CI step downloaded the 140 MB speaker model in 2 s. Then it stopped with `Helper test timed out` from `scripts/test-nemo.ts` after 180 s.
- The step passed in about 4 s on the self-hosted runner.
- The helper wrote nothing to stderr, so the log gave no progress.

## What Didn't Work

- **Showing the helper's stderr.** A diagnostic run streamed stderr. The helper wrote 0 bytes during the full run, so stderr cannot tell a slow helper from a stopped one.
- **Sampling the stack with `pgrep -f longhand-nemo-helper`.** The pattern also matched the bun process, whose arguments contain the helper path. The samples showed bun waiting, not the helper.
- **Trusting "it ran on hosted macOS before".** Nothing in the test, its audio fixture or the SDK pin changed after the test was added. No record showed that the step passed on GitHub's runner. Do not assume a regression without a passing run to compare against.

## Solution

1. **Find if it is slow or stopped, before you change a timeout.** A scratch branch with a push-triggered workflow ran only the helper build and the smoke test on `macos-latest`. It logged each protocol event (`hello`, `ready`, `ack`) with the elapsed time. The log showed steady progress: about 1.6 s of audio every 11.5 s, and 284 s in total (14 s to load, 256 s to process 34 s of audio). The helper was slow, not stopped. The runner reported `Apple M1 (Virtual)`, `hw.ncpu` 3, 7 GB.
2. **Find the thread count.** The SDK source is not in this repository. `scripts/build-nemo.sh` clones it at the pinned revision into the NeMo cache. In the NVIDIA/NeMo-Speech.cpp repository at the pinned revision `97a15afa5caa9bce5baaa86c1184103877af4101` (named in `scripts/build-nemo.sh`), `src/runtime/ggml/session.cpp:911` computes each graph with `ggml_graph_compute_helper_async(sched.get(), cr.gf, 4)`. The diarizer C API has no thread field.
3. **Measure each thread count on the same runner.** The diagnostic workflow edited that line, rebuilt, and ran the test again:

   | SDK CPU threads | Total | Processing |
   |---|---|---|
   | 4 (pinned) | 284 s | 256 s |
   | 3 | 26 s | 8.3 s |
   | 2 | 7 s | 6.3 s |

4. **The fix (PR twistymaze/longhand#4).** `scripts/build-nemo.sh:22` runs `scripts/patch-nemo.ts` after the revision checks. The script changes the thread count to the hardware thread count, with a maximum of 4, and marks the change "Modified by Longhand". Machines with 4 or more hardware threads keep the old behaviour. The script does not rewrite a source that is already patched, so a warm cache does not recompile. It keeps CRLF line endings, and it refuses a source that does not match the pinned text. `native/nemo-helper/PROVENANCE.md` records the change.
5. **A clear timeout message.** When the smoke test times out, it now reports the ready time and the number of samples that the helper acknowledged. A slow helper and a stopped helper now give different messages.

After the fix, the hosted step took 23 s.

## Why This Works

ggml's CPU workers busy-wait for each other at the end of each graph operation. When there are more workers than cores, at least one worker is not scheduled. The other workers then wait for the scheduler to give that worker a time slice, and this occurs at each operation. The result is not a proportional slowdown (4 threads on 3 cores is not 4/3 slower). It is a large stall at each operation, about 30 times here. Workers equal to or fewer than the hardware threads remove the stall.

NeMo-Speech.cpp `main` limits its recognizer to the hardware thread count, with the comment that more busy-waiting CPU workers than hardware threads slow the graph. Its diarizer still uses the default of 4. So a pin bump alone does not fix the `separate` route that the smoke test uses.

## Prevention

- Do not set a ggml CPU thread count above the hardware thread count. Treat it as a defect, not as a tuning choice.
- When you bump the NeMo pin, read the diarizer and session thread setup at the new revision. If upstream sets the count from the hardware, remove `scripts/patch-nemo.ts` and its call. If not, update the patch's anchor text.
- To tell slow from stopped in a CI timeout, log protocol progress with the elapsed time. Do not rely on stderr.
- To measure on GitHub's runner without the full checks job, push a scratch branch with a minimal workflow that runs on `push` to that branch. Delete the branch after the measurement.
- The self-hosted runner has 4 or more cores, so only a run with `hosted=true` (`gh workflow run ci.yml --ref <branch> -f hosted=true`) tests the low-core case.
