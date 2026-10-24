//! The merge of the import record of an old storage folder into a new one,
//! for a Move to a new storage folder (R11).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::store::{
    ComputerId, EntryRead, Identity, entry_path, identity_folder, imports_folder, read_entry,
    write_new,
};
use shared_settings::inside::check_write_folder;

/// The result of a merge.
#[derive(Debug, Default, PartialEq)]
pub struct Merged {
    /// The entry files that the merge copied into the new storage folder.
    pub copied: usize,
    /// The entry files that the new storage folder has already. The merge
    /// does not replace them.
    pub kept: usize,
    /// The entry files and identity folders of the old storage folder that
    /// the merge cannot read, or that do not parse.
    pub not_copied: Vec<PathBuf>,
}

/// Copy each entry file of the import record of `old_root` into `new_root`
/// when `new_root` has no file for that identity and computer. The merge
/// reads only folder names that are an identity and file names that are a
/// computer id, and copies only entries that parse. The bytes stay the
/// same, so unknown fields and the schema version stay.
pub fn merge(old_root: &Path, new_root: &Path) -> io::Result<Merged> {
    let mut merged = Merged::default();
    let identities = match fs::read_dir(imports_folder(old_root)) {
        Ok(items) => items,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(merged),
        Err(e) => return Err(e),
    };
    for item in identities {
        let item = item?;
        let Some(identity) = item.file_name().to_str().and_then(Identity::parse) else {
            continue;
        };
        let from_folder = identity_folder(old_root, &identity);
        let files = match fs::read_dir(&from_folder) {
            Ok(files) => files,
            Err(_) => {
                merged.not_copied.push(from_folder);
                continue;
            }
        };
        for file in files {
            let file = file?;
            let Some(computer) = file
                .file_name()
                .to_str()
                .and_then(|name| name.strip_suffix(".json"))
                .and_then(ComputerId::parse)
            else {
                continue;
            };
            let to = entry_path(new_root, &identity, &computer);
            if fs::symlink_metadata(&to).is_ok() {
                merged.kept += 1;
                continue;
            }
            let from = entry_path(old_root, &identity, &computer);
            let bytes = match read_entry(old_root, &from) {
                EntryRead::Parsed(_, bytes) => bytes,
                EntryRead::Missing => continue,
                EntryRead::DoesNotParse(_) | EntryRead::Unreadable(_) => {
                    merged.not_copied.push(from);
                    continue;
                }
            };
            let to_folder = identity_folder(new_root, &identity);
            check_write_folder(new_root, &to_folder)?;
            fs::create_dir_all(&to_folder)?;
            let name = to.file_name().expect("an entry path has a file name");
            if write_new(&to_folder, &name.to_string_lossy(), &bytes)? {
                merged.copied += 1;
            } else {
                merged.kept += 1;
            }
        }
    }
    Ok(merged)
}
