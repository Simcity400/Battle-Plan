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
    pub(crate) const ALL: [Self; 18] = [
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
            Self::Write => {
                "context, clock witness and frame records: framing, CRC, file writes and segment barriers"
            }
            Self::Commit => "baseline commit, branch record and per-hour output statistics",
            Self::Observers => {
                "between-tick observer and clock-capture storage preparation each pass"
            }
            Self::Sizing => {
                "live sizing reads and the capture-buffer growth they request; on the spool thread, the hand-over of the sizing thread's observations (with no sizing thread, the reads themselves)"
            }
            Self::Retire => {
                "freeing the baselines, rows and inputs encoder commits handed back; on the sizing thread, or on the spool thread without one"
            }
            Self::Flush => "periodic durability checks; the sync itself runs on its worker",
            Self::Status => "status snapshot construction, hand-over and refresh requests",
            Self::Wait => "idle wait for a ready buffer, a parked callback or the next deadline",
        }
    }
}

/// Cumulative calls, QPC ticks and thread cycles of each phase.
pub(crate) struct Totals {
    phases: [[AtomicU64; 3]; Phase::ALL.len()],
    packs: [AtomicU64; crate::extended::PACK_NAMES.len()],
}

impl Totals {
    pub(crate) const fn new() -> Self {
        Self {
            phases: [const { [const { AtomicU64::new(0) }; 3] }; Phase::ALL.len()],
            packs: [const { AtomicU64::new(0) }; crate::extended::PACK_NAMES.len()],
        }
    }

    /// Start a span on the calling thread; finish or drop it to record.
    pub(crate) fn span(&self, phase: Phase) -> Span<'_> {
        Span {
            totals: self,
            phase,
            qpc: crate::sampling::qpc(),
            cycles: thread_cycles(),
        }
    }

    fn record(&self, phase: Phase, ticks: i64, cycles: u64) {
        let slot = &self.phases[phase as usize];
        slot[0].fetch_add(1, Ordering::Relaxed);
        slot[1].fetch_add(ticks.max(0) as u64, Ordering::Relaxed);
        slot[2].fetch_add(cycles, Ordering::Relaxed);
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
                status!({
                    "name": phase.name(),
                    "calls": calls,
                    "wall_ns": ns(ticks),
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

const SCOPE: &str = "cumulative spool-thread work since attach by non-overlapping phase: calls, wall_ns from QPC and the thread's own cycles from QueryThreadCycleTime (cycle_ns converts them with hook_timing's tsc_hz). The phases partition the spool loop, so their cycles sum to the spool thread's measured CPU apart from thread start-up; wall exceeds cycles where the thread was preempted or waited. pack_cycles splits convert_packs by pack";

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
}

impl<'a> Span<'a> {
    pub(crate) fn finish(self) {}

    /// Record this phase and start the next one at the same instant.
    pub(crate) fn switch(mut self, phase: Phase) -> Span<'a> {
        let (qpc, cycles) = (crate::sampling::qpc(), thread_cycles());
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
        self.totals.record(
            self.phase,
            crate::sampling::qpc() - self.qpc,
            thread_cycles().saturating_sub(self.cycles),
        );
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
