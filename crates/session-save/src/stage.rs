//! The copy path (KTD17): when the working location and the destination
//! are on different volumes, the session is copied into staging file by
//! file, each copy flushed and then checked against its source by size and
//! SHA-256 before anything is called staged. The working copy stays until
//! the save is recorded.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::fsync::{sync_dir, sync_file};

/// The copy of the save's Copying step: [`copy_verified`], with the
/// marker of `note_id` written into `to` first. A copy that stops part
/// way, and whose removal fails, then still names its session, so the next
/// save can remove it.
pub fn copy_staging(from: &Path, to: &Path, note_id: &str, skip: &[&str]) -> io::Result<u64> {
    std::fs::create_dir(to)?;
    let marker = to.join(crate::MARKER);
    std::fs::write(&marker, note_id)?;
    sync_file(&marker)?;
    let skip: Vec<&str> = skip.iter().copied().chain([crate::MARKER]).collect();
    let mut bytes = 0;
    copy_dir(from, to, &skip, &mut bytes)?;
    Ok(bytes)
}

/// Copy `from` into the new directory `to`, leaving out the top-level
/// entries named in `skip`, and verify every file.
pub fn copy_verified(from: &Path, to: &Path, skip: &[&str]) -> io::Result<u64> {
    std::fs::create_dir(to)?;
    let mut bytes = 0;
    copy_dir(from, to, skip, &mut bytes)?;
    Ok(bytes)
}

fn copy_dir(from: &Path, to: &Path, skip: &[&str], bytes: &mut u64) -> io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            std::fs::create_dir(&dst)?;
            copy_dir(&src, &dst, &[], bytes)?;
        } else if kind.is_file() {
            *bytes += copy_file(&src, &dst)?;
        }
        // Symlinks are never followed out of the session.
    }
    sync_dir(to)
}

fn copy_file(src: &Path, dst: &Path) -> io::Result<u64> {
    let mut input = File::open(src)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(dst)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        output.write_all(&buf[..n])?;
        len += n as u64;
    }
    output.sync_all()?;
    drop(output);
    let (copied_len, copied_hash) = digest(dst)?;
    if copied_len != len || copied_hash != hasher.finalize().as_slice() {
        return Err(io::Error::other(format!(
            "the copy of {} does not match its source",
            src.display()
        )));
    }
    sync_file(dst)?;
    Ok(len)
}

fn digest(path: &Path) -> io::Result<(u64, Vec<u8>)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
    Ok((len, hasher.finalize().to_vec()))
}
