//! Async proxy for native data requests. No replay bytes or decoder live here.
//! Replies are reassembled from bounded fragments and handed to the frontend
//! as raw JSON bytes; the host never parses them.

use dashboard::{
    protocol::replay_revision,
    transport::{DataCommand, DataEvent, MapSettingsTarget, ProductRequest, ReplyAssembler},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Mutex,
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
}

impl DashboardService {
    fn submit(
        &self,
        window: &WebviewWindow,
        host: &NativeHost,
        id: &str,
        command: DataCommand,
    ) -> Result<oneshot::Receiver<Result<String, String>>, String> {
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
    }
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
) -> Result<Response, String> {
    let receive = service.submit(window, host, &id, command)?;
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
    request_id: String,
) -> Result<Response, String> {
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
