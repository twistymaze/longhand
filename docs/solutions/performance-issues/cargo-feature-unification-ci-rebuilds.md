---
title: Separate cargo invocations rebuilt ggml three times per Windows CI job
date: 2026-09-24
category: performance-issues
module: CI workflow (GitHub Actions cargo steps on macOS and Windows)
problem_type: performance_issue
component: infrastructure
severity: medium
symptoms:
  - Cold Windows CI job took 71 minutes against 8 on macOS
  - ggml (transcribe-cpp-sys CMake build) compiled three times in one job, once per cargo step with a different package selection
  - The bindings check (cargo test -p longhand export_bindings) recompiled 34 crates already built
  - Tauri app test binaries failed to start on Windows with STATUS_ENTRYPOINT_NOT_FOUND, which is why the tests were split
  - rust-cache kept restoring an exact-hit cache and never saved the new artifacts
root_cause: config_error
resolution_type: config_change
framework_version: rust 1.94.0
related_components:
  - tooling
  - testing_framework
tags:
  - cargo-feature-unification
  - ci-compile-time
  - github-actions
  - windows-ci
  - rust-cache
  - dev-drive
  - larger-runners
  - tauri-windows-manifest
  - ggml
  - cargo-test
---

# Separate cargo invocations rebuilt ggml three times per Windows CI job

## Problem

A cold CI run on the Windows runner took about 71 minutes against about 8 on macOS (per this session's measurements of the pre-fix runs). The cause was not a slow machine: ggml, which `transcribe-cpp-sys` builds from source with CMake (C++ plus Vulkan shaders on Windows), was compiled **three times in one job**, because three cargo steps each selected a different set of packages and so resolved shared crates to different feature sets. Archived PR 1 (merged to main) fixed it.

## Symptoms

Per-step `Compiling` counts in the downloaded Windows job log (step times are session measurements):

| Step | Command | Time | What compiled |
|---|---|---|---|
| clippy | `cargo clippy --workspace --all-targets` | 21 min | first ggml build (expected) |
| tests (part 1) | `cargo test --workspace --exclude longhand` | 18 min | ggml **rebuilt** |
| tests (part 2) | `cargo test -p longhand --no-run` | 20 min | ggml rebuilt **a third time**, plus about 200 other crates |
| installer | `bun tauri build` | 3 min | 12 crates |

- `transcribe-cpp-sys` appeared in a `Compiling` line under more than one step. One ggml build takes 5 to 14 minutes on the runner.
- The app crate (`longhand`, `src-tauri/`) was split out with `--no-run` because its test binaries failed to start on Windows with `STATUS_ENTRYPOINT_NOT_FOUND`. A binary that links Tauri needs the Common-Controls v6 dependency declared in an application manifest, and Tauri put that manifest only into the app's resource file, so test binaries never got it.
- macOS did not show the problem: its tests were already one `cargo test --workspace`, the same selection as clippy.
- The old "Bindings are current" step ran `cargo test -p longhand export_bindings`, yet another selection. It recompiled 34 crates (about 35 s) to regenerate a file the main test run had already regenerated.

## What Didn't Work

- **The original workaround for the manifest crash (session history).** When the first Windows CI run hit `STATUS_ENTRYPOINT_NOT_FOUND`, the app crate's tests were compiled on Windows without running them and run only on macOS. That created the `--exclude longhand` / `-p longhand` split, and with it the extra ggml builds. Neither linker-embedding the manifest nor nextest was tried then.
- **A cache that never refreshed (session history).** rust-cache initially saved only on success, and every early Windows job failed or was cancelled, so every run started cold (the first fully green cold Windows run took 103 minutes). Once the key matched exactly, rust-cache treated the cache as current and never saved new artifacts, so after the app bundle moved to the debug profile every run rebuilt everything. `cache-on-failure: true` and prefix-key bumps came out of this.
- **Reordering steps** (full build first, then clippy, then tests) does not help. Clippy only type-checks (it emits metadata) and tests need codegen, so neither reuses the other's Rust artifacts in any order. What can be shared is the build-script run, and only when the resolved features match. Matching the selection is the fix, not the order.
- **cargo-nextest** was the fallback: it compiles the whole workspace in one pass and can skip the app's tests at run time. It was not needed once the manifest fix let the app's tests run.
- **Larger GitHub runners** are only available to organizations on Team or Enterprise plans, not a personal Pro account, and more cores would mostly speed up the CMake build rather than remove the duplicate builds. The repository later moved to the `twistymaze` organization, and archived PR 7 put the Windows job on an 8-core runner (see [Follow-up](#follow-up-dev-drive-8-core-runner-and-a-cache-on-main)).
- **The `custom-protocol` split** was left alone. `tauri build` turns on Tauri's `custom-protocol` feature (the app serves its pages from the bundle), which changes `tauri` and everything built on it (plugins, wry, tao, muda, the app). Per session measurements that costs about 52 crates (about 42 s) on macOS and about 40 on Windows. Matching it would mean testing in bundled mode or restructuring the workspace.
- **Downloading SPIRV-Headers directly** instead of through vcpkg would save only about 15 of 38 s, so Handy's vcpkg setup stays (`.github/workflows/ci.yml:103-115`). **A Dev Drive on Windows** was deferred to a separate, measured experiment, which archived PR 7 did (see [Follow-up](#follow-up-dev-drive-8-core-runner-and-a-cache-on-main)).

## Solution

1. **Put the manifest in every app binary** (`src-tauri/build.rs:22-44`). On MSVC, build.rs calls `tauri_build::WindowsAttributes::new_without_app_manifest()`, so Tauri stops putting the manifest into its resource file. `embed_windows_manifest()` then passes it to the linker for every binary the crate links, test binaries included:
   ```rust
   println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
   println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
   ```
   The manifest is Tauri's default, copied to `src-tauri/windows-app-manifest.xml`, and declares the Common-Controls v6 dependency. `new_without_app_manifest()` is the switch Tauri exposes for handing the manifest to the linker yourself. The app crate's unit tests now start and pass on Windows; before, they only compiled.
2. **Run one cargo test command on both OSes** (`.github/workflows/ci.yml:168-174`): `cargo test --workspace`, over the same crates as clippy (`ci.yml:157-159`, `cargo clippy --workspace --all-targets`). Checks that give the same result on every platform run on macOS only: frontend, rustfmt, clippy, bindings and provenance (each step gated `if: runner.os == 'macOS'` between `ci.yml:141` and `ci.yml:185`; the tests step in that span runs on both).
   - Clippy on the Windows runner is replaced by `scripts/check-windows.sh` running on macOS (`ci.yml:161-166`). It runs clippy for `x86_64-pc-windows-msvc` but excludes the crates that build C or C++ (`scripts/check-windows.sh:12`). Those crates' Windows-only code is compiled by the Windows tests but not linted with `-D warnings`.
3. **Drop the bindings rebuild** (`ci.yml:176-179`). `export_bindings` is an ordinary test, so `cargo test --workspace` already regenerates `src/bindings.ts`, and the step only runs `git diff --exit-code src/bindings.ts`.
4. **Build the bundles in the debug profile** (`ci.yml:187-196`, `ci.yml:221-224`). `bun tauri build --debug` reuses what the tests built; a release build recompiles every dependency (412 crates) and doubles the cache (`ci.yml:187-189`).
5. **Bump the rust-cache key** from `v1-rust` to `v2-rust` (`ci.yml:131`), with `cache-on-failure: true` (`ci.yml:132`). The key follows Cargo.lock, and an exact key hit is never saved again (`ci.yml:123-128`), so after a change to which cargo commands run, the old cache would keep being restored without the new artifacts. GitHub also keeps a pull request's cache separate from its branch's, so the first PR run was cold even though branch runs had saved caches.

Results (session measurements): a cold Windows run went from 71 to 39 minutes (tests step about 28 minutes, 25 compiling everything once and 3.5 running; installer 5; cache save 4.5). A cold macOS run takes about 12 minutes. The first warm run on the PR (run 35988462884, dependencies unchanged) took 4m17s on macOS and 14m36s on Windows.

## Why This Works

Cargo reuses a compiled unit only when its fingerprint matches, and the fingerprint covers the resolved feature set along with version, profile, target and rustflags. Features are resolved **once per cargo invocation, over the packages that invocation selects**:

- The workspace sets `resolver = "3"` (`Cargo.toml:2`), which has resolver 2's feature rules. Under them, dev-dependencies of the selected packages add their features only when the invocation builds targets that need them (`cargo test`, `--all-targets`); build-dependencies and proc-macros are unified with each other on the host, separately from normal dependencies.
- So `--workspace`, `--workspace --exclude longhand` and `-p longhand` are three feature resolutions. A crate shared by all three can get a different feature set, and so a different fingerprint and a separate build, in each. Cargo keeps the variants side by side under `target/`, so nothing is overwritten and nothing is shared.
- When the changed crate sits in a native crate's **build-dependency chain**, the build script is recompiled and rerun, and for `transcribe-cpp-sys` a rerun is the whole CMake build of ggml.

Checked in the current tree with `cargo tree` on the macOS host (Windows adds platform crates, same mechanism). `transcribe-cpp-sys`'s build-dependencies are `cmake` and `serde_json`, and the host `serde_json` features depend on the selection:

```
--workspace                    serde_json [alloc,default,std,unbounded_depth]  serde_core [alloc,default,result,std]  memchr [alloc,default,std]
--workspace --exclude longhand serde_json [default,std]                        serde_core [std]                        memchr [alloc,std]
-p longhand                    same as --workspace
```

The extra features come from `cargo_metadata`, which asks for serde_json's `unbounded_depth` and reaches the graph only through the app crate's build tooling (`tauri-build` → `tauri-utils` → `cargo_metadata`). Excluding `longhand` removes them, so `transcribe-cpp-sys`'s build script links a different `serde_json`, and ggml rebuilds. The third rebuild (`-p longhand` after `--exclude longhand`) was seen in the Windows logs but not reproduced for the Windows target locally; on the macOS host `-p longhand` and `--workspace` share the build-dependency subtree.

Other splits in the same diff cost Rust compile time rather than a C++ rebuild: `sysinfo` gets `system` from the recorder's soak-test dev-dependency (`crates/recorder/Cargo.toml:24`) while normal dependencies turn on only `disk` (`crates/recorder/Cargo.toml:12`, `src-tauri/Cargo.toml:37`); `objc2`, `tokio`, `serde`, `regex`, `url` and others also resolve differently.

Making clippy and tests select exactly `--workspace` (both activating dev-dependencies: `--all-targets` for clippy, `cargo test` by nature) gives one feature resolution per job, so ggml's build script and its build-dependencies get one fingerprint and ggml builds once. The manifest change is what made that possible: the only reason for `--exclude longhand` was that the app's test binaries could not start without it. The remaining extra compile, in the Tauri build step, is the bounded `custom-protocol` split, which does not reach ggml's build-dependency chain.

## Prevention

### Checklist before adding or changing a cargo step in `ci.yml`

- [ ] Does it select exactly the same packages as the tests step (`--workspace`)? Any `-p X`, `--exclude X`, `--features`/`--all-features`/`--no-default-features`, `--release`/`--profile`, `--target`, or different `RUSTFLAGS` compiles its own copies of shared crates and may rerun ggml's CMake build (5 to 14 minutes on Windows).
- [ ] Can the work ride along with `cargo test --workspace` instead? A generated-file check can be a test followed by `git diff --exit-code`, as `ci.yml:176-179` does.
- [ ] If it must differ, check its feature diff locally first (below), and put it in its own job with its own `prefix-key` when it is heavy. The soak job does both: `-p recorder --release` with `prefix-key: v1-soak` (`ci.yml:245-259`).
- [ ] Does a new or changed dev-dependency add features to a crate that is also a normal or build dependency? Harmless while every step uses `--workspace`, costly if any step does not.
- [ ] Bump `prefix-key` in the same commit (below).
- [ ] After the first run, count `Compiling` lines per step (below). No heavy native crate should appear under more than one step.

The same applies locally: the walking-skeleton plan's verification table runs clippy with `--all-features` (`docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md:1242`), a different resolution from `cargo test --workspace`, so running both compiles shared crates twice. CI runs clippy without it; match CI.

### Diff the resolved features of two selections locally

Arrays keep this working in zsh, which does not split words:

```sh
ftree() { cargo tree "$@" -e normal,build,dev -f '{p} [{f}]' --prefix none 2>/dev/null \
           | sed 's/ (\*)$//; s/ (proc-macro)//' | sort -u; }
a=(--workspace); b=(--workspace --exclude longhand)
diff <(ftree "${a[@]}") <(ftree "${b[@]}")
# ggml's build-dependency subtree only:
cargo tree "${b[@]}" -e normal,build -f '{p} [{f}]' | grep -A12 'transcribe-cpp-sys'
```

To see why cargo rebuilds a unit, run `CARGO_LOG=cargo::core::compiler::fingerprint=info cargo test --workspace`, which logs the reason each unit is dirty.

### Count `Compiling` lines per step in a CI job log

```sh
repo=twistymaze/longhand
run=<run-id>        # gh run list -R "$repo" --workflow ci.yml
gh run view "$run" -R "$repo" --json jobs --jq '.jobs[] | "\(.databaseId)  \(.name)"'
job=<job-id>
# Write to a file: printing the log to a terminal needs --allow-escape-sequences.
gh api "repos/$repo/actions/jobs/$job/logs" > "job-$job.log"

# Compiling lines per step, in step order
awk '
  /##\[group\]Run / { s = $0; sub(/.*##\[group\]Run /, "", s)
                      if (!(s in seen)) { seen[s] = 1; order[++n] = s } }
  / Compiling /     { c[s]++ }
  END { for (i = 1; i <= n; i++) if (c[order[i]]) printf "%6d  %s\n", c[order[i]], order[i] }
' "job-$job.log"

# Which steps compiled the heavy native crates (each should appear under one step only)
awk '
  /##\[group\]Run / { s = $0; sub(/.*##\[group\]Run /, "", s) }
  / Compiling (transcribe-cpp-sys|ort-sys|libsqlite3-sys) / { sub(/.* Compiling /, ""); print s "  ->  " $0 }
' "job-$job.log"
```

Step boundaries are GitHub's `##[group]Run <first line of the step>` markers. A healthy warm run shows few `Compiling` lines outside the workspace crates; a healthy cold run shows the bulk under `cargo test --workspace` and a bounded remainder under the Tauri build.

### When to bump the rust-cache `prefix-key`

rust-cache's automatic key follows the lockfile and toolchain, not how CI invokes cargo, and an exact hit is never saved again (`ci.yml:123-128`). Bump `prefix-key` (`v2-rust` to `v3-rust`, and so on) whenever a change alters which artifacts the job produces: adding, removing or reselecting a cargo step; changing flags, profile or target; changing feature flags. A Cargo.lock change does not need a bump. Expect the first run after a bump to be cold. A new PR restores the cache that the last run on main saved, so its first run is cold only when main has not run since the key changed.

## Follow-up: Dev Drive, 8-core runner and a cache on main

Archived PR 7 (2026-09-25) made three more changes to the Windows job and to when CI runs.

1. **A Dev Drive** (`ci.yml:73-90`). `samypr100/setup-dev-drive@v4` creates a trusted ReFS drive on a dynamic 40 GB VHDX after checkout, and copies the workspace onto it. `CARGO_HOME` and `RUSTUP_HOME` also move onto it. The steps that build use `working-directory: ${{ env.DEV_DRIVE_WORKSPACE || github.workspace }}`, and rust-cache uses the same workspace. A trusted Dev Drive also stops Defender from scanning each file. The action's post step dismounts the drive, and post steps run in reverse order, so the Dev Drive step must come before rust-cache. Otherwise rust-cache saves after the drive is gone.
2. **The org's 8-core runner** (`ci.yml:46-56`): `windows-8core`, 8 cores, 32 GB, the Windows Server 2025 image. The matrix keeps `os: windows-latest` and adds `runner: windows-8core`. The job name `Checks (windows-latest)` is a required check in the main ruleset, and a job with a new name never reports that check, so auto-merge waits for ever.
3. **CI on pushes to main**, except pushes that change only docs (`ci.yml:8-27`). A pull request cannot restore the cache of another pull request, and main had no cache, so the first run of each pull request compiled everything. Docs are every Markdown file except `PROVENANCE.md` (the provenance check reads it), plus `docs/`, `.claude/` and `.compound-engineering/`. Runs on main are not cancelled (`ci.yml:35-39`), so each one saves its cache.

Results (cold Windows runs, no cache, minutes):

| Step | Before (standard runner) | Dev Drive (standard) | Dev Drive (8-core) |
|---|---|---|---|
| Job total | 38.6 | 27.0 | 11.9 |
| `cargo test --workspace` | 29.4 | 19.7 | 8.1 |
| of which running the tests | about 3 | 2.7 | 1.6 |
| Tauri build (NSIS installer) | 5.1 | 4.1 | 2.1 |
| rust-cache save | 2.7 | 1.3 | 0.35 |

Runs 36151678746, 36176109623 and 36178741653. The Dev Drive decreased the job time by 30% at no cost. The 8-core runner decreased it by a further 56%. The installer step also decreased by half, because most of it is the Rust build of the app, not NSIS. The Dev Drive used 11.8 GB of its 40 GB. An 8-core Windows minute costs about 4 standard minutes, and the plan's included minutes do not apply to larger runners, so a cold 8-core run costs about 1.2 times the cold run before these changes. After the tests step, the longest single item is `crates/pipeline/tests/runner.rs` (75 s).

What to know:

- **The Dev Drive letter is part of the cache key.** rust-cache hashes the values of `CARGO_HOME`, `RUSTUP_HOME` and `CMAKE_PREFIX_PATH`, and the cached paths are absolute. On the standard runner the Dev Drive is `E:`, because `D:` is the runner's temp disk. The 8-core runner has one disk, so the Dev Drive is `D:`. A change of runner type thus gives one cold run. The action's `mount-path` input can mount the drive at a fixed folder, but Windows can resolve a path through a folder mount point to `\\?\Volume{GUID}\`, which CMake and the MSVC tools often do not accept. One cold run per change of runner type costs less.
- **Main cannot restore a pull request's cache.** So the first run on main after a key change is cold, even when the pull request saved a cache with the same key.
- **`rust-lld` was not used.** `src-tauri/build.rs` embeds the manifest with `/MANIFEST:EMBED /MANIFESTINPUT`. `rust-lld` is built without libxml2, and so it may not merge a manifest input. Test it in its own change if link time becomes a large part of a warm run.
- **Already done before archived PR 7:** debug-profile bundles, NSIS as the only bundle, and `CARGO_INCREMENTAL=0` (rust-cache sets it). A next change to measure is `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, which makes the PDBs smaller and linking faster.

## Follow-up: self-hosted runners

Since October 2026 the macOS and Windows checks run on self-hosted runners that keep `target/` between jobs ([Checks and CI](../../development-workflow.md#checks-and-ci)). rust-cache, the Dev Drive step and the 8-core runner apply only when a job runs on GitHub's runners. The `ci.yml` line numbers in this document refer to the file at the time of archived PR 1 and archived PR 7. The feature-unification rule still applies: one set of cargo commands per job keeps each self-hosted runner's `target/` to one build of ggml.

## Related Issues

- Archived PR 1: the walking skeleton, including the manifest embedding and CI restructure.
- Archived PR 7: the Dev Drive, the 8-core runner and CI on pushes to main.
- `docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`: U2 created `ci.yml` and `src-tauri/build.rs`; its file map (line 567) still describes the pre-fix CI layout (historical record).
