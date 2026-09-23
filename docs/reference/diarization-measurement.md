# Diarization measurement (U1)

Phase 1's verdict on whether the borrowed diarization stack preserves enough
conversational structure to build on. Written per U1 of
`docs/plans/2026-09-21-2052-feat-longhand-walking-skeleton-plan.md`.

## What is being decided, and what is not

The transcript's consumer is an LLM summarizer, not a reader. Knowing that an
exchange took place is what lets a summary describe a discussion rather than a
monologue; knowing which participant said a given line is not something the
summary reports. So the gate is **collapse**, and attribution accuracy is a
diagnostic.

The two errors are not symmetric. Splitting one person across two labels leaves
the exchange structure intact and costs the summarizer nothing. Merging two
people into one destroys it — that is the flattening the Problem Frame names as
a reason this product exists. Only the second is a failure.

## The frozen gate

Fixed before any measurement was run, so it cannot be relaxed after seeing a
result. Both conditions must hold on the session recording.

| Condition | Threshold | Rationale |
|---|---|---|
| Five-minute windows in which the reference shows >= 2 speakers, where the hypothesis also shows >= 2 | >= 90% | Detects collapse into a single speaker, at the granularity a summarizer reads. |
| Turn count, hypothesis over reference | 0.5x to 2.0x | Detects the same failure from the other side: a stack emitting a handful of enormous turns has flattened the session even if it nominally found several speakers. |

Everything else on this page is recorded to show where the stack stands and to
give a later change something to be compared against. None of it gates Phase 2.

## Inputs

| Input | What it is | Role |
|---|---|---|
| `fixtures/session/sample.wav` | 3:00:38 of a real in-person session, one microphone, 5-6 participants, converted to mono 16 kHz | The gate is measured on this. Held locally, never committed, never uploaded. |
| `fixtures/session/sample.rttm` | Reference derived from an existing third-party transcript of the same session | **Boundaries only.** That tool reported 16 speakers where 5-6 were present, so its labels are not usable and every comparison against it is label-free. |
| `fixtures/session-bounded/sample.rttm` | The same boundaries with labels collapsed to 6 | Exists solely to set the evaluation binary's speaker bound, which it derives from the reference's distinct-label count. Asserts nothing about who spoke. |
| `fixtures/diar-test/` | AMI single-distant-mic and VoxConverse test subsets, human ground truth, CC BY 4.0 | Absolute DER, against real annotation. AMI SDM is a meeting room on one distant microphone, which is the condition this product targets. |

## Weight provenance (R28)

Measured against the weights in the pinned clone
`fastrepl/anarlog@7040ace23d1606dcc4371b5bb8c5aca1c3450044`.

| File | Size | SHA-256 |
|---|---|---|
| `crates/pyannote-local/src/data/segmentation.onnx` | 5,986,908 | `057ee564753071c0b09b5b611648b50ac188d50846bff5f01e9f7bbf1591ea25` |
| `crates/embedding/src/onnx/embedding.onnx` | 26,543,975 | `841645a9b5109369858ca89695e8b23ecfd4486b30bdd5cb3f99fef8d1c29205` |

Upstream match (checked in U4, 2026-09-22, by SHA-256 against the publishers' LFS metadata):

- `segmentation.onnx` — **matched**: byte-identical to `onnx/model.onnx` in `onnx-community/pyannote-segmentation-3.0` at revision `733a93b6473d019a773298e08cefa686894b1854` (MIT), an ONNX export of `pyannote/segmentation-3.0`.
- `embedding.onnx` — **not matched**: no public release has this size or hash, and anarlog added the file without recording a source. The catalog instead pins `voxceleb_resnet34_LM.onnx` from `Wespeaker/wespeaker-voxceleb-resnet34-LM` at revision `f0c48c298fd835726c27956a5d617bad7115627e` (26,530,309 bytes, SHA-256 `7bb2f06e9df17cdf1ef14ee8a15ab08ed28e8d0ef5054ee135741560df2ec068`, CC BY 4.0) — the ResNet34-LM model pyannote's pipeline uses. The results below were measured with the unprovenanced blob; U9 re-measures with the WeSpeaker file.

## Results

### The gate

| Run | Multi-speaker 5-min windows kept | Turn ratio | Verdict |
|---|---|---|---|
| Unconstrained | 37/37 = 1.000 | 1.354 | **PASS** |
| Bounded at 6 (`--exact-speakers`) | 29/37 = 0.784 | 0.605 | FAIL |

**Phase 1 passes on the unconstrained configuration.** The session does not
collapse into a monologue, which is the only outcome that would have blocked
Phase 2.

### Absolute DER, human ground truth

12 files, 4.67 hours, collar 0.25. Every file here is between 7 and 39 minutes.

| Corpus | Files | DER | miss | FA | conf | Speaker count recovery |
|---|---|---|---|---|---|---|
| AMI single distant mic | 6 | 0.096 - 0.248 | 0.045 - 0.171 | 0.020 - 0.034 | 0.017 - 0.051 | 4->4 on three, 4->6, 4->5, 4->2 |
| VoxConverse | 6 | 0.018 - 0.078 | 0.001 - 0.040 | 0.006 - 0.036 | 0.009 - 0.027 | 7->7, 12->12, 10->10, 9->10, 2->2, 3->2 |
| **Overall** | **12** | **0.1124** | 0.0640 | 0.0247 | 0.0237 | 67 reference speakers, 68 found |

The stack recovers twelve speakers from a twelve-speaker recording. Its quality
is not in question.

### The session, and why it differs

| Measure | Unconstrained | Bounded at 6 |
|---|---|---|
| Labels emitted | 149 | 6 |
| Speech mass in top 6 labels | 88.7% | - |
| Speakers per 5-min window (reference: 4) | 12 | 2 |
| Boundary agreement F1, label-free | 0.395 | 0.032 |
| Windowed DER, 5 min | 0.580 | 0.559 |
| Global DER from the eval binary | 2.361 | 0.569 |

The last two rows disagree about which run is better, and the structural
measures are the ones to believe. Global DER rewards the bounded run for having
the right number of labels while it merges four speakers into two inside a
window; that is the error that flattens an exchange.

### Root cause: `max_windows` stretches the analysis hop

`DiarizationConfig::max_windows` is 1800 and `step_samples` grows the hop so the
window count stays under it. The hop is therefore a function of session length:

| Session length | Effective hop |
|---|---|
| 20 min | 1.99 s |
| 39 min | 1.99 s |
| 59.9 min | 2.00 s (the cliff) |
| 3.01 h (this session) | 6.02 s |
| 4 h | 8.02 s |

Every corpus file above sits under the cliff and runs at the native 2 s hop.
This session runs at three times that, and a four-hour session would run at four
times. The 149 fragments and the weak boundary agreement are what a 6 s hop
produces; they are not evidence about the models.

### Confirming the cause: the same audio at the native hop

Rerun unchanged except for `max_windows: 6000`, which holds the hop at 2.00 s
across all three hours.

| Measure | cap 1800 (hop 6.02 s) | cap 6000 (hop 2.00 s) |
|---|---|---|
| Labels emitted | 149 | **7** |
| Speech mass in top 6 labels | 88.7% | **100.0%** |
| Speakers per 5-min window (reference: 4) | 12 | **4** |
| Speakers per 10-min window (reference: 5) | 21 | 4 |
| Turn count ratio vs reference | 1.354 | **1.056** |
| Windowed DER, 5 min | 0.580 | **0.480** |
| Boundary agreement F1 | 0.395 | 0.349 |
| Gate | PASS | PASS |

Seven labels against a real headcount of five or six, with the top six holding
all the speech. The fragment tail is gone entirely, and the per-window speaker
count matches the reference exactly. The hop was the whole cause.

Boundary agreement F1 dips because fewer labels mean fewer turn changes, and
recall is measured against a reference that itself over-segments into 16 labels.
Precision rose, 0.457 to 0.516, which is the trustworthy direction here.

### Cost of the fix

| Measure | cap 1800 | cap 6000 | Delta |
|---|---|---|---|
| Peak RSS | 1.121 GB | 1.164 GB | **+43 MB** |
| Wall time, 3.01 h of audio | 725 s | 2,103 s | **2.9x** |
| Realtime factor | 0.067 | 0.194 | |

Memory is not the constraint. `linkage` allocates an n x n f32 distance matrix,
which is 13 MB at 1,800 windows, 52 MB at 3,612, and 209 MB at 7,228; embeddings
are negligible beside it at 256 floats per window. Wall time scales close to
linearly with window count rather than quadratically, so embedding dominates and
the cap can rise without the cost exploding.

`max_windows` is very nearly 30 per minute of audio:

| Cap | Session length held at a 2 s hop |
|---|---|
| 1,804 | 60 min |
| 2,708 | 90 min |
| **3,612** | **120 min** |
| 5,420 | 180 min |
| 7,228 | 240 min |

### Resource profile

| Measure | Value |
|---|---|
| Realtime factor | 0.067 (3.01 h diarized in 12.1 min; a 4 h session costs ~16 min) |
| Peak RSS, as measured | 1.12 GB |
| Audio buffer within that | 694 MB (the eval binary reads the whole WAV into a `Vec<f32>`) |
| Working set, buffer subtracted | ~430 MB -- the figure R7 constrains, since the shipping path reads through a file-backed source |
| Growth over three hours | none; RSS was flat from minute 3 onward |

### Risks this retires

- **Segmentation cap.** Only 1.0% of 10-second windows carry four or more
  distinct speakers, so the three-distinct-speakers-per-window limit almost
  never binds on this material.
- **Memory at length.** Flat, not growing with session length.

## Verdict

**PASS, with one design consequence for U9.**

Settled: `max_windows` rises to **3,612**, holding the native 2 s hop for
sessions up to 120 minutes, and sessions longer than that are clustered in
consecutive windows of at most that length. Speaker numbering continues across
windows rather than restarting, so one person may carry two labels while a label
never covers two people. The reasoning below records why.

The borrowed stack is sound: 11.2% DER against human annotation, with speaker
counts recovered exactly on recordings up to 39 minutes. It degrades on long
sessions for a single identifiable reason, and the fix is structural rather than
a model change.

**Hold the native 2 s hop, and cap the clustering window rather than the
session.** The hop is what decides quality; the cap only decides how much
compute a very long session spends. 120 minutes covers routine use in a single
window, and memory at that size is 52 MB, which nothing in R7 notices.

**Do not apply a global exact speaker count on a long session.** The Risk table
named R36's count as the first lever when diarization underperforms; measured
here, it is a regression -- a single global cut cannot track a voice across three
hours of far-field drift, so it merges speakers inside a window while splitting
them across windows. R36 stays useful per window, not per session.

**Filter clusters by speech mass.** The top 6 of 149 labels hold 88.7% of
speech. `min_cluster_seconds: 12.0` is far too permissive at this length.

## Re-measure with the shipping embedding (U9, 2026-09-22)

Everything above measured the embedding blob compiled into anarlog's
`crates/embedding`, which carries no provenance. Longhand ships the model
catalog's WeSpeaker ResNet34-LM download instead (SHA-256 `7bb2f06e…`, against
the blob's `841645a9…`). Rerun on the same session and configuration (cap 6000,
2 s hop), built inside Longhand's workspace with weights loaded from disk:

| Measure | anarlog blob | catalog WeSpeaker |
|---|---|---|
| Multi-speaker 5-min windows kept | 37/37 | **37/37** |
| Turn count ratio vs reference | 1.056 | **1.056** |
| Labels emitted | 7 | **7** |
| Windowed DER, 5 min | 0.480 | **0.480** |
| Boundary agreement F1 | 0.349 | **0.349** |
| Wall time, 3.01 h of audio | 2,103 s | 2,125 s |
| Peak RSS | 1.164 GB | 1.150 GB |
| Gate | PASS | **PASS** |

Identical to three decimals on every statistic, so the two files are almost
certainly the same network serialized differently. The model's `feats`/`embs`
input and output names and the 80-bin fbank front end in `crates/embedding`
match the catalog file as downloaded. The gate stands for the weights that ship;
no full rerun is needed.
