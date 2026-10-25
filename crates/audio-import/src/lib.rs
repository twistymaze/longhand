//! Read an audio file that another device recorded: copy it with its
//! SHA-256, decode the copy to mono samples, and read its recording time.
//!
//! The content identity of a file is the SHA-256 of the whole file (KTD3).
//! The decoder seeks inside m4a files, so the hash comes from one sequential
//! copy, and the decoder reads the copy, never the source.

mod copy;
mod decode;
mod metadata;

pub use copy::{Copied, copy_with_hash, hash_file, hash_file_counted};
pub use decode::{DecodeError, DecodeInfo, decode_mono, decode_mono_until};
pub use metadata::recording_time;

/// The decoder name that a `failed` entry of the import record holds (KTD11).
pub const DECODER_NAME: &str = "symphonia";
/// The decoder version that a `failed` entry of the import record holds.
pub const DECODER_VERSION: &str = "0.6.1";

/// The file extensions that Longhand imports (R19), in lower case. Voice
/// Memos gives `qta` files: AAC in a QuickTime container, which the m4a
/// demuxer reads.
pub const ACCEPTED_EXTENSIONS: [&str; 4] = ["m4a", "qta", "mp3", "wav"];

/// The file name of `path`, with invalid UTF-8 replaced. Empty when `path`
/// has no file name.
pub fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Whether `path` has an accepted extension, in any case.
pub fn is_accepted(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ACCEPTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
