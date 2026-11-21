use std::cell::Cell;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;

pub const NO_FRAME: &str = "event feed witness has no open detour frame";
pub const OTHER_FRAME: &str = "event feed witness reached another detour's frame";
pub const WITNESS_ALLOCATION: &str = "event feed detour witness allocation failed";
pub const WITNESS_ABANDONED: &str = "event feed detour witness abandoned before publication";
pub const WITNESS_RESET: &str = "event feed detour witnesses were reset before publication";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Witness {
    Reactivated {
        status: usize,
        definition: usize,
        instance: usize,
    },
    Selected { instance: usize },
    Ignored { instance: usize, was: bool },
    RemovalRequested { instance: usize, was: bool },
    FiredMatch { state: usize, definition: usize },
}

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
    CandidateFinalization,
}

std::thread_local! {
    static SEQUENCE: Cell<u32> = const { Cell::new(0) };
    static CURRENT: Cell<u64> = const { Cell::new(0) };
    static CURRENT_KIND: Cell<u8> = const { Cell::new(0) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

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

pub(crate) struct Detour {
    flight: super::Flight,
    token: u64,
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
    fn close(&mut self) {
        if !self.closed {
            self.closed = true;
            CURRENT.set(self.parent);
            CURRENT_KIND.set(self.parent_kind);
            DEPTH.set(DEPTH.get().wrapping_sub(1));
        }
    }
    pub(crate) fn publish(mut self, mut publish: impl FnMut(Witness)) {
        self.close();
        loop {
            match super::feed::take_staged(self.token, self.epoch) {
                Ok(Some(witness)) => publish(witness),
                Ok(None) => break,
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
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
pub(crate) fn open_frames() -> usize {
    DEPTH.with(Cell::get) as usize
}

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
