//! The long-silence cut (R12 to R16 of the forgotten-recording plan).
//!
//! [`detect`] finds the stretches of long silence in the current recording
//! and returns a [`CutPlan`] (KTD16, KTD17). It is a pure function of plain
//! data. [`read_input`] reads that data from the `audio-recovery/` folder of
//! a note: the chunk list and the speech sidecars of the segments of the
//! current recording. [`run`] applies the plan before Assemble (KTD15,
//! KTD18, KTD19).

mod apply;
mod detect;

pub use apply::{PLAN_KEY, origin_key, run, unfinished};
pub use detect::detect;

use std::collections::BTreeSet;
use std::path::Path;

use recorder::chunks::list_chunks;
use recorder::speech::{Cell, chunk_cells};
use session_core::Segment;

use crate::live::LiveRow;

/// The settings snapshot values that the cut uses (KTD13, KTD14).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutSettings {
    /// The long-silence cut time in minutes. `None` when the cut is off.
    pub cut_minutes: Option<u32>,
    /// The silence check time in minutes. `None` when the check is off.
    pub silence_check_minutes: Option<u32>,
    /// True when the silence check stopped the note automatically. Then the
    /// trailing stretch uses the smaller of the cut time and the silence
    /// check time (KTD17).
    pub stopped_by_silence_check: bool,
}

/// The data that [`detect`] reads.
#[derive(Debug, Clone, PartialEq)]
pub struct CutInput {
    /// Every segment of the note, with the chunks of the segments of the
    /// current recording. A segment of the saved prefix needs no chunks.
    pub segments: Vec<SegmentInput>,
    /// Where the current recording starts on the note timeline: at Start,
    /// at Resume, or at Record into a saved note. A segment that starts
    /// before this point is in the saved prefix, and the cut never reads or
    /// changes it.
    pub recording_start_ms: u64,
    /// The live words, on the note timeline.
    pub live_words: Vec<WordSpan>,
    pub settings: CutSettings,
}

/// One segment row and its chunks.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentInput {
    pub row: Segment,
    /// True when an earlier cut checked this segment.
    pub cut_checked: bool,
    /// The closed chunks of the segment, in time order.
    pub chunks: Vec<ChunkInput>,
}

/// One closed chunk and its cells.
#[derive(Debug, Clone, PartialEq)]
pub struct ChunkInput {
    /// The new audio of the chunk on the note timeline. The lookback does
    /// not count.
    pub start_ms: u64,
    pub end_ms: u64,
    /// One cell for each 250 ms. `None` is a cell with no data.
    pub cells: Vec<Option<Cell>>,
}

/// The time of one live word on the note timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordSpan {
    pub start_ms: u64,
    pub end_ms: u64,
}

/// One chunk of a segment, as the segment index and the new audio of the
/// chunk on the note timeline. The segment index is the index before the
/// cut.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ChunkSpan {
    pub segment: u32,
    pub start_ms: u64,
    pub end_ms: u64,
}

impl ChunkSpan {
    pub fn duration_ms(&self) -> u64 {
        self.end_ms - self.start_ms
    }
}

/// One segment of the current recording after the cut: a part of an
/// original segment (KTD16).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PlannedSegment {
    /// The index of the original segment before the cut.
    pub source: u32,
    /// The new segment row. The index is new: the cut numbers the segments
    /// of the current recording in time order, from the lowest index of the
    /// current recording up. The offset is the original note timeline
    /// position of the first kept chunk.
    pub row: Segment,
    /// The kept chunks of the part, in time order.
    pub chunks: Vec<ChunkSpan>,
}

/// The result of [`detect`] for the current recording.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CutPlan {
    /// The segments of the current recording after the cut, in time order.
    /// A segment that the cut removes completely has no entry.
    pub segments: Vec<PlannedSegment>,
    /// The chunks that the cut removes, in time order.
    pub removed: Vec<ChunkSpan>,
    /// The original indices of the segments that the cut checked. Each gets
    /// the `cut_checked` mark, so a second run finds nothing.
    pub checked: Vec<u32>,
}

impl CutPlan {
    /// True when the cut removes no audio.
    pub fn removes_nothing(&self) -> bool {
        self.removed.is_empty()
    }
}

/// Build the input of [`detect`] from the `audio-recovery/` folder
/// `recovery_dir` of a note. `segments` are the segment rows of the note,
/// `checked` the indices that [`session_core::Store::cut_checked`] gives,
/// and `live_rows` the live rows of the note. Only the chunks and the
/// sidecars of the segments of the current recording are read.
pub fn read_input(
    recovery_dir: &Path,
    segments: &[Segment],
    checked: &BTreeSet<u32>,
    live_rows: &[LiveRow],
    recording_start_ms: u64,
    settings: CutSettings,
) -> std::io::Result<CutInput> {
    let chunks = list_chunks(recovery_dir)?;
    let segments: Vec<SegmentInput> = segments
        .iter()
        .map(|row| {
            let current = row.offset_ms >= recording_start_ms;
            let chunks = if current {
                chunks
                    .iter()
                    .filter(|chunk| chunk.segment == row.index)
                    .map(|chunk| ChunkInput {
                        start_ms: chunk.start_ms,
                        end_ms: chunk.end_ms,
                        cells: chunk_cells(chunk),
                    })
                    .collect()
            } else {
                Vec::new()
            };
            SegmentInput {
                row: row.clone(),
                cut_checked: checked.contains(&row.index),
                chunks,
            }
        })
        .collect();
    // A live word is in milliseconds from the start of its segment.
    let live_words = live_rows
        .iter()
        .filter_map(|live| {
            let offset_ms = segments
                .iter()
                .find(|s| s.row.index == live.segment)?
                .row
                .offset_ms;
            Some(live.words.iter().map(move |word| WordSpan {
                start_ms: offset_ms + word.start_ms,
                end_ms: offset_ms + word.end_ms,
            }))
        })
        .flatten()
        .collect();
    Ok(CutInput {
        segments,
        recording_start_ms,
        live_words,
        settings,
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use chrono::DateTime;
    use recorder::speech::{ChannelCell, sidecar_path, write_sidecar};

    use super::*;

    fn segment(index: u32, offset_ms: u64, duration_ms: u64) -> Segment {
        let wall = DateTime::parse_from_rfc3339("2026-11-16T10:00:00+00:00").unwrap();
        Segment {
            index,
            offset_ms,
            duration_ms,
            wall_start: wall,
            wall_end: wall,
            audio_file: crate::recover::segment_audio_file(index),
            channels: 1,
            ends_in_pause: false,
            cut_before_ms: 0,
            cut_after_ms: 0,
        }
    }

    #[test]
    fn the_reader_reads_the_current_recording_only() {
        let dir = tempfile::tempdir().unwrap();
        let quiet = Cell {
            mic: ChannelCell {
                voiced: 0,
                level: 70,
            },
            app: None,
        };
        // Segment 0 is the saved prefix, segment 1 the current recording.
        for (segment, start, end) in [(0u32, 0u64, 60_000u64), (1, 60_000, 120_000)] {
            std::fs::write(
                dir.path()
                    .join(recorder::chunk_file_name(segment, 1700, start, end, start)),
                b"",
            )
            .unwrap();
            write_sidecar(
                &sidecar_path(dir.path(), segment, 1700, start),
                1,
                &vec![quiet; 240],
                File::sync_all,
            )
            .unwrap();
        }
        let rows = [segment(0, 0, 60_000), segment(1, 60_000, 60_000)];
        let live = [LiveRow {
            segment: 1,
            part: 0,
            start_ms: 0,
            end_ms: 60_000,
            words: vec![asr::LiveWord {
                start_ms: 1_000,
                end_ms: 1_500,
                text: "hello".into(),
            }],
        }];
        let settings = CutSettings {
            cut_minutes: Some(10),
            silence_check_minutes: Some(30),
            stopped_by_silence_check: false,
        };
        let input = read_input(
            dir.path(),
            &rows,
            &BTreeSet::from([0]),
            &live,
            60_000,
            settings,
        )
        .unwrap();
        assert!(input.segments[0].chunks.is_empty());
        assert!(input.segments[0].cut_checked);
        assert!(!input.segments[1].cut_checked);
        assert_eq!(
            input.segments[1].chunks,
            vec![ChunkInput {
                start_ms: 60_000,
                end_ms: 120_000,
                cells: vec![Some(quiet); 240],
            }]
        );
        assert_eq!(
            input.live_words,
            vec![WordSpan {
                start_ms: 61_000,
                end_ms: 61_500,
            }]
        );
    }
}
