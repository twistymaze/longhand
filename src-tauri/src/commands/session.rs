//! The session list and each session's state, notes and saved documents,
//! read from the job store and the home folder, which are the truth: a
//! window shown mid-processing rebuilds from these, then follows events
//! (U12, AE4).

use std::path::Path;
use std::sync::Arc;

use pipeline::folder_rename::FolderRename;
use serde::Serialize;
use session_core::folder::{SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Event, SessionRecord, SessionState, Stage};
use specta::Type;
use tauri::{AppHandle, State};

use crate::managers::capture::CaptureManager;
use crate::managers::processing::{ProcessingManager, stage_name};
use crate::managers::settings;

#[derive(Serialize, Clone, Type)]
pub struct SessionEntry {
    pub id: String,
    /// When the first segment began (RFC 3339), or when the session was made.
    pub started: String,
    /// Recorded time, gaps excluded.
    pub duration_ms: u64,
    /// The lifecycle state's name: capturing, interrupted, queued,
    /// processing, blocked, processing_failed, awaiting_destination, saved.
    pub state: String,
    /// The stage started last: the running one, or the one a failed
    /// session stopped at.
    pub stage: Option<String>,
    /// The proposed or typed title, if the session has one (R28).
    pub title: Option<String>,
    /// The number of speakers that processing found (KTD11). `None` before
    /// the join, or when the transcript has no speaker marks.
    pub found_speakers: Option<u32>,
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
    /// Why the summary is missing, when the session will be saved with a stub.
    pub summary_problem: Option<String>,
    /// The state machine allows Resume now (R6a).
    pub resumable: bool,
    pub home_path: Option<String>,
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
    /// Between segments; `at_ms` is where the earlier segment ends, and
    /// `wall_start` (RFC 3339) is when the next segment started (KTD12).
    Gap {
        at_ms: u64,
        duration_ms: u64,
        wall_start: Option<String>,
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
    /// The text of `transcript.md`, which the Transcript tab copies (R17).
    pub transcript_markdown: String,
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
        title: record.title.as_ref().map(|t| t.text.clone()),
        found_speakers: record.found_speakers,
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
    let (record, segments, saving, audio_intact) = {
        let store = shared.lock().unwrap_or_else(|e| e.into_inner());
        let record = store
            .session(&session_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("no note {session_id}"))?;
        let segments = store.segments(&session_id).map_err(|e| e.to_string())?;
        // A completed session has finished saving; only one still saving
        // refuses a resume.
        let saving = record.state != SessionState::Saved
            && pipeline::save::begun(&store, &session_id).map_err(e)?;
        let audio_intact = pipeline::save::home_intact(&store, &session_id).map_err(e)?;
        (record, segments, saving, audio_intact)
    };
    let snapshot = pipeline::progress::snapshot(&shared, &session_id).map_err(e)?;

    let resumable = !saving
        && record
            .state
            .transition(&Event::UserResume {
                space_ok: true,
                audio_intact,
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
        pipeline::save::SUMMARY_FAILED,
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
    let (audio_kept, transcript_kept) = match &record.home_path {
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
        home_path: record.home_path.clone(),
        endpoint,
        audio_kept,
        transcript_kept,
    })
}

/// A session's notes document, editable whatever its state (R20).
#[tauri::command]
#[specta::specta]
pub fn session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<String, String> {
    capture
        .session_notes(&session_id)
        .map_err(|e| e.to_string())
}

/// Replace a session's notes document. The editor sends the whole
/// document; on a saved session the home folder's `notes.md` follows (R20).
/// The write runs off the main thread, since it can reach a synced folder.
#[tauri::command]
#[specta::specta]
pub async fn set_session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
    markdown: String,
) -> Result<(), String> {
    let capture = Arc::clone(&capture);
    tauri::async_runtime::spawn_blocking(move || {
        capture
            .set_session_notes(&session_id, &markdown)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What a rename did to the session's folder. The title changes in every
/// case.
#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderRenameResult {
    /// The folder already has the name the title gives.
    Unchanged,
    /// The session records, waits, processes or regenerates its summary:
    /// the folder takes the new name when that ends.
    Later,
    /// The folder has the new name at `path`.
    Moved { path: String },
    /// The folder could not be renamed and keeps its old name.
    Kept { message: String },
}

impl From<FolderRename> for FolderRenameResult {
    fn from(rename: FolderRename) -> Self {
        match rename {
            FolderRename::Unchanged => Self::Unchanged,
            FolderRename::Later => Self::Later,
            FolderRename::Moved(path) => Self::Moved {
                path: path.to_string_lossy().into_owned(),
            },
            FolderRename::Kept(message) => Self::Kept { message },
        }
    }
}

/// Rename a session (R9, R35): `title` is stored as the user's title, and
/// an empty title clears it. The folder of a saved, idle session takes
/// the new name now; any other session's folder takes it later. The rename
/// runs off the main thread, since it can reach a synced folder.
#[tauri::command]
#[specta::specta]
pub async fn rename_session(
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    title: String,
) -> Result<FolderRenameResult, String> {
    let processing = Arc::clone(&processing);
    tauri::async_runtime::spawn_blocking(move || {
        processing
            .rename_session(&session_id, &title)
            .map(FolderRenameResult::from)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The saved transcript and summary, read from the session's folder.
#[tauri::command]
#[specta::specta]
pub fn session_document(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<SessionDocument, String> {
    let home = {
        let store = capture.shared_store();
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .session(&session_id)
            .map_err(|e| e.to_string())?
            .and_then(|r| r.home_path)
            .ok_or_else(|| "this note has not finished processing yet".to_string())?
    };
    let folder = Path::new(&home);
    let read = |name: &str| {
        std::fs::read_to_string(folder.join(name))
            .map_err(|e| format!("could not read {name} in {home}: {e}"))
    };
    let doc = SessionDoc::parse(&read(pipeline::save::SESSION_FILE)?).map_err(|e| e.to_string())?;
    let transcript_markdown = read(pipeline::join::TRANSCRIPT_FILE)?;
    let transcript = Transcript::parse(&transcript_markdown).map_err(|e| e.to_string())?;
    Ok(SessionDocument {
        has_speakers: transcript.has_speakers,
        lines: doc_lines(&doc, transcript),
        transcript_markdown,
        summary: read(pipeline::summarize::SUMMARY_FILE)
            .ok()
            .and_then(|text| Summary::parse(&text).ok())
            .map(summary_view),
    })
}

/// Transcript lines, each gap placed at the end of the segment before it
/// and given the wall-clock start of the segment after it.
fn doc_lines(doc: &SessionDoc, transcript: Transcript) -> Vec<DocLine> {
    let mut ordered: Vec<_> = doc.segments.iter().collect();
    ordered.sort_by_key(|s| s.offset_ms);
    let mut gaps = ordered.windows(2).map(|pair| {
        (
            pair[0].offset_ms + pair[0].duration_ms,
            pair[1].wall_start.to_rfc3339(),
        )
    });
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
            TranscriptLine::Gap { duration_ms } => {
                let (at_ms, wall_start) =
                    gaps.next().map_or((0, None), |(at, wall)| (at, Some(wall)));
                DocLine::Gap {
                    at_ms,
                    duration_ms,
                    wall_start,
                }
            }
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

/// Why a delete did not complete (R8, R33). Nothing after the failed step
/// changed.
#[derive(Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeleteSessionError {
    /// The session records, is processed, or has its summary regenerated.
    /// Nothing changed.
    Busy { message: String },
    /// `path` cannot go to the Trash (R33). It and the session remain, and
    /// `delete_session_permanently` can delete them. When `path` is the home
    /// folder, which goes first, nothing changed.
    CannotMoveToTrash { path: String, reason: String },
    /// A permanent delete or the store failed.
    Failed { message: String },
}

fn busy_message(state: &str) -> String {
    match state {
        "capturing" => "This note is recording. Stop the recording before you delete it.",
        "interrupted" => {
            "This note's recording was interrupted. Stop it before you delete the note."
        }
        "queued" => {
            "This note is waiting to be processed. You can delete it when processing is done."
        }
        "regenerating" => {
            "This note's summary is being made again. You can delete it when that is done."
        }
        _ => "This note is being processed. You can delete it when processing is done.",
    }
    .to_string()
}

/// How a delete removes a directory.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Removal {
    Trash,
    Permanent,
}

fn delete_error(error: session_core::DeleteError, removal: Removal) -> DeleteSessionError {
    use session_core::DeleteError;
    match error {
        DeleteError::Busy { state } => DeleteSessionError::Busy {
            message: busy_message(state),
        },
        DeleteError::Remove { path, message } if removal == Removal::Trash => {
            DeleteSessionError::CannotMoveToTrash {
                path: path.to_string_lossy().into_owned(),
                reason: message,
            }
        }
        DeleteError::Remove { path, message } => DeleteSessionError::Failed {
            message: format!("Could not delete {}: {message}", path.display()),
        },
        DeleteError::Store(e) => DeleteSessionError::Failed {
            message: e.to_string(),
        },
    }
}

/// Move a directory to the Trash (KTD9). On macOS, NSFileManager does it:
/// the default Finder method runs AppleScript and asks for Automation
/// permission.
fn move_to_trash(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let context = {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context
    };
    #[cfg(not(target_os = "macos"))]
    let context = trash::TrashContext::default();
    context.delete(path).map_err(|e| match e {
        trash::Error::Unknown { description } | trash::Error::Os { description, .. } => description,
        other => other.to_string(),
    })
}

fn remove_permanently(path: &Path) -> Result<(), String> {
    std::fs::remove_dir_all(path).map_err(|e| e.to_string())
}

/// Remove a session on a blocking thread: the Trash call on Windows uses
/// COM, and a folder on a network volume can be slow.
async fn delete(
    capture: &CaptureManager,
    processing: &ProcessingManager,
    session_id: String,
    removal: Removal,
) -> Result<(), DeleteSessionError> {
    let store = capture.shared_store();
    let working = capture.working_root().join(&session_id);
    let claims = processing.claims();
    let remove = match removal {
        Removal::Trash => move_to_trash,
        Removal::Permanent => remove_permanently,
    };
    tauri::async_runtime::spawn_blocking(move || {
        session_core::jobs::delete_session(&store, &session_id, &working, &claims, remove)
            .inspect(|()| log::info!("deleted session {session_id} ({removal:?})"))
    })
    .await
    .map_err(|e| DeleteSessionError::Failed {
        message: e.to_string(),
    })?
    .map_err(|e| delete_error(e, removal))
}

/// Move the session's folder to the Trash and remove the session from
/// Longhand (R8). Refused while it records or is processed.
#[tauri::command]
#[specta::specta]
pub async fn delete_session(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), DeleteSessionError> {
    delete(&capture, &processing, session_id, Removal::Trash).await
}

/// Delete the session's folder permanently, for a folder that cannot go
/// to the Trash (R33).
#[tauri::command]
#[specta::specta]
pub async fn delete_session_permanently(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), DeleteSessionError> {
    delete(&capture, &processing, session_id, Removal::Permanent).await
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use session_core::DeleteError;

    #[test]
    fn a_trash_failure_is_a_typed_error_naming_the_folder() {
        let e = delete_error(
            DeleteError::Remove {
                path: "/d/Budget review".into(),
                message: "no Trash on this volume".into(),
            },
            Removal::Trash,
        );
        assert!(
            matches!(&e, DeleteSessionError::CannotMoveToTrash { path, reason }
                if path == "/d/Budget review" && reason == "no Trash on this volume"),
            "{e:?}"
        );
        let e = delete_error(
            DeleteError::Remove {
                path: "/d/x".into(),
                message: "denied".into(),
            },
            Removal::Permanent,
        );
        assert!(matches!(e, DeleteSessionError::Failed { .. }), "{e:?}");
    }

    #[test]
    fn a_refusal_says_why_in_plain_words() {
        for (state, word) in [
            ("capturing", "recording"),
            ("interrupted", "interrupted"),
            ("queued", "waiting"),
            ("processing", "processed"),
            ("regenerating", "summary"),
        ] {
            match delete_error(DeleteError::Busy { state }, Removal::Trash) {
                DeleteSessionError::Busy { message } => {
                    assert!(message.contains(word), "{state}: {message}")
                }
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn a_permanent_delete_removes_the_folder_and_the_rows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let working = dir.path().join("working");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        std::fs::write(home.join("audio").join("segment-01.opus"), "x").unwrap();
        std::fs::create_dir_all(&working).unwrap();
        let mut store = session_core::Store::open_in_memory().unwrap();
        let created = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        store.create_session("s", created, None).unwrap();
        store
            .apply(
                "s",
                &session_core::Event::UserStop {
                    at: created,
                    retained_audio: false,
                },
            )
            .unwrap();
        store.set_home_path("s", home.to_str().unwrap()).unwrap();
        let store = std::sync::Mutex::new(store);
        let claims = session_core::jobs::Claims::default();
        session_core::jobs::delete_session(&store, "s", &working, &claims, remove_permanently)
            .unwrap();
        assert!(!home.exists() && !working.exists());
        assert!(store.lock().unwrap().session("s").unwrap().is_none());
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
            &lines[1],
            DocLine::Gap {
                at_ms: 600_000,
                duration_ms: 600_000,
                wall_start: Some(w),
            } if w == "2026-09-22T14:20:00+00:00"
        ));
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;
    use session_core::{Title, TitleSource};

    #[test]
    fn an_entry_carries_the_title_and_the_found_speaker_count() {
        let created = chrono::DateTime::parse_from_rfc3339("2026-09-22T14:00:00+00:00").unwrap();
        let record = SessionRecord {
            id: "s".into(),
            created_at: created,
            state: SessionState::Saved,
            queue_seq: None,
            speaker_count: None,
            asr_model: None,
            home_path: None,
            title: Some(Title {
                text: "Budget review".into(),
                source: TitleSource::Proposed,
            }),
            found_speakers: Some(3),
            live_transcript: None,
        };
        let e = entry(&record, &[], None);
        assert_eq!(e.title.as_deref(), Some("Budget review"));
        assert_eq!(e.found_speakers, Some(3));
        let e = entry(
            &SessionRecord {
                title: None,
                found_speakers: None,
                ..record
            },
            &[],
            None,
        );
        assert_eq!(e.title, None);
        assert_eq!(e.found_speakers, None);
    }
}
