//! The downloaded update archives (KTD10). Each verified archive is saved as
//! `<version>.tar.gz` in the updates folder, with the manifest's signature
//! in `<version>.tar.gz.sig`. `Update::install` does not check the signature,
//! and another process can change a saved file, so each install from a
//! saved archive checks it again first.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use minisign_verify::{PublicKey, Signature};

fn archive_path(dir: &Path, version: &str) -> PathBuf {
    dir.join(format!("{version}.tar.gz"))
}

fn signature_path(dir: &Path, version: &str) -> PathBuf {
    dir.join(format!("{version}.tar.gz.sig"))
}

fn decode_base64_text(value: &str, what: &str) -> Result<String, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|e| format!("the {what} is not base64: {e}"))?;
    String::from_utf8(bytes).map_err(|_| format!("the {what} is not text"))
}

/// Check `bytes` against a signature from the update manifest, with the
/// updater public key from `tauri.conf.json`. Both are base64 text, as the
/// updater plugin reads them.
pub fn verify(bytes: &[u8], signature: &str, pubkey: &str) -> Result<(), String> {
    let key = PublicKey::decode(&decode_base64_text(pubkey, "public key")?)
        .map_err(|e| format!("the updater public key is not valid: {e}"))?;
    let signature = Signature::decode(&decode_base64_text(signature, "signature")?)
        .map_err(|e| format!("the signature is not valid: {e}"))?;
    key.verify(bytes, &signature, false)
        .map_err(|e| format!("the signature does not match the download: {e}"))
}

/// Save a verified archive and its signature.
pub fn save(dir: &Path, version: &str, bytes: &[u8], signature: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    std::fs::write(signature_path(dir, version), signature)
        .and_then(|()| std::fs::write(archive_path(dir, version), bytes))
        .map_err(|e| {
            remove(dir, version);
            format!("cannot save the download: {e}")
        })
}

pub fn remove(dir: &Path, version: &str) {
    let _ = std::fs::remove_file(archive_path(dir, version));
    let _ = std::fs::remove_file(signature_path(dir, version));
}

/// Read a saved archive and check it against its saved signature. A failed
/// check deletes the archive.
pub fn load_verified(dir: &Path, version: &str, pubkey: &str) -> Result<Vec<u8>, String> {
    let result = std::fs::read(archive_path(dir, version))
        .map_err(|e| format!("cannot read the saved download: {e}"))
        .and_then(|bytes| {
            let signature = std::fs::read_to_string(signature_path(dir, version))
                .map_err(|e| format!("cannot read the saved signature: {e}"))?;
            verify(&bytes, &signature, pubkey).map(|()| bytes)
        });
    if result.is_err() {
        remove(dir, version);
    }
    result
}

/// The version of the newest saved archive that is higher than `current`.
/// This deletes each other archive and each archive with no signature.
pub fn newest_saved(dir: &Path, current: &str) -> Option<String> {
    let current = semver::Version::parse(current).ok()?;
    let mut versions: Vec<semver::Version> = Vec::new();
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let name = entry.file_name();
        let Some(version) = name.to_str().and_then(|n| n.strip_suffix(".tar.gz")) else {
            continue;
        };
        match semver::Version::parse(version) {
            Ok(v) if v > current && signature_path(dir, version).exists() => versions.push(v),
            _ => remove(dir, version),
        }
    }
    versions.sort();
    let newest = versions.pop()?;
    for older in versions {
        remove(dir, &older.to_string());
    }
    Some(newest.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A test key pair made with `tauri signer generate`. Only the public key
    // and one signature are here; the private key is not kept.
    const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEZCODMzNjY3RTc4OEVCMEMKUldRTTY0am5aemFEK3puZlhMYXduOE9CeWRuK1dkN3ZQY1FoU25GYmFtZjM3RGE3V1d0Tk4zM0EK";
    const SIGNED: &[u8] = b"longhand update test archive\n";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVRTTY0am5aemFEKzJWbzFKdlVPL2pUNVgwbzRXLytwWUJobk9YWm0zMFRCZVBUUjNzL0hOckpEV3Z6RFVOZ0tzVFkrajRPVnFVTDkxTVc4Mm5sUHQ2NkVZYkNYbXJRV2dnPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkwNjQ2MDM3CWZpbGU6YXJjaGl2ZS5iaW4KWGdkS2RQUXpOUW1GRUV4R0ZCbTJaM1o4ZXhYM1dIQmNBQkxBam4zU21vN3Y4NElqUDEyQVZqS1FVT3A2QUoxQ3Qrb296d2JmUmNyRzR2OUJydXJKQ0E9PQo=";

    const CURRENT: &str = "1.0.3";
    const NEW: &str = "1.0.10";

    #[test]
    fn a_signed_archive_passes_and_a_changed_archive_fails() {
        assert_eq!(verify(SIGNED, SIGNATURE, PUBKEY), Ok(()));
        assert!(verify(b"changed", SIGNATURE, PUBKEY).is_err());
        assert!(verify(SIGNED, "not base64!", PUBKEY).is_err());
        assert!(verify(SIGNED, SIGNATURE, "").is_err());
    }

    #[test]
    fn a_saved_archive_loads_when_its_signature_matches() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), NEW, SIGNED, SIGNATURE).unwrap();
        assert_eq!(load_verified(dir.path(), NEW, PUBKEY).unwrap(), SIGNED);
    }

    #[test]
    fn a_saved_archive_that_another_process_changed_is_deleted_and_not_loaded() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), NEW, SIGNED, SIGNATURE).unwrap();
        std::fs::write(archive_path(dir.path(), NEW), b"changed").unwrap();
        assert!(load_verified(dir.path(), NEW, PUBKEY).is_err());
        assert!(!archive_path(dir.path(), NEW).exists());
        assert!(!signature_path(dir.path(), NEW).exists());
    }

    #[test]
    fn only_the_newest_archive_above_the_running_version_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        for version in [CURRENT, "1.0.1", NEW, "1.0.9"] {
            save(dir.path(), version, SIGNED, SIGNATURE).unwrap();
        }
        std::fs::write(dir.path().join("1.0.11.tar.gz"), b"no signature").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"other").unwrap();
        assert_eq!(newest_saved(dir.path(), CURRENT), Some(NEW.to_string()));
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            vec![
                format!("{NEW}.tar.gz"),
                format!("{NEW}.tar.gz.sig"),
                "notes.txt".to_string()
            ]
        );
    }

    #[test]
    fn an_archive_of_the_running_version_is_deleted() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), CURRENT, SIGNED, SIGNATURE).unwrap();
        assert_eq!(newest_saved(dir.path(), CURRENT), None);
        assert!(!archive_path(dir.path(), CURRENT).exists());
    }

    #[test]
    fn no_folder_means_no_saved_archive() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(newest_saved(&dir.path().join("absent"), CURRENT), None);
    }
}
