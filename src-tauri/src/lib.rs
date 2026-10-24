mod audio;
mod autostart;
mod commands;
mod interface_zoom;
mod managers;
#[cfg(target_os = "macos")]
mod menu;
mod shortcut;
mod startup;
mod tray;
mod widget;
mod window;
#[cfg(target_os = "macos")]
mod zoom;

use tauri::Manager;
use tauri_specta::{Builder, collect_commands, collect_events};

/// Where the generated TypeScript bindings are written, relative to this crate.
pub const BINDINGS_PATH: &str = "../src/bindings.ts";

/// The level of the app's log for a target that [`LOG_TARGET_LEVELS`] does
/// not name.
pub const LOG_LEVEL: log::LevelFilter = log::LevelFilter::Info;

/// The level of the app's log for each named target. An entry also applies
/// to the submodules of its target (`rig` applies to `rig::completions`).
pub const LOG_TARGET_LEVELS: [(&str, log::LevelFilter); 3] = [
    // onnxruntime logs every graph pass at INFO; left in, it rotates the
    // capture warnings out of the default 40 KB file in minutes.
    ("ort", log::LevelFilter::Warn),
    // Rig writes `tracing` events, and `ort` turns on tracing's `log`
    // feature, so the events reach this log. Rig's completion span has
    // fields for the system prompt, the messages and the reply (R3, KTD4).
    // Rig names its targets `rig::...` or by module path, `rig_core::...`.
    ("rig", log::LevelFilter::Off),
    ("rig_core", log::LevelFilter::Off),
];

/// The command and event set exposed to the frontend. Kept in one function so
/// the running app and the `export_bindings` test generate identical bindings.
pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            commands::settings::get_settings,
            commands::settings::set_recognition_defaults,
            commands::models::resolve_recognition,
            commands::models::download_model_bundle,
            commands::settings::login_item_status,
            commands::settings::set_login_item,
            commands::settings::open_login_item_settings,
            commands::settings::change_interface_zoom,
            commands::settings::choose_provider,
            commands::settings::set_provider_address,
            commands::settings::set_provider_model,
            commands::settings::set_provider_context_limit,
            commands::settings::set_default_speaker_count,
            commands::settings::select_asr_model,
            commands::settings::set_live_transcript,
            commands::settings::retry_recognition_downloads,
            commands::settings::list_microphones,
            commands::settings::set_microphone,
            commands::settings::app_audio_available,
            commands::settings::list_running_apps,
            commands::settings::add_app,
            commands::settings::add_call_apps,
            commands::settings::remove_app,
            commands::settings::app_icon,
            commands::settings::set_new_note_records,
            commands::settings::set_alert_sound,
            commands::settings::set_show_recording_widget,
            commands::settings::set_check_updates_automatically,
            commands::settings::set_audio_removal,
            commands::settings::audio_removal_count,
            commands::settings::set_hide_from_dock,
            commands::settings::summary_prompts,
            commands::settings::create_summary_prompt,
            commands::settings::restore_summary_prompt,
            commands::settings::duplicate_summary_prompt,
            commands::settings::rename_summary_prompt,
            commands::settings::set_summary_prompt_text,
            commands::settings::set_summary_prompt_title_text,
            commands::settings::delete_summary_prompt,
            commands::settings::activate_summary_prompt,
            commands::settings::set_background,
            commands::settings::note_types,
            commands::settings::add_note_type,
            commands::settings::set_default_note_type,
            commands::settings::set_note_type_prompt,
            commands::settings::rename_note_type,
            commands::settings::delete_note_type,
            commands::settings::rename_tag,
            commands::settings::delete_tag,
            commands::settings::pick_storage_folder,
            commands::settings::change_storage_folder,
            commands::settings::move_status,
            commands::settings::pick_watched_folder,
            commands::settings::watched_folder_preview,
            commands::settings::set_watched_folder,
            commands::settings::watched_folder_status,
            commands::settings::retry_move,
            commands::settings::show_storage_folder,
            commands::settings::agent_instructions_status,
            commands::settings::add_agent_instructions,
            commands::settings::provider_credential_status,
            commands::settings::set_provider_credential,
            commands::settings::delete_provider_credential,
            commands::settings::repair_keychain_access,
            commands::settings::check_provider,
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
            commands::import::pick_audio_files,
            commands::import::import_files,
            commands::capture::stop_recording,
            commands::capture::pause_recording,
            commands::capture::continue_recording,
            commands::capture::resume_recording,
            commands::capture::record_note,
            commands::capture::recording_status,
            commands::capture::set_note_microphone,
            commands::capture::set_note_apps,
            commands::capture::set_session_speaker_count,
            commands::capture::shortcut_status,
            commands::capture::set_shortcut,
            commands::capture::suspend_shortcuts,
            commands::capture::resume_shortcuts,
            commands::alerts::play_alert_sound,
            commands::processing::regenerate_summary,
            commands::processing::check_transcribe_again,
            commands::processing::transcribe_again,
            commands::processing::retry_session,
            commands::processing::unblock_session,
            commands::session::list_sessions,
            commands::session::library_notices,
            commands::session::dismiss_import_failure,
            commands::session::convert_old_notes,
            commands::session::session_detail,
            commands::session::load_session_notes,
            commands::session::save_session_notes,
            commands::session::rename_session,
            commands::session::set_note_type,
            commands::session::set_note_tags,
            commands::session::list_tags,
            commands::session::session_document,
            commands::session::live_transcript,
            commands::session::delete_session,
            commands::session::delete_session_permanently,
            commands::session::leave_note,
            commands::session::set_shown_note,
            commands::session::remove_audio,
            commands::window::open_settings,
            commands::window::close_settings_window,
            commands::window::open_licences,
            commands::build_info::build_info,
            commands::updates::update_state,
            commands::updates::check_for_updates,
            commands::updates::choose_update,
            commands::updates::show_update_window,
            commands::licences::licence_text,
            commands::clipboard::write_clipboard,
            commands::webview_log::log_from_webview,
            commands::widget::widget_hide,
            commands::widget::widget_open_note,
        ])
        .events(collect_events![
            managers::models::ModelDownloadEvent,
            managers::settings::SettingsChanged,
            managers::capture::CaptureEnded,
            managers::capture::CaptureTick,
            managers::capture::RecordingChanged,
            managers::processing::ProcessingUpdate,
            managers::processing::MoveProgress,
            managers::processing::MovePaused,
            managers::library::LibraryChanged,
            managers::watched_folder::WatchedFolderChanged,
            managers::import::ImportNoticesChanged,
            managers::live::LiveTranscriptEvent,
            managers::live::LiveRecognitionEvent,
            managers::live::LivePartSaved,
            managers::live::LiveTranscriptStatus,
            tray::ShowSession,
            shortcut::ShortcutFailed,
            widget::WidgetPointer,
            widget::WidgetShown,
            window::MainWindowHidden,
            window::SettingsPaneRequested,
            managers::updates::UpdateStateChanged,
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
    // A restart after an update starts this copy before the old copy exits
    // (tauri's `process::restart`). Wait up to 5 s for the old copy to
    // release the lock.
    let mut acquired = lock::acquire(&data, &instance);
    for _ in 0..50 {
        if !matches!(acquired, Acquired::HeldBy(_)) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        acquired = lock::acquire(&data, &instance);
    }
    match acquired {
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

/// The body of `setup`. `run` handles its error (KTD7).
fn setup_app(
    app: &mut tauri::App,
    specta_builder: &Builder<tauri::Wry>,
) -> Result<(), Box<dyn std::error::Error>> {
    let launch_reason = autostart::launch_reason();
    log::info!("startup launch reason: {launch_reason:?}");
    specta_builder.mount_events(app);
    app.manage(commands::session::ShownNote::default());
    app.manage(commands::session::Conversion::default());

    let handle = app.handle().clone();
    let liveness = take_instance_lock(&handle)?;
    // Before any manager starts: a saved update that no quit
    // installed installs now, and the app restarts (KTD10).
    if matches!(liveness, session_core::jobs::Liveness::OnlyInstance) {
        managers::updates::install_at_launch(&handle);
    }
    // After the update check, so a newer build that installs at launch
    // shows no alert, and before any reader opens `longhand.db` (KTD5).
    // Without the instance lock the check is skipped; a database that this
    // build cannot open then stops the start with the R9 alert.
    startup::check_database(&handle, &liveness)?;
    // Read once before any other reader that writes, so a first run writes
    // the defaults and an old file migrates. A file that can't be read
    // shows an alert (Quit or Use Default Settings). The migration puts the old
    // shared values into the cache (R21). The join with the storage
    // folder runs on the launch thread, before the first library
    // scan; until then the shared view comes from the cache.
    let local = managers::settings::load_at_launch(&handle)?;
    log::info!(
        "settings loaded (schema version {})",
        local.settings_schema_version
    );
    // Before the shared settings join and the first library scan: a
    // new install gets the default storage folder (R2, KTD12).
    let local = managers::settings::ensure_storage_folder(&handle, local);
    app.manage(std::sync::Arc::new(
        managers::shared::SharedSettingsManager::new(
            managers::settings::shared_cache_dir(&handle)?,
            local.storage_folder(),
        ),
    ));
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
        // After the recovery, so that the notes it recovers keep
        // their models (R9).
        commands::models::remove_unneeded_supporting_models(&handle);
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
            if let Some(updates) =
                app.try_state::<std::sync::Arc<managers::updates::UpdateManager>>()
            {
                updates.wake();
            }
            std::thread::spawn(move || capture.did_wake(&app));
        }
    });
    app.manage(std::sync::Mutex::new(watch));
    // Managed before anything records: capture starts a live worker
    // for each segment of a note in live mode.
    let live = std::sync::Arc::new(managers::live::LiveManager::new(std::sync::Arc::clone(
        &models,
    )));
    app.manage(std::sync::Arc::clone(&live));
    let processing = std::sync::Arc::new(managers::processing::ProcessingManager::new(
        &handle,
        &capture,
        &live,
        models,
        secret_store,
        liveness,
    ));
    // The settings commands start a removal pass after a change
    // (R17), and the count before a change reads it.
    app.manage(std::sync::Arc::clone(processing.audio_removal()));
    processing.start();
    let working = std::sync::Arc::clone(&processing);
    app.manage(processing);
    // Busy means that a note records or processes (KTD7).
    let recording = std::sync::Arc::clone(&capture);
    let updates = managers::updates::UpdateManager::new(&handle, move || {
        recording.is_capturing() || working.is_working()
    });
    updates.start();
    app.manage(updates);
    // Resolving the microphone queries the audio HAL; keep it off the
    // launch path. A failure here is retried at the first press.
    let preparing = std::sync::Arc::clone(&capture);
    let preparing_app = handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = preparing.prepare(&preparing_app) {
            log::warn!("microphone not prepared at launch: {e}");
        }
    });
    window::create_main_window(&handle)?;
    widget::create(&handle)?;
    tray::create_tray(&handle)?;
    #[cfg(target_os = "macos")]
    menu::set_app_menu(&handle)?;
    shortcut::register(&handle);
    // After the tray exists, so the restored alert can mark it.
    alerts.restore(&handle, &capture);
    // After the tray and the restored alert: a note that a crash interrupted
    // shows the recording widget at launch (R1).
    widget::start(&handle);

    if launch_reason == autostart::LaunchReason::Manual {
        window::show_main_window(&handle);
    }
    Ok(())
}

pub fn run() {
    let specta_builder = specta_builder();

    #[cfg(debug_assertions)]
    export_bindings(&specta_builder);

    let invoke_handler = specta_builder.invoke_handler();

    let builder = tauri::Builder::default()
        // First, so a second launch goes no further than fronting this one
        // (R34, AE21).
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if !autostart::background_request(&args) {
                window::show_main_window(app);
            }
        }))
        .plugin(
            LOG_TARGET_LEVELS
                .iter()
                .fold(
                    tauri_plugin_log::Builder::new().level(LOG_LEVEL),
                    |builder, &(target, level)| builder.level_for(target, level),
                )
                .max_file_size(5_000_000)
                .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(4))
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Restore the main and Settings windows to their saved size and position.
        // The licences window keeps its centred default, and the recording
        // widget its place above the Dock (R3).
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .with_denylist(&[
                    window::LICENCES_WINDOW,
                    window::UPDATE_WINDOW,
                    widget::WIDGET_WINDOW,
                ])
                .build(),
        )
        // The About pane opens the acknowledged projects' pages in the
        // default browser (KTD8).
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_macos_permissions::init());

    // The recording widget is an NSPanel on macOS (KTD3).
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    #[cfg(target_os = "windows")]
    let builder = builder.plugin(
        tauri_plugin_autostart::Builder::new()
            .args([autostart::LOGIN_ARGUMENT])
            .build(),
    );

    // Before `build`, so the alert after a build error can name the log
    // file. No app handle exists then.
    let context = tauri::generate_context!();
    let log_file = startup::log_file_for(
        &context.config().identifier,
        context
            .config()
            .product_name
            .as_deref()
            .unwrap_or(env!("CARGO_PKG_NAME")),
    );

    #[allow(unused_mut)]
    let mut app = builder
        .setup(move |app| {
            // An error never leaves `setup`: the app shows the R9 alert and
            // exits with code 1, and no panic occurs (KTD7).
            if let Err(e) = setup_app(app, &specta_builder) {
                startup::fail(e.as_ref(), startup::log_file(app.handle()).as_deref());
            }
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .invoke_handler(invoke_handler)
        .build(context)
        // The logger can be missing here; the alert shows all the same (KTD7).
        .unwrap_or_else(|e| startup::fail(&e, log_file.as_deref()));

    // Before the event loop, as Handy does (KTD2). With "Hide from Dock" on,
    // the app starts as Accessory: an ordinary show promotes it to Regular,
    // and a login launch never acquires a Dock icon first. With the switch
    // off, it starts as Regular, so a login launch also shows the Dock icon.
    // The store plugin is ready here: `build` initializes the plugins, and
    // `setup` runs later, in `run`.
    #[cfg(target_os = "macos")]
    {
        let hide_from_dock = managers::settings::read_settings(app.handle()).hide_from_dock;
        app.set_activation_policy(window::startup_dock_policy(hide_from_dock).into());
    }

    app.run(|app, event| match event {
        // Stop the live workers and free the recognition engine before
        // static destructors run.
        tauri::RunEvent::Exit => {
            log::info!("app exit");
            if let Some(processing) =
                app.try_state::<std::sync::Arc<managers::processing::ProcessingManager>>()
            {
                processing.shutdown();
            }
            // After processing stops: install the build that the user
            // asked for (KTD10).
            if let Some(updates) =
                app.try_state::<std::sync::Arc<managers::updates::UpdateManager>>()
            {
                updates.install_at_exit();
            }
        }
        #[cfg(target_os = "macos")]
        // Launching the running bundle again (Finder, Spotlight, Dock) is a
        // request to see the window it hid on close.
        tauri::RunEvent::Reopen { .. } if !autostart::background_request(&[]) => {
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
