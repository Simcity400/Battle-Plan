//! Construction and occupation acquisition; heap work is confined to buffer growth
//! and replay conversion on the spool thread.
use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, cbuilding, cbuilding_production_line, cbuilding_status, cbuilding_template,
    ccountry_collaboration_data, ccountry_collaboration_status, ccountry_occupation_data,
    ccountry_occupation_status, cgame_state, cpdx_array::Array, cprovince,
    crailway_production_line, cresistance, cstate, cstate_garrison_data, state_resource_entry,
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
    /// Raw-copy split gate for the Tier-4 slot-3 finished-buildings tables
    /// (0..2). Set by the game thread on refresh hours with a valid raw
    /// payload; `capture` then skips the typed buildings walk and the decode
    /// worker fills these rows before the ready handoff. Preserved across
    /// `reset` (set per refresh, cleared by the worker after decode). Hourly
    /// tables are never gated by this flag.
    pub(crate) raw_mode: bool,
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
            raw_mode: false,
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
        self.reset();
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Tier-4 finished-buildings catalog shares one walk; off-hours skip.
        // Raw-copy split: on refresh hours the game thread stages raw bytes
        // via `copy_buildings_refresh` and the decode worker fills the typed
        // rows before the ready handoff, so the typed walk is skipped here.
        // Matched flags and costs come from the raw compare, applied by the
        // caller after this returns. Hourly tables below always run.
        let walk = replay::cadence::should_walk(
            replay::rows::TableId::StateBuildings,
            hour,
            force_full,
        ) && !self.raw_mode;
        self.state_buildings.checked = needed[0] && walk;
        self.province_buildings.checked = needed[1] && walk;
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

// ===========================================================================
// Tier-4 slot-3 raw-copy split (StateBuildings, ProvinceBuildings; the split
// pack is handled per table with independent fault masks over the shared
// walk's read set).
//
// The game thread copies raw bytes only: location roots, pointer pages,
// field gathers, state-authentication rounds and building records. No row is
// decoded on the game thread. The single FIFO decode worker replays the
// existing `cbuilding_status::walk` against the recorded spans
// (`crate::raw::Recorded`), so typed rows, ordinals, skips, truncation and
// failure reasons are identical by construction (see `crate::raw`).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: validation, precedence and error scope
// are local parses, so the replay derives every outcome from the same
// recorded spans and later extras are harmless.
// ===========================================================================

/// Fault-attribution bits, one per buildings table in family order.
const M_STATES: u32 = 0x01;
const M_PROVINCES: u32 = 0x02;
const M_BOTH: u32 = 0x03;

/// Location pointer pages are read in 64-entry batches from slot 1,
/// exactly like the typed cursor.
const LOCATION_BATCH: u32 = 64;

/// One refresh of raw buildings payload. Flat pools recycle capacities and
/// the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_buildings_job`).
#[derive(Debug)]
pub(crate) struct BuildingsJob {
    pub log: RawLog<2>,
    pub game_state: usize,
}

impl BuildingsJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 3k state plus 5.2k province
            // rows with small records; both pools start above that so steady
            // state never overflows.
            log: RawLog::new(1 << 18, 16 << 20),
            game_state: 0,
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
        self.game_state = 0;
    }

    pub fn grow_for_next(&self) -> bool {
        self.log.grow_for_next()
    }

    /// Double every pool off the game thread until copies fit. Converges like
    /// the typed capacity growth; overflowed refreshes fall back to the typed
    /// walk in the meantime, so sizing is a pure performance knob.
    pub fn grow(&mut self) {
        self.log.grow();
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BuildingsCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [states, provinces] order.
    pub matched: [bool; 2],
    pub copy_ticks: [u64; 2],
    pub raw_bytes: [u64; 2],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct BuildingsCopyContext {
    pub game_state: usize,
    pub needed: [bool; 2],
}

fn pool_u64(log: &RawLog<2>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn bspan(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Destination offsets of the seven field spans inside one 72-byte record:
/// vtable, id, state link, status pointer, flags, buildings array, indices
/// array.
fn record_offset(index: usize) -> usize {
    match index {
        0 => 0,
        1 => 8,
        2 => 12,
        3 => 20,
        4 => 28,
        5 => 40,
        _ => 56,
    }
}

/// Mirror of one `walk_locations` pass: roots, pointer pages, field gathers,
/// state-authentication rounds and building records. Takes are unconditional
/// over discoverable addresses (a superset); the replay derives precedence
/// and outcomes.
fn copy_buildings_walk(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    game_state: usize,
    province_scope: bool,
    mask: u32,
) {
    let states_start = bspan(log, memory, game_state.checked_add(cgame_state::STATES), 16, M_BOTH);
    let Some(states_start) = states_start else {
        return;
    };
    let mut states_raw = [0u8; 16];
    states_raw.copy_from_slice(&log.bytes[states_start..states_start + 16]);
    let states = match Array::from_header(states_raw, 8) {
        Some(states) => states,
        None => return,
    };
    let locations = if province_scope {
        let provinces_start = bspan(
            log,
            memory,
            game_state.checked_add(cgame_state::PROVINCES),
            16,
            M_PROVINCES,
        );
        let Some(provinces_start) = provinces_start else {
            return;
        };
        let mut provinces_raw = [0u8; 16];
        provinces_raw.copy_from_slice(&log.bytes[provinces_start..provinces_start + 16]);
        match Array::from_header(provinces_raw, 8) {
            Some(locations) => locations,
            None => return,
        }
    } else {
        states
    };
    let (id_offset, status_offset, with_state_link) = if province_scope {
        (cprovince::ID, cprovince::BUILDING_STATUS, true)
    } else {
        (cstate::ID, cstate::BUILDING_STATUS, false)
    };
    let mut first = 1u32;
    while first < locations.count {
        let count = (locations.count - first).min(LOCATION_BATCH);
        // Pointer takes in walk order, one 8-byte span per slot exactly
        // like the typed scatter. The first failure stops the batch; the
        // successfully-taken prefix is still fully processed below before
        // the walk aborts, mirroring the typed visit-then-error order.
        let mut page = [0u8; 64 * 8];
        let mut taken = 0u32;
        let mut page_failed = false;
        for slot in 0..count {
            let addr = locations.element(first + slot, 8);
            match addr.and_then(|addr| bspan(log, memory, Some(addr), 8, mask)) {
                Some(start) => {
                    page[slot as usize * 8..slot as usize * 8 + 8]
                        .copy_from_slice(&log.bytes[start..start + 8]);
                    taken += 1;
                }
                None => {
                    page_failed = true;
                    break;
                }
            }
            if log.overflowed {
                return;
            }
        }
        // Live entries parsed from the taken prefix.
        let mut live: [(usize, u32); 64] = [(0, 0); 64];
        let mut live_len = 0usize;
        for slot in 0..taken as usize {
            let pointer = usize::try_from(u64::from_le_bytes(
                page[slot * 8..slot * 8 + 8].try_into().unwrap(),
            ))
            .ok()
            .filter(|&pointer| pointer != 0);
            if let Some(pointer) = pointer {
                live[live_len] = (pointer, first + slot as u32);
                live_len += 1;
            }
        }
        // Field gathers in row-major order (seven spans per live entry).
        // The state-link span is empty for the states walk, matching the
        // layout's zero span without touching memory.
        let mut fields: [([u8; 72], bool); 64] = [([0; 72], false); 64];
        for (slot, (pointer, _)) in live[..live_len].iter().enumerate() {
            let mut record = [0u8; 72];
            let mut ok = true;
            let spans = [
                (*pointer, 8),
                (pointer.checked_add(id_offset).unwrap_or(0), 4),
                (
                    pointer.checked_add(cprovince::STATE).unwrap_or(0),
                    if with_state_link { 8 } else { 0 },
                ),
                (pointer.checked_add(status_offset).unwrap_or(0), 8),
                (
                    pointer.checked_add(status_offset + cbuilding_status::FLAGS).unwrap_or(0),
                    12,
                ),
                (
                    pointer.checked_add(status_offset + cbuilding_status::BUILDINGS).unwrap_or(0),
                    16,
                ),
                (
                    pointer.checked_add(status_offset + cbuilding_status::BUILDING_INDICES)
                        .unwrap_or(0),
                    16,
                ),
            ];
            for (index, (addr, len)) in spans.iter().enumerate() {
                if *len == 0 {
                    continue;
                }
                match bspan(log, memory, Some(*addr), *len, mask) {
                    Some(start) => {
                        record[record_offset(index)..record_offset(index) + len]
                            .copy_from_slice(&log.bytes[start..start + len]);
                    }
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            fields[slot] = (record, ok);
            if log.overflowed {
                return;
            }
        }
        // State-authentication rounds for the province walk, staged from the
        // recorded state links exactly like the typed gather.
        if with_state_link {
            copy_state_identities(log, memory, &states, &live[..live_len], &fields, mask);
            if log.overflowed {
                return;
            }
        }
        // Building records for every live entry.
        for (record, _) in fields.iter().take(live_len) {
            copy_location_buildings(log, memory, record, mask);
            if log.overflowed {
                return;
            }
        }
        // A failed pointer page aborts the walk after its prefix, exactly
        // like the typed visit-then-error order.
        if page_failed {
            return;
        }
        first += count;
    }
}

/// Mirror of the state-identity gather: identity round plus table-slot round
/// for staged nonzero state links. Takes are unconditional; the replay
/// derives the completed prefixes.
fn copy_state_identities(
    log: &mut RawLog<2>,
    memory: &impl Memory,
    states: &Array,
    live: &[(usize, u32)],
    fields: &[([u8; 72], bool); 64],
    mask: u32,
) {
    let mut pointers = [0usize; 64];
    let mut staged = 0usize;
    for (slot, _) in live.iter().enumerate() {
        let (record, ok) = &fields[slot];
        if !ok {
            continue;
        }
        let pointer = usize::try_from(u64::from_le_bytes(record[12..20].try_into().unwrap()))
            .ok()
            .filter(|&pointer| pointer != 0);
        if let Some(pointer) = pointer {
            pointers[staged] = pointer;
            staged += 1;
        }
    }
    if staged == 0 {
        return;
    }
    // Round 1: vtable plus id per staged state.
    let mut identities = [([0u8; 12], false); 64];
    for (slot, pointer) in pointers[..staged].iter().enumerate() {
        let vtable = bspan(log, memory, Some(*pointer), 8, mask);
        let id = bspan(log, memory, pointer.checked_add(cstate::ID), 4, mask);
        match (vtable, id) {
            (Some(vtable), Some(id)) => {
                let mut identity = [0u8; 12];
                identity[..8].copy_from_slice(&log.bytes[vtable..vtable + 8]);
                identity[8..].copy_from_slice(&log.bytes[id..id + 4]);
                identities[slot] = (identity, true);
            }
            _ => {
                identities[slot] = ([0u8; 12], false);
            }
        }
        if log.overflowed {
            return;
        }
    }
    // Round 2: state-table slot each fully-read id names. Null table slots
    // read nothing, matching copy_records' stop rule.
    for (identity, ok) in identities[..staged].iter() {
        if !ok {
            continue;
        }
        let id = u32::from_le_bytes(identity[8..12].try_into().unwrap());
        let table_addr = states.element(id, 8);
        if table_addr.is_some_and(|addr| addr != 0) {
            let _ = bspan(log, memory, table_addr, 8, mask);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of one location's building reads: pointer vector, field gathers,
/// template fields and index entries per building. All takes are
/// unconditional over discoverable addresses.
fn copy_location_buildings(log: &mut RawLog<2>, memory: &impl Memory, record: &[u8; 72], mask: u32) {
    let Ok(buildings_data) = usize::try_from(u64::from_le_bytes(record[40..48].try_into().unwrap()))
    else {
        return;
    };
    let buildings_count = u32::from_le_bytes(record[48..52].try_into().unwrap());
    let Ok(indices_data) = usize::try_from(u64::from_le_bytes(record[56..64].try_into().unwrap()))
    else {
        return;
    };
    let indices_count = u32::from_le_bytes(record[68..72].try_into().unwrap());
    for ordinal in 0..buildings_count as usize {
        let addr = (ordinal.checked_mul(8)).and_then(|offset| buildings_data.checked_add(offset));
        let pointer = addr
            .and_then(|addr| bspan(log, memory, Some(addr), 8, mask))
            .and_then(|start| usize::try_from(pool_u64(log, start)).ok())
            .filter(|&pointer| pointer != 0);
        let Some(pointer) = pointer else {
            continue;
        };
        // Field gather: vtable, type, level block, health, backlink.
        let field_takes = [
            (pointer, 8),
            (pointer.checked_add(cbuilding::TYPE_ID).unwrap_or(0), 4),
            (pointer.checked_add(cbuilding::LEVEL).unwrap_or(0), 4),
            (pointer.checked_add(cbuilding::PARTIAL_HEALTH).unwrap_or(0), 8),
            (pointer.checked_add(cbuilding::BUILDING_STATUS).unwrap_or(0), 16),
        ];
        let mut field_starts = [0usize; 5];
        let mut fields_ok = true;
        for (slot, (addr, len)) in field_takes.iter().enumerate() {
            match bspan(log, memory, Some(*addr), *len, mask) {
                Some(start) => field_starts[slot] = start,
                None => {
                    fields_ok = false;
                    break;
                }
            }
        }
        if !fields_ok {
            continue;
        }
        // Template fields and index entry, discovered from recorded bytes
        // with the walk's own bounds checks.
        let template = usize::try_from(pool_u64(log, field_starts[4] + 8))
            .ok()
            .filter(|&template| template != 0);
        if let Some(template) = template {
            let type_id = bspan(
                log,
                memory,
                template.checked_add(cbuilding_template::TYPE_ID),
                4,
                mask,
            );
            let ordinal_take = bspan(
                log,
                memory,
                template.checked_add(cbuilding_template::DATABASE_ORDINAL),
                4,
                mask,
            );
            if let Some(ordinal_start) = ordinal_take {
                let ordinal = u32::from_le_bytes(
                    log.bytes[ordinal_start..ordinal_start + 4].try_into().unwrap(),
                );
                let addr = (ordinal < indices_count)
                    .then(|| (ordinal as usize).checked_mul(4))
                    .flatten()
                    .and_then(|offset| indices_data.checked_add(offset));
                if let Some(addr) = addr {
                    let _ = bspan(log, memory, Some(addr), 4, mask);
                }
            }
            let _ = type_id;
        }
        if log.overflowed {
            return;
        }
    }
}

impl Captured {
    /// Game-thread raw copy for a slot-3 refresh hour. Copies `first` always
    /// and `second` while either buildings table still needs double-read
    /// checks, compares the raw logs, and records copy-only costs plus the
    /// existing checked/mismatches bookkeeping. Returns whether the payload
    /// is complete (`raw_ok`); on staging overflow the caller falls back to
    /// the typed walk and enqueues nothing.
    pub(crate) fn copy_buildings_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: BuildingsCopyContext,
        first: &mut BuildingsJob,
        second: Option<&mut BuildingsJob>,
    ) -> BuildingsCopyReport {
        let started = qpc();
        first.game_state = ctx.game_state;
        copy_buildings_walk(&mut first.log, memory, ctx.game_state, false, M_STATES);
        copy_buildings_walk(&mut first.log, memory, ctx.game_state, true, M_PROVINCES);
        let mut ticks = [0u64; 2];
        ticks[0] = elapsed(started);
        let mut matched = [true; 2];
        if let Some(second_job) = second {
            let second_started = qpc();
            copy_buildings_walk(&mut second_job.log, memory, ctx.game_state, false, M_STATES);
            copy_buildings_walk(&mut second_job.log, memory, ctx.game_state, true, M_PROVINCES);
            ticks[0] += elapsed(second_started);
            let equal =
                first.log.logs_equal(&second_job.log) && !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        self.state_buildings.checked = ctx.needed[0];
        self.state_buildings.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.province_buildings.checked = ctx.needed[1];
        self.province_buildings.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        let raw_ok = !first.log.overflowed;
        BuildingsCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes match the typed walk exactly because the
    /// identical walk runs against the recorded spans; checked/mismatches
    /// set by the raw compare survive. Clears `raw_mode` so the next capture
    /// starts clean.
    pub(crate) fn decode_buildings_payload(
        &mut self,
        base: usize,
        job: &BuildingsJob,
        decode_ticks: &mut [u64; 2],
    ) {
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let walked = cbuilding_status::walk(
            &recorded,
            base,
            job.game_state,
            &mut self.state_buildings.rows,
            &mut self.province_buildings.rows,
        );
        self.state_buildings.outcome = walked.states;
        self.province_buildings.outcome = walked.provinces;
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
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

/// Shared synthetic address space for the buildings copy/decode split
/// harness: one state and two provinces (one with a state link) each with
/// buildings, plus empty and fault-injection coverage.
#[cfg(test)]
pub(crate) mod buildings_test_kit {
    use super::{BuildingsCopyContext, BuildingsCopyReport, BuildingsJob, Capacities, Captured, N};
    use engine::{
        cbuilding, cbuilding_status, cbuilding_template, cgame_state, cprovince, cstate,
    };

    pub(crate) const BASE: usize = 0x140000000;
    pub(crate) const GAME_STATE: usize = 0x10000;
    const ADDRESS: usize = 0x10000;

    const SARR: usize = 0x2000;
    const PARR: usize = 0x2100;
    pub(crate) const ST1: usize = 0x3000;
    pub(crate) const PR1: usize = 0x3400;
    pub(crate) const PR2: usize = 0x3800;
    // Inline status objects live inside their locations.
    const SST1: usize = 0x3000 + 0x120;
    const SPR1: usize = 0x3400 + 0x190;
    const SPR2: usize = 0x3800 + 0x190;
    const SB1: usize = 0x5000;
    const PB1: usize = 0x5200;
    const TM1: usize = 0x6000;
    const TM2: usize = 0x6100;
    const IDX1: usize = 0x6300;
    const IDX2: usize = 0x6400;
    const SBARR1: usize = 0x6700;
    const PBARR1: usize = 0x6710;

    pub(crate) fn memory(bytes: &[u8]) -> engine::memory::SliceMemory<'_> {
        engine::memory::SliceMemory { base: ADDRESS, bytes }
    }

    fn put<const COUNT: usize>(bytes: &mut [u8], offset: usize, value: [u8; COUNT]) {
        bytes[offset..offset + COUNT].copy_from_slice(&value);
    }

    fn pointer(bytes: &mut [u8], offset: usize, target: usize) {
        put(bytes, offset, ((ADDRESS + target) as u64).to_le_bytes());
    }

    fn array(bytes: &mut [u8], offset: usize, data: usize, count: u32) {
        pointer(bytes, offset, data);
        put(bytes, offset + 8, count.to_le_bytes());
        put(bytes, offset + 12, count.to_le_bytes());
    }

    fn status(
        bytes: &mut [u8],
        object: usize,
        location: u32,
        province_scope: bool,
        building: usize,
        indices: usize,
    ) {
        put(
            bytes,
            object,
            ((BASE + cbuilding_status::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(
            bytes,
            object + cbuilding_status::FLAGS,
            (if province_scope { 1_u32 } else { 0_u32 }).to_le_bytes(),
        );
        put(
            bytes,
            object + cbuilding_status::FLAGS + 4,
            location.to_le_bytes(),
        );
        put(
            bytes,
            object + cbuilding_status::FLAGS + 8,
            1_u32.to_le_bytes(),
        );
        array(bytes, object + cbuilding_status::BUILDINGS, building, 1);
        array(bytes, object + cbuilding_status::BUILDING_INDICES, indices, 1);
    }

    fn building(bytes: &mut [u8], object: usize, status: usize, ordinal_type: u32, template: usize) {
        put(
            bytes,
            object,
            ((BASE + cbuilding::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(bytes, object + cbuilding::TYPE_ID, ordinal_type.to_le_bytes());
        put(bytes, object + cbuilding::LEVEL, 2_i16.to_le_bytes());
        put(bytes, object + cbuilding::HEALTHY_LEVEL, 2_i16.to_le_bytes());
        put(
            bytes,
            object + cbuilding::PARTIAL_HEALTH,
            5_000_000_i64.to_le_bytes(),
        );
        pointer(bytes, object + cbuilding::BUILDING_STATUS, status);
        put(bytes, object + cbuilding::BUILDING_STATUS + 8, 0_u64.to_le_bytes());
        pointer(bytes, object + cbuilding::TEMPLATE, template);
        put(
            bytes,
            template,
            ((BASE + cbuilding::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(bytes, template + cbuilding_template::TYPE_ID, ordinal_type.to_le_bytes());
        // Database ordinal doubles as the location index-vector position in
        // the walk, so it must name a present entry holding this ordinal.
        put(bytes, template + cbuilding_template::DATABASE_ORDINAL, 0_u32.to_le_bytes());
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x7000];
        // Game-state object at the image base; roots at raw offsets.
        array(&mut bytes, cgame_state::STATES, SARR, 2);
        array(&mut bytes, cgame_state::PROVINCES, PARR, 3);
        pointer(&mut bytes, SARR + 8, ST1);
        pointer(&mut bytes, PARR + 8, PR1);
        pointer(&mut bytes, PARR + 16, PR2);
        // State slot 1 with id 1 (ids index the state table). The status
        // object is inline at +0x120, not a pointer.
        put(
            &mut bytes,
            ST1,
            ((BASE + cstate::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, ST1 + cstate::ID, 1_u32.to_le_bytes());
        status(&mut bytes, SST1, 1, false, SBARR1, IDX1);
        // Province slot 1 linked to ST1, status inline at +0x190.
        put(
            &mut bytes,
            PR1,
            ((BASE + cprovince::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, PR1 + cprovince::ID, 1_u32.to_le_bytes());
        pointer(&mut bytes, PR1 + cprovince::STATE, ST1);
        status(&mut bytes, SPR1, 1, true, PBARR1, IDX2);
        // Province slot 2 with a null state link (sea province) and no
        // buildings; its null state pointer reads as state 0.
        put(
            &mut bytes,
            PR2,
            ((BASE + cprovince::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, PR2 + cprovince::ID, 2_u32.to_le_bytes());
        status(&mut bytes, SPR2, 2, true, 0x6500, 0x6600);
        array(&mut bytes, SPR2 + cbuilding_status::BUILDINGS, 0x6500, 0);
        building(&mut bytes, SB1, SST1, 7, TM1);
        building(&mut bytes, PB1, SPR1, 9, TM2);
        // Building pointer vectors and index entries.
        pointer(&mut bytes, SBARR1, SB1);
        pointer(&mut bytes, PBARR1, PB1);
        // Index entries map native ordinals to themselves.
        put(&mut bytes, IDX1, 0_u32.to_le_bytes());
        put(&mut bytes, IDX2, 0_u32.to_le_bytes());
        bytes
    }

    /// Reference typed walk with identical pooled buffers.
    pub(crate) fn direct(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let walked = cbuilding_status::walk(
            memory,
            BASE,
            GAME_STATE,
            &mut captured.state_buildings.rows,
            &mut captured.province_buildings.rows,
        );
        captured.state_buildings.outcome = walked.states;
        captured.province_buildings.outcome = walked.provinces;
        captured
    }

    /// Raw-copy split: two raw passes compared, then the first payload
    /// decoded into fresh pooled buffers of the same capacities.
    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, BuildingsCopyReport, BuildingsJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = BuildingsCopyContext {
            game_state: GAME_STATE,
            needed: [true, true],
        };
        let mut first = BuildingsJob::new();
        let mut second = BuildingsJob::new();
        let report = captured.copy_buildings_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 2];
        captured.decode_buildings_payload(BASE, &first, &mut ticks);
        (captured, report, first)
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(
            split.state_buildings.outcome, direct.state_buildings.outcome,
            "states outcome"
        );
        assert_eq!(
            split.province_buildings.outcome, direct.province_buildings.outcome,
            "provinces outcome"
        );
        let states_len = split.state_buildings.outcome.len();
        assert_eq!(direct.state_buildings.outcome.len(), states_len, "states length");
        assert_eq!(
            &split.state_buildings.rows[..states_len],
            &direct.state_buildings.rows[..states_len],
            "state rows"
        );
        let provinces_len = split.province_buildings.outcome.len();
        assert_eq!(
            direct.province_buildings.outcome.len(),
            provinces_len,
            "provinces length"
        );
        assert_eq!(
            &split.province_buildings.rows[..provinces_len],
            &direct.province_buildings.rows[..provinces_len],
            "province rows"
        );
    }
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

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [256; N],
            routes: 1024,
            law_bytes: 1024,
        }
    }

    #[test]
    fn split_copy_matches_direct_walk() {
        use super::buildings_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: one state row and one province row (the sea province
        // holds no buildings and contributes no rows).
        assert_eq!(split.state_buildings.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.state_buildings.rows[0].building_type, 7);
        assert_eq!(split.state_buildings.rows[0].location, 1);
        assert_eq!(split.province_buildings.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.province_buildings.rows[0].building_type, 9);
        assert_eq!(split.province_buildings.rows[0].location, 1);
    }

    #[test]
    fn split_copy_matches_direct_walk_under_truncation() {
        use super::buildings_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities {
            rows: [1; N],
            routes: 0,
            law_bytes: 0,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn split_copy_matches_direct_walk_on_fault_matrix() {
        use super::buildings_test_kit as kit;
        use engine::memory::SliceMemory;
        struct Fault<'a> {
            memory: SliceMemory<'a>,
            fault: std::ops::Range<usize>,
        }
        impl engine::Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let Some(end) = address.checked_add(output.len()) else {
                    return false;
                };
                if address < self.fault.end && end > self.fault.start {
                    return false;
                }
                self.memory.copy(address, output)
            }
        }
        // (fault offset from the image base, expected raw-compare verdicts in
        // family order). Outcomes including reason strings must match the
        // typed walk exactly; verdicts follow the fault-attribution masks,
        // with the shared state root blocking both tables.
        for (offset, matched) in [
            (cgame_state::STATES, [false, false]),
            (cgame_state::PROVINCES, [true, false]),
            (kit::ST1, [false, false]),
            (kit::PR1, [true, false]),
        ] {
            let bytes = kit::fixture();
            let fault = kit::GAME_STATE + offset;
            let memory = Fault {
                memory: SliceMemory {
                    base: kit::GAME_STATE,
                    bytes: &bytes,
                },
                fault: fault..fault + 1,
            };
            let capacities = split_capacities();
            let direct = kit::direct(&memory, &capacities);
            let (split, report, _) = kit::split(&memory, &capacities);
            assert!(report.raw_ok, "offset={offset:x}");
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(report.matched, matched, "offset={offset:x}");
        }
    }

    #[test]
    fn double_copy_compare_blocks_checks_on_tearing() {
        use super::buildings_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = BuildingsCopyContext {
            game_state: kit::GAME_STATE,
            needed: [true, true],
        };
        let mut first = BuildingsJob::new();
        let mut second = BuildingsJob::new();
        let report =
            captured.copy_buildings_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 2]);
        // Same-hour tearing between the passes changes the log.
        let mut torn = bytes.clone();
        torn[kit::ST1 + cstate::ID] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let ctx = BuildingsCopyContext {
            game_state: kit::GAME_STATE,
            needed: [false, false],
        };
        let mut scratch = Captured::with_capacities(&capacities, &[0; N]);
        let mut after = BuildingsJob::new();
        let after_report =
            scratch.copy_buildings_refresh(&memory, ctx, &mut after, None);
        assert!(after_report.raw_ok);
        let mut after_torn = BuildingsJob::new();
        let torn_report =
            scratch.copy_buildings_refresh(&torn_memory, ctx, &mut after_torn, None);
        assert!(torn_report.raw_ok);
        assert!(!after.log.logs_equal(&after_torn.log));
        let mut repeat = BuildingsJob::new();
        let repeat_report =
            scratch.copy_buildings_refresh(&memory, ctx, &mut repeat, None);
        assert!(repeat_report.raw_ok);
        assert!(after.log.logs_equal(&repeat.log));
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::buildings_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = BuildingsCopyContext {
            game_state: kit::GAME_STATE,
            needed: [true, true],
        };
        let mut tiny = BuildingsJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
        };
        let report = captured.copy_buildings_refresh(&memory, ctx, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }
}
