//! The shared view: the shared settings and the prompts that this computer
//! uses now, from the storage folder or from the local cache (R16, KTD6).
//!
//! A load reads the storage folder and never writes into it. When the
//! folder is available, the view comes from its files, and the load gives
//! a new cache when the result differs from the old cache. When the folder
//! is missing or not reachable, the view comes from the cache and is
//! read-only. When the settings file exists and does not parse, the
//! settings come from the cache and are read-only, and the prompts and the
//! Background come from the folder and can change (R22).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use summarize::prompts::Prompt;

use crate::background::{BackgroundRead, BackgroundUnavailable, read_background};
use crate::cache::{Cache, read_cache, write_cache};
use crate::file::{
    AudioRemoval, Field, FileRead, ParseError, SettingsFile, SharedFields, Unavailable,
    default_settings, read_settings,
};
use crate::prompts::{PromptScan, scan_prompts};

/// Whether this computer can change the shared settings now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// The view comes from the storage folder, and a save can write.
    Available,
    /// The storage folder exists and has no settings file. The view comes
    /// from the cache until a join fills the folder (KTD8).
    MissingFile,
    /// The view comes from the cache and is read-only.
    NotAvailable(Unavailable),
}

/// The prompt that the next summary uses: the active prompt, or the prompt
/// of a note type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivePrompt {
    BuiltIn(Prompt),
    Custom {
        prompt: Prompt,
        /// The text is the last copy that this computer read, because the
        /// prompt's file is missing or has a conflict copy (R14).
        from_cache: bool,
    },
    /// No prompt has this id, and the cache has no copy of it. The app
    /// uses the Topic summary. A value that is not an id shows as its JSON
    /// text.
    Missing(String),
}

impl ActivePrompt {
    /// The prompt, when one exists.
    pub fn prompt(&self) -> Option<&Prompt> {
        match self {
            ActivePrompt::BuiltIn(prompt) | ActivePrompt::Custom { prompt, .. } => Some(prompt),
            ActivePrompt::Missing(_) => None,
        }
    }
}

/// The shared settings that this computer uses now.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedView {
    pub availability: Availability,
    /// The whole settings object.
    pub raw: Map<String, Value>,
    pub fields: SharedFields,
    /// The custom prompts, in the order that Settings shows them.
    pub prompts: Vec<Prompt>,
    pub active: ActivePrompt,
    /// The prompt of each id that the type prompts field names, by id, in
    /// the same order of sources as the active prompt (KTD2). A prompt
    /// that this computer never read is not here. Find the prompt of a
    /// type with [`SharedView::type_prompt`].
    pub type_prompts: BTreeMap<String, ActivePrompt>,
    /// The Background text that summaries use. It is the cached text when
    /// the view comes from the cache or the Background file cannot be
    /// used (KTD3).
    pub background: String,
    /// Why the Background file cannot be used, when the rest of the view
    /// comes from the storage folder. The Background is then read-only.
    pub background_unavailable: Option<BackgroundUnavailable>,
    /// The prompts and the Background come from the storage folder, and a
    /// change of a prompt or of the Background can write. This is also
    /// true while the settings file does not parse (R22).
    pub files_writable: bool,
    /// The custom prompts that the settings name, that the folder does not
    /// have, and that this computer read before (R20, KTD8). A summary uses
    /// the default prompt for them. Empty unless the view comes from a
    /// settings file that parsed.
    pub missing_prompts: BTreeSet<String>,
    /// The ids of the prompts whose files are over the size limit of the
    /// scan. Such a prompt shows with no text (R21).
    pub too_large_prompts: BTreeSet<String>,
}

impl SharedView {
    /// Whether a save of the settings file can write now. The shared
    /// settings are read-only in every other case (R16).
    pub fn writable(&self) -> bool {
        self.availability == Availability::Available
    }

    /// Whether a save of the Background can write now (R5, KTD3).
    pub fn background_writable(&self) -> bool {
        self.files_writable && self.background_unavailable.is_none()
    }

    /// The settings file exists and does not parse (R22). The settings come
    /// from the cache, and the prompts and the Background from the folder.
    pub fn parse_error(&self) -> Option<&ParseError> {
        match &self.availability {
            Availability::NotAvailable(Unavailable::DoesNotParse(error)) => Some(error),
            _ => None,
        }
    }

    /// The delay in whole days after which the removal pass removes the
    /// audio of a saved note. `None` means that no audio goes
    /// automatically: the setting is off, it has a value that this build
    /// does not know, or the view does not come from a settings file that
    /// parsed (R2, AE10, KTD3). A view from the cache or from the defaults
    /// is only for display. An absent field in a file that parsed is the
    /// default.
    pub fn audio_removal_days(&self) -> Option<u32> {
        if self.availability != Availability::Available {
            return None;
        }
        match &self.fields.audio_removal {
            Field::Absent => AudioRemoval::default().days(),
            Field::Known(removal) => removal.days(),
            Field::Unknown(_) => None,
        }
    }
}

/// The time after which a write that a user change causes clears the
/// settings that name a missing prompt (R20). The wait keeps the settings
/// while a sync service delivers a rename as a delete and then a create.
pub const MISSING_PROMPT_WAIT: Duration = Duration::from_secs(60);

/// When each missing prompt of the views went missing (R20). The app keeps
/// this in memory only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MissingSince {
    since: BTreeMap<String, Instant>,
}

impl MissingSince {
    /// Record the missing prompts of `view` at `now`. A view that does not
    /// come from a settings file that parsed changes nothing.
    pub fn update(&mut self, view: &SharedView, now: Instant) {
        if !view.writable() {
            return;
        }
        self.since.retain(|id, _| view.missing_prompts.contains(id));
        for id in &view.missing_prompts {
            self.since.entry(id.clone()).or_insert(now);
        }
    }

    /// The prompts that are missing since `wait` or longer at `now`.
    pub fn due(&self, now: Instant, wait: Duration) -> BTreeSet<String> {
        self.since
            .iter()
            .filter(|(_, since)| now.saturating_duration_since(**since) >= wait)
            .map(|(id, _)| id.clone())
            .collect()
    }
}

/// The result of a load.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub view: SharedView,
    /// The cache to write, when it differs from the old cache.
    pub cache: Option<Cache>,
}

/// Read the shared settings of the storage folder `root`. `cache` is the
/// last cache of this computer. This never writes.
///
/// A cache from another folder, or from the local store of a version
/// before shared settings, can hold prompts that the folder does not have.
/// Only a join merges them (R21, R22), so this load gives no new cache for
/// it.
pub fn load(root: &Path, cache: Option<&Cache>) -> Loaded {
    let mut loaded = load_after_join(root, cache);
    if cache.is_some_and(|c| c.source != root) {
        loaded.cache = None;
    }
    loaded
}

/// [`load`] after a join that merged `cache` into `root`. The new cache
/// replaces a cache from any folder.
pub(crate) fn load_after_join(root: &Path, cache: Option<&Cache>) -> Loaded {
    let (file, parse_error) = match read_settings(root) {
        FileRead::Parsed(file) => (file, None),
        FileRead::Missing => return from_cache(cache, Availability::MissingFile),
        // The settings come from the cache, and the prompt files and the
        // Background file from the folder (R22). A cache from another
        // folder waits for the join, so the whole view comes from it.
        FileRead::NotAvailable(Unavailable::DoesNotParse(error))
            if cache.is_none_or(|c| c.source == root) =>
        {
            let raw = cache.map_or_else(default_settings, |c| c.settings.clone());
            (SettingsFile::from_raw(raw), Some(error))
        }
        FileRead::NotAvailable(cause) => {
            return from_cache(cache, Availability::NotAvailable(cause));
        }
    };
    let scan = match scan_prompts(root) {
        Ok(scan) => scan,
        Err(cause) => return from_cache(cache, Availability::NotAvailable(cause)),
    };
    let active = active_prompt(&file.fields.active_summary_prompt, Some(&scan), cache);
    let type_prompts = type_prompts(&file.fields.note_type_prompts, Some(&scan), cache);
    let prompts: Vec<Prompt> = scan.prompts.iter().map(Prompt::from).collect();
    let too_large_prompts: BTreeSet<String> = scan
        .prompts
        .iter()
        .filter(|p| p.too_large)
        .map(|p| p.id.clone())
        .collect();
    let missing = missing_prompts(&file.fields, &scan, cache);
    // A missing file in a folder with a settings file is an empty
    // Background. A file that cannot be used keeps the cached text.
    let (background, background_unavailable) = match read_background(root) {
        BackgroundRead::Text(text) => (text, None),
        BackgroundRead::Missing => (String::new(), None),
        BackgroundRead::NotAvailable(cause) => (cached_background(cache), Some(cause)),
    };
    // The cache keeps the last copy of a missing prompt that the settings
    // name, so this computer knows later that it read the prompt (KTD8). A
    // prompt with no text in the scan is not a copy.
    let kept = |id: &str| {
        missing
            .contains(id)
            .then(|| cached_prompt(cache, id))
            .flatten()
    };
    let active_copy = match &file.fields.active_summary_prompt {
        Field::Known(id) => custom_prompt(&active).or_else(|| kept(id)),
        _ => custom_prompt(&active),
    };
    let mut type_copies: Vec<Prompt> = type_prompts.values().filter_map(custom_prompt).collect();
    for id in file.fields.type_prompt_map().values() {
        if !type_copies.iter().any(|p| p.id == *id)
            && let Some(copy) = kept(id)
        {
            type_copies.push(copy);
        }
    }
    let new_cache = Cache {
        source: root.to_path_buf(),
        settings: file.raw.clone(),
        prompts: prompts
            .iter()
            .filter(|p| !too_large_prompts.contains(&p.id))
            .cloned()
            .collect(),
        active_prompt: active_copy,
        type_prompts: type_copies,
        background: background.clone(),
    };
    let availability = match parse_error {
        None => Availability::Available,
        Some(error) => Availability::NotAvailable(Unavailable::DoesNotParse(error)),
    };
    let from_folder = availability == Availability::Available;
    let view = SharedView {
        availability,
        raw: file.raw,
        fields: file.fields,
        prompts,
        active,
        type_prompts,
        background,
        background_unavailable,
        files_writable: true,
        // Only a view of a settings file that parsed names the missing
        // prompts: the repair writes the settings file (R20).
        missing_prompts: if from_folder {
            missing
        } else {
            BTreeSet::new()
        },
        too_large_prompts,
    };
    // While the settings file does not parse, the cache stays as it is.
    let changed = from_folder && cache != Some(&new_cache);
    Loaded {
        view,
        cache: changed.then_some(new_cache),
    }
}

/// The custom prompts that the settings `fields` name and the folder of
/// `scan` does not have, of which `cache` has a copy: this computer read
/// them before (R20, KTD8). A prompt that this computer never read is not
/// here, so this computer never clears the settings that name it.
fn missing_prompts(
    fields: &SharedFields,
    scan: &PromptScan,
    cache: Option<&Cache>,
) -> BTreeSet<String> {
    fields
        .active_summary_prompt
        .known()
        .into_iter()
        .chain(fields.type_prompt_map().values())
        .filter(|id| summarize::prompts::built_in(id).is_none())
        .filter(|id| scan.is_missing(id) && cached_prompt(cache, id).is_some())
        .cloned()
        .collect()
}

/// The custom prompt of `prompt`, for the cache. A built-in prompt and a
/// missing prompt give `None`.
fn custom_prompt(prompt: &ActivePrompt) -> Option<Prompt> {
    match prompt {
        ActivePrompt::Custom { prompt, .. } => Some(prompt.clone()),
        ActivePrompt::BuiltIn(_) | ActivePrompt::Missing(_) => None,
    }
}

/// [`load`] with the cache in the folder `cache_dir`: read the cache, load,
/// and write the cache when it changed. A failed cache write is logged:
/// the view is still correct.
pub fn refresh(root: &Path, cache_dir: &Path) -> SharedView {
    let old = read_cache(cache_dir);
    let loaded = load(root, old.as_ref());
    store(cache_dir, loaded.cache.as_ref());
    loaded.view
}

/// The view from `cache`, or from the defaults when there is no cache,
/// with `availability`. This reads no folder. The app uses it before the
/// first read of the storage folder, and while no storage folder is set.
pub fn cached(cache: Option<&Cache>, availability: Availability) -> SharedView {
    from_cache(cache, availability).view
}

/// Write `cache` into `cache_dir`, when there is one.
pub fn store(cache_dir: &Path, cache: Option<&Cache>) {
    if let Some(cache) = cache
        && let Err(e) = write_cache(cache_dir, cache)
    {
        log::warn!(
            "shared settings: cannot write the cache in {}: {e}",
            cache_dir.display()
        );
    }
}

/// The view from the cache, or from the defaults when there is no cache.
/// It is read-only, and it gives no new cache.
fn from_cache(cache: Option<&Cache>, availability: Availability) -> Loaded {
    let raw = cache.map_or_else(default_settings, |c| c.settings.clone());
    let fields = SharedFields::from_raw(&raw);
    let prompts = cache.map(|c| c.prompts.clone()).unwrap_or_default();
    let active = active_prompt(&fields.active_summary_prompt, None, cache);
    let type_prompts = type_prompts(&fields.note_type_prompts, None, cache);
    Loaded {
        view: SharedView {
            availability,
            raw,
            fields,
            prompts,
            active,
            type_prompts,
            background: cached_background(cache),
            background_unavailable: None,
            files_writable: false,
            missing_prompts: BTreeSet::new(),
            too_large_prompts: BTreeSet::new(),
        },
        cache: None,
    }
}

/// The Background text of `cache`, or an empty text when there is no
/// cache.
fn cached_background(cache: Option<&Cache>) -> String {
    cache.map(|c| c.background.clone()).unwrap_or_default()
}

/// The last copy of the prompt `id` in the cache.
fn cached_prompt(cache: Option<&Cache>, id: &str) -> Option<Prompt> {
    let cache = cache?;
    cache
        .active_prompt
        .iter()
        .chain(&cache.type_prompts)
        .chain(&cache.prompts)
        .find(|p| p.id == id)
        .cloned()
}

/// The active prompt that the active field names. An absent field is the
/// Topic summary. A value that is not an id is missing.
fn active_prompt(
    field: &Field<String>,
    scan: Option<&PromptScan>,
    cache: Option<&Cache>,
) -> ActivePrompt {
    match field {
        Field::Absent => ActivePrompt::BuiltIn(summarize::prompts::topic_summary()),
        Field::Known(id) => resolve(id, scan, cache),
        Field::Unknown(value) => ActivePrompt::Missing(value.to_string()),
    }
}

/// The prompt with the id `id` (KTD2, KTD8), in this order: the built-in
/// prompt, the folder copy when it has no conflict copy and has its text,
/// the last copy in the cache, the folder copy with a conflict copy, and
/// else missing. A prompt whose file is missing is missing at once: the
/// cache copy stands in only for a file that exists and cannot be used now
/// (R20). `scan` is `None` when the view comes from the cache.
fn resolve(id: &str, scan: Option<&PromptScan>, cache: Option<&Cache>) -> ActivePrompt {
    if let Some(prompt) = summarize::prompts::built_in(id) {
        return ActivePrompt::BuiltIn(prompt);
    }
    if scan.is_some_and(|scan| scan.is_missing(id)) {
        return ActivePrompt::Missing(id.to_string());
    }
    let in_folder = scan.and_then(|scan| scan.get(id));
    let conflicted = scan.is_some_and(|scan| scan.conflicted.contains(id));
    // The scan has no text of a file over the size limit.
    let in_folder = in_folder.filter(|p| !p.too_large).map(Prompt::from);
    // While the id has a conflict copy, or its file cannot be used, the
    // text comes from the cache (R14).
    if !conflicted && let Some(prompt) = &in_folder {
        return ActivePrompt::Custom {
            prompt: prompt.clone(),
            from_cache: false,
        };
    }
    if let Some(prompt) = cached_prompt(cache, id) {
        return ActivePrompt::Custom {
            prompt,
            from_cache: true,
        };
    }
    match in_folder {
        Some(prompt) => ActivePrompt::Custom {
            prompt,
            from_cache: false,
        },
        None => ActivePrompt::Missing(id.to_string()),
    }
}

/// The prompt of each id that the type prompts field names, when the
/// prompt exists (R7, KTD2). A missing prompt is not in the map, so its
/// types use the default prompt.
fn type_prompts(
    field: &Field<BTreeMap<String, String>>,
    scan: Option<&PromptScan>,
    cache: Option<&Cache>,
) -> BTreeMap<String, ActivePrompt> {
    let Some(map) = field.known() else {
        return BTreeMap::new();
    };
    map.values()
        .filter_map(|id| {
            let prompt = resolve(id, scan, cache);
            prompt.prompt().is_some().then(|| (id.clone(), prompt))
        })
        .collect()
}
