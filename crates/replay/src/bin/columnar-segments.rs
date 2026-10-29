//! Walks a `p3-convert` columnar recording one segment per worker: each segment from its
//! restart (or the recording's start) through the hour before the next restart, every
//! segment on its own walker state. Reports wall time, the walkers' CPU per recorded hour and
//! each segment's. `--check` also requires an undamaged recording and every segment to end in
//! exactly the state of the restart holding its last hour, its run's last restart included.
//! `columnar-segments <file.h4col> [--threads <n>] [--check]`

use replay::columnar_frames::{Recording, Segment, schema};
use replay_columnar::walk::State;
use serde_json::json;
use std::{
    io,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// This thread's CPU time in nanoseconds, at the scheduler's resolution.
#[cfg(windows)]
fn thread_ns() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the current thread and four owned FILETIMEs.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

#[cfg(not(windows))]
fn thread_ns() -> u64 {
    0
}

/// One walked segment: its hours, its walker's CPU and, when checked, whether it ended in
/// the next restart's state.
struct Walked {
    segment: Segment,
    hours: u32,
    cpu_ns: u64,
    ends_at_restart: Option<bool>,
}

/// Whether `state` holds what the restart on page `at` of `run` restores: every table,
/// status, sample hour and truncated view, and the dictionary.
fn same_as_restart(file: &Recording, run: u32, at: usize, state: &State) -> io::Result<bool> {
    let mut restored = State::new(schema()?)?;
    file.restore(run, at, &mut restored)?;
    let tables = state.columns().len();
    let dictionary = (0..state.dictionary().len() as u32)
        .all(|id| state.dictionary().get(id) == restored.dictionary().get(id));
    Ok(dictionary
        && state.dictionary().len() == restored.dictionary().len()
        && (0..tables).all(|table| {
            state.table(table) == restored.table(table)
                && state.status(table) == restored.status(table)
                && state.sampled_at(table) == restored.sampled_at(table)
                && state.truncated_view(table) == restored.truncated_view(table)
        }))
}

fn walk(
    file: &Recording,
    segment: Segment,
    next: Option<usize>,
    check: bool,
) -> io::Result<Walked> {
    let begin = thread_ns();
    let mut state = State::new(schema()?)?;
    let mut hours = 0;
    file.walk_segment(&segment, &mut state, |_, _, _| {
        hours += 1;
        Ok(true)
    })?;
    let cpu_ns = thread_ns() - begin;
    if hours != segment.last - segment.first + 1 {
        return Err(invalid(format!(
            "segment from hour ordinal {} walked {hours} hours",
            segment.first
        )));
    }
    let ends_at_restart = match (check, next) {
        (true, Some(at)) => Some(same_as_restart(file, segment.run, at, &state)?),
        _ => None,
    };
    Ok(Walked {
        segment,
        hours,
        cpu_ns,
        ends_at_restart,
    })
}

fn main() -> io::Result<()> {
    let usage = || invalid("usage: columnar-segments <file.h4col> [--threads <n>] [--check]");
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or_else(usage)?);
    let mut threads = std::thread::available_parallelism().map_or(1, usize::from);
    let mut check = false;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--threads") => {
                threads = args
                    .next()
                    .and_then(|value| value.to_str()?.parse().ok())
                    .filter(|&count| count > 0)
                    .ok_or_else(usage)?;
            }
            Some("--check") => check = true,
            _ => return Err(usage()),
        }
    }
    let mut file = Recording::open(std::fs::read(&path)?)?;
    // A check also holds every hour to the pages' hour digest chain.
    file.verify(check);
    let file = file;
    let damage = file.damage();
    if check && !damage.is_empty() {
        return Err(invalid(format!(
            "The recording is damaged: {}",
            damage.join("; ")
        )));
    }
    let segments = file.segments();
    let next = AtomicUsize::new(0);
    let walked = Mutex::new(Vec::with_capacity(segments.len()));
    let begin = Instant::now();
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.min(segments.len()))
            .map(|_| {
                scope.spawn(|| -> io::Result<()> {
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&segment) = segments.get(at) else {
                            return Ok(());
                        };
                        let ends = file.restart_after(&segment);
                        let done = walk(&file, segment, ends, check)?;
                        walked.lock().expect("walked segments").push(done);
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .try_for_each(|worker| worker.join().expect("segment worker"))
    })?;
    let wall = begin.elapsed().as_secs_f64();
    let mut walked = walked.into_inner().expect("walked segments");
    walked.sort_by_key(|done| (done.segment.run, done.segment.first));
    let hours: u64 = walked.iter().map(|done| u64::from(done.hours)).sum();
    let cpu_ns: u64 = walked.iter().map(|done| done.cpu_ns).sum();
    let failed = walked
        .iter()
        .filter(|done| done.ends_at_restart == Some(false))
        .count();
    let report = json!({
        "file": path.display().to_string(),
        "hours": hours,
        "segments": walked.len(),
        "threads": threads.min(walked.len()),
        "wall_seconds": wall,
        "cpu_seconds": cpu_ns as f64 / 1e9,
        "cpu_us_per_hour": cpu_ns as f64 / 1e3 / hours.max(1) as f64,
        "checked": check,
        "damage": damage,
        "restart_mismatches": failed,
        "per_segment": walked.iter().map(|done| json!({
            "run": done.segment.run,
            "restart": done.segment.restart,
            "first": done.segment.first,
            "last": done.segment.last,
            "cpu_us_per_hour": done.cpu_ns as f64 / 1e3 / f64::from(done.hours.max(1)),
            "ends_at_restart": done.ends_at_restart,
        })).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    if failed > 0 {
        return Err(invalid(format!(
            "{failed} segments did not end in the next restart's state"
        )));
    }
    Ok(())
}
