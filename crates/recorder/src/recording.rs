mod game_facts;
mod history_requests;
mod progress_io;
mod scripted_map_demand;
use crate::{
    directories::{default_capture_directory, resolve_user_directory},
    replay_io,
    windows::{self, Event, Process, ProcessMemory},
};
use engine_executable::build::hash_executable;
use content_reader_layers::content::{GatherOptions, GatheredContent, gather_cancellable};
use content_reader_localisation::localisation::Localisation;
use history_requests::HistoryRequest;
use recorder_platform::games::{Discovery, GameProcess, OtherGames, choose_game};
use replay_settings::recorder_settings::RecorderSettings;
use replay_data::identity::IdentityCodec;
use replay_data::{identity::SessionStart, scope::CaptureMode};
use replay_data::{witness::TerminalWitness, witness::WITNESS_FILE_NAME};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use windows_sys::Win32::System::{
    Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
    Performance::QueryPerformanceFrequency,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub campaign: String,
    pub unsaved: replay_io::Unsaved,
    pub saving: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetAside {
    Saved(String, Option<PathBuf>),
    Discarded(String),
    Failed(String, String),
}

#[derive(Default)]
struct Settling {
    save: AtomicBool,
    requests: std::sync::Mutex<Vec<(String, bool)>>,
    waiting: std::sync::Mutex<Option<Vec<Waiting>>>,
    done: std::sync::Mutex<Vec<SetAside>>,
    loaded: std::sync::Mutex<Option<String>>,
    left: std::sync::Mutex<BTreeSet<String>>,
    stopped: Arc<AtomicBool>,
    collector: bool,
}

fn locked<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

type Settled = BTreeMap<String, (replay_io::Unsaved, Option<Waiting>, bool)>;

struct RetakeHistory<F: FnMut(&str) -> Result<(), String>> {
    request: F,
    armed: bool,
}

impl<F: FnMut(&str) -> Result<(), String>> Drop for RetakeHistory<F> {
    fn drop(&mut self) {
        if self.armed {
            if let Err(error) = (self.request)("retake") {
                eprintln!("The collector could not hold its history again: {error}");
            }
        }
    }
}

#[cfg(test)]
fn with_lent_history<T>(
    request: impl FnMut(&str) -> Result<(), String>,
    work: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let mut guard = RetakeHistory {
        request,
        armed: true,
    };
    let result = (guard.request)("lend").and_then(|()| work());
    let retaken = (guard.request)("retake");
    guard.armed = retaken.is_err();
    match (result, retaken) {
        (result, Ok(())) => result,
        (Ok(_), Err(error)) => Err(format!(
            "the collector could not hold its history again: {error}"
        )),
        (Err(error), Err(retake)) => {
            Err(format!("{error}; history could not be retaken: {retake}"))
        }
    }
}

fn ask_collector_until(
    session: &Path,
    action: &str,
    subject: &str,
    stopped: &AtomicBool,
) -> Result<(), String> {
    let request = HistoryRequest::publish(session, action, subject)?;
    let result = request.wait(stopped);
    if result.is_err() {
        request.return_history()?;
    }
    result
}

fn history_loan(
    loan: &mut Option<HistoryRequest>,
    session: &Path,
    action: &str,
    campaign: &str,
    stopped: &AtomicBool,
) -> Result<(), String> {
    if action == "retake" {
        return match loan {
            Some(loan) => loan.retake(stopped),
            None => Ok(()),
        };
    }
    HistoryRequest::retry_returns(session, campaign, stopped)?;
    match HistoryRequest::publish(session, action, campaign) {
        Ok(request) => *loan = Some(request),
        Err(error) => {
            *loan = HistoryRequest::lent(session, campaign);
            return Err(error);
        }
    }
    loan.as_ref().unwrap().wait(stopped)
}

fn with_history<T>(
    settling: &Settling,
    session: &Path,
    campaign: &str,
    work: impl FnOnce() -> Result<T, String>,
) -> Option<Result<T, String>> {
    if !settling.collector {
        return Some(work());
    }
    let mut loan = None;
    let mut guard = RetakeHistory {
        request: |action: &str| {
            history_loan(&mut loan, session, action, campaign, &settling.stopped)
        },
        armed: true,
    };
    if let Err(error) = (guard.request)("release") {
        eprintln!("Campaign {campaign} cannot be saved now: {error}");
        return None;
    }
    let result = work();
    let retaken = (guard.request)("retake");
    guard.armed = retaken.is_err();
    if let Err(error) = retaken {
        eprintln!("The collector could not hold campaign {campaign}'s history again: {error}");
    }
    Some(result)
}

fn settle(
    session: &Path,
    target: &Path,
    live: &mut replay_io::Live,
    settling: &Settling,
    settled: &mut Settled,
    naming: replay_io::Naming<'_>,
) -> Result<(), String> {
    let root = session
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| failure_text(session, "not in a capture root"))?;
    let mut requests = latest_choices(std::mem::take(&mut *locked(&settling.requests)));
    let requests = &mut requests;
    let mut changed = false;
    let campaigns = live.unsaved_campaigns();
    let before = settled.len();
    settled.retain(|campaign, _| campaigns.contains(campaign));
    changed |= settled.len() != before;
    for campaign in &campaigns {
        let campaign = campaign.clone();
        let at = requests.iter().rposition(|(asked, _)| *asked == campaign);
        if locked(&settling.loaded).as_ref() == Some(&campaign) {
            changed |= settled.remove(&campaign).is_some();
            if at.is_some() {
                requests.retain(|(asked, _)| *asked != campaign);
                locked(&settling.done).push(SetAside::Failed(
                    campaign,
                    "it is being recorded again; save or discard it once the game leaves it"
                        .to_owned(),
                ));
            }
            continue;
        }
        let Some(unsaved) = live.unsaved(&campaign) else {
            continue;
        };
        let request = at.map(|at| requests[at].1);
        let unsaved_waits = settled.get(&campaign).is_some_and(|(_, waiting, failed)| {
            !failed
                && waiting
                    .as_ref()
                    .is_some_and(|waiting| !waiting.unsaved.undecided)
        });
        if request.is_none()
            && !(unsaved_waits && settling.save.load(Ordering::Acquire))
            && settled
                .get(&campaign)
                .is_some_and(|(known, _, _)| *known == unsaved)
        {
            continue;
        }
        changed = true;
        if request == Some(true) {
            requests.retain(|(asked, _)| *asked != campaign);
            let discarded = with_history(settling, session, &campaign, || {
                live.discard(session, target, &campaign, replay_io::Discarded::Unsaved)?;
                discard_recording(root, target, &campaign)
            });
            let Some(discarded) = discarded else {
                requeue(settling, campaign, true);
                continue;
            };
            settled.remove(&campaign);
            locked(&settling.done).push(match discarded {
                Ok(()) => SetAside::Discarded(campaign),
                Err(error) => SetAside::Failed(campaign, error),
            });
            continue;
        }
        requests.retain(|(asked, _)| *asked != campaign);
        let save = request == Some(false) || settling.save.load(Ordering::Acquire);
        let mut waiting = Waiting {
            campaign: campaign.clone(),
            saving: save && !unsaved.undecided,
            unsaved: unsaved.clone(),
        };
        if waiting.saving {
            settled.insert(
                campaign.clone(),
                (unsaved.clone(), Some(waiting.clone()), false),
            );
            *locked(&settling.waiting) = Some(waiting_list(settled));
            let saved = with_history(settling, session, &campaign, || {
                let saved = live.save(session, target, &campaign, naming)?;
                unlist_waiting_sessions(root, target, &campaign, Some(session))?;
                Ok(saved)
            });
            let Some(saved) = saved else {
                settled.insert(
                    campaign.clone(),
                    (replay_io::Unsaved::default(), Some(waiting), false),
                );
                if let Some(request) = request {
                    requeue(settling, campaign, request);
                }
                continue;
            };
            locked(&settling.done).push(match saved {
                Ok(replay) => {
                    settled.insert(campaign.clone(), (unsaved, None, false));
                    SetAside::Saved(campaign, replay)
                }
                Err(error) => {
                    waiting.saving = false;
                    settled.insert(campaign.clone(), (unsaved, Some(waiting), true));
                    SetAside::Failed(campaign, error)
                }
            });
            continue;
        }
        if request == Some(false) && unsaved.undecided {
            locked(&settling.done).push(SetAside::Failed(
                campaign.clone(),
                "it can be saved once recording stops".into(),
            ));
        }
        settled.insert(campaign, (unsaved, Some(waiting), false));
    }
    if settling.save.load(Ordering::Acquire) {
        let left = std::mem::take(&mut *locked(&settling.left));
        for campaign in left {
            if campaigns.contains(&campaign) {
                continue;
            }
            if locked(&settling.loaded).as_ref() == Some(&campaign) {
                locked(&settling.left).insert(campaign);
                continue;
            }
            match with_history(settling, session, &campaign, || {
                save_recording(root, target, &campaign, naming)
            }) {
                None => {
                    locked(&settling.left).insert(campaign);
                }
                Some(Ok(None)) => {}
                Some(Ok(Some(saved))) => {
                    locked(&settling.done).push(SetAside::Saved(campaign, Some(saved)));
                }
                Some(Err(error)) => locked(&settling.done).push(SetAside::Failed(campaign, error)),
            }
        }
    }
    for (campaign, discard) in std::mem::take(requests) {
        if locked(&settling.loaded).as_ref() == Some(&campaign) {
            locked(&settling.done).push(SetAside::Failed(
                campaign,
                "it is being recorded again; save or discard it once the game leaves it".to_owned(),
            ));
            continue;
        }
        let result = with_history(settling, session, &campaign, || {
            if discard {
                live.discard(session, target, &campaign, replay_io::Discarded::Unsaved)?;
                discard_recording(root, target, &campaign).map(|()| None)
            } else {
                save_recording(root, target, &campaign, naming).map(Some)
            }
        });
        let Some(result) = result else {
            requeue(settling, campaign, discard);
            continue;
        };
        locked(&settling.done).push(match result {
            Ok(None) => SetAside::Discarded(campaign),
            Ok(Some(saved)) => SetAside::Saved(campaign, saved),
            Err(error) => SetAside::Failed(campaign, error),
        });
    }
    if changed {
        *locked(&settling.waiting) = Some(waiting_list(settled));
    }
    Ok(())
}

fn waiting_list(settled: &Settled) -> Vec<Waiting> {
    settled
        .values()
        .filter_map(|(_, waiting, _)| waiting.clone())
        .collect()
}

fn background_naming<T>(work: impl FnOnce() -> T) -> T {
    job_priority::JobPriority::run(Arc::new(AtomicBool::new(true)), work)
}

type AheadResult = (replay_io::Live, Option<Localisation>, Duration);

struct FinalizingAhead {
    packing: Option<Arc<replay_settings::packing::Packing>>,
    stop: Arc<AtomicBool>,
    promoted: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<AheadResult>>,
    asked: Arc<AtomicBool>,
    names: Arc<Mutex<BTreeMap<String, String>>>,
}

fn predicted_names(
    session: &Path,
    target: &Path,
    live: &mut replay_io::Live,
    naming: replay_io::Naming<'_>,
) -> BTreeMap<String, String> {
    let mut names = live.names(target, naming);
    let Ok(root) = root_of(session) else {
        return names;
    };
    let waiting: BTreeSet<_> = session_folders(root)
        .iter()
        .flat_map(|ended| waiting_campaigns(ended))
        .filter(|waiting| waiting.target == target && !names.contains_key(&waiting.campaign))
        .map(|waiting| waiting.campaign)
        .collect();
    for campaign in waiting {
        if let Some(name) = replay_io::waiting_name(target, &campaign, naming) {
            names.insert(campaign, name);
        }
    }
    names
}

impl FinalizingAhead {
    fn start(
        session: &Path,
        target: &Path,
        content: &GatheredContent,
        settling: Option<Arc<Settling>>,
        idle: Option<Arc<Event>>,
    ) -> Self {
        let stop = settling
            .as_ref()
            .map(|settling| Arc::clone(&settling.stopped))
            .unwrap_or_default();
        let (session, target, stopping) = (session.to_owned(), target.to_owned(), stop.clone());
        let (layers, language) = (content.layers.clone(), content.language.clone());
        let names = Arc::<Mutex<BTreeMap<String, String>>>::default();
        let (published, asked) = (Arc::clone(&names), Arc::new(AtomicBool::new(false)));
        let asking = Arc::clone(&asked);
        let promoted = Arc::new(AtomicBool::new(false));
        let packing = replay_settings::packing::Packing::new()
            .map(Arc::new)
            .inspect_err(|error| {
                eprintln!("Continuous packing could not start; journal retained: {error}");
            })
            .ok();
        if let (Some(packing), Some(idle)) = (&packing, idle) {
            let gate_stop = Arc::clone(&stop);
            let gate_promoted = Arc::clone(&promoted);
            let gate_asked = Arc::clone(&asked);
            let origin = std::time::Instant::now();
            let idle_since_ms = std::sync::atomic::AtomicU64::new(0);
            packing.set_gate(move || {
                const IDLE_QUIET_MS: u64 = 1000;
                const POLL_MS: u64 = 11;
                loop {
                    if gate_stop.load(Ordering::Acquire)
                        || gate_promoted.load(Ordering::Acquire)
                        || gate_asked.load(Ordering::Acquire)
                    {
                        return;
                    }
                    if idle.handle.signaled().unwrap_or(true) {
                        let now_ms =
                            origin.elapsed().as_millis().min(u64::MAX as u128 - 1) as u64 + 1;
                        let since = idle_since_ms.load(Ordering::Acquire);
                        if since == 0 {
                            let _ = idle_since_ms.compare_exchange(
                                0,
                                now_ms,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        } else if now_ms.saturating_sub(since) >= IDLE_QUIET_MS {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(POLL_MS));
                    } else {
                        idle_since_ms.store(0, Ordering::Release);
                        let _ = idle.handle.wait(50);
                        if idle.handle.signaled().unwrap_or(false) {
                            let now_ms =
                                origin.elapsed().as_millis().min(u64::MAX as u128 - 1) as u64 + 1;
                            idle_since_ms.store(now_ms, Ordering::Release);
                            std::thread::sleep(Duration::from_millis(POLL_MS));
                        }
                    }
                }
            });
        }
        let carried = replay_settings::exit::Carry::current();
        let thread = packing.clone().and_then(|packing| std::thread::Builder::new()
            .name("finalize ahead".into())
            .spawn(move || carried.run(|| {
                let _pass_progress = progress_io::passes(&session);
                packing.run(|| {
                packing.wait_for_game();
                let localisation = content_reader_localisation::localisation::read(&layers, &language)
                    .inspect_err(|error| {
                        eprintln!("Countries are named by tag: the localisation could not be read: {error}");
                    })
                    .ok();
                drop(layers);
                let stopped = || stopping.load(Ordering::Acquire);
                let mut live = replay_io::Live::default();
                let mut writing = Duration::ZERO;
                let mut failed = false;
                let mut fail = |error: String| {
                    if !std::mem::replace(&mut failed, true) {
                        eprintln!("Writing the recordings paused: {error}");
                    }
                };
                let mut settled = Settled::new();
                let settings = RecorderSettings::path();
                let saves = |_: &str| {
                    settling
                        .as_ref()
                        .is_none_or(|settling| settling.save.load(Ordering::Acquire))
                };
                while !stopped() {
                    packing.wait_for_game();
                    let before = thread_cpu();
                    let naming = replay_io::Naming {
                        localisation: localisation.as_ref(),
                        settings: &settings,
                        report: None,
                    };
                    let grew = {
                        let _phase = replay_settings::packing::Packing::phase(replay_settings::packing::Phase::Drain);
                        match live.advance(&session, &target, &saves, naming, &stopped) {
                            Ok(grew) => grew,
                            Err(error) => {
                                fail(error);
                                false
                            }
                        }
                    };
                    if asking.swap(false, Ordering::AcqRel) {
                        let named = predicted_names(&session, &target, &mut live, naming);
                        *published
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = named;
                    }
                    writing += thread_cpu() - before;
                    if let Some(settling) = &settling
                        && let Err(error) =
                            settle(&session, &target, &mut live, settling, &mut settled, naming)
                    {
                        eprintln!("Set-aside campaigns could not be looked at: {error}");
                    }
                    if !grew && !stopped() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                (live, localisation, writing)
            }) }))
            .ok());
        Self {
            packing,
            stop,
            promoted,
            thread,
            asked,
            names,
        }
    }

    fn promote(&self) {
        self.promoted.store(true, Ordering::Release);
        if let Some(packing) = &self.packing {
            packing.promote();
        }
    }

    fn ask_names(&self) {
        self.asked.store(true, Ordering::Release);
    }

    fn names(&self) -> BTreeMap<String, String> {
        self.names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn finish(mut self) -> (replay_io::Live, Option<Localisation>) {
        self.promote();
        self.stop.store(true, Ordering::Release);
        match self.thread.take().map(std::thread::JoinHandle::join) {
            Some(Ok((live, localisation, writing))) => {
                println!(
                    "Live writer: lifetime_cpu_s={:.3}",
                    writing.as_secs_f64(),
                );
                (live, localisation)
            }
            _ => (replay_io::Live::default(), None),
        }
    }
}

impl Drop for FinalizingAhead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn thread_cpu() -> Duration {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let mut times: [FILETIME; 4] = unsafe { std::mem::zeroed() };
    let [creation, exit, kernel, user] = &mut times;
    if unsafe { GetThreadTimes(GetCurrentThread(), creation, exit, kernel, user) } == 0 {
        return Duration::ZERO;
    }
    let ticks =
        |time: &FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

fn performance_counter() -> Option<u64> {
    let mut counter = 0;
    unsafe { windows_sys::Win32::System::Performance::QueryPerformanceCounter(&mut counter) }
        .ne(&0)
        .then_some(counter as u64)
}

fn counter_seconds(start: Option<u64>, frequency: u64) -> Option<f64> {
    let elapsed = performance_counter()?.checked_sub(start?)?;
    (frequency != 0).then_some(elapsed as f64 / frequency as f64)
}

fn system_times() -> Option<(u64, u64)> {
    use windows_sys::Win32::{Foundation::FILETIME, System::Threading::GetSystemTimes};
    let mut idle = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut kernel = idle;
    let mut user = idle;
    if unsafe { GetSystemTimes(&mut idle, &mut kernel, &mut user) } == 0 {
        return None;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Some((ticks(idle), ticks(kernel) + ticks(user)))
}

fn system_busy(before: Option<(u64, u64)>) -> Option<f64> {
    let (idle, total) = system_times()?;
    let (before_idle, before_total) = before?;
    let total = total.checked_sub(before_total)?;
    let idle = idle.checked_sub(before_idle)?;
    (total != 0).then_some(100.0 * total.saturating_sub(idle) as f64 / total as f64)
}

fn requested_stop_counter(control: &Control, directory: &Path) -> Option<u64> {
    Some(control.stop_qpc.load(Ordering::Acquire))
        .filter(|qpc| *qpc != 0)
        .or_else(|| {
            fs::read_to_string(directory.join("stop.request"))
                .ok()?
                .trim()
                .parse()
                .ok()
        })
        .filter(|qpc| *qpc != 0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stop {
    Save(Option<String>),
    Discard(String),
}

#[derive(Debug, Default)]
struct Ending {
    stop: Option<Stop>,
    sealed: bool,
}

#[derive(Debug, Default)]
pub struct Control {
    ending: std::sync::Mutex<Ending>,
    stop_qpc: std::sync::atomic::AtomicU64,
    set_aside: std::sync::Mutex<Vec<(String, bool)>>,
    accepting: AtomicBool,
}

impl Control {
    pub const fn new() -> Self {
        Self {
            ending: std::sync::Mutex::new(Ending {
                stop: None,
                sealed: false,
            }),
            stop_qpc: std::sync::atomic::AtomicU64::new(0),
            set_aside: std::sync::Mutex::new(Vec::new()),
            accepting: AtomicBool::new(false),
        }
    }

    pub fn set_aside(&self, campaign: String, discard: bool) -> bool {
        let mut requests = locked(&self.set_aside);
        if self.accepting.load(Ordering::Acquire) {
            requests.push((campaign, discard));
        }
        self.accepting.load(Ordering::Acquire)
    }

    fn accept_requests(&self) {
        let _requests = locked(&self.set_aside);
        self.accepting.store(true, Ordering::Release);
    }

    fn end_requests(&self) -> Vec<(String, bool)> {
        let mut requests = locked(&self.set_aside);
        self.accepting.store(false, Ordering::Release);
        std::mem::take(&mut *requests)
    }

    pub fn stop(&self, stop: Stop) -> bool {
        let mut ending = locked(&self.ending);
        if let Some(asked) = &ending.stop {
            return *asked == stop;
        }
        if ending.sealed {
            return false;
        }
        if let Some(qpc) = performance_counter() {
            let _ = self
                .stop_qpc
                .compare_exchange(0, qpc, Ordering::AcqRel, Ordering::Acquire);
        }
        ending.stop = Some(stop);
        true
    }

    pub fn stopping(&self) -> bool {
        locked(&self.ending).stop.is_some()
    }

    pub fn seal(&self) -> Option<Stop> {
        let mut ending = locked(&self.ending);
        ending.sealed = true;
        ending.stop.clone()
    }

    pub fn reset(&self) {
        *locked(&self.ending) = Ending::default();
        self.stop_qpc.store(0, Ordering::Release);
        let mut requests = locked(&self.set_aside);
        self.accepting.store(false, Ordering::Release);
        requests.clear();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Undiscarded {
    Unread,
    Another,
}

impl Undiscarded {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unread => "the campaign in progress couldn’t be identified",
            Self::Another => "another campaign was loaded before Discard reached the recorder",
        }
    }
}

fn discard_target<'a>(
    ending: Option<&'a Stop>,
    read_at_stop: Option<&str>,
) -> Option<Result<&'a str, Undiscarded>> {
    let Some(Stop::Discard(asked)) = ending else {
        return None;
    };
    Some(match read_at_stop {
        Some(read) if read == asked => Ok(asked.as_str()),
        Some(_) => Err(Undiscarded::Another),
        None => Err(Undiscarded::Unread),
    })
}

#[derive(Debug)]
pub enum Stage<'a> {
    Preparing,
    TakingOver { session: &'a Path, target: &'a Path },
    Restarting { reason: &'a str },
    Recording {
        session: &'a Path,
        status: Option<&'a Value>,
        names: &'a BTreeMap<String, String>,
    },
    Saving { records: u64 },
    Finished {
        result: Result<&'a CaptureOutcome, &'a str>,
    },
    Unloading(UnloadObservation),
    SetAside {
        waiting: Option<&'a [Waiting]>,
        done: &'a [SetAside],
    },
}

#[derive(Debug, Default)]
pub struct CaptureOutcome {
    pub detach_requested: bool,
    pub process_exited: bool,
    pub replays: Vec<PathBuf>,
    pub discarded: bool,
    pub undiscarded: Option<Undiscarded>,
    pub rerouted: bool,
    pub omitted: Vec<Omission>,
    pub waiting: Vec<String>,
    pub unrenamed: Vec<String>,
    pub failure_report: Option<PathBuf>,
}

pub type AdmitLoad<'a> = dyn Fn(&str, bool) -> Result<(), Refusal> + 'a;

use crate::replay_io::write_json;

pub use crate::replay_io::{Omission, Refusal};

pub(crate) const RECORDER_VERSION: &str = match option_env!("BATTLEPLAN_VERSION") {
    Some(version) if !version.is_empty() => version,
    _ => env!("CARGO_PKG_VERSION"),
};

struct Console {
    quit: AtomicBool,
    capture: Control,
}

impl Console {
    const fn new() -> Self {
        Self {
            quit: AtomicBool::new(false),
            capture: Control::new(),
        }
    }

    fn interrupt(&self) {
        self.quit.store(true, Ordering::Release);
        self.capture.stop(Stop::Save(None));
    }

    fn quitting(&self) -> bool {
        self.quit.load(Ordering::Acquire)
    }

    fn next_capture(&self) -> Option<&Control> {
        self.capture.reset();
        (!self.quitting()).then_some(&self.capture)
    }
}

static CONSOLE: Console = Console::new();
unsafe extern "system" fn console_control(kind: u32) -> i32 {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        CONSOLE.interrupt();
        1
    } else {
        0
    }
}

struct Options {
    output: PathBuf,
    user: Option<PathBuf>,
    once: bool,
    game_score: bool,
    operation_outcomes: bool,
    politics_capture: bool,
    capture_mode: CaptureMode,
    #[cfg(feature = "alloc-fault-injection")]
    alloc_fault: Option<AllocationFault>,
}

#[cfg(feature = "alloc-fault-injection")]
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AllocationFault {
    phase: String,
    ordinal: u64,
    after_hours: u64,
}

#[cfg(feature = "alloc-fault-injection")]
fn allocation_fault(value: &OsStr) -> Result<AllocationFault, String> {
    let (phase, ordinal) = value
        .to_str()
        .and_then(|value| value.split_once(':'))
        .ok_or(
            "--alloc-fault requires <phase>:<positive allocation ordinal>[:<committed-hours>]",
        )?;
    let (ordinal, after_hours) = if let Some((ordinal, after_hours)) = ordinal.split_once(':') {
        let after_hours = after_hours
            .parse::<u64>()
            .map_err(|_| "--alloc-fault requires a nonnegative committed-hours count")?;
        (ordinal, after_hours)
    } else {
        (ordinal, 0)
    };
    let ordinal = ordinal
        .parse::<u64>()
        .ok()
        .filter(|ordinal| *ordinal != 0)
        .ok_or("--alloc-fault requires a positive allocation ordinal")?;
    if phase.is_empty() {
        return Err("--alloc-fault requires a phase name".into());
    }
    Ok(AllocationFault {
        phase: phase.into(),
        ordinal,
        after_hours,
    })
}

#[cfg(feature = "alloc-fault-injection")]
fn fault_config(mut config: Value, fault: Option<&AllocationFault>) -> Value {
    if let Some(fault) = fault {
        let mut requested = json!({"phase":fault.phase,"ordinal":fault.ordinal});
        if fault.after_hours != 0 {
            requested["after_hours"] = json!(fault.after_hours);
        }
        config["alloc_fault"] = requested;
    }
    config
}

fn worker_sampler_config(mut config: Value, requested: Option<String>) -> Value {
    if let Some(requested) = requested {
        config["worker_sampler"] = Value::String(requested);
    }
    config
}

fn usage() -> String {
    #[cfg(feature = "alloc-fault-injection")]
    let fault_option = " [--alloc-fault <phase>:<ordinal>[:<committed-hours>]]";
    #[cfg(not(feature = "alloc-fault-injection"))]
    let fault_option = "";
    format!(
        "BattlePlan command-line recorder

recorder [record] [--out <directory>] [--user-dir <Paradox directory>] [--once] [--capture-mode full|map] [--no-game-score] [--disable-operation-outcomes] [--disable-politics-capture]{fault_option}
recorder detach [--out <directory>]
recorder refresh [--out <directory>]
recorder finalize <session-directory> [--out <directory>]
recorder inspect <campaign.battleplan>
recorder recompress <campaign.battleplan> --level N
recorder commands <recording.battleplan|recording.h4col> [--branch <id>] [--events]
recorder inspect <campaign.battleplan> --characters <substring> --hour <n> --branch <id>
{}
{}
{}
{}
{}
{}
{}
Environment: BATTLEPLAN_WORKER_SAMPLER (comma-separated off, interval_us=<n>)
  Forwarded verbatim to collector attach; a nonempty value requires collector feature worker_sampler.
Environment: BATTLEPLAN_MAP_EVENTS (comma-separated map event family names, or none)
  The map event families the collector installs; every family when unset. The others record unavailable coverage.",
        crate::clock::USAGE,
        crate::metrics::USAGE,
        crate::oracle::USAGE,
        crate::movement_oracle::USAGE,
        crate::ship_name_witness::USAGE,
        crate::sizes::USAGE,
        crate::derived::USAGE
    )
}

fn map_event_config(mut config: serde_json::Value, families: &str) -> serde_json::Value {
    let selected = families
        .split(',')
        .map(str::trim)
        .filter(|family| !family.is_empty() && *family != "none")
        .collect::<Vec<_>>();
    config["map_events"] = json!(selected);
    config
}

fn parse_options(command: &str, args: Vec<OsString>) -> Result<Options, String> {
    let mut options = Options {
        output: default_capture_directory(),
        user: None,
        once: false,
        game_score: true,
        operation_outcomes: true,
        politics_capture: true,
        capture_mode: CaptureMode::Full,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: None,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--out") => options.output = PathBuf::from(args.next().ok_or_else(usage)?),
            Some("--user-dir") if command == "record" => {
                options.user = Some(PathBuf::from(args.next().ok_or_else(usage)?))
            }
            Some("--once") if command == "record" => options.once = true,
            Some("--capture-mode") if command == "record" => {
                let name = args.next().ok_or_else(usage)?;
                options.capture_mode = name
                    .to_str()
                    .and_then(CaptureMode::from_name)
                    .ok_or_else(|| format!("--capture-mode must be full or map, not {name:?}"))?;
            }
            Some("--no-game-score") if command == "record" => options.game_score = false,
            Some("--disable-operation-outcomes") if command == "record" => {
                options.operation_outcomes = false
            }
            Some("--disable-politics-capture") if command == "record" => {
                options.politics_capture = false
            }
            #[cfg(feature = "alloc-fault-injection")]
            Some("--alloc-fault") if command == "record" && options.alloc_fault.is_none() => {
                options.alloc_fault = Some(allocation_fault(&args.next().ok_or_else(usage)?)?);
            }
            #[cfg(not(feature = "alloc-fault-injection"))]
            Some("--alloc-fault") if command == "record" => {
                return Err("--alloc-fault requires recorder feature alloc-fault-injection".into());
            }
            _ => return Err(usage()),
        }
    }
    Ok(options)
}

fn recompress(args: &[OsString]) -> Result<(), String> {
    let (path, level) = match args {
        [path, flag, level] if flag == "--level" => (path, level.to_str()),
        [path, level] => (path, level.to_str().and_then(|level| level.strip_prefix("--level="))),
        _ => return Err(usage()),
    };
    let level: i32 = level.and_then(|level| level.parse().ok()).ok_or_else(usage)?;
    let path = Path::new(path);
    let megabytes = |bytes: u64| bytes as f64 / (1 << 20) as f64;
    let mut shown = None;
    let outcome = replay_files::recompress::recompress_file(path, level, false, |progress| {
        let percent = progress.units * 100 / progress.total.max(1);
        if shown != Some(percent) {
            shown = Some(percent);
            eprintln!(
                "units {} of {}, {:.1} MB read, {:.1} MB written",
                progress.units,
                progress.total,
                megabytes(progress.read),
                megabytes(progress.written)
            );
        }
        true
    });
    match outcome {
        Ok(replay_files::recompress::Outcome::Replaced { before, after, unflushed }) => {
            println!(
                "{}: compressed at level {level}, {:.1} MB to {:.1} MB",
                path.display(),
                megabytes(before),
                megabytes(after)
            );
            match unflushed {
                Some(error) => Err(format!("{}: the smaller file replaced the recording, but was not confirmed on disk: {error}", path.display())),
                None => Ok(()),
            }
        }
        Ok(replay_files::recompress::Outcome::AlreadyAt(level)) => {
            println!("{}: at level {level} already; left as it is", path.display());
            Ok(())
        }
        Err(kept) => Err(format!("{}: kept as it is: {kept}", path.display())),
    }
}

pub fn command(mut args: Vec<OsString>) -> Result<(), String> {
    if args.first().is_some_and(|arg| replay_files::recording_files::is_recording(Path::new(arg))) {
        return crate::metrics::command(args);
    }
    let command = if args
        .first()
        .is_some_and(|arg| !arg.to_string_lossy().starts_with('-'))
    {
        args.remove(0).into_string().map_err(|_| usage())?
    } else {
        "record".into()
    };
    if command == "inspect" && args.len() == 1 {
        return replay_io::inspect(Path::new(&args[0]));
    }
    if command == "commands" {
        return crate::player_commands::command(args);
    }
    if command == "features" {
        println!("features: {}", replay_journal::FEATURES.join(", "));
        return Ok(());
    }
    if command == "inspect" {
        return inspect_characters(&args);
    }
    if command == "oracle" {
        return crate::oracle::command(args).map_err(|error| error.to_string());
    }
    if command == "movement-oracle" {
        return crate::movement_oracle::command(args);
    }
    if command == "ship-name-witness" {
        return crate::ship_name_witness::command(args);
    }
    if command == "metrics" {
        return crate::metrics::command(args);
    }
    if command == "sizes" {
        return crate::sizes::command(args);
    }
    if command == "derived" {
        return crate::derived::command(args);
    }
    if command == "recompress" {
        return recompress(&args);
    }
    if command == "clock" {
        return crate::clock::command(args);
    }
    if command == "--help" || args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return Ok(());
    }
    let session = if command == "finalize" && !args.is_empty() {
        Some(PathBuf::from(args.remove(0)))
    } else {
        None
    };
    let mut options = parse_options(&command, args)?;
    fs::create_dir_all(&options.output).map_err(|e| e.to_string())?;
    options.output = options.output.canonicalize().map_err(|e| e.to_string())?;
    match command.as_str() {
        "record" => {
            // SAFETY: lowers only this recorder process's priority class during live capture.
            unsafe {
                use windows_sys::Win32::System::Threading::{
                    BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, SetPriorityClass,
                };
                SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
            }
            record(options)
        }
        "detach" => request_detach(&options.output),
        "refresh" => request_refresh(&options.output),
        "finalize" => {
            let session = session.ok_or_else(usage)?;
            if session_discarded(&session) {
                return Err(format!(
                    "{} was discarded; there is nothing to finalize",
                    session.display()
                ));
            }
            let (process, expected) = ensure_finalizable(&session)?;
            let dll = session.join("bin/collector.dll");
            let process_exited = CollectorPresence::observe(process.as_ref(), &dll).process_exited;
            let refresh = || CollectorPresence::wait(process.as_ref(), &dll);
            let _lock = finalization_lock(&options.output)?;
            let terminal = terminal_outcome(
                &session,
                expected.id,
                expected.process_start_time,
                false,
                process_exited,
                true,
            );
            finalize_cli_session(&session, &options.output, terminal, refresh)
        }
        _ => Err(usage()),
    }
}

fn finalize_cli_session(
    session: &Path,
    output: &Path,
    mut terminal: TerminalOutcome,
    refresh: impl FnOnce() -> CollectorPresence,
) -> Result<(), String> {
    let _pass_progress = progress_io::passes(session);
    replay_io::with_packing(|| {
        if !terminal.recovery_allowed {
            return Err(terminal.issues.join("; "));
        }
        if let Some(witness) = &terminal.witness {
            publish_failure_status(session, witness)?;
        }
        let finalized = match replay_io::finalize_session_outcome(session, output) {
            Ok(finalized) => finalized,
            Err(error) => {
                terminal
                    .issues
                    .push(format!("replay finalization failed: {error}"));
                replay_io::Finalized::default()
            }
        };
        terminal.issues.extend(
            finalized
                .errors
                .iter()
                .map(|error| format!("replay finalization failed: {error}")),
        );
        complete_cli_finalization(
            session,
            output,
            &finalized,
            json!({
                "detach_requested": terminal.witness.is_some(),
                "drain_complete": terminal.drain_complete,
                "capture_failed": terminal.capture_failed,
                "failure_code": terminal.witness.as_ref().map(|_| "allocation_failed"),
                "failed_hour": terminal.witness.as_ref().and_then(|witness| witness.failed_hour),
                "partial": !terminal.issues.is_empty(), "issues": terminal.issues,
                "warnings": terminal.warnings,
                "omitted": finalized.omitted.iter().map(Omission::to_json).collect::<Vec<_>>(),
            }),
            refresh,
        )?;
        if terminal.issues.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "recording completion is partial: {}; spool retained at {}",
                terminal.issues.join("; "),
                session.display()
            ))
        }
    })
}

fn complete_cli_finalization(
    session: &Path,
    output: &Path,
    finalized: &replay_io::Finalized,
    details: Value,
    refresh: impl FnOnce() -> CollectorPresence,
) -> Result<(), String> {
    let _lock = completion_lock(session)?;
    let presence = refresh();
    let path = waiting_record(session);
    let mut completed = if path.exists() {
        read_status(&path).ok_or_else(|| failure_text(&path, "unreadable; spool retained"))?
    } else {
        json!({})
    };
    if !completed.is_object() {
        return Err(failure_text(
            &path,
            "invalid completion record; spool retained",
        ));
    }
    let mut waiting = match completed.get("waiting") {
        Some(waiting) => waiting
            .as_array()
            .cloned()
            .ok_or_else(|| failure_text(&path, "invalid waiting list; spool retained"))?,
        None => Vec::new(),
    };
    waiting.retain(|entry| {
        !entry["campaign"].as_str().is_some_and(|campaign| {
            finalized.saved_campaigns.contains(campaign)
                || finalized.failed_campaigns.contains(campaign)
        })
    });
    for campaign in &finalized.failed_campaigns {
        waiting.extend(waits(output, campaign));
    }
    let mut replays = completed["replays"].as_array().cloned().unwrap_or_default();
    for replay in &finalized.replays {
        let replay = json!(replay);
        if !replays.contains(&replay) {
            replays.push(replay);
        }
    }
    let left_out = completed
        .get("omitted")
        .filter(|_| finalized.omitted.is_empty())
        .cloned();
    for (key, value) in details.as_object().unwrap() {
        completed[key] = value.clone();
    }
    if let Some(left_out) = left_out {
        completed["omitted"] = left_out;
    }
    completed["replays"] = json!(replays);
    completed["target"] = json!(output);
    completed["waiting"] = json!(waiting);
    publish_completion(session, output, completed, presence)?;
    if session.join(WAITING_FILE).exists() {
        fs::remove_file(session.join(WAITING_FILE))
            .map_err(|error| failure_text(&session.join(WAITING_FILE), error))?;
    }
    Ok(())
}

fn inspect_characters(args: &[OsString]) -> Result<(), String> {
    let (path, options) = args.split_first().ok_or_else(usage)?;
    let (mut needle, mut hour, mut branch) = (None, None, None);
    for pair in options.chunks(2) {
        let [option, value] = pair else {
            return Err(usage());
        };
        let value = value.to_str().ok_or_else(usage)?;
        let slot = match option.to_str() {
            Some("--characters") => &mut needle,
            Some("--hour") => &mut hour,
            Some("--branch") => &mut branch,
            _ => return Err(usage()),
        };
        if slot.replace(value).is_some() {
            return Err(usage());
        }
    }
    let (Some(needle), Some(hour), Some(branch)) = (needle, hour, branch) else {
        return Err(usage());
    };
    replay_io::inspect_characters(
        Path::new(path),
        needle,
        hour.parse()
            .map_err(|_| "--hour requires an engine i32 hour".to_string())?,
        branch
            .parse()
            .map_err(|_| "--branch requires a u64 branch id".to_string())?,
    )
}

pub(crate) const COLLECTOR_LOADED: &str =
    "collector.dll is already loaded in this process; detach its recorder first";
pub(crate) const GAME_CLAIMED: &str =
    "another recorder is attaching to or recording this game; nothing recorded";

fn game_claim(pid: u32, start_time: u64) -> String {
    format!("Local\\BattlePlan.Game.{pid}.{start_time}")
}
const COLLECTOR_UNLOAD_WARNING: &str =
    "collector unload is unconfirmed; quit the game before recording again";

pub(crate) fn discover_game() -> Result<Discovery, String> {
    Ok(choose_game(&windows::game_processes()?))
}

fn watch_chosen(
    pid: u32,
    exited: bool,
    found: Option<&[GameProcess]>,
    others: &mut OtherGames,
) -> Result<(), String> {
    if exited {
        return Err("game exited while content identity was being gathered".into());
    }
    if let Some(notice) = found.and_then(|found| others.notice(pid, found, windows::local_clock)) {
        eprintln!("{notice}");
    }
    Ok(())
}

const ORPHAN_ELSEWHERE: &str = "collector.dll is loaded from a session in another capture folder; record into that folder to take it over";
const ORPHAN_PINNED: &str = "the collector left by a recorder that stopped did not unload after its detach; quit the game before recording again";

struct Build<'a> {
    recorder_version: &'a str,
    collector_hash: [u8; 32],
}

struct Orphan {
    directory: PathBuf,
    target: PathBuf,
    config: Value,
    session: SessionStart,
}

impl Orphan {
    fn cancel_file(&self) -> PathBuf {
        self.config["cancel_file"].as_str().map_or_else(
            || self.directory.join("bin/collector.cancel"),
            PathBuf::from,
        )
    }
}

struct Events {
    ready: Event,
    detach: Event,
    done: Event,
    idle: Event,
}

impl Events {
    fn create(names: [&str; 4]) -> Result<Self, String> {
        let [ready, detach, done, idle] = names;
        Ok(Self {
            ready: Event::new(ready)?,
            detach: Event::new(detach)?,
            done: Event::new(done)?,
            idle: Event::new(idle)?,
        })
    }

    fn open(config: &Value) -> Result<Self, String> {
        let open = |key: &str| {
            config[key]
                .as_str()
                .ok_or_else(|| format!("the configuration has no {key}"))
                .and_then(Event::open)
        };
        Ok(Self {
            ready: open("ready_event")?,
            detach: open("detach_event")?,
            done: open("done_event")?,
            idle: open("idle_event")?,
        })
    }
}

enum Inherited {
    Nothing,
    TakeOver(Orphan, Events),
    Restart(Orphan, String),
}

fn claim_game(
    output: &Path,
    pid: u32,
    start_time: u64,
    loaded: impl FnOnce() -> Result<Vec<PathBuf>, String>,
    build: &Build<'_>,
    recorder_alive: impl Fn(u32, u64) -> bool,
) -> Result<(windows::Claim, Inherited), String> {
    let claim = windows::Claim::take(&game_claim(pid, start_time))?.ok_or(GAME_CLAIMED)?;
    let inherited = match loaded()?.as_slice() {
        [] => Inherited::Nothing,
        [dll] => orphan(output, pid, start_time, dll, build, recorder_alive)?,
        _ => return Err(COLLECTOR_LOADED.into()),
    };
    Ok((claim, inherited))
}

fn orphan(
    output: &Path,
    pid: u32,
    start_time: u64,
    dll: &Path,
    build: &Build<'_>,
    recorder_alive: impl Fn(u32, u64) -> bool,
) -> Result<Inherited, String> {
    let sessions = output.join("sessions");
    let name = dll
        .parent()
        .filter(|bin| {
            bin.file_name()
                .is_some_and(|name| name.eq_ignore_ascii_case("bin"))
        })
        .and_then(Path::parent)
        .filter(|session| {
            let canonical = |path: &Path| path.canonicalize().ok();
            session
                .parent()
                .and_then(canonical)
                .is_some_and(|parent| Some(parent) == canonical(&sessions))
        })
        .and_then(Path::file_name)
        .ok_or_else(|| format!("{ORPHAN_ELSEWHERE}: {}", dll.display()))?;
    let directory = sessions.join(name);
    let refused = |why: &str| format!("{COLLECTOR_LOADED}; session {}: {why}", directory.display());
    let (config, session) = collector_config(&directory).map_err(|error| {
        refused(&format!(
            "its configuration cannot be read ({error}); quit the game before recording again"
        ))
    })?;
    if session.process_id != pid || session.process_start_time != start_time {
        return Err(refused("it names another game process"));
    }
    if session_ended(&directory) {
        return Err(COLLECTOR_LOADED.into());
    }
    if let Some(active) = read_status(&output.join("active.json"))
        && names_session(&active, &directory)
        && let Some(recorder) = active["recorder_pid"]
            .as_u64()
            .and_then(|pid| u32::try_from(pid).ok())
        && let Some(started) = active["recorder_start_time"].as_u64()
        && recorder_alive(recorder, started)
    {
        return Err(GAME_CLAIMED.into());
    }
    let target = PathBuf::from(
        config["replay_dir"]
            .as_str()
            .ok_or_else(|| refused("its configuration names no recording folder"))?,
    );
    let orphan = Orphan {
        directory,
        target,
        config,
        session,
    };
    let identity = &orphan.session.identity;
    if identity.recorder_version != build.recorder_version
        || identity.collector_hash != build.collector_hash
    {
        let reason = format!(
            "it was started by recorder {} with collector {}, and this is recorder {} with collector {}",
            identity.recorder_version,
            hex(&identity.collector_hash),
            build.recorder_version,
            hex(&build.collector_hash)
        );
        return Ok(Inherited::Restart(orphan, reason));
    }
    let reason = match live_failure(&orphan.directory, orphan.session.id, start_time) {
        Ok(Some(witness)) => Some(format!(
            "its collector's capture failed at hour {:?}",
            witness.failed_hour
        )),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    let reason = reason.or_else(|| {
        let error = orphan.directory.join("bin/collector.error.txt");
        error.exists().then(|| {
            format!(
                "its collector stopped with an error: {}",
                fs::read_to_string(&error)
                    .unwrap_or_else(|e| e.to_string())
                    .trim()
            )
        })
    });
    if let Some(reason) = reason {
        return Ok(Inherited::Restart(orphan, reason));
    }
    let events = match Events::open(&orphan.config) {
        Ok(events) => events,
        Err(error) => {
            let reason = format!("its collector's events cannot be opened: {error}");
            return Ok(Inherited::Restart(orphan, reason));
        }
    };
    if events.detach.handle.signaled()?
        || events.done.handle.signaled()?
        || orphan.cancel_file().exists()
    {
        let reason = "its recorder had already asked its collector to detach".to_owned();
        return Ok(Inherited::Restart(orphan, reason));
    }
    Ok(Inherited::TakeOver(orphan, events))
}

fn recorder_alive(pid: u32, start_time: u64) -> bool {
    Process::open(pid, false).is_ok_and(|process| {
        process.start_time == start_time && !process.handle.signaled().unwrap_or(false)
    })
}

fn loaded_collectors(pid: u32) -> Result<Vec<PathBuf>, String> {
    Ok(windows::modules(pid)?
        .into_iter()
        .filter(|module| {
            module
                .path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("collector.dll"))
        })
        .map(|module| module.path)
        .collect())
}

fn names_session(active: &Value, directory: &Path) -> bool {
    active["session_dir"]
        .as_str()
        .is_some_and(|held| Path::new(held) == directory)
}

fn write_active(
    output: &Path,
    recorder: (u32, u64),
    session: &SessionStart,
    (directory, dll): (&Path, &Path),
    detach_event: &str,
    done_event: &str,
) -> Result<(), String> {
    write_json(
        &output.join("active.json"),
        &json!({
            "recorder_pid":recorder.0,"recorder_start_time":recorder.1,
            "game_pid":session.process_id,"session_id":session.id,
            "process_start_time":session.process_start_time,"session_dir":directory,
            "dll":dll,"detach_event":detach_event,"done_event":done_event,
        }),
    )
}

fn take_over(output: &Path, orphan: &Orphan, recorder: (u32, u64)) -> Result<(), String> {
    let held = read_status(&output.join("active.json"))
        .filter(|active| names_session(active, &orphan.directory));
    let held_path = |key: &str| {
        held.as_ref()
            .and_then(|active| active[key].as_str())
            .map(PathBuf::from)
    };
    let directory = held_path("session_dir").unwrap_or_else(|| orphan.directory.clone());
    let dll = held_path("dll").unwrap_or_else(|| directory.join("bin").join("collector.dll"));
    let stopped = held
        .as_ref()
        .and_then(|active| active["recorder_pid"].as_u64());
    let status = read_status(&orphan.directory.join("status.json"));
    let status = status.as_ref();
    let after_hour = status
        .filter(|status| status["hours"].as_u64().is_some_and(|hours| hours > 0))
        .and_then(|status| status["last_hour"].as_i64());
    let path = orphan.directory.join(replay_io::RESTARTS_FILE);
    let mut restarts = read_status(&path)
        .and_then(|restarts| restarts.as_array().cloned())
        .unwrap_or_default();
    restarts.push(json!({
        "recorder_pid":recorder.0,"stopped_pid":stopped,"after_hour":after_hour,
        "campaign":status.and_then(|status| status["campaign"].as_str()),
    }));
    write_json(&path, &Value::Array(restarts))?;
    let event = |key: &str| {
        orphan.config[key]
            .as_str()
            .ok_or_else(|| format!("the session's configuration has no {key}"))
    };
    write_active(
        output,
        recorder,
        &orphan.session,
        (&directory, &dll),
        event("detach_event")?,
        event("done_event")?,
    )
}

fn end_orphan(
    output: &Path,
    orphan: &Orphan,
    reason: &str,
    unloaded: impl FnOnce() -> Result<CollectorPresence, String>,
) -> Result<(), String> {
    let cancel = orphan.cancel_file();
    let cancelled =
        fs::write(&cancel, b"cancelled\n").map_err(|error| failure_text(&cancel, error));
    let signalled = orphan.config["detach_event"]
        .as_str()
        .ok_or_else(|| "the session's configuration has no detach event".to_owned())
        .and_then(|name| Event::open(name)?.set());
    if let (Err(cancelled), Err(signalled)) = (&cancelled, &signalled) {
        return Err(format!(
            "the collector left by a recorder that stopped could not be asked to detach: {cancelled}; {signalled}"
        ));
    }
    let presence = unloaded()?;
    if presence.collector_loaded != Some(false) {
        return Err(ORPHAN_PINNED.into());
    }
    match end_session(
        output,
        &orphan.directory,
        &orphan.config,
        &orphan.session,
        presence,
        Some(reason),
    )? {
        Abandoned::Settled => Ok(()),
        Abandoned::Busy => Err(format!(
            "the session left by a recorder that stopped waits for a save in {}; record again once it ends",
            orphan.target.display()
        )),
        Abandoned::Refused(why) => Err(failure_text(&orphan.directory, why)),
    }
}

fn orphan_unloaded(process: &Process, orphan: &Orphan) -> Result<CollectorPresence, String> {
    let dll = orphan.directory.join("bin").join("collector.dll");
    let done = orphan.config["done_event"]
        .as_str()
        .and_then(|name| Event::open(name).ok());
    let mut wait = CollectorWait::new(Instant::now(), DONE_STALL);
    loop {
        let finished = done
            .as_ref()
            .is_some_and(|done| done.handle.signaled().unwrap_or(false));
        let gone =
            collector_present(process, &dll, Instant::now() + UNLOAD_POLL).ok() == Some(false);
        if finished || gone || process.handle.signaled()? {
            break;
        }
        if let Some(end) = wait.ended(Instant::now(), collector_stamp(&orphan.directory)) {
            return Err(format!(
                "the collector left by a recorder that stopped did not detach {}; its spool is retained",
                end.describe(&orphan.directory)
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(CollectorPresence::wait(Some(process), &dll))
}

fn record(options: Options) -> Result<(), String> {
    unsafe {
        SetConsoleCtrlHandler(Some(console_control), 1);
    }
    let user = options
        .user
        .clone()
        .map(Ok)
        .unwrap_or_else(resolve_user_directory)?;
    let user = user.canonicalize().map_err(|e| {
        format!(
            "Paradox user folder {}: {e}; pass --user-dir",
            user.display()
        )
    })?;
    println!("Paradox user folder: {}", user.display());
    println!("Waiting for hoi4.exe. Recording is passive; Ctrl+C detaches and finalizes.");
    let mut several_named = None;
    loop {
        if CONSOLE.quitting() {
            return Ok(());
        }
        let game = match discover_game()? {
            Discovery::One(game) => game,
            Discovery::None => {
                several_named = None;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            Discovery::Several(several) => {
                if several_named.as_ref() != Some(&several) {
                    eprintln!("{several}");
                    several_named = Some(several);
                }
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
        };
        several_named = None;
        let Some(process) = Process::open_selected(&game, true)? else {
            continue;
        };
        let Some(control) = CONSOLE.next_capture() else {
            return Ok(());
        };
        let outcome = capture(process, &cli_capture(&options, &user), control, &|stage| {
            if let Stage::Unloading(observation) = stage {
                if let Err(error) = observation.finish() {
                    eprintln!("Collector warning: {error}");
                }
            }
        })?;
        if outcome.detach_requested || options.once || CONSOLE.quitting() {
            return Ok(());
        }
        println!("Waiting for the next game process.");
    }
}

pub(crate) struct CaptureOptions<'a> {
    pub output: &'a Path,
    pub target: &'a Path,
    pub user: &'a Path,
    pub game_score: bool,
    pub operation_outcomes: bool,
    pub politics_capture: bool,
    pub capture_mode: CaptureMode,
    pub nothing_recorded_is_ok: bool,
    pub live_status: bool,
    pub admit: Option<&'a AdmitLoad<'a>>,
    pub loaded: Option<&'a dyn Fn() -> Option<(String, i64)>>,
    pub opening: Option<(&'a str, i64)>,
    pub auto_save: Option<&'a dyn Fn() -> bool>,
    pub names_wanted: Option<&'a dyn Fn() -> bool>,
    #[cfg(feature = "alloc-fault-injection")]
    pub alloc_fault: Option<&'a AllocationFault>,
}

struct ContentScan {
    cancelled: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<std::io::Result<GatheredContent>>>,
}

impl ContentScan {
    fn start(options: GatherOptions) -> Self {
        let cancelled = Arc::new(AtomicBool::new(false));
        let stopping = Arc::clone(&cancelled);
        Self {
            cancelled,
            thread: Some(std::thread::spawn(move || {
                gather_cancellable(&options, stopping)
            })),
        }
    }

    fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished)
    }

    fn take(mut self) -> Result<std::io::Result<GatheredContent>, String> {
        let thread = self.thread.take().ok_or("content was taken twice")?;
        thread
            .join()
            .map_err(|_| "content hashing worker panicked".to_owned())
    }
}

impl Drop for ContentScan {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.cancelled.store(true, Ordering::Release);
            let _ = thread.join();
        }
    }
}

struct DetachOnDrop {
    event: Event,
    cancel: PathBuf,
}

impl DetachOnDrop {
    fn arm(config: &Value, cancel: PathBuf) -> Result<Self, String> {
        let name = config["detach_event"]
            .as_str()
            .ok_or("the session's configuration has no detach_event")?;
        Ok(Self {
            event: Event::open(name)?,
            cancel,
        })
    }
}

impl Drop for DetachOnDrop {
    fn drop(&mut self) {
        let _ = fs::write(&self.cancel, b"cancelled\n");
        let _ = self.event.set();
    }
}

fn await_scan(
    finished: impl Fn() -> bool,
    inherited: Option<&DetachOnDrop>,
    stopping: impl Fn() -> bool,
    mut watch: impl FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let mut detached = false;
    loop {
        if !detached && stopping() {
            let Some(inherited) = inherited else {
                return Ok(false);
            };
            inherited.event.set()?;
            detached = true;
        }
        if finished() {
            return Ok(true);
        }
        watch()?;
    }
}

fn cli_capture<'a>(options: &'a Options, user: &'a Path) -> CaptureOptions<'a> {
    CaptureOptions {
        output: &options.output,
        target: &options.output,
        user,
        game_score: options.game_score,
        operation_outcomes: options.operation_outcomes,
        politics_capture: options.politics_capture,
        capture_mode: options.capture_mode,
        nothing_recorded_is_ok: false,
        live_status: false,
        admit: None,
        loaded: None,
        opening: None,
        auto_save: None,
        names_wanted: None,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault: options.alloc_fault.as_ref(),
    }
}

struct Admission<'a> {
    session: &'a Path,
    target: &'a Path,
    seen: BTreeSet<(String, i64)>,
    admitted: replay_io::Admissions,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    Seen,
    Admitted,
    Refused,
}

impl<'a> Admission<'a> {
    fn new(session: &'a Path, target: &'a Path) -> Self {
        Self {
            session,
            target,
            seen: BTreeSet::new(),
            admitted: replay_io::Admissions::default(),
        }
    }

    fn resume(session: &'a Path, target: &'a Path) -> Result<Self, String> {
        let mut admission = Self::new(session, target);
        if let Some(admitted) = replay_io::session_admissions(session)? {
            admission.seen = admitted
                .loads
                .iter()
                .flat_map(|(campaign, loads)| loads.iter().map(|load| (campaign.clone(), *load)))
                .chain(admitted.refused.keys().cloned())
                .collect();
            admission.admitted = admitted;
        }
        Ok(admission)
    }

    fn admit_opening(&mut self, campaign: &str, load: i64) -> Result<(), String> {
        self.seen.insert((campaign.to_owned(), load));
        self.admitted
            .admit(self.session, self.target, campaign, load)
    }

    fn check(
        &mut self,
        campaign: &str,
        load: i64,
        recorded: impl FnOnce() -> bool,
        admit: &dyn Fn(&str, bool) -> Result<(), Refusal>,
    ) -> Result<Decision, String> {
        if !self.seen.insert((campaign.to_owned(), load)) {
            return Ok(Decision::Seen);
        }
        match admit(campaign, recorded()) {
            Ok(()) => {
                self.admitted
                    .admit(self.session, self.target, campaign, load)?;
                Ok(Decision::Admitted)
            }
            Err(refusal) => {
                if let Err(error) =
                    self.admitted
                        .refuse(self.session, self.target, campaign, load, refusal)
                {
                    eprintln!(
                        "Load {load} of campaign {campaign}: its refusal could not be recorded: {error}"
                    );
                }
                Ok(Decision::Refused)
            }
        }
    }

    fn check_journaled(
        &mut self,
        live: &mut replay_io::Live,
        admit: &dyn Fn(&str, bool) -> Result<(), Refusal>,
    ) -> Result<(), String> {
        let settings = RecorderSettings::path();
        let naming = replay_io::Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        loop {
            live.advance(self.session, self.target, &|_| false, naming, &|| false)?;
            let Some(next) = live
                .undecided()
                .into_iter()
                .find(|load| !self.seen.contains(&(load.campaign.clone(), load.load)))
            else {
                return Ok(());
            };
            let session = self.session;
            self.check(
                &next.campaign,
                next.load,
                || replay_io::recorded_before_load(session, &next.campaign, next.load),
                admit,
            )?;
        }
    }
}

struct StatusReads {
    live: bool,
    next_print: Instant,
    next_live: Instant,
}

impl StatusReads {
    const PRINT: Duration = Duration::from_secs(60);
    const LIVE: Duration = Duration::from_secs(2);

    fn new(live: bool, now: Instant) -> Self {
        Self {
            live,
            next_print: now,
            next_live: now,
        }
    }

    fn print_due(&mut self, now: Instant) -> bool {
        let due = now >= self.next_print;
        if due {
            self.next_print = now + Self::PRINT;
        }
        due
    }

    fn live_due(&self, now: Instant) -> bool {
        self.live && now >= self.next_live
    }

    fn live_read(&mut self, now: Instant) {
        self.next_live = now + Self::LIVE;
    }
}

pub(crate) fn capture(
    process: Process,
    options: &CaptureOptions<'_>,
    control: &Control,
    stage: &dyn Fn(Stage<'_>),
) -> Result<CaptureOutcome, String> {
    let CaptureOptions {
        output,
        target,
        user,
        game_score,
        operation_outcomes,
        politics_capture,
        capture_mode,
        nothing_recorded_is_ok,
        live_status,
        admit,
        loaded,
        opening,
        auto_save,
        names_wanted,
        #[cfg(feature = "alloc-fault-injection")]
        alloc_fault,
    } = *options;
    stage(Stage::Preparing);
    fs::create_dir_all(target).map_err(|error| failure_text(target, error))?;
    let pid = process.pid;
    let tables = engine_executable::executable_tables::initialize_executable(&process.path)
        .map_err(|error| format!("Cannot prepare game tables: {error}"))?;
    let exe = tables.identity().clone();
    for warning in tables.warnings() {
        println!("Warning: {warning}");
    }
    let game_version = match &tables.version {
        Ok(version) => version.clone(),
        Err(_) => format!("unknown version, SHA-256 {}", hex(&exe.sha256)),
    };
    let collector_source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("collector.dll");
    let build = Build {
        recorder_version: RECORDER_VERSION,
        collector_hash: hash_executable(
            File::open(&collector_source).map_err(|e| failure_text(&collector_source, e))?,
        )
        .map_err(|e| e.to_string())?
        .sha256,
    };
    let recorder = (
        std::process::id(),
        Process::open(std::process::id(), false)?.start_time,
    );
    let (_attachment, inherited) = claim_game(
        output,
        pid,
        process.start_time,
        || loaded_collectors(pid),
        &build,
        recorder_alive,
    )?;
    let mut armed = None;
    let taken = match inherited {
        Inherited::Nothing => None,
        Inherited::TakeOver(orphan, events) => {
            println!(
                "Taking over the recording left by a recorder that stopped: {}",
                orphan.directory.display()
            );
            stage(Stage::TakingOver {
                session: &orphan.directory,
                target: &orphan.target,
            });
            take_over(output, &orphan, recorder)?;
            armed = Some(DetachOnDrop::arm(&orphan.config, orphan.cancel_file())?);
            Some((orphan, events))
        }
        Inherited::Restart(orphan, reason) => {
            println!(
                "Ending the recording left by a recorder that stopped, then recording afresh: {reason}."
            );
            stage(Stage::Restarting { reason: &reason });
            end_orphan(output, &orphan, &reason, || {
                orphan_unloaded(&process, &orphan)
            })?;
            require_collector_absent(Some(!loaded_collectors(pid)?.is_empty()))?;
            None
        }
    };
    let taking_over = taken.is_some();
    let taken_target = taken.as_ref().map(|(orphan, _)| orphan.target.clone());
    let target: &Path = taken_target.as_deref().unwrap_or(target);
    fs::create_dir_all(target).map_err(|error| failure_text(target, error))?;
    let game_dir = process
        .path
        .parent()
        .ok_or("game executable has no folder")?
        .to_owned();
    let facts_cache = replay_settings::recorder_settings::directory();
    let gather_options = GatherOptions {
        game_dir,
        user_dir: user.to_owned(),
        cache_dir: facts_cache.clone(),
        dlc_order: None,
    };
    println!("Verifying game, DLC and enabled mod hashes in the background...");
    let scan = ContentScan::start(gather_options);
    let mut next_discovery = Instant::now();
    let mut others = OtherGames::default();
    let watch = || {
        let exited = process.handle.signaled()?;
        let found = if !exited && Instant::now() >= next_discovery {
            next_discovery = Instant::now() + Duration::from_secs(2);
            windows::game_processes().ok()
        } else {
            None
        };
        watch_chosen(pid, exited, found.as_deref(), &mut others)?;
        std::thread::sleep(Duration::from_millis(100));
        Ok(())
    };
    if !await_scan(|| scan.is_finished(), armed.as_ref(), || control.stopping(), watch)? {
        return Ok(CaptureOutcome {
            detach_requested: true,
            ..CaptureOutcome::default()
        });
    }
    let content = scan.take()?.map_err(|e| e.to_string())?;
    println!(
        "Content: {} files, {:.1} MB covered, {} cache hits, {:.3} s.",
        content.stats.files,
        content.stats.bytes as f64 / 1e6,
        content.stats.cache_hits,
        content.stats.elapsed_ms as f64 / 1000.0
    );
    if let Some(unsaved) = &content.stats.cache_unsaved {
        println!("Content hash cache was not saved: {unsaved}");
    }
    if let Some(note) = &content.order_note {
        println!("Content order is unverified: {note}");
    }
    let (directory, session, events, config, game_facts) = match taken {
        Some((orphan, events)) => {
            let game_facts = game_facts::GameFacts::start(
                &content,
                &orphan.session.identity,
                &orphan.directory,
                &facts_cache,
            );
            (
                orphan.directory,
                orphan.session,
                events,
                orphan.config,
                game_facts,
            )
        }
        None => {
            let base = crate::game_sites::module_base(&process, pid)?;
            let readable = crate::game_sites::locate(&process.path, base)?
                .layouts()
                .map_err(|withheld| withheld.to_string())
                .and_then(|()| {
                    engine_native::cgame_state::clock_sites(base)
                        .and_then(|()| engine_native::command_inputs::idler_sites(base))
                        .map_err(str::to_owned)
                });
            if let Err(withheld) = readable {
                println!(
                    "The loaded campaign can’t be read on this build ({withheld}); connecting now."
                );
            } else if !wait_for_loaded_campaign(&process, pid, control)? {
                return Ok(CaptureOutcome {
                    detach_requested: true,
                    ..CaptureOutcome::default()
                });
            }
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?
                .as_nanos();
            let directory = output
                .join("sessions")
                .join(format!("{}-{nonce}", process.start_time));
            let bin = directory.join("bin");
            fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
            let dll = bin.join("collector.dll");
            fs::copy(&collector_source, &dll)
                .map_err(|e| format!("copy {}: {e}", collector_source.display()))?;
            let dll_hash = hash_executable(File::open(&dll).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?
                .sha256;
            let identity = content
                .identity(
                    game_version,
                    exe.sha256,
                    exe.size,
                    RECORDER_VERSION.into(),
                    dll_hash,
                )
                .map_err(|e| e.to_string())?;
            let game_facts =
                game_facts::GameFacts::start(&content, &identity, &directory, &facts_cache);
            let mut frequency = 0;
            if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
                return Err("QueryPerformanceFrequency failed".into());
            }
            let session = SessionStart {
                id: process.start_time,
                process_id: pid,
                process_start_time: process.start_time,
                qpc_frequency: frequency as u64,
                sample_boundary: replay_data::identity::SampleBoundary::AfterHourlyUpdate,
                capture_mode,
                identity,
            };
            let mut session_bytes = Vec::new();
            session
                .encode(&mut session_bytes)
                .map_err(|e| e.to_string())?;
            let prefix = format!("Local\\BattlePlan.{}.{}", process.start_time, nonce);
            let names =
                ["ready", "detach", "done", "idle"].map(|event| format!("{prefix}.{event}"));
            let events = Events::create(names.each_ref().map(String::as_str))?;
            events.idle.set()?;
            let [ready_name, detach_name, done_name, idle_name] = &names;
            let cancel_file = bin.join("collector.cancel");
            let expected_mounts = &content.expected_mounts;
            let scripted_maps = scripted_map_demand::read(&content.layers, &content.language)
                .map_err(|error| format!("scripted map capture demand: {error}"))?;
            create_failure_witness(&directory, session.id, session.process_start_time)?;
            for error in end_abandoned_sessions(output).errors {
                eprintln!("An earlier session could not be ended: {error}");
            }
            for ended in session_folders(output) {
                replay_io::reconcile_session(&ended, target)?;
            }
            let config = json!({"session_id":session.id,"output_dir":directory,"replay_dir":target,"session_start":session_bytes,"ready_event":ready_name,"detach_event":detach_name,"done_event":done_name,"idle_event":idle_name,"cancel_file":cancel_file,"content_order_note":content.order_note,"expected_mounts":expected_mounts,"mod_descriptors":content.mod_descriptors,"game_score":game_score,"operation_outcomes":operation_outcomes,"politics_capture":politics_capture,"scripted_maps":scripted_maps,"without_history":Vec::<String>::new()});
            let sampler = match std::env::var("BATTLEPLAN_WORKER_SAMPLER").or_else(|_| std::env::var("HOI4_WORKER_SAMPLER")) {
                Ok(value) => Some(value),
                Err(std::env::VarError::NotPresent) => None,
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err("BATTLEPLAN_WORKER_SAMPLER is not valid Unicode".into());
                }
            };
            let config = worker_sampler_config(config, sampler);
            let config = match std::env::var("BATTLEPLAN_MAP_EVENTS").or_else(|_| std::env::var("HOI4_MAP_EVENTS")) {
                Ok(families) => map_event_config(config, &families),
                Err(std::env::VarError::NotPresent) => config,
                Err(std::env::VarError::NotUnicode(_)) => {
                    return Err("BATTLEPLAN_MAP_EVENTS is not valid Unicode".into());
                }
            };
            #[cfg(feature = "alloc-fault-injection")]
            let config = fault_config(config, alloc_fault);
            write_json(&bin.join("collector.json"), &config)?;
            (directory, session, events, config, game_facts)
        }
    };
    let _pass_progress = progress_io::passes(&directory);
    let bin = directory.join("bin");
    let dll = bin.join("collector.dll");
    let named = |key: &str| {
        config[key]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("the session's configuration has no {key}"))
    };
    let (detach_name, done_name) = (named("detach_event")?, named("done_event")?);
    let cancel_file = PathBuf::from(named("cancel_file")?);
    let Events {
        ready,
        detach,
        done,
        idle,
    } = events;
    let idle = Arc::new(idle);
    let mut admission = Admission::resume(&directory, target)?;
    let admit_staged = admit;
    if admit.is_some() {
        if replay_io::session_admissions(&directory)?.is_none() {
            replay_io::begin_admissions(&directory, target)?;
        }
        if let Some((campaign, load)) = opening {
            admission.admit_opening(campaign, load)?;
        }
    }
    let _detach_on_error = match armed {
        Some(armed) => armed,
        None => DetachOnDrop::arm(&config, cancel_file)?,
    };
    if !taking_over {
        process.inject(&dll, || control.stopping())?;
    }
    let error_path = bin.join("collector.error.txt");
    let mut wait = CollectorWait::new(Instant::now(), READY_STALL);
    let mut next_progress = Instant::now() + Duration::from_secs(10);
    while !ready.handle.signaled()? {
        if let Some(witness) = live_failure(&directory, session.id, session.process_start_time)? {
            publish_failure_status(&directory, &witness)?;
            detach.set()?;
            break;
        }
        if error_path.exists() {
            let error = fs::read_to_string(&error_path).unwrap_or_else(|e| e.to_string());
            return Err(collector_error(&directory, &error));
        }
        if process.handle.signaled()? {
            return Err("game exited before collector became ready".into());
        }
        if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
            return Err(format!(
                "collector readiness timed out {}; detach requested; spool retained",
                end.describe(&directory)
            ));
        }
        if control.stopping() {
            detach.set()?;
        }
        if Instant::now() >= next_progress {
            println!("Preparing collector and prior campaign history before hourly sampling...");
            next_progress = Instant::now() + Duration::from_secs(10);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if let Some(summary) = step_summary(&directory, "attach_started", "ready") {
        println!("Collector attach {summary}");
    }
    let active = output.join("active.json");
    if !taking_over {
        write_active(
            output,
            recorder,
            &session,
            (&directory, &dll),
            &detach_name,
            &done_name,
        )?;
    }
    println!("Recording process {pid}. Spool: {}", directory.display());
    let settling = auto_save.map(|auto_save| {
        let settling = Arc::new(Settling {
            collector: true,
            ..Settling::default()
        });
        settling.save.store(auto_save(), Ordering::Release);
        settling
    });
    let ahead = FinalizingAhead::start(
        &directory,
        target,
        &content,
        settling.clone(),
        Some(Arc::clone(&idle)),
    );
    let status_path = directory.join("status.json");
    stage(Stage::Recording {
        session: &directory,
        status: None,
        names: &BTreeMap::new(),
    });
    let mut status_reads = StatusReads::new(live_status, Instant::now());
    let mut admission_failures = Vec::new();
    let mut rerouted = false;
    let mut requested_detach = false;
    let mut stop_qpc = None;
    let mut stop_observed = false;
    let mut stop_load = None;
    let mut next_discovery = Instant::now();
    control.accept_requests();
    let mut admitted: Option<(String, i64)> =
        opening.map(|(campaign, load)| (campaign.to_owned(), load));
    let mut read_at_stop: Option<Option<String>> = None;
    let mut closing = Vec::new();
    let capture_ended = Arc::new(AtomicBool::new(false));
    loop {
        requested_detach |= control.stopping() || detach.handle.signaled()?;
        if requested_detach && !stop_observed {
            stop_observed = true;
            stop_qpc = requested_stop_counter(control, &directory);
            stop_load = system_times();
            if control.stopping() && admit_staged.is_some() {
                let now = loaded.and_then(|loaded| loaded());
                read_at_stop = Some(now.as_ref().map(|(campaign, _)| campaign.clone()));
                if let Some((campaign, load)) = now {
                    if let Some((left, _)) = admitted
                        .replace((campaign.clone(), load))
                        .filter(|(left, _)| *left != campaign)
                        && let Some(settling) = &settling
                        && admission.admitted.loads.contains_key(&left)
                    {
                        locked(&settling.left).insert(left);
                    }
                    if let Some(settling) = &settling {
                        *locked(&settling.loaded) = Some(campaign);
                    }
                }
            }
            ahead.promote();
        }
        if process.handle.signaled()? {
            break;
        }
        if let Some(witness) = live_failure(&directory, session.id, session.process_start_time)? {
            publish_failure_status(&directory, &witness)?;
            eprintln!(
                "Collector capture failed at hour {:?} (phase {}, code {}); detaching and recovering the committed prefix.",
                witness.failed_hour, witness.phase, witness.code
            );
            detach.set()?;
            requested_detach = true;
            break;
        }
        if requested_detach {
            detach.set()?;
            break;
        }
        if done.handle.signaled()? {
            requested_detach = true;
            break;
        }
        if Instant::now() >= next_discovery {
            if let Some(notice) = windows::game_processes()
                .ok()
                .and_then(|found| others.notice(pid, &found, windows::local_clock))
            {
                eprintln!("{notice}");
            }
            next_discovery = Instant::now() + Duration::from_secs(2);
        }
        if status_reads.print_due(Instant::now()) {
            print_status(&status_path);
        }
        if status_reads.live_due(Instant::now()) {
            let status = read_status(&status_path);
            if let Some(admit) = &admit_staged
                && let Some((campaign, load)) = loaded.and_then(|loaded| loaded())
            {
                if let Some((left, left_load)) = admitted
                    .replace((campaign.clone(), load))
                    .filter(|(left, _)| *left != campaign)
                {
                    if let Some(settling) = &settling
                        && admission.admitted.loads.contains_key(&left)
                    {
                        locked(&settling.left).insert(left.clone());
                    }
                    let session = directory.clone();
                    let ended = Arc::clone(&capture_ended);
                    closing.push(std::thread::spawn(move || {
                        let subject = format!("{left} {left_load}");
                        if let Err(error) = ask_collector_until(&session, "close", &subject, &ended)
                        {
                            eprintln!("Campaign {left} saves after the next hour: {error}");
                        }
                    }));
                }
                let recorded = || replay_io::recorded_before_load(&directory, &campaign, load);
                match admission.check(&campaign, load, recorded, admit) {
                    Ok(Decision::Refused) => {
                        println!(
                            "Load {load} of campaign {campaign} is not recorded in this session; detaching."
                        );
                        requested_detach = true;
                        rerouted = true;
                    }
                    Ok(Decision::Admitted | Decision::Seen) => {}
                    Err(error) => {
                        eprintln!(
                            "Load {load} of campaign {campaign} could not be admitted: {error}"
                        );
                        admission_failures.push(error);
                        requested_detach = true;
                    }
                }
                if requested_detach {
                    detach.set()?;
                    break;
                }
            }
            if names_wanted.is_some_and(|wanted| wanted()) {
                ahead.ask_names();
            }
            stage(Stage::Recording {
                session: &directory,
                status: status.as_ref(),
                names: &ahead.names(),
            });
            if let (Some(settling), Some(auto_save)) = (&settling, auto_save) {
                settling.save.store(auto_save(), Ordering::Release);
                locked(&settling.loaded)
                    .clone_from(&admitted.as_ref().map(|(campaign, _)| campaign.clone()));
                let asked = std::mem::take(&mut *locked(&control.set_aside));
                locked(&settling.requests).extend(asked);
                report_set_aside(settling, stage);
            }
            status_reads.live_read(Instant::now());
        }
        if error_path.exists() {
            detach.set()?;
            requested_detach = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if requested_detach && !stop_observed {
        stop_observed = true;
        stop_qpc = requested_stop_counter(control, &directory);
        stop_load = system_times();
    }
    ahead.promote();
    capture_ended.store(true, Ordering::Release);
    ahead.stop.store(true, Ordering::Release);
    for worker in closing {
        let _ = worker.join();
    }
    let packing = ahead.packing.clone();
    let join_began = Instant::now();
    let (mut ahead, localisation, unanswered) = finish_ahead(ahead, settling.as_deref());
    let mut decisions: BTreeMap<String, bool> = unanswered.into_iter().collect();
    let join_seconds = join_began.elapsed().as_secs_f64();
    if !process.handle.signaled()? {
        detach.set()?;
        let mut wait = CollectorWait::new(Instant::now(), DONE_STALL);
        while !done.handle.signaled()? && !process.handle.signaled()? {
            if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
                let timed_out = format!(
                    "detach timed out {}; collector may still be loaded",
                    end.describe(&directory)
                );
                decisions.extend(control.end_requests());
                let timed_out = with_lost_decisions(timed_out, &decisions);
                let ending = control.seal();
                let read = read_at_stop.as_ref().and_then(Option::as_deref);
                let discarded = match discard_target(ending.as_ref(), read) {
                    None => {
                        return Err(format!("{timed_out}; spool retained and not finalized"));
                    }
                    Some(Err(undiscarded)) => {
                        return Err(format!(
                            "{timed_out}; spool retained and not finalized, and nothing was discarded: {}",
                            undiscarded.reason()
                        ));
                    }
                    Some(Ok(discarded)) => discarded,
                };
                let besides = others_admitted(
                    admit.is_some().then_some(&admission.admitted),
                    Some(discarded),
                );
                let dropped = dropped_by_discard(
                    &directory,
                    target,
                    &ahead,
                    Some(discarded),
                    besides,
                    &mut Vec::new(),
                );
                drop(ahead);
                return Err(
                    match dropped.and_then(|(dropped, _)| discard_held(&directory, target, &dropped)) {
                        Ok(()) => format!(
                            "{timed_out}; the session is discarded: its recording is as it was before it, and its journal goes once the game exits"
                        ),
                        Err(error) => format!(
                            "{timed_out}; Discard is recorded but not carried out yet, and is finished when the session is next taken up: {error}"
                        ),
                    },
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(summary) = step_summary(&directory, "detach_requested", "done") {
            println!("Collector detach {summary}");
        }
    }
    let process_exited = process.handle.signaled()?;
    let spool_final = process_exited || done.handle.signaled()?;
    HistoryRequest::retire_session(&directory, false, process_exited);
    print_status(&status_path);
    let ending = control.seal();
    requested_detach |= ending.is_some() || detach.handle.signaled()?;
    if requested_detach && !stop_observed {
        stop_qpc = requested_stop_counter(control, &directory);
        stop_load = system_times();
    }
    let detach_seconds = counter_seconds(stop_qpc, session.qpc_frequency);
    let settings = RecorderSettings::path();
    let naming = replay_io::Naming {
        localisation: localisation.as_ref(),
        settings: &settings,
        report: None,
    };
    if let Some(settling) = &settling {
        report_set_aside(settling, stage);
    }
    let current = in_progress(
        admitted.as_ref().map(|(campaign, _)| campaign.as_str()),
        read_at_stop.as_ref().map(Option::is_some),
        status_campaign(&status_path),
    );
    if read_at_stop == Some(None) && admitted.is_some() && current.is_none() {
        println!(
            "The campaign in progress could not be read when Stop was chosen, so Stop neither saves nor discards one."
        );
    }
    let _lock = finalization_lock(target)?;
    let mut outcome = terminal_outcome(
        &directory,
        session.id,
        session.process_start_time,
        requested_detach,
        process_exited,
        spool_final,
    );
    outcome.issues.extend(
        admission_failures
            .drain(..)
            .map(|error| format!("a load could not be admitted, so it is not published: {error}")),
    );
    decisions.extend(control.end_requests());
    let recorded_here = replay_io::written_campaigns(&directory)?;
    decisions.retain(|campaign, &mut discard| {
        let here = recorded_here.contains(campaign);
        if !here {
            settle_decision(output, target, campaign, discard, naming, settling.as_deref(), &mut outcome.issues);
        } else if discard {
            ahead.forget_save(campaign);
        }
        here
    });
    if let Some(settling) = &settling {
        report_set_aside(settling, stage);
    }
    for warning in &outcome.warnings {
        eprintln!("Collector warning: {warning}");
    }
    if let Some(witness) = &outcome.witness
        && let Err(error) = publish_failure_status(&directory, witness)
    {
        outcome
            .issues
            .push(format!("terminal status publication failed: {error}"));
    }
    if !outcome.recovery_allowed
        && let Err(error) = write_json(
            &directory.join("status.json"),
            &json!({
                "state":"incomplete","session":session.id,"partial":true,
                "capture_failed":outcome.capture_failed,"failure_code":null,"failed_hour":null,
                "drain_complete":outcome.drain_complete,"error":outcome.issues.join("; "),
            }),
        )
    {
        outcome
            .issues
            .push(format!("incomplete status publication failed: {error}"));
    }
    let discard = discard_target(ending.as_ref(), read_at_stop.flatten().as_deref());
    let discarding = discard.is_some();
    let discarded = discard.and_then(Result::ok);
    let undiscarded = discard.and_then(Result::err);
    let current = stopped_on(ending.as_ref(), discarding, discarded, current);
    let (dropped, others_keep) = match discarded {
        Some(discarded) => {
            let besides = others_admitted(
                admit.is_some().then_some(&admission.admitted),
                Some(discarded),
            );
            dropped_by_discard(
                &directory,
                target,
                &ahead,
                Some(discarded),
                besides,
                &mut outcome.issues,
            )?
        }
        None => (Vec::new(), false),
    };
    note_discards(&directory, target, &dropped)?;
    match game_facts.finish() {
        Ok(summary) => println!("Game facts: {summary}."),
        Err(error) => {
            println!("Game facts are not carried: {error}");
            outcome
                .warnings
                .push(format!("The recording carries no game facts: {error}"));
        }
    }
    if let Some(undiscarded) = undiscarded {
        println!("Nothing is discarded: {}.", undiscarded.reason());
    }
    if discarded.is_some() && others_keep {
        if let Some(current) = dropped.first() {
            println!("Discarding campaign {current} of this session.");
        }
        discard_dropped(&directory, target, &mut ahead, &dropped)?;
    } else if discarded.is_some() {
        if process_exited {
            println!("Game process exited; discarding this session.");
        } else {
            println!("Collector journal closed; discarding this session.");
        }
        discard_dropped(&directory, target, &mut ahead, &dropped)?;
        let (completed, result) = discarded_end(
            &directory,
            target,
            &outcome,
            requested_detach,
            current.as_deref(),
            CaptureOutcome {
                detach_requested: requested_detach,
                process_exited,
                rerouted,
                ..CaptureOutcome::default()
            },
        );
        return finish_capture(
            &directory,
            target,
            &active,
            &session,
            &dll,
            process_exited,
            completed,
            result,
            stage,
            || None,
        );
    }
    if let Some(admit) = &admit_staged
        && outcome.recovery_allowed
        && let Err(error) = admission.check_journaled(&mut ahead, admit)
    {
        admission_failures.push(error);
    }
    outcome.issues.extend(
        admission_failures
            .into_iter()
            .map(|error| format!("a load could not be admitted, so it is not published: {error}")),
    );
    if process_exited {
        println!("Game process exited; recovering the flushed journal.");
    } else {
        println!("Collector journal closed; finalizing the committed prefix.");
    }
    let auto = auto_save.is_none_or(|auto_save| auto_save());
    let destination = |campaign: &str| {
        if let Some(decided) = decided_destination(&decisions, campaign) {
            decided
        } else if saved_at_end(
            campaign,
            current.as_deref(),
            auto,
            ending.is_some(),
            discarding,
        ) {
            replay_io::Destination::Recording
        } else if discarding && current.as_deref() == Some(campaign) {
            replay_io::Destination::Neither
        } else {
            replay_io::Destination::Unsaved
        }
    };
    let spooled = replay_io::written_campaigns(&directory)?;
    let admitted = admit.is_some().then_some(&admission.admitted);
    let recovery_allowed = outcome.recovery_allowed;
    let saving_now = |campaign: &str| {
        recovery_allowed
            && publishes(admitted, campaign)
            && destination(campaign) == replay_io::Destination::Recording
    };
    let mut waiting = Vec::new();
    for campaign in spooled.iter().filter(|campaign| {
        outcome.recovery_allowed
            && publishes(admitted, campaign)
            && destination(campaign) == replay_io::Destination::Unsaved
    }) {
        waiting.push((campaign.clone(), ahead.unsaved(campaign)));
    }
    let listed: Vec<_> = waiting
        .iter()
        .map(|(campaign, unsaved)| waiting_json(campaign, unsaved.as_ref()))
        .chain(
            spooled
                .iter()
                .filter(|campaign| saving_now(campaign))
                .map(|campaign| waiting_json(campaign, None)),
        )
        .collect();
    if !spooled.is_empty() {
        replay_io::write_session_end(
            &directory,
            target,
            WAITING_FILE,
            &json!({"target":target, "waiting":listed}),
        )?;
    }
    let (continued, blocked) = save_continued(
        output,
        target,
        &spooled,
        admitted,
        &|campaign| destination(campaign) == replay_io::Destination::Recording,
        naming,
    );
    outcome.issues.extend(blocked.values().cloned());
    if nothing_recorded_is_ok
        && outcome.recovery_allowed
        && outcome.issues.is_empty()
        && outcome.witness.is_none()
        && spooled.is_empty()
    {
        if continued.is_empty() {
            println!("Nothing was recorded in this session.");
        }
        return finish_capture(
            &directory,
            target,
            &active,
            &session,
            &dll,
            process_exited,
            json!({
                "replays":continued,"nothing_recorded":continued.is_empty(),
                "discarded_campaign":current.as_ref().filter(|_| discarding),
                "detach_requested":requested_detach,
                "drain_complete":outcome.drain_complete,"capture_failed":false,
                "partial":false,"issues":[],"warnings":outcome.warnings,
            }),
            Ok(CaptureOutcome {
                detach_requested: requested_detach,
                process_exited,
                replays: continued,
                discarded: discarded.is_some(),
                undiscarded,
                rerouted,
                ..CaptureOutcome::default()
            }),
            stage,
            || None,
        );
    }
    let destination = |campaign: &str| {
        if blocked.contains_key(campaign) {
            replay_io::Destination::Neither
        } else {
            destination(campaign)
        }
    };
    let saving = |records| stage(Stage::Saving { records });
    let progress = FinalizationProgress::new(&directory, &saving);
    if spooled
        .iter()
        .any(|campaign| destination(campaign) == replay_io::Destination::Recording)
    {
        stage(Stage::Saving { records: 0 });
    }
    let finalize = || {
        replay_io::finalize_session_ahead(
            &directory,
            target,
            &|| {
                progress.advance();
                Ok(())
            },
            ahead,
            naming,
            &destination,
        )
    };
    let finalized_result = outcome.recovery_allowed.then(|| match &packing {
        Some(packing) => packing.run(finalize),
        None => finalize(),
    });
    let (mut finalized, omitted, unrenamed, failed, saved_campaigns) = match finalized_result {
        Some(Ok(done)) => {
            outcome.issues.extend(
                done.errors
                    .into_iter()
                    .map(|error| format!("replay finalization failed: {error}")),
            );
            (done.replays, done.omitted, done.unrenamed, done.failed_campaigns, done.saved_campaigns)
        }
        Some(Err(error)) => {
            outcome
                .issues
                .push(format!("replay finalization failed: {error}"));
            Default::default()
        }
        None => Default::default(),
    };
    finalized.extend(continued);
    for (campaign, _) in decisions.iter().filter(|(_, discard)| **discard) {
        if let Err(error) = discard_recording(output, target, campaign) {
            outcome.issues.push(format!("campaign {campaign} wasn’t discarded: {error}"));
        }
    }
    let waiting: Vec<(String, Value)> = spooled
        .iter()
        .filter(|campaign| {
            failed.contains(*campaign) || waiting.iter().any(|(set_aside, _)| set_aside == *campaign)
        })
        .filter_map(|campaign| Some((campaign.clone(), waits(target, campaign)?)))
        .collect();
    for campaign in &saved_campaigns {
        if let Err(error) = unlist_waiting_sessions(output, target, campaign, Some(&directory)) {
            eprintln!("Campaign {campaign} is saved, and still listed as waiting: {error}");
        }
    }
    if !waiting.is_empty() {
        print!(
            "{} campaign(s) wait to be saved in {}",
            waiting.len(),
            directory.display()
        );
        match auto_save {
            None => println!(
                "; `recorder finalize \"{}\" --out \"{}\"` saves them.",
                directory.display(),
                target.display()
            ),
            Some(_) => println!("."),
        }
    }
    drop(localisation);
    let completed = json!({
        "replays":finalized,
        "target":target,
        "waiting":waiting.iter().map(|(_, entry)| entry).collect::<Vec<_>>(),
        "discarded_campaign":current.as_ref().filter(|_| discarding),
        "omitted":omitted.iter().map(Omission::to_json).collect::<Vec<_>>(),
        "detach_requested":requested_detach,
        "drain_complete":outcome.drain_complete,
        "capture_failed":outcome.capture_failed,
        "failure_code":outcome.witness.as_ref().map(|_| "allocation_failed"),
        "failed_hour":outcome.witness.as_ref().and_then(|witness| witness.failed_hour),
        "partial":!outcome.issues.is_empty(),
        "issues":outcome.issues,
        "warnings":outcome.warnings,
    });
    let result = capture_result(
        &directory,
        &outcome,
        CaptureOutcome {
            detach_requested: requested_detach,
            process_exited,
            replays: finalized,
            discarded: discarded.is_some(),
            undiscarded,
            rerouted,
            omitted,
            waiting: waiting.into_iter().map(|(campaign, _)| campaign).collect(),
            unrenamed,
            failure_report: None,
        },
    );
    finish_capture(
        &directory,
        target,
        &active,
        &session,
        &dll,
        process_exited,
        completed,
        result,
        stage,
        || {
            let seconds =
                |duration: Option<Duration>| duration.map(|duration| duration.as_secs_f64());
            let phases = packing
                .as_ref()
                .map(|packing| packing.timings())
                .unwrap_or([None; 5]);
            let diagnostics = json!({
                "save_ready": outcome.issues.is_empty(),
                "stop_to_save_ready_s": outcome.issues.is_empty().then(|| counter_seconds(stop_qpc, session.qpc_frequency)).flatten(),
                "detach_s": detach_seconds, "join_s": join_seconds,
                "drain_s": seconds(phases[0]), "pack_s": seconds(phases[1]),
                "write_s": seconds(phases[2]), "readback_s": seconds(phases[3]),
                "publication_s": seconds(phases[4]),
                "machine_cpu_busy_percent": system_busy(stop_load),
                "phase_accounting": "coordinator wall intervals after promotion; detach and join overlap packing; unobserved phases are null",
            });
            Some(diagnostics)
        },
    )
}

fn status_campaign(status_path: &Path) -> Option<String> {
    read_status(status_path)
        .and_then(|status| status["campaign"].as_str().map(str::to_owned))
        .filter(|campaign| replay_data::identity::is_campaign_uuid(campaign.as_bytes()))
}

fn others_admitted(admitted: Option<&replay_io::Admissions>, current: Option<&str>) -> bool {
    admitted.is_some_and(|admitted| {
        admitted
            .loads
            .keys()
            .any(|campaign| Some(campaign.as_str()) != current)
    })
}

fn dropped_by_discard(
    directory: &Path,
    target: &Path,
    live: &replay_io::Live,
    current: Option<&str>,
    others_admitted: bool,
    issues: &mut Vec<String>,
) -> Result<(Vec<String>, bool), String> {
    let spooled = replay_io::written_campaigns(directory)?;
    let others_keep =
        others_admitted || others_unsaved(directory, target, live, &spooled, current, issues);
    let dropped = if others_keep {
        current
            .filter(|current| spooled.iter().any(|spooled| spooled == current))
            .map(str::to_owned)
            .into_iter()
            .collect()
    } else {
        spooled
    };
    Ok((dropped, others_keep))
}

fn note_discards(directory: &Path, target: &Path, dropped: &[String]) -> Result<(), String> {
    for campaign in dropped {
        replay_io::note_discard(directory, target, campaign).map_err(|error| {
            format!("The discard couldn’t be recorded, so nothing was discarded: {error}")
        })?;
    }
    Ok(())
}

fn discard_dropped(
    directory: &Path,
    target: &Path,
    live: &mut replay_io::Live,
    dropped: &[String],
) -> Result<(), String> {
    for campaign in dropped {
        live.discard(directory, target, campaign, replay_io::Discarded::Session)
            .map_err(|error| {
                format!(
                    "campaign {campaign} is not discarded yet: {error}; the discard stands and is finished when the session is next taken up"
                )
            })?;
    }
    Ok(())
}

fn discard_held(directory: &Path, target: &Path, dropped: &[String]) -> Result<(), String> {
    let _lock = finalization_lock(target)?;
    note_discards(directory, target, dropped)?;
    replay_io::reconcile_session(directory, target)
}

fn in_progress(
    loaded: Option<&str>,
    stop_read: Option<bool>,
    written: Option<String>,
) -> Option<String> {
    match loaded {
        Some(loaded)
            if stop_read == Some(false)
                && written.as_deref().is_some_and(|written| written != loaded) =>
        {
            None
        }
        Some(loaded) => Some(loaded.to_owned()),
        None => written,
    }
}

fn stopped_on(ending: Option<&Stop>, discarding: bool, discarded: Option<&str>, current: Option<String>) -> Option<String> {
    match ending {
        _ if discarding => discarded.map(str::to_owned),
        Some(Stop::Save(Some(confirmed))) => Some(confirmed.clone()),
        _ => current,
    }
}

fn saved_at_end(
    campaign: &str,
    current: Option<&str>,
    auto_save: bool,
    stop: bool,
    discard: bool,
) -> bool {
    let in_progress = current == Some(campaign);
    !(discard && in_progress) && (auto_save || (stop && !discard && in_progress))
}

fn with_lost_decisions(timed_out: String, decisions: &BTreeMap<String, bool>) -> String {
    if decisions.is_empty() {
        return timed_out;
    }
    let lost: Vec<String> = decisions.iter().map(|(campaign, &discard)| format!("{} of campaign {campaign}", if discard { "Discard" } else { "Save" })).collect();
    format!("{timed_out}; not carried out, so choose again once the session is taken up: {}", lost.join(", "))
}

fn latest_choices(requests: Vec<(String, bool)>) -> Vec<(String, bool)> {
    let mut seen = BTreeSet::new();
    let mut latest: Vec<_> = requests.into_iter().rev().filter(|(campaign, _)| seen.insert(campaign.clone())).collect();
    latest.reverse();
    latest
}

fn requeue(settling: &Settling, campaign: String, discard: bool) {
    let mut requests = locked(&settling.requests);
    if !requests.iter().any(|(asked, _)| *asked == campaign) {
        requests.insert(0, (campaign, discard));
    }
}

fn finish_ahead(
    ahead: FinalizingAhead,
    settling: Option<&Settling>,
) -> (replay_io::Live, Option<Localisation>, Vec<(String, bool)>) {
    let (live, localisation) = ahead.finish();
    let unanswered = settling.map(|settling| std::mem::take(&mut *locked(&settling.requests))).unwrap_or_default();
    (live, localisation, unanswered)
}

fn decided_destination(decisions: &BTreeMap<String, bool>, campaign: &str) -> Option<replay_io::Destination> {
    decisions.get(campaign).map(|&discard| if discard { replay_io::Destination::Unsaved } else { replay_io::Destination::Recording })
}

fn settle_decision(root: &Path, target: &Path, campaign: &str, discard: bool, naming: replay_io::Naming<'_>, settling: Option<&Settling>, issues: &mut Vec<String>) {
    let settled = if discard {
        discard_recording(root, target, campaign).map(|()| SetAside::Discarded(campaign.to_owned()))
    } else {
        save_recording(root, target, campaign, naming).map(|saved| SetAside::Saved(campaign.to_owned(), saved))
    };
    let done = settled.unwrap_or_else(|error| SetAside::Failed(campaign.to_owned(), error));
    match settling {
        Some(settling) => locked(&settling.done).push(done),
        None => {
            if let SetAside::Failed(campaign, error) = done {
                issues.push(format!("campaign {campaign}: {error}"));
            }
        }
    }
}

fn report_set_aside(settling: &Settling, stage: &dyn Fn(Stage<'_>)) {
    let waiting = locked(&settling.waiting).take();
    let done = std::mem::take(&mut *locked(&settling.done));
    if waiting.is_some() || !done.is_empty() {
        stage(Stage::SetAside {
            waiting: waiting.as_deref(),
            done: &done,
        });
    }
}

fn remove_active(active: &Path, directory: &Path) -> Result<(), String> {
    if fs::read(active)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| names_session(&value, directory))
    {
        fs::remove_file(active).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn discard_session(directory: &Path) -> Result<(), String> {
    read_status(&directory.join("completed.json"))
        .ok_or("discarded session has no readable completion record")?;
    for entry in fs::read_dir(directory).map_err(|error| failure_text(directory, error))? {
        let entry = entry.map_err(|error| failure_text(directory, error))?;
        if entry.file_name() == "completed.json" || entry.file_name() == "completion.lock" {
            continue;
        }
        let path = entry.path();
        let removed = if entry
            .file_type()
            .map_err(|error| failure_text(&path, error))?
            .is_dir()
        {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        match removed {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            removed => removed.map_err(|error| format!("discard {}: {error}", path.display()))?,
        }
    }
    {
        let _lock = completion_lock(directory)?;
        let path = directory.join("completed.json");
        let mut completed =
            read_status(&path).ok_or("discarded session has no readable completion record")?;
        completed["cleanup_pending"] = json!(false);
        write_json(&path, &completed)?;
    }
    let _ = fs::remove_file(directory.join("completion.lock"));
    Ok(())
}

fn failure_text(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

fn session_discarded(directory: &Path) -> bool {
    read_status(&directory.join("completed.json")).is_some_and(|value| value["discarded"] == true)
}

fn read_status(path: &Path) -> Option<Value> {
    let value = history_requests::open_shared(path)
        .ok()
        .and_then(|file| serde_json::from_reader(file).ok());
    #[cfg(test)]
    completion_step(path, CompletionStep::Read);
    value
}

#[derive(Debug)]
struct TerminalOutcome {
    drain_complete: Option<bool>,
    issues: Vec<String>,
    warnings: Vec<String>,
    witness: Option<TerminalWitness>,
    recovery_allowed: bool,
    capture_failed: bool,
}

fn classify_terminal(
    session: u64,
    requested_detach: bool,
    process_exited: bool,
    status: Option<&Value>,
    error_file: Option<&str>,
) -> TerminalOutcome {
    let mut outcome = TerminalOutcome {
        drain_complete: None,
        issues: Vec::new(),
        warnings: Vec::new(),
        witness: None,
        recovery_allowed: true,
        capture_failed: status.is_some_and(|status| status["capture_failed"] == true),
    };
    let mut issue = |reason: String| {
        if !outcome.issues.contains(&reason) {
            outcome.issues.push(reason);
        }
    };
    if let Some(error) = error_file.filter(|error| !error.trim().is_empty()) {
        issue(format!("collector terminal error: {}", error.trim()));
    }
    if let Some(status) = status {
        for warning in [
            status["warning"].as_str(),
            status["last_error"].as_str(),
            status["restoration_error"].as_str(),
        ]
        .into_iter()
        .flatten()
        .filter(|warning| !warning.trim().is_empty())
        {
            if !outcome.warnings.iter().any(|previous| previous == warning) {
                outcome.warnings.push(warning.to_owned());
            }
        }
        let captures = ["columnar", "columnar_finished"]
            .into_iter()
            .filter_map(|list| status["spool_work"][list].as_array())
            .flatten();
        for capture in captures {
            let failures = capture["failures"].as_u64().unwrap_or(0);
            if failures == 0 {
                continue;
            }
            let reason = capture["stopped"].as_str().unwrap_or("no reason recorded");
            let lost = capture["lost_hours"].as_u64().unwrap_or(0);
            issue(format!(
                "native capture failed {failures} time(s), last at {reason}; its lost hours are gaps ({lost} \
                 the rows hold), and each failure began a new capture"
            ));
        }
        let correct_session = status["session"].as_u64() == Some(session);
        if !correct_session {
            let reason =
                "collector terminal status has a missing or different session id".to_owned();
            if requested_detach || !process_exited {
                issue(reason);
            } else {
                outcome.warnings.push(reason);
            }
        }
        if let Some(error) = status["error"]
            .as_str()
            .filter(|error| !error.trim().is_empty())
        {
            issue(format!("collector terminal error: {}", error.trim()));
        }
        if matches!(
            status["state"].as_str(),
            Some("detached" | "pinned" | "capture_failed")
        ) && correct_session
        {
            outcome.drain_complete = status["drain_complete"].as_bool();
            if outcome.drain_complete == Some(false) {
                issue("collector final queue drain or spool sync was incomplete".into());
            }
        }
    }
    if requested_detach && outcome.drain_complete != Some(true) {
        issue(if process_exited {
            "game exited after detach was requested without confirmation of a completed collector drain".into()
        } else {
            "collector ended without confirming a completed final queue drain and spool sync".into()
        });
    }
    outcome
}

fn others_unsaved(
    directory: &Path,
    target: &Path,
    live: &replay_io::Live,
    written: &[String],
    current: Option<&str>,
    issues: &mut Vec<String>,
) -> bool {
    let journaled = replay_io::journal_campaigns(directory).unwrap_or_else(|_| written.to_vec());
    let mut unsaved = false;
    for campaign in written
        .iter()
        .filter(|campaign| Some(campaign.as_str()) != current)
    {
        if journaled.contains(campaign) || live.unsaved(campaign).is_some() {
            unsaved = true;
            continue;
        }
        match replay_io::unsaved(target, campaign) {
            Ok(held) => unsaved |= held.is_some(),
            Err(error) => {
                unsaved = true;
                issues.push(format!(
                    "campaign {campaign}'s unsaved hours could not be read, so it is kept: {error}"
                ));
            }
        }
    }
    unsaved
}

fn terminal_outcome(
    directory: &Path,
    session: u64,
    process_start_time: u64,
    requested_detach: bool,
    process_exited: bool,
    spool_final: bool,
) -> TerminalOutcome {
    let mut status_read_issues = Vec::new();
    let mut error_read_issues = Vec::new();
    let status_path = directory.join("status.json");
    let mut status = match fs::read(&status_path) {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(status) => Some(status),
            Err(error) => {
                status_read_issues.push(format!(
                    "cannot parse terminal status {}: {error}",
                    status_path.display()
                ));
                None
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            status_read_issues.push(format!(
                "cannot read terminal status {}: {error}",
                status_path.display()
            ));
            None
        }
    };
    let witness = replay_io::read_failure_witness(directory).and_then(|witness| {
        if witness.as_ref().is_some_and(|witness| {
            witness.session != session || witness.process_start_time != process_start_time
        }) {
            Err("collector failure witness belongs to a different process session".into())
        } else {
            Ok(witness)
        }
    });
    if let Ok(Some(witness)) = &witness {
        status = Some(failure_status(status, witness));
    }
    if let Some(status) = &mut status
        && let Some(error) = status["error"]
            .as_str()
            .map(|error| collector_error(directory, error))
    {
        status["error"] = json!(error);
    }
    let error_path = directory.join("bin/collector.error.txt");
    let error_file = match fs::read_to_string(&error_path) {
        Ok(error) => Some(collector_error(directory, &error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            error_read_issues.push(format!(
                "cannot read collector terminal error {}: {error}",
                error_path.display()
            ));
            None
        }
    };
    let mut outcome = classify_terminal(
        session,
        requested_detach,
        process_exited,
        status.as_ref(),
        error_file.as_deref(),
    );
    if requested_detach || !process_exited {
        outcome.issues.extend(status_read_issues);
    } else {
        outcome.warnings.extend(status_read_issues);
    }
    outcome.issues.extend(error_read_issues);
    match witness {
        Ok(Some(witness)) => outcome.witness = Some(witness),
        Ok(None) => {
            if status
                .as_ref()
                .is_some_and(|status| status["capture_failed"] == true)
            {
                outcome.issues.push("collector capture failed without a valid terminal witness; spool retained and not finalized".into());
                outcome.recovery_allowed = false;
            } else if requested_detach && outcome.drain_complete != Some(true) && !spool_final {
                outcome.recovery_allowed = false;
            }
        }
        Err(error) => {
            outcome.issues.push(format!(
                "invalid collector failure witness: {error}; spool retained and not finalized"
            ));
            outcome.recovery_allowed = false;
        }
    }
    outcome
}

fn create_failure_witness(
    directory: &Path,
    session: u64,
    process_start_time: u64,
) -> Result<(), String> {
    use std::io::Write;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join(WITNESS_FILE_NAME))
        .map_err(|error| error.to_string())?;
    file.write_all(&TerminalWitness::prepared(session, process_start_time))
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn failure_status(status: Option<Value>, witness: &TerminalWitness) -> Value {
    let mut status = status
        .filter(|status| status.is_object() && status["session"].as_u64() == Some(witness.session))
        .unwrap_or_else(|| json!({}));
    status["state"] = json!("capture_failed");
    status["session"] = json!(witness.session);
    status["drain_complete"] = json!(witness.drain_complete);
    status["capture_failed"] = json!(true);
    status["failure_code"] = json!("allocation_failed");
    status["failure_phase"] = json!(witness.phase);
    status["failure_detail"] = json!(witness.code);
    status["failed_hour"] = json!(witness.failed_hour);
    status["partial"] = json!(true);
    status["error"] = json!("collector allocation failed");
    status
}

fn publish_failure_status(directory: &Path, witness: &TerminalWitness) -> Result<(), String> {
    let path = directory.join("status.json");
    let previous = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let mut status = failure_status(previous, witness);
    if let Ok(bytes) = fs::read(directory.join("collector-failure-detail.json"))
        && let Ok(detail) = serde_json::from_slice::<Value>(&bytes)
        && detail["session"].as_u64() == Some(witness.session)
        && detail["process_start_time"].as_u64() == Some(witness.process_start_time)
        && detail["family"].is_string()
        && detail["requested_bytes"].is_u64()
    {
        status["allocation_family"] = detail["family"].clone();
        status["allocation_requested_bytes"] = detail["requested_bytes"].clone();
    }
    write_json(&path, &status)
}

fn live_failure(
    directory: &Path,
    session: u64,
    process_start: u64,
) -> Result<Option<TerminalWitness>, String> {
    let Ok(Some(witness)) = replay_io::read_failure_witness(directory) else {
        return Ok(None);
    };
    if witness.session != session || witness.process_start_time != process_start {
        return Err(
            "collector failure witness belongs to a different process session; spool retained"
                .into(),
        );
    }
    Ok(Some(witness))
}

fn refuse_failed_refresh(directory: &Path, active: &Value) -> Result<(), String> {
    let process_start = active["process_start_time"]
        .as_u64()
        .ok_or("active recorder has no process start time")?;
    let session = active["session_id"].as_u64().unwrap_or(process_start);
    if let Some(witness) = live_failure(directory, session, process_start)? {
        publish_failure_status(directory, &witness)?;
        return Err(format!(
            "collector capture failed at hour {:?} (phase {}, code {}); refresh unavailable; detach recovers the committed prefix",
            witness.failed_hour, witness.phase, witness.code
        ));
    }
    Ok(())
}

fn completed_detach(value: &Value) -> Result<bool, String> {
    let partial = value["partial"]
        .as_bool()
        .ok_or("completion has no partial outcome")?;
    let issues = value["issues"]
        .as_array()
        .ok_or("completion has no issue list")?;
    let issues: Vec<_> = issues
        .iter()
        .map(|issue| issue.as_str().ok_or("completion issue is not text"))
        .collect::<Result<_, _>>()?;
    if partial || !issues.is_empty() {
        return Err(format!(
            "recording completion is partial: {}",
            if issues.is_empty() {
                "no completion details were recorded".into()
            } else {
                issues.join("; ")
            }
        ));
    }
    let requested = value["detach_requested"]
        .as_bool()
        .ok_or("completion has no detach-request outcome")?;
    let detached = value["detached"]
        .as_bool()
        .ok_or("completion has no detached outcome")?;
    let exited = value["process_exited"]
        .as_bool()
        .ok_or("completion has no process-exit outcome")?;
    if (requested || detached) && value["drain_complete"].as_bool() != Some(true) {
        return Err("requested detach has no completed collector drain".into());
    }
    if !detached && !exited {
        return Err("recording finalized, but collector unload is unconfirmed; quit the game before recording again".into());
    }
    Ok(detached)
}

fn request_refresh(output: &Path) -> Result<(), String> {
    request_refresh_with(output, |_, published| {
        if published.elapsed() >= Duration::from_secs(30) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
        true
    })
}

fn request_refresh_with(
    output: &Path,
    mut wait: impl FnMut(&Path, Instant) -> bool,
) -> Result<(), String> {
    let active: Value = serde_json::from_slice(
        &fs::read(output.join("active.json"))
            .map_err(|e| format!("no active recorder here: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let session_dir = PathBuf::from(
        active["session_dir"]
            .as_str()
            .ok_or("active recorder has no session directory")?,
    );
    refuse_failed_refresh(&session_dir, &active)?;
    let token = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos()
    );
    let folder = session_dir.join("refresh-requests");
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let ack = folder.join(format!("{token}.ack"));
    let request = folder.join(format!("{token}.request"));
    let temporary = folder.join(format!("{token}.tmp"));
    fs::write(&temporary, &token).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&temporary, &request) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    let published = Instant::now();
    let withdraw = || {
        let _ = fs::remove_file(&request);
        let _ = fs::remove_file(&ack);
    };
    loop {
        if let Err(error) = refuse_failed_refresh(&session_dir, &active) {
            withdraw();
            return Err(error);
        }
        if fs::read_to_string(&ack).is_ok_and(|text| text.trim() == token) {
            withdraw();
            println!("Full refresh requested: the next hour's sample reads every scheduled table, taken as the following hourly update is entered.");
            return Ok(());
        }
        if !wait(&request, published) {
            withdraw();
            return Err("the collector did not acknowledge the refresh request".into());
        }
    }
}

fn signal_detach_request(
    session: &Path,
    counter: Option<u64>,
    signal: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let written = counter
        .ok_or_else(|| "performance counter unavailable".to_owned())
        .and_then(|qpc| {
            fs::write(session.join("stop.request"), qpc.to_string())
                .map_err(|error| error.to_string())
        });
    if let Err(error) = written {
        eprintln!("Stop-to-save-ready timing unavailable; detach will still be requested: {error}");
    }
    signal()
}

fn request_detach(output: &Path) -> Result<(), String> {
    let active: Value = serde_json::from_slice(
        &fs::read(output.join("active.json"))
            .map_err(|e| format!("no active recorder here: {e}"))?,
    )
    .map_err(|e| e.to_string())?;
    let name = active["detach_event"]
        .as_str()
        .ok_or("active recorder has no detach event")?;
    let session_dir = PathBuf::from(
        active["session_dir"]
            .as_str()
            .ok_or("active recorder has no session directory")?,
    );
    let recorder_pid = active["recorder_pid"]
        .as_u64()
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or("active recorder has no valid recorder pid")?;
    let recorder = Process::open(recorder_pid, false)
        .map_err(|error| format!("recording controller unavailable: {error}"))?;
    signal_detach_request(&session_dir, performance_counter(), || {
        Event::open(name)?.set()
    })?;
    println!("Detach requested. Waiting for recorder finalization.");
    let mut watch = ProgressWatch::new(Instant::now());
    while !session_dir.join("completed.json").exists() {
        if recorder.handle.signaled()? {
            if session_dir.join("completed.json").exists() {
                break;
            }
            return Err(
                "recording controller exited before detach/finalization completed; spool retained"
                    .into(),
            );
        }
        if watch.stalled(
            Instant::now(),
            progress_stamp(&session_dir, recorder_activity(&recorder)),
        ) {
            return Err("detach/finalization stopped making progress; inspect the recorder console and retained spool".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut completed: Value = serde_json::from_slice(
        &fs::read(session_dir.join("completed.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    if completed["unload_pending"] == true
        && completed["partial"] == false
        && completed["issues"].as_array().is_some_and(Vec::is_empty)
    {
        let observation = UnloadObservation {
            directory: session_dir.clone(),
            dll: PathBuf::from(
                active["dll"]
                    .as_str()
                    .ok_or("active recorder has no collector path")?,
            ),
            pid: active["game_pid"]
                .as_u64()
                .and_then(|pid| u32::try_from(pid).ok())
                .ok_or("active recorder has no valid game pid")?,
            process_start: active["process_start_time"]
                .as_u64()
                .ok_or("active recorder has no game start time")?,
            process_exited: false,
        };
        refresh_unload_completion(&session_dir, observation.observe())?;
        completed = read_status(&session_dir.join("completed.json"))
            .ok_or("saved session has no readable completion record")?;
    }
    if completed_detach(&completed)? {
        println!("Detached and finalized: {}", session_dir.display());
    } else {
        println!(
            "Game exited and replay finalized: {}",
            session_dir.display()
        );
    }
    Ok(())
}

const DETACH_STALL_TIMEOUT: Duration = DONE_STALL;
struct ProgressWatch {
    work: StampWatch<(
        Option<SystemTime>,
        Option<SystemTime>,
        Option<u64>,
        Option<u64>,
    )>,
    activity: Option<(u64, u64, u64)>,
    last_activity: Instant,
}
impl ProgressWatch {
    fn new(now: Instant) -> Self {
        Self {
            work: StampWatch::new(now),
            activity: None,
            last_activity: now,
        }
    }
    fn stalled(&mut self, now: Instant, stamp: ProgressStamp) -> bool {
        let previous = self.work.stamp;
        let advanced =
            |next: Option<SystemTime>, old: Option<SystemTime>| next.is_some() && next != old;
        let changed = advanced(stamp.collector, previous.0)
            || advanced(stamp.finalization, previous.1)
            || stamp
                .work
                .is_some_and(|work| previous.2.is_none_or(|old| work > old))
            || stamp
                .passes
                .is_some_and(|work| previous.3.is_none_or(|old| work > old));
        if changed {
            self.work.last_progress = now;
            self.last_activity = now;
        }
        self.work.stamp = (
            stamp.collector.or(previous.0),
            stamp.finalization.or(previous.1),
            stamp.work.or(previous.2),
            stamp.passes.or(previous.3),
        );
        if stamp.activity.is_some() && stamp.activity != self.activity {
            self.last_activity = now;
        }
        self.activity = stamp.activity;
        let quiet = now.duration_since(self.work.last_progress);
        quiet >= self.work.stall
            && (now.duration_since(self.last_activity) >= self.work.stall
                || quiet >= Duration::from_secs(10 * 60))
    }
}
#[derive(Default, PartialEq, Eq, Clone, Copy)]
struct ProgressStamp {
    collector: Option<SystemTime>,
    finalization: Option<SystemTime>,
    work: Option<u64>,
    passes: Option<u64>,
    activity: Option<(u64, u64, u64)>,
}
struct StampWatch<S> {
    last_progress: Instant,
    stamp: S,
    stall: Duration,
}
impl<S: Default + PartialEq> StampWatch<S> {
    fn new(now: Instant) -> Self {
        Self::with_stall(now, DETACH_STALL_TIMEOUT)
    }
    fn with_stall(now: Instant, stall: Duration) -> Self {
        Self {
            last_progress: now,
            stamp: S::default(),
            stall,
        }
    }
    fn stalled(&mut self, now: Instant, stamp: S) -> bool {
        if stamp != self.stamp {
            self.stamp = stamp;
            self.last_progress = now;
        }
        now.duration_since(self.last_progress) >= self.stall
    }
}
fn recorder_activity(recorder: &Process) -> Option<(u64, u64, u64)> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetProcessIoCounters, GetProcessTimes, IO_COUNTERS},
    };
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    let mut io = IO_COUNTERS::default();
    if unsafe { GetProcessTimes(recorder.handle.0, creation, exit, kernel, user) } == 0
        || unsafe { GetProcessIoCounters(recorder.handle.0, &mut io) } == 0
    {
        return None;
    }
    let ticks =
        |time: FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Some((
        ticks(*kernel).saturating_add(ticks(*user)),
        io.ReadTransferCount,
        io.WriteTransferCount,
    ))
}

fn progress_stamp(directory: &Path, activity: Option<(u64, u64, u64)>) -> ProgressStamp {
    let metadata = |name| {
        history_requests::open_shared(&directory.join(name))
            .ok()?
            .metadata()
            .ok()
    };
    let modified = |name| metadata(name).and_then(|metadata| metadata.modified().ok());
    let finalization = modified("finalization-progress.json");
    let work = metadata("finalization-work").map(|metadata| metadata.len());
    let finalizing = finalization.is_some()
        || work.is_some()
        || directory.join("history-requests/detached").exists();
    ProgressStamp {
        collector: if finalizing {
            None
        } else {
            modified("status.json").max(modified(COLLECTOR_PROGRESS))
        },
        finalization,
        work,
        passes: metadata("finalization-pass-work").map(|metadata| metadata.len()),
        activity: if finalizing { activity } else { None },
    }
}

const COLLECTOR_PROGRESS: &str = "progress";
const READY_STALL: Duration = Duration::from_secs(180);
const LOAD_POLL: Duration = Duration::from_millis(100);

fn wait_for_loaded_campaign(
    process: &Process,
    pid: u32,
    control: &Control,
) -> Result<bool, String> {
    let base = crate::game_sites::module_base(process, pid)?;
    crate::game_sites::locate(&process.path, base)?;
    let memory = ProcessMemory(&process.handle);
    let mut announced = false;
    loop {
        if process.handle.signaled()? {
            return Err("game exited before a campaign was loaded".into());
        }
        if control.stopping() {
            return Ok(false);
        }
        if crate::clock::campaign_loaded(&memory, base) {
            return Ok(true);
        }
        if !announced {
            println!("Waiting for a campaign to load before attaching.");
            announced = true;
        }
        std::thread::sleep(LOAD_POLL);
    }
}
const DONE_STALL: Duration = Duration::from_secs(180);
const COLLECTOR_WAIT_CAP: Duration = Duration::from_secs(15 * 60);

type CollectorStamp = (Option<u64>, Option<SystemTime>);

fn collector_stamp(directory: &Path) -> CollectorStamp {
    (
        fs::metadata(directory.join(COLLECTOR_PROGRESS))
            .ok()
            .map(|metadata| metadata.len()),
        fs::metadata(directory.join("status.json"))
            .ok()
            .and_then(|metadata| metadata.modified().ok()),
    )
}

struct CollectorWait {
    started: Instant,
    watch: StampWatch<CollectorStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitEnd {
    Stalled(Duration),
    Capped,
}

impl WaitEnd {
    fn describe(self, directory: &Path) -> String {
        let step = fs::read_to_string(directory.join(COLLECTOR_PROGRESS))
            .ok()
            .and_then(|text| last_step(&text))
            .unwrap_or_else(|| "none recorded".into());
        match self {
            Self::Stalled(stall) => format!(
                "after {} s without collector progress (last step: {step})",
                stall.as_secs()
            ),
            Self::Capped => format!(
                "after the {} s cap (last step: {step})",
                COLLECTOR_WAIT_CAP.as_secs()
            ),
        }
    }
}

impl CollectorWait {
    fn new(now: Instant, stall: Duration) -> Self {
        Self {
            started: now,
            watch: StampWatch::with_stall(now, stall),
        }
    }

    fn ended(&mut self, now: Instant, stamp: CollectorStamp) -> Option<WaitEnd> {
        if self.watch.stalled(now, stamp) {
            Some(WaitEnd::Stalled(self.watch.stall))
        } else if now.duration_since(self.started) >= COLLECTOR_WAIT_CAP {
            Some(WaitEnd::Capped)
        } else {
            None
        }
    }
}

fn steps(text: &str) -> impl Iterator<Item = (u64, &str)> {
    text.lines().filter_map(|line| {
        let (millis, step) = line.split_once(' ')?;
        Some((millis.parse().ok()?, step))
    })
}

fn last_step(text: &str) -> Option<String> {
    let (millis, step) = steps(text).last()?;
    Some(format!("{step} at {:.1} s", millis as f64 / 1000.0))
}

fn steps_between(text: &str, from: &str, to: &str) -> Option<String> {
    let all: Vec<(u64, &str)> = steps(text).collect();
    let first = all.iter().rposition(|(_, step)| *step == from)?;
    let last = first + all[first..].iter().position(|(_, step)| *step == to)?;
    let span = &all[first..=last];
    let mut durations: Vec<(u64, &str)> = span
        .windows(2)
        .map(|pair| (pair[1].0.saturating_sub(pair[0].0), pair[1].1))
        .collect();
    durations.sort_by_key(|&(millis, _)| std::cmp::Reverse(millis));
    let slowest: Vec<String> = durations
        .iter()
        .take(5)
        .map(|(millis, step)| format!("{step} {:.1} s", *millis as f64 / 1000.0))
        .collect();
    Some(format!(
        "took {:.1} s; slowest steps: {}",
        (span[span.len() - 1].0 - span[0].0) as f64 / 1000.0,
        slowest.join(", ")
    ))
}

fn step_summary(directory: &Path, from: &str, to: &str) -> Option<String> {
    steps_between(
        &fs::read_to_string(directory.join(COLLECTOR_PROGRESS)).ok()?,
        from,
        to,
    )
}
#[cfg(test)]
#[derive(Default)]
struct ProgressClock {
    fixed: Option<std::rc::Rc<dyn Fn() -> Instant>>,
}

#[cfg(not(test))]
#[derive(Default)]
struct ProgressClock;

impl ProgressClock {
    #[inline]
    fn now(&self) -> Instant {
        self.fixed().unwrap_or_else(Instant::now)
    }

    #[cfg(test)]
    fn fixed(&self) -> Option<Instant> {
        self.fixed.as_ref().map(|clock| clock())
    }

    #[cfg(not(test))]
    #[inline]
    fn fixed(&self) -> Option<Instant> {
        None
    }
}

struct FinalizationProgress<'a> {
    directory: &'a Path,
    observe: &'a dyn Fn(u64),
    records: Cell<u64>,
    clock: ProgressClock,
    last_write: Cell<Instant>,
    warned: Cell<bool>,
    work_warned: Cell<bool>,
}
impl<'a> FinalizationProgress<'a> {
    fn new(directory: &'a Path, observe: &'a dyn Fn(u64)) -> Self {
        let progress = Self {
            directory,
            observe,
            records: Cell::new(0),
            clock: ProgressClock::default(),
            last_write: Cell::new(Instant::now()),
            warned: Cell::new(false),
            work_warned: Cell::new(false),
        };
        progress.publish();
        progress
    }
    fn advance(&self) {
        self.records.set(self.records.get().saturating_add(1));
        let now = self.clock.now();
        if now.duration_since(self.last_write.get()) >= Duration::from_secs(1) {
            self.last_write.set(now);
            self.publish();
            (self.observe)(self.records.get());
        }
    }
    fn publish(&self) {
        let work = self.directory.join("finalization-work");
        if let Err(error) = progress_io::work(&work, self.records.get())
            && !self.work_warned.replace(true)
        {
            eprintln!(
                "Finalization work signal unavailable at {}: {error}; the detach watch also follows recorder CPU and I/O.",
                work.display()
            );
        }
        let path = self.directory.join("finalization-progress.json");
        if let Err(error) = progress_io::publish(
            &path,
            &json!({"recorder_pid": std::process::id(), "records": self.records.get()}),
        ) && !self.warned.replace(true)
        {
            eprintln!(
                "Finalization progress unavailable at {}: {error}; replay finalization continues with the independent work signal.",
                path.display()
            );
        }
    }
}

const CONTENT_REFUSED: &str = "content verification failed";

fn collector_error(directory: &Path, error: &str) -> String {
    let error = error.trim();
    if error != CONTENT_REFUSED {
        return error.to_owned();
    }
    let content = read_status(&directory.join("content.json"));
    let Some(reason) = content
        .as_ref()
        .and_then(|content| content["verification"]["reason"].as_str())
    else {
        return format!("{CONTENT_REFUSED}; the reason could not be read");
    };
    let mut text = format!("{CONTENT_REFUSED}: {reason}");
    if let Some(difference) = content
        .as_ref()
        .map(|content| &content["mount_difference"])
        .filter(|difference| difference.is_object())
    {
        let folder = |key: &str| difference[key].as_str().unwrap_or("no folder").to_owned();
        let mount = difference["mount"]
            .as_u64()
            .map_or_else(|| "a mount".to_owned(), |mount| format!("mount {mount}"));
        text.push_str(&format!(
            "; {mount} {}: the game mounts {} and the session's content expects {}",
            difference["differs"].as_str().unwrap_or("differs"),
            folder("observed"),
            folder("expected"),
        ));
    }
    text
}

fn print_status(path: &Path) {
    if let Ok(bytes) = fs::read(path)
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        if let Some((site, reason)) = destruction_refusal(&value) {
            eprintln!("EquipmentDestruction refusal site={site}: {reason}");
        }
        if let Some(panic) = value["worker_panic"].as_str() {
            eprintln!("Collector worker panic: {panic}");
        }
        if let Some(error) = value["error"]
            .as_str()
            .or_else(|| value["last_error"].as_str())
            .or_else(|| value["warning"].as_str())
        {
            let directory = path.parent().unwrap_or(Path::new(""));
            eprintln!("Collector: {}", collector_error(directory, error));
        }
        if let Some(stalled) = value["dropped_spool_stalled"].as_u64().filter(|&n| n > 0) {
            eprintln!("Collector: {stalled} hours dropped: spool stalled");
        }
        if value["hours"].as_u64().unwrap_or(0) == 0 {
            println!(
                "{}; waiting for the first recorded game hour.",
                value["state"].as_str().unwrap_or("Attached")
            );
            return;
        }
        let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
        let frames = &value["acquisition_work"]["frames"];
        let behind = match (frames["in_flight"].as_u64(), frames["steady"].as_u64()) {
            (Some(in_flight), Some(steady)) if in_flight > steady => format!(
                ", falling behind by {in_flight} hours ({:.1} MB of frames)",
                frames["in_flight_bytes"].as_f64().unwrap_or(0.0) / 1e6
            ),
            _ => String::new(),
        };
        println!(
            "{}: {:.0} hours, last {:.3} ms, p95 {:.3} ms, {:.2} h/s, spool {:.1} MB, {:.0} dropped, {:.0} truncated{behind}",
            value["state"].as_str().unwrap_or("recording"),
            number("hours"),
            number("total_ns") / 1e6,
            number("p95_ns") / 1e6,
            number("hours_per_second"),
            number("spool_bytes") / 1e6,
            number("dropped_frames"),
            number("truncated_tables")
        );
    } else {
        println!("Attached; waiting for the first recorded game hour.");
    }
}

fn destruction_refusal(status: &Value) -> Option<(u64, &str)> {
    let refusal = &status["equipment_destruction"]["refusal"];
    Some((refusal["site_index"].as_u64()?, refusal["reason"].as_str()?))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn collector_present(process: &Process, dll: &Path, deadline: Instant) -> Result<bool, String> {
    if process.handle.signaled()? {
        return Ok(false);
    }
    match process.has_module_until(dll, deadline) {
        Ok(present) => Ok(present),
        Err(_) if process.handle.signaled()? => Ok(false),
        Err(error) => Err(error),
    }
}

#[derive(Clone, Copy)]
struct CollectorPresence {
    process_exited: bool,
    collector_loaded: Option<bool>,
}
impl CollectorPresence {
    fn exited() -> Self {
        Self {
            process_exited: true,
            collector_loaded: Some(false),
        }
    }
    fn wait(process: Option<&Process>, dll: &Path) -> Self {
        wait_collector_absence(
            |deadline| Self::observe_until(process, dll, deadline),
            Instant::now,
            std::thread::sleep,
        )
    }
    fn observe(process: Option<&Process>, dll: &Path) -> Self {
        Self::observe_until(process, dll, Instant::now() + UNLOAD_POLL)
    }
    fn observe_until(process: Option<&Process>, dll: &Path, deadline: Instant) -> Self {
        let Some(process) = process else {
            return Self::exited();
        };
        let collector_loaded = collector_present(process, dll, deadline).ok();
        if process.handle.signaled().unwrap_or(false) {
            Self::exited()
        } else {
            Self {
                process_exited: false,
                collector_loaded,
            }
        }
    }
}

const UNLOAD_WAIT: Duration = Duration::from_secs(10);
const UNLOAD_POLL: Duration = Duration::from_millis(100);

fn wait_collector_absence(
    mut refresh: impl FnMut(Instant) -> CollectorPresence,
    mut now: impl FnMut() -> Instant,
    mut sleep: impl FnMut(Duration),
) -> CollectorPresence {
    let deadline = now() + UNLOAD_WAIT;
    let mut presence = CollectorPresence {
        process_exited: false,
        collector_loaded: None,
    };
    loop {
        if now() >= deadline {
            return presence;
        }
        presence = refresh(deadline);
        if presence.process_exited || presence.collector_loaded == Some(false) {
            return presence;
        }
        let remaining = deadline.saturating_duration_since(now());
        if remaining.is_zero() {
            return presence;
        }
        sleep(remaining.min(UNLOAD_POLL));
    }
}

fn require_collector_absent(loaded: Option<bool>) -> Result<(), String> {
    if loaded == Some(false) {
        Ok(())
    } else {
        Err(COLLECTOR_LOADED.into())
    }
}

fn capture_result(
    directory: &Path,
    terminal: &TerminalOutcome,
    result: CaptureOutcome,
) -> Result<CaptureOutcome, String> {
    if terminal.issues.is_empty() {
        Ok(result)
    } else {
        Err(format!(
            "recording completion is partial: {}; session evidence retained at {}",
            terminal.issues.join("; "),
            directory.display()
        ))
    }
}

fn discarded_end(
    directory: &Path,
    target: &Path,
    terminal: &TerminalOutcome,
    requested_detach: bool,
    campaign: Option<&str>,
    result: CaptureOutcome,
) -> (Value, Result<CaptureOutcome, String>) {
    let mut completed = discarded_completion(terminal, requested_detach, campaign);
    let mut result = CaptureOutcome {
        discarded: true,
        ..result
    };
    if !terminal.issues.is_empty() {
        for issue in &terminal.issues {
            eprintln!("The discarded capture had failed: {issue}");
        }
        match keep_failure_report(directory, target, &completed) {
            Ok(report) => {
                println!("Failure report kept in {}.", report.display());
                completed["failure_report"] = json!(report);
                result.failure_report = Some(report);
            }
            Err(error) => {
                eprintln!("The failure report could not be kept: {error}");
                if let Some(warnings) = completed["warnings"].as_array_mut() {
                    warnings.push(json!(format!("the failure report could not be kept: {error}")));
                }
            }
        }
    }
    (completed, Ok(result))
}

pub const FAILURE_REPORTS: &str = "failure-reports";

const FAILURE_REPORTS_KEPT: usize = 5;

const DIAGNOSTICS: [&str; 8] = [
    "status.json",
    "bin/collector.error.txt",
    "collector-failure-detail.json",
    WITNESS_FILE_NAME,
    "recorder.log",
    COLLECTOR_PROGRESS,
    "content.json",
    "finalization-status.json",
];

fn keep_failure_report(
    directory: &Path,
    target: &Path,
    completed: &Value,
) -> Result<PathBuf, String> {
    let reports = target.join(FAILURE_REPORTS);
    let name = directory
        .file_name()
        .ok_or_else(|| failure_text(directory, "the session folder has no name"))?;
    let report = reports.join(name);
    fs::create_dir_all(&report).map_err(|error| failure_text(&report, error))?;
    for diagnostic in DIAGNOSTICS {
        let from = directory.join(diagnostic);
        let to = report.join(Path::new(diagnostic).file_name().unwrap_or(diagnostic.as_ref()));
        match fs::copy(&from, &to) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            copied => {
                copied.map_err(|error| failure_text(&from, error))?;
            }
        }
    }
    write_json(&report.join("completed.json"), completed)?;
    prune_failure_reports(&reports, FAILURE_REPORTS_KEPT)?;
    Ok(report)
}

fn prune_failure_reports(reports: &Path, kept: usize) -> Result<(), String> {
    let mut found = Vec::new();
    for entry in fs::read_dir(reports).map_err(|error| failure_text(reports, error))? {
        let entry = entry.map_err(|error| failure_text(reports, error))?;
        let path = entry.path();
        if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let written = fs::metadata(path.join("completed.json"))
            .or_else(|_| entry.metadata())
            .and_then(|metadata| metadata.modified())
            .map_err(|error| failure_text(&path, error))?;
        found.push((written, path));
    }
    found.sort();
    let excess = found.len().saturating_sub(kept);
    for (_, path) in found.into_iter().take(excess) {
        fs::remove_dir_all(&path).map_err(|error| failure_text(&path, error))?;
    }
    Ok(())
}

fn discarded_completion(
    terminal: &TerminalOutcome,
    requested_detach: bool,
    campaign: Option<&str>,
) -> Value {
    json!({
        "replays":[],"discarded":true,"discarded_campaign":campaign,
        "detach_requested":requested_detach,
        "cleanup_pending":true,
        "drain_complete":terminal.drain_complete,"capture_failed":terminal.capture_failed,
        "failure_code":terminal.witness.as_ref().map(|_| "allocation_failed"),
        "failed_hour":terminal.witness.as_ref().and_then(|witness| witness.failed_hour),
        "partial":!terminal.issues.is_empty(),"issues":terminal.issues,"warnings":terminal.warnings,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_capture(
    directory: &Path,
    target: &Path,
    active: &Path,
    session: &SessionStart,
    dll: &Path,
    process_exited: bool,
    mut completed: Value,
    result: Result<CaptureOutcome, String>,
    stage: &dyn Fn(Stage<'_>),
    diagnostics: impl FnOnce() -> Option<Value>,
) -> Result<CaptureOutcome, String> {
    HistoryRequest::retire_session(directory, false, process_exited);
    note_restarts(directory, &mut completed);
    {
        let _lock = completion_lock(directory)?;
        completion_presence(
            directory,
            &mut completed,
            CollectorPresence {
                process_exited,
                collector_loaded: process_exited.then_some(false),
            },
            !process_exited,
        );
        replay_io::write_session_end(directory, target, "completed.json", &completed)?;
    }
    let _ = fs::remove_file(directory.join(WAITING_FILE));
    remove_active(active, directory)?;
    stage(Stage::Finished {
        result: result.as_ref().map_err(String::as_str),
    });
    if let Some(diagnostics) = diagnostics() {
        println!("Stop to save-ready: {diagnostics}");
        if let Err(error) = write_json(&directory.join("finalization-status.json"), &diagnostics) {
            eprintln!("Finalization diagnostics unavailable: {error}");
        }
    }
    stage(Stage::Unloading(UnloadObservation {
        directory: directory.to_path_buf(),
        dll: dll.to_path_buf(),
        pid: session.process_id,
        process_start: session.process_start_time,
        process_exited,
    }));
    result
}

fn note_restarts(directory: &Path, completed: &mut Value) {
    if let Some(restarts) = read_status(&directory.join(replay_io::RESTARTS_FILE)) {
        completed["recorder_restarts"] = restarts;
    }
}

#[derive(Debug)]
pub struct UnloadObservation {
    directory: PathBuf,
    dll: PathBuf,
    pid: u32,
    process_start: u64,
    process_exited: bool,
}

impl UnloadObservation {
    pub fn finish(self) -> Result<(), String> {
        let presence = self.observe();
        self.finish_with(presence)
    }

    fn observe(&self) -> CollectorPresence {
        if self.process_exited {
            CollectorPresence::exited()
        } else {
            wait_collector_absence(
                |deadline| match Process::open(self.pid, false) {
                    Ok(process) if process.start_time != self.process_start => {
                        CollectorPresence::exited()
                    }
                    Ok(process) => {
                        CollectorPresence::observe_until(Some(&process), &self.dll, deadline)
                    }
                    Err(_)
                        if windows::game_present(self.pid, self.process_start)
                            .is_ok_and(|present| !present) =>
                    {
                        CollectorPresence::exited()
                    }
                    Err(_) => CollectorPresence {
                        process_exited: false,
                        collector_loaded: None,
                    },
                },
                Instant::now,
                std::thread::sleep,
            )
        }
    }

    fn finish_with(self, presence: CollectorPresence) -> Result<(), String> {
        let discarded = refresh_unload_completion(&self.directory, presence)?;
        if presence.collector_loaded != Some(false) {
            return Err(COLLECTOR_UNLOAD_WARNING.into());
        }
        if discarded {
            discard_session(&self.directory)?;
        }
        Ok(())
    }
}

fn refresh_unload_completion(
    directory: &Path,
    presence: CollectorPresence,
) -> Result<bool, String> {
    HistoryRequest::retire_session(
        directory,
        presence.collector_loaded == Some(false),
        presence.process_exited,
    );
    let _lock = completion_lock(directory)?;
    let mut completed = read_status(&directory.join("completed.json"))
        .ok_or("saved session has no readable completion record")?;
    completion_presence(directory, &mut completed, presence, false);
    write_json(&directory.join("completed.json"), &completed)?;
    Ok(completed["discarded"] == true)
}

const COLLECTOR_RESIDENT: &str = "bin/collector.resident.txt";

fn completion_presence(
    directory: &Path,
    completed: &mut Value,
    presence: CollectorPresence,
    pending: bool,
) {
    if !completed["warnings"].is_array() {
        completed["warnings"] = json!([]);
    }
    completed["warnings"].as_array_mut().unwrap().retain(|warning| {
        warning.as_str().is_none_or(|warning| {
            warning != COLLECTOR_UNLOAD_WARNING && !warning.starts_with("collector stays loaded")
        })
    });
    completed["detached"] =
        json!(!presence.process_exited && presence.collector_loaded == Some(false));
    completed["collector_loaded"] = json!(presence.collector_loaded);
    completed["process_exited"] = json!(presence.process_exited);
    completed["unload_pending"] = json!(pending);
    if !pending && presence.collector_loaded != Some(false) {
        let warning = match fs::read_to_string(directory.join(COLLECTOR_RESIDENT)) {
            Ok(resident) => format!("{}; quit the game before recording again", resident.trim()),
            Err(_) => COLLECTOR_UNLOAD_WARNING.into(),
        };
        completed["warnings"].as_array_mut().unwrap().push(json!(warning));
    }
}

#[cfg(test)]
fn write_completion(
    directory: &Path,
    target: &Path,
    completed: Value,
    refresh: impl FnOnce() -> CollectorPresence,
) -> Result<CollectorPresence, String> {
    let presence = refresh();
    let _lock = completion_lock(directory)?;
    publish_completion(directory, target, completed, presence)
}

fn publish_completion(
    directory: &Path,
    target: &Path,
    mut completed: Value,
    presence: CollectorPresence,
) -> Result<CollectorPresence, String> {
    HistoryRequest::retire_session(
        directory,
        presence.collector_loaded == Some(false),
        presence.process_exited,
    );
    completion_presence(directory, &mut completed, presence, false);
    replay_io::write_session_end(directory, target, "completed.json", &completed)?;
    if presence.collector_loaded != Some(false) {
        eprintln!("Collector warning: {COLLECTOR_UNLOAD_WARNING}");
    }
    Ok(presence)
}

fn collector_config(directory: &Path) -> Result<(Value, SessionStart), String> {
    let config: Value = serde_json::from_slice(
        &fs::read(directory.join("bin/collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bytes: Vec<u8> =
        serde_json::from_value(config["session_start"].clone()).map_err(|e| e.to_string())?;
    let mut decoder = replay_data::codec::Decoder::new(&bytes);
    let session = SessionStart::decode(&mut decoder).map_err(|e| e.to_string())?;
    decoder.finish().map_err(|e| e.to_string())?;
    Ok((config, session))
}

fn ensure_finalizable(directory: &Path) -> Result<(Option<Process>, SessionStart), String> {
    let (config, session) = collector_config(directory)?;
    if !windows::game_present(session.process_id, session.process_start_time)? {
        HistoryRequest::retire_session(directory, false, true);
        return Ok((None, session));
    }
    let process = match Process::open(session.process_id, false) {
        Ok(process) => process,
        Err(_) if !windows::game_present(session.process_id, session.process_start_time)? => {
            HistoryRequest::retire_session(directory, false, true);
            return Ok((None, session));
        }
        Err(error) => return Err(error),
    };
    if process.start_time != session.process_start_time {
        HistoryRequest::retire_session(directory, false, true);
        return Ok((None, session));
    }
    let done = config["done_event"]
        .as_str()
        .ok_or("collector configuration has no done event")?;
    let done = Event::open(done)
        .and_then(|event| event.handle.signaled())
        .unwrap_or(false);
    let collector_loaded = match collector_present(
        &process,
        &directory.join("bin/collector.dll"),
        Instant::now() + UNLOAD_POLL,
    ) {
        Ok(loaded) => Some(loaded),
        Err(_) if done => None,
        Err(error) => return Err(error),
    };
    if collector_loaded != Some(false) && !done {
        return Err("collector is still loaded without done; its spool may still change; use recorder detach or quit the game before finalizing".into());
    }
    let process_exited = process.handle.signaled()?;
    HistoryRequest::retire_session(directory, collector_loaded == Some(false), process_exited);
    if !process_exited && !done {
        if !directory.join("completed.json").is_file()
            && !replay_io::read_failure_witness(directory)?.is_some_and(|witness| {
                witness.session == session.id
                    && witness.process_start_time == session.process_start_time
            })
        {
            return Err(
                "live process has no completed detach witness; wait for its recorder or game exit"
                    .into(),
            );
        }
    }
    Ok((Some(process), session))
}

fn finalization_lock(output: &Path) -> Result<File, String> {
    exclusive_file(&output.join("finalize.lock"))
}

fn completion_lock(directory: &Path) -> Result<File, String> {
    exclusive_file(&directory.join("completion.lock"))
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CompletionStep {
    Read,
    Blocked,
}

#[cfg(test)]
thread_local! {
    static COMPLETION_STEP: std::cell::RefCell<Option<Box<dyn Fn(&Path, CompletionStep)>>> = const {
        std::cell::RefCell::new(None)
    };
}

#[cfg(test)]
fn completion_step(path: &Path, step: CompletionStep) {
    COMPLETION_STEP.with(|hook| {
        if let Some(hook) = hook.borrow().as_ref() {
            hook(path, step);
        }
    });
}

fn try_exclusive_file(path: &Path) -> Result<Option<File>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    match fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .share_mode(0)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.raw_os_error() == Some(32) => Ok(None),
        Err(error) => Err(failure_text(path, error)),
    }
}

fn exclusive_file(path: &Path) -> Result<File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .share_mode(0)
            .open(&path)
        {
            Ok(file) => return Ok(file),
            Err(error) if error.raw_os_error() == Some(32) && Instant::now() < deadline => {
                #[cfg(test)]
                completion_step(path, CompletionStep::Blocked);
                std::thread::sleep(Duration::from_millis(100))
            }
            Err(error) => {
                return Err(format!(
                    "output is busy recording/finalizing or unavailable: {}: {error}",
                    path.display()
                ));
            }
        }
    }
}

fn waiting_json(campaign: &str, unsaved: Option<&replay_io::Unsaved>) -> Value {
    json!({
        "campaign": campaign,
        "hours": unsaved.map(|unsaved| unsaved.hours),
        "first_hour": unsaved.and_then(|unsaved| unsaved.first_hour),
        "last_hour": unsaved.and_then(|unsaved| unsaved.last_hour),
    })
}

fn waits(target: &Path, campaign: &str) -> Option<Value> {
    match replay_io::unsaved(target, campaign) {
        Ok(unsaved) => unsaved.map(|unsaved| waiting_json(campaign, Some(&unsaved))),
        Err(error) => {
            eprintln!("Campaign {campaign} waits to be saved; its hours could not be read: {error}");
            Some(waiting_json(campaign, None))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingCampaign {
    pub campaign: String,
    pub target: PathBuf,
    pub hours: Option<u64>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

pub fn session_folders(output: &Path) -> Vec<PathBuf> {
    fs::read_dir(output.join("sessions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

pub fn end_abandoned_sessions(output: &Path) -> AbandonedPass {
    let mut pass = AbandonedPass::default();
    for session in session_folders(output) {
        match end_abandoned(output, &session) {
            Ok(Abandoned::Settled) => {}
            Ok(Abandoned::Busy) => pass.retry = true,
            Ok(Abandoned::Refused(why)) => pass.errors.push(failure_text(&session, why)),
            Err(error) => {
                pass.retry = true;
                pass.errors.push(failure_text(&session, error));
            }
        }
    }
    pass
}

#[derive(Debug, Default)]
pub struct AbandonedPass {
    pub errors: Vec<String>,
    pub retry: bool,
}

enum Abandoned {
    Settled,
    Busy,
    Refused(String),
}

fn end_abandoned(output: &Path, directory: &Path) -> Result<Abandoned, String> {
    let completed = directory.join("completed.json");
    let record = completed
        .is_file()
        .then(|| read_status(&completed))
        .flatten();
    let flag = |key: &str| record.as_ref().is_some_and(|record| record[key] == true);
    let (unload_pending, cleanup_pending) = (flag("unload_pending"), flag("cleanup_pending"));
    if cleanup_pending
        && !unload_pending
        && record
            .as_ref()
            .is_some_and(|record| record["collector_loaded"] == false)
    {
        return discard_session(directory).map(|()| Abandoned::Settled);
    }
    let recovering = recovery_listed(directory);
    if !unload_pending && !cleanup_pending && !recovering && session_ended(directory) {
        return Ok(Abandoned::Settled);
    }
    if !directory.join("bin/collector.json").is_file() {
        return Ok(Abandoned::Settled);
    }
    let (config, session) = collector_config(directory)?;
    let Some(_claim) =
        windows::Claim::take(&game_claim(session.process_id, session.process_start_time))?
    else {
        return Ok(Abandoned::Settled);
    };
    let Some(presence) = collector_gone(directory, &session)? else {
        return Ok(Abandoned::Settled);
    };
    if unload_pending || cleanup_pending {
        return UnloadObservation {
            directory: directory.to_path_buf(),
            dll: directory.join("bin/collector.dll"),
            pid: session.process_id,
            process_start: session.process_start_time,
            process_exited: presence.process_exited,
        }
        .finish_with(presence)
        .map(|()| Abandoned::Settled);
    }
    end_session(output, directory, &config, &session, presence, None)
}

fn end_session(
    output: &Path,
    directory: &Path,
    config: &Value,
    session: &SessionStart,
    presence: CollectorPresence,
    restart: Option<&str>,
) -> Result<Abandoned, String> {
    let target = PathBuf::from(
        config["replay_dir"]
            .as_str()
            .ok_or("collector configuration has no recording folder")?,
    );
    fs::create_dir_all(&target).map_err(|error| failure_text(&target, error))?;
    let Some(_lock) = try_exclusive_file(&target.join("finalize.lock"))? else {
        return Ok(Abandoned::Busy);
    };
    let _completion = completion_lock(directory)?;
    let recovering = recovery_listed(directory);
    if !recovering && session_ended(directory) {
        return Ok(Abandoned::Settled);
    }
    let terminal = terminal_outcome(
        directory,
        session.id,
        session.process_start_time,
        restart.is_some(),
        presence.process_exited,
        true,
    );
    if !terminal.recovery_allowed {
        return Ok(Abandoned::Refused(format!(
            "{}; recover it with recorder finalize",
            terminal.issues.join("; ")
        )));
    }
    if let Some(witness) = &terminal.witness {
        publish_failure_status(directory, witness)?;
    }
    let spooled = replay_io::written_campaigns(directory)?;
    let (listed, discarded) = if recovering {
        let record = read_status(&directory.join(WAITING_FILE))
            .ok_or_else(|| failure_text(&directory.join(WAITING_FILE), "unreadable"))?;
        let listed = record["waiting"]
            .as_array()
            .cloned()
            .ok_or_else(|| failure_text(&directory.join(WAITING_FILE), "unreadable"))?;
        (listed, record["discarded_campaign"].as_str().map(str::to_owned))
    } else {
        let admissions = replay_io::session_admissions(directory)?;
        let noted = replay_io::noted_discards(directory)?;
        let listed: Vec<Value> = spooled
            .iter()
            .filter(|campaign| publishes(admissions.as_ref(), campaign) && !noted.contains(campaign))
            .map(|campaign| waiting_json(campaign, None))
            .collect();
        let discarded = noted
            .into_iter()
            .find(|campaign| publishes(admissions.as_ref(), campaign));
        replay_io::write_session_end(
            directory,
            &target,
            WAITING_FILE,
            &json!({
                "target":target,"waiting":listed,"recovered":true,"discarded_campaign":discarded,
            }),
        )?;
        (listed, discarded)
    };
    let listed: BTreeSet<&str> = listed
        .iter()
        .filter_map(|entry| entry["campaign"].as_str())
        .collect();
    let mut waiting = Vec::new();
    if !listed.is_empty() {
        let settings = RecorderSettings::path();
        let finalized = replay_io::finalize_session_ahead(
            directory,
            &target,
            &|| Ok(()),
            replay_io::Live::default(),
            replay_io::Naming {
                localisation: None,
                settings: &settings,
                report: None,
            },
            &|campaign| {
                if listed.contains(campaign) {
                    replay_io::Destination::Unsaved
                } else {
                    replay_io::Destination::Neither
                }
            },
        )?;
        if !finalized.errors.is_empty() {
            return Err(finalized.errors.join("; "));
        }
        for campaign in &listed {
            waiting.extend(waits(&target, campaign));
        }
    }
    let mut completed = json!({
        "replays":[],"target":target,"waiting":waiting,"recovered":true,
        "discarded_campaign":discarded,
        "nothing_recorded":spooled.is_empty(),"detach_requested":restart.is_some(),
        "drain_complete":terminal.drain_complete,"capture_failed":terminal.capture_failed,
        "failure_code":terminal.witness.as_ref().map(|_| "allocation_failed"),
        "failed_hour":terminal.witness.as_ref().and_then(|witness| witness.failed_hour),
        "partial":!terminal.issues.is_empty(),"issues":terminal.issues,
        "warnings":terminal.warnings,"recorder_restart":restart,
    });
    note_restarts(directory, &mut completed);
    publish_completion(directory, &target, completed, presence)?;
    let _ = fs::remove_file(directory.join(WAITING_FILE));
    remove_active(&output.join("active.json"), directory)?;
    Ok(Abandoned::Settled)
}

fn recovery_listed(directory: &Path) -> bool {
    !directory.join("completed.json").is_file()
        && read_status(&directory.join(WAITING_FILE)).is_some_and(|listed| listed["recovered"] == true)
}

fn session_ended(directory: &Path) -> bool {
    ["completed.json", WAITING_FILE]
        .iter()
        .any(|name| directory.join(name).is_file())
}

fn collector_gone(
    directory: &Path,
    session: &SessionStart,
) -> Result<Option<CollectorPresence>, String> {
    if !game_alive(session)? {
        return Ok(Some(CollectorPresence::exited()));
    }
    let Ok(process) = Process::open(session.process_id, false) else {
        return Ok(None);
    };
    let presence =
        CollectorPresence::observe(Some(&process), &directory.join("bin").join("collector.dll"));
    Ok((presence.collector_loaded == Some(false)).then_some(presence))
}

fn game_alive(session: &SessionStart) -> Result<bool, String> {
    if !windows::game_present(session.process_id, session.process_start_time)? {
        return Ok(false);
    }
    match Process::open(session.process_id, false) {
        Ok(process) => {
            Ok(process.start_time == session.process_start_time && !process.handle.signaled()?)
        }
        Err(_) => windows::game_present(session.process_id, session.process_start_time),
    }
}

const WAITING_FILE: &str = "waiting.json";

fn waiting_record(session: &Path) -> PathBuf {
    let completed = session.join("completed.json");
    if completed.exists() {
        completed
    } else {
        session.join(WAITING_FILE)
    }
}

pub fn waiting_campaigns(session: &Path) -> Vec<WaitingCampaign> {
    let Some(completed) = read_status(&waiting_record(session)) else {
        return Vec::new();
    };
    let Some(target) = completed["target"].as_str().map(PathBuf::from) else {
        return Vec::new();
    };
    let hour = |value: &Value| value.as_i64().and_then(|hour| i32::try_from(hour).ok());
    completed["waiting"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            Some(WaitingCampaign {
                campaign: entry["campaign"].as_str()?.to_owned(),
                target: target.clone(),
                hours: entry["hours"].as_u64(),
                first_hour: hour(&entry["first_hour"]),
                last_hour: hour(&entry["last_hour"]),
            })
        })
        .collect()
}

fn unlist_waiting(session: &Path, campaign: &str) -> Result<(), String> {
    let _lock = completion_lock(session)?;
    let path = waiting_record(session);
    let mut completed = read_status(&path).ok_or_else(|| failure_text(&path, "unreadable"))?;
    if let Some(waiting) = completed["waiting"].as_array_mut() {
        waiting.retain(|entry| entry["campaign"] != campaign);
    }
    write_json(&path, &completed)
}

fn waiting_target(session: &Path, campaign: &str) -> Result<PathBuf, String> {
    waiting_campaigns(session)
        .into_iter()
        .find(|waiting| waiting.campaign == campaign)
        .map(|waiting| waiting.target)
        .ok_or_else(|| {
            format!(
                "{campaign} is not waiting to be saved in {}",
                session.display()
            )
        })
}

pub fn waiting_folder(session: &Path, campaign: &str) -> Result<PathBuf, String> {
    waiting_target(session, campaign)
}

fn waiting_sessions(root: &Path, target: &Path, campaign: &str) -> Vec<PathBuf> {
    let mut sessions: Vec<_> = session_folders(root)
        .into_iter()
        .filter(|session| {
            waiting_campaigns(session)
                .iter()
                .any(|waiting| waiting.campaign == campaign && waiting.target == target)
        })
        .collect();
    sessions.sort();
    sessions
}

fn save_recording(
    root: &Path,
    target: &Path,
    campaign: &str,
    naming: replay_io::Naming<'_>,
) -> Result<Option<PathBuf>, String> {
    replay_io::with_packing(|| {
        let saved = replay_io::save_waiting(target, campaign, naming)?;
        unlist_waiting_sessions(root, target, campaign, None)?;
        Ok(saved)
    })
}

fn unlist_waiting_sessions(
    root: &Path,
    target: &Path,
    campaign: &str,
    current: Option<&Path>,
) -> Result<(), String> {
    for session in waiting_sessions(root, target, campaign)
        .into_iter()
        .filter(|session| Some(session.as_path()) != current)
    {
        unlist_waiting(&session, campaign)?;
    }
    Ok(())
}

fn publishes(admitted: Option<&replay_io::Admissions>, campaign: &str) -> bool {
    admitted.is_none_or(|admitted| admitted.loads.contains_key(campaign))
}

fn save_continued(
    root: &Path,
    target: &Path,
    written: &[String],
    admitted: Option<&replay_io::Admissions>,
    saves: &dyn Fn(&str) -> bool,
    naming: replay_io::Naming<'_>,
) -> (Vec<PathBuf>, BTreeMap<String, String>) {
    let (mut continued, mut blocked) = (Vec::new(), BTreeMap::new());
    for campaign in admitted
        .into_iter()
        .flat_map(|admitted| admitted.loads.keys())
        .filter(|campaign| !written.contains(campaign) && saves(campaign))
    {
        match save_recording(root, target, campaign, naming) {
            Ok(saved) => continued.extend(saved),
            Err(error) => {
                blocked.insert(campaign.clone(), error);
            }
        }
    }
    (continued, blocked)
}

fn discard_recording(root: &Path, target: &Path, campaign: &str) -> Result<(), String> {
    replay_io::discard_waiting(target, campaign)?;
    unlist_waiting_sessions(root, target, campaign, None)
}

fn root_of(session: &Path) -> Result<&Path, String> {
    session
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| failure_text(session, "not in a capture root"))
}

fn installed_content() -> Result<content_reader_layers::ResolveOptions, String> {
    let settings = RecorderSettings::load()
        .map_err(|error| format!("the recorder settings could not be read: {error}"))?;
    let user_dir = match settings.user_folder {
        Some(folder) => folder,
        None => resolve_user_directory()?,
    };
    let game_dir = crate::directories::default_game_directory()
        .map_err(|_| "Steam lists no the game installation".to_owned())?;
    Ok(content_reader_layers::ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: replay_settings::recorder_settings::directory(),
    })
}

type InstallationKey = Result<(PathBuf, PathBuf), String>;
type SettingsStamp = Option<(SystemTime, u64)>;
type InstalledContent = Result<content_reader_layers::ResolveOptions, String>;

const NAMING_RETRY: Duration = Duration::from_secs(30);
const NAMING_RETRY_MAX: Duration = Duration::from_secs(600);

static INSTALLED_CONTENT: Mutex<Option<(SettingsStamp, Instant, InstalledContent)>> =
    Mutex::new(None);

fn cached_installed_content() -> InstalledContent {
    let stamp = fs::metadata(RecorderSettings::path())
        .ok()
        .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())));
    let now = Instant::now();
    let mut cached = locked(&INSTALLED_CONTENT);
    if let Some((known, checked, content)) = &*cached
        && *known == stamp
        && now.saturating_duration_since(*checked) < NAMING_RETRY
    {
        return content.clone();
    }
    let content = installed_content();
    *cached = Some((stamp, now, content.clone()));
    content
}

struct NamingStack {
    identity: replay_data::identity::Identity,
    installation: InstallationKey,
    resolved: Option<content_reader_layers::ResolvedContent>,
    localisation: Option<Result<Localisation, String>>,
    retry: Option<(Instant, Duration)>,
    #[cfg(test)]
    resolutions: usize,
    #[cfg(test)]
    reads: usize,
}

impl NamingStack {
    fn load(&mut self, content: &InstalledContent, now: Instant) {
        if self.localisation.as_ref().is_some_and(Result::is_ok)
            || self
                .retry
                .is_some_and(|(checked, delay)| now.saturating_duration_since(checked) < delay)
        {
            return;
        }
        let loaded = (|| {
            if self.resolved.is_none() {
                #[cfg(test)]
                {
                    self.resolutions += 1;
                }
                let options = content.as_ref().map_err(String::clone)?;
                let resolved = content_reader_layers::resolve(&self.identity, options)
                    .and_then(content_reader_layers::ResolvedContent::exact)
                    .map_err(|error| error.to_string())?;
                self.resolved = Some(resolved);
            }
            #[cfg(test)]
            {
                self.reads += 1;
            }
            content_reader_localisation::localisation::read(
                &self.resolved.as_ref().unwrap().layers,
                &self.identity.language,
            )
            .map_err(|error| error.to_string())
        })();
        self.retry = loaded.is_err().then(|| {
            (
                now,
                self.retry
                    .map_or(NAMING_RETRY, |(_, delay)| (delay * 2).min(NAMING_RETRY_MAX)),
            )
        });
        self.localisation = Some(loaded);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct WaitingName {
    pub name: Option<String>,
    pub retry: bool,
}

pub struct WaitingNames {
    content: InstalledContent,
    installed: bool,
    stacks: Vec<NamingStack>,
}

impl WaitingNames {
    pub fn installed() -> Self {
        Self {
            installed: true,
            ..Self::new(cached_installed_content())
        }
    }

    pub fn installation_key() -> InstallationKey {
        cached_installed_content().map(|content| (content.game_dir, content.user_dir))
    }

    pub fn new(content: InstalledContent) -> Self {
        Self {
            content,
            installed: false,
            stacks: Vec::new(),
        }
    }

    pub fn predictions(&mut self, waiting: &[(PathBuf, String)]) -> Vec<WaitingName> {
        background_naming(|| {
            if self.installed {
                self.content = cached_installed_content();
            }
            let installation = self
                .content
                .as_ref()
                .map(|content| (content.game_dir.clone(), content.user_dir.clone()))
                .map_err(String::clone);
            let identities: Vec<_> = waiting
                .iter()
                .filter_map(|(session, _)| crate::controller::session_identity(session).ok())
                .collect();
            self.stacks.retain(|stack| {
                stack.installation == installation
                    && identities
                        .iter()
                        .any(|identity| stack.identity.same_content(identity))
            });
            waiting
                .iter()
                .map(|(session, campaign)| self.prediction(session, campaign))
                .collect()
        })
    }

    fn localisation(&mut self, session: &Path) -> Result<&Localisation, String> {
        if self.installed {
            self.content = cached_installed_content();
        }
        let identity = crate::controller::session_identity(session).map_err(|error| {
            format!(
                "{}: its content stack could not be read: {error}",
                session.display()
            )
        })?;
        let installation = self
            .content
            .as_ref()
            .map(|content| (content.game_dir.clone(), content.user_dir.clone()))
            .map_err(String::clone);
        let at = match self.stacks.iter().position(|known| {
            known.identity.same_content(&identity) && known.installation == installation
        }) {
            Some(at) => at,
            None => {
                self.stacks.push(NamingStack {
                    identity,
                    installation,
                    resolved: None,
                    localisation: None,
                    retry: None,
                    #[cfg(test)]
                    resolutions: 0,
                    #[cfg(test)]
                    reads: 0,
                });
                self.stacks.len() - 1
            }
        };
        let stack = &mut self.stacks[at];
        stack.load(&self.content, Instant::now());
        stack
            .localisation
            .as_ref()
            .unwrap()
            .as_ref()
            .map_err(String::clone)
    }

    pub fn name(&mut self, session: &Path, campaign: &str) -> Option<String> {
        self.prediction(session, campaign).name
    }

    pub fn prediction(&mut self, session: &Path, campaign: &str) -> WaitingName {
        let mut retry = true;
        let name = (|| {
            let target = waiting_target(session, campaign).ok()?;
            let settings = RecorderSettings::path();
            let localisation = self.localisation(session);
            let by_tag = AtomicBool::new(false);
            let report = |why: Option<&str>| by_tag.store(why.is_some(), Ordering::Relaxed);
            let naming = replay_io::Naming {
                localisation: localisation.as_ref().ok().copied(),
                settings: &settings,
                report: Some(&report),
            };
            let name = replay_io::waiting_name(&target, campaign, naming)?;
            retry = localisation.is_err() || by_tag.load(Ordering::Relaxed);
            Some(name)
        })();
        WaitingName { name, retry }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct SavedWaiting {
    pub recording: Option<PathBuf>,
    pub by_tag: Option<String>,
}

pub struct WaitingLock {
    session: PathBuf,
    campaign: String,
    target: PathBuf,
    _lock: File,
}

pub fn lock_waiting(session: &Path, campaign: &str) -> Result<WaitingLock, String> {
    let target = waiting_target(session, campaign)?;
    fs::create_dir_all(&target).map_err(|error| failure_text(&target, error))?;
    let lock = finalization_lock(&target)?;
    if waiting_target(session, campaign)? != target {
        return Err(format!("{campaign} moved while BattlePlan Recorder waited to act on it; choose again"));
    }
    Ok(WaitingLock { session: session.to_owned(), campaign: campaign.to_owned(), target, _lock: lock })
}

impl WaitingLock {
    pub fn save(&self) -> Result<SavedWaiting, String> {
        save_waiting_with(self, &mut WaitingNames::installed())
    }

    pub fn discard(&self) -> Result<(), String> {
        discard_recording(root_of(&self.session)?, &self.target, &self.campaign)
    }
}

fn save_waiting_with(locked: &WaitingLock, names: &mut WaitingNames) -> Result<SavedWaiting, String> {
    let (session, campaign, target) = (&locked.session, locked.campaign.as_str(), &locked.target);
    replay_io::with_packing(|| {
        let localisation = names.localisation(session);
        if let Err(why) = &localisation {
            eprintln!("Countries are named by tag: {why}");
        }
        let settings = RecorderSettings::path();
        let refusal = Mutex::new(None::<String>);
        let report = |why: Option<&str>| {
            *refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = why.map(str::to_owned);
        };
        let naming = replay_io::Naming {
            localisation: localisation.as_ref().ok().copied(),
            settings: &settings,
            report: Some(&report),
        };
        let recording = save_recording(root_of(session)?, target, campaign, naming)?;
        let by_tag = localisation.err().or_else(|| {
            refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        Ok(SavedWaiting { recording, by_tag })
    })
}


#[cfg(test)]
mod tests {
    #[test]
    fn failed_cli_timestamp_never_blocks_detach_or_invents_the_controller_poll_time() {
        let root = crate::test_support::allocation_test_directory("stop-timestamp-failure");
        fs::create_dir(root.join("stop.request")).unwrap();
        let signalled = Cell::new(0);
        for result in [Ok(()), Err("detach event unavailable".to_owned())] {
            let expected = result.clone();
            assert_eq!(
                signal_detach_request(&root, Some(42), || {
                    signalled.set(signalled.get() + 1);
                    result
                }),
                expected
            );
        }
        assert_eq!(signalled.get(), 2);
        let control = Control::new();
        assert_eq!(requested_stop_counter(&control, &root), None);
        assert_eq!(
            counter_seconds(requested_stop_counter(&control, &root), 10_000_000),
            None
        );
        control.stop_qpc.store(84, Ordering::Release);
        assert_eq!(requested_stop_counter(&control, &root), Some(84));
    }

    #[test]
    fn cli_timestamp_and_missing_counter_are_diagnostics_only() {
        let root = crate::test_support::allocation_test_directory("stop-timestamp-success");
        let control = Control::new();
        let signalled = Cell::new(false);
        signal_detach_request(&root, None, || {
            signalled.set(true);
            Ok(())
        })
        .unwrap();
        assert!(signalled.get());
        assert_eq!(requested_stop_counter(&control, &root), None);
        signal_detach_request(&root, Some(42), || Ok(())).unwrap();
        assert_eq!(requested_stop_counter(&control, &root), Some(42));
    }

    #[test]
    fn a_failed_explicit_retake_leaves_the_drop_guard_armed() {
        let attempts = Cell::new(0);
        let result = with_lent_history(
            |action| {
                if action == "retake" {
                    attempts.set(attempts.get() + 1);
                    if attempts.get() == 1 {
                        return Err("return publication failed".to_owned());
                    }
                }
                Ok(())
            },
            || Ok(()),
        );
        assert!(result.unwrap_err().contains("return publication failed"));
        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn a_failed_retake_is_reported_even_when_the_lend_also_failed() {
        for lent in [false, true] {
            let result = with_lent_history(
                |action| match action {
                    "lend" if lent => Ok(()),
                    "lend" => Err("lost lend ack".to_owned()),
                    _ => Err("retake deadline passed".to_owned()),
                },
                || Ok(()),
            );
            let error = result.unwrap_err();
            assert!(error.contains("retake deadline passed"));
            if !lent {
                assert!(error.contains("lost lend ack"));
            }
        }
    }

    use super::*;
    use replay::campaign::Replay;
    use std::{cell::RefCell, collections::BTreeMap, ffi::OsStr};

    #[test]
    fn a_dropped_content_scan_is_stopped_and_has_ended() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-content-scan-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (game_dir, user_dir) = (root.join("game"), root.join("user"));
        fs::create_dir_all(game_dir.join("common")).unwrap();
        fs::create_dir_all(game_dir.join("dlc")).unwrap();
        fs::create_dir_all(&user_dir).unwrap();
        fs::write(game_dir.join("hoi4.exe"), b"synthetic executable").unwrap();
        for index in 0..2_000 {
            fs::write(
                game_dir.join(format!("common/f{index:04}.txt")),
                format!("file {index}"),
            )
            .unwrap();
        }
        fs::write(user_dir.join("settings.txt"), b"language=l_english").unwrap();
        fs::write(
            user_dir.join("dlc_load.json"),
            br#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
        )
        .unwrap();
        let cache_dir = root.join("cache");
        let scan = ContentScan::start(GatherOptions {
            game_dir,
            user_dir,
            cache_dir: cache_dir.clone(),
            dlc_order: None,
        });
        let cancelled = Arc::clone(&scan.cancelled);
        drop(scan);
        assert!(cancelled.load(Ordering::Acquire));
        let written = fs::read_dir(&cache_dir)
            .unwrap()
            .any(|entry| entry.unwrap().file_type().unwrap().is_file());
        assert!(written);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn one_recorder_at_a_time_holds_a_game_and_a_reused_pid_is_another_game() {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        let pid = std::process::id();
        let held = windows::Claim::take(&game_claim(pid, start))
            .unwrap()
            .unwrap();
        assert!(
            windows::Claim::take(&game_claim(pid, start))
                .unwrap()
                .is_none()
        );
        assert!(
            windows::Claim::take(&game_claim(pid, start + 1))
                .unwrap()
                .is_some()
        );
        drop(held);
        assert!(
            windows::Claim::take(&game_claim(pid, start))
                .unwrap()
                .is_some()
        );
    }

    fn orphan_game() -> (u32, u64) {
        let start = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64;
        (std::process::id(), start)
    }

    fn orphaned_session(root: &Path, pid: u32, start: u64) -> (PathBuf, PathBuf, PathBuf, Events) {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, write_campaign_journal,
        };
        use replay_data::identity::IdentityCodec as _;
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let output = root.join("captures");
        let target = root.join("replays");
        let session = output.join("sessions").join(format!("{start}-1"));
        fs::create_dir_all(session.join("bin")).unwrap();
        fs::write(session.join("bin/collector.dll"), b"synthetic collector").unwrap();
        let replay_capture_spool::spool::Record::SessionStart(mut started) = allocation_records().remove(0)
        else {
            unreachable!()
        };
        started.process_id = pid;
        started.process_start_time = start;
        let mut session_bytes = Vec::new();
        started.encode(&mut session_bytes).unwrap();
        let names = ["ready", "detach", "done", "idle"]
            .map(|event| format!("Local\\BattlePlan.test.{pid}.{start}.{event}"));
        let events = Events::create(names.each_ref().map(String::as_str)).unwrap();
        write_json(
            &session.join("bin/collector.json"),
            &json!({
                "session_id":started.id,"output_dir":session,"replay_dir":target,
                "session_start":session_bytes,"ready_event":names[0],"detach_event":names[1],
                "done_event":names[2],"idle_event":names[3],
                "cancel_file":session.join("bin/collector.cancel"),
            }),
        )
        .unwrap();
        create_failure_witness(&session, started.id, start).unwrap();
        write_campaign_journal(&session, &allocation_records());
        write_json(
            &session.join("status.json"),
            &json!({"session":7,"state":"recording","hours":3,"last_hour":4,"campaign":campaign}),
        )
        .unwrap();
        replay_io::begin_admissions(&session, &target).unwrap();
        Admission::new(&session, &target)
            .admit_opening(campaign, FIRST_LOAD)
            .unwrap();
        write_json(
            &output.join("active.json"),
            &json!({
                "recorder_pid":1,"recorder_start_time":1,"game_pid":pid,"session_id":7,
                "process_start_time":start,"session_dir":session,
                "dll":session.join("bin").join("collector.dll"),
                "detach_event":names[1],"done_event":names[2],
            }),
        )
        .unwrap();
        (output, target, session, events)
    }

    const THIS_BUILD: Build<'static> = Build {
        recorder_version: "test",
        collector_hash: [3; 32],
    };

    #[test]
    fn a_recorder_takes_over_the_session_a_stopped_recorder_left_loaded() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_test_directory};
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let root = allocation_test_directory("orphan-take-over");
        let (pid, start) = orphan_game();
        let (output, target, session, events) = orphaned_session(&root, pid, start);
        let looked = Cell::new(0);
        let mut stopped = replay_io::Live::default();
        stopped
            .advance(&session, &target, &|_| false, unnamed(), &|| {
                looked.set(looked.get() + 1);
                looked.get() > 3
            })
            .unwrap();
        drop(stopped);
        let dll = session.join("bin").join("collector.dll");
        let (claim, inherited) = claim_game(
            &output,
            pid,
            start,
            || Ok(vec![dll.clone()]),
            &THIS_BUILD,
            |_, _| false,
        )
        .unwrap();
        let Inherited::TakeOver(orphan, _events) = inherited else {
            panic!("the orphaned session was not taken over");
        };
        assert_eq!(orphan.directory, session);
        assert_eq!(orphan.target, target);
        assert_eq!(orphan.session.id, 7);
        let held = read_status(&output.join("active.json")).unwrap();
        take_over(&output, &orphan, (4242, 99)).unwrap();
        let active = read_status(&output.join("active.json")).unwrap();
        assert_eq!(active["recorder_pid"], 4242);
        assert_eq!(active["recorder_start_time"], 99);
        assert_eq!(active["session_id"], 7);
        assert_eq!(active["session_dir"], held["session_dir"]);
        assert_eq!(active["dll"], held["dll"]);
        assert_eq!(Path::new(active["session_dir"].as_str().unwrap()), session);
        assert_eq!(Path::new(active["dll"].as_str().unwrap()), dll);
        assert_eq!(active["detach_event"], orphan.config["detach_event"]);
        assert_eq!(active["done_event"], orphan.config["done_event"]);
        assert_eq!(
            read_status(&session.join(replay_io::RESTARTS_FILE)).unwrap(),
            json!([{"recorder_pid":4242,"stopped_pid":1,"after_hour":4,"campaign":campaign}])
        );
        assert!(
            windows::Claim::take(&game_claim(pid, start))
                .unwrap()
                .is_none()
        );
        let mut admission = Admission::resume(&session, &target).unwrap();
        let ask = |_: &str, _: bool| -> Result<(), Refusal> { unreachable!() };
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || unreachable!(), &ask),
            Ok(Decision::Seen)
        );
        let finalized = finalize_with(&session, &target, replay_io::Live::default());
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        let recording = Replay::open(fs::read(&finalized.replays[0]).unwrap()).unwrap();
        assert_eq!(
            recording.recovery_notes,
            [replay_recording::records::RecoveryNote {
                session: Some(7),
                segment: 0,
                valid_bytes: 0,
                reason: "the recorder restarted: recorder 1 stopped while the game ran and recorder 4242 took the session up after hour 4; the collector's journal held every hour meanwhile".into(),
            }]
        );
        assert!(replay_io::is_restart(&recording.recovery_notes[0]));
        drop((claim, events));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_or_discard_during_a_take_over_scan_detaches_the_inherited_collector_and_ends_its_session() {
        use crate::test_support::allocation_test_directory;
        for discard in [false, true] {
            let root = allocation_test_directory(&format!("take-over-scan-stop-{discard}"));
            let (pid, start) = orphan_game();
            let (output, _target, session, events) = orphaned_session(&root, pid, start);
            let dll = session.join("bin").join("collector.dll");
            let (claim, inherited) = claim_game(
                &output,
                pid,
                start,
                || Ok(vec![dll.clone()]),
                &THIS_BUILD,
                |_, _| false,
            )
            .unwrap();
            let Inherited::TakeOver(orphan, _) = inherited else {
                panic!("the orphaned session was not taken over");
            };
            take_over(&output, &orphan, (4242, 99)).unwrap();
            let armed = DetachOnDrop::arm(&orphan.config, orphan.cancel_file()).unwrap();
            let control = Control::new();
            let asked = if discard {
                Stop::Discard("campaign".into())
            } else {
                Stop::Save(None)
            };
            let looks = Cell::new(0);
            let finished = || {
                looks.set(looks.get() + 1);
                looks.get() > 3
            };
            let stopping = || {
                if looks.get() == 2 {
                    control.stop(asked.clone());
                }
                control.stopping()
            };
            let goes_on = await_scan(finished, Some(&armed), stopping, || Ok(())).unwrap();
            assert!(goes_on, "Stop (discard: {discard}) left the session taken over unended");
            assert!(events.detach.handle.signaled().unwrap());
            assert!(!orphan.cancel_file().exists(), "Stop drains the collector");
            assert_eq!(control.seal(), Some(asked));
            drop(armed);
            assert!(orphan.cancel_file().is_file());
            drop((claim, events));
            fs::remove_dir_all(root).unwrap();
        }
        assert!(!await_scan(|| false, None, || true, || Ok(())).unwrap());
        let root = allocation_test_directory("take-over-scan-exit");
        let (pid, start) = orphan_game();
        let (_output, _target, session, events) = orphaned_session(&root, pid, start);
        let (config, _) = collector_config(&session).unwrap();
        let cancel = session.join("bin/collector.cancel");
        let armed = DetachOnDrop::arm(&config, cancel.clone()).unwrap();
        let exited = || Err("game exited while content identity was being gathered".to_owned());
        let left = await_scan(|| false, Some(&armed), || false, exited);
        assert_eq!(left, Err("game exited while content identity was being gathered".to_owned()));
        assert!(!events.detach.handle.signaled().unwrap());
        drop(armed);
        assert!(events.detach.handle.signaled().unwrap());
        assert!(cancel.is_file());
        drop(events);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_second_live_game_never_ends_the_content_scan_of_the_chosen_game() {
        use recorder_platform::games::ProcessState::Running;
        let chosen = GameProcess {
            pid: 58524,
            start_time: 2,
            state: Running,
        };
        let other = GameProcess {
            pid: 75600,
            start_time: 1,
            state: Running,
        };
        let mut others = OtherGames::default();
        assert_eq!(
            watch_chosen(chosen.pid, false, Some(&[other, chosen]), &mut others),
            Ok(())
        );
        assert_eq!(watch_chosen(chosen.pid, false, None, &mut others), Ok(()));
        assert_eq!(
            watch_chosen(chosen.pid, true, Some(&[chosen]), &mut others),
            Err("game exited while content identity was being gathered".to_owned())
        );
    }

    #[test]
    fn a_session_another_build_started_ends_with_the_game_alive_before_recording_afresh() {
        use crate::test_support::{ALLOCATION_CAMPAIGN, allocation_test_directory};
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let root = allocation_test_directory("orphan-restart");
        let (pid, start) = orphan_game();
        let (output, _target, session, events) = orphaned_session(&root, pid, start);
        let dll = session.join("bin").join("collector.dll");
        let other = Build {
            recorder_version: "test",
            collector_hash: [4; 32],
        };
        let (claim, inherited) = claim_game(
            &output,
            pid,
            start,
            || Ok(vec![dll.clone()]),
            &other,
            |_, _| false,
        )
        .unwrap();
        let Inherited::Restart(orphan, reason) = inherited else {
            panic!("a session of another build was taken over");
        };
        assert_eq!(
            reason,
            format!(
                "it was started by recorder test with collector {}, and this is recorder test with collector {}",
                "03".repeat(32),
                "04".repeat(32)
            )
        );
        assert_eq!(
            end_orphan(&output, &orphan, &reason, || {
                assert!(events.detach.handle.signaled().unwrap());
                assert!(session.join("bin/collector.cancel").is_file());
                Ok(CollectorPresence {
                    process_exited: false,
                    collector_loaded: Some(true),
                })
            }),
            Err(ORPHAN_PINNED.to_owned())
        );
        assert!(!session.join("completed.json").exists());
        end_orphan(&output, &orphan, &reason, || {
            write_json(
                &session.join("status.json"),
                &json!({"session":7,"state":"detached","drain_complete":true,"hours":3,"last_hour":4,"campaign":campaign}),
            )
            .unwrap();
            Ok(CollectorPresence {
                process_exited: false,
                collector_loaded: Some(false),
            })
        })
        .unwrap();
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["recovered"], true);
        assert_eq!(completed["detach_requested"], true);
        assert_eq!(completed["detached"], true);
        assert_eq!(completed["process_exited"], false);
        assert_eq!(completed["collector_loaded"], false);
        assert_eq!(completed["partial"], false, "{completed}");
        assert_eq!(completed["recorder_restart"], json!(reason));
        assert_eq!(
            completed["waiting"],
            json!([{"campaign":campaign,"hours":3,"first_hour":2,"last_hour":4}])
        );
        assert!(!output.join("active.json").exists());
        assert!(
            windows::Claim::take(&game_claim(pid, start))
                .unwrap()
                .is_none()
        );
        drop((claim, events));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_refused_content_check_names_its_reason_and_both_folders() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("content-refused");
        let session = root.join("session");
        fs::create_dir_all(session.join("bin")).unwrap();
        let reason = "content mounts differ from the session's (observed 2, expected 2): mount 1, highest priority first, differs as written and as a folder";
        write_json(
            &session.join("content.json"),
            &json!({
                "verification":{"state":"unverified","reason":reason},
                "mount_difference":{
                    "mount":1,"observed":"C:/mods/top","expected":"C:/mods/base",
                    "differs":"differs as written and as a folder",
                },
            }),
        )
        .unwrap();
        let explained = format!(
            "content verification failed: {reason}; mount 1 differs as written and as a folder: the game mounts C:/mods/top and the session's content expects C:/mods/base"
        );
        assert_eq!(
            collector_error(&session, "content verification failed\n"),
            explained
        );
        fs::write(
            session.join("bin/collector.error.txt"),
            "content verification failed",
        )
        .unwrap();
        write_json(
            &session.join("status.json"),
            &json!({"session":7,"state":"recording","error":"content verification failed"}),
        )
        .unwrap();
        let terminal = terminal_outcome(&session, 7, 13, false, true, true);
        assert_eq!(
            terminal.issues,
            [format!("collector terminal error: {explained}")]
        );
        write_json(
            &session.join("content.json"),
            &json!({
                "verification":{"state":"unverified","reason":"mount 2 was not expected"},
                "mount_difference":{
                    "mount":2,"observed":"C:/mods/extra","expected":null,"differs":"was not expected",
                },
            }),
        )
        .unwrap();
        assert_eq!(
            collector_error(&session, "content verification failed"),
            "content verification failed: mount 2 was not expected; mount 2 was not expected: the game mounts C:/mods/extra and the session's content expects no folder"
        );
        assert_eq!(
            collector_error(&session, " hourly hook construction failed\n"),
            "hourly hook construction failed"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_unreadable_content_record_says_the_reason_could_not_be_read() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("content-unreadable");
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        let unread = "content verification failed; the reason could not be read";
        assert_eq!(
            collector_error(&session, "content verification failed"),
            unread
        );
        fs::write(session.join("content.json"), b"{\"verification\":").unwrap();
        assert_eq!(
            collector_error(&session, "content verification failed"),
            unread
        );
        write_json(
            &session.join("content.json"),
            &json!({"verification":{"state":"verified"},"mount_difference":null}),
        )
        .unwrap();
        assert_eq!(
            collector_error(&session, "content verification failed"),
            unread
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_live_recorders_game_and_an_ended_sessions_collector_stay_refused() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("orphan-refused");
        let (pid, start) = orphan_game();
        let (output, _target, session, events) = orphaned_session(&root, pid, start);
        let dll = session.join("bin").join("collector.dll");
        let claim = |alive: bool, loaded: Vec<PathBuf>| {
            claim_game(
                &output,
                pid,
                start,
                || Ok(loaded),
                &THIS_BUILD,
                |recorder, started| {
                    assert_eq!((recorder, started), (1, 1));
                    alive
                },
            )
        };
        let held = windows::Claim::take(&game_claim(pid, start))
            .unwrap()
            .unwrap();
        let refused = claim_game(
            &output,
            pid,
            start,
            || unreachable!("a held game is not looked into"),
            &THIS_BUILD,
            |_, _| unreachable!(),
        );
        assert_eq!(refused.err().as_deref(), Some(GAME_CLAIMED));
        drop(held);
        assert_eq!(
            claim(true, vec![dll.clone()]).err().as_deref(),
            Some(GAME_CLAIMED)
        );
        assert!(matches!(
            claim(false, Vec::new()),
            Ok((_, Inherited::Nothing))
        ));
        let elsewhere = root.join("other/sessions/1-1/bin");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("collector.dll"), b"synthetic collector").unwrap();
        assert!(
            claim(false, vec![elsewhere.join("collector.dll")])
                .err()
                .unwrap()
                .starts_with(ORPHAN_ELSEWHERE)
        );
        write_json(&session.join("completed.json"), &json!({"partial":false})).unwrap();
        assert_eq!(
            claim(false, vec![dll]).err().as_deref(),
            Some(COLLECTOR_LOADED)
        );
        drop(events);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_sessions_end_saves_every_campaign_with_auto_save_and_otherwise_only_what_stop_saves() {
        let (current, other) = ("current", "other");
        let saved = |auto_save, stop, discard| {
            [current, other]
                .map(|campaign| saved_at_end(campaign, Some(current), auto_save, stop, discard))
        };
        assert_eq!(saved(true, false, false), [true, true]);
        assert_eq!(saved(true, true, false), [true, true]);
        assert_eq!(saved(true, true, true), [false, true]);
        assert_eq!(saved(false, false, false), [false, false]);
        assert_eq!(saved(false, true, false), [true, false]);
        assert_eq!(saved(false, true, true), [false, false]);
    }

    #[test]
    fn discard_drops_only_the_campaign_the_user_confirmed_and_only_while_it_is_in_progress() {
        let confirmed = Stop::Discard("a".into());
        assert_eq!(discard_target(Some(&confirmed), Some("a")), Some(Ok("a")));
        assert_eq!(
            discard_target(Some(&confirmed), Some("b")),
            Some(Err(Undiscarded::Another)),
            "the game loaded b before the stop was taken up, so b’s hours stay"
        );
        assert_eq!(
            discard_target(Some(&confirmed), None),
            Some(Err(Undiscarded::Unread))
        );
        assert_eq!(discard_target(Some(&Stop::Save(None)), Some("a")), None);
        let read = || Some("b".to_owned());
        assert_eq!(stopped_on(Some(&Stop::Save(Some("a".into()))), false, None, read()).as_deref(), Some("a"), "a Save keeps the campaign the user confirmed, though the game has loaded another since");
        assert_eq!(stopped_on(Some(&Stop::Save(None)), false, None, read()).as_deref(), Some("b"), "a Save that names none, as Ctrl+C and a quit ask, keeps the one in progress");
        assert_eq!(stopped_on(Some(&confirmed), true, Some("a"), read()).as_deref(), Some("a"));
        assert_eq!(discard_target(None, Some("a")), None);
    }

    #[test]
    fn a_discard_whose_stop_time_read_failed_keeps_the_campaign_it_last_saw() {
        let confirmed = Stop::Discard("a".into());
        let failed_read = None;
        assert_eq!(
            in_progress(Some("a"), Some(false), Some("a".into())).as_deref(),
            Some("a"),
            "a Save still keeps the campaign the recorder last saw"
        );
        assert_eq!(
            discard_target(Some(&confirmed), failed_read),
            Some(Err(Undiscarded::Unread)),
            "but deleting its hours needs the game to name it at Stop"
        );
    }

    #[test]
    fn stop_acts_on_no_campaign_when_it_cannot_read_the_one_the_game_left_for() {
        let written = |campaign: &str| Some(campaign.to_owned());
        assert_eq!(
            in_progress(Some("b"), Some(true), written("a")).as_deref(),
            Some("b")
        );
        assert_eq!(
            in_progress(Some("a"), None, written("b")).as_deref(),
            Some("a")
        );
        assert_eq!(
            in_progress(Some("a"), Some(false), written("a")).as_deref(),
            Some("a")
        );
        assert_eq!(
            in_progress(Some("a"), Some(false), None).as_deref(),
            Some("a")
        );
        assert_eq!(in_progress(Some("a"), Some(false), written("b")), None);
        assert_eq!(
            in_progress(None, Some(false), written("b")).as_deref(),
            Some("b")
        );
    }

    fn take_done(settling: &Settling) -> Vec<SetAside> {
        std::mem::take(&mut *locked(&settling.done))
    }

    fn unnamed() -> replay_io::Naming<'static> {
        static SETTINGS: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
        replay_io::Naming {
            localisation: None,
            report: None,
            settings: SETTINGS.get_or_init(|| {
                std::env::temp_dir().join(format!(
                    "hoi4-unwritten-recorder-settings-{}.json",
                    std::process::id()
                ))
            }),
        }
    }

    #[test]
    fn naming_background_jobs_propagate_to_recovery_and_localisation_helpers() {
        use job_priority::{JobPriority, Level};
        assert!(!JobPriority::current().background());
        background_naming(|| {
            assert!(JobPriority::current().background());
            let priority = JobPriority::current();
            std::thread::spawn(move || {
                priority.enter(Level::Normal);
                assert!(JobPriority::current().background());
                #[cfg(windows)]
                unsafe {
                    use windows_sys::Win32::System::Threading::{
                        GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
                    };
                    assert_eq!(
                        GetThreadPriority(GetCurrentThread()),
                        THREAD_PRIORITY_BELOW_NORMAL
                    );
                }
                let priority = JobPriority::current();
                std::thread::spawn(move || {
                    priority.enter(Level::BelowNormal);
                    assert!(JobPriority::current().background());
                    #[cfg(windows)]
                    unsafe {
                        use windows_sys::Win32::System::Threading::{
                            GetCurrentThread, GetThreadPriority, THREAD_PRIORITY_LOWEST,
                        };
                        assert_eq!(
                            GetThreadPriority(GetCurrentThread()),
                            THREAD_PRIORITY_LOWEST
                        );
                    }
                })
                .join()
                .unwrap();
            })
            .join()
            .unwrap();
        });
        assert!(!JobPriority::current().background());
    }

    #[test]
    fn requests_the_finalizer_stopped_before_acting_on_decide_their_campaigns_destination() {
        let settling = Settling::default();
        locked(&settling.requests).push(("forwarded".to_owned(), false));
        let unstarted = FinalizingAhead {
            packing: None,
            stop: Arc::default(),
            promoted: Arc::default(),
            thread: None,
            asked: Arc::default(),
            names: Arc::default(),
        };
        let (_, _, unanswered) = finish_ahead(unstarted, Some(&settling));
        assert!(locked(&settling.requests).is_empty());
        assert_eq!(unanswered, [("forwarded".to_owned(), false)], "a request the ending session never took is handed to its final destinations");
        assert!(take_done(&settling).is_empty(), "and it is never answered as a failure");
        locked(&settling.requests).push(("newer".to_owned(), false));
        requeue(&settling, "newer".to_owned(), true);
        requeue(&settling, "retried".to_owned(), true);
        assert_eq!(*locked(&settling.requests), [("retried".to_owned(), true), ("newer".to_owned(), false)], "a retry goes ahead of what was accepted after it, and never over a newer choice for its campaign");
        locked(&settling.requests).clear();
        assert_eq!(
            latest_choices(vec![("a".to_owned(), true), ("b".to_owned(), false), ("a".to_owned(), false)]),
            [("b".to_owned(), false), ("a".to_owned(), false)],
            "a Discard and a later Save in one batch act as the Save alone, so a failed release can't keep the Discard"
        );
        let mut decisions: BTreeMap<String, bool> = unanswered.into_iter().collect();
        decisions.extend([("forwarded".to_owned(), true), ("dropped".to_owned(), true)]);
        assert_eq!(
            with_lost_decisions("detach timed out".to_owned(), &decisions),
            "detach timed out; not carried out, so choose again once the session is taken up: Discard of campaign dropped, Discard of campaign forwarded",
            "a timeout names every decision it lost, not only the first"
        );
        assert_eq!(with_lost_decisions("detach timed out".to_owned(), &BTreeMap::new()), "detach timed out");
        assert_eq!(decided_destination(&decisions, "forwarded"), Some(replay_io::Destination::Unsaved), "a decision taken after one already forwarded wins: this later Discard keeps the campaign out of the recordings, to be rolled back");
        assert_eq!(decided_destination(&BTreeMap::from([("kept".to_owned(), false)]), "kept"), Some(replay_io::Destination::Recording), "an accepted Save publishes, whatever Auto save says");
        assert_eq!(decided_destination(&decisions, "dropped"), Some(replay_io::Destination::Unsaved), "an accepted Discard is never published, even with Auto save on");
        assert_eq!(decided_destination(&decisions, "other"), None);
    }

    #[test]
    fn finalization_progress_locked_reader_does_not_abort_publication() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let root = allocation_test_directory("finalization-progress-reader");
        let session = root.join("session");
        let output = root.join("replays");
        write_campaign_journal(&session, &allocation_records());
        let observed = Cell::new(0);
        let observe = |records| observed.set(records);
        let (base, ahead) = (Instant::now(), std::rc::Rc::new(Cell::new(Duration::ZERO)));
        let mut progress = FinalizationProgress::new(&session, &observe);
        progress.clock.fixed = Some({
            let ahead = std::rc::Rc::clone(&ahead);
            std::rc::Rc::new(move || base + ahead.get())
        });
        progress.last_write.set(base);
        let path = session.join("finalization-progress.json");
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_json(&path, &json!({"records": 1})).is_err());

        let done = replay_io::finalize_session_outcome_with_progress(&session, &output, &|| {
            ahead.set(ahead.get() + Duration::from_secs(1));
            progress.advance();
            Ok(())
        })
        .unwrap();
        assert!(done.errors.is_empty(), "{:?}", done.errors);
        assert_eq!(done.replays.len(), 1);
        assert_eq!(published_hours(&done.replays[0]), [(9, 2), (9, 3), (9, 4)]);
        assert!(observed.get() > 0);
        assert_eq!(observed.get(), progress.records.get());
        assert_eq!(fs::read(&path).unwrap(), before);
        drop(reader);

        ahead.set(ahead.get() + Duration::from_secs(1));
        progress.advance();
        let resumed: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(resumed["records"], observed.get());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn finalization_progress_initial_write_failure_is_advisory_and_throttled() {
        use crate::test_support::allocation_test_directory;

        let root = allocation_test_directory("finalization-progress-initial-write");
        let session = root.join("session");
        fs::write(&session, b"not a directory").unwrap();
        let observed = Cell::new(0);
        let observe = |records| observed.set(records);
        let (base, ahead) = (Instant::now(), std::rc::Rc::new(Cell::new(Duration::ZERO)));
        let mut progress = FinalizationProgress::new(&session, &observe);
        progress.clock.fixed = Some({
            let ahead = std::rc::Rc::clone(&ahead);
            std::rc::Rc::new(move || base + ahead.get())
        });
        progress.last_write.set(base);
        ahead.set(Duration::from_secs(1) - Duration::from_nanos(1));
        progress.advance();
        assert_eq!(observed.get(), 0, "no attempt before a second has passed");
        ahead.set(Duration::from_secs(1));
        progress.advance();
        assert_eq!(observed.get(), 2);
        assert_eq!(progress.last_write.get(), base + Duration::from_secs(1), "a failed attempt still waits its second");
        ahead.set(Duration::from_secs(2) - Duration::from_nanos(1));
        progress.advance();
        assert_eq!(observed.get(), 2);
        fs::remove_file(&session).unwrap();
        fs::create_dir(&session).unwrap();
        ahead.set(Duration::from_secs(2));
        progress.advance();
        let resumed: Value =
            serde_json::from_slice(&fs::read(session.join("finalization-progress.json")).unwrap())
                .unwrap();
        assert_eq!(resumed["records"], 4);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn busy_process_activity_cannot_extend_a_stalled_finalizer_indefinitely() {
        let start = Instant::now();
        let mut watch = ProgressWatch::new(start);
        for second in [0, 179, 358, 537, 599] {
            assert!(!watch.stalled(
                start + Duration::from_secs(second),
                ProgressStamp {
                    activity: Some((second + 1, second + 1, second + 1)),
                    ..ProgressStamp::default()
                }
            ));
        }
        assert!(watch.stalled(
            start + Duration::from_secs(600),
            ProgressStamp {
                activity: Some((1000, 1000, 1000)),
                ..ProgressStamp::default()
            }
        ));
        assert!(!watch.stalled(
            start + Duration::from_secs(601),
            ProgressStamp {
                work: Some(8),
                ..ProgressStamp::default()
            }
        ));
        assert!(!watch.stalled(
            start + Duration::from_secs(1200),
            ProgressStamp {
                activity: Some((2000, 2000, 2000)),
                ..ProgressStamp::default()
            }
        ));
        assert!(watch.stalled(
            start + Duration::from_secs(1201),
            ProgressStamp {
                work: Some(8),
                activity: Some((3000, 3000, 3000)),
                ..ProgressStamp::default()
            }
        ));
    }

    #[test]
    fn stale_json_follows_real_work_and_process_activity_but_still_detects_inactivity() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("finalization-work-watch");
        let observe = |_| {};
        let progress = FinalizationProgress::new(&root, &observe);
        let path = root.join("finalization-progress.json");
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let start = Instant::now();
        let mut watch = ProgressWatch::new(start);
        let mut stamp = progress_stamp(&root, None);
        assert!(!watch.stalled(start, stamp));
        for seconds in [179, 358, 537] {
            progress.records.set(progress.records.get() + 1);
            progress.publish();
            let next = progress_stamp(&root, None);
            assert_eq!(next.finalization, stamp.finalization);
            assert_ne!(next.work, stamp.work);
            assert!(!watch.stalled(start + Duration::from_secs(seconds), next));
            stamp = next;
        }
        for seconds in [700, 860] {
            stamp.activity = Some((seconds, 1, 1));
            assert!(!watch.stalled(start + Duration::from_secs(seconds), stamp));
        }
        assert!(watch.stalled(start + Duration::from_secs(1040), stamp));
        drop(reader);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unreadable_progress_files_follow_cpu_reads_and_writes_without_inventing_counts() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("finalization-process-watch");
        fs::create_dir(root.join("finalization-progress.json")).unwrap();
        fs::create_dir(root.join("finalization-work")).unwrap();
        fs::create_dir(root.join("history-requests")).unwrap();
        fs::write(root.join("history-requests/detached"), b"released").unwrap();
        let start = Instant::now();
        let mut watch = ProgressWatch::new(start);
        let mut stamp = progress_stamp(&root, None);
        assert!(stamp.finalization.is_none());
        assert!(stamp.work.is_none());
        assert!(stamp.activity.is_none());
        assert!(!watch.stalled(start, stamp));
        for (seconds, activity) in [(179, (1, 0, 0)), (358, (1, 1, 0)), (537, (1, 1, 1))] {
            stamp = progress_stamp(&root, Some(activity));
            assert!(!watch.stalled(start + Duration::from_secs(seconds), stamp));
        }
        assert!(watch.stalled(start + Duration::from_secs(717), stamp));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn detach_wait_follows_progress_past_the_old_total_deadline_and_detects_stalls() {
        let start = Instant::now();
        let mut watch = ProgressWatch::new(start);
        for seconds in [0, 60, 119, 127, 240, 1000] {
            let stamp = Some(UNIX_EPOCH + Duration::from_secs(seconds));
            assert!(!watch.stalled(
                start + Duration::from_secs(seconds),
                ProgressStamp {
                    finalization: stamp,
                    ..ProgressStamp::default()
                }
            ));
        }
        let stamp = ProgressStamp {
            finalization: Some(UNIX_EPOCH + Duration::from_secs(1000)),
            ..ProgressStamp::default()
        };
        assert!(!watch.stalled(start + Duration::from_secs(1150), stamp));
        assert!(!watch.stalled(start + Duration::from_secs(1179), stamp));
        assert!(watch.stalled(start + Duration::from_secs(1180), stamp));
    }

    #[test]
    fn detach_wait_follows_collector_steps_and_ends_a_stall() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut wait = CollectorWait::new(start, DONE_STALL);
        let mut length = 0;
        for second in 0..=92 {
            if second % 5 == 0 {
                length += 40;
            }
            assert_eq!(
                wait.ended(at(second), (Some(length), None)),
                None,
                "{second} s"
            );
        }
        let mut wait = CollectorWait::new(start, DONE_STALL);
        assert_eq!(wait.ended(at(10), (Some(40), None)), None);
        assert_eq!(wait.ended(at(130), (Some(80), None)), None, "a 120 s step");
        assert_eq!(wait.ended(at(309), (Some(80), None)), None);
        assert_eq!(
            wait.ended(at(310), (Some(80), None)),
            Some(WaitEnd::Stalled(Duration::from_secs(180)))
        );
        let mut ready = CollectorWait::new(start, READY_STALL);
        assert_eq!(ready.ended(at(10), (Some(40), None)), None);
        assert_eq!(
            ready.ended(at(160), (Some(40), None)),
            None,
            "a 150 s attach step"
        );
        assert_eq!(
            ready.ended(at(190), (Some(40), None)),
            Some(WaitEnd::Stalled(Duration::from_secs(180)))
        );
        let mut wait = CollectorWait::new(start, DONE_STALL);
        let cap = COLLECTOR_WAIT_CAP.as_secs();
        for second in (0..cap).step_by(30) {
            let stamp = (None, Some(UNIX_EPOCH + Duration::from_secs(second)));
            assert_eq!(wait.ended(at(second), stamp), None);
        }
        let stamp = (None, Some(UNIX_EPOCH + Duration::from_secs(cap)));
        assert_eq!(wait.ended(at(cap), stamp), Some(WaitEnd::Capped));
    }

    #[test]
    fn step_log_names_the_last_step_and_times_the_slowest() {
        let log = "0 attach_started\n15 ready\n90000 detach_requested\n90100 restore Hourly done\n\
                   120100 restore Raid done\n121000 spool_draining 3\n182400 done\n";
        assert_eq!(last_step(log).unwrap(), "done at 182.4 s");
        assert_eq!(
            steps_between(log, "detach_requested", "done").unwrap(),
            "took 92.4 s; slowest steps: done 61.4 s, restore Raid done 30.0 s, \
             spool_draining 3 0.9 s, restore Hourly done 0.1 s"
        );
        assert_eq!(
            steps_between(log, "attach_started", "ready").unwrap(),
            "took 0.0 s; slowest steps: ready 0.0 s"
        );
        assert_eq!(steps_between(log, "detach_requested", "missing"), None);
        assert_eq!(last_step(""), None);
    }

    #[test]
    fn politics_capture_defaults_on_and_can_only_be_disabled_for_recording() {
        assert!(parse_options("record", vec![]).unwrap().politics_capture);
        let option: OsString = "--disable-politics-capture".into();
        assert!(
            !parse_options("record", vec![option.clone()])
                .unwrap()
                .politics_capture
        );
        for command in ["detach", "refresh", "finalize"] {
            assert!(parse_options(command, vec![option.clone()]).is_err());
        }
        #[cfg(feature = "alloc-fault-injection")]
        {
            let options = parse_options(
                "record",
                vec![option, "--alloc-fault".into(), "callback:1".into()],
            )
            .unwrap();
            assert!(!options.politics_capture);
            assert_eq!(options.alloc_fault.unwrap().ordinal, 1);
        }
    }

    #[test]
    fn game_score_cli_defaults_enabled_and_disables_at_attach() {
        assert!(parse_options("record", Vec::new()).unwrap().game_score);
        let options = parse_options(
            "record",
            vec![
                "--once".into(),
                "--no-game-score".into(),
                "--out".into(),
                "output".into(),
                "--user-dir".into(),
                "user".into(),
            ],
        )
        .unwrap();
        assert!(!options.game_score);
        assert!(options.once);
        assert_eq!(options.output, PathBuf::from("output"));
        assert_eq!(options.user, Some(PathBuf::from("user")));
        assert!(usage().contains("[--no-game-score]"));
    }

    #[test]
    fn capture_mode_cli_defaults_full_and_names_only_known_modes() {
        assert_eq!(
            parse_options("record", Vec::new()).unwrap().capture_mode,
            CaptureMode::Full
        );
        let options = parse_options("record", vec!["--capture-mode".into(), "map".into()]).unwrap();
        assert_eq!(options.capture_mode, CaptureMode::Map);
        for refused in [
            vec!["--capture-mode".into(), "light".into()],
            vec!["--capture-mode".into()],
        ] {
            assert!(parse_options("record", refused).is_err());
        }
        for command in ["detach", "refresh", "finalize"] {
            assert!(parse_options(command, vec!["--capture-mode".into(), "map".into()]).is_err());
        }
        assert!(usage().contains("[--capture-mode full|map]"));
    }

    #[test]
    fn worker_sampler_config_preserves_environment_text_verbatim() {
        let original = json!({"game_score": true, "output_dir": "synthetic/session"});
        assert_eq!(worker_sampler_config(original.clone(), None), original);
        for requested in ["", "off", r"interval_us=50"] {
            let config = worker_sampler_config(original.clone(), Some(requested.into()));
            assert_eq!(config["worker_sampler"], requested);
            assert_eq!(config["game_score"], original["game_score"]);
            assert_eq!(config["output_dir"], original["output_dir"]);
        }
        assert!(usage().contains("BATTLEPLAN_WORKER_SAMPLER"));
    }

    #[test]
    fn game_score_cli_refuses_disable_for_other_commands() {
        for command in ["detach", "refresh", "finalize"] {
            assert!(parse_options(command, vec!["--no-game-score".into()]).is_err());
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn allocation_fault_cli_requires_a_named_phase_and_positive_ordinal() {
        assert_eq!(
            allocation_fault(OsStr::new("callback:17")).unwrap(),
            AllocationFault {
                phase: "callback".into(),
                ordinal: 17,
                after_hours: 0,
            }
        );
        assert_eq!(
            allocation_fault(OsStr::new("encode:1:3")).unwrap(),
            AllocationFault {
                phase: "encode".into(),
                ordinal: 1,
                after_hours: 3,
            }
        );
        assert_eq!(
            allocation_fault(OsStr::new("teardown:1:0"))
                .unwrap()
                .after_hours,
            0
        );
        for value in [
            "callback",
            "callback:0",
            "callback:-1",
            ":1",
            "callback:1:",
            "callback:1:-1",
            "callback:1:2:3",
            "callback:1:18446744073709551616",
        ] {
            assert!(allocation_fault(OsStr::new(value)).is_err(), "{value}");
        }
    }

    #[cfg(feature = "alloc-fault-injection")]
    #[test]
    fn allocation_fault_json_is_omitted_until_requested() {
        let healthy = json!({"session_id":7});
        assert_eq!(fault_config(healthy.clone(), None), healthy);
        let fault = allocation_fault(OsStr::new("spool:540")).unwrap();
        assert_eq!(
            fault_config(healthy.clone(), Some(&fault)),
            json!({"session_id":7,"alloc_fault":{"phase":"spool","ordinal":540}})
        );
        let fault = allocation_fault(OsStr::new("encode:1:3")).unwrap();
        assert_eq!(
            fault_config(healthy, Some(&fault)),
            json!({"session_id":7,"alloc_fault":{"phase":"encode","ordinal":1,"after_hours":3}})
        );
    }

    #[cfg(not(feature = "alloc-fault-injection"))]
    #[test]
    fn allocation_fault_cli_refuses_feature_option_in_default_build() {
        assert!(!usage().contains("--alloc-fault"));
        assert_eq!(
            command(vec![
                "record".into(),
                "--alloc-fault".into(),
                "spool:1".into()
            ])
            .unwrap_err(),
            "--alloc-fault requires recorder feature alloc-fault-injection"
        );
    }

    #[test]
    fn destruction_console_diagnostic_retains_the_site_and_reason() {
        for site in 0..4 {
            let status = json!({"equipment_destruction": {
                "refusal": {"site_index": site, "reason": "synthetic pool increase"}
            }});
            assert_eq!(
                destruction_refusal(&status),
                Some((site, "synthetic pool increase"))
            );
        }
        for status in [
            json!({}),
            json!({"equipment_destruction": {"refusal": null}}),
        ] {
            assert_eq!(destruction_refusal(&status), None);
        }
        let status = json!({"equipment_destruction": {
            "refusal": {"site_index": null, "reason": "buffer allocation failed"}
        }});
        assert_eq!(destruction_refusal(&status), None);
    }

    #[test]
    fn allocation_failure_witness_overrides_stale_success_and_publishes_terminal_status() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("terminal-status");
        create_failure_witness(&directory, 7, 13).unwrap();
        assert!(
            replay_io::read_failure_witness(&directory)
                .unwrap()
                .is_none()
        );
        assert!(create_failure_witness(&directory, 7, 13).is_err());
        write_json(
            &directory.join("status.json"),
            &json!({
                "session":7,"state":"detached","drain_complete":true,"hours":3,
            }),
        )
        .unwrap();
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let outcome = terminal_outcome(&directory, 7, 13, true, false, false);
        assert_eq!(outcome.witness, Some(witness));
        assert_eq!(outcome.drain_complete, Some(false));
        assert!(outcome.recovery_allowed);
        assert!(
            outcome
                .issues
                .iter()
                .any(|issue| issue.contains("collector allocation failed"))
        );
        publish_failure_status(&directory, &witness).unwrap();
        let status: Value =
            serde_json::from_slice(&fs::read(directory.join("status.json")).unwrap()).unwrap();
        assert_eq!(status["state"], "capture_failed");
        assert_eq!(status["capture_failed"], true);
        assert_eq!(status["failure_code"], "allocation_failed");
        assert_eq!(status["failed_hour"], 99);
        assert_eq!(status["partial"], true);
        assert_eq!(status["drain_complete"], false);
        assert_eq!(status["hours"], 3);
        let completed = json!({"partial":true,"capture_failed":true,"failure_code":"allocation_failed","failed_hour":99,"issues":outcome.issues});
        assert!(completed_detach(&completed).is_err());
    }

    #[test]
    fn live_failure_refuses_refresh_and_authenticates_request_diagnostics() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("live-failure-refresh");
        create_failure_witness(&directory, 7, 13).unwrap();
        let active = json!({"session_id":7,"process_start_time":13});
        assert_eq!(live_failure(&directory, 7, 13).unwrap(), None);
        assert!(refuse_failed_refresh(&directory, &active).is_ok());
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        write_json(
            &directory.join("status.json"),
            &json!({"session":7,"state":"recording","hours":16}),
        )
        .unwrap();
        for session in [8, 7] {
            write_json(&directory.join("collector-failure-detail.json"), &json!({
                "session":session,"process_start_time":13,"family":"raw bytes","requested_bytes":322536576u64
            })).unwrap();
            assert_eq!(live_failure(&directory, 7, 13).unwrap(), Some(witness));
            assert!(live_failure(&directory, 7, 14).is_err());
            let error = refuse_failed_refresh(&directory, &active).unwrap_err();
            assert!(error.contains("capture failed") && error.contains("refresh unavailable"));
            let status: Value =
                serde_json::from_slice(&fs::read(directory.join("status.json")).unwrap()).unwrap();
            assert_eq!(status["state"], "capture_failed");
            assert_eq!(status["hours"], 16);
            if session == 7 {
                assert_eq!(status["allocation_family"], "raw bytes");
                assert_eq!(status["allocation_requested_bytes"], 322536576u64);
            } else {
                assert!(status.get("allocation_family").is_none());
            }
        }
    }

    #[test]
    fn final_spool_allows_an_incomplete_drain_without_claiming_clean_detach() {
        use crate::test_support::allocation_test_directory;
        let directory = allocation_test_directory("final-pinned-spool");
        create_failure_witness(&directory, 7, 13).unwrap();
        write_json(
            &directory.join("status.json"),
            &json!({
                "session":7,"state":"pinned","spool_closed":true,"recording_stopped":true,
                "drain_complete":false,"error":"synthetic hook restoration refused",
            }),
        )
        .unwrap();
        let live = terminal_outcome(&directory, 7, 13, true, false, false);
        assert!(!live.recovery_allowed);
        let ended = terminal_outcome(&directory, 7, 13, true, false, true);
        assert!(ended.recovery_allowed);
        assert_eq!(ended.drain_complete, Some(false));
        assert!(!ended.issues.is_empty());
        assert!(completed_detach(&json!({
            "partial":true,"issues":ended.issues,"detach_requested":true,
            "detached":false,"collector_loaded":true,"process_exited":false,"drain_complete":false,
        })).is_err());
    }

    #[test]
    fn done_with_a_retained_module_saves_only_the_committed_prefix_and_its_gap() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, allocation_witness,
        };
        let root = allocation_test_directory("done-retained-module");
        let session = root.join("session");
        let output = root.join("recordings");
        let records = allocation_records();
        let end = records
            .iter()
            .position(
                |record| matches!(record, replay_capture_spool::spool::Record::Frame(frame) if frame.hour == 4),
            )
            .unwrap();
        crate::test_support::write_capture(
            &session.join(std::str::from_utf8(crate::test_support::ALLOCATION_CAMPAIGN).unwrap()),
            0,
            &records[..end],
            false,
        );
        let mut witness = allocation_witness(0);
        witness.failed_hour = Some(4);
        witness.phase = 6;
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let outcome = terminal_outcome(&session, 7, 13, true, false, true);
        assert!(outcome.recovery_allowed);
        fs::create_dir_all(&output).unwrap();
        let _lock = finalization_lock(&output).unwrap();
        let error = finalize_cli_session(
            &session,
            &output,
            terminal_outcome(&session, 7, 13, true, false, true),
            || CollectorPresence {
                process_exited: false,
                collector_loaded: Some(true),
            },
        )
        .unwrap_err();
        assert!(error.contains("partial"));
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["detached"], false);
        assert_eq!(completed["collector_loaded"], true);
        assert_eq!(completed["process_exited"], false);
        assert_eq!(completed["partial"], true);
        let paths = completed["replays"].as_array().unwrap();
        assert_eq!(paths.len(), 1);
        let path = PathBuf::from(paths[0].as_str().unwrap());
        assert_eq!(published_hours(&path), [(9, 2), (9, 3)]);
        let replay = replay::campaign::Replay::open_path(&path).unwrap();
        assert_eq!(
            replay
                .gaps
                .iter()
                .map(|gap| (gap.branch, gap.first_hour, gap.last_hour))
                .collect::<Vec<_>>(),
            [(9, 4, 4)]
        );
        assert!(matches!(
            replay.seek(9, 4).unwrap(),
            replay::campaign::SeekResult::Gap { .. }
        ));
    }

    #[test]
    fn cli_finalize_preserves_incomplete_drain_without_an_allocation_witness() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        let root = allocation_test_directory("cli-incomplete-drain");
        let session = root.join("session");
        let output = root.join("output");
        let records = allocation_records();
        let end = records
            .iter()
            .position(
                |record| matches!(record, replay_capture_spool::spool::Record::Frame(frame) if frame.hour == 4),
            )
            .unwrap();
        write_campaign_journal(&session, &records[..end]);
        create_failure_witness(&session, 7, 13).unwrap();
        write_json(
            &session.join("status.json"),
            &json!({
                "session":7,"state":"pinned","spool_closed":true,"drain_complete":false,
                "error":"synthetic final spool sync failed","warning":"earlier write retried",
            }),
        )
        .unwrap();
        write_json(
            &session.join("completed.json"),
            &json!({
                "replays":[],"partial":true,"issues":["synthetic final spool sync failed"],
                "drain_complete":false,"warnings":[],
            }),
        )
        .unwrap();
        for loaded in [Some(true), Some(false)] {
            let terminal = terminal_outcome(&session, 7, 13, false, false, true);
            assert!(terminal.recovery_allowed);
            assert!(terminal.witness.is_none());
            assert!(!terminal.capture_failed);
            assert_eq!(terminal.drain_complete, Some(false));
            let error = finalize_cli_session(&session, &output, terminal, || CollectorPresence {
                process_exited: false,
                collector_loaded: loaded,
            })
            .unwrap_err();
            assert!(
                error.contains("partial") && error.contains("synthetic final spool sync failed")
            );
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["partial"], true);
            assert_eq!(completed["drain_complete"], false);
            assert_eq!(completed["capture_failed"], false);
            assert_eq!(completed["failure_code"], Value::Null);
            assert!(completed["issues"].as_array().unwrap().iter().any(|issue| {
                issue
                    .as_str()
                    .unwrap()
                    .contains("final queue drain or spool sync was incomplete")
            },));
            assert!(
                completed["warnings"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("earlier write retried"))
            );
            assert_eq!(
                completed["warnings"]
                    .as_array()
                    .unwrap()
                    .contains(&json!(COLLECTOR_UNLOAD_WARNING)),
                loaded != Some(false)
            );
            assert!(
                completed_detach(&completed)
                    .unwrap_err()
                    .contains("synthetic final spool sync failed")
            );
            let paths = completed["replays"].as_array().unwrap();
            assert_eq!(paths.len(), 1);
            assert_eq!(
                published_hours(Path::new(paths[0].as_str().unwrap())),
                [(9, 2), (9, 3)]
            );
        }
    }

    #[test]
    fn completion_writers_serialize_both_orders_without_resurrecting_waiting_campaigns() {
        use crate::test_support::allocation_test_directory;
        use std::sync::mpsc;

        #[derive(Clone, Copy, Debug)]
        enum Writer {
            Unload,
            Unlist,
            Cli,
        }
        #[derive(Debug, PartialEq, Eq)]
        enum Event {
            Read,
            Blocked,
            Finished,
        }

        fn write(writer: Writer, session: &Path, target: &Path) -> Result<(), String> {
            let presence = CollectorPresence {
                process_exited: false,
                collector_loaded: Some(false),
            };
            match writer {
                Writer::Unload => refresh_unload_completion(session, presence).map(|_| ()),
                Writer::Unlist => {
                    let _lock = finalization_lock(target)?;
                    unlist_waiting(session, "A")
                }
                Writer::Cli => {
                    let _lock = finalization_lock(target)?;
                    complete_cli_finalization(
                        session,
                        target,
                        &replay_io::Finalized {
                            replays: vec![target.join("saved.battleplan")],
                            saved_campaigns: BTreeSet::from(["A".to_owned()]),
                            ..replay_io::Finalized::default()
                        },
                        json!({"cli_updated":true}),
                        || presence,
                    )
                }
            }
        }

        for (first, second) in [
            (Writer::Unload, Writer::Unlist),
            (Writer::Unlist, Writer::Unload),
            (Writer::Unload, Writer::Cli),
            (Writer::Cli, Writer::Unload),
        ] {
            let root =
                allocation_test_directory(&format!("completion-writers-{first:?}-{second:?}"));
            let session = root.join("session");
            let target = root.join("output");
            fs::create_dir_all(&session).unwrap();
            fs::create_dir_all(&target).unwrap();
            write_json(
                &session.join("completed.json"),
                &json!({
                    "target":target,"waiting":[{"campaign":"A"},{"campaign":"B"}],
                    "partial":false,"issues":[],"warnings":["historical warning"],
                    "unload_pending":true,"collector_loaded":true,"detached":false,
                }),
            )
            .unwrap();
            let (read_tx, read_rx) = mpsc::channel();
            let (release_first_tx, release_first_rx) = mpsc::channel();
            let (event_tx, event_rx) = mpsc::channel();
            let (release_second_tx, release_second_rx) = mpsc::channel();
            std::thread::scope(|scope| {
                let session_ref = &session;
                let target_ref = &target;
                let first_thread = scope.spawn(move || {
                    let read = Cell::new(false);
                    COMPLETION_STEP.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |path, step| {
                            if step == CompletionStep::Read
                                && path.file_name() == Some(OsStr::new("completed.json"))
                                && !read.replace(true)
                            {
                                read_tx.send(Event::Read).unwrap();
                                release_first_rx.recv().unwrap();
                            }
                        }))
                    });
                    write(first, session_ref, target_ref)
                });
                assert_eq!(read_rx.recv().unwrap(), Event::Read);
                let second_thread = scope.spawn(move || {
                    let blocked = Cell::new(false);
                    let hook_tx = event_tx.clone();
                    COMPLETION_STEP.with(|hook| {
                        *hook.borrow_mut() = Some(Box::new(move |path, step| {
                            if step == CompletionStep::Blocked
                                && path.file_name() == Some(OsStr::new("completion.lock"))
                                && !blocked.replace(true)
                            {
                                hook_tx.send(Event::Blocked).unwrap();
                                release_second_rx.recv().unwrap();
                            }
                        }))
                    });
                    let result = write(second, session_ref, target_ref);
                    event_tx.send(Event::Finished).unwrap();
                    result
                });
                let event = event_rx.recv().unwrap();
                release_first_tx.send(()).unwrap();
                first_thread.join().unwrap().unwrap();
                let _ = release_second_tx.send(());
                second_thread.join().unwrap().unwrap();
                assert_eq!(event, Event::Blocked);
            });
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["waiting"], json!([{"campaign":"B"}]));
            assert_eq!(completed["collector_loaded"], false);
            assert_eq!(completed["detached"], true);
            assert_eq!(completed["unload_pending"], false);
            assert_eq!(completed["warnings"], json!(["historical warning"]));
            if matches!(first, Writer::Cli) || matches!(second, Writer::Cli) {
                assert_eq!(completed["cli_updated"], true);
                assert_eq!(
                    completed["replays"],
                    json!([target.join("saved.battleplan")])
                );
            }
        }
    }

    #[test]
    fn saved_notification_and_diagnostics_precede_deferred_unload() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        for loaded in [true, false] {
            let root = allocation_test_directory(&format!("save-ready-before-unload-{loaded}"));
            let directory = root.join("session");
            let target = root.join("output");
            let records = allocation_records();
            let replay_capture_spool::spool::Record::SessionStart(session) = &records[0] else {
                unreachable!()
            };
            write_campaign_journal(&directory, &records);
            let finalized = replay_io::finalize_session_outcome(&directory, &target).unwrap();
            assert!(finalized.errors.is_empty());
            let active = root.join("active.json");
            write_json(&active, &json!({"session_dir":directory})).unwrap();
            let elapsed = Cell::new(Duration::from_secs(25));
            let shown = Cell::new(false);
            let pending = RefCell::new(None);
            let result = finish_capture(
                &directory, &target, &active, session, &directory.join("bin/collector.dll"), false,
                json!({"replays":finalized.replays,"partial":false,"issues":[],"drain_complete":true}),
                Ok(CaptureOutcome { replays:finalized.replays.clone(), ..CaptureOutcome::default() }),
                &|stage| match stage {
                    Stage::Finished { result } => {
                        assert!(result.is_ok());
                        assert!(!active.exists());
                        for path in &result.unwrap().replays {
                            assert_eq!(published_hours(path), [(9, 2), (9, 3), (9, 4)]);
                        }
                        let completed = read_status(&directory.join("completed.json")).unwrap();
                        assert_eq!(completed["unload_pending"], true);
                        assert_eq!(completed["warnings"], json!([]));
                        assert!(require_collector_absent(completed["collector_loaded"].as_bool()).is_err());
                        elapsed.set(elapsed.get() + Duration::from_secs(2));
                        shown.set(true);
                    }
                    Stage::Unloading(observation) => {
                        assert!(shown.get());
                        let status = read_status(&directory.join("finalization-status.json")).unwrap();
                        assert_eq!(status["stop_to_save_ready_s"], 27);
                        pending.replace(Some(observation));
                    }
                    _ => unreachable!(),
                },
                || {
                    assert!(shown.get());
                    Some(json!({"save_ready":true,"stop_to_save_ready_s":elapsed.get().as_secs()}))
                },
            ).unwrap();
            assert_eq!(result.replays, finalized.replays);
            assert_eq!(elapsed.get(), Duration::from_secs(27));
            let started = Instant::now();
            let observation_started = elapsed.get();
            let presence = wait_collector_absence(
                |_| CollectorPresence {
                    process_exited: false,
                    collector_loaded: Some(loaded),
                },
                || started + (elapsed.get() - observation_started),
                |delay| elapsed.set(elapsed.get() + delay),
            );
            let update = pending.into_inner().unwrap().finish_with(presence);
            assert_eq!(update.is_err(), loaded);
            assert_eq!(
                elapsed.get(),
                Duration::from_secs(if loaded { 37 } else { 27 })
            );
            let completed = read_status(&directory.join("completed.json")).unwrap();
            assert_eq!(completed["partial"], false);
            assert_eq!(completed["unload_pending"], false);
            assert_eq!(completed["collector_loaded"], loaded);
            assert_eq!(
                completed["warnings"],
                if loaded {
                    json!([COLLECTOR_UNLOAD_WARNING])
                } else {
                    json!([])
                }
            );
            let status = read_status(&directory.join("finalization-status.json")).unwrap();
            assert_eq!(status["stop_to_save_ready_s"], 27);
        }
    }

    #[test]
    fn unload_followup_preserves_later_completion_and_does_not_reconcile_checkpoints() {
        use crate::test_support::allocation_test_directory;
        let directory = allocation_test_directory("unload-followup-completion");
        let campaign = directory.join("00000000-0000-4000-8000-000000000000");
        fs::create_dir_all(&campaign).unwrap();
        let proof = campaign.join("checkpoint.json");
        fs::write(&proof, b"synthetic journal owned by another finalizer").unwrap();
        let completed = json!({
            "partial":true,"issues":["later finalization failed"],
            "replays":["later publication"],"waiting":["later waiting campaign"],
            "warnings":["historical warning"],"unload_pending":true,
        });
        write_json(&directory.join("completed.json"), &completed).unwrap();
        refresh_unload_completion(
            &directory,
            CollectorPresence {
                process_exited: false,
                collector_loaded: Some(false),
            },
        )
        .unwrap();
        let updated = read_status(&directory.join("completed.json")).unwrap();
        for key in ["partial", "issues", "replays", "waiting", "warnings"] {
            assert_eq!(updated[key], completed[key]);
        }
        assert_eq!(updated["unload_pending"], false);
        assert_eq!(updated["detached"], true);
        assert_eq!(
            fs::read(proof).unwrap(),
            b"synthetic journal owned by another finalizer"
        );
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::write(
            directory.join(COLLECTOR_RESIDENT),
            "collector stays loaded: 16 of its threads are still running",
        )
        .unwrap();
        for _ in 0..2 {
            refresh_unload_completion(
                &directory,
                CollectorPresence {
                    process_exited: false,
                    collector_loaded: Some(true),
                },
            )
            .unwrap();
        }
        let updated = read_status(&directory.join("completed.json")).unwrap();
        assert_eq!(
            updated["warnings"],
            json!([
                "historical warning",
                "collector stays loaded: 16 of its threads are still running; quit the game before recording again"
            ])
        );
    }

    #[test]
    fn a_discard_after_a_failed_capture_leaves_the_recording_as_it_was_and_keeps_only_a_failure_report() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, RELOAD, allocation_records, allocation_test_directory,
            allocation_witness, load_segment_records, write_campaign_journal, write_capture,
        };
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for failure in ["sync", "allocation", "clean"] {
            for exited in [false, true] {
                let root =
                    allocation_test_directory(&format!("discard-terminal-{failure}-{exited}"));
                let target = root.join("output");
                let settings = root.join("settings.json");
                let naming = replay_io::Naming {
                    localisation: None,
                    settings: &settings,
                    report: None,
                };
                let first = root.join("sessions").join("1");
                write_campaign_journal(&first, &allocation_records());
                let seeded = replay_io::finalize_session_ahead(
                    &first,
                    &target,
                    &|| Ok(()),
                    replay_io::Live::default(),
                    naming,
                    &|_| replay_io::Destination::Recording,
                )
                .unwrap();
                let recording = seeded.replays[0].clone();
                let seed = fs::read(&recording).unwrap();
                let directory = root.join("sessions").join("2");
                let records = load_segment_records(10, 5..=6, RELOAD);
                let replay_capture_spool::spool::Record::SessionStart(session) = &records[0] else {
                    unreachable!()
                };
                write_capture(&directory.join(campaign), 0, &records, true);
                let mut live = replay_io::Live::default();
                live.advance(&directory, &target, &|_| true, naming, &|| false)
                    .unwrap();
                assert_ne!(fs::read(&recording).unwrap(), seed);
                create_failure_witness(&directory, 7, 13).unwrap();
                if failure == "allocation" {
                    fs::write(
                        directory.join(WITNESS_FILE_NAME),
                        allocation_witness(0).encode(),
                    )
                    .unwrap();
                }
                let witness = fs::read(directory.join(WITNESS_FILE_NAME)).unwrap();
                fs::write(directory.join("progress"), b"synthetic done").unwrap();
                fs::write(directory.join("recorder.log"), b"synthetic hook log").unwrap();
                fs::write(directory.join("content.json"), b"{}").unwrap();
                fs::write(directory.join("facts.bin"), b"synthetic facts").unwrap();
                write_json(
                    &directory.join("status.json"),
                    &json!({
                        "session":7,"state":"pinned","spool_closed":true,
                        "drain_complete":failure=="clean",
                        "error":(failure=="sync").then_some("synthetic final spool sync failed"),
                    }),
                )
                .unwrap();
                let terminal = terminal_outcome(&directory, 7, 13, true, exited, true);
                let partial = failure != "clean";
                assert_eq!(!terminal.issues.is_empty(), partial);
                let (dropped, others_keep) = dropped_by_discard(
                    &directory,
                    &target,
                    &live,
                    Some(campaign),
                    false,
                    &mut Vec::new(),
                )
                .unwrap();
                assert!(!others_keep);
                assert_eq!(dropped, [campaign]);
                note_discards(&directory, &target, &dropped).unwrap();
                discard_dropped(&directory, &target, &mut live, &dropped).unwrap();
                let (completed, result) = discarded_end(
                    &directory,
                    &target,
                    &terminal,
                    true,
                    Some(campaign),
                    CaptureOutcome::default(),
                );
                assert_eq!(completed["discarded_campaign"], campaign);
                assert_eq!(completed["drain_complete"], failure == "clean");
                assert_eq!(completed["capture_failed"], failure == "allocation");
                assert_eq!(
                    completed["failure_code"],
                    if failure == "allocation" {
                        json!("allocation_failed")
                    } else {
                        Value::Null
                    }
                );
                assert_eq!(
                    completed["failed_hour"],
                    if failure == "allocation" {
                        json!(99)
                    } else {
                        Value::Null
                    }
                );
                let reported = result.as_ref().unwrap().failure_report.clone();
                assert!(result.as_ref().unwrap().discarded);
                assert_eq!(reported.is_some(), partial);
                let pending = RefCell::new(None);
                let result = finish_capture(
                    &directory,
                    &target,
                    &root.join("active.json"),
                    session,
                    &directory.join("bin/collector.dll"),
                    exited,
                    completed,
                    result,
                    &|stage| match stage {
                        Stage::Finished { result } => assert!(result.is_ok()),
                        Stage::Unloading(observation) => {
                            pending.replace(Some(observation));
                        }
                        _ => unreachable!(),
                    },
                    || None,
                );
                assert!(result.is_ok());
                pending
                    .into_inner()
                    .unwrap()
                    .finish_with(if exited {
                        CollectorPresence::exited()
                    } else {
                        CollectorPresence {
                            process_exited: false,
                            collector_loaded: Some(false),
                        }
                    })
                    .unwrap();
                assert_eq!(
                    fs::read(&recording).unwrap(),
                    seed,
                    "the recording is as it was before the session, to the byte"
                );
                let left: Vec<_> = fs::read_dir(&directory)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                    .collect();
                assert!(
                    left.iter()
                        .all(|name| name == "completed.json" || name == "completion.lock"),
                    "only the completion record is left: {left:?}"
                );
                let completed = read_status(&directory.join("completed.json")).unwrap();
                assert_eq!(completed["discarded"], true);
                assert_eq!(completed["partial"], partial);
                assert_eq!(completed["issues"], json!(terminal.issues));
                let reports = target.join(FAILURE_REPORTS);
                if !partial {
                    assert!(!reports.exists());
                    continue;
                }
                let report = reported.unwrap();
                assert_eq!(report, reports.join("2"));
                assert_eq!(completed["failure_report"], json!(report));
                let mut kept: Vec<_> = fs::read_dir(&report)
                    .unwrap()
                    .map(|entry| {
                        let entry = entry.unwrap();
                        assert!(entry.file_type().unwrap().is_file());
                        entry.file_name().into_string().unwrap()
                    })
                    .collect();
                kept.sort();
                assert_eq!(
                    kept,
                    [
                        "collector-failure.bin",
                        "completed.json",
                        "content.json",
                        "progress",
                        "recorder.log",
                        "status.json",
                    ],
                    "the diagnostics, and no recording, journal, spool or game facts"
                );
                assert_eq!(fs::read(report.join(WITNESS_FILE_NAME)).unwrap(), witness);
                assert_eq!(
                    read_status(&report.join("completed.json")).unwrap()["issues"],
                    json!(terminal.issues)
                );
            }
        }
    }

    #[test]
    fn failure_reports_keep_only_the_newest_five() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("failure-reports-bound");
        let target = root.join("output");
        let reports = target.join(FAILURE_REPORTS);
        let written = SystemTime::now() - Duration::from_secs(3_600);
        for (at, name) in ["a", "b", "c", "d", "e"].into_iter().enumerate() {
            let report = reports.join(name);
            fs::create_dir_all(&report).unwrap();
            let completed = report.join("completed.json");
            fs::write(&completed, b"{}").unwrap();
            fs::File::options()
                .write(true)
                .open(&completed)
                .unwrap()
                .set_modified(written + Duration::from_secs(60 * at as u64))
                .unwrap();
        }
        let session = root.join("sessions").join("f");
        fs::create_dir_all(&session).unwrap();
        write_json(&session.join("status.json"), &json!({"session":7})).unwrap();
        let report = keep_failure_report(&session, &target, &json!({"discarded":true})).unwrap();
        assert_eq!(report, reports.join("f"));
        let mut kept: Vec<_> = fs::read_dir(&reports)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        kept.sort();
        assert_eq!(kept, ["b", "c", "d", "e", "f"], "the oldest went");
        assert!(report.join("status.json").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn fully_drained_pinned_collector_finalizes_with_warnings_and_blocks_attach() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        let root = allocation_test_directory("restoration-only-warning");
        let session = root.join("session");
        let output = root.join("output");
        write_campaign_journal(&session, &allocation_records());
        create_failure_witness(&session, 7, 13).unwrap();
        write_json(
            &session.join("status.json"),
            &json!({
                "session":7,"state":"pinned","spool_closed":true,"drain_complete":true,
                "warning":"earlier write retried","restoration_error":"synthetic hook stayed busy",
                "collector_pinned":true,"recording_stopped":true,
            }),
        )
        .unwrap();
        let terminal = terminal_outcome(&session, 7, 13, true, false, true);
        assert_eq!(terminal.drain_complete, Some(true));
        assert!(terminal.issues.is_empty());
        assert!(terminal.witness.is_none());
        assert_eq!(
            terminal.warnings,
            ["earlier write retried", "synthetic hook stayed busy"]
        );
        finalize_cli_session(&session, &output, terminal, || CollectorPresence {
            process_exited: false,
            collector_loaded: Some(true),
        })
        .unwrap();
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["partial"], false);
        assert_eq!(completed["issues"], json!([]));
        assert_eq!(completed["drain_complete"], true);
        assert_eq!(completed["detached"], false);
        assert_eq!(
            completed["warnings"],
            json!([
                "earlier write retried",
                "synthetic hook stayed busy",
                COLLECTOR_UNLOAD_WARNING,
            ])
        );
        assert!(require_collector_absent(completed["collector_loaded"].as_bool()).is_err());
        let paths = completed["replays"].as_array().unwrap();
        assert_eq!(paths.len(), 1);
        assert_eq!(
            published_hours(Path::new(paths[0].as_str().unwrap())),
            [(9, 2), (9, 3), (9, 4)]
        );
    }

    #[test]
    fn healthy_stop_waits_for_unload_only_after_publishing_its_replays() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        for empty in [false, true] {
            let root = allocation_test_directory(&format!("healthy-stop-unload-{empty}"));
            let session = root.join("session");
            let output = root.join("output");
            fs::create_dir_all(&session).unwrap();
            let finalized = if empty {
                replay_io::Finalized::default()
            } else {
                write_campaign_journal(&session, &allocation_records());
                replay_io::finalize_session_outcome(&session, &output).unwrap()
            };
            assert!(finalized.errors.is_empty());
            assert_eq!(finalized.replays.is_empty(), empty);
            let started = Instant::now();
            let elapsed = Cell::new(Duration::ZERO);
            let polls = Cell::new(0);
            complete_cli_finalization(
                &session,
                &output,
                &finalized,
                json!({"partial":false,"issues":[],"detach_requested":true,"drain_complete":true}),
                || {
                    wait_collector_absence(
                        |_| {
                            polls.set(polls.get() + 1);
                            for replay in &finalized.replays {
                                assert!(replay.is_file());
                                assert_eq!(published_hours(replay), [(9, 2), (9, 3), (9, 4)]);
                            }
                            assert!(!session.join("completed.json").exists());
                            CollectorPresence {
                                process_exited: false,
                                collector_loaded: Some(elapsed.get() < Duration::from_secs(3)),
                            }
                        },
                        || started + elapsed.get(),
                        |delay| elapsed.set(elapsed.get() + delay),
                    )
                },
            )
            .unwrap();
            assert_eq!(elapsed.get(), Duration::from_secs(3));
            assert_eq!(polls.get(), 31);
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["detached"], true);
            assert_eq!(completed["partial"], false);
            assert_eq!(completed["warnings"], json!([]));
            assert!(completed_detach(&completed).unwrap());
        }
    }

    #[test]
    fn unload_warning_is_reported_only_after_the_bounded_wait_expires() {
        use crate::test_support::allocation_test_directory;
        for loaded in [Some(true), None] {
            let root = allocation_test_directory(&format!("unload-wait-expired-{loaded:?}"));
            let session = root.join("session");
            let output = root.join("output");
            fs::create_dir_all(&session).unwrap();
            let started = Instant::now();
            let elapsed = Cell::new(Duration::ZERO);
            let polls = Cell::new(0);
            let presence = write_completion(
                &session,
                &output,
                json!({"partial":false,"issues":[],"detach_requested":true,"drain_complete":true}),
                || {
                    wait_collector_absence(
                        |_| {
                            polls.set(polls.get() + 1);
                            assert!(!session.join("completed.json").exists());
                            CollectorPresence {
                                process_exited: false,
                                collector_loaded: loaded,
                            }
                        },
                        || started + elapsed.get(),
                        |delay| {
                            assert!(delay <= UNLOAD_POLL && !delay.is_zero());
                            elapsed.set(elapsed.get() + delay);
                        },
                    )
                },
            )
            .unwrap();
            assert_eq!(elapsed.get(), UNLOAD_WAIT);
            assert_eq!(polls.get(), 100);
            assert_eq!(presence.collector_loaded, loaded);
            assert!(require_collector_absent(presence.collector_loaded).is_err());
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["detached"], false);
            assert_eq!(completed["partial"], false);
            assert_eq!(completed["issues"], json!([]));
            assert_eq!(completed["warnings"], json!([COLLECTOR_UNLOAD_WARNING]));
        }
    }

    #[test]
    fn unload_observation_uses_the_remaining_deadline_and_still_writes_completion() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("unload-observation-deadline");
        let session = root.join("session");
        let output = root.join("output");
        fs::create_dir_all(&session).unwrap();
        let started = Instant::now();
        let elapsed = Cell::new(Duration::ZERO);
        let polls = Cell::new(0);
        let presence = write_completion(
            &session,
            &output,
            json!({"partial":false,"issues":[],"drain_complete":true}),
            || {
                wait_collector_absence(
                    |deadline| {
                        assert_eq!(deadline, started + UNLOAD_WAIT);
                        assert!(elapsed.get() < UNLOAD_WAIT);
                        polls.set(polls.get() + 1);
                        let remaining = deadline.duration_since(started + elapsed.get());
                        elapsed.set(elapsed.get() + remaining.min(Duration::from_secs(3)));
                        CollectorPresence {
                            process_exited: false,
                            collector_loaded: None,
                        }
                    },
                    || started + elapsed.get(),
                    |delay| elapsed.set(elapsed.get() + delay),
                )
            },
        )
        .unwrap();
        assert_eq!(elapsed.get(), UNLOAD_WAIT);
        assert_eq!(polls.get(), 4);
        assert!(require_collector_absent(presence.collector_loaded).is_err());
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["collector_loaded"], Value::Null);
        assert_eq!(completed["partial"], false);
        assert_eq!(completed["warnings"], json!([COLLECTOR_UNLOAD_WARNING]));
    }

    #[test]
    fn unload_wait_returns_immediately_for_absence_or_process_exit() {
        for presence in [
            CollectorPresence::exited(),
            CollectorPresence {
                process_exited: false,
                collector_loaded: Some(false),
            },
        ] {
            let polls = Cell::new(0);
            let result = wait_collector_absence(
                |_| {
                    polls.set(polls.get() + 1);
                    presence
                },
                Instant::now,
                |_| panic!("confirmed absence must not wait"),
            );
            assert_eq!(polls.get(), 1);
            assert_eq!(result.process_exited, presence.process_exited);
            assert_eq!(result.collector_loaded, Some(false));
        }
    }

    #[test]
    fn completion_refreshes_slow_unload_after_finalization_without_a_partial_recording() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        let root = allocation_test_directory("completion-slow-unload");
        let session = root.join("session");
        let output = root.join("output");
        write_campaign_journal(&session, &allocation_records());
        let loaded = Cell::new(Some(true));
        let finalized = replay_io::finalize_session_outcome(&session, &output).unwrap();
        assert!(loaded.get().unwrap());
        loaded.set(Some(false));
        complete_cli_finalization(
            &session, &output, &finalized,
            json!({"partial":false,"issues":[],"warnings":[],"detach_requested":true,"drain_complete":true}),
            || CollectorPresence { process_exited: false, collector_loaded: loaded.get() },
        ).unwrap();
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["detached"], true);
        assert_eq!(completed["collector_loaded"], false);
        assert_eq!(completed["process_exited"], false);
        assert_eq!(completed["partial"], false);
        assert_eq!(completed["issues"], json!([]));
        assert_eq!(completed["warnings"], json!([]));
        assert!(completed_detach(&completed).unwrap());
        assert!(require_collector_absent(loaded.get()).is_ok());
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
    }

    #[test]
    fn a_failed_campaign_waits_only_with_the_hours_its_recording_holds_unsaved() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
            write_campaign_journal,
        };
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for (saved, expected) in [
            (true, json!([])),
            (
                false,
                json!([{"campaign":campaign,"hours":3,"first_hour":2,"last_hour":4}]),
            ),
        ] {
            let root = allocation_test_directory(&format!("failed-campaign-waits-{saved}"));
            let (session, output) = (root.join("session"), root.join("output"));
            let settings = root.join("settings.json");
            write_campaign_journal(&session, &allocation_records());
            let destination = if saved {
                replay_io::Destination::Recording
            } else {
                replay_io::Destination::Unsaved
            };
            let ended = replay_io::finalize_session_ahead(
                &session,
                &output,
                &|| Ok(()),
                replay_io::Live::default(),
                replay_io::Naming {
                    localisation: None,
                    settings: &settings,
                    report: None,
                },
                &|_| destination,
            )
            .unwrap();
            assert!(ended.errors.is_empty(), "{:?}", ended.errors);
            complete_cli_finalization(
                &session,
                &output,
                &replay_io::Finalized {
                    failed_campaigns: BTreeSet::from([campaign.to_owned()]),
                    ..replay_io::Finalized::default()
                },
                json!({"partial":true,"issues":["its end failed"]}),
                CollectorPresence::exited,
            )
            .unwrap();
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["waiting"], expected);
        }
    }

    #[test]
    fn unconfirmed_unload_warns_without_making_saved_data_partial_or_allowing_attach() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_journal,
        };
        for loaded in [Some(true), None] {
            let root = allocation_test_directory(&format!("completion-unconfirmed-{loaded:?}"));
            let session = root.join("session");
            let output = root.join("output");
            write_campaign_journal(&session, &allocation_records());
            finalize_cli_session(
                &session,
                &output,
                terminal_outcome(&session, 7, 13, false, true, true),
                || CollectorPresence {
                    process_exited: false,
                    collector_loaded: loaded,
                },
            )
            .unwrap();
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["partial"], false);
            assert_eq!(completed["issues"], json!([]));
            assert_eq!(completed["warnings"], json!([COLLECTOR_UNLOAD_WARNING]));
            assert_eq!(completed["detached"], false);
            assert_eq!(completed["collector_loaded"], json!(loaded));
            assert_eq!(completed["process_exited"], false);
            assert!(
                completed_detach(&completed)
                    .unwrap_err()
                    .contains("unload is unconfirmed")
            );
            assert!(require_collector_absent(loaded).is_err());
            let paths = completed["replays"].as_array().unwrap();
            assert_eq!(paths.len(), 1);
            assert_eq!(
                published_hours(Path::new(paths[0].as_str().unwrap())),
                [(9, 2), (9, 3), (9, 4)]
            );
            finalize_cli_session(
                &session,
                &output,
                terminal_outcome(&session, 7, 13, false, true, true),
                || CollectorPresence {
                    process_exited: false,
                    collector_loaded: Some(false),
                },
            )
            .unwrap();
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["partial"], false);
            assert_eq!(completed["warnings"], json!([]));
            assert_eq!(completed["detached"], true);
        }
    }

    #[test]
    fn completion_refreshes_process_exit_without_claiming_a_live_detach() {
        use crate::test_support::allocation_test_directory;
        let root = allocation_test_directory("completion-process-exit");
        let session = root.join("session");
        let output = root.join("output");
        fs::create_dir_all(&session).unwrap();
        write_completion(
            &session,
            &output,
            json!({"partial":false,"issues":[],"detach_requested":true,"drain_complete":true}),
            CollectorPresence::exited,
        )
        .unwrap();
        let completed = read_status(&session.join("completed.json")).unwrap();
        assert_eq!(completed["process_exited"], true);
        assert_eq!(completed["collector_loaded"], false);
        assert_eq!(completed["detached"], false);
        assert_eq!(completed["partial"], false);
        assert_eq!(completed["warnings"], json!([]));
        assert!(!completed_detach(&completed).unwrap());
    }

    #[test]
    fn allocation_failure_missing_invalid_or_foreign_witness_never_reports_success() {
        use crate::test_support::{allocation_test_directory, allocation_witness};
        let directory = allocation_test_directory("terminal-refusals");
        write_json(
            &directory.join("status.json"),
            &json!({
                "session":7,"state":"detached","drain_complete":true,"capture_failed":true,
            }),
        )
        .unwrap();
        let missing = terminal_outcome(&directory, 7, 13, true, false, true);
        assert!(!missing.recovery_allowed);
        assert!(!missing.issues.is_empty());
        fs::create_dir_all(directory.join("bin")).unwrap();
        fs::write(directory.join("bin/collector.json"), b"configured").unwrap();
        write_json(
            &directory.join("status.json"),
            &json!({"session":7,"state":"detached","drain_complete":true}),
        )
        .unwrap();
        let missing = terminal_outcome(&directory, 7, 13, true, false, true);
        assert!(!missing.recovery_allowed);
        assert!(
            missing
                .issues
                .iter()
                .any(|issue| issue.contains("precreated terminal witness is missing"))
        );
        let witness = allocation_witness(1);
        fs::write(directory.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        let foreign = terminal_outcome(&directory, 7, 14, true, false, true);
        assert!(!foreign.recovery_allowed);
        assert!(foreign.witness.is_none());
        let mut invalid = witness.encode();
        invalid[31] ^= 1;
        fs::write(directory.join(WITNESS_FILE_NAME), invalid).unwrap();
        let invalid = terminal_outcome(&directory, 7, 13, true, false, true);
        assert!(!invalid.recovery_allowed);
        assert!(invalid.witness.is_none());
    }

    #[test]
    fn refresh_request_is_renamed_whole_and_acknowledged() {
        let root = std::env::temp_dir().join(format!("hoi4-refresh-cli-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let session = root.join("session");
        fs::create_dir_all(&session).unwrap();
        fs::write(
            root.join("active.json"),
            json!({"session_dir": session,"session_id":7,"process_start_time":13}).to_string(),
        )
        .unwrap();
        let folder = session.join("refresh-requests");
        let published = || -> Vec<_> {
            fs::read_dir(&folder)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect()
        };
        let mut waits = 0;
        request_refresh_with(&root, |request, _| {
            waits += 1;
            assert_eq!(published(), [request.to_path_buf()]);
            assert!(request.extension().is_some_and(|extension| extension == "request"), "the collector reads only .request files");
            let token = fs::read_to_string(request).unwrap();
            assert_eq!(token, request.file_stem().unwrap().to_str().unwrap());
            if waits == 2 {
                fs::write(request.with_extension("ack"), &token).unwrap();
            }
            true
        })
        .unwrap();
        assert_eq!(waits, 2);
        assert!(published().is_empty());
        let mut refused = 0;
        let unacknowledged = request_refresh_with(&root, |request, _| {
            assert_eq!(published(), [request.to_path_buf()]);
            refused += 1;
            false
        });
        assert_eq!(refused, 1, "the refusal saw the published request");
        assert!(unacknowledged.unwrap_err().contains("did not acknowledge"));
        assert!(published().is_empty());
        assert!(request_refresh(&session).is_err());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn late_terminal_drain_errors_cannot_be_reported_as_clean_detach() {
        let status = json!({"state":"detached","session":7,"drain_complete":false,"error":"final spool sync failed"});
        let outcome = classify_terminal(
            7,
            true,
            false,
            Some(&status),
            Some("final spool sync failed\n"),
        );
        assert_eq!(outcome.drain_complete, Some(false));
        assert!(
            outcome
                .issues
                .iter()
                .any(|issue| issue.contains("final spool sync failed"))
        );
        assert_eq!(
            outcome
                .issues
                .iter()
                .filter(|issue| issue.contains("final spool sync failed"))
                .count(),
            1
        );
        let completed = json!({"partial":true,"issues":outcome.issues,"detach_requested":true,"detached":false,"process_exited":false,"drain_complete":false});
        assert!(
            completed_detach(&completed)
                .unwrap_err()
                .contains("final spool sync failed")
        );
    }

    #[test]
    fn historical_recovered_warnings_do_not_make_a_completed_drain_partial() {
        let status = json!({"state":"detached","session":7,"drain_complete":true,"warning":"segment write retried","last_error":"segment write retried","write_errors":1});
        let outcome = classify_terminal(7, true, false, Some(&status), None);
        assert_eq!(outcome.drain_complete, Some(true));
        assert!(outcome.issues.is_empty());
        assert_eq!(outcome.warnings, ["segment write retried"]);
        let completed = json!({"partial":false,"issues":[],"detach_requested":true,"detached":true,"process_exited":false,"drain_complete":true});
        assert!(completed_detach(&completed).unwrap());
        let late_file = classify_terminal(
            7,
            true,
            false,
            Some(&status),
            Some("cannot publish terminal status"),
        );
        assert!(
            late_file
                .issues
                .iter()
                .any(|issue| issue.contains("cannot publish terminal status"))
        );
    }

    #[test]
    fn process_exit_recovery_does_not_require_a_detach_record_or_terminal_drain() {
        let running = json!({"state":"recording","session":7,"last_hour":100,"warning":"earlier segment retried"});
        assert!(
            classify_terminal(7, false, true, Some(&running), None)
                .issues
                .is_empty()
        );
        assert!(
            classify_terminal(7, false, true, None, None)
                .issues
                .is_empty()
        );
        let unavailable_status = classify_terminal(7, false, true, Some(&json!({})), None);
        assert!(unavailable_status.issues.is_empty());
        assert!(!unavailable_status.warnings.is_empty());
        let interrupted = classify_terminal(7, true, true, Some(&running), None);
        assert!(
            interrupted
                .issues
                .iter()
                .any(|issue| issue.contains("game exited after detach was requested"))
        );
        let completed = json!({"partial":false,"issues":[],"detach_requested":false,"detached":false,"process_exited":true,"drain_complete":null});
        assert!(!completed_detach(&completed).unwrap());
    }

    #[test]
    fn ctrl_c_quits_the_command_line_recorder_after_a_capture_sealed_its_ending_and_between_games() {
        let console = Console::new();
        let first = console.next_capture().expect("nothing asked to quit yet");
        assert_eq!(first.seal(), None, "the game exited by itself");
        console.interrupt();
        assert!(console.quitting(), "Ctrl+C during finalization is kept for after it");
        assert!(console.next_capture().is_none(), "and no next game is recorded");

        let console = Console::new();
        let first = console.next_capture().unwrap();
        console.interrupt();
        assert_eq!(first.seal(), Some(Stop::Save(None)), "Ctrl+C while recording saves the capture");
        assert!(console.next_capture().is_none());
    }

    #[test]
    fn control_keeps_the_first_stop_until_the_capture_seals_its_ending_and_resets_between_captures()
    {
        let control = Control::new();
        assert!(!control.stopping());
        assert!(control.stop(Stop::Save(None)));
        assert!(control.stop(Stop::Save(None)), "the same stop asked again stands");
        assert!(
            !control.stop(Stop::Discard("a".into())),
            "a later Discard can’t turn the save into one"
        );
        assert_eq!(control.seal(), Some(Stop::Save(None)));
        control.reset();
        assert!(!control.stopping());
        assert_eq!(control.seal(), None, "a capture that ends by itself keeps what it has");
        assert!(
            !control.stop(Stop::Discard("a".into())),
            "a stop after that is refused"
        );
        assert!(!control.stopping());
        control.reset();
        assert!(control.stop(Stop::Discard("a".into())));
        assert_eq!(control.seal(), Some(Stop::Discard("a".into())));
    }

    fn finalize_with(session: &Path, target: &Path, live: replay_io::Live) -> replay_io::Finalized {
        replay_io::finalize_session_ahead(session, target, &|| Ok(()), live, unnamed(), &|_| {
            replay_io::Destination::Recording
        })
        .unwrap()
    }

    fn published_hours(path: &Path) -> Vec<(replay_data::BranchId, replay_data::Hour)> {
        let replay = Replay::open(fs::read(path).unwrap()).unwrap();
        replay
            .index
            .iter()
            .flat_map(|block| replay.read_block(block).unwrap())
            .map(|frame| (frame.branch, frame.hour))
            .collect()
    }

    #[test]
    fn admission_asks_once_per_load_and_records_each_decision_on_disk() {
        use crate::test_support::allocation_test_directory;
        const KEPT: &str = "00000000-0000-4000-8000-000000000001";
        const REFUSED: &str = "00000000-0000-4000-8000-000000000002";
        let root = allocation_test_directory("admission-record");
        let session = root.join("session");
        let target = root.join("target");
        replay_io::begin_admissions(&session, &target).unwrap();
        let on_disk = || replay_io::session_admissions(&session).unwrap().unwrap();
        let calls = Cell::new(0);
        let refuse_reload = Cell::new(false);
        let admit = |campaign: &str, _recorded: bool| -> Result<(), Refusal> {
            calls.set(calls.get() + 1);
            if campaign == REFUSED {
                Err(Refusal::Off)
            } else if refuse_reload.get() {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let mut admission = Admission::new(&session, &target);
        assert_eq!(
            admission.check(KEPT, 1, || false, &admit),
            Ok(Decision::Admitted)
        );
        let first = BTreeMap::from([(KEPT.to_owned(), BTreeSet::from([1]))]);
        assert_eq!(on_disk().loads, first);
        assert_eq!(
            admission.check(REFUSED, 1, || false, &admit),
            Ok(Decision::Refused)
        );
        assert_eq!(
            admission.check(REFUSED, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(
            admission.check(KEPT, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(calls.get(), 2);
        refuse_reload.set(true);
        assert_eq!(
            admission.check(KEPT, 2, || true, &admit),
            Ok(Decision::Refused)
        );
        let record = on_disk();
        assert_eq!(record.loads, first);
        assert_eq!(
            record.refused,
            BTreeMap::from([
                ((REFUSED.to_owned(), 1), Refusal::Off),
                ((KEPT.to_owned(), 2), Refusal::Elsewhere),
            ])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_opening_load_is_admitted_before_the_collector_starts() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_journal,
        };
        let root = allocation_test_directory("opening-load");
        let session = root.join("session");
        let target = root.join("target");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &target).unwrap();
        let mut admission = Admission::new(&session, &target);
        admission.admit_opening(campaign, FIRST_LOAD).unwrap();
        assert_eq!(
            replay_io::session_admissions(&session)
                .unwrap()
                .unwrap()
                .loads,
            BTreeMap::from([(campaign.to_owned(), BTreeSet::from([FIRST_LOAD]))])
        );
        let ask = |_: &str, _: bool| -> Result<(), Refusal> { unreachable!() };
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || unreachable!(), &ask),
            Ok(Decision::Seen)
        );
        write_campaign_journal(&session, &allocation_records());
        let mut live = replay_io::Live::default();
        admission.check_journaled(&mut live, &ask).unwrap();
        let finalized = finalize_with(&session, &target, live);
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(published_hours(&finalized.replays[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_brief_load_never_probed_is_admitted_at_save_and_published() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_journal,
        };
        let root = allocation_test_directory("brief-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        write_campaign_journal(&session, &allocation_records());
        let asked = RefCell::new(Vec::new());
        let admit = |campaign: &str, recorded: bool| -> Result<(), Refusal> {
            asked.borrow_mut().push((campaign.to_owned(), recorded));
            Ok(())
        };
        let mut live = replay_io::Live::default();
        admission.check_journaled(&mut live, &admit).unwrap();
        assert_eq!(asked.into_inner(), [(campaign.to_owned(), false)]);
        assert_eq!(
            replay_io::session_admissions(&session)
                .unwrap()
                .unwrap()
                .loads,
            BTreeMap::from([(campaign.to_owned(), BTreeSet::from([FIRST_LOAD]))])
        );
        let finalized = finalize_with(&session, &output, live);
        assert!(finalized.errors.is_empty() && finalized.omitted.is_empty());
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_reload_that_ends_before_its_probe_is_asked_at_save_by_the_same_rule() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_journal,
        };
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for refusal in [Refusal::Off, Refusal::Elsewhere] {
            let root = allocation_test_directory(&format!("reload-at-save-{}", refusal.as_str()));
            let session = root.join("session");
            let output = root.join("replays");
            replay_io::begin_admissions(&session, &output).unwrap();
            let mut admission = Admission::new(&session, &output);
            admission.admit_opening(campaign, FIRST_LOAD).unwrap();
            write_campaign_journal(&session, &two_load_records());
            let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
                if recorded { Err(refusal) } else { Ok(()) }
            };
            let mut live = replay_io::Live::default();
            admission.check_journaled(&mut live, &admit).unwrap();
            let finalized = finalize_with(&session, &output, live);
            assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
            assert_eq!(
                published_hours(&finalized.replays[0]),
                [(9, 2), (9, 3), (9, 4)]
            );
            assert_eq!(
                finalized.omitted,
                [Omission {
                    campaign: campaign.to_owned(),
                    load: Some(RELOAD),
                    hours: 2,
                    refusal: Some(refusal),
                }]
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn save_time_decisions_follow_load_order_across_campaigns() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
            write_campaign_journal,
        };
        use replay_recording::control::ControlRecord;
        use replay_capture_spool::spool::Record;
        const LATER: &str = "00000000-0000-4000-8000-000000000001";
        let first = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        assert!(LATER < first, "the later campaign's UUID sorts first");
        let root = allocation_test_directory("load-order");
        let session = root.join("session");
        let output = root.join("replays");
        write_campaign_journal(&session, &allocation_records());
        let later: Vec<_> = allocation_records()
            .into_iter()
            .map(|mut record| {
                match &mut record {
                    Record::BranchStart(branch) => branch.id = 12,
                    Record::Frame(frame) => frame.branch = 12,
                    Record::Control(ControlRecord::ClockWitness {
                        qpc, load_signal, ..
                    }) => {
                        *qpc += 10_000;
                        *load_signal = Some(31);
                    }
                    _ => {}
                }
                record
            })
            .collect();
        crate::test_support::write_capture(&session.join(LATER), 0, &later, true);
        let chosen: RefCell<Option<String>> = RefCell::new(None);
        let admit = |campaign: &str, _: bool| -> Result<(), Refusal> {
            let mut chosen = chosen.borrow_mut();
            let pressed = chosen.get_or_insert_with(|| campaign.to_owned()).as_str() == campaign;
            if pressed { Ok(()) } else { Err(Refusal::Off) }
        };
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        let mut live = replay_io::Live::default();
        admission.check_journaled(&mut live, &admit).unwrap();
        assert_eq!(chosen.borrow().as_deref(), Some(first));
        assert_eq!(
            replay_io::first_loaded_campaign(&session).as_deref(),
            Some(first)
        );
        let finalized = finalize_with(&session, &output, live);
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        assert_eq!(
            finalized.omitted,
            [Omission {
                campaign: LATER.to_owned(),
                load: Some(31),
                hours: 3,
                refusal: Some(Refusal::Off),
            }]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_refused_reload_stays_unpublished_when_the_session_never_completes() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_journal,
        };
        let root = allocation_test_directory("refused-reload-unfinished");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session, campaign, load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        write_campaign_journal(&session, &two_load_records());
        let mut live = replay_io::Live::default();
        live.advance(&session, &output, &|_| true, unnamed(), &|| false).unwrap();
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!session.join("completed.json").exists());
        drop(live);
        let paths = replay_io::finalize_session(&session, &output).unwrap();
        assert_eq!(published_hours(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn new_file_chosen_after_the_first_load_waits_for_the_reload() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_journal,
        };
        let root = allocation_test_directory("new-file-after-first-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let new_file = Cell::new(false);
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if new_file.replace(false) && recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session, campaign, load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        new_file.set(true);
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Seen)
        );
        assert!(new_file.get());
        write_campaign_journal(&session, &two_load_records());
        let mut live = replay_io::Live::default();
        live.advance(&session, &output, &|_| true, unnamed(), &|| false).unwrap();
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!new_file.get());
        let finalized = finalize_with(&session, &output, live);
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_finalization_of_a_failed_session_publishes_exactly_its_admitted_loads() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, allocation_witness,
            two_load_records, write_campaign_journal,
        };
        let root = allocation_test_directory("failed-session-admissions");
        let session = root.join("session");
        let output = root.join("replays");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        write_campaign_journal(&session, &two_load_records());
        let mut witness = allocation_witness(0);
        witness.branch = Some(10);
        witness.load_signal = Some(RELOAD);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        replay_io::Admissions::default()
            .admit(&session, &output, name, FIRST_LOAD)
            .unwrap();
        let recording = output.join("Campaign.battleplan");
        let mut published: Option<Vec<u8>> = None;
        for _ in 0..2 {
            let error = finalize_cli_session(
                &session,
                &output,
                terminal_outcome(&session, 7, 13, false, true, true),
                CollectorPresence::exited,
            )
            .unwrap_err();
            assert!(error.contains("partial"), "{error}");
            assert_eq!(published_hours(&recording), [(9, 2), (9, 3), (9, 4)]);
            let bytes = fs::read(&recording).unwrap();
            assert!(
                published
                    .replace(bytes.clone())
                    .is_none_or(|previous| previous == bytes)
            );
            let completed: Value =
                serde_json::from_slice(&fs::read(session.join("completed.json")).unwrap()).unwrap();
            assert_eq!(
                completed["omitted"],
                json!([{"campaign": name, "load": RELOAD, "hours": 2, "reason": "not_admitted"}])
            );
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_command_line_reads_status_once_a_minute_and_the_tray_every_two_seconds() {
        let start = Instant::now();
        let reads = |live| {
            let mut schedule = StatusReads::new(live, start);
            let (mut printed, mut live_reads) = (0, 0);
            for step in 0..1_200 {
                let now = start + Duration::from_millis(100 * step);
                printed += u32::from(schedule.print_due(now));
                if schedule.live_due(now) {
                    live_reads += 1;
                    schedule.live_read(now);
                }
            }
            (printed, live_reads)
        };
        assert_eq!(reads(false), (2, 0));
        assert_eq!(reads(true), (2, 60));
        let options = parse_options("record", vec!["--out".into(), "captures".into()]).unwrap();
        let user = PathBuf::from("user");
        let capture = cli_capture(&options, &user);
        assert!(!capture.live_status);
        assert!(capture.admit.is_none() && capture.loaded.is_none());
        assert_eq!(capture.target, capture.output);
    }

    #[test]
    fn discarded_session_keeps_only_its_completion_record_and_refuses_finalization() {
        let root = std::env::temp_dir().join(format!(
            "hoi4-discard-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let session = root.join("sessions").join("7-1");
        let campaign = session.join("00000000-0000-4000-8000-000000000000");
        fs::create_dir_all(session.join("bin")).unwrap();
        fs::create_dir_all(&campaign).unwrap();
        fs::write(session.join("bin/collector.dll"), b"synthetic").unwrap();
        fs::write(campaign.join("columnar.0.0.h4j"), b"synthetic").unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), b"synthetic").unwrap();
        write_json(&session.join("status.json"), &json!({"session":7})).unwrap();
        let earlier = root.join("00000000-0000-4000-8000-000000000000.battleplan");
        fs::write(&earlier, b"earlier recording").unwrap();
        let active = root.join("active.json");
        write_json(&active, &json!({"session_dir": session})).unwrap();
        assert!(!session_discarded(&session));
        write_json(
            &session.join("completed.json"),
            &json!({"replays":[],"discarded":true,"partial":false,"issues":[]}),
        )
        .unwrap();
        remove_active(&active, &session).unwrap();
        discard_session(&session).unwrap();
        let left: Vec<_> = fs::read_dir(&session)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["completed.json"]);
        assert!(!active.exists());
        assert_eq!(fs::read(&earlier).unwrap(), b"earlier recording");
        assert!(session_discarded(&session));
        let error = command(vec![
            "finalize".into(),
            session.clone().into_os_string(),
            "--out".into(),
            root.clone().into_os_string(),
        ])
        .unwrap_err();
        assert!(error.contains("discarded"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn missing_foreign_or_inconsistent_terminal_results_refuse_clean_success() {
        let foreign = json!({"state":"detached","session":8,"drain_complete":true});
        let outcome = classify_terminal(7, true, false, Some(&foreign), None);
        assert_eq!(outcome.drain_complete, None);
        assert!(!outcome.issues.is_empty());
        assert!(
            !classify_terminal(7, true, false, None, None)
                .issues
                .is_empty()
        );
        assert!(completed_detach(&json!({"replays":["file"]})).is_err());
        assert!(completed_detach(&json!({"partial":false,"issues":["late failure"]})).is_err());
        assert!(completed_detach(&json!({"partial":false,"issues":[],"detach_requested":true,"detached":true,"process_exited":false,"drain_complete":false})).is_err());
    }
}
