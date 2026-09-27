//! One saved native webview scale for all app windows.
use crate::managers::settings;
use serde::Deserialize;
use specta::Type;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};

// Window creation and changes hold this lock through native application.
static OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum ZoomAction {
    Increase,
    Decrease,
    Reset,
}

pub fn normalize(percent: u16) -> u16 {
    if (50..=200).contains(&percent) && percent.is_multiple_of(10) {
        percent
    } else {
        100
    }
}

fn transact(
    current: u16,
    action: ZoomAction,
    save: impl FnOnce(u16) -> Result<(), String>,
    apply: impl FnOnce(u16) -> Result<(), String>,
) -> Result<u16, String> {
    let current = normalize(current);
    let next = match action {
        ZoomAction::Increase => (current + 10).min(200),
        ZoomAction::Decrease => current.saturating_sub(10).max(50),
        ZoomAction::Reset => 100,
    };
    save(next)?;
    apply(next)?;
    Ok(next)
}

pub fn change(app: &AppHandle, action: ZoomAction) -> Result<u16, String> {
    let _guard = OPERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    transact(
        settings::get_settings(app).interface_zoom_percent,
        action,
        |percent| {
            settings::update_settings(app, |s| {
                s.interface_zoom_percent = percent;
                Ok(())
            })
            .map(drop)
        },
        |percent| {
            let errors: Vec<_> = app
                .webview_windows()
                .values()
                .filter_map(|window| apply(window, percent).err())
                .collect();
            if errors.is_empty() {
                Ok(())
            } else {
                Err(errors.join("; "))
            }
        },
    )
}

/// Hold the same operation lock while creating a hidden window at its saved scale.
pub fn build(
    app: &AppHandle,
    create: impl FnOnce(u16) -> tauri::Result<WebviewWindow>,
) -> tauri::Result<WebviewWindow> {
    let _guard = OPERATIONS.lock().unwrap_or_else(|e| e.into_inner());
    let percent = normalize(settings::get_settings(app).interface_zoom_percent);
    let window = create(percent)?;
    if let Err(error) = window.set_zoom(f64::from(percent) / 100.0) {
        let _ = window.destroy();
        return Err(error);
    }
    Ok(window)
}

pub fn initialization(percent: u16) -> String {
    format!(
        "window.__interfaceScale ??= '{}'; document.addEventListener('DOMContentLoaded', () => document.documentElement.style.setProperty('--interface-scale', window.__interfaceScale));",
        f64::from(percent) / 100.0
    )
}

fn apply(window: &WebviewWindow, percent: u16) -> Result<(), String> {
    window
        .set_zoom(f64::from(percent) / 100.0)
        .map_err(|e| format!("{}: {e}", window.label()))?;
    window.eval(&format!("window.__interfaceScale = '{}'; document.documentElement?.style.setProperty('--interface-scale', window.__interfaceScale);", f64::from(percent) / 100.0)).map_err(|e| format!("{}: {e}", window.label()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    #[test]
    fn concurrent_windows_save_and_apply_in_one_order() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicU16, Ordering},
        };
        let saved = Arc::new(AtomicU16::new(100));
        let applied = Arc::new(Mutex::new(Vec::new()));
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let saved = Arc::clone(&saved);
                let applied = Arc::clone(&applied);
                scope.spawn(move || {
                    for _ in 0..3 {
                        let _guard = OPERATIONS.lock().unwrap();
                        transact(
                            saved.load(Ordering::SeqCst),
                            ZoomAction::Increase,
                            |p| {
                                saved.store(p, Ordering::SeqCst);
                                Ok(())
                            },
                            |p| {
                                applied.lock().unwrap().push(p);
                                Ok(())
                            },
                        )
                        .unwrap();
                    }
                });
            }
        });
        assert_eq!(saved.load(Ordering::SeqCst), 160);
        assert_eq!(*applied.lock().unwrap(), [110, 120, 130, 140, 150, 160]);
    }
    #[test]
    fn bounds_reset_and_invalid_values() {
        let mut current = 100;
        for _ in 0..30 {
            current = transact(current, ZoomAction::Increase, |_| Ok(()), |_| Ok(())).unwrap();
        }
        assert_eq!(current, 200);
        for _ in 0..30 {
            current = transact(current, ZoomAction::Decrease, |_| Ok(()), |_| Ok(())).unwrap();
        }
        assert_eq!(current, 50);
        assert_eq!(
            transact(current, ZoomAction::Reset, |_| Ok(()), |_| Ok(())).unwrap(),
            100
        );
        for invalid in [0, 49, 51, 201, u16::MAX] {
            assert_eq!(normalize(invalid), 100);
        }
    }
    #[test]
    fn save_failure_does_not_apply_and_apply_failure_keeps_saved_value() {
        assert_eq!(
            transact(
                100,
                ZoomAction::Increase,
                |_| Err("disk".into()),
                |_| panic!("must not apply")
            ),
            Err("disk".into())
        );
        let saved = Cell::new(100);
        assert!(
            transact(
                saved.get(),
                ZoomAction::Increase,
                |p| {
                    saved.set(p);
                    Ok(())
                },
                |_| Err("window".into())
            )
            .is_err()
        );
        assert_eq!(saved.get(), 110);
        assert_eq!(
            transact(
                saved.get(),
                ZoomAction::Increase,
                |p| {
                    saved.set(p);
                    Ok(())
                },
                |_| Ok(())
            )
            .unwrap(),
            120
        );
    }
}
