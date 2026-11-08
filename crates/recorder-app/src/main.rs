#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Battle Plan Recorder: a tray program that records campaigns while the game runs.

mod commands;
mod flags;
mod panel;
mod recordings;
mod snapshot;
mod startup;
mod targets;
mod tray;
mod updates;
mod worker;

use std::sync::Arc;

fn main() {
    if app_update::refused_during_setup("Battle Plan Recorder") {
        return;
    }
    let Some(_instance) = startup::single_instance() else {
        // Another recorder already runs for this user.
        return;
    };
    let recorder = Arc::new(worker::Recorder::default());
    let app = tauri::Builder::default()
        .manage(Arc::clone(&recorder))
        .manage(panel::Panel::default())
        .manage(recordings::Library::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            tray::create(&handle, Arc::clone(&recorder))?;
            app_update::init(
                &handle,
                app_update::Role::Recorder,
                Arc::new(updates::Host(Arc::clone(&recorder))),
            )?;
            recorder.start(handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::recorder_status,
            commands::panel_opened,
            commands::panel_fit,
            commands::close_panel,
            commands::start_recording,
            commands::stop_recording,
            commands::cancel_stop_prompt,
            commands::recorder_settings,
            commands::update_recorder_settings,
            commands::set_campaign_choice,
            commands::choose_save_folder,
            commands::set_user_folder,
            commands::open_recordings_folder,
            commands::open_details,
            commands::open_replay,
            commands::recordings,
            commands::act_on_waiting,
            commands::reveal_recording,
            app_update::commands::app_update_state,
            app_update::commands::app_update_check,
            app_update::commands::app_update_download,
            app_update::commands::app_update_install,
            app_update::commands::app_update_set_channel,
            app_update::commands::app_update_open_release,
        ])
        .build(tauri::generate_context!())
        .expect("Could not start Battle Plan Recorder");
    app.run(|_, event| {
        // Closing the panel must not end the recorder; only Quit does.
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}
