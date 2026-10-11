# CI runner timing reference

Measured on 2026-10-10 and 2026-10-11 during the self-hosted CI runners work (`docs/plans/2026-10-10-1612-ci-self-hosted-runners-plan.md`). Each time is the wall-clock time of one `Checks` job. A "cold" job starts with an empty `target/` on the runner. A "warm" job starts with `target/` from an earlier job.

## Runners

- **GitHub macOS:** `macos-latest` (3 cores).
- **GitHub Windows:** the org's larger runner `windows-8core` (8 cores, 32 GB).
- **Home macOS:** a Tart VM with 16 CPUs and 64 GB on an M3 Ultra Mac Studio.
- **Home Windows:** a Windows Server 2025 VM with 32 vCPUs. Two runners share the VM, and each runner has its own `target/`.

## Checks job

| Job | GitHub runner | Self-hosted runner, cold | Self-hosted runner, warm |
|---|---|---|---|
| macOS checks | about 14 min before U3, about 9.5 min after U3 | 7.7 min | 6.5 min |
| Windows checks | about 20 min | 13.4 min | 4.9 to 5.0 min |

U3 builds `pipeline`, `earshot`, `sha2`, `vad` and `recorder` with `opt-level = 3` in the dev profile. It helps on every runner: `crates/pipeline/tests/runner.rs` takes 46 s on the Mac Studio (71 s before) and 89 s with three threads (429 s before).

On the Windows VM, 32 vCPUs gave the same times as 16. A warm job is mostly linking and a few long test binaries.

## Windows checks with the release installer

`gh workflow run ci.yml -f windows_installer=true` adds a release installer build to the Windows job.

| Run | Windows job | Release installer step |
|---|---|---|
| 38140458607, cold | 15.9 min | 668 s |
| 38141550636, warm | 12.3 min | 445 s |

The release step is long even when warm. The warm run reused only part of the cold build. In run 38141550636 the macOS job took 10.0 min, because it built the NeMo helper again (130 s) after the VM was reset to its clean copy.

## Limits

- Each row is one or two runs, not an average.
- A warm time depends on how much changed since the runner's last job.
- A job for an offline self-hosted runner waits in the queue. `-f hosted=true` runs the checks on GitHub's runners.
