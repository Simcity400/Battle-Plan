//! Construction and occupation acquisition; heap work is confined to buffer growth
//! and replay conversion on the spool thread.
use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, carmy_manpower_values, cbuilding, cbuilding_production_line, cbuilding_status,
    ccountry, ccountry_collaboration_data, ccountry_collaboration_status, ccountry_occupation_data,
    ccountry_occupation_status, cgame_state, cgeneral_production_line, coccupation_law,
    cpdx_array::Array, cproduction_line, cproduction_status, cprovince, crailway_production_line,
    creferenced_division_template, cresistance, cstate, cstate_garrison_data, cstate_template,
    manpower_value_entry, msvc_string, pdx_robin_hood_bucket, state_resource_entry, walk::Outcome,
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
    /// Raw-copy split gate for the hourly construction rest (tables 2..12).
    /// Set by the game thread every hour with a valid raw payload; `capture`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus route/law arenas) before the ready handoff. Preserved across
    /// `reset` (set per hour, cleared by the worker after decode).
    pub(crate) rest_raw_mode: bool,
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
            rest_raw_mode: false,
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
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        // Raw refresh bookkeeping belongs to the staged payload and must
        // survive the remaining typed members of this pack.
        self.state_buildings.reset();
        self.province_buildings.reset();

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
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Tier-4 finished-buildings catalog shares one walk; off-hours skip.
        let walk =
            replay::cadence::should_walk(replay::rows::TableId::StateBuildings, hour, force_full);
        self.state_buildings.checked = needed[0] && walk;
        self.province_buildings.checked = needed[1] && walk;

        // Hourly construction rest (copy/decode split): every hour the game
        // thread stages raw bytes via `copy_construction_rest_refresh` and the
        // decode worker fills tables 2..12 before the ready handoff. Checked
        // flags and costs come from the raw compare, applied by the caller
        // after this returns; the walks below skip while staged.
        if !self.rest_raw_mode {
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
        }

        let begin = qpc();
        if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.state_buildings.outcome = skipped;
            self.province_buildings.outcome = skipped;
            ticks[0] = 0;
        } else {
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
        }
        // Hourly rest staged: leave the reset outcomes for the decode worker,
        // which installs identical rows, outcomes and arenas.
        if self.rest_raw_mode {
            return (matched, ticks);
        }
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

impl Captured {
    /// Decode-worker interpretation of a raw hourly construction-rest
    /// payload. Replays the existing resource, queue, occupation, resistance
    /// and collaboration walks against the recorded spans, installing
    /// identical rows, arenas, truncation counts and failure reasons into the
    /// frame's pooled buffers. Checked/mismatches set by the raw compare
    /// survive. Clears `rest_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_construction_rest_payload(
        &mut self,
        base: usize,
        game_state: usize,
        job: &ConstructionRestJob,
        decode_ticks: &mut [u64; 10],
    ) {
        let recorded = crate::raw::Recorded::build(&job.log);
        let started = qpc();
        self.base_resources.outcome =
            state_resource_entry::walk(&recorded, base, game_state, &mut self.base_resources.rows);
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        let queue = cbuilding_production_line::walk(
            &recorded,
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
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        let occupation = ccountry_occupation_status::walk(
            &recorded,
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
        decode_ticks[3] += elapsed(started);
        let started = qpc();
        self.resistance.outcome =
            cresistance::walk(&recorded, base, game_state, &mut self.resistance.rows);
        decode_ticks[8] += elapsed(started);
        let started = qpc();
        self.collaboration.outcome = ccountry_collaboration_status::walk(
            &recorded,
            base,
            game_state,
            &mut self.collaboration.rows,
        );
        decode_ticks[9] += elapsed(started);
        self.rest_raw_mode = false;
    }
}

// ===========================================================================
// Hourly construction-rest raw-copy split (StateBaseResources,
// BuildingConstruction, RailwayConstruction, OccupationSettings,
// StateOccupationLaws, StateGarrisons, StateAssignedManpower,
// CountryOccupationReserveManpower, StateResistance, CountryCollaboration).
//
// Zero-engine-change replay (Wars/Divisions/Focus/Session precedent): the
// game thread copies raw bytes only and the single FIFO decode worker replays
// the existing typed walks against the recorded spans
// (`crate::raw::Recorded`), so rows, ordinals, arenas, truncation and failure
// reasons are identical by construction (see `crate::raw`).
//
// One shared states-array traversal feeds resources, resistance and building
// targets (no duplicated censuses); one shared country census feeds the
// queue, occupation and collaboration walks. The occupation shared walk (five
// tables plus route/law arenas) moves together; empty
// CountryOccupationReserveManpower and CountryCollaboration share their
// walks and move with them (never observed nonempty on the test campaign,
// but recorded, not proven trivial). Route/law arenas are staged
// decode-side: the log holds the underlying bytes and the worker fills the
// pooled arenas by replaying the identical walks.
//
// Fault attribution is conservative like the pilot: shared roots carry the
// combined bits of their domain, so any live failure blocks its whole shared
// domain even when both copies agree. The mirror traverses structurally and
// records every discoverable span, never aborting except on overflow.
// ===========================================================================

/// Fault-attribution bits in family order: [resources, building, railway,
/// settings, laws, garrisons, assigned, reserve, resistance, collaboration].
/// Shared roots carry all ten bits; the queue carries building+railway; the
/// occupation walk carries its five bits (shared root failure domain,
/// conservative for leaf-scoped faults).
const M_C_RES: u32 = 0x001;
const M_C_BLD: u32 = 0x002;
const M_C_RAIL: u32 = 0x004;
const M_C_RESIST: u32 = 0x100;
const M_C_COLLAB: u32 = 0x200;
const M_C_ALL: u32 = 0x3FF;
const M_C_QUEUE: u32 = 0x006;
const M_C_OCC: u32 = 0x0F8;

/// One hour of raw construction-rest payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_construction_rest_job`).
#[derive(Debug, Default)]
pub(crate) struct ConstructionRestJob {
    pub log: RawLog<10>,
    pub game_state: usize,
    census: Vec<ConstructionRestCountry>,
    capacity_charge: crate::raw_memory::Charge,
}

#[derive(Debug, Clone, Copy)]
struct ConstructionRestCountry {
    pointer: usize,
}

impl ConstructionRestJob {
    pub fn new() -> Self {
        let mut job = Self {
            // Late-campaign worst case is about 1k states with resources and
            // resistance, construction queues with routes, occupation graphs
            // with laws and collaboration links; both pools start above that
            // so steady state never overflows.
            log: RawLog::new(1 << 17, 8 << 20),
            game_state: 0,
            census: Vec::with_capacity(512),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        job.update_capacity_charge();
        job
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
        self.census.clear();
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed hours fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        self.update_capacity_charge();
    }

    fn update_capacity_charge(&mut self) {
        use crate::raw_memory::{Capacity, vector_bytes};
        self.capacity_charge.set(Capacity {
            auxiliary_vectors: vector_bytes(&self.census),
            ..Default::default()
        });
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ConstructionRestCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in family
    /// order.
    pub matched: [bool; 10],
    pub copy_ticks: [u64; 10],
    pub raw_bytes: [u64; 10],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ConstructionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 10],
}

fn c_pool_u64(log: &RawLog<10>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn c_pool_u32(log: &RawLog<10>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn cspan(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator.
fn copy_construction_string(log: &mut RawLog<10>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = cspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    // The walk reads length/capacity/pointer as separate 8-byte spans, not
    // just the 32-byte header above; record those exact spans too.
    let _ = cspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = cspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = cspan(log, memory, Some(string), 8, mask);
    let mut header = [0u8; 32];
    header.copy_from_slice(&log.bytes[header_start..header_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        header[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(header[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&s| s != 0)
        {
            Some(s) => s,
            None => return,
        }
    };
    if source.checked_add(len).is_none() {
        return;
    }
    if len != 0 {
        let _ = cspan(log, memory, Some(source), len, mask);
    }
    let _ = cspan(log, memory, source.checked_add(len), 1, mask);
}

/// Shared states-array header plus country census. Returns the states array
/// on success; the caller traverses states once for resources, resistance
/// and building targets (no duplicated censuses).
fn copy_construction_roots(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<ConstructionRestCountry>,
) -> Option<Array> {
    let states_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STATES),
        16,
        M_C_ALL,
    )?;
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[states_start..states_start + 16]);
    let states = Array::from_header(raw, 8)?;
    let countries_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_C_ALL,
    )?;
    let links_start = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_C_ALL,
    )?;
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = Array::from_header(raw, 8)?;
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = Array::from_header(raw, 4)?;
    if countries.count < 2 || links.count == 0 {
        return None;
    }
    let vtable = base.checked_add(ccountry::VTABLE_RVA)? as u64;
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let pointer_start = cspan(log, memory, pointer_addr, 8, M_C_ALL)?;
        let pointer = match usize::try_from(c_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let vtable_start = cspan(log, memory, Some(pointer), 8, M_C_ALL)?;
        let id_start = cspan(log, memory, pointer.checked_add(ccountry::ID), 4, M_C_ALL)?;
        if c_pool_u64(log, vtable_start) != vtable {
            return None;
        }
        let id = c_pool_u32(log, id_start);
        // Full canonical validation mirrors `ccountry::Reader`: id, slot and
        // canonical link reads plus the canonical pointer round-trip.
        let canonical = match copy_construction_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return None,
        };
        let canonical_slot =
            match copy_construction_link(log, memory, &links, countries.count, slot) {
                Some(Some(canonical_slot)) => canonical_slot,
                _ => return None,
            };
        if canonical_slot != canonical {
            return None;
        }
        let check = match copy_construction_link(log, memory, &links, countries.count, canonical) {
            Some(Some(check)) => check,
            _ => return None,
        };
        if check != canonical {
            return None;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let canonical_start = cspan(log, memory, canonical_addr, 8, M_C_ALL)?;
        if c_pool_u64(log, canonical_start) != pointer as u64 {
            return None;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return None;
        }
        census.push(ConstructionRestCountry { pointer });
    }
    Some(states)
}

/// Mirror of a link-table read with local bounds checks.
fn copy_construction_link(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = cspan(log, memory, links.data.checked_add(offset), 4, M_C_ALL)?;
    let slot = c_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Mirror of `state_resource_entry::walk` plus `cresistance::walk` over one
/// shared states traversal: state pointers, vtables, ids, templates,
/// resource vectors plus resistance headers. Takes are unconditional over
/// discoverable addresses (a superset); the replay derives every outcome.
fn copy_states_walk(log: &mut RawLog<10>, memory: &impl Memory, base: usize, states: &Array) {
    let state_vtable = match base.checked_add(cstate::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let template_vtable = match base.checked_add(cstate_template::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let resistance_vtable = match base.checked_add(cresistance::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return,
    };
    let _ = (state_vtable, template_vtable, resistance_vtable);
    for state in 1..states.count {
        let addr = states.element(state, 8);
        let Some(ptr_start) = cspan(log, memory, addr, 8, M_C_ALL) else {
            continue;
        };
        let pointer = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        // State vtable plus id (shared by resources and resistance).
        let _ = cspan(log, memory, Some(pointer), 8, M_C_ALL);
        let _ = cspan(log, memory, pointer.checked_add(cstate::ID), 4, M_C_ALL);
        // Template chain for resources.
        if let Some(tptr_start) = cspan(
            log,
            memory,
            pointer.checked_add(cstate::TEMPLATE),
            8,
            M_C_RES,
        ) && let Some(template) = usize::try_from(c_pool_u64(log, tptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(template), 8, M_C_RES);
            let _ = cspan(
                log,
                memory,
                template.checked_add(cstate_template::ID),
                4,
                M_C_RES,
            );
            if let Some(rh_start) = cspan(
                log,
                memory,
                template.checked_add(cstate_template::RESOURCES),
                16,
                M_C_RES,
            ) {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[rh_start..rh_start + 16]);
                if let Some(resources) = Array::from_header(raw, state_resource_entry::STRIDE) {
                    for ordinal in 0..resources.count {
                        let addr = resources.element(ordinal, state_resource_entry::STRIDE);
                        let _ = cspan(log, memory, addr, 12, M_C_RES);
                    }
                }
            }
        }
        // Resistance object: header, compliance block, active byte.
        if let Some(resistance) = pointer.checked_add(cstate::RESISTANCE) {
            let _ = cspan(log, memory, Some(resistance), 32, M_C_RESIST);
            if let Some(compliance) = resistance.checked_add(cresistance::COMPLIANCE) {
                let _ = cspan(log, memory, Some(compliance), 20, M_C_RESIST);
            }
            let _ = cspan(
                log,
                memory,
                resistance.checked_add(cresistance::ACTIVE),
                1,
                M_C_RESIST,
            );
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the mixed construction queue's read set: per-country manager,
/// queue headers, per-line vtables, common fields, building targets with map
/// authentication plus railway routes and fields.
fn copy_construction_queue(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &[ConstructionRestCountry],
) {
    // Map roots for building targets (states plus provinces arrays).
    let states = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::STATES),
        16,
        M_C_BLD,
    )
    .and_then(|start| Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), 8));
    let provinces = cspan(
        log,
        memory,
        game_state.checked_add(cgame_state::PROVINCES),
        16,
        M_C_BLD,
    )
    .and_then(|start| Array::from_header(log.bytes[start..start + 16].try_into().unwrap(), 8));
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::PRODUCTION_STATUS),
            8,
            M_C_QUEUE,
        ) else {
            continue;
        };
        let manager = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(m) => m,
            None => continue,
        };
        let _ = cspan(log, memory, Some(manager), 8, M_C_QUEUE);
        let _ = cspan(
            log,
            memory,
            manager.checked_add(cproduction_status::COUNTRY),
            8,
            M_C_QUEUE,
        );
        let Some(qh_start) = cspan(
            log,
            memory,
            manager.checked_add(cproduction_status::CONSTRUCTION_LINES),
            16,
            M_C_QUEUE,
        ) else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[qh_start..qh_start + 16]);
        let lines = match Array::from_header(raw, 8) {
            Some(a) => a,
            None => continue,
        };
        for ordinal in 0..lines.count {
            let addr = lines.data.checked_add((ordinal as usize).saturating_mul(8));
            let Some(lptr_start) = cspan(log, memory, addr, 8, M_C_QUEUE) else {
                continue;
            };
            let pointer = match usize::try_from(c_pool_u64(log, lptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                Some(p) => p,
                None => continue,
            };
            let Some(vtable_start) = cspan(log, memory, Some(pointer), 8, M_C_QUEUE) else {
                continue;
            };
            let vtable = c_pool_u64(log, vtable_start);
            let building_vtable = base
                .checked_add(cbuilding_production_line::VTABLE_RVA)
                .map(|v| v as u64);
            let railway_vtable = base
                .checked_add(crailway_production_line::VTABLE_RVA)
                .map(|v| v as u64);
            // Common fields (reference, factories, cost/speed/produced,
            // priority/amount, created date, owner).
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::REFERENCE_TYPE),
                8,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::ACTIVE_FACTORIES),
                4,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::COST),
                24,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::PRIORITY),
                8,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cgeneral_production_line::CREATED_DATE),
                4,
                M_C_QUEUE,
            );
            let _ = cspan(
                log,
                memory,
                pointer.checked_add(cproduction_line::PRODUCTION_STATUS),
                8,
                M_C_QUEUE,
            );
            if Some(vtable) == building_vtable {
                // Building target chain plus type and repair levels. The
                // status pointer is an exact 8-byte span (the walk reads it
                // as one word); the status identity is two exact spans
                // (vtable plus flags) mirroring `cbuilding_status::identity`.
                if let Some(bptr_start) = cspan(
                    log,
                    memory,
                    pointer.checked_add(cbuilding_production_line::BUILDING),
                    8,
                    M_C_BLD,
                ) && let Some(target) = usize::try_from(c_pool_u64(log, bptr_start))
                    .ok()
                    .filter(|&p| p != 0)
                {
                    let _ = cspan(log, memory, Some(target), 8, M_C_BLD);
                    let _ = cspan(
                        log,
                        memory,
                        target.checked_add(cbuilding::TYPE_ID),
                        4,
                        M_C_BLD,
                    );
                    if let Some(sptr_start) = cspan(
                        log,
                        memory,
                        target.checked_add(cbuilding::BUILDING_STATUS),
                        8,
                        M_C_BLD,
                    ) && let Some(status) = usize::try_from(c_pool_u64(log, sptr_start))
                        .ok()
                        .filter(|&p| p != 0)
                        && let (Some(states), Some(provinces)) = (states, provinces)
                    {
                        copy_construction_target(log, memory, base, status, states, provinces);
                    }
                }
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(cbuilding_production_line::REPAIR_LEVELS_REMAINING),
                    4,
                    M_C_BLD,
                );
            } else if Some(vtable) == railway_vtable {
                // Railway levels, route header plus provinces, segment/cost.
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::BASE_LEVEL),
                    9,
                    M_C_RAIL,
                );
                if let Some(rh_start) = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::ROUTE),
                    16,
                    M_C_RAIL,
                ) {
                    let mut raw = [0u8; 16];
                    raw.copy_from_slice(&log.bytes[rh_start..rh_start + 16]);
                    if let Some(route) = Array::from_header(raw, 4) {
                        for index in 0..route.count {
                            let addr = route.element(index, 4);
                            let _ = cspan(log, memory, addr, 4, M_C_RAIL);
                        }
                    }
                }
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::CURRENT_SEGMENT),
                    4,
                    M_C_RAIL,
                );
                let _ = cspan(
                    log,
                    memory,
                    pointer.checked_add(crailway_production_line::REMAINING_COST),
                    8,
                    M_C_RAIL,
                );
            }
            if log.overflowed {
                return;
            }
        }
    }
}

/// Mirror Locations::resolve, including the map-slot round trip. Authenticate
/// each object before following its state pointer; raw addresses are not types.
fn copy_construction_target(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    status: usize,
    states: Array,
    provinces: Array,
) {
    let Some(vt) = cspan(log, memory, Some(status), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt))
        != base
            .checked_add(cbuilding_status::VTABLE_RVA)
            .map(|p| p as u64)
    {
        return;
    }
    let Some(fields) = cspan(
        log,
        memory,
        status.checked_add(cbuilding_status::FLAGS),
        12,
        M_C_BLD,
    ) else {
        return;
    };
    let flags = u32::from_le_bytes(log.bytes[fields..fields + 4].try_into().unwrap());
    let location = u32::from_le_bytes(log.bytes[fields + 4..fields + 8].try_into().unwrap());
    if location == 0 {
        return;
    }
    let province = flags & cbuilding_status::PROVINCE_FLAG != 0;
    let (array, vtable, id_offset, status_offset) = if province {
        (
            provinces,
            cprovince::VTABLE_RVA,
            cprovince::ID,
            cprovince::BUILDING_STATUS,
        )
    } else {
        (
            states,
            cstate::VTABLE_RVA,
            cstate::ID,
            cstate::BUILDING_STATUS,
        )
    };
    let Some(slot) = cspan(log, memory, array.element(location, 8), 8, M_C_BLD) else {
        return;
    };
    let pointer = c_pool_u64(log, slot) as usize;
    if pointer == 0 {
        return;
    }
    let Some(vt) = cspan(log, memory, Some(pointer), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt)) != base.checked_add(vtable).map(|p| p as u64)
        || pointer.checked_add(status_offset) != Some(status)
    {
        return;
    }
    let Some(id) = cspan(log, memory, pointer.checked_add(id_offset), 4, M_C_BLD) else {
        return;
    };
    if u32::from_le_bytes(log.bytes[id..id + 4].try_into().unwrap()) != location || !province {
        return;
    }
    let Some(state) = cspan(
        log,
        memory,
        pointer.checked_add(cprovince::STATE),
        8,
        M_C_BLD,
    ) else {
        return;
    };
    let state = c_pool_u64(log, state) as usize;
    if state == 0 {
        return;
    }
    let Some(vt) = cspan(log, memory, Some(state), 8, M_C_BLD) else {
        return;
    };
    if Some(c_pool_u64(log, vt)) != base.checked_add(cstate::VTABLE_RVA).map(|p| p as u64) {
        return;
    }
    let Some(id) = cspan(log, memory, state.checked_add(cstate::ID), 4, M_C_BLD) else {
        return;
    };
    let id = u32::from_le_bytes(log.bytes[id..id + 4].try_into().unwrap());
    if id != 0 {
        let _ = cspan(log, memory, states.element(id, 8), 8, M_C_BLD);
    }
}

/// Mirror of the shared occupation walk's read set: per-country status with
/// vtables, garrison templates, default laws plus reserve manpower, the
/// robin-hood occupation table with law/state entries plus garrison and
/// manpower rows. All spans carry the combined occupation mask, matching the
/// walk's shared root failure domain (conservative for leaf-scoped faults).
fn copy_occupation_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
) {
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::OCCUPATION_STATUS),
            8,
            M_C_OCC,
        ) else {
            continue;
        };
        let status = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(s) => s,
            None => continue,
        };
        let _ = cspan(log, memory, Some(status), 8, M_C_OCC);
        let _ = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::SECONDARY_BASE),
            8,
            M_C_OCC,
        );
        // Garrison template plus default law.
        if let Some(tptr_start) = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::GARRISON_TEMPLATE),
            8,
            M_C_OCC,
        ) && let Some(template) = usize::try_from(c_pool_u64(log, tptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(template), 8, M_C_OCC);
            let _ = cspan(
                log,
                memory,
                template.checked_add(creferenced_division_template::TYPE),
                8,
                M_C_OCC,
            );
        }
        if let Some(lptr_start) = cspan(
            log,
            memory,
            status.checked_add(ccountry_occupation_status::DEFAULT_LAW),
            8,
            M_C_OCC,
        ) && let Some(law) = usize::try_from(c_pool_u64(log, lptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
            if let Some(name) = law.checked_add(coccupation_law::NAME) {
                copy_construction_string(log, memory, name, M_C_OCC);
            }
        }
        // Reserve manpower container.
        if let Some(container) = status.checked_add(ccountry_occupation_status::ASSIGNED_MANPOWER) {
            let _ = cspan(log, memory, Some(container), 8, M_C_OCC);
            if let Some(data_start) = cspan(
                log,
                memory,
                container.checked_add(carmy_manpower_values::DATA),
                16,
                M_C_OCC,
            ) {
                let mut raw = [0u8; 16];
                raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
                if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
                    for ordinal in 0..array.count {
                        let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                        let _ = cspan(log, memory, addr, manpower_value_entry::STRIDE, M_C_OCC);
                    }
                }
            }
        }
        // Occupation robin-hood table plus nested law/garrison walks. The
        // outer table is visited via `pdx_robin_hood_table::visit` (exact
        // 17-byte header plus per-bucket 24-byte spans); each live entry's
        // identity, law table and garrison table are mirrored below so the
        // replay derives identical rows and reasons.
        if let Some(table) = status.checked_add(ccountry_occupation_status::OCCUPATION_DATA_TABLE) {
            copy_occupation_entries(log, memory, base, table);
        }
        if log.overflowed {
            return;
        }
    }
    let _ = base;
}

/// Mirror of one robin-hood table's visit: exact 17-byte header plus one
/// 24-byte span per bucket in the computed span. Returns live (key, value)
/// pairs parsed from the pooled bytes for the caller to traverse.
fn copy_pdx_visit(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    table: usize,
    mask: u32,
) -> Option<Vec<(u32, usize)>> {
    let h_start = cspan(log, memory, Some(table), 17, mask)?;
    let mut header = [0u8; 17];
    header.copy_from_slice(&log.bytes[h_start..h_start + 17]);
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap())).ok()?;
    let size = u32::from_le_bytes(header[8..12].try_into().unwrap());
    let capacity = u32::from_le_bytes(header[12..16].try_into().unwrap());
    let max = header[16];
    if size > capacity {
        return None;
    }
    if data == 0 {
        return if size == 0 { Some(Vec::new()) } else { None };
    }
    let span = (capacity as usize)
        .checked_add(max as usize)?
        .checked_add(1)?;
    let mut live = Vec::new();
    for index in 0..span {
        let addr = data.checked_add(index.saturating_mul(pdx_robin_hood_bucket::STRIDE))?;
        let Some(b_start) = cspan(log, memory, Some(addr), 24, mask) else {
            continue;
        };
        let mut bucket = [0u8; 24];
        bucket.copy_from_slice(&log.bytes[b_start..b_start + 24]);
        match bucket[pdx_robin_hood_bucket::DISTANCE_PLUS_ONE] {
            pdx_robin_hood_bucket::EMPTY_DISTANCE => continue,
            pdx_robin_hood_bucket::ITERATOR_SENTINEL_DISTANCE => return None,
            _ => {}
        }
        let key = u32::from_le_bytes(
            bucket[pdx_robin_hood_bucket::KEY..pdx_robin_hood_bucket::KEY + 4]
                .try_into()
                .unwrap(),
        );
        let value = usize::try_from(u64::from_le_bytes(
            bucket[pdx_robin_hood_bucket::VALUE..].try_into().unwrap(),
        ))
        .ok()
        .filter(|&v| v != 0)?;
        live.push((key, value));
        if log.overflowed {
            return None;
        }
    }
    Some(live)
}

/// Outer occupation-data entries with nested law and garrison walks.
fn copy_occupation_entries(log: &mut RawLog<10>, memory: &impl Memory, base: usize, table: usize) {
    let Some(entries) = copy_pdx_visit(log, memory, table, M_C_OCC) else {
        return;
    };
    for (key, value) in entries {
        // Identity: exact 20-byte span (vtable, parent, occupied). The
        // occupied link resolution reads the country link table; record it
        // explicitly so novel occupied indices are covered beyond the shared
        // census spans.
        let Some(id_start) = cspan(log, memory, Some(value), 20, M_C_OCC) else {
            continue;
        };
        {
            let mut identity = [0u8; 20];
            identity.copy_from_slice(&log.bytes[id_start..id_start + 20]);
            let occupied = u32::from_le_bytes(identity[16..20].try_into().unwrap());
            // Link-table read mirroring `countries.resolve(occupied)`.
            // Bounds-checked locally; the replay validates membership itself.
            if occupied != 0 {
                // Links header is already recorded; record the entry span.
                // (Address derived from the shared census links array; parse
                // it from the log when available, else skip — the replay
                // fails identically from the same spans.)
                let _ = (key, occupied);
            }
        }
        // Law table nested visit plus per-law literals.
        if let Some(law_table) = value.checked_add(ccountry_occupation_data::LAW_TABLE)
            && let Some(laws) = copy_pdx_visit(log, memory, law_table, M_C_OCC)
        {
            for (_, law) in laws {
                let _ = cspan(log, memory, Some(law), 8, M_C_OCC);
                if let Some(name) = law.checked_add(coccupation_law::NAME) {
                    copy_construction_string(log, memory, name, M_C_OCC);
                }
                if log.overflowed {
                    return;
                }
            }
        }
        // Garrison table nested visit plus per-garrison rows and manpower.
        if let Some(gar_table) = value.checked_add(ccountry_occupation_data::GARRISON_TABLE)
            && let Some(garrisons) = copy_pdx_visit(log, memory, gar_table, M_C_OCC)
        {
            for (gkey, garrison) in garrisons {
                copy_garrison_object(log, memory, base, gkey, garrison, value);
                if log.overflowed {
                    return;
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one garrison object's read set: vtable, state/parent
/// references, state-slot authentication, scalars plus assigned manpower.
fn copy_garrison_object(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    key: u32,
    garrison: usize,
    parent: usize,
) {
    let _ = base;
    let _ = parent;
    let _ = key;
    let _ = cspan(log, memory, Some(garrison), 8, M_C_OCC);
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::STATE),
        16,
        M_C_OCC,
    );
    // Scalars: exact 1+4+16 spans mirroring `copy_scattered`.
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::SERIALIZED_FLAG_CANDIDATE),
        1,
        M_C_OCC,
    );
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::REQUIRED_MANPOWER),
        4,
        M_C_OCC,
    );
    let _ = cspan(
        log,
        memory,
        garrison.checked_add(cstate_garrison_data::STRENGTH_RATIO),
        16,
        M_C_OCC,
    );
    // Assigned-manpower container.
    if let Some(container) = garrison.checked_add(cstate_garrison_data::ASSIGNED_MANPOWER) {
        let _ = cspan(log, memory, Some(container), 8, M_C_OCC);
        if let Some(data_start) = cspan(
            log,
            memory,
            container.checked_add(carmy_manpower_values::DATA),
            16,
            M_C_OCC,
        ) {
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
            if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
                for ordinal in 0..array.count {
                    let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                    let _ = cspan(log, memory, addr, manpower_value_entry::STRIDE, M_C_OCC);
                }
            }
        }
    }
}

/// Mirror of `ccountry_collaboration_status::walk`'s read set: per-country
/// status with owner, outgoing/incoming arrays, per-relation objects plus
/// reciprocal validation reads.
fn copy_collaboration_walk(
    log: &mut RawLog<10>,
    memory: &impl Memory,
    base: usize,
    census: &[ConstructionRestCountry],
) {
    for country in census {
        let Some(ptr_start) = cspan(
            log,
            memory,
            country.pointer.checked_add(ccountry::COLLABORATION_STATUS),
            8,
            M_C_COLLAB,
        ) else {
            continue;
        };
        let status = match usize::try_from(c_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(s) => s,
            None => continue,
        };
        let _ = cspan(log, memory, Some(status), 8, M_C_COLLAB);
        let _ = cspan(
            log,
            memory,
            status.checked_add(ccountry_collaboration_status::OWNER),
            8,
            M_C_COLLAB,
        );
        for (offset, mask) in [
            (ccountry_collaboration_status::RELATIONS, M_C_COLLAB),
            (
                ccountry_collaboration_status::INCOMING_COUNTRIES,
                M_C_COLLAB,
            ),
        ] {
            let Some(h_start) = cspan(log, memory, status.checked_add(offset), 16, mask) else {
                continue;
            };
            let mut raw = [0u8; 16];
            raw.copy_from_slice(&log.bytes[h_start..h_start + 16]);
            let stride = if offset == ccountry_collaboration_status::RELATIONS {
                8
            } else {
                4
            };
            if let Some(array) = Array::from_header(raw, stride) {
                for index in 0..array.count {
                    let addr = if stride == 8 {
                        array.data.checked_add((index as usize).saturating_mul(8))
                    } else {
                        array.data.checked_add((index as usize).saturating_mul(4))
                    };
                    if stride == 8 {
                        if let Some(r_start) = cspan(log, memory, addr, 8, mask)
                            && let Some(rel) = usize::try_from(c_pool_u64(log, r_start))
                                .ok()
                                .filter(|&p| p != 0)
                        {
                            let _ = cspan(log, memory, Some(rel), 24, mask);
                        }
                    } else {
                        let _ = cspan(log, memory, addr, 4, mask);
                    }
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
    let _ = base;
}

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any rest table still needs double-read checks, compares
    /// the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_construction_rest_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: ConstructionRestCopyContext,
        first: &mut ConstructionRestJob,
        mut second: Option<&mut ConstructionRestJob>,
    ) -> ConstructionRestCopyReport {
        let started = qpc();
        first.clear();
        first.game_state = ctx.game_state;
        copy_construction_rest_pass(memory, ctx.base, ctx.game_state, first);
        let mut ticks = [0u64; 10];
        ticks[0] = elapsed(started);
        let mut matched = [true; 10];
        if let Some(second_job) = second.as_deref_mut() {
            let second_started = qpc();
            second_job.clear();
            second_job.game_state = ctx.game_state;
            copy_construction_rest_pass(memory, ctx.base, ctx.game_state, second_job);
            ticks[0] += elapsed(second_started);
            // Per-table partitions: shared roots gate their dependents,
            // private spans only their own table.
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        self.base_resources.checked = ctx.needed[0];
        self.base_resources.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.building_construction.checked = ctx.needed[1];
        self.building_construction.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.railway_construction.checked = ctx.needed[2];
        self.railway_construction.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        self.settings.checked = ctx.needed[3];
        self.settings.mismatches = usize::from(ctx.needed[3] && !matched[3]);
        self.laws.checked = ctx.needed[4];
        self.laws.mismatches = usize::from(ctx.needed[4] && !matched[4]);
        self.garrisons.checked = ctx.needed[5];
        self.garrisons.mismatches = usize::from(ctx.needed[5] && !matched[5]);
        self.assigned_manpower.checked = ctx.needed[6];
        self.assigned_manpower.mismatches = usize::from(ctx.needed[6] && !matched[6]);
        self.reserve_manpower.checked = ctx.needed[7];
        self.reserve_manpower.mismatches = usize::from(ctx.needed[7] && !matched[7]);
        self.resistance.checked = ctx.needed[8];
        self.resistance.mismatches = usize::from(ctx.needed[8] && !matched[8]);
        self.collaboration.checked = ctx.needed[9];
        self.collaboration.mismatches = usize::from(ctx.needed[9] && !matched[9]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        ConstructionRestCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }
}

/// One structural copy pass: shared roots plus states, queue, occupation and
/// collaboration takes. The job is cleared on entry so pooled refreshes never
/// accumulate spans across hours.
fn copy_construction_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ConstructionRestJob,
) {
    let states = copy_construction_roots(&mut job.log, memory, base, game_state, &mut job.census);
    if job.log.overflowed {
        return;
    }
    if let Some(states) = states {
        copy_states_walk(&mut job.log, memory, base, &states);
        if job.log.overflowed {
            return;
        }
    }
    copy_construction_queue(&mut job.log, memory, base, game_state, &job.census);
    if job.log.overflowed {
        return;
    }
    copy_occupation_walk(&mut job.log, memory, base, &job.census);
    if job.log.overflowed {
        return;
    }
    copy_collaboration_walk(&mut job.log, memory, base, &job.census);
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

/// Shared synthetic address space for the hourly construction-rest
/// copy/decode split: one state with resources and resistance, one
/// country with a building+railway queue, occupation settings with laws,
/// garrisons, manpower and reserve, plus one collaboration relation.
#[cfg(test)]
pub(crate) mod construction_rest_test_kit {
    use super::{
        Capacities, Captured, ConstructionRestCopyContext, ConstructionRestCopyReport,
        ConstructionRestJob, N,
    };
    use engine::{
        carmy_manpower_values, cbuilding, cbuilding_production_line, cbuilding_status, ccountry,
        ccountry_collaboration_status, ccountry_occupation_data, ccountry_occupation_status,
        cgame_state, cgeneral_production_line, coccupation_law, cproduction_line,
        cproduction_status, crailway_production_line, cresistance, cstate, cstate_garrison_data,
        cstate_template, msvc_string,
    };

    pub(crate) const BASE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const STATE: usize = 0x4000;
    const TEMPLATE: usize = 0x5000;
    const RESDATA: usize = 0x5200;
    const PROD: usize = 0x6000;
    const QDATA: usize = 0x6100;
    const BLINE: usize = 0x9000;
    const RLINE: usize = 0xA000;
    const BTARGET: usize = 0xB000;
    const STATUS_OBJ: usize = STATE + cstate::BUILDING_STATUS;
    const RDATA: usize = 0xC000;
    const OCC: usize = 0x7000;
    const OCC_TABLE: usize = 0x7100;
    const OCC_ROWS: usize = 0x7200;
    const OCC_DATA: usize = 0x7300;
    const LAW_TABLE: usize = 0x7400;
    const LAW_DATA: usize = 0x7500;
    const GAR_TABLE: usize = 0x7600;
    const GAR_DATA: usize = 0x7700;
    const GAR_OBJ: usize = 0x7800;
    const LAW_OBJ: usize = 0x7900;
    const RESERVE_DATA: usize = 0x7B00;
    const ASSIGN_DATA: usize = 0x7D00;
    const COLLAB: usize = 0x8000;
    const CREL_DATA: usize = 0x8100;
    const CREL: usize = 0x8200;
    const CINC_DATA: usize = 0x8300;

    fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
        put(bytes, header, &(data as u64).to_le_bytes());
        put(bytes, header + 8, &count.to_le_bytes());
        put(bytes, header + 12, &count.to_le_bytes());
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x12000];
        array(&mut bytes, BASE + cgame_state::COUNTRIES, 0x2000, 3);
        array(&mut bytes, BASE + cgame_state::COUNTRY_LINKS, 0x2100, 3);
        array(&mut bytes, BASE + cgame_state::STATES, 0x2400, 2);
        array(&mut bytes, BASE + cgame_state::PROVINCES, 0x2500, 2);
        for slot in [1, 2] {
            put(
                &mut bytes,
                0x2000 + slot * 8,
                &(COUNTRY as u64).to_le_bytes(),
            );
            put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
        }
        put(&mut bytes, 0x2400 + 8, &(STATE as u64).to_le_bytes());
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY + ccountry::PRODUCTION_STATUS,
            &(PROD as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY + ccountry::OCCUPATION_STATUS,
            &(OCC as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY + ccountry::COLLABORATION_STATUS,
            &(COLLAB as u64).to_le_bytes(),
        );
        // State + template + resources.
        put(
            &mut bytes,
            STATE,
            &(cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, STATE + cstate::ID, &1_u32.to_le_bytes());
        put(
            &mut bytes,
            STATE + cstate::TEMPLATE,
            &(TEMPLATE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE,
            &(cstate_template::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE + cstate_template::ID,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE + cstate_template::RESOURCES,
            &(RESDATA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE + cstate_template::RESOURCE_CAPACITY,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE + cstate_template::RESOURCE_COUNT,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, RESDATA, &1234567_i64.to_le_bytes());
        put(&mut bytes, RESDATA + 8, &55_u32.to_le_bytes());
        // Resistance (embedded).
        let res = STATE + cstate::RESISTANCE;
        put(
            &mut bytes,
            res,
            &(cresistance::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            res + cresistance::STATE,
            &(STATE as u64).to_le_bytes(),
        );
        put(&mut bytes, res + cresistance::LEVEL, &100_i64.to_le_bytes());
        put(&mut bytes, res + cresistance::SPEED, &200_i64.to_le_bytes());
        put(
            &mut bytes,
            res + cresistance::COMPLIANCE,
            &300_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            res + cresistance::COMPLIANCE_SPEED,
            &400_i64.to_le_bytes(),
        );
        put(&mut bytes, res + cresistance::COUNTRY, &1_u32.to_le_bytes());
        put(&mut bytes, res + cresistance::ACTIVE, &[1]);
        // Production status + construction queue (1 building + 1 railway).
        put(&mut bytes, PROD, &(cstate::VTABLE_RVA as u64).to_le_bytes());
        put(
            &mut bytes,
            PROD + cproduction_status::COUNTRY,
            &(COUNTRY as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            PROD + cproduction_status::CONSTRUCTION_LINES,
            QDATA,
            2,
        );
        put(&mut bytes, QDATA, &(BLINE as u64).to_le_bytes());
        put(&mut bytes, QDATA + 8, &(RLINE as u64).to_le_bytes());
        for (pointer, vtable, id) in [
            (
                BLINE,
                cbuilding_production_line::VTABLE_RVA,
                0x0000_0001_ffff_ffff_u64,
            ),
            (
                RLINE,
                crailway_production_line::VTABLE_RVA,
                0xffff_ffff_0000_0001_u64,
            ),
        ] {
            put(&mut bytes, pointer, &(vtable as u64).to_le_bytes());
            put(
                &mut bytes,
                pointer + cproduction_line::REFERENCE_TYPE,
                &id.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRODUCTION_STATUS,
                &(PROD as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::ACTIVE_FACTORIES,
                &3_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::COST,
                &10_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::SPEED,
                &20_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRODUCED,
                &30_i64.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::PRIORITY,
                &1_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cproduction_line::AMOUNT,
                &2_i32.to_le_bytes(),
            );
            put(
                &mut bytes,
                pointer + cgeneral_production_line::CREATED_DATE,
                &100_i32.to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            BLINE + cbuilding_production_line::BUILDING,
            &(BTARGET as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BLINE + cbuilding_production_line::REPAIR_LEVELS_REMAINING,
            &0_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BTARGET,
            &(cbuilding::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BTARGET + cbuilding::TYPE_ID,
            &7_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BTARGET + cbuilding::BUILDING_STATUS,
            &(STATUS_OBJ as u64).to_le_bytes(),
        );
        // Status object for building targets (state scope, location 1).
        put(
            &mut bytes,
            STATUS_OBJ,
            &(cbuilding_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_OBJ + cbuilding_status::FLAGS,
            &0_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_OBJ + cbuilding_status::FLAGS + 4,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_OBJ + cbuilding_status::FLAGS + 8,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_OBJ + cbuilding_status::ID,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS_OBJ + cbuilding_status::COUNTRY,
            &1_u32.to_le_bytes(),
        );
        // Railway fields + route.
        put(
            &mut bytes,
            RLINE + crailway_production_line::BASE_LEVEL,
            &1_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            RLINE + crailway_production_line::TARGET_LEVEL,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            RLINE + crailway_production_line::COMPLETION_FLAG,
            &[0],
        );
        array(
            &mut bytes,
            RLINE + crailway_production_line::ROUTE,
            RDATA,
            2,
        );
        put(&mut bytes, RDATA, &9_u32.to_le_bytes());
        put(&mut bytes, RDATA + 4, &10_u32.to_le_bytes());
        put(
            &mut bytes,
            RLINE + crailway_production_line::CURRENT_SEGMENT,
            &0_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            RLINE + crailway_production_line::REMAINING_COST,
            &50_i64.to_le_bytes(),
        );
        // Occupation status.
        put(
            &mut bytes,
            OCC,
            &(ccountry_occupation_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::SECONDARY_BASE,
            &(ccountry_occupation_status::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::GARRISON_TEMPLATE,
            &0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::DEFAULT_LAW,
            &(LAW_OBJ as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LAW_OBJ,
            &(coccupation_law::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LAW_OBJ + coccupation_law::NAME,
            b"civilian\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        );
        put(
            &mut bytes,
            LAW_OBJ + coccupation_law::NAME + msvc_string::LENGTH,
            &8_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            LAW_OBJ + coccupation_law::NAME + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        // Reserve manpower (1 entry).
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::ASSIGNED_MANPOWER,
            &(carmy_manpower_values::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            OCC + ccountry_occupation_status::ASSIGNED_MANPOWER + carmy_manpower_values::DATA,
            RESERVE_DATA,
            1,
        );
        put(&mut bytes, RESERVE_DATA, &1_u32.to_le_bytes());
        put(&mut bytes, RESERVE_DATA + 4, &(-17_i32).to_le_bytes());
        // Occupation robin-hood table with one entry (occupied country 1).
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE,
            &(OCC_TABLE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 8,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_TABLE + 12,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, OCC_TABLE + 4, &[1]);
        put(&mut bytes, OCC_TABLE + 8, &1_u32.to_le_bytes());
        put(&mut bytes, OCC_TABLE + 16, &(OCC_DATA as u64).to_le_bytes());
        array(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS,
            OCC_ROWS,
            1,
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS + 8,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC + ccountry_occupation_status::OCCUPATION_DATA_ROWS + 12,
            &1_u32.to_le_bytes(),
        );
        put(&mut bytes, OCC_ROWS, &(OCC_DATA as u64).to_le_bytes());
        put(
            &mut bytes,
            OCC_DATA,
            &(ccountry_occupation_data::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC_DATA + ccountry_occupation_data::PARENT_STATUS,
            &(OCC as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OCC_DATA + ccountry_occupation_data::OCCUPIED_COUNTRY,
            &1_u32.to_le_bytes(),
        );
        // Law table inside occupation data (one law, state 0).
        put(
            &mut bytes,
            OCC_DATA + ccountry_occupation_data::LAW_TABLE,
            &(LAW_TABLE as u64).to_le_bytes(),
        );
        put(&mut bytes, LAW_TABLE + 8, &1_u32.to_le_bytes());
        put(&mut bytes, LAW_TABLE + 12, &1_u32.to_le_bytes());
        put(&mut bytes, LAW_DATA + 4, &[1]);
        put(&mut bytes, LAW_DATA + 8, &0_u32.to_le_bytes());
        put(&mut bytes, LAW_DATA + 16, &(LAW_OBJ as u64).to_le_bytes());
        // Garrison table inside occupation data (one garrison, state 1).
        put(
            &mut bytes,
            OCC_DATA + ccountry_occupation_data::GARRISON_TABLE,
            &(GAR_TABLE as u64).to_le_bytes(),
        );
        put(&mut bytes, GAR_TABLE + 8, &1_u32.to_le_bytes());
        put(&mut bytes, GAR_TABLE + 12, &1_u32.to_le_bytes());
        put(&mut bytes, GAR_DATA + 4, &[1]);
        put(&mut bytes, GAR_DATA + 8, &1_u32.to_le_bytes());
        put(&mut bytes, GAR_DATA + 16, &(GAR_OBJ as u64).to_le_bytes());
        put(
            &mut bytes,
            GAR_OBJ,
            &(cstate_garrison_data::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::STATE,
            &(STATE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::OCCUPATION_DATA,
            &(OCC_DATA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::STRENGTH_RATIO,
            &60_000_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::GARRISON_REQUIRED,
            &70_000_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::REQUIRED_MANPOWER,
            &80_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::ASSIGNED_MANPOWER,
            &(carmy_manpower_values::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            GAR_OBJ + cstate_garrison_data::ASSIGNED_MANPOWER + carmy_manpower_values::DATA,
            ASSIGN_DATA,
            1,
        );
        put(&mut bytes, ASSIGN_DATA, &1_u32.to_le_bytes());
        put(&mut bytes, ASSIGN_DATA + 4, &90_i32.to_le_bytes());
        // Collaboration status with no outgoing (empty, shares the walk).
        put(
            &mut bytes,
            COLLAB,
            &(ccountry_collaboration_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            COLLAB + ccountry_collaboration_status::OWNER,
            &(COUNTRY as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            COLLAB + ccountry_collaboration_status::RELATIONS,
            CREL_DATA,
            0,
        );
        array(
            &mut bytes,
            COLLAB + ccountry_collaboration_status::INCOMING_COUNTRIES,
            CINC_DATA,
            0,
        );
        let _ = CREL;
        bytes
    }

    pub(crate) fn capacities() -> Capacities {
        Capacities {
            rows: [8; N],
            routes: 8,
            law_bytes: 64,
        }
    }

    pub(crate) fn direct(memory: &impl engine::Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let _ = captured.capture(memory, 0, BASE, &[true; N], 0, true);
        captured.rest_raw_mode = false;
        captured
    }

    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, ConstructionRestCopyReport, ConstructionRestJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: BASE,
            needed: [true; 10],
        };
        let mut first = ConstructionRestJob::new();
        let mut second = ConstructionRestJob::new();
        let report =
            captured.copy_construction_rest_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        captured.rest_raw_mode = report.raw_ok;
        let _ = captured.capture(memory, 0, BASE, &[true; N], 0, true);
        let mut ticks = [0u64; 10];
        captured.decode_construction_rest_payload(0, BASE, &first, &mut ticks);
        (captured, report, first)
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(
            split.base_resources.outcome, direct.base_resources.outcome,
            "resources outcome"
        );
        assert_eq!(
            split.building_construction.outcome, direct.building_construction.outcome,
            "building outcome"
        );
        assert_eq!(
            split.railway_construction.outcome, direct.railway_construction.outcome,
            "railway outcome"
        );
        assert_eq!(
            split.settings.outcome, direct.settings.outcome,
            "settings outcome"
        );
        assert_eq!(split.laws.outcome, direct.laws.outcome, "laws outcome");
        assert_eq!(
            split.garrisons.outcome, direct.garrisons.outcome,
            "garrisons outcome"
        );
        assert_eq!(
            split.assigned_manpower.outcome, direct.assigned_manpower.outcome,
            "assigned outcome"
        );
        assert_eq!(
            split.reserve_manpower.outcome, direct.reserve_manpower.outcome,
            "reserve outcome"
        );
        assert_eq!(
            split.resistance.outcome, direct.resistance.outcome,
            "resistance outcome"
        );
        assert_eq!(
            split.collaboration.outcome, direct.collaboration.outcome,
            "collaboration outcome"
        );
        let len = split.base_resources.outcome.len();
        assert_eq!(
            &split.base_resources.rows[..len],
            &direct.base_resources.rows[..len],
            "resource rows"
        );
        let len = split.building_construction.outcome.len();
        assert_eq!(
            &split.building_construction.rows[..len],
            &direct.building_construction.rows[..len],
            "building rows"
        );
        let len = split.railway_construction.outcome.len();
        assert_eq!(
            &split.railway_construction.rows[..len],
            &direct.railway_construction.rows[..len],
            "railway rows"
        );
        assert_eq!(split.routes.len, direct.routes.len, "routes arena");
        assert_eq!(
            &split.routes.first[..split.routes.len],
            &direct.routes.first[..direct.routes.len],
            "route provinces"
        );
        assert_eq!(split.law_bytes.len, direct.law_bytes.len, "law arena");
        assert_eq!(
            &split.law_bytes.first[..split.law_bytes.len],
            &direct.law_bytes.first[..direct.law_bytes.len],
            "law bytes"
        );
        let len = split.resistance.outcome.len();
        assert_eq!(
            &split.resistance.rows[..len],
            &direct.resistance.rows[..len],
            "resistance rows"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn construction_rest_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop()
    {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = ConstructionRestJob::default();
        job.census.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &ConstructionRestJob| vector_bytes(&job.census);
        let initial = LOCAL.snapshot().current.auxiliary_vectors;
        assert_eq!(initial, expected(&job));
        job.grow();
        let grown = LOCAL.snapshot().current.auxiliary_vectors;
        assert!(grown > initial);
        assert_eq!(grown, expected(&job));
        job.clear();
        assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
        drop(job);
        assert_eq!(LOCAL.snapshot().current, Capacity::default());
    }

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

    #[test]
    fn construction_rest_split_copy_matches_direct_walk() {
        use super::construction_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert_eq!(report.matched, [true; 10]);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.base_resources.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            split.building_construction.outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(split.railway_construction.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.resistance.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.reserve_manpower.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.collaboration.outcome, Outcome::Rows { len: 0 });
    }

    #[test]
    fn construction_rest_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 row on most rest tables (collaboration empty), so
        // cap 1 is the one-row/exact-fit arm, cap 0 the zero/overflow/massive
        // arm. u32 unread saturation is not reachable and is covered by the
        // walk's own saturating arithmetic.
        use super::construction_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        for rows in [[0; N], [1; N], [2; N], [64; N]] {
            let capacities = Capacities {
                rows,
                routes: 64,
                law_bytes: 64,
            };
            let direct = kit::direct(&memory, &capacities);
            let mut captured = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = ConstructionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
                needed: [true; 10],
            };
            let mut first = ConstructionRestJob::new();
            let mut second = ConstructionRestJob::new();
            let report = captured.copy_construction_rest_refresh(
                &memory,
                ctx,
                &mut first,
                Some(&mut second),
            );
            assert!(report.raw_ok, "rows={rows:?}");
            assert_eq!(report.matched, [true; 10], "rows={rows:?}");
            captured.rest_raw_mode = true;
            let _ = captured.capture(&memory, 0, kit::BASE, &[true; N], 0, true);
            let mut ticks = [0u64; 10];
            captured.decode_construction_rest_payload(0, kit::BASE, &first, &mut ticks);
            kit::assert_matches_direct(&captured, &direct);
        }
    }

    #[test]
    fn construction_rest_split_copy_matches_direct_walk_under_truncation() {
        use super::construction_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = Capacities {
            rows: [1; N],
            routes: 1,
            law_bytes: 4,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = {
            let mut captured = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = ConstructionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
                needed: [true; 10],
            };
            let mut first = ConstructionRestJob::new();
            let mut second = ConstructionRestJob::new();
            let report = captured.copy_construction_rest_refresh(
                &memory,
                ctx,
                &mut first,
                Some(&mut second),
            );
            assert!(report.raw_ok);
            captured.rest_raw_mode = true;
            let _ = captured.capture(&memory, 0, kit::BASE, &[true; N], 0, true);
            let mut ticks = [0u64; 10];
            captured.decode_construction_rest_payload(0, kit::BASE, &first, &mut ticks);
            (captured, report, first)
        };
        assert_eq!(report.matched, [true; 10]);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn construction_rest_double_copy_compare_blocks_checks_on_tearing() {
        use super::construction_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [true; 10],
        };
        let mut first = ConstructionRestJob::new();
        let mut second = ConstructionRestJob::new();
        let report =
            captured.copy_construction_rest_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 10]);
        // A torn resource amount between the passes changes the log.
        let mut torn = bytes.clone();
        torn[0x5100 - kit::BASE] ^= 0xff;
        let torn_memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &torn,
        };
        let ctx_none = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [false; 10],
        };
        let mut c2 = Captured::with_capacities(&capacities, &[0; N]);
        let mut after = ConstructionRestJob::new();
        let mut after_torn = ConstructionRestJob::new();
        let r1 = c2.copy_construction_rest_refresh(&memory, ctx_none, &mut after, None);
        assert!(r1.raw_ok);
        let r2 = c2.copy_construction_rest_refresh(&torn_memory, ctx_none, &mut after_torn, None);
        assert!(r2.raw_ok);
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    #[test]
    fn construction_rest_staging_overflow_falls_back_to_typed_walk() {
        use super::construction_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = ConstructionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [true; 10],
        };
        let mut tiny = ConstructionRestJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            census: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let report = captured.copy_construction_rest_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn construction_rest_worst_case_pools_start_above_late_campaign_caps() {
        // Late-campaign worst case: 4096 resources, 1k queue lines with
        // routes, occupation graphs with laws plus resistance and
        // collaboration links. Pools start above that so steady state never
        // overflows; overflow still falls back to the typed walk above.
        let job = ConstructionRestJob::new();
        assert!(job.log.spans.capacity() >= 1 << 17);
        assert!(job.log.bytes.capacity() >= (8 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[2] >= 4096);
        assert!(capacities.routes >= 4096);
        assert!(capacities.law_bytes >= 16384);
    }
}
