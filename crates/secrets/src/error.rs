//! The secret-store error taxonomy. The platform mapping follows
//! `reference-source/anarlog/plugins/store2/src/commands.rs` (`secure_store_error`,
//! `keychain_error_code`); the typed variants are new, so a caller can tell
//! "the store needs unlocking" from "the store failed" (R20b).

/// macOS `errSecAuthFailed`: the keychain refused this application, usually
/// because an item's ACL was captured by a different code signature.
#[cfg(target_os = "macos")]
pub(crate) const ERR_SEC_AUTH_FAILED: i32 = -25293;
/// macOS `errSecMissingEntitlement`: the binary is not signed in a way that
/// entitles it to the keychain it asked for.
#[cfg(target_os = "macos")]
pub(crate) const ERR_SEC_MISSING_ENTITLEMENT: i32 = -34018;
/// macOS `errSecInteractionNotAllowed`: the keychain is locked and no UI may be shown.
#[cfg(target_os = "macos")]
pub(crate) const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
/// macOS `errSecUserCanceled`: the user dismissed the keychain prompt.
#[cfg(target_os = "macos")]
pub(crate) const ERR_SEC_USER_CANCELED: i32 = -128;

#[cfg(target_os = "macos")]
const MACOS_KEYCHAIN_ACCESS_ERROR: &str = "macOS couldn't access your login Keychain. Use \u{201c}Repair Keychain Access\u{201d}, then try again.";
#[cfg(target_os = "linux")]
const LINUX_SECRET_SERVICE_ACCESS_ERROR: &str =
    "Linux couldn't access Secret Service. Unlock your login keyring, then try again.";
#[cfg(target_os = "linux")]
const LINUX_SECRET_SERVICE_UNAVAILABLE_ERROR: &str =
    "Linux Secret Service is unavailable. Start your desktop keyring service, then try again.";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// The store exists but is locked or refused access. Recoverable by the
    /// user (unlock, allow, repair); a stage waiting on it is blocked, not failed.
    #[error("{message}")]
    Locked { message: String },
    /// The binary's signature does not entitle it to the store. A build
    /// problem, never something the user can fix from the app.
    #[error("{message}")]
    MissingEntitlement { message: String },
    /// No secret store is running on this machine.
    #[error("{message}")]
    Unavailable { message: String },
    /// The caller asked for an empty or malformed location.
    #[error("{message}")]
    InvalidLocation { message: String },
    /// Any other platform failure, with the platform's own message.
    #[error("{message}")]
    Platform { message: String },
}

impl SecretError {
    /// Whether a stage needing this credential should wait for the user
    /// rather than fail (R20b).
    pub fn needs_unlock(&self) -> bool {
        matches!(self, SecretError::Locked { .. })
    }

    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        SecretError::InvalidLocation {
            message: message.into(),
        }
    }
}

/// Map a keyring failure onto the taxonomy. `NoEntry` is never an error here;
/// callers turn it into an absent credential before reaching this.
pub(crate) fn from_keyring(error: keyring::Error) -> SecretError {
    #[cfg(target_os = "macos")]
    if let Some(code) = keychain_error_code(&error) {
        let platform = error.to_string();
        match code {
            ERR_SEC_AUTH_FAILED | ERR_SEC_USER_CANCELED => {
                return SecretError::Locked {
                    message: MACOS_KEYCHAIN_ACCESS_ERROR.to_string(),
                };
            }
            ERR_SEC_INTERACTION_NOT_ALLOWED => {
                return SecretError::Locked { message: platform };
            }
            ERR_SEC_MISSING_ENTITLEMENT => {
                return SecretError::MissingEntitlement { message: platform };
            }
            _ => {}
        }
    }

    match error {
        #[cfg(target_os = "linux")]
        keyring::Error::NoStorageAccess(_) => SecretError::Locked {
            message: LINUX_SECRET_SERVICE_ACCESS_ERROR.to_string(),
        },
        #[cfg(target_os = "linux")]
        keyring::Error::PlatformFailure(_) => SecretError::Unavailable {
            message: LINUX_SECRET_SERVICE_UNAVAILABLE_ERROR.to_string(),
        },
        keyring::Error::NoStorageAccess(_) => SecretError::Locked {
            message: error.to_string(),
        },
        keyring::Error::NoDefaultStore => SecretError::Unavailable {
            message: error.to_string(),
        },
        keyring::Error::Invalid(..) | keyring::Error::TooLong(..) => SecretError::InvalidLocation {
            message: error.to_string(),
        },
        other => SecretError::Platform {
            message: other.to_string(),
        },
    }
}

#[cfg(target_os = "macos")]
fn keychain_error_code(error: &keyring::Error) -> Option<i32> {
    let source = match error {
        keyring::Error::PlatformFailure(source) | keyring::Error::NoStorageAccess(source) => source,
        _ => return None,
    };
    source
        .downcast_ref::<security_framework::base::Error>()
        .map(|error| error.code())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(message: &str) -> Box<dyn std::error::Error + Send + Sync> {
        Box::new(std::io::Error::other(message.to_string()))
    }

    #[test]
    fn no_storage_access_is_locked_and_keeps_a_message() {
        let mapped = from_keyring(keyring::Error::NoStorageAccess(io("keychain locked")));
        assert!(mapped.needs_unlock());
        assert!(matches!(mapped, SecretError::Locked { ref message } if !message.is_empty()));
    }

    #[test]
    fn missing_store_is_unavailable_not_locked() {
        let mapped = from_keyring(keyring::Error::NoDefaultStore);
        assert!(matches!(mapped, SecretError::Unavailable { .. }));
        assert!(!mapped.needs_unlock());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_auth_failure_points_at_repair() {
        let error = keyring::Error::PlatformFailure(Box::new(
            security_framework::base::Error::from_code(ERR_SEC_AUTH_FAILED),
        ));
        assert_eq!(
            from_keyring(error),
            SecretError::Locked {
                message: MACOS_KEYCHAIN_ACCESS_ERROR.to_string()
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_missing_entitlement_is_a_signing_problem() {
        let error = keyring::Error::PlatformFailure(Box::new(
            security_framework::base::Error::from_code(ERR_SEC_MISSING_ENTITLEMENT),
        ));
        let mapped = from_keyring(error);
        assert!(matches!(mapped, SecretError::MissingEntitlement { .. }));
        assert!(!mapped.needs_unlock());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unrelated_failure_keeps_the_platform_message() {
        let platform = security_framework::base::Error::from_code(-25299);
        let expected = format!("Platform failure: {platform}");
        let mapped = from_keyring(keyring::Error::PlatformFailure(Box::new(platform)));
        assert_eq!(mapped, SecretError::Platform { message: expected });
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_locked_and_absent_are_distinct() {
        let locked = from_keyring(keyring::Error::NoStorageAccess(io("locked")));
        let absent = from_keyring(keyring::Error::PlatformFailure(io("unavailable")));
        assert!(matches!(locked, SecretError::Locked { .. }));
        assert!(matches!(absent, SecretError::Unavailable { .. }));
    }
}
