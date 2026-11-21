
mod gamepad;
mod lifecycle;
mod pending;

use dashboard::protocol::{
    DashboardMapView, ViewBounds, ViewerState, validate_map_views,
};
pub use gamepad::start_gamepad;
use pending::{Pending, SeekTarget};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::WebviewWindow;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub map_details: Option<dashboard::protocol::MapDetails>,
    pub running: bool,
    pub ready: bool,
    pub visible: bool,
    pub viewer: ViewerState,
    pub error: Option<String>,
    pub camera_mode: dashboard::protocol::CameraMode,
    pub camera_overlays: bool,
    pub fly_aircraft: Option<dashboard::protocol::FlyAircraftStatus>,
}

#[derive(Default)]
struct Inner {
    map: Option<viewer::host::Map>,
    starting: bool,
    relaunch: bool,
    ended_waiters: Vec<Box<dyn FnOnce() + Send>>,
    generation: u64,
    status: HostStatus,
    bounds: Option<ViewBounds>,
    pending: Pending,
    requested_path: Option<String>,
    requested_revision: Option<String>,
    requested_seek: Option<SeekTarget>,
    dashboard_views: Option<Vec<DashboardMapView>>,
    active_dashboard: bool,
    shutting_down: bool,
    last_diagnostic: Option<String>,
    diagnostics: Option<bool>,
    folders: Option<dashboard::Config>,
    display: Vec<(u64, dashboard::protocol::MapAction)>,
    display_sent: u64,
    popout: Option<PopOutWindow>,
    muted: Option<bool>,
    exports: Vec<Export>,
    export_requests: u64,
}

struct Export {
    request: u64,
    generation: u64,
    answer: tokio::sync::oneshot::Sender<Result<String, String>>,
}

pub type StatusChanged = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct PopOutWindow {
    parent: usize,
    changed: StatusChanged,
}

impl Inner {
    fn export_ended(&mut self, generation: u64, request: u64, answer: Result<String, String>) {
        if let Some(at) = self.exports.iter().position(|export| export.request == request && export.generation == generation) {
            let _ = self.exports.remove(at).answer.send(answer);
        }
    }

    fn refuse_exports(&mut self, why: &str) {
        self.pending.actions.retain(|action| !matches!(action, dashboard::protocol::MapAction::Export { .. }));
        for export in self.exports.drain(..) {
            let _ = export.answer.send(Err(why.to_owned()));
        }
    }

    fn flush(&mut self) {
        let Some(map) = self.map.as_ref() else {
            return;
        };
        let visible = self.status.visible;
        if !self.pending.can_send(visible) {
            return;
        }
        for command in self.pending.take_commands(visible) {
            if !map.send(command) {
                break;
            }
        }
    }

    fn ask_export(&mut self, path: String, view_id: Option<String>) -> Result<tokio::sync::oneshot::Receiver<Result<String, String>>, String> {
        if self.map.is_none() || !self.status.ready || self.shutting_down {
            return Err("The native map is not ready".into());
        }
        self.pending.action_room()?;
        let request = self.export_requests + 1;
        let action = dashboard::protocol::MapAction::Export { path, view_id, request };
        action.validate().map_err(str::to_owned)?;
        self.export_requests = request;
        let (answer, answered) = tokio::sync::oneshot::channel();
        self.exports.push(Export { request, generation: self.generation, answer });
        self.pending.actions.push_back(action);
        self.flush();
        Ok(answered)
    }

    fn publish(&self, app: &tauri::AppHandle) {
        use tauri::Emitter;
        match &self.popout {
            Some(window) => (window.changed)(),
            None => {
                let _ = app.emit("viewer-state", self.status.clone());
            }
        }
    }
}

fn same_display(a: &dashboard::protocol::MapAction, b: &dashboard::protocol::MapAction) -> bool {
    use dashboard::protocol::MapAction;
    matches!(
        (a, b),
        (MapAction::Vsync { .. }, MapAction::Vsync { .. })
            | (MapAction::UiScale { .. }, MapAction::UiScale { .. })
            | (MapAction::FrameLimit { .. }, MapAction::FrameLimit { .. })
    )
}

fn launch_display(
    read: impl IntoIterator<Item = dashboard::protocol::MapAction>,
    sent: &[(u64, dashboard::protocol::MapAction)],
    since: u64,
) -> Vec<dashboard::protocol::MapAction> {
    let newer: Vec<_> = sent
        .iter()
        .filter(|(number, _)| *number > since)
        .map(|(_, action)| action)
        .collect();
    read.into_iter()
        .filter(|action| !newer.iter().any(|latest| same_display(latest, action)))
        .chain(newer.iter().map(|&action| action.clone()))
        .collect()
}

#[derive(Clone, Default)]
pub struct NativeHost {
    shared: Arc<Mutex<Inner>>,
}

impl NativeHost {
    fn flush(&self) {
        self.shared.lock().unwrap_or_else(|e| e.into_inner()).flush();
    }

    pub fn popout(parent: usize, changed: StatusChanged) -> Self {
        let host = Self::default();
        {
            let mut inner = host.shared.lock().unwrap_or_else(|e| e.into_inner());
            inner.popout = Some(PopOutWindow { parent, changed });
            inner.pending.campaign_map = true;
        }
        host
    }

    pub fn select_view(&self, dashboard: bool) -> Result<(), String> {
        let mut inner = self.shared.lock().map_err(|error| error.to_string())?;
        if inner.shutting_down {
            return Err("The application is closing".into());
        }
        inner.active_dashboard = dashboard;
        if dashboard {
            inner.pending.campaign_map = false;
            inner.pending.bounds = None;
            inner.pending.visibility = None;
            inner.pending.open = None;
            inner.pending.seek = None;
            inner.pending.maps = Some(inner.dashboard_views.clone().unwrap_or_default());
        } else {
            inner.pending.maps = None;
            inner.pending.campaign_map = true;
            inner.pending.bounds = inner.bounds;
            inner.status.visible = true;
            inner.pending.visibility = Some(true);
        }
        drop(inner);
        self.flush();
        Ok(())
    }

    pub fn set_dashboard_views(
        &self,
        window: &WebviewWindow,
        views: &[DashboardMapView],
    ) -> Result<(), String> {
        validate_map_views(views).map_err(str::to_owned)?;
        {
            let mut inner = self.shared.lock().map_err(|error| error.to_string())?;
            if inner.shutting_down {
                return Err("The application is closing".into());
            }
            inner.dashboard_views = Some(views.to_vec());
            if !inner.active_dashboard {
                return Ok(());
            }
            inner.status.visible = views
                .iter()
                .any(|view| view.visible && view.clip.width > 0 && view.clip.height > 0);
            inner.pending.maps = Some(views.to_vec());
            inner.pending.bounds = None;
            inner.pending.visibility = None;
            inner.pending.open = None;
            inner.pending.seek = None;
            inner.requested_path = None;
            inner.requested_revision = None;
            inner.requested_seek = None;
        }
        if !views.is_empty() {
            self.ensure_started(window)?;
        }
        self.flush();
        Ok(())
    }

    pub fn status(&self) -> HostStatus {
        self.shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    pub fn export(&self, path: String, view_id: Option<String>) -> Result<tokio::sync::oneshot::Receiver<Result<String, String>>, String> {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.ask_export(path, view_id)
    }

    pub fn map_action(&self, action: dashboard::protocol::MapAction) -> Result<(), String> {
        action.validate().map_err(str::to_owned)?;
        if matches!(action, dashboard::protocol::MapAction::Export { .. }) {
            return Err("An export is asked for with export_map, which answers it".into());
        }
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if let dashboard::protocol::MapAction::Unload { path } = &action
            && inner.map.is_none()
        {
            inner.forget_recording(path);
            return Ok(());
        }
        if !inner.status.ready || inner.shutting_down {
            return Err("The native map is not ready".into());
        }
        if matches!(
            action,
            dashboard::protocol::MapAction::CameraMode { .. }
                | dashboard::protocol::MapAction::FlyAircraft { .. }
        ) {
            let kind = std::mem::discriminant(&action);
            inner
                .pending
                .actions
                .retain(|queued| std::mem::discriminant(queued) != kind);
        }
        inner.pending.action_room()?;
        if let dashboard::protocol::MapAction::Unload { path } = &action {
            inner.forget_recording(path);
        }
        inner.pending.actions.push_back(action);
        drop(inner);
        self.flush();
        Ok(())
    }

    pub fn set_diagnostics(&self, enabled: bool) {
        use dashboard::protocol::MapAction;
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.diagnostics = Some(enabled);
        if inner.map.is_none() || inner.shutting_down {
            return;
        }
        inner
            .pending
            .actions
            .retain(|action| !matches!(action, MapAction::Diagnostics { .. }));
        inner
            .pending
            .actions
            .push_back(MapAction::Diagnostics { enabled });
        drop(inner);
        self.flush();
    }

    pub fn set_display(&self, action: dashboard::protocol::MapAction) {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.display_sent += 1;
        let number = inner.display_sent;
        inner
            .display
            .retain(|(_, kept)| !same_display(kept, &action));
        inner.display.push((number, action.clone()));
        if inner.map.is_none() || inner.shutting_down {
            return;
        }
        inner
            .pending
            .actions
            .retain(|queued| !same_display(queued, &action));
        inner.pending.actions.push_back(action);
        drop(inner);
        self.flush();
    }

    pub fn recordings(&self) -> Vec<String> {
        let inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        [
            inner.status.viewer.path.clone(),
            inner.requested_path.clone(),
            inner.requested_seek.as_ref().map(|seek| seek.path.clone()),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub fn map_settings_saved(&self) {
        self.queue_once(dashboard::protocol::MapAction::MapSettingsSaved);
    }

    pub fn refresh_visual_mods(&self) {
        self.queue_once(dashboard::protocol::MapAction::RefreshVisualMods);
    }

    fn queue_once(&self, action: dashboard::protocol::MapAction) {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if (inner.map.is_none() && !inner.starting) || inner.shutting_down {
            return;
        }
        let kind = std::mem::discriminant(&action);
        if !inner
            .pending
            .actions
            .iter()
            .any(|queued| std::mem::discriminant(queued) == kind)
        {
            inner.pending.actions.push_back(action);
        }
        drop(inner);
        self.flush();
    }

    pub fn set_muted(&self, muted: bool) {
        use dashboard::protocol::MapAction;
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.muted = Some(muted);
        if inner.map.is_none() || inner.shutting_down {
            return;
        }
        inner
            .pending
            .actions
            .retain(|action| !matches!(action, MapAction::Mute { .. }));
        inner.pending.actions.push_back(MapAction::Mute { muted });
        drop(inner);
        self.flush();
    }

    pub fn set_folders(&self, config: &dashboard::Config) {
        use dashboard::protocol::MapAction;
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.folders = Some(config.clone());
        if inner.map.is_none() || inner.shutting_down {
            return;
        }
        inner
            .pending
            .actions
            .retain(|action| !matches!(action, MapAction::Folders { .. }));
        inner.pending.actions.push_back(MapAction::Folders {
            game: config.game_dir.to_string_lossy().into_owned(),
            user: config.user_dir.to_string_lossy().into_owned(),
        });
        drop(inner);
        self.flush();
    }

    pub fn set_view(
        &self,
        window: &WebviewWindow,
        visible: bool,
        bounds: ViewBounds,
    ) -> Result<HostStatus, String> {
        let bounds = bounds.validate().map_err(str::to_owned)?;
        {
            let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
            inner.bounds = Some(bounds);
            if inner.active_dashboard {
                return Ok(inner.status.clone());
            }
            inner.status.visible = visible;
            inner.pending.bounds = Some(bounds);
            inner.pending.visibility = Some(visible);
        }
        if visible {
            self.ensure_started(window)?;
        }
        self.flush();
        Ok(self.status())
    }

    pub fn set_bounds(&self, bounds: ViewBounds) -> Result<(), String> {
        let bounds = bounds.validate().map_err(str::to_owned)?;
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        inner.bounds = Some(bounds);
        if inner.map.is_some() && !inner.active_dashboard {
            inner.pending.bounds = Some(bounds);
        }
        drop(inner);
        self.flush();
        Ok(())
    }

    pub fn open(&self, path: String) -> Result<(), String> {
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        inner.dashboard_views = None;
        inner.pending.maps = None;
        inner.requested_path = Some(path.clone());
        inner.requested_revision = None;
        inner.requested_seek = None;
        inner.pending.open = Some((path, None));
        inner.pending.seek = None;
        drop(inner);
        self.flush();
        Ok(())
    }

    pub fn seek(
        &self,
        path: String,
        revision: String,
        branch: String,
        hour: i32,
        playback: Option<dashboard::protocol::MapPlayback>,
    ) -> Result<(), String> {
        if let Some(playback) = playback {
            playback.validate()?;
        }
        branch.parse::<u64>().map_err(|_| "Invalid replay branch")?;
        if revision.is_empty() || revision.len() > 128 {
            return Err("A map seek must carry the revision its analysis read".into());
        }
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        inner.dashboard_views = None;
        inner.pending.maps = None;
        let current_matches = inner.status.viewer.path.as_ref() == Some(&path)
            && inner.status.viewer.revision.as_ref() == Some(&revision)
            && inner.requested_path.is_none();
        if !current_matches {
            inner.requested_path = Some(path.clone());
            inner.requested_revision = Some(revision.clone());
            inner.pending.open = Some((path.clone(), Some(revision.clone())));
        }
        let target = SeekTarget {
            path,
            revision,
            branch,
            hour,
            playback,
        };
        inner.requested_seek = Some(target.clone());
        inner.pending.seek = Some(target);
        drop(inner);
        self.flush();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
