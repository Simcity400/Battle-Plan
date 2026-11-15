//! The pop-out map: the campaign map alone in its own window, driven by the desktop app.
//!
//! Each pop-out window has its own map ([`NativeHost::popout`]) on a thread of its own in this
//! process, like the main map. The frontend sends it the same open, seek and configure commands
//! as the campaign map, from the same clock; host-wide display choices reach it as they reach the
//! main map, queued while it starts. Closing the window, unloading its recording and closing the
//! app retire its map; the main map's restarts leave it alone. A map that stops reports why, and
//! the app offers to restart it.
//!
//! One owner thread runs the window's [`lifecycle`]: the app's requests, the window's own
//! reports and the results of its work reach it in order, so a close while opening, a reopen
//! while closing and a crash while closing each resolve one way. Every command names the window
//! generation it is for, and the owner drops one for any other. The owner alone publishes to the
//! frontend; every event carries the window's generation and a sequence number that only grows,
//! the frontend asks the owner to publish the current state once it listens, and it ignores any
//! event that is not newer than the last it took.
//!
//! The pop-out's map has its own camera mode and overlay choice: the campaign map's controls
//! choose them for window `generation` ([`popout_camera_mode`]), and the map's own shortcut cycles
//! the mode in the window. Its viewer reports both in its status. Controllers reach it while its window is in front
//! ([`PopOuts::in_front`]). The campaign's aircraft for Fly mode reaches it by its own command
//! ([`popout_fly_aircraft`]), and its viewer reports in its status how that aircraft loaded.
mod lifecycle;
mod shell;

use crate::native::{HostStatus, NativeHost};
use dashboard::protocol::{MapAction, MapPlayback, ViewBounds};
use lifecycle::{Effect, Input, Lifecycle};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
};
use tauri::{AppHandle, Emitter, Manager, State};

/// The export target naming the pop-out's map.
pub const VIEW_ID: &str = "popout";
/// The event carrying [`PopOutState`] and, between them, the viewer's own [`PopOutStatus`].
/// Only the owner thread emits it.
const STATE_EVENT: &str = "popout-state";
/// The event carrying a [`PopOutError`] the window or viewer could not report itself.
const ERROR_EVENT: &str = "popout-error";

struct PopOut {
    shell: shell::Shell,
    host: NativeHost,
}

/// What every thread may read: the open window with its generation, and the last generation
/// published to the frontend.
#[derive(Default)]
struct Shared {
    current: Mutex<Option<(u64, PopOut)>>,
    published: AtomicU64,
}

/// The open window's generation and host, read together.
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

/// The commands the frontend refuses for a window that is no longer the current one.
const CLOSED_WINDOW: &str = "That pop-out map window has closed";

/// A request for the owner thread.
enum Message {
    Input(Input),
    /// Open, beside the main window's handle, answering once open or failed.
    Open {
        main: usize,
        reply: Sender<Result<(), String>>,
    },
    /// Close window `generation`, or whatever is there with `None`, answering once nothing is
    /// open, opening or closing.
    Close {
        generation: Option<u64>,
        done: Option<Sender<()>>,
    },
    Restart {
        generation: u64,
        reply: Sender<Result<(), String>>,
    },
    /// Publish the whole state as it is now: a frontend that just started listening.
    Publish,
}

#[derive(Default)]
pub struct PopOuts {
    shared: Arc<Shared>,
    owner: Mutex<Option<Sender<Message>>>,
}

/// The whole pop-out state, from opening, closing and a frontend asking for it.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutState {
    generation: u64,
    /// The owner's event number, which only grows.
    sequence: u64,
    /// Which viewer of the window this is: each start or restart is a new one, so the frontend
    /// sends a restarted viewer everything again.
    viewer: u64,
    open: bool,
    status: Option<HostStatus>,
    /// A failure the window reported and has not recovered from, such as a resize.
    error: Option<String>,
}

/// The viewer status of window `generation`, read when the owner publishes it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutStatus {
    generation: u64,
    sequence: u64,
    viewer: u64,
    status: HostStatus,
}

/// An error the window or viewer of window `generation` could not report itself.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PopOutError {
    generation: u64,
    sequence: u64,
    error: String,
}

/// How a host-wide display choice reaches the pop-out's viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mirror {
    Diagnostics(bool),
    /// Every map's audio is silenced or restored together.
    Muted(bool),
    /// VSync and map scale, queued like the main viewer's.
    Display,
    SettingsSaved,
    /// The user asked for installed mods to be read again: the pop-out's maps check theirs too.
    RefreshVisualMods,
}

/// The display choices the main viewer receives that the pop-out's also needs; recording,
/// perspective and export actions name their own target.
fn mirrored(action: &MapAction) -> Option<Mirror> {
    match action {
        MapAction::Diagnostics { enabled } => Some(Mirror::Diagnostics(*enabled)),
        MapAction::Mute { muted } => Some(Mirror::Muted(*muted)),
        MapAction::Vsync { .. } | MapAction::UiScale { .. } => Some(Mirror::Display),
        MapAction::MapSettingsSaved => Some(Mirror::SettingsSaved),
        MapAction::RefreshVisualMods => Some(Mirror::RefreshVisualMods),
        MapAction::Unload { .. }
        | MapAction::Configure { .. }
        | MapAction::Export { .. }
        | MapAction::Inspect
        | MapAction::Folders { .. }
        // Each map has its own camera, and each is told its aircraft by its own command.
        | MapAction::CameraMode { .. }
        | MapAction::FlyAircraft { .. } => None,
    }
}

/// A recording path in one spelling: no extended-length prefix, backslashes, any case.
fn path_key(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_lowercase()
}

/// Whether `path` is one of the recordings a host shows or is opening.
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

    /// Sends `message` to the owner thread, starting it on first use.
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

    /// Host-wide display choices the main viewer receives also reach the pop-out, queued while
    /// its viewer starts; one started later takes the saved choices at launch.
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

    /// Newly saved content folders reach the pop-out's viewer too.
    pub fn set_folders(&self, config: &dashboard::Config) {
        if let Some(host) = self.host() {
            host.set_folders(config);
        }
    }

    /// The open window's generation when `foreground` is its window: the window in front, which
    /// Windows.Gaming.Input reports controllers to.
    pub fn in_front(&self, foreground: usize) -> Option<u64> {
        self.shared
            .with(|generation, popout| {
                (foreground != 0 && popout.shell.hwnd() == foreground).then_some(generation)
            })
            .flatten()
    }

    /// The host of pop-out window `generation` while it is the open one.
    pub fn host_of(&self, generation: u64) -> Option<NativeHost> {
        self.shared
            .current()
            .and_then(|(current, host)| (current == generation).then_some(host))
    }

    /// The open window's generation, captured when a request starts.
    pub fn current_generation(&self) -> Option<u64> {
        self.shared.current().map(|(generation, _)| generation)
    }

    /// Unloading a recording closes pop-out window `generation`, the one the frontend saw, when
    /// it is still the open window and shows the recording or is opening it.
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

    /// Closes whatever pop-out is there and waits, briefly, until its viewer and window are
    /// gone: the app is exiting.
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

/// The owner thread: the only code that changes the lifecycle, opens or releases a window, or
/// publishes the pop-out's whole state.
struct Owner {
    app: AppHandle,
    shared: Arc<Shared>,
    /// Its own queue, for outcomes that must come after what is already waiting.
    sender: Sender<Message>,
    lifecycle: Lifecycle,
    main: usize,
    /// The window being created, until it is published or released.
    opening: Option<(u64, PopOut)>,
    /// The open request being handled, until the lifecycle names the generation answering it.
    asking: Option<Sender<Result<(), String>>>,
    /// Open requests by the generation whose outcome answers them.
    open_waiters: Vec<(u64, Sender<Result<(), String>>)>,
    restart_waiters: Vec<Sender<Result<(), String>>>,
    close_waiters: Vec<Sender<()>>,
    /// Why a generation's window or viewer failed to open or start, until answered.
    errors: std::collections::BTreeMap<u64, String>,
    /// The last event number published.
    sequence: u64,
    /// The last viewer started, in any window.
    viewer: u64,
    /// A failure the current window reported and has not recovered from.
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
                    // Every open request is bound to a generation; one never is is refused.
                    if let Some(reply) = self.asking.take() {
                        let _ = reply.send(Err("The pop-out map could not open".into()));
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
            // A close completes once nothing is open, opening or closing, on every path.
            if self.lifecycle.is_closed() {
                self.answer_closed();
            }
        }
    }

    fn next_sequence(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    /// The whole state as it is now, numbered `sequence`.
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
                        // The viewer's status changes reach the frontend only through here.
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
                // After anything already waiting, such as the window's own close.
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
                // Each start or restart is a new viewer of the window.
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
                // A resize never starts a viewer; only a start or restart does.
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
                    // Recovered: the next whole state no longer carries the error.
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
                self.shared.with(|_, popout| popout.shell.raise());
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
                    // The window takes the viewer's child window with it, so it goes only once the
                    // viewer has let go of the surface it made there.
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
                // Only the named window's error answers it.
                let error = self.errors.remove(&generation).unwrap_or_else(|| {
                    if generation == self.lifecycle.generation() {
                        "The map is not popped out".into()
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

    /// Starts window `generation`'s viewer at the window's size.
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
            .ok_or("The map is not popped out")?;
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

    /// The open requests `generation`'s outcome answers.
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

/// Waits off the async runtime for the owner's answer.
async fn answer<T: Send + 'static>(receiver: Receiver<Result<T, String>>) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv()
            .unwrap_or_else(|_| Err("The pop-out map stopped responding".into()))
    })
    .await
    .map_err(|error| error.to_string())?
}

/// Opens the pop-out, or raises the open one. Its state reaches the frontend as events only.
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

/// Closes pop-out window `generation`, the one the frontend shows.
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

/// Starts the exited viewer of pop-out window `generation` again.
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

/// Asks the owner to publish the whole state as an event: the frontend calls it once its
/// listeners are registered, so nothing published after that is missed.
#[tauri::command]
pub fn popout_publish(app: AppHandle, popouts: State<'_, PopOuts>) {
    popouts.send(&app, Message::Publish);
}

/// The user unloaded a recording while pop-out window `generation` was shown; that window
/// closes if it shows the recording. Reconnecting a recording does not call this, so the pop-out
/// follows the reopened recording instead.
#[tauri::command]
pub fn popout_unloaded(app: AppHandle, popouts: State<'_, PopOuts>, path: String, generation: u64) {
    popouts.unloaded(&path, generation, &app);
}

/// Moves pop-out window `generation` to the next monitor.
#[tauri::command]
pub fn popout_next_monitor(popouts: State<'_, PopOuts>, generation: u64) {
    popouts.shared.with(|at, popout| {
        if at == generation {
            popout.shell.next_monitor();
        }
    });
}

/// The host of pop-out window `generation` (or of the open one with `None`) while its viewer
/// runs; one that exited names why, and a command for another window is refused.
fn running(popouts: &PopOuts, generation: Option<u64>) -> Result<NativeHost, String> {
    let (current, host) = popouts
        .shared
        .current()
        .ok_or("The map is not popped out")?;
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
#[allow(clippy::too_many_arguments)]
pub fn popout_seek(
    popouts: State<'_, PopOuts>,
    generation: u64,
    path: String,
    revision: String,
    branch: String,
    hour: i32,
    playback: Option<MapPlayback>,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.seek(
        crate::replay_path(&path)?.to_string_lossy().into_owned(),
        revision,
        branch,
        hour,
        playback,
    )
}

/// The pop-out's own perspective and fog, which the campaign map's controls choose.
#[tauri::command]
pub fn popout_configure(
    popouts: State<'_, PopOuts>,
    generation: u64,
    options: dashboard::protocol::MapOptions,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::Configure { options })
}

/// The camera mode of pop-out window `generation`'s map and whether a free mode shows the map's
/// interface, which the campaign map's controls choose.
#[tauri::command]
pub fn popout_camera_mode(
    popouts: State<'_, PopOuts>,
    generation: u64,
    mode: dashboard::protocol::CameraMode,
    overlays: bool,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::CameraMode { mode, overlays })
}

/// The aircraft the map of pop-out window `generation` flies in Fly mode: the campaign's one
/// choice, which the campaign map's controls send to each map; its status reports how it loaded.
#[tauri::command]
pub fn popout_fly_aircraft(
    popouts: State<'_, PopOuts>,
    generation: u64,
    aircraft: Option<String>,
) -> Result<(), String> {
    running(&popouts, Some(generation))?.map_action(MapAction::FlyAircraft { aircraft })
}

/// Exports the map of pop-out window `generation`, captured before the save dialog opened; a
/// window that closed meanwhile is refused.
pub fn export(popouts: &PopOuts, generation: Option<u64>, path: String) -> Result<(), String> {
    let generation = generation.ok_or("The map is not popped out")?;
    running(popouts, Some(generation))?.map_action(MapAction::Export {
        path,
        view_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_display_choices_reach_the_pop_out() {
        assert_eq!(
            mirrored(&MapAction::Diagnostics { enabled: true }),
            Some(Mirror::Diagnostics(true))
        );
        assert_eq!(
            mirrored(&MapAction::Mute { muted: true }),
            Some(Mirror::Muted(true))
        );
        assert_eq!(
            mirrored(&MapAction::Vsync { enabled: false }),
            Some(Mirror::Display)
        );
        assert_eq!(
            mirrored(&MapAction::UiScale { scale: None }),
            Some(Mirror::Display)
        );
        assert_eq!(
            mirrored(&MapAction::MapSettingsSaved),
            Some(Mirror::SettingsSaved)
        );
        // A refresh reads the mods every viewer shows, the pop-out's too.
        assert_eq!(
            mirrored(&MapAction::RefreshVisualMods),
            Some(Mirror::RefreshVisualMods)
        );
        for action in [
            MapAction::Unload { path: "a".into() },
            MapAction::Configure {
                options: Default::default(),
            },
            MapAction::Export {
                path: "a.png".into(),
                view_id: None,
            },
            MapAction::Inspect,
            MapAction::Folders {
                game: "g".into(),
                user: "u".into(),
            },
            MapAction::CameraMode {
                mode: dashboard::protocol::CameraMode::Fly,
                overlays: true,
            },
            MapAction::FlyAircraft {
                aircraft: Some("plane_entity".into()),
            },
        ] {
            assert_eq!(mirrored(&action), None, "{action:?}");
        }
    }

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
