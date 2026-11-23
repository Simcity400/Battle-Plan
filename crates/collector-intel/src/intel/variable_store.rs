use crate::variable_changes::{Changes, Counts, Taken};
use engine_allocation as allocation;
use engine_native_memory::{Memory, cpdx_array::Array, walk::{Buffer, Outcome}};
use engine_native::{ccountry, cgame_state, cstate};
use engine_scripts::{
    cvariables, cvariables::HEADER, cvariables::Row, cvariables::Scope, cvariables::ScopeRow,
    cvariables::ScopeWalk, cvariables::Store, cvariables::VariableCarry,
};
use std::ops::Range;

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

#[derive(Default)]
struct Held {
    rows: Vec<ScopeRow>,
    entries: Vec<Row>,
    names: Vec<u8>,
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
        self.rows.clear();
        self.entries.clear();
        self.names.clear();
        self.meta.clear();
        self.index.clear();
        self.always.clear();
        self.fresh.clear();
        self.taken = None;
        self.generation = None;
        self.quiet = None;
    }

    fn find(&self, container: usize) -> Option<usize> {
        let at = self
            .index
            .binary_search_by_key(&container, |&(held, _)| held)
            .ok()?;
        Some(self.index[at].1 as usize)
    }

    fn rows_of(&self, index: usize) -> (&[Row], Range<usize>) {
        let row = &self.rows[index];
        let entries = &self.entries[row.entries_start..row.entries_start + row.entries_len];
        let names = match (entries.first(), entries.last()) {
            (Some(first), Some(last)) => first.name_start..last.name_start + last.name_len,
            _ => 0..0,
        };
        (entries, names)
    }

    fn publish(&mut self, rows: &[ScopeRow], entries: &[Row], names: &[u8]) -> bool {
        let stored = allocation::advisory(|| {
            self.rows.clear();
            self.entries.clear();
            self.names.clear();
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.entries, entries.len())?;
            allocation::reserve(&mut self.names, names.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        self.rows.extend_from_slice(rows);
        self.entries.extend_from_slice(entries);
        self.names.extend_from_slice(names);
        #[cfg(any(test, feature = "test-support"))]
        {
            self.full_copies += 1;
        }
        true
    }

    fn patch(
        &mut self,
        visits: &[(u32, bool)],
        rows: &[ScopeRow],
        entries: &[Row],
        names: &[u8],
    ) -> bool {
        if rows.len() != self.rows.len()
            || entries.len() != self.entries.len()
            || names.len() != self.names.len()
        {
            return false;
        }
        let range = |row: &ScopeRow| row.entries_start..row.entries_start + row.entries_len;
        let unmoved = visits.iter().all(|&(index, _)| {
            let at = index as usize;
            let (Some(held), Some(row)) = (self.rows.get(at), rows.get(at)) else {
                return false;
            };
            held.scope == row.scope
                && range(held) == range(row)
                && self
                    .entries
                    .get(range(held))
                    .zip(entries.get(range(row)))
                    .is_some_and(|(kept, now)| {
                        kept.iter().zip(now).all(|(kept, now)| {
                            (kept.name_start, kept.name_len) == (now.name_start, now.name_len)
                        })
                    })
        });
        if !unmoved {
            return false;
        }
        for &(index, _) in visits {
            let at = index as usize;
            let span = range(&rows[at]);
            self.rows[at] = rows[at];
            self.entries[span.clone()].copy_from_slice(&entries[span.clone()]);
            for entry in &entries[span] {
                let bytes = entry.name_start..entry.name_start + entry.name_len;
                self.names[bytes.clone()].copy_from_slice(&names[bytes]);
            }
        }
        true
    }

    fn certify(&mut self, plans: &[Plan]) -> bool {
        let len = self.rows.len();
        let stored = allocation::advisory(|| {
            self.meta.clear();
            self.index.clear();
            self.always.clear();
            self.fresh.clear();
            allocation::reserve(&mut self.meta, len)?;
            allocation::reserve(&mut self.index, len)?;
            allocation::reserve(&mut self.always, len)?;
            allocation::reserve(&mut self.fresh, len)?;
            std::io::Result::Ok(())
        });
        if stored.is_err() || plans.len() != len || u32::try_from(len).is_err() {
            return false;
        }
        self.meta.extend(plans.iter().map(|plan| plan.meta));
        for (index, meta) in self.meta.iter().enumerate() {
            match meta.certificate {
                Some(_) => self.index.push((meta.container, index as u32)),
                None => self.always.push(index as u32),
            }
        }
        self.index.sort_unstable();
        let mut kept = 0;
        let mut at = 0;
        while at < self.index.len() {
            let container = self.index[at].0;
            let mut end = at + 1;
            while end < self.index.len() && self.index[end].0 == container {
                end += 1;
            }
            if end - at == 1 {
                self.index[kept] = self.index[at];
                kept += 1;
            } else {
                for duplicate in at..end {
                    let scope = self.index[duplicate].1;
                    self.always.push(scope);
                }
            }
            at = end;
        }
        self.index.truncate(kept);
        self.always.sort_unstable();
        self.fresh.extend(
            self.index
                .iter()
                .map(|&(_, index)| index)
                .filter(|&index| self.meta[index as usize].fresh),
        );
        self.fresh.sort_unstable();
        true
    }

    fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<ScopeRow>()
            + self.entries.capacity() * size_of::<Row>()
            + self.names.capacity()
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
    unlisted: bool,
    quiet: bool,
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
        self.visits.clear();
        self.visiting = None;
        self.clear_changed();
        self.quiet = false;
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
                        .and_then(|held| self.held.rows.get(held as usize))
                        .is_some_and(|held| {
                            held.scope == rows[index].scope && held.seed == rows[index].seed
                        });
                if !kept {
                    self.change(index as u32);
                }
            }
        }
        delta.same_scopes = self.quiet
            || (rows.len() == self.held.rows.len()
                && rows
                    .iter()
                    .zip(&self.held.rows)
                    .all(|(row, held)| row.scope == held.scope));
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
        self.visits.clear();
        self.visiting = None;
        self.clear_changed();
        self.quiet = false;
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
            + self.changed.capacity() * size_of::<u32>();
    }
}

fn same_entries(left: &[Row], left_names: &[u8], right: &[Row], right_names: &[u8]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value_raw == right.value_raw
                && left.name_len == right.name_len
                && left_names.get(left.name_start..left.name_start + left.name_len)
                    == right_names.get(right.name_start..right.name_start + right.name_len)
        })
}

fn grow<T: Default>(values: &mut Vec<T>, len: usize) -> Option<()> {
    Buffer::Growing(values)
        .ensure(len)
        .ok()
        .filter(|&room| room)
        .map(|_| ())
}

struct Out<'o> {
    rows: &'o mut Vec<ScopeRow>,
    entries: &'o mut Vec<Row>,
    names: &'o mut Vec<u8>,
    len: usize,
    entries_len: usize,
    bytes_len: usize,
}

impl Out<'_> {
    fn copy(&mut self, held: &Held, range: Range<usize>) -> Option<()> {
        let rows = held.rows.get(range)?;
        let (Some(first), Some(last)) = (rows.first(), rows.last()) else {
            return Some(());
        };
        let entries = held
            .entries
            .get(first.entries_start..last.entries_start + last.entries_len)?;
        let names: &[u8] = match (entries.first(), entries.last()) {
            (Some(start), Some(end)) => held
                .names
                .get(start.name_start..end.name_start + end.name_len)?,
            _ => &[],
        };
        let row_end = self.len.checked_add(rows.len())?;
        let entry_end = self.entries_len.checked_add(entries.len())?;
        let byte_end = self.bytes_len.checked_add(names.len())?;
        grow(self.rows, row_end)?;
        grow(self.entries, entry_end)?;
        grow(self.names, byte_end)?;
        let entry_shift = self.entries_len.wrapping_sub(first.entries_start);
        for (target, row) in self.rows[self.len..row_end].iter_mut().zip(rows) {
            *target = ScopeRow {
                entries_start: row.entries_start.wrapping_add(entry_shift),
                ..*row
            };
        }
        let name_shift = self
            .bytes_len
            .wrapping_sub(entries.first().map_or(0, |entry| entry.name_start));
        for (target, entry) in self.entries[self.entries_len..entry_end]
            .iter_mut()
            .zip(entries)
        {
            *target = Row {
                name_start: entry.name_start.wrapping_add(name_shift),
                ..*entry
            };
        }
        self.names[self.bytes_len..byte_end].copy_from_slice(names);
        self.len = row_end;
        self.entries_len = entry_end;
        self.bytes_len = byte_end;
        Some(())
    }

    fn push(&mut self, scope: Scope, count: usize, bytes: usize, seed: Option<u32>) -> Option<()> {
        let row_end = self.len.checked_add(1)?;
        let entry_end = self.entries_len.checked_add(count)?;
        grow(self.rows, row_end)?;
        for entry in self.entries.get_mut(self.entries_len..entry_end)? {
            entry.name_start = entry.name_start.checked_add(self.bytes_len)?;
        }
        self.rows[self.len] = ScopeRow {
            scope,
            entries_start: self.entries_len,
            entries_len: count,
            seed,
        };
        self.len = row_end;
        self.entries_len = entry_end;
        self.bytes_len = self.bytes_len.checked_add(bytes)?;
        Some(())
    }
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

    pub fn quiet_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut Vec<ScopeRow>,
        entries: &mut Vec<Row>,
        names: &mut Vec<u8>,
    ) -> Option<ScopeWalk> {
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
        let walk = self.publish(memory, base, rows, entries, names);
        if walk.is_none() {
            self.store.restart();
        }
        walk
    }

    fn publish(
        &mut self,
        memory: &impl Memory,
        base: usize,
        rows: &mut Vec<ScopeRow>,
        entries: &mut Vec<Row>,
        names: &mut Vec<u8>,
    ) -> Option<ScopeWalk> {
        let mut out = Out {
            rows,
            entries,
            names,
            len: 0,
            entries_len: 0,
            bytes_len: 0,
        };
        let mut cursor = 0;
        for visit in 0..self.store.visits.len() {
            let (index, marked) = self.store.visits[visit];
            let at = index as usize;
            if let Some(&(next, _)) = self.store.visits.get(visit + 1)
                && let Some(meta) = self.store.held.meta.get(next as usize)
            {
                memory.prefetch(meta.container, cvariables::STORAGE + HEADER);
            }
            out.copy(&self.store.held, cursor..at)?;
            let scope = self.store.held.rows[at].scope;
            let container = self.store.held.meta[at].container;
            self.store.visiting = Some((index, marked));
            let read = cvariables::read_store_carried(
                memory,
                base,
                scope,
                container,
                Buffer::GrowingFrom(&mut *out.entries, out.entries_len),
                Buffer::GrowingFrom(&mut *out.names, out.bytes_len),
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
            let held = &self.store.held;
            let (kept, _) = held.rows_of(at);
            let same = held.rows[at].seed == seed
                && out
                    .entries
                    .get(out.entries_len..out.entries_len.checked_add(count)?)
                    .zip(
                        out.names
                            .get(out.bytes_len..out.bytes_len.checked_add(walk.bytes_len)?),
                    )
                    .is_some_and(|(entries, names)| {
                        same_entries(entries, names, kept, &held.names)
                    });
            self.store.renewed |= !same;
            if !same {
                self.store.change(index);
            }
            out.push(scope, count, walk.bytes_len, seed)?;
            cursor = at + 1;
        }
        out.copy(&self.store.held, cursor..self.store.held.rows.len())?;
        let store = &mut *self.store;
        store.quiet = true;
        store.counts.quiet = true;
        store.counts.visited = store.visits.len() as u64;
        store.counts.reused = out.len as u64 - store.counts.read;
        store.counts.reused_rows = out.entries_len as u64 - store.counts.read_rows;
        Some(ScopeWalk {
            outcome: Outcome::Rows { len: out.len },
            entries_len: out.entries_len,
            required_entries: out.entries_len,
            bytes_len: out.bytes_len,
            required_bytes: out.bytes_len,
        })
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
                && rows.len() == store.held.rows.len()
                && (!store.renewed
                    || store.held.patch(&store.visits, rows, entries, names)
                    || store.held.publish(rows, entries, names));
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
                && store.held.publish(rows, entries, names)
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
            certificate.is_some()
                && meta.certificate == certificate
                && meta.container == store.container
                && meta.header == store.header
                && held.rows[index].scope == store.scope
                && (!unmarked || meta.fresh || held.rows[index].seed == store.seed)
        });
        let carry = current.map(|index| {
            let (entries, names) = held.rows_of(index);
            (entries.len(), names.len())
        });
        let plans = &mut state.plans;
        let token = plans.len();
        if allocation::advisory(|| allocation::reserve(plans, 1)).is_err() {
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
        plan.carried = true;
        let (held_entries, held_names) = state.held.rows_of(index as usize);
        let base = held_names.start;
        for (target, entry) in entries.iter_mut().zip(held_entries) {
            *target = Row {
                name_start: entry.name_start - base,
                ..*entry
            };
        }
        names.copy_from_slice(&state.held.names[held_names]);
        state.counts.reused += 1;
        state.counts.reused_rows += entries.len() as u64;
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
