//! Credentials in the operating system's secret store (R33), keyed so a
//! credential is bound to the endpoint host it was entered for (KTD15, R30a).
//!
//! A modified copy of `reference-source/anarlog/plugins/store2/` without its
//! Tauri plugin surface; see PROVENANCE.md. Nothing here depends on Tauri, and
//! the store sits behind [`Backend`] so the location and error rules are tested
//! without touching the real keychain.

mod error;

pub use error::SecretError;

const SECURE_STORE_SUFFIX: &str = "secure-store";

/// The scope summarization credentials live under; the key is the endpoint's host.
pub const SUMMARIZE_SCOPE: &str = "summarize";

/// Raw access to one platform credential location. Errors are keyring's, so
/// the mapping onto [`SecretError`] is shared by every backend.
pub trait Backend: Send + Sync {
    fn get(&self, service: &str, account: &str) -> keyring::Result<String>;
    fn set(&self, service: &str, account: &str, value: &str) -> keyring::Result<()>;
    fn delete(&self, service: &str, account: &str) -> keyring::Result<()>;
}

/// The platform secret store: Keychain Services, Windows Credential Manager,
/// or Secret Service.
pub struct KeyringBackend;

impl Backend for KeyringBackend {
    fn get(&self, service: &str, account: &str) -> keyring::Result<String> {
        keyring::Entry::new(service, account)?.get_password()
    }

    fn set(&self, service: &str, account: &str, value: &str) -> keyring::Result<()> {
        keyring::Entry::new(service, account)?.set_password(value)
    }

    fn delete(&self, service: &str, account: &str) -> keyring::Result<()> {
        keyring::Entry::new(service, account)?.delete_credential()
    }
}

pub struct SecretStore<B: Backend = KeyringBackend> {
    service: String,
    backend: B,
}

impl SecretStore<KeyringBackend> {
    pub fn new(bundle_identifier: &str) -> Self {
        Self::with_backend(bundle_identifier, KeyringBackend)
    }
}

impl<B: Backend> SecretStore<B> {
    pub fn with_backend(bundle_identifier: &str, backend: B) -> Self {
        Self {
            service: service_name(bundle_identifier),
            backend,
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    /// `Ok(None)` is an absent credential, which is not a failure.
    pub fn get(&self, scope: &str, key: &str) -> Result<Option<String>, SecretError> {
        let account = account_name(scope, key)?;
        match self.backend.get(&self.service, &account) {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error::from_keyring(error)),
        }
    }

    pub fn set(&self, scope: &str, key: &str, value: &str) -> Result<(), SecretError> {
        let account = account_name(scope, key)?;
        self.backend
            .set(&self.service, &account, value)
            .map_err(error::from_keyring)
    }

    /// Deleting an absent credential succeeds.
    pub fn delete(&self, scope: &str, key: &str) -> Result<(), SecretError> {
        let account = account_name(scope, key)?;
        match self.backend.delete(&self.service, &account) {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error::from_keyring(error)),
        }
    }

    /// The credential for the endpoint at `base_url`, keyed by its host so a
    /// changed base URL has no credential until one is entered for it.
    pub fn endpoint_credential(&self, base_url: &str) -> Result<Option<String>, SecretError> {
        self.get(SUMMARIZE_SCOPE, &endpoint_key(base_url)?)
    }

    pub fn set_endpoint_credential(&self, base_url: &str, value: &str) -> Result<(), SecretError> {
        self.set(SUMMARIZE_SCOPE, &endpoint_key(base_url)?, value)
    }

    pub fn delete_endpoint_credential(&self, base_url: &str) -> Result<(), SecretError> {
        self.delete(SUMMARIZE_SCOPE, &endpoint_key(base_url)?)
    }
}

pub fn service_name(bundle_identifier: &str) -> String {
    format!("{bundle_identifier}.{SECURE_STORE_SUFFIX}")
}

fn account_name(scope: &str, key: &str) -> Result<String, SecretError> {
    if scope.trim().is_empty() || key.trim().is_empty() {
        return Err(SecretError::invalid(
            "secure-store scope and key must not be empty",
        ));
    }
    Ok(format!("{scope}:{key}"))
}

/// The key an endpoint's credential is stored under: its lowercased host,
/// plus the port when one is written explicitly. Scheme and path are not part
/// of it, so `http` and `https` on one host share a credential, while a
/// different port — a different server — does not.
pub fn endpoint_key(base_url: &str) -> Result<String, SecretError> {
    let url = url::Url::parse(base_url.trim())
        .map_err(|e| SecretError::invalid(format!("not a valid endpoint URL: {e}")))?;
    let host = url
        .host_str()
        .filter(|host| !host.is_empty())
        .ok_or_else(|| SecretError::invalid("endpoint URL has no host"))?
        .to_ascii_lowercase();
    Ok(match url.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// Lock and re-unlock the login keychain so macOS re-prompts for access.
/// Recovers from an ACL captured by an earlier code signature.
#[cfg(target_os = "macos")]
pub fn repair_keychain_access() -> Result<(), SecretError> {
    use objc2_security::SecKeychain;

    const ERR_SEC_SUCCESS: i32 = 0;

    #[allow(deprecated)]
    let lock_status = unsafe { SecKeychain::lock(None) };
    if lock_status != ERR_SEC_SUCCESS {
        return Err(SecretError::Platform {
            message: format!("macOS couldn't lock your login Keychain (OSStatus {lock_status})."),
        });
    }

    #[allow(deprecated)]
    let unlock_status = unsafe { SecKeychain::unlock(None, 0, std::ptr::null(), false) };
    if unlock_status == error::ERR_SEC_USER_CANCELED {
        return Err(SecretError::Locked {
            message: "Keychain unlock was cancelled. Your login Keychain is still locked; run the repair again to unlock it.".to_string(),
        });
    }
    if unlock_status != ERR_SEC_SUCCESS {
        return Err(SecretError::Locked {
            message: format!(
                "macOS couldn't unlock your login Keychain (OSStatus {unlock_status}). Your login Keychain is still locked."
            ),
        });
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn repair_keychain_access() -> Result<(), SecretError> {
    Err(SecretError::Platform {
        message: "Keychain repair is only available on macOS.".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemoryBackend {
        items: Mutex<HashMap<(String, String), String>>,
        fail_with_no_access: bool,
    }

    impl Backend for MemoryBackend {
        fn get(&self, service: &str, account: &str) -> keyring::Result<String> {
            if self.fail_with_no_access {
                return Err(keyring::Error::NoStorageAccess(Box::new(
                    std::io::Error::other(
                        "The user name or passphrase you entered is not correct.",
                    ),
                )));
            }
            self.items
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned()
                .ok_or(keyring::Error::NoEntry)
        }

        fn set(&self, service: &str, account: &str, value: &str) -> keyring::Result<()> {
            self.items.lock().unwrap().insert(
                (service.to_string(), account.to_string()),
                value.to_string(),
            );
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> keyring::Result<()> {
            self.items
                .lock()
                .unwrap()
                .remove(&(service.to_string(), account.to_string()))
                .map(|_| ())
                .ok_or(keyring::Error::NoEntry)
        }
    }

    fn store() -> SecretStore<MemoryBackend> {
        SecretStore::with_backend("net.ewal.longhand", MemoryBackend::default())
    }

    #[test]
    fn service_follows_the_bundle_identifier() {
        assert_eq!(store().service(), "net.ewal.longhand.secure-store");
    }

    #[test]
    fn absent_credential_is_none_not_an_error() {
        assert_eq!(
            store().endpoint_credential("https://api.example.com/v1"),
            Ok(None)
        );
    }

    #[test]
    fn locked_store_is_a_typed_error_distinct_from_absent() {
        let store = SecretStore::with_backend(
            "net.ewal.longhand",
            MemoryBackend {
                fail_with_no_access: true,
                ..Default::default()
            },
        );
        let error = store
            .endpoint_credential("https://api.example.com/v1")
            .unwrap_err();
        assert!(error.needs_unlock());
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn credential_round_trips_for_its_host() {
        let store = store();
        store
            .set_endpoint_credential("https://api.example.com/v1", "sk-test")
            .unwrap();
        assert_eq!(
            store.endpoint_credential("https://API.example.com/other/path"),
            Ok(Some("sk-test".to_string()))
        );
    }

    #[test]
    fn credential_for_one_host_is_absent_for_another() {
        let store = store();
        store
            .set_endpoint_credential("https://api.example.com/v1", "sk-test")
            .unwrap();
        assert_eq!(
            store.endpoint_credential("https://api.example.net/v1"),
            Ok(None)
        );
        assert_eq!(
            store.endpoint_credential("https://api.example.com:8443/v1"),
            Ok(None)
        );
    }

    #[test]
    fn credentials_live_under_the_summarize_scope() {
        let store = store();
        store
            .set_endpoint_credential("http://localhost:4000", "k")
            .unwrap();
        assert_eq!(
            store.get(SUMMARIZE_SCOPE, "localhost:4000"),
            Ok(Some("k".into()))
        );
    }

    #[test]
    fn deleting_an_absent_credential_succeeds() {
        assert_eq!(
            store().delete_endpoint_credential("https://api.example.com"),
            Ok(())
        );
    }

    #[test]
    fn empty_scope_or_key_is_rejected_before_the_store() {
        assert!(matches!(
            store().get("", "k"),
            Err(SecretError::InvalidLocation { .. })
        ));
        assert!(matches!(
            store().get("summarize", "  "),
            Err(SecretError::InvalidLocation { .. })
        ));
    }

    #[test]
    fn endpoint_key_is_host_and_explicit_port_only() {
        assert_eq!(
            endpoint_key("https://Api.Example.com/v1/").unwrap(),
            "api.example.com"
        );
        assert_eq!(
            endpoint_key("http://127.0.0.1:1234").unwrap(),
            "127.0.0.1:1234"
        );
        // A default port written out is normalized away by the URL parser.
        assert_eq!(
            endpoint_key("https://api.example.com:443").unwrap(),
            "api.example.com"
        );
        assert!(endpoint_key("not a url").is_err());
        assert!(endpoint_key("file:///tmp/x").is_err());
    }
}
