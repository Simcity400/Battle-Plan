#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dashboard_service;
mod follow_push;
mod leaving;
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

use app_update::saving::{Saving, Wait, drained};
use dashboard_core::protocol::{DashboardMapView, ViewBounds};
use dashboard_service::DashboardService;
use leaving::{Answered, Batch, Closing, Leaving, Reach, Stop, Unanswered};
use native::{HostStatus, NativeHost};
use std::sync::Arc;
use tauri::http::HeaderValue;
use tauri::{Manager, State, WebviewWindow};

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
    if !replay::recording_files::is_recording(std::path::Path::new(path)) {
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

fn main_page(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() == "main" {
        Ok(())
    } else {
        Err("Only BattlePlan’s main window decides whether it may close.".into())
    }
}

struct Receipt {
    app: tauri::AppHandle,
    seq: u64,
    reported: bool,
}

impl Receipt {
    fn report(&mut self, reached: bool) {
        if std::mem::replace(&mut self.reported, true) {
            return;
        }
        if !reached {
            eprintln!("BattlePlan’s page didn’t run its close questions.");
        }
        if let Some(why) = self.app.state::<Leaving>().delivered(self.seq, reached) {
            offer_to_close(&self.app, why);
        }
    }
}

impl Drop for Receipt {
    fn drop(&mut self) {
        self.report(false);
    }
}

fn reach(webview: tauri::Webview, callback: u32) -> Reach {
    Arc::new(move |batch: Batch| {
        let receipt = std::sync::Mutex::new(Some(Receipt { app: webview.app_handle().clone(), seq: batch.seq, reported: false }));
        let questions = match serde_json::to_string(&batch) {
            Ok(questions) => questions,
            Err(error) => {
                eprintln!("BattlePlan couldn’t write its close questions for its page: {error}");
                return;
            }
        };
        let script = format!("window.__TAURI_INTERNALS__.callbacks.get({callback})?.({questions}) === true");
        let ran = move |result: String| {
            let taken = receipt.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
            if let Some(mut receipt) = taken {
                receipt.report(result == "true");
            }
        };
        if let Err(error) = webview.eval_with_callback(script, ran) {
            eprintln!("BattlePlan couldn’t pass its close questions to its page: {error}");
        }
    })
}

#[tauri::command]
fn close_watch_document(webview: tauri::Webview, leaving: State<'_, Leaving>) -> Result<u64, String> {
    main_page(&webview)?;
    Ok(leaving.document())
}

#[tauri::command]
fn watch_close_requests(
    webview: tauri::Webview,
    leaving: State<'_, Leaving>,
    document: u64,
    seq: u64,
    callback: u32,
) -> Result<(), String> {
    main_page(&webview)?;
    leaving.watch(document, seq, move || reach(webview, callback)).map_err(str::to_owned)
}

#[tauri::command]
fn refresh_close_requests(webview: tauri::Webview, leaving: State<'_, Leaving>, document: u64, seq: u64) -> Result<u64, String> {
    main_page(&webview)?;
    leaving.refresh(document, seq).map_err(str::to_owned)
}

#[tauri::command]
fn answer_close_question(
    webview: tauri::Webview,
    leaving: State<'_, Leaving>,
    id: u64,
    stays: bool,
    basis: Vec<u64>,
) -> Result<bool, String> {
    main_page(&webview)?;
    Ok(match leaving.answer(id, stays, &basis) {
        Answered::Leave => {
            webview.app_handle().exit(0);
            true
        }
        Answered::Heard => true,
        Answered::Unknown => false,
    })
}

#[tauri::command]
fn leave_app(webview: tauri::Webview, leaving: State<'_, Leaving>) -> Result<(), String> {
    main_page(&webview)?;
    leaving.exit();
    webview.app_handle().exit(0);
    Ok(())
}

fn close_without_answer(owner: Option<isize>, why: Unanswered) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_DEFBUTTON2, MB_ICONWARNING, MB_YESNO, MessageBoxW};
    let reason = match why {
        Unanswered::Close => "BattlePlan’s page hasn’t answered, so it can’t say whether it has unsaved changes.",
        Unanswered::Unresponsive => "BattlePlan’s page isn’t responding, so it can’t say whether it has unsaved changes.",
        Unanswered::Unreached => "BattlePlan couldn’t ask its page whether it has unsaved changes.",
        Unanswered::Blind => "BattlePlan can’t follow its page’s reloads and crashes, so it can’t ask whether it has unsaved changes.",
    };
    let wide = |text: &str| text.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let text = wide(&format!("{reason}\n\nClose BattlePlan now? Anything unsaved is lost. Choose No to keep waiting."));
    let caption = wide("BattlePlan");
    let owner = owner.map_or(std::ptr::null_mut(), |hwnd| hwnd as *mut std::ffi::c_void);
    unsafe { MessageBoxW(owner, text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) == IDYES }
}

fn offer_to_close(app: &tauri::AppHandle, why: Unanswered) {
    let owner = app.get_webview_window("main").and_then(|window| window.hwnd().ok()).map(|hwnd| hwnd.0 as isize);
    let handle = app.clone();
    let shown = std::thread::Builder::new().name("close choice".into()).spawn(move || {
        let close = close_without_answer(owner, why);
        let leaving = handle.state::<Leaving>();
        leaving.chosen();
        if close {
            leaving.exit();
            handle.exit(0);
        }
    });
    if let Err(error) = shown {
        eprintln!("BattlePlan couldn’t offer to close without its page’s answer, so the window stays open: {error}");
        app.state::<Leaving>().chosen();
    }
}

fn stop_page(window: WebviewWindow) -> Stop {
    Arc::new(move || {
        let stopped = window.with_webview(|webview| {
            if let Err(error) = unsafe { webview.controller().CoreWebView2().and_then(|page| page.Stop()) } {
                eprintln!("BattlePlan couldn’t stop its page’s reload while it leaves: {error}");
            }
        });
        if let Err(error) = stopped {
            eprintln!("BattlePlan couldn’t stop its page’s reload while it leaves: {error}");
        }
    })
}

trait Shut {
    fn disable(&self);
    fn hide(&self) -> tauri::Result<()>;
}

struct MainWindow {
    window: WebviewWindow,
    hwnd: isize,
}

impl Shut for MainWindow {
    fn disable(&self) {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
        unsafe { EnableWindow(self.hwnd as *mut std::ffi::c_void, 0) };
    }

    fn hide(&self) -> tauri::Result<()> {
        self.window.hide()
    }
}

fn shut(window: &impl Shut) {
    window.disable();
    if let Err(error) = window.hide() {
        eprintln!("BattlePlan couldn’t hide its window while it leaves, so a dialog that closes could let input back into its page: {error}");
    }
}

fn watch_page(window: &WebviewWindow) -> tauri::Result<()> {
    let app = window.app_handle().clone();
    let main = MainWindow { window: window.clone(), hwnd: window.hwnd()?.0 as isize };
    app.state::<Leaving>().follow(
        stop_page(window.clone()),
        Arc::new(move || shut(&main)),
    );
    window.with_webview(move |webview| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_PROCESS_FAILED_KIND, COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
            COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED, COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
        };
        use webview2_com::{ContentLoadingEventHandler, NavigationCompletedEventHandler, NavigationStartingEventHandler, ProcessFailedEventHandler};
        let failed = {
            let app = app.clone();
            ProcessFailedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND::default();
                unsafe { args.ProcessFailedKind(&mut kind)? };
                let leaving = app.state::<Leaving>();
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                    || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                {
                    eprintln!("BattlePlan’s page process ended, so closing no longer waits for it.");
                    if leaving.page_gone() {
                        app.exit(0);
                    }
                } else if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE && leaving.page_unresponsive() {
                    eprintln!("BattlePlan’s page isn’t responding, so its update questions were refused and the user is asked whether to close.");
                    offer_to_close(&app, Unanswered::Unresponsive);
                }
                Ok(())
            }))
        };
        let starting = {
            let app = app.clone();
            NavigationStartingEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut navigation = 0_u64;
                unsafe { args.NavigationId(&mut navigation)? };
                if !app.state::<Leaving>().navigation_starting(navigation) {
                    eprintln!("BattlePlan is leaving, so its page doesn’t reload until it has left or stays.");
                    unsafe { args.SetCancel(true)? };
                }
                Ok(())
            }))
        };
        let loading = {
            let app = app.clone();
            ContentLoadingEventHandler::create(Box::new(move |page, args| {
                let Some(args) = args else { return Ok(()) };
                let mut navigation = 0_u64;
                unsafe { args.NavigationId(&mut navigation)? };
                if app.state::<Leaving>().content_loading(navigation) {
                    eprintln!("BattlePlan is leaving, so the page a reload began loading is stopped.");
                    if let Some(page) = page {
                        unsafe { page.Stop()? };
                    }
                }
                Ok(())
            }))
        };
        let completed = {
            let app = app.clone();
            NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                let Some(args) = args else { return Ok(()) };
                let mut navigation = 0_u64;
                unsafe { args.NavigationId(&mut navigation)? };
                app.state::<Leaving>().navigation_completed(navigation);
                Ok(())
            }))
        };
        let mut token = 0_i64;
        let watched = unsafe {
            webview.controller().CoreWebView2().and_then(|page| {
                page.add_ProcessFailed(&failed, &mut token)?;
                page.add_NavigationStarting(&starting, &mut token)?;
                page.add_ContentLoading(&loading, &mut token)?;
                page.add_NavigationCompleted(&completed, &mut token)
            })
        };
        if let Err(error) = watched {
            eprintln!("BattlePlan can’t follow its page’s reloads and crashes, so it refuses updates by name and asks at each close: {error}");
            app.state::<Leaving>().blind();
        }
    })
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
    if app_update::refused_during_setup("BattlePlan") {
        return;
    }
    let context = context();
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
        .invoke_system(include_str!("ipc.js"))
        .register_asynchronous_uri_scheme_protocol("push", move |context, request, responder| {
            use tauri::http::{StatusCode, header};
            if request.headers().get(header::ORIGIN).and_then(|origin| origin.to_str().ok()) != Some(&page_origin) {
                return pull::refuse(responder, StatusCode::FORBIDDEN, "Only the dashboard may pull followed answers");
            }
            if request.uri().path() != "/follow" {
                return pull::refuse(responder, StatusCode::NOT_FOUND, "No such stream");
            }
            context.app_handle().state::<follow_push::FollowPush>().ask(&request, responder);
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
            viewer::host::install_follow(move |kind, payload| {
                if kind == dashboard_follow::push::FOLLOW || kind == dashboard_follow::push::PANEL {
                    push.state::<follow_push::FollowPush>().offer(|queue| queue.offer_kind(kind, payload));
                }
            })?;
            if let Some(window) = app.get_webview_window("main") {
                watch_page(&window)?;
                if let Err(error) = app_update::session::follow(window.hwnd()?.0 as isize, Arc::new(SessionEnding(app.handle().clone())), SAVING_CHANGES) {
                    eprintln!("BattlePlan can’t hear Windows end the session, so a sign-out or shutdown may not wait for its changes to be saved: {error}");
                }
                app.state::<NativeHost>().ensure_started(&window)?;
            }
            Ok(())
        })
        .manage(NativeHost::default())
        .manage(popout::PopOuts::default())
        .manage(DashboardService::default())
        .manage(follow_push::FollowPush::default())
        .manage(Leaving::default())
        .invoke_handler(tauri::generate_handler![
            video::video_settings,
            video::set_video_settings,
            video::map_diagnostics,
            video::set_map_diagnostics,
            video::map_muted,
            video::set_map_muted,
            desktop_state,
            playback_pressure_merges,
            close_watch_document,
            watch_close_requests,
            refresh_close_requests,
            answer_close_question,
            leave_app,
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
        } if label == "main" => match app.state::<Leaving>().close_requested() {
            Closing::Through => {
                if exit_after_maps(app) {
                    api.prevent_close();
                }
            }
            Closing::Held => api.prevent_close(),
            Closing::Offer(why) => {
                api.prevent_close();
                offer_to_close(app, why);
            }
        },
        tauri::RunEvent::ExitRequested { api, .. } if exit_after_maps(app) => api.prevent_exit(),
        tauri::RunEvent::Exit => app_update::session::end(&SessionEnding(app.clone())),
        _ => {}
    });
}

struct SessionEnding(tauri::AppHandle);

impl app_update::session::Ending for SessionEnding {
    fn commit(&self) {
        exit_after_maps(&self.0);
    }

    fn drained_within(&self, limit: std::time::Duration) -> bool {
        self.0.state::<Leaving>().launched_within(limit) && replay::writes::WRITES.drained_within(limit)
    }
}

const EXIT_BOUND: std::time::Duration = std::time::Duration::from_secs(5);
const STARTING_UPDATE: &str = "Starting the update before BattlePlan closes…";
const SAVING_CHANGES: &str = "Saving your changes before BattlePlan closes…";

fn within<T: Send + 'static>(limit: std::time::Duration, work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("shutdown".into())
        .spawn(move || {
            let _ = done.send(work());
        })
        .ok()?;
    finished.recv_timeout(limit).ok()
}

fn ordered<P: Send + 'static>(
    page: impl FnOnce() -> P,
    steps: impl FnOnce() -> bool + Send + 'static,
    summary: impl FnOnce(P) + Send + 'static,
    limit: std::time::Duration,
) -> Option<bool> {
    let page = page();
    let deadline = std::time::Instant::now().checked_add(limit);
    let ended = within(limit, steps);
    let left = deadline.map_or(limit, |deadline| deadline.saturating_duration_since(std::time::Instant::now()));
    let summarized = within(left, move || summary(page));
    ended.filter(|_| summarized.is_some())
}

fn drain_admitted(leaving: &Leaving, show: impl FnOnce(&'static str) -> Result<Saving, String>) {
    let launched = |limit: std::time::Duration| leaving.launched_within(limit);
    let written = |limit: std::time::Duration| replay::writes::WRITES.drained_within(limit);
    drained(&[Wait { what: STARTING_UPDATE, done_within: &launched }, Wait { what: SAVING_CHANGES, done_within: &written }], show);
}

fn end_now() -> ! {
    std::process::exit(0);
}

fn exit_after_maps(app: &tauri::AppHandle) -> bool {
    static EXIT: ExitOrder = ExitOrder::new();
    EXIT.begin(
        || app.state::<Leaving>().exit(),
        || {
            let handle = app.clone();
            std::thread::Builder::new()
                .name("ordered exit".into())
                .spawn(move || {
                    drain_admitted(handle.state::<Leaving>().inner(), |what| Saving::show("BattlePlan", what));
                    let ordered_steps = handle.clone();
                    let ended = ordered(|| perf::finish_page(&handle), move || shutdown(&ordered_steps), perf::finish, EXIT_BOUND);
                    EXIT.done();
                    if ended == Some(true) {
                        handle.exit(0);
                    } else {
                        if ended.is_none() {
                            eprintln!("BattlePlan’s shutdown didn’t finish within {} seconds, so it ends now.", EXIT_BOUND.as_secs());
                        }
                        end_now();
                    }
                })
                .map(drop)
        },
        || {
            drain_admitted(app.state::<Leaving>().inner(), |_| Err("its exit thread didn’t start".into()));
            end_now();
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

    fn begin(&self, commit: impl FnOnce(), start: impl FnOnce() -> std::io::Result<()>, fallback: impl FnOnce()) -> bool {
        use std::sync::atomic::Ordering;
        commit();
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
    use super::{ExitOrder, Shut, ordered, shut, within};
    use crate::leaving::Leaving;
    use std::cell::Cell;
    use std::time::Duration;

    #[test]
    fn a_shutdown_that_stalls_after_battleplan_commits_to_exit_ends_the_process_instead_of_leaving_it_running_unseen() {
        let (_never, stalled) = std::sync::mpsc::channel::<()>();
        let ended = within(Duration::ZERO, move || stalled.recv().is_ok());
        assert_eq!(ended, None, "a dashboard worker stuck reading an unresponsive share doesn’t hold the exit");
        assert_eq!(within(Duration::MAX, || true), Some(true), "a shutdown that finishes ends in order");

        let (_never, stalled) = std::sync::mpsc::channel::<()>();
        let (wrote, written) = std::sync::mpsc::channel();
        let ended = ordered(|| Some(true), move || stalled.recv().is_ok(), move |page| wrote.send(page).unwrap(), Duration::ZERO);
        assert_eq!(ended, None);
        assert_eq!(written.recv(), Ok(Some(true)), "a page that took its whole wait still has its summary written, though the steps after it overran");

        let (_never, stalled) = std::sync::mpsc::channel::<()>();
        let ended = ordered(|| (), || true, move |()| { let _ = stalled.recv(); }, Duration::from_millis(100));
        assert_eq!(ended, None, "a measurement summary that stalls ends the process from the exit thread, never on the UI thread after it");
    }

    #[test]
    fn the_ordered_exit_commits_to_exit_before_it_starts_anything_and_on_every_call() {
        let writes: &'static replay::writes::Writes = Box::leak(Box::new(replay::writes::Writes::new()));
        let leaving = Leaving::gated(writes);
        let exit = ExitOrder::new();
        assert!(exit.begin(|| leaving.exit(), || {
            assert!(writes.begin().is_err(), "an exit setup or Tauri asked for closes admission before the maps begin to shut");
            Ok(())
        }, || unreachable!()));
        assert_eq!(leaving.close_requested(), crate::leaving::Closing::Through);
        let mut committed = 0;
        assert!(exit.begin(|| committed += 1, || unreachable!(), || unreachable!()));
        assert_eq!(committed, 1, "a second exit request commits too, and then waits on the first");
    }

    struct Window {
        enabled: Cell<bool>,
        visible: Cell<bool>,
    }

    impl Shut for Window {
        fn disable(&self) {
            self.enabled.set(false);
        }

        fn hide(&self) -> tauri::Result<()> {
            self.visible.set(false);
            Ok(())
        }
    }

    struct Modal {
        disabled_owner: bool,
    }

    impl Modal {
        fn open(owner: &Window) -> Self {
            Self { disabled_owner: owner.enabled.replace(false) }
        }

        fn close(self, owner: &Window) {
            if self.disabled_owner {
                owner.enabled.set(true);
            }
        }
    }

    #[test]
    fn a_dialog_that_closes_after_battleplan_commits_to_exit_lets_no_input_back_into_its_window() {
        let window = Window { enabled: Cell::new(true), visible: Cell::new(true) };
        let choice = Modal::open(&window);
        shut(&window);
        choice.close(&window);
        assert!(window.enabled.get(), "the dialog re-enables the window it disabled, as Windows does");
        assert!(!window.visible.get(), "so the window is out of input by a means no dialog undoes");
    }


    #[test]
    fn a_close_stays_held_when_the_exit_cannot_start_and_the_fallback_ends_the_process() {
        let exit = ExitOrder::new();
        let mut fell_back = 0;
        let held = exit.begin(|| {}, || Err(std::io::Error::other("no thread")), || fell_back += 1);
        assert!(held, "the window must outlive the map's surface");
        assert_eq!(fell_back, 1);
        assert!(exit.begin(|| {}, || unreachable!(), || unreachable!()), "a later close is held too");

        let exit = ExitOrder::new();
        assert!(exit.begin(|| {}, || Ok(()), || unreachable!()));
        assert!(exit.begin(|| {}, || unreachable!(), || unreachable!()));
        exit.done();
        assert!(!exit.begin(|| {}, || unreachable!(), || unreachable!()), "once the maps end, closes go through");
    }
}
