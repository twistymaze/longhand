//! Thin IPC over the capture manager. The shortcut drives the same manager;
//! these let the surface start, stop, resume and record into a note, and
//! carry the scratchpad.

use std::sync::Arc;

use serde::Deserialize;
use shared_settings::note_types::NoteTypeError;
use specta::Type;
use tauri::{AppHandle, Manager, State};

use crate::managers::capture::{CaptureError, CaptureManager, RecordingStatus, StopOutcome};
use crate::managers::destination;
use crate::managers::processing::ProcessingManager;
use crate::managers::settings::{self, MicrophoneChoice, SpeakerCountSetting};
use crate::managers::shared::{NoteTypeList, SharedSettingsManager};
use crate::shortcut::{self, ShortcutKind, ShortcutRegistration, ShortcutStatuses};

use super::blocking;

/// The note type of a new note (R23, R24 of the note types plan).
#[derive(Deserialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NewNoteType {
    /// The default type from the shared settings (R7).
    Default,
    /// The built-in type General: the home folder is directly in the
    /// storage folder.
    General,
    /// A type of the effective type list.
    Named { name: String },
}

/// The type of a new note from `choice` and the effective type list
/// `types`, with the spelling of the list. `None` is General. No choice is
/// the default type. A named type must be a valid type of the list (KTD8,
/// KTD9).
pub fn chosen_type(
    choice: Option<&NewNoteType>,
    types: &NoteTypeList,
) -> Result<Option<String>, String> {
    match choice {
        None | Some(NewNoteType::Default) => Ok(types.default_type.clone()),
        Some(NewNoteType::General) => Ok(None),
        Some(NewNoteType::Named { name }) => named_type(types, name).map(Some),
    }
}

/// The valid type of the list `types` that has the name `name`, with the
/// spelling of the list (KTD8, KTD9). General has no type folder, so its
/// name is refused: the caller uses `None` for General.
pub fn named_type(types: &NoteTypeList, name: &str) -> Result<String, String> {
    types
        .types
        .iter()
        .find(|t| t.valid && !t.general && session_save::naming::same_type_name(&t.name, name))
        .map(|t| t.name.clone())
        .ok_or_else(|| NoteTypeError::NoSuchType.to_string())
}

/// The type of a new note from `choice`, with the type list of the
/// storage folder now. The shortcut and the New Note button give no
/// choice, so they use the default type (R23).
pub fn type_of_new_note(
    app: &AppHandle,
    choice: Option<&NewNoteType>,
) -> Result<Option<String>, String> {
    let Some(shared) = app.try_state::<Arc<SharedSettingsManager>>() else {
        return chosen_type(choice, &NoteTypeList::default());
    };
    let folders = crate::commands::settings::storage_folders(app)?;
    chosen_type(choice, &shared.note_types(&folders))
}

/// Start a new note (R7, KTD2). With `record`, the note records at once;
/// without it, the note opens with no recording, and a recording that runs
/// continues. Refused while the storage folder is not available, for both
/// choices. `config` applies to a recording start only. `note_type` is the
/// type of the note; no choice is the default type (R23, R24). A named type
/// that is not a valid type of the list is refused. Returns the id of the
/// new note.
#[tauri::command]
#[specta::specta]
pub async fn start_recording(
    app: AppHandle,
    record: bool,
    config: Option<models::recognition::RecognitionConfig>,
    note_type: Option<NewNoteType>,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<String, String> {
    let manager = Arc::clone(&manager);
    let settings = settings::get_settings(&app);
    let count = shortcut::speaker_count(settings.default_speaker_count);
    let folder = settings.storage_folder;
    blocking(move || {
        // A folder on a network volume can be slow to answer.
        if let Some(reason) = destination::note_start_refusal(&destination::status(&folder)) {
            return Err(reason.to_string());
        }
        let note_type = type_of_new_note(&app, note_type.as_ref())?;
        let started = if record {
            manager.start_session_with_config(&app, count, config, note_type.as_deref())
        } else {
            manager.open_note(&app, note_type.as_deref())
        };
        started.map_err(open_error)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn stop_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
) -> Result<StopOutcome, String> {
    let manager = Arc::clone(&manager);
    let outcome = blocking(move || {
        manager
            .stop_session(&app)
            .inspect(|outcome| log::info!("stopped: {outcome:?}"))
            .map_err(|e| e.to_string())
    })
    .await?;
    processing.wake();
    Ok(outcome)
}

/// Pause the note that records (R1, KTD8). A note that is already paused,
/// or no note that records, does not change.
#[tauri::command]
#[specta::specta]
pub async fn pause_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<(), String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.pause_session(&app).map_err(|e| e.to_string())).await
}

/// Continue the paused note (R1, KTD8). A note that records, or no paused
/// note, does not change. When the microphone cannot open, the note is
/// interrupted (R5).
#[tauri::command]
#[specta::specta]
pub async fn continue_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<(), String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.continue_session(&app).map_err(open_error)).await
}

/// Record into a note with no audio (R11, KTD2), from the note page or the
/// recording shortcut. Refused with the reason that the note page shows
/// (R12, R21, R23): the note records or is processed on another computer,
/// another computer did not finish its recording, its storage folder is
/// unavailable, or it waits for a sync. A note with audio uses
/// [`resume_recording`].
#[tauri::command]
#[specta::specta]
pub async fn record_note(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<(), String> {
    let manager = Arc::clone(&manager);
    let processing = Arc::clone(&processing);
    blocking(move || {
        crate::commands::session::check_record(
            &manager.shared_store(),
            processing.library_status(),
            processing.own_runs(),
            &session_id,
            chrono::Local::now().fixed_offset(),
        )?;
        manager
            .record_session(&app, &session_id)
            .map_err(open_error)
    })
    .await
}

/// Record more into a note with audio, or resume an interrupted note.
/// Refused with the reason that the note page shows (R12, R21, R23) while
/// another computer uses the note or did not finish its recording.
#[tauri::command]
#[specta::specta]
pub async fn resume_recording(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    processing: State<'_, Arc<ProcessingManager>>,
    session_id: String,
) -> Result<String, String> {
    // Resume replaces the result of the note (KTD11).
    crate::managers::processing::refuse_while_syncing(processing.library_status(), &session_id)?;
    let manager = Arc::clone(&manager);
    let checks = Arc::clone(&processing);
    let id = session_id.clone();
    let state = blocking(move || {
        // The same refusals as Record for another computer's work (R21).
        crate::commands::session::check_resume(
            &manager.shared_store(),
            checks.library_status(),
            checks.own_runs(),
            &id,
            chrono::Local::now().fixed_offset(),
        )?;
        manager
            .resume_session(&app, &id)
            .map(|state| state.name().to_string())
            .map_err(open_error)
    })
    .await?;
    // Out of the queue (R20c); a stage running on it stops at its next checkpoint.
    if state == "capturing" {
        processing.cancel(&session_id);
    }
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn recording_status(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
) -> Result<RecordingStatus, String> {
    let manager = Arc::clone(&manager);
    blocking(move || manager.status(&app).map_err(|e| e.to_string())).await
}

/// Change the microphone of one note from the recording page (R2). With no
/// `session_id`, the next note uses it. `None` is "System default". Settings
/// does not change. Returns the status after the change.
#[tauri::command]
#[specta::specta]
pub async fn set_note_microphone(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    session_id: Option<String>,
    microphone: Option<MicrophoneChoice>,
) -> Result<RecordingStatus, String> {
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_note_microphone(&app, session_id.as_deref(), microphone)
            .and_then(|()| manager.status(&app))
            .map_err(|e| e.to_string())
    })
    .await
}

/// The refusal while Computer audio is not available.
pub const SYSTEM_AUDIO_UNAVAILABLE: &str = "Computer audio is not available yet.";

/// Turn system audio on or off for one note from the recording page (R8).
/// With no `session_id`, the next note starts with it. Settings does not
/// change. Returns the status after the change.
#[tauri::command]
#[specta::specta]
pub async fn set_note_system_audio(
    app: AppHandle,
    manager: State<'_, Arc<CaptureManager>>,
    session_id: Option<String>,
    enabled: bool,
) -> Result<RecordingStatus, String> {
    if enabled && !crate::managers::capture::SYSTEM_AUDIO_AVAILABLE {
        return Err(SYSTEM_AUDIO_UNAVAILABLE.to_string());
    }
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_note_system_audio(&app, session_id.as_deref(), enabled)
            .and_then(|()| manager.status(&app))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn set_session_speaker_count(
    manager: State<'_, Arc<CaptureManager>>,
    count: Option<SpeakerCountSetting>,
) -> Result<(), String> {
    if let Some(count) = &count {
        count.validate()?;
    }
    let manager = Arc::clone(&manager);
    blocking(move || {
        manager
            .set_speaker_count(shortcut::speaker_count(count))
            .map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub fn shortcut_status(registration: State<'_, ShortcutRegistration>) -> ShortcutStatuses {
    registration.statuses()
}

/// Set, change or clear one shortcut. `None` clears it. Async, so the plugin
/// calls it makes run off the main thread.
#[tauri::command]
#[specta::specta]
pub async fn set_shortcut(
    app: AppHandle,
    kind: ShortcutKind,
    chord: Option<String>,
) -> Result<ShortcutStatuses, String> {
    blocking(move || shortcut::set_shortcut(&app, kind, chord)).await
}

/// Pause both shortcuts while the user presses keys for a new one (R11).
#[tauri::command]
#[specta::specta]
pub async fn suspend_shortcuts(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        shortcut::suspend(&app);
        Ok(())
    })
    .await
}

/// Register the saved chords again after capture ends.
#[tauri::command]
#[specta::specta]
pub async fn resume_shortcuts(app: AppHandle) -> Result<(), String> {
    blocking(move || {
        shortcut::resume(&app);
        Ok(())
    })
    .await
}

/// The text of an error from a command that opens the microphone. A Stop
/// that cancelled the open is the user's own request, so its text is empty
/// and the page shows no message.
pub(crate) fn open_error(error: CaptureError) -> String {
    match error {
        CaptureError::Audio(audio::Error::MicStartCancelled) => String::new(),
        error => error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managers::shared::{NoteTypeEntry, NoteTypeList};

    fn entry(name: &str, valid: bool) -> NoteTypeEntry {
        NoteTypeEntry {
            name: name.into(),
            valid,
            shared: true,
            notes: 0,
            general: false,
            prompt_choice: None,
            prompt_name: None,
            summary_prompt_id: summarize::prompts::TOPIC_SUMMARY_ID.into(),
        }
    }

    fn list(default_type: Option<&str>) -> NoteTypeList {
        NoteTypeList {
            types: vec![
                NoteTypeEntry {
                    general: true,
                    shared: false,
                    ..entry("General", true)
                },
                entry("Pathfinder", true),
                entry("Bad:Name", false),
            ],
            default_type: default_type.map(String::from),
        }
    }

    // R7, R23: no choice is the default type, a shared setting. The
    // shortcut and the New Note button use it.
    #[test]
    fn no_choice_and_the_default_choice_use_the_default_type() {
        let types = list(Some("Pathfinder"));
        assert_eq!(chosen_type(None, &types), Ok(Some("Pathfinder".into())));
        assert_eq!(
            chosen_type(Some(&NewNoteType::Default), &types),
            Ok(Some("Pathfinder".into()))
        );
    }

    // R7: a default that names no valid type of the effective list is
    // General (the list gives `None`).
    #[test]
    fn a_default_type_that_is_gone_starts_the_note_in_general() {
        assert_eq!(chosen_type(None, &list(None)), Ok(None));
    }

    // R24: the New Note menu starts a note of a named type, or of General.
    #[test]
    fn a_named_type_uses_the_spelling_of_the_list() {
        let types = list(Some("Pathfinder"));
        assert_eq!(chosen_type(Some(&NewNoteType::General), &types), Ok(None));
        assert_eq!(
            chosen_type(
                Some(&NewNoteType::Named {
                    name: " pathfinder".into()
                }),
                &types
            ),
            Ok(Some("Pathfinder".into()))
        );
    }

    // KTD8, KTD9: a type that is not in the list, or that is not valid, is
    // refused. General is chosen only as `NewNoteType::General`, so a type
    // name never makes a folder named "General".
    #[test]
    fn a_type_that_is_not_a_valid_type_of_the_list_is_refused() {
        let types = list(None);
        for name in ["Gone", "Bad:Name", "General"] {
            assert_eq!(
                chosen_type(Some(&NewNoteType::Named { name: name.into() }), &types),
                Err("That type no longer exists.".to_string()),
                "{name}"
            );
        }
    }

    #[test]
    fn a_cancelled_open_has_no_text_and_other_open_errors_keep_theirs() {
        assert_eq!(
            open_error(CaptureError::Audio(audio::Error::MicStartCancelled)),
            ""
        );
        let timeout = CaptureError::Audio(audio::Error::MicStreamInitializationFailed(
            "the microphone did not start within 5s".into(),
        ));
        assert!(open_error(timeout).contains("did not start within 5s"));
    }
}
