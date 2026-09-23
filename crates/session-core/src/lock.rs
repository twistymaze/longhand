//! One instance per working location (R34, KTD22, U19).
//!
//! An exclusive OS lock on a file in the app's data directory, held for the
//! life of the process. The operating system releases it when the process
//! ends, however it ends, so holding it is proof that no other instance is
//! alive: every claim another instance left in the job store is a dead one
//! and can be reclaimed at once (U17), and the launch sweeps (U16, U18) run
//! with nothing else touching the working location. The file's text names
//! the holder, for the message a refused launch gives and for the log.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

pub const LOCK_FILE: &str = "longhand.lock";

/// Who holds, or last held, the lock: as written by that process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub instance: String,
}

/// Held until dropped, or until the process ends.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    /// The instance that held the lock before, now gone.
    pub displaced: Option<Holder>,
}

#[derive(Debug)]
pub enum Acquired {
    /// This process is the only instance.
    Exclusive(InstanceLock),
    /// Another live process holds the lock. Its name is `None` where it
    /// cannot be read (on Windows the lock also bars reading).
    HeldBy(Option<Holder>),
    /// The lock could not be tried at all; whether another instance is
    /// alive is unknown.
    Unavailable(std::io::Error),
}

/// Take the lock in `dir` for `instance`.
pub fn acquire(dir: &Path, instance: &str) -> Acquired {
    let path = dir.join(LOCK_FILE);
    let mut file = match OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
    {
        Ok(file) => file,
        Err(e) => return Acquired::Unavailable(e),
    };
    match file.try_lock() {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => return Acquired::HeldBy(read_holder(&mut file)),
        Err(TryLockError::Error(e)) => return Acquired::Unavailable(e),
    }
    let displaced = read_holder(&mut file);
    let written = (|| {
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        writeln!(file, "{} {}", std::process::id(), instance)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        // The lock is what matters; the name is for messages.
        log::warn!("could not record this instance in {}: {e}", path.display());
    }
    Acquired::Exclusive(InstanceLock {
        _file: file,
        displaced,
    })
}

fn read_holder(file: &mut File) -> Option<Holder> {
    let mut text = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut text).ok()?;
    parse(&text)
}

fn parse(text: &str) -> Option<Holder> {
    let (pid, instance) = text.trim().split_once(' ')?;
    Some(Holder {
        pid: pid.parse().ok()?,
        instance: instance.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_holder_parses_from_what_it_wrote() {
        assert_eq!(
            parse("4242 run-a\n"),
            Some(Holder {
                pid: 4242,
                instance: "run-a".into()
            })
        );
        assert_eq!(parse(""), None);
        assert_eq!(parse("not-a-pid run"), None);
    }
}
