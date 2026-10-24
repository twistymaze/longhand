//! The read and the write of the import record in a storage folder. The
//! rules of `docs/solutions/integration-issues/shared-settings-in-a-synced-folder-rules.md`
//! apply: a lookup never writes, only exact file names count, a file that
//! does not parse makes its identity wait and is never overwritten, a write
//! keeps unknown fields and never lowers the schema version, a link outside
//! the storage folder is refused, and a read has a size limit.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::entry::{Entry, SCHEMA_VERSION, State};
use shared_settings::inside::{OUTSIDE_FILE, check_write_folder, inside_file, open_checked};

/// The hidden Longhand folder of a storage folder.
pub const LONGHAND_FOLDER: &str = ".longhand";

/// The folder of the import record in the hidden Longhand folder.
pub const IMPORTS_FOLDER: &str = "imports";

/// The most bytes that one entry file can hold. An entry is a few hundred
/// bytes. A synced file can be of any size, and a read must not use all
/// the memory.
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024;

/// The extension of an entry file.
const ENTRY_EXTENSION: &str = ".json";

/// The identity of a file: the SHA-256 of its content as 64 lowercase hex
/// characters. The name of its folder in the import record.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Identity(String);

impl Identity {
    /// The identity `text`, when it is 64 lowercase hex characters. Text
    /// from a synced folder name becomes a path, so only this form is
    /// accepted (`docs/solutions/security-issues/synced-note-id-became-a-delete-path.md`).
    pub fn parse(text: &str) -> Option<Self> {
        is_lower_hex(text, 64).then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The id of a computer: 32 lowercase hex characters, as the `computer`
/// table of `session-core` makes it. The name of the computer's entry file.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ComputerId(String);

impl ComputerId {
    /// The computer id `text`, when it is 32 lowercase hex characters.
    pub fn parse(text: &str) -> Option<Self> {
        is_lower_hex(text, 32).then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The computer id of the entry file name `name`, which is
    /// `<computer_id>.json` exactly. A conflict copy such as
    /// `<computer_id> 2.json` gives none.
    fn of_file_name(name: &str) -> Option<Self> {
        name.strip_suffix(ENTRY_EXTENSION).and_then(Self::parse)
    }

    fn file_name(&self) -> String {
        format!("{}{ENTRY_EXTENSION}", self.0)
    }
}

/// Whether `text` is `len` lowercase hex characters.
fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The result of a lookup of an identity.
#[derive(Debug, PartialEq)]
pub enum Lookup {
    /// No computer has an entry for the identity.
    New,
    /// A computer imported the file.
    Imported(Entry),
    /// A computer skipped the file, and no computer imported it.
    Skipped(Entry),
    /// The decoder refused the file on each computer with an entry. The
    /// entry is the one with the newest Longhand version.
    Failed(Entry),
    /// An entry file of the identity cannot be used now: it does not parse,
    /// it is too large, it cannot be read, or it links outside the storage
    /// folder. A sync can still be in progress. Do not import the file.
    Wait,
}

/// The reason that a write did not change the entry file.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("the computer id of the entry is not valid")]
    InvalidComputerId,
    /// The entry file of this computer exists and does not parse. The
    /// write does not overwrite it.
    #[error("the entry file does not parse: {0}")]
    DoesNotParse(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The folder of the import record of the storage folder `root`.
pub fn imports_folder(root: &Path) -> PathBuf {
    root.join(LONGHAND_FOLDER).join(IMPORTS_FOLDER)
}

/// The folder of the identity `identity` in the import record of `root`.
pub fn identity_folder(root: &Path, identity: &Identity) -> PathBuf {
    imports_folder(root).join(identity.as_str())
}

/// The entry file of the computer `computer` for `identity`.
pub fn entry_path(root: &Path, identity: &Identity, computer: &ComputerId) -> PathBuf {
    identity_folder(root, identity).join(computer.file_name())
}

/// The read of one entry file.
pub(crate) enum EntryRead {
    /// Nothing is at the path.
    Missing,
    /// The entry and the bytes of the file.
    Parsed(Box<Entry>, Vec<u8>),
    /// The file is too large or does not parse.
    DoesNotParse(String),
    /// The file cannot be read, or it links outside the storage folder.
    Unreadable(io::Error),
}

/// Read the entry file at `path` in the storage folder `root`. The read
/// follows a link only to a file in the storage folder, and it reads at
/// most [`MAX_ENTRY_BYTES`].
pub(crate) fn read_entry(root: &Path, path: &Path) -> EntryRead {
    let checked = match inside_file(root, path) {
        Ok(checked) => checked,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return EntryRead::Missing,
        Err(e) => {
            if e.to_string() == OUTSIDE_FILE {
                log::warn!("import record: cannot read {}: {e}", path.display());
            }
            return EntryRead::Unreadable(e);
        }
    };
    let mut bytes = Vec::new();
    let read = open_checked(root, &checked)
        .and_then(|file| file.take(MAX_ENTRY_BYTES + 1).read_to_end(&mut bytes));
    match read {
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => return EntryRead::Missing,
        Err(e) => return EntryRead::Unreadable(e),
    }
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return EntryRead::DoesNotParse("the file is too large".into());
    }
    match serde_json::from_slice::<Entry>(&bytes) {
        Ok(entry) => EntryRead::Parsed(Box::new(entry), bytes),
        Err(e) => EntryRead::DoesNotParse(e.to_string()),
    }
}

/// Look up `identity` in the import record of the storage folder `root`
/// (R12, R13). The lookup reads each file `<computer_id>.json` in exactly
/// `imports/<identity>/`, and ignores every other name. It resolves
/// `Imported` over `Skipped` over `Failed`. It never writes.
pub fn lookup(root: &Path, identity: &Identity) -> Lookup {
    let folder = identity_folder(root, identity);
    let items = match fs::read_dir(&folder) {
        Ok(items) => items,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Lookup::New,
        Err(_) => return Lookup::Wait,
    };
    let mut best: Option<Entry> = None;
    for item in items {
        let Ok(item) = item else {
            return Lookup::Wait;
        };
        let name = item.file_name();
        if name.to_str().and_then(ComputerId::of_file_name).is_none() {
            continue;
        }
        match read_entry(root, &folder.join(&name)) {
            EntryRead::Missing => {}
            EntryRead::Parsed(entry, _) => {
                if best.as_ref().is_none_or(|best| ranks_above(&entry, best)) {
                    best = Some(*entry);
                }
            }
            EntryRead::DoesNotParse(_) | EntryRead::Unreadable(_) => return Lookup::Wait,
        }
    }
    match best {
        None => Lookup::New,
        Some(entry) => match entry.state {
            State::Imported => Lookup::Imported(entry),
            State::Skipped => Lookup::Skipped(entry),
            State::Failed => Lookup::Failed(entry),
        },
    }
}

/// The source file name and the size of each entry in the import record of
/// `root`, in any state. The first-pick question of the watched folder
/// counts the files that match none of them (R6, KTD12). The read follows
/// the same rules as [`lookup`]: only identity folders and exact entry
/// names, and an entry that cannot be read or does not parse gives
/// nothing. It never writes.
pub fn names_and_sizes(root: &Path) -> HashSet<(String, u64)> {
    let mut found = HashSet::new();
    let Ok(folders) = fs::read_dir(imports_folder(root)) else {
        return found;
    };
    for folder in folders.flatten() {
        let name = folder.file_name();
        let Some(identity) = name.to_str().and_then(Identity::parse) else {
            continue;
        };
        let folder = identity_folder(root, &identity);
        let Ok(items) = fs::read_dir(&folder) else {
            continue;
        };
        for item in items.flatten() {
            let name = item.file_name();
            if name.to_str().and_then(ComputerId::of_file_name).is_none() {
                continue;
            }
            if let EntryRead::Parsed(entry, _) = read_entry(root, &folder.join(&name)) {
                found.insert((entry.source_name, entry.size));
            }
        }
    }
    found
}

/// The entry of the computer `computer` for `identity`, when its file
/// parses. The import worker reads its own entry to find a write whose
/// record a crash lost (KTD6). It never writes.
pub fn entry_of(root: &Path, identity: &Identity, computer: &ComputerId) -> Option<Entry> {
    match read_entry(root, &entry_path(root, identity, computer)) {
        EntryRead::Parsed(entry, _) => Some(*entry),
        EntryRead::Missing | EntryRead::DoesNotParse(_) | EntryRead::Unreadable(_) => None,
    }
}

/// Whether `entry` decides the lookup over `best`: a higher state, or for
/// two `Failed` entries, a newer Longhand version.
fn ranks_above(entry: &Entry, best: &Entry) -> bool {
    match entry.state.cmp(&best.state) {
        std::cmp::Ordering::Equal => {
            entry.state == State::Failed && entry.version() > best.version()
        }
        order => order.is_gt(),
    }
}

/// Write `entry` as the entry file of its computer for `identity` in the
/// storage folder `root` (KTD2). A computer writes only its own file. When
/// the file exists, the write keeps its unknown fields and its higher
/// schema version. A file that does not parse stays as it is, and the
/// write returns [`WriteError::DoesNotParse`]. Returns the entry as written.
pub fn write(root: &Path, identity: &Identity, entry: &Entry) -> Result<Entry, WriteError> {
    let computer = ComputerId::parse(&entry.computer_id).ok_or(WriteError::InvalidComputerId)?;
    let folder = identity_folder(root, identity);
    check_write_folder(root, &folder)?;
    fs::create_dir_all(&folder)?;
    let mut written = entry.clone();
    match read_entry(root, &folder.join(computer.file_name())) {
        EntryRead::Missing => {}
        EntryRead::Parsed(current, _) => {
            let mut other = current.other;
            other.extend(entry.other.clone());
            written.other = other;
            written.schema_version = written.schema_version.max(current.schema_version);
        }
        EntryRead::DoesNotParse(cause) => return Err(WriteError::DoesNotParse(cause)),
        EntryRead::Unreadable(e) => return Err(WriteError::Io(e)),
    }
    written.schema_version = written.schema_version.max(SCHEMA_VERSION);
    let mut bytes = serde_json::to_vec_pretty(&written).expect("an entry serializes");
    bytes.push(b'\n');
    session_save::home_write::write_atomic(&folder, &computer.file_name(), &bytes)?;
    Ok(written)
}

/// Write `contents` as `dir/name` only when nothing is at that name. The
/// file is complete before it gets its name. The staged name starts with
/// `.`, so no lookup reads it. Returns false, and writes nothing, when the
/// name is taken. This is the `write_new` of `shared-settings`.
pub(crate) fn write_new(dir: &Path, name: &str, contents: &[u8]) -> io::Result<bool> {
    let staged = format!(".{}.new", uuid::Uuid::new_v4().simple());
    session_save::home_write::write_atomic(dir, &staged, contents)?;
    let placed = session_save::rename::rename_noreplace(&dir.join(&staged), &dir.join(name));
    if let Err(e) = placed {
        let _ = fs::remove_file(dir.join(&staged));
        return if session_save::rename::is_name_taken(&e) {
            Ok(false)
        } else {
            Err(e)
        };
    }
    session_save::fsync::sync_dir(dir)?;
    Ok(true)
}
