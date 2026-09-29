//! Model weights on this machine: where they live, which are present, and
//! the background downloads that fetch them (KTD19, R28, R1c).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use models::download::{Event as DownloadProgress, partial_path};
use models::{CancellationToken, CatalogModel, Limits, ModelKind, Outcome};
use serde::Serialize;
use session_core::{SessionRecord, SessionState};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::managers::settings::{self, Settings, SettingsChanged};

#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelRole {
    Recognition,
    RecognitionArtifact,
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
    pub speaker_preferences: Option<models::catalog::SpeakerPreferences>,
    pub id: String,
    pub role: ModelRole,
    pub name: String,
    pub description: String,
    /// Recognition models only: whether a live transcript is possible (R10).
    pub streaming: bool,
    /// Recognition models only: the language codes the model recognizes.
    pub languages: Vec<String>,
    /// The model the picker marks as recommended (R4).
    pub recommended: bool,
    pub size_bytes: u64,
    /// Needed by the current settings: the selected recognition model and
    /// both diarization weights.
    pub required: bool,
    pub state: ModelState,
    pub repository: String,
    pub revision: String,
    pub licence: LicenceInfo,
    pub assessment: Option<models::catalog::Assessment>,
    /// The artifact ids of the model bundle (KTD6). A model that is not a
    /// transcription model has only its own id.
    pub bundle: Vec<String>,
    /// The total size of the bundle.
    pub bundle_size_bytes: u64,
    /// Every file of the bundle is on disk.
    pub bundle_complete: bool,
}

/// The speaker-detection weights as one item (R7).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SpeakerModelsStatus {
    /// The catalog ids, for one download of all of them.
    pub ids: Vec<String>,
    /// The sum of the sizes.
    pub size_bytes: u64,
    /// Present only when every weight is present.
    pub state: ModelState,
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

/// What a pick of a recognition model does, after the settings change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    /// Start the download of the picked model.
    pub download: bool,
    /// Stop the download of a pending pick that this pick replaces.
    pub cancel: Option<String>,
}

/// Apply a pick of the recognition model `id` to `settings` (R5, KTD11).
/// The pick is selected at once when it is downloaded, or when the selected
/// model is not downloaded. Otherwise the selected model stays in use and the
/// pick waits as the pending pick until its download is verified.
pub fn apply_pick(settings: &mut Settings, id: &str, is_present: impl Fn(&str) -> bool) -> Pick {
    settings.pending_recognition = None;
    let cancel = settings.pending_asr_model.take().filter(|p| p != id);
    let picked_present = is_present(id);
    if id == settings.asr_model || picked_present || !is_present(&settings.asr_model) {
        settings.asr_model = id.to_string();
    } else {
        settings.pending_asr_model = Some(id.to_string());
    }
    Pick {
        download: !picked_present,
        cancel,
    }
}

/// Apply the outcome of a download of `id` to the pending pick. Returns
/// whether the settings changed.
pub fn settle_pending(settings: &mut Settings, id: &str, update: &DownloadUpdate) -> bool {
    if settings.pending_asr_model.as_deref() != Some(id) {
        return false;
    }
    match update {
        DownloadUpdate::Completed => {
            settings.asr_model = id.to_string();
            settings.pending_asr_model = None;
            true
        }
        DownloadUpdate::Failed { .. } | DownloadUpdate::Cancelled => {
            settings.pending_asr_model = None;
            true
        }
        DownloadUpdate::Progress { .. } | DownloadUpdate::Verifying => false,
    }
}

/// Why the app cannot delete the model `id` now, or `None` when it can
/// (R6, KTD12). `live_model` is the model of the live worker.
pub fn delete_refusal(
    id: &str,
    settings: &Settings,
    live_model: Option<&str>,
    sessions: &[SessionRecord],
) -> Option<String> {
    let Some(catalog_model) = models::model(id) else {
        return Some(format!("no model named '{id}'"));
    };
    if settings.asr_model == id
        || models::recognition::resolve(&settings.recognition_config())
            .is_ok_and(|spec| spec.artifacts.iter().any(|artifact| artifact.id == id))
    {
        return Some("This model is selected. Select another model first.".into());
    }
    if settings
        .pending_recognition
        .as_ref()
        .and_then(|c| models::recognition::resolve(c).ok())
        .is_some_and(|spec| spec.artifacts.iter().any(|a| a.id == id))
    {
        return Some("This model is part of a pending configuration.".into());
    }
    if settings.pending_asr_model.as_deref() == Some(id) {
        return Some("This model is downloading to become the selected model.".into());
    }
    if live_model == Some(id) {
        return Some("The current recording uses this model.".into());
    }
    // A Transcribe Again run keeps the old `asr_model` until its save and
    // uses the run's model. This is the same order as `Runner::run_next`.
    let needed = sessions.iter().any(|s| {
        let model = s
            .transcribe_again_model
            .as_deref()
            .or(s.asr_model.as_deref());
        let snapshot_needs = s
            .recognition_snapshot
            .as_ref()
            .and_then(|v| {
                serde_json::from_value::<models::recognition::ExecutionSpec>(v.clone()).ok()
            })
            .is_some_and(|spec| spec.artifacts.iter().any(|a| a.id == id));
        // Older notes use the original speaker pipeline without a snapshot.
        let legacy_speaker_needs = s.recognition_snapshot.is_none()
            && matches!(
                catalog_model.kind,
                ModelKind::Segmentation | ModelKind::Embedding
            );
        (model == Some(id) || snapshot_needs || legacy_speaker_needs) && is_unfinished(s)
    });
    if needed {
        return Some("A note that isn't finished needs this model.".into());
    }
    None
}

/// Whether a session is not finished: every state except Saved and
/// Discarded. This is the same rule as [`delete_refusal`].
fn is_unfinished(session: &SessionRecord) -> bool {
    !matches!(session.state, SessionState::Saved | SessionState::Discarded)
}

/// The supporting models on disk that no kept bundle contains (KTD7). The
/// kept bundles are the bundles of the transcription models on disk, of the
/// selected model, and of the pending model. The caller also applies
/// [`delete_refusal`] to each result, so that unfinished notes and the live
/// model keep their files.
pub fn unneeded_supporting_models(statuses: &[ModelStatus], settings: &Settings) -> Vec<String> {
    let mut kept_models: Vec<&str> = statuses
        .iter()
        .filter(|s| s.role == ModelRole::Recognition && s.state == ModelState::Present)
        .map(|s| s.id.as_str())
        .collect();
    kept_models.push(&settings.asr_model);
    kept_models.extend(settings.pending_asr_model.as_deref());
    kept_models.extend(
        settings
            .pending_recognition
            .as_ref()
            .map(|c| c.asr_model.as_str()),
    );
    let kept: Vec<String> = kept_models
        .into_iter()
        .flat_map(models::recognition::bundle)
        .map(|a| a.id)
        .collect();
    statuses
        .iter()
        .filter(|s| s.role != ModelRole::Recognition && s.state == ModelState::Present)
        .filter(|s| !kept.contains(&s.id))
        .map(|s| s.id.clone())
        .collect()
}

/// One status for the speaker-detection weights (R7).
pub fn speaker_models(statuses: &[ModelStatus]) -> SpeakerModelsStatus {
    let weights: Vec<_> = statuses
        .iter()
        .filter(|s| s.required && matches!(s.role, ModelRole::Segmentation | ModelRole::Embedding))
        .collect();
    let state = if weights.iter().all(|s| s.state == ModelState::Present) {
        ModelState::Present
    } else if weights.iter().any(|s| s.state == ModelState::Downloading) {
        ModelState::Downloading
    } else {
        // The bytes on disk: a present weight counts in full, so one
        // progress covers both weights.
        let partial_bytes = weights
            .iter()
            .map(|s| match s.state {
                ModelState::Absent { partial_bytes } => partial_bytes,
                ModelState::Present => s.size_bytes,
                ModelState::Downloading => 0,
            })
            .sum();
        ModelState::Absent { partial_bytes }
    };
    SpeakerModelsStatus {
        ids: weights.iter().map(|s| s.id.clone()).collect(),
        size_bytes: weights.iter().map(|s| s.size_bytes).sum(),
        state,
    }
}

pub struct ModelsManager {
    dir: PathBuf,
    active: Mutex<HashMap<String, CancellationToken>>,
    /// The model that the live worker has loaded, when one runs (KTD12).
    live_model: Mutex<Option<String>>,
}

impl ModelsManager {
    pub fn new(app: &AppHandle) -> tauri::Result<Self> {
        Ok(Self::at(app.path().app_data_dir()?.join("models")))
    }

    pub(crate) fn at(dir: PathBuf) -> Self {
        Self {
            dir,
            active: Mutex::new(HashMap::new()),
            live_model: Mutex::new(None),
        }
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancellationToken>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Record the model that the live worker uses, or `None` when it stops.
    /// Delete refuses that model.
    pub fn set_live_model(&self, model: Option<String>) {
        *self.live_model.lock().unwrap_or_else(|e| e.into_inner()) = model;
    }

    pub fn live_model(&self) -> Option<String> {
        self.live_model
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn statuses(&self, asr_model: &str) -> Vec<ModelStatus> {
        let required: Vec<&str> = models::required_for(asr_model)
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        let active = self.active();
        models::CATALOG
            .iter()
            .map(|m| (m, m.kind.into()))
            .chain(
                models::catalog::RUNTIME_ARTIFACTS
                    .iter()
                    .map(|m| (m, ModelRole::RecognitionArtifact)),
            )
            .map(|(m, role)| {
                let state = self.state_of(
                    m,
                    active.contains_key(&m.id)
                        || active.contains_key(&format!("{}@{}", m.id, m.sha256)),
                );
                let bundle = if role == ModelRole::Recognition {
                    models::recognition::bundle(&m.id)
                } else {
                    Vec::new()
                };
                let (bundle, bundle_size_bytes, bundle_complete) = if bundle.is_empty() {
                    (
                        vec![m.id.clone()],
                        m.size_bytes,
                        state == ModelState::Present,
                    )
                } else {
                    (
                        bundle.iter().map(|a| a.id.clone()).collect(),
                        bundle.iter().map(|a| a.size_bytes).sum(),
                        bundle.iter().all(|a| self.artifact_path(a).is_some()),
                    )
                };
                ModelStatus {
                    bundle,
                    bundle_size_bytes,
                    bundle_complete,
                    speaker_preferences: m.speaker_preferences.clone(),
                    id: m.id.clone(),
                    assessment: m.assessment.clone(),
                    role,
                    name: m.name.clone(),
                    description: m.description.clone(),
                    size_bytes: m.size_bytes,
                    streaming: m.streaming,
                    languages: m.languages.clone(),
                    recommended: m.recommended,
                    required: required.contains(&m.id.as_str()),
                    state,
                    repository: m.repository.clone(),
                    revision: m.revision.clone(),
                    licence: LicenceInfo {
                        name: m.licence.name.clone(),
                        url: m.licence.url.clone(),
                        attribution: m.licence.attribution.clone(),
                    },
                }
            })
            .collect()
    }

    fn state_of(&self, model: &CatalogModel, downloading: bool) -> ModelState {
        if self.is_present(&model.id) {
            ModelState::Present
        } else if downloading {
            ModelState::Downloading
        } else {
            let legacy = partial_path(&self.dir.join(&model.filename));
            let pinned = partial_path(
                &self
                    .dir
                    .join("pinned")
                    .join(&model.sha256)
                    .join(&model.filename),
            );
            let partial_bytes = [legacy, pinned]
                .iter()
                .filter_map(|p| p.metadata().ok())
                .map(|m| m.len())
                .max()
                .unwrap_or(0);
            ModelState::Absent { partial_bytes }
        }
    }

    /// Where a catalog model lives on disk, present or not.
    pub fn file_path(&self, id: &str) -> Option<PathBuf> {
        models::model(id).map(|m| {
            let pinned = self.dir.join("pinned").join(&m.sha256).join(&m.filename);
            if pinned
                .metadata()
                .is_ok_and(|f| f.is_file() && f.len() == m.size_bytes)
            {
                pinned
            } else {
                self.dir.join(&m.filename)
            }
        })
    }

    /// Whether the catalog model `id` is on disk and complete.
    pub fn is_present(&self, id: &str) -> bool {
        models::model(id).is_some_and(|m| {
            self.file_path(id).is_some_and(|p| {
                p.metadata()
                    .is_ok_and(|f| f.is_file() && f.len() == m.size_bytes)
            })
        })
    }

    /// Resolve an exact snapshot file. Prefer its isolated pinned copy.
    pub fn artifact_path(&self, artifact: &models::recognition::ArtifactSpec) -> Option<PathBuf> {
        if models::download::pinned_artifact_present(&self.dir, artifact) {
            return models::download::pinned_artifact_path(&self.dir, artifact).ok();
        }
        models::model(&artifact.id)
            .filter(|m| {
                m.sha256 == artifact.sha256
                    && m.filename == artifact.filename
                    && m.revision == artifact.revision
                    && m.size_bytes == artifact.size_bytes
                    && models::is_present(&self.dir, m)
            })
            .map(|m| self.dir.join(&m.filename))
    }

    pub fn missing_spec(&self, spec: &models::recognition::ExecutionSpec) -> Vec<String> {
        spec.artifacts
            .iter()
            .filter(|a| self.artifact_path(a).is_none())
            .map(|a| a.id.clone())
            .collect()
    }

    /// Download the missing files of the bundle of `asr_model` (KTD5,
    /// KTD9), except the ids in `skip`, which another download fetches.
    pub fn start_bundle_downloads(
        self: &Arc<Self>,
        app: &AppHandle,
        asr_model: &str,
        skip: &[String],
    ) -> Result<(), String> {
        let artifacts: Vec<_> = models::recognition::bundle(asr_model)
            .into_iter()
            .filter(|a| !skip.contains(&a.id))
            .collect();
        self.start_artifact_downloads(app, &artifacts)
    }

    fn start_artifact_downloads(
        self: &Arc<Self>,
        app: &AppHandle,
        artifacts: &[models::recognition::ArtifactSpec],
    ) -> Result<(), String> {
        for artifact in artifacts {
            models::download::pinned_artifact_path(&self.dir, artifact)
                .map_err(|e| e.to_string())?;
        }
        let mut wanted = Vec::new();
        {
            let mut active = self.active();
            for artifact in artifacts {
                let key = format!("{}@{}", artifact.id, artifact.sha256);
                if self.artifact_path(artifact).is_none() && !active.contains_key(&key) {
                    let token = CancellationToken::new();
                    active.insert(key.clone(), token.clone());
                    wanted.push((key, artifact.clone(), token));
                }
            }
        }
        let manager = Arc::clone(self);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            for (key, artifact, token) in wanted {
                let progress = |event: DownloadProgress<'_>| match event {
                    DownloadProgress::Progress(p) => emit(
                        &app,
                        &artifact.id,
                        DownloadUpdate::Progress {
                            downloaded: p.downloaded,
                            total: p.total,
                        },
                    ),
                    DownloadProgress::VerificationStarted => {
                        emit(&app, &artifact.id, DownloadUpdate::Verifying)
                    }
                    DownloadProgress::VerificationCompleted => {}
                };
                let update = match models::download::ensure_artifact(
                    &manager.dir,
                    &artifact,
                    Limits::default(),
                    &token,
                    &progress,
                )
                .await
                {
                    Ok(Outcome::Completed) => DownloadUpdate::Completed,
                    Ok(Outcome::Cancelled) => DownloadUpdate::Cancelled,
                    Err(e) => DownloadUpdate::Failed {
                        message: e.to_string(),
                    },
                };
                manager.active().remove(&key);
                if models::model(&artifact.id)
                    .is_some_and(|m| m.sha256 == artifact.sha256 && m.revision == artifact.revision)
                {
                    settle_pending_pick(&app, &artifact.id, &update);
                }
                emit(&app, &artifact.id, update);
            }
        });
        Ok(())
    }

    pub fn statuses_for(
        &self,
        config: &models::recognition::RecognitionConfig,
    ) -> Result<Vec<ModelStatus>, String> {
        let spec = models::recognition::resolve(config).map_err(|e| e.to_string())?;
        let mut statuses = self.statuses(&config.asr_model);
        for status in &mut statuses {
            status.required = spec.artifacts.iter().any(|a| a.id == status.id);
        }
        Ok(statuses)
    }

    /// Catalog ids required by `asr_model` that are not on disk.
    pub fn missing(&self, asr_model: &str) -> Vec<String> {
        models::required_for(asr_model)
            .into_iter()
            .filter(|m| !self.is_present(&m.id))
            .map(|m| m.id.clone())
            .collect()
    }

    /// Remove the file of the model `id` and its partial download. The
    /// caller applies [`delete_refusal`] first. A download in progress is
    /// refused, because it continues to write the partial file.
    pub fn delete_files(&self, id: &str) -> Result<(), String> {
        let model = models::model(id).ok_or_else(|| format!("no model named '{id}'"))?;
        let legacy = self.dir.join(&model.filename);
        let pinned = self
            .dir
            .join("pinned")
            .join(&model.sha256)
            .join(&model.filename);
        if self
            .active()
            .keys()
            .any(|key| key == id || key.starts_with(&format!("{id}@")))
        {
            return Err("This model is downloading. Cancel the download first.".into());
        }
        for path in [partial_path(&legacy), legacy, partial_path(&pinned), pinned] {
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    log::warn!("could not remove model file {}: {e}", path.display());
                    return Err(format!("Couldn't delete the model: {e}"));
                }
            }
        }
        log::info!("model '{id}' deleted");
        Ok(())
    }

    /// Fetch `ids` one after another in the background. An id already
    /// present or already downloading is skipped; an unknown id is refused.
    /// Each model gets its own cancellation, so a cancel of one pick stops
    /// only that download.
    pub fn start_downloads(
        self: &Arc<Self>,
        app: &AppHandle,
        ids: Vec<String>,
    ) -> Result<(), String> {
        let artifacts = ids
            .iter()
            .map(|id| {
                let m = models::model(id).ok_or_else(|| format!("no model named '{id}'"))?;
                Ok(models::recognition::ArtifactSpec {
                    id: m.id.clone(),
                    filename: m.filename.clone(),
                    sha256: m.sha256.clone(),
                    revision: m.revision.clone(),
                    url: m.url.clone(),
                    size_bytes: m.size_bytes,
                    format: String::new(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.start_artifact_downloads(app, &artifacts)
    }

    /// Stop the download of `id`, if one is in progress. The partial is
    /// kept.
    pub fn cancel(&self, id: &str) {
        for (key, token) in self.active().iter() {
            if key == id || key.starts_with(&format!("{id}@")) {
                token.cancel();
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

/// A download of the pending pick finished: select it, or clear it after a
/// failure, and tell the surface. The settings change before the download
/// event is emitted, so a surface that reads the settings on the event sees
/// the new selection.
fn settle_pending_pick(app: &AppHandle, id: &str, update: &DownloadUpdate) {
    let current = settings::get_settings(app);
    if current.pending_asr_model.as_deref() != Some(id) && current.pending_recognition.is_none() {
        return;
    }
    let mut changed = false;
    match settings::update_settings(app, |s| {
        changed = settle_pending(s, id, update);
        if let Some(config) = s.pending_recognition.clone() {
            let spec = models::recognition::resolve(&config).map_err(|e| e.to_string())?;
            if spec.artifacts.iter().any(|a| a.id == id) {
                match update {
                    DownloadUpdate::Completed => {
                        if app
                            .state::<Arc<ModelsManager>>()
                            .missing_spec(&spec)
                            .is_empty()
                        {
                            s.apply_recognition(&config);
                            changed = true;
                        }
                    }
                    DownloadUpdate::Failed { .. } | DownloadUpdate::Cancelled => {
                        s.pending_recognition = None;
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }) {
        Ok(settings) if changed => {
            log::info!(
                "pending model '{id}' settled; selected model is '{}'",
                settings.asr_model
            );
            if let Err(e) = (SettingsChanged { settings }).emit(app) {
                log::warn!("failed to emit the settings change: {e}");
            }
        }
        Ok(_) => {}
        Err(e) => log::error!("could not settle the pending model '{id}': {e}"),
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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset};

    use super::*;

    const WHISPER: &str = "whisper-large-v3-turbo-q5km";
    const PARAKEET: &str = "parakeet-unified-en-0.6b-q80";
    const TDT_V3: &str = "parakeet-tdt-0.6b-v3-q80";
    const NEMOTRON_3_5: &str = "nemotron-3.5-asr-streaming-0.6b-q80";

    fn whisper_selected() -> Settings {
        Settings {
            asr_model: WHISPER.into(),
            ..Settings::default()
        }
    }

    fn only(present: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |id| present.contains(&id)
    }

    #[test]
    fn a_pick_waits_for_its_download_while_the_selected_model_stays() {
        // AE5: Whisper is selected and downloaded, Parakeet TDT v3 is not.
        let mut settings = whisper_selected();
        let pick = apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        assert_eq!(
            pick,
            Pick {
                download: true,
                cancel: None
            }
        );
        assert_eq!(settings.asr_model, WHISPER);
        assert_eq!(settings.pending_asr_model.as_deref(), Some(TDT_V3));

        // Progress and the outcome of another model change nothing.
        assert!(!settle_pending(
            &mut settings,
            TDT_V3,
            &DownloadUpdate::Verifying
        ));
        assert!(!settle_pending(
            &mut settings,
            NEMOTRON_3_5,
            &DownloadUpdate::Completed
        ));
        assert_eq!(settings.asr_model, WHISPER);

        assert!(settle_pending(
            &mut settings,
            TDT_V3,
            &DownloadUpdate::Completed
        ));
        assert_eq!(settings.asr_model, TDT_V3);
        assert_eq!(settings.pending_asr_model, None);
    }

    #[test]
    fn a_pick_with_the_selected_model_absent_applies_at_once() {
        let mut settings = whisper_selected();
        let pick = apply_pick(&mut settings, TDT_V3, only(&[]));
        assert!(pick.download);
        assert_eq!(settings.asr_model, TDT_V3);
        assert_eq!(settings.pending_asr_model, None);
    }

    #[test]
    fn a_pick_of_a_downloaded_model_applies_at_once() {
        let mut settings = whisper_selected();
        let pick = apply_pick(&mut settings, TDT_V3, only(&[WHISPER, TDT_V3]));
        assert!(!pick.download);
        assert_eq!(settings.asr_model, TDT_V3);
    }

    #[test]
    fn a_second_pick_replaces_the_pending_pick_and_cancels_its_download() {
        let mut settings = whisper_selected();
        apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        let pick = apply_pick(&mut settings, NEMOTRON_3_5, only(&[WHISPER]));
        assert_eq!(
            pick,
            Pick {
                download: true,
                cancel: Some(TDT_V3.into())
            }
        );
        assert_eq!(settings.pending_asr_model.as_deref(), Some(NEMOTRON_3_5));
        // The cancelled download does not clear the new pick.
        assert!(!settle_pending(
            &mut settings,
            TDT_V3,
            &DownloadUpdate::Cancelled
        ));
        assert_eq!(settings.pending_asr_model.as_deref(), Some(NEMOTRON_3_5));
    }

    #[test]
    fn a_pick_of_the_pending_model_again_keeps_its_download() {
        let mut settings = whisper_selected();
        apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        let pick = apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        assert_eq!(
            pick,
            Pick {
                download: true,
                cancel: None
            }
        );
        assert_eq!(settings.pending_asr_model.as_deref(), Some(TDT_V3));
    }

    #[test]
    fn a_pick_of_the_selected_model_clears_the_pending_pick() {
        let mut settings = whisper_selected();
        apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        let pick = apply_pick(&mut settings, WHISPER, only(&[WHISPER]));
        assert_eq!(
            pick,
            Pick {
                download: false,
                cancel: Some(TDT_V3.into())
            }
        );
        assert_eq!(settings.asr_model, WHISPER);
        assert_eq!(settings.pending_asr_model, None);
    }

    #[test]
    fn a_failed_download_clears_the_pending_pick() {
        let mut settings = whisper_selected();
        apply_pick(&mut settings, TDT_V3, only(&[WHISPER]));
        let failed = DownloadUpdate::Failed {
            message: "checksum".into(),
        };
        assert!(settle_pending(&mut settings, TDT_V3, &failed));
        assert_eq!(settings.asr_model, WHISPER);
        assert_eq!(settings.pending_asr_model, None);
    }

    fn note(model: &str, state: SessionState) -> SessionRecord {
        SessionRecord {
            id: "note".into(),
            created_at: stopped(),
            state,
            queue_seq: None,
            speaker_count: None,
            asr_model: Some(model.into()),
            recognition_snapshot: None,
            home_path: None,
            title: None,
            found_speakers: None,
            live_transcript: None,
            transcribe_again_model: None,
        }
    }

    fn stopped() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-25T10:00:00+00:00").unwrap()
    }

    #[test]
    fn delete_refuses_the_selected_model_and_allows_another() {
        // AE8.
        let settings = Settings {
            asr_model: PARAKEET.into(),
            ..Settings::default()
        };
        assert!(delete_refusal(PARAKEET, &settings, None, &[]).is_some());
        assert_eq!(delete_refusal(WHISPER, &settings, None, &[]), None);
    }

    #[test]
    fn status_includes_the_exact_native_artifact_required_by_the_live_route() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().into());
        let config = models::recognition::RecognitionConfig {
            asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
            processing_mode: models::recognition::ProcessingMode::DuringRecording,
            speaker_source: models::recognition::SpeakerSource::Nemotron,
            ..Default::default()
        };
        let statuses = manager.statuses_for(&config).unwrap();
        let native = statuses
            .iter()
            .find(|s| s.id == "nemo-nemotron-speech-streaming-en-0.6b-q80")
            .expect("native artifact status");
        assert!(native.required);
        assert!(matches!(native.state, ModelState::Absent { .. }));
        assert_ne!(native.role, ModelRole::Recognition);
    }

    #[test]
    fn delete_refuses_the_native_artifact_selected_by_the_live_route() {
        let settings = Settings {
            asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
            live_transcript: true,
            speaker_source: models::recognition::SpeakerSource::Nemotron,
            ..Settings::default()
        };
        assert!(
            delete_refusal(
                "nemo-nemotron-speech-streaming-en-0.6b-q80",
                &settings,
                None,
                &[]
            )
            .is_some()
        );
    }

    #[test]
    fn delete_refuses_the_pending_and_the_live_model() {
        let mut settings = whisper_selected();
        settings.pending_asr_model = Some(TDT_V3.into());
        assert!(delete_refusal(TDT_V3, &settings, None, &[]).is_some());
        assert!(delete_refusal(PARAKEET, &settings, Some(PARAKEET), &[]).is_some());
        assert_eq!(
            delete_refusal(PARAKEET, &settings, Some(NEMOTRON_3_5), &[]),
            None
        );
    }

    #[test]
    fn the_live_worker_records_its_model_for_the_delete_check() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().to_path_buf());
        assert_eq!(manager.live_model(), None);
        manager.set_live_model(Some(PARAKEET.into()));
        let live = manager.live_model();
        assert!(delete_refusal(PARAKEET, &whisper_selected(), live.as_deref(), &[]).is_some());
        manager.set_live_model(None);
        assert_eq!(manager.live_model(), None);
    }

    #[test]
    fn delete_refuses_a_model_that_an_unfinished_note_needs() {
        let settings = whisper_selected();
        let stopped_at = stopped();
        let unfinished = [
            SessionState::Capturing,
            SessionState::Queued { stopped_at },
            SessionState::Processing { stopped_at },
            SessionState::Blocked { stopped_at },
            SessionState::ProcessingFailed {
                stopped_at,
                stage: session_core::Stage::Transcribe,
            },
            SessionState::AwaitingDestination {
                stopped_at,
                problem: "offline".into(),
            },
            SessionState::Interrupted {
                reason: session_core::InterruptReason::Crash,
                since: stopped_at,
                acknowledged: false,
            },
        ];
        for state in unfinished {
            let name = state.name();
            let sessions = [note(PARAKEET, state)];
            assert!(
                delete_refusal(PARAKEET, &settings, None, &sessions).is_some(),
                "{name}"
            );
        }
        for state in [SessionState::Saved, SessionState::Discarded] {
            let sessions = [note(PARAKEET, state)];
            assert_eq!(delete_refusal(PARAKEET, &settings, None, &sessions), None);
        }
    }

    #[test]
    fn delete_refuses_the_model_of_a_transcribe_again_run_and_allows_the_old_model() {
        // A Transcribe Again run keeps the old `asr_model` until its save
        // and uses the run's model (KTD14).
        let settings = whisper_selected();
        let stopped_at = stopped();
        for state in [
            SessionState::Queued { stopped_at },
            SessionState::Processing { stopped_at },
        ] {
            let name = state.name();
            let sessions = [SessionRecord {
                transcribe_again_model: Some(TDT_V3.into()),
                ..note(PARAKEET, state)
            }];
            assert!(
                delete_refusal(TDT_V3, &settings, None, &sessions).is_some(),
                "{name}"
            );
            assert_eq!(
                delete_refusal(PARAKEET, &settings, None, &sessions),
                None,
                "{name}"
            );
        }
        // A saved note with a stale mark needs no model.
        let sessions = [SessionRecord {
            transcribe_again_model: Some(TDT_V3.into()),
            ..note(PARAKEET, SessionState::Saved)
        }];
        assert_eq!(delete_refusal(TDT_V3, &settings, None, &sessions), None);
    }

    #[test]
    fn delete_protects_needed_speaker_weights_and_allows_unused_weights() {
        let settings = whisper_selected();
        assert!(delete_refusal("pyannote-segmentation-3.0", &settings, None, &[]).is_some());
        assert!(delete_refusal("no-such-model", &settings, None, &[]).is_some());
        let mut without_speakers = settings;
        without_speakers.speaker_source = models::recognition::SpeakerSource::Off;
        assert!(
            delete_refusal(
                "pyannote-segmentation-3.0",
                &without_speakers,
                None,
                &[note(
                    PARAKEET,
                    SessionState::Queued {
                        stopped_at: stopped()
                    }
                )],
            )
            .is_some()
        );
        assert_eq!(
            delete_refusal("pyannote-segmentation-3.0", &without_speakers, None, &[]),
            None
        );
        assert_eq!(
            delete_refusal(
                "wespeaker-voxceleb-resnet34-lm",
                &without_speakers,
                None,
                &[]
            ),
            None
        );
    }

    /// The statuses for `settings`, with the models in `present` on disk.
    fn statuses_with(settings: &Settings, present: &[&str]) -> Vec<ModelStatus> {
        let dir = tempfile::tempdir().unwrap();
        let mut statuses = ModelsManager::at(dir.path().to_path_buf())
            .statuses_for(&settings.recognition_config())
            .unwrap();
        for status in &mut statuses {
            if present.contains(&status.id.as_str()) {
                status.state = ModelState::Present;
            }
        }
        statuses
    }

    const SORTFORMER: &str = "sortformer-v2-q80";
    const NEMOTRON_STREAMING: &str = "nemotron-speech-streaming-en-0.6b-q80";
    const NEMO_COPY: &str = "nemo-nemotron-speech-streaming-en-0.6b-q80";

    #[test]
    fn a_supporting_model_that_another_bundle_needs_stays() {
        // AE7: the bundle of Parakeet Unified keeps the speaker models.
        let settings = Settings {
            asr_model: PARAKEET.into(),
            ..Settings::default()
        };
        let statuses = statuses_with(
            &settings,
            &[PARAKEET, SORTFORMER, "pyannote-segmentation-3.0"],
        );
        assert!(unneeded_supporting_models(&statuses, &settings).is_empty());
    }

    #[test]
    fn a_supporting_model_that_only_a_deleted_bundle_needs_goes() {
        // AE8: Nemotron Speech Streaming is deleted, so its NeMo copy goes,
        // and the speaker models stay for Parakeet Unified.
        let settings = Settings {
            asr_model: PARAKEET.into(),
            ..Settings::default()
        };
        let statuses = statuses_with(&settings, &[PARAKEET, SORTFORMER, NEMO_COPY]);
        assert_eq!(
            unneeded_supporting_models(&statuses, &settings),
            [NEMO_COPY]
        );
        let kept = statuses_with(
            &settings,
            &[PARAKEET, NEMOTRON_STREAMING, SORTFORMER, NEMO_COPY],
        );
        assert!(unneeded_supporting_models(&kept, &settings).is_empty());
    }

    #[test]
    fn the_selected_and_pending_bundles_keep_their_files_without_the_model() {
        // The selected model is not on disk (a failed download), and a
        // pending model downloads: both bundles stay.
        let settings = Settings {
            asr_model: NEMOTRON_STREAMING.into(),
            pending_asr_model: Some(PARAKEET.into()),
            ..Settings::default()
        };
        let statuses = statuses_with(&settings, &[SORTFORMER, NEMO_COPY]);
        assert!(unneeded_supporting_models(&statuses, &settings).is_empty());
    }

    #[test]
    fn a_supporting_model_outside_every_kept_bundle_goes() {
        // AE9: Whisper does not use the NeMo copy.
        let settings = whisper_selected();
        let statuses = statuses_with(&settings, &[WHISPER, NEMO_COPY]);
        assert_eq!(
            unneeded_supporting_models(&statuses, &settings),
            [NEMO_COPY]
        );
    }

    #[test]
    fn a_transcription_model_reports_its_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().to_path_buf());
        let status = |id: &str| {
            manager
                .statuses(WHISPER)
                .into_iter()
                .find(|s| s.id == id)
                .unwrap()
        };
        let parakeet = status(PARAKEET);
        let bundle = models::recognition::bundle(PARAKEET);
        assert_eq!(
            parakeet.bundle,
            bundle.iter().map(|a| a.id.clone()).collect::<Vec<_>>()
        );
        assert_eq!(
            parakeet.bundle_size_bytes,
            bundle.iter().map(|a| a.size_bytes).sum::<u64>()
        );
        assert!(!parakeet.bundle_complete);
        let sortformer = status(SORTFORMER);
        assert_eq!(sortformer.bundle, [SORTFORMER]);
        assert!(!sortformer.bundle_complete);
    }

    #[test]
    fn delete_files_removes_the_model_and_its_partial() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().to_path_buf());
        let path = manager.file_path(WHISPER).unwrap();
        std::fs::write(&path, b"weights").unwrap();
        std::fs::write(partial_path(&path), b"part").unwrap();
        manager.delete_files(WHISPER).unwrap();
        assert!(!path.exists());
        assert!(!partial_path(&path).exists());
        // A second delete finds nothing and succeeds.
        manager.delete_files(WHISPER).unwrap();
        assert!(manager.delete_files("no-such-model").is_err());
    }

    #[test]
    fn delete_files_refuses_a_model_that_is_downloading() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().to_path_buf());
        let path = manager.file_path(WHISPER).unwrap();
        std::fs::write(partial_path(&path), b"part").unwrap();
        manager
            .active()
            .insert(WHISPER.into(), CancellationToken::new());
        assert!(manager.delete_files(WHISPER).is_err());
        assert!(partial_path(&path).exists());
    }

    #[test]
    fn statuses_list_the_five_recognition_models_with_their_catalog_fields() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().to_path_buf());
        let statuses = manager.statuses(PARAKEET);
        let asr: Vec<_> = statuses
            .iter()
            .filter(|s| s.role == ModelRole::Recognition)
            .collect();
        assert_eq!(asr.len(), 5);
        let parakeet = asr.iter().find(|s| s.id == PARAKEET).unwrap();
        assert!(parakeet.recommended && parakeet.streaming && parakeet.required);
        assert_eq!(parakeet.languages, ["en"]);
    }

    fn weight_status(id: &str, size_bytes: u64, state: ModelState) -> ModelStatus {
        let dir = tempfile::tempdir().unwrap();
        let mut status = ModelsManager::at(dir.path().to_path_buf())
            .statuses(WHISPER)
            .into_iter()
            .find(|s| s.id == id)
            .unwrap();
        status.size_bytes = size_bytes;
        status.state = state;
        status
    }

    const SEGMENTATION: &str = "pyannote-segmentation-3.0";
    const EMBEDDING: &str = "wespeaker-voxceleb-resnet34-lm";

    #[test]
    fn the_speaker_models_are_present_only_when_both_weights_are() {
        let both = [
            weight_status(SEGMENTATION, 6, ModelState::Present),
            weight_status(EMBEDDING, 26, ModelState::Present),
            weight_status(WHISPER, 1000, ModelState::Absent { partial_bytes: 0 }),
        ];
        assert_eq!(
            speaker_models(&both),
            SpeakerModelsStatus {
                ids: vec![SEGMENTATION.into(), EMBEDDING.into()],
                size_bytes: 32,
                state: ModelState::Present,
            }
        );

        let one = [
            weight_status(SEGMENTATION, 6, ModelState::Present),
            weight_status(EMBEDDING, 26, ModelState::Absent { partial_bytes: 4 }),
        ];
        let status = speaker_models(&one);
        assert_eq!(status.size_bytes, 32);
        assert_eq!(status.state, ModelState::Absent { partial_bytes: 10 });

        let downloading = [
            weight_status(SEGMENTATION, 6, ModelState::Absent { partial_bytes: 0 }),
            weight_status(EMBEDDING, 26, ModelState::Downloading),
        ];
        assert_eq!(speaker_models(&downloading).state, ModelState::Downloading);
    }
}
