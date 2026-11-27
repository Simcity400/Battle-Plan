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

use std::sync::{Arc, OnceLock};
use tauri::Manager;

struct Exiting {
    done: app_update::session::Done,
    app: OnceLock<tauri::AppHandle>,
}

fn start_exit_worker() -> std::io::Result<&'static Exiting> {
    static EXITING: std::sync::OnceLock<Exiting> = std::sync::OnceLock::new();
    let done = app_update::session::Done::new()?;
    let exiting: &'static Exiting = EXITING.get_or_init(|| Exiting { done, app: OnceLock::new() });
    std::thread::Builder::new().name("quitting".into()).spawn(move || {
        let exit = &replay_settings::exit::EXIT;
        exit.wait_sealed();
        app_update::saving::wait_until_empty(app_update::recorder_name(), &exit.notices(), &|limit| exit.empty_within(limit), &|| exit.held().map(worker::saving), &|| exiting.done.awaited());
        app_update::control::stop();
        exiting.done.set();
        let Some(app) = exiting.app.get() else { std::process::exit(0) };
        app.exit(0);
        std::thread::sleep(std::time::Duration::from_secs(5));
        std::process::exit(0);
    })?;
    Ok(exiting)
}

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
    let exiting = match start_exit_worker() {
        Ok(exiting) => exiting,
        Err(error) => {
            eprintln!("BattlePlan Recorder couldn’t start, because the thread that quits it in order didn’t start: {error}");
            std::process::exit(2);
        }
    };
    let jobs = Arc::new(compress::Jobs::default());
    let recorder = Arc::new(worker::Recorder::with(Arc::clone(&jobs)));
    let app = tauri::Builder::default()
        .invoke_system(include_str!("../../desktop/src/ipc.js"))
        .manage(Arc::clone(&recorder))
        .manage(Arc::clone(&jobs))
        .manage(panel::Panel::default())
        .manage(recordings::Library::default())
        .setup(move |app| {
            let handle = app.handle().clone();
            let _ = exiting.app.set(handle.clone());
            if let Ok(settings) = replay_settings::recorder_settings::RecorderSettings::load() {
                for root in std::iter::once(worker::save_folder(&settings)).chain(settings.earlier_save_folders.iter().cloned()) {
                    jobs.take_up(&root, &settings);
                }
            }
            let changed = handle.clone();
            jobs.serve(move || worker::recordings_changed(&changed))?;
            tray::create(&handle, Arc::clone(&recorder))?;
            app_update::prepare(
                &handle,
                app_update::Role::Recorder,
                Arc::new(updates::Host(Arc::clone(&recorder))),
            );
            if let Err(error) = app_update::session::watch(Arc::new(worker::SessionEnding(handle.clone(), &exiting.done)), app_update::recorder_name()) {
                eprintln!("BattlePlan Recorder can’t hear Windows end the session, so a sign-out or shutdown may not wait for its recording to be saved: {error}");
            }
            recorder.start(handle)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::recorder_status,
            commands::panel_opened,
            commands::panel_fit,
            commands::close_panel,
            commands::prompt_seen,
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
    app.run(move |app, event| match event {
        tauri::RunEvent::Ready => {
            let app = app.clone();
            tauri::async_runtime::spawn_blocking(move || app_update::start(&app));
        }
        tauri::RunEvent::ExitRequested {
            code: None, api, ..
        } => api.prevent_exit(),
        tauri::RunEvent::ExitRequested { api, .. } if !exiting.done.is_set() => {
            app.state::<Arc<worker::Recorder>>().exit_requested(app);
            api.prevent_exit();
        }
        tauri::RunEvent::Exit => {
            app.state::<Arc<worker::Recorder>>().exit_requested(app);
            exiting.done.wait_serving_sent_messages();
        }
        _ => {}
    });
}
