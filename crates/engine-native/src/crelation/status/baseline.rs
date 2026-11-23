
use super::{
    Array, Counters, MARK_BYTES, MARK_FIELDS, MARK_LAYOUT, OpinionRow, POINTER_LAYOUT, Resolver,
    ccountry,
};
use engine_native_memory::{Memory, memory::RecordSources};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub status: usize,
    pub diplomacy: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Observation<'a> {
    pub token: (u64, u64),
    pub marks: Option<&'a [Mark]>,
}

#[derive(Default)]
pub struct Baseline {
    scope: (usize, usize),
    token: Option<(u64, u64)>,
    census: u32,
    owners: Vec<Identity>,
    rows: Vec<Vec<Row>>,
    pending: Vec<Vec<usize>>,
    demand: Vec<[usize; 2]>,
    by_diplomacy: Vec<(usize, u32)>,
    recorded: usize,
    take: Vec<(u32, usize)>,
    updates: Vec<Row>,
    updates_len: usize,
    cursor: usize,
    owner: u32,
    visiting: Option<Identity>,
    scratch: Vec<Row>,
    scratch_len: usize,
    scratch_count: usize,
    recording: bool,
    next_census: u32,
    required: [usize; 3],
    counts: Counts,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    pub country: usize,
    pub id: u32,
    pub diplomacy: usize,
    pub matrix: Array,
}

impl Identity {
    fn valid(&self) -> bool {
        self.country != 0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Row {
    owner: u32,
    slot: u32,
    target: u32,
    opinion: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub full_walks: u64,
    pub incremental_walks: u64,
    pub reused_owners: u64,
    pub read_owners: u64,
    pub marks: u64,
    pub refused_observations: u64,
    pub invalidated_owners: u64,
}

impl Baseline {
    pub fn reserve_marks(&mut self, marks: usize) -> std::io::Result<()> {
        self.required[2] = self.required[2].max(marks);
        self.grow()
    }

    pub fn clear(&mut self) {
        self.token = None;
        self.forget();
    }

    pub fn token(&self) -> Option<(u64, u64)> {
        self.token
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn owner(&self, diplomacy: usize) -> Option<u32> {
        let recorded = &self.by_diplomacy[..self.recorded];
        let at = recorded
            .binary_search_by_key(&diplomacy, |&(diplomacy, _)| diplomacy)
            .ok()?;
        let owner = self.owners.get(recorded[at].1 as usize)?;
        owner.valid().then_some(owner.id)
    }

    pub fn rows(&self) -> usize {
        self.rows.iter().map(Vec::len).sum()
    }

    #[cfg(test)]
    pub fn valid_owners(&self) -> usize {
        self.owners.iter().filter(|owner| owner.valid()).count()
    }

    pub fn needs_growth(&self) -> bool {
        let [slots, scratch, take] = self.required;
        slots > self.owners.len()
            || scratch > self.scratch.len()
            || take > self.take.len()
            || self.demand.iter().any(|demand| *demand != [0, 0])
    }

    pub fn grow(&mut self) -> std::io::Result<()> {
        fn fit<T: Default>(vector: &mut Vec<T>, required: usize) -> std::io::Result<()> {
            if required > vector.len() {
                let len = required;
                engine_allocation::reserve_exact(vector, len - vector.len())?;
                vector.resize_with(len, T::default);
            }
            Ok(())
        }
        fn reserve<T>(vector: &mut Vec<T>, required: usize) -> std::io::Result<()> {
            if required > vector.capacity() {
                let len = required;
                engine_allocation::reserve_exact(vector, len - vector.len())?;
            }
            Ok(())
        }
        let [slots, scratch, take] = self.required;
        if slots > self.owners.len() {
            let additional = slots - self.owners.len();
            engine_allocation::reserve_exact(&mut self.owners, additional)?;
            let additional = slots - self.rows.len();
            engine_allocation::reserve_exact(&mut self.rows, additional)?;
            let additional = slots - self.pending.len();
            engine_allocation::reserve_exact(&mut self.pending, additional)?;
            let additional = slots - self.demand.len();
            engine_allocation::reserve_exact(&mut self.demand, additional)?;
            let additional = slots - self.by_diplomacy.len();
            engine_allocation::reserve_exact(&mut self.by_diplomacy, additional)?;
            self.owners.resize(slots, Identity::default());
            self.rows.resize_with(slots, Vec::new);
            self.pending.resize_with(slots, Vec::new);
            self.demand.resize(slots, [0, 0]);
            self.by_diplomacy.resize(slots, (0, 0));
        }
        for (slot, demand) in self.demand.iter_mut().enumerate() {
            if let Some(rows) = self.rows.get_mut(slot) {
                reserve(rows, demand[0])?;
            }
            if let Some(pending) = self.pending.get_mut(slot) {
                reserve(pending, demand[1])?;
            }
            *demand = [0, 0];
        }
        fit(&mut self.scratch, scratch)?;
        fit(&mut self.take, take)?;
        fit(&mut self.updates, take)?;
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn shorten_updates(&mut self, len: usize) {
        self.updates.truncate(len);
        self.updates.shrink_to_fit();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.owners.capacity() * size_of::<Identity>()
            + self
                .rows
                .iter()
                .map(|rows| rows.capacity() * size_of::<Row>())
                .sum::<usize>()
            + self
                .pending
                .iter()
                .map(|pending| pending.capacity() * size_of::<usize>())
                .sum::<usize>()
            + self.rows.capacity() * size_of::<Vec<Row>>()
            + self.pending.capacity() * size_of::<Vec<usize>>()
            + self.demand.capacity() * size_of::<[usize; 2]>()
            + self.by_diplomacy.capacity() * size_of::<(usize, u32)>()
            + self.take.capacity() * size_of::<(u32, usize)>()
            + (self.updates.capacity() + self.scratch.capacity()) * size_of::<Row>()
    }

    fn forget(&mut self) {
        for owner in &mut self.owners {
            *owner = Identity::default();
        }
        for rows in &mut self.rows {
            rows.clear();
        }
        for pending in &mut self.pending {
            pending.clear();
        }
        self.recorded = 0;
    }

    fn invalidate(&mut self, slot: u32) {
        let slot = slot as usize;
        if self.owners.get(slot).is_some_and(Identity::valid) {
            self.counts.invalidated_owners += 1;
        }
        if let Some(owner) = self.owners.get_mut(slot) {
            *owner = Identity::default();
        }
        if let Some(rows) = self.rows.get_mut(slot) {
            rows.clear();
        }
        if let Some(pending) = self.pending.get_mut(slot) {
            pending.clear();
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        memory: &impl Memory,
        scope: (usize, usize),
        countries: &ccountry::Reader<'_, impl Memory>,
        status_vtable: u64,
        resolved: &mut Resolver,
        observed: Option<Observation<'_>>,
        select: &impl Fn(u32) -> bool,
    ) -> bool {
        self.cursor = 0;
        self.updates_len = 0;
        let token = self.token.take();
        let census = countries.count();
        let storage = self.owners.len() >= census as usize;
        let Some(observed) = observed else {
            self.forget();
            self.counts.full_walks += 1;
            return false;
        };
        self.counts.incremental_walks += 1;
        let continues = token == Some(observed.token)
            && self.scope == scope
            && self.census == census
            && storage;
        let Some(marks) = observed.marks.filter(|_| continues) else {
            if continues {
                self.counts.refused_observations += 1;
            }
            self.forget();
            return true;
        };
        for mark in marks {
            let recorded = &self.by_diplomacy[..self.recorded];
            let Ok(at) =
                recorded.binary_search_by_key(&mark.diplomacy, |&(diplomacy, _)| diplomacy)
            else {
                continue;
            };
            let slot = recorded[at].1;
            if !self.owners[slot as usize].valid() {
                if self.demand[slot as usize][1] != 0 {
                    self.demand[slot as usize][1] = self.demand[slot as usize][1].saturating_add(1);
                }
                continue;
            }
            let pending = &mut self.pending[slot as usize];
            if pending.len() == pending.capacity() {
                self.demand[slot as usize][1] = pending.len().saturating_add(1);
                self.invalidate(slot);
                continue;
            }
            pending.push(mark.status);
        }
        let mut taken = 0;
        let mut needed = 0;
        for slot in 1..census.min(self.owners.len() as u32) {
            let owner = self.owners[slot as usize];
            if !owner.valid() || !select(owner.id) {
                continue;
            }
            let pending = &self.pending[slot as usize];
            needed += pending.len();
            for &status in pending {
                if taken < self.take.len() {
                    self.take[taken] = (slot, status);
                }
                taken += 1;
            }
        }
        self.required[2] = self.required[2].max(needed);
        if taken > self.take.len().min(self.updates.len()) {
            for slot in 1..census.min(self.owners.len() as u32) {
                let owner = self.owners[slot as usize];
                if owner.valid() && select(owner.id) {
                    self.invalidate(slot);
                }
            }
            return true;
        }
        if self
            .read_marks(memory, countries, status_vtable, resolved, taken)
            .is_none()
        {
            self.counts.refused_observations += 1;
            self.forget();
            return true;
        }
        for slot in 1..census.min(self.owners.len() as u32) {
            let owner = self.owners[slot as usize];
            if owner.valid() && select(owner.id) {
                self.pending[slot as usize].clear();
            }
        }
        let updates = &mut self.updates[..self.updates_len];
        updates.sort_unstable_by_key(|row| (row.owner, row.slot));
        let mut kept = 0;
        for index in 0..updates.len() {
            if kept == 0
                || (updates[kept - 1].owner, updates[kept - 1].slot)
                    != (updates[index].owner, updates[index].slot)
            {
                updates[kept] = updates[index];
                kept += 1;
            }
        }
        self.updates_len = kept;
        self.counts.marks += taken as u64;
        true
    }

    fn read_marks(
        &mut self,
        memory: &impl Memory,
        countries: &ccountry::Reader<'_, impl Memory>,
        status_vtable: u64,
        resolved: &mut Resolver,
        taken: usize,
    ) -> Option<()> {
        let mut records = [0_u8; super::BATCH * MARK_BYTES];
        let mut statuses = [0_usize; super::BATCH];
        let mut addresses = [0_usize; super::BATCH];
        let mut entries = [0_u8; super::BATCH * 8];
        let mut start = 0;
        while start < taken {
            let len = (taken - start).min(super::BATCH);
            let chunk = &self.take[start..start + len];
            for (status, &(_, marked)) in statuses.iter_mut().zip(chunk) {
                *status = marked;
            }
            let records = &mut records[..len * MARK_BYTES];
            if memory.copy_records(
                RecordSources::Pointers(&statuses[..len]),
                &MARK_LAYOUT,
                records,
            ) < len * MARK_FIELDS.len()
            {
                return None;
            }
            for (index, record) in records.chunks_exact(MARK_BYTES).enumerate() {
                let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                let quad = |at: usize| u64::from_le_bytes(record[at..at + 8].try_into().unwrap());
                let owner = chunk[index].0;
                let identity = self.owners[owner as usize];
                if quad(0) != status_vtable || quad(8) != identity.diplomacy as u64 {
                    return None;
                }
                let target = word(16);
                let slot = resolved.resolve(countries, target).ok()?.canonical_slot;
                if slot == 0 || slot >= identity.matrix.count {
                    return None;
                }
                addresses[index] = identity.matrix.element(slot, 8)?;
                self.updates[self.updates_len] = Row {
                    owner,
                    slot,
                    target,
                    opinion: word(20) as i32,
                };
                self.updates_len += 1;
            }
            let entries = &mut entries[..len * 8];
            if memory.copy_records(
                RecordSources::Pointers(&addresses[..len]),
                &POINTER_LAYOUT,
                entries,
            ) < len
            {
                return None;
            }
            for (&status, entry) in statuses[..len].iter().zip(entries.chunks_exact(8)) {
                if u64::from_le_bytes(entry.try_into().unwrap()) != status as u64 {
                    return None;
                }
            }
            start += len;
        }
        Some(())
    }

    pub(super) fn begin(&mut self, scope: (usize, usize), recording: bool, census: u32) {
        if self.scope != scope {
            self.token = None;
            self.forget();
            self.scope = scope;
        }
        self.next_census = census;
        self.required[0] = census as usize;
        self.recording = recording && self.owners.len() >= census as usize;
        self.visiting = None;
    }

    pub(super) fn unchanged(&self, slot: u32, identity: Identity) -> bool {
        self.owners
            .get(slot as usize)
            .is_some_and(|owner| owner.valid() && *owner == identity)
    }

    pub(super) fn empty(&mut self, slot: u32) {
        if self.recording {
            self.invalidate(slot);
        }
    }

    pub(super) fn carry(&mut self, slot: u32, country: usize) {
        if self.recording
            && self
                .owners
                .get(slot as usize)
                .is_some_and(|owner| owner.valid() && owner.country != country)
        {
            self.invalidate(slot);
        }
    }

    pub(super) fn visit(&mut self, slot: u32, identity: Identity) {
        self.owner = slot;
        self.visiting = Some(identity);
        self.required[1] = self.required[1].max(identity.matrix.count as usize);
        if let Some(demand) = self.demand.get_mut(slot as usize)
            && self.pending[slot as usize].capacity() < identity.matrix.count as usize
        {
            demand[1] = demand[1].max(identity.matrix.count as usize);
        }
        self.scratch_len = 0;
        self.scratch_count = 0;
    }

    pub(super) fn record(&mut self, slot: u32, target: u32, opinion: i32) {
        self.scratch_count += 1;
        if !self.recording {
            return;
        }
        if self.scratch_len < self.scratch.len() {
            self.scratch[self.scratch_len] = Row {
                owner: self.owner,
                slot,
                target,
                opinion,
            };
            self.scratch_len += 1;
        }
    }

    pub(super) fn replay(
        &mut self,
        slot: u32,
        owner_country: u32,
        counters: &mut Counters,
        opinions: &mut engine_native_memory::walk::Buffer<'_, OpinionRow>,
    ) -> Result<(), &'static str> {
        self.counts.reused_owners += 1;
        while self.cursor < self.updates_len && self.updates[self.cursor].owner < slot {
            self.cursor += 1;
        }
        let mut old = 0;
        let mut new = self.cursor;
        loop {
            let recorded = self.rows[slot as usize].get(old).copied();
            let marked = (new < self.updates_len && self.updates[new].owner == slot)
                .then(|| self.updates[new]);
            let row = match (recorded, marked) {
                (None, None) => break,
                (Some(recorded), Some(marked)) if recorded.slot == marked.slot => {
                    old += 1;
                    new += 1;
                    marked
                }
                (Some(recorded), Some(marked)) if recorded.slot < marked.slot => {
                    old += 1;
                    recorded
                }
                (Some(recorded), None) => {
                    old += 1;
                    recorded
                }
                (_, Some(marked)) => {
                    new += 1;
                    marked
                }
            };
            if row.opinion != 0 {
                self.record(row.slot, row.target, row.opinion);
                counters.push_opinion(
                    opinions,
                    OpinionRow {
                        owner_country,
                        target_country: row.target,
                        opinion: row.opinion,
                    },
                )?;
            }
        }
        self.cursor = new;
        Ok(())
    }

    pub(super) fn end_owner(&mut self, read_in_full: bool) {
        let Some(identity) = self.visiting.take() else {
            return;
        };
        if !self.recording {
            return;
        }
        if read_in_full {
            self.counts.read_owners += 1;
        }
        let slot = self.owner as usize;
        self.required[1] = self.required[1].max(self.scratch_count);
        let rows = &mut self.rows[slot];
        if self.scratch_count > self.scratch_len || self.scratch_len > rows.capacity() {
            self.demand[slot][0] = self.scratch_count;
            self.invalidate(self.owner);
            return;
        }
        rows.clear();
        rows.extend_from_slice(&self.scratch[..self.scratch_len]);
        self.owners[slot] = identity;
    }

    pub(super) fn finish(&mut self, complete: bool, observed: Option<(u64, u64)>) {
        let kept = complete && self.recording && observed.is_some();
        self.recording = false;
        self.visiting = None;
        if !kept {
            self.token = None;
            self.forget();
            return;
        }
        self.census = self.next_census;
        self.token = observed;
        let mut len = 0;
        for slot in 1..(self.census as usize).min(self.owners.len()) {
            let owner = self.owners[slot];
            if owner.valid() {
                self.by_diplomacy[len] = (owner.diplomacy, slot as u32);
                len += 1;
            }
        }
        self.by_diplomacy[..len].sort_unstable();
        self.recorded = len;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_merges_marks_into_recorded_rows_in_slot_order() {
        let row = |owner, slot, target, opinion| Row {
            owner,
            slot,
            target,
            opinion,
        };
        let mut baseline = Baseline {
            rows: vec![
                vec![],
                vec![row(1, 2, 2, 5), row(1, 4, 4, -3)],
                vec![row(2, 1, 1, 7)],
            ],
            updates: vec![
                row(1, 2, 2, 0),
                row(1, 3, 30, 9),
            ],
            updates_len: 2,
            ..Baseline::default()
        };
        let mut counters = Counters::default();
        let mut output = vec![OpinionRow::default(); 4];
        baseline
            .replay(
                1,
                100,
                &mut counters,
                &mut engine_native_memory::walk::Buffer::Growing(&mut output),
            )
            .unwrap();
        baseline
            .replay(
                2,
                200,
                &mut counters,
                &mut engine_native_memory::walk::Buffer::Growing(&mut output),
            )
            .unwrap();
        let emitted = |owner_country, target_country, opinion| OpinionRow {
            owner_country,
            target_country,
            opinion,
        };
        assert_eq!(
            output[..3],
            [emitted(100, 30, 9), emitted(100, 4, -3), emitted(200, 1, 7)]
        );
        assert_eq!(counters.opinions, 3);
    }

    #[test]
    fn owners_are_found_by_their_recorded_diplomacy() {
        let identity = |id: u32, diplomacy| Identity {
            country: 0x1000 + id as usize,
            id,
            diplomacy,
            matrix: Array::default(),
        };
        let baseline = Baseline {
            owners: vec![Identity::default(), identity(7, 0x500), Identity::default()],
            rows: vec![vec![], vec![Row::default(); 2], vec![]],
            by_diplomacy: vec![(0x300, 2), (0x500, 1)],
            recorded: 2,
            ..Baseline::default()
        };
        assert_eq!(baseline.owner(0x500), Some(7));
        assert_eq!(baseline.owner(0x300), None);
        assert_eq!(baseline.owner(0x400), None);
        assert_eq!(baseline.rows(), 2);
    }
}

#[cfg(test)]
mod demand_tests {
    use super::*;
    #[test]
    fn empty_baseline_stays_empty_and_grows_to_original_demand() {
        let mut baseline = Baseline::default();
        baseline.grow().unwrap();
        assert!(baseline.owners.is_empty());
        assert!(baseline.scratch.is_empty());
        baseline.required = [5_001, 5_001, 5_001];
        baseline.grow().unwrap();
        assert_eq!(baseline.owners.len(), 5_001);
        assert_eq!(baseline.scratch.len(), 5_001);
        assert_eq!(baseline.take.len(), 5_001);
        baseline.demand[5_000] = [5_001, 5_001];
        baseline.grow().unwrap();
        assert_eq!(baseline.pending[5_000].capacity(), 5_001);
        assert_eq!(baseline.rows[5_000].capacity(), 5_001);
    }
}
