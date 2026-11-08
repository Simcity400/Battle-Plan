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

pub(crate) struct Steps {
    /// Steps started, so tests can see which thread took a step.
    #[cfg(test)]
    started: [std::sync::atomic::AtomicU64; Phase::Wait as usize + 1],
}

impl Steps {
    const fn new() -> Self {
        Self {
            #[cfg(test)]
            started: [const { std::sync::atomic::AtomicU64::new(0) }; Phase::Wait as usize + 1],
        }
    }

    /// Starts `phase` once the game's tick is out of the way, unless it is the idle wait.
    pub(crate) fn span(&self, phase: Phase) -> Span<'_> {
        if phase != Phase::Wait {
            crate::tick_gate::wait();
        }
        self.span_now(phase)
    }

    /// [`Self::span`] started at once, also in the game's tick.
    pub(crate) fn span_now(&self, _phase: Phase) -> Span<'_> {
        #[cfg(test)]
        self.started[_phase as usize].fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Span(self)
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
pub(crate) struct Span<'a>(&'a Steps);

impl<'a> Span<'a> {
    pub(crate) fn finish(self) {}

    /// Ends this step and starts the next.
    pub(crate) fn switch(self, phase: Phase) -> Span<'a> {
        self.0.span(phase)
    }
}
