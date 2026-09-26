//! Shared span-log raw-copy primitive for the decode-worker rollout.
//!
//! Each moved family stages one refresh of guarded reads into a `RawLog`:
//! exact (address, length) spans with success flags, affected-table masks
//! and byte pools. The game thread never decodes, sorts or compares rows; it
//! only copies bytes and parses locally what the next read address is. The
//! single FIFO decode worker serves the log through `Recorded` (a `Memory`
//! impl failing every unrecorded read) and replays the family's existing
//! typed walk. Correctness requires staging every read that walk needs and
//! preserving its failure prefix; synthetic-memory tests and fresh-save
//! comparisons validate those contracts.
//!
//! Mirror rules (exact address/length keys retain their observation order,
//! including failures; bulk spans additionally serve contained requests):
//! - reads served locally by an authenticated view need no log entry beyond
//!   the view's own spans;
//! - pooled row and arena capacities are ignored while copying (a superset
//!   is recorded and the replay truncates identically against the real
//!   pooled buffers);
//! - a failed or unparsable span stops only the sub-path that needs it; the
//!   replay fails identically from the same recorded spans, so later extras
//!   are harmless;
//! - pool overflow aborts the copy and grows staging capacity on recycle.
//!   The caller may use the typed fallback only before a prepared source
//!   observation; afterward, that capture fails without repeating live reads.

use crate::raw_plan::{Attempt, CapturePlan, Observations, Request};
use engine::Memory;
use std::cell::RefCell;
use std::hash::Hasher;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

// Worker-side index preparation only. No timers or atomic updates are added
// to memory lookups or to the game-thread copy path.
static INDEX_REUSES: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILDS: AtomicU64 = AtomicU64::new(0);
static INDEX_REUSE_NS: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILD_NS: AtomicU64 = AtomicU64::new(0);
static INDEX_REUSED_SPANS: AtomicU64 = AtomicU64::new(0);
static INDEX_REBUILT_SPANS: AtomicU64 = AtomicU64::new(0);

pub(crate) fn read_plan_diagnostics() -> serde_json::Value {
    crate::raw_plan::diagnostics()
}

pub(crate) fn index_diagnostics() -> serde_json::Value {
    serde_json::json!({
        "scope": "shared raw indexes since attach; preparation only, not row decoding",
        "reuses": INDEX_REUSES.load(Ordering::Relaxed),
        "rebuilds": INDEX_REBUILDS.load(Ordering::Relaxed),
        "reuse_ns": INDEX_REUSE_NS.load(Ordering::Relaxed),
        "rebuild_ns": INDEX_REBUILD_NS.load(Ordering::Relaxed),
        "reused_spans": INDEX_REUSED_SPANS.load(Ordering::Relaxed),
        "rebuilt_spans": INDEX_REBUILT_SPANS.load(Ordering::Relaxed),
    })
}

static GROWTH_DIAGNOSTICS: Mutex<std::collections::BTreeMap<&'static str, serde_json::Value>> =
    Mutex::new(std::collections::BTreeMap::new());

/// Worker-only capacity diagnostics: distinguish genuine demand from a mirror
/// following unreadable bytes. Never allocate or take this lock in the callback.
pub(crate) fn note_growth<const N: usize>(family: &'static str, log: &RawLog<N>) {
    if !log.overflowed {
        return;
    }
    let failed = log.spans.iter().find(|span| !span.ok);
    let value = serde_json::json!({
        "spans": log.spans.len(), "span_capacity": log.spans.capacity(),
        "bytes": log.bytes.len(), "byte_capacity": log.bytes.capacity(),
        "faults": log.fault.as_slice(), "growing": log.grow_for_next(),
        "first_failed_address": failed.map(|span| span.addr),
        "first_failed_length": failed.map(|span| span.len),
        "first_failed_mask": failed.map(|span| span.mask),
        "last_address": log.spans.last().map(|span| span.addr),
        "last_length": log.spans.last().map(|span| span.len),
        "last_mask": log.spans.last().map(|span| span.mask),
    });
    publish_growth(family, value);
}

/// Publish an equivalent worker-owned packet capacity diagnostic.
pub(crate) fn publish_growth(family: &'static str, value: serde_json::Value) {
    if let Ok(mut diagnostics) = GROWTH_DIAGNOSTICS.lock() {
        diagnostics.insert(family, value);
    }
}

pub(crate) fn growth_diagnostics() -> serde_json::Value {
    GROWTH_DIAGNOSTICS
        .lock()
        .map(|rows| serde_json::json!(*rows))
        .unwrap_or_default()
}

/// Increase capacity even when an oversized first request left the vector
/// empty. `reserve_exact(capacity)` alone is a no-op after `clear()`.
pub(crate) fn double_capacity<T>(rows: &mut Vec<T>) {
    let target = rows.capacity().max(1).saturating_mul(2);
    rows.reserve_exact(target.saturating_sub(rows.len()));
}

/// x86-64 canonical range check, identical to `engine::memory`'s gate.
///
/// Duplicated (not imported) so this crate needs no new public engine API;
/// the two implementations are proved equal by mirrored unit vectors in both
/// crates. Noncanonical sources fault with `ExceptionInformation[1] == -1`,
/// which `copy_fault` cannot attribute to the requested range. Gating here
/// records the same graceful fault the typed walk produces for an unreadable
/// source, before any guarded instruction runs. Empty spans succeed without
/// a check, matching `Memory`. The Rust gate makes the C `-1` path
/// unreachable through `take_inner`/`LiveMemory`; the C fix remains as
/// containment for direct FFI callers.
#[inline]
pub(crate) fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    // Bit 47 is part of the sign extension, not an independent address bit.
    let high = addr >> 47;
    matches!(high, 0 | 0x1FFFF) && end >> 47 == high
}

/// One recorded guarded read: exact address and length, success flag, table
/// mask, byte-pool offset for successful spans, and whether this span is a
/// bulk chunk whose sub-ranges the replay may request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub addr: usize,
    pub len: u32,
    pub ok: bool,
    pub mask: u32,
    pub start: u32,
    pub bulk: bool,
}

/// Fully initialized reusable storage with a separately published byte prefix.
/// Spare bytes may contain an earlier capture or a failed partial copy; only
/// the successfully copied logical prefix is exposed through slice operations.
/// Initialization and capacity growth happen off the game thread.
pub struct RawBytes {
    storage: Vec<u8>,
    used: usize,
    observations: Option<RefCell<Observations>>,
}

impl RawBytes {
    fn with_capacity(capacity: usize) -> Self {
        let mut storage = vec![0; capacity];
        // Initialize the actual allocation, including any allocator headroom.
        storage.resize(storage.capacity(), 0);
        Self {
            storage,
            used: 0,
            observations: None,
        }
    }

    #[inline(always)]
    pub fn len(&self) -> usize {
        self.used
    }

    #[inline(always)]
    pub fn is_empty(&self) -> bool {
        self.used == 0
    }

    #[inline]
    fn observe<T: ?Sized>(&self, value: &T) {
        if let Some(observations) = &self.observations {
            let mut observations = observations.borrow_mut();
            if observations.active {
                let start =
                    (value as *const T).cast::<u8>() as usize - self.storage.as_ptr() as usize;
                let len = std::mem::size_of_val(value);
                observations.observe(start, &self.storage[start..start + len]);
            }
        }
    }

    #[inline]
    fn untracked(&self) {
        if let Some(observations) = &self.observations {
            observations.borrow_mut().escape();
        }
    }

    pub fn get<I: std::slice::SliceIndex<[u8]>>(&self, index: I) -> Option<&I::Output> {
        let value = self.storage[..self.used].get(index)?;
        self.observe(value);
        Some(value)
    }

    pub fn capacity(&self) -> usize {
        self.storage.capacity()
    }

    fn clear(&mut self) {
        self.used = 0;
        if let Some(observations) = &mut self.observations {
            observations.get_mut().active = false;
        }
    }

    /// Worker-only growth. The logical prefix and existing bytes survive.
    fn grow_to(&mut self, capacity: usize) {
        if capacity > self.storage.capacity() {
            self.storage.reserve_exact(capacity - self.storage.len());
            self.storage.resize(self.storage.capacity(), 0);
        }
    }

    /// Append already-owned bytes within a prepared arena. The lexical
    /// exceptional decoder uses this after reserving the exact payload size.
    /// This never allocates and never publishes an unfilled destination.
    pub fn extend_from_slice(&mut self, bytes: &[u8]) {
        self.untracked();
        let end = self
            .used
            .checked_add(bytes.len())
            .expect("raw byte length overflow");
        self.storage[self.used..end].copy_from_slice(bytes);
        self.used = end;
    }

    #[cfg(test)]
    fn resize(&mut self, len: usize, value: u8) {
        self.grow_to(len);
        if len > self.used {
            self.storage[self.used..len].fill(value);
        }
        self.used = len;
    }

    #[cfg(test)]
    fn truncate(&mut self, len: usize) {
        self.used = self.used.min(len);
    }
}

impl std::ops::Deref for RawBytes {
    type Target = [u8];

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        self.untracked();
        &self.storage[..self.used]
    }
}

impl std::ops::DerefMut for RawBytes {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.untracked();
        &mut self.storage[..self.used]
    }
}

impl<I: std::slice::SliceIndex<[u8]>> std::ops::Index<I> for RawBytes {
    type Output = I::Output;

    #[inline]
    fn index(&self, index: I) -> &Self::Output {
        let value = &self.storage[..self.used][index];
        self.observe(value);
        value
    }
}

impl<I: std::slice::SliceIndex<[u8]>> std::ops::IndexMut<I> for RawBytes {
    #[inline]
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        self.untracked();
        &mut self.storage[..self.used][index]
    }
}

impl std::fmt::Debug for RawBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}

impl AsRef<[u8]> for RawBytes {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl<T: AsRef<[u8]> + ?Sized> PartialEq<T> for RawBytes {
    fn eq(&self, other: &T) -> bool {
        self.as_ref() == other.as_ref()
    }
}

impl Eq for RawBytes {}

/// One refresh of raw payload for a family of at most 32 tables. Flat pools
/// keep every span fixed-size so `clear` recycles capacities and the steady
/// state allocates on neither thread; growth happens on the decode side only.
#[derive(Debug)]
pub struct RawLog<const N: usize> {
    pub spans: Vec<Span>,
    pub bytes: RawBytes,
    pub fault: [bool; N],
    pub raw_bytes: [u64; N],
    pub overflowed: bool,
    span_exhausted: bool,
    required_bytes: usize,
    /// Set when the log holds bulk spans wider than the replay's requests
    /// (contiguous table copies). The decode side then serves sub-ranges
    /// from the enclosing span instead of exact matches only.
    pub has_bulk: bool,
    // Address metadata only, never game bytes. Taken and returned by the
    // decode worker; clear() on the copy side leaves it untouched.
    index: RefCell<Option<RecordedIndex>>,
    plan: CapturePlan<N>,
    /// A complete plan loans its immutable span vector until decode/recycle.
    planned_spans_loaned: bool,
    /// Most recent prepared backend wall time; excludes continuation and span restoration.
    pub(crate) backend_ns: u64,
    capacity_charge: crate::raw_memory::Charge,
}

impl<const N: usize> Default for RawLog<N> {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<const N: usize> RawLog<N> {
    /// Cold constructor; steady state reuses pooled jobs through the
    /// decode-worker acquire/recycle pair, so this never runs per refresh.
    pub fn new(span_cap: usize, byte_cap: usize) -> Self {
        debug_assert!(N <= 32, "span masks cover at most 32 tables");
        let mut log = Self {
            spans: Vec::with_capacity(span_cap),
            bytes: RawBytes::with_capacity(byte_cap),
            fault: [false; N],
            raw_bytes: [0; N],
            overflowed: false,
            span_exhausted: false,
            required_bytes: 0,
            has_bulk: false,
            index: RefCell::new(None),
            plan: CapturePlan::default(),
            planned_spans_loaned: false,
            backend_ns: 0,
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        log.update_capacity_charge();
        log
    }

    fn update_capacity_charge(&mut self) {
        self.capacity_charge.set(crate::raw_memory::Capacity {
            payload_vectors: crate::raw_memory::vector_bytes(&self.spans)
                + self.bytes.capacity() as u64,
            auxiliary_vectors: self.plan.capacity_bytes()
                + self
                    .bytes
                    .observations
                    .as_ref()
                    .map_or(0, |observations| observations.borrow().capacity_bytes()),
            ..Default::default()
        });
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.return_planned_spans();
        self.spans.clear();
        self.backend_ns = 0;
        self.bytes.clear();
        self.fault = [false; N];
        self.raw_bytes = [0; N];
        self.overflowed = false;
        self.span_exhausted = false;
        self.required_bytes = 0;
        self.has_bulk = false;
        self.plan.active = false;
        self.plan.sealed = false;
        self.plan.blocked = false;
        self.plan.source_observed = false;
        self.plan.attempt = Attempt::None;
    }

    /// Opt in off-thread. Trace exhaustion changes only optimization eligibility.
    pub(crate) fn initialize_read_plan(
        &mut self,
        read_capacity: usize,
        control_capacity: usize,
        control_byte_capacity: usize,
    ) {
        self.plan.enabled = true;
        self.plan.requests = Vec::with_capacity(read_capacity);
        self.bytes.observations = Some(RefCell::new(Observations::new(
            control_capacity,
            control_byte_capacity,
        )));
        self.update_capacity_charge();
    }

    /// The producer opts in only after its scattered native observations and
    /// auxiliary state have the same complete-plan/ordered-resume contract.
    pub(crate) fn initialize_scattered_read_plan(
        &mut self,
        reads: usize,
        controls: usize,
        bytes: usize,
    ) {
        self.initialize_read_plan(reads, controls, bytes);
        self.plan.scattered = true;
    }

    /// Begin on an empty log with the same fresh provider used by the producer.
    /// False requires running that producer before finish_planned_capture().
    pub(crate) fn begin_planned_capture(&mut self, memory: &impl Memory, key: [usize; 4]) -> bool {
        if !self.plan.enabled {
            return false;
        }
        self.plan.begin(key);
        self.bytes
            .observations
            .as_mut()
            .expect("enabled trace storage")
            .get_mut()
            .begin();
        if !self.spans.is_empty() || !self.bytes.is_empty() || self.overflowed {
            self.plan.block();
            self.fault.fill(true);
            self.bytes.observations.as_mut().unwrap().get_mut().eligible = false;
            return false;
        }
        let Some(compiled) = &mut self.plan.compiled else {
            return false;
        };
        if self.plan.retire
            || compiled.key != key
            || compiled.spans.len() > self.spans.capacity()
            || compiled.plan.destination_len() > self.bytes.capacity()
        {
            self.plan.retire = true;
            return false;
        }
        let backend_started = std::time::Instant::now();
        let outcome = memory.execute_read_plan(&compiled.plan, &mut self.bytes.storage);
        let backend_elapsed = backend_started.elapsed();
        self.backend_ns = backend_elapsed.as_nanos().min(u64::MAX as u128) as u64;
        CapturePlan::<N>::note_backend_elapsed(backend_elapsed);
        let Some(outcome) = outcome else {
            // Unsupported execution guarantees that no source read occurred.
            return false;
        };
        use engine::read_plan::Outcome;
        match outcome {
            Outcome::Complete => {
                let completed = compiled.requests.len();
                self.bytes.used = compiled.plan.destination_len();
                self.has_bulk = compiled.has_bulk;
                self.raw_bytes = compiled.raw_bytes;
                // The geometry is immutable for this successful acquisition.
                // Loan it to owned decoding rather than copying every span on
                // the game critical path. Recycle returns the same allocation.
                std::mem::swap(&mut self.spans, &mut compiled.spans);
                self.planned_spans_loaned = true;
                self.plan.attempt = Attempt::Complete;
                self.plan.note_execution(completed, false);
                self.bytes.observations.as_mut().unwrap().get_mut().active = false;
                true
            }
            Outcome::ControlChanged { completed_reads } => {
                if completed_reads > compiled.requests.len() {
                    self.plan.block();
                    self.fault.fill(true);
                } else {
                    self.plan.prefix_len = completed_reads;
                    self.plan.attempt = Attempt::Changed;
                    self.plan.retire = true;
                    self.plan.note_execution(completed_reads, false);
                }
                false
            }
            Outcome::ReadFailed { completed_reads } => {
                if completed_reads >= compiled.requests.len() {
                    self.plan.block();
                    self.fault.fill(true);
                } else {
                    self.plan.prefix_len = completed_reads + 1;
                    self.plan.failed_read = Some(completed_reads);
                    self.plan.attempt = Attempt::Faulted;
                    self.plan.retire = true;
                    self.plan.note_execution(completed_reads, true);
                }
                false
            }
        }
    }

    pub(crate) fn finish_planned_capture(&mut self) {
        if !self.plan.enabled || !self.plan.active {
            return;
        }
        let observations = self.bytes.observations.as_mut().unwrap().get_mut();
        observations.active = false;
        let eligible =
            observations.eligible && !self.overflowed && !self.fault.iter().any(|fault| *fault);
        self.plan.finish(eligible, self.overflowed);
        if self.plan.blocked {
            self.fault.fill(true);
        }
    }

    pub(crate) fn planned_capture_failed(&self) -> bool {
        self.plan.blocked
    }

    pub(crate) fn planning(&self) -> bool {
        self.plan.active
    }

    /// Retain the producer's current, validated lookahead in a prepared pass.
    /// Observing the selecting raw bytes creates controls before this hint.
    pub(crate) fn prefetch(&self, address: usize, len: usize) {
        if let Some(observations) = &self.bytes.observations {
            observations.borrow_mut().hint(address, len);
        }
        engine::prefetch::span(address, len);
    }

    /// Capacity exhaustion after a prepared source observation must not enter
    /// an outer fresh typed retry. Keep its fault bits clear so normal worker
    /// growth can repair capacity before the next independent capture.
    pub(crate) fn planned_capture_requires_no_retry(&self) -> bool {
        self.planned_capture_failed() || (self.overflowed && self.plan.source_observed)
    }

    pub(crate) fn planned_capture_succeeded(&self) -> bool {
        self.plan.attempt == Attempt::Complete && !self.plan.blocked
    }

    /// Compound acquisitions cannot retry an earlier section's planned source
    /// observations when a later section exhausts its staging storage.
    pub(crate) fn planned_source_observed(&self) -> bool {
        self.plan.source_observed
    }

    /// Request retirement without dropping owned storage on the acquisition thread.
    pub(crate) fn invalidate_read_plan(&mut self) {
        self.plan.retire = true;
        self.plan.sealed = false;
    }

    /// Worker-only compilation and retirement, before clear/recycle.
    pub(crate) fn prepare_read_plan(&mut self) {
        self.return_planned_spans();
        let len = self.bytes.len();
        if let Some(observations) = &mut self.bytes.observations {
            self.plan
                .prepare(observations.get_mut(), self.has_bulk, self.raw_bytes, len);
            self.update_capacity_charge();
        }
    }

    fn return_planned_spans(&mut self) {
        if self.planned_spans_loaned {
            let compiled = self.plan.compiled.as_mut().expect("loaned plan geometry");
            std::mem::swap(&mut self.spans, &mut compiled.spans);
            self.planned_spans_loaned = false;
        }
    }

    pub fn grow_for_next(&self) -> bool {
        // More storage cannot repair a failed source read. In particular an
        // invalid optional-object path must not double every pool indefinitely.
        self.overflowed
            && self.required_bytes <= u32::MAX as usize
            && !self.fault.iter().any(|fault| *fault)
    }

    /// Grow only exhausted storage off-thread. Span-heavy censuses must not
    /// repeatedly double a mostly empty byte arena (and vice versa).
    pub fn grow(&mut self) {
        if self.span_exhausted {
            double_capacity(&mut self.spans);
        }
        if self.required_bytes > self.bytes.capacity() {
            let target = self
                .required_bytes
                .max(self.bytes.capacity().saturating_mul(2));
            self.bytes.grow_to(target);
        }
        self.update_capacity_charge();
    }

    /// Preflight `extra` more bytes before any read that must not be
    /// retried: when they do not fit, mark overflow and the size to grow to.
    pub fn reserve(&mut self, extra: usize) {
        let end = self.bytes.len().saturating_add(extra);
        if end > self.bytes.capacity() {
            self.required_bytes = self.required_bytes.max(end);
            self.overflowed = true;
        }
    }

    /// Guarded copy of one span into the pool. Returns the pool offset of the
    /// copied bytes on success and records failures with the affected-table
    /// mask. Empty spans read nothing (matching `Memory`), null or
    /// overflowing addresses fail without touching memory. On pool overflow
    /// sets `overflowed` and returns `None`.
    #[inline]
    pub fn take(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u32,
    ) -> Option<usize> {
        self.take_inner(memory, addr, len, mask, false)
    }

    /// Bulk variant of [`take`]: the span additionally serves replay requests
    /// fully enclosed in it (contiguous table copies). Use only for spans no
    /// smaller exact span overlaps.
    #[inline]
    pub fn take_bulk(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u32,
    ) -> Option<usize> {
        self.take_inner(memory, addr, len, mask, true)
    }

    #[inline]
    fn take_inner(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u32,
        bulk: bool,
    ) -> Option<usize> {
        if !self.plan.active {
            return self.take_scalar(memory, addr, len, mask, bulk);
        }
        if self.plan.blocked {
            return None;
        }
        // Empty fields consume neither source memory nor destination bytes.
        // Exclude them from the prepared trace and its continuation ordinal;
        // the next nonempty observation still matches address/width/scope.
        if len == 0 {
            return self.take_scalar(memory, addr, len, mask, bulk);
        }
        let result = if self.plan.prefix_cursor < self.plan.prefix_len {
            let index = self.plan.prefix_cursor;
            let request = self
                .plan
                .compiled
                .as_ref()
                .and_then(|compiled| compiled.requests.get(index))
                .copied();
            if !request
                .is_some_and(|request| request.matches(addr, len, mask, bulk, self.bytes.len()))
            {
                self.plan.block();
                self.fault.fill(true);
                self.bytes.observations.as_mut().unwrap().get_mut().eligible = false;
                return None;
            }
            let request = request.unwrap();
            self.plan.prefix_cursor += 1;
            if self.plan.failed_read == Some(index) {
                self.note_fail(addr, len as u32, mask);
                None
            } else {
                if len != 0 {
                    self.spans.push(Span {
                        addr,
                        len: len as u32,
                        ok: true,
                        mask,
                        start: request.start as u32,
                        bulk,
                    });
                    self.bytes.used = request.start + len;
                    self.has_bulk |= bulk;
                    self.raw_bytes[lowest_table(mask)] += len as u64;
                }
                Some(request.start)
            }
        } else {
            self.take_scalar(memory, addr, len, mask, bulk)
        };
        let observations = self.bytes.observations.as_mut().unwrap().get_mut();
        observations.after_reads += 1;
        if let Some(start) = result {
            if self.plan.requests.len() < self.plan.requests.capacity() {
                self.plan.requests.push(Request {
                    address: addr,
                    len,
                    start,
                    mask,
                    bulk,
                });
            } else {
                observations.eligible = false;
            }
        } else {
            observations.eligible = false;
        }
        result
    }

    #[inline]
    fn take_scalar(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u32,
        bulk: bool,
    ) -> Option<usize> {
        if self.overflowed {
            return None;
        }
        if len == 0 {
            return Some(self.bytes.len());
        }
        let Ok(len32) = u32::try_from(len) else {
            self.overflowed = true;
            return None;
        };
        if addr == 0 || addr.checked_add(len).is_none() {
            self.note_fail(addr, len32, mask);
            return None;
        }
        // Noncanonical x86-64 sources (e.g. ASCII "_factor" bytes read as a
        // pointer) fault with ExceptionInformation[1] == -1, which the C
        // filter cannot attribute and would crash. Record the same graceful
        // fault the typed walk reports for an unreadable source, before any
        // guarded copy runs. Ordering matches the typed walk: null, overflow,
        // then canonical, each with identical Unavailable/Truncated reasons
        // derived by the replay from these recorded spans.
        if !is_canonical_range(addr, len) {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if self.spans.len() == self.spans.capacity()
            || self
                .bytes
                .len()
                .checked_add(len)
                .is_none_or(|end| end > self.bytes.capacity() || end > u32::MAX as usize)
        {
            self.span_exhausted = self.spans.len() == self.spans.capacity();
            self.required_bytes = self.bytes.len().saturating_add(len);
            self.overflowed = true;
            return None;
        }
        let start = self.bytes.len();
        // The whole destination is already initialized. A failed partial
        // copy stays outside the logical prefix and records only its fault.
        if self.plan.active {
            self.plan.canonical_reads += 1;
        }
        if !memory.copy(addr, &mut self.bytes.storage[start..start + len]) {
            self.note_fail(addr, len32, mask);
            return None;
        }
        self.bytes.used = start + len;
        self.spans.push(Span {
            addr,
            len: len32,
            ok: true,
            mask,
            start: start as u32,
            bulk,
        });
        self.has_bulk |= bulk;
        self.raw_bytes[lowest_table(mask)] += len as u64;
        Some(start)
    }

    /// Stage independent fields with one guarded call, retaining the exact
    /// successful prefix and first failure. Each field keeps its table mask.
    #[inline(always)]
    pub fn take_scattered<const M: usize>(
        &mut self,
        memory: &impl Memory,
        addresses: [usize; M],
        lengths: [usize; M],
        masks: [u32; M],
    ) -> Option<([usize; M], usize)> {
        self.take_scattered_inner::<M, false>(memory, addresses, lengths, masks)
    }

    /// The same ordered capture with advisory hints for bounded independent
    /// spans. Admission, planning eligibility and failure accounting are shared
    /// with `take_scattered`; a short copy must not be retried.
    #[inline(always)]
    pub fn take_scattered_prefetched<const M: usize>(
        &mut self,
        memory: &impl Memory,
        addresses: [usize; M],
        lengths: [usize; M],
        masks: [u32; M],
    ) -> Option<([usize; M], usize)> {
        self.take_scattered_inner::<M, true>(memory, addresses, lengths, masks)
    }

    #[inline(always)]
    fn take_scattered_inner<const M: usize, const PREFETCH: bool>(
        &mut self,
        memory: &impl Memory,
        addresses: [usize; M],
        lengths: [usize; M],
        masks: [u32; M],
    ) -> Option<([usize; M], usize)> {
        if !self.plan.scattered && !self.allow_unplanned_operation() {
            return None;
        }
        if self.plan.blocked {
            return None;
        }
        if self.overflowed {
            return None;
        }
        let start = self.bytes.len();
        let mut end = start;
        let mut starts = [0; M];
        for (index, len) in lengths.into_iter().enumerate() {
            starts[index] = end;
            let Some(next) = end.checked_add(len).filter(|_| len <= u32::MAX as usize) else {
                self.overflowed = true;
                return None;
            };
            end = next;
        }
        let spans = lengths.iter().filter(|&&len| len != 0).count();
        if spans > self.spans.capacity() - self.spans.len()
            || end > self.bytes.capacity()
            || end > u32::MAX as usize
        {
            self.span_exhausted = spans > self.spans.capacity() - self.spans.len();
            self.required_bytes = end;
            self.overflowed = true;
            return None;
        }
        if self.plan.active && self.plan.prefix_cursor < self.plan.prefix_len {
            // A changed control or failed read can stop inside this scatter.
            // Replay its ordered prefix through take_inner, then acquire only
            // the untouched suffix. Full scatter admission still precedes it.
            for index in 0..M {
                match self.take_inner(
                    memory,
                    addresses[index],
                    lengths[index],
                    masks[index],
                    false,
                ) {
                    Some(start) => starts[index] = start,
                    None if self.plan.blocked => return None,
                    None => return Some((starts, index)),
                }
            }
            return Some((starts, M));
        }
        let completed = {
            if PREFETCH && let Some(observations) = &self.bytes.observations {
                let mut observations = observations.borrow_mut();
                for (address, len) in addresses.into_iter().zip(lengths) {
                    observations.hint(address, len);
                }
            }
            let mut remaining = &mut self.bytes.storage[start..end];
            let mut destinations: [&mut [u8]; M] = std::array::from_fn(|_| &mut [][..]);
            for (destination, len) in destinations.iter_mut().zip(lengths) {
                let (field, rest) = remaining.split_at_mut(len);
                *destination = field;
                remaining = rest;
            }
            let guarded = std::array::from_fn(|index| {
                if is_canonical_range(addresses[index], lengths[index]) {
                    addresses[index]
                } else {
                    0
                }
            });
            if PREFETCH {
                memory.copy_scattered_prefetched(guarded, &mut destinations)
            } else {
                memory.copy_scattered(guarded, &mut destinations)
            }
        };
        debug_assert!(completed <= M);
        self.bytes.used = if completed == M {
            end
        } else {
            starts[completed]
        };
        for index in 0..completed {
            let len = lengths[index];
            if len != 0 {
                self.spans.push(Span {
                    addr: addresses[index],
                    len: len as u32,
                    ok: true,
                    mask: masks[index],
                    start: starts[index] as u32,
                    bulk: false,
                });
                self.raw_bytes[lowest_table(masks[index])] += len as u64;
            }
        }
        if completed != M {
            self.note_fail(
                addresses[completed],
                lengths[completed] as u32,
                masks[completed],
            );
        }
        if self.plan.active && self.plan.scattered {
            self.plan.canonical_reads += 1;
            let observations = self.bytes.observations.as_mut().unwrap().get_mut();
            observations.after_reads += lengths[..M.min(completed + 1)]
                .iter()
                .filter(|&&len| len != 0)
                .count();
            observations.eligible &= completed == M;
            for index in 0..completed {
                if lengths[index] == 0 {
                    continue;
                }
                if self.plan.requests.len() == self.plan.requests.capacity() {
                    observations.eligible = false;
                    break;
                }
                self.plan.requests.push(Request {
                    address: addresses[index],
                    len: lengths[index],
                    start: starts[index],
                    mask: masks[index],
                    bulk: false,
                });
            }
        }
        Some((starts, completed))
    }

    /// Copy at most 64 consecutive fixed-width elements in one guard. Retain
    /// the successful prefix as one bulk span and the first failed element
    /// separately; never retry a partial copy or read beyond that failure.
    /// The limit bounds one guarded call; callers chunk the complete array.
    #[inline(always)]
    pub fn take_array_chunk(
        &mut self,
        memory: &impl Memory,
        address: usize,
        width: usize,
        count: usize,
        mask: u32,
    ) -> Option<(usize, usize)> {
        self.take_array_inner::<true>(memory, address, width, count, mask)
    }

    /// [`take_array_chunk`](Self::take_array_chunk) retaining each copied
    /// element as its own exact span, so the log is the one per-element
    /// [`take`](Self::take)s would record: the same spans, bytes and first
    /// failure, and no bulk span. Use it where the replay reads the elements
    /// one by one; it then serves each from its exact key in record order
    /// instead of searching the bulk ranges.
    #[inline(always)]
    pub fn take_array_entries(
        &mut self,
        memory: &impl Memory,
        address: usize,
        width: usize,
        count: usize,
        mask: u32,
    ) -> Option<(usize, usize)> {
        self.take_array_inner::<false>(memory, address, width, count, mask)
    }

    #[inline(always)]
    fn take_array_inner<const BULK: bool>(
        &mut self,
        memory: &impl Memory,
        address: usize,
        width: usize,
        count: usize,
        mask: u32,
    ) -> Option<(usize, usize)> {
        if !self.allow_unplanned_operation() {
            return None;
        }
        if self.overflowed || count > 64 || width == 0 || width > u32::MAX as usize {
            return None;
        }
        let start = self.bytes.len();
        if count == 0 {
            return Some((start, 0));
        }
        let size = count.checked_mul(width)?;
        let end = start.checked_add(size)?;
        // The copied prefix and one failed element.
        let spans = if BULK { 2 } else { count + 1 };
        if self.spans.capacity() - self.spans.len() < spans
            || end > self.bytes.capacity()
            || end > u32::MAX as usize
        {
            self.span_exhausted = self.spans.capacity() - self.spans.len() < spans;
            self.required_bytes = end;
            self.overflowed = true;
            return None;
        }
        // The ordinary contiguous range has one arithmetic/canonical gate.
        // An invalid suffix still permits its exact earlier elements, including
        // an actual read fault before that suffix. No pointer array is staged.
        let admitted = if address != 0
            && address.checked_add(size).is_some()
            && is_canonical_range(address, size)
        {
            count
        } else {
            (0..count)
                .take_while(|&index| {
                    address.checked_add(index * width).is_some_and(|source| {
                        source != 0
                            && source.checked_add(width).is_some()
                            && is_canonical_range(source, width)
                    })
                })
                .count()
        };
        let fields = [engine::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: width,
        }];
        let layout = engine::memory::RecordLayout::new(&fields, width)?;
        let completed = memory.copy_records(
            engine::memory::RecordSources::Strided {
                base: address,
                stride: width,
                count: admitted,
            },
            &layout,
            &mut self.bytes.storage[start..end],
        );
        debug_assert!(completed <= count);
        self.bytes.used = start + completed * width;
        if completed != 0 && BULK {
            self.spans.push(Span {
                addr: address,
                len: (completed * width) as u32,
                ok: true,
                mask,
                start: start as u32,
                bulk: true,
            });
            self.has_bulk = true;
        }
        if !BULK {
            for index in 0..completed {
                self.spans.push(Span {
                    addr: address + index * width,
                    len: width as u32,
                    ok: true,
                    mask,
                    start: (start + index * width) as u32,
                    bulk: false,
                });
            }
        }
        if completed != 0 {
            self.raw_bytes[lowest_table(mask)] += (completed * width) as u64;
        }
        if completed != count {
            // Fault metadata retains the original field address after either
            // admission failure or a live fault. A failed read is not retried.
            let failed_address = address.checked_add(completed * width).unwrap_or(0);
            self.note_fail(failed_address, width as u32, mask);
        }
        Some((start, completed))
    }

    fn note_fail(&mut self, addr: usize, len32: u32, mask: u32) {
        if self.spans.len() == self.spans.capacity() {
            self.span_exhausted = true;
            self.overflowed = true;
            return;
        }
        self.spans.push(Span {
            addr,
            len: len32,
            ok: false,
            mask,
            start: 0,
            bulk: false,
        });
        for (slot, fault) in self.fault.iter_mut().enumerate() {
            if mask & (1 << slot) != 0 {
                *fault = true;
            }
        }
    }

    /// A different source-operation kind cannot consume a scalar prefix.
    /// Without a prefix, it only makes this capture ineligible for planning.
    fn allow_unplanned_operation(&mut self) -> bool {
        if !self.plan.active {
            return true;
        }
        self.bytes.observations.as_mut().unwrap().get_mut().eligible = false;
        self.plan.retire = true;
        if self.plan.prefix_cursor != self.plan.prefix_len {
            self.plan.block();
            self.fault.fill(true);
        }
        !self.plan.blocked
    }

    /// Compare staged implementations in synthetic-memory tests only.
    #[cfg(test)]
    pub fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
    }
}

#[cfg(test)]
mod read_plan_tests {
    use super::*;
    use std::cell::Cell;

    struct Script {
        reads: Vec<(usize, Option<Vec<u8>>)>,
        cursor: Cell<usize>,
    }

    impl Script {
        fn new(reads: Vec<(usize, Option<Vec<u8>>)>) -> Self {
            Self {
                reads,
                cursor: Cell::new(0),
            }
        }

        fn complete(&self) {
            assert_eq!(
                self.cursor.get(),
                self.reads.len(),
                "all expected source observations consumed"
            );
        }
    }

    impl Memory for Script {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let index = self.cursor.get();
            let (expected, bytes) = self
                .reads
                .get(index)
                .expect("unexpected source read or retry");
            assert_eq!(address, *expected, "source occurrence {index}");
            self.cursor.set(index + 1);
            if let Some(bytes) = bytes {
                assert_eq!(destination.len(), bytes.len());
                destination.copy_from_slice(bytes);
                true
            } else {
                if let Some(first) = destination.first_mut() {
                    *first = 0xEE;
                }
                false
            }
        }
    }

    fn log() -> RawLog<2> {
        let mut log = RawLog::new(16, 128);
        log.initialize_read_plan(16, 16, 128);
        log
    }

    fn linked(memory: &impl Memory, log: &mut RawLog<2>) {
        let Some(root) = log.take(memory, 0x1000, 8, 3) else {
            return;
        };
        let child = u64::from_le_bytes(log.bytes[root..root + 8].try_into().unwrap()) as usize;
        let _ = log.take(memory, child, 4, 2);
    }

    fn linked_script(child: usize, value: [u8; 4]) -> Script {
        Script::new(vec![
            (0x1000, Some((child as u64).to_le_bytes().to_vec())),
            (child, Some(value.to_vec())),
        ])
    }

    #[test]
    fn fresh_payload_reuses_plan_and_changed_pointer_resumes_without_repeating_root() {
        let mut log = log();
        let first = linked_script(0x2000, [1; 4]);
        assert!(!log.begin_planned_capture(&first, [1, 2, 3, 4]));
        linked(&first, &mut log);
        log.finish_planned_capture();
        first.complete();
        let shape = log.spans.clone();
        log.prepare_read_plan();
        log.clear();

        let fresh = linked_script(0x2000, [9; 4]);
        assert!(log.begin_planned_capture(&fresh, [1, 2, 3, 4]));
        log.finish_planned_capture();
        fresh.complete();
        assert!(log.planned_capture_succeeded());
        assert_eq!(log.spans, shape);
        assert_eq!(&log.bytes[8..], &[9; 4]);
        assert_eq!(log.raw_bytes, [8, 4]);
        log.prepare_read_plan();
        log.clear();

        let changed = linked_script(0x3000, [7; 4]);
        assert!(!log.begin_planned_capture(&changed, [1, 2, 3, 4]));
        assert!(log.bytes.is_empty());
        linked(&changed, &mut log);
        log.finish_planned_capture();
        changed.complete();
        assert!(!log.planned_capture_failed());
        assert_eq!(log.spans[1].addr, 0x3000);
        assert_eq!(&log.bytes[8..], &[7; 4]);
        log.prepare_read_plan();
        log.clear();
        let rebuilt = linked_script(0x3000, [6; 4]);
        assert!(log.begin_planned_capture(&rebuilt, [1, 2, 3, 4]));
        log.finish_planned_capture();
        rebuilt.complete();
    }

    #[test]
    fn complete_plan_loans_geometry_for_decode_and_returns_it_before_reuse_or_retirement() {
        let mut log = log();
        let staging = log.spans.as_ptr();
        let first = linked_script(0x2000, [1; 4]);
        assert!(!log.begin_planned_capture(&first, [1; 4]));
        linked(&first, &mut log);
        log.finish_planned_capture();
        first.complete();
        log.prepare_read_plan();
        let geometry = log.plan.compiled.as_ref().unwrap().spans.as_ptr();
        log.clear();

        for value in [2, 3] {
            let fresh = linked_script(0x2000, [value; 4]);
            assert!(log.begin_planned_capture(&fresh, [1; 4]));
            log.finish_planned_capture();
            fresh.complete();
            assert_eq!(log.spans.as_ptr(), geometry);
            let recorded = Recorded::build(&log);
            let mut bytes = [0; 4];
            assert!(recorded.copy(0x2000, &mut bytes));
            assert_eq!(bytes, [value; 4]);
            drop(recorded);
            // Both normal recycle and clear without preparation return the loan.
            if value == 2 {
                log.prepare_read_plan();
            }
            log.clear();
            assert_eq!(log.spans.as_ptr(), staging);
            assert_eq!(log.plan.compiled.as_ref().unwrap().spans.as_ptr(), geometry);
        }

        let fresh = linked_script(0x2000, [4; 4]);
        assert!(log.begin_planned_capture(&fresh, [1; 4]));
        log.finish_planned_capture();
        fresh.complete();
        log.invalidate_read_plan();
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
        assert_eq!(log.spans.as_ptr(), staging);
        log.clear();
        let changed = linked_script(0x3000, [5; 4]);
        assert!(!log.begin_planned_capture(&changed, [1; 4]));
        linked(&changed, &mut log);
        log.finish_planned_capture();
        changed.complete();
        assert_eq!(log.spans[1].addr, 0x3000);
    }

    #[test]
    fn failed_occurrence_is_replayed_once_and_partial_bytes_stay_unpublished() {
        let mut log = log();
        let first = Script::new(vec![
            (0x10, Some(vec![1; 4])),
            (0x20, Some(vec![2; 4])),
            (0x30, Some(vec![3; 4])),
        ]);
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        for address in [0x10, 0x20, 0x30] {
            log.take(&first, address, 4, 1);
        }
        log.finish_planned_capture();
        log.prepare_read_plan();
        log.clear();
        let failed = Script::new(vec![
            (0x10, Some(vec![8; 4])),
            (0x20, None),
            (0x30, Some(vec![9; 4])),
        ]);
        assert!(!log.begin_planned_capture(&failed, [0; 4]));
        assert!(log.bytes.is_empty());
        assert_eq!(log.take(&failed, 0x10, 4, 1), Some(0));
        assert_eq!(log.take(&failed, 0x20, 4, 1), None);
        assert_eq!(log.bytes.len(), 4);
        assert_eq!(log.take(&failed, 0x30, 4, 1), Some(4));
        log.finish_planned_capture();
        failed.complete();
        assert_eq!(&log.bytes[..], &[8, 8, 8, 8, 9, 9, 9, 9]);
        assert_eq!(log.raw_bytes, [8, 0]);
        assert_eq!(log.fault, [true, false]);
        assert!(!log.spans[1].ok);
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
    }

    #[test]
    fn duplicate_observations_keep_their_ordinals_without_empty_trace_entries() {
        let mut log = log();
        let walk = |memory: &Script, log: &mut RawLog<2>| {
            assert_eq!(log.take(memory, 0, 0, 3), Some(0));
            for value in [1u8, 2, 1] {
                let start = log.take(memory, 0x10, 1, 1).unwrap();
                let observed = log.bytes.get(start).copied().unwrap();
                assert_eq!(observed, value);
            }
            assert_eq!(log.take(memory, usize::MAX, 0, 2), Some(3));
            assert_eq!(log.bytes[0], 1);
            assert_eq!(log.bytes.get(1..2), Some(&[2][..]));
        };
        let sequence = || {
            Script::new(vec![
                (0x10, Some(vec![1])),
                (0x10, Some(vec![2])),
                (0x10, Some(vec![1])),
            ])
        };
        let first = sequence();
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        walk(&first, &mut log);
        log.finish_planned_capture();
        log.prepare_read_plan();
        assert_eq!(log.plan.compiled.as_ref().unwrap().requests.len(), 3);
        log.clear();
        let fresh = sequence();
        assert!(log.begin_planned_capture(&fresh, [0; 4]));
        log.finish_planned_capture();
        fresh.complete();
        assert_eq!(log.spans.len(), 3);
        assert_eq!(&log.bytes[..], &[1, 2, 1]);
    }

    #[test]
    fn empty_scatter_slots_preserve_changed_controls_and_every_failed_prefix() {
        fn walk(source: &Script, log: &mut RawLog<2>) {
            let Some((starts, 5)) = log.take_scattered(
                source,
                [0, 0x10, usize::MAX, 0x20, 0],
                [0, 1, 0, 1, 0],
                [3, 1, 2, 2, 1],
            ) else {
                return;
            };
            let target = if log.bytes[starts[1]] == 1 {
                0x30
            } else {
                0x40
            };
            assert_eq!(log.take(source, usize::MAX, 0, 3), Some(2));
            log.take(source, target, 1, 2);
        }
        for changed in [false, true] {
            for failure in [None, Some(0), Some(1), Some(2)] {
                let mut prepared = log();
                prepared.initialize_scattered_read_plan(16, 16, 128);
                let baseline = Script::new(vec![
                    (0x10, Some(vec![1])),
                    (0x20, Some(vec![2])),
                    (0x30, Some(vec![3])),
                ]);
                assert!(!prepared.begin_planned_capture(&baseline, [0; 4]));
                walk(&baseline, &mut prepared);
                prepared.finish_planned_capture();
                baseline.complete();
                prepared.prepare_read_plan();
                assert_eq!(prepared.plan.compiled.as_ref().unwrap().requests.len(), 3);
                prepared.clear();
                let mut reads = vec![
                    (0x10, Some(vec![if changed { 4 } else { 1 }])),
                    (0x20, Some(vec![5])),
                    (if changed { 0x40 } else { 0x30 }, Some(vec![6])),
                ];
                if let Some(index) = failure {
                    reads[index].1 = None;
                    reads.truncate(index + 1);
                }
                let fresh = Script::new(reads.clone());
                let mut direct = log();
                walk(&fresh, &mut direct);
                fresh.complete();
                let fresh = Script::new(reads);
                if !prepared.begin_planned_capture(&fresh, [0; 4]) {
                    walk(&fresh, &mut prepared);
                }
                prepared.finish_planned_capture();
                fresh.complete();
                assert!(!prepared.planned_capture_failed());
                assert_eq!(prepared.spans, direct.spans);
                assert_eq!(&prepared.bytes[..], &direct.bytes[..]);
                assert_eq!(prepared.fault, direct.fault);
                assert_eq!(prepared.raw_bytes, direct.raw_bytes);
            }
        }
    }

    #[test]
    fn prefix_descriptor_mismatch_blocks_all_later_reads_and_retires_plan() {
        for (address, len, mask, bulk) in [
            (0x11, 1, 1, false),
            (0x10, 2, 1, false),
            (0x10, 1, 2, false),
            (0x10, 1, 1, true),
        ] {
            let mut log = log();
            let first = Script::new(vec![(0x10, Some(vec![1]))]);
            assert!(!log.begin_planned_capture(&first, [0; 4]));
            log.take(&first, 0x10, 1, 1);
            assert_eq!(log.bytes[0], 1);
            log.finish_planned_capture();
            log.prepare_read_plan();
            log.clear();
            let changed = Script::new(vec![(0x10, Some(vec![2]))]);
            assert!(!log.begin_planned_capture(&changed, [0; 4]));
            let result = if bulk {
                log.take_bulk(&changed, address, len, mask)
            } else {
                log.take(&changed, address, len, mask)
            };
            assert_eq!(result, None);
            assert_eq!(log.take(&changed, 0x10, 1, 1), None);
            log.finish_planned_capture();
            changed.complete();
            assert!(log.planned_capture_failed());
            assert_eq!(log.fault, [true; 2]);
            log.prepare_read_plan();
            assert!(log.plan.compiled.is_none());
        }
    }

    #[test]
    fn planned_prefix_then_payload_overflow_forbids_retry_without_preventing_growth() {
        let mut log = RawLog::<2>::new(4, 8);
        log.initialize_read_plan(4, 4, 8);
        let first = Script::new(vec![(0x10, Some(vec![1])), (0x20, Some(vec![7]))]);
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        let root = log.take(&first, 0x10, 1, 1).unwrap();
        assert_eq!(log.bytes[root], 1);
        assert_eq!(log.take(&first, 0x20, 1, 1), Some(1));
        log.finish_planned_capture();
        log.prepare_read_plan();
        log.clear();

        let changed = Script::new(vec![(0x10, Some(vec![2]))]);
        assert!(!log.begin_planned_capture(&changed, [0; 4]));
        let root = log.take(&changed, 0x10, 1, 1).unwrap();
        assert_eq!(log.bytes[root], 2);
        let overflowing_len = log.bytes.capacity();
        assert_eq!(log.take(&changed, 0x20, overflowing_len, 1), None);
        assert_eq!(log.take(&changed, 0x30, 1, 1), None);
        log.finish_planned_capture();
        changed.complete();
        assert!(log.overflowed);
        assert!(log.planned_capture_requires_no_retry());
        assert!(!log.planned_capture_failed());
        assert_eq!(log.fault, [false; 2]);
        assert!(log.grow_for_next());
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
        log.grow();
        assert!(log.bytes.capacity() > overflowing_len);
        log.clear();
        assert!(!log.planned_capture_requires_no_retry());

        // The same ordinary overflow keeps the existing fallback contract.
        let ordinary = Script::new(vec![(0x10, Some(vec![2]))]);
        assert!(!log.begin_planned_capture(&ordinary, [0; 4]));
        log.take(&ordinary, 0x10, 1, 1);
        let overflowing_len = log.bytes.capacity();
        log.take(&ordinary, 0x20, overflowing_len, 1);
        log.finish_planned_capture();
        ordinary.complete();
        assert!(log.overflowed);
        assert!(!log.planned_capture_requires_no_retry());
        assert!(log.grow_for_next());
    }

    #[test]
    fn trace_exhaustion_grows_only_off_thread_and_never_truncates_the_payload() {
        let mut log = RawLog::<2>::new(16, 128);
        log.initialize_read_plan(1, 0, 0);
        let first = linked_script(0x2000, [1; 4]);
        assert!(!log.begin_planned_capture(&first, [0; 4]));
        linked(&first, &mut log);
        log.finish_planned_capture();
        first.complete();
        assert!(!log.overflowed);
        assert_eq!(log.fault, [false; 2]);
        assert_eq!(log.bytes.len(), 12);
        assert_eq!(log.plan.requests.capacity(), 1);
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_none());
        assert!(log.plan.requests.capacity() >= 2);
        assert!(
            log.bytes
                .observations
                .as_ref()
                .unwrap()
                .borrow()
                .expected
                .capacity()
                >= 8
        );
        log.clear();
        let next = linked_script(0x2000, [2; 4]);
        assert!(!log.begin_planned_capture(&next, [0; 4]));
        linked(&next, &mut log);
        log.finish_planned_capture();
        log.prepare_read_plan();
        assert!(log.plan.compiled.is_some());
    }

    #[test]
    fn untracked_slice_escape_and_non_scalar_acquisition_only_disable_optimization() {
        for operation in 0..3 {
            let mut log = log();
            let first = Script::new(vec![(0x10, Some(vec![1])), (0x20, Some(vec![2]))]);
            assert!(!log.begin_planned_capture(&first, [0; 4]));
            log.take(&first, 0x10, 1, 1);
            if operation == 0 {
                let escaped: &[u8] = &log.bytes;
                assert_eq!(escaped, &[1]);
                log.take(&first, 0x20, 1, 2);
            } else if operation == 1 {
                log.take_scattered(&first, [0x20], [1], [2]);
            } else {
                log.take_scattered_prefetched(&first, [0x20], [1], [2]);
            }
            log.finish_planned_capture();
            first.complete();
            assert!(!log.overflowed);
            assert!(!log.planned_capture_failed());
            assert_eq!(log.fault, [false; 2]);
            assert_eq!(&log.bytes[..], &[1, 2]);
            log.prepare_read_plan();
            assert!(log.plan.compiled.is_none());
        }
    }

    #[test]
    fn prefetched_scatter_obeys_pending_and_consumed_plan_prefix_gates() {
        for prefetched in [false, true] {
            for failed in [false, true] {
                for consume_prefix in [false, true] {
                    let mut log = log();
                    log.initialize_scattered_read_plan(16, 16, 128);
                    let first = Script::new(vec![(0x10, Some(vec![1]))]);
                    assert!(!log.begin_planned_capture(&first, [0; 4]));
                    assert_eq!(log.take(&first, 0x10, 1, 1), Some(0));
                    assert_eq!(log.bytes[0], 1);
                    log.finish_planned_capture();
                    first.complete();
                    log.prepare_read_plan();
                    assert!(log.plan.compiled.is_some());
                    log.clear();

                    let mut reads = vec![(0x10, (!failed).then(|| vec![2]))];
                    if consume_prefix {
                        reads.push((0x20, Some(vec![7])));
                    }
                    let changed = Script::new(reads);
                    assert!(!log.begin_planned_capture(&changed, [0; 4]));
                    assert_eq!(log.plan.prefix_len, 1);
                    if consume_prefix {
                        assert_eq!(log.take(&changed, 0x10, 1, 1), (!failed).then_some(0));
                    }
                    let result = if prefetched {
                        log.take_scattered_prefetched(&changed, [0x20], [1], [2])
                    } else {
                        log.take_scattered(&changed, [0x20], [1], [2])
                    };
                    assert_eq!(
                        result,
                        consume_prefix.then_some(([usize::from(!failed)], 1))
                    );
                    assert!(log.plan.retire);
                    assert_eq!(
                        log.bytes.observations.as_ref().unwrap().borrow().eligible,
                        consume_prefix && !failed
                    );
                    assert_eq!(log.planned_capture_failed(), !consume_prefix);
                    if consume_prefix {
                        assert_eq!(log.fault, [failed, false]);
                        assert_eq!(log.raw_bytes, [u64::from(!failed), 1]);
                        assert_eq!(log.bytes[log.bytes.len() - 1], 7);
                    } else {
                        assert_eq!(log.fault, [true; 2]);
                        assert!(log.spans.is_empty());
                        assert!(log.bytes.is_empty());
                        assert_eq!(log.take(&changed, 0x20, 1, 2), None);
                    }
                    log.finish_planned_capture();
                    changed.complete();
                    log.prepare_read_plan();
                    assert_eq!(log.plan.compiled.is_some(), consume_prefix && !failed);
                }
            }
        }
    }
}

/// Attribute shared spans to the anchor table; private spans carry a single
/// bit already. A zero mask retains the existing table-zero attribution.
#[inline]
pub fn lowest_table(mask: u32) -> usize {
    if mask == 0 {
        0
    } else {
        mask.trailing_zeros() as usize
    }
}

/// Decode-side address space: serves exactly the recorded spans and fails
/// every other read, so the replayed walk observes the identical bytes and
/// faults the copy observed. Empty reads succeed and null or overflowing
/// addresses fail without a lookup, matching `Memory`. Logs with bulk spans
/// additionally serve any request fully enclosed in one bulk span.
///
/// Repeated takes of one span are served in record order: the replay issues
/// the same read sequence the copy recorded, so witness-before/after pairs
/// (and any other repeated read) observe their own values rather than the
/// last write. Once the recorded multiplicity is exhausted, the last value
/// keeps serving: shared spans (one census feeding several replays) are
/// taken once and requested many times with identical bytes, while a span
/// the mirror never took has no entry at all and still fails loudly.
pub struct Recorded<'a> {
    index: RecordedIndex,
    recycle: &'a RefCell<Option<RecordedIndex>>,
    spans: &'a [Span],
    bytes: &'a [u8],
    next_exact: std::cell::Cell<usize>,
    last_bulk: std::cell::Cell<usize>,
}

#[derive(Debug, Default)]
struct RecordedIndex {
    has_bulk: bool,
    // Every exact key, including failed reads and whole bulk chunks. Widths
    // 0..=254 have individual bits; larger widths share a conservative bucket.
    exact_lengths: [u64; 4],
    // Empty buckets use usize::MAX; occupied buckets hold an observation
    // cursor. Advancing that cursor never changes its exact address/length key.
    slots: Vec<std::cell::Cell<usize>>,
    distinct: usize,
    // Globally unique keys use usize::MAX. Repeated observations form a
    // forward chain whose final entry points to itself, never to that sentinel.
    next: Vec<usize>,
    // Only duplicate keys need a saved first observation for reader reset.
    // Bucket, first observation. Bucket positions are resolved after the final
    // build-time rehash, then remain stable throughout reuse and reading.
    repeated: Vec<(usize, usize)>,
    // Exact structural comparison, not a hash witness. Payload bytes and
    // byte-pool offsets always come from the current refresh's spans.
    shape: Vec<(usize, u32, bool, bool)>,
    // Start, end, current-log span index, maximum end through this entry.
    bulk: Vec<(usize, usize, usize, usize)>,
    capacity_charge: crate::raw_memory::Charge,
}

/// Process-local integer read keys, not untrusted strings or file identity.
/// Exact address/length equality still resolves every bucket collision. This
/// cheaper mixer affects lookup cost only; it is never a correctness witness.
#[derive(Debug, Default)]
struct ReadAddressHasher(u64);

impl Hasher for ReadAddressHasher {
    fn finish(&self) -> u64 {
        let mut value = self.0;
        value ^= value >> 32;
        value = value.wrapping_mul(0xd6e8_feb8_6659_fd93);
        value ^ (value >> 32)
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u64(u64::from(byte));
        }
    }

    fn write_u64(&mut self, value: u64) {
        self.0 = (self.0 ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(value as u64);
    }

    fn write_u32(&mut self, value: u32) {
        self.write_u64(u64::from(value));
    }
}

impl RecordedIndex {
    #[inline]
    fn mark_exact_length(&mut self, len: u32) {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    fn may_have_exact_length(&self, len: u32) -> bool {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    #[inline]
    fn hash(address: usize, len: u32) -> usize {
        let mut hash = ReadAddressHasher::default();
        hash.write_usize(address);
        hash.write_u32(len);
        hash.finish() as usize
    }

    /// Return the matching bucket or the first empty bucket. No deletions occur
    /// within a build, so an empty bucket terminates a lookup. Bound the scan
    /// even if an internal invariant were to leave a completely full table.
    #[inline]
    fn slot(&self, spans: &[Span], address: usize, len: u32) -> Option<usize> {
        let mask = self.slots.len().checked_sub(1)?;
        let mut slot = Self::hash(address, len) & mask;
        for _ in 0..self.slots.len() {
            let current = self.slots[slot].get();
            if current == usize::MAX {
                return Some(slot);
            }
            let span = &spans[current];
            if span.addr == address && span.len == len {
                return Some(slot);
            }
            slot = (slot + 1) & mask;
        }
        None
    }

    #[inline]
    fn cursor(&self, spans: &[Span], address: usize, len: u32) -> Option<&std::cell::Cell<usize>> {
        let cursor = &self.slots[self.slot(spans, address, len)?];
        (cursor.get() != usize::MAX).then_some(cursor)
    }

    fn grow_slots(&mut self, spans: &[Span]) {
        let size = if self.slots.is_empty() {
            8
        } else {
            self.slots
                .len()
                .checked_mul(2)
                .expect("raw index bucket count overflow")
        };
        // Allocations and the temporary old table live only on the worker.
        // The retained-capacity diagnostic excludes transient realloc overlap.
        let old = std::mem::replace(
            &mut self.slots,
            vec![std::cell::Cell::new(usize::MAX); size],
        );
        for cursor in old {
            let current = cursor.get();
            if current != usize::MAX {
                let span = &spans[current];
                let slot = self
                    .slot(spans, span.addr, span.len)
                    .expect("grown raw index must have an empty bucket");
                self.slots[slot].set(current);
            }
        }
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            index_vectors: vector_bytes(&self.slots)
                + vector_bytes(&self.next)
                + vector_bytes(&self.repeated)
                + vector_bytes(&self.shape)
                + vector_bytes(&self.bulk),
            ..Default::default()
        });
    }

    fn prepare(&mut self, spans: &[Span], has_bulk: bool) -> bool {
        if self.has_bulk == has_bulk
            && self.shape.len() == spans.len()
            && self
                .shape
                .iter()
                .zip(spans)
                .all(|(shape, span)| *shape == (span.addr, span.len, span.ok, span.bulk))
        {
            for &(slot, first) in &self.repeated {
                self.slots[slot].set(first);
            }
            return true;
        }
        self.slots.fill(std::cell::Cell::new(usize::MAX));
        self.distinct = 0;
        self.has_bulk = has_bulk;
        self.exact_lengths = [0; 4];
        // Grow for distinct read keys, not repeated observations. In the
        // calibration matrix those counts can differ by orders of magnitude.
        self.next.clear();
        self.next.resize(spans.len(), usize::MAX);
        self.repeated.clear();
        self.bulk.clear();
        self.shape.clear();
        for (index, span) in spans.iter().enumerate() {
            self.shape.push((span.addr, span.len, span.ok, span.bulk));
            self.mark_exact_length(span.len);
            if self.slots.is_empty() {
                self.grow_slots(spans);
            }
            let mut slot = self
                .slot(spans, span.addr, span.len)
                .expect("raw index must have an empty bucket");
            let previous = self.slots[slot].get();
            if previous == usize::MAX {
                // At most half full. Repeated observations must not trigger
                // growth, even when their count dwarfs the distinct key count.
                if self.distinct == self.slots.len() / 2 {
                    self.grow_slots(spans);
                    slot = self
                        .slot(spans, span.addr, span.len)
                        .expect("grown raw index must have an empty bucket");
                }
                self.slots[slot].set(index);
                self.distinct += 1;
            } else {
                // During construction each cursor holds the last observation.
                // Record the root once, but not its as-yet-unstable bucket.
                if self.next[previous] == usize::MAX {
                    self.repeated.push((usize::MAX, previous));
                }
                self.next[previous] = index;
                self.next[index] = index;
                self.slots[slot].set(index);
            }
            // Only flagged bulk chunks serve sub-ranges: small exact
            // spans inside a chunk must never shadow it.
            if span.ok && self.has_bulk && span.bulk && span.len > 0 {
                self.bulk
                    .push((span.addr, span.addr + span.len as usize, index, 0));
            }
        }
        // The same cell is the authoritative read cursor after preparation.
        // Unique entries already name their sole span and need no reset.
        for root in 0..self.repeated.len() {
            let first = self.repeated[root].1;
            let span = &spans[first];
            let slot = self
                .slot(spans, span.addr, span.len)
                .expect("raw index must retain every repeated key");
            self.repeated[root].0 = slot;
            self.slots[slot].set(first);
        }
        if !self.bulk.is_empty() {
            self.bulk.sort_unstable();
            let mut maximum_end = 0;
            for span in &mut self.bulk {
                maximum_end = maximum_end.max(span.1);
                span.3 = maximum_end;
            }
        }
        self.update_capacity_charge();
        false
    }
}

impl<'a> Recorded<'a> {
    pub fn build<const N: usize>(log: &'a RawLog<N>) -> Self {
        let started = std::time::Instant::now();
        let mut index = log
            .index
            .borrow_mut()
            .take()
            .unwrap_or_else(|| RecordedIndex {
                capacity_charge: crate::raw_memory::Charge::empty_on(log.capacity_charge.tracker()),
                ..Default::default()
            });
        let reused = index.prepare(&log.spans, log.has_bulk);
        let ns = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        let (count, ticks, spans) = if reused {
            (&INDEX_REUSES, &INDEX_REUSE_NS, &INDEX_REUSED_SPANS)
        } else {
            (&INDEX_REBUILDS, &INDEX_REBUILD_NS, &INDEX_REBUILT_SPANS)
        };
        ticks.fetch_add(ns, Ordering::Relaxed);
        spans.fetch_add(log.spans.len() as u64, Ordering::Relaxed);
        count.fetch_add(1, Ordering::Relaxed);
        Self {
            index,
            recycle: &log.index,
            spans: &log.spans,
            bytes: &log.bytes,
            next_exact: std::cell::Cell::new(0),
            last_bulk: std::cell::Cell::new(usize::MAX),
        }
    }

    #[inline(always)]
    fn serve(&self, address: usize, destination: &mut [u8]) -> bool {
        let hint = self.next_exact.get();
        let len = destination.len() as u32;
        let exact = if self.spans.get(hint).is_some_and(|span| {
            span.addr == address && span.len == len && self.index.next[hint] == usize::MAX
        }) {
            // A globally unique exact key has no observation cursor to
            // advance. Repeated keys always use the authoritative hash cursor,
            // even when a later occurrence happens to match this hint.
            Some(hint)
        } else if self.index.has_bulk && !self.index.may_have_exact_length(len) {
            // No exact observation of this width can override a bulk range.
            // A negative lookup must not advance either exact-reader cursor.
            None
        } else {
            self.index.cursor(self.spans, address, len).map(|cursor| {
                let current = cursor.get();
                let next = self.index.next[current];
                if next != usize::MAX {
                    cursor.set(next);
                }
                current
            })
        };
        if let Some(index) = exact {
            // Resynchronize on ordinary exact lookups too. An exact failure
            // retains its priority over bulk and still advances the hint.
            self.next_exact.set(index + 1);
            let span = &self.spans[index];
            if !span.ok {
                return false;
            }
            let start = span.start as usize;
            let Some(source) = self.bytes.get(start..start + destination.len()) else {
                return false;
            };
            destination.copy_from_slice(source);
            return true;
        }
        if self.index.bulk.is_empty() {
            return false;
        }
        let end = address.checked_add(destination.len());
        let end = match end {
            Some(end) => end,
            None => return false,
        };
        let cached = self.last_bulk.get();
        let selected = if self.index.bulk.get(cached).is_some_and(|span| {
            span.0 <= address
                && end <= span.1
                && self
                    .index
                    .bulk
                    .get(cached + 1)
                    .is_none_or(|next| next.0 > address)
        }) {
            // Reuse only while this is still the nearest starting span. A
            // nested/overlapping span beginning at or before the new request
            // forces the full search, even if this cached span encloses it.
            cached
        } else {
            // Prefer the nearest enclosing span. A smaller nested array must
            // not hide later parent fields or requests extending beyond it.
            let index = self.index.bulk.partition_point(|span| span.0 <= address);
            let enclosing = self.index.bulk[..index]
                .iter()
                .enumerate()
                .rev()
                .take_while(|(_, span)| span.3 >= end)
                .find(|(_, span)| span.1 >= end);
            let Some((selected, _)) = enclosing else {
                return false;
            };
            self.last_bulk.set(selected);
            selected
        };
        let (start, _, source_index, _) = self.index.bulk[selected];
        let offset = self.spans[source_index].start as usize + (address - start);
        let Some(source) = self.bytes.get(offset..offset + destination.len()) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

impl Drop for Recorded<'_> {
    fn drop(&mut self) {
        // Simultaneous readers get independent cursors. Only one cached index
        // is retained per payload; any additional index is dropped off-thread.
        let mut recycled = self.recycle.borrow_mut();
        if recycled.is_none() {
            *recycled = Some(std::mem::take(&mut self.index));
        }
    }
}

impl Memory for Recorded<'_> {
    fn immutable_view(
        &self,
        address: usize,
        len: usize,
    ) -> Option<engine::memory::SliceMemory<'_>> {
        if !self.index.has_bulk || address == 0 || len == 0 || !is_canonical_range(address, len) {
            return None;
        }
        let end = address.checked_add(len)?;
        let mut enclosing = None;
        for span in self.spans {
            if span.len == 0 {
                continue;
            }
            let span_end = span.addr.checked_add(span.len as usize)?;
            if span_end <= address || end <= span.addr {
                continue;
            }
            // Exactly one observation may overlap the requested view. This
            // rejects ordinary exact reads (including failures), duplicates,
            // and nested/overlapping/tied bulk winners without cursor access.
            if enclosing.is_some()
                || !span.ok
                || !span.bulk
                || span.addr == 0
                || !is_canonical_range(span.addr, span.len as usize)
                || span.addr > address
                || span_end < end
                // A whole bulk is itself an exact key. Keep that observation
                // on the ordinary path even when its current key is unique.
                || (span.addr == address && span_end == end)
            {
                return None;
            }
            enclosing = Some(span);
        }
        let span = enclosing?;
        let start = span.start as usize;
        let source = self
            .bytes
            .get(start..start.checked_add(span.len as usize)?)?;
        let offset = address - span.addr;
        Some(engine::memory::SliceMemory {
            base: address,
            bytes: source.get(offset..offset.checked_add(len)?)?,
        })
    }

    #[inline(always)]
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        if address == 0 || address.checked_add(destination.len()).is_none() {
            return false;
        }
        // Mirror the live canonical gate so the replay fails the same way for
        // a noncanonical derived pointer even if a log ever contained it.
        // `take_inner` never records such spans, so this is unreachable in
        // normal decodes and only keeps outcomes identical by construction.
        if !is_canonical_range(address, destination.len()) {
            return false;
        }
        if destination.len() > u32::MAX as usize {
            return false;
        }
        self.serve(address, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::memory::SliceMemory;

    fn immutable_view_positions(reader: &Recorded<'_>) -> (Vec<usize>, usize, usize) {
        (
            reader
                .index
                .slots
                .iter()
                .map(std::cell::Cell::get)
                .collect(),
            reader.next_exact.get(),
            reader.last_bulk.get(),
        )
    }

    fn immutable_view_log() -> RawLog<1> {
        let mut log = RawLog::new(16, 512);
        log.has_bulk = true;
        let bytes: Vec<u8> = (0..64).collect();
        assert_eq!(
            log.take_bulk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                64,
                1
            ),
            Some(0)
        );
        log
    }

    #[test]
    fn immutable_view_borrows_current_bytes_without_changing_independent_cursors() {
        let mut log = immutable_view_log();
        for value in [71, 93] {
            assert!(
                log.take(
                    &SliceMemory {
                        base: 0x3000,
                        bytes: &[value]
                    },
                    0x3000,
                    1,
                    1
                )
                .is_some()
            );
        }
        // An empty observation cannot override a nonempty contained request.
        log.spans.push(Span {
            addr: 0x1010,
            len: 0,
            ok: false,
            mask: 1,
            start: 0,
            bulk: false,
        });
        {
            let first = Recorded::build(&log);
            let second = Recorded::build(&log);
            let mut outside = [0];
            assert!(first.copy(0x3000, &mut outside));
            assert_eq!(outside, [71]);
            let before = immutable_view_positions(&first);
            let view = first.immutable_view(0x1001, 62).unwrap();
            assert_eq!(view.base, 0x1001);
            assert!(std::ptr::eq(view.bytes.as_ptr(), log.bytes[1..].as_ptr()));
            for (address, len) in [(0x1001, 1), (0x1007, 9), (0x103d, 2)] {
                let mut direct = vec![0; len];
                let mut replay = vec![0; len];
                assert!(view.copy(address, &mut direct));
                assert!(second.copy(address, &mut replay));
                assert_eq!(direct, replay);
            }
            assert_eq!(immutable_view_positions(&first), before);
            assert!(first.immutable_view(0x1000, 64).is_none());
            assert_eq!(immutable_view_positions(&first), before);
            assert!(first.copy(0x3000, &mut outside));
            assert_eq!(outside, [93]);
            assert!(second.copy(0x3000, &mut outside));
            assert_eq!(outside, [71]);
            assert!(second.copy(0x3000, &mut outside));
            assert_eq!(outside, [93]);
            assert!(first.copy(0x3000, &mut outside));
            assert_eq!(outside, [93]);
        }
        // The cached shape excludes start and payload bytes. A new reader
        // must use this job's current start/used arena, never a cached slice.
        log.bytes.extend_from_slice(&[0xa7; 64]);
        let current_start = log.bytes.len() - 64;
        log.spans[0].start = current_start as u32;
        log.spans[0].mask = 0;
        let reader = Recorded::build(&log);
        assert_eq!(
            reader.immutable_view(0x1001, 62).unwrap().bytes,
            &[0xa7; 62]
        );
    }

    #[test]
    fn immutable_view_rejects_every_competing_span_without_advancing_replay() {
        for (address, len, ok, bulk) in [
            (0x1008, 4, true, false),   // successful exact override
            (0x1008, 4, false, false),  // failed exact override
            (0x1000, 64, false, false), // failed whole-bulk exact key
            (0x1000, 64, true, false),  // ordinary whole-bulk exact key
            (0x1008, 16, true, true),   // nested
            (0x0ff0, 96, true, true),   // parent
            (0x1000, 32, true, true),   // tied start, smaller
            (0x1000, 96, true, true),   // tied start, larger
            (0x1000, 64, true, true),   // duplicate whole bulk
            (0x0ff0, 24, true, true),   // left overlap
            (0x1030, 32, true, true),   // right overlap
        ] {
            let mut log = immutable_view_log();
            log.bytes.extend_from_slice(&[0xe1; 96]);
            log.spans.push(Span {
                addr: address,
                len,
                ok,
                mask: 1,
                start: 64,
                bulk,
            });
            let reader = Recorded::build(&log);
            let before = immutable_view_positions(&reader);
            assert!(
                reader.immutable_view(0x1004, 56).is_none(),
                "{address:x}/{len}/{ok}/{bulk}"
            );
            assert_eq!(immutable_view_positions(&reader), before);
        }
        let mut log = immutable_view_log();
        for value in [Some(11), None, Some(29)] {
            match value {
                Some(value) => {
                    assert!(
                        log.take(
                            &SliceMemory {
                                base: 0x1008,
                                bytes: &[value; 4]
                            },
                            0x1008,
                            4,
                            1
                        )
                        .is_some()
                    );
                }
                None => log.note_fail(0x1008, 4, 1),
            }
        }
        let reader = Recorded::build(&log);
        for value in [Some(11), None, Some(29), Some(29)] {
            let before = immutable_view_positions(&reader);
            assert!(reader.immutable_view(0x1004, 56).is_none());
            assert_eq!(immutable_view_positions(&reader), before);
            let mut bytes = [0xcc; 4];
            assert_eq!(reader.copy(0x1008, &mut bytes), value.is_some());
            assert_eq!(bytes, [value.unwrap_or(0xcc); 4]);
        }
        // An exact key outside the requested view does not poison admission.
        assert!(reader.immutable_view(0x1020, 16).is_some());
    }

    #[test]
    fn immutable_view_declines_empty_full_invalid_ranges_and_unpublished_storage() {
        let log = immutable_view_log();
        let reader = Recorded::build(&log);
        for (address, len) in [
            (0x1000, 0),
            (0, 1),
            (0x1000, 64),
            (0x0fff, 2),
            (0x103f, 2),
            (usize::MAX - 1, 4),
            (0x0000_8000_0000_0000, 1),
            (0x0000_7fff_ffff_ffff, 2),
            (0x1000, u32::MAX as usize + 1),
        ] {
            let before = immutable_view_positions(&reader);
            assert!(
                reader.immutable_view(address, len).is_none(),
                "{address:x}/{len}"
            );
            assert_eq!(immutable_view_positions(&reader), before);
        }
        drop(reader);
        for mode in 0..6 {
            let mut log = immutable_view_log();
            match mode {
                0 => log.has_bulk = false,
                1 => log.spans[0].ok = false,
                2 => log.spans[0].bulk = false,
                3 => log.spans[0].start = u32::MAX,
                4 => {
                    // Dirty initialized capacity is not published input.
                    log.bytes.clear();
                    log.bytes.extend_from_slice(&[0xb5; 63]);
                }
                5 => log.spans.push(Span {
                    addr: usize::MAX,
                    len: 2,
                    ok: false,
                    mask: 1,
                    start: 0,
                    bulk: false,
                }),
                _ => unreachable!(),
            }
            let reader = Recorded::build(&log);
            assert!(reader.immutable_view(0x1001, 62).is_none(), "mode {mode}");
        }
    }

    #[test]
    fn capacity_accounting_follows_raw_growth_clear_and_independent_reader_lifetimes() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[7; 32],
        };
        let mut log = RawLog::<1>::new(4, 8);
        log.capacity_charge = Charge::empty_on(&LOCAL);
        log.update_capacity_charge();
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
        // A fresh empty reader has no index-vector allocation to count.
        drop(Recorded::build(&log));
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
        let initial = LOCAL.snapshot().current;
        assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
        assert!(log.bytes.is_empty());
        assert!(log.bytes.storage.iter().all(|&byte| byte == 0));
        assert_eq!(
            initial.payload_vectors,
            vector_bytes(&log.spans) + log.bytes.capacity() as u64
        );
        assert_eq!(log.take(&memory, 0x1000, 16, 1), None);
        log.grow();
        let grown = LOCAL.snapshot().current;
        assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
        assert!(log.bytes.is_empty());
        assert!(log.bytes.storage.iter().all(|&byte| byte == 0));
        assert!(grown.payload_vectors > initial.payload_vectors);
        assert_eq!(
            grown.payload_vectors,
            vector_bytes(&log.spans) + log.bytes.capacity() as u64
        );
        log.clear();
        assert_eq!(LOCAL.snapshot().current, grown);
        assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(0));
        assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(4));
        assert_eq!(log.take_bulk(&memory, 0x1008, 8, 1), Some(8));

        let first = Recorded::build(&log);
        let one_index = LOCAL.snapshot().current;
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(
            one_index.index_vectors,
            vector_bytes(&first.index.slots)
                + vector_bytes(&first.index.next)
                + vector_bytes(&first.index.repeated)
                + vector_bytes(&first.index.shape)
                + vector_bytes(&first.index.bulk)
        );
        assert_eq!(one_index.index_map_entries_lower_bound, 0);
        assert_eq!(one_index.payload_vectors, grown.payload_vectors);
        let second = Recorded::build(&log);
        let two_indexes = LOCAL.snapshot().current;
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 2);
        assert_eq!(two_indexes.index_vectors, one_index.index_vectors * 2);
        assert_eq!(
            two_indexes.index_map_entries_lower_bound,
            one_index.index_map_entries_lower_bound * 2
        );
        drop(first); // Its index moves into the cache: still charged once.
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 2);
        assert_eq!(LOCAL.snapshot().current, two_indexes);
        drop(second); // Occupied cache causes this independent index to free.
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(LOCAL.snapshot().current, one_index);
        let reused = Recorded::build(&log);
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(LOCAL.snapshot().current, one_index);
        drop(reused);
        log.clear();
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(LOCAL.snapshot().current, one_index);
        // Preparing an empty shape retains the previous index allocations.
        drop(Recorded::build(&log));
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(LOCAL.snapshot().current, one_index);
        let peak = LOCAL.snapshot().peak_total_lower_bound;
        let moved = Some(log);
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 1);
        assert_eq!(LOCAL.snapshot().current, one_index);
        drop(moved);
        assert_eq!(LOCAL.snapshot().allocated_index_owners, 0);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
        assert_eq!(LOCAL.snapshot().peak_total_lower_bound, peak);
    }

    #[test]
    fn initialized_arena_clear_growth_and_equality_expose_only_the_current_prefix() {
        let mut log = RawLog::<1>::new(8, 16);
        let mut other = RawLog::<1>::new(8, 32);
        log.bytes.storage.fill(0xa7);
        other.bytes.storage.fill(0x3e);
        assert!(log.logs_equal(&other));
        assert_eq!(format!("{:?}", log.bytes), "[]");
        assert_eq!(log.bytes.first(), None);
        let initial_pointer = log.bytes.as_ptr();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[9, 8, 7, 6],
        };
        assert_eq!(log.take(&memory, 0x1000, 4, 1), Some(0));
        assert_eq!(other.take(&memory, 0x1000, 4, 1), Some(0));
        assert!(log.logs_equal(&other));
        assert_eq!(format!("{:?}", log.bytes), "[9, 8, 7, 6]");
        assert!(log.bytes.get(4).is_none());
        log.clear();
        assert_eq!(log.bytes.as_ptr(), initial_pointer);
        assert!(log.bytes.is_empty());
        assert_eq!(&log.bytes.storage[..4], &[9, 8, 7, 6]);
        assert!(!Recorded::build(&log).copy(0x1000, &mut [0; 4]));

        log.bytes.extend_from_slice(&[1, 2, 3]);
        let capacity = log.bytes.capacity();
        let old_storage = log.bytes.storage.clone();
        log.required_bytes = capacity + 1;
        log.overflowed = true;
        log.grow();
        assert!(log.bytes.capacity() > capacity);
        assert_eq!(log.bytes.storage.len(), log.bytes.capacity());
        assert_eq!(&log.bytes.storage[..capacity], old_storage);
        assert!(log.bytes.storage[capacity..].iter().all(|&byte| byte == 0));
        assert_eq!(log.bytes, [1, 2, 3]);
        assert!(log.bytes.get(3).is_none());
        log.clear();
        assert!(log.bytes.is_empty());
        assert!(log.bytes.storage.iter().any(|&byte| byte != 0));
        assert_eq!(log.take_bulk(&memory, 0x1000, 4, 1), Some(0));
        assert_eq!(log.bytes, [9, 8, 7, 6]);
    }

    #[test]
    fn initialized_arena_gathers_hide_dirty_failed_fields_and_unread_suffixes() {
        struct PartialSource {
            fail: usize,
            partial: usize,
            calls: RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for PartialSource {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.borrow_mut().push((address, destination.len()));
                let complete = address != self.fail;
                let len = if complete {
                    destination.len()
                } else {
                    self.partial.min(destination.len())
                };
                destination[..len].fill((address / 8) as u8);
                complete
            }
        }
        for completed in 0..=4 {
            for partial in [0, 3, 7] {
                let mut log = RawLog::<2>::new(8, 64);
                log.bytes.storage.fill(0xda);
                log.bytes.extend_from_slice(&[0xb3, 0x91, 0x7e]);
                let source = PartialSource {
                    fail: 0x1000 + completed * 8,
                    partial,
                    calls: RefCell::default(),
                };
                assert_eq!(
                    log.take_array_chunk(&source, 0x1000, 8, 4, 2),
                    Some((3, completed))
                );
                let mut expected = vec![0xb3, 0x91, 0x7e];
                for index in 0..completed {
                    expected.extend_from_slice(&[(0x1000 / 8 + index) as u8; 8]);
                }
                assert_eq!(log.bytes, expected);
                assert!(log.bytes.get(expected.len()).is_none());
                assert_eq!(source.calls.borrow().len(), (completed + 1).min(4));
                assert_eq!(log.raw_bytes, [0, (completed * 8) as u64]);
                assert_eq!(log.fault, [false, completed != 4]);
                if completed != 4 {
                    let reader = Recorded::build(&log);
                    let mut failed = [0xe7; 8];
                    assert!(!reader.copy(source.fail, &mut failed));
                    assert_eq!(failed, [0xe7; 8]);
                }
                // Resume through a separate real observation at the exact
                // failed-prefix end; old partial and unattempted bytes cannot
                // become a successful span without this new full copy.
                let current = SliceMemory {
                    base: 0x2000,
                    bytes: &[31; 4],
                };
                assert_eq!(log.take(&current, 0x2000, 4, 1), Some(expected.len()));
                expected.extend_from_slice(&[31; 4]);
                assert_eq!(log.bytes, expected);
            }
        }
        for partial in [0, 2, 3] {
            let mut log = RawLog::<2>::new(8, 32);
            log.bytes.storage.fill(0xe3);
            log.bytes.extend_from_slice(&[0xb3, 0x91, 0x7e]);
            let source = PartialSource {
                fail: 0x1008,
                partial,
                calls: RefCell::default(),
            };
            assert_eq!(
                log.take_scattered(
                    &source,
                    [0x1000, usize::MAX, 0x1008, 0x1010],
                    [8, 0, 4, 4],
                    [1, 1, 2, 1],
                ),
                Some(([3, 11, 11, 15], 2))
            );
            assert_eq!(*source.calls.borrow(), vec![(0x1000, 8), (0x1008, 4)]);
            assert_eq!(log.bytes, [0xb3, 0x91, 0x7e, 0, 0, 0, 0, 0, 0, 0, 0]);
            assert!(log.bytes.get(11).is_none());
            assert_eq!(log.raw_bytes, [8, 0]);
            assert_eq!(log.fault, [false, true]);
            let reader = Recorded::build(&log);
            assert!(!reader.copy(0x1008, &mut [0; 4]));
            assert!(!reader.copy(0x1010, &mut [0; 4]));
        }
    }

    /// Frozen pre-specialization helpers keep the original full-array address
    /// preparation and stateful scatter splitter as the differential reference.
    fn raw_specialization_frozen_scattered<const M: usize>(
        log: &mut RawLog<3>,
        memory: &impl Memory,
        addresses: [usize; M],
        lengths: [usize; M],
        masks: [u32; M],
    ) -> Option<([usize; M], usize)> {
        if log.overflowed {
            return None;
        }
        let start = log.bytes.len();
        let mut end = start;
        let mut starts = [0; M];
        for (index, len) in lengths.into_iter().enumerate() {
            starts[index] = end;
            let Some(next) = end.checked_add(len).filter(|_| len <= u32::MAX as usize) else {
                log.overflowed = true;
                return None;
            };
            end = next;
        }
        let spans = lengths.iter().filter(|&&len| len != 0).count();
        if spans > log.spans.capacity() - log.spans.len()
            || end > log.bytes.capacity()
            || end > u32::MAX as usize
        {
            log.span_exhausted = spans > log.spans.capacity() - log.spans.len();
            log.required_bytes = end;
            log.overflowed = true;
            return None;
        }
        let completed = {
            let mut remaining = &mut log.bytes.storage[start..end];
            let mut destinations: [&mut [u8]; M] = std::array::from_fn(|index| {
                let (field, rest) = std::mem::take(&mut remaining).split_at_mut(lengths[index]);
                remaining = rest;
                field
            });
            let guarded = std::array::from_fn(|index| {
                if is_canonical_range(addresses[index], lengths[index]) {
                    addresses[index]
                } else {
                    0
                }
            });
            memory.copy_scattered(guarded, &mut destinations)
        };
        debug_assert!(completed <= M);
        log.bytes.used = if completed == M {
            end
        } else {
            starts[completed]
        };
        for index in 0..completed {
            let len = lengths[index];
            if len != 0 {
                log.spans.push(Span {
                    addr: addresses[index],
                    len: len as u32,
                    ok: true,
                    mask: masks[index],
                    start: starts[index] as u32,
                    bulk: false,
                });
                log.raw_bytes[lowest_table(masks[index])] += len as u64;
            }
        }
        if completed != M {
            log.note_fail(
                addresses[completed],
                lengths[completed] as u32,
                masks[completed],
            );
        }
        Some((starts, completed))
    }

    fn raw_specialization_frozen_array(
        log: &mut RawLog<3>,
        memory: &impl Memory,
        address: usize,
        width: usize,
        count: usize,
        mask: u32,
    ) -> Option<(usize, usize)> {
        if log.overflowed || count > 64 || width == 0 || width > u32::MAX as usize {
            return None;
        }
        let start = log.bytes.len();
        if count == 0 {
            return Some((start, 0));
        }
        let size = count.checked_mul(width)?;
        let end = start.checked_add(size)?;
        if log.spans.capacity() - log.spans.len() < 2
            || end > log.bytes.capacity()
            || end > u32::MAX as usize
        {
            log.span_exhausted = log.spans.capacity() - log.spans.len() < 2;
            log.required_bytes = end;
            log.overflowed = true;
            return None;
        }
        let addresses: [usize; 64] =
            std::array::from_fn(|index| address.checked_add(index * width).unwrap_or(0));
        let guarded = addresses.map(|address| {
            if is_canonical_range(address, width) {
                address
            } else {
                0
            }
        });
        let fields = [engine::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: width,
        }];
        let completed = memory.copy_records(
            engine::memory::RecordSources::Pointers(&guarded[..count]),
            &engine::memory::RecordLayout::new(&fields, width).unwrap(),
            &mut log.bytes.storage[start..end],
        );
        debug_assert!(completed <= count);
        log.bytes.used = start + completed * width;
        if completed != 0 {
            log.spans.push(Span {
                addr: address,
                len: (completed * width) as u32,
                ok: true,
                mask,
                start: start as u32,
                bulk: true,
            });
            log.has_bulk = true;
            log.raw_bytes[lowest_table(mask)] += (completed * width) as u64;
        }
        if completed != count {
            log.note_fail(addresses[completed], width as u32, mask);
        }
        Some((start, completed))
    }

    #[derive(Default)]
    struct RawSpecializationSource {
        fail_call: Option<usize>,
        calls: RefCell<Vec<(usize, usize)>>,
    }

    impl Memory for RawSpecializationSource {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let call = self.calls.borrow().len();
            self.calls.borrow_mut().push((address, destination.len()));
            let ok = self.fail_call != Some(call);
            let copied = if ok {
                destination.len()
            } else {
                destination.len() / 2
            };
            for (offset, byte) in destination[..copied].iter_mut().enumerate() {
                *byte = (address as u8)
                    .wrapping_add((call as u8).wrapping_mul(17))
                    .wrapping_add(offset as u8);
            }
            ok
        }
    }

    fn raw_specialization_log(span_capacity: usize, byte_capacity: usize) -> RawLog<3> {
        let mut log = RawLog::new(span_capacity, byte_capacity);
        log.bytes.storage.fill(0xd7);
        if byte_capacity >= 3 {
            log.bytes.extend_from_slice(&[0x91, 0x72, 0x53]);
        }
        log
    }

    fn assert_raw_specialization_equal(actual: &RawLog<3>, expected: &RawLog<3>) {
        assert_eq!(actual.spans, expected.spans);
        assert_eq!(actual.bytes, expected.bytes);
        assert_eq!(actual.bytes.storage, expected.bytes.storage);
        assert_eq!(actual.fault, expected.fault);
        assert_eq!(actual.raw_bytes, expected.raw_bytes);
        assert_eq!(actual.has_bulk, expected.has_bulk);
        assert_eq!(actual.overflowed, expected.overflowed);
        assert_eq!(actual.span_exhausted, expected.span_exhausted);
        assert_eq!(actual.required_bytes, expected.required_bytes);
        assert_eq!(actual.spans.capacity(), expected.spans.capacity());
        assert_eq!(actual.bytes.capacity(), expected.bytes.capacity());
        assert_eq!(actual.grow_for_next(), expected.grow_for_next());
    }

    #[test]
    fn raw_specialization_array_prefix_matches_frozen_reads_and_faults() {
        for count in [0, 1, 63, 64] {
            for address in [
                0x1000,
                0,
                0x0000_7fff_ffff_fff0,
                0x0000_8000_0000_0000,
                usize::MAX - 15,
                usize::MAX - 3,
            ] {
                for fail in 0..=count {
                    let fail_call = (fail < count).then_some(fail);
                    let actual_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let expected_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let mut actual = raw_specialization_log(8, 1024);
                    let mut expected = raw_specialization_log(8, 1024);
                    assert_eq!(
                        actual.take_array_chunk(&actual_source, address, 8, count, 6),
                        raw_specialization_frozen_array(
                            &mut expected,
                            &expected_source,
                            address,
                            8,
                            count,
                            6,
                        ),
                        "count={count}, address={address:#x}, fail={fail}"
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                    assert_eq!(
                        *actual_source.calls.borrow(),
                        *expected_source.calls.borrow()
                    );
                    assert!(actual_source.calls.borrow().len() <= count);
                    if count == 0 {
                        assert!(actual_source.calls.borrow().is_empty());
                        assert!(actual.spans.is_empty());
                    }
                    // Later independent observations reuse only the published
                    // prefix; a partially overwritten failed destination stays
                    // hidden unless this fresh copy fills it successfully.
                    let actual_resume = RawSpecializationSource::default();
                    let expected_resume = RawSpecializationSource::default();
                    assert_eq!(
                        actual.take(&actual_resume, 0x4000, 4, 1),
                        expected.take(&expected_resume, 0x4000, 4, 1)
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                }
            }
        }
    }

    #[test]
    fn raw_specialization_array_admission_and_growth_match_frozen() {
        for count in [0, 1, 63, 64, 65] {
            for (span_capacity, byte_capacity) in [
                (0, 0),
                (1, 1024),
                (2, 0),
                (2, 3),
                (2, 10),
                (2, 11),
                (2, 514),
                (2, 515),
            ] {
                let mut actual = raw_specialization_log(span_capacity, byte_capacity);
                let mut expected = raw_specialization_log(span_capacity, byte_capacity);
                let actual_source = RawSpecializationSource::default();
                let expected_source = RawSpecializationSource::default();
                assert_eq!(
                    actual.take_array_chunk(&actual_source, 0x1000, 8, count, 1),
                    raw_specialization_frozen_array(
                        &mut expected,
                        &expected_source,
                        0x1000,
                        8,
                        count,
                        1,
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(
                    *actual_source.calls.borrow(),
                    *expected_source.calls.borrow()
                );
                if actual.grow_for_next() {
                    actual.grow();
                    expected.grow();
                    assert_raw_specialization_equal(&actual, &expected);
                    actual.clear();
                    expected.clear();
                    assert_raw_specialization_equal(&actual, &expected);
                    let actual_retry = RawSpecializationSource::default();
                    let expected_retry = RawSpecializationSource::default();
                    assert_eq!(
                        actual.take_array_chunk(&actual_retry, 0x1000, 8, count, 1),
                        raw_specialization_frozen_array(
                            &mut expected,
                            &expected_retry,
                            0x1000,
                            8,
                            count,
                            1,
                        )
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                    assert_eq!(*actual_retry.calls.borrow(), *expected_retry.calls.borrow());
                }
            }
        }
        for width in [0, u32::MAX as usize, u32::MAX as usize + 1] {
            for count in [0, 1, 64] {
                let mut actual = raw_specialization_log(2, 16);
                let mut expected = raw_specialization_log(2, 16);
                let actual_source = RawSpecializationSource::default();
                let expected_source = RawSpecializationSource::default();
                assert_eq!(
                    actual.take_array_chunk(&actual_source, usize::MAX - 3, width, count, 1),
                    raw_specialization_frozen_array(
                        &mut expected,
                        &expected_source,
                        usize::MAX - 3,
                        width,
                        count,
                        1,
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert!(actual_source.calls.borrow().is_empty());
                assert!(expected_source.calls.borrow().is_empty());
            }
        }
    }

    /// One guarded copy per chunk records the log per-entry takes record:
    /// the same exact spans, bytes, partial failed bytes, first failure and
    /// source requests, with no bulk span. Admission failures (null,
    /// noncanonical, overflowing) and read faults at every position alike.
    #[test]
    fn array_entries_record_the_per_entry_takes_log() {
        for count in [0, 1, 2, 63, 64] {
            for address in [
                0x1000,
                0,
                0x0000_7fff_ffff_fff0,
                0x0000_8000_0000_0000,
                usize::MAX - 15,
                usize::MAX - 3,
            ] {
                for fail in 0..=count {
                    let fail_call = (fail < count).then_some(fail);
                    let actual_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let expected_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let mut actual = raw_specialization_log(80, 1024);
                    let mut expected = raw_specialization_log(80, 1024);
                    let start = expected.bytes.len();
                    let completed = (0..count)
                        .take_while(|&index| {
                            expected
                                .take(&expected_source, address.wrapping_add(index * 8), 8, 6)
                                .is_some()
                        })
                        .count();
                    assert_eq!(
                        actual.take_array_entries(&actual_source, address, 8, count, 6),
                        Some((start, completed)),
                        "count={count}, address={address:#x}, fail={fail}"
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                    assert!(!actual.has_bulk);
                    assert_eq!(
                        *actual_source.calls.borrow(),
                        *expected_source.calls.borrow()
                    );
                }
            }
        }
    }

    /// Too few spans for every entry and one failure overflow before any
    /// source request, and growth then admits the chunk.
    #[test]
    fn array_entries_preflight_every_span_before_copying() {
        let mut log = raw_specialization_log(64, 1024);
        let source = RawSpecializationSource::default();
        assert_eq!(log.take_array_entries(&source, 0x1000, 8, 64, 1), None);
        assert!(log.overflowed && log.span_exhausted && log.grow_for_next());
        assert!(log.spans.is_empty() && source.calls.borrow().is_empty());
        log.grow();
        log.clear();
        assert_eq!(
            log.take_array_entries(&source, 0x1000, 8, 64, 1),
            Some((0, 64))
        );
        assert_eq!(log.spans.len(), 64);
    }

    #[test]
    fn raw_specialization_scattered_split_matches_frozen_mixed_and_empty_fields() {
        let lengths = [0, 1, 4, 0, 8, 1, 32, 4, 0];
        let masks = [1, 1, 2, 4, 3, 1, 2, 4, 1];
        let original = [
            usize::MAX,
            0x1000,
            0x1004,
            0,
            0x1010,
            0x1000,
            0x1020,
            0x1004,
            usize::MAX,
        ];
        // Both repeated exact keys see distinct current bytes because the
        // synthetic source changes its payload on each actual observation.
        for replaced in 0..=lengths.len() {
            for replacement in [0, 0x0000_8000_0000_0000, usize::MAX - 3] {
                let mut addresses = original;
                if replaced < addresses.len() {
                    addresses[replaced] = replacement;
                }
                for fail in 0..=6 {
                    let fail_call = (fail < 6).then_some(fail);
                    let actual_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let expected_source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let mut actual = raw_specialization_log(16, 128);
                    let mut expected = raw_specialization_log(16, 128);
                    assert_eq!(
                        actual.take_scattered(&actual_source, addresses, lengths, masks),
                        raw_specialization_frozen_scattered(
                            &mut expected,
                            &expected_source,
                            addresses,
                            lengths,
                            masks,
                        )
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                    assert_eq!(
                        *actual_source.calls.borrow(),
                        *expected_source.calls.borrow()
                    );
                }
            }
        }
        for span_capacity in 0..=9 {
            for byte_capacity in [0, 3, 4, 8, 16, 52, 53, 128] {
                let mut actual = raw_specialization_log(span_capacity, byte_capacity);
                let mut expected = raw_specialization_log(span_capacity, byte_capacity);
                let actual_source = RawSpecializationSource::default();
                let expected_source = RawSpecializationSource::default();
                assert_eq!(
                    actual.take_scattered(&actual_source, original, lengths, masks),
                    raw_specialization_frozen_scattered(
                        &mut expected,
                        &expected_source,
                        original,
                        lengths,
                        masks,
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(
                    *actual_source.calls.borrow(),
                    *expected_source.calls.borrow()
                );
            }
        }
        let mut actual = raw_specialization_log(0, 0);
        let mut expected = raw_specialization_log(0, 0);
        let actual_source = RawSpecializationSource::default();
        let expected_source = RawSpecializationSource::default();
        assert_eq!(
            actual.take_scattered(&actual_source, [usize::MAX; 9], [0; 9], [1; 9]),
            raw_specialization_frozen_scattered(
                &mut expected,
                &expected_source,
                [usize::MAX; 9],
                [0; 9],
                [1; 9],
            )
        );
        assert_raw_specialization_equal(&actual, &expected);
        assert_eq!(
            actual.take_scattered(&actual_source, [], [], []),
            raw_specialization_frozen_scattered(&mut expected, &expected_source, [], [], [])
        );
        assert_raw_specialization_equal(&actual, &expected);
        assert!(actual_source.calls.borrow().is_empty());
        assert!(expected_source.calls.borrow().is_empty());
    }

    #[test]
    fn prefetched_scattered_matches_frozen_faults_masks_duplicates_and_dirty_suffixes() {
        let lengths = [0, 8, 12, 8, 1, 32, 0, 8];
        let masks = [0, 3, 2, 4, 0, 5, 1, 2];
        let original = [
            usize::MAX,
            0x1000,
            0x1010,
            0x1020,
            0x1030,
            0x1040,
            0,
            0x1000,
        ];
        for replaced in 0..=original.len() {
            for replacement in [0, 0x0000_8000_0000_0000, usize::MAX - 3] {
                let mut addresses = original;
                if replaced < addresses.len() {
                    addresses[replaced] = replacement;
                }
                for fail in 0..=6 {
                    let fail_call = (fail < 6).then_some(fail);
                    let source = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let reference = RawSpecializationSource {
                        fail_call,
                        ..Default::default()
                    };
                    let mut actual = raw_specialization_log(16, 128);
                    let mut expected = raw_specialization_log(16, 128);
                    assert_eq!(
                        actual.take_scattered_prefetched(&source, addresses, lengths, masks),
                        raw_specialization_frozen_scattered(
                            &mut expected,
                            &reference,
                            addresses,
                            lengths,
                            masks,
                        )
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                    assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
                    assert!(actual.bytes.get(actual.bytes.len()).is_none());
                    // A later independent field overwrites only its new span;
                    // dirty failed bytes and untouched suffixes stay unpublished.
                    let resumed = RawSpecializationSource::default();
                    let resumed_reference = RawSpecializationSource::default();
                    assert_eq!(
                        actual.take(&resumed, 0x2000, 4, 1),
                        expected.take(&resumed_reference, 0x2000, 4, 1)
                    );
                    assert_raw_specialization_equal(&actual, &expected);
                }
            }
        }
    }

    #[test]
    fn prefetched_scattered_preserves_capacity_empty_and_length_admission() {
        for span_capacity in 0..=6 {
            for byte_capacity in 0..=65 {
                let mut actual = raw_specialization_log(span_capacity, byte_capacity);
                let mut expected = raw_specialization_log(span_capacity, byte_capacity);
                let source = RawSpecializationSource::default();
                let reference = RawSpecializationSource::default();
                assert_eq!(
                    actual.take_scattered_prefetched(
                        &source,
                        [0x1000, 0x1010, 0x1020, 0x1030, 0x1040],
                        [8, 12, 8, 1, 32],
                        [1; 5],
                    ),
                    expected.take_scattered(
                        &reference,
                        [0x1000, 0x1010, 0x1020, 0x1030, 0x1040],
                        [8, 12, 8, 1, 32],
                        [1; 5],
                    )
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert_eq!(*source.calls.borrow(), *reference.calls.borrow());
                if actual.grow_for_next() {
                    actual.grow();
                    expected.grow();
                    assert_raw_specialization_equal(&actual, &expected);
                }
            }
        }
        for lengths in [
            [0, 0],
            [1, u32::MAX as usize],
            [u32::MAX as usize + 1, 0],
            [8, usize::MAX],
        ] {
            for overflowed in [false, true] {
                let mut actual = raw_specialization_log(2, 16);
                let mut expected = raw_specialization_log(2, 16);
                actual.overflowed = overflowed;
                expected.overflowed = overflowed;
                let source = RawSpecializationSource::default();
                assert_eq!(
                    actual.take_scattered_prefetched(&source, [usize::MAX; 2], lengths, [0, 3]),
                    expected.take_scattered(&source, [usize::MAX; 2], lengths, [0, 3])
                );
                assert_raw_specialization_equal(&actual, &expected);
                assert!(source.calls.borrow().is_empty());
            }
        }
        let mut empty = RawLog::<1>::default();
        let source = RawSpecializationSource::default();
        assert_eq!(
            empty.take_scattered_prefetched(&source, [], [], []),
            Some(([], 0))
        );
        assert_eq!(
            empty.take_scattered_prefetched(&source, [usize::MAX; 2], [0; 2], [1; 2]),
            Some(([0; 2], 2))
        );
        assert!(empty.spans.is_empty());
        assert!(empty.bytes.is_empty());
        assert!(!empty.overflowed);
        assert!(source.calls.borrow().is_empty());
    }

    #[test]
    fn prefetched_scattered_selects_only_the_requested_provider_entry() {
        #[derive(Default)]
        struct Provider {
            ordinary: std::cell::Cell<usize>,
            prefetched: std::cell::Cell<usize>,
        }
        impl Memory for Provider {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("raw scatter must use the provider's grouped operation")
            }
            fn copy_scattered<const M: usize>(
                &self,
                addresses: [usize; M],
                destinations: &mut [&mut [u8]; M],
            ) -> usize {
                self.ordinary.set(self.ordinary.get() + 1);
                SliceMemory {
                    base: 0x1000,
                    bytes: &[7; 8],
                }
                .copy_scattered(addresses, destinations)
            }
            fn copy_scattered_prefetched<const M: usize>(
                &self,
                addresses: [usize; M],
                destinations: &mut [&mut [u8]; M],
            ) -> usize {
                self.prefetched.set(self.prefetched.get() + 1);
                SliceMemory {
                    base: 0x1000,
                    bytes: &[7; 8],
                }
                .copy_scattered(addresses, destinations)
            }
        }
        let provider = Provider::default();
        let mut log = RawLog::<1>::new(4, 16);
        assert_eq!(
            log.take_scattered(&provider, [0x1000, 0x1004], [4; 2], [1; 2]),
            Some(([0, 4], 2))
        );
        assert_eq!((provider.ordinary.get(), provider.prefetched.get()), (1, 0));
        assert_eq!(
            log.take_scattered_prefetched(&provider, [0x1000, 0x1004], [4; 2], [1; 2]),
            Some(([8, 12], 2))
        );
        assert_eq!((provider.ordinary.get(), provider.prefetched.get()), (1, 1));
        assert_eq!(log.raw_bytes, [16]);
    }

    #[test]
    fn lowest_table_preserves_zero_single_bits_and_mixed_mask_anchors() {
        assert_eq!(lowest_table(0), 0);
        for bit in 0..32 {
            assert_eq!(lowest_table(1u32 << bit), bit as usize);
            for other in 0..32 {
                assert_eq!(
                    lowest_table((1u32 << bit) | (1u32 << other)),
                    bit.min(other) as usize
                );
            }
        }
        for (mask, expected) in [
            (u32::MAX, 0),
            (0x8000_003f, 0),
            (0xffff_ff00, 8),
            (0xaaaa_aaa0, 5),
            (0x5555_5500, 8),
            (0xf000_0000, 28),
        ] {
            assert_eq!(lowest_table(mask), expected);
        }
    }

    #[test]
    fn array_chunk_coalesces_exact_success_prefix_without_retry() {
        struct Source<'a> {
            memory: SliceMemory<'a>,
            calls: std::cell::Cell<usize>,
        }
        impl Memory for Source<'_> {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("expected one record gather")
            }
            fn copy_records(
                &self,
                sources: engine::memory::RecordSources<'_>,
                layout: &engine::memory::RecordLayout<'_>,
                destination: &mut [u8],
            ) -> usize {
                self.calls.set(self.calls.get() + 1);
                self.memory.copy_records(sources, layout, destination)
            }
        }
        for readable in 0..=4 {
            let bytes: Vec<u8> = (0..32).collect();
            let memory = Source {
                memory: SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..readable * 8],
                },
                calls: std::cell::Cell::new(0),
            };
            let mut log = RawLog::<2>::new(2, 32);
            assert_eq!(
                log.take_array_chunk(&memory, 0x1000, 8, 4, 2),
                Some((0, readable))
            );
            assert_eq!(memory.calls.get(), 1);
            assert_eq!(log.bytes, bytes[..readable * 8]);
            assert_eq!(log.raw_bytes, [0, (readable * 8) as u64]);
            assert_eq!(log.fault, [false, readable != 4]);
            assert_eq!(
                log.spans.len(),
                usize::from(readable != 0) + usize::from(readable != 4)
            );
            let recorded = Recorded::build(&log);
            for index in 0..4 {
                let mut field = [0; 8];
                assert_eq!(
                    recorded.copy(0x1000 + index * 8, &mut field),
                    index < readable
                );
                if index < readable {
                    assert_eq!(field, bytes[index * 8..index * 8 + 8]);
                }
            }
        }
    }

    #[test]
    fn array_chunk_capacity_failure_reads_nothing_and_empty_chunk_succeeds() {
        let mut log = RawLog::<1>::new(2, 8);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[3; 16],
        };
        assert_eq!(log.take_array_chunk(&memory, 0, 8, 0, 1), Some((0, 0)));
        assert_eq!(log.take_array_chunk(&memory, 0x1000, 8, 2, 1), None);
        assert!(log.overflowed);
        assert!(log.spans.is_empty());
        assert!(log.bytes.is_empty());
        log.grow();
        log.clear();
        assert_eq!(log.take_array_chunk(&memory, 0x1000, 8, 2, 1), Some((0, 2)));
    }

    #[test]
    fn raw_growth_stops_when_offsets_cannot_represent_the_required_arena() {
        let mut log = RawLog::<1> {
            overflowed: true,
            required_bytes: u32::MAX as usize,
            ..Default::default()
        };
        assert!(log.grow_for_next());
        log.required_bytes += 1;
        assert!(!log.grow_for_next());
    }

    #[test]
    fn scattered_copy_uses_one_gather_and_keeps_failed_field_masks() {
        struct Scatter<'a> {
            memory: SliceMemory<'a>,
            gathers: std::cell::Cell<usize>,
        }
        impl Memory for Scatter<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.memory.copy(address, destination)
            }
            fn copy_scattered<const M: usize>(
                &self,
                addresses: [usize; M],
                destinations: &mut [&mut [u8]; M],
            ) -> usize {
                self.gathers.set(self.gathers.get() + 1);
                self.memory.copy_scattered(addresses, destinations)
            }
        }
        let memory = Scatter {
            memory: SliceMemory {
                base: 0x1000,
                bytes: &[31; 8],
            },
            gathers: std::cell::Cell::new(0),
        };
        let mut log = RawLog::<2>::new(4, 32);
        assert_eq!(
            log.take_scattered(
                &memory,
                [0x1000, 0, 0x1008, 0x1000],
                [8, 0, 4, 4],
                [1, 1, 2, 1]
            ),
            Some(([0, 8, 8, 12], 2))
        );
        assert_eq!(memory.gathers.get(), 1);
        assert_eq!(log.spans.len(), 2);
        assert_eq!(log.bytes.len(), 8);
        assert_eq!(log.fault, [false, true]);
        assert_eq!(log.raw_bytes, [8, 0]);
        assert_eq!(log.spans[1].mask, 2);
        assert!(!log.spans[1].ok);
        let recorded = Recorded::build(&log);
        assert!(recorded.copy(0x1000, &mut [0; 8]));
        assert!(!recorded.copy(0x1008, &mut [0; 4]));
        assert!(!recorded.copy(0x1000, &mut [0; 4]));
    }

    #[test]
    fn scattered_overflow_declines_before_touching_any_source() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[7; 8],
        };
        let mut log = RawLog::<1>::new(1, 4);
        assert_eq!(
            log.take_scattered(&memory, [0x1000, 0x1004], [4, 4], [1, 1]),
            None
        );
        assert!(log.overflowed);
        assert!(log.spans.is_empty());
        assert!(log.bytes.is_empty());
        log.grow();
        log.clear();
        assert_eq!(
            log.take_scattered(&memory, [0x1000, 0x1004], [4, 4], [1, 1]),
            Some(([0, 4], 2))
        );
    }

    #[test]
    fn reused_index_reads_only_current_bytes_and_resets_duplicate_cursors() {
        let mut log = RawLog::<1>::new(4, 32);
        for (first, second) in [(7, 11), (23, 47)] {
            log.clear();
            for byte in [first, second] {
                let memory = SliceMemory {
                    base: 0x1000,
                    bytes: &[byte; 8],
                };
                log.take(&memory, 0x1000, 8, 1).unwrap();
            }
            let recorded = Recorded::build(&log);
            let mut bytes = [0; 8];
            assert!(recorded.copy(0x1000, &mut bytes));
            assert_eq!(bytes, [first; 8]);
            assert!(recorded.copy(0x1000, &mut bytes));
            assert_eq!(bytes, [second; 8]);
            assert!(recorded.copy(0x1000, &mut bytes));
            assert_eq!(bytes, [second; 8]);
        }
        assert!(log.index.borrow().is_some());
    }

    #[test]
    fn cached_index_rebuilds_when_address_or_failure_changes() {
        let mut log = RawLog::<1>::new(4, 32);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[9; 16],
        };
        log.take(&memory, 0x1000, 8, 1).unwrap();
        assert!(Recorded::build(&log).copy(0x1000, &mut [0; 8]));
        log.clear();
        log.take(&memory, 0x1008, 8, 1).unwrap();
        {
            let recorded = Recorded::build(&log);
            assert!(!recorded.copy(0x1000, &mut [0; 8]));
            assert!(recorded.copy(0x1008, &mut [0; 8]));
        }
        log.clear();
        assert!(log.take(&FailAll, 0x1008, 8, 1).is_none());
        assert!(!Recorded::build(&log).copy(0x1008, &mut [0; 8]));
    }

    #[test]
    fn cached_bulk_index_uses_current_pool_offsets_and_bulk_gate() {
        let mut log = RawLog::<1>::new(4, 32);
        log.has_bulk = true;
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[9; 16],
        };
        log.take_bulk(&memory, 0x1000, 16, 1).unwrap();
        assert!(Recorded::build(&log).copy(0x1004, &mut [0; 8]));
        let original = log.bytes.to_vec();
        log.bytes.clear();
        log.bytes.extend_from_slice(&[71; 8]);
        log.bytes.extend_from_slice(&original);
        log.spans[0].start += 8;
        let mut bytes = [0; 8];
        assert!(Recorded::build(&log).copy(0x1004, &mut bytes));
        assert_eq!(bytes, [9; 8]);
        log.has_bulk = false;
        assert!(!Recorded::build(&log).copy(0x1004, &mut bytes));
    }

    #[test]
    fn simultaneous_recorded_views_have_independent_cursors() {
        let mut log = RawLog::<1>::new(4, 32);
        for byte in [7, 19] {
            log.take(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &[byte; 8],
                },
                0x1000,
                8,
                1,
            )
            .unwrap();
        }
        let first = Recorded::build(&log);
        let second = Recorded::build(&log);
        let mut bytes = [0; 8];
        assert!(first.copy(0x1000, &mut bytes));
        assert!(first.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [19; 8]);
        assert!(second.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [7; 8]);
        drop(first);
        assert!(Recorded::build(&log).copy(0x1000, &mut bytes));
        assert_eq!(bytes, [7; 8]);
        assert!(second.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [19; 8]);
    }

    fn set_index_fixture(log: &mut RawLog<1>, reads: &[(usize, u32, Option<u8>, bool)]) {
        log.clear();
        for &(addr, len, value, bulk) in reads {
            let start = log.bytes.len() as u32;
            if let Some(value) = value {
                log.bytes.resize(log.bytes.len() + len as usize, value);
            }
            log.spans.push(Span {
                addr,
                len,
                ok: value.is_some(),
                mask: 1,
                start,
                bulk,
            });
            log.has_bulk |= bulk;
        }
    }

    // Deliberately linear reference: no hash index, cached shape or bulk-
    // search metadata is shared with the implementation.
    fn assert_recorded_query(
        log: &RawLog<1>,
        recorded: &Recorded<'_>,
        observations: &mut std::collections::BTreeMap<(usize, u32), usize>,
        (address, len): (usize, u32),
    ) {
        let mut exact = log
            .spans
            .iter()
            .filter(|span| span.addr == address && span.len == len);
        let source = if let Some(first) = exact.next() {
            let observation = observations.entry((address, len)).or_default();
            let mut selected = first;
            for span in exact.take(*observation) {
                selected = span;
            }
            *observation += 1;
            selected.ok.then_some((selected, 0))
        } else {
            log.spans
                .iter()
                .enumerate()
                .filter(|(_, span)| {
                    log.has_bulk
                        && span.ok
                        && span.bulk
                        && span.addr <= address
                        && address + len as usize <= span.addr + span.len as usize
                })
                .max_by_key(|(index, span)| (span.addr, span.addr + span.len as usize, *index))
                .map(|(_, span)| (span, address - span.addr))
        };
        let mut bytes = vec![0xee; len as usize];
        assert_eq!(recorded.copy(address, &mut bytes), source.is_some());
        if let Some((span, offset)) = source {
            let start = span.start as usize + offset;
            assert_eq!(bytes, log.bytes[start..start + len as usize]);
        } else {
            assert!(bytes.iter().all(|byte| *byte == 0xee));
        }
    }

    fn assert_index_fixture(log: &RawLog<1>) {
        let recorded = Recorded::build(log);
        let mut observations = std::collections::BTreeMap::<(usize, u32), usize>::new();
        for _ in 0..3 {
            for span in &log.spans {
                for (address, len) in [
                    (span.addr, span.len),
                    (span.addr + 1, span.len.saturating_sub(1).max(1)),
                    (span.addr, span.len.saturating_add(1)),
                ] {
                    assert_recorded_query(log, &recorded, &mut observations, (address, len));
                }
            }
        }
    }

    #[test]
    fn exact_length_mask_absent_width_keeps_bulk_winners_and_unique_hint() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x5000, 4, Some(3), false),
                (0x1000, 128, Some(7), true),
                (0x1020, 32, Some(11), true),
                (0x1020, 32, Some(19), true),
                (0x5004, 8, Some(23), false),
            ],
        );
        let recorded = Recorded::build(&log);
        assert!(!recorded.index.may_have_exact_length(12));
        let mut observations = std::collections::BTreeMap::new();
        assert_recorded_query(&log, &recorded, &mut observations, (0x5000, 4));
        assert_eq!(recorded.next_exact.get(), 1);
        for address in [0x1003, 0x1024, 0x1040, 0x1025, 0x1078, 0x1004] {
            assert_recorded_query(&log, &recorded, &mut observations, (address, 12));
            assert_eq!(recorded.next_exact.get(), 1);
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x1020, 32).unwrap().get(),
                2
            );
        }
        // A bulk-only query did not consume the next unique exact key.
        assert_recorded_query(&log, &recorded, &mut observations, (0x1000, 128));
        assert_eq!(recorded.next_exact.get(), 2);
        assert_recorded_query(&log, &recorded, &mut observations, (0x5004, 8));
        assert_eq!(recorded.next_exact.get(), 5);
    }

    #[test]
    fn exact_length_mask_includes_failures_and_preserves_exact_over_bulk() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 128, Some(7), true),
                (0x1008, 12, None, false),
                (0x1008, 12, Some(19), false),
                (0x1008, 12, None, false),
                (0x2000, 13, None, true),
            ],
        );
        let recorded = Recorded::build(&log);
        // Failed exact and failed whole-bulk keys both populate the mask.
        for len in [12, 13, 128] {
            assert!(recorded.index.may_have_exact_length(len));
        }
        let mut observations = std::collections::BTreeMap::new();
        for query in [
            (0x1008, 12),
            (0x1009, 12),
            (0x1008, 12),
            (0x1008, 12),
            (0x1008, 12),
            (0x2000, 13),
            (0x1004, 13),
        ] {
            assert_recorded_query(&log, &recorded, &mut observations, query);
        }
        assert_eq!(
            recorded.index.cursor(&log.spans, 0x1008, 12).unwrap().get(),
            3
        );
    }

    #[test]
    fn exact_length_mask_large_width_bucket_is_conservative() {
        let widths = [
            0,
            1,
            12,
            63,
            64,
            127,
            128,
            191,
            192,
            254,
            255,
            256,
            u32::MAX,
        ];
        for marked in widths {
            let mut index = RecordedIndex::default();
            index.mark_exact_length(marked);
            for queried in widths {
                assert_eq!(
                    index.may_have_exact_length(queried),
                    marked.min(255) == queried.min(255),
                    "marked {marked}, queried {queried}"
                );
            }
        }
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 1024, Some(7), true),
                (0x1100, 255, Some(11), false),
                (0x1200, 256, None, false),
                (0x4000, u32::MAX, None, false),
            ],
        );
        let recorded = Recorded::build(&log);
        assert!(recorded.index.may_have_exact_length(300));
        assert!(recorded.index.may_have_exact_length(u32::MAX));
        let mut observations = std::collections::BTreeMap::new();
        // Unrepresented large widths conservatively use the exact lookup,
        // then still resolve the same bulk range after that lookup misses.
        for query in [
            (0x1001, 300),
            (0x1100, 255),
            (0x1200, 256),
            (0x1201, 256),
            (0x1002, 254),
        ] {
            assert_recorded_query(&log, &recorded, &mut observations, query);
        }
    }

    #[test]
    fn exact_length_mask_reuse_rebuild_empty_and_bulk_gate_follow_current_shape() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[(0x1000, 64, Some(7), true), (0x2000, 12, Some(11), false)],
        );
        let mut index = RecordedIndex::default();
        assert!(!index.prepare(&log.spans, log.has_bulk));
        let mask = index.exact_lengths;
        assert!(index.may_have_exact_length(12));
        assert!(index.prepare(&log.spans, log.has_bulk));
        assert_index_fixture(&log);

        // Identical ordered shape retains the mask but serves current bytes
        // and offsets, including absent-width requests within the bulk span.
        log.bytes.clear();
        log.bytes.resize(13, 0xa5);
        for (position, span) in log.spans.iter_mut().enumerate() {
            span.start = log.bytes.len() as u32;
            log.bytes
                .resize(log.bytes.len() + span.len as usize, 31 + position as u8);
        }
        assert!(index.prepare(&log.spans, log.has_bulk));
        assert_eq!(index.exact_lengths, mask);
        assert_index_fixture(&log);

        // Removing a width clears its bit on rebuild; changes to bulk or
        // failure flags alone must also invalidate the full-shape reuse.
        set_index_fixture(&mut log, &[(0x1000, 64, Some(41), true)]);
        assert!(!index.prepare(&log.spans, log.has_bulk));
        assert!(!index.may_have_exact_length(12));
        assert_index_fixture(&log);
        log.spans[0].ok = false;
        assert!(!index.prepare(&log.spans, log.has_bulk));
        assert!(index.may_have_exact_length(64));
        assert_index_fixture(&log);
        log.spans[0].ok = true;
        log.spans[0].bulk = false;
        assert!(!index.prepare(&log.spans, log.has_bulk));
        assert_index_fixture(&log);
        log.has_bulk = false;
        assert!(!index.prepare(&log.spans, log.has_bulk));
        assert!(index.may_have_exact_length(64));
        assert_index_fixture(&log);

        log.clear();
        assert!(!index.prepare(&log.spans, log.has_bulk));
        assert_eq!(index.exact_lengths, [0; 4]);
        assert!(index.prepare(&log.spans, log.has_bulk));
        assert_index_fixture(&log);
        assert_eq!(log.index.borrow().as_ref().unwrap().exact_lengths, [0; 4]);
    }

    #[test]
    fn exact_length_mask_bulk_subranges_leave_repeated_cursors_independent() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 64, Some(7), true),
                (0x1000, 64, None, true),
                (0x1000, 64, Some(23), true),
                (0x3000, 4, Some(31), false),
            ],
        );
        let first = Recorded::build(&log);
        let second = Recorded::build(&log);
        for recorded in [&first, &second] {
            assert!(!recorded.index.may_have_exact_length(12));
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
                0
            );
            let mut observations = std::collections::BTreeMap::new();
            for expected_cursor in [1, 2, 2, 2] {
                assert_recorded_query(&log, recorded, &mut observations, (0x1000, 64));
                let next_exact = recorded.next_exact.get();
                for address in [0x1001, 0x1010, 0x1038, 0x1002] {
                    assert_recorded_query(&log, recorded, &mut observations, (address, 12));
                    assert_eq!(recorded.next_exact.get(), next_exact);
                    assert_eq!(
                        recorded.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
                        expected_cursor
                    );
                }
            }
        }
        drop(first);
        drop(second);
        // Reusing the retained index still restarts the duplicate stream.
        let rebuilt = Recorded::build(&log);
        assert_eq!(
            rebuilt.index.cursor(&log.spans, 0x1000, 64).unwrap().get(),
            0
        );
        let mut observations = std::collections::BTreeMap::new();
        assert_recorded_query(&log, &rebuilt, &mut observations, (0x1000, 64));
    }

    #[test]
    fn compact_map_preserves_reset_roots_and_terminal_duplicate_faults() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 4, Some(7), false),
                (0x2000, 4, Some(11), false),
                (0x3000, 8, None, false),
                (0x2000, 4, None, false),
                (0x4000, 4, Some(19), false),
                (0x3000, 8, Some(23), false),
                (0x2000, 4, None, false),
                (0x3000, 8, Some(31), false),
            ],
        );
        // Both fresh construction and exact-shape reuse reset to the first
        // observation. Unique keys have no reset metadata or cursor writes.
        for _ in 0..3 {
            let recorded = Recorded::build(&log);
            assert_eq!(
                recorded.index.repeated,
                [
                    (recorded.index.slot(&log.spans, 0x2000, 4).unwrap(), 1),
                    (recorded.index.slot(&log.spans, 0x3000, 8).unwrap(), 2),
                ]
            );
            assert_eq!(
                recorded.index.next,
                [usize::MAX, 3, 5, 6, usize::MAX, 7, 6, 7]
            );
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x2000, 4).unwrap().get(),
                1
            );
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x3000, 8).unwrap().get(),
                2
            );
            let mut observations = std::collections::BTreeMap::new();
            for query in [
                (0x1000, 4),
                (0x2000, 4),
                (0x3000, 8),
                (0x4000, 4),
                (0x2000, 4),
                (0x3000, 8),
                (0x2000, 4),
                (0x3000, 8),
                (0x2000, 4),
                (0x3000, 8),
                (0x1000, 4),
                (0x4000, 4),
            ] {
                assert_recorded_query(&log, &recorded, &mut observations, query);
            }
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x2000, 4).unwrap().get(),
                6
            );
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x3000, 8).unwrap().get(),
                7
            );
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x1000, 4).unwrap().get(),
                0
            );
            assert_eq!(
                recorded.index.cursor(&log.spans, 0x4000, 4).unwrap().get(),
                4
            );
        }
    }

    #[test]
    fn unique_exact_hint_matches_linear_reads_in_sequential_and_arbitrary_order() {
        let reads: Vec<_> = (0..64)
            .map(|index| {
                (
                    0x1000 + index * 32,
                    4 + index as u32 % 3,
                    (index % 7 != 0).then_some(index as u8),
                    false,
                )
            })
            .collect();
        let mut log = RawLog::<1>::default();
        set_index_fixture(&mut log, &reads);
        let recorded = Recorded::build(&log);
        assert!(recorded.index.next.iter().all(|&next| next == usize::MAX));
        let mut observations = std::collections::BTreeMap::new();
        for (index, &(address, len, _, _)) in reads.iter().enumerate() {
            assert_recorded_query(&log, &recorded, &mut observations, (address, len));
            assert_eq!(recorded.next_exact.get(), index + 1);
        }
        for index in (0..reads.len())
            .rev()
            .chain((0..256).map(|index| index * 37 % 64))
        {
            let (address, len, _, _) = reads[index];
            assert_recorded_query(&log, &recorded, &mut observations, (address, len));
            assert_eq!(recorded.next_exact.get(), index + 1);
            // Same address with another width must not use the exact hint.
            assert_recorded_query(&log, &recorded, &mut observations, (address, len + 9));
        }
    }

    #[test]
    fn unique_exact_hint_never_skips_an_earlier_duplicate_observation() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 4, Some(7), false),
                (0x2000, 4, Some(11), false),
                (0x1000, 4, None, false),
                (0x3000, 4, Some(19), false),
                (0x1000, 4, Some(23), false),
                (0x4000, 8, Some(31), false),
            ],
        );
        let recorded = Recorded::build(&log);
        assert_eq!(
            recorded.index.next,
            [2, usize::MAX, 4, usize::MAX, 4, usize::MAX]
        );
        let mut observations = std::collections::BTreeMap::new();
        // Resolving the unique entry before a later duplicate primes a
        // misleading hint. The first request must still observe 7, not a fault.
        for query in [
            (0x2000, 4),
            (0x1000, 4),
            (0x3000, 4),
            (0x1000, 4),
            (0x3000, 4),
            (0x1000, 4),
            (0x2000, 4),
            (0x1000, 4),
            (0x4000, 8),
            (0x1000, 4),
        ] {
            assert_recorded_query(&log, &recorded, &mut observations, query);
        }
    }

    #[test]
    fn unique_exact_hint_preserves_fault_priority_bulk_ties_and_independent_readers() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (0x1000, 64, Some(7), true),
                (0x1010, 16, Some(11), true),
                (0x1010, 16, Some(19), true),
                (0x1014, 4, None, false),
                (0x2000, 8, Some(23), false),
                (0x1014, 4, Some(31), false),
                (0x1008, 4, None, false),
            ],
        );
        let first = Recorded::build(&log);
        let second = Recorded::build(&log);
        let mut first_observations = std::collections::BTreeMap::new();
        let mut second_observations = std::collections::BTreeMap::new();
        let queries = [
            (0x1000, 64),
            (0x1011, 4),
            (0x1010, 16),
            (0x2000, 8),
            (0x1014, 4),
            (0x1014, 4),
            (0x1011, 24),
            (0x1010, 16),
            (0x1010, 16),
            (0x1008, 4),
            (0x1008, 4),
            (0x1040, 1),
        ];
        for (index, &query) in queries.iter().enumerate() {
            assert_recorded_query(&log, &first, &mut first_observations, query);
            assert_recorded_query(
                &log,
                &second,
                &mut second_observations,
                queries[queries.len() - index - 1],
            );
        }
        drop(first);
        let third = Recorded::build(&log);
        assert_eq!(third.next_exact.get(), 0);
        let mut third_observations = std::collections::BTreeMap::new();
        for query in queries {
            assert_recorded_query(&log, &third, &mut third_observations, query);
        }
        assert_recorded_query(&log, &second, &mut second_observations, (0x1014, 4));
    }

    #[test]
    fn unique_exact_hint_uses_current_bytes_and_offsets_after_shape_reuse() {
        let mut log = RawLog::<1>::default();
        let reads = [
            (0x1000, 4, Some(7), false),
            (0x2000, 8, Some(11), false),
            (0x3000, 4, None, false),
            (0x4000, 4, Some(19), false),
        ];
        set_index_fixture(&mut log, &reads);
        assert_index_fixture(&log);
        let original_slots = log.index.borrow().as_ref().unwrap().slots.capacity();
        // Keep exactly the same key/ok/bulk shape, but replace and relocate
        // every successful byte span. No old offset or byte may be cached.
        log.bytes.clear();
        log.bytes.resize(13, 0xa5);
        for (index, span) in log.spans.iter_mut().enumerate() {
            if span.ok {
                span.start = log.bytes.len() as u32;
                log.bytes
                    .resize(log.bytes.len() + span.len as usize, 41 + index as u8);
            }
        }
        assert_index_fixture(&log);
        assert_eq!(
            log.index.borrow().as_ref().unwrap().slots.capacity(),
            original_slots
        );
        // A formerly unique key becoming repeated must clear the sentinel
        // at every occurrence; deleting that duplicate restores uniqueness.
        for tail in [Some((0x1000, 4, Some(59), false)), None] {
            let mut changed = reads.to_vec();
            changed.extend(tail);
            set_index_fixture(&mut log, &changed);
            assert_index_fixture(&log);
            let cached = log.index.borrow();
            let next = &cached.as_ref().unwrap().next;
            assert_eq!(next[0] == usize::MAX, tail.is_none());
            if tail.is_some() {
                assert_eq!(next[4], 4);
            }
        }
    }

    #[test]
    fn index_refresh_handles_insertions_deletions_reorders_and_duplicate_faults() {
        let a = (0x1000, 4, Some(7), false);
        let b = (0x2000, 8, Some(11), false);
        let c = (0x3000, 4, Some(19), false);
        let d = (0x4000, 4, Some(31), false);
        let failed_a = (0x1000, 4, None, false);
        let later_a = (0x1000, 4, Some(47), false);
        let mut log = RawLog::<1>::default();
        for reads in [
            vec![a, b, later_a, c],
            vec![a, b, failed_a, later_a, c],
            vec![d, a, b, failed_a, later_a, c],
            vec![d, a, failed_a, later_a, c],
            vec![d, a, failed_a, later_a, c, b],
            vec![c, failed_a, d, later_a, a, b],
            vec![c, (0x1000, 8, Some(59), false), d, a, b],
            vec![],
            vec![failed_a, later_a, a],
        ] {
            set_index_fixture(&mut log, &reads);
            assert_index_fixture(&log);
            let cached = log.index.borrow();
            let index = cached.as_ref().unwrap();
            let keys: std::collections::BTreeSet<_> =
                reads.iter().map(|read| (read.0, read.1)).collect();
            assert_eq!(index.distinct, keys.len());
            assert_eq!(index.next.len(), reads.len());
            assert_eq!(index.shape.len(), reads.len());
            assert!(
                keys.iter()
                    .all(|key| index.cursor(&log.spans, key.0, key.1).is_some())
            );
        }
    }

    #[test]
    fn index_refresh_handles_bulk_flags_faults_and_current_pool_offsets() {
        let mut log = RawLog::<1>::default();
        for (parent, child, exact, bulk_gate) in [
            (Some(7), Some(11), Some(19), true),
            (Some(23), Some(31), None, true),
            (Some(37), None, Some(41), true),
            (None, Some(43), Some(47), true),
            (Some(53), Some(59), Some(61), false),
            (Some(67), Some(71), Some(73), true),
        ] {
            set_index_fixture(
                &mut log,
                &[
                    (0x1000, 32, parent, bulk_gate),
                    (0x1008, 8, child, true),
                    (0x1009, 4, exact, false),
                ],
            );
            log.has_bulk = bulk_gate;
            assert_index_fixture(&log);
        }
    }

    #[test]
    fn index_refresh_drops_inactive_keys_and_retains_only_peak_capacity() {
        let mut log = RawLog::<1>::default();
        let mut index = RecordedIndex::default();
        let mut peak_count = 0;
        let mut peak_capacity = 0;
        for refresh in 0..64 {
            let count = if refresh == 12 { 17 } else { 4 };
            let reads: Vec<_> = (0..count)
                .map(|row| (0x1000 + refresh * 0x1000 + row * 16, 4, Some(7), false))
                .collect();
            set_index_fixture(&mut log, &reads);
            index.prepare(&log.spans, false);
            assert_eq!(index.distinct, count);
            if count > peak_count {
                peak_count = count;
                peak_capacity = index.slots.capacity();
            }
            assert_eq!(index.slots.capacity(), peak_capacity);
            assert_eq!(index.next.len(), count);
            assert_eq!(index.shape.len(), count);
            assert!(index.repeated.is_empty());
            assert!(
                reads
                    .iter()
                    .all(|read| index.cursor(&log.spans, read.0, read.1).is_some())
            );
        }
    }

    #[test]
    fn compact_index_hash_matches_the_previous_tuple_hasher() {
        use std::hash::Hash;
        for address in [
            0,
            1,
            0x1000,
            0x1_4000_0000,
            0xffff_8000_0000_1000,
            usize::MAX,
        ] {
            for len in [0, 1, 4, 8, 16, 32, 64, u32::MAX] {
                let mut previous = ReadAddressHasher::default();
                (address, len).hash(&mut previous);
                assert_eq!(
                    RecordedIndex::hash(address, len),
                    previous.finish() as usize
                );
            }
        }
    }

    #[test]
    fn compact_index_collisions_wrap_and_keep_each_observation_stream() {
        let addresses: Vec<_> = (1..=4096)
            .map(|index| 0x1000 + index * 16)
            .filter(|&address| RecordedIndex::hash(address, 4) & 7 == 7)
            .take(5)
            .collect();
        assert_eq!(addresses.len(), 5);
        let mut log = RawLog::<1>::default();
        set_index_fixture(
            &mut log,
            &[
                (addresses[0], 4, Some(7), false),
                (addresses[1], 4, Some(11), false),
                (addresses[2], 4, None, false),
                (addresses[3], 4, Some(19), false),
                (addresses[0], 4, None, false),
                (addresses[2], 4, Some(23), false),
                (addresses[0], 4, Some(31), false),
            ],
        );
        for _ in 0..3 {
            let recorded = Recorded::build(&log);
            assert_eq!(recorded.index.slots.len(), 8);
            assert_eq!(recorded.index.distinct, 4);
            for (index, &address) in addresses[..4].iter().enumerate() {
                assert_eq!(
                    recorded.index.slot(&log.spans, address, 4),
                    Some((7 + index) & 7)
                );
            }
            assert_eq!(recorded.index.slot(&log.spans, addresses[4], 4), Some(3));
            assert!(recorded.index.cursor(&log.spans, addresses[4], 4).is_none());
            let mut observations = std::collections::BTreeMap::new();
            for _ in 0..4 {
                for &address in addresses.iter().rev() {
                    assert_recorded_query(&log, &recorded, &mut observations, (address, 4));
                    assert_recorded_query(&log, &recorded, &mut observations, (address, 8));
                }
            }
        }
    }

    #[test]
    fn compact_index_resolves_early_duplicate_roots_after_all_rehashes() {
        let first = (1..=4096)
            .map(|index| 0x1000 + index * 16)
            .find(|&address| (32..64).contains(&(RecordedIndex::hash(address, 4) & 255)))
            .unwrap();
        let second = first + 16;
        let mut reads = vec![
            (first, 4, Some(7), false),
            (second, 4, Some(11), false),
            (first, 4, None, false),
            (second, 4, Some(19), false),
        ];
        reads.extend((0..70).map(|index| (first + 0x1000 + index * 16, 4, Some(37), false)));
        reads.extend([(first, 4, Some(23), false), (second, 4, None, false)]);
        let mut log = RawLog::<1>::default();
        set_index_fixture(&mut log, &reads);
        for _ in 0..3 {
            assert_index_fixture(&log);
            let recorded = Recorded::build(&log);
            assert_eq!(recorded.index.slots.len(), 256);
            assert_eq!(recorded.index.distinct, 72);
            let first_slot = recorded.index.slot(&log.spans, first, 4).unwrap();
            let second_slot = recorded.index.slot(&log.spans, second, 4).unwrap();
            assert_ne!(first_slot, RecordedIndex::hash(first, 4) & 7);
            assert_eq!(recorded.index.repeated, [(first_slot, 0), (second_slot, 1)]);
            assert_eq!(recorded.index.slots[first_slot].get(), 0);
            assert_eq!(recorded.index.slots[second_slot].get(), 1);
        }
    }

    #[test]
    fn compact_index_capacity_depends_on_distinct_keys_not_observations() {
        let reads: Vec<_> = (0..2048)
            .map(|index| {
                (
                    0x1000,
                    if index % 2 == 0 { 4 } else { 8 },
                    (index % 7 != 0).then_some((index % 251) as u8),
                    false,
                )
            })
            .collect();
        let mut log = RawLog::<1>::default();
        set_index_fixture(&mut log, &reads);
        for _ in 0..3 {
            let recorded = Recorded::build(&log);
            assert_eq!(recorded.index.distinct, 2);
            assert_eq!(recorded.index.slots.len(), 8);
            assert_eq!(recorded.index.slots.capacity(), 8);
            assert_eq!(recorded.index.repeated.len(), 2);
            for &(address, len, value, _) in &reads {
                let mut bytes = [0xee; 8];
                assert_eq!(
                    recorded.copy(address, &mut bytes[..len as usize]),
                    value.is_some()
                );
                let expected = [value.unwrap_or(0xee); 8];
                assert_eq!(bytes[..len as usize], expected[..len as usize]);
            }
            for &(address, len, value, _) in &reads[reads.len() - 2..] {
                let mut bytes = [0xee; 8];
                assert_eq!(
                    recorded.copy(address, &mut bytes[..len as usize]),
                    value.is_some()
                );
                let expected = [value.unwrap_or(0xee); 8];
                assert_eq!(bytes[..len as usize], expected[..len as usize]);
            }
        }
    }

    #[test]
    fn compact_index_shrink_clears_exhausted_cursors_before_using_new_spans() {
        let mut reads: Vec<_> = (0..70)
            .map(|index| (0x1000 + index * 16, 4, Some(7), false))
            .collect();
        reads.extend((0..70).map(|index| (0x1000 + index * 16, 4, None, false)));
        let mut log = RawLog::<1>::default();
        set_index_fixture(&mut log, &reads);
        assert_index_fixture(&log);
        let capacity = log.index.borrow().as_ref().unwrap().slots.capacity();
        {
            let cached = log.index.borrow();
            let index = cached.as_ref().unwrap();
            assert_eq!(index.cursor(&log.spans, 0x1000, 4).unwrap().get(), 70);
        }
        set_index_fixture(
            &mut log,
            &[(0x1000, 4, Some(201), false), (0x1000, 4, None, false)],
        );
        {
            let recorded = Recorded::build(&log);
            assert_eq!(recorded.index.slots.capacity(), capacity);
            assert_eq!(recorded.index.distinct, 1);
            let mut observations = std::collections::BTreeMap::new();
            for query in [
                (0x1000, 4),
                (0x1000, 4),
                (0x1000, 4),
                (0x1010, 4),
                (0x1450, 4),
            ] {
                assert_recorded_query(&log, &recorded, &mut observations, query);
            }
        }
        set_index_fixture(&mut log, &[]);
        {
            let recorded = Recorded::build(&log);
            assert_eq!(recorded.index.slots.capacity(), capacity);
            assert_eq!(recorded.index.distinct, 0);
            assert!(
                recorded
                    .index
                    .slots
                    .iter()
                    .all(|slot| slot.get() == usize::MAX)
            );
            assert!(recorded.index.repeated.is_empty());
            assert!(!recorded.copy(0x1000, &mut [0; 4]));
        }
        set_index_fixture(&mut log, &[(0x1000, 8, Some(211), true)]);
        assert_index_fixture(&log);
        let recorded = Recorded::build(&log);
        assert_eq!(recorded.index.slots.capacity(), capacity);
        let mut bytes = [0; 4];
        assert!(recorded.copy(0x1002, &mut bytes));
        assert_eq!(bytes, [211; 4]);
    }

    #[test]
    fn edited_indexes_keep_simultaneous_recorded_views_independent() {
        let mut log = RawLog::<1>::default();
        set_index_fixture(&mut log, &[(0x1000, 4, Some(7), false)]);
        assert_index_fixture(&log);
        set_index_fixture(
            &mut log,
            &[
                (0x2000, 4, Some(11), false),
                (0x1000, 4, Some(19), false),
                (0x1000, 4, None, false),
                (0x1000, 4, Some(23), false),
            ],
        );
        let first = Recorded::build(&log);
        let second = Recorded::build(&log);
        for view in [&first, &second] {
            let mut bytes = [0; 4];
            assert!(view.copy(0x1000, &mut bytes));
            assert_eq!(bytes, [19; 4]);
            assert!(!view.copy(0x1000, &mut bytes));
        }
        drop(first);
        let third = Recorded::build(&log);
        let mut bytes = [0; 4];
        assert!(third.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [19; 4]);
        assert!(second.copy(0x1000, &mut bytes));
        assert_eq!(bytes, [23; 4]);
    }

    struct FailAll;

    impl Memory for FailAll {
        fn copy(&self, _address: usize, _destination: &mut [u8]) -> bool {
            false
        }
    }

    #[test]
    fn take_records_bytes_and_failures_with_table_masks() {
        let bytes = [9u8; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<3>::new(8, 64);
        let start = log.take(&memory, 0x1000, 4, 0x01).expect("copy");
        assert_eq!(&log.bytes[start..start + 4], &[9, 9, 9, 9]);
        assert_eq!(log.raw_bytes, [4, 0, 0]);
        // Empty spans read nothing and record nothing.
        assert_eq!(log.take(&memory, 0xdead, 0, 0x02), Some(4));
        assert_eq!(log.spans.len(), 1);
        // Null addresses fail without touching memory.
        assert!(log.take(&memory, 0, 4, 0x02).is_none());
        assert!(log.fault[1]);
        assert!(!log.fault[0]);
        // Live failures record the mask and block only attributed tables.
        assert!(log.take(&FailAll, 0x1010, 4, 0x06).is_none());
        assert_eq!(log.fault, [false, true, true]);
    }

    #[test]
    fn initialized_arena_keeps_exact_scalar_bulk_prefixes_after_partial_faults() {
        struct Traced {
            partial: usize,
            calls: RefCell<Vec<(usize, usize)>>,
            initial: RefCell<Vec<Vec<u8>>>,
        }
        impl Memory for Traced {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                // Reading the initialized destination is valid, but its old
                // contents do not determine the source bytes or copy result.
                self.initial.borrow_mut().push(destination.to_vec());
                self.calls.borrow_mut().push((address, destination.len()));
                let completed = if address == 0x2000 {
                    self.partial.min(destination.len())
                } else {
                    destination.len()
                };
                for (index, byte) in destination[..completed].iter_mut().enumerate() {
                    *byte = (address.wrapping_add(index * 29) as u8) ^ 0xa5;
                }
                address != 0x2000
            }
        }

        for len in [0_usize, 1, 2, 3, 4, 7, 8, 15, 16, 24, 26, 31, 32, 33, 64] {
            for partial in [0, len / 2, len.saturating_sub(1)] {
                let scalar_memory = Traced {
                    partial,
                    calls: RefCell::default(),
                    initial: RefCell::default(),
                };
                let generic_memory = Traced {
                    partial,
                    calls: RefCell::default(),
                    initial: RefCell::default(),
                };
                let mut scalar = RawLog::<2>::new(8, 256);
                let mut generic = RawLog::<2>::new(8, 256);
                let prefix = [0xb3, 0, 0xff, 0x71];
                for log in [&mut scalar, &mut generic] {
                    // Every physical byte is initialized, but stale payloads
                    // remain outside the four-byte currently published prefix.
                    log.bytes.resize(log.bytes.capacity(), 0xcd);
                    log.bytes.truncate(prefix.len());
                    log.bytes.copy_from_slice(&prefix);
                }
                let mut expected = prefix.to_vec();
                for (address, mask) in [
                    (0x1000, 1),
                    (0x2000, 2),
                    (0x1100, 1),
                    (0, 2),
                    (0x0072_6f74_6361_665f, 2),
                    (usize::MAX, 2),
                ] {
                    let scalar_result = scalar.take(&scalar_memory, address, len, mask);
                    // Scalar and bulk retain distinct recorded span semantics.
                    // Normalize only the marker for a complete log comparison.
                    let generic_result = generic.take_bulk(&generic_memory, address, len, mask);
                    if generic_result.is_some() && len != 0 {
                        generic.spans.last_mut().unwrap().bulk = false;
                    }
                    assert_eq!(scalar_result, generic_result);
                    assert!(scalar.logs_equal(&generic));
                    assert_eq!(scalar.fault, generic.fault);
                    assert_eq!(scalar.raw_bytes, generic.raw_bytes);
                    assert_eq!(scalar.overflowed, generic.overflowed);
                    assert_eq!(scalar.span_exhausted, generic.span_exhausted);
                    assert_eq!(scalar.required_bytes, generic.required_bytes);
                    assert_eq!(scalar_memory.calls, generic_memory.calls);
                    assert_eq!(&scalar.bytes[..prefix.len()], &prefix);
                    if len != 0 && matches!(address, 0x1000 | 0x1100) {
                        expected.extend(
                            (0..len).map(|index| (address.wrapping_add(index * 29) as u8) ^ 0xa5),
                        );
                    }
                    assert_eq!(scalar.bytes, expected);
                    assert!(
                        scalar
                            .bytes
                            .get(expected.len()..expected.len() + 1)
                            .is_none()
                    );
                }
                let expected_calls = if len == 0 {
                    Vec::new()
                } else {
                    vec![(0x1000, len), (0x2000, len), (0x1100, len)]
                };
                assert_eq!(*scalar_memory.calls.borrow(), expected_calls);
                assert_eq!(scalar.raw_bytes, [(len * 2) as u64, 0]);
                if len != 0 {
                    assert_eq!(scalar_memory.initial.borrow()[0], vec![0xcd; len]);
                    assert_eq!(scalar_memory.initial, generic_memory.initial);
                    let reader = Recorded::build(&scalar);
                    let mut rejected = vec![0xe7; len];
                    assert!(!reader.copy(0x2000, &mut rejected));
                    assert_eq!(rejected, vec![0xe7; len]);
                }
            }
        }
    }

    #[test]
    fn scalar_fixed_width_append_preserves_short_pool_preflight() {
        struct NoRead;
        impl Memory for NoRead {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("pool preflight must reject before the source read")
            }
        }
        for len in [1, 2, 3, 4, 8, 16, 32, 64] {
            for span_full in [false, true] {
                let mut scalar = RawLog::<1>::new(usize::from(!span_full), len + 4);
                let mut generic = RawLog::<1>::new(usize::from(!span_full), len + 4);
                for log in [&mut scalar, &mut generic] {
                    let used = if span_full {
                        4
                    } else {
                        log.bytes.capacity() - len + 1
                    };
                    log.bytes.resize(used, 0xa7);
                }
                let before = scalar.bytes.to_vec();
                let capacities = (scalar.spans.capacity(), scalar.bytes.capacity());
                assert_eq!(scalar.take(&NoRead, 0x1000, len, 1), None);
                assert_eq!(generic.take_bulk(&NoRead, 0x1000, len, 1), None);
                assert!(scalar.logs_equal(&generic));
                assert_eq!(scalar.bytes, before);
                assert_eq!(scalar.spans.capacity(), capacities.0);
                assert_eq!(scalar.bytes.capacity(), capacities.1);
                assert!(scalar.overflowed);
                assert_eq!(scalar.span_exhausted, span_full);
                assert_eq!(scalar.span_exhausted, generic.span_exhausted);
                assert_eq!(scalar.required_bytes, before.len() + len);
                assert_eq!(scalar.required_bytes, generic.required_bytes);
                assert_eq!(scalar.fault, [false]);
                assert_eq!(scalar.raw_bytes, [0]);
                // An overflowed log still short-circuits later requests.
                assert_eq!(scalar.take(&NoRead, 0x1100, 1, 1), None);
                assert_eq!(scalar.bytes, before);
            }
        }
    }

    #[test]
    fn recorded_serves_exact_spans_and_fails_everything_else() {
        let bytes = [7u8; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<2>::new(8, 64);
        log.take(&memory, 0x1000, 8, 0x01);
        log.take(&memory, 0x1020, 4, 0x02);
        let recorded = Recorded::build(&log);
        let mut out = [0u8; 8];
        assert!(recorded.copy(0x1000, &mut out));
        assert_eq!(out, [7; 8]);
        // Overlapping but unrecorded ranges fail: service is exact-match.
        assert!(!recorded.copy(0x1000, &mut out[..4]));
        assert!(!recorded.copy(0x1004, &mut out));
        assert!(!recorded.copy(0x2000, &mut out));
        // Empty reads succeed and null addresses fail without a lookup.
        assert!(recorded.copy(0x2000, &mut []));
        assert!(!recorded.copy(0, &mut out));
    }

    #[test]
    fn overflow_aborts_and_grows_off_thread() {
        let bytes = [1u8; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(1, 4);
        assert!(log.take(&memory, 0x1000, 4, 0x01).is_some());
        assert!(log.take(&memory, 0x1010, 4, 0x01).is_none());
        assert!(log.overflowed);
        assert!(log.grow_for_next());
        // Further takes short-circuit once overflowed.
        assert!(log.take(&memory, 0x1000, 1, 0x01).is_none());
        log.grow();
        log.clear();
        assert!(!log.overflowed);
        assert!(log.take(&memory, 0x1000, 4, 0x01).is_some());
    }

    #[test]
    fn logs_compare_by_spans_and_bytes() {
        let bytes = [3u8; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut first = RawLog::<1>::new(8, 64);
        let mut second = RawLog::<1>::new(8, 64);
        first.take(&memory, 0x1000, 4, 0x01);
        second.take(&memory, 0x1000, 4, 0x01);
        assert!(first.logs_equal(&second));
        second.take(&memory, 0x1010, 4, 0x01);
        assert!(!first.logs_equal(&second));
    }

    #[test]
    fn growth_increases_empty_and_partially_filled_capacities() {
        let mut empty = Vec::<u8>::with_capacity(16);
        double_capacity(&mut empty);
        assert!(empty.capacity() >= 32);
        assert!(empty.is_empty());
        let mut partial = Vec::with_capacity(16);
        partial.extend_from_slice(&[7u8; 3]);
        double_capacity(&mut partial);
        assert!(partial.capacity() >= 32);
        assert_eq!(partial, [7; 3]);
        let mut zero = Vec::<u8>::new();
        double_capacity(&mut zero);
        assert!(zero.capacity() >= 2);
    }

    #[test]
    fn span_pressure_does_not_grow_the_byte_arena() {
        let bytes = [1; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(1, 64);
        log.take(&memory, 0x1000, 4, 1);
        log.take(&memory, 0x1004, 4, 1);
        assert!(log.grow_for_next());
        log.grow();
        assert!(log.spans.capacity() >= 2);
        assert_eq!(log.bytes.capacity(), 64);
    }

    #[test]
    fn byte_pressure_does_not_grow_the_span_arena() {
        let bytes = [1; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(8, 4);
        log.take_bulk(&memory, 0x1000, 32, 1);
        assert!(log.grow_for_next());
        log.grow();
        assert_eq!(log.spans.capacity(), 8);
        assert!(log.bytes.capacity() >= 32);
    }

    #[test]
    fn repeated_takes_serve_in_record_order_then_repeat_the_last() {
        let first_bytes = [3u8; 64];
        let first = SliceMemory {
            base: 0x1000,
            bytes: &first_bytes,
        };
        let mut second_bytes = [3u8; 64];
        second_bytes[0] = 9;
        let second = SliceMemory {
            base: 0x1000,
            bytes: &second_bytes,
        };
        let mut log = RawLog::<1>::new(8, 64);
        log.take(&first, 0x1000, 4, 0x01);
        log.take(&second, 0x1000, 4, 0x01);
        let recorded = Recorded::build(&log);
        // Repeated exact-key requests observe their own values in order.
        let mut out = [0u8; 4];
        assert!(recorded.copy(0x1000, &mut out));
        assert_eq!(out, [3, 3, 3, 3]);
        assert!(recorded.copy(0x1000, &mut out));
        assert_eq!(out, [9, 3, 3, 3]);
        // Shared spans taken once keep serving the last value; spans never
        // taken still fail loudly (see the exact-match test above).
        assert!(recorded.copy(0x1000, &mut out));
        assert_eq!(out, [9, 3, 3, 3]);
        assert!(!recorded.copy(0x2000, &mut out));
    }

    #[test]
    fn bulk_spans_serve_enclosed_unaligned_sub_ranges() {
        // S3: moved walks issue bulk copies (session 0x34/0x40 headers,
        // players 64 B string pairs, map order vectors) wider than the
        // walk's scattered reads (4 B keys, 8 B words, 1 B flags inside).
        // `Recorded` serves any request fully enclosed in one bulk span,
        // including deliberately unaligned sub-ranges; exact-only logs
        // fail the same requests. Bulk wins for sub-ranges: small exact
        // spans never shadow an enclosing bulk chunk.
        let mut bytes = [0u8; 128];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = (index & 0xff) as u8;
        }
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(8, 128);
        log.has_bulk = true;
        // Bulk chunk covering 0x1000..0x1040, plus an exact span elsewhere.
        log.take_bulk(&memory, 0x1000, 64, 0x01);
        log.take(&memory, 0x1060, 8, 0x01);
        let recorded = Recorded::build(&log);
        // Aligned sub-range inside the bulk chunk.
        let mut out = [0u8; 8];
        assert!(recorded.copy(0x1008, &mut out));
        assert_eq!(out, bytes[8..16]);
        // Deliberately unaligned overlapping sub-range (addr+3, len 7).
        let mut unaligned = [0u8; 7];
        assert!(recorded.copy(0x1003, &mut unaligned));
        assert_eq!(unaligned, bytes[3..10]);
        // Single byte at the very end of the bulk chunk.
        let mut tail = [0u8; 1];
        assert!(recorded.copy(0x103f, &mut tail));
        assert_eq!(tail, [bytes[63]]);
        // Straddling the bulk edge fails (not fully enclosed).
        let mut straddle = [0u8; 8];
        assert!(!recorded.copy(0x103c, &mut straddle));
        // Exact span still serves its own range.
        let mut exact = [0u8; 8];
        assert!(recorded.copy(0x1060, &mut exact));
        // Without bulk, the same sub-range requests fail (exact-only).
        let mut plain = RawLog::<1>::new(8, 128);
        plain.take(&memory, 0x1000, 64, 0x01);
        let plain_recorded = Recorded::build(&plain);
        assert!(!plain_recorded.copy(0x1008, &mut out));
    }

    #[test]
    fn take_inner_rejects_noncanonical_addresses_as_faults_pre_copy() {
        // Live crash runs resolved to ASCII-derived noncanonical sources:
        // run 1 `rdx=0x00726f746361665f` ("_factor\0") and run 2
        // `rdx=0x6f746361665f7374` ("ts_facto"). `take_inner` must record the
        // same fault the typed walk produces for an unreadable source, before
        // any guarded copy runs (a crash cannot happen in tests; the traced
        // memory proves no copy was attempted).
        use std::cell::Cell;
        struct Traced<'a> {
            inner: SliceMemory<'a>,
            calls: Cell<usize>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.calls.set(self.calls.get() + 1);
                self.inner.copy(address, destination)
            }
        }
        let bytes = [9u8; 64];
        let memory = Traced {
            inner: SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            calls: Cell::new(0),
        };
        let mut log = RawLog::<2>::new(8, 64);
        for noncanonical in [0x0072_6f74_6361_665fusize, 0x6f74_6361_665f_7374usize] {
            assert!(
                log.take(&memory, noncanonical, 8, 0x01).is_none(),
                "noncanonical {noncanonical:x} must fail pre-copy"
            );
            assert!(log.fault[0]);
            assert!(!log.overflowed);
        }
        assert_eq!(
            memory.calls.get(),
            0,
            "noncanonical takes must not touch memory"
        );
        // Canonical low-half and high-half kernel addresses still go through.
        assert!(log.take(&memory, 0x1000, 8, 0x02).is_some());
        assert_eq!(memory.calls.get(), 1);
        // `Recorded` mirrors the live gate so decodes fail identically.
        let recorded = Recorded::build(&log);
        let mut out = [0u8; 8];
        assert!(!recorded.copy(0x0072_6f74_6361_665f, &mut out));
        assert!(!recorded.copy(0x6f74_6361_665f_7374, &mut out));
        assert!(recorded.copy(0x1000, &mut out));
    }

    #[test]
    fn nested_bulk_arrays_do_not_hide_later_parent_object_fields() {
        let bytes: Vec<u8> = (0..128).collect();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(8, 256);
        log.has_bulk = true;
        log.take_bulk(&memory, 0x1000, 128, 1);
        log.take_bulk(&memory, 0x1010, 16, 1);
        log.take_bulk(&memory, 0x1040, 8, 1);
        let recorded = Recorded::build(&log);
        let mut out = [0; 24];
        assert!(recorded.copy(0x1028, &mut out));
        assert_eq!(out, bytes[40..64]);
        assert!(recorded.copy(0x1044, &mut out));
        assert_eq!(out, bytes[68..92]);
        assert!(!recorded.copy(0x1070, &mut out));
    }

    #[test]
    fn bulk_cache_keeps_nested_winners_parent_fallback_and_exact_faults() {
        let mut log = RawLog::<1>::new(8, 256);
        log.has_bulk = true;
        for (base, bytes) in [
            (0x1000, &[1; 128][..]),
            (0x1010, &[2; 16][..]),
            (0x1040, &[3; 8][..]),
        ] {
            log.take_bulk(&SliceMemory { base, bytes }, base, bytes.len(), 1)
                .unwrap();
        }
        log.take(&FailAll, 0x1018, 4, 1);
        let recorded = Recorded::build(&log);
        for (address, len, expected) in [
            (0x1001, 4, Some(1)),  // primes the parent
            (0x1005, 4, Some(1)),  // parent cache hit
            (0x1011, 4, Some(2)),  // child must supersede it
            (0x1012, 24, Some(1)), // extends past child: parent fallback
            (0x1013, 4, Some(2)),  // smaller next read still picks child
            (0x1018, 4, None),     // exact failure beats a primed bulk cache
            (0x1041, 4, Some(3)),
            (0x1044, 12, Some(1)),
            (0x107f, 2, None),
        ] {
            let mut bytes = vec![0xa5; len];
            assert_eq!(recorded.copy(address, &mut bytes), expected.is_some());
            assert_eq!(bytes, vec![expected.unwrap_or(0xa5); len]);
        }
        // Changing direction and requested width must never turn the cached
        // winner into an implicit override of the full ordered interval search.
        for address in (0x0ffc..0x1084).rev() {
            for len in 1..12 {
                let expected = if address == 0x1018 && len == 4 {
                    None
                } else {
                    recorded
                        .index
                        .bulk
                        .iter()
                        .rev()
                        .find(|span| span.0 <= address && address + len <= span.1)
                        .map(|span| log.bytes[log.spans[span.2].start as usize])
                };
                let mut bytes = vec![0xa5; len];
                assert_eq!(
                    recorded.copy(address, &mut bytes),
                    expected.is_some(),
                    "{address:x}/{len}"
                );
                assert_eq!(
                    bytes,
                    vec![expected.unwrap_or(0xa5); len],
                    "{address:x}/{len}"
                );
            }
        }
    }

    #[test]
    fn canonical_gate_includes_bit_47_in_the_sign_extension() {
        assert!(!is_canonical_range(0x0000_8000_0000_0000, 8));
        assert!(!is_canonical_range(0xFFFF_0000_0000_0000, 8));
        assert!(is_canonical_range(0x0000_7FFF_FFFF_FFF8, 8));
        assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
    }

    #[test]
    fn repeated_failed_reads_keep_their_place_and_do_not_fall_back_to_bulk() {
        let bytes = [7; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(8, 128);
        log.has_bulk = true;
        log.take_bulk(&memory, 0x1000, 64, 1);
        log.take(&memory, 0x1010, 8, 1);
        log.take(&FailAll, 0x1010, 8, 1);
        log.take(&memory, 0x1010, 8, 1);
        log.take(&FailAll, 0x1020, 8, 1);
        let recorded = Recorded::build(&log);
        let mut out = [0; 8];
        assert!(recorded.copy(0x1010, &mut out));
        assert!(!recorded.copy(0x1010, &mut out));
        assert!(recorded.copy(0x1010, &mut out));
        assert!(recorded.copy(0x1010, &mut out));
        assert!(!recorded.copy(0x1020, &mut out));
        assert!(!recorded.copy(0x1020, &mut out));
    }

    #[test]
    fn canonical_gate_matches_engine_vectors_and_keeps_valid_data() {
        // The collector gate duplicates `engine::memory`'s check without a new
        // public API; both must agree on the live crash vectors and on valid
        // game-like addresses. Overflow and empty spans keep `Memory` semantics.
        assert!(!is_canonical_range(0x0072_6f74_6361_665f, 8));
        assert!(!is_canonical_range(0x6f74_6361_665f_7374, 8));
        assert!(is_canonical_range(0x1_4000_0000, 8));
        assert!(is_canonical_range(0xFFFF_8000_0000_0000, 8));
        assert!(!is_canonical_range(0x0000_7FFF_FFFF_FFF0, 32));
        assert!(is_canonical_range(0x2000, 0));
        assert!(!is_canonical_range(usize::MAX - 3, 8));
    }
}
