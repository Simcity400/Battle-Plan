#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
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

pub trait PhaseObserver {
    fn entered(&self, phase: Phase);
}

impl PhaseObserver for () {
    #[inline]
    fn entered(&self, _: Phase) {}
}

pub struct Steps<O = ()> {
    observer: O,
    current: std::sync::atomic::AtomicU8,
    walls: [std::sync::atomic::AtomicU64; STEPS],
    gated: std::sync::atomic::AtomicU64,
    #[cfg(feature = "phase_cpu")]
    cycles: [std::sync::atomic::AtomicU64; Phase::Wait as usize + 1],
    #[cfg(feature = "phase_cpu")]
    cycle_failures: std::sync::atomic::AtomicU64,
}

#[cfg(feature = "phase_cpu")]
static TABLE_NS: [std::sync::atomic::AtomicU64; replay_data::rows::TABLE_COUNT] =
    [const { std::sync::atomic::AtomicU64::new(0) }; replay_data::rows::TABLE_COUNT];

#[cfg(feature = "phase_cpu")]
/// # Safety
/// Only this session's originating spool may publish the timings of its completed frame.
pub unsafe fn add_table_ns(table_ns: &[u64; replay_data::rows::TABLE_COUNT]) {
    for (sum, ns) in TABLE_NS.iter().zip(table_ns) {
        sum.fetch_add(*ns, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(feature = "phase_cpu")]
pub fn status() -> impl serde::Serialize {
    let tables: std::collections::BTreeMap<String, u64> = replay_data::rows::TableId::ALL
        .iter()
        .map(|table| {
            (
                format!("{table:?}"),
                TABLE_NS[table.index()].load(std::sync::atomic::Ordering::Relaxed),
            )
        })
        .filter(|(_, ns)| *ns != 0)
        .collect();
    let load = |steps: &Steps| {
        steps
            .cycles
            .each_ref()
            .map(|c| c.load(std::sync::atomic::Ordering::Relaxed))
    };
    collector_status::status::status!({
        "order": "Receive Observe ConvertCore ConvertPacks HelperWait Merge Statistics Recycle Compare Encode Columnar Write Commit Observers Sizing Retire Flush Status Wait",
        "spool": load(&SPOOL).as_slice().to_vec(),
        "helper": load(&HELPER).as_slice().to_vec(),
        "sizing": load(&SIZING).as_slice().to_vec(),
        "cycle_query_failures": [&SPOOL, &HELPER, &SIZING]
            .map(|steps| steps.cycle_failures.load(std::sync::atomic::Ordering::Relaxed))
            .to_vec(),
        "table_ns": tables,
    })
}

impl<O: PhaseObserver> Steps<O> {
    const fn new(observer: O) -> Self {
        Self {
            observer,
            current: std::sync::atomic::AtomicU8::new(Phase::Wait as u8),
            walls: [const { std::sync::atomic::AtomicU64::new(0) }; STEPS],
            gated: std::sync::atomic::AtomicU64::new(0),
            #[cfg(feature = "phase_cpu")]
            cycles: [const { std::sync::atomic::AtomicU64::new(0) }; Phase::Wait as usize + 1],
            #[cfg(feature = "phase_cpu")]
            cycle_failures: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn current(&self) -> Phase {
        PHASES[usize::from(self.current.load(std::sync::atomic::Ordering::Relaxed))]
    }

    /// # Safety
    /// Only a worker designated by this attachment to record phases on these counters may start a
    /// span, or an isolated fixture may use its own counters. Keep that authority through the
    /// span's finish or drop.
    pub unsafe fn span(&self, phase: Phase) -> Span<'_, O> {
        if phase != Phase::Wait {
            let begin = collector_primitives::clock::qpc();
            crate::tick_gate::wait();
            self.gated.fetch_add(
                (collector_primitives::clock::qpc() - begin).max(0) as u64,
                std::sync::atomic::Ordering::Relaxed,
            );
        }
        // SAFETY: This conversion worker records its phase on the session's helper counters.
        unsafe { self.span_now(phase) }
    }

    pub fn walls(&self) -> [u64; STEPS + 1] {
        let mut out = [0; STEPS + 1];
        for (out, wall) in out.iter_mut().zip(&self.walls) {
            *out = wall.load(std::sync::atomic::Ordering::Relaxed);
        }
        out[STEPS] = self.gated.load(std::sync::atomic::Ordering::Relaxed);
        out
    }

    /// # Safety
    /// Only a worker designated by this attachment to record phases on these counters may start a
    /// span, or an isolated fixture may use its own counters. Keep that authority through the
    /// span's finish or drop.
    pub unsafe fn span_now(&self, _phase: Phase) -> Span<'_, O> {
        self.current
            .store(_phase as u8, std::sync::atomic::Ordering::Relaxed);
        self.observer.entered(_phase);
        Span(
            self,
            _phase,
            collector_primitives::clock::qpc(),
            #[cfg(feature = "phase_cpu")]
            thread_cycles(),
            std::marker::PhantomData,
        )
    }

}

pub static SPOOL: Steps = Steps::new(());
pub static HELPER: Steps = Steps::new(());
pub static SIZING: Steps = Steps::new(());

#[must_use = "a step ends when finished or dropped"]
pub struct Span<'a, O: PhaseObserver = ()>(
    &'a Steps<O>,
    Phase,
    i64,
    #[cfg(feature = "phase_cpu")] Option<u64>,
    std::marker::PhantomData<*mut ()>,
);

pub fn thread_cycles() -> Option<u64> {
    unsafe extern "system" {
        fn QueryThreadCycleTime(
            thread: windows_sys::Win32::Foundation::HANDLE,
            cycles: *mut u64,
        ) -> i32;
    }
    let mut cycles = 0;
    let ok = unsafe {
        QueryThreadCycleTime(
            windows_sys::Win32::System::Threading::GetCurrentThread(),
            &raw mut cycles,
        )
    };
    (ok != 0).then_some(cycles)
}
pub fn cycles_since(begin: Option<u64>) -> Option<u64> {
    Some(thread_cycles()?.saturating_sub(begin?))
}

impl<O: PhaseObserver> Drop for Span<'_, O> {
    fn drop(&mut self) {
        let phase = self.1 as usize;
        self.0.walls[phase].fetch_add(
            (collector_primitives::clock::qpc() - self.2).max(0) as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
        #[cfg(feature = "phase_cpu")]
        match cycles_since(self.3) {
            Some(cycles) => {
                self.0.cycles[phase].fetch_add(cycles, std::sync::atomic::Ordering::Relaxed);
            }
            None => {
                self.0
                    .cycle_failures
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
    }
}

pub const WALL_NAMES: [&str; STEPS + 1] = [
    "Receive",
    "Observe",
    "ConvertCore",
    "ConvertPacks",
    "HelperWait",
    "Merge",
    "Statistics",
    "Recycle",
    "Compare",
    "Encode",
    "Columnar",
    "Write",
    "Commit",
    "Observers",
    "Sizing",
    "Retire",
    "Flush",
    "Status",
    "Wait",
    "Gate",
];

impl<'a, O: PhaseObserver> Span<'a, O> {
    pub fn finish(self) {}

    pub fn switch(self, phase: Phase) -> Span<'a, O> {
        let steps = self.0;
        drop(self);
        // SAFETY: This worker records phases on the counters assigned to its spool role.
        unsafe { steps.span(phase) }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod fixtures {
    use super::{Phase, PhaseObserver, STEPS, Steps};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Entries([AtomicUsize; STEPS]);

    impl PhaseObserver for Entries {
        fn entered(&self, phase: Phase) {
            self.0[phase as usize].fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Steps<Entries> {
        pub fn counted() -> Self {
            Self::new(Entries([const { AtomicUsize::new(0) }; STEPS]))
        }

        pub fn entries(&self, phase: Phase) -> usize {
            self.observer.0[phase as usize].load(Ordering::Relaxed)
        }
    }
}

static RETRY_CAUSE: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(0);

/// # Safety
/// Only the sole spool may publish a retry cause from its own frame. Only the admitted capture
/// may consume that cause for its next frame, while both belong to the same attachment.
pub unsafe fn retry_cause() -> &'static std::sync::atomic::AtomicU16 {
    &RETRY_CAUSE
}
