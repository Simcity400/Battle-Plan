#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod dashboard_service;
mod export_jobs;
mod export_settings;
mod export_webview;
mod follow_push;
mod game_windows;
mod leaving;
mod pull;
mod native;
mod page_memory;
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
async fn set_map_holes(host: State<'_, NativeHost>, holes: Vec<ViewBounds>) -> Result<(), String> {
    if let Some(applied) = host.set_holes(holes)? {
        applied.await.map_err(|_| "The map stopped or replaced its window holes before applying them".to_owned())?;
    }
    Ok(())
}

#[tauri::command]
fn select_native_view(host: State<'_, NativeHost>, dashboard: bool) -> Result<(), String> {
    host.select_view(dashboard)
}

#[tauri::command]
fn map_action(
    host: State<'_, NativeHost>,
    popouts: State<'_, popout::PopOuts>,
    game_windows: State<'_, game_windows::GameWindows>,
    action: dashboard_core::protocol::MapAction,
) -> Result<(), String> {
    if matches!(action, dashboard_core::protocol::MapAction::Folders { .. }) {
        return Err("Set the content folders in Settings".into());
    }
    if matches!(
        action,
        dashboard_core::protocol::MapAction::RefreshVisualMods
            | dashboard_core::protocol::MapAction::MapSettingsSaved
    ) {
        game_windows.clear_cache();
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
) -> Result<Option<viewer::host::Saved>, String> {
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

fn main_page(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() == "main" {
        Ok(())
    } else {
        Err("Only BattlePlan’s main window decides whether it may close.".into())
    }
}

#[derive(Default)]
struct ShellReady(std::sync::atomic::AtomicBool);

#[tauri::command]
async fn shell_ready(webview: tauri::Webview) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Only the main window starts BattlePlan’s background services.".into());
    }
    if !webview.state::<ShellReady>().0.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let sweep = tauri::async_runtime::spawn_blocking(export_settings::sweep_pending_stages);
        tauri::async_runtime::spawn_blocking(recorder::start_recorder).await.map_err(|error| error.to_string())?;
        sweep.await.map_err(|error| error.to_string())?;
    }
    Ok(())
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
fn refresh_close_requests(webview: tauri::Webview, leaving: State<'_, Leaving>, document: u64, seq: u64, attempt: u64) -> Result<u64, String> {
    main_page(&webview)?;
    leaving.refresh(document, seq, attempt).map_err(str::to_owned)
}

struct Attempt {
    app: tauri::AppHandle,
    document: u64,
    attempt: u64,
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.app.state::<Leaving>().attempt_ended(self.document, self.attempt);
    }
}

fn install_attempt(invoke: tauri::ipc::Invoke) -> bool {
    #[derive(serde::Deserialize)]
    struct Asked {
        document: Option<u64>,
        attempt: u64,
    }
    let app = invoke.message.webview().app_handle().clone();
    let asked = match invoke.message.payload() {
        tauri::ipc::InvokeBody::Json(value) => serde_json::from_value::<Asked>(value.clone()).map_err(|error| error.to_string()),
        tauri::ipc::InvokeBody::Raw(_) => Err("A restart is asked with its page and attempt.".to_owned()),
    };
    let registered = asked.and_then(|asked| {
        let document = asked.document.ok_or_else(|| leaving::UNWATCHED.to_owned())?;
        app.state::<Leaving>().attempt(document, asked.attempt).map_err(str::to_owned)?;
        Ok(Attempt { app: app.clone(), document, attempt: asked.attempt })
    });
    let attempt = match registered {
        Ok(attempt) => attempt,
        Err(refused) => {
            invoke.resolver.reject(refused);
            return true;
        }
    };
    invoke.resolver.respond_async(async move {
        tauri::async_runtime::spawn_blocking(move || {
            let _attempt = attempt;
            app_update::commands::install_now(&app)
        })
        .await
        .map_err(|error| tauri::ipc::InvokeError::from(error.to_string()))?
        .map_err(tauri::ipc::InvokeError::from)
    });
    true
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
        Answered::Leave | Answered::Heard => true,
        Answered::Unknown => false,
    })
}

#[tauri::command]
fn leave_app(webview: tauri::Webview, leaving: State<'_, Leaving>) -> Result<(), String> {
    main_page(&webview)?;
    leaving.exit();
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
    let hwnd = window.hwnd()?.0 as isize;
    let main = MainWindow { window: window.clone(), hwnd };
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
                unsafe { args.ProcessFailedKind(&raw mut kind)? };
                let leaving = app.state::<Leaving>();
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED
                    || kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_EXITED
                {
                    eprintln!("BattlePlan’s page process ended, so closing no longer waits for it.");
                    leaving.page_gone();
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
                unsafe { args.NavigationId(&raw mut navigation)? };
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
                unsafe { args.NavigationId(&raw mut navigation)? };
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
                unsafe { args.NavigationId(&raw mut navigation)? };
                app.state::<Leaving>().navigation_completed(navigation);
                Ok(())
            }))
        };
        let mut token = 0_i64;
        let watched = unsafe {
            webview.controller().CoreWebView2().and_then(|page| {
                page_memory::watch(hwnd, &page);
                page.add_ProcessFailed(&failed, &raw mut token)?;
                page.add_NavigationStarting(&starting, &raw mut token)?;
                page.add_ContentLoading(&loading, &raw mut token)?;
                page.add_NavigationCompleted(&completed, &raw mut token)
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
    if let Err(error) = dashboard_text::artwork::prepare_desktop() {
        eprintln!("{error}");
    }
    let exiting: &'static Exiting = match start_exit_worker() {
        Ok(exiting) => exiting,
        Err(error) => {
            eprintln!("BattlePlan couldn’t start, because the thread that closes it in order didn’t start: {error}");
            std::process::exit(2);
        }
    };
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
            match request.uri().path() {
                "/follow" => {
                    if pull::param(&request, "session").as_deref().is_some_and(dashboard_follow::registry::is_export_session) {
                        context.app_handle().state::<follow_push::ExportFollowPush>().ask(&request, responder);
                    } else {
                        context.app_handle().state::<follow_push::FollowPush>().ask(&request, responder);
                    }
                }
                "/credit" => match (pull::param(&request, "session"), pull::param(&request, "released").and_then(|released| released.parse::<u64>().ok())) {
                    (Some(session), Some(released)) => {
                        if dashboard_follow::registry::is_export_session(&session) {
                            if let Some(epoch) = context.app_handle().state::<follow_push::ExportFollowPush>().outbox(|relay| relay.epoch_of(&session)) {
                                dashboard_follow::registry::export_registry().credit(epoch, released);
                            }
                        } else if let Some(epoch) = context.app_handle().state::<follow_push::FollowPush>().outbox(|relay| relay.epoch_of(&session)) {
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
        .setup(move |app| {
            let _ = exiting.app.set(app.handle().clone());
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
            native::start_gamepad(app.handle().clone());
            let push = app.handle().clone();
            viewer::host::install_follow(move |kind, payload, session| {
                if kind == dashboard_follow::push::FOLLOW || kind == dashboard_follow::push::PANEL {
                    push.state::<follow_push::FollowPush>().offer(|queue| queue.offer(payload, session));
                }
            })?;
            let export_push = app.handle().clone();
            viewer::host::install_export_follow(move |kind, payload, session| {
                if kind == dashboard_follow::push::FOLLOW || kind == dashboard_follow::push::PANEL {
                    export_push.state::<follow_push::ExportFollowPush>().offer(|queue| queue.offer(payload, session));
                }
            })?;
            if let Some(window) = app.get_webview_window("main") {
                watch_page(&window)?;
                if let Err(error) = app_update::session::follow(window.hwnd()?.0 as isize, Arc::new(SessionEnding(app.handle().clone(), exiting))) {
                    eprintln!("BattlePlan can’t hear Windows end the session, so a sign-out or shutdown may not wait for its changes to be saved: {error}");
                }
                app.state::<NativeHost>().ensure_started(&window)?;
            }
            Ok(())
        })
        .manage(NativeHost::default())
        .manage(popout::PopOuts::default())
        .manage(DashboardService::default())
        .manage(game_windows::GameWindows::default())
        .manage(follow_push::FollowPush::default())
        .manage(follow_push::ExportFollowPush::default())
        .manage(export_jobs::ExportJobs::default())
        .manage(export_webview::ExportWebviewState::default())
        .manage(Leaving::default())
        .manage(ShellReady::default())
        .invoke_handler({
            let commands: fn(tauri::ipc::Invoke) -> bool = tauri::generate_handler![
            video::video_settings,
            video::set_video_settings,
            video::map_diagnostics,
            video::set_map_diagnostics,
            video::map_muted,
            video::set_map_muted,
            desktop_state,
            shell_ready,
            playback_pressure_merges,
            close_watch_document,
            watch_close_requests,
            refresh_close_requests,
            answer_close_question,
            leave_app,
            set_map_view,
            set_map_holes,
            select_native_view,
            map_action,
            export_map,
            export_settings::export_settings,
            export_settings::set_export_settings,
            export_settings::export_backends,
            export_settings::choose_ffmpeg_binary,
            export_jobs::video_export_state,
            export_jobs::video_export_active,
            export_jobs::map_export_snapshot,
            export_jobs::choose_video_export_path,
            export_jobs::preview_map_video,
            export_jobs::start_map_video_export,
            export_jobs::surface_export_timelines,
            export_jobs::video_export_page_ack,
            export_jobs::preview_surface_video,
            export_jobs::start_surface_video_export,
            export_jobs::cancel_video_export,
            export_jobs::save_partial_video_export,
            export_jobs::dismiss_video_export,
            export_jobs::reveal_export_file,
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
            game_windows::render_game_window,
            game_windows::cancel_game_window,
            game_windows::forget_game_window_cancel,
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
            ];
            move |invoke| if invoke.message.command() == "app_update_install" { install_attempt(invoke) } else { commands(invoke) }
        })
        .build(context)
        .expect("Couldn’t start BattlePlan");
    app.run(move |app, event| match event {
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } if label == "main" => {
            api.prevent_close();
            match app.state::<Leaving>().close_requested() {
                Closing::Through => app.state::<Leaving>().exit(),
                Closing::Held => {}
                Closing::Offer(why) => offer_to_close(app, why),
            }
        }
        tauri::RunEvent::ExitRequested { api, .. } if !exiting.done.is_set() => {
            app.state::<Leaving>().exit();
            api.prevent_exit();
        }
        tauri::RunEvent::Exit => {
            app.state::<Leaving>().exit();
            exiting.done.wait_serving_sent_messages();
        }
        _ => {}
    });
}

struct Exiting {
    done: app_update::session::Done,
    app: std::sync::OnceLock<tauri::AppHandle>,
}

struct SessionEnding(tauri::AppHandle, &'static Exiting);

impl app_update::session::Ending for SessionEnding {
    fn commit(&self) {
        self.0.state::<Leaving>().exit();
    }

    fn done(&self) -> &app_update::session::Done {
        &self.1.done
    }

    fn saving(&self) -> Option<&'static str> {
        replay_settings::exit::EXIT.held().map(saving)
    }

    fn check_drafts(&self, answer: Box<dyn FnOnce(app_update::session::Drafts) + Send>) {
        let app = self.0.clone();
        let asked = std::thread::Builder::new().name("shutdown drafts".into()).spawn(move || {
            answer(match app.state::<Leaving>().shutdown_query() {
                Some(false) => app_update::session::Drafts::Clean,
                Some(true) => app_update::session::Drafts::Unsaved,
                None => app_update::session::Drafts::Unknown,
            });
        });
        if let Err(error) = asked {
            eprintln!("BattlePlan couldn’t ask its page about unsaved changes before Windows ends the session: {error}");
        }
    }

    fn retire_drafts(&self) {
        self.0.state::<Leaving>().retire_shutdown_query();
    }
}

const EXIT_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

fn saving(held: replay_settings::exit::What) -> &'static str {
    match held {
        replay_settings::exit::What::Install => "Starting the update before BattlePlan closes…",
        replay_settings::exit::What::Export => "Saving the map picture before BattlePlan closes…",
        _ => "Saving your changes before BattlePlan closes…",
    }
}

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

fn end_now() -> ! {
    std::process::exit(0);
}

fn start_exit_worker() -> std::io::Result<&'static Exiting> {
    static EXITING: std::sync::OnceLock<Exiting> = std::sync::OnceLock::new();
    let done = app_update::session::Done::new()?;
    let exiting: &'static Exiting = EXITING.get_or_init(|| Exiting { done, app: std::sync::OnceLock::new() });
    std::thread::Builder::new().name("ordered exit".into()).spawn(move || {
        let exit = &replay_settings::exit::EXIT;
        exit.wait_sealed();
        app_update::saving::wait_until_empty(app_update::app_name(), &exit.notices(), &|limit| exit.empty_within(limit), &|| exit.held().map(saving), &|| exiting.done.awaited());
        app_update::control::stop();
        exiting.done.set();
        let Some(app) = exiting.app.get().cloned() else { end_now() };
        let steps = app.clone();
        let ended = ordered(|| perf::finish_page(&app), move || shutdown(&steps), perf::finish, EXIT_BOUND);
        if ended != Some(true) {
            if ended.is_none() {
                eprintln!("BattlePlan’s shutdown didn’t finish within {} seconds, so it ends now.", EXIT_BOUND.as_secs());
            }
            end_now();
        }
        app.exit(0);
        std::thread::sleep(EXIT_BOUND);
        end_now();
    })?;
    Ok(exiting)
}

fn shutdown(app: &tauri::AppHandle) -> bool {
    app.state::<export_jobs::ExportJobs>().cancel_and_cleanup_for_shutdown();
    app.state::<export_webview::ExportWebviewState>().close_window(app);
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
    use super::{Shut, shut};
    use std::cell::Cell;

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

}
