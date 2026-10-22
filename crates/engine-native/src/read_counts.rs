//! Source requests, measured below all owned snapshots and caches.
//!
//! Wrap the raw source once, before constructing any acquisition adapters.
//! Batch methods delegate to that source's overrides, never to this wrapper's
//! scalar fallback. Thread-local counters need no per-read allocation, lock or
//! atomic. Owners drain them at table/job boundaries, including on unwind.

use crate::{
    Memory,
    memory::{RecordLayout, RecordSources, SliceMemory},
    read_plan::{Outcome, PreparedPlan},
};
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
use std::ptr::NonNull;
use std::{cell::Cell, ops::AddAssign};

/// Requested work at the raw Memory boundary, not bytes retained in a snapshot.
/// Calls include empty requests and rejected/faulted calls. Spans exclude empty
/// destinations and count the completed prefix plus its first failed/rejected
/// source request; a plan control rejection has no following source request.
/// Bytes sum those spans' requested lengths, including an entire faulted span
/// whose partial copied length is unknown. Invalid destination geometry has no
/// source requests. Neither successful-byte counts nor cache-line traffic is
/// inferred from these counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReadCounts {
    pub copy_calls: u64,
    pub scattered_calls: u64,
    pub record_calls: u64,
    pub plan_calls: u64,
    pub spans: u64,
    pub bytes: u64,
}

thread_local! {
    static COUNTS: Cell<ReadCounts> = const { Cell::new(ReadCounts::ZERO) };
}

impl ReadCounts {
    pub const ZERO: Self = Self {
        copy_calls: 0,
        scattered_calls: 0,
        record_calls: 0,
        plan_calls: 0,
        spans: 0,
        bytes: 0,
    };

    /// Drain this thread's raw requests. Nested job owners must retain and
    /// restore the previous value so caller-assisted work is not charged twice.
    pub fn take_current() -> Self {
        COUNTS.with(|counts| counts.replace(Self::ZERO))
    }

    /// Restore a suspended owner or add a batch's source requests.
    #[inline]
    pub fn add_current(self) {
        COUNTS.with(|counts| self.add_to(counts));
    }

    #[inline]
    fn add_to(self, counts: &Cell<Self>) {
        let mut value = counts.get();
        value += self;
        counts.set(value);
    }
}

impl AddAssign for ReadCounts {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.copy_calls = self.copy_calls.saturating_add(rhs.copy_calls);
        self.scattered_calls = self.scattered_calls.saturating_add(rhs.scattered_calls);
        self.record_calls = self.record_calls.saturating_add(rhs.record_calls);
        self.plan_calls = self.plan_calls.saturating_add(rhs.plan_calls);
        self.spans = self.spans.saturating_add(rhs.spans);
        self.bytes = self.bytes.saturating_add(rhs.bytes);
    }
}

/// A single accounting boundary around a raw source (LiveMemory in production,
/// a synthetic fixture in tests). Put caching/recording adapters outside it:
/// owned reads then bypass this boundary and misses retain the native fast path.
/// On Windows x64 MSVC, the private pointer makes this neither Send nor Sync.
pub struct CountedMemory<M>(
    pub M,
    #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))] NonNull<Cell<ReadCounts>>,
);

impl<M> CountedMemory<M> {
    pub fn new(source: M) -> Self {
        Self(
            source,
            #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
            COUNTS.with(|counts| NonNull::from(counts)),
        )
    }

    #[inline]
    fn add_counts(&self, counts: ReadCounts) {
        #[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
        {
            // SAFETY: on this target, pinned std stores const/no-Drop COUNTS
            // in fixed native TLS until thread/module exit. Private !Send/!Sync
            // handle stays on that thread; collector quiescence ends all uses
            // before DLL unload. No destructor destroys the Cell; resets only
            // replace its contents. This short shared borrow permits nesting.
            counts.add_to(unsafe { self.1.as_ref() });
        }
        #[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
        counts.add_current();
    }
}

impl<M: Memory> Memory for CountedMemory<M> {
    #[inline]
    fn read_scalar<T: crate::memory::Raw>(&self, address: usize) -> Option<T> {
        let len = std::mem::size_of::<T>();
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(len != 0),
            bytes: len as u64,
            ..ReadCounts::ZERO
        });
        self.0.read_scalar(address)
    }

    #[inline]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(!destination.is_empty()),
            bytes: destination.len() as u64,
            ..ReadCounts::ZERO
        });
        self.0.copy(address, destination)
    }

    #[inline]
    fn copy_uninit<'a>(
        &self,
        address: usize,
        destination: &'a mut [std::mem::MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        self.add_counts(ReadCounts {
            copy_calls: 1,
            spans: u64::from(!destination.is_empty()),
            bytes: destination.len() as u64,
            ..ReadCounts::ZERO
        });
        self.0.copy_uninit(address, destination)
    }

    #[inline]
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.0.copy_scattered(addresses, destinations);
        self.add_counts(count_scattered(destinations, completed));
        completed
    }

    #[inline]
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.0.copy_scattered_prefetched(addresses, destinations);
        self.add_counts(count_scattered(destinations, completed));
        completed
    }

    #[inline(always)]
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let completed = self.0.copy_records(sources, layout, destination);
        let (spans, bytes) = match layout.count(sources, destination.len()) {
            Some(count) => {
                let requested = completed
                    .saturating_add(usize::from(completed < count))
                    .min(count);
                (
                    layout.completed_spans(requested),
                    layout.completed_bytes(requested),
                )
            }
            None => (0, 0),
        };
        self.add_counts(ReadCounts {
            record_calls: 1,
            spans: spans as u64,
            bytes: bytes as u64,
            ..ReadCounts::ZERO
        });
        completed
    }

    #[inline]
    fn execute_read_plan(&self, plan: &PreparedPlan, destination: &mut [u8]) -> Option<Outcome> {
        let result = self.0.execute_read_plan(plan, destination);
        let requested = match result {
            Some(Outcome::Complete) => plan.reads().len(),
            Some(Outcome::ControlChanged { completed_reads }) => completed_reads,
            Some(Outcome::ReadFailed { completed_reads }) => completed_reads.saturating_add(1),
            None => 0,
        };
        let (spans, bytes) = plan.requested_work(requested);
        self.add_counts(ReadCounts {
            plan_calls: 1,
            spans,
            bytes,
            ..ReadCounts::ZERO
        });
        result
    }

    fn immutable_view(&self, address: usize, len: usize) -> Option<SliceMemory<'_>> {
        self.0.immutable_view(address, len)
    }

    fn authenticated_countries(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::AuthenticatedRoster<'_>> {
        self.0.authenticated_countries(base, game_state)
    }

    fn authenticated_reserved_country(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<Result<crate::ccountry::Country, &'static str>> {
        self.0.authenticated_reserved_country(base, game_state)
    }

    fn authenticated_reserved_country_observation(
        &self,
        base: usize,
        game_state: usize,
    ) -> Option<crate::ccountry::ReservedCountryObservation> {
        self.0
            .authenticated_reserved_country_observation(base, game_state)
    }

    fn modifier_accumulator_value(
        &self,
        address: usize,
        key: u32,
    ) -> Option<Result<crate::modifier_accumulator::Value, &'static str>> {
        self.0.modifier_accumulator_value(address, key)
    }

    fn modifier_definitions(&self) -> Option<&crate::modifier_definition::PassDefinitions> {
        self.0.modifier_definitions()
    }

    fn core_supply_access(
        &self,
        base: usize,
        game_state: usize,
        owner_raw: u32,
        area: usize,
    ) -> Option<Result<bool, &'static str>> {
        self.0.core_supply_access(base, game_state, owner_raw, area)
    }

    fn movement_primary_root(
        &self,
        base: usize,
        member: usize,
    ) -> Option<Result<usize, &'static str>> {
        self.0.movement_primary_root(base, member)
    }
}

#[inline]
fn count_scattered<const N: usize>(destinations: &[&mut [u8]; N], completed: usize) -> ReadCounts {
    let mut counts = ReadCounts {
        scattered_calls: 1,
        ..ReadCounts::ZERO
    };
    for destination in destinations.iter().take(completed.saturating_add(1)) {
        counts.spans = counts
            .spans
            .saturating_add(u64::from(!destination.is_empty()));
        counts.bytes = counts.bytes.saturating_add(destination.len() as u64);
    }
    counts
}

#[cfg(test)]
mod tests;
