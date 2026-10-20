#!/usr/bin/env bash
# Prints `changed=true` when a licence input changed between a base commit
# and HEAD, and `changed=false` otherwise. CI runs the full licence check
# (cargo-about) only on `changed=true` (R12).
#
#   scripts/licence-inputs-changed.sh <base commit>
#
# A licence input is a file that can change licences/generated/macos.txt or
# windows.txt: a Cargo manifest or lock file, the npm manifest or lock file,
# the cargo-about configuration, the licences folder, the generator, or the
# CI workflow. An interface source change can only add a bundled package,
# and the frontend build then writes a different js-packages.json, which the
# drift check finds without cargo-about.
#
# With no base (a manual run, the first push of a branch) or a base that
# cannot be fetched, it prints `changed=true`.

set -euo pipefail

cd "$(dirname "$0")/.."

base="${1:-}"
if [[ -z "$base" || "$base" =~ ^0+$ ]]; then
  echo "licence inputs: no base commit, so the full check runs" >&2
  echo "changed=true"
  exit 0
fi

if ! git cat-file -e "$base^{commit}" 2>/dev/null &&
  ! git fetch --quiet --no-tags --depth=1 origin "$base" >&2; then
  echo "licence inputs: could not fetch $base, so the full check runs" >&2
  echo "changed=true"
  exit 0
fi

inputs="$(git diff --name-only "$base" HEAD -- \
  Cargo.lock '*Cargo.toml' about.toml \
  package.json package-lock.json \
  'licences/**' 'scripts/licences/**' scripts/check-licences.sh scripts/licence-inputs-changed.sh \
  .github/workflows/ci.yml)"

if [[ -n "$inputs" ]]; then
  echo "licence inputs changed:" >&2
  printf '%s\n' "$inputs" | sed 's/^/  /' >&2
  echo "changed=true"
else
  echo "licence inputs: no change, so only js-packages.json is checked" >&2
  echo "changed=false"
fi
