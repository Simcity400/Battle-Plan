//! Per-invocation witnesses for detours whose native ABI omits a selected
//! operand. An enclosing detour opens a frame, runs its original once under
//! a writer flight and publishes the committed results its latches staged
//! inside, in native order. A latch (a selected tap or helper CALL inside
//! the original) only stages into the innermost open frame and never
//! publishes.
//!
//! Witnesses live in the shared store's demand-sized arena, tagged with
//! their frame's token; a latch appends through the normal callback entry,
//! so a declined stage is a missed observation like any callback; a burst
//! past the prepared arena grows it, and only a failed allocation refuses
//! by name. Frame nesting lives
//! on the native call stack: a detour saves its parent's token and restores
//! it, so only two small thread-local counters are kept, with no cap. A
//! frame abandoned by an unwind leaves its witnesses for the next boundary
//! to drop and refuse.
use std::cell::Cell;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

pub const NO_FRAME: &str = "event feed witness has no open detour frame";
pub const OTHER_FRAME: &str = "event feed witness reached another detour's frame";
pub const WITNESS_ALLOCATION: &str = "event feed detour witness allocation failed";
pub const WITNESS_ABANDONED: &str = "event feed detour witness abandoned before publication";
pub const WITNESS_RESET: &str = "event feed detour witnesses were reset before publication";

/// What a latch saw, staged for its enclosing detour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Witness {
    /// A reached mission reactivation of `instance`.
    Reactivated {
        status: usize,
        definition: usize,
        instance: usize,
    },
    /// The selected timed or targeted instance.
    Selected { instance: usize },
    /// A selected ignore store, with the instance's ignore flag before it.
    Ignored { instance: usize, was: bool },
    /// The selected removal request, with whether it was already requested.
    RemovalRequested { instance: usize, was: bool },
    /// The fired-match branch of a replaced-event retirement.
    FiredMatch { state: usize, definition: usize },
}

/// The enclosing detour a frame belongs to. A latch's kind must match its
/// innermost open frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum Unit {
    AutoActivation,
    ScriptedActivation,
    TimedSelection,
    TargetedSelection,
    TargetedIgnore,
    IgnoreAll,
    RemovalRequest,
    FiredRetirement,
    /// Its clears have no latch: its frame stays empty.
    CandidateFinalization,
}

std::thread_local! {
    /// This thread's next frame sequence. Monotonic; never reused, so a
    /// token identifies its frame for the whole session.
    static SEQUENCE: Cell<u32> = const { Cell::new(0) };
    /// The innermost open frame's token, or zero. A token's high word is the
    /// OS thread id, never zero, so zero is unambiguously "no frame".
    static CURRENT: Cell<u64> = const { Cell::new(0) };
    /// That frame's [`Unit`].
    static CURRENT_KIND: Cell<u8> = const { Cell::new(0) };
    /// Open frames on this thread. Diagnostics and tests only.
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// A latch's witness for the innermost open frame. Without one, or of the
/// wrong unit, it refuses by name; it never publishes. A declined or
/// storage-short store entry is counted like any callback.
pub(crate) fn stage(unit: Unit, witness: Witness) {
    let token = CURRENT.get();
    if token == 0 {
        super::feed::refuse(NO_FRAME);
    } else if CURRENT_KIND.get() != unit as u8 {
        super::feed::refuse(OTHER_FRAME);
    } else {
        super::feed::stage_witness(token, witness);
    }
}

/// An enclosing detour's invocation: a writer flight from before its
/// original through its publication, and a frame its latches stage into. Its
/// parent frame is saved on the native stack and restored when it closes.
pub(crate) struct Detour {
    flight: super::Flight,
    token: u64,
    /// The witness arena's epoch at entry, when the store was entered. A
    /// boundary resets the arena and bumps it, dropping witnesses staged so
    /// far; publication then refuses by name instead of publishing nothing.
    epoch: Option<u64>,
    parent: u64,
    parent_kind: u8,
    closed: bool,
}
impl Detour {
    pub(crate) fn enter(unit: Unit) -> Self {
        let flight = super::Flight::enter();
        let seq = SEQUENCE.get();
        SEQUENCE.set(seq.wrapping_add(1));
        let token = (u64::from(unsafe { GetCurrentThreadId() }) << 32) | u64::from(seq);
        let parent = CURRENT.replace(token);
        let parent_kind = CURRENT_KIND.replace(unit as u8);
        DEPTH.set(DEPTH.get().wrapping_add(1));
        // A declined entry is a missed observation: its interval is refused
        // either way, so publication then has nothing to check against.
        let epoch = super::feed::staging_epoch();
        Self {
            flight,
            token,
            epoch,
            parent,
            parent_kind,
            closed: false,
        }
    }
    /// Restore the parent frame. Idempotent between [`Self::publish`] and the
    /// unwind [`Drop`].
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            CURRENT.set(self.parent);
            CURRENT_KIND.set(self.parent_kind);
            DEPTH.set(DEPTH.get().wrapping_sub(1));
        }
    }
    /// After the original returned normally: close the frame, then publish
    /// and remove its staged witnesses in native order, and mark the flight
    /// returned. A publisher may enter its own detour, so the frame is
    /// closed first.
    pub(crate) fn publish(mut self, mut publish: impl FnMut(Witness)) {
        self.close();
        loop {
            match super::feed::take_staged(self.token, self.epoch) {
                Ok(Some(witness)) => publish(witness),
                Ok(None) => break,
                // A boundary reset the arena while this detour was open on
                // this thread: its witnesses are gone, so refuse its
                // interval rather than silently publishing nothing.
                Err(reason) => {
                    super::feed::refuse(reason);
                    break;
                }
            }
        }
        self.flight.returned();
    }
}
impl Drop for Detour {
    /// An abnormal exit closes the frame but never finishes a mutation: its
    /// witnesses stay in the store for the next boundary to drop and refuse,
    /// and the flight retains native ownership.
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
pub(crate) fn open_frames() -> usize {
    DEPTH.with(Cell::get) as usize
}

/// Test only: the committed values each detour's publisher read, in order.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Published {
    Activation {
        instance: usize,
        state: i32,
        days: i32,
    },
    Selection {
        instance: usize,
        days: i32,
    },
    Ignore {
        instance: usize,
        was: bool,
        now: bool,
    },
    Removal {
        instance: usize,
        was: bool,
        now: u8,
    },
    Fired {
        definition: usize,
    },
    Duration {
        instance: usize,
        days: i32,
    },
    Candidates {
        status: usize,
    },
}
#[cfg(test)]
std::thread_local! {
    static PUBLISHED: std::cell::RefCell<Vec<Published>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[cfg(test)]
pub(crate) fn published(publication: Published) {
    PUBLISHED.with_borrow_mut(|log| log.push(publication));
}
#[cfg(test)]
pub(crate) fn take_published() -> Vec<Published> {
    PUBLISHED.with_borrow_mut(std::mem::take)
}
