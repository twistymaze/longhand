//! The shared import record of a storage folder (KTD2 of the audio import
//! plan). Each imported file has one folder `.longhand/imports/<sha256>/`,
//! and each computer writes one file in it, `<computer_id>.json`. No two
//! computers write the same file. Callers run each function off the main
//! thread, because a read of a synced folder can block.

pub mod entry;
pub mod merge;
pub mod store;

pub use entry::{Entry, SCHEMA_VERSION, State};
pub use merge::{Merged, merge};
pub use store::{
    ComputerId, IMPORTS_FOLDER, Identity, LONGHAND_FOLDER, Lookup, MAX_ENTRY_BYTES, WriteError,
    entry_of, entry_path, identity_folder, imports_folder, lookup, write,
};
