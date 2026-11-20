#!/usr/bin/env bash
# Run the dependency-age check on the working tree against HEAD, at a fixed
# time two days after left-pad 1.3.0 was published. The negative example
# adds that version to package-lock.json, so it is always too young here.
set -euo pipefail
base=$(mktemp -d)
trap 'rm -rf "$base"' EXIT
git show HEAD:Cargo.lock > "$base/Cargo.lock"
git show HEAD:package-lock.json > "$base/package-lock.json"
node scripts/check-dependency-age.ts "$base" . --now 2018-04-11T00:00:00Z
