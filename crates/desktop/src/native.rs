
mod gamepad;
mod lifecycle;
mod pending;

use dashboard_core::protocol::{DashboardMapView, ViewBounds, ViewerState, validate_map_views};
pub use gamepad::start_gamepad;
use pending::{DashboardSnapshot, Pending, SeekTarget};
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::WebviewWindow;

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    pub map_details: Option<dashboard_core::protocol::MapDetails>,
    pub running: bool,
    pub ready: bool,
    pub visible: bool,
    pub viewer: ViewerState,
    pub error: Option<String>,
    pub camera_mode: dashboard_core::protocol::CameraMode,
    pub camera_overlays: bool,
    pub fly_aircraft: Option<dashboard_core::protocol::FlyAircraftStatus>,
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
    holes: Vec<ViewBounds>,
    pending: Pending,
    requested_path: Option<String>,
    requested_revision: Option<String>,
    requested_seek: Option<SeekTarget>,
    dashboard_views: Option<DashboardSnapshot>,
    active_dashboard: bool,
    shutting_down: bool,
    last_diagnostic: Option<String>,
    diagnostics: Option<bool>,
    folders: Option<dashboard_core::config::Config>,
    display: Vec<(u64, dashboard_core::protocol::MapAction)>,
    display_sent: u64,
    popout: Option<PopOutWindow>,
    muted: Option<bool>,
    clock_origins: viewer::page_clock::OriginGate,
}

pub type StatusChanged = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
struct PopOutWindow {
    parent: usize,
    changed: StatusChanged,
}

impl Inner {
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
                self.pending.hole_waiters.clear();
                return;
            }
        }
        if let Some(rects) = self.pending.holes.take() {
            let waiting = std::mem::take(&mut self.pending.hole_waiters);
            let applied = Box::new(move || waiting.into_iter().for_each(|waiter| { let _ = waiter.send(()); }));
            map.send(viewer::host::Command::Holes(viewer::host::Holes { rects, applied }));
        }
    }

    fn set_holes(&mut self, holes: Vec<ViewBounds>) -> Result<Option<tokio::sync::oneshot::Receiver<()>>, String> {
        dashboard_core::protocol::validate_map_holes(&holes).map_err(str::to_owned)?;
        let shown = self.map.is_some() && self.status.ready;
        self.holes.clone_from(&holes);
        self.pending.holes = Some(holes);
        let answered = shown.then(|| {
            let (applied, answered) = tokio::sync::oneshot::channel();
            self.pending.hole_waiters.push(applied);
            answered
        });
        self.flush();
        Ok(answered)
    }

    fn ask_export(&mut self, path: String, view_id: Option<String>) -> Result<tokio::sync::oneshot::Receiver<viewer::host::Answer>, String> {
        if self.map.is_none() || !self.status.ready || self.shutting_down {
            return Err("The map isn’t ready yet".into());
        }
        self.pending.action_room()?;
        dashboard_core::protocol::validate_export(&path, view_id.as_deref()).map_err(str::to_owned)?;
        let receipt = replay_settings::exit::EXIT.admit(replay_settings::exit::What::Export).map_err(|error| error.to_string())?;
        let (answer, answered) = tokio::sync::oneshot::channel();
        let reply = viewer::host::Reply::new(move |result| drop(answer.send(result)));
        let job = viewer::host::ExportJob::new(path, view_id, receipt, reply);
        self.flush();
        if let Some(Err(job)) = self.map.as_ref().map(|map| map.send_export(job)) {
            job.answer(Err(viewer::host::STOPPED.to_owned()));
        }
        Ok(answered)
    }

    pub(super) fn dashboard_snapshot(&self) -> DashboardSnapshot {
        self.dashboard_views
            .clone()
            .unwrap_or_else(|| DashboardSnapshot {
                clock_id: self.clock_origins.active().map(str::to_owned),
                views: Vec::new(),
            })
    }

    fn apply_dashboard_views(
        &mut self,
        clock_id: &str,
        views: &[DashboardMapView],
    ) -> Result<bool, String> {
        validate_map_views(clock_id, views).map_err(str::to_owned)?;
        if self.shutting_down {
            return Err("BattlePlan is closing.".into());
        }
        if self.clock_origins.admit(clock_id) == viewer::page_clock::OriginStatus::Retired {
            return Ok(false);
        }
        let snapshot = DashboardSnapshot {
            clock_id: Some(clock_id.to_owned()),
            views: views.to_vec(),
        };
        self.dashboard_views = Some(snapshot.clone());
        if !self.active_dashboard {
            return Ok(false);
        }
        self.status.visible = views
            .iter()
            .any(|view| view.visible && view.clip.width > 0 && view.clip.height > 0);
        self.pending.maps = Some(snapshot);
        self.pending.bounds = None;
        self.pending.visibility = None;
        self.pending.open = None;
        self.pending.seek = None;
        self.requested_path = None;
        self.requested_revision = None;
        self.requested_seek = None;
        Ok(true)
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

fn same_display(a: &dashboard_core::protocol::MapAction, b: &dashboard_core::protocol::MapAction) -> bool {
    use dashboard_core::protocol::MapAction;
    matches!(
        (a, b),
        (MapAction::Vsync { .. }, MapAction::Vsync { .. })
            | (MapAction::UiScale { .. }, MapAction::UiScale { .. })
            | (MapAction::FrameLimit { .. }, MapAction::FrameLimit { .. })
    )
}

fn launch_display(
    read: impl IntoIterator<Item = dashboard_core::protocol::MapAction>,
    sent: &[(u64, dashboard_core::protocol::MapAction)],
    since: u64,
) -> Vec<dashboard_core::protocol::MapAction> {
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
            return Err("BattlePlan is closing.".into());
        }
        inner.active_dashboard = dashboard;
        if dashboard {
            inner.pending.campaign_map = false;
            inner.pending.bounds = None;
            inner.pending.visibility = None;
            inner.pending.open = None;
            inner.pending.seek = None;
            inner.pending.maps = Some(inner.dashboard_snapshot());
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
        clock_id: &str,
        views: &[DashboardMapView],
    ) -> Result<(), String> {
        let admitted = {
            let mut inner = self.shared.lock().map_err(|error| error.to_string())?;
            inner.apply_dashboard_views(clock_id, views)?
        };
        if !admitted {
            return Ok(());
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

    pub fn set_holes(&self, holes: Vec<ViewBounds>) -> Result<Option<tokio::sync::oneshot::Receiver<()>>, String> {
        self.shared.lock().map_err(|e| e.to_string())?.set_holes(holes)
    }

    pub fn export(&self, path: String, view_id: Option<String>) -> Result<tokio::sync::oneshot::Receiver<viewer::host::Answer>, String> {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        inner.ask_export(path, view_id)
    }

    pub fn map_action(&self, action: dashboard_core::protocol::MapAction) -> Result<(), String> {
        action.validate().map_err(str::to_owned)?;
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if let dashboard_core::protocol::MapAction::Unload { path } = &action
            && inner.map.is_none()
        {
            inner.forget_recording(path);
            return Ok(());
        }
        if !inner.status.ready || inner.shutting_down {
            return Err("The map isn’t ready yet".into());
        }
        if matches!(
            action,
            dashboard_core::protocol::MapAction::CameraMode { .. }
                | dashboard_core::protocol::MapAction::FlyAircraft { .. }
        ) {
            let kind = std::mem::discriminant(&action);
            inner
                .pending
                .actions
                .retain(|queued| std::mem::discriminant(queued) != kind);
        }
        inner.pending.action_room()?;
        if let dashboard_core::protocol::MapAction::Unload { path } = &action {
            inner.forget_recording(path);
        }
        inner.pending.actions.push_back(action);
        drop(inner);
        self.flush();
        Ok(())
    }

    pub fn set_diagnostics(&self, enabled: bool) {
        use dashboard_core::protocol::MapAction;
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

    pub fn set_display(&self, action: dashboard_core::protocol::MapAction) {
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
        self.queue_once(dashboard_core::protocol::MapAction::MapSettingsSaved);
    }

    pub fn refresh_visual_mods(&self) {
        self.queue_once(dashboard_core::protocol::MapAction::RefreshVisualMods);
    }

    fn queue_once(&self, action: dashboard_core::protocol::MapAction) {
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
        use dashboard_core::protocol::MapAction;
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

    pub fn set_folders(&self, config: &dashboard_core::config::Config) {
        use dashboard_core::protocol::MapAction;
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
        playback: Option<dashboard_core::protocol::MapPlayback>,
    ) -> Result<(), String> {
        if let Some(playback) = &playback {
            playback.validate()?;
        }
        branch.parse::<u64>().map_err(|_| "Invalid campaign history")?;
        if revision.is_empty() || revision.len() > 128 {
            return Err("A map seek must carry the revision its analysis read".into());
        }
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        if playback.as_ref().is_some_and(|clock| {
            inner.clock_origins.admit(&clock.clock_id)
                == viewer::page_clock::OriginStatus::Retired
        }) {
            return Ok(());
        }
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
