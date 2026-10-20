---
title: Move the JavaScript packages from bun to npm
description: "Use npm and package-lock.json instead of bun.lock, so that GitHub Dependabot sees every JavaScript package"
area: distribution
added: 2026-10-18
source: "conversation about the dependency upkeep and GitHub security features (2026-10-18)"
plan: docs/plans/2026-10-19-2254-chore-bun-to-npm-plan.md
---

# Move the JavaScript packages from bun to npm

## Idea

Use npm, with `package-lock.json`, as the package manager for the interface packages instead of bun and `bun.lock`.

## Why

The GitHub dependency graph does not read `bun.lock`. It reads only `package.json`, so it knows only the packages that Longhand names directly, not the packages that they bring in. Dependabot therefore does not report an advisory for most JavaScript packages. For example, `bun audit` reports GHSA-68fv-2mgg-jv7q in `source-map-js` (Vite brings it in), and Dependabot does not. The dependency graph reads `package-lock.json`, so after the move Dependabot reports all JavaScript packages, as it does for the crates in `Cargo.lock`.

Longhand uses bun because Handy uses it (KTD2 of the walking skeleton plan), not because of a decision about bun itself. The user does not accept the current state for the long term. Until the move, the JavaScript packages are not in the Dependabot checks on purpose; the weekly `bun audit` of the upkeep routine reports their advisories.

## Notes / Current state

bun does more than install packages. Each of these needs a replacement or a decision:

- **Package installs and `bun.lock`:** `package-lock.json`. Change `bun install --frozen-lockfile` to `npm ci` in CI, `scripts/check-licences.sh` and the upkeep routine.
- **The 7-day minimum release age** (`minimumReleaseAge` in `bunfig.toml`): npm 11.10 and later has `min-release-age`, in days. The CI age check reads the lockfile too.
- **Frontend tests** (`bun test`, the happy-dom preload in `bunfig.toml`): for example Vitest, which works with Vite.
- **Repository scripts** (`bun scripts/*.ts`): some use bun functions (`Bun.spawnSync`, `Bun.file`). Node 22 and later can run TypeScript directly; change the bun functions to Node functions. A script can also stay on bun when it uses no packages.
- **Tauri commands** in `src-tauri/tauri.conf.json` (`beforeDevCommand`, `beforeBuildCommand`) and the CI setup steps (`setup-bun`, the self-hosted bun cache rule).
- **The agent rule** in `CLAUDE.md` ("JavaScript packages only through `bun`") and `docs/development-workflow.md`.
- **The upkeep routine:** `bun audit` and `bun outdated` become `npm audit` and `npm outdated`.

pnpm is the other choice: GitHub reads `pnpm-lock.yaml`, and anarlog uses pnpm. npm 11.10 and later also has a minimum release age (`min-release-age`, in days), so the plan chose npm.

## Outcome

Shipped in [PR 55](https://github.com/twistymaze/longhand/pull/55) on 2026-10-20. Longhand uses npm 12 and `package-lock.json`, with no Bun left. `.npmrc` sets the 7-day minimum release age and refuses install scripts, git packages and remote packages. `devEngines` stops an install with an older npm. The frontend tests run on Vitest, and the repository scripts run on Node with small readers for versions, TOML and frontmatter. The cargo deny job keeps a fallback for a base without `package-lock.json`, until no open PR has a base from before PR 55.
