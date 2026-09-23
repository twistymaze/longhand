mod commands;
mod managers;
mod shortcut;
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
            commands::settings::get_settings,
            commands::settings::set_endpoint,
            commands::settings::set_context_limit,
            commands::settings::set_default_speaker_count,
            commands::settings::set_asr_model,
            commands::settings::choose_destination,
            commands::settings::set_current_destination,
            commands::settings::credential_status,
            commands::settings::set_credential,
            commands::settings::delete_credential,
            commands::settings::repair_keychain_access,
            commands::models::model_statuses,
            commands::models::download_models,
            commands::models::cancel_model_downloads,
            commands::readiness::readiness,
            commands::capture::start_recording,
            commands::capture::stop_recording,
            commands::capture::resume_recording,
            commands::capture::recording_status,
            commands::capture::begin_note,
            commands::capture::edit_note,
            commands::capture::scratchpad_notes,
            commands::capture::set_session_speaker_count,
            commands::capture::shortcut_status,
            commands::processing::regenerate_summary,
            commands::processing::retry_session,
            commands::processing::unblock_session,
            commands::session::list_sessions,
            commands::session::session_detail,
            commands::session::session_notes,
            commands::session::session_document,
        ])
        .events(collect_events![
            managers::models::ModelDownloadEvent,
            managers::capture::CaptureEnded,
            managers::capture::CaptureTick,
            managers::capture::RecordingChanged,
            managers::processing::ProcessingUpdate,
        ])
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
            // onnxruntime logs every graph pass at INFO; left in, it rotates
            // the capture warnings out of the default 40 KB file in minutes.
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .level_for("ort", log::LevelFilter::Warn)
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(4))
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
            let secrets = managers::secrets::SecretsManager::new(&handle);
            let secret_store = secrets.store();
            app.manage(secrets);
            let models = std::sync::Arc::new(managers::models::ModelsManager::new(&handle)?);
            app.manage(std::sync::Arc::clone(&models));
            let capture = std::sync::Arc::new(
                managers::capture::CaptureManager::new(&handle)
                    .map_err(|e| Box::<dyn std::error::Error>::from(e.to_string()))?,
            );
            app.manage(std::sync::Arc::clone(&capture));
            // Sleep closes the segment on the notifying thread, before the
            // machine stops; the wake's restart runs off it (U13).
            let sleeping = std::sync::Arc::clone(&capture);
            let sleep_app = handle.clone();
            let watch = power::SleepWatch::start(move |event| match event {
                power::PowerEvent::WillSleep => sleeping.will_sleep(&sleep_app),
                power::PowerEvent::DidWake => {
                    let capture = std::sync::Arc::clone(&sleeping);
                    let app = sleep_app.clone();
                    std::thread::spawn(move || capture.did_wake(&app));
                }
            });
            app.manage(std::sync::Mutex::new(watch));
            let processing = std::sync::Arc::new(managers::processing::ProcessingManager::new(
                &handle,
                &capture,
                models,
                secret_store,
            ));
            processing.start();
            app.manage(processing);
            // Resolving the microphone queries the audio HAL; keep it off the
            // launch path. A failure here is retried at the first press.
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = capture.prepare() {
                    log::warn!("microphone not prepared at launch: {e}");
                }
            });
            // Read once at launch so a first run writes the defaults and an old
            // store migrates before anything else reads it.
            let settings = managers::settings::get_settings(&handle);
            log::info!(
                "settings loaded (schema version {})",
                settings.settings_schema_version
            );

            window::create_main_window(&handle)?;
            tray::create_tray(&handle)?;
            shortcut::register(&handle);

            window::show_main_window(&handle);
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| match event {
        // Free the recognition engine before static destructors run.
        tauri::RunEvent::Exit => {
            if let Some(processing) =
                app.try_state::<std::sync::Arc<managers::processing::ProcessingManager>>()
            {
                processing.shutdown();
            }
        }
        #[cfg(target_os = "macos")]
        tauri::RunEvent::Reopen { .. } => {
            // Launching the running bundle again (Finder, Spotlight, Dock) is a
            // request to see the window it hid on close.
            window::show_main_window(app);
        }
        _ => {}
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
