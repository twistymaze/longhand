mod commands;
mod managers;
mod probe;
mod tray;
mod window;

use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};

/// Where the generated TypeScript bindings land, relative to this crate.
pub const BINDINGS_PATH: &str = "../src/bindings.ts";

/// The command and event set exposed to the frontend. Kept in one function so
/// the running app and the `export_bindings` test generate identical bindings.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::settings::get_probe_setting,
            commands::settings::set_probe_setting,
            commands::settings::credential_status,
            commands::settings::set_credential,
            commands::settings::delete_credential,
            commands::settings::repair_keychain_access,
            probe::probe_status,
        ])
        .events(collect_events![probe::ProbeShortcutFired])
}

pub fn export_bindings(builder: &Builder<tauri::Wry>) {
    builder
        .export(
            specta_typescript::Typescript::default()
                .bigint(specta_typescript::BigIntExportBehavior::Number)
                .header("// @ts-nocheck"),
            BINDINGS_PATH,
        )
        .expect("failed to export TypeScript bindings");
}

pub fn run() {
    let specta_builder = specta_builder();

    #[cfg(debug_assertions)]
    export_bindings(&specta_builder);

    let invoke_handler = specta_builder.invoke_handler();

    let builder = tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_macos_permissions::init());

    let app = builder
        .setup(move |app| {
            specta_builder.mount_events(app);

            let handle = app.handle().clone();
            app.manage(managers::secrets::SecretsManager::new(&handle));
            // Read once at launch so a first run writes the defaults and an old
            // store migrates before anything else reads it.
            let settings = managers::settings::get_settings(&handle);
            log::info!(
                "settings loaded (schema version {})",
                settings.settings_schema_version
            );

            window::create_main_window(&handle)?;
            tray::create_tray(&handle)?;
            probe::register(&handle);

            window::show_main_window(&handle);
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            // Launching the running bundle again (Finder, Spotlight, Dock) is a
            // request to see the window it hid on close.
            window::show_main_window(app);
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (app, event);
    });
}

#[cfg(test)]
mod tests {
    /// `cargo test -p longhand export_bindings` regenerates `src/bindings.ts`
    /// without launching the app; CI diffs the result against the committed file.
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::specta_builder());
    }
}
