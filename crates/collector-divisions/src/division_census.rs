use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};
use engine_native::{
    carmy,
    carmy_leader,
    ccountry,
    cequipment_variant::pool::DivisionSource,
    cgame_state,
    cprovince,
    creferenced_division_template,
    ctheatre,
};

use engine_allocation::SharedBox;

const BATCH: usize = 64;

#[cfg(test)]
mod batching_tests;
pub use engine_visibility::unit_viewer_entry::DivisionSource as Entry;

#[derive(Debug)]
pub struct UnitInputs {
    seeds: Vec<carmy::DivisionSeed>,
    outcome: Outcome,
    roster_slots: Vec<Entry>,
    intel_outcome: Outcome,
}
impl UnitInputs {
    pub fn new(capacity: usize) -> Self {
        Self {
            seeds: engine_allocation::owned_vec(engine_allocation::vec(capacity)),
            outcome: Outcome::Unavailable {
                reason: "division census not captured",
            },
            roster_slots: engine_allocation::owned_vec(engine_allocation::vec(
                capacity.saturating_mul(2),
            )),
            intel_outcome: Outcome::Unavailable {
                reason: "division census not captured",
            },
        }
    }
    pub fn seeds(&self) -> &[carmy::DivisionSeed] {
        &self.seeds
    }
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn intel_sources(&self) -> engine_visibility::unit_viewer_entry::DivisionSources<'_> {
        engine_visibility::unit_viewer_entry::DivisionSources {
            entries: &self.roster_slots,
            outcome: self.intel_outcome,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture(seeds: Vec<carmy::DivisionSeed>, outcome: Outcome) -> Self {
        let roster_slots = seeds
            .iter()
            .map(|seed| Entry {
                pointer: seed.pointer,
                canonical: seed.roster_owner.unwrap_or(0),
            })
            .collect();
        Self {
            seeds,
            outcome,
            roster_slots,
            intel_outcome: outcome,
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn empty_rows() -> Self {
        Self::fixture(Vec::new(), Outcome::Rows { len: 0 })
    }
}

pub const MAX_PARTS: usize = 4;
const PART_ENTRIES: usize = 1_500;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JoinCapacity {
    entries: usize,
    roster_slots: usize,
    countries: usize,
}

pub fn parts_for(entries: usize) -> usize {
    (entries / PART_ENTRIES).clamp(1, MAX_PARTS)
}

#[derive(Debug)]
pub struct Census {
    counts: Vec<Option<replay_data::rows::DivisionCensusRow>>,
    required_counts: usize,
    counts_error: Option<&'static str>,
    entries: Vec<Entry>,
    unit_inputs: Option<SharedBox<UnitInputs>>,
    rows: Vec<carmy::Row>,
    sources: Vec<DivisionSource>,
    census_result: Outcome,
    row_error: Option<&'static str>,
    core_root_error: Option<&'static str>,
    required: usize,
    required_slots: usize,
    pub overflowed: bool,
    core_roots: Option<CoreRoots>,
    core_captured: bool,
    slot_ends: Vec<(u32, u32)>,
    leaders: HeldLeaders,
    groups: HeldGroups,
    theatres: ctheatre::UnitTheatres,
}

#[derive(Debug, Default)]
struct HeldLeaders {
    held: std::collections::HashMap<usize, ((u64, u64), u64)>,
    next: std::collections::HashMap<usize, ((u64, u64), u64)>,
    fresh: Vec<(usize, (u64, u64))>,
}

#[derive(Debug, Default)]
struct HeldGroups {
    held: std::collections::HashMap<usize, ((u64, u64), carmy_leader::GroupFields)>,
    next: std::collections::HashMap<usize, ((u64, u64), carmy_leader::GroupFields)>,
    fresh: Vec<(usize, (u64, u64))>,
}

struct Carry<'a, L, G> {
    leaders: &'a mut HeldLeaders,
    groups: &'a mut HeldGroups,
    leader_changes: &'a L,
    group_changes: &'a G,
}

fn carried<V: Copy>(
    held: &std::collections::HashMap<usize, ((u64, u64), V)>,
    next: &mut std::collections::HashMap<usize, ((u64, u64), V)>,
    fresh: &mut Vec<(usize, (u64, u64))>,
    key: usize,
    certificate: (u64, u64),
) -> Option<V> {
    match held.get(&key) {
        Some(&(previous, value)) if previous == certificate => {
            if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut *next, 1))
                .is_ok()
            {
                next.insert(key, (certificate, value));
            }
            Some(value)
        }
        _ => {
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut *fresh, 1)).is_ok()
            {
                fresh.push((key, certificate));
            }
            None
        }
    }
}

impl<L, G> carmy_leader::SelectionCarry for Carry<'_, L, G>
where
    L: collector_observers::unit_leader_changes::Changes,
    G: collector_observers::orders_group_changes::Changes,
{
    fn leader(&mut self, leader: usize) -> Option<u64> {
        let certificate = self.leader_changes.sequence(leader)?;
        let HeldLeaders { held, next, fresh } = &mut *self.leaders;
        carried(held, next, fresh, leader, certificate)
    }
    fn group(&mut self, group: usize) -> Option<carmy_leader::GroupFields> {
        let certificate = self.group_changes.sequence(group)?;
        let HeldGroups { held, next, fresh } = &mut *self.groups;
        carried(held, next, fresh, group, certificate)
    }
    fn group_read(&mut self, group: usize, fields: carmy_leader::GroupFields) {
        let HeldGroups { next, fresh, .. } = &mut *self.groups;
        if let Ok(at) = fresh.binary_search_by_key(&group, |entry| entry.0)
            && engine_allocation::advisory(|| engine_allocation::reserve_map(&mut *next, 1)).is_ok()
        {
            next.insert(group, (fresh[at].1, fields));
        }
    }
}

fn select(
    memory: &impl Memory,
    base: usize,
    seeds: &mut [carmy::DivisionSeed],
    leaders: &mut HeldLeaders,
    groups: &mut HeldGroups,
    leader_changes: &impl collector_observers::unit_leader_changes::Changes,
    group_changes: &impl collector_observers::orders_group_changes::Changes,
) {
    leaders.next.clear();
    leaders.fresh.clear();
    groups.next.clear();
    groups.fresh.clear();
    let result = carmy_leader::capture_selections(
        memory,
        base,
        seeds,
        &mut Carry {
            leaders: &mut *leaders,
            groups: &mut *groups,
            leader_changes,
            group_changes,
        },
    );
    let Ok(counts) = result else {
        return;
    };
    let HeldLeaders { held, next, fresh } = leaders;
    fresh.sort_unstable_by_key(|entry| entry.0);
    for selected in seeds.iter().filter_map(|seed| seed.selected) {
        if let (Some(Ok(leader)), Some(handle)) = (selected.leader, selected.handle)
            && let Ok(at) = fresh.binary_search_by_key(&leader, |entry| entry.0)
            && engine_allocation::advisory(|| engine_allocation::reserve_map(&mut *next, 1)).is_ok()
        {
            next.insert(leader, (fresh[at].1, handle));
        }
    }
    std::mem::swap(held, next);
    std::mem::swap(&mut groups.held, &mut groups.next);
    collector_observers::orders_group_changes::record(
        &collector_observers::orders_group_changes::Counts {
            carried: counts.carried_groups,
            read: counts.groups - counts.carried_groups,
        },
    );
}

impl Default for Census {
    fn default() -> Self {
        Self {
            counts: Vec::new(),
            required_counts: 0,
            counts_error: None,
            entries: Vec::new(),
            unit_inputs: None,
            rows: Vec::new(),
            sources: Vec::new(),
            census_result: Outcome::Unavailable {
                reason: "division census not captured",
            },
            row_error: None,
            core_root_error: None,
            required: 0,
            required_slots: 0,
            overflowed: false,
            core_roots: None,
            core_captured: false,
            slot_ends: Vec::new(),
            leaders: HeldLeaders::default(),
            groups: HeldGroups::default(),
            theatres: ctheatre::UnitTheatres::default(),
        }
    }
}
impl Census {
    pub fn new_worker() -> Self {
        Self {
            unit_inputs: engine_allocation::shared(UnitInputs::new(0)).ok(),
            ..Self::default()
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn new() -> Self {
        Self {
            counts: engine_allocation::owned_vec(engine_allocation::default_vec(
                collector_primitives::country_slots::census_rows(512),
            )),
            entries: engine_allocation::owned_vec(engine_allocation::vec(8192)),
            unit_inputs: engine_allocation::shared(UnitInputs::new(8192)).ok(),
            rows: engine_allocation::owned_vec(engine_allocation::vec(8192)),
            sources: engine_allocation::owned_vec(engine_allocation::vec(8192)),
            ..Self::default()
        }
    }
    pub fn clear(&mut self) {
        self.counts.fill(None);
        self.required_counts = 0;
        self.counts_error = None;
        self.entries.clear();
        if let Some(shared) = &mut self.unit_inputs {
            let inputs = SharedBox::get_mut(shared)
                .expect("prior division input leases released before reuse");
            inputs.seeds.clear();
            inputs.roster_slots.clear();
            inputs.outcome = Outcome::Unavailable {
                reason: "division census not captured",
            };
            inputs.intel_outcome = inputs.outcome;
        }
        self.rows.clear();
        self.sources.clear();
        self.census_result = Outcome::Unavailable {
            reason: "division census not captured",
        };
        self.row_error = None;
        self.core_root_error = None;
        self.required = 0;
        self.required_slots = 0;
        self.overflowed = false;
        self.core_roots = None;
        self.core_captured = false;
        self.slot_ends.clear();
        self.theatres.clear();
    }
    pub fn allocated_bytes(&self) -> u64 {
        collector_primitives::raw_memory::vector_bytes(&self.counts)
            + collector_primitives::raw_memory::vector_bytes(&self.entries)
            + self.unit_inputs.as_ref().map_or(0, |inputs| {
                collector_primitives::raw_memory::vector_bytes(&inputs.seeds)
                    + collector_primitives::raw_memory::vector_bytes(&inputs.roster_slots)
            })
            + collector_primitives::raw_memory::vector_bytes(&self.rows)
            + collector_primitives::raw_memory::vector_bytes(&self.sources)
            + collector_primitives::raw_memory::vector_bytes(&self.slot_ends)
            + self.theatres.allocated_bytes()
    }
    pub fn country_count(&self) -> usize {
        self.required_counts
    }
    pub fn counts_need_growth(&self) -> bool {
        self.required_counts > self.counts.len()
    }
    pub fn initialized(&self) -> bool {
        self.entries.capacity() != 0
    }
    pub fn source_requirement(&self) -> usize {
        self.required.max(self.sources.len())
    }
    pub fn unit_inputs(&self) -> Option<&SharedBox<UnitInputs>> {
        self.unit_inputs.as_ref()
    }
    pub fn grow(&mut self) {
        let counts = self
            .required_counts
            .max(collector_primitives::country_slots::census_rows(
                self.counts.len(),
            ));
        if !collector_primitives::table_capture::grow_vec(&mut self.counts, counts) {
            return;
        }
        let target = self.required;
        let additional = target.saturating_sub(self.entries.len());
        if engine_allocation::reserve_exact(&mut self.entries, additional).is_err() {
            return;
        }
        if let Some(shared) = &mut self.unit_inputs {
            let inputs =
                SharedBox::get_mut(shared).expect("division input leases released before growth");
            let additional = target.saturating_sub(inputs.seeds.len());
            if engine_allocation::reserve_exact(&mut inputs.seeds, additional).is_err() {
                return;
            }
            let slots = self.required_slots;
            let additional = slots.saturating_sub(inputs.roster_slots.len());
            if engine_allocation::reserve_exact(&mut inputs.roster_slots, additional).is_err() {
                return;
            }
        } else {
            let Ok(inputs) = engine_allocation::shared(UnitInputs::new(target)) else {
                return;
            };
            self.unit_inputs = Some(inputs);
        }
        let additional = target.saturating_sub(self.rows.len());
        if engine_allocation::reserve_exact(&mut self.rows, additional).is_err() {
            return;
        }
        let additional = target.saturating_sub(self.sources.len());
        let _ = engine_allocation::reserve_exact(&mut self.sources, additional);
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn reserve_sources(&mut self, capacity: usize) {
        if self.entries.capacity() < capacity {
            self.required = self.required.max(capacity);
            self.grow();
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn census_outcome(&self) -> Outcome {
        self.census_result
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn intel_sources(&self) -> engine_visibility::unit_viewer_entry::DivisionSources<'_> {
        self.unit_inputs.as_ref().map_or(
            engine_visibility::unit_viewer_entry::DivisionSources {
                entries: &[],
                outcome: self.census_result,
            },
            |inputs| inputs.intel_sources(),
        )
    }
    pub fn sources(&self) -> (&[DivisionSource], Outcome) {
        let outcome = self.outcome();
        (&self.sources[..outcome.len()], outcome)
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn rows(&self) -> (&[carmy::Row], Outcome) {
        let outcome = self.outcome();
        (&self.rows[..outcome.len()], outcome)
    }
    pub fn install_rows(&self, output: &mut Vec<carmy::Row>) -> Outcome {
        let outcome = self.outcome();
        let len = outcome.len();
        if let Err(reason) = engine_native_memory::walk::Buffer::Growing(output).ensure(len) {
            return Outcome::Unavailable { reason };
        }
        output[..len].copy_from_slice(&self.rows[..len]);
        outcome
    }
    pub fn outcome(&self) -> Outcome {
        if self.overflowed {
            return Outcome::Unavailable {
                reason: "division owned census capacity exhausted",
            };
        }
        if let Outcome::Unavailable { reason } = self.census_result {
            return Outcome::Unavailable { reason };
        }
        if let Some(reason) = self.core_root_error {
            return Outcome::Unavailable { reason };
        }
        if !self.core_captured {
            return Outcome::Unavailable {
                reason: "division core not captured",
            };
        }
        if self.rows.len() < self.entries.len() {
            return Outcome::Unavailable {
                reason: self.row_error.unwrap_or("division owned row not captured"),
            };
        }
        Outcome::Rows {
            len: self.entries.len(),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.capture_inputs(memory, base, state);
        self.capture_core(memory);
    }
    pub fn capture_inputs(&mut self, memory: &impl Memory, base: usize, state: usize) {
        self.capture_inputs_mode(memory, base, state, None);
    }
    pub fn capture_inputs_part(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        slots: (u32, u32),
    ) {
        self.capture_inputs_mode(memory, base, state, Some(slots));
    }
    fn capture_inputs_mode(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        slots: Option<(u32, u32)>,
    ) {
        self.clear();
        if self.unit_inputs.is_none() {
            self.unit_inputs = engine_allocation::shared(UnitInputs::new(0)).ok();
        }
        let result = self.capture_inputs_inner(memory, base, state, slots);
        if let Err(reason) = result {
            if matches!(self.census_result, Outcome::Rows { .. }) {
                self.row_error = Some(reason);
            } else {
                self.census_result = Outcome::Unavailable { reason };
            }
            if let Some(inputs) = self.unit_inputs.as_mut().and_then(SharedBox::get_mut)
                && !matches!(inputs.outcome, Outcome::Rows { .. })
            {
                inputs.outcome = Outcome::Unavailable { reason };
            }
            if let Some(inputs) = self.unit_inputs.as_mut().and_then(SharedBox::get_mut)
                && !matches!(inputs.intel_outcome, Outcome::Rows { .. })
            {
                inputs.intel_outcome = Outcome::Unavailable { reason };
            }
        }
    }
    pub fn absorb(&mut self, part: &mut Census) -> Result<(), &'static str> {
        if !self.joinable() {
            return Ok(());
        }
        self.required = self.required.saturating_add(part.required);
        self.required_slots = self.required_slots.saturating_add(part.required_slots);
        let inputs = self
            .unit_inputs
            .as_mut()
            .and_then(SharedBox::get_mut)
            .ok_or("division input packet still leased")?;
        let part_inputs = part
            .unit_inputs
            .as_ref()
            .ok_or("division input part not captured")?;
        if part.overflowed
            || part.row_error.is_some()
            || !matches!(part.census_result, Outcome::Rows { .. })
        {
            self.overflowed = part.overflowed;
            self.census_result = part.census_result;
            self.row_error = part.row_error;
            inputs.outcome = part_inputs.outcome;
            inputs.intel_outcome = part_inputs.intel_outcome;
            self.core_roots = None;
            return Ok(());
        }
        let offset = self.entries.len() as u32;
        engine_allocation::reserve(&mut self.entries, part.entries.len())
            .map_err(|_| "division census allocation failed")?;
        engine_allocation::reserve(&mut inputs.seeds, part_inputs.seeds.len())
            .map_err(|_| "division seed allocation failed")?;
        engine_allocation::reserve(&mut inputs.roster_slots, part_inputs.roster_slots.len())
            .map_err(|_| "division roster allocation failed")?;
        self.entries.extend_from_slice(&part.entries);
        inputs.seeds.extend_from_slice(&part_inputs.seeds);
        inputs
            .roster_slots
            .extend_from_slice(&part_inputs.roster_slots);
        for (count, theirs) in self.counts.iter_mut().zip(&part.counts) {
            if theirs.is_some() {
                *count = theirs.clone();
            }
        }
        self.counts_error = self.counts_error.or(part.counts_error);
        if engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.slot_ends, part.slot_ends.len())
        })
        .is_ok()
        {
            self.slot_ends.extend(
                part.slot_ends
                    .iter()
                    .map(|&(slot, end)| (slot, end + offset)),
            );
        }
        self.census_result = Outcome::Rows {
            len: self.entries.len(),
        };
        inputs.outcome = self.census_result;
        inputs.intel_outcome = Outcome::Rows {
            len: inputs.roster_slots.len(),
        };
        if let (Some(roots), Some(theirs)) = (self.core_roots.as_mut(), part.core_roots) {
            roots.found |= theirs.found;
        }
        Ok(())
    }

    pub fn joinable(&self) -> bool {
        matches!(self.census_result, Outcome::Rows { .. })
            && !self.overflowed
            && self.row_error.is_none()
    }

    pub fn join_capacity(&self) -> JoinCapacity {
        JoinCapacity {
            entries: self.entries.len().saturating_mul(2),
            roster_slots: self
                .unit_inputs
                .as_ref()
                .map_or(0, |inputs| inputs.roster_slots.len())
                .saturating_mul(2),
            countries: self.slot_ends.len().saturating_mul(2),
        }
    }

    pub fn reserve_join(&mut self, capacity: JoinCapacity) {
        let entries = capacity.entries.saturating_sub(self.entries.len());
        let countries = capacity.countries.saturating_sub(self.slot_ends.len());
        let _ = engine_allocation::advisory(|| {
            engine_allocation::reserve(&mut self.entries, entries)?;
            engine_allocation::reserve(&mut self.slot_ends, countries)?;
            if let Some(inputs) = self.unit_inputs.as_mut().and_then(SharedBox::get_mut) {
                let seeds = capacity.entries.saturating_sub(inputs.seeds.len());
                let slots = capacity
                    .roster_slots
                    .saturating_sub(inputs.roster_slots.len());
                engine_allocation::reserve(&mut inputs.seeds, seeds)?;
                engine_allocation::reserve(&mut inputs.roster_slots, slots)?;
            }
            Ok::<(), std::io::Error>(())
        });
    }

    pub fn part_starts(&self, starts: &mut [u32; MAX_PARTS - 1]) -> usize {
        let total = self.entries.len() as u64;
        let parts = parts_for(self.entries.len()) as u64;
        let mut filled = 0;
        for part in 1..parts {
            let target = total * part / parts;
            let at = self
                .slot_ends
                .partition_point(|&(_, end)| u64::from(end) <= target);
            if let Some(&(start, _)) = self.slot_ends.get(at)
                && start > 1
                && (filled == 0 || start > starts[filled - 1])
            {
                starts[filled] = start;
                filled += 1;
            }
        }
        filled
    }

    pub fn capture_core(&mut self, memory: &impl Memory) {
        if self.core_captured {
            return;
        }
        let Some(roots) = self.core_roots.take() else {
            return;
        };
        let result = self.capture_core_inner(memory, roots);
        if let Err(reason) = result {
            self.row_error = Some(reason);
        }
        self.core_captured = true;
    }
    fn capture_inputs_inner(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        slots: Option<(u32, u32)>,
    ) -> Result<(), &'static str> {
        let reader = ccountry::Reader::new(memory, base, state)?;
        let [countries, links] = reader.arrays();
        self.required_counts = links.count as usize;
        engine_native_memory::walk::Buffer::Growing(&mut self.counts).ensure(self.required_counts)?;

        let countries = Countries { countries, links };
        let mut found = false;
        let mut counting = false;
        let mut scalar_capacity_failed = false;
        let mut pointers = [0usize; BATCH];
        let mut pointer_bytes = [0u8; BATCH * 8];
        const POINTER: [RecordField; 1] = [RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        }];
        const POINTER_LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&POINTER, 8).expect("valid division pointer layout");
        let (first_slot, end_slot) = slots.unwrap_or((1, reader.count()));
        for slot in first_slot.max(1)..end_slot.min(reader.count()) {
            let Some(country) = reader.at(slot).map_err(|reason| match reason {
                "country pointer unreadable or outside country table" => {
                    "country pointer unreadable"
                }
                other => other,
            })?
            else {
                continue;
            };
            let (pointer, canonical) = (country.pointer, country.canonical_slot);
            if let Some(count) = self.counts.get_mut(country.id as usize) {
                *count = Some(replay_data::rows::DivisionCensusRow {
                    country: country.id,
                    ..Default::default()
                });
            } else {
                self.counts_error = Some("division coverage country capacity exhausted");
            }
            found = true;
            let roster = self.array(
                memory,
                pointer,
                ccountry::DIVISIONS,
                8,
                "division roster unreadable or invalid",
            )?;
            self.required_slots = self.required_slots.saturating_add(roster.count as usize);
            let inputs = self
                .unit_inputs
                .as_mut()
                .and_then(SharedBox::get_mut)
                .ok_or("division input packet still leased")?;
            engine_allocation::reserve(&mut inputs.roster_slots, roster.count as usize)
                .map_err(|_| "division roster allocation failed")?;
            engine_allocation::reserve(&mut inputs.seeds, roster.count as usize)
                .map_err(|_| "division seed allocation failed")?;
            engine_allocation::reserve(&mut self.entries, roster.count as usize)
                .map_err(|_| "division census allocation failed")?;

            if counting {
                self.required = self.required.saturating_add(roster.count as usize);
                continue;
            }
            'roster: for first in (0..roster.count).step_by(BATCH) {
                let count = ((roster.count - first) as usize).min(BATCH);
                for (i, p) in pointers.iter_mut().take(count).enumerate() {
                    *p = roster
                        .element(first + i as u32, 8)
                        .ok_or("division roster pointer unreadable")?;
                }
                let copied = memory.copy_records(
                    RecordSources::Pointers(&pointers[..count]),
                    &POINTER_LAYOUT,
                    &mut pointer_bytes,
                );
                let Some(inputs) = self.unit_inputs.as_mut().and_then(SharedBox::get_mut) else {
                    self.overflowed = true;
                    return Err("division owned roster capacity exhausted");
                };
                for i in 0..count {
                    let raw = if i < copied {
                        u64::from_le_bytes(pointer_bytes[i * 8..i * 8 + 8].try_into().unwrap())
                    } else {
                        read::<u64>(memory, pointers[i]).ok_or("division roster pointer unreadable")?
                    };
                    let pointer = usize::try_from(raw)
                        .map_err(|_| "division roster pointer unreadable")?;
                    if inputs.roster_slots.len() == inputs.roster_slots.capacity() {
                        let unread = roster.count as usize - (first as usize + i);
                        self.required = self.required.saturating_add(unread);
                        counting = true;
                        break 'roster;
                    }
                    inputs.roster_slots.push(Entry { pointer, canonical });
                    if pointer == 0 {
                        continue;
                    }
                    self.required += 1;
                    if self.entries.len() == self.entries.capacity() {
                        self.overflowed = true;
                    } else {
                        self.entries.push(Entry { pointer, canonical });
                        if inputs.seeds.len() == inputs.seeds.capacity() {
                            scalar_capacity_failed = true;
                        } else {
                            inputs.seeds.push(carmy::DivisionSeed {
                                pointer,
                                country_ordinal: Some(first + i as u32),
                                ..Default::default()
                            });
                        }
                    }
                }
            }
            if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.slot_ends, 1))
                .is_ok()
            {
                self.slot_ends.push((slot, self.entries.len() as u32));
            }
        }
        if counting {
            self.overflowed = true;
            return Err("division owned roster capacity exhausted");
        }
        if self.overflowed {
            return Err("division owned census capacity exhausted");
        }
        self.census_result = Outcome::Rows {
            len: self.entries.len(),
        };
        let Some(shared) = &mut self.unit_inputs else {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        };
        let inputs =
            SharedBox::get_mut(shared).expect("division inputs uniquely owned during acquisition");
        inputs.intel_outcome = Outcome::Rows {
            len: inputs.roster_slots.len(),
        };
        if scalar_capacity_failed || self.entries.len() > inputs.seeds.capacity() {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        carmy::capture_division_seeds(memory, &mut inputs.seeds);
        for seed in &mut inputs.seeds {
            seed.division = seed.handle.unwrap_or(0);
        }
        carmy::capture_division_movers(memory, &mut inputs.seeds);
        select(
            memory,
            base,
            &mut inputs.seeds,
            &mut self.leaders,
            &mut self.groups,
            &collector_observers::unit_leader_changes::Observer,
            &collector_observers::orders_group_changes::Observer,
        );
        inputs.outcome = self.census_result;
        self.core_roots = Some(CoreRoots {
            base,
            state,
            countries,
            found,
        });
        Ok(())
    }
    fn capture_core_inner(
        &mut self,
        memory: &impl Memory,
        roots: CoreRoots,
    ) -> Result<(), &'static str> {
        let CoreRoots {
            base,
            state,
            countries,
            found,
        } = roots;
        if !found {
            self.core_root_error = Some("country table has no readable countries");
            return Ok(());
        }
        let provinces = match self.array(
            memory,
            state,
            cgame_state::PROVINCES,
            8,
            "province array unreadable or invalid",
        ) {
            Ok(provinces) if provinces.count >= 2 => provinces,
            Ok(_) => {
                self.core_root_error = Some("province table has no province slots");
                return Ok(());
            }
            Err(reason) => {
                self.core_root_error = Some(reason);
                return Ok(());
            }
        };
        let take = self.entries.len();
        engine_allocation::reserve(&mut self.rows, take)
            .map_err(|_| "division row allocation failed")?;
        engine_allocation::reserve(&mut self.sources, take)
            .map_err(|_| "division source allocation failed")?;

        if take > self.rows.capacity() || take > self.sources.capacity() {
            self.overflowed = true;
            return Err("division owned scalar capacity exhausted");
        }
        let inputs = self.unit_inputs.as_ref().unwrap();
        let mut identities = CoreIdentities::new(base, provinces);
        let mut theatres = TheatreLookup::new(&mut self.theatres);
        for (index, (entry, seed)) in self
            .entries
            .iter()
            .zip(&inputs.seeds)
            .take(take)
            .enumerate()
        {
            if index.is_multiple_of(64) {
                identities.stage_provinces(memory, &inputs.seeds[index..(index + 64).min(take)]);
            }
            const LOOKAHEAD: usize = 4;
            if index.is_multiple_of(LOOKAHEAD) {
                for pending in inputs
                    .seeds
                    .iter()
                    .skip(index)
                    .take((take - index).min(LOOKAHEAD))
                {
                    for pointer in [pending.standard_template, pending.orders_group]
                        .into_iter()
                        .flatten()
                        .filter(|&pointer| pointer != 0)
                    {
                        memory.prefetch(pointer, 16);
                    }
                    if let Some(skip) = pending
                        .standard_template
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| {
                            pointer.checked_add(creferenced_division_template::REVEAL_SKIP)
                        })
                    {
                        memory.prefetch(skip, 1);
                    }
                }
            }
            let row = project(
                memory,
                &countries,
                *entry,
                seed,
                &mut identities,
                &mut theatres,
            )?;
            Self::count_division(
                &mut self.counts,
                self.required_counts,
                &mut self.counts_error,
                &row,
            );
            self.sources.push(DivisionSource {
                pointer: entry.pointer,
                division: row.id,
                template: row.template,
                country: row.roster_owner,
            });
            self.rows.push(row);
        }
        Ok(())
    }
    fn count_division(
        counts: &mut [Option<replay_data::rows::DivisionCensusRow>],
        country_count: usize,
        counts_error: &mut Option<&'static str>,
        row: &carmy::Row,
    ) {
        let owner = if (row.expeditionary_owner as i32) > 0 {
            row.expeditionary_owner
        } else {
            row.roster_owner
        };
        if owner as usize >= country_count {
            *counts_error = Some("division owner outside country links");
        } else if let Some(count) = counts.get_mut(owner as usize) {
            let count = count.get_or_insert_with(|| replay_data::rows::DivisionCensusRow {
                country: owner,
                ..Default::default()
            });
            let value = if row.orders_group == 0 {
                &mut count.ungrouped
            } else {
                &mut count.grouped
            };
            if let Some(next) = value.checked_add(1) {
                *value = next;
            } else {
                *counts_error = Some("division coverage count overflow");
            }
        } else {
            *counts_error = Some("division coverage country capacity exhausted");
        }
    }
    pub fn install_counts(&self, output: &mut [replay_data::rows::DivisionCensusRow]) -> Outcome {
        if let Some(reason) = self.counts_error {
            return Outcome::Unavailable { reason };
        }
        if !matches!(self.outcome(), Outcome::Rows { .. }) {
            return Outcome::Unavailable {
                reason: "division coverage census incomplete",
            };
        }
        let mut len = 0;
        let mut missing = 0;
        for count in self.counts.iter().flatten() {
            if let Some(row) = output.get_mut(len) {
                *row = count.clone();
                len += 1;
            } else {
                missing += 1;
            }
        }
        complete(len, missing)
    }
    fn array(
        &mut self,
        memory: &impl Memory,
        pointer: usize,
        offset: usize,
        stride: usize,
        reason: &'static str,
    ) -> Result<Array, &'static str> {
        let bytes = pointer
            .checked_add(offset)
            .and_then(|p| read::<[u8; 16]>(memory, p))
            .ok_or(reason)?;
        Array::from_header(bytes, stride).ok_or(reason)
    }
}

#[derive(Debug, Clone, Copy)]
struct CoreRoots {
    base: usize,
    state: usize,
    countries: Countries,
    found: bool,
}

#[derive(Debug, Clone, Copy)]
struct Countries {
    countries: Array,
    links: Array,
}
impl Countries {
    fn pointer(&self, memory: &impl Memory, slot: u32) -> Result<usize, &'static str> {
        self.countries
            .element(slot, 8)
            .and_then(|p| read::<u64>(memory, p))
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("country pointer unreadable")
    }

    fn canonical(&self, memory: &impl Memory, raw: u32) -> Result<u32, &'static str> {
        if raw == 0 {
            return Err("country id is zero");
        }
        self.links
            .element(raw, 4)
            .and_then(|p| read::<u32>(memory, p))
            .filter(|&slot| slot > 0 && slot < self.countries.count)
            .ok_or("country link unreadable or outside country table")
    }
}

fn complete(len: usize, missing: usize) -> Outcome {
    if missing == 0 {
        Outcome::Rows { len }
    } else {
        match u32::try_from(missing) {
            Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
            Err(_) => Outcome::Unavailable {
                reason: "division unread row count overflow",
            },
        }
    }
}

const CORE_IDENTITY_SLOTS: usize = 256;

#[derive(Clone, Copy, Default)]
struct CoreIdentity {
    pointer: usize,
    value: u64,
}

const PROVINCE_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cprovince::ID,
        destination_offset: 8,
        len: 4,
    },
];
const PROVINCE_IDENTITY: usize = 12;
const PROVINCE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PROVINCE_FIELDS, PROVINCE_IDENTITY).expect("valid province identity layout");
const TEMPLATE_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: creferenced_division_template::TYPE,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: creferenced_division_template::REVEAL_SKIP,
        destination_offset: 16,
        len: 1,
    },
];
const TEMPLATE_IDENTITY: usize = 17;
const TEMPLATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TEMPLATE_FIELDS, TEMPLATE_IDENTITY).expect("valid template identity layout");

struct CoreIdentities {
    base: usize,
    provinces: Array,
    locations: [CoreIdentity; CORE_IDENTITY_SLOTS],
    province_batch: cprovince::IdentityBatch,
    templates: [CoreIdentity; CORE_IDENTITY_SLOTS],
    template_skips: [u8; CORE_IDENTITY_SLOTS],
    groups: [CoreIdentity; CORE_IDENTITY_SLOTS],
}

impl CoreIdentities {
    fn new(base: usize, provinces: Array) -> Self {
        Self {
            base,
            provinces,
            locations: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
            province_batch: cprovince::IdentityBatch::default(),
            templates: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
            template_skips: [0; CORE_IDENTITY_SLOTS],
            groups: [CoreIdentity::default(); CORE_IDENTITY_SLOTS],
        }
    }

    fn slot(pointer: usize) -> usize {
        collector_primitives::table_hash::slot((pointer >> 4) as u64, CORE_IDENTITY_SLOTS)
    }

    fn stage_provinces(&mut self, memory: &impl Memory, seeds: &[carmy::DivisionSeed]) {
        let mut pointers = [0_usize; 64];
        let mut len = 0;
        for seed in seeds {
            if let Some(pointer) = seed.location.filter(|&at| at != 0)
                && self.locations[Self::slot(pointer)].pointer != pointer
                && !pointers[..len].contains(&pointer)
            {
                pointers[len] = pointer;
                len += 1;
            }
        }
        self.province_batch
            .capture(memory, self.base, self.provinces, &pointers[..len], |_| {
                true
            });
    }

    fn province(&mut self, memory: &impl Memory, location: usize) -> Result<u32, &'static str> {
        if location == 0 {
            return Ok(0);
        }
        let slot = Self::slot(location);
        if self.locations[slot].pointer == location {
            return Ok(self.locations[slot].value as u32);
        }
        let mut identity = [0_u8; PROVINCE_IDENTITY];
        let gathered = match self.province_batch.identity(location) {
            Some((vtable, id)) => {
                identity[..8].copy_from_slice(&vtable.to_le_bytes());
                identity[8..].copy_from_slice(&id.to_le_bytes());
                PROVINCE_FIELDS.len()
            }
            None => memory.copy_records(
                RecordSources::Pointers(&[location]),
                &PROVINCE_LAYOUT,
                &mut identity,
            ),
        };
        if gathered == 0
            || Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                != engine_sites::at(self.base, cprovince::VTABLE_RVA).map(|p| p as u64)
        {
            return Err("division province vtable mismatch");
        }
        if gathered < PROVINCE_FIELDS.len() {
            return Err("division province id unreadable");
        }
        let id = u32::from_le_bytes(identity[8..].try_into().unwrap());
        if id == 0 {
            return Err("non-null division province has zero id");
        }
        let registered = self
            .province_batch
            .slot(id)
            .map(|pointer| pointer as u64)
            .or_else(|| {
                self.provinces
                    .element(id, 8)
                    .and_then(|p| read::<u64>(memory, p))
            })
            .ok_or("division province slot unreadable or out of range")?;
        if registered != location as u64 {
            return Err("division province differs from registered slot");
        }
        self.locations[slot] = CoreIdentity {
            pointer: location,
            value: id as u64,
        };
        Ok(id)
    }

    fn template(
        &mut self,
        memory: &impl Memory,
        template_pointer: usize,
    ) -> Result<(u64, u8), &'static str> {
        if template_pointer == 0 {
            return Ok((0, 0));
        }
        let slot = Self::slot(template_pointer);
        if self.templates[slot].pointer == template_pointer {
            let value = self.templates[slot].value;
            return Ok((value, self.template_skips[slot]));
        }
        let mut identity = [0_u8; TEMPLATE_IDENTITY];
        let gathered = memory.copy_records(
            RecordSources::Pointers(&[template_pointer]),
            &TEMPLATE_LAYOUT,
            &mut identity,
        );
        if gathered == 0
            || Some(u64::from_le_bytes(identity[..8].try_into().unwrap()))
                != engine_sites::at(self.base, creferenced_division_template::VTABLE_RVA)
                    .map(|p| p as u64)
        {
            return Err("division template vtable mismatch");
        }
        if gathered < TEMPLATE_FIELDS.len() {
            return Err("division template handle unreadable");
        }
        let handle = u64::from_le_bytes(identity[8..16].try_into().unwrap());
        let skip = identity[16];
        if handle == 0 {
            return Err("non-null division template has null handle");
        }
        self.templates[slot] = CoreIdentity {
            pointer: template_pointer,
            value: handle,
        };
        self.template_skips[slot] = skip;
        Ok((handle, skip))
    }

    fn group(&mut self, memory: &impl Memory, group: usize) -> Result<u64, &'static str> {
        if group == 0 {
            return Ok(0);
        }
        let slot = Self::slot(group);
        if self.groups[slot].pointer == group {
            return Ok(self.groups[slot].value);
        }
        let identity =
            read::<[u8; 16]>(memory, group).ok_or("division orders group identity unreadable")?;
        let vtable = u64::from_le_bytes(identity[..8].try_into().unwrap());
        let normal = engine_sites::at(self.base, engine_native::corders_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let army = engine_sites::at(self.base, engine_native_parts::carmy_group::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        if vtable != normal && vtable != army {
            return Err("division orders group vtable mismatch");
        }
        let handle = u64::from_le_bytes(identity[8..].try_into().unwrap());
        if handle as u32 != engine_native::corders_group::REFERENCE_TYPE {
            return Err("division orders group reference type mismatch");
        }
        self.groups[slot] = CoreIdentity {
            pointer: group,
            value: handle,
        };
        Ok(handle)
    }
}

struct TheatreLookup<'a> {
    theatres: &'a mut ctheatre::UnitTheatres,
    country: Option<(u32, usize)>,
}

impl<'a> TheatreLookup<'a> {
    fn new(theatres: &'a mut ctheatre::UnitTheatres) -> Self {
        theatres.clear();
        Self {
            theatres,
            country: None,
        }
    }

    fn handle(
        &mut self,
        memory: &impl Memory,
        base: usize,
        countries: &Countries,
        canonical: u32,
        pointer: Option<usize>,
    ) -> Result<u64, &'static str> {
        let pointer = pointer.ok_or("division theatre pointer unreadable")?;
        if pointer == 0 {
            return Ok(0);
        }
        let held = match self.country {
            Some((slot, held)) if slot == canonical => held,
            _ => {
                let held = countries.pointer(memory, canonical)?;
                self.country = Some((canonical, held));
                held
            }
        };
        self.theatres.handle(memory, base, held, pointer)
    }
}

fn project(
    memory: &impl Memory,
    countries: &Countries,
    entry: Entry,
    scalar: &carmy::DivisionSeed,
    identities: &mut CoreIdentities,
    theatres: &mut TheatreLookup<'_>,
) -> Result<carmy::Row, &'static str> {
    let primary = engine_sites::at(identities.base, carmy::VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    let secondary = engine_sites::at(identities.base, carmy::SECONDARY_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    if scalar
        .primary_vtable
        .ok_or("division primary vtable mismatch")?
        != primary
    {
        return Err("division primary vtable mismatch");
    }
    if scalar
        .secondary_vtable
        .ok_or("division secondary vtable mismatch")?
        != secondary
    {
        return Err("division secondary vtable mismatch");
    }
    let id = scalar.handle.ok_or("division handle unreadable")?;
    if id == 0 {
        return Err("non-null division has null handle");
    }
    let roster_owner = scalar
        .roster_owner
        .ok_or("division roster owner unreadable")?;
    if countries.canonical(memory, roster_owner)? != entry.canonical {
        return Err("division owner differs from roster country");
    }
    let logical_owner = scalar
        .logical_owner
        .ok_or("division logical owner unreadable")?;
    let expeditionary_owner = scalar
        .expeditionary_owner
        .ok_or("division expeditionary owner unreadable")?;
    let location = scalar
        .location
        .ok_or("division location pointer unreadable")?;
    let province = identities.province(memory, location)?;
    let template_pointer = scalar
        .standard_template
        .ok_or("division template pointer unreadable")?;
    let (template, reveal_skip) = identities.template(memory, template_pointer)?;
    let hit_points = scalar.strength.ok_or("division hit points unreadable")?;
    let organisation = scalar
        .organisation
        .ok_or("division organisation unreadable")?;
    let group = scalar
        .orders_group
        .ok_or("division orders group pointer unreadable")?;
    let orders_group = identities.group(memory, group)?;
    let theatre = theatres
        .handle(
            memory,
            identities.base,
            countries,
            entry.canonical,
            scalar.theatre,
        )
        .ok();
    Ok(carmy::Row {
        id,
        roster_owner,
        logical_owner,
        expeditionary_owner,
        province,
        template,
        hit_points,
        organisation,
        orders_group,
        reveal_skip,
        theatre,
    })
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod coverage_tests;
