#!/usr/bin/env bash
# Check the release markers in a PR title (docs/releases.md).
#
# The PR title workflow runs this after the semantic PR title action, which
# checks the type. This script checks what the action cannot:
#   - `+` (a minor release) is valid only on `feat`, and not with `!`.
#   - `!` (a major release) is valid only on a type that makes a release.
#
#   PR_TITLE='feat(capture)+: record system audio' scripts/check-pr-title.sh
#
# The title comes in an environment variable, so the shell never runs it.
# The marker grammar and the types that make no release are also in
# scripts/release-version.ts and cliff.toml. Change them together.

set -euo pipefail

title="${PR_TITLE:?PR_TITLE is not set}"
failed=0

marker='^([a-z]+)(\([^)]*\))?(\+?)(!?):'
if [[ "$title" =~ $marker ]]; then
  type="${BASH_REMATCH[1]}"
  plus="${BASH_REMATCH[3]}"
  bang="${BASH_REMATCH[4]}"

  if [[ -n "$plus" && "$type" != "feat" ]]; then
    echo "PR title: \`+\` (a minor release) is valid only on \`feat\`, not on \`$type\`." >&2
    failed=1
  fi
  if [[ -n "$plus" && -n "$bang" ]]; then
    echo "PR title: use \`+\` (minor) or \`!\` (major), not both." >&2
    failed=1
  fi
  if [[ -n "$bang" ]]; then
    case "$type" in
      docs | ci | test | chore | style)
        echo "PR title: \`!\` (a major release) is not valid on \`$type\`, because \`$type\` makes no release." >&2
        failed=1
        ;;
    esac
  fi
fi

exit "$failed"
