
use super::Lane;
use std::{
    sync::{
        OnceLock,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::Instant,
};

pub struct Timing<'a> {
    pub id: &'a str,
    pub lane: Option<&'static str>,
    pub outcome: &'static str,
    pub queue_ms: Option<f64>,
    pub compute_ms: Option<f64>,
    pub deliver_ms: Option<f64>,
    pub total_ms: f64,
    pub bytes: Option<u64>,
}

type Sink = Box<dyn Fn(Timing<'_>) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();

pub fn install(sink: Sink) -> bool {
    SINK.set(sink).is_ok()
}

#[inline]
pub(super) fn enabled() -> bool {
    SINK.get().is_some()
}

const UNSET: u64 = u64::MAX;

pub(super) struct RequestTiming {
    admitted: Instant,
    lane: AtomicU8,
    first_start: AtomicU64,
    last_start: AtomicU64,
    completed: AtomicU64,
    bytes: AtomicU64,
    reported: AtomicBool,
}

impl RequestTiming {
    pub(super) fn new() -> Option<Self> {
        enabled().then(|| Self {
            admitted: Instant::now(),
            lane: AtomicU8::new(u8::MAX),
            first_start: AtomicU64::new(UNSET),
            last_start: AtomicU64::new(UNSET),
            completed: AtomicU64::new(UNSET),
            bytes: AtomicU64::new(UNSET),
            reported: AtomicBool::new(false),
        })
    }

    fn now(&self) -> u64 {
        self.admitted.elapsed().as_nanos().min(u128::from(UNSET - 1)) as u64
    }

    pub(super) fn queued(&self, lane: Lane) {
        let index = Lane::ALL.iter().position(|each| *each == lane).unwrap_or(0);
        self.lane.store(index as u8, Ordering::Relaxed);
    }

    pub(super) fn started(&self) {
        let now = self.now();
        let _ = self
            .first_start
            .compare_exchange(UNSET, now, Ordering::Relaxed, Ordering::Relaxed);
        self.last_start.store(now, Ordering::Relaxed);
    }

    pub(super) fn completed(&self) {
        self.completed.store(self.now(), Ordering::Relaxed);
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
        let (first, last, completed) = (stamp(&self.first_start), stamp(&self.last_start), stamp(&self.completed));
        let queue_ms = first.map(|first| ms(0, first));
        let compute_ms = match (last, completed) {
            (Some(last), Some(completed)) => Some(ms(last, completed)),
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
            bytes: stamp(&self.bytes),
        });
    }
}
