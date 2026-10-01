//! Hosts the existing native viewer, with a bounded pipe for control messages.

mod job;
mod lifecycle;
mod line_reader;
mod pending;

use dashboard::protocol::{
    DashboardMapView, MAX_MESSAGE_BYTES, NativeCommand, ViewBounds, ViewerState, replay_revision,
    validate_map_views,
};
use dashboard::transport::DataCommand;
use lifecycle::Process;
use pending::{Pending, SeekTarget};
use serde::Serialize;
use std::sync::{Arc, Condvar, Mutex};
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
}

#[derive(Default)]
struct Inner {
    process: Option<Process>,
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
    /// The last diagnostics choice sent with [`NativeHost::set_diagnostics`],
    /// which a viewer started afterwards takes over its launch-time read.
    diagnostics: Option<bool>,
}

#[derive(Clone, Default)]
pub struct NativeHost {
    shared: Arc<(Mutex<Inner>, Condvar)>,
}

impl NativeHost {
    /// Select the geometry owner before the frontend mounts/unmounts that view.
    /// Late cleanup from the other owner cannot hide the newly selected surface.
    pub fn select_view(&self, dashboard: bool) -> Result<(), String> {
        let mut inner = self.shared.0.lock().map_err(|error| error.to_string())?;
        if inner.shutting_down {
            return Err("The application is closing".into());
        }
        inner.active_dashboard = dashboard;
        if dashboard {
            inner.pending.standalone = false;
            inner.pending.bounds = None;
            inner.pending.visibility = None;
            inner.pending.open = None;
            inner.pending.seek = None;
            inner.pending.maps = Some(inner.dashboard_views.clone().unwrap_or_default());
        } else {
            inner.pending.maps = None;
            inner.pending.standalone = true;
            inner.pending.bounds = inner.bounds;
            inner.status.visible = true;
            inner.pending.visibility = Some(true);
        }
        drop(inner);
        self.shared.1.notify_all();
        Ok(())
    }

    pub(crate) fn send_data(
        &self,
        window: &WebviewWindow,
        request: DataCommand,
    ) -> Result<(), String> {
        let cleanup = matches!(
            request,
            DataCommand::Cancel { .. } | DataCommand::Release { .. }
        );
        if !cleanup {
            // The data runtime starts without a visible map or game assets.
            self.ensure_started(window)?;
        }
        self.enqueue_data(request)
    }

    /// Enqueue without creating a window/process. Request admission can hold its
    /// own small lock across this operation without an OS-window callback.
    pub(crate) fn enqueue_data(&self, request: DataCommand) -> Result<(), String> {
        let cleanup = matches!(
            request,
            DataCommand::Cancel { .. } | DataCommand::Release { .. }
        );
        let length = serde_json::to_vec(&NativeCommand::Data {
            request: request.clone(),
        })
        .map_err(|error| error.to_string())?
        .len()
            + 1;
        if length > MAX_MESSAGE_BYTES {
            return Err("Dashboard request exceeds the native control message budget".into());
        }
        let mut inner = self.shared.0.lock().map_err(|error| error.to_string())?;
        if inner.shutting_down || inner.process.is_none() {
            if cleanup {
                return Ok(());
            }
            return Err("The native data connection closed".into());
        }
        inner.pending.enqueue_data(request, length)?;
        drop(inner);
        self.shared.1.notify_all();
        Ok(())
    }

    /// One replacement snapshot for every native map, including hidden bodies.
    /// The window and process remain shared regardless of the number of maps.
    pub fn set_dashboard_views(
        &self,
        window: &WebviewWindow,
        views: &[DashboardMapView],
    ) -> Result<(), String> {
        validate_map_views(views).map_err(str::to_owned)?;
        let command = NativeCommand::MapViews {
            views: views.to_vec(),
        };
        let bytes = serde_json::to_vec(&command).map_err(|error| error.to_string())?;
        if bytes.len() + 1 > MAX_MESSAGE_BYTES {
            return Err("The map layout exceeds the native control message budget".into());
        }
        {
            let mut inner = self.shared.0.lock().map_err(|error| error.to_string())?;
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
        self.shared.1.notify_all();
        Ok(())
    }

    pub fn status(&self) -> HostStatus {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .status
            .clone()
    }

    pub fn map_action(&self, action: dashboard::protocol::MapAction) -> Result<(), String> {
        action.validate().map_err(str::to_owned)?;
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if let dashboard::protocol::MapAction::Unload { path } = &action
            && inner.process.is_none()
        {
            inner.forget_recording(path);
            return Ok(());
        }
        if !inner.status.ready || inner.shutting_down {
            return Err("The native map is not ready".into());
        }
        if inner.pending.actions.len() >= 16 {
            return Err("Map controls are busy; try again".into());
        }
        if let dashboard::protocol::MapAction::Unload { path } = &action {
            inner.forget_recording(path);
        }
        inner.pending.actions.push_back(action);
        drop(inner);
        self.shared.1.notify_all();
        Ok(())
    }

    /// Sends the diagnostics choice to a running viewer, including one still
    /// starting, and keeps it for a viewer `ensure_started` starts later.
    /// The caller holds the saved choice's lock, so changes arrive in the
    /// order they were saved.
    pub fn set_diagnostics(&self, enabled: bool) {
        use dashboard::protocol::MapAction;
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        inner.diagnostics = Some(enabled);
        if inner.process.is_none() || inner.shutting_down {
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
        self.shared.1.notify_all();
    }

    pub fn set_view(
        &self,
        window: &WebviewWindow,
        visible: bool,
        bounds: ViewBounds,
    ) -> Result<HostStatus, String> {
        let bounds = bounds.validate().map_err(str::to_owned)?;
        {
            let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
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
        self.shared.1.notify_all();
        Ok(self.status())
    }

    pub fn open(&self, path: String) -> Result<(), String> {
        if path.len() > 32_000 {
            return Err("Replay path is too long".into());
        }
        let revision = replay_revision(&path).map_err(|e| e.to_string())?;
        let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        inner.dashboard_views = None;
        inner.pending.maps = None;
        inner.requested_path = Some(path.clone());
        inner.requested_revision = Some(revision.clone());
        inner.requested_seek = None;
        inner.pending.open = Some((path, revision));
        inner.pending.seek = None;
        drop(inner);
        self.shared.1.notify_all();
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
        if replay_revision(&path).map_err(|e| e.to_string())? != revision {
            return Err("The recording changed since this analysis. Refresh the analysis before viewing this sample on the map.".into());
        }
        let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        inner.dashboard_views = None;
        inner.pending.maps = None;
        let current_matches = inner.status.viewer.path.as_ref() == Some(&path)
            && inner.status.viewer.revision.as_ref() == Some(&revision)
            && inner.requested_path.is_none();
        if !current_matches {
            inner.requested_path = Some(path.clone());
            inner.requested_revision = Some(revision.clone());
            inner.pending.open = Some((path.clone(), revision.clone()));
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
        self.shared.1.notify_all();
        Ok(())
    }
}

#[cfg(test)]
mod tests;
