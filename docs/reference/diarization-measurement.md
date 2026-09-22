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

PENDING.

## Verdict

PENDING.
