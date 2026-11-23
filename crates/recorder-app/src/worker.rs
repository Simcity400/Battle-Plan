use crate::{
    flags::{BackgroundPriority, FlagContent, FlagImage},
    panel,
    snapshot::{self, OmissionReason, OmittedLoads, Phase, Problem, Snapshot},
    targets, tray,
};
use recorder::controller::{
    self, CaptureOutcome, Control, Discovery, GameProcess, LoadedCampaign, Omission, Probe,
    Recording, Refusal, SetAside, Stage, Waiting,
};
use replay::campaign::Replay;
use replay_settings::recorder_settings::RecorderSettings;
use replay_files::recording_files;
use replay_data::scope::CaptureMode;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard, mpsc},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

pub const STATUS_EVENT: &str = "recorder-status";
pub const SETTINGS_EVENT: &str = "recorder-settings";
pub const RECORDINGS_EVENT: &str = "recorder-recordings";

pub fn recordings_changed(app: &AppHandle) {
    if let Err(error) = app.emit(RECORDINGS_EVENT, ()) {
        eprintln!("Recordings could not reach the panel: {error}");
    }
}

fn settings_used(app: &AppHandle) {
    if let Err(error) = app.emit(SETTINGS_EVENT, ()) {
        eprintln!("Recorder settings could not reach the panel: {error}");
    }
}

#[derive(Default)]
struct Inner {
    snapshot: Snapshot,
    record_requested: bool,
    quit_when_idle: bool,
    quit_after_stop: bool,
    capturing: bool,
    unloading: bool,
    result: bool,
    sticky: Option<Problem>,
    panel_open: bool,
    seen: bool,
    hours: u64,
    earlier: Option<(String, Option<PathBuf>, Option<u64>)>,
    idle_recording: targets::RecordingCache,
    saved_recording: targets::RecordingCache,
    session: Option<(PathBuf, PathBuf)>,
    game_running: bool,
    menu: Option<(bool, bool)>,
    set_aside: Vec<Waiting>,
    names: BTreeMap<String, String>,
    flag_keys: Option<FlagAsk>,
    flag_folders: Option<(PathBuf, PathBuf)>,
    last_flags: BTreeMap<String, FlagImage>,
    leaving: Option<Leaving>,
    closing: bool,
    acting: usize,
}

pub enum Leave {
    Exit,
    Install(app_update::Launch),
}

struct Leaving {
    leave: Leave,
    reply: mpsc::SyncSender<Result<(), String>>,
}

const LEAVE_WAIT: Duration = Duration::from_secs(30);
const SAVING_FOR_UPDATE: &str =
    "BattlePlan Recorder is saving a recording. Restart to update once it has saved.";
const RECORDING_FOR_UPDATE: &str =
    "BattlePlan Recorder is recording. Restart to update once the recording stops.";
const CONNECTED_FOR_UPDATE: &str =
    "BattlePlan Recorder is connected to the game. Restart to update once the game has closed.";
const CLOSING_FOR_UPDATE: &str = "BattlePlan Recorder is closing for an update.";

fn update_busy(inner: &Inner) -> Option<&'static str> {
    if inner.unloading || inner.acting > 0 || inner.snapshot.phase == Phase::Saving {
        Some(SAVING_FOR_UPDATE)
    } else if inner.capturing && inner.snapshot.phase == Phase::Recording {
        Some(RECORDING_FOR_UPDATE)
    } else if inner.capturing {
        Some(CONNECTED_FOR_UPDATE)
    } else {
        None
    }
}

pub struct Acting(Arc<Recorder>);

impl Drop for Acting {
    fn drop(&mut self) {
        self.0.lock().acting -= 1;
    }
}

#[derive(Default)]
pub struct Recorder {
    control: Control,
    inner: Mutex<Inner>,
    wake: Condvar,
    flag_content: Mutex<Option<(PathBuf, Arc<FlagContent>)>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FlagAsk {
    session: PathBuf,
    campaign: String,
    keys: Vec<String>,
}

fn request_flag(app: &AppHandle, (game_dir, user_dir): (PathBuf, PathBuf), ask: FlagAsk) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("recorder-flag".into())
        .spawn(move || {
            let recorder = Arc::clone(app.state::<Arc<Recorder>>().inner());
            let _priority = BackgroundPriority::enter();
            let flag = recorder
                .flag_content(&ask.session, game_dir, user_dir)
                .and_then(|content| content.find(&ask.keys))
                .unwrap_or_else(|error| {
                    eprintln!(
                        "The flag of {} could not be resolved: {error}",
                        ask.campaign
                    );
                    None
                });
            recorder.update(&app, |inner| flag_found(inner, &ask, flag));
        });
    if let Err(error) = spawned {
        eprintln!("The flag could not be resolved: {error}");
    }
}

fn flag_found(inner: &mut Inner, ask: &FlagAsk, flag: Option<FlagImage>) {
    if inner.flag_keys.as_ref() != Some(ask) {
        return;
    }
    match &flag {
        Some(flag) => {
            inner.last_flags.insert(ask.campaign.clone(), flag.clone());
        }
        None => {
            inner.last_flags.remove(&ask.campaign);
        }
    }
    if inner.snapshot.campaign.as_deref() == Some(ask.campaign.as_str()) {
        inner.snapshot.flag = flag;
    }
}

fn file_name(path: &Path) -> Option<String> {
    path.file_stem()?.to_str().map(str::to_owned)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

pub fn save_folder(settings: &RecorderSettings) -> PathBuf {
    settings.effective_save_folder()
}

fn keep_session_root(settings: &Path, root: &Path) {
    if let Err(error) = RecorderSettings::update_at(settings, |settings| {
        settings.keep_session_root(root.to_path_buf())
    }) {
        eprintln!("The session's save folder could not be kept listed: {error}");
    }
}

fn save_root(settings: &RecorderSettings) -> Result<PathBuf, String> {
    let root = save_folder(settings);
    fs::create_dir_all(&root)
        .and_then(|()| root.canonicalize())
        .map_err(|error| format!("Save folder {}: {error}", root.display()))
}

struct GameProbe {
    pid: u32,
    start: u64,
    probe: Result<Probe, String>,
}

impl GameProbe {
    fn settled(&self, start: u64) -> bool {
        self.start == start
            && self
                .probe
                .as_ref()
                .err()
                .is_none_or(|error| controller::probe_withheld(error))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Hold {
    Game,
    Load,
}

#[derive(Debug, PartialEq, Eq)]
struct Held {
    start: u64,
    stopped: Option<Option<(String, i64)>>,
}

impl Held {
    fn game(start: u64) -> Self {
        Self {
            start,
            stopped: None,
        }
    }

    fn released_by(&self, loaded: Option<&LoadedCampaign>, settings: &RecorderSettings) -> bool {
        self.stopped.as_ref().is_some_and(|stopped| {
            loaded.is_some_and(|loaded| {
                settings.records_campaign(&loaded.campaign)
                    && stopped.as_ref().is_none_or(|(campaign, load)| {
                        *campaign != loaded.campaign || *load != loaded.load
                    })
            })
        })
    }
}

#[derive(Default)]
struct Present(Vec<(u32, u64)>);

impl Present {
    fn update(&mut self, found: &[GameProcess]) -> bool {
        let now: Vec<(u32, u64)> = found
            .iter()
            .map(|game| (game.pid, game.start_time))
            .collect();
        let left = self.0.iter().any(|seen| !now.contains(seen));
        self.0 = now;
        left
    }

    fn contains(&self, pid: u32, start: u64) -> bool {
        self.0.contains(&(pid, start))
    }

    fn look(
        &mut self,
        found: &[GameProcess],
        choice: &Discovery,
        followed: Option<(u32, u64)>,
        ended_abandoned: bool,
        retry_due: bool,
    ) -> Look {
        let left = self.update(found);
        Look {
            pass: !ended_abandoned || left || retry_due,
            gone: matches!(choice, Discovery::None)
                && followed.is_none_or(|(pid, start)| !self.contains(pid, start)),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Look {
    pass: bool,
    gone: bool,
}

struct Ended {
    hold: Option<Hold>,
    last_load: Option<(String, i64)>,
    rerouted: bool,
    handover: Option<String>,
}

fn continuation(outcome: &CaptureOutcome, stopped: bool) -> Ended {
    let rerouted = outcome.rerouted && !stopped;
    Ended {
        hold: (outcome.detach_requested && !outcome.process_exited && !rerouted)
            .then_some(Hold::Load),
        last_load: None,
        rerouted,
        handover: None,
    }
}

impl Ended {
    fn held() -> Self {
        Self {
            hold: Some(Hold::Game),
            ..Self::again()
        }
    }

    fn again() -> Self {
        Self {
            hold: None,
            last_load: None,
            rerouted: false,
            handover: None,
        }
    }
}

struct SessionAdmission<'a> {
    settings: &'a Path,
    root: &'a Path,
    target: &'a Path,
    mode: CaptureMode,
    manual: bool,
    manual_campaign: RefCell<Option<String>>,
    first_loaded: Box<dyn Fn() -> Option<String> + 'a>,
    handover: RefCell<Option<String>>,
    used_up: Box<dyn Fn() + 'a>,
}

impl SessionAdmission<'_> {
    fn admit(&self, campaign: &str, recorded: bool) -> Result<(), Refusal> {
        let pressed = self.manual && {
            let mut chosen = self.manual_campaign.borrow_mut();
            if chosen.is_none() {
                *chosen = Some((self.first_loaded)().unwrap_or_else(|| campaign.to_owned()));
            }
            chosen.as_deref() == Some(campaign)
        };
        let Ok(settings) = RecorderSettings::load_from(self.settings) else {
            return Err(Refusal::Settings);
        };
        if !pressed && !settings.records_campaign(campaign) {
            return Err(Refusal::Off);
        }
        let target =
            targets::session_target_at(self.settings, self.root, campaign, recorded, self.mode);
        if let Ok((_, true)) = target {
            (self.used_up)();
        }
        match target {
            Ok((folder, _)) if folder == self.target => Ok(()),
            Ok(_) => {
                if pressed {
                    *self.handover.borrow_mut() = Some(campaign.to_owned());
                }
                Err(Refusal::Elsewhere)
            }
            Err(_) => Err(Refusal::Settings),
        }
    }
}

fn omitted_loads(omitted: &[Omission]) -> Option<OmittedLoads> {
    let shown: Vec<_> = omitted
        .iter()
        .filter(|omission| omission.hours > 0)
        .collect();
    let first = shown.first()?;
    let reason = shown
        .iter()
        .all(|omission| omission.refusal == first.refusal)
        .then_some(match first.refusal {
            Some(Refusal::Off) => OmissionReason::Off,
            Some(Refusal::Elsewhere) => OmissionReason::Elsewhere,
            Some(Refusal::Settings) => OmissionReason::Settings,
            None => OmissionReason::NotAdmitted,
        });
    Some(OmittedLoads {
        loads: shown.len(),
        reason,
    })
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn flag_content(
        &self,
        session: &Path,
        game_dir: PathBuf,
        user_dir: PathBuf,
    ) -> std::io::Result<Arc<FlagContent>> {
        let mut cached = self
            .flag_content
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((owner, content)) = cached.as_ref()
            && owner.as_path() == session
        {
            return Ok(Arc::clone(content));
        }
        let identity = controller::session_identity(session).map_err(std::io::Error::other)?;
        let options = content_reader_layers::ResolveOptions {
            game_dir,
            user_dir,
            cache_dir: replay_settings::recorder_settings::directory(),
        };
        let content = Arc::new(FlagContent::load(&identity, &options)?);
        *cached = Some((session.to_path_buf(), Arc::clone(&content)));
        Ok(content)
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Inner)) {
        let (published, menu, menu_changed, light, tooltip) = {
            let mut inner = self.lock();
            let before = inner.snapshot.clone();
            change(&mut inner);
            if inner.panel_open && (inner.result || inner.sticky.is_some()) {
                inner.seen = true;
            }
            let menu = (
                inner.game_running && !inner.capturing && !inner.unloading,
                inner.capturing
                    && inner.snapshot.phase == Phase::Recording
                    && !inner.snapshot.stopping,
            );
            let menu_changed = inner.menu.replace(menu) != Some(menu);
            let published = (inner.snapshot != before).then(|| inner.snapshot.clone());
            (
                published,
                menu,
                menu_changed,
                inner.snapshot.light(),
                tray::tooltip(&inner.snapshot),
            )
        };
        if let Some(snapshot) = &published
            && let Err(error) = app.emit(STATUS_EVENT, snapshot)
        {
            eprintln!("Recorder status could not reach the panel: {error}");
        }
        if published.is_some() || menu_changed {
            tray::show(app, light, &tooltip, menu.0, menu.1);
        } else {
            tray::follow_theme(app, light);
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.lock().snapshot.clone()
    }

    pub fn set_aside(
        &self,
    ) -> (
        Option<(PathBuf, PathBuf)>,
        Vec<Waiting>,
        BTreeMap<String, String>,
    ) {
        let inner = self.lock();
        (
            inner.session.clone().filter(|_| inner.capturing),
            inner.set_aside.clone(),
            inner.names.clone(),
        )
    }

    pub fn live_target(&self, session: &Path) -> Option<PathBuf> {
        let inner = self.lock();
        inner
            .session
            .as_ref()
            .filter(|(live, _)| inner.capturing && live == session)
            .map(|(_, target)| target.clone())
    }

    pub fn flags(&self) -> BTreeMap<String, FlagImage> {
        self.lock().last_flags.clone()
    }

    pub fn act_on_recording(&self, target: &Path, campaign: &str, discard: bool) -> bool {
        let running = {
            let inner = self.lock();
            inner.capturing
                && inner
                    .session
                    .as_ref()
                    .is_some_and(|(_, live)| live == target)
        };
        running && self.control.set_aside(campaign.to_owned(), discard)
    }

    pub fn start(self: &Arc<Self>, app: AppHandle) -> std::io::Result<()> {
        let recorder = Arc::clone(self);
        std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || recorder.run(&app))
            .map(|_| ())
    }

    pub fn record(&self) {
        self.lock().record_requested = true;
        self.wake.notify_all();
    }

    pub fn stop(&self, app: &AppHandle, discard: bool) {
        let mut accepted = false;
        self.update(app, |inner| {
            accepted = accept_stop(inner, &self.control, discard);
        });
        if accepted {
            self.wake.notify_all();
        }
    }

    fn recording(&self) -> bool {
        let inner = self.lock();
        inner.capturing && inner.snapshot.phase == Phase::Recording && !inner.snapshot.stopping
    }

    pub fn save(&self, app: &AppHandle) {
        if self.recording() {
            self.stop(app, false);
        }
    }

    pub fn ask_discard(&self, app: &AppHandle) {
        if self.recording() {
            panel::open(app, None, panel::Opening::DiscardPrompt);
        }
    }

    pub fn quit(&self, app: &AppHandle) {
        let mut inner = self.lock();
        if !inner.capturing && !inner.unloading {
            drop(inner);
            app.exit(0);
        } else if !inner.capturing {
            inner.quit_when_idle = true;
            drop(inner);
            self.wake.notify_all();
        } else if inner.snapshot.phase == Phase::Recording && !inner.snapshot.stopping {
            inner.quit_after_stop = true;
            drop(inner);
            panel::open(app, None, panel::Opening::QuitPrompt);
        } else {
            inner.quit_when_idle = true;
            let stopping = inner.snapshot.stopping;
            drop(inner);
            if !stopping {
                self.control.stop(false);
            }
            self.wake.notify_all();
        }
    }

    pub fn cancel_stop_prompt(&self) {
        let mut inner = self.lock();
        if !self.control.stopping() {
            inner.quit_after_stop = false;
        }
    }

    pub fn panel_opened(&self, app: &AppHandle) {
        self.update(app, |inner| inner.panel_open = true);
    }

    pub fn panel_closed(&self, app: &AppHandle) {
        let stopping = self.control.stopping();
        self.update(app, |inner| {
            inner.panel_open = false;
            if !stopping {
                inner.quit_after_stop = false;
            }
            if std::mem::take(&mut inner.seen) {
                inner.result = false;
                inner.sticky = None;
            }
        });
        self.wake.notify_all();
    }

    pub fn settings_changed(&self) {
        self.wake.notify_all();
    }

    pub fn update_blocked(&self) -> Option<&'static str> {
        let inner = self.lock();
        update_busy(&inner).or(inner.closing.then_some(CLOSING_FOR_UPDATE))
    }

    pub fn leave(&self, leave: Leave) -> Result<(), String> {
        let (reply, answer) = mpsc::sync_channel(1);
        {
            let mut inner = self.lock();
            if inner.closing && matches!(leave, Leave::Exit) {
                return Ok(());
            }
            if let Some(reason) = update_busy(&inner) {
                return Err(reason.into());
            }
            if inner.closing || inner.leaving.is_some() {
                return Err(CLOSING_FOR_UPDATE.into());
            }
            inner.leaving = Some(Leaving { leave, reply });
        }
        self.wake.notify_all();
        match answer.recv_timeout(LEAVE_WAIT) {
            Ok(result) => result,
            Err(_) => {
                if self.lock().leaving.take().is_some() {
                    return Err(
                        "BattlePlan Recorder stayed busy, so it didn’t close for the update.".into(),
                    );
                }
                answer.recv().unwrap_or_else(|_| {
                    Err("BattlePlan Recorder stopped before it answered the update.".into())
                })
            }
        }
    }

    pub fn acting(self: &Arc<Self>) -> Result<Acting, String> {
        let mut inner = self.lock();
        if inner.closing || inner.leaving.is_some() {
            return Err(CLOSING_FOR_UPDATE.into());
        }
        inner.acting += 1;
        Ok(Acting(Arc::clone(self)))
    }

    fn leave_now(&self, app: &AppHandle, leaving: Leaving) -> bool {
        let refused = {
            let mut inner = self.lock();
            let refused = (inner.acting > 0).then_some(SAVING_FOR_UPDATE);
            inner.closing = refused.is_none();
            refused
        };
        if let Some(reason) = refused {
            let _ = leaving.reply.send(Err(reason.into()));
            return false;
        }
        let result = match leaving.leave {
            Leave::Exit => Ok(()),
            Leave::Install(launch) => launch(),
        };
        let closing = result.is_ok();
        if !closing {
            self.lock().closing = false;
        }
        let _ = leaving.reply.send(result);
        if closing {
            app.exit(0);
        }
        closing
    }

    fn wait(&self, timeout: Duration) {
        let inner = self.lock();
        if inner.record_requested {
            return;
        }
        let _ = self
            .wake
            .wait_timeout(inner, timeout)
            .unwrap_or_else(|error| error.into_inner());
    }

    fn exit_requested(&self) -> bool {
        let inner = self.lock();
        inner.quit_when_idle || inner.quit_after_stop
    }

    fn run(&self, app: &AppHandle) {
        let mut held: Option<Held> = None;
        let mut game: Option<GameProbe> = None;
        let mut rerouted = false;
        let mut handover: Option<(u64, String)> = None;
        let mut ended_abandoned = false;
        let mut abandoned_retry = AbandonedRetry::default();
        let mut present = Present::default();
        loop {
            {
                let inner = self.lock();
                if inner.unloading {
                    let _ = self
                        .wake
                        .wait_timeout(inner, Duration::from_millis(100))
                        .unwrap_or_else(|error| error.into_inner());
                    continue;
                }
            }
            let leaving = self.lock().leaving.take();
            if let Some(leaving) = leaving
                && self.leave_now(app, leaving)
            {
                return;
            }
            if self.exit_requested() {
                app.exit(0);
                return;
            }
            let settings = RecorderSettings::load().unwrap_or_else(|error| {
                eprintln!("Recorder settings could not be read: {error}");
                RecorderSettings::default()
            });
            let manual = std::mem::take(&mut self.lock().record_requested);
            let listed = controller::game_processes().map(|found| {
                let choice = controller::choose_game(&found);
                let followed = game.as_ref().map(|game| (game.pid, game.start));
                let look = present.look(
                    &found,
                    &choice,
                    followed,
                    ended_abandoned,
                    abandoned_retry.due(),
                );
                (choice, look)
            });
            if let Ok((_, look)) = &listed {
                if look.pass {
                    abandoned_retry.after(self.end_abandoned(app, &settings));
                    ended_abandoned = true;
                }
                if look.gone {
                    held = None;
                    handover = None;
                    game = None;
                }
            }
            match listed.map(|(choice, _)| choice) {
                Err(error) => self.idle(
                    app,
                    &settings,
                    true,
                    None,
                    Some(Problem::failure(
                        controller::problem_text(&error, false),
                        error,
                    )),
                ),
                Ok(Discovery::None) => self.idle(app, &settings, false, None, None),
                Ok(Discovery::Several(several)) => self.idle(
                    app,
                    &settings,
                    true,
                    None,
                    Some(Problem::failure(several.to_string(), several.to_string())),
                ),
                Ok(Discovery::One(selected)) => {
                    let start = selected.start_time;
                    if manual || held.as_ref().is_some_and(|held| held.start != start) {
                        held = None;
                    }
                    if !game.as_ref().is_some_and(|game| game.settled(start)) {
                        let probe = Probe::open(&selected);
                        if let Err(error) = &probe
                            && game.as_ref().is_none_or(|game| {
                                game.start != start || game.probe.as_ref().err() != Some(error)
                            })
                        {
                            eprintln!("The running game could not be read: {error}");
                        }
                        game = Some(GameProbe {
                            pid: selected.pid,
                            start,
                            probe,
                        });
                    }
                    let probe = game.as_ref().and_then(|game| game.probe.as_ref().ok());
                    let loaded = probe.and_then(Probe::loaded);
                    if handover.as_ref().is_some_and(|(game, _)| *game != start) {
                        handover = None;
                    }
                    let manual = manual
                        || loaded.as_ref().is_some_and(|loaded| {
                            handover
                                .take_if(|(_, campaign)| *campaign == loaded.campaign)
                                .is_some()
                        });
                    if std::mem::take(&mut rerouted) && loaded.is_none() && !manual {
                        held = Some(Held::game(start));
                    }
                    if held
                        .as_ref()
                        .is_some_and(|held| held.released_by(loaded.as_ref(), &settings))
                    {
                        held = None;
                    }
                    let wanted = match &loaded {
                        Some(loaded) => settings.records_campaign(&loaded.campaign),
                        None => settings.record_every_campaign,
                    };
                    if held.is_none() && (manual || wanted) {
                        let started = loaded
                            .as_ref()
                            .map(|loaded| (loaded.campaign.clone(), loaded.load));
                        let ended = self.capture(app, selected, &settings, manual, loaded, probe);
                        held = ended.hold.map(|hold| match hold {
                            Hold::Game => Held::game(start),
                            Hold::Load => Held {
                                start,
                                stopped: Some(ended.last_load.or(started)),
                            },
                        });
                        rerouted = ended.rerouted;
                        handover = ended.handover.map(|campaign| (start, campaign));
                        continue;
                    }
                    self.idle(app, &settings, true, loaded.as_ref(), None);
                }
            }
            self.wait(Duration::from_secs(1));
        }
    }

    fn end_abandoned(&self, app: &AppHandle, settings: &RecorderSettings) -> bool {
        let mut retry = false;
        for root in settings.session_roots() {
            let root = root.canonicalize().unwrap_or(root);
            let pass = controller::end_abandoned_sessions(&root);
            for error in pass.errors {
                eprintln!("An earlier session could not be ended: {error}");
            }
            retry |= pass.retry;
        }
        recordings_changed(app);
        retry
    }

    fn idle(
        &self,
        app: &AppHandle,
        settings: &RecorderSettings,
        game_running: bool,
        loaded: Option<&LoadedCampaign>,
        problem: Option<Problem>,
    ) {
        let named = loaded.and_then(|loaded| {
            let root = save_root(settings).ok()?;
            let folder = targets::current_folder(settings, &root, &loaded.campaign);
            let recording = self.lock().idle_recording.get(&folder, &loaded.campaign);
            Some((loaded.clone(), recording))
        });
        let saved = {
            let mut guard = self.lock();
            let inner = &mut *guard;
            let snapshot = &inner.snapshot;
            match (inner.result, &snapshot.recording, snapshot.campaign.clone()) {
                (true, Some(recording), Some(campaign)) => Path::new(recording)
                    .parent()
                    .map(Path::to_path_buf)
                    .and_then(|folder| inner.saved_recording.get(&folder, &campaign)),
                _ => None,
            }
        };
        self.update(app, |inner| {
            inner.game_running = game_running;
            let automatic = settings.record_every_campaign;
            if inner.result {
                inner.snapshot.automatic = automatic;
                if let Some(saved) = saved {
                    inner.snapshot.name = file_name(&saved);
                    inner.snapshot.recording = Some(saved.display().to_string());
                }
                return;
            }
            inner.snapshot = Snapshot {
                phase: if game_running {
                    Phase::NotRecording
                } else {
                    Phase::Waiting
                },
                problem: problem.or_else(|| inner.sticky.clone()),
                automatic,
                ..Snapshot::default()
            };
            if let Some((loaded, recording)) = named {
                inner.snapshot.saved_bytes = recording
                    .as_ref()
                    .and_then(|recording| fs::metadata(recording).ok())
                    .map(|metadata| metadata.len());
                inner.snapshot.continued = recording.is_some();
                inner.snapshot.flag = inner.last_flags.get(&loaded.campaign).cloned();
                inner.snapshot.campaign = Some(loaded.campaign);
                inner.snapshot.name = recording.as_deref().and_then(file_name);
                inner.snapshot.recording = recording.map(|path| path.display().to_string());
                inner.snapshot.hour = Some(loaded.hour);
            }
        });
    }

    fn capture(
        &self,
        app: &AppHandle,
        selected: GameProcess,
        settings: &RecorderSettings,
        manual: bool,
        loaded: Option<LoadedCampaign>,
        probe: Option<&Probe>,
    ) -> Ended {
        let game = match controller::open_game(&selected) {
            Ok(Some(game)) => game,
            Ok(None) => return Ended::again(),
            Err(error) => {
                self.fail(app, error, false);
                return Ended::held();
            }
        };
        let folders = (|| -> Result<(PathBuf, PathBuf, PathBuf), String> {
            let root = save_root(settings)?;
            let target = match &loaded {
                Some(loaded) => {
                    let (target, used_up) =
                        targets::session_target(&root, &loaded.campaign, settings.capture_mode)?;
                    if used_up {
                        settings_used(app);
                    }
                    target
                }
                None => root.clone(),
            };
            let user = match &settings.user_folder {
                Some(folder) => folder.clone(),
                None => controller::resolve_user_directory()?,
            };
            let user = user
                .canonicalize()
                .map_err(|error| format!("Paradox user folder {}: {error}", user.display()))?;
            Ok((root, target, user))
        })();
        let (root, target, user) = match folders {
            Ok(folders) => folders,
            Err(error) => {
                self.fail(app, error, false);
                return Ended::held();
            }
        };
        self.control.reset();
        let automatic = settings.record_every_campaign;
        let opening = loaded.as_ref().map(|loaded| {
            let recording = targets::recording(&target, &loaded.campaign);
            let bytes = recording
                .as_ref()
                .and_then(|recording| fs::metadata(recording).ok())
                .map(|metadata| metadata.len());
            (loaded.clone(), recording, bytes)
        });
        let flag_folders = probe
            .and_then(Probe::game_dir)
            .map(|game_dir| (game_dir, user.clone()));
        self.update(app, |inner| {
            inner.capturing = true;
            inner.game_running = true;
            inner.result = false;
            inner.sticky = None;
            inner.seen = false;
            inner.hours = 0;
            inner.earlier = None;
            inner.session = None;
            inner.set_aside.clear();
            inner.names.clear();
            inner.flag_keys = None;
            inner.flag_folders = flag_folders;
            inner.snapshot = Snapshot {
                phase: Phase::Waiting,
                automatic,
                capture_mode: Some(settings.capture_mode.name()),
                ..Snapshot::default()
            };
            if let Some((loaded, recording, bytes)) = opening {
                inner.earlier = Some((loaded.campaign.clone(), recording.clone(), bytes));
                inner.snapshot.flag = inner.last_flags.get(&loaded.campaign).cloned();
                let snapshot = &mut inner.snapshot;
                snapshot.phase = Phase::Recording;
                snapshot.started_at = Some(now_ms());
                snapshot.campaign = Some(loaded.campaign);
                snapshot.continued = recording.is_some();
                snapshot.recording = recording.map(|path| path.display().to_string());
                snapshot.hour = Some(loaded.hour);
                snapshot.saved_bytes = bytes;
            }
        });
        let settings_path = RecorderSettings::path();
        let session_dir: RefCell<Option<PathBuf>> = RefCell::new(None);
        let admission = SessionAdmission {
            settings: &settings_path,
            root: &root,
            target: &target,
            mode: settings.capture_mode,
            manual,
            manual_campaign: RefCell::new(
                manual
                    .then(|| loaded.as_ref().map(|loaded| loaded.campaign.clone()))
                    .flatten(),
            ),
            first_loaded: Box::new(|| {
                session_dir
                    .borrow()
                    .as_deref()
                    .and_then(controller::first_loaded_campaign)
            }),
            handover: RefCell::new(None),
            used_up: Box::new(|| settings_used(app)),
        };
        let admit = |campaign: &str, recorded: bool| admission.admit(campaign, recorded);
        let last_load = RefCell::new(None);
        let current = || {
            let named = probe
                .and_then(Probe::loaded)
                .map(|loaded| (loaded.campaign, loaded.load));
            if named.is_some() {
                last_load.replace(named.clone());
            }
            named
        };
        let last_auto_save = Cell::new(settings.auto_save);
        let auto_save = || match RecorderSettings::load_from(&settings_path) {
            Ok(read) => {
                last_auto_save.set(read.auto_save);
                read.auto_save
            }
            Err(error) => {
                eprintln!(
                    "Recorder settings could not be read; Auto save stays as it was: {error}"
                );
                last_auto_save.get()
            }
        };
        let started = Cell::new(false);
        let finished = RefCell::new(None);
        let writes_into = RefCell::new(target.clone());
        let stage = |stage: Stage<'_>| match stage {
            Stage::Preparing => {}
            Stage::TakingOver { session, target } => {
                println!(
                    "Recording: taking over the recording left by a recorder that stopped ({}).",
                    session.display()
                );
                writes_into.replace(target.to_path_buf());
            }
            Stage::Restarting { reason } => {
                println!(
                    "Recording: ending the recording left by a recorder that stopped, then recording afresh: {reason}."
                );
            }
            Stage::Recording {
                session,
                status,
                names,
            } => {
                started.set(true);
                let target = writes_into.borrow();
                if session_dir.borrow().is_none() {
                    session_dir.replace(Some(session.to_path_buf()));
                    self.lock().session = Some((session.to_path_buf(), target.clone()));
                    keep_session_root(&settings_path, &root);
                }
                if !self.control.stopping() {
                    self.live(app, &target, session, status, names, probe);
                }
            }
            Stage::Saving { .. } => self.update(app, |inner| {
                inner.snapshot.phase = Phase::Saving;
                inner.snapshot.problem = None;
            }),
            Stage::SetAside { waiting, done } => self.set_aside_changed(app, waiting, done),
            Stage::Finished { result } => {
                self.lock().unloading = true;
                finished.replace(Some(self.finish(app, result, started.get())));
            }
            Stage::Unloading(observation) => self.observe_unload(app, observation),
        };
        let recording = Recording {
            output: &root,
            target: &target,
            user: &user,
            capture_mode: settings.capture_mode,
            admit: &admit,
            loaded: &current,
            opening: loaded
                .as_ref()
                .map(|loaded| (loaded.campaign.as_str(), loaded.load)),
            auto_save: &auto_save,
            names_wanted: &|| self.lock().panel_open,
        };
        let result = controller::record(game, &recording, &self.control, &stage);
        let mut ended = finished.into_inner().unwrap_or_else(|| {
            self.finish(app, result.as_ref().map_err(String::as_str), started.get())
        });
        if ended.rerouted {
            ended.handover = admission.handover.take();
        }
        ended.last_load = last_load.into_inner();
        ended
    }

    fn set_aside_changed(&self, app: &AppHandle, waiting: Option<&[Waiting]>, done: &[SetAside]) {
        let failure = done.iter().find_map(|done| match done {
            SetAside::Failed(campaign, error) => Some(format!("Campaign {campaign}: {error}")),
            _ => None,
        });
        if let Some(error) = &failure {
            eprintln!("Recording: {error}");
        }
        let jobs = app.state::<Arc<crate::compress::Jobs>>();
        for done in done {
            if let SetAside::Saved(_, Some(path)) = done {
                jobs.saved(path);
            }
        }
        self.update(app, |inner| {
            if let Some(waiting) = waiting {
                inner.set_aside = waiting.to_vec();
            }
            if let Some(error) = failure {
                let problem = Problem::failure(controller::problem_text(&error, true), error);
                inner.sticky = Some(problem.clone());
                inner.snapshot.problem = Some(problem);
            }
        });
        recordings_changed(app);
    }

    fn live(
        &self,
        app: &AppHandle,
        target: &Path,
        session: &Path,
        status: Option<&Value>,
        names: &BTreeMap<String, String>,
        probe: Option<&Probe>,
    ) {
        let live = status.map(snapshot::read_live).unwrap_or_default();
        let mut live = loaded_live(live, probe.and_then(Probe::loaded));
        let (recording, earlier) = match &live.campaign {
            Some(campaign) => {
                let cached = self.lock().earlier.clone();
                match cached {
                    Some((id, recording, bytes)) if &id == campaign => (recording, bytes),
                    _ => {
                        let recording = targets::recording(target, campaign);
                        let bytes = recording
                            .as_ref()
                            .and_then(|recording| fs::metadata(recording).ok())
                            .map(|metadata| metadata.len());
                        self.lock().earlier = Some((campaign.clone(), recording.clone(), bytes));
                        (recording, bytes)
                    }
                }
            }
            None => (None, None),
        };
        let name = live
            .campaign
            .as_ref()
            .and_then(|campaign| names.get(campaign))
            .cloned();
        let flag_request = match (&live.campaign, live.flag_keys.take()) {
            (Some(campaign), Some(keys)) => {
                let ask = FlagAsk {
                    session: session.to_path_buf(),
                    campaign: campaign.clone(),
                    keys,
                };
                let mut inner = self.lock();
                if inner.flag_keys.as_ref() == Some(&ask) {
                    None
                } else {
                    inner.flag_keys = Some(ask.clone());
                    inner.flag_folders.clone().map(|folders| (folders, ask))
                }
            }
            _ => None,
        };
        if let Some((folders, ask)) = flag_request {
            request_flag(app, folders, ask);
        }
        self.update(app, |inner| {
            fold_live(inner, live, session, names, name, recording, earlier);
        });
    }

    fn finish(
        &self,
        app: &AppHandle,
        result: Result<&CaptureOutcome, &str>,
        started: bool,
    ) -> Ended {
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                self.fail(app, error.to_owned(), started);
                return Ended::held();
            }
        };
        let ended = continuation(&outcome, self.control.stopping());
        recordings_changed(app);
        if outcome.discarded {
            self.update(app, |inner| {
                inner.capturing = false;
                inner.session = None;
                inner.snapshot.capture_mode = None;
                inner.result = true;
                inner.set_aside.clear();
                let snapshot = &mut inner.snapshot;
                snapshot.phase = Phase::Discarded;
                snapshot.stopping = false;
                snapshot.problem = outcome.failure_report.as_deref().map(discarded_failure);
                snapshot.spool_bytes = None;
            });
            return ended;
        }
        let jobs = app.state::<Arc<crate::compress::Jobs>>();
        for path in &outcome.replays {
            if path.parent().and_then(Path::file_name).is_none_or(|folder| folder != ".unsaved") {
                jobs.saved(path);
            }
        }
        let campaign = self.lock().snapshot.campaign.clone();
        let campaign_of = |path: &Path| {
            recording_files::campaign_of(path)
                .ok()
                .flatten()
                .and_then(|id| String::from_utf8(id).ok())
        };
        let saved = outcome
            .replays
            .iter()
            .find(|path| campaign.is_some() && campaign_of(path) == campaign)
            .or(outcome.replays.last())
            .cloned();
        let omitted = omitted_loads(&outcome.omitted);
        let unsaved = campaign
            .as_ref()
            .is_some_and(|campaign| outcome.waiting.contains(campaign));
        let saved = saved.filter(|_| !unsaved);
        let unrenamed = (!outcome.unrenamed.is_empty()).then(|| {
            Problem::caveat(
                "Saved, but the file kept its old name. It’s renamed at the next save.",
                outcome.unrenamed.join("\n"),
            )
        });
        let kept = outcome.undiscarded.then(|| {
            Problem::failure(
                "Nothing was discarded: the campaign in progress couldn’t be identified.",
                "Stop couldn’t read which campaign was loaded; every hour was kept.",
            )
        });
        match saved {
            Some(path) => {
                let span = Replay::read_hour_span(&path).ok().flatten();
                let bytes = fs::metadata(&path).ok().map(|metadata| metadata.len());
                self.update(app, |inner| {
                    inner.capturing = false;
                    inner.session = None;
                    inner.snapshot.capture_mode = None;
                    inner.result = true;
                    inner.set_aside.clear();
                    let snapshot = &mut inner.snapshot;
                    snapshot.phase = Phase::Saved;
                    snapshot.omitted = omitted;
                    snapshot.campaign = campaign_of(&path).or(snapshot.campaign.take());
                    snapshot.name = file_name(&path);
                    snapshot.recording = Some(path.display().to_string());
                    snapshot.first_hour = span.map(|(first, _)| first);
                    snapshot.hour = span.map(|(_, last)| last).or(snapshot.hour);
                    snapshot.saved_bytes = bytes;
                    snapshot.spool_bytes = None;
                    snapshot.continued = true;
                    snapshot.stopping = false;
                    snapshot.problem = kept.or(unrenamed);
                });
            }
            None => self.update(app, |inner| {
                inner.capturing = false;
                inner.session = None;
                inner.set_aside.clear();
                inner.snapshot.capture_mode = None;
                inner.snapshot.stopping = false;
                if unsaved {
                    inner.result = true;
                    inner.snapshot.phase = Phase::Unsaved;
                    inner.snapshot.omitted = omitted;
                } else if omitted.is_some() {
                    inner.result = true;
                    inner.snapshot.phase = Phase::NotRecording;
                    inner.snapshot.omitted = omitted;
                }
                if let Some(kept) = kept {
                    inner.sticky = Some(kept.clone());
                    inner.snapshot.problem = Some(kept);
                }
            }),
        }
        ended
    }

    fn observe_unload(&self, app: &AppHandle, observation: controller::UnloadObservation) {
        self.update(app, |inner| inner.unloading = true);
        let app_copy = app.clone();
        let recorder = Arc::clone(app.state::<Arc<Recorder>>().inner());
        let spawned = std::thread::Builder::new()
            .name("recorder-unload".into())
            .spawn(move || {
                let result = observation.finish();
                recorder.update(&app_copy, |inner| unload_finished(inner, result));
                recordings_changed(&app_copy);
                recorder.wake.notify_all();
            });
        if let Err(error) = spawned {
            self.update(app, |inner| unload_finished(inner, Err(format!(
                "Couldn’t confirm the game released the recorder: {error}. Quit the game before recording again."
            ))));
            self.wake.notify_all();
        }
    }

    fn fail(&self, app: &AppHandle, error: String, started: bool) {
        eprintln!("Recording: {error}");
        let problem = Problem::failure(controller::problem_text(&error, started), error);
        recordings_changed(app);
        self.update(app, |inner| {
            inner.capturing = false;
            inner.session = None;
            inner.set_aside.clear();
            inner.snapshot.capture_mode = None;
            inner.sticky = Some(problem.clone());
            let snapshot = &mut inner.snapshot;
            snapshot.phase = Phase::NotRecording;
            snapshot.stopping = false;
            snapshot.problem = Some(problem);
        });
    }
}

fn discarded_failure(report: &Path) -> Problem {
    Problem::caveat(
        "Session discarded. The capture had failed, so a failure report was kept.",
        format!(
            "Nothing of the session was kept. The failure report holds only its diagnostics: {}",
            report.display()
        ),
    )
}

fn accept_stop(inner: &mut Inner, control: &Control, discard: bool) -> bool {
    if !inner.capturing {
        return false;
    }
    control.stop(discard);
    inner.snapshot.stopping = true;
    if !discard {
        inner.snapshot.phase = Phase::Saving;
        inner.snapshot.problem = None;
    }
    true
}

fn loaded_live(mut live: snapshot::Live, loaded: Option<LoadedCampaign>) -> snapshot::Live {
    if let Some(loaded) = loaded
        && live.campaign.as_deref() != Some(loaded.campaign.as_str())
    {
        live.campaign = Some(loaded.campaign);
        live.hour = Some(loaded.hour);
        live.flag_keys = None;
    }
    live
}

fn fold_live(
    inner: &mut Inner,
    live: snapshot::Live,
    session: &Path,
    names: &BTreeMap<String, String>,
    name: Option<String>,
    recording: Option<PathBuf>,
    earlier: Option<u64>,
) {
    if let Some(hours) = live.hours {
        inner.hours = hours;
    }
    inner.names.clone_from(names);
    let snapshot = &mut inner.snapshot;
    snapshot.session = Some(session.display().to_string());
    if let Some(campaign) = live.campaign {
        if snapshot.phase != Phase::Recording {
            snapshot.phase = Phase::Recording;
            snapshot.started_at = Some(now_ms());
        }
        if snapshot.campaign.as_deref() != Some(campaign.as_str()) {
            snapshot.flag = inner.last_flags.get(&campaign).cloned();
        }
        snapshot.campaign = Some(campaign);
        snapshot.name = name;
        snapshot.continued = recording.is_some();
        snapshot.recording = recording.map(|path| path.display().to_string());
        snapshot.hour = live.hour.or(snapshot.hour);
        snapshot.saved_bytes = earlier;
        snapshot.spool_bytes = live.spool_bytes;
    }
    if live.build_warning.is_some() {
        snapshot.build_warning = live.build_warning;
    }
    snapshot.behind = live.behind;
    snapshot.problem = live.problem.or_else(|| inner.sticky.clone());
}

fn unload_finished(inner: &mut Inner, result: Result<(), String>) {
    inner.unloading = false;
    if let Err(error) = result {
        let mut problem = inner
            .snapshot
            .problem
            .clone()
            .or_else(|| inner.sticky.clone())
            .unwrap_or(Problem::caveat("Recording finished with a warning.", ""));
        if !problem.detail.is_empty() {
            problem.detail.push('\n');
        }
        problem.detail.push_str(&error);
        inner.snapshot.problem = Some(problem.clone());
        inner.sticky = Some(problem);
        inner.seen = false;
    }
}

#[derive(Default)]
struct AbandonedRetry {
    at: Option<Instant>,
    wait: Duration,
}

impl AbandonedRetry {
    const FIRST: Duration = Duration::from_secs(5);
    const LONGEST: Duration = Duration::from_secs(300);

    fn due(&self) -> bool {
        self.at.is_some_and(|at| Instant::now() >= at)
    }

    fn after(&mut self, retry: bool) {
        if retry {
            self.wait = (self.wait * 2).clamp(Self::FIRST, Self::LONGEST);
            self.at = Some(Instant::now() + self.wait);
        } else {
            *self = Self::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_settings::recorder_settings::NextLoad;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";
    const OTHER: &str = "00000000-0000-4000-8000-000000000001";

    fn game(pid: u32, start_time: u64, state: controller::ProcessState) -> GameProcess {
        GameProcess {
            pid,
            start_time,
            state,
        }
    }

    #[test]
    fn a_stopped_load_stays_held_while_its_game_freezes_and_resumes() {
        use controller::ProcessState::{Frozen, Running};
        let on = RecorderSettings {
            record_every_campaign: true,
            ..RecorderSettings::default()
        };
        let mut present = Present::default();
        let mut held = Some(Held {
            start: 70,
            stopped: Some(Some((CAMPAIGN.to_owned(), 100))),
        });
        for found in [
            [game(7, 70, Running)],
            [game(7, 70, Frozen)],
            [game(7, 70, Running)],
        ] {
            let choice = controller::choose_game(&found);
            let look = present.look(&found, &choice, Some((7, 70)), true, false);
            if look.gone {
                held = None;
            }
        }
        assert!(
            held.is_some_and(|held| !held.released_by(Some(&loaded(CAMPAIGN, 100)), &on)),
            "a freeze released the user's Stop"
        );
    }

    #[test]
    fn a_frozen_orphans_game_leaving_the_listing_requests_recovery() {
        use controller::ProcessState::Frozen;
        let mut present = Present::default();
        let orphan = [game(9, 90, Frozen)];
        let choice = controller::choose_game(&orphan);
        assert!(present.look(&orphan, &choice, None, false, false).pass);
        assert!(!present.look(&orphan, &choice, None, true, false).pass);
        let look = present.look(&[], &Discovery::None, None, true, false);
        assert!(look.pass, "the orphan's game exited, freeing its session");
    }

    #[test]
    fn the_abandoned_session_pass_runs_with_no_game_or_two_running_games() {
        use controller::ProcessState::{Exiting, Frozen, Running};
        let mut present = Present::default();
        let exiting = [game(5, 50, Exiting)];
        let choice = controller::choose_game(&exiting);
        assert!(!present.look(&exiting, &choice, None, true, false).pass);
        assert!(present.look(&[], &Discovery::None, None, true, false).pass);
        assert!(present.look(&[], &Discovery::None, None, true, true).pass);

        let mut present = Present::default();
        let two = [game(1, 10, Running), game(2, 20, Running)];
        let three = [two[0], two[1], game(3, 30, Frozen)];
        let several = controller::choose_game(&three);
        let still = controller::choose_game(&two);
        assert!(present.look(&three, &several, None, false, false).pass);
        assert!(!present.look(&three, &several, None, true, false).pass);
        assert!(present.look(&two, &still, None, true, false).pass);
        assert!(present.look(&two, &still, None, true, true).pass);
    }

    #[test]
    fn unload_followup_keeps_saved_state_and_prior_failures_without_losing_record_requests() {
        for warning in [false, true] {
            let mut inner = Inner::default();
            inner.snapshot.phase = Phase::Saved;
            inner.result = true;
            inner.unloading = true;
            inner.record_requested = true;
            if warning {
                inner.snapshot.problem = Some(Problem::failure(
                    "Earlier capture failure",
                    "synthetic drain failure",
                ));
            }
            unload_finished(
                &mut inner,
                if warning {
                    Err("collector remains loaded".into())
                } else {
                    Ok(())
                },
            );
            assert!(!inner.unloading && !inner.capturing);
            assert!(inner.record_requested && inner.result);
            assert_eq!(inner.snapshot.phase, Phase::Saved);
            if warning {
                let problem = inner.snapshot.problem.unwrap();
                assert_eq!(problem.text, "Earlier capture failure");
                assert!(problem.detail.contains("synthetic drain failure"));
                assert!(problem.detail.contains("collector remains loaded"));
                assert_eq!(inner.sticky.unwrap().detail, problem.detail);
            } else {
                assert!(inner.snapshot.problem.is_none());
            }
        }
    }

    #[test]
    fn a_stop_after_the_capture_ended_keeps_its_result_and_stops_no_later_capture() {
        let control = Control::new();
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        assert!(accept_stop(&mut inner, &control, false));
        assert!(control.stopping() && !control.discarding());
        assert_eq!(inner.snapshot.phase, Phase::Saving);
        assert!(inner.snapshot.stopping);
        let control = Control::new();
        let mut inner = Inner {
            result: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Saved;
        let ended = inner.snapshot.clone();
        assert!(!accept_stop(&mut inner, &control, false));
        assert_eq!(inner.snapshot, ended);
        assert!(!control.stopping());
    }

    #[test]
    fn an_unread_status_shows_no_spool_size_and_keeps_the_sessions_hour_count() {
        let session = Path::new("session");
        let names = BTreeMap::new();
        let mut inner = Inner::default();
        let read = snapshot::read_live(
            &serde_json::json!({"campaign": CAMPAIGN, "hours": 12, "last_hour": 9, "spool_bytes": 4096}),
        );
        fold_live(&mut inner, read, session, &names, None, None, None);
        assert_eq!((inner.hours, inner.snapshot.spool_bytes), (12, Some(4096)));
        let unread = snapshot::Live {
            campaign: Some(CAMPAIGN.into()),
            hour: Some(10),
            ..snapshot::Live::default()
        };
        fold_live(&mut inner, unread, session, &names, None, None, None);
        assert_eq!(inner.snapshot.spool_bytes, None);
        assert_eq!(inner.hours, 12);
        assert_eq!(inner.snapshot.phase, Phase::Recording);
    }

    #[test]
    fn a_campaign_loaded_since_the_last_written_hour_is_the_live_one() {
        const OTHER: &str = "00000000-0000-4000-8000-000000000001";
        let status = serde_json::json!({
            "campaign": CAMPAIGN, "hours": 12, "last_hour": 9, "spool_bytes": 4096,
            "player": {"tag":"OWN","cosmetic":"","original":"OWN","group":"synthetic_group"},
        });
        let same = loaded_live(snapshot::read_live(&status), Some(loaded(CAMPAIGN, 7)));
        assert_eq!(
            (same.campaign.as_deref(), same.hour),
            (Some(CAMPAIGN), Some(9))
        );
        assert!(same.flag_keys.is_some());
        let other = loaded_live(snapshot::read_live(&status), Some(loaded(OTHER, 8)));
        assert_eq!(
            (other.campaign.as_deref(), other.hour),
            (Some(OTHER), Some(1))
        );
        assert_eq!(other.flag_keys, None);
        assert_eq!((other.hours, other.spool_bytes), (Some(12), Some(4096)));
        let unread = loaded_live(snapshot::read_live(&status), None);
        assert_eq!(unread.campaign.as_deref(), Some(CAMPAIGN));
    }

    #[test]
    fn a_flag_lookup_answers_only_the_session_that_asked() {
        let flag = |shade| FlagImage {
            width: 1,
            height: 1,
            pixels: vec![shade, 0, 0, 255],
        };
        let ask = |session: &str| FlagAsk {
            session: PathBuf::from(session),
            campaign: CAMPAIGN.into(),
            keys: vec!["OWN".into()],
        };
        let mut inner = Inner::default();
        inner.snapshot.campaign = Some(CAMPAIGN.into());
        inner.flag_keys = Some(ask("later"));
        flag_found(&mut inner, &ask("earlier"), Some(flag(1)));
        assert_eq!(inner.snapshot.flag, None);
        assert!(inner.last_flags.is_empty());
        flag_found(&mut inner, &ask("later"), Some(flag(2)));
        assert_eq!(inner.snapshot.flag, Some(flag(2)));
        assert_eq!(inner.last_flags.get(CAMPAIGN), Some(&flag(2)));
        flag_found(&mut inner, &ask("earlier"), None);
        assert_eq!(inner.snapshot.flag, Some(flag(2)));
        flag_found(&mut inner, &ask("later"), None);
        assert_eq!(inner.snapshot.flag, None);
        assert!(inner.last_flags.is_empty());
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battleplan-recorder-admission-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("recordings")).unwrap();
        root
    }

    fn admission<'a>(
        settings: &'a Path,
        root: &'a Path,
        target: &'a Path,
        manual: bool,
    ) -> SessionAdmission<'a> {
        SessionAdmission {
            settings,
            root,
            target,
            mode: CaptureMode::Full,
            manual,
            manual_campaign: RefCell::new(None),
            first_loaded: Box::new(|| None),
            handover: RefCell::new(None),
            used_up: Box::new(|| {}),
        }
    }

    #[test]
    fn record_pressed_overrides_eligibility_but_never_the_campaigns_folder() {
        let scratch = scratch("folder");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        RecorderSettings::update_at(&settings, |settings| {
            settings.record_every_campaign = false;
            settings
                .campaigns
                .entry(CAMPAIGN.into())
                .or_default()
                .folder = Some(format!("{CAMPAIGN} (2)"));
        })
        .unwrap();
        let menu = admission(&settings, &root, &root, true);
        assert_eq!(menu.admit(CAMPAIGN, false), Err(Refusal::Elsewhere));
        assert_eq!(menu.handover.borrow().as_deref(), Some(CAMPAIGN));
        let automatic = admission(&settings, &root, &root, false);
        assert_eq!(automatic.admit(CAMPAIGN, false), Err(Refusal::Off));
        assert!(automatic.handover.borrow().is_none());
        let folder = root.join(format!("{CAMPAIGN} (2)"));
        let own = admission(&settings, &root, &folder, true);
        assert_eq!(own.admit(CAMPAIGN, false), Ok(()));
        assert_eq!(own.admit(OTHER, false), Err(Refusal::Off));
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn unreadable_settings_leave_a_load_out_instead_of_guessing_its_file() {
        let scratch = scratch("unreadable-settings");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        fs::write(&settings, b"not settings").unwrap();
        for manual in [false, true] {
            let session = admission(&settings, &root, &root, manual);
            assert_eq!(session.admit(CAMPAIGN, true), Err(Refusal::Settings));
        }
        let omitted = Omission {
            campaign: CAMPAIGN.to_owned(),
            load: Some(1),
            hours: 2,
            refusal: Some(Refusal::Settings),
        };
        assert_eq!(
            omitted_loads(&[omitted]),
            Some(OmittedLoads {
                loads: 1,
                reason: Some(OmissionReason::Settings),
            })
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn a_pending_new_file_routes_a_record_session_to_a_new_folder() {
        let scratch = scratch("new-file");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        let earlier = root.join("Germany 1936–1941.battleplan");
        targets::tests::recording_file(&earlier, CAMPAIGN);
        let contents = fs::read(&earlier).unwrap();
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        let used = Cell::new(0);
        let mut session = admission(&settings, &root, &root, true);
        session.used_up = Box::new(|| used.set(used.get() + 1));
        assert!(
            session.admit(CAMPAIGN, false) == Err(Refusal::Elsewhere),
            "the earlier recording is not appended to"
        );
        assert_eq!(session.handover.borrow().as_deref(), Some(CAMPAIGN));
        assert_eq!(used.get(), 1);
        assert_eq!(session.admit(CAMPAIGN, false), Err(Refusal::Elsewhere));
        assert_eq!(used.get(), 1);
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        assert_eq!(fs::read(&earlier).unwrap(), contents);
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn dismissing_the_stop_prompt_cancels_a_pending_quit() {
        let recorder = Recorder::default();
        recorder.lock().quit_after_stop = true;
        recorder.cancel_stop_prompt();
        assert!(!recorder.lock().quit_after_stop);
        recorder.lock().quit_after_stop = true;
        recorder.control.stop(false);
        recorder.cancel_stop_prompt();
        assert!(recorder.lock().quit_after_stop);
    }

    #[test]
    fn a_reload_after_new_file_moves_to_a_new_folder_when_only_the_spool_holds_the_campaign() {
        let scratch = scratch("reload-new-file");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        RecorderSettings::update_at(&settings, |settings| {
            settings.campaigns.entry(CAMPAIGN.into()).or_default().next = NextLoad::NewFile;
        })
        .unwrap();
        assert_eq!(targets::recording(&root, CAMPAIGN), None);
        let session = admission(&settings, &root, &root, false);
        assert_eq!(session.admit(CAMPAIGN, true), Err(Refusal::Elsewhere));
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        let folder = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            admission(&settings, &root, &folder, false).admit(CAMPAIGN, false),
            Ok(())
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn stop_while_a_refusal_detaches_holds_the_load_instead_of_attaching_again() {
        let rerouting = CaptureOutcome {
            detach_requested: true,
            rerouted: true,
            ..CaptureOutcome::default()
        };
        let ended = continuation(&rerouting, false);
        assert!(ended.hold.is_none() && ended.rerouted);
        let ended = continuation(&rerouting, true);
        assert!(ended.hold == Some(Hold::Load) && !ended.rerouted);
        let exited = CaptureOutcome {
            process_exited: true,
            ..rerouting
        };
        assert!(continuation(&exited, true).hold.is_none());
        assert_eq!(Ended::held().hold, Some(Hold::Game));
    }

    fn loaded(campaign: &str, load: i64) -> LoadedCampaign {
        LoadedCampaign {
            campaign: campaign.to_owned(),
            hour: 1,
            load,
        }
    }

    #[test]
    fn after_stop_auto_record_records_the_next_load_of_any_campaign_but_not_the_stopped_one() {
        let stopped = Held {
            start: 7,
            stopped: Some(Some((CAMPAIGN.to_owned(), 100))),
        };
        let auto = |record_every_campaign| RecorderSettings {
            record_every_campaign,
            ..RecorderSettings::default()
        };
        let (on, off) = (auto(true), auto(false));
        assert!(!stopped.released_by(Some(&loaded(CAMPAIGN, 100)), &on));
        assert!(!stopped.released_by(None, &on));
        assert!(stopped.released_by(Some(&loaded(CAMPAIGN, 101)), &on));
        assert!(stopped.released_by(Some(&loaded(OTHER, 100)), &on));
        assert!(!stopped.released_by(Some(&loaded(CAMPAIGN, 101)), &off));
        assert!(!stopped.released_by(Some(&loaded(OTHER, 5)), &off));
        let choose = |mut settings: RecorderSettings, record| {
            settings.campaigns.insert(
                OTHER.to_owned(),
                replay_settings::recorder_settings::CampaignChoice {
                    record: Some(record),
                    ..Default::default()
                },
            );
            settings
        };
        assert!(stopped.released_by(Some(&loaded(OTHER, 5)), &choose(auto(false), true)));
        assert!(!stopped.released_by(Some(&loaded(CAMPAIGN, 101)), &choose(auto(false), true)));
        assert!(!stopped.released_by(Some(&loaded(OTHER, 5)), &choose(auto(true), false)));
        let unnamed = Held {
            start: 7,
            stopped: Some(None),
        };
        assert!(unnamed.released_by(Some(&loaded(CAMPAIGN, 100)), &on));
        assert!(!Held::game(7).released_by(Some(&loaded(OTHER, 5)), &on));
    }

    #[test]
    fn record_pressed_at_the_menu_belongs_to_the_first_campaign_the_spool_shows() {
        let scratch = scratch("first-loaded");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        RecorderSettings::update_at(&settings, |settings| settings.record_every_campaign = false)
            .unwrap();
        let mut session = admission(&settings, &root, &root, true);
        session.first_loaded = Box::new(|| Some(CAMPAIGN.to_owned()));
        assert_eq!(session.admit(OTHER, false), Err(Refusal::Off));
        assert_eq!(session.admit(CAMPAIGN, false), Ok(()));
        let session = admission(&settings, &root, &root, true);
        assert_eq!(session.admit(OTHER, false), Ok(()));
        assert_eq!(session.admit(CAMPAIGN, false), Err(Refusal::Off));
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn the_panel_names_left_out_loads_that_recorded_hours_and_why() {
        let omission = |hours, refusal| Omission {
            campaign: CAMPAIGN.to_owned(),
            load: Some(1),
            hours,
            refusal,
        };
        assert_eq!(omitted_loads(&[]), None);
        assert_eq!(omitted_loads(&[omission(0, Some(Refusal::Off))]), None);
        assert_eq!(
            omitted_loads(&[omission(3, Some(Refusal::Off)), omission(0, None)]),
            Some(OmittedLoads {
                loads: 1,
                reason: Some(OmissionReason::Off),
            })
        );
        assert_eq!(
            omitted_loads(&[
                omission(2, Some(Refusal::Elsewhere)),
                omission(1, Some(Refusal::Elsewhere)),
            ]),
            Some(OmittedLoads {
                loads: 2,
                reason: Some(OmissionReason::Elsewhere),
            })
        );
        assert_eq!(
            omitted_loads(&[omission(2, Some(Refusal::Off)), omission(1, None)]),
            Some(OmittedLoads {
                loads: 2,
                reason: None,
            })
        );
    }
}
