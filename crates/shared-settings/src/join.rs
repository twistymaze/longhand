//! The folder join (R21, R22, KTD8): what this computer does with the
//! shared settings of a storage folder after a change of folder, and at
//! each launch.
//!
//! The rule has three cases:
//!
//! - The folder has a settings file that parses. This computer uses it.
//!   After a change of folder, each custom prompt of this computer that
//!   the folder does not have is added to the folder.
//! - The folder is reachable and has no settings file. This computer
//!   writes its cache into it, or the defaults when it has no cache.
//! - The folder is not reachable, or its settings file does not parse.
//!   This computer uses its cache, read-only, and writes nothing.
//!
//! A join writes each added prompt as its own file, so a join that runs
//! again adds nothing.
//!
//! A join writes the Background file only when the cache text is not
//! empty and the folder has no Background file. It does this when it also
//! writes the settings file, or after a change of folder (KTD2). It never
//! writes over a Background file.

use std::io;
use std::path::Path;

use summarize::prompts::Prompt;

use crate::background::write_new_background;
use crate::cache::{Cache, read_cache};
use crate::file::{FileRead, apply_patch, default_settings, read_settings, write_new_settings};
use crate::prompts::{
    built_in_names, free_prompt_name, is_built_in, joined_id, same_name, scan_prompts,
    write_new_prompt,
};
use crate::view::{SharedView, load, load_after_join, store};

/// Why the join runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinKind {
    /// The user changed the storage folder, or this is the first launch
    /// after the update that added shared settings (R21). The cache of any
    /// folder can fill the new folder, and its prompts merge into a folder
    /// that has settings.
    ChangeOfFolder,
    /// A launch or a read with no change of folder. Only a cache from this
    /// same folder can fill it, and no prompt merges.
    SameFolder,
}

/// What a join wrote into the storage folder.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Written {
    /// The join wrote the settings file.
    pub settings_file: bool,
    /// The file names of the prompts that the join added.
    pub prompt_files: Vec<String>,
    /// The join wrote the Background file (KTD2).
    pub background_file: bool,
}

/// The result of a join.
#[derive(Debug, Clone, PartialEq)]
pub struct Joined {
    /// The shared view after the join.
    pub view: SharedView,
    pub written: Written,
    /// The cache to write, when it differs from the old cache.
    pub cache: Option<Cache>,
}

/// Join the storage folder `root` with `cache`, the last cache of this
/// computer.
pub fn join(root: &Path, cache: Option<&Cache>, kind: JoinKind) -> io::Result<Joined> {
    let mut written = Written::default();
    match read_settings(root) {
        FileRead::Parsed(_) => {
            if kind == JoinKind::ChangeOfFolder
                && let Some(cache) = cache
            {
                written.prompt_files = merge_prompts(root, &prompts_of(cache))?;
                written.background_file = write_new_background(root, &cache.background)?;
            }
        }
        FileRead::Missing => {
            let may_fill = match cache {
                None => true,
                Some(cache) => kind == JoinKind::ChangeOfFolder || cache.source == root,
            };
            if may_fill {
                let settings = cache.map_or_else(default_settings, |c| c.settings.clone());
                // False when a sync delivered a settings file in the
                // meantime. That file wins, as in the first case.
                written.settings_file = write_new_settings(root, &apply_patch(&settings, &[]))?;
                if let Some(cache) = cache
                    && (written.settings_file || kind == JoinKind::ChangeOfFolder)
                {
                    written.prompt_files = merge_prompts(root, &prompts_of(cache))?;
                    written.background_file = write_new_background(root, &cache.background)?;
                }
            }
        }
        FileRead::NotAvailable(_) => {}
    }
    // Only a change of folder merges a cache from another folder, so only
    // it replaces that cache.
    let loaded = match kind {
        JoinKind::ChangeOfFolder => load_after_join(root, cache),
        JoinKind::SameFolder => load(root, cache),
    };
    Ok(Joined {
        view: loaded.view,
        written,
        cache: loaded.cache,
    })
}

/// [`join`] with the cache in the folder `cache_dir`: read the cache, join,
/// and write the cache when it changed.
pub fn join_folder(root: &Path, cache_dir: &Path, kind: JoinKind) -> io::Result<Joined> {
    let old = read_cache(cache_dir);
    let joined = join(root, old.as_ref(), kind)?;
    store(cache_dir, joined.cache.as_ref());
    Ok(joined)
}

/// The custom prompts of `cache`, the active prompt and the prompts of
/// the note types included when the list does not have them.
fn prompts_of(cache: &Cache) -> Vec<Prompt> {
    let mut prompts = cache.prompts.clone();
    for prompt in cache.active_prompt.iter().chain(&cache.type_prompts) {
        if !prompts.iter().any(|p| p.id == prompt.id) {
            prompts.push(prompt.clone());
        }
    }
    prompts
}

/// Add each prompt of `prompts` that the folder does not have (R22). A
/// prompt with the same name and the same texts, or with the same id and
/// the same texts, is there already. The texts are the summary text and
/// the title text (KTD9). A prompt with the same name and other texts gets
/// a free name. Returns the file names that it wrote.
fn merge_prompts(root: &Path, prompts: &[Prompt]) -> io::Result<Vec<String>> {
    let scan = scan_prompts(root).map_err(|cause| io::Error::other(cause.to_string()))?;
    // The id, the names and the texts of each prompt in the folder.
    let mut folder: Vec<(String, Vec<String>, (String, String))> = scan
        .prompts
        .iter()
        .map(|p| {
            (
                p.id.clone(),
                vec![p.name.clone(), p.stored_name.clone()],
                (p.text.clone(), p.title_text.clone()),
            )
        })
        .collect();
    let mut written = Vec::new();
    for prompt in prompts {
        if is_built_in(&prompt.id) || prompt.text.trim().is_empty() {
            continue;
        }
        let id_taken = folder.iter().any(|(id, _, _)| *id == prompt.id);
        let id = if id_taken {
            joined_id(&prompt.id)
        } else {
            prompt.id.clone()
        };
        let texts = (prompt.text.clone(), prompt.title_text.clone());
        let present = folder.iter().any(|(folder_id, names, folder_texts)| {
            *folder_texts == texts
                && (*folder_id == prompt.id
                    || *folder_id == id
                    || names.iter().any(|n| same_name(n, &prompt.name)))
        });
        if present {
            continue;
        }
        let taken: Vec<String> = built_in_names()
            .chain(
                folder
                    .iter()
                    .flat_map(|(_, names, _)| names.iter().cloned()),
            )
            .collect();
        let name = free_prompt_name(prompt.name.trim(), taken.iter().map(String::as_str));
        let file_name = write_new_prompt(root, &id, &name, &prompt.text, &prompt.title_text)?;
        folder.push((id, vec![name], texts));
        written.push(file_name);
    }
    Ok(written)
}
