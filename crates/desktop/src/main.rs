#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dashboard_service;
mod game_process;
mod native;
mod popout;
mod preparation;
mod recorder;
mod replay_library;
mod video;

use dashboard::protocol::{DashboardMapView, ViewBounds};
use dashboard_service::DashboardService;
use native::{HostStatus, NativeHost};
use std::path::PathBuf;
use tauri::{Manager, State, WebviewWindow};

#[tauri::command]
fn desktop_state(window: WebviewWindow, host: State<'_, NativeHost>) -> Result<HostStatus, String> {
    // Establish configuration/onboarding even before a recording makes the map visible.
    host.ensure_started(&window)?;
    Ok(host.status())
}

#[tauri::command]
fn set_map_view(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    visible: bool,
    bounds: ViewBounds,
) -> Result<HostStatus, String> {
    host.set_view(&window, visible, bounds)
}

#[tauri::command]
fn select_native_view(host: State<'_, NativeHost>, dashboard: bool) -> Result<(), String> {
    host.select_view(dashboard)
}

#[tauri::command]
fn map_action(
    host: State<'_, NativeHost>,
    popouts: State<'_, popout::PopOuts>,
    action: dashboard::protocol::MapAction,
) -> Result<(), String> {
    // The host saves folders first and sends the viewer what it saved.
    if matches!(action, dashboard::protocol::MapAction::Folders { .. }) {
        return Err("Set the content folders in Settings".into());
    }
    host.map_action(action.clone())?;
    // A user unload closes the pop-out through `popout_unloaded`; reconnecting a recording
    // unloads it too, and the pop-out follows the reopened one instead.
    popouts.mirror(&action);
    Ok(())
}

#[tauri::command]
async fn export_map(
    host: State<'_, NativeHost>,
    popouts: State<'_, popout::PopOuts>,
    view_id: Option<String>,
) -> Result<(), String> {
    // The pop-out window to export is the one open when the request starts, not after the dialog.
    let popout = (view_id.as_deref() == Some(popout::VIEW_ID))
        .then(|| popouts.current_generation())
        .flatten();
    let path = tauri::async_runtime::spawn_blocking(|| {
        rfd::FileDialog::new()
            .set_title("Export campaign map")
            .add_filter("PNG image", &["png"])
            .set_file_name("campaign-map.png")
            .save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Some(path) = path {
        let path = path.to_string_lossy().into_owned();
        if view_id.as_deref() == Some(popout::VIEW_ID) {
            return popout::export(&popouts, popout, path);
        }
        host.map_action(dashboard::protocol::MapAction::Export { path, view_id })?;
    }
    Ok(())
}

#[tauri::command]
async fn choose_content_folder(
    window: WebviewWindow,
    kind: String,
    initial: String,
) -> Result<Option<String>, String> {
    let title = match kind.as_str() {
        "game" => "Choose your game installation",
        "user" => "Choose your Paradox user folder",
        _ => return Err("Unknown content folder type".into()),
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut dialog = rfd::FileDialog::new().set_parent(&window).set_title(title);
        if let Ok(directory) = std::fs::canonicalize(&initial)
            && directory.is_dir()
        {
            dialog = dialog.set_directory(directory);
        }
        dialog
            .pick_folder()
            .map(|path| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn set_dashboard_maps(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    views: Vec<DashboardMapView>,
) -> Result<(), String> {
    host.set_dashboard_views(&window, &views)
}

fn replay_path(path: &str) -> Result<PathBuf, String> {
    if path.len() > 32_000 {
        return Err("Replay path is too long".into());
    }
    let path = std::fs::canonicalize(path).map_err(|e| format!("Cannot open recording: {e}"))?;
    if !path.is_file() {
        return Err("Choose a replay file".into());
    }
    if !path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("battleplan") || extension.eq_ignore_ascii_case("hoi4replay"))
    {
        return Err("Choose a .battleplan recording".into());
    }
    Ok(path)
}

#[tauri::command]
fn open_replay(host: State<'_, NativeHost>, path: String) -> Result<(), String> {
    host.open(replay_path(&path)?.to_string_lossy().into_owned())
}

#[tauri::command]
fn seek_map(
    host: State<'_, NativeHost>,
    path: String,
    revision: String,
    branch: String,
    hour: i32,
    playback: Option<dashboard::protocol::MapPlayback>,
) -> Result<(), String> {
    host.seek(
        replay_path(&path)?.to_string_lossy().into_owned(),
        revision,
        branch,
        hour,
        playback,
    )
}

fn main() {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "battle-plan\n\nreplay-desktop [recording.battleplan] [viewer options]\n\nMap keeps the existing native viewer. Analytics opens in its own side tab.\n--game-dir and --user-dir override the saved content folders for this run.\nViewer options such as --branch, --hour and --camera are forwarded to the\nnative viewer. --diagnostics shows the maps' technical\nstats and data limitations for this run; F3 or Settings > Diagnostics toggles\nand saves them. Run viewer --help for the full list.\n\nDevelopment: npm run dev\nProduction: npm run build:desktop"
        );
        return;
    }
    let app = tauri::Builder::default()
        .manage(video::VideoState::default())
        .manage(preparation::PreparationMonitor::default())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main")
                && let Err(error) = video::restore(&window)
            {
                *app.state::<video::VideoState>()
                    .0
                    .lock()
                    .map_err(|e| std::io::Error::other(e.to_string()))? = Err(error);
            }
            recorder::start_recorder();
            app.state::<preparation::PreparationMonitor>()
                .start(app.handle())?;
            native::start_gamepad(app.handle().clone());
            Ok(())
        })
        .manage(NativeHost::default())
        .manage(popout::PopOuts::default())
        .manage(DashboardService::default())
        .invoke_handler(tauri::generate_handler![
            video::video_settings,
            video::set_video_settings,
            video::map_diagnostics,
            video::set_map_diagnostics,
            desktop_state,
            set_map_view,
            select_native_view,
            map_action,
            export_map,
            choose_content_folder,
            dashboard_service::content_folders,
            dashboard_service::set_content_folders,
            set_dashboard_maps,
            replay_library::replay_library,
            replay_library::set_replay_folder,
            replay_library::choose_replay_folder,
            replay_library::choose_recording,
            recorder::recorder_settings,
            recorder::recording_preferences,
            recorder::set_recording_names,
            recorder::set_campaign_flag,
            open_replay,
            seek_map,
            popout::popout_open,
            popout::popout_close,
            popout::popout_restart,
            popout::popout_unloaded,
            popout::popout_next_monitor,
            popout::popout_publish,
            popout::popout_seek,
            popout::popout_configure,
            popout::popout_camera_mode,
            popout::popout_fly_aircraft,
            dashboard_service::dashboard_open_source,
            dashboard_service::dashboard_product,
            dashboard_service::dashboard_prepared_query,
            dashboard_service::dashboard_preparation_preferences,
            dashboard_service::dashboard_preparation_status,
            dashboard_service::dashboard_prepare_source,
            dashboard_service::dashboard_cancel_preparation,
            dashboard_service::dashboard_clear_preparation,
            dashboard_service::dashboard_map_settings,
            dashboard_service::dashboard_history_cache,
            dashboard_service::dashboard_cancel,
            dashboard_service::dashboard_release_source,
        ])
        .build(tauri::generate_context!())
        .expect("Could not initialize the replay desktop");
    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            app.state::<preparation::PreparationMonitor>().shutdown(app);
            app.state::<DashboardService>().shutdown();
            app.state::<popout::PopOuts>().close(app);
            app.state::<NativeHost>().shutdown();
        }
    });
}
