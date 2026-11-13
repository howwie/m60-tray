//! M60 Tray: a system-tray controller for the Edifier M60 speakers.
//!
//! `m60-core` owns the protocol and the connection state machine, `ble` plugs btleplug into it,
//! and this crate adds the Tauri tray, the volume popup, start-at-login and housekeeping.

// No console window in release builds on Windows (needed for a startup app).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod ble;
mod busy;
mod commands;
mod config;
mod editor;
mod icon;
mod instance;
mod logging;
mod menu;
mod popup;
mod presets;
mod settings_window;
mod startup;
mod tray;
mod version;

use std::sync::Arc;

use m60_core::{Controller, Settings};

fn main() {
    let dir = config::app_dir();
    logging::init(&dir);
    log::info!("{} starting", version::VERSION_LINE);

    // Keep the lock file open for the whole run.
    let _instance_lock = match instance::acquire(&dir) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => {
            log::info!("already running; exiting");
            return;
        }
        Err(e) => {
            log::warn!("could not take the instance lock ({e}); continuing anyway");
            None
        }
    };

    let store = Arc::new(presets::Store::open(&dir));
    let remembered_by = store.clone();
    let levels_kept_by = store.clone();
    let resume_kept_by = store.clone();
    let (controller, handle) = Controller::new(
        ble::BleRadio::new(),
        Settings {
            remembered: store.address(),
            on_connected: Box::new(move |id| remembered_by.set_address(id)),
            share: store.sharing(),
            remember_volume: store.remember_volume(),
            input_volumes: store.input_volumes(),
            on_input_volumes: Box::new(move |levels| levels_kept_by.set_input_volumes(levels)),
            bluetooth_on_power_off: store.bluetooth_on_power_off(),
            resume: store.resume(),
            on_resume: Box::new(move |r| resume_kept_by.set_resume(r)),
        },
    );

    let ui_handle = handle.clone();
    let app = tauri::Builder::default()
        .manage(handle)
        .manage(store)
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::set_volume,
            commands::media,
            commands::hide_popup,
            commands::get_presets,
            commands::save_preset,
            commands::delete_preset,
            commands::apply_preset,
            commands::get_settings,
            commands::fit_settings,
            commands::set_app_setting,
            commands::set_backlight,
            commands::set_bt_prompt,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // menu bar only, no Dock icon
            tauri::async_runtime::spawn(controller.run());
            tray::install(app.handle(), ui_handle)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("could not start the app");

    app.run(|_app, event| {
        // Closing or hiding the popup must not end the app; only "Quit" does (it passes an exit code).
        if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}
