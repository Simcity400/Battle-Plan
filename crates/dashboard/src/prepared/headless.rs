//! The dashboard's headless role: prepares the recordings the recorder saved, while no the game runs.

use crate::{
    Config, Service,
    background::{self, Activity, Request, Status},
    prepared::{self, PauseReason, State, owner},
};
use std::{
    collections::BTreeSet,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

pub use crate::background::ROLE;

const USAGE: &str = "--prepare [--recording R]...";

const TICK: Duration = Duration::from_secs(1);

/// How long recordings waiting for disk space wait before free space is measured again.
const SPACE_RETRY: Duration = Duration::from_secs(60);

/// An update asked this preparer to close: its job stops at the next block boundary, as when the game
/// starts, and the queue waits for the preparer the installer starts again.
static CLOSING: AtomicBool = AtomicBool::new(false);

/// Close for an update; the queue is kept.
pub fn close() {
    CLOSING.store(true, Ordering::Release);
}

fn closing() -> bool {
    CLOSING.load(Ordering::Acquire)
}

/// Wait `duration`, a tick at a time; whether the preparer is closing.
fn rest(duration: Duration) -> bool {
    let mut left = duration;
    while !closing() && !left.is_zero() {
        let step = left.min(TICK);
        std::thread::sleep(step);
        left -= step;
    }
    closing()
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

fn recordings(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<Vec<PathBuf>> {
    let mut recordings = Vec::new();
    while let Some(flag) = args.next() {
        match (flag.to_str(), args.next()) {
            (Some("--recording"), Some(path)) => recordings.push(path.into()),
            _ => return Err(invalid(USAGE)),
        }
    }
    Ok(recordings)
}

/// How one recording's turn ended.
enum Turn {
    Answered,
    /// Waiting for disk space.
    Kept,
    Game,
    Off,
    /// A later save superseded the revision it prepared: it is taken up again.
    Again,
    /// Closing for an update.
    Closing,
}

/// Works the queue with `args`, those after [`ROLE`], asked for too.
pub fn run(args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<()> {
    for recording in recordings(args)? {
        background::request(&recording)?;
    }
    owner::yield_to_others();
    loop {
        let Some(runner) = background::runner()? else {
            return Ok(());
        };
        let mut status = Status::read()?;
        status.running = true;
        status.pid = std::process::id();
        status.started = recorder_platform::windows::Process::open(status.pid, false)
            .map_err(io::Error::other)?
            .start_time;
        status.error = None;
        status.write()?;
        let worked = work(&mut status);
        // Its stderr reaches no one when the recorder starts it: the recorder shows this.
        status.running = false;
        status.error = worked.as_ref().err().map(ToString::to_string);
        let written = status.write();
        drop(runner);
        worked?;
        written?;
        // An ask made as the queue emptied found the runner held.
        if closing() || background::requests()?.is_empty() {
            return Ok(());
        }
    }
}

/// Works the queue until it is empty or the setting is off.
fn work(status: &mut Status) -> io::Result<()> {
    let mut kept: BTreeSet<PathBuf> = BTreeSet::new();
    loop {
        if closing() {
            return Ok(());
        }
        let waiting = background::requests()?;
        withdrawn(status, &waiting)?;
        if waiting.is_empty() {
            return Ok(());
        }
        if !Config::load()?.background_preparation_enabled() {
            return turned_off(status);
        }
        let Some(mut request) = next(waiting, &kept) else {
            if rest(SPACE_RETRY) {
                return Ok(());
            }
            kept.clear();
            continue;
        };
        if game_running()? {
            status.set(&request.recording, Activity::WaitingForGame, None, None);
            status.write()?;
            while game_running()? {
                if !Config::load()?.background_preparation_enabled() {
                    return turned_off(status);
                }
                withdrawn(status, &background::requests()?)?;
                if rest(Duration::from_secs(5)) {
                    return Ok(());
                }
            }
            continue;
        }
        let config = Config::load()?;
        let service = Service::new(Arc::new(|_| true))?;
        service.apply_saved_preparation()?;
        // The app's binding of content to stores, so the store is the one the app reads.
        service.configure_content(&config, false)?;
        let watch = GameWatch::start(Arc::clone(&service));
        let ended = loop {
            match prepare(&service, &watch, &request, status)? {
                Turn::Answered => background::forget(&request)?,
                Turn::Again => {}
                Turn::Kept => {
                    // Else admitting the next recording would resume this newer one first.
                    service.leave_library(&request.recording);
                    kept.insert(request.recording.clone());
                }
                ended => break ended,
            }
            match next(background::requests()?, &kept) {
                Some(following) => request = following,
                None => break Turn::Answered,
            }
        };
        drop(watch);
        // A job stops at its next block boundary, keeping what it published.
        service.shutdown();
        drop(service);
        match ended {
            Turn::Off => return turned_off(status),
            Turn::Closing => return Ok(()),
            _ => {}
        }
    }
}

/// What became of a recording given up to the app: Clear takes its asks back before the lock.
fn given_up(path: &Path) -> io::Result<Activity> {
    Ok(if background::pending(path)? {
        Activity::InApp
    } else {
        Activity::Cleared
    })
}

/// An unfinished recording no longer asked for was cleared in the app.
fn withdrawn(status: &mut Status, waiting: &[Request]) -> io::Result<()> {
    let left: Vec<PathBuf> = status
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.activity,
                Activity::Preparing | Activity::WaitingForGame | Activity::WaitingForSpace
            ) && !waiting
                .iter()
                .any(|request| request.recording == entry.recording)
        })
        .map(|entry| entry.recording.clone())
        .collect();
    if left.is_empty() {
        return Ok(());
    }
    for recording in left {
        status.set(&recording, Activity::Cleared, None, None);
    }
    status.write()
}

fn next(waiting: Vec<Request>, kept: &BTreeSet<PathBuf>) -> Option<Request> {
    waiting
        .into_iter()
        .find(|request| !kept.contains(&request.recording))
}

/// Nothing asked for is prepared, and each says so.
fn turned_off(status: &mut Status) -> io::Result<()> {
    let waiting = background::requests()?;
    withdrawn(status, &waiting)?;
    for request in &waiting {
        status.set(&request.recording, Activity::Off, None, None);
    }
    // Written before the asks go, as every outcome is: the recorder reads the queue first.
    status.write()?;
    waiting.iter().try_for_each(background::forget)
}

fn game_running() -> io::Result<bool> {
    recorder_platform::windows::games()
        .map(|games| !games.is_empty())
        .map_err(|error| io::Error::other(format!("the game's processes could not be listed: {error}")))
}

/// Shuts the service down the moment the game starts, or an update asks the preparer to close,
/// whatever it is doing then.
struct GameWatch {
    game: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl GameWatch {
    fn start(service: Arc<Service>) -> Self {
        let game = Arc::new(AtomicBool::new(false));
        let error = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (game, error, stop) = (Arc::clone(&game), Arc::clone(&error), Arc::clone(&stop));
            std::thread::spawn(move || {
                while !stop.load(Ordering::Acquire) {
                    if !closing() {
                        match game_running() {
                            Ok(false) => {
                                std::thread::sleep(TICK);
                                continue;
                            }
                            Ok(true) => game.store(true, Ordering::Release),
                            Err(failure) => {
                                *error.lock().unwrap_or_else(|error| error.into_inner()) =
                                    Some(failure.to_string())
                            }
                        }
                    }
                    service.shutdown();
                    return;
                }
            })
        };
        Self {
            game,
            error,
            stop,
            thread: Some(thread),
        }
    }

    /// Whether a game started; an unreadable process list is an error.
    fn game(&self) -> io::Result<bool> {
        if let Some(error) = self
            .error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            return Err(io::Error::other(error));
        }
        Ok(self.game.load(Ordering::Acquire))
    }
}

impl Drop for GameWatch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Prepares `request`'s recording through the service's library admission.
fn prepare(
    service: &Service,
    watch: &GameWatch,
    request: &Request,
    status: &mut Status,
) -> io::Result<Turn> {
    let path = &request.recording;
    let mut show = |activity, progress, error: Option<String>| {
        status.set(path, activity, progress, error);
        status.write()
    };
    show(Activity::Preparing, Some(0.0), None)?;
    service.upgrade_recording(path);
    // Held so its job's outcome stays readable once the service lets the job go.
    let source = match service.open_map(path, None) {
        Ok(source) => source,
        Err(_) if closing() => return Ok(Turn::Closing),
        Err(_) if watch.game()? => {
            show(Activity::WaitingForGame, None, None)?;
            return Ok(Turn::Game);
        }
        Err(error) => {
            show(Activity::Failed, None, Some(error))?;
            return Ok(Turn::Answered);
        }
    };
    let folder = match dashboard_base::prepared::recording_folder(&source) {
        Ok(folder) => folder,
        Err(error) => {
            show(Activity::Failed, None, Some(error.to_string()))?;
            return Ok(Turn::Answered);
        }
    };
    let yielded = owner::yielded(&folder);
    // The app's from then on: its job is stopped, and nothing here admits it again.
    let retire = || -> io::Result<()> {
        service.leave_library(path);
        prepared::cancel(&source);
        prepared::wait_finished(&source, &AtomicBool::new(false))
    };
    // The service cancels a revision a later save replaced, as the app's library scan does.
    let superseded =
        || crate::protocol::replay_revision(path).is_ok_and(|now| now != source.key().revision);
    let mut stalled = 0;
    loop {
        if closing() {
            return Ok(Turn::Closing);
        }
        if watch.game()? {
            show(Activity::WaitingForGame, None, None)?;
            return Ok(Turn::Game);
        }
        service.apply_saved_preparation()?;
        if !service.preparation_preferences().background_enabled {
            return Ok(Turn::Off);
        }
        // Given up to the app, or cleared there.
        if owner::yielded(&folder) > yielded || !background::pending(path)? {
            retire()?;
            show(given_up(path)?, None, None)?;
            return Ok(Turn::Answered);
        }
        let reply = prepared::status(&source, "", "");
        match reply.state {
            State::Ready => {
                show(Activity::Ready, Some(1.0), unprepared(&reply))?;
                return Ok(Turn::Answered);
            }
            State::Failed => {
                let error = reply.error.unwrap_or_else(|| "Preparation failed".into());
                if owner::held_elsewhere(&error) {
                    retire()?;
                    show(given_up(path)?, None, None)?;
                    return Ok(Turn::Answered);
                }
                if superseded() {
                    return Ok(Turn::Again);
                }
                show(Activity::Failed, None, Some(error))?;
                return Ok(Turn::Answered);
            }
            State::Cancelled if owner::yielded(&folder) > yielded => {
                retire()?;
                show(given_up(path)?, None, None)?;
                return Ok(Turn::Answered);
            }
            State::Cancelled if superseded() => return Ok(Turn::Again),
            State::Cancelled => {
                service.leave_library(path);
                let error = reply
                    .error
                    .unwrap_or_else(|| "Preparation was cancelled".into());
                show(Activity::Failed, None, Some(error))?;
                return Ok(Turn::Answered);
            }
            State::Queued | State::Preparing => {
                stalled = 0;
                show(Activity::Preparing, progress(&reply), None)?;
            }
            State::Idle | State::Paused => {
                // Twice, so a job that stopped just after an admission is admitted again.
                if stalled == 2 {
                    if room(path, &reply)? {
                        let error = "Preparation could not start".to_owned();
                        show(Activity::Failed, None, Some(error))?;
                        return Ok(Turn::Answered);
                    }
                    show(Activity::WaitingForSpace, None, None)?;
                    return Ok(Turn::Kept);
                }
                stalled += 1;
            }
        }
        // Every tick, as the app's library monitor does: it also pauses running work below
        // the disk margin.
        match service.prepare_library(std::slice::from_ref(path)) {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) if closing() => return Ok(Turn::Closing),
            Err(_) if watch.game()? => {
                show(Activity::WaitingForGame, None, None)?;
                return Ok(Turn::Game);
            }
            result => result?,
        }
        std::thread::sleep(TICK);
    }
}

/// The parts of a ready store that could not be prepared, and why the first could not.
fn unprepared(reply: &prepared::Reply) -> Option<String> {
    let failed: Vec<_> = reply
        .sections
        .iter()
        .filter(|section| section.state == State::Failed || section.error.is_some())
        .collect();
    let first = failed.first()?;
    let labels: Vec<&str> = failed
        .iter()
        .map(|section| section.label.as_str())
        .collect();
    Some(format!(
        "{} couldn't be prepared: {}",
        labels.join(", "),
        first.error.as_deref().unwrap_or("no reason was given")
    ))
}

/// Whether the volume has room for the store, as library admission judges it.
fn room(path: &Path, reply: &prepared::Reply) -> io::Result<bool> {
    let free = prepared::space::free();
    let store = prepared::recording_store(path)?;
    let needed = prepared::space::needed_for(path, store.folder())?;
    Ok(reply.pause_reason != Some(PauseReason::DiskSpace)
        && prepared::space::keeps(free)
        && prepared::space::admits(free, needed))
}

/// The least share of the recording any status row has prepared.
fn progress(reply: &prepared::Reply) -> Option<f64> {
    reply
        .sections
        .iter()
        .filter_map(|section| {
            let done: f64 = section.done.parse().ok()?;
            let total: f64 = section.total.as_ref()?.parse().ok()?;
            (total > 0.0).then(|| (done / total).min(1.0))
        })
        .reduce(f64::min)
}
