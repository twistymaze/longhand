#!/usr/bin/env bash
# Prints `changed=true` when the website changed between a base commit and
# HEAD, and `changed=false` otherwise. A push to main that changes the
# website deploys it also when no release is due (R17 of the landing page
# plan).
#
#   scripts/site-changed.sh <base commit>
#
# The website is the site/ folder and scripts/site.ts, which writes it. With
# no base (a manual run, the first push of a branch) or a base that cannot
# be fetched, it prints `changed=true`: a deploy of an unchanged site
# changes nothing.

set -euo pipefail

cd "$(dirname "$0")/.."

base="${1:-}"
if [[ -z "$base" || "$base" =~ ^0+$ ]]; then
  echo "site: no base commit, so the site deploys" >&2
  echo "changed=true"
  exit 0
fi

if ! git cat-file -e "$base^{commit}" 2>/dev/null &&
  ! git fetch --quiet --no-tags --depth=1 origin "$base" >&2; then
  echo "site: could not fetch $base, so the site deploys" >&2
  echo "changed=true"
  exit 0
fi

if [[ -n "$(git diff --name-only "$base" HEAD -- site scripts/site.ts)" ]]; then
  echo "changed=true"
else
  echo "changed=false"
fi
