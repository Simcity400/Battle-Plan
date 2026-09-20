//! Async proxy for native projected queries. No replay bytes or decoder live here.
//! Response assembly is bounded; parsing never occupies the native pipe reader.

use analytics::QueryRequest;
use desktop_protocol::{
    dashboard_data::{DataCommand, DataEvent, MAX_REPLY_BYTES, ReplyAssembler},
    replay_revision,
};
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tokio::sync::{Semaphore, oneshot};

use crate::native::NativeHost;

const MAX_PENDING: usize = 64;
const ASSEMBLY_BUDGET: usize = 64 * 1024 * 1024;
const CANCELLED: &str = "Dashboard request cancelled";

#[derive(Default)]
struct ResponseBudget(AtomicUsize);

struct BytePermit {
    budget: Arc<ResponseBudget>,
    bytes: usize,
}

impl ResponseBudget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<BytePermit, String> {
        if bytes == 0 || bytes > MAX_REPLY_BYTES {
            return Err("Dashboard response exceeds its byte budget".into());
        }
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|next| *next <= ASSEMBLY_BUDGET)
            })
            .map_err(|_| "Dashboard responses occupy the memory budget. Retry this view.")?;
        Ok(BytePermit {
            budget: Arc::clone(self),
            bytes,
        })
    }
}

impl Drop for BytePermit {
    fn drop(&mut self) {
        self.budget.0.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

struct RawReply {
    json: String,
    permit: BytePermit,
}

/// Retain the response admission charge until Tauri serializes the parsed DTO.
/// This bounds input-equivalent bytes, not allocator overhead or Value expansion.
pub struct ProjectedResponse {
    value: Value,
    _permit: BytePermit,
}

impl Serialize for ProjectedResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.value.serialize(serializer)
    }
}

struct PendingReply {
    reply: oneshot::Sender<Result<RawReply, String>>,
    assembler: ReplyAssembler,
    permit: Option<BytePermit>,
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

    fn begin(&mut self, id: &str) -> Result<oneshot::Receiver<Result<RawReply, String>>, String> {
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
            return Err("Dashboard request capacity reached. Retry this view.".into());
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
                permit: None,
            },
        );
        Ok(receive)
    }
}

pub struct DashboardService {
    requests: Mutex<Requests>,
    budget: Arc<ResponseBudget>,
    parsers: Arc<Semaphore>,
}

impl Default for DashboardService {
    fn default() -> Self {
        Self {
            requests: Mutex::new(Requests::default()),
            budget: Arc::default(),
            parsers: Arc::new(Semaphore::new(2)),
        }
    }
}

impl DashboardService {
    fn submit(
        &self,
        window: &WebviewWindow,
        host: &NativeHost,
        id: &str,
        command: DataCommand,
    ) -> Result<oneshot::Receiver<Result<RawReply, String>>, String> {
        host.ensure_started(window)?;
        // Admission and native enqueue are atomic with cancellation. The native
        // receiver releases its own lock before calling receive_data below.
        let mut requests = self.requests.lock().map_err(|error| error.to_string())?;
        let receive = requests.begin(id)?;
        if let Err(error) = host.enqueue_data(command) {
            requests.pending.remove(id);
            return Err(error);
        }
        Ok(receive)
    }

    fn cancel(&self, host: &NativeHost, id: String) -> Result<(), String> {
        if id.is_empty() || id.len() > 128 {
            return Err("Invalid dashboard request identifier".into());
        }
        let mut requests = self.requests.lock().map_err(|error| error.to_string())?;
        requests.remember_cancel(&id);
        if let Some(entry) = requests.pending.remove(&id) {
            let _ = entry.reply.send(Err(CANCELLED.into()));
        }
        host.enqueue_data(DataCommand::Cancel { id })
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
        self.parsers.close();
    }
}

/// Only bounded fragment assembly runs on the native reader. Completed strings
/// move to their awaiting command, which uses one of two parsing slots.
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
                let result = (|| {
                    if entry.permit.is_none() {
                        entry.permit = Some(service.budget.reserve(total_bytes as usize)?);
                    }
                    entry.assembler.push(sequence, total_bytes, last, &fragment)
                })();
                match result {
                    Ok(Some(json)) => {
                        let entry = requests
                            .pending
                            .remove(&id)
                            .expect("response still pending");
                        let raw = RawReply {
                            json,
                            permit: entry.permit.expect("assembled reply has byte permit"),
                        };
                        if entry.reply.send(Ok(raw)).is_err() {
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
    if let Some(window) = app.get_webview_window("main") {
        let _ = app
            .state::<NativeHost>()
            .send_data(&window, DataCommand::Cancel { id });
    }
}

pub fn disconnected(app: &AppHandle, message: &str) {
    app.state::<DashboardService>().fail_all(message, false);
}

async fn request(
    window: &WebviewWindow,
    host: &NativeHost,
    service: &DashboardService,
    id: String,
    command: DataCommand,
) -> Result<ProjectedResponse, String> {
    let receive = service.submit(window, host, &id, command)?;
    let raw = receive
        .await
        .map_err(|_| "Dashboard response channel closed".to_owned())??;
    let permit = Arc::clone(&service.parsers)
        .acquire_owned()
        .await
        .map_err(|_| "Dashboard is closing")?;
    if service
        .requests
        .lock()
        .map_err(|error| error.to_string())?
        .cancelled(&id)
    {
        return Err(CANCELLED.into());
    }
    let parsed = tauri::async_runtime::spawn_blocking(move || {
        let _parser = permit;
        serde_json::from_str(&raw.json)
            .map(|value| ProjectedResponse {
                value,
                _permit: raw.permit,
            })
            .map_err(|error| format!("Invalid dashboard response: {error}"))
    })
    .await
    .map_err(|error| error.to_string())?;
    if parsed.is_err() {
        let _ = host.send_data(window, DataCommand::Cancel { id: id.clone() });
    }
    if service
        .requests
        .lock()
        .map_err(|error| error.to_string())?
        .cancelled(&id)
    {
        return Err(CANCELLED.into());
    }
    parsed
}

#[tauri::command]
pub async fn dashboard_open_source(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    path: String,
    request_id: String,
) -> Result<ProjectedResponse, String> {
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
pub async fn dashboard_query(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    query: QueryRequest,
    request_id: String,
) -> Result<ProjectedResponse, String> {
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::Query {
            id: request_id,
            request: query,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_focus_history(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    query: desktop_protocol::dashboard_data::FocusRequest,
    request_id: String,
) -> Result<ProjectedResponse, String> {
    query.validate()?;
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::FocusHistory {
            id: request_id,
            request: query,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_country_flags(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    query: desktop_protocol::dashboard_data::CountryFlagRequest,
    request_id: String,
) -> Result<ProjectedResponse, String> {
    query.validate()?;
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::CountryFlags {
            id: request_id,
            request: query,
        },
    )
    .await
}

#[tauri::command]
pub async fn dashboard_land_owners(
    window: WebviewWindow,
    host: State<'_, NativeHost>,
    service: State<'_, DashboardService>,
    source_id: String,
    revision: String,
    request_id: String,
) -> Result<ProjectedResponse, String> {
    request(
        &window,
        &host,
        &service,
        request_id.clone(),
        DataCommand::LandOwners {
            id: request_id,
            source_id,
            revision,
        },
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
    target: Option<desktop_protocol::dashboard_data::MapSettingsTarget>,
    source_id: String,
    revision: String,
    settings: Option<desktop_protocol::CampaignMapSettings>,
    request_id: String,
) -> Result<ProjectedResponse, String> {
    request(
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
    host: State<'_, NativeHost>,
    source_id: String,
) -> Result<(), String> {
    host.send_data(&window, DataCommand::Release { source_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancellation_before_admission_cannot_create_an_orphaned_open() {
        let mut requests = Requests::default();
        requests.remember_cancel("cancelled-open");
        assert!(requests.begin("cancelled-open").is_err());
        assert!(requests.begin("other-widget").is_ok());
        assert_eq!(requests.pending.len(), 1);
    }

    #[test]
    fn response_budget_remains_charged_after_assembly_until_owner_drops() {
        let budget = Arc::new(ResponseBudget::default());
        let first = budget.reserve(MAX_REPLY_BYTES).unwrap();
        let second = budget.reserve(MAX_REPLY_BYTES).unwrap();
        assert!(budget.reserve(1).is_err());
        drop(first);
        assert!(budget.reserve(1).is_ok());
        drop(second);
        assert_eq!(budget.0.load(Ordering::Acquire), 0);
    }
}
