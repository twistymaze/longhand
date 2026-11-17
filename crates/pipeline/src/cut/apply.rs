//! Cut execution (KTD15, KTD16, KTD18, KTD19 of the forgotten-recording
//! plan).
//!
//! [`run`] runs at the start of the Assemble stage, before Assemble reads
//! the segment rows. It finds the plan with [`super::detect`] and applies
//! it in a crash-safe order:
//!
//! 1. Write the plan as an Assemble checkpoint ([`PLAN_KEY`]).
//! 2. In one store transaction: write the segment rows, the cut marks, the
//!    re-keyed live rows, source proofs, snapshots and feature keys, and
//!    the clipped epoch records.
//! 3. Rename the chunks and sidecars of each part that has a new index.
//! 4. Delete the chunks and sidecars of the removed audio, after a check of
//!    each name and entry kind.
//! 5. Mark the cut done.
//!
//! The segment rows come before the renames, because a chunk with no row
//! gets a row at launch (`recover::register_chunks`); that step waits
//! while a cut is not done ([`unfinished`]). At the next run, a plan that is
//! not done finishes first. Each file step checks its source, so it can run
//! again.
//!
//! The live results of the kept audio stay valid (KTD18). One read pass
//! over the chunks of each split segment verifies the old `live-source`
//! proof and makes one proof for each part. Live rows split at the part
//! edges. Live speaker feature windows that lie fully inside a part keep
//! their place on the original grid; the other windows are deleted, so
//! `live::features::complete` computes only the windows at the edges.

use std::collections::BTreeSet;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use session_core::jobs::CutRows;
use session_core::{Segment, Stage};

use super::{ChunkSpan, CutPlan, CutSettings, PlannedSegment, detect, read_input};
use crate::live::archive::{self, Archive, ArchiveReader, SourceProof};
use crate::live::{self, LiveRow, features};
use crate::{Control, RECOVERY_DIR, SharedStore, StageError, checkpoint, lock};

/// The Assemble checkpoint that holds the cut plan of the note.
pub const PLAN_KEY: &str = "cut-plan";

/// The samples of one live speaker feature window at 16 kHz.
const FEATURE_WINDOW: usize = 160_000;

/// The Assemble checkpoint that holds the offset of the original segment
/// of a part that starts after it. Assemble uses it to give the part the
/// processing samples of the original segment.
pub fn origin_key(index: u32) -> String {
    format!("cut-origin-{index:08}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Step {
    /// Step 1 is done.
    Planned,
    /// Step 2 is done.
    Rows,
    /// Step 5 is done.
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Stored {
    step: Step,
    plan: CutPlan,
    /// The rows of the segments of the current recording before the cut.
    original: Vec<Segment>,
    /// The live feature files that step 4 deletes, relative to the
    /// `live-features` folder.
    #[serde(default)]
    feature_files: Vec<PathBuf>,
}

/// The fields of the recording settings snapshot that the cut reads.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Guards {
    silence_check_minutes: Option<u32>,
    cut_minutes: Option<u32>,
}

/// True when the note has a cut that is not done.
pub fn unfinished(store: &SharedStore, id: &str) -> Result<bool, StageError> {
    Ok(stored(store, id)?.is_some_and(|stored| stored.step != Step::Done))
}

fn stored(store: &SharedStore, id: &str) -> Result<Option<Stored>, StageError> {
    checkpoint::get(store, id, Stage::Assemble, PLAN_KEY)
}

/// Find and apply the cut of the current recording of the note `id`. A
/// cut that is not done finishes first. A note with no snapshot, with the
/// cut off, or with a checked current recording is not changed.
pub fn run(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    control: &dyn Control,
) -> Result<(), StageError> {
    if let Some(stored) = stored(store, id)?
        && stored.step != Step::Done
    {
        return finish(store, id, session_dir, stored, control);
    }
    let Some(settings) = settings(store, id)? else {
        return Ok(());
    };
    let recovery = session_dir.join(RECOVERY_DIR);
    // A crash while capturing leaves a `.part` open; close it from its length.
    recorder::recover_partial_chunks(&recovery)?;
    let segments = lock(store).segments(id)?;
    let Some(start) = recording_start(session_dir, &segments)? else {
        return Ok(());
    };
    let checked = lock(store).cut_checked(id)?;
    let live_rows = live::rows(store, id)?;
    let input = read_input(&recovery, &segments, &checked, &live_rows, start, settings)?;
    let Some(plan) = detect(&input) else {
        return Ok(());
    };
    let original: Vec<Segment> = segments
        .into_iter()
        .filter(|s| s.offset_ms >= start)
        .collect();
    if plan.removes_nothing() && unchanged(&plan, &original) {
        let indices: Vec<u32> = plan.segments.iter().map(|p| p.row.index).collect();
        lock(store).mark_cut_checked(id, &indices)?;
        return Ok(());
    }
    let removed_ms: u64 = plan.removed.iter().map(ChunkSpan::duration_ms).sum();
    log::info!(
        "{id}: the cut removes {} chunks ({removed_ms} ms) and keeps {} segments",
        plan.removed.len(),
        plan.segments.len()
    );
    let stored = Stored {
        step: Step::Planned,
        plan,
        original,
        feature_files: Vec::new(),
    };
    checkpoint::put(store, id, Stage::Assemble, PLAN_KEY, &stored)?;
    crash_after(1)?;
    finish(store, id, session_dir, stored, control)
}

#[cfg(test)]
thread_local! {
    /// The step after which a test stops the cut, as a crash does.
    static CRASH_AFTER: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// Stop after `step` when a test asks for it.
fn crash_after(step: u8) -> Result<(), StageError> {
    #[cfg(test)]
    if CRASH_AFTER.with(|c| c.get()) == step {
        return Err(StageError::Failed(format!("a crash after step {step}")));
    }
    let _ = step;
    Ok(())
}

/// The cut settings of the snapshot of the note. `None` for a note with no
/// snapshot or with the cut off.
fn settings(store: &SharedStore, id: &str) -> Result<Option<CutSettings>, StageError> {
    let (guards, auto_stop) = {
        let guard = lock(store);
        (guard.recording_guards(id)?, guard.auto_stop(id)?)
    };
    let Some(guards) = guards else {
        return Ok(None);
    };
    let guards: Guards = match serde_json::from_str(&guards) {
        Ok(guards) => guards,
        Err(e) => {
            log::warn!("{id}: the recording settings snapshot does not read: {e}");
            return Ok(None);
        }
    };
    if guards.cut_minutes.is_none() {
        return Ok(None);
    }
    Ok(Some(CutSettings {
        cut_minutes: guards.cut_minutes,
        silence_check_minutes: guards.silence_check_minutes,
        stopped_by_silence_check: auto_stop.is_some_and(|s| s.silence_check_minutes.is_some()),
    }))
}

/// Where the current recording starts on the note timeline. A Resume of a
/// saved note lists the segments that the note had before; the current
/// recording is the segments after them. `None` when the note has no
/// segment in the current recording.
fn recording_start(session_dir: &Path, segments: &[Segment]) -> Result<Option<u64>, StageError> {
    let old = crate::old_audio::old_segments(session_dir)?.unwrap_or_default();
    Ok(segments
        .iter()
        .filter(|s| !old.iter().any(|o| o.audio_file == s.audio_file))
        .map(|s| s.offset_ms)
        .min())
}

/// True when the plan keeps each segment as it is.
fn unchanged(plan: &CutPlan, original: &[Segment]) -> bool {
    plan.segments.len() == original.len()
        && plan
            .segments
            .iter()
            .all(|part| part.source == part.row.index && original.contains(&part.row))
}

/// Do the steps 2 to 5 that are not done.
fn finish(
    store: &SharedStore,
    id: &str,
    session_dir: &Path,
    mut stored: Stored,
    control: &dyn Control,
) -> Result<(), StageError> {
    let recovery = session_dir.join(RECOVERY_DIR);
    if stored.step == Step::Planned {
        control.proceed()?;
        let (mut rows, feature_files) = store_changes(store, id, &recovery, &stored)?;
        stored.step = Step::Rows;
        stored.feature_files = feature_files;
        rows.checkpoints.push((
            Stage::Assemble,
            PLAN_KEY.into(),
            Some(serde_json::to_string(&stored)?),
        ));
        lock(store).apply_cut(id, &rows)?;
        crash_after(2)?;
    }
    rename_parts(&recovery, &stored.plan)?;
    crash_after(3)?;
    delete_removed(&recovery, &stored.plan.removed)?;
    delete_feature_files(&session_dir.join("live-features"), &stored.feature_files)?;
    crash_after(4)?;
    stored.step = Step::Done;
    checkpoint::put(store, id, Stage::Assemble, PLAN_KEY, &stored)?;
    Ok(())
}

// ------------------------------------------------------------ step 2

/// One checkpoint row.
struct Row {
    stage: Stage,
    key: String,
    payload: String,
}

/// The store changes of step 2, and the live feature files that the cut
/// deletes. Every read is done before the transaction.
fn store_changes(
    store: &SharedStore,
    id: &str,
    recovery: &Path,
    stored: &Stored,
) -> Result<(CutRows, Vec<PathBuf>), StageError> {
    let plan = &stored.plan;
    let transcribe = checkpoint::all(store, id, Stage::Transcribe)?;
    let diarize = checkpoint::all(store, id, Stage::Diarize)?;
    let epochs: Vec<Row> = [(Stage::Transcribe, &transcribe), (Stage::Diarize, &diarize)]
        .into_iter()
        .flat_map(|(stage, rows)| {
            rows.iter()
                .filter(|(key, _)| key.starts_with("recognition-epoch-"))
                .map(move |(key, payload)| Row {
                    stage,
                    key: key.clone(),
                    payload: payload.clone(),
                })
        })
        .collect();

    let mut deletes: Vec<(Stage, String)> = Vec::new();
    let mut puts: Vec<Row> = Vec::new();
    let mut feature_files = Vec::new();
    for segment in &stored.original {
        let s = segment.index;
        let parts: Vec<&PlannedSegment> = plan.segments.iter().filter(|p| p.source == s).collect();
        // Every key of the original index goes. The parts get new keys.
        let owned = |key: &str, prefixes: &[String]| prefixes.iter().any(|p| key.starts_with(p));
        let transcribe_prefixes = [
            checkpoint::live_prefix(s),
            archive::key(s),
            format!("live-snapshot-{s:08}"),
            format!("live-failure-{s:08}"),
        ];
        let diarize_prefixes = [features::key(s), format!("live-feature-{s:08}-")];
        for (key, _) in &transcribe {
            if owned(key, &transcribe_prefixes) {
                deletes.push((Stage::Transcribe, key.clone()));
            }
        }
        for (key, _) in &diarize {
            if owned(key, &diarize_prefixes) {
                deletes.push((Stage::Diarize, key.clone()));
            }
        }
        deletes.push((Stage::Assemble, origin_key(s)));

        let rows = live::segment_rows(store, id, s)?;
        let value = |stage_rows: &[(String, String)], key: &str| {
            stage_rows
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
        };
        let whole = parts.len() == 1
            && parts[0].row.offset_ms == segment.offset_ms
            && parts[0].row.duration_ms == segment.duration_ms;

        // The live rows: each part keeps the words inside it.
        for part in &parts {
            let i = part.row.index;
            let from = part.row.offset_ms - segment.offset_ms;
            for (number, row) in split_rows(&rows, from, from + part.row.duration_ms, i)
                .into_iter()
                .enumerate()
            {
                let row = LiveRow {
                    part: number as u32,
                    ..row
                };
                puts.push(Row {
                    stage: Stage::Transcribe,
                    key: checkpoint::live_key(i, row.part),
                    payload: serde_json::to_string(&row)?,
                });
            }
            if let Some(failure) = value(&transcribe, &format!("live-failure-{s:08}")) {
                puts.push(Row {
                    stage: Stage::Transcribe,
                    key: format!("live-failure-{i:08}"),
                    payload: failure,
                });
            }
            if part.row.offset_ms != segment.offset_ms {
                puts.push(Row {
                    stage: Stage::Assemble,
                    key: origin_key(i),
                    payload: segment.offset_ms.to_string(),
                });
            }
        }

        // The source proofs and the snapshot.
        let snapshot = value(&transcribe, &format!("live-snapshot-{s:08}"));
        let proofs: Vec<Option<String>> = match value(&transcribe, &archive::key(s)) {
            Some(proof) if whole => vec![Some(proof)],
            Some(proof) => {
                let proof: SourceProof = serde_json::from_str(&proof)?;
                part_proofs(recovery, segment, &parts, &proof, &rows, &epochs)?
                    .into_iter()
                    .map(|proof| {
                        proof
                            .filter(|p| p.samples > 0)
                            .map(|p| serde_json::to_string(&p))
                            .transpose()
                    })
                    .collect::<Result<_, _>>()?
            }
            None => vec![None; parts.len()],
        };
        for (part, proof) in parts.iter().zip(proofs) {
            let Some(proof) = proof else {
                continue;
            };
            let i = part.row.index;
            puts.push(Row {
                stage: Stage::Transcribe,
                key: archive::key(i),
                payload: proof,
            });
            if let Some(snapshot) = &snapshot {
                puts.push(Row {
                    stage: Stage::Transcribe,
                    key: format!("live-snapshot-{i:08}"),
                    payload: snapshot.clone(),
                });
            }
        }

        // The live speaker features.
        let windows: Vec<(usize, &String)> = diarize
            .iter()
            .filter_map(|(key, payload)| {
                let window = key
                    .strip_prefix(&format!("live-feature-{s:08}-"))?
                    .parse::<usize>()
                    .ok()?;
                Some((window, payload))
            })
            .collect();
        if whole {
            let i = parts[0].row.index;
            for (window, payload) in &windows {
                puts.push(Row {
                    stage: Stage::Diarize,
                    key: features::window_key(i, *window),
                    payload: (*payload).clone(),
                });
            }
            for (key, target) in [
                (features::key(s), features::key(i)),
                (features::grid_key(s), features::grid_key(i)),
            ] {
                if let Some(payload) = value(&diarize, &key) {
                    puts.push(Row {
                        stage: Stage::Diarize,
                        key: target,
                        payload,
                    });
                }
            }
        } else if !windows.is_empty() {
            let index: Option<serde_json::Value> = value(&diarize, &features::key(s))
                .map(|v| serde_json::from_str(&v))
                .transpose()?;
            let step = index
                .as_ref()
                .and_then(|index| index["step"].as_u64())
                .or_else(|| {
                    serde_json::from_str::<serde_json::Value>(windows[0].1)
                        .ok()?
                        .get("step")?
                        .as_u64()
                })
                .unwrap_or(0) as usize;
            let total = index
                .as_ref()
                .and_then(|index| index["total"].as_u64())
                .unwrap_or(segment.duration_ms * 16) as usize;
            let mut kept = BTreeSet::new();
            for part in &parts {
                let i = part.row.index;
                let edge = ((part.row.offset_ms - segment.offset_ms) * 16) as usize;
                let length = (part.row.duration_ms * 16) as usize;
                let Some(first) = edge.checked_div(step) else {
                    continue;
                };
                let grid_start = (first * step) as i64 - edge as i64;
                puts.push(Row {
                    stage: Stage::Diarize,
                    key: features::grid_key(i),
                    payload: grid_start.to_string(),
                });
                for (window, payload) in windows.iter().filter(|(w, _)| *w >= first) {
                    let at = grid_start + ((window - first) * step) as i64;
                    let len = FEATURE_WINDOW.min(total.saturating_sub(window * step));
                    if at >= 0 && at as usize + len <= length {
                        kept.insert(*window);
                        puts.push(Row {
                            stage: Stage::Diarize,
                            key: features::window_key(i, window - first),
                            payload: (*payload).clone(),
                        });
                    }
                }
            }
            for (window, payload) in &windows {
                if !kept.contains(window)
                    && let Ok(saved) = serde_json::from_str::<features::Window>(payload)
                {
                    feature_files.push(saved.file);
                }
            }
        }
    }

    // The epoch records of the current recording clip to the kept audio.
    let first = stored.original.iter().map(|s| s.offset_ms * 16).min();
    let last = stored.original.iter().map(|s| s.end_ms() * 16).max();
    if let (Some(first), Some(last)) = (first, last) {
        let kept: Vec<Range<u64>> = plan
            .segments
            .iter()
            .map(|p| p.row.offset_ms * 16..p.row.end_ms() * 16)
            .collect();
        for epoch in &epochs {
            let mut record: serde_json::Value = serde_json::from_str(&epoch.payload)?;
            let (Some(start), Some(end)) = (
                record["range"]["start"].as_u64(),
                record["range"]["end"].as_u64(),
            ) else {
                continue;
            };
            if start >= last || end <= first {
                continue;
            }
            let inside: Vec<Range<u64>> = kept
                .iter()
                .filter(|k| k.start < end && k.end > start)
                .map(|k| k.start.max(start)..k.end.min(end))
                .collect();
            match (inside.first(), inside.last()) {
                (Some(a), Some(b)) => {
                    if a.start != start || b.end != end {
                        record["range"]["start"] = a.start.into();
                        record["range"]["end"] = b.end.into();
                        puts.push(Row {
                            stage: epoch.stage,
                            key: epoch.key.clone(),
                            payload: serde_json::to_string(&record)?,
                        });
                    }
                }
                _ => deletes.push((epoch.stage, epoch.key.clone())),
            }
        }
    }

    let indices: BTreeSet<u32> = plan.segments.iter().map(|p| p.row.index).collect();
    let mut checkpoints: Vec<(Stage, String, Option<String>)> = deletes
        .into_iter()
        .map(|(stage, key)| (stage, key, None))
        .collect();
    checkpoints.extend(
        puts.into_iter()
            .map(|row| (row.stage, row.key, Some(row.payload))),
    );
    Ok((
        CutRows {
            deleted_segments: stored
                .original
                .iter()
                .map(|s| s.index)
                .filter(|i| !indices.contains(i))
                .collect(),
            segments: plan.segments.iter().map(|p| p.row.clone()).collect(),
            checked: indices.into_iter().collect(),
            checkpoints,
        },
        feature_files,
    ))
}

/// The pieces of the live rows `rows` inside the milliseconds `from..to`
/// of their segment, for the part with the index `index`. Each piece keeps
/// the words that start inside the part, and its times move by `from`.
fn split_rows(rows: &[LiveRow], from: u64, to: u64, index: u32) -> Vec<LiveRow> {
    rows.iter()
        .filter_map(|row| {
            let start = row.start_ms.max(from);
            let end = row.end_ms.min(to);
            (start < end).then(|| LiveRow {
                segment: index,
                part: row.part,
                start_ms: start - from,
                end_ms: end - from,
                words: row
                    .words
                    .iter()
                    .filter(|w| w.start_ms >= from && w.start_ms < to)
                    .map(|w| asr::LiveWord {
                        start_ms: w.start_ms - from,
                        end_ms: w.end_ms.min(to) - from,
                        text: w.text.clone(),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// One read pass over the chunks of `segment`: verify its old proof, and
/// give one proof for each part. A proof covers only the audio that the
/// old proof covered. When the old proof does not verify, every part gets
/// `None`.
fn part_proofs(
    recovery: &Path,
    segment: &Segment,
    parts: &[&PlannedSegment],
    old: &SourceProof,
    rows: &[LiveRow],
    epochs: &[Row],
) -> Result<Vec<Option<SourceProof>>, StageError> {
    let rate = u64::from(old.rate);
    let frames = |ms: u64| ms * rate / 1000;
    let ranges = parts
        .iter()
        .map(|part| {
            let from = part.row.offset_ms - segment.offset_ms;
            let to = if part.row.end_ms() >= segment.end_ms() {
                u64::MAX
            } else {
                frames(from + part.row.duration_ms)
            };
            frames(from)..to
        })
        .collect();
    let mut reader = ArchiveReader::with_parts(
        Archive {
            dir: recovery.to_path_buf(),
            segment: segment.index,
            offset_ms: segment.offset_ms,
            rate: old.rate,
        },
        ranges,
    );
    while reader.position() < old.samples && reader.next_frame()?.is_some() {}
    let covered_end =
        segment.offset_ms * 16 + crate::resample::output_sample_count(old.samples, rate, 16_000);
    let start = segment.offset_ms * 16;
    let end = segment.end_ms() * 16;
    let mut verified = reader.proof() == *old
        && rows
            .iter()
            .all(|row| (segment.offset_ms + row.end_ms) * 16 <= covered_end);
    for epoch in epochs {
        let at = epoch
            .key
            .strip_prefix("recognition-epoch-")
            .and_then(|s| s.parse::<u64>().ok());
        if at.is_some_and(|at| at >= start && at < end) {
            let record: serde_json::Value = serde_json::from_str(&epoch.payload)?;
            verified &= record["range"]["end"]
                .as_u64()
                .is_some_and(|end| end <= covered_end);
        }
    }
    if !verified {
        log::warn!(
            "the live source proof of segment {} does not verify; the cut writes no proofs for its parts",
            segment.index
        );
        return Ok(vec![None; parts.len()]);
    }
    Ok(reader.part_proofs().into_iter().map(Some).collect())
}

// ------------------------------------------------------------ steps 3 and 4

/// Give the chunks and sidecars of each part its new index. A chunk is
/// found by its times, so a rename that a crash interrupted continues.
fn rename_parts(recovery: &Path, plan: &CutPlan) -> Result<(), StageError> {
    let chunks = recorder::list_chunks(recovery)?;
    for part in &plan.segments {
        let index = part.row.index;
        for span in part.chunks.iter().filter(|span| span.segment != index) {
            let Some(chunk) = chunks.iter().find(|c| {
                c.start_ms == span.start_ms
                    && c.end_ms == span.end_ms
                    && (c.segment == span.segment || c.segment == index)
            }) else {
                log::warn!(
                    "the chunk at {} ms of segment {} is missing; the cut cannot rename it",
                    span.start_ms,
                    span.segment
                );
                continue;
            };
            if chunk.segment == span.segment {
                std::fs::rename(&chunk.path, recovery.join(chunk_name(index, chunk)))?;
            }
            let sidecar = recorder::speech::sidecar_path(
                recovery,
                span.segment,
                chunk.capture_started_at,
                chunk.start_ms,
            );
            if std::fs::symlink_metadata(&sidecar).is_ok_and(|m| m.file_type().is_file()) {
                std::fs::rename(
                    &sidecar,
                    recorder::speech::sidecar_path(
                        recovery,
                        index,
                        chunk.capture_started_at,
                        chunk.start_ms,
                    ),
                )?;
            }
        }
    }
    Ok(())
}

/// The name that the chunk writer gives `chunk` as a chunk of `segment`.
fn chunk_name(segment: u32, chunk: &recorder::ChunkInfo) -> String {
    format!(
        "{segment}-{}-{}-{}-{}.wav",
        chunk.capture_started_at, chunk.start_ms, chunk.end_ms, chunk.audio_start_ms
    )
}

/// Delete the chunks of `removed` and their sidecars. Only a plain file
/// with the exact name that the chunk writer gives is deleted
/// (`docs/solutions/security-issues/audio-removal-trusted-synced-file-names-before-delete.md`).
fn delete_removed(recovery: &Path, removed: &[ChunkSpan]) -> Result<(), StageError> {
    if removed.is_empty() {
        return Ok(());
    }
    if !std::fs::symlink_metadata(recovery).is_ok_and(|m| m.file_type().is_dir()) {
        return Err(StageError::Failed(format!(
            "{} is not a folder; the cut deletes no audio",
            recovery.display()
        )));
    }
    let chunks = recorder::list_chunks(recovery)?;
    for span in removed {
        for chunk in chunks.iter().filter(|c| {
            c.segment == span.segment && c.start_ms == span.start_ms && c.end_ms == span.end_ms
        }) {
            let name = chunk.path.file_name().map(|n| n.to_string_lossy());
            if name.as_deref() != Some(chunk_name(chunk.segment, chunk).as_str()) {
                log::warn!(
                    "{} is not a name that Longhand writes; the cut does not delete it",
                    chunk.path.display()
                );
                continue;
            }
            // The sidecar goes first: a crash then leaves the chunk, and
            // the next run finds both again.
            remove_plain_file(&recorder::speech::sidecar_path(
                recovery,
                chunk.segment,
                chunk.capture_started_at,
                chunk.start_ms,
            ))?;
            remove_plain_file(&chunk.path)?;
        }
    }
    Ok(())
}

/// Delete the live feature files `files` in `dir`. Only a plain file with
/// a plain `.diar` name is deleted.
fn delete_feature_files(dir: &Path, files: &[PathBuf]) -> Result<(), StageError> {
    for file in files {
        let plain = file.extension().is_some_and(|e| e == "diar")
            && file.components().count() == 1
            && matches!(file.components().next(), Some(Component::Normal(_)));
        if plain {
            remove_plain_file(&dir.join(file))?;
        }
    }
    Ok(())
}

/// Delete `path` when it is a plain file. A symlink or a folder stays. A
/// missing file is not an error.
fn remove_plain_file(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => std::fs::remove_file(path),
        Ok(_) => {
            log::warn!(
                "{} is not a plain file; the cut does not delete it",
                path.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::sync::{Arc, Mutex};

    use recorder::speech::{Cell, ChannelCell, sidecar_of, write_sidecar};
    use session_core::Store;

    use super::*;
    use crate::Unattended;

    const ID: &str = "s";
    const MIN: u64 = 60_000;

    /// A note with one 16-kHz segment of ten chunks: speech, speech, six
    /// quiet chunks, speech, speech. With a cut time of 3 minutes, the cut
    /// removes the chunks 3 to 6. The live row has a word in each part.
    fn note(dir: &Path) -> SharedStore {
        let store: SharedStore = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let wall = chrono::DateTime::parse_from_rfc3339("2026-11-16T10:00:00+00:00").unwrap();
        let recovery = dir.join(RECOVERY_DIR);
        let mut writer = recorder::ChunkedWriter::with_probe(
            recorder::SegmentArgs {
                dir: recovery.clone(),
                segment: 0,
                capture_started_at: 1_800_000_000_000,
                offset_ms: 0,
                sample_rate: 16_000,
                channels: 1,
                floor_bytes: 0,
            },
            Box::new(|_| Some(u64::MAX)),
        )
        .unwrap();
        let audio: Vec<f32> = (0..10 * 60 * 16_000usize)
            .map(|i| ((i * 7_919 % 2_003) as f32 / 2_003.0 - 0.5) * 0.5)
            .collect();
        for block in audio.chunks(320) {
            writer.write(block).unwrap();
        }
        writer.finish().unwrap();
        for chunk in recorder::list_chunks(&recovery).unwrap() {
            let speech = matches!(chunk.start_ms / MIN, 0 | 1 | 8 | 9);
            let cell = Cell {
                mic: ChannelCell {
                    voiced: if speech { 255 } else { 0 },
                    level: if speech { 20 } else { 70 },
                },
                app: None,
            };
            write_sidecar(&sidecar_of(&chunk), 1, &[cell; 240], File::sync_all).unwrap();
        }
        {
            let mut s = lock(&store);
            s.create_session(ID, wall, None).unwrap();
            s.put_segment(
                ID,
                &Segment {
                    index: 0,
                    offset_ms: 0,
                    duration_ms: 10 * MIN,
                    wall_start: wall,
                    wall_end: wall + chrono::Duration::minutes(10),
                    audio_file: crate::recover::segment_audio_file(0),
                    channels: 1,
                    ends_in_pause: false,
                    cut_before_ms: 0,
                    cut_after_ms: 0,
                },
            )
            .unwrap();
            s.set_recording_guards(ID, Some(r#"{"cut_minutes":3,"countdown_minutes":2}"#))
                .unwrap();
        }
        let mut reader = ArchiveReader::new(Archive {
            dir: recovery,
            segment: 0,
            offset_ms: 0,
            rate: 16_000,
        });
        while reader.next_frame().unwrap().is_some() {}
        let config = live::LiveConfig::new(ID, 0, "model", 16_000);
        archive::save_proof(&store, &config, &reader).unwrap();
        let word = |at: u64| asr::LiveWord {
            start_ms: at,
            end_ms: at + 400,
            text: "word".into(),
        };
        live::save_part(
            &store,
            ID,
            0,
            0,
            &asr::LivePart {
                start_ms: 0,
                end_ms: 10 * MIN,
                words: vec![word(30_000), word(480_000)],
            },
        )
        .unwrap();
        store
    }

    /// Everything that the cut changes.
    #[derive(Debug, PartialEq)]
    struct State {
        segments: Vec<Segment>,
        checked: BTreeSet<u32>,
        files: Vec<String>,
        transcribe: Vec<(String, String)>,
    }

    fn state(store: &SharedStore, dir: &Path) -> State {
        let mut files: Vec<String> = std::fs::read_dir(dir.join(RECOVERY_DIR))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        let guard = lock(store);
        State {
            segments: guard.segments(ID).unwrap(),
            checked: guard.cut_checked(ID).unwrap(),
            files,
            transcribe: guard.checkpoints(ID, Stage::Transcribe).unwrap(),
        }
    }

    fn crash(step: u8) {
        CRASH_AFTER.with(|c| c.set(step));
    }

    #[test]
    fn a_crash_after_each_step_finishes_the_cut_at_the_next_run() {
        let clean = tempfile::tempdir().unwrap();
        let store = note(clean.path());
        run(&store, ID, clean.path(), &Unattended).unwrap();
        let expected = state(&store, clean.path());
        assert_eq!(expected.segments.len(), 2);
        assert_eq!(expected.files.len(), 12);
        for step in 1..=4 {
            let dir = tempfile::tempdir().unwrap();
            let store = note(dir.path());
            crash(step);
            assert!(run(&store, ID, dir.path(), &Unattended).is_err());
            crash(0);
            assert!(unfinished(&store, ID).unwrap(), "step {step}");
            // Launch recovery makes no row for a chunk while the cut is
            // not done.
            let before = lock(&store).segments(ID).unwrap();
            crate::recover::register_chunks(&store, ID, dir.path(), None).unwrap();
            assert_eq!(lock(&store).segments(ID).unwrap(), before, "step {step}");
            run(&store, ID, dir.path(), &Unattended).unwrap();
            assert!(!unfinished(&store, ID).unwrap());
            assert_eq!(state(&store, dir.path()), expected, "step {step}");
        }
    }

    #[test]
    fn a_rename_that_a_crash_interrupted_continues() {
        let clean = tempfile::tempdir().unwrap();
        let store = note(clean.path());
        run(&store, ID, clean.path(), &Unattended).unwrap();
        let expected = state(&store, clean.path());

        let dir = tempfile::tempdir().unwrap();
        let store = note(dir.path());
        crash(2);
        assert!(run(&store, ID, dir.path(), &Unattended).is_err());
        crash(0);
        // One chunk of part B has its new name, and its sidecar does not.
        let recovery = dir.path().join(RECOVERY_DIR);
        let chunk = recorder::list_chunks(&recovery)
            .unwrap()
            .into_iter()
            .find(|c| c.start_ms == 8 * MIN)
            .unwrap();
        std::fs::rename(&chunk.path, recovery.join(chunk_name(1, &chunk))).unwrap();
        run(&store, ID, dir.path(), &Unattended).unwrap();
        assert_eq!(state(&store, dir.path()), expected);
    }
}
