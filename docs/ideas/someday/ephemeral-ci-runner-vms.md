---
title: A new runner VM for each CI job
description: "Start each CI job in a new clone of a clean VM image, so nothing that one job leaves can reach a later job"
area: distribution
added: 2026-10-10
source: "the self-hosted CI runners brainstorm on 2026-10-10"
related: [self-hosted-ci-runners]
---

# A new runner VM for each CI job

## Idea
Each CI job starts in a new clone of a clean VM image, and the clone is deleted after the job. A cache folder from the host can keep the builds warm.

## Why
The first runner VMs keep their state between jobs, for speed. If contributions open to people outside the collaborators, a job from one PR must not leave files or processes that a later job runs.

A clean VM for each job also removes the faults that come from state that a job leaves: a stale CMake cache, old build output, or a broken tool cache.

## Notes / Current state
Added on 2026-10-20, after PR #57 moved the macOS checks to GitHub's `macos-latest-xlarge` runner. The repository variable `MACOS_RUNNER` selects the runner, so a new home runner needs no workflow change. The Mac Studio runner stops for now. A new runner can be on the old Mac Studio or on the Mac mini.

Possible design for macOS:

- **Image.** A Tart golden image with the toolchain (Xcode, Rust 1.95.0, Node, uv, Ninja, cargo-about) and the fetched crates. Packer (Tart plugin) can build it again when the toolchain changes.
- **One clone for each job.** A host service runs `tart clone`, gets a single-use runner config from `POST /orgs/twistymaze/actions/runners/generate-jitconfig`, runs the job in the VM, then runs `tart delete`. Tartelet (open source) and Cirrus Labs' Orchard do this loop. A script of about 50 lines can also do it.
- **Limit.** Apple's licence allows 2 macOS VMs at the same time on one Mac.

Possible design for Windows: the runner is a Proxmox VM (VM 420 on bravo). Roll back to a clean snapshot after each job (`qm rollback`, with an `--ephemeral` runner), or make a linked clone of a template for each job.

Warm builds: a new VM has no `target/`, so sccache keeps the builds warm. Use a shared sccache store on a home-lab server, not a folder in the VM:

- Redis (`SCCACHE_REDIS`) with `maxmemory` and `allkeys-lru`, or MinIO/S3 (`SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`) with a lifecycle rule that deletes old objects. sccache does not delete S3 entries.
- One key prefix for each platform (`SCCACHE_S3_KEY_PREFIX`), so that one platform's cache can be cleared alone.
- A wired LAN between the runner and the server: a build makes thousands of small reads and writes, so latency is more important than bandwidth.
- A server, not a host folder that the VMs share: the sccache local folder store allows one sccache server for each folder, so two VMs at the same time cannot share one folder.
- Cache credentials in the image or given at clone time, never in the repository. Optional: read-only access for PR jobs and write access only for `main`.
- `.github/actions/sccache-setup` selects the store. Its self-hosted branch sets `SCCACHE_DIR` now; it would set the server settings.

Expected speed: not measured. Test binaries, proc-macros and links are not cached, so a job in a new VM is slower than the old runner with a kept `target/`. Measured warm macOS checks on 2026-10-20, for comparison: self-hosted Mac Studio about 6 min, `macos-latest-xlarge` 11.1 to 13.4 min, `macos-latest` 22.1 min. The CodeQL Rust analysis took about 13 min. The sccache experiment (`docs/solutions/performance-issues/sccache-compile-cache-experiment.md`) measured 92 to 95% cache hits after a `Cargo.lock` change.
