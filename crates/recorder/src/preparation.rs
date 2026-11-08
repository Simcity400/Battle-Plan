//! Starts the dashboard's headless preparation for each saved recording, and says how it goes.

use dashboard_core::{
    background::{self, Activity, Entry, Status},
    config::Config,
};
use recorder_platform::windows::Process;
use std::{
    io,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Mutex,
};
use windows_sys::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS,
};

#[derive(Clone, Default)]
struct Asked {
    /// Each recording asked for, and when, in milliseconds since the Unix epoch.
    recordings: Vec<(PathBuf, u64)>,
    /// The preparer started last, and its creation time.
    started: Option<(u32, u64)>,
    /// It was started inside the recorder's job, which forbids leaving it.
    contained: bool,
    error: Option<String>,
}

static ASKED: Mutex<Option<Asked>> = Mutex::new(None);

/// Asks the preparer for `recordings`, unless the setting is off; [`summary`] says how it went.
pub fn ask(recordings: &[PathBuf]) {
    if recordings.is_empty() {
        return;
    }
    let at = background::now_ms();
    let launched = launch(recordings);
    let mut held = ASKED.lock().unwrap_or_else(|error| error.into_inner());
    let asked = held.get_or_insert_with(Asked::default);
    match launched {
        Ok(None) => {}
        Ok(Some(launch)) => {
            for path in launch.recordings {
                asked.recordings.retain(|(held, _)| *held != path);
                asked.recordings.push((path, at));
            }
            asked.started = launch.started;
            asked.contained = launch.contained;
            asked.error = None;
        }
        Err(error) => asked.error = Some(error),
    }
}

struct Launch {
    /// None once it has already exited.
    started: Option<(u32, u64)>,
    /// Their canonical paths.
    recordings: Vec<PathBuf>,
    contained: bool,
}

/// None when the setting is off.
fn launch(recordings: &[PathBuf]) -> Result<Option<Launch>, String> {
    let config =
        Config::load().map_err(|error| format!("the app's settings could not be read: {error}"))?;
    if !config.background_preparation_enabled() {
        return Ok(None);
    }
    let dashboard = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .with_file_name(background::DASHBOARD);
    if !dashboard.is_file() {
        return Err(format!(
            "Battle Plan was not found at {}",
            dashboard.display()
        ));
    }
    let asked = recordings
        .iter()
        .map(|recording| background::request(recording).map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let spawn = |flags| {
        Command::new(&dashboard)
            .arg(background::ROLE)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(flags)
            .spawn()
    };
    let flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    // A job that forbids leaving it refuses breakaway: the preparer then runs inside it.
    let (child, contained) = match spawn(flags | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(child) => (child, false),
        Err(_) => (spawn(flags).map_err(|error| error.to_string())?, true),
    };
    let started = Process::open(child.id(), false)
        .ok()
        .map(|process| (process.pid, process.start_time));
    Ok(Some(Launch {
        started,
        recordings: asked,
        contained,
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub text: String,
    pub failed: bool,
}

/// What became of the recordings this recorder asked for, failures first; None before it asked.
pub fn summary() -> Option<Summary> {
    let asked = ASKED
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()?;
    let failed = |text: String| {
        Some(Summary {
            text: format!("Couldn't prepare for the dashboard: {text}"),
            failed: true,
        })
    };
    if let Some(error) = asked.error {
        return failed(error);
    }
    // Read before the status: the preparer writes an outcome before it takes the ask back.
    let mut queued = Vec::with_capacity(asked.recordings.len());
    for (path, _) in &asked.recordings {
        match background::pending(path) {
            Ok(pending) => queued.push(pending),
            Err(error) => return failed(format!("its queue can't be read: {error}")),
        }
    }
    let status = match Status::read() {
        Ok(status) => status,
        Err(error) => return failed(format!("its progress can't be read: {error}")),
    };
    let working =
        (status.running && alive((status.pid, status.started))) || asked.started.is_some_and(alive);
    // One the user deleted or moved since leaves the preparer's queue, and is not shown.
    let asked_for: Vec<(&PathBuf, u64, bool)> = asked
        .recordings
        .iter()
        .zip(queued)
        .filter(|((path, _), _)| {
            !std::fs::metadata(path).is_err_and(|error| error.kind() == io::ErrorKind::NotFound)
        })
        .map(|((path, at), queued)| (path, *at, queued))
        .collect();
    let first_asked = asked_for.iter().map(|(_, at, _)| *at).min()?;
    if let Some(error) = status.error.as_ref().filter(|_| !working)
        && status.at >= first_asked
    {
        return failed(format!("the preparer stopped: {error}"));
    }
    // Each recording, newest first, with what the preparer wrote of it since it was asked for,
    // and whether it is still queued: one Clear took back before the preparer reached it is not.
    let outcomes: Vec<(&Path, Option<&Entry>, bool)> = asked_for
        .iter()
        .rev()
        .map(|&(path, at, queued)| {
            let entry = status
                .entries
                .iter()
                .find(|entry| entry.recording == *path && entry.at >= at);
            (path.as_path(), entry, queued)
        })
        .collect();
    // A ready store with parts it could not prepare is a failure too.
    if let Some((path, entry)) = outcomes.iter().find_map(|&(path, entry, _)| {
        entry
            .filter(|entry| {
                entry.activity == Activity::Failed
                    || (entry.activity == Activity::Ready && entry.error.is_some())
            })
            .map(|entry| (path, entry))
    }) {
        let reason = entry.error.as_deref().unwrap_or("no reason was given");
        return Some(Summary {
            text: match entry.activity {
                Activity::Ready => {
                    format!("{} is ready in the dashboard, but {reason}", name(path))
                }
                _ => format!(
                    "Couldn't prepare for the dashboard: {}: {reason}",
                    name(path)
                ),
            },
            failed: true,
        });
    }
    let &(path, entry, queued) = outcomes
        .iter()
        .find(|(_, entry, queued)| *queued && entry.is_none_or(|entry| unfinished(entry.activity)))
        .or(outcomes.first())?;
    let name = name(path);
    let Some(entry) = entry else {
        return if !queued {
            note(format!("{name} was cleared in Battle Plan"), false)
        } else if working {
            note(
                format!("{name} waits to be prepared for the dashboard"),
                asked.contained,
            )
        } else {
            failed(format!("the preparer stopped before preparing {name}"))
        };
    };
    let unfinished = unfinished(entry.activity);
    // Clear took its asks back while it waited: what the entry says is past.
    if unfinished && !queued {
        return note(format!("{name} was cleared in Battle Plan"), false);
    }
    if unfinished && !working {
        return failed(format!("the preparer of {name} stopped before it finished"));
    }
    let text = match (entry.activity, &entry.error) {
        (Activity::Preparing, _) => match entry.progress {
            Some(done) => format!("Preparing {name} for the dashboard: {:.0}%", done * 100.0),
            None => format!("Preparing {name} for the dashboard"),
        },
        (Activity::WaitingForGame, _) => {
            format!("{name} is prepared for the dashboard once the game closes")
        }
        (Activity::WaitingForSpace, _) => {
            format!("{name} waits for free disk space to be prepared for the dashboard")
        }
        (Activity::Ready, None) => format!("{name} is ready in the dashboard"),
        (Activity::Ready, Some(missing)) => {
            return Some(Summary {
                text: format!("{name} is ready in the dashboard, but {missing}"),
                failed: true,
            });
        }
        (Activity::InApp, _) => format!("Battle Plan took over preparing {name}"),
        (Activity::Cleared, _) => format!("{name} was cleared in Battle Plan"),
        (Activity::Off, _) => {
            format!("{name} isn't prepared: preparing in the background is off in Battle Plan")
        }
        (Activity::Failed, error) => {
            let reason = error.as_deref().unwrap_or("no reason was given");
            return failed(format!("{name}: {reason}"));
        }
    };
    note(text, asked.contained && unfinished)
}

/// The preparer stays up while any of its recordings is at one of these.
fn unfinished(activity: Activity) -> bool {
    matches!(
        activity,
        Activity::Preparing | Activity::WaitingForGame | Activity::WaitingForSpace
    )
}

/// `text`, saying when the preparer stops with the recorder.
fn note(text: String, contained: bool) -> Option<Summary> {
    Some(Summary {
        text: if contained {
            format!("{text}; it stops if the recorder closes")
        } else {
            text
        },
        failed: false,
    })
}

/// Whether the process created at `started` as `pid` still runs.
fn alive((pid, started): (u32, u64)) -> bool {
    Process::open(pid, false).is_ok_and(|process| {
        process.start_time == started && matches!(process.handle.signaled(), Ok(false))
    })
}

fn name(path: &Path) -> String {
    path.file_stem().map_or_else(
        || path.display().to_string(),
        |stem| stem.to_string_lossy().into_owned(),
    )
}
