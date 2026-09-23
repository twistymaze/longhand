//! Model weights on this machine: where they live, which are present, and
//! the background downloads that fetch them (KTD19, R28, R1c).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use models::download::{Event as DownloadProgress, partial_path};
use models::{CancellationToken, CatalogModel, Limits, ModelKind, Outcome};
use serde::Serialize;
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Recognition,
    Segmentation,
    Embedding,
}

impl From<ModelKind> for ModelRole {
    fn from(kind: ModelKind) -> Self {
        match kind {
            ModelKind::Asr => Self::Recognition,
            ModelKind::Segmentation => Self::Segmentation,
            ModelKind::Embedding => Self::Embedding,
        }
    }
}

#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ModelState {
    Present,
    Absent { partial_bytes: u64 },
    Downloading,
}

#[derive(Serialize, Type, Debug, Clone)]
pub struct LicenceInfo {
    pub name: String,
    pub url: String,
    pub attribution: String,
}

#[derive(Serialize, Type, Debug, Clone)]
pub struct ModelStatus {
    pub id: String,
    pub role: ModelRole,
    pub name: String,
    pub description: String,
    /// Recognition models only: whether a live transcript is possible (R10).
    pub streaming: bool,
    pub size_bytes: u64,
    /// Needed by the current settings: the selected recognition model and
    /// both diarization weights.
    pub required: bool,
    pub state: ModelState,
    pub repository: String,
    pub revision: String,
    pub licence: LicenceInfo,
}

#[derive(Serialize, Type, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DownloadUpdate {
    Progress { downloaded: u64, total: u64 },
    Verifying,
    Completed,
    Cancelled,
    Failed { message: String },
}

#[derive(Serialize, Type, Debug, Clone, Event)]
pub struct ModelDownloadEvent {
    pub model_id: String,
    pub update: DownloadUpdate,
}

pub struct ModelsManager {
    dir: PathBuf,
    active: Mutex<HashMap<String, CancellationToken>>,
}

impl ModelsManager {
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        Ok(Self {
            dir: app.path().app_data_dir()?.join("models"),
            active: Mutex::new(HashMap::new()),
        })
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancellationToken>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn statuses(&self, asr_model: &str) -> Vec<ModelStatus> {
        let required: Vec<&str> = models::required_for(asr_model)
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        let active = self.active();
        models::CATALOG
            .iter()
            .map(|m| ModelStatus {
                id: m.id.clone(),
                role: m.kind.into(),
                name: m.name.clone(),
                description: m.description.clone(),
                size_bytes: m.size_bytes,
                streaming: m.streaming,
                required: required.contains(&m.id.as_str()),
                state: self.state_of(m, active.contains_key(&m.id)),
                repository: m.repository.clone(),
                revision: m.revision.clone(),
                licence: LicenceInfo {
                    name: m.licence.name.clone(),
                    url: m.licence.url.clone(),
                    attribution: m.licence.attribution.clone(),
                },
            })
            .collect()
    }

    fn state_of(&self, model: &CatalogModel, downloading: bool) -> ModelState {
        if models::is_present(&self.dir, model) {
            ModelState::Present
        } else if downloading {
            ModelState::Downloading
        } else {
            let partial_bytes = partial_path(&self.dir.join(&model.filename))
                .metadata()
                .map(|m| m.len())
                .unwrap_or(0);
            ModelState::Absent { partial_bytes }
        }
    }

    /// Where a catalog model lives on disk, present or not.
    pub fn file_path(&self, id: &str) -> Option<PathBuf> {
        models::model(id).map(|m| self.dir.join(&m.filename))
    }

    /// Catalog ids required by `asr_model` that are not on disk.
    pub fn missing(&self, asr_model: &str) -> Vec<String> {
        models::required_for(asr_model)
            .into_iter()
            .filter(|m| !models::is_present(&self.dir, m))
            .map(|m| m.id.clone())
            .collect()
    }

    /// Fetch `ids` one after another in the background. An id already
    /// present or already downloading is skipped; an unknown id is refused.
    pub fn start_downloads(
        self: &Arc<Self>,
        app: &AppHandle,
        ids: Vec<String>,
    ) -> Result<(), String> {
        let mut wanted = Vec::new();
        for id in &ids {
            let model = models::model(id).ok_or_else(|| format!("no model named '{id}'"))?;
            if !models::is_present(&self.dir, model) {
                wanted.push(model);
            }
        }
        let token = CancellationToken::new();
        {
            let mut active = self.active();
            wanted.retain(|m| !active.contains_key(&m.id));
            for model in &wanted {
                active.insert(model.id.clone(), token.clone());
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }
        let manager = Arc::clone(self);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            for model in wanted {
                let update = manager.download_one(&app, model, &token).await;
                manager.active().remove(&model.id);
                emit(&app, &model.id, update);
            }
        });
        Ok(())
    }

    async fn download_one(
        &self,
        app: &AppHandle,
        model: &'static CatalogModel,
        token: &CancellationToken,
    ) -> DownloadUpdate {
        if token.is_cancelled() {
            return DownloadUpdate::Cancelled;
        }
        // Progress arrives per network chunk; the surface needs a whole
        // percent at most.
        let last_percent = Mutex::new(u64::MAX);
        let on_event = |event: DownloadProgress<'_>| match event {
            DownloadProgress::Progress(p) => {
                let percent = (p.downloaded * 100).checked_div(p.total).unwrap_or(0);
                let mut last = last_percent.lock().unwrap_or_else(|e| e.into_inner());
                if *last != percent {
                    *last = percent;
                    emit(
                        app,
                        &model.id,
                        DownloadUpdate::Progress {
                            downloaded: p.downloaded,
                            total: p.total,
                        },
                    );
                }
            }
            DownloadProgress::VerificationStarted => {
                emit(app, &model.id, DownloadUpdate::Verifying)
            }
            DownloadProgress::VerificationCompleted => {}
        };
        match models::ensure_model(&self.dir, model, Limits::default(), token, &on_event).await {
            Ok(Outcome::Completed) => {
                log::info!("model '{}' downloaded and verified", model.id);
                DownloadUpdate::Completed
            }
            Ok(Outcome::Cancelled) => DownloadUpdate::Cancelled,
            Err(e) => {
                log::warn!("model '{}' download failed: {e}", model.id);
                DownloadUpdate::Failed {
                    message: e.to_string(),
                }
            }
        }
    }

    /// Stop every download in flight. Partials are kept, so the next
    /// download resumes where this one stopped.
    pub fn cancel_all(&self) {
        for token in self.active().values() {
            token.cancel();
        }
    }
}

fn emit(app: &AppHandle, model_id: &str, update: DownloadUpdate) {
    let event = ModelDownloadEvent {
        model_id: model_id.to_string(),
        update,
    };
    if let Err(e) = event.emit(app) {
        log::warn!("failed to emit model download event: {e}");
    }
}
