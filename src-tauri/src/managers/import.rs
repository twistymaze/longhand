//! The import manager (U4 of the audio import plan): the import worker of
//! `pipeline::import` in the app. Its journal is a file in the app data
//! folder, next to `settings.json` (KTD6).
//!
//! The manual import (U5) and the watched folder (U6) call
//! [`ImportManager::import`] off the main thread. The notices (U8) read
//! [`ImportManager::notices`].

use std::path::PathBuf;
use std::sync::Arc;

use pipeline::early_home::EarlyHomes;
use pipeline::import::{
    ComputerId, ImportEnvironment, ImportEvent, Imports, JOURNAL_FILE, Notices, Request,
};
use session_core::jobs::Claims;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::managers::capture::{self, CaptureManager};
use crate::managers::library::LibraryChanged;
use crate::managers::models::ModelsManager;
use crate::managers::processing::ProcessingManager;
use crate::managers::settings;

/// The wait between two reads of the recording state while a conversion
/// waits for a recording to end.
const SUSPENDED_POLL: std::time::Duration = std::time::Duration::from_millis(500);

/// The import notices of Home changed (U8): an import waits for free space
/// or no longer waits, or the decoder refused a file. Home reads the
/// library notices again.
#[derive(serde::Serialize, Clone, Debug, specta::Type, tauri_specta::Event)]
pub struct ImportNoticesChanged;

/// The imports of this computer.
pub struct ImportManager {
    app: AppHandle,
    /// `None` when the journal cannot be read: imports are then off, and
    /// the journal file stays as it is.
    imports: Option<Arc<Imports>>,
}

impl ImportManager {
    /// Read the journal. `suspended` is the predicate of the runner: a
    /// recording runs or the live worker drains (KTD9). `claims` and
    /// `early_homes` are processing's.
    pub fn new(
        app: &AppHandle,
        capture: &Arc<CaptureManager>,
        suspended: Box<dyn Fn() -> bool + Send + Sync>,
        claims: Claims,
        early_homes: Arc<EarlyHomes>,
    ) -> Self {
        let imports = open(app, capture, suspended, claims, early_homes);
        if let Err(e) = &imports {
            log::error!("imports are off: {e}");
        }
        Self {
            app: app.clone(),
            imports: imports.ok(),
        }
    }

    /// At launch, with the instance lock held: remove the importing notes
    /// that the journal does not know, then continue each import in the
    /// journal on the import thread.
    pub fn start(&self) {
        if let Some(imports) = &self.imports {
            imports.launch();
            imports.start();
        }
    }

    /// Import the audio file of `request` into a new note (U5, U6). The
    /// note shows on Home at once in the state "Preparing" (R21), and the
    /// worker converts the file and queues the note. Returns the note id.
    /// Call it off the main thread: it reads the file's recording time.
    pub fn import(&self, request: Request) -> Result<String, String> {
        let imports = self.imports.as_ref().ok_or("imports are off")?;
        let settings = settings::get_settings(&self.app);
        let models = self.app.state::<Arc<ModelsManager>>();
        let recognition = capture::import_recognition(&settings, |spec| models.missing_spec(spec))
            .map_err(|e| e.to_string())?;
        imports
            .import(request, recognition)
            .map_err(|e| e.to_string())
    }

    /// Mark the import of the note `note_id` deleted, before the delete of
    /// the note (KTD6). The worker then records the file, so the file does
    /// not come back. False when the note has no open import.
    pub fn delete(&self, note_id: &str) -> bool {
        self.imports
            .as_ref()
            .is_some_and(|imports| imports.delete(note_id))
    }

    /// Try each import that waits again: for free space or the storage
    /// folder. A scan of the watched folder calls it (KTD9).
    pub fn retry(&self) {
        if let Some(imports) = &self.imports {
            imports.retry();
        }
    }

    /// The facts that the notices need (U8): the imports that wait for
    /// free space, and the files that the decoder refused.
    pub fn notices(&self) -> Notices {
        self.imports
            .as_ref()
            .map(|imports| imports.notices())
            .unwrap_or_default()
    }

    /// The user dismissed the failure of the import `note_id` (U8). The
    /// dismissal stays in memory, as the failures do.
    pub fn dismiss(&self, note_id: &str) {
        if let Some(imports) = &self.imports {
            imports.dismiss(note_id);
        }
    }

    /// The source files of the open imports, also of deleted ones. The
    /// watched folder does not import them again (KTD6).
    pub fn open_sources(&self) -> Vec<PathBuf> {
        self.imports
            .as_ref()
            .map(|imports| imports.open_sources())
            .unwrap_or_default()
    }

    /// Before the process exits: the conversion in progress stops. The
    /// imports continue at the next launch.
    pub fn stop(&self) {
        if let Some(imports) = &self.imports {
            imports.stop();
        }
    }
}

fn open(
    app: &AppHandle,
    capture: &Arc<CaptureManager>,
    suspended: Box<dyn Fn() -> bool + Send + Sync>,
    claims: Claims,
    early_homes: Arc<EarlyHomes>,
) -> Result<Arc<Imports>, String> {
    let data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let store = capture.shared_store();
    let computer = pipeline::lock(&store)
        .computer_id()
        .map_err(|e| e.to_string())?;
    let computer_id =
        ComputerId::parse(&computer).ok_or_else(|| format!("{computer} is not a computer id"))?;
    let storage = app.clone();
    let events = app.clone();
    Imports::open(ImportEnvironment {
        store,
        working_root: capture.working_root().to_path_buf(),
        journal: data.join(JOURNAL_FILE),
        computer_id,
        longhand_version: app.package_info().version.to_string(),
        storage_folder: Box::new(move || settings::get_settings(&storage).storage_folder()),
        suspended,
        free_space: Arc::new(recorder::storage::available_space),
        remove: Box::new(crate::commands::session::move_to_trash),
        claims,
        early_homes: Some(early_homes),
        events: Box::new(move |event| on_event(&events, event)),
        poll: SUSPENDED_POLL,
    })
    .map_err(|e| format!("the import journal cannot be read: {e}"))
}

/// Tell Home of each row change, and wake processing for a queued note.
fn on_event(app: &AppHandle, event: ImportEvent) {
    let changes = match event {
        ImportEvent::Created { note_id } => LibraryChanged {
            changed: vec![note_id],
            removed: Vec::new(),
        },
        ImportEvent::Queued { note_id } => {
            if let Some(processing) = app.try_state::<Arc<ProcessingManager>>() {
                processing.wake();
            }
            LibraryChanged {
                changed: vec![note_id],
                removed: Vec::new(),
            }
        }
        ImportEvent::Removed { note_id } => LibraryChanged {
            changed: Vec::new(),
            removed: vec![note_id],
        },
        ImportEvent::Paused { note_id } => {
            log::info!("the import of {note_id} waits for the recording to end");
            return;
        }
        ImportEvent::Continued { note_id } => {
            log::info!("the import of {note_id} continues");
            return;
        }
        ImportEvent::DiskWait { .. } | ImportEvent::DiskClear { .. } | ImportEvent::Failed(_) => {
            if let Err(e) = ImportNoticesChanged.emit(app) {
                log::warn!("could not emit an import notices change: {e}");
            }
            return;
        }
        ImportEvent::Ended { .. } => return,
    };
    if let Err(e) = changes.emit(app) {
        log::warn!("could not emit a library change: {e}");
    }
}
