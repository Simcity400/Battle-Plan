use super::*;
use std::sync::Weak;

static EVENTS: AtomicUsize = AtomicUsize::new(0);
static SPANS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static SINGLE_SPANS: AtomicUsize = AtomicUsize::new(0);
static SINGLE_BYTES: AtomicUsize = AtomicUsize::new(0);
static SIZING: Mutex<()> = Mutex::new(());
static SCRATCH: Mutex<Pool> = Mutex::new(Pool::new());
static LOANED: AtomicUsize = AtomicUsize::new(0);

pub trait ScratchPoolAccounting: Send + Sync {
    fn replace_scratch_bytes(&self, previous: usize, next: usize);
}
struct Pool {
    entries: Vec<Scratch>,
    // Loans retain their charge; only off-thread replacement changes capacity.
    bytes: usize,
    accounting: Option<Weak<dyn ScratchPoolAccounting>>,
}
impl Pool {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            accounting: None,
        }
    }
    fn account_with(&mut self, accounting: Weak<dyn ScratchPoolAccounting>) {
        if let Some(previous) = self.accounting.as_ref().and_then(Weak::upgrade) {
            previous.replace_scratch_bytes(self.bytes, 0);
        }
        if let Some(next) = accounting.upgrade() {
            next.replace_scratch_bytes(0, self.bytes);
        }
        self.accounting = Some(accounting);
    }
    fn replace(&mut self, entries: &mut Vec<Scratch>, bytes: usize) {
        if let Some(accounting) = self.accounting.as_ref().and_then(Weak::upgrade) {
            accounting.replace_scratch_bytes(self.bytes, bytes);
        }
        self.bytes = bytes;
        std::mem::swap(&mut self.entries, entries);
    }
    fn release(&mut self) -> Vec<Scratch> {
        let mut retired = Vec::new();
        self.replace(&mut retired, 0);
        self.accounting = None;
        retired
    }
}
pub fn account_scratch(accounting: Weak<dyn ScratchPoolAccounting>) {
    SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .account_with(accounting);
}
fn allocated_bytes(entries: &Vec<Scratch>) -> usize {
    entries.capacity() * std::mem::size_of::<Scratch>()
        + entries
            .iter()
            .map(|scratch| {
                scratch.log.spans.capacity() * std::mem::size_of::<collector_raw::raw::Span>()
                    + scratch.log.bytes.capacity()
                    + scratch.interests.allocated_bytes()
            })
            .sum::<usize>()
}

pub(super) struct Scratch {
    pub log: RawLog<1>,
    pub interests: sources::Interests,
}
impl Scratch {
    fn clear(&mut self) {
        self.log.clear();
        self.interests.entries.clear();
        self.interests.organisations.clear();
        self.interests.organisation_pointers.clear();
    }
}
pub(super) struct Loan(Option<Scratch>);
impl std::ops::Deref for Loan {
    type Target = Scratch;
    fn deref(&self) -> &Scratch {
        self.0.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for Loan {
    fn deref_mut(&mut self) -> &mut Scratch {
        self.0.as_mut().unwrap()
    }
}
impl Drop for Loan {
    fn drop(&mut self) {
        let scratch = self.0.take().unwrap();
        let mut pool = SCRATCH
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        assert!(pool.entries.len() < pool.entries.capacity());
        pool.entries.push(scratch);
        LOANED.fetch_sub(1, Ordering::Relaxed);
    }
}
pub(super) fn scratch() -> Option<Loan> {
    let mut pool = SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = pool
        .entries
        .iter()
        .rposition(|scratch| !scratch.log.overflowed)?;
    let mut scratch = pool.entries.swap_remove(index);
    LOANED.fetch_add(1, Ordering::Relaxed);
    scratch.clear();
    Some(Loan(Some(scratch)))
}
pub(super) fn census(
    events: usize,
    spans: usize,
    bytes: usize,
    single_spans: usize,
    single_bytes: usize,
) {
    EVENTS.fetch_max(events, Ordering::Relaxed);
    SPANS.fetch_max(spans.saturating_mul(2), Ordering::Relaxed);
    BYTES.fetch_max(bytes.saturating_mul(2), Ordering::Relaxed);
    SINGLE_SPANS.fetch_max(single_spans, Ordering::Relaxed);
    SINGLE_BYTES.fetch_max(single_bytes, Ordering::Relaxed);
}
pub(super) fn occurrence(batch: &mut Batch, log: &RawLog<1>) {
    batch.required_spans = batch.required_spans.saturating_add(log.spans.len());
    batch.required_bytes = batch.required_bytes.saturating_add(log.bytes.len());
    SPANS.fetch_max(batch.required_spans.saturating_mul(2), Ordering::Relaxed);
    BYTES.fetch_max(batch.required_bytes.saturating_mul(2), Ordering::Relaxed);
    SINGLE_SPANS.fetch_max(log.spans.len(), Ordering::Relaxed);
    SINGLE_BYTES.fetch_max(log.bytes.len(), Ordering::Relaxed);
}
pub(super) fn completed(batch: &Batch) {
    EVENTS.fetch_max(batch.required_events.saturating_mul(2), Ordering::Relaxed);
    SPANS.fetch_max(batch.required_spans.saturating_mul(2), Ordering::Relaxed);
    BYTES.fetch_max(batch.required_bytes.saturating_mul(2), Ordering::Relaxed);
    if batch.log.overflowed {
        SPANS.fetch_max(
            batch.log.spans.capacity().saturating_mul(2).max(1),
            Ordering::Relaxed,
        );
        BYTES.fetch_max(
            batch.log.bytes.capacity().saturating_mul(2).max(1),
            Ordering::Relaxed,
        );
    }
}
pub(super) fn append(
    target: &mut RawLog<1>,
    source: &RawLog<1>,
) -> Result<(usize, usize), &'static str> {
    let bytes = target.bytes.len();
    let end = bytes
        .checked_add(source.bytes.len())
        .ok_or("variant change raw byte count overflow")?;
    if end > u32::MAX as usize
        || end > target.bytes.capacity()
        || source.spans.len() > target.spans.capacity().saturating_sub(target.spans.len())
    {
        return Err("variant change raw storage exhausted");
    }
    let start = target.spans.len();
    target.bytes.extend_from_slice(&source.bytes);
    target.spans.extend(source.spans.iter().map(|span| {
        let mut span = *span;
        if span.ok {
            span.start += bytes as u32;
        }
        span
    }));
    target.has_bulk |= source.has_bulk;
    target.raw_bytes[0] = target.raw_bytes[0].saturating_add(source.raw_bytes[0]);
    Ok((start, target.spans.len()))
}
fn adequate(batch: &Batch, events: usize, spans: usize, bytes: usize) -> bool {
    batch.events.capacity() >= events
        && batch.log.spans.capacity() >= spans
        && batch.log.bytes.capacity() >= bytes
        && !batch.log.overflowed
}
fn prepared(events: usize, spans: usize, bytes: usize) -> Batch {
    let mut batch = Batch::empty();
    batch.prepare(events, spans, bytes);
    batch
}
pub fn presize() {
    engine_allocation::advisory(presize_inner);
}
fn presize_inner() {
    let Ok(_sizing) = SIZING.try_lock() else {
        return;
    };
    let Some(store) = STORE.get() else {
        return;
    };
    let (events, spans, bytes, active, spare) = {
        let store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let events = EVENTS
            .load(Ordering::Relaxed)
            .max(store.batch.required_events)
            .max(store.spare.required_events);
        let mut spans = SPANS
            .load(Ordering::Relaxed)
            .max(events.saturating_mul(SINGLE_SPANS.load(Ordering::Relaxed)));
        let mut bytes = BYTES
            .load(Ordering::Relaxed)
            .max(events.saturating_mul(SINGLE_BYTES.load(Ordering::Relaxed)));
        for batch in [&store.batch, &store.spare] {
            if batch.log.overflowed {
                spans = spans.max(batch.log.spans.capacity().saturating_mul(2));
                bytes = bytes.max(batch.log.bytes.capacity().saturating_mul(2));
            }
        }
        (
            events,
            spans,
            bytes,
            store.batch.events.is_empty()
                && store.batch.failure.is_none()
                && !adequate(&store.batch, events, spans, bytes),
            store.spare.events.is_empty()
                && store.spare.failure.is_none()
                && !adequate(&store.spare, events, spans, bytes),
        )
    };
    let mut replacements = [
        active.then(|| prepared(events, spans, bytes)),
        spare.then(|| prepared(events, spans, bytes)),
    ];
    {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Store { batch, spare, .. } = &mut *store;
        for (index, batch) in [batch, spare].into_iter().enumerate() {
            if batch.events.is_empty()
                && batch.failure.is_none()
                && !adequate(batch, events, spans, bytes)
                && let Some(replacement) = &mut replacements[index]
                && adequate(replacement, events, spans, bytes)
            {
                std::mem::swap(batch, replacement);
            }
        }
    }
    // Retired arenas and their committed pages are released outside STORE.
    drop(replacements);
    let spans = SINGLE_SPANS.load(Ordering::Relaxed);
    let bytes = SINGLE_BYTES.load(Ordering::Relaxed);
    if spans == 0 || bytes == 0 {
        return;
    }
    let lanes = std::thread::available_parallelism().map_or(1, usize::from);
    let mut prepared = {
        let mut pool = SCRATCH
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if LOANED.load(Ordering::Relaxed) != 0
            || (pool.entries.len() >= lanes
                && pool.entries.iter().all(|scratch| {
                    !scratch.log.overflowed
                        && scratch.log.spans.capacity() >= spans
                        && scratch.log.bytes.capacity() >= bytes
                }))
        {
            return;
        }
        std::mem::take(&mut pool.entries)
    };
    let additional = lanes.saturating_sub(prepared.len());
    if engine_allocation::reserve(&mut prepared, additional).is_ok() {
        while prepared.len() < lanes {
            prepared.push(Scratch {
                log: RawLog::new(0, 0),
                interests: sources::Interests::default(),
            });
        }
    }
    for scratch in &mut prepared {
        scratch.log.grow();
        scratch.clear();
        scratch.log.presize(spans, bytes);
        let _ = scratch.interests.prepare(spans);
        let _ = scratch.interests.prepare_organisations(spans);
    }
    let bytes = allocated_bytes(&prepared);
    let mut pool = SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pool.replace(&mut prepared, bytes);
    drop(pool);
    drop(prepared);
}

pub struct SetupPart {
    ordinal: usize,
    work: SetupWork,
}
enum SetupWork {
    Batch {
        events: usize,
        spans: usize,
        bytes: usize,
        output: Option<Batch>,
    },
    Scratch {
        spans: usize,
        bytes: usize,
        entries: Vec<Scratch>,
    },
}
impl SetupPart {
    pub fn capture(&mut self) {
        let _phase = profile::Timer::new(profile::Phase::ColdStorageParts);
        engine_allocation::advisory(|| match &mut self.work {
            SetupWork::Batch {
                events,
                spans,
                bytes,
                output,
            } => {
                *output = Some(prepared(*events, *spans, *bytes));
            }
            SetupWork::Scratch {
                spans,
                bytes,
                entries,
            } => {
                for scratch in entries {
                    scratch.log.grow();
                    scratch.clear();
                    scratch.log.presize(*spans, *bytes);
                    let _ = scratch.interests.prepare(*spans);
                    let _ = scratch.interests.prepare_organisations(*spans);
                }
            }
        });
    }
}
pub(in crate::production) fn presize_partitioned(
    lanes: usize,
    acquire: &mut impl FnMut(Vec<SetupPart>) -> Result<Vec<SetupPart>, &'static str>,
) {
    let _phase = profile::Timer::new(profile::Phase::ColdStorage);
    engine_allocation::advisory(|| presize_with(lanes, acquire));
}
fn presize_with(
    worker_lanes: usize,
    acquire: &mut impl FnMut(Vec<SetupPart>) -> Result<Vec<SetupPart>, &'static str>,
) {
    let Ok(_sizing) = SIZING.try_lock() else {
        return;
    };
    let Some(store) = STORE.get() else {
        return;
    };
    let (events, spans, bytes, active, spare) = {
        let store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let events = EVENTS
            .load(Ordering::Relaxed)
            .max(store.batch.required_events)
            .max(store.spare.required_events);
        let mut spans = SPANS
            .load(Ordering::Relaxed)
            .max(events.saturating_mul(SINGLE_SPANS.load(Ordering::Relaxed)));
        let mut bytes = BYTES
            .load(Ordering::Relaxed)
            .max(events.saturating_mul(SINGLE_BYTES.load(Ordering::Relaxed)));
        for batch in [&store.batch, &store.spare] {
            if batch.log.overflowed {
                spans = spans.max(batch.log.spans.capacity().saturating_mul(2));
                bytes = bytes.max(batch.log.bytes.capacity().saturating_mul(2));
            }
        }
        (
            events,
            spans,
            bytes,
            store.batch.events.is_empty()
                && store.batch.failure.is_none()
                && !adequate(&store.batch, events, spans, bytes),
            store.spare.events.is_empty()
                && store.spare.failure.is_none()
                && !adequate(&store.spare, events, spans, bytes),
        )
    };
    let Ok(mut parts) = engine_allocation::vec(2) else {
        return;
    };
    for (ordinal, needed) in [active, spare].into_iter().enumerate() {
        if needed {
            parts.push(SetupPart {
                ordinal,
                work: SetupWork::Batch {
                    events,
                    spans,
                    bytes,
                    output: None,
                },
            });
        }
    }
    let Ok(parts) = acquire(parts) else { return };
    let mut replacements = [None, None];
    for part in parts {
        if let SetupWork::Batch { output, .. } = part.work {
            replacements[part.ordinal] = output;
        }
    }
    {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Store { batch, spare, .. } = &mut *store;
        for (index, batch) in [batch, spare].into_iter().enumerate() {
            if batch.events.is_empty()
                && batch.failure.is_none()
                && !adequate(batch, events, spans, bytes)
                && let Some(replacement) = &mut replacements[index]
                && adequate(replacement, events, spans, bytes)
            {
                std::mem::swap(batch, replacement);
            }
        }
    }
    // Retired arenas and their committed pages are released outside STORE.
    drop(replacements);
    let spans = SINGLE_SPANS.load(Ordering::Relaxed);
    let bytes = SINGLE_BYTES.load(Ordering::Relaxed);
    if spans == 0 || bytes == 0 {
        return;
    }
    let lanes = std::thread::available_parallelism().map_or(1, usize::from);
    let mut prepared = {
        let mut pool = SCRATCH
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if LOANED.load(Ordering::Relaxed) != 0
            || (pool.entries.len() >= lanes
                && pool.entries.iter().all(|scratch| {
                    !scratch.log.overflowed
                        && scratch.log.spans.capacity() >= spans
                        && scratch.log.bytes.capacity() >= bytes
                }))
        {
            return;
        }
        std::mem::take(&mut pool.entries)
    };
    struct Charge;
    impl Drop for Charge {
        fn drop(&mut self) {
            let mut pool = SCRATCH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let bytes = allocated_bytes(&pool.entries);
            if let Some(accounting) = pool.accounting.as_ref().and_then(Weak::upgrade) {
                accounting.replace_scratch_bytes(pool.bytes, bytes);
            }
            pool.bytes = bytes;
        }
    }
    let _charge = Charge;
    let additional = lanes.saturating_sub(prepared.len());
    if engine_allocation::reserve(&mut prepared, additional).is_ok() {
        while prepared.len() < lanes {
            prepared.push(Scratch {
                log: RawLog::new(0, 0),
                interests: sources::Interests::default(),
            });
        }
    }
    let width = prepared.len().div_ceil(worker_lanes.max(1)).max(1);
    let Ok(mut parts) = engine_allocation::vec(prepared.len().div_ceil(width)) else {
        let mut pool = SCRATCH
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let bytes = allocated_bytes(&prepared);
        pool.replace(&mut prepared, bytes);
        return;
    };
    for ordinal in 0..prepared.len().div_ceil(width) {
        let Ok(entries) = engine_allocation::vec(width) else {
            let mut pool = SCRATCH
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let bytes = allocated_bytes(&prepared);
            pool.replace(&mut prepared, bytes);
            return;
        };
        parts.push(SetupPart {
            ordinal,
            work: SetupWork::Scratch {
                spans,
                bytes,
                entries,
            },
        });
    }
    let mut source = prepared.drain(..);
    for part in &mut parts {
        let SetupWork::Scratch { entries, .. } = &mut part.work else {
            unreachable!()
        };
        entries.extend(source.by_ref().take(width));
    }
    drop(source);
    let Ok(mut parts) = acquire(parts) else {
        return;
    };
    parts.sort_unstable_by_key(|part| part.ordinal);
    for part in parts {
        if let SetupWork::Scratch { mut entries, .. } = part.work {
            prepared.append(&mut entries);
        }
    }
    let bytes = allocated_bytes(&prepared);
    let mut pool = SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    pool.replace(&mut prepared, bytes);
    drop(pool);
    drop(prepared);
}

pub(super) fn recycle(mut batch: Batch) {
    let Some(store) = STORE.get() else {
        return;
    };
    {
        let mut store = store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if store.spare.events.capacity() <= batch.events.capacity()
            && store.spare.log.spans.capacity() <= batch.log.spans.capacity()
            && store.spare.log.bytes.capacity() <= batch.log.bytes.capacity()
        {
            std::mem::swap(&mut store.spare, &mut batch);
        }
    }
    drop(batch);
}
pub(super) fn release() {
    let mut pool = SCRATCH
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    assert_eq!(LOANED.load(Ordering::Relaxed), 0);
    let retired = pool.release();
    drop(pool);
    drop(retired);
    for counter in [&EVENTS, &SPANS, &BYTES, &SINGLE_SPANS, &SINGLE_BYTES] {
        counter.store(0, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_raw::raw::Recorded;

    struct Accounting(AtomicU64);
    impl ScratchPoolAccounting for Accounting {
        fn replace_scratch_bytes(&self, previous: usize, next: usize) {
            if next >= previous {
                self.0
                    .fetch_add((next - previous) as u64, Ordering::Relaxed);
            } else {
                self.0
                    .fetch_sub((previous - next) as u64, Ordering::Relaxed);
            }
        }
    }
    #[test]
    fn retained_scratch_charges_containers_arenas_and_interests_through_loans_and_release() {
        let mut entries = Vec::with_capacity(3);
        let mut interests = sources::Interests::default();
        interests.prepare(7).unwrap();
        interests.prepare_organisations(11).unwrap();
        entries.push(Scratch {
            log: RawLog::new(13, 17),
            interests,
        });
        let expected = entries.capacity() * std::mem::size_of::<Scratch>()
            + entries[0].log.spans.capacity() * std::mem::size_of::<collector_raw::raw::Span>()
            + entries[0].log.bytes.capacity()
            + entries[0].interests.allocated_bytes();
        let mut pool = Pool::new();
        pool.replace(&mut entries, expected);
        let counter = std::sync::Arc::new(Accounting(AtomicU64::new(31)));
        let accounting = std::sync::Arc::downgrade(&counter);
        pool.account_with(accounting);
        assert_eq!(counter.0.load(Ordering::Relaxed), 31 + expected as u64);
        let mut loan = pool.entries.pop().unwrap();
        loan.clear();
        assert_eq!(counter.0.load(Ordering::Relaxed), 31 + expected as u64);
        collector_test_support::with_allocations_denied(|| pool.entries.push(loan));
        assert_eq!(allocated_bytes(&pool.entries), expected);
        let mut replacement = std::mem::take(&mut pool.entries);
        replacement.reserve(9);
        replacement[0].log.presize(101, 1009);
        replacement[0].interests.prepare(103).unwrap();
        replacement[0].interests.prepare_organisations(107).unwrap();
        let grown = allocated_bytes(&replacement);
        assert!(grown > expected);
        pool.replace(&mut replacement, grown);
        drop(replacement);
        assert_eq!(counter.0.load(Ordering::Relaxed), 31 + grown as u64);
        let replacement_counter = std::sync::Arc::new(Accounting(AtomicU64::new(43)));
        let accounting = std::sync::Arc::downgrade(&replacement_counter);
        pool.account_with(accounting);
        assert_eq!(counter.0.load(Ordering::Relaxed), 31);
        assert_eq!(
            replacement_counter.0.load(Ordering::Relaxed),
            43 + grown as u64
        );
        let mut empty = Vec::with_capacity(2);
        let container = allocated_bytes(&empty);
        assert_eq!(container, empty.capacity() * std::mem::size_of::<Scratch>());
        pool.replace(&mut empty, container);
        drop(empty);
        assert_eq!(
            replacement_counter.0.load(Ordering::Relaxed),
            43 + container as u64
        );
        drop(pool.release());
        assert!(pool.entries.is_empty() && pool.entries.capacity() == 0);
        assert!(pool.accounting.is_none());
        assert_eq!(pool.bytes, 0);
        assert_eq!(replacement_counter.0.load(Ordering::Relaxed), 43);
    }
    #[test]
    fn spliced_snapshots_keep_occurrence_bytes_after_live_memory_changes() {
        let mut source = RawLog::<1>::new(4, 8);
        let mut target = RawLog::<1>::new(8, 16);
        let mut ranges = Vec::new();
        for value in [11_u64, 29] {
            source.clear();
            let bytes = value.to_le_bytes();
            let memory = engine_native::memory::SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            };
            source.take_bulk(&memory, 0x1000, 8, 1).unwrap();
            ranges.push(
                collector_test_support::with_allocations_denied(|| append(&mut target, &source))
                    .unwrap(),
            );
        }
        for ((start, end), value) in ranges.into_iter().zip([11_u64, 29]) {
            let recorded = Recorded::build_range(&target, start..end).unwrap();
            assert_eq!(engine_native::read::<u64>(&recorded, 0x1000), Some(value));
        }
        let before = (target.spans.len(), target.bytes.len());
        assert_eq!(
            append(&mut target, &source),
            Err("variant change raw storage exhausted")
        );
        assert_eq!((target.spans.len(), target.bytes.len()), before);
    }
    #[test]
    fn refused_burst_is_explicit_and_sizes_both_logs_before_the_next_interval() {
        struct Scalar;
        impl Memory for Scalar {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == engine_native::cgame_state::SINGLETON_RVA.0 && output.len() == 8 {
                    output.copy_from_slice(&0x2000_u64.to_le_bytes());
                } else if address == 0x2000 + engine_native::cgame_state::CURRENT_HOUR
                    && output.len() == 4
                {
                    output.copy_from_slice(&7_i32.to_le_bytes());
                } else if address == 0x1000 + native::AUTO_UPGRADED && output.len() == 1 {
                    output[0] = 1;
                } else {
                    return false;
                }
                true
            }
        }
        let mut batch = Batch::empty();
        batch.prepare(8, 1, 1);
        let mut watched = HashMap::new();
        watched.insert(
            0x1000,
            Watch {
                id: 3,
                pointer: 0x1000,
                flags: [0; 3],
                dirty: false,
            },
        );
        assert!(
            STORE
                .set(Mutex::new(Store {
                    batch,
                    spare: Batch::empty(),
                    watched,
                    interests: sources::Interests::default(),
                    epoch: EPOCH.load(Ordering::Acquire)
                }))
                .is_ok()
        );
        ENABLED.store(true, Ordering::Release);
        RECOVER.store(false, Ordering::Release);
        census(8, 1, 1, 1, 1);
        collector_test_support::with_allocations_denied(|| {
            for _ in 0..3 {
                scalar::observe(&Scalar, 0, 0x1000, scalar::Field::AutoUpgraded);
            }
        });
        let mut output = Vec::new();
        assert_eq!(
            drain_with(&Scalar, 0, 7, &mut output),
            Err("variant change raw storage exhausted")
        );
        assert!(output.is_empty());
        assert_eq!(status().refused_intervals, 1);
        assert!(needs_full());
        presize();
        let store = STORE.get().unwrap().lock().unwrap();
        for batch in [&store.batch, &store.spare] {
            assert!(adequate(batch, 8, 8, 8));
        }
        drop(store);
        let mut scratch = scratch().unwrap();
        let spans = scratch.log.spans.capacity();
        let bytes = scratch.log.bytes.capacity();
        collector_test_support::with_allocations_denied(|| {
            scratch
                .log
                .take_bulk(&Scalar, 0x1000 + native::AUTO_UPGRADED, 1, 1)
                .unwrap();
        });
        assert_eq!(
            (scratch.log.spans.capacity(), scratch.log.bytes.capacity()),
            (spans, bytes)
        );
    }
}

#[cfg(test)]
mod cold_setup_tests {
    use super::*;
    fn parts(countries: usize) -> Vec<SetupPart> {
        vec![
            SetupPart {
                ordinal: 0,
                work: SetupWork::Batch {
                    events: countries * 2,
                    spans: countries * 21,
                    bytes: countries * 64,
                    output: None,
                },
            },
            SetupPart {
                ordinal: 1,
                work: SetupWork::Scratch {
                    spans: countries,
                    bytes: countries * 64,
                    entries: (0..7)
                        .map(|_| Scratch {
                            log: RawLog::new(0, 0),
                            interests: sources::Interests::default(),
                        })
                        .collect(),
                },
            },
        ]
    }
    fn demands(parts: &[SetupPart]) -> Vec<(usize, usize, usize, usize)> {
        parts
            .iter()
            .map(|part| match &part.work {
                SetupWork::Batch { output, .. } => {
                    let batch = output.as_ref().unwrap();
                    (
                        part.ordinal,
                        batch.events.capacity(),
                        batch.log.spans.capacity(),
                        batch.log.bytes.capacity(),
                    )
                }
                SetupWork::Scratch { entries, .. } => (
                    part.ordinal,
                    entries.len(),
                    entries.iter().map(|s| s.log.spans.capacity()).sum(),
                    entries.iter().map(|s| s.log.bytes.capacity()).sum(),
                ),
            })
            .collect()
    }
    #[test]
    fn cold_storage_parts_preserve_serial_capacity_for_uneven_and_large_censuses() {
        for countries in [67, 1500] {
            let mut expected = parts(countries);
            for part in &mut expected {
                part.capture();
            }
            let mut actual = std::thread::scope(|scope| {
                let jobs: Vec<_> = parts(countries)
                    .into_iter()
                    .map(|mut part| {
                        scope.spawn(move || {
                            part.capture();
                            part
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .rev()
                    .map(|job| job.join().unwrap())
                    .collect::<Vec<_>>()
            });
            actual.sort_unstable_by_key(|part| part.ordinal);
            assert_eq!(demands(&actual), demands(&expected));
        }
    }
    #[test]
    fn optional_cold_storage_denial_never_calls_the_terminal_hook() {
        const NAME: &str = "optional_cold_storage_denial_never_calls_the_terminal_hook";
        const ENV: &str = "BATTLE_PLAN_COLD_STORAGE_TEST";
        if std::env::var(ENV).ok().as_deref() != Some(NAME) {
            assert!(
                std::process::Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        &format!("production::design_changes::storage::cold_setup_tests::{NAME}")
                    ])
                    .env(ENV, NAME)
                    .status()
                    .unwrap()
                    .success()
            );
            return;
        }
        fn failed() {
            panic!("optional scratch allocation became terminal")
        }
        let mut parts = parts(67);
        engine_allocation::set_hooks(None, Some(failed));
        collector_test_support::with_allocations_denied(|| {
            for part in &mut parts {
                part.capture();
            }
        });
        engine_allocation::set_hooks(None, None);
        assert_eq!(demands(&parts)[0].1, 0);
    }
}
