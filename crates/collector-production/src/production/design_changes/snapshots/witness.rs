use super::*;
use std::{
    cell::{Cell, RefCell},
    ops::Range,
};

pub(super) struct Observed<'a, M> {
    source: &'a M,
    reads: RefCell<Vec<Range<usize>>>,
    complete: Cell<bool>,
    allocation_failed: Cell<bool>,
    read_failed: Cell<bool>,
}
impl<'a, M: Memory> Observed<'a, M> {
    pub fn new(source: &'a M, spans: usize) -> Self {
        let mut reads = Vec::new();
        // Witnesses are optional: an allocation failure falls back to full-byte reuse.
        let complete = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut reads, spans.saturating_mul(4))
        })
        .is_ok();
        Self {
            source,
            reads: RefCell::new(reads),
            complete: Cell::new(complete),
            allocation_failed: Cell::new(!complete),
            read_failed: Cell::new(false),
        }
    }
    pub fn diagnostics(&self) -> (bool, bool, bool) {
        (
            self.complete.get(),
            self.allocation_failed.get(),
            self.read_failed.get(),
        )
    }
    pub fn finish(
        self,
        log: &RawLog<1>,
        range: Range<usize>,
        order: &[usize],
        trace: bool,
    ) -> Option<Witness> {
        engine_allocation::advisory(move || self.build(log, range, order, trace))
    }
    fn trace(source: &M, reads: &[Range<usize>]) -> Option<(Vec<Request>, Vec<u8>)> {
        let len = reads
            .iter()
            .try_fold(0_usize, |len, read| len.checked_add(read.len()))?;
        let mut bytes = engine_allocation::vec(len).ok()?;
        bytes.resize(len, 0);
        let mut requests = engine_allocation::vec(reads.len()).ok()?;
        let mut start = 0;
        for read in reads {
            let end = start + read.len();
            if !source.copy(read.start, &mut bytes[start..end]) {
                return None;
            }
            requests.push(Request {
                address: read.start,
                bytes: start..end,
            });
            start = end;
        }
        Some((requests, bytes))
    }
    fn build(
        self,
        log: &RawLog<1>,
        range: Range<usize>,
        order: &[usize],
        trace: bool,
    ) -> Option<Witness> {
        if !self.complete.get() {
            return None;
        }
        let mut reads = self.reads.into_inner();
        // Only overlap-consistent immutable occurrences can request a relocation trace.
        let (requests, bytes) = if trace {
            Self::trace(self.source, &reads)
        } else {
            None
        }
        .unwrap_or_default();
        reads.sort_unstable_by_key(|range| range.start);
        let mut count = 0;
        for index in 0..reads.len() {
            if count != 0 && reads[index].start <= reads[count - 1].end {
                reads[count - 1].end = reads[count - 1].end.max(reads[index].end);
            } else {
                reads.swap(count, index);
                count += 1;
            }
        }
        reads.truncate(count);
        let mut pieces = engine_allocation::vec(range.len().saturating_add(reads.len())).ok()?;
        for (index, span) in log.spans[range.clone()].iter().enumerate() {
            let end = span.addr.checked_add(span.len as usize)?;
            // Merged reads are disjoint and sorted, so their ends are sorted too.
            let first = reads.partition_point(|read| read.end <= span.addr);
            for read in reads[first..].iter().take_while(|read| read.start < end) {
                let start = span.addr.max(read.start) - span.addr;
                let end = end.min(read.end) - span.addr;
                engine_allocation::reserve(&mut pieces, 1).ok()?;
                pieces.push(Piece {
                    span: index,
                    bytes: start..end,
                });
            }
        }
        let mut overlaps = engine_allocation::vec(order.len()).ok()?;
        let mut covering: Option<usize> = None;
        for &index in order {
            let span = &log.spans[index];
            let end = span.addr + span.len as usize;
            if let Some(previous) = covering {
                let previous_span = &log.spans[previous];
                let previous_end = previous_span.addr + previous_span.len as usize;
                if previous_end > span.addr {
                    engine_allocation::reserve(&mut overlaps, 1).ok()?;
                    overlaps.push(Overlap {
                        left: previous - range.start,
                        right: index - range.start,
                        offset: span.addr - previous_span.addr,
                        len: (previous_end - span.addr).min(span.len as usize),
                    });
                }
                if previous_end >= end {
                    continue;
                }
            }
            covering = Some(index);
        }
        Some(Witness {
            pieces,
            overlaps,
            requests,
            bytes,
        })
    }
}
impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.source.copy(address, output);
        if !ok {
            self.read_failed.set(true);
            self.complete.set(false);
        } else if !output.is_empty() && self.complete.get() {
            let mut reads = self.reads.borrow_mut();
            if let Some(end) = address.checked_add(output.len()) {
                if engine_allocation::advisory(|| engine_allocation::reserve(&mut reads, 1)).is_ok()
                {
                    reads.push(address..end);
                } else {
                    self.allocation_failed.set(true);
                    self.complete.set(false);
                }
            } else {
                self.complete.set(false);
            }
        }
        ok
    }
}
#[derive(PartialEq, Eq)]
struct Piece {
    span: usize,
    bytes: Range<usize>,
}
struct Overlap {
    left: usize,
    right: usize,
    offset: usize,
    len: usize,
}
pub(super) struct Request {
    pub address: usize,
    pub bytes: Range<usize>,
}
pub(super) struct Witness {
    pub requests: Vec<Request>,
    pub bytes: Vec<u8>,
    pieces: Vec<Piece>,
    overlaps: Vec<Overlap>,
}
impl Witness {
    pub fn matches(&self, memory: &impl Memory, buffer: &mut Vec<u8>) -> Option<bool> {
        if self.requests.is_empty() {
            return None;
        }
        for request in &self.requests {
            engine_allocation::reserve(buffer, request.bytes.len().saturating_sub(buffer.len()))
                .ok()?;
            buffer.resize(request.bytes.len(), 0);
            if !memory.copy(request.address, buffer)
                || buffer.as_slice() != &self.bytes[request.bytes.clone()]
            {
                return Some(false);
            }
        }
        Some(true)
    }
    pub fn key(
        &self,
        log: &RawLog<1>,
        model: Range<usize>,
        current: Range<usize>,
        pointer: usize,
    ) -> Option<u64> {
        let (model, current) = (&log.spans[model], &log.spans[current]);
        if !same_shape(model, current) {
            return None;
        }
        let mut hash = DefaultHasher::new();
        pointer.hash(&mut hash);
        for span in current {
            (span.addr, span.len, span.ok, span.bulk).hash(&mut hash);
        }
        for piece in &self.pieces {
            (piece.span, piece.bytes.start, piece.bytes.end).hash(&mut hash);
            hash.write(&payload(log, &current[piece.span])[piece.bytes.clone()]);
        }
        Some(hash.finish())
    }
    pub fn equal(&self, log: &RawLog<1>, left: Range<usize>, right: Range<usize>) -> bool {
        self.mismatch(log, left, right).is_none()
    }
    pub fn mismatch(
        &self,
        log: &RawLog<1>,
        left: Range<usize>,
        right: Range<usize>,
    ) -> Option<CacheOutcome> {
        let (left, right) = (&log.spans[left], &log.spans[right]);
        if let Some(reason) = shape_miss(left, right) {
            return Some(reason);
        }
        if !self.overlaps.iter().all(|overlap| {
            payload(log, &right[overlap.left])[overlap.offset..overlap.offset + overlap.len]
                == payload(log, &right[overlap.right])[..overlap.len]
        }) {
            return Some(CacheOutcome::Overlap);
        }
        if !self.pieces.iter().all(|piece| {
            payload(log, &left[piece.span])[piece.bytes.clone()]
                == payload(log, &right[piece.span])[piece.bytes.clone()]
        }) {
            return Some(CacheOutcome::ReadBytes);
        }
        None
    }
}
fn same_shape(left: &[collector_raw::raw::Span], right: &[collector_raw::raw::Span]) -> bool {
    shape_miss(left, right).is_none()
}
pub(super) fn shape_miss(
    left: &[collector_raw::raw::Span],
    right: &[collector_raw::raw::Span],
) -> Option<CacheOutcome> {
    if left.len() != right.len() {
        return Some(CacheOutcome::SpanCount);
    }
    for (left, right) in left.iter().zip(right) {
        if left.addr != right.addr {
            return Some(CacheOutcome::SpanAddress);
        }
        if left.len != right.len || left.ok != right.ok || left.bulk != right.bulk {
            return Some(CacheOutcome::SpanMetadata);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pieces_are_each_spans_read_runs_across_many_spans_and_reads() {
        const SPANS: usize = 2_048;
        const LEN: usize = 64;
        let base = 0x10_0000;
        let bytes: Vec<u8> = (0..SPANS * LEN).map(|at| at as u8).collect();
        let memory = engine_native::memory::SliceMemory {
            base,
            bytes: &bytes,
        };
        let mut log = RawLog::<1>::new(SPANS, SPANS * LEN);
        for span in 0..SPANS {
            log.take(&memory, base + span * LEN, LEN, 1).unwrap();
        }
        let observed = Observed::new(&memory, SPANS);
        let mut covered = vec![false; SPANS * LEN];
        let mut buffer = [0; 5];
        for at in (0..SPANS * LEN - buffer.len()).step_by(11) {
            let len = 1 + at % buffer.len();
            assert!(observed.copy(base + at, &mut buffer[..len]));
            covered[at..at + len].fill(true);
        }
        let order: Vec<usize> = (0..SPANS).collect();
        let witness = observed.finish(&log, 0..SPANS, &order, false).unwrap();
        let mut expected = Vec::new();
        for span in 0..SPANS {
            let covered = &covered[span * LEN..][..LEN];
            let mut offset = 0;
            while offset < LEN {
                let start = offset;
                while offset < LEN && covered[offset] {
                    offset += 1;
                }
                if start < offset {
                    expected.push(Piece {
                        span,
                        bytes: start..offset,
                    });
                } else {
                    offset += 1;
                }
            }
        }
        assert!(witness.pieces == expected);
    }
}
