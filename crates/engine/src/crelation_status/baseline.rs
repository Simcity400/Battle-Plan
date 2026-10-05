//! Nonzero opinions kept between relations walks, per owner, so a walk whose
//! caller observed every opinion write since an owner was last read reads only
//! that owner's changed statuses.
//!
//! Opinion writers (hoi4re, a729; the instruction-level proof is recorded in
//! `captures/hooks/writers.md`, "Relation opinions"): of every instruction
//! that stores to `CRelationStatus+0x300..+0x303`, directly, through a derived
//! pointer or by a bulk copy, only four functions write a status:
//! - `CRelationStatus::RecomputeOpinion@140D19090` (store at `140D191F5`), the
//!   only writer at runtime besides `Reset`. Opinion-modifier add and remove
//!   (`AddOrUpdateOpinionModifier@140D04C70`, `RemoveOpinionModifier@140D17F70`),
//!   daily expiry (`140D10A00`), diplomatic actions (`1410EE2D0`), improve
//!   relations (`140D159B0`) and the daily sweep (`1406DB660`, a
//!   `CGameState::DailyUpdate` TBB task) all reach it.
//! - `CRelationStatus::Reset@140D09D50` (stores 0 at `140D0A327`), reachable at
//!   runtime from civil wars, annexation, state transfer and dynamic-country
//!   reuse.
//! - `CRelationStatus::LoadToken@140D16AE0`, loading `cached_sum`.
//! - `CRelationStatus::constructor@140D03410`, only at `CGameState::reset`.
//!
//! The matrix and each status's vtable and target are written only by the
//! constructors at `CGameState::reset` (see [`super::walk`]), statuses are
//! freed only in `~CDiplomacyStatus`, and no whole status is ever copied. So
//! an owner whose country, diplomacy and matrix header are unchanged keeps
//! every status, and its opinions change only where one of those writers ran.
//! The caller observes the first two (marking a status whose value changed)
//! and treats the last two as the end of its observation.
//!
//! Relations are read once a day, one owner slice per hour
//! (`replay::cadence::Cadence::Sliced`). Each mark therefore carries its
//! status's owning diplomacy, `CRelationStatus+0x1F0`, which the writer's
//! detour reads from the status it is writing. Its only writer is the status
//! constructor (`140D0356C`, the `CDiplomacyStatus*` argument; proof in
//! `captures/hooks/writers.md`, "Relation opinions", and
//! `captures/p6_relations_roles/opinion_owner_0x1f0.md`), and it equals the
//! owner's `CCountry+0xF88`, the diplomacy the walk authenticates. A mark is
//! filed under its owner without reading memory and waits for that owner's
//! slice; only then is it authenticated and its opinion read. Each owner is
//! valid on its own: a lost observation, an overflowing owner or a changed
//! owner makes owners invalid, and an invalid owner is read in full at its
//! next slice and is valid again from then on.

use super::{
    Array, Counters, MARK_BYTES, MARK_FIELDS, MARK_LAYOUT, OpinionRow, POINTER_LAYOUT, Resolver,
    ccountry,
};
use crate::{Memory, memory::RecordSources};

/// One observed opinion write: the status whose opinion changed and its
/// owning diplomacy, `CRelationStatus+0x1F0`, as the writer's detour read it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub status: usize,
    pub diplomacy: usize,
}

/// The caller's observation of opinion writes. `token` names it: a new one
/// after any load, gap or unobserved interval. `marks`, when present, holds
/// every write a writer made since the previous walk under this token, in any
/// order and possibly repeated; when absent (writes went unobserved), every
/// owner is invalid and is read in full at its next slice.
#[derive(Debug, Clone, Copy)]
pub struct Observation<'a> {
    pub token: (u64, u64),
    pub marks: Option<&'a [Mark]>,
}

/// The caller's retained opinions. Storage grows only in [`Baseline::grow`],
/// off the game thread; an owner whose rows or marks do not fit is invalid
/// until its next slice after growth.
///
/// Memory is about 16 bytes per nonzero opinion and 8 bytes per pending mark,
/// each with a quarter headroom, plus about 80 bytes per country.
#[derive(Default)]
pub struct Baseline {
    scope: (usize, usize),
    /// The observation the owners are current for; `None` when none is.
    token: Option<(u64, u64)>,
    census: u32,
    /// Per canonical slot: the owner its rows were recorded for. A default
    /// identity (no country) holds no rows and is read in full at its slice.
    owners: Vec<Identity>,
    /// Per canonical slot: its nonzero opinions, in matrix-slot order.
    rows: Vec<Vec<Row>>,
    /// Per canonical slot: statuses marked since its last read.
    pending: Vec<Vec<usize>>,
    /// Per canonical slot: `[rows, pending]` it needed that did not fit.
    demand: Vec<[u32; 2]>,
    /// `(diplomacy, slot)` of every valid owner, sorted, in the first
    /// `recorded` entries.
    by_diplomacy: Vec<(usize, u32)>,
    recorded: usize,
    /// This walk's marks of selected owners, `(owner slot, status)`.
    take: Vec<(u32, usize)>,
    /// Their authenticated opinions, sorted by owner and matrix slot.
    updates: Vec<Row>,
    updates_len: usize,
    cursor: usize,
    /// The owner being visited, its identity and its rows so far.
    owner: u32,
    visiting: Option<Identity>,
    scratch: Vec<Row>,
    scratch_len: usize,
    scratch_count: usize,
    recording: bool,
    next_census: u32,
    /// `[slots, scratch rows, marks taken in one walk]`.
    required: [usize; 3],
    counts: Counts,
}

/// What made an owner's statuses the same objects as last time.
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

/// One nonzero opinion: the owner's canonical slot, the matrix slot and the
/// raw target reference it is emitted with.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Row {
    owner: u32,
    slot: u32,
    target: u32,
    opinion: i32,
}

/// Cumulative work, for diagnostics and benches.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Opinion walks without a usable observation: every selected owner read
    /// in full.
    pub full_walks: u64,
    /// Opinion walks under an observation.
    pub incremental_walks: u64,
    /// Selected owners whose rows were replayed without reading their matrix.
    pub reused_owners: u64,
    /// Selected owners read in full (invalid or changed) under an observation.
    pub read_owners: u64,
    /// Marked statuses read.
    pub marks: u64,
    /// Walks whose observation could not be used (lost writes, a mark that
    /// did not authenticate, short storage): every owner became invalid.
    pub refused_observations: u64,
    /// Owners made invalid on their own (rows or marks that did not fit, a
    /// replaced country).
    pub invalidated_owners: u64,
}

impl Baseline {
    /// Scratch rows every grown baseline holds: one owner's whole matrix at
    /// the vanilla country count.
    pub const MIN_SCRATCH: usize = 1024;
    /// Marks every grown baseline can take in one walk (64 KiB).
    pub const MIN_MARKS: usize = 4096;
    /// Marks every slot can hold between two of its reads: a day's writes to
    /// one owner's statuses fit before any overflow has been seen.
    pub const MIN_PENDING: usize = 64;

    /// Off the game thread: room for `marks` marks in one walk, as when the
    /// caller knows its observation's bound.
    pub fn reserve_marks(&mut self, marks: usize) -> std::io::Result<()> {
        self.required[2] = self.required[2].max(marks);
        self.grow()
    }

    /// Forget every owner: each is read in full at its next slice.
    pub fn clear(&mut self) {
        self.token = None;
        self.forget();
    }

    /// The observation the owners are current for, if any.
    pub fn token(&self) -> Option<(u64, u64)> {
        self.token
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Owners currently valid.
    pub fn valid_owners(&self) -> usize {
        self.owners.iter().filter(|owner| owner.valid()).count()
    }

    /// Whether the last walk needed more storage than it had.
    pub fn needs_growth(&self) -> bool {
        let [slots, scratch, take] = self.required;
        slots > self.owners.len()
            || scratch > self.scratch.len()
            || take > self.take.len()
            || self.demand.iter().any(|demand| *demand != [0, 0])
            || self
                .pending
                .iter()
                .any(|pending| pending.capacity() < Self::MIN_PENDING)
    }

    /// Grow to the last walk's demand, with a quarter headroom for rows and
    /// marks. Allocation failure is reported before admitting another walk.
    pub fn grow(&mut self) -> std::io::Result<()> {
        fn fit<T: Default>(vector: &mut Vec<T>, required: usize) -> std::io::Result<()> {
            if required > vector.len() {
                let len = required
                    .checked_add(required / 4)
                    .ok_or_else(crate::allocation::oom)?;
                crate::allocation::reserve_exact(vector, len - vector.len())?;
                vector.resize_with(len, T::default);
            }
            Ok(())
        }
        fn reserve<T>(vector: &mut Vec<T>, required: usize) -> std::io::Result<()> {
            if required > vector.capacity() {
                let len = required
                    .checked_add(required / 4)
                    .ok_or_else(crate::allocation::oom)?;
                crate::allocation::reserve_exact(vector, len - vector.len())?;
            }
            Ok(())
        }
        let [slots, scratch, take] = self.required;
        if slots > self.owners.len() {
            let additional = slots - self.owners.len();
            crate::allocation::reserve_exact(&mut self.owners, additional)?;
            let additional = slots - self.rows.len();
            crate::allocation::reserve_exact(&mut self.rows, additional)?;
            let additional = slots - self.pending.len();
            crate::allocation::reserve_exact(&mut self.pending, additional)?;
            let additional = slots - self.demand.len();
            crate::allocation::reserve_exact(&mut self.demand, additional)?;
            let additional = slots - self.by_diplomacy.len();
            crate::allocation::reserve_exact(&mut self.by_diplomacy, additional)?;
            self.owners.resize(slots, Identity::default());
            self.rows.resize_with(slots, Vec::new);
            self.pending.resize_with(slots, Vec::new);
            self.demand.resize(slots, [0, 0]);
            self.by_diplomacy.resize(slots, (0, 0));
        }
        for (slot, demand) in self.demand.iter_mut().enumerate() {
            if let Some(rows) = self.rows.get_mut(slot) {
                reserve(rows, demand[0] as usize)?;
            }
            if let Some(pending) = self.pending.get_mut(slot) {
                reserve(pending, (demand[1] as usize).max(Self::MIN_PENDING))?;
            }
            *demand = [0, 0];
        }
        fit(&mut self.scratch, scratch.max(Self::MIN_SCRATCH))?;
        fit(&mut self.take, take.max(Self::MIN_MARKS))?;
        fit(&mut self.updates, take.max(Self::MIN_MARKS))?;
        Ok(())
    }

    /// Tests: `updates` shorter than `take`, as after its growth failed.
    #[cfg(test)]
    pub(super) fn shorten_updates(&mut self, len: usize) {
        self.updates.truncate(len);
        self.updates.shrink_to_fit();
    }

    /// Heap payload only.
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
            + self.demand.capacity() * size_of::<[u32; 2]>()
            + self.by_diplomacy.capacity() * size_of::<(usize, u32)>()
            + self.take.capacity() * size_of::<(u32, usize)>()
            + (self.updates.capacity() + self.scratch.capacity()) * size_of::<Row>()
    }

    /// Every owner invalid, its rows and marks dropped (storage kept).
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

    /// One owner invalid: it is read in full at its next slice.
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

    /// Continue the observation and file this walk's marks under their owners.
    /// Returns whether this walk may replay valid owners; the selected owners'
    /// marks are then authenticated, read and sorted in `updates`.
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
        // Only `finish` restores a token, so a walk that never finishes (a
        // panic mid-walk, with owners half rewritten) leaves none.
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
            // A new observation (a load, a gap, lost writes): no owner is
            // current until its slice reads it again.
            if continues {
                self.counts.refused_observations += 1;
            }
            self.forget();
            return true;
        };
        // File each mark under its owner. A mark of an invalid owner, or of
        // no recorded owner, is not needed: that owner is read in full.
        for mark in marks {
            let recorded = &self.by_diplomacy[..self.recorded];
            let Ok(at) =
                recorded.binary_search_by_key(&mark.diplomacy, |&(diplomacy, _)| diplomacy)
            else {
                continue;
            };
            let slot = recorded[at].1;
            // An owner this filing made invalid stays listed until `finish`.
            if !self.owners[slot as usize].valid() {
                continue;
            }
            let pending = &mut self.pending[slot as usize];
            if pending.len() == pending.capacity() {
                // Twice what it held: the owner is read in full at its slice,
                // and its marks fit from the next day on.
                self.demand[slot as usize][1] = (pending.len() as u32)
                    .saturating_mul(2)
                    .max(Self::MIN_PENDING as u32);
                self.invalidate(slot);
                continue;
            }
            pending.push(mark.status);
        }
        // Take the selected valid owners' marks.
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
        // Both buffers hold every taken mark: they grow separately, so one
        // may have grown where the other's allocation failed.
        if taken > self.take.len().min(self.updates.len()) {
            // More marks than this walk can take: the selected owners are read
            // in full now, and the storage grows for the next walk.
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
        // A status marked twice was read once per mark, at this instant.
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

    /// Authenticate the first `taken` marks as `status(owner, canonical(target))`
    /// of their valid owner, under the diplomacy that owner was recorded with,
    /// and read their opinions. `None` refuses the whole observation.
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
            // The owner's matrix names this very status at that slot.
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

    /// Start recording this walk's owners when opinions are walked under an
    /// observation and the per-slot storage holds the census.
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

    /// Whether `slot` holds rows recorded for this very owner.
    pub(super) fn unchanged(&self, slot: u32, identity: Identity) -> bool {
        self.owners
            .get(slot as usize)
            .is_some_and(|owner| owner.valid() && *owner == identity)
    }

    /// An empty slot holds no owner.
    pub(super) fn empty(&mut self, slot: u32) {
        if self.recording {
            self.invalidate(slot);
        }
    }

    /// An unselected owner keeps its rows and marks for its own slice, unless
    /// its slot now holds another country.
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

    /// Start the selected owner this walk reads or replays at `slot`.
    pub(super) fn visit(&mut self, slot: u32, identity: Identity) {
        self.owner = slot;
        self.visiting = Some(identity);
        self.scratch_len = 0;
        self.scratch_count = 0;
    }

    /// One nonzero opinion of the owner being visited.
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

    /// Emit and record the visited owner's rows: the recorded ones, with each
    /// marked status's fresh opinion in place of its recorded one.
    pub(super) fn replay(
        &mut self,
        slot: u32,
        owner_country: u32,
        counters: &mut Counters,
        opinions: &mut [OpinionRow],
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

    /// End the visited owner: keep its rows for its next slice when they fit,
    /// else make it invalid and ask for the storage.
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
            self.demand[slot][0] = self.scratch_count as u32;
            self.invalidate(self.owner);
            return;
        }
        rows.clear();
        rows.extend_from_slice(&self.scratch[..self.scratch_len]);
        self.owners[slot] = identity;
    }

    /// Keep this walk's owners for the next walks when the walk completed and
    /// its caller observed every write; otherwise forget every owner.
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
                // Now zero: dropped.
                row(1, 2, 2, 0),
                // Newly nonzero: inserted in slot order.
                row(1, 3, 30, 9),
            ],
            updates_len: 2,
            ..Baseline::default()
        };
        let mut counters = Counters::default();
        let mut output = [OpinionRow::default(); 4];
        baseline.replay(1, 100, &mut counters, &mut output).unwrap();
        baseline.replay(2, 200, &mut counters, &mut output).unwrap();
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
}
