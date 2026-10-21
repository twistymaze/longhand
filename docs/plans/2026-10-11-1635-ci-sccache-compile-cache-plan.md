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

- **Objective:** Release builds on `main`, and PR checks after a cache loss, take less time when the compile cache is cold, and warm builds do not become much slower. The user decides from measured data whether sccache stays.
- **Means:** A first-approximation sccache experiment on the two hosted release builds and the self-hosted runners, then one keep-or-remove decision (KTD1, KTD2).
- **Product authority:** The user makes the keep-or-remove decision (R16). This plan does not set a threshold.
- **Execution profile:** `ce-work` in a worktree from `origin/main`. U1 ships as its own PR. U2 to U4 stay on the experiment branch, which does not merge before the decision (R13).
- **Stop conditions:** Stop and report when a measurement run would need the `release` environment or its secrets (R11), when the Windows installer smoke test fails under the Ninja generator (KTD4), or when the cache use would pass 10 GB (R12).
- **Who finishes:** The agent finishes U1 to U4 and U6, and presents the report. U5 starts only after the user's decision.
- **Open blockers:** None.

---

## Product Contract

**Product Contract preservation:** changed: R1, R2, R5, R6, R7, R9, R15. The user narrowed the measurement to a first approximation: the hosted checks and repeat runs are dropped, existing run times are the baseline, cold runs decide, and one warm run is a guard. R7 states the cause that research found. Added: R18 to R20. The first warm runs showed that the release profile's last step (full LTO with one codegen unit) takes 8 to 11 min, which sccache cannot cache, so the user added the release profile to the experiment.

### Summary

Run sccache on the two hosted release builds and give the self-hosted runners a local sccache cache. Measure each sccache setup once cold and once warm, and compare it with today's measured times. Report the result. The user then keeps sccache everywhere or removes it everywhere.

### Problem Frame

CI is slow when the compile cache misses. On 2026-10-11, the hosted runs (`hosted=true` dispatch, run 38172180403) took 36 min on Windows and 23.5 min on macOS. Windows found no cache and compiled 526 crates. macOS had a full rust-cache hit but still compiled 185 crates. The macOS release build ("Preview build", run 38155311321) took 24 min. A cold ggml CMake build alone takes 5 to 14 min on the Windows runner (`docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`).

The self-hosted PR checks are fast (6.7 min macOS, 4.7 min Windows, run 38170140849) because target/ stays on the runner between jobs. On macOS, the test runs take 158 s of that time, and the rebuilds of changed workspace crates take most of the remaining compile time. sccache cannot skip those rebuilds. When target/ is removed, or a Cargo.lock change invalidates it, these runners compile everything again.

The first Windows release build (run 38176960753) failed after 2.5 min and saved a 177 MB partial cache under the exact rust-cache key. rust-cache does not save again after an exact key hit, so each later Windows release build with the same Cargo.lock restores that partial cache and compiles almost everything.

### Requirements

**Hosted release builds**

- R1. The experiment covers the two hosted release builds: `release-build` in `.github/workflows/ci.yml` (macOS) and `build` in `.github/workflows/release-windows.yml` (Windows).
- R2. The experiment compares two sccache setups, rust-cache plus sccache and sccache without rust-cache, with today's rust-cache times from runs that already exist (R6).
- R3. Both sccache setups use sccache's GitHub Actions cache backend.
- R4. In both sccache setups, sccache also caches the native C/C++ compiles that the build runs, including the ggml CMake build.
- R5. Each sccache setup is measured once cold (no usable cache) on each release build. It is also measured once warm after a change (a later commit with one dependency update in Cargo.lock and one workspace source change, built on the cache that the earlier commit saved), to show that warm builds do not become much slower.
- R6. The baseline times are the existing runs: run 38155311321 for the macOS release build, and run 38178105848 for the Windows release build. If the Windows run does not succeed, the hosted Windows checks run 38172180403 is the Windows baseline, and the report says that it is a debug build.
- R7. A failed release build does not leave a cache that later release builds on `main` keep restoring. The next successful release build saves a complete cache. This applies to both release builds.

**Self-hosted runners**

- R8. The self-hosted macOS and Windows runners get a local sccache disk cache that stays between jobs.
- R9. On the self-hosted runners, the experiment measures a switch between two Cargo.lock states, once with and once without the local sccache cache. (Reduced on 2026-10-11: the normal PR run and the run after target/ is removed are not measured. sccache cannot skip the rebuild of a changed workspace crate, and a target/ removal is rare.)

**Safety of the experiment**

- R10. No measurement run publishes a release, creates a tag, or attaches a file to a release.
- R11. No measurement run uses the `release` environment or reads its signing secrets, and the environment's main-only rule does not change. A branch measurement of a release build stops before signing and notarization, in the same way for every setup.
- R12. The experiment keeps the GitHub Actions cache below the repository limit and keeps the entries that `main` saved. It measures one setup at a time, deletes that setup's entries when its measurements end, and confirms before each warm run that the entry it restores exists. A warm run whose entry is missing is run again, not reported.
- R13. Nothing from the experiment merges to `main` before the user's decision (R16).
- R14. Release builds on `main` read only cache entries that `main`-branch runs wrote.

**Release profile**

- R18. The experiment measures two release profile options next to today's profile (`lto = true`, `codegen-units = 1`) on both release builds: option B, `lto = "thin"` with `codegen-units = 16`, and option C, `lto = false` with `codegen-units = 16` (Rust's default release profile).
- R19. For each option, the report gives the cold and warm compile step times, the time of the last step (from the last `Compiling` line to `Finished`), the size of the app binary and of the DMG or installer, and the result of the installed-helper smoke test. Runtime speed is not measured: the user accepts a loss of a few percent, and most of the heavy work runs in C/C++ and in the NeMo helper, which the Rust LTO setting does not change. The report says that it was not measured.
- R20. The release profile decision is separate from the sccache decision (R16), and is made in the same discussion. A profile change goes to `main` in its own PR, because it changes the shipped app.

**Report and decision**

- R15. A report gives, for each release build and setup, the cold and warm job times next to the baseline (R6), the time of each compile step, the cache restore and save times, the sccache hit rate, the cache write errors, and the GitHub Actions cache size that the setup uses. For each self-hosted runner, it gives the measured times, the local cache's disk use and size limit, and the effect of the weekly VM reset on the cache. It notes that the branch release-build times exclude release signing and notarization. It also says what the result means for the `ephemeral-ci-runner-vms` idea.
- R16. The user decides from the report, in one discussion, to keep sccache on every job or to remove it from every job. The kept setup (rust-cache plus sccache, or sccache only) is part of that decision.
- R17. If the user keeps sccache, the final change applies it to all jobs in R1 and R8 and to the hosted `checks` matrix. If the user removes it, no sccache change merges, and the measured result is recorded in `docs/solutions/` so the question is not tested again without a new reason. R7 applies whatever the decision is.

### Key Decisions

- **The scope includes the self-hosted runners as well as the hosted jobs.** The user wants faster PR CI, and a local cache makes a wipe or a Cargo.lock change less costly. The expected gain on a normal PR is small. Governs R8, R9. (session-settled: user-directed — chosen over "hosted jobs only" and over "tie sccache to ephemeral VMs": the user wants every runner covered now, without the larger VM work.)
- **One decision for all jobs, made from data.** The user does not want a pass/fail threshold before the numbers exist. Governs R16, R17. (session-settled: user-directed — chosen over "keep per job when faster and not slower" and "keep with no gate": it is too early to know what should drive the decision.)
- **Compare both sccache setups, with C/C++ caching on.** This shows whether rust-cache is still needed, and it targets ggml, the largest single cold cost. Governs R2, R4. (session-settled: user-approved — chosen over "rust-cache plus sccache only" and "both setups, Rust only".)
- **The release profile is part of the experiment, and only the two fast options are measured.** The user does not need the last few percent of runtime speed, so the cautious option (`lto = "thin"` with `codegen-units = 1`) is not measured. Governs R18, R19, R20. (session-settled: user-directed — the user asked to include the profile change, and said that a few percent of runtime speed is not important to them.)
- **A first approximation: cold runs decide, one warm run is a guard, existing runs are the baseline, no repeats.** The hosted checks seldom run, because PR checks use the self-hosted runners. Governs R1, R2, R5, R6. (session-settled: user-directed — chosen over a full matrix of 4 jobs, 3 setups, 3 cache states and 3 repeats, about 108 runs: the user wants a first approximation, not a controlled study.)

### Scope Boundaries

- The hosted `checks` matrix (`hosted=true`) is not measured. A keep still applies to it (R17), because those jobs use the same setup action.
- Clean VMs for each job (`docs/ideas/done/ephemeral-ci-runner-vms.md`) are out of scope. R15 only reports what the result means for them.
- A local cache that survives the weekly VM reset is out of scope. It needs a separate disk in the private runner setup. R15 reports the effect.
- Moving the release build to a home runner (`docs/ideas/done/home-signing-runner.md`) is out of scope.
- Link time is out of scope, except for the LTO setting of the release profile (R18). The choice of linker is out of scope. sccache does not cache linking, and crates that the linker produces (bin, cdylib, proc-macro) are not cached. The Vulkan shader compiles (`glslc`) are not cached either.
- Test run time is out of scope. sccache does not change it.
- Turborepo is out of scope (rejected in `docs/ideas/done/sccache-for-hosted-builds.md`).
- Considered and not built: a retry layer for lost sccache cache writes (GitHub limits cache uploads to 200 a minute, and sccache 0.17 drops a write that gets HTTP 429). R15 reports the write errors instead. A retry layer would be worth it only if the write errors turn out to decide the result.
- Considered and not built: a script that collects the measurements. The report is one table from `gh run view` step times and the uploaded sccache stats, so a script would cost more than it saves. A second experiment would change this.

### Dependencies / Assumptions

- The repository is public, so hosted runner minutes cost nothing.
- The GitHub Actions cache limit is 10 GB for the repository. On 2026-10-11 at about 22:00, 4.14 GB was in use: `main` had the macOS release entry (0.72 GB), a stale `v1-preview` entry from the old preview job (0.72 GB) and the partial Windows release entry (0.19 GB). The merged PR branch `worktree-releases-in-source-repository` had two `v3-rust-checks` entries (2.5 GB together).
- GitHub scopes cache entries by branch. A run on `main` cannot restore a cache that a PR branch saved. R14 depends on this.
- A `workflow_run` run of `release-windows.yml` can write the cache in this repository: run 38176960753 logged `Cache mode: write` and saved its entry under `refs/heads/main`.

### Sources / Research

- `docs/ideas/done/sccache-for-hosted-builds.md`: the original idea and the Turborepo rejection.
- `docs/solutions/performance-issues/cargo-feature-unification-ci-rebuilds.md`: the cost of ggml builds, rust-cache key behaviour (`prefix-key`, exact-hit no-save), earlier Windows times, and the per-step timing recipe.
- `.github/actions/windows-build-setup/action.yml`: the shared Windows setup that holds the Windows rust-cache step and the Dev Drive.
- https://xuanwo.io/en-us/reports/2023-04/: one report of up to about 50% faster warm builds than rust-cache. It is one project, not a controlled test.

---

## Planning Contract

### Key Technical Decisions

- KTD1. **The hosted measurements run as experiment jobs in `ci.yml` on the experiment branch, started by new `workflow_dispatch` inputs.** GitHub dispatches only a workflow that is registered on the default branch, so a new workflow file that stays on the branch cannot be started. `ci.yml` is registered, and `gh workflow run ci.yml --ref <branch>` uses the branch's copy of the file with its inputs. When the experiment input is set, only the experiment jobs run. The experiment jobs build the branch head with the same compile commands as the two release builds, with ad-hoc signing (`APPLE_SIGNING_IDENTITY: "-"`) and no updater files on macOS, and an NSIS build on Windows. They have no `environment`, only `contents: read`, and no find, attach or publish job. Their job names differ from the required checks. The existing paths cannot be used: a `release-windows.yml` dispatch with a real tag builds that tag's commit and attaches to the real release, and `release-build` needs the `release` environment. Governs R1, R10, R11, R13.
- KTD2. **sccache comes from `mozilla-actions/sccache-action` v0.0.11, pinned by commit SHA, with sccache v0.17.0.** v0.18.0 can panic at startup with "no CA certificates loaded" (sccache issue #2869). `RUSTC_WRAPPER=sccache`, and each setup, OS and cold run gets its own `SCCACHE_GHA_VERSION`, which namespaces the cache and makes a cold run. Each job sets `--zero-stats` at the start and uploads `sccache --show-stats --stats-format json` at the end. Governs R3, R15.
- KTD3. **"sccache only" keeps rust-cache with `cache-targets: false`.** rust-cache then caches only the crate registry and git checkouts, and sccache caches all compiled output. Without it, every job downloads all crates again, and the comparison measures download time. "rust-cache plus sccache" keeps rust-cache as today, with a new prefix key. Governs R2. (session-settled: user-approved — chosen over a literal "no rust-cache step at all": the comparison must measure compile time, not crate downloads.)
- KTD4. **C/C++ goes through sccache with `CMAKE_C_COMPILER_LAUNCHER` and `CMAKE_CXX_COMPILER_LAUNCHER`, and on Windows with `CMAKE_GENERATOR=Ninja`.** cmake-rs does not pass `RUSTC_WRAPPER` to CMake, and the Visual Studio generator that cmake-rs picks on Windows ignores compiler launchers. The cc-rs crates (bundled SQLite, ring and others) need only `RUSTC_WRAPPER`. ggml stays a CMake `Release` configuration, which has no `/Zi`, so its MSVC objects are cacheable. The Ninja generator changes how the shipped Windows build is made, so the installer smoke test must pass in each sccache setup. Governs R4. (session-settled: user-approved — the cost of the Ninja generator was shown, and the user confirmed the scope with it.)
- KTD5. **A cold run uses a new `SCCACHE_GHA_VERSION` and a new rust-cache prefix key. A warm run builds a second commit on the experiment branch.** That commit changes one dependency patch version in Cargo.lock and one workspace source file, and it runs after the cold run of the same setup saved its cache. Governs R5.
- KTD6. **The cache budget is managed by hand with `gh cache list` and `gh cache delete`.** Before the first run, U3 deletes the two entries of the merged PR branch (about 2.5 GB). It keeps every `main` entry, including the unused `v1-preview` entry (0.72 GB), and counts it in the budget. After each setup's runs, it deletes that setup's entries. Governs R12.
- KTD7. **Release builds stop saving the rust-cache on failure, and their prefix keys change.** `cache-on-failure: true` exists so that a check job that never passed still gets a cache. A release build runs only after the checks pass, so a failed release build has no such reason. New prefix keys (`v2-release`, `v2-release-windows`) leave the partial entry unused. Governs R7.
- KTD8. **The self-hosted local cache is a folder in the runner's tool cache, set by a workflow step.** The step sets `SCCACHE_DIR` to `$RUNNER_TOOL_CACHE/sccache` unless the runner environment file already sets it, and it sets `SCCACHE_CACHE_SIZE` to 20G and `SCCACHE_BASEDIRS` to the work folder and Cargo home. This follows the `LONGHAND_NEMO_CACHE` fallback in `ci.yml`, so the experiment needs no change in the private runner repository. A new `sccache` input on the `ci.yml` dispatch turns it on, so normal PR runs do not change. Each runner has its own folder, because the sccache local cache supports one server for each folder. Governs R8, R9.

- KTD9. **The profile options are set by environment variables from a dispatch input, not by edits to `Cargo.toml`.** A `release-profile` input (`current`, `thin-16`, `off-16`) sets `CARGO_PROFILE_RELEASE_LTO` and `CARGO_PROFILE_RELEASE_CODEGEN_UNITS` in both experiment jobs, so the measured commits stay the same as in U3. The codegen-unit count is part of the rustc arguments of every crate, and `lto = false` also changes how the dependencies embed bitcode, so each option has no sccache hits from another option and needs its own cold run. The runs use the sccache setup that U3 measured as faster, and a cache version for each option. Governs R18, R19.

### Assumptions

- The CMake launcher variables take effect only when CMake configures a build folder for the first time. On a self-hosted runner, the "after target/ is removed" run (R9) configures from scratch. The "after a Cargo.lock change" run may reuse folders that were configured without the launcher, so its C/C++ result can understate sccache.
- sccache's GitHub Actions writes can fail under the 200 uploads a minute limit. U3 runs the macOS and Windows jobs one after the other, not in parallel, so that they do not share the limit.

### Sequencing

U1 is independent and ships first as its own PR. U2 to U4 are the experiment on one branch: U2 builds the workflow, U3 runs the hosted measurements, and U4 adds and measures the self-hosted cache. U6 follows U3 on the same branch. U3, U4 and U6 can otherwise run in any order. The report follows U3, U4 and U6. U5 waits for the user's decision.

---

## Implementation Units

### U1. Release builds keep a complete rust-cache

**Goal:** A failed release build no longer leaves a partial cache that later release builds restore.

**Requirements:** R7, R17.

**Dependencies:** None.

**Files:**
- `.github/workflows/ci.yml` (the `release-build` rust-cache step)
- `.github/workflows/release-windows.yml` (the setup action's prefix key)
- `.github/actions/windows-build-setup/action.yml` (a new input for `cache-on-failure`, default `true`)

**Approach:**
1. In `release-build`, set `cache-on-failure: false` and change the prefix key to `v2-release` (KTD7).
2. Add a `cache-on-failure` input to the Windows setup action, so that the checks keep `true` and `release-windows.yml` passes `false` with the prefix key `v2-release-windows`.
3. Update the comments that explain the cache rules, in ASD-STE100.
4. After the PR merges and a release build succeeds, delete the old `v1-release-windows-build` entry if it is still listed.

**Patterns to follow:** the existing rust-cache comment blocks in `ci.yml` and `action.yml`; full-SHA action pins.

**Test expectation:** none -- workflow configuration only. The proof is the next release run (Verification).

**Verification:** `actionlint` passes. After the next successful release build on `main`, `gh cache list` shows a `v2-release` or `v2-release-windows` entry of full size (hundreds of MB, not 177 MB).

### U2. Experiment workflow and sccache setup

**Goal:** One manual dispatch of `ci.yml` on the experiment branch can build either release configuration on the branch head with either sccache setup.

**Requirements:** R1, R2, R3, R4, R10, R11, R13, R14.

**Dependencies:** None.

**Files:**
- `.github/workflows/ci.yml` (new `workflow_dispatch` inputs, two experiment jobs, and an `if:` on the other jobs so that they skip when the experiment input is set)
- `.github/actions/sccache-setup/action.yml` (new composite action, shared by both experiment jobs and later by U5)
- `.github/actions/windows-build-setup/action.yml` (a `sccache-setup` input that selects the rust-cache mode and the prefix key)

**Approach:**
1. Dispatch inputs: `experiment-os` (none, macos or windows; none is the default and keeps normal CI), `experiment-setup` (`rust-cache-sccache` or `sccache-only`) and `cache-version` (the `SCCACHE_GHA_VERSION` suffix, KTD5).
2. The macOS job mirrors the `release-build` compile: the release config file without `createUpdaterArtifacts`, `APPLE_SIGNING_IDENTITY: "-"`, and `bun tauri build --bundles app,dmg`. It does ad-hoc signing only, with no secrets, release signing, notarization or publish steps (R11).
3. The Windows job mirrors the `release-windows.yml` build: the Windows setup action, then `bun tauri build --bundles nsis` in pwsh, then the installer smoke steps from the checks job.
4. The sccache setup action installs sccache (KTD2) and sets `RUSTC_WRAPPER`, `SCCACHE_GHA_VERSION`, the CMake launcher variables, and on Windows `CMAKE_GENERATOR=Ninja` (KTD4). Set env through `$GITHUB_ENV`, not job-level `env`, where it depends on `runner.*`.
5. On Windows, the sccache step runs after the Dev Drive step, so its post step runs before the drive is removed.
6. Both jobs zero the stats at the start, and they upload the JSON stats and write them to the job summary at the end.
7. The experiment jobs have `permissions: contents: read` and no `environment`. The existing `ci-${{ github.ref }}` concurrency group runs them one at a time on the branch.

**Patterns to follow:** `release-build` and `release-windows.yml` `build` for the compile steps; the `checks` job for ad-hoc signing (`APPLE_SIGNING_IDENTITY: "-"`) and the installer smoke steps; `shell: pwsh` for every Windows Rust step (`docs/solutions/build-errors/windows-rust-selects-git-linker-after-msvc-setup.md`); explicit `shell: bash` for pipefail (`docs/solutions/workflow-issues/default-run-shell-hides-pipe-failures.md`); runner paths through `$GITHUB_ENV` (`docs/solutions/workflow-issues/actions-context-not-available-fails-before-jobs.md`).

**Test expectation:** none -- workflow configuration only. A smoke run proves it (Verification).

**Verification:** `actionlint` passes. `gh workflow run ci.yml --ref <branch>` with the experiment inputs is accepted, and only the experiment job runs. One smoke dispatch of each OS job builds the app, the Windows installer smoke steps pass under Ninja, the sccache stats show C/C++ and Rust requests, and no step reads a secret.

### U3. Hosted measurements

**Goal:** One cold and one warm run of each sccache setup on each release build, with the data that the report needs.

**Requirements:** R5, R6, R12, R15.

**Dependencies:** U2.

**Files:** none in the repository (a measurement commit on the experiment branch changes `Cargo.lock` and one workspace source file, KTD5).

**Approach:**
1. Delete the merged branch's cache entries (KTD6) and record the cache use.
2. For each setup, one at a time, with the branch head at the base commit: dispatch a cold run on macOS, then on Windows, each with a new cache version. Push the warm-change commit, confirm with `gh cache list` that the setup's entries exist and record how many sccache entries the cold run stored, then dispatch the warm runs. Record the step times, stats and cache size, then delete the setup's entries.
3. Revert the warm-change commit before the next setup's cold runs, and push the same change again for its warm runs, so both setups build the same two commits. The branch ends on the reverted state.
4. Choose a warm-change dependency that is not in the build graph of `transcribe-cpp-sys`, so that its CMake build script does not run again for a reason other than sccache.
5. The report notes that the Windows baseline used the Visual Studio generator, and the sccache runs use Ninja (KTD4), so part of a Windows difference can come from the generator.

**Test expectation:** none -- measurement only.

**Verification:** Eight completed runs (2 setups × 2 OS × cold and warm), each with uploaded stats. The cache use stayed under 10 GB, and the `main` entries are still listed.

### U4. Self-hosted local cache and measurements

**Goal:** The self-hosted runners can use a local sccache cache, and its effect is measured.

**Requirements:** R8, R9, R15.

**Dependencies:** U2 (the sccache setup action).

**Files:**
- `.github/workflows/ci.yml` (a `sccache` input on `workflow_dispatch`, and a step that calls the sccache setup action in local-cache mode on self-hosted runners)
- `.github/actions/sccache-setup/action.yml` (a local-cache mode, KTD8)

**Approach:** (reduced at the user's request on 2026-10-11: only the Cargo.lock change is measured, and only Rust goes through sccache.)
1. When the `sccache` input is true and the runner is self-hosted, the step sets the local cache variables (KTD8) and calls the sccache setup action without the GitHub Actions backend. The local mode sets only `RUSTC_WRAPPER`: no CMake launcher and no Ninja generator, because the runners keep CMake build folders that the Visual Studio generator made, and a Cargo.lock change that does not touch the ggml build graph does not run CMake.
2. Two branches hold the two Cargo.lock states: a base branch with the U4 workflow, and a second branch with the warm-change commit (KTD5) on top. The local cache has no branch scope, so both branches use it. Each dispatch runs the `checks` matrix on both self-hosted runners.
3. Run in this order, one dispatch at a time: off on the base branch (to set target/ to the base state, not measured), off on the change branch, off on the base branch, on on the change branch, on on the base branch (these two fill the cache), on on the change branch, on on the base branch. The off pair and the last on pair are the measurement: each is a switch between two Cargo.lock states. Cargo can rebuild everything when `RUSTC_WRAPPER` changes, so the first on run can be a full build.
4. Record the step times, the stats and the folder size with its limit.

**Patterns to follow:** the `LONGHAND_NEMO_CACHE` fallback in the `checks` job; the `runner.environment == 'github-hosted'` gates.

**Test expectation:** none -- workflow configuration and measurement.

**Verification:** Seven completed dispatches, each on both runners, each with stats when sccache is on. A normal PR run without the input behaves as before.

### U6. Release profile measurements

**Goal:** One cold and one warm run of each release profile option on each release build, with the data that the report needs.

**Requirements:** R18, R19, R20.

**Dependencies:** U2, U3 (U3 selects the sccache setup and leaves the branch on the reverted state).

**Files:**
- `.github/workflows/ci.yml` (a `release-profile` dispatch input; both experiment jobs set the two profile variables from it, and write the sizes of the app binary and of the DMG or installer to the job summary)

**Approach:**
1. When the input is `current`, the jobs set no profile variable, so the U3 runs stay valid as the comparison.
2. For each option, one at a time: with the branch head at the reverted state, dispatch a cold run on macOS, then on Windows, each with a new cache version. Push the warm-change commit (KTD5) again and dispatch the warm runs with the same cache version. Revert the commit before the next option, and delete the option's cache entries when its runs end (KTD6).
3. Take the time of the last step from the job log: the time between the last `Compiling` line and the `Finished` line.

**Patterns to follow:** U2's experiment jobs; the sccache stats step for the job summary.

**Test expectation:** none -- workflow configuration and measurement.

**Verification:** Eight completed runs (2 options × 2 OS × cold and warm), each with stats and sizes, and the installed-helper smoke tests pass. A dispatch without the input gives the same build as before.

### U5. Apply the decision

**Goal:** CI matches the user's decision, and the result is recorded.

**Requirements:** R13, R16, R17, R20.

**Dependencies:** U3, U4, U6, the report, and the user's decision.

**Files:**
- Keep: `.github/workflows/ci.yml` (`release-build`, `checks`, and removal of the experiment jobs and inputs), `.github/workflows/release-windows.yml`, `.github/actions/windows-build-setup/action.yml`, `.github/actions/sccache-setup/action.yml`.
- Either way: `docs/solutions/performance-issues/sccache-compile-cache-experiment.md` (new), committed on `main` after the decision as a docs-only change.

**Approach:**
1. Keep: apply the chosen setup to the release builds and the checks, hosted and self-hosted, without the dispatch input. The release builds use sccache in read-write mode only on `main` (R14).
2. Remove: close the experiment branch without a merge, and remove the local cache folders from the self-hosted runners.
3. Either way: write the solution doc from the report, and move the idea file with `bun scripts/ideas.ts move`.
4. If the user chooses a release profile option, change `[profile.release]` in `Cargo.toml` in its own PR (R20), separate from the sccache change.

**Test expectation:** none -- workflow configuration and a docs record.

**Verification:** Keep: the PR checks pass, and the next release builds on `main` show sccache stats. Remove: no sccache setting is left in the workflows on `main` or in the self-hosted runners' tool caches. The solution doc is on `main`.

---

## Verification Contract

| Check | Applies to | Done signal |
|---|---|---|
| `actionlint` on the changed workflows | U1, U2, U4, U5 | No findings |
| Local checks in `docs/development-workflow.md#checks-and-ci` | U1, U5 | Pass before each push |
| PR CI | U1, U5 | Green on the PR |
| `bun scripts/ideas.ts check` | U5 | No problems |
| Grep of staged files for home-network details (`docs/solutions/workflow-issues/keep-home-network-details-out-of-the-repository.md`) | U4, U5, report | Nothing found |

## Definition of Done

- U1 merged, and a later release build on `main` saved a complete cache.
- The report (R15) is in front of the user with all eight hosted runs and fourteen self-hosted runs, or it names the runs that failed and why.
- The user's decision is applied by U5, and the solution doc is on `main`.
- No experiment job, experiment input or measurement commit is on `main`. No abandoned attempt code is left in the diff.
