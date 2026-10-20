#!/usr/bin/env bash
# Build the pinned SDK and the app-owned helper. Run in an MSVC environment on Windows.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache="${LONGHAND_NEMO_CACHE:-$root/target/nemo}"
source_dir="${LONGHAND_NEMO_SOURCE:-$cache/source}"
backend="${LONGHAND_NEMO_BACKEND:-cpu}"
case "$backend" in cpu|metal|vulkan) ;; *) echo 'Unsupported NeMo backend' >&2; exit 2 ;; esac
sdk_build="${LONGHAND_NEMO_BUILD:-$source_dir/build/$backend-asr}"
helper_build="${LONGHAND_NEMO_HELPER_BUILD:-$cache/helper-$backend}"
tokenizer_source="${LONGHAND_SENTENCEPIECE_SOURCE:-$cache/sentencepiece}"
tokenizer_build="${LONGHAND_SENTENCEPIECE_BUILD:-$cache/sentencepiece-build}"
mkdir -p "$cache"
if [[ ! -e "$source_dir/.git" ]]; then
  git clone --no-checkout https://github.com/NVIDIA/NeMo-Speech.cpp.git "$source_dir"
  git -C "$source_dir" checkout --detach 97a15afa5caa9bce5baaa86c1184103877af4101
  git -C "$source_dir" submodule update --init ggml llama.cpp
fi
[[ "$(git -C "$source_dir" rev-parse HEAD)" == 97a15afa5caa9bce5baaa86c1184103877af4101 ]] || { echo 'NeMo revision mismatch' >&2; exit 2; }
[[ "$(git -C "$source_dir/ggml" rev-parse HEAD)" == c03b4e2bcece5134827881af90242086daf75be5 ]] || { echo 'ggml revision mismatch' >&2; exit 2; }
[[ "$(git -C "$source_dir/llama.cpp" rev-parse HEAD)" == 560445bf34c87356ad0f8d80fb03ec5488850b65 ]] || { echo 'llama.cpp revision mismatch' >&2; exit 2; }
node "$root/scripts/patch-nemo.ts" "$source_dir"
if [[ ! -e "$tokenizer_source/.git" ]]; then
  git clone --no-checkout https://github.com/google/sentencepiece.git "$tokenizer_source"
  git -C "$tokenizer_source" checkout --detach 17d7580d6407802f85855d2cc9190634e2c95624
fi
[[ "$(git -C "$tokenizer_source" rev-parse HEAD)" == 17d7580d6407802f85855d2cc9190634e2c95624 ]] || { echo 'SentencePiece revision mismatch' >&2; exit 2; }
cmake -S "$tokenizer_source" -B "$tokenizer_build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DSPM_BUILD_TEST=OFF -DSPM_ENABLE_SHARED=OFF -DSPM_ENABLE_TCMALLOC=OFF
cmake --build "$tokenizer_build" --target sentencepiece-static --config Release
spm_library="$tokenizer_build/src/libsentencepiece.a"
if [[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]]; then spm_library="$tokenizer_build/src/sentencepiece.lib"; fi
# Build the CPU code for a fixed baseline (AVX2 on x86, no -mcpu=native on ARM),
# not for the CPU of the build machine, so the helper runs on the users' computers.
# Each option is set: a native configure caches the x86 options as OFF, and a
# build folder that CI keeps would otherwise keep those values.
cpu_baseline=(GGML_NATIVE=OFF GGML_SSE42=ON GGML_AVX=ON GGML_AVX2=ON GGML_BMI2=ON GGML_FMA=ON GGML_F16C=ON
  GGML_AVX_VNNI=OFF GGML_AVX512=OFF GGML_AVX512_VBMI=OFF GGML_AVX512_VNNI=OFF GGML_AVX512_BF16=OFF)
"$source_dir/scripts/configure.sh" "$backend-asr" -B "$sdk_build" "${cpu_baseline[@]/#/-D}" -DNEMO_SPEECH_BUILD_MIC_CAPTURE=OFF -DNEMO_SPEECH_BUILD_CLI=OFF -DNEMO_SPEECH_BUILD_TESTS=ON -DNEMO_SPEECH_WITH_NORM=OFF -DSENTENCEPIECE_LIB="$spm_library" -DSENTENCEPIECE_INCLUDE_DIR="$tokenizer_source/src"
for setting in "${cpu_baseline[@]}"; do
  grep -Eq "^${setting%%=*}:[A-Z]+=${setting#*=}"$'\r?$' "$sdk_build/CMakeCache.txt" || { echo "the NeMo build must set ${setting}" >&2; exit 2; }
done
cmake --build "$sdk_build" --config Release
cmake -S "$root/native/nemo-helper" -B "$helper_build" -G Ninja -DCMAKE_BUILD_TYPE=Release -DLONGHAND_NEMO_SOURCE="$source_dir" -DLONGHAND_NEMO_BUILD="$sdk_build"
cmake --build "$helper_build" --config Release
# This manifest is input to package-nemo.ts. The packager owns the private dependency closure.
node -e 'require("node:fs").writeFileSync(process.argv[1], JSON.stringify({helper:process.argv[2],sdkBin:process.argv[3]},null,2)+"\n")' "$helper_build/build-output.json" "$helper_build/longhand-nemo-helper$([[ "$OSTYPE" == msys* || "$OSTYPE" == cygwin* ]] && echo .exe || true)" "$sdk_build/bin"
echo "$helper_build/build-output.json"
