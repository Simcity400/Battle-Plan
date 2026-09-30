//! Hourly naval combat inputs. Ship rows and their arenas grow on the
//! acquisition worker from their original observations; projection is deferred.
use crate::{
    sampling::qpc,
    spooling::table,
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::naval_air_efficiency_inputs;
use engine::{
    Memory, cfex_air, cfex_damage_entry, cfex_member, cnaval_combat, naval_combat_air_hit,
    naval_combat_hit, naval_ship_modifier_inputs, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
pub const N: usize = 2;
pub const TABLES: [TableId; N] = [TableId::NavalCombatDetails, TableId::NavalCombatShips];
pub const READ_GROUPS: &[(&str, usize, &[usize])] =
    &[("shared naval combat registry census", 0, &[0, 1])];
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    details: usize,
    ships: usize,
    groups: usize,
    damage: usize,
    air: usize,
    hits: usize,
    planes: usize,
    plane_air: usize,
    plane_hits: usize,
    plane_names: usize,
    plane_kills: usize,
    commanders: usize,
    weapon_sources: usize,
    weapon_terms: usize,
    side_units: usize,
}
impl Default for Capacities {
    fn default() -> Self {
        Self {
            details: 64,
            ships: 1024,
            groups: 512,
            damage: 2048,
            air: 1024,
            hits: 2048,
            planes: 128,
            plane_air: 256,
            plane_hits: 512,
            plane_names: 512,
            plane_kills: 1024,
            commanders: 16,
            weapon_sources: 4096,
            weapon_terms: 16384,
            side_units: 256,
        }
    }
}
pub struct Captured {
    details: CaptureRows<cnaval_combat::DetailsRow>,
    ships: CaptureRows<cfex_member::DetailsRow>,
    damage: Arena<cfex_damage_entry::Row>,
    air: Arena<naval_combat_air_hit::Row>,
    hits: Arena<naval_combat_hit::Row>,
    groups: Vec<usize>,
    required_groups: usize,
    planes: Arena<cfex_air::Row>,
    plane_air: Arena<naval_combat_air_hit::Row>,
    plane_hits: Arena<naval_combat_hit::Row>,
    plane_names: Arena<engine::msvc_string::Text<256>>,
    plane_kills: Arena<naval_air_efficiency_inputs::WingKill>,
    commanders: Arena<engine::naval_commander_inputs::Row>,
    weapon_sources: Arena<naval_ship_modifier_inputs::SourceRow>,
    weapon_terms: Arena<naval_ship_modifier_inputs::ModifierTerm>,
    /// Each side's task-force handles (`CCombatant+0x20`), grown after the hour.
    side_units: Arena<u64>,
    /// Side country tags; grown by the worker at admission, retained on recycle.
    summary_countries: Vec<u32>,
    summary_countries_len: usize,
}
impl Captured {
    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            details: 0,
            ships: 0,
            groups: 0,
            damage: 0,
            air: 0,
            hits: 0,
            planes: 0,
            plane_air: 0,
            plane_hits: 0,
            plane_names: 0,
            plane_kills: 0,
            commanders: 0,
            weapon_sources: 0,
            weapon_terms: 0,
            side_units: 0,
        })
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            details: CaptureRows::new(c.details),
            ships: CaptureRows::new(c.ships),
            damage: Arena::new(c.damage),
            air: Arena::new(c.air),
            hits: Arena::new(c.hits),
            groups: vec![0; c.groups],
            required_groups: 0,
            planes: Arena::new(c.planes),
            plane_air: Arena::new(c.plane_air),
            plane_hits: Arena::new(c.plane_hits),
            plane_names: Arena::new(c.plane_names),
            plane_kills: Arena::new(c.plane_kills),
            commanders: Arena::new(c.commanders),
            weapon_sources: Arena::new(c.weapon_sources),
            weapon_terms: Arena::new(c.weapon_terms),
            side_units: Arena::new(c.side_units),
            summary_countries: Vec::new(),
            summary_countries_len: 0,
        }
    }
    pub fn reset(&mut self) {
        self.details.reset();
        self.ships.reset();
        self.damage.reset();
        self.air.reset();
        self.hits.reset();
        self.planes.reset();
        self.plane_air.reset();
        self.plane_hits.reset();
        self.plane_names.reset();
        self.plane_kills.reset();
        self.commanders.reset();
        self.weapon_sources.reset();
        self.weapon_terms.reset();
        self.side_units.reset();
        self.summary_countries_len = 0;
        self.required_groups = 0;
    }
    pub fn allocated_bytes(&self) -> usize {
        self.details.allocated_bytes()
            + self.ships.allocated_bytes()
            + self.damage.allocated_bytes()
            + self.air.allocated_bytes()
            + self.hits.allocated_bytes()
            + self.planes.allocated_bytes()
            + self.plane_air.allocated_bytes()
            + self.plane_hits.allocated_bytes()
            + self.plane_names.allocated_bytes()
            + self.plane_kills.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.weapon_sources.allocated_bytes()
            + self.weapon_terms.allocated_bytes()
            + self.side_units.allocated_bytes()
            + self.groups.capacity() * size_of::<usize>()
            + self.summary_countries.capacity() * size_of::<u32>()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        // Adopt allocations made by the worker before replenishing other
        // pooled frames. Used lengths remain separate from allocated capacity.
        c.ships = c.ships.max(self.ships.rows.capacity());
        c.groups = c.groups.max(self.groups.capacity());
        c.damage = c.damage.max(self.damage.first.capacity());
        c.air = c.air.max(self.air.first.capacity());
        c.hits = c.hits.max(self.hits.first.capacity());
        c.weapon_sources = c.weapon_sources.max(self.weapon_sources.first.capacity());
        c.weapon_terms = c.weapon_terms.max(self.weapon_terms.first.capacity());
        self.details.grow(&mut c.details);
        self.ships.grow(&mut c.ships);
        self.damage.grow(&mut c.damage);
        self.air.grow(&mut c.air);
        self.hits.grow(&mut c.hits);
        grow_capacity(&mut c.groups, self.required_groups);
        grow_vec(&mut self.groups, c.groups);
        self.planes.grow(&mut c.planes);
        self.plane_air.grow(&mut c.plane_air);
        self.plane_hits.grow(&mut c.plane_hits);
        self.plane_names.grow(&mut c.plane_names);
        self.plane_kills.grow(&mut c.plane_kills);
        self.commanders.grow(&mut c.commanders);
        self.weapon_sources.grow(&mut c.weapon_sources);
        self.weapon_terms.grow(&mut c.weapon_terms);
        self.side_units.grow(&mut c.side_units);
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let mut read_phase = crate::read_accounting::table(TABLES[0]);
        let capture_started = qpc();
        let mut buffers = cfex_member::DetailBuffers {
            damage: &mut self.damage.first,
            air_hits: &mut self.air.first,
            naval_hits: &mut self.hits.first,
            lengths: [0; 3],
            required: [0; 3],
            planes: &mut self.planes.first,
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: &mut self.plane_air.first,
            plane_naval_hits: &mut self.plane_hits.first,
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_names: &mut self.plane_names.first,
            plane_names_len: 0,
            plane_names_required: 0,
            plane_kills: &mut self.plane_kills.first,
            plane_kills_len: 0,
            plane_kills_required: 0,
            commander_inputs: &mut self.commanders.first,
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            side_units: &mut self.side_units.first,
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut self.summary_countries,
            summary_countries_len: 0,
            weapon_sources: &mut self.weapon_sources.first,
            weapon_terms: &mut self.weapon_terms.first,
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
        };
        let first = cnaval_combat::walk_details_timed(
            memory,
            base,
            state,
            &mut self.details.rows,
            &mut self.ships.rows,
            &mut self.groups,
            &mut buffers,
            |phase| {
                read_phase.select(TABLES[phase]);
                qpc().max(0) as u64
            },
        );
        self.details.outcome = first.capture.combats;
        self.ships.outcome = first.capture.participants;
        self.damage.len = buffers.lengths[0];
        self.air.len = buffers.lengths[1];
        self.hits.len = buffers.lengths[2];
        self.damage.required = buffers.required[0];
        self.air.required = buffers.required[1];
        self.hits.required = buffers.required[2];
        self.planes.len = buffers.planes_len;
        self.planes.required = buffers.planes_required;
        self.plane_air.len = buffers.plane_hit_lengths[0];
        self.plane_air.required = buffers.plane_hit_required[0];
        self.plane_hits.len = buffers.plane_hit_lengths[1];
        self.plane_hits.required = buffers.plane_hit_required[1];
        self.plane_names.len = buffers.plane_names_len;
        self.plane_names.required = buffers.plane_names_required;
        self.plane_kills.len = buffers.plane_kills_len;
        self.plane_kills.required = buffers.plane_kills_required;
        self.commanders.len = buffers.commander_inputs_len;
        self.commanders.required = buffers.commander_inputs_required;
        self.side_units.len = buffers.side_units_len;
        self.side_units.required = buffers.side_units_required;
        self.weapon_sources.len = buffers.weapon_lengths[0];
        self.weapon_sources.required = buffers.weapon_required[0];
        self.weapon_terms.len = buffers.weapon_lengths[1];
        self.weapon_terms.required = buffers.weapon_required[1];
        self.summary_countries_len = buffers.summary_countries_len;
        self.required_groups = first.capture.required_group_capacity;
        let mut ticks = first.read_ticks;
        let total = qpc().saturating_sub(capture_started).max(0) as u64;
        ticks[0] = ticks[0].saturating_add(total.saturating_sub(ticks.iter().sum::<u64>()));
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [self.details.outcome, self.ships.outcome]
    }
    pub fn tables(&self) -> [TableView; N] {
        let out = self.outcomes();
        [
            table::<tables::NavalCombatDetails, _>(out[0], &self.details.rows, |r| {
                rows::NavalCombatDetailsRow {
                    combat: r.combat,
                    province: r.province,
                    day: r.day,
                    duration: r.duration,
                    progress: r.progress,
                    sides: r.sides.map(|s| rows::NavalCombatSideDetails {
                        commander: s.commander,
                        commander_inputs: range(&self.commanders.first, s.commander_inputs)
                            .first()
                            .map(commander_inputs),
                        units: range(&self.side_units.first, s.units).to_vec(),
                        engaged: s.engaged,
                        retreating: s.retreating,
                        capital_screening: s.capital_screening,
                        carrier_screening: s.carrier_screening,
                        positioning: s.positioning,
                        anti_air: s.anti_air,
                        summary: rows::NavalCombatSummaryInputs {
                            recent_join_penalty: s.summary.recent_join_penalty,
                            escort_bonus: s.summary.escort_bonus,
                            is_attacker: s.summary.is_attacker,
                            countries: range(&self.summary_countries, s.summary.countries)
                                .to_vec(),
                            secondary_countries: range(
                                &self.summary_countries,
                                s.summary.secondary_countries,
                            )
                            .to_vec(),
                            modifiers: s.summary.modifiers.map(|scope| {
                                scope.map(|m| rows::NavalCombatSummaryModifier {
                                    raw: m.raw,
                                    present: m.present,
                                })
                            }),
                            modifier_metadata: s.summary.modifier_metadata,
                            feature_flags: s.summary.feature_flags,
                            eligibility_callback: s.summary.eligibility_callback,
                            fixed_defines: s.summary.fixed_defines,
                            integer_defines: s.summary.integer_defines,
                        },
                        planes: range(&self.planes.first, s.planes)
                            .iter()
                            .map(|p| rows::NavalCombatPlane {
                                source: p.source,
                                group_index: p.group_index,
                                index: p.index,
                                carrier_slot: p.carrier_slot,
                                carrier_state: p.carrier_state,
                                air_base: p.air_base,
                                casualties: p.casualties,
                                time_duration: p.time_duration,
                                wings: std::array::from_fn(|i| {
                                    p.wings[i][..p.wing_counts[i]].to_vec()
                                }),
                                initial_planes: p.initial_planes,
                                current_planes: p.current_planes,
                                country: p.country,
                                date: p.date,
                                flags: p.flags,
                                name: p.name.as_bytes().to_vec(),
                                variant: p.variant,
                                cached_values: p.cached_values,
                                efficiency: rows::NavalCombatAirEfficiency {
                                    source: p.efficiency.source,
                                    carrier: p.efficiency.carrier,
                                    first_wing: p.efficiency.first_wing,
                                    pool_type_flags: p.efficiency.pool_type_flags,
                                    task_force: p.efficiency.task_force,
                                    mission: p.efficiency.mission,
                                    mission_factor: p.efficiency.mission_factor,
                                    is_attacker: p.efficiency.is_attacker,
                                    country: p.efficiency.country,
                                    container_present: p.efficiency.container_present,
                                    base_crowding: p.efficiency.base_crowding,
                                    carrier_organisation: p.efficiency.carrier_organisation,
                                    carrier_base_organisation: p
                                        .efficiency
                                        .carrier_base_organisation,
                                    night: p.efficiency.night,
                                    combat_duration: p.efficiency.combat_duration,
                                    base_efficiency: p.efficiency.base_efficiency,
                                    mission_duration_threshold: p
                                        .efficiency
                                        .mission_duration_threshold,
                                    modifiers: p.efficiency.modifiers.map(|m| {
                                        rows::NavalCombatAirEfficiencyModifier {
                                            raw: m.raw,
                                            present: m.present,
                                        }
                                    }),
                                    modifier_metadata: p.efficiency.modifier_metadata,
                                    feature_flags: p.efficiency.feature_flags,
                                    eligibility_callback: p.efficiency.eligibility_callback,
                                },
                                kills: range(&self.plane_kills.first, p.kills)
                                    .iter()
                                    .map(|k| rows::NavalCombatAirWingKill {
                                        wing: k.wing,
                                        index: k.index,
                                        category: k.category,
                                        count: k.count,
                                    })
                                    .collect(),
                                names: range(&self.plane_names.first, p.names)
                                    .iter()
                                    .map(|n| n.as_bytes().to_vec())
                                    .collect(),
                                air_hits: p.air_hits.map(|span| {
                                    range(&self.plane_air.first, span)
                                        .iter()
                                        .map(air_hit)
                                        .collect()
                                }),
                                naval_hits: range(&self.plane_hits.first, p.naval_hits)
                                    .iter()
                                    .map(naval_hit)
                                    .collect(),
                            })
                            .collect(),
                    }),
                }
            }),
            table::<tables::NavalCombatShips, _>(out[1], &self.ships.rows, |r| self.ship(r)),
        ]
    }
    fn ship(&self, r: &cfex_member::DetailsRow) -> rows::NavalCombatShipRow {
        rows::NavalCombatShipRow {
            combat: r.member.combat,
            side: r.member.side,
            battle_slot: r.member.battle_slot,
            ship: r.member.ship,
            owner: r.member.owner,
            state: r.member.state,
            convoy: r.convoy,
            subunit_index: r.subunit_index,
            formation_type: r.formation_type,
            build_cost_ic: r.build_cost_ic,
            cached_strength_ratio: r.cached_strength_ratio,
            pride: r.pride,
            is_convoy: r.is_convoy,
            name: r.name.as_bytes().to_vec(),
            type_name: r.type_name.as_bytes().to_vec(),
            sprite: r.sprite.as_bytes().to_vec(),
            sunk_by: r.sunk_by.as_bytes().to_vec(),
            variant: r.variant,
            arrival_hours: r.arrival_hours,
            received_criticals: r.received_criticals,
            evacuation_state: r.evacuation_state,
            detection_state: r.detection_state,
            escape_progress: r.escape_progress,
            received_damage: r.received_damage,
            live_ship: r.live_ship,
            strength: r.strength,
            organisation: r.organisation,
            base_organisation: r.base_organisation,
            maximum_hit_points: r.maximum_hit_points,
            formation_flags: r.formation_flags,
            member_source: r.member_source,
            group_index: r.group_index,
            member_index: r.member_index,
            live_convoy: r.live_convoy,
            last_target: rows::NavalCombatLastTarget {
                present: r.last_target.present,
                is_convoy: r.last_target.is_convoy,
                target_group_member_count: r.last_target.target_group_member_count,
            },
            weapon_context: rows::NavalShipWeaponContext {
                task_force: r.weapon_context.task_force,
                country: r.weapon_context.country,
                commander: r.weapon_context.commander,
                ship_class: r.weapon_context.ship_class,
                subunit: r.weapon_context.subunit,
                weapon_bases: r.weapon_context.weapon_bases,
                detection_base: r.weapon_context.detection_base,
                air_base: r.weapon_context.air_base,
                air_base_maximum: r.weapon_context.air_base_maximum,
                air_detection_base: r.weapon_context.air_detection_base,
                weather_factor: r.weapon_context.weather_factor,
                fuel: r.weapon_context.fuel,
                night: r.weapon_context.night,
                formation_flags: r.weapon_context.formation_flags,
                terrain_name: r.weapon_context.terrain_name.as_bytes().to_vec(),
                sources: range(&self.weapon_sources.first, r.weapon_context.sources)
                    .iter()
                    .map(|s| rows::NavalShipModifierSource {
                        scope: s.scope,
                        index: s.index,
                        selector: s.selector,
                        reference: s.reference,
                        dynamic_keys: s.dynamic_keys,
                        weights: s.weights,
                        terms: range(&self.weapon_terms.first, s.terms)
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
            },
            damage_entries: range(&self.damage.first, r.damage_entries)
                .iter()
                .map(|v| rows::NavalCombatDamage {
                    source: v.source,
                    country: v.country,
                    damage: v.damage,
                })
                .collect(),
            air_hits: r
                .air_hits
                .map(|span| range(&self.air.first, span).iter().map(air_hit).collect()),
            naval_hits: range(&self.hits.first, r.naval_hits)
                .iter()
                .map(naval_hit)
                .collect(),
        }
    }
}
/// Spool-thread commander projection, shared with retained naval results.
pub(super) fn commander_inputs(
    c: &engine::naval_commander_inputs::Row,
) -> rows::NavalCommanderInputs {
    rows::NavalCommanderInputs {
        commander: c.commander,
        country: c.country,
        category: c.category,
        experience: c.experience,
        skills: c.skills,
        skill_definitions: c.skill_definitions.map(commander_skill),
        next_skill: commander_skill(c.next_skill),
        name: c.name[..c.name_len].to_vec(),
        description: c.description[..c.description_len].to_vec(),
        presentation: super::combat_details::commander_presentation(&c.presentation),
        traits: c.traits[..c.traits_len]
            .iter()
            .map(|t| rows::LandCombatCommanderTrait {
                token_id: t.token_id,
                definition_ordinal: t.definition_ordinal,
                kind: t.kind,
                relevant: t.relevant,
                token: c.trait_text[t.token_start..t.token_start + t.token_len].to_vec(),
            })
            .collect(),
        learning_traits: c.learning_traits[..c.learning_traits_len]
            .iter()
            .map(|t| rows::LandCombatCommanderLearningTrait {
                token_id: t.token_id,
                definition_ordinal: t.definition_ordinal,
                required_experience: t.required_experience,
                experience: t.experience,
                token: c.learning_text[t.token_start..t.token_start + t.token_len].to_vec(),
            })
            .collect(),
        learning_threshold: c.learning_threshold,
        modifier_feature_flags: c.modifier_feature_flags,
        modifier_callback_kind: c.modifier_callback_kind,
        modifiers: c.modifiers[..c.modifiers_len]
            .iter()
            .map(|m| rows::LandCombatCommanderPortraitModifier {
                source: m.source,
                id: m.id,
                value: m.value,
                metadata_flags: m.metadata_flags,
                category_mask: m.category_mask,
                feature_selector: m.feature_selector,
            })
            .collect(),
        tooltips: c.tooltips[..c.tooltips_len]
            .iter()
            .map(|t| rows::LandCombatCommanderPortraitTooltip {
                source: t.source,
                bucket: t.bucket,
                key: t.key,
                text: c.tooltip_text[t.text_start..t.text_start + t.text_len].to_vec(),
            })
            .collect(),
    }
}
pub(super) fn commander_skill(
    s: engine::naval_commander_inputs::SkillDefinition,
) -> rows::CombatCommanderSkillDefinition {
    rows::CombatCommanderSkillDefinition {
        present: s.present,
        valid: s.valid,
        level: s.level,
        required_experience: s.required_experience,
        category: s.category,
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
    fn arena_pressure_grows_primary_storage_and_preserves_rows() {
        let mut capacity = Capacities {
            details: 1,
            ships: 1,
            groups: 1,
            damage: 1,
            air: 1,
            hits: 1,
            planes: 1,
            plane_air: 1,
            plane_hits: 1,
            plane_names: 1,
            plane_kills: 1,
            commanders: 1,
            weapon_sources: 1,
            weapon_terms: 1,
            side_units: 1,
        };
        let mut captured = Captured::with_capacities(&capacity);
        captured.details.rows[0].combat = 42;
        captured.ships.rows[0].member.combat = 42;
        let before = captured.allocated_bytes();
        // The worker may grow these from original count observations before
        // any outcome exists; recycling must adopt all actual allocations.
        captured.ships.rows.resize(19, Default::default());
        captured.groups.resize(17, 0);
        captured.damage.first.resize(13, Default::default());
        captured.air.first.resize(9, Default::default());
        captured.hits.first.resize(11, Default::default());
        captured.weapon_sources.first.resize(7, Default::default());
        captured.weapon_terms.first.resize(21, Default::default());
        assert!(captured.allocated_bytes() > before);
        let worker_capacities = [
            captured.ships.rows.capacity(),
            captured.groups.capacity(),
            captured.damage.first.capacity(),
            captured.air.first.capacity(),
            captured.hits.first.capacity(),
            captured.weapon_sources.first.capacity(),
            captured.weapon_terms.first.capacity(),
        ];
        captured.plane_air.first[0].variant = 111;
        captured.plane_air.required = 9;
        captured.plane_hits.required = 5;
        captured.plane_names.required = 6;
        captured.plane_kills.required = 7;
        captured.commanders.required = 3;
        captured.side_units.required = 4;
        captured.grow(&mut capacity);
        assert_eq!(
            [
                capacity.ships,
                capacity.groups,
                capacity.damage,
                capacity.air,
                capacity.hits,
                capacity.weapon_sources,
                capacity.weapon_terms
            ],
            worker_capacities
        );
        assert_eq!(Captured::vacant().allocated_bytes(), 0);
        assert!(capacity.plane_air >= 9 && capacity.plane_hits >= 5);
        assert!(capacity.plane_names >= 6 && capacity.plane_kills >= 7 && capacity.commanders >= 3);
        assert!(capacity.side_units >= 4 && captured.side_units.first.len() == capacity.side_units);
        assert_eq!(captured.plane_air.first.len(), capacity.plane_air);
        assert_eq!(captured.details.rows[0].combat, 42);
        assert_eq!(captured.ships.rows[0].member.combat, 42);
        assert_eq!(captured.plane_air.first[0].variant, 111);
        captured.details.outcome = Outcome::Rows { len: 0 };
        captured.ships.outcome = Outcome::Unavailable {
            reason: "native ship read failed",
        };
        assert_eq!(captured.outcomes()[0], Outcome::Rows { len: 0 });
        assert!(matches!(
            captured.outcomes()[1],
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn plane_and_ship_hit_spans_project_from_independent_arenas() {
        let mut c = Captured::with_capacities(&Capacities::default());
        c.details.outcome = Outcome::Rows { len: 1 };
        c.ships.outcome = Outcome::Rows { len: 1 };
        c.planes.len = 1;
        c.plane_air.len = 1;
        c.plane_hits.len = 1;
        c.air.len = 1;
        c.hits.len = 1;
        c.weapon_sources.len = 1;
        c.weapon_terms.len = 1;
        c.details.rows[0].combat = 42;
        c.details.rows[0].sides[0].planes = [0, 1];
        c.planes.first[0].air_hits[0] = [0, 1];
        c.planes.first[0].naval_hits = [0, 1];
        c.plane_air.first[0].variant = 111;
        c.plane_hits.first[0].target_slot = 12;
        c.ships.rows[0].member.combat = 42;
        c.ships.rows[0].member.battle_slot = 5;
        c.ships.rows[0].air_hits[0] = [0, 1];
        c.ships.rows[0].naval_hits = [0, 1];
        c.ships.rows[0].weapon_context.sources = [0, 1];
        c.weapon_sources.first[0].scope = 3;
        c.weapon_sources.first[0].reference = 0x1234_0000_5678;
        c.weapon_sources.first[0].terms = [0, 1];
        c.weapon_terms.first[0].key = 12;
        c.weapon_terms.first[0].raw = -456;
        c.air.first[0].variant = 222;
        c.hits.first[0].target_slot = 34;
        let [
            TableView::Complete(rows::TableRows::NavalCombatDetails(details)),
            TableView::Complete(rows::TableRows::NavalCombatShips(ships)),
        ] = c.tables()
        else {
            panic!("complete naval operand rows were not projected")
        };
        assert_eq!(details[0].sides[0].planes[0].air_hits[0][0].variant, 111);
        assert_eq!(details[0].sides[0].planes[0].naval_hits[0].target_slot, 12);
        assert_eq!(ships[0].air_hits[0][0].variant, 222);
        assert_eq!(ships[0].naval_hits[0].target_slot, 34);
        assert_eq!(ships[0].weapon_context.sources[0].scope, 3);
        assert_eq!(
            ships[0].weapon_context.sources[0].reference,
            0x1234_0000_5678
        );
        assert_eq!(ships[0].weapon_context.sources[0].terms[0].raw, -456);
    }
}
