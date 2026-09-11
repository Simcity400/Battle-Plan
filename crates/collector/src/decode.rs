//! Single FIFO decode worker for the research pilot family (copy/decode split).
//!
//! The game thread copies raw research bytes only (via the factored engine
//! `copy_*` helpers in `extended::governance`), enqueues the raw job with
//! hour/seq metadata, and records copy-only nanoseconds in `table_ns`. This
//! worker interprets the raw buffers off the game thread (via the factored
//! `decode_*` helpers, reached through the governance decode path), installs
//! the identical typed rows into the same pooled frame, and hands the frame
//! to the spool ready queue. Conversion, differencing, encoding and baselines
//! downstream of the ready queue are untouched.
//!
//! Ordering: every frame transits this one FIFO in game-thread production
//! order, so spool handoff order is inherently hour order and no reorder
//! buffer exists. Skip (off-refresh) hours carry no payload and bypass decode
//! work entirely, but they still transit the FIFO as ordering placeholders so
//! a slow refresh decode can never let a later skip hour overtake it.
//!
//! Backpressure: the game thread never blocks. The FIFO holds one lightweight
//! job per pooled buffer (payload byte buffers already live on the heap;
//! payload jobs are bounded and recycled, never pool memory), so an owned
//! buffer guarantees a free slot exactly as the spool ready queue does today.
//! A full queue takes the existing whole-frame drop plus `LossBatch` gap path
//! with the existing counters and reason strings.
//!
//! Queue sizing (analytic, provisional until live calibration): the FIFO depth
//! equals the buffer pool (`BUFFERS` = 256 indices, ~2 KiB). Raw payloads are
//! recycled job boxes (`JOB_POOL_KEEP` = 2 live payloads plus the one being
//! copied): a late-game refresh carries on the order of 242k technology
//! instances x 52 B (~12.6 MB) plus redundant per-occurrence template headers
//! (~552 templates x 439 countries x ~96 B ≈ 23 MB worst case, deduplicated on
//! the decode thread) plus slots (~0.2 MB), so one payload is bounded near
//! ~40 MB and the live set near ~120 MB transient, frozen after warmup and
//! recycled, never reallocated in steady state. Game-thread production is one
//! refresh per 4 game hours (hundreds of milliseconds at speed 5) against an
//! expected decode cost in the tens of milliseconds, so the steady-state queue
//! depth is ~1; depth 2 absorbs stalls and anything beyond drops through the
//! existing gap path. Live measurement (not this change) confirms or resizes.
//!
//! Calibration: per-pilot-table copy ns, decode ns and raw bytes are monotonic
//! additive counters readable from status (`calibration_json`, merged into the
//! runtime status additively). Per-frame copy-only ns also stays in the
//! existing `table_ns` slots for the pilot tables.

use crate::{
    extended::governance::ResearchJob,
    sampling::{self, BUFFERS, Shared},
};
use crossbeam_queue::ArrayQueue;
use replay::rows::TableId;
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

/// Pilot family in calibration order: [technologies, slots, definitions].
pub const PILOT: [TableId; 3] = [
    TableId::ResearchTechnologies,
    TableId::ResearchSlots,
    TableId::TechnologyDefinitions,
];

/// Live raw payloads beyond this count are not retained; further refreshes
/// take the existing gap path until the worker catches up (see module docs).
const JOB_POOL_KEEP: usize = 2;

/// One FIFO item. The payload's byte buffers already live on the heap inside
/// its vectors; pooled-frame typed rows are filled by the worker before the
/// ready-queue handoff.
#[derive(Debug)]
pub struct DecodeJob {
    pub index: usize,
    pub seq: u64,
    pub hour: i32,
    pub pilot_refresh: bool,
    pub force_full: bool,
    pub game_elapsed: i64,
    pub payload: Option<ResearchJob>,
}

static COPY_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static DECODE_NS: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static RAW_BYTES: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static JOBS: AtomicU64 = AtomicU64::new(0);
static REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static FULL_REFRESH_JOBS: AtomicU64 = AtomicU64::new(0);
static DROPS: AtomicU64 = AtomicU64::new(0);
static LAST_SEQ: AtomicU64 = AtomicU64::new(0);
static LAST_HOUR: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(0);
static STOP: AtomicBool = AtomicBool::new(false);
static WORKER: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);
static JOB_POOL: Mutex<Vec<ResearchJob>> = Mutex::new(Vec::new());

/// Pilot refresh gate: the whole family shares stagger slot 2 (`H%4==2`);
/// branch starts and keyframes force a full refresh like every other table.
pub fn pilot_refresh_hour(hour: i32, force_full: bool) -> bool {
    let walk = replay::cadence::should_walk(PILOT[0], hour, force_full);
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(PILOT[1], hour, force_full)
    );
    debug_assert_eq!(
        walk,
        replay::cadence::should_walk(PILOT[2], hour, force_full)
    );
    walk
}

/// Monotonic frame sequence for FIFO order proofs. Gaps in the sequence are
/// dropped frames; order within the sequence is queue order.
pub fn next_seq() -> u64 {
    SEQ.fetch_add(1, Ordering::Relaxed)
}

/// Recycled raw-payload job. Cold calls allocate; steady state reuses jobs
/// with their capacities, so neither thread allocates per refresh.
pub fn acquire_job() -> ResearchJob {
    if let Ok(mut pool) = JOB_POOL.try_lock()
        && let Some(job) = pool.pop()
    {
        return job;
    }
    ResearchJob::new()
}

/// Return a payload job to the pool after clearing it. Growth decided here
/// runs on the calling (off-game-thread) side; the game thread never grows.
pub fn recycle_job(mut job: ResearchJob) {
    job.clear();
    if job.grow_for_next() {
        // Overflowed capacities double off the game thread until copies fit.
        job.grow();
    }
    if let Ok(mut pool) = JOB_POOL.try_lock()
        && pool.len() < JOB_POOL_KEEP + 1
    {
        pool.push(job);
    }
}

/// Enqueue a frame for decode (or ordered forwarding for skip hours).
/// Starts the worker lazily; never blocks. A full queue returns the job for
/// the existing gap path and counts the drop on the existing counters.
pub fn enqueue(shared: &Shared, job: DecodeJob) -> Option<DecodeJob> {
    ensure_worker();
    match shared.decode_queue.push(job) {
        Ok(()) => {
            JOBS.fetch_add(1, Ordering::Relaxed);
            None
        }
        Err(job) => {
            DROPS.fetch_add(1, Ordering::Relaxed);
            Some(job)
        }
    }
}

/// The single FIFO decode worker. Game frames arrive in hour order and hand
/// off to the spool ready queue in the same order; no reorder buffer exists.
fn decode_loop() {
    loop {
        if STOP.load(Ordering::Acquire) {
            break;
        }
        let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
        if shared_ptr.is_null() {
            std::thread::yield_now();
            continue;
        }
        let shared = unsafe { &*shared_ptr };
        match shared.decode_queue.pop() {
            Some(job) => process_job(shared, &job),
            None => std::thread::sleep(std::time::Duration::from_millis(1)),
        }
    }
}

fn ensure_worker() {
    if let Ok(mut slot) = WORKER.try_lock() {
        let running = slot.as_ref().is_some_and(|handle| !handle.is_finished());
        if !running {
            STOP.store(false, Ordering::Release);
            if let Ok(handle) = std::thread::Builder::new()
                .name("hoi4-decode".into())
                .spawn(decode_loop)
            {
                *slot = Some(handle);
            }
        }
    }
}

/// Decode one FIFO item and hand its pooled frame to the spool ready queue
/// in FIFO hour order. Skip items forward without touching typed rows.
pub fn process_job(shared: &Shared, job: &DecodeJob) {
    LAST_SEQ.store(job.seq, Ordering::Relaxed);
    LAST_HOUR.store(job.hour as u64, Ordering::Relaxed);
    if job.force_full {
        FULL_REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    if job.pilot_refresh
        && let Some(payload) = job.payload.as_ref()
    {
        let frame = unsafe { &mut *shared.frames[job.index].load(Ordering::Acquire) };
        let mut decode_ticks = [0; 3];
        frame.extended.research_pilot().decode_research_payload(
            shared.base,
            payload,
            &mut decode_ticks,
        );
        // The phased split mirrors the copy attribution in
        // `extended::governance` (technologies, slots, definitions).
        for (total, ticks) in DECODE_NS.iter().zip(decode_ticks) {
            if ticks != 0 {
                total.fetch_add(shared.ns(ticks as i64), Ordering::Relaxed);
            }
        }
        REFRESH_JOBS.fetch_add(1, Ordering::Relaxed);
    }
    // Pool invariant: this frame is owned by the worker, so it occupies no
    // ready slot and the push below always has room while the spool thread
    // lives. The worker may wait (only the game thread must never block).
    while shared.ready.push(job.index).is_err() {
        std::thread::yield_now();
    }
    shared.completed[job.index].store(job.game_elapsed.max(1), Ordering::Release);
}

/// Detach drain (runtime only, after callback quiescence): stop the worker,
/// join it, then synchronously decode any remaining FIFO items in order and
/// hand them to the spool ready queue before the existing spool drain runs.
/// The spool thread is still consuming, so handoffs make progress.
pub fn detach_drain() {
    STOP.store(true, Ordering::Release);
    if let Ok(mut slot) = WORKER.lock()
        && let Some(handle) = slot.take()
    {
        let _ = handle.join();
    }
    let shared_ptr = sampling::SHARED.load(Ordering::Acquire);
    if shared_ptr.is_null() {
        STOP.store(false, Ordering::Release);
        return;
    }
    let shared = unsafe { &*shared_ptr };
    while let Some(job) = shared.decode_queue.pop() {
        process_job(shared, &job);
    }
    STOP.store(false, Ordering::Release);
}

/// Additive calibration snapshot. Every counter is monotonic since attach.
pub fn calibration() -> Calibration {
    Calibration {
        jobs: JOBS.load(Ordering::Relaxed),
        refresh_jobs: REFRESH_JOBS.load(Ordering::Relaxed),
        full_refresh_jobs: FULL_REFRESH_JOBS.load(Ordering::Relaxed),
        drops: DROPS.load(Ordering::Relaxed),
        last_seq: LAST_SEQ.load(Ordering::Relaxed),
        last_hour: LAST_HOUR.load(Ordering::Relaxed) as i32,
        copy_ns: COPY_NS.each_ref().map(|cell| cell.load(Ordering::Relaxed)),
        decode_ns: DECODE_NS
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
        raw_bytes: RAW_BYTES
            .each_ref()
            .map(|cell| cell.load(Ordering::Relaxed)),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Calibration {
    pub jobs: u64,
    pub refresh_jobs: u64,
    pub full_refresh_jobs: u64,
    pub drops: u64,
    pub last_seq: u64,
    pub last_hour: i32,
    pub copy_ns: [u64; 3],
    pub decode_ns: [u64; 3],
    pub raw_bytes: [u64; 3],
}

impl Calibration {
    pub fn json(&self) -> Value {
        json!({
            "scope": "decode worker totals since attach; monotonic",
            "jobs": self.jobs,
            "refresh_jobs": self.refresh_jobs,
            "full_refresh_jobs": self.full_refresh_jobs,
            "drops": self.drops,
            "last_seq": self.last_seq,
            "last_hour": self.last_hour,
            "copy_ns": self.copy_ns.as_slice(),
            "decode_ns": self.decode_ns.as_slice(),
            "raw_bytes": self.raw_bytes.as_slice(),
            "tables": ["ResearchTechnologies", "ResearchSlots", "TechnologyDefinitions"],
        })
    }
}

/// Additive status merge for the runtime statuswriters. Existing keys survive.
pub fn calibration_json() -> Value {
    calibration().json()
}

pub(crate) fn add_copy_ns(table: usize, ns: u64) {
    COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_raw_bytes(table: usize, bytes: u64) {
    RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

/// Bounded raw FIFO type shared with sampling's pool (indices only).
pub type DecodeQueue = ArrayQueue<DecodeJob>;

pub fn new_queue() -> DecodeQueue {
    ArrayQueue::new(BUFFERS)
}

#[cfg(test)]
pub(crate) fn reset_calibration_for_test() {
    for cell in COPY_NS
        .iter()
        .chain(DECODE_NS.iter())
        .chain(RAW_BYTES.iter())
    {
        cell.store(0, Ordering::Relaxed);
    }
    JOBS.store(0, Ordering::Relaxed);
    REFRESH_JOBS.store(0, Ordering::Relaxed);
    FULL_REFRESH_JOBS.store(0, Ordering::Relaxed);
    DROPS.store(0, Ordering::Relaxed);
    LAST_SEQ.store(0, Ordering::Relaxed);
    LAST_HOUR.store(0, Ordering::Relaxed);
    SEQ.store(0, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extended::governance::research_test_kit;
    use std::sync::atomic::Ordering;

    /// Calibration globals are shared across tests in this binary; the tests
    /// below that reset or advance them hold this lock so assertions stay
    /// deterministic under parallel execution.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn pilot_gate_matches_stagger_slot_two_and_force_full() {
        // Slot 2 refreshes exactly on H%4==2; branch starts and keyframes
        // force a full refresh regardless of hour.
        for hour in -8..8i32 {
            let refresh = hour.rem_euclid(4) == 2;
            assert_eq!(pilot_refresh_hour(hour, false), refresh, "hour {hour}");
            assert!(pilot_refresh_hour(hour, true), "forced hour {hour}");
        }
    }

    #[test]
    fn fifo_hands_off_every_frame_in_production_order_without_reorder() {
        let shared = Shared::new(0, 0, 1);
        let mut indices = Vec::new();
        for _ in 0..4 {
            indices.push(shared.free.pop().expect("pooled buffer"));
        }
        // Enqueue out-of-seq labels; handoff order must still be FIFO.
        for (order, index) in indices.iter().enumerate() {
            shared.completed[*index].store(0, Ordering::Relaxed);
            let job = DecodeJob {
                index: *index,
                seq: 100 + order as u64,
                hour: 200 + order as i32,
                pilot_refresh: false,
                force_full: false,
                game_elapsed: 7,
                payload: None,
            };
            shared.decode_queue.push(job).unwrap();
        }
        let mut handed = Vec::new();
        while let Some(job) = shared.decode_queue.pop() {
            process_job(&shared, &job);
            handed.push(job.index);
        }
        assert_eq!(handed, indices);
        for index in indices {
            assert_eq!(shared.ready.pop(), Some(index));
            assert_eq!(shared.completed[index].load(Ordering::Acquire), 7);
        }
        assert!(shared.ready.pop().is_none());
    }

    #[test]
    fn queue_full_returns_the_job_for_the_existing_gap_path() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let shared = Shared::new(0, 0, 1);
        for seq in 0..BUFFERS {
            let job = DecodeJob {
                index: 0,
                seq: seq as u64,
                hour: 0,
                pilot_refresh: false,
                force_full: false,
                game_elapsed: 1,
                payload: None,
            };
            assert!(shared.decode_queue.push(job).is_ok());
        }
        let before = DROPS.load(Ordering::Relaxed);
        let job = DecodeJob {
            index: 0,
            seq: u64::MAX,
            hour: 0,
            pilot_refresh: false,
            force_full: false,
            game_elapsed: 1,
            payload: None,
        };
        assert!(enqueue(&shared, job).is_some());
        assert_eq!(DROPS.load(Ordering::Relaxed), before + 1);
    }

    #[test]
    fn calibration_counters_are_monotonic_and_additive_per_table() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        add_copy_ns(0, 11);
        add_copy_ns(0, 4);
        add_copy_ns(1, 7);
        add_raw_bytes(2, 100);
        DECODE_NS[2].fetch_add(9, Ordering::Relaxed);
        let first = calibration();
        assert_eq!(first.copy_ns, [15, 7, 0]);
        assert_eq!(first.raw_bytes, [0, 0, 100]);
        assert_eq!(first.decode_ns, [0, 0, 9]);
        add_copy_ns(0, 1);
        let second = calibration();
        assert!(second.copy_ns[0] > first.copy_ns[0]);
        assert_eq!(second.copy_ns[1], first.copy_ns[1]);
        let json = second.json();
        assert_eq!(json["copy_ns"][0], 16);
        assert_eq!(json["tables"][1], "ResearchSlots");
        assert!(json["scope"].as_str().unwrap().contains("monotonic"));
    }

    #[test]
    fn worker_decodes_a_refresh_payload_into_the_same_typed_rows() {
        let _guard = TEST_LOCK.lock().unwrap();
        reset_calibration_for_test();
        let (bytes, layout) = research_test_kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: layout.base,
            bytes: &bytes,
        };
        let shared = Shared::new(layout.base, 0, 1);
        let index = shared.free.pop().expect("pooled buffer");
        let direct = research_test_kit::direct(&memory, &layout, 8, 4, 8, 32, 64);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        let (report, first) =
            research_test_kit::split(&memory, &layout, frame.extended.research_pilot(), true);
        assert!(report.raw_ok);
        shared.completed[index].store(0, Ordering::Relaxed);
        let job = DecodeJob {
            index,
            seq: next_seq(),
            hour: 2,
            pilot_refresh: true,
            force_full: false,
            game_elapsed: 13,
            payload: Some(first),
        };
        process_job(&shared, &job);
        assert_eq!(shared.ready.pop(), Some(index));
        assert_eq!(shared.completed[index].load(Ordering::Acquire), 13);
        let frame = unsafe { &mut *shared.frames[index].load(Ordering::Acquire) };
        research_test_kit::assert_matches_direct(frame.extended.research_pilot(), &direct);
    }
}
