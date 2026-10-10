//! The saved session folder's markdown (R24, R24a, R24b, R36, R20a).
//!
//! Each file renders from the session's data and parses back, so the session
//! can be rebuilt from its folder alone (R25). Positions are rendered as
//! `hh:mm:ss`, so a round trip is exact to the second.
//!
//! A removal marker beside a segment audio file records that the audio of
//! the segment was removed (KTD1 of the audio removal plan).

use std::path::Path;

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

use crate::jobs::{Title, TitleSource};
use crate::timeline::Segment;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FolderError {
    #[error("{file}: missing YAML frontmatter")]
    MissingFrontmatter { file: &'static str },
    #[error("{file}: {message}")]
    Yaml { file: &'static str, message: String },
    #[error("{file} line {line}: {message}")]
    Line {
        file: &'static str,
        line: usize,
        message: String,
    },
}

/// An optional bound on the number of speakers (R36). Recorded as a count,
/// never as an identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SpeakerCount {
    Exact(u32),
    Range { min: u32, max: u32 },
}

// ---------------------------------------------------------------- positions

pub fn format_position(ms: u64) -> String {
    let secs = ms / 1_000;
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3_600,
        (secs / 60) % 60,
        secs % 60
    )
}

pub fn parse_position(text: &str) -> Option<u64> {
    let mut parts = text.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || m.len() != 2 || s.len() != 2 {
        return None;
    }
    let (h, m, s): (u64, u64, u64) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    (m < 60 && s < 60).then_some(((h * 60 + m) * 60 + s) * 1_000)
}

/// Split a file into its frontmatter and its body. The file can have LF or
/// CRLF line ends, because a Windows tool can rewrite it (KTD2).
fn split_frontmatter<'a>(
    file: &'static str,
    text: &'a str,
) -> Result<(&'a str, &'a str), FolderError> {
    let missing = FolderError::MissingFrontmatter { file };
    let rest = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"))
        .ok_or(missing.clone())?;
    let mut from = 0;
    loop {
        let end = from + rest[from..].find("\n---").ok_or(missing.clone())?;
        let after = &rest[end + 4..];
        if let Some(body) = after
            .strip_prefix('\n')
            .or_else(|| after.strip_prefix("\r\n"))
        {
            return Ok((&rest[..end + 1], body));
        }
        from = end + 4;
    }
}

/// The fingerprint of a file's bytes: the SHA-256 as lowercase hex.
/// `session.md` records the fingerprints of `transcript.md` and
/// `summary.md` (KTD2).
pub fn fingerprint(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn yaml_error(file: &'static str) -> impl Fn(serde_yaml::Error) -> FolderError {
    move |e| FolderError::Yaml {
        file,
        message: e.to_string(),
    }
}

// ---------------------------------------------------------------- session.md

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDoc {
    pub session_id: String,
    pub started: DateTime<FixedOffset>,
    /// `None` until processing proposes a title or the user sets one (R28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Where the title came from. A file written before this field has
    /// none; [`SessionDoc::title_and_source`] then gives the user (R5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_source: Option<TitleSource>,
    /// The tags of the note, as their exact texts (R11, KTD2). A user
    /// field: each writer copies it from the file that it replaces.
    #[serde(
        rename = "longhand_tags",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub tags: Vec<String>,
    /// A type change that waits for its move (KTD2, KTD3). A user field:
    /// each writer copies it from the file that it replaces.
    #[serde(
        rename = "longhand_pending_type",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub pending_type: Option<PendingType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_count: Option<SpeakerCount>,
    /// The recognition model that made the transcript (R3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_artifact: Option<crate::recognition::ArtifactReference>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_audio: Option<std::collections::BTreeMap<String, String>>,
    /// The number of speakers that processing found (R3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_speakers: Option<u32>,
    /// The [`fingerprint`] of `transcript.md` when `session.md` was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_fingerprint: Option<String>,
    /// The [`fingerprint`] of `summary.md` when `session.md` was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_fingerprint: Option<String>,
    /// Why the note has no result yet (R6, KTD1). `None` once a result is
    /// saved. A file that an older build wrote can lose the field; read the
    /// state with [`SessionDoc::result_flag`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ResultFlag>,
    /// Empty for a note with no audio. A file with no field has none.
    #[serde(default)]
    pub segments: Vec<SegmentRecord>,
}

/// The `longhand_pending_type` field of `session.md` (KTD2, KTD3, KTD13):
/// the user changed the note type, and the home folder waits for its move.
/// `None` as a type is General: the home folder is directly in the
/// storage folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingType {
    /// The new type.
    pub to: Option<String>,
    /// The type of the folder that the home folder was in when the user
    /// changed the type. A second change keeps this value.
    pub from: Option<String>,
    /// The ID of the computer that does the move.
    pub by: String,
    /// The time of the change, or of the takeover of the move (KTD13).
    pub at: DateTime<FixedOffset>,
}

/// The keys of the user fields in the frontmatter of `session.md` (KTD2).
const USER_FIELDS: [&str; 2] = ["longhand_tags", "longhand_pending_type"];

/// The [`fingerprint`] of the bytes of a `session.md`, with the user
/// fields removed from its frontmatter (KTD2). A check that compares a
/// whole `session.md` uses it, so a tag or type edit does not fail a run.
/// The body counts. Bytes that are not a `session.md` with a frontmatter
/// mapping give the [`fingerprint`] of the bytes.
pub fn session_fingerprint(bytes: &[u8]) -> String {
    let without = std::str::from_utf8(bytes).ok().and_then(|text| {
        let (yaml, body) = split_frontmatter("session.md", text).ok()?;
        let mut map: serde_yaml::Mapping = serde_yaml::from_str(yaml).ok()?;
        // `shift_remove` keeps the order of the other keys.
        for key in USER_FIELDS {
            map.shift_remove(key);
        }
        let yaml = serde_yaml::to_string(&map).ok()?;
        Some(format!("---\n{yaml}---\n{body}"))
    });
    fingerprint(without.as_deref().map_or(bytes, str::as_bytes))
}

/// The `result` field of `session.md` (R6, KTD1): the note has no result
/// yet, and this tells why. A missing transcript, summary or audio of a
/// note with this flag does not mean that the note waits for a sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResultFlag {
    /// The note never recorded.
    #[serde(rename = "none")]
    NoRecording,
    /// The note has a recording that is not saved yet.
    #[serde(rename = "pending")]
    NotSaved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentRecord {
    pub index: u32,
    pub wall_start: DateTime<FixedOffset>,
    pub wall_end: DateTime<FixedOffset>,
    pub offset_ms: u64,
    pub duration_ms: u64,
    pub audio: String,
    /// 2 for a stereo segment. A mono segment writes no field, so its
    /// record stays as before (KTD6 of the system audio plan).
    #[serde(default = "crate::timeline::mono", skip_serializing_if = "is_mono")]
    pub channels: u16,
    /// True when the note paused after this segment. A segment with no
    /// pause writes no field, so its record stays as before (KTD4 of the
    /// pause plan).
    #[serde(default, skip_serializing_if = "is_false")]
    pub ends_in_pause: bool,
}

fn is_mono(channels: &u16) -> bool {
    *channels == 1
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<&Segment> for SegmentRecord {
    fn from(s: &Segment) -> Self {
        Self {
            index: s.index,
            wall_start: s.wall_start,
            wall_end: s.wall_end,
            offset_ms: s.offset_ms,
            duration_ms: s.duration_ms,
            audio: s.audio_file.clone(),
            channels: s.channels,
            ends_in_pause: s.ends_in_pause,
        }
    }
}

impl From<&SegmentRecord> for Segment {
    fn from(r: &SegmentRecord) -> Self {
        Self {
            index: r.index,
            offset_ms: r.offset_ms,
            duration_ms: r.duration_ms,
            wall_start: r.wall_start,
            wall_end: r.wall_end,
            audio_file: r.audio.clone(),
            channels: r.channels,
            ends_in_pause: r.ends_in_pause,
        }
    }
}

impl SessionDoc {
    /// The document of a note that starts at its first segment. `None`
    /// when the note has no segment.
    pub fn new(
        session_id: impl Into<String>,
        speaker_count: Option<SpeakerCount>,
        segments: &[Segment],
    ) -> Option<Self> {
        let first = segments.first()?;
        Some(Self::at(
            session_id,
            first.wall_start,
            speaker_count,
            segments,
        ))
    }

    /// The document of a note that never recorded: `started` is the time
    /// the note opened (R16), and the flag is
    /// [`ResultFlag::NoRecording`] (R6).
    pub fn without_audio(
        session_id: impl Into<String>,
        started: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
    ) -> Self {
        Self {
            result: Some(ResultFlag::NoRecording),
            ..Self::at(session_id, started, speaker_count, &[])
        }
    }

    /// The document of a note that starts at `started`, with no result
    /// flag. A note opened before it records keeps its open time (R16).
    pub fn at(
        session_id: impl Into<String>,
        started: DateTime<FixedOffset>,
        speaker_count: Option<SpeakerCount>,
        segments: &[Segment],
    ) -> Self {
        Self {
            session_id: session_id.into(),
            started,
            title: None,
            title_source: None,
            tags: Vec::new(),
            pending_type: None,
            speaker_count,
            asr_model: None,
            recognition_snapshot: None,
            recognition_artifact: None,
            recognition_audio: None,
            found_speakers: None,
            transcript_fingerprint: None,
            summary_fingerprint: None,
            result: None,
            segments: segments.iter().map(SegmentRecord::from).collect(),
        }
    }

    /// The result flag of the note (KTD1). With no `result` field, a note
    /// with no segment and no transcript or summary fingerprint reads as
    /// [`ResultFlag::NoRecording`]: an older build that rewrites the file
    /// drops the field, and the note must not wait for a sync. Any other
    /// note with no field has a result.
    pub fn result_flag(&self) -> Option<ResultFlag> {
        self.result.or_else(|| {
            (self.segments.is_empty()
                && self.transcript_fingerprint.is_none()
                && self.summary_fingerprint.is_none())
            .then_some(ResultFlag::NoRecording)
        })
    }

    pub fn timeline_segments(&self) -> Vec<Segment> {
        self.segments.iter().map(Segment::from).collect()
    }

    /// The title with its source. A title with no recorded source counts as
    /// set by the user, so a proposal never replaces it (R5, KTD2).
    pub fn title_and_source(&self) -> Option<Title> {
        self.title.as_ref().map(|text| Title {
            text: text.clone(),
            source: self.title_source.unwrap_or(TitleSource::User),
        })
    }

    /// The index of the first stereo segment, from the segment records
    /// (KTD2). `None` when every segment is mono.
    pub fn stereo_from(&self) -> Option<u32> {
        self.segments
            .iter()
            .find(|s| s.channels == 2)
            .map(|s| s.index)
    }

    pub fn render(&self) -> String {
        let yaml = serde_yaml::to_string(self).expect("session frontmatter serializes");
        let heading = self.title.as_deref().unwrap_or("Longhand session");
        let mut out = format!("---\n{yaml}---\n\n# {heading}\n\n");
        out.push_str(&format!(
            "Started {}.\n\n",
            self.started.format("%Y-%m-%d %H:%M")
        ));
        if let Some(count) = self.speaker_count {
            let text = match count {
                SpeakerCount::Exact(n) => format!("{n}"),
                SpeakerCount::Range { min, max } => format!("{min} to {max}"),
            };
            out.push_str(&format!("Speakers: {text}.\n\n"));
        }
        out.push_str(
            "| Segment | Starts | Ends | At | Length | Audio |\n|---|---|---|---|---|---|\n",
        );
        for s in &self.segments {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | [{}]({}) |\n",
                s.index + 1,
                s.wall_start.format("%H:%M:%S"),
                s.wall_end.format("%H:%M:%S"),
                format_position(s.offset_ms),
                format_position(s.duration_ms),
                s.audio,
                s.audio
            ));
        }
        for pair in self.segments.windows(2) {
            let gap = pair[1].offset_ms - (pair[0].offset_ms + pair[0].duration_ms);
            let kind = if pair[0].ends_in_pause {
                "Pause"
            } else {
                "Gap"
            };
            out.push_str(&format!(
                "\n{kind} of {} between segments {} and {}.",
                format_position(gap),
                pair[0].index + 1,
                pair[1].index + 1
            ));
        }
        if self.segments.len() > 1 {
            out.push('\n');
        }
        out
    }

    /// Only the frontmatter is authoritative; the body is for people.
    pub fn parse(text: &str) -> Result<Self, FolderError> {
        let (yaml, _) = split_frontmatter("session.md", text)?;
        serde_yaml::from_str(yaml).map_err(yaml_error("session.md"))
    }

    /// Read and parse the `session.md` in the note folder `folder`.
    pub fn read(folder: &Path) -> std::io::Result<Self> {
        Self::parse(&std::fs::read_to_string(folder.join("session.md"))?)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

// ---------------------------------------------------------------- removal markers

/// The end of the name of a removal marker: the marker of
/// `audio/segment-01.opus` is `audio/segment-01.opus.removed` (KTD1).
/// The name has no leading dot, because the folder watcher skips such
/// names.
pub const REMOVAL_MARKER_SUFFIX: &str = ".removed";

/// The content of each removal marker. The content is fixed, so two
/// computers that write one marker at the same time make the same file.
/// A reader does not open a marker: a marker with any content counts.
pub const REMOVAL_MARKER_TEXT: &str = "Longhand removed the audio of this segment.\n";

/// The note type that the position of the home folder `home` in the
/// storage folder `root` gives (KTD1, KTD4). `Some(None)` is a home folder
/// directly in `root`: it is in General. `Some(Some(name))` is a home folder
/// in the type folder `name` in `root`. `None` is a home folder that is not
/// at depth 0 or 1 of `root`, or that is in a hidden folder. This reads no
/// file.
pub fn note_type_in(root: &Path, home: &Path) -> Option<Option<String>> {
    let parent = home.parent()?;
    if parent == root {
        return Some(None);
    }
    if parent.parent()? != root {
        return None;
    }
    let name = parent.file_name()?.to_string_lossy();
    if name.starts_with('.') {
        return None;
    }
    Some(Some(name.into_owned()))
}

/// The path of the removal marker of the segment audio file `audio`, in
/// the same form as `audio`: relative to the note folder.
pub fn removal_marker(audio: &str) -> String {
    format!("{audio}{REMOVAL_MARKER_SUFFIX}")
}

/// Whether the note folder `dir` has the removal marker of the segment
/// audio file `audio`. This reads metadata only and opens no file, so an
/// online-only file is not downloaded (R12 of the library plan). An error
/// counts as no marker.
pub fn audio_removed(dir: &Path, audio: &str) -> bool {
    dir.join(removal_marker(audio))
        .try_exists()
        .unwrap_or(false)
}

/// The audio of a note, from its folder alone (KTD9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioState {
    /// Every segment has its audio file and no removal marker.
    Present,
    /// At least one segment has a removal marker. Its audio file can still
    /// be on disk (KTD2). A marked segment does not get its audio back, so
    /// this state wins over [`AudioState::Waiting`].
    Removed,
    /// At least one segment has no audio file and no removal marker: the
    /// sync can still deliver the file or its marker.
    Waiting,
}

/// The [`AudioState`] of the note folder `dir` whose segments have the
/// audio files `audio`. This reads metadata only and opens no file.
pub fn audio_state<'a>(dir: &Path, audio: impl IntoIterator<Item = &'a str>) -> AudioState {
    let mut state = AudioState::Present;
    for audio in audio {
        if audio_removed(dir, audio) {
            return AudioState::Removed;
        }
        if !dir.join(audio).is_file() {
            state = AudioState::Waiting;
        }
    }
    state
}

// ---------------------------------------------------------------- transcript.md

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptLine {
    Turn {
        position_ms: u64,
        /// `None` when diarization failed (R20a).
        speaker: Option<u32>,
        text: String,
    },
    /// Between segments, where the note stopped and resumed or lost audio.
    Gap { duration_ms: u64 },
    /// Between segments, where the user paused the note (KTD5 of the pause
    /// plan). Processing does not treat this time as missing audio.
    Pause { duration_ms: u64 },
    /// A within-segment silence past the threshold.
    Silence { duration_ms: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    /// False when diarization failed; the file then says so at its head.
    pub has_speakers: bool,
    pub lines: Vec<TranscriptLine>,
}

const SPEAKERS_OFF_NOTE: &str = "> Speaker recognition was off for this transcript.";

const NO_SPEAKERS_NOTE: &str =
    "> This transcript carries no speaker marks: speaker separation failed for this session.";

impl Transcript {
    pub fn render(&self) -> String {
        self.render_with_note(NO_SPEAKERS_NOTE)
    }

    /// Render a transcript whose selected speaker recognition mode was off.
    pub fn render_speaker_off(&self) -> String {
        self.render_with_note(SPEAKERS_OFF_NOTE)
    }

    fn render_with_note(&self, note: &str) -> String {
        let mut out = String::from("# Transcript\n\n");
        if !self.has_speakers {
            out.push_str(note);
            out.push_str("\n\n");
        }
        for line in &self.lines {
            match line {
                TranscriptLine::Turn {
                    position_ms,
                    speaker,
                    text,
                } => {
                    let text = one_line(text);
                    match speaker {
                        Some(n) => out.push_str(&format!(
                            "- [{}] Speaker {n}: {text}\n",
                            format_position(*position_ms)
                        )),
                        None => {
                            out.push_str(&format!("- [{}] {text}\n", format_position(*position_ms)))
                        }
                    }
                }
                TranscriptLine::Gap { duration_ms } => out.push_str(&format!(
                    "\n--- gap {} ---\n\n",
                    format_position(*duration_ms)
                )),
                TranscriptLine::Pause { duration_ms } => out.push_str(&format!(
                    "\n--- pause {} ---\n\n",
                    format_position(*duration_ms)
                )),
                TranscriptLine::Silence { duration_ms } => out.push_str(&format!(
                    "\n--- silence {} ---\n\n",
                    format_position(*duration_ms)
                )),
            }
        }
        out
    }

    pub fn parse(text: &str) -> Result<Self, FolderError> {
        const FILE: &str = "transcript.md";
        let mut has_speakers = true;
        let mut lines = Vec::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_end();
            let err = |message: &str| FolderError::Line {
                file: FILE,
                line: i + 1,
                message: message.to_string(),
            };
            if line.is_empty() || line.starts_with("# ") {
                continue;
            }
            if line == NO_SPEAKERS_NOTE || line == SPEAKERS_OFF_NOTE {
                has_speakers = false;
                continue;
            }
            if let Some(marker) = line
                .strip_prefix("--- ")
                .and_then(|l| l.strip_suffix(" ---"))
            {
                let (kind, time) = marker
                    .split_once(' ')
                    .ok_or_else(|| err("malformed marker"))?;
                let duration_ms = parse_position(time).ok_or_else(|| err("bad duration"))?;
                lines.push(match kind {
                    "gap" => TranscriptLine::Gap { duration_ms },
                    "pause" => TranscriptLine::Pause { duration_ms },
                    "silence" => TranscriptLine::Silence { duration_ms },
                    _ => return Err(err("unknown marker")),
                });
                continue;
            }
            let (position_ms, rest) =
                parse_bracketed(line).ok_or_else(|| err("expected `- [hh:mm:ss] ...`"))?;
            let (speaker, text) = match rest
                .strip_prefix("Speaker ")
                .and_then(|r| r.split_once(": "))
                .and_then(|(n, t)| n.parse::<u32>().ok().map(|n| (n, t)))
            {
                Some((n, t)) if has_speakers => (Some(n), t),
                _ => (None, rest),
            };
            lines.push(TranscriptLine::Turn {
                position_ms,
                speaker,
                text: text.to_string(),
            });
        }
        Ok(Self {
            has_speakers,
            lines,
        })
    }
}

/// `- [hh:mm:ss] rest` → (position, rest)
fn parse_bracketed(line: &str) -> Option<(u64, &str)> {
    let rest = line.strip_prefix("- [")?;
    let (time, rest) = rest.split_once("] ")?;
    Some((parse_position(time)?, rest))
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- notes.md

/// `notes.md` is the notes document as the user typed it (R25): no heading
/// is added and no line changes.
pub fn render_notes(document: &str) -> String {
    document.to_string()
}

// ---------------------------------------------------------------- summary.md

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SummaryMeta {
    /// Produced by the endpoint the user configured (R24b). Never the credential.
    Complete {
        endpoint_host: String,
        model: String,
        /// The name of the summary prompt that made the summary (R11). A
        /// summary made before prompts were recorded has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_name: Option<String>,
        /// The id of the summary prompt that made the summary (KTD4). The
        /// note page compares it with the prompt of the note type now
        /// (R9). A summary made before prompt ids were recorded has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_id: Option<String>,
    },
    /// Summarization failed; the session was saved anyway (R22).
    Stub { failed_stage: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Summary {
    pub meta: SummaryMeta,
    pub body: String,
}

impl Summary {
    pub fn render(&self) -> String {
        let yaml = serde_yaml::to_string(&self.meta).expect("summary frontmatter serializes");
        format!("---\n{yaml}---\n\n{}\n", self.body.trim_end())
    }

    pub fn parse(text: &str) -> Result<Self, FolderError> {
        let (yaml, body) = split_frontmatter("summary.md", text)?;
        Ok(Self {
            meta: serde_yaml::from_str(yaml).map_err(yaml_error("summary.md"))?,
            body: body.trim().to_string(),
        })
    }
}
