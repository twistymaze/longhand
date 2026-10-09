//! One session timeline in monotonic milliseconds (KTD9, R3a, R16a).
//!
//! A segment's `offset_ms` is its start on the session timeline; a gap is the
//! difference between one segment's end and the next one's offset. Wall-clock
//! bounds ride along for display and `session.md`, and never enter the
//! arithmetic.

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

/// A segment under this length is dropped when it closes (R35).
pub const MIN_SEGMENT_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Segment {
    pub index: u32,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub wall_start: DateTime<FixedOffset>,
    pub wall_end: DateTime<FixedOffset>,
    /// Relative to the session folder, e.g. `audio/segment-01.opus`.
    pub audio_file: String,
    /// 1 for a mono segment. 2 for a stereo segment: the microphone on the
    /// left and system audio on the right (KTD6 of the system audio plan).
    #[serde(default = "mono")]
    pub channels: u16,
    /// True when the note paused after this segment (KTD4 of the pause
    /// plan). A resume of the note clears it.
    #[serde(default)]
    pub ends_in_pause: bool,
}

/// The channel count of a record that has no `channels` field: mono. The
/// timeline and the folder records both use it as their serde default.
pub(crate) fn mono() -> u16 {
    1
}

impl Segment {
    pub fn end_ms(&self) -> u64 {
        self.offset_ms + self.duration_ms
    }
}

/// The segment currently recording, whose end is not yet known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSegment {
    pub index: u32,
    pub offset_ms: u64,
    pub wall_start: DateTime<FixedOffset>,
    /// 1 until [`Timeline::set_open_channels`] makes it stereo.
    pub channels: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Location {
    /// Before the first segment starts.
    BeforeStart,
    InSegment {
        index: u32,
        within_ms: u64,
    },
    /// Between the end of segment `after` and the next segment's start, or
    /// after the last closed segment while nothing is recording.
    InGap {
        after: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TimelineError {
    #[error(
        "segment {index} starts at {offset_ms} ms, before the previous segment ends at {previous_end_ms} ms"
    )]
    Overlap {
        index: u32,
        offset_ms: u64,
        previous_end_ms: u64,
    },
    #[error("no segment {0}")]
    NoSuchSegment(u32),
    #[error("{within_ms} ms is past the end of segment {index} ({duration_ms} ms)")]
    PastSegmentEnd {
        index: u32,
        within_ms: u64,
        duration_ms: u64,
    },
    #[error("a segment is already open")]
    AlreadyOpen,
    #[error("no segment is open")]
    NotOpen,
}

/// What closing a segment did with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Closed {
    Kept(Segment),
    /// Shorter than [`MIN_SEGMENT_MS`] and dropped (R35).
    Discarded,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Timeline {
    segments: Vec<Segment>,
    open: Option<OpenSegment>,
}

impl Timeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuild from stored segments, as `session.md` or the job store hold them.
    pub fn from_segments(mut segments: Vec<Segment>) -> Result<Self, TimelineError> {
        segments.sort_by_key(|s| s.offset_ms);
        for pair in segments.windows(2) {
            if pair[1].offset_ms < pair[0].end_ms() {
                return Err(TimelineError::Overlap {
                    index: pair[1].index,
                    offset_ms: pair[1].offset_ms,
                    previous_end_ms: pair[0].end_ms(),
                });
            }
        }
        Ok(Self {
            segments,
            open: None,
        })
    }

    pub fn segments(&self) -> &[Segment] {
        &self.segments
    }

    pub fn open_segment(&self) -> Option<&OpenSegment> {
        self.open.as_ref()
    }

    /// The index the next opened segment takes. A dropped sub-second segment
    /// gives its index back, so saved audio files number without holes.
    fn next_index(&self) -> u32 {
        let closed = self.segments.last().map(|s| s.index + 1).unwrap_or(0);
        closed.max(self.open.as_ref().map(|o| o.index + 1).unwrap_or(0))
    }

    pub fn open(
        &mut self,
        offset_ms: u64,
        wall_start: DateTime<FixedOffset>,
    ) -> Result<u32, TimelineError> {
        if self.open.is_some() {
            return Err(TimelineError::AlreadyOpen);
        }
        let index = self.next_index();
        if let Some(last) = self.segments.last()
            && offset_ms < last.end_ms()
        {
            return Err(TimelineError::Overlap {
                index,
                offset_ms,
                previous_end_ms: last.end_ms(),
            });
        }
        self.open = Some(OpenSegment {
            index,
            offset_ms,
            wall_start,
            channels: 1,
        });
        Ok(index)
    }

    /// Set the channel count of the open segment: 2 for a stereo segment.
    /// The segment keeps it when it closes.
    pub fn set_open_channels(&mut self, channels: u16) -> Result<(), TimelineError> {
        let open = self.open.as_mut().ok_or(TimelineError::NotOpen)?;
        open.channels = channels;
        Ok(())
    }

    /// Forget the open segment: its stream never started, so it holds no
    /// audio and leaves no gap of its own.
    pub fn abandon_open(&mut self) {
        self.open = None;
    }

    /// Close the open segment at `end_ms` on the session timeline.
    pub fn close(
        &mut self,
        end_ms: u64,
        wall_end: DateTime<FixedOffset>,
        audio_file: String,
    ) -> Result<Closed, TimelineError> {
        let open = self.open.take().ok_or(TimelineError::NotOpen)?;
        let duration_ms = end_ms.saturating_sub(open.offset_ms);
        if duration_ms < MIN_SEGMENT_MS {
            return Ok(Closed::Discarded);
        }
        self.keep(open, duration_ms, wall_end, audio_file)
    }

    /// Close the open segment at an input change (KTD3 of the system audio
    /// plan). A segment shorter than [`MIN_SEGMENT_MS`] is kept, so a quick
    /// change or a Stop just after one loses no audio. Only a segment with
    /// no audio is discarded.
    pub fn close_at_switch(
        &mut self,
        end_ms: u64,
        wall_end: DateTime<FixedOffset>,
        audio_file: String,
    ) -> Result<Closed, TimelineError> {
        let open = self.open.take().ok_or(TimelineError::NotOpen)?;
        let duration_ms = end_ms.saturating_sub(open.offset_ms);
        if duration_ms == 0 {
            return Ok(Closed::Discarded);
        }
        self.keep(open, duration_ms, wall_end, audio_file)
    }

    fn keep(
        &mut self,
        open: OpenSegment,
        duration_ms: u64,
        wall_end: DateTime<FixedOffset>,
        audio_file: String,
    ) -> Result<Closed, TimelineError> {
        let segment = Segment {
            index: open.index,
            offset_ms: open.offset_ms,
            duration_ms,
            wall_start: open.wall_start,
            wall_end,
            audio_file,
            channels: open.channels,
            ends_in_pause: false,
        };
        self.segments.push(segment.clone());
        Ok(Closed::Kept(segment))
    }

    pub fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.open.is_none()
    }

    fn segment(&self, index: u32) -> Result<&Segment, TimelineError> {
        self.segments
            .iter()
            .find(|s| s.index == index)
            .ok_or(TimelineError::NoSuchSegment(index))
    }

    /// Gaps between consecutive closed segments, as (after index, gap ms).
    pub fn gaps(&self) -> Vec<(u32, u64)> {
        self.segments
            .windows(2)
            .map(|pair| (pair[0].index, pair[1].offset_ms - pair[0].end_ms()))
            .collect()
    }

    /// A point inside a segment, expressed on the session timeline.
    pub fn to_position(&self, index: u32, within_ms: u64) -> Result<u64, TimelineError> {
        if let Some(open) = &self.open
            && open.index == index
        {
            return Ok(open.offset_ms + within_ms);
        }
        let segment = self.segment(index)?;
        if within_ms > segment.duration_ms {
            return Err(TimelineError::PastSegmentEnd {
                index,
                within_ms,
                duration_ms: segment.duration_ms,
            });
        }
        Ok(segment.offset_ms + within_ms)
    }

    pub fn locate(&self, position_ms: u64) -> Location {
        if let Some(open) = &self.open
            && position_ms >= open.offset_ms
        {
            return Location::InSegment {
                index: open.index,
                within_ms: position_ms - open.offset_ms,
            };
        }
        let mut previous: Option<&Segment> = None;
        for segment in &self.segments {
            if position_ms < segment.offset_ms {
                return match previous {
                    Some(p) => Location::InGap { after: p.index },
                    None => Location::BeforeStart,
                };
            }
            // A segment's end instant belongs to it, so a position at the
            // end of a segment locates inside that segment.
            if position_ms <= segment.end_ms() {
                return Location::InSegment {
                    index: segment.index,
                    within_ms: position_ms - segment.offset_ms,
                };
            }
            previous = Some(segment);
        }
        match previous {
            Some(p) => Location::InGap { after: p.index },
            None => Location::BeforeStart,
        }
    }
}
