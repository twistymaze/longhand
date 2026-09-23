//! The session list and each session's state, notes and landed documents,
//! read from the job store and the landed folder, which are the truth: a
//! window shown mid-processing rebuilds from these, then follows events
//! (U12, AE4).

use std::path::Path;
use std::sync::Arc;

use chrono::Local;
use serde::Serialize;
use session_core::folder::{SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Event, SessionRecord, SessionState, Stage};
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::capture::{CaptureManager, NoteView};
use crate::managers::processing::stage_name;
use crate::managers::settings;

#[derive(Serialize, Clone, Type)]
pub struct SessionEntry {
    pub id: String,
    /// When the first segment began (RFC 3339), or when the session was made.
    pub started: String,
    /// Recorded time, gaps excluded.
    pub duration_ms: u64,
    /// The lifecycle state's name: capturing, interrupted, queued,
    /// processing, blocked, processing_failed, awaiting_destination, landed.
    pub state: String,
    /// The stage started last: the running one, or the one a failed
    /// session stopped at.
    pub stage: Option<String>,
}

#[derive(Serialize, Type)]
pub struct EndpointView {
    pub host: String,
    pub model: String,
    /// False when the transcript would cross the network unencrypted.
    pub encrypted: bool,
}

#[derive(Serialize, Type)]
pub struct SessionDetail {
    pub entry: SessionEntry,
    /// Stages whose work is complete.
    pub done: Vec<String>,
    /// The stage started last.
    pub current: Option<String>,
    pub windows_done: Option<u32>,
    pub windows_total: Option<u32>,
    /// The stage a failed session stopped at.
    pub failed_stage: Option<String>,
    /// Why it failed, is blocked, or is waiting for its destination.
    pub message: Option<String>,
    /// Why the summary is missing, when the session will land with a stub.
    pub summary_problem: Option<String>,
    /// The state machine allows Resume now (R6a).
    pub resumable: bool,
    pub landed_path: Option<String>,
    /// Where the summary goes, and what is sent: the transcript and notes (R30).
    pub endpoint: Option<EndpointView>,
    pub audio_kept: bool,
    pub transcript_kept: bool,
}

#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocLine {
    Turn {
        position_ms: u64,
        speaker: Option<u32>,
        text: String,
    },
    /// Between segments; `at_ms` is where the earlier segment ends.
    Gap {
        at_ms: u64,
        duration_ms: u64,
    },
    Silence {
        duration_ms: u64,
    },
}

#[derive(Serialize, Type)]
pub struct SummaryView {
    /// Markdown.
    pub body: String,
    /// Summarization failed and this stands in for it.
    pub stub: bool,
    pub host: Option<String>,
    pub model: Option<String>,
}

#[derive(Serialize, Type)]
pub struct SessionDocument {
    pub has_speakers: bool,
    pub lines: Vec<DocLine>,
    pub summary: Option<SummaryView>,
}

fn entry(
    record: &SessionRecord,
    segments: &[session_core::Segment],
    stage: Option<Stage>,
) -> SessionEntry {
    let stage = match &record.state {
        SessionState::ProcessingFailed { stage, .. } => Some(*stage),
        SessionState::Processing { .. } => stage,
        _ => None,
    };
    SessionEntry {
        id: record.id.clone(),
        started: segments
            .first()
            .map_or(record.created_at, |s| s.wall_start)
            .to_rfc3339(),
        duration_ms: segments.iter().map(|s| s.duration_ms).sum(),
        state: record.state.name().to_string(),
        stage: stage.map(stage_name),
    }
}

/// Every session but the discarded ones: the switcher's list, newest first.
#[tauri::command]
#[specta::specta]
pub fn list_sessions(capture: State<'_, Arc<CaptureManager>>) -> Result<Vec<SessionEntry>, String> {
    let store = capture.shared_store();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let records = store.sessions().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for record in records.iter().rev() {
        if record.state != SessionState::Discarded {
            let segments = store.segments(&record.id).map_err(|e| e.to_string())?;
            let stage = store.current_stage(&record.id).map_err(|e| e.to_string())?;
            out.push(entry(record, &segments, stage));
        }
    }
    Ok(out)
}

fn has_files(dir: &Path) -> bool {
    std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some())
}

#[tauri::command]
#[specta::specta]
pub fn session_detail(
    app: AppHandle,
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<SessionDetail, String> {
    let shared = capture.shared_store();
    let e = |e: pipeline::StageError| e.to_string();
    let (record, segments, most_recent, landing) = {
        let store = shared.lock().unwrap_or_else(|e| e.into_inner());
        let record = store
            .session(&session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no session {session_id}"))?;
        let segments = store.segments(&session_id).map_err(|e| e.to_string())?;
        let most_recent = store
            .is_most_recent(&session_id)
            .map_err(|e| e.to_string())?;
        let landing = pipeline::land::begun(&store, &session_id).map_err(e)?;
        (record, segments, most_recent, landing)
    };
    let snapshot = pipeline::progress::snapshot(&shared, &session_id).map_err(e)?;

    let resumable = !landing
        && record
            .state
            .transition(&Event::UserResume {
                now: Local::now().fixed_offset(),
                is_most_recent: most_recent,
                space_ok: true,
            })
            .is_ok()
        && !matches!(record.state, SessionState::Capturing);

    let (failed_stage, message) = match &record.state {
        SessionState::ProcessingFailed { stage, .. } => (
            Some(*stage),
            pipeline::progress::error(&shared, &session_id, *stage).map_err(e)?,
        ),
        SessionState::Blocked { .. } => (
            None,
            pipeline::progress::error(&shared, &session_id, Stage::Summarize).map_err(e)?,
        ),
        SessionState::AwaitingDestination { problem, .. } => (None, Some(problem.clone())),
        _ => (None, None),
    };
    let summary_problem: Option<String> = pipeline::checkpoint::get(
        &shared,
        &session_id,
        Stage::Summarize,
        pipeline::land::SUMMARY_FAILED,
    )
    .map_err(e)?;

    let s = settings::get_settings(&app);
    let endpoint = s.endpoint_configured().then(|| EndpointView {
        host: secrets::endpoint_key(&s.endpoint_base_url)
            .unwrap_or_else(|_| s.endpoint_base_url.clone()),
        model: s.endpoint_model.clone(),
        encrypted: s.endpoint_base_url.trim().starts_with("https://"),
    });

    let working = capture.working_root().join(&session_id);
    let (audio_kept, transcript_kept) = match &record.landed_path {
        Some(path) => (
            has_files(&Path::new(path).join("audio")),
            Path::new(path)
                .join(pipeline::join::TRANSCRIPT_FILE)
                .exists(),
        ),
        None => (
            has_files(&working.join("audio")) || has_files(&working.join(pipeline::RECOVERY_DIR)),
            working.join(pipeline::join::TRANSCRIPT_FILE).exists(),
        ),
    };

    Ok(SessionDetail {
        entry: entry(&record, &segments, snapshot.current),
        done: snapshot.done.into_iter().map(stage_name).collect(),
        current: snapshot.current.map(stage_name),
        windows_done: snapshot.windows.map(|(d, _)| d as u32),
        windows_total: snapshot.windows.map(|(_, t)| t as u32),
        failed_stage: failed_stage.map(stage_name),
        message,
        summary_problem,
        resumable,
        landed_path: record.landed_path.clone(),
        endpoint,
        audio_kept,
        transcript_kept,
    })
}

/// A session's notes, editable whatever its state (R11a).
#[tauri::command]
#[specta::specta]
pub fn session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<Vec<NoteView>, String> {
    let store = capture.shared_store();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    Ok(store
        .notes(&session_id)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(NoteView::from)
        .collect())
}

/// The landed transcript and summary, read from the session's folder.
#[tauri::command]
#[specta::specta]
pub fn session_document(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<SessionDocument, String> {
    let landed = {
        let store = capture.shared_store();
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .session(&session_id)
            .map_err(|e| e.to_string())?
            .and_then(|r| r.landed_path)
            .ok_or_else(|| "this session has not landed yet".to_string())?
    };
    let folder = Path::new(&landed);
    let read = |name: &str| {
        std::fs::read_to_string(folder.join(name))
            .map_err(|e| format!("could not read {name} in {landed}: {e}"))
    };
    let doc = SessionDoc::parse(&read(pipeline::land::SESSION_FILE)?).map_err(|e| e.to_string())?;
    let transcript =
        Transcript::parse(&read(pipeline::join::TRANSCRIPT_FILE)?).map_err(|e| e.to_string())?;
    Ok(SessionDocument {
        has_speakers: transcript.has_speakers,
        lines: doc_lines(&doc, transcript),
        summary: read(pipeline::summarize::SUMMARY_FILE)
            .ok()
            .and_then(|text| Summary::parse(&text).ok())
            .map(summary_view),
    })
}

/// Transcript lines, each gap placed at the end of the segment before it.
fn doc_lines(doc: &SessionDoc, transcript: Transcript) -> Vec<DocLine> {
    let mut ends: Vec<u64> = doc
        .segments
        .iter()
        .map(|s| s.offset_ms + s.duration_ms)
        .collect();
    ends.sort_unstable();
    let mut gaps = ends.into_iter();
    transcript
        .lines
        .into_iter()
        .map(|line| match line {
            TranscriptLine::Turn {
                position_ms,
                speaker,
                text,
            } => DocLine::Turn {
                position_ms,
                speaker,
                text,
            },
            TranscriptLine::Gap { duration_ms } => DocLine::Gap {
                at_ms: gaps.next().unwrap_or_default(),
                duration_ms,
            },
            TranscriptLine::Silence { duration_ms } => DocLine::Silence { duration_ms },
        })
        .collect()
}

fn summary_view(summary: Summary) -> SummaryView {
    let body = summary.body;
    match summary.meta {
        SummaryMeta::Complete {
            endpoint_host,
            model,
        } => SummaryView {
            body,
            stub: false,
            host: Some(endpoint_host),
            model: Some(model),
        },
        SummaryMeta::Stub { .. } => SummaryView {
            body,
            stub: true,
            host: None,
            model: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gaps_sit_at_the_end_of_the_segment_before_them() {
        let text = "---\nsession_id: s\nstarted: 2026-09-22T14:00:00+00:00\nsegments:\n- index: 0\n  wall_start: 2026-09-22T14:00:00+00:00\n  wall_end: 2026-09-22T14:10:00+00:00\n  offset_ms: 0\n  duration_ms: 600000\n  audio: audio/segment-01.opus\n- index: 1\n  wall_start: 2026-09-22T14:20:00+00:00\n  wall_end: 2026-09-22T14:30:00+00:00\n  offset_ms: 1200000\n  duration_ms: 600000\n  audio: audio/segment-02.opus\n---\n";
        let doc = SessionDoc::parse(text).unwrap();
        let transcript = Transcript {
            has_speakers: false,
            lines: vec![
                TranscriptLine::Turn {
                    position_ms: 1_000,
                    speaker: None,
                    text: "a".into(),
                },
                TranscriptLine::Gap {
                    duration_ms: 600_000,
                },
                TranscriptLine::Turn {
                    position_ms: 1_201_000,
                    speaker: None,
                    text: "b".into(),
                },
            ],
        };
        let lines = doc_lines(&doc, transcript);
        assert!(matches!(
            lines[1],
            DocLine::Gap {
                at_ms: 600_000,
                duration_ms: 600_000
            }
        ));
    }
}
