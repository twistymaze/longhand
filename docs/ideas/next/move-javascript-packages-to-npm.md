---
title: Move the JavaScript packages from bun to npm
description: "Use npm and package-lock.json instead of bun.lock, so that GitHub Dependabot sees every JavaScript package"
area: distribution
added: 2026-10-18
source: "conversation about the dependency upkeep and GitHub security features (2026-10-18)"
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
- **The 7-day minimum release age** (`minimumReleaseAge` in `bunfig.toml`): npm has no rolling age setting, only `--before <date>`. Find how to keep the 7-day wait, for example a check of `package-lock.json` like the planned check of `Cargo.lock`.
- **Frontend tests** (`bun test`, the happy-dom preload in `bunfig.toml`): for example Vitest, which works with Vite.
- **Repository scripts** (`bun scripts/*.ts`): some use bun functions (`Bun.spawnSync`, `Bun.file`). Node 22 and later can run TypeScript directly; change the bun functions to Node functions. A script can also stay on bun when it uses no packages.
- **Tauri commands** in `src-tauri/tauri.conf.json` (`beforeDevCommand`, `beforeBuildCommand`) and the CI setup steps (`setup-bun`, the self-hosted bun cache rule).
- **The agent rule** in `CLAUDE.md` ("JavaScript packages only through `bun`") and `docs/development-workflow.md`.
- **The upkeep routine:** `bun audit` and `bun outdated` become `npm audit` and `npm outdated`.

pnpm is the other choice: GitHub reads `pnpm-lock.yaml`, and pnpm has a `minimumReleaseAge` setting. anarlog uses pnpm.
