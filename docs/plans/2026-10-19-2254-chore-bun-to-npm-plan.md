---
title: Bun to npm - Plan
type: chore
date: 2026-10-19
topic: bun-to-npm
artifact_contract: ce-unified-plan/v1
product_contract_source: ce-brainstorm
execution: code
---

# Bun to npm - Plan

## Goal Capsule

- **Objective:** GitHub Dependabot reports advisories for all JavaScript packages of Longhand, including the packages that other packages bring in, and the project needs only one JavaScript tool.
- **Means:** Remove Bun completely. npm 12 installs the packages, Vitest runs the frontend tests, and Node runs the repository scripts. The weekly upkeep routine changes to match.
- **Product authority:** The maintainer. The shipped app does not change.
- **Open blockers:** None.

---

## Product Contract

### Summary

Longhand stops using Bun. npm 12 installs the JavaScript packages and writes `package-lock.json`, with a 7-day minimum release age and no dependency install scripts. Vitest runs the frontend tests, and Node runs the repository scripts and their tests. Small functions in the repository replace the Bun functions for version comparison, TOML and YAML, so that the jobs that run without a package install stay that way. The upkeep routine in the private upkeep repository treats Longhand as an npm project after the Longhand change merges.

### Problem Frame

The GitHub dependency graph does not read `bun.lock`. It reads only `package.json`, so it knows only the packages that Longhand names directly. Dependabot therefore does not report advisories for most JavaScript packages. For example, `bun audit` reports GHSA-68fv-2mgg-jv7q in `source-map-js`, which Vite brings in, and Dependabot does not. The weekly upkeep routine runs `bun audit` to fill this gap, but that is a temporary measure that the maintainer does not accept for the long term (`docs/ideas/next/move-javascript-packages-to-npm.md`).

Longhand uses Bun because Handy uses it (KTD2 of the walking skeleton plan), not because of a decision about Bun. Bun now does four jobs: it installs packages, it holds back package versions for 7 days, it runs the tests, and it runs the repository scripts with Bun-only functions. The Bun setup in CI also caused failures on the self-hosted Windows runner (`docs/solutions/build-errors/setup-bun-ebusy-on-shared-windows-runner.md`).

### Key Decisions

- **Remove Bun completely, not only the package manager.** One JavaScript tool in CI, local work and agent rules; the Bun setup failures on Windows stop. Governs R1, R9, R11, R17. (session-settled: user-directed — chosen over npm for packages only, and over npm plus Vitest with the scripts left on Bun: both keep two JavaScript tools in CI)
- **npm, not pnpm.** npm is the standard tool and comes with Node. pnpm's main extra advantage, a built-in minimum release age, now exists in npm (`min-release-age`, npm 11.10 and later). anarlog uses pnpm because it has a large workspace; Longhand has one `package.json`. Governs R1. (session-settled: user-approved — chosen over pnpm: no remaining advantage that matters for one package)
- **npm 12 is the minimum, and a guard stops on an older npm.** npm 12 does not run dependency install scripts by default, as Bun does now. npm 11 ignores those settings and runs every install script, so an old npm would reduce the protection without a warning. Governs R5, R7, R8, R21. (session-settled: user-directed — the user required npm 12 for its install-script defaults)
- **Own code for version comparison, TOML and YAML.** Node has no built-in functions for these. A package would make the publish jobs and the check job need a package install, which the releases plan forbids for the publish jobs (R25 of `docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md`). Governs R12, R13. (session-settled: user-approved — chosen over npm packages for semver, TOML and YAML: they would need `npm ci` in jobs that can write to the repository)
- **The upkeep scripts stay on Bun.** Only the way that the upkeep routine handles Longhand changes. Governs R19, R22. (session-settled: user-directed — chosen over moving the upkeep scripts to Node)
- **The upkeep changes are part of this work and come after the Longhand PR merges.** No upkeep run happens between the merge and the upkeep commits, so the plan does not guard that window. Governs R23. (session-settled: user-directed — the user put the upkeep sync in scope and set the order)
- **One PR with a `chore` title and no release.** The shipped app does not change, so users get no update notice. Governs R24.

### Requirements

**Packages**

- R1. npm installs the JavaScript packages, and `package-lock.json` is the only JavaScript lockfile. `bun.lock` and `bunfig.toml` are removed.
- R2. CI, `scripts/check-licences.sh` and the upkeep routine install with `npm ci`, which fails when `package-lock.json` does not match `package.json`.
- R3. npm takes a package version only when it is at least 7 days old, from a setting in the repository. To add an exact younger version that `min-age-exceptions.toml` lists, the lockfile update may exempt that one package from npm's age filter; the repository setting stays at 7 days.
- R4. The CI dependency-age check (`scripts/check-dependency-age.ts`) reads `package-lock.json` in place of `bun.lock`. Its rules do not change: `min-age-exceptions.toml` allows one exact version with a reason, and an entry that does not come from the npm registry fails unless the exceptions file lists it. An entry comes from the npm registry only when its download URL is the registry.npmjs.org tarball of the name and version that the check looks up, it has an integrity hash, and it is not an alias for another package.
- R5. No dependency install script runs unless the repository approves that package. Each approval has a reason. At the start no package is approved, unless a package does not work without its script.
- R6. npm refuses git and remote-URL dependencies, as the npm 12 defaults do.

**npm version**

- R7. Each place that runs npm stops with a clear message before it installs a package when npm is older than 12. This includes CI, the local npm scripts and `scripts/check-licences.sh`. In local work, npm itself makes the check before it installs, for example with the `devEngines` package-manager field, because npm runs the project's own install scripts only after it installs the dependencies.
- R8. One place in the repository names the Node version and the npm version, and CI reads them from there.

**Tests**

- R9. Vitest runs the frontend tests in `src/`. The happy-dom page is registered before each test file, as `bunfig.toml` does now.
- R10. Node runs the tests of the repository scripts.
- R11. All tests that pass with Bun pass after the move. A changed test waits for the event that it checks, not for a delay.

**Repository scripts**

- R12. Node runs each script in `scripts/`. No script and no test uses a Bun function, the Bun-only `import.meta.dir`, or `bun:test`.
- R13. Version comparison, TOML reading (`Cargo.lock` and `min-age-exceptions.toml`) and YAML frontmatter reading (`docs/ideas`) are functions in the repository, with tests. They use no npm package.
- R14. The publish jobs and the base-checkout check job still run no package install.
- R15. A file that a PR adds or changes cannot change how the base-checkout check job runs its scripts. This replaces the `bunfig.toml` protection (`docs/solutions/security-issues/pr-bunfig-controls-base-check-scripts.md`). It includes the tools that the scripts start: the advisory check audits the head lockfile with the base npm settings only, so a `.npmrc` that the PR adds or changes has no effect.
- R16. On the PR that makes the move, the base has only the Bun check scripts, which read `bun.lock`. For that one PR, the base-checkout check job runs the head's Node scripts against `package-lock.json`, and the PR description records this. Every later PR follows R15.

**Tools, docs and agent rules**

- R17. No current file names Bun as a tool of the project: CI workflows, composite actions, the Tauri build commands, shell scripts, `package.json`, `tsconfig.json`, `README.md`, `CLAUDE.md`, `AGENTS.md`, `docs/development-workflow.md`, `licences/README.md`, `native/nemo-helper/README.md` and `scripts/icons/`. Earlier plans and solutions stay as they are.
- R18. The agent rule says "JavaScript packages only through npm".

**Upkeep routine** (private upkeep repository)

- R19. The upkeep routine handles Longhand as an npm project. It reads `package-lock.json` for alerts, the malware stop, the update selection, the age exceptions and the PR checks. It uses the npm commands for update, audit and outdated, and `npm ci` for installs.
- R20. The upkeep PR check refuses a change to `.npmrc` in place of `bunfig.toml`. It also refuses a change to the install-script approvals (R5) and to the Node and npm versions (R8), wherever planning puts them.
- R21. The upkeep routine and its setup script stop when npm is older than 12.
- R22. The upkeep routine runs Longhand's own scripts with Node. The upkeep scripts themselves continue to run on Bun.
- R23. The upkeep changes are direct commits on the upkeep `main`, made after the Longhand PR merges. They do not change `scripts/items.ts` or its tests, which another session will change next.

**Delivery**

- R24. The Longhand change is one PR with a `chore` title.

### Acceptance Examples

- AE1. **Covers R7.** **Given** a CI runner whose npm is 11. **When** the job reaches its first npm step. **Then** the job stops with a message that names the required npm version, and no package is installed.
- AE2. **Covers R4.** **Given** a PR that adds a package version that the npm registry published 3 days ago. **When** the dependency-age check runs. **Then** the check fails, unless `min-age-exceptions.toml` lists that exact version with a reason.
- AE3. **Covers R5.** **Given** a PR that adds a package with a `postinstall` script. **When** `npm ci` runs. **Then** the script does not run. When the package needs the script, the PR approves the package with a reason.
- AE4. **Covers R20.** **Given** an upkeep PR that changes `.npmrc`. **When** the upkeep PR check runs. **Then** the check refuses the PR.
- AE5. **Covers R15.** **Given** a PR whose `.npmrc` sets the registry to another host and whose lockfile adds a version with an advisory. **When** the advisory check runs. **Then** the check fails.
- AE6. **Covers R4.** **Given** a PR whose lockfile keeps an old name and version but downloads it from another host. **When** the dependency-age check runs. **Then** the check fails, unless the exceptions file lists that entry.

### Success Criteria

- After the merge, the GitHub dependency graph of `twistymaze/longhand` lists JavaScript packages that Longhand does not name directly, for example `source-map-js` at the version in `package-lock.json`. When that version has a current advisory, Dependabot shows its alert.
- A new clone builds, tests and bundles the app with Node and npm, without Bun installed.
- CI passes on the macOS and Windows runners, including the release build and the installed-helper smoke test.
- A signed macOS build works as before at the manual test step.
- The first upkeep run after the upkeep commits completes without a "Failed:" notice that comes from the move.

### Scope Boundaries

- Dependabot version-update PRs (`dependabot.yml`) are out of scope. The upkeep routine selects updates.
- The upkeep scripts do not move to Node.
- The upkeep item issues (`scripts/items.ts`) do not change.
- Earlier plans and solutions keep their Bun text as history.
- A JavaScript formatter or linter is not added.

### Dependencies / Assumptions

- npm 12 is available for the macOS and Windows CI runners, the maintainer's Mac and the upkeep Mac.
- esbuild works without its `postinstall` script. Of the installed top-level packages, only esbuild has an install script. Nested packages were not checked.
- Vitest gives an equivalent for each `bun:test` function in use: `test`, `it`, `describe`, `expect`, the hooks, `mock`, `spyOn` and `jest` (4 files, probably fake timers). One frontend test (`src/settings/About.test.tsx`) also reads a file with `Bun.file`.
- The GitHub dependency graph reads `package-lock.json`.

### Outstanding Questions

**Deferred to Planning**

- Which Node version to use. A Node version that ships npm 12 needs no separate npm install step. A global npm install on the self-hosted Windows runner can fail when a process uses the file, as `setup-bun` did.
- Whether npm 12 fails or only warns on an unapproved install script, and whether the strict setting is necessary for R5.
- Which npm setting makes the local version check of R7 before the install: `devEngines` with a failure on mismatch, or `engines` with `engine-strict`.
- Which npm commands replace `bun update --lockfile-only --ignore-scripts --minimum-release-age`, `bun audit` and `bun outdated` in the upkeep update selection, and how their JSON output differs.
- How the base-checkout check job meets R15 when it runs scripts with Node.
- How much TOML the own reader must support. `Cargo.lock` and `min-age-exceptions.toml` use only tables, arrays of tables, strings, arrays of strings and integers (`Cargo.lock` starts with `version = 4`).

### Sources / Research

- `docs/ideas/next/move-javascript-packages-to-npm.md`: the idea and its list of Bun uses.
- `docs/plans/2026-10-11-1323-ci-releases-in-source-repository-plan.md`: R25, no build code in the publish jobs.
- `docs/solutions/security-issues/pr-bunfig-controls-base-check-scripts.md` and `docs/solutions/build-errors/setup-bun-ebusy-on-shared-windows-runner.md`.
- `reference-source/anarlog`: pnpm 11 with `allowBuilds`, Vitest for app packages, and Node `.mjs` scripts with `node --test`.
- Bun functions in use: `Bun.spawn`, `Bun.spawnSync`, `Bun.file`, `Bun.write`, `Bun.CryptoHasher`, `Bun.sleep`, `Bun.which`, `Bun.semver` (`check-new-advisories.ts`, `publish-release.ts`, `release-version.ts`), `Bun.TOML` (`lockfiles.ts`, `check-dependency-age.ts`) and `Bun.YAML` (`ideas.ts`). The Bun-only `import.meta.dir` is in 9 scripts, 4 script tests and `src/settings/About.test.tsx`; in Node it is undefined, with no error.
- npm `min-release-age`: npm 11.10.0 and later, in days (https://dev.classmethod.jp/articles/npm-v11-min-release-age/).
- npm 12.0.0 (July 2026): dependency install scripts, git dependencies and remote-URL dependencies are off by default; `npm approve-scripts` records approvals (https://www.infoq.com/news/2026/08/npm-12-released/).
- Upkeep repository: `ROUTINE.md`, `README.md`, `bootstrap.sh`, `scripts/select-updates.ts`, `scripts/alerts.ts`, `scripts/check-pr.ts`, `scripts/exceptions.ts`, `scripts/sensitive-paths.json`.
