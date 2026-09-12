//! Shared span-log raw-copy primitive for the decode-worker rollout.
//!
//! Each moved family stages one refresh of guarded reads into a `RawLog`:
//! exact (address, length) spans with success flags, affected-table masks
//! and byte pools. The game thread never decodes, sorts or compares rows; it
//! only copies bytes and parses locally what the next read address is. The
//! single FIFO decode worker serves the log through `Recorded` (a `Memory`
//! impl failing every unrecorded read) and replays the family's existing
//! typed walk, so rows, ordinals, skips, truncation counts and failure
//! reasons are identical by construction: every read the walk performs on
//! its success paths is in the log, and every recorded failure reproduces
//! the walk's own reason at the same read site.
//!
//! Mirror rules (log order is irrelevant; the log is a set keyed by exact
//! address and length):
//! - reads served locally by an authenticated view need no log entry beyond
//!   the view's own spans;
//! - pooled row and arena capacities are ignored while copying (a superset
//!   is recorded and the replay truncates identically against the real
//!   pooled buffers);
//! - a failed or unparsable span stops only the sub-path that needs it; the
//!   replay fails identically from the same recorded spans, so later extras
//!   are harmless;
//! - pool overflow aborts the whole copy: the caller falls back to the typed
//!   walk (identical rows through the existing path) and enqueues nothing.
//!
//! Conservative rule (as the research pilot): any live failure attributed to
//! a table blocks that table's check even when both copies agree, so
//! same-hour tearing never advances the windows. A readable-but-invalid
//! refresh (deterministic fault, both copies equal) still counts its check
//! while emitting the identical Unavailable outcome.

use engine::Memory;
use std::collections::HashMap;

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
fn is_canonical_range(addr: usize, len: usize) -> bool {
    if len == 0 {
        return true;
    }
    let Some(end) = addr.checked_add(len.wrapping_sub(1)) else {
        return false;
    };
    let high = |value: usize| value >> 47;
    let canonical = |value: usize| {
        let top = value >> 48;
        top == 0 || top == 0xFFFF
    };
    canonical(addr) && canonical(end) && high(addr) == high(end)
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

/// One refresh of raw payload for a family of at most 32 tables. Flat pools
/// keep every span fixed-size so `clear` recycles capacities and the steady
/// state allocates on neither thread; growth happens on the decode side only.
#[derive(Debug)]
pub struct RawLog<const N: usize> {
    pub spans: Vec<Span>,
    pub bytes: Vec<u8>,
    pub fault: [bool; N],
    pub raw_bytes: [u64; N],
    pub overflowed: bool,
    /// Set when the log holds bulk spans wider than the replay's requests
    /// (contiguous table copies). The decode side then serves sub-ranges
    /// from the enclosing span instead of exact matches only.
    pub has_bulk: bool,
}

impl<const N: usize> RawLog<N> {
    /// Cold constructor; steady state reuses pooled jobs through the
    /// decode-worker acquire/recycle pair, so this never runs per refresh.
    pub fn new(span_cap: usize, byte_cap: usize) -> Self {
        debug_assert!(N <= 32, "span masks cover at most 32 tables");
        Self {
            spans: Vec::with_capacity(span_cap),
            bytes: Vec::with_capacity(byte_cap),
            fault: [false; N],
            raw_bytes: [0; N],
            overflowed: false,
            has_bulk: false,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.spans.clear();
        self.bytes.clear();
        self.fault = [false; N];
        self.raw_bytes = [0; N];
        self.overflowed = false;
        self.has_bulk = false;
    }

    pub fn grow_for_next(&self) -> bool {
        self.overflowed
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.spans.reserve_exact(self.spans.capacity().max(1));
        self.bytes.reserve_exact(self.bytes.capacity().max(1));
    }

    /// Guarded copy of one span into the pool. Returns the pool offset of the
    /// copied bytes on success and records failures with the affected-table
    /// mask. Empty spans read nothing (matching `Memory`), null or
    /// overflowing addresses fail without touching memory. On pool overflow
    /// sets `overflowed` and returns `None`.
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
    pub fn take_bulk(
        &mut self,
        memory: &impl Memory,
        addr: usize,
        len: usize,
        mask: u32,
    ) -> Option<usize> {
        self.take_inner(memory, addr, len, mask, true)
    }

    fn take_inner(
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
                .is_none_or(|end| end > self.bytes.capacity())
        {
            self.overflowed = true;
            return None;
        }
        let start = self.bytes.len();
        self.bytes.resize(start + len, 0);
        if !memory.copy(addr, &mut self.bytes[start..start + len]) {
            self.bytes.truncate(start);
            self.note_fail(addr, len32, mask);
            return None;
        }
        self.spans.push(Span {
            addr,
            len: len32,
            ok: true,
            mask,
            start: start as u32,
            bulk,
        });
        self.raw_bytes[lowest_table(mask)] += len as u64;
        Some(start)
    }

    fn note_fail(&mut self, addr: usize, len32: u32, mask: u32) {
        if self.spans.len() == self.spans.capacity() {
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

    #[allow(dead_code)]
    pub fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
    }

    /// Per-partition equality for independently-walked tables. Compares only
    /// the spans whose mask carries `slot`'s bit (shared census spans carry
    /// several bits and gate every dependent partition; private spans carry
    /// one bit and gate only their own). Byte contents compare per span, not
    /// whole-pool, so tearing one table's private bytes flips only that
    /// table's partition. `has_bulk` is log-level and must agree, otherwise
    /// the bulk-serving mode differs and every partition disagrees.
    /// Callers still gate each slot by its own live-fault bits:
    /// `matched[slot] = partition_equal(slot) && !fault[slot]`.
    pub fn partition_equal(&self, other: &Self, slot: usize) -> bool {
        debug_assert!(slot < N, "partition slot out of range");
        if slot >= N {
            return false;
        }
        // Overflowed logs never compare equal per partition: the caller
        // falls back to the typed walk and enqueues nothing.
        if self.overflowed || other.overflowed {
            return false;
        }
        if self.has_bulk != other.has_bulk {
            return false;
        }
        let bit = 1u32 << slot;
        // Filter in record order; the copy is deterministic so shared and
        // private spans interleave identically when the underlying bytes do.
        let mut left_count = 0usize;
        let mut right_count = 0usize;
        for span in &self.spans {
            if span.mask & bit != 0 {
                left_count += 1;
            }
        }
        for span in &other.spans {
            if span.mask & bit != 0 {
                right_count += 1;
            }
        }
        if left_count != right_count {
            return false;
        }
        let mut left_iter = self.spans.iter().filter(|span| span.mask & bit != 0);
        let mut right_iter = other.spans.iter().filter(|span| span.mask & bit != 0);
        loop {
            match (left_iter.next(), right_iter.next()) {
                (None, None) => return true,
                (Some(left), Some(right)) => {
                    if left.addr != right.addr
                        || left.len != right.len
                        || left.ok != right.ok
                        || left.mask != right.mask
                        || left.bulk != right.bulk
                    {
                        return false;
                    }
                    if left.ok {
                        let left_bytes = self
                            .bytes
                            .get(left.start as usize..left.start as usize + left.len as usize);
                        let right_bytes = other
                            .bytes
                            .get(right.start as usize..right.start as usize + right.len as usize);
                        if left_bytes != right_bytes {
                            return false;
                        }
                    }
                }
                _ => return false,
            }
        }
    }
}

/// Attribute shared spans to the anchor table; private spans carry a single
/// bit already. Only `take` calls this, so byte accounting never allocates.
pub fn lowest_table(mask: u32) -> usize {
    for slot in 0..32 {
        if mask & (1 << slot) != 0 {
            return slot as usize;
        }
    }
    0
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
    map: HashMap<(usize, u32), Vec<usize>>,
    next: std::cell::RefCell<HashMap<(usize, u32), usize>>,
    bulk: Vec<(usize, usize, usize)>,
    bytes: &'a [u8],
}

impl<'a> Recorded<'a> {
    pub fn build<const N: usize>(log: &'a RawLog<N>) -> Self {
        let mut map: HashMap<(usize, u32), Vec<usize>> = HashMap::with_capacity(log.spans.len());
        let mut bulk = Vec::new();
        for span in &log.spans {
            if span.ok {
                map.entry((span.addr, span.len))
                    .or_default()
                    .push(span.start as usize);
                // Only flagged bulk chunks serve sub-ranges: small exact
                // spans inside a chunk must never shadow it.
                if log.has_bulk && span.bulk && span.len > 0 {
                    bulk.push((
                        span.addr,
                        span.addr + span.len as usize,
                        span.start as usize,
                    ));
                }
            }
        }
        if log.has_bulk {
            bulk.sort_unstable();
        }
        Self {
            map,
            next: std::cell::RefCell::new(HashMap::new()),
            bulk,
            bytes: &log.bytes,
        }
    }

    fn serve(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(starts) = self.map.get(&(address, destination.len() as u32)) {
            let mut next = self.next.borrow_mut();
            let cursor = next.entry((address, destination.len() as u32)).or_insert(0);
            // In-order values while the multiplicity lasts, then the last
            // value for shared spans requested more often than taken.
            let start = starts.get(*cursor).or(starts.last()).copied();
            let Some(start) = start else {
                return false;
            };
            *cursor += 1;
            let Some(source) = self.bytes.get(start..start + destination.len()) else {
                return false;
            };
            destination.copy_from_slice(source);
            return true;
        }
        if self.bulk.is_empty() {
            return false;
        }
        let end = address.checked_add(destination.len());
        let end = match end {
            Some(end) => end,
            None => return false,
        };
        // Enclosing bulk span by greatest start at or below the request.
        let index = self.bulk.partition_point(|span| span.0 <= address);
        let Some(&(start, span_end, pool)) = self.bulk[..index].last() else {
            return false;
        };
        if end > span_end {
            return false;
        }
        let offset = pool + (address - start);
        let Some(source) = self.bytes.get(offset..offset + destination.len()) else {
            return false;
        };
        destination.copy_from_slice(source);
        true
    }
}

impl Memory for Recorded<'_> {
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
        // Witness-before/after pairs observe their own values in order.
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
    fn partition_equal_isolates_private_bytes_and_shares_census() {
        let bytes = [5u8; 64];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut first = RawLog::<2>::new(8, 64);
        let mut second = RawLog::<2>::new(8, 64);
        // Shared census span (both bits) plus one private span per slot.
        first.take(&memory, 0x1000, 4, 0x03);
        first.take(&memory, 0x1010, 4, 0x01);
        first.take(&memory, 0x1020, 4, 0x02);
        second.take(&memory, 0x1000, 4, 0x03);
        second.take(&memory, 0x1010, 4, 0x01);
        second.take(&memory, 0x1020, 4, 0x02);
        assert!(first.partition_equal(&second, 0));
        assert!(first.partition_equal(&second, 1));
        // Tear slot 1's private bytes: only slot 1 flips.
        let private_start = second.spans[2].start as usize;
        second.bytes[private_start] ^= 0xff;
        assert!(first.partition_equal(&second, 0));
        assert!(!first.partition_equal(&second, 1));
        assert!(!first.logs_equal(&second));
        // Tear the shared census: both partitions flip.
        let census_start = second.spans[0].start as usize;
        second.bytes[census_start] ^= 0xff;
        // Slot 0 still compares its private span equal, but the shared
        // census differs, so slot 0 now disagrees as well.
        assert!(!first.partition_equal(&second, 0));
        assert!(!first.partition_equal(&second, 1));
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
