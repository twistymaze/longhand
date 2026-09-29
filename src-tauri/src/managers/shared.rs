//! The shared settings manager (KTD7, KTD8): the shared settings and the
//! custom summary prompts of the storage folder, as this computer uses them
//! now.
//!
//! The manager keeps the last shared view in memory, so a read of the
//! settings does no file I/O. Each change reads the files again, writes
//! only its own values, and then reads the result (R15). The manager writes
//! into the storage folder only for a change that this computer makes, and
//! for the join at launch or after a change of folder (R17, R21).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use shared_settings::cache::{self, Cache};
use shared_settings::file::{self, Field, FileRead, SaveError, SharedValue, Unavailable};
use shared_settings::join::{self, JoinKind};
use shared_settings::prompts::{self, PromptError};
use shared_settings::view::{self, Availability, SharedView};
use specta::Type;
use summarize::prompts::TOPIC_SUMMARY_ID;

/// The name of a prompt that the user adds (R3).
const NEW_PROMPT_NAME: &str = "New prompt";
const NO_SUCH_PROMPT: &str = "That prompt no longer exists.";

/// Whether this computer can change the shared settings now (R16).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SharedAvailability {
    Available,
    /// The shared settings come from the local cache, or from the defaults
    /// when there is no cache.
    ReadOnly {
        cause: ReadOnlyCause,
    },
}

/// Why the shared settings are read-only.
#[derive(Serialize, Type, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReadOnlyCause {
    /// No storage folder is set.
    NoStorageFolder,
    /// Longhand did not read the storage folder since launch.
    NotReadYet,
    /// The storage folder does not exist, for example because its disk is
    /// not connected.
    FolderNotAvailable,
    /// The storage folder or its settings file cannot be read.
    NotReadable,
    /// The settings file is not one complete JSON object, for example
    /// because a sync delivered only a part of it (KTD3).
    DoesNotParse,
    /// The storage folder has no settings file, and Longhand did not write
    /// one into it.
    NoSettingsFile,
}

impl ReadOnlyCause {
    /// The UI text of a change that the cause refuses.
    pub fn message(self) -> &'static str {
        match self {
            Self::NoStorageFolder => "Choose a storage folder to change this setting.",
            Self::NotReadYet => "Longhand is still reading the storage folder. Try again.",
            Self::FolderNotAvailable => {
                "The storage folder is not available, so this setting can't be changed now."
            }
            Self::NotReadable => {
                "Longhand can't read the shared settings in the storage folder, so this setting can't be changed now."
            }
            Self::DoesNotParse => {
                "The shared settings file in the storage folder is not complete. Try again after the sync ends."
            }
            Self::NoSettingsFile => "The storage folder has no shared settings yet.",
        }
    }
}

impl From<&Unavailable> for ReadOnlyCause {
    fn from(cause: &Unavailable) -> Self {
        match cause {
            Unavailable::FolderMissing => Self::FolderNotAvailable,
            Unavailable::Unreadable(_) => Self::NotReadable,
            Unavailable::DoesNotParse(_) => Self::DoesNotParse,
        }
    }
}

/// A value in the shared settings file that this version does not know.
/// Longhand keeps it in the file (R17).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct UnknownSharedValue {
    pub key: String,
    /// The value as JSON text.
    pub value: String,
}

/// The state of the shared settings, for the Settings window.
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct SharedState {
    pub availability: SharedAvailability,
    /// The settings file comes from a newer version of Longhand, so it can
    /// hold settings that this version does not show.
    pub newer_version: bool,
    pub unknown_values: Vec<UnknownSharedValue>,
}

impl Default for SharedState {
    fn default() -> Self {
        SharedSnapshot::default().state()
    }
}

/// The shared view and whether this computer can change it.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedSnapshot {
    pub view: SharedView,
    pub availability: SharedAvailability,
}

impl Default for SharedSnapshot {
    /// The defaults, before the manager reads anything.
    fn default() -> Self {
        Self {
            view: view::cached(None, Availability::MissingFile),
            availability: SharedAvailability::ReadOnly {
                cause: ReadOnlyCause::NotReadYet,
            },
        }
    }
}

impl SharedSnapshot {
    pub fn state(&self) -> SharedState {
        let fields = &self.view.fields;
        let mut unknown_values = Vec::new();
        let mut note = |key: &str, value: Option<&serde_json::Value>| {
            if let Some(value) = value {
                unknown_values.push(UnknownSharedValue {
                    key: key.to_string(),
                    value: value.to_string(),
                });
            }
        };
        note(
            file::KEY_ACTIVE_SUMMARY_PROMPT,
            unknown(&fields.active_summary_prompt),
        );
        note(
            file::KEY_DEFAULT_SPEAKER_COUNT,
            unknown(&fields.default_speaker_count),
        );
        note(file::KEY_ASR_MODEL, unknown(&fields.asr_model));
        note(file::KEY_LIVE_TRANSCRIPT, unknown(&fields.live_transcript));
        note(file::KEY_SPEAKER_SOURCE, unknown(&fields.speaker_source));
        note(
            file::KEY_SPEAKER_PROCESSING_MODE,
            unknown(&fields.speaker_processing_mode),
        );
        note(
            file::KEY_RECOGNITION_LANGUAGE,
            unknown(&fields.recognition_language),
        );
        note(
            file::KEY_CHECK_UPDATES_AUTOMATICALLY,
            unknown(&fields.check_updates_automatically),
        );
        SharedState {
            availability: self.availability.clone(),
            newer_version: fields.newer_schema(),
            unknown_values,
        }
    }
}

fn unknown<T>(field: &Field<T>) -> Option<&serde_json::Value> {
    match field {
        Field::Unknown(value) => Some(value),
        Field::Absent | Field::Known(_) => None,
    }
}

/// Put `legacy`, the shared values of an old local store, into the cache
/// in `cache_dir` when no cache exists (R21). Returns whether it wrote the
/// cache.
pub fn seed_cache(
    cache_dir: &Path,
    legacy: crate::managers::settings::LegacyShared,
) -> io::Result<bool> {
    if cache::read_cache(cache_dir).is_some() {
        return Ok(false);
    }
    cache::write_cache(cache_dir, &legacy.into_cache())?;
    Ok(true)
}

/// Why a change of the shared files failed.
enum Failure {
    /// The folder has no settings file. A join fills it (KTD8).
    Missing,
    NotAvailable(Unavailable),
    Refused(String),
}

impl Failure {
    fn message(self) -> String {
        match self {
            Failure::Missing => ReadOnlyCause::NoSettingsFile.message().to_string(),
            Failure::NotAvailable(cause) => ReadOnlyCause::from(&cause).message().to_string(),
            Failure::Refused(message) => message,
        }
    }
}

impl From<SaveError> for Failure {
    fn from(error: SaveError) -> Self {
        match error {
            SaveError::Missing => Failure::Missing,
            SaveError::NotAvailable(cause) => Failure::NotAvailable(cause),
            SaveError::Io(e) => {
                Failure::Refused(format!("The shared settings can't be saved: {e}"))
            }
        }
    }
}

impl From<PromptError> for Failure {
    fn from(error: PromptError) -> Self {
        match error {
            PromptError::NoSettingsFile => Failure::Missing,
            PromptError::NotAvailable(cause) => Failure::NotAvailable(cause),
            other => Failure::Refused(other.to_string()),
        }
    }
}

struct Inner {
    /// The storage folder. `None` while no storage folder is set.
    root: Option<PathBuf>,
    /// A join read the storage folder since launch or since the last
    /// change of folder.
    read: bool,
    view: SharedView,
}

/// The shared settings of the storage folder (KTD7).
pub struct SharedSettingsManager {
    /// The folder of the local cache (KTD6).
    cache_dir: PathBuf,
    inner: Mutex<Inner>,
}

impl SharedSettingsManager {
    /// A manager for the storage folder `root`. Until the first join, the
    /// view comes from the cache and is read-only. This reads only the
    /// local cache.
    pub fn new(cache_dir: PathBuf, root: Option<PathBuf>) -> Self {
        let view = view::cached(
            cache::read_cache(&cache_dir).as_ref(),
            Availability::MissingFile,
        );
        Self {
            cache_dir,
            inner: Mutex::new(Inner {
                root,
                read: false,
                view,
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The shared view that this computer uses now. This does no file I/O.
    pub fn snapshot(&self) -> SharedSnapshot {
        let inner = self.lock();
        snapshot_of(&inner)
    }

    /// The join at launch (R21, KTD8): read the storage folder, and fill it
    /// when it has no settings file. Call it off the main thread, before
    /// the first library scan. Returns whether the view changed.
    pub fn join_at_launch(&self) -> bool {
        let mut inner = self.lock();
        self.join_locked(&mut inner, None)
    }

    /// The storage folder changed to `root` (R21). The new folder's shared
    /// settings apply, and this computer's custom prompts merge into them
    /// (R22). With no folder, the view comes from the cache and is
    /// read-only. Returns whether the view changed.
    pub fn change_folder(&self, root: Option<PathBuf>) -> bool {
        let mut inner = self.lock();
        let before = snapshot_of(&inner);
        inner.root = root;
        inner.read = false;
        if inner.root.is_some() {
            self.join_locked(&mut inner, Some(JoinKind::ChangeOfFolder));
        } else {
            inner.view = view::cached(
                cache::read_cache(&self.cache_dir).as_ref(),
                Availability::MissingFile,
            );
        }
        snapshot_of(&inner) != before
    }

    /// Save the values of `patch` into the shared settings file (R15).
    pub fn save(&self, patch: &[SharedValue]) -> Result<SharedSnapshot, String> {
        self.change(|root| {
            file::save_settings(root, patch)
                .map(drop)
                .map_err(Failure::from)
        })?;
        Ok(self.snapshot())
    }

    /// Add a prompt named "New prompt", or a free name made from it, with
    /// the Topic summary text (R3). Returns its id.
    pub fn create_prompt(&self) -> Result<String, String> {
        let text = topic_summary().text;
        self.change(|root| {
            prompts::create_prompt(root, NEW_PROMPT_NAME, &text)
                .map(|p| p.id)
                .map_err(Failure::from)
        })
    }

    /// Add a copy of the prompt `id`, named "X copy", or a free name made
    /// from it (R3). Returns the id of the copy.
    pub fn duplicate_prompt(&self, id: &str) -> Result<String, String> {
        self.change(|root| {
            let source = find_prompt(root, id).ok_or(Failure::Refused(NO_SUCH_PROMPT.into()))?;
            prompts::create_prompt(root, &format!("{} copy", source.name), &source.text)
                .map(|p| p.id)
                .map_err(Failure::from)
        })
    }

    pub fn rename_prompt(&self, id: &str, name: &str) -> Result<(), String> {
        self.change(|root| {
            prompts::rename_prompt(root, id, name)
                .map(drop)
                .map_err(Failure::from)
        })
    }

    pub fn set_prompt_text(&self, id: &str, text: &str) -> Result<(), String> {
        self.change(|root| {
            prompts::set_prompt_text(root, id, text)
                .map(drop)
                .map_err(Failure::from)
        })
    }

    /// Remove a custom prompt. When it was active, the Topic summary becomes
    /// active in the same change (R4).
    pub fn delete_prompt(&self, id: &str) -> Result<(), String> {
        self.change(|root| {
            prompts::delete_prompt(root, id)?;
            let active = match file::read_settings(root) {
                FileRead::Parsed(settings) => settings.fields.active_summary_prompt,
                FileRead::Missing => return Err(Failure::Missing),
                FileRead::NotAvailable(cause) => return Err(Failure::NotAvailable(cause)),
            };
            if active.known().is_some_and(|active| active == id) {
                file::save_settings(
                    root,
                    &[SharedValue::ActiveSummaryPrompt(TOPIC_SUMMARY_ID.into())],
                )?;
            }
            Ok(())
        })
    }

    /// Make a prompt the one that the next summary uses (R1).
    pub fn activate_prompt(&self, id: &str) -> Result<(), String> {
        self.change(|root| {
            if find_prompt(root, id).is_none() {
                return Err(Failure::Refused(NO_SUCH_PROMPT.into()));
            }
            file::save_settings(root, &[SharedValue::ActiveSummaryPrompt(id.into())])?;
            Ok(())
        })
    }

    /// Run `op` on the storage folder, then read the result into the view.
    /// A view that is not writable is joined again first, so a folder that
    /// came back is used. A folder with no settings file gets one from the
    /// join, and `op` runs again (KTD8).
    fn change<T>(&self, op: impl Fn(&Path) -> Result<T, Failure>) -> Result<T, String> {
        let mut inner = self.lock();
        let Some(root) = inner.root.clone() else {
            return Err(ReadOnlyCause::NoStorageFolder.message().into());
        };
        if !inner.read || !inner.view.writable() {
            self.join_locked(&mut inner, None);
        }
        let mut result = op(&root);
        if matches!(result, Err(Failure::Missing)) {
            self.join_locked(&mut inner, Some(JoinKind::SameFolder));
            result = op(&root);
        }
        inner.view = view::refresh(&root, &self.cache_dir);
        result.map_err(Failure::message)
    }

    /// Join the storage folder. With no `kind`, the join is a change of
    /// folder when the cache came from another folder, or from the local
    /// store of a version before shared settings (R21).
    fn join_locked(&self, inner: &mut Inner, kind: Option<JoinKind>) -> bool {
        let Some(root) = inner.root.clone() else {
            return false;
        };
        let before = snapshot_of(inner);
        let kind = kind.unwrap_or_else(|| match cache::read_cache(&self.cache_dir) {
            Some(Cache { source, .. }) if source != root => JoinKind::ChangeOfFolder,
            _ => JoinKind::SameFolder,
        });
        inner.view = match join::join_folder(&root, &self.cache_dir, kind) {
            Ok(joined) => {
                if joined.written.settings_file || !joined.written.prompt_files.is_empty() {
                    log::info!(
                        "shared settings: the join wrote the settings file: {}, and {} prompt file(s)",
                        joined.written.settings_file,
                        joined.written.prompt_files.len()
                    );
                }
                joined.view
            }
            Err(e) => {
                log::warn!(
                    "shared settings: the join of {} failed: {e}",
                    root.display()
                );
                view::refresh(&root, &self.cache_dir)
            }
        };
        inner.read = true;
        snapshot_of(inner) != before
    }
}

fn snapshot_of(inner: &Inner) -> SharedSnapshot {
    let availability = match (&inner.root, inner.read, &inner.view.availability) {
        (None, _, _) => SharedAvailability::ReadOnly {
            cause: ReadOnlyCause::NoStorageFolder,
        },
        (Some(_), false, _) => SharedAvailability::ReadOnly {
            cause: ReadOnlyCause::NotReadYet,
        },
        (Some(_), true, Availability::Available) => SharedAvailability::Available,
        (Some(_), true, Availability::MissingFile) => SharedAvailability::ReadOnly {
            cause: ReadOnlyCause::NoSettingsFile,
        },
        (Some(_), true, Availability::NotAvailable(cause)) => SharedAvailability::ReadOnly {
            cause: cause.into(),
        },
    };
    SharedSnapshot {
        view: inner.view.clone(),
        availability,
    }
}

/// The built-in or custom prompt `id`, from the files in `root`.
fn find_prompt(root: &Path, id: &str) -> Option<summarize::Prompt> {
    summarize::prompts::built_in(id).or_else(|| {
        let scan = prompts::scan_prompts(root).ok()?;
        scan.get(id).map(|p| summarize::Prompt {
            id: p.id.clone(),
            name: p.name.clone(),
            text: p.text.clone(),
        })
    })
}

fn topic_summary() -> summarize::Prompt {
    summarize::prompts::built_in(TOPIC_SUMMARY_ID).expect("the Topic summary is a built-in prompt")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{Value, json};
    use summarize::prompts::PLAY_BY_PLAY_ID;
    use tempfile::TempDir;

    use super::*;
    use crate::managers::settings::{self, LocalSettings, Settings};

    /// One computer: its local cache folder.
    struct Computer {
        cache: TempDir,
    }

    impl Computer {
        fn new() -> Self {
            Self {
                cache: TempDir::new().unwrap(),
            }
        }

        /// The first launch after the update, with `stored` as the old
        /// local store.
        fn upgrade(&self, stored: &Value) {
            let legacy = settings::legacy_shared(stored).expect("an old store");
            assert!(seed_cache(self.cache.path(), legacy).unwrap());
        }

        fn manager(&self, root: Option<&Path>) -> SharedSettingsManager {
            SharedSettingsManager::new(self.cache.path().to_path_buf(), root.map(Path::to_path_buf))
        }
    }

    fn settings_of(manager: &SharedSettingsManager) -> Settings {
        Settings::from_parts(LocalSettings::default(), &manager.snapshot())
    }

    fn file_of(root: &Path) -> serde_json::Map<String, Value> {
        let bytes = std::fs::read(file::settings_path(root)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn prompt_files(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = match std::fs::read_dir(prompts::prompts_folder(root)) {
            Ok(entries) => entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names
    }

    fn names(settings: &Settings) -> Vec<&str> {
        let mut names: Vec<&str> = settings
            .summary_prompts
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        names.sort();
        names
    }

    /// A local store of the last version before shared settings, with one
    /// custom prompt that is active and a model choice.
    fn old_store() -> Value {
        json!({
            "settings_schema_version": 5,
            "endpoint_model": "llama3",
            "asr_model": "parakeet-tdt-0.6b-v3-q80",
            "speaker_source": "sortformer",
            "live_transcript": false,
            "summary_prompts": [{ "id": "b1", "name": "Standup", "text": "Standup text." }],
            "active_summary_prompt": "b1",
        })
    }

    /// A storage folder that computer A set up: A's model choice and A's
    /// prompt "Recap".
    fn folder_of_a(root: &Path) {
        let a = Computer::new();
        let manager = a.manager(Some(root));
        manager.join_at_launch();
        manager
            .save(&[SharedValue::AsrModel("whisper-large-v3-turbo-q5km".into())])
            .unwrap();
        let id = manager.create_prompt().unwrap();
        manager.rename_prompt(&id, "Recap").unwrap();
        manager.set_prompt_text(&id, "Recap text.").unwrap();
    }

    fn available(manager: &SharedSettingsManager) -> bool {
        manager.snapshot().availability == SharedAvailability::Available
    }

    fn read_only(cause: ReadOnlyCause) -> SharedAvailability {
        SharedAvailability::ReadOnly { cause }
    }

    #[test]
    fn the_first_launch_writes_the_old_prompts_and_recognition_choice_into_an_empty_folder() {
        // R21: a reachable folder with no settings file gets this computer's
        // old shared settings.
        let folder = TempDir::new().unwrap();
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(Some(folder.path()));
        assert!(manager.join_at_launch());
        assert!(available(&manager));

        let written = file_of(folder.path());
        assert_eq!(written["asr_model"], "parakeet-tdt-0.6b-v3-q80");
        assert_eq!(written["speaker_source"], "sortformer");
        assert_eq!(written["live_transcript"], false);
        assert_eq!(written["active_summary_prompt"], "b1");
        assert_eq!(prompt_files(folder.path()), ["Standup.md"]);

        let settings = settings_of(&manager);
        assert_eq!(settings.asr_model, "parakeet-tdt-0.6b-v3-q80");
        assert!(!settings.live_transcript);
        assert_eq!(names(&settings), ["Standup"]);
        assert_eq!(
            settings.active_summary_prompt_snapshot().text,
            "Standup text."
        );
    }

    #[test]
    fn the_first_launch_uses_a_folder_that_has_settings_and_merges_the_old_prompts() {
        // R21, R22, AE3: the folder's settings win, and B's prompt is added.
        let folder = TempDir::new().unwrap();
        folder_of_a(folder.path());
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(Some(folder.path()));
        manager.join_at_launch();

        let settings = settings_of(&manager);
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
        assert_eq!(names(&settings), ["Recap", "Standup"]);
        assert_eq!(
            file_of(folder.path())["asr_model"],
            "whisper-large-v3-turbo-q5km"
        );
    }

    #[test]
    fn a_later_launch_reads_the_folder_and_writes_nothing() {
        // R17: after the first join, a launch only reads. A prompt that
        // another computer removed does not come back.
        let folder = TempDir::new().unwrap();
        let b = Computer::new();
        b.upgrade(&old_store());
        b.manager(Some(folder.path())).join_at_launch();
        std::fs::remove_file(prompts::prompts_folder(folder.path()).join("Standup.md")).unwrap();
        let before = std::fs::read(file::settings_path(folder.path())).unwrap();

        let manager = b.manager(Some(folder.path()));
        manager.join_at_launch();
        assert!(prompt_files(folder.path()).is_empty());
        assert_eq!(
            std::fs::read(file::settings_path(folder.path())).unwrap(),
            before
        );
        let settings = settings_of(&manager);
        assert!(settings.summary_prompts.is_empty());
        // The active prompt's file is gone, so the last copy applies (R14).
        assert_eq!(
            settings.active_summary_prompt_snapshot().text,
            "Standup text."
        );
    }

    #[test]
    fn a_new_install_writes_the_defaults_into_an_empty_folder() {
        let folder = TempDir::new().unwrap();
        let manager = Computer::new().manager(Some(folder.path()));
        manager.join_at_launch();
        assert!(available(&manager));
        assert_eq!(file_of(folder.path()), file::default_settings());
        assert_eq!(settings_of(&manager).asr_model, models::DEFAULT_ASR_MODEL);
    }

    #[test]
    fn a_save_writes_only_its_own_value_and_keeps_values_that_it_does_not_know() {
        // R15, R17, AE11.
        let folder = TempDir::new().unwrap();
        let mut raw = file::default_settings();
        raw.insert("asr_model".into(), json!("model-from-the-future"));
        raw.insert("from_the_future".into(), json!({ "a": 1 }));
        raw.insert("schema_version".into(), json!(99));
        std::fs::create_dir_all(file::longhand_folder(folder.path())).unwrap();
        std::fs::write(
            file::settings_path(folder.path()),
            serde_json::to_vec(&raw).unwrap(),
        )
        .unwrap();
        let manager = Computer::new().manager(Some(folder.path()));
        manager.join_at_launch();
        let state = settings_of(&manager).shared;
        assert!(state.newer_version);
        assert_eq!(state.unknown_values.len(), 1);

        manager
            .save(&[SharedValue::ActiveSummaryPrompt(PLAY_BY_PLAY_ID.into())])
            .unwrap();
        let written = file_of(folder.path());
        assert_eq!(written["active_summary_prompt"], PLAY_BY_PLAY_ID);
        assert_eq!(written["asr_model"], "model-from-the-future");
        assert_eq!(written["from_the_future"], json!({ "a": 1 }));
        assert_eq!(written["schema_version"], 99);
        assert_eq!(settings_of(&manager).active_summary_prompt, PLAY_BY_PLAY_ID);
    }

    #[test]
    fn a_save_keeps_a_change_that_another_computer_made_since_the_last_read() {
        // AE12: B saves live transcript after A changed the active prompt.
        let folder = TempDir::new().unwrap();
        let b = Computer::new();
        let b = b.manager(Some(folder.path()));
        b.join_at_launch();
        let a = Computer::new();
        let a = a.manager(Some(folder.path()));
        a.join_at_launch();
        a.save(&[SharedValue::ActiveSummaryPrompt(PLAY_BY_PLAY_ID.into())])
            .unwrap();

        b.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        let written = file_of(folder.path());
        assert_eq!(written["active_summary_prompt"], PLAY_BY_PLAY_ID);
        assert_eq!(written["live_transcript"], false);
    }

    #[test]
    fn with_the_folder_not_reachable_the_settings_are_read_only_and_a_change_fails() {
        // R16, AE7: the prompts of the cache stay, and nothing is written.
        let parent = TempDir::new().unwrap();
        let folder = parent.path().join("External");
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(Some(&folder));
        manager.join_at_launch();

        let settings = settings_of(&manager);
        assert_eq!(
            settings.shared.availability,
            read_only(ReadOnlyCause::FolderNotAvailable)
        );
        assert_eq!(names(&settings), ["Standup"]);
        assert_eq!(settings.asr_model, "parakeet-tdt-0.6b-v3-q80");

        let error = manager.create_prompt().unwrap_err();
        assert_eq!(error, ReadOnlyCause::FolderNotAvailable.message());
        assert!(manager.activate_prompt(PLAY_BY_PLAY_ID).is_err());
        assert!(manager.save(&[SharedValue::LiveTranscript(true)]).is_err());
        assert!(!folder.exists());
    }

    #[test]
    fn a_folder_that_comes_back_is_used_by_the_next_change() {
        let parent = TempDir::new().unwrap();
        let folder = parent.path().join("External");
        let computer = Computer::new();
        let manager = computer.manager(Some(&folder));
        manager.join_at_launch();
        assert!(!available(&manager));

        std::fs::create_dir(&folder).unwrap();
        manager.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        assert!(available(&manager));
        assert_eq!(file_of(&folder)["live_transcript"], false);
    }

    #[test]
    fn a_settings_file_that_does_not_parse_is_read_only_and_stays() {
        // KTD3: a sync delivered a part of the file.
        let folder = TempDir::new().unwrap();
        std::fs::create_dir_all(file::longhand_folder(folder.path())).unwrap();
        std::fs::write(file::settings_path(folder.path()), b"{\"asr_model\": \"par").unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        assert_eq!(
            manager.snapshot().availability,
            read_only(ReadOnlyCause::DoesNotParse)
        );
        assert!(manager.save(&[SharedValue::LiveTranscript(false)]).is_err());
        assert!(manager.create_prompt().is_err());
        assert_eq!(
            std::fs::read(file::settings_path(folder.path())).unwrap(),
            b"{\"asr_model\": \"par"
        );
        assert!(prompt_files(folder.path()).is_empty());
    }

    #[test]
    fn with_no_storage_folder_the_settings_come_from_the_cache_and_are_read_only() {
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(None);
        assert!(!manager.join_at_launch());
        let settings = settings_of(&manager);
        assert_eq!(
            settings.shared.availability,
            read_only(ReadOnlyCause::NoStorageFolder)
        );
        assert_eq!(names(&settings), ["Standup"]);
        assert_eq!(
            manager.create_prompt().unwrap_err(),
            ReadOnlyCause::NoStorageFolder.message()
        );
    }

    #[test]
    fn before_the_first_read_a_change_joins_the_folder_first() {
        let folder = TempDir::new().unwrap();
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(Some(folder.path()));
        assert_eq!(
            manager.snapshot().availability,
            read_only(ReadOnlyCause::NotReadYet)
        );
        manager.save(&[SharedValue::LiveTranscript(true)]).unwrap();
        // The join ran as the first launch: B's prompt is in the folder.
        assert_eq!(prompt_files(folder.path()), ["Standup.md"]);
        assert_eq!(file_of(folder.path())["live_transcript"], true);
    }

    #[test]
    fn a_settings_file_that_went_away_is_written_again_before_a_save() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        manager.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        std::fs::remove_file(file::settings_path(folder.path())).unwrap();

        manager
            .save(&[SharedValue::CheckUpdatesAutomatically(false)])
            .unwrap();
        let written = file_of(folder.path());
        assert_eq!(written["live_transcript"], false);
        assert_eq!(written["check_updates_automatically"], false);
    }

    #[test]
    fn create_and_duplicate_name_the_prompts_in_turn() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let first = manager.create_prompt().unwrap();
        let second = manager.create_prompt().unwrap();
        assert_ne!(first, second);
        let topic = topic_summary();
        let settings = settings_of(&manager);
        assert_eq!(names(&settings), ["New prompt", "New prompt 2"]);
        assert!(
            settings
                .summary_prompts
                .iter()
                .all(|p| p.text == topic.text)
        );

        manager.duplicate_prompt(TOPIC_SUMMARY_ID).unwrap();
        manager.duplicate_prompt(TOPIC_SUMMARY_ID).unwrap();
        manager.rename_prompt(&first, "Team sync").unwrap();
        manager.set_prompt_text(&first, "Write a recap.").unwrap();
        let copy = manager.duplicate_prompt(&first).unwrap();
        let settings = settings_of(&manager);
        let copy = settings
            .summary_prompts
            .iter()
            .find(|p| p.id == copy)
            .unwrap();
        assert_eq!(copy.name, "Team sync copy");
        assert_eq!(copy.text, "Write a recap.");
        assert_eq!(
            names(&settings),
            [
                "New prompt 2",
                "Team sync",
                "Team sync copy",
                "Topic summary copy",
                "Topic summary copy 2"
            ]
        );
        assert_eq!(
            manager.duplicate_prompt("missing").unwrap_err(),
            NO_SUCH_PROMPT
        );
    }

    #[test]
    fn a_change_of_a_prompt_refuses_bad_input_and_built_ins() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let id = manager.create_prompt().unwrap();
        let other = manager.create_prompt().unwrap();
        manager.rename_prompt(&other, "One on one").unwrap();

        assert!(manager.rename_prompt(&id, "  topic SUMMARY ").is_err());
        assert!(manager.rename_prompt(&id, "one ON one").is_err());
        assert!(manager.rename_prompt(&id, "   ").is_err());
        assert!(manager.rename_prompt(TOPIC_SUMMARY_ID, "Mine").is_err());
        assert!(manager.rename_prompt("missing", "Mine").is_err());
        assert!(manager.set_prompt_text(&id, " \n ").is_err());
        let too_long = "é".repeat(summarize::prompts::PROMPT_TEXT_LIMIT + 1);
        assert!(manager.set_prompt_text(&id, &too_long).is_err());
        assert!(manager.delete_prompt(PLAY_BY_PLAY_ID).is_err());
        assert!(manager.delete_prompt("missing").is_err());
        assert_eq!(names(&settings_of(&manager)), ["New prompt", "One on one"]);
    }

    #[test]
    fn deleting_the_active_prompt_makes_topic_summary_active_in_the_same_change() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let kept = manager.create_prompt().unwrap();
        let other = manager.create_prompt().unwrap();
        manager.activate_prompt(&kept).unwrap();

        manager.delete_prompt(&other).unwrap();
        assert_eq!(settings_of(&manager).active_summary_prompt, kept);
        assert_eq!(
            file_of(folder.path())["active_summary_prompt"],
            kept.as_str()
        );

        manager.delete_prompt(&kept).unwrap();
        let settings = settings_of(&manager);
        assert!(settings.summary_prompts.is_empty());
        assert_eq!(settings.active_summary_prompt, TOPIC_SUMMARY_ID);
        assert_eq!(
            file_of(folder.path())["active_summary_prompt"],
            TOPIC_SUMMARY_ID
        );
    }

    #[test]
    fn activate_accepts_any_known_prompt_and_refuses_an_unknown_id() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let id = manager.create_prompt().unwrap();
        manager.activate_prompt(PLAY_BY_PLAY_ID).unwrap();
        assert_eq!(settings_of(&manager).active_summary_prompt, PLAY_BY_PLAY_ID);
        manager.activate_prompt(&id).unwrap();
        assert_eq!(settings_of(&manager).active_summary_prompt, id);
        assert_eq!(
            manager.activate_prompt("missing").unwrap_err(),
            NO_SUCH_PROMPT
        );
        assert_eq!(file_of(folder.path())["active_summary_prompt"], id.as_str());
    }

    #[test]
    fn a_change_of_folder_to_a_folder_with_settings_merges_this_computers_prompts() {
        // AE3, AE4: B's "Recap" has another text, so it gets a free name.
        let old = TempDir::new().unwrap();
        let shared = TempDir::new().unwrap();
        folder_of_a(shared.path());
        let b = Computer::new();
        let manager = b.manager(Some(old.path()));
        manager.join_at_launch();
        let id = manager.create_prompt().unwrap();
        manager.rename_prompt(&id, "Recap").unwrap();
        manager.set_prompt_text(&id, "B's recap.").unwrap();
        let standup = manager.create_prompt().unwrap();
        manager.rename_prompt(&standup, "Standup").unwrap();

        assert!(manager.change_folder(Some(shared.path().to_path_buf())));
        let settings = settings_of(&manager);
        assert_eq!(settings.asr_model, "whisper-large-v3-turbo-q5km");
        assert_eq!(names(&settings), ["Recap", "Recap 2", "Standup"]);
        // The old folder keeps its copy (R21).
        assert_eq!(prompt_files(old.path()).len(), 2);
        assert!(file::settings_path(old.path()).exists());
    }

    #[test]
    fn a_change_of_folder_to_a_folder_with_notes_and_no_settings_writes_this_computers_settings() {
        // AE10.
        let old = TempDir::new().unwrap();
        let new = TempDir::new().unwrap();
        std::fs::create_dir(new.path().join("2026-09-01 Planning")).unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(old.path()));
        manager.join_at_launch();
        manager.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        manager.create_prompt().unwrap();

        manager.change_folder(Some(new.path().to_path_buf()));
        assert!(available(&manager));
        assert_eq!(file_of(new.path())["live_transcript"], false);
        assert_eq!(prompt_files(new.path()), ["New prompt.md"]);
    }
}
