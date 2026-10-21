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
//! - a record gather logs the field it stopped at as a failed span even when
//!   its row pointer is null (address 0) or non-canonical; the replay's
//!   `copy_records` stops at the same field before any lookup;
//! - allocating worker roles grow before each original request; fixed pool
//!   overflow aborts the copy and grows staging capacity on recycle.
//!   The caller may use the typed fallback only before a prepared source
//!   observation; afterward, that capture fails without repeating live reads.

pub use crate::address_range::is_canonical_range;
use crate::raw_plan::{Attempt, CapturePlan, Observations, Request};
use engine_native::Memory;
use std::cell::RefCell;
use std::sync::Mutex;

mod recorded;

use recorded::RecordedIndex;
pub use recorded::index_diagnostics;
pub use recorded::{Recorded, RecordedCache};
#[cfg(test)]
use {recorded::ReadAddressHasher, std::hash::Hasher};

pub fn read_plan_diagnostics() -> impl serde::Serialize {
    crate::raw_plan::diagnostics()
}

#[derive(serde::Serialize)]
#[serde(untagged)]
pub enum Growth {
    Raw(RawGrowth),
    Fog {
        row_capacity: usize,
        required_rows: usize,
        province_capacity: usize,
        required_provinces: usize,
    },
}
#[derive(serde::Serialize)]
#[serde(untagged)]
pub enum Faults {
    Raw(Vec<bool>),
    Packet([bool; 1]),
}
#[derive(serde::Serialize)]
pub struct RawGrowth {
    pub spans: usize,
    pub span_capacity: usize,
    pub bytes: usize,
    pub byte_capacity: usize,
    pub faults: Faults,
    pub growing: bool,
    pub first_failed_address: Option<usize>,
    pub first_failed_length: Option<usize>,
    pub first_failed_mask: Option<u32>,
    pub last_address: Option<usize>,
    pub last_length: Option<usize>,
    pub last_mask: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_capacity_scope: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub packet_inline_bytes_excluded_from_vector_accounting: Option<usize>,
}

static GROWTH_DIAGNOSTICS: Mutex<Vec<(&'static str, Growth)>> = Mutex::new(Vec::new());

/// Worker-only capacity diagnostics: never allocate or lock in the callback.
pub fn note_growth<const N: usize>(family: &'static str, log: &RawLog<N>) {
    if !log.overflowed {
        return;
    }
    let failed = log.spans.iter().find(|span| !span.ok);
    let mut faults = Vec::new();
    if faults.try_reserve_exact(N).is_err() {
        return;
    }
    faults.extend_from_slice(&log.fault);
    publish_growth(
        family,
        Growth::Raw(RawGrowth {
            spans: log.spans.len(),
            span_capacity: log.spans.capacity(),
            bytes: log.bytes.len(),
            byte_capacity: log.bytes.capacity(),
            faults: Faults::Raw(faults),
            growing: log.grow_for_next(),
            first_failed_address: failed.map(|span| span.addr),
            first_failed_length: failed.map(|span| span.len as usize),
            first_failed_mask: failed.map(|span| span.mask),
            last_address: log.spans.last().map(|span| span.addr),
            last_length: log.spans.last().map(|span| span.len as usize),
            last_mask: log.spans.last().map(|span| span.mask),
            span_capacity_scope: None,
            packet_inline_bytes_excluded_from_vector_accounting: None,
        }),
    );
}

/// Publish an equivalent worker-owned packet capacity diagnostic.
pub fn publish_growth(family: &'static str, value: Growth) {
    if let Ok(mut diagnostics) = GROWTH_DIAGNOSTICS.lock() {
        match diagnostics.binary_search_by_key(&family, |row| row.0) {
            Ok(index) => diagnostics[index].1 = value,
            Err(index) => {
                if diagnostics.try_reserve(1).is_err() {
                    return;
                }
                diagnostics.insert(index, (family, value));
            }
        }
    }
}

pub fn growth_diagnostics() -> impl serde::Serialize {
    GrowthDiagnostics(GROWTH_DIAGNOSTICS.lock().ok())
}
struct GrowthDiagnostics(Option<std::sync::MutexGuard<'static, Vec<(&'static str, Growth)>>>);
impl serde::Serialize for GrowthDiagnostics {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let Some(rows) = &self.0 else {
            return serializer.serialize_none();
        };
        let mut map = serializer.serialize_map(Some(rows.len()))?;
        for (family, value) in rows.iter() {
            map.serialize_entry(family, value)?;
        }
        map.end()
    }
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
        let mut storage = Vec::new();
        if engine_allocation::reserve_exact(&mut storage, capacity).is_ok() {
            storage.resize(storage.capacity(), 0);
        }
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
        if capacity > self.storage.capacity()
            && crate::allocation::reserve_raw(&mut self.storage, capacity, "raw bytes").is_ok()
        {
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
/// keep every span fixed-size so `clear` recycles capacities. Allocating worker
/// roles may grow before a read; fixed callback logs grow on decode recycling.
#[derive(Debug)]
pub struct RawLog<const N: usize> {
    pub spans: Vec<Span>,
    pub bytes: RawBytes,
    pub fault: [bool; N],
    pub raw_bytes: [u64; N],
    pub overflowed: bool,
    span_exhausted: bool,
    required_spans: usize,
    required_bytes: usize,
    worker_growth: bool,
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
    pub backend_ns: u64,
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
        let mut spans = Vec::new();
        let allocated = engine_allocation::reserve_exact(&mut spans, span_cap).is_ok();
        let mut log = Self {
            spans,
            bytes: RawBytes::with_capacity(if allocated { byte_cap } else { 0 }),
            fault: [false; N],
            raw_bytes: [0; N],
            overflowed: false,
            span_exhausted: false,
            required_spans: 0,
            required_bytes: 0,
            worker_growth: false,
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
        self.required_spans = 0;
        self.required_bytes = 0;
        self.worker_growth = false;
        self.has_bulk = false;
        self.plan.active = false;
        self.plan.sealed = false;
        self.plan.blocked = false;
        self.plan.source_observed = false;
        self.plan.attempt = Attempt::None;
    }

    /// Opt in off-thread. Trace exhaustion changes only optimization eligibility.
    pub fn initialize_read_plan(
        &mut self,
        read_capacity: usize,
        control_capacity: usize,
        control_byte_capacity: usize,
    ) {
        let mut requests = Vec::new();
        if engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut requests, read_capacity)
        })
        .is_err()
        {
            return;
        }
        self.plan.requests = requests;
        self.bytes.observations = Some(RefCell::new(Observations::new(
            control_capacity,
            control_byte_capacity,
        )));
        self.plan.enabled = !crate::allocation::failed();
        self.update_capacity_charge();
    }

    /// The producer opts in only after its scattered native observations and
    /// auxiliary state have the same complete-plan/ordered-resume contract.
    pub fn initialize_scattered_read_plan(&mut self, reads: usize, controls: usize, bytes: usize) {
        self.initialize_read_plan(reads, controls, bytes);
        self.plan.scattered = true;
    }

    /// Begin on an empty log with the same fresh provider used by the producer.
    /// False requires running that producer before finish_planned_capture().
    pub fn begin_planned_capture(&mut self, memory: &impl Memory, key: [usize; 4]) -> bool {
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
        use engine_native::read_plan::Outcome;
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

    pub fn finish_planned_capture(&mut self) {
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

    pub fn planned_capture_failed(&self) -> bool {
        self.plan.blocked
    }

    pub fn planning(&self) -> bool {
        self.plan.active
    }

    /// Retain the producer's current, validated lookahead in a prepared pass.
    /// Observing the selecting raw bytes creates controls before this hint.
    pub fn prefetch(&self, address: usize, len: usize) {
        if let Some(observations) = &self.bytes.observations {
            observations.borrow_mut().hint(address, len);
        }
        engine_native::prefetch::span(address, len);
    }

    /// Capacity exhaustion after a prepared source observation must not enter
    /// an outer fresh typed retry. Keep its fault bits clear so normal worker
    /// growth can repair capacity before the next independent capture.
    pub fn planned_capture_requires_no_retry(&self) -> bool {
        self.planned_capture_failed() || (self.overflowed && self.plan.source_observed)
    }

    pub fn planned_capture_succeeded(&self) -> bool {
        self.plan.attempt == Attempt::Complete && !self.plan.blocked
    }

    /// Compound acquisitions cannot retry an earlier section's planned source
    /// observations when a later section exhausts its staging storage.
    pub fn planned_source_observed(&self) -> bool {
        self.plan.source_observed
    }

    /// Request retirement without dropping owned storage on the acquisition thread.
    pub fn invalidate_read_plan(&mut self) {
        self.plan.retire = true;
        self.plan.sealed = false;
    }

    /// Worker-only compilation and retirement, before clear/recycle.
    pub fn prepare_read_plan(&mut self) {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
        if crate::allocation::failed() {
            return;
        }
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

    /// Off-thread live presizing. Retain the current prefix and verdicts;
    /// acquisition uses only the storage prepared here.
    pub fn presize(&mut self, spans: usize, bytes: usize) {
        engine_allocation::advisory(|| self.presize_storage(spans, bytes));
    }

    fn presize_storage(&mut self, spans: usize, bytes: usize) {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
        if spans > self.spans.capacity() {
            let additional = spans.saturating_sub(self.spans.len());
            let _ = engine_allocation::reserve_exact(&mut self.spans, additional);
        }
        if !crate::allocation::failed() {
            self.bytes.grow_to(bytes);
        }
        self.update_capacity_charge();
    }

    /// Grow only exhausted storage off-thread. Span-heavy censuses must not
    /// repeatedly double a mostly empty byte arena (and vice versa).
    pub fn grow(&mut self) {
        engine_allocation::advisory(|| self.grow_storage());
    }

    fn grow_storage(&mut self) {
        let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
        if crate::allocation::failed() {
            return;
        }
        let required_spans = self.required_spans.max(if self.span_exhausted {
            self.spans.len().saturating_add(1)
        } else {
            0
        });
        if required_spans > self.spans.capacity() {
            let _ = crate::allocation::reserve_raw(&mut self.spans, required_spans, "raw spans");
            if crate::allocation::failed() {
                return;
            }
        }
        if self.required_bytes > self.bytes.capacity() {
            self.bytes.grow_to(self.required_bytes);
        }
        self.update_capacity_charge();
    }

    /// Preflight `extra` more bytes before any read that must not be
    /// retried: when they do not fit, mark overflow and the size to grow to.
    pub fn reserve(&mut self, extra: usize) {
        let end = self.bytes.len().saturating_add(extra);
        let _ = self.ensure_storage(0, end);
    }

    /// Allocating acquisition roles opt in after clear, before their first
    /// source read. Growth preserves the acquired prefix and never retries it.
    pub fn enable_worker_growth(&mut self) {
        self.worker_growth = true;
    }

    pub fn worker_growth_enabled(&self) -> bool {
        self.worker_growth
    }

    /// Whether `spans` more spans and `bytes` more pool bytes fit the
    /// storage this log already holds, without growing it or marking
    /// overflow. A batched take admits its worst case up front; when that
    /// does not fit, its caller takes the batch's requests one by one, so
    /// the log overflows only where those requests would.
    pub fn fits(&self, spans: usize, bytes: usize) -> bool {
        !self.overflowed
            && self
                .spans
                .len()
                .checked_add(spans)
                .is_some_and(|needed| needed <= self.spans.capacity())
            && self
                .bytes
                .len()
                .checked_add(bytes)
                .is_some_and(|end| end <= self.bytes.capacity() && end <= u32::MAX as usize)
    }

    fn ensure_storage(&mut self, spans: usize, end: usize) -> bool {
        let needed_spans = self.spans.len().checked_add(spans);
        let span_short = needed_spans.is_none_or(|needed| needed > self.spans.capacity());
        let byte_short = end > self.bytes.capacity();
        if self.worker_growth && end <= u32::MAX as usize && (span_short || byte_short) {
            let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
            if span_short && let Some(needed) = needed_spans {
                let target = needed;
                let _ = crate::allocation::reserve_raw(&mut self.spans, target, "raw spans");
            }
            if byte_short && !crate::allocation::failed() {
                self.bytes.grow_to(end);
            }
            self.update_capacity_charge();
        }
        if needed_spans.is_none_or(|needed| needed > self.spans.capacity())
            || end > self.bytes.capacity()
            || end > u32::MAX as usize
        {
            self.span_exhausted = span_short;
            self.required_spans = self.required_spans.max(needed_spans.unwrap_or(usize::MAX));
            self.required_bytes = self.required_bytes.max(end);
            self.overflowed = true;
            false
        } else {
            true
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
        // Noncanonical x86-64 sources (e.g. text bytes read as a pointer)
        // fault with ExceptionInformation[1] == -1, which the C
        // filter cannot attribute and would crash. Record the same graceful
        // fault the typed walk reports for an unreadable source, before any
        // guarded copy runs. Ordering matches the typed walk: null, overflow,
        // then canonical, each with identical Unavailable/Truncated reasons
        // derived by the replay from these recorded spans.
        if !is_canonical_range(addr, len) {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if !self.ensure_storage(1, self.bytes.len().saturating_add(len)) {
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

    /// Append observations held from an earlier refresh of sources the
    /// caller proved unchanged since, as that refresh recorded them: each of
    /// `spans` is a successful read whose bytes start at its `start` in
    /// `bytes`. Nothing is read, and no raw byte is counted. False, with
    /// overflow and the size to grow to marked, when storage cannot hold
    /// them.
    pub fn splice(&mut self, spans: &[Span], bytes: &[u8]) -> bool {
        if self.overflowed || !self.allow_unplanned_operation() {
            return false;
        }
        let base = self.bytes.len();
        let Some(end) = base.checked_add(bytes.len()) else {
            self.overflowed = true;
            return false;
        };
        if !self.ensure_storage(spans.len(), end) {
            return false;
        }
        self.bytes.extend_from_slice(bytes);
        for span in spans {
            debug_assert!(span.ok && span.start as usize + span.len as usize <= bytes.len());
            self.spans.push(Span {
                start: (base + span.start as usize) as u32,
                ..*span
            });
            self.has_bulk |= span.bulk;
        }
        true
    }

    /// Record one observation that a batched gather already made, exactly as
    /// `take` would have recorded reading it now: `None` is that read's
    /// failure. The span, its fault mask and capacity demand follow `take`'s
    /// order; only reads the replay consumes need recording. The gather
    /// charges its own raw bytes. Unplanned acquisition only.
    pub fn record(
        &mut self,
        addr: usize,
        len: usize,
        observed: Option<&[u8]>,
        mask: u32,
    ) -> Option<usize> {
        debug_assert!(observed.is_none_or(|bytes| bytes.len() == len));
        if !self.allow_unplanned_operation() || self.overflowed {
            return None;
        }
        if len == 0 {
            return Some(self.bytes.len());
        }
        let Ok(len32) = u32::try_from(len) else {
            self.overflowed = true;
            return None;
        };
        if addr == 0 || addr.checked_add(len).is_none() || !is_canonical_range(addr, len) {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if !self.ensure_storage(1, self.bytes.len().saturating_add(len)) {
            return None;
        }
        let Some(bytes) = observed else {
            self.note_fail(addr, len32, mask);
            return None;
        };
        let start = self.bytes.len();
        self.bytes.storage[start..start + len].copy_from_slice(bytes);
        self.bytes.used = start + len;
        self.spans.push(Span {
            addr,
            len: len32,
            ok: true,
            mask,
            start: start as u32,
            bulk: false,
        });
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
        if !self.ensure_storage(spans, end) {
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
        if !self.ensure_storage(spans, end) {
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
        let fields = [engine_native::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: width,
        }];
        let layout = engine_native::memory::RecordLayout::new(&fields, width)?;
        let completed = memory.copy_records(
            engine_native::memory::RecordSources::Strided {
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

    /// Stage fixed-layout fields of several records with one guarded call.
    /// Record `row` lands at `start + row * stride`; each completed nonempty
    /// field is its own exact span and the first failed field is recorded.
    /// Nothing after that failure is read. Returns the completed field count.
    #[cfg(any(test, feature = "test-support"))]
    pub fn take_records(
        &mut self,
        memory: &impl Memory,
        sources: engine_native::memory::RecordSources<'_>,
        layout: &engine_native::memory::RecordLayout<'_>,
        mask: u32,
    ) -> Option<(usize, usize)> {
        if !self.allow_unplanned_operation() || self.overflowed {
            return None;
        }
        let rows = sources.len();
        let fields = layout.fields();
        let start = self.bytes.len();
        let end = rows
            .checked_mul(layout.stride())
            .and_then(|size| start.checked_add(size))?;
        let spans = rows.saturating_mul(fields.len()).saturating_add(1);
        if !self.ensure_storage(spans, end) {
            return None;
        }
        let completed = memory.copy_records(sources, layout, &mut self.bytes.storage[start..end]);
        let width = fields.len();
        debug_assert!(completed <= rows * width);
        if width == 0 {
            return Some((start, completed));
        }
        let (full, partial) = (completed / width, completed % width);
        self.bytes.used = start + (full + usize::from(partial != 0)) * layout.stride();
        let table = lowest_table(mask);
        for row in 0..rows.min(full + 1) {
            let pointer = sources.pointer(row).unwrap_or(0);
            let base = start + row * layout.stride();
            let done = if row < full { width } else { partial };
            for field in &fields[..done] {
                if field.len != 0 {
                    self.spans.push(Span {
                        addr: pointer.wrapping_add(field.source_offset),
                        len: field.len as u32,
                        ok: true,
                        mask,
                        start: (base + field.destination_offset) as u32,
                        bulk: false,
                    });
                    self.raw_bytes[table] += field.len as u64;
                }
            }
            if row == full && full < rows {
                let field = &fields[partial];
                let address = sources
                    .pointer(row)
                    .filter(|&pointer| pointer != 0)
                    .and_then(|pointer| pointer.checked_add(field.source_offset))
                    .unwrap_or(0);
                self.note_fail(address, field.len as u32, mask);
            }
        }
        Some((start, completed))
    }

    fn note_fail(&mut self, addr: usize, len32: u32, mask: u32) {
        if !self.ensure_storage(1, self.bytes.len()) {
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
    #[cfg(any(test, feature = "test-support"))]
    pub fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
    }
}

#[cfg(test)]
mod read_plan_tests;

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

/// Child-process fault sweep: discovery, compilation, cursor preparation and
/// growth all use ordinary production APIs against stack-owned memory.
#[cfg(any(test, feature = "test-support"))]
pub fn allocation_exercise() -> bool {
    use engine_native::memory::SliceMemory;
    let bytes = [7_u8; 64];
    let memory = SliceMemory {
        base: 0x1000,
        bytes: &bytes,
    };
    let mut log = RawLog::<1>::new(4, 32);
    if crate::allocation::failed() {
        return false;
    }
    log.initialize_read_plan(4, 8, 32);
    if crate::allocation::failed() {
        return false;
    }
    if log.begin_planned_capture(&memory, [1, 2, 3, 4]) {
        return false;
    }
    for address in [0x1000, 0x1000, 0x1008] {
        let Some(start) = log.take_bulk(&memory, address, 8, 1) else {
            return false;
        };
        if log.bytes.get(start..start + 4) != Some(&[7; 4][..]) {
            return false;
        }
    }
    log.finish_planned_capture();
    log.prepare_read_plan();
    if crate::allocation::failed() {
        return false;
    }
    {
        let recorded = Recorded::build(&log);
        if crate::allocation::failed() {
            return false;
        }
        if !recorded.copy(0x1000, &mut [0; 8]) || !recorded.copy(0x1002, &mut [0; 3]) {
            return false;
        }
    }
    log.clear();
    // Demand grows both arenas beyond their prepared capacities.
    while !log.span_exhausted {
        let _ = log.take(&memory, 0x1000, 1, 1);
    }
    log.reserve(64);
    note_growth("synthetic-allocation", &log);
    if crate::allocation::failed() {
        return false;
    }
    // The second diagnostic replaces the same family without growing keys.
    note_growth("synthetic-allocation", &log);
    if crate::allocation::failed() {
        return false;
    }
    log.grow();
    if crate::allocation::failed() {
        return false;
    }
    engine_allocation::advisory(crate::raw_plan::allocation_exercise)
}

#[cfg(test)]
mod tests;
