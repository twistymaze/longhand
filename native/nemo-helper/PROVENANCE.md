# Provenance

## `main.cpp`

- Repository: https://github.com/NVIDIA/NeMo-Speech.cpp
- Revision: 97a15afa5caa9bce5baaa86c1184103877af4101
- Source paths: `app/transcribe.cpp`, `include/nemo_speech/diar.h`, `src/asr/recognizer.h`
- Copyright: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES
- Licence: Apache-2.0, licences/sources/nemo-speech-LICENSE.txt
- Status: reference. Longhand owns the framed protocol and process lifecycle. The adapter follows the SDK's stream polling and speaker tag refresh sequence. The pinned SDK has one source change before the build: `scripts/patch-nemo.ts` caps the CPU graph threads in `src/runtime/ggml/session.cpp` at the hardware thread count, and marks the change "Modified by Longhand". Its notices and linked dependency licences are listed in `licences/manifest.json`.

### Windows Vulkan loader

- Source: https://sdk.lunarg.com/sdk/download/1.4.309.0/windows/VulkanRT-1.4.309.0-Components.zip
- Version: 1.4.309.0, x64
- Archive SHA-256: `7d969f4d7b44e387667d3148f61559497c22d50cbe3d50adc9e5409afbce2df1`
- DLL SHA-256: `6f6b53f86d2be29c77fdcd69e8aa2034bb90fd116b0b9dc403cf091835a88d70`
- Runtime licence: upstream runtime notices in `licences/sources/vulkan-runtime-LICENSE.txt`.
- Distribution: redistributed binary. CI verifies the archive and extracted x64 DLL hashes. The helper package includes the loader and checks its imported dependencies. No DLL is copied from the build host's system directory.
