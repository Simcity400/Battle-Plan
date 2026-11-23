use crate::{
    clock::qpc,
    table_capture::{Arena, CaptureRows, grow_capacity},
    table_conversion::table,
};
use engine_allocation::{collect_vec, copy_vec, filled, owned_vec, resize_default};
use engine_native_naval::naval_air_efficiency_inputs;
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::{cfex_air, cfex_damage_entry, cfex_member, cnaval_combat};
use engine_native_naval::{
    naval_combat_air_hit,
    naval_combat_hit,
    naval_ship_modifier_inputs,
};
use engine_naval_history::naval_combat_air_base_wings;
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::frame::TableView;
#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static TERM_MEMO_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    pub static SOURCE_MEMO_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn member_phase_data() -> impl serde::Serialize {
    use std::sync::atomic::Ordering::Relaxed;
    let phases = &engine_native_naval::naval_member_details::MEMBER_PHASES;
    let mut frequency = 0;
    let known = unsafe {
        windows_sys::Win32::System::Performance::QueryPerformanceFrequency(&mut frequency)
    } != 0;
    let ns = |ticks: u64| {
        (known && frequency > 0)
            .then(|| (u128::from(ticks) * 1_000_000_000 / frequency as u128) as u64)
    };
    crate::status::status!({
        "members": phases[0].load(Relaxed),
        "members_ns": ns(phases[3].load(Relaxed)),
        "weapon_contexts": phases[1].load(Relaxed),
        "weapon_contexts_ns": ns(phases[2].load(Relaxed)),
        "scope": "cumulative since attach: naval combat member detail reads and their time, and the live ships' weapon contexts (weapon modifier sources) within it",
    })
}

fn source_memo() -> Option<engine_native_naval::naval_ship_modifier_inputs::SourceMemo> {
    #[cfg(any(test, feature = "test-support"))]
    if SOURCE_MEMO_OFF.get() {
        return None;
    }
    Some(Default::default())
}

fn term_metadata() -> engine_native_naval::naval_ship_modifier_inputs::TermMetadata {
    #[cfg(any(test, feature = "test-support"))]
    if TERM_MEMO_OFF.get() {
        return engine_native_naval::naval_ship_modifier_inputs::TermMetadata::unkept();
    }
    Default::default()
}

pub const N: usize = 5;
pub const TABLES: [TableId; N] = [
    TableId::NavalCombatDetails,
    TableId::NavalCombatShips,
    TableId::NavalCombatAirBaseWingHandles,
    TableId::NavalModifierTrees,
    TableId::NavalModifierTreeSamples,
];

#[derive(Clone, Copy, Default, PartialEq, Eq)]
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
    commander_timed_traits: usize,
    weapon_sources: usize,
    weapon_terms: usize,
    dense_bases: usize,
    dense_wings: usize,
    side_units: usize,
    custom_sea: usize,
    summary_countries: usize,
    damage_sources: usize,
    wing_types: usize,
    plane_wings: usize,
}

collector_primitives::capacity_max!(Capacities {
    details,
    ships,
    groups,
    damage,
    air,
    hits,
    planes,
    plane_air,
    plane_hits,
    plane_kills,
    commanders,
    commander_timed_traits,
    weapon_sources,
    weapon_terms,
    dense_bases,
    dense_wings,
    side_units,
    custom_sea,
    summary_countries,
    damage_sources,
    wing_types,
    plane_wings,
});
impl Capacities {
    pub fn raise_for_live_details(
        &mut self,
        demand: engine_naval_history::naval_combat_demand::Demand,
    ) {
        for (capacity, required) in [
            (&mut self.details, demand.rows),
            (&mut self.ships, demand.ships),
            (&mut self.damage, demand.damage),
            (&mut self.air, demand.air_hits),
            (&mut self.hits, demand.naval_hits),
            (&mut self.plane_kills, demand.plane_kills),
            (
                &mut self.commander_timed_traits,
                demand.commander_timed_traits,
            ),
            (&mut self.custom_sea, demand.custom_sea),
            (&mut self.summary_countries, demand.summary_countries),
            (&mut self.dense_bases, demand.dense_bases),
            (&mut self.dense_wings, demand.dense_wings),
            (&mut self.weapon_sources, demand.weapon_sources),
            (&mut self.weapon_terms, demand.weapon_terms),
            (&mut self.groups, demand.groups),
            (&mut self.planes, demand.planes),
            (&mut self.commanders, demand.commanders),
            (&mut self.side_units, demand.side_units),
            (&mut self.damage_sources, demand.damage_sources),
            (&mut self.wing_types, demand.wing_types),
            (&mut self.plane_wings, demand.plane_wings),
            (&mut self.plane_air, demand.plane_air_hits),
            (&mut self.plane_hits, demand.plane_naval_hits),
        ] {
            grow_capacity(capacity, required);
        }
    }
}
pub struct Captured {
    details: CaptureRows<engine_native_naval::naval_combat_details::DetailsRow>,
    ships: CaptureRows<engine_native_naval::naval_member_details::DetailsRow>,
    damage: Arena<cfex_damage_entry::Row>,
    air: Arena<naval_combat_air_hit::Row>,
    hits: Arena<naval_combat_hit::Row>,
    groups: Vec<usize>,
    required_groups: usize,
    planes: Arena<engine_native_naval::naval_air_details::Row>,
    plane_air: Arena<naval_combat_air_hit::Row>,
    plane_hits: Arena<naval_combat_hit::Row>,
    plane_kills: Arena<naval_air_efficiency_inputs::WingKill>,
    commanders: Arena<engine_native_naval::naval_commander_inputs::Row>,
    commander_timed_traits: Arena<engine_native::cunit_leader::TimedTrait>,
    weapon_sources: Arena<naval_ship_modifier_inputs::SourceRow>,
    weapon_terms: Arena<naval_ship_modifier_inputs::ModifierTerm>,
    dense_bases: CaptureRows<naval_combat_air_base_wings::Row>,
    dense_wings: Arena<u64>,
    base_handles: naval_combat_air_base_wings::UniqueHandles,
    side_units: Arena<u64>,
    custom_sea: Arena<i16>,
    damage_sources: Arena<(u32, i64)>,
    wing_types: Arena<u32>,
    plane_wings: Arena<u64>,
    summary_countries: Vec<u32>,
    summary_countries_len: usize,
    trees: super::naval_trees::Trees,
}
impl Captured {
    pub fn try_grow_hint(&mut self, c: &Capacities) -> Result<(), &'static str> {
        crate::table_capture::grow_hint! {
            self.details.rows => c.details,
            self.ships.rows => c.ships,
            self.damage.first => c.damage,
            self.air.first => c.air,
            self.hits.first => c.hits,
            self.plane_kills.first => c.plane_kills,
            self.commander_timed_traits.first => c.commander_timed_traits,
            self.custom_sea.first => c.custom_sea,
            self.summary_countries => c.summary_countries,
            self.dense_bases.rows => c.dense_bases,
            self.dense_wings.first => c.dense_wings,
            self.weapon_sources.first => c.weapon_sources,
            self.weapon_terms.first => c.weapon_terms,
            self.groups => c.groups,
            self.planes.first => c.planes,
            self.commanders.first => c.commanders,
            self.side_units.first => c.side_units,
            self.damage_sources.first => c.damage_sources,
            self.wing_types.first => c.wing_types,
            self.plane_wings.first => c.plane_wings,
            self.plane_air.first => c.plane_air,
            self.plane_hits.first => c.plane_hits,
        }
    }
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities::default())
    }

    pub fn with_capacities(c: &Capacities) -> Self {
        Self {
            details: CaptureRows::new(c.details),
            ships: CaptureRows::new(c.ships),
            damage: Arena::new(c.damage),
            air: Arena::new(c.air),
            hits: Arena::new(c.hits),
            groups: owned_vec(filled(c.groups, 0)),
            required_groups: 0,
            planes: Arena::new(c.planes),
            plane_air: Arena::new(c.plane_air),
            plane_hits: Arena::new(c.plane_hits),
            plane_kills: Arena::new(c.plane_kills),
            commanders: Arena::new(c.commanders),
            commander_timed_traits: Arena::new(c.commander_timed_traits),
            weapon_sources: Arena::new(c.weapon_sources),
            weapon_terms: Arena::new(c.weapon_terms),
            dense_bases: CaptureRows::new(c.dense_bases),
            dense_wings: Arena::new(c.dense_wings),
            base_handles: naval_combat_air_base_wings::UniqueHandles::try_with_capacity(
                c.dense_bases,
            )
            .map_err(crate::allocation::io)
            .unwrap_or_default(),
            side_units: Arena::new(c.side_units),
            custom_sea: Arena::new(c.custom_sea),
            damage_sources: Arena::new(c.damage_sources),
            wing_types: Arena::new(c.wing_types),
            plane_wings: Arena::new(c.plane_wings),
            summary_countries: owned_vec(filled(c.summary_countries, 0)),
            summary_countries_len: 0,
            trees: super::naval_trees::Trees::default(),
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
        self.commander_timed_traits.reset();
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
        self.summary_countries_len = 0;
        self.required_groups = 0;
        self.trees.reset();
    }
    pub fn allocated_bytes(&self) -> usize {
        self.details.allocated_bytes()
            + self
                .ships
                .rows
                .iter()
                .map(engine_native_naval::naval_member_details::DetailsRow::allocated_bytes)
                .sum::<usize>()
            + self
                .hits
                .first
                .iter()
                .chain(&self.plane_hits.first)
                .map(naval_combat_hit::Row::allocated_bytes)
                .sum::<usize>()
            + self.ships.allocated_bytes()
            + self.damage.allocated_bytes()
            + self.air.allocated_bytes()
            + self.hits.allocated_bytes()
            + self.planes.allocated_bytes()
            + self.plane_air.allocated_bytes()
            + self.plane_hits.allocated_bytes()
            + self.plane_kills.allocated_bytes()
            + self.commanders.allocated_bytes()
            + self
                .commanders
                .first
                .iter()
                .map(|row| row.allocated_bytes())
                .sum::<usize>()
            + self.commander_timed_traits.allocated_bytes()
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
            + self.groups.capacity() * size_of::<usize>()
            + self.summary_countries.capacity() * size_of::<u32>()
            + self.trees.allocated_bytes()
    }
    pub fn grow(&mut self, c: &mut Capacities) {
        c.details = c.details.max(self.details.outcome.len());
        c.summary_countries = c.summary_countries.max(self.summary_countries_len);
        c.ships = c.ships.max(self.ships.outcome.len());
        c.groups = c.groups.max(self.required_groups);
        c.damage = c.damage.max(self.damage.len);
        c.air = c.air.max(self.air.len);
        c.hits = c.hits.max(self.hits.len);
        c.weapon_sources = c.weapon_sources.max(self.weapon_sources.len);
        c.weapon_terms = c.weapon_terms.max(self.weapon_terms.len);
        self.details.grow(&mut c.details);
        self.ships.grow(&mut c.ships);
        if self.damage.first.len() < c.damage
            && resize_default(&mut self.damage.first, c.damage).is_err()
        {
            return;
        }
        if self.air.first.len() < c.air && resize_default(&mut self.air.first, c.air).is_err() {
            return;
        }
        if self.hits.first.len() < c.hits && resize_default(&mut self.hits.first, c.hits).is_err() {
            return;
        }
        grow_capacity(&mut c.groups, self.required_groups);
        if self.groups.len() < c.groups && resize_default(&mut self.groups, c.groups).is_err() {
            return;
        }
        for (capacity, observed) in [
            (&mut c.planes, self.planes.len),
            (&mut c.plane_air, self.plane_air.len),
            (&mut c.plane_hits, self.plane_hits.len),
            (&mut c.plane_kills, self.plane_kills.len),
            (&mut c.commanders, self.commanders.len),
            (&mut c.dense_bases, self.dense_bases.outcome.len()),
            (&mut c.dense_wings, self.dense_wings.len),
            (&mut c.side_units, self.side_units.len),
            (&mut c.custom_sea, self.custom_sea.len),
            (&mut c.damage_sources, self.damage_sources.len),
            (&mut c.wing_types, self.wing_types.len),
            (&mut c.plane_wings, self.plane_wings.len),
        ] {
            *capacity = (*capacity).max(observed);
        }
        self.planes.grow(&mut c.planes);
        self.plane_air.grow(&mut c.plane_air);
        self.plane_hits.grow(&mut c.plane_hits);
        self.plane_kills.grow(&mut c.plane_kills);
        self.commanders.grow(&mut c.commanders);
        c.commander_timed_traits = c
            .commander_timed_traits
            .max(self.commander_timed_traits.len);
        self.commander_timed_traits
            .grow(&mut c.commander_timed_traits);
        if self.weapon_sources.first.len() < c.weapon_sources
            && resize_default(&mut self.weapon_sources.first, c.weapon_sources).is_err()
        {
            return;
        }
        if self.weapon_terms.first.len() < c.weapon_terms
            && resize_default(&mut self.weapon_terms.first, c.weapon_terms).is_err()
        {
            return;
        }
        self.dense_bases.grow(&mut c.dense_bases);
        self.dense_wings.grow(&mut c.dense_wings);
        if self.base_handles.len() >= c.dense_bases.saturating_sub(c.dense_bases / 4) {
            grow_capacity(
                &mut c.dense_bases,
                self.base_handles.len().saturating_add(1),
            );
        }
        if self.base_handles.capacity() < c.dense_bases
            && self
                .base_handles
                .try_reserve(c.dense_bases)
                .map_err(crate::allocation::io)
                .is_err()
        {
            return;
        }
        self.side_units.grow(&mut c.side_units);
        self.custom_sea.grow(&mut c.custom_sea);
        self.damage_sources.grow(&mut c.damage_sources);
        self.wing_types.grow(&mut c.wing_types);
        self.plane_wings.grow(&mut c.plane_wings);
    }

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
                        self.base_handles
                            .try_insert(plane.air_base)
                            .map_err(crate::allocation::io)
                            .map_err(|_| "collector allocation failed")?;
                    }
                }
            }
        }
        for ship in ships {
            let handle = ship.weapon_context.air_base;
            if ship.live_ship != 0 && handle != 0 {
                self.base_handles
                    .try_insert(handle)
                    .map_err(crate::allocation::io)
                    .map_err(|_| "collector allocation failed")?;
            }
        }
        Ok(())
    }
    pub fn capture(&mut self, memory: &impl Memory, base: usize, state: usize) -> [u64; N] {
        let mut read_phase = crate::read_accounting::table(TABLES[0]);
        let capture_started = qpc();
        let mut buffers = engine_native_naval::naval_member_details::DetailBuffers {
            damage: &mut self.damage.first,
            air_hits: &mut self.air.first,
            naval_hits: &mut self.hits.first,
            lengths: [0; 3],
            required: [0; 3],
            planes: engine_native_memory::walk::Buffer::Growing(&mut self.planes.first),
            planes_len: 0,
            planes_required: 0,
            plane_air_hits: engine_native_memory::walk::Buffer::Growing(&mut self.plane_air.first),
            plane_naval_hits: engine_native_memory::walk::Buffer::Growing(&mut self.plane_hits.first),
            plane_hit_lengths: [0; 2],
            plane_hit_required: [0; 2],
            plane_kills: engine_native_memory::walk::Buffer::Growing(&mut self.plane_kills.first),
            plane_kills_len: 0,
            plane_kills_required: 0,
            commander_inputs: engine_native_memory::walk::Buffer::Growing(&mut self.commanders.first),
            commander_inputs_len: 0,
            commander_inputs_required: 0,
            commander_timed_traits: &mut self.commander_timed_traits.first,
            commander_timed_traits_len: 0,
            side_units: engine_native_memory::walk::Buffer::Growing(&mut self.side_units.first),
            side_units_len: 0,
            side_units_required: 0,
            summary_countries: &mut self.summary_countries,
            summary_countries_len: 0,
            custom_sea: engine_native_memory::walk::Buffer::Growing(&mut self.custom_sea.first),
            custom_sea_len: 0,
            custom_sea_required: 0,
            damage_sources: engine_native_memory::walk::Buffer::Growing(&mut self.damage_sources.first),
            damage_sources_len: 0,
            damage_sources_required: 0,
            wing_types: engine_native_memory::walk::Buffer::Growing(&mut self.wing_types.first),
            wing_types_len: 0,
            wing_types_required: 0,
            plane_wings: engine_native_memory::walk::Buffer::Growing(&mut self.plane_wings.first),
            plane_wings_len: 0,
            plane_wings_required: 0,
            weapon_sources: &mut self.weapon_sources.first,
            weapon_terms: &mut self.weapon_terms.first,
            weapon_lengths: [0; 2],
            weapon_required: [0; 2],
            doctrine_keys: Default::default(),
            term_metadata: term_metadata(),
            source_memo: source_memo(),
        };
        let first = engine_native_naval::naval_combat_details::walk_details_timed(
            memory,
            base,
            state,
            engine_native_memory::walk::Buffer::Growing(&mut self.details.rows),
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
        self.commander_timed_traits.len = buffers.commander_timed_traits_len;
        self.commander_timed_traits.required = buffers.commander_timed_traits_len;
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
        let mut ticks = [first.read_ticks[0], first.read_ticks[1], 0, 0, 0];
        ticks[2] = qpc().saturating_sub(dense_started).max(0) as u64;
        let trees_started = qpc();
        read_phase.select(TABLES[3]);
        let combats = matches!(self.details.outcome, Outcome::Rows { .. })
            .then(|| self.details.rows.get(..self.details.outcome.len()))
            .flatten();
        let members = matches!(self.ships.outcome, Outcome::Rows { .. })
            .then(|| self.ships.rows.get(..self.ships.outcome.len()))
            .flatten();
        self.trees.capture(
            memory,
            base,
            state,
            combats,
            members,
            &self.planes.first[..self.planes.len],
            &self.weapon_sources.first[..self.weapon_sources.len],
        );
        ticks[3] = qpc().saturating_sub(trees_started).max(0) as u64;
        let total = qpc().saturating_sub(capture_started).max(0) as u64;
        ticks[0] = ticks[0].saturating_add(total.saturating_sub(ticks.iter().sum::<u64>()));
        ticks
    }
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.details.outcome,
            self.ships.outcome,
            self.dense_bases.outcome,
            self.trees.outcome(),
            self.trees.samples_outcome(),
        ]
    }
    pub fn tables(&self) -> [TableView; N] {
        let out = self.outcomes();
        let [trees, samples] = self.trees.tables();
        [
            table::<tables::NavalCombatDetails, _>(out[0], &self.details.rows, |r| {
                rows::NavalCombatDetailsRow {
                    combat: r.combat,
                    province: r.province,
                    day: r.day,
                    duration: r.duration,
                    progress: r.progress,
                    custom_sea: r.custom_sea.map(|span| {
                        let source = range(&self.custom_sea.first, span);
                        let mut bytes = owned_vec(filled(source.len().saturating_mul(2), 0));
                        for (out, value) in bytes.chunks_exact_mut(2).zip(source) {
                            out.copy_from_slice(&value.to_le_bytes());
                        }
                        bytes
                    }),
                    custom_sea_error: u8::from(r.custom_sea_error.is_some()),
                    sides: r.sides.each_ref().map(|s| rows::NavalCombatSideDetails {
                        damage_history: s.damage_history,
                        original_health: s.original_health,
                        total_damage: s.total_damage,
                        damage_by_type: owned_vec(collect_vec(
                            range(&self.damage_sources.first, s.damage_by_type)
                                .iter()
                                .map(|&(token, damage)| rows::NavalDamageSource { token, damage }),
                        )),
                        commander: s.commander,
                        commander_inputs: range(&self.commanders.first, s.commander_inputs)
                            .first()
                            .map(|commander| {
                                commander_inputs(commander, &self.commander_timed_traits.first)
                            }),
                        units: owned_vec(copy_vec(range(&self.side_units.first, s.units))),
                        engaged: s.engaged,
                        retreating: s.retreating,
                        capital_screening: s.capital_screening,
                        carrier_screening: s.carrier_screening,
                        positioning: s.positioning,
                        anti_air: s.anti_air,
                        summary: rows::NavalCombatSummaryInputs {
                            recent_join_penalty: s.summary.recent_join_penalty,
                            escort_bonus: s.summary.escort_bonus,
                            countries: owned_vec(copy_vec(range(
                                &self.summary_countries,
                                s.summary.countries,
                            ))),
                            secondary_countries: owned_vec(copy_vec(range(
                                &self.summary_countries,
                                s.summary.secondary_countries,
                            ))),
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
                        planes: owned_vec(collect_vec(
                            range(&self.planes.first, s.planes).iter().map(|p| {
                                rows::NavalCombatPlane {
                                    wing_types: owned_vec(copy_vec(range(
                                        &self.wing_types.first,
                                        p.wing_types,
                                    ))),
                                    source: p.source,
                                    group_index: p.group_index,
                                    index: p.index,
                                    carrier_slot: p.carrier_slot,
                                    carrier_member_present: p.carrier_member_present,
                                    carrier_state: p.carrier_state,
                                    air_base: p.air_base,
                                    casualties: p.casualties,
                                    time_duration: p.time_duration,
                                    wings: p.wings.map(|span| {
                                        owned_vec(copy_vec(range(&self.plane_wings.first, span)))
                                    }),
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
                                    kills: owned_vec(collect_vec(
                                        range(&self.plane_kills.first, p.kills).iter().map(|k| {
                                            rows::NavalCombatAirWingKill {
                                                wing: k.wing,
                                                index: k.index,
                                                category: k.category,
                                                count: k.count,
                                            }
                                        }),
                                    )),
                                    air_hits: p.air_hits.map(|span| {
                                        owned_vec(collect_vec(
                                            range(&self.plane_air.first, span).iter().map(air_hit),
                                        ))
                                    }),
                                    naval_hits: owned_vec(collect_vec(
                                        range(&self.plane_hits.first, p.naval_hits)
                                            .iter()
                                            .map(naval_hit),
                                    )),
                                }
                            }),
                        )),
                    }),
                }
            }),
            table::<tables::NavalCombatShips, _>(out[1], &self.ships.rows, |r| self.ship(r)),
            table::<tables::NavalCombatAirBaseWingHandles, _>(
                out[2],
                &self.dense_bases.rows,
                |r| rows::NavalCombatAirBaseWingHandlesRow {
                    base: r.base,
                    wings: owned_vec(copy_vec(range(
                        &self.dense_wings.first,
                        [r.wings_start, r.wings_len],
                    ))),
                },
            ),
            trees,
            samples,
        ]
    }
    fn ship(&self, r: &engine_native_naval::naval_member_details::DetailsRow) -> rows::NavalCombatShipRow {
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
            name: owned_vec(copy_vec(r.name.as_bytes())),
            type_name: owned_vec(copy_vec(r.type_name.as_bytes())),
            sprite: owned_vec(copy_vec(r.sprite.as_bytes())),
            sunk_by: owned_vec(copy_vec(r.sunk_by.as_bytes())),
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
                terrain_name: owned_vec(copy_vec(r.weapon_context.terrain_name.as_bytes())),
                sources: owned_vec(collect_vec(
                    range(&self.weapon_sources.first, r.weapon_context.sources)
                        .iter()
                        .map(|s| rows::NavalShipModifierSource {
                            scope: s.scope,
                            index: s.index,
                            selector: s.selector,
                            reference: s.reference,
                            dynamic_keys: s.dynamic_keys,
                            weights: s.weights,
                            movement: s.movement,
                            terms: owned_vec(collect_vec(
                                range(&self.weapon_terms.first, s.terms).iter().map(|t| {
                                    rows::NavalShipModifierTerm {
                                        key: t.key,
                                        raw: t.raw,
                                        metadata_flags: t.metadata_flags,
                                        feature_selector: t.feature_selector,
                                    }
                                }),
                            )),
                        }),
                )),
            },
            damage_entries: owned_vec(collect_vec(
                range(&self.damage.first, r.damage_entries).iter().map(|v| {
                    rows::NavalCombatDamage {
                        source: v.source,
                        country: v.country,
                        damage: v.damage,
                    }
                }),
            )),
            air_hits: r.air_hits.map(|span| {
                owned_vec(collect_vec(
                    range(&self.air.first, span).iter().map(air_hit),
                ))
            }),
            naval_hits: owned_vec(collect_vec(
                range(&self.hits.first, r.naval_hits).iter().map(naval_hit),
            )),
        }
    }
}
pub fn commander_inputs(
    c: &engine_native_naval::naval_commander_inputs::Row,
    timed: &[engine_native::cunit_leader::TimedTrait],
) -> rows::NavalCommanderInputs {
    rows::NavalCommanderInputs {
        commander: c.commander,
        country: c.country,
        category: c.category,
        experience: c.experience,
        skills: c.skills,
        skill_definitions: c.skill_definitions.map(commander_skill),
        next_skill: commander_skill(c.next_skill),
        name: owned_vec(copy_vec(&c.name[..c.name_len])),
        description: owned_vec(copy_vec(&c.description[..c.description_len])),
        presentation: super::combat_details::commander_presentation(&c.presentation),
        traits: owned_vec(collect_vec(c.traits[..c.traits_len].iter().map(|t| {
            rows::LandCombatCommanderTrait {
                token_id: t.token_id,
                definition_ordinal: t.definition_ordinal,
                kind: t.kind,
                relevant: t.relevant,
                token: owned_vec(copy_vec(
                    &c.trait_text[t.token_start..t.token_start + t.token_len],
                )),
            }
        }))),
        timed_traits: rows::Recorded::Value(owned_vec(collect_vec(
            range(timed, c.timed_traits)
                .iter()
                .map(|t| rows::LandCombatCommanderTimedTrait {
                    token_id: t.token_id,
                    definition_ordinal: t.definition_ordinal,
                    remaining_days: t.remaining_days,
                }),
        ))),
        learning_traits: owned_vec(collect_vec(
            c.learning_traits[..c.learning_traits_len].iter().map(|t| {
                rows::LandCombatCommanderLearningTrait {
                    token_id: t.token_id,
                    definition_ordinal: t.definition_ordinal,
                    required_experience: t.required_experience,
                    experience: t.experience,
                    token: owned_vec(copy_vec(
                        &c.learning_text[t.token_start..t.token_start + t.token_len],
                    )),
                }
            }),
        )),
        learning_threshold: c.learning_threshold,
        modifier_feature_flags: c.modifier_feature_flags,
        modifier_callback_kind: c.modifier_callback_kind,
        modifiers: owned_vec(collect_vec(c.modifiers[..c.modifiers_len].iter().map(
            |m| rows::LandCombatCommanderPortraitModifier {
                source: m.source,
                id: m.id,
                value: m.value,
                metadata_flags: m.metadata_flags,
                category_mask: m.category_mask,
                feature_selector: m.feature_selector,
            },
        ))),
        tooltips: owned_vec(collect_vec(c.tooltips[..c.tooltips_len].iter().map(|t| {
            rows::LandCombatCommanderPortraitTooltip {
                source: t.source,
                bucket: t.bucket,
                key: t.key,
                text: owned_vec(copy_vec(
                    &c.tooltip_text[t.text_start..t.text_start + t.text_len],
                )),
            }
        }))),
    }
}
pub fn commander_skill(
    s: engine_native_naval::naval_commander_inputs::SkillDefinition,
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
pub fn air_hit(v: &naval_combat_air_hit::Row) -> rows::NavalCombatAirHit {
    rows::NavalCombatAirHit {
        variant: v.variant,
        count: v.count,
        country: v.country,
    }
}
pub fn naval_hit(v: &naval_combat_hit::Row) -> rows::NavalCombatHit {
    rows::NavalCombatHit {
        target_slot: v.target_slot,
        target_name: owned_vec(copy_vec(v.target_name.as_bytes())),
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
    fn dense_base_selection_uses_only_active_forward_planes_and_live_carrier_ships() {
        let mut c = Captured::with_capacities(&Capacities {
            details: 1,
            ships: 2,
            planes: 3,
            dense_bases: 2,
            dense_wings: 2,
            ..Capacities::default()
        });
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

    #[test]
    fn dense_base_selection_keeps_first_reference_order_without_allocation() {
        let countries = 12;
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
        c.select_dense_base_handles().unwrap();
        assert_eq!(c.base_handles.len(), countries);
        assert!(
            c.base_handles
                .as_slice()
                .iter()
                .enumerate()
                .all(|(country, &handle)| handle == base(country))
        );
        assert_eq!(c.allocated_bytes(), reserved);
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
        captured.ships.rows.resize(19, Default::default());
        captured.ships.outcome = Outcome::Rows { len: 19 };
        captured.groups.resize(17, 0);
        captured.required_groups = 17;
        captured.damage.first.resize(13, Default::default());
        captured.damage.len = 13;
        captured.air.first.resize(9, Default::default());
        captured.air.len = 9;
        captured.hits.first.resize(11, Default::default());
        captured.hits.len = 11;
        captured.weapon_sources.first.resize(7, Default::default());
        captured.weapon_sources.len = 7;
        captured.weapon_terms.first.resize(21, Default::default());
        captured.weapon_terms.len = 21;
        captured
            .commander_timed_traits
            .first
            .resize(5000, Default::default());
        captured.commander_timed_traits.len = 5000;
        assert!(captured.allocated_bytes() > before);
        captured.plane_air.first[0].variant = 111;
        captured.plane_air.required = 9;
        captured.plane_hits.required = 5;
        captured.plane_kills.required = 7;
        captured.commanders.required = 3;
        captured.side_units.required = 4;
        captured.grow(&mut capacity);
        for (prepared, observed) in [
            capacity.ships,
            capacity.groups,
            capacity.damage,
            capacity.air,
            capacity.hits,
            capacity.weapon_sources,
            capacity.weapon_terms,
        ]
        .into_iter()
        .zip([19, 17, 13, 9, 11, 7, 21])
        {
            assert!(prepared >= observed && prepared <= observed * 2);
        }
        assert_eq!(Captured::vacant().allocated_bytes(), 0);
        assert!(capacity.plane_air >= 9 && capacity.plane_hits >= 5);
        assert!(capacity.plane_kills >= 7 && capacity.commanders >= 3);
        assert!(capacity.side_units >= 4 && captured.side_units.first.len() == capacity.side_units);
        assert_eq!(captured.plane_air.first.len(), capacity.plane_air);
        assert_eq!(captured.details.rows[0].combat, 42);
        assert_eq!(captured.ships.rows[0].member.combat, 42);
        assert_eq!(captured.plane_air.first[0].variant, 111);
        assert!(capacity.commander_timed_traits >= 5000);
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
    fn commander_projection_preserves_timed_trait_order_for_live_and_results() {
        let timed = [
            engine_native::cunit_leader::TimedTrait {
                token_id: 9,
                definition_ordinal: 3,
                remaining_days: -1,
            },
            engine_native::cunit_leader::TimedTrait {
                token_id: 9,
                definition_ordinal: 3,
                remaining_days: 0,
            },
            engine_native::cunit_leader::TimedTrait {
                token_id: 7,
                definition_ordinal: 8,
                remaining_days: i32::MAX,
            },
        ];
        let commander = engine_native_naval::naval_commander_inputs::Row {
            commander: (9 << 32) | 4713,
            timed_traits: [1, 2],
            ..Default::default()
        };
        let row = commander_inputs(&commander, &timed);
        assert_eq!(
            row.timed_traits
                .require("NavalCommanderInputs.timed_traits")
                .unwrap()
                .iter()
                .map(|value| (
                    value.token_id,
                    value.definition_ordinal,
                    value.remaining_days
                ))
                .collect::<Vec<_>>(),
            [(9, 3, 0), (7, 8, i32::MAX)]
        );
        let row = commander_inputs(
            &engine_native_naval::naval_commander_inputs::Row {
                commander: commander.commander,
                ..Default::default()
            },
            &[],
        );
        assert!(
            row.timed_traits
                .require("NavalCommanderInputs.timed_traits")
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn plane_and_ship_hit_spans_project_from_independent_arenas() {
        const WINGS: usize = 257;
        let mut c = Captured::with_capacities(&Capacities {
            details: 1,
            ships: 1,
            planes: 1,
            plane_air: 1,
            plane_hits: 1,
            air: 1,
            hits: 1,
            weapon_sources: 1,
            weapon_terms: 1,
            custom_sea: 3,
            damage_sources: 1,
            wing_types: 2,
            plane_wings: WINGS,
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
        let [
            TableView::Complete(rows::TableRows::NavalCombatDetails(details)),
            TableView::Complete(rows::TableRows::NavalCombatShips(ships)),
            _,
            _,
            _,
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
    }
}

#[cfg(test)]
#[path = "naval_details_scale_tests.rs"]
mod scale_tests;

#[cfg(any(test, feature = "test-support"))]
#[path = "naval_details_fixtures.rs"]
pub mod fixtures;
