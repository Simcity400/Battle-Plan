//! How fast a change-run index serves the map of a native columnar recording, as JSON: its
//! preparation's wall, the process's CPU per recorded hour and the index's bytes per hour;
//! moves to random hours far apart, to hours near the one held and hour after hour, each timed
//! without and then with its frame's build; rows read by key from a few tables; and the
//! process's peak memory after preparing and after reading.
//!
//! `map-seek <recording.h4col> [--replay <replay.battleplan>] [--index <file.h4runs>]
//!  [--prepared] [--threads N] [--workers N] [--level L] [--seeks N] [--play HOURS] [--seed S]`
//!
//! The index is prepared into `--index` (by default the recording's path with `.h4runs` added)
//! on `--threads` workers, unless `--prepared` reads the one there. The replay, when given,
//! names the opaque cells' format and the capture mode; a native recording's are this build's
//! and full.

use dashboard_base::map_tables::selected_tables;
use engine_allocation::credit::MemoryBudget;
use replay::{
    columnar_frames::{Limits, Recording},
    columnar_runs::{self, IndexFile, Layout, Snapshots, Whole},
    container::Parallel,
};
use replay_data::{format::FormatVersion, rows::TableId, scope::CaptureMode};
use replay_frame::frame::{DecodedFrame, TableView};
use serde_json::{Value, json};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, BufWriter},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

struct Options {
    file: PathBuf,
    replay: Option<PathBuf>,
    index: Option<PathBuf>,
    prepared: bool,
    threads: usize,
    workers: usize,
    level: i32,
    seeks: usize,
    play: u32,
    seed: u64,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

fn options() -> io::Result<Options> {
    let usage = "map-seek <recording.h4col> [--replay <replay.battleplan>] [--index <file.h4runs>] [--prepared] [--threads N] [--workers N] [--level L] [--seeks N] [--play HOURS] [--seed S]";
    let cores = std::thread::available_parallelism().map_or(1, usize::from);
    let mut args = std::env::args_os().skip(1);
    let mut file = None;
    let mut options = Options {
        file: PathBuf::new(),
        replay: None,
        index: None,
        prepared: false,
        threads: cores,
        workers: (cores / 2).max(1),
        level: columnar_runs::LEVEL,
        seeks: 200,
        play: 2_000,
        seed: 1,
    };
    let number = |value: Option<OsString>| -> io::Result<u64> {
        value
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse().ok())
            .ok_or_else(|| invalid(usage))
    };
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--replay") => {
                options.replay = Some(PathBuf::from(args.next().ok_or_else(|| invalid(usage))?))
            }
            Some("--index") => {
                options.index = Some(PathBuf::from(args.next().ok_or_else(|| invalid(usage))?))
            }
            Some("--prepared") => options.prepared = true,
            Some("--threads") => options.threads = (number(args.next())? as usize).max(1),
            Some("--workers") => options.workers = (number(args.next())? as usize).max(1),
            Some("--level") => {
                options.level = i32::try_from(number(args.next())?).map_err(|_| invalid(usage))?
            }
            Some("--seeks") => options.seeks = number(args.next())? as usize,
            Some("--play") => {
                options.play = u32::try_from(number(args.next())?).map_err(|_| invalid(usage))?
            }
            Some("--seed") => options.seed = number(args.next())?,
            Some(flag) if flag.starts_with("--") => return Err(invalid(usage)),
            _ if file.is_none() => file = Some(PathBuf::from(arg)),
            _ => return Err(invalid(usage)),
        }
    }
    options.file = file.ok_or_else(|| invalid(usage))?;
    Ok(options)
}

/// A walk of `next`'s numbers: the same for the same seed.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) % bound.max(1)
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

/// p50, p90, p99 and the most of `samples`, in milliseconds.
fn percentiles(samples: &mut [Duration]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    samples.sort_unstable();
    let at = |share: f64| {
        let index = ((samples.len() as f64 * share).ceil() as usize).clamp(1, samples.len()) - 1;
        ms(samples[index])
    };
    json!({
        "count": samples.len(),
        "p50_ms": at(0.5),
        "p90_ms": at(0.9),
        "p99_ms": at(0.99),
        "max_ms": ms(samples[samples.len() - 1]),
        "mean_ms": ms(samples.iter().sum::<Duration>()) / samples.len() as f64,
    })
}

/// The process's CPU nanoseconds so far.
fn cpu() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the current process and four owned FILETIMEs.
    unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    (ticks(kernel) + ticks(user)) * 100
}

/// The process's peak working set and peak committed bytes so far.
fn peak_memory() -> Value {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged_pool: usize,
        quota_paged_pool: usize,
        quota_peak_non_paged_pool: usize,
        quota_non_paged_pool: usize,
        committed: usize,
        peak_committed: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: windows_sys::Win32::Foundation::HANDLE,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let mut counters = Counters {
        size: size_of::<Counters>() as u32,
        ..Counters::default()
    };
    // SAFETY: the pseudo-handle of the current process and an owned block of the size it names.
    let read = unsafe {
        K32GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            &mut counters,
            counters.size,
        )
    };
    if read == 0 {
        return Value::Null;
    }
    json!({
        "peak_working_set_mb": counters.peak_working_set as f64 / (1 << 20) as f64,
        "peak_committed_mb": counters.peak_committed as f64 / (1 << 20) as f64,
    })
}

/// Times of moves to hours, each without its frame and then with its build.
#[derive(Default)]
struct Moves {
    moved: Vec<Duration>,
    built: Vec<Duration>,
    pages: Vec<usize>,
    bytes: Vec<usize>,
}

impl Moves {
    /// Moves `snapshots` to `hour` of `branch`, then builds its frame.
    fn take(
        &mut self,
        snapshots: &mut Snapshots,
        parallel: &Parallel<'_>,
        branch: u64,
        hour: i32,
    ) -> io::Result<DecodedFrame> {
        let started = Instant::now();
        if !snapshots.move_to(branch, hour, parallel.on_worker, &|| false)? {
            return Err(invalid(format!(
                "No run holds hour {hour} of branch {branch}"
            )));
        }
        let moved = started.elapsed();
        let (pages, bytes) = snapshots.read();
        let frame = snapshots.build(Some(parallel))?;
        self.moved.push(moved);
        self.built.push(started.elapsed());
        self.pages.push(pages);
        self.bytes.push(bytes);
        Ok(frame)
    }

    fn json(mut self) -> Value {
        let most = |values: &[usize]| values.iter().copied().max().unwrap_or(0);
        let mean =
            |values: &[usize]| values.iter().sum::<usize>() as f64 / values.len().max(1) as f64;
        json!({
            "move": percentiles(&mut self.moved),
            "move_and_build": percentiles(&mut self.built),
            "pages_mean": mean(&self.pages),
            "pages_max": most(&self.pages),
            "page_mb_mean": mean(&self.bytes) / (1 << 20) as f64,
            "page_mb_max": most(&self.bytes) as f64 / (1 << 20) as f64,
        })
    }
}

/// The keys of the first `count` rows of `table` in `frame`.
fn keys(frame: &DecodedFrame, table: TableId, count: usize) -> Vec<u128> {
    match &*frame.tables[table.index()] {
        TableView::Complete(rows) | TableView::Truncated { rows, .. } => {
            rows.iter().take(count).map(|row| row.key()).collect()
        }
        _ => Vec::new(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = options()?;
    let (format, mode) = match &options.replay {
        Some(path) => {
            let replay = replay::container::Replay::open_path(path)?;
            (replay.format, replay.capture_mode)
        }
        None => (FormatVersion::CURRENT, CaptureMode::Full),
    };
    let tables = selected_tables();
    let path = options.index.clone().unwrap_or_else(|| {
        let mut path = options.file.clone().into_os_string();
        path.push(".h4runs");
        PathBuf::from(path)
    });
    let mut recording = Recording::open_path(&options.file, Limits::default())?;
    recording.select(tables);
    let hours = recording.hours();
    let file_bytes = recording.len();
    // The one native recording given, every session of it ended.
    let whole = Whole {
        campaign: recording.campaign().to_vec(),
        recordings: vec![recording],
        pending: Vec::new(),
    };

    let preparation = if options.prepared {
        Value::Null
    } else {
        let (cpu_before, started) = (cpu(), Instant::now());
        let scratch = match path.parent() {
            Some(folder) if !folder.as_os_str().is_empty() => folder.to_path_buf(),
            _ => PathBuf::from("."),
        };
        let (out, stats) = columnar_runs::prepare(
            &whole,
            tables,
            Layout::RECORDING,
            options.level,
            options.threads,
            &scratch,
            &AtomicBool::new(false),
            BufWriter::new(File::create(&path)?),
        )?;
        out.into_inner()
            .map_err(io::IntoInnerError::into_error)?
            .sync_all()?;
        let (wall, cpu_ns) = (started.elapsed(), cpu() - cpu_before);
        json!({
            "threads": options.threads,
            "level": options.level,
            "wall_s": wall.as_secs_f64(),
            "cpu_us_per_hour": cpu_ns as f64 / 1_000.0 / hours.max(1) as f64,
            "wall_us_per_hour": wall.as_secs_f64() * 1e6 / hours.max(1) as f64,
            "walks": stats.iter().map(|stats| stats.walks).sum::<usize>(),
            "spool_mb": stats.iter().map(|stats| stats.spool_bytes).max().unwrap_or(0) as f64
                / (1 << 20) as f64,
            "memory": peak_memory(),
        })
    };

    let opened = Instant::now();
    let file = Arc::new(IndexFile::open(&path)?);
    let open = opened.elapsed();
    if !file.indexes(&whole.recordings, &whole.pending, tables) {
        return Err(invalid(format!(
            "{} does not index {}",
            path.display(),
            options.file.display()
        ))
        .into());
    }
    let all: Vec<(u64, i32)> = file
        .index
        .runs
        .iter()
        .flat_map(|run| run.hours.iter().map(move |&hour| (run.branch, hour)))
        .collect();
    let alone = || {};
    let parallel = Parallel {
        workers: options.workers,
        on_worker: &alone,
    };
    let mut snapshots = Snapshots::new(
        Arc::clone(&file),
        tables,
        format,
        mode,
        options.workers,
        MemoryBudget::new(usize::MAX / 4),
    )?
    .with_change_facts(MemoryBudget::new(usize::MAX / 4));
    let mut random = Random(options.seed);
    let pick = |random: &mut Random| all[random.below(all.len() as u64) as usize];

    // The first move builds every view from nothing.
    let mut first = Moves::default();
    let (branch, hour) = pick(&mut random);
    first.take(&mut snapshots, &parallel, branch, hour)?;

    let mut far = Moves::default();
    for _ in 0..options.seeks {
        let (branch, hour) = pick(&mut random);
        far.take(&mut snapshots, &parallel, branch, hour)?;
    }

    // From a random hour, to one up to a day either way in its run.
    let mut near = Moves::default();
    for _ in 0..options.seeks {
        let at = random.below(all.len() as u64) as usize;
        let (branch, hour) = all[at];
        snapshots.frame(branch, hour, Some(&parallel))?;
        let offset = random.below(49) as usize;
        let to = (at + offset).saturating_sub(24).min(all.len() - 1);
        let (to_branch, to_hour) = all[to];
        if to_branch == branch {
            near.take(&mut snapshots, &parallel, to_branch, to_hour)?;
        }
    }

    // Hour after hour from a random hour, as playback asks.
    let mut play = Moves::default();
    let start = random.below(all.len() as u64) as usize;
    let (branch, _) = all[start];
    let mut frame = None;
    for &(held, hour) in all[start..].iter().take(options.play as usize) {
        if held != branch {
            break;
        }
        frame = Some(play.take(&mut snapshots, &parallel, held, hour)?);
    }

    // A few tables' rows by key, at random hours.
    let mut rows = serde_json::Map::new();
    if let Some(frame) = &frame {
        for table in [
            TableId::Provinces,
            TableId::Countries,
            TableId::Divisions,
            TableId::States,
        ] {
            if !file.holds(&[table]) {
                continue;
            }
            let mut timed: Vec<(usize, Vec<Duration>)> = vec![(1, Vec::new()), (10, Vec::new())];
            for (count, samples) in &mut timed {
                let asked = keys(frame, table, *count);
                if asked.is_empty() {
                    continue;
                }
                for _ in 0..options.seeks.min(100) {
                    let (branch, hour) = pick(&mut random);
                    let started = Instant::now();
                    file.rows_at(branch, hour, table, &asked, format, mode)?;
                    samples.push(started.elapsed());
                }
            }
            rows.insert(
                format!("{table:?}"),
                json!({
                    "one_key": percentiles(&mut timed[0].1),
                    "ten_keys": percentiles(&mut timed[1].1),
                }),
            );
        }
    }

    let report = json!({
        "file": options.file.display().to_string(),
        "file_bytes": file_bytes,
        "hours": hours,
        "tables": tables.len(),
        "workers": options.workers,
        "preparation": preparation,
        "index": {
            "path": path.display().to_string(),
            "bytes": file.len(),
            "bytes_per_hour": file.len() as f64 / hours.max(1) as f64,
            "open_ms": ms(open),
            "pages_read": file.reads().0,
            "dictionary_chunks_read": file.reads().1,
            "dictionary_chunks": file.index.runs.iter().map(|run| run.chunks.len()).sum::<usize>(),
        },
        "first": first.json(),
        "far": far.json(),
        "near": near.json(),
        "play": play.json(),
        "rows_at": rows,
        "held_mb": snapshots.held_bytes() as f64 / (1 << 20) as f64,
        "memory": peak_memory(),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
