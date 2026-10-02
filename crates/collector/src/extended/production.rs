//! Production/equipment acquisition. Only construction/growth and conversion
//! allocate; worker-owned variants grow during their original acquisition.

use crate::{
    projection::{Projection, scalar_table},
    raw::RawLog,
    read_accounting,
    sampling::qpc,
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, army_upgrade_request_row, carmy_manpower_values, carmy_upgrades_requests,
    carmy_upgrades_status, ccountry, cdeployment_status, cequipment_production_license,
    cequipment_production_line, cequipment_upgrades_instance, cequipment_variant,
    cequipment_variant_pool, cgame_state, cmilitary_deployment, cmilitary_deployment_conveyor,
    cmilitary_deployment_line, cname_group_member, cpdx_array::Array, cproduction_line,
    cproduction_status, cupgrade_delivery, equipment_entry, equipment_module_container,
    manpower_value_entry, msvc_string, walk::Outcome,
};
use replay::{
    frame::{TableInput, TableView},
    rows::{self, TableId, tables},
};

pub const N: usize = 17;
pub const TABLES: [TableId; N] = [
    TableId::ProductionLines,
    TableId::Stockpiles,
    TableId::DivisionEquipment,
    TableId::VariantDefinitions,
    TableId::VariantModules,
    TableId::VariantStats,
    TableId::VariantUpgrades,
    TableId::ArmyUpgradeGroups,
    TableId::ArmyUpgradeRequests,
    TableId::ArmyUpgradeDeliveries,
    TableId::ArmyUpgradeProduced,
    TableId::ProductionLicenses,
    TableId::DeploymentConveyors,
    TableId::DeploymentLines,
    TableId::DeploymentManpower,
    TableId::DeploymentEquipment,
    TableId::ProductionLicenseRecords,
];

/// Name, local cost slot, and local member indices. Zero cost in a non-anchor
/// slot means the work is accounted for by this group, not that reads are free.
pub const READ_GROUPS: &[(&str, usize, &[usize])] = &[
    ("variant modules, stats and upgrades", 4, &[4, 5, 6]),
    (
        "army upgrade groups, requests, deliveries and produced equipment",
        7,
        &[7, 8, 9, 10],
    ),
    (
        "deployment conveyors, lines, manpower and in-transit equipment",
        12,
        &[12, 13, 14, 15],
    ),
    ("production licences and licence records", 11, &[11, 16]),
];

/// Borrowed current census owned by shared acquisition, in native source order.
pub struct Sources<'a, T> {
    pub rows: &'a [T],
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub definition_bytes: usize,
    pub conveyor_name_bytes: usize,
    pub line_custom_name_bytes: usize,
    pub license_handles: usize,
}

impl Default for Capacities {
    fn default() -> Self {
        Self {
            rows: [
                2048, 8192, 32768, 4096, 8192, 4096, 8192, 2048, 8192, 2048, 8192, 2048, 2048,
                8192, 16384, 32768, 1024,
            ],
            definition_bytes: 128 * 1024,
            conveyor_name_bytes: 128 * 1024,
            line_custom_name_bytes: 16 * 1024,
            license_handles: 4096,
        }
    }
}

/// Variant leaves loaned to the worker-only acquisition job.
pub(crate) struct Variants {
    definitions: CaptureRows<cequipment_variant::DefinitionRow>,
    modules: CaptureRows<equipment_module_container::Row>,
    stats: CaptureRows<cequipment_variant::StatsRow>,
    upgrades: CaptureRows<cequipment_upgrades_instance::Row>,
    definition_bytes: Arena<u8>,
}

impl Variants {
    fn with_capacities(capacities: &Capacities) -> Self {
        Self {
            definitions: CaptureRows::new(capacities.rows[3]),
            modules: CaptureRows::new(capacities.rows[4]),
            stats: CaptureRows::new(capacities.rows[5]),
            upgrades: CaptureRows::new(capacities.rows[6]),
            definition_bytes: Arena::new(capacities.definition_bytes),
        }
    }

    pub(crate) fn vacant() -> Self {
        Self {
            definitions: CaptureRows::new(0),
            modules: CaptureRows::new(0),
            stats: CaptureRows::new(0),
            upgrades: CaptureRows::new(0),
            definition_bytes: Arena::new(0),
        }
    }

    fn reset(&mut self) {
        self.definitions.reset();
        self.modules.reset();
        self.stats.reset();
        self.upgrades.reset();
        self.definition_bytes.reset();
    }

    pub(crate) fn allocated_bytes(&self) -> usize {
        self.definitions.allocated_bytes()
            + self.modules.allocated_bytes()
            + self.stats.allocated_bytes()
            + self.upgrades.allocated_bytes()
            + self.definition_bytes.allocated_bytes()
    }

    fn grow(&mut self, capacities: &mut Capacities) {
        capacities.rows[3] = capacities.rows[3].max(self.definitions.rows.capacity());
        self.definitions.grow(&mut capacities.rows[3]);
        capacities.rows[4] = capacities.rows[4].max(self.modules.rows.capacity());
        self.modules.grow(&mut capacities.rows[4]);
        capacities.rows[5] = capacities.rows[5].max(self.stats.rows.capacity());
        self.stats.grow(&mut capacities.rows[5]);
        capacities.rows[6] = capacities.rows[6].max(self.upgrades.rows.capacity());
        self.upgrades.grow(&mut capacities.rows[6]);
        capacities.definition_bytes = capacities
            .definition_bytes
            .max(self.definition_bytes.first.capacity());
        self.definition_bytes.grow(&mut capacities.definition_bytes);
    }

    pub(crate) fn fail(&mut self, reason: &'static str, hour: i32, force_full: bool) {
        self.reset();
        for (outcome, table) in [
            (&mut self.definitions.outcome, TABLES[3]),
            (&mut self.modules.outcome, TABLES[4]),
            (&mut self.stats.outcome, TABLES[5]),
            (&mut self.upgrades.outcome, TABLES[6]),
        ] {
            *outcome = Outcome::Unavailable {
                reason: if replay::cadence::should_walk(table, hour, force_full) {
                    reason
                } else {
                    replay::cadence::SKIPPED_REASON
                },
            };
        }
    }

    /// Read variant fields from the current complete registry census.
    pub(crate) fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        variants: Sources<'_, cequipment_variant::RegistryRow>,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let mut ticks = [0; N];
        let mut reads = read_accounting::table(TABLES[3]);
        let begin = qpc();
        let source = complete_source(variants.rows, variants.outcome);
        capture_text(
            &mut self.definitions,
            &mut self.definition_bytes,
            source,
            |source, rows, arena| {
                cequipment_variant::walk_definitions(memory, base, source, rows, arena)
            },
        );
        ticks[3] = elapsed(begin);
        if !replay::cadence::should_walk(TableId::VariantModules, hour, force_full) {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.modules.outcome = skipped;
            self.stats.outcome = skipped;
            self.upgrades.outcome = skipped;
            return ticks;
        }
        reads.select(TABLES[4]);
        let begin = qpc();
        capture_source(&mut self.modules, source, |source, rows| {
            equipment_module_container::walk(memory, base, source, rows)
        });
        capture_source(&mut self.stats, source, |source, rows| {
            cequipment_variant::walk_stats(memory, source, rows)
        });
        capture_source(&mut self.upgrades, source, |source, rows| {
            cequipment_upgrades_instance::walk(memory, base, source, rows)
        });
        ticks[4] = elapsed(begin);
        ticks
    }
}

pub struct Captured {
    variants: Variants,
    pub lines: CaptureRows<cequipment_production_line::Row>,
    pub stockpiles: CaptureRows<cequipment_variant_pool::StockpileRow>,
    pub division_equipment: CaptureRows<cequipment_variant_pool::DivisionRow>,
    pub groups: CaptureRows<carmy_upgrades_requests::Row>,
    pub requests: CaptureRows<army_upgrade_request_row::Row>,
    pub deliveries: CaptureRows<cupgrade_delivery::Row>,
    pub produced: CaptureRows<cupgrade_delivery::ProducedRow>,
    pub licenses: CaptureRows<cproduction_status::LicenseRow>,
    pub conveyors: CaptureRows<cdeployment_status::ConveyorRow>,
    pub deployment_lines: CaptureRows<cdeployment_status::LineRow>,
    pub deployment_manpower: CaptureRows<cdeployment_status::ManpowerRow>,
    pub deployment_equipment: CaptureRows<cdeployment_status::EquipmentRow>,
    pub conveyor_name_bytes: Arena<u8>,
    pub line_custom_name_bytes: Arena<u8>,
    pub license_records: CaptureRows<cproduction_status::LicenseRecordRow>,
    pub license_handles: Arena<u64>,
    /// Raw-copy split gate for scheduled division equipment and upgrade details.
    /// Set when an acquisition job returns a valid raw payload;
    /// `acquire` then skips those typed detail
    /// walks and the decode worker fills the rows before the ready handoff.
    /// Preserved across `reset` and cleared by the worker after decode.
    /// Production and deployment use the independent rest payload below.
    pub(crate) divisions_raw_mode: bool,
    /// Raw-copy split gate for scheduled production rest (lines 0,
    /// stockpiles 1, licences 11, deployment 12..15, licence records 16).
    /// Set when an acquisition job returns a valid raw payload; `acquire`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus the licence/name arenas) before the ready handoff. Preserved
    /// across `reset` (set per hour, cleared by the worker after decode).
    pub(crate) rest_raw_mode: bool,
    /// The acquisition's selected detail activity survives through decode failures.
    division_details_active: bool,
}

impl Captured {
    pub(crate) fn take_variants(&mut self) -> Variants {
        std::mem::replace(&mut self.variants, Variants::vacant())
    }

    pub(crate) fn restore_variants(&mut self, variants: Variants) {
        self.variants = variants;
    }

    pub fn with_capacities(capacities: &Capacities) -> Self {
        Self {
            variants: Variants::with_capacities(capacities),
            lines: CaptureRows::new(capacities.rows[0]),
            stockpiles: CaptureRows::new(capacities.rows[1]),
            division_equipment: CaptureRows::new(capacities.rows[2]),
            groups: CaptureRows::new(capacities.rows[7]),
            requests: CaptureRows::new(capacities.rows[8]),
            deliveries: CaptureRows::new(capacities.rows[9]),
            produced: CaptureRows::new(capacities.rows[10]),
            licenses: CaptureRows::new(capacities.rows[11]),
            conveyors: CaptureRows::new(capacities.rows[12]),
            deployment_lines: CaptureRows::new(capacities.rows[13]),
            deployment_manpower: CaptureRows::new(capacities.rows[14]),
            deployment_equipment: CaptureRows::new(capacities.rows[15]),
            conveyor_name_bytes: Arena::new(capacities.conveyor_name_bytes),
            line_custom_name_bytes: Arena::new(capacities.line_custom_name_bytes),
            license_records: CaptureRows::new(capacities.rows[16]),
            license_handles: Arena::new(capacities.license_handles),
            divisions_raw_mode: false,
            rest_raw_mode: false,
            division_details_active: true,
        }
    }

    pub fn reset(&mut self) {
        self.variants.reset();
        self.lines.reset();
        self.stockpiles.reset();
        self.division_equipment.reset();
        self.groups.reset();
        self.requests.reset();
        self.deliveries.reset();
        self.produced.reset();
        self.licenses.reset();
        self.conveyors.reset();
        self.deployment_lines.reset();
        self.deployment_manpower.reset();
        self.deployment_equipment.reset();
        self.conveyor_name_bytes.reset();
        self.line_custom_name_bytes.reset();
        self.license_records.reset();
        self.license_handles.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.variants.allocated_bytes()
            + self.lines.allocated_bytes()
            + self.stockpiles.allocated_bytes()
            + self.division_equipment.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.requests.allocated_bytes()
            + self.deliveries.allocated_bytes()
            + self.produced.allocated_bytes()
            + self.licenses.allocated_bytes()
            + self.conveyors.allocated_bytes()
            + self.deployment_lines.allocated_bytes()
            + self.deployment_manpower.allocated_bytes()
            + self.deployment_equipment.allocated_bytes()
            + self.conveyor_name_bytes.allocated_bytes()
            + self.line_custom_name_bytes.allocated_bytes()
            + self.license_records.allocated_bytes()
            + self.license_handles.allocated_bytes()
    }

    pub fn grow(&mut self, capacities: &mut Capacities) {
        self.variants.grow(capacities);
        self.lines.grow(&mut capacities.rows[0]);
        self.stockpiles.grow(&mut capacities.rows[1]);
        self.division_equipment.grow(&mut capacities.rows[2]);
        capacities.rows[7] = capacities.rows[7].max(self.groups.rows.capacity());
        self.groups.grow(&mut capacities.rows[7]);
        self.requests.grow(&mut capacities.rows[8]);
        self.deliveries.grow(&mut capacities.rows[9]);
        self.produced.grow(&mut capacities.rows[10]);
        self.licenses.grow(&mut capacities.rows[11]);
        self.conveyors.grow(&mut capacities.rows[12]);
        self.deployment_lines.grow(&mut capacities.rows[13]);
        self.deployment_manpower.grow(&mut capacities.rows[14]);
        self.deployment_equipment.grow(&mut capacities.rows[15]);
        self.conveyor_name_bytes
            .grow(&mut capacities.conveyor_name_bytes);
        self.line_custom_name_bytes
            .grow(&mut capacities.line_custom_name_bytes);
        self.license_records.grow(&mut capacities.rows[16]);
        self.license_handles.grow(&mut capacities.license_handles);
    }

    pub(crate) fn division_details_active(&self) -> bool {
        self.division_details_active
    }

    /// Acquire scheduled non-variant tables from the current shared census.
    /// Durations are QPC ticks.
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        divisions: Sources<'_, cequipment_variant_pool::DivisionSource>,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        let mut ticks = [0; N];
        let rest_active = replay::cadence::should_walk(TableId::ProductionLines, hour, force_full);
        let divisions_active =
            replay::cadence::should_walk(TableId::DivisionEquipment, hour, force_full);
        self.division_details_active = divisions_active;
        if !rest_active {
            self.fail_production_rest_capture(replay::cadence::SKIPPED_REASON);
            self.conveyor_name_bytes.reset();
            self.line_custom_name_bytes.reset();
            self.license_handles.reset();
        }
        if !divisions_active {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.division_equipment.outcome = skipped;
            self.groups.outcome = skipped;
            self.requests.outcome = skipped;
            self.deliveries.outcome = skipped;
            self.produced.outcome = skipped;
            self.divisions_raw_mode = false;
        }
        if rest_active && !self.rest_raw_mode {
            let mut reads = read_accounting::table(TABLES[0]);
            let begin = qpc();
            capture_rows(&mut self.lines, |rows| {
                cproduction_status::walk_lines(memory, base, game_state, rows)
            });
            ticks[0] = elapsed(begin);
            reads.select(TABLES[1]);
            let begin = qpc();
            capture_rows(&mut self.stockpiles, |rows| {
                cequipment_variant_pool::walk_stockpiles(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        }
        if divisions_active && !self.divisions_raw_mode {
            let mut reads = read_accounting::table(TABLES[2]);
            let begin = qpc();
            let source = complete_source(divisions.rows, divisions.outcome);
            self.division_equipment.outcome = match source {
                Ok(source) => cequipment_variant_pool::walk_divisions(
                    memory,
                    base,
                    source,
                    &mut self.division_equipment.rows,
                ),
                Err(reason) => Outcome::Unavailable { reason },
            };
            ticks[2] = elapsed(begin);
            reads.select(TABLES[7]);
            let begin = qpc();
            let result = match source {
                Ok(source) => carmy_upgrades_status::walk(
                    memory,
                    base,
                    source,
                    &mut self.groups.rows,
                    &mut self.requests.rows,
                    &mut self.deliveries.rows,
                    &mut self.produced.rows,
                ),
                Err(reason) => carmy_upgrades_status::Outcomes::unavailable(reason),
            };
            self.groups.outcome = result.groups;
            self.requests.outcome = result.requests;
            self.deliveries.outcome = result.deliveries;
            self.produced.outcome = result.produced;
            ticks[7] = elapsed(begin);
        }
        if rest_active && !self.rest_raw_mode {
            let mut reads = read_accounting::table(TABLES[11]);
            let begin = qpc();
            capture_rows(&mut self.licenses, |rows| {
                cproduction_status::walk_licenses(memory, base, game_state, rows)
            });
            let records = cproduction_status::walk_license_records(
                memory,
                base,
                game_state,
                &mut self.license_records.rows,
                &mut self.license_handles.first,
            );
            self.license_records.outcome = records.outcome;
            self.license_handles.len = records.handles_len;
            self.license_handles.required = records.required_handles;
            ticks[11] = elapsed(begin);
            reads.select(TABLES[12]);
            let begin = qpc();
            let deployment = self.deployment(memory, base, game_state);
            self.conveyors.outcome = deployment.conveyors;
            self.deployment_lines.outcome = deployment.lines;
            self.deployment_manpower.outcome = deployment.manpower;
            self.deployment_equipment.outcome = deployment.equipment;
            self.conveyor_name_bytes.len = deployment.name_bytes;
            self.conveyor_name_bytes.required = deployment.required_name_bytes;
            self.line_custom_name_bytes.len = deployment.custom_name_bytes;
            self.line_custom_name_bytes.required = deployment.required_custom_name_bytes;
            ticks[12] = elapsed(begin);
        }
        ticks
    }

    /// One shared traversal fills all four deployment buffers.
    fn deployment(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
    ) -> cdeployment_status::Outcomes {
        cdeployment_status::walk(
            memory,
            base,
            game_state,
            &mut self.conveyors.rows,
            &mut self.deployment_lines.rows,
            &mut self.deployment_manpower.rows,
            &mut self.deployment_equipment.rows,
            &mut self.conveyor_name_bytes.first,
            &mut self.line_custom_name_bytes.first,
        )
    }

    /// Decode-worker interpretation of a scheduled production-rest payload.
    /// Replays the existing lines, stockpile, licence, deployment and licence-
    /// record walks against the recorded spans, installing identical rows,
    /// arenas, truncation counts and failure reasons into the frame's pooled
    /// buffers. Clears `rest_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_production_rest_payload(
        &mut self,
        base: usize,
        game_state: usize,
        job: &ProductionRestJob,
        decode_ticks: &mut [u64; 8],
    ) {
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        self.lines.outcome =
            cproduction_status::walk_lines(&recorded, base, game_state, &mut self.lines.rows);
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.stockpiles.outcome = cequipment_variant_pool::walk_stockpiles(
            &recorded,
            base,
            game_state,
            &mut self.stockpiles.rows,
        );
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        self.licenses.outcome =
            cproduction_status::walk_licenses(&recorded, base, game_state, &mut self.licenses.rows);
        let result = cproduction_status::walk_license_records(
            &recorded,
            base,
            game_state,
            &mut self.license_records.rows,
            &mut self.license_handles.first,
        );
        self.license_records.outcome = result.outcome;
        self.license_handles.len = result.handles_len;
        self.license_handles.required = result.required_handles;
        decode_ticks[2] += elapsed(started);
        let started = qpc();
        let walked = cdeployment_status::walk(
            &recorded,
            base,
            game_state,
            &mut self.conveyors.rows,
            &mut self.deployment_lines.rows,
            &mut self.deployment_manpower.rows,
            &mut self.deployment_equipment.rows,
            &mut self.conveyor_name_bytes.first,
            &mut self.line_custom_name_bytes.first,
        );
        self.conveyors.outcome = walked.conveyors;
        self.deployment_lines.outcome = walked.lines;
        self.deployment_manpower.outcome = walked.manpower;
        self.deployment_equipment.outcome = walked.equipment;
        self.conveyor_name_bytes.len = walked.name_bytes;
        self.conveyor_name_bytes.required = walked.required_name_bytes;
        self.line_custom_name_bytes.len = walked.custom_name_bytes;
        self.line_custom_name_bytes.required = walked.required_custom_name_bytes;
        decode_ticks[3] += elapsed(started);
        self.rest_raw_mode = false;
    }
}

// ===========================================================================
// Scheduled production-rest raw-copy split (ProductionLines, Stockpiles,
// ProductionLicenses, DeploymentConveyors, DeploymentLines,
// DeploymentManpower, DeploymentEquipment, ProductionLicenseRecords).
//
// The acquisition worker copies the country census and per-country raw bytes, and
// the single FIFO decode worker replays the existing typed walks against the
// recorded spans (`crate::raw::Recorded`), so rows, ordinals, arenas,
// truncation and failure reasons are identical by construction (see
// `crate::raw` for the mirror rules).
//
// One shared country census feeds all eight tables (no duplicated censuses);
// the production manager is shared by lines/stockpiles/licences/records and
// the deployment manager by the four deployment tables. Empty
// ProductionLicenseRecords shares the licence walk and moves with it (never
// observed nonempty on the test campaign, but the walk is shared so it is
// recorded, not proven trivial). Licence arenas (produced handles, conveyor
// names, line custom names) are staged decode-side: the log holds the
// underlying string/array bytes and the worker fills the pooled arenas by
// replaying the identical walks.
//
// Fault attribution is conservative: shared census spans carry
// all eight bits, production-manager spans the four production bits, and
// deployment spans all four deployment bits, so any live failure blocks its
// whole shared domain. Per-table leaves use
// single-bit masks. The mirror traverses structurally and records every
// discoverable span, never aborting except on overflow: the replay derives
// every outcome, precedence and error scope from the same recorded spans, so
// later extras are harmless.
// ===========================================================================

/// Fault-attribution bits in family order: [lines, stockpiles, licences,
/// conveyors, lines, manpower, equipment, records]. Shared census spans
/// carry all eight bits; production-manager spans the four production bits;
/// deployment spans all four deployment bits (the walk's shared failure
/// domain).
const M_P_LINES: u32 = 0x01;
const M_P_STOCK: u32 = 0x02;
const M_P_LIC: u32 = 0x04;
const M_P_REC: u32 = 0x80;
const M_P_PROD_MGR: u32 = 0x87;
const M_P_DEPLOY: u32 = 0x78;
const M_P_CENSUS: u32 = 0xFF;

/// One scheduled refresh of raw production-rest payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_production_rest_job`).
#[derive(Debug, Default)]
pub(crate) struct ProductionRestJob {
    pub log: RawLog<8>,
    pub game_state: usize,
    census: Vec<ProductionRestCountry>,
    capacity_charge: crate::raw_memory::Charge,
}

#[derive(Debug, Clone, Copy)]
struct ProductionRestCountry {
    pointer: usize,
}

impl ProductionRestJob {
    pub fn new() -> Self {
        let mut log = RawLog::new(1 << 17, 8 << 20);
        // Optional discovery tracing has its own bounded storage. Exhaustion
        // disables only plan compilation; the ordinary capture still completes.
        log.initialize_read_plan(1 << 17, 1 << 15, 1 << 20);
        let mut job = Self {
            // Late-campaign worst case is about 768 lines, 1.6k stockpiles,
            // licences plus deployment queues with names; both pools start
            // above that so steady state never overflows.
            log,
            game_state: 0,
            census: Vec::with_capacity(crate::country_scale::census_rows(512)),
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
    /// walk when no prepared source was consumed. Prepared captures that
    /// overflow report a local failure until their storage grows.
    pub fn grow(&mut self) {
        self.log.grow();
        crate::raw::double_capacity(&mut self.census);
        self.update_capacity_charge();
    }

    /// Off the game thread: hold one census row per observed country slot so
    /// the first hour after a large save loads does not overflow.
    pub fn presize_countries(&mut self) {
        if crate::country_scale::reserve_census(&mut self.census) {
            self.update_capacity_charge();
        }
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
pub(crate) struct ProductionRestCopyReport {
    /// False when staging overflowed or a prepared capture failed. The caller
    /// may use the typed walk only when plan_failed is also false.
    pub raw_ok: bool,
    /// Transcript disagreement or capacity exhaustion after prepared source
    /// observations. This capture must not trigger another live acquisition.
    pub plan_failed: bool,
    pub copy_ticks: [u64; 8],
    pub raw_bytes: [u64; 8],
}

/// Acquisition-job parameters supplied at the sampling boundary.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProductionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
}

fn p_pool_u64(log: &RawLog<8>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

fn p_pool_u32(log: &RawLog<8>, start: usize) -> u32 {
    u32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn pspan(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Mirror of one MSVC string's reads: 32-byte header plus heap payload and
/// terminator. Inline strings ride in the header span; the replay requests
/// exact ranges, so record the payload exactly like a heap one.
fn copy_production_string(log: &mut RawLog<8>, memory: &impl Memory, string: usize, mask: u32) {
    let Some(header_start) = pspan(log, memory, Some(string), 32, mask) else {
        return;
    };
    // The walk reads length/capacity/pointer as separate 8-byte spans, not
    // just the 32-byte header above; record those exact spans too so
    // exact-match service hits. Extras are harmless.
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        mask,
    );
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        mask,
    );
    let _ = pspan(log, memory, Some(string), 8, mask);
    let len = match usize::try_from(p_pool_u64(log, header_start + msvc_string::LENGTH)) {
        Ok(len) => len,
        Err(_) => return,
    };
    let capacity = p_pool_u64(log, header_start + msvc_string::CAPACITY);
    if len as u64 > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return;
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        string
    } else {
        match usize::try_from(p_pool_u64(log, header_start))
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
        let _ = pspan(log, memory, Some(source), len, mask);
    }
    let _ = pspan(log, memory, source.checked_add(len), 1, mask);
}

/// Authenticated country census shared by all eight tables. Records the exact
/// read set of `ccountry::Reader::new` plus one full `read_at` per slot.
fn copy_production_census(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    census: &mut Vec<ProductionRestCountry>,
) -> bool {
    let Some(countries_start) = pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRIES),
        16,
        M_P_CENSUS,
    ) else {
        return false;
    };
    let Some(links_start) = pspan(
        log,
        memory,
        game_state.checked_add(cgame_state::COUNTRY_LINKS),
        16,
        M_P_CENSUS,
    ) else {
        return false;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[countries_start..countries_start + 16]);
    let countries = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return false,
    };
    raw.copy_from_slice(&log.bytes[links_start..links_start + 16]);
    let links = match Array::from_header(raw, 4) {
        Some(a) => a,
        None => return false,
    };
    if countries.count < 2 || links.count == 0 {
        return false;
    }
    let vtable = match base.checked_add(ccountry::VTABLE_RVA) {
        Some(v) => v as u64,
        None => return false,
    };
    for slot in 1..countries.count {
        let pointer_addr = (slot as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(pointer_start) = pspan(log, memory, pointer_addr, 8, M_P_CENSUS) else {
            return false;
        };
        let pointer = match usize::try_from(p_pool_u64(log, pointer_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(vtable_start) = pspan(log, memory, Some(pointer), 8, M_P_CENSUS) else {
            return false;
        };
        let Some(id_start) = pspan(
            log,
            memory,
            pointer.checked_add(ccountry::ID),
            4,
            M_P_CENSUS,
        ) else {
            return false;
        };
        if p_pool_u64(log, vtable_start) != vtable {
            return false;
        }
        let id = p_pool_u32(log, id_start);
        let canonical = match copy_production_link(log, memory, &links, countries.count, id) {
            Some(Some(canonical)) => canonical,
            _ => return false,
        };
        let canonical_slot = match copy_production_link(log, memory, &links, countries.count, slot)
        {
            Some(Some(canonical_slot)) => canonical_slot,
            _ => return false,
        };
        if canonical_slot != canonical {
            return false;
        }
        let check = match copy_production_link(log, memory, &links, countries.count, canonical) {
            Some(Some(check)) => check,
            _ => return false,
        };
        if check != canonical {
            return false;
        }
        let canonical_addr = (canonical as usize)
            .checked_mul(8)
            .and_then(|o| countries.data.checked_add(o));
        let Some(canonical_start) = pspan(log, memory, canonical_addr, 8, M_P_CENSUS) else {
            return false;
        };
        if p_pool_u64(log, canonical_start) != pointer as u64 {
            return false;
        }
        if census.len() == census.capacity() {
            log.overflowed = true;
            return false;
        }
        census.push(ProductionRestCountry { pointer });
    }
    true
}

fn copy_production_link(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    links: &Array,
    countries: u32,
    raw: u32,
) -> Option<Option<u32>> {
    if raw == 0 || raw >= links.count {
        return None;
    }
    let offset = (raw as usize).checked_mul(4)?;
    let start = pspan(log, memory, links.data.checked_add(offset), 4, M_P_CENSUS)?;
    let slot = p_pool_u32(log, start);
    if slot == 0 {
        return Some(None);
    }
    if slot >= countries {
        return None;
    }
    Some(Some(slot))
}

/// Production manager for one country (shared by lines/stockpiles/licences/
/// records). Returns the manager pointer on success.
fn copy_production_manager(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    country: &ProductionRestCountry,
) -> Option<usize> {
    let ptr_start = pspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::PRODUCTION_STATUS),
        8,
        M_P_PROD_MGR,
    )?;
    let pointer = usize::try_from(p_pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)?;
    let expected = base.checked_add(cproduction_status::VTABLE_RVA)? as u64;
    let vtable_start = pspan(log, memory, Some(pointer), 8, M_P_PROD_MGR)?;
    let owner_start = pspan(
        log,
        memory,
        pointer.checked_add(cproduction_status::COUNTRY),
        8,
        M_P_PROD_MGR,
    )?;
    if p_pool_u64(log, vtable_start) != expected {
        return None;
    }
    if p_pool_u64(log, owner_start) != country.pointer as u64 {
        return None;
    }
    Some(pointer)
}

/// Deployment manager for one country (shared by the four deployment tables).
fn copy_deployment_manager(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    country: &ProductionRestCountry,
) -> Option<usize> {
    let ptr_start = pspan(
        log,
        memory,
        country.pointer.checked_add(ccountry::DEPLOYMENT_MANAGER),
        8,
        M_P_DEPLOY,
    )?;
    let pointer = usize::try_from(p_pool_u64(log, ptr_start))
        .ok()
        .filter(|&p| p != 0)?;
    let expected = base.checked_add(cdeployment_status::VTABLE_RVA)? as u64;
    let vtable_start = pspan(log, memory, Some(pointer), 8, M_P_DEPLOY)?;
    let owner_start = pspan(
        log,
        memory,
        pointer.checked_add(cdeployment_status::OWNER),
        8,
        M_P_DEPLOY,
    )?;
    if p_pool_u64(log, vtable_start) != expected {
        return None;
    }
    if p_pool_u64(log, owner_start) != country.pointer as u64 {
        return None;
    }
    Some(pointer)
}

fn copy_variant_identity(log: &mut RawLog<8>, memory: &impl Memory, pointer: usize, mask: u32) {
    let _ = pspan(log, memory, Some(pointer), 16, mask);
}

/// Mirror of `walk_lines`' read set: array header, per-line pointers, the
/// eight scattered field spans per line plus variant identities, then each
/// line's maximum-factory and efficiency reads. `defines` holds the six
/// maximum-factory defines once a line of this pass has staged them; the walk
/// reads them once, at the first line that needs one.
fn copy_production_lines(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    manager: usize,
    defines: &mut Option<[i32; 6]>,
) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LINES),
        16,
        M_P_LINES,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for index in 0..array.count {
        let addr = array.data.checked_add((index as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_LINES) else {
            continue;
        };
        let pointer = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        // Eight scattered spans exactly like `read_row`'s `copy_scattered`.
        let takes = [
            (pointer, 16),
            (
                pointer
                    .checked_add(cproduction_line::ACTIVE_FACTORIES)
                    .unwrap_or(0),
                4,
            ),
            (
                pointer
                    .checked_add(cproduction_line::PRODUCTION_STATUS)
                    .unwrap_or(0),
                8,
            ),
            (pointer.checked_add(cproduction_line::COST).unwrap_or(0), 24),
            (
                pointer.checked_add(cproduction_line::PRIORITY).unwrap_or(0),
                8,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::EQUIPMENT_VARIANT)
                    .unwrap_or(0),
                16,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::REQUESTED_FACTORIES)
                    .unwrap_or(0),
                4,
            ),
            (
                pointer
                    .checked_add(cequipment_production_line::FACTORY_EFFICIENCIES)
                    .unwrap_or(0),
                16,
            ),
        ];
        let mut starts = [None; 8];
        for (start, (addr, len)) in starts.iter_mut().zip(takes) {
            *start = pspan(log, memory, Some(addr), len, M_P_LINES);
        }
        let variant = starts[5].and_then(|start| {
            usize::try_from(p_pool_u64(log, start))
                .ok()
                .filter(|&p| p != 0)
        });
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LINES);
            if let [Some(identity), Some(active), .., Some(header)] = starts {
                let kind = usize::try_from(p_pool_u64(log, identity))
                    .ok()
                    .and_then(|vtable| vtable.checked_sub(base))
                    .and_then(line_kind);
                let active = i32::from_le_bytes(log.bytes[active..active + 4].try_into().unwrap());
                let header = log.bytes[header..header + 16].try_into().unwrap();
                if let Some(kind) = kind {
                    copy_line_efficiency(log, memory, base, kind, v, active, header, defines);
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// `read_row`'s line kind by exact primary vtable.
fn line_kind(rva: usize) -> Option<u8> {
    Some(match rva {
        engine::cmilitary_production_line::VTABLE_RVA => cequipment_production_line::KIND_MILITARY,
        engine::cnaval_production_line::VTABLE_RVA => cequipment_production_line::KIND_NAVAL,
        engine::cship_refit_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_SHIP_REFIT
        }
        engine::crailway_gun_production_line::VTABLE_RVA => {
            cequipment_production_line::KIND_RAILWAY_GUN
        }
        _ => return None,
    })
}

/// Mirror of `read_row`'s maximum factories and the efficiency entries they
/// select. A failed or unparsable span stops here; the replay fails the same.
#[allow(clippy::too_many_arguments)]
fn copy_line_efficiency(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    active: i32,
    header: [u8; 16],
    defines: &mut Option<[i32; 6]>,
) {
    let Some(maximum) = copy_max_factories(log, memory, base, kind, variant, defines) else {
        return;
    };
    let count = u32::try_from(active.min(maximum)).unwrap_or(0);
    if count == 0 {
        return;
    }
    let Some(array) = Array::from_header(header, 8).filter(|array| count <= array.count) else {
        return;
    };
    let batch = cequipment_production_line::EFFICIENCY_BATCH as u32;
    for first in (0..count).step_by(batch as usize) {
        let len = (count - first).min(batch) as usize;
        if pspan(log, memory, array.element(first, 8), len * 8, M_P_LINES).is_none() {
            return;
        }
    }
}

/// Mirror of `cequipment_production_line::max_factories`: the variant's type
/// link and classifier, then the type's and its archetype's optionals, then
/// the defines the kind selects.
fn copy_max_factories(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    kind: u8,
    variant: usize,
    defines: &mut Option<[i32; 6]>,
) -> Option<i32> {
    use engine::cequipment_type;
    let optional = match kind {
        cequipment_production_line::KIND_RAILWAY_GUN => {
            return copy_factory_define(log, memory, base, defines, 5);
        }
        cequipment_production_line::KIND_MILITARY => 0,
        _ => cequipment_type::MAX_NAV_FACTORIES - cequipment_type::MAX_MIL_FACTORIES,
    };
    let definition = pspan(
        log,
        memory,
        variant.checked_add(cequipment_variant::DEFINITION),
        8,
        M_P_LINES,
    )?;
    let flags = pspan(
        log,
        memory,
        variant.checked_add(cequipment_variant::CLASSIFIER_FLAGS),
        8,
        M_P_LINES,
    )?;
    let expected = base.checked_add(cequipment_type::VTABLE_RVA)? as u64;
    let mut equipment = usize::try_from(p_pool_u64(log, definition)).ok()?;
    for archetype in [false, true] {
        if equipment == 0 {
            return None;
        }
        let vtable = pspan(log, memory, Some(equipment), 8, M_P_LINES)?;
        let maximum = pspan(
            log,
            memory,
            equipment.checked_add(cequipment_type::MAX_MIL_FACTORIES),
            16,
            M_P_LINES,
        )?;
        let next = if archetype {
            None
        } else {
            Some(pspan(
                log,
                memory,
                equipment.checked_add(cequipment_type::ARCHETYPE),
                8,
                M_P_LINES,
            )?)
        };
        if p_pool_u64(log, vtable) != expected {
            return None;
        }
        let value = maximum + optional;
        if log.bytes[value + 4] != 0 {
            return Some(i32::from_le_bytes(
                log.bytes[value..value + 4].try_into().unwrap(),
            ));
        }
        if let Some(next) = next {
            equipment = usize::try_from(p_pool_u64(log, next)).ok()?;
        }
    }
    let flags = p_pool_u64(log, flags);
    let index = if kind == cequipment_production_line::KIND_MILITARY {
        0
    } else if flags & cequipment_variant::CONVOY_CLASSIFIER_MASK != 0 {
        1
    } else if flags & cequipment_variant::FLOATING_HARBOR_CLASSIFIER_MASK != 0 {
        2
    } else if flags & cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK != 0 {
        4
    } else {
        3
    };
    copy_factory_define(log, memory, base, defines, index)
}

/// Stages the six maximum-factory defines at the first line that needs one;
/// the walk reads them together once.
fn copy_factory_define(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    defines: &mut Option<[i32; 6]>,
    index: usize,
) -> Option<i32> {
    if defines.is_none() {
        let mut values = [0; 6];
        for (value, rva) in values
            .iter_mut()
            .zip(cequipment_production_line::MAX_FACTORY_DEFINE_RVAS)
        {
            let start = pspan(log, memory, base.checked_add(rva), 4, M_P_LINES)?;
            *value = i32::from_le_bytes(log.bytes[start..start + 4].try_into().unwrap());
        }
        *defines = Some(values);
    }
    defines.map(|values| values[index])
}

/// Mirror of `walk_stockpiles`' read set: pool vtable, array header,
/// per-entry words plus variant identities.
fn copy_production_stockpiles(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let pool = match manager.checked_add(cproduction_status::STOCKPILE_POOL) {
        Some(p) => p,
        None => return,
    };
    let _ = pspan(log, memory, Some(pool), 8, M_P_STOCK);
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::STOCKPILE),
        16,
        M_P_STOCK,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, equipment_entry::STRIDE) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.element(ordinal, equipment_entry::STRIDE);
        let Some(entry_start) = pspan(log, memory, addr, equipment_entry::STRIDE, M_P_STOCK) else {
            continue;
        };
        let variant = usize::try_from(p_pool_u64(log, entry_start))
            .ok()
            .filter(|&p| p != 0);
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_STOCK);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `walk_licenses`' read set: array header, per-licence pointers
/// plus variant identities.
fn copy_production_licenses(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LICENSES),
        16,
        M_P_LIC,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.data.checked_add((ordinal as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_LIC) else {
            continue;
        };
        let variant = usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0);
        // Licence entries hold variant pointers directly; authenticate via identity.
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LIC);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `walk_license_records`' read set: array header, per-record
/// 0x58 heads, parent identities, equipment headers plus per-handle pointers
/// and identities.
fn copy_production_records(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cproduction_status::LICENSE_RECORDS),
        16,
        M_P_REC,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let array = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for ordinal in 0..array.count {
        let addr = array.data.checked_add((ordinal as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_REC) else {
            continue;
        };
        let pointer = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(head_start) = pspan(log, memory, Some(pointer), 0x58, M_P_REC) else {
            continue;
        };
        let parent = usize::try_from(p_pool_u64(
            log,
            head_start + cequipment_production_license::PARENT,
        ))
        .ok()
        .filter(|&p| p != 0);
        if let Some(p) = parent {
            copy_variant_identity(log, memory, p, M_P_REC);
        }
        let equipment = Array::from_header(
            log.bytes[head_start + cequipment_production_license::EQUIPMENT
                ..head_start + cequipment_production_license::EQUIPMENT + 16]
                .try_into()
                .unwrap(),
            8,
        );
        if let Some(equipment) = equipment {
            for index in 0..equipment.count {
                let addr = equipment
                    .data
                    .checked_add((index as usize).saturating_mul(8));
                let Some(h_start) = pspan(log, memory, addr, 8, M_P_REC) else {
                    continue;
                };
                let variant = usize::try_from(p_pool_u64(log, h_start))
                    .ok()
                    .filter(|&p| p != 0);
                if let Some(v) = variant {
                    copy_variant_identity(log, memory, v, M_P_REC);
                }
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of the shared deployment walk's read set: conveyor array, per
/// conveyor heads plus templates, line arrays and names, per-line heads plus
/// name objects, custom strings, deployment objects with pools and manpower,
/// plus per-entry words and variant identities. All spans carry the combined
/// deployment mask, matching the walk's shared failure domain.
fn copy_deployment_queue(log: &mut RawLog<8>, memory: &impl Memory, base: usize, manager: usize) {
    let Some(header_start) = pspan(
        log,
        memory,
        manager.checked_add(cdeployment_status::CONVEYORS),
        16,
        M_P_DEPLOY,
    ) else {
        return;
    };
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&log.bytes[header_start..header_start + 16]);
    let queue = match Array::from_header(raw, 8) {
        Some(a) => a,
        None => return,
    };
    for index in 0..queue.count {
        let addr = queue.data.checked_add((index as usize).saturating_mul(8));
        let Some(ptr_start) = pspan(log, memory, addr, 8, M_P_DEPLOY) else {
            continue;
        };
        let conveyor = match usize::try_from(p_pool_u64(log, ptr_start))
            .ok()
            .filter(|&p| p != 0)
        {
            Some(p) => p,
            None => continue,
        };
        let Some(head_start) = pspan(log, memory, Some(conveyor), 0x9c, M_P_DEPLOY) else {
            continue;
        };
        let template = usize::try_from(p_pool_u64(
            log,
            head_start + cmilitary_deployment_conveyor::TEMPLATE,
        ))
        .ok()
        .filter(|&p| p != 0);
        if let Some(t) = template {
            // Template vtable plus reference words.
            let _ = pspan(log, memory, Some(t), 16, M_P_DEPLOY);
        }
        // Conveyor name string.
        if let Some(name) = conveyor.checked_add(cmilitary_deployment_conveyor::NAME) {
            copy_production_string(log, memory, name, M_P_DEPLOY);
        }
        // Line array header as its own exact span: the walk reads it via
        // `Array::read` at +0x78, which exact-match service does not derive
        // from the 0x9c head span above.
        let _ = pspan(
            log,
            memory,
            conveyor.checked_add(cmilitary_deployment_conveyor::LINES),
            16,
            M_P_DEPLOY,
        );
        let lines = Array::from_header(
            log.bytes[head_start + cmilitary_deployment_conveyor::LINES
                ..head_start + cmilitary_deployment_conveyor::LINES + 16]
                .try_into()
                .unwrap(),
            8,
        );
        let Some(lines) = lines else {
            continue;
        };
        for ordinal in 0..lines.count {
            let addr = lines.data.checked_add((ordinal as usize).saturating_mul(8));
            let Some(lptr_start) = pspan(log, memory, addr, 8, M_P_DEPLOY) else {
                continue;
            };
            let line = match usize::try_from(p_pool_u64(log, lptr_start))
                .ok()
                .filter(|&p| p != 0)
            {
                Some(p) => p,
                None => continue,
            };
            copy_deployment_line(log, memory, base, conveyor, line);
            if log.overflowed {
                return;
            }
        }
        if log.overflowed {
            return;
        }
    }
}

fn copy_deployment_line(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    conveyor: usize,
    line: usize,
) {
    let _ = base;
    let Some(head_start) = pspan(log, memory, Some(line), 0x40, M_P_DEPLOY) else {
        return;
    };
    let name = usize::try_from(p_pool_u64(
        log,
        head_start + cmilitary_deployment_line::NAME,
    ))
    .ok()
    .filter(|&p| p != 0);
    let deployment = usize::try_from(p_pool_u64(
        log,
        head_start + cmilitary_deployment_line::DEPLOYMENT,
    ))
    .ok()
    .filter(|&p| p != 0);
    let (Some(name), Some(deployment)) = (name, deployment) else {
        return;
    };
    // Name object: vtable, order, override header, flags; then custom bytes.
    let _ = pspan(log, memory, Some(name), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::ORDER),
        4,
        M_P_DEPLOY,
    );
    let string = match name.checked_add(cname_group_member::OVERRIDE_NAME) {
        Some(s) => s,
        None => return,
    };
    let Some(chead_start) = pspan(log, memory, Some(string), 32, M_P_DEPLOY) else {
        return;
    };
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::LENGTH),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        string.checked_add(msvc_string::CAPACITY),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(log, memory, Some(string), 8, M_P_DEPLOY);
    let len = match usize::try_from(p_pool_u64(log, chead_start + msvc_string::LENGTH)) {
        Ok(l) => l,
        Err(_) => return,
    };
    let capacity = p_pool_u64(log, chead_start + msvc_string::CAPACITY);
    if len as u64 <= capacity && capacity >= msvc_string::INLINE_CAPACITY {
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            string
        } else {
            match usize::try_from(p_pool_u64(log, chead_start))
                .ok()
                .filter(|&s| s != 0)
            {
                Some(s) => s,
                None => string,
            }
        };
        if source.checked_add(len).is_some() {
            if len != 0 {
                let _ = pspan(log, memory, Some(source), len, M_P_DEPLOY);
            }
            let _ = pspan(log, memory, source.checked_add(len), 1, M_P_DEPLOY);
        }
    }
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::FLAG_A),
        1,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        name.checked_add(cname_group_member::FLAG_B),
        1,
        M_P_DEPLOY,
    );
    // Parent backlink is inside the head span already; deployment object next.
    let _ = conveyor;
    copy_deployment_object(log, memory, base, deployment, line);
}

fn copy_deployment_object(
    log: &mut RawLog<8>,
    memory: &impl Memory,
    base: usize,
    deployment: usize,
    line: usize,
) {
    let _ = base;
    let _ = line;
    let _ = pspan(log, memory, Some(deployment), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::SECONDARY_VTABLE),
        8,
        M_P_DEPLOY,
    );
    let pool = match deployment.checked_add(cmilitary_deployment::EQUIPMENT_POOL) {
        Some(p) => p,
        None => return,
    };
    let _ = pspan(log, memory, Some(pool), 8, M_P_DEPLOY);
    let _ = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ALLOW_ZERO),
        1,
        M_P_DEPLOY,
    );
    // Equipment pool arrays.
    if let Some(h) = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ITEMS),
        16,
        M_P_DEPLOY,
    ) {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[h..h + 16]);
        if let Some(array) = Array::from_header(raw, equipment_entry::STRIDE) {
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, equipment_entry::STRIDE);
                if let Some(e_start) = pspan(log, memory, addr, equipment_entry::STRIDE, M_P_DEPLOY)
                {
                    let variant = usize::try_from(p_pool_u64(log, e_start))
                        .ok()
                        .filter(|&p| p != 0);
                    if let Some(v) = variant {
                        copy_variant_identity(log, memory, v, M_P_DEPLOY);
                    }
                }
            }
        }
    }
    if let Some(h) = pspan(
        log,
        memory,
        pool.checked_add(cequipment_variant_pool::ALLOCATIONS),
        16,
        M_P_DEPLOY,
    ) {
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[h..h + 16]);
        if let Some(array) = Array::from_header(raw, 24) {
            // Allocation entries are counted, not decoded per row here; record
            // the contiguous block as individual entry spans via takes below.
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, 24);
                let _ = pspan(log, memory, addr, 24, M_P_DEPLOY);
            }
        }
    }
    // Scalars.
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::TRAINING),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::MAX_TRAINING),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::MAX_MANPOWER),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::OWNING_LINE),
        8,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::DISTRIBUTABLE_PRIORITY),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::HALTING_REASON),
        4,
        M_P_DEPLOY,
    );
    let _ = pspan(
        log,
        memory,
        deployment.checked_add(cmilitary_deployment::HALTING_REASON_PRESENT),
        1,
        M_P_DEPLOY,
    );
    // Manpower containers.
    for offset in [
        cmilitary_deployment::MANPOWER_HAVE,
        cmilitary_deployment::MANPOWER_NEED,
    ] {
        let container = match deployment.checked_add(offset) {
            Some(c) => c,
            None => continue,
        };
        let _ = pspan(log, memory, Some(container), 8, M_P_DEPLOY);
        let Some(data_start) = pspan(
            log,
            memory,
            container.checked_add(carmy_manpower_values::DATA),
            16,
            M_P_DEPLOY,
        ) else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[data_start..data_start + 16]);
        if let Some(array) = Array::from_header(raw, manpower_value_entry::STRIDE) {
            for ordinal in 0..array.count {
                let addr = array.element(ordinal, manpower_value_entry::STRIDE);
                let _ = pspan(log, memory, addr, manpower_value_entry::STRIDE, M_P_DEPLOY);
            }
        }
    }
}

/// Acquire one owned payload and report its per-table costs.
pub(crate) fn copy_production_rest_refresh(
    memory: &impl Memory,
    ctx: ProductionRestCopyContext,
    job: &mut ProductionRestJob,
) -> ProductionRestCopyReport {
    let started = qpc();
    job.clear();
    job.game_state = ctx.game_state;
    acquire_production_rest_pass(memory, ctx.base, ctx.game_state, job);
    let mut ticks = [0; 8];
    ticks[0] = elapsed(started);
    let plan_failed = job.log.planned_capture_requires_no_retry();
    ProductionRestCopyReport {
        raw_ok: !plan_failed && !job.log.overflowed,
        plan_failed,
        copy_ticks: ticks,
        raw_bytes: job.log.raw_bytes,
    }
}

impl Captured {
    /// Finish an accepted acquisition failure without retrying live memory.
    pub(crate) fn fail_production_rest_capture(&mut self, reason: &'static str) {
        self.lines.outcome = Outcome::Unavailable { reason };
        self.stockpiles.outcome = Outcome::Unavailable { reason };
        self.licenses.outcome = Outcome::Unavailable { reason };
        self.conveyors.outcome = Outcome::Unavailable { reason };
        self.deployment_lines.outcome = Outcome::Unavailable { reason };
        self.deployment_manpower.outcome = Outcome::Unavailable { reason };
        self.deployment_equipment.outcome = Outcome::Unavailable { reason };
        self.license_records.outcome = Outcome::Unavailable { reason };
        self.rest_raw_mode = false;
    }
}

/// Reuse only the discovery program. Every successful replay acquires fresh
/// payload bytes, and its controls must match before a dependent address is
/// read. A changed control resumes the producer with the already-observed
/// ordinal prefix; it never rereads that prefix or a failed source.
fn acquire_production_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ProductionRestJob,
) {
    let _reads = read_accounting::table(TABLES[0]);
    // These are the producer's external inputs. Output-table capacities
    // do not select its reads; the raw producer captures a superset.
    // Census capacity does affect admission, so it belongs to the plan key.
    let key = [base, game_state, job.census.capacity(), 1];
    if !job.log.begin_planned_capture(memory, key) {
        copy_production_rest_pass(memory, base, game_state, job);
    }
    job.log.finish_planned_capture();
}

/// One structural copy pass: shared census plus every country's production
/// and deployment takes. The job is cleared on entry so pooled refreshes
/// never accumulate spans across hours.
fn copy_production_rest_pass(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    job: &mut ProductionRestJob,
) {
    if !copy_production_census(&mut job.log, memory, base, game_state, &mut job.census) {
        return;
    }
    let mut defines = None;
    for index in 0..job.census.len() {
        let country = job.census[index];
        if let Some(manager) = copy_production_manager(&mut job.log, memory, base, &country) {
            copy_production_lines(&mut job.log, memory, base, manager, &mut defines);
            if job.log.overflowed {
                return;
            }
            copy_production_stockpiles(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
            copy_production_licenses(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
            copy_production_records(&mut job.log, memory, manager);
            if job.log.overflowed {
                return;
            }
        }
        if let Some(manager) = copy_deployment_manager(&mut job.log, memory, base, &country) {
            copy_deployment_queue(&mut job.log, memory, base, manager);
            if job.log.overflowed {
                return;
            }
        }
    }
}

impl Captured {
    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.lines.outcome,
            self.stockpiles.outcome,
            self.division_equipment.outcome,
            self.variants.definitions.outcome,
            self.variants.modules.outcome,
            self.variants.stats.outcome,
            self.variants.upgrades.outcome,
            self.groups.outcome,
            self.requests.outcome,
            self.deliveries.outcome,
            self.produced.outcome,
            self.licenses.outcome,
            self.conveyors.outcome,
            self.deployment_lines.outcome,
            self.deployment_manpower.outcome,
            self.deployment_equipment.outcome,
            self.license_records.outcome,
        ]
    }

    #[cfg(test)]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples rows"),
        })
    }

    /// Fresh owned division payloads compare directly with the encoder's exact
    /// complete baseline before allocating replay rows. Other production tables
    /// retain their existing arena validation and conversion paths.
    pub(crate) fn table_inputs(
        &self,
        mut projection: Option<&mut Projection<'_>>,
    ) -> [TableInput; N] {
        [
            table::<tables::ProductionLines, _>(self.lines.outcome, &self.lines.rows, |r| {
                rows::ProductionLineRow {
                    handle: r.handle,
                    country: r.country,
                    kind: r.kind,
                    variant: r.variant,
                    mio: r.mio,
                    active_factories: r.active_factories,
                    requested_factories: r.requested_factories,
                    cost_raw: r.cost_raw,
                    speed_raw: r.speed_raw,
                    produced_raw: r.produced_raw,
                    priority: r.priority,
                    amount: r.amount,
                    max_factories: r.max_factories,
                    efficiency_raw: r.efficiency_raw,
                }
            })
            .into(),
            table::<tables::Stockpiles, _>(self.stockpiles.outcome, &self.stockpiles.rows, |r| {
                rows::StockpileRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                }
            })
            .into(),
            scalar_table::<tables::DivisionEquipment, _>(
                projection.as_deref_mut(),
                self.division_equipment.outcome,
                &self.division_equipment.rows,
                |r| (r.division as u128) << 32 | r.ordinal as u128,
                |r| rows::DivisionEquipmentRow {
                    division: r.division,
                    template: r.template,
                    country: r.country,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                },
            ),
            definitions(&self.variants.definitions, &self.variants.definition_bytes).into(),
            table::<tables::VariantModules, _>(
                self.variants.modules.outcome,
                &self.variants.modules.rows,
                |r| rows::VariantModuleRow {
                    variant: r.variant,
                    source_ordinal: r.source_ordinal,
                    slot: r.slot,
                    definition_token: r.definition_token,
                    definition_active: r.definition_active,
                },
            )
            .into(),
            table::<tables::VariantStats, _>(
                self.variants.stats.outcome,
                &self.variants.stats.rows,
                |r| rows::VariantStatsRow {
                    variant: r.variant,
                    values: r.values,
                },
            )
            .into(),
            table::<tables::VariantUpgrades, _>(
                self.variants.upgrades.outcome,
                &self.variants.upgrades.rows,
                |r| rows::VariantUpgradeRow {
                    variant: r.variant,
                    source_ordinal: r.source_ordinal,
                    definition_token: r.definition_token,
                    level: r.level,
                },
            )
            .into(),
            scalar_table::<tables::ArmyUpgradeGroups, _>(
                projection.as_deref_mut(),
                self.groups.outcome,
                &self.groups.rows,
                |r| r.army as u128,
                |r| rows::ArmyUpgradeGroupRow {
                    country: r.country,
                    army: r.army,
                    priority: r.priority,
                },
            ),
            scalar_table::<tables::ArmyUpgradeRequests, _>(
                projection.as_deref_mut(),
                self.requests.outcome,
                &self.requests.rows,
                |r| (r.army as u128) << 32 | r.ordinal as u128,
                |r| rows::ArmyUpgradeRequestRow {
                    country: r.country,
                    army: r.army,
                    ordinal: r.ordinal,
                    equipment: r.equipment,
                    request_raw: r.request_raw,
                    total_raw: r.total_raw,
                },
            ),
            scalar_table::<tables::ArmyUpgradeDeliveries, _>(
                projection.as_deref_mut(),
                self.deliveries.outcome,
                &self.deliveries.rows,
                |r| (r.army as u128) << 32 | r.ordinal as u128,
                |r| rows::ArmyUpgradeDeliveryRow {
                    country: r.country,
                    army: r.army,
                    ordinal: r.ordinal,
                    status: r.status,
                    progress_raw: r.progress_raw,
                    total_progress_raw: r.total_progress_raw,
                    allow_zero_entries: r.allow_zero_entries,
                },
            ),
            scalar_table::<tables::ArmyUpgradeProduced, _>(
                projection,
                self.produced.outcome,
                &self.produced.rows,
                |r| (r.army as u128) << 64 | (r.delivery_ordinal as u128) << 32 | r.ordinal as u128,
                |r| rows::ArmyUpgradeProducedRow {
                    country: r.country,
                    army: r.army,
                    delivery_ordinal: r.delivery_ordinal,
                    ordinal: r.ordinal,
                    equipment: r.equipment,
                    amount_raw: r.amount_raw,
                },
            ),
            table::<tables::ProductionLicenses, _>(
                self.licenses.outcome,
                &self.licenses.rows,
                |r| rows::ProductionLicenseRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    variant: r.variant,
                },
            )
            .into(),
            conveyors(&self.conveyors, &self.conveyor_name_bytes).into(),
            deployment_lines(&self.deployment_lines, &self.line_custom_name_bytes).into(),
            table::<tables::DeploymentManpower, _>(
                self.deployment_manpower.outcome,
                &self.deployment_manpower.rows,
                |r| rows::DeploymentManpowerRow {
                    line: r.line,
                    side: r.side,
                    ordinal: r.ordinal,
                    source_country: r.source_country,
                    amount: r.amount,
                },
            )
            .into(),
            table::<tables::DeploymentEquipment, _>(
                self.deployment_equipment.outcome,
                &self.deployment_equipment.rows,
                |r| rows::DeploymentEquipmentRow {
                    line: r.line,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                },
            )
            .into(),
            license_records(&self.license_records, &self.license_handles).into(),
        ]
    }
}

/// Produced-variant handles live in a shared arena; a span outside it fails the
/// table rather than publishing a shortened or borrowed list.
fn license_records(
    rows: &CaptureRows<cproduction_status::LicenseRecordRow>,
    arena: &Arena<u64>,
) -> TableView {
    let result = rows.outcome;
    if let Outcome::Unavailable { reason } = result {
        return unavailable(reason);
    }
    let Some(source) = rows.rows.get(..result.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    let Some(handles) = arena.first.get(..arena.len) else {
        return unavailable("production licence handles lie outside captured arena");
    };
    if source
        .iter()
        .any(|row| span(handles, row.equipment_start, row.equipment_len).is_none())
    {
        return unavailable("production licence handles lie outside captured arena");
    }
    table::<tables::ProductionLicenseRecords, _>(result, source, |r| {
        rows::ProductionLicenseRecordRow {
            owner: r.owner,
            ordinal: r.ordinal,
            giver: r.giver,
            lended_cic: r.lended_cic,
            required_cic: r.required_cic,
            start_date: r.start_date,
            parent: r.parent,
            operand_raw: r.operand_raw,
            equipment: span(handles, r.equipment_start, r.equipment_len)
                .unwrap()
                .to_vec(),
        }
    })
}

fn span(arena: &[u64], start: usize, len: usize) -> Option<&[u64]> {
    arena.get(start..start.checked_add(len)?)
}

/// Conveyor names live in a shared arena; a span outside it fails the table
/// rather than publishing a truncated or borrowed-from-elsewhere name.
fn conveyors(rows: &CaptureRows<cdeployment_status::ConveyorRow>, arena: &Arena<u8>) -> TableView {
    let result = rows.outcome;
    if let Outcome::Unavailable { reason } = result {
        return unavailable(reason);
    }
    let Some(source) = rows.rows.get(..result.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    let Some(bytes) = arena.first.get(..arena.len) else {
        return unavailable("deployment conveyor names lie outside captured arena");
    };
    if source
        .iter()
        .any(|row| name(bytes, row.name_start, row.name_len).is_none())
    {
        return unavailable("deployment conveyor names lie outside captured arena");
    }
    table::<tables::DeploymentConveyors, _>(result, source, |r| rows::DeploymentConveyorRow {
        handle: r.handle,
        country: r.country,
        ordinal: r.ordinal,
        template: r.template,
        amount: r.amount,
        location: r.location,
        priority: r.priority,
        role_token: r.role_token,
        closed: r.closed,
        name: name(bytes, r.name_start, r.name_len).unwrap().to_vec(),
        exile_country: r.exile_country,
    })
}

/// Line custom names live in a shared arena; a span outside it fails the table
/// rather than publishing a truncated or borrowed-from-elsewhere name.
fn deployment_lines(
    rows: &CaptureRows<cdeployment_status::LineRow>,
    arena: &Arena<u8>,
) -> TableView {
    let result = rows.outcome;
    if let Outcome::Unavailable { reason } = result {
        return unavailable(reason);
    }
    let Some(source) = rows.rows.get(..result.len()) else {
        return unavailable("walker row count exceeds allocated buffer");
    };
    let Some(bytes) = arena.first.get(..arena.len) else {
        return unavailable("deployment line custom names lie outside captured arena");
    };
    if source
        .iter()
        .any(|row| name(bytes, row.custom_name_start, row.custom_name_len).is_none())
    {
        return unavailable("deployment line custom names lie outside captured arena");
    }
    table::<tables::DeploymentLines, _>(result, source, |r| rows::DeploymentLineRow {
        handle: r.handle,
        conveyor: r.conveyor,
        country: r.country,
        ordinal: r.ordinal,
        name_order: r.name_order,
        amount: r.amount,
        training_raw: r.training_raw,
        max_training_raw: r.max_training_raw,
        max_manpower: r.max_manpower,
        allow_zero_entries: r.allow_zero_entries,
        equipment_allocations: r.equipment_allocations,
        distributable_priority: r.distributable_priority,
        halting_reason_present: r.halting_reason_present,
        halting_reason: r.halting_reason,
        custom_name: name(bytes, r.custom_name_start, r.custom_name_len)
            .unwrap()
            .to_vec(),
        custom_flag_a: r.custom_flag_a,
        custom_flag_b: r.custom_flag_b,
    })
}

fn name(arena: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    arena.get(start..start.checked_add(len)?)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

pub(crate) fn complete_source<T>(source: &[T], outcome: Outcome) -> Result<&[T], &'static str> {
    match outcome {
        Outcome::Rows { len } => source
            .get(..len)
            .ok_or("source row count exceeds acquired buffer"),
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Truncated { .. } => Err("source census incomplete"),
    }
}

fn capture_rows<R>(rows: &mut CaptureRows<R>, mut walk: impl FnMut(&mut [R]) -> Outcome) {
    rows.outcome = walk(&mut rows.rows);
}

fn capture_text<R>(
    rows: &mut CaptureRows<R>,
    arena: &mut Arena<u8>,
    source: Result<&[cequipment_variant::RegistryRow], &'static str>,
    mut walk: impl FnMut(
        &[cequipment_variant::RegistryRow],
        &mut Vec<R>,
        &mut Vec<u8>,
    ) -> cequipment_variant::TextOutcome,
) {
    let result = match source {
        Ok(source) => walk(source, &mut rows.rows, &mut arena.first),
        Err(reason) => cequipment_variant::TextOutcome {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        },
    };
    rows.outcome = result.outcome;
    arena.len = result.bytes_len;
    arena.required = result.required_bytes;
}

fn capture_source<R>(
    rows: &mut CaptureRows<R>,
    source: Result<&[cequipment_variant::RegistryRow], &'static str>,
    walk: impl FnOnce(&[cequipment_variant::RegistryRow], &mut Vec<R>) -> Outcome,
) {
    rows.outcome = match source {
        Ok(source) => walk(source, &mut rows.rows),
        Err(reason) => Outcome::Unavailable { reason },
    };
}

fn text(arena: &[u8], span: cequipment_variant::TextSpan) -> Option<&[u8]> {
    arena.get(span.start..span.start.checked_add(span.len)?)
}

fn checked_text<'a, T, const N: usize>(
    rows: &'a CaptureRows<T>,
    arena: &'a Arena<u8>,
    spans: impl Fn(&T) -> [cequipment_variant::TextSpan; N],
) -> Result<&'a [u8], TableView> {
    let result = rows.outcome;
    if let Outcome::Unavailable { reason } = result {
        return Err(unavailable(reason));
    }
    let Some(source) = rows.rows.get(..result.len()) else {
        return Err(unavailable("walker row count exceeds allocated buffer"));
    };
    let Some(bytes) = arena.first.get(..arena.len) else {
        return Err(unavailable("production text lies outside captured arena"));
    };
    if source.iter().any(|row| {
        spans(row)
            .into_iter()
            .any(|span| text(bytes, span).is_none())
    }) {
        return Err(unavailable("production text lies outside captured arena"));
    }
    Ok(bytes)
}

fn definitions(
    rows: &CaptureRows<cequipment_variant::DefinitionRow>,
    arena: &Arena<u8>,
) -> TableView {
    let bytes = match checked_text(rows, arena, |r| {
        [r.display_name, r.variant_name, r.explicit_entity, r.icon]
    }) {
        Ok(bytes) => bytes,
        Err(failure) => return failure,
    };
    table::<tables::VariantDefinitions, _>(rows.outcome, &rows.rows, |r| {
        rows::VariantDefinitionRow {
            id: r.identity.id,
            equipment_token: r.identity.equipment_token,
            archetype_token: r.identity.archetype_token,
            creator_link: r.identity.creator_link,
            origin_link: r.identity.origin_link,
            append_variant_name: r.identity.append_variant_name,
            obsolete: r.identity.obsolete,
            classifier_flags: r.identity.classifier_flags,
            mission_mask: r.identity.mission_mask,
            display_name: text(bytes, r.display_name).unwrap().to_vec(),
            variant_name: text(bytes, r.variant_name).unwrap().to_vec(),
            explicit_entity: text(bytes, r.explicit_entity).unwrap().to_vec(),
            icon_active: r.icon_active,
            icon_kind: r.icon_kind,
            icon: text(bytes, r.icon).unwrap().to_vec(),
        }
    })
}

/// Shared synthetic address space for the scheduled production-rest
/// copy/decode split: one country with one line, one stockpile entry,
/// one licence, one licence record with one handle, and a one-conveyor
/// deployment queue with one line, manpower and equipment.
#[cfg(test)]
pub(crate) mod production_rest_test_kit {
    use super::{
        Capacities, Captured, N, ProductionRestCopyContext, ProductionRestCopyReport,
        ProductionRestJob, Sources,
    };
    use engine::{
        carmy_manpower_values, ccountry, cdeployment_status, cequipment_production_license,
        cequipment_production_line, cequipment_variant, cequipment_variant_pool, cgame_state,
        cmilitary_deployment, cmilitary_deployment_conveyor, cmilitary_deployment_line,
        cname_group_member, cproduction_line, cproduction_status, creferenced_division_template,
        msvc_string, walk::Outcome,
    };

    pub(crate) const BASE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const PROD: usize = 0x10000;
    const DEPLOY: usize = 0x4000;
    const QUEUE: usize = 0x4100;
    const CONVEYOR: usize = 0x5000;
    const LINE_ARRAY: usize = 0x5200;
    const LINE: usize = 0x6000;
    const NAME_OBJECT: usize = 0x6200;
    const DEPLOYMENT: usize = 0x7000;
    const EQUIPMENT: usize = 0x7400;
    const HAVE: usize = 0x7500;
    const NEED: usize = 0x7580;
    const TEMPLATE: usize = 0x7600;
    const ALLOCATIONS: usize = 0x7680;
    pub(super) const VARIANT: usize = 0x7700;
    pub(super) const PLINE: usize = 0x11000;
    const PDATA: usize = 0x11100;
    const SDATA: usize = 0x11200;
    const LDATA: usize = 0x11300;
    const RDATA: usize = 0x11400;
    const RECORD: usize = 0x11500;
    const RLIST: usize = 0x11600;
    const EDATA: usize = 0x11700;
    /// The variant's equipment type, its own archetype, as constructed.
    pub(super) const EQUIPMENT_TYPE: usize = 0x12000;

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
        for slot in [1, 2] {
            put(
                &mut bytes,
                0x2000 + slot * 8,
                &(COUNTRY as u64).to_le_bytes(),
            );
            put(&mut bytes, 0x2100 + slot * 4, &1_u32.to_le_bytes());
        }
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &2_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY + ccountry::PRODUCTION_STATUS,
            &(PROD as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY + ccountry::DEPLOYMENT_MANAGER,
            &(DEPLOY as u64).to_le_bytes(),
        );
        // Production manager.
        put(
            &mut bytes,
            PROD,
            &(cproduction_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            PROD + cproduction_status::COUNTRY,
            &(COUNTRY as u64).to_le_bytes(),
        );
        array(&mut bytes, PROD + cproduction_status::LINES, PDATA, 1);
        put(&mut bytes, PDATA, &(PLINE as u64).to_le_bytes());
        array(&mut bytes, PROD + cproduction_status::LICENSES, LDATA, 1);
        put(&mut bytes, LDATA, &(VARIANT as u64).to_le_bytes());
        array(
            &mut bytes,
            PROD + cproduction_status::LICENSE_RECORDS,
            RDATA,
            1,
        );
        put(&mut bytes, RDATA, &(RECORD as u64).to_le_bytes());
        put(
            &mut bytes,
            PROD + cproduction_status::STOCKPILE_POOL,
            &(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(&mut bytes, PROD + cproduction_status::STOCKPILE, SDATA, 1);
        put(&mut bytes, SDATA, &(VARIANT as u64).to_le_bytes());
        put(&mut bytes, SDATA + 8, &77_i64.to_le_bytes());
        // Production line (military).
        for (offset, value) in [
            (0, engine::cmilitary_production_line::VTABLE_RVA as u64),
            (cproduction_line::REFERENCE_TYPE, 0x8000_1234_0000_0038),
            (cproduction_line::PRODUCTION_STATUS, PROD as u64),
            (cproduction_line::COST, 100),
            (cproduction_line::SPEED, 200),
            (cproduction_line::PRODUCED, 300),
            (
                cequipment_production_line::EQUIPMENT_VARIANT,
                VARIANT as u64,
            ),
            (
                cequipment_production_line::INDUSTRIAL_ORGANISATION_CID,
                0x1234_5678_0000_0095,
            ),
        ] {
            put(&mut bytes, PLINE + offset, &value.to_le_bytes());
        }
        for (offset, value) in [
            (cproduction_line::ACTIVE_FACTORIES, 12_i32),
            (cproduction_line::PRIORITY, 17),
            (cproduction_line::AMOUNT, -1),
            (cequipment_production_line::REQUESTED_FACTORIES, 14),
        ] {
            put(&mut bytes, PLINE + offset, &value.to_le_bytes());
        }
        // Twenty per-factory efficiencies, Q15, of which the 12 active count.
        array(
            &mut bytes,
            PLINE + cequipment_production_line::FACTORY_EFFICIENCIES,
            EDATA,
            20,
        );
        for index in 0..20 {
            let value = (index as i64 + 40) << 15;
            put(&mut bytes, EDATA + index * 8, &value.to_le_bytes());
        }
        // The variant's equipment type sets its own military maximum, so no
        // define outside this memory is read.
        put(
            &mut bytes,
            VARIANT + cequipment_variant::DEFINITION,
            &(EQUIPMENT_TYPE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EQUIPMENT_TYPE,
            &(engine::cequipment_type::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EQUIPMENT_TYPE + engine::cequipment_type::ARCHETYPE,
            &(EQUIPMENT_TYPE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EQUIPMENT_TYPE + engine::cequipment_type::MAX_MIL_FACTORIES,
            &[150, 0, 0, 0, 1],
        );
        // Variant identity.
        put(
            &mut bytes,
            VARIANT,
            &(cequipment_variant::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            VARIANT + cequipment_variant::TYPE,
            &0xfedc_ba98_0000_0046_u64.to_le_bytes(),
        );
        // Licence record.
        put(
            &mut bytes,
            RECORD,
            &(cequipment_production_license::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            RECORD + cequipment_production_license::OWNER,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            RECORD + cequipment_production_license::UNSERIALIZED_OPERAND,
            &100_000_i64.to_le_bytes(),
        );
        for (offset, value) in [
            (cequipment_production_license::LENDED_CIC, 3_i32),
            (cequipment_production_license::REQUIRED_CIC, 4_i32),
            (cequipment_production_license::GIVER, 5),
            (cequipment_production_license::START_DATE, 6),
        ] {
            put(&mut bytes, RECORD + offset, &value.to_le_bytes());
        }
        put(
            &mut bytes,
            RECORD + cequipment_production_license::PARENT,
            &(VARIANT as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            RECORD + cequipment_production_license::EQUIPMENT,
            RLIST,
            1,
        );
        put(&mut bytes, RLIST, &(VARIANT as u64).to_le_bytes());
        // Deployment manager + queue (mirrors the deployment walk fixture).
        put(
            &mut bytes,
            DEPLOY,
            &(cdeployment_status::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOY + cdeployment_status::OWNER,
            &(COUNTRY as u64).to_le_bytes(),
        );
        array(&mut bytes, DEPLOY + cdeployment_status::CONVEYORS, QUEUE, 1);
        put(&mut bytes, QUEUE, &(CONVEYOR as u64).to_le_bytes());
        put(
            &mut bytes,
            CONVEYOR,
            &(cmilitary_deployment_conveyor::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::TYPE,
            &0xfedc_ba98_0000_0040_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::TEMPLATE,
            &(TEMPLATE as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::NAME,
            b"Panzer\0",
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::NAME + msvc_string::LENGTH,
            &6_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::NAME + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::AMOUNT,
            &(-3_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::LOCATION,
            &9_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::PRIORITY,
            &7_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::ROLE_TOKEN,
            &0_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::CLOSED,
            &[0xaa],
        );
        array(
            &mut bytes,
            CONVEYOR + cmilitary_deployment_conveyor::LINES,
            LINE_ARRAY,
            1,
        );
        put(&mut bytes, LINE_ARRAY, &(LINE as u64).to_le_bytes());
        put(
            &mut bytes,
            TEMPLATE,
            &(creferenced_division_template::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            TEMPLATE + creferenced_division_template::TYPE,
            &0x0000_0021_0000_0034_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE,
            &(cmilitary_deployment_line::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::TYPE,
            &0x0000_00c8_0000_003f_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::NAME,
            &(NAME_OBJECT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::AMOUNT,
            &(-1_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::DEPLOYMENT,
            &(DEPLOYMENT as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::PARENT,
            &(CONVEYOR as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            NAME_OBJECT,
            &(cname_group_member::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            NAME_OBJECT + cname_group_member::ORDER,
            &11_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::LENGTH,
            &0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::CAPACITY,
            &msvc_string::INLINE_CAPACITY.to_le_bytes(),
        );
        put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_A, &[1]);
        put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_B, &[0]);
        put(
            &mut bytes,
            DEPLOYMENT,
            &(cmilitary_deployment::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::SECONDARY_VTABLE,
            &(cmilitary_deployment::SECONDARY_VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL,
            &(cequipment_variant_pool::VTABLE_RVA as u64).to_le_bytes(),
        );
        array(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            EQUIPMENT,
            1,
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::EQUIPMENT_POOL + cequipment_variant_pool::ALLOW_ZERO,
            &[1],
        );
        array(
            &mut bytes,
            DEPLOYMENT
                + cmilitary_deployment::EQUIPMENT_POOL
                + cequipment_variant_pool::ALLOCATIONS,
            ALLOCATIONS,
            0,
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::DISTRIBUTABLE_PRIORITY,
            &2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::HALTING_REASON,
            &3_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::HALTING_REASON_PRESENT,
            &[1],
        );
        put(&mut bytes, EQUIPMENT, &(VARIANT as u64).to_le_bytes());
        put(&mut bytes, EQUIPMENT + 8, &55_i64.to_le_bytes());
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::TRAINING,
            &0_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::MAX_TRAINING,
            &66_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::MAX_MANPOWER,
            &77_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::OWNING_LINE,
            &(LINE as u64).to_le_bytes(),
        );
        for (offset, container, count) in [
            (cmilitary_deployment::MANPOWER_HAVE, HAVE, 1_u32),
            (cmilitary_deployment::MANPOWER_NEED, NEED, 0),
        ] {
            put(
                &mut bytes,
                DEPLOYMENT + offset,
                &(carmy_manpower_values::VTABLE_RVA as u64).to_le_bytes(),
            );
            array(
                &mut bytes,
                DEPLOYMENT + offset + carmy_manpower_values::DATA,
                container,
                count,
            );
        }
        put(&mut bytes, HAVE, &8_u32.to_le_bytes());
        put(&mut bytes, HAVE + 4, &(-5_i32).to_le_bytes());
        bytes
    }

    pub(crate) fn capacities() -> Capacities {
        Capacities {
            rows: [8; N],
            definition_bytes: 64,
            conveyor_name_bytes: 64,
            line_custom_name_bytes: 64,
            license_handles: 64,
        }
    }

    pub(crate) fn direct(memory: &impl engine::Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities);
        let divisions = Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 },
        };
        let _ = captured.acquire(memory, 0, BASE, divisions, 0, true);
        captured.rest_raw_mode = false;
        captured
    }

    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, ProductionRestCopyReport, ProductionRestJob) {
        let mut captured = Captured::with_capacities(capacities);
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: BASE,
        };
        let mut first = ProductionRestJob::new();
        let report =
            crate::extended::production::copy_production_rest_refresh(memory, ctx, &mut first);
        assert!(report.raw_ok);
        captured.rest_raw_mode = report.raw_ok;
        // Typed walks skip while staged; the worker fills the buffers.
        let divisions = Sources {
            rows: &[],
            outcome: Outcome::Rows { len: 0 },
        };
        let _ = captured.acquire(memory, 0, BASE, divisions, 0, true);
        let mut ticks = [0u64; 8];
        captured.decode_production_rest_payload(0, BASE, &first, &mut ticks);
        (captured, report, first)
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(split.lines.outcome, direct.lines.outcome, "lines outcome");
        assert_eq!(
            split.stockpiles.outcome, direct.stockpiles.outcome,
            "stockpiles outcome"
        );
        assert_eq!(
            split.licenses.outcome, direct.licenses.outcome,
            "licences outcome"
        );
        assert_eq!(
            split.conveyors.outcome, direct.conveyors.outcome,
            "conveyors outcome"
        );
        assert_eq!(
            split.deployment_lines.outcome, direct.deployment_lines.outcome,
            "lines outcome"
        );
        assert_eq!(
            split.deployment_manpower.outcome, direct.deployment_manpower.outcome,
            "manpower outcome"
        );
        assert_eq!(
            split.deployment_equipment.outcome, direct.deployment_equipment.outcome,
            "equipment outcome"
        );
        assert_eq!(
            split.license_records.outcome, direct.license_records.outcome,
            "records outcome"
        );
        let len = split.lines.outcome.len();
        assert_eq!(
            &split.lines.rows[..len],
            &direct.lines.rows[..len],
            "line rows"
        );
        let len = split.stockpiles.outcome.len();
        assert_eq!(
            &split.stockpiles.rows[..len],
            &direct.stockpiles.rows[..len],
            "stockpile rows"
        );
        let len = split.conveyors.outcome.len();
        assert_eq!(
            &split.conveyors.rows[..len],
            &direct.conveyors.rows[..len],
            "conveyor rows"
        );
        let len = split.license_records.outcome.len();
        assert_eq!(
            &split.license_records.rows[..len],
            &direct.license_records.rows[..len],
            "record rows"
        );
        assert_eq!(
            &split.conveyor_name_bytes.first[..split.conveyor_name_bytes.len],
            &direct.conveyor_name_bytes.first[..direct.conveyor_name_bytes.len],
            "conveyor names"
        );
        assert_eq!(
            &split.license_handles.first[..split.license_handles.len],
            &direct.license_handles.first[..direct.license_handles.len],
            "licence handles"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_daily_groups_skip_native_reads_outside_their_slots() {
        use production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::CountedMemory::new(engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        });
        let divisions = [cequipment_variant_pool::DivisionSource {
            pointer: kit::VARIANT,
            division: 0x1234_0000_001a,
            template: 1,
            country: 2,
        }];
        let mut captured = Captured::with_capacities(&kit::capacities());
        assert!(captured.division_details_active());
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        for (hour, force_full) in (-24_i32..24).map(|hour| (hour, false)).chain([(0, true)]) {
            let rest_active = force_full || hour.rem_euclid(24) == 8;
            let divisions_active = force_full || hour.rem_euclid(24) == 2;
            captured.rest_raw_mode = !rest_active;
            captured.divisions_raw_mode = !divisions_active;
            let native = read_accounting::Capture::new();
            let ticks = captured.acquire(
                &memory,
                0,
                kit::BASE,
                Sources {
                    rows: &divisions,
                    outcome: Outcome::Rows { len: 1 },
                },
                hour,
                force_full,
            );
            let counts = native.finish();
            assert_eq!(captured.division_details_active(), divisions_active);
            for (active, indices) in [
                (rest_active, &[0, 1, 11, 12, 13, 14, 15, 16][..]),
                (divisions_active, &[2, 7, 8, 9, 10][..]),
            ] {
                for &index in indices {
                    if active {
                        assert_ne!(captured.outcomes()[index], skipped, "{hour}: {index}");
                    } else {
                        assert_eq!(captured.outcomes()[index], skipped, "{hour}: {index}");
                        assert_eq!(ticks[index], 0);
                        assert_eq!(
                            counts[TABLES[index].index()],
                            engine::memory::ReadCounts::ZERO
                        );
                    }
                }
            }
            assert_eq!(counts[TABLES[0].index()].spans > 0, rest_active);
            assert_eq!(counts[TABLES[2].index()].spans > 0, divisions_active);
        }
    }

    #[test]
    fn variant_definitions_read_hourly_while_details_wait_for_their_slot() {
        use production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::CountedMemory::new(engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        });
        let registry = [cequipment_variant::RegistryRow::read(&memory.0, 0, kit::VARIANT).unwrap()];
        let mut captured = Captured::with_capacities(&kit::capacities());
        let skipped = Outcome::Unavailable {
            reason: replay::cadence::SKIPPED_REASON,
        };
        for (hour, force_full) in (-24_i32..24).map(|hour| (hour, false)).chain([(0, true)]) {
            let details_active = force_full || hour.rem_euclid(24) == 22;
            let native = read_accounting::Capture::new();
            let ticks = captured.variants.capture(
                &memory,
                0,
                Sources {
                    rows: &registry,
                    outcome: Outcome::Rows { len: 1 },
                },
                hour,
                force_full,
            );
            let counts = native.finish();
            assert_ne!(captured.variants.definitions.outcome, skipped);
            assert!(counts[TABLES[3].index()].spans > 0);
            assert_eq!(counts[TABLES[4].index()].spans > 0, details_active);
            for (index, table) in TABLES.iter().enumerate().take(7).skip(4) {
                if details_active {
                    assert_ne!(captured.outcomes()[index], skipped);
                } else {
                    assert_eq!(captured.outcomes()[index], skipped);
                    assert_eq!(ticks[index], 0);
                    assert_eq!(counts[table.index()], engine::memory::ReadCounts::ZERO);
                }
            }
        }
    }

    #[test]
    fn production_rest_terminal_failure_preserves_variant_and_division_tables() {
        let mut captured = Captured::with_capacities(&production_rest_test_kit::capacities());
        captured.rest_raw_mode = true;
        captured.variants.definitions.outcome = Outcome::Rows { len: 1 };
        captured.division_equipment.outcome = Outcome::Rows { len: 1 };
        captured.fail_production_rest_capture("accepted acquisition failed");
        assert!(!captured.rest_raw_mode);
        assert_eq!(
            captured.variants.definitions.outcome,
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            captured.division_equipment.outcome,
            Outcome::Rows { len: 1 }
        );
        for index in [0, 1, 11, 12, 13, 14, 15, 16] {
            assert_eq!(
                captured.outcomes()[index],
                Outcome::Unavailable {
                    reason: "accepted acquisition failed"
                }
            );
        }
    }

    #[test]
    fn production_rest_job_capacity_accounting_tracks_all_vectors_through_growth_clear_and_drop() {
        use crate::raw_memory::{Capacity, Charge, Tracker, vector_bytes};
        static LOCAL: Tracker = Tracker::new();
        let mut job = ProductionRestJob::default();
        job.census.reserve_exact(3);
        job.capacity_charge = Charge::empty_on(&LOCAL);
        job.update_capacity_charge();
        let expected = |job: &ProductionRestJob| vector_bytes(&job.census);
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

    use cequipment_variant::{TextOutcome, TextSpan};
    use replay::rows::TableSchema;

    fn capacities() -> Capacities {
        Capacities {
            rows: [2; N],
            definition_bytes: 16,
            conveyor_name_bytes: 16,
            line_custom_name_bytes: 16,
            license_handles: 4,
        }
    }

    fn complete<T: TableSchema>(view: &TableView) -> &[T::Row] {
        let TableView::Complete(rows) = view else {
            panic!("expected complete rows, got {view:?}")
        };
        rows.rows::<T>().unwrap()
    }

    #[test]
    fn conversion_preserves_native_precision_full_keys_and_raw_control_bytes() {
        let mut captured = Captured::with_capacities(&capacities());
        let army = 0xfedc_ba98_0000_001a;
        let variant = 0xffff_fffe_0000_0046;
        captured.lines.outcome = Outcome::Rows { len: 1 };
        captured.lines.rows[0] = cequipment_production_line::Row {
            handle: u64::MAX,
            variant,
            mio: u64::MAX - 1,
            cost_raw: i64::MIN,
            speed_raw: -100_001,
            produced_raw: i64::MAX,
            priority: i32::MIN,
            active_factories: -1,
            requested_factories: i32::MAX,
            max_factories: i32::MIN,
            efficiency_raw: i64::MIN,
            ..Default::default()
        };
        captured.stockpiles.outcome = Outcome::Rows { len: 1 };
        captured.stockpiles.rows[0] = cequipment_variant_pool::StockpileRow {
            country: u32::MAX,
            ordinal: u32::MAX,
            variant_handle: variant,
            amount_raw: -100_001,
        };
        captured.division_equipment.outcome = Outcome::Rows { len: 1 };
        captured.division_equipment.rows[0] = cequipment_variant_pool::DivisionRow {
            division: army,
            template: u64::MAX - 2,
            country: u32::MAX,
            ordinal: u32::MAX,
            variant_handle: variant,
            amount_raw: i64::MIN,
        };
        captured.groups.outcome = Outcome::Rows { len: 1 };
        captured.groups.rows[0] = carmy_upgrades_requests::Row {
            country: u32::MAX,
            army,
            priority: u32::MAX,
        };
        captured.requests.outcome = Outcome::Rows { len: 1 };
        captured.requests.rows[0] = army_upgrade_request_row::Row {
            country: u32::MAX,
            army,
            ordinal: u32::MAX,
            equipment: variant,
            request_raw: i64::MIN,
            total_raw: i64::MAX,
        };
        captured.deliveries.outcome = Outcome::Rows { len: 1 };
        captured.deliveries.rows[0] = cupgrade_delivery::Row {
            country: u32::MAX,
            army,
            ordinal: u32::MAX,
            status: u32::MAX,
            progress_raw: Some(i64::MIN),
            total_progress_raw: i64::MAX,
            allow_zero_entries: 0xff,
        };
        captured.produced.outcome = Outcome::Rows { len: 1 };
        captured.produced.rows[0] = cupgrade_delivery::ProducedRow {
            country: u32::MAX,
            army,
            delivery_ordinal: u32::MAX,
            ordinal: u32::MAX,
            equipment: variant,
            amount_raw: i64::MIN,
        };
        captured.variants.stats.outcome = Outcome::Rows { len: 1 };
        captured.variants.stats.rows[0] = cequipment_variant::StatsRow {
            variant,
            values: [i64::MIN, -100_001, -1, 0, 1, 100_001, 7, 8, 9, i64::MAX],
        };
        let views = captured.tables();
        let line = &complete::<tables::ProductionLines>(&views[0])[0];
        assert_eq!(
            (line.handle, line.variant, line.mio),
            (u64::MAX, variant, u64::MAX - 1)
        );
        assert_eq!(
            (line.cost_raw, line.speed_raw, line.produced_raw),
            (i64::MIN, -100_001, i64::MAX)
        );
        assert_eq!(
            (
                line.priority,
                line.active_factories,
                line.requested_factories
            ),
            (i32::MIN, -1, i32::MAX)
        );
        assert_eq!(
            (line.max_factories, line.efficiency_raw),
            (i32::MIN, i64::MIN)
        );
        let stock = &complete::<tables::Stockpiles>(&views[1])[0];
        assert_eq!(
            (stock.key(), stock.amount_raw),
            (u64::MAX as u128, -100_001)
        );
        let equipment = &complete::<tables::DivisionEquipment>(&views[2])[0];
        assert_eq!(
            equipment.key(),
            (u128::from(army) << 32) | u128::from(u32::MAX)
        );
        assert_eq!(
            (equipment.template, equipment.country, equipment.amount_raw),
            (u64::MAX - 2, u32::MAX, i64::MIN)
        );
        assert_eq!(
            complete::<tables::VariantStats>(&views[5])[0].values,
            captured.variants.stats.rows[0].values
        );
        assert_eq!(
            complete::<tables::ArmyUpgradeGroups>(&views[7])[0].priority,
            u32::MAX
        );
        let request = &complete::<tables::ArmyUpgradeRequests>(&views[8])[0];
        assert_eq!(
            (request.equipment, request.request_raw, request.total_raw),
            (variant, i64::MIN, i64::MAX)
        );
        let delivery = &complete::<tables::ArmyUpgradeDeliveries>(&views[9])[0];
        assert_eq!(
            (
                delivery.status,
                delivery.progress_raw,
                delivery.allow_zero_entries
            ),
            (u32::MAX, Some(i64::MIN), 0xff)
        );
        let produced = &complete::<tables::ArmyUpgradeProduced>(&views[10])[0];
        assert_eq!(
            produced.key(),
            (u128::from(army) << 64) | u128::from(u64::MAX)
        );
        assert_eq!(produced.amount_raw, i64::MIN);

        // Daily detail projections materialize their complete recovery rows,
        // preserving the same full-width keys, including all three produced
        // key components, even when a complete encoder baseline exists.
        let mut baseline = std::array::from_fn(|_| unavailable("not part of this fixture"));
        for (id, view) in TABLES.into_iter().zip(views) {
            baseline[id.index()] = view;
        }
        let mut encoder = replay::frame::FrameEncoder::new();
        encoder
            .encode(
                replay::frame::DecodedFrame {
                    branch: 1,
                    hour: 0,
                    tables: baseline.map(Into::into),
                    metrics: replay::frame::Metrics::default(),
                },
                false,
            )
            .unwrap();
        let mut scratch = crate::projection::ProjectionScratch::default();
        let mut projection = Projection::new(&encoder, 1, 1, false, &mut scratch);
        let original_inputs = captured.table_inputs(None);
        let inputs = captured.table_inputs(Some(&mut projection));
        for index in [2, 7, 8, 9, 10] {
            assert_eq!(inputs[index], original_inputs[index]);
            assert!(matches!(
                inputs[index],
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
        captured.division_equipment.rows[0].template ^= 1;
        captured.groups.rows[0].priority ^= 1;
        captured.requests.rows[0].total_raw ^= 1;
        captured.deliveries.rows[0].progress_raw = None;
        captured.produced.rows[0].amount_raw ^= 1;
        let inputs = captured.table_inputs(Some(&mut projection));
        let converted = captured.table_inputs(None);
        for index in [2, 7, 8, 9, 10] {
            assert_eq!(inputs[index], converted[index]);
            assert_ne!(inputs[index], original_inputs[index]);
            assert!(matches!(
                inputs[index],
                TableInput::Sampled(TableView::Complete(_))
            ));
        }
    }

    #[test]
    fn licences_and_deployment_conversion_keep_order_names_and_signed_extrema() {
        let mut captured = Captured::with_capacities(&capacities());
        let line = 0x0000_00c8_0000_003f_u64;
        let conveyor = 0xfedc_ba98_0000_0040_u64;
        captured.licenses.outcome = Outcome::Rows { len: 2 };
        captured.licenses.rows[0] = cproduction_status::LicenseRow {
            country: u32::MAX,
            ordinal: 0,
            variant: u64::MAX,
        };
        captured.licenses.rows[1] = cproduction_status::LicenseRow {
            country: u32::MAX,
            ordinal: 1,
            variant: u64::MAX,
        };
        captured.conveyors.outcome = Outcome::Rows { len: 1 };
        captured.conveyor_name_bytes.first[..6].copy_from_slice(b"Panzer");
        captured.conveyor_name_bytes.len = 6;
        captured.conveyors.rows[0] = cdeployment_status::ConveyorRow {
            handle: conveyor,
            country: u32::MAX,
            ordinal: u32::MAX,
            template: u64::MAX - 1,
            amount: i32::MIN,
            location: u32::MAX,
            priority: u32::MAX,
            role_token: 0,
            closed: 0xaa,
            name_start: 0,
            name_len: 6,
            exile_country: Some(u32::MAX),
        };
        captured.deployment_lines.outcome = Outcome::Rows { len: 1 };
        captured.line_custom_name_bytes.first[..6].copy_from_slice(b"Custom");
        captured.line_custom_name_bytes.len = 6;
        captured.deployment_lines.rows[0] = cdeployment_status::LineRow {
            handle: line,
            conveyor,
            country: u32::MAX,
            ordinal: u32::MAX,
            name_order: i32::MIN,
            amount: i32::MAX,
            training_raw: 0,
            max_training_raw: i64::MIN,
            max_manpower: i32::MIN,
            allow_zero_entries: 0xff,
            equipment_allocations: i32::MAX,
            distributable_priority: i32::MIN,
            halting_reason_present: 0xaa,
            halting_reason: i32::MIN,
            custom_name_start: 0,
            custom_name_len: 6,
            custom_flag_a: 1,
            custom_flag_b: 0xbb,
        };
        captured.deployment_manpower.outcome = Outcome::Rows { len: 2 };
        captured.deployment_manpower.rows[0] = cdeployment_status::ManpowerRow {
            line,
            side: 0,
            ordinal: u32::MAX,
            source_country: 0,
            amount: i32::MIN,
        };
        captured.deployment_manpower.rows[1] = cdeployment_status::ManpowerRow {
            line,
            side: 1,
            ordinal: u32::MAX,
            source_country: u32::MAX,
            amount: i32::MAX,
        };
        captured.deployment_equipment.outcome = Outcome::Rows { len: 1 };
        captured.deployment_equipment.rows[0] = cdeployment_status::EquipmentRow {
            line,
            ordinal: u32::MAX,
            variant_handle: u64::MAX,
            amount_raw: i64::MIN,
        };

        let views = captured.tables();
        let licenses = complete::<tables::ProductionLicenses>(&views[11]);
        assert_eq!(licenses.len(), 2);
        assert_eq!(
            (licenses[0].key(), licenses[1].key()),
            (
                (u128::from(u32::MAX) << 32),
                (u128::from(u32::MAX) << 32) | 1
            )
        );
        assert_eq!(licenses[0].variant, u64::MAX);
        let entry = &complete::<tables::DeploymentConveyors>(&views[12])[0];
        assert_eq!(entry.key(), u128::from(conveyor));
        assert_eq!(entry.name, b"Panzer");
        assert_eq!(entry.exile_country, Some(u32::MAX));
        assert_eq!(
            (
                entry.amount,
                entry.location,
                entry.priority,
                entry.role_token,
                entry.closed
            ),
            (i32::MIN, u32::MAX, u32::MAX, 0, 0xaa)
        );
        let queued = &complete::<tables::DeploymentLines>(&views[13])[0];
        assert_eq!(queued.key(), u128::from(line));
        assert_eq!(queued.custom_name, b"Custom");
        assert_eq!((queued.custom_flag_a, queued.custom_flag_b), (1, 0xbb));
        assert_eq!(
            (
                queued.name_order,
                queued.training_raw,
                queued.max_training_raw,
                queued.max_manpower,
                queued.allow_zero_entries
            ),
            (i32::MIN, 0, i64::MIN, i32::MIN, 0xff)
        );
        assert_eq!(
            (
                queued.equipment_allocations,
                queued.distributable_priority,
                queued.halting_reason_present,
                queued.halting_reason
            ),
            (i32::MAX, i32::MIN, 0xaa, i32::MIN)
        );
        assert_eq!(
            (
                queued.equipment_allocations,
                queued.distributable_priority,
                queued.halting_reason_present,
                queued.halting_reason
            ),
            (i32::MAX, i32::MIN, 0xaa, i32::MIN)
        );
        let manpower = complete::<tables::DeploymentManpower>(&views[14]);
        assert_eq!(
            (manpower[0].key(), manpower[1].key()),
            (
                (u128::from(line) << 33) | u128::from(u32::MAX),
                (u128::from(line) << 33) | (1 << 32) | u128::from(u32::MAX)
            )
        );
        assert_eq!(
            (manpower[0].amount, manpower[1].amount),
            (i32::MIN, i32::MAX)
        );
        captured.license_records.outcome = Outcome::Rows { len: 1 };
        captured.license_handles.first[..2].copy_from_slice(&[u64::MAX, 0]);
        captured.license_handles.len = 2;
        captured.license_records.rows[0] = cproduction_status::LicenseRecordRow {
            owner: u32::MAX,
            ordinal: 0,
            giver: 7,
            lended_cic: i32::MIN,
            required_cic: i32::MAX,
            start_date: i32::MIN,
            parent: u64::MAX - 3,
            operand_raw: i64::MIN,
            equipment_start: 0,
            equipment_len: 2,
        };
        let carried = &complete::<tables::DeploymentEquipment>(&views[15])[0];
        assert_eq!(
            carried.key(),
            (u128::from(line) << 32) | u128::from(u32::MAX)
        );
        assert_eq!(carried.amount_raw, i64::MIN);
        let views = captured.tables();
        let record = &complete::<tables::ProductionLicenseRecords>(&views[16])[0];
        assert_eq!(record.key(), u128::from(u32::MAX) << 32);
        assert_eq!(
            (
                record.giver,
                record.lended_cic,
                record.required_cic,
                record.start_date,
                record.parent
            ),
            (7, i32::MIN, i32::MAX, i32::MIN, u64::MAX - 3)
        );
        assert_eq!(record.operand_raw, i64::MIN);
        assert_eq!(record.operand_raw, i64::MIN);
        assert_eq!(record.equipment, vec![u64::MAX, 0]);
    }

    #[test]
    fn a_licence_handle_span_outside_the_arena_fails_only_its_own_table() {
        let mut captured = Captured::with_capacities(&capacities());
        captured.license_records.outcome = Outcome::Rows { len: 1 };
        captured.license_records.rows[0] = cproduction_status::LicenseRecordRow {
            equipment_start: 1,
            equipment_len: 99,
            ..Default::default()
        };
        captured.license_handles.len = 2;
        captured.licenses.outcome = Outcome::Rows { len: 0 };
        let views = captured.tables();
        assert!(matches!(views[16], TableView::Unavailable { .. }));
        assert!(matches!(views[11], TableView::Complete(_)));
    }

    #[test]
    fn a_conveyor_name_outside_the_arena_fails_only_its_own_table() {
        let mut captured = Captured::with_capacities(&capacities());
        captured.conveyors.outcome = Outcome::Rows { len: 1 };
        captured.conveyors.rows[0] = cdeployment_status::ConveyorRow {
            handle: 1,
            name_start: 4,
            name_len: 99,
            ..Default::default()
        };
        captured.conveyor_name_bytes.len = 6;
        captured.deployment_lines.outcome = Outcome::Rows { len: 0 };
        let views = captured.tables();
        assert!(matches!(views[12], TableView::Unavailable { .. }));
        assert!(matches!(views[13], TableView::Complete(_)));
    }

    #[test]
    fn a_line_custom_name_outside_the_arena_fails_only_its_own_table() {
        let mut captured = Captured::with_capacities(&capacities());
        captured.conveyors.outcome = Outcome::Rows { len: 0 };
        captured.deployment_lines.outcome = Outcome::Rows { len: 1 };
        captured.deployment_lines.rows[0] = cdeployment_status::LineRow {
            handle: 1,
            custom_name_start: 4,
            custom_name_len: 99,
            ..Default::default()
        };
        captured.line_custom_name_bytes.len = 6;
        let views = captured.tables();
        assert!(matches!(views[12], TableView::Complete(_)));
        assert!(matches!(views[13], TableView::Unavailable { .. }));
    }

    #[test]
    fn arenas_preserve_bytes_and_native_ordinals_and_refuse_out_of_bounds_spans() {
        let mut captured = Captured::with_capacities(&capacities());
        captured.variants.definition_bytes.first[..4].copy_from_slice(&[0xff, 0, 0x80, b'A']);
        captured.variants.definition_bytes.len = 4;
        captured.variants.definitions.outcome = Outcome::Rows { len: 1 };
        captured.variants.definitions.rows[0] = cequipment_variant::DefinitionRow {
            identity: cequipment_variant::Row {
                id: u64::MAX,
                equipment_token: u32::MAX,
                append_variant_name: 0xff,
                obsolete: 0x80,
                classifier_flags: u64::MAX,
                ..Default::default()
            },
            display_name: TextSpan { start: 0, len: 2 },
            variant_name: TextSpan { start: 2, len: 2 },
            explicit_entity: TextSpan { start: 3, len: 1 },
            icon_active: 0xfe,
            icon_kind: u32::MAX,
            icon: TextSpan { start: 1, len: 3 },
        };
        captured.variants.modules.outcome = Outcome::Rows { len: 2 };
        captured.variants.modules.rows[0] = equipment_module_container::Row {
            variant: u64::MAX,
            source_ordinal: 7,
            slot: 3,
            definition_token: u32::MAX,
            definition_active: 0xfe,
        };
        captured.variants.modules.rows[1] = equipment_module_container::Row {
            source_ordinal: 1,
            slot: 9,
            ..captured.variants.modules.rows[0]
        };
        captured.variants.upgrades.outcome = Outcome::Rows { len: 1 };
        captured.variants.upgrades.rows[0] = cequipment_upgrades_instance::Row {
            variant: u64::MAX,
            source_ordinal: u32::MAX,
            definition_token: i32::MIN,
            level: u8::MAX,
        };
        let views = captured.tables();
        let definition = &complete::<tables::VariantDefinitions>(&views[3])[0];
        assert_eq!(
            (&definition.display_name[..], &definition.variant_name[..]),
            (&[0xff, 0][..], &[0x80, b'A'][..])
        );
        assert_eq!(definition.explicit_entity, b"A");
        assert_eq!(
            (
                definition.icon_active,
                definition.icon_kind,
                &definition.icon[..]
            ),
            (0xfe, u32::MAX, &[0, 0x80, b'A'][..])
        );
        assert_eq!(
            (
                definition.equipment_token,
                definition.append_variant_name,
                definition.obsolete
            ),
            (u32::MAX, 0xff, 0x80)
        );
        let modules = complete::<tables::VariantModules>(&views[4]);
        assert_eq!((modules[0].source_ordinal, modules[0].slot), (1, 9));
        assert_eq!((modules[1].source_ordinal, modules[1].slot), (7, 3));
        assert_eq!(modules[0].definition_active, 0xfe);
        let upgrade = &complete::<tables::VariantUpgrades>(&views[6])[0];
        assert_eq!(
            (upgrade.definition_token, upgrade.level),
            (i32::MIN, u8::MAX)
        );
        captured.variants.definitions.rows[0].variant_name = TextSpan {
            start: usize::MAX,
            len: 2,
        };
        assert!(matches!(
            captured.tables()[3],
            TableView::Unavailable { .. }
        ));
        captured.reset();
        assert!(
            captured
                .tables()
                .iter()
                .all(|table| matches!(table, TableView::Unavailable { .. }))
        );
    }

    #[test]
    fn complete_census_distinguishes_empty_success_invalid_count_and_source_fault() {
        let source = [10usize, 20];
        assert_eq!(
            complete_source(
                &source,
                Outcome::Truncated {
                    len: 2,
                    unread_rows: 1
                }
            ),
            Err("source census incomplete")
        );
        assert_eq!(
            complete_source(&source, Outcome::Rows { len: 3 }),
            Err("source row count exceeds acquired buffer")
        );
        assert_eq!(
            complete_source(&source, Outcome::Rows { len: 0 }),
            Ok(&[][..])
        );
        let mut rows = CaptureRows::<u32>::new(2);
        let mut arena = Arena::new(2);
        let mut calls = 0;
        capture_text(&mut rows, &mut arena, Err("failed census"), |_, _, _| {
            calls += 1;
            unreachable!()
        });
        assert_eq!(calls, 0);
        assert_eq!(
            rows.outcome,
            Outcome::Unavailable {
                reason: "failed census"
            }
        );
        assert_eq!((arena.len, arena.required), (0, 0));
    }

    #[test]
    fn division_details_require_a_complete_current_source_before_reading() {
        struct Unreadable;
        impl Memory for Unreadable {
            fn copy(&self, _: usize, _: &mut [u8]) -> bool {
                panic!("incomplete census must not reach live detail reads")
            }
        }
        let source = [cequipment_variant_pool::DivisionSource {
            pointer: 0x1000,
            division: 0x1234_0000_001a,
            template: 1,
            country: 7,
        }];
        let mut captured = Captured::with_capacities(&capacities());
        captured.rest_raw_mode = true;
        for (outcome, reason) in [
            (
                Outcome::Unavailable {
                    reason: "division parent unreadable",
                },
                "division parent unreadable",
            ),
            (
                Outcome::Truncated {
                    len: 1,
                    unread_rows: 1,
                },
                "source census incomplete",
            ),
            (
                Outcome::Rows { len: 2 },
                "source row count exceeds acquired buffer",
            ),
        ] {
            captured.reset();
            captured.acquire(
                &Unreadable,
                0,
                0,
                Sources {
                    rows: &source,
                    outcome,
                },
                0,
                true,
            );
            for index in [2, 7, 8, 9, 10] {
                assert_eq!(captured.outcomes()[index], Outcome::Unavailable { reason });
            }
        }
    }

    #[test]
    fn text_capture_keeps_first_bytes_and_primary_truncation_grows_off_thread() {
        let mut rows = CaptureRows::<u32>::new(2);
        let mut arena = Arena::new(2);
        let mut calls = 0;
        capture_text(&mut rows, &mut arena, Ok(&[]), |_, rows, arena| {
            calls += 1;
            rows[0] = 7;
            arena[0] = 0xff;
            TextOutcome {
                outcome: Outcome::Rows { len: 1 },
                bytes_len: 1,
                required_bytes: 1,
            }
        });
        assert_eq!(calls, 1);
        assert_eq!(rows.rows[0], 7);
        assert_eq!(arena.first[0], 0xff);
        assert_eq!(rows.outcome, Outcome::Rows { len: 1 });
        let mut capacities = capacities();
        let mut captured = Captured::with_capacities(&capacities);
        captured.stockpiles.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 5,
        };
        captured.stockpiles.rows[0].ordinal = 0;
        captured.stockpiles.rows[1].ordinal = 1;
        captured.variants.definition_bytes.required = 33;
        assert!(matches!(
            captured.tables()[1],
            TableView::Truncated { missing: 5, .. }
        ));
        captured.grow(&mut capacities);
        assert!(
            captured.stockpiles.rows.len() >= 7
                && captured.variants.definition_bytes.first.len() >= 33
        );
        let allocated = captured.allocated_bytes();
        captured.reset();
        captured.grow(&mut capacities);
        assert_eq!(captured.allocated_bytes(), allocated);
    }

    #[test]
    fn variant_loan_retains_buffers_and_adopts_worker_growth() {
        let mut capacities = capacities();
        let mut captured = Captured::with_capacities(&capacities);
        let before = captured.allocated_bytes();
        let mut variants = captured.take_variants();
        assert_eq!(captured.variants.allocated_bytes(), 0);
        assert_eq!(
            captured.allocated_bytes() + variants.allocated_bytes(),
            before
        );
        variants.definitions.rows.resize(33, Default::default());
        variants.definition_bytes.first.resize(65, 0);
        let row_capacity = variants.definitions.rows.capacity();
        let byte_capacity = variants.definition_bytes.first.capacity();
        variants.fail("variant worker stopped", 0, true);
        captured.restore_variants(variants);
        assert!(captured.outcomes()[3..7].iter().all(|outcome| {
            *outcome
                == Outcome::Unavailable {
                    reason: "variant worker stopped",
                }
        }));
        let allocated = captured.allocated_bytes();
        captured.grow(&mut capacities);
        assert_eq!(capacities.rows[3], row_capacity);
        assert_eq!(capacities.definition_bytes, byte_capacity);
        assert_eq!(captured.allocated_bytes(), allocated);
        let mut variants = captured.take_variants();
        variants.fail("variant worker stopped", 0, false);
        captured.restore_variants(variants);
        for (index, table) in TABLES.iter().enumerate().take(7).skip(3) {
            assert_eq!(
                captured.outcomes()[index],
                Outcome::Unavailable {
                    reason: if replay::cadence::should_walk(*table, 0, false) {
                        "variant worker stopped"
                    } else {
                        replay::cadence::SKIPPED_REASON
                    }
                }
            );
        }
    }

    #[test]
    fn production_rest_split_copy_matches_direct_walk() {
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, _, _) = kit::split(&memory, &capacities);
        kit::assert_matches_direct(&split, &direct);
        assert_eq!(split.lines.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.stockpiles.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.licenses.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.conveyors.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.deployment_lines.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.deployment_manpower.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.deployment_equipment.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.license_records.outcome, Outcome::Rows { len: 1 });
    }

    /// The kit's memory plus the six maximum-factory defines at their module
    /// addresses (module base zero).
    struct DefinedMemory<'a>(&'a [u8]);

    impl Memory for DefinedMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if let Some(index) = cequipment_production_line::MAX_FACTORY_DEFINE_RVAS
                .iter()
                .position(|&rva| rva == address)
            {
                destination.copy_from_slice(&(index as i32 + 3).to_le_bytes());
                return true;
            }
            engine::memory::SliceMemory {
                base: production_rest_test_kit::BASE,
                bytes: self.0,
            }
            .copy(address, destination)
        }
    }

    #[test]
    fn production_rest_split_copy_stages_line_maximums_defines_and_efficiencies() {
        use super::production_rest_test_kit as kit;
        let at = |address: usize| address - kit::BASE;
        let capacities = kit::capacities();
        let mut bytes = kit::fixture();
        let row = |bytes: &[u8]| {
            let direct = kit::direct(&DefinedMemory(bytes), &capacities);
            let (split, _, _) = kit::split(&DefinedMemory(bytes), &capacities);
            kit::assert_matches_direct(&split, &direct);
            assert_eq!(split.lines.outcome, direct.lines.outcome);
            (split.lines.outcome, split.lines.rows[0])
        };
        // 12 active of the type's 150: the average of 40..=51 percent.
        let (outcome, line) = row(&bytes);
        assert_eq!(outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            (line.max_factories, line.efficiency_raw),
            (150, (40..52).sum::<i64>() * 32_768 / 12)
        );
        // Without the type's optional the military define (index 0) applies.
        let flag = at(kit::EQUIPMENT_TYPE + engine::cequipment_type::MAX_MIL_FACTORIES + 4);
        bytes[flag] = 0;
        let (_, line) = row(&bytes);
        assert_eq!(line.max_factories, 3);
        assert_eq!(line.efficiency_raw, (40 + 41 + 42) * 32_768 / 3);
        // A naval line with the capital-ship bit takes that define (index 4).
        let vtable = at(kit::PLINE);
        bytes[vtable..vtable + 8].copy_from_slice(
            &(engine::cnaval_production_line::VTABLE_RVA as u64).to_le_bytes(),
        );
        let flags = at(kit::VARIANT + cequipment_variant::CLASSIFIER_FLAGS);
        bytes[flags..flags + 8]
            .copy_from_slice(&cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK.to_le_bytes());
        let (_, line) = row(&bytes);
        assert_eq!(line.max_factories, 7);
        // An efficiency array shorter than the clamped factories fails alike.
        let count = at(kit::PLINE + cequipment_production_line::FACTORY_EFFICIENCIES + 12);
        bytes[count..count + 4].copy_from_slice(&6_u32.to_le_bytes());
        let (outcome, _) = row(&bytes);
        assert_eq!(
            outcome,
            Outcome::Unavailable {
                reason: "production line efficiency array shorter than its factories"
            }
        );
    }

    struct PlanFixtureMemory<'a> {
        bytes: &'a [u8],
        calls: std::cell::RefCell<Vec<(usize, usize)>>,
        fail: Option<usize>,
    }

    impl Memory for PlanFixtureMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.calls.borrow_mut().push((address, destination.len()));
            if self.fail == Some(address) {
                if let Some(byte) = destination.first_mut() {
                    *byte = 0xEC;
                }
                return false;
            }
            engine::memory::SliceMemory {
                base: production_rest_test_kit::BASE,
                bytes: self.bytes,
            }
            .copy(address, destination)
        }
    }

    fn fixture_plan_context() -> ProductionRestCopyContext {
        ProductionRestCopyContext {
            base: 0,
            game_state: production_rest_test_kit::BASE,
        }
    }

    #[test]
    fn production_plan_copies_changed_payload_without_rebuilding_discovery() {
        use super::production_rest_test_kit as kit;
        let mut bytes = kit::fixture();
        let mut job = ProductionRestJob::new();
        let initial = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        assert!(copy_production_rest_refresh(&initial, fixture_plan_context(), &mut job).raw_ok);
        assert!(!job.census.is_empty());
        job.log.prepare_read_plan();
        let cost = kit::PLINE + cproduction_line::COST - kit::BASE;
        bytes[cost..cost + 8].copy_from_slice(&999_i64.to_le_bytes());
        let memory = engine::memory::CountedMemory::new(PlanFixtureMemory {
            bytes: &bytes,
            calls: Default::default(),
            fail: None,
        });
        let native = read_accounting::Capture::new();
        let report = copy_production_rest_refresh(&memory, fixture_plan_context(), &mut job);
        let planned_counts = native.finish();
        assert!(report.raw_ok && !report.plan_failed);
        assert!(job.log.planned_capture_succeeded());
        assert!(
            job.census.is_empty(),
            "complete plan must skip the discovery producer"
        );
        let planned_calls = memory.0.calls.take();
        let mut expected_counts = read_accounting::ZERO;
        expected_counts[TABLES[0].index()] = engine::memory::ReadCounts {
            plan_calls: 1,
            spans: planned_calls.iter().filter(|&&(_, len)| len != 0).count() as u64,
            bytes: planned_calls.iter().map(|&(_, len)| len as u64).sum(),
            ..engine::memory::ReadCounts::ZERO
        };
        assert!(expected_counts[TABLES[0].index()].spans > 0);
        assert_eq!(planned_counts, expected_counts);
        let mut canonical = ProductionRestJob::new();
        let native = read_accounting::Capture::new();
        let fresh = copy_production_rest_refresh(&memory, fixture_plan_context(), &mut canonical);
        let canonical_counts = native.finish();
        assert!(fresh.raw_ok);
        assert_eq!(planned_calls, memory.0.calls.take());
        let canonical_work = canonical_counts[TABLES[0].index()];
        assert_eq!(canonical_work.plan_calls, 0);
        assert_eq!(
            canonical_work.spans,
            planned_counts[TABLES[0].index()].spans
        );
        assert_eq!(
            canonical_work.bytes,
            planned_counts[TABLES[0].index()].bytes
        );
        expected_counts[TABLES[0].index()] = canonical_work;
        assert_eq!(canonical_counts, expected_counts);
        assert!(job.log.logs_equal(&canonical.log));
        let capacities = kit::capacities();
        let direct = kit::direct(&memory.0, &capacities);
        let mut captured = Captured::with_capacities(&capacities);
        let native = read_accounting::Capture::new();
        captured.decode_production_rest_payload(0, kit::BASE, &job, &mut [0; 8]);
        assert_eq!(native.finish(), read_accounting::ZERO);
        kit::assert_matches_direct(&captured, &direct);
        assert_eq!(captured.lines.rows[0].cost_raw, 999);
    }

    #[test]
    fn production_plan_rebuilds_changed_pointer_and_preserves_a_later_fault_once() {
        use super::production_rest_test_kit as kit;
        let mut bytes = kit::fixture();
        let mut job = ProductionRestJob::new();
        let initial = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        assert!(copy_production_rest_refresh(&initial, fixture_plan_context(), &mut job).raw_ok);
        job.log.prepare_read_plan();
        let next_variant = kit::BASE + bytes.len();
        // The identity through the type link and classifier the line reads.
        let variant = kit::VARIANT - kit::BASE;
        let identity = bytes[variant..variant + cequipment_variant::CLASSIFIER_FLAGS + 8].to_vec();
        bytes.extend_from_slice(&identity);
        let pointer = kit::PLINE + cequipment_production_line::EQUIPMENT_VARIANT - kit::BASE;
        bytes[pointer..pointer + 8].copy_from_slice(&(next_variant as u64).to_le_bytes());
        for fail in [None, Some(next_variant)] {
            let memory = PlanFixtureMemory {
                bytes: &bytes,
                calls: Default::default(),
                fail,
            };
            let report = copy_production_rest_refresh(&memory, fixture_plan_context(), &mut job);
            assert!(report.raw_ok && !report.plan_failed);
            assert!(
                !job.census.is_empty(),
                "changed pointer or fault resumes discovery"
            );
            let planned_calls = memory.calls.take();
            let mut canonical = ProductionRestJob::new();
            let fresh =
                copy_production_rest_refresh(&memory, fixture_plan_context(), &mut canonical);
            assert!(fresh.raw_ok && !fresh.plan_failed);
            assert_eq!(
                planned_calls,
                memory.calls.take(),
                "no consumed source may be retried"
            );
            assert!(job.log.logs_equal(&canonical.log));
            assert_eq!(job.log.fault[0], fail.is_some());
            assert_eq!(&job.log.fault[1..], &[false; 7]);
            if fail.is_none() {
                job.log.prepare_read_plan();
            }
        }
    }

    #[test]
    fn production_rest_truncation_matrix_matches_direct_with_same_caps() {
        // B3: zero/one/exact-fit/overflow-by-one/massive run typed AND
        // copy→decode against the SAME memory with the SAME pooled caps,
        // asserting identical outcome (incl. len/unread) and identical rows.
        // Fixture holds 1 row per rest table, so cap 1 is both the one-row
        // and exact-fit arm, cap 0 the zero/overflow-by-one/massive arm.
        // u32 unread saturation is not reachable and is covered by the
        // walk's own saturating arithmetic.
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        for rows in [[0; N], [1; N], [2; N], [64; N]] {
            let capacities = Capacities {
                rows,
                definition_bytes: 0,
                conveyor_name_bytes: 64,
                line_custom_name_bytes: 64,
                license_handles: 64,
            };
            let direct = kit::direct(&memory, &capacities);
            let mut captured = Captured::with_capacities(&capacities);
            let ctx = ProductionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
            };
            let mut first = ProductionRestJob::new();
            let report =
                crate::extended::production::copy_production_rest_refresh(&memory, ctx, &mut first);
            assert!(report.raw_ok, "rows={rows:?}");
            captured.rest_raw_mode = true;
            let divisions = Sources {
                rows: &[],
                outcome: Outcome::Rows { len: 0 },
            };
            let _ = captured.acquire(&memory, 0, kit::BASE, divisions, 0, true);
            let mut ticks = [0u64; 8];
            captured.decode_production_rest_payload(0, kit::BASE, &first, &mut ticks);
            kit::assert_matches_direct(&captured, &direct);
        }
    }

    #[test]
    fn production_rest_split_copy_matches_direct_walk_under_truncation() {
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = Capacities {
            rows: [1; N],
            definition_bytes: 0,
            conveyor_name_bytes: 2,
            line_custom_name_bytes: 0,
            license_handles: 0,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, _, _) = {
            let mut captured = Captured::with_capacities(&capacities);
            let ctx = ProductionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
            };
            let mut first = ProductionRestJob::new();
            let report =
                crate::extended::production::copy_production_rest_refresh(&memory, ctx, &mut first);
            assert!(report.raw_ok);
            captured.rest_raw_mode = true;
            let divisions = Sources {
                rows: &[],
                outcome: Outcome::Rows { len: 0 },
            };
            let _ = captured.acquire(&memory, 0, kit::BASE, divisions, 0, true);
            let mut ticks = [0u64; 8];
            captured.decode_production_rest_payload(0, kit::BASE, &first, &mut ticks);
            (captured, report, first)
        };
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn production_rest_staging_overflow_falls_back_to_typed_walk() {
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
        };
        let mut tiny = ProductionRestJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            census: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let report =
            crate::extended::production::copy_production_rest_refresh(&memory, ctx, &mut tiny);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }

    #[test]
    fn production_rest_worst_case_pools_start_above_late_campaign_caps() {
        // Late-campaign worst case: 2048 lines, 8192 stockpiles, licences and
        // deployment queues with names plus licence handles. Pools start
        // above that so steady state never overflows; overflow still falls
        // back to the typed walk above.
        let job = ProductionRestJob::new();
        assert!(job.log.spans.capacity() >= 1 << 17);
        assert!(job.log.bytes.capacity() >= (8 << 20));
        let capacities = Capacities::default();
        assert!(capacities.rows[0] >= 2048);
        assert!(capacities.rows[1] >= 8192);
        assert!(capacities.conveyor_name_bytes >= 128 * 1024);
        assert!(capacities.license_handles >= 4096);
    }
}

#[cfg(test)]
#[path = "variants_scale_tests.rs"]
mod variant_scale_tests;
