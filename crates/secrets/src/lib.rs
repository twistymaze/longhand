//! Credentials in the operating system's secret store (R33), keyed so a
//! credential is bound to the provider and the host it was entered for
//! (KTD8, R10).
//!
//! A modified copy of `reference-source/anarlog/plugins/store2/` without its
//! Tauri plugin surface; see PROVENANCE.md. Nothing here depends on Tauri, and
//! the store sits behind [`Backend`] so the location and error rules are tested
//! without touching the real keychain.

mod error;

use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};

pub use error::SecretError;

const SECURE_STORE_SUFFIX: &str = "secure-store";

/// The scope of the summary credentials. The key is `{provider}:{host}`.
/// Before KTD8, the key was the host only.
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
    /// Held across each read, save and delete of a summary credential, so
    /// that these operations do not overlap. Without it, a read can copy the
    /// old entry after a concurrent Remove deleted both entries, and the
    /// removed key comes back (KTD8). The process has one store.
    summarize: Mutex<()>,
    /// Gets a message when an operation waits for `summarize`.
    #[cfg(test)]
    on_wait: Mutex<Option<std::sync::mpsc::Sender<&'static str>>>,
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
            summarize: Mutex::new(()),
            #[cfg(test)]
            on_wait: Mutex::new(None),
        }
    }

    /// Lock `summarize`. A panic in an earlier holder does not stop the
    /// store: the lock protects no data.
    fn lock_summarize(&self) -> MutexGuard<'_, ()> {
        match self.summarize.try_lock() {
            Ok(guard) => return guard,
            Err(TryLockError::Poisoned(e)) => return e.into_inner(),
            Err(TryLockError::WouldBlock) => {}
        }
        #[cfg(test)]
        if let Some(on_wait) = &*self.on_wait.lock().unwrap() {
            let _ = on_wait.send("waiting");
        }
        self.summarize
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
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

    /// The credential of `provider` for the host of `base_url` (R10). The
    /// account is `summarize:{provider}:{host}`, so a changed host has no
    /// credential until one is entered for it, and two providers on one host
    /// keep separate credentials.
    ///
    /// With `legacy`, the provider is the one that the settings migration
    /// chose, and an old host-only entry (`summarize:{host}`) can still
    /// exist (KTD8, R23). When the provider's entry is absent, the old
    /// value is copied into it, and the old entry is deleted only after that
    /// write succeeds. When both entries exist, the old entry is deleted and
    /// the provider's entry does not change. A failed read or write returns
    /// the error and keeps the old entry. A failed delete of the old entry
    /// returns the value: the next read tries the delete again.
    ///
    /// When the old entry is also absent, the provider's entry is read
    /// again: an operation outside this store (another process) can move
    /// the old entry between the two reads.
    pub fn provider_credential(
        &self,
        provider: &str,
        base_url: &str,
        legacy: bool,
    ) -> Result<Option<String>, SecretError> {
        let host = endpoint_key(base_url)?;
        let key = provider_key(provider, &host)?;
        let _summarize = self.lock_summarize();
        let current = self.get(SUMMARIZE_SCOPE, &key)?;
        if !legacy {
            return Ok(current);
        }
        let value = match current {
            Some(value) => value,
            None => {
                let Some(old) = self.get(SUMMARIZE_SCOPE, &host)? else {
                    return self.get(SUMMARIZE_SCOPE, &key);
                };
                self.set(SUMMARIZE_SCOPE, &key, &old)?;
                old
            }
        };
        // The value is safe in the provider's entry. A failed delete leaves
        // the old entry for the next read.
        let _ = self.delete(SUMMARIZE_SCOPE, &host);
        Ok(Some(value))
    }

    /// Save the credential of `provider` for the host of `base_url`.
    pub fn set_provider_credential(
        &self,
        provider: &str,
        base_url: &str,
        value: &str,
    ) -> Result<(), SecretError> {
        let key = provider_key(provider, &endpoint_key(base_url)?)?;
        let _summarize = self.lock_summarize();
        self.set(SUMMARIZE_SCOPE, &key, value)
    }

    /// Delete the credential of `provider` for the host of `base_url`. With
    /// `legacy` (see [`SecretStore::provider_credential`]), the old
    /// host-only entry is deleted too, so a removed key cannot come back
    /// from it (KTD8).
    pub fn delete_provider_credential(
        &self,
        provider: &str,
        base_url: &str,
        legacy: bool,
    ) -> Result<(), SecretError> {
        let host = endpoint_key(base_url)?;
        let key = provider_key(provider, &host)?;
        let _summarize = self.lock_summarize();
        self.delete(SUMMARIZE_SCOPE, &key)?;
        if legacy {
            self.delete(SUMMARIZE_SCOPE, &host)?;
        }
        Ok(())
    }
}

/// The key of a provider's credential under [`SUMMARIZE_SCOPE`]:
/// `{provider}:{host}` (KTD8).
fn provider_key(provider: &str, host: &str) -> Result<String, SecretError> {
    if provider.trim().is_empty() || provider.contains(':') {
        return Err(SecretError::invalid(
            "a provider name must not be empty or contain ':'",
        ));
    }
    Ok(format!("{provider}:{host}"))
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
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;

    use super::*;

    const CUSTOM: &str = "custom";
    const LITELLM: &str = "https://litellm.ewal.io/v1";

    type Hook = Box<dyn FnOnce() + Send>;

    #[derive(Default)]
    struct MemoryBackend {
        items: Mutex<HashMap<(String, String), String>>,
        /// Every operation fails as a locked Keychain.
        fail_with_no_access: bool,
        /// Only writes fail, as a locked Keychain.
        writes_locked: bool,
        /// Only deletes fail, as a locked Keychain.
        deletes_locked: AtomicBool,
        /// Runs one time, after a read of the account, before the read
        /// returns.
        after_get: Mutex<Option<(String, Hook)>>,
    }

    fn no_access() -> keyring::Error {
        keyring::Error::NoStorageAccess(Box::new(std::io::Error::other(
            "The user name or passphrase you entered is not correct.",
        )))
    }

    impl MemoryBackend {
        fn holding(entries: &[(&str, &str)]) -> Self {
            let backend = Self::default();
            for (account, value) in entries {
                backend.items.lock().unwrap().insert(
                    (service_name("net.ewal.longhand"), account.to_string()),
                    value.to_string(),
                );
            }
            backend
        }
    }

    impl Backend for MemoryBackend {
        fn get(&self, service: &str, account: &str) -> keyring::Result<String> {
            if self.fail_with_no_access {
                return Err(no_access());
            }
            let result = self
                .items
                .lock()
                .unwrap()
                .get(&(service.to_string(), account.to_string()))
                .cloned()
                .ok_or(keyring::Error::NoEntry);
            let mut after_get = self.after_get.lock().unwrap();
            if after_get.as_ref().is_some_and(|(at, _)| at == account) {
                let (_, hook) = after_get.take().unwrap();
                drop(after_get);
                hook();
            }
            result
        }

        fn set(&self, service: &str, account: &str, value: &str) -> keyring::Result<()> {
            if self.fail_with_no_access || self.writes_locked {
                return Err(no_access());
            }
            self.items.lock().unwrap().insert(
                (service.to_string(), account.to_string()),
                value.to_string(),
            );
            Ok(())
        }

        fn delete(&self, service: &str, account: &str) -> keyring::Result<()> {
            if self.fail_with_no_access || self.deletes_locked.load(Ordering::SeqCst) {
                return Err(no_access());
            }
            self.items
                .lock()
                .unwrap()
                .remove(&(service.to_string(), account.to_string()))
                .map(|_| ())
                .ok_or(keyring::Error::NoEntry)
        }
    }

    fn store() -> SecretStore<MemoryBackend> {
        with(MemoryBackend::default())
    }

    fn with(backend: MemoryBackend) -> SecretStore<MemoryBackend> {
        SecretStore::with_backend("net.ewal.longhand", backend)
    }

    /// The accounts in the store, sorted, with their values.
    fn accounts(store: &SecretStore<MemoryBackend>) -> Vec<(String, String)> {
        let mut items: Vec<(String, String)> = store
            .backend
            .items
            .lock()
            .unwrap()
            .iter()
            .map(|((_, account), value)| (account.clone(), value.clone()))
            .collect();
        items.sort();
        items
    }

    fn entry(account: &str, value: &str) -> (String, String) {
        (account.to_string(), value.to_string())
    }

    #[test]
    fn service_follows_the_bundle_identifier() {
        assert_eq!(store().service(), "net.ewal.longhand.secure-store");
    }

    #[test]
    fn absent_credential_is_none_not_an_error() {
        assert_eq!(
            store().provider_credential(CUSTOM, "https://api.example.com/v1", false),
            Ok(None)
        );
    }

    #[test]
    fn locked_store_is_a_typed_error_distinct_from_absent() {
        let store = with(MemoryBackend {
            fail_with_no_access: true,
            ..Default::default()
        });
        let error = store
            .provider_credential(CUSTOM, "https://api.example.com/v1", false)
            .unwrap_err();
        assert!(error.needs_unlock());
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn credential_round_trips_for_its_provider_and_host() {
        let store = store();
        store
            .set_provider_credential(CUSTOM, "https://api.example.com/v1", "sk-test")
            .unwrap();
        assert_eq!(
            store.provider_credential(CUSTOM, "https://API.example.com/other/path", false),
            Ok(Some("sk-test".to_string()))
        );
    }

    #[test]
    fn credential_for_one_host_is_absent_for_another() {
        let store = store();
        store
            .set_provider_credential(CUSTOM, "https://api.example.com/v1", "sk-test")
            .unwrap();
        assert_eq!(
            store.provider_credential(CUSTOM, "https://api.example.net/v1", false),
            Ok(None)
        );
        assert_eq!(
            store.provider_credential(CUSTOM, "https://api.example.com:8443/v1", false),
            Ok(None)
        );
    }

    #[test]
    fn credentials_live_under_the_summarize_scope_with_provider_and_host() {
        let store = store();
        store
            .set_provider_credential("ollama", "http://localhost:4000", "k")
            .unwrap();
        assert_eq!(
            store.get(SUMMARIZE_SCOPE, "ollama:localhost:4000"),
            Ok(Some("k".into()))
        );
        assert_eq!(
            accounts(&store),
            [entry("summarize:ollama:localhost:4000", "k")]
        );
    }

    /// Covers AE8.
    #[test]
    fn two_providers_on_one_host_keep_separate_keys() {
        let store = store();
        store
            .set_provider_credential("openrouter", "https://openrouter.ai/api/v1", "key-a")
            .unwrap();
        store
            .set_provider_credential(CUSTOM, "https://openrouter.ai/api/v1", "key-b")
            .unwrap();
        assert_eq!(
            store.provider_credential("openrouter", "https://openrouter.ai/api/v1", false),
            Ok(Some("key-a".into()))
        );
        assert_eq!(
            store.provider_credential(CUSTOM, "https://openrouter.ai/api/v1", false),
            Ok(Some("key-b".into()))
        );
        assert_eq!(
            accounts(&store),
            [
                entry("summarize:custom:openrouter.ai", "key-b"),
                entry("summarize:openrouter:openrouter.ai", "key-a"),
            ]
        );
    }

    #[test]
    fn the_lazy_move_copies_the_old_entry_and_then_deletes_it() {
        let store = with(MemoryBackend::holding(&[(
            "summarize:litellm.ewal.io",
            "sk-old",
        )]));
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(
            accounts(&store),
            [entry("summarize:custom:litellm.ewal.io", "sk-old")]
        );
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
    }

    #[test]
    fn the_lazy_move_runs_only_for_the_migrated_provider() {
        let store = with(MemoryBackend::holding(&[(
            "summarize:litellm.ewal.io",
            "sk-old",
        )]));
        assert_eq!(store.provider_credential(CUSTOM, LITELLM, false), Ok(None));
        assert_eq!(
            accounts(&store),
            [entry("summarize:litellm.ewal.io", "sk-old")]
        );
    }

    #[test]
    fn a_failed_write_during_the_move_keeps_the_old_entry() {
        let store = with(MemoryBackend {
            writes_locked: true,
            ..MemoryBackend::holding(&[("summarize:litellm.ewal.io", "sk-old")])
        });
        let error = store
            .provider_credential(CUSTOM, LITELLM, true)
            .unwrap_err();
        assert!(error.needs_unlock());
        assert_eq!(
            accounts(&store),
            [entry("summarize:litellm.ewal.io", "sk-old")]
        );
    }

    #[test]
    fn a_locked_keychain_during_the_move_leaves_both_entries_and_reports_locked() {
        let store = with(MemoryBackend {
            fail_with_no_access: true,
            ..MemoryBackend::holding(&[
                ("summarize:litellm.ewal.io", "sk-old"),
                ("summarize:custom:litellm.ewal.io", "sk-new"),
            ])
        });
        let error = store
            .provider_credential(CUSTOM, LITELLM, true)
            .unwrap_err();
        assert!(error.needs_unlock());
        assert_eq!(
            accounts(&store),
            [
                entry("summarize:custom:litellm.ewal.io", "sk-new"),
                entry("summarize:litellm.ewal.io", "sk-old"),
            ]
        );
    }

    #[test]
    fn a_failed_delete_after_the_move_returns_the_key_and_the_next_read_cleans_up() {
        let store = with(MemoryBackend::holding(&[(
            "summarize:litellm.ewal.io",
            "sk-old",
        )]));
        store.backend.deletes_locked.store(true, Ordering::SeqCst);
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(
            accounts(&store),
            [
                entry("summarize:custom:litellm.ewal.io", "sk-old"),
                entry("summarize:litellm.ewal.io", "sk-old"),
            ]
        );
        store.backend.deletes_locked.store(false, Ordering::SeqCst);
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(
            accounts(&store),
            [entry("summarize:custom:litellm.ewal.io", "sk-old")]
        );
    }

    #[test]
    fn with_both_entries_the_read_deletes_the_old_entry_and_keeps_the_new_key() {
        let store = with(MemoryBackend::holding(&[
            ("summarize:litellm.ewal.io", "sk-old"),
            ("summarize:custom:litellm.ewal.io", "sk-new"),
        ]));
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-new".into()))
        );
        assert_eq!(
            accounts(&store),
            [entry("summarize:custom:litellm.ewal.io", "sk-new")]
        );
    }

    #[test]
    fn remove_while_the_old_entry_exists_deletes_both() {
        let store = with(MemoryBackend::holding(&[
            ("summarize:litellm.ewal.io", "sk-old"),
            ("summarize:custom:litellm.ewal.io", "sk-new"),
        ]));
        store
            .delete_provider_credential(CUSTOM, LITELLM, true)
            .unwrap();
        assert_eq!(accounts(&store), []);
        assert_eq!(store.provider_credential(CUSTOM, LITELLM, true), Ok(None));
    }

    const OLD: &str = "summarize:litellm.ewal.io";
    const NEW: &str = "summarize:custom:litellm.ewal.io";

    type Shared = Arc<SecretStore<MemoryBackend>>;

    /// After the next read of `account` in `store`, start `other` on a new
    /// thread. The read continues when `other` has finished or when `other`
    /// waits for the credential lock. The returned slot gets the handle of
    /// the thread.
    fn interleave<T: Send + 'static>(
        store: &Shared,
        account: &str,
        other: impl FnOnce(&SecretStore<MemoryBackend>) -> T + Send + 'static,
    ) -> Arc<Mutex<Option<thread::JoinHandle<T>>>> {
        let (events, event) = mpsc::channel();
        *store.on_wait.lock().unwrap() = Some(events.clone());
        let slot = Arc::new(Mutex::new(None));
        let (shared, handle_slot) = (Arc::clone(store), Arc::clone(&slot));
        let hook = move || {
            let handle = thread::spawn(move || {
                let result = other(&shared);
                let _ = events.send("done");
                result
            });
            event.recv().unwrap();
            *handle_slot.lock().unwrap() = Some(handle);
        };
        *store.backend.after_get.lock().unwrap() = Some((account.into(), Box::new(hook)));
        slot
    }

    fn join<T>(slot: Arc<Mutex<Option<thread::JoinHandle<T>>>>) -> T {
        slot.lock().unwrap().take().unwrap().join().unwrap()
    }

    #[test]
    fn a_remove_during_the_lazy_move_is_not_undone() {
        let store: Shared = Arc::new(with(MemoryBackend::holding(&[(OLD, "sk-old")])));
        // The Remove starts after the read got the old entry and before it
        // writes the provider's entry.
        let remove = interleave(&store, OLD, |s| {
            s.delete_provider_credential(CUSTOM, LITELLM, true)
        });
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(join(remove), Ok(()));
        assert_eq!(accounts(&store), []);
    }

    #[test]
    fn a_save_during_the_lazy_move_keeps_the_saved_key() {
        let store: Shared = Arc::new(with(MemoryBackend::holding(&[(OLD, "sk-old")])));
        let save = interleave(&store, OLD, |s| {
            s.set_provider_credential(CUSTOM, LITELLM, "sk-new")
        });
        store.provider_credential(CUSTOM, LITELLM, true).unwrap();
        assert_eq!(join(save), Ok(()));
        assert_eq!(accounts(&store), [entry(NEW, "sk-new")]);
    }

    #[test]
    fn two_overlapping_reads_both_return_the_moved_key() {
        let store: Shared = Arc::new(with(MemoryBackend::holding(&[(OLD, "sk-old")])));
        // The second read starts after the first read found no provider
        // entry and before it reads the old entry.
        let second = interleave(&store, NEW, |s| {
            s.provider_credential(CUSTOM, LITELLM, true)
        });
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(join(second), Ok(Some("sk-old".into())));
        assert_eq!(accounts(&store), [entry(NEW, "sk-old")]);
    }

    #[test]
    fn a_key_that_another_process_moves_between_the_two_reads_is_returned() {
        let store: Shared = Arc::new(with(MemoryBackend::holding(&[(OLD, "sk-old")])));
        let shared = Arc::clone(&store);
        let other_process = move || {
            let mut items = shared.backend.items.lock().unwrap();
            let service = shared.service().to_string();
            let old = items.remove(&(service.clone(), OLD.into())).unwrap();
            items.insert((service, NEW.into()), old);
        };
        *store.backend.after_get.lock().unwrap() = Some((NEW.into(), Box::new(other_process)));
        assert_eq!(
            store.provider_credential(CUSTOM, LITELLM, true),
            Ok(Some("sk-old".into()))
        );
        assert_eq!(accounts(&store), [entry(NEW, "sk-old")]);
    }

    #[test]
    fn remove_for_another_provider_keeps_the_old_entry() {
        let store = with(MemoryBackend::holding(&[(
            "summarize:litellm.ewal.io",
            "sk-old",
        )]));
        store
            .delete_provider_credential(CUSTOM, LITELLM, false)
            .unwrap();
        assert_eq!(
            accounts(&store),
            [entry("summarize:litellm.ewal.io", "sk-old")]
        );
    }

    #[test]
    fn deleting_an_absent_credential_succeeds() {
        assert_eq!(
            store().delete_provider_credential(CUSTOM, "https://api.example.com", true),
            Ok(())
        );
    }

    #[test]
    fn a_provider_name_that_is_empty_or_has_a_colon_is_rejected() {
        for provider in ["", " ", "a:b"] {
            assert!(matches!(
                store().provider_credential(provider, "https://api.example.com", false),
                Err(SecretError::InvalidLocation { .. })
            ));
        }
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
