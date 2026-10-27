//! Bounded status-only timing of the FIFO path that returns stores and frames.
use super::{DecodeJob, Shared};
use crate::status::{sequence, status};
use std::sync::{
    Mutex, TryLockError,
    atomic::{AtomicU64, Ordering},
};

const CAPACITY: usize = 64;
const PHASES: [&str; 4] = [
    "retirement",
    "presentation",
    "remaining_decode",
    "release_and_handoff",
];
static RECORDS: Mutex<Records> = Mutex::new(Records::new());
static COMPLETED: AtomicU64 = AtomicU64::new(0);
static RETENTION_BUSY: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Default)]
struct Stamp {
    qpc: i64,
    cycles: Option<u64>,
    cpu_ns: Option<u64>,
    scheduler: crate::thread_diagnostics::Snapshot,
}
impl Stamp {
    fn now() -> Self {
        let qpc = crate::sampling::qpc();
        let cycles = crate::spool_phases::thread_cycles();
        Self {
            qpc,
            cycles: (cycles != 0).then_some(cycles),
            cpu_ns: crate::runtime::thread_cpu_ns().ok(),
            scheduler: crate::thread_diagnostics::Snapshot::now(true),
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    hour: i32,
    seq: u64,
    index: usize,
    free_at_start: usize,
    frequency: u64,
    queued_qpc: Option<i64>,
    store_returned: Option<bool>,
    marks: [Option<Stamp>; 5],
}
impl Sample {
    fn ticks(&self) -> i64 {
        self.marks[4]
            .unwrap()
            .qpc
            .saturating_sub(self.queued_qpc.unwrap_or(self.marks[0].unwrap().qpc))
            .max(0)
    }

    fn status(self) -> impl serde::Serialize {
        let ns = move |ticks: i64| {
            (ticks.max(0) as u128 * 1_000_000_000 / u128::from(self.frequency.max(1))) as u64
        };
        let begin = self.marks[0].unwrap();
        let end = self.marks[4].unwrap();
        status!({
            "hour": self.hour,
            "seq": self.seq,
            "index": self.index,
            "free_at_start": self.free_at_start,
            "qpc_frequency": self.frequency,
            "queued_qpc": self.queued_qpc,
            "begin_qpc": begin.qpc,
            "presentation_end_qpc": self.marks[2].map(|stamp| stamp.qpc),
            "store_returned": self.store_returned,
            "presentation_returned_qpc": self.store_returned.filter(|returned| *returned).and(self.marks[2]).map(|stamp| stamp.qpc),
            "end_qpc": end.qpc,
            "queue_ns": self.queued_qpc.and_then(|queued| begin.qpc.checked_sub(queued)).filter(|ticks| *ticks >= 0).map(ns),
            "service_ns": ns(end.qpc.saturating_sub(begin.qpc)),
            "cpu_ns": begin.cpu_ns.zip(end.cpu_ns).and_then(|(begin, end)| end.checked_sub(begin)),
            "context_switches": begin.scheduler.switches_to(end.scheduler),
            "process_faults": begin.scheduler.faults_to(end.scheduler),
            "priority_at_begin": begin.scheduler.priority,
            "priority_at_end": end.scheduler.priority,
            "cycles": begin.cycles.zip(end.cycles).and_then(|(begin, end)| end.checked_sub(begin)),
            "phases": sequence(PHASES.into_iter().enumerate().map(move |(index, name)| {
                let stamps = self.marks[index].zip(self.marks[index + 1]);
                status!({
                    "name": name,
                    "context_switches": stamps.and_then(|(begin, end)| begin.scheduler.switches_to(end.scheduler)),
                    "wait_reason_bitmap": stamps.and_then(|(begin, end)| begin.scheduler.switches_to(end.scheduler).and(end.scheduler.wait_reasons)),
                    "process_faults": stamps.and_then(|(begin, end)| begin.scheduler.faults_to(end.scheduler)),
                    "priority_at_begin": stamps.and_then(|(begin, _)| begin.scheduler.priority),
                    "priority_at_end": stamps.and_then(|(_, end)| end.scheduler.priority),
                    "wall_ns": stamps.and_then(|(begin, end)| end.qpc.checked_sub(begin.qpc)).filter(|ticks| *ticks >= 0).map(ns),
                    "cpu_ns": stamps.and_then(|(begin, end)| begin.cpu_ns.zip(end.cpu_ns)).and_then(|(begin, end)| end.checked_sub(begin)),
                    "cycles": stamps.and_then(|(begin, end)| begin.cycles.zip(end.cycles)).and_then(|(begin, end)| end.checked_sub(begin)),
                })
            })),
        })
    }
}

#[derive(Clone, Copy)]
struct Records {
    slowest: [Option<Sample>; CAPACITY],
    len: usize,
}
impl Records {
    const fn new() -> Self {
        Self {
            slowest: [None; CAPACITY],
            len: 0,
        }
    }
    fn insert(&mut self, sample: Sample) {
        let index = self.slowest[..self.len]
            .partition_point(|entry| entry.unwrap().ticks() >= sample.ticks());
        if index == CAPACITY {
            return;
        }
        let len = (self.len + 1).min(CAPACITY);
        self.slowest.copy_within(index..len - 1, index + 1);
        self.slowest[index] = Some(sample);
        self.len = len;
    }
}

pub(super) struct Service(Sample);
impl Service {
    pub(super) fn start(shared: &Shared, job: &DecodeJob) -> Self {
        Self(Sample {
            hour: job.hour,
            seq: job.seq,
            index: job.index,
            free_at_start: shared.free.len(),
            frequency: shared.frequency,
            queued_qpc: (job.queued_qpc > 0).then_some(job.queued_qpc),
            store_returned: None,
            marks: [Some(Stamp::now()), None, None, None, None],
        })
    }
    pub(super) fn presentation_begin(&mut self) {
        self.0.marks[1] = Some(Stamp::now());
    }
    pub(super) fn presentation_returned(&mut self, returned: bool) {
        self.0.marks[2] = Some(Stamp::now());
        self.0.store_returned = Some(returned);
    }
    pub(super) fn decoded(&mut self) {
        self.0.marks[3] = Some(Stamp::now());
    }
    pub(super) fn finish(mut self) {
        self.0.marks[4] = Some(Stamp::now());
        for pair in self.0.marks.windows(2) {
            if let Some((begin, end)) = pair[0].zip(pair[1]) {
                crate::thread_diagnostics::record_service(
                    begin.scheduler,
                    end.scheduler,
                    begin
                        .cycles
                        .zip(end.cycles)
                        .and_then(|(begin, end)| end.checked_sub(begin)),
                    end.qpc - begin.qpc,
                );
            }
        }
        COMPLETED.fetch_add(1, Ordering::Relaxed);
        let mut records = match RECORDS.try_lock() {
            Ok(records) => records,
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
            Err(TryLockError::WouldBlock) => {
                RETENTION_BUSY.fetch_add(1, Ordering::Relaxed);
                return;
            }
        };
        records.insert(self.0);
    }
}

pub(super) fn status() -> impl serde::Serialize {
    let records = *RECORDS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    status!({
        "capacity": CAPACITY,
        "completed": COMPLETED.load(Ordering::Relaxed),
        "retention_busy": RETENTION_BUSY.load(Ordering::Relaxed),
        "slowest": sequence(records.slowest.into_iter().flatten().map(Sample::status)),
        "scope": "status-only completed FIFO jobs, retained by enqueue-to-ready wall with earlier ties first; no allocation or blocking on decode retention; queue_ns includes the existing idle poll and scheduling before dequeue; retirement includes frame setup and missing-payload verdicts; presentation includes the army store return when present, before unrelated decode; store_returned reports whether a loan was returned, and only then supplies presentation_returned_qpc; CPU is GetThreadTimes user+kernel at scheduler-tick resolution, cycles are QueryThreadCycleTime, neither identifies the reason for off-CPU time; context switches, per-phase wait-reason masks, priority offsets and process page-fault deltas supplement them as described by thread_diagnostics; unavailable queries or skipped phases are null; measurements run in plain release too and never enter recorded rows",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(seq: u64, queued: Option<i64>, marks: [i64; 5]) -> Sample {
        Sample {
            hour: 123,
            seq,
            index: 0,
            free_at_start: 2,
            frequency: 1_000_000_000,
            queued_qpc: queued,
            store_returned: Some(true),
            marks: marks.map(|qpc| {
                Some(Stamp {
                    qpc,
                    cycles: Some(qpc as u64 * 2),
                    cpu_ns: Some(qpc as u64),
                    scheduler: crate::thread_diagnostics::Snapshot {
                        context_switches: Some(qpc as u32),
                        process_faults: Some(qpc as u32 * 2),
                        wait_reasons: Some(4),
                        priority: Some(1),
                    },
                })
            }),
        }
    }

    #[test]
    fn queue_store_return_and_disjoint_service_phases_keep_their_boundaries() {
        let row = serde_json::to_value(sample(1, Some(10), [20, 30, 40, 70, 90]).status()).unwrap();
        assert_eq!(row["queue_ns"], 10);
        assert_eq!(row["service_ns"], 70);
        assert_eq!(row["context_switches"], 70);
        assert_eq!(row["process_faults"], 140);
        assert_eq!(row["phases"][1]["context_switches"], 10);
        assert_eq!(row["phases"][1]["wait_reason_bitmap"], 4);
        assert_eq!(row["presentation_returned_qpc"], 40);
        let phases = row["phases"].as_array().unwrap();
        assert_eq!(
            phases
                .iter()
                .map(|phase| phase["wall_ns"].as_u64().unwrap())
                .sum::<u64>(),
            70
        );
        assert_eq!(
            phases
                .iter()
                .map(|phase| phase["cpu_ns"].as_u64().unwrap())
                .sum::<u64>(),
            70
        );
        assert_eq!(
            phases
                .iter()
                .map(|phase| phase["cycles"].as_u64().unwrap())
                .sum::<u64>(),
            140
        );
        assert_eq!(phases[1]["name"], "presentation");
        assert_eq!(phases[1]["wall_ns"], 10);
    }

    #[test]
    fn missing_queue_queries_and_skipped_phases_stay_null() {
        let mut missing = sample(1, None, [20, 30, 40, 70, 90]);
        missing.marks[0].as_mut().unwrap().cycles = None;
        missing.marks[0].as_mut().unwrap().cpu_ns = None;
        missing.marks[0].as_mut().unwrap().scheduler = Default::default();
        missing.marks[1] = None;
        missing.marks[2] = None;
        missing.store_returned = None;
        let row = serde_json::to_value(missing.status()).unwrap();
        for key in [
            "queue_ns",
            "cycles",
            "cpu_ns",
            "presentation_returned_qpc",
            "context_switches",
            "process_faults",
            "priority_at_begin",
        ] {
            assert!(row[key].is_null(), "{key}");
        }
        assert!(row["phases"][1]["wall_ns"].is_null());
        assert!(row["phases"][1]["cpu_ns"].is_null());
        assert!(row["store_returned"].is_null());
        let mut without_loan = missing;
        without_loan.marks[2] = Some(Stamp::default());
        without_loan.store_returned = Some(false);
        let row = serde_json::to_value(without_loan.status()).unwrap();
        assert_eq!(row["store_returned"], false);
        assert!(row["presentation_returned_qpc"].is_null());
    }

    #[test]
    fn bounded_retention_accounts_for_queue_time_without_allocating_and_keeps_earlier_ties() {
        let mut records = Records::new();
        crate::test_support::with_allocations_denied(|| {
            for seq in 0..CAPACITY as u64 + 2 {
                records.insert(sample(seq, Some(10), [20, 30, 40, 70, 90]));
            }
            records.insert(sample(1000, Some(1), [20, 30, 40, 70, 90]));
        });
        assert_eq!(records.len, CAPACITY);
        assert_eq!(records.slowest[0].unwrap().seq, 1000);
        assert_eq!(records.slowest[1].unwrap().seq, 0);
        assert_eq!(
            records.slowest[CAPACITY - 1].unwrap().seq,
            CAPACITY as u64 - 2
        );
    }
}
