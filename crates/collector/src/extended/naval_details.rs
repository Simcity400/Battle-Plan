//! Hourly naval combat inputs. Ship rows and their arenas grow on the
//! acquisition worker from their original observations; projection is deferred.
use crate::{
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows, grow_capacity, grow_vec},
};
use engine::naval_air_efficiency_inputs;
use engine::{
    Memory, cfex_air, cfex_damage_entry, cfex_member, cnaval_combat, naval_combat_air_base_wings,
    naval_combat_air_hit, naval_combat_hit, naval_ship_modifier_inputs, walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};
pub const N: usize = 4;
pub const TABLES: [TableId; N] = [
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalCombatAirBaseWingHandles,
    TableId::NavalModifierBreakdowns,
];
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("shared naval combat registry census", 0, &[0, 1]),
    ("dense combat air base wing occurrences", 2, &[2]),
    ("live naval hover modifier child trees", 3, &[3]),
];
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
    plane_kills: usize,
    commanders: usize,
    weapon_sources: usize,
    weapon_terms: usize,
    dense_bases: usize,
    dense_wings: usize,
    side_units: usize,
    custom_sea: usize,
    damage_sources: usize,
    wing_types: usize,
    plane_wings: usize,
    breakdown_requests: usize,
    breakdown_roots: usize,
    breakdown_nodes: usize,
    breakdown_text: usize,
    breakdown_depth: usize,
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
            plane_kills: 1024,
            commanders: 16,
            weapon_sources: 4096,
            weapon_terms: 16384,
            dense_bases: 128,
            dense_wings: 1024,
            side_units: 256,
            custom_sea: 256,
            damage_sources: 256,
            wing_types: 1024,
            plane_wings: 2048,
            breakdown_requests: 0,
            breakdown_roots: 0,
            breakdown_nodes: 0,
            breakdown_text: 0,
            breakdown_depth: 0,
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
    plane_kills: Arena<naval_air_efficiency_inputs::WingKill>,
    commanders: Arena<engine::naval_commander_inputs::Row>,
    weapon_sources: Arena<naval_ship_modifier_inputs::SourceRow>,
    weapon_terms: Arena<naval_ship_modifier_inputs::ModifierTerm>,
    /// Air bases the hour's combats reference, in first-reference order.
    dense_bases: CaptureRows<naval_combat_air_base_wings::Row>,
    dense_wings: Arena<u64>,
    base_handles: naval_combat_air_base_wings::UniqueHandles,
    /// Each side's task-force handles (`CCombatant+0x20`), grown after the hour.
    side_units: Arena<u64>,
    custom_sea: Arena<i16>,
    damage_sources: Arena<(u32, i64)>,
    wing_types: Arena<u32>,
    plane_wings: Arena<u64>,
    breakdown_requests: Arena<engine::naval_ship_modifier_breakdown::Request>,
    breakdown_roots: Arena<engine::naval_ship_modifier_breakdown::Root>,
    breakdown_nodes: Arena<engine::naval_ship_modifier_breakdown::RawNode>,
    breakdown_text: Arena<u8>,
    breakdown_depth: Arena<engine::naval_ship_modifier_breakdown::Frame>,
    breakdown_outcome: Outcome,
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
            plane_kills: 0,
            commanders: 0,
            weapon_sources: 0,
            weapon_terms: 0,
            dense_bases: 0,
            dense_wings: 0,
            side_units: 0,
            custom_sea: 0,
            damage_sources: 0,
            wing_types: 0,
            plane_wings: 0,
            breakdown_requests: 0,
            breakdown_roots: 0,
            breakdown_nodes: 0,
            breakdown_text: 0,
            breakdown_depth: 0,
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
            plane_kills: Arena::new(c.plane_kills),
            commanders: Arena::new(c.commanders),
            weapon_sources: Arena::new(c.weapon_sources),
            weapon_terms: Arena::new(c.weapon_terms),
            dense_bases: CaptureRows::new(c.dense_bases),
            dense_wings: Arena::new(c.dense_wings),
            base_handles: naval_combat_air_base_wings::UniqueHandles::with_capacity(c.dense_bases),
            side_units: Arena::new(c.side_units),
            custom_sea: Arena::new(c.custom_sea),
            damage_sources: Arena::new(c.damage_sources),
            wing_types: Arena::new(c.wing_types),
            plane_wings: Arena::new(c.plane_wings),
            breakdown_requests: Arena::new(c.breakdown_requests),
            breakdown_roots: Arena::new(c.breakdown_roots),
            breakdown_nodes: Arena::new(c.breakdown_nodes),
            breakdown_text: Arena::new(c.breakdown_text),
            breakdown_depth: Arena::new(c.breakdown_depth),
            breakdown_outcome: Outcome::Rows { len: 0 },
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
        self.plane_kills.reset();
        self.commanders.reset();
        self.weapon_sources.reset();
        self.weapon_terms.reset();
        self.dense_bases.reset();
        self.dense_wings.reset();
        self.base_handles.clear();
        self.side_units.reset();
        self.custom_sea.reset();
        self.damage_sources.reset();
        self.wing_types.reset();
        self.plane_wings.reset();
        self.breakdown_requests.reset();
        self.breakdown_roots.reset();
        self.breakdown_nodes.reset();
        self.breakdown_text.reset();
        self.breakdown_depth.reset();
        self.breakdown_outcome = Outcome::Rows { len: 0 };
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
            + self.plane_kills.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self.weapon_sources.allocated_bytes()
            + self.weapon_terms.allocated_bytes()
            + self.dense_bases.allocated_bytes()
            + self.dense_wings.allocated_bytes()
            + self.base_handles.allocated_bytes()
            + self.side_units.allocated_bytes()
            + self.custom_sea.allocated_bytes()
            + self.damage_sources.allocated_bytes()
            + self.wing_types.allocated_bytes()
            + self.plane_wings.allocated_bytes()
            + self.breakdown_requests.allocated_bytes()
            + self.breakdown_roots.allocated_bytes()
            + self.breakdown_nodes.allocated_bytes()
            + self.breakdown_text.allocated_bytes()
            + self.breakdown_depth.allocated_bytes()
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
        // These owned arenas grew from successfully read entries in the
        // worker. A failed payload's declared count must not size a pool.
        grow_vec(&mut self.damage.first, c.damage);
        grow_vec(&mut self.air.first, c.air);
        grow_vec(&mut self.hits.first, c.hits);
        grow_capacity(&mut c.groups, self.required_groups);
        grow_vec(&mut self.groups, c.groups);
        self.planes.grow(&mut c.planes);
        self.plane_air.grow(&mut c.plane_air);
        self.plane_hits.grow(&mut c.plane_hits);
        self.plane_kills.grow(&mut c.plane_kills);
        self.commanders.grow(&mut c.commanders);
        grow_vec(&mut self.weapon_sources.first, c.weapon_sources);
        grow_vec(&mut self.weapon_terms.first, c.weapon_terms);
        self.dense_bases.grow(&mut c.dense_bases);
        self.dense_wings.grow(&mut c.dense_wings);
        if self.base_handles.len() >= c.dense_bases.saturating_sub(c.dense_bases / 4) {
            grow_capacity(
                &mut c.dense_bases,
                self.base_handles.len().saturating_add(1),
            );
        }
        if self.base_handles.capacity() < c.dense_bases {
            self.base_handles.reserve(c.dense_bases);
        }
        self.side_units.grow(&mut c.side_units);
        self.custom_sea.grow(&mut c.custom_sea);
        self.damage_sources.grow(&mut c.damage_sources);
        self.wing_types.grow(&mut c.wing_types);
        self.plane_wings.grow(&mut c.plane_wings);
        self.breakdown_requests.grow(&mut c.breakdown_requests);
        self.breakdown_roots.grow(&mut c.breakdown_roots);
        self.breakdown_nodes.grow(&mut c.breakdown_nodes);
        self.breakdown_text.grow(&mut c.breakdown_text);
        self.breakdown_depth.grow(&mut c.breakdown_depth);
    }

    /// Select only bases used by native naval-combat calculations in this
    /// complete hourly combat census. Positions read active forward planes;
    /// surface detection reads live ship carrier bases. The same base appears
    /// once in the capture table even if multiple combat objects reference it.
    fn select_dense_base_handles(&mut self) -> Result<(), &'static str> {
        let (Outcome::Rows { len: details_len }, Outcome::Rows { len: ships_len }) =
            (self.details.outcome, self.ships.outcome)
        else {
            return Err("naval combat air base source incomplete");
        };
        let details = self
            .details
            .rows
            .get(..details_len)
            .ok_or("naval combat air base detail source out of bounds")?;
        let ships = self
            .ships
            .rows
            .get(..ships_len)
            .ok_or("naval combat air base ship source out of bounds")?;
        let planes = self
            .planes
            .first
            .get(..self.planes.len)
            .ok_or("naval combat air base plane source out of bounds")?;
        self.base_handles.clear();
        for detail in details {
            for side in &detail.sides {
                let [start, count] = side.planes;
                let end = start
                    .checked_add(count)
                    .ok_or("naval combat air base plane span overflow")?;
                let source = planes
                    .get(start..end)
                    .ok_or("naval combat air base plane span out of bounds")?;
                for plane in source {
                    if plane.source == 0
                        && plane.group_index != u32::MAX
                        && plane.flags[1] == 0
                        && matches!(plane.efficiency.source, 2 | 3)
                    {
                        self.base_handles.insert(plane.air_base);
                    }
                }
            }
        }
        for ship in ships {
            let handle = ship.weapon_context.air_base;
            if ship.live_ship != 0 && handle != 0 {
                self.base_handles.insert(handle);
            }
        }
        Ok(())
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
            custom_sea: &mut self.custom_sea.first,
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: &mut self.damage_sources.first,
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: &mut self.wing_types.first,
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: &mut self.plane_wings.first,
            plane_wings_len: 0,
            plane_wings_required: 0,
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
        self.custom_sea.len = buffers.custom_sea_len;
        self.custom_sea.required = buffers.custom_sea_required;
        self.damage_sources.len = buffers.damage_sources_len;
        self.damage_sources.required = buffers.damage_sources_required;
        self.wing_types.len = buffers.wing_types_len;
        self.wing_types.required = buffers.wing_types_required;
        self.plane_wings.len = buffers.plane_wings_len;
        self.plane_wings.required = buffers.plane_wings_required;
        self.required_groups = first.capture.required_group_capacity;
        let dense_started = qpc();
        read_phase.select(TABLES[2]);
        self.dense_bases.outcome = match self.select_dense_base_handles() {
            Ok(()) => {
                let walked = naval_combat_air_base_wings::walk(
                    memory,
                    base,
                    &self.base_handles,
                    &mut self.dense_bases.rows,
                    &mut self.dense_wings.first,
                );
                self.dense_wings.len = walked.wings_len;
                self.dense_wings.required = walked.required_wings;
                walked.outcome
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        let mut ticks = [first.read_ticks[0], first.read_ticks[1], 0];
        ticks[2] = qpc().saturating_sub(dense_started).max(0) as u64;
        let breakdown_started = qpc();
        read_phase.select(TABLES[3]);
        let mut breakdown = engine::naval_ship_modifier_breakdown::Buffers {
            requests: &mut self.breakdown_requests.first,
            request_len: 0,
            request_required: 0,
            roots: &mut self.breakdown_roots.first,
            root_len: 0,
            root_required: 0,
            nodes: &mut self.breakdown_nodes.first,
            node_len: 0,
            node_required: 0,
            text: &mut self.breakdown_text.first,
            text_len: 0,
            text_required: 0,
            stack: &mut self.breakdown_depth.first,
            stack_required: 0,
        };
        self.breakdown_outcome = match (self.details.outcome, self.ships.outcome) {
            (Outcome::Rows { len: details_len }, Outcome::Rows { len: ships_len }) => {
                match engine::naval_ship_modifier_breakdown::capture(
                    memory,
                    base,
                    state,
                    &self.details.rows[..details_len],
                    &self.ships.rows[..ships_len],
                    &self.planes.first[..self.planes.len],
                    &self.weapon_sources.first[..self.weapon_sources.len],
                    &mut breakdown,
                ) {
                    Ok(()) => Outcome::Rows {
                        len: breakdown.root_len,
                    },
                    Err(reason) => Outcome::Unavailable { reason },
                }
            }
            _ => Outcome::Unavailable {
                reason: "naval modifier breakdown live source incomplete",
            },
        };
        self.breakdown_requests.len = breakdown.request_len;
        self.breakdown_requests.required = breakdown.request_required;
        self.breakdown_roots.len = breakdown.root_len;
        self.breakdown_roots.required = breakdown.root_required;
        self.breakdown_nodes.len = breakdown.node_len;
        self.breakdown_nodes.required = breakdown.node_required;
        self.breakdown_text.len = breakdown.text_len;
        self.breakdown_text.required = breakdown.text_required;
        self.breakdown_depth.required = breakdown.stack_required;
        let mut ticks = [ticks[0], ticks[1], ticks[2], 0];
        ticks[3] = qpc().saturating_sub(breakdown_started).max(0) as u64;
        let total = qpc().saturating_sub(capture_started).max(0) as u64;
        ticks[0] = ticks[0].saturating_add(total.saturating_sub(ticks.iter().sum::<u64>()));
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.details.outcome,
            self.ships.outcome,
            self.dense_bases.outcome,
            self.breakdown_outcome,
        ]
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
                    custom_sea: r.custom_sea.map(|span| {
                        range(&self.custom_sea.first, span)
                            .iter()
                            .flat_map(|value| value.to_le_bytes())
                            .collect()
                    }),
                    custom_sea_error: u8::from(r.custom_sea_error.is_some()),
                    sides: r.sides.each_ref().map(|s| rows::NavalCombatSideDetails {
                        damage_history: s.damage_history,
                        original_health: s.original_health,
                        total_damage: s.total_damage,
                        damage_by_type: range(&self.damage_sources.first, s.damage_by_type)
                            .iter()
                            .map(|&(token, damage)| rows::NavalDamageSource { token, damage })
                            .collect(),
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
                            countries: range(&self.summary_countries, s.summary.countries).to_vec(),
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
                        },
                        planes: range(&self.planes.first, s.planes)
                            .iter()
                            .map(|p| rows::NavalCombatPlane {
                                wing_types: range(&self.wing_types.first, p.wing_types).to_vec(),
                                source: p.source,
                                group_index: p.group_index,
                                index: p.index,
                                carrier_slot: p.carrier_slot,
                                carrier_member_present: p.carrier_member_present,
                                carrier_state: p.carrier_state,
                                air_base: p.air_base,
                                casualties: p.casualties,
                                time_duration: p.time_duration,
                                wings: p
                                    .wings
                                    .map(|span| range(&self.plane_wings.first, span).to_vec()),
                                initial_planes: p.initial_planes,
                                current_planes: p.current_planes,
                                country: p.country,
                                date: p.date,
                                flags: p.flags,
                                variant: p.variant,
                                efficiency: rows::NavalCombatAirEfficiency {
                                    source: p.efficiency.source,
                                    pool_type_flags: p.efficiency.pool_type_flags,
                                    mission: p.efficiency.mission,
                                    container_present: p.efficiency.container_present,
                                    base_crowding: p.efficiency.base_crowding,
                                    night: p.efficiency.night,
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
            table::<tables::NavalCombatAirBaseWingHandles, _>(
                out[2],
                &self.dense_bases.rows,
                |r| rows::NavalCombatAirBaseWingHandlesRow {
                    base: r.base,
                    wings: range(&self.dense_wings.first, [r.wings_start, r.wings_len]).to_vec(),
                },
            ),
            self.breakdown_table(),
        ]
    }
    fn breakdown_table(&self) -> TableView {
        if let Outcome::Unavailable { reason } = self.breakdown_outcome {
            return unavailable(reason);
        }
        match self.project_breakdowns() {
            Ok(projected) => table::<tables::NavalModifierBreakdowns, _>(
                Outcome::Rows {
                    len: projected.len(),
                },
                &projected,
                Clone::clone,
            ),
            Err(reason) => unavailable(reason),
        }
    }
    /// Source joins, zero-subtree pruning, string copies and row assembly run
    /// on the spool thread, after the hourly game block has returned.
    fn project_breakdowns(&self) -> Result<Vec<rows::NavalModifierBreakdownRow>, &'static str> {
        use engine::naval_ship_modifier_breakdown::KEYS;
        let roots = &self.breakdown_roots.first[..self.breakdown_roots.len];
        let requests = &self.breakdown_requests.first[..self.breakdown_requests.len];
        let nodes = &self.breakdown_nodes.first[..self.breakdown_nodes.len];
        let names = &self.breakdown_text.first[..self.breakdown_text.len];
        let mut result = Vec::new();
        let mut at = 0;
        for root in roots {
            let raw = range(nodes, root.nodes);
            if raw.is_empty() {
                return Err("naval modifier root has no nodes");
            }
            while at < requests.len() && requests[at].accumulator == root.accumulator {
                let key = requests[at].key;
                let key_index = KEYS
                    .binary_search(&key)
                    .map_err(|_| "naval modifier request key unsupported")?;
                let first = at;
                while at < requests.len()
                    && requests[at].accumulator == root.accumulator
                    && requests[at].key == key
                {
                    at += 1;
                }
                let mut cursor = 0;
                let mut projected = Vec::new();
                copy_visible_subtree(raw, names, &mut cursor, key_index, true, &mut projected)?;
                if cursor != raw.len() {
                    return Err("naval modifier tree has extra nodes");
                }
                // A childless projected interior root must stay interior for
                // 14054D0F0's formatting gate, while 1415B4BB0 still reads
                // its native value for the convoy escape sum.
                if raw[0].children != 0 && projected[0].children == 0 {
                    projected[0].children = 1;
                    projected.push(rows::ModifierBreakdownNode::default());
                }
                result.push(rows::NavalModifierBreakdownRow {
                    accumulator: root.accumulator as u64,
                    key,
                    uses: requests[first..at]
                        .iter()
                        .map(|request| rows::NavalModifierUse {
                            combat: request.usage.combat,
                            side: request.usage.side,
                            battle_slot: request.usage.battle_slot,
                            source_scope: request.usage.source_scope,
                            source_index: request.usage.source_index,
                            order: request.usage.order,
                        })
                        .collect(),
                    nodes: projected,
                });
            }
        }
        if at != requests.len() {
            return Err("naval modifier request has no captured root");
        }
        Ok(result)
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
            escape_speed_base: r.escape_speed_base,
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
                        movement: s.movement,
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
fn copy_visible_subtree(
    raw: &[engine::naval_ship_modifier_breakdown::RawNode],
    names: &[u8],
    cursor: &mut usize,
    key: usize,
    root: bool,
    output: &mut Vec<rows::ModifierBreakdownNode>,
) -> Result<bool, &'static str> {
    let node = raw
        .get(*cursor)
        .ok_or("naval modifier preorder is incomplete")?;
    *cursor += 1;
    let start = output.len();
    output.push(rows::ModifierBreakdownNode::default());
    let mut kept_children = 0_u32;
    for _ in 0..node.children {
        if copy_visible_subtree(raw, names, cursor, key, false, output)? {
            kept_children += 1;
        }
    }
    if !root && (node.children != 0 || node.values[key] == 0) && kept_children == 0 {
        output.truncate(start);
        return Ok(false);
    }
    let end = node.name[0]
        .checked_add(node.name[1])
        .ok_or("naval modifier display span overflow")?;
    output[start] = rows::ModifierBreakdownNode {
        children: kept_children,
        value: node.values[key],
        name: names
            .get(node.name[0]..end)
            .ok_or("naval modifier display span invalid")?
            .to_vec(),
    };
    Ok(true)
}

pub(super) fn air_hit(v: &naval_combat_air_hit::Row) -> rows::NavalCombatAirHit {
    rows::NavalCombatAirHit {
        variant: v.variant,
        count: v.count,
        country: v.country,
    }
}
pub(super) fn naval_hit(v: &naval_combat_hit::Row) -> rows::NavalCombatHit {
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
    fn shared_root_projection_prunes_zero_branches_and_keeps_combat_uses() {
        use engine::naval_ship_modifier_breakdown::{RawNode, Request, Root, Use};
        let mut captured = Captured::with_capacities(&Capacities {
            breakdown_requests: 2,
            breakdown_roots: 1,
            breakdown_nodes: 3,
            breakdown_text: 8,
            ..Capacities::default()
        });
        captured.breakdown_roots.first[0] = Root {
            accumulator: 0x1230,
            nodes: [0, 3],
        };
        captured.breakdown_roots.len = 1;
        captured.breakdown_nodes.first[..3].copy_from_slice(&[
            RawNode {
                children: 2,
                ..Default::default()
            },
            RawNode {
                children: 0,
                name: [0, 4],
                ..Default::default()
            },
            RawNode {
                children: 0,
                name: [4, 4],
                values: [0, 0, 0, 0, 0, 0, 900, 0, 0],
            },
        ]);
        captured.breakdown_nodes.len = 3;
        captured.breakdown_text.first[..8].copy_from_slice(b"zerokept");
        captured.breakdown_text.len = 8;
        for (index, combat) in [11, 22].into_iter().enumerate() {
            captured.breakdown_requests.first[index] = Request {
                accumulator: 0x1230,
                key: 0x1b2,
                usage: Use {
                    combat,
                    side: 0,
                    battle_slot: u32::MAX,
                    order: index as u32,
                    ..Default::default()
                },
            };
        }
        captured.breakdown_requests.len = 2;
        let rows = captured.project_breakdowns().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]
                .uses
                .iter()
                .map(|usage| usage.combat)
                .collect::<Vec<_>>(),
            [11, 22]
        );
        assert_eq!(rows[0].nodes.len(), 2);
        assert_eq!(rows[0].nodes[0].children, 1);
        assert_eq!(rows[0].nodes[1].name, b"kept");
        assert_eq!(rows[0].nodes[1].value, 900);
        captured.breakdown_nodes.first[0].values[6] = 123;
        captured.breakdown_nodes.first[2].values[6] = 0;
        let rows = captured.project_breakdowns().unwrap();
        assert_eq!(rows[0].nodes.len(), 2);
        assert_eq!(rows[0].nodes[0].children, 1);
        assert_eq!(rows[0].nodes[0].value, 123);
        assert_eq!(rows[0].nodes[1].value, 0);
    }
    #[test]
    fn dense_base_selection_uses_only_active_forward_planes_and_live_carrier_ships() {
        let mut c = Captured::with_capacities(&Capacities::default());
        c.details.outcome = Outcome::Rows { len: 1 };
        c.ships.outcome = Outcome::Rows { len: 2 };
        c.details.rows[0].sides[0].planes = [0, 3];
        c.planes.len = 3;
        for (index, plane) in c.planes.first[..3].iter_mut().enumerate() {
            plane.air_base = (index as u64 + 1) << 32 | 65;
            plane.efficiency.source = 2;
        }
        c.planes.first[1].source = 1;
        c.planes.first[2].flags[1] = 1;
        c.ships.rows[0].live_ship = 1;
        c.ships.rows[0].weapon_context.air_base = c.planes.first[0].air_base;
        c.ships.rows[1].live_ship = 1;
        c.ships.rows[1].weapon_context.air_base = 4 << 32 | 65;
        c.select_dense_base_handles().unwrap();
        assert_eq!(c.base_handles.as_slice(), [1 << 32 | 65, 4 << 32 | 65]);

        c.dense_bases.outcome = Outcome::Rows { len: 2 };
        c.dense_bases.rows[0] = naval_combat_air_base_wings::Row {
            base: c.base_handles.as_slice()[0],
            wings_start: 0,
            wings_len: 2,
        };
        c.dense_bases.rows[1] = naval_combat_air_base_wings::Row {
            base: c.base_handles.as_slice()[1],
            wings_start: 2,
            wings_len: 0,
        };
        c.dense_wings.first[..2].copy_from_slice(&[69, 69]);
        c.dense_wings.len = 2;
        let TableView::Complete(rows::TableRows::NavalCombatAirBaseWingHandles(rows)) =
            &c.tables()[2]
        else {
            panic!("complete dense base rows not projected");
        };
        assert_eq!(rows[0].wings, [69, 69]);
        assert!(rows[1].wings.is_empty());

        c.ships.outcome = Outcome::Unavailable {
            reason: "ship census incomplete",
        };
        assert_eq!(
            c.select_dense_base_handles(),
            Err("naval combat air base source incomplete")
        );
    }

    /// One carrier base per country, each referenced by two active planes on
    /// both sides of a combat and by its live carrier ship: selection is one
    /// indexed membership test per reference, in first-reference order, with
    /// no allocation once the hour's capacity is reserved.
    #[test]
    fn dense_base_selection_is_linear_at_country_scale() {
        for countries in [1_500_usize, 5_000] {
            let planes = 2 * countries;
            let mut c = Captured::with_capacities(&Capacities {
                details: 1,
                ships: countries,
                planes,
                dense_bases: countries,
                ..Capacities::default()
            });
            let base = |country: usize| ((country as u64 + 1) << 32) | 65;
            c.details.outcome = Outcome::Rows { len: 1 };
            c.details.rows[0].sides[0].planes = [0, countries];
            c.details.rows[0].sides[1].planes = [countries, countries];
            c.planes.len = planes;
            for (index, plane) in c.planes.first[..planes].iter_mut().enumerate() {
                // Side 1 references the same bases in reverse.
                let country = if index < countries {
                    index
                } else {
                    planes - 1 - index
                };
                plane.air_base = base(country);
                plane.efficiency.source = 3;
            }
            c.ships.outcome = Outcome::Rows { len: countries };
            for (country, ship) in c.ships.rows[..countries].iter_mut().enumerate() {
                ship.live_ship = 1;
                ship.weapon_context.air_base = base(country);
            }
            let reserved = c.allocated_bytes();
            let started = std::time::Instant::now();
            c.select_dense_base_handles().unwrap();
            let elapsed = started.elapsed();
            assert_eq!(c.base_handles.len(), countries);
            assert!(
                c.base_handles
                    .as_slice()
                    .iter()
                    .enumerate()
                    .all(|(country, &handle)| handle == base(country))
            );
            assert_eq!(c.allocated_bytes(), reserved);
            eprintln!(
                "naval combat air base selection, {countries} countries ({} references): {elapsed:?} (test build)",
                planes + countries
            );
        }
    }

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
            plane_kills: 1,
            commanders: 1,
            weapon_sources: 1,
            weapon_terms: 1,
            dense_bases: 1,
            dense_wings: 1,
            side_units: 1,
            ..Capacities::default()
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
        assert!(capacity.plane_kills >= 7 && capacity.commanders >= 3);
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
        let mut c = Captured::with_capacities(&Capacities {
            breakdown_requests: 1,
            breakdown_roots: 1,
            breakdown_nodes: 1,
            breakdown_text: 13,
            ..Capacities::default()
        });
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
        c.details.rows[0].custom_sea = Some([0, 3]);
        c.custom_sea.first[..3].copy_from_slice(&[-3, 0, 4]);
        c.custom_sea.len = 3;
        c.details.rows[0].sides[0].original_health = 12_000;
        c.details.rows[0].sides[0].damage_history[2][3] = 750;
        c.details.rows[0].sides[0].damage_by_type = [0, 1];
        c.damage_sources.first[0] = (17, 750);
        c.damage_sources.len = 1;
        c.details.rows[0].sides[0].planes = [0, 1];
        c.planes.first[0].wing_types = [0, 2];
        c.wing_types.first[..2].copy_from_slice(&[17, 23]);
        c.wing_types.len = 2;
        c.planes.first[0].air_hits[0] = [0, 1];
        c.planes.first[0].naval_hits = [0, 1];
        c.plane_air.first[0].variant = 111;
        c.plane_hits.first[0].target_slot = 12;
        const WINGS: usize = 257;
        c.planes.first[0].wings[0] = [0, WINGS];
        for (slot, value) in c.plane_wings.first[..WINGS]
            .iter_mut()
            .zip(1..=WINGS as u64)
        {
            *slot = value;
        }
        c.plane_wings.len = WINGS;
        c.ships.rows[0].member.combat = 42;
        c.ships.rows[0].member.battle_slot = 5;
        c.ships.rows[0].escape_speed_base = 19_000;
        c.ships.rows[0].air_hits[0] = [0, 1];
        c.ships.rows[0].naval_hits = [0, 1];
        c.ships.rows[0].weapon_context.sources = [0, 1];
        c.weapon_sources.first[0].scope = 3;
        c.weapon_sources.first[0].reference = 0x1234_0000_5678;
        c.weapon_sources.first[0].movement = -123;
        c.weapon_sources.first[0].terms = [0, 1];
        c.weapon_terms.first[0].key = 12;
        c.weapon_terms.first[0].raw = -456;
        c.air.first[0].variant = 222;
        c.hits.first[0].target_slot = 34;
        c.breakdown_outcome = Outcome::Rows { len: 1 };
        c.breakdown_requests.first[0] = engine::naval_ship_modifier_breakdown::Request {
            accumulator: 0x1230,
            key: 0x5b,
            usage: engine::naval_ship_modifier_breakdown::Use {
                combat: 42,
                side: 0,
                battle_slot: 5,
                source_scope: 3,
                source_index: 0,
                order: 0,
            },
        };
        c.breakdown_requests.len = 1;
        c.breakdown_roots.first[0] = engine::naval_ship_modifier_breakdown::Root {
            accumulator: 0x1230,
            nodes: [0, 1],
        };
        c.breakdown_roots.len = 1;
        c.breakdown_nodes.first[0] = engine::naval_ship_modifier_breakdown::RawNode {
            children: 0,
            name: [0, 13],
            values: [0, 0, 0, 750, 0, 0, 0, 0, 0],
        };
        c.breakdown_nodes.len = 1;
        c.breakdown_text.first[..13].copy_from_slice(b"stored source");
        c.breakdown_text.len = 13;
        let [
            TableView::Complete(rows::TableRows::NavalCombatDetails(details)),
            TableView::Complete(rows::TableRows::NavalCombatShips(ships)),
            _,
            TableView::Complete(rows::TableRows::NavalModifierBreakdowns(breakdowns)),
        ] = c.tables()
        else {
            panic!("complete naval operand rows were not projected")
        };
        assert_eq!(details[0].sides[0].planes[0].air_hits[0][0].variant, 111);
        assert_eq!(details[0].sides[0].planes[0].naval_hits[0].target_slot, 12);
        assert_eq!(details[0].sides[0].planes[0].wings[0].len(), WINGS);
        assert_eq!(
            details[0].sides[0].planes[0].wings[0][WINGS - 1],
            WINGS as u64
        );
        assert_eq!(
            details[0].custom_sea.as_deref(),
            Some(&[253, 255, 0, 0, 4, 0][..])
        );
        assert_eq!(details[0].sides[0].original_health, 12_000);
        assert_eq!(details[0].sides[0].damage_history[2][3], 750);
        assert_eq!(details[0].sides[0].damage_by_type[0].token, 17);
        assert_eq!(details[0].sides[0].planes[0].wing_types, [17, 23]);
        assert_eq!(ships[0].air_hits[0][0].variant, 222);
        assert_eq!(ships[0].naval_hits[0].target_slot, 34);
        assert_eq!(ships[0].weapon_context.sources[0].scope, 3);
        assert_eq!(
            ships[0].weapon_context.sources[0].reference,
            0x1234_0000_5678
        );
        assert_eq!(ships[0].weapon_context.sources[0].terms[0].raw, -456);
        assert_eq!(ships[0].escape_speed_base, 19_000);
        assert_eq!(ships[0].weapon_context.sources[0].movement, -123);
        assert_eq!(breakdowns[0].nodes[0].name, b"stored source".to_vec());
    }
}

#[cfg(test)]
#[path = "naval_details_scale_tests.rs"]
mod scale_tests;
