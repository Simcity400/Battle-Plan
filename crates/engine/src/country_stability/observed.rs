//! First exact-span observations of one stability walk. Guarded speculative
//! batches retain only complete successes; a failed batch yields to the serial
//! readers once. Their authoritative failures remain memoized without retries.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
};

use crate::{
    Memory,
    memory::{RecordLayout, RecordSources},
};

// One 64-pointer page with the six fixed fields of a dynamic definition.
// Transfer geometry only: later pages and larger payloads continue normally.
const BATCH_FIELDS: usize = 64 * 6;
const BATCH_BYTES: usize = 8192;

/// Shared static observations of one hourly Stability job. The caller clears
/// this before every hour, including failed, skipped, load and full hours.
/// Static modifier binding selects the shared CStaticModifiers; Static modifier
/// lookup owns their +0x1A8 names. Neither country-owned operands nor
/// source-selection pointers enter this memo.
#[derive(Debug, Default)]
pub struct Memo {
    spans: HashMap<(usize, usize), Option<usize>>,
    bytes: Vec<u8>,
}

impl Memo {
    pub fn clear(&mut self) {
        self.spans.clear();
        self.bytes.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes.capacity()
            + self.spans.capacity() * std::mem::size_of::<((usize, usize), Option<usize>)>()
    }

    fn copy(&self, address: usize, destination: &mut [u8]) -> Option<bool> {
        self.spans.get(&(address, destination.len())).map(|start| {
            if let Some(start) = start {
                destination.copy_from_slice(&self.bytes[*start..*start + destination.len()]);
                true
            } else {
                false
            }
        })
    }

    /// Keep the first observation of an exact span. Retention is optional
    /// storage: when it cannot allocate, nothing (no partial bytes) is kept
    /// and the observation stands as acquired, so a later use reads the span
    /// as the walk without the memo did. A failed copy ends the country's
    /// walk, so within a walk it is never read again either way.
    fn retain(&mut self, address: usize, bytes: &[u8], copied: bool) {
        let key = (address, bytes.len());
        if bytes.is_empty() || self.spans.contains_key(&key) {
            return;
        }
        crate::allocation::advisory(|| {
            crate::allocation::reserve_map(&mut self.spans, 1).ok()?;
            let start = if copied {
                crate::allocation::reserve(&mut self.bytes, bytes.len()).ok()?;
                let start = self.bytes.len();
                self.bytes.extend_from_slice(bytes);
                Some(start)
            } else {
                None
            };
            self.spans.insert(key, start);
            Some(())
        });
    }
}

/// Ordinary observations are country-local. Only an explicitly selected
/// static's source or identity reader uses `shared`; subsequent consumers may
/// reuse those exact spans. Context::read already reads slots, defines,
/// modifier metadata and language once per hour.
pub(super) struct Observed<'a, M> {
    source: &'a M,
    local: RefCell<&'a mut Memo>,
    shared: RefCell<&'a mut Memo>,
    fallbacks: Cell<u64>,
    stopped: Cell<bool>,
}

impl<'a, M: Memory> Observed<'a, M> {
    pub(super) fn new(source: &'a M, local: &'a mut Memo, shared: &'a mut Memo) -> Self {
        local.clear();
        Self {
            source,
            local: RefCell::new(local),
            shared: RefCell::new(shared),
            fallbacks: Cell::new(0),
            stopped: Cell::new(false),
        }
    }

    pub(super) fn shared(&self) -> Shared<'_, 'a, M> {
        Shared(self)
    }

    /// Preparation may follow retained controls, but must never acquire a
    /// missing scalar ahead of the authoritative serial object reader.
    pub(super) fn prepared(&self) -> Prepared<'_, 'a, M> {
        Prepared(self)
    }

    pub(super) fn fallbacks(&self) -> u64 {
        self.fallbacks.get()
    }

    /// Exact spans reached through acquired pointers/headers. Bounded transfer
    /// pages are throughput units, never limits on objects, entries or text.
    /// Reserve optional retention BEFORE reading: if it cannot be kept, the
    /// original reader acquires the span itself, with no speculative attempt.
    pub(super) fn gather(&self, spans: impl IntoIterator<Item = (usize, usize)>, shared: bool) {
        // Overlapping later preparation must not speculate on a failed batch
        // again. The rest of this country uses its original readers; successful
        // observations already retained still stand. No retry loop is possible.
        if self.stopped.get() {
            return;
        }
        let mut addresses = [0; BATCH_FIELDS];
        let mut lengths = [0; BATCH_FIELDS];
        let mut count = 0;
        let mut size = 0;
        for (address, len) in spans {
            if len == 0
                || len > BATCH_BYTES
                || address == 0
                || address.checked_add(len).is_none()
                || self.local.borrow().spans.contains_key(&(address, len))
                || self.shared.borrow().spans.contains_key(&(address, len))
                || addresses[..count]
                    .iter()
                    .zip(&lengths[..count])
                    .any(|(&at, &size)| at == address && size == len)
            {
                continue;
            }
            if count == BATCH_FIELDS || size + len > BATCH_BYTES {
                if !self.gather_page(addresses, lengths, count, size, shared) {
                    return;
                }
                addresses.fill(0);
                lengths.fill(0);
                count = 0;
                size = 0;
            }
            addresses[count] = address;
            lengths[count] = len;
            count += 1;
            size += len;
        }
        if count != 0 {
            self.gather_page(addresses, lengths, count, size, shared);
        }
    }

    fn gather_page(
        &self,
        addresses: [usize; BATCH_FIELDS],
        lengths: [usize; BATCH_FIELDS],
        count: usize,
        size: usize,
        shared: bool,
    ) -> bool {
        let mut memo = if shared {
            self.shared.borrow_mut()
        } else {
            self.local.borrow_mut()
        };
        let reserved = crate::allocation::advisory(|| {
            crate::allocation::reserve_map(&mut memo.spans, count).ok()?;
            crate::allocation::reserve(&mut memo.bytes, size).ok()?;
            Some(())
        });
        if reserved.is_none() {
            self.stopped.set(true);
            return false;
        }
        let mut bytes = [0; BATCH_BYTES];
        let mut remaining = &mut bytes[..size];
        let mut destinations: [&mut [u8]; BATCH_FIELDS] = std::array::from_fn(|index| {
            let (destination, tail) = std::mem::take(&mut remaining).split_at_mut(lengths[index]);
            remaining = tail;
            destination
        });
        if self.source.copy_scattered(addresses, &mut destinations) != addresses.len() {
            // None of the failed batch's prefix or failed operand is installed.
            // The callers' ordinary per-object readers run once, in their own
            // order, locating a persistent failure or recovering a one-shot.
            self.fallbacks.set(self.fallbacks.get().saturating_add(1));
            self.stopped.set(true);
            return false;
        }
        let mut offset = 0;
        for (&address, &len) in addresses[..count].iter().zip(&lengths[..count]) {
            let start = memo.bytes.len();
            memo.bytes.extend_from_slice(&bytes[offset..offset + len]);
            memo.spans.insert((address, len), Some(start));
            offset += len;
        }
        true
    }

    fn copy_observed(&self, address: usize, destination: &mut [u8], shared: bool) -> bool {
        if let Some(copied) = self.local.borrow().copy(address, destination) {
            return copied;
        }
        if let Some(copied) = self.shared.borrow().copy(address, destination) {
            return copied;
        }
        let copied = self.source.copy(address, destination);
        if shared {
            self.shared
                .borrow_mut()
                .retain(address, destination, copied);
        } else {
            self.local.borrow_mut().retain(address, destination, copied);
        }
        copied
    }
}

impl<M: Memory> Memory for Observed<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.copy_observed(address, destination, false)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        // Keep existing batches and their failure prefixes intact. Save only
        // each completed exact operand (and the attempted failure), in order.
        let completed = self.source.copy_scattered(addresses, destinations);
        for (index, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter())
            .take(completed.saturating_add(1))
            .enumerate()
        {
            self.local
                .borrow_mut()
                .retain(address, destination, index < completed);
        }
        completed
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let completed = self.source.copy_records(sources, layout, destination);
        let fields = layout.fields();
        for index in 0..count.min(completed.saturating_add(1)) {
            let row = index / fields.len();
            let field = fields[index % fields.len()];
            let Some(address) = sources
                .pointer(row)
                .filter(|&p| p != 0)
                .and_then(|p| p.checked_add(field.source_offset))
                .filter(|p| p.checked_add(field.len).is_some())
            else {
                break;
            };
            let start = row * layout.stride() + field.destination_offset;
            self.local.borrow_mut().retain(
                address,
                &destination[start..start + field.len],
                index < completed,
            );
        }
        completed
    }
}

pub(super) struct Shared<'a, 'b, M>(&'a Observed<'b, M>);

impl<M: Memory> Memory for Shared<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.0.copy_observed(address, destination, true)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.0.source.prefetch(address, len);
    }
}

pub(super) struct Prepared<'a, 'b, M>(&'a Observed<'b, M>);

impl<M: Memory> Memory for Prepared<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if self.0.stopped.get() {
            return false;
        }
        self.0
            .local
            .borrow()
            .copy(address, destination)
            .or_else(|| self.0.shared.borrow().copy(address, destination))
            .unwrap_or(false)
    }
}
