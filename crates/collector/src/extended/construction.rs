//! Construction and occupation acquisition; building growth runs on the reserved
//! acquisition worker, with other growth and conversion off the game thread.
use crate::{
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory,
    allocation::{copy_vec, map_slice, owned_vec, reserve, reserve_exact},
    carmy_manpower_values, cbuilding, cbuilding_production_line, cbuilding_status, ccountry,
    ccountry_collaboration_data, ccountry_collaboration_status, ccountry_occupation_data,
    ccountry_occupation_status, cgame_state, cgeneral_production_line, coccupation_law,
    cpdx_array::Array,
    cproduction_line, cproduction_status, cprovince, crailway_production_line,
    creferenced_division_template, cresistance, cstate, cstate_garrison_data, cstate_template,
    manpower_value_entry, msvc_string, pdx_robin_hood_bucket, state_resource_entry,
    walk::Outcome,
};
use replay::{
    frame::TableView,
    rows::{self, TableId, tables},
};

pub const N: usize = 12;
pub const TABLES: [TableId; N] = [
    TableId::StateBuildings,
    TableId::ProvinceBuildings,
    TableId::StateBaseResources,
    TableId::BuildingConstruction,
    TableId::RailwayConstruction,
    TableId::OccupationSettings,
    TableId::StateOccupationLaws,
    TableId::StateGarrisons,
    TableId::StateAssignedManpower,
    TableId::CountryOccupationReserveManpower,
    TableId::StateResistance,
    TableId::CountryCollaboration,
];

/// Name, local cost anchor, and local member indices. Reporting sums all member
/// ticks; physically shared walks assign their full read/check cost to the anchor.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("Buildings", 0, &[0, 1]),
    ("ConstructionQueue", 3, &[3, 4]),
    ("Occupation", 5, &[5, 6, 7, 8, 9]),
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: [usize; N],
    routes: usize,
    law_bytes: usize,
    occupied: usize,
    group_laws: usize,
    equipment: usize,
}

impl Capacities {
    #[cfg(test)]
    pub(super) fn for_test_buildings(rows: usize) -> Self {
        let mut capacity = Self::default();
        capacity.rows[..2].fill(rows);
        capacity
    }

    pub(crate) fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let rows = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("construction sizing row count overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        for (index, outcome) in [
            (2, state_resource_entry::walk(memory, base, state, &mut [])),
            (10, cresistance::walk(memory, base, state, &mut [])),
        ] {
            crate::table_capture::grow_capacity(&mut self.rows[index], rows(outcome)?);
        }
        let queue = cbuilding_production_line::walk(
            memory,
            base,
            state,
            cbuilding_production_line::Buffers {
                buildings: &mut [],
                railways: &mut [],
                route_provinces: &mut [],
            },
        );
        crate::table_capture::grow_capacity(&mut self.rows[3], rows(queue.buildings)?);
        crate::table_capture::grow_capacity(&mut self.rows[4], rows(queue.railways)?);
        crate::table_capture::grow_capacity(&mut self.routes, queue.required_route_len);
        Ok(())
    }

    pub(crate) fn raise_for_occupied(&mut self, entries: usize) {
        crate::country_scale::raise(&mut self.occupied, crate::country_scale::rows(entries, 1));
    }

    /// OccupationSettings has one row per canonical country.
    pub(crate) fn raise_for_countries(&mut self, slots: usize) {
        use crate::country_scale::{raise, rows};
        raise(&mut self.rows[5], rows(slots, 1));
    }
}

pub(crate) struct Buildings {
    state_buildings: CaptureRows<cbuilding::Row>,
    province_buildings: CaptureRows<cbuilding::Row>,
}

impl Buildings {
    pub(crate) fn vacant() -> Self {
        Self {
            state_buildings: CaptureRows::new(0),
            province_buildings: CaptureRows::new(0),
        }
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.state_buildings.allocated_bytes() + self.province_buildings.allocated_bytes()
    }

    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> u64 {
        if !replay::cadence::should_walk(TableId::StateBuildings, hour, force_full) {
            self.fail(replay::cadence::SKIPPED_REASON);
            return 0;
        }
        let _reads = read_accounting::table(TABLES[0]);
        let begin = qpc();
        let buildings = cbuilding_status::walk(
            memory,
            base,
            game_state,
            &mut self.state_buildings.rows,
            &mut self.province_buildings.rows,
        );
        self.state_buildings.outcome = buildings.states;
        self.province_buildings.outcome = buildings.provinces;
        elapsed(begin)
    }

    pub(crate) fn fail(&mut self, reason: &'static str) {
        self.state_buildings.outcome = Outcome::Unavailable { reason };
        self.province_buildings.outcome = Outcome::Unavailable { reason };
    }
}

pub struct Captured {
    buildings: Buildings,
    buildings_ticks: Option<u64>,
    base_resources: CaptureRows<state_resource_entry::Row>,
    building_construction: CaptureRows<cbuilding_production_line::Row>,
    railway_construction: CaptureRows<crailway_production_line::Row>,
    settings: CaptureRows<ccountry_occupation_status::Row>,
    laws: CaptureRows<ccountry_occupation_data::LawRow>,
    group_laws: CaptureRows<ccountry_occupation_data::GroupLawRow>,
    fallback_law: Option<(usize, usize)>,
    garrisons: CaptureRows<cstate_garrison_data::Row>,
    assigned_manpower: CaptureRows<cstate_garrison_data::ManpowerRow>,
    reserve_manpower: CaptureRows<ccountry_occupation_status::ReserveManpowerRow>,
    resistance: CaptureRows<cresistance::Row>,
    collaboration: CaptureRows<ccountry_collaboration_data::Row>,
    routes: Arena<u32>,
    law_bytes: Arena<u8>,
    occupied: Arena<u32>,
    equipment: Arena<cstate_garrison_data::equipment::Row>,
    /// Raw-copy split gate for scheduled construction-rest tables.
    /// Set when an acquisition job returns a valid raw payload; `capture`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus route/law arenas) before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) rest_raw_mode: bool,
}

impl Captured {
    pub(crate) fn observe_occupied(
        &self,
        work: &mut super::country_counters::RosterWork,
    ) -> std::io::Result<()> {
        let rows = self
            .settings
            .rows
            .get(..self.settings.outcome.len())
            .ok_or_else(|| std::io::Error::other("occupation settings row count exceeds buffer"))?;
        work.observe_occupied(rows, &self.occupied)
    }

    /// Buildings belong to Governance; staged rest rows belong to World.
    pub(crate) fn fail_gated_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        if !self.rest_raw_mode {
            let reason =
                if replay::cadence::should_walk(TableId::StateBaseResources, hour, force_full) {
                    reason
                } else {
                    replay::cadence::SKIPPED_REASON
                };
            self.fail_construction_rest_capture(reason);
        }
    }

    /// Allocation-free placeholder while a worker owns the capture buffers.
    pub(crate) fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            routes: 0,
            law_bytes: 0,
            occupied: 0,
            group_laws: 0,
            equipment: 0,
        })
    }

    pub fn with_capacities(capacity: &Capacities) -> Self {
        Self {
            buildings: Buildings {
                state_buildings: CaptureRows::new(capacity.rows[0]),
                province_buildings: CaptureRows::new(capacity.rows[1]),
            },
            buildings_ticks: None,
            base_resources: CaptureRows::new(capacity.rows[2]),
            building_construction: CaptureRows::new(capacity.rows[3]),
            railway_construction: CaptureRows::new(capacity.rows[4]),
            settings: CaptureRows::new(capacity.rows[5]),
            laws: CaptureRows::new(capacity.rows[6]),
            group_laws: CaptureRows::new(capacity.group_laws),
            fallback_law: None,
            garrisons: CaptureRows::new(capacity.rows[7]),
            assigned_manpower: CaptureRows::new(capacity.rows[8]),
            reserve_manpower: CaptureRows::new(capacity.rows[9]),
            resistance: CaptureRows::new(capacity.rows[10]),
            collaboration: CaptureRows::new(capacity.rows[11]),
            routes: Arena::new(capacity.routes),
            law_bytes: Arena::new(capacity.law_bytes),
            occupied: Arena::new(capacity.occupied),
            equipment: Arena::new(capacity.equipment),
            rest_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.buildings.state_buildings.reset();
        self.buildings.province_buildings.reset();
        self.buildings_ticks = None;
        self.base_resources.reset();
        self.building_construction.reset();
        self.railway_construction.reset();
        self.settings.reset();
        self.laws.reset();
        self.group_laws.reset();
        self.fallback_law = None;
        self.garrisons.reset();
        self.assigned_manpower.reset();
        self.reserve_manpower.reset();
        self.resistance.reset();
        self.collaboration.reset();
        self.routes.reset();
        self.law_bytes.reset();
        self.occupied.reset();
        self.equipment.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.buildings.allocated_bytes()
            + self.base_resources.allocated_bytes()
            + self.building_construction.allocated_bytes()
            + self.railway_construction.allocated_bytes()
            + self.settings.allocated_bytes()
            + self.laws.allocated_bytes()
            + self.group_laws.allocated_bytes()
            + self.garrisons.allocated_bytes()
            + self.assigned_manpower.allocated_bytes()
            + self.reserve_manpower.allocated_bytes()
            + self.resistance.allocated_bytes()
            + self.collaboration.allocated_bytes()
            + self.routes.allocated_bytes()
            + self.law_bytes.allocated_bytes()
            + self.occupied.allocated_bytes()
            + self.equipment.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.rows[0] = capacity.rows[0].max(self.buildings.state_buildings.rows.len());
        capacity.rows[1] = capacity.rows[1].max(self.buildings.province_buildings.rows.len());
        capacity.rows[5] = capacity.rows[5].max(self.settings.rows.len());
        capacity.group_laws = capacity.group_laws.max(self.group_laws.rows.len());
        capacity.law_bytes = capacity.law_bytes.max(self.law_bytes.first.len());
        self.buildings.state_buildings.grow(&mut capacity.rows[0]);
        self.buildings
            .province_buildings
            .grow(&mut capacity.rows[1]);
        self.base_resources.grow(&mut capacity.rows[2]);
        self.building_construction.grow(&mut capacity.rows[3]);
        self.railway_construction.grow(&mut capacity.rows[4]);
        self.settings.grow(&mut capacity.rows[5]);
        self.laws.grow(&mut capacity.rows[6]);
        self.group_laws.grow(&mut capacity.group_laws);
        self.garrisons.grow(&mut capacity.rows[7]);
        self.assigned_manpower.grow(&mut capacity.rows[8]);
        self.reserve_manpower.grow(&mut capacity.rows[9]);
        self.resistance.grow(&mut capacity.rows[10]);
        self.collaboration.grow(&mut capacity.rows[11]);
        self.routes.grow(&mut capacity.routes);
        self.law_bytes.grow(&mut capacity.law_bytes);
        self.occupied.grow(&mut capacity.occupied);
        capacity.equipment = capacity.equipment.max(self.equipment.first.len());
        self.equipment.grow(&mut capacity.equipment);
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.buildings.state_buildings.outcome,
            self.buildings.province_buildings.outcome,
            self.base_resources.outcome,
            self.building_construction.outcome,
            self.railway_construction.outcome,
            self.settings.outcome,
            self.laws.outcome,
            self.garrisons.outcome,
            self.assigned_manpower.outcome,
            self.reserve_manpower.outcome,
            self.resistance.outcome,
            self.collaboration.outcome,
        ]
    }

    pub(crate) fn take_buildings(&mut self) -> Buildings {
        self.buildings_ticks = None;
        std::mem::replace(&mut self.buildings, Buildings::vacant())
    }

    pub(crate) fn restore_buildings(
        &mut self,
        mut part: Buildings,
        result: Result<u64, &'static str>,
    ) {
        let ticks = match result {
            Ok(ticks) => ticks,
            Err(reason) => {
                part.fail(reason);
                0
            }
        };
        self.buildings = part;
        self.buildings_ticks = Some(ticks);
    }

    pub(crate) fn finish_buildings(&mut self, hour: i32, force_full: bool) -> u64 {
        let ticks = self.buildings_ticks.take();
        if !replay::cadence::should_walk(TableId::StateBuildings, hour, force_full) {
            self.buildings.fail(replay::cadence::SKIPPED_REASON);
            return 0;
        }
        match ticks {
            Some(ticks) => ticks,
            None => {
                self.buildings.fail("building acquisition not supplied");
                0
            }
        }
    }

    /// All elapsed values are raw QPC ticks; the containing callback owns their
    /// conversion to nanoseconds and its own complete callback measurement.
    #[cfg(test)]
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_buffers(memory, base, game_state, hour, force_full, false)
    }

    pub(crate) fn capture_growing(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.capture_buffers(memory, base, game_state, hour, force_full, true)
    }

    fn capture_buffers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
        growing: bool,
    ) -> [u64; N] {
        if !self.rest_raw_mode {
            self.base_resources.reset();
            self.building_construction.reset();
            self.railway_construction.reset();
            self.settings.reset();
            self.laws.reset();
            self.group_laws.reset();
            self.fallback_law = None;
            self.garrisons.reset();
            self.assigned_manpower.reset();
            self.reserve_manpower.reset();
            self.resistance.reset();
            self.collaboration.reset();
            self.routes.reset();
            self.law_bytes.reset();
            self.equipment.reset();
        }
        let mut ticks = [0; N];
        if !replay::cadence::should_walk(TableId::StateBaseResources, hour, force_full) {
            self.fail_construction_rest_capture(replay::cadence::SKIPPED_REASON);
            self.routes.reset();
            self.law_bytes.reset();
            self.equipment.reset();
            return ticks;
        }
        if self.rest_raw_mode {
            return ticks;
        }
        ticks[2] = read_accounting::with_table(TABLES[2], || {
            single(&mut self.base_resources, |output| {
                state_resource_entry::walk_buffer(
                    memory,
                    base,
                    game_state,
                    engine::walk::Buffer::for_worker(output, growing),
                )
            })
        });
        let mut reads = read_accounting::table(TABLES[3]);
        let begin = qpc();
        let queue = cbuilding_production_line::walk_buffer(
            memory,
            base,
            game_state,
            cbuilding_production_line::BuffersBuffer {
                buildings: engine::walk::Buffer::for_worker(
                    &mut self.building_construction.rows,
                    growing,
                ),
                railways: engine::walk::Buffer::for_worker(
                    &mut self.railway_construction.rows,
                    growing,
                ),
                route_provinces: engine::walk::Buffer::for_worker(&mut self.routes.first, growing),
            },
        );
        self.building_construction.outcome = queue.buildings;
        self.railway_construction.outcome = queue.railways;
        self.routes.len = queue.route_len;
        self.routes.required = queue.required_route_len;
        ticks[3] = elapsed(begin);
        reads.select(TABLES[5]);
        let begin = qpc();
        let occupation = ccountry_occupation_status::walk_with_occupied_buffer(
            memory,
            base,
            game_state,
            ccountry_occupation_status::BuffersBuffer {
                settings: engine::walk::Buffer::for_worker(&mut self.settings.rows, growing),
                laws: engine::walk::Buffer::for_worker(&mut self.laws.rows, growing),
                group_laws: engine::walk::Buffer::for_worker(&mut self.group_laws.rows, growing),
                garrisons: engine::walk::Buffer::for_worker(&mut self.garrisons.rows, growing),
                manpower: engine::walk::Buffer::for_worker(
                    &mut self.assigned_manpower.rows,
                    growing,
                ),
                reserve_manpower: engine::walk::Buffer::for_worker(
                    &mut self.reserve_manpower.rows,
                    growing,
                ),
                names: engine::walk::Buffer::for_worker(&mut self.law_bytes.first, growing),
                equipment: engine::walk::Buffer::for_worker(&mut self.equipment.first, growing),
            },
            engine::walk::Buffer::for_worker(&mut self.occupied.first, growing),
        );
        self.settings.outcome = occupation.settings;
        self.laws.outcome = occupation.laws;
        self.group_laws.outcome = occupation.group_laws;
        self.fallback_law = occupation.fallback_law;
        self.garrisons.outcome = occupation.garrisons;
        self.assigned_manpower.outcome = occupation.manpower;
        self.reserve_manpower.outcome = occupation.reserve_manpower;
        self.law_bytes.len = occupation.bytes_len;
        self.law_bytes.required = occupation.required_bytes;
        self.occupied.len = occupation.occupied_len;
        self.occupied.required = occupation.required_occupied;
        self.equipment.len = occupation.equipment_len;
        self.equipment.required = occupation.required_equipment;
        ticks[5] = elapsed(begin);
        reads.select(TABLES[10]);
        ticks[10] = single(&mut self.resistance, |output| {
            cresistance::walk_buffer(
                memory,
                base,
                game_state,
                engine::walk::Buffer::for_worker(output, growing),
            )
        });
        reads.select(TABLES[11]);
        ticks[11] = single(&mut self.collaboration, |output| {
            ccountry_collaboration_status::walk_buffer(
                memory,
                base,
                game_state,
                engine::walk::Buffer::for_worker(output, growing),
            )
        });
        ticks
    }

    fn garrison_equipment(
        &self,
        row: &cstate_garrison_data::Row,
    ) -> rows::Recorded<rows::GarrisonEquipmentInputs> {
        let mut inputs = rows::GarrisonEquipmentInputs::default();
        match row.equipment {
            Some((start, count)) => {
                let span = &self.equipment.first[..self.equipment.len][start..start + count];
                inputs.rows = owned_vec(map_slice(span, |value| rows::GarrisonEquipment {
                    equipment_token: value.equipment_token,
                    archetype_token: value.archetype_token,
                    archetype_active: value.archetype_active,
                    required_raw: value.required_raw,
                    required_lookup_raw: value.required_lookup_raw,
                    allocated_raw: value.allocated_raw,
                    weight_raw: value.weight_raw,
                }));
            }
            None => {
                inputs.failure = Some(owned_vec(copy_vec(
                    row.equipment_failure
                        .unwrap_or("garrison equipment capture not supplied")
                        .as_bytes(),
                )))
            }
        }
        rows::Recorded::Value(inputs)
    }

    /// Runs only on the spool thread. All twelve tables use shared sort,
    /// duplicate-key checks, field masks and native-width serialization.
    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::StateBuildings, _>(
                self.buildings.state_buildings.outcome,
                &self.buildings.state_buildings.rows,
                building,
            ),
            table::<tables::ProvinceBuildings, _>(
                self.buildings.province_buildings.outcome,
                &self.buildings.province_buildings.rows,
                building,
            ),
            table::<tables::StateBaseResources, _>(
                self.base_resources.outcome,
                &self.base_resources.rows,
                |row| rows::StateBaseResourceRow {
                    state: row.state,
                    resource: row.resource,
                    ordinal: row.ordinal,
                    amount: row.amount,
                },
            ),
            table::<tables::BuildingConstruction, _>(
                self.building_construction.outcome,
                &self.building_construction.rows,
                |row| rows::BuildingConstructionRow {
                    id: row.id,
                    country: row.country,
                    queue_ordinal: row.queue_ordinal,
                    active_factories: row.active_factories,
                    cost: row.cost,
                    speed: row.speed,
                    produced: row.produced,
                    priority: row.priority,
                    amount: row.amount,
                    created_hour: row.created_hour,
                    building_type: row.building_type,
                    target_flags: row.target_flags,
                    target_location: row.target_location,
                    target_state: row.target_state,
                    target_country: row.target_country,
                    repair_levels: row.repair_levels,
                },
            ),
            railway_table(&self.railway_construction, &self.routes),
            settings_table(
                &self.settings,
                &self.group_laws,
                self.fallback_law,
                &self.law_bytes,
                &self.occupied,
            ),
            law_table(&self.laws, &self.law_bytes),
            table::<tables::StateGarrisons, _>(
                self.garrisons.outcome,
                &self.garrisons.rows,
                |row| rows::StateGarrisonRow {
                    controller: row.controller,
                    occupied: row.occupied,
                    state: row.state,
                    required_manpower: row.required_manpower,
                    strength_ratio: row.strength_ratio,
                    garrison_required: row.garrison_required,
                    serialized_flag: row.serialized_flag,
                    equipment: self.garrison_equipment(row),
                },
            ),
            table::<tables::StateAssignedManpower, _>(
                self.assigned_manpower.outcome,
                &self.assigned_manpower.rows,
                |row| rows::StateAssignedManpowerRow {
                    controller: row.controller,
                    occupied: row.occupied,
                    state: row.state,
                    ordinal: row.ordinal,
                    source_country: row.source_country,
                    amount: row.amount,
                },
            ),
            table::<tables::CountryOccupationReserveManpower, _>(
                self.reserve_manpower.outcome,
                &self.reserve_manpower.rows,
                |row| rows::CountryOccupationReserveManpowerRow {
                    country: row.country,
                    ordinal: row.ordinal,
                    source_country: row.source_country,
                    amount: row.amount,
                },
            ),
            table::<tables::StateResistance, _>(
                self.resistance.outcome,
                &self.resistance.rows,
                |row| rows::StateResistanceRow {
                    state: row.state,
                    country: row.country,
                    level: row.level,
                    speed: row.speed,
                    compliance: row.compliance,
                    compliance_speed: row.compliance_speed,
                    active: row.active,
                },
            ),
            table::<tables::CountryCollaboration, _>(
                self.collaboration.outcome,
                &self.collaboration.rows,
                |row| rows::CountryCollaborationRow {
                    source: row.source,
                    target: row.target,
                    progress: row.progress,
                },
            ),
        ]
    }
}

fn elapsed(begin: i64) -> u64 {
    (qpc() - begin).max(0) as u64
}

mod owned;
#[cfg(test)]
pub(crate) use owned::copy_construction_rest_refresh;

pub(crate) use owned::{
    ConstructionRestCopyContext, ConstructionRestCopyReport, ConstructionRestJob,
    copy_construction_rest_refresh_growing,
};
#[cfg(test)]
use owned::{M_C_OCC, copy_construction_string};

fn single<R>(rows: &mut CaptureRows<R>, mut walk: impl FnMut(&mut Vec<R>) -> Outcome) -> u64 {
    let begin = qpc();
    rows.outcome = walk(&mut rows.rows);
    elapsed(begin)
}

fn range<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
    arena.get(start..start.checked_add(len)?)
}

fn source<R>(rows: &CaptureRows<R>) -> Result<(Outcome, &[R]), TableView> {
    let outcome = rows.outcome;
    match outcome {
        Outcome::Unavailable { reason } => Err(unavailable(reason)),
        _ => rows
            .rows
            .get(..outcome.len())
            .map(|rows| (outcome, rows))
            .ok_or_else(|| unavailable("walker row count exceeds allocated buffer")),
    }
}

fn building(row: &cbuilding::Row) -> rows::BuildingRow {
    rows::BuildingRow {
        location: row.location,
        building_type: row.building_type,
        ordinal: row.ordinal,
        status_flags: row.status_flags,
        country: row.country,
        level: row.level,
        healthy_level: row.healthy_level,
        partial_health: row.partial_health,
        active: row.active,
    }
}

fn railway_table(
    captured: &CaptureRows<crailway_production_line::Row>,
    arena: &Arena<u32>,
) -> TableView {
    let (outcome, source) = match source(captured) {
        Ok(source) => source,
        Err(failure) => return failure,
    };
    let Some(routes) = arena.first.get(..arena.len) else {
        return unavailable("railway route arena length is invalid");
    };
    if source
        .iter()
        .any(|row| range(routes, row.route_start, row.route_len).is_none())
    {
        return unavailable("railway route lies outside the captured arena");
    }
    table::<tables::RailwayConstruction, _>(outcome, source, |row| rows::RailwayConstructionRow {
        id: row.id,
        country: row.country,
        queue_ordinal: row.queue_ordinal,
        active_factories: row.active_factories,
        cost: row.cost,
        speed: row.speed,
        produced: row.produced,
        priority: row.priority,
        amount: row.amount,
        created_hour: row.created_hour,
        base_level: row.base_level,
        target_level: row.target_level,
        // The native byte's verified save name is cancel.
        cancelled: row.completed,
        current_segment: row.current_segment,
        remaining_cost: row.remaining_cost,
        route: owned_vec(copy_vec(
            &routes[row.route_start..row.route_start + row.route_len],
        )),
    })
}

fn settings_table(
    captured: &CaptureRows<ccountry_occupation_status::Row>,
    groups: &CaptureRows<ccountry_occupation_data::GroupLawRow>,
    fallback: Option<(usize, usize)>,
    arena: &Arena<u8>,
    occupied: &Arena<u32>,
) -> TableView {
    let (outcome, source) = match source(captured) {
        Ok(source) => source,
        Err(failure) => return failure,
    };
    let Some(names) = arena.first.get(..arena.len) else {
        return unavailable("occupation law arena length is invalid");
    };
    if source
        .iter()
        .any(|row| range(names, row.law_start, row.law_len).is_none())
    {
        return unavailable("occupation default law lies outside the captured arena");
    }
    let groups = source_groups(groups, names).ok();
    let fallback = fallback.and_then(|(start, len)| range(names, start, len));
    table::<tables::OccupationSettings, _>(outcome, source, |row| rows::OccupationSettingsRow {
        country: row.country,
        garrison_template: row.garrison_template,
        default_law_present: row.default_law_present,
        law: owned_vec(copy_vec(&names[row.law_start..row.law_start + row.law_len])),
        occupied: rows::Recorded::Value(row.occupied.and_then(|span| {
            span.start
                .checked_add(span.count)
                .filter(|&end| end <= occupied.len)
                .and_then(|end| occupied.first.get(span.start..end))
                .map(|values| owned_vec(copy_vec(values)))
        })),
        group_laws: rows::Recorded::Value(groups.map(|groups| {
            let first = groups.partition_point(|group| group.controller < row.country);
            let count = groups[first..].partition_point(|group| group.controller == row.country);
            owned_vec(map_slice(&groups[first..first + count], |group| {
                rows::OccupationGroupLaw {
                    occupied: group.occupied,
                    scope_country: group.scope_country,
                    law: (group.present != 0).then(|| {
                        owned_vec(copy_vec(
                            &names[group.name_start..group.name_start + group.name_len],
                        ))
                    }),
                }
            }))
        })),
        fallback_law: rows::Recorded::Value(fallback.map(|name| owned_vec(copy_vec(name)))),
    })
}

fn source_groups<'a>(
    groups: &'a CaptureRows<ccountry_occupation_data::GroupLawRow>,
    names: &[u8],
) -> Result<&'a [ccountry_occupation_data::GroupLawRow], &'static str> {
    let Outcome::Rows { len } = groups.outcome else {
        return Err("occupied-country law inventory incomplete");
    };
    let groups = groups
        .rows
        .get(..len)
        .ok_or("occupied-country law inventory length invalid")?;
    if groups
        .iter()
        .any(|row| range(names, row.name_start, row.name_len).is_none())
    {
        return Err("occupied-country law lies outside the captured arena");
    }
    Ok(groups)
}

fn law_table(
    captured: &CaptureRows<ccountry_occupation_data::LawRow>,
    arena: &Arena<u8>,
) -> TableView {
    let (outcome, source) = match source(captured) {
        Ok(source) => source,
        Err(failure) => return failure,
    };
    let Some(names) = arena.first.get(..arena.len) else {
        return unavailable("occupation law arena length is invalid");
    };
    if source
        .iter()
        .any(|row| range(names, row.name_start, row.name_len).is_none())
    {
        return unavailable("occupation state law lies outside the captured arena");
    }
    table::<tables::StateOccupationLaws, _>(outcome, source, |row| rows::StateOccupationLawRow {
        controller: row.controller,
        occupied: row.occupied,
        state: row.state,
        name: owned_vec(copy_vec(
            &names[row.name_start..row.name_start + row.name_len],
        )),
    })
}

/// Shared synthetic address space for the scheduled construction-rest
/// copy/decode split: one state with resources and resistance, one
/// country with a building+railway queue, occupation settings with laws,
/// garrisons, manpower and reserve, plus one collaboration relation.
#[cfg(test)]
pub(crate) mod construction_rest_test_kit;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod buildings_tests;
