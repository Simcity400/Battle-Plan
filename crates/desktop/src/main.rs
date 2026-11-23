#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dashboard_service;
mod follow_push;
mod pull;
mod native;
mod perf;
mod popout;
mod recorder;
mod replay_library;
#[cfg(feature = "served-ui")]
mod served_ui;
mod updates;
mod video;

use dashboard_core::protocol::{DashboardMapView, ViewBounds};
use dashboard_service::DashboardService;
use native::{HostStatus, NativeHost};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::http::HeaderValue;
use tauri::{Emitter, Manager, State, WebviewWindow};

#[global_allocator]
static ALLOCATOR: engine_heap::Heap = engine_heap::Heap;

fn serve_artwork(request: tauri::http::Request<Vec<u8>>, responder: tauri::UriSchemeResponder, page_origin: HeaderValue) {
    use tauri::http::{Response, StatusCode, header};
    let name = request.uri().path().strip_prefix('/').unwrap_or_default().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let mut response = Response::new(Vec::new());
        response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, page_origin);
        match dashboard_text::artwork::bytes(&name) {
            Some(png) => {
                *response.body_mut() = png.to_vec();
                let headers = response.headers_mut();
                headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
                headers.insert(
                    header::CACHE_CONTROL,
                    HeaderValue::from_static("public, max-age=31536000, immutable"),
                );
            }
            None => *response.status_mut() = StatusCode::NOT_FOUND,
        }
        responder.respond(response);
    });
}

#[tauri::command]
fn desktop_state(window: WebviewWindow, host: State<'_, NativeHost>) -> Result<HostStatus, String> {
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
    action: dashboard_core::protocol::MapAction,
) -> Result<(), String> {
    if matches!(action, dashboard_core::protocol::MapAction::Folders { .. }) {
        return Err("Set the content folders in Settings".into());
    }
    host.map_action(action.clone())?;
    popouts.mirror(&action);
    Ok(())
}

#[tauri::command]
async fn export_map(
    host: State<'_, NativeHost>,
    popouts: State<'_, popout::PopOuts>,
    view_id: Option<String>,
) -> Result<Option<String>, String> {
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
    let Some(path) = path else { return Ok(None) };
    let path = path.to_string_lossy().into_owned();
    let answered = if view_id.as_deref() == Some(popout::VIEW_ID) {
        popout::export(&popouts, popout, path)?
    } else {
        host.export(path, view_id)?
    };
    answered
        .await
        .map_err(|_| "The map stopped before the export finished".to_owned())?
        .map(Some)
        .map_err(|error| format!("Couldn’t save the map image: {error}"))
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
    clock_id: String,
    views: Vec<DashboardMapView>,
) -> Result<(), String> {
    host.set_dashboard_views(&window, &clock_id, &views)
}

pub(crate) fn replay_path(path: &str) -> Result<String, String> {
    if path.is_empty() {
        return Err("The recording path is empty".into());
    }
    if path.len() > 32_000 {
        return Err("The recording path is too long".into());
    }
    if path.contains('\0') {
        return Err("The recording path holds a NUL character".into());
    }
    if !replay_files::recording_files::is_recording(std::path::Path::new(path)) {
        return Err("Choose a BattlePlan recording.".into());
    }
    Ok(path.to_owned())
}

#[tauri::command]
fn open_replay(host: State<'_, NativeHost>, path: String) -> Result<(), String> {
    host.open(replay_path(&path)?)
}

#[tauri::command]
fn seek_map(
    host: State<'_, NativeHost>,
    path: String,
    revision: String,
    branch: String,
    hour: i32,
    playback: Option<dashboard_core::protocol::MapPlayback>,
) -> Result<(), String> {
    host.seek(replay_path(&path)?, revision, branch, hour, playback)
}

#[cfg(not(feature = "served-ui"))]
fn context() -> tauri::Context {
    tauri::generate_context!()
}

#[cfg(feature = "served-ui")]
fn context() -> tauri::Context {
    let mut context = tauri::generate_context!();
    match served_ui::serve(&mut context) {
        Ok(folder) => eprintln!("Serving the UI from {}", folder.display()),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
    context
}

#[derive(Default)]
struct UnsavedWork {
    unsaved: AtomicBool,
    discarded: AtomicBool,
}

impl UnsavedWork {
    const UPDATE_REFUSED: &'static str =
        "BattlePlan has unsaved changes. Save or discard them in BattlePlan, then restart to update.";

    fn kept(&self) -> bool {
        self.unsaved.load(Ordering::Acquire) && !self.discarded.load(Ordering::Acquire)
    }

    fn update_refused(&self) -> Option<String> {
        self.kept().then(|| Self::UPDATE_REFUSED.to_owned())
    }

    fn holds_close<E: std::fmt::Display>(&self, ask: impl FnOnce() -> Result<(), E>) -> bool {
        if !self.kept() {
            return false;
        }
        if let Err(error) = ask() {
            eprintln!("Couldn’t ask about unsaved changes before closing, so BattlePlan stays open: {error}");
        }
        true
    }

    fn close_discarding<E>(&self, close: impl FnOnce() -> Result<(), E>) -> Result<(), E> {
        self.discarded.store(true, Ordering::Release);
        let closed = close();
        if closed.is_err() {
            self.discarded.store(false, Ordering::Release);
        }
        closed
    }
}

#[tauri::command]
fn set_unsaved_work(work: State<'_, UnsavedWork>, unsaved: bool) {
    work.unsaved.store(unsaved, Ordering::Release);
}

#[tauri::command]
fn close_discarding_unsaved_work(window: WebviewWindow, work: State<'_, UnsavedWork>) -> Result<(), String> {
    work.close_discarding(|| window.close()).map_err(|error| format!("Couldn’t close BattlePlan: {error}"))
}

fn ask_before_close(app: &tauri::AppHandle) -> bool {
    app.state::<UnsavedWork>().holds_close(|| app.emit_to("main", "close-requested", ()))
}

#[tauri::command]
fn playback_pressure_merges() -> u64 { dashboard_follow::lead::pressure_merges() }


fn main() {
    engine_heap::return_freed_memory_promptly();
    viewer_runtime::parallel::name_shared_pool();
    if std::env::args().any(|arg| arg == "--help" || arg == "-h") {
        println!(
            "BattlePlan\n\nreplay-desktop [recording.battleplan] [map options]\n\nThe map runs inside this app. Analytics opens in its own side tab.\n--game-dir and --user-dir override the saved content folders for this run.\nMap options such as --branch, --hour and --camera open the campaign map\nthere. --diagnostics shows the maps' technical stats and data limitations\nfor this run; F3 or Settings > Diagnostics toggles and saves them.\n--window-dump <recording> --out <frame.png> draws one frame of the map to a\nfile and exits (the release smoke).\n\nDevelopment: npm run dev\nProduction: npm run build:desktop"
        );
        return;
    }
    if std::env::args().any(|arg| arg == "--window-dump") {
        let code = match viewer::host::window_dump(std::env::args_os().skip(1)) {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("{error}");
                1
            }
        };
        std::process::exit(code);
    }
    if app_update::refused_during_setup(app_update::app_name()) {
        return;
    }
    let mut context = context();
    if let Some(window) = context.config_mut().app.windows.iter_mut().find(|window| window.label == "main") {
        window.title = app_update::app_name().to_owned();
    }
    perf::init();
    if let Some(prefix) = perf::map_prefix() {
        viewer::host::log_map_frames(prefix);
    }
    shader_pipeline::artwork::warm();
    if let Err(error) = dashboard_text::artwork::configure_desktop() {
        eprintln!("{error}");
    }
    let page_origin = context.config().build.dev_url.as_ref().filter(|_| tauri::is_dev())
        .map_or_else(|| "http://tauri.localhost".to_owned(), |url| url.origin().ascii_serialization());
    let artwork_origin = match HeaderValue::from_str(&page_origin) {
        Ok(origin) => origin,
        Err(error) => {
            eprintln!("The dashboard origin {page_origin} isn’t a header value: {error}");
            std::process::exit(2);
        }
    };
    let app = tauri::Builder::default()
        .register_asynchronous_uri_scheme_protocol("push", move |context, request, responder| {
            use tauri::http::{StatusCode, header};
            if request.headers().get(header::ORIGIN).and_then(|origin| origin.to_str().ok()) != Some(&page_origin) {
                return pull::refuse(responder, StatusCode::FORBIDDEN, "Only the dashboard may pull followed answers");
            }
            match request.uri().path() {
                "/follow" => context.app_handle().state::<follow_push::FollowPush>().ask(&request, responder),
                "/credit" => match (pull::param(&request, "session"), pull::param(&request, "released").and_then(|released| released.parse::<u64>().ok())) {
                    (Some(session), Some(released)) => {
                        if let Some(epoch) = context.app_handle().state::<follow_push::FollowPush>().outbox(|relay| relay.epoch_of(session)) {
                            dashboard_follow::registry::registry().credit(epoch, released);
                        }
                        pull::refuse(responder, StatusCode::NO_CONTENT, "");
                    }
                    _ => pull::refuse(responder, StatusCode::BAD_REQUEST, "Credit names its session and the bytes it has released"),
                },
                _ => pull::refuse(responder, StatusCode::NOT_FOUND, "No such stream"),
            }
        })
        .register_asynchronous_uri_scheme_protocol("artwork", move |_, request, responder| {
            serve_artwork(request, responder, artwork_origin.clone())
        })
        .manage(video::VideoState::default())
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main")
                && let Err(error) = video::restore(&window)
            {
                *app.state::<video::VideoState>()
                    .0
                    .lock()
                    .map_err(|e| std::io::Error::other(e.to_string()))? = Err(error);
            }
            app_update::init(
                app.handle(),
                app_update::Role::Dashboard,
                Arc::new(updates::Host(app.handle().clone())),
            )?;
            recorder::start_recorder();
            native::start_gamepad(app.handle().clone());
            let push = app.handle().clone();
            viewer::host::install_follow(move |kind, payload, session| {
                if kind == dashboard_follow::push::FOLLOW || kind == dashboard_follow::push::PANEL {
                    push.state::<follow_push::FollowPush>().offer(|queue| queue.offer(payload, session));
                }
            })?;
            if let Some(window) = app.get_webview_window("main") {
                app.state::<NativeHost>().ensure_started(&window)?;
            }
            Ok(())
        })
        .manage(NativeHost::default())
        .manage(popout::PopOuts::default())
        .manage(DashboardService::default())
        .manage(follow_push::FollowPush::default())
        .manage(UnsavedWork::default())
        .invoke_handler(tauri::generate_handler![
            video::video_settings,
            video::set_video_settings,
            video::map_diagnostics,
            video::set_map_diagnostics,
            video::map_muted,
            video::set_map_muted,
            desktop_state,
            playback_pressure_merges,
            set_unsaved_work,
            close_discarding_unsaved_work,
            set_map_view,
            select_native_view,
            map_action,
            export_map,
            choose_content_folder,
            dashboard_service::content_folders,
            dashboard_service::set_content_folders,
            set_dashboard_maps,
            replay_library::replay_library,
            replay_library::replay_library_stamp,
            replay_library::set_replay_folder,
            replay_library::choose_replay_folder,
            replay_library::choose_recording,
            recorder::recorder_settings,
            recorder::set_recording_compression,
            recorder::set_auto_compress,
            recorder::recording_preferences,
            recorder::rename_recording,
            recorder::locate_recordings,
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
            dashboard_service::dashboard_history_choice,
            dashboard_service::dashboard_map_settings,
            dashboard_service::dashboard_cancel,
            dashboard_service::dashboard_release_source,
            perf::perf_enabled,
            perf::perf_write,
            perf::perf_finished,
            perf::perf_stop,
            app_update::commands::app_update_state,
            app_update::commands::app_update_check,
            app_update::commands::app_update_download,
            app_update::commands::app_update_install,
            app_update::commands::app_update_set_channel,
            app_update::commands::app_update_open_release,
        ])
        .build(context)
        .expect("Couldn’t start BattlePlan");
    app.run(|app, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" && ask_before_close(app) => api.prevent_close(),
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" && exit_after_maps(app) => api.prevent_close(),
        tauri::RunEvent::ExitRequested { api, .. } if exit_after_maps(app) => api.prevent_exit(),
        tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit => {
            shutdown(app);
            perf::close();
        }
        _ => {}
    });
}

fn exit_after_maps(app: &tauri::AppHandle) -> bool {
    static EXIT: ExitOrder = ExitOrder::new();
    EXIT.begin(
        || {
            let handle = app.clone();
            std::thread::Builder::new()
                .name("ordered exit".into())
                .spawn(move || {
                    let ended = shutdown(&handle);
                    perf::finish(&handle);
                    EXIT.done();
                    if ended {
                        handle.exit(0);
                    } else {
                        std::process::exit(0);
                    }
                })
                .map(drop)
        },
        || {
            perf::close();
            std::process::exit(0);
        },
    )
}

struct ExitOrder(std::sync::atomic::AtomicU8);

impl ExitOrder {
    const RUNNING: u8 = 1;
    const DONE: u8 = 2;

    const fn new() -> Self {
        Self(std::sync::atomic::AtomicU8::new(0))
    }

    fn begin(&self, start: impl FnOnce() -> std::io::Result<()>, fallback: impl FnOnce()) -> bool {
        use std::sync::atomic::Ordering;
        match self.0.compare_exchange(0, Self::RUNNING, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => {}
            Err(Self::RUNNING) => return true,
            Err(_) => return false,
        }
        if start().is_err() {
            fallback();
        }
        true
    }

    fn done(&self) {
        self.0.store(Self::DONE, std::sync::atomic::Ordering::Release);
    }
}

fn shutdown(app: &tauri::AppHandle) -> bool {
    app.state::<popout::PopOuts>().close(app);
    let (ended, waited) = std::sync::mpsc::channel();
    app.state::<NativeHost>().shutdown(move || {
        let _ = ended.send(());
    });
    let stopped = waited.recv_timeout(std::time::Duration::from_secs(2)).is_ok();
    app.state::<DashboardService>().shutdown();
    stopped
}

#[cfg(test)]
mod tests {
    use super::{ExitOrder, UnsavedWork};
    use std::sync::atomic::Ordering;

    #[test]
    fn unsaved_work_refuses_an_update_until_it_is_saved_or_discarded() {
        let work = UnsavedWork::default();
        assert_eq!(work.update_refused(), None, "nothing unsaved lets the update close the app");
        work.unsaved.store(true, Ordering::Release);
        assert_eq!(work.update_refused().as_deref(), Some(UnsavedWork::UPDATE_REFUSED));
        work.discarded.store(true, Ordering::Release);
        assert_eq!(work.update_refused(), None, "work the user chose to discard no longer holds the update");
        work.discarded.store(false, Ordering::Release);
        work.unsaved.store(false, Ordering::Release);
        assert_eq!(work.update_refused(), None, "saved work no longer holds the update");
    }

    #[test]
    fn unsaved_work_holds_a_close_even_when_the_question_cannot_be_asked() {
        let work = UnsavedWork::default();
        assert!(!work.holds_close(|| -> Result<(), &'static str> { unreachable!() }), "nothing unsaved closes at once");
        work.unsaved.store(true, Ordering::Release);
        let mut asked = 0;
        assert!(work.holds_close(|| -> Result<(), &'static str> {
            asked += 1;
            Ok(())
        }));
        assert_eq!(asked, 1);
        assert!(work.holds_close(|| Err("no window")), "unsaved work is never closed silently");
    }

    #[test]
    fn a_failed_discarding_close_leaves_the_work_guarded() {
        let work = UnsavedWork::default();
        work.unsaved.store(true, Ordering::Release);
        assert_eq!(work.close_discarding(|| Err("no window")), Err("no window"));
        assert!(work.kept(), "a close that failed doesn’t authorise a later close or update");
        assert_eq!(work.update_refused().as_deref(), Some(UnsavedWork::UPDATE_REFUSED));
        assert_eq!(work.close_discarding(|| Ok::<(), &str>(())), Ok(()));
        assert!(!work.kept(), "the closing window isn’t asked again");
    }

    #[test]
    fn a_close_stays_held_when_the_exit_cannot_start_and_the_fallback_ends_the_process() {
        let exit = ExitOrder::new();
        let mut fell_back = 0;
        let held = exit.begin(|| Err(std::io::Error::other("no thread")), || fell_back += 1);
        assert!(held, "the window must outlive the map's surface");
        assert_eq!(fell_back, 1);
        assert!(exit.begin(|| unreachable!(), || unreachable!()), "a later close is held too");

        let exit = ExitOrder::new();
        assert!(exit.begin(|| Ok(()), || unreachable!()));
        assert!(exit.begin(|| unreachable!(), || unreachable!()));
        exit.done();
        assert!(!exit.begin(|| unreachable!(), || unreachable!()), "once the maps end, closes go through");
    }
}
