mod lifecycle;
mod shell;

use crate::native::{HostStatus, NativeHost};
use dashboard_core::protocol::{MapAction, MapPlayback, ViewBounds};
use lifecycle::{Effect, Input, Lifecycle};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};
use tauri::{AppHandle, Emitter, Manager, State};

pub const VIEW_ID: &str = "popout";
const STATE_EVENT: &str = "popout-state";
const ERROR_EVENT: &str = "popout-error";

struct PopOut {
    shell: shell::Shell,
    host: NativeHost,
}

#[derive(Default)]
struct Shared {
    current: Mutex<Option<(u64, PopOut)>>,
    published: AtomicU64,
}

type Current = (u64, NativeHost);

impl Shared {
    fn with<R>(&self, read: impl FnOnce(u64, &PopOut) -> R) -> Option<R> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|(generation, popout)| read(*generation, popout))
    }

    fn current(&self) -> Option<Current> {
        self.with(|generation, popout| (generation, popout.host.clone()))
    }
}

const CLOSED_WINDOW: &str = "That pop-out map window has closed";

enum Message {
    Input(Input),
    Open {
        main: usize,
        reply: Sender<Result<(), String>>,
    },
    Close {
        generation: Option<u64>,
        done: Option<Sender<()>>,
    },
    Restart {
        generation: u64,
        reply: Sender<Result<(), String>>,
    },
    Publish,
}

#[derive(Default)]
pub struct PopOuts {
    shared: Arc<Shared>,
    owner: Mutex<Option<Sender<Message>>>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutState {
    generation: u64,
    sequence: u64,
    viewer: u64,
    open: bool,
    status: Option<HostStatus>,
    error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutStatus {
    generation: u64,
    sequence: u64,
    viewer: u64,
    status: HostStatus,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutError {
    generation: u64,
    sequence: u64,
    error: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mirror {
    Diagnostics(bool),
    Muted(bool),
    Display,
    SettingsSaved,
    RefreshVisualMods,
}

fn mirrored(action: &MapAction) -> Option<Mirror> {
    match action {
        MapAction::Diagnostics { enabled } => Some(Mirror::Diagnostics(*enabled)),
        MapAction::Mute { muted } => Some(Mirror::Muted(*muted)),
        MapAction::Vsync { .. } | MapAction::UiScale { .. } | MapAction::FrameLimit { .. } => Some(Mirror::Display),
        MapAction::MapSettingsSaved => Some(Mirror::SettingsSaved),
        MapAction::RefreshVisualMods => Some(Mirror::RefreshVisualMods),
        MapAction::Unload { .. }
        | MapAction::Configure { .. }
        | MapAction::Inspect
        | MapAction::Folders { .. }
        | MapAction::CameraMode { .. }
        | MapAction::FlyAircraft { .. } => None,
    }
}

fn path_key(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_lowercase()
}

fn shows(recordings: &[String], path: &str) -> bool {
    let wanted = path_key(path);
    recordings
        .iter()
        .any(|recording| path_key(recording) == wanted)
}

impl PopOuts {
    fn host(&self) -> Option<NativeHost> {
        self.shared.with(|_, popout| popout.host.clone())
    }

    fn send(&self, app: &AppHandle, message: Message) {
        let mut owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        let sender = owner.get_or_insert_with(|| {
            let (sender, receiver) = mpsc::channel();
            let owner = Owner {
                app: app.clone(),
                shared: Arc::clone(&self.shared),
                sender: sender.clone(),
                lifecycle: Lifecycle::default(),
                main: 0,
                opening: None,
                asking: None,
                open_waiters: Vec::new(),
                restart_waiters: Vec::new(),
                close_waiters: Vec::new(),
                errors: Default::default(),
                sequence: 0,
                viewer: 0,
                window_error: None,
            };
            let _ = std::thread::Builder::new()
                .name("popout-map-owner".into())
                .spawn(move || owner.run(receiver));
            sender
        });
        let _ = sender.send(message);
    }

    pub fn mirror(&self, action: &MapAction) {
        let (Some(mirror), Some(host)) = (mirrored(action), self.host()) else {
            return;
        };
        match mirror {
            Mirror::Diagnostics(enabled) => host.set_diagnostics(enabled),
            Mirror::Muted(muted) => host.set_muted(muted),
            Mirror::Display => host.set_display(action.clone()),
            Mirror::SettingsSaved => host.map_settings_saved(),
            Mirror::RefreshVisualMods => host.refresh_visual_mods(),
        }
    }

    pub fn set_folders(&self, config: &dashboard_core::config::Config) {
        if let Some(host) = self.host() {
            host.set_folders(config);
        }
    }

    pub fn in_front(&self, foreground: usize) -> Option<u64> {
        self.shared
            .with(|generation, popout| {
                (foreground != 0 && popout.shell.hwnd() == foreground).then_some(generation)
            })
            .flatten()
    }

    pub fn host_of(&self, generation: u64) -> Option<NativeHost> {
        self.shared
            .current()
            .and_then(|(current, host)| (current == generation).then_some(host))
    }

    pub fn current_generation(&self) -> Option<u64> {
        self.shared.current().map(|(generation, _)| generation)
    }

    pub fn unloaded(&self, path: &str, generation: u64, app: &AppHandle) {
        if let Some((current, host)) = self.shared.current()
            && current == generation
            && shows(&host.recordings(), path)
        {
            self.send(
                app,
                Message::Close {
                    generation: Some(generation),
                    done: None,
                },
            );
        }
    }

    pub fn close(&self, app: &AppHandle) {
        let (done, closed) = mpsc::channel();
        self.send(
            app,
            Message::Close {
                generation: None,
                done: Some(done),
            },
        );
        let _ = closed.recv_timeout(std::time::Duration::from_secs(10));
    }
}

struct Owner {
    app: AppHandle,
    shared: Arc<Shared>,
    sender: Sender<Message>,
    lifecycle: Lifecycle,
    main: usize,
    opening: Option<(u64, PopOut)>,
    asking: Option<Sender<Result<(), String>>>,
    open_waiters: Vec<(u64, Sender<Result<(), String>>)>,
    restart_waiters: Vec<Sender<Result<(), String>>>,
    close_waiters: Vec<Sender<()>>,
    errors: std::collections::BTreeMap<u64, String>,
    sequence: u64,
    viewer: u64,
    window_error: Option<(u64, String)>,
}

impl Owner {
    fn run(mut self, receiver: Receiver<Message>) {
        for message in receiver {
            match message {
                Message::Input(input) => self.feed(input),
                Message::Open { main, reply } => {
                    self.main = main;
                    self.asking = Some(reply);
                    self.feed(Input::Open);
                    if let Some(reply) = self.asking.take() {
                        let _ = reply.send(Err("The pop-out map couldn’t open".into()));
                    }
                }
                Message::Close { generation, done } => {
                    self.close_waiters.extend(done);
                    self.feed(Input::Close { generation });
                }
                Message::Restart { generation, reply } => {
                    self.restart_waiters.push(reply);
                    self.feed(Input::Restart(generation));
                }
                Message::Publish => {
                    let sequence = self.next_sequence();
                    let _ = self.app.emit(STATE_EVENT, self.state(sequence));
                }
            }
            if self.lifecycle.is_closed() {
                self.answer_closed();
            }
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn state(&self, sequence: u64) -> PopOutState {
        self.shared
            .with(|generation, popout| PopOutState {
                generation,
                sequence,
                viewer: self.viewer,
                open: true,
                status: Some(popout.host.status()),
                error: self
                    .window_error
                    .as_ref()
                    .filter(|(at, _)| *at == generation)
                    .map(|(_, error)| error.clone()),
            })
            .unwrap_or_else(|| PopOutState {
                generation: self.shared.published.load(Ordering::Acquire),
                sequence,
                viewer: self.viewer,
                open: false,
                status: None,
                error: None,
            })
    }

    fn feed(&mut self, input: Input) {
        for effect in self.lifecycle.handle(input) {
            self.apply(effect);
        }
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::CreateWindow(generation) => {
                let sender = self.sender.clone();
                let opened = shell::Shell::open(
                    self.main,
                    Arc::new(move |event| {
                        let input = match event {
                            shell::Event::Resized { width, height } => Input::Resized {
                                generation,
                                width,
                                height,
                            },
                            shell::Event::CloseRequested => Input::Close {
                                generation: Some(generation),
                            },
                            shell::Event::Closed => Input::Destroyed(generation),
                        };
                        let _ = sender.send(Message::Input(input));
                    }),
                );
                let ok = match opened {
                    Ok(shell) => {
                        let sender = self.sender.clone();
                        let host = NativeHost::popout(
                            shell.hwnd(),
                            Arc::new(move || {
                                let _ =
                                    sender.send(Message::Input(Input::StatusChanged(generation)));
                            }),
                        );
                        self.opening = Some((generation, PopOut { shell, host }));
                        true
                    }
                    Err(error) => {
                        self.errors.insert(generation, error);
                        false
                    }
                };
                let _ = self
                    .sender
                    .send(Message::Input(Input::Created { generation, ok }));
            }
            Effect::StartViewer {
                generation,
                restart,
            } => {
                if let Some((opening, _)) = &self.opening
                    && *opening == generation
                {
                    let popout = self.opening.take().map(|(_, popout)| popout);
                    *self
                        .shared
                        .current
                        .lock()
                        .unwrap_or_else(|e| e.into_inner()) =
                        popout.map(|popout| (generation, popout));
                }
                self.viewer += 1;
                let started = self.start(generation);
                let ok = started.is_ok();
                if let Err(error) = started {
                    self.errors.insert(generation, error);
                }
                self.feed(Input::Started {
                    generation,
                    ok,
                    restart,
                });
            }
            Effect::Resize {
                generation,
                width,
                height,
            } => {
                let resized = self
                    .shared
                    .with(|at, popout| (at == generation).then(|| popout.host.clone()))
                    .flatten()
                    .map(|host| {
                        host.set_bounds(ViewBounds {
                            x: 0,
                            y: 0,
                            width,
                            height,
                        })
                    });
                match resized {
                    Some(Err(error)) => {
                        self.window_error = Some((generation, error.clone()));
                        let sequence = self.next_sequence();
                        let _ = self.app.emit(
                            ERROR_EVENT,
                            PopOutError {
                                generation,
                                sequence,
                                error,
                            },
                        );
                    }
                    Some(Ok(()))
                        if self
                            .window_error
                            .as_ref()
                            .is_some_and(|(at, _)| *at == generation) =>
                    {
                        self.window_error = None;
                        let sequence = self.next_sequence();
                        let _ = self.app.emit(STATE_EVENT, self.state(sequence));
                    }
                    Some(Ok(())) | None => {}
                }
            }
            Effect::Raise => {
                if let Some(hwnd) = self.shared.with(|_, popout| popout.shell.hwnd()) {
                    shell::raise(hwnd);
                }
            }
            Effect::Release(generation) => {
                let current = {
                    let mut current = self
                        .shared
                        .current
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if current.as_ref().is_some_and(|(at, _)| *at == generation) {
                        current.take()
                    } else {
                        None
                    }
                };
                let opening = self.opening.take_if(|(at, _)| *at == generation);
                for (_, PopOut { shell, host }) in current.into_iter().chain(opening) {
                    shell.hide();
                    host.shutdown(move || drop(shell));
                }
                self.feed(Input::Released(generation));
            }
            Effect::Publish { generation, open } => {
                self.shared.published.store(generation, Ordering::Release);
                let sequence = self.next_sequence();
                let state = if open {
                    self.state(sequence)
                } else {
                    PopOutState {
                        generation,
                        sequence,
                        viewer: self.viewer,
                        open: false,
                        status: None,
                        error: None,
                    }
                };
                let _ = self.app.emit(STATE_EVENT, state);
            }
            Effect::PublishStatus(generation) => {
                if let Some(status) = self
                    .shared
                    .with(|at, popout| (at == generation).then(|| popout.host.status()))
                    .flatten()
                {
                    let sequence = self.next_sequence();
                    let _ = self.app.emit(
                        STATE_EVENT,
                        PopOutStatus {
                            generation,
                            sequence,
                            viewer: self.viewer,
                            status,
                        },
                    );
                }
            }
            Effect::AwaitOpen(generation) => {
                if let Some(reply) = self.asking.take() {
                    self.open_waiters.push((generation, reply));
                }
            }
            Effect::Opened(generation) => {
                for reply in self.take_waiters(generation) {
                    let _ = reply.send(Ok(()));
                }
                for reply in self.restart_waiters.drain(..) {
                    let _ = reply.send(Ok(()));
                }
            }
            Effect::OpenFailed(generation) => {
                let error = self
                    .errors
                    .remove(&generation)
                    .unwrap_or_else(|| "The pop-out map window closed while opening".into());
                for reply in self.take_waiters(generation) {
                    let _ = reply.send(Err(error.clone()));
                }
            }
            Effect::RestartFailed(generation) => {
                let error = self.errors.remove(&generation).unwrap_or_else(|| {
                    if generation == self.lifecycle.generation() {
                        "The map isn’t popped out".into()
                    } else {
                        CLOSED_WINDOW.into()
                    }
                });
                for reply in self.restart_waiters.drain(..) {
                    let _ = reply.send(Err(error.clone()));
                }
            }
        }
    }

    fn start(&self, generation: u64) -> Result<(), String> {
        let size = self
            .shared
            .with(|at, popout| (at == generation).then(|| popout.shell.client_size()))
            .flatten()
            .ok_or("The pop-out map window closed while opening")?;
        let (width, height) = size.ok_or("The pop-out map window closed while opening")?;
        self.set_view(generation, width, height)
    }

    fn set_view(&self, generation: u64, width: u32, height: u32) -> Result<(), String> {
        let host = self
            .shared
            .with(|at, popout| (at == generation).then(|| popout.host.clone()))
            .flatten()
            .ok_or("The map isn’t popped out")?;
        let window = self
            .app
            .get_webview_window("main")
            .ok_or("The app window is gone")?;
        host.set_view(
            &window,
            true,
            ViewBounds {
                x: 0,
                y: 0,
                width,
                height,
            },
        )
        .map(|_| ())
    }

    fn take_waiters(&mut self, generation: u64) -> Vec<Sender<Result<(), String>>> {
        let (answered, waiting) = std::mem::take(&mut self.open_waiters)
            .into_iter()
            .partition(|(at, _)| *at == generation);
        self.open_waiters = waiting;
        answered.into_iter().map(|(_, reply)| reply).collect()
    }

    fn answer_closed(&mut self) {
        for done in self.close_waiters.drain(..) {
            let _ = done.send(());
        }
    }
}

async fn answer<T: Send + 'static>(receiver: Receiver<Result<T, String>>) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .unwrap_or_else(|_| Err("The pop-out map stopped responding".into()))
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn popout_open(
    window: tauri::WebviewWindow,
    popouts: State<'_, PopOuts>,
) -> Result<(), String> {
    let main = window.hwnd().map_err(|e| e.to_string())?.0 as usize;
    let (reply, answered) = mpsc::channel();
    popouts.send(window.app_handle(), Message::Open { main, reply });
    answer(answered).await
}

#[tauri::command]
pub fn popout_close(app: AppHandle, popouts: State<'_, PopOuts>, generation: u64) {
    popouts.send(
        &app,
        Message::Close {
            generation: Some(generation),
            done: None,
        },
    );
}

#[tauri::command]
pub async fn popout_restart(
    app: AppHandle,
    popouts: State<'_, PopOuts>,
    generation: u64,
) -> Result<(), String> {
    let (reply, answered) = mpsc::channel();
    popouts.send(&app, Message::Restart { generation, reply });
    answer(answered).await
}

#[tauri::command]
pub fn popout_publish(app: AppHandle, popouts: State<'_, PopOuts>) {
    popouts.send(&app, Message::Publish);
}

#[tauri::command]
pub fn popout_unloaded(app: AppHandle, popouts: State<'_, PopOuts>, path: String, generation: u64) {
    popouts.unloaded(&path, generation, &app);
}

#[tauri::command]
pub fn popout_next_monitor(popouts: State<'_, PopOuts>, generation: u64) {
    popouts.shared.with(|at, popout| {
        if at == generation {
            popout.shell.next_monitor();
        }
    });
}

fn running(popouts: &PopOuts, generation: Option<u64>) -> Result<NativeHost, String> {
    let (current, host) = popouts
        .shared
        .current()
        .ok_or("The map isn’t popped out")?;
    if generation.is_some_and(|generation| generation != current) {
        return Err(CLOSED_WINDOW.into());
    }
    let status = host.status();
    if !status.running {
        return Err(status
            .error
            .map(|error| format!("The pop-out map stopped: {error}"))
            .unwrap_or_else(|| "The pop-out map stopped".into()));
    }
    Ok(host)
}

#[tauri::command]
pub fn popout_seek(
    popouts: State<'_, PopOuts>,
    generation: u64,
    path: String,
    revision: String,
    branch: String,
    hour: i32,
    playback: Option<MapPlayback>,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.seek(crate::replay_path(&path)?, revision, branch, hour, playback)
}

#[tauri::command]
pub fn popout_configure(
    popouts: State<'_, PopOuts>,
    generation: u64,
    options: dashboard_core::protocol::MapOptions,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::Configure { options })
}

#[tauri::command]
pub fn popout_camera_mode(
    popouts: State<'_, PopOuts>,
    generation: u64,
    mode: dashboard_core::protocol::CameraMode,
    overlays: bool,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::CameraMode { mode, overlays })
}

#[tauri::command]
pub fn popout_fly_aircraft(
    popouts: State<'_, PopOuts>,
    generation: u64,
    aircraft: Option<String>,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::FlyAircraft { aircraft })
}

pub fn export(popouts: &PopOuts, generation: Option<u64>, path: String) -> Result<tokio::sync::oneshot::Receiver<viewer::host::Answer>, String> {
    let generation = generation.ok_or("The map isn’t popped out")?;
    running(popouts, Some(generation))?.export(path, None)
}

pub fn snapshot(popouts: &PopOuts, generation: Option<u64>) -> Result<tokio::sync::oneshot::Receiver<viewer::host::SnapshotAnswer>, String> {
    let generation = generation.ok_or("The map isn’t popped out")?;
    running(popouts, Some(generation))?.snapshot(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unloading_matches_the_shown_requested_or_seeking_recording_in_any_spelling() {
        let recordings = vec![
            r"\\?\C:\Captures\Shown.battleplan".to_owned(),
            r"C:\Captures\Opening.battleplan".to_owned(),
        ];
        assert!(shows(&recordings, r"C:\Captures\Shown.battleplan"));
        assert!(shows(&recordings, "c:/captures/opening.battleplan"));
        assert!(!shows(&recordings, r"C:\Captures\Other.battleplan"));
        assert!(!shows(&[], r"C:\Captures\Shown.battleplan"));
    }
}
