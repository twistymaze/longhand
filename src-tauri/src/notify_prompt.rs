//! The system notification of the "Still recording?" prompt (R5, R9, KTD10
//! of the forgotten-recording plan).
//!
//! The notification has no buttons and plays no sound. A click on it opens
//! Longhand. `tauri-plugin-notification` cannot remove a shown notification
//! on macOS or Windows, so this module uses native code:
//!
//! - macOS: the plugin shows the notification with no action type. The
//!   removal scans the delivered notifications of the app for the prompt
//!   title.
//! - Windows: Longhand shows and removes its own toast, with a tag and a
//!   group, through `windows::UI::Notifications`.
//!
//! Longhand registers no `on_action` handler: one handler makes the plugin
//! track every notification, with one waiting thread each.

use std::sync::{Mutex, MutexGuard};

use tauri::AppHandle;

use crate::managers::guard::{Prompt, PromptNotice};

/// The title of the prompt notification. The macOS removal finds the
/// notification by this title.
pub const TITLE: &str = "Still recording?";

/// The tag of the Windows toast.
#[cfg(any(windows, test))]
const TAG: &str = "still-recording";

/// The group of the Windows toast.
#[cfg(any(windows, test))]
const GROUP: &str = "longhand";

/// The body of the notification, with the countdown of the settings
/// snapshot.
pub fn body(countdown_minutes: u32) -> String {
    let unit = if countdown_minutes == 1 {
        "minute"
    } else {
        "minutes"
    };
    format!(
        "Longhand stops this note in {countdown_minutes} {unit}. Open Longhand to keep recording."
    )
}

/// Whether a delivered notification with `title` is a prompt notification.
#[cfg(any(target_os = "macos", test))]
fn is_prompt_title(title: Option<&str>) -> bool {
    title == Some(TITLE)
}

/// The XML of the Windows toast: the title and the body, silent audio, and
/// no actions. The text is fixed text with no XML characters.
#[cfg(any(windows, test))]
fn toast_xml(body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\">\
         <text>{TITLE}</text><text>{body}</text>\
         </binding></visual><audio silent=\"true\"/></toast>"
    )
}

/// The app id of `tauri-winrt-notification` for an app that is not
/// installed.
#[cfg(any(windows, test))]
const POWERSHELL_APP_ID: &str =
    "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

/// The app id of the toast for the executable folder `exe_dir`: the same
/// app id that `tauri-plugin-notification` uses, so that a click opens
/// Longhand.
#[cfg(any(windows, test))]
fn app_id(exe_dir: &str, identifier: &str) -> String {
    let dev = ["debug", "release"].iter().any(|profile| {
        exe_dir.ends_with(&format!("\\target\\{profile}"))
            || exe_dir.ends_with(&format!("/target/{profile}"))
    });
    if dev {
        POWERSHELL_APP_ID.to_string()
    } else {
        identifier.to_string()
    }
}

/// The native prompt notification.
pub struct NativeNotice {
    app: AppHandle,
    /// The generation of the prompt that the notification shows.
    shown: Mutex<Option<u64>>,
}

impl NativeNotice {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            shown: Mutex::new(None),
        }
    }

    fn shown(&self) -> MutexGuard<'_, Option<u64>> {
        self.shown.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl PromptNotice for NativeNotice {
    fn show(&self, prompt: &Prompt) {
        let mut shown = self.shown();
        // A check that joined changes no text of the notification, so no
        // second banner shows.
        if *shown == Some(prompt.generation) {
            return;
        }
        if shown.is_some() {
            platform::remove(&self.app);
        }
        platform::show(&self.app, &body(prompt.countdown_minutes));
        *shown = Some(prompt.generation);
    }

    fn remove(&self, generation: u64) {
        let mut shown = self.shown();
        // The notification of a newer prompt stays.
        if shown.is_some_and(|g| g != generation) {
            return;
        }
        *shown = None;
        platform::remove(&self.app);
    }
}

/// Remove a prompt notification that a crash or a quit left (KTD10).
pub fn remove_leftover(app: &AppHandle) {
    platform::remove(app);
}

#[cfg(target_os = "macos")]
mod platform {
    use tauri::AppHandle;

    use super::{TITLE, is_prompt_title};

    /// Show the notification through the plugin, with no action type and
    /// no sound.
    pub fn show(app: &AppHandle, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(TITLE).body(body).show() {
            log::warn!("could not show the prompt notification: {e}");
        }
    }

    /// Remove each delivered notification of the app with the prompt
    /// title. `NSUserNotificationCenter` is the API that the plugin uses
    /// to show it.
    #[allow(deprecated)]
    pub fn remove(_app: &AppHandle) {
        use objc2_foundation::NSUserNotificationCenter;
        let center = NSUserNotificationCenter::defaultUserNotificationCenter();
        for notification in center.deliveredNotifications().iter() {
            let title = notification.title().map(|t| t.to_string());
            if is_prompt_title(title.as_deref()) {
                center.removeDeliveredNotification(&notification);
                log::info!("the prompt notification is removed");
            }
        }
    }
}

#[cfg(windows)]
mod platform {
    use tauri::AppHandle;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    use super::{GROUP, TAG, app_id, toast_xml};

    /// The app id of the toast for the running executable.
    fn current_app_id(app: &AppHandle) -> Option<String> {
        let exe = tauri::utils::platform::current_exe()
            .inspect_err(|e| log::warn!("could not find the executable: {e}"))
            .ok()?;
        let dir = exe.parent()?.display().to_string();
        Some(app_id(&dir, &app.config().identifier))
    }

    /// Show the toast with its tag and group, silent and with no actions.
    pub fn show(app: &AppHandle, body: &str) {
        let Some(app_id) = current_app_id(app) else {
            return;
        };
        let shown = (|| {
            let xml = XmlDocument::new()?;
            xml.LoadXml(&HSTRING::from(toast_xml(body)))?;
            let toast = ToastNotification::CreateToastNotification(&xml)?;
            toast.SetTag(&HSTRING::from(TAG))?;
            toast.SetGroup(&HSTRING::from(GROUP))?;
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(&app_id))?
                .Show(&toast)
        })();
        if let Err(e) = shown {
            log::warn!("could not show the prompt notification: {e}");
        }
    }

    /// Remove the toast by its tag and group, also from the notification
    /// centre.
    pub fn remove(app: &AppHandle) {
        let Some(app_id) = current_app_id(app) else {
            return;
        };
        let removed = ToastNotificationManager::History().and_then(|history| {
            history.RemoveGroupedTagWithId(
                &HSTRING::from(TAG),
                &HSTRING::from(GROUP),
                &HSTRING::from(&app_id),
            )
        });
        if let Err(e) = removed {
            log::warn!("could not remove the prompt notification: {e}");
        }
    }
}

/// Other platforms: the plugin shows the notification, and nothing
/// removes it.
#[cfg(not(any(target_os = "macos", windows)))]
mod platform {
    use tauri::AppHandle;

    use super::TITLE;

    pub fn show(app: &AppHandle, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app.notification().builder().title(TITLE).body(body).show() {
            log::warn!("could not show the prompt notification: {e}");
        }
    }

    pub fn remove(_app: &AppHandle) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_names_the_countdown_of_the_snapshot() {
        assert_eq!(
            body(2),
            "Longhand stops this note in 2 minutes. Open Longhand to keep recording."
        );
        assert_eq!(
            body(10),
            "Longhand stops this note in 10 minutes. Open Longhand to keep recording."
        );
        assert_eq!(
            body(1),
            "Longhand stops this note in 1 minute. Open Longhand to keep recording."
        );
    }

    #[test]
    fn the_macos_scan_matches_only_the_prompt_title() {
        assert!(is_prompt_title(Some("Still recording?")));
        assert!(!is_prompt_title(Some("Note ready")));
        assert!(!is_prompt_title(Some("Still recording")));
        assert!(!is_prompt_title(Some("")));
        assert!(!is_prompt_title(None));
    }

    #[test]
    fn the_toast_is_silent_and_has_no_actions() {
        let xml = toast_xml(&body(2));
        assert!(xml.starts_with("<toast>"), "{xml}");
        assert!(xml.contains("<text>Still recording?</text>"), "{xml}");
        assert!(
            xml.contains(
                "<text>Longhand stops this note in 2 minutes. Open Longhand to keep recording.</text>"
            ),
            "{xml}"
        );
        assert!(xml.contains(r#"<audio silent="true"/>"#), "{xml}");
        assert!(!xml.contains("<actions"), "{xml}");
        assert!(!xml.contains("<action "), "{xml}");
        // No scenario: a reminder or an alarm toast shows a button.
        assert!(!xml.contains("scenario"), "{xml}");
    }

    #[test]
    fn the_toast_uses_the_app_id_of_the_installed_app() {
        assert_eq!(
            app_id(r"C:\Program Files\Longhand", "net.ewal.longhand"),
            "net.ewal.longhand"
        );
        // A build that runs from the target folder has no registered app
        // id, so the toast uses the PowerShell app id, as the plugin does.
        assert_eq!(
            app_id(r"C:\src\longhand\target\release", "net.ewal.longhand"),
            POWERSHELL_APP_ID
        );
        assert_eq!(
            app_id(r"C:\src\longhand\target\debug", "net.ewal.longhand"),
            POWERSHELL_APP_ID
        );
    }

    #[test]
    fn the_toast_tag_and_group_identify_the_prompt() {
        assert_eq!(TAG, "still-recording");
        assert_eq!(GROUP, "longhand");
        // Windows limits a tag and a group to 64 characters.
        assert!(TAG.len() <= 64 && GROUP.len() <= 64);
    }
}
