use crate::{
    flags::{BackgroundPriority, FlagContent, FlagImage},
    panel,
    snapshot::{self, OmissionReason, OmittedLoads, Phase, Problem, Snapshot},
    targets, tray,
};
use recorder::controller::{
    self, CaptureOutcome, Control, Discovery, GameProcess, LoadedCampaign, Omission, Probe,
    Recording, Refusal, SetAside, Stage, Stop, Undiscarded, Waiting,
};
use replay::campaign::Replay;
use replay_files::recording_files;
use replay_settings::{
    exit::{Exit, Receipt, What},
    recorder_settings::RecorderSettings,
};
use replay_data::scope::CaptureMode;
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard},
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
    quit_after_stop: Option<u64>,
    prompts: panel::Prompts,
    prompt_revision: u64,
    seen_prompt: BTreeMap<u64, u64>,
    closing_itself: BTreeMap<u64, u64>,
    sessions: u64,
    stopped: Option<(u64, bool)>,
    capturing: bool,
    unloading: bool,
    result: bool,
    sticky: Option<Problem>,
    panel_open: bool,
    seen: bool,
    hours: u64,
    earlier: Option<(String, Option<PathBuf>, Option<u64>)>,
    session: Option<(PathBuf, PathBuf)>,
    game_running: bool,
    set_aside: Vec<Waiting>,
    names: BTreeMap<String, String>,
    flag_keys: Option<FlagAsk>,
    flag_folders: Option<(PathBuf, PathBuf)>,
    last_flags: BTreeMap<String, FlagImage>,
    exit: Coordinator,
    jobs: Arc<crate::compress::Jobs>,
    tray: u64,
}

struct Coordinator(&'static Exit);

impl Default for Coordinator {
    #[cfg(test)]
    fn default() -> Self {
        Self(Box::leak(Box::new(Exit::new())))
    }

    #[cfg(not(test))]
    fn default() -> Self {
        Self(&replay_settings::exit::EXIT)
    }
}

pub struct SessionEnding(pub AppHandle, pub &'static app_update::session::Done);

impl app_update::session::Ending for SessionEnding {
    fn commit(&self) {
        self.0.state::<Arc<Recorder>>().exit_requested(&self.0);
    }

    fn done(&self) -> &app_update::session::Done {
        self.1
    }

    fn saving(&self) -> Option<&'static str> {
        let recorder = self.0.state::<Arc<Recorder>>();
        let inner = recorder.lock();
        inner.live_session().map(|_| RECORDING).or_else(|| inner.exit.0.held().map(saving))
    }
}

pub fn saving(held: What) -> &'static str {
    match held {
        What::Install => STARTING_UPDATE,
        What::Recording => FINISHING_RECORDING,
        _ => SAVING,
    }
}

pub enum Leave {
    Exit,
    Install(app_update::Launch),
}
const SAVING_FOR_UPDATE: &str =
    "BattlePlan Recorder is saving a recording. Restart to update once it has saved.";
const RECORDING_FOR_UPDATE: &str =
    "BattlePlan Recorder is recording. Restart to update once the recording stops.";
const CONNECTED_FOR_UPDATE: &str =
    "BattlePlan Recorder is connected to the game. Restart to update once the game has closed.";
const CLOSING_FOR_UPDATE: &str = "BattlePlan Recorder is closing for an update.";
const QUITTING: &str = "BattlePlan Recorder is quitting, so it starts nothing more.";
const STARTING_UPDATE: &str = "Starting the update before BattlePlan Recorder quits…";
const FINISHING_RECORDING: &str = "Saving the recording before BattlePlan Recorder quits…";
const SAVING: &str = "Saving your changes before BattlePlan Recorder quits…";
const RECORDING: &str = "Recording a campaign, which is saved before BattlePlan Recorder quits";
const SESSION_ENDED: &str = "The session you asked about has already ended, so nothing was changed.";
const SESSION_STOPPING: &str = "BattlePlan Recorder is already stopping this session.";
const SESSION_SAVING: &str =
    "BattlePlan Recorder had already begun saving this session, so nothing was changed.";

fn cancelled(inner: &mut Inner, through: u64, stopping: bool) {
    inner.prompts.retire(through);
    if !stopping && inner.quit_after_stop.is_some_and(|quit| quit <= through) {
        inner.quit_after_stop = None;
    }
}

impl Inner {
    fn quit(&mut self) -> bool {
        if let Some(session) = self.live_session() {
            self.ask(session, true);
            return true;
        }
        self.exit();
        false
    }

    fn exiting(&self) -> bool {
        self.exit.0.sealed()
    }

    fn settle(&mut self, control: &Control) {
        if !self.capturing && self.quit_after_stop.is_some() {
            self.exit();
        }
        if self.exiting() && self.capturing && (control.stopping() || control.stop(Stop::Save(None))) && !self.snapshot.stopping {
            self.snapshot.stopping = true;
            self.prompts.clear();
            if self.snapshot.phase == Phase::Recording {
                self.snapshot.phase = Phase::Saving;
                self.snapshot.problem = None;
            }
        }
    }

    fn exit(&mut self) {
        if self.exiting() {
            return;
        }
        self.record_requested = false;
        let notices = self.jobs.close();
        self.exit.0.seal_with(notices);
    }

    fn finishing(&self) -> bool {
        self.capturing || self.unloading || self.exit.0.held().is_some()
    }

    fn saving_to_quit(&self) -> bool {
        self.exiting() && self.finishing()
    }

    fn new_session(&mut self) {
        self.sessions += 1;
        self.snapshot.session_key = Some(self.sessions);
        let quit = self.prompts.follow(self.sessions);
        if self.quit_after_stop.is_some() && quit.is_some() {
            self.quit_after_stop = quit;
        }
    }

    fn live_session(&self) -> Option<u64> {
        (self.capturing && self.snapshot.phase == Phase::Recording && !self.snapshot.stopping)
            .then_some(self.snapshot.session_key)
            .flatten()
    }

    fn ask(&mut self, session: u64, quitting: bool) -> panel::Prompt {
        let prompt = self.prompts.ask(session, quitting);
        if quitting {
            self.quit_after_stop = Some(prompt.id);
        }
        prompt
    }

    fn publish(&mut self, before: &Snapshot) -> Option<Snapshot> {
        self.snapshot.prompt = self.prompts.pending(self.live_session());
        self.snapshot.revision = before.revision;
        if self.snapshot == *before {
            return None;
        }
        self.snapshot.revision += 1;
        if self.snapshot.prompt != before.prompt {
            self.prompt_revision = self.snapshot.revision;
        }
        Some(self.snapshot.clone())
    }

    fn dismiss(&mut self, panel: u64, revision: u64) -> bool {
        let dismissible = self.snapshot.prompt.is_none() || self.prompt_revision <= revision;
        if dismissible {
            self.closing_itself.insert(panel, self.prompts.issued());
        }
        dismissible
    }

    fn dismissal_failed(&mut self, panel: u64) {
        self.closing_itself.remove(&panel);
    }

    fn prompt_seen(&mut self, panel: u64, prompt: u64) {
        let seen = self.seen_prompt.entry(panel).or_insert(prompt);
        *seen = (*seen).max(prompt);
    }

    fn panel_gone(&mut self, panel: u64, stopping: bool, dismissed: bool) -> bool {
        self.panel_open = false;
        let issued = self.closing_itself.remove(&panel);
        let seen = self.seen_prompt.remove(&panel);
        self.closing_itself.retain(|&by, _| by > panel);
        self.seen_prompt.retain(|&by, _| by > panel);
        let through = match issued {
            Some(issued) => seen.filter(|_| issued > 0).map(|seen| seen.min(issued - 1)),
            None => seen,
        };
        if let Some(through) = through {
            cancelled(self, through, stopping);
        }
        if std::mem::take(&mut self.seen) {
            self.result = false;
            self.sticky = None;
        }
        !dismissed && self.prompts.pending(self.live_session()).is_some()
    }
}

fn update_busy(inner: &Inner) -> Option<&'static str> {
    let saving = [What::Change, What::WaitingRecording, What::Recovery].into_iter().any(|what| inner.exit.0.count(what) > 0);
    if inner.exit.0.count(What::Install) > 0 {
        Some(CLOSING_FOR_UPDATE)
    } else if inner.unloading || saving || inner.snapshot.phase == Phase::Saving {
        Some(SAVING_FOR_UPDATE)
    } else if inner.capturing && inner.snapshot.phase == Phase::Recording {
        Some(RECORDING_FOR_UPDATE)
    } else if inner.capturing {
        Some(CONNECTED_FOR_UPDATE)
    } else {
        None
    }
}


#[derive(Default)]
pub struct Recorder {
    control: Control,
    inner: Mutex<Inner>,
    wake: Condvar,
    flag_content: Mutex<Option<(PathBuf, Arc<FlagContent>)>>,
    recordings: Mutex<Recordings>,
}

#[derive(Default)]
struct Recordings {
    idle: targets::RecordingCache,
    saved: targets::RecordingCache,
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
    match RecorderSettings::update_at(settings, |settings| settings.keep_session_root(root.to_path_buf())) {
        Ok(applied) => drop(applied.logged("The session's save folder")),
        Err(error) => eprintln!("The session's save folder could not be kept listed: {error}"),
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

    fn recordings(&self) -> MutexGuard<'_, Recordings> {
        self.recordings.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Inner)) {
        let (published, shown) = {
            let mut inner = self.lock();
            let before = inner.snapshot.clone();
            change(&mut inner);
            inner.settle(&self.control);
            let published = inner.publish(&before);
            if inner.panel_open && (inner.result || inner.sticky.is_some()) {
                inner.seen = true;
            }
            inner.tray += 1;
            let saving_to_quit = inner.saving_to_quit();
            let shown = tray::Tray {
                revision: inner.tray,
                light: if saving_to_quit { crate::snapshot::Light::Saving } else { inner.snapshot.light() },
                tooltip: tray::tooltip_for(&inner.snapshot, saving_to_quit),
                can_start: inner.game_running && !inner.capturing && !inner.unloading,
                can_stop: inner.capturing
                    && inner.snapshot.phase == Phase::Recording
                    && !inner.snapshot.stopping,
            };
            (published, shown)
        };
        self.wake.notify_all();
        if let Some(snapshot) = &published
            && let Err(error) = app.emit(STATUS_EVENT, snapshot)
        {
            eprintln!("Recorder status could not reach the panel: {error}");
        }
        tray::show(app, shown);
    }

    pub fn exit_requested(&self, app: &AppHandle) {
        self.update(app, Inner::exit);
    }

    pub fn with(jobs: Arc<crate::compress::Jobs>) -> Self {
        let recorder = Self::default();
        recorder.lock().jobs = jobs;
        recorder
    }

    pub fn admit(&self, what: What) -> Result<Receipt<'static>, String> {
        self.lock().exit.0.admit(what).map_err(|error| match error.kind() {
            std::io::ErrorKind::Interrupted => QUITTING.to_owned(),
            std::io::ErrorKind::ResourceBusy => CLOSING_FOR_UPDATE.to_owned(),
            _ => error.to_string(),
        })
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
        let mut inner = self.lock();
        inner.record_requested = !inner.exiting();
        drop(inner);
        self.wake.notify_all();
    }

    pub fn stop(&self, app: &AppHandle, discard: bool, session: u64) -> Result<(), String> {
        let mut accepted = Ok(false);
        self.update(app, |inner| {
            accepted = accept_stop(inner, &self.control, discard, session);
        });
        if accepted? {
            self.wake.notify_all();
        }
        Ok(())
    }

    pub fn save(&self, app: &AppHandle) {
        let session = self.lock().live_session();
        if let Some(session) = session
            && let Err(error) = self.stop(app, false, session)
        {
            eprintln!("The tray’s Save didn’t stop the recording: {error}");
        }
    }

    pub fn ask_discard(&self, app: &AppHandle) {
        let mut prompted = false;
        self.update(app, |inner| {
            if let Some(session) = inner.live_session() {
                inner.ask(session, false);
                prompted = true;
            }
        });
        if prompted {
            panel::open(app, None, true);
        }
    }

    pub fn quit(&self, app: &AppHandle) {
        let mut prompted = false;
        self.update(app, |inner| prompted = inner.quit());
        if prompted {
            panel::open(app, None, true);
        } else {
            self.wake.notify_all();
        }
    }

    pub fn cancel_prompt(&self, app: &AppHandle, through: u64) {
        let stopping = self.control.stopping();
        self.update(app, |inner| cancelled(inner, through, stopping));
    }

    pub fn panel_opened(&self, app: &AppHandle) {
        self.update(app, |inner| inner.panel_open = true);
    }

    pub fn prompt_seen(&self, panel: u64, prompt: u64) {
        self.lock().prompt_seen(panel, prompt);
    }

    pub fn dismiss(&self, panel: u64, revision: u64) -> bool {
        self.lock().dismiss(panel, revision)
    }

    pub fn dismissal_failed(&self, panel: u64) {
        self.lock().dismissal_failed(panel);
    }

    pub fn panel_closed(&self, app: &AppHandle, panel: u64, dismissed: bool) {
        let stopping = self.control.stopping();
        let mut unseen = false;
        self.update(app, |inner| unseen = inner.panel_gone(panel, stopping, dismissed));
        self.wake.notify_all();
        if unseen {
            panel::open(app, None, true);
        }
    }

    pub fn settings_changed(&self) {
        self.wake.notify_all();
    }

    pub fn update_blocked(&self) -> Option<&'static str> {
        let inner = self.lock();
        update_busy(&inner).or(inner.exiting().then_some(QUITTING))
    }

    pub fn leave(&self, leave: Leave) -> Result<(), String> {
        let launch = {
            let mut inner = self.lock();
            match leave {
                Leave::Exit if inner.exiting() => None,
                Leave::Install(_) if inner.exiting() => return Err(QUITTING.into()),
                leave => {
                    if let Some(reason) = update_busy(&inner) {
                        return Err(reason.into());
                    }
                    match leave {
                        Leave::Exit => {
                            inner.exit();
                            None
                        }
                        Leave::Install(launch) => {
                            let receipt = inner.exit.0.admit(What::Install).map_err(|_| SAVING_FOR_UPDATE.to_owned())?;
                            Some((launch, receipt))
                        }
                    }
                }
            }
        };
        self.wake.notify_all();
        let Some((launch, receipt)) = launch else { return Ok(()) };
        struct Launching<'a> {
            recorder: &'a Recorder,
            launched: bool,
            _receipt: Receipt<'static>,
        }
        impl Drop for Launching<'_> {
            fn drop(&mut self) {
                if self.launched {
                    self.recorder.lock().exit();
                }
                self.recorder.wake.notify_all();
            }
        }
        let mut launching = Launching { recorder: self, launched: false, _receipt: receipt };
        let launched = launch();
        launching.launched = launched.is_ok();
        launched
    }

    #[cfg(test)]
    pub(crate) fn commit_exit(&self) {
        self.lock().exit();
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
            if self.lock().exiting() {
                self.update(app, |_| {});
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
                if look.pass
                    && let Some(retry) = self.end_abandoned(app, &settings)
                {
                    abandoned_retry.after(retry);
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

    fn abandoned_pass(&self, pass: impl FnOnce() -> bool) -> Option<bool> {
        let receipt = self.admit(What::Recovery).ok()?;
        Some(receipt.carry(pass))
    }

    fn end_abandoned(&self, app: &AppHandle, settings: &RecorderSettings) -> Option<bool> {
        let retry = self.abandoned_pass(|| {
            let mut retry = false;
            for root in settings.session_roots() {
                let root = root.canonicalize().unwrap_or(root);
                let pass = controller::end_abandoned_sessions(&root);
                for error in pass.errors {
                    eprintln!("An earlier session could not be ended: {error}");
                }
                retry |= pass.retry;
            }
            retry
        })?;
        recordings_changed(app);
        Some(retry)
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
            let recording = self.recordings().idle.get(&folder, &loaded.campaign);
            let bytes = recording.as_ref().and_then(|recording| fs::metadata(recording).ok()).map(|metadata| metadata.len());
            Some((loaded.clone(), recording, bytes))
        });
        let shown = {
            let inner = self.lock();
            let snapshot = &inner.snapshot;
            match (inner.result, &snapshot.recording, snapshot.campaign.clone()) {
                (true, Some(recording), Some(campaign)) => {
                    Path::new(recording).parent().map(|folder| (folder.to_path_buf(), campaign))
                }
                _ => None,
            }
        };
        let saved = shown.and_then(|(folder, campaign)| self.recordings().saved.get(&folder, &campaign));
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
            if let Some((loaded, recording, bytes)) = named {
                inner.snapshot.saved_bytes = bytes;
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
        self.control.reset();
        let Ok(receipt) = self.admit(What::Recording) else {
            return Ended::again();
        };
        self.update(app, |inner| inner.capturing = true);
        receipt.carry(|| self.capture_admitted(app, game, settings, manual, loaded, probe))
    }

    fn capture_admitted(
        &self,
        app: &AppHandle,
        game: controller::OpenedGame,
        settings: &RecorderSettings,
        manual: bool,
        loaded: Option<LoadedCampaign>,
        probe: Option<&Probe>,
    ) -> Ended {
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
                inner.new_session();
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
        let kept = outcome.undiscarded.map(|undiscarded| {
            Problem::failure(
                format!("Nothing was discarded: {}.", undiscarded.reason()),
                match undiscarded {
                    Undiscarded::Unread => {
                        "Stop couldn’t read which campaign was loaded; every hour was kept."
                    }
                    Undiscarded::Another => {
                        "Discard named the campaign you saw, and the game had moved on to another one, so every hour was kept."
                    }
                },
            )
        });
        match saved {
            Some(path) => {
                let span = Replay::read_hour_span(&path).ok().flatten();
                let bytes = fs::metadata(&path).ok().map(|metadata| metadata.len());
                let named = campaign_of(&path);
                self.update(app, |inner| {
                    inner.capturing = false;
                    inner.session = None;
                    inner.snapshot.capture_mode = None;
                    inner.result = true;
                    inner.set_aside.clear();
                    let snapshot = &mut inner.snapshot;
                    snapshot.phase = Phase::Saved;
                    snapshot.omitted = omitted;
                    snapshot.campaign = named.or(snapshot.campaign.take());
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
        let carried = replay_settings::exit::Carry::current();
        let spawned = std::thread::Builder::new()
            .name("recorder-unload".into())
            .spawn(move || carried.run(|| {
                let result = observation.finish();
                recorder.update(&app_copy, |inner| unload_finished(inner, result));
                recordings_changed(&app_copy);
                recorder.wake.notify_all();
            }));
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

fn accept_stop(inner: &mut Inner, control: &Control, discard: bool, session: u64) -> Result<bool, String> {
    if inner.stopped == Some((session, discard)) {
        return Ok(false);
    }
    if inner.live_session() != Some(session) {
        return Err(if !inner.capturing || inner.snapshot.session_key != Some(session) {
            SESSION_ENDED
        } else if inner.snapshot.stopping {
            SESSION_STOPPING
        } else {
            SESSION_SAVING
        }
        .into());
    }
    let stop = match (discard, &inner.snapshot.campaign) {
        (false, campaign) => Stop::Save(campaign.clone()),
        (true, Some(campaign)) => Stop::Discard(campaign.clone()),
        (true, None) => return Err(SESSION_ENDED.into()),
    };
    if !control.stop(stop) {
        return Err(SESSION_SAVING.into());
    }
    inner.stopped = Some((session, discard));
    inner.prompts.clear();
    inner.snapshot.stopping = true;
    if inner.quit_after_stop.is_some() {
        inner.exit();
    }
    if !discard {
        inner.snapshot.phase = Phase::Saving;
        inner.snapshot.problem = None;
    }
    Ok(true)
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
    if let Some(campaign) = &live.campaign
        && (inner.snapshot.phase != Phase::Recording || inner.snapshot.campaign.as_ref() != Some(campaign))
    {
        inner.new_session();
    }
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
    fn quit_waits_for_a_waiting_recording_being_saved_and_the_tray_says_so() {
        let mut inner = Inner::default();
        let saving = inner.exit.0.admit(What::WaitingRecording).unwrap();
        assert!(!inner.quit(), "a recording is being saved without a capture, so Quit doesn’t end the process inside it");
        assert!(inner.exiting() && inner.saving_to_quit());
        assert!(tray::tooltip_for(&inner.snapshot, inner.saving_to_quit()).contains("Saving a recording, then quitting"));
        drop(saving);
        assert!(!inner.finishing(), "the Quit goes ahead once the save has published");
    }

    fn committed(inner: Inner, how: &str) {
        assert!(inner.exiting(), "{how}: the recorder committed to quit");
        let recorder = Recorder::default();
        *recorder.lock() = inner;
        assert_eq!(recorder.admit(What::Change).err(), Some(QUITTING.to_owned()), "{how}: no save, discard or settings change is admitted after");
        recorder.record();
        assert!(!recorder.lock().record_requested, "{how}: nor a new capture");
        assert_eq!(recorder.leave(Leave::Install(Box::new(|| -> Result<(), String> { unreachable!("no installer starts once the recorder quits") }))), Err(QUITTING.to_owned()), "{how}");
        assert_eq!(recorder.leave(Leave::Exit), Ok(()), "{how}: setup’s close is agreed to at once, since the recorder is leaving");
        assert!(recorder.update_blocked().is_some(), "{how}");
    }

    #[test]
    fn every_way_the_recorder_quits_commits_once_before_anything_more_is_admitted() {
        let mut idle = Inner::default();
        assert!(!idle.quit());
        assert!(!idle.finishing(), "an idle Quit waits on nothing, but still leaves through the ordered quit");
        committed(idle, "tray Quit while idle");

        let mut saving = Inner::default();
        let _saving = saving.exit.0.admit(What::WaitingRecording).unwrap();
        assert!(!saving.quit());
        committed(saving, "tray Quit during an offline save");

        let control = Control::new();
        let mut connected = Inner { capturing: true, ..Inner::default() };
        assert!(!connected.quit());
        connected.settle(&control);
        assert!(control.stopping(), "the transition itself asks the capture to stop");
        committed(connected, "tray Quit while connected to the game");

        let control = Control::new();
        let mut answered = Inner { capturing: true, ..Inner::default() };
        answered.snapshot.phase = Phase::Recording;
        answered.new_session();
        let live = answered.snapshot.session_key.unwrap();
        assert!(answered.quit());
        assert!(!answered.exiting(), "a Quit that asks first commits to nothing until it is answered");
        assert_eq!(accept_stop(&mut answered, &control, false, live), Ok(true));
        committed(answered, "the Quit prompt answered with Save");
    }

    fn recording(control: &Control) -> Inner {
        control.reset();
        let mut inner = Inner { capturing: true, ..Inner::default() };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        inner
    }

    #[test]
    fn an_exit_asked_for_in_code_or_by_windows_stops_the_live_capture_it_then_waits_for() {
        let control = Control::new();
        let mut inner = recording(&control);
        let capture = inner.exit.0.admit(What::Recording).unwrap();
        inner.exit();
        inner.settle(&control);
        assert!(control.stopping(), "the stop is asked for in the transition, before anything waits on the capture");
        assert!(inner.snapshot.stopping && inner.snapshot.phase == Phase::Saving && inner.saving_to_quit());
        assert!(control.stop(Stop::Save(None)), "and it is a save, never a discard");
        assert!(inner.exit.0.admit(What::Recording).is_err(), "no capture is admitted once the quit commits, so a refused one consumes no New file");
        drop(capture);
        assert!(inner.exit.0.empty_within(Duration::ZERO));
    }

    #[test]
    fn a_quit_left_waiting_commits_under_the_lock_that_ends_its_session() {
        let control = Control::new();
        let mut inner = recording(&control);
        assert!(inner.quit(), "a Quit during a live session asks first");
        inner.settle(&control);
        assert!(!inner.exiting());
        inner.capturing = false;
        inner.settle(&control);
        assert!(inner.exiting(), "the update that ends the session commits the Quit at once");
    }

    #[test]
    fn setups_install_and_a_quit_decide_in_one_locked_step_whichever_comes_first() {
        let recorder = Recorder::default();
        let (started, launching) = std::sync::mpsc::channel();
        let (release, released) = std::sync::mpsc::channel::<()>();
        std::thread::scope(|scope| {
            let install = scope.spawn(|| recorder.leave(Leave::Install(Box::new(move || {
                started.send(()).unwrap();
                released.recv().unwrap();
                Err("The installer couldn’t be started.".to_owned())
            }))));
            launching.recv().unwrap();
            assert_eq!(recorder.admit(What::Change).err(), Some(CLOSING_FOR_UPDATE.to_owned()), "a settings change can’t run under the installer launch");
            recorder.commit_exit();
            assert!(recorder.lock().saving_to_quit(), "a Quit after the take waits for the installer to start or fail");
            release.send(()).unwrap();
            assert!(install.join().unwrap().is_err());
        });
        assert!(recorder.lock().exiting(), "a failed launch leaves the independent Quit committed");
        assert!(recorder.lock().exit.0.empty_within(Duration::ZERO));

        let recorder = Recorder::default();
        recorder.commit_exit();
        assert_eq!(recorder.leave(Leave::Install(Box::new(|| unreachable!("no installer starts once the recorder quits")))), Err(QUITTING.to_owned()), "a Quit before the take refuses the install by name");

        let recorder = Recorder::default();
        let saving = recorder.admit(What::WaitingRecording).unwrap();
        assert_eq!(recorder.leave(Leave::Install(Box::new(|| unreachable!("nothing launches while a save runs")))), Err(SAVING_FOR_UPDATE.to_owned()));
        drop(saving);
        assert_eq!(recorder.leave(Leave::Install(Box::new(|| Ok(())))), Ok(()));
        assert!(recorder.lock().exiting(), "a launch that starts commits before it lets its receipt go");
    }

    #[test]
    fn setups_close_commits_when_it_is_decided() {
        let recorder = Recorder::default();
        assert_eq!(recorder.leave(Leave::Exit), Ok(()));
        assert!(recorder.lock().exiting(), "the close commits in the call that answers it");
    }

    #[test]
    fn an_abandoned_session_is_ended_only_under_the_recorders_admission_and_the_quit_waits_for_it() {
        let recorder = Recorder::default();
        let during = recorder.abandoned_pass(|| {
            assert!(recorder.lock().finishing(), "a quit committed now waits for the pass");
            assert!(recorder.admit(What::Change).is_ok(), "the pass’s own settings writes continue under it");
            true
        });
        assert_eq!(during, Some(true));
        recorder.commit_exit();
        assert_eq!(recorder.abandoned_pass(|| unreachable!("no recovery starts once the recorder quits")), None);
    }

    #[test]
    fn a_stop_after_the_capture_ended_keeps_its_result_and_stops_no_later_capture() {
        let control = Control::new();
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        assert_eq!(accept_stop(&mut inner, &control, false, session), Ok(true));
        assert_eq!(control.seal(), Some(Stop::Save(None)));
        assert_eq!(inner.snapshot.phase, Phase::Saving);
        assert!(inner.snapshot.stopping);
        let control = Control::new();
        let mut inner = Inner {
            result: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Saved;
        let ended = inner.snapshot.clone();
        assert_eq!(accept_stop(&mut inner, &control, false, session), Err(SESSION_ENDED.into()));
        assert_eq!(inner.snapshot, ended);
        assert!(!control.stopping());
    }

    #[test]
    fn a_stop_acts_only_on_the_session_it_was_asked_about_and_a_repeat_changes_nothing() {
        let control = Control::new();
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let asked = inner.snapshot.session_key.unwrap();
        let discard = inner.ask(asked, false);
        inner.new_session();
        let live = inner.snapshot.session_key.unwrap();
        assert_eq!(inner.prompts.pending(Some(live)), None, "the Discard asked about the earlier session isn’t shown for this one");
        assert_eq!(accept_stop(&mut inner, &control, true, discard.session_key), Err(SESSION_ENDED.into()), "and discards nothing");
        assert!(!control.stopping());

        let quit = inner.ask(live, true);
        assert_eq!(accept_stop(&mut inner, &control, false, live), Ok(true));
        assert_eq!(inner.prompts.pending(Some(live)), None, "a stop answers every prompt about its session");
        assert_eq!(inner.quit_after_stop, Some(quit.id), "and the Quit it answered goes ahead");
        assert_eq!(accept_stop(&mut inner, &control, false, live), Ok(false), "the same stop sent again after a lost reply");
        assert_eq!(accept_stop(&mut inner, &control, true, live), Err(SESSION_STOPPING.into()), "a later Discard can’t turn the save into one");
        assert_eq!(control.seal(), Some(Stop::Save(None)));
    }

    #[test]
    fn a_discard_names_the_campaign_it_was_asked_about_and_none_is_taken_once_the_session_is_saving() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.snapshot.campaign = Some("shown".into());
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();

        let control = Control::new();
        inner.snapshot.phase = Phase::Saving;
        let saving = inner.snapshot.clone();
        assert_eq!(
            accept_stop(&mut inner, &control, true, session),
            Err(SESSION_SAVING.into()),
            "a session that ended by itself is already being saved"
        );
        assert_eq!(inner.snapshot, saving);
        assert!(!control.stopping());

        inner.snapshot.phase = Phase::Recording;
        assert_eq!(control.seal(), None);
        assert_eq!(
            accept_stop(&mut inner, &control, true, session),
            Err(SESSION_SAVING.into()),
            "nor once the capture has settled what it keeps, before the panel hears it is saving"
        );
        assert!(!inner.snapshot.stopping && inner.stopped.is_none());

        let control = Control::new();
        assert_eq!(accept_stop(&mut inner, &control, true, session), Ok(true));
        assert_eq!(
            control.seal(),
            Some(Stop::Discard("shown".into())),
            "the capture is told which campaign the user chose to discard"
        );
    }

    #[test]
    fn statuses_are_numbered_and_the_panel_closes_only_over_the_prompts_it_showed() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let first = inner.publish(&Snapshot::default()).expect("the new session is published");
        assert_eq!(inner.publish(&first), None, "an unchanged status isn’t published again");

        let discard = inner.ask(session, false);
        let asked = inner.publish(&first).unwrap();
        assert!(asked.revision > first.revision);
        assert_eq!(asked.prompt, Some(discard));
        assert!(
            !inner.dismiss(1, first.revision),
            "a panel judging the status before the Discard can’t close over it"
        );
        assert!(inner.dismiss(1, asked.revision));

        inner.prompt_seen(1, discard.id);
        let quit = inner.ask(session, true);
        let before = inner.snapshot.clone();
        inner.publish(&before);
        assert!(inner.panel_gone(1, false, false), "the Quit the panel never showed needs a panel");
        assert_eq!(inner.prompts.pending(Some(session)).map(|prompt| prompt.id), Some(quit.id));
        assert_eq!(inner.quit_after_stop, Some(quit.id), "closing the panel doesn’t cancel a Quit it never showed");

        inner.prompt_seen(2, quit.id);
        assert!(!inner.panel_gone(2, false, false));
        assert_eq!(inner.prompts.pending(Some(session)), None, "closing a panel cancels what the user acted on in it");
        assert_eq!(inner.quit_after_stop, None);
    }

    #[test]
    fn a_prompt_counts_as_seen_only_once_the_user_acts_on_it() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let quit = inner.ask(session, true);
        assert!(inner.panel_gone(1, false, false), "a panel that closed itself with the Quit drawn but untouched asks again, whatever its window showed");
        inner.prompt_seen(2, quit.id);
        assert!(!inner.panel_gone(2, false, false), "a click or key inside the Quit means the user saw it, so closing the panel answers it");
        assert_eq!(inner.prompts.pending(Some(session)), None);
        assert_eq!(inner.quit_after_stop, None);
    }

    #[test]
    fn a_panel_the_user_dismissed_isnt_opened_again_and_its_prompt_waits_in_the_tray() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let quit = inner.ask(session, true);
        assert!(!inner.panel_gone(1, false, true), "a click away, the tray icon or Cancel on the message that the panel couldn’t be shown opens no other panel");
        assert_eq!(inner.prompts.pending(Some(session)).map(|prompt| prompt.id), Some(quit.id), "the Quit waits for the tray to open the panel");
        assert_eq!(inner.quit_after_stop, Some(quit.id));
        let shown = inner.publish(&Snapshot::default()).expect("the status carries the Quit");
        assert!(tray::tooltip(&shown).contains("Quit BattlePlan Recorder?"), "and the tray says it waits: {}", tray::tooltip(&shown));
        assert!(inner.panel_gone(2, false, false), "a panel that closes itself before the Quit was acted on asks again");
    }

    #[test]
    fn a_retired_panels_late_close_changes_nothing_about_how_its_replacement_closes() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let judged = inner.publish(&Snapshot::default()).unwrap();
        assert!(!inner.panel_gone(1, false, true), "panel 1 closes on a click away");
        assert!(inner.dismiss(1, judged.revision), "its page’s own close arrives after its window went");
        inner.dismissal_failed(2);
        let quit = inner.ask(session, true);
        let before = inner.snapshot.clone();
        inner.publish(&before);
        inner.prompt_seen(2, quit.id);
        assert!(!inner.panel_gone(2, false, true), "panel 2 closes on a click away after the user acted on the Quit in it");
        assert_eq!(inner.prompts.pending(Some(session)), None, "so the Quit is answered, though panel 1 closed itself before it was asked");
        assert_eq!(inner.quit_after_stop, None);
    }

    #[test]
    fn a_retired_panels_late_close_leaves_its_replacements_own_close_in_place() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let judged = inner.publish(&Snapshot::default()).unwrap();
        assert!(!inner.panel_gone(1, false, true), "panel 1 closes on a click away");
        assert!(inner.dismiss(2, judged.revision), "panel 2 closes itself with nothing pending");
        assert!(inner.dismiss(1, judged.revision), "panel 1’s page asked to close before its window went, and its request lands now");
        let quit = inner.ask(session, true);
        let before = inner.snapshot.clone();
        inner.publish(&before);
        inner.prompt_seen(2, quit.id);
        assert!(inner.panel_gone(2, false, false), "the Quit asked after panel 2 judged its status asks again");
        assert_eq!(inner.prompts.pending(Some(session)).map(|prompt| prompt.id), Some(quit.id));
        assert_eq!(inner.quit_after_stop, Some(quit.id));
    }

    #[test]
    fn a_panel_closing_itself_retires_nothing_asked_after_the_status_it_judged() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let session = inner.snapshot.session_key.unwrap();
        let judged = inner.publish(&Snapshot::default()).unwrap();
        assert!(inner.dismiss(1, judged.revision), "nothing is pending, so the compact panel closes");
        let quit = inner.ask(session, true);
        let before = inner.snapshot.clone();
        inner.publish(&before);
        inner.prompt_seen(1, quit.id);
        assert!(inner.panel_gone(1, false, false), "the Quit that arrived before the window went asks again");
        assert_eq!(inner.prompts.pending(Some(session)).map(|prompt| prompt.id), Some(quit.id));
        assert_eq!(inner.quit_after_stop, Some(quit.id));
    }

    #[test]
    fn a_quit_carried_to_a_new_session_is_a_new_prompt_the_panel_has_not_shown() {
        let mut inner = Inner {
            capturing: true,
            ..Inner::default()
        };
        inner.snapshot.phase = Phase::Recording;
        inner.new_session();
        let quit = inner.ask(inner.snapshot.session_key.unwrap(), true);
        inner.prompt_seen(1, quit.id);
        inner.new_session();

        let session = inner.snapshot.session_key.unwrap();
        let carried = inner.prompts.pending(Some(session)).expect("the Quit asks about the new session");
        assert_ne!(carried.id, quit.id);
        assert_eq!(inner.quit_after_stop, Some(carried.id));
        assert!(inner.panel_gone(1, false, false), "closing the panel doesn’t cancel a Quit about a session it never showed");
        assert_eq!(inner.quit_after_stop, Some(carried.id));
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
        let _ = RecorderSettings::update_at(&settings, |settings| {
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
        let _ = RecorderSettings::update_at(&settings, |settings| {
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
    fn cancelling_a_prompt_cancels_the_quit_it_carries_and_no_newer_one() {
        let mut inner = Inner::default();
        let discard = inner.ask(1, false);
        let quit = inner.ask(1, true);
        cancelled(&mut inner, discard.id, false);
        assert_eq!(inner.quit_after_stop, Some(quit.id), "cancelling the older Discard keeps the newer Quit");
        assert_eq!(inner.prompts.pending(Some(1)), Some(quit));
        cancelled(&mut inner, discard.id, false);
        assert_eq!(inner.quit_after_stop, Some(quit.id), "a cancel sent again changes nothing");
        cancelled(&mut inner, quit.id, false);
        assert_eq!(inner.quit_after_stop, None);
        let quit = inner.ask(1, true);
        cancelled(&mut inner, quit.id, true);
        assert_eq!(inner.quit_after_stop, Some(quit.id), "a quit whose session is already stopping goes ahead");
    }

    #[test]
    fn a_reload_after_new_file_moves_to_a_new_folder_when_only_the_spool_holds_the_campaign() {
        let scratch = scratch("reload-new-file");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        let _ = RecorderSettings::update_at(&settings, |settings| {
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
        let _ = RecorderSettings::update_at(&settings, |settings| settings.record_every_campaign = false)
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
