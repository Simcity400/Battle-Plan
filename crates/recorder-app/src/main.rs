#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod compress;
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
    let mut args = std::env::args_os().skip(1);
    let first = args.next();
    if first.as_ref().is_some_and(|arg| arg == compress::ARGUMENT) {
        std::process::exit(compress::child(args.next()));
    }
    if first.is_some_and(|arg| arg == replay_settings::recorder_settings::STARTED_WITH_REPLAY) && startup::held_by_a_job() {
        return;
    }
    if app_update::refused_during_setup(app_update::recorder_name()) {
        return;
    }
    let Some(_instance) = startup::single_instance() else {
        return;
    };
    let recorder = Arc::new(worker::Recorder::default());
    let jobs = Arc::new(compress::Jobs::default());
    let app = tauri::Builder::default()
        .manage(Arc::clone(&recorder))
        .manage(Arc::clone(&jobs))
        .manage(panel::Panel::default())
        .manage(recordings::Library::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            if let Ok(settings) = replay_settings::recorder_settings::RecorderSettings::load() {
                for root in std::iter::once(worker::save_folder(&settings)).chain(settings.earlier_save_folders.iter().cloned()) {
                    jobs.take_up(&root, &settings);
                }
            }
            let changed = handle.clone();
            jobs.serve(move || worker::recordings_changed(&changed))?;
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
            commands::show_session_files,
            commands::open_replay,
            commands::recordings,
            commands::act_on_waiting,
            commands::reveal_recording,
            commands::compress_recording,
            app_update::commands::app_update_state,
            app_update::commands::app_update_check,
            app_update::commands::app_update_download,
            app_update::commands::app_update_install,
            app_update::commands::app_update_set_channel,
            app_update::commands::app_update_open_release,
        ])
        .build(tauri::generate_context!())
        .expect("Could not start BattlePlan Recorder");
    app.run(|_, event| {
        if let tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}
