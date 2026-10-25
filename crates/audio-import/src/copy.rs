//! The sequential copy and hash of a source file (KTD3).

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use sha2::{Digest, Sha256};

/// The read size of the copy and the hash.
const BUFFER: usize = 64 * 1024;

/// A copy of a source file and the SHA-256 of its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// The path of the copy.
    pub path: PathBuf,
    /// The SHA-256 of the copied bytes, as 64 lowercase hex characters.
    pub sha256: String,
    /// The number of copied bytes.
    pub bytes: u64,
}

/// Copy `source` to `destination` in one sequential read, and hash the bytes
/// that it copies. A failed copy removes the partial destination.
pub fn copy_with_hash(source: &Path, destination: &Path) -> io::Result<Copied> {
    let result = (|| {
        let mut out = File::create(destination)?;
        let (sha256, bytes) = stream(source, |chunk| out.write_all(chunk))?;
        out.sync_all()?;
        Ok(Copied {
            path: destination.to_path_buf(),
            sha256,
            bytes,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(destination);
    }
    result
}

/// The SHA-256 of `path`, from one sequential read with no copy (KTD12).
pub fn hash_file(path: &Path) -> io::Result<String> {
    stream(path, |_| Ok(())).map(|(sha256, _)| sha256)
}

/// The SHA-256 of `path` as [`hash_file`] gives it. `read` counts the bytes
/// read so far, so another thread can see whether the read makes progress
/// (KTD8).
pub fn hash_file_counted(path: &Path, read: &AtomicU64) -> io::Result<String> {
    stream(path, |chunk| {
        read.fetch_add(chunk.len() as u64, Ordering::SeqCst);
        Ok(())
    })
    .map(|(sha256, _)| sha256)
}

/// Read `path` from start to end, hash each chunk and give it to `sink`.
fn stream(path: &Path, mut sink: impl FnMut(&[u8]) -> io::Result<()>) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut bytes = 0u64;
    loop {
        let read = match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        hasher.update(&buffer[..read]);
        sink(&buffer[..read])?;
        bytes += read as u64;
    }
    let sha256 = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    Ok((sha256, bytes))
}
