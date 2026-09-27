//! OS-owned login registration. Adapted from Handy; see PROVENANCE.md.
use serde::Serialize;
use specta::Type;
use tauri::AppHandle;

pub const LOGIN_ARGUMENT: &str = "--longhand-login";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum LoginItemStatus {
    Disabled,
    Enabled,
    ApprovalRequired,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchReason {
    Manual,
    Login,
}

pub fn has_login_argument(args: impl IntoIterator<Item = impl AsRef<str>>) -> bool {
    args.into_iter().any(|arg| arg.as_ref() == LOGIN_ARGUMENT)
}

#[cfg(any(target_os = "macos", test))]
fn is_login_event(class: u32, event: u32, property: Option<u32>) -> bool {
    class == u32::from_be_bytes(*b"aevt")
        && event == u32::from_be_bytes(*b"oapp")
        && property == Some(u32::from_be_bytes(*b"lgit"))
}

/// Read before setup starts work: the Apple event exists only during dispatch.
pub fn launch_reason() -> LaunchReason {
    #[cfg(target_os = "macos")]
    let login = macos::current_event_is_login();
    #[cfg(not(target_os = "macos"))]
    let login = has_login_argument(std::env::args());
    if login {
        LaunchReason::Login
    } else {
        LaunchReason::Manual
    }
}

pub fn background_request(args: &[String]) -> bool {
    if has_login_argument(args) {
        return true;
    }
    #[cfg(target_os = "macos")]
    return macos::current_event_is_login();
    #[cfg(not(target_os = "macos"))]
    false
}

pub fn status(app: &AppHandle) -> Result<LoginItemStatus, String> {
    #[cfg(target_os = "macos")]
    {
        let _ = app;
        Ok(macos::status())
    }
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch()
            .is_enabled()
            .map(|enabled| {
                if enabled {
                    LoginItemStatus::Enabled
                } else {
                    LoginItemStatus::Disabled
                }
            })
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = app;
        Ok(LoginItemStatus::Unavailable)
    }
}

pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<LoginItemStatus, String> {
    #[cfg(target_os = "macos")]
    macos::set_enabled(enabled)?;
    #[cfg(target_os = "windows")]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = enabled;
        return Err("Start at login is unavailable on this system".into());
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    status(app)
}

pub fn open_settings() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        unsafe {
            objc2_service_management::SMAppService::openSystemSettingsLoginItems();
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    Err("Login item settings are unavailable on this system".into())
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use objc2_foundation::NSAppleEventManager;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    pub fn current_event_is_login() -> bool {
        let Some(event) = NSAppleEventManager::sharedAppleEventManager().currentAppleEvent() else {
            return false;
        };
        let property = event
            .paramDescriptorForKeyword(u32::from_be_bytes(*b"prdt"))
            .map(|value| value.enumCodeValue());
        is_login_event(event.eventClass(), event.eventID(), property)
    }

    pub fn status() -> LoginItemStatus {
        classify_status(unsafe { SMAppService::mainAppService().status() })
    }

    fn classify_status(status: SMAppServiceStatus) -> LoginItemStatus {
        match status {
            // A main app can return NotFound before its first registration.
            // Registration is still supported, so leave the toggle available.
            SMAppServiceStatus::NotRegistered | SMAppServiceStatus::NotFound => {
                LoginItemStatus::Disabled
            }
            SMAppServiceStatus::Enabled => LoginItemStatus::Enabled,
            SMAppServiceStatus::RequiresApproval => LoginItemStatus::ApprovalRequired,
            _ => LoginItemStatus::Unavailable,
        }
    }

    pub fn set_enabled(enabled: bool) -> Result<(), String> {
        let service = unsafe { SMAppService::mainAppService() };
        match (enabled, status()) {
            (true, LoginItemStatus::Enabled) | (false, LoginItemStatus::Disabled) => Ok(()),
            (true, _) => unsafe { service.registerAndReturnError() }.map_err(|e| e.to_string()),
            (false, _) => unsafe { service.unregisterAndReturnError() }.map_err(|e| e.to_string()),
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn a_main_app_not_yet_known_to_service_management_can_be_enabled() {
            assert_eq!(
                classify_status(SMAppServiceStatus::NotFound),
                LoginItemStatus::Disabled
            );
        }

        #[test]
        fn registered_approval_and_unknown_states_stay_distinct() {
            assert_eq!(
                classify_status(SMAppServiceStatus::NotRegistered),
                LoginItemStatus::Disabled
            );
            assert_eq!(
                classify_status(SMAppServiceStatus::Enabled),
                LoginItemStatus::Enabled
            );
            assert_eq!(
                classify_status(SMAppServiceStatus::RequiresApproval),
                LoginItemStatus::ApprovalRequired
            );
            assert_eq!(
                classify_status(SMAppServiceStatus(99)),
                LoginItemStatus::Unavailable
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_exact_argument_requests_background_launch() {
        assert!(has_login_argument(["Longhand", LOGIN_ARGUMENT]));
        assert!(!has_login_argument(["Longhand"]));
        assert!(!has_login_argument(["--longhand-login=false"]));
    }

    #[test]
    fn only_the_login_open_event_requests_background_launch() {
        let class = u32::from_be_bytes(*b"aevt");
        let open = u32::from_be_bytes(*b"oapp");
        let login = Some(u32::from_be_bytes(*b"lgit"));
        assert!(is_login_event(class, open, login));
        assert!(!is_login_event(class, open, None));
        assert!(!is_login_event(class, u32::from_be_bytes(*b"rapp"), login));
        assert!(!is_login_event(0, open, login));
    }
}
