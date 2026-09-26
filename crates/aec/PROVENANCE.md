# Provenance

Borrowed material in this crate, one record per file (R27). Only the 128-unit model size is taken. anarlog's 256 and 512 sizes, its feature flags, its recorded test inputs, snapshots, benchmark and Audacity example are not taken. The tests in `tests/cancel.rs` are Longhand's own and use synthetic signals only.

## `data/model_128_1.onnx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/data/models/model_128_1.onnx`
- Origin: DTLN-aec by Nils L. Westhausen, https://github.com/breizhn/DTLN-aec (stage 1 of the two-stage model; block 512, shift 128, 128 LSTM units). anarlog has no nearer notice on this file.
- Copyright: Copyright (c) 2020 Nils L. Westhausen
- Licence: MIT, LICENSES/DTLN-aec-MIT.txt
- Status: copied

## `data/model_128_2.onnx`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/data/models/model_128_2.onnx`
- Origin: DTLN-aec by Nils L. Westhausen, https://github.com/breizhn/DTLN-aec (stage 2 of the two-stage model; block 512, shift 128, 128 LSTM units). anarlog has no nearer notice on this file.
- Copyright: Copyright (c) 2020 Nils L. Westhausen
- Licence: MIT, LICENSES/DTLN-aec-MIT.txt
- Status: copied

## `src/lib.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/src/lib.rs` (`CircularBuffer`)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — `CircularBuffer` copied; the feature flags removed; `EchoCanceller` added: it checks the chunk length, runs the model and then `cancel_linear_echo` with the reference delayed by the model latency, and keeps the linear gain between chunks, as anarlog's `process_aec` in `crates/audio-actual/src/capture/stream.rs` does

## `src/error.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/src/error.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — wraps the errors of Longhand's `crates/onnx`; the `Serialize` implementation removed; `ChunkLength` added

## `src/onnx/mod.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/src/onnx/mod.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — `AEC` renamed `Dtln` and made crate-private; loads through `crates/onnx` with up to four threads inside each operator; the batch `process` and the per-chunk output normalization removed (the linear step clamps the output); the snapshot tests removed

## `src/onnx/model.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/aec/src/onnx/model.rs`
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — only the 128 size, with the models in `data/`

## `src/linear.rs`

- Repository: https://github.com/fastrepl/anarlog
- Revision: 39613e537ba4c2ddbd2eb5fb4f33c6bf9e79d11e
- Source paths: `crates/audio-actual/src/capture/stream.rs` (`cancel_linear_echo`, `segmented_trimmed_gain` and the `AEC_*` constants that they use)
- Copyright: Copyright (c) 2023-present Fastrepl, Inc.
- Licence: MIT, LICENSES/anarlog-MIT.txt
- Status: modified — moved out of the capture code into this crate; `cancel_linear_echo` made public; unit tests added
