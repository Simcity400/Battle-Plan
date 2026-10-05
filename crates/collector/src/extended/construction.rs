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
    allocation::{copy_vec, owned_vec, reserve, reserve_exact, vec},
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    rows: [usize; N],
    routes: usize,
    law_bytes: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                4096, 4096, 4096, 1024, 256, 512, 1024, 1024, 2048, 1024, 2048, 1024,
            ],
            routes: 4096,
            law_bytes: 16384,
        }
    }
}
impl Capacities {
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
    garrisons: CaptureRows<cstate_garrison_data::Row>,
    assigned_manpower: CaptureRows<cstate_garrison_data::ManpowerRow>,
    reserve_manpower: CaptureRows<ccountry_occupation_status::ReserveManpowerRow>,
    resistance: CaptureRows<cresistance::Row>,
    collaboration: CaptureRows<ccountry_collaboration_data::Row>,
    routes: Arena<u32>,
    law_bytes: Arena<u8>,
    /// Raw-copy split gate for scheduled construction-rest tables.
    /// Set when an acquisition job returns a valid raw payload; `capture`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus route/law arenas) before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) rest_raw_mode: bool,
}

impl Captured {
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
            garrisons: CaptureRows::new(capacity.rows[7]),
            assigned_manpower: CaptureRows::new(capacity.rows[8]),
            reserve_manpower: CaptureRows::new(capacity.rows[9]),
            resistance: CaptureRows::new(capacity.rows[10]),
            collaboration: CaptureRows::new(capacity.rows[11]),
            routes: Arena::new(capacity.routes),
            law_bytes: Arena::new(capacity.law_bytes),
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
        self.garrisons.reset();
        self.assigned_manpower.reset();
        self.reserve_manpower.reset();
        self.resistance.reset();
        self.collaboration.reset();
        self.routes.reset();
        self.law_bytes.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.buildings.allocated_bytes()
            + self.base_resources.allocated_bytes()
            + self.building_construction.allocated_bytes()
            + self.railway_construction.allocated_bytes()
            + self.settings.allocated_bytes()
            + self.laws.allocated_bytes()
            + self.garrisons.allocated_bytes()
            + self.assigned_manpower.allocated_bytes()
            + self.reserve_manpower.allocated_bytes()
            + self.resistance.allocated_bytes()
            + self.collaboration.allocated_bytes()
            + self.routes.allocated_bytes()
            + self.law_bytes.allocated_bytes()
    }

    pub fn grow(&mut self, capacity: &mut Capacities) {
        capacity.rows[0] = capacity.rows[0].max(self.buildings.state_buildings.rows.len());
        capacity.rows[1] = capacity.rows[1].max(self.buildings.province_buildings.rows.len());
        capacity.rows[5] = capacity.rows[5].max(self.settings.rows.len());
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
        self.garrisons.grow(&mut capacity.rows[7]);
        self.assigned_manpower.grow(&mut capacity.rows[8]);
        self.reserve_manpower.grow(&mut capacity.rows[9]);
        self.resistance.grow(&mut capacity.rows[10]);
        self.collaboration.grow(&mut capacity.rows[11]);
        self.routes.grow(&mut capacity.routes);
        self.law_bytes.grow(&mut capacity.law_bytes);
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
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        if !self.rest_raw_mode {
            self.base_resources.reset();
            self.building_construction.reset();
            self.railway_construction.reset();
            self.settings.reset();
            self.laws.reset();
            self.garrisons.reset();
            self.assigned_manpower.reset();
            self.reserve_manpower.reset();
            self.resistance.reset();
            self.collaboration.reset();
            self.routes.reset();
            self.law_bytes.reset();
        }
        let mut ticks = [0; N];
        if !replay::cadence::should_walk(TableId::StateBaseResources, hour, force_full) {
            self.fail_construction_rest_capture(replay::cadence::SKIPPED_REASON);
            self.routes.reset();
            self.law_bytes.reset();
            return ticks;
        }
        if self.rest_raw_mode {
            return ticks;
        }
        ticks[2] = read_accounting::with_table(TABLES[2], || {
            single(&mut self.base_resources, |output| {
                state_resource_entry::walk(memory, base, game_state, output)
            })
        });
        let mut reads = read_accounting::table(TABLES[3]);
        let begin = qpc();
        let queue = cbuilding_production_line::walk(
            memory,
            base,
            game_state,
            cbuilding_production_line::Buffers {
                buildings: &mut self.building_construction.rows,
                railways: &mut self.railway_construction.rows,
                route_provinces: &mut self.routes.first,
            },
        );
        self.building_construction.outcome = queue.buildings;
        self.railway_construction.outcome = queue.railways;
        self.routes.len = queue.route_len;
        self.routes.required = queue.required_route_len;
        ticks[3] = elapsed(begin);
        reads.select(TABLES[5]);
        let begin = qpc();
        let occupation = ccountry_occupation_status::walk(
            memory,
            base,
            game_state,
            ccountry_occupation_status::Buffers {
                settings: &mut self.settings.rows,
                laws: &mut self.laws.rows,
                garrisons: &mut self.garrisons.rows,
                manpower: &mut self.assigned_manpower.rows,
                reserve_manpower: &mut self.reserve_manpower.rows,
                names: &mut self.law_bytes.first,
            },
        );
        self.settings.outcome = occupation.settings;
        self.laws.outcome = occupation.laws;
        self.garrisons.outcome = occupation.garrisons;
        self.assigned_manpower.outcome = occupation.manpower;
        self.reserve_manpower.outcome = occupation.reserve_manpower;
        self.law_bytes.len = occupation.bytes_len;
        self.law_bytes.required = occupation.required_bytes;
        ticks[5] = elapsed(begin);
        reads.select(TABLES[10]);
        ticks[10] = single(&mut self.resistance, |output| {
            cresistance::walk(memory, base, game_state, output)
        });
        reads.select(TABLES[11]);
        ticks[11] = single(&mut self.collaboration, |output| {
            ccountry_collaboration_status::walk(memory, base, game_state, output)
        });
        ticks
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
            settings_table(&self.settings, &self.law_bytes),
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

pub(crate) use owned::{
    ConstructionRestCopyContext, ConstructionRestCopyReport, ConstructionRestJob,
    copy_construction_rest_refresh,
};
#[cfg(test)]
use owned::{M_C_OCC, copy_construction_string};

fn single<R>(rows: &mut CaptureRows<R>, mut walk: impl FnMut(&mut [R]) -> Outcome) -> u64 {
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
        .any(|row| range(names, row.law_start, row.law_len).is_none())
    {
        return unavailable("occupation default law lies outside the captured arena");
    }
    table::<tables::OccupationSettings, _>(outcome, source, |row| rows::OccupationSettingsRow {
        country: row.country,
        garrison_template: row.garrison_template,
        default_law_present: row.default_law_present,
        law: owned_vec(copy_vec(&names[row.law_start..row.law_start + row.law_len])),
    })
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
