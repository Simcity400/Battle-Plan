//! Opt-in request timings for a measurement run. Nothing is timed until a host installs a sink.

use super::Lane;
use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};

/// One request's path through the service, in milliseconds.
pub struct Timing<'a> {
    pub id: &'a str,
    /// The lane that last queued it.
    pub lane: Option<&'static str>,
    /// `ok` (last fragment handed to the host), `cancelled` or `error`.
    pub outcome: &'static str,
    /// Waiting in lane queues, both queues for a read that moved to the sampled lane.
    pub queue_ms: Option<f64>,
    /// Running on loaders, including a prepared attempt that moved to the sampled lane.
    pub compute_ms: Option<f64>,
    /// From the finished result until its last fragment was handed to the host.
    pub deliver_ms: Option<f64>,
    /// From admission until the request ended.
    pub total_ms: f64,
    /// Moved from the prepared lane to the sampled lane.
    pub sampled: bool,
    /// The reply JSON's length.
    pub bytes: Option<u64>,
    /// Answered from the disk reply cache.
    pub disk_cached: bool,
}

type Sink = Box<dyn Fn(Timing<'_>) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

/// Times every later request and hands each to `sink` as it ends; false if one was installed.
pub fn install(sink: Sink) -> bool {
    SINK.set(sink).is_ok()
}

#[inline]
pub(super) fn enabled() -> bool {
    SINK.get().is_some()
}

thread_local! {
    static DISK_HIT: Cell<bool> = const { Cell::new(false) };
}

/// The job on this thread read its reply from the disk reply cache.
pub(crate) fn note_disk_hit() {
    if enabled() {
        DISK_HIT.with(|hit| hit.set(true));
    }
}

const UNSET: u64 = u64::MAX;

/// Nanoseconds after admission, [`UNSET`] until stamped.
pub(super) struct RequestTiming {
    admitted: Instant,
    lane: AtomicU8,
    first_start: AtomicU64,
    requeued: AtomicU64,
    last_start: AtomicU64,
    completed: AtomicU64,
    bytes: AtomicU64,
    disk_cached: AtomicBool,
    reported: AtomicBool,
}

impl RequestTiming {
    pub(super) fn new() -> Option<Self> {
        enabled().then(|| Self {
            admitted: Instant::now(),
            lane: AtomicU8::new(u8::MAX),
            first_start: AtomicU64::new(UNSET),
            requeued: AtomicU64::new(UNSET),
            last_start: AtomicU64::new(UNSET),
            completed: AtomicU64::new(UNSET),
            bytes: AtomicU64::new(UNSET),
            disk_cached: AtomicBool::new(false),
            reported: AtomicBool::new(false),
        })
    }

    fn now(&self) -> u64 {
        self.admitted.elapsed().as_nanos().min(u128::from(UNSET - 1)) as u64
    }

    pub(super) fn queued(&self, lane: Lane) {
        let index = Lane::ALL.iter().position(|each| *each == lane).unwrap_or(0);
        self.lane.store(index as u8, Ordering::Relaxed);
        if self.first_start.load(Ordering::Relaxed) != UNSET {
            self.requeued.store(self.now(), Ordering::Relaxed);
        }
    }

    pub(super) fn started(&self) {
        let now = self.now();
        let _ = self
            .first_start
            .compare_exchange(UNSET, now, Ordering::Relaxed, Ordering::Relaxed);
        self.last_start.store(now, Ordering::Relaxed);
        DISK_HIT.with(|hit| hit.set(false));
    }

    pub(super) fn completed(&self) {
        self.completed.store(self.now(), Ordering::Relaxed);
        if DISK_HIT.with(|hit| hit.replace(false)) {
            self.disk_cached.store(true, Ordering::Relaxed);
        }
    }

    pub(super) fn bytes(&self, bytes: usize) {
        self.bytes.store(bytes as u64, Ordering::Relaxed);
    }

    pub(super) fn report(&self, id: &str, outcome: &'static str) {
        if self.reported.swap(true, Ordering::AcqRel) {
            return;
        }
        let Some(sink) = SINK.get() else {
            return;
        };
        let end = self.now();
        let stamp = |value: &AtomicU64| Some(value.load(Ordering::Relaxed)).filter(|&v| v != UNSET);
        let ms = |from: u64, to: u64| to.saturating_sub(from) as f64 / 1e6;
        let (first, requeued, last, completed) = (
            stamp(&self.first_start),
            stamp(&self.requeued),
            stamp(&self.last_start),
            stamp(&self.completed),
        );
        let sampled = requeued.is_some();
        let queue_ms = first.map(|first| {
            ms(0, first)
                + match (requeued, last) {
                    (Some(requeued), Some(last)) if last >= requeued => ms(requeued, last),
                    (Some(requeued), _) => ms(requeued, end),
                    _ => 0.0,
                }
        });
        let compute_ms = match (first, last, completed) {
            (Some(first), Some(last), Some(completed)) => Some(match requeued {
                Some(requeued) if last >= requeued => ms(last, completed) + ms(first, requeued),
                _ => ms(last, completed),
            }),
            _ => None,
        };
        let lane = self.lane.load(Ordering::Relaxed);
        sink(Timing {
            id,
            lane: Lane::ALL.get(usize::from(lane)).map(|lane| lane.name()),
            outcome,
            queue_ms,
            compute_ms,
            deliver_ms: completed.map(|completed| ms(completed, end)),
            total_ms: ms(0, end),
            sampled,
            bytes: stamp(&self.bytes),
            disk_cached: self.disk_cached.load(Ordering::Relaxed),
        });
    }
}
