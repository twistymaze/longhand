//! The local cache of the shared settings (R16, KTD6).
//!
//! Each computer keeps a copy of the last shared settings file that
//! parsed, the prompts with their ids, names and texts, and the storage
//! folder that they came from. When the storage folder is missing or not
//! reachable, the shared settings come from this copy, not from the
//! defaults. The cache is in a local folder, so its write never breaks the
//! rule that a read of the storage folder never writes into it.

use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use summarize::prompts::Prompt;

/// The name of the cache file in the cache folder.
pub const CACHE_FILE: &str = "shared-settings.json";

/// The last shared settings that this computer read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// The storage folder that the settings came from. A join fills only
    /// this folder from the cache, unless the user changes the folder.
    pub source: PathBuf,
    /// The whole settings object, with the fields that this build does
    /// not know.
    pub settings: Map<String, Value>,
    /// The custom prompts, as Settings showed them.
    pub prompts: Vec<Prompt>,
    /// The last text of the active custom prompt that this computer read.
    /// It stays while the file of that prompt is missing or has a conflict
    /// copy (R14).
    #[serde(default)]
    pub active_prompt: Option<Prompt>,
    /// The last texts of the custom prompts that the note types choose,
    /// as this computer read them. They stay while the file of a prompt is
    /// missing or has a conflict copy, as the active prompt does (R7,
    /// KTD2).
    #[serde(default)]
    pub type_prompts: Vec<Prompt>,
    /// The last Background text that this computer read. Summaries use it
    /// while the Background file cannot be used (KTD3). A cache from a
    /// version before the Background has an empty text.
    #[serde(default)]
    pub background: String,
}

/// Read the cache in `dir`. A cache that is missing or does not parse
/// gives `None`.
pub fn read_cache(dir: &Path) -> Option<Cache> {
    let path = dir.join(CACHE_FILE);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            log::warn!(
                "shared settings: cannot read the cache {}: {e}",
                path.display()
            );
            return None;
        }
    };
    match serde_json::from_slice(&bytes) {
        Ok(cache) => Some(cache),
        Err(e) => {
            log::warn!(
                "shared settings: the cache {} does not parse: {e}",
                path.display()
            );
            None
        }
    }
}

/// Replace the cache in `dir` with `cache`, and make `dir` when it does not
/// exist.
pub fn write_cache(dir: &Path, cache: &Cache) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let bytes = serde_json::to_vec_pretty(cache).expect("a cache serializes");
    session_save::home_write::write_atomic(dir, CACHE_FILE, &bytes)
}
