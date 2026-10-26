---
title: A new Windows runner VM for each CI job
description: "Give each Windows CI job a clean VM, as the macOS jobs already have, so state from one branch's job cannot break a later job"
area: distribution
added: 2026-10-24
source: "the Windows half of ephemeral-ci-runner-vms, which shipped for macOS only (PR #60); the stale DLL fault on PR #68"
related: [ephemeral-ci-runner-vms, self-hosted-ci-runners]
---

# A new Windows runner VM for each CI job

## Idea
Each Windows CI job starts in a clean VM, and the VM goes back to its clean state after the job, as the macOS jobs do since PR #60. The builds stay warm through a shared cache, not through a kept `target\` folder.

## Why
The self-hosted Windows runner (`longhand-windows`) kept its state between jobs and shared one `target\` folder for all branches. State that one branch's job left broke later jobs:

- **Stale transcribe.cpp DLLs (2026-10-24, PR #68).** The `transcribe-cpp-sys` build script copies `transcribe.dll` and `ggml*.dll` into `target\debug`, `deps` and `examples`, but only when the build script runs. Cargo does not track these copies. A branch on transcribe.cpp 0.4.1 overwrote `deps\transcribe.dll`. The next branch, on 0.2.4, reused its cached build script output, so nothing copied the right DLL back. Its `asr` test binary loaded the 0.4.1 DLL and stopped with `STATUS_ENTRYPOINT_NOT_FOUND` (0xc0000139). A rerun fails the same way. The fault comes back each time two branches with different transcribe.cpp versions run on the runner.
- **Earlier faults of the same kind:** a stale CMake cache (C1041), a broken tool cache, and `setup-bun` EBUSY on the shared runner (`docs/solutions/build-errors/setup-bun-ebusy-on-shared-windows-runner.md`).

A job from a PR must also not leave files or processes that a later job runs, which matters when the repository becomes public.

## Notes / Current state
Possible designs:

- Roll the VM back to a clean snapshot after each job (`qm rollback`), with a single-use (`--ephemeral` or JIT) runner registration.
- Or make a linked clone of a template for each job and delete it after the job.
- Keep the build warm with the GitHub Actions cache protocol, as the macOS runners do (`LONGHAND_RUNNER_EPHEMERAL`), or with a shared sccache server. The design notes on cache stores are in [ephemeral-ci-runner-vms](ephemeral-ci-runner-vms.md).

Smaller fixes for the DLL fault, until the VM is ephemeral:

- A CI step before the Windows tests that copies the DLLs from the current build's `transcribe-cpp-sys-*/out` folder into `target\debug` and `deps`. `ci.yml` already finds the newest build folder this way for the release check.
- A separate `target\` folder for each branch on the runner.
- An upstream fix in `transcribe-cpp-sys`: the build script names the copied DLLs in `rerun-if-changed`, so cargo runs it again when another build overwrites them.

The runner setup scripts are in the private `twistymaze/homelab-ci` repository (`windows/`).

## Outcome
Not built. PR #71 (2026-10-25) moved the Windows checks to GitHub's runners, so each Windows job gets a new VM. Pull requests use the free `windows-latest`, and `main` and runs on demand use an 8-core larger runner (`WINDOWS_RUNNER`). The self-hosted Windows runner is removed. The comparison that made this choice is in the private homelab-ci plan `docs/plans/2026-10-25-1017-ci-ephemeral-windows-runner-plan.md` (U1): warm Windows checks took about 13 to 15 minutes on 8 cores and about 24 minutes on `windows-latest`. PR #71 also fixed the CI step that deleted the CMake builds that rust-cache restores, and added a NeMo build cache.
