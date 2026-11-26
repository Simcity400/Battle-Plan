use crate::variable_changes::{Changes, Counts, Taken};
use engine_allocation as allocation;
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    walk::{Buffer, Outcome},
};
use engine_native::{ccountry, cgame_state, cstate};
use engine_scripts::{
    cvariables, cvariables::HEADER, cvariables::Row, cvariables::Scope, cvariables::ScopeRow,
    cvariables::ScopeWalk, cvariables::Store, cvariables::VariableCarry,
};
use std::sync::Arc;

const SCOPE_CHUNK: usize = 64;

#[derive(Clone, Debug)]
pub struct HeldScope {
    pub scope: Scope,
    pub seed: Option<u32>,
    pub entries: Vec<Row>,
    pub names: Vec<u8>,
}

impl HeldScope {
    fn from_normalized(
        scope: Scope,
        seed: Option<u32>,
        entries: &[Row],
        names: &[u8],
    ) -> Option<Arc<Self>> {
        if !entries
            .iter()
            .all(|entry| entry.name_start.checked_add(entry.name_len).is_some_and(|end| end <= names.len()))
        {
            return None;
        }
        allocation::advisory(|| {
            let entries = allocation::copy_vec(entries).ok()?;
            let names = allocation::copy_vec(names).ok()?;
            allocation::allocation_attempt().ok()?;
            Some(Arc::new(Self {
                scope,
                seed,
                entries,
                names,
            }))
        })
    }

    fn from_raw(
        scope: Scope,
        seed: Option<u32>,
        raw_entries: &[Row],
        all_names: &[u8],
    ) -> Option<Arc<Self>> {
        let mut total_bytes = 0_usize;
        for entry in raw_entries {
            let end = entry.name_start.checked_add(entry.name_len)?;
            if end > all_names.len() {
                return None;
            }
            total_bytes = total_bytes.checked_add(entry.name_len)?;
        }
        allocation::advisory(|| {
            let mut entries = allocation::vec(raw_entries.len()).ok()?;
            let mut names = allocation::vec(total_bytes).ok()?;
            for entry in raw_entries {
                let rel_start = names.len();
                names.extend_from_slice(
                    &all_names[entry.name_start..entry.name_start + entry.name_len],
                );
                entries.push(Row {
                    name_start: rel_start,
                    ..*entry
                });
            }
            allocation::allocation_attempt().ok()?;
            Some(Arc::new(Self {
                scope,
                seed,
                entries,
                names,
            }))
        })
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.entries.capacity() * size_of::<Row>()
            + self.names.capacity()
    }
}

#[derive(Debug, Default)]
struct FlatBase {
    rows: Vec<ScopeRow>,
    name_spans: Vec<(usize, usize)>,
    entries: Vec<Row>,
    names: Vec<u8>,
}

impl FlatBase {
    fn from_flat(rows: &[ScopeRow], all_entries: &[Row], all_names: &[u8]) -> Option<Arc<Self>> {
        let mut total_entries = 0_usize;
        let mut total_bytes = 0_usize;
        for row in rows {
            let end = row.entries_start.checked_add(row.entries_len)?;
            let slice = all_entries.get(row.entries_start..end)?;
            total_entries = total_entries.checked_add(slice.len())?;
            for entry in slice {
                let entry_end = entry.name_start.checked_add(entry.name_len)?;
                if entry_end > all_names.len() {
                    return None;
                }
                total_bytes = total_bytes.checked_add(entry.name_len)?;
            }
        }
        allocation::advisory(|| {
            let mut out_rows = allocation::vec(rows.len()).ok()?;
            let mut name_spans = allocation::vec(rows.len()).ok()?;
            let mut out_entries = allocation::vec(total_entries).ok()?;
            let mut out_names = allocation::vec(total_bytes).ok()?;
            for row in rows {
                let slice = &all_entries[row.entries_start..row.entries_start + row.entries_len];
                let entry_start = out_entries.len();
                let name_base = out_names.len();
                if !slice.is_empty() {
                    let first_start = slice[0].name_start;
                    let last = slice[slice.len() - 1];
                    let contiguous_end = last.name_start + last.name_len;
                    let mut contiguous = first_start <= contiguous_end
                        && contiguous_end <= all_names.len();
                    if contiguous {
                        let mut expected = first_start;
                        for entry in slice {
                            if entry.name_start != expected {
                                contiguous = false;
                                break;
                            }
                            expected += entry.name_len;
                        }
                        contiguous &= expected == contiguous_end;
                    }
                    if contiguous {
                        out_names.extend_from_slice(&all_names[first_start..contiguous_end]);
                        for entry in slice {
                            out_entries.push(Row {
                                name_start: entry.name_start - first_start,
                                ..*entry
                            });
                        }
                    } else {
                        for entry in slice {
                            let rel_start = out_names.len() - name_base;
                            out_names.extend_from_slice(
                                &all_names[entry.name_start..entry.name_start + entry.name_len],
                            );
                            out_entries.push(Row {
                                name_start: rel_start,
                                ..*entry
                            });
                        }
                    }
                }
                let name_len = out_names.len() - name_base;
                name_spans.push((name_base, name_len));
                out_rows.push(ScopeRow {
                    scope: row.scope,
                    entries_start: entry_start,
                    entries_len: slice.len(),
                    seed: row.seed,
                });
            }
            allocation::allocation_attempt().ok()?;
            Some(Arc::new(Self {
                rows: out_rows,
                name_spans,
                entries: out_entries,
                names: out_names,
            }))
        })
    }

    fn get(&self, ordinal: usize) -> Option<(Scope, Option<u32>, &[Row], &[u8])> {
        let row = self.rows.get(ordinal)?;
        let (n_start, n_len) = *self.name_spans.get(ordinal)?;
        let entries = &self.entries[row.entries_start..row.entries_start + row.entries_len];
        let names = &self.names[n_start..n_start + n_len];
        Some((row.scope, row.seed, entries, names))
    }

    fn allocated_bytes(&self) -> usize {
        size_of::<Self>()
            + self.rows.capacity() * size_of::<ScopeRow>()
            + self.name_spans.capacity() * size_of::<(usize, usize)>()
            + self.entries.capacity() * size_of::<Row>()
            + self.names.capacity()
    }
}

#[derive(Clone, Debug)]
enum ScopeSlot {
    Base(usize),
    Override(Arc<HeldScope>),
}

#[derive(Clone, Debug)]
enum ScopeChunk {
    Base(usize, usize),
    Owned(Arc<Vec<ScopeSlot>>),
}

#[derive(Clone, Debug, Default)]
pub struct VariableSnapshot {
    base: Option<Arc<FlatBase>>,
    chunks: Vec<ScopeChunk>,
    len: usize,
    entries_len: usize,
    bytes_len: usize,
    override_count: usize,
}

impl VariableSnapshot {
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn entries_len(&self) -> usize {
        self.entries_len
    }

    pub fn bytes_len(&self) -> usize {
        self.bytes_len
    }

    fn can_shallow_carry(&self) -> bool {
        self.base.is_some() && self.override_count <= (self.len / 4).max(512)
    }

    fn slot(&self, ordinal: usize) -> Option<ScopeSlot> {
        let chunk = self.chunks.get(ordinal / SCOPE_CHUNK)?;
        let slot = ordinal % SCOPE_CHUNK;
        match chunk {
            ScopeChunk::Base(start, count) => {
                (slot < *count).then_some(ScopeSlot::Base(start + slot))
            }
            ScopeChunk::Owned(slots) => slots.get(slot).cloned(),
        }
    }

    fn base_index(&self, ordinal: usize) -> Option<usize> {
        let chunk = self.chunks.get(ordinal / SCOPE_CHUNK)?;
        let slot = ordinal % SCOPE_CHUNK;
        match chunk {
            ScopeChunk::Base(start, count) => (slot < *count).then_some(start + slot),
            ScopeChunk::Owned(slots) => match slots.get(slot)? {
                ScopeSlot::Base(index) => Some(*index),
                ScopeSlot::Override(_) => None,
            },
        }
    }

    pub fn get(&self, ordinal: usize) -> Option<(Scope, Option<u32>, &[Row], &[u8])> {
        let chunk = self.chunks.get(ordinal / SCOPE_CHUNK)?;
        let slot = ordinal % SCOPE_CHUNK;
        match chunk {
            ScopeChunk::Base(start, count) => {
                if slot >= *count {
                    return None;
                }
                self.base.as_ref()?.get(start + slot)
            }
            ScopeChunk::Owned(slots) => match slots.get(slot)? {
                ScopeSlot::Base(base_index) => self.base.as_ref()?.get(*base_index),
                ScopeSlot::Override(held) => {
                    Some((held.scope, held.seed, &held.entries, &held.names))
                }
            },
        }
    }

    fn clear(&mut self) {
        self.base = None;
        self.chunks.clear();
        self.len = 0;
        self.entries_len = 0;
        self.bytes_len = 0;
        self.override_count = 0;
    }

    fn shallow_clone(&self) -> Option<Self> {
        let chunks = allocation::advisory(|| {
            let mut out = allocation::vec(self.chunks.len()).ok()?;
            out.extend(self.chunks.iter().cloned());
            Some(out)
        })?;
        Some(Self {
            base: self.base.as_ref().map(Arc::clone),
            chunks,
            len: self.len,
            entries_len: self.entries_len,
            bytes_len: self.bytes_len,
            override_count: self.override_count,
        })
    }

    fn from_base(base: Arc<FlatBase>) -> Option<Self> {
        let len = base.rows.len();
        let entries_len = base.entries.len();
        let bytes_len = base.names.len();
        let chunk_count = len.div_ceil(SCOPE_CHUNK);
        let chunks = allocation::advisory(|| {
            let mut chunks = allocation::vec(chunk_count).ok()?;
            let mut start = 0;
            while start < len {
                let take = (len - start).min(SCOPE_CHUNK);
                chunks.push(ScopeChunk::Base(start, take));
                start += take;
            }
            Some(chunks)
        })?;
        Some(Self {
            base: Some(base),
            chunks,
            len,
            entries_len,
            bytes_len,
            override_count: 0,
        })
    }

    fn rebuild_from_walk(
        &self,
        rows: &[ScopeRow],
        plans: &[Plan],
        entries: &[Row],
        names: &[u8],
    ) -> Option<Self> {
        let base = self.base.as_ref()?;
        if rows.len() != plans.len() {
            return None;
        }
        let len = rows.len();
        let chunk_count = len.div_ceil(SCOPE_CHUNK);
        let mut chunks = allocation::advisory(|| allocation::vec(chunk_count).ok())?;
        let mut entries_len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut override_count = 0_usize;
        let mut chunk_start = 0_usize;
        while chunk_start < len {
            let count = (len - chunk_start).min(SCOPE_CHUNK);
            let contiguous_base = plans[chunk_start]
                .carried
                .then(|| plans[chunk_start].held.and_then(|h| self.base_index(h as usize)))
                .flatten()
                .filter(|&base_start| {
                    base_start + count <= base.rows.len()
                        && (0..count).all(|offset| {
                            let j = chunk_start + offset;
                            let plan = &plans[j];
                            plan.carried
                                && plan.held.and_then(|h| self.base_index(h as usize))
                                    == Some(base_start + offset)
                                && base.rows[base_start + offset].scope == rows[j].scope
                                && base.rows[base_start + offset].seed == rows[j].seed
                        })
                });
            if let Some(base_start) = contiguous_base {
                for offset in 0..count {
                    let b = base_start + offset;
                    entries_len += base.rows[b].entries_len;
                    bytes_len += base.name_spans[b].1;
                }
                chunks.push(ScopeChunk::Base(base_start, count));
            } else {
                let mut slots = allocation::advisory(|| allocation::vec(count).ok())?;
                for offset in 0..count {
                    let j = chunk_start + offset;
                    let row = &rows[j];
                    let plan = &plans[j];
                    if plan.carried {
                        let old_idx = plan.held? as usize;
                        let (old_scope, old_seed, old_entries, old_names) = self.get(old_idx)?;
                        if old_scope == row.scope && old_seed == row.seed {
                            let slot = self.slot(old_idx)?;
                            if matches!(slot, ScopeSlot::Override(_)) {
                                override_count += 1;
                            }
                            entries_len += old_entries.len();
                            bytes_len += old_names.len();
                            slots.push(slot);
                        } else {
                            let held =
                                HeldScope::from_normalized(row.scope, row.seed, old_entries, old_names)?;
                            override_count += 1;
                            entries_len += held.entries.len();
                            bytes_len += held.names.len();
                            slots.push(ScopeSlot::Override(held));
                        }
                    } else {
                        let end = row.entries_start.checked_add(row.entries_len)?;
                        let raw_entries = entries.get(row.entries_start..end)?;
                        let held = HeldScope::from_raw(row.scope, row.seed, raw_entries, names)?;
                        override_count += 1;
                        entries_len += held.entries.len();
                        bytes_len += held.names.len();
                        slots.push(ScopeSlot::Override(held));
                    }
                }
                let arc = allocation::advisory(|| {
                    allocation::allocation_attempt().ok()?;
                    Some(Arc::new(slots))
                })?;
                chunks.push(ScopeChunk::Owned(arc));
            }
            chunk_start += count;
        }
        Some(Self {
            base: Some(Arc::clone(base)),
            chunks,
            len,
            entries_len,
            bytes_len,
            override_count,
        })
    }

    fn replace(&mut self, ordinal: usize, scope: Arc<HeldScope>) -> Option<()> {
        let (old_entries, old_bytes) = {
            let (_, _, entries, names) = self.get(ordinal)?;
            (entries.len(), names.len())
        };
        let new_entries = scope.entries.len();
        let new_bytes = scope.names.len();
        let chunk_ref = self.chunks.get_mut(ordinal / SCOPE_CHUNK)?;
        let slot = ordinal % SCOPE_CHUNK;
        let added_override;
        match chunk_ref {
            ScopeChunk::Base(start, count) => {
                if slot >= *count {
                    return None;
                }
                let base_start = *start;
                let base_count = *count;
                let next = allocation::advisory(|| {
                    let mut slots = allocation::vec(base_count).ok()?;
                    for offset in 0..base_count {
                        if offset == slot {
                            slots.push(ScopeSlot::Override(Arc::clone(&scope)));
                        } else {
                            slots.push(ScopeSlot::Base(base_start + offset));
                        }
                    }
                    allocation::allocation_attempt().ok()?;
                    Some(Arc::new(slots))
                })?;
                *chunk_ref = ScopeChunk::Owned(next);
                added_override = true;
            }
            ScopeChunk::Owned(chunk_arc) => {
                if slot >= chunk_arc.len() {
                    return None;
                }
                added_override = matches!(chunk_arc[slot], ScopeSlot::Base(_));
                if let Some(chunk) = Arc::get_mut(chunk_arc) {
                    chunk[slot] = ScopeSlot::Override(scope);
                } else {
                    let next = allocation::advisory(|| {
                        let mut chunk = allocation::vec(chunk_arc.len()).ok()?;
                        chunk.extend(chunk_arc.iter().cloned());
                        chunk[slot] = ScopeSlot::Override(scope);
                        allocation::allocation_attempt().ok()?;
                        Some(Arc::new(chunk))
                    })?;
                    *chunk_arc = next;
                }
            }
        }
        if added_override {
            self.override_count += 1;
        }
        self.entries_len = self.entries_len - old_entries + new_entries;
        self.bytes_len = self.bytes_len - old_bytes + new_bytes;
        Some(())
    }

    pub fn allocated_bytes_shallow(&self) -> usize {
        self.chunks.capacity() * size_of::<ScopeChunk>()
    }

    fn allocated_bytes_owned(&self) -> usize {
        let mut bytes = self.allocated_bytes_shallow();
        if let Some(base) = &self.base {
            bytes += base.allocated_bytes();
        }
        for chunk in &self.chunks {
            if let ScopeChunk::Owned(slots) = chunk {
                bytes += size_of::<Vec<ScopeSlot>>() + slots.capacity() * size_of::<ScopeSlot>();
                for slot in slots.iter() {
                    if let ScopeSlot::Override(scope) = slot {
                        bytes += scope.allocated_bytes();
                    }
                }
            }
        }
        bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    fn materialize(
        &self,
        rows: &mut Vec<ScopeRow>,
        entries: &mut Vec<Row>,
        names: &mut Vec<u8>,
    ) -> Option<ScopeWalk> {
        grow(rows, self.len)?;
        grow(entries, self.entries_len)?;
        grow(names, self.bytes_len)?;
        let mut row_at = 0;
        let mut entry_at = 0;
        let mut byte_at = 0;
        for ordinal in 0..self.len {
            let (scope, seed, scope_entries, scope_names) = self.get(ordinal)?;
            let count = scope_entries.len();
            let bytes = scope_names.len();
            rows[row_at] = ScopeRow {
                scope,
                entries_start: entry_at,
                entries_len: count,
                seed,
            };
            for (target, entry) in entries[entry_at..entry_at + count]
                .iter_mut()
                .zip(scope_entries)
            {
                *target = Row {
                    name_start: entry.name_start + byte_at,
                    ..*entry
                };
            }
            names[byte_at..byte_at + bytes].copy_from_slice(scope_names);
            row_at += 1;
            entry_at += count;
            byte_at += bytes;
        }
        Some(ScopeWalk {
            outcome: Outcome::Rows { len: row_at },
            entries_len: entry_at,
            required_entries: entry_at,
            bytes_len: byte_at,
            required_bytes: byte_at,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuietKey {
    taken: Taken,
    roster: (u64, u64),
    scope: (usize, usize),
}

#[derive(Default)]
struct Witness {
    global: usize,
    countries: Vec<(usize, u32)>,
    state_header: [u8; 16],
    states: Vec<usize>,
}

impl Witness {
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> bool {
        self.countries.clear();
        self.states.clear();
        let Some(global) = state
            .checked_add(cgame_state::GLOBAL_VARIABLES)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
        else {
            return false;
        };
        self.global = global;
        let Ok(countries) = ccountry::Reader::new(memory, base, state) else {
            return false;
        };
        for slot in 1..countries.count() {
            match countries.at(slot) {
                Ok(Some(country)) => {
                    if allocation::advisory(|| allocation::reserve(&mut self.countries, 1)).is_err()
                    {
                        return false;
                    }
                    self.countries.push((country.pointer, country.id));
                }
                Ok(None) => {}
                Err(_) => return false,
            }
        }
        let Some(header) = cstate::header(memory, state) else {
            return false;
        };
        let Some(array) = Array::from_header(header, 8).filter(|array| array.count >= 2) else {
            return false;
        };
        self.state_header = header;
        allocation::advisory(|| {
            allocation::resize_default(&mut self.states, array.count as usize - 1)
        })
        .is_ok()
            && cstate::read_slots(memory, array, &mut self.states).is_ok()
    }

    fn same(&self, other: &Self) -> bool {
        self.global == other.global
            && self.countries == other.countries
            && self.state_header == other.state_header
            && self.states == other.states
    }

    fn agrees(&self, rows: &[ScopeRow], global: Option<usize>) -> bool {
        let mut scopes = rows.iter().map(|row| row.scope);
        if scopes.next() != Some(Scope::Global) || global != Some(self.global) {
            return false;
        }
        for &(_, id) in &self.countries {
            if scopes.next() != Some(Scope::Country(id)) {
                return false;
            }
        }
        for (slot, &pointer) in self.states.iter().enumerate() {
            if pointer != 0 && scopes.next() != Some(Scope::State(slot as u32 + 1)) {
                return false;
            }
        }
        scopes.all(|scope| matches!(scope, Scope::Character(_)))
    }

    fn measure(&self) -> usize {
        self.countries.capacity() * size_of::<(usize, u32)>()
            + self.states.capacity() * size_of::<usize>()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Meta {
    container: usize,
    certificate: Option<(u64, u64)>,
    fresh: bool,
    header: [u8; HEADER],
}

#[derive(Clone, Copy, Debug)]
struct Plan {
    held: Option<u32>,
    carried: bool,
    meta: Meta,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VariableDelta {
    pub generation: Option<u64>,
    pub since: Option<u64>,
    pub same_scopes: bool,
    pub changed: Vec<u32>,
}

impl VariableDelta {
    pub fn clear(&mut self) {
        self.generation = None;
        self.since = None;
        self.same_scopes = false;
        self.changed.clear();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.changed.capacity() * size_of::<u32>()
    }
}

#[inline]
fn pointer_slot(container: usize, mask: usize) -> usize {
    let hash = (container as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    ((hash ^ (hash >> 32)) as usize) & mask
}

#[derive(Default)]
struct Held {
    snapshot: VariableSnapshot,
    meta: Vec<Meta>,
    index: Vec<(usize, u32)>,
    always: Vec<u32>,
    fresh: Vec<u32>,
    taken: Option<Taken>,
    generation: Option<u64>,
    quiet: Option<QuietKey>,
    witness: Witness,
    #[cfg(any(test, feature = "test-support"))]
    full_copies: usize,
}

impl Held {
    fn clear(&mut self) {
        self.snapshot.clear();
        self.meta.clear();
        self.index.clear();
        self.always.clear();
        self.fresh.clear();
        self.taken = None;
        self.generation = None;
        self.quiet = None;
    }

    #[inline]
    fn find(&self, container: usize) -> Option<usize> {
        if container == 0 || self.index.is_empty() {
            return None;
        }
        let mask = self.index.len() - 1;
        let mut slot = pointer_slot(container, mask);
        loop {
            let (held, index) = self.index[slot];
            if held == 0 {
                return None;
            }
            if held == container {
                return (index != u32::MAX).then_some(index as usize);
            }
            slot = (slot + 1) & mask;
        }
    }

    fn publish_from(&mut self, rows: &[ScopeRow], entries: &[Row], names: &[u8]) -> bool {
        let Some(next) =
            FlatBase::from_flat(rows, entries, names).and_then(VariableSnapshot::from_base)
        else {
            return false;
        };
        self.snapshot = next;
        #[cfg(any(test, feature = "test-support"))]
        {
            self.full_copies += 1;
        }
        true
    }

    fn publish_shallow(
        &mut self,
        rows: &[ScopeRow],
        plans: &[Plan],
        entries: &[Row],
        names: &[u8],
    ) -> Option<VariableSnapshot> {
        let next = self.snapshot.rebuild_from_walk(rows, plans, entries, names)?;
        let published = next.shallow_clone()?;
        self.snapshot = next;
        Some(published)
    }

    fn certify(&mut self, plans: &[Plan]) -> bool {
        let len = self.snapshot.len();
        let buckets = if len == 0 {
            0
        } else {
            let Some(buckets) = len
                .checked_mul(2)
                .and_then(|n| n.max(16).checked_next_power_of_two())
            else {
                return false;
            };
            buckets
        };
        let stored = allocation::advisory(|| {
            self.meta.clear();
            self.index.clear();
            self.always.clear();
            self.fresh.clear();
            allocation::reserve(&mut self.meta, len)?;
            allocation::reserve(&mut self.index, buckets)?;
            allocation::reserve(&mut self.always, len)?;
            allocation::reserve(&mut self.fresh, len)?;
            std::io::Result::Ok(())
        });
        if stored.is_err() || plans.len() != len || u32::try_from(len).is_err() || len == u32::MAX as usize {
            return false;
        }
        self.meta.extend(plans.iter().map(|plan| plan.meta));
        self.index.resize(buckets, (0, 0));
        let mask = buckets.saturating_sub(1);
        let mut had_duplicate = false;
        for (index, meta) in self.meta.iter().enumerate() {
            if meta.certificate.is_some() && meta.container != 0 && buckets != 0 {
                let mut slot = pointer_slot(meta.container, mask);
                loop {
                    let (held, existing) = self.index[slot];
                    if held == 0 {
                        self.index[slot] = (meta.container, index as u32);
                        break;
                    }
                    if held == meta.container {
                        had_duplicate = true;
                        if existing != u32::MAX {
                            self.always.push(existing);
                            self.index[slot] = (meta.container, u32::MAX);
                        }
                        self.always.push(index as u32);
                        break;
                    }
                    slot = (slot + 1) & mask;
                }
            } else {
                self.always.push(index as u32);
            }
        }
        if had_duplicate {
            self.always.sort_unstable();
        }
        for (index, meta) in self.meta.iter().enumerate() {
            if meta.fresh
                && meta.certificate.is_some()
                && meta.container != 0
                && (!had_duplicate || self.find(meta.container) == Some(index))
            {
                self.fresh.push(index as u32);
            }
        }
        true
    }

    fn measure(&self) -> usize {
        self.snapshot.allocated_bytes_owned()
            + self.meta.capacity() * size_of::<Meta>()
            + self.index.capacity() * size_of::<(usize, u32)>()
            + (self.always.capacity() + self.fresh.capacity()) * size_of::<u32>()
            + self.witness.measure()
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Hour {
    taken: Option<Taken>,
    roster: Option<(u64, u64)>,
    scope: (usize, usize),
    marks: bool,
    witness: bool,
}

impl Hour {
    fn key(&self) -> Option<QuietKey> {
        Some(QuietKey {
            taken: self.taken?,
            roster: self.roster?,
            scope: self.scope,
        })
        .filter(|_| self.witness)
    }
}

#[derive(Default)]
pub struct VariableStore {
    held: Held,
    hour: Hour,
    marked: Vec<usize>,
    witness: Witness,
    plans: Vec<Plan>,
    visits: Vec<(u32, bool)>,
    visiting: Option<(u32, bool)>,
    since: Option<u64>,
    changed: Vec<u32>,
    scratch_entries: Vec<Row>,
    scratch_names: Vec<u8>,
    pending_updates: Vec<(usize, Arc<HeldScope>)>,
    unlisted: bool,
    quiet: bool,
    shallow_carry: bool,
    renewed: bool,
    broken: bool,
    counts: Counts,
    bytes: usize,
    forgotten: bool,
}

impl VariableStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn full_copies(&self) -> usize {
        self.held.full_copies
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    fn begin(&mut self, changes: &dyn Changes) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
        }
        self.since = self.held.generation;
        self.plans.clear();
        let target_plans = self.held.meta.len();
        if self.plans.capacity() < target_plans {
            let _ = allocation::advisory(|| allocation::reserve(&mut self.plans, target_plans));
        }
        self.visits.clear();
        self.visiting = None;
        self.pending_updates.clear();
        self.clear_changed();
        self.quiet = false;
        self.shallow_carry = false;
        self.renewed = false;
        self.broken = false;
        self.counts = Counts::default();
        self.marked.clear();
        let marked = &mut self.marked;
        let mut complete = true;
        let taken = changes.take(&mut |container| {
            if allocation::advisory(|| allocation::reserve(marked, 1)).is_ok() {
                marked.push(container);
            } else {
                complete = false;
            }
        });
        self.marked.sort_unstable();
        self.marked.dedup();
        self.hour = Hour {
            taken,
            roster: changes.roster(),
            scope: (0, 0),
            marks: complete,
            witness: false,
        };
    }

    fn unmarked(&self, container: usize) -> bool {
        self.hour.marks
            && self.hour.taken.is_some()
            && self.hour.taken == self.held.taken
            && self.marked.binary_search(&container).is_err()
    }

    fn plan_visits(&mut self) -> bool {
        let Self {
            held,
            marked,
            visits,
            ..
        } = self;
        visits.clear();
        let demand = marked.len() + held.always.len() + held.fresh.len();
        if allocation::advisory(|| allocation::reserve(visits, demand)).is_err() {
            return false;
        }
        for &container in marked.iter() {
            if let Some(index) = held.find(container) {
                visits.push((index as u32, true));
            }
        }
        visits.extend(held.always.iter().map(|&index| (index, false)));
        visits.extend(held.fresh.iter().map(|&index| (index, true)));
        visits.sort_unstable();
        visits.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 |= later.1;
            }
            same
        });
        true
    }

    fn clear_changed(&mut self) {
        self.changed.clear();
        self.unlisted = false;
    }

    fn change(&mut self, index: u32) {
        if self.unlisted
            || allocation::advisory(|| allocation::reserve(&mut self.changed, 1)).is_err()
        {
            self.unlisted = true;
        } else {
            self.changed.push(index);
        }
    }

    fn delta(&mut self, rows: &[ScopeRow], delta: &mut VariableDelta) {
        if !self.quiet {
            self.clear_changed();
            self.unlisted = self.plans.len() != rows.len();
            for index in 0..self.plans.len().min(rows.len()) {
                let plan = &self.plans[index];
                let kept = plan.carried
                    && plan
                        .held
                        .and_then(|held| self.held.snapshot.get(held as usize))
                        .is_some_and(|(scope, seed, _, _)| {
                            scope == rows[index].scope && seed == rows[index].seed
                        });
                if !kept {
                    self.change(index as u32);
                }
            }
        }
        delta.same_scopes = self.quiet
            || (rows.len() == self.held.snapshot.len()
                && rows.iter().enumerate().all(|(index, row)| {
                    self.held
                        .snapshot
                        .get(index)
                        .is_some_and(|(scope, _, _, _)| row.scope == scope)
                }));
        if let Some(since) = self.since
            && !self.unlisted
            && allocation::advisory(|| allocation::reserve(&mut delta.changed, self.changed.len()))
                .is_ok()
        {
            delta.changed.extend_from_slice(&self.changed);
            delta.since = Some(since);
        }
    }

    fn restart(&mut self) {
        self.plans.clear();
        let target_plans = self.held.meta.len();
        if self.plans.capacity() < target_plans {
            let _ = allocation::advisory(|| allocation::reserve(&mut self.plans, target_plans));
        }
        self.visits.clear();
        self.visiting = None;
        self.pending_updates.clear();
        self.clear_changed();
        self.quiet = false;
        self.shallow_carry = false;
        self.renewed = false;
        self.broken = false;
        self.counts = Counts::default();
    }

    fn measure(&mut self) {
        self.bytes = self.held.measure()
            + self.witness.measure()
            + self.marked.capacity() * size_of::<usize>()
            + self.plans.capacity() * size_of::<Plan>()
            + self.visits.capacity() * size_of::<(u32, bool)>()
            + self.changed.capacity() * size_of::<u32>()
            + self.scratch_entries.capacity() * size_of::<Row>()
            + self.scratch_names.capacity()
            + self.pending_updates.capacity() * size_of::<(usize, Arc<HeldScope>)>();
    }
}

fn same_entries(left: &[Row], left_names: &[u8], right: &[Row], right_names: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    if left_names.len() == right_names.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value_raw == right.value_raw
                && left.name_len == right.name_len
                && left.name_start == right.name_start
        })
    {
        return left_names == right_names;
    }
    left.iter().zip(right).all(|(left, right)| {
        left.value_raw == right.value_raw
            && left.name_len == right.name_len
            && left_names.get(left.name_start..left.name_start + left.name_len)
                == right_names.get(right.name_start..right.name_start + right.name_len)
    })
}

#[cfg(any(test, feature = "test-support"))]
fn grow<T: Default>(values: &mut Vec<T>, len: usize) -> Option<()> {
    Buffer::Growing(values)
        .ensure(len)
        .ok()
        .filter(|&room| room)
        .map(|_| ())
}

pub struct VariableReuse<'a> {
    store: &'a mut VariableStore,
    changes: &'a dyn Changes,
}

impl<'a> VariableReuse<'a> {
    pub fn begin(store: &'a mut VariableStore, changes: &'a dyn Changes) -> Self {
        store.begin(changes);
        Self { store, changes }
    }

    pub fn enable_shallow(&mut self) -> bool {
        if self.store.held.snapshot.can_shallow_carry() {
            self.store.shallow_carry = true;
            true
        } else {
            false
        }
    }

    pub fn quiet_hour_snapshot(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Option<(ScopeWalk, VariableSnapshot)> {
        let store = &mut *self.store;
        store.hour.scope = (base, state);
        store.hour.witness = store.witness.read(memory, base, state);
        let key = store.hour.key();
        let matched = key.is_some()
            && store.held.quiet == key
            && store.hour.marks
            && store.witness.same(&store.held.witness);
        if !matched || !store.plan_visits() {
            return None;
        }
        let result = self.publish_snapshot(memory, base);
        if result.is_none() {
            self.store.restart();
        }
        result
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn quiet_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut Vec<ScopeRow>,
        entries: &mut Vec<Row>,
        names: &mut Vec<u8>,
    ) -> Option<ScopeWalk> {
        let (_, snapshot) = self.quiet_hour_snapshot(memory, base, state)?;
        let walk = snapshot.materialize(rows, entries, names);
        if walk.is_none() {
            self.store.restart();
        }
        walk
    }

    fn publish_snapshot(
        &mut self,
        memory: &impl Memory,
        base: usize,
    ) -> Option<(ScopeWalk, VariableSnapshot)> {
        let mut scratch_entries = std::mem::take(&mut self.store.scratch_entries);
        let mut scratch_names = std::mem::take(&mut self.store.scratch_names);
        let mut pending_updates = std::mem::take(&mut self.store.pending_updates);
        pending_updates.clear();

        let visited_ok = (|| -> Option<()> {
            let total = self.store.visits.len();
            let mut containers = [0_usize; cvariables::HEAD_BATCH];
            let mut heads = [cvariables::Head::default(); cvariables::HEAD_BATCH];
            let mut start = 0_usize;
            while start < total {
                let batch = (total - start).min(cvariables::HEAD_BATCH);
                for j in 0..batch {
                    let (index, _) = self.store.visits[start + j];
                    containers[j] = self.store.held.meta[index as usize].container;
                }
                cvariables::read_heads(memory, &containers[..batch], &mut heads[..batch]);
                for j in 0..batch {
                    let visit = start + j;
                    let (index, marked) = self.store.visits[visit];
                    let at = index as usize;
                    let (scope, _, _, _) = self.store.held.snapshot.get(at)?;
                    let container = containers[j];
                    self.store.visiting = Some((index, marked));
                    let read = cvariables::read_store_carried_with_head(
                        memory,
                        base,
                        scope,
                        container,
                        heads[j],
                        Buffer::Growing(&mut scratch_entries),
                        Buffer::Growing(&mut scratch_names),
                        &mut *self,
                    );
                    self.store.visiting = None;
                    let (walk, seed) = read.ok()?;
                    let Outcome::Rows { len: count } = walk.outcome else {
                        return None;
                    };
                    if self.store.plans.len() != visit + 1 {
                        return None;
                    }
                    let (_, old_seed, old_entries, old_names) = self.store.held.snapshot.get(at)?;
                    let same = old_seed == seed
                        && (self.store.plans[visit].carried
                            || scratch_entries
                                .get(..count)
                                .zip(scratch_names.get(..walk.bytes_len))
                                .is_some_and(|(entries, names)| {
                                    same_entries(entries, names, old_entries, old_names)
                                }));
                    if !same {
                        let entries_slice = scratch_entries.get(..count)?;
                        let names_slice = scratch_names.get(..walk.bytes_len)?;
                        let next =
                            HeldScope::from_normalized(scope, seed, entries_slice, names_slice)?;
                        allocation::advisory(|| allocation::reserve(&mut pending_updates, 1))
                            .ok()?;
                        pending_updates.push((at, next));
                        self.store.renewed = true;
                        self.store.change(index);
                    }
                }
                start += batch;
            }
            Some(())
        })();

        self.store.scratch_entries = scratch_entries;
        self.store.scratch_names = scratch_names;

        if visited_ok.is_none() || self.store.broken {
            pending_updates.clear();
            self.store.pending_updates = pending_updates;
            return None;
        }

        let store = &mut *self.store;
        let mut applied = true;
        for (at, next) in pending_updates.drain(..) {
            if store.held.snapshot.replace(at, next).is_none() {
                applied = false;
                break;
            }
        }
        store.pending_updates = pending_updates;
        if !applied {
            store.held.clear();
            return None;
        }
        let Some(snapshot) = store.held.snapshot.shallow_clone() else {
            store.held.clear();
            return None;
        };
        store.quiet = true;
        store.counts.quiet = true;
        store.counts.visited = store.visits.len() as u64;
        store.counts.reused = snapshot.len() as u64 - store.counts.read;
        store.counts.reused_rows = snapshot.entries_len() as u64 - store.counts.read_rows;
        let walk = ScopeWalk {
            outcome: Outcome::Rows {
                len: snapshot.len(),
            },
            entries_len: snapshot.entries_len(),
            required_entries: snapshot.entries_len(),
            bytes_len: snapshot.bytes_len(),
            required_bytes: snapshot.bytes_len(),
        };
        Some((walk, snapshot))
    }

    pub fn finish_quiet(self, delta: &mut VariableDelta) {
        let store = self.store;
        delta.clear();
        store.delta(&[], delta);
        let held = !store.broken && store.plans.len() == store.visits.len();
        if held {
            for (plan, &(index, _)) in store.plans.iter().zip(&store.visits) {
                store.held.meta[index as usize] = plan.meta;
            }
            store.held.fresh.clear();
            store.held.generation = Some(collector_projection::projection::generation());
            delta.generation = store.held.generation;
        } else {
            store.held.clear();
        }
        let scopes = store.held.snapshot.len();
        store.plans.clear();
        store.visits.clear();
        store.counts.quiet = store.quiet;
        store.measure();
        crate::variable_changes::record(&store.counts, held.then_some(scopes));
    }

    pub fn finish_shallow(
        self,
        complete: bool,
        rows: &[ScopeRow],
        entries: &[Row],
        names: &[u8],
        delta: &mut VariableDelta,
    ) -> Option<(ScopeWalk, VariableSnapshot)> {
        let store = self.store;
        delta.clear();
        if complete {
            store.delta(rows, delta);
        }
        let holds = complete && !store.broken && store.plans.len() == rows.len();
        let published = if holds {
            store.held.publish_shallow(rows, &store.plans, entries, names)
        } else {
            None
        };
        let result = if let Some(snapshot) = published
            && store.held.certify(&store.plans)
        {
            store.held.generation = Some(collector_projection::projection::generation());
            delta.generation = store.held.generation;
            store.held.taken = store.hour.taken;
            let global = store.plans.first().map(|plan| plan.meta.container);
            let key = store
                .hour
                .key()
                .filter(|_| store.witness.agrees(rows, global));
            store.held.quiet = key;
            if key.is_some() {
                std::mem::swap(&mut store.held.witness, &mut store.witness);
            }
            let walk = ScopeWalk {
                outcome: Outcome::Rows {
                    len: snapshot.len(),
                },
                entries_len: snapshot.entries_len(),
                required_entries: snapshot.entries_len(),
                bytes_len: snapshot.bytes_len(),
                required_bytes: snapshot.bytes_len(),
            };
            Some((walk, snapshot))
        } else {
            store.held.clear();
            None
        };
        store.plans.clear();
        store.visits.clear();
        store.shallow_carry = false;
        store.counts.quiet = store.quiet;
        store.measure();
        crate::variable_changes::record(
            &store.counts,
            result.as_ref().map(|(_, snap)| snap.len()),
        );
        result
    }

    pub fn finish(
        self,
        complete: bool,
        rows: &[ScopeRow],
        entries: &[Row],
        names: &[u8],
        delta: &mut VariableDelta,
    ) {
        let store = self.store;
        delta.clear();
        if complete {
            store.delta(rows, delta);
        }
        let holds = complete && !store.broken;
        if store.quiet {
            let held = holds
                && store.plans.len() == store.visits.len()
                && rows.len() == store.held.snapshot.len();
            if held {
                for (plan, &(index, _)) in store.plans.iter().zip(&store.visits) {
                    store.held.meta[index as usize] = plan.meta;
                }
                store.held.fresh.clear();
                store.held.generation = Some(collector_projection::projection::generation());
                delta.generation = store.held.generation;
            } else {
                store.held.clear();
            }
        } else {
            let held = holds
                && store.plans.len() == rows.len()
                && store.held.publish_from(rows, entries, names)
                && store.held.certify(&store.plans);
            if held {
                store.held.generation = Some(collector_projection::projection::generation());
                delta.generation = store.held.generation;
                store.held.taken = store.hour.taken;
                let global = store.plans.first().map(|plan| plan.meta.container);
                let key = store
                    .hour
                    .key()
                    .filter(|_| store.witness.agrees(rows, global));
                store.held.quiet = key;
                if key.is_some() {
                    std::mem::swap(&mut store.held.witness, &mut store.witness);
                }
            } else {
                store.held.clear();
            }
        }
        store.plans.clear();
        store.visits.clear();
        store.counts.quiet = store.quiet;
        store.measure();
        crate::variable_changes::record(&store.counts, complete.then_some(rows.len()));
    }
}

impl VariableCarry for VariableReuse<'_> {
    fn plan(&mut self, store: &Store) -> (usize, Option<(usize, usize)>) {
        let changes = self.changes;
        let state = &mut *self.store;
        let registration = changes.register(store.container);
        let certificate = registration.map(|(certificate, _)| certificate);
        let (candidate, unmarked) = match state.visiting {
            Some((index, marked)) => (Some(index as usize), !marked),
            None => (
                state.held.find(store.container),
                state.unmarked(store.container),
            ),
        };
        let held = &state.held;
        let current = candidate.filter(|&index| {
            let meta = &held.meta[index];
            let Some((row_scope, row_seed, _, _)) = held.snapshot.get(index) else {
                return false;
            };
            certificate.is_some()
                && meta.certificate == certificate
                && meta.container == store.container
                && meta.header == store.header
                && row_scope == store.scope
                && (!unmarked || meta.fresh || row_seed == store.seed)
        });
        let carry = current.and_then(|index| {
            if state.shallow_carry && state.visiting.is_none() {
                Some((0, 0))
            } else {
                let (_, _, row_entries, row_names) = held.snapshot.get(index)?;
                Some((row_entries.len(), row_names.len()))
            }
        });
        let plans = &mut state.plans;
        let token = plans.len();
        if plans.len() == plans.capacity()
            && allocation::advisory(|| allocation::reserve(plans, 1)).is_err()
        {
            state.broken = true;
            return (usize::MAX, None);
        }
        plans.push(Plan {
            held: current.map(|index| index as u32),
            carried: false,
            meta: Meta {
                container: store.container,
                certificate,
                fresh: registration.is_some_and(|(_, fresh)| fresh),
                header: store.header,
            },
        });
        (token, carry)
    }

    fn carry(&mut self, token: usize, entries: &mut [Row], names: &mut [u8]) {
        let state = &mut *self.store;
        let Some(plan) = state.plans.get_mut(token) else {
            return;
        };
        let Some(index) = plan.held else {
            return;
        };
        let Some((_, _, held_entries, held_names)) = state.held.snapshot.get(index as usize) else {
            return;
        };
        plan.carried = true;
        if !(state.shallow_carry && state.visiting.is_none()) {
            entries.copy_from_slice(held_entries);
            names.copy_from_slice(held_names);
        }
        state.counts.reused += 1;
        state.counts.reused_rows += held_entries.len() as u64;
    }

    fn read(&mut self, token: usize, _: Outcome, entries: &[Row], _: &[u8]) {
        let state = &mut *self.store;
        state.counts.read += 1;
        state.counts.read_rows += entries.len() as u64;
        if token >= state.plans.len() {
            state.broken = true;
        }
    }
}
