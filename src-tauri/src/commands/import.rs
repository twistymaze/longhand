//! The manual import (U5 of the audio import plan): the user chooses or
//! drops audio files, and each file becomes a new note (R1, KTD14).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pipeline::import::{Origin, Request};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::managers::destination::{self, DestinationStatus};
use crate::managers::processing::ProcessingManager;
use crate::managers::settings;

use super::blocking;
use super::capture::{NewNoteType, type_of_new_note};

/// What a manual import did.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct ImportResult {
    /// The ids of the new notes, in the order of the paths.
    pub note_ids: Vec<String>,
    /// The files that did not import, each with its reason (R20).
    pub refused: Vec<ImportRefusal>,
}

/// A file that did not import.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct ImportRefusal {
    pub path: String,
    /// The reason, as Home shows it. It names the file.
    pub reason: String,
}

/// Choose audio files with the system dialog (KTD14). The dialog shows
/// only the accepted formats (R19). An empty list when the user closes
/// the dialog.
#[tauri::command]
#[specta::specta]
pub async fn pick_audio_files(app: AppHandle) -> Result<Vec<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .set_title("Import Audio")
            .add_filter("Audio", &audio_import::ACCEPTED_EXTENSIONS)
            .blocking_pick_files()
    })
    .await
    .map_err(|e| e.to_string())?;
    picked
        .unwrap_or_default()
        .into_iter()
        .map(|file| {
            file.into_path()
                .map(|path| path.to_string_lossy().into_owned())
                .map_err(|e| e.to_string())
        })
        .collect()
}

/// Import each file of `paths` into a new note (R1, R3). `note_type` and
/// `tags` apply to each note (R2). `title` is a title that the user set in
/// the import sheet for one file (R2a); with no title, each note gets its
/// file name as a proposed title (R15). Refused while the storage folder
/// is not available, as New Note is. A path that is not a file with an
/// accepted extension gets a refusal, and the other files import (R20).
#[tauri::command]
#[specta::specta]
pub async fn import_files(
    app: AppHandle,
    processing: State<'_, Arc<ProcessingManager>>,
    paths: Vec<String>,
    note_type: Option<NewNoteType>,
    tags: Vec<String>,
    title: Option<String>,
) -> Result<ImportResult, String> {
    let processing = Arc::clone(&processing);
    let folder = settings::get_settings(&app).storage_folder;
    blocking(move || {
        // A folder on a network volume can be slow to answer.
        let storage = destination::status(&folder);
        import_paths(
            &paths,
            &storage,
            &Choices { tags, title },
            || type_of_new_note(&app, note_type.as_ref()),
            |request| processing.imports().import(request),
        )
    })
    .await
}

/// The tags and the title that the import sheet set.
struct Choices {
    tags: Vec<String>,
    title: Option<String>,
}

/// The work of [`import_files`] with the storage folder status `storage`.
/// `note_type` gives the type of the notes; it reads the type list, so it
/// runs after the check of the storage folder, as in New Note. `import`
/// starts the import of one file and returns the note id.
fn import_paths(
    paths: &[String],
    storage: &DestinationStatus,
    choices: &Choices,
    note_type: impl FnOnce() -> Result<Option<String>, String>,
    mut import: impl FnMut(Request) -> Result<String, String>,
) -> Result<ImportResult, String> {
    if let Some(reason) = destination::note_start_refusal(storage) {
        return Err(reason.to_string());
    }
    let note_type = note_type()?;
    let tags = choices
        .tags
        .iter()
        .map(|tag| pipeline::tags::check_tag(tag))
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = ImportResult {
        note_ids: Vec::new(),
        refused: Vec::new(),
    };
    for path in paths {
        let source = PathBuf::from(path);
        let started = if is_audio_file(&source) {
            import(Request {
                source,
                origin: Origin::Manual,
                note_type: note_type.clone(),
                tags: tags.clone(),
                title: choices.title.clone(),
            })
        } else {
            Err(format!(
                "Longhand can’t import “{}”. Only m4a, mp3 and wav files can be imported.",
                file_name(&source)
            ))
        };
        match started {
            Ok(note_id) => result.note_ids.push(note_id),
            Err(reason) => result.refused.push(ImportRefusal {
                path: path.clone(),
                reason,
            }),
        }
    }
    Ok(result)
}

/// Whether `path` is a regular file with an accepted extension (R19).
fn is_audio_file(path: &Path) -> bool {
    audio_import::is_accepted(path) && std::fs::metadata(path).is_ok_and(|m| m.is_file())
}

/// The last part of `path`, as the user knows the file.
fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| path.to_string_lossy(), |name| name.to_string_lossy())
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> DestinationStatus {
        DestinationStatus::Ready {
            path: "/notes".into(),
            free_bytes: None,
        }
    }

    fn choices() -> Choices {
        Choices {
            tags: vec!["Team".into()],
            title: None,
        }
    }

    fn file(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, b"audio").unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Run `import_paths` and record each request.
    fn run(
        paths: &[String],
        storage: &DestinationStatus,
        choices: &Choices,
    ) -> (Result<ImportResult, String>, Vec<Request>) {
        let mut requests = Vec::new();
        let note_type = || Ok(Some("Pathfinder".to_string()));
        let result = import_paths(paths, storage, choices, note_type, |request| {
            requests.push(request);
            Ok(format!("note-{}", requests.len()))
        });
        (result, requests)
    }

    // R1, R2: one note for each file, with the type and the tags of the
    // sheet. With no title, the worker gives the file name (R15).
    #[test]
    fn each_file_imports_with_the_type_and_tags_of_the_sheet() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![file(dir.path(), "a.m4a"), file(dir.path(), "b.MP3")];
        let (result, requests) = run(&paths, &ready(), &choices());
        assert_eq!(
            result,
            Ok(ImportResult {
                note_ids: vec!["note-1".into(), "note-2".into()],
                refused: Vec::new(),
            })
        );
        assert_eq!(requests.len(), 2);
        for (request, path) in requests.iter().zip(&paths) {
            assert_eq!(request.source, PathBuf::from(path));
            assert_eq!(request.origin, Origin::Manual);
            assert_eq!(request.note_type.as_deref(), Some("Pathfinder"));
            assert_eq!(request.tags, vec!["Team".to_string()]);
            assert_eq!(request.title, None);
        }
    }

    // R2a: the title that the user set in the sheet is a user title.
    #[test]
    fn the_title_of_the_sheet_goes_with_the_request() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![file(dir.path(), "Team sync.m4a")];
        let set = Choices {
            title: Some("Budget review".into()),
            ..choices()
        };
        let (_, requests) = run(&paths, &ready(), &set);
        assert_eq!(requests[0].title.as_deref(), Some("Budget review"));
    }

    // AE2, R3: a manual import does not read the import record, so a file
    // that the record holds makes a new note. The worker test
    // `a_manual_import_of_an_imported_file_makes_a_new_note` shows that the
    // worker does not stop a `Manual` request.
    #[test]
    fn a_file_that_the_import_record_holds_makes_a_new_note() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![file(dir.path(), "Team sync.m4a")];
        let (result, requests) = run(&paths, &ready(), &choices());
        assert_eq!(result.unwrap().note_ids, vec!["note-1".to_string()]);
        assert_eq!(requests[0].origin, Origin::Manual);
    }

    // R19, R20: a file with another extension, and a folder, each get one
    // refusal that names them. The other files import.
    #[test]
    fn a_file_with_another_extension_or_a_folder_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let text = file(dir.path(), "notes.txt");
        let audio = file(dir.path(), "Team sync.m4a");
        let folder = dir.path().join("Recordings.m4a");
        std::fs::create_dir(&folder).unwrap();
        let folder = folder.to_string_lossy().into_owned();
        let paths = vec![text.clone(), audio.clone(), folder.clone()];
        let (result, requests) = run(&paths, &ready(), &choices());
        let result = result.unwrap();
        assert_eq!(result.note_ids, vec!["note-1".to_string()]);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].source, PathBuf::from(&audio));
        let refused: Vec<_> = result.refused.iter().map(|r| r.path.clone()).collect();
        assert_eq!(refused, vec![text, folder]);
        assert!(result.refused[0].reason.contains("“notes.txt”"));
        assert!(result.refused[1].reason.contains("“Recordings.m4a”"));
    }

    // R7 of New Note: while the storage folder is not available, no file
    // imports, and the reason is the reason of New Note.
    #[test]
    fn an_unavailable_storage_folder_refuses_all_files_with_the_new_note_reason() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![file(dir.path(), "a.m4a"), file(dir.path(), "b.wav")];
        let missing = DestinationStatus::Missing {
            path: "/Volumes/Gone".into(),
        };
        let (result, requests) = run(&paths, &missing, &choices());
        assert_eq!(result, Err(destination::NOTE_CANNOT_START.to_string()));
        assert!(requests.is_empty());
    }

    // The tags of the sheet pass the check of a tag that the user typed.
    #[test]
    fn a_tag_that_is_not_valid_refuses_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let paths = vec![file(dir.path(), "a.m4a")];
        let set = Choices {
            tags: vec!["  ".into()],
            ..choices()
        };
        let (result, requests) = run(&paths, &ready(), &set);
        assert!(result.is_err());
        assert!(requests.is_empty());
    }
}
