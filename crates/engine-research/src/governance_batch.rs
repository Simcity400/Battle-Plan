use engine_native_memory::batch_fallback::{self, Batch};
use engine_native_memory::memory::{Memory, RecordLayout, RecordSources};
use std::cell::{Cell, RefCell};

const PENDING: u8 = 0;
const CACHED: u8 = 1;
const FAILED: u8 = 2;

fn destinations_disjoint(layout: &RecordLayout<'_>) -> bool {
    let fields = layout.fields();
    for (index, left) in fields.iter().enumerate() {
        if left.len == 0 {
            continue;
        }
        let left_end = left.destination_offset + left.len;
        for right in &fields[index + 1..] {
            if right.len == 0 {
                continue;
            }
            let right_end = right.destination_offset + right.len;
            if left.destination_offset < right_end && right.destination_offset < left_end {
                return false;
            }
        }
    }
    true
}

pub struct Records<'a> {
    layout: &'a RecordLayout<'a>,
    index: Vec<(usize, usize)>,
    bytes: RefCell<Vec<u8>>,
    states: RefCell<Vec<u8>>,
    cached_bytes: Cell<usize>,
    pending_bytes: Cell<usize>,
    failed_bytes: Cell<usize>,
}

impl<'a> Records<'a> {
    pub fn gather(
        memory: &impl Memory,
        batch: Batch,
        sources: RecordSources<'_>,
        layout: &'a RecordLayout<'a>,
    ) -> Self {
        let mut records = Self {
            layout,
            index: Vec::new(),
            bytes: RefCell::new(Vec::new()),
            states: RefCell::new(Vec::new()),
            cached_bytes: Cell::new(0),
            pending_bytes: Cell::new(0),
            failed_bytes: Cell::new(0),
        };
        let field_count = layout.field_count();
        if sources.is_empty() || field_count == 0 || !destinations_disjoint(layout) {
            return records;
        }
        let field_bytes: usize = layout.fields().iter().map(|field| field.len).sum();
        let mut bytes = Vec::new();
        let mut states = Vec::new();
        let prepared = crate::allocation::advisory(|| {
            let len = sources.len().checked_mul(layout.stride())?;
            sources.len().checked_mul(field_count)?;
            crate::allocation::reserve_exact(&mut records.index, sources.len()).ok()?;
            crate::allocation::reserve_exact(&mut bytes, len).ok()?;
            crate::allocation::reserve_exact(&mut states, len).ok()?;
            for row in 0..sources.len() {
                records.index.push((sources.pointer(row)?, row));
            }
            bytes.resize(len, 0);
            states.resize(len, PENDING);
            Some(())
        });
        if prepared.is_none() {
            records.index.clear();
            return records;
        }
        let copied = memory.copy_records(sources, layout, &mut bytes);
        let done = (copied / field_count).min(sources.len());
        if done != sources.len() {
            batch_fallback::record(batch);
        }
        states[..done * layout.stride()].fill(CACHED);
        records.index.retain(|&(pointer, _)| pointer != 0);
        records.index.sort_unstable();
        records.index.dedup_by_key(|(pointer, _)| *pointer);
        let done_rows = records.index.iter().filter(|&&(_, row)| row < done).count();
        let pending_rows = records.index.len() - done_rows;
        records.bytes = RefCell::new(bytes);
        records.states = RefCell::new(states);
        records.cached_bytes.set(done_rows * field_bytes);
        records.pending_bytes.set(pending_rows * field_bytes);
        records
    }

    fn get_cached<const N: usize>(&self, address: usize) -> Option<[u8; N]> {
        let mut out = [0_u8; N];
        let mut copied = 0;
        while copied < N {
            let at = address.checked_add(copied)?;
            let hit = self.copy_cached_prefix(at, &mut out[copied..]);
            if hit == 0 {
                return None;
            }
            copied += hit;
        }
        Some(out)
    }

    pub fn targets(&self, offset: usize) -> Vec<usize> {
        if self.cached_bytes.get() == 0 {
            return Vec::new();
        }
        crate::allocation::advisory(|| {
            let mut targets = Vec::new();
            crate::allocation::reserve_exact(&mut targets, self.index.len()).ok()?;
            for &(pointer, _) in &self.index {
                if let Some(raw) = pointer
                    .checked_add(offset)
                    .and_then(|at| self.get_cached::<8>(at))
                    && let Ok(target) = usize::try_from(u64::from_le_bytes(raw))
                    && target != 0
                {
                    targets.push(target);
                }
            }
            targets.sort_unstable();
            targets.dedup();
            Some(targets)
        })
        .unwrap_or_default()
    }

    fn cached_span_len(&self, address: usize) -> usize {
        if self.cached_bytes.get() == 0 {
            return 0;
        }
        let states = self.states.borrow();
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let Some(pointer) = address.checked_sub(field.source_offset) else {
                continue;
            };
            let upper = self.index.partition_point(|&(at, _)| at <= pointer);
            for &(at, row) in self.index[..upper].iter().rev() {
                let Some(offset) = at
                    .checked_add(field.source_offset)
                    .and_then(|start| address.checked_sub(start))
                else {
                    break;
                };
                if offset >= field.len {
                    break;
                }
                let base = row * self.layout.stride() + field.destination_offset + offset;
                if states[base] == CACHED {
                    return states[base..base + (field.len - offset)]
                        .iter()
                        .take_while(|&&state| state == CACHED)
                        .count();
                }
            }
        }
        0
    }

    fn copy_cached_prefix(&self, address: usize, destination: &mut [u8]) -> usize {
        if self.cached_bytes.get() == 0 || destination.is_empty() {
            return 0;
        }
        let states = self.states.borrow();
        let bytes = self.bytes.borrow();
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let Some(pointer) = address.checked_sub(field.source_offset) else {
                continue;
            };
            let upper = self.index.partition_point(|&(at, _)| at <= pointer);
            for &(at, row) in self.index[..upper].iter().rev() {
                let Some(offset) = at
                    .checked_add(field.source_offset)
                    .and_then(|start| address.checked_sub(start))
                else {
                    break;
                };
                if offset >= field.len {
                    break;
                }
                let base = row * self.layout.stride() + field.destination_offset + offset;
                if states[base] == CACHED {
                    let max_len = (field.len - offset).min(destination.len());
                    let count = states[base..base + max_len]
                        .iter()
                        .take_while(|&&state| state == CACHED)
                        .count();
                    destination[..count].copy_from_slice(&bytes[base..base + count]);
                    return count;
                }
            }
        }
        0
    }

    fn next_cached(&self, address: usize, limit: usize) -> Option<usize> {
        if self.cached_bytes.get() == 0 || limit <= 1 {
            return None;
        }
        let next_address = address.checked_add(1)?;
        let states = self.states.borrow();
        let end = address.saturating_add(limit);
        let mut best: Option<usize> = None;
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let min_at = field
                .source_offset
                .checked_add(field.len - 1)
                .map_or(0, |span| next_address.saturating_sub(span));
            let start_idx = self.index.partition_point(|&(at, _)| at < min_at);
            for &(at, row) in &self.index[start_idx..] {
                let Some(f_start) = at.checked_add(field.source_offset) else {
                    break;
                };
                let limit_end = best.map_or(end, |distance| address.saturating_add(distance));
                if f_start >= limit_end {
                    break;
                }
                let Some(f_end) = f_start.checked_add(field.len) else {
                    break;
                };
                let scan_start = next_address.max(f_start);
                let scan_end = f_end.min(limit_end);
                if scan_start >= scan_end {
                    continue;
                }
                let base = row * self.layout.stride()
                    + field.destination_offset
                    + (scan_start - f_start);
                if let Some(pos) = states[base..base + (scan_end - scan_start)]
                    .iter()
                    .position(|&state| state == CACHED)
                {
                    let distance = (scan_start + pos) - address;
                    best = Some(best.map_or(distance, |current| current.min(distance)));
                }
            }
        }
        best
    }

    fn next_pending_boundary(&self, address: usize, limit: usize) -> Option<usize> {
        if self.pending_bytes.get() == 0 || limit <= 1 {
            return None;
        }
        let states = self.states.borrow();
        let end = address.saturating_add(limit);
        let mut best: Option<usize> = None;
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let min_at = field
                .source_offset
                .checked_add(field.len - 1)
                .map_or(0, |span| address.saturating_sub(span));
            let start_idx = self.index.partition_point(|&(at, _)| at < min_at);
            for &(at, row) in &self.index[start_idx..] {
                let Some(f_start) = at.checked_add(field.source_offset) else {
                    break;
                };
                let limit_end = best.map_or(end, |distance| address.saturating_add(distance));
                if f_start >= limit_end {
                    break;
                }
                let Some(f_end) = f_start.checked_add(field.len) else {
                    break;
                };
                let base = row * self.layout.stride() + field.destination_offset;
                let clipped_end = f_end.min(limit_end);
                if address < f_start {
                    if let Some(pos) = states[base..base + (clipped_end - f_start)]
                        .iter()
                        .position(|&state| state == PENDING)
                    {
                        let distance = (f_start + pos) - address;
                        best = Some(best.map_or(distance, |current| current.min(distance)));
                    }
                } else {
                    let rel = address - f_start;
                    let in_pending = states[base + rel] == PENDING;
                    let limit_rel = clipped_end - f_start;
                    if let Some(pos) = states[base + rel + 1..base + limit_rel]
                        .iter()
                        .position(|&state| (state == PENDING) != in_pending)
                    {
                        let distance = rel + 1 + pos - rel;
                        best = Some(best.map_or(distance, |current| current.min(distance)));
                    } else if in_pending && f_end < limit_end {
                        let distance = f_end - address;
                        best = Some(best.map_or(distance, |current| current.min(distance)));
                    }
                }
            }
        }
        best
    }

    fn intersects_pending(&self, address: usize, len: usize) -> bool {
        if self.pending_bytes.get() == 0 || len == 0 {
            return false;
        }
        let end = address.saturating_add(len);
        let states = self.states.borrow();
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let min_at = field
                .source_offset
                .checked_add(field.len - 1)
                .map_or(0, |span| address.saturating_sub(span));
            let start_idx = self.index.partition_point(|&(at, _)| at < min_at);
            for &(at, row) in &self.index[start_idx..] {
                let Some(f_start) = at.checked_add(field.source_offset) else {
                    break;
                };
                if f_start >= end {
                    break;
                }
                let f_end = f_start.saturating_add(field.len);
                let overlap_start = address.max(f_start);
                let overlap_end = end.min(f_end);
                let base = row * self.layout.stride()
                    + field.destination_offset
                    + (overlap_start - f_start);
                if states[base..base + (overlap_end - overlap_start)].contains(&PENDING) {
                    return true;
                }
            }
        }
        false
    }

    fn mark_copied(&self, address: usize, data: &[u8]) {
        if self.pending_bytes.get() == 0 || data.is_empty() {
            return;
        }
        let end = address.saturating_add(data.len());
        let mut states = self.states.borrow_mut();
        let mut bytes = self.bytes.borrow_mut();
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let min_at = field
                .source_offset
                .checked_add(field.len - 1)
                .map_or(0, |span| address.saturating_sub(span));
            let start_idx = self.index.partition_point(|&(at, _)| at < min_at);
            for &(at, row) in &self.index[start_idx..] {
                let Some(f_start) = at.checked_add(field.source_offset) else {
                    break;
                };
                if f_start >= end {
                    break;
                }
                let f_end = f_start.saturating_add(field.len);
                let overlap_start = address.max(f_start);
                let overlap_end = end.min(f_end);
                let dst_base = row * self.layout.stride()
                    + field.destination_offset
                    + (overlap_start - f_start);
                let src_base = overlap_start - address;
                let count = overlap_end - overlap_start;
                let mut newly_cached = 0;
                for offset in 0..count {
                    if states[dst_base + offset] == PENDING {
                        bytes[dst_base + offset] = data[src_base + offset];
                        states[dst_base + offset] = CACHED;
                        newly_cached += 1;
                    }
                }
                if newly_cached != 0 {
                    self.pending_bytes.set(self.pending_bytes.get() - newly_cached);
                    self.cached_bytes.set(self.cached_bytes.get() + newly_cached);
                }
            }
        }
    }

    fn mark_failed(&self, address: usize, len: usize) {
        if self.pending_bytes.get() == 0 || len == 0 {
            return;
        }
        let end = address.saturating_add(len);
        let mut states = self.states.borrow_mut();
        for field in self.layout.fields() {
            if field.len == 0 {
                continue;
            }
            let min_at = field
                .source_offset
                .checked_add(field.len - 1)
                .map_or(0, |span| address.saturating_sub(span));
            let start_idx = self.index.partition_point(|&(at, _)| at < min_at);
            for &(at, row) in &self.index[start_idx..] {
                let Some(f_start) = at.checked_add(field.source_offset) else {
                    break;
                };
                if f_start >= end {
                    break;
                }
                let f_end = f_start.saturating_add(field.len);
                let overlap_start = address.max(f_start);
                let overlap_end = end.min(f_end);
                let dst_base = row * self.layout.stride()
                    + field.destination_offset
                    + (overlap_start - f_start);
                let count = overlap_end - overlap_start;
                let mut newly_failed = 0;
                for offset in 0..count {
                    if states[dst_base + offset] == PENDING {
                        states[dst_base + offset] = FAILED;
                        newly_failed += 1;
                    }
                }
                if newly_failed != 0 {
                    self.pending_bytes.set(self.pending_bytes.get() - newly_failed);
                    self.failed_bytes.set(self.failed_bytes.get() + newly_failed);
                }
            }
        }
    }

    pub fn view<'b, M: Memory>(&'b self, source: &'b M) -> View<'b, 'a, M> {
        View {
            source,
            records: std::slice::from_ref(self),
        }
    }
}

pub struct View<'b, 'a, M> {
    source: &'b M,
    records: &'b [Records<'a>],
}

impl<M> View<'_, '_, M> {
    fn cached_span_len(&self, address: usize) -> usize {
        for records in self.records {
            let len = records.cached_span_len(address);
            if len != 0 {
                return len;
            }
        }
        0
    }

    fn has_uncached_failed(&self, address: usize, len: usize) -> bool {
        if len == 0 || self.records.iter().all(|r| r.failed_bytes.get() == 0) {
            return false;
        }
        let end = address.saturating_add(len);
        for records in self.records {
            if records.failed_bytes.get() == 0 {
                continue;
            }
            let states = records.states.borrow();
            for field in records.layout.fields() {
                if field.len == 0 {
                    continue;
                }
                let min_at = field
                    .source_offset
                    .checked_add(field.len - 1)
                    .map_or(0, |span| address.saturating_sub(span));
                let start_idx = records.index.partition_point(|&(at, _)| at < min_at);
                for &(at, row) in &records.index[start_idx..] {
                    let Some(f_start) = at.checked_add(field.source_offset) else {
                        break;
                    };
                    if f_start >= end {
                        break;
                    }
                    let f_end = f_start.saturating_add(field.len);
                    let overlap_start = address.max(f_start);
                    let overlap_end = end.min(f_end);
                    let dst_base = row * records.layout.stride()
                        + field.destination_offset
                        + (overlap_start - f_start);
                    let count = overlap_end - overlap_start;
                    for offset in 0..count {
                        if states[dst_base + offset] == FAILED
                            && self.cached_span_len(overlap_start + offset) == 0
                        {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn copy_cached_prefix(&self, address: usize, destination: &mut [u8]) -> usize {
        for records in self.records {
            let hit = records.copy_cached_prefix(address, destination);
            if hit != 0 {
                return hit;
            }
        }
        0
    }

    fn next_boundary(&self, address: usize, limit: usize) -> Option<usize> {
        let cached = self
            .records
            .iter()
            .filter_map(|records| records.next_cached(address, limit))
            .min();
        let pending = self
            .records
            .iter()
            .filter_map(|records| records.next_pending_boundary(address, limit))
            .min();
        match (cached, pending) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    fn intersects_any(&self, address: usize, len: usize) -> bool {
        len != 0
            && (self.cached_span_len(address) != 0
                || self.next_boundary(address, len).is_some()
                || self.has_uncached_failed(address, len)
                || self
                    .records
                    .iter()
                    .any(|records| records.intersects_pending(address, len)))
    }

    fn mark_copied(&self, address: usize, data: &[u8]) {
        for records in self.records {
            records.mark_copied(address, data);
        }
    }

    fn mark_failed(&self, address: usize, len: usize) {
        for records in self.records {
            records.mark_failed(address, len);
        }
    }
}

impl<M: Memory> Memory for View<'_, '_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return self.source.copy(address, destination);
        }
        if self.has_uncached_failed(address, destination.len()) {
            return false;
        }
        let mut copied = 0;
        while copied < destination.len() {
            let Some(at) = address.checked_add(copied) else {
                return false;
            };
            let hit = self.copy_cached_prefix(at, &mut destination[copied..]);
            if hit != 0 {
                copied += hit;
            } else {
                let remaining = destination.len() - copied;
                let count = self.next_boundary(at, remaining).unwrap_or(remaining);
                let out = &mut destination[copied..copied + count];
                if !self.source.copy(at, out) {
                    self.mark_failed(at, count);
                    return false;
                }
                self.mark_copied(at, out);
                copied += count;
            }
        }
        true
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.source.prefetch(address, len);
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if !addresses
            .iter()
            .zip(destinations.iter())
            .any(|(&at, out)| self.intersects_any(at, out.len()))
        {
            return self.source.copy_scattered(addresses, destinations);
        }
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return completed;
            }
        }
        N
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
        if count == 0 {
            return 0;
        }
        let piecewise = (0..sources.len()).any(|row| {
            sources.pointer(row).is_some_and(|pointer| {
                pointer != 0
                    && layout.fields().iter().any(|field| {
                        field.len != 0
                            && pointer
                                .checked_add(field.source_offset)
                                .is_some_and(|at| self.intersects_any(at, field.len))
                    })
            })
        });
        if !piecewise {
            return self.source.copy_records(sources, layout, destination);
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        completed
    }
}

const POINTER_FIELDS: [engine_native_memory::memory::RecordField; 1] = engine_native_memory::memory::packed_fields([(0, 8)]);
pub const POINTER_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POINTER_FIELDS, 8).expect("valid pointer layout");

const ASSIGNED_FIELDS: [engine_native_memory::memory::RecordField; 7] = [
    crate::ctechnology::FIELDS[0],
    crate::ctechnology::FIELDS[1],
    crate::ctechnology::FIELDS[2],
    crate::ctechnology::FIELDS[3],
    crate::ctechnology::FIELDS[4],
    crate::ctechnology::FIELDS[5],
    engine_native_memory::memory::RecordField {
        source_offset: crate::ctechnology::APPLIED_SPEED_BONUS,
        destination_offset: 56,
        len: 24,
    },
];
const ASSIGNED_LAYOUT: RecordLayout<'static> = RecordLayout::new(&ASSIGNED_FIELDS, 80).unwrap();
const TECHNOLOGY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&crate::ctechnology::FIELDS, 56).unwrap();
const TEMPLATE_FIELDS: [engine_native_memory::memory::RecordField; 3] = engine_native_memory::memory::packed_fields([
    (crate::ctechnology_template::ID, 32),
    (crate::ctechnology_template::DATABASE_INDEX, 8),
    (crate::ctechnology_template::YEAR, 16),
]);
const TEMPLATE_LAYOUT: RecordLayout<'static> = RecordLayout::new(&TEMPLATE_FIELDS, 56).unwrap();
const IDENTITY_FIELDS: [engine_native_memory::memory::RecordField; 1] =
    engine_native_memory::memory::packed_fields([(crate::ctechnology_template::DATABASE_INDEX, 8)]);
const IDENTITY_LAYOUT: RecordLayout<'static> = RecordLayout::new(&IDENTITY_FIELDS, 8).unwrap();

pub(crate) struct Assignments([Records<'static>; 4]);
impl Assignments {
    pub(crate) fn gather(
        memory: &impl Memory,
        roster: engine_native_memory::cpdx_array::Array,
        len: usize,
        inputs: bool,
    ) -> Self {
        let pointers = Records::gather(
            memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: roster.data,
                stride: 8,
                count: len,
            },
            &POINTER_LAYOUT,
        );
        let targets = pointers.targets(0);
        let slots = Records::gather(
            memory,
            Batch::GovernanceSlots,
            RecordSources::Pointers(&targets),
            &crate::cresearch_slot::LAYOUT,
        );
        let targets = slots.targets(crate::cresearch_slot::TECHNOLOGY);
        let technologies = Records::gather(
            memory,
            Batch::GovernanceAssignedTechnologies,
            RecordSources::Pointers(&targets),
            if inputs {
                &ASSIGNED_LAYOUT
            } else {
                &TECHNOLOGY_LAYOUT
            },
        );
        let targets = technologies.targets(crate::ctechnology::TEMPLATE);
        let templates = Records::gather(
            memory,
            Batch::GovernanceAssignedTemplates,
            RecordSources::Pointers(&targets),
            if inputs {
                &TEMPLATE_LAYOUT
            } else {
                &IDENTITY_LAYOUT
            },
        );
        Self([pointers, slots, technologies, templates])
    }
    pub(crate) fn view<'a, M: Memory>(&'a self, source: &'a M) -> View<'a, 'static, M> {
        View {
            source,
            records: &self.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::{RecordField, SliceMemory};
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;

    const BASE: usize = 0x1000;

    struct CountedFault<'a> {
        inner: SliceMemory<'a>,
        fault: Option<usize>,
        once: bool,
        spent: Cell<bool>,
        reads: RefCell<HashMap<usize, usize>>,
    }

    impl<'a> CountedFault<'a> {
        fn new(bytes: &'a [u8], fault: Option<usize>, once: bool) -> Self {
            Self {
                inner: SliceMemory { base: BASE, bytes },
                fault,
                once,
                spent: Cell::new(false),
                reads: RefCell::new(HashMap::new()),
            }
        }

        fn reads_at(&self, address: usize) -> usize {
            self.reads.borrow().get(&address).copied().unwrap_or(0)
        }
    }

    impl Memory for CountedFault<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            {
                let mut reads = self.reads.borrow_mut();
                for offset in 0..destination.len() {
                    *reads.entry(address + offset).or_insert(0) += 1;
                }
            }
            if self
                .fault
                .is_some_and(|at| address <= at && at < address.saturating_add(destination.len()))
                && (!self.once || !self.spent.get())
            {
                if self.once {
                    self.spent.set(true);
                }
                destination.fill(0xff);
                return false;
            }
            self.inner.copy(address, destination)
        }
    }

    fn put(bytes: &mut [u8], address: usize, value: u64) {
        let at = address - BASE;
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }

    #[test]
    fn partial_batch_keeps_completed_rows_retries_faulted_once_and_reads_pages_piecewise() {
        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1000, 0x1200);
        put(&mut bytes, 0x1008, 0x1300);
        put(&mut bytes, 0x1010, 0x1400);

        let memory = CountedFault::new(&bytes, Some(0x1008), true);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: 0x1000,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        assert_eq!(pointers.targets(0), vec![0x1200]);
        let view = pointers.view(&memory);
        let mut page = [0_u8; 24];
        assert!(view.copy(0x1000, &mut page));
        assert_eq!(&page, &bytes[..24]);
        assert_eq!(memory.reads_at(0x1000), 1);
        assert_eq!(memory.reads_at(0x1008), 2);
        assert_eq!(memory.reads_at(0x1010), 1);

        let memory = CountedFault::new(&bytes, Some(0x1008), false);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: 0x1000,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        assert_eq!(pointers.targets(0), vec![0x1200]);
        let view = pointers.view(&memory);
        let mut page = [0_u8; 24];
        assert!(!view.copy(0x1000, &mut page));
        assert_eq!(memory.reads_at(0x1000), 1);
        assert_eq!(memory.reads_at(0x1008), 2);
        let mut single = [0_u8; 8];
        assert!(!view.copy(0x1008, &mut single));
        assert_eq!(memory.reads_at(0x1008), 2);
        assert!(view.copy(0x1010, &mut single));
        assert_eq!(u64::from_le_bytes(single), 0x1400);
        assert_eq!(memory.reads_at(0x1010), 1);

        let memory = CountedFault::new(&bytes, Some(0x1010), false);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: 0x1000,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        let view = pointers.view(&memory);
        let mut page = [0_u8; 24];
        assert!(!view.copy(0x1000, &mut page));
        assert_eq!(memory.reads_at(0x1000), 1);
        assert_eq!(memory.reads_at(0x1008), 1);
        assert_eq!(memory.reads_at(0x1010), 2);
        assert!(view.copy(0x1008, &mut single));
        assert_eq!(u64::from_le_bytes(single), 0x1300);
        assert_eq!(memory.reads_at(0x1008), 1);
        assert!(!view.copy(0x1010, &mut single));
        assert_eq!(memory.reads_at(0x1010), 2);

        let memory = CountedFault::new(&bytes, Some(0x1000), true);
        let pointers = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1008, 0x1000]),
            &POINTER_LAYOUT,
        );
        let view = pointers.view(&memory);
        let mut page = [0_u8; 16];
        assert!(view.copy(0x1000, &mut page));
        assert_eq!(&page, &bytes[..16]);
        assert_eq!(memory.reads_at(0x1008), 1);
        assert_eq!(memory.reads_at(0x1000), 2);
    }

    #[test]
    fn partial_batch_alias_lookup_uses_earliest_completed_duplicate_and_scattered_keeps_cached() {
        const PAIR_FIELDS: [RecordField; 2] = engine_native_memory::memory::packed_fields([(0, 8), (0x20, 8)]);
        const PAIR_LAYOUT: RecordLayout<'static> = RecordLayout::new(&PAIR_FIELDS, 16).unwrap();

        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1200, 11);
        put(&mut bytes, 0x1220, 22);
        put(&mut bytes, 0x1300, 33);
        put(&mut bytes, 0x1320, 44);

        for once in [true, false] {
            let memory = CountedFault::new(&bytes, Some(0x1320), once);
            let records = Records::gather(
                &memory,
                Batch::GovernanceAssignedTechnologies,
                RecordSources::Pointers(&[0x1200, 0x1300, 0x1200]),
                &PAIR_LAYOUT,
            );
            let view = records.view(&memory);
            let mut a0 = [0_u8; 8];
            let mut a1 = [0_u8; 8];
            assert_eq!(view.copy_scattered([0x1200, 0x1220], &mut [&mut a0, &mut a1]), 2);
            assert_eq!(u64::from_le_bytes(a0), 11);
            assert_eq!(u64::from_le_bytes(a1), 22);
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1220), 1);

            let mut mixed_a = [0_u8; 8];
            let mut mixed_b = [0_u8; 8];
            let copied = view.copy_scattered([0x1200, 0x1320], &mut [&mut mixed_a, &mut mixed_b]);
            assert_eq!(copied, if once { 2 } else { 1 });
            assert_eq!(u64::from_le_bytes(mixed_a), 11);
            if once {
                assert_eq!(u64::from_le_bytes(mixed_b), 44);
            }
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1320), 2);

            let mut out = [0_u8; 16];
            assert_eq!(
                view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut out),
                2
            );
            assert_eq!(memory.reads_at(0x1200), 1);
            assert_eq!(memory.reads_at(0x1220), 1);
        }

        let memory = CountedFault::new(&bytes, None, false);
        let overflow = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Strided {
                base: usize::MAX - 8,
                stride: 8,
                count: 3,
            },
            &POINTER_LAYOUT,
        );
        assert!(overflow.index.is_empty());
        assert!(overflow.bytes.borrow().is_empty());
        assert!(overflow.states.borrow().is_empty());
    }

    #[test]
    fn view_retry_memoization_uses_zero_allocations_records_failures_and_handles_overlaps() {
        const PAIR_FIELDS: [RecordField; 2] = engine_native_memory::memory::packed_fields([(0, 8), (0x20, 8)]);
        const PAIR_LAYOUT: RecordLayout<'static> = RecordLayout::new(&PAIR_FIELDS, 16).unwrap();

        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1200, 11);
        put(&mut bytes, 0x1220, 22);

        let memory = CountedFault::new(&bytes, Some(0x1220), true);
        let records = Records::gather(
            &memory,
            Batch::GovernanceAssignedTechnologies,
            RecordSources::Pointers(&[0x1200]),
            &PAIR_LAYOUT,
        );
        let growths = crate::allocation::growths();
        let view = records.view(&memory);
        let mut first = [0_u8; 16];
        let mut second = [0_u8; 16];
        assert_eq!(
            view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut first),
            2
        );
        assert_eq!(
            view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut second),
            2
        );
        assert_eq!(first, second);
        assert_eq!(memory.reads_at(0x1220), 2);
        assert_eq!(crate::allocation::growths(), growths);

        struct TwoShot<'a> {
            inner: SliceMemory<'a>,
            fault: usize,
            attempts: Cell<usize>,
        }
        impl Memory for TwoShot<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= self.fault && self.fault < address.saturating_add(destination.len()) {
                    let attempt = self.attempts.get() + 1;
                    self.attempts.set(attempt);
                    if attempt <= 2 {
                        destination.fill(0xff);
                        return false;
                    }
                }
                self.inner.copy(address, destination)
            }
        }
        let two_shot = TwoShot {
            inner: SliceMemory {
                base: BASE,
                bytes: &bytes,
            },
            fault: 0x1220,
            attempts: Cell::new(0),
        };
        let records = Records::gather(
            &two_shot,
            Batch::GovernanceAssignedTechnologies,
            RecordSources::Pointers(&[0x1200]),
            &PAIR_LAYOUT,
        );
        let growths = crate::allocation::growths();
        let view = records.view(&two_shot);
        let mut out = [0_u8; 16];
        assert_eq!(
            view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut out),
            1
        );
        assert_eq!(
            view.copy_records(RecordSources::Pointers(&[0x1200]), &PAIR_LAYOUT, &mut out),
            1
        );
        assert_eq!(two_shot.attempts.get(), 2);
        assert_eq!(crate::allocation::growths(), growths);

        const OVERLAP_DST_FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 8,
            },
            RecordField {
                source_offset: 0x20,
                destination_offset: 0,
                len: 8,
            },
        ];
        const OVERLAP_DST_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&OVERLAP_DST_FIELDS, 8).unwrap();
        let memory = CountedFault::new(&bytes, Some(0x1220), true);
        let records = Records::gather(
            &memory,
            Batch::GovernanceAssignedTechnologies,
            RecordSources::Pointers(&[0x1200]),
            &PAIR_LAYOUT,
        );
        let view = records.view(&memory);
        let mut aliased = [0_u8; 8];
        assert_eq!(
            view.copy_records(
                RecordSources::Pointers(&[0x1200]),
                &OVERLAP_DST_LAYOUT,
                &mut aliased,
            ),
            2
        );
        let mut check = [0_u8; 8];
        assert!(view.copy(0x1200, &mut check));
        assert_eq!(u64::from_le_bytes(check), 11);
        assert!(view.copy(0x1220, &mut check));
        assert_eq!(u64::from_le_bytes(check), 22);

        const OVERLAP_SRC_FIELDS: [RecordField; 2] =
            engine_native_memory::memory::packed_fields([(0x4c, 4), (0x40, 16)]);
        const OVERLAP_SRC_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&OVERLAP_SRC_FIELDS, 20).unwrap();
        put(&mut bytes, 0x1040, 0x1111_2222_3333_4444);
        put(&mut bytes, 0x1048, 0x5555_6666_7777_8888);
        let memory = CountedFault::new(&bytes, Some(0x104c), true);
        let records = Records::gather(
            &memory,
            Batch::GovernanceFocusProgress,
            RecordSources::Pointers(&[0x1000]),
            &OVERLAP_SRC_LAYOUT,
        );
        let view = records.view(&memory);
        let mut count_bytes = [0_u8; 4];
        assert!(view.copy(0x104c, &mut count_bytes));
        let mut header_bytes = [0_u8; 16];
        assert!(view.copy(0x1040, &mut header_bytes));
        assert_eq!(&header_bytes, &bytes[0x40..0x50]);
        assert_eq!(memory.reads_at(0x104c), 2);
    }

    #[test]
    fn overlapping_pending_spans_do_not_reread_shared_bytes_in_bulk_or_misattribute_faults() {
        struct FaultFirstAndThird<'a> {
            inner: SliceMemory<'a>,
            fault: usize,
            reads: RefCell<HashMap<usize, usize>>,
        }
        impl<'a> FaultFirstAndThird<'a> {
            fn new(bytes: &'a [u8], fault: usize) -> Self {
                Self {
                    inner: SliceMemory { base: BASE, bytes },
                    fault,
                    reads: RefCell::new(HashMap::new()),
                }
            }
            fn reads_at(&self, address: usize) -> usize {
                self.reads.borrow().get(&address).copied().unwrap_or(0)
            }
        }
        impl Memory for FaultFirstAndThird<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let mut fail = false;
                {
                    let mut reads = self.reads.borrow_mut();
                    for offset in 0..destination.len() {
                        let at = address + offset;
                        let count = reads.entry(at).or_insert(0);
                        *count += 1;
                        if at == self.fault && (*count == 1 || *count >= 3) {
                            fail = true;
                        }
                    }
                }
                if fail {
                    destination.fill(0xff);
                    return false;
                }
                self.inner.copy(address, destination)
            }
        }

        let mut bytes = vec![0_u8; 0x1000];
        put(&mut bytes, 0x1200, 0x1122_3344_5566_7788);
        put(&mut bytes, 0x1208, 0x99aa_bbcc_ddee_ff00);

        let memory = FaultFirstAndThird::new(&bytes, 0x1204);
        let records = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1200, 0x1204]),
            &POINTER_LAYOUT,
        );
        let view = records.view(&memory);
        let mut first = [0_u8; 8];
        let mut second = [0_u8; 8];
        assert_eq!(
            view.copy_scattered([0x1200, 0x1204], &mut [&mut first, &mut second]),
            2
        );
        assert_eq!(&first, &bytes[0x200..0x208]);
        assert_eq!(&second, &bytes[0x204..0x20c]);
        assert_eq!(memory.reads_at(0x1204), 2);

        let memory = FaultFirstAndThird::new(&bytes, 0x1204);
        let records = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1200, 0x1204]),
            &POINTER_LAYOUT,
        );
        let view = records.view(&memory);
        let mut out = [0_u8; 16];
        assert_eq!(
            view.copy_records(
                RecordSources::Pointers(&[0x1200, 0x1204]),
                &POINTER_LAYOUT,
                &mut out,
            ),
            2
        );
        assert_eq!(&out[..8], &bytes[0x200..0x208]);
        assert_eq!(&out[8..16], &bytes[0x204..0x20c]);
        assert_eq!(memory.reads_at(0x1204), 2);

        let memory = CountedFault::new(&bytes, Some(0x1200), false);
        let records = Records::gather(
            &memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1200, 0x1204]),
            &POINTER_LAYOUT,
        );
        let view = records.view(&memory);
        let mut first = [0_u8; 8];
        let mut second = [0_u8; 8];
        assert!(!view.copy(0x1200, &mut first));
        assert_eq!(memory.reads_at(0x1200), 2);
        assert!(view.copy(0x1204, &mut second));
        assert_eq!(&second, &bytes[0x204..0x20c]);
        assert_eq!(memory.reads_at(0x1204), 2);
        assert_eq!(memory.reads_at(0x1208), 1);
        assert!(view.copy(0x1204, &mut second));
        assert_eq!(memory.reads_at(0x1204), 2);
        assert_eq!(memory.reads_at(0x1208), 1);

        let mut split_bytes = vec![0_u8; 0x1000];
        put(&mut split_bytes, 0x1200, 0x0102_0304_0506_0708);
        put(&mut split_bytes, 0x1208, 0x1112_1314_1516_1718);
        struct SplitFault<'a> {
            inner: SliceMemory<'a>,
            once_at: usize,
            always_at: usize,
            spent: Cell<bool>,
            reads: RefCell<HashMap<usize, usize>>,
        }
        impl Memory for SplitFault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let end = address.saturating_add(destination.len());
                {
                    let mut reads = self.reads.borrow_mut();
                    for offset in 0..destination.len() {
                        *reads.entry(address + offset).or_insert(0) += 1;
                    }
                }
                if address <= self.always_at && self.always_at < end {
                    destination.fill(0xff);
                    return false;
                }
                if address <= self.once_at && self.once_at < end && !self.spent.get() {
                    self.spent.set(true);
                    destination.fill(0xff);
                    return false;
                }
                self.inner.copy(address, destination)
            }
        }
        let split_memory = SplitFault {
            inner: SliceMemory {
                base: BASE,
                bytes: &split_bytes,
            },
            once_at: 0x1200,
            always_at: 0x1208,
            spent: Cell::new(false),
            reads: RefCell::new(HashMap::new()),
        };
        let records = Records::gather(
            &split_memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1200, 0x1204, 0x1208]),
            &POINTER_LAYOUT,
        );
        let view = records.view(&split_memory);
        let mut page = [0_u8; 16];
        assert!(!view.copy(0x1200, &mut page));
        let before_0 = split_memory.reads.borrow().get(&0x1200).copied().unwrap_or(0);
        let before_4 = split_memory.reads.borrow().get(&0x1204).copied().unwrap_or(0);
        let mut recovered = [0_u8; 8];
        assert!(view.copy(0x1200, &mut recovered));
        assert_eq!(&recovered, &split_bytes[0x200..0x208]);
        assert_eq!(
            split_memory.reads.borrow().get(&0x1200).copied().unwrap_or(0),
            before_0
        );
        assert_eq!(
            split_memory.reads.borrow().get(&0x1204).copied().unwrap_or(0),
            before_4
        );

        const WIDE_FIELDS: [RecordField; 1] = engine_native_memory::memory::packed_fields([(0, 12)]);
        const WIDE_LAYOUT: RecordLayout<'static> = RecordLayout::new(&WIDE_FIELDS, 12).unwrap();
        let mut wide_bytes = vec![0_u8; 0x1000];
        wide_bytes[0x300..0x30c].copy_from_slice(b"0123456789ab");
        struct DisjointFault<'a> {
            inner: SliceMemory<'a>,
            faults: [usize; 2],
            reads: RefCell<HashMap<usize, usize>>,
        }
        impl Memory for DisjointFault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let end = address.saturating_add(destination.len());
                {
                    let mut reads = self.reads.borrow_mut();
                    for offset in 0..destination.len() {
                        *reads.entry(address + offset).or_insert(0) += 1;
                    }
                }
                if self.faults.iter().any(|&at| address <= at && at < end) {
                    destination.fill(0xff);
                    return false;
                }
                self.inner.copy(address, destination)
            }
        }
        let wide_memory = DisjointFault {
            inner: SliceMemory {
                base: BASE,
                bytes: &wide_bytes,
            },
            faults: [0x1300, 0x1308],
            reads: RefCell::new(HashMap::new()),
        };
        let records = Records::gather(
            &wide_memory,
            Batch::GovernanceSlotPointers,
            RecordSources::Pointers(&[0x1300]),
            &WIDE_LAYOUT,
        );
        let view = records.view(&wide_memory);
        let mut chunk = [0_u8; 4];
        assert!(!view.copy(0x1300, &mut chunk));
        assert!(!view.copy(0x1308, &mut chunk));
        let reads_0 = wide_memory.reads.borrow().get(&0x1300).copied().unwrap_or(0);
        let reads_8 = wide_memory.reads.borrow().get(&0x1308).copied().unwrap_or(0);
        assert!(!view.copy(0x1300, &mut chunk));
        assert!(!view.copy(0x1308, &mut chunk));
        assert_eq!(
            wide_memory.reads.borrow().get(&0x1300).copied().unwrap_or(0),
            reads_0
        );
        assert_eq!(
            wide_memory.reads.borrow().get(&0x1308).copied().unwrap_or(0),
            reads_8
        );
        assert!(view.copy(0x1304, &mut chunk));
        assert_eq!(&chunk, b"4567");
        let reads_4 = wide_memory.reads.borrow().get(&0x1304).copied().unwrap_or(0);
        assert!(view.copy(0x1304, &mut chunk));
        assert_eq!(
            wide_memory.reads.borrow().get(&0x1304).copied().unwrap_or(0),
            reads_4
        );
    }
}
