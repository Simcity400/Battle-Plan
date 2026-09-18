#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analytics_service;
mod native;

use analytics_service::AnalyticsService;
use desktop_protocol::ViewBounds;
use native::{HostStatus, NativeHost};
use std::path::PathBuf;
use tauri::{Manager, State, WebviewWindow};

#[tauri::command]
fn desktop_state(host: State<'_, NativeHost>) -> HostStatus {
    host.status()
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
async fn choose_replay(window: WebviewWindow) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        rfd::FileDialog::new()
            .set_parent(&window)
            .set_title("Open a campaign recording")
            .add_filter("Battle Plan", &["battleplan", "hoi4replay"])
            .pick_file()
            .map(|path| {
                replay_path(&path.to_string_lossy()).map(|path| path.to_string_lossy().into_owned())
            })
            .transpose()
    })
    .await
    .map_err(|e| e.to_string())?
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
) -> Result<(), String> {
    host.seek(
        replay_path(&path)?.to_string_lossy().into_owned(),
        revision,
        branch,
        hour,
    )
}

fn main() {
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "battle-plan\n\nreplay-desktop [recording.battleplan] [viewer options]\n\nMap keeps the existing native viewer. Analytics opens in its own side tab.\nViewer options such as --game-dir, --user-dir, --branch, --hour and --camera\nare forwarded to the native viewer. Run viewer --help for the full list.\n\nDevelopment: npm run dev\nProduction: npm run build:desktop"
        );
        return;
    }
    let app = tauri::Builder::default()
        .manage(NativeHost::default())
        .manage(AnalyticsService::default())
        .invoke_handler(tauri::generate_handler![
            desktop_state,
            set_map_view,
            choose_replay,
            open_replay,
            seek_map,
            analytics_service::analytics_metadata,
            analytics_service::analytics_series,
            analytics_service::cancel_analytics,
        ])
        .build(tauri::generate_context!())
        .expect("Could not initialize the replay desktop");
    app.run(|app, event| {
        if matches!(
            event,
            tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
        ) {
            app.state::<AnalyticsService>().shutdown();
            app.state::<NativeHost>().shutdown();
        }
    });
}
