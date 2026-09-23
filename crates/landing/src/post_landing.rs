//! Writes into a landed folder (R22f): a temporary file in the same folder,
//! then a rename over the target, so an external tool never sees a partial
//! file. The temporary name is unpredictable and created with create-new,
//! which refuses anything already at the path, a symlink included: in a
//! folder another account or process can write to, a predictable name could
//! be pre-created as a link, and the write would truncate what it points at.

use std::fs::OpenOptions;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::fsync::sync_dir;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn unpredictable() -> u64 {
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    h.write_u32(std::process::id());
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

/// Replace `dir/name` with `contents` atomically.
pub fn write_atomic(dir: &Path, name: &str, contents: &[u8]) -> io::Result<()> {
    let temp = format!(".{name}.{:016x}.tmp", unpredictable());
    write_atomic_via(dir, name, &temp, contents)
}

/// [`write_atomic`] through a given temporary name, for tests.
pub fn write_atomic_via(dir: &Path, name: &str, temp: &str, contents: &[u8]) -> io::Result<()> {
    let temp_path = dir.join(temp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW).mode(0o644);
    }
    let mut file = options.open(&temp_path)?;
    let written = file.write_all(contents).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| std::fs::rename(&temp_path, dir.join(name))) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(e);
    }
    sync_dir(dir)
}
