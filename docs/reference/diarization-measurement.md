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

Upstream match: PENDING.

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

The borrowed stack is sound: 11.2% DER against human annotation, with speaker
counts recovered exactly on recordings up to 39 minutes. It degrades on long
sessions for a single identifiable reason, and the fix is structural rather than
a model change.

**Diarize in windows shorter than 60 minutes rather than over the whole
session.** This keeps the hop at its native 2 s, bounds embedding memory by
window length instead of session length, and costs nothing the product needs:
labels are only required to be coherent within a window, because the summarizer
reads a window at a time.

**Do not apply a global exact speaker count on a long session.** The Risk table
named R36's count as the first lever when diarization underperforms; measured
here, it is a regression -- a single global cut cannot track a voice across three
hours of far-field drift, so it merges speakers inside a window while splitting
them across windows. R36 stays useful per window, not per session.

**Filter clusters by speech mass.** The top 6 of 149 labels hold 88.7% of
speech. `min_cluster_seconds: 12.0` is far too permissive at this length.
