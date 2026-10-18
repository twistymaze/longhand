---
title: The NeMo helper was built for the CPU of the build machine and stopped with an illegal instruction on another CPU
date: 2026-10-17
category: build-errors
module: NeMo helper build (scripts/build-nemo.sh) and its CI smoke test
problem_type: build_error
component: service_layer
symptoms:
  - "\"Installed NeMo helper CPU inference (Windows)\" failed with \"Helper exited before ready\" within one second"
  - "The helper exited with code 29 (0x1d) and wrote nothing to stderr"
  - "\"Native helper protocol smoke test\" in the same job passed"
  - "The configure log said \"Adding CPU backend variant ggml-cpu: /arch:AVX512 GGML_AVX512\" on a CPU without AVX-512"
root_cause: config_error
resolution_type: config_change
severity: high
related_components:
  - infrastructure
  - testing_framework
tags:
  - nemo-helper
  - ggml-native
  - cpu-baseline
  - illegal-instruction
  - self-hosted-runner
  - cmake-cache
---

# The NeMo helper was built for the CPU of the build machine and stopped with an illegal instruction on another CPU

## Problem

ggml turns on `GGML_NATIVE` by default (its top-level CMakeLists.txt at the revision that `build-nemo.sh` pins; ggml is in the NeMo source checkout, not in this repository). `scripts/build-nemo.sh` did not set it, so the NeMo helper's CPU code was compiled for the CPU of the machine that built it: `-mcpu=native` on ARM, and on MSVC the result of `FindSIMD.cmake`. Each release could then run only on CPUs like its build machine. The Windows release on GitHub's `windows-latest` got `/arch:AVX2` by chance; a runner with AVX-512 would have made an installer that stops on most computers.

It became visible when the self-hosted Windows runner (VM 420 on bravo) changed its CPU type from `host` (Zen 4, with AVX-512) to `x86-64-v3` (no AVX-512). The runner keeps `W:\nemo-cache` between jobs, so the helper built earlier with `/arch:AVX512` was used again, and CPU inference stopped at the first AVX-512 instruction (PR #35).

## Diagnosis

- **Exit code 29 (0x1d) on Windows with no stderr is the low byte of `STATUS_ILLEGAL_INSTRUCTION` (0xC000001D).** The process stops at the instruction, so it writes nothing. `native/nemo-helper/main.cpp` exits only with 0, 64, 70, 72 or 74.
- The protocol smoke test passes because it sends only a rejected `start` and runs no ggml compute. Only the installed-helper test runs CPU inference.
- `scripts/test-nemo.ts` threw away the helper's stderr and exit code. It now puts both in the error when the helper stops early (PR #34); without that, the cause was not visible in the log.
- Compare the configure line `Adding CPU backend variant ggml-cpu: ...` with the CPU of the computer that runs the helper.

## Solution

`scripts/build-nemo.sh` sets each ggml CPU option of the baseline and checks the CMake cache after the configure:

```bash
cpu_baseline=(GGML_NATIVE=OFF GGML_SSE42=ON GGML_AVX=ON GGML_AVX2=ON GGML_BMI2=ON GGML_FMA=ON GGML_F16C=ON
  GGML_AVX_VNNI=OFF GGML_AVX512=OFF GGML_AVX512_VBMI=OFF GGML_AVX512_VNNI=OFF GGML_AVX512_BF16=OFF)
```

**`GGML_NATIVE=OFF` alone is not enough in a build folder that is kept.** ggml declares `GGML_SSE42`, `GGML_AVX`, `GGML_AVX2`, `GGML_BMI2`, `GGML_FMA` and `GGML_F16C` with the default `INS_ENB`, which is OFF after a native configure. The CMake cache keeps those OFF values when native is turned off later, so a kept folder builds without AVX2 while a clean release build uses AVX2. The independent cross-model review found this. On MSVC, `GGML_FMA` and `GGML_F16C` are not options (AVX2 implies them), so the cache check accepts any type (`NAME:[A-Z]+=VALUE`, with an optional CR).

With the baseline, the Windows log says `/arch:AVX2 GGML_AVX2;GGML_FMA;GGML_F16C;__BMI2__;GGML_BMI2`, and macOS gets no `-mcpu` flag. The other ggml copy in the app (transcribe-cpp) already ships one ggml-cpu module per instruction set and chooses at runtime, so it was not affected.

## Prevention

- Build ggml CPU code that ships to users for a named baseline, never with `GGML_NATIVE` on. For wider CPU support, use `GGML_BACKEND_DL` with `GGML_CPU_ALL_VARIANTS` (it requires native off and changes how the backends load).
- When you change a CMake option of a build folder that CI keeps, check what the cache keeps from the earlier configure, or delete the folder on the runner (`W:\nemo-cache\source\build\vulkan-asr` and `W:\nemo-cache\helper-vulkan` on the Windows runner).
- CI on one CPU cannot prove that a build runs on older CPUs. The configure line and the cache check are the evidence.

## Related

- [../performance-issues/nemo-cpu-threads-above-core-count-stall-inference.md](../performance-issues/nemo-cpu-threads-above-core-count-stall-inference.md): another NeMo CPU inference failure that wrote nothing to stderr.
