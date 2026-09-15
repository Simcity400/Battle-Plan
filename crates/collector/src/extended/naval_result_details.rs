//! Retained naval results: copied snapshots and hit lists, with an independent
//! 200-observation consistency window. Growth/projection run on the spool thread.
use crate::{
    sampling::{full_match, qpc},
    spooling::table,
    table_capture::{Arena, CaptureRows},
};
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
#[derive(Default, Clone, Copy, PartialEq, Eq)]
struct Side {
    details: side::DetailsRow,
    ships: [usize; 2],
    planes: [usize; 2],
    hq_modifiers: [usize; 2],
    commanders: [Option<usize>; 2],
}
#[derive(Default, Clone, Copy, PartialEq, Eq)]
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
    commander_keys: Vec<u64>,
    commander_slots: Vec<u32>,
}
impl Captured {
    pub fn with_capacities(c: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        Self {
            results: CaptureRows::new(c.results, successful[0]),
            ships: Arena::new(c.ships, successful[0]),
            planes: Arena::new(c.planes, successful[0]),
            air: Arena::new(c.air, successful[0]),
            hits: Arena::new(c.hits, successful[0]),
            sources: Arena::new(c.sources, successful[0]),
            terms: Arena::new(c.terms, successful[0]),
            commanders: Arena::new(c.commanders, successful[0]),
            commander_keys: vec![0; 65_536],
            commander_slots: vec![0; 131_072],
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
            + self.ships.allocated_bytes()
            + self.planes.allocated_bytes()
            + self.air.allocated_bytes()
            + self.hits.allocated_bytes()
            + self.sources.allocated_bytes()
            + self.terms.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.commander_keys.capacity() * size_of::<u64>()
            + self.commander_slots.capacity() * size_of::<u32>()
    }
    pub fn grow(&mut self, c: &mut Capacities, successful: &[u16]) {
        self.results.grow(&mut c.results, successful[0]);
        self.ships.grow(&mut c.ships, successful[0]);
        self.planes.grow(&mut c.planes, successful[0]);
        self.air.grow(&mut c.air, successful[0]);
        self.hits.grow(&mut c.hits, successful[0]);
        self.sources.grow(&mut c.sources, successful[0]);
        self.terms.grow(&mut c.terms, successful[0]);
        self.commanders.grow(&mut c.commanders, successful[0]);
    }
    fn pass(&mut self, memory: &impl Memory, base: usize, state: usize, second: bool) -> Outcome {
        self.commander_slots.fill(0);
        let mut pass = Pass {
            rows: if second {
                &mut self.results.second
            } else {
                &mut self.results.rows
            },
            ships: if second {
                &mut self.ships.second
            } else {
                &mut self.ships.first
            },
            planes: if second {
                &mut self.planes.second
            } else {
                &mut self.planes.first
            },
            air: if second {
                &mut self.air.second
            } else {
                &mut self.air.first
            },
            hits: if second {
                &mut self.hits.second
            } else {
                &mut self.hits.first
            },
            sources: if second {
                &mut self.sources.second
            } else {
                &mut self.sources.first
            },
            terms: if second {
                &mut self.terms.second
            } else {
                &mut self.terms.first
            },
            commanders: if second {
                &mut self.commanders.second
            } else {
                &mut self.commanders.first
            },
            commander_keys: &mut self.commander_keys,
            commander_slots: &mut self.commander_slots,
            commanders_len: 0,
            commanders_required: 0,
            len: 0,
            required: 0,
            lengths: [0; 6],
            required_arenas: [0; 6],
            truncated: false,
        };
        let census = result::visit_sources(memory, base, state, |source| {
            pass.visit(memory, base, source)
        });
        let outcome = match census {
            Err(reason) => Outcome::Unavailable { reason },
            Ok(_) if pass.truncated => Outcome::Truncated {
                len: pass.len,
                unread_rows: pass
                    .required
                    .saturating_sub(pass.len)
                    .min(u32::MAX as usize) as u32,
            },
            Ok(_) => Outcome::Rows { len: pass.len },
        };
        let lengths = pass.lengths;
        let required = pass.required_arenas;
        let commander_length = pass.commanders_len;
        let commander_required = pass.commanders_required;
        if second {
            let matched = self.ships.compare(lengths[0], required[0])
                & self.planes.compare(lengths[1], required[1])
                & self.air.compare(lengths[2], required[2])
                & self.hits.compare(lengths[3], required[3])
                & self.sources.compare(lengths[4], required[4])
                & self.terms.compare(lengths[5], required[5])
                & self
                    .commanders
                    .compare(commander_length, commander_required);
            self.results.mismatches += usize::from(!matched);
        } else {
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
        }
        outcome
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let started = qpc();
        self.results.mismatches = 0;
        self.results.checked = needed[0];
        let first = self.pass(memory, base, state, false);
        self.results.outcome = first;
        let mut matched = false;
        if needed[0] {
            let second = self.pass(memory, base, state, true);
            let nested = self.results.mismatches;
            self.results.compare(second);
            self.results.mismatches += nested;
            matched = full_match(first, second, self.results.mismatches);
        }
        ([matched], [qpc().saturating_sub(started).max(0) as u64])
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [if self.results.checked && self.results.mismatches != 0 {
            Outcome::Unavailable {
                reason: "naval result detail snapshots changed during double read",
            }
        } else {
            self.results.outcome
        }]
    }
    pub fn checked_mismatches(&self) -> [usize; N] {
        [self.results.mismatches]
    }
    pub fn tables(&self) -> [TableView; N] {
        [table::<tables::NavalCombatResultDetails, _>(
            self.outcomes()[0],
            &self.results.rows,
            |r| rows::NavalCombatResultDetailsRow {
                result: r.result,
                sides: r.sides.map(|s| rows::NavalResultSideDetails {
                    leader: s.details.leader,
                    hq_leader: s.details.hq_leader,
                    hq_influences: s.details.hq_influences[..s.details.hq_influence_count].to_vec(),
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
                    hq_modifiers: range(&self.sources.first, s.hq_modifiers)
                        .iter()
                        .map(|v| rows::NavalShipModifierSource {
                            scope: v.scope,
                            index: v.index,
                            selector: v.selector,
                            reference: v.reference,
                            weights: v.weights,
                            dynamic_keys: v.dynamic_keys,
                            terms: range(&self.terms.first, v.terms)
                                .iter()
                                .map(|t| rows::NavalShipModifierTerm {
                                    key: t.key,
                                    raw: t.raw,
                                    metadata_flags: t.metadata_flags,
                                    feature_selector: t.feature_selector,
                                })
                                .collect(),
                        })
                        .collect(),
                    ships: range(&self.ships.first, s.ships)
                        .iter()
                        .map(|r| rows::NavalResultShip {
                            member: r.member,
                            owner: r.owner,
                            subunit_index: r.subunit_index,
                            formation_type: r.formation_type,
                            build_cost_ic: r.build_cost_ic,
                            strength_ratio: r.strength_ratio,
                            pride: r.pride,
                            is_convoy: r.is_convoy,
                            name: r.name.as_bytes().to_vec(),
                            type_name: r.type_name.as_bytes().to_vec(),
                            sprite: r.sprite.as_bytes().to_vec(),
                            sunk_by: r.sunk_by.as_bytes().to_vec(),
                            variant: r.variant,
                            convoy_client: r.convoy_client,
                            convoy_index: r.convoy_index,
                            naval_hits: range(&self.hits.first, r.naval_hits)
                                .iter()
                                .map(naval_hit)
                                .collect(),
                            air_hits: r.air_hits.map(|span| {
                                range(&self.air.first, span).iter().map(air_hit).collect()
                            }),
                        })
                        .collect(),
                    planes: range(&self.planes.first, s.planes)
                        .iter()
                        .map(|r| rows::NavalResultPlane {
                            variant: r.variant,
                            variant_summary_inputs: r.variant_summary_inputs,
                            initial_planes: r.initial_planes,
                            current_planes: r.current_planes,
                            lost_planes: r.lost_planes,
                            country: r.country,
                            name: r.name.as_bytes().to_vec(),
                            naval_hits: range(&self.hits.first, r.naval_hits)
                                .iter()
                                .map(naval_hit)
                                .collect(),
                            air_hits: r.air_hits.map(|span| {
                                range(&self.air.first, span).iter().map(air_hit).collect()
                            }),
                        })
                        .collect(),
                }),
            },
        )]
    }
}

struct Pass<'a> {
    rows: &'a mut [Row],
    ships: &'a mut [ship::DetailsRow],
    planes: &'a mut [plane::DetailsRow],
    air: &'a mut [naval_combat_air_hit::Row],
    hits: &'a mut [naval_combat_hit::Row],
    sources: &'a mut [influence::SourceRow],
    terms: &'a mut [influence::ModifierTerm],
    commanders: &'a mut [commander::Row],
    commander_keys: &'a mut [u64],
    commander_slots: &'a mut [u32],
    commanders_len: usize,
    commanders_required: usize,
    len: usize,
    required: usize,
    lengths: [usize; 6],
    required_arenas: [usize; 6],
    truncated: bool,
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
        let mut hash = handle ^ (handle >> 33);
        hash = hash.wrapping_mul(0xff51_afd7_ed55_8ccd);
        hash ^= hash >> 33;
        let mask = self.commander_slots.len() - 1;
        let mut slot = hash as usize & mask;
        for _ in 0..self.commander_slots.len() {
            let entry = self.commander_slots[slot];
            if entry == 0 {
                let index = self.commanders_required;
                if index == self.commander_keys.len() {
                    return Err("naval result commander census bound exceeded");
                }
                self.commander_keys[index] = handle;
                self.commander_slots[slot] = (index + 1) as u32;
                self.commanders_required += 1;
                if self.truncated || index >= self.commanders.len() {
                    self.truncated = true;
                    return Ok(Some(index));
                }
                //1417C8060 hides a missing retained commander portrait. Preserve
                // the side's full handle while the optional context is absent.
                self.commanders[index] =
                    if engine::creference_registry::resolve(memory, base, handle)?.is_some() {
                        commander::capture(memory, base, handle)?
                    } else {
                        commander::Row::default()
                    };
                self.commanders_len = index + 1;
                return Ok(Some(index));
            }
            let index = entry as usize - 1;
            if self.commander_keys[index] == handle {
                return Ok(Some(index));
            }
            slot = (slot + 1) & mask;
        }
        Err("naval result commander hash bound exceeded")
    }
    fn count_omitted_hits(
        &mut self,
        memory: &impl Memory,
        base: usize,
        pointer: usize,
        air: bool,
    ) -> Result<(), &'static str> {
        let counts = ship::hit_counts(memory, base, pointer, air)?;
        for (lane, count) in counts.into_iter().enumerate() {
            self.required_arenas[lane + 2] = self.required_arenas[lane + 2]
                .checked_add(count)
                .ok_or("naval result required hit count overflow")?;
        }
        Ok(())
    }
    fn visit(
        &mut self,
        memory: &impl Memory,
        base: usize,
        source: result::Source,
    ) -> Result<(), &'static str> {
        self.required += 1;
        self.truncated |= self.len == self.rows.len();
        let saved = self.lengths;
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
                sources: if self.truncated {
                    &mut []
                } else {
                    self.sources
                },
                terms: if self.truncated { &mut [] } else { self.terms },
                lengths: &mut lengths,
                required: &mut required,
            };
            let captured = side::capture_influences(memory, base, pointer, &mut influences);
            self.lengths[4..6].copy_from_slice(&lengths);
            self.required_arenas[4..6].copy_from_slice(&required);
            match captured {
                Ok(span) => s.hq_modifiers = span,
                Err(influence::CAPACITY) => self.truncated = true,
                Err(reason) => return Err(reason),
            }
            side::visit_entries(memory, base, pointer, false, |_, entry| {
                self.required_arenas[0] += 1;
                if self.truncated {
                    return self.count_omitted_hits(memory, base, entry, false);
                }
                if self.lengths[0] == self.ships.len() {
                    self.truncated = true;
                    return self.count_omitted_hits(memory, base, entry, false);
                }
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
                match captured {
                    Ok(r) => {
                        self.ships[self.lengths[0]] = r;
                        self.lengths[0] += 1;
                        s.ships[1] += 1;
                        Ok(())
                    }
                    Err(ship::ARENA_CAPACITY) => {
                        self.truncated = true;
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            })?;
            side::visit_entries(memory, base, pointer, true, |_, entry| {
                self.required_arenas[1] += 1;
                if self.truncated {
                    return self.count_omitted_hits(memory, base, entry, true);
                }
                if self.lengths[1] == self.planes.len() {
                    self.truncated = true;
                    return self.count_omitted_hits(memory, base, entry, true);
                }
                let mut buffers = ship::HitBuffers {
                    air_hits: self.air,
                    naval_hits: self.hits,
                    lengths: [self.lengths[2], self.lengths[3]],
                    required: [self.required_arenas[2], self.required_arenas[3]],
                };
                let captured = plane::capture_details(memory, base, entry, &mut buffers);
                self.lengths[2..4].copy_from_slice(&buffers.lengths);
                self.required_arenas[2..4].copy_from_slice(&buffers.required);
                match captured {
                    Ok(r) => {
                        self.planes[self.lengths[1]] = r;
                        self.lengths[1] += 1;
                        s.planes[1] += 1;
                        Ok(())
                    }
                    Err(ship::ARENA_CAPACITY) => {
                        self.truncated = true;
                        Ok(())
                    }
                    Err(reason) => Err(reason),
                }
            })?;
            if !self.truncated && side::capture_details(memory, base, pointer)? != s.details {
                return Err("naval result detail side changed during entry capture");
            }
            row.sides[side_index] = s;
        }
        if self.truncated {
            self.lengths = saved;
        } else {
            self.rows[self.len] = row;
            self.len += 1;
        }
        Ok(())
    }
}
fn range<T>(items: &[T], span: [usize; 2]) -> &[T] {
    &items[span[0]..span[0] + span[1]]
}
fn air_hit(v: &naval_combat_air_hit::Row) -> rows::NavalCombatAirHit {
    rows::NavalCombatAirHit {
        variant: v.variant,
        count: v.count,
        country: v.country,
    }
}
fn naval_hit(v: &naval_combat_hit::Row) -> rows::NavalCombatHit {
    rows::NavalCombatHit {
        target_slot: v.target_slot,
        target_name: v.target_name.as_bytes().to_vec(),
        convoy: v.convoy,
        damage: v.damage,
        denominator: v.denominator,
        last_hit: v.last_hit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn independent_result_window_retires_all_arenas() {
        for successes in [0, 200] {
            let pool = Captured::with_capacities(&Capacities::default(), &[successes]);
            eprintln!(
                "naval result default pool at {successes} successful checks: {} bytes",
                pool.allocated_bytes()
            );
        }
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
        let mut captured = Captured::with_capacities(&c, &[0]);
        assert_eq!(captured.results.second.len(), 1);
        assert_eq!(captured.ships.second.len(), 1);
        let mut c = c;
        captured.grow(&mut c, &[200]);
        assert!(captured.results.second.is_empty());
        assert!(captured.ships.second.is_empty());
        assert!(captured.planes.second.is_empty());
        assert!(captured.air.second.is_empty());
        assert!(captured.hits.second.is_empty());
        assert!(captured.sources.second.is_empty());
        assert!(captured.terms.second.is_empty());
        assert!(captured.commanders.second.is_empty());
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
        let mut captured = Captured::with_capacities(&c, &[0]);
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        let (matched, _) = captured.capture(&memory, 0, 0, &[true]);
        assert!(!matched[0]);
        assert!(matches!(
            captured.outcomes()[0],
            Outcome::Unavailable { .. }
        ));
    }
    #[test]
    fn omitted_commander_demand_is_unique_without_reading_payloads() {
        let mut keys = [0; 16];
        let mut slots = [0; 32];
        let mut pass = Pass {
            rows: &mut [],
            ships: &mut [],
            planes: &mut [],
            air: &mut [],
            hits: &mut [],
            sources: &mut [],
            terms: &mut [],
            commanders: &mut [],
            commander_keys: &mut keys,
            commander_slots: &mut slots,
            commanders_len: 0,
            commanders_required: 0,
            len: 0,
            required: 0,
            lengths: [0; 6],
            required_arenas: [0; 6],
            truncated: true,
        };
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &[],
        };
        assert_eq!(pass.commander(&memory, 0, 0).unwrap(), None);
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
        assert_eq!(pass.commanders_len, 0);
    }
}
