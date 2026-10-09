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
use session_core::BulkEdit;
use session_save::naming::same_type_name;
use shared_settings::background::{self, BackgroundError};
use shared_settings::cache::{self, Cache};
use shared_settings::file::{self, SaveError, SharedValue, Unavailable};
use shared_settings::join::{self, JoinKind};
use shared_settings::note_types;
use shared_settings::prompts::{self, PromptError};
use shared_settings::view::{self, Availability, SharedView};
use specta::Type;
use summarize::prompts::{TOPIC_SUMMARY_ID, topic_summary};

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
    /// The delay in whole days for the automatic removal of old audio, or
    /// `None` when no audio goes automatically (R2, KTD3). This is the only
    /// value that the removal pass reads. It is `None` unless the view comes
    /// from the settings file of the storage folder and this build knows the
    /// value: while the view comes from the cache or the defaults, no audio
    /// goes.
    pub fn audio_removal_days(&self) -> Option<u32> {
        if self.availability != SharedAvailability::Available {
            return None;
        }
        self.view.audio_removal_days()
    }

    pub fn state(&self) -> SharedState {
        let fields = &self.view.fields;
        SharedState {
            availability: self.availability.clone(),
            newer_version: fields.newer_schema(),
            unknown_values: fields
                .unknown_values()
                .into_iter()
                .map(|(key, value)| UnknownSharedValue {
                    key: key.to_string(),
                    value: value.to_string(),
                })
                .collect(),
        }
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

impl From<BackgroundError> for Failure {
    fn from(error: BackgroundError) -> Self {
        match error {
            BackgroundError::NoSettingsFile => Failure::Missing,
            BackgroundError::NotAvailable(cause) => Failure::NotAvailable(cause),
            other => Failure::Refused(other.to_string()),
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
    /// Goes up by one at each join and each change. A read that runs with
    /// no lock held keeps its result only when no join or change came
    /// during the read (KTD13).
    generation: u64,
}

/// A read of the shared files that runs with no lock held (KTD13).
struct PendingRead {
    root: PathBuf,
    generation: u64,
    cache: Option<Cache>,
}

/// How [`SharedSettingsManager::reread`] starts.
enum ReadStart {
    /// No storage folder is set, or no join ran yet.
    Nothing,
    /// The cache came from another folder, so the read joined the folder.
    /// The value tells whether the view changed.
    Joined(bool),
    Read(Box<PendingRead>),
}

/// The shared settings of the storage folder (KTD7).
pub struct SharedSettingsManager {
    /// The folder of the local cache (KTD6).
    cache_dir: PathBuf,
    /// Held only for short steps with no file I/O in the storage folder, so
    /// [`SharedSettingsManager::snapshot`] never waits for a slow write or
    /// for an online-only file.
    inner: Mutex<Inner>,
    /// Lets one change, join or change of folder run at a time. Only a
    /// holder of this lock changes `inner.root`. A read waits for it before
    /// it starts.
    writer: Mutex<()>,
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
                generation: 0,
            }),
            writer: Mutex::new(()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn writer(&self) -> std::sync::MutexGuard<'_, ()> {
        self.writer.lock().unwrap_or_else(|e| e.into_inner())
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
        let _writer = self.writer();
        self.join(None)
    }

    /// The storage folder changed to `root` (R21). The new folder's shared
    /// settings apply, and this computer's custom prompts merge into them
    /// (R22). With no folder, the view comes from the cache and is
    /// read-only. Returns whether the view changed.
    pub fn change_folder(&self, root: Option<PathBuf>) -> bool {
        let _writer = self.writer();
        let has_root = root.is_some();
        let before = {
            let mut inner = self.lock();
            inner.generation += 1;
            let before = snapshot_of(&inner);
            inner.root = root;
            inner.read = false;
            before
        };
        if has_root {
            self.join(Some(JoinKind::ChangeOfFolder));
        } else {
            let view = view::cached(
                cache::read_cache(&self.cache_dir).as_ref(),
                Availability::MissingFile,
            );
            self.store_view(view);
        }
        self.snapshot() != before
    }

    /// Read the shared files again, after a file event in the storage
    /// folder's `.longhand/` folder or at the full scan (KTD13, R20).
    /// Returns whether the view changed. A read that gives the same view as
    /// before, such as the read after this computer's own change, returns
    /// false. A read of a folder that this computer joined never writes
    /// into it, and a read does nothing before the first join. It holds
    /// no `inner` lock while it reads the storage folder, so a read that
    /// waits for an online-only file does not stop
    /// [`SharedSettingsManager::snapshot`]. Call it off the main thread.
    ///
    /// When the cache did not come from this folder, the read joins the
    /// folder instead (R21, R22). The join at launch or after a change of
    /// folder did not merge the cache when the folder was not available.
    /// A plain read would replace the cache, and this computer's old
    /// prompts would be lost.
    pub fn reread(&self) -> bool {
        let read = match self.begin_read() {
            ReadStart::Nothing => return false,
            ReadStart::Joined(changed) => return changed,
            ReadStart::Read(read) => *read,
        };
        let loaded = view::load(&read.root, read.cache.as_ref());
        self.finish_read(read, loaded)
    }

    fn begin_read(&self) -> ReadStart {
        // A read that starts during a change starts after it, so the read
        // sees the files as the change left them.
        let _writer = self.writer();
        let (root, generation) = {
            let inner = self.lock();
            // Before the first join, the join reads the folder.
            if !inner.read {
                return ReadStart::Nothing;
            }
            let Some(root) = inner.root.clone() else {
                return ReadStart::Nothing;
            };
            (root, inner.generation)
        };
        let cache = cache::read_cache(&self.cache_dir);
        if cache.as_ref().is_some_and(|c| c.source != root) {
            return ReadStart::Joined(self.join(None));
        }
        ReadStart::Read(Box::new(PendingRead {
            root,
            generation,
            cache,
        }))
    }

    /// Keep the result of `read`, unless a join or a change came during
    /// the read: that result is newer.
    fn finish_read(&self, read: PendingRead, loaded: view::Loaded) -> bool {
        let mut inner = self.lock();
        if inner.generation != read.generation || inner.root.as_ref() != Some(&read.root) {
            return false;
        }
        view::store(&self.cache_dir, loaded.cache.as_ref());
        let before = snapshot_of(&inner);
        inner.view = loaded.view;
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
    /// the Topic summary text and title text (R3). Returns its id.
    pub fn create_prompt(&self) -> Result<String, String> {
        let topic = topic_summary();
        self.change(|root| {
            prompts::create_prompt_with_title(root, NEW_PROMPT_NAME, &topic.text, &topic.title_text)
                .map(|p| p.id)
                .map_err(Failure::from)
        })
    }

    /// Add a copy of the prompt `id`, named "X copy", or a free name made
    /// from it, with the texts of the source (R3). Returns the id of the
    /// copy.
    pub fn duplicate_prompt(&self, id: &str) -> Result<String, String> {
        self.change(|root| {
            let source = find_prompt(root, id).ok_or(Failure::Refused(NO_SUCH_PROMPT.into()))?;
            prompts::create_prompt_with_title(
                root,
                &format!("{} copy", source.name),
                &source.text,
                &source.title_text,
            )
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

    /// Give a custom prompt a new title text. It can be empty (R4).
    pub fn set_prompt_title_text(&self, id: &str, title_text: &str) -> Result<(), String> {
        self.change(|root| {
            prompts::set_prompt_title_text(root, id, title_text)
                .map(drop)
                .map_err(Failure::from)
        })
    }

    /// Remove a custom prompt. When it was active, the Topic summary becomes
    /// active in the same change (R4). Each type that chose it changes to
    /// "Default" in the same save (R5, KTD6).
    pub fn delete_prompt(&self, id: &str) -> Result<(), String> {
        self.change(|root| {
            let settings = prompts::delete_prompt(root, id)?;
            let fields = &settings.fields;
            let mut patch = Vec::new();
            if fields
                .active_summary_prompt
                .known()
                .is_some_and(|active| active == id)
            {
                patch.push(SharedValue::ActiveSummaryPrompt(TOPIC_SUMMARY_ID.into()));
            }
            patch.extend(
                note_types::prompts_without(&fields.type_prompt_map(), id)
                    .map(SharedValue::NoteTypePrompts),
            );
            if !patch.is_empty() {
                file::save_settings(root, &patch)?;
            }
            Ok(())
        })
    }

    /// Save the Background text that each summary uses (R4, R5). Only a
    /// change that the user makes calls this. An empty text writes an empty
    /// file (KTD2).
    pub fn set_background(&self, text: &str) -> Result<(), String> {
        self.change(|root| background::save_background(root, text).map_err(Failure::from))
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
    /// The caller holds no lock: this takes the writer lock.
    fn change<T>(&self, op: impl Fn(&Path) -> Result<T, Failure>) -> Result<T, String> {
        let _writer = self.writer();
        let (root, join_first) = {
            let mut inner = self.lock();
            inner.generation += 1;
            let Some(root) = inner.root.clone() else {
                return Err(ReadOnlyCause::NoStorageFolder.message().into());
            };
            (root, !inner.read || !inner.view.writable())
        };
        if join_first {
            self.join(None);
        }
        let mut result = op(&root);
        if matches!(result, Err(Failure::Missing)) {
            self.join(Some(JoinKind::SameFolder));
            result = op(&root);
        }
        self.store_view(view::refresh(&root, &self.cache_dir));
        result.map_err(Failure::message)
    }

    /// Join the storage folder. With no `kind`, the join is a change of
    /// folder when the cache came from another folder, or from the local
    /// store of a version before shared settings (R21). The caller holds
    /// the writer lock.
    fn join(&self, kind: Option<JoinKind>) -> bool {
        let (root, before) = {
            let mut inner = self.lock();
            inner.generation += 1;
            let Some(root) = inner.root.clone() else {
                return false;
            };
            (root, snapshot_of(&inner))
        };
        let view = self.joined_view(&root, kind);
        let mut inner = self.lock();
        inner.generation += 1;
        inner.view = view;
        inner.read = true;
        snapshot_of(&inner) != before
    }

    /// The view after a join of `root`. This does the file I/O of the join
    /// with no lock on `inner`.
    fn joined_view(&self, root: &Path, kind: Option<JoinKind>) -> SharedView {
        let old = cache::read_cache(&self.cache_dir);
        let kind = kind.unwrap_or_else(|| match &old {
            Some(Cache { source, .. }) if source != root => JoinKind::ChangeOfFolder,
            _ => JoinKind::SameFolder,
        });
        match join::join(root, old.as_ref(), kind) {
            Ok(joined) => {
                view::store(&self.cache_dir, joined.cache.as_ref());
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
                view::refresh(root, &self.cache_dir)
            }
        }
    }

    /// Use `view` from now on. A read that started before this gives no
    /// result (KTD13).
    fn store_view(&self, view: SharedView) {
        let mut inner = self.lock();
        inner.generation += 1;
        inner.view = view;
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

/// One note type of the effective type list, for the surface (R3, KTD9).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq)]
pub struct NoteTypeEntry {
    pub name: String,
    /// The name passes the type name check. Only a valid type is offered
    /// for a new note or a type change. Settings offers only a rename or a
    /// delete for a type that is not valid.
    pub valid: bool,
    /// The shared list has the type. False for a type that comes only from
    /// a folder in the storage folder (R34).
    pub shared: bool,
    /// The number of notes of the type.
    pub notes: u32,
    /// The built-in type General: the notes whose home folder is directly
    /// in the storage folder. It is first in the list. It cannot be renamed
    /// or deleted, and a note of General has the type `None`.
    pub general: bool,
    /// The id of the summary prompt that the type chooses, or `None` for
    /// "Default" (R1). A choice that names a prompt that this computer
    /// never read is "Default" (R7). A type that is not valid has no
    /// choice (R18).
    pub prompt_choice: Option<String>,
    /// The name of the prompt of `prompt_choice`, from the last copy that
    /// this computer read. Settings shows this name also when the file of
    /// the prompt is missing (R7).
    pub prompt_name: Option<String>,
    /// The id of the prompt that a summary of the type uses now: the
    /// chosen prompt, or the default prompt for "Default" (R3, KTD2).
    pub summary_prompt_id: String,
}

/// The effective type list and the default type (R2, R7, KTD9).
#[derive(Serialize, Type, Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteTypeList {
    /// General, then the types of the shared list and the folder-only
    /// types in name order that ignores case.
    pub types: Vec<NoteTypeEntry>,
    /// The type of a new note from the New Note button and the shortcut.
    /// `None` is General. A shared default that names no valid type of the
    /// list is General.
    pub default_type: Option<String>,
}

/// The type folders and the home folders of the storage folder, from the
/// store rows (KTD4, KTD9). This reads no file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct StorageFolders {
    pub types: Vec<note_types::TypeFolder>,
    /// The names of the home folders directly in the storage folder.
    pub homes: Vec<String>,
}

impl StorageFolders {
    /// The folders of the storage folder `root` that hold the home folders
    /// of `records`. A discarded note and a note in another folder do not
    /// count. A type folder is leaving when each of its notes has a pending
    /// type with that folder as `from` (KTD9).
    pub fn from_records(root: &Path, records: &[session_core::SessionRecord]) -> Self {
        let mut folders = Self::default();
        for record in records {
            if record.state == session_core::SessionState::Discarded {
                continue;
            }
            let Some(home) = record.home_path.as_deref().map(Path::new) else {
                continue;
            };
            let leaves = |folder: &str| {
                record.pending_type.as_ref().is_some_and(|pending| {
                    pending
                        .from
                        .as_deref()
                        .is_some_and(|from| same_type_name(from, folder))
                })
            };
            match session_core::folder::note_type_in(root, home) {
                Some(None) => {
                    if let Some(name) = home.file_name() {
                        folders.homes.push(name.to_string_lossy().into_owned());
                    }
                }
                Some(Some(name)) => {
                    let leaving = leaves(&name);
                    match folders.types.iter_mut().find(|f| f.name == name) {
                        Some(folder) => {
                            folder.notes += 1;
                            folder.leaving &= leaving;
                        }
                        None => folders.types.push(note_types::TypeFolder {
                            name,
                            notes: 1,
                            leaving,
                        }),
                    }
                }
                None => {}
            }
        }
        folders
    }

    /// Mark the type folders of `names` as leaving: a type rename, merge or
    /// delete of this computer is not done yet, so the old folder does not
    /// come back as a type (KTD7).
    pub fn mark_leaving<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        for name in names {
            for folder in &mut self.types {
                if same_type_name(&folder.name, name) {
                    folder.leaving = true;
                }
            }
        }
    }
}

/// The note types of the shared settings `fields`. A list of a form that
/// this build does not know gives no types.
fn shared_types(fields: &file::SharedFields) -> Vec<String> {
    fields.note_types.known().cloned().unwrap_or_default()
}

/// The note types of the settings file of `root`, read at the time of a
/// change (R15).
fn types_in_file(root: &Path) -> Result<Vec<String>, Failure> {
    Ok(shared_types(&fields_in_file(root)?))
}

/// The shared fields of the settings file of `root`, read at the time of a
/// change (R15).
fn fields_in_file(root: &Path) -> Result<file::SharedFields, Failure> {
    match file::read_settings(root) {
        file::FileRead::Parsed(settings) => Ok(settings.fields),
        file::FileRead::Missing => Err(Failure::Missing),
        file::FileRead::NotAvailable(cause) => Err(Failure::NotAvailable(cause)),
    }
}

/// The default type of the shared fields `fields`. `None` is General.
fn default_type(fields: &file::SharedFields) -> Option<String> {
    fields.default_note_type.known().cloned().flatten()
}

/// The values that a type rename, merge or delete writes (R6, R7, KTD6):
/// the list, the default type when it names the old type, and the type
/// prompt choices when the old type has one. `merged` tells that `to` is
/// another type.
fn edit_patch(
    fields: &file::SharedFields,
    list: Vec<String>,
    from: &str,
    to: Option<&str>,
    merged: bool,
) -> Vec<SharedValue> {
    let mut patch = vec![SharedValue::NoteTypes(list)];
    if let Some(default) = note_types::default_after(default_type(fields).as_deref(), from, to) {
        patch.push(SharedValue::DefaultNoteType(default));
    }
    patch.extend(prompts_patch(fields, from, to, merged));
    patch
}

/// The type prompt choices after the edit of the type `from`, when the
/// edit changes them (KTD6). See [`note_types::prompts_after_edit`].
fn prompts_patch(
    fields: &file::SharedFields,
    from: &str,
    to: Option<&str>,
    merged: bool,
) -> Option<SharedValue> {
    note_types::prompts_after_edit(&fields.type_prompt_map(), from, to, merged)
        .map(SharedValue::NoteTypePrompts)
}

impl SharedSettingsManager {
    /// General and the effective type list, and the default type, from the
    /// shared view in memory and `folders` (KTD9). This does no file I/O
    /// and never writes.
    pub fn note_types(&self, folders: &StorageFolders) -> NoteTypeList {
        let view = self.snapshot().view;
        let fields = &view.fields;
        let types = note_types::effective_types(&shared_types(fields), &folders.types);
        let default = default_type(fields);
        let count = |notes: usize| u32::try_from(notes).unwrap_or(u32::MAX);
        // The default prompt. A missing prompt is the Topic summary.
        let default_prompt = view
            .active
            .prompt()
            .map_or(TOPIC_SUMMARY_ID, |p| p.id.as_str());
        // A type that is not valid uses the default prompt (R18). A folder
        // named "General" is such a type, and it does not get the choice of
        // the built-in General.
        let prompt_of = |note_type: Option<&str>, valid: bool| {
            let choice = fields
                .note_type_prompts
                .known()
                .and_then(|map| note_types::prompt_choice(map, note_type, valid))
                .and_then(|id| view.type_prompts.get(id))
                .and_then(|p| p.prompt());
            let summary_prompt_id = choice.map_or_else(|| default_prompt.into(), |p| p.id.clone());
            (
                choice.map(|p| p.id.clone()),
                choice.map(|p| p.name.clone()),
                summary_prompt_id,
            )
        };
        let (prompt_choice, prompt_name, summary_prompt_id) = prompt_of(None, true);
        let general = NoteTypeEntry {
            name: session_save::naming::GENERAL.to_string(),
            valid: true,
            shared: false,
            notes: count(folders.homes.len()),
            general: true,
            prompt_choice,
            prompt_name,
            summary_prompt_id,
        };
        let entries: Vec<NoteTypeEntry> = types
            .iter()
            .map(|t| {
                let (prompt_choice, prompt_name, summary_prompt_id) =
                    prompt_of(Some(&t.name), t.valid);
                NoteTypeEntry {
                    name: t.name.clone(),
                    valid: t.valid,
                    shared: t.shared,
                    notes: count(t.notes),
                    general: false,
                    prompt_choice,
                    prompt_name,
                    summary_prompt_id,
                }
            })
            .collect();
        NoteTypeList {
            default_type: note_types::effective_default(default.as_deref(), &types),
            types: std::iter::once(general).chain(entries).collect(),
        }
    }

    /// Add the note type `name` to the shared list (R3). The name goes
    /// through the type name check (R28, KTD8), and a name that the list
    /// has is refused. The save also puts the folder-only types into the
    /// list (KTD9). Returns the name as saved.
    pub fn add_note_type(&self, name: &str, folders: &StorageFolders) -> Result<String, String> {
        self.change(|root| {
            let fields = fields_in_file(root)?;
            let (list, name) =
                note_types::add_type(&shared_types(&fields), &folders.types, &folders.homes, name)
                    .map_err(|e| Failure::Refused(e.to_string()))?;
            let mut patch = vec![SharedValue::NoteTypes(list)];
            // A new type starts with "Default", so an old choice with its
            // name goes (R1, KTD6).
            patch.extend(prompts_patch(&fields, &name, None, false));
            file::save_settings(root, &patch)?;
            Ok(name)
        })
    }

    /// Make `name` the default type, or General for `None` (R7). The
    /// type must be a valid type of the effective list. A folder-only type
    /// goes into the shared list in the same save (KTD9).
    pub fn set_default_note_type(
        &self,
        name: Option<&str>,
        folders: &StorageFolders,
    ) -> Result<(), String> {
        self.change(|root| {
            let shared = types_in_file(root)?;
            let (default, list) = note_types::default_to_save(&shared, &folders.types, name)
                .map_err(|e| Failure::Refused(e.to_string()))?;
            let mut patch = vec![SharedValue::DefaultNoteType(default)];
            patch.extend(list.map(SharedValue::NoteTypes));
            file::save_settings(root, &patch)?;
            Ok(())
        })
    }

    /// Choose the summary prompt `prompt_id` for the note type `name`, or
    /// "Default" for `None` (R1, R2). `None` and "General" as `name` are
    /// General. The type must be a valid type of the effective list, and
    /// the prompt must exist. A folder-only type goes into the shared list
    /// in the same save (KTD6, KTD9).
    pub fn set_note_type_prompt(
        &self,
        name: &str,
        prompt_id: Option<&str>,
        folders: &StorageFolders,
    ) -> Result<NoteTypeList, String> {
        self.change(|root| {
            let fields = fields_in_file(root)?;
            // The type check of a default type is the same check: General,
            // or a valid type with the spelling of the list.
            let (chosen, list) =
                note_types::default_to_save(&shared_types(&fields), &folders.types, Some(name))
                    .map_err(|e| Failure::Refused(e.to_string()))?;
            let key = chosen.unwrap_or_else(|| session_save::naming::GENERAL.to_string());
            if let Some(id) = prompt_id
                && find_prompt(root, id).is_none()
            {
                return Err(Failure::Refused(NO_SUCH_PROMPT.into()));
            }
            let mut map = fields.type_prompt_map();
            note_types::set_prompt_choice(&mut map, &key, prompt_id);
            let mut patch = vec![SharedValue::NoteTypePrompts(map)];
            patch.extend(list.map(SharedValue::NoteTypes));
            file::save_settings(root, &patch)?;
            Ok(())
        })?;
        Ok(self.note_types(folders))
    }

    /// Rename the note type `name` to `new_name` (R6). A new name of
    /// another type merges the two types. The default type follows the
    /// rename (R7). `before_save` receives the edit before the list
    /// changes, so the caller records it first (KTD7). Returns the edit,
    /// or `None` when the name does not change.
    pub fn rename_note_type(
        &self,
        name: &str,
        new_name: &str,
        folders: &StorageFolders,
        before_save: &dyn Fn(&BulkEdit) -> Result<(), String>,
    ) -> Result<Option<BulkEdit>, String> {
        self.change(|root| {
            let fields = fields_in_file(root)?;
            let renamed = note_types::rename_type(
                &shared_types(&fields),
                &folders.types,
                &folders.homes,
                name,
                new_name,
            )
            .map_err(|e| Failure::Refused(e.to_string()))?;
            if renamed.from == renamed.to {
                return Ok(None);
            }
            let edit = BulkEdit::RenameType {
                from: renamed.from.clone(),
                to: renamed.to.clone(),
            };
            before_save(&edit).map_err(Failure::Refused)?;
            let patch = edit_patch(
                &fields,
                renamed.list,
                &renamed.from,
                Some(&renamed.to),
                renamed.merged,
            );
            file::save_settings(root, &patch)?;
            Ok(Some(edit))
        })
    }

    /// Delete the note type `name` (R6). When it is the default type, the
    /// default becomes General in the same save (R7). `before_save`
    /// receives the edit before the list changes (KTD7).
    pub fn delete_note_type(
        &self,
        name: &str,
        folders: &StorageFolders,
        before_save: &dyn Fn(&BulkEdit) -> Result<(), String>,
    ) -> Result<BulkEdit, String> {
        self.change(|root| {
            let fields = fields_in_file(root)?;
            let (list, name) =
                note_types::delete_type(&shared_types(&fields), &folders.types, name)
                    .map_err(|e| Failure::Refused(e.to_string()))?;
            let edit = BulkEdit::DeleteType { name: name.clone() };
            before_save(&edit).map_err(Failure::Refused)?;
            file::save_settings(root, &edit_patch(&fields, list, &name, None, false))?;
            Ok(edit)
        })
    }

    /// Continue a type rename, merge or delete whose list change a quit or
    /// a crash stopped (KTD7): write only what the list, the default type
    /// and the type prompt choices do not have yet (KTD6).
    pub fn finish_type_edit(&self, edit: &BulkEdit) -> Result<(), String> {
        let (from, to) = match edit {
            BulkEdit::RenameType { from, to } => (from.as_str(), Some(to.as_str())),
            BulkEdit::DeleteType { name } => (name.as_str(), None),
            // A tag edit has no shared-settings part (KTD11).
            BulkEdit::RenameTag { .. } | BulkEdit::DeleteTag { .. } => return Ok(()),
        };
        self.change(|root| {
            let fields = fields_in_file(root)?;
            let mut patch = Vec::new();
            // The edit is a merge when the list has the target type. After
            // a saved edit, the old type has no choice, so the choices do
            // not change.
            let merged = to.is_some_and(|to| {
                !same_type_name(from, to)
                    && shared_types(&fields).iter().any(|t| same_type_name(t, to))
            });
            patch.extend(prompts_patch(&fields, from, to, merged));
            if let Some(list) = note_types::list_after_edit(&shared_types(&fields), from, to) {
                patch.push(SharedValue::NoteTypes(list));
            }
            if let Some(default) =
                note_types::default_after(default_type(&fields).as_deref(), from, to)
                && default.as_deref() != default_type(&fields).as_deref()
            {
                patch.push(SharedValue::DefaultNoteType(default));
            }
            if !patch.is_empty() {
                file::save_settings(root, &patch)?;
            }
            Ok(())
        })
    }
}

/// The built-in or custom prompt `id`, from the files in `root`.
fn find_prompt(root: &Path, id: &str) -> Option<summarize::Prompt> {
    summarize::prompts::built_in(id).or_else(|| {
        let scan = prompts::scan_prompts(root).ok()?;
        scan.get(id).map(summarize::Prompt::from)
    })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};
    use summarize::prompts::PLAY_BY_PLAY_ID;
    use tempfile::TempDir;

    use super::*;
    use crate::managers::settings::{self, AudioRemovalSetting, LocalSettings, Settings};

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
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "parakeet-tdt-0.6b-v3-q80"
        );
        assert!(!settings.chosen_recognition.live_transcript);
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
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "whisper-large-v3-turbo-q5km"
        );
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
        assert_eq!(
            settings_of(&manager).chosen_recognition.asr_model,
            models::DEFAULT_ASR_MODEL
        );
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
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "parakeet-tdt-0.6b-v3-q80"
        );

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
    fn the_audio_removal_delay_is_known_only_while_the_shared_settings_are_available() {
        // R2, AE10, KTD3.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        // Before the first read, the view comes from the defaults.
        assert_eq!(
            settings_of(&manager).audio_removal,
            AudioRemovalSetting::default()
        );
        assert_eq!(manager.snapshot().audio_removal_days(), None);
        manager.join_at_launch();
        assert_eq!(manager.snapshot().audio_removal_days(), Some(30));
        manager
            .save(&[SharedValue::AudioRemoval(file::AudioRemoval::AfterDays(90))])
            .unwrap();
        assert_eq!(manager.snapshot().audio_removal_days(), Some(90));
        assert_eq!(
            settings_of(&manager).audio_removal,
            AudioRemovalSetting::AfterDays { days: 90 }
        );

        // A half-synced file: the cache still shows 90 days, but no audio
        // goes, and the read writes nothing.
        std::fs::write(file::settings_path(folder.path()), b"{\"audio_rem").unwrap();
        manager.reread();
        assert_eq!(
            manager.snapshot().availability,
            read_only(ReadOnlyCause::DoesNotParse)
        );
        assert_eq!(
            settings_of(&manager).audio_removal,
            AudioRemovalSetting::AfterDays { days: 90 }
        );
        assert_eq!(manager.snapshot().audio_removal_days(), None);
        assert_eq!(
            std::fs::read(file::settings_path(folder.path())).unwrap(),
            b"{\"audio_rem"
        );

        // A value that this build does not know is not a delay.
        std::fs::write(
            file::settings_path(folder.path()),
            br#"{"schema_version": 1, "audio_removal": 0}"#,
        )
        .unwrap();
        manager.reread();
        assert!(available(&manager));
        assert_eq!(manager.snapshot().audio_removal_days(), None);

        // With no storage folder, the cache gives no delay.
        let no_folder = computer.manager(None);
        assert!(!no_folder.join_at_launch());
        assert_eq!(no_folder.snapshot().audio_removal_days(), None);
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
        // A new prompt starts with the Topic summary title text (R3).
        assert!(
            settings
                .summary_prompts
                .iter()
                .all(|p| p.text == topic.text && p.title_text == topic.title_text)
        );

        manager.duplicate_prompt(TOPIC_SUMMARY_ID).unwrap();
        manager.duplicate_prompt(TOPIC_SUMMARY_ID).unwrap();
        manager.rename_prompt(&first, "Team sync").unwrap();
        manager.set_prompt_text(&first, "Write a recap.").unwrap();
        manager
            .set_prompt_title_text(&first, "Name the team.")
            .unwrap();
        let copy = manager.duplicate_prompt(&first).unwrap();
        let settings = settings_of(&manager);
        let copy = settings
            .summary_prompts
            .iter()
            .find(|p| p.id == copy)
            .unwrap();
        assert_eq!(copy.name, "Team sync copy");
        assert_eq!(copy.text, "Write a recap.");
        // A duplicate copies the title text of its source (R3).
        assert_eq!(copy.title_text, "Name the team.");
        let built_in_copy = settings
            .summary_prompts
            .iter()
            .find(|p| p.name == "Topic summary copy")
            .unwrap();
        assert_eq!(built_in_copy.title_text, topic.title_text);
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
        assert!(
            manager
                .set_prompt_title_text(TOPIC_SUMMARY_ID, "Mine.")
                .is_err()
        );
        assert!(manager.set_prompt_title_text("missing", "Mine.").is_err());
        let too_long = "é".repeat(summarize::prompts::TITLE_TEXT_LIMIT + 1);
        assert!(manager.set_prompt_title_text(&id, &too_long).is_err());
        assert!(manager.delete_prompt(PLAY_BY_PLAY_ID).is_err());
        assert!(manager.delete_prompt("missing").is_err());
        assert_eq!(names(&settings_of(&manager)), ["New prompt", "One on one"]);
    }

    #[test]
    fn a_title_text_can_be_empty_or_at_the_limit() {
        // R4: an empty title text saves. The summary text stays.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let id = manager.create_prompt().unwrap();
        let texts = |manager: &SharedSettingsManager| {
            let settings = settings_of(manager);
            let prompt = settings.summary_prompts.into_iter().find(|p| p.id == id);
            prompt.map(|p| (p.text, p.title_text)).unwrap()
        };

        manager.set_prompt_title_text(&id, "").unwrap();
        assert_eq!(texts(&manager), (topic_summary().text, String::new()));
        let at_limit = "é".repeat(summarize::prompts::TITLE_TEXT_LIMIT);
        manager.set_prompt_title_text(&id, &at_limit).unwrap();
        assert_eq!(texts(&manager).1, at_limit);
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
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "whisper-large-v3-turbo-q5km"
        );
        assert_eq!(names(&settings), ["Recap", "Recap 2", "Standup"]);
        // The old folder keeps its copy (R21).
        assert_eq!(prompt_files(old.path()).len(), 2);
        assert!(file::settings_path(old.path()).exists());
    }

    /// Everything in `root`, with the bytes of each file.
    fn tree(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
        let mut files = Vec::new();
        let mut folders = vec![root.to_path_buf()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    folders.push(path.clone());
                    files.push((path, Vec::new()));
                } else {
                    let bytes = std::fs::read(&path).unwrap();
                    files.push((path, bytes));
                }
            }
        }
        files.sort();
        files
    }

    #[test]
    fn a_read_shows_a_prompt_that_another_computer_changed() {
        // AE2, R14, R20: B reads A's new prompt text and active prompt
        // while it runs.
        let folder = TempDir::new().unwrap();
        let (a, b) = (Computer::new(), Computer::new());
        let a = a.manager(Some(folder.path()));
        a.join_at_launch();
        let id = a.create_prompt().unwrap();
        a.rename_prompt(&id, "Recap").unwrap();
        let b = b.manager(Some(folder.path()));
        b.join_at_launch();
        assert!(!b.reread(), "nothing changed since the join");

        a.set_prompt_text(&id, "New recap text.").unwrap();
        let before = tree(folder.path());
        assert!(b.reread());
        assert_eq!(tree(folder.path()), before, "a read never writes");
        let settings = settings_of(&b);
        let recap = settings
            .summary_prompts
            .iter()
            .find(|p| p.id == id)
            .unwrap();
        assert_eq!(recap.text, "New recap text.");

        a.activate_prompt(&id).unwrap();
        assert!(b.reread());
        assert_eq!(settings_of(&b).active_summary_prompt, id);
        assert!(!b.reread());
    }

    #[test]
    fn a_read_after_this_computers_own_change_gives_no_change() {
        // KTD13: the event of this computer's own write causes no loop.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        manager.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        assert!(!manager.reread());
        let id = manager.create_prompt().unwrap();
        manager.set_prompt_text(&id, "Mine.").unwrap();
        assert!(!manager.reread());
    }

    #[test]
    fn a_read_when_the_folder_goes_away_keeps_the_prompts_and_is_read_only() {
        // R16: the view comes from the cache.
        let parent = TempDir::new().unwrap();
        let folder = parent.path().join("External");
        std::fs::create_dir(&folder).unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(&folder));
        manager.join_at_launch();
        let id = manager.create_prompt().unwrap();
        manager.rename_prompt(&id, "Recap").unwrap();

        std::fs::rename(&folder, parent.path().join("Elsewhere")).unwrap();
        assert!(manager.reread());
        let settings = settings_of(&manager);
        assert_eq!(
            settings.shared.availability,
            read_only(ReadOnlyCause::FolderNotAvailable)
        );
        assert_eq!(names(&settings), ["Recap"]);
        assert!(!manager.reread());

        std::fs::rename(parent.path().join("Elsewhere"), &folder).unwrap();
        assert!(manager.reread());
        assert!(available(&manager));
    }

    #[test]
    fn a_read_after_a_first_launch_with_the_folder_not_reachable_merges_the_old_prompts() {
        // R21, R22: the join at launch wrote nothing, so the first read
        // that finds the folder joins it. B's old prompt is added, and A's
        // settings and prompts stay.
        let parent = TempDir::new().unwrap();
        let folder = parent.path().join("External");
        let b = Computer::new();
        b.upgrade(&old_store());
        let manager = b.manager(Some(&folder));
        manager.join_at_launch();
        assert_eq!(
            manager.snapshot().availability,
            read_only(ReadOnlyCause::FolderNotAvailable)
        );

        std::fs::create_dir(&folder).unwrap();
        folder_of_a(&folder);
        assert!(manager.reread());
        assert!(available(&manager));
        assert_eq!(prompt_files(&folder), ["Recap.md", "Standup.md"]);
        assert_eq!(file_of(&folder)["asr_model"], "whisper-large-v3-turbo-q5km");
        let settings = settings_of(&manager);
        assert_eq!(names(&settings), ["Recap", "Standup"]);
        assert_eq!(
            settings.chosen_recognition.asr_model,
            "whisper-large-v3-turbo-q5km"
        );

        // The folder is joined now, so the next read only reads.
        let before = tree(&folder);
        assert!(!manager.reread());
        assert_eq!(tree(&folder), before, "a read never writes");
    }

    #[test]
    fn a_read_before_the_first_join_or_with_no_folder_does_nothing() {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        assert!(!manager.reread());
        assert!(!file::settings_path(folder.path()).exists());
        assert_eq!(
            manager.snapshot().availability,
            read_only(ReadOnlyCause::NotReadYet)
        );
        assert!(!computer.manager(None).reread());
    }

    #[test]
    fn a_read_that_a_change_passes_does_not_replace_the_view_of_the_change() {
        // The read runs with no lock held. A change during the read wins.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let ReadStart::Read(started) = manager.begin_read() else {
            panic!("the read did not start");
        };
        let loaded = view::load(&started.root, started.cache.as_ref());
        manager.save(&[SharedValue::LiveTranscript(false)]).unwrap();
        assert!(!manager.finish_read(*started, loaded));
        assert!(!settings_of(&manager).chosen_recognition.live_transcript);
    }

    #[test]
    fn a_snapshot_during_a_change_gives_the_view_from_before_the_change() {
        // A slow save, such as a save to an online-only file, does not
        // stop a read of the settings.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let before = manager.snapshot();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel::<()>();
        let (read_tx, read_rx) = std::sync::mpsc::channel();
        let manager = &manager;
        std::thread::scope(|scope| {
            let change = scope.spawn(move || {
                manager.change(|root| {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    file::save_settings(root, &[SharedValue::LiveTranscript(false)])
                        .map(drop)
                        .map_err(Failure::from)
                })
            });
            started_rx.recv().unwrap();
            scope.spawn(move || read_tx.send(manager.snapshot()).unwrap());
            let during = read_rx.recv_timeout(std::time::Duration::from_secs(5));
            finish_tx.send(()).unwrap();
            change.join().unwrap().unwrap();
            assert_eq!(
                during.ok(),
                Some(before),
                "the snapshot waited for the change"
            );
        });
        assert!(!settings_of(manager).chosen_recognition.live_transcript);
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

    fn background_file(root: &Path) -> Option<String> {
        std::fs::read_to_string(shared_settings::background::background_path(root)).ok()
    }

    #[test]
    fn a_background_save_writes_the_file_and_the_settings_show_the_new_text() {
        // R4, R5.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let settings = settings_of(&manager);
        assert_eq!(settings.background, "");
        assert_eq!(settings.background_read_only, None);
        assert_eq!(
            settings.background_limit as usize,
            shared_settings::background::BACKGROUND_TEXT_LIMIT
        );

        manager.set_background("I am Erv Walter.").unwrap();
        assert_eq!(settings_of(&manager).background, "I am Erv Walter.");
        assert_eq!(
            background_file(folder.path()).as_deref(),
            Some("I am Erv Walter.")
        );

        // A clear writes an empty file. It does not delete the file (KTD2).
        manager.set_background("").unwrap();
        assert_eq!(settings_of(&manager).background, "");
        assert_eq!(background_file(folder.path()).as_deref(), Some(""));
    }

    #[test]
    fn a_background_over_the_limit_is_refused_and_writes_nothing() {
        // R6.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let limit = shared_settings::background::BACKGROUND_TEXT_LIMIT;

        let error = manager.set_background(&"x".repeat(limit + 1)).unwrap_err();
        assert_eq!(
            error,
            shared_settings::background::BackgroundError::TooLong.to_string()
        );
        assert_eq!(background_file(folder.path()), None);

        manager.set_background(&"x".repeat(limit)).unwrap();
        assert_eq!(settings_of(&manager).background.chars().count(), limit);
    }

    #[test]
    fn with_the_folder_not_reachable_the_background_is_read_only_and_a_save_fails() {
        // R5: the cached text stays, and nothing is written.
        let parent = TempDir::new().unwrap();
        let folder = parent.path().join("External");
        let computer = Computer::new();
        let manager = computer.manager(Some(&folder));
        manager.join_at_launch();

        let settings = settings_of(&manager);
        assert_eq!(
            settings.background_read_only.as_deref(),
            Some(ReadOnlyCause::FolderNotAvailable.message())
        );
        let error = manager.set_background("Text.").unwrap_err();
        assert_eq!(error, ReadOnlyCause::FolderNotAvailable.message());
        assert!(!folder.exists());
    }

    #[test]
    fn a_background_file_that_cannot_be_used_keeps_the_cached_text_and_stays() {
        // KTD3: summaries use the cached text, the Background is read-only
        // with a reason, and a save does not write over the file.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        manager.set_background("Cached text.").unwrap();

        let too_long = "x".repeat(shared_settings::background::BACKGROUND_TEXT_LIMIT + 1);
        std::fs::write(
            shared_settings::background::background_path(folder.path()),
            &too_long,
        )
        .unwrap();
        assert!(manager.reread());

        let settings = settings_of(&manager);
        assert_eq!(settings.background, "Cached text.");
        assert_eq!(
            settings.background_read_only,
            Some(shared_settings::background::BackgroundUnavailable::TooLong.to_string())
        );
        // The rest of the shared settings stay writable.
        assert!(available(&manager));
        assert!(manager.set_background("New text.").is_err());
        assert_eq!(background_file(folder.path()), Some(too_long));
    }

    #[test]
    fn a_background_that_another_computer_changed_shows_after_a_read() {
        let folder = TempDir::new().unwrap();
        let (computer_a, computer_b) = (Computer::new(), Computer::new());
        let a = computer_a.manager(Some(folder.path()));
        let b_manager = computer_b.manager(Some(folder.path()));
        a.join_at_launch();
        b_manager.join_at_launch();

        a.set_background("From A.").unwrap();
        assert!(b_manager.reread());
        assert_eq!(settings_of(&b_manager).background, "From A.");
    }

    // ------------------------------------------------------------ note types

    fn type_folder(name: &str, notes: usize, leaving: bool) -> note_types::TypeFolder {
        note_types::TypeFolder {
            name: name.into(),
            notes,
            leaving,
        }
    }

    fn type_names(list: &NoteTypeList) -> Vec<&str> {
        list.types.iter().map(|t| t.name.as_str()).collect()
    }

    fn record(
        id: &str,
        home: Option<PathBuf>,
        pending_from: Option<&str>,
    ) -> session_core::SessionRecord {
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-07T14:00:00+00:00").unwrap();
        session_core::SessionRecord {
            id: id.into(),
            created_at: at,
            state: session_core::SessionState::Saved,
            queue_seq: None,
            speaker_count: None,
            asr_model: None,
            recognition_snapshot: None,
            home_path: home.map(|h| h.to_string_lossy().into_owned()),
            title: None,
            found_speakers: None,
            live_transcript: None,
            transcribe_again_model: None,
            started_at: None,
            note_type: None,
            tags: Vec::new(),
            pending_type: pending_from.map(|from| session_core::folder::PendingType {
                to: None,
                from: Some(from.into()),
                by: "computer-b".into(),
                at,
            }),
        }
    }

    #[test]
    fn the_storage_folders_come_from_the_rows_in_the_storage_folder() {
        // KTD4, KTD9.
        let root = Path::new("/notes");
        let mut discarded = record("d", Some(root.join("Board/2026-10-01 Plan")), None);
        discarded.state = session_core::SessionState::Discarded;
        let records = [
            record(
                "a",
                Some(root.join("Pathfinder/2026-10-02 Session 12")),
                None,
            ),
            record(
                "b",
                Some(root.join("Pathfinder/2026-10-03 Session 13")),
                None,
            ),
            record("c", Some(root.join("2026-10-07 0900 Standup")), None),
            // B deleted "Old" while this note was busy: it is leaving.
            record("e", Some(root.join("Old/2026-09-01 Retro")), Some("Old")),
            // One note of "Mixed" leaves and one stays.
            record("f", Some(root.join("Mixed/2026-09-02 A")), Some("Mixed")),
            record("g", Some(root.join("Mixed/2026-09-03 B")), None),
            record(
                "h",
                Some(PathBuf::from("/other/Pathfinder/2026-10-04 X")),
                None,
            ),
            record("i", None, None),
            discarded,
        ];
        let folders = StorageFolders::from_records(root, &records);
        assert_eq!(folders.homes, ["2026-10-07 0900 Standup"]);
        assert_eq!(
            folders.types,
            [
                type_folder("Pathfinder", 2, false),
                type_folder("Old", 1, true),
                type_folder("Mixed", 2, false),
            ]
        );
    }

    #[test]
    fn the_type_list_holds_shared_and_folder_types_and_the_read_writes_nothing() {
        // R2, R34, KTD9: the list read uses the view in memory only.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        manager
            .add_note_type("One on One", &StorageFolders::default())
            .unwrap();
        let path = file::settings_path(folder.path());
        let bytes = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

        let folders = StorageFolders {
            types: vec![type_folder("Customer Calls", 2, false)],
            homes: vec!["2026-10-07 0900 Standup".into()],
        };
        let list = manager.note_types(&folders);
        assert_eq!(
            type_names(&list),
            ["General", "Customer Calls", "One on One"]
        );
        // General is first: the notes at the top of the storage folder.
        assert_eq!(
            list.types[0],
            NoteTypeEntry {
                name: "General".into(),
                valid: true,
                shared: false,
                notes: 1,
                general: true,
                prompt_choice: None,
                prompt_name: None,
                summary_prompt_id: TOPIC_SUMMARY_ID.into(),
            }
        );
        assert_eq!(
            list.types[1],
            NoteTypeEntry {
                name: "Customer Calls".into(),
                valid: true,
                shared: false,
                notes: 2,
                general: false,
                prompt_choice: None,
                prompt_name: None,
                summary_prompt_id: TOPIC_SUMMARY_ID.into(),
            }
        );
        assert_eq!(list.default_type, None);
        assert!(!manager.reread());
        manager.note_types(&folders);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(file_of(folder.path())["note_types"], json!(["One on One"]));
    }

    #[test]
    fn an_add_saves_the_list_with_the_folder_types_and_refuses_a_bad_or_taken_name() {
        // R3, R28, KTD8, KTD9.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let folders = StorageFolders {
            types: vec![
                type_folder("Customer Calls", 1, false),
                type_folder("con", 1, false),
                type_folder("Old", 1, true),
            ],
            homes: vec!["2026-10-07 0900 Standup".into()],
        };
        assert_eq!(
            manager.add_note_type("  Pathfinder ", &folders).unwrap(),
            "Pathfinder"
        );
        assert_eq!(
            file_of(folder.path())["note_types"],
            json!(["Customer Calls", "Pathfinder"])
        );
        let before = std::fs::read(file::settings_path(folder.path())).unwrap();
        assert_eq!(
            manager.add_note_type("pathfinder", &folders).unwrap_err(),
            "A type with this name already exists."
        );
        assert_eq!(
            manager.add_note_type("1:1s", &folders).unwrap_err(),
            session_save::naming::TypeNameError::ForbiddenCharacter(':').to_string()
        );
        assert_eq!(
            manager
                .add_note_type("2026-10-07 0900 standup", &folders)
                .unwrap_err(),
            "A note folder in the storage folder has this name."
        );
        assert_eq!(
            std::fs::read(file::settings_path(folder.path())).unwrap(),
            before
        );
        // "con" is listed but is not valid, and "Old" is leaving.
        let list = manager.note_types(&folders);
        assert_eq!(
            type_names(&list),
            ["General", "con", "Customer Calls", "Pathfinder"]
        );
        assert!(!list.types[1].valid);
    }

    #[test]
    fn the_default_type_is_saved_and_general_clears_it() {
        // R7, R23.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let folders = StorageFolders {
            types: vec![type_folder("Customer Calls", 1, false)],
            homes: Vec::new(),
        };
        manager
            .add_note_type("Pathfinder", &StorageFolders::default())
            .unwrap();
        manager
            .set_default_note_type(Some("customer calls"), &folders)
            .unwrap();
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["default_note_type"], json!("Customer Calls"));
        assert_eq!(
            on_disk["note_types"],
            json!(["Customer Calls", "Pathfinder"])
        );
        assert_eq!(
            manager.note_types(&folders).default_type.as_deref(),
            Some("Customer Calls")
        );
        assert_eq!(
            manager
                .set_default_note_type(Some("Board"), &folders)
                .unwrap_err(),
            "That type no longer exists."
        );
        manager.set_default_note_type(None, &folders).unwrap();
        assert_eq!(file_of(folder.path())["default_note_type"], Value::Null);
        assert_eq!(manager.note_types(&folders).default_type, None);
        // The name of the built-in type also clears the default.
        manager
            .set_default_note_type(Some("Pathfinder"), &folders)
            .unwrap();
        manager
            .set_default_note_type(Some("general"), &folders)
            .unwrap();
        assert_eq!(file_of(folder.path())["default_note_type"], Value::Null);
    }

    #[test]
    fn a_folder_named_general_is_an_invalid_type_beside_the_built_in_general() {
        // A folder that a user made in Finder. It cannot be the built-in
        // type, so Settings offers only a rename or a delete for it.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        let folders = StorageFolders {
            types: vec![type_folder("General", 2, false)],
            homes: Vec::new(),
        };
        let list = manager.note_types(&folders);
        assert_eq!(type_names(&list), ["General", "General"]);
        assert!(list.types[0].general && list.types[0].valid);
        assert_eq!(list.types[0].notes, 0);
        assert!(!list.types[1].general && !list.types[1].valid);
        assert_eq!(list.types[1].notes, 2);
        assert_eq!(
            manager.add_note_type("general", &folders).unwrap_err(),
            "General is a built-in type."
        );
    }

    #[test]
    fn a_type_change_is_refused_and_writes_nothing_while_the_file_does_not_parse() {
        // Rule 4 of the shared settings rules.
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        manager
            .add_note_type("Pathfinder", &StorageFolders::default())
            .unwrap();
        let path = file::settings_path(folder.path());
        std::fs::write(&path, r#"{"note_types": ["Path"#).unwrap();
        manager.reread();
        // The list comes from the cache, and the read wrote nothing.
        assert_eq!(
            type_names(&manager.note_types(&StorageFolders::default())),
            ["General", "Pathfinder"]
        );
        assert!(
            manager
                .add_note_type("Board", &StorageFolders::default())
                .is_err()
        );
        assert!(
            manager
                .set_default_note_type(None, &StorageFolders::default())
                .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"note_types": ["Path"#
        );
    }

    #[test]
    fn a_change_of_folder_copies_the_types_and_the_default_into_a_folder_with_no_settings() {
        // KTD14.
        let old = TempDir::new().unwrap();
        let new = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(old.path()));
        manager.join_at_launch();
        manager
            .add_note_type("Pathfinder", &StorageFolders::default())
            .unwrap();
        manager
            .set_default_note_type(Some("Pathfinder"), &StorageFolders::default())
            .unwrap();

        manager.change_folder(Some(new.path().to_path_buf()));
        let on_disk = file_of(new.path());
        assert_eq!(on_disk["note_types"], json!(["Pathfinder"]));
        assert_eq!(on_disk["default_note_type"], json!("Pathfinder"));
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(type_names(&list), ["General", "Pathfinder"]);
        assert_eq!(list.default_type.as_deref(), Some("Pathfinder"));
    }

    #[test]
    fn a_change_of_folder_to_a_folder_with_settings_keeps_its_types() {
        // Rule 8: the folder's settings apply. This computer's types do not
        // go into a folder that has settings.
        let old = TempDir::new().unwrap();
        let shared = TempDir::new().unwrap();
        let computer_a = Computer::new();
        let a = computer_a.manager(Some(shared.path()));
        a.join_at_launch();
        a.add_note_type("Customer Calls", &StorageFolders::default())
            .unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(old.path()));
        manager.join_at_launch();
        manager
            .add_note_type("Pathfinder", &StorageFolders::default())
            .unwrap();

        manager.change_folder(Some(shared.path().to_path_buf()));
        assert_eq!(
            file_of(shared.path())["note_types"],
            json!(["Customer Calls"])
        );
        assert_eq!(
            type_names(&manager.note_types(&StorageFolders::default())),
            ["General", "Customer Calls"]
        );
    }

    /// A manager on a new storage folder with the types `names`.
    fn with_types(names: &[&str]) -> (TempDir, Computer, SharedSettingsManager) {
        let folder = TempDir::new().unwrap();
        let computer = Computer::new();
        let manager = computer.manager(Some(folder.path()));
        manager.join_at_launch();
        for name in names {
            manager
                .add_note_type(name, &StorageFolders::default())
                .unwrap();
        }
        (folder, computer, manager)
    }

    #[test]
    fn deleting_the_default_type_makes_the_default_general_in_the_same_save() {
        // Covers AE2 (the default) and R7.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        manager
            .set_default_note_type(Some("Pathfinder"), &StorageFolders::default())
            .unwrap();
        let recorded = std::cell::RefCell::new(Vec::new());
        let edit = manager
            .delete_note_type("pathfinder", &StorageFolders::default(), &|edit| {
                // The edit is recorded before the list changes (KTD7).
                assert_eq!(
                    file_of(folder.path())["note_types"],
                    json!(["Board", "Pathfinder"])
                );
                recorded.borrow_mut().push(edit.clone());
                Ok(())
            })
            .unwrap();
        assert_eq!(
            edit,
            BulkEdit::DeleteType {
                name: "Pathfinder".into()
            }
        );
        assert_eq!(recorded.into_inner(), [edit]);
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["note_types"], json!(["Board"]));
        assert_eq!(on_disk["default_note_type"], Value::Null);
        assert_eq!(
            manager.note_types(&StorageFolders::default()).default_type,
            None
        );
    }

    #[test]
    fn a_rename_keeps_the_default_on_the_renamed_type() {
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        manager
            .set_default_note_type(Some("Pathfinder"), &StorageFolders::default())
            .unwrap();
        let edit = manager
            .rename_note_type("Pathfinder", "D&D", &StorageFolders::default(), &|_| Ok(()))
            .unwrap();
        assert_eq!(
            edit,
            Some(BulkEdit::RenameType {
                from: "Pathfinder".into(),
                to: "D&D".into()
            })
        );
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["note_types"], json!(["Board", "D&D"]));
        assert_eq!(on_disk["default_note_type"], json!("D&D"));
    }

    #[test]
    fn a_rename_to_the_same_name_records_and_writes_nothing() {
        let (folder, _computer, manager) = with_types(&["Pathfinder"]);
        let before = std::fs::read_to_string(file::settings_path(folder.path())).unwrap();
        let edit = manager
            .rename_note_type(
                "Pathfinder",
                " Pathfinder ",
                &StorageFolders::default(),
                &|_| panic!("nothing to record"),
            )
            .unwrap();
        assert_eq!(edit, None);
        assert_eq!(
            std::fs::read_to_string(file::settings_path(folder.path())).unwrap(),
            before
        );
    }

    #[test]
    fn a_refused_record_changes_no_list() {
        let (folder, _computer, manager) = with_types(&["Pathfinder"]);
        assert!(
            manager
                .delete_note_type("Pathfinder", &StorageFolders::default(), &|_| Err(
                    "no record".into()
                ))
                .is_err()
        );
        assert_eq!(file_of(folder.path())["note_types"], json!(["Pathfinder"]));
    }

    #[test]
    fn a_continued_type_edit_writes_only_what_is_missing() {
        // KTD7: the quit came after the record and before the list save.
        let (folder, _computer, manager) = with_types(&["1 on 1", "One on One"]);
        manager
            .set_default_note_type(Some("1 on 1"), &StorageFolders::default())
            .unwrap();
        let merge = BulkEdit::RenameType {
            from: "1 on 1".into(),
            to: "One on One".into(),
        };
        manager.finish_type_edit(&merge).unwrap();
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["note_types"], json!(["One on One"]));
        assert_eq!(on_disk["default_note_type"], json!("One on One"));
        let before = std::fs::read_to_string(file::settings_path(folder.path())).unwrap();
        manager.finish_type_edit(&merge).unwrap();
        assert_eq!(
            std::fs::read_to_string(file::settings_path(folder.path())).unwrap(),
            before
        );
    }

    #[test]
    fn an_open_type_edit_keeps_its_old_folder_out_of_the_list() {
        // KTD7: the old folder does not come back as a type while its
        // moves are open.
        let mut folders = StorageFolders {
            types: vec![type_folder("Pathfinder", 2, false)],
            homes: Vec::new(),
        };
        folders.mark_leaving(["pathfinder"]);
        assert!(folders.types[0].leaving);
        let (_folder, _computer, manager) = with_types(&["D&D"]);
        assert_eq!(
            type_names(&manager.note_types(&folders)),
            ["General", "D&D"]
        );
    }

    // ------------------------------------------------- type prompt choices

    /// The entry of the type `name` in `list`.
    fn entry<'a>(list: &'a NoteTypeList, name: &str) -> &'a NoteTypeEntry {
        list.types
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("no type {name}"))
    }

    /// The prompt choice and the resolved prompt id of the type `name`.
    fn choice(list: &NoteTypeList, name: &str) -> (Option<String>, String) {
        let e = entry(list, name);
        (e.prompt_choice.clone(), e.summary_prompt_id.clone())
    }

    /// A custom prompt named `name` with the text `text`.
    fn custom_prompt(manager: &SharedSettingsManager, name: &str, text: &str) -> String {
        let id = manager.create_prompt().unwrap();
        manager.rename_prompt(&id, name).unwrap();
        manager.set_prompt_text(&id, text).unwrap();
        id
    }

    fn type_prompts_on_disk(root: &Path) -> Value {
        file_of(root)
            .get(file::KEY_NOTE_TYPE_PROMPTS)
            .cloned()
            .unwrap_or(json!({}))
    }

    #[test]
    fn a_prompt_choice_for_a_type_shows_in_the_type_list() {
        // R1, R2.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        let recap = custom_prompt(&manager, "Recap", "Recap text.");
        let list = manager
            .set_note_type_prompt("pathfinder", Some(&recap), &StorageFolders::default())
            .unwrap();
        let list_after = manager.note_types(&StorageFolders::default());
        assert_eq!(list, list_after);
        assert_eq!(
            choice(&list, "Pathfinder"),
            (Some(recap.clone()), recap.clone())
        );
        // A type with no choice uses the default prompt.
        assert_eq!(choice(&list, "Board"), (None, TOPIC_SUMMARY_ID.to_string()));
        assert_eq!(
            choice(&list, "General"),
            (None, TOPIC_SUMMARY_ID.to_string())
        );
        assert_eq!(
            type_prompts_on_disk(folder.path()),
            json!({ "Pathfinder": recap })
        );

        manager
            .set_note_type_prompt("General", Some(PLAY_BY_PLAY_ID), &StorageFolders::default())
            .unwrap();
        manager.activate_prompt(&recap).unwrap();
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(
            choice(&list, "General"),
            (Some(PLAY_BY_PLAY_ID.into()), PLAY_BY_PLAY_ID.into())
        );
        // "Default" follows the default prompt.
        assert_eq!(choice(&list, "Board"), (None, recap.clone()));

        // "Default" removes the entry.
        manager
            .set_note_type_prompt("Pathfinder", None, &StorageFolders::default())
            .unwrap();
        assert_eq!(
            type_prompts_on_disk(folder.path()),
            json!({ "General": PLAY_BY_PLAY_ID })
        );
        assert_eq!(
            manager
                .set_note_type_prompt("Pathfinder", Some("missing"), &StorageFolders::default())
                .unwrap_err(),
            NO_SUCH_PROMPT
        );
        assert_eq!(
            manager
                .set_note_type_prompt("Gone", Some(PLAY_BY_PLAY_ID), &StorageFolders::default())
                .unwrap_err(),
            "That type no longer exists."
        );
    }

    #[test]
    fn a_prompt_choice_for_a_folder_only_type_adds_the_type_to_the_list() {
        // KTD6.
        let (folder, _computer, manager) = with_types(&["Board"]);
        let folders = StorageFolders {
            types: vec![type_folder("Customer Calls", 2, false)],
            homes: Vec::new(),
        };
        let list = manager
            .set_note_type_prompt("customer calls", Some(PLAY_BY_PLAY_ID), &folders)
            .unwrap();
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["note_types"], json!(["Board", "Customer Calls"]));
        assert_eq!(
            on_disk[file::KEY_NOTE_TYPE_PROMPTS],
            json!({ "Customer Calls": PLAY_BY_PLAY_ID })
        );
        assert!(entry(&list, "Customer Calls").shared);
        assert_eq!(
            choice(&list, "Customer Calls"),
            (Some(PLAY_BY_PLAY_ID.into()), PLAY_BY_PLAY_ID.into())
        );
    }

    #[test]
    fn a_type_with_an_invalid_name_or_a_prompt_never_read_shows_default() {
        // R7, R18.
        let (_folder, _computer, manager) = with_types(&["Pathfinder"]);
        manager
            .save(&[SharedValue::NoteTypePrompts(
                [
                    ("Pathfinder".to_string(), "never-read".to_string()),
                    ("1:1s".to_string(), PLAY_BY_PLAY_ID.to_string()),
                ]
                .into(),
            )])
            .unwrap();
        let folders = StorageFolders {
            types: vec![type_folder("1:1s", 1, false)],
            homes: Vec::new(),
        };
        let list = manager.note_types(&folders);
        assert_eq!(
            choice(&list, "Pathfinder"),
            (None, TOPIC_SUMMARY_ID.to_string())
        );
        assert_eq!(choice(&list, "1:1s"), (None, TOPIC_SUMMARY_ID.to_string()));
        let settings = settings_of(&manager);
        assert_eq!(
            settings.summary_prompt_for_type(Some("Pathfinder")).id,
            TOPIC_SUMMARY_ID
        );
        assert_eq!(
            settings.summary_prompt_for_type(Some("1:1s")).id,
            TOPIC_SUMMARY_ID
        );
    }

    #[test]
    fn deleting_a_prompt_that_a_type_uses_resets_the_type_to_default() {
        // AE5, R5: one save resets the type and the active prompt.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        let recap = custom_prompt(&manager, "Recap", "Recap text.");
        manager.activate_prompt(&recap).unwrap();
        manager
            .set_note_type_prompt("Pathfinder", Some(&recap), &StorageFolders::default())
            .unwrap();
        manager
            .set_note_type_prompt("Board", Some(PLAY_BY_PLAY_ID), &StorageFolders::default())
            .unwrap();

        manager.delete_prompt(&recap).unwrap();
        let on_disk = file_of(folder.path());
        assert_eq!(
            on_disk[file::KEY_NOTE_TYPE_PROMPTS],
            json!({ "Board": PLAY_BY_PLAY_ID })
        );
        assert_eq!(on_disk["active_summary_prompt"], TOPIC_SUMMARY_ID);
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(
            choice(&list, "Pathfinder"),
            (None, TOPIC_SUMMARY_ID.to_string())
        );
    }

    #[test]
    fn a_rename_moves_the_choice_and_a_merge_keeps_the_target_choice() {
        // AE6, R6.
        let (folder, _computer, manager) = with_types(&["Work", "Meetings", "Board"]);
        let decisions = custom_prompt(&manager, "Decisions", "Decisions text.");
        manager
            .set_note_type_prompt("Work", Some(&decisions), &StorageFolders::default())
            .unwrap();
        manager
            .set_note_type_prompt("Board", Some(PLAY_BY_PLAY_ID), &StorageFolders::default())
            .unwrap();

        manager
            .rename_note_type("Work", "Meetings", &StorageFolders::default(), &|_| Ok(()))
            .unwrap();
        assert_eq!(
            type_prompts_on_disk(folder.path()),
            json!({ "Board": PLAY_BY_PLAY_ID })
        );
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(choice(&list, "Meetings").0, None);

        manager
            .rename_note_type(
                "Board",
                "Directors",
                &StorageFolders::default(),
                &|_| Ok(()),
            )
            .unwrap();
        assert_eq!(
            type_prompts_on_disk(folder.path()),
            json!({ "Directors": PLAY_BY_PLAY_ID })
        );

        manager
            .delete_note_type("Directors", &StorageFolders::default(), &|_| Ok(()))
            .unwrap();
        assert_eq!(type_prompts_on_disk(folder.path()), json!({}));
    }

    #[test]
    fn the_launch_replay_of_a_type_edit_moves_the_choice_too() {
        // KTD6: the quit came after the record and before the list save.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Work", "Meetings"]);
        manager
            .save(&[SharedValue::NoteTypePrompts(
                [
                    ("Pathfinder".to_string(), PLAY_BY_PLAY_ID.to_string()),
                    ("Work".to_string(), PLAY_BY_PLAY_ID.to_string()),
                ]
                .into(),
            )])
            .unwrap();
        manager
            .finish_type_edit(&BulkEdit::RenameType {
                from: "Pathfinder".into(),
                to: "D&D".into(),
            })
            .unwrap();
        manager
            .finish_type_edit(&BulkEdit::RenameType {
                from: "Work".into(),
                to: "Meetings".into(),
            })
            .unwrap();
        let on_disk = file_of(folder.path());
        assert_eq!(on_disk["note_types"], json!(["Meetings", "D&D"]));
        assert_eq!(
            on_disk[file::KEY_NOTE_TYPE_PROMPTS],
            json!({ "D&D": PLAY_BY_PLAY_ID })
        );
    }

    #[test]
    fn adding_a_type_removes_an_old_choice_with_its_name() {
        // KTD6: a new type starts with "Default" (R1).
        let (folder, _computer, manager) = with_types(&[]);
        manager
            .save(&[SharedValue::NoteTypePrompts(
                [("Pathfinder".to_string(), PLAY_BY_PLAY_ID.to_string())].into(),
            )])
            .unwrap();
        manager
            .add_note_type("pathfinder", &StorageFolders::default())
            .unwrap();
        assert_eq!(type_prompts_on_disk(folder.path()), json!({}));
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(choice(&list, "pathfinder").0, None);
    }

    #[test]
    fn a_prompt_choice_is_refused_while_the_settings_are_read_only() {
        // R19 of the pane, rule 4 of the shared settings rules.
        let (folder, _computer, manager) = with_types(&["Pathfinder"]);
        let path = file::settings_path(folder.path());
        std::fs::write(&path, r#"{"note_types": ["Path"#).unwrap();
        manager.reread();
        assert_eq!(
            manager
                .set_note_type_prompt(
                    "Pathfinder",
                    Some(PLAY_BY_PLAY_ID),
                    &StorageFolders::default()
                )
                .unwrap_err(),
            ReadOnlyCause::DoesNotParse.message()
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            r#"{"note_types": ["Path"#
        );
    }

    #[test]
    fn the_prompt_of_a_note_type_follows_its_choice() {
        // R3, R7, KTD2.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        let recap = custom_prompt(&manager, "Recap", "Recap text.");
        let standup = custom_prompt(&manager, "Standup", "Standup text.");
        manager.activate_prompt(&standup).unwrap();
        manager
            .set_note_type_prompt("Pathfinder", Some(&recap), &StorageFolders::default())
            .unwrap();
        manager
            .set_note_type_prompt("General", Some(PLAY_BY_PLAY_ID), &StorageFolders::default())
            .unwrap();
        let settings = settings_of(&manager);
        assert_eq!(
            settings.summary_prompt_for_type(Some("pathfinder")).text,
            "Recap text."
        );
        // A type with no entry uses the default prompt.
        assert_eq!(
            settings.summary_prompt_for_type(Some("Board")).text,
            "Standup text."
        );
        assert_eq!(settings.summary_prompt_for_type(None).id, PLAY_BY_PLAY_ID);

        // The file of the prompt goes away: the last copy read applies.
        let file_name = prompt_files(folder.path())
            .into_iter()
            .find(|name| name.starts_with("Recap"))
            .unwrap();
        std::fs::remove_file(prompts::prompts_folder(folder.path()).join(file_name)).unwrap();
        manager.reread();
        let settings = settings_of(&manager);
        assert!(settings.summary_prompts.iter().all(|p| p.id != recap));
        assert_eq!(
            settings.summary_prompt_for_type(Some("Pathfinder")).text,
            "Recap text."
        );
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(choice(&list, "Pathfinder"), (Some(recap.clone()), recap));
    }

    #[test]
    fn the_type_list_names_the_chosen_prompt_also_when_its_file_is_missing() {
        // R7: Settings shows the prompt that a summary of the type uses.
        let (folder, _computer, manager) = with_types(&["Pathfinder", "Board"]);
        let recap = custom_prompt(&manager, "Recap", "Recap text.");
        manager
            .set_note_type_prompt("Pathfinder", Some(&recap), &StorageFolders::default())
            .unwrap();
        let file_name = prompt_files(folder.path())
            .into_iter()
            .find(|name| name.starts_with("Recap"))
            .unwrap();
        std::fs::remove_file(prompts::prompts_folder(folder.path()).join(file_name)).unwrap();
        manager.reread();
        let list = manager.note_types(&StorageFolders::default());
        assert_eq!(
            entry(&list, "Pathfinder").prompt_name.as_deref(),
            Some("Recap")
        );
        assert_eq!(entry(&list, "Board").prompt_name, None);
    }
}
