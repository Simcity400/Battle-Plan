use super::*;
use std::{cell::Cell, ops::Range};

pub(super) fn sources(log: &RawLog<1>, range: Range<usize>, pointer: usize) -> Option<(u32, u32)> {
    let span = log.spans.get(range.start).filter(|_| !range.is_empty())?;
    if !span.ok || span.addr != pointer || span.len as usize != native::STATS {
        return None;
    }
    let bytes = payload(log, span);
    let header: [u8; 16] = bytes[native::BONUS_SOURCES..][..16].try_into().ok()?;
    let array = Array::from_header(header, 8)?;
    Some((
        array.count,
        u32::from_le_bytes(header[8..12].try_into().ok()?),
    ))
}

pub(super) struct MemoryView<'a> {
    log: &'a RawLog<1>,
    range: Range<usize>,
    order: &'a [usize],
    last: Cell<usize>,
}
impl<'a> MemoryView<'a> {
    // Only the cache's complete overlap-consistency check may create this view.
    pub fn new(log: &'a RawLog<1>, range: Range<usize>, order: &'a [usize]) -> Self {
        Self {
            last: Cell::new(range.start),
            log,
            range,
            order,
        }
    }
    fn bytes(&self, index: usize, address: usize, len: usize) -> Option<&'a [u8]> {
        let span = self.log.spans.get(index)?;
        let offset = address.checked_sub(span.addr)?;
        if !span.ok
            || ((!self.log.has_bulk || !span.bulk) && (offset != 0 || len != span.len as usize))
        {
            return None;
        }
        payload(self.log, span).get(offset..offset.checked_add(len)?)
    }
}
impl Memory for MemoryView<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if output.is_empty() {
            return true;
        }
        if address == 0 || !collector_raw::raw::is_canonical_range(address, output.len()) {
            return false;
        }
        let last = self.last.get();
        for index in [last, last.saturating_add(1)] {
            if self.range.contains(&index)
                && let Some(bytes) = self.bytes(index, address, output.len())
            {
                output.copy_from_slice(bytes);
                self.last.set(index);
                return true;
            }
        }
        let end = self
            .order
            .partition_point(|index| self.log.spans[*index].addr <= address);
        for &index in self.order[..end].iter().rev() {
            if let Some(bytes) = self.bytes(index, address, output.len()) {
                output.copy_from_slice(bytes);
                self.last.set(index);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_consistent_view_serves_only_the_same_exact_or_bulk_requests_as_recorded() {
        let first = engine_native::memory::SliceMemory {
            base: 0x1000,
            bytes: &[1, 2, 3, 4, 5, 6, 7, 8],
        };
        let second = engine_native::memory::SliceMemory {
            base: 0x1003,
            bytes: &[4, 5, 6],
        };
        let mut log = RawLog::<1>::new(4, 32);
        log.take_bulk(&first, 0x1000, 8, 1).unwrap();
        log.take(&second, 0x1003, 3, 1).unwrap();
        log.take(&first, 0x1000, 8, 1).unwrap();
        let mut cache = Cache::default();
        assert!(cache.consistent(&log, 0..3).unwrap());
        let memory = MemoryView::new(&log, 0..3, &cache.order);
        let recorded = crate::raw::Recorded::build_range(&log, 0..3).unwrap();
        for address in 0xfff..0x100a {
            for len in 0..12 {
                let mut actual = [0; 12];
                let mut expected = [0; 12];
                let got = memory.copy(address, &mut actual[..len]);
                assert_eq!(got, recorded.copy(address, &mut expected[..len]));
                if got {
                    assert_eq!(actual, expected);
                }
            }
        }
        drop(recorded);
        log.spans[0].bulk = false;
        let memory = MemoryView::new(&log, 0..3, &cache.order);
        assert!(!memory.copy(0x1001, &mut [0; 2]));
        assert!(memory.copy(0x1003, &mut [0; 3]));
        assert!(!memory.copy(0, &mut [0; 1]));
        assert!(!memory.copy(usize::MAX, &mut [0; 2]));
        log.has_bulk = false;
        let memory = MemoryView::new(&log, 0..3, &cache.order);
        let recorded = crate::raw::Recorded::build_range(&log, 0..3).unwrap();
        assert!(!memory.copy(0x1001, &mut [0; 2]));
        assert!(!recorded.copy(0x1001, &mut [0; 2]));
        assert!(memory.copy(0x1000, &mut [0; 8]));
        assert!(recorded.copy(0x1000, &mut [0; 8]));
    }
}
