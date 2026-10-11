---
title: Compile cache for the hosted builds
description: "Test sccache with its GitHub Actions cache on the hosted release builds, in place of or beside rust-cache"
area: distribution
added: 2026-10-11
source: "conversation during the releases-in-source-repository PR (twistymaze/longhand#2)"
related: [windows-preview-builds-and-updates, self-hosted-ci-runners]
---

# Compile cache for the hosted builds

## Idea

Set `RUSTC_WRAPPER=sccache` with sccache's built-in GitHub Actions cache backend (`SCCACHE_GHA_ENABLED=true`) in the jobs that run on GitHub-hosted runners: `release-build`, `release-windows.yml` and the `hosted=true` checks. sccache caches each compiled crate. rust-cache saves and restores the whole `target/` folder. Measure cold and warm job times with and without sccache before you keep it.

## Why

The hosted builds start with an empty disk. When the rust-cache key changes, or the cache misses, they compile every crate and ggml again. A cold Windows release build on the standard runner takes about 25 to 30 minutes (`docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`). sccache rebuilds only the crates whose inputs changed. One report measured up to about 50% faster than rust-cache with a warm cache (https://xuanwo.io/en-us/reports/2023-04/). That was one project, not a controlled test.

## Notes / Current state

- The self-hosted PR runners keep `target/` between jobs, so they would gain little. The idea is for the hosted jobs only.
- No Windows-specific measurements were found. Test on the Windows release build first, because it is the slowest job.
- sccache does not cache linking. `link.exe` time on Windows stays.
- Check how sccache and rust-cache work together: use both, or sccache only. The GitHub Actions cache has a size limit for each repository.
- **Turborepo was considered and rejected.** Its Cargo support (experimental, `experimentalCargoWorkspaces`) skips tasks for each crate and embeds sccache. Tests for each crate mean separate cargo runs, which resolve features differently and rebuild shared dependencies (the lesson in `cargo-feature-unification-ci-rebuilds.md`). The Tauri app depends on almost every crate, so task skipping would rarely skip the expensive build. The useful part, sccache, works without Turborepo.
