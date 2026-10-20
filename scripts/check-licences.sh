#!/usr/bin/env bash
# Enforce R12: the stored full licence list of each shipped platform matches
# what that platform ships.
#
# Runs the frontend build (which writes licences/generated/js-packages.json),
# then the licence generator, then fails when any file in licences/generated/
# differs from the committed version. A dependency change without a new list
# fails here with the name of the stale file.
#
#   scripts/check-licences.sh              check the repository
#   scripts/check-licences.sh --self-test  run the generator tests and prove the drift check fails when it should
#
# CI options, which can go together:
#   --skip-build    do not run `npm ci` and `npm run build`: an earlier step ran them
#   --no-generate   do not run the generator: only compare the js-packages.json
#                   that the build wrote (scripts/licence-inputs-changed.sh)
#
# Needs cargo-about 0.9.2 (`cargo install cargo-about --locked --version 0.9.2`)
# and npm 12 or later.

set -euo pipefail

GENERATED_DIR=licences/generated

# Prints each changed or new file under the generated folder and fails when
# there is one. $1 is the root of a git work tree.
check_drift() {
  local root="$1" changes
  changes="$(git -C "$root" status --porcelain --untracked-files=all -- "$GENERATED_DIR")"
  if [[ -n "$changes" ]]; then
    echo "The full licence list is stale. Run scripts/check-licences.sh and commit these files:"
    printf '%s\n' "$changes" | sed 's/^.. /  /'
    return 1
  fi
}

self_test() {
  local status=0 tmp out
  node --test "scripts/licences/**/*.test.ts" || status=1

  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  git -C "$tmp" init -q
  mkdir -p "$tmp/$GENERATED_DIR"
  echo "serde 1.0.0" > "$tmp/$GENERATED_DIR/macos.txt"
  git -C "$tmp" add -A
  git -C "$tmp" -c user.name=self-test -c user.email=self-test@invalid commit -qm "list"

  if ! check_drift "$tmp" >/dev/null; then
    echo "self-test: a current list was reported as stale"; status=1
  fi

  echo "tokio 1.0.0" >> "$tmp/$GENERATED_DIR/macos.txt"
  if out="$(check_drift "$tmp")"; then
    echo "self-test: a stale list passed"; status=1
  elif [[ "$out" != *"$GENERATED_DIR/macos.txt"* ]]; then
    echo "self-test: the stale-list failure did not name the file: $out"; status=1
  fi

  git -C "$tmp" checkout -q -- "$GENERATED_DIR"
  echo "list" > "$tmp/$GENERATED_DIR/windows.txt"
  if check_drift "$tmp" >/dev/null; then
    echo "self-test: a new, uncommitted list passed"; status=1
  fi

  [[ $status -eq 0 ]] && echo "self-test passed"
  return "$status"
}

cd "$(dirname "$0")/.."

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

skip_build=false
generate=true
for arg in "$@"; do
  case "$arg" in
    --skip-build) skip_build=true ;;
    --no-generate) generate=false ;;
    *) echo "unknown option: $arg"; exit 1 ;;
  esac
done

if [[ "$generate" == false ]]; then
  [[ "$skip_build" == true ]] || npm run build
  if check_drift .; then
    echo "licences ok (js-packages.json only)"
    exit 0
  fi
  exit 1
fi

if ! command -v cargo-about >/dev/null; then
  echo "cargo-about is not installed: cargo install cargo-about --locked --version 0.9.2"
  exit 1
fi

# An npm older than 10.9 does not read devEngines in package.json, so this
# check stops an npm older than 12 before it installs a package.
npm_version="$(npm --version)"
if [[ "${npm_version%%.*}" -lt 12 ]]; then
  echo "npm $npm_version is older than 12. Install npm 12: npm install --global npm@12"
  exit 1
fi

# npm ci installs exactly the versions in package-lock.json. A copied or old
# node_modules would write wrong versions into js-packages.json.
if [[ "$skip_build" == false ]]; then
  npm ci
  npm run build
fi
node scripts/licences/generate.ts
if check_drift .; then
  echo "licences ok"
else
  exit 1
fi
