//! The saved session folder's markdown (R24, R24a, R24b, R36, R20a).
//!
//! Each file renders from the session's data and parses back, so the session
//! can be rebuilt from its folder alone (R25). Positions are rendered as
//! `hh:mm:ss`, so a round trip is exact to the second.

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker_count: Option<SpeakerCount>,
    /// The recognition model that made the transcript (R3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_snapshot: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recognition_artifact: Option<crate::recognition::ArtifactReference>,
    /// The number of speakers that processing found (R3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub found_speakers: Option<u32>,
    /// The [`fingerprint`] of `transcript.md` when `session.md` was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_fingerprint: Option<String>,
    /// The [`fingerprint`] of `summary.md` when `session.md` was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary_fingerprint: Option<String>,
    pub segments: Vec<SegmentRecord>,
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
}

fn is_mono(channels: &u16) -> bool {
    *channels == 1
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
        }
    }
}

impl SessionDoc {
    pub fn new(
        session_id: impl Into<String>,
        speaker_count: Option<SpeakerCount>,
        segments: &[Segment],
    ) -> Option<Self> {
        let first = segments.first()?;
        Some(Self {
            session_id: session_id.into(),
            started: first.wall_start,
            title: None,
            title_source: None,
            speaker_count,
            asr_model: None,
            recognition_snapshot: None,
            recognition_artifact: None,
            found_speakers: None,
            transcript_fingerprint: None,
            summary_fingerprint: None,
            segments: segments.iter().map(SegmentRecord::from).collect(),
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
            out.push_str(&format!(
                "\nGap of {} between segments {} and {}.",
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
    /// Between segments.
    Gap { duration_ms: u64 },
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
