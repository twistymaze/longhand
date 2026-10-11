---
title: Compile Cache for CI - Plan
type: ci
date: 2026-10-11
topic: sccache-compile-cache
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Compile Cache for CI - Plan

## Goal Capsule

- **Objective:** CI on pull requests and the release builds on `main` take less time. The user decides from measured data whether sccache stays.
- **Means:** A measured sccache experiment on the hosted jobs and the self-hosted runners. A report follows, then one keep-or-remove decision.
- **Product authority:** The user makes the keep-or-remove decision (R16). This plan does not set a threshold.
- **Open blockers:** None.

---

## Product Contract

### Summary

Run sccache on the GitHub-hosted jobs and give the self-hosted runners a local sccache cache. Measure each setup against today's rust-cache, cold and warm. Report the result. The user then keeps sccache everywhere or removes it everywhere.

### Problem Frame

CI is slow when the compile cache misses. On 2026-10-11, the hosted runs (`hosted=true` dispatch, run 38172180403) took 36 min on Windows and 23.5 min on macOS. Windows found no cache and compiled 526 crates. macOS had a full rust-cache hit but still compiled 185 crates. The macOS release build ("Preview build", run 38155311321) took 24 min. A cold ggml CMake build alone takes 5 to 14 min on the Windows runner (`docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`).

The self-hosted PR checks are fast (6.7 min macOS, 4.7 min Windows, run 38170140849) because target/ stays on the runner between jobs. On macOS, the test runs take 158 s of that time, and the rebuilds of changed workspace crates take most of the remaining compile time. sccache cannot skip those rebuilds. When target/ is removed, or a Cargo.lock change invalidates it, these runners compile everything again. The Windows release build on `main` is cold every time: its rust-cache step restores but never saves, because the workflow starts by `workflow_run`.

### Requirements

**Hosted jobs**

- R1. The experiment covers the hosted jobs that compile Rust: `release-build` and the `checks` matrix with `hosted=true` in `.github/workflows/ci.yml`, and `build` in `.github/workflows/release-windows.yml`.
- R2. The experiment compares three cache setups on each hosted job: today's rust-cache (baseline), rust-cache plus sccache, and sccache without rust-cache.
- R3. Both sccache setups use sccache's GitHub Actions cache backend.
- R4. In both sccache setups, sccache also caches the native C/C++ compiles that the build runs, including the ggml CMake build.
- R5. Each setup is measured in three cache states, on hosted macOS and hosted Windows: cold (no usable cache), warm (the same commit built again on the cache that the previous run of the same setup saved), and warm after a change (a later commit with one dependency update in Cargo.lock and one workspace source change, built on the cache that the earlier commit saved).
- R6. Each setup and cache state is measured at least three times.
- R7. The Windows release build on `main` saves its cache entries. Today it cannot, because the workflow starts by `workflow_run`, and GitHub gives such runs read-only cache access unless the workflow allows writes.

**Self-hosted runners**

- R8. The self-hosted macOS and Windows runners get a local sccache disk cache that stays between jobs.
- R9. On the self-hosted runners, the experiment measures a normal PR run, a run after target/ is removed, and a run after a Cargo.lock change, each with and without the local sccache cache.

**Safety of the experiment**

- R10. No measurement run publishes a release, creates a tag, or attaches a file to a release.
- R11. No measurement run uses the `release` environment or reads its signing secrets, and the environment's main-only rule does not change. A branch measurement of `release-build` stops before signing and notarization, in the same way for every setup.
- R12. The experiment keeps the GitHub Actions cache below the repository limit and keeps the entries that `main` saved. It measures one setup at a time, deletes that setup's entries when its measurements end, and confirms before each warm run that the entry it restores exists. A warm run whose entry is missing is run again, not reported.
- R13. Nothing from the experiment merges to `main` before the user's decision (R16).
- R14. Release builds on `main` read only cache entries that `main`-branch runs wrote.

**Report and decision**

- R15. A report gives, for each job, setup and cache state, the median and range of the total job time, the time of each compile step, the cache restore and save times, the sccache hit rate, and the GitHub Actions cache size that the setup uses. For each self-hosted runner, it gives the local cache's disk use and size limit. For each job, it says how often the job runs and which cache state it normally gets in real CI, and it notes that the branch `release-build` times exclude signing and notarization. It also says what the result means for the `ephemeral-ci-runner-vms` idea.
- R16. The user decides from the report, in one discussion, to keep sccache on every job or to remove it from every job. The kept setup (rust-cache plus sccache, or sccache only) is part of that decision.
- R17. If the user keeps sccache, the final change applies it to all jobs in R1 and R8. If the user removes it, no sccache change merges, and the measured result is recorded in `docs/solutions/` so the question is not tested again without a new reason. R7 applies whatever the decision is.

### Key Decisions

- **The scope includes the self-hosted runners as well as the hosted jobs.** The user wants faster PR CI, and a local cache makes a wipe or a Cargo.lock change less costly. The expected gain on a normal PR is small. Governs R8, R9. (session-settled: user-directed — chosen over "hosted jobs only" and over "tie sccache to ephemeral VMs": the user wants every runner covered now, without the larger VM work.)
- **One decision for all jobs, made from data.** The user does not want a pass/fail threshold before the numbers exist. Governs R16, R17. (session-settled: user-directed — chosen over "keep per job when faster and not slower" and "keep with no gate": it is too early to know what should drive the decision.)
- **Compare both sccache setups, with C/C++ caching on.** This shows whether rust-cache is still needed, and it targets ggml, the largest single cold cost. Governs R2, R4. (session-settled: user-approved — chosen over "rust-cache plus sccache only" and "both setups, Rust only".)

### Scope Boundaries

- Clean VMs for each job (`docs/ideas/someday/ephemeral-ci-runner-vms.md`) are out of scope. R15 only reports what the result means for them.
- Moving the release build to a home runner (`docs/ideas/someday/home-signing-runner.md`) is out of scope.
- Link time is out of scope. sccache does not cache linking, and crates that the linker produces (bin, cdylib, proc-macro) are not cached.
- Test run time is out of scope. sccache does not change it.
- Turborepo is out of scope (rejected in `docs/ideas/next/sccache-for-hosted-builds.md`).

### Dependencies / Assumptions

- The repository is public, so hosted runner minutes cost nothing. The measurement runs can repeat as needed.
- The GitHub Actions cache limit is 10 GB for the repository. 3.2 GB was in use on 2026-10-11 (two branch rust-cache entries of 1.2 to 1.3 GB and one `main` release entry of 0.71 GB).
- GitHub scopes cache entries by branch. A run on `main` cannot restore a cache that a PR branch saved. R14 depends on this.

### Outstanding Questions

**Deferred to Planning**

- How to run `release-build` and `release-windows.yml` `build` from a branch for measurement without the publish and attach steps (R10) and without the `release` environment (R11). Today `release-build` runs only on a `main` push, and the Windows workflow's dispatch input is a release tag.
- How to make a cold run for each setup: a new cache key, removal of cache entries, or a separate sccache cache version.
- Which compile flags must change for sccache to cache the MSVC and CMake compiles (for example, the debug-information format), and whether that change has an effect on the shipped build.
- Where the local cache folder goes on each self-hosted runner, its size limit, and whether the runner image or the workflow sets it.

### Sources / Research

- `docs/ideas/next/sccache-for-hosted-builds.md`: the original idea and the Turborepo rejection.
- `docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`: the cost of ggml builds, rust-cache key behaviour (`prefix-key`, exact-hit no-save), and earlier Windows times.
- `.github/actions/windows-build-setup/action.yml`: the shared Windows setup that holds the Windows rust-cache step and the Dev Drive.
- https://xuanwo.io/en-us/reports/2023-04/: one report of up to about 50% faster warm builds than rust-cache. It is one project, not a controlled test.
