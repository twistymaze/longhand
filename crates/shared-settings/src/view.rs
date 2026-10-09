//! The shared view: the shared settings and the prompts that this computer
//! uses now, from the storage folder or from the local cache (R16, KTD6).
//!
//! A load reads the storage folder and never writes into it. When the
//! folder is available, the view comes from its files, and the load gives
//! a new cache when the result differs from the old cache. When the folder
//! is missing, not reachable, or has a settings file that does not parse,
//! the view comes from the cache and is read-only.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Map, Value};
use summarize::prompts::Prompt;

use crate::background::{BackgroundRead, BackgroundUnavailable, read_background};
use crate::cache::{Cache, read_cache, write_cache};
use crate::file::{
    AudioRemoval, Field, FileRead, SharedFields, Unavailable, default_settings, read_settings,
};
use crate::note_types::{NoteType, type_prompt_id};
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
}

impl SharedView {
    /// Whether a save or a change of a prompt can write now. The shared
    /// settings are read-only in every other case (R16).
    pub fn writable(&self) -> bool {
        self.availability == Availability::Available
    }

    /// The prompt that the note type `note_type` chooses, or `None` when
    /// the type uses the default prompt (R1, R3, R7). `None` and "General"
    /// are General, and `types` is the effective type list. A choice that
    /// names a prompt that this computer never read is "Default".
    pub fn type_prompt(
        &self,
        note_type: Option<&str>,
        types: &[NoteType],
    ) -> Option<&ActivePrompt> {
        let map = self.fields.note_type_prompts.known()?;
        self.type_prompts
            .get(type_prompt_id(map, types, note_type)?)
    }

    /// Whether a save of the Background can write now (R5, KTD3).
    pub fn background_writable(&self) -> bool {
        self.writable() && self.background_unavailable.is_none()
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
    let file = match read_settings(root) {
        FileRead::Parsed(file) => file,
        FileRead::Missing => return from_cache(cache, Availability::MissingFile),
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
    // A missing file in a folder with a settings file is an empty
    // Background. A file that cannot be used keeps the cached text.
    let (background, background_unavailable) = match read_background(root) {
        BackgroundRead::Text(text) => (text, None),
        BackgroundRead::Missing => (String::new(), None),
        BackgroundRead::NotAvailable(cause) => (cached_background(cache), Some(cause)),
    };
    let new_cache = Cache {
        source: root.to_path_buf(),
        settings: file.raw.clone(),
        prompts: prompts.clone(),
        active_prompt: custom_prompt(&active),
        type_prompts: type_prompts.values().filter_map(custom_prompt).collect(),
        background: background.clone(),
    };
    let view = SharedView {
        availability: Availability::Available,
        raw: file.raw,
        fields: file.fields,
        prompts,
        active,
        type_prompts,
        background,
        background_unavailable,
    };
    let changed = cache != Some(&new_cache);
    Loaded {
        view,
        cache: changed.then_some(new_cache),
    }
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

/// The prompt with the id `id` (KTD2), in this order: the built-in
/// prompt, the folder copy when it has no conflict copy, the last copy in
/// the cache, the folder copy with a conflict copy, and else missing.
/// `scan` is `None` when the view comes from the cache.
fn resolve(id: &str, scan: Option<&PromptScan>, cache: Option<&Cache>) -> ActivePrompt {
    if let Some(prompt) = summarize::prompts::built_in(id) {
        return ActivePrompt::BuiltIn(prompt);
    }
    let in_folder = scan.and_then(|scan| scan.get(id)).map(Prompt::from);
    let conflicted = scan.is_some_and(|scan| scan.conflicted.contains(id));
    // While the id has a conflict copy, or its file is missing, the text
    // comes from the cache (R14).
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
