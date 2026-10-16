//! ScriptVariables rows held between hours ([`crate::variable_changes`]).
//!
//! The store holds the latest complete capture as it was published: its
//! scope rows, entries and key bytes in walk order, and per scope its store,
//! the registration its rows were read or carried under and its hash header.
//!
//! A quiet hour walks no scope. Before any read the capture takes the
//! domain's marks ([`Changes::take`]: every store a hooked writer touched
//! since the previous take, the marks' epoch and the structure token, which
//! every release and every first-chance C++ exception moves) and the
//! character roster's token, then reads the scope list's witnesses: the
//! global root, the country census (the callback's, which costs no read) and
//! the state array header and slot pointers. When the take, the tokens and
//! the witnesses all equal those taken before the held walk, its scope list
//! is this hour's. A recorded scope's store is created only with its owner
//! (variable store construction runs only in the owners'
//! constructors and the character loader), so a new global, country, state
//! or roster character moves a witness or the roster token; it is destroyed
//! only through the release, which moves the structure token; and the
//! owners' roots (`CCountry+0x218`, `CState+0x7F8`) are written only by
//! Their constructors  (country construction,
//! state construction), a character's store being embedded.
//! Every writer of a store's nodes, header and seed is a hooked site
//! ([`engine::script_variable_mutations`]), except a seed change in an
//! exception's unwinding, which the exception's token move covers; so an
//! unmarked store's held rows, header and seed are a fresh read's, slot for
//! slot. The hour publishes the held rows and reads afresh only the marked
//! stores, its sampled slice's and the stores the held walk could not
//! certify (unobserved, or one store held by two scopes), each exactly as
//! the walk reads a scope ([`cvariables::read_store_carried`]).
//!
//! Any other hour walks every scope ([`cvariables::walk_scopes_carried`]):
//! it reads every scope's identity and every store's vtable, header and
//! seed, and carries a store's entries and key bytes, without reading its
//! nodes or keys, only while the held read of the same scope, store and
//! header is proved current (same watch epoch and sequence; a load, gap,
//! recovery, table change, poison or running writer revokes it). When the
//! hour's take is the held walk's (no release or exception since), a store
//! whose registration still holds and that the take did not mark, but whose
//! header or seed differs from the held read, was written where no hook saw:
//! it poisons reuse. An hour after an exception reads every seed afresh and
//! counts no such difference.
//!
//! Each hour reads the carryable stores of its sampled slice
//! ([`crate::sampled_verify`], keyed by [`key`]; every one under
//! `variable_verify`) afresh, publishes the fresh rows and poisons reuse on
//! a difference; a quiet hour that finds one walks afresh. Under
//! `variable_verify` no hour is quiet: one whose quiet key matched walks and
//! its scope list is compared with the held one.
//!
//! The store follows the IntelResidual job's loan and holds only the latest
//! complete capture, so its size follows live demand. Allocation is
//! advisory: a capture whose rows cannot be held holds nothing.
use crate::{
    sampled_verify::{Check, ROTATION},
    variable_changes::{Changes, Counts, Taken},
};
use engine::{
    Memory, allocation, ccountry, cgame_state,
    cpdx_array::Array,
    cstate,
    cvariables::{self, HEADER, Row, Scope, ScopeRow, ScopeWalk, Store, VariableCarry},
    walk::{Buffer, Outcome},
};
use std::ops::Range;

/// Sampling phases: one per hour of the rotation.
const PHASES: usize = ROTATION as usize;

/// What a quiet hour must find unchanged since the held walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuietKey {
    taken: Taken,
    roster: (u64, u64),
    scope: (usize, usize),
}

/// The scope list's witnesses, read before any store.
#[derive(Default)]
struct Witness {
    /// The global store (`CGameState+0x980`).
    global: usize,
    /// The countries in walk order: pointer and id.
    countries: Vec<(usize, u32)>,
    /// The state array header and the pointer of every slot after slot
    /// zero.
    state_header: [u8; 16],
    states: Vec<usize>,
}

impl Witness {
    /// Reads every witness; `false` when one does not read or its storage
    /// cannot grow.
    fn read(&mut self, memory: &impl Memory, base: usize, state: usize) -> bool {
        self.countries.clear();
        self.states.clear();
        let Some(global) = state
            .checked_add(cgame_state::GLOBAL_VARIABLES)
            .and_then(|at| engine::read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
        else {
            return false;
        };
        self.global = global;
        // The walk's own census: the callback's authenticated countries.
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

    /// Whether a complete walk's scopes are the ones these witnesses list,
    /// in walk order: the global store at `global`, the countries, the
    /// occupied state slots, then characters.
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

/// One held scope's store and what certifies its rows.
#[derive(Clone, Copy, Debug, Default)]
struct Meta {
    container: usize,
    /// The registration `(epoch, sequence)` its rows were read or carried
    /// under; `None` when it was not observed.
    certificate: Option<(u64, u64)>,
    /// The capture registered the store after reading its header and seed,
    /// so a writer between the two went unobserved.
    fresh: bool,
    header: [u8; HEADER],
}

/// One store's plan in this hour's capture.
#[derive(Clone, Copy, Debug)]
struct Plan {
    /// The held scope this read continues: its rows are carried or compared.
    held: Option<u32>,
    meta: Meta,
}

/// The latest complete capture.
#[derive(Default)]
struct Held {
    /// As published, in walk order: rows, their entries (name starts into
    /// `names`) and key bytes.
    rows: Vec<ScopeRow>,
    entries: Vec<Row>,
    names: Vec<u8>,
    /// Per row.
    meta: Vec<Meta>,
    /// Certified scopes by store, sorted; a store two scopes hold is in
    /// neither.
    index: Vec<(usize, u32)>,
    /// The scopes no certificate covers, which a quiet hour always reads.
    always: Vec<u32>,
    /// Certified scopes the walk registered after reading their header and
    /// seed, which the next quiet hour reads afresh once.
    fresh: Vec<u32>,
    /// Scope indices by sampling phase: phase `p` at
    /// `phases[starts[p]..starts[p + 1]]`.
    phases: Vec<u32>,
    starts: [u32; PHASES + 1],
    /// The take before the walk.
    taken: Option<Taken>,
    /// What lets a quiet hour publish these rows, and its witnesses.
    quiet: Option<QuietKey>,
    witness: Witness,
    /// Captures held by copying every row, entry and key byte.
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
        self.phases.clear();
        self.starts = [0; PHASES + 1];
        self.taken = None;
        self.quiet = None;
    }

    /// The scope holding certified store `container`.
    fn find(&self, container: usize) -> Option<usize> {
        let at = self
            .index
            .binary_search_by_key(&container, |&(held, _)| held)
            .ok()?;
        Some(self.index[at].1 as usize)
    }

    /// Scope `index`'s entries (name starts into `names`) and its key bytes.
    fn rows_of(&self, index: usize) -> (&[Row], Range<usize>) {
        let row = &self.rows[index];
        let entries = &self.entries[row.entries_start..row.entries_start + row.entries_len];
        let names = match (entries.first(), entries.last()) {
            (Some(first), Some(last)) => first.name_start..last.name_start + last.name_len,
            _ => 0..0,
        };
        (entries, names)
    }

    /// Holds a complete capture's published rows; `false` when the storage
    /// could not grow.
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

    /// A quiet hour's published capture into the held one by writing only
    /// its `visits`' scopes: when each visited scope kept its entry range
    /// and each entry its key byte span, every other scope's rows, entries
    /// and key bytes were copied from the held capture unmoved, so the held
    /// capture becomes the published one, position for position. `false`,
    /// changing nothing, when a visited scope moved anything; the caller
    /// then copies the whole capture ([`Self::publish`]).
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

    /// A walk's per-scope stores and certificates (`plans`, one per row),
    /// with the index, the scopes no certificate covers and the sampling
    /// phases; `false` when the storage could not grow.
    fn certify(&mut self, plans: &[Plan]) -> bool {
        let len = self.rows.len();
        let stored = allocation::advisory(|| {
            self.meta.clear();
            self.index.clear();
            self.always.clear();
            self.fresh.clear();
            self.phases.clear();
            allocation::reserve(&mut self.meta, len)?;
            allocation::reserve(&mut self.index, len)?;
            allocation::reserve(&mut self.always, len)?;
            allocation::reserve(&mut self.fresh, len)?;
            allocation::reserve(&mut self.phases, len)?;
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
        // A store two scopes hold names no single held read.
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
        self.starts = [0; PHASES + 1];
        for row in &self.rows {
            self.starts[phase(row.scope) + 1] += 1;
        }
        for at in 0..PHASES {
            self.starts[at + 1] += self.starts[at];
        }
        let mut next = [0_u32; PHASES];
        next.copy_from_slice(&self.starts[..PHASES]);
        self.phases.resize(len, 0);
        for (index, row) in self.rows.iter().enumerate() {
            let at = &mut next[phase(row.scope)];
            self.phases[*at as usize] = index as u32;
            *at += 1;
        }
        true
    }

    fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<ScopeRow>()
            + self.entries.capacity() * size_of::<Row>()
            + self.names.capacity()
            + self.meta.capacity() * size_of::<Meta>()
            + self.index.capacity() * size_of::<(usize, u32)>()
            + (self.always.capacity() + self.fresh.capacity() + self.phases.capacity())
                * size_of::<u32>()
            + self.witness.measure()
    }
}

/// The hour's view before any store is read.
#[derive(Clone, Copy, Debug, Default)]
struct Hour {
    taken: Option<Taken>,
    roster: Option<(u64, u64)>,
    scope: (usize, usize),
    /// Every marked store was recorded.
    marks: bool,
    /// The witnesses read.
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

/// The latest complete capture and the capture being made.
#[derive(Default)]
pub struct VariableStore {
    held: Held,
    hour: Hour,
    /// The stores this hour's take visited, sorted.
    marked: Vec<usize>,
    /// This hour's witnesses.
    witness: Witness,
    /// One per store the capture planned, in its order.
    plans: Vec<Plan>,
    /// A quiet hour's visits: the held scope and whether a writer marked
    /// its store.
    visits: Vec<(u32, bool)>,
    /// The visit being read.
    visiting: Option<(u32, bool)>,
    /// The capture published a quiet hour.
    quiet: bool,
    /// A quiet visit found a difference: the hour walks afresh.
    differs: bool,
    /// A quiet hour's visit published entries, key bytes or a seed other
    /// than the held capture's, so the held capture is renewed from the
    /// published rows; otherwise they equal it and nothing is copied.
    renewed: bool,
    /// Compare the walk's scope list with the held one (verify mode, quiet
    /// key matched).
    compare: bool,
    /// Storage could not grow or a plan went missing: hold nothing.
    broken: bool,
    counts: Counts,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next capture drops every held row.
    forgotten: bool,
}

impl VariableStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next capture reads afresh.
    /// Allocation-free for the game thread.
    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    /// Captures held by copying every row, entry and key byte.
    #[cfg(any(test, feature = "test-support"))]
    pub fn full_copies(&self) -> usize {
        self.held.full_copies
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Before any read of the hour: takes the marks and the roster token.
    fn begin(&mut self, changes: &dyn Changes) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
        }
        self.plans.clear();
        self.visits.clear();
        self.visiting = None;
        self.quiet = false;
        self.differs = false;
        self.renewed = false;
        self.compare = false;
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

    /// Whether no hooked writer touched `container` between the take before
    /// the held walk and this hour's, and no release or exception ran: this
    /// hour's take is the held walk's.
    fn unmarked(&self, container: usize) -> bool {
        self.hour.marks
            && self.hour.taken.is_some()
            && self.hour.taken == self.held.taken
            && self.marked.binary_search(&container).is_err()
    }

    /// The quiet hour's visits: every marked held store, every scope no
    /// certificate covers, every freshly registered one and the slice's
    /// scopes, once each in walk order; marked and fresh ones are read
    /// afresh without a comparison. `false` when the list cannot grow.
    fn plan_visits(&mut self, check: Check) -> bool {
        let Self {
            held,
            marked,
            visits,
            ..
        } = self;
        visits.clear();
        let covered = |phase: usize| check.covers_phase(phase as u64);
        let mut demand = marked.len() + held.always.len() + held.fresh.len();
        for phase in (0..PHASES).filter(|&phase| covered(phase)) {
            demand += (held.starts[phase + 1] - held.starts[phase]) as usize;
        }
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
        for phase in (0..PHASES).filter(|&phase| covered(phase)) {
            let slice = &held.phases[held.starts[phase] as usize..held.starts[phase + 1] as usize];
            visits.extend(
                slice
                    .iter()
                    .filter(|&&index| check.covers(key(held.rows[index as usize].scope)))
                    .map(|&index| (index, false)),
            );
        }
        visits.sort_unstable();
        // One visit per scope; a marked one stays marked.
        visits.dedup_by(|later, kept| {
            let same = later.0 == kept.0;
            if same {
                kept.1 |= later.1;
            }
            same
        });
        true
    }

    /// A quiet hour that cannot publish: the hour walks afresh, keeping what
    /// its visits compared.
    fn restart(&mut self) {
        self.plans.clear();
        self.visits.clear();
        self.visiting = None;
        self.quiet = false;
        self.differs = false;
        self.renewed = false;
        self.broken = false;
        self.counts = Counts {
            verified: self.counts.verified,
            mismatches: self.counts.mismatches,
            ..Counts::default()
        };
    }

    /// Verify mode, quiet key matched: the walk's scopes and stores must be
    /// the held ones, position for position.
    fn compare_scopes(&mut self, changes: &dyn Changes, rows: &[ScopeRow]) {
        let held = &self.held;
        let same = rows.len() == held.rows.len()
            && self.plans.len() == rows.len()
            && rows
                .iter()
                .zip(&self.plans)
                .zip(held.rows.iter().zip(&held.meta))
                .all(|((row, plan), (held_row, meta))| {
                    row.scope == held_row.scope && plan.meta.container == meta.container
                });
        if !same {
            self.counts.mismatches += 1;
            changes.scope_mismatch();
        }
    }

    fn measure(&mut self) {
        self.bytes = self.held.measure()
            + self.witness.measure()
            + self.marked.capacity() * size_of::<usize>()
            + self.plans.capacity() * size_of::<Plan>()
            + self.visits.capacity() * size_of::<(u32, bool)>();
    }
}

/// A scope's stable sampling key: its kind and id.
pub fn key(scope: Scope) -> u64 {
    let (kind, id) = super::scope_identity(scope);
    (u64::from(kind) << 32) | u64::from(id)
}

fn phase(scope: Scope) -> usize {
    (key(scope) % ROTATION) as usize
}

/// Whether two stores' entries hold the same values and keys, in order;
/// each entry's name start indexes its own arena.
fn same_entries(left: &[Row], left_names: &[u8], right: &[Row], right_names: &[u8]) -> bool {
    left.len() == right.len()
        && left.iter().zip(right).all(|(left, right)| {
            left.value_raw == right.value_raw
                && left.name_len == right.name_len
                && left_names.get(left.name_start..left.name_start + left.name_len)
                    == right_names.get(right.name_start..right.name_start + right.name_len)
        })
}

/// Grows `values` to hold `len`, as a growing walk buffer does.
fn grow<T: Default>(values: &mut Vec<T>, len: usize) -> Option<()> {
    Buffer::Growing(values)
        .ensure(len)
        .ok()
        .filter(|&room| room)
        .map(|_| ())
}

/// A quiet hour's output, written in walk order.
struct Out<'o> {
    rows: &'o mut Vec<ScopeRow>,
    entries: &'o mut Vec<Row>,
    names: &'o mut Vec<u8>,
    len: usize,
    entries_len: usize,
    bytes_len: usize,
}

impl Out<'_> {
    /// Appends held scopes `range` as published, their entries and key bytes
    /// moved to this hour's positions.
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

    /// Appends a visited scope whose `count` entries (name starts relative
    /// to its own bytes) and `bytes` key bytes a read wrote at this hour's
    /// positions.
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

/// One capture's view of the store and the observation that certifies
/// reuse.
pub struct VariableReuse<'a> {
    store: &'a mut VariableStore,
    changes: &'a dyn Changes,
    /// Which carryable stores this hour reads afresh and compares.
    check: Check,
}

impl<'a> VariableReuse<'a> {
    /// Before any read of the hour; takes its marks and tokens.
    pub fn begin(store: &'a mut VariableStore, changes: &'a dyn Changes, check: Check) -> Self {
        store.begin(changes);
        Self {
            store,
            changes,
            check,
        }
    }

    /// Before the walk: reads the scope list's witnesses and, when the hour
    /// is quiet, publishes the held capture into `rows`, `entries` and
    /// `names`, reading only its visits, and returns the walk. `None` when
    /// the hour walks every scope ([`cvariables::walk_scopes_carried`] with
    /// this reuse).
    pub fn quiet_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        rows: &mut Vec<ScopeRow>,
        entries: &mut Vec<Row>,
        names: &mut Vec<u8>,
    ) -> Option<ScopeWalk> {
        let check = self.check;
        let store = &mut *self.store;
        store.hour.scope = (base, state);
        store.hour.witness = store.witness.read(memory, base, state);
        let key = store.hour.key();
        let matched = key.is_some()
            && store.held.quiet == key
            && store.hour.marks
            && store.witness.same(&store.held.witness);
        if !matched {
            return None;
        }
        if check == Check::Every {
            store.compare = true;
            return None;
        }
        if !store.plan_visits(check) {
            return None;
        }
        let walk = self.publish(memory, base, rows, entries, names);
        if walk.is_none() {
            self.store.restart();
        }
        walk
    }

    /// The quiet hour's output: the held capture, with each visit read as
    /// the walk reads its scope. `None` at a difference or a failed read.
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
            // A hint for the next visit's store head while this one is read:
            // it fetches no value and faults nothing.
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
            if self.store.differs || self.store.plans.len() != visit + 1 {
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

    /// Ends the capture: `complete` when every scope was read or carried
    /// whole, with the rows it published. A complete capture is held for the
    /// next hour, a quiet one as the held capture with its visits' stores
    /// renewed; anything else holds nothing.
    pub fn finish(self, complete: bool, rows: &[ScopeRow], entries: &[Row], names: &[u8]) {
        let Self { store, changes, .. } = self;
        if store.compare && complete {
            store.compare_scopes(changes, rows);
        }
        let holds = complete && !store.broken;
        if store.quiet {
            // Unless a visit renewed it, the published rows are the held
            // capture's, position for position.
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
                // Each was read after its registration.
                store.held.fresh.clear();
            } else {
                store.held.clear();
            }
        } else {
            let held = holds
                && store.plans.len() == rows.len()
                && store.held.publish(rows, entries, names)
                && store.held.certify(&store.plans);
            if held {
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
    /// Registers one store and plans its rows: carryable when the held read
    /// of the same scope, store and header is proved current, and carried
    /// without a read unless the hour's [`Check`] covers it. Registration
    /// precedes every read it certifies; the header and seed were read
    /// before it.
    fn plan(&mut self, store: &Store) -> (usize, Option<(usize, usize)>) {
        let changes = self.changes;
        let sampled = self.check.covers(key(store.scope));
        let state = &mut *self.store;
        let registration = changes.register(store.container);
        let certificate = registration.map(|(certificate, _)| certificate);
        // The held read this one continues: a quiet visit's own scope, or
        // the held scope of the same store.
        let (candidate, unmarked) = match state.visiting {
            Some((index, marked)) => (Some(index as usize), !marked),
            None => (
                state.held.find(store.container),
                state.unmarked(store.container),
            ),
        };
        let held = &state.held;
        let same = candidate.filter(|&index| {
            let meta = &held.meta[index];
            certificate.is_some()
                && meta.certificate == certificate
                && meta.container == store.container
                && held.rows[index].scope == store.scope
        });
        let mut current = same.filter(|&index| held.meta[index].header == store.header);
        if let Some(index) = same
            && unmarked
            && !held.meta[index].fresh
            && (current.is_none() || held.rows[index].seed != store.seed)
        {
            // No observed writer touched the store since the held read
            // registered it, yet its header or seed moved: a write no hook
            // saw.
            state.counts.mismatches += 1;
            changes.mismatch();
            if state.visiting.is_some() {
                state.differs = true;
            }
            current = None;
        }
        let carry = current.filter(|_| !sampled).map(|index| {
            let (entries, names) = state.held.rows_of(index);
            (entries.len(), names.len())
        });
        let plans = &mut state.plans;
        let token = plans.len();
        if allocation::advisory(|| allocation::reserve(plans, 1)).is_err() {
            // Without a plan the store is read afresh and nothing is held.
            state.broken = true;
            return (usize::MAX, None);
        }
        plans.push(Plan {
            held: current.map(|index| index as u32),
            meta: Meta {
                container: store.container,
                certificate,
                fresh: registration.is_some_and(|(_, fresh)| fresh),
                header: store.header,
            },
        });
        (token, carry)
    }

    /// The held rows of a carryable plan, at this hour's positions.
    fn carry(&mut self, token: usize, entries: &mut [Row], names: &mut [u8]) {
        let state = &mut *self.store;
        let Some(index) = state.plans.get(token).and_then(|plan| plan.held) else {
            return;
        };
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

    /// A fresh read of a planned store: `entries` in native slot order when
    /// `outcome` is complete. They are compared with the held rows it
    /// continues; a difference poisons reuse.
    fn read(&mut self, token: usize, outcome: Outcome, entries: &[Row], names: &[u8]) {
        let state = &mut *self.store;
        state.counts.read += 1;
        state.counts.read_rows += entries.len() as u64;
        let Some(plan) = state.plans.get(token).copied() else {
            state.broken = true;
            return;
        };
        let Some(index) = plan.held else {
            return;
        };
        state.counts.verified += 1;
        let (held_entries, _) = state.held.rows_of(index as usize);
        if !matches!(outcome, Outcome::Rows { .. })
            || !same_entries(held_entries, &state.held.names, entries, names)
        {
            state.counts.mismatches += 1;
            self.changes.mismatch();
            if state.visiting.is_some() {
                state.differs = true;
            }
        }
    }
}
