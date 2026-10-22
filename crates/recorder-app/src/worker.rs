//! The record loop and the snapshot it keeps. One worker thread waits for a
//! single game, attaches when the settings say so and finalizes; the tray and
//! panel reach it through this state.
use crate::{
    flags::{BackgroundPriority, FlagContent, FlagImage},
    panel,
    snapshot::{self, OmissionReason, OmittedLoads, Phase, Problem, Snapshot},
    targets, tray,
};
use recorder::controller::{
    self, CaptureOutcome, Control, LoadedCampaign, Omission, Probe, Recording, Refusal, Stage,
};
use replay::{container::Replay, recorder_settings::RecorderSettings};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};

pub const STATUS_EVENT: &str = "recorder-status";
/// The worker changed the saved settings itself: the panel reads them again.
pub const SETTINGS_EVENT: &str = "recorder-settings";

fn settings_used(app: &AppHandle) {
    if let Err(error) = app.emit(SETTINGS_EVENT, ()) {
        eprintln!("Recorder settings could not reach the panel: {error}");
    }
}

#[derive(Default)]
struct Inner {
    snapshot: Snapshot,
    record_requested: bool,
    /// Exit once the current capture or save ends.
    quit_when_idle: bool,
    /// Exit after the Stop prompt that Quit opened is answered and carried out.
    quit_after_stop: bool,
    capturing: bool,
    /// A finished session's Saved or Discarded state stays until the panel has shown it.
    result: bool,
    /// A capture's failure stays until the panel has shown it.
    sticky: Option<Problem>,
    panel_open: bool,
    /// The panel has shown the result or failure while open.
    seen: bool,
    /// Hours the collector has processed this session.
    hours: u64,
    /// Bytes of the earlier recording this session continues, by campaign.
    earlier: Option<(String, Option<u64>)>,
    /// The campaign and flag keys whose flag was last asked for this session.
    flag_keys: Option<(String, Vec<String>)>,
    /// This session's game and Paradox user folders, for its content stack.
    flag_folders: Option<(PathBuf, PathBuf)>,
    /// The last flag shown for each campaign, shown again while not recording it.
    last_flags: BTreeMap<String, FlagImage>,
}

#[derive(Default)]
pub struct Recorder {
    control: Control,
    inner: Mutex<Inner>,
    wake: Condvar,
    /// The content stack of the session whose flags were last resolved.
    flag_content: Mutex<Option<(PathBuf, Arc<FlagContent>)>>,
}

/// Resolve the viewed country's flag off the record loop, then show it if
/// the same keys are still wanted.
fn request_flag(
    app: &AppHandle,
    session: PathBuf,
    (game_dir, user_dir): (PathBuf, PathBuf),
    campaign: String,
    keys: Vec<String>,
) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("recorder-flag".into())
        .spawn(move || {
            let recorder = Arc::clone(app.state::<Arc<Recorder>>().inner());
            let _priority = BackgroundPriority::enter();
            let flag = recorder
                .flag_content(&session, game_dir, user_dir)
                .and_then(|content| content.find(&keys))
                .unwrap_or_else(|error| {
                    eprintln!("The flag of {campaign} could not be resolved: {error}");
                    None
                });
            recorder.update(&app, |inner| {
                if !inner
                    .flag_keys
                    .as_ref()
                    .is_some_and(|(asked, asked_keys)| *asked == campaign && *asked_keys == keys)
                {
                    return;
                }
                if let Some(flag) = &flag {
                    inner.last_flags.insert(campaign.clone(), flag.clone());
                }
                if inner.snapshot.campaign.as_deref() == Some(campaign.as_str()) {
                    inner.snapshot.flag = flag;
                }
            });
        });
    if let Err(error) = spawned {
        eprintln!("The flag could not be resolved: {error}");
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Where recordings are saved: the chosen folder or the recorder's default.
pub fn save_folder(settings: &RecorderSettings) -> PathBuf {
    settings.effective_save_folder()
}

/// The save folder, created and in canonical form so campaign folders compare equal.
fn save_root(settings: &RecorderSettings) -> Result<PathBuf, String> {
    let root = save_folder(settings);
    fs::create_dir_all(&root)
        .and_then(|()| root.canonicalize())
        .map_err(|error| format!("Save folder {}: {error}", root.display()))
}

/// Unsupported builds stay refused for the process; other probe failures retry at the next poll.
struct GameProbe {
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
                .is_none_or(|error| controller::unsupported_build(error))
    }
}

/// How a session ended, for the record loop.
struct Ended {
    /// Do not attach to this game again automatically: the user stopped it or
    /// its capture failed.
    hold: bool,
    /// The session ended because a load was refused.
    rerouted: bool,
    /// The campaign Record was pressed for, refused only because it records
    /// into another folder: the next session for it is a Record session too.
    handover: Option<String>,
}

/// How a finished capture continues. A session that ended for a refused load
/// looks again at once: the campaign may be skipped, or record into its own
/// folder. The user's Stop (Save or Discard, even while a refusal
/// was already detaching) holds the game instead, until it restarts or Record
/// is pressed.
fn continuation(outcome: &CaptureOutcome, stopped: bool) -> Ended {
    let rerouted = outcome.rerouted && !stopped;
    Ended {
        hold: outcome.detach_requested && !outcome.process_exited && !rerouted,
        rerouted,
        handover: None,
    }
}

impl Ended {
    fn held() -> Self {
        Self {
            hold: true,
            rerouted: false,
            handover: None,
        }
    }
}

/// One session's admission rule. Record pressed makes the campaign it was
/// pressed for (or the first one loaded) record even when it does not record
/// automatically; every campaign still records only into its own folder.
struct SessionAdmission<'a> {
    settings: &'a Path,
    root: &'a Path,
    target: &'a Path,
    manual: bool,
    manual_campaign: RefCell<Option<String>>,
    /// The campaign the session's spool shows loaded first, if any: Record
    /// pressed at the menu belongs to it, even when it recorded and ended
    /// before any probe named it.
    first_loaded: Box<dyn Fn() -> Option<String> + 'a>,
    handover: RefCell<Option<String>>,
    /// Called once a load used up its campaign's pending New file.
    used_up: Box<dyn Fn() + 'a>,
}

impl SessionAdmission<'_> {
    /// `recorded`: this session's spool already holds the campaign's hours
    /// from an earlier load, which a pending New file keeps apart.
    fn admit(&self, campaign: &str, recorded: bool) -> Result<(), Refusal> {
        let pressed = self.manual && {
            let mut chosen = self.manual_campaign.borrow_mut();
            if chosen.is_none() {
                *chosen = Some((self.first_loaded)().unwrap_or_else(|| campaign.to_owned()));
            }
            chosen.as_deref() == Some(campaign)
        };
        // Unknown settings decide nothing: the load is left out, kept in the
        // session folder and reported, never guessed into a recording.
        let Ok(settings) = RecorderSettings::load_from(self.settings) else {
            return Err(Refusal::Settings);
        };
        if !pressed && !settings.records_campaign(campaign) {
            return Err(Refusal::Off);
        }
        // Its folder (a pending New file starting one) must be this session's.
        let target = targets::session_target_at(self.settings, self.root, campaign, recorded);
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

/// What the panel says about loads a session left out: those that recorded
/// hours, and why when it is the same for all.
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

    /// The session's recorded content stack, resolved once per session.
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
        let options = installed_content_core::ResolveOptions {
            game_dir,
            user_dir,
            cache_dir: replay::recorder_settings::directory(),
        };
        let content = Arc::new(FlagContent::load(&identity, &options)?);
        *cached = Some((session.to_path_buf(), Arc::clone(&content)));
        Ok(content)
    }

    /// Unchanged snapshots must still follow taskbar theme changes.
    fn update(&self, app: &AppHandle, change: impl FnOnce(&mut Inner)) {
        let (published, light) = {
            let mut inner = self.lock();
            let before = inner.snapshot.clone();
            change(&mut inner);
            if inner.panel_open && (inner.result || inner.sticky.is_some()) {
                inner.seen = true;
            }
            let published = (inner.snapshot != before).then(|| {
                let can_stop = inner.capturing
                    && inner.snapshot.phase == Phase::Recording
                    && !inner.snapshot.stopping;
                (inner.snapshot.clone(), can_stop)
            });
            (published, inner.snapshot.light())
        };
        match published {
            Some((snapshot, can_stop)) => {
                if let Err(error) = app.emit(STATUS_EVENT, &snapshot) {
                    eprintln!("Recorder status could not reach the panel: {error}");
                }
                tray::show(app, snapshot.light(), &snapshot.tooltip(), can_stop);
            }
            None => tray::follow_theme(app, light),
        }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.lock().snapshot.clone()
    }

    pub fn start(self: &Arc<Self>, app: AppHandle) -> std::io::Result<()> {
        let recorder = Arc::clone(self);
        std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || recorder.run(&app))
            .map(|_| ())
    }

    /// Record pressed: attach to the running game even if automatic recording
    /// is off for its campaign or it was stopped.
    pub fn record(&self) {
        self.lock().record_requested = true;
        self.wake.notify_all();
    }

    /// Save or Discard chosen on the Stop prompt.
    pub fn stop(&self, app: &AppHandle, discard: bool) {
        if !self.lock().capturing {
            return;
        }
        self.control.stop(discard);
        self.update(app, |inner| {
            inner.snapshot.stopping = true;
            if !discard {
                inner.snapshot.phase = Phase::Saving;
                inner.snapshot.progress = None;
                inner.snapshot.problem = None;
            }
        });
        self.wake.notify_all();
    }

    /// Right-click Stop recording: the panel opens on the Stop prompt.
    pub fn ask_stop(&self, app: &AppHandle) {
        let recording = {
            let inner = self.lock();
            inner.capturing && inner.snapshot.phase == Phase::Recording && !inner.snapshot.stopping
        };
        if recording {
            panel::open(app, None, true);
        }
    }

    /// Quit: while recording, the same choice as Stop first.
    pub fn quit(&self, app: &AppHandle) {
        let mut inner = self.lock();
        if !inner.capturing {
            drop(inner);
            app.exit(0);
        } else if inner.snapshot.phase == Phase::Recording && !inner.snapshot.stopping {
            inner.quit_after_stop = true;
            drop(inner);
            panel::open(app, None, true);
        } else {
            // Nothing recorded yet, or already saving: finish, then exit.
            inner.quit_when_idle = true;
            let stopping = inner.snapshot.stopping;
            drop(inner);
            if !stopping {
                self.control.stop(false);
            }
            self.wake.notify_all();
        }
    }

    /// The Stop prompt was dismissed (Stop pressed again): a Quit waiting on it
    /// is cancelled too, unless Save or Discard is already being carried out.
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
                // Closing the Stop prompt that Quit opened cancels the quit.
                inner.quit_after_stop = false;
            }
            if std::mem::take(&mut inner.seen) {
                inner.result = false;
                inner.sticky = None;
            }
        });
        self.wake.notify_all();
    }

    /// Settings changed: look again now rather than at the next poll.
    pub fn settings_changed(&self) {
        self.wake.notify_all();
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
        // The game process the user stopped, or whose capture failed. It is not
        // attached again until it exits or Record is pressed.
        let mut held: Option<u64> = None;
        let mut game: Option<GameProbe> = None;
        // The last session ended because a load was refused.
        let mut rerouted = false;
        // The game and campaign a Record session hands over to its own folder.
        let mut handover: Option<(u64, String)> = None;
        loop {
            if self.exit_requested() {
                app.exit(0);
                return;
            }
            let settings = RecorderSettings::load().unwrap_or_else(|error| {
                eprintln!("Recorder settings could not be read: {error}");
                RecorderSettings::default()
            });
            let manual = std::mem::take(&mut self.lock().record_requested);
            match controller::single_game() {
                Err(error) => self.idle(
                    app,
                    &settings,
                    true,
                    None,
                    Some(Problem {
                        text: controller::problem_text(&error, false).into(),
                        detail: error,
                    }),
                ),
                Ok(None) => {
                    held = None;
                    handover = None;
                    game = None;
                    self.idle(app, &settings, false, None, None);
                }
                Ok(Some(pid)) => match controller::game_start_time(pid) {
                    // The process is going away; look again shortly.
                    Err(_) => {}
                    Ok(start) => {
                        if manual || held.is_some_and(|held| held != start) {
                            held = None;
                        }
                        if !game.as_ref().is_some_and(|game| game.settled(start)) {
                            // An unsupported build cannot be read; its capture refuses it.
                            let probe = Probe::open(pid);
                            if let Err(error) = &probe
                                && game.as_ref().is_none_or(|game| {
                                    game.start != start || game.probe.as_ref().err() != Some(error)
                                })
                            {
                                eprintln!("The running game could not be read: {error}");
                            }
                            game = Some(GameProbe { start, probe });
                        }
                        let probe = game.as_ref().and_then(|game| game.probe.as_ref().ok());
                        let loaded = probe.and_then(Probe::loaded);
                        // Record pressed for a campaign that turned out to belong in
                        // another folder carries over to its session there.
                        if handover.as_ref().is_some_and(|(game, _)| *game != start) {
                            handover = None;
                        }
                        let manual = manual
                            || loaded.as_ref().is_some_and(|loaded| {
                                handover
                                    .take_if(|(_, campaign)| *campaign == loaded.campaign)
                                    .is_some()
                            });
                        // A refused load is routed by what the game has loaded.
                        // If that cannot be read, attaching at the menu again would
                        // only meet the same refusal, so the game is held instead.
                        if std::mem::take(&mut rerouted) && loaded.is_none() && !manual {
                            held = Some(start);
                        }
                        let wanted = match &loaded {
                            Some(loaded) => settings.records_campaign(&loaded.campaign),
                            None => settings.record_every_campaign,
                        };
                        if held.is_none() && (manual || wanted) {
                            let ended = self.capture(app, pid, &settings, manual, loaded, probe);
                            if ended.hold {
                                held = Some(start);
                            }
                            rerouted = ended.rerouted;
                            handover = ended.handover.map(|campaign| (start, campaign));
                            continue;
                        }
                        self.idle(app, &settings, true, loaded.as_ref(), None);
                    }
                },
            }
            self.wait(Duration::from_secs(1));
        }
    }

    /// The snapshot while nothing is being captured. `loaded` is the campaign
    /// the running game has loaded, when it can be read.
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
            let recording = targets::recording_path(&folder, &loaded.campaign);
            Some((loaded.clone(), recording))
        });
        self.update(app, |inner| {
            let automatic = settings.record_every_campaign;
            if inner.result {
                inner.snapshot.automatic = automatic;
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
                inner.snapshot.continued = recording.exists();
                inner.snapshot.flag = inner.last_flags.get(&loaded.campaign).cloned();
                inner.snapshot.campaign = Some(loaded.campaign);
                inner.snapshot.recording = Some(recording.display().to_string());
                inner.snapshot.hour = Some(loaded.hour);
            }
        });
    }

    /// Record one session.
    fn capture(
        &self,
        app: &AppHandle,
        pid: u32,
        settings: &RecorderSettings,
        manual: bool,
        loaded: Option<LoadedCampaign>,
        probe: Option<&Probe>,
    ) -> Ended {
        let folders = (|| -> Result<(PathBuf, PathBuf, PathBuf), String> {
            let root = save_root(settings)?;
            let target = match &loaded {
                Some(loaded) => {
                    let (target, used_up) = targets::session_target(&root, &loaded.campaign)?;
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
            let recording = targets::recording_path(&target, &loaded.campaign);
            let bytes = fs::metadata(&recording).ok().map(|metadata| metadata.len());
            (loaded.clone(), recording, bytes)
        });
        let flag_folders = probe
            .and_then(Probe::game_dir)
            .map(|game_dir| (game_dir, user.clone()));
        self.update(app, |inner| {
            inner.capturing = true;
            inner.result = false;
            inner.sticky = None;
            inner.seen = false;
            inner.hours = 0;
            inner.earlier = None;
            inner.flag_keys = None;
            inner.flag_folders = flag_folders;
            inner.snapshot = Snapshot {
                phase: Phase::Waiting,
                automatic,
                ..Snapshot::default()
            };
            // A loaded campaign is recording from attachment; its first hour
            // follows when the game runs.
            if let Some((loaded, recording, bytes)) = opening {
                inner.earlier = Some((loaded.campaign.clone(), bytes));
                inner.snapshot.flag = inner.last_flags.get(&loaded.campaign).cloned();
                let snapshot = &mut inner.snapshot;
                snapshot.phase = Phase::Recording;
                snapshot.started_at = Some(now_ms());
                snapshot.campaign = Some(loaded.campaign);
                snapshot.recording = Some(recording.display().to_string());
                snapshot.continued = bytes.is_some();
                snapshot.hour = Some(loaded.hour);
                snapshot.bytes = bytes;
            }
        });
        let settings_path = RecorderSettings::path();
        // The session folder, known once recording starts.
        let session_dir: RefCell<Option<PathBuf>> = RefCell::new(None);
        let admission = SessionAdmission {
            settings: &settings_path,
            root: &root,
            target: &target,
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
        let current = || {
            probe
                .and_then(Probe::loaded)
                .map(|loaded| (loaded.campaign, loaded.load))
        };
        let started = Cell::new(false);
        let stage = |stage: Stage<'_>| match stage {
            Stage::Preparing => {}
            Stage::Recording { session, status } => {
                started.set(true);
                session_dir
                    .borrow_mut()
                    .get_or_insert_with(|| session.to_path_buf());
                if !self.control.stopping() {
                    self.live(app, &target, session, status, probe);
                }
            }
            Stage::Saving { records } => self.update(app, |inner| {
                inner.snapshot.phase = Phase::Saving;
                inner.snapshot.problem = None;
                inner.snapshot.progress = snapshot::saving_progress(records, inner.hours);
            }),
        };
        let recording = Recording {
            output: &root,
            target: &target,
            user: &user,
            game_score: settings.game_score,
            operation_outcomes: settings.operation_outcomes,
            politics_capture: settings.politics,
            admit: &admit,
            loaded: &current,
            // Attached for this load: it records even if the game closes before
            // the first poll.
            opening: loaded
                .as_ref()
                .map(|loaded| (loaded.campaign.as_str(), loaded.load)),
        };
        let result = controller::record(pid, &recording, &self.control, &stage);
        let mut ended = self.finish(app, result, started.get());
        if ended.rerouted {
            ended.handover = admission.handover.take();
        }
        ended
    }

    /// Fold one collector status publication into the snapshot. Before the
    /// collector names a campaign, the game's loaded campaign stands in.
    fn live(
        &self,
        app: &AppHandle,
        target: &Path,
        session: &Path,
        status: Option<&Value>,
        probe: Option<&Probe>,
    ) {
        let mut live = status.map(snapshot::read_live).unwrap_or_default();
        if live.campaign.is_none()
            && let Some(loaded) = probe.and_then(Probe::loaded)
        {
            live.campaign = Some(loaded.campaign);
            live.hour = live.hour.or(Some(loaded.hour));
        }
        let recording = live
            .campaign
            .as_ref()
            .map(|campaign| targets::recording_path(target, campaign));
        // The earlier recording is not written until this session saves, so its
        // size is read once per campaign.
        let earlier = match (&live.campaign, &recording) {
            (Some(campaign), Some(path)) => {
                let cached = self.lock().earlier.clone();
                match cached {
                    Some((id, bytes)) if &id == campaign => bytes,
                    _ => {
                        let bytes = fs::metadata(path).ok().map(|metadata| metadata.len());
                        self.lock().earlier = Some((campaign.clone(), bytes));
                        bytes
                    }
                }
            }
            _ => None,
        };
        // A new set of flag keys (another country, cosmetic tag or ruling group)
        // or another campaign asks for its flag once.
        let flag_request = match (&live.campaign, live.flag_keys.take()) {
            (Some(campaign), Some(keys)) => {
                let mut inner = self.lock();
                if inner
                    .flag_keys
                    .as_ref()
                    .is_some_and(|(asked, asked_keys)| asked == campaign && *asked_keys == keys)
                {
                    None
                } else {
                    inner.flag_keys = Some((campaign.clone(), keys.clone()));
                    inner
                        .flag_folders
                        .clone()
                        .map(|folders| (folders, campaign.clone(), keys))
                }
            }
            _ => None,
        };
        if let Some((folders, campaign, keys)) = flag_request {
            request_flag(app, session.to_path_buf(), folders, campaign, keys);
        }
        self.update(app, |inner| {
            inner.hours = live.hours;
            let snapshot = &mut inner.snapshot;
            snapshot.session = Some(session.display().to_string());
            if let Some(campaign) = live.campaign {
                if snapshot.phase != Phase::Recording {
                    snapshot.phase = Phase::Recording;
                    snapshot.started_at = Some(now_ms());
                }
                // Another campaign shows its own last flag until its new one resolves.
                if snapshot.campaign.as_deref() != Some(campaign.as_str()) {
                    snapshot.flag = inner.last_flags.get(&campaign).cloned();
                }
                snapshot.campaign = Some(campaign);
                snapshot.recording = recording.map(|path| path.display().to_string());
                snapshot.continued = earlier.is_some();
                snapshot.hour = live.hour.or(snapshot.hour);
                snapshot.bytes = Some(earlier.unwrap_or(0) + live.spool_bytes);
            }
            snapshot.problem = live.problem;
        });
    }

    fn finish(
        &self,
        app: &AppHandle,
        result: Result<CaptureOutcome, String>,
        started: bool,
    ) -> Ended {
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                self.fail(app, error, started);
                return Ended::held();
            }
        };
        let ended = continuation(&outcome, self.control.stopping());
        if outcome.discarded {
            self.update(app, |inner| {
                inner.capturing = false;
                inner.result = true;
                let snapshot = &mut inner.snapshot;
                snapshot.phase = Phase::Discarded;
                snapshot.stopping = false;
                snapshot.progress = None;
                snapshot.problem = None;
            });
            return ended;
        }
        let campaign = self.lock().snapshot.campaign.clone();
        let saved = outcome
            .replays
            .iter()
            .find(|path| {
                campaign.as_deref().is_some_and(|campaign| {
                    path.file_stem().and_then(|stem| stem.to_str()) == Some(campaign)
                })
            })
            .or(outcome.replays.last())
            .cloned();
        let omitted = omitted_loads(&outcome.omitted);
        match saved {
            Some(path) => {
                let span = Replay::read_hour_span(&path).ok().flatten();
                let bytes = fs::metadata(&path).ok().map(|metadata| metadata.len());
                self.update(app, |inner| {
                    inner.capturing = false;
                    inner.result = true;
                    let snapshot = &mut inner.snapshot;
                    snapshot.phase = Phase::Saved;
                    snapshot.omitted = omitted;
                    snapshot.campaign = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .map(str::to_owned)
                        .or(snapshot.campaign.take());
                    snapshot.recording = Some(path.display().to_string());
                    snapshot.first_hour = span.map(|(first, _)| first);
                    snapshot.hour = span.map(|(_, last)| last).or(snapshot.hour);
                    snapshot.bytes = bytes;
                    snapshot.stopping = false;
                    snapshot.progress = None;
                    snapshot.problem = None;
                });
            }
            // Nothing was saved: nothing reached the spool (the game closed at
            // its menu, say), or every load was left out, which stays shown
            // until the panel has shown it.
            None => self.update(app, |inner| {
                inner.capturing = false;
                inner.snapshot.stopping = false;
                inner.snapshot.progress = None;
                if omitted.is_some() {
                    inner.result = true;
                    inner.snapshot.phase = Phase::NotRecording;
                    inner.snapshot.omitted = omitted;
                }
            }),
        }
        ended
    }

    fn fail(&self, app: &AppHandle, error: String, started: bool) {
        eprintln!("Recording: {error}");
        let problem = Problem {
            text: controller::problem_text(&error, started).into(),
            detail: error,
        };
        self.update(app, |inner| {
            inner.capturing = false;
            inner.sticky = Some(problem.clone());
            let snapshot = &mut inner.snapshot;
            snapshot.phase = Phase::NotRecording;
            snapshot.stopping = false;
            snapshot.progress = None;
            snapshot.problem = Some(problem);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::recorder_settings::NextLoad;

    const CAMPAIGN: &str = "00000000-0000-4000-8000-000000000000";
    const OTHER: &str = "00000000-0000-4000-8000-000000000001";

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-admission-{name}-{}-{}",
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
        // Record at the menu, then the campaign whose recording lives in "(2)":
        // not admitted into the save folder, and handed over to its own folder.
        let menu = admission(&settings, &root, &root, true);
        assert_eq!(menu.admit(CAMPAIGN, false), Err(Refusal::Elsewhere));
        assert_eq!(menu.handover.borrow().as_deref(), Some(CAMPAIGN));
        // Without Record the campaign does not record at all, and nothing is handed over.
        let automatic = admission(&settings, &root, &root, false);
        assert_eq!(automatic.admit(CAMPAIGN, false), Err(Refusal::Off));
        assert!(automatic.handover.borrow().is_none());
        // The handed-over Record session in the campaign's folder admits it.
        let folder = root.join(format!("{CAMPAIGN} (2)"));
        let own = admission(&settings, &root, &folder, true);
        assert_eq!(own.admit(CAMPAIGN, false), Ok(()));
        // Record covers only the campaign it was pressed for.
        assert_eq!(own.admit(OTHER, false), Err(Refusal::Off));
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn unreadable_settings_leave_a_load_out_instead_of_guessing_its_file() {
        let scratch = scratch("unreadable-settings");
        let settings = scratch.join("settings.json");
        let root = scratch.join("recordings");
        fs::write(&settings, b"not settings").unwrap();
        // Neither whether the campaign records nor its folder is known, even
        // with Record pressed for it.
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
        fs::write(
            targets::recording_path(&root, CAMPAIGN),
            b"earlier recording",
        )
        .unwrap();
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
        assert_eq!(
            fs::read(targets::recording_path(&root, CAMPAIGN)).unwrap(),
            b"earlier recording"
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn dismissing_the_stop_prompt_cancels_a_pending_quit() {
        let recorder = Recorder::default();
        recorder.lock().quit_after_stop = true;
        recorder.cancel_stop_prompt();
        assert!(!recorder.lock().quit_after_stop);
        // Once Save or Discard is being carried out, the quit stands.
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
        // The first recording of the campaign is still only this session's
        // spool; New file then a reload must not join the two loads.
        assert!(!targets::recording_path(&root, CAMPAIGN).exists());
        let session = admission(&settings, &root, &root, false);
        assert_eq!(session.admit(CAMPAIGN, true), Err(Refusal::Elsewhere));
        let saved = RecorderSettings::load_from(&settings).unwrap();
        assert_eq!(
            saved.campaigns[CAMPAIGN].folder.as_deref(),
            Some(&*format!("{CAMPAIGN} (2)"))
        );
        // The next session records the reload there.
        let folder = root.join(format!("{CAMPAIGN} (2)"));
        assert_eq!(
            admission(&settings, &root, &folder, false).admit(CAMPAIGN, false),
            Ok(())
        );
        fs::remove_dir_all(scratch).unwrap();
    }

    #[test]
    fn stop_while_a_refusal_detaches_holds_the_game_instead_of_attaching_again() {
        let rerouting = CaptureOutcome {
            detach_requested: true,
            rerouted: true,
            ..CaptureOutcome::default()
        };
        // A refusal alone looks again at once.
        let ended = continuation(&rerouting, false);
        assert!(!ended.hold && ended.rerouted);
        // Save or Discard chosen during that detach holds the game.
        let ended = continuation(&rerouting, true);
        assert!(ended.hold && !ended.rerouted);
        // The game exiting never holds.
        let exited = CaptureOutcome {
            process_exited: true,
            ..rerouting
        };
        assert!(!continuation(&exited, true).hold);
    }

    /// Record pressed at the menu with automatic recording off: campaign A
    /// records briefly and ends before a probe, then the probe names B.
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
        // With nothing spooled yet, the first campaign asked about is the first loaded.
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
        // A load left out before it recorded an hour lost nothing.
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
        // Different reasons: the count alone.
        assert_eq!(
            omitted_loads(&[omission(2, Some(Refusal::Off)), omission(1, None)]),
            Some(OmittedLoads {
                loads: 2,
                reason: None,
            })
        );
    }
}
