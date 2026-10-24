//! Read an audio file that another device recorded (U2 of the audio import
//! plan): copy it with its SHA-256, decode the copy to mono samples, and read
//! its recording time.
//!
//! The content identity of a file is the SHA-256 of the whole file (KTD3).
//! The decoder seeks inside m4a files, so the hash comes from one sequential
//! copy, and the decoder reads the copy, never the source.

mod copy;
mod decode;
mod metadata;

pub use copy::{Copied, copy_with_hash, hash_file};
pub use decode::{DecodeError, DecodeInfo, decode_mono};
pub use metadata::recording_time;

/// The decoder name that a `failed` entry of the import record holds (KTD11).
pub const DECODER_NAME: &str = "symphonia";
/// The decoder version that a `failed` entry of the import record holds.
pub const DECODER_VERSION: &str = "0.6.1";

/// The file extensions that Longhand imports (R19), in lower case.
pub const ACCEPTED_EXTENSIONS: [&str; 3] = ["m4a", "mp3", "wav"];

/// Whether `path` has an accepted extension, in any case.
pub fn is_accepted(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ACCEPTED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}
