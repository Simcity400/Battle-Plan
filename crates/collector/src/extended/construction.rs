//! Construction and occupation acquisition; heap work is confined to buffer growth
//! and replay conversion on the spool thread.
use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cbuilding, cbuilding_production_line, cbuilding_status, ccountry_collaboration_data,
    ccountry_collaboration_status, ccountry_occupation_data, ccountry_occupation_status,
    crailway_production_line, cresistance, cstate_garrison_data, state_resource_entry,
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

pub struct Captured {
    state_buildings: CaptureRows<cbuilding::Row>,
    province_buildings: CaptureRows<cbuilding::Row>,
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
}

fn group_success(successful: &[u16], start: usize, end: usize) -> u16 {
    successful[start..end].iter().copied().min().unwrap()
}

impl Captured {
    pub fn with_capacities(capacity: &Capacities, successful: &[u16]) -> Self {
        // Completed secondary outputs can be empty even in a joint walk.
        // Named rows compare their own bytes rather than shared arena offsets.
        let names = group_success(successful, 5, 7);
        Self {
            state_buildings: CaptureRows::new(capacity.rows[0], successful[0]),
            province_buildings: CaptureRows::new(capacity.rows[1], successful[1]),
            base_resources: CaptureRows::new(capacity.rows[2], successful[2]),
            building_construction: CaptureRows::new(capacity.rows[3], successful[3]),
            railway_construction: CaptureRows::new(capacity.rows[4], successful[4]),
            settings: CaptureRows::new(capacity.rows[5], successful[5]),
            laws: CaptureRows::new(capacity.rows[6], successful[6]),
            garrisons: CaptureRows::new(capacity.rows[7], successful[7]),
            assigned_manpower: CaptureRows::new(capacity.rows[8], successful[8]),
            reserve_manpower: CaptureRows::new(capacity.rows[9], successful[9]),
            resistance: CaptureRows::new(capacity.rows[10], successful[10]),
            collaboration: CaptureRows::new(capacity.rows[11], successful[11]),
            routes: Arena::new(capacity.routes, successful[4]),
            law_bytes: Arena::new(capacity.law_bytes, names),
        }
    }

    pub fn reset(&mut self) {
        self.state_buildings.reset();
        self.province_buildings.reset();
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
        self.state_buildings.allocated_bytes()
            + self.province_buildings.allocated_bytes()
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

    pub fn grow(&mut self, capacity: &mut Capacities, successful: &[u16]) {
        let names = group_success(successful, 5, 7);
        self.state_buildings
            .grow(&mut capacity.rows[0], successful[0]);
        self.province_buildings
            .grow(&mut capacity.rows[1], successful[1]);
        self.base_resources
            .grow(&mut capacity.rows[2], successful[2]);
        self.building_construction
            .grow(&mut capacity.rows[3], successful[3]);
        self.railway_construction
            .grow(&mut capacity.rows[4], successful[4]);
        self.settings.grow(&mut capacity.rows[5], successful[5]);
        self.laws.grow(&mut capacity.rows[6], successful[6]);
        self.garrisons.grow(&mut capacity.rows[7], successful[7]);
        self.assigned_manpower
            .grow(&mut capacity.rows[8], successful[8]);
        self.reserve_manpower
            .grow(&mut capacity.rows[9], successful[9]);
        self.resistance.grow(&mut capacity.rows[10], successful[10]);
        self.collaboration
            .grow(&mut capacity.rows[11], successful[11]);
        self.routes.grow(&mut capacity.routes, successful[4]);
        self.law_bytes.grow(&mut capacity.law_bytes, names);
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            checked_outcome(&self.state_buildings),
            checked_outcome(&self.province_buildings),
            checked_outcome(&self.base_resources),
            checked_outcome(&self.building_construction),
            checked_outcome(&self.railway_construction),
            checked_outcome(&self.settings),
            checked_outcome(&self.laws),
            checked_outcome(&self.garrisons),
            checked_outcome(&self.assigned_manpower),
            checked_outcome(&self.reserve_manpower),
            checked_outcome(&self.resistance),
            checked_outcome(&self.collaboration),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            usize::from(self.state_buildings.checked) * self.state_buildings.mismatches,
            usize::from(self.province_buildings.checked) * self.province_buildings.mismatches,
            usize::from(self.base_resources.checked) * self.base_resources.mismatches,
            usize::from(self.building_construction.checked) * self.building_construction.mismatches,
            usize::from(self.railway_construction.checked) * self.railway_construction.mismatches,
            usize::from(self.settings.checked) * self.settings.mismatches,
            usize::from(self.laws.checked) * self.laws.mismatches,
            usize::from(self.garrisons.checked) * self.garrisons.mismatches,
            usize::from(self.assigned_manpower.checked) * self.assigned_manpower.mismatches,
            usize::from(self.reserve_manpower.checked) * self.reserve_manpower.mismatches,
            usize::from(self.resistance.checked) * self.resistance.mismatches,
            usize::from(self.collaboration.checked) * self.collaboration.mismatches,
        ]
    }

    /// All elapsed values are raw QPC ticks; the containing callback owns their
    /// conversion to nanoseconds and its own complete callback measurement.
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
    ) -> ([bool; N], [u64; N]) {
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        self.state_buildings.checked = needed[0];
        self.province_buildings.checked = needed[1];
        self.base_resources.checked = needed[2];
        self.building_construction.checked = needed[3];
        self.railway_construction.checked = needed[4];
        self.settings.checked = needed[5];
        self.laws.checked = needed[6];
        self.garrisons.checked = needed[7];
        self.assigned_manpower.checked = needed[8];
        self.reserve_manpower.checked = needed[9];
        self.resistance.checked = needed[10];
        self.collaboration.checked = needed[11];

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
        if needed[0] || needed[1] {
            let second = cbuilding_status::walk(
                memory,
                base,
                game_state,
                &mut self.state_buildings.second,
                &mut self.province_buildings.second,
            );
            if needed[0] {
                matched[0] = self.state_buildings.compare(second.states);
            }
            if needed[1] {
                matched[1] = self.province_buildings.compare(second.provinces);
            }
        }
        ticks[0] = elapsed(begin);
        (matched[2], ticks[2]) = single(&mut self.base_resources, needed[2], |output| {
            state_resource_entry::walk(memory, base, game_state, output)
        });

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
        if needed[3] || needed[4] {
            let second = cbuilding_production_line::walk(
                memory,
                base,
                game_state,
                cbuilding_production_line::Buffers {
                    buildings: &mut self.building_construction.second,
                    railways: &mut self.railway_construction.second,
                    route_provinces: &mut self.routes.second,
                },
            );
            self.routes.required = self.routes.required.max(second.required_route_len);
            if needed[3] {
                matched[3] = self.building_construction.compare(second.buildings);
            }
            if needed[4] {
                let first_routes = self.routes.first.get(..self.routes.len);
                let second_routes = self.routes.second.get(..second.route_len);
                matched[4] = compare_by(
                    &mut self.railway_construction,
                    second.railways,
                    |first, second| {
                        let mut adjusted = *second;
                        adjusted.route_start = first.route_start;
                        *first == adjusted
                            && equal_range(
                                first_routes,
                                first.route_start,
                                first.route_len,
                                second_routes,
                                second.route_start,
                                second.route_len,
                            )
                    },
                );
            }
        }
        ticks[3] = elapsed(begin);

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
        if needed[5..10].iter().any(|&value| value) {
            let second = ccountry_occupation_status::walk(
                memory,
                base,
                game_state,
                ccountry_occupation_status::Buffers {
                    settings: &mut self.settings.second,
                    laws: &mut self.laws.second,
                    garrisons: &mut self.garrisons.second,
                    manpower: &mut self.assigned_manpower.second,
                    reserve_manpower: &mut self.reserve_manpower.second,
                    names: &mut self.law_bytes.second,
                },
            );
            self.law_bytes.required = self.law_bytes.required.max(second.required_bytes);
            let first_names = self.law_bytes.first.get(..self.law_bytes.len);
            let second_names = self.law_bytes.second.get(..second.bytes_len);
            if needed[5] {
                matched[5] = compare_by(&mut self.settings, second.settings, |first, second| {
                    first.country == second.country
                        && first.garrison_template == second.garrison_template
                        && first.default_law_present == second.default_law_present
                        && equal_range(
                            first_names,
                            first.law_start,
                            first.law_len,
                            second_names,
                            second.law_start,
                            second.law_len,
                        )
                });
            }
            if needed[6] {
                matched[6] = compare_laws(&mut self.laws, second.laws, first_names, second_names);
            }
            if needed[7] {
                matched[7] = self.garrisons.compare(second.garrisons);
            }
            if needed[8] {
                matched[8] = self.assigned_manpower.compare(second.manpower);
            }
            if needed[9] {
                matched[9] = self.reserve_manpower.compare(second.reserve_manpower);
            }
        }
        ticks[5] = elapsed(begin);
        (matched[10], ticks[10]) = single(&mut self.resistance, needed[10], |output| {
            cresistance::walk(memory, base, game_state, output)
        });
        (matched[11], ticks[11]) = single(&mut self.collaboration, needed[11], |output| {
            ccountry_collaboration_status::walk(memory, base, game_state, output)
        });
        (matched, ticks)
    }

    /// Runs only on the spool thread. All twelve tables use shared sort,
    /// duplicate-key checks, field masks and native-width serialization.
    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::StateBuildings, _>(
                checked_outcome(&self.state_buildings),
                &self.state_buildings.rows,
                building,
            ),
            table::<tables::ProvinceBuildings, _>(
                checked_outcome(&self.province_buildings),
                &self.province_buildings.rows,
                building,
            ),
            table::<tables::StateBaseResources, _>(
                checked_outcome(&self.base_resources),
                &self.base_resources.rows,
                |row| rows::StateBaseResourceRow {
                    state: row.state,
                    resource: row.resource,
                    ordinal: row.ordinal,
                    amount: row.amount,
                },
            ),
            table::<tables::BuildingConstruction, _>(
                checked_outcome(&self.building_construction),
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
                checked_outcome(&self.garrisons),
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
                checked_outcome(&self.assigned_manpower),
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
                checked_outcome(&self.reserve_manpower),
                &self.reserve_manpower.rows,
                |row| rows::CountryOccupationReserveManpowerRow {
                    country: row.country,
                    ordinal: row.ordinal,
                    source_country: row.source_country,
                    amount: row.amount,
                },
            ),
            table::<tables::StateResistance, _>(
                checked_outcome(&self.resistance),
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
                checked_outcome(&self.collaboration),
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

fn single<R: Default + Clone + PartialEq>(
    rows: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> (bool, u64) {
    let begin = qpc();
    rows.outcome = walk(&mut rows.rows);
    let matched = needed && {
        let second = walk(&mut rows.second);
        rows.compare(second)
    };
    (matched, elapsed(begin))
}

fn compare_by<R>(
    rows: &mut CaptureRows<R>,
    second: Outcome,
    mut same: impl FnMut(&R, &R) -> bool,
) -> bool {
    rows.mismatches = usize::from(rows.outcome != second);
    match rows
        .rows
        .get(..rows.outcome.len())
        .zip(rows.second.get(..second.len()))
    {
        Some((first, second)) => {
            rows.mismatches += first
                .iter()
                .zip(second)
                .filter(|(first, second)| !same(first, second))
                .count();
        }
        None => rows.mismatches += 1,
    }
    full_match(rows.outcome, second, rows.mismatches)
}

fn checked_outcome<R>(rows: &CaptureRows<R>) -> Outcome {
    if rows.checked && rows.mismatches != 0 {
        Outcome::Unavailable {
            reason: "construction or occupation table changed during double read",
        }
    } else {
        rows.outcome
    }
}

fn compare_laws(
    rows: &mut CaptureRows<ccountry_occupation_data::LawRow>,
    second: Outcome,
    first_names: Option<&[u8]>,
    second_names: Option<&[u8]>,
) -> bool {
    compare_by(rows, second, |first, second| {
        (first.controller, first.occupied, first.state)
            == (second.controller, second.occupied, second.state)
            && equal_range(
                first_names,
                first.name_start,
                first.name_len,
                second_names,
                second.name_start,
                second.name_len,
            )
    })
}

fn range<T>(arena: &[T], start: usize, len: usize) -> Option<&[T]> {
    arena.get(start..start.checked_add(len)?)
}

fn equal_range<T: PartialEq>(
    first: Option<&[T]>,
    first_start: usize,
    first_len: usize,
    second: Option<&[T]>,
    second_start: usize,
    second_len: usize,
) -> bool {
    first
        .and_then(|first| range(first, first_start, first_len))
        .zip(second.and_then(|second| range(second, second_start, second_len)))
        .is_some_and(|(first, second)| first == second)
}

fn source<R>(rows: &CaptureRows<R>) -> Result<(Outcome, &[R]), TableView> {
    let outcome = checked_outcome(rows);
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
        state: row.state,
        building_type: row.building_type,
        ordinal: row.ordinal,
        status_flags: row.status_flags,
        country: row.country,
        level: row.level,
        healthy_level: row.healthy_level,
        partial_health: row.partial_health,
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
        route: routes[row.route_start..row.route_start + row.route_len].to_vec(),
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
        law: names[row.law_start..row.law_start + row.law_len].to_vec(),
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
        name: names[row.name_start..row.name_start + row.name_len].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sampling::CONSISTENCY_TARGET;
    use replay::rows::TableSchema;

    fn capacity() -> Capacities {
        Capacities {
            rows: [2; N],
            routes: 4,
            law_bytes: 8,
        }
    }

    fn fixture() -> Captured {
        let mut captured = Captured::with_capacities(&capacity(), &[0; N]);
        captured.state_buildings.outcome = Outcome::Rows { len: 1 };
        captured.province_buildings.outcome = Outcome::Rows { len: 1 };
        captured.base_resources.outcome = Outcome::Rows { len: 1 };
        captured.building_construction.outcome = Outcome::Rows { len: 1 };
        captured.railway_construction.outcome = Outcome::Rows { len: 1 };
        captured.settings.outcome = Outcome::Rows { len: 1 };
        captured.laws.outcome = Outcome::Rows { len: 1 };
        captured.garrisons.outcome = Outcome::Rows { len: 1 };
        captured.assigned_manpower.outcome = Outcome::Rows { len: 1 };
        captured.reserve_manpower.outcome = Outcome::Rows { len: 1 };
        captured.resistance.outcome = Outcome::Rows { len: 1 };
        captured.collaboration.outcome = Outcome::Rows { len: 1 };
        captured.state_buildings.rows[0] = cbuilding::Row {
            location: u32::MAX,
            state: 7,
            building_type: 99,
            ordinal: 4,
            status_flags: 0x102,
            country: 0x8000_0001,
            level: i16::MIN,
            healthy_level: i16::MAX,
            partial_health: i64::MIN,
        };
        captured.province_buildings.rows[0] = cbuilding::Row {
            location: 12,
            state: 0,
            status_flags: 1,
            ..captured.state_buildings.rows[0]
        };
        captured.base_resources.rows[0] = state_resource_entry::Row {
            state: 9,
            resource: u32::MAX,
            ordinal: 2,
            amount: i64::MIN + 1,
        };
        captured.building_construction.rows[0] = cbuilding_production_line::Row {
            id: u64::MAX,
            country: 1,
            queue_ordinal: 2,
            active_factories: i32::MAX,
            cost: i64::MAX,
            speed: i64::MIN,
            produced: -9_876_543_210,
            priority: i32::MIN,
            amount: i32::MAX,
            created_hour: 60_000_123,
            building_type: 77,
            target_flags: 0x102,
            target_location: 9,
            target_state: 9,
            target_country: u32::MAX,
            repair_levels: -3,
        };
        captured.railway_construction.rows[0] = crailway_production_line::Row {
            id: u64::MAX - 1,
            country: 1,
            queue_ordinal: 3,
            active_factories: i32::MAX,
            cost: i64::MAX,
            speed: i64::MIN,
            produced: -9_876_543_210,
            priority: i32::MIN,
            amount: i32::MAX,
            created_hour: 60_000_123,
            base_level: i32::MIN,
            target_level: i32::MAX,
            completed: 1,
            current_segment: -1,
            remaining_cost: i64::MAX - 1,
            route_start: 0,
            route_len: 3,
        };
        captured.routes.first[..3].copy_from_slice(&[7, 2, 7]);
        captured.routes.len = 3;
        captured.routes.required = 3;
        captured.settings.rows[0] = ccountry_occupation_status::Row {
            country: 1,
            garrison_template: Some(u64::MAX),
            default_law_present: 1,
            law_start: 0,
            law_len: 3,
        };
        captured.laws.rows[0] = ccountry_occupation_data::LawRow {
            controller: u32::MAX,
            occupied: u32::MAX - 1,
            state: 0,
            name_start: 3,
            name_len: 2,
        };
        captured.law_bytes.first[..5].copy_from_slice(&[b'A', 0, 255, 0, b'B']);
        captured.law_bytes.len = 5;
        captured.law_bytes.required = 5;
        captured.garrisons.rows[0] = cstate_garrison_data::Row {
            controller: 1,
            occupied: 2,
            state: 9,
            required_manpower: i32::MIN,
            strength_ratio: i64::MAX,
            garrison_required: i64::MIN,
            serialized_flag: 0xff,
        };
        captured.assigned_manpower.rows[0] = cstate_garrison_data::ManpowerRow {
            controller: 1,
            occupied: 2,
            state: 9,
            ordinal: u32::MAX,
            source_country: 0x8000_0001,
            amount: i32::MIN,
        };
        captured.reserve_manpower.rows[0] = ccountry_occupation_status::ReserveManpowerRow {
            country: 1,
            ordinal: 8,
            source_country: 0x8000_0001,
            amount: i32::MAX,
        };
        captured.resistance.rows[0] = cresistance::Row {
            state: 9,
            country: u32::MAX,
            level: i64::MIN,
            speed: i64::MAX,
            compliance: -6_000_000_001,
            compliance_speed: 7_000_000_003,
            active: 0xfe,
        };
        captured.collaboration.rows[0] = ccountry_collaboration_data::Row {
            source: u32::MAX,
            target: u32::MAX - 1,
            progress: i64::MIN,
        };
        captured
    }

    fn complete<S: TableSchema>(view: &TableView) -> &[S::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete table");
        };
        rows.rows::<S>().expect("correct typed table")
    }

    #[test]
    fn conversions_preserve_native_widths_order_bytes_and_scope() {
        let captured = fixture();
        let views = captured.tables();
        assert_eq!(
            complete::<tables::StateBuildings>(&views[0])[0].level,
            i16::MIN
        );
        assert_eq!(
            complete::<tables::StateBuildings>(&views[0])[0].partial_health,
            i64::MIN
        );
        assert_eq!(complete::<tables::ProvinceBuildings>(&views[1])[0].state, 0);
        assert_eq!(
            complete::<tables::StateBaseResources>(&views[2])[0].amount,
            i64::MIN + 1
        );
        let construction = &complete::<tables::BuildingConstruction>(&views[3])[0];
        assert_eq!(
            (
                construction.id,
                construction.cost,
                construction.speed,
                construction.produced
            ),
            (u64::MAX, i64::MAX, i64::MIN, -9_876_543_210)
        );
        assert_eq!(
            (
                construction.target_flags,
                construction.priority,
                construction.repair_levels
            ),
            (0x102, i32::MIN, -3)
        );
        let railway = &complete::<tables::RailwayConstruction>(&views[4])[0];
        assert_eq!(
            (
                railway.cancelled,
                railway.current_segment,
                railway.remaining_cost
            ),
            (1, -1, i64::MAX - 1)
        );
        assert_eq!(railway.route, [7, 2, 7]);
        assert_eq!(
            complete::<tables::OccupationSettings>(&views[5])[0].law,
            [b'A', 0, 255]
        );
        let law = &complete::<tables::StateOccupationLaws>(&views[6])[0];
        assert_eq!(
            (law.controller, law.occupied, law.state),
            (u32::MAX, u32::MAX - 1, 0)
        );
        assert_eq!(law.name, [0, b'B']);
        assert!(law.key() > u64::MAX as u128);
        assert_eq!(
            complete::<tables::StateGarrisons>(&views[7])[0].garrison_required,
            i64::MIN
        );
        assert_eq!(
            complete::<tables::StateAssignedManpower>(&views[8])[0].amount,
            i32::MIN
        );
        assert_eq!(
            complete::<tables::CountryOccupationReserveManpower>(&views[9])[0].amount,
            i32::MAX
        );
        assert_eq!(
            complete::<tables::StateResistance>(&views[10])[0].compliance,
            -6_000_000_001
        );
        assert_eq!(
            complete::<tables::CountryCollaboration>(&views[11])[0].progress,
            i64::MIN
        );
    }

    #[test]
    fn invalid_arenas_duplicate_keys_and_checked_mismatches_are_table_local() {
        let mut captured = fixture();
        captured.railway_construction.rows[0].route_start = usize::MAX;
        captured.settings.rows[0].law_start = 6;
        captured.state_buildings.checked = true;
        captured.state_buildings.mismatches = 1;
        let views = captured.tables();
        for index in [0, 4, 5] {
            assert!(matches!(views[index], TableView::Unavailable { .. }));
        }
        assert!(matches!(views[1], TableView::Complete(_)));
        assert!(matches!(views[6], TableView::Complete(_)));
        captured.building_construction.rows[1] = captured.building_construction.rows[0];
        captured.building_construction.outcome = Outcome::Rows { len: 2 };
        assert!(matches!(
            captured.tables()[3],
            TableView::Unavailable { .. }
        ));
        captured.laws.rows[1] = captured.laws.rows[0];
        captured.laws.rows[1].occupied -= 1;
        captured.laws.outcome = Outcome::Rows { len: 2 };
        assert_eq!(
            complete::<tables::StateOccupationLaws>(&captured.tables()[6]).len(),
            2
        );
        captured.province_buildings.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 5,
        };
        assert!(matches!(
            captured.tables()[1],
            TableView::Truncated { missing: 5, .. }
        ));
        captured.reset();
        assert_eq!(captured.checked_mismatches(), [0; N]);
        assert!(
            captured
                .outcomes()
                .iter()
                .all(|outcome| matches!(outcome, Outcome::Unavailable { .. }))
        );
    }

    #[test]
    fn changed_sibling_arena_positions_do_not_change_a_laws_native_value() {
        let mut rows = CaptureRows::<ccountry_occupation_data::LawRow>::new(1, 0);
        rows.checked = true;
        rows.outcome = Outcome::Rows { len: 1 };
        rows.rows[0] = ccountry_occupation_data::LawRow {
            controller: 1,
            occupied: 2,
            state: 0,
            name_start: 3,
            name_len: 2,
        };
        rows.second[0] = ccountry_occupation_data::LawRow {
            name_start: 0,
            ..rows.rows[0]
        };
        assert!(compare_laws(
            &mut rows,
            Outcome::Rows { len: 1 },
            Some(b"abcXY"),
            Some(b"XY")
        ));
        assert_eq!(rows.mismatches, 0);
        assert!(!compare_laws(
            &mut rows,
            Outcome::Rows { len: 1 },
            Some(b"abcXY"),
            Some(b"XZ")
        ));
        assert_eq!(rows.mismatches, 1);
        assert!(matches!(
            checked_outcome(&rows),
            Outcome::Unavailable { .. }
        ));
        rows.checked = false;
        assert_eq!(checked_outcome(&rows), Outcome::Rows { len: 1 });
    }

    #[test]
    fn growth_counts_omitted_rows_and_releases_completed_comparisons_independently() {
        let mut capacity = capacity();
        let mut captured = Captured::with_capacities(&capacity, &[0; N]);
        captured.railway_construction.outcome = Outcome::Truncated {
            len: 1,
            unread_rows: 20,
        };
        captured.routes.required = 29;
        captured.law_bytes.required = 41;
        let mut successful = [CONSISTENCY_TARGET; N];
        successful[6] = 0;
        captured.grow(&mut capacity, &successful);
        assert!(capacity.rows[4] >= 21 && capacity.routes >= 29 && capacity.law_bytes >= 41);
        assert!(captured.settings.second.is_empty());
        assert!(!captured.laws.second.is_empty());
        assert!(captured.railway_construction.second.is_empty());
        assert!(captured.routes.second.is_empty());
        assert!(!captured.law_bytes.second.is_empty());
        let before = captured.allocated_bytes();
        successful[6] = CONSISTENCY_TARGET;
        captured.grow(&mut capacity, &successful);
        assert!(captured.laws.second.is_empty() && captured.law_bytes.second.is_empty());
        assert!(captured.allocated_bytes() < before);
        let fresh = Captured::with_capacities(&capacity, &successful);
        assert!(fresh.laws.second.is_empty() && fresh.routes.second.is_empty());
    }
}
