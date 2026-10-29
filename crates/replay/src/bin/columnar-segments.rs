//! Walks a `p3-convert` columnar recording one segment per worker: each segment from its
//! restart (or the recording's start) through the hour before the next restart, every
//! segment on its own walker state. Reports wall time, the walkers' CPU per recorded hour and
//! each segment's, a segment at a time. `--check` also requires an undamaged recording and
//! every segment to end in exactly the state of the restart holding its last hour, its run's
//! last restart included, every string of its dictionary too.
//! `columnar-segments <file.h4col> [--threads <n>] [--check]`

use replay::columnar_frames::{Limits, Recording, Segment, Walker, schema};
use replay_columnar::walk::{Dictionary, State};
use serde_json::json;
use std::{
    io,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
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

/// Whether two dictionaries hold the same strings at every id, each read back from its run's
/// chunks where a walker dropped it.
fn same_strings(left: &Dictionary, right: &Dictionary) -> io::Result<bool> {
    let (mut left_chunk, mut right_chunk) = (None, None);
    for id in 0..left.len().min(right.len()) as u32 {
        if left.resolve(id, &mut left_chunk)? != right.resolve(id, &mut right_chunk)? {
            return Ok(false);
        }
    }
    Ok(left.len() == right.len())
}

/// Whether `state` holds what the restart on page `at` of `run` restores: every table,
/// status, sample hour and truncated view, and every string of the dictionary. The walker
/// that walked it restores, so no worker waits for a second share of the budget.
fn same_as_restart(
    walker: &mut Walker<'_>,
    run: u32,
    at: usize,
    state: &State,
) -> io::Result<bool> {
    let mut restored = State::new(schema()?)?;
    walker.restore(run, at, &mut restored)?;
    let tables = state.columns().len();
    Ok(same_strings(state.dictionary(), restored.dictionary())?
        && (0..tables).all(|table| {
            state.table(table) == restored.table(table)
                && state.status(table) == restored.status(table)
                && state.sampled_at(table) == restored.sampled_at(table)
                && state.truncated_view(table) == restored.truncated_view(table)
        }))
}

fn walk(
    walker: &mut Walker<'_>,
    segment: Segment,
    next: Option<usize>,
    check: bool,
) -> io::Result<Walked> {
    let begin = thread_ns();
    let mut state = State::new(schema()?)?;
    let mut hours = 0;
    walker.begin(&segment, &mut state)?;
    walker.walk(&segment, &mut state, |_, _, _| {
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
        (true, Some(at)) => Some(same_as_restart(walker, segment.run, at, &state)?),
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
    let mut file = Recording::open_path(&path, Limits::default())?;
    // A check also holds every hour to the pages' hour digest chain.
    file.verify(check);
    let file = file;
    let damage: Vec<String> = file.damage().iter().map(ToString::to_string).collect();
    if check && !damage.is_empty() {
        return Err(invalid(format!(
            "The recording is damaged: {}",
            damage.join("; ")
        )));
    }
    let segments = file.segments();
    let workers = threads.min(segments.len());
    let (next, stop) = (AtomicUsize::new(0), AtomicBool::new(false));
    let totals = Mutex::new((0_u64, 0_u64, 0_u64, 0_u64));
    let begin = Instant::now();
    print!("{{\"per_segment\":[");
    std::thread::scope(|scope| {
        let workers: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| -> io::Result<()> {
                    // A worker's walker keeps its buffers from segment to segment.
                    let mut walker = Walker::new(&file, &stop)?;
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&segment) = segments.get(at) else {
                            return Ok(());
                        };
                        let ends = file.restart_after(&segment);
                        let done = walk(&mut walker, segment, ends, check)?;
                        let mut totals = totals.lock().expect("walked segments");
                        if totals.0 != 0 {
                            print!(",");
                        }
                        print!(
                            "{}",
                            json!({
                                "run": done.segment.run,
                                "restart": done.segment.restart,
                                "first": done.segment.first,
                                "last": done.segment.last,
                                "cpu_us_per_hour": done.cpu_ns as f64 / 1e3 / f64::from(done.hours.max(1)),
                                "ends_at_restart": done.ends_at_restart,
                            })
                        );
                        totals.0 += 1;
                        totals.1 += u64::from(done.hours);
                        totals.2 += done.cpu_ns;
                        totals.3 += u64::from(done.ends_at_restart == Some(false));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .try_for_each(|worker| worker.join().expect("segment worker"))
    })?;
    let (walked, hours, cpu_ns, failed) = totals.into_inner().expect("walked segments");
    let report = json!({
        "file": path.display().to_string(),
        "hours": hours,
        "segments": walked,
        "threads": workers,
        "wall_seconds": begin.elapsed().as_secs_f64(),
        "cpu_seconds": cpu_ns as f64 / 1e9,
        "cpu_us_per_hour": cpu_ns as f64 / 1e3 / hours.max(1) as f64,
        "checked": check,
        "damage": damage,
        "restart_mismatches": failed,
    });
    println!("],\"summary\":{report}}}");
    if failed > 0 {
        return Err(invalid(format!(
            "{failed} segments did not end in the next restart's state"
        )));
    }
    Ok(())
}
