
use job_priority::SpawnCarried as _;
use dashboard_core::protocol::{CameraMode, DashboardMapState, DashboardMapView, GamepadMessage, NativeCommand, NativeEvent, ViewBounds, ViewerState, validate_map_holes, validate_map_views};
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
    pub playback: Option<dashboard_core::protocol::MapPlayback>,
}

impl Seek {
    pub fn matches(&self, path: &Path, revision: Option<&str>) -> bool {
        self.path.as_path() == path && revision == Some(self.revision.as_str())
    }

    pub fn open_request(&self) -> OpenRequest {
        OpenRequest {
            path: self.path.clone(),
            revision: Some(self.revision.clone()),
            open_id: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub path: PathBuf,
    pub revision: Option<String>,
    pub open_id: Option<u64>,
}

fn validate_snapshot(path: &str, revision: Option<&str>) -> Result<(), String> {
    if path.is_empty() || path.contains('\0') {
        return Err("A replay path must be nonempty and contain no NUL".into());
    }
    if revision.is_some_and(|revision| revision.is_empty() || revision.len() > 128) {
        return Err("A replay command must include its recording revision".into());
    }
    Ok(())
}

#[derive(Default)]
pub struct Pending {
    pub actions: VecDeque<dashboard_core::protocol::MapAction>,
    pub exports: Vec<crate::host::ExportJob>,
    pub snapshots: Vec<crate::host::SnapshotJob>,
    pub campaign_map: bool,
    pub open: Option<OpenRequest>,
    pub seek: Option<Seek>,
    pub close: bool,
    pub error: Option<String>,
    pub map_views: Option<(Option<String>, Vec<DashboardMapView>)>,
    pub gamepad: Option<GamepadMessage>,
    bounds: Option<ViewBounds>,
    visibility: Option<bool>,
    holes: Option<Vec<ViewBounds>>,
    holes_applied: Vec<Box<dyn FnOnce() + Send>>,
    clock_origins: crate::page_clock::OriginGate,
}

impl Pending {
    fn take(&mut self) -> Self {
        let origins = std::mem::take(&mut self.clock_origins);
        let taken = std::mem::take(self);
        self.clock_origins = origins;
        taken
    }

    fn is_empty(&self) -> bool {
        self.open.is_none()
            && self.actions.is_empty()
            && self.exports.is_empty()
            && self.snapshots.is_empty()
            && !self.campaign_map
            && self.map_views.is_none()
            && self.seek.is_none()
            && !self.close
            && self.bounds.is_none()
            && self.visibility.is_none()
            && self.holes.is_none()
            && self.gamepad.is_none()
            && self.error.is_none()
    }

    fn push(&mut self, command: impl Into<crate::host::Command>) -> Result<(), String> {
        if self.close {
            return Ok(());
        }
        let command = match command.into() {
            crate::host::Command::Native(command) => command,
            crate::host::Command::Export(job) => {
                self.exports.push(job);
                return Ok(());
            }
            crate::host::Command::Snapshot(job) => {
                self.snapshots.push(job);
                return Ok(());
            }
            crate::host::Command::Holes(holes) => {
                validate_map_holes(&holes.rects)?;
                self.holes = Some(holes.rects);
                self.holes_applied.push(holes.applied);
                return Ok(());
            }
        };
        match command {
            NativeCommand::MapAction { action } => {
                action.validate().map_err(str::to_owned)?;
                if let dashboard_core::protocol::MapAction::Unload { path } = &action {
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
            NativeCommand::MapViews { clock_id, views } => {
                match clock_id.as_deref() {
                    Some(clock_id) => {
                        validate_map_views(clock_id, &views)?;
                        if self.clock_origins.admit(clock_id)
                            == crate::page_clock::OriginStatus::Retired
                        {
                            return Ok(());
                        }
                    }
                    None if !views.is_empty() => {
                        return Err(
                            "Map clock identity is missing or exceeds its control budget".into(),
                        );
                    }
                    None => {
                        if self.clock_origins.active().is_some() {
                            return Ok(());
                        }
                    }
                }
                self.map_views = Some((clock_id, views));
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
            NativeCommand::Open { path, revision, open_id } => {
                validate_snapshot(&path, revision.as_deref())?;
                self.open = Some(OpenRequest {
                    path: PathBuf::from(path),
                    revision,
                    open_id,
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
                if let Some(playback) = &playback {
                    playback.validate()?;
                }
                validate_snapshot(&path, Some(&revision))?;
                if branch.is_empty() || !branch.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err("A branch must be an unsigned decimal string".into());
                }
                let branch = branch
                    .parse()
                    .map_err(|_| "Branch exceeds the replay's u64 id")?;
                if playback.as_ref().is_some_and(|clock| {
                    self.clock_origins.admit(&clock.clock_id)
                        == crate::page_clock::OriginStatus::Retired
                }) {
                    return Ok(());
                }
                self.seek = Some(Seek {
                    path: PathBuf::from(path),
                    revision,
                    branch,
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
    open_id: Option<u64>,
    error: Option<String>,
}

impl ReportedState {
    fn into_event(self) -> NativeEvent {
        NativeEvent::State {
            path: self.viewer.path,
            branch: self.viewer.branch,
            hour: self.viewer.hour,
            revision: self.viewer.revision,
            open_id: self.open_id,
            error: self.error,
        }
    }
}

#[derive(Default)]
struct Outgoing {
    details: Option<dashboard_core::protocol::MapDetails>,
    ready: Option<(String, Option<Vec<u32>>)>,
    state: Option<ReportedState>,
    error: Option<String>,
    maps: BTreeMap<String, DashboardMapState>,
    map_order: VecDeque<String>,
    toggle_diagnostics: bool,
    camera_mode: Option<(CameraMode, bool)>,
    fly_aircraft: Option<dashboard_core::protocol::FlyAircraftStatus>,
    view_options: BTreeMap<String, dashboard_core::protocol::MapOptions>,
    view_toggles: VecDeque<(String, dashboard_core::protocol::MapToggle)>,
    view_modes: BTreeMap<String, Vec<dashboard_core::protocol::MapModeChoice>>,
    view_observes: BTreeMap<String, dashboard_core::protocol::ObservedCountry>,
}

impl Outgoing {
    fn toggle_diagnostics(&mut self) {
        self.toggle_diagnostics = !self.toggle_diagnostics;
    }

    fn take_toggle_diagnostics(&mut self) -> Option<NativeEvent> {
        std::mem::take(&mut self.toggle_diagnostics).then_some(NativeEvent::ToggleDiagnostics)
    }

    fn report_camera_mode(&mut self, mode: CameraMode, overlays: bool) {
        self.camera_mode = Some((mode, overlays));
    }

    fn take_camera_mode(&mut self) -> Option<NativeEvent> {
        self.camera_mode
            .take()
            .map(|(mode, overlays)| NativeEvent::CameraMode { mode, overlays })
    }

    fn is_empty(&self) -> bool {
        !self.toggle_diagnostics
            && self.camera_mode.is_none()
            && self.fly_aircraft.is_none()
            && self.view_options.is_empty()
            && self.view_toggles.is_empty()
            && self.view_modes.is_empty()
            && self.view_observes.is_empty()
            && self.ready.is_none()
            && self.details.is_none()
            && self.state.is_none()
            && self.error.is_none()
    }

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
    fn input(&self, command: crate::host::Command) {
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

    fn details(&self, details: dashboard_core::protocol::MapDetails) {
        self.outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .details = Some(details);
        self.output_ready.notify_one();
    }

    fn maps(&self, sent: &mut Sent, states: Vec<DashboardMapState>, now: Instant) -> Option<Duration> {
        let mut outgoing = self
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let ids: BTreeSet<_> = states.iter().map(|state| state.id.clone()).collect();
        outgoing.maps.retain(|id, _| ids.contains(id));
        outgoing.map_order.retain(|id| ids.contains(id));
        sent.0.retain(|id, _| ids.contains(id));
        let mut queued = false;
        let mut owed = None;
        for state in states {
            let held = sent.0.get(&state.id);
            if held.is_some_and(|(old, _)| same_ack(old, &state)) {
                continue;
            }
            let urgent = held.is_none_or(|(old, _)| !clock_only(old, &state));
            let waited = held.map(|(_, at)| now.duration_since(*at));
            if urgent || waited.is_none_or(|waited| waited >= STATE_INTERVAL) {
                if !outgoing.maps.contains_key(&state.id) {
                    outgoing.map_order.push_back(state.id.clone());
                }
                outgoing.maps.insert(state.id.clone(), state.clone());
                sent.0.insert(state.id.clone(), (state, now));
                queued = true;
                continue;
            }
            let remaining = STATE_INTERVAL.saturating_sub(waited.unwrap_or_default());
            owed = Some(owed.map_or(remaining, |owed: Duration| owed.min(remaining)));
        }
        drop(outgoing);
        if queued {
            self.maps_ready.notify_one();
        }
        owed
    }
}

#[derive(Default)]
struct Sent(BTreeMap<String, (DashboardMapState, Instant)>);

fn same_ack(old: &DashboardMapState, new: &DashboardMapState) -> bool {
    let bare = |state: &DashboardMapState| DashboardMapState {
        fraction: None,
        behind: None,
        drawn_at: None,
        ..state.clone()
    };
    bare(old) == bare(new)
}

fn clock_only(old: &DashboardMapState, new: &DashboardMapState) -> bool {
    let clockless = |state: &DashboardMapState| DashboardMapState {
        hour: None,
        fraction: None,
        sequence: 0,
        behind: None,
        drawn_at: None,
        ..state.clone()
    };
    clockless(old) == clockless(new)
}

pub fn install_follow(send: impl Fn(u8, Vec<u8>, u64) + Send + 'static) -> io::Result<bool> {
    let budget = dashboard_follow::lead::budget();
    let push = Arc::new(dashboard_follow::push::Push::with_budget(Arc::clone(&budget)));
    let drained = Arc::clone(&push);
    thread::Builder::new().name("follow push".into()).spawn(move || {
        while let Some((kind, bytes, session)) = drained.take_sent() {
            send(kind, bytes, session);
        }
    })?;
    Ok(dashboard_follow::registry::install(Arc::new(FollowPlayheads {
        push,
        budget,
        hosts: Mutex::default(),
        building: Mutex::default(),
        private: false,
    })))
}

pub fn install_export_follow(send: impl Fn(u8, Vec<u8>, u64) + Send + 'static) -> io::Result<bool> {
    let budget = dashboard_follow::lead::export_budget();
    let push = Arc::new(dashboard_follow::push::Push::with_budget(Arc::clone(&budget)));
    let drained = Arc::clone(&push);
    thread::Builder::new().name("export follow push".into()).spawn(move || {
        while let Some((kind, bytes, session)) = drained.take_sent() {
            send(kind, bytes, session);
        }
    })?;
    Ok(dashboard_follow::registry::install_export(Arc::new(FollowPlayheads {
        push,
        budget,
        hosts: Mutex::default(),
        building: Mutex::default(),
        private: true,
    })))
}

type FollowHost = dashboard_follow::host::PlayheadHost<crate::playhead_host::Entry>;

type FollowKey = (String, u64, i32);

struct FollowPlayheads {
    push: Arc<dashboard_follow::push::Push>,
    budget: Arc<playhead::ByteBudget>,
    hosts: Mutex<BTreeMap<FollowKey, std::sync::Weak<FollowHost>>>,
    building: Mutex<BTreeMap<FollowKey, std::sync::Weak<Mutex<()>>>>,
    private: bool,
}

impl FollowPlayheads {
    fn hosted(&self, key: &FollowKey) -> Option<Arc<FollowHost>> {
        let mut hosts = self.hosts.lock().unwrap_or_else(|error| error.into_inner());
        hosts.retain(|_, host| host.strong_count() > 0);
        hosts.get(key).and_then(std::sync::Weak::upgrade)
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
    fn credit(&self, session: u64, released: u64) {
        self.push.credit(session, released);
    }

    fn restart(&self) -> u64 {
        self.push.restart()
    }

    fn open(&self, path: &Path, branch: u64, clock: i32) -> Result<Arc<dyn dashboard_follow::registry::Hosted>, String> {
        let push = &self.push;
        if let Some(failed) = push.failed() {
            return Err(failed);
        }
        let recording = replay_recording::columnar_frames::Recording::open_path_as(path, replay_recording::columnar_frames::Open::Saved).map_err(|error| error.to_string())?;
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
        let (units, _) = crate::playhead_host::units::at_path(path, Some(branch)).map_err(|error| error.to_string())?;
        let entry = if self.private {
            crate::playhead_host::open_private(units, crate::playhead_host::settings(), crate::playhead_host::Side::Follow)
        } else {
            crate::playhead_host::open_clocked(units, crate::playhead_host::settings(), clock, crate::playhead_host::Side::Follow)
        }
        .map_err(|error| error.to_string())?;
        entry.set_capture(capture);
        let follow = dashboard_follow::Follow::new(clock);
        let out = Arc::clone(push);
        let hook = follow.hook(Box::new(move |stats, message| {
            if let Some(message) = message { out.offer_from(stats.session, dashboard_follow::push::FOLLOW, message.bytes); }
        }));
        let held = dashboard_follow::host::held_tables(&**entry.pages(), entry.kinds().len());
        let host = dashboard_follow::host::PlayheadHost::new_with_budget(Arc::clone(&entry), crate::playhead_host::Entry::playhead, crate::playhead_host::Entry::follow_anchor, follow, held, Arc::clone(&self.budget)).map_err(|error| error.to_string())?;
        let hook = entry.hooks().add(hook);
        let rows = Arc::new(FollowRows { entry, branch, hook, views: std::sync::Mutex::new(std::sync::Weak::new()), on_publish: std::sync::Mutex::new(None) });
        host.show_panels(rows, Arc::clone(push));
        self.hosts.lock().unwrap_or_else(|error| error.into_inner()).insert(key, Arc::downgrade(&host));
        Ok(host as Arc<dyn dashboard_follow::registry::Hosted>)
    }
}

struct FollowRows {
    hook: u64,
    entry: Arc<crate::playhead_host::Entry>,
    branch: u64,
    views: std::sync::Mutex<std::sync::Weak<crate::playhead_host::views::apart::Following>>,
    on_publish: std::sync::Mutex<Option<dashboard_follow::panel::Wake>>,
}

impl Drop for FollowRows {
    fn drop(&mut self) {
        self.entry.hooks().remove(self.hook);
    }
}

impl dashboard_follow::panel::PanelRows for FollowRows {
    fn reader(&self, tables: &[replay_data::rows::TableId]) -> io::Result<Box<dyn dashboard_follow::panel::PanelReader>> {
        use crate::playhead_host::views::{Need, Pace, apart::Following};
        let (following, created) = {
            let mut held = self.views.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            match held.upgrade() {
                Some(following) => (following, false),
                None => {
                    let following = Arc::new(Following::open(&self.entry, self.branch)?);
                    *held = Arc::downgrade(&following);
                    (following, true)
                }
            }
        };
        if created
            && let Some(wake) = self.on_publish.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
        {
            following.views().set_on_publish(wake);
        }
        let reader = following.views().reader_at("follow panel", &tables.iter().copied().map(Need::every).collect::<Vec<Need>>(), Pace::Step);
        Ok(Box::new(FollowRowReader { reader, entry: Arc::clone(&self.entry), _following: following }))
    }

    fn on_publish(&self, wake: dashboard_follow::panel::Wake) {
        *self.on_publish.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(Arc::clone(&wake));
        let following = self.views.lock().unwrap_or_else(std::sync::PoisonError::into_inner).upgrade();
        if let Some(following) = following {
            following.views().set_on_publish(wake);
        }
    }
}

struct FollowRowReader {
    reader: crate::playhead_host::views::Reader,
    entry: Arc<crate::playhead_host::Entry>,
    _following: Arc<crate::playhead_host::views::apart::Following>,
}

impl dashboard_follow::panel::PanelReader for FollowRowReader {
    fn through_seek(&self, frame: &Arc<replay_frame::frame::DecodedFrame>) -> Option<(replay_data::Hour, u64)> {
        self.reader.through_seek(Arc::as_ptr(frame))
    }

    fn frame(&self) -> Option<Arc<replay_frame::frame::DecodedFrame>> {
        self.reader.frame()
    }

    fn stopped(&self) -> Option<(replay_data::Hour, String)> {
        let playhead = self.entry.playhead();
        let error = playhead.error()?;
        let head = playhead.head(std::time::Instant::now());
        Some((self.reader.frame().map_or(head, |frame| frame.hour.min(head)), error))
    }
}

pub struct Parent {
    window: crate::host::Window,
    commands: mpsc::Receiver<crate::host::Command>,
    events: crate::host::Events,
    service: Arc<dashboard::Service>,
}

impl Parent {
    pub fn new(
        window: crate::host::Window,
        commands: mpsc::Receiver<crate::host::Command>,
        events: crate::host::Events,
        service: Arc<dashboard::Service>,
    ) -> Self {
        Self { window, commands, events, service }
    }

    pub fn attach(self, ctx: &egui::Context) -> io::Result<Controller> {
        let Self { window, commands, events, service } = self;
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
        let (output_finished, output_done) = mpsc::sync_channel::<()>(0);
        let controller = Controller {
            window,
            shared: Arc::clone(&shared),
            last_state: None,
            last_report: None,
            closing: false,
            output_done,
            service,
            sent_maps: Sent::default(),
        };
        let input = Arc::clone(&shared);
        thread::Builder::new()
            .name("viewer-commands".into())
            .spawn_carried(move || {
                while !input.stopped.load(Ordering::Acquire) {
                    let Ok(command) = commands.recv() else { break };
                    let close = matches!(command, crate::host::Command::Native(NativeCommand::Close));
                    input.input(command);
                    if close {
                        return;
                    }
                }
                input.input(NativeCommand::Close.into());
            })?;
        let output = Arc::clone(&shared);
        let output_events = Arc::clone(&events);
        thread::Builder::new()
            .name("viewer-events".into())
            .spawn_carried(move || {
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
                            outgoing
                                .view_observes
                                .pop_first()
                                .map(|(view_id, country)| NativeEvent::ObserveCountry { view_id, country }),
                        ];
                    drop(outgoing);
                    for event in events.into_iter().flatten() {
                        output_events(event);
                    }
                }
            })?;
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
            dashboard_core::map_settings::sample_counts(),
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
    sent_maps: Sent,
}

impl Controller {
    pub fn service(&self) -> Arc<dashboard::Service> {
        Arc::clone(&self.service)
    }

    pub fn poll(&mut self, ctx: &egui::Context) -> Pending {
        let mut pending = self
            .shared
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        pending.close |= self.closing;
        if !pending.close {
            let holes_applied = std::mem::take(&mut pending.holes_applied);
            let applied = (|| {
                if pending.campaign_map {
                    self.window.campaign();
                }
                if let Some(holes) = pending.holes.take() {
                    self.window.holes(holes);
                }
                if let Some(bounds) = pending.bounds {
                    self.window.bounds(bounds)?;
                }
                if let Some(visible) = pending.visibility {
                    self.window.visibility(visible);
                }
                if let Some((_, views)) = &pending.map_views {
                    self.window.map_views(views)?;
                }
                self.window.reshape()
            })();
            match applied {
                Ok(()) => holes_applied.into_iter().for_each(|applied| applied()),
                Err(error) => {
                    self.error(format!("Embedding window: {error}"));
                    pending.close = true;
                }
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

    pub fn report_details(&self, details: dashboard_core::protocol::MapDetails) {
        if let Some(path) = std::env::var_os("BATTLEPLAN_MAP_DETAILS_FILE").or_else(|| std::env::var_os("HOI4_MAP_DETAILS_FILE"))
            && let Ok(bytes) = serde_json::to_vec(&serde_json::json!({
                "at": crate::perf_log::epoch_ms(), "details": &details,
            }))
        {
            let _ = std::fs::write(path, bytes);
        }
        self.shared.details(details);
    }

    pub fn report_fly_aircraft(&self, status: dashboard_core::protocol::FlyAircraftStatus) {
        self.shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .fly_aircraft = Some(status);
        self.shared.output_ready.notify_one();
    }

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

    pub fn report_view_options(&self, view_id: &str, options: dashboard_core::protocol::MapOptions) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.view_options.insert(view_id.to_owned(), options);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    pub fn report_view_observe(&self, view_id: &str, country: dashboard_core::protocol::ObservedCountry) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.view_observes.insert(view_id.to_owned(), country);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    pub fn report_view_modes(&self, view_id: &str, modes: Vec<dashboard_core::protocol::MapModeChoice>) {
        let mut outgoing = self
            .shared
            .outgoing
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        outgoing.view_modes.insert(view_id.to_owned(), modes);
        drop(outgoing);
        self.shared.output_ready.notify_one();
    }

    pub fn report_view_toggle(&self, view_id: &str, toggle: dashboard_core::protocol::MapToggle) {
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

    pub fn report_maps(&mut self, states: Vec<DashboardMapState>, ctx: &egui::Context) {
        if let Some(remaining) = self.shared.maps(&mut self.sent_maps, states, Instant::now()) {
            ctx.request_repaint_after(remaining);
        }
    }

    fn error(&self, message: impl Into<String>) {
        self.shared.error(message);
    }

    pub fn report(&mut self, state: ViewerState, open_id: Option<u64>, error: Option<&str>, ctx: &egui::Context) {
        let state = ReportedState {
            viewer: state,
            open_id,
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
            let _outgoing = self
                .shared
                .outgoing
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            self.shared.stopped.store(true, Ordering::Release);
        }
        self.shared.output_ready.notify_one();
        self.shared.maps_ready.notify_one();
        let _ = self.output_done.recv_timeout(Duration::from_millis(100));
    }
}

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

struct NativeWindow {
    window: crate::host::Window,
    placement: Cell<Placement>,
    views: RefCell<Option<Vec<ViewBounds>>>,
    holes: RefCell<Vec<ViewBounds>>,
    region: RefCell<Option<Shape>>,
    drawn: Cell<bool>,
    shaped: Cell<bool>,
}

#[derive(Clone, Debug, PartialEq)]
struct Shape {
    parts: Vec<ViewBounds>,
    holes: Vec<ViewBounds>,
}

fn shape(views: Option<&[ViewBounds]>, holes: &[ViewBounds], window: ViewBounds) -> Option<Shape> {
    let local = |rect: ViewBounds| ViewBounds { x: rect.x - window.x, y: rect.y - window.y, ..rect };
    let holes: Vec<_> = holes.iter().filter_map(|&hole| intersect_bounds(hole, window)).map(local).collect();
    let parts = match views {
        Some(views) => views.iter().map(|&view| local(view)).collect(),
        None if holes.is_empty() => return None,
        None => vec![local(window)],
    };
    Some(Shape { parts, holes })
}

fn build_region(shape: &Shape) -> io::Result<(HRGN, bool)> {
    // SAFETY: every region made here is combined into `region` and deleted, or deleted on failure.
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        if region.is_null() {
            return Err(io::Error::last_os_error());
        }
        let cuts = shape.parts.iter().map(|part| (part, RGN_OR)).chain(shape.holes.iter().map(|hole| (hole, RGN_DIFF)));
        let mut kind = NULLREGION;
        for (rect, mode) in cuts {
            let part = CreateRectRgn(rect.x, rect.y, rect.x + rect.width as i32, rect.y + rect.height as i32);
            if part.is_null() {
                DeleteObject(region);
                return Err(io::Error::last_os_error());
            }
            kind = CombineRgn(region, region, part, mode);
            DeleteObject(part);
            if kind == 0 {
                DeleteObject(region);
                return Err(io::Error::other("Cannot combine native map clipping regions"));
            }
        }
        Ok((region, kind != NULLREGION))
    }
}

impl NativeWindow {
    fn new(window: crate::host::Window) -> io::Result<Self> {
        if unsafe { GetParent(window.hwnd() as HWND) }.is_null() {
            return Err(io::Error::other("The map’s window has no host window"));
        }
        Ok(Self {
            window,
            placement: Cell::new(Placement::default()),
            views: RefCell::new(None),
            holes: RefCell::new(Vec::new()),
            region: RefCell::new(None),
            drawn: Cell::new(true),
            shaped: Cell::new(true),
        })
    }

    fn hwnd(&self) -> HWND {
        self.window.hwnd() as HWND
    }

    fn bounds(&self, bounds: ViewBounds) -> io::Result<()> {
        let bounds = bounds.validate().map_err(io::Error::other)?;
        if self.placement.get().bounds != Some(bounds) {
            self.shaped.set(false);
        }
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
        let focus = self.holes.borrow().is_empty();
        self.placement.set(placement);
        // SAFETY: asynchronous calls on the map's window; its own thread applies them.
        unsafe {
            ShowWindowAsync(self.hwnd(), if placement.show() { SW_SHOWNOACTIVATE } else { SW_HIDE });
            if focus && placement.show() && GetForegroundWindow() == GetAncestor(self.hwnd(), GA_ROOT) {
                self.window.focus();
            }
        }
    }

    fn campaign(&self) {
        if self.views.replace(None).is_some() {
            self.shaped.set(false);
        }
    }

    fn holes(&self, holes: Vec<ViewBounds>) {
        if *self.holes.borrow() != holes {
            *self.holes.borrow_mut() = holes;
            self.shaped.set(false);
        }
    }

    fn map_views(&self, views: &[DashboardMapView]) -> io::Result<()> {
        let mut client = RECT::default();
        if unsafe { GetClientRect(GetParent(self.hwnd()), &raw mut client) } == 0 {
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
        if self.views.borrow().as_ref() != Some(&regions) {
            *self.views.borrow_mut() = Some(regions);
            self.shaped.set(false);
        }
        Ok(())
    }

    fn reshape(&self) -> io::Result<()> {
        if self.shaped.replace(true) {
            return Ok(());
        }
        let Some(window) = self.placement.get().bounds else {
            return Ok(());
        };
        let shape = shape(self.views.borrow().as_deref(), &self.holes.borrow(), window);
        if *self.region.borrow() != shape {
            let (region, drawn) = match &shape {
                None => (ptr::null_mut(), true),
                Some(shape) => build_region(shape)?,
            };
            // SAFETY: Windows takes the region only when it accepts it, so a refused one is still deleted here.
            if unsafe { SetWindowRgn(self.hwnd(), region, 1) } == 0 {
                let error = io::Error::last_os_error();
                if !region.is_null() {
                    // SAFETY: `build_region` made this region and nothing else holds it.
                    unsafe { DeleteObject(region) };
                }
                self.shaped.set(false);
                return Err(error);
            }
            *self.region.borrow_mut() = shape;
            self.drawn.set(drawn);
        }
        if self.views.borrow().is_some() && self.placement.get().visible != self.drawn.get() {
            self.visibility(self.drawn.get());
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
        shared.details(dashboard_core::protocol::MapDetails {
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
        let mut sent = Sent::default();
        shared.maps(
            &mut sent,
            vec![DashboardMapState {
                id: "m".into(),
                error: Some(map_error.clone()),
                ..Default::default()
            }],
            Instant::now(),
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
        use dashboard_core::protocol::MapAction;
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
        assert!(same_ack(&drawn(5, "default", 10.0), &drawn(5, "default", 20.0)));
        assert!(!same_ack(&drawn(5, "default", 10.0), &drawn(5, "terrain", 20.0)));
        assert!(!same_ack(&drawn(5, "default", 10.0), &drawn(6, "default", 20.0)));
        assert!(clock_only(&drawn(5, "default", 10.0), &drawn(6, "default", 20.0)));
        assert!(!clock_only(&drawn(5, "default", 10.0), &drawn(6, "terrain", 20.0)));
    }

    #[test]
    fn a_views_latest_mode_list_replaces_an_unsent_one_and_leaves_on_the_event_queue() {
        let mut outgoing = Outgoing::default();
        let choice = |token: &str| dashboard_core::protocol::MapModeChoice {
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

    #[test]
    fn controller_messages_wait_merged_and_camera_modes_reach_the_map() {
        use dashboard_core::protocol::{CameraMode, GamepadButton, GamepadOwner, GamepadState, MapAction};
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
            revision: Some("v1".into()),
            open_id: None,
        };
        pending.push(open.clone()).unwrap();
        pending
            .push(NativeCommand::MapAction {
                action: dashboard_core::protocol::MapAction::Unload { path: "B".into() },
            })
            .unwrap();
        assert!(pending.open.is_some());
        pending
            .push(NativeCommand::MapAction {
                action: dashboard_core::protocol::MapAction::Unload { path: "A".into() },
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
                    playback: Some(dashboard_core::protocol::MapPlayback {
                        playing: true,
                        seek_sequence: (hour / 100) as u64,
                        fraction: 0.375,
                        hours_per_second: 1440.0,
                        composed_at: 0.0,
                        clock_id: "page-1".into(),
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
                playback: Some(dashboard_core::protocol::MapPlayback {
                    playing: true,
                    seek_sequence: 99,
                    fraction: 0.375,
                    hours_per_second: 1440.0,
                    composed_at: 0.0,
                    clock_id: "page-1".into(),
                }),
            })
        );
        pending
            .push(NativeCommand::Open {
                path: "new.battleplan".into(),
                revision: Some("100:20:0".into()),
                open_id: None,
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
                revision: Some("100:30:0".into()),
                open_id: None,
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
                revision: Some("100:10:0".into()),
                open_id: None,
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
        assert_eq!(
            pending.seek.as_ref().unwrap().open_request(),
            OpenRequest {
                path: PathBuf::from("B.battleplan"),
                revision: Some("120:30:0".into()),
                open_id: None,
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
            open_id: None,
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

    const FAILURE: &str = "the follow test's recording could not be read";

    struct Breaking {
        inner: Arc<dyn playhead::pages::Pages>,
        runs: Vec<BreakingRun>,
    }

    struct BreakingRun {
        inner: Arc<dyn playhead::pages::Pages>,
        run: usize,
        broken: Arc<std::sync::atomic::AtomicBool>,
    }

    impl replay_columnar::unit::reader::Extents for BreakingRun {
        fn read_at(&self, offset: u64, length: usize, out: &mut Vec<u8>) -> io::Result<()> {
            if self.broken.load(std::sync::atomic::Ordering::Acquire) {
                return Err(io::Error::other(FAILURE));
            }
            self.inner.extents(self.run).read_at(offset, length, out)
        }
    }

    impl playhead::pages::Pages for Breaking {
        fn extents(&self, run: usize) -> &dyn replay_columnar::unit::reader::Extents {
            &self.runs[run]
        }

        fn runs(&self) -> &[playhead::pages::Run] {
            self.inner.runs()
        }

        fn table_entry(&self, run: usize, block: usize, table: usize) -> std::io::Result<playhead::pages::EntryRef> {
            self.inner.table_entry(run, block, table)
        }

        fn held_tables(&self, tables: usize) -> Vec<bool> {
            self.inner.held_tables(tables)
        }

        fn written_tables(&self, runs: &[usize], tables: usize) -> Vec<playhead::pages::TableWritten> {
            self.inner.written_tables(runs, tables)
        }

        fn table_has_activity(&self, table: usize) -> bool {
            self.inner.table_has_activity(table)
        }

        fn kinds(&self, table: usize) -> Option<&[replay_columnar::state::Kind]> {
            self.inner.kinds(table)
        }

        fn held(&self, table: usize) -> usize {
            self.inner.held(table)
        }

        fn format(&self) -> u16 {
            self.inner.format()
        }
    }

    #[test]
    fn a_follow_panel_reports_its_playheads_read_error_at_its_last_rows_and_its_last_reader_retires_the_store() {
        use crate::playhead_host::views::tests::{RECORDING, hourly_steps, record};
        use dashboard_follow::panel::PanelRows as _;
        let broken = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut units = record("follow panel stops", 2 * replay_columnar::unit::BLOCK);
        let runs = (0..units.pages.runs().len()).map(|run| BreakingRun { inner: Arc::clone(&units.pages), run, broken: Arc::clone(&broken) }).collect();
        units.pages = Arc::new(Breaking { inner: Arc::clone(&units.pages), runs });
        let entry = crate::playhead_host::open_clocked(units, hourly_steps(), 0, crate::playhead_host::Side::Follow).unwrap();
        entry.set_capture(replay_data::scope::CaptureMode::Full);
        entry.playhead().seek(1);
        let rows = FollowRows { hook: entry.hooks().add(Box::new(|_, _| {})), entry: Arc::clone(&entry), branch: RECORDING.branch, views: std::sync::Mutex::new(std::sync::Weak::new()), on_publish: std::sync::Mutex::new(None) };
        let panel = rows.reader(&[replay_data::rows::TableId::Countries]).unwrap();
        let following = rows.views.lock().unwrap().clone();
        assert!(following.upgrade().is_some(), "the panel's reader holds the store it reads");
        assert_eq!(panel.stopped(), None);
        entry.playhead().wait_for(|| panel.frame().is_some_and(|frame| frame.hour == 1)).unwrap();

        broken.store(true, std::sync::atomic::Ordering::Release);
        entry.playhead().seek(1_000);
        let error = entry.playhead().wait_for(|| false).expect_err("the playhead stops on the failed read");
        assert!(error.contains(FAILURE), "the playhead's error: {error}");
        let (hour, reason) = panel.stopped().expect("a stopped playhead stops the panel");
        assert_eq!(hour, 1, "the hour of the last rows the panel was given");
        assert_eq!(reason, error, "the playhead's own error, with nothing added that could read as the recording's");

        let declarations = entry.playhead().declarations();
        drop(panel);
        assert!(following.upgrade().is_none(), "the last panel reader's close stops the store");
        assert!(entry.playhead().declarations() > declarations, "the store's playhead reader is retired");
    }

    #[test]
    fn coalescing_rejects_seeks_from_retired_clock_origins_before_drain() {
        let mut pending = Pending::default();
        let seek = |path: &str, hour: i32, clock_id: &str| NativeCommand::Seek {
            path: path.into(),
            revision: "rev-1".into(),
            branch: "1".into(),
            hour,
            playback: Some(dashboard_core::protocol::MapPlayback {
                playing: true,
                seek_sequence: 1,
                fraction: 0.0,
                hours_per_second: 1440.0,
                composed_at: 10.0,
                clock_id: clock_id.into(),
            }),
        };
        pending.push(seek("old.battleplan", 100, "page-A")).unwrap();
        pending.push(seek("new.battleplan", 200, "page-B")).unwrap();
        pending.push(seek("old.battleplan", 300, "page-A")).unwrap();
        let drained = pending.take();
        let kept = drained.seek.expect("page-B seek survives");
        assert_eq!(kept.path, PathBuf::from("new.battleplan"));
        assert_eq!(kept.hour, 200);
        assert_eq!(
            kept.playback.as_ref().map(|clock| clock.clock_id.as_str()),
            Some("page-B")
        );
        pending.push(seek("old.battleplan", 400, "page-A")).unwrap();
        assert!(pending.take().seek.is_none(), "origin retirement persists across drains");
    }

    #[test]
    fn map_views_envelope_gates_empty_clears_and_state_dedup_distinguishes_clock_origin() {
        let bounds = ViewBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        };
        let make = |clock_id: &str, sequence: u64, hour: i32| DashboardMapView {
            options: Default::default(),
            id: "map-1".into(),
            source_id: "source".into(),
            path: "campaign.battleplan".into(),
            revision: "rev-1".into(),
            branch_id: "1".into(),
            playing: false,
            hours_per_second: 24.0,
            hour,
            fraction: 0.0,
            sequence,
            seek_sequence: Some(sequence),
            composed_at: 1000.0,
            clock_id: clock_id.into(),
            bounds,
            clip: bounds,
            visible: true,
            focus_province: None,
            battle: None,
        };

        let mut pending = Pending::default();
        pending
            .push(NativeCommand::MapViews {
                clock_id: Some("page-A".into()),
                views: vec![make("page-A", 80, 100)],
            })
            .unwrap();
        pending
            .push(NativeCommand::MapViews {
                clock_id: Some("page-B".into()),
                views: vec![make("page-B", 0, 260)],
            })
            .unwrap();
        pending
            .push(NativeCommand::MapViews {
                clock_id: Some("page-A".into()),
                views: Vec::new(),
            })
            .unwrap();
        let (clock_id, views) = pending.take().map_views.expect("B's widgets survive late A clear");
        assert_eq!(clock_id.as_deref(), Some("page-B"));
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].hour, 260);

        pending
            .push(NativeCommand::MapViews {
                clock_id: Some("page-B".into()),
                views: Vec::new(),
            })
            .unwrap();
        pending
            .push(NativeCommand::MapViews {
                clock_id: Some("page-A".into()),
                views: vec![make("page-A", 81, 101)],
            })
            .unwrap();
        let (clock_id, views) = pending
            .take()
            .map_views
            .expect("B's legitimate empty clear survives late A snapshot");
        assert_eq!(clock_id.as_deref(), Some("page-B"));
        assert!(views.is_empty());

        let state_a = DashboardMapState {
            id: "map-1".into(),
            clock_id: "page-A".into(),
            source_id: Some("source".into()),
            revision: Some("rev-1".into()),
            hour: Some(260),
            sequence: 0,
            requested_sequence: 0,
            ..Default::default()
        };
        let state_b = DashboardMapState {
            clock_id: "page-B".into(),
            ..state_a.clone()
        };
        assert!(!same_ack(&state_a, &state_b));
        assert!(!clock_only(&state_a, &state_b));

        let mut after_seek = Pending::default();
        after_seek
            .push(NativeCommand::Seek {
                path: "campaign.battleplan".into(),
                revision: "rev-1".into(),
                branch: "1".into(),
                hour: 100,
                playback: Some(dashboard_core::protocol::MapPlayback {
                    playing: false,
                    seek_sequence: 1,
                    fraction: 0.0,
                    hours_per_second: 24.0,
                    composed_at: 100.0,
                    clock_id: "page-A".into(),
                }),
            })
            .unwrap();
        let _ = after_seek.take();
        after_seek
            .push(NativeCommand::MapViews {
                clock_id: Some("page-A".into()),
                views: Vec::new(),
            })
            .unwrap();
        assert_eq!(
            after_seek.take().map_views,
            Some((Some("page-A".into()), Vec::new())),
            "a stamped empty clear for the active origin is admitted to hide the campaign map"
        );
    }

    #[test]
    fn popover_holes_cut_the_window_region_in_window_coordinates() {
        let rect = |x, y, width, height| ViewBounds { x, y, width, height };
        let campaign = rect(100, 50, 800, 600);
        assert_eq!(shape(None, &[], campaign), None, "a campaign map with no hole keeps no region");
        assert_eq!(shape(None, &[rect(0, 0, 90, 40)], campaign), None, "a hole beside the map leaves it whole");
        let cut = shape(None, &[rect(850, 40, 200, 100)], campaign).unwrap();
        assert_eq!(cut, Shape { parts: vec![rect(0, 0, 800, 600)], holes: vec![rect(750, 0, 50, 90)] });

        let root = rect(0, 0, 1000, 800);
        let views = [rect(0, 0, 400, 300), rect(500, 0, 400, 300)];
        let dashboard = shape(Some(&views), &[rect(350, 0, 600, 300), rect(0, 900, 10, 10)], root).unwrap();
        assert_eq!(dashboard, Shape { parts: views.to_vec(), holes: vec![rect(350, 0, 600, 300)] }, "a hole off the window is dropped");
        assert_eq!(shape(Some(&[]), &[], root), Some(Shape { parts: Vec::new(), holes: Vec::new() }), "a dashboard with no view keeps an empty region");
    }
}
