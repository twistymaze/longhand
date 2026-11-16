---
title: Bundled-package entries in package-lock.json fail the dependency age check
date: 2026-11-15
module: dependency age check
problem_type: workflow_issue
component: development_workflow
severity: medium
applies_when:
  - "A dependency update changes package-lock.json"
  - "scripts/check-dependency-age.ts reports a package as not from the npm registry"
symptoms:
  - "The age check reported packages under @tailwindcss/oxide-wasm32-wasi as not from the npm registry."
  - "The failed entries had \"inBundle\": true and no resolved or integrity value."
root_cause: config_error
resolution_type: dependency_update
tags:
  - npm
  - package-lock
  - dependency-age
  - min-age
  - tailwindcss
---

# Bundled-package entries in package-lock.json fail the dependency age check

## Problem

During the dependency update in PR #95, `npm install` with npm 12 wrote entries with `"inBundle": true` into `package-lock.json`. These entries are the packages that `@tailwindcss/oxide-wasm32-wasi` bundles in its own tarball. They have no `resolved` URL and no `integrity` value.

`scripts/check-dependency-age.ts` accepts a package-lock.json entry as a registry entry only when its `resolved` URL is the registry.npmjs.org tarball of its name and version and it has an `integrity` value (see the comment at the top of the script). So the check failed each bundled entry with "not from the npm registry", and told us to add it to `min-age-exceptions.toml`.

## Root cause

The bundled entries are not real dependency sources. They come in the tarball of their parent package, and the parent has a normal registry entry that the check verifies. `main` did not have these entries, so the check saw them as new versions from an unknown source.

## Solution

Do not add exceptions for bundled entries. Remove them from the lockfile and resolve it again:

1. Remove each `node_modules/@tailwindcss/oxide-wasm32-wasi/node_modules/...` entry that has `"inBundle": true` from `package-lock.json`.
2. Run `npm install --package-lock-only`. In PR #95, this did not write the entries again.
3. Make sure that the entries are gone (`grep -c inBundle package-lock.json` gives 0), then run the age check again.

## Prevention

After each npm update, search `package-lock.json` for `inBundle` before you run the age check. If the entries come back, remove them as above. Add an exception to `min-age-exceptions.toml` only for a real dependency that cannot wait 7 days.
