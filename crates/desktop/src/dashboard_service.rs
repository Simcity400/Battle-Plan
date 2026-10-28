//! Host-direct dashboard queries through the shared data library.
//! The native viewer owns map rendering; dashboard reads do not start it.

use dashboard::{
    protocol::replay_revision,
    transport::{DataCommand, DataEvent, MapSettingsTarget, ProductRequest, ReplyAssembler},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tauri::{AppHandle, Manager, State, WebviewWindow, ipc::Response};
use tokio::sync::oneshot;

use crate::native::NativeHost;

const MAX_PENDING: usize = 64;
const CANCELLED: &str = "Dashboard request cancelled";

struct PendingReply {
    reply: oneshot::Sender<Result<String, String>>,
    assembler: ReplyAssembler,
}

#[derive(Default)]
struct Requests {
    pending: BTreeMap<String, PendingReply>,
    cancelled: VecDeque<String>,
    closed: bool,
}

impl Requests {
    fn cancelled(&self, id: &str) -> bool {
        self.cancelled.iter().any(|old| old == id)
    }

    fn remember_cancel(&mut self, id: &str) {
        if !self.cancelled(id) {
            self.cancelled.push_back(id.to_owned());
            while self.cancelled.len() > MAX_PENDING * 4 {
                self.cancelled.pop_front();
            }
        }
    }

    fn begin(&mut self, id: &str) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        if id.is_empty() || id.len() > 128 || id.contains('\0') {
            return Err("Invalid dashboard request identifier".into());
        }
        if self.closed {
            return Err("The dashboard is closing".into());
        }
        if self.cancelled(id) {
            return Err(CANCELLED.into());
        }
        if self.pending.len() >= MAX_PENDING {
            // Starts with the service's own refusal, so the frontend tries it again.
            return Err(format!(
                "{}. Retry this view.",
                dashboard::transport::CAPACITY_REACHED
            ));
        }
        if self.pending.contains_key(id) {
            return Err("Dashboard request identifier was reused".into());
        }
        let (reply, receive) = oneshot::channel();
        self.pending.insert(
            id.to_owned(),
            PendingReply {
                reply,
                assembler: ReplyAssembler::default(),
            },
        );
        Ok(receive)
    }
}

#[derive(Default)]
pub struct DashboardService {
    requests: Mutex<Requests>,
    runtime: Mutex<Option<Arc<dashboard::Service>>>,
    configured: Mutex<Option<(std::path::PathBuf, std::path::PathBuf, bool)>>,
}

impl DashboardService {
    fn submit(
        &self,
        window: &WebviewWindow,
        id: &str,
        command: DataCommand,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
        let runtime = self.runtime(window.app_handle())?;
        let mut requests = self.requests.lock().map_err(|error| error.to_string())?;
        let receive = requests.begin(id)?;
        runtime.dispatch(command);
        Ok(receive)
    }

    pub(crate) fn runtime(&self, app: &AppHandle) -> Result<Arc<dashboard::Service>, String> {
        let runtime = {
            let mut saved = self.runtime.lock().map_err(|error| error.to_string())?;
            if self.requests.lock().map_err(|e| e.to_string())?.closed {
                return Err("The dashboard is closing".into());
            }
            if saved.is_none() {
                let app = app.clone();
                let runtime = dashboard::Service::new(Arc::new(move |event| {
                    receive_data(&app, event);
                    true
                }))
                .map_err(|error| error.to_string())?;
                runtime
                    .set_preparation_suspended(crate::game_process::game_running().unwrap_or(true));
                *saved = Some(runtime);
            }
            Arc::clone(saved.as_ref().expect("runtime initialized"))
        };
        let (config, allow_unverified) = host_config()?;
        runtime.set_background_preparation(config.background_preparation_enabled());
        let key = (
            config.game_dir.clone(),
            config.user_dir.clone(),
            allow_unverified,
        );
        let mut configured = self.configured.lock().map_err(|error| error.to_string())?;
        if configured.as_ref() != Some(&key) {
            // Recording-only products remain available when content refuses.
            if let Err(error) = runtime.configure_content(&config, allow_unverified) {
                eprintln!("Dashboard content unavailable: {error}");
            }
            *configured = Some(key);
        }
        Ok(runtime)
    }

    pub(crate) fn suspend_preparation(&self, suspended: bool) {
        if let Some(runtime) = self
            .runtime
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            runtime.set_preparation_suspended(suspended);
        }
    }

    fn cancel(&self, _host: &NativeHost, id: String) -> Result<(), String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid dashboard request identifier".into());
        }
        let mut requests = self.requests.lock().map_err(|error| error.to_string())?;
        requests.remember_cancel(&id);
        if let Some(entry) = requests.pending.remove(&id) {
            let _ = entry.reply.send(Err(CANCELLED.into()));
        }
        drop(requests);
        if let Some(runtime) = self
            .runtime
            .lock()
            .map_err(|error| error.to_string())?
            .as_ref()
        {
            runtime.dispatch(DataCommand::Cancel { id });
        }
        Ok(())
    }

    fn fail_all(&self, message: &str, closing: bool) {
        let mut requests = self
            .requests
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        requests.closed |= closing;
        let pending = std::mem::take(&mut requests.pending);
        for (id, entry) in pending {
            requests.remember_cancel(&id);
            let _ = entry.reply.send(Err(message.to_owned()));
        }
    }

    pub fn shutdown(&self) {
        self.fail_all("The dashboard data connection closed", true);
        let runtime = self
            .runtime
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(runtime) = runtime {
            runtime.shutdown();
        }
    }
}

/// The host's saved content preferences with this run's command-line folder
/// overrides, and whether unverified content is allowed.
pub(crate) fn host_config() -> Result<(dashboard::Config, bool), String> {
    let mut config = dashboard::Config::load().map_err(|error| error.to_string())?;
    let mut allow_unverified = false;
    let mut args = std::env::args_os().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--game-dir" {
            config.game_dir = args.next().ok_or("--game-dir requires a folder")?.into();
        } else if argument == "--user-dir" {
            config.user_dir = args.next().ok_or("--user-dir requires a folder")?.into();
        } else if argument == "--allow-unverified-content" {
            allow_unverified = true;
        }
    }
    Ok((config, allow_unverified))
}

/// The effective game and user folders, as the folder settings show them.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentFolders {
    game: String,
    user: String,
    content_ready: bool,
}

impl ContentFolders {
    fn of(config: &dashboard::Config) -> Self {
        Self {
            game: config.game_dir.to_string_lossy().into_owned(),
            user: config.user_dir.to_string_lossy().into_owned(),
            content_ready: config.resolve_options().is_ok(),
        }
    }
}

#[tauri::command]
pub async fn content_folders() -> Result<ContentFolders, String> {
    tauri::async_runtime::spawn_blocking(|| {
        host_config().map(|(config, _)| ContentFolders::of(&config))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Saves folders that name a usable installation, then reconfigures the host's
/// data runtime and sends them to a running native viewer.
#[tauri::command]
pub async fn set_content_folders(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    game: String,
    user: String,
) -> Result<ContentFolders, String> {
    for folder in [&game, &user] {
        if folder.trim().is_empty() || folder.len() > 32_000 || folder.contains(' ') {
            return Err("Choose the game and user folders.".into());
        }
    }
    let config = tauri::async_runtime::spawn_blocking(move || {
        let mut config = dashboard::Config::load().map_err(|error| error.to_string())?;
        config.game_dir = game.trim().into();
        config.user_dir = user.trim().into();
        config.save().map_err(|error| error.to_string())?;
        host_config().map(|(config, _)| config)
    })
    .await
    .map_err(|error| error.to_string())??;
    // Unchanged folders bind again too: an installation that was unavailable may be back.
    *service
        .configured
        .lock()
        .map_err(|error| error.to_string())? = None;
    service.runtime(window.app_handle())?;
    host.set_folders(&config);
    window
        .state::<crate::popout::PopOuts>()
        .set_folders(&config);
    Ok(ContentFolders::of(&config))
}

/// Only bounded fragment assembly runs on the native reader. Completed replies
/// move to their awaiting command.
pub fn receive_data(app: &AppHandle, event: DataEvent) {
    let service = app.state::<DashboardService>();
    let mut requests = service
        .requests
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let mut cancel = None;
    match event {
        DataEvent::Error { id, message } => {
            if let Some(entry) = requests.pending.remove(&id) {
                let _ = entry.reply.send(Err(message));
            }
        }
        DataEvent::Chunk {
            id,
            sequence,
            total_bytes,
            last,
            fragment,
        } => {
            if let Some(entry) = requests.pending.get_mut(&id) {
                match entry.assembler.push(sequence, total_bytes, last, &fragment) {
                    Ok(Some(json)) => {
                        let entry = requests
                            .pending
                            .remove(&id)
                            .expect("response still pending");
                        if entry.reply.send(Ok(json)).is_err() {
                            cancel = Some(id);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        let entry = requests
                            .pending
                            .remove(&id)
                            .expect("response still pending");
                        let _ = entry.reply.send(Err(error));
                        cancel = Some(id);
                    }
                }
            } else {
                // This includes an abandoned source-open receiver. The native
                // Cancel rolls back a lease created before its response arrived.
                cancel = Some(id);
            }
        }
    }
    drop(requests);
    if let Some(id) = cancel {
        cancel_native(app, id);
    }
}

fn cancel_native(app: &AppHandle, id: String) {
    if let Some(runtime) = app
        .state::<DashboardService>()
        .runtime
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
    {
        runtime.dispatch(DataCommand::Cancel { id });
    }
}

pub fn disconnected(_app: &AppHandle, _message: &str) {
    // A viewer disconnect does not close the independent host data runtime.
}

async fn request(
    window: &WebviewWindow,
    _host: &NativeHost,
    service: &DashboardService,
    id: String,
    command: DataCommand,
) -> Result<Response, String> {
    let submit_window = window.clone();
    let submit_id = id.clone();
    let receive = tauri::async_runtime::spawn_blocking(move || {
        submit_window
            .state::<DashboardService>()
            .submit(&submit_window, &submit_id, command)
    })
    .await
    .map_err(|error| error.to_string())??;
    let json = receive
        .await
        .map_err(|_| "Dashboard response channel closed".to_owned())??;
    if service
        .requests
        .lock()
        .map_err(|error| error.to_string())?
        .cancelled(&id)
    {
        return Err(CANCELLED.into());
    }
    // The frontend parses the JSON once; the host only checked its framing.
    Ok(Response::new(json.into_bytes()))
}

#[tauri::command]
pub async fn dashboard_open_source(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    path: String,
    request_id: String,
) -> Result<Response, String> {
    let path = crate::replay_path(&path)?;
    let revision = replay_revision(&path).map_err(|error| error.to_string())?;
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::Open {
            id: request_id,
            path: path.to_string_lossy().into_owned(),
            revision,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_product(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    query: ProductRequest,
    request_id: String,
) -> Result<Response, String> {
    query.validate()?;
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::Product {
            id: request_id,
            request: query,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_prepared_query(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    query: ProductRequest,
    refresh: Option<bool>,
    request_id: String,
) -> Result<Response, String> {
    query.validate()?;
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::PreparedQuery {
            id: request_id,
            request: query,
            refresh: refresh.unwrap_or(false),
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_preparation_preferences(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    enabled: Option<bool>,
    request_id: String,
) -> Result<Response, String> {
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::PreparationPreferences {
            id: request_id,
            enabled,
        },
    )
    .await
}

async fn preparation_command(
    window: &WebviewWindow,
    host: &NativeHost,
    service: &DashboardService,
    source_id: String,
    revision: String,
    request_id: String,
    action: dashboard::prepared::Action,
) -> Result<Response, String> {
    request(
        window,
        host,
        service,
        request_id.clone(),
        DataCommand::Preparation {
            id: request_id,
            source_id,
            revision,
            action,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_preparation_status(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    source_id: String,
    revision: String,
    request_id: String,
) -> Result<Response, String> {
    preparation_command(
        &window,
        &host,
        &service,
        source_id,
        revision,
        request_id,
        dashboard::prepared::Action::Status,
    )
    .await
}

#[tauri::command]
pub async fn dashboard_prepare_source(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    source_id: String,
    revision: String,
    request_id: String,
) -> Result<Response, String> {
    preparation_command(
        &window,
        &host,
        &service,
        source_id,
        revision,
        request_id,
        dashboard::prepared::Action::Start,
    )
    .await
}

#[tauri::command]
pub async fn dashboard_cancel_preparation(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    source_id: String,
    revision: String,
    request_id: String,
) -> Result<Response, String> {
    preparation_command(
        &window,
        &host,
        &service,
        source_id,
        revision,
        request_id,
        dashboard::prepared::Action::Cancel,
    )
    .await
}

#[tauri::command]
pub async fn dashboard_clear_preparation(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    source_id: String,
    revision: String,
    request_id: String,
) -> Result<Response, String> {
    preparation_command(
        &window,
        &host,
        &service,
        source_id,
        revision,
        request_id,
        dashboard::prepared::Action::Clear,
    )
    .await
}

#[tauri::command]
// Tauri injects window/state separately from the command's wire arguments.
#[allow(clippy::too_many_arguments)]
pub async fn dashboard_map_settings(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    target: Option<MapSettingsTarget>,
    source_id: String,
    revision: String,
    settings: Option<dashboard::protocol::CampaignMapSettings>,
    flip: Option<dashboard::protocol::MapSetting>,
    request_id: String,
) -> Result<Response, String> {
    let saving = settings.is_some() || flip.is_some();
    let response = request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::MapSettings {
            id: request_id,
            target: target.unwrap_or_default(),
            source_id,
            revision,
            settings,
            flip,
        },
    )
    .await;
    // The viewer process keeps its own copy of the preferences this host saves. A reply that
    // fails after the save (or a save that failed) still has the viewer read the file again,
    // which costs nothing when it did not change.
    if saving {
        host.map_settings_saved();
        window
            .state::<crate::popout::PopOuts>()
            .mirror(&dashboard::protocol::MapAction::MapSettingsSaved);
    }
    response
}

#[tauri::command]
pub async fn dashboard_history_cache(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    clear: bool,
    request_id: String,
) -> Result<Response, String> {
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::HistoryCache {
            id: request_id,
            clear,
        },
    )
    .await
}

#[tauri::command]
pub fn dashboard_cancel(
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    request_id: String,
) -> Result<(), String> {
    service.cancel(&host, request_id)
}

#[tauri::command]
pub fn dashboard_release_source(
    window: WebviewWindow,
    service: State<'_, DashboardService>,
    source_id: String,
) -> Result<(), String> {
    service
        .runtime(window.app_handle())?
        .dispatch(DataCommand::Release { source_id });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_past_capacity_are_refused_as_retryable_and_admitted_once_one_ends() {
        let mut requests = Requests::default();
        let receivers: Vec<_> = (0..MAX_PENDING)
            .map(|index| requests.begin(&format!("read-{index}")).unwrap())
            .collect();
        let refusal = requests.begin("late").unwrap_err();
        assert!(refusal.starts_with(dashboard::transport::CAPACITY_REACHED));
        assert!(dashboard::transport::transient_error(&refusal));
        // The desktop's other refusals are not for want of room.
        assert!(!dashboard::transport::transient_error(CANCELLED));
        drop(receivers);
        requests.pending.remove("read-0");
        assert!(requests.begin("late").is_ok());
    }

    #[test]
    fn cancellation_before_admission_cannot_create_an_orphaned_open() {
        let mut requests = Requests::default();
        requests.remember_cancel("open-1");
        assert_eq!(requests.begin("open-1").unwrap_err(), CANCELLED);
        let receive = requests.begin("open-2").unwrap();
        assert!(
            requests.begin("open-2").is_err(),
            "ids cannot be reused while pending"
        );
        drop(receive);
        for index in 0..MAX_PENDING * 5 {
            requests.remember_cancel(&format!("old-{index}"));
        }
        assert_eq!(requests.cancelled.len(), MAX_PENDING * 4);
        assert!(
            requests.begin("open-1").is_ok(),
            "evicted tombstones no longer block an id"
        );
    }

    #[test]
    fn assembled_replies_are_delivered_once_and_errors_replace_them() {
        let mut requests = Requests::default();
        let mut receive = requests.begin("p-1").unwrap();
        let entry = requests.pending.get_mut("p-1").unwrap();
        assert_eq!(entry.assembler.push(0, 6, false, "{\"a").unwrap(), None);
        let json = entry.assembler.push(1, 6, true, "\":1").unwrap().unwrap();
        let entry = requests.pending.remove("p-1").unwrap();
        entry.reply.send(Ok(json)).unwrap();
        assert_eq!(receive.try_recv().unwrap().unwrap(), "{\"a\":1");
        assert!(
            requests.begin("p-1").is_ok(),
            "a delivered id may be reused"
        );
    }
}
