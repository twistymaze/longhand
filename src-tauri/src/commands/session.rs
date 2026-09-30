//! The session list and each session's state, notes and saved documents,
//! read from the job store and the home folder, which are the truth: a
//! window shown mid-processing rebuilds from these, then follows events
//! (U12, AE4).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pipeline::folder_rename::FolderRename;
use serde::Serialize;
use session_core::folder::{SessionDoc, Summary, SummaryMeta, Transcript, TranscriptLine};
use session_core::{Event, SessionRecord, SessionState, Stage};
use specta::Type;
use tauri::{AppHandle, State};

use library::scan::Waiting;
use library::status::LibraryStatus;

use crate::managers::capture::CaptureManager;
use crate::managers::live::{LiveManager, LiveTranscript};
use crate::managers::processing::{ProcessingManager, SpeakerProgressView, stage_name};
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
    /// The note's storage folder is unavailable, or its `session.md` cannot
    /// be read now (R16). Opening it and its actions wait.
    pub storage_unavailable: bool,
}

/// A part of a note that the sync has not delivered yet (R6).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum WaitingPart {
    Transcript,
    Summary,
    Notes,
    Audio,
}

/// The audio of a note, from its folder (KTD9). The note page shows no
/// mark for it. It gives the reason of an unavailable action only (R14).
#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug, Type)]
#[serde(rename_all = "snake_case")]
pub enum AudioView {
    /// Every segment has its audio file.
    Present,
    /// At least one segment has a removal marker. The audio does not come
    /// back.
    Removed,
    /// At least one segment has no audio file yet: the sync can still
    /// deliver it.
    Waiting,
}

impl From<pipeline::save::AudioState> for AudioView {
    fn from(state: pipeline::save::AudioState) -> Self {
        use pipeline::save::AudioState;
        match state {
            AudioState::Present => Self::Present,
            AudioState::Removed => Self::Removed,
            AudioState::Waiting => Self::Waiting,
        }
    }
}

/// The parts that `waiting` names, in its order, each one time.
fn waiting_parts(waiting: &[Waiting]) -> Vec<WaitingPart> {
    let mut parts = Vec::new();
    for w in waiting {
        let part = match w {
            Waiting::Transcript => WaitingPart::Transcript,
            Waiting::Summary => WaitingPart::Summary,
            Waiting::Notes => WaitingPart::Notes,
            Waiting::Audio(_) => WaitingPart::Audio,
        };
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    parts
}

/// The names of the other versions of a note: conflict copies and other
/// folders with the same note id (R15).
fn file_names(paths: &[PathBuf]) -> Vec<String> {
    paths
        .iter()
        .map(|p| {
            p.file_name()
                .map_or_else(|| p.to_string_lossy(), |n| n.to_string_lossy())
                .into_owned()
        })
        .collect()
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
    /// The speaker stage's last saved progress while that stage runs.
    pub speakers: Option<SpeakerProgressView>,
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
    /// The audio of the note (KTD9). Transcribe Again, Resume and Retry
    /// read it for their reason.
    pub audio: AudioView,
    /// The recognition model that the note recorded when it started
    /// (KTD2). `None` for a note that started before the update.
    pub asr_model: Option<String>,
    /// The note records a live transcript (R8, R18). The Transcript tab
    /// reads this, not the model selected now (R9).
    pub live_transcript: bool,
    /// A Transcribe Again run waits or runs (KTD14). It has no live text.
    pub transcribe_again: bool,
    /// Why the last Transcribe Again run failed. The note kept its
    /// previous transcript and summary (R19).
    pub transcribe_again_error: Option<String>,
    /// Why the last Transcribe Again run kept the previous summary with
    /// the new transcript (R19).
    pub summary_not_updated: Option<String>,
    /// The parts that the sync has not delivered yet (R6). Notes edits,
    /// Regenerate Summary, Transcribe Again and Resume wait for them.
    pub waiting: Vec<WaitingPart>,
    /// The file or folder names of other versions of the note, which the
    /// user resolves in Finder (R15).
    pub conflict_files: Vec<String>,
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
    /// The name of the prompt that made the summary. A summary made before
    /// prompts had names, and a stub, have none.
    pub prompt: Option<String>,
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
    storage_unavailable: bool,
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
        storage_unavailable,
    }
}

fn storage_unavailable(status: &LibraryStatus, record: &SessionRecord) -> bool {
    status.storage_unavailable(&record.id, record.home_path.as_deref().map(Path::new))
}

/// Every session but the discarded ones: the switcher's list, newest first.
/// The storage flag comes from the latest library scan: the list reads no
/// folder.
#[tauri::command]
#[specta::specta]
pub fn list_sessions(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
) -> Result<Vec<SessionEntry>, String> {
    let status = processing.library_status();
    let store = capture.shared_store();
    let store = store.lock().unwrap_or_else(|e| e.into_inner());
    let records = store.sessions().map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for record in records.iter().rev() {
        if record.state != SessionState::Discarded {
            let segments = store.segments(&record.id).map_err(|e| e.to_string())?;
            let stage = store.current_stage(&record.id).map_err(|e| e.to_string())?;
            let unavailable = storage_unavailable(status, record);
            out.push(entry(record, &segments, stage, unavailable));
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
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<SessionDetail, String> {
    let shared = capture.shared_store();
    let e = |e: pipeline::StageError| e.to_string();
    let (record, segments, saving, audio, again) = {
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
        let audio = pipeline::save::audio_state(&store, &session_id).map_err(e)?;
        let again = store
            .transcribe_again_marks(&session_id)
            .map_err(|e| e.to_string())?;
        (record, segments, saving, audio, again)
    };
    let audio_intact = audio == pipeline::save::AudioState::Present;
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
    let endpoint = s.endpoint_configured().then(|| {
        let base_url = s.effective_endpoint_base_url();
        EndpointView {
            host: secrets::endpoint_key(base_url).unwrap_or_else(|_| base_url.to_string()),
            model: s.endpoint_model.clone(),
            encrypted: base_url.starts_with("https://"),
        }
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

    let status = processing.library_status();
    let note = status.note(&session_id);
    Ok(SessionDetail {
        entry: entry(
            &record,
            &segments,
            snapshot.current,
            storage_unavailable(status, &record),
        ),
        done: snapshot.done.into_iter().map(stage_name).collect(),
        current: snapshot.current.map(stage_name),
        windows_done: snapshot.windows.map(|(d, _)| d as u32),
        windows_total: snapshot.windows.map(|(_, t)| t as u32),
        speakers: snapshot.speakers.map(SpeakerProgressView::from),
        failed_stage: failed_stage.map(stage_name),
        message,
        summary_problem,
        resumable,
        home_path: record.home_path.clone(),
        endpoint,
        audio_kept,
        transcript_kept,
        audio: audio.into(),
        asr_model: record.asr_model.clone(),
        live_transcript: record.live_transcript == Some(true),
        transcribe_again: again.model.is_some(),
        transcribe_again_error: again.error,
        summary_not_updated: again.summary_error,
        waiting: waiting_parts(&note.waiting),
        conflict_files: file_names(&note.other_versions),
    })
}

/// A session's notes document and the fingerprint of the `notes.md` that
/// holds it. The editor sends the fingerprint back with each save
/// (KTD10).
#[derive(Serialize, Type)]
pub struct LoadedNotes {
    pub markdown: String,
    pub fingerprint: String,
}

/// A session's notes document, editable whatever its state (R20), with the
/// fingerprint that [`save_session_notes`] checks.
#[tauri::command]
#[specta::specta]
pub fn load_session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    session_id: String,
) -> Result<LoadedNotes, String> {
    let markdown = capture
        .session_notes(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(LoadedNotes {
        fingerprint: pipeline::save::notes_fingerprint(&markdown),
        markdown,
    })
}

/// What a notes save did.
#[derive(Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotesSaveResult {
    /// `fingerprint` is the fingerprint of the saved document, which the
    /// editor sends with its next save.
    Saved { fingerprint: String },
    /// Another computer changed `notes.md` after the editor loaded it. The
    /// file keeps that text, and the edit is in the draft file `draft`
    /// beside it. The editor loads the notes again.
    Conflict { draft: String },
}

/// Replace a session's notes document. The editor sends the whole
/// document; on a saved session the home folder's `notes.md` follows (R20).
/// The write runs off the main thread, since it can reach a synced folder.
/// `loaded_fingerprint` is the fingerprint that [`load_session_notes`] gave
/// for the document that the editor changed; `None` skips the check.
#[tauri::command]
#[specta::specta]
pub async fn save_session_notes(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
    markdown: String,
    loaded_fingerprint: Option<String>,
) -> Result<NotesSaveResult, String> {
    let capture = Arc::clone(&capture);
    let claims = processing.claims();
    tauri::async_runtime::spawn_blocking(move || {
        let saved = capture
            .save_session_notes(
                &session_id,
                &markdown,
                loaded_fingerprint.as_deref(),
                &claims,
            )
            .map_err(|e| e.to_string())?;
        Ok(match saved {
            pipeline::save::NotesSave::Saved { .. } => NotesSaveResult::Saved {
                fingerprint: pipeline::save::notes_fingerprint(&markdown),
            },
            pipeline::save::NotesSave::Conflict { draft } => NotesSaveResult::Conflict {
                draft: draft.to_string_lossy().into_owned(),
            },
        })
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

/// The live transcript of a note so far: its saved parts and the text of
/// the open part (KTD10). The Transcript tab reads it when it opens during
/// recording, and then follows the live events.
#[tauri::command]
#[specta::specta]
pub fn live_transcript(
    capture: State<'_, Arc<CaptureManager>>,
    live: State<'_, Arc<LiveManager>>,
    session_id: String,
) -> Result<LiveTranscript, String> {
    live.transcript(&capture.shared_store(), &session_id)
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
    let previous: Option<SessionDoc> = {
        let store = capture.shared_store();
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .result_state(&session_id, "verified-manifest")
            .map_err(|e| e.to_string())?
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| e.to_string())?
    };
    let result =
        session_core::generation::resolve(folder, previous.as_ref()).map_err(|e| e.to_string())?;
    let doc = result.doc;
    let transcript_markdown = String::from_utf8(result.transcript).map_err(|e| e.to_string())?;
    let transcript = Transcript::parse(&transcript_markdown).map_err(|e| e.to_string())?;
    Ok(SessionDocument {
        has_speakers: transcript.has_speakers,
        lines: doc_lines(&doc, transcript),
        transcript_markdown,
        summary: String::from_utf8(result.summary)
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
            prompt_name,
        } => SummaryView {
            body,
            stub: false,
            host: Some(endpoint_host),
            model: Some(model),
            prompt: prompt_name,
        },
        SummaryMeta::Stub { .. } => SummaryView {
            body,
            stub: true,
            host: None,
            model: None,
            prompt: None,
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
        "moving" => {
            "This note is being moved to the new storage folder. You can delete it when that is done."
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

/// Why Remove Audio did not complete (R13).
#[derive(Debug, Serialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RemoveAudioFailure {
    /// The removal did not start. Nothing changed.
    Refused { message: String },
    /// The removal started and did not complete for each segment. A
    /// segment that has its marker counts as removed, and the next pass
    /// deletes its file (KTD2).
    Failed { message: String },
}

/// The reason that the guard gives while a Move runs or is paused.
const MOVE_OPEN_REASON: &str = "move open";

/// The reason that the guard gives while another computer uses the note.
const IN_USE_REASON: &str = "in use";

/// The text of a refusal of Remove Audio. `reason` is a state name, a
/// claim name or a guard reason.
fn removal_refusal(reason: &str) -> String {
    match reason {
        "capturing" => "This note is recording. Stop the recording before you remove its audio.",
        "interrupted" => {
            "This note’s recording was interrupted. Stop it before you remove its audio."
        }
        "queued" | "processing" | "blocked" => {
            "This note is being processed. You can remove its audio when processing is done."
        }
        "regenerating" => {
            "This note’s summary is being made again. You can remove its audio when that is done."
        }
        "moving" | MOVE_OPEN_REASON => {
            "Longhand is moving your notes. You can remove the audio when the move is done."
        }
        "deleting" => "This note is being deleted.",
        IN_USE_REASON => "This note is in use on another computer. Try again later.",
        _ => "You can remove the audio of this note only after it is saved.",
    }
    .to_string()
}

/// The text of a note whose storage folder is unavailable (R16).
const FOLDER_UNAVAILABLE: &str = "The storage folder for this note is unavailable.";

/// The checks of Remove Audio that the removal step does after its claim
/// and its check that the note is saved. The folder check runs with no
/// store lock, and the store check runs under the store lock.
fn removal_guard() -> pipeline::remove_audio::Guard<
    pipeline::remove_audio::FolderCheck,
    pipeline::remove_audio::StoreCheck,
> {
    pipeline::remove_audio::Guard {
        folder: |record| removal_folder_checks(record, chrono::Local::now().fixed_offset()),
        store: |store, _| removal_store_checks(store),
    }
}

/// The store checks of Remove Audio. The removal pass uses them too (KTD5).
/// The caller holds the store lock.
pub(crate) fn removal_store_checks(store: &session_core::Store) -> Result<(), &'static str> {
    // A Move changes the home folders of the notes (KTD11). When the
    // journal cannot be read, the guard also refuses.
    if !matches!(store.move_journal(), Ok(None)) {
        return Err(MOVE_OPEN_REASON);
    }
    Ok(())
}

/// The folder checks of Remove Audio at the time `now`. The removal pass
/// uses them too (KTD5). They read the home folder, so the caller holds no
/// store lock.
pub(crate) fn removal_folder_checks(
    record: &SessionRecord,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<(), &'static str> {
    // A fresh in-use marker shows that a computer records into or processes
    // the note (KTD4). A folder that cannot be read also refuses. A missing
    // folder is not in use: the step then refuses it as unavailable.
    if let Some(home) = record.home_path.as_deref() {
        match pipeline::in_use::in_use(std::path::Path::new(home), now) {
            Ok(false) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(true) | Err(_) => return Err(IN_USE_REASON),
        }
    }
    Ok(())
}

/// Remove the audio of the note `session_id` with the checks of Remove
/// Audio. The setting of audio removal has no effect here (R13).
fn run_removal(
    store: &pipeline::SharedStore,
    claims: &session_core::jobs::Claims,
    session_id: &str,
) -> Result<(), RemoveAudioFailure> {
    use pipeline::remove_audio::RemoveAudioError;
    let removal = pipeline::remove_audio::remove_audio(store, claims, session_id, removal_guard())
        .map_err(|e| match e {
            RemoveAudioError::Busy(reason) => RemoveAudioFailure::Refused {
                message: removal_refusal(reason),
            },
            RemoveAudioError::NoFolder => RemoveAudioFailure::Refused {
                message: FOLDER_UNAVAILABLE.to_string(),
            },
            RemoveAudioError::Failed(message) => RemoveAudioFailure::Failed {
                message: format!("Could not remove the audio of this note: {message}"),
            },
        })?;
    if removal.complete() {
        log::info!("removed the audio of session {session_id}");
        return Ok(());
    }
    for segment in &removal.segments {
        log::warn!(
            "the audio {} of session {session_id} was not removed: {:?}",
            segment.audio_file,
            segment.outcome
        );
    }
    Err(RemoveAudioFailure::Failed {
        message: "Some of the audio of this note could not be removed.".to_string(),
    })
}

/// Remove the audio of a saved note now, whether the setting of audio
/// removal is on or off (R13). The transcript, summary and notes stay.
/// Refused while the note is busy, while a Move runs or is paused, or while
/// its storage folder is unavailable.
#[tauri::command]
#[specta::specta]
pub async fn remove_audio(
    capture: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), RemoveAudioFailure> {
    let store = capture.shared_store();
    let home = {
        let store = store.lock().unwrap_or_else(|e| e.into_inner());
        store
            .session(&session_id)
            .map_err(|e| RemoveAudioFailure::Failed {
                message: e.to_string(),
            })?
            .and_then(|r| r.home_path)
    };
    if processing
        .library_status()
        .storage_unavailable(&session_id, home.as_deref().map(Path::new))
    {
        return Err(RemoveAudioFailure::Refused {
            message: FOLDER_UNAVAILABLE.to_string(),
        });
    }
    let claims = processing.claims();
    // A folder on a network volume can be slow.
    tauri::async_runtime::spawn_blocking(move || run_removal(&store, &claims, &session_id))
        .await
        .map_err(|e| RemoveAudioFailure::Failed {
            message: e.to_string(),
        })?
}

#[cfg(test)]
mod remove_audio_tests {
    use super::*;
    use session_core::folder::{SessionDoc, fingerprint};
    use session_core::jobs::{Adoption, Claims};

    const ID: &str = "20260901-0900-abcd";

    struct Note {
        _root: tempfile::TempDir,
        home: PathBuf,
        store: pipeline::SharedStore,
    }

    /// A saved note folder with the audio file of its one segment, and its
    /// row in the store.
    fn saved_note() -> Note {
        let root = tempfile::tempdir().unwrap();
        let home = root.path().canonicalize().unwrap().join("Planning");
        std::fs::create_dir_all(home.join("audio")).unwrap();
        let end = chrono::DateTime::parse_from_rfc3339("2026-09-01T09:01:00-05:00").unwrap();
        let segments = [session_core::Segment {
            index: 0,
            offset_ms: 0,
            duration_ms: 60_000,
            wall_start: end - chrono::TimeDelta::minutes(1),
            wall_end: end,
            audio_file: "audio/segment-01.opus".into(),
            channels: 1,
        }];
        let transcript = "# Transcript\n\n- [00:00:01] Speaker 1: Hello.\n";
        let summary = "---\nstatus: stub\nfailed_stage: summarize\n---\n\nNo summary.\n";
        let mut doc = SessionDoc::new(ID, None, &segments).unwrap();
        doc.transcript_fingerprint = Some(fingerprint(transcript.as_bytes()));
        doc.summary_fingerprint = Some(fingerprint(summary.as_bytes()));
        std::fs::write(home.join("session.md"), doc.render()).unwrap();
        std::fs::write(home.join("transcript.md"), transcript).unwrap();
        std::fs::write(home.join("summary.md"), summary).unwrap();
        std::fs::write(home.join("notes.md"), "My notes\n").unwrap();
        std::fs::write(home.join("audio/segment-01.opus"), b"opus").unwrap();
        let mut store = session_core::Store::open_in_memory().unwrap();
        let adoption = Adoption::from_doc(
            &doc,
            "My notes\n".into(),
            home.to_string_lossy().into_owned(),
            Some(fingerprint(transcript.as_bytes())),
        );
        assert!(store.adopt(&adoption).unwrap());
        Note {
            _root: root,
            home,
            store: Arc::new(std::sync::Mutex::new(store)),
        }
    }

    fn refused(result: Result<(), RemoveAudioFailure>) -> String {
        match result {
            Err(RemoveAudioFailure::Refused { message }) => message,
            other => panic!("{other:?}"),
        }
    }

    // Covers AE8: the command reads no setting, so it removes the audio
    // whether the setting is on or off.
    #[test]
    fn remove_audio_removes_the_audio_and_keeps_the_documents() {
        let note = saved_note();
        run_removal(&note.store, &Claims::default(), ID).unwrap();
        assert!(!note.home.join("audio/segment-01.opus").exists());
        for name in ["session.md", "transcript.md", "summary.md", "notes.md"] {
            assert!(note.home.join(name).is_file(), "{name}");
        }
        let store = note.store.lock().unwrap();
        assert_eq!(
            pipeline::save::audio_state(&store, ID).unwrap(),
            pipeline::save::AudioState::Removed
        );
    }

    #[test]
    fn an_open_move_refuses_with_a_reason_and_keeps_the_audio() {
        let note = saved_note();
        note.store
            .lock()
            .unwrap()
            .open_move(Path::new("/old"), Path::new("/new"))
            .unwrap();
        let message = refused(run_removal(&note.store, &Claims::default(), ID));
        assert!(message.contains("moving your notes"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_fresh_in_use_marker_of_another_computer_refuses_and_keeps_the_audio() {
        let note = saved_note();
        let now = chrono::Local::now().fixed_offset();
        assert!(pipeline::in_use::write_marker(&note.home, "other-computer", now).unwrap());
        let message = refused(run_removal(&note.store, &Claims::default(), ID));
        assert!(message.contains("in use on another computer"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_busy_note_refuses_with_a_reason_and_keeps_the_audio() {
        let note = saved_note();
        let claims = Claims::default();
        let _claim = claims.regenerate(ID).unwrap();
        let message = refused(run_removal(&note.store, &claims, ID));
        assert!(message.contains("summary"), "{message}");
        assert!(note.home.join("audio/segment-01.opus").is_file());
    }

    #[test]
    fn a_missing_folder_refuses_as_an_unavailable_storage_folder() {
        let note = saved_note();
        std::fs::remove_dir_all(&note.home).unwrap();
        let message = refused(run_removal(&note.store, &Claims::default(), ID));
        assert_eq!(message, FOLDER_UNAVAILABLE);
        assert!(!note.home.exists(), "the step makes no folder");
    }

    #[test]
    fn each_refusal_says_why_in_plain_words() {
        for (reason, word) in [
            ("capturing", "recording"),
            ("interrupted", "interrupted"),
            ("queued", "processed"),
            ("processing", "processed"),
            ("regenerating", "summary"),
            ("moving", "moving"),
            (MOVE_OPEN_REASON, "moving"),
            ("deleting", "deleted"),
            (IN_USE_REASON, "another computer"),
            ("processing_failed", "saved"),
            ("awaiting_destination", "saved"),
        ] {
            let message = removal_refusal(reason);
            assert!(message.contains(word), "{reason}: {message}");
        }
    }
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
            ("moving", "moved"),
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

    #[test]
    fn a_summary_view_carries_the_recorded_prompt_name() {
        let complete = |prompt_name: Option<&str>| Summary {
            meta: SummaryMeta::Complete {
                endpoint_host: "api.example.com".into(),
                model: "m".into(),
                prompt_name: prompt_name.map(Into::into),
            },
            body: "We agreed.".into(),
        };
        let named = summary_view(complete(Some("Team sync")));
        assert_eq!(named.prompt.as_deref(), Some("Team sync"));
        assert_eq!(named.host.as_deref(), Some("api.example.com"));
        assert!(summary_view(complete(None)).prompt.is_none());
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
            recognition_snapshot: None,
            home_path: None,
            title: Some(Title {
                text: "Budget review".into(),
                source: TitleSource::Proposed,
            }),
            found_speakers: Some(3),
            live_transcript: None,
            transcribe_again_model: None,
        };
        let e = entry(&record, &[], None, false);
        assert!(!e.storage_unavailable);
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
            true,
        );
        assert!(e.storage_unavailable);
        assert_eq!(e.title, None);
        assert_eq!(e.found_speakers, None);
    }
}
