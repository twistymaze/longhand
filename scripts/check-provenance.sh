#!/usr/bin/env bash
# Enforce R26 and R27 over every provenance record in the repository.
#
# Fails when a directory known to hold borrowed code has no PROVENANCE.md,
# when a record is missing a field or names a file that does not exist, when a
# record's licence text is absent, or when anything cites anarlog's
# commercially licensed enterprise/ tree.
#
#   scripts/check-provenance.sh [root]      check a tree (default: repo root)
#   scripts/check-provenance.sh --self-test prove the checks fail when they should

set -euo pipefail

# Directories that contain borrowed code. A unit that borrows into a new
# directory adds it here, so the record cannot be forgotten.
BORROWED_DIRS=(
  .github/workflows
  native/nemo-helper
  crates/aec
  crates/audio
  crates/audio-cpal
  crates/audio-sync
  crates/audio-system
  crates/asr
  crates/audio-mock
  crates/diarize
  crates/embedding
  crates/models
  crates/onnx
  crates/pipeline
  crates/power
  crates/pyannote-local
  crates/recorder
  crates/secrets
  crates/supervisor
  crates/vad
  src-tauri
  src/editor
  src/settings
  src/widget
)

FIELDS=("Repository" "Revision" "Source paths" "Copyright" "Licence" "Status")

check_record() {
  local record="$1" root="$2" dir failures=0
  dir="$(dirname "$record")"

  if grep -n 'enterprise/' "$record" >/dev/null; then
    echo "$record: cites enterprise/ (R26): $(grep -n 'enterprise/' "$record" | head -1)"
    failures=1
  fi

  # Split into sections on "## `path`" headings and validate each one.
  local sections
  sections="$(awk '/^## /{n++} n{print n "\t" $0}' "$record")"
  if [[ -z "$sections" ]]; then
    echo "$record: no records (expected one '## \`path\`' section per borrowed file)"
    return 1
  fi

  local count
  count="$(printf '%s\n' "$sections" | cut -f1 | sort -un | tail -1)"
  for ((i = 1; i <= count; i++)); do
    local body heading path
    body="$(printf '%s\n' "$sections" | awk -F'\t' -v i="$i" '$1 == i {sub(/^[0-9]+\t/, ""); print}')"
    heading="$(printf '%s\n' "$body" | head -1)"
    path="$(printf '%s\n' "$heading" | sed -n 's/^## `\(.*\)`$/\1/p')"
    if [[ -z "$path" ]]; then
      echo "$record: malformed heading: $heading"
      failures=1
      continue
    fi
    if [[ ! -e "$dir/$path" ]]; then
      echo "$record: record for $path, which does not exist"
      failures=1
    fi
    for field in "${FIELDS[@]}"; do
      if ! printf '%s\n' "$body" | grep -q "^- $field: ."; then
        echo "$record: $path is missing '$field'"
        failures=1
      fi
    done
    if ! printf '%s\n' "$body" | grep -Eq '^- Revision: [0-9a-f]{40}$'; then
      echo "$record: $path revision is not an immutable 40-character commit"
      failures=1
    fi
    if ! printf '%s\n' "$body" | grep -Eq '^- Status: (copied|modified|reference)'; then
      echo "$record: $path status must begin with copied, modified or reference"
      failures=1
    fi
    local licence_file
    licence_file="$(printf '%s\n' "$body" | sed -n 's/^- Licence: [^,]*, \(.*\)$/\1/p')"
    if [[ -z "$licence_file" || ! -f "$root/$licence_file" ]]; then
      echo "$record: $path licence text '${licence_file:-<none>}' not found"
      failures=1
    fi
  done
  return "$failures"
}

check_tree() {
  local root="$1" failures=0
  for dir in "${BORROWED_DIRS[@]}"; do
    if [[ -d "$root/$dir" && ! -f "$root/$dir/PROVENANCE.md" ]]; then
      echo "$dir: holds borrowed code but has no PROVENANCE.md (R27)"
      failures=1
    fi
  done
  while IFS= read -r record; do
    check_record "$record" "$root" || failures=1
  done < <(find "$root" -name PROVENANCE.md \
    -not -path '*/target/*' -not -path '*/node_modules/*' -not -path '*/reference-source/*' | sort)
  return "$failures"
}

self_test() {
  local tmp status=0
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  mkdir -p "$tmp/licences/sources" "$tmp/crates/audio/src"
  echo "MIT" > "$tmp/licences/sources/anarlog-MIT.txt"
  touch "$tmp/crates/audio/src/x.rs"
  write_record() {
    cat > "$tmp/crates/audio/PROVENANCE.md" <<RECORD
# Provenance

## \`src/x.rs\`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 7040ace23d1606dcc4371b5bb8c5aca1c3450044
- Source paths: $1
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, licences/sources/anarlog-MIT.txt
- Status: copied
RECORD
  }

  write_record '`crates/audio/src/lib.rs`'
  if ! check_tree "$tmp" >/dev/null; then
    echo "self-test: a valid record was rejected"; status=1
  fi

  write_record '`enterprise/x.rs`'
  if out="$(check_tree "$tmp")"; then
    echo "self-test: an enterprise/ citation passed"; status=1
  elif [[ "$out" != *"crates/audio/PROVENANCE.md"* ]]; then
    echo "self-test: the enterprise/ failure did not name the file: $out"; status=1
  fi

  rm "$tmp/crates/audio/PROVENANCE.md"
  if check_tree "$tmp" >/dev/null; then
    echo "self-test: a borrowed directory without a record passed"; status=1
  fi

  [[ $status -eq 0 ]] && echo "self-test passed"
  return "$status"
}

if [[ "${1:-}" == "--self-test" ]]; then
  self_test
  exit $?
fi

root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
if check_tree "$root"; then
  echo "provenance ok"
else
  exit 1
fi
