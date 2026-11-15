use super::NativeHost;
use dashboard::protocol::{DashboardMapState, NativeEvent, ViewerState};
use std::{sync::Arc, thread};
use tauri::{AppHandle, Emitter, WebviewWindow};
use viewer::host::{Ending, Launch, Map, Role};

/// The folders a new map starts with: the last ones Apply sent, which may be
/// newer than the preferences read before taking the host lock.
fn launch_folders(
    applied: Option<&dashboard::Config>,
    saved: Option<&dashboard::Config>,
) -> Vec<std::ffi::OsString> {
    applied.or(saved).map(folder_arguments).unwrap_or_default()
}

/// The effective folders as the map's folder options; the last occurrence
/// of an option wins over one forwarded from the host's own command line.
fn folder_arguments(config: &dashboard::Config) -> Vec<std::ffi::OsString> {
    [
        ("--game-dir", &config.game_dir),
        ("--user-dir", &config.user_dir),
    ]
    .into_iter()
    .filter(|(_, folder)| !folder.as_os_str().is_empty())
    .flat_map(|(option, folder)| [std::ffi::OsString::from(option), folder.into()])
    .collect()
}

impl super::Inner {
    /// Tells the frontend a map stopped or could not start: its status, and each dashboard map
    /// its views wait on, with why.
    fn publish_stopped(&self, app: &AppHandle) {
        self.publish(app);
        if let Some(views) = &self.dashboard_views {
            let states: Vec<_> = views
                .iter()
                .map(|view| DashboardMapState {
                    id: view.id.clone(),
                    requested_sequence: view.sequence,
                    error: self.status.error.clone(),
                    ..Default::default()
                })
                .collect();
            let _ = app.emit("dashboard-map-state", states);
        }
    }

    /// The map of this generation ended: whether to start a new one. A lost device is recovered
    /// by a new map, which makes a new device and surface and is sent what the last one showed,
    /// unless the device was lost again before the new map was ready. Any other end is shown by
    /// name; the dashboard's data does not depend on the map and keeps working.
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
                format!("{reason}, again before the map was ready, so it was not started again")
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

/// How a launch finished.
#[derive(Debug, PartialEq, Eq)]
enum Launched {
    /// The map it installed ended meanwhile and asked for the next: it launches again.
    Again,
    Running,
    /// It could not start the map, and says why.
    Stopped,
}

impl super::Inner {
    /// Whether to launch a map: not while one runs, and while one is launching only once that
    /// launch finishes, for its map may already have ended.
    fn begin_launch(&mut self) -> Result<bool, String> {
        if self.map.is_some() {
            return Ok(false);
        }
        if self.starting {
            self.relaunch = !self.shutting_down;
            return Ok(false);
        }
        if self.shutting_down {
            return Err("The application is closing".into());
        }
        self.starting = true;
        self.status.running = true;
        self.status.ready = false;
        self.status.error = None;
        Ok(true)
    }

    /// A launch [`begin_launch`](Self::begin_launch) began finished, as `started`.
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

    /// What waits for the map to end, once none runs or starts.
    fn take_ended_waiters(&mut self) -> Vec<Box<dyn FnOnce() + Send>> {
        if self.map.is_some() || self.starting {
            return Vec::new();
        }
        std::mem::take(&mut self.ended_waiters)
    }
}

impl NativeHost {
    /// Starts a map unless one runs or is starting, on a thread of its own: reading the display
    /// preferences takes the window's getters, which the caller, the main thread for a command,
    /// must not wait on. Meanwhile it reports running and not ready; a start that fails says why.
    pub(crate) fn ensure_started(&self, window: &WebviewWindow) -> Result<(), String> {
        use tauri::Manager;
        let popout = {
            let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
            if !inner.begin_launch()? {
                return Ok(());
            }
            inner.popout.is_some()
        };
        // Read here: off the main thread a window getter waits on it, which may wait on this lock.
        // A pop-out's parent is its own window, already known.
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

        // Preference resolution itself reads host status, so no lock is held across it.
        let since = self
            .shared
            .lock()
            .map_err(|e| e.to_string())?
            .display_sent;
        let display = crate::video::launch_actions(window)?;
        // The host owns the content folders; the map reads none of its own.
        // Unreadable preferences start it without folders, and Settings shows why.
        let saved_folders = crate::dashboard_service::host_config()
            .ok()
            .map(|(config, _)| config);
        let saved_diagnostics =
            crate::video::current_map_diagnostics(&window.state::<crate::video::VideoState>());
        let saved_muted =
            crate::video::current_map_muted(&window.state::<crate::video::VideoState>());
        let parent = match &self.shared.lock().map_err(|e| e.to_string())?.popout {
            Some(window) => window.parent,
            None => own.unwrap_or_else(|| Err("The map's window has no handle".into()))?,
        };
        // The map's own window is a child of the host's, made and pumped by the app's thread; no
        // lock is held while that thread makes it.
        let (made, created) = std::sync::mpsc::sync_channel(1);
        window
            .run_on_main_thread(move || {
                let _ = made.send(viewer::host::create_window(parent).map_err(|error| error.to_string()));
            })
            .map_err(|error| error.to_string())?;
        let (map_window, inputs) = created
            .recv()
            .map_err(|_| "The app's thread made no map window".to_owned())?
            .map_err(|error| format!("The map's window: {error}"))?;
        let mut inner = self.shared.lock().map_err(|e| e.to_string())?;
        if inner.shutting_down {
            map_window.destroy();
            return Err("The application is closing".into());
        }
        // A pop-out shows only what the app sends it; the app's own command line (a recording,
        // an hour, a camera) belongs to the main map.
        let mut args: Vec<std::ffi::OsString> = if inner.popout.is_some() {
            std::env::args_os()
                .skip(1)
                .filter(|argument| argument == "--allow-unverified-content")
                .collect()
        } else {
            std::env::args_os().skip(1).collect()
        };
        args.extend(launch_folders(inner.folders.as_ref(), saved_folders.as_ref()));
        // Choices saved while the preferences were read, which `set_diagnostics`, `set_muted`
        // and `set_folders` kept.
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
        // A new map starts at the game's camera with its overlays hidden, holding no controller
        // until the host hands it over; nothing queued for the last one reaches it.
        inner.status.camera_mode = Default::default();
        inner.status.camera_overlays = false;
        inner.pending.gamepad = None;
        inner.status.fly_aircraft = None;
        inner.last_diagnostic = None;
        if inner.active_dashboard {
            let views = inner.dashboard_views.clone().unwrap_or_default();
            inner.pending.maps = Some(views);
        } else {
            inner.pending.bounds = inner.bounds;
            inner.pending.visibility = Some(inner.status.visible);
        }
        // A display choice sent since the read above began has already reached `set_display`.
        let display = super::launch_display(display, &inner.display, since);
        inner.pending.actions.extend(display);
        inner
            .pending
            .actions
            .push_back(dashboard::protocol::MapAction::Diagnostics {
                enabled: diagnostics,
            });
        inner
            .pending
            .actions
            .push_back(dashboard::protocol::MapAction::Mute { muted });
        let role = if inner.popout.is_some() { Role::PopOut } else { Role::Main };
        let receiver = self.clone();
        let app = window.app_handle().clone();
        let events: viewer::host::Events = Arc::new(move |event| receiver.receive(event, generation, &app));
        let ending = self.clone();
        let ended_window = window.clone();
        // The map's events wait on this lock until its map is installed.
        let closing = map_window.clone();
        let map = Map::start(Launch { args, window: map_window, inputs, events, role }, move |end| {
            ending.ended(end, generation, &ended_window)
        })
        .map_err(|error| {
            closing.destroy();
            format!("Could not start the map: {error}")
        })?;
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
        if inner.generation != generation || inner.shutting_down || inner.map.is_none() {
            return;
        }
        match event {
            NativeEvent::MapDetails { details } => inner.status.map_details = Some(details),
            NativeEvent::FlyAircraft { aircraft, state } => {
                inner.status.fly_aircraft =
                    Some(dashboard::protocol::FlyAircraftStatus { aircraft, state });
            }
            NativeEvent::ToggleDiagnostics => {
                // The shortcut pressed in the native map; the host owns the choice.
                drop(inner);
                if let Err(error) = crate::video::change_map_diagnostics(app, None) {
                    let _ = app.emit("map-diagnostics-error", error);
                }
                return;
            }
            NativeEvent::MapOptions { view_id, options } => {
                // The picker changed a view's options; the web interface stores them per view so
                // its next layout carries them back.
                drop(inner);
                let _ = app.emit(
                    "dashboard-map-options",
                    serde_json::json!({ "viewId": view_id, "options": options }),
                );
                return;
            }
            NativeEvent::MapModes { view_id, modes } => {
                // The web interface's own map mode control offers what the view's picker does.
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
            NativeEvent::MapStates { mut states } => {
                let Some(requested) = inner.dashboard_views.as_ref() else {
                    return;
                };
                states.retain_mut(|state| {
                    let Some(view) = requested.iter().find(|view| view.id == state.id) else {
                        return false;
                    };
                    if state
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
                let _ = app.emit("dashboard-map-state", states);
                return;
            }
            NativeEvent::Ready { sample_counts, .. } => {
                inner.status.ready = true;
                // The host's own data service advertises and checks map anti-aliasing against
                // the renderer that draws it.
                if let Some(counts) = sample_counts {
                    dashboard::map_settings::set_sample_counts(counts);
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
                error,
            } => {
                if inner.requested_path.as_ref() == path.as_ref()
                    && inner.requested_revision.as_ref() == revision.as_ref()
                {
                    inner.requested_path = None;
                    inner.requested_revision = None;
                }
                inner.status.viewer = ViewerState {
                    path,
                    branch,
                    hour,
                    revision,
                };
                if inner.requested_path.is_none()
                    && inner
                        .requested_seek
                        .as_ref()
                        .is_some_and(|target| target.matches(&inner.status.viewer))
                {
                    inner.requested_seek = None;
                }
                inner.status.error = error;
            }
            NativeEvent::Error { message } => {
                inner.last_diagnostic = Some(message.clone());
                inner.status.error = Some(message);
            }
        }
        // Publish while holding the state lock so events and ends reach the frontend in
        // mutation order.
        inner.publish(app);
    }

    /// Closes the map for good. `after` runs once no map runs or starts, so once the map has let
    /// go of the surface on its window, which its parent window may then take with it: at once
    /// when none runs, else on the thread that saw the map end. Its last state is kept.
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
    use super::{Launched, folder_arguments, launch_folders};
    use crate::native::{HostStatus, Inner};
    use viewer::host::Ending;

    #[test]
    fn the_viewer_receives_only_the_folders_the_host_has() {
        let config = |game: &str, user: &str| dashboard::Config {
            game_dir: game.into(),
            user_dir: user.into(),
            older_game_dirs: Vec::new(),
        };
        assert_eq!(
            folder_arguments(&config(r"C:\Game", r"C:\User")),
            ["--game-dir", r"C:\Game", "--user-dir", r"C:\User"]
        );
        assert_eq!(
            folder_arguments(&config("", r"C:\User")),
            ["--user-dir", r"C:\User"]
        );
        assert!(folder_arguments(&config("", "")).is_empty());
    }

    /// Apply can save between the launch's preference read and the host lock;
    /// the folders it sent win over that older read.
    #[test]
    fn a_new_viewer_starts_with_the_folders_apply_last_sent() {
        let config = |game: &str| dashboard::Config {
            game_dir: game.into(),
            user_dir: r"C:\User".into(),
            older_game_dirs: Vec::new(),
        };
        let (applied, saved) = (config(r"C:\New"), config(r"C:\Old"));
        assert_eq!(
            launch_folders(Some(&applied), Some(&saved)),
            folder_arguments(&applied)
        );
        assert_eq!(launch_folders(None, Some(&saved)), folder_arguments(&saved));
        assert!(launch_folders(None, None).is_empty());
    }

    fn showing(path: &str) -> Inner {
        Inner {
            generation: 3,
            status: HostStatus {
                running: true,
                ready: true,
                viewer: dashboard::protocol::ViewerState {
                    path: Some(path.into()),
                    revision: Some("v1".into()),
                    branch: Some("1".into()),
                    hour: Some(12),
                },
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// A ready map that lost its device is started again, and the new one is sent the recording
    /// and hour the last one showed; one that loses it again before it is ready is not.
    #[test]
    fn a_lost_device_starts_a_new_map_at_the_hour_the_last_showed() {
        let mut inner = showing("A");
        assert!(inner.map_ended(Ending::DeviceLost("The map's GPU device was lost (Unknown: removed)".into())));
        assert_eq!(inner.generation, 4);
        assert!(!inner.status.running && !inner.status.ready && inner.status.error.is_none());
        inner.restore_commands();
        assert_eq!(inner.pending.open, Some(("A".into(), "v1".into())));
        assert_eq!(inner.pending.seek.as_ref().map(|seek| seek.hour), Some(12));
        // The new map lost it again before it was ready.
        assert!(!inner.map_ended(Ending::DeviceLost("The map's GPU device was lost (Unknown: hung)".into())));
        assert_eq!(
            inner.status.error.as_deref(),
            Some("The map's GPU device was lost (Unknown: hung), again before the map was ready, so it was not started again: The map's GPU device was lost (Unknown: removed)")
        );
    }

    /// A map that panicked is not started again: its status names why, and the next start (the
    /// frontend reopening the map) is sent what it showed.
    #[test]
    fn a_map_that_stopped_is_shown_by_name_and_reopens_where_it_was() {
        let mut inner = showing("A");
        assert!(!inner.map_ended(Ending::Failed("The map stopped: the picture's layer list is empty".into())));
        assert_eq!(
            inner.status.error.as_deref(),
            Some("The map stopped: the picture's layer list is empty")
        );
        assert!(!inner.status.running);
        inner.restore_commands();
        assert_eq!(inner.pending.open, Some(("A".into(), "v1".into())));
        // Closing the app is no failure.
        let mut closing = showing("A");
        closing.shutting_down = true;
        assert!(!closing.map_ended(Ending::Failed("late".into())));
        assert!(closing.status.error.is_none());
    }

    /// A map that is ready and loses its device before its launch has finished is started again
    /// once that launch finishes: the restart it asked for while the launch was finishing is kept.
    #[test]
    fn a_map_lost_before_its_launch_finished_is_started_again_once_it_finishes() {
        let mut inner = Inner::default();
        assert_eq!(inner.begin_launch(), Ok(true));
        // The launch installed its map, which became ready and lost its device before the
        // launch finished.
        inner.status.ready = true;
        assert!(inner.map_ended(Ending::DeviceLost("The map's GPU device was lost (Unknown: removed)".into())));
        assert_eq!(inner.begin_launch(), Ok(false), "a launch is still finishing");
        assert_eq!(inner.launch_finished(Ok(())), Launched::Again);
        assert!(inner.starting && inner.status.running && inner.status.error.is_none());
        assert_eq!(inner.launch_finished(Ok(())), Launched::Running);
        assert!(!inner.starting);
    }
}
