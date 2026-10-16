//! Publication measurements stay in caller-owned scratch until every hold
//! resolves. Only then are they merged into status and attach progress.
use crate::status::{map, status};
use std::{
    cell::Cell,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Summary {
    pub attempts: u64,
    pub total_hold_ns: u64,
    pub longest_hold_ns: u64,
    pub longest_synchronization_ns: u64,
    pub slowest_context_ns: u64,
    pub slowest_context_thread_id: u32,
}
impl Summary {
    fn merge(&mut self, other: Self) {
        self.attempts = self.attempts.saturating_add(other.attempts);
        self.total_hold_ns = self.total_hold_ns.saturating_add(other.total_hold_ns);
        self.longest_hold_ns = self.longest_hold_ns.max(other.longest_hold_ns);
        self.longest_synchronization_ns = self
            .longest_synchronization_ns
            .max(other.longest_synchronization_ns);
        if other.slowest_context_thread_id != 0
            && (self.slowest_context_thread_id == 0
                || other.slowest_context_ns > self.slowest_context_ns)
        {
            self.slowest_context_ns = other.slowest_context_ns;
            self.slowest_context_thread_id = other.slowest_context_thread_id;
        }
    }
}

thread_local! {
    static FAMILY: Cell<Option<&'static str>> = const { Cell::new(None) };
}
static FAMILIES: Mutex<Vec<(&'static str, Summary)>> = Mutex::new(Vec::new());
static AVAILABLE: AtomicBool = AtomicBool::new(true);

pub fn select_family(name: Option<&'static str>) {
    FAMILY.set(name);
}

pub(super) fn current_family() -> Option<&'static str> {
    FAMILY.get()
}

#[cfg(any(test, feature = "test-support"))]
pub fn complete_test_attempt() {
    Attempt::new().finish();
}

fn record(name: &'static str, sample: Summary) {
    let mut families = FAMILIES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, summary)) = families.iter_mut().find(|(family, _)| *family == name) {
        summary.merge(sample);
    } else if families.try_reserve(1).is_ok() {
        families.push((name, sample));
    } else {
        AVAILABLE.store(false, Ordering::Relaxed);
    }
}

pub fn snapshot() -> (Vec<(&'static str, Summary)>, bool) {
    let families = FAMILIES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut snapshot = Vec::new();
    let copied = snapshot.try_reserve_exact(families.len()).is_ok();
    if copied {
        snapshot.extend_from_slice(&families);
    }
    (snapshot, copied && AVAILABLE.load(Ordering::Relaxed))
}

/// Serializes the owned snapshot through a borrowing iterator: cloning an
/// owning iterator would copy the snapshot again, unchecked, on every write.
struct Families(Vec<(&'static str, Summary)>);
impl serde::Serialize for Families {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        map(self.0.iter().map(|(name, summary)| (*name, summary))).serialize(serializer)
    }
}

pub fn status() -> impl serde::Serialize + use<> {
    let (snapshot, available) = snapshot();
    status!({"families": Families(snapshot), "snapshot_available": available})
}

pub(super) enum Clock {
    Live(Instant),
    #[cfg(test)]
    Fake(std::rc::Rc<Cell<u64>>),
}
impl Clock {
    fn now(&self) -> u64 {
        match self {
            Self::Live(begin) => begin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
            #[cfg(test)]
            Self::Fake(now) => now.get(),
        }
    }
}

/// No heap use, formatting, logging or locks in any measurement operation.
pub(super) struct Attempt {
    pub sample: Summary,
    pub clock: Clock,
    family: Option<&'static str>,
    hold_begin: Option<u64>,
    held: usize,
}
impl Attempt {
    pub fn new() -> Self {
        Self {
            sample: Summary {
                attempts: 1,
                ..Default::default()
            },
            clock: Clock::Live(Instant::now()),
            family: FAMILY.get(),
            hold_begin: None,
            held: 0,
        }
    }
    pub fn now(&self) -> u64 {
        self.clock.now()
    }
    pub fn elapsed(&self, begin: u64) -> u64 {
        self.now().saturating_sub(begin)
    }
    pub fn acquired(&mut self, begin: u64) {
        if self.held == 0 {
            self.hold_begin = Some(begin);
        }
        self.held += 1;
    }
    pub fn resolved(&mut self) {
        // Synthetic controllers can start with holds already acquired.
        if self.held == 0 {
            return;
        }
        self.held -= 1;
        if self.held == 0 {
            let begin = self.hold_begin.take().unwrap();
            let elapsed = self.elapsed(begin);
            self.sample.total_hold_ns = self.sample.total_hold_ns.saturating_add(elapsed);
            self.sample.longest_hold_ns = self.sample.longest_hold_ns.max(elapsed);
        }
    }
    pub fn context(&mut self, begin: u64, id: u32) {
        let elapsed = self.elapsed(begin);
        if self.sample.slowest_context_thread_id == 0 || elapsed > self.sample.slowest_context_ns {
            self.sample.slowest_context_ns = elapsed;
            self.sample.slowest_context_thread_id = id;
        }
    }
    /// Called by Threads::drop after its unconditional resume and handle cleanup.
    pub fn finish(&mut self) {
        debug_assert_eq!(self.held, 0);
        if let Some(family) = self.family.take() {
            record(family, self.sample);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::with_allocations_denied;
    use serde::Serialize;

    #[test]
    fn a_copied_snapshot_serializes_without_allocating() {
        struct Count(usize);
        impl std::io::Write for Count {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0 += bytes.len();
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        record(
            "serialization_fixture",
            Summary {
                attempts: 1,
                total_hold_ns: 7,
                ..Summary::default()
            },
        );
        let value = status();
        let mut count = Count(0);
        with_allocations_denied(|| value.serialize(&mut serde_json::Serializer::new(&mut count)))
            .unwrap();
        let mut bytes = Vec::new();
        engine::allocation::reserve_exact(&mut bytes, count.0).unwrap();
        with_allocations_denied(|| serde_json::to_writer(&mut bytes, &value)).unwrap();
        let status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            status["families"]["serialization_fixture"]["total_hold_ns"],
            7
        );
    }
}
