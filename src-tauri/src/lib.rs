mod audio;
mod commands;
mod managers;
#[cfg(target_os = "macos")]
mod menu;
mod shortcut;
mod tray;
mod window;
#[cfg(target_os = "macos")]
mod zoom;

use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};

/// Where the generated TypeScript bindings are written, relative to this crate.
pub const BINDINGS_PATH: &str = "../src/bindings.ts";

/// The command and event set exposed to the frontend. Kept in one function so
/// the running app and the `export_bindings` test generate identical bindings.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::settings::get_settings,
            commands::settings::set_endpoint_base_url,
            commands::settings::set_endpoint_model,
            commands::settings::set_context_limit,
            commands::settings::set_default_speaker_count,
            commands::settings::select_asr_model,
            commands::settings::set_live_transcript,
            commands::settings::list_microphones,
            commands::settings::set_microphone,
            commands::settings::set_system_audio_default,
            commands::settings::summary_prompts,
            commands::settings::create_summary_prompt,
            commands::settings::duplicate_summary_prompt,
            commands::settings::rename_summary_prompt,
            commands::settings::set_summary_prompt_text,
            commands::settings::delete_summary_prompt,
            commands::settings::activate_summary_prompt,
            commands::settings::choose_destination,
            commands::settings::set_current_destination,
            commands::settings::credential_status,
            commands::settings::set_credential,
            commands::settings::delete_credential,
            commands::settings::repair_keychain_access,
            commands::settings::list_endpoint_models,
            commands::models::model_statuses,
            commands::models::download_models,
            commands::models::cancel_model_downloads,
            commands::models::speaker_models_status,
            commands::models::delete_model,
            commands::readiness::readiness,
            commands::readiness::system_audio_permission,
            commands::readiness::request_system_audio_permission,
            commands::readiness::open_system_audio_settings,
            commands::capture::start_recording,
            commands::capture::stop_recording,
            commands::capture::resume_recording,
            commands::capture::recording_status,
            commands::capture::set_note_microphone,
            commands::capture::set_note_system_audio,
            commands::capture::set_session_speaker_count,
            commands::capture::shortcut_status,
            commands::capture::set_shortcut,
            commands::capture::suspend_shortcuts,
            commands::capture::resume_shortcuts,
            commands::alerts::play_alert_sound,
            commands::processing::regenerate_summary,
            commands::processing::transcribe_again,
            commands::processing::retry_session,
            commands::processing::unblock_session,
            commands::session::list_sessions,
            commands::session::session_detail,
            commands::session::load_session_notes,
            commands::session::save_session_notes,
            commands::session::rename_session,
            commands::session::session_document,
            commands::session::live_transcript,
            commands::session::delete_session,
            commands::session::delete_session_permanently,
            commands::window::open_settings,
            commands::window::close_settings_window,
            commands::window::open_licences,
            commands::licences::licence_text,
            commands::webview_log::log_from_webview,
        ])
        .events(collect_events![
            managers::models::ModelDownloadEvent,
            managers::settings::SettingsChanged,
            managers::capture::CaptureEnded,
            managers::capture::CaptureTick,
            managers::capture::RecordingChanged,
            managers::processing::ProcessingUpdate,
            managers::library::LibraryChanged,
            managers::live::LiveTranscriptEvent,
            managers::live::LivePartSaved,
            managers::live::LiveTranscriptStatus,
            tray::ShowSession,
        ])
        .constant("DEFAULT_SHORTCUT", managers::settings::DEFAULT_SHORTCUT)
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

/// Take the single-instance lock in the app's data directory before the
/// store opens (U19). A live holder means another copy is running that the
/// single-instance plugin did not reach (a direct launch of the binary):
/// this one says so and exits before touching anything. A lock that cannot
/// be tried leaves liveness unknown, and launch then trusts only stale
/// heartbeats and leaves recording sessions alone.
fn take_instance_lock(
    app: &tauri::AppHandle,
) -> Result<session_core::jobs::Liveness, Box<dyn std::error::Error>> {
    use session_core::lock::{self, Acquired};
    let data = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data)?;
    let instance = uuid::Uuid::new_v4().to_string();
    match lock::acquire(&data, &instance) {
        Acquired::Exclusive(held) => {
            if let Some(previous) = &held.displaced {
                log::info!(
                    "took over the instance lock from {} (pid {}), which is no longer running",
                    previous.instance,
                    previous.pid
                );
            }
            app.manage(held);
            Ok(session_core::jobs::Liveness::OnlyInstance)
        }
        Acquired::HeldBy(holder) => {
            let who = holder.map_or_else(
                || "another process".to_string(),
                |h| format!("process {}", h.pid),
            );
            let message = format!("Longhand is already running ({who}); this copy is exiting.");
            log::error!("{message}");
            eprintln!("{message}");
            std::process::exit(0);
        }
        Acquired::Unavailable(e) => {
            log::warn!("the instance lock could not be taken ({e}); judging claims by heartbeat");
            Ok(session_core::jobs::Liveness::Unknown {
                now: chrono::Local::now().fixed_offset(),
            })
        }
    }
}

pub fn run() {
    let specta_builder = specta_builder();

    #[cfg(debug_assertions)]
    export_bindings(&specta_builder);

    let invoke_handler = specta_builder.invoke_handler();

    let builder = tauri::Builder::default()
        // First, so a second launch goes no further than fronting this one
        // (R34, AE21).
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main_window(app);
        }))
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
        // The main window opens at the size and place it had when the app
        // quit. The Settings and licences windows open at their own sizes,
        // centred.
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_denylist(&[window::SETTINGS_WINDOW, window::LICENCES_WINDOW])
                .build(),
        )
        // The About pane opens the acknowledged projects' pages in the
        // default browser (KTD8).
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_macos_permissions::init());

    let app = builder
        .setup(move |app| {
            specta_builder.mount_events(app);

            let handle = app.handle().clone();
            let liveness = take_instance_lock(&handle)?;
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
            // Before processing's launch sweep and any writer (U16), and
            // only when no other instance can be recording.
            if matches!(liveness, session_core::jobs::Liveness::OnlyInstance) {
                capture.recover_at_launch();
            }
            let alerts = std::sync::Arc::new(managers::alerts::AlertsManager::default());
            app.manage(std::sync::Arc::clone(&alerts));
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
            // Managed before anything records: capture starts a live worker
            // for each segment of a note in live mode.
            let live = std::sync::Arc::new(managers::live::LiveManager::new(
                std::sync::Arc::clone(&models),
            ));
            app.manage(std::sync::Arc::clone(&live));
            let processing = std::sync::Arc::new(managers::processing::ProcessingManager::new(
                &handle,
                &capture,
                &live,
                models,
                secret_store,
                liveness,
            ));
            processing.start();
            app.manage(processing);
            // Resolving the microphone queries the audio HAL; keep it off the
            // launch path. A failure here is retried at the first press.
            let preparing = std::sync::Arc::clone(&capture);
            let preparing_app = handle.clone();
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = preparing.prepare(&preparing_app) {
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
            #[cfg(target_os = "macos")]
            menu::set_app_menu(&handle)?;
            shortcut::register(&handle);
            // After the tray exists, so the restored alert can mark it.
            alerts.restore(&handle, &capture);

            window::show_main_window(&handle);
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(invoke_handler)
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app, event| match event {
        // Stop the live workers and free the recognition engine before
        // static destructors run.
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
