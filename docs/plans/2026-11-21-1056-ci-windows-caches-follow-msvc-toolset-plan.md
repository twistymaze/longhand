---
title: Windows Caches Follow the MSVC Toolset - Plan
type: ci
date: 2026-11-21
topic: windows-caches-follow-msvc-toolset
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-plan-bootstrap
execution: code
---

# Windows Caches Follow the MSVC Toolset - Plan

## Goal Capsule

- **Objective:** An update of GitHub's Windows runner image does not make a pull request fail in a step that the PR did not change. Agent commands that run outside this checkout are not blocked by Longhand's package-manager rule.
- **Means:** The NeMo cache key and the rust-cache key contain the MSVC toolset version (KTD1, KTD2). The two CMake cache guards also configure a build folder again when it names tools of another MSVC version (KTD3). The `agent-tool-choice` hook rule denies bun, bunx, yarn and pnpm only when the command runs inside this checkout (KTD4).
- **Authority:** The Product Contract wins on behaviour. The KTDs win on mechanism.
- **Stop conditions:** Stop and ask the user when the MSVC toolset version cannot be read before the rust-cache step on a hosted runner, or when the self-hosted Windows path needs a change other than the guard.
- **Execution profile:** One PR with a title of type `ci` that names both changes (no release). Units are committed in order on one branch in a worktree from `origin/main`.
- **Who finishes:** `ce-work` implements and checks locally, the PR's own Windows CI job is the first real proof, then `ce-babysit-pr` until the PR is ready to merge. After the merge, PR #104 is rebased on `main`.
- **Open blockers:** None.

---

## Product Contract

### Summary

The Windows CI job reuses CMake build folders from two caches: the NeMo cache (`.github/workflows/ci.yml`) and the rust-cache of `target/` (`.github/actions/windows-build-setup/action.yml`). Neither key names the MSVC toolset, so a cache that one runner image saved is restored on later images. The guards that check a restored folder find only a tool path that is missing. This plan puts the toolset version in both keys and makes both guards reject a folder from another toolset. It also corrects the hook rule from PR #103 that blocks `bun` in every repository.

### Problem Frame

On 2026-11-21 PR #104 (dependency updates only) failed in "Build and stage NeMo helper" on image win25-vs2026/20261116.315 (MSVC 14.52.37025). The step restored the NeMo cache that a push to `main` saved on 2026-11-20 on image 20261103.298 (MSVC 14.51.36231). CMake's compiler test called the 14.51 `link.exe`, which the new image does not have. A PR run on the new image the day before passed only because it had a cache miss. PR #103 added `CMAKE_LINKER`, `CMAKE_AR` and `CMAKE_MT` to the NeMo guard, but the guard still accepts a folder whose old tools are installed next to the new ones, and the key still lets every later image restore an old cache. PR #104's branch does not have PR #103's guard, so a rebase alone fixes the failure that was seen. This plan adds what that guard does not do: keys that do not match a cache from another toolset, and a guard that also rejects a folder whose old tools are still installed. The same PR #103 added the hook rule `agent-tool-choice`, which denies every `bun` command, also in another repository that needs bun.

### Requirements

- R1. On a hosted Windows job, an exact key hit of the NeMo cache or the rust-cache comes only from a job with the same MSVC toolset version. A cache from another toolset that is restored anyway (the rust-cache's fallback restore keys) is rebuilt by R2.
- R2. A restored CMake build folder whose compiler, linker, ar or mt path is under another MSVC toolset folder than the current one is configured again, also when those tools exist. This applies to `scripts/build-nemo.sh` and to the sccache setup action, which keep the same check. The keys cannot cover three cases that this guard covers: a rust-cache fallback restore, a build folder that stays between jobs (self-hosted runner, a local Windows build), and a job whose actual toolset differs from the default version that the key read.
- R3. A machine without an MSVC environment (macOS, Linux, a Windows shell without vcvars) keeps today's guard behaviour.
- R4. The rule `agent-tool-choice` denies bun, bunx, yarn and pnpm when the command runs inside this checkout, and allows them when the command clearly runs outside it. When the folder is not known, the rule denies.
- R5. The code, tests and docs name no private repository.
- R6. A learning in `docs/solutions/` records the cause and the prevention.

### Scope Boundaries

- No change to the macOS caches, the self-hosted runner flow, or the other Windows CI steps.
- No change to the Python, pip and prettier parts of `agent-tool-choice`.
- Considered and not built: a key on the runner image version (`ImageVersion`). Each weekly image update would make a cold NeMo and CMake build, also when MSVC did not change. The MSVC key plus the guard covers the failure. Evidence that would change this: a failure from an image change that keeps the MSVC version.
- Considered and not built: a step that fails the job when `VCToolsVersion` after `msvc-dev-cmd` differs from the version in the key. The guard (R2) already rebuilds such folders, so a mismatch costs only build time.

---

## Planning Contract

### Key Technical Decisions

- KTD1. The toolset version comes from the Visual Studio install, read before rust-cache. A new step in `windows-build-setup` reads the default toolset version (`VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt` under the path that `vswhere` gives), the same version that `ilammy/msvc-dev-cmd` selects, and exports it as `LONGHAND_MSVC_TOOLSET`. rust-cache runs before `msvc-dev-cmd`, so `VCToolsVersion` does not exist yet at that step. (session-settled: user-approved — chosen over the runner image version: an image update that keeps MSVC keeps the warm cache.)
- KTD2. Both keys use `LONGHAND_MSVC_TOOLSET`. The rust-cache `key:` adds it next to the toolchain-file hash. The NeMo restore key in `ci.yml` adds it after the OS. The save step reuses the restore step's primary key, so it needs no change.
- KTD3. The guard compares the MSVC version folder in each cached tool path with `$VCToolsVersion`, the toolset that `msvc-dev-cmd` actually set up. Both guards run after `msvc-dev-cmd`, so the variable is set on Windows CI; the guard does not use `LONGHAND_MSVC_TOOLSET`, which is only the key's prediction. A tool path that contains `/VC/Tools/MSVC/<version>/` (case-insensitive, either slash) with a different `<version>` makes `missing_input` return true. When `VCToolsVersion` is not set, the check does nothing (R3). The two copies stay the same, and `scripts/cmake-cache-guards.test.ts` tests both.
- KTD4. The hook tracks the working folder of each segment. `decide()` starts from the hook input's `cwd` and follows a segment that is `cd <dir>` (absolute, `~`, or relative to the current folder). Each rule predicate gets the folder as a second argument. The bun/yarn/pnpm rule denies when the folder is inside the checkout root (two levels above `.claude/hooks/`) or not known: no `cwd`, or a `cd` to a variable, `-` or a command substitution. "Inside" means the folder equals the root, or the relative path from the root to the folder does not start with `..` and is not absolute, so a sibling folder whose name starts with the checkout's name is outside. The rule compares real paths: the root and each known folder go through `realpath` when they exist, and a folder that does not exist uses the real path of its nearest existing parent (macOS `/tmp` and `/var` are symbolic links). (session-settled: user-approved — chosen over a separate PR: the rule came from the same lesson set and blocks this upkeep work now.)

### Implementation Units

### U1. Toolset version in the cache keys

**Goal:** R1.
**Dependencies:** none.
**Files:** `.github/actions/windows-build-setup/action.yml`, `.github/workflows/ci.yml`.
**Approach:**
1. Add a step before rust-cache that reads the default MSVC toolset version with `vswhere` and writes it to `GITHUB_ENV`. Fail the step when the file is not found on a hosted runner, because a key without the version would bring back the defect.
2. Add the variable to the rust-cache `key:`.
3. Add it to the NeMo restore key, and update the comment above the restore step.
**Patterns to follow:** the existing "Locate the VC++ runtime" step in the same action.
**Test expectation:** none in unit tests -- CI configuration. The PR's Windows job shows the new key in the restore log (a cache miss on the first run).
**Verification:** the restore and rust-cache log lines show a key with `14.52.37025` or the image's current version.

### U2. Guards reject another toolset

**Goal:** R2, R3.
**Dependencies:** none.
**Files:** `scripts/build-nemo.sh`, `.github/actions/sccache-setup/action.yml`, `scripts/cmake-cache-guards.test.ts`, `rules/cmake-cache-guards.toml`.
**Approach:** extend `missing_input` in both files as KTD3 says, and update the comments and the rule text in `rules/cmake-cache-guards.toml`. Keep the negative example of the rule passing.
**Test scenarios:**
- With `VCToolsVersion=14.52.1`, a cache where one of `CMAKE_C_COMPILER`, `CMAKE_CXX_COMPILER`, `CMAKE_LINKER`, `CMAKE_AR` or `CMAKE_MT` is an existing file under `VC/Tools/MSVC/14.51.1/bin/` is stale, for each key, in both files.
- With `VCToolsVersion=14.52.1`, a cache whose tools are existing files under `VC/Tools/MSVC/14.52.1/` is kept.
- With `VCToolsVersion` unset, the same 14.51 cache with existing files is kept (R3).
- A Windows-style path with backslashes and mixed case (`Msvc`) is matched.
- The existing missing-tool scenarios still pass.
**Verification:** `node --test scripts/cmake-cache-guards.test.ts` passes, and the rule's negative example still fails as expected.

### U3. Hook rule follows the working folder

**Goal:** R4, R5.
**Dependencies:** none.
**Files:** `.claude/hooks/rules.mjs`, `.claude/hooks/rules.test.mjs`, `rules/agent-tool-choice.toml`.
**Approach:** implement KTD4. Other rules ignore the second argument. Update the rule text in `rules/agent-tool-choice.toml` to say "inside this checkout".
**Test scenarios:**
- The test helper copies the hook to `<tmp>/.claude/hooks/rules.mjs`, so the temporary folder is the checkout root of the copy.
- `bun install` with `cwd` inside the checkout is denied, also when `cwd` names the root through a symbolic link (`/var/...` for `/private/var/...`).
- `bun install` with `cwd` set to a sibling folder named `<root name>-other` is allowed.
- `bun install` with `cwd` outside the checkout is allowed.
- `cd /elsewhere && bun test` from inside the checkout is allowed, and `cd /elsewhere && cd <checkout> && bun x` is denied.
- An absolute bun path (`/Users/x/.bun/bin/bun scripts/a.ts`) with `cwd` outside is allowed.
- `cd "$DIR" && bun x`, and input with no `cwd`, are denied.
- `cd ../sibling && pnpm i` from the checkout root is allowed; `cd src && pnpm i` is denied.
- `python3` and `prettier` stay denied outside the checkout.
**Verification:** `node --test .claude/hooks/rules.test.mjs` passes.

### U4. Learning

**Goal:** R6.
**Dependencies:** U1, U2.
**Files:** a new `docs/solutions/build-errors/windows-cache-from-another-msvc-toolset.md`; a short link from `docs/solutions/performance-issues/rust-cache-dropped-cmake-toolchain-hash-wiped-warm-windows-builds.md`.
**Approach:** record the symptom (a missing 14.51 `link.exe` in a CMake compiler test), the cause (keys without the toolset), why the missing-path guard was not enough, and the fix. Name the CI runs by number, and no private repository.
**Test expectation:** none -- documentation.

### Verification Contract

| Gate | When |
|---|---|
| `node --test scripts/cmake-cache-guards.test.ts .claude/hooks/rules.test.mjs scripts/check-rules.test.ts` | after U2 and U3 |
| `node scripts/check-rules.ts` (runs the negative example of each rule) | before push |
| The PR's Windows CI job passes with a NeMo cache miss on the new key | after push |

### Risks & Dependencies

- The first `main` push after the merge has a cold NeMo build and a cold rust-cache on Windows (one time). Acceptable.
- If `vswhere` reports more than one Visual Studio install, the step must take the one that `msvc-dev-cmd` uses (the latest). The implementer checks this against `msvc-dev-cmd`'s behaviour.

### Definition of Done

- U1 to U4 are committed, and the local gates pass.
- The PR's Windows job passes, and its log shows the toolset version in both keys.
- After the merge, PR #104 is rebased on `main` and its Windows job passes. This item shows only that nothing regressed: PR #103's guard alone already fixes the failure that PR #104 had. The proof of this plan is the toolset version in the key logs (U1) and the U2 tests.

### Sources / Research

- Failed job: twistymaze/longhand Actions run 44922525635 (PR #104). Saving run: 44666459894 (2026-11-20, image 20261103.298). Passing PR run with a cache miss: 44789807015.
- PR #102 and PR #103 (guard keys), `docs/solutions/performance-issues/rust-cache-dropped-cmake-toolchain-hash-wiped-warm-windows-builds.md`, `docs/solutions/build-errors/windows-dev-drive-letter-split-pr-and-main-caches.md`.
