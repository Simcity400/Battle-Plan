//! The points where the spool, helper and sizing threads start a step of their loop: each step
//! but the idle wait waits out the game's hourly tick first.

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

const PHASES: [Phase; Phase::Wait as usize + 1] = [
    Phase::Receive,
    Phase::Observe,
    Phase::ConvertCore,
    Phase::ConvertPacks,
    Phase::HelperWait,
    Phase::Merge,
    Phase::Statistics,
    Phase::Recycle,
    Phase::Compare,
    Phase::Encode,
    Phase::Columnar,
    Phase::Write,
    Phase::Commit,
    Phase::Observers,
    Phase::Sizing,
    Phase::Retire,
    Phase::Flush,
    Phase::Status,
    Phase::Wait,
];

const STEPS: usize = Phase::Wait as usize + 1;

pub(crate) struct Steps {
    /// The step last started, for naming what a waiting hour's frames wait on.
    current: std::sync::atomic::AtomicU8,
    /// Cumulative wall per step, and spent waiting out the game's tick before a step, in QPC
    /// ticks: the spool diffs them across a frame to name a slow one.
    walls: [std::sync::atomic::AtomicU64; STEPS],
    gated: std::sync::atomic::AtomicU64,
    /// Steps started, so tests can see which thread took a step.
    #[cfg(test)]
    started: [std::sync::atomic::AtomicU64; Phase::Wait as usize + 1],
    #[cfg(feature = "phase_cpu")]
    cycles: [std::sync::atomic::AtomicU64; Phase::Wait as usize + 1],
    /// Steps whose cycle query failed, so their cycles are absent from `cycles`.
    #[cfg(feature = "phase_cpu")]
    cycle_failures: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "phase_cpu")]
static TABLE_NS: [std::sync::atomic::AtomicU64; replay_data::rows::TABLE_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; replay_data::rows::TABLE_COUNT];

#[cfg(feature = "phase_cpu")]
pub(crate) fn add_table_ns(table_ns: &[u64; replay_data::rows::TABLE_COUNT]) {
    for (sum, ns) in TABLE_NS.iter().zip(table_ns) {
        sum.fetch_add(*ns, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(feature = "phase_cpu")]
static PACK_TICKS: [std::sync::atomic::AtomicU64; crate::extended::PACK_NAMES.len()] =
    [const { std::sync::atomic::AtomicU64::new(0) }; crate::extended::PACK_NAMES.len()];

#[cfg(feature = "phase_cpu")]
pub(crate) fn add_pack_ticks(ticks: &[Option<i64>; crate::extended::PACK_NAMES.len()]) {
    for (sum, ticks) in PACK_TICKS.iter().zip(ticks) {
        sum.fetch_add(ticks.unwrap_or(0).max(0) as u64, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(feature = "phase_cpu")]
pub(crate) fn status() -> impl serde::Serialize {
    let packs: std::collections::BTreeMap<String, u64> = crate::extended::PACK_NAMES
        .iter()
        .zip(&PACK_TICKS)
        .map(|(name, ticks)| (format!("{name}"), ticks.load(std::sync::atomic::Ordering::Relaxed)))
        .filter(|(_, ticks)| *ticks != 0)
        .collect();
    let tables: std::collections::BTreeMap<String, u64> = replay_data::rows::TableId::ALL
        .iter()
        .map(|table| (format!("{table:?}"), TABLE_NS[table.index()].load(std::sync::atomic::Ordering::Relaxed)))
        .filter(|(_, ns)| *ns != 0)
        .collect();
    let load = |steps: &Steps| {
        steps
            .cycles
            .each_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    };
    crate::status::status!({
        "order": "Receive Observe ConvertCore ConvertPacks HelperWait Merge Statistics Recycle Compare Encode Columnar Write Commit Observers Sizing Retire Flush Status Wait",
        "spool": load(&SPOOL).as_slice().to_vec(),
        "helper": load(&HELPER).as_slice().to_vec(),
        "sizing": load(&SIZING).as_slice().to_vec(),
        "cycle_query_failures": [&SPOOL, &HELPER, &SIZING]
            .map(|steps| steps.cycle_failures.load(std::sync::atomic::Ordering::Relaxed))
            .to_vec(),
        "table_ns": tables,
        "pack_ticks": packs,
    })
}

impl Steps {
    const fn new() -> Self {
        Self {
            current: std::sync::atomic::AtomicU8::new(Phase::Wait as u8),
            walls: [const { std::sync::atomic::AtomicU64::new(0) }; STEPS],
            gated: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            started: [const { std::sync::atomic::AtomicU64::new(0) }; Phase::Wait as usize + 1],
            #[cfg(feature = "phase_cpu")]
            cycles: [const { std::sync::atomic::AtomicU64::new(0) }; Phase::Wait as usize + 1],
            #[cfg(feature = "phase_cpu")]
            cycle_failures: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub(crate) fn current(&self) -> Phase {
        PHASES[usize::from(self.current.load(std::sync::atomic::Ordering::Relaxed))]
    }

    /// Starts `phase` once the game's tick is out of the way, unless it is the idle wait.
    pub(crate) fn span(&self, phase: Phase) -> Span<'_> {
        if phase != Phase::Wait {
            let begin = crate::clock::qpc();
            crate::tick_gate::wait();
            self.gated.fetch_add(
                (crate::clock::qpc() - begin).max(0) as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        self.span_now(phase)
    }

    /// Every step's cumulative wall, then the tick-gate wait, in QPC ticks.
    pub(crate) fn walls(&self) -> [u64; STEPS + 1] {
        let mut out = [0; STEPS + 1];
        for (out, wall) in out.iter_mut().zip(&self.walls) {
            *out = wall.load(std::sync::atomic::Ordering::Relaxed);
        }
        out[STEPS] = self.gated.load(std::sync::atomic::Ordering::Relaxed);
        out
    }

    /// [`Self::span`] started at once, also in the game's tick.
    pub(crate) fn span_now(&self, _phase: Phase) -> Span<'_> {
        self.current
            .store(_phase as u8, std::sync::atomic::Ordering::Relaxed);
        #[cfg(test)]
        self.started[_phase as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Span(
            self,
            _phase,
            crate::clock::qpc(),
            #[cfg(feature = "phase_cpu")]
            thread_cycles(),
        )
    }

    /// How many `phase` steps started.
    #[cfg(test)]
    pub(crate) fn phase(&self, phase: Phase) -> (u64,) {
        (self.started[phase as usize].load(std::sync::atomic::Ordering::Relaxed),)
    }
}

pub(crate) static SPOOL: Steps = Steps::new();
pub(crate) static HELPER: Steps = Steps::new();
pub(crate) static SIZING: Steps = Steps::new();

#[must_use = "a step ends when finished or dropped"]
pub(crate) struct Span<'a>(&'a Steps, Phase, i64, #[cfg(feature = "phase_cpu")] Option<u64>);

/// This thread's cycles so far; none when the query fails.
pub(crate) fn thread_cycles() -> Option<u64> {
    unsafe extern "system" {
        fn QueryThreadCycleTime(thread: windows_sys::Win32::Foundation::HANDLE, cycles: *mut u64) -> i32;
    }
    let mut cycles = 0;
    let ok = unsafe {
        QueryThreadCycleTime(windows_sys::Win32::System::Threading::GetCurrentThread(), &mut cycles)
    };
    (ok != 0).then_some(cycles)
}
/// The cycles since `begin`; none when either query failed.
pub(crate) fn cycles_since(begin: Option<u64>) -> Option<u64> {
    Some(thread_cycles()?.saturating_sub(begin?))
}

impl Drop for Span<'_> {
    fn drop(&mut self) {
        let phase = self.1 as usize;
        self.0.walls[phase].fetch_add(
            (crate::clock::qpc() - self.2).max(0) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        #[cfg(feature = "phase_cpu")]
        match cycles_since(self.3) {
            Some(cycles) => {
                self.0.cycles[phase].fetch_add(cycles, std::sync::atomic::Ordering::Relaxed);
            }
            None => {
                self.0.cycle_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

/// The steps a slow spool frame spent its wall in: names and QPC ticks, `Gate` last.
pub(crate) const WALL_NAMES: [&str; STEPS + 1] = [
    "Receive", "Observe", "ConvertCore", "ConvertPacks", "HelperWait", "Merge", "Statistics",
    "Recycle", "Compare", "Encode", "Columnar", "Write", "Commit", "Observers", "Sizing",
    "Retire", "Flush", "Status", "Wait", "Gate",
];

impl<'a> Span<'a> {
    pub(crate) fn finish(self) {}

    /// Ends this step and starts the next.
    pub(crate) fn switch(self, phase: Phase) -> Span<'a> {
        let steps = self.0;
        drop(self);
        steps.span(phase)
    }
}
