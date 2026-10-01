//! Model weights on this machine: where they live, which are present, and
//! the background downloads that fetch them (KTD19, R28, R1c).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use models::download::{Event as DownloadProgress, partial_path};
use models::recognition::{ArtifactSpec, ExecutionSpec, RecognitionConfig};
use models::{CancellationToken, CatalogModel, Limits, ModelKind, Outcome};
use serde::Serialize;
use session_core::{SessionRecord, SessionState};
use specta::Type;
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use crate::managers::settings::{self, RecognitionChoice, RecognitionFailure, Settings};

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

/// The download key of an artifact: its id and its SHA-256, so that two
/// snapshots of one id download apart.
fn download_key(artifact: &ArtifactSpec) -> String {
    format!("{}@{}", artifact.id, artifact.sha256)
}

/// The download keys of the artifacts that `config` needs. Empty when the
/// configuration does not resolve.
fn needed_keys(config: Option<&RecognitionConfig>) -> HashSet<String> {
    config
        .and_then(|c| models::recognition::resolve(c).ok())
        .map(|spec| spec.artifacts.iter().map(download_key).collect())
        .unwrap_or_default()
}

/// What one comparison of the chosen recognition configuration with the
/// configuration in use does (R18, KTD14).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChoicePlan {
    /// The files to download now.
    pub download: Vec<ArtifactSpec>,
    /// The keys of the downloads that the step started and that neither
    /// configuration needs. These downloads stop.
    pub stop: Vec<String>,
    /// Every file of the chosen configuration is on disk: this becomes the
    /// configuration in use.
    pub apply: Option<RecognitionChoice>,
    /// The files that the chosen configuration waits for.
    pub waiting: Vec<ArtifactSpec>,
    /// Why the chosen configuration cannot apply now.
    pub failure: Option<RecognitionFailure>,
}

/// Compare the chosen configuration in `settings` with the configuration in
/// use (R18, KTD14). `missing` gives the files of a resolved configuration
/// that are not on disk. `started` holds the keys of the downloads that
/// earlier steps started. `failure` is a failed download of this chosen
/// configuration: while it is set, no download of it starts again.
///
/// A chosen configuration with a value that this version does not know
/// keeps the configuration in use and starts no download.
pub fn plan_choice(
    settings: &Settings,
    missing: impl Fn(&ExecutionSpec) -> Vec<ArtifactSpec>,
    started: &HashSet<String>,
    failure: Option<&RecognitionFailure>,
) -> ChoicePlan {
    let mut kept = needed_keys(Some(&settings.recognition_config()));
    let mut plan = ChoicePlan::default();
    if let Some(config) = settings.chosen_config()
        && settings.chosen_recognition != settings.in_use()
    {
        match models::recognition::resolve(&config) {
            Err(e) => {
                plan.failure = Some(RecognitionFailure {
                    model_id: config.asr_model.clone(),
                    message: e.to_string(),
                });
            }
            Ok(spec) => {
                kept.extend(spec.artifacts.iter().map(download_key));
                let missing = missing(&spec);
                if missing.is_empty() {
                    plan.apply = Some(settings.chosen_recognition.clone());
                } else {
                    match failure {
                        Some(failure) => plan.failure = Some(failure.clone()),
                        None => plan.download = missing.clone(),
                    }
                    plan.waiting = missing;
                }
            }
        }
    }
    plan.stop = started
        .iter()
        .filter(|key| !kept.contains(*key))
        .cloned()
        .collect();
    plan.stop.sort();
    plan
}

/// The state of the downloads of the chosen configuration.
#[derive(Debug, Default)]
struct ChoiceState {
    /// The chosen configuration of the last step.
    chosen: Option<RecognitionChoice>,
    /// The files that `chosen` waits for.
    waiting: Vec<ArtifactSpec>,
    /// The keys of the downloads that a step started and that did not
    /// finish.
    started: HashSet<String>,
    /// The keys of the downloads that a step stopped and that did not end
    /// yet.
    stopped: HashSet<String>,
    /// A failed download of `chosen`. It stays until Retry or until the
    /// chosen configuration changes.
    failure: Option<RecognitionFailure>,
}

/// The text of a download of the chosen configuration that the user
/// stopped.
const DOWNLOAD_STOPPED: &str = "The download was stopped.";

/// Why the app cannot delete the model `id` now, or `None` when it can
/// (R6, KTD12). `live_model` is the model of the live worker. The models of
/// the configuration in use and of the chosen configuration stay (R18).
pub fn delete_refusal(
    id: &str,
    settings: &Settings,
    live_model: Option<&str>,
    sessions: &[SessionRecord],
) -> Option<String> {
    let Some(catalog_model) = models::model(id) else {
        return Some(format!("no model named '{id}'"));
    };
    let needs = |config: Option<RecognitionConfig>| {
        config
            .and_then(|c| models::recognition::resolve(&c).ok())
            .is_some_and(|spec| spec.artifacts.iter().any(|artifact| artifact.id == id))
    };
    if settings.asr_model == id || needs(Some(settings.recognition_config())) {
        return Some("This model is selected. Select another model first.".into());
    }
    if settings.chosen_recognition.asr_model == id || needs(settings.chosen_config()) {
        return Some("The chosen transcription settings need this model.".into());
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
/// model in use, and of the chosen model (R18). The caller also applies
/// [`delete_refusal`] to each result, so that unfinished notes and the live
/// model keep their files.
pub fn unneeded_supporting_models(statuses: &[ModelStatus], settings: &Settings) -> Vec<String> {
    let mut kept_models: Vec<&str> = statuses
        .iter()
        .filter(|s| s.role == ModelRole::Recognition && s.state == ModelState::Present)
        .map(|s| s.id.as_str())
        .collect();
    kept_models.push(&settings.asr_model);
    kept_models.push(&settings.chosen_recognition.asr_model);
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
    /// The downloads of the chosen recognition configuration (R18).
    choice: Mutex<ChoiceState>,
    /// One comparison of the chosen configuration at a time.
    follow_lock: Mutex<()>,
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
            choice: Mutex::new(ChoiceState::default()),
            follow_lock: Mutex::new(()),
        }
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, CancellationToken>> {
        self.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn choice(&self) -> std::sync::MutexGuard<'_, ChoiceState> {
        self.choice.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The files of `spec` that are not on disk.
    fn missing_artifacts(&self, spec: &ExecutionSpec) -> Vec<ArtifactSpec> {
        spec.artifacts
            .iter()
            .filter(|a| self.artifact_path(a).is_none())
            .cloned()
            .collect()
    }

    /// Compare the chosen configuration in `settings` with the
    /// configuration in use, and keep what the chosen configuration waits
    /// for. A failed download stays failed for the same chosen
    /// configuration. Returns the plan, and whether the files that it waits
    /// for or the failure changed.
    fn plan_choice_now(&self, settings: &Settings) -> (ChoicePlan, bool) {
        let mut state = self.choice();
        if state.chosen.as_ref() != Some(&settings.chosen_recognition) {
            state.failure = None;
        }
        let plan = plan_choice(
            settings,
            |spec| self.missing_artifacts(spec),
            &state.started,
            state.failure.as_ref(),
        );
        let changed = state.waiting != plan.waiting || state.failure != plan.failure;
        state.chosen = Some(settings.chosen_recognition.clone());
        state.waiting = plan.waiting.clone();
        state.failure = plan.failure.clone();
        (plan, changed)
    }

    /// Keep the outcome of the download `key`. A download of a file that the
    /// chosen configuration waits for that fails or stops makes the
    /// failure. A download that a step stopped is not a failure. Returns
    /// whether the failure changed.
    fn note_outcome(&self, key: &str, artifact_id: &str, update: &DownloadUpdate) -> bool {
        let mut state = self.choice();
        state.started.remove(key);
        let stopped_by_step = state.stopped.remove(key);
        let message = match update {
            DownloadUpdate::Failed { message } => message.clone(),
            DownloadUpdate::Cancelled if !stopped_by_step => DOWNLOAD_STOPPED.to_string(),
            _ => return false,
        };
        if !state.waiting.iter().any(|a| download_key(a) == key) {
            return false;
        }
        state.failure = Some(RecognitionFailure {
            model_id: artifact_id.to_string(),
            message,
        });
        true
    }

    /// Forget the failed download, so that the next step starts it again
    /// (R19).
    pub fn retry_choice(&self) {
        self.choice().failure = None;
    }

    /// Put the downloads that the chosen configuration in `settings` waits
    /// for, and the failure, into `settings` (R19).
    pub fn report_choice(&self, settings: &mut Settings) {
        let state = self.choice();
        if settings.chosen_recognition == settings.in_use()
            || state.chosen.as_ref() != Some(&settings.chosen_recognition)
        {
            return;
        }
        settings.recognition.waiting_for = state.waiting.iter().map(|a| a.id.clone()).collect();
        settings.recognition.failure = state.failure.clone();
    }

    /// Compare the chosen recognition configuration with the configuration
    /// in use (R18, KTD14). This starts each download that the chosen
    /// configuration needs, stops each download that an earlier step
    /// started and that neither configuration needs, and makes the chosen
    /// configuration the configuration in use when every file is on disk.
    /// Then the surface gets `SettingsChanged`. Run it at launch, after
    /// each change of the shared settings, and after each download.
    pub fn follow_choice(self: &Arc<Self>, app: &AppHandle) {
        let _one_at_a_time = self.follow_lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut changed = false;
        // A second pass stops the downloads of the configuration that was
        // in use before an apply.
        for _ in 0..2 {
            let settings = settings::get_settings(app);
            let (plan, plan_changed) = self.plan_choice_now(&settings);
            changed |= plan_changed;
            for key in &plan.stop {
                log::info!("recognition: stop the download {key}, which no configuration needs");
                {
                    let mut state = self.choice();
                    state.started.remove(key);
                    state.stopped.insert(key.clone());
                }
                self.cancel_key(key);
            }
            if !plan.download.is_empty() {
                match self.start_artifact_downloads(app, &plan.download) {
                    Ok(keys) => self.choice().started.extend(keys),
                    Err(e) => log::error!("could not start the chosen model downloads: {e}"),
                }
            }
            let Some(choice) = plan.apply else {
                break;
            };
            match settings::update_settings(app, |s| {
                s.set_in_use(&choice);
                Ok(())
            }) {
                Ok(_) => {
                    log::info!(
                        "recognition: the chosen configuration is in use ({})",
                        choice.asr_model
                    );
                    changed = true;
                }
                Err(e) => {
                    log::error!("could not use the chosen recognition configuration: {e}");
                    break;
                }
            }
        }
        if changed {
            settings::emit_settings_changed(app);
        }
    }

    /// Stop the download with the key `key`, if it runs.
    fn cancel_key(&self, key: &str) {
        if let Some(token) = self.active().get(key) {
            token.cancel();
        }
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
        self.start_artifact_downloads(app, &artifacts).map(drop)
    }

    /// Download each of `artifacts` that is not on disk and not
    /// downloading, one after another. Returns the keys of the downloads
    /// that this call started.
    fn start_artifact_downloads(
        self: &Arc<Self>,
        app: &AppHandle,
        artifacts: &[models::recognition::ArtifactSpec],
    ) -> Result<Vec<String>, String> {
        for artifact in artifacts {
            models::download::pinned_artifact_path(&self.dir, artifact)
                .map_err(|e| e.to_string())?;
        }
        let mut wanted = Vec::new();
        {
            let mut active = self.active();
            for artifact in artifacts {
                let key = download_key(artifact);
                if self.artifact_path(artifact).is_none() && !active.contains_key(&key) {
                    let token = CancellationToken::new();
                    active.insert(key.clone(), token.clone());
                    wanted.push((key, artifact.clone(), token));
                }
            }
        }
        let keys = wanted.iter().map(|(key, _, _)| key.clone()).collect();
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
                // The settings change before the download event, so a
                // surface that reads the settings on the event sees the
                // new configuration in use.
                if manager.note_outcome(&key, &artifact.id, &update) {
                    log::warn!(
                        "recognition: a download of the chosen configuration did not finish: {}",
                        artifact.id
                    );
                    settings::emit_settings_changed(&app);
                }
                match update {
                    DownloadUpdate::Completed => {
                        manager.follow_choice(&app);
                        // A note that waits for this file can be processed
                        // now.
                        if let Some(processing) =
                            app.try_state::<Arc<crate::managers::processing::ProcessingManager>>()
                        {
                            processing.wake();
                        }
                    }
                    // The chosen configuration can need a stopped file
                    // again. Then its download starts again.
                    DownloadUpdate::Cancelled => manager.follow_choice(&app),
                    _ => {}
                }
                emit(&app, &artifact.id, update);
            }
        });
        Ok(keys)
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
        self.start_artifact_downloads(app, &artifacts).map(drop)
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

#[cfg(test)]
mod tests {
    use chrono::{DateTime, FixedOffset};

    use super::*;

    const WHISPER: &str = "whisper-large-v3-turbo-q5km";
    const PARAKEET: &str = "parakeet-unified-en-0.6b-q80";
    const TDT_V3: &str = "parakeet-tdt-0.6b-v3-q80";
    const NEMOTRON_3_5: &str = "nemotron-3.5-asr-streaming-0.6b-q80";

    /// Settings with `choice` chosen and in use.
    fn using(choice: RecognitionChoice) -> Settings {
        let mut settings = Settings {
            chosen_recognition: choice.clone(),
            ..Settings::default()
        };
        settings.set_in_use(&choice);
        settings
    }

    fn model_choice(asr_model: &str) -> RecognitionChoice {
        RecognitionChoice {
            asr_model: asr_model.into(),
            ..RecognitionChoice::default()
        }
    }

    fn whisper_selected() -> Settings {
        using(model_choice(WHISPER))
    }

    /// Whisper with pyannote is in use, and Parakeet TDT v3 with Sortformer
    /// is chosen.
    fn tdt_with_sortformer_chosen() -> Settings {
        let mut settings = whisper_selected();
        settings.chosen_recognition = RecognitionChoice {
            asr_model: TDT_V3.into(),
            live_transcript: false,
            speaker_source: models::recognition::SpeakerSource::Sortformer,
            ..RecognitionChoice::default()
        };
        settings
    }

    fn ids(artifacts: &[ArtifactSpec]) -> Vec<&str> {
        artifacts.iter().map(|a| a.id.as_str()).collect()
    }

    /// Every file of a resolved configuration is missing.
    fn nothing_on_disk(spec: &ExecutionSpec) -> Vec<ArtifactSpec> {
        spec.artifacts.clone()
    }

    fn key_of(id: &str) -> String {
        let m = models::model(id).unwrap();
        format!("{}@{}", m.id, m.sha256)
    }

    #[test]
    fn a_chosen_model_and_speaker_source_that_are_not_on_disk_start_both_downloads() {
        // AE5: the configuration in use stays until both are verified.
        let settings = tdt_with_sortformer_chosen();
        let plan = plan_choice(&settings, nothing_on_disk, &HashSet::new(), None);
        assert_eq!(ids(&plan.download), [TDT_V3, SORTFORMER]);
        assert_eq!(ids(&plan.waiting), [TDT_V3, SORTFORMER]);
        assert_eq!(plan.apply, None);
        assert_eq!(plan.failure, None);
        assert!(plan.stop.is_empty());
        assert_eq!(settings.recognition_config().asr_model, WHISPER);

        // One file is on disk: only the other one downloads.
        let plan = plan_choice(
            &settings,
            |spec| {
                spec.artifacts
                    .iter()
                    .filter(|a| a.id != TDT_V3)
                    .cloned()
                    .collect()
            },
            &HashSet::new(),
            None,
        );
        assert_eq!(ids(&plan.download), [SORTFORMER]);
        assert_eq!(plan.apply, None);
    }

    #[test]
    fn when_the_last_file_is_on_disk_the_chosen_configuration_is_in_use() {
        let settings = tdt_with_sortformer_chosen();
        let plan = plan_choice(&settings, |_| Vec::new(), &HashSet::new(), None);
        assert_eq!(plan.apply.as_ref(), Some(&settings.chosen_recognition));
        assert!(plan.download.is_empty() && plan.waiting.is_empty());

        // The apply changes the six values as one unit.
        let mut after = settings.clone();
        after.set_in_use(&plan.apply.unwrap());
        assert_eq!(after.in_use(), settings.chosen_recognition);
        assert!(!after.recognition.differs);
        assert!(settings::shared_patch(&settings, &after).is_empty());
    }

    #[test]
    fn a_launch_with_the_chosen_configuration_in_use_starts_nothing() {
        // Nothing is on disk, as on a new install: the step starts no
        // download, and the Settings window downloads the models.
        let plan = plan_choice(&Settings::default(), nothing_on_disk, &HashSet::new(), None);
        assert_eq!(plan, ChoicePlan::default());
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().into());
        let (plan, changed) = manager.plan_choice_now(&Settings::default());
        assert_eq!(plan, ChoicePlan::default());
        assert!(!changed);
    }

    #[test]
    fn a_failed_download_keeps_the_configuration_in_use_until_retry() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().into());
        let settings = tdt_with_sortformer_chosen();
        let (plan, changed) = manager.plan_choice_now(&settings);
        assert!(changed);
        assert_eq!(ids(&plan.download), [TDT_V3, SORTFORMER]);
        manager
            .choice()
            .started
            .extend(plan.download.iter().map(download_key));

        let failed = DownloadUpdate::Failed {
            message: "checksum mismatch".into(),
        };
        assert!(manager.note_outcome(&key_of(TDT_V3), TDT_V3, &failed));
        let mut reported = settings.clone();
        manager.report_choice(&mut reported);
        assert_eq!(
            reported.recognition.failure,
            Some(RecognitionFailure {
                model_id: TDT_V3.into(),
                message: "checksum mismatch".into()
            })
        );
        assert_eq!(reported.recognition.waiting_for, [TDT_V3, SORTFORMER]);
        assert_eq!(reported.asr_model, WHISPER);

        // A later step, for example after a file event from another
        // computer, starts nothing and keeps the failure.
        let (plan, changed) = manager.plan_choice_now(&settings);
        assert!(!changed);
        assert!(plan.download.is_empty());
        assert_eq!(plan.apply, None);
        assert_eq!(plan.failure.unwrap().model_id, TDT_V3);

        // Retry starts the downloads again.
        manager.retry_choice();
        let (plan, _) = manager.plan_choice_now(&settings);
        assert_eq!(ids(&plan.download), [TDT_V3, SORTFORMER]);
        assert_eq!(plan.failure, None);
    }

    #[test]
    fn a_stopped_download_of_the_chosen_configuration_is_a_failure_and_another_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().into());
        manager.plan_choice_now(&tdt_with_sortformer_chosen());
        // A file that the chosen configuration does not wait for.
        assert!(!manager.note_outcome(&key_of(WHISPER), WHISPER, &DownloadUpdate::Cancelled));
        assert!(!manager.note_outcome(&key_of(TDT_V3), TDT_V3, &DownloadUpdate::Completed));
        // A download that a step stopped is not a failure, also when the
        // chosen configuration needs the file again.
        manager.choice().stopped.insert(key_of(TDT_V3));
        assert!(!manager.note_outcome(&key_of(TDT_V3), TDT_V3, &DownloadUpdate::Cancelled));
        assert_eq!(manager.choice().failure, None);
        assert!(manager.note_outcome(&key_of(TDT_V3), TDT_V3, &DownloadUpdate::Cancelled));
        assert_eq!(
            manager.choice().failure.as_ref().unwrap().message,
            DOWNLOAD_STOPPED
        );
    }

    #[test]
    fn a_failure_goes_when_the_chosen_configuration_changes() {
        let dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(dir.path().into());
        let settings = tdt_with_sortformer_chosen();
        manager.plan_choice_now(&settings);
        manager.note_outcome(
            &key_of(TDT_V3),
            TDT_V3,
            &DownloadUpdate::Failed {
                message: "offline".into(),
            },
        );
        let mut other = settings.clone();
        other.chosen_recognition.asr_model = NEMOTRON_3_5.into();
        let (plan, _) = manager.plan_choice_now(&other);
        assert_eq!(plan.failure, None);
        assert_eq!(ids(&plan.download), [NEMOTRON_3_5, SORTFORMER]);
    }

    #[test]
    fn a_chosen_model_that_this_version_does_not_know_starts_no_download() {
        use shared_settings::view::{self, Availability};
        let mut raw = shared_settings::file::default_settings();
        raw.insert(
            "asr_model".into(),
            serde_json::json!("model-from-the-future"),
        );
        raw.insert("speaker_source".into(), serde_json::json!("sortformer"));
        let cache = shared_settings::cache::Cache {
            source: PathBuf::from("/vault"),
            settings: raw,
            prompts: Vec::new(),
            active_prompt: None,
        };
        let shared = crate::managers::shared::SharedSnapshot {
            view: view::cached(Some(&cache), Availability::Available),
            ..Default::default()
        };
        let local = settings::LocalSettings {
            asr_model: WHISPER.into(),
            ..Default::default()
        };
        let settings = Settings::from_parts(local, &shared);
        assert!(settings.recognition.chosen_not_known);
        // The known values differ, and still the configuration in use stays.
        assert!(settings.recognition.differs);
        let plan = plan_choice(&settings, nothing_on_disk, &HashSet::new(), None);
        assert_eq!(plan, ChoicePlan::default());
    }

    #[test]
    fn a_change_of_choice_stops_a_download_that_no_configuration_needs() {
        // TDT v3 and Sortformer download for the first choice. Then the
        // user chooses Nemotron 3.5 with Sortformer: TDT v3 stops, and
        // Sortformer continues.
        let mut settings = tdt_with_sortformer_chosen();
        let started: HashSet<String> = [key_of(TDT_V3), key_of(SORTFORMER)].into();
        settings.chosen_recognition.asr_model = NEMOTRON_3_5.into();
        let plan = plan_choice(&settings, nothing_on_disk, &started, None);
        assert_eq!(plan.stop, [key_of(TDT_V3)]);
        assert_eq!(ids(&plan.download), [NEMOTRON_3_5, SORTFORMER]);

        // A download of a file of the configuration in use continues.
        let started: HashSet<String> = [key_of(WHISPER), key_of(TDT_V3)].into();
        let plan = plan_choice(&whisper_selected(), nothing_on_disk, &started, None);
        assert_eq!(plan.stop, [key_of(TDT_V3)]);
    }

    /// A change from another computer arrives through the shared settings
    /// manager's read (U5), and the step then starts the downloads.
    #[test]
    fn a_change_from_another_computer_starts_the_downloads() {
        use shared_settings::file::SharedValue;
        let folder = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let shared = crate::managers::shared::SharedSettingsManager::new(
            cache.path().into(),
            Some(folder.path().into()),
        );
        shared.join_at_launch();
        let models_dir = tempfile::tempdir().unwrap();
        let manager = ModelsManager::at(models_dir.path().into());
        let settings = Settings::from_parts(settings::LocalSettings::default(), &shared.snapshot());
        assert_eq!(manager.plan_choice_now(&settings).0, ChoicePlan::default());

        // Computer A chooses Parakeet TDT v3 with Sortformer.
        shared_settings::file::save_settings(
            folder.path(),
            &[
                SharedValue::AsrModel(TDT_V3.into()),
                SharedValue::SpeakerSource(models::recognition::SpeakerSource::Sortformer),
            ],
        )
        .unwrap();
        assert!(shared.reread());
        let settings = Settings::from_parts(settings::LocalSettings::default(), &shared.snapshot());
        assert_eq!(settings.chosen_recognition.asr_model, TDT_V3);
        assert_eq!(settings.asr_model, models::DEFAULT_ASR_MODEL);
        let (plan, changed) = manager.plan_choice_now(&settings);
        assert!(changed);
        assert_eq!(ids(&plan.download), [TDT_V3, SORTFORMER]);
        assert_eq!(plan.apply, None);
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
            started_at: None,
        }
    }

    fn stopped() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-09-25T10:00:00+00:00").unwrap()
    }

    #[test]
    fn delete_refuses_the_selected_model_and_allows_another() {
        // AE8.
        let settings = using(model_choice(PARAKEET));
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
        let settings = using(RecognitionChoice {
            asr_model: "nemotron-speech-streaming-en-0.6b-q80".into(),
            live_transcript: true,
            speaker_source: models::recognition::SpeakerSource::Nemotron,
            ..RecognitionChoice::default()
        });
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
    fn delete_refuses_the_chosen_and_the_live_model() {
        let settings = tdt_with_sortformer_chosen();
        assert_eq!(
            delete_refusal(TDT_V3, &settings, None, &[]).as_deref(),
            Some("The chosen transcription settings need this model.")
        );
        // A speaker model of the chosen configuration stays too.
        assert!(delete_refusal(SORTFORMER, &settings, None, &[]).is_some());
        // The configuration in use keeps its models.
        assert!(delete_refusal(WHISPER, &settings, None, &[]).is_some());
        assert!(delete_refusal("pyannote-segmentation-3.0", &settings, None, &[]).is_some());
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
        let without_speakers = using(RecognitionChoice {
            speaker_source: models::recognition::SpeakerSource::Off,
            ..model_choice(WHISPER)
        });
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
    fn the_bundles_in_use_and_chosen_keep_their_files_without_the_model() {
        // The model in use is not on disk (a failed download), and the
        // chosen model downloads: both bundles stay.
        let mut settings = using(model_choice(NEMOTRON_STREAMING));
        settings.chosen_recognition = model_choice(PARAKEET);
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
