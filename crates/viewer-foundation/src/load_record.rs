//! What a map load did, stage by stage: each stage's thread, its wall span from the record's
//! start and its thread CPU, and counts of what was reused or made by kind. The playback-cost
//! harness's toggle mode keeps it ([`begin`], [`take`]); off (the window), a stage and a count
//! return at once.
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// One stage as it ended: inclusive of the stages it holds.
#[derive(Clone, Debug)]
pub struct Stage {
    pub name: &'static str,
    pub thread: String,
    /// Microseconds from [`begin`] to the stage's start.
    pub start_us: f64,
    pub wall_us: f64,
    /// The stage's own thread's CPU; `None` where the platform does not give it.
    pub cpu_us: Option<f64>,
}

/// A kind's count and bytes over the record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Count {
    pub kind: &'static str,
    pub n: u64,
    pub bytes: u64,
}

#[derive(Default)]
struct Record {
    epoch: Option<Instant>,
    stages: Vec<Stage>,
    counts: Vec<Count>,
}

static ON: AtomicBool = AtomicBool::new(false);
static RECORD: Mutex<Record> = Mutex::new(Record {
    epoch: None,
    stages: Vec::new(),
    counts: Vec::new(),
});

fn record() -> std::sync::MutexGuard<'static, Record> {
    RECORD.lock().unwrap_or_else(|error| error.into_inner())
}

/// Starts a record at `epoch`, dropping what the last one held.
pub fn begin(epoch: Instant) {
    *record() = Record {
        epoch: Some(epoch),
        ..Record::default()
    };
    ON.store(true, Ordering::Release);
}

pub fn on() -> bool {
    ON.load(Ordering::Acquire)
}

/// The stages that ended and the counts since [`begin`], stages in the order they started.
pub fn take() -> (Vec<Stage>, Vec<Count>) {
    let mut record = record();
    let mut stages = std::mem::take(&mut record.stages);
    stages.sort_by(|a, b| a.start_us.total_cmp(&b.start_us));
    (stages, std::mem::take(&mut record.counts))
}

/// Adds `n` and `bytes` to `kind`.
pub fn count(kind: &'static str, n: u64, bytes: u64) {
    if !on() {
        return;
    }
    let mut record = record();
    match record.counts.iter_mut().find(|count| count.kind == kind) {
        Some(count) => {
            count.n += n;
            count.bytes += bytes;
        }
        None => record.counts.push(Count { kind, n, bytes }),
    }
}

/// Times a stage until the guard drops; `None` while no record is kept.
#[must_use = "the stage ends when the guard drops"]
pub fn stage(name: &'static str) -> Option<Timer> {
    on().then(|| Timer {
        name,
        started: Instant::now(),
        cpu: crate::profiling::current_thread_cpu_time(),
    })
}

pub struct Timer {
    name: &'static str,
    started: Instant,
    cpu: Option<Duration>,
}

impl Drop for Timer {
    fn drop(&mut self) {
        let wall = self.started.elapsed();
        let cpu = self
            .cpu
            .zip(crate::profiling::current_thread_cpu_time())
            .and_then(|(before, after)| after.checked_sub(before));
        let thread = std::thread::current();
        let mut record = record();
        let Some(epoch) = record.epoch else {
            return;
        };
        let start_us = self
            .started
            .checked_duration_since(epoch)
            .map_or(0.0, |since| since.as_secs_f64() * 1e6);
        record.stages.push(Stage {
            name: self.name,
            thread: thread.name().unwrap_or("unnamed").to_owned(),
            start_us,
            wall_us: wall.as_secs_f64() * 1e6,
            cpu_us: cpu.map(|cpu| cpu.as_secs_f64() * 1e6),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_keeps_its_stages_in_start_order_and_sums_counts_by_kind() {
        let epoch = Instant::now();
        begin(epoch);
        let outer = stage("outer");
        drop(stage("inner"));
        drop(outer);
        count("textures", 2, 100);
        count("textures", 1, 50);
        count("programs", 1, 0);
        let (stages, counts) = take();
        assert_eq!(stages.iter().map(|stage| stage.name).collect::<Vec<_>>(), ["outer", "inner"]);
        assert!(stages[0].wall_us >= stages[1].wall_us);
        assert_eq!(counts, [Count { kind: "textures", n: 3, bytes: 150 }, Count { kind: "programs", n: 1, bytes: 0 }]);
        begin(Instant::now());
        assert_eq!(take().0.len(), 0, "a new record starts empty");
    }
}
