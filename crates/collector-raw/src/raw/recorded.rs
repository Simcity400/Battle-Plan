
use super::{RawLog, Span, is_canonical_range};
use engine_native_memory::Memory;
use std::cell::RefCell;

pub struct Recorded<'a> {
    pub(super) index: RecordedIndex,
    recycle: &'a RefCell<Option<RecordedIndex>>,
    spans: &'a [Span],
    bytes: &'a [u8],
    pub(super) next_exact: std::cell::Cell<usize>,
    pub(super) last_bulk: std::cell::Cell<usize>,
}

#[derive(Debug, Default)]
pub struct RecordedCache {
    index: RefCell<Option<RecordedIndex>>,
}

#[derive(Debug, Default)]
pub(super) struct RecordedIndex {
    has_bulk: bool,
    pub(super) exact_lengths: [u64; 4],
    pub(super) slots: Vec<std::cell::Cell<usize>>,
    pub(super) distinct: usize,
    pub(super) next: Vec<usize>,
    pub(super) repeated: Vec<(usize, usize)>,
    pub(super) shape: Vec<(usize, u32, bool, bool)>,
    pub(super) bulk: Vec<(usize, usize, usize, usize)>,
    capacity_charge: crate::raw_memory::Charge,
}

impl RecordedIndex {
    #[inline]
    pub(super) fn mark_exact_length(&mut self, len: u32) {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] |= 1u64 << (bit % 64);
    }

    #[inline]
    pub(super) fn may_have_exact_length(&self, len: u32) -> bool {
        let bit = len.min(255) as usize;
        self.exact_lengths[bit / 64] & (1u64 << (bit % 64)) != 0
    }

    #[inline]
    pub(super) fn hash(address: usize, len: u32) -> usize {
        let mix = |state: u64, value: u64| (state ^ value).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        let mut value = mix(mix(0, address as u64), u64::from(len));
        value ^= value >> 32;
        value = value.wrapping_mul(0xd6e8_feb8_6659_fd93);
        (value ^ (value >> 32)) as usize
    }

    #[inline]
    pub(super) fn slot(&self, spans: &[Span], address: usize, len: u32) -> Option<usize> {
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
    pub(super) fn cursor(
        &self,
        spans: &[Span],
        address: usize,
        len: u32,
    ) -> Option<&std::cell::Cell<usize>> {
        let cursor = &self.slots[self.slot(spans, address, len)?];
        (cursor.get() != usize::MAX).then_some(cursor)
    }

    fn grow_slots(&mut self, spans: &[Span]) -> std::io::Result<()> {
        let size = if self.slots.is_empty() {
            8
        } else {
            self.slots.len().saturating_mul(2)
        };
        let mut slots = Vec::new();
        crate::allocation::reserve_raw(&mut slots, size, "raw index buckets")?;
        slots.resize(size, std::cell::Cell::new(usize::MAX));
        let old = std::mem::replace(&mut self.slots, slots);
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
        Ok(())
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

    pub(super) fn prepare(&mut self, spans: &[Span], has_bulk: bool) -> std::io::Result<bool> {
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
            return Ok(true);
        }
        crate::allocation::reserve_raw(&mut self.next, spans.len(), "raw index next")?;
        crate::allocation::reserve_raw(&mut self.repeated, spans.len(), "raw index repeated")?;
        crate::allocation::reserve_raw(&mut self.shape, spans.len(), "raw index shape")?;
        if has_bulk {
            crate::allocation::reserve_raw(&mut self.bulk, spans.len(), "raw index bulk")?;
        }
        let target = spans
            .len()
            .saturating_mul(2)
            .checked_next_power_of_two()
            .unwrap_or(self.slots.len())
            .max(8);
        if self.slots.len() > target {
            self.slots.truncate(target);
        }
        self.slots.fill(std::cell::Cell::new(usize::MAX));
        self.distinct = 0;
        self.has_bulk = has_bulk;
        self.exact_lengths = [0; 4];
        self.next.clear();
        self.next.resize(spans.len(), usize::MAX);
        self.repeated.clear();
        self.bulk.clear();
        self.shape.clear();
        let mut grew_after_repeat = false;
        for (index, span) in spans.iter().enumerate() {
            self.shape.push((span.addr, span.len, span.ok, span.bulk));
            self.mark_exact_length(span.len);
            if self.slots.is_empty() {
                self.grow_slots(spans)?;
            }
            let mut slot = self
                .slot(spans, span.addr, span.len)
                .expect("raw index must have an empty bucket");
            let previous = self.slots[slot].get();
            if previous == usize::MAX {
                if self.distinct == self.slots.len() / 2 {
                    self.grow_slots(spans)?;
                    if !self.repeated.is_empty() {
                        grew_after_repeat = true;
                    }
                    slot = self
                        .slot(spans, span.addr, span.len)
                        .expect("grown raw index must have an empty bucket");
                }
                self.slots[slot].set(index);
                self.distinct += 1;
            } else {
                if self.next[previous] == usize::MAX {
                    self.repeated.push((slot, previous));
                }
                self.next[previous] = index;
                self.next[index] = index;
                self.slots[slot].set(index);
            }
            if span.ok && self.has_bulk && span.bulk && span.len > 0 {
                self.bulk
                    .push((span.addr, span.addr + span.len as usize, index, 0));
            }
        }
        if grew_after_repeat {
            for root in 0..self.repeated.len() {
                let first = self.repeated[root].1;
                let span = &spans[first];
                let slot = self
                    .slot(spans, span.addr, span.len)
                    .expect("raw index must retain every repeated key");
                self.repeated[root].0 = slot;
                self.slots[slot].set(first);
            }
        } else {
            for &(slot, first) in &self.repeated {
                self.slots[slot].set(first);
            }
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
        Ok(false)
    }
}

impl<'a> Recorded<'a> {
    pub fn build<const N: usize>(log: &'a RawLog<N>) -> Result<Self, &'static str> {
        Self::build_range(log, 0..log.spans.len())
    }

    pub fn build_range<const N: usize>(
        log: &'a RawLog<N>,
        range: std::ops::Range<usize>,
    ) -> Result<Self, &'static str> {
        Self::build_range_recycling(log, range, &log.index)
    }

    pub fn build_range_cached<const N: usize>(
        log: &'a RawLog<N>,
        range: std::ops::Range<usize>,
        cache: &'a RecordedCache,
    ) -> Result<Self, &'static str> {
        Self::build_range_recycling(log, range, &cache.index)
    }

    fn build_range_recycling<const N: usize>(
        log: &'a RawLog<N>,
        range: std::ops::Range<usize>,
        recycle: &'a RefCell<Option<RecordedIndex>>,
    ) -> Result<Self, &'static str> {
        let source_spans = log.spans.get(range).ok_or("recorded span range invalid")?;
        let _phase = crate::allocation::enter(crate::allocation::Phase::Raw);
        let mut index = recycle
            .borrow_mut()
            .take()
            .unwrap_or_else(|| RecordedIndex {
                capacity_charge: crate::raw_memory::Charge::empty_on(log.capacity_charge.tracker()),
                ..Default::default()
            });
        if !std::ptr::eq(
            index.capacity_charge.tracker(),
            log.capacity_charge.tracker(),
        ) {
            index.capacity_charge =
                crate::raw_memory::Charge::empty_on(log.capacity_charge.tracker());
            index.update_capacity_charge();
        }
        let has_bulk = log.has_bulk && source_spans.iter().any(|span| span.bulk);
        if index.prepare(source_spans, has_bulk).is_err() {
            return Err("recorded span index allocation failed");
        }
        Ok(Self {
            index,
            recycle,
            spans: source_spans,
            bytes: &log.bytes,
            next_exact: std::cell::Cell::new(0),
            last_bulk: std::cell::Cell::new(usize::MAX),
        })
    }

    pub fn unique_observation(&self, address: usize, len: usize) -> Option<(&Span, Option<&Span>)> {
        let len = u32::try_from(len).ok()?;
        let index = self.index.cursor(self.spans, address, len)?.get();
        if self.index.next.get(index) != Some(&usize::MAX) {
            return None;
        }
        Some((
            self.spans.get(index)?,
            index
                .checked_sub(1)
                .and_then(|previous| self.spans.get(previous)),
        ))
    }

    #[inline(always)]
    fn serve(&self, address: usize, destination: &mut [u8]) -> bool {
        let hint = self.next_exact.get();
        let len = destination.len() as u32;
        let exact = if self.spans.get(hint).is_some_and(|span| {
            span.addr == address && span.len == len && self.index.next[hint] == usize::MAX
        }) {
            Some(hint)
        } else if self.index.has_bulk && !self.index.may_have_exact_length(len) {
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
            cached
        } else {
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
    ) -> Option<engine_native_memory::memory::SliceMemory<'_>> {
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
            if enclosing.is_some()
                || !span.ok
                || !span.bulk
                || span.addr == 0
                || !is_canonical_range(span.addr, span.len as usize)
                || span.addr > address
                || span_end < end
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
        Some(engine_native_memory::memory::SliceMemory {
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
        if !is_canonical_range(address, destination.len()) {
            return false;
        }
        if destination.len() > u32::MAX as usize {
            return false;
        }
        self.serve(address, destination)
    }
}
