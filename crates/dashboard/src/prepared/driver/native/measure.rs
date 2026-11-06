//! What a native preparation of the families cost, measured on its production run: CPU per game
//! hour of each family's operators, ordered emit and publication, of the walks shared by every
//! lane and of their world, the slowest serial lane, and the peak resident bytes, beside the goal
//! the whole preparation is judged on ([`GOAL`]). The change-run index, prepared before the
//! families, is not measured here ([`Measured::excludes`]).

use super::backend::Spent;
use serde::Serialize;
use std::{collections::BTreeMap, io, time::Instant};

/// Game hours in fifty years: the game's calendar has no leap days.
const FIFTY_YEARS: f64 = 50.0 * 365.0 * 24.0;

/// Fifty years prepared within a minute on sixteen threads: about 2,200 CPU µs a game hour in
/// all, no serial lane past 137 µs an hour, and resident bytes flat under 6 GB.
pub const GOAL: Goal = Goal {
    wall_seconds: 60.0,
    threads: 16,
    cpu_us_per_hour: 2_200.0,
    serial_us_per_hour: 137.0,
    resident_bytes: 6 << 30,
};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct Goal {
    pub wall_seconds: f64,
    pub threads: usize,
    pub cpu_us_per_hour: f64,
    pub serial_us_per_hour: f64,
    pub resident_bytes: usize,
}

/// One family's CPU per game hour.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Phases {
    /// Its operators, on any worker.
    pub operator_us_per_hour: f64,
    /// Its ordered emit keeping hours, then publishing them: one lane, a segment at a time.
    pub emit_us_per_hour: f64,
    pub publish_us_per_hour: f64,
    pub serial_us_per_hour: f64,
}

/// What the run did and the most it held at once ([`replay::segments::Stats`]).
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Run {
    pub walks: usize,
    pub chained: usize,
    pub deferred: usize,
    pub segments_held: usize,
    /// The most its items and parked operators held on disk at once, and wrote in all.
    pub spool_bytes_held: u64,
    pub spooled_bytes: u64,
    pub carry_bytes_held: usize,
    pub carry_scratch_bytes_held: usize,
    pub parked: usize,
    pub spilled: usize,
    pub loaded: usize,
    pub derived: usize,
    pub measured: usize,
    pub rewalked: usize,
    /// The walkers' buffers held at once, and the most one walk charged its tables.
    pub walker_bytes_held: usize,
    pub walk_table_bytes_held: usize,
}

impl From<replay::segments::Stats> for Run {
    fn from(stats: replay::segments::Stats) -> Self {
        Self {
            walks: stats.walks,
            chained: stats.chained,
            deferred: stats.deferred,
            segments_held: stats.segments,
            spool_bytes_held: stats.spool_bytes,
            spooled_bytes: stats.spooled,
            carry_bytes_held: stats.carry_bytes,
            carry_scratch_bytes_held: stats.carry_scratch,
            parked: stats.parked,
            spilled: stats.spilled,
            loaded: stats.loaded,
            derived: stats.derived,
            measured: stats.measured,
            rewalked: stats.rewalked,
            walker_bytes_held: stats.walker_bytes,
            walk_table_bytes_held: stats.table_bytes,
        }
    }
}

/// The narrow run of the families prepared ahead of the union run, over the whole recording,
/// decoding their tables alone.
#[derive(Clone, Debug, Serialize)]
pub struct Ahead {
    pub families: Vec<&'static str>,
    pub wall_seconds: f64,
    pub cpu_us_per_hour: f64,
    /// Its walks' decode and apply of their tables: its CPU less its families' and its world's.
    pub walk_us_per_hour: f64,
    pub world_us_per_hour: f64,
    pub run: Run,
    /// The most each segment's walks charged their tables, by segment.
    pub walk_table_bytes: Vec<usize>,
}

/// A measured native preparation.
#[derive(Clone, Debug, Serialize)]
pub struct Measured {
    pub hours: u64,
    pub segments: usize,
    pub threads: usize,
    /// Its runs walked for lanes doing no family's work: what they cost is the floor under
    /// the families' ([`super::Floor`]).
    pub walk_only: bool,
    /// What the measurement leaves out of the whole preparation.
    pub excludes: &'static [&'static str],
    /// The families' preparation: opening, content, the run and its commits.
    pub wall_seconds: f64,
    pub cpu_us_per_hour: f64,
    pub content_wall_seconds: f64,
    /// The narrow run of the families prepared ahead, when it ran: none of its time or CPU is
    /// the union run's.
    pub ahead: Option<Ahead>,
    /// The union run.
    pub run_wall_seconds: f64,
    pub run_cpu_us_per_hour: f64,
    /// The walks' decode and apply of the selected tables, shared by every lane: the run's CPU
    /// less the families' and the worlds'. Below zero, those count more than the run spent.
    pub walk_us_per_hour: f64,
    pub world_us_per_hour: f64,
    pub families: BTreeMap<&'static str, Phases>,
    /// What families counted over the run, by family and name, for those that count.
    pub counts: BTreeMap<&'static str, BTreeMap<&'static str, u64>>,
    /// The most each family spooled in one walk of a segment.
    pub largest_output_bytes: BTreeMap<&'static str, usize>,
    /// The slowest family's serial lane.
    pub serial_us_per_hour: f64,
    pub peak_working_set_bytes: usize,
    pub peak_commit_bytes: usize,
    pub resident: Resident,
    pub run: Run,
    /// The most each segment's walks charged their tables, by segment.
    pub walk_table_bytes: Vec<usize>,
    /// Fifty years of the families' preparation at both runs' wall time per hour, less what it
    /// excludes ([`Self::excludes`]).
    pub fifty_years_seconds: f64,
    pub goal: Goal,
}

/// What the preparation held resident, by holder, in bytes.
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct Resident {
    /// The process's working set as the preparation began.
    pub start_bytes: usize,
    /// What loading content added to it.
    pub content_bytes: usize,
    /// Parked operators held resident at most: projected items wait on disk.
    pub carry_bytes: usize,
    /// The most the walkers' buffers held.
    pub walker_held_bytes: usize,
    /// The most one walk charged its tables.
    pub walk_table_bytes: usize,
    /// The working set as the run ended, its families' states still held, less at its start.
    pub run_end_bytes: usize,
}

/// The process's working set now, in bytes.
pub(crate) fn working_set() -> io::Result<usize> {
    counters().map(|counters| counters.working)
}

/// Wall time, process CPU and the cycle counter, from a moment on.
#[derive(Clone, Copy)]
pub(crate) struct Clock {
    wall: Instant,
    process: u64,
    stamp: u64,
}

/// What passed since a [`Clock`] started.
#[derive(Clone, Copy)]
pub(crate) struct Lap {
    pub seconds: f64,
    /// Process CPU nanoseconds.
    pub cpu: u64,
    pub stamps: u64,
}

impl Clock {
    pub(crate) fn start() -> Self {
        Self {
            wall: Instant::now(),
            process: process_cpu(),
            stamp: replay_frame::cpu::stamp(),
        }
    }

    pub(crate) fn lap(&self) -> Lap {
        Lap {
            seconds: self.wall.elapsed().as_secs_f64(),
            cpu: process_cpu().saturating_sub(self.process),
            stamps: replay_frame::cpu::stamp().wrapping_sub(self.stamp),
        }
    }
}

/// What the narrow run of the families prepared ahead counted.
pub(crate) struct Narrow {
    pub lap: Lap,
    pub world: u64,
    pub stats: replay::segments::Stats,
    pub tables: Vec<usize>,
    pub families: Vec<&'static str>,
}

/// Everything a measured preparation counted.
pub(crate) struct Counted<'a> {
    pub hours: u64,
    pub segments: usize,
    pub threads: usize,
    pub walk_only: bool,
    pub whole: Lap,
    pub content: Lap,
    pub run: Lap,
    pub world: u64,
    pub families: &'a [(&'static str, Spent)],
    pub counts: &'a [(&'static str, Vec<(&'static str, u64)>)],
    pub largest: &'a [(&'static str, Option<usize>)],
    pub stats: replay::segments::Stats,
    pub tables: Vec<usize>,
    /// The working set as the preparation began, after content loaded, and as the run ended.
    pub working: [usize; 3],
    pub ahead: Option<Narrow>,
}

impl Measured {
    pub(crate) fn new(counted: Counted<'_>) -> io::Result<Self> {
        let hours = counted.hours.max(1) as f64;
        // Thread cycles count at the counter's rate, taken over the run.
        let rate = counted.run.stamps as f64 / counted.run.seconds.max(f64::MIN_POSITIVE);
        let us = |cycles: u64| cycles as f64 / rate.max(f64::MIN_POSITIVE) * 1e6 / hours;
        let cpu = |lap: Lap| lap.cpu as f64 / 1e3 / hours;
        let mut families = BTreeMap::new();
        let (mut spent, mut ahead) = (0, 0);
        for &(name, family) in counted.families {
            let cycles = family.operators + family.emit + family.publish;
            match &counted.ahead {
                Some(narrow) if narrow.families.contains(&name) => ahead += cycles,
                _ => spent += cycles,
            }
            let (emit, publish) = (us(family.emit), us(family.publish));
            families.insert(
                name,
                Phases {
                    operator_us_per_hour: us(family.operators),
                    emit_us_per_hour: emit,
                    publish_us_per_hour: publish,
                    serial_us_per_hour: emit + publish,
                },
            );
        }
        let serial = families
            .values()
            .map(|phases| phases.serial_us_per_hour)
            .fold(0.0, f64::max);
        let (working, commit) = peaks()?;
        let narrow_seconds = counted
            .ahead
            .as_ref()
            .map_or(0.0, |narrow| narrow.lap.seconds);
        let narrow_stats = counted.ahead.as_ref().map(|narrow| narrow.stats);
        let held = |of: fn(&replay::segments::Stats) -> usize| {
            narrow_stats
                .iter()
                .chain([&counted.stats])
                .map(of)
                .max()
                .unwrap_or(0)
        };
        let walker_held = held(|stats| stats.walker_bytes);
        let table_held = held(|stats| stats.table_bytes);
        let narrow = counted.ahead.map(|narrow| Ahead {
            families: narrow.families,
            wall_seconds: narrow.lap.seconds,
            cpu_us_per_hour: cpu(narrow.lap),
            walk_us_per_hour: cpu(narrow.lap) - us(ahead) - us(narrow.world),
            world_us_per_hour: us(narrow.world),
            run: narrow.stats.into(),
            walk_table_bytes: narrow.tables,
        });
        Ok(Self {
            hours: counted.hours,
            segments: counted.segments,
            threads: counted.threads,
            walk_only: counted.walk_only,
            excludes: &["the change-run index, prepared before the families"],
            wall_seconds: counted.whole.seconds,
            cpu_us_per_hour: cpu(counted.whole),
            content_wall_seconds: counted.content.seconds,
            ahead: narrow,
            run_wall_seconds: counted.run.seconds,
            run_cpu_us_per_hour: cpu(counted.run),
            walk_us_per_hour: cpu(counted.run) - us(spent) - us(counted.world),
            world_us_per_hour: us(counted.world),
            families,
            counts: counted
                .counts
                .iter()
                .filter(|(_, counts)| !counts.is_empty())
                .map(|(name, counts)| (*name, counts.iter().copied().collect()))
                .collect(),
            largest_output_bytes: counted
                .largest
                .iter()
                .filter_map(|&(name, largest)| Some((name, largest?)))
                .collect(),
            serial_us_per_hour: serial,
            peak_working_set_bytes: working,
            peak_commit_bytes: commit,
            resident: Resident {
                start_bytes: counted.working[0],
                content_bytes: counted.working[1].saturating_sub(counted.working[0]),
                carry_bytes: counted.stats.carry_bytes,
                walker_held_bytes: walker_held,
                walk_table_bytes: table_held,
                run_end_bytes: counted.working[2].saturating_sub(counted.working[0]),
            },
            run: counted.stats.into(),
            walk_table_bytes: counted.tables,
            fifty_years_seconds: (counted.run.seconds + narrow_seconds) / hours * FIFTY_YEARS,
            goal: GOAL,
        })
    }
}

/// The process's CPU so far, in nanoseconds.
fn process_cpu() -> u64 {
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

/// The calling thread's CPU so far, in cycles of the counter [`replay_frame::cpu::stamp`] reads.
pub(crate) fn cycles() -> u64 {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut cycles = 0;
    // SAFETY: the pseudo-handle of the current thread and an owned counter.
    unsafe {
        QueryThreadCycleTime(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            &mut cycles,
        )
    };
    cycles
}

/// The process's peak working set and peak commit so far, in bytes.
pub fn peaks() -> io::Result<(usize, usize)> {
    counters().map(|counters| (counters.peak_working, counters.peak_pagefile))
}

#[repr(C)]
#[derive(Default)]
struct Counters {
    size: u32,
    faults: u32,
    peak_working: usize,
    working: usize,
    peak_paged: usize,
    paged: usize,
    peak_nonpaged: usize,
    nonpaged: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

/// The process's memory counters.
fn counters() -> io::Result<Counters> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: windows_sys::Win32::Foundation::HANDLE,
            counters: *mut Counters,
            size: u32,
        ) -> i32;
    }
    let size = size_of::<Counters>() as u32;
    let mut counters = Counters {
        size,
        ..Default::default()
    };
    // SAFETY: the pseudo-handle of the current process and an owned block of `size` bytes.
    if unsafe {
        K32GetProcessMemoryInfo(
            windows_sys::Win32::System::Threading::GetCurrentProcess(),
            &mut counters,
            size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(counters)
}
