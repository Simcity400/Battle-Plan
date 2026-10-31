use super::{NativeHost, job::Job, line_reader::read_lines};
use dashboard::protocol::{
    DashboardMapState, EVENT_PREFIX, NativeCommand, NativeEvent, ViewerState,
};
use std::{
    io::Write,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, WebviewWindow};

/// The folders a new viewer starts with: the last ones Apply sent, which may be
/// newer than the preferences read before taking the host lock.
fn launch_folders(
    applied: Option<&dashboard::Config>,
    saved: Option<&dashboard::Config>,
) -> Vec<std::ffi::OsString> {
    applied.or(saved).map(folder_arguments).unwrap_or_default()
}

/// The effective folders as the viewer's folder options; the last occurrence
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

pub(super) struct Process {
    child: Child,
    _job: Job,
}

impl super::Inner {
    /// Tells the frontend a viewer stopped or could not start: its status, and each dashboard map
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
}

impl NativeHost {
    /// Starts a viewer unless one runs or is starting, on a thread of its own: creating its
    /// process takes a third of a second, which the caller, the main thread for a command, does
    /// not wait for. Meanwhile it reports running and not ready; a start that fails reports why.
    pub(crate) fn ensure_started(&self, window: &WebviewWindow) -> Result<(), String> {
        use tauri::Manager;
        let popout = {
            let mut inner = self.shared.0.lock().map_err(|e| e.to_string())?;
            if inner.process.is_some() || inner.starting {
                return Ok(());
            }
            if inner.shutting_down {
                return Err("The application is closing".into());
            }
            inner.starting = true;
            inner.status.running = true;
            inner.status.ready = false;
            inner.status.error = None;
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
        thread::spawn(move || {
            let started = host.start(&window, own);
            let mut inner = host.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            inner.starting = false;
            if let Err(error) = started
                && inner.process.is_none()
            {
                inner.status.running = false;
                inner.status.ready = false;
                if !inner.shutting_down {
                    inner.status.error = Some(error);
                }
                inner.publish_stopped(window.app_handle());
            } else {
                inner.publish(window.app_handle());
            }
            drop(inner);
            host.shared.1.notify_all();
        });
        Ok(())
    }

    fn start(
        &self,
        window: &WebviewWindow,
        own: Option<Result<usize, String>>,
    ) -> Result<(), String> {
        use std::os::windows::process::CommandExt;
        use tauri::Manager;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

        // Preference resolution itself reads host status, so no lock is held across it.
        let since = self
            .shared
            .0
            .lock()
            .map_err(|e| e.to_string())?
            .display_sent;
        let display = crate::video::launch_actions(window)?;
        // The host owns the content folders; the viewer reads none of its own.
        // Unreadable preferences start it without folders, and Settings shows why.
        let saved_folders = crate::dashboard_service::host_config()
            .ok()
            .map(|(config, _)| config);
        let saved_diagnostics =
            crate::video::current_map_diagnostics(&window.state::<crate::video::VideoState>());
        let saved_muted =
            crate::video::current_map_muted(&window.state::<crate::video::VideoState>());
        let inner = self.shared.0.lock().map_err(|e| e.to_string())?;
        // A change saved after the reads above has already reached `set_folders`, which keeps
        // it under this lock; one saved while the process is created is sent once it runs.
        let applied = inner.folders.clone();
        let folders = launch_folders(applied.as_ref(), saved_folders.as_ref());
        if inner.shutting_down {
            return Err("The application is closing".into());
        }
        let executable = std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("viewer.exe");
        if !executable.is_file() {
            return Err(format!(
                "Native viewer is missing at {}. Run npm run dev or npm run build:desktop to build both executables.",
                executable.display()
            ));
        }
        let parent = match &inner.popout {
            Some(window) => window.parent,
            None => own.unwrap_or_else(|| Err("The map's window has no handle".into()))?,
        };
        // A pop-out shows only what the app sends it; the app's own command line (a recording,
        // an hour, a camera) belongs to the main viewer.
        let forwarded: Vec<std::ffi::OsString> = if inner.popout.is_some() {
            std::env::args_os()
                .skip(1)
                .filter(|argument| argument == "--allow-unverified-content")
                .collect()
        } else {
            std::env::args_os().skip(1).collect()
        };
        drop(inner);
        let mut child = Command::new(&executable)
            .args(forwarded)
            .args(folders)
            .env("BATTLE_PLAN_VIEWER_PARENT", parent.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", executable.display()))?;
        let job = match Job::attach(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let input = child
            .stdin
            .take()
            .ok_or("Native viewer input pipe is missing")?;
        let output = child
            .stdout
            .take()
            .ok_or("Native viewer output pipe is missing")?;
        let errors = child
            .stderr
            .take()
            .ok_or("Native viewer error pipe is missing")?;
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.shutting_down {
            let _ = child.kill();
            let _ = child.wait();
            return Err("The application is closing".into());
        }
        // Choices saved while the process was created, which `set_diagnostics`, `set_muted`
        // and `set_folders` kept without sending.
        let diagnostics = inner.diagnostics.unwrap_or(saved_diagnostics);
        let muted = inner.muted.unwrap_or(saved_muted);
        let moved = inner
            .folders
            .clone()
            .filter(|config| applied.as_ref() != Some(config));
        if inner.generation > 0 {
            inner.restore_commands();
        }
        inner.generation += 1;
        let generation = inner.generation;
        inner.status.running = true;
        inner.status.ready = false;
        inner.status.error = None;
        // A new viewer starts at the game's camera with its overlays hidden, holding no controller
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
        if let Some(config) = moved {
            inner
                .pending
                .actions
                .push_back(dashboard::protocol::MapAction::Folders {
                    game: config.game_dir.to_string_lossy().into_owned(),
                    user: config.user_dir.to_string_lossy().into_owned(),
                });
        }
        inner.process = Some(Process { child, _job: job });
        drop(inner);

        let writer = self.clone();
        let writer_app = window.app_handle().clone();
        thread::spawn(move || writer.write_commands(input, generation, &writer_app));
        let reader = self.clone();
        let reader_app = window.app_handle().clone();
        thread::spawn(move || {
            read_lines(output, |line| {
                if let Some(json) = line.strip_prefix(EVENT_PREFIX)
                    && let Ok(event) = serde_json::from_str::<NativeEvent>(json)
                {
                    reader.receive(event, generation, &reader_app);
                }
            });
        });
        let diagnostics = self.clone();
        thread::spawn(move || {
            read_lines(errors, |line| {
                let mut inner = diagnostics
                    .shared
                    .0
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                if inner.generation == generation && !line.trim().is_empty() {
                    inner.last_diagnostic = Some(line.chars().take(2000).collect());
                }
            });
        });
        let monitor = self.clone();
        let monitor_app = window.app_handle().clone();
        thread::spawn(move || monitor.monitor(generation, &monitor_app));
        Ok(())
    }

    fn write_commands(&self, mut input: impl Write, generation: u64, app: &AppHandle) {
        loop {
            let commands = {
                let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
                while !inner.pending.can_send(inner.status.visible)
                    && inner.generation == generation
                    && inner.process.is_some()
                {
                    inner = self.shared.1.wait(inner).unwrap_or_else(|e| e.into_inner());
                }
                if inner.generation != generation || inner.process.is_none() {
                    return;
                }
                let visible = inner.status.visible;
                inner.pending.take_commands(visible)
            };
            for command in commands {
                let close = matches!(command, NativeCommand::Close);
                let result = serde_json::to_writer(&mut input, &command)
                    .map_err(std::io::Error::other)
                    .and_then(|()| input.write_all(b"\n"))
                    .and_then(|()| input.flush());
                if let Err(error) = result {
                    self.receive(
                        NativeEvent::Error {
                            message: format!("Native viewer connection closed: {error}"),
                        },
                        generation,
                        app,
                    );
                    if !self.is_popout() {
                        crate::dashboard_service::disconnected(
                            app,
                            "The native data connection closed",
                        );
                    }
                    return;
                }
                if close {
                    return;
                }
            }
        }
    }

    fn receive(&self, event: NativeEvent, generation: u64, app: &AppHandle) {
        let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        if inner.generation != generation || inner.shutting_down || inner.process.is_none() {
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
            // A pop-out's viewer serves no data requests.
            NativeEvent::Data { .. } if inner.popout.is_some() => return,
            NativeEvent::Data { response } => {
                drop(inner);
                crate::dashboard_service::receive_data(app, response);
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
                // the renderer that draws it, not a device of this process.
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
        // Publish while holding the state lock so reader and exit notifications
        // reach the frontend in mutation order.
        inner.publish(app);
    }

    fn monitor(&self, generation: u64, app: &AppHandle) {
        loop {
            thread::sleep(Duration::from_millis(250));
            {
                let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
                if inner.generation != generation {
                    return;
                }
                let Some(process) = inner.process.as_mut() else {
                    return;
                };
                match process.child.try_wait() {
                    Ok(None) => continue,
                    result => {
                        inner.process = None;
                        inner.generation += 1;
                        inner.status.running = false;
                        inner.status.ready = false;
                        inner.status.fly_aircraft = None;
                        inner.pending.data.clear();
                        if !inner.shutting_down {
                            let reason = match result {
                                Ok(Some(exit)) => format!("Native viewer exited ({exit})"),
                                Err(error) => format!("Cannot observe native viewer: {error}"),
                                Ok(None) => unreachable!(),
                            };
                            inner.status.error = Some(match inner.last_diagnostic.as_ref() {
                                Some(detail) => format!("{reason}: {detail}"),
                                None => reason,
                            });
                        }
                        inner.publish_stopped(app);
                    }
                }
            };
            if !self.is_popout() {
                crate::dashboard_service::disconnected(
                    app,
                    "The native data runtime exited. Reopen the recording to restore its sources.",
                );
            }
            self.shared.1.notify_all();
            return;
        }
    }

    fn is_popout(&self) -> bool {
        self.shared
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .popout
            .is_some()
    }

    pub fn shutdown(&self) {
        {
            let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            if inner.shutting_down {
                return;
            }
            inner.shutting_down = true;
            inner.pending.close = true;
        }
        self.shared.1.notify_all();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let mut inner = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            let Some(process) = inner.process.as_mut() else {
                // A start under way closes the process it created once it sees the shutdown.
                if inner.starting && Instant::now() < deadline {
                    drop(inner);
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                break;
            };
            if process.child.try_wait().ok().flatten().is_some() {
                inner.process = None;
                break;
            }
            if Instant::now() >= deadline {
                let _ = process.child.kill();
                let _ = process.child.wait();
                inner.process = None;
                break;
            }
            drop(inner);
            thread::sleep(Duration::from_millis(20));
        }
        self.shared.1.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{folder_arguments, launch_folders};

    #[test]
    fn the_viewer_receives_only_the_folders_the_host_has() {
        let config = |game: &str, user: &str| dashboard::Config {
            game_dir: game.into(),
            user_dir: user.into(),
            background_preparation: Some(false),
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
            background_preparation: None,
        };
        let (applied, saved) = (config(r"C:\New"), config(r"C:\Old"));
        assert_eq!(
            launch_folders(Some(&applied), Some(&saved)),
            folder_arguments(&applied)
        );
        assert_eq!(launch_folders(None, Some(&saved)), folder_arguments(&saved));
        assert!(launch_folders(None, None).is_empty());
    }
}
