
pub use engine_native_memory::memory::is_canonical_range;
use crate::raw_plan::{Attempt, CapturePlan, Observations, Request};
use engine_native_memory::Memory;
use std::cell::RefCell;
use std::sync::Mutex;

mod country_slots;
mod recorded;

pub use country_slots::CountrySlots;
use recorded::RecordedIndex;
pub use recorded::{Recorded, RecordedCache};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<&'static str>,
}

static GROWTH_DIAGNOSTICS: Mutex<Vec<(&'static str, Growth)>> = Mutex::new(Vec::new());

pub fn note_growth<const N: usize>(family: &'static str, log: &RawLog<N>) {
    if !log.overflowed && !log.torn {
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
            refusal: log.refusal(),
        }),
    );
}

pub const TORN_READ: &str = "raw read longer than any game span: torn or foreign header";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub addr: usize,
    pub len: u32,
    pub ok: bool,
    pub mask: u32,
    pub start: u32,
    pub bulk: bool,
}

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
                    std::ptr::from_ref::<T>(value).cast::<u8>() as usize - self.storage.as_ptr() as usize;
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

    fn grow_to(&mut self, capacity: usize) {
        if capacity > self.storage.capacity()
            && crate::allocation::reserve_raw(&mut self.storage, capacity, "raw bytes").is_ok()
        {
            self.storage.resize(self.storage.capacity(), 0);
        }
    }

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

#[derive(Debug)]
pub struct RawLog<const N: usize> {
    pub spans: Vec<Span>,
    pub bytes: RawBytes,
    pub fault: [bool; N],
    pub raw_bytes: [u64; N],
    pub overflowed: bool,
    torn: bool,
    span_exhausted: bool,
    required_spans: usize,
    required_bytes: usize,
    pub has_bulk: bool,
    index: RefCell<Option<RecordedIndex>>,
    plan: CapturePlan<N>,
    planned_spans_loaned: bool,
    quiet_streak: u8,
    quiet_peak_spans: usize,
    quiet_peak_bytes: usize,
    pub backend_ns: u64,
    capacity_charge: crate::raw_memory::Charge,
}

impl<const N: usize> Default for RawLog<N> {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl<const N: usize> RawLog<N> {
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
            torn: false,
            span_exhausted: false,
            required_spans: 0,
            required_bytes: 0,
            has_bulk: false,
            index: RefCell::new(None),
            plan: CapturePlan::default(),
            planned_spans_loaned: false,
            quiet_streak: 0,
            quiet_peak_spans: 0,
            quiet_peak_bytes: 0,
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

    pub fn clear(&mut self) {
        self.return_planned_spans();
        self.spans.clear();
        self.backend_ns = 0;
        self.bytes.clear();
        self.fault = [false; N];
        self.raw_bytes = [0; N];
        self.overflowed = false;
        self.torn = false;
        self.span_exhausted = false;
        self.required_spans = 0;
        self.required_bytes = 0;
        self.has_bulk = false;
        self.plan.active = false;
        self.plan.sealed = false;
        self.plan.blocked = false;
        self.plan.source_observed = false;
        self.plan.attempt = Attempt::None;
    }

    pub fn drop_index(&mut self) {
        *self.index.get_mut() = None;
    }

    pub fn trim_if_quiet(&mut self) -> bool {
        self.return_planned_spans();
        if self.overflowed || self.plan.attempt != Attempt::None {
            self.quiet_streak = 0;
            self.quiet_peak_spans = 0;
            self.quiet_peak_bytes = 0;
            return false;
        }
        let used_spans = self.spans.len();
        let used_bytes = self.bytes.len();
        let span_cap = self.spans.capacity();
        let byte_cap = self.bytes.storage.capacity();
        let has_unused_plan = self.plan.compiled.is_some()
            || self.plan.requests.capacity() > 32_768
            || self
                .bytes
                .observations
                .as_ref()
                .is_some_and(|observations| observations.borrow().capacity_bytes() > 256 * 1024);
        let oversized = (span_cap > 32_768 && used_spans.saturating_mul(4) < span_cap)
            || (byte_cap > 512 * 1024 && used_bytes.saturating_mul(4) < byte_cap)
            || has_unused_plan;
        if !oversized {
            self.quiet_streak = 0;
            self.quiet_peak_spans = 0;
            self.quiet_peak_bytes = 0;
            return true;
        }
        self.quiet_streak = self.quiet_streak.saturating_add(1);
        self.quiet_peak_spans = self.quiet_peak_spans.max(used_spans);
        self.quiet_peak_bytes = self.quiet_peak_bytes.max(used_bytes);
        if self.quiet_streak < 4 {
            return true;
        }
        let target_spans = self.quiet_peak_spans.saturating_mul(2).max(16_384);
        let target_bytes = self.quiet_peak_bytes.saturating_mul(2).max(256 * 1024);
        self.quiet_streak = 0;
        self.quiet_peak_spans = 0;
        self.quiet_peak_bytes = 0;
        let mut changed = false;
        if self.plan.compiled.is_some() {
            self.plan.compiled = None;
            self.plan.retire = false;
            self.plan.sealed = false;
            changed = true;
        }
        if self.plan.requests.capacity() > target_spans.saturating_mul(2) {
            self.plan.requests = Vec::new();
            changed = true;
        }
        if let Some(observations) = &mut self.bytes.observations
            && observations.get_mut().capacity_bytes() > 256 * 1024
        {
            observations.get_mut().release_buffers();
            changed = true;
        }
        if self.spans.capacity() > target_spans.saturating_mul(2) {
            let mut trimmed = Vec::new();
            if engine_allocation::advisory(|| {
                engine_allocation::reserve_exact(&mut trimmed, target_spans)
            })
            .is_ok()
            {
                self.spans = trimmed;
                changed = true;
            }
        }
        if self.bytes.storage.capacity() > target_bytes.saturating_mul(2) {
            #[expect(clippy::slow_vector_initialization, reason = "the buffer is reserved through the advisory allocator so a refused allocation is handled, then zero-filled; vec![0; n] would allocate outside it")]
            let mut trimmed = Vec::new();
            if engine_allocation::advisory(|| {
                engine_allocation::reserve_exact(&mut trimmed, target_bytes)
            })
            .is_ok()
            {
                trimmed.resize(trimmed.capacity(), 0);
                self.bytes.storage = trimmed;
                self.bytes.used = 0;
                changed = true;
            }
        }
        if changed {
            self.drop_index();
            self.update_capacity_charge();
        }
        true
    }

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

    pub fn initialize_scattered_read_plan(&mut self, reads: usize, controls: usize, bytes: usize) {
        self.initialize_read_plan(reads, controls, bytes);
        self.plan.scattered = true;
    }

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
            return false;
        };
        use engine_native_memory::read_plan::Outcome;
        match outcome {
            Outcome::Complete => {
                let completed = compiled.requests.len();
                self.bytes.used = compiled.plan.destination_len();
                self.has_bulk = compiled.has_bulk;
                self.raw_bytes = compiled.raw_bytes;
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

    pub fn prefetch(&self, address: usize, len: usize) {
        if let Some(observations) = &self.bytes.observations {
            observations.borrow_mut().hint(address, len);
        }
        engine_native_memory::prefetch::span(address, len);
    }

    pub fn planned_capture_requires_no_retry(&self) -> bool {
        self.planned_capture_failed() || (self.overflowed && self.plan.source_observed)
    }

    pub fn planned_capture_succeeded(&self) -> bool {
        self.plan.attempt == Attempt::Complete && !self.plan.blocked
    }

    pub fn planned_source_observed(&self) -> bool {
        self.plan.source_observed
    }

    pub fn invalidate_read_plan(&mut self) {
        self.plan.retire = true;
        self.plan.sealed = false;
    }

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

    pub fn refusal(&self) -> Option<&'static str> {
        self.torn.then_some(TORN_READ)
    }

    fn refuse_torn(&mut self, mask: u32) {
        self.torn = true;
        self.mark_fault(mask);
    }

    fn mark_fault(&mut self, mask: u32) {
        for (slot, fault) in self.fault.iter_mut().enumerate() {
            if mask & (1 << slot) != 0 {
                *fault = true;
            }
        }
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed
            && self.required_bytes <= u32::MAX as usize
            && !self.fault.iter().any(|fault| *fault)
    }

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

    pub fn reserve(&mut self, extra: usize) {
        let end = self.bytes.len().saturating_add(extra);
        let _ = self.ensure_storage(0, end);
    }

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
        if end <= u32::MAX as usize && (span_short || byte_short) {
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
            self.refuse_torn(mask);
            return None;
        };
        if addr == 0 || addr.checked_add(len).is_none() {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if !is_canonical_range(addr, len) {
            self.note_fail(addr, len32, mask);
            return None;
        }
        if !self.ensure_storage(1, self.bytes.len().saturating_add(len)) {
            return None;
        }
        let start = self.bytes.len();
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
        self.spans.extend(spans.iter().map(|span| {
            debug_assert!(span.ok && span.start as usize + span.len as usize <= bytes.len());
            self.has_bulk |= span.bulk;
            Span {
                start: (base + span.start as usize) as u32,
                ..*span
            }
        }));
        true
    }

    pub fn clone_with_extra(&self, extra_spans: usize, extra_bytes: usize) -> Option<Self> {
        let total_spans = self.spans.len().checked_add(extra_spans)?;
        let total_bytes = self.bytes.len().checked_add(extra_bytes)?;
        let mut cloned = Self::new(total_spans, total_bytes);
        if cloned.spans.capacity() < total_spans || cloned.bytes.capacity() < total_bytes {
            return None;
        }
        cloned.spans.extend_from_slice(&self.spans);
        cloned.bytes.extend_from_slice(self.bytes.as_ref());
        cloned.fault = self.fault;
        cloned.raw_bytes = self.raw_bytes;
        cloned.overflowed = self.overflowed;
        cloned.has_bulk = self.has_bulk;
        Some(cloned)
    }

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
            self.refuse_torn(mask);
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
            if len > u32::MAX as usize {
                self.refuse_torn(masks[index]);
                return None;
            }
            let Some(next) = end.checked_add(len) else {
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
        let spans = if BULK { 2 } else { count + 1 };
        if !self.ensure_storage(spans, end) {
            return None;
        }
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
        let fields = [engine_native_memory::memory::RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: width,
        }];
        let layout = engine_native_memory::memory::RecordLayout::new(&fields, width)?;
        let completed = memory.copy_records(
            engine_native_memory::memory::RecordSources::Strided {
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
            let failed_address = address.checked_add(completed * width).unwrap_or(0);
            self.note_fail(failed_address, width as u32, mask);
        }
        Some((start, completed))
    }

    pub fn take_records(
        &mut self,
        memory: &impl Memory,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
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
        self.mark_fault(mask);
    }

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
    }
}

#[cfg(test)]
mod read_plan_tests;

#[inline]
pub fn lowest_table(mask: u32) -> usize {
    if mask == 0 {
        0
    } else {
        mask.trailing_zeros() as usize
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn allocation_exercise() -> bool {
    use engine_native_memory::memory::SliceMemory;
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
        let Ok(recorded) = Recorded::build(&log) else {
            return false;
        };
        if !recorded.copy(0x1000, &mut [0; 8]) || !recorded.copy(0x1002, &mut [0; 3]) {
            return false;
        }
    }
    log.clear();
    for _ in 0..64 {
        if log.take(&memory, 0x1000, 1, 1).is_none() {
            return false;
        }
    }
    log.reserve(64);
    if crate::allocation::failed() {
        return false;
    }
    let _ = log.take(&memory, 0x1000, u32::MAX as usize + 1, 1);
    note_growth("synthetic-allocation", &log);
    if crate::allocation::failed() {
        return false;
    }
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
