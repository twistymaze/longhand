//! Cut detection (KTD3, KTD4, KTD16, KTD17 of the forgotten-recording plan).
//!
//! The current recording is a list of whole chunks in time order. A Pause
//! divides it into blocks. A stretch is a run of removable chunks in one
//! block, so a stretch runs across a microphone change, and only a Pause, a
//! Resume or the Stop ends it.

use std::collections::VecDeque;

use chrono::Duration;
use recorder::speech::{
    Cell, QUIET_LEVEL_DBFS, QUIET_PERCENTILE, SPEECH_WINDOW_CELLS, SpeechWindow,
};
use session_core::Segment;

use super::{ChunkInput, ChunkSpan, CutInput, CutPlan, PlannedSegment, SegmentInput, WordSpan};
use crate::recover::segment_audio_file;

/// The kept audio between a removed stretch and the speech next to it is
/// whole chunks of at least this length (R13).
const MARGIN_MS: u64 = 60_000;
/// When the cut would remove all audio of the current recording, whole
/// chunks from the start of at least this length stay (R12).
const FIRST_AUDIO_MS: u64 = 60_000;

/// One chunk of the current recording.
struct Unit {
    /// The position of the segment in the current recording.
    segment: usize,
    span: ChunkSpan,
    /// A segment with no chunk gives one unit with no chunk. It is never
    /// removed.
    real: bool,
    removable: bool,
    /// The block of the chunk: a Pause starts a new block.
    block: usize,
    removed: bool,
}

/// The cut plan for the current recording of `input`. `None` when the cut
/// is off, when the note has no segment in the current recording, or when
/// an earlier cut checked the current recording.
pub fn detect(input: &CutInput) -> Option<CutPlan> {
    let settings = input.settings;
    let cut_ms = minutes_ms(settings.cut_minutes?);
    let mut current: Vec<&SegmentInput> = input
        .segments
        .iter()
        .filter(|s| s.row.offset_ms >= input.recording_start_ms)
        .collect();
    current.sort_by_key(|s| s.row.offset_ms);
    if current.is_empty() || current.iter().any(|s| s.cut_checked) {
        return None;
    }
    // The silent end of an automatic stop is cut also when the silence
    // check is shorter than the cut time (KTD13, KTD17).
    let trailing_ms = match settings.silence_check_minutes {
        Some(minutes) if settings.stopped_by_silence_check => cut_ms.min(minutes_ms(minutes)),
        _ => cut_ms,
    };

    let mut units = units(&current, &input.live_words);
    mark_stretches(&mut units, cut_ms, trailing_ms);
    // When every chunk would go, the first audio stays.
    if units.iter().all(|u| u.removed) {
        let mut kept_ms = 0;
        for unit in units.iter_mut() {
            if kept_ms >= FIRST_AUDIO_MS {
                break;
            }
            unit.removed = false;
            kept_ms += unit.span.duration_ms();
        }
    }
    Some(plan(&current, &units))
}

fn minutes_ms(minutes: u32) -> u64 {
    u64::from(minutes) * 60_000
}

/// The chunks of the current recording in time order.
fn units(current: &[&SegmentInput], words: &[WordSpan]) -> Vec<Unit> {
    let mut units = Vec::new();
    let mut block = 0;
    for (position, segment) in current.iter().enumerate() {
        if segment.chunks.is_empty() {
            units.push(Unit {
                segment: position,
                span: ChunkSpan {
                    segment: segment.row.index,
                    start_ms: segment.row.offset_ms,
                    end_ms: segment.row.end_ms(),
                },
                real: false,
                removable: false,
                block,
                removed: false,
            });
        }
        let removable = removable_chunks(&segment.chunks, words);
        for (chunk, removable) in segment.chunks.iter().zip(removable) {
            units.push(Unit {
                segment: position,
                span: ChunkSpan {
                    segment: segment.row.index,
                    start_ms: chunk.start_ms,
                    end_ms: chunk.end_ms,
                },
                real: true,
                removable,
                block,
                removed: false,
            });
        }
        if segment.row.ends_in_pause {
            block += 1;
        }
    }
    units
}

/// Whether the cut can remove each chunk of one segment (KTD17).
fn removable_chunks(chunks: &[ChunkInput], words: &[WordSpan]) -> Vec<bool> {
    // The speech test runs over the cells of the whole segment, so speech
    // across a chunk end counts in both chunks. A speech cell is a voiced
    // cell in a window that holds speech.
    let mut speech = vec![false; chunks.len()];
    let mut channels = [ChannelSpeech::default(), ChannelSpeech::default()];
    for (position, chunk) in chunks.iter().enumerate() {
        for cell in &chunk.cells {
            let mic = cell.as_ref().map(|c| c.mic);
            let app = cell.as_ref().and_then(|c| c.app);
            for (track, channel) in channels.iter_mut().zip([mic, app]) {
                track.push(
                    position,
                    channel.is_some_and(|c| c.is_voiced()),
                    &mut speech,
                );
            }
        }
    }
    chunks
        .iter()
        .zip(speech)
        .map(|(chunk, speech)| {
            !speech
                && complete(chunk)
                && quiet(chunk)
                && !words.iter().any(|w| {
                    w.start_ms < chunk.end_ms && w.end_ms.max(w.start_ms + 1) > chunk.start_ms
                })
        })
        .collect()
}

/// The speech test of one channel over the cells of a segment (KTD3).
#[derive(Default)]
struct ChannelSpeech {
    window: SpeechWindow,
    /// The chunk of each cell in the window, and whether the cell is voiced.
    recent: VecDeque<(usize, bool)>,
}

impl ChannelSpeech {
    /// Add the next cell, in the chunk `position`. When the window holds
    /// speech, mark the chunks of its voiced cells in `speech`.
    fn push(&mut self, position: usize, voiced: bool, speech: &mut [bool]) {
        self.recent.push_back((position, voiced));
        if self.recent.len() > SPEECH_WINDOW_CELLS as usize {
            self.recent.pop_front();
        }
        if self.window.push(voiced) {
            for &(p, voiced) in &self.recent {
                speech[p] |= voiced;
            }
        }
    }
}

/// Whether each cell of `chunk` has data, and no channel of a cell is both
/// voiced and above the quiet level (KTD17).
fn complete(chunk: &ChunkInput) -> bool {
    !chunk.cells.is_empty()
        && chunk.cells.iter().all(|cell| {
            cell.is_some_and(|cell| {
                cell.channels()
                    .all(|c| !(c.is_voiced() && c.level_dbfs() > QUIET_LEVEL_DBFS))
            })
        })
}

/// Whether the level at [`QUIET_PERCENTILE`] of the cells is at or below
/// [`QUIET_LEVEL_DBFS`] on every recorded channel (KTD4). Call it only for
/// a chunk whose cells all have data.
fn quiet(chunk: &ChunkInput) -> bool {
    let cells: Vec<Cell> = chunk.cells.iter().flatten().copied().collect();
    let mic: Vec<f32> = cells.iter().map(|c| c.mic.level_dbfs()).collect();
    let app: Vec<f32> = cells
        .iter()
        .filter_map(|c| c.app.map(|a| a.level_dbfs()))
        .collect();
    [mic, app]
        .into_iter()
        .filter(|levels| !levels.is_empty())
        .all(|levels| percentile(levels) <= QUIET_LEVEL_DBFS)
}

/// The nearest-rank percentile [`QUIET_PERCENTILE`] of `levels`.
fn percentile(mut levels: Vec<f32>) -> f32 {
    levels.sort_by(f32::total_cmp);
    let rank = (QUIET_PERCENTILE * levels.len() as f32).ceil() as usize;
    levels[rank.clamp(1, levels.len()) - 1]
}

/// Mark the removed chunks: for each stretch that is at least as long as
/// its limit, the chunks outside the margins (KTD17).
fn mark_stretches(units: &mut [Unit], cut_ms: u64, trailing_ms: u64) {
    let mut start = 0;
    while start < units.len() {
        if !units[start].removable {
            start += 1;
            continue;
        }
        let block = units[start].block;
        let mut end = start;
        while end < units.len() && units[end].removable && units[end].block == block {
            end += 1;
        }
        // A side next to a chunk that stays in the same block is next to
        // speech. A side at a Pause, a Resume, the Stop or the recording
        // start keeps no margin.
        let speech_before = start > 0 && units[start - 1].block == block;
        let speech_after = end < units.len() && units[end].block == block;
        let limit = if end == units.len() {
            trailing_ms
        } else {
            cut_ms
        };
        let length: u64 = units[start..end].iter().map(|u| u.span.duration_ms()).sum();
        if length >= limit {
            let mut first = start;
            if speech_before {
                let mut kept_ms = 0;
                while first < end && kept_ms < MARGIN_MS {
                    kept_ms += units[first].span.duration_ms();
                    first += 1;
                }
            }
            let mut last = end;
            if speech_after {
                let mut kept_ms = 0;
                while last > first && kept_ms < MARGIN_MS {
                    last -= 1;
                    kept_ms += units[last].span.duration_ms();
                }
            }
            for unit in &mut units[first..last] {
                unit.removed = true;
            }
        }
        start = end;
    }
}

/// The segments after the cut, the removed chunks and the cut fields
/// (KTD16, KTD21).
fn plan(current: &[&SegmentInput], units: &[Unit]) -> CutPlan {
    let lowest = current.iter().map(|s| s.row.index).min().unwrap_or(0);
    let mut parts: Vec<PlannedSegment> = Vec::new();
    let mut removed = Vec::new();
    // The removed audio since the last kept part.
    let mut pending_ms = 0;
    // Whether the block has a kept part before the pending removal.
    let mut block_has_part = false;
    for (position, segment) in current.iter().enumerate() {
        // The chunks of the open part, and the removal before its start.
        let mut part: Vec<&Unit> = Vec::new();
        let mut before_ms = 0;
        for unit in units.iter().filter(|u| u.segment == position) {
            if unit.removed {
                if !part.is_empty() {
                    parts.push(part_of(segment, &std::mem::take(&mut part), before_ms));
                }
                removed.push(unit.span);
                pending_ms += unit.span.duration_ms();
                continue;
            }
            if part.is_empty() {
                // A removal after a kept part of the same block is a
                // silence line after that part. A removal at the start of
                // a block is a silence line before the new part.
                before_ms = 0;
                if block_has_part && let Some(previous) = parts.last_mut() {
                    previous.row.cut_after_ms += std::mem::take(&mut pending_ms);
                } else {
                    before_ms = std::mem::take(&mut pending_ms);
                }
                block_has_part = true;
            }
            part.push(unit);
        }
        if !part.is_empty() {
            parts.push(part_of(segment, &part, before_ms));
        }
        if segment.row.ends_in_pause {
            if let Some(last) = parts.last_mut() {
                last.row.cut_after_ms += std::mem::take(&mut pending_ms);
                last.row.ends_in_pause = true;
            }
            block_has_part = false;
        }
    }
    if let Some(last) = parts.last_mut() {
        last.row.cut_after_ms += pending_ms;
    }
    for (number, part) in parts.iter_mut().enumerate() {
        let index = lowest + number as u32;
        if index != part.source {
            part.row.audio_file = segment_audio_file(index);
        }
        part.row.index = index;
    }
    CutPlan {
        segments: parts,
        removed,
        checked: current.iter().map(|s| s.row.index).collect(),
    }
}

/// The row and the chunks of one part of `segment`.
fn part_of(segment: &SegmentInput, units: &[&Unit], cut_before_ms: u64) -> PlannedSegment {
    let row = &segment.row;
    let first = units.first().expect("a part has a chunk");
    let last = units.last().expect("a part has a chunk");
    let starts_segment = first.span.start_ms <= row.offset_ms || !first.real;
    let ends_segment = last.span.end_ms >= row.end_ms() || !last.real;
    let offset_ms = if starts_segment {
        row.offset_ms
    } else {
        first.span.start_ms
    };
    let end_ms = if ends_segment {
        row.end_ms()
    } else {
        last.span.end_ms
    };
    let wall_start = row.wall_start + Duration::milliseconds((offset_ms - row.offset_ms) as i64);
    let wall_end = if ends_segment {
        row.wall_end
    } else {
        wall_start + Duration::milliseconds((end_ms - offset_ms) as i64)
    };
    PlannedSegment {
        source: row.index,
        row: Segment {
            offset_ms,
            duration_ms: end_ms - offset_ms,
            wall_start,
            wall_end,
            ends_in_pause: false,
            cut_before_ms,
            cut_after_ms: 0,
            ..row.clone()
        },
        chunks: units.iter().filter(|u| u.real).map(|u| u.span).collect(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration};
    use recorder::speech::{Cell, ChannelCell};
    use session_core::Segment;

    use super::*;
    use crate::cut::{ChunkInput, ChunkSpan, CutSettings, SegmentInput, WordSpan};

    const MIN: u64 = 60_000;

    fn channel(voiced: u8, level: u8) -> ChannelCell {
        ChannelCell { voiced, level }
    }

    /// Room tone at -70 dBFS with no voiced frames.
    fn quiet(stereo: bool) -> Cell {
        Cell {
            mic: channel(0, 70),
            app: stereo.then(|| channel(0, 70)),
        }
    }

    /// The cells of one chunk:
    /// - `S`: speech on the microphone at -20 dBFS;
    /// - `Q`: quiet room tone;
    /// - `N`: noise with no speech at -35 dBFS (a loud air conditioner);
    /// - `A`: speech on the app audio channel only, at -25 dBFS;
    /// - `M`: quiet, but one cell has no data;
    /// - `B`: quiet, with a voiced burst of 1 s (4 cells) at -20 dBFS;
    /// - `b`: quiet, with a voiced burst of 0.75 s (3 cells) at -20 dBFS,
    ///   too short to be speech (KTD3).
    fn cells(kind: char, stereo: bool, count: usize) -> Vec<Option<Cell>> {
        let base = quiet(stereo);
        (0..count)
            .map(|i| {
                let burst = |n: usize| (100..100 + n).contains(&i);
                match kind {
                    'S' => Some(Cell {
                        mic: channel(255, 20),
                        ..base
                    }),
                    'Q' => Some(base),
                    'N' => Some(Cell {
                        mic: channel(0, 35),
                        app: stereo.then(|| channel(0, 35)),
                    }),
                    'A' => Some(Cell {
                        app: Some(channel(255, 25)),
                        ..base
                    }),
                    'M' => (i != 100).then_some(base),
                    'B' if burst(4) => Some(Cell {
                        mic: channel(255, 20),
                        ..base
                    }),
                    'b' if burst(3) => Some(Cell {
                        mic: channel(255, 20),
                        ..base
                    }),
                    'B' | 'b' => Some(base),
                    other => panic!("no chunk kind {other}"),
                }
            })
            .collect()
    }

    /// A segment with one chunk of `ms` for each `(kind, ms)`.
    fn segment_ms(
        index: u32,
        offset_ms: u64,
        chunks: &[(char, u64)],
        stereo: bool,
    ) -> SegmentInput {
        let wall = DateTime::parse_from_rfc3339("2026-11-16T10:00:00+00:00").unwrap()
            + Duration::milliseconds(offset_ms as i64);
        let mut start = offset_ms;
        let chunks: Vec<ChunkInput> = chunks
            .iter()
            .map(|&(kind, ms)| {
                let chunk = ChunkInput {
                    start_ms: start,
                    end_ms: start + ms,
                    cells: cells(kind, stereo, ms.div_ceil(250) as usize),
                };
                start += ms;
                chunk
            })
            .collect();
        let duration_ms = start - offset_ms;
        SegmentInput {
            row: Segment {
                index,
                offset_ms,
                duration_ms,
                wall_start: wall,
                wall_end: wall + Duration::milliseconds(duration_ms as i64),
                audio_file: crate::recover::segment_audio_file(index),
                channels: if stereo { 2 } else { 1 },
                ends_in_pause: false,
                cut_before_ms: 0,
                cut_after_ms: 0,
            },
            cut_checked: false,
            chunks,
        }
    }

    /// A segment with one 60-s chunk for each character of `minutes`.
    fn segment(index: u32, offset_ms: u64, minutes: &str) -> SegmentInput {
        let chunks: Vec<(char, u64)> = minutes.chars().map(|c| (c, MIN)).collect();
        segment_ms(index, offset_ms, &chunks, false)
    }

    fn stereo_segment(index: u32, offset_ms: u64, minutes: &str) -> SegmentInput {
        let chunks: Vec<(char, u64)> = minutes.chars().map(|c| (c, MIN)).collect();
        segment_ms(index, offset_ms, &chunks, true)
    }

    fn paused(mut segment: SegmentInput) -> SegmentInput {
        segment.row.ends_in_pause = true;
        segment
    }

    fn settings(cut: Option<u32>, silence: Option<u32>, automatic: bool) -> CutSettings {
        CutSettings {
            cut_minutes: cut,
            silence_check_minutes: silence,
            stopped_by_silence_check: automatic,
        }
    }

    /// The default settings and a stop by the user.
    fn defaults() -> CutSettings {
        settings(Some(10), Some(30), false)
    }

    fn input(segments: Vec<SegmentInput>, settings: CutSettings) -> CutInput {
        CutInput {
            recording_start_ms: segments[0].row.offset_ms,
            segments,
            live_words: Vec::new(),
            settings,
        }
    }

    /// The minutes of the note timeline where the removed chunks start.
    fn removed_minutes(plan: &CutPlan) -> Vec<u64> {
        plan.removed.iter().map(|c| c.start_ms / MIN).collect()
    }

    fn minutes(range: std::ops::Range<u64>) -> Vec<u64> {
        range.collect()
    }

    fn removed_ms(plan: &CutPlan) -> u64 {
        plan.removed.iter().map(ChunkSpan::duration_ms).sum()
    }

    fn kept_ms(plan: &CutPlan) -> u64 {
        plan.segments
            .iter()
            .flat_map(|s| &s.chunks)
            .map(ChunkSpan::duration_ms)
            .sum()
    }

    /// The silence lines that the cut fields give, in time order.
    fn silence_lines(plan: &CutPlan) -> Vec<u64> {
        plan.segments
            .iter()
            .flat_map(|s| [s.row.cut_before_ms, s.row.cut_after_ms])
            .filter(|&ms| ms > 0)
            .collect()
    }

    #[test]
    fn ae1_the_trailing_stretch_keeps_one_to_two_minutes_after_the_last_speech() {
        // The meeting ends after 10 minutes. The silence check triggers 30
        // minutes later, and the countdown ends 2 minutes after that.
        let note = segment(0, 0, &format!("{}{}", "S".repeat(10), "Q".repeat(32)));
        let plan = detect(&input(vec![note], settings(Some(10), Some(30), true))).unwrap();
        let last_speech_end = 10 * MIN;
        let kept_end = plan.segments.last().unwrap().row.end_ms();
        let margin = kept_end - last_speech_end;
        assert!((MIN..=2 * MIN).contains(&margin), "margin {margin}");
        // No margin at the Stop: the removal goes to the end.
        assert_eq!(plan.removed.last().unwrap().end_ms, 42 * MIN);
        assert_eq!(removed_ms(&plan), 42 * MIN - kept_end);
        // One silence line, at the end.
        assert_eq!(silence_lines(&plan), vec![removed_ms(&plan)]);
        assert_eq!(
            plan.segments.last().unwrap().row.cut_after_ms,
            removed_ms(&plan)
        );
    }

    #[test]
    fn ae3_a_35_minute_quiet_stretch_removes_31_to_33_whole_minutes() {
        let note = segment(
            0,
            0,
            &format!("{}{}{}", "S".repeat(5), "Q".repeat(35), "S".repeat(5)),
        );
        let plan = detect(&input(vec![note], defaults())).unwrap();
        let removed = removed_ms(&plan);
        assert!(
            (31 * MIN..=33 * MIN).contains(&removed),
            "removed {removed}"
        );
        assert_eq!(removed % MIN, 0);
        assert_eq!(silence_lines(&plan), vec![removed]);
        // The kept parts keep their place on the note timeline.
        assert_eq!(plan.segments.len(), 2);
        assert_eq!(plan.segments[0].row.offset_ms, 0);
        assert_eq!(plan.segments[1].row.end_ms(), 45 * MIN);
        assert_eq!(
            plan.segments[1].row.offset_ms - plan.segments[0].row.end_ms(),
            removed
        );
    }

    #[test]
    fn ae2_noisy_chunks_are_not_removed() {
        let note = segment(0, 0, &format!("{}{}", "S".repeat(10), "N".repeat(32)));
        let plan = detect(&input(vec![note], settings(Some(10), Some(30), true))).unwrap();
        assert!(plan.removes_nothing());
        assert_eq!(kept_ms(&plan), 42 * MIN);
    }

    #[test]
    fn ae5_speech_on_the_app_channel_only_blocks_the_cut() {
        let note = stereo_segment(0, 0, &format!("S{}S", "A".repeat(40)));
        let plan = detect(&input(vec![note], defaults())).unwrap();
        assert!(plan.removes_nothing());
        // The same minutes with a quiet app channel are cut.
        let note = stereo_segment(0, 0, &format!("S{}S", "Q".repeat(40)));
        let plan = detect(&input(vec![note], defaults())).unwrap();
        assert_eq!(removed_ms(&plan), 38 * MIN);
    }

    #[test]
    fn ae9_an_all_silent_recording_keeps_its_first_minute() {
        let note = segment(0, 0, &"Q".repeat(180));
        let plan = detect(&input(vec![note], settings(Some(10), None, false))).unwrap();
        assert_eq!(plan.segments.len(), 1);
        assert_eq!(plan.segments[0].row.offset_ms, 0);
        assert_eq!(plan.segments[0].row.duration_ms, MIN);
        assert_eq!(removed_ms(&plan), 179 * MIN);
        assert_eq!(silence_lines(&plan), vec![179 * MIN]);
    }

    #[test]
    fn ae9_the_first_minute_continues_into_the_next_segment_after_a_mic_change() {
        // A microphone change ends the first segment after 20 s.
        let first = segment_ms(0, 0, &[('Q', 20_000)], false);
        let second = segment(1, 20_000, &"Q".repeat(180));
        let plan = detect(&input(vec![first, second], defaults())).unwrap();
        assert!(kept_ms(&plan) >= MIN, "kept {}", kept_ms(&plan));
        // Whole chunks from the start: 20 s and one minute.
        assert_eq!(kept_ms(&plan), 80_000);
        assert_eq!(plan.segments[0].row.offset_ms, 0);
        assert_eq!(plan.segments[1].row.offset_ms, 20_000);
        assert_eq!(plan.segments[1].row.duration_ms, MIN);
        assert_eq!(silence_lines(&plan), vec![179 * MIN]);
    }

    #[test]
    fn a_stretch_across_a_mic_change_removes_the_segment_between_as_one_stretch() {
        let a = segment(0, 0, "SSQQQ");
        let b = segment(1, 5 * MIN, "QQQQQ");
        let c = segment(2, 10 * MIN, "QQQSS");
        let plan = detect(&input(vec![a, b, c], defaults())).unwrap();
        // 11 quiet minutes, with a margin of one minute on each side.
        assert_eq!(removed_minutes(&plan), minutes(3..12));
        let sources: Vec<u32> = plan.segments.iter().map(|s| s.source).collect();
        assert_eq!(sources, vec![0, 2]);
        assert!(plan.removed.iter().any(|c| c.segment == 1));
        assert_eq!(silence_lines(&plan), vec![9 * MIN]);
        assert_eq!(plan.segments[0].row.end_ms(), 3 * MIN);
        assert_eq!(plan.segments[1].row.offset_ms, 12 * MIN);
    }

    #[test]
    fn a_pause_ends_a_stretch_and_keeps_no_margin_at_the_pause() {
        let before = paused(segment(0, 0, &format!("SS{}", "Q".repeat(12))));
        // Paused time is not on the timeline: the next segment starts
        // after the pause.
        let after = segment(1, 20 * MIN, &format!("{}SS", "Q".repeat(12)));
        let plan = detect(&input(vec![before, after], defaults())).unwrap();
        // A margin next to the speech only.
        let mut expected = minutes(3..14);
        expected.extend(minutes(20..31));
        assert_eq!(removed_minutes(&plan), expected);
        assert_eq!(plan.segments.len(), 2);
        let (a, b) = (&plan.segments[0].row, &plan.segments[1].row);
        assert!(a.ends_in_pause);
        assert!(!b.ends_in_pause);
        // A silence line before the pause and one after it.
        assert_eq!(a.cut_after_ms, 11 * MIN);
        assert_eq!(b.cut_before_ms, 11 * MIN);
    }

    #[test]
    fn a_quiet_stretch_that_reaches_a_pause_is_not_one_stretch_with_the_next() {
        // 6 quiet minutes before the pause and 6 after it: each stretch is
        // shorter than the cut time.
        let before = paused(segment(0, 0, &format!("SS{}", "Q".repeat(6))));
        let after = segment(1, 20 * MIN, &format!("{}SS", "Q".repeat(6)));
        let plan = detect(&input(vec![before, after], defaults())).unwrap();
        assert!(plan.removes_nothing());
    }

    #[test]
    fn the_saved_prefix_of_a_resumed_note_is_never_in_the_plan() {
        let prefix = segment(0, 0, &"Q".repeat(20));
        let current = segment(1, 30 * MIN, &format!("SS{}", "Q".repeat(15)));
        let mut note = input(vec![prefix, current], defaults());
        note.recording_start_ms = 30 * MIN;
        let plan = detect(&note).unwrap();
        assert!(plan.segments.iter().all(|s| s.source == 1));
        assert!(plan.removed.iter().all(|c| c.segment == 1));
        assert_eq!(plan.checked, vec![1]);
        assert_eq!(removed_minutes(&plan), minutes(33..47));
        // The current recording keeps its index.
        assert_eq!(plan.segments[0].row.index, 1);
    }

    #[test]
    fn a_missing_cell_blocks_the_removal_of_its_chunk() {
        let minutes_ = format!("S{}M{}S", "Q".repeat(12), "Q".repeat(12));
        let plan = detect(&input(vec![segment(0, 0, &minutes_)], defaults())).unwrap();
        assert!(!removed_minutes(&plan).contains(&13));
        let mut expected = minutes(2..12);
        expected.extend(minutes(15..25));
        assert_eq!(removed_minutes(&plan), expected);
    }

    #[test]
    fn a_live_word_in_a_quiet_chunk_blocks_its_removal() {
        let minutes_ = format!("S{}S", "Q".repeat(25));
        let mut note = input(vec![segment(0, 0, &minutes_)], defaults());
        note.live_words.push(WordSpan {
            start_ms: 13 * MIN + 30_000,
            end_ms: 13 * MIN + 30_400,
        });
        let plan = detect(&note).unwrap();
        assert!(!removed_minutes(&plan).contains(&13));
        let mut expected = minutes(2..12);
        expected.extend(minutes(15..25));
        assert_eq!(removed_minutes(&plan), expected);
    }

    #[test]
    fn a_stretch_shorter_than_the_cut_time_is_not_cut() {
        let note = segment(0, 0, &format!("S{}S", "Q".repeat(9)));
        let plan = detect(&input(vec![note], defaults())).unwrap();
        assert!(plan.removes_nothing());
    }

    #[test]
    fn an_11_minute_stretch_between_speech_is_cut_with_the_default() {
        let note = segment(0, 0, &format!("S{}S", "Q".repeat(11)));
        let plan = detect(&input(vec![note], defaults())).unwrap();
        assert_eq!(removed_minutes(&plan), minutes(2..11));
        assert_eq!(silence_lines(&plan), vec![9 * MIN]);
    }

    #[test]
    fn a_1_s_voiced_burst_in_a_quiet_stretch_blocks_its_chunk() {
        // 15 quiet minutes, the burst in the eighth. No live word: the live
        // transcript is off.
        for burst in ["B", "b"] {
            let minutes_ = format!("S{}{burst}{}S", "Q".repeat(7), "Q".repeat(7));
            let plan = detect(&input(vec![segment(0, 0, &minutes_)], defaults())).unwrap();
            // Each side of the burst is shorter than the cut time.
            assert!(plan.removes_nothing(), "burst {burst}");
        }
        // The same stretch with no burst is cut.
        let minutes_ = format!("S{}S", "Q".repeat(15));
        let plan = detect(&input(vec![segment(0, 0, &minutes_)], defaults())).unwrap();
        assert_eq!(removed_ms(&plan), 13 * MIN);
    }

    #[test]
    fn the_trailing_stretch_of_an_automatic_stop_uses_the_silence_check_time() {
        // A 5-minute silence check and a 2-minute countdown: 7 quiet
        // minutes, shorter than the 10-minute cut.
        let minutes_ = format!("SSS{}", "Q".repeat(7));
        let automatic = settings(Some(10), Some(5), true);
        let plan = detect(&input(vec![segment(0, 0, &minutes_)], automatic)).unwrap();
        assert_eq!(removed_minutes(&plan), minutes(4..10));
        // A stop by the user does not cut the same stretch.
        let by_user = settings(Some(10), Some(5), false);
        let plan = detect(&input(vec![segment(0, 0, &minutes_)], by_user)).unwrap();
        assert!(plan.removes_nothing());
    }

    #[test]
    fn the_silence_check_time_applies_to_the_trailing_stretch_only() {
        let minutes_ = format!("S{}S{}", "Q".repeat(7), "Q".repeat(7));
        let automatic = settings(Some(10), Some(5), true);
        let plan = detect(&input(vec![segment(0, 0, &minutes_)], automatic)).unwrap();
        assert_eq!(removed_minutes(&plan), minutes(10..16));
    }

    #[test]
    fn after_a_cut_the_indices_are_in_time_order() {
        // Segments 0 and 1 are the saved prefix; the current recording is
        // segments 2 and 3, and segment 2 splits in three parts.
        let prefix_a = segment(0, 0, "SS");
        let prefix_b = segment(1, 2 * MIN, "SS");
        let split = segment(
            2,
            10 * MIN,
            &format!("S{}S{}S", "Q".repeat(12), "Q".repeat(12)),
        );
        let last = segment(3, 37 * MIN, "SS");
        let mut note = input(vec![prefix_a, prefix_b, split, last], defaults());
        note.recording_start_ms = 10 * MIN;
        let plan = detect(&note).unwrap();
        let rows: Vec<(u32, u32, u64)> = plan
            .segments
            .iter()
            .map(|s| (s.source, s.row.index, s.row.offset_ms))
            .collect();
        assert_eq!(
            rows,
            vec![
                (2, 2, 10 * MIN),
                (2, 3, 22 * MIN),
                (2, 4, 35 * MIN),
                (3, 5, 37 * MIN),
            ]
        );
        assert_eq!(plan.segments[1].row.audio_file, "audio/segment-04.opus");
        assert_eq!(plan.checked, vec![2, 3]);
        // Each part starts and ends at a chunk boundary.
        for part in &plan.segments {
            assert_eq!(part.chunks.first().unwrap().start_ms, part.row.offset_ms);
            assert_eq!(part.chunks.last().unwrap().end_ms, part.row.end_ms());
        }
        // The wall clock moves with the offset.
        let first = &plan.segments[0].row;
        let second = &plan.segments[1].row;
        assert_eq!(
            second.wall_start - first.wall_start,
            Duration::milliseconds((12 * MIN) as i64)
        );
    }

    #[test]
    fn a_second_run_on_the_output_finds_nothing() {
        let note = segment(
            0,
            0,
            &format!(
                "{}{}{}{}",
                "S".repeat(5),
                "Q".repeat(35),
                "S".repeat(5),
                "Q".repeat(20)
            ),
        );
        let first = input(vec![note.clone()], defaults());
        let plan = detect(&first).unwrap();
        assert!(!plan.removes_nothing());

        // The segments after the cut, with the cut_checked mark.
        let chunks_of = |part: &crate::cut::PlannedSegment, checked: bool| SegmentInput {
            row: part.row.clone(),
            cut_checked: checked,
            chunks: part
                .chunks
                .iter()
                .map(|c| {
                    note.chunks
                        .iter()
                        .find(|n| n.start_ms == c.start_ms)
                        .unwrap()
                        .clone()
                })
                .collect(),
        };
        let marked: Vec<SegmentInput> = plan.segments.iter().map(|p| chunks_of(p, true)).collect();
        assert_eq!(detect(&input(marked, defaults())), None);

        // Also with no mark, the kept audio has no stretch to cut.
        let unmarked: Vec<SegmentInput> =
            plan.segments.iter().map(|p| chunks_of(p, false)).collect();
        let second = detect(&input(unmarked, defaults())).unwrap();
        assert!(second.removes_nothing());
    }

    #[test]
    fn the_cut_off_gives_no_plan() {
        let note = segment(0, 0, &"Q".repeat(60));
        assert_eq!(
            detect(&input(vec![note], settings(None, Some(30), true))),
            None
        );
    }
}
