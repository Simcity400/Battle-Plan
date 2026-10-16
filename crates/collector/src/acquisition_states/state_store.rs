//! States rows held between hours ([`crate::territory_changes`]).
//!
//! Each hour the States job still reads the state array header, every slot
//! pointer, each state's identity, available manpower (rewritten every hour
//! by reinforcement) and population in one record, and the shared infrastructure and continent operands
//! ([`cstate::globals`]), and the StateFlags visitor still reads every
//! state. A state's owner, controller, continent, movement infrastructure,
//! names and railway fraction are carried, without reading its tags,
//! template, building status or strings, only when all of these hold: the
//! take of the hour's marked states returned the epoch of the held complete
//! capture, the state was not marked, and the header, every slot pointer and
//! the shared operands are those of the held capture. Every writer of those
//! fields is a hooked site ([`engine::territory_mutations`]) or a building
//! writer the building family forwards, so a carried row equals a fresh
//! read's. Each hour also reads the carryable states of its sampled slice
//! ([`crate::sampled_verify`], keyed by state id; every one under
//! `territory_verify`) afresh, publishes the fresh rows and poisons reuse on
//! a difference.
//!
//! The take clears the marks, so a capture that does not complete holds
//! nothing and a failed job forgets the store: the next hour reads every
//! state. Allocation is advisory; a store that cannot grow holds nothing.
use super::Inputs;
use crate::{
    extended::state_flags,
    sampled_verify::Check,
    territory_changes::{self, Changes, Counts, Table},
};
use engine::{
    Memory, allocation, cpdx_array::Array, crailway_manager::fresh_inputs::StateFractionRow,
    cstate, walk::Outcome,
};

type Fraction = Result<i64, &'static str>;

/// The latest complete capture.
#[derive(Default)]
struct Held {
    /// The marks' epoch it was certified in; `None` holds nothing.
    epoch: Option<u64>,
    header: [u8; 16],
    globals: Option<cstate::Globals>,
    /// In census (state id) order, with spans into `names`.
    rows: Vec<cstate::Row>,
    pointers: Vec<usize>,
    names: Vec<u8>,
    fractions: Vec<Fraction>,
}

impl Held {
    fn clear(&mut self) {
        self.epoch = None;
        self.rows.clear();
        self.pointers.clear();
        self.names.clear();
        self.fractions.clear();
    }

    fn names(&self, index: usize) -> [&[u8]; 2] {
        let row = &self.rows[index];
        [row.custom_name, row.owner_name].map(|span| &self.names[span.start..][..span.len])
    }

    fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<cstate::Row>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.names.capacity()
            + self.fractions.capacity() * size_of::<Fraction>()
    }
}

/// The hour's view before any read: its marks and the operands that certify
/// carried rows.
#[derive(Default)]
struct Hour {
    epoch: Option<u64>,
    header: Option<[u8; 16]>,
    globals: Option<cstate::Globals>,
    /// Marks lost to a failed scratch allocation certify nothing.
    complete: bool,
}

/// The States job's held rows and scratch.
pub(crate) struct StateStore {
    changes: &'static dyn Changes,
    /// Tests fix the comparison; production takes the hour's.
    check: Option<Check>,
    held: Held,
    hour: Hour,
    /// Set on the game thread; the next capture drops every held row.
    forgotten: bool,
    /// Allocated bytes as of the last capture, for the game thread.
    bytes: usize,
    marked: Vec<u32>,
    slots: Vec<usize>,
    ids: Vec<u32>,
    manpower: Vec<cstate::Manpower>,
    /// Per held row: read afresh, and whether only for comparison.
    reread: Vec<bool>,
    sampled: Vec<bool>,
    counts: Counts,
}

impl StateStore {
    pub(crate) fn new(changes: &'static dyn Changes) -> Self {
        Self {
            changes,
            check: None,
            held: Held::default(),
            hour: Hour::default(),
            forgotten: false,
            bytes: 0,
            marked: Vec::new(),
            slots: Vec::new(),
            ids: Vec::new(),
            manpower: Vec::new(),
            reread: Vec::new(),
            sampled: Vec::new(),
            counts: Counts::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_check(mut self, check: Check) -> Self {
        self.check = Some(check);
        self
    }

    /// Nothing held certifies anything: the next capture reads afresh.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last capture, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }

    /// Worker, first in the job: take the hour's marked states and read the
    /// operands that certify carried rows.
    pub(super) fn begin(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
        }
        self.counts = Counts::default();
        self.marked.clear();
        let marked = &mut self.marked;
        let mut complete = true;
        let epoch = self.changes.take_states(&mut |id| match u32::try_from(id) {
            Ok(id) if allocation::advisory(|| allocation::reserve(marked, 1)).is_ok() => {
                marked.push(id)
            }
            _ => complete = false,
        });
        self.marked.sort_unstable();
        self.hour = Hour {
            epoch,
            header: cstate::header(memory, game_state),
            globals: cstate::globals(memory, base),
            complete,
        };
    }

    fn check(&self) -> Check {
        self.check
            .unwrap_or_else(|| crate::sampled_verify::current_burned_in(territory_changes::VERIFY))
    }

    /// Whether this hour may carry the held rows.
    fn carryable(&self) -> bool {
        self.hour.complete
            && self.held.epoch.is_some()
            && self.hour.epoch == self.held.epoch
            && self.hour.header == Some(self.held.header)
            && self.hour.globals.is_some()
            && self.hour.globals == self.held.globals
    }

    /// The carried capture: rows, names, the visitor's sources and the
    /// fractions, as a fresh read would leave them. `None`, with nothing
    /// visited, when the held rows cannot be carried or a read the carry
    /// needs fails; the caller then reads every state.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        growing: bool,
        rows: &mut Vec<cstate::Row>,
        names: &mut Vec<u8>,
        visitor: &mut state_flags::Captured,
        inputs: &mut Inputs,
    ) -> Option<(Outcome, cstate::NameWalk)> {
        if !self.carryable() {
            return None;
        }
        let array = Array::from_header(self.held.header, 8).filter(|array| array.count >= 2)?;
        let len = self.held.rows.len();
        let scratch = allocation::advisory(|| {
            allocation::resize_default(&mut self.slots, array.count as usize - 1)?;
            for values in [&mut self.reread, &mut self.sampled] {
                values.clear();
                allocation::resize_default(values, len)?;
            }
            allocation::resize_default(&mut self.ids, len)?;
            allocation::resize_default(&mut self.manpower, len)
        });
        scratch.ok()?;
        // The occupied slots are exactly the held states, at the same
        // addresses.
        cstate::read_slots(memory, array, &mut self.slots).ok()?;
        let mut occupied = self
            .slots
            .iter()
            .enumerate()
            .filter(|(_, pointer)| **pointer != 0);
        for (index, row) in self.held.rows.iter().enumerate() {
            let (slot, &pointer) = occupied.next()?;
            if slot + 1 != row.id as usize || pointer != self.held.pointers[index] {
                return None;
            }
            self.ids[index] = row.id;
        }
        if occupied.next().is_some() {
            return None;
        }
        cstate::read_manpower(
            memory,
            base,
            &self.held.pointers,
            &self.ids,
            &mut self.manpower,
        )
        .ok()?;
        let check = self.check();
        for &id in &self.marked {
            if let Ok(index) = self.ids.binary_search(&id) {
                self.reread[index] = true;
            }
        }
        for (index, &id) in self.ids.iter().enumerate() {
            if !self.reread[index] && check.covers(u64::from(id)) {
                self.reread[index] = true;
                self.sampled[index] = true;
            }
        }
        if !engine::walk::Buffer::for_worker(rows, growing)
            .ensure(len)
            .ok()?
        {
            return None;
        }
        for (index, row) in rows[..len].iter_mut().enumerate() {
            *row = cstate::Row {
                manpower_available: self.manpower[index].available,
                population: self.manpower[index].population,
                ..self.held.rows[index]
            };
            if self.reread[index] {
                let [owner, controller] =
                    cstate::read_country_tags(memory, self.held.pointers[index]).ok()?;
                (row.owner, row.controller) = (owner, controller);
            }
        }
        let census = Outcome::Rows { len };
        let reread = &self.reread;
        cstate::read_continents_selected(memory, base, game_state, census, rows, |index| {
            reread[index]
        });
        if cstate::read_movement_infrastructure_selected(
            memory,
            base,
            game_state,
            census,
            rows,
            |index| reread[index],
        ) != census
        {
            return None;
        }
        let held = &self.held;
        let walk = cstate::read_names_carried(
            memory,
            game_state,
            census,
            rows,
            engine::walk::Buffer::for_worker(names, growing),
            |index| (!reread[index]).then(|| held.names(index)),
        );
        if walk.outcome != census {
            return None;
        }
        // Nothing below falls back: the visitor sees exactly the census.
        for (&pointer, &id) in self.held.pointers.iter().zip(&self.ids) {
            let source = cstate::Source { pointer, id };
            if growing {
                visitor.visit_growing(source);
            } else {
                visitor.visit(source);
            }
        }
        visitor.finish(census);
        inputs.acquire_carried(memory, base, visitor, growing, |index| {
            (!reread[index]).then(|| held.fractions[index])
        });
        self.compare(rows, names, inputs);
        self.hold(rows, names, walk, visitor, inputs);
        Some((census, walk))
    }

    /// Compares each sampled row's fresh read with its held row; a
    /// difference poisons reuse.
    fn compare(&mut self, rows: &[cstate::Row], names: &[u8], inputs: &Inputs) {
        let fractions = inputs.complete(rows.len().min(self.held.rows.len()));
        for (index, held) in self.held.rows.iter().enumerate() {
            if !self.reread[index] {
                self.counts.reused += 1;
                continue;
            }
            self.counts.read += 1;
            if !self.sampled[index] {
                continue;
            }
            self.counts.verified += 1;
            let fresh = &rows[index];
            let same = fresh.owner == held.owner
                && fresh.controller == held.controller
                && fresh.continent == held.continent
                && fresh.movement_infrastructure == held.movement_infrastructure
                && text(fresh, names) == text(held, &self.held.names)
                && fractions
                    .and_then(|rows| rows.get(index))
                    .map(|row| row.fraction)
                    == Some(self.held.fractions[index]);
            if !same {
                self.counts.mismatches += 1;
                self.changes.mismatch();
            }
        }
        self.counts.quiet = self.marked.is_empty();
    }

    /// Holds a capture whose census, names, sources and fractions are all
    /// complete, under the hour's marks; otherwise holds nothing.
    pub(super) fn hold(
        &mut self,
        rows: &[cstate::Row],
        names: &[u8],
        walk: cstate::NameWalk,
        visitor: &state_flags::Captured,
        inputs: &Inputs,
    ) {
        let len = walk.outcome.len();
        let (sources, source, _) = visitor.completed_sources();
        let fractions = inputs.complete(len);
        let complete = self.hour.complete
            && walk.outcome == (Outcome::Rows { len })
            && len != 0
            && source == walk.outcome
            && sources.len() == len
            && sources
                .iter()
                .zip(rows)
                .all(|(source, row)| source.id == row.id);
        let (Some(epoch), Some(header), Some(globals), Some(fractions), true) = (
            self.hour.epoch,
            self.hour.header,
            self.hour.globals,
            fractions,
            complete,
        ) else {
            self.held.clear();
            return;
        };
        let held = &mut self.held;
        let stored = allocation::advisory(|| {
            held.rows.clear();
            held.pointers.clear();
            held.names.clear();
            held.fractions.clear();
            allocation::reserve(&mut held.rows, len)?;
            allocation::reserve(&mut held.pointers, len)?;
            allocation::reserve(&mut held.names, walk.bytes_len)?;
            allocation::reserve(&mut held.fractions, len)?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            held.clear();
            return;
        }
        held.rows.extend_from_slice(&rows[..len]);
        held.pointers
            .extend(sources.iter().map(|source| source.pointer));
        held.names.extend_from_slice(&names[..walk.bytes_len]);
        held.fractions
            .extend(fractions.iter().map(|row| row.fraction));
        held.epoch = Some(epoch);
        held.header = header;
        held.globals = Some(globals);
    }

    /// Worker, last in the job: publish the capture's counts; `full` when
    /// it read all `len` states afresh.
    pub(super) fn finish(&mut self, full: bool, len: usize) {
        self.counts.full = full;
        if full {
            self.counts = Counts {
                read: len as u64,
                full,
                ..Counts::default()
            };
        }
        let slots = self
            .hour
            .header
            .and_then(|header| Array::from_header(header, 8))
            .map(|array| array.count as usize);
        territory_changes::record(Table::States, &self.counts, slots);
        self.bytes = self.held.measure()
            + self.marked.capacity() * size_of::<u32>()
            + self.slots.capacity() * size_of::<usize>()
            + self.ids.capacity() * size_of::<u32>()
            + self.manpower.capacity() * size_of::<cstate::Manpower>()
            + self.reread.capacity()
            + self.sampled.capacity();
    }
}

/// A row's custom and owner name bytes in `bytes`.
fn text<'a>(row: &cstate::Row, bytes: &'a [u8]) -> [&'a [u8]; 2] {
    [row.custom_name, row.owner_name].map(|span| &bytes[span.start..][..span.len])
}

/// The fractions of a complete capture.
impl Inputs {
    pub(super) fn complete(&self, len: usize) -> Option<&[StateFractionRow]> {
        (self.outcome == (Outcome::Rows { len })).then(|| &self.rows[..len])
    }
}
