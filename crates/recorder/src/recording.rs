//! Passive process discovery, injection, detach and finalization.
mod history_requests;
mod progress_io;
mod scripted_map_demand;
use crate::{
    directories::{default_capture_directory, resolve_user_directory},
    replay_io,
    windows::{self, Event, Process},
};
use engine_executable::{build::PINNED_BUILD, build::hash_executable};
use installed_content_core::{content::GatherOptions, content::GatheredContent, content::gather};
use installed_content_localisation::localisation::Localisation;
use history_requests::HistoryRequest;
use replay::{compression::Compression, recorder_settings::RecorderSettings};
use replay_data::identity::IdentityCodec;
use replay_data::{identity::SessionStart, scope::CaptureMode};
use replay_data::{witness::TerminalWitness, witness::WITNESS_FILE_NAME};
use serde_json::{Value, json};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    ffi::{OsStr, OsString},
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

/// Retry failed publication without adding a dependency to capture.
const CHECKPOINT_RETRY: Duration = Duration::from_secs(10);

/// Saves the campaign being recorded as it grows, on the finalize-ahead thread in background
/// mode, so its spool stays small and the save at the end covers only the hours since.
#[derive(Default)]
struct Checkpoints {
    /// When a campaign whose last checkpoint saved nothing or failed is tried again.
    retry: BTreeMap<String, Instant>,
}

impl Checkpoints {
    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        session: &Path,
        target: &Path,
        ahead: &mut replay_io::Ahead,
        settling: Option<&Settling>,
        naming: replay_io::Naming<'_>,
        stopped: &AtomicBool,
    ) {
        let Ok(campaigns) = replay_io::spooled_campaigns(session) else {
            return;
        };
        for campaign in campaigns {
            if stopped.load(Ordering::Acquire) {
                return;
            }
            let directory = session.join(&campaign);
            if let Err(error) = replay_io::drop_checkpointed(&directory) {
                eprintln!("Checkpointed spool segments could not be deleted: {error}");
            }
            if settling.is_some_and(|settling| {
                locked(&settling.loaded).as_deref() != Some(campaign.as_str())
            }) || self
                .retry
                .get(&campaign)
                .is_some_and(|retry| Instant::now() < *retry)
            {
                continue;
            }
            let Some(through) = ahead.completed_through(&directory) else {
                continue;
            };
            let began = Instant::now();
            let auto_save = settling.is_none_or(|settling| settling.save.load(Ordering::Acquire));
            let result = replay_io::checkpoint_campaign(
                session,
                target,
                &campaign,
                through,
                auto_save,
                ahead,
                naming,
                &|| Ok(()),
            );
            match result {
                Ok(Some(done)) => {
                    println!(
                        "Checkpoint: {} hours of campaign {campaign} saved into {} in {:.1} s; {} spool segments deleted.",
                        done.hours,
                        done.recording.display(),
                        began.elapsed().as_secs_f64(),
                        done.deleted
                    );
                    if done.deleted == 0 {
                        self.retry
                            .insert(campaign, Instant::now() + CHECKPOINT_RETRY);
                    } else {
                        self.retry.remove(&campaign);
                    }
                }
                Ok(None) => {
                    self.retry
                        .insert(campaign, Instant::now() + CHECKPOINT_RETRY);
                }
                Err(error) => {
                    eprintln!(
                        "Checkpoint of campaign {campaign} failed; its spool keeps its hours for the session's save: {error}"
                    );
                    self.retry
                        .insert(campaign, Instant::now() + CHECKPOINT_RETRY);
                }
            }
        }
    }
}

/// A campaign the running session set aside (the game loaded another), not saved yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub campaign: String,
    pub unsaved: replay_io::Unsaved,
    pub saving: bool,
}

/// A save or discard of a set-aside campaign, carried out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetAside {
    /// `None`: no admitted load recorded an hour.
    Saved(String, Option<PathBuf>),
    Discarded(String),
    Failed(String, String),
}

/// Shared by the capture loop and the finalize-ahead thread.
#[derive(Default)]
struct Settling {
    /// Auto save, as the settings say at the capture loop's last look.
    save: AtomicBool,
    /// Save (false) or Discard (true) asked for a recording.
    requests: std::sync::Mutex<Vec<(String, bool)>>,
    /// The waiting campaigns, when changed since last taken.
    waiting: std::sync::Mutex<Option<Vec<Waiting>>>,
    done: std::sync::Mutex<Vec<SetAside>>,
    /// The campaign the game has loaded: recorded, never set aside, even before its first hour.
    loaded: std::sync::Mutex<Option<String>>,
    /// Campaigns with an admitted load the game left: with no hour spooled, Auto save saves the
    /// hours ended sessions left waiting of them, as it would with one.
    left: std::sync::Mutex<BTreeSet<String>>,
    stopped: Arc<AtomicBool>,
    /// A collector holds the histories, so it is asked to let go of one before it changes.
    collector: bool,
}

fn locked<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Each set-aside campaign's unsaved segments last acted on, whether they wait, and whether their save failed.
type Settled = BTreeMap<String, (Vec<(u32, u64)>, Option<Waiting>, bool)>;

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
            // A request visible despite its failed flush is returned by the retake.
            *loan = HistoryRequest::lent(session, campaign);
            return Err(error);
        }
    }
    loan.as_ref().unwrap().wait(stopped)
}

/// Run `work` while the collector lets go of the campaign's history; `None` when it would not.
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

/// Save, discard or list each campaign whose segments the collector closed.
fn settle(
    session: &Path,
    target: &Path,
    ahead: &mut replay_io::Ahead,
    settling: &Settling,
    settled: &mut Settled,
    naming: replay_io::Naming<'_>,
) -> Result<(), String> {
    let root = session
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| failure_text(session, "not in a capture root"))?;
    let mut requests = std::mem::take(&mut *locked(&settling.requests));
    let mut changed = false;
    let campaigns = replay_io::spooled_campaigns(session)?;
    let before = settled.len();
    settled.retain(|campaign, _| campaigns.contains(campaign));
    changed |= settled.len() != before;
    for campaign in &campaigns {
        let campaign = campaign.clone();
        let campaign_dir = session.join(&campaign);
        let unsaved = replay_io::unsaved_segments(&campaign_dir)?;
        let at = requests.iter().rposition(|(asked, _)| *asked == campaign);
        // Nothing of it waits here: what it asks for acts on the hours ended sessions left.
        let Some((_, newest, _)) = unsaved.last() else {
            changed |= settled.remove(&campaign).is_some();
            continue;
        };
        // The collector still writes the campaign it records, or the game loaded it again.
        if locked(&settling.loaded).as_ref() == Some(&campaign)
            || replay_io::open_held(newest).is_err()
        {
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
        let request = at.map(|at| requests.remove(at).1);
        requests.retain(|(asked, _)| *asked != campaign);
        let segments: Vec<_> = unsaved
            .iter()
            .map(|(number, _, bytes)| (*number, *bytes))
            .collect();
        // Auto save turned on saves what waits, except a save that failed.
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
                .is_some_and(|(known, _, _)| *known == segments)
        {
            continue;
        }
        changed = true;
        if request == Some(true) {
            let discarded = with_history(settling, session, &campaign, || {
                replay::recording_files::unsaved_recording(target, campaign.as_bytes())
                    .map_err(|error| failure_text(target, error))?;
                replay_io::discard_checkpoints(session, &campaign)?;
                discard_recording(root, target, &campaign)?;
                replay_io::discard_unsaved(session, &campaign)
            });
            let Some(discarded) = discarded else {
                locked(&settling.requests).push((campaign, true));
                continue;
            };
            ahead.forget(&campaign_dir);
            settled.remove(&campaign);
            locked(&settling.done).push(match discarded {
                Ok(()) => SetAside::Discarded(campaign),
                Err(error) => SetAside::Failed(campaign, error),
            });
            continue;
        }
        let summary = replay_io::unsaved(session, &campaign)?;
        let save = request == Some(false) || settling.save.load(Ordering::Acquire);
        let waiting = summary.map(|unsaved| Waiting {
            campaign: campaign.clone(),
            saving: save && !unsaved.undecided,
            unsaved,
        });
        if request == Some(false) && waiting.is_none() {
            // No hour of it here is admitted: Save saves those ended sessions left waiting.
            requests.push((campaign.clone(), false));
        }
        if let Some(waiting) = waiting.as_ref().filter(|waiting| waiting.saving) {
            settled.insert(
                campaign.clone(),
                (segments.clone(), Some(waiting.clone()), false),
            );
            *locked(&settling.waiting) = Some(waiting_list(settled));
            let saved = with_history(settling, session, &campaign, || {
                // Hours earlier sessions left waiting are saved first, in order.
                let earlier =
                    save_recording_except(root, target, &campaign, naming, Some(session))?;
                replay_io::save_campaign(
                    session,
                    target,
                    &campaign,
                    false,
                    &|| Ok(()),
                    ahead,
                    naming,
                )
                .map(|(replay, omitted)| (replay.or(earlier), omitted))
            });
            let Some(saved) = saved else {
                settled.insert(campaign.clone(), (Vec::new(), Some(waiting.clone()), false));
                if let Some(request) = request {
                    locked(&settling.requests).push((campaign, request));
                }
                continue;
            };
            locked(&settling.done).push(match saved {
                Ok((replay, _)) => {
                    settled.insert(campaign.clone(), (segments, None, false));
                    SetAside::Saved(campaign, replay)
                }
                Err(error) => {
                    let mut waiting = waiting.clone();
                    waiting.saving = false;
                    settled.insert(campaign.clone(), (segments, Some(waiting), true));
                    SetAside::Failed(campaign, format!("{error}; spool retained"))
                }
            });
            ahead.forget(&campaign_dir);
            continue;
        }
        if request == Some(false)
            && waiting
                .as_ref()
                .is_some_and(|waiting| waiting.unsaved.undecided)
        {
            // Its loads are decided when recording stops; it is saved then if asked.
            locked(&settling.done).push(SetAside::Failed(
                campaign.clone(),
                "it can be saved once recording stops".into(),
            ));
        }
        settled.insert(campaign, (segments, waiting, false));
    }
    // Left with no hour spooled, they save as a campaign left with one does.
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
                save_recording_except(root, target, &campaign, naming, Some(session))
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
    // Recordings with nothing waiting in this session: their hours waiting from ended sessions.
    for (campaign, discard) in requests {
        // Loaded again, even with no hour in this session yet, it is recorded.
        if locked(&settling.loaded).as_ref() == Some(&campaign) {
            locked(&settling.done).push(SetAside::Failed(
                campaign,
                "it is being recorded again; save or discard it once the game leaves it".to_owned(),
            ));
            continue;
        }
        let result = with_history(settling, session, &campaign, || {
            if discard {
                discard_recording(root, target, &campaign).map(|()| None)
            } else {
                save_recording_except(root, target, &campaign, naming, Some(session)).map(Some)
            }
        });
        let Some(result) = result else {
            locked(&settling.requests).push((campaign, discard));
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

// The loaded 1941 run lost coordinator CPU with 1.5 GB unwritten; one normal open block is not debt.
#[derive(Default)]
struct PackingCatchUp {
    since: Option<Duration>,
}

impl PackingCatchUp {
    fn update(&mut self, now: Duration, backlog: Option<&replay_io::PackingBacklog>) -> bool {
        let Some(backlog) = backlog else {
            self.since = None;
            return false;
        };
        let seconds = replay::container::Outstanding {
            rows: backlog.runnable.work.rows,
            ..Default::default()
        }
        .runnable_seconds();
        let closed = backlog.runnable.closed_blocks;
        match self.since {
            None if seconds >= 12.0 || closed >= 2 => self.since = Some(now),
            Some(since)
                if seconds <= 3.0
                    && closed == 0
                    && now.saturating_sub(since) >= Duration::from_secs(10) =>
            {
                self.since = None
            }
            _ => {}
        }
        self.since.is_some()
    }
}

fn packing_progress(
    packing: &replay::container::Packing,
    ahead: &replay_io::Ahead,
    session: &Path,
    policy: &mut PackingCatchUp,
    elapsed: Duration,
    last: &mut Option<Duration>,
) -> Option<f64> {
    let backlog = ahead.backlog(session);
    let known = backlog.as_ref().ok();
    let runnable = known.map(|backlog| backlog.runnable);
    let blocked = known.map(|backlog| backlog.blocked);
    let before = packing.mode();
    if before == "catch_up_normal" && policy.since.is_none() {
        policy.since = Some(elapsed);
    }
    packing.catch_up(policy.update(elapsed, known));
    packing
        .watch_progress(runnable.is_some_and(|debt| debt.work.rows > 0 || debt.closed_blocks > 0));
    let mode = packing.mode();
    if before != mode
        || last.is_none_or(|last| elapsed.saturating_sub(last) >= Duration::from_secs(5))
    {
        *last = Some(elapsed);
        let (read, written, packed) = (ahead.frames_read(), ahead.written(), ahead.hours());
        println!(
            "Packing progress: {}",
            json!({
                "elapsed_s": elapsed.as_secs_f64(),
                "scope": "accepted current spool tails; counts reset on retirement or campaign removal",
                "tail_read_hours": read,
                "tail_written_hours": written,
                "tail_packed_hours": packed,
                "unwritten_hours": read.saturating_sub(written),
                "unpacked_hours": read.saturating_sub(packed),
                "backlog_scope": "runnable admitted prefix; blocked includes unadmitted, refused or stopped work",
                "backlog_error": backlog.as_ref().err(),
                "runnable_unwritten_hours": runnable.map(|debt| debt.unwritten_hours),
                "closed_unpacked_blocks": runnable.map(|debt| debt.closed_blocks),
                "rows_bytes": runnable.map(|debt| debt.work.rows),
                "compression_work_bytes": runnable.map(|debt| debt.work.compression),
                "verified_pending_raw_bytes": runnable.map(|debt| debt.work.readback),
                "open_raw_bytes": runnable.map(|debt| debt.work.open),
                "estimated_pack_readback_s": runnable.map(|debt| debt.work.stop_seconds()),
                "blocked_unwritten_hours": blocked.map(|debt| debt.unwritten_hours),
                "blocked_closed_unpacked_blocks": blocked.map(|debt| debt.closed_blocks),
                "blocked_rows_bytes": blocked.map(|debt| debt.work.rows),
                "blocked_compression_work_bytes": blocked.map(|debt| debt.work.compression),
                "blocked_verified_pending_raw_bytes": blocked.map(|debt| debt.work.readback),
                "blocked_open_raw_bytes": blocked.map(|debt| debt.work.open),
                "priority_mode": mode,
                "priority_mode_scope": "requested; OS transition failures are reported to stderr",
                "cpu_credits": replay::container::Packing::workers(),
                "logical_io_mib_s": if mode == "foreground" { None } else { Some(32) },
                "logical_io_bytes": packing.io_bytes(),
                "limiter_wait_thread_s": packing.io_wait().as_secs_f64(),
            })
        );
    }
    runnable.map(|debt| debt.work.runnable_seconds())
}

type AheadResult = (replay_io::Ahead, Option<Localisation>, Duration, Duration);

/// Continuously packs a growing spool at every speed, independently of the hourly callback.
struct FinalizingAhead {
    packing: Option<Arc<replay::container::Packing>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<AheadResult>>,
    asked: Arc<AtomicBool>,
    /// The file name each recording it saves would get so far, by campaign.
    names: Arc<Mutex<BTreeMap<String, String>>>,
}

/// The file name a save through this session would give each recording in `target` now, by
/// campaign: those it spools, and those ended sessions left waiting there, which it saves alike.
fn predicted_names(
    session: &Path,
    target: &Path,
    ahead: &replay_io::Ahead,
    naming: replay_io::Naming<'_>,
) -> BTreeMap<String, String> {
    let mut names = ahead.names(session, target, naming);
    let (Ok(root), Ok(spooled)) = (root_of(session), replay_io::spooled_campaigns(session)) else {
        return names;
    };
    let waiting: BTreeSet<_> = session_folders(root)
        .iter()
        .flat_map(|ended| waiting_campaigns(ended))
        .filter(|waiting| waiting.target == target && !spooled.contains(&waiting.campaign))
        .map(|waiting| waiting.campaign)
        .collect();
    for campaign in waiting {
        let sessions = waiting_sessions(root, target, &campaign);
        if let Some(name) = replay_io::waiting_name(&sessions, target, &campaign, naming) {
            names.insert(campaign, name);
        }
    }
    names
}

impl FinalizingAhead {
    /// With `settling`, it also saves or lists the campaigns the session set aside.
    fn start(
        session: &Path,
        target: &Path,
        content: &GatheredContent,
        settling: Option<Arc<Settling>>,
        compression: Option<Compression>,
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
        let initial_settings = RecorderSettings::load().unwrap_or_default();
        let packing = replay::container::Packing::new()
            .map(|packing| {
                packing
                    .set_compression(compression.unwrap_or_else(|| initial_settings.compression));
                let initial_revision = initial_settings.compression_revision;
                packing.set_compression_source(move || {
                    RecorderSettings::load().ok().map(|current| {
                        if current.compression_revision == initial_revision {
                            compression.unwrap_or(current.compression)
                        } else {
                            current.compression
                        }
                    })
                });
                Arc::new(packing)
            })
            .inspect_err(|error| {
                eprintln!("Continuous packing could not start; spool retained: {error}");
            })
            .ok();
        let thread = packing.clone().and_then(|packing| std::thread::Builder::new()
            .name("finalize ahead".into())
            .spawn(move || {
                let _pass_progress = progress_io::passes(&session);
                packing.run(|| {
                // The session's own content stack names its countries as the game does.
                let localisation = installed_content_localisation::localisation::read(&layers, &language)
                    .inspect_err(|error| {
                        eprintln!("Countries are named by tag: the localisation could not be read: {error}");
                    })
                    .ok();
                drop(layers);
                let stopped = || stopping.load(Ordering::Acquire);
                let mut ahead = replay_io::Ahead::default();
                let (mut reading, mut writing) = (Duration::ZERO, Duration::ZERO);
                let mut failed = false;
                let mut fail = |error: String| {
                    if !std::mem::replace(&mut failed, true) {
                        eprintln!("Finalizing ahead paused: {error}");
                    }
                };
                let mut settled = Settled::new();
                let mut checkpoints = Checkpoints::default();
                println!("Continuous packing: background_cpu_credits={} logical_io_mib_s=32 burst_mib=8 natural_block_hours=720; collector has no dependency on these workers", replay::container::Packing::workers());
                let mut service_scale = 1.0_f64;
                let mut catch_up = PackingCatchUp::default();
                let progress_began = Instant::now();
                let mut last_progress = None;
                let mut last_work = None;
                let settings = RecorderSettings::path();
                while !stopped() {
                    let work_before = last_work;
                    let frames_before = ahead.written();
                    let before = thread_cpu();
                    packing.watch_progress(last_work.is_some_and(|work| work > 0.001));
                    {
                        let _phase = replay::container::Packing::phase(replay::container::Phase::Drain);
                        if let Err(error) = ahead.read(&session, &target) {
                            fail(error);
                        }
                    }
                    let _ = packing_progress(&packing, &ahead, &session, &mut catch_up, progress_began.elapsed(), &mut last_progress);
                    let naming = replay_io::Naming {
                        localisation: localisation.as_ref(),
                        settings: &settings,
                        report: None,
                    };
                    if asking.swap(false, Ordering::AcqRel) {
                        let named = predicted_names(&session, &target, &ahead, naming);
                        *published
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) = named;
                    }
                    reading += thread_cpu() - before;
                    packing.watch_progress(true);
                    if let Some(settling) = &settling
                        && let Err(error) =
                            settle(&session, &target, &mut ahead, settling, &mut settled, naming)
                    {
                        eprintln!("Set-aside campaigns could not be looked at: {error}");
                    }
                    {
                        let began = Instant::now();
                        let before_work = ahead.backlog(&session).ok().map(|backlog| backlog.runnable.work.runnable_seconds());
                        packing.watch_progress(before_work.is_some_and(|work| work > 0.001));
                        let slice = Duration::from_secs_f64(before_work.map_or(0.05, |work| (service_scale * work).clamp(0.05, 1.0)));
                        let busy = || stopped() || began.elapsed() >= slice;
                        let before = thread_cpu();
                        if let Err(error) = ahead.write(&session, &target, &busy) {
                            fail(error);
                        }
                        writing += thread_cpu() - before;
                        let after_work = ahead.backlog(&session).ok().map(|backlog| backlog.runnable.work.runnable_seconds());
                        if let (Some(before), Some(after)) = (before_work, after_work)
                            && before - after > 0.01
                        {
                            service_scale = 0.75 * service_scale + 0.25 * (began.elapsed().as_secs_f64() / (before - after)).clamp(0.1, 20.0);
                        }
                    }
                    packing.watch_progress(true);
                    checkpoints.run(
                        &session,
                        &target,
                        &mut ahead,
                        settling.as_deref(),
                        naming,
                        &stopping,
                    );
                    let work_after = packing_progress(&packing, &ahead, &session, &mut catch_up, progress_began.elapsed(), &mut last_progress);
                    let progressed = ahead.written() != frames_before
                        || matches!((work_before, work_after), (Some(before), Some(after)) if after < before);
                    last_work = work_after;
                    if (work_after.is_none_or(|work| work <= 0.001) || !progressed) && !stopped() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                (ahead, localisation, reading, writing)
            }) })
            .ok());
        Self {
            packing,
            stop,
            thread,
            asked,
            names,
        }
    }

    fn promote(&self) {
        if let Some(packing) = &self.packing {
            packing.promote();
        }
    }

    /// Names are worked out only while someone shows them.
    fn ask_names(&self) {
        self.asked.store(true, Ordering::Release);
    }

    fn names(&self) -> BTreeMap<String, String> {
        self.names
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// What was written ahead, and the localisation that names the countries.
    fn finish(mut self) -> (replay_io::Ahead, Option<Localisation>) {
        self.promote();
        self.stop.store(true, Ordering::Release);
        match self.thread.take().map(std::thread::JoinHandle::join) {
            Some(Ok((ahead, localisation, reading, writing))) => {
                let work = ahead.outstanding();
                println!(
                    "Stop outstanding: rows_bytes={} compression_work_bytes={} verified_pending_raw_bytes={} open_raw_bytes={} estimated_pack_readback_s={:.3} (offline service seed; excludes detach and staging)",
                    work.rows,
                    work.compression,
                    work.readback,
                    work.open,
                    work.stop_seconds()
                );
                let (read, written) = (ahead.frames_read(), ahead.written());
                println!(
                    "Finalize ahead: tail_read_hours={read} tail_written_hours={written} tail_packed_hours={} lifetime_coordinator_read_cpu_s={:.3} lifetime_coordinator_write_cpu_s={:.3} (helper CPU excluded)",
                    ahead.hours(),
                    reading.as_secs_f64(),
                    writing.as_secs_f64(),
                );
                (ahead, localisation)
            }
            _ => (replay_io::Ahead::default(), None),
        }
    }
}

impl Drop for FinalizingAhead {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

/// The calling thread's CPU time.
fn thread_cpu() -> Duration {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    // SAFETY: FILETIME is plain data.
    let mut times: [FILETIME; 4] = unsafe { std::mem::zeroed() };
    let [creation, exit, kernel, user] = &mut times;
    // SAFETY: the calling thread's pseudo-handle and four owned FILETIMEs.
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

/// Stop and discard requests for a capture. The command line's console
/// handler owns one; the tray recorder owns one per capture.
#[derive(Debug, Default)]
pub struct Control {
    stop: AtomicBool,
    stop_qpc: std::sync::atomic::AtomicU64,
    discard: AtomicBool,
    /// Save (false) or Discard (true) asked for a campaign the session set aside.
    set_aside: std::sync::Mutex<Vec<(String, bool)>>,
    /// The capture loop takes set-aside requests; changed only under `set_aside`'s lock.
    accepting: AtomicBool,
}

impl Control {
    pub const fn new() -> Self {
        Self {
            stop: AtomicBool::new(false),
            stop_qpc: std::sync::atomic::AtomicU64::new(0),
            discard: AtomicBool::new(false),
            set_aside: std::sync::Mutex::new(Vec::new()),
            accepting: AtomicBool::new(false),
        }
    }

    /// Save or (with `discard`) Discard a campaign the running session set aside; false once its
    /// capture loop has ended, when the caller acts on it itself.
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

    /// Stop taking set-aside requests; those taken and not yet acted on.
    fn end_requests(&self) -> Vec<(String, bool)> {
        let mut requests = locked(&self.set_aside);
        self.accepting.store(false, Ordering::Release);
        std::mem::take(&mut *requests)
    }

    /// Ask the capture to detach. `discard` drops this session's capture
    /// instead of saving it; only an explicit user choice may set it.
    pub fn stop(&self, discard: bool) {
        // The choice is visible before the stop that acts on it.
        if let Some(qpc) = performance_counter() {
            let _ = self
                .stop_qpc
                .compare_exchange(0, qpc, Ordering::AcqRel, Ordering::Acquire);
        }
        self.discard.store(discard, Ordering::Release);
        self.stop.store(true, Ordering::Release);
    }

    pub fn stopping(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    pub fn discarding(&self) -> bool {
        self.discard.load(Ordering::Acquire)
    }

    /// Clear every request before the next capture.
    pub fn reset(&self) {
        self.stop.store(false, Ordering::Release);
        self.stop_qpc.store(0, Ordering::Release);
        self.discard.store(false, Ordering::Release);
        let mut requests = locked(&self.set_aside);
        self.accepting.store(false, Ordering::Release);
        requests.clear();
    }
}

/// What a capture is doing, for a controller that shows it.
#[derive(Debug)]
pub enum Stage<'a> {
    /// Verifying the game and its content, injecting and preparing history.
    Preparing,
    /// The collector is sampling into `session`. `status` is its latest
    /// readable publication, delivered about every two seconds.
    Recording {
        session: &'a Path,
        status: Option<&'a Value>,
        /// The file name a save through this session would give each recording in its folder
        /// now, by campaign, while asked for; one not known is left out.
        names: &'a BTreeMap<String, String>,
    },
    /// The spool is final; `records` counts finalization work done so far.
    Saving { records: u64 },
    /// Published data is ready for the user, independently of DLL unload.
    Finished {
        result: Result<&'a CaptureOutcome, &'a str>,
    },
    /// Observe unload separately after showing the saved result.
    Unloading(UnloadObservation),
    /// Campaigns set aside: those waiting, when that changed, and what was done since.
    SetAside {
        waiting: Option<&'a [Waiting]>,
        done: &'a [SetAside],
    },
}

/// The end of one capture.
#[derive(Debug, Default)]
pub struct CaptureOutcome {
    /// A stop was requested (by the controller or the collector), rather than
    /// the game exiting on its own.
    pub detach_requested: bool,
    pub process_exited: bool,
    /// Recordings written or appended by this session.
    pub replays: Vec<PathBuf>,
    /// The session was discarded at the user's request; nothing was published.
    pub discarded: bool,
    /// The session ended because the controller refused a load, not because
    /// of a stop or the game exiting.
    pub rerouted: bool,
    /// Loads the session left out of its recordings; their hours stay in the
    /// session folder.
    pub omitted: Vec<Omission>,
    /// Campaigns whose recorded hours wait in the session folder to be saved.
    pub waiting: Vec<String>,
    /// Recordings saved under their old name because renaming them failed.
    pub unrenamed: Vec<String>,
}

/// Asks whether a load of a campaign is recorded; the flag says this session's
/// spool already holds the campaign's hours from an earlier load.
pub type AdmitLoad<'a> = dyn Fn(&str, bool) -> Result<(), Refusal> + 'a;

use crate::replay_io::write_json;
pub use crate::replay_io::{Omission, Refusal};

static CONSOLE: Control = Control::new();
unsafe extern "system" fn console_control(kind: u32) -> i32 {
    if kind == CTRL_C_EVENT || kind == CTRL_BREAK_EVENT {
        CONSOLE.stop(false);
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
    compression: Option<Compression>,
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

/// Keep the sampler grammar opaque to the recorder: the feature-enabled DLL
/// owns parsing, while an ordinary DLL names and refuses a nonempty request.
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
        "recorder [record] [--out <directory>] [--user-dir <Paradox directory>] [--once] [--capture-mode full|map] [--compression low|high] [--no-game-score] [--disable-operation-outcomes] [--disable-politics-capture]{fault_option}
recorder detach [--out <directory>]
recorder refresh [--out <directory>]
recorder finalize <session-directory> [--out <directory>]
recorder inspect <campaign.battleplan>
recorder inspect <campaign.battleplan> --characters <substring> --hour <n> --branch <id>
{}
{}
{}
{}
{}
{}
Environment: BATTLE_PLAN_WORKER_SAMPLER (comma-separated off, interval_us=<n>)
  Forwarded verbatim to collector attach; a nonempty value requires collector feature worker_sampler.
Environment: BATTLE_PLAN_MAP_EVENTS (comma-separated map event family names, or none)
  The map event families the collector installs; every family when unset. The others record unavailable coverage.",
        crate::metrics::USAGE,
        crate::oracle::USAGE,
        crate::movement_oracle::USAGE,
        crate::ship_name_witness::USAGE,
        crate::sizes::USAGE,
        crate::derived::USAGE
    )
}

/// The map event families named in `families` (comma-separated; `none` for
/// none), passed to the collector, which installs only those.
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
        compression: None,
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
            Some("--compression") if command == "record" => {
                let name = args.next().ok_or_else(usage)?;
                options.compression = Some(
                    name.to_str()
                        .and_then(Compression::from_name)
                        .ok_or_else(|| "--compression must be low or high".to_owned())?,
                );
            }
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

pub fn command(mut args: Vec<OsString>) -> Result<(), String> {
    // bench.ps1 passes these four positionals directly to its MetricsTool.
    if args.first().is_some_and(|arg| {
        Path::new(arg)
            .extension()
            .and_then(OsStr::to_str)
            .is_some_and(|extension| extension.eq_ignore_ascii_case("battleplan") || extension.eq_ignore_ascii_case("hoi4replay"))
    }) {
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
        "record" => record(options),
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
    // Installed first, so the packing workers inherit the pass progress.
    let _pass_progress = progress_io::passes(session);
    replay_io::with_compression(&RecorderSettings::path(), || {
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
        let unsaved = replay_io::unsaved(session, campaign).ok().flatten();
        waiting.push(waiting_json(campaign, unsaved.as_ref()));
    }
    let mut replays = completed["replays"].as_array().cloned().unwrap_or_default();
    for replay in &finalized.replays {
        let replay = json!(replay);
        if !replays.contains(&replay) {
            replays.push(replay);
        }
    }
    for (key, value) in details.as_object().unwrap() {
        completed[key] = value.clone();
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

/// `inspect <replay> --characters <substring> --hour <n> --branch <id>`, the
/// options in any order, each once.
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

pub(crate) const TWO_GAMES: &str =
    "two hoi4.exe processes are unsupported; no second process will be recorded";
pub(crate) const UNSUPPORTED_BUILD: &str = "unsupported game build";
pub(crate) const COLLECTOR_LOADED: &str =
    "collector.dll is already loaded in this process; detach its recorder first";
const COLLECTOR_UNLOAD_WARNING: &str =
    "collector unload is unconfirmed; quit the game before recording again";

pub(crate) fn checked_single_game() -> Result<Option<u32>, String> {
    match windows::games()?.as_slice() {
        [] => Ok(None),
        [pid] => Ok(Some(*pid)),
        _ => Err(TWO_GAMES.into()),
    }
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
    loop {
        if CONSOLE.stopping() {
            return Ok(());
        }
        let Some(pid) = checked_single_game()? else {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        };
        let outcome = capture(pid, &cli_capture(&options, &user), &CONSOLE, &|stage| {
            if let Stage::Unloading(observation) = stage {
                if let Err(error) = observation.finish() {
                    eprintln!("Collector warning: {error}");
                }
            }
        })?;
        if outcome.detach_requested || options.once || CONSOLE.stopping() {
            return Ok(());
        }
        println!("Waiting for the next game process.");
    }
}

/// Where and what one capture records.
pub(crate) struct CaptureOptions<'a> {
    /// The capture root: session folders and the active record.
    pub output: &'a Path,
    /// Where prior history is read and finished recordings are published.
    pub target: &'a Path,
    pub user: &'a Path,
    pub game_score: bool,
    pub operation_outcomes: bool,
    pub politics_capture: bool,
    /// Which tables the session records; one mode per recording file.
    pub capture_mode: CaptureMode,
    pub compression: Option<Compression>,
    /// A session that reached no spool (the game closed at its menu, say) ends
    /// quietly instead of as a failed finalization.
    pub nothing_recorded_is_ok: bool,
    /// Read the collector's status every two seconds for `Stage::Recording`
    /// and admission. The command line reads it only to print, once a minute.
    pub live_status: bool,
    /// Whether a load the game names may be recorded in this session, asked
    /// once per load, live and again at save for loads the spool identifies
    /// that were never asked about; the flag says this session's spool already
    /// holds the campaign's hours from an earlier load. Each admitted load is
    /// recorded in the session folder as it is admitted, and only admitted
    /// loads are ever published. A live refusal ends the session. `None` (the
    /// command line) publishes every load.
    pub admit: Option<&'a AdmitLoad<'a>>,
    /// The campaign the game has loaded right now and its load signal, read
    /// without the collector. Only these, the spool at save, and `opening`
    /// name loads for admission.
    pub loaded: Option<&'a dyn Fn() -> Option<(String, i64)>>,
    /// The load this session attaches for, already chosen by the controller:
    /// admitted before the collector starts.
    pub opening: Option<(&'a str, i64)>,
    /// Auto save, asked as each decision is made; `None` (the command line) always saves.
    pub auto_save: Option<&'a dyn Fn() -> bool>,
    /// Whether the names recordings would get are shown now.
    pub names_wanted: Option<&'a dyn Fn() -> bool>,
    #[cfg(feature = "alloc-fault-injection")]
    pub alloc_fault: Option<&'a AllocationFault>,
}

/// The command line's capture: the capture root is also the publication folder,
/// every campaign is recorded, and status is read only to print it.
fn cli_capture<'a>(options: &'a Options, user: &'a Path) -> CaptureOptions<'a> {
    CaptureOptions {
        output: &options.output,
        target: &options.output,
        user,
        game_score: options.game_score,
        operation_outcomes: options.operation_outcomes,
        politics_capture: options.politics_capture,
        capture_mode: options.capture_mode,
        compression: options.compression,
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

/// The loads a Battle Plan Recorder session admitted. Each load the game names is
/// asked about once and, when admitted, recorded in the session folder before
/// anything depends on it.
struct Admission<'a> {
    session: &'a Path,
    target: &'a Path,
    seen: BTreeSet<(String, i64)>,
    admitted: replay_io::Admissions,
}

#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Asked about before.
    Seen,
    /// Admitted and recorded.
    Admitted,
    /// Refused: the session ends and the load is never published.
    Refused,
    /// Left out without ending the session: earlier hours of its campaign wait unstaged.
    Omitted,
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

    /// The load the session attaches for, chosen by the controller: admitted
    /// without asking again, before the collector starts.
    fn admit_opening(&mut self, campaign: &str, load: i64) -> Result<(), String> {
        self.seen.insert((campaign.to_owned(), load));
        self.admitted
            .admit(self.session, self.target, campaign, load)
    }

    /// The load the session attaches for, left out: earlier hours of its campaign wait unstaged.
    fn omit_opening(&mut self, campaign: &str, load: i64) -> Result<(), String> {
        self.seen.insert((campaign.to_owned(), load));
        self.admitted
            .refuse(self.session, self.target, campaign, load, Refusal::Waiting)
    }

    /// Ask once per load. `recorded` (asked only for a new load) says the
    /// session's spool already holds the campaign's hours from an earlier
    /// load. Fails when an admitted load cannot be recorded; it is then not
    /// published. A refusal's reason is kept only to report what was left out.
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
                Ok(if refusal == Refusal::Waiting {
                    Decision::Omitted
                } else {
                    Decision::Refused
                })
            }
        }
    }

    /// At save, once nothing is being captured: ask about each load the spool
    /// identifies that was never asked about (one that recorded and ended
    /// between polls, when the game closed or Stop was pressed), by the same rule
    /// and in spool order.
    fn check_spooled(
        &mut self,
        admit: &dyn Fn(&str, bool) -> Result<(), Refusal>,
    ) -> Result<(), String> {
        for (campaign, load) in replay_io::spooled_loads(self.session)? {
            let campaign_dir = self.session.join(&campaign);
            self.check(
                &campaign,
                load,
                || replay_io::recorded_before_load(&campaign_dir, load),
                admit,
            )?;
        }
        Ok(())
    }
}

/// When capture reads the collector's status: once a minute to print it, and
/// with `live` (Battle Plan Recorder) every two seconds for its stage callback and
/// admission. The command line is never live.
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

    /// Whether a printed read is due at `now`, scheduling the next one.
    fn print_due(&mut self, now: Instant) -> bool {
        let due = now >= self.next_print;
        if due {
            self.next_print = now + Self::PRINT;
        }
        due
    }

    /// Whether a live read is due at `now`.
    fn live_due(&self, now: Instant) -> bool {
        self.live && now >= self.next_live
    }

    /// A live read finished at `now`.
    fn live_read(&mut self, now: Instant) {
        self.next_live = now + Self::LIVE;
    }
}

pub(crate) fn capture(
    pid: u32,
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
        compression,
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
    let process = Process::open(pid, true)?;
    let mut executable = File::open(&process.path).map_err(|e| e.to_string())?;
    let exe = hash_executable(&mut executable).map_err(|e| e.to_string())?;
    if !PINNED_BUILD.matches(&exe) {
        return Err(format!(
            "{UNSUPPORTED_BUILD}; wanted {} bytes / {}; found {} bytes / {}. Nothing recorded.",
            PINNED_BUILD.exe_size,
            hex(&PINNED_BUILD.exe_sha256),
            exe.size,
            hex(&exe.sha256)
        ));
    }
    engine_executable::executable_tables::initialize_executable(&process.path)
        .map_err(|error| format!("Cannot prepare game tables: {error}"))?;
    let game_version = PINNED_BUILD
        .read_version_banner(&mut executable)
        .map_err(|e| format!("game version text: {e}"))?;
    let collector_loaded = windows::modules(pid)?.iter().any(|module| {
        module
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("collector.dll"))
    });
    require_collector_absent(Some(collector_loaded))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let directory = output
        .join("sessions")
        .join(format!("{}-{nonce}", process.start_time));
    let _pass_progress = progress_io::passes(&directory);
    let bin = directory.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    let dll = bin.join("collector.dll");
    let source = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("collector.dll");
    fs::copy(&source, &dll).map_err(|e| format!("copy {}: {e}", source.display()))?;
    let dll_hash = hash_executable(File::open(&dll).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?
        .sha256;
    let game_dir = process
        .path
        .parent()
        .ok_or("game executable has no folder")?
        .to_owned();
    let gather_options = GatherOptions {
        game_dir,
        user_dir: user.to_owned(),
        // Match the recorder's fallback for absent or empty LOCALAPPDATA.
        cache_dir: replay::recorder_settings::directory(),
        dlc_order: None,
    };
    println!("Verifying installed content hashes in the background...");
    let hash_thread = std::thread::spawn(move || gather(&gather_options));
    let mut next_discovery = Instant::now();
    while !hash_thread.is_finished() {
        if process.handle.signaled()? {
            return Err("game exited while content identity was being gathered".into());
        }
        if Instant::now() >= next_discovery {
            if checked_single_game()? != Some(pid) {
                return Err("game changed while content identity was being gathered".into());
            }
            next_discovery = Instant::now() + Duration::from_secs(2);
        }
        if control.stopping() {
            return Ok(CaptureOutcome {
                detach_requested: true,
                ..CaptureOutcome::default()
            });
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let content = hash_thread
        .join()
        .map_err(|_| "content hashing worker panicked")?
        .map_err(|e| e.to_string())?;
    println!(
        "Content: {} files, {:.1} MB covered, {} cache hits, {:.3} s.",
        content.stats.files,
        content.stats.bytes as f64 / 1e6,
        content.stats.cache_hits,
        content.stats.elapsed_ms as f64 / 1000.0
    );
    if let Some(note) = &content.order_note {
        println!("Content order is unverified: {note}");
    }
    let identity = content
        .identity(
            game_version,
            exe.sha256,
            exe.size,
            env!("CARGO_PKG_VERSION").into(),
            dll_hash,
        )
        .map_err(|e| e.to_string())?;
    let mut frequency = 0;
    if unsafe { QueryPerformanceFrequency(&mut frequency) } == 0 || frequency <= 0 {
        return Err("QueryPerformanceFrequency failed".into());
    }
    let session = SessionStart {
        id: process.start_time,
        process_id: pid,
        process_start_time: process.start_time,
        qpc_frequency: frequency as u64,
        sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
        capture_mode,
        identity,
    };
    let mut session_bytes = Vec::new();
    session
        .encode(&mut session_bytes)
        .map_err(|e| e.to_string())?;
    let prefix = format!("Local\\HOI4Replay.{}.{}", process.start_time, nonce);
    let ready_name = format!("{prefix}.ready");
    let detach_name = format!("{prefix}.detach");
    let done_name = format!("{prefix}.done");
    let ready = Event::new(&ready_name)?;
    let detach = Event::new(&detach_name)?;
    let done = Event::new(&done_name)?;
    let cancel_file = bin.join("collector.cancel");
    let expected_mounts = &content.expected_mounts;
    let scripted_maps = scripted_map_demand::read(&content.layers, &content.language)
        .map_err(|error| format!("scripted map capture demand: {error}"))?;
    create_failure_witness(&directory, session.id, session.process_start_time)?;
    let mut admission = Admission::new(&directory, target);
    // Before the collector reads any history: this session continues every hour waiting.
    let refused = stage_waiting(output, target)?;
    // Once staged, so a recording those hours become later is noted as this session's own.
    replay_io::select_recordings(&directory, target)?;
    let unstaged = &refused;
    let admit_staged = admit.map(|admit| {
        move |campaign: &str, recorded: bool| {
            if unstaged.contains_key(campaign) {
                Err(Refusal::Waiting)
            } else {
                admit(campaign, recorded)
            }
        }
    });
    if admit.is_some() {
        // Before the collector starts: a Battle Plan Recorder session publishes only
        // the loads this record will list, starting with the one it attaches for.
        replay_io::begin_admissions(&directory, target)?;
        if let Some((campaign, load)) = opening {
            if refused.contains_key(campaign) {
                admission.omit_opening(campaign, load)?;
            } else {
                admission.admit_opening(campaign, load)?;
            }
        }
    } else if let Some((campaign, reason)) = refused.iter().next() {
        // The command line publishes every load, so it cannot leave one out.
        return Err(format!(
            "campaign {campaign} has hours waiting to be saved that could not be prepared ({reason})"
        ));
    }
    let config = json!({"session_id":session.id,"output_dir":directory,"replay_dir":target,"session_start":session_bytes,"ready_event":ready_name,"detach_event":detach_name,"done_event":done_name,"cancel_file":cancel_file,"content_order_note":content.order_note,"expected_mounts":expected_mounts,"mod_descriptors":content.mod_descriptors,"game_score":game_score,"operation_outcomes":operation_outcomes,"politics_capture":politics_capture,"scripted_maps":scripted_maps,"without_history":refused.keys().collect::<Vec<_>>()});
    let sampler = match std::env::var("BATTLE_PLAN_WORKER_SAMPLER").or_else(|_| std::env::var("HOI4_WORKER_SAMPLER")) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("BATTLE_PLAN_WORKER_SAMPLER is not valid Unicode".into());
        }
    };
    let config = worker_sampler_config(config, sampler);
    let config = match std::env::var("BATTLE_PLAN_MAP_EVENTS").or_else(|_| std::env::var("HOI4_MAP_EVENTS")) {
        Ok(families) => map_event_config(config, &families),
        Err(std::env::VarError::NotPresent) => config,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err("BATTLE_PLAN_MAP_EVENTS is not valid Unicode".into());
        }
    };
    #[cfg(feature = "alloc-fault-injection")]
    let config = fault_config(config, alloc_fault);
    write_json(&bin.join("collector.json"), &config)?;
    struct DetachOnDrop<'a> {
        event: &'a Event,
        cancel: &'a Path,
    }
    impl Drop for DetachOnDrop<'_> {
        fn drop(&mut self) {
            let _ = fs::write(self.cancel, b"cancelled\n");
            let _ = self.event.set();
        }
    }
    let _detach_on_error = DetachOnDrop {
        event: &detach,
        cancel: &cancel_file,
    };
    checked_single_game()?
        .filter(|&found| found == pid)
        .ok_or("game changed before injection")?;
    process.inject(&dll, || control.stopping())?;
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
            return Err(fs::read_to_string(&error_path).unwrap_or_else(|e| e.to_string()));
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
    write_json(
        &active,
        &json!({"recorder_pid":std::process::id(),"game_pid":pid,"session_id":session.id,"process_start_time":process.start_time,"session_dir":directory,"dll":dll,"detach_event":detach_name,"done_event":done_name}),
    )?;
    println!("Recording process {pid}. Spool: {}", directory.display());
    // A Battle Plan Recorder session saves or lists each campaign it sets aside.
    let settling = auto_save.map(|auto_save| {
        let settling = Arc::new(Settling {
            collector: true,
            ..Settling::default()
        });
        settling.save.store(auto_save(), Ordering::Release);
        settling
    });
    let ahead = FinalizingAhead::start(&directory, target, &content, settling.clone(), compression);
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
    let mut second_reported = false;
    let mut next_discovery = Instant::now();
    control.accept_requests();
    // The campaign and load the game has loaded, from the one attached for, whatever their
    // admission; loading another campaign closes its segment, so it saves within seconds.
    let mut admitted: Option<(String, i64)> =
        opening.map(|(campaign, load)| (campaign.to_owned(), load));
    let mut closing = Vec::new();
    let capture_ended = Arc::new(AtomicBool::new(false));
    loop {
        requested_detach |= control.stopping() || detach.handle.signaled()?;
        if requested_detach && !stop_observed {
            stop_observed = true;
            stop_qpc = requested_stop_counter(control, &directory);
            stop_load = system_times();
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
            let second = windows::games()?.iter().any(|&other| other != pid);
            if second && !second_reported {
                eprintln!("A second hoi4.exe is unsupported and will not be recorded.");
            }
            second_reported = second;
            next_discovery = Instant::now() + Duration::from_secs(2);
        }
        if status_reads.print_due(Instant::now()) {
            print_status(&status_path);
        }
        if status_reads.live_due(Instant::now()) {
            let status = read_status(&status_path);
            // Only a load the game names (campaign and load signal) is asked
            // about, usually while it is still paused; the collector's status
            // naming a campaign never is. Hours of a load not admitted, asked
            // about too late or never, are never published.
            if let Some(admit) = &admit_staged
                && let Some((campaign, load)) = loaded.and_then(|loaded| loaded())
            {
                // Leaving a campaign for any other load closes it, whatever that load's admission.
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
                let recorded = || replay_io::recorded_before_load(&directory.join(&campaign), load);
                match admission.check(&campaign, load, recorded, admit) {
                    Ok(Decision::Refused) => {
                        println!(
                            "Load {load} of campaign {campaign} is not recorded in this session; detaching."
                        );
                        requested_detach = true;
                        rerouted = true;
                    }
                    Ok(Decision::Admitted | Decision::Seen) => {}
                    Ok(Decision::Omitted) => println!(
                        "Load {load} of campaign {campaign} is left out: its earlier hours wait to be saved."
                    ),
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
    // Asked after the last poll took them: nothing acts on them now, so they are answered.
    let unanswered = control.end_requests();
    if let Some(settling) = &settling {
        answer_ending(settling, unanswered);
    }
    // None outlives the capture, nor waits on a collector detaching: an unanswered close gives up.
    capture_ended.store(true, Ordering::Release);
    ahead.stop.store(true, Ordering::Release);
    for worker in closing {
        let _ = worker.join();
    }
    let packing = ahead.packing.clone();
    let join_began = Instant::now();
    let (ahead, localisation) = finish_ahead(ahead, settling.as_deref());
    let join_seconds = join_began.elapsed().as_secs_f64();
    if !process.handle.signaled()? {
        detach.set()?;
        let mut wait = CollectorWait::new(Instant::now(), DONE_STALL);
        while !done.handle.signaled()? && !process.handle.signaled()? {
            if let Some(end) = wait.ended(Instant::now(), collector_stamp(&directory)) {
                return Err(format!(
                    "detach timed out {}; collector may still be loaded; spool retained and not finalized",
                    end.describe(&directory)
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(summary) = step_summary(&directory, "detach_requested", "done") {
            println!("Collector detach {summary}");
        }
    }
    // Done follows joined spool/status writers and release of the collector's output lock.
    let process_exited = process.handle.signaled()?;
    let spool_final = process_exited || done.handle.signaled()?;
    HistoryRequest::retire_session(&directory, false, process_exited);
    print_status(&status_path);
    requested_detach |= control.stopping() || detach.handle.signaled()?;
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
    // The campaign in progress: the one the game has loaded, else the one the collector wrote last.
    let current = admitted
        .as_ref()
        .map(|(campaign, _)| campaign.clone())
        .or_else(|| {
            read_status(&status_path)
                .and_then(|status| status["campaign"].as_str().map(str::to_owned))
                .filter(|campaign| replay_data::identity::is_campaign_uuid(campaign.as_bytes()))
        });
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
    let discarding = control.discarding();
    let spooled = replay_io::spooled_campaigns(&directory)?;
    let set_aside_unsaved = discarding
        && others_unsaved(
            &directory,
            &spooled,
            current.as_deref(),
            &mut outcome.issues,
        );
    // Another campaign the session admitted may still save the hours it continues.
    let others_admitted = admit.is_some()
        && admission
            .admitted
            .loads
            .keys()
            .any(|campaign| Some(campaign) != current.as_ref());
    if discarding && (set_aside_unsaved || others_admitted) {
        // Discard drops the campaign in progress; the others keep theirs.
        if let Some(current) = current.as_ref().filter(|current| spooled.contains(current)) {
            println!("Discarding campaign {current} of this session.");
            if outcome.issues.is_empty() {
                replay_io::discard_checkpoints(&directory, current)?;
                replay_io::discard_unsaved(&directory, current)?;
            }
        }
    } else if discarding {
        // Published checkpoint hours stay; only this session's unsaved hours are discarded.
        if outcome.issues.is_empty() {
            for campaign in &spooled {
                replay_io::discard_checkpoints(&directory, campaign)?;
            }
        }
        if process_exited {
            println!("Game process exited; discarding this session.");
        } else {
            println!("Collector spool closed; discarding this session's unsaved hours.");
        }
        return finish_capture(
            &directory,
            target,
            &active,
            &session,
            &dll,
            process_exited,
            discarded_completion(&outcome, requested_detach),
            capture_result(
                &directory,
                &outcome,
                CaptureOutcome {
                    detach_requested: requested_detach,
                    process_exited,
                    discarded: true,
                    rerouted,
                    ..CaptureOutcome::default()
                },
            ),
            stage,
            || None,
        );
    }
    // Saving, with the game no longer captured: a load that recorded and ended
    // between polls (the game closing, or Stop) is asked about now by the same
    // rule, so a wanted load is never left out for being brief.
    if let Some(admit) = &admit_staged
        && let Err(error) = admission.check_spooled(admit)
    {
        admission_failures.push(error);
    }
    outcome.issues.extend(
        admission_failures
            .into_iter()
            .map(|error| format!("a load could not be admitted, so it is not published: {error}")),
    );
    if process_exited {
        println!("Game process exited; recovering the flushed spool.");
    } else {
        println!("Collector spool closed; finalizing the committed prefix.");
    }
    let auto = auto_save.is_none_or(|auto_save| auto_save());
    let destination = |campaign: &str| {
        if saved_at_end(
            campaign,
            current.as_deref(),
            auto,
            control.stopping(),
            discarding,
        ) {
            replay_io::Destination::Recording
        } else if discarding && current.as_deref() == Some(campaign) {
            replay_io::Destination::Neither
        } else {
            replay_io::Destination::Unsaved
        }
    };
    let spooled = replay_io::spooled_campaigns(&directory)?;
    let admitted = admit.is_some().then_some(&admission.admitted);
    let recovery_allowed = outcome.recovery_allowed;
    // A campaign without an admitted load is only reported left out: it neither saves nor waits.
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
        match replay_io::unsaved(&directory, campaign) {
            Ok(None) => {}
            Ok(Some(unsaved)) => waiting.push((campaign.clone(), Some(unsaved))),
            Err(error) => {
                eprintln!(
                    "Campaign {campaign} waits to be saved; its hours could not be read: {error}"
                );
                waiting.push((campaign.clone(), None));
            }
        }
    }
    // Listed before saving, so a session that never completes still offers Save and Discard.
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
        Some(&directory),
        target,
        &spooled,
        admitted,
        &|campaign| destination(campaign) == replay_io::Destination::Recording,
        naming,
    );
    outcome.issues.extend(blocked.values().cloned());
    // Nothing recorded means no spooled hours at all. Hours left out of the
    // recordings are reported by finalization, never as nothing recorded.
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
                discarded: discarding,
                rerouted,
                ..CaptureOutcome::default()
            }),
            stage,
            || None,
        );
    }
    // Hours behind older ones that could not be saved stay in the spool, waiting with them.
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
    let (mut finalized, omitted, unrenamed) = match finalized_result {
        Some(Ok(done)) => {
            outcome.issues.extend(
                done.errors
                    .into_iter()
                    .map(|error| format!("replay finalization failed: {error}")),
            );
            (done.replays, done.omitted, done.unrenamed)
        }
        Some(Err(error)) => {
            outcome
                .issues
                .push(format!("replay finalization failed: {error}"));
            (Vec::new(), Vec::new(), Vec::new())
        }
        None => (Vec::new(), Vec::new(), Vec::new()),
    };
    finalized.extend(continued);
    // A save that failed leaves its hours waiting, to be saved or discarded later.
    for campaign in spooled.iter().filter(|campaign| saving_now(campaign)) {
        let campaign_dir = directory.join(campaign);
        if replay_io::unsaved_segments(&campaign_dir).is_ok_and(|segments| segments.is_empty()) {
            continue;
        }
        waiting.push((
            campaign.clone(),
            replay_io::unsaved(&directory, campaign).ok().flatten(),
        ));
    }
    if !waiting.is_empty() {
        println!(
            "{} campaign(s) wait to be saved in {}.",
            waiting.len(),
            directory.display()
        );
    }
    drop(localisation);
    let completed = json!({
        "replays":finalized,
        "target":target,
        "waiting":waiting.iter().map(|(campaign, unsaved)| waiting_json(campaign, unsaved.as_ref())).collect::<Vec<_>>(),
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
            discarded: discarding,
            rerouted,
            omitted,
            waiting: waiting.into_iter().map(|(campaign, _)| campaign).collect(),
            unrenamed,
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
                "scope": "normal save; continue-a-saved-campaign staging and holds proof are not covered by the 30 s target",
            });
            Some(diagnostics)
        },
    )
}

/// Without Auto save, only Stop's Save saves, and only the campaign in progress.
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

/// Stop finalizing ahead; requests the last poll forwarded that it never acted on are answered.
fn finish_ahead(
    ahead: FinalizingAhead,
    settling: Option<&Settling>,
) -> (replay_io::Ahead, Option<Localisation>) {
    let ahead = ahead.finish();
    if let Some(settling) = settling {
        let unanswered = std::mem::take(&mut *locked(&settling.requests));
        answer_ending(settling, unanswered);
    }
    ahead
}

/// Save or Discard asked as the session ended, which nothing carries out now.
fn answer_ending(settling: &Settling, requests: Vec<(String, bool)>) {
    locked(&settling.done).extend(requests.into_iter().map(|(campaign, _)| {
        SetAside::Failed(
            campaign,
            "the session was ending; choose Save or Discard again".to_owned(),
        )
    }));
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

/// The active file is this recorder's own control record, outside the repository.
fn remove_active(active: &Path, directory: &Path) -> Result<(), String> {
    if fs::read(active)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|value| value["session_dir"] == json!(directory))
    {
        fs::remove_file(active).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Remove a discarded session's spool, copied collector and evidence, keeping
/// only its completion record. Call only after collector absence or game exit.
fn discard_session(directory: &Path) -> Result<(), String> {
    let completed = read_status(&directory.join("completed.json"))
        .ok_or("discarded session has no readable completion record")?;
    if completed["partial"] != false || !completed["issues"].as_array().is_some_and(Vec::is_empty) {
        return Ok(());
    }
    for entry in fs::read_dir(directory).map_err(|error| failure_text(directory, error))? {
        let entry = entry.map_err(|error| failure_text(directory, error))?;
        if entry.file_name() == "completed.json" {
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
        removed.map_err(|error| format!("discard {}: {error}", path.display()))?;
    }
    Ok(())
}

fn failure_text(path: &Path, error: impl std::fmt::Display) -> String {
    format!("{}: {error}", path.display())
}

/// Whether the session's completion record says the user discarded it.
fn session_discarded(directory: &Path) -> bool {
    read_status(&directory.join("completed.json")).is_some_and(|value| value["discarded"] == true)
}

/// A JSON publication, when it exists and parses.
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

/// Whether a spooled campaign besides `current` may hold unsaved hours; an unreadable one may.
fn others_unsaved(
    directory: &Path,
    spooled: &[String],
    current: Option<&str>,
    issues: &mut Vec<String>,
) -> bool {
    let mut unsaved = false;
    for campaign in spooled
        .iter()
        .filter(|campaign| Some(campaign.as_str()) != current)
    {
        match replay_io::unsaved_segments(&directory.join(campaign)) {
            Ok(segments) => unsaved |= !segments.is_empty(),
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
    let error_path = directory.join("bin/collector.error.txt");
    let error_file = match fs::read_to_string(&error_path) {
        Ok(error) => Some(error),
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

/// A complete first-failure witness is authoritative while capture is loaded.
/// A concurrent fixed-record write may not yet have a valid checksum; terminal
/// recovery still validates all evidence strictly after the spool becomes final.
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

/// Validate the recorder's terminal result before a second console claims success.
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

/// Ask the attached collector to read every scheduled table at the next
/// hourly sample (a full-refresh hour), for example before an oracle save.
/// The collector acknowledges within about a second; nothing touches the game.
fn request_refresh(output: &Path) -> Result<(), String> {
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
    let ack = session_dir.join("refresh.ack");
    let request = session_dir.join("refresh.request");
    let _ = fs::remove_file(&ack);
    // Write, then rename: the collector never sees a partial token.
    let temporary = session_dir.join(format!("refresh.request.{token}.tmp"));
    fs::write(&temporary, &token).map_err(|e| e.to_string())?;
    if let Err(error) = fs::rename(&temporary, &request) {
        let _ = fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Err(error) = refuse_failed_refresh(&session_dir, &active) {
            if fs::read_to_string(&request).is_ok_and(|text| text.trim() == token) {
                let _ = fs::remove_file(&request);
            }
            return Err(error);
        }
        if fs::read_to_string(&ack).is_ok_and(|text| text.trim() == token) {
            println!("Full refresh requested: the next hourly sample reads every scheduled table.");
            return Ok(());
        }
        if Instant::now() >= deadline {
            // Withdraw our own request so a later attach does not act on it.
            if fs::read_to_string(&request).is_ok_and(|text| text.trim() == token) {
                let _ = fs::remove_file(&request);
            }
            return Err("the collector did not acknowledge the refresh request".into());
        }
        std::thread::sleep(Duration::from_millis(100));
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
            // Completion publication and controller exit can coincide.
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

// This is an inactivity timeout, never a limit on total drain/finalization time.
// While the collector detaches it follows the collector's own steps, so it
// allows what the recording controller allows (DONE_STALL).
const DETACH_STALL_TIMEOUT: Duration = DONE_STALL;
/// The detach command's watch over the session's status and finalization.
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

/// The collector's step log in its session directory (`progress` in the
/// collector), one line per attach or detach step: milliseconds since attach
/// began, the step and an optional count.
const COLLECTOR_PROGRESS: &str = "progress";
/// Readiness and detach wait while the collector takes steps (loading a
/// campaign history; installing, restoring and quiescing each hook family;
/// joining its workers; draining its queues), up to this long for one step.
/// A hook publication suspends every game thread and cannot log while they
/// are held; on a loaded machine one took 43.9 s (2d1ede901c 1936 burn-in,
/// where the game's working set was being paged out), so the limit leaves
/// room for several times that.
const READY_STALL: Duration = Duration::from_secs(180);
const DONE_STALL: Duration = Duration::from_secs(180);
/// No attach or detach waits longer in total, progressing or not.
const COLLECTOR_WAIT_CAP: Duration = Duration::from_secs(15 * 60);

/// The collector's progress: its step log's length and its status's time.
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

/// A controller wait on the collector: it continues while the collector's
/// stamp changes within `stall`, up to [`COLLECTOR_WAIT_CAP`] in total.
struct CollectorWait {
    started: Instant,
    watch: StampWatch<CollectorStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaitEnd {
    /// No collector step for this long.
    Stalled(Duration),
    /// The total cap.
    Capped,
}

impl WaitEnd {
    /// "after …", naming the collector's last step.
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

/// `(milliseconds, step)` of each step-log line.
fn steps(text: &str) -> impl Iterator<Item = (u64, &str)> {
    text.lines().filter_map(|line| {
        let (millis, step) = line.split_once(' ')?;
        Some((millis.parse().ok()?, step))
    })
}

/// The last step, with its time.
fn last_step(text: &str) -> Option<String> {
    let (millis, step) = steps(text).last()?;
    Some(format!("{step} at {:.1} s", millis as f64 / 1000.0))
}

/// "took N s; slowest steps: …" over the steps from the last `from` to the
/// first `to` after it, each step timed from the one before it.
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
struct FinalizationProgress<'a> {
    directory: &'a Path,
    /// Told the record count even when the advisory file cannot be published.
    observe: &'a dyn Fn(u64),
    records: Cell<u64>,
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
            last_write: Cell::new(Instant::now()),
            warned: Cell::new(false),
            work_warned: Cell::new(false),
        };
        progress.publish();
        progress
    }
    fn advance(&self) {
        self.records.set(self.records.get().saturating_add(1));
        let now = Instant::now();
        if now.duration_since(self.last_write.get()) >= Duration::from_secs(1) {
            // Throttle failed attempts too: progress must not stall replay work.
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

fn print_status(path: &Path) {
    if let Ok(bytes) = fs::read(path)
        && let Ok(value) = serde_json::from_slice::<Value>(&bytes)
    {
        if let Some((site, reason)) = destruction_refusal(&value) {
            eprintln!("EquipmentDestruction refusal site={site}: {reason}");
        }
        if let Some(error) = value["error"]
            .as_str()
            .or_else(|| value["last_error"].as_str())
            .or_else(|| value["warning"].as_str())
        {
            eprintln!("Collector: {error}");
        }
        if value["hours"].as_u64().unwrap_or(0) == 0 {
            println!(
                "{}; waiting for the first recorded game hour.",
                value["state"].as_str().unwrap_or("Attached")
            );
            return;
        }
        let number = |key: &str| value[key].as_f64().unwrap_or(0.0);
        println!(
            "{}: {:.0} hours, last {:.3} ms, p95 {:.3} ms, {:.2} h/s, spool {:.1} MB, {:.0} dropped, {:.0} truncated",
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

fn discarded_completion(terminal: &TerminalOutcome, requested_detach: bool) -> Value {
    json!({
        "replays":[],"discarded":true,"detach_requested":requested_detach,
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
    {
        let _lock = completion_lock(directory)?;
        completion_presence(
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
                    Err(_) if windows::games().is_ok_and(|games| !games.contains(&self.pid)) => {
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
    completion_presence(&mut completed, presence, false);
    write_json(&directory.join("completed.json"), &completed)?;
    Ok(completed["discarded"] == true)
}

fn completion_presence(completed: &mut Value, presence: CollectorPresence, pending: bool) {
    if !completed["warnings"].is_array() {
        completed["warnings"] = json!([]);
    }
    completed["warnings"]
        .as_array_mut()
        .unwrap()
        .retain(|warning| warning.as_str() != Some(COLLECTOR_UNLOAD_WARNING));
    completed["detached"] =
        json!(!presence.process_exited && presence.collector_loaded == Some(false));
    completed["collector_loaded"] = json!(presence.collector_loaded);
    completed["process_exited"] = json!(presence.process_exited);
    completed["unload_pending"] = json!(pending);
    if !pending && presence.collector_loaded != Some(false) {
        completed["warnings"]
            .as_array_mut()
            .unwrap()
            .push(json!(COLLECTOR_UNLOAD_WARNING));
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
    completion_presence(&mut completed, presence, false);
    replay_io::write_session_end(directory, target, "completed.json", &completed)?;
    if presence.collector_loaded != Some(false) {
        eprintln!("Collector warning: {COLLECTOR_UNLOAD_WARNING}");
    }
    Ok(presence)
}

/// Prove the original session's spool is final, independently of DLL unload.
fn ensure_finalizable(directory: &Path) -> Result<(Option<Process>, SessionStart), String> {
    let config: Value = serde_json::from_slice(
        &fs::read(directory.join("bin/collector.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let bytes: Vec<u8> =
        serde_json::from_value(config["session_start"].clone()).map_err(|e| e.to_string())?;
    let mut decoder = replay_data::codec::Decoder::new(&bytes);
    let session = SessionStart::decode(&mut decoder).map_err(|e| e.to_string())?;
    decoder.finish().map_err(|e| e.to_string())?;
    if !windows::games()?.contains(&session.process_id) {
        HistoryRequest::retire_session(directory, false, true);
        return Ok((None, session));
    }
    let process = match Process::open(session.process_id, false) {
        Ok(process) => process,
        Err(_) if !windows::games()?.contains(&session.process_id) => {
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
        // A completed recorder has already observed done and module absence; the
        // named event ceases to exist when all process handles have closed.
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

/// A campaign waiting in a session's completion record. Unknown hours stay unknown.
fn waiting_json(campaign: &str, unsaved: Option<&replay_io::Unsaved>) -> Value {
    json!({
        "campaign": campaign,
        "hours": unsaved.map(|unsaved| unsaved.hours),
        "first_hour": unsaved.and_then(|unsaved| unsaved.first_hour),
        "last_hour": unsaved.and_then(|unsaved| unsaved.last_hour),
    })
}

/// A campaign whose recorded hours wait in an ended session's folder for Save or Discard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingCampaign {
    pub campaign: String,
    /// The folder its recording is saved into.
    pub target: PathBuf,
    pub hours: Option<u64>,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
}

/// The session folders under a capture root.
pub fn session_folders(output: &Path) -> Vec<PathBuf> {
    fs::read_dir(output.join("sessions"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

/// Lists a session's waiting campaigns before it stages them, until its completion record does.
const WAITING_FILE: &str = "waiting.json";

/// The record that lists a session's waiting campaigns: its completion record once written.
fn waiting_record(session: &Path) -> PathBuf {
    let completed = session.join("completed.json");
    if completed.exists() {
        completed
    } else {
        session.join(WAITING_FILE)
    }
}

/// The campaigns an ended session left waiting, as its completion record lists them.
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

/// Take a campaign off its session's waiting list once saved or discarded.
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

/// The folder a waiting campaign of an ended session saves into.
pub fn waiting_folder(session: &Path, campaign: &str) -> Result<PathBuf, String> {
    waiting_target(session, campaign)
}

/// The ended sessions whose hours of a recording wait, oldest first.
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

/// Save a recording's hours waiting from ended sessions.
fn save_recording(
    root: &Path,
    target: &Path,
    campaign: &str,
    naming: replay_io::Naming<'_>,
) -> Result<Option<PathBuf>, String> {
    save_recording_except(root, target, campaign, naming, None)
}

fn save_recording_except(
    root: &Path,
    target: &Path,
    campaign: &str,
    naming: replay_io::Naming<'_>,
    current: Option<&Path>,
) -> Result<Option<PathBuf>, String> {
    replay_io::with_compression(naming.settings, || {
        let mut saved = match current {
            Some(session) => replay_io::publish_checkpointed(session, target, campaign, naming)?,
            None => None,
        };
        // Each session's spool, in order: hours already staged are matched, any not staged are added.
        for session in waiting_sessions(root, target, campaign)
            .into_iter()
            .filter(|session| Some(session.as_path()) != current)
        {
            let (replay, _) = replay_io::save_campaign(
                &session,
                target,
                campaign,
                false,
                &|| Ok(()),
                &mut replay_io::Ahead::default(),
                naming,
            )
            .map_err(|error| format!("{error}; spool retained"))?;
            saved = replay.or(saved);
            unlist_waiting(&session, campaign)?;
            replay_io::reconcile_session_checkpoints(&session, target)?;
        }
        // Staged by a session that ended before listing it.
        Ok(replay_io::publish_unsaved(target, campaign, naming)?.or(saved))
    })
}

/// Whether a session publishes hours of a campaign: one it admitted a load of. `admitted` is
/// `None` for the command line, which publishes every campaign.
fn publishes(admitted: Option<&replay_io::Admissions>, campaign: &str) -> bool {
    admitted.is_none_or(|admitted| admitted.loads.contains_key(campaign))
}

/// At a session's end, save first the hours ended sessions left waiting of each campaign it
/// `saves` and publishes, spooled or admitted without an hour. Returns the recordings saved of
/// those without an hour, and each campaign whose hours could not be saved, with why.
fn save_continued(
    root: &Path,
    current: Option<&Path>,
    target: &Path,
    spooled: &[String],
    admitted: Option<&replay_io::Admissions>,
    saves: &dyn Fn(&str) -> bool,
    naming: replay_io::Naming<'_>,
) -> (Vec<PathBuf>, BTreeMap<String, String>) {
    let (mut continued, mut blocked) = (Vec::new(), BTreeMap::new());
    let unspooled = admitted
        .into_iter()
        .flat_map(|admitted| admitted.loads.keys())
        .filter(|campaign| !spooled.contains(campaign));
    for campaign in spooled
        .iter()
        .chain(unspooled)
        .filter(|campaign| publishes(admitted, campaign) && saves(campaign))
    {
        match save_recording_except(root, target, campaign, naming, current) {
            Ok(saved) if !spooled.contains(campaign) => continued.extend(saved),
            Ok(_) => {}
            Err(error) => {
                blocked.insert(campaign.clone(), error);
            }
        }
    }
    (continued, blocked)
}

/// Stage every ended session's waiting hours into their unsaved recordings, oldest first. Returns
/// each campaign that could not be staged, with why: its loads are left out, since a capture of it
/// would continue a history without those hours, and the others record.
fn stage_waiting(root: &Path, target: &Path) -> Result<BTreeMap<String, String>, String> {
    replay_io::with_compression(&RecorderSettings::path(), || {
        fs::create_dir_all(target).map_err(|error| failure_text(target, error))?;
        let mut refused = BTreeMap::new();
        let _lock = finalization_lock(target)?;
        for session in session_folders(root) {
            replay_io::reconcile_session_checkpoints(&session, target)?;
        }
        let campaigns: BTreeSet<_> = session_folders(root)
            .iter()
            .flat_map(|session| waiting_campaigns(session))
            .filter(|waiting| waiting.target == target)
            .map(|waiting| waiting.campaign)
            .collect();
        if campaigns.is_empty() {
            return Ok(refused);
        }
        // Staging names no recording.
        let settings = RecorderSettings::path();
        let naming = replay_io::Naming {
            localisation: None,
            settings: &settings,
            report: None,
        };
        for campaign in campaigns {
            for session in waiting_sessions(root, target, &campaign) {
                if let Err(error) = replay_io::save_campaign(
                    &session,
                    target,
                    &campaign,
                    true,
                    &|| Ok(()),
                    &mut replay_io::Ahead::default(),
                    naming,
                ) {
                    refused.insert(
                        campaign.clone(),
                        format!("{error}; save or discard them in Recordings"),
                    );
                    break;
                }
            }
        }
        Ok(refused)
    })
}

/// Drop a recording's hours waiting from ended sessions, keeping what it saved.
fn discard_recording(root: &Path, target: &Path, campaign: &str) -> Result<(), String> {
    let sessions = waiting_sessions(root, target, campaign);
    for session in &sessions {
        replay_io::reconcile_session_checkpoints(session, target)?;
    }
    replay_io::discard_unsaved_recording(target, campaign)?;
    for session in sessions {
        replay_io::discard_unsaved(&session, campaign)?;
        unlist_waiting(&session, campaign)?;
    }
    Ok(())
}

fn root_of(session: &Path) -> Result<&Path, String> {
    session
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| failure_text(session, "not in a capture root"))
}

/// Where a capture reads the game's content while no game runs: the game installation Steam
/// records, and the Paradox user folder and content hash cache a capture uses.
fn installed_content() -> Result<installed_content_core::ResolveOptions, String> {
    let settings = RecorderSettings::load()
        .map_err(|error| format!("the recorder settings could not be read: {error}"))?;
    let user_dir = match settings.user_folder {
        Some(folder) => folder,
        None => resolve_user_directory()?,
    };
    let game_dir = crate::directories::default_game_directory()
        .map_err(|_| "Steam lists no the game installation".to_owned())?;
    Ok(installed_content_core::ResolveOptions {
        game_dir,
        user_dir,
        cache_dir: replay::recorder_settings::directory(),
    })
}

type InstallationKey = Result<(PathBuf, PathBuf), String>;
type SettingsStamp = Option<(SystemTime, u64)>;
type InstalledContent = Result<installed_content_core::ResolveOptions, String>;

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
    resolved: Option<installed_content_core::ResolvedContent>,
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
                let resolved = installed_content_core::resolve(&self.identity, options)
                    .and_then(installed_content_core::ResolvedContent::exact)
                    .map_err(|error| error.to_string())?;
                self.resolved = Some(resolved);
            }
            #[cfg(test)]
            {
                self.reads += 1;
            }
            installed_content_localisation::localisation::read(
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

/// Names recordings ended sessions left waiting as [`save_waiting`] saves them, each through its
/// own content stack, read once per stack and installation.
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

    /// Names the whole waiting set, dropping stacks no recording still uses.
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

    /// The localisation that names the countries of what `session` left waiting, or why none does.
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

    /// The file name [`save_waiting`] would give the recording `session` left waiting of
    /// `campaign`; `None` while it is not known.
    pub fn name(&mut self, session: &Path, campaign: &str) -> Option<String> {
        self.prediction(session, campaign).name
    }

    pub fn prediction(&mut self, session: &Path, campaign: &str) -> WaitingName {
        let mut retry = true;
        let name = (|| {
            let target = waiting_target(session, campaign).ok()?;
            let root = root_of(session).ok()?;
            let sessions = waiting_sessions(root, &target, campaign);
            let settings = RecorderSettings::path();
            let localisation = self.localisation(session);
            let by_tag = AtomicBool::new(false);
            let report = |why: Option<&str>| by_tag.store(why.is_some(), Ordering::Relaxed);
            let naming = replay_io::Naming {
                localisation: localisation.as_ref().ok().copied(),
                settings: &settings,
                report: Some(&report),
            };
            let name = replay_io::waiting_name(&sessions, &target, campaign, naming)?;
            retry = localisation.is_err() || by_tag.load(Ordering::Relaxed);
            Some(name)
        })();
        WaitingName { name, retry }
    }
}

/// A recording saved from the Recordings view.
#[derive(Debug, PartialEq, Eq)]
pub struct SavedWaiting {
    /// `None` when no admitted hour of it waited.
    pub recording: Option<PathBuf>,
    /// Why its country uses a tag: content resolution or a named localisation refusal.
    pub by_tag: Option<String>,
}

/// Save a recording an ended session left waiting, with every ended session's hours of it, its
/// countries named through its own content stack as a capture names them.
pub fn save_waiting(session: &Path, campaign: &str) -> Result<SavedWaiting, String> {
    save_waiting_with(session, campaign, &mut WaitingNames::installed())
}

fn save_waiting_with(
    session: &Path,
    campaign: &str,
    names: &mut WaitingNames,
) -> Result<SavedWaiting, String> {
    replay_io::with_compression(&RecorderSettings::path(), || {
        let target = waiting_target(session, campaign)?;
        let localisation = names.localisation(session);
        if let Err(why) = &localisation {
            eprintln!("Countries are named by tag: {why}");
        }
        fs::create_dir_all(&target).map_err(|error| failure_text(&target, error))?;
        let _lock = finalization_lock(&target)?;
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
        let recording = save_recording(root_of(session)?, &target, campaign, naming)?;
        let by_tag = localisation.err().or_else(|| {
            refusal
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        });
        Ok(SavedWaiting { recording, by_tag })
    })
}

/// Discard a recording's waiting hours from every ended session; what it saved stays.
pub fn discard_waiting(session: &Path, campaign: &str) -> Result<(), String> {
    let target = waiting_target(session, campaign)?;
    fs::create_dir_all(&target).map_err(|error| failure_text(&target, error))?;
    let _lock = finalization_lock(&target)?;
    discard_recording(root_of(session)?, &target, campaign)
}

#[cfg(test)]
mod tests {
    #[test]
    fn packing_escalation_uses_work_and_closed_blocks_with_hysteresis_and_a_hold() {
        let mut policy = PackingCatchUp::default();
        let at = Duration::from_secs;
        let backlog = |rows, closed| replay_io::PackingBacklog {
            runnable: replay_io::PackingDebt {
                work: replay::container::Outstanding {
                    rows,
                    ..Default::default()
                },
                closed_blocks: closed,
                ..Default::default()
            },
            ..Default::default()
        };
        for seconds in 0..100 {
            assert!(!policy.update(at(seconds), Some(&backlog(45_917_453, 1))));
        }
        assert!(policy.update(at(100), Some(&backlog(100_000_000, 0))));
        assert!(policy.update(at(101), Some(&backlog(0, 0))));
        assert!(policy.update(at(109), Some(&backlog(0, 0))));
        assert!(policy.update(at(110), Some(&backlog(30_000_000, 0))));
        assert!(policy.update(at(111), Some(&backlog(0, 1))));
        assert!(!policy.update(at(112), Some(&backlog(20_000_000, 0))));
        assert!(!policy.update(at(113), Some(&backlog(60_000_000, 1))));
        assert!(policy.update(at(114), Some(&backlog(0, 2))));
        assert!(policy.update(at(123), Some(&backlog(0, 0))));
        assert!(!policy.update(at(124), Some(&backlog(0, 0))));
    }

    #[test]
    fn blocked_debt_cannot_enter_or_hold_catch_up_after_healthy_work_drains() {
        let mut policy = PackingCatchUp::default();
        let mut backlog = replay_io::PackingBacklog {
            blocked: replay_io::PackingDebt {
                work: replay::container::Outstanding {
                    rows: 1_500_000_000,
                    compression: 500_000_000,
                    readback: 500_000_000,
                    open: 500_000_000,
                },
                unwritten_hours: 4000,
                closed_blocks: 6,
            },
            ..Default::default()
        };
        let at = Duration::from_secs;
        assert!(!policy.update(at(0), Some(&backlog)));
        backlog.runnable.work.rows = 100_000_000;
        backlog.runnable.closed_blocks = 2;
        assert!(policy.update(at(1), Some(&backlog)));
        backlog.runnable = Default::default();
        assert!(policy.update(at(10), Some(&backlog)));
        assert!(!policy.update(at(11), Some(&backlog)));
        for now in 12..100 {
            assert!(!policy.update(at(now), Some(&backlog)));
        }
        backlog.runnable.closed_blocks = 2;
        assert!(policy.update(at(100), Some(&backlog)));
        assert!(!policy.update(at(101), None));
        assert!(policy.since.is_none());
    }

    #[test]
    fn unknown_backlog_leaves_catch_up_without_undoing_stop_promotion() {
        let directory = crate::test_support::allocation_test_directory("unknown-packing-policy");
        let session = directory.join("session");
        let target = directory.join("target");
        replay_io::begin_admissions(&session, &target).unwrap();
        fs::write(session.join(replay_io::ADMISSIONS_FILE), b"not json").unwrap();
        let ahead = replay_io::Ahead::default();
        let packing = replay::container::Packing::new().unwrap();
        let mut policy = PackingCatchUp::default();
        let mut last = None;
        packing.run(|| {
            packing.catch_up(true);
            assert_eq!(packing.mode(), "catch_up_normal");
            assert_eq!(
                packing_progress(
                    &packing,
                    &ahead,
                    &session,
                    &mut policy,
                    Duration::from_secs(1),
                    &mut last
                ),
                None
            );
            assert_eq!(packing.mode(), "background");
            assert!(policy.since.is_none());
            packing.promote();
            assert_eq!(
                packing_progress(
                    &packing,
                    &ahead,
                    &session,
                    &mut policy,
                    Duration::from_secs(2),
                    &mut last
                ),
                None
            );
            assert_eq!(packing.mode(), "foreground");
        });
        fs::remove_dir_all(directory).unwrap();
    }

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

    #[test]
    fn continuous_commit_succeeds_without_any_collector_or_callback_endpoint() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, recorded_frames,
            write_load_segment,
        };
        let root = allocation_test_directory("continuous-no-collector");
        let session = root.join("session");
        let target = root.join("recordings");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        let first = write_load_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        write_load_segment(&campaign, 1, 10, 5..=6, RELOAD);
        let mut ahead = replay_io::Ahead::default();
        ahead.read(&session, &target).unwrap();
        ahead.write(&session, &target, &|| false).unwrap();
        assert_eq!(ahead.completed_through(&campaign), Some(0));
        Checkpoints::default().run(
            &session,
            &target,
            &mut ahead,
            None,
            unnamed(),
            &AtomicBool::new(false),
        );
        assert!(!first.exists());
        let saved = replay::recording_files::recording_of(&target, ALLOCATION_CAMPAIGN)
            .unwrap()
            .unwrap();
        assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
        assert_eq!(ahead.written(), 2);
    }

    #[test]
    fn continuous_packing_has_no_speed_gate_or_collector_request() {
        let source = include_str!("recording.rs");
        let checkpoints = source
            .split("impl Checkpoints {")
            .nth(1)
            .unwrap()
            .split("/// A campaign the running session set aside")
            .next()
            .unwrap();
        assert!(!checkpoints.contains("ask_collector"));
        assert!(!checkpoints.contains("with_lent_history"));
        assert!(checkpoints.contains("completed_through"));
    }

    use super::*;
    use replay::container::Replay;
    use std::{cell::RefCell, collections::BTreeMap};

    #[test]
    fn a_sessions_end_saves_every_campaign_with_auto_save_and_otherwise_only_what_stop_saves() {
        let (current, other) = ("current", "other");
        let saved = |auto_save, stop, discard| {
            [current, other]
                .map(|campaign| saved_at_end(campaign, Some(current), auto_save, stop, discard))
        };
        // Auto save: the game closing, Stop's Save, or a refused load saves everything.
        assert_eq!(saved(true, false, false), [true, true]);
        assert_eq!(saved(true, true, false), [true, true]);
        // Discard drops the campaign in progress; Auto save still saves the rest.
        assert_eq!(saved(true, true, true), [false, true]);
        // Without Auto save, only Stop's Save saves, and only the campaign in progress.
        assert_eq!(saved(false, false, false), [false, false]);
        assert_eq!(saved(false, true, false), [true, false]);
        assert_eq!(saved(false, true, true), [false, false]);
    }

    fn take_done(settling: &Settling) -> Vec<SetAside> {
        std::mem::take(&mut *locked(&settling.done))
    }

    /// Saved from the Recordings view without the game's content, so countries are named by tag.
    fn saved_waiting(session: &Path, campaign: &str) -> Result<SavedWaiting, String> {
        save_waiting_with(
            session,
            campaign,
            &mut WaitingNames::new(Err("no installation in this test".into())),
        )
    }

    /// Names by tag, with no name chosen.
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
    fn a_campaign_the_game_left_is_saved_while_another_records_or_waits_without_auto_save() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            recorded_frames, write_load_segment,
        };
        const RECORDING: &str = "00000000-0000-4000-8000-000000000001";
        let directory = allocation_test_directory("settle-set-aside");
        let session = directory.join("session");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        replay_io::begin_admissions(&session, &target).unwrap();
        let mut admissions = replay_io::Admissions::default();
        for load in [FIRST_LOAD, RELOAD, THIRD_LOAD, THIRD_LOAD + 1] {
            admissions.admit(&session, &target, name, load).unwrap();
        }
        // The game left the first campaign; the collector writes the other.
        write_load_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        let recording = session.join(RECORDING).join("spool.0000.h4s");
        fs::create_dir_all(recording.parent().unwrap()).unwrap();
        let mut writer = File::create(&recording).unwrap();
        let settling = Settling::default();
        settling.save.store(true, Ordering::Release);
        let mut ahead = replay_io::Ahead::default();
        let mut settled = Settled::new();
        let mut settle_now = |ahead: &mut replay_io::Ahead| {
            ahead.read(&session, &target).unwrap();
            settle(&session, &target, ahead, &settling, &mut settled, unnamed()).unwrap();
        };
        settle_now(&mut ahead);
        let saved = target.join("Campaign.battleplan");
        assert_eq!(
            take_done(&settling),
            [SetAside::Saved(name.to_owned(), Some(saved.clone()))]
        );
        assert_eq!(recorded_frames(&saved), [(9, 2), (9, 3), (9, 4)]);
        // The campaign being recorded is neither saved nor read.
        assert_eq!(
            replay::recording_files::recording_of(&target, RECORDING.as_bytes()).unwrap(),
            None
        );
        std::io::Write::write_all(&mut writer, b"still recording").unwrap();
        assert_eq!(locked(&settling.waiting).take(), Some(Vec::new()));
        // Nothing changed: nothing is saved again.
        settle_now(&mut ahead);
        assert!(take_done(&settling).is_empty());
        // Loaded again and left again: exactly its new hours are appended.
        write_load_segment(&campaign, 1, 10, 3..=4, RELOAD);
        settle_now(&mut ahead);
        assert_eq!(
            take_done(&settling),
            [SetAside::Saved(name.to_owned(), Some(saved.clone()))]
        );
        let both = [(9, 2), (9, 3), (9, 4), (10, 3), (10, 4)];
        assert_eq!(recorded_frames(&saved), both);
        // Auto save turned off: the next time it is left, it waits.
        settling.save.store(false, Ordering::Release);
        write_load_segment(&campaign, 2, 11, 5..=6, THIRD_LOAD);
        settle_now(&mut ahead);
        assert!(take_done(&settling).is_empty());
        let waiting = locked(&settling.waiting).take().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(
            (
                waiting[0].campaign.as_str(),
                waiting[0].unsaved.hours,
                waiting[0].saving
            ),
            (name, 2, false)
        );
        assert_eq!(recorded_frames(&saved), both);
        // Save pressed for it.
        locked(&settling.requests).push((name.to_owned(), false));
        settle_now(&mut ahead);
        assert_eq!(
            take_done(&settling),
            [SetAside::Saved(name.to_owned(), Some(saved.clone()))]
        );
        let three = [(9, 2), (9, 3), (9, 4), (10, 3), (10, 4), (11, 5), (11, 6)];
        assert_eq!(recorded_frames(&saved), three);
        // Discard pressed for the next: only its own hours go.
        let discarded = write_load_segment(&campaign, 3, 12, 7..=8, THIRD_LOAD + 1);
        settle_now(&mut ahead);
        locked(&settling.requests).push((name.to_owned(), true));
        settle_now(&mut ahead);
        assert_eq!(take_done(&settling), [SetAside::Discarded(name.to_owned())]);
        assert!(!discarded.exists());
        assert_eq!(recorded_frames(&saved), three);
        assert_eq!(fs::read(&recording).unwrap(), b"still recording");
        drop(writer);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saving_a_set_aside_checkpoint_manually_or_with_auto_save_keeps_the_session_usable() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            recorded_frames, write_load_segment,
        };
        use replay::spool::{Record, encode_record, recover_prefix};
        use replay_data::rows::{CampaignRow, TableId, TableRows};
        use replay_frame::frame::TableUpdate;

        const CURRENT: &str = "00000000-0000-4000-8000-000000000001";
        for auto_save in [false, true] {
            for earlier_waiting in [false, true] {
                for context_only in [false, true] {
                    let directory = allocation_test_directory(&format!(
                        "settle-checkpoint-save-{auto_save}-{earlier_waiting}-{context_only}"
                    ));
                    let target = directory.join("target");
                    let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
                    let earlier = earlier_waiting.then(|| {
                        let earlier =
                            ended_waiting(&directory, "1-1", name, 8, 1..=1, FIRST_LOAD - 1);
                        assert!(stage_waiting(&directory, &target).unwrap().is_empty());
                        earlier
                    });
                    let session = directory.join("sessions/1-2");
                    let campaign = session.join(name);
                    replay_io::begin_admissions(&session, &target).unwrap();
                    let mut admissions = replay_io::Admissions::default();
                    for load in [FIRST_LOAD, RELOAD] {
                        admissions.admit(&session, &target, name, load).unwrap();
                    }
                    admissions
                        .admit(&session, &target, CURRENT, THIRD_LOAD)
                        .unwrap();
                    let first = write_load_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
                    if context_only {
                        let context = recover_prefix(&fs::read(&first).unwrap());
                        assert!(context.damage.is_none());
                        fs::write(
                            campaign.join("spool.0001.h4s"),
                            context
                                .records
                                .iter()
                                .filter(|record| !matches!(record, Record::Frame(_)))
                                .flat_map(|record| encode_record(record).unwrap())
                                .collect::<Vec<_>>(),
                        )
                        .unwrap();
                    } else {
                        write_load_segment(&campaign, 1, 10, 5..=6, RELOAD);
                    }
                    let mut ahead = replay_io::Ahead::default();
                    let checkpoint = replay_io::checkpoint_campaign(
                        &session,
                        &target,
                        name,
                        0,
                        false,
                        &mut ahead,
                        unnamed(),
                        &|| Ok(()),
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!(checkpoint.deleted, 1);
                    assert!(campaign.join("checkpoint.json").exists());
                    let current = session.join(CURRENT);
                    for (number, branch, hours) in [(0, 11, 7..=8), (1, 12, 9..=10)] {
                        let path = write_load_segment(&current, number, branch, hours, THIRD_LOAD);
                        let mut recovered = recover_prefix(&fs::read(&path).unwrap());
                        assert!(recovered.damage.is_none());
                        for record in &mut recovered.records {
                            if let Record::Frame(frame) = record {
                                frame.tables[TableId::Campaign.index()] =
                                    TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                                        id: CURRENT.as_bytes().to_vec(),
                                    }]));
                            }
                        }
                        fs::write(
                            &path,
                            recovered
                                .records
                                .iter()
                                .flat_map(|record| encode_record(record).unwrap())
                                .collect::<Vec<_>>(),
                        )
                        .unwrap();
                    }
                    let settling = Settling::default();
                    *locked(&settling.loaded) = Some(CURRENT.to_owned());
                    let mut settled = Settled::new();
                    settle(
                        &session,
                        &target,
                        &mut ahead,
                        &settling,
                        &mut settled,
                        unnamed(),
                    )
                    .unwrap();
                    assert!(take_done(&settling).is_empty());
                    assert!(settled[name].1.is_some());
                    if auto_save {
                        settling.save.store(true, Ordering::Release);
                    } else {
                        locked(&settling.requests).push((name.to_owned(), false));
                    }
                    settle(
                        &session,
                        &target,
                        &mut ahead,
                        &settling,
                        &mut settled,
                        unnamed(),
                    )
                    .unwrap();
                    let saved = replay::recording_files::recording_of(&target, ALLOCATION_CAMPAIGN)
                        .unwrap()
                        .unwrap();
                    assert_eq!(
                        take_done(&settling),
                        [SetAside::Saved(name.to_owned(), Some(saved.clone()))]
                    );
                    let mut expected = vec![(9, 2), (9, 3), (9, 4)];
                    if earlier_waiting {
                        expected.insert(0, (8, 1));
                    }
                    if !context_only {
                        expected.extend([(10, 5), (10, 6)]);
                    }
                    assert_eq!(recorded_frames(&saved), expected);
                    assert!(
                        replay::recording_files::recording_of(&target, CURRENT.as_bytes())
                            .unwrap()
                            .is_none()
                    );
                    assert!(!checkpoint.recording.exists());
                    assert!(!campaign.join("checkpoint.json").exists());
                    if let Some(earlier) = earlier {
                        assert!(waiting_campaigns(&earlier).is_empty());
                    }
                    let next = replay_io::checkpoint_campaign(
                        &session,
                        &target,
                        CURRENT,
                        0,
                        auto_save,
                        &mut ahead,
                        unnamed(),
                        &|| Ok(()),
                    )
                    .unwrap()
                    .unwrap();
                    assert_eq!((next.hours, next.deleted), (2, 1));
                    let done = replay_io::finalize_session_ahead(
                        &session,
                        &target,
                        &|| Ok(()),
                        ahead,
                        unnamed(),
                        &|_| replay_io::Destination::Recording,
                    )
                    .unwrap();
                    assert!(done.errors.is_empty(), "{:?}", done.errors);
                    assert_eq!(recorded_frames(&saved), expected);
                    let current_saved =
                        replay::recording_files::recording_of(&target, CURRENT.as_bytes())
                            .unwrap()
                            .unwrap();
                    assert_eq!(
                        recorded_frames(&current_saved),
                        [(11, 7), (11, 8), (12, 9), (12, 10)]
                    );
                    replay_io::write_session_end(
                        &session, &target, "completed.json",
                        &json!({"target": target, "replays": [saved, current_saved], "waiting": []}),
                    )
                    .unwrap();
                    assert!(session.join("completed.json").exists());
                    assert!(!current.join("checkpoint.json").exists());
                    fs::remove_dir_all(directory).unwrap();
                }
            }
        }
    }

    #[test]
    fn saving_older_checkpoint_only_waiting_sessions_proves_their_revisions_in_the_grown_copy() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, recorded_frames, write_load_segment,
        };
        use replay::spool::{Record, encode_record, recover_prefix};

        for published_first in [false, true] {
            let directory = allocation_test_directory(&format!(
                "waiting-checkpoint-ancestors-{published_first}"
            ));
            let target = directory.join("target");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let mut sessions = Vec::new();
            for (id, branch, hours, load) in [
                ("1-1", 8, 1..=2, 17),
                ("1-2", 9, 3..=4, 19),
                ("1-3", 10, 5..=6, 21),
            ] {
                let session = directory.join("sessions").join(id);
                let campaign = session.join(name);
                replay_io::begin_admissions(&session, &target).unwrap();
                replay_io::select_recordings(&session, &target).unwrap();
                replay_io::Admissions::default()
                    .admit(&session, &target, name, load)
                    .unwrap();
                let first = write_load_segment(&campaign, 0, branch, hours, load);
                let recovered = recover_prefix(&fs::read(&first).unwrap());
                assert!(recovered.damage.is_none());
                let tail = campaign.join("spool.0001.h4s");
                fs::write(
                    &tail,
                    recovered
                        .records
                        .iter()
                        .filter(|record| !matches!(record, Record::Frame(_)))
                        .flat_map(|record| encode_record(record).unwrap())
                        .collect::<Vec<_>>(),
                )
                .unwrap();
                let checkpoint = replay_io::checkpoint_campaign(
                    &session,
                    &target,
                    name,
                    0,
                    false,
                    &mut replay_io::Ahead::default(),
                    unnamed(),
                    &|| Ok(()),
                )
                .unwrap()
                .unwrap();
                assert_eq!((checkpoint.hours, checkpoint.deleted), (2, 1));
                assert!(!first.exists());
                assert_eq!(replay_io::segment_paths(&campaign).unwrap(), [(1, tail)]);
                let waiting = replay_io::unsaved(&session, name).unwrap().unwrap();
                replay_io::write_session_end(
                    &session,
                    &target,
                    "completed.json",
                    &json!({"target": target, "waiting": [waiting_json(name, Some(&waiting))]}),
                )
                .unwrap();
                assert!(!campaign.join("checkpoint.json").exists());
                sessions.push(session);
            }
            let staged = replay::publication::unsaved_path(&target, name);
            let bytes = fs::read(&staged).unwrap();
            if published_first {
                assert!(
                    replay_io::publish_unsaved(&target, name, unnamed())
                        .unwrap()
                        .is_some()
                );
            }
            let saved = save_waiting(&sessions[0], name).unwrap().recording.unwrap();
            assert_eq!(
                fs::read(&saved).unwrap(),
                bytes,
                "no hours or metadata added"
            );
            assert_eq!(
                recorded_frames(&saved),
                [(8, 1), (8, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
            );
            let revision = replay::container::Replay::open_path(&saved)
                .unwrap()
                .revision();
            for session in sessions {
                assert!(waiting_campaigns(&session).is_empty());
                assert!(
                    replay_io::segment_paths(&session.join(name))
                        .unwrap()
                        .is_empty()
                );
                let bookkeeping = read_status(&session.join(name).join("saved.json")).unwrap();
                assert_eq!(bookkeeping["replay"], json!(saved));
                assert_eq!(
                    bookkeeping["revision"],
                    json!([
                        revision.end,
                        revision.trailer,
                        revision.length,
                        revision.crc,
                    ])
                );
                assert_eq!(
                    replay_io::save_campaign(
                        &session,
                        &target,
                        name,
                        false,
                        &|| Ok(()),
                        &mut replay_io::Ahead::default(),
                        unnamed(),
                    )
                    .unwrap()
                    .0,
                    Some(saved.clone())
                );
                assert_eq!(fs::read(&saved).unwrap(), bytes);
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn discarding_a_set_aside_campaign_keeps_published_hours_and_drops_unsaved_checkpoints() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            write_load_segment,
        };
        let directory = allocation_test_directory("settle-checkpoint-discard");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let earlier = directory.join("earlier");
        write_load_segment(&earlier.join(name), 0, 8, 1..=1, FIRST_LOAD - 1);
        let saved = replay_io::finalize_session(&earlier, &target)
            .unwrap()
            .remove(0);
        let session = directory.join("sessions/current");
        let campaign = session.join(name);
        write_load_segment(&campaign, 0, 9, 2..=4, FIRST_LOAD);
        write_load_segment(&campaign, 1, 10, 5..=6, RELOAD);
        replay_io::checkpoint_campaign(
            &session,
            &target,
            name,
            0,
            true,
            &mut replay_io::Ahead::default(),
            unnamed(),
            &|| Ok(()),
        )
        .unwrap();
        let published = fs::read(&saved).unwrap();
        assert_eq!(
            crate::test_support::recorded_frames(&saved),
            [(8, 1), (9, 2), (9, 3), (9, 4)]
        );
        write_load_segment(&campaign, 2, 11, 7..=8, THIRD_LOAD);
        replay_io::checkpoint_campaign(
            &session,
            &target,
            name,
            1,
            false,
            &mut replay_io::Ahead::default(),
            unnamed(),
            &|| Ok(()),
        )
        .unwrap();
        let staged = replay::publication::unsaved_path(&target, name);
        assert!(staged.exists());
        let settling = Settling::default();
        locked(&settling.requests).push((name.to_owned(), true));
        settle(
            &session,
            &target,
            &mut replay_io::Ahead::default(),
            &settling,
            &mut Settled::new(),
            unnamed(),
        )
        .unwrap();
        assert_eq!(take_done(&settling), [SetAside::Discarded(name.to_owned())]);
        assert_eq!(fs::read(&saved).unwrap(), published);
        assert_eq!(
            crate::test_support::recorded_frames(&saved),
            [(8, 1), (9, 2), (9, 3), (9, 4)]
        );
        assert!(!staged.exists());
        assert!(replay_io::segment_paths(&campaign).unwrap().is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn waiting_discard_keeps_published_checkpoints_after_final_staging_and_later_sessions() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, write_load_segment,
        };
        for later in [false, true] {
            let directory =
                allocation_test_directory(&format!("waiting-checkpoint-discard-{later}"));
            let target = directory.join("target");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let earlier = directory.join("earlier");
            write_load_segment(&earlier.join(name), 0, 8, 1..=1, 17);
            let saved = replay_io::finalize_session(&earlier, &target)
                .unwrap()
                .remove(0);
            let session = directory.join("sessions/1-1");
            let campaign = session.join(name);
            write_load_segment(&campaign, 0, 9, 2..=4, 19);
            write_load_segment(&campaign, 1, 10, 5..=6, 21);
            replay_io::checkpoint_campaign(
                &session,
                &target,
                name,
                0,
                true,
                &mut replay_io::Ahead::default(),
                unnamed(),
                &|| Ok(()),
            )
            .unwrap();
            let original = fs::read(&saved).unwrap();
            write_load_segment(&campaign, 2, 11, 7..=8, 23);
            replay_io::checkpoint_campaign(
                &session,
                &target,
                name,
                1,
                false,
                &mut replay_io::Ahead::default(),
                unnamed(),
                &|| Ok(()),
            )
            .unwrap();
            let stage = |session: &Path| {
                replay_io::save_campaign(
                    session,
                    &target,
                    name,
                    true,
                    &|| Ok(()),
                    &mut replay_io::Ahead::default(),
                    unnamed(),
                )
                .unwrap();
                let unsaved = replay_io::unsaved(session, name).unwrap();
                replay_io::write_session_end(
                    session,
                    &target,
                    "completed.json",
                    &json!({"target": target, "waiting": [waiting_json(name, unsaved.as_ref())]}),
                )
                .unwrap();
            };
            stage(&session);
            assert!(!campaign.join("checkpoint.json").exists());
            let next = directory.join("sessions/1-2");
            if later {
                write_load_segment(&next.join(name), 0, 12, 9..=10, 25);
                stage(&next);
            }
            discard_waiting(&session, name).unwrap();
            assert_eq!(fs::read(&saved).unwrap(), original);
            assert!(!replay::publication::unsaved_path(&target, name).exists());
            assert!(waiting_campaigns(&session).is_empty());
            assert!(replay_io::segment_paths(&campaign).unwrap().is_empty());
            if later {
                assert!(waiting_campaigns(&next).is_empty());
                assert!(
                    replay_io::segment_paths(&next.join(name))
                        .unwrap()
                        .is_empty()
                );
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn waiting_hours_never_staged_or_never_completed_are_still_saved() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            recorded_frames, write_load_segment,
        };
        let directory = allocation_test_directory("waiting-unstaged");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let recording = target.join("Campaign.battleplan");
        // `staged`: the session staged its hours; `record`: the file that lists them waiting.
        let session = |folder: &str, branch, load, hours, staged: bool, record: &str| {
            let session = directory.join("sessions").join(folder);
            replay_io::begin_admissions(&session, &target).unwrap();
            replay_io::Admissions::default()
                .admit(&session, &target, name, load)
                .unwrap();
            write_load_segment(&session.join(name), 0, branch, hours, load);
            let unsaved = replay_io::unsaved(&session, name).unwrap();
            if staged {
                replay_io::save_campaign(
                    &session,
                    &target,
                    name,
                    true,
                    &|| Ok(()),
                    &mut replay_io::Ahead::default(),
                    unnamed(),
                )
                .unwrap();
            }
            write_json(
                &session.join(record),
                &json!({"target": target, "waiting": [waiting_json(name, unsaved.as_ref())]}),
            )
            .unwrap();
            session
        };
        let first = session("1-1", 9, FIRST_LOAD, 2..=4, true, "completed.json");
        // Its staging failed: its hours are only in its spool.
        let second = session("1-2", 10, RELOAD, 5..=6, false, "completed.json");
        let both = [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)];
        // A capture starting now stages them first, so it continues every waiting hour.
        assert!(stage_waiting(&directory, &target).unwrap().is_empty());
        assert_eq!(
            recorded_frames(&replay::publication::unsaved_path(&target, name)),
            both
        );
        assert_eq!(waiting_campaigns(&second).len(), 1);
        assert_eq!(
            saved_waiting(&first, name).unwrap().recording,
            Some(recording.clone())
        );
        assert_eq!(recorded_frames(&recording), both);
        assert!(waiting_campaigns(&first).is_empty() && waiting_campaigns(&second).is_empty());
        // Ended after staging but before its completion record: listed by what it wrote first.
        let third = session("1-3", 11, THIRD_LOAD, 7..=8, true, WAITING_FILE);
        assert_eq!(waiting_campaigns(&third).len(), 1);
        assert_eq!(
            saved_waiting(&third, name).unwrap().recording,
            Some(recording.clone())
        );
        assert_eq!(
            recorded_frames(&recording),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6), (11, 7), (11, 8)]
        );
        assert!(waiting_campaigns(&third).is_empty());
        assert!(!replay::publication::unsaved_path(&target, name).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_recordings_hours_waiting_in_ended_sessions_save_or_discard_together_from_disk() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            recorded_frames, write_load_segment,
        };
        let directory = allocation_test_directory("waiting-sessions");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let recording = target.join("Campaign.battleplan");
        // Each session ends with Auto save off: its hours are staged; only the disk knows.
        let session = |folder: &str, branch, load, hours: std::ops::RangeInclusive<i32>| {
            let session = directory.join("sessions").join(folder);
            replay_io::begin_admissions(&session, &target).unwrap();
            replay_io::Admissions::default()
                .admit(&session, &target, name, load)
                .unwrap();
            let first = *hours.start();
            write_load_segment(&session.join(name), 0, branch, hours.clone(), load);
            write_load_segment(&session.join(name), 1, branch, first..=first - 1, load);
            let checkpoint = replay_io::checkpoint_campaign(
                &session,
                &target,
                name,
                0,
                false,
                &mut replay_io::Ahead::default(),
                unnamed(),
                &|| Ok(()),
            )
            .unwrap()
            .unwrap();
            assert_eq!(checkpoint.deleted, 1);
            assert!(!session.join(name).join("spool.0000.h4s").exists());
            let unsaved = replay_io::unsaved(&session, name).unwrap();
            replay_io::save_campaign(
                &session,
                &target,
                name,
                true,
                &|| Ok(()),
                &mut replay_io::Ahead::default(),
                unnamed(),
            )
            .unwrap();
            replay_io::write_session_end(
                &session,
                &target,
                "completed.json",
                &json!({"target": target, "waiting": [waiting_json(name, unsaved.as_ref())]}),
            )
            .unwrap();
            assert!(!session.join(name).join("checkpoint.json").exists());
            session
        };
        let first = session("1-1", 9, FIRST_LOAD, 2..=4);
        let second = session("1-2", 10, RELOAD, 5..=6);
        assert_eq!(
            waiting_campaigns(&first),
            [WaitingCampaign {
                campaign: name.to_owned(),
                target: target.clone(),
                hours: Some(3),
                first_hour: Some(2),
                last_hour: Some(4),
            }]
        );
        assert!(!recording.exists());
        // Save from either: both sessions' hours, and neither lists it any more.
        assert_eq!(
            saved_waiting(&second, name).unwrap().recording,
            Some(recording.clone())
        );
        assert_eq!(
            recorded_frames(&recording),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
        );
        assert!(waiting_campaigns(&first).is_empty() && waiting_campaigns(&second).is_empty());
        assert!(
            saved_waiting(&first, name).is_err(),
            "nothing waits there now"
        );
        // Two later sessions, discarded: the saved sessions stay in the file.
        let bytes = fs::read(&recording).unwrap();
        let third = session("1-3", 11, THIRD_LOAD, 7..=8);
        let fourth = session("1-4", 12, THIRD_LOAD + 1, 9..=9);
        discard_waiting(&third, name).unwrap();
        for later in [&third, &fourth] {
            assert!(waiting_campaigns(later).is_empty());
            assert!(
                replay_io::segment_paths(&later.join(name))
                    .unwrap()
                    .is_empty()
            );
        }
        assert!(!replay::publication::unsaved_path(&target, name).exists());
        assert_eq!(fs::read(&recording).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_waiting_recording_is_named_through_its_own_mods_as_its_save_names_it() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, named_session_records,
            write_campaign_spool,
        };
        let directory = allocation_test_directory("waiting-localisation");
        let (game, user) = (directory.join("game"), directory.join("user"));
        use replay::spool::Record;
        let names_mod = directory.join("names mod");
        let descriptor = user.join("mod/names.mod");
        let mod_path = names_mod.to_string_lossy().replace('\\', "/");
        for (path, text) in [
            (game.join("hoi4.exe"), "synthetic executable".to_owned()),
            (user.join("settings.txt"), "language=english".to_owned()),
            (
                user.join("dlc_load.json"),
                r#"{"enabled_mods":["mod/names.mod"],"disabled_dlcs":[]}"#.to_owned(),
            ),
            (
                descriptor.clone(),
                format!("name=Names path=\"{mod_path}\""),
            ),
            // Only the mod names the country.
            (
                names_mod.join("localisation/names_l_english.yml"),
                "\u{feff}l_english:\n Z01_democratic:0 \"Republic of Nowhere\"\n Z01_fascism:0 \"[ROOT.SyntheticName]\"\n".to_owned(),
            ),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fs::create_dir_all(game.join("dlc")).unwrap();
        let cache = directory.join("cache");
        let executable =
            engine_executable::build::hash_executable(File::open(game.join("hoi4.exe")).unwrap())
                .unwrap();
        let identity = installed_content_core::gather(&installed_content_core::GatherOptions {
            game_dir: game.clone(),
            user_dir: user.clone(),
            cache_dir: cache.clone(),
            dlc_order: Some(Vec::new()),
        })
        .unwrap()
        .identity(
            "synthetic".into(),
            executable.sha256,
            executable.size,
            "test".into(),
            [0; 32],
        )
        .unwrap();
        assert_eq!(identity.mods.len(), 1);
        let content = || {
            Ok(installed_content_core::ResolveOptions {
                game_dir: game.clone(),
                user_dir: user.clone(),
                cache_dir: cache.clone(),
            })
        };
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // An ended session of Z01 under democrats whose hours wait staged, its content recorded.
        let waiting = |root: &str, group: &str| {
            let target = directory.join(root);
            let session = target.join("sessions").join("1-1");
            let mut records = named_session_records(7, 9, &[(5, group, 1)]);
            let Record::SessionStart(start) = &mut records[0] else {
                unreachable!()
            };
            start.identity = identity.clone();
            let mut start_bytes = Vec::new();
            start.encode(&mut start_bytes).unwrap();
            fs::create_dir_all(session.join("bin")).unwrap();
            write_json(
                &session.join("bin/collector.json"),
                &json!({"session_id": start.id, "session_start": start_bytes}),
            )
            .unwrap();
            create_failure_witness(&session, start.id, start.process_start_time).unwrap();
            write_campaign_spool(&session, &records);
            let unsaved = replay_io::unsaved(&session, name).unwrap();
            replay_io::save_campaign(
                &session,
                &target,
                name,
                true,
                &|| Ok(()),
                &mut replay_io::Ahead::default(),
                unnamed(),
            )
            .unwrap();
            write_json(
                &session.join("completed.json"),
                &json!({"target": target, "waiting": [waiting_json(name, unsaved.as_ref())]}),
            )
            .unwrap();
            session
        };
        let stem = |saved: &SavedWaiting| {
            saved
                .recording
                .as_ref()
                .and_then(|path| path.file_stem())
                .map(|stem| stem.to_string_lossy().into_owned())
        };
        // Named by the mod it recorded with, as the Recordings view says before Save.
        let named = waiting("named", "democratic");
        let mut cached = WaitingNames::new(content());
        let prediction = cached.prediction(&named, name);
        assert!(!prediction.retry);
        let predicted = prediction.name;
        assert_eq!(predicted.as_deref(), Some("Republic of Nowhere"));
        let saved = save_waiting_with(&named, name, &mut WaitingNames::new(content())).unwrap();
        assert_eq!(stem(&saved), predicted);
        assert_eq!(saved.by_tag, None);
        let scripted = waiting("scripted", "fascism");
        let prediction = cached.prediction(&scripted, name);
        assert!(prediction.retry);
        let predicted = prediction.name;
        assert_eq!(predicted.as_deref(), Some("Z01"));
        assert_eq!(cached.prediction(&scripted, name).name, predicted);
        assert_eq!(cached.stacks.len(), 1);
        assert_eq!(
            (cached.stacks[0].resolutions, cached.stacks[0].reads),
            (1, 1)
        );
        // A localisation read retry keeps its successfully resolved stack.
        let now = Instant::now();
        let stack = &mut cached.stacks[0];
        stack.localisation = Some(Err("synthetic read failure".into()));
        stack.retry = Some((now, NAMING_RETRY));
        stack.load(&content(), now + NAMING_RETRY);
        assert_eq!((stack.resolutions, stack.reads), (1, 2));
        assert!(stack.localisation.as_ref().unwrap().is_ok());
        cached.content = Err("another installation".into());
        assert!(cached.prediction(&scripted, name).retry);
        assert_eq!(cached.stacks.len(), 2);
        cached.content = content();
        assert_eq!(cached.prediction(&scripted, name).name, predicted);
        assert_eq!(
            (cached.stacks[0].resolutions, cached.stacks[0].reads),
            (1, 2)
        );
        let saved = save_waiting_with(&scripted, name, &mut WaitingNames::new(content())).unwrap();
        assert_eq!(stem(&saved), predicted);
        let why = saved.by_tag.unwrap();
        assert!(
            why.contains("Z01_fascism") && why.contains("unsupported scripted"),
            "{why}"
        );
        let localisation_file = names_mod.join("localisation/names_l_english.yml");
        let original_localisation = fs::read(&localisation_file).unwrap();
        let original_descriptor = fs::read(&descriptor).unwrap();
        for change in [
            "localisation",
            "descriptor",
            "missing-localisation",
            "missing-descriptor",
        ] {
            let session = waiting(change, "democratic");
            let saving = waiting(&format!("{change}-save"), "democratic");
            let batch = [(session.clone(), name.to_owned())];
            let mut revised = WaitingNames::new(content());
            assert_eq!(
                revised.predictions(&batch)[0].name.as_deref(),
                Some("Republic of Nowhere")
            );
            match change {
                "localisation" => fs::write(
                    &localisation_file,
                    "\u{feff}l_english:\n Z01_democratic:0 \"Changed Republic\"\n",
                )
                .unwrap(),
                "descriptor" => {
                    let mut bytes = original_descriptor.clone();
                    bytes.extend_from_slice(b"\nversion=changed");
                    fs::write(&descriptor, bytes).unwrap();
                }
                "missing-localisation" => fs::remove_file(&localisation_file).unwrap(),
                "missing-descriptor" => fs::remove_file(&descriptor).unwrap(),
                _ => unreachable!(),
            }
            revised.stacks[0].load(&content(), Instant::now() + NAMING_RETRY_MAX);
            let predicted = revised.predictions(&batch).remove(0);
            assert!(!predicted.retry, "{change}");
            assert_eq!(
                predicted.name.as_deref(),
                Some("Republic of Nowhere"),
                "{change}"
            );
            assert_eq!(
                (revised.stacks[0].resolutions, revised.stacks[0].reads),
                (1, 1)
            );
            let saved =
                save_waiting_with(&saving, name, &mut WaitingNames::new(content())).unwrap();
            assert_eq!(stem(&saved).as_deref(), Some("Z01"), "{change}");
            revised.predictions(&[]);
            assert!(revised.stacks.is_empty());
            let reopened = revised.predictions(&batch).remove(0);
            assert!(reopened.retry, "{change}");
            assert_eq!(reopened.name, stem(&saved), "{change}");
            assert!(revised.stacks[0].resolved.is_none());
            let why = saved.by_tag.unwrap();
            assert!(
                why.contains("hash mismatch") || why.contains("names.mod"),
                "{why}"
            );
            fs::write(&localisation_file, &original_localisation).unwrap();
            fs::write(&descriptor, &original_descriptor).unwrap();
            revised.predictions(&[]);
            assert!(revised.stacks.is_empty());
        }
        let retained = waiting("retained-stack", "democratic");
        cached.predictions(&[(retained, name.to_owned())]);
        assert_eq!(cached.stacks.len(), 1);
        cached.predictions(&[]);
        assert!(cached.stacks.is_empty());
        // With that mod gone, both name it by tag, and the save says which mod is missing.
        let descriptor_bytes = fs::read(&descriptor).unwrap();
        fs::remove_file(&descriptor).unwrap();
        let tagged = waiting("tagged", "democratic");
        let mut failed = WaitingNames::new(content());
        let prediction = failed.prediction(&tagged, name);
        assert!(prediction.retry);
        let predicted = prediction.name;
        assert_eq!(predicted.as_deref(), Some("Z01"));
        assert_eq!(failed.prediction(&tagged, name).name, predicted);
        let stack = &mut failed.stacks[0];
        assert_eq!((stack.resolutions, stack.reads), (1, 0));
        let mut now = stack.retry.unwrap().0;
        for delay in [30, 60, 120, 240, 480, 600, 600] {
            let delay = Duration::from_secs(delay);
            assert_eq!(stack.retry, Some((now, delay)));
            let attempts = stack.resolutions;
            stack.load(&content(), now + delay - Duration::from_nanos(1));
            assert_eq!(stack.resolutions, attempts);
            now += delay;
            stack.load(&content(), now);
            assert_eq!(stack.resolutions, attempts + 1);
        }
        let saved = save_waiting_with(&tagged, name, &mut WaitingNames::new(content())).unwrap();
        assert_eq!(stem(&saved), predicted);
        let why = saved.by_tag.unwrap();
        assert!(why.contains("names.mod"), "{why}");
        fs::write(&descriptor, descriptor_bytes).unwrap();
        let stack = &mut failed.stacks[0];
        let (checked, delay) = stack.retry.unwrap();
        stack.load(&content(), checked + delay);
        assert!(stack.localisation.as_ref().unwrap().is_ok());
        assert_eq!(stack.retry, None);
        assert!(failed.localisation(&tagged).is_ok());
        fs::remove_dir_all(directory).unwrap();
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

    /// An ended session listing `hours` of `campaign` at `load` waiting, unstaged.
    fn ended_waiting(
        directory: &Path,
        folder: &str,
        campaign: &str,
        branch: u64,
        hours: std::ops::RangeInclusive<i32>,
        load: i64,
    ) -> PathBuf {
        let session = directory.join("sessions").join(folder);
        let target = directory.join("target");
        replay_io::begin_admissions(&session, &target).unwrap();
        replay_io::Admissions::default()
            .admit(&session, &target, campaign, load)
            .unwrap();
        crate::test_support::write_load_segment(&session.join(campaign), 0, branch, hours, load);
        let unsaved = replay_io::unsaved(&session, campaign).unwrap();
        write_json(
            &session.join("completed.json"),
            &json!({"target": target, "waiting": [waiting_json(campaign, unsaved.as_ref())]}),
        )
        .unwrap();
        session
    }

    #[test]
    fn a_campaign_left_or_saved_with_no_admitted_hour_here_saves_what_ended_sessions_left() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, THIRD_LOAD, allocation_test_directory,
            recorded_frames, write_load_segment,
        };
        let directory = allocation_test_directory("settle-continued");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let recording = target.join("Campaign.battleplan");
        let first = ended_waiting(&directory, "1-1", name, 9, 2..=4, FIRST_LOAD);
        // This session loaded it and the game left it before an hour.
        let session = directory.join("sessions").join("2-1");
        replay_io::begin_admissions(&session, &target).unwrap();
        let settling = Settling::default();
        locked(&settling.left).insert(name.to_owned());
        let settle_now = || {
            let mut ahead = replay_io::Ahead::default();
            settle(
                &session,
                &target,
                &mut ahead,
                &settling,
                &mut Settled::new(),
                unnamed(),
            )
            .unwrap();
        };
        // Without Auto save it waits; loaded again, it is not saved.
        settle_now();
        settling.save.store(true, Ordering::Release);
        *locked(&settling.loaded) = Some(name.to_owned());
        settle_now();
        assert!(take_done(&settling).is_empty() && !recording.exists());
        // Left again, Auto save saves the hours it continues.
        *locked(&settling.loaded) = None;
        settle_now();
        assert_eq!(
            take_done(&settling),
            [SetAside::Saved(name.to_owned(), Some(recording.clone()))]
        );
        assert_eq!(recorded_frames(&recording), [(9, 2), (9, 3), (9, 4)]);
        assert!(waiting_campaigns(&first).is_empty() && locked(&settling.left).is_empty());
        // Hours left out here, with more waiting from an ended session: Save saves those.
        let second = ended_waiting(&directory, "1-2", name, 10, 5..=6, RELOAD);
        replay_io::Admissions::default()
            .refuse(&session, &target, name, THIRD_LOAD, Refusal::Waiting)
            .unwrap();
        write_load_segment(&session.join(name), 0, 11, 7..=8, THIRD_LOAD);
        settling.save.store(false, Ordering::Release);
        locked(&settling.requests).push((name.to_owned(), false));
        settle_now();
        assert_eq!(
            take_done(&settling),
            [SetAside::Saved(name.to_owned(), Some(recording.clone()))]
        );
        assert_eq!(
            recorded_frames(&recording),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
        );
        assert!(waiting_campaigns(&second).is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_sessions_end_saves_what_a_campaign_continues_without_an_hour_and_never_one_left_out() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, recorded_frames,
        };
        const LEFT_OUT: &str = "00000000-0000-4000-8000-000000000001";
        let directory = allocation_test_directory("save-continued");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let recording = target.join("Campaign.battleplan");
        let first = ended_waiting(&directory, "1-1", name, 9, 2..=4, FIRST_LOAD);
        // Another campaign's waiting hours cannot be read, so its loads here were left out.
        let broken = directory.join("sessions").join("1-2");
        fs::create_dir_all(broken.join(LEFT_OUT)).unwrap();
        fs::write(broken.join(LEFT_OUT).join("spool.0000.h4s"), b"junk").unwrap();
        write_json(
            &broken.join("completed.json"),
            &json!({"target": target, "waiting": [waiting_json(LEFT_OUT, None)]}),
        )
        .unwrap();
        // This session admitted a load of the first and spooled no hour of it; it spooled the other.
        let mut admitted = replay_io::Admissions::default();
        admitted
            .loads
            .entry(name.to_owned())
            .or_default()
            .insert(RELOAD);
        let spooled = [LEFT_OUT.to_owned()];
        let end = |saves: &dyn Fn(&str) -> bool| {
            save_continued(
                &directory,
                None,
                &target,
                &spooled,
                Some(&admitted),
                saves,
                unnamed(),
            )
        };
        // Not saved at this end, it keeps waiting.
        assert_eq!(end(&|_| false), (Vec::new(), BTreeMap::new()));
        assert!(!recording.exists());
        // Saved, its hours are; the campaign left out is neither saved nor failed.
        assert_eq!(end(&|_| true), (vec![recording.clone()], BTreeMap::new()));
        assert_eq!(recorded_frames(&recording), [(9, 2), (9, 3), (9, 4)]);
        assert!(waiting_campaigns(&first).is_empty());
        assert_eq!(waiting_campaigns(&broken).len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn ending_a_session_lists_it_before_saving_older_hours_without_consuming_its_final_callback() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, recorded_frames,
            write_load_segment,
        };
        let directory = allocation_test_directory("save-continued-current-checkpoints");
        let target = directory.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let earlier = ended_waiting(&directory, "1-1", name, 9, 2..=4, FIRST_LOAD);
        assert!(!target.exists());
        assert!(stage_waiting(&directory, &target).unwrap().is_empty());
        let session = directory.join("sessions/1-2");
        write_load_segment(&session.join(name), 0, 10, 5..=6, RELOAD);
        let unsaved = replay_io::unsaved(&session, name).unwrap();
        replay_io::write_session_end(
            &session,
            &target,
            WAITING_FILE,
            &json!({"target":target,"waiting":[waiting_json(name, unsaved.as_ref())]}),
        )
        .unwrap();
        let (_, blocked) = save_continued(
            &directory,
            Some(&session),
            &target,
            &[name.to_owned()],
            None,
            &|_| true,
            unnamed(),
        );
        assert!(blocked.is_empty());
        assert!(waiting_campaigns(&earlier).is_empty());
        assert_eq!(waiting_campaigns(&session).len(), 1);
        let recording = replay::recording_files::recording_of(&target, ALLOCATION_CAMPAIGN)
            .unwrap()
            .unwrap();
        assert_eq!(recorded_frames(&recording), [(9, 2), (9, 3), (9, 4)]);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&seen);
        let mut ahead = replay_io::Ahead::default();
        ahead.on_committed(Box::new(move |_, replay, blocks| {
            locked(&observed).extend(
                blocks
                    .iter()
                    .flat_map(|block| replay.read_block(block).unwrap())
                    .map(|frame| (frame.branch, frame.hour)),
            );
        }));
        let done = replay_io::finalize_session_ahead(
            &session,
            &target,
            &|| Ok(()),
            ahead,
            unnamed(),
            &|_| replay_io::Destination::Recording,
        )
        .unwrap();
        assert!(done.errors.is_empty());
        assert_eq!(*locked(&seen), [(10, 5), (10, 6)]);
        assert_eq!(
            recorded_frames(&done.replays[0]),
            [(9, 2), (9, 3), (9, 4), (10, 5), (10, 6)]
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn requests_the_finalizer_stopped_before_acting_on_are_answered() {
        let settling = Settling::default();
        locked(&settling.requests).push(("forwarded".to_owned(), false));
        let unstarted = FinalizingAhead {
            packing: None,
            stop: Arc::default(),
            thread: None,
            asked: Arc::default(),
            names: Arc::default(),
        };
        finish_ahead(unstarted, Some(&settling));
        assert!(locked(&settling.requests).is_empty());
        assert_eq!(
            take_done(&settling),
            [SetAside::Failed(
                "forwarded".to_owned(),
                "the session was ending; choose Save or Discard again".to_owned()
            )]
        );
    }

    #[test]
    fn finalization_progress_locked_reader_does_not_abort_publication() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        let root = allocation_test_directory("finalization-progress-reader");
        let session = root.join("session");
        let output = root.join("replays");
        write_campaign_spool(&session, &allocation_records());
        let observed = Cell::new(0);
        let observe = |records| observed.set(records);
        let progress = FinalizationProgress::new(&session, &observe);
        let path = session.join("finalization-progress.json");
        // Model a reader that permits reads/writes but denies rename-over.
        let reader = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        let before = fs::read(&path).unwrap();
        assert!(write_json(&path, &json!({"records": 1})).is_err());

        let done = replay_io::finalize_session_outcome_with_progress(&session, &output, &|| {
            // Exercise publication at every finalizer callback without sleeps.
            progress
                .last_write
                .set(Instant::now() - Duration::from_secs(2));
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

        progress
            .last_write
            .set(Instant::now() - Duration::from_secs(2));
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
        let progress = FinalizationProgress::new(&session, &observe);
        let previous_attempt = Instant::now() - Duration::from_secs(2);
        progress.last_write.set(previous_attempt);
        progress.advance();
        assert_eq!(observed.get(), 1);
        assert!(progress.last_write.get() > previous_attempt);
        fs::remove_file(&session).unwrap();
        fs::create_dir(&session).unwrap();
        progress
            .last_write
            .set(Instant::now() - Duration::from_secs(2));
        progress.advance();
        let resumed: Value =
            serde_json::from_slice(&fs::read(session.join("finalization-progress.json")).unwrap())
                .unwrap();
        assert_eq!(resumed["records"], 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_healthy_long_holds_comparison_keeps_the_watch_alive_without_record_callbacks() {
        let root = crate::test_support::allocation_test_directory("long-holds-progress");
        let mut records = crate::test_support::allocation_records();
        // Nothing may follow the Detach that ends the fixture's segment.
        records.insert(
            records.len() - 1,
            replay::spool::Record::Control(replay::spool::ControlRecord::Partial {
                session: 7,
                failed_hour: None,
                reason: "synthetic comparison bytes ".repeat(40_000),
            }),
        );
        let replay = Replay::from_records(
            crate::test_support::ALLOCATION_CAMPAIGN.to_vec(),
            &[records],
        )
        .unwrap();
        let earlier = root.join("earlier.battleplan");
        let staged = root.join("staged.battleplan");
        fs::write(&earlier, replay.bytes()).unwrap();
        fs::write(&staged, replay.bytes()).unwrap();
        let held = Replay::open_path(&staged).unwrap();
        let observe = |_| {};
        let records_seen = FinalizationProgress::new(&root, &observe);
        let start = Instant::now();
        let state = Arc::new(Mutex::new((0_u64, ProgressWatch::new(start))));
        let observed = Arc::clone(&state);
        let directory = root.clone();
        let signal = progress_io::PassProgress::new(&root);
        let progress = replay::progress::Progress::new(move || {
            let mut state = locked(&observed);
            state.0 += 60;
            let now = start + Duration::from_secs(state.0);
            signal.completed_at(now);
            assert!(!state.1.stalled(now, progress_stamp(&directory, None)));
        })
        .install();
        assert!(held.holds(&earlier).unwrap());
        drop(progress);
        assert_eq!(records_seen.records.get(), 0);
        let mut state = locked(&state);
        assert!(state.0 > 10 * 60);
        let stopped = start + Duration::from_secs(state.0 + 180);
        assert!(state.1.stalled(stopped, progress_stamp(&root, None)));
        drop(held);
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
        // CPU/I/O remains observable even if both advisory files become unreadable.
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
        // The detach command allows the controller's 180 s without collector
        // progress: 150 s passes (the old 120 s limit failed it), 180 s ends.
        let stamp = ProgressStamp {
            finalization: Some(UNIX_EPOCH + Duration::from_secs(1000)),
            ..ProgressStamp::default()
        };
        assert!(!watch.stalled(start + Duration::from_secs(1150), stamp));
        assert!(!watch.stalled(start + Duration::from_secs(1179), stamp));
        assert!(watch.stalled(start + Duration::from_secs(1180), stamp));
    }

    /// The 48da4248f1 detach: the collector took steps for 92 s before done.
    /// The controller's old fixed 60 s deadline gave up at 60 s with the
    /// collector still loaded; the progress wait follows the steps, still
    /// ends a collector that stops taking them, and caps the total.
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
        // One step of 120 s (a hook publication on a loaded machine, like
        // the 2d1ede901c 1936 burn-in's 43.9 s Opinion restore and worse)
        // continues; a collector that stops is ended 180 s after its last
        // step, not before. The old 60 s limit ended both.
        let mut wait = CollectorWait::new(start, DONE_STALL);
        assert_eq!(wait.ended(at(10), (Some(40), None)), None);
        assert_eq!(wait.ended(at(130), (Some(80), None)), None, "a 120 s step");
        assert_eq!(wait.ended(at(309), (Some(80), None)), None);
        assert_eq!(
            wait.ended(at(310), (Some(80), None)),
            Some(WaitEnd::Stalled(Duration::from_secs(180)))
        );
        let mut ready = CollectorWait::new(start, READY_STALL);
        // Readiness: the step seen at 10 s, unchanged at 160 s (the old
        // 120 s limit ended it) and ended at 190 s.
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
        // Status writes are progress too; the cap ends any wait.
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
        assert!(usage().contains("BATTLE_PLAN_WORKER_SAMPLER"));
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
            allocation_records, allocation_test_directory, allocation_witness, write_campaign_spool,
        };
        let root = allocation_test_directory("done-retained-module");
        let session = root.join("session");
        let output = root.join("recordings");
        let records = allocation_records();
        let end = records
            .iter()
            .position(
                |record| matches!(record, replay::spool::Record::Frame(frame) if frame.hour == 4),
            )
            .unwrap();
        let campaign = write_campaign_spool(&session, &records[..end]);
        let spool = campaign.join("spool.0000.h4s");
        let mut witness = allocation_witness(fs::metadata(&spool).unwrap().len());
        witness.failed_hour = Some(4);
        witness.phase = 6;
        let mut bytes = fs::read(&spool).unwrap();
        for record in &records[end..] {
            bytes.extend(replay::spool::encode_record(record).unwrap());
        }
        bytes.extend_from_slice(b"synthetic torn tail");
        fs::write(&spool, &bytes).unwrap();
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
        let replay = replay::container::Replay::open_path(&path).unwrap();
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
            replay::container::SeekResult::Gap { .. }
        ));
        assert_eq!(fs::read(&spool).unwrap(), bytes);
    }

    #[test]
    fn cli_finalize_preserves_incomplete_drain_without_an_allocation_witness() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let root = allocation_test_directory("cli-incomplete-drain");
        let session = root.join("session");
        let output = root.join("output");
        let records = allocation_records();
        let end = records
            .iter()
            .position(
                |record| matches!(record, replay::spool::Record::Frame(frame) if frame.hour == 4),
            )
            .unwrap();
        write_campaign_spool(&session, &records[..end]);
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
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        for loaded in [true, false] {
            let root = allocation_test_directory(&format!("save-ready-before-unload-{loaded}"));
            let directory = root.join("session");
            let target = root.join("output");
            let records = allocation_records();
            let replay::spool::Record::SessionStart(session) = &records[0] else {
                unreachable!()
            };
            write_campaign_spool(&directory, &records);
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
    }

    #[test]
    fn a_campaign_whose_unsaved_hours_cannot_be_read_is_kept_and_reported() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let directory = allocation_test_directory("others-unsaved-unreadable");
        let campaign = write_campaign_spool(&directory, &allocation_records());
        let name = campaign.file_name().unwrap().to_str().unwrap().to_owned();
        let spooled = vec![name.clone()];
        let mut issues = Vec::new();
        assert!(others_unsaved(&directory, &spooled, None, &mut issues));
        assert!(issues.is_empty());
        fs::write(directory.join(WITNESS_FILE_NAME), b"torn").unwrap();
        assert!(!others_unsaved(
            &directory,
            &spooled,
            Some(name.as_str()),
            &mut issues
        ));
        assert!(issues.is_empty());
        assert!(others_unsaved(&directory, &spooled, None, &mut issues));
        assert_eq!(issues.len(), 1);
        assert!(issues[0].contains(&name));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn discarded_partial_sessions_keep_terminal_fields_and_all_failure_evidence() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, allocation_witness, write_campaign_spool,
        };
        for failure in ["sync", "allocation", "clean"] {
            for exited in [false, true] {
                let root =
                    allocation_test_directory(&format!("discard-terminal-{failure}-{exited}"));
                let directory = root.join("session");
                let target = root.join("output");
                fs::create_dir_all(&target).unwrap();
                let records = allocation_records();
                let replay::spool::Record::SessionStart(session) = &records[0] else {
                    unreachable!()
                };
                let campaign = write_campaign_spool(&directory, &records);
                let spool = campaign.join("spool.0000.h4s");
                let bytes = fs::read(&spool).unwrap();
                create_failure_witness(&directory, 7, 13).unwrap();
                if failure == "allocation" {
                    fs::write(
                        directory.join(WITNESS_FILE_NAME),
                        allocation_witness(bytes.len() as u64).encode(),
                    )
                    .unwrap();
                }
                let witness = fs::read(directory.join(WITNESS_FILE_NAME)).unwrap();
                fs::write(directory.join("progress"), b"synthetic done").unwrap();
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
                let completed = discarded_completion(&terminal, true);
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
                let pending = RefCell::new(None);
                let result = finish_capture(
                    &directory,
                    &target,
                    &root.join("active.json"),
                    session,
                    &directory.join("bin/collector.dll"),
                    exited,
                    completed,
                    capture_result(
                        &directory,
                        &terminal,
                        CaptureOutcome {
                            discarded: true,
                            ..CaptureOutcome::default()
                        },
                    ),
                    &|stage| match stage {
                        Stage::Finished { result } => assert_eq!(result.is_err(), partial),
                        Stage::Unloading(observation) => {
                            pending.replace(Some(observation));
                        }
                        _ => unreachable!(),
                    },
                    || None,
                );
                assert_eq!(result.is_err(), partial);
                assert!(spool.exists());
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
                let completed = read_status(&directory.join("completed.json")).unwrap();
                assert_eq!(completed["discarded"], true);
                assert_eq!(completed["partial"], partial);
                assert_eq!(completed["issues"], json!(terminal.issues));
                if partial {
                    discard_session(&directory).unwrap();
                    assert_eq!(fs::read(&spool).unwrap(), bytes);
                    assert_eq!(
                        fs::read(directory.join(WITNESS_FILE_NAME)).unwrap(),
                        witness
                    );
                    assert_eq!(
                        fs::read(directory.join("progress")).unwrap(),
                        b"synthetic done"
                    );
                    assert!(directory.join("status.json").exists());
                    assert!(completed_detach(&completed).is_err());
                } else {
                    assert!(!spool.exists());
                    assert!(!directory.join(WITNESS_FILE_NAME).exists());
                }
            }
        }
    }

    #[test]
    fn fully_drained_pinned_collector_finalizes_with_warnings_and_blocks_attach() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let root = allocation_test_directory("restoration-only-warning");
        let session = root.join("session");
        let output = root.join("output");
        write_campaign_spool(&session, &allocation_records());
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
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        for empty in [false, true] {
            let root = allocation_test_directory(&format!("healthy-stop-unload-{empty}"));
            let session = root.join("session");
            let output = root.join("output");
            fs::create_dir_all(&session).unwrap();
            // A session without spools refuses finalization; only its unload wait is checked.
            let finalized = if empty {
                replay_io::Finalized::default()
            } else {
                write_campaign_spool(&session, &allocation_records());
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
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        let root = allocation_test_directory("completion-slow-unload");
        let session = root.join("session");
        let output = root.join("output");
        write_campaign_spool(&session, &allocation_records());
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
    fn unconfirmed_unload_warns_without_making_saved_data_partial_or_allowing_attach() {
        use crate::test_support::{
            allocation_records, allocation_test_directory, write_campaign_spool,
        };
        for loaded in [Some(true), None] {
            let root = allocation_test_directory(&format!("completion-unconfirmed-{loaded:?}"));
            let session = root.join("session");
            let output = root.join("output");
            write_campaign_spool(&session, &allocation_records());
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
        // A collector stand-in acknowledges the whole token it finds.
        let watcher = {
            let session = session.clone();
            std::thread::spawn(move || {
                // Only a hang guard: a busy machine can delay the request.
                let deadline = Instant::now() + Duration::from_secs(120);
                while Instant::now() < deadline {
                    if let Ok(token) = fs::read_to_string(session.join("refresh.request")) {
                        assert!(!token.trim().is_empty());
                        fs::write(session.join("refresh.ack"), token.trim()).unwrap();
                        fs::remove_file(session.join("refresh.request")).unwrap();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("no refresh request appeared");
            })
        };
        request_refresh(&root).unwrap();
        watcher.join().unwrap();
        // No temporary file is left behind.
        let leftovers: Vec<_> = fs::read_dir(&session)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
        // Without an active recorder the request is refused.
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
    fn control_requests_discard_only_when_asked_and_resets_between_captures() {
        let control = Control::new();
        assert!(!control.stopping() && !control.discarding());
        control.stop(false);
        assert!(control.stopping() && !control.discarding());
        control.reset();
        assert!(!control.stopping() && !control.discarding());
        control.stop(true);
        assert!(control.stopping() && control.discarding());
        control.reset();
        assert!(!control.stopping() && !control.discarding());
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
        // On disk the moment it is admitted.
        let first = BTreeMap::from([(KEPT.to_owned(), BTreeSet::from([1]))]);
        assert_eq!(on_disk().loads, first);
        assert_eq!(
            admission.check(REFUSED, 1, || false, &admit),
            Ok(Decision::Refused)
        );
        // A campaign whose waiting hours could not be staged is left out, and the session goes on.
        let unstaged = |_: &str, _: bool| -> Result<(), Refusal> { Err(Refusal::Waiting) };
        assert_eq!(
            admission.check(REFUSED, 7, || false, &unstaged),
            Ok(Decision::Omitted)
        );
        // Each load is asked once: a later change of choice does not end a
        // session that already admitted it, and a refusal is not repeated.
        assert_eq!(
            admission.check(REFUSED, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(
            admission.check(KEPT, 1, || unreachable!(), &admit),
            Ok(Decision::Seen)
        );
        assert_eq!(calls.get(), 2);
        // Another load of the admitted campaign is asked again; refused, it is
        // never admitted, so never published, and its reason is kept.
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
                ((REFUSED.to_owned(), 7), Refusal::Waiting),
                ((KEPT.to_owned(), 2), Refusal::Elsewhere),
            ])
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_opening_load_is_admitted_before_the_collector_starts() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        let root = allocation_test_directory("opening-load");
        let session = root.join("session");
        let target = root.join("target");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &target).unwrap();
        let mut admission = Admission::new(&session, &target);
        admission.admit_opening(campaign, FIRST_LOAD).unwrap();
        // On disk before any hour is spooled, and never asked about again.
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
        // the game closed before the first poll: the load still publishes.
        write_campaign_spool(&session, &allocation_records());
        admission.check_spooled(&ask).unwrap();
        let paths = replay_io::finalize_session(&session, &target).unwrap();
        assert_eq!(published_hours(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Attached at the game's menu with automatic recording on, a load
    /// records and the game closes (or Stop is pressed) before the first probe.
    #[test]
    fn a_brief_load_never_probed_is_admitted_at_save_and_published() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        let root = allocation_test_directory("brief-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        write_campaign_spool(&session, &allocation_records());
        let asked = RefCell::new(Vec::new());
        let admit = |campaign: &str, recorded: bool| -> Result<(), Refusal> {
            asked.borrow_mut().push((campaign.to_owned(), recorded));
            Ok(())
        };
        admission.check_spooled(&admit).unwrap();
        assert_eq!(asked.into_inner(), [(campaign.to_owned(), false)]);
        assert_eq!(
            replay_io::session_admissions(&session)
                .unwrap()
                .unwrap()
                .loads,
            BTreeMap::from([(campaign.to_owned(), BTreeSet::from([FIRST_LOAD]))])
        );
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
        assert!(finalized.errors.is_empty() && finalized.omitted.is_empty());
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    /// A is admitted, then reloads and records hours, and the game closes before
    /// the next probe. At save the reload is asked by the same rule: with
    /// automatic recording turned off for A it is left out, and after New
    /// file it belongs in a new file. Either way it is reported, with why.
    #[test]
    fn a_reload_that_ends_before_its_probe_is_asked_at_save_by_the_same_rule() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, two_load_records,
            write_campaign_spool,
        };
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        for refusal in [Refusal::Off, Refusal::Elsewhere] {
            let root = allocation_test_directory(&format!("reload-at-save-{}", refusal.as_str()));
            let session = root.join("session");
            let output = root.join("replays");
            replay_io::begin_admissions(&session, &output).unwrap();
            let mut admission = Admission::new(&session, &output);
            admission.admit_opening(campaign, FIRST_LOAD).unwrap();
            write_campaign_spool(&session, &two_load_records());
            let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
                if recorded { Err(refusal) } else { Ok(()) }
            };
            admission.check_spooled(&admit).unwrap();
            let finalized =
                replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                    .unwrap();
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

    /// Record pressed at the menu with automatic recording off. Campaign A
    /// loads and records first, then campaign B, and the game closes before any
    /// probe; B's UUID sorts before A's. Save-time decisions follow load
    /// order, so Record belongs to A.
    #[test]
    fn save_time_decisions_follow_load_order_across_campaigns() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_records, allocation_test_directory,
            write_campaign_spool,
        };
        use replay::{spool::ControlRecord, spool::Record};
        const LATER: &str = "00000000-0000-4000-8000-000000000001";
        let first = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        assert!(LATER < first, "the later campaign's UUID sorts first");
        let root = allocation_test_directory("load-order");
        let session = root.join("session");
        let output = root.join("replays");
        write_campaign_spool(&session, &allocation_records());
        // B: its own branch and load, witnessed after all of A's hours.
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
        let spool: Vec<u8> = later
            .iter()
            .flat_map(|record| replay::spool::encode_record(record).unwrap())
            .collect();
        fs::create_dir_all(session.join(LATER)).unwrap();
        fs::write(session.join(LATER).join("spool.0000.h4s"), spool).unwrap();
        assert_eq!(
            replay_io::first_spooled_campaign(&session).as_deref(),
            Some(first)
        );
        // Record pressed: the first campaign asked about records; others are off.
        let chosen: RefCell<Option<String>> = RefCell::new(None);
        let admit = |campaign: &str, _: bool| -> Result<(), Refusal> {
            let mut chosen = chosen.borrow_mut();
            let pressed = chosen.get_or_insert_with(|| campaign.to_owned()).as_str() == campaign;
            if pressed { Ok(()) } else { Err(Refusal::Off) }
        };
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        admission.check_spooled(&admit).unwrap();
        assert_eq!(chosen.borrow().as_deref(), Some(first));
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
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

    /// New file is pending and the campaign's first recording is still only
    /// this session's spool. The reload, unpaused before the check, is refused;
    /// then detach times out (or the recorder crashes), so only a later
    /// `finalize` publishes.
    #[test]
    fn a_refused_reload_stays_unpublished_when_the_session_never_completes() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_records, allocation_test_directory,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("refused-reload-unfinished");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // New file starts a new recording for a load that follows recorded hours.
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session.join(campaign), load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        write_campaign_spool(&session, &allocation_records());
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        write_campaign_spool(&session, &two_load_records());
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!session.join("completed.json").exists());
        let paths = replay_io::finalize_session(&session, &output).unwrap();
        assert_eq!(published_hours(&paths[0]), [(9, 2), (9, 3), (9, 4)]);
        fs::remove_dir_all(root).unwrap();
    }

    /// Attached to a paused campaign, the probe admits its load before the
    /// collector names the campaign, and New file is chosen once recording
    /// starts. Polls name only loads, so nothing uses the choice up until the
    /// reload, which starts the new recording; the first load publishes alone.
    #[test]
    fn new_file_chosen_after_the_first_load_waits_for_the_reload() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_records, allocation_test_directory,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("new-file-after-first-load");
        let session = root.join("session");
        let output = root.join("replays");
        let campaign = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        // New file as the tray applies it: used up by the next load asked
        // about, which starts a new recording when it follows recorded hours.
        let new_file = Cell::new(false);
        let admit = |_: &str, recorded: bool| -> Result<(), Refusal> {
            if new_file.replace(false) && recorded {
                Err(Refusal::Elsewhere)
            } else {
                Ok(())
            }
        };
        let recorded = |load| replay_io::recorded_before_load(&session.join(campaign), load);
        replay_io::begin_admissions(&session, &output).unwrap();
        let mut admission = Admission::new(&session, &output);
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Admitted)
        );
        write_campaign_spool(&session, &allocation_records());
        new_file.set(true);
        // Later polls name the same load: nothing is asked, so New file waits.
        assert_eq!(
            admission.check(campaign, FIRST_LOAD, || recorded(FIRST_LOAD), &admit),
            Ok(Decision::Seen)
        );
        assert!(new_file.get());
        write_campaign_spool(&session, &two_load_records());
        assert_eq!(
            admission.check(campaign, RELOAD, || recorded(RELOAD), &admit),
            Ok(Decision::Refused)
        );
        assert!(!new_file.get());
        let finalized =
            replay_io::finalize_session_outcome_with_progress(&session, &output, &|| Ok(()))
                .unwrap();
        assert!(finalized.errors.is_empty(), "{:?}", finalized.errors);
        assert_eq!(
            published_hours(&finalized.replays[0]),
            [(9, 2), (9, 3), (9, 4)]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cli_finalize_durably_completes_and_unlists_saved_campaigns_before_cleanup() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, write_load_segment,
        };
        for waiting in [false, true] {
            let root = allocation_test_directory(&format!("cli-finalize-cleanup-{waiting}"));
            let session = root.join("sessions/1-1");
            let target = root.join("target");
            let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
            let campaign = session.join(name);
            write_load_segment(&campaign, 0, 9, 2..=4, 19);
            if waiting {
                replay_io::save_campaign(
                    &session,
                    &target,
                    name,
                    true,
                    &|| Ok(()),
                    &mut replay_io::Ahead::default(),
                    unnamed(),
                )
                .unwrap();
                replay_io::write_session_end(
                    &session,
                    &target,
                    WAITING_FILE,
                    &json!({"target": target, "waiting": [{"campaign": name}]}),
                )
                .unwrap();
            }
            finalize_cli_session(
                &session,
                &target,
                terminal_outcome(&session, 7, 13, false, true, true),
                CollectorPresence::exited,
            )
            .unwrap();
            assert!(waiting_campaigns(&session).is_empty());
            assert!(replay_io::segment_paths(&campaign).unwrap().is_empty());
            let completed = read_status(&session.join("completed.json")).unwrap();
            assert_eq!(completed["process_exited"], true);
            assert_eq!(completed["waiting"], json!([]));
            assert_eq!(completed["replays"].as_array().unwrap().len(), 1);
            finalize_cli_session(
                &session,
                &target,
                terminal_outcome(&session, 7, 13, false, true, true),
                CollectorPresence::exited,
            )
            .unwrap();
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn cli_finalize_keeps_failed_campaigns_waiting_without_blocking_successful_cleanup() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, allocation_test_directory, write_load_segment,
        };
        let root = allocation_test_directory("cli-finalize-partial-cleanup");
        let session = root.join("sessions/1-1");
        let target = root.join("target");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = session.join(name);
        write_load_segment(&campaign, 0, 9, 2..=4, 19);
        let refused = "00000000-0000-4000-8000-000000000000";
        let spool = write_load_segment(&session.join(refused), 0, 10, 5..=6, 21);
        let before = fs::read(&spool).unwrap();
        assert!(
            finalize_cli_session(
                &session,
                &target,
                terminal_outcome(&session, 7, 13, false, true, true),
                CollectorPresence::exited
            )
            .is_err()
        );
        assert!(replay_io::segment_paths(&campaign).unwrap().is_empty());
        assert_eq!(fs::read(&spool).unwrap(), before);
        let waiting = waiting_campaigns(&session);
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].campaign, refused);
        assert_eq!(
            read_status(&session.join("completed.json")).unwrap()["partial"],
            true
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_finalization_of_a_failed_session_publishes_exactly_its_admitted_loads() {
        use crate::test_support::{
            ALLOCATION_CAMPAIGN, FIRST_LOAD, RELOAD, allocation_test_directory, allocation_witness,
            two_load_records, write_campaign_spool,
        };
        let root = allocation_test_directory("failed-session-admissions");
        let session = root.join("session");
        let output = root.join("replays");
        let name = std::str::from_utf8(ALLOCATION_CAMPAIGN).unwrap();
        let campaign = write_campaign_spool(&session, &two_load_records());
        let committed = fs::metadata(campaign.join("spool.0000.h4s")).unwrap().len();
        // The collector failed during the reload, which was never admitted.
        let mut witness = allocation_witness(committed);
        witness.branch = Some(10);
        witness.load_signal = Some(RELOAD);
        fs::write(session.join(WITNESS_FILE_NAME), witness.encode()).unwrap();
        replay_io::begin_admissions(&session, &output).unwrap();
        replay_io::Admissions::default()
            .admit(&session, &output, name, FIRST_LOAD)
            .unwrap();
        // Each `finalize` rewrites the completion record; the admitted load,
        // and only it, publishes every time, unchanged, and the record keeps
        // the load it left out. Its hours have no country or date to name it by.
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
            // Two minutes of the capture loop's 100 ms polls.
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
        fs::write(campaign.join("spool.0001.h4s"), b"synthetic").unwrap();
        fs::write(session.join(WITNESS_FILE_NAME), b"synthetic").unwrap();
        write_json(&session.join("status.json"), &json!({"session":7})).unwrap();
        // An earlier recording of the campaign sits beside the sessions folder.
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
