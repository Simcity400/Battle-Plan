//! Retained naval results: primary snapshots and hit lists.
//! Original-pass growth runs only in the Ordinary worker; projection is deferred.
use crate::{
    extended::naval_details::{air_hit, naval_hit},
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity},
};
use engine::allocation::{collect_vec, copy_vec, owned_vec, reserve_map, resize_default};
use engine::{
    Memory, cnaval_combat_air_entry as plane, cnaval_combat_result_side as side,
    cnaval_combat_results as result, cnaval_combat_ship_entry as ship, naval_combat_air_hit,
    naval_combat_hit, naval_commander_inputs as commander, naval_ship_modifier_inputs as influence,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
use std::collections::HashMap;

pub const N: usize = 1;
pub const TABLES: [TableId; N] = [TableId::NavalCombatResultDetails];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("retained naval result detail census", 0, &[0])];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    results: usize,
    ships: usize,
    planes: usize,
    air: usize,
    hits: usize,
    sources: usize,
    terms: usize,
    commanders: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            results: 128,
            ships: 2048,
            planes: 256,
            air: 2048,
            hits: 4096,
            sources: 1024,
            terms: 4096,
            commanders: 32,
        }
    }
}
#[derive(Default, Clone, PartialEq, Eq)]
struct Side {
    details: side::DetailsRow,
    ships: [usize; 2],
    planes: [usize; 2],
    hq_modifiers: [usize; 2],
    commanders: [Option<usize>; 2],
}
#[derive(Default, Clone, PartialEq, Eq)]
struct Row {
    result: u64,
    sides: [Side; 2],
}

pub struct Captured {
    results: CaptureRows<Row>,
    ships: Arena<ship::DetailsRow>,
    planes: Arena<plane::DetailsRow>,
    air: Arena<naval_combat_air_hit::Row>,
    hits: Arena<naval_combat_hit::Row>,
    sources: Arena<influence::SourceRow>,
    terms: Arena<influence::ModifierTerm>,
    commanders: Arena<commander::Row>,
    commander_keys: HashMap<u64, usize>,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self {
            results: CaptureRows::new(0),
            ships: Arena::new(0),
            planes: Arena::new(0),
            air: Arena::new(0),
            hits: Arena::new(0),
            sources: Arena::new(0),
            terms: Arena::new(0),
            commanders: Arena::new(0),
            commander_keys: HashMap::default(),
        }
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        let mut commander_keys = HashMap::default();
        if reserve_map(&mut commander_keys, c.commanders).is_err() {
            return Self::vacant();
        }
        Self {
            results: CaptureRows::new(c.results),
            ships: Arena::new(c.ships),
            planes: Arena::new(c.planes),
            air: Arena::new(c.air),
            hits: Arena::new(c.hits),
            sources: Arena::new(c.sources),
            terms: Arena::new(c.terms),
            commanders: Arena::new(c.commanders),
            commander_keys,
        }
    }
    pub fn reset(&mut self) {
        self.results.reset();
        self.ships.reset();
        self.planes.reset();
        self.air.reset();
        self.hits.reset();
        self.sources.reset();
        self.terms.reset();
        self.commanders.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.results.allocated_bytes()
            + self
                .results
                .rows
                .iter()
                .flat_map(|row| &row.sides)
                .map(|side| side.details.hq_influences.capacity() * size_of::<u64>())
                .sum::<usize>()
            + self.ships.allocated_bytes()
            + self.planes.allocated_bytes()
            + self.air.allocated_bytes()
            + self.hits.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.terms.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.commander_keys.capacity() * (size_of::<(u64, usize)>() + 1)
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        // Ordinary can grow owned storage during its original native walk.
        // Carry actual allocations forward to the other reusable frames.
        c.results = c.results.max(self.results.rows.capacity());
        c.ships = c.ships.max(self.ships.first.capacity());
        c.planes = c.planes.max(self.planes.first.capacity());
        c.air = c.air.max(self.air.first.capacity());
        c.hits = c.hits.max(self.hits.first.capacity());
        c.sources = c.sources.max(self.sources.first.capacity());
        c.terms = c.terms.max(self.terms.first.capacity());
        c.commanders = c.commanders.max(self.commanders.first.capacity());
        self.results.grow(&mut c.results);
        // All owned arenas grow from readable entries in the original pass;
        // copy their allocations without trusting a failed payload's count.
        if self.ships.first.len() < c.ships
            && resize_default(&mut self.ships.first, c.ships).is_err()
        {
            return;
        }
        if self.planes.first.len() < c.planes
            && resize_default(&mut self.planes.first, c.planes).is_err()
        {
            return;
        }
        if self.air.first.len() < c.air && resize_default(&mut self.air.first, c.air).is_err() {
            return;
        }
        if self.hits.first.len() < c.hits && resize_default(&mut self.hits.first, c.hits).is_err() {
            return;
        }
        if self.sources.first.len() < c.sources
            && resize_default(&mut self.sources.first, c.sources).is_err()
        {
            return;
        }
        if self.terms.first.len() < c.terms
            && resize_default(&mut self.terms.first, c.terms).is_err()
        {
            return;
        }
        if self.commanders.first.len() < c.commanders {
            let _ = resize_default(&mut self.commanders.first, c.commanders);
        }
    }
    fn pass(&mut self, memory: &impl Memory, base: usize, state: usize) -> Outcome {
        self.commander_keys.clear();
        let mut pass = Pass {
            rows: &mut self.results.rows,
            ships: &mut self.ships.first,
            planes: &mut self.planes.first,
            air: &mut self.air.first,
            hits: &mut self.hits.first,
            sources: &mut self.sources.first,
            terms: &mut self.terms.first,
            commanders: &mut self.commanders.first,
            commander_keys: &mut self.commander_keys,
            commanders_len: 0,
            commanders_required: 0,
            len: 0,
            lengths: [0; 6],
            required_arenas: [0; 6],
        };
        let census = result::visit_sources(memory, base, state, |source| {
            pass.visit(memory, base, source)
        });
        let outcome = match census {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(_) => Outcome::Rows { len: pass.len },
        };
        let lengths = pass.lengths;
        let required = pass.required_arenas;
        let commander_length = pass.commanders_len;
        let commander_required = pass.commanders_required;
        self.commanders.len = commander_length;
        self.commanders.required = commander_required;
        for (index, arena) in [
            &mut self.ships.len,
            &mut self.planes.len,
            &mut self.air.len,
            &mut self.hits.len,
            &mut self.sources.len,
            &mut self.terms.len,
        ]
        .into_iter()
        .enumerate()
        {
            *arena = lengths[index];
        }
        for (index, arena) in [
            &mut self.ships.required,
            &mut self.planes.required,
            &mut self.air.required,
            &mut self.hits.required,
            &mut self.sources.required,
            &mut self.terms.required,
        ]
        .into_iter()
        .enumerate()
        {
            *arena = required[index];
        }
        outcome
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let _reads = crate::read_accounting::table(TABLES[0]);
        let started = qpc();
        self.results.outcome = self.pass(memory, base, state);
        [qpc().saturating_sub(started).max(0) as u64]
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.results.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::NavalCombatResultDetails, _>(
            self.outcomes()[0],
            &self.results.rows,
            |r| rows::NavalCombatResultDetailsRow {
                result: r.result,
                sides: r.sides.each_ref().map(|s| rows::NavalResultSideDetails {
                    leader: s.details.leader,
                    hq_leader: s.details.hq_leader,
                    hq_influences: owned_vec(copy_vec(&s.details.hq_influences)),
                    commander_context: s.commanders[0]
                        .filter(|index| self.commanders.first[*index].commander != 0)
                        .map(|index| {
                            super::naval_details::commander_inputs(&self.commanders.first[index])
                        }),
                    hq_commander_context: s.commanders[1]
                        .filter(|index| self.commanders.first[*index].commander != 0)
                        .map(|index| {
                            super::naval_details::commander_inputs(&self.commanders.first[index])
                        }),
                    hq_modifiers: owned_vec(collect_vec(
                        range(&self.sources.first, s.hq_modifiers).iter().map(|v| {
                            rows::NavalShipModifierSource {
                                scope: v.scope,
                                index: v.index,
                                selector: v.selector,
                                reference: v.reference,
                                weights: v.weights,
                                movement: v.movement,
                                dynamic_keys: v.dynamic_keys,
                                terms: owned_vec(collect_vec(
                                    range(&self.terms.first, v.terms).iter().map(|t| {
                                        rows::NavalShipModifierTerm {
                                            key: t.key,
                                            raw: t.raw,
                                            metadata_flags: t.metadata_flags,
                                            feature_selector: t.feature_selector,
                                        }
                                    }),
                                )),
                            }
                        }),
                    )),
                    ships: owned_vec(collect_vec(range(&self.ships.first, s.ships).iter().map(
                        |r| rows::NavalResultShip {
                            member: r.member,
                            owner: r.owner,
                            subunit_index: r.subunit_index,
                            formation_type: r.formation_type,
                            build_cost_ic: r.build_cost_ic,
                            strength_ratio: r.strength_ratio,
                            pride: r.pride,
                            is_convoy: r.is_convoy,
                            name: owned_vec(copy_vec(r.name.as_bytes())),
                            type_name: owned_vec(copy_vec(r.type_name.as_bytes())),
                            sprite: owned_vec(copy_vec(r.sprite.as_bytes())),
                            sunk_by: owned_vec(copy_vec(r.sunk_by.as_bytes())),
                            variant: r.variant,
                            convoy_client: r.convoy_client,
                            convoy_index: r.convoy_index,
                            naval_hits: owned_vec(collect_vec(
                                range(&self.hits.first, r.naval_hits).iter().map(naval_hit),
                            )),
                            air_hits: r.air_hits.map(|span| {
                                owned_vec(collect_vec(
                                    range(&self.air.first, span).iter().map(air_hit),
                                ))
                            }),
                        },
                    ))),
                    planes: owned_vec(collect_vec(range(&self.planes.first, s.planes).iter().map(
                        |r| rows::NavalResultPlane {
                            variant: r.variant,
                            variant_summary_inputs: r.variant_summary_inputs,
                            initial_planes: r.initial_planes,
                            current_planes: r.current_planes,
                            lost_planes: r.lost_planes,
                            country: r.country,
                            name: owned_vec(copy_vec(r.name.as_bytes())),
                            naval_hits: owned_vec(collect_vec(
                                range(&self.hits.first, r.naval_hits).iter().map(naval_hit),
                            )),
                            air_hits: r.air_hits.map(|span| {
                                owned_vec(collect_vec(
                                    range(&self.air.first, span).iter().map(air_hit),
                                ))
                            }),
                        },
                    ))),
                }),
            },
        )]
    }
}

struct Pass<'a> {
    rows: &'a mut Vec<Row>,
    ships: &'a mut Vec<ship::DetailsRow>,
    planes: &'a mut Vec<plane::DetailsRow>,
    air: &'a mut Vec<naval_combat_air_hit::Row>,
    hits: &'a mut Vec<naval_combat_hit::Row>,
    sources: &'a mut Vec<influence::SourceRow>,
    terms: &'a mut Vec<influence::ModifierTerm>,
    commanders: &'a mut Vec<commander::Row>,
    commander_keys: &'a mut HashMap<u64, usize>,
    commanders_len: usize,
    commanders_required: usize,
    len: usize,
    lengths: [usize; 6],
    required_arenas: [usize; 6],
}
impl Pass<'_> {
    fn commander(
        &mut self,
        memory: &impl Memory,
        base: usize,
        handle: u64,
    ) -> Result<Option<usize>, &'static str> {
        if handle == 0 {
            return Ok(None);
        }
        if let Some(&index) = self.commander_keys.get(&handle) {
            return Ok(Some(index));
        }
        let index = self.commanders_required;
        let end = index
            .checked_add(1)
            .ok_or("naval result commander demand overflow")?;
        reserve_map(self.commander_keys, 1).map_err(|_| "collector allocation failed")?;
        self.commander_keys.insert(handle, index);
        self.commanders_required = end;
        ensure(self.commanders, end)?;
        // P: 1417C8060 hides a missing retained commander portrait. Preserve
        // the side's full handle while the optional context is absent.
        self.commanders[index] =
            if engine::creference_registry::resolve(memory, base, handle)?.is_some() {
                commander::capture(memory, base, handle)?
            } else {
                commander::Row::default()
            };
        self.commanders_len = end;
        Ok(Some(index))
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: result::Source,
    ) -> Result<(), &'static str> {
        ensure(
            self.rows,
            self.len
                .checked_add(1)
                .ok_or("naval result row count overflow")?,
        )?;
        let mut row = Row {
            result: source.id,
            ..Row::default()
        };
        for (side_index, offset) in [result::FIRST_SIDE, result::SECOND_SIDE]
            .into_iter()
            .enumerate()
        {
            let pointer = source
                .pointer
                .checked_add(offset)
                .ok_or("naval result detail side overflow")?;
            let mut s = Side {
                ships: [self.lengths[0], 0],
                planes: [self.lengths[1], 0],
                ..Side::default()
            };
            s.details = side::capture_details(memory, base, pointer)?;
            s.commanders = [
                self.commander(memory, base, s.details.leader)?,
                self.commander(memory, base, s.details.hq_leader)?,
            ];
            let mut lengths = [self.lengths[4], self.lengths[5]];
            let mut required = [self.required_arenas[4], self.required_arenas[5]];
            let mut influences = influence::InfluenceBuffers {
                sources: self.sources,
                terms: self.terms,
                lengths: &mut lengths,
                required: &mut required,
            };
            let captured = side::capture_influences(memory, base, pointer, &mut influences);
            self.lengths[4..6].copy_from_slice(&lengths);
            self.required_arenas[4..6].copy_from_slice(&required);
            s.hq_modifiers = captured?;
            side::visit_entries(memory, base, pointer, false, |_, entry| {
                self.required_arenas[0] = self.required_arenas[0]
                    .checked_add(1)
                    .ok_or("naval result ship count overflow")?;
                ensure(self.ships, self.required_arenas[0])?;
                let handle = engine::read(
                    memory,
                    entry
                        .checked_add(ship::HANDLE)
                        .ok_or("naval result ship handle overflow")?,
                )
                .ok_or("naval result ship handle unreadable")?;
                let mut buffers = ship::HitBuffers {
                    air_hits: self.air,
                    naval_hits: self.hits,
                    lengths: [self.lengths[2], self.lengths[3]],
                    required: [self.required_arenas[2], self.required_arenas[3]],
                };
                let captured = ship::capture_details(memory, base, entry, handle, &mut buffers);
                self.lengths[2..4].copy_from_slice(&buffers.lengths);
                self.required_arenas[2..4].copy_from_slice(&buffers.required);
                self.ships[self.lengths[0]] = captured?;
                self.lengths[0] += 1;
                s.ships[1] += 1;
                Ok(())
            })?;
            side::visit_entries(memory, base, pointer, true, |_, entry| {
                self.required_arenas[1] = self.required_arenas[1]
                    .checked_add(1)
                    .ok_or("naval result plane count overflow")?;
                ensure(self.planes, self.required_arenas[1])?;
                let mut buffers = ship::HitBuffers {
                    air_hits: self.air,
                    naval_hits: self.hits,
                    lengths: [self.lengths[2], self.lengths[3]],
                    required: [self.required_arenas[2], self.required_arenas[3]],
                };
                let captured = plane::capture_details(memory, base, entry, &mut buffers);
                self.lengths[2..4].copy_from_slice(&buffers.lengths);
                self.required_arenas[2..4].copy_from_slice(&buffers.required);
                self.planes[self.lengths[1]] = captured?;
                self.lengths[1] += 1;
                s.planes[1] += 1;
                Ok(())
            })?;
            row.sides[side_index] = s;
        }
        self.rows[self.len] = row;
        self.len += 1;
        Ok(())
    }
}
fn ensure<T: Default + Clone>(rows: &mut Vec<T>, required: usize) -> Result<(), &'static str> {
    let mut capacity = rows.len();
    grow_capacity(&mut capacity, required);
    if rows.len() < capacity {
        resize_default(rows, capacity).map_err(|_| "collector allocation failed")?;
    }
    Ok(())
}
fn range<T>(items: &[T], span: [usize; 2]) -> &[T] {
    &items[span[0]..span[0] + span[1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Traced<'a> {
        bytes: &'a [u8],
        reads: RefCell<Vec<(usize, usize)>>,
        fault: Option<usize>,
    }
    impl Memory for Traced<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            let ordinal = self.reads.borrow().len();
            self.reads.borrow_mut().push((address, destination.len()));
            if self.fault == Some(ordinal) {
                destination.fill(0xed);
                return false;
            }
            engine::memory::SliceMemory {
                base: 0,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    fn capacities(n: usize) -> Capacities {
        Capacities {
            results: n,
            ships: n,
            planes: n,
            air: n,
            hits: n,
            sources: n,
            terms: n,
            commanders: n,
        }
    }
    #[test]
    fn unavailable_result_census_cannot_be_reported_as_empty() {
        let c = Capacities {
            results: 1,
            ships: 1,
            planes: 1,
            air: 1,
            hits: 1,
            sources: 1,
            terms: 1,
            commanders: 1,
        };
        let mut captured = Captured::with_capacities(&c);
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        captured.capture(&memory, 0, 0);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn failed_owned_payload_demand_cannot_size_the_recycled_pool() {
        let mut c = capacities(1);
        let mut captured = Captured::with_capacities(&c);
        captured.results.outcome = Outcome::Unavailable {
            reason: "naval detail nested pointer unreadable",
        };
        captured.hits.required = i32::MAX as usize;
        captured.air.required = 2 * i32::MAX as usize;
        captured.terms.required = i32::MAX as usize;
        captured.grow(&mut c);
        assert_eq!((c.hits, c.air, c.terms), (1, 1, 1));
        assert_eq!(
            (
                captured.hits.first.len(),
                captured.air.first.len(),
                captured.terms.first.len()
            ),
            (1, 1, 1)
        );
    }
    #[test]
    fn commander_growth_keeps_unique_missing_context_and_checked_demand() {
        struct Missing(RefCell<Vec<usize>>);
        impl Memory for Missing {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                self.0.borrow_mut().push(address);
                destination.fill(0);
                true
            }
        }
        let mut pass = Pass {
            rows: &mut Vec::new(),
            ships: &mut Vec::new(),
            planes: &mut Vec::new(),
            air: &mut Vec::new(),
            hits: &mut Vec::new(),
            sources: &mut Vec::new(),
            terms: &mut Vec::new(),
            commanders: &mut Vec::new(),
            commander_keys: &mut HashMap::default(),
            commanders_len: 0,
            commanders_required: 0,
            len: 0,
            lengths: [0; 6],
            required_arenas: [0; 6],
        };
        let memory = Missing(RefCell::default());
        assert_eq!(pass.commander(&memory, 0, 0).unwrap(), None);
        assert!(pass.commander_keys.is_empty());
        assert_eq!(
            pass.commander(&memory, 0, 0xaaaa_bbbb_cccc_dddd).unwrap(),
            Some(0)
        );
        assert_eq!(
            pass.commander(&memory, 0, 0xaaaa_bbbb_cccc_dddd).unwrap(),
            Some(0)
        );
        assert_eq!(
            pass.commander(&memory, 0, 0xffff_ffff_0000_0011).unwrap(),
            Some(1)
        );
        assert_eq!(pass.commanders_required, 2);
        assert_eq!(pass.commanders_len, 2);
        assert!(pass.commanders[..2].iter().all(|row| row.commander == 0));
        assert_eq!(
            *memory.0.borrow(),
            [
                engine::creference_registry::HIGH_ROOT_RVA,
                engine::creference_registry::LOW_ROOTS_RVA + 17 * 8
            ]
        );
        pass.commanders_required = usize::MAX;
        assert_eq!(
            pass.commander(&memory, 0, 0x1234_5678),
            Err("naval result commander demand overflow")
        );
        assert_eq!(memory.0.borrow().len(), 2);
    }

    #[test]
    fn commander_census_grows_past_65536_without_repeating_context() {
        struct Missing;
        impl Memory for Missing {
            fn copy(&self, _: usize, destination: &mut [u8]) -> bool {
                destination.fill(0);
                true
            }
        }
        let mut captured = Captured::vacant();
        let mut pass = Pass {
            rows: &mut captured.results.rows,
            ships: &mut captured.ships.first,
            planes: &mut captured.planes.first,
            air: &mut captured.air.first,
            hits: &mut captured.hits.first,
            sources: &mut captured.sources.first,
            terms: &mut captured.terms.first,
            commanders: &mut captured.commanders.first,
            commander_keys: &mut captured.commander_keys,
            commanders_len: 0,
            commanders_required: 0,
            len: 0,
            lengths: [0; 6],
            required_arenas: [0; 6],
        };
        const COUNT: usize = 65_537;
        for index in 0..COUNT {
            let handle = ((index as u64 + 1) << 32) | 1;
            assert_eq!(pass.commander(&Missing, 0, handle), Ok(Some(index)));
            assert_eq!(pass.commander(&Missing, 0, handle), Ok(Some(index)));
        }
        assert_eq!(pass.commander_keys.len(), COUNT);
        assert_eq!(
            (pass.commanders_len, pass.commanders_required),
            (COUNT, COUNT)
        );
        assert!(pass.commanders.len() >= COUNT);
        assert!(
            pass.commanders[..COUNT]
                .iter()
                .all(|row| row.commander == 0)
        );
    }

    #[test]
    fn projection_keeps_all_retained_hq_influences_past_256() {
        const COUNT: usize = 257;
        let mut captured = Captured::with_capacities(&capacities(1));
        captured.results.outcome = Outcome::Rows { len: 1 };
        captured.results.rows[0].result = 42;
        let side = &mut captured.results.rows[0].sides[0].details;
        side.hq_influences = (1..=COUNT as u64).collect();
        let [TableView::Complete(rows::TableRows::NavalCombatResultDetails(rows))] =
            captured.tables()
        else {
            panic!("complete retained HQ references were not projected")
        };
        assert_eq!(rows[0].sides[0].hq_influences.len(), COUNT);
        assert_eq!(rows[0].sides[0].hq_influences[COUNT - 1], COUNT as u64);
    }

    #[test]
    fn result_growth_from_empty_preserves_occurrences_and_fault_prefixes() {
        const BASE: usize = 0x1_4000_0000;
        const STATE: usize = 0x100;
        let mut bytes = vec![0; 0x4000];
        let word = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes())
        };
        let array = |bytes: &mut [u8], at: usize, data: u64, count: u32| {
            word(bytes, at, data);
            bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
            bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
        };
        array(
            &mut bytes,
            STATE + engine::cgame_state::NAVAL_COMBAT_RESULTS,
            0x800,
            2,
        );
        for (at, value) in [
            (0x800, 0x1000u64),
            (0x808, 0x1000),
            (0x1000, (BASE + result::VTABLE_RVA) as u64),
            (0x1008, 0xaabb_ccdd_0000_0010),
            (0x1018, (BASE + side::VTABLE_RVA) as u64),
            (0x1088, (BASE + side::VTABLE_RVA) as u64),
            (0x1800, 0x2000),
            (0x1808, 0x2000),
            (0x1820, 0x2200),
            (0x1900, 0x2800),
            (0x1908, 0x2800),
            (0x1920, 0x2900),
            (0x2000, (BASE + ship::VTABLE_RVA) as u64),
            (0x2008, 0x1122_3344_0000_0020),
            (0x2200, (BASE + plane::VTABLE_RVA) as u64),
            (0x2208, 0x2400),
            (
                0x2400,
                (BASE + engine::cequipment_variant::VTABLE_RVA) as u64,
            ),
            (0x2408, 0x1234_5678_0000_0030),
            (0x26a0, i64::MIN as u64),
            (0x26a8, i64::MAX as u64),
            (0x2800, (BASE + naval_combat_hit::VTABLE_RVA) as u64),
            (0x2838, i64::MIN as u64),
            (0x2840, i64::MAX as u64),
            (0x2900, (BASE + naval_combat_air_hit::VTABLE_RVA) as u64),
            (0x2908, 0x2400),
        ] {
            word(&mut bytes, at, value);
        }
        // Empty MSVC strings still require the inline capacity sentinel.
        // Initialize ship names, the plane name and the hit target name.
        for string in [0x2040, 0x2060, 0x2080, 0x20a0, 0x2220, 0x2810] {
            word(
                &mut bytes,
                string + engine::msvc_string::CAPACITY,
                engine::msvc_string::INLINE_CAPACITY,
            );
        }
        array(&mut bytes, 0x1018 + side::MEMBERS, 0x1800, 2);
        array(&mut bytes, 0x1018 + 8, 0x1820, 1);
        for (pointer, offsets) in [(0x2000, [0xd8, 0xf0, 0x108]), (0x2200, [0x40, 0x58, 0x70])] {
            array(&mut bytes, pointer + offsets[0], 0x1900, 2);
            array(&mut bytes, pointer + offsets[1], 0x1920, 1);
            array(&mut bytes, pointer + offsets[2], 0x1920, 1);
        }
        let memory = Traced {
            bytes: &bytes,
            reads: RefCell::default(),
            fault: None,
        };
        let mut full = Captured::with_capacities(&capacities(32));
        full.capture(&memory, BASE, STATE);
        assert_eq!(full.outcomes(), [Outcome::Rows { len: 2 }]);
        assert_eq!(
            (full.ships.len, full.planes.len, full.air.len, full.hits.len),
            (4, 2, 12, 12)
        );
        let trace = memory.reads.borrow().clone();
        for capacity in [0, 1] {
            for fault in (0..trace.len()).map(Some).chain([None]) {
                let reference = Traced {
                    bytes: &bytes,
                    reads: RefCell::default(),
                    fault,
                };
                full.reset();
                full.capture(&reference, BASE, STATE);
                let observed = Traced {
                    bytes: &bytes,
                    reads: RefCell::default(),
                    fault,
                };
                let mut grown = if capacity == 0 {
                    Captured::vacant()
                } else {
                    Captured::with_capacities(&capacities(capacity))
                };
                if capacity == 0 {
                    assert_eq!(grown.allocated_bytes(), 0);
                }
                grown.capture(&observed, BASE, STATE);
                assert_eq!(grown.outcomes(), full.outcomes());
                assert_eq!(*observed.reads.borrow(), *reference.reads.borrow());
                for (actual, expected) in [
                    (grown.ships.len, full.ships.len),
                    (grown.planes.len, full.planes.len),
                    (grown.air.len, full.air.len),
                    (grown.hits.len, full.hits.len),
                ] {
                    assert_eq!(actual, expected);
                }
                assert_eq!(
                    &grown.ships.first[..grown.ships.len],
                    &full.ships.first[..full.ships.len]
                );
                assert_eq!(
                    &grown.planes.first[..grown.planes.len],
                    &full.planes.first[..full.planes.len]
                );
                assert_eq!(
                    &grown.air.first[..grown.air.len],
                    &full.air.first[..full.air.len]
                );
                assert_eq!(
                    &grown.hits.first[..grown.hits.len],
                    &full.hits.first[..full.hits.len]
                );
                if let Some(ordinal) = fault {
                    assert!(matches!(grown.outcomes()[0], Outcome::Unavailable { .. }));
                    assert_eq!(*observed.reads.borrow(), trace[..=ordinal]);
                } else {
                    assert!(grown.results.rows[..2] == full.results.rows[..2]);
                    assert_eq!(grown.results.rows[1].sides[0].ships, [2, 2]);
                    assert_eq!(grown.results.rows[1].sides[0].planes, [1, 1]);
                    assert_eq!(
                        (
                            grown.ships.required,
                            grown.planes.required,
                            grown.air.required,
                            grown.hits.required
                        ),
                        (4, 2, 12, 12)
                    );
                    let mut c = capacities(0);
                    grown.grow(&mut c);
                    assert!(
                        c.results >= 2
                            && c.ships >= 4
                            && c.planes >= 2
                            && c.air >= 12
                            && c.hits >= 12
                    );
                    assert!(
                        grown.allocated_bytes()
                            >= grown.results.rows.capacity() * size_of::<Row>()
                                + grown.ships.first.capacity() * size_of::<ship::DetailsRow>()
                                + grown.planes.first.capacity() * size_of::<plane::DetailsRow>()
                                + grown.air.first.capacity()
                                    * size_of::<naval_combat_air_hit::Row>()
                                + grown.hits.first.capacity() * size_of::<naval_combat_hit::Row>()
                    );
                }
            }
        }
    }
}
