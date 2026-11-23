use collector_raw::raw::{RawLog, Span, is_canonical_range};
use engine_native::{Memory, memory::SliceMemory, walk::Buffer};
use std::cell::{Cell, RefCell};

const SHARED_MASK: u32 = 3;

pub(super) struct ExactIndex<'a> {
    slots: Buffer<'a, usize>,
    used: usize,
    required_slots: usize,
}

impl<'a> ExactIndex<'a> {
    pub(super) fn new(slots: &'a mut Vec<usize>) -> Self {
        slots.fill(usize::MAX);
        Self {
            slots: Buffer::Growing(slots),
            used: 0,
            required_slots: 0,
        }
    }

    pub(super) fn required_slots(&self) -> usize {
        self.required_slots
    }

    fn reset(&mut self) {
        self.slots.fill(usize::MAX);
        self.used = 0;
        self.required_slots = 0;
    }

    fn require(&mut self, keys: usize, spans: &[Span]) -> Result<(), &'static str> {
        let required = keys
            .checked_mul(2)
            .ok_or("carrier exact index capacity overflow")?;
        self.required_slots = self.required_slots.max(required);
        if self.slots.is_growing() && required > self.slots.len() {
            let old = engine_allocation::copy_vec(&self.slots)
                .map_err(|_| "capture buffer allocation failed")?;
            self.slots.ensure(required)?;
            self.slots.fill(usize::MAX);
            self.used = 0;
            for index in old.into_iter().filter(|index| *index != usize::MAX) {
                let span = spans.get(index).ok_or("carrier exact index invalid")?;
                let (slot, previous) = self.lookup(spans, span.addr, span.len)?;
                if previous.is_some() {
                    return Err("carrier exact index invalid");
                }
                self.insert(slot, index);
            }
        }
        if keys > self.slots.len() / 2 {
            return Err("carrier exact index capacity");
        }
        Ok(())
    }

    fn lookup(
        &mut self,
        spans: &[Span],
        address: usize,
        len: u32,
    ) -> Result<(usize, Option<usize>), &'static str> {
        if self.slots.is_empty() {
            self.require(1, spans)?;
        }
        let mut hash = (address as u64 ^ u64::from(len)).wrapping_mul(0xd6e8_feb8_6659_fd93);
        hash ^= hash >> 32;
        let mut slot = hash as usize % self.slots.len();
        for _ in 0..self.slots.len() {
            let index = self.slots[slot];
            if index == usize::MAX {
                return Ok((slot, None));
            }
            let span = spans.get(index).ok_or("carrier exact index invalid")?;
            if span.addr == address && span.len == len {
                return Ok((slot, Some(index)));
            }
            slot += 1;
            if slot == self.slots.len() {
                slot = 0;
            }
        }
        Err("carrier exact index invalid")
    }

    fn insert(&mut self, slot: usize, span: usize) {
        self.slots[slot] = span;
        self.used += 1;
    }
}

pub(super) fn seed_census(
    log: &mut RawLog<2>,
    source: &RawLog<3>,
    index: &mut ExactIndex<'_>,
) -> Result<(), &'static str> {
    log.clear();
    index.reset();
    if source.overflowed {
        return Err("carrier original census overflowed");
    }
    index.require(source.spans.len(), &log.spans)?;
    for span in &source.spans {
        if span.len == 0 {
            continue;
        }
        let bytes = if span.ok {
            let start = span.start as usize;
            let end = start
                .checked_add(span.len as usize)
                .ok_or("carrier census byte range invalid")?;
            Some(
                source
                    .bytes
                    .get(start..end)
                    .ok_or("carrier census byte range invalid")?,
            )
        } else {
            None
        };
        let (slot, previous) = index.lookup(&log.spans, span.addr, span.len)?;
        if let Some(previous) = previous {
            let previous = &log.spans[previous];
            if !previous.ok || !span.ok {
                return Err("carrier census repeated failed observation");
            }
            let start = previous.start as usize;
            let end = start
                .checked_add(previous.len as usize)
                .ok_or("carrier census byte range invalid")?;
            if previous.bulk != span.bulk || log.bytes.get(start..end) != bytes {
                return Err("carrier census repeated observation changed");
            }
            continue;
        }
        let span_index = log.spans.len();
        let copied = match bytes {
            Some(bytes) if span.bulk => log.take_bulk(
                &SliceMemory {
                    base: span.addr,
                    bytes,
                },
                span.addr,
                bytes.len(),
                SHARED_MASK,
            ),
            _ => log.record(span.addr, span.len as usize, bytes, SHARED_MASK),
        };
        log.raw_bytes = [0; 2];
        if log.overflowed {
            return Err("carrier census seed capacity");
        }
        if span.ok && copied.is_none() {
            return Err("carrier census source span invalid");
        }
        if log.spans.len() != span_index + 1 {
            return Err("carrier census source span invalid");
        }
        index.insert(slot, span_index);
    }
    Ok(())
}

pub(super) struct CaptureMemory<'a, 'slots, M: Memory> {
    log: &'a RefCell<RawLog<2>>,
    live: &'a M,
    index: &'a RefCell<ExactIndex<'slots>>,
    mask: Cell<u32>,
}

impl<'a, 'slots, M: Memory> CaptureMemory<'a, 'slots, M> {
    pub(super) fn new(
        log: &'a RefCell<RawLog<2>>,
        live: &'a M,
        index: &'a RefCell<ExactIndex<'slots>>,
    ) -> Self {
        Self {
            log,
            live,
            index,
            mask: Cell::new(SHARED_MASK),
        }
    }

    pub(super) fn set_mask(&self, mask: u32) {
        assert!(matches!(mask, 1..=3), "carrier read mask invalid");
        self.mask.set(mask);
    }
}

fn stored_bulk(log: &RawLog<2>, address: usize, len: usize) -> Option<&Span> {
    let end = address.checked_add(len)?;
    log.spans
        .iter()
        .enumerate()
        .rev()
        .filter_map(|(index, span)| {
            let span_end = span.addr.checked_add(span.len as usize)?;
            (span.ok && span.bulk && span.addr <= address && end <= span_end)
                .then_some(((span.addr, span_end, index), span))
        })
        .max_by_key(|(key, _)| *key)
        .map(|(_, span)| span)
}

fn copy_stored(log: &RawLog<2>, span: &Span, address: usize, destination: &mut [u8]) -> bool {
    if !span.ok {
        return false;
    }
    let Some(offset) = address.checked_sub(span.addr) else {
        return false;
    };
    if offset
        .checked_add(destination.len())
        .is_none_or(|end| end > span.len as usize)
    {
        return false;
    }
    let Some(start) = (span.start as usize).checked_add(offset) else {
        return false;
    };
    let Some(end) = start.checked_add(destination.len()) else {
        return false;
    };
    let Some(bytes) = log.bytes.get(start..end) else {
        return false;
    };
    destination.copy_from_slice(bytes);
    true
}

fn mark_fault(log: &mut RawLog<2>, mask: u32) {
    for (slot, fault) in log.fault.iter_mut().enumerate() {
        if mask & (1 << slot) != 0 {
            *fault = true;
        }
    }
}

impl<M: Memory> Memory for CaptureMemory<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        let mut log = self.log.borrow_mut();
        let Ok(len) = u32::try_from(destination.len()) else {
            let _ = log.take(self.live, address, destination.len(), self.mask.get());
            return false;
        };
        let mut index = self.index.borrow_mut();
        let Ok((_, exact)) = index.lookup(&log.spans, address, len) else {
            return false;
        };
        let valid = address != 0
            && address.checked_add(destination.len()).is_some()
            && is_canonical_range(address, destination.len());
        if let Some(exact) = exact {
            let copied = valid && copy_stored(&log, &log.spans[exact], address, destination);
            if !copied {
                mark_fault(&mut log, self.mask.get());
            }
            return copied;
        }
        if valid
            && log.has_bulk
            && let Some(span) = stored_bulk(&log, address, destination.len())
        {
            let copied = copy_stored(&log, span, address, destination);
            if !copied {
                mark_fault(&mut log, self.mask.get());
            }
            return copied;
        }
        let next_keys = index.used.saturating_add(1);
        if log.overflowed || index.require(next_keys, &log.spans).is_err() {
            return false;
        }
        let Ok((slot, None)) = index.lookup(&log.spans, address, len) else {
            return false;
        };
        let span_index = log.spans.len();
        let copied = log.take(self.live, address, destination.len(), self.mask.get());
        if log.spans.len() == span_index + 1 {
            index.insert(slot, span_index);
        }
        let Some(start) = copied else {
            return false;
        };
        let Some(end) = start.checked_add(destination.len()) else {
            return false;
        };
        let Some(bytes) = log.bytes.get(start..end) else {
            return false;
        };
        destination.copy_from_slice(bytes);
        true
    }
}

#[cfg(test)]
#[path = "carrier_memory_tests.rs"]
mod tests;
