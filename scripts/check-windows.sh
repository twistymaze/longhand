#!/usr/bin/env bash
# Lint the workspace for Windows from macOS or Linux, before a push, so a
# Windows-only warning or cfg mistake shows up in seconds rather than in the
# Windows CI job (which runs only on pull requests into main and manual runs).
#
# Crates whose dependencies build C or C++ (bundled SQLite, ggml, ONNX
# Runtime, opus) need a Windows toolchain and are left out; CI checks them.
# If a new crate fails here for that reason, add it to EXCLUDE.
set -euo pipefail

TARGET=x86_64-pc-windows-msvc
EXCLUDE=(longhand asr recorder session-core diarize pyannote-local embedding pipeline)

if ! rustup target list --installed | grep -qx "$TARGET"; then
    rustup target add "$TARGET"
fi

args=()
for crate in "${EXCLUDE[@]}"; do
    args+=(--exclude "$crate")
done

cargo clippy --workspace "${args[@]}" --all-targets --target "$TARGET" -- -D warnings
echo "windows lint ok (excluded: ${EXCLUDE[*]})"
