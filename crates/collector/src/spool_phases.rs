//! Spool-thread work by phase: wall time (QPC) and the thread's own CPU cycles
//! (QueryThreadCycleTime), cumulative since attach. The phases partition the
//! spool loop: spans never nest, so their CPU sums to the measured spool work
//! and a status difference attributes the thread's cost per game hour.
//! Recording is allocation-free; only the spool thread records the global
//! totals, so relaxed atomics suffice and status reads them on that thread.

use crate::status::{sequence, status};
use std::sync::atomic::{AtomicU64, Ordering};
use windows_sys::Win32::{Foundation::HANDLE, System::Threading::GetCurrentThread};

unsafe extern "system" {
    fn QueryThreadCycleTime(thread: HANDLE, cycles: *mut u64) -> i32;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Receive,
    Observe,
    ConvertCore,
    ConvertPacks,
    HelperWait,
    Merge,
    Statistics,
    Recycle,
    Compare,
    Encode,
    Columnar,
    Write,
    Commit,
    Observers,
    Sizing,
    Retire,
    Flush,
    Status,
    Wait,
}

impl Phase {
    pub(crate) const ALL: [Self; 19] = [
        Self::Receive,
        Self::Observe,
        Self::ConvertCore,
        Self::ConvertPacks,
        Self::HelperWait,
        Self::Merge,
        Self::Statistics,
        Self::Recycle,
        Self::Compare,
        Self::Encode,
        Self::Columnar,
        Self::Write,
        Self::Commit,
        Self::Observers,
        Self::Sizing,
        Self::Retire,
        Self::Flush,
        Self::Status,
        Self::Wait,
    ];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Receive => "receive",
            Self::Observe => "observe",
            Self::ConvertCore => "convert_core",
            Self::ConvertPacks => "convert_packs",
            Self::HelperWait => "helper_wait",
            Self::Merge => "merge",
            Self::Statistics => "statistics",
            Self::Recycle => "recycle",
            Self::Compare => "compare",
            Self::Encode => "encode",
            Self::Columnar => "columnar",
            Self::Write => "write",
            Self::Commit => "commit",
            Self::Observers => "observers",
            Self::Sizing => "sizing",
            Self::Retire => "retire",
            Self::Flush => "flush",
            Self::Status => "status",
            Self::Wait => "wait",
        }
    }

    const fn scope(self) -> &'static str {
        match self {
            Self::Receive => {
                "ready buffer handoff: completion wait, roster verification, campaign preflight and queue-loss witnesses"
            }
            Self::Observe => {
                "per-hour diagnostics observed from the capture before conversion, campaign and branch selection"
            }
            Self::ConvertCore => {
                "core, map and border-conflict rows, including their native projections against encoder baselines; with the helper, the map rows are in helper_phases"
            }
            Self::ConvertPacks => {
                "extended pack conversion; with the helper, its @helper packs are in helper_phases; per-pack cycles on either thread in pack_cycles"
            }
            Self::HelperWait => "waiting for the conversion helper after this thread's share",
            Self::Merge => {
                "sliced merges and SliceReads, ledgers and line totals, staggered carry and fixed-layout refusals"
            }
            Self::Statistics => "owned success statistics and witnesses retained past buffer reuse",
            Self::Recycle => {
                "retry and forget requests, capture-buffer growth after the hour and its return to the game thread"
            }
            Self::Compare => {
                "comparison of complete hourly tables with the encoder baselines before encoding; with the helper, its share is in helper_phases"
            }
            Self::Encode => {
                "frame encoding against the campaign baselines; with the helpers, their share of the larger tables is in helper_phases"
            }
            Self::Columnar => {
                "the columnar recording's hour: its tables prepared and committed, with the helpers' share in helper_phases"
            }
            Self::Write => {
                "context, clock witness and frame records: framing, CRC, speculative baseline staging and owned writer-queue admission; waits only for byte backpressure or rare segment barriers"
            }
            Self::Commit => "admitted per-hour state and completion-ledger handoff",
            Self::Observers => {
                "between-tick observer and clock-capture storage preparation each pass"
            }
            Self::Sizing => {
                "live sizing reads and the capture-buffer growth they request; on the spool thread, the hand-over of the sizing thread's observations (with no sizing thread, the reads themselves)"
            }
            Self::Retire => {
                "freeing the baselines, rows and inputs encoder commits handed back; on the sizing thread, or on the spool thread without one"
            }
            Self::Flush => {
                "writer completions: recorded-hour evidence, publication receipts and deferred failures; file writes and sync run on the writer"
            }
            Self::Status => "status snapshot construction, hand-over and refresh requests",
            Self::Wait => "idle wait for a ready buffer, a parked callback or the next deadline",
        }
    }
}

/// Cumulative calls, QPC ticks and thread cycles of each phase.
pub(crate) struct Totals {
    phases: [[AtomicU64; 4]; Phase::ALL.len()],
    packs: [AtomicU64; crate::extended::PACK_NAMES.len()],
    scheduler: [[AtomicU64; 3]; Phase::ALL.len()],
}

impl Totals {
    pub(crate) const fn new() -> Self {
        Self {
            phases: [const { [const { AtomicU64::new(0) }; 4] }; Phase::ALL.len()],
            packs: [const { AtomicU64::new(0) }; crate::extended::PACK_NAMES.len()],
            scheduler: [const { [const { AtomicU64::new(0) }; 3] }; Phase::ALL.len()],
        }
    }

    /// Start a span on the calling thread; finish or drop it to record.
    pub(crate) fn span(&self, phase: Phase) -> Span<'_> {
        if phase != Phase::Wait {
            crate::tick_gate::wait();
        }
        self.span_now(phase)
    }

    /// [`Self::span`] started at once, also in the game's tick.
    pub(crate) fn span_now(&self, phase: Phase) -> Span<'_> {
        Span {
            totals: self,
            phase,
            qpc: crate::sampling::qpc(),
            cycles: thread_cycles(),
            scheduler: crate::thread_diagnostics::Snapshot::now(false),
        }
    }

    fn record(&self, phase: Phase, ticks: i64, cycles: u64) {
        let slot = &self.phases[phase as usize];
        slot[0].fetch_add(1, Ordering::Relaxed);
        slot[1].fetch_add(ticks.max(0) as u64, Ordering::Relaxed);
        slot[2].fetch_add(cycles, Ordering::Relaxed);
        slot[3].fetch_max(ticks.max(0) as u64, Ordering::Relaxed);
    }

    fn record_scheduler(
        &self,
        phase: Phase,
        begin: crate::thread_diagnostics::Snapshot,
        end: crate::thread_diagnostics::Snapshot,
    ) {
        if let Some(switches) = begin.switches_to(end) {
            let slot = &self.scheduler[phase as usize];
            slot[0].fetch_add(1, Ordering::Relaxed);
            slot[1].fetch_add(u64::from(switches), Ordering::Relaxed);
            if let Some(reasons) = end.wait_reasons {
                slot[2].fetch_or(reasons, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn record_pack(&self, pack: usize, cycles: u64) {
        if let Some(slot) = self.packs.get(pack) {
            slot.fetch_add(cycles, Ordering::Relaxed);
        }
    }

    /// (calls, QPC ticks, cycles) of one phase.
    pub(crate) fn phase(&self, phase: Phase) -> (u64, u64, u64) {
        let slot = &self.phases[phase as usize];
        (
            slot[0].load(Ordering::Relaxed),
            slot[1].load(Ordering::Relaxed),
            slot[2].load(Ordering::Relaxed),
        )
    }

    pub(crate) fn status(&self, qpc_frequency: u64) -> impl serde::Serialize + '_ {
        let hz = crate::hook_timing::tsc_hz();
        let ns = move |ticks: u64| {
            (u128::from(ticks) * 1_000_000_000 / u128::from(qpc_frequency.max(1))) as u64
        };
        status!({
            "scope": SCOPE,
            "tsc_hz": hz,
            "phases": sequence(Phase::ALL.into_iter().map(move |phase| {
                let (calls, ticks, cycles) = self.phase(phase);
                let scheduler = &self.scheduler[phase as usize];
                let covered = scheduler[0].load(Ordering::Relaxed);
                status!({
                    "scheduler_samples": covered,
                    "context_switches": (covered != 0).then(|| scheduler[1].load(Ordering::Relaxed)),
                    "wait_reason_bitmap": (covered != 0).then(|| scheduler[2].load(Ordering::Relaxed)),
                    "name": phase.name(),
                    "calls": calls,
                    "wall_ns": ns(ticks),
                    "max_wall_ns": ns(self.phases[phase as usize][3].load(Ordering::Relaxed)),
                    "cycles": cycles,
                    "cycle_ns": crate::hook_timing::ticks_to_ns(cycles, hz),
                    "scope": phase.scope(),
                })
            })),
            "pack_cycles": sequence(crate::extended::PACK_NAMES.iter().zip(&self.packs).map(move |(name, cycles)| {
                let cycles = cycles.load(Ordering::Relaxed);
                status!({
                    "name": name,
                    "cycles": cycles,
                    "cycle_ns": crate::hook_timing::ticks_to_ns(cycles, hz),
                })
            })),
        })
    }
}

const SCOPE: &str = "cumulative spool-thread work since attach by non-overlapping phase: calls, wall_ns from QPC and the thread's own cycles from QueryThreadCycleTime (cycle_ns converts them with hook_timing's tsc_hz). The phases partition the spool loop, so their cycles sum to the spool thread's measured CPU apart from thread start-up; wall exceeds cycles where the thread was preempted or waited. pack_cycles splits convert_packs by pack; scheduler_samples counts spans with successful dispatch-counter boundaries, context_switches sums their deltas and wait_reason_bitmap unions their OS reason masks; these are counts/masks, not off-CPU durations, with query limits in thread_diagnostics";

/// The spool thread's totals.
pub(crate) static SPOOL: Totals = Totals::new();
/// The conversion helpers' totals: map rows as convert_core, their packs as
/// convert_packs and their share of the comparison, measured on the helpers.
pub(crate) static HELPER: Totals = Totals::new();
/// The sizing thread's totals: sizing reads, observer preparation, freeing
/// what encoder commits handed back, and its idle wait.
pub(crate) static SIZING: Totals = Totals::new();

#[must_use = "a span records when finished or dropped"]
pub(crate) struct Span<'a> {
    totals: &'a Totals,
    phase: Phase,
    qpc: i64,
    cycles: u64,
    scheduler: crate::thread_diagnostics::Snapshot,
}

impl<'a> Span<'a> {
    pub(crate) fn finish(self) {}

    /// Record this phase and start the next one at the same instant.
    pub(crate) fn switch(mut self, phase: Phase) -> Span<'a> {
        if phase != Phase::Wait {
            crate::tick_gate::wait();
        }
        let (qpc, cycles) = (crate::sampling::qpc(), thread_cycles());
        let scheduler = crate::thread_diagnostics::Snapshot::now(false);
        self.totals
            .record_scheduler(self.phase, self.scheduler, scheduler);
        if self.phase != Phase::Wait {
            crate::thread_diagnostics::record_service(
                self.scheduler,
                scheduler,
                (self.cycles != 0 && cycles != 0).then(|| cycles.saturating_sub(self.cycles)),
                qpc - self.qpc,
            );
        }
        self.scheduler = scheduler;
        self.totals.record(
            self.phase,
            qpc - self.qpc,
            cycles.saturating_sub(self.cycles),
        );
        self.phase = phase;
        self.qpc = qpc;
        self.cycles = cycles;
        self
    }
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        let scheduler = crate::thread_diagnostics::Snapshot::now(false);
        let (ticks, end_cycles) = (crate::sampling::qpc() - self.qpc, thread_cycles());
        let cycles = end_cycles.saturating_sub(self.cycles);
        self.totals
            .record_scheduler(self.phase, self.scheduler, scheduler);
        if self.phase != Phase::Wait {
            crate::thread_diagnostics::record_service(
                self.scheduler,
                scheduler,
                (self.cycles != 0 && end_cycles != 0).then_some(cycles),
                ticks,
            );
        }
        self.totals.record(self.phase, ticks, cycles);
    }
}

/// The calling thread's cycles; zero if the query fails, which records no CPU.
pub(crate) fn thread_cycles() -> u64 {
    let mut cycles = 0;
    // SAFETY: the pseudo-handle names only the calling thread and needs no close.
    if unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut cycles) } == 0 {
        return 0;
    }
    cycles
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scheduler_coverage_preserves_missing_values_and_phase_maxima() {
        let totals = Totals::new();
        totals.record(Phase::Status, 20, 3);
        totals.record(Phase::Status, 10, 2);
        let before = serde_json::to_value(totals.status(1_000_000_000)).unwrap();
        let row = &before["phases"][Phase::Status as usize];
        assert_eq!(row["max_wall_ns"], 20);
        assert_eq!(row["scheduler_samples"], 0);
        assert!(row["context_switches"].is_null());
        let start = crate::thread_diagnostics::Snapshot {
            context_switches: Some(5),
            ..Default::default()
        };
        let end = crate::thread_diagnostics::Snapshot {
            context_switches: Some(8),
            wait_reasons: Some(64),
            ..Default::default()
        };
        totals.record_scheduler(Phase::Status, start, end);
        let after = serde_json::to_value(totals.status(1_000_000_000)).unwrap();
        let row = &after["phases"][Phase::Status as usize];
        assert_eq!(row["scheduler_samples"], 1);
        assert_eq!(row["context_switches"], 3);
        assert_eq!(row["wait_reason_bitmap"], 64);
    }

    #[test]
    fn spans_partition_work_by_phase_and_count_this_threads_cycles() {
        let totals = Totals::new();
        let span = totals.span(Phase::Encode);
        let mut value = 0_u64;
        for index in 0..2_000_000_u64 {
            value = std::hint::black_box(value.wrapping_mul(31).wrapping_add(index));
        }
        // The next phase starts where this one ends.
        let wait = span.switch(Phase::Wait);
        std::thread::sleep(std::time::Duration::from_millis(20));
        drop(wait);
        totals.record_pack(1, 7);
        let (calls, ticks, cycles) = totals.phase(Phase::Encode);
        assert_eq!(calls, 1);
        assert!(ticks > 0 && cycles > 0);
        let (calls, wait_ticks, wait_cycles) = totals.phase(Phase::Wait);
        assert_eq!(calls, 1);
        // Sleeping takes wall time but almost none of the thread's cycles.
        assert!(wait_cycles < cycles, "{wait_cycles} >= {cycles}");
        assert!(wait_ticks > 0);
        for phase in Phase::ALL {
            if !matches!(phase, Phase::Encode | Phase::Wait) {
                assert_eq!(totals.phase(phase), (0, 0, 0));
            }
        }
        let status = serde_json::to_value(totals.status(10_000_000)).unwrap();
        let phases = status["phases"].as_array().unwrap();
        assert_eq!(phases.len(), Phase::ALL.len());
        assert_eq!(phases[Phase::Encode as usize]["name"], "encode");
        assert_eq!(phases[Phase::Encode as usize]["calls"], 1);
        assert_eq!(phases[Phase::Encode as usize]["cycles"], cycles);
        assert!(phases[Phase::Wait as usize]["wall_ns"].as_u64().unwrap() >= 20_000_000);
        assert_eq!(status["pack_cycles"][1]["cycles"], 7);
        assert_eq!(
            status["pack_cycles"][1]["name"],
            crate::extended::PACK_NAMES[1]
        );
    }

    #[test]
    fn recording_a_span_allocates_nothing() {
        let totals = Totals::new();
        crate::test_support::with_allocations_denied(|| {
            totals.span(Phase::Sizing).finish();
            totals.record_pack(0, 1);
        });
        assert_eq!(totals.phase(Phase::Sizing).0, 1);
    }
}
