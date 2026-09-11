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

    pub fn logs_equal(&self, other: &Self) -> bool {
        self.spans == other.spans && self.bytes == other.bytes
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
}
