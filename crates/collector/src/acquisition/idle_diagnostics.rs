use super::Ticket;
use crate::acquisition_diagnostics::{Identity, ns};
use std::cell::RefCell;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Address {
    #[default]
    Readiness,
    #[cfg(any(test, feature = "callback_profile"))]
    Slot,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Wait {
    pub address: Address,
    pub ticket: Option<Ticket>,
    pub comparison: u64,
    pub observed: u64,
    pub begin: i64,
    pub end: i64,
    pub thread: u32,
    pub cycles: Option<u64>,
}
impl Wait {
    fn ticks(self) -> i64 {
        self.end.saturating_sub(self.begin)
    }
    fn status(self, frequency: u64) -> impl serde::Serialize {
        crate::status::status!({
            "address": match self.address {
                Address::Readiness => "readiness",
                #[cfg(any(test, feature = "callback_profile"))]
                Address::Slot => "slot",
            },
            "ticket": self.ticket, "comparison": self.comparison, "observed": self.observed,
            "begin_qpc": self.begin, "return_qpc": self.end,
            "wall_ns": ns(self.ticks().max(0) as u64, frequency),
            "thread_id": self.thread, "thread_cycles": self.cycles,
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Signal {
    pub ticket: Option<Ticket>,
    pub identity: Option<Identity>,
    pub published: i64,
    pub released: i64,
    pub wake_begin: i64,
    pub wake_end: i64,
    pub thread: u32,
    pub readiness: u64,
    pub slot: bool,
    pub handoff: bool,
}
impl Signal {
    #[cfg(any(test, feature = "callback_profile"))]
    fn matches(self, wait: Wait) -> bool {
        if self.wake_begin == 0 || self.wake_begin > wait.end {
            return false;
        }
        match wait.address {
            Address::Readiness => {
                self.readiness > wait.comparison && self.readiness <= wait.observed
            }
            Address::Slot => self.slot && self.ticket == wait.ticket,
        }
    }
    fn status(self, frequency: u64, wait: Wait) -> impl serde::Serialize {
        let duration = |begin: i64, end: i64| {
            (begin != 0 && end >= begin).then(|| ns((end - begin) as u64, frequency))
        };
        crate::status::status!({
            "ticket": self.ticket, "identity": self.identity.map(Identity::status),
            "source": if self.handoff { "inputs_handoff" } else if self.published != 0 { "publication" } else { "claim" },
            "readiness_sequence": (self.readiness != 0).then_some(self.readiness),
            "publisher_thread_id": self.thread,
            "published_qpc": (self.published != 0).then_some(self.published),
            "released_qpc": (self.released != 0).then_some(self.released),
            "wake_begin_qpc": self.wake_begin, "wake_end_qpc": self.wake_end,
            "publish_to_release_ns": duration(self.published, self.released),
            "release_to_wake_ns": duration(self.released, self.wake_begin),
            "wake_call_ns": duration(self.wake_begin, self.wake_end),
            "publish_to_wait_return_ns": duration(self.published, wait.end),
            "wake_to_wait_return_ns": duration(self.wake_begin, wait.end),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Inputs {
    pub publisher_thread_id: u32,
    pub lock_begin_qpc: i64,
    pub lock_acquired_qpc: i64,
    pub merge_end_qpc: i64,
    pub store_begin_qpc: i64,
    pub store_end_qpc: i64,
    pub submit_begin_qpc: i64,
    pub unlock_begin_qpc: i64,
    pub unlock_qpc: i64,
    pub thread_cycles: Option<u64>,
    pub deferred_polls: u64,
}
impl Inputs {
    fn status(self, frequency: u64) -> impl serde::Serialize {
        let stamp = |qpc| (qpc != 0).then_some(qpc);
        let duration = |begin: i64, end: i64| {
            (begin != 0 && end >= begin).then(|| ns((end - begin) as u64, frequency))
        };
        crate::status::status!({
            "publisher_thread_id": self.publisher_thread_id,
            "lock_begin_qpc": stamp(self.lock_begin_qpc), "lock_acquired_qpc": stamp(self.lock_acquired_qpc),
            "merge_end_qpc": stamp(self.merge_end_qpc), "store_begin_qpc": stamp(self.store_begin_qpc),
            "store_end_qpc": stamp(self.store_end_qpc), "submit_begin_qpc": stamp(self.submit_begin_qpc),
            "unlock_begin_qpc": stamp(self.unlock_begin_qpc), "unlock_qpc": stamp(self.unlock_qpc),
            "lock_wait_ns": duration(self.lock_begin_qpc, self.lock_acquired_qpc),
            "merge_ns": duration(self.lock_acquired_qpc, self.merge_end_qpc),
            "store_loan_ns": duration(self.store_begin_qpc, self.store_end_qpc),
            "submit_ns": duration(self.submit_begin_qpc, self.unlock_begin_qpc),
            "unlock_ns": duration(self.unlock_begin_qpc, self.unlock_qpc),
            "thread_cycles": self.thread_cycles, "deferred_polls": self.deferred_polls,
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Table {
    index: usize,
    ticks: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Family {
    total: u64,
    top: [Table; 4],
    len: usize,
}
impl Family {
    pub(crate) fn new(tables: impl IntoIterator<Item = (replay_data::rows::TableId, u64)>) -> Self {
        let mut family = Self::default();
        for (table, ticks) in tables {
            family.total = family.total.saturating_add(ticks);
            if ticks == 0 {
                continue;
            }
            let at = family.top[..family.len].partition_point(|old| old.ticks >= ticks);
            if at == family.top.len() {
                continue;
            }
            let len = (family.len + 1).min(family.top.len());
            family.top.copy_within(at..len - 1, at + 1);
            family.top[at] = Table {
                index: table.index(),
                ticks,
            };
            family.len = len;
        }
        family
    }
    fn status(self, frequency: u64) -> impl serde::Serialize {
        crate::status::status!({
            "measured_table_ns": ns(self.total, frequency),
            "top_tables": crate::status::sequence(self.top.into_iter().take(self.len).map(move |table| crate::status::status!({
                "table": crate::status::DebugValue(replay_data::rows::TableId::ALL[table.index]),
                "wall_ns": ns(table.ticks, frequency),
            }))),
        })
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hour {
    pub wait: Option<Wait>,
    pub signal: Option<Signal>,
    pub matching_signals: u64,
    pub dropped_signals: u64,
    pub tracing_available: bool,
    pub inputs: Option<Inputs>,
    pub governance: Option<Family>,
    pub buildings_ticks: Option<u64>,
    pub projects: Option<Family>,
    pub variants_cycles: Option<u64>,
    pub governance_cycles: Option<u64>,
    pub projects_cycles: Option<u64>,
}
impl Hour {
    #[cfg(any(test, feature = "callback_profile"))]
    fn signal(&mut self, signal: Signal) {
        if self.wait.is_some_and(|wait| signal.matches(wait)) {
            self.matching_signals += 1;
            if self
                .signal
                .is_none_or(|old| signal.wake_begin < old.wake_begin)
            {
                self.signal = Some(signal);
            }
        }
    }
    pub(crate) fn status(self, frequency: u64) -> impl serde::Serialize {
        crate::status::status!({
            "longest_address_wait": self.wait.map(move |wait| wait.status(frequency)),
            "first_matching_signal": self.signal.zip(self.wait).map(move |(signal, wait)| signal.status(frequency, wait)),
            "tracing_available": self.tracing_available,
            "matching_signals": self.tracing_available.then_some(self.matching_signals), "dropped_signals": self.tracing_available.then_some(self.dropped_signals),
            "inputs_handoff": self.inputs.map(move |inputs| inputs.status(frequency)),
            "governance": self.governance.map(move |family| family.status(frequency)),
            "buildings_ns": self.buildings_ticks.map(move |ticks| ns(ticks, frequency)),
            "gated_projects": self.projects.map(move |family| family.status(frequency)),
            "variants_thread_cycles": self.variants_cycles,
            "governance_thread_cycles": self.governance_cycles,
            "gated_projects_thread_cycles": self.projects_cycles,
        })
    }
}
thread_local! {
    static HOUR: RefCell<Hour> = RefCell::new(Hour::default());
}
pub(crate) fn reset() {
    update(|hour| *hour = Hour::default());
}
pub(crate) fn update(f: impl FnOnce(&mut Hour)) {
    HOUR.with_borrow_mut(f);
}
pub(crate) fn take() -> Hour {
    HOUR.with_borrow_mut(std::mem::take)
}

#[cfg(feature = "callback_profile")]
pub(crate) fn wait(
    address: Address,
    ticket: Option<Ticket>,
    comparison: u64,
    run: impl FnOnce() -> u64,
) {
    if !super::callback_parks::active() {
        run();
        return;
    }
    let before = crate::callback_profile::thread_cycles();
    let begin = crate::sampling::qpc();
    let observed = run();
    let end = crate::sampling::qpc();
    let after = crate::callback_profile::thread_cycles();
    let wait = Wait {
        address,
        ticket,
        comparison,
        observed,
        begin,
        end,
        thread: unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() },
        cycles: before
            .zip(after)
            .and_then(|(before, after)| after.checked_sub(before)),
    };
    update(|hour| {
        if hour.wait.is_none_or(|old| wait.ticks() > old.ticks()) {
            hour.wait = Some(wait);
        }
    });
}

#[cfg(any(test, feature = "callback_profile"))]
mod events {
    use super::*;
    use std::{
        cell::UnsafeCell,
        sync::atomic::{AtomicUsize, Ordering},
    };
    pub(crate) struct Events {
        next: AtomicUsize,
        entries: Box<[UnsafeCell<Signal>]>,
    }
    // SAFETY: each writer owns a unique index; only a drained and settled batch reads or resets.
    unsafe impl Sync for Events {}
    impl Events {
        pub(crate) fn new(capacity: usize) -> Option<Self> {
            engine_allocation::advisory(|| {
                let mut entries = Vec::new();
                entries.try_reserve_exact(capacity).ok()?;
                entries.resize_with(capacity, || UnsafeCell::new(Signal::default()));
                Some(Self {
                    next: AtomicUsize::new(0),
                    entries: entries.into_boxed_slice(),
                })
            })
        }
        pub(crate) fn allocated_bytes(&self) -> usize {
            std::mem::size_of_val(&*self.entries)
        }
        pub(crate) fn record(&self, signal: Signal) {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            if let Some(entry) = self.entries.get(index) {
                // SAFETY: this fetch_add assigned the index to this publisher alone.
                unsafe { entry.get().write(signal) };
            }
        }
        pub(crate) unsafe fn drain_into(&self, hour: &mut Hour) {
            hour.tracing_available = true;
            let count = self.next.swap(0, Ordering::Relaxed);
            hour.dropped_signals += count.saturating_sub(self.entries.len()) as u64;
            for entry in self.entries.iter().take(count) {
                // SAFETY: the caller ended every publisher before draining, including detached continuations.
                hour.signal(unsafe { *entry.get() });
            }
        }
    }
}
#[cfg(any(test, feature = "callback_profile"))]
pub(crate) use events::Events;

#[cfg(test)]
mod tests;
