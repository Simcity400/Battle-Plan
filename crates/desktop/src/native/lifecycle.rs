use super::NativeHost;
use dashboard_core::protocol::{DashboardMapState, NativeEvent, ViewerState};
use std::{sync::Arc, thread};
use tauri::{AppHandle, Emitter, WebviewWindow};
use viewer::host::{Ending, Launch, LaunchOptions, Map, Role};

impl super::Inner {
    fn publish_stopped(&self, app: &AppHandle) {
        self.publish(app);
        if let Some(snapshot) = &self.dashboard_views {
            let states: Vec<_> = snapshot
                .views
                .iter()
                .map(|view| DashboardMapState {
                    id: view.id.clone(),
                    clock_id: view.clock_id.clone(),
                    requested_sequence: view.sequence,
                    error: self.status.error.clone(),
                    ..Default::default()
                })
                .collect();
            let _ = app.emit("dashboard-map-state", states);
        }
    }

    pub(super) fn filter_map_states(
        &self,
        mut states: Vec<DashboardMapState>,
    ) -> Vec<DashboardMapState> {
        let Some(requested) = self.dashboard_views.as_ref() else {
            return Vec::new();
        };
        states.retain_mut(|state| {
            if requested.clock_id.as_deref() != Some(state.clock_id.as_str()) {
                return false;
            }
            let Some(view) = requested.views.iter().find(|view| view.id == state.id) else {
                return false;
            };
            if state.clock_id != view.clock_id
                || state
                    .source_id
                    .as_ref()
                    .is_some_and(|id| *id != view.source_id)
                || state
                    .revision
                    .as_ref()
                    .is_some_and(|revision| *revision != view.revision)
            {
                return false;
            }
            if state.requested_sequence < view.sequence {
                state.requested_sequence = view.sequence;
                state.loading = true;
                state.error = None;
            }
            true
        });
        states
    }

    pub(super) fn map_ended(&mut self, ending: Ending) -> bool {
        let was_ready = self.status.ready;
        self.map = None;
        self.generation += 1;
        self.status.running = false;
        self.status.ready = false;
        self.status.fly_aircraft = None;
        if self.shutting_down {
            return false;
        }
        let error = match ending {
            Ending::DeviceLost(reason) if was_ready => {
                self.last_diagnostic = Some(reason);
                self.status.error = None;
                return true;
            }
            Ending::DeviceLost(reason) => {
                format!("The map lost its graphics device again before it was ready, so it wasn’t started again: {reason}")
            }
            Ending::Closed => "The map closed".to_owned(),
            Ending::Failed(reason) => reason,
        };
        self.status.error = Some(match self.last_diagnostic.as_ref() {
            Some(detail) if *detail != error => format!("{error}: {detail}"),
            _ => error,
        });
        false
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Launched {
    Again,
    Running,
    Stopped,
}

impl super::Inner {
    pub(super) fn hear(&mut self, event: NativeEvent, generation: u64) -> Option<NativeEvent> {
        (self.generation == generation && !self.shutting_down && self.map.is_some()).then_some(event)
    }

    fn begin_launch(&mut self) -> Result<bool, String> {
        if self.map.is_some() {
            return Ok(false);
        }
        if self.starting {
            self.relaunch = !self.shutting_down;
            return Ok(false);
        }
        if self.shutting_down {
            return Err("BattlePlan is closing.".into());
        }
        self.starting = true;
        self.status.running = true;
        self.status.ready = false;
        self.status.error = None;
        Ok(true)
    }

    fn launch_finished(&mut self, started: Result<(), String>) -> Launched {
        if std::mem::take(&mut self.relaunch) && self.map.is_none() && !self.shutting_down {
            self.status.running = true;
            self.status.ready = false;
            self.status.error = None;
            return Launched::Again;
        }
        self.starting = false;
        match started {
            Err(error) if self.map.is_none() => {
                self.status.running = false;
                self.status.ready = false;
                if !self.shutting_down {
                    self.status.error = Some(error);
                }
                Launched::Stopped
            }
            _ => Launched::Running,
        }
    }

    fn take_ended_waiters(&mut self) -> Vec<Box<dyn FnOnce() + Send>> {
        if self.map.is_some() || self.starting {
            return Vec::new();
        }
        std::mem::take(&mut self.ended_waiters)
    }
}

impl NativeHost {
    pub(crate) fn ensure_started(&self, window: &WebviewWindow) -> Result<(), String> {
        use tauri::Manager;
        let popout = {
            let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
            if !inner.begin_launch()? {
                return Ok(());
            }
            inner.popout.is_some()
        };
        let own = (!popout).then(|| {
            window
                .hwnd()
                .map(|hwnd| hwnd.0 as usize)
                .map_err(|e| e.to_string())
        });
        let host = self.clone();
        let window = window.clone();
        thread::spawn(move || loop {
            let started = host.start(&window, own.clone());
            let mut inner = host.shared.lock().unwrap_or_else(|e| e.into_inner());
            let launched = inner.launch_finished(started);
            match launched {
                Launched::Again => {}
                Launched::Running => inner.publish(window.app_handle()),
                Launched::Stopped => inner.publish_stopped(window.app_handle()),
            }
            let waiting = inner.take_ended_waiters();
            drop(inner);
            waiting.into_iter().for_each(|after| after());
            if launched != Launched::Again {
                break;
            }
        });
        Ok(())
    }

    fn start(
        &self,
        window: &WebviewWindow,
        own: Option<Result<usize, String>>,
    ) -> Result<(), String> {
        use tauri::Manager;

        let since = self
            .shared
            .lock()
            .map_err(|e| e.to_string())?
            .display_sent;
        let display = crate::video::launch_actions(window)?;
        let saved_folders = crate::dashboard_service::host_config()
            .ok()
            .map(|(config, _)| config);
        let saved_diagnostics =
            crate::video::current_map_diagnostics(&window.state::<crate::video::VideoState>());
        let saved_muted =
            crate::video::current_map_muted(&window.state::<crate::video::VideoState>());
        let service = window
            .state::<crate::dashboard_service::DashboardService>()
            .runtime(window.app_handle())?;
        let mut options = match self.launch.as_deref() {
            Some(launch) => launch.clone()?,
            None => LaunchOptions::parse(std::env::args_os().skip(1).filter(|arg| arg == "--allow-unverified-content"))
                .map_err(|error| error.to_string())?,
        };
        let parent = match &self.shared.lock().map_err(|e| e.to_string())?.popout {
            Some(window) => window.parent,
            None => own.unwrap_or_else(|| Err("Couldn’t find the map’s window.".into()))?,
        };
        let (made, created) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                let _ = made.send(viewer::host::create_window(parent).map_err(|error| error.to_string()));
            })
            .map_err(|error| error.to_string())?;
        let (map_window, inputs) = created
            .recv()
            .map_err(|_| "Couldn’t create the map’s window.".to_owned())?
            .map_err(|error| format!("Couldn’t create the map’s window: {error}"))?;
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        if inner.shutting_down {
            map_window.destroy();
            return Err("BattlePlan is closing.".into());
        }
        if inner.popout.is_none() {
            let app = window.app_handle().clone();
            map_window.on_press(move || { let _ = app.emit("map-pressed", ()); });
        }
        if let Some(config) = inner.folders.as_ref().or(saved_folders.as_ref()) {
            options.apply_folders(config);
        }
        let diagnostics = inner.diagnostics.unwrap_or(saved_diagnostics);
        let muted = inner.muted.unwrap_or(saved_muted);
        if inner.generation > 0 {
            inner.restore_commands();
        }
        inner.generation += 1;
        let generation = inner.generation;
        inner.status.running = true;
        inner.status.ready = false;
        inner.status.error = None;
        inner.status.camera_mode = Default::default();
        inner.status.camera_overlays = false;
        inner.pending.gamepad = None;
        inner.status.fly_aircraft = None;
        inner.last_diagnostic = None;
        if inner.active_dashboard {
            inner.pending.maps = Some(inner.dashboard_snapshot());
        } else {
            inner.pending.bounds = inner.bounds;
            inner.pending.visibility = Some(inner.status.visible);
        }
        let display = super::launch_display(display, &inner.display, since);
        inner.pending.actions.extend(display);
        inner
            .pending
            .actions
            .push_back(dashboard_core::protocol::MapAction::Diagnostics {
                enabled: diagnostics,
            });
        inner
            .pending
            .actions
            .push_back(dashboard_core::protocol::MapAction::Mute { muted });
        let role = if inner.popout.is_some() { Role::PopOut } else { Role::Main };
        let receiver = self.clone();
        let app = window.app_handle().clone();
        let events: viewer::host::Events = Arc::new(move |event| receiver.receive(event, generation, &app));
        let ending = self.clone();
        let ended_window = window.clone();
        let closing = map_window.clone();
        let map = Map::start(Launch { options, window: map_window, inputs, events, role, service }, move |end| {
            ending.ended(end, generation, &ended_window)
        })
        .map_err(|error| {
            closing.destroy();
            format!("Couldn’t start the map: {error}")
        })?;
        if inner.popout_open {
            map.send(viewer::host::Command::PopoutOpen(true));
        }
        inner.map = Some(map);
        inner.flush();
        Ok(())
    }

    fn ended(&self, ending: Ending, generation: u64, window: &WebviewWindow) {
        use tauri::Manager;
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation {
            return;
        }
        let restart = inner.map_ended(ending);
        if !restart {
            inner.publish_stopped(window.app_handle());
        }
        let waiting = inner.take_ended_waiters();
        drop(inner);
        waiting.into_iter().for_each(|after| after());
        if restart && let Err(error) = self.ensure_started(window) {
            let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
            inner.status.error = Some(error);
            inner.publish_stopped(window.app_handle());
        }
    }

    fn receive(&self, event: NativeEvent, generation: u64, app: &AppHandle) {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        let Some(event) = inner.hear(event, generation) else {
            return;
        };
        match event {
            NativeEvent::MapDetails { details } => inner.status.map_details = Some(details),
            NativeEvent::FlyAircraft { aircraft, state } => {
                inner.status.fly_aircraft =
                    Some(dashboard_core::protocol::FlyAircraftStatus { aircraft, state });
            }
            NativeEvent::ToggleDiagnostics => {
                drop(inner);
                if let Err(failed) = crate::video::change_map_diagnostics(app, None) {
                    crate::video::diagnostics_failed(app, None, &failed);
                }
                return;
            }
            NativeEvent::MapOptions { view_id, options } => {
                drop(inner);
                let _ = app.emit(
                    "dashboard-map-options",
                    serde_json::json!({ "viewId": view_id, "options": options }),
                );
                return;
            }
            NativeEvent::MapModes { view_id, modes } => {
                drop(inner);
                let _ = app.emit(
                    "dashboard-map-modes",
                    serde_json::json!({ "viewId": view_id, "modes": modes }),
                );
                return;
            }
            NativeEvent::MapToggle { view_id, toggle } => {
                drop(inner);
                let _ = app.emit(
                    "dashboard-map-toggle",
                    serde_json::json!({ "viewId": view_id, "toggle": toggle }),
                );
                return;
            }
            NativeEvent::ObserveCountry { view_id, country } => {
                drop(inner);
                let _ = app.emit(
                    "dashboard-map-observe",
                    serde_json::json!({ "viewId": view_id, "country": country }),
                );
                return;
            }
            NativeEvent::MapStates { states } => {
                let states = inner.filter_map_states(states);
                if !states.is_empty() {
                    let _ = app.emit("dashboard-map-state", states);
                }
                return;
            }
            NativeEvent::Ready { sample_counts, .. } => {
                inner.status.ready = true;
                if let Some(counts) = sample_counts {
                    dashboard_core::map_settings::set_sample_counts(counts);
                }
            }
            NativeEvent::CameraMode { mode, overlays } => {
                inner.status.camera_mode = mode;
                inner.status.camera_overlays = overlays;
            }
            NativeEvent::State {
                path,
                branch,
                hour,
                revision,
                drawn,
                open_id,
                error,
            } => {
                inner.note_viewer_state(ViewerState { path, branch, hour, revision, drawn }, open_id);
                inner.status.error = error;
            }
            NativeEvent::Error { message } => {
                inner.last_diagnostic = Some(message.clone());
                inner.status.error = Some(message);
            }
        }
        inner.publish(app);
    }

    pub fn shutdown(&self, after: impl FnOnce() + Send + 'static) {
        let mut inner = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        if !inner.shutting_down {
            inner.shutting_down = true;
            inner.pending.close = true;
            inner.flush();
        }
        inner.ended_waiters.push(Box::new(after));
        let waiting = inner.take_ended_waiters();
        drop(inner);
        waiting.into_iter().for_each(|after| after());
    }
}

#[cfg(test)]
mod tests {
    use super::Launched;
    use crate::native::{HostStatus, Inner, NativeHost};
    use dashboard_core::protocol::MapAction;
    use viewer::host::Ending;

    #[test]
    fn unloading_a_recording_during_viewer_startup_reaches_the_viewer() {
        let host = NativeHost::default();
        assert!(host.shared.lock().unwrap().begin_launch().unwrap());
        assert!(host.map_action(MapAction::Inspect).is_err());
        host.map_action(MapAction::Unload { path: "launch.battleplan".into() }).unwrap();
        let mut inner = host.shared.lock().unwrap();
        assert!(matches!(inner.pending.actions.pop_front(), Some(MapAction::Unload { path }) if path == "launch.battleplan"));
    }

    fn showing(path: &str) -> Inner {
        Inner {
            generation: 3,
            status: HostStatus {
                running: true,
                ready: true,
                viewer: dashboard_core::protocol::ViewerState {
                    path: Some(path.into()),
                    revision: Some("v1".into()),
                    branch: Some("1".into()),
                    hour: Some(12),
                    drawn: None,
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn a_lost_device_starts_a_new_map_at_the_hour_the_last_showed() {
        let mut inner = showing("A");
        assert!(inner.map_ended(Ending::DeviceLost("The map lost its graphics device: Removed".into())));
        assert_eq!(inner.generation, 4);
        assert!(!inner.status.running && !inner.status.ready && inner.status.error.is_none());
        inner.restore_commands();
        assert_eq!(inner.pending.open, Some(("A".into(), Some("v1".into()), None)));
        assert_eq!(inner.pending.seek.as_ref().map(|seek| seek.hour), Some(12));
        assert!(!inner.map_ended(Ending::DeviceLost("The map lost its graphics device: Hung".into())));
        assert_eq!(
            inner.status.error.as_deref(),
            Some("The map lost its graphics device again before it was ready, so it wasn’t started again: The map lost its graphics device: Hung: The map lost its graphics device: Removed")
        );
    }

    #[test]
    fn a_map_that_stopped_is_shown_by_name_and_reopens_where_it_was() {
        let mut inner = showing("A");
        assert!(!inner.map_ended(Ending::Failed("The map stopped: the picture’s layer list is empty".into())));
        assert_eq!(
            inner.status.error.as_deref(),
            Some("The map stopped: the picture’s layer list is empty")
        );
        assert!(!inner.status.running);
        inner.restore_commands();
        assert_eq!(inner.pending.open, Some(("A".into(), Some("v1".into()), None)));
        let mut closing = showing("A");
        closing.shutting_down = true;
        assert!(!closing.map_ended(Ending::Failed("late".into())));
        assert!(closing.status.error.is_none());
    }

    #[test]
    fn a_map_lost_before_its_launch_finished_is_started_again_once_it_finishes() {
        let mut inner = Inner::default();
        assert_eq!(inner.begin_launch(), Ok(true));
        inner.status.ready = true;
        assert!(inner.map_ended(Ending::DeviceLost("The map lost its graphics device: Removed".into())));
        assert_eq!(inner.begin_launch(), Ok(false), "a launch is still finishing");
        assert_eq!(inner.launch_finished(Ok(())), Launched::Again);
        assert!(inner.starting && inner.status.running && inner.status.error.is_none());
        assert_eq!(inner.launch_finished(Ok(())), Launched::Running);
        assert!(!inner.starting);
    }
}
