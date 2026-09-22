//! Owns the process's [`SecretStore`], named after the bundle identifier so
//! the keychain service is stable for as long as the identifier is.

use std::sync::Arc;

use secrets::{SecretError, SecretStore};
use tauri::AppHandle;

pub struct SecretsManager {
    store: Arc<SecretStore>,
}

impl SecretsManager {
    pub fn new(app: &AppHandle) -> Self {
        Self {
            store: Arc::new(SecretStore::new(&app.config().identifier)),
        }
    }

    /// Run a store operation on the blocking pool: a keychain call can wait on
    /// a system prompt, and must never hold an async worker while it does.
    pub async fn run<T, F>(&self, op: F) -> Result<T, SecretError>
    where
        T: Send + 'static,
        F: FnOnce(&SecretStore) -> Result<T, SecretError> + Send + 'static,
    {
        let store = Arc::clone(&self.store);
        tauri::async_runtime::spawn_blocking(move || op(&store))
            .await
            .map_err(|e| SecretError::Platform {
                message: e.to_string(),
            })?
    }
}
