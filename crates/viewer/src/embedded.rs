//! The map's link to the desktop host in the same process, and the geometry of its window.
//!
//! A command wakes the map's thread only for work; the map's UI and GPU path can stay idle.
//! The map's window belongs to the host's thread, so its geometry changes are posted to it.

use job_priority::SpawnCarried as _;
use dashboard::protocol::{
    CameraMode, DashboardMapState, DashboardMapView, GamepadMessage, NativeCommand, NativeEvent,
    ViewBounds, ViewerState, validate_map_views,
};
use eframe::egui;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    Graphics::Gdi::*,
    UI::WindowsAndMessaging::*,
};

const STATE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq)]
pub struct Seek {
    pub path: PathBuf,
    pub revision: String,
    pub branch: replay_data::BranchId,
    pub hour: i32,
    pub playback: Option<dashboard::protocol::MapPlayback>,
}

impl Seek {
    pub fn matches(&self, path: &Path, revision: Option<&str>) -> bool {
        self.path.as_path() == path && revision == Some(self.revision.as_str())
    }

    pub fn open_request(&self) -> OpenRequest {
        OpenRequest {
            path: self.path.clone(),
            revision: Some(self.revision.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub path: PathBuf,
    // Native picker/CLI requests establish their own snapshot at load time.
    pub revision: Option<String>,
}

fn validate_snapshot(path: &str, revision: &str) -> Result<(), String> {
    if path.is_empty() || path.contains('\0') {
        return Err("A replay path must be nonempty and contain no NUL".into());
    }
    if revision.is_empty() || revision.len() > 128 {
        return Err("A replay command must include its recording revision".into());
    }
    Ok(())
}

/// One slot per command kind. A new open invalidates seeks for the old replay;
/// a close is terminal, including if more commands were sent after it.
#[derive(Default)]
pub struct Pending {
    pub actions: VecDeque<dashboard::protocol::MapAction>,
    /// The Campaign map takes the window back from the dashboard's maps.
    pub campaign_map: bool,
    pub open: Option<OpenRequest>,
    pub seek: Option<Seek>,
    pub close: bool,
    pub error: Option<String>,
    pub map_views: Option<Vec<DashboardMapView>>,
    /// The host's controller message, merged with any after it
    /// ([`GamepadMessage::merge`]).
    pub gamepad: Option<GamepadMessage>,
    bounds: Option<ViewBounds>,
    visibility: Option<bool>,
}

impl Pending {
    fn is_empty(&self) -> bool {
        self.open.is_none()
            && self.actions.is_empty()
            && !self.campaign_map
            && self.map_views.is_none()
            && self.seek.is_none()
            && !self.close
            && self.bounds.is_none()
            && self.visibility.is_none()
            && self.gamepad.is_none()
            && self.error.is_none()
    }

    fn push(&mut self, command: NativeCommand) -> Result<(), String> {
        if self.close {
            return Ok(());
        }
        match command {
            NativeCommand::MapAction { action } => {
                action.validate().map_err(str::to_owned)?;
                if let dashboard::protocol::MapAction::Unload { path } = &action {
                    let path = Path::new(path);
                    if self
                        .open
                        .as_ref()
                        .is_some_and(|pending| pending.path == path)
                    {
                        self.open = None;
                    }
                    if self
                        .seek
                        .as_ref()
                        .is_some_and(|pending| pending.path == path)
                    {
                        self.seek = None;
                    }
                }
                self.actions.push_back(action);
            }
            NativeCommand::CampaignMap => {
                self.campaign_map = true;
                self.map_views = None;
            }
            NativeCommand::MapViews { views } => {
                validate_map_views(&views)?;
                self.map_views = Some(views);
                self.campaign_map = false;
                self.open = None;
                self.seek = None;
                self.error = None;
            }
            NativeCommand::Visibility { visible } => self.visibility = Some(visible),
            NativeCommand::Bounds {
                x,
                y,
                width,
                height,
            } => {
                self.bounds = Some(
                    ViewBounds {
                        x,
                        y,
                        width,
                        height,
                    }
                    .validate()?,
                );
            }
            NativeCommand::Open { path, revision } => {
                validate_snapshot(&path, &revision)?;
                self.open = Some(OpenRequest {
                    path: PathBuf::from(path),
                    revision: Some(revision),
                });
                self.seek = None;
                self.error = None;
            }
            NativeCommand::Seek {
                path,
                revision,
                branch,
                hour,
                playback,
            } => {
                if let Some(playback) = playback {
                    playback.validate()?;
                }
                validate_snapshot(&path, &revision)?;
                if branch.is_empty() || !branch.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("A branch must be an unsigned decimal string".into());
                }
                self.seek = Some(Seek {
                    path: PathBuf::from(path),
                    revision,
                    branch: branch
                        .parse()
                        .map_err(|_| "Branch exceeds the replay's u64 id")?,
                    hour,
                    playback,
                });
                self.error = None;
            }
            NativeCommand::Gamepad { message } => {
                message.validate()?;
                self.gamepad = Some(GamepadMessage::merge(self.gamepad.take(), message));
            }
            NativeCommand::Close => {
                *self = Self {
                    close: true,
                    ..Self::default()
                };
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ReportedState {
    viewer: ViewerState,
    error: Option<String>,
}

impl ReportedState {
    fn into_event(self) -> NativeEvent {
        NativeEvent::State {
            path: self.viewer.path,
            branch: self.viewer.branch,
            hour: self.viewer.hour,
            revision: self.viewer.revision,
            error: self.error,
        }
    }
}

#[derive(Default)]
struct Outgoing {
    details: Option<dashboard::protocol::MapDetails>,
    /// The window handle, and the map device's sample counts as attaching found them.
    ready: Option<(String, Option<Vec<u32>>)>,
    state: Option<ReportedState>,
    error: Option<String>,
    maps: BTreeMap<String, DashboardMapState>,
    map_order: VecDeque<String>,
    /// An odd number of diagnostics-shortcut presses not yet sent; two presses
    /// between sends cancel out.
    toggle_diagnostics: bool,
    /// The full map's camera mode and overlay choice to report; a newer pair replaces it.
    camera_mode: Option<(CameraMode, bool)>,
    /// The chosen aircraft's latest status not yet sent.
    fly_aircraft: Option<dashboard::protocol::FlyAircraftStatus>,
    /// Each view's latest picker options; a newer change replaces an unsent one.
    view_options: BTreeMap<String, dashboard::protocol::MapOptions>,
    /// Picker toggle clicks in order; each is one flip of a host setting.
    view_toggles: VecDeque<(String, dashboard::protocol::MapToggle)>,
    /// Each view's latest mode list not yet sent; a newer list replaces it.
    view_modes: BTreeMap<String, Vec<dashboard::protocol::MapModeChoice>>,
}

impl Outgoing {
    /// Records one diagnostics-shortcut press; a second press before the
    /// writer sends it cancels the first.
    fn toggle_diagnostics(&mut self) {
        self.toggle_diagnostics = !self.toggle_diagnostics;
    }

    /// The toggle event to send, if an odd number of presses is pending.
    fn take_toggle_diagnostics(&mut self) -> Option<NativeEvent> {
        std::mem::take(&mut self.toggle_diagnostics).then_some(NativeEvent::ToggleDiagnostics)
    }

    /// Records the camera mode and overlay choice to report; a newer pair replaces one not yet
    /// sent.
    fn report_camera_mode(&mut self, mode: CameraMode, overlays: bool) {
        self.camera_mode = Some((mode, overlays));
    }

    /// The camera mode event to send, once.
    fn take_camera_mode(&mut self) -> Option<NativeEvent> {
        self.camera_mode
            .take()
            .map(|(mode, overlays)| NativeEvent::CameraMode { mode, overlays })
    }

    /// Whether the event queue has nothing to send; map states have a queue of their own.
    fn is_empty(&self) -> bool {
        !self.toggle_diagnostics
            && self.camera_mode.is_none()
            && self.fly_aircraft.is_none()
            && self.view_options.is_empty()
            && self.view_toggles.is_empty()
            && self.view_modes.is_empty()
            && self.ready.is_none()
            && self.details.is_none()
            && self.state.is_none()
            && self.error.is_none()
    }

    /// Every pending map state, oldest first, as one event.
    fn take_maps(&mut self) -> Option<NativeEvent> {
        let states: Vec<_> = std::mem::take(&mut self.map_order)
            .into_iter()
            .filter_map(|id| self.maps.remove(&id))
            .collect();
        self.maps.clear();
        (!states.is_empty()).then_some(NativeEvent::MapStates { states })
    }
}

struct Shared {
    pending: Mutex<Pending>,
    outgoing: Mutex<Outgoing>,
    output_ready: Condvar,
    maps_ready: Condvar,
    stopped: AtomicBool,
    terminal_error: AtomicBool,
    ctx: egui::Context,
}

impl Shared {
    fn input(&self, command: NativeCommand) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let wake = pending.is_empty();
        if let Err(error) = pending.push(command) {
            pending.error = Some(error);
        }
        drop(pending);
        if wake {
            self.ctx.request_repaint();
        }
    }

    fn error(&self, message: impl Into<String>) {
        self.terminal_error.store(true, Ordering::Release);
        let mut outgoing = self
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.state = None;
        outgoing.error = Some(message.into());
        drop(outgoing);
        self.output_ready.notify_one();
    }

    /// Queues the map's detail report whole.
    fn details(&self, details: dashboard::protocol::MapDetails) {
        self.outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .details = Some(details);
        self.output_ready.notify_one();
    }

    /// Queues each map whose acknowledgement changed since `last`, which becomes `states`.
    fn maps(&self, last: &mut Option<Vec<DashboardMapState>>, states: Vec<DashboardMapState>) {
        if last.as_ref().is_some_and(|previous| {
            previous.len() == states.len()
                && previous.iter().zip(&states).all(|(old, new)| same_ack(old, new))
        }) {
            return;
        }
        let mut outgoing = self
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let ids: BTreeSet<_> = states.iter().map(|state| state.id.clone()).collect();
        outgoing.maps.retain(|id, _| ids.contains(id));
        outgoing.map_order.retain(|id| ids.contains(id));
        for state in &states {
            if last
                .as_ref()
                .is_some_and(|previous| previous.iter().any(|old| same_ack(old, state)))
            {
                continue;
            }
            // Updating a pending view preserves its FIFO position, so a fast
            // clock cannot starve another map's first ready/error response.
            if !outgoing.maps.contains_key(&state.id) {
                outgoing.map_order.push_back(state.id.clone());
            }
            outgoing.maps.insert(state.id.clone(), state.clone());
        }
        *last = Some(states);
        drop(outgoing);
        self.maps_ready.notify_one();
    }
}

/// Whether two map states acknowledge the same: the frame's fraction, how far it is behind the
/// clock and when it was drawn change with every frame, and the host shows none of them. So the
/// first state of a new hour or mode keeps the time of the frame that first drew it.
fn same_ack(old: &DashboardMapState, new: &DashboardMapState) -> bool {
    let bare = |state: &DashboardMapState| DashboardMapState {
        fraction: None,
        behind: None,
        drawn_at: None,
        ..state.clone()
    };
    bare(old) == bare(new)
}

/// Followed answers go to `send` from a thread of their own, so the stepper only queues: the
/// desktop installs the playheads once for its process. Until a playhead exists every follow is
/// "not available" by name and the widgets keep asking.
pub fn install_follow(send: impl Fn(u8, Vec<u8>) + Send + 'static) -> io::Result<bool> {
    let push = Arc::new(dashboard_follow::push::Push::default());
    let drained = Arc::clone(&push);
    thread::Builder::new().name("follow push".into()).spawn(move || {
        while let Some((kind, bytes, _, _)) = drained.take_kind() {
            send(kind, bytes);
        }
    })?;
    Ok(dashboard_follow::registry::install(Arc::new(FollowPlayheads { push, hosts: Mutex::default(), building: Mutex::default() })))
}

type FollowHost = dashboard_follow::host::PlayheadHost<crate::playhead_host::Entry>;

/// A follow host's recording, timeline and clock mapping.
type FollowKey = (String, u64, i32);

/// Follow on the map's playheads: one `Follow` a playhead, its hook in the host's slot, its reader
/// bound to `redeclare`, and its driver stepping the playhead when no map is drawn.
struct FollowPlayheads {
    push: Arc<dashboard_follow::push::Push>,
    hosts: Mutex<BTreeMap<FollowKey, Arc<FollowHost>>>,
    /// Hosts being built, by key: a caller of the same key waits for that build, one of another
    /// key or recording never does.
    building: Mutex<BTreeMap<FollowKey, std::sync::Weak<Mutex<()>>>>,
}

impl FollowPlayheads {
    fn hosted(&self, key: &FollowKey) -> Option<Arc<FollowHost>> {
        self.hosts.lock().unwrap_or_else(|error| error.into_inner()).get(key).cloned()
    }

    fn gate(&self, key: &FollowKey) -> Arc<Mutex<()>> {
        let mut building = self.building.lock().unwrap_or_else(|error| error.into_inner());
        building.retain(|_, gate| gate.strong_count() > 0);
        if let Some(gate) = building.get(key).and_then(std::sync::Weak::upgrade) {
            return gate;
        }
        let gate = Arc::new(Mutex::new(()));
        building.insert(key.clone(), Arc::downgrade(&gate));
        gate
    }
}

impl dashboard_follow::registry::Playheads for FollowPlayheads {
    fn open(&self, path: &Path, branch: u64, clock: i32) -> Result<Arc<dyn dashboard_follow::registry::Hosted>, String> {
        let push = &self.push;
        if let Some(failed) = push.failed() {
            return Err(failed);
        }
        let recording = replay::columnar_frames::Recording::open_path_as(path, replay::columnar_frames::Open::Saved).map_err(|error| error.to_string())?;
        let capture = replay_data::scope::CaptureMode::from_byte(recording.root().identity.capture_mode).map_err(|error| error.to_string())?;
        let key = (crate::playhead_host::units::name(&recording), branch, clock);
        if let Some(host) = self.hosted(&key) {
            return Ok(host as Arc<dyn dashboard_follow::registry::Hosted>);
        }
        let gate = self.gate(&key);
        let _building = gate.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(host) = self.hosted(&key) {
            return Ok(host as Arc<dyn dashboard_follow::registry::Hosted>);
        }
        // The map's read of the same recording is shared, never repeated ([`units::shared`]).
        let (units, _) = crate::playhead_host::units::at_path(path, Some(branch)).map_err(|error| error.to_string())?;
        // A mapping other than the map's (game hours, 0) runs a playhead of its own.
        let entry = crate::playhead_host::open_clocked(units, crate::playhead_host::settings(), clock).map_err(|error| error.to_string())?;
        entry.set_capture(capture);
        let follow = dashboard_follow::Follow::new(64 << 10);
        let due = dashboard_follow::due::Due::new(Arc::clone(push));
        entry.hooks().add(follow.hook(due.sink()));
        let held = dashboard_follow::host::held_tables(&**entry.pages(), entry.kinds().len());
        let rows = Arc::new(FollowRows { entry: Arc::clone(&entry), branch });
        let host = dashboard_follow::host::PlayheadHost::new(entry, crate::playhead_host::Entry::playhead, |entry, anchor| entry.anchor(anchor.hour, anchor.playing, anchor.speed, anchor.sought), crate::playhead_host::Entry::drawn, follow, due, held).map_err(|error| error.to_string())?;
        host.show_panels(rows, Arc::clone(push));
        self.hosts.lock().unwrap_or_else(|error| error.into_inner()).insert(key, Arc::clone(&host));
        Ok(host as Arc<dyn dashboard_follow::registry::Hosted>)
    }
}

/// Rows at the head of a follow host's playhead for its panels, at a panel's cadence.
struct FollowRows {
    entry: Arc<crate::playhead_host::Entry>,
    branch: u64,
}

impl dashboard_follow::panel::PanelRows for FollowRows {
    fn reader(&self, tables: &[replay_data::rows::TableId]) -> io::Result<Box<dyn dashboard_follow::panel::PanelReader>> {
        use crate::playhead_host::rows::{Cadence, Recording, Rows};
        let rows = Rows::of(&self.entry, Recording { branch: self.branch })?;
        Ok(Box::new(FollowRowReader(rows.reader(tables, Cadence::PerSecond(4)))))
    }
}

struct FollowRowReader(crate::playhead_host::rows::RowReader);

impl dashboard_follow::panel::PanelReader for FollowRowReader {
    fn frame(&self) -> Option<Arc<replay_frame::frame::DecodedFrame>> {
        self.0.frame()
    }

    fn stopped(&self) -> Option<(replay_data::Hour, String)> {
        self.0.stopped()
    }
}

/// The map's window, a child of the host's, and the map's link to its host: the commands it sends
/// and where the map's events go.
pub struct Parent {
    window: crate::host::Window,
    commands: mpsc::Receiver<NativeCommand>,
    events: crate::host::Events,
}

impl Parent {
    pub fn new(
        window: crate::host::Window,
        commands: mpsc::Receiver<NativeCommand>,
        events: crate::host::Events,
    ) -> Self {
        Self { window, commands, events }
    }

    pub fn attach(self, ctx: &egui::Context) -> io::Result<Controller> {
        let Self { window, commands, events } = self;
        let window = NativeWindow::new(window)?;
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
            outgoing: Mutex::new(Outgoing::default()),
            output_ready: Condvar::new(),
            maps_ready: Condvar::new(),
            stopped: AtomicBool::new(false),
            terminal_error: AtomicBool::new(false),
            ctx: ctx.clone(),
        });
        // The map's own service, which its loads read sources and preferences from; the host's
        // dashboard queries go to the host's own.
        let service = dashboard::Service::new(Arc::new(|_: dashboard::transport::DataEvent| false))?;
        let (output_finished, output_done) = mpsc::sync_channel::<()>(0);
        let controller = Controller {
            window,
            shared: Arc::clone(&shared),
            last_state: None,
            last_report: None,
            closing: false,
            output_done,
            service,
            last_maps: None,
        };
        let input = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-commands".into())
            .spawn_carried(move || {
                while !input.stopped.load(Ordering::Acquire) {
                    // The host dropping its end closes the map.
                    let Ok(command) = commands.recv() else { break };
                    let close = matches!(command, NativeCommand::Close);
                    input.input(command);
                    if close {
                        return;
                    }
                }
                input.input(NativeCommand::Close);
            })?;
        let output = Arc::clone(&shared);
        let output_events = Arc::clone(&events);
        thread::Builder::new()
            .name("viewer-events".into())
            .spawn_carried(move || {
                // Disconnecting this zero-capacity channel signals that all
                // queued events were sent, without blocking the sender.
                let _finished = output_finished;
                loop {
                    let mut outgoing = output
                        .outgoing
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    while outgoing.is_empty() && !output.stopped.load(Ordering::Acquire) {
                        outgoing = output
                            .output_ready
                            .wait(outgoing)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                    if outgoing.is_empty() {
                        return;
                    }
                    let events =
                        [
                            outgoing
                                .details
                                .take()
                                .map(|details| NativeEvent::MapDetails { details }),
                            outgoing
                                .ready
                                .take()
                                .map(|(hwnd, sample_counts)| NativeEvent::Ready {
                                    hwnd,
                                    sample_counts,
                                }),
                            outgoing.state.take().map(ReportedState::into_event),
                            outgoing
                                .error
                                .take()
                                .map(|message| NativeEvent::Error { message }),
                            outgoing.take_toggle_diagnostics(),
                            outgoing.take_camera_mode(),
                            outgoing
                                .fly_aircraft
                                .take()
                                .map(|status| NativeEvent::FlyAircraft {
                                    aircraft: status.aircraft,
                                    state: status.state,
                                }),
                            outgoing.view_options.pop_first().map(|(view_id, options)| {
                                NativeEvent::MapOptions { view_id, options }
                            }),
                            outgoing.view_toggles.pop_front().map(|(view_id, toggle)| {
                                NativeEvent::MapToggle { view_id, toggle }
                            }),
                            outgoing
                                .view_modes
                                .pop_first()
                                .map(|(view_id, modes)| NativeEvent::MapModes { view_id, modes }),
                        ];
                    drop(outgoing);
                    for event in events.into_iter().flatten() {
                        output_events(event);
                    }
                }
            })?;
        // Map acknowledgements go on a thread of their own, so a burst of events never queues
        // ahead of them.
        let maps = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-map-states".into())
            .spawn(move || {
                loop {
                    let mut outgoing = maps
                        .outgoing
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    while outgoing.maps.is_empty() && !maps.stopped.load(Ordering::Acquire) {
                        outgoing = maps
                            .maps_ready
                            .wait(outgoing)
                            .unwrap_or_else(|error| error.into_inner());
                    }
                    let Some(event) = outgoing.take_maps() else {
                        return;
                    };
                    drop(outgoing);
                    events(event);
                }
            })?;
        shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .ready = Some((
            controller.window.window.hwnd().to_string(),
            // Recorded as the map opened its device ([`crate::render::configuration`]).
            dashboard::map_settings::sample_counts(),
        ));
        shared.output_ready.notify_one();
        ctx.request_repaint();
        Ok(controller)
    }
}

pub struct Controller {
    window: NativeWindow,
    shared: Arc<Shared>,
    last_state: Option<ReportedState>,
    last_report: Option<Instant>,
    closing: bool,
    output_done: mpsc::Receiver<()>,
    service: Arc<dashboard::Service>,
    last_maps: Option<Vec<DashboardMapState>>,
}

impl Controller {
    pub fn service(&self) -> Arc<dashboard::Service> {
        Arc::clone(&self.service)
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Pending {
        let mut pending = std::mem::take(
            &mut *self
                .shared
                .pending
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        );
        pending.close |= self.closing;
        if !pending.close {
            let applied = (|| {
                if pending.campaign_map {
                    self.window.clear_region()?;
                }
                if let Some(bounds) = pending.bounds {
                    self.window.bounds(bounds)?;
                }
                if let Some(visible) = pending.visibility {
                    self.window.visibility(visible);
                }
                if let Some(views) = &pending.map_views {
                    self.window.map_views(views)?;
                }
                Ok::<_, io::Error>(())
            })();
            if let Err(error) = applied {
                self.error(format!("Embedding window: {error}"));
                pending.close = true;
            }
        }
        if pending.close {
            self.closing = true;
            self.window.visibility(false);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        pending
    }

    pub fn visible(&self) -> bool {
        self.window.visible()
    }

    pub fn frame_size(&self) -> Option<[u32; 2]> {
        self.window.placement.get().bounds.map(|bounds| [bounds.width, bounds.height])
    }

    pub fn report_details(&self, details: dashboard::protocol::MapDetails) {
        if let Some(path) = std::env::var_os("BATTLE_PLAN_MAP_DETAILS_FILE").or_else(|| std::env::var_os("HOI4_MAP_DETAILS_FILE"))
            && let Ok(bytes) = serde_json::to_vec(&serde_json::json!({
                "at": crate::perf_log::epoch_ms(), "details": &details,
            }))
        {
            let _ = std::fs::write(path, bytes);
        }
        self.shared.details(details);
    }

    /// Sends the chosen aircraft's status; one not yet sent is replaced.
    pub fn report_fly_aircraft(&self, status: dashboard::protocol::FlyAircraftStatus) {
        self.shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .fly_aircraft = Some(status);
        self.shared.output_ready.notify_one();
    }

    /// Asks the host to flip its diagnostics choice; it answers with
    /// `MapAction::Diagnostics`.
    pub fn toggle_diagnostics(&self) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.toggle_diagnostics();
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    /// Reports the full map's camera mode and overlay choice to the host, whose controls follow
    /// them.
    pub fn report_camera_mode(&self, mode: CameraMode, overlays: bool) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.report_camera_mode(mode, overlays);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    /// Reports a view's options after the picker changed them; the host stores them.
    pub fn report_view_options(&self, view_id: &str, options: dashboard::protocol::MapOptions) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.view_options.insert(view_id.to_owned(), options);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    /// Reports the modes a view's picker offers, for the host's own map mode control.
    pub fn report_view_modes(&self, view_id: &str, modes: Vec<dashboard::protocol::MapModeChoice>) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.view_modes.insert(view_id.to_owned(), modes);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    /// Reports a picker toggle click whose setting the host owns.
    pub fn report_view_toggle(&self, view_id: &str, toggle: dashboard::protocol::MapToggle) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing
            .view_toggles
            .push_back((view_id.to_owned(), toggle));
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    pub fn report_maps(&mut self, states: Vec<DashboardMapState>) {
        self.shared.maps(&mut self.last_maps, states);
    }

    fn error(&self, message: impl Into<String>) {
        self.shared.error(message);
    }

    pub fn report(&mut self, state: ViewerState, error: Option<&str>, ctx: &egui::Context) {
        let state = ReportedState {
            viewer: state,
            error: error.map(str::to_owned),
        };
        if self.last_state.as_ref() == Some(&state) {
            return;
        }
        if let Some(last) = self.last_report
            && let Some(remaining) = STATE_INTERVAL.checked_sub(last.elapsed())
        {
            ctx.request_repaint_after(remaining);
            return;
        }
        self.last_state = Some(state.clone());
        self.last_report = Some(Instant::now());
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.shared.terminal_error.load(Ordering::Acquire) {
            return;
        }
        outgoing.state = Some(state);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        {
            // Pair shutdown with the condition variable's mutex so a writer
            // entering its wait cannot miss the final notification.
            let _outgoing = self
                .shared
                .outgoing
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shared.stopped.store(true, Ordering::Release);
        }
        self.shared.output_ready.notify_one();
        self.shared.maps_ready.notify_one();
        let service = Arc::clone(&self.service);
        // Runtime/file joins must not stall eframe's window destruction.
        let _ = thread::Builder::new()
            .name("dashboard shutdown".into())
            .spawn_carried(move || service.shutdown());
        // Let the host receive the final error/state before the map ends, but never hang the
        // map's thread on a host that stopped taking events.
        let _ = self.output_done.recv_timeout(Duration::from_millis(100));
        // Close ends the command thread; a close from the window itself leaves it waiting until
        // the host drops its end. Never join it on the map's thread.
    }
}

/// Where the host placed the map's window: shown only once it is both visible and placed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Placement {
    visible: bool,
    bounds: Option<ViewBounds>,
}

impl Placement {
    fn show(self) -> bool {
        self.visible && self.bounds.is_some()
    }
}

/// The map's window as its host places it.
struct NativeWindow {
    window: crate::host::Window,
    placement: Cell<Placement>,
    region: RefCell<Option<Vec<ViewBounds>>>,
}

impl NativeWindow {
    fn new(window: crate::host::Window) -> io::Result<Self> {
        // SAFETY: a query on the map's window handle.
        if unsafe { GetParent(window.hwnd() as HWND) }.is_null() {
            return Err(io::Error::other("The map's window has no host window"));
        }
        Ok(Self { window, placement: Cell::new(Placement::default()), region: RefCell::new(None) })
    }

    fn hwnd(&self) -> HWND {
        self.window.hwnd() as HWND
    }

    fn bounds(&self, bounds: ViewBounds) -> io::Result<()> {
        let bounds = bounds.validate().map_err(io::Error::other)?;
        let placement = Placement { bounds: Some(bounds), ..self.placement.get() };
        self.placement.set(placement);
        let visibility = if placement.show() { SWP_SHOWWINDOW } else { SWP_HIDEWINDOW };
        // SAFETY: an asynchronous call on the map's window; its own thread applies it.
        let success = unsafe {
            SetWindowPos(
                self.hwnd(),
                HWND_TOP,
                bounds.x,
                bounds.y,
                bounds.width as i32,
                bounds.height as i32,
                SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS | visibility,
            )
        };
        if success == 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
    }

    fn visibility(&self, visible: bool) {
        let placement = Placement { visible, ..self.placement.get() };
        self.placement.set(placement);
        // SAFETY: asynchronous calls on the map's window; its own thread applies them.
        unsafe {
            ShowWindowAsync(self.hwnd(), if placement.show() { SW_SHOWNOACTIVATE } else { SW_HIDE });
            if placement.show() && GetForegroundWindow() == GetAncestor(self.hwnd(), GA_ROOT) {
                self.window.focus();
            }
        }
    }

    fn map_views(&self, views: &[DashboardMapView]) -> io::Result<()> {
        let mut client = RECT::default();
        // SAFETY: a query on the host's window, the map window's parent.
        if unsafe { GetClientRect(GetParent(self.hwnd()), &mut client) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let root = ViewBounds {
            x: 0,
            y: 0,
            width: (client.right - client.left).max(1) as u32,
            height: (client.bottom - client.top).max(1) as u32,
        };
        let regions: Vec<_> = views
            .iter()
            .filter(|view| view.visible)
            .filter_map(|view| intersect_bounds(view.bounds, view.clip))
            .filter_map(|clip| intersect_bounds(clip, root))
            .collect();
        if self.placement.get().bounds != Some(root) {
            self.bounds(root)?;
        }
        if self.region.borrow().as_ref() != Some(&regions) {
            // The window is borderless: client and window-relative origins agree.
            // Windows owns the final HRGN only after SetWindowRgn succeeds.
            unsafe {
                let region = CreateRectRgn(0, 0, 0, 0);
                if region.is_null() {
                    return Err(io::Error::last_os_error());
                }
                for clip in &regions {
                    let part = CreateRectRgn(
                        clip.x,
                        clip.y,
                        clip.x + clip.width as i32,
                        clip.y + clip.height as i32,
                    );
                    if part.is_null() {
                        DeleteObject(region);
                        return Err(io::Error::last_os_error());
                    }
                    let combined = CombineRgn(region, region, part, RGN_OR);
                    DeleteObject(part);
                    if combined == 0 {
                        DeleteObject(region);
                        return Err(io::Error::other(
                            "Cannot combine native map clipping regions",
                        ));
                    }
                }
                if SetWindowRgn(self.hwnd(), region, 1) == 0 {
                    DeleteObject(region);
                    return Err(io::Error::last_os_error());
                }
            }
            *self.region.borrow_mut() = Some(regions.clone());
        }
        let visible = !regions.is_empty();
        if self.placement.get().visible != visible {
            self.visibility(visible);
        }
        Ok(())
    }

    fn clear_region(&self) -> io::Result<()> {
        if self.region.borrow().is_some() {
            if unsafe { SetWindowRgn(self.hwnd(), ptr::null_mut(), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            *self.region.borrow_mut() = None;
        }
        Ok(())
    }

    fn visible(&self) -> bool {
        self.placement.get().show() && self.window.shown()
    }
}

impl Drop for NativeWindow {
    fn drop(&mut self) {
        self.visibility(false);
    }
}

fn intersect_bounds(left: ViewBounds, right: ViewBounds) -> Option<ViewBounds> {
    let x = i64::from(left.x).max(i64::from(right.x));
    let y = i64::from(left.y).max(i64::from(right.y));
    let end_x = (i64::from(left.x) + i64::from(left.width))
        .min(i64::from(right.x) + i64::from(right.width));
    let end_y = (i64::from(left.y) + i64::from(left.height))
        .min(i64::from(right.y) + i64::from(right.height));
    (end_x > x && end_y > y).then_some(ViewBounds {
        x: x as i32,
        y: y as i32,
        width: (end_x - x).max(0) as u32,
        height: (end_y - y).max(0) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> Shared {
        Shared {
            pending: Mutex::default(),
            outgoing: Mutex::default(),
            output_ready: Condvar::new(),
            maps_ready: Condvar::new(),
            stopped: AtomicBool::new(false),
            terminal_error: AtomicBool::new(false),
            ctx: egui::Context::default(),
        }
    }

    #[test]
    fn a_detail_report_past_sixteen_thousand_bytes_arrives_with_every_line() {
        let mut messages: Vec<String> = (0..436)
            .map(|table| format!("Table{table:03}: unavailable ({})", "not in this recording ".repeat(4)))
            .collect();
        messages.push("Capture 1.250 ms".into());
        assert!(serde_json::to_vec(&messages).unwrap().len() > 16_000);
        let shared = shared();
        shared.details(dashboard::protocol::MapDetails {
            messages: messages.clone(),
            ..Default::default()
        });
        let mut outgoing = shared.outgoing.into_inner().unwrap();
        assert!(outgoing.error.is_none());
        let details = outgoing.details.take().expect("the report is queued");
        assert_eq!(details.messages, messages);
    }

    #[test]
    fn a_long_map_error_and_a_long_message_arrive_whole() {
        let shared = shared();
        let map_error = "é".repeat(500);
        assert_eq!(map_error.len(), 1_000);
        let mut last = None;
        shared.maps(
            &mut last,
            vec![DashboardMapState {
                id: "m".into(),
                error: Some(map_error.clone()),
                ..Default::default()
            }],
        );
        let message = "x".repeat(10_000);
        shared.error(message.clone());
        let mut outgoing = shared.outgoing.into_inner().unwrap();
        let Some(NativeEvent::MapStates { states }) = outgoing.take_maps() else {
            panic!("pending map states")
        };
        assert_eq!(states[0].error.as_deref(), Some(map_error.as_str()));
        assert_eq!(outgoing.error, Some(message));
    }

    #[test]
    fn twenty_map_actions_queued_before_a_frame_are_all_taken_in_order() {
        use dashboard::protocol::MapAction;
        let names: Vec<String> = (0..20).map(|n| format!("plane {n}")).collect();
        let mut pending = Pending::default();
        for name in &names {
            pending
                .push(NativeCommand::MapAction {
                    action: MapAction::FlyAircraft {
                        aircraft: Some(name.clone()),
                    },
                })
                .unwrap();
        }
        let taken: Vec<String> = std::mem::take(&mut pending)
            .actions
            .into_iter()
            .map(|action| match action {
                MapAction::FlyAircraft {
                    aircraft: Some(name),
                } => name,
                other => panic!("unexpected action {other:?}"),
            })
            .collect();
        assert_eq!(taken, names);
    }

    #[test]
    fn two_diagnostics_presses_before_a_send_cancel_out() {
        let mut outgoing = Outgoing::default();
        assert!(outgoing.is_empty());
        outgoing.toggle_diagnostics();
        outgoing.toggle_diagnostics();
        assert!(outgoing.is_empty());
        assert!(outgoing.take_toggle_diagnostics().is_none());
        outgoing.toggle_diagnostics();
        assert!(!outgoing.is_empty());
        assert!(matches!(
            outgoing.take_toggle_diagnostics(),
            Some(NativeEvent::ToggleDiagnostics)
        ));
        // Sent once: the press is consumed.
        assert!(outgoing.is_empty() && outgoing.take_toggle_diagnostics().is_none());
    }

    #[test]
    fn an_acknowledgement_keeps_the_time_of_the_first_frame_that_drew_it() {
        let drawn = |hour, mode: &str, at| DashboardMapState {
            id: "m".into(),
            hour: Some(hour),
            map_mode: Some(mode.into()),
            drawn_at: Some(at),
            ..Default::default()
        };
        // Later frames of the same hour and mode acknowledge nothing new.
        assert!(same_ack(&drawn(5, "default", 10.0), &drawn(5, "default", 20.0)));
        assert!(!same_ack(&drawn(5, "default", 10.0), &drawn(5, "terrain", 20.0)));
        assert!(!same_ack(&drawn(5, "default", 10.0), &drawn(6, "default", 20.0)));
    }

    #[test]
    fn a_views_latest_mode_list_replaces_an_unsent_one_and_leaves_on_the_event_queue() {
        let mut outgoing = Outgoing::default();
        let choice = |token: &str| dashboard::protocol::MapModeChoice {
            token: token.into(),
            label: token.into(),
            unavailable: None,
        };
        outgoing.view_modes.insert("m".into(), vec![choice("default")]);
        outgoing.view_modes.insert("m".into(), vec![choice("terrain")]);
        assert!(!outgoing.is_empty());
        assert_eq!(outgoing.view_modes.pop_first(), Some(("m".into(), vec![choice("terrain")])));
        assert!(outgoing.is_empty());
    }

    #[test]
    fn map_states_leave_together_in_order_and_off_the_event_queue() {
        let mut outgoing = Outgoing::default();
        for id in ["b", "a"] {
            outgoing.map_order.push_back(id.into());
            outgoing.maps.insert(
                id.into(),
                DashboardMapState {
                    id: id.into(),
                    ..Default::default()
                },
            );
        }
        assert!(outgoing.is_empty());
        let Some(NativeEvent::MapStates { states }) = outgoing.take_maps() else {
            panic!("pending map states")
        };
        assert_eq!(
            states.iter().map(|state| state.id.as_str()).collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert!(outgoing.maps.is_empty() && outgoing.take_maps().is_none());
    }

    /// The host hears the latest camera mode and overlay choice once.
    #[test]
    fn the_latest_camera_mode_is_reported_once() {
        let mut outgoing = Outgoing::default();
        outgoing.report_camera_mode(CameraMode::Fly, false);
        outgoing.report_camera_mode(CameraMode::Walk, true);
        assert!(!outgoing.is_empty());
        assert!(matches!(
            outgoing.take_camera_mode(),
            Some(NativeEvent::CameraMode {
                mode: CameraMode::Walk,
                overlays: true,
            })
        ));
        assert!(outgoing.is_empty() && outgoing.take_camera_mode().is_none());
    }

    /// A take waits with its newest state; a state outside its ranges is refused and the waiting
    /// message kept; a release queued after a state wins over it; a camera mode passes as a map
    /// action.
    #[test]
    fn controller_messages_wait_merged_and_camera_modes_reach_the_map() {
        use dashboard::protocol::{
            CameraMode, GamepadButton, GamepadOwner, GamepadState, MapAction,
        };
        let owner = GamepadOwner {
            viewer: 1,
            handoff: 4,
        };
        let state = |x| GamepadState {
            sticks: [[x, 0.0], [0.0; 2]],
            ..GamepadState::default()
        };
        let mut pending = Pending::default();
        pending
            .push(NativeCommand::Gamepad {
                message: GamepadMessage::Take {
                    owner,
                    state: state(0.25),
                },
            })
            .unwrap();
        for x in [0.5, 0.75] {
            pending
                .push(NativeCommand::Gamepad {
                    message: GamepadMessage::State {
                        owner,
                        state: state(x),
                        pressed: 0,
                    },
                })
                .unwrap();
        }
        assert!(
            pending
                .push(NativeCommand::Gamepad {
                    message: GamepadMessage::State {
                        owner,
                        state: GamepadState {
                            triggers: [2.0, 0.0],
                            buttons: GamepadButton::South.bit(),
                            ..GamepadState::default()
                        },
                        pressed: 0,
                    },
                })
                .is_err()
        );
        assert_eq!(
            pending.gamepad,
            Some(GamepadMessage::Take {
                owner,
                state: state(0.75)
            })
        );
        pending
            .push(NativeCommand::Gamepad {
                message: GamepadMessage::Release,
            })
            .unwrap();
        pending
            .push(NativeCommand::Gamepad {
                message: GamepadMessage::State {
                    owner,
                    state: state(0.9),
                    pressed: 0,
                },
            })
            .unwrap();
        assert_eq!(pending.gamepad, Some(GamepadMessage::Release));
        assert!(!pending.is_empty());
        pending
            .push(NativeCommand::MapAction {
                action: MapAction::CameraMode {
                    mode: CameraMode::Fly,
                    overlays: true,
                },
            })
            .unwrap();
        assert!(matches!(
            pending.actions.back(),
            Some(MapAction::CameraMode {
                mode: CameraMode::Fly,
                overlays: true,
            })
        ));
        let line = serde_json::to_string(&NativeCommand::MapAction {
            action: MapAction::CameraMode {
                mode: CameraMode::Walk,
                overlays: true,
            },
        })
        .unwrap();
        assert_eq!(
            line,
            r#"{"command":"map_action","action":{"kind":"cameraMode","mode":"walk","overlays":true}}"#
        );
        // A host that leaves the choice out asks for the terrain alone.
        let terse: NativeCommand = serde_json::from_str(
            r#"{"command":"map_action","action":{"kind":"cameraMode","mode":"fly"}}"#,
        )
        .unwrap();
        assert!(matches!(
            terse,
            NativeCommand::MapAction {
                action: MapAction::CameraMode {
                    mode: CameraMode::Fly,
                    overlays: false,
                },
            }
        ));
    }

    #[test]
    fn unload_cancels_an_earlier_open_but_allows_a_later_reopen() {
        let mut pending = Pending::default();
        let open = NativeCommand::Open {
            path: "A".into(),
            revision: "v1".into(),
        };
        pending.push(open.clone()).unwrap();
        pending
            .push(NativeCommand::MapAction {
                action: dashboard::protocol::MapAction::Unload { path: "B".into() },
            })
            .unwrap();
        assert!(pending.open.is_some());
        pending
            .push(NativeCommand::MapAction {
                action: dashboard::protocol::MapAction::Unload { path: "A".into() },
            })
            .unwrap();
        assert!(pending.open.is_none());
        pending.push(open).unwrap();
        assert!(pending.open.is_some());
    }

    #[test]
    fn coalescing_preserves_open_seek_order_and_terminal_close() {
        let mut pending = Pending::default();
        for hour in 0..10_000 {
            pending
                .push(NativeCommand::Seek {
                    path: "initial.battleplan".into(),
                    revision: "100:10:0".into(),
                    branch: u64::MAX.to_string(),
                    hour,
                    playback: Some(dashboard::protocol::MapPlayback {
                        playing: true,
                        seek_sequence: (hour / 100) as u64,
                        fraction: 0.375,
                        hours_per_second: 1440.0,
                    }),
                })
                .unwrap();
        }
        assert_eq!(
            pending.seek,
            Some(Seek {
                path: PathBuf::from("initial.battleplan"),
                revision: "100:10:0".into(),
                branch: u64::MAX,
                hour: 9999,
                playback: Some(dashboard::protocol::MapPlayback {
                    playing: true,
                    seek_sequence: 99,
                    fraction: 0.375,
                    hours_per_second: 1440.0,
                }),
            })
        );
        pending
            .push(NativeCommand::Open {
                path: "new.battleplan".into(),
                revision: "100:20:0".into(),
            })
            .unwrap();
        assert_eq!(pending.seek, None);
        assert_eq!(
            pending.open.as_ref().unwrap().revision.as_deref(),
            Some("100:20:0")
        );
        pending
            .push(NativeCommand::Seek {
                path: "new.battleplan".into(),
                revision: "100:20:0".into(),
                branch: "2".into(),
                hour: 12,
                playback: None,
            })
            .unwrap();
        assert_eq!(
            pending.seek,
            Some(Seek {
                path: PathBuf::from("new.battleplan"),
                revision: "100:20:0".into(),
                branch: 2,
                hour: 12,
                playback: None,
            })
        );
        pending.push(NativeCommand::Close).unwrap();
        pending
            .push(NativeCommand::Open {
                path: "late.battleplan".into(),
                revision: "100:30:0".into(),
            })
            .unwrap();
        assert!(pending.close && pending.open.is_none() && pending.seek.is_none());
    }

    #[test]
    fn coalesced_seeks_retain_their_own_snapshot_even_with_the_same_position() {
        let mut pending = Pending::default();
        pending
            .push(NativeCommand::Open {
                path: "older.battleplan".into(),
                revision: "100:10:0".into(),
            })
            .unwrap();
        for (path, revision) in [
            ("A.battleplan", "100:20:0"),
            ("B.battleplan", "100:20:0"),
            ("B.battleplan", "120:30:0"),
        ] {
            pending
                .push(NativeCommand::Seek {
                    path: path.into(),
                    revision: revision.into(),
                    branch: u64::MAX.to_string(),
                    hour: 12,
                    playback: None,
                })
                .unwrap();
            let seek = pending.seek.as_ref().unwrap();
            assert_eq!(seek.path, PathBuf::from(path));
            assert_eq!(seek.revision, revision);
            assert_eq!(seek.branch, u64::MAX);
            assert_eq!(seek.hour, 12);
        }
        // The app can supersede the earlier open using the seek's target;
        // neither the earlier open nor another replay's position identifies it.
        assert_eq!(
            pending.seek.as_ref().unwrap().open_request(),
            OpenRequest {
                path: PathBuf::from("B.battleplan"),
                revision: Some("120:30:0".into()),
            }
        );
        assert_eq!(
            pending.open.as_ref().unwrap().path,
            PathBuf::from("older.battleplan")
        );
    }

    #[test]
    fn malformed_seek_target_cannot_replace_a_valid_pending_seek() {
        let mut pending = Pending::default();
        pending
            .push(NativeCommand::Seek {
                path: "A.battleplan".into(),
                revision: "100:20:0".into(),
                branch: "1".into(),
                hour: 12,
                playback: None,
            })
            .unwrap();
        let retained = pending.seek.clone();
        for (path, revision) in [
            (String::new(), "100:20:0".into()),
            ("B\0.battleplan".into(), "100:20:0".into()),
            ("B.battleplan".into(), String::new()),
            ("B.battleplan".into(), "x".repeat(129)),
        ] {
            assert!(
                pending
                    .push(NativeCommand::Seek {
                        path,
                        revision,
                        branch: "1".into(),
                        hour: 12,
                        playback: None,
                    })
                    .is_err()
            );
            assert_eq!(pending.seek, retained);
        }
    }

    #[test]
    fn error_only_changes_are_part_of_the_same_revision_state() {
        let mut reported = ReportedState {
            viewer: ViewerState {
                path: Some("session.battleplan".into()),
                branch: Some(u64::MAX.to_string()),
                hour: Some(12),
                revision: Some("100:20:0".into()),
            },
            error: None,
        };
        let previous = reported.clone();
        reported.error = Some("Requested hour is outside the branch".into());
        assert_ne!(previous, reported);
        let NativeEvent::State {
            revision, error, ..
        } = reported.into_event()
        else {
            panic!("wrong event");
        };
        assert_eq!(revision.as_deref(), Some("100:20:0"));
        assert_eq!(
            error.as_deref(),
            Some("Requested hour is outside the branch")
        );
    }

    #[test]
    fn the_map_shows_only_once_it_is_both_visible_and_placed() {
        let mut placement = Placement::default();
        assert!(!placement.show());
        placement.visible = true;
        assert!(!placement.show(), "visible before the host placed it");
        placement.bounds = Some(ViewBounds { x: 70, y: 0, width: 800, height: 600 });
        assert!(placement.show());
        placement.visible = false;
        assert!(!placement.show());
    }
}
