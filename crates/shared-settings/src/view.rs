//! The shared view: the shared settings and the prompts that this computer
//! uses now, from the storage folder or from the local cache (R16, KTD6).
//!
//! A load reads the storage folder and never writes into it. When the
//! folder is available, the view comes from its files, and the load gives
//! a new cache when the result differs from the old cache. When the folder
//! is missing, not reachable, or has a settings file that does not parse,
//! the view comes from the cache and is read-only.

use std::path::Path;

use serde_json::{Map, Value};
use summarize::prompts::Prompt;

use crate::cache::{Cache, read_cache, write_cache};
use crate::file::{Field, FileRead, SharedFields, Unavailable, default_settings, read_settings};
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

/// The prompt that the next summary uses.
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
}

impl SharedView {
    /// Whether a save or a change of a prompt can write now. The shared
    /// settings are read-only in every other case (R16).
    pub fn writable(&self) -> bool {
        self.availability == Availability::Available
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
    let active = active_from_folder(&file.fields.active_summary_prompt, &scan, cache);
    let prompts: Vec<Prompt> = scan.prompts.iter().map(Prompt::from).collect();
    let new_cache = Cache {
        source: root.to_path_buf(),
        settings: file.raw.clone(),
        prompts: prompts.clone(),
        active_prompt: match &active {
            ActivePrompt::Custom { prompt, .. } => Some(prompt.clone()),
            ActivePrompt::BuiltIn(_) | ActivePrompt::Missing(_) => None,
        },
    };
    let view = SharedView {
        availability: Availability::Available,
        raw: file.raw,
        fields: file.fields,
        prompts,
        active,
    };
    let changed = cache != Some(&new_cache);
    Loaded {
        view,
        cache: changed.then_some(new_cache),
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
    let active = active_from_cache(&fields.active_summary_prompt, cache);
    Loaded {
        view: SharedView {
            availability,
            raw,
            fields,
            prompts,
            active,
        },
        cache: None,
    }
}

/// The id that the active field names. An absent field is the default.
enum ActiveId<'a> {
    BuiltIn(Prompt),
    Custom(&'a str),
    Missing(String),
}

fn active_id(field: &Field<String>) -> ActiveId<'_> {
    match field {
        Field::Absent => ActiveId::BuiltIn(summarize::prompts::topic_summary()),
        Field::Known(id) => match summarize::prompts::built_in(id) {
            Some(prompt) => ActiveId::BuiltIn(prompt),
            None => ActiveId::Custom(id),
        },
        Field::Unknown(value) => ActiveId::Missing(value.to_string()),
    }
}

/// The last copy of the prompt `id` in the cache.
fn cached_prompt(cache: Option<&Cache>, id: &str) -> Option<Prompt> {
    let cache = cache?;
    cache
        .active_prompt
        .iter()
        .chain(&cache.prompts)
        .find(|p| p.id == id)
        .cloned()
}

fn active_from_folder(
    field: &Field<String>,
    scan: &PromptScan,
    cache: Option<&Cache>,
) -> ActivePrompt {
    let id = match active_id(field) {
        ActiveId::BuiltIn(prompt) => return ActivePrompt::BuiltIn(prompt),
        ActiveId::Missing(text) => return ActivePrompt::Missing(text),
        ActiveId::Custom(id) => id,
    };
    let in_folder = scan.get(id).map(Prompt::from);
    // While the id has a conflict copy, or its file is missing, the text
    // comes from the cache (R14).
    if let Some(prompt) = in_folder.clone().filter(|_| !scan.conflicted.contains(id)) {
        return ActivePrompt::Custom {
            prompt,
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

fn active_from_cache(field: &Field<String>, cache: Option<&Cache>) -> ActivePrompt {
    match active_id(field) {
        ActiveId::BuiltIn(prompt) => ActivePrompt::BuiltIn(prompt),
        ActiveId::Missing(text) => ActivePrompt::Missing(text),
        ActiveId::Custom(id) => match cached_prompt(cache, id) {
            Some(prompt) => ActivePrompt::Custom {
                prompt,
                from_cache: true,
            },
            None => ActivePrompt::Missing(id.to_string()),
        },
    }
}
