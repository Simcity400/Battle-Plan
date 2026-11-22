use super::Inputs;
use crate::extended::state_flags;
use collector_observers::territory_changes::{self, Changes, Counts, Table};
use engine_allocation as allocation;
use engine_native::{Memory, cpdx_array::Array, cstate, cstate::infrastructure_map, walk::Outcome};
use engine_railways::crailway_manager::fresh_inputs::StateFractionRow;

type Fraction = Result<i64, &'static str>;

#[derive(Default)]
struct Held {
    epoch: Option<u64>,
    header: [u8; 16],
    globals: Option<cstate::Globals>,
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

    fn names(&self, index: usize) -> cstate::Carried<'_> {
        let row = &self.rows[index];
        let bytes = |span: cstate::NameSpan| &self.names[span.start..][..span.len];
        (
            [row.custom_name, row.owner_name].map(bytes),
            row.single_controller,
            row.best_vp_index,
            row.marked_owners.map(bytes),
        )
    }

    fn measure(&self) -> usize {
        self.rows.capacity() * size_of::<cstate::Row>()
            + self.pointers.capacity() * size_of::<usize>()
            + self.names.capacity()
            + self.fractions.capacity() * size_of::<Fraction>()
    }
}

#[derive(Default)]
struct Hour {
    epoch: Option<u64>,
    header: Option<[u8; 16]>,
    globals: Option<cstate::Globals>,
    complete: bool,
}

#[derive(Default)]
struct Queues {
    instances: Vec<usize>,
    at: Vec<u32>,
    bytes: Vec<u8>,
    complete: Vec<bool>,
}

type HeldInfrastructure = Option<(infrastructure_map::Held, infrastructure_map::Proof)>;

impl Queues {
    fn gather(
        &mut self,
        memory: &impl Memory,
        held: &[HeldInfrastructure],
        reread: &[bool],
    ) -> bool {
        let rows = held.len();
        let grown = allocation::advisory(|| {
            self.instances.clear();
            self.at.clear();
            allocation::reserve(&mut self.instances, rows)?;
            allocation::resize(&mut self.at, rows, u32::MAX)?;
            allocation::resize(&mut self.bytes, rows * infrastructure_map::QUEUE_BYTES, 0)?;
            allocation::resize(&mut self.complete, rows, false)
        });
        if grown.is_err() {
            self.at.clear();
            return false;
        }
        for (index, entry) in held.iter().enumerate() {
            if let Some((state, _)) = entry
                && !reread[index]
                && let Some(instance) = state.instance()
            {
                self.at[index] = self.instances.len() as u32;
                self.instances.push(instance);
            }
        }
        infrastructure_map::gather_queues(
            memory,
            &self.instances,
            &mut self.bytes,
            &mut self.complete,
        );
        true
    }

    fn queue(&self, index: usize) -> Option<&infrastructure_map::Queue> {
        let at = *self.at.get(index)? as usize;
        if !*self.complete.get(at)? {
            return None;
        }
        let start = at * infrastructure_map::QUEUE_BYTES;
        self.bytes.get(start..start + infrastructure_map::QUEUE_BYTES)?.try_into().ok()
    }

    fn allocated_bytes(&self) -> usize {
        self.instances.capacity() * size_of::<usize>()
            + self.at.capacity() * size_of::<u32>()
            + self.bytes.capacity()
            + self.complete.capacity()
    }
}

pub struct StateStore {
    changes: &'static dyn Changes,
    held: Held,
    hour: Hour,
    forgotten: bool,
    bytes: usize,
    marked: Vec<u32>,
    slots: Vec<usize>,
    ids: Vec<u32>,
    manpower: Vec<cstate::Manpower>,
    categories: Vec<(u64, Option<u32>)>,
    reread: Vec<bool>,
    counts: Counts,
    carried: bool,
    infrastructure: Vec<Option<(infrastructure_map::Held, infrastructure_map::Proof)>>,
    infrastructure_cache: infrastructure_map::Cache,
    queues: Queues,
    infrastructure_bytes: usize,
}

impl StateStore {
    pub fn new(changes: &'static dyn Changes) -> Self {
        Self {
            changes,
            held: Held::default(),
            hour: Hour::default(),
            forgotten: false,
            bytes: 0,
            marked: Vec::new(),
            slots: Vec::new(),
            ids: Vec::new(),
            manpower: Vec::new(),
            categories: Vec::new(),
            reread: Vec::new(),
            counts: Counts::default(),
            carried: false,
            infrastructure: Vec::new(),
            infrastructure_cache: infrastructure_map::Cache::default(),
            queues: Queues::default(),
            infrastructure_bytes: 0,
        }
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes + self.infrastructure_bytes
    }

    pub(super) fn scripted(&self, id: u32) -> (bool, bool) {
        if self.hour.complete && self.hour.epoch.is_some() {
            crate::scripted_maps::state(id)
        } else {
            (false, false)
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub(super) fn begin(&mut self, memory: &impl Memory, base: usize, game_state: usize) {
        if std::mem::take(&mut self.forgotten) {
            self.held.clear();
            self.infrastructure.clear();
            self.infrastructure_cache.forget();
        }
        self.carried = false;
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

    fn carryable(&self) -> bool {
        self.hour.complete
            && self.held.epoch.is_some()
            && self.hour.epoch == self.held.epoch
            && self.hour.header == Some(self.held.header)
            && self.hour.globals.is_some()
            && self.hour.globals == self.held.globals
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn carry(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
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
            self.reread.clear();
            allocation::resize_default(&mut self.reread, len)?;
            allocation::resize_default(&mut self.ids, len)?;
            allocation::resize_default(&mut self.manpower, len)?;
            self.categories.clear();
            allocation::reserve(&mut self.categories, len)
        });
        scratch.ok()?;
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
        for &id in &self.marked {
            if let Ok(index) = self.ids.binary_search(&id) {
                self.reread[index] = true;
            }
        }
        if !engine_native::walk::Buffer::Growing(rows)
            .ensure(len)
            .ok()?
        {
            return None;
        }
        for (index, row) in rows[..len].iter_mut().enumerate() {
            let pointer = self.manpower[index].category;
            let held = self
                .categories
                .iter()
                .find(|(held, _)| *held == pointer)
                .map(|&(_, token)| token);
            let category = match held {
                Some(token) => token,
                None => {
                    let token = (pointer != 0)
                        .then(|| cstate::category_token(memory, base, pointer).ok())
                        .flatten();
                    self.categories.push((pointer, token));
                    token
                }
            };
            *row = cstate::Row {
                manpower_available: self.manpower[index].available,
                manpower_locked: self.manpower[index].locked,
                population: self.manpower[index].population,
                category,
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
        let walk = cstate::read_names_carried_scripted(
            memory,
            game_state,
            census,
            rows,
            engine_native::walk::Buffer::Growing(names),
            |index| (!reread[index]).then(|| held.names(index)),
            crate::scripted_maps::state,
        );
        if walk.outcome != census {
            return None;
        }
        for (&pointer, &id) in self.held.pointers.iter().zip(&self.ids) {
            let source = cstate::Source { pointer, id };
            visitor.visit(source);
        }
        visitor.finish(census);
        inputs.acquire_carried(memory, base, visitor, |index| {
            (!reread[index]).then(|| held.fractions[index])
        });
        self.count();
        self.hold(rows, names, walk, visitor, inputs);
        self.carried = true;
        Some((census, walk))
    }

    pub(super) fn infrastructure(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        census: Outcome,
        rows: &mut [cstate::Row],
    ) {
        let len = census.len();
        let rows = &mut rows[..len];
        for row in rows.iter_mut() {
            row.infrastructure = None;
        }
        let holds = self.held.epoch.is_some()
            && self.held.rows.len() == len
            && self.held.rows.iter().zip(rows.iter()).all(|(held, row)| held.id == row.id);
        let carried = self.carried && holds && self.infrastructure.len() == len;
        if !carried {
            self.infrastructure.clear();
        }
        let hour = infrastructure_map::Hour::new(
            memory,
            base,
            game_state,
            &mut self.infrastructure_cache,
            carried,
        );
        match hour {
            Err(_) => self.infrastructure.clear(),
            Ok(mut hour) if !holds => {
                for row in rows.iter_mut() {
                    row.infrastructure = hour
                        .state_of(row.id)
                        .and_then(|state| hour.fresh(state))
                        .map(|(_, proof)| proof.value())
                        .ok();
                }
            }
            Ok(mut hour) => {
                let held = &mut self.infrastructure;
                let sized = carried
                    || allocation::advisory(|| allocation::resize_default(held, len)).is_ok();
                let gathered = carried
                    && hour.proves()
                    && self.queues.gather(memory, held, &self.reread);
                for (index, row) in rows.iter_mut().enumerate() {
                    let pointer = self.held.pointers[index];
                    let kept = if carried && !self.reread[index] { held[index] } else { None };
                    let next = match kept {
                        Some((state, proof))
                            if gathered
                                && hour.quiet(
                                    &state,
                                    &proof,
                                    self.queues.queue(index),
                                    self.manpower[index].category,
                                ) =>
                        {
                            Ok((state, proof))
                        }
                        Some((state, _)) => {
                            hour.carried(pointer, state).map(|proof| (state, proof))
                        }
                        None => hour.fresh(pointer),
                    };
                    row.infrastructure = next.as_ref().ok().map(|(_, proof)| proof.value());
                    if sized {
                        held[index] = next.ok();
                    }
                }
                if !sized {
                    held.clear();
                }
            }
        }
        self.infrastructure_bytes = self.infrastructure.capacity()
            * size_of::<Option<(infrastructure_map::Held, infrastructure_map::Proof)>>()
            + self.infrastructure_cache.allocated_bytes()
            + self.queues.allocated_bytes();
    }

    fn count(&mut self) {
        for &reread in &self.reread {
            if reread {
                self.counts.read += 1;
            } else {
                self.counts.reused += 1;
            }
        }
        self.counts.quiet = self.marked.is_empty();
    }

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
            + self.categories.capacity() * size_of::<(u64, Option<u32>)>()
            + self.reread.capacity();
    }
}

impl Inputs {
    pub(super) fn complete(&self, len: usize) -> Option<&[StateFractionRow]> {
        (self.outcome == (Outcome::Rows { len })).then(|| &self.rows[..len])
    }
}
