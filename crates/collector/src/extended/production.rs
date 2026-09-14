//! Production/equipment acquisition. Only construction/growth and conversion
//! allocate; the callback fills typed buffers and measures shared graph work.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
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
    frame::TableView,
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
    (
        "variant definitions, modules, stats and upgrades",
        3,
        &[3, 4, 5, 6],
    ),
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

/// Borrowed buffers owned by shared acquisition. Each pass must carry its
/// independently measured complete census, in stable order.
pub struct Sources<'a, T> {
    pub first: &'a [T],
    pub first_outcome: Outcome,
    pub second: &'a [T],
    pub second_outcome: Outcome,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub definition_bytes: usize,
    pub module_bytes: usize,
    pub upgrade_bytes: usize,
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
            module_bytes: 128 * 1024,
            upgrade_bytes: 128 * 1024,
            conveyor_name_bytes: 128 * 1024,
            line_custom_name_bytes: 16 * 1024,
            license_handles: 4096,
        }
    }
}

pub struct Captured {
    pub lines: CaptureRows<cequipment_production_line::Row>,
    pub stockpiles: CaptureRows<cequipment_variant_pool::StockpileRow>,
    pub division_equipment: CaptureRows<cequipment_variant_pool::DivisionRow>,
    pub definitions: CaptureRows<cequipment_variant::DefinitionRow>,
    pub modules: CaptureRows<equipment_module_container::Row>,
    pub stats: CaptureRows<cequipment_variant::StatsRow>,
    pub upgrades: CaptureRows<cequipment_upgrades_instance::Row>,
    pub groups: CaptureRows<carmy_upgrades_requests::Row>,
    pub requests: CaptureRows<army_upgrade_request_row::Row>,
    pub deliveries: CaptureRows<cupgrade_delivery::Row>,
    pub produced: CaptureRows<cupgrade_delivery::ProducedRow>,
    pub licenses: CaptureRows<cproduction_status::LicenseRow>,
    pub conveyors: CaptureRows<cdeployment_status::ConveyorRow>,
    pub deployment_lines: CaptureRows<cdeployment_status::LineRow>,
    pub deployment_manpower: CaptureRows<cdeployment_status::ManpowerRow>,
    pub deployment_equipment: CaptureRows<cdeployment_status::EquipmentRow>,
    pub definition_bytes: Arena<u8>,
    pub module_bytes: Arena<u8>,
    pub upgrade_bytes: Arena<u8>,
    pub conveyor_name_bytes: Arena<u8>,
    pub line_custom_name_bytes: Arena<u8>,
    pub license_records: CaptureRows<cproduction_status::LicenseRecordRow>,
    pub license_handles: Arena<u64>,
    /// Raw-copy split gate for the hourly divisions group (equipment table
    /// 2 plus the upgrade graph 7..11). Set by the game thread every hour
    /// with a valid raw payload; `acquire` then skips those typed detail
    /// walks and the decode worker fills the rows before the ready handoff.
    /// Preserved across `reset` (set per hour, cleared by the worker after
    /// decode). Lines, stockpiles, licences and deployment stay game-side.
    pub(crate) divisions_raw_mode: bool,
    /// Raw-copy split gate for the hourly production rest (lines 0,
    /// stockpiles 1, licences 11, deployment 12..15, licence records 16).
    /// Set by the game thread every hour with a valid raw payload; `acquire`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus the licence/name arenas) before the ready handoff. Preserved
    /// across `reset` (set per hour, cleared by the worker after decode).
    pub(crate) rest_raw_mode: bool,
}

impl Captured {
    pub fn with_capacities(capacities: &Capacities, successful: &[u16]) -> Self {
        assert_eq!(successful.len(), N);
        let graph_successful = *successful[7..11].iter().min().unwrap();
        let deployment = *successful[12..16].iter().min().unwrap();
        Self {
            lines: CaptureRows::new(capacities.rows[0], successful[0]),
            stockpiles: CaptureRows::new(capacities.rows[1], successful[1]),
            division_equipment: CaptureRows::new(capacities.rows[2], successful[2]),
            definitions: CaptureRows::new(capacities.rows[3], successful[3]),
            modules: CaptureRows::new(capacities.rows[4], successful[4]),
            stats: CaptureRows::new(capacities.rows[5], successful[5]),
            upgrades: CaptureRows::new(capacities.rows[6], successful[6]),
            groups: CaptureRows::new(capacities.rows[7], graph_successful),
            requests: CaptureRows::new(capacities.rows[8], graph_successful),
            deliveries: CaptureRows::new(capacities.rows[9], graph_successful),
            produced: CaptureRows::new(capacities.rows[10], graph_successful),
            licenses: CaptureRows::new(capacities.rows[11], successful[11]),
            conveyors: CaptureRows::new(capacities.rows[12], deployment),
            deployment_lines: CaptureRows::new(capacities.rows[13], deployment),
            deployment_manpower: CaptureRows::new(capacities.rows[14], deployment),
            deployment_equipment: CaptureRows::new(capacities.rows[15], deployment),
            definition_bytes: Arena::new(capacities.definition_bytes, successful[3]),
            module_bytes: Arena::new(capacities.module_bytes, successful[4]),
            upgrade_bytes: Arena::new(capacities.upgrade_bytes, successful[6]),
            conveyor_name_bytes: Arena::new(capacities.conveyor_name_bytes, deployment),
            line_custom_name_bytes: Arena::new(capacities.line_custom_name_bytes, deployment),
            license_records: CaptureRows::new(capacities.rows[16], successful[16]),
            license_handles: Arena::new(capacities.license_handles, successful[16]),
            divisions_raw_mode: false,
            rest_raw_mode: false,
        }
    }

    pub fn reset(&mut self) {
        self.lines.reset();
        self.stockpiles.reset();
        self.division_equipment.reset();
        self.definitions.reset();
        self.modules.reset();
        self.stats.reset();
        self.upgrades.reset();
        self.groups.reset();
        self.requests.reset();
        self.deliveries.reset();
        self.produced.reset();
        self.licenses.reset();
        self.conveyors.reset();
        self.deployment_lines.reset();
        self.deployment_manpower.reset();
        self.deployment_equipment.reset();
        self.definition_bytes.reset();
        self.module_bytes.reset();
        self.upgrade_bytes.reset();
        self.conveyor_name_bytes.reset();
        self.line_custom_name_bytes.reset();
        self.license_records.reset();
        self.license_handles.reset();
    }

    pub fn allocated_bytes(&self) -> usize {
        self.lines.allocated_bytes()
            + self.stockpiles.allocated_bytes()
            + self.division_equipment.allocated_bytes()
            + self.definitions.allocated_bytes()
            + self.modules.allocated_bytes()
            + self.stats.allocated_bytes()
            + self.upgrades.allocated_bytes()
            + self.groups.allocated_bytes()
            + self.requests.allocated_bytes()
            + self.deliveries.allocated_bytes()
            + self.produced.allocated_bytes()
            + self.licenses.allocated_bytes()
            + self.conveyors.allocated_bytes()
            + self.deployment_lines.allocated_bytes()
            + self.deployment_manpower.allocated_bytes()
            + self.deployment_equipment.allocated_bytes()
            + self.definition_bytes.allocated_bytes()
            + self.module_bytes.allocated_bytes()
            + self.upgrade_bytes.allocated_bytes()
            + self.conveyor_name_bytes.allocated_bytes()
            + self.line_custom_name_bytes.allocated_bytes()
            + self.license_records.allocated_bytes()
            + self.license_handles.allocated_bytes()
    }

    pub fn grow(&mut self, capacities: &mut Capacities, successful: &[u16]) {
        assert_eq!(successful.len(), N);
        self.lines.grow(&mut capacities.rows[0], successful[0]);
        self.stockpiles.grow(&mut capacities.rows[1], successful[1]);
        self.division_equipment
            .grow(&mut capacities.rows[2], successful[2]);
        self.definitions
            .grow(&mut capacities.rows[3], successful[3]);
        self.modules.grow(&mut capacities.rows[4], successful[4]);
        self.stats.grow(&mut capacities.rows[5], successful[5]);
        self.upgrades.grow(&mut capacities.rows[6], successful[6]);
        let graph = *successful[7..11].iter().min().unwrap();
        self.groups.grow(&mut capacities.rows[7], graph);
        self.requests.grow(&mut capacities.rows[8], graph);
        self.deliveries.grow(&mut capacities.rows[9], graph);
        self.produced.grow(&mut capacities.rows[10], graph);
        self.licenses.grow(&mut capacities.rows[11], successful[11]);
        let deployment = *successful[12..16].iter().min().unwrap();
        self.conveyors.grow(&mut capacities.rows[12], deployment);
        self.deployment_lines
            .grow(&mut capacities.rows[13], deployment);
        self.deployment_manpower
            .grow(&mut capacities.rows[14], deployment);
        self.deployment_equipment
            .grow(&mut capacities.rows[15], deployment);
        self.definition_bytes
            .grow(&mut capacities.definition_bytes, successful[3]);
        self.module_bytes
            .grow(&mut capacities.module_bytes, successful[4]);
        self.upgrade_bytes
            .grow(&mut capacities.upgrade_bytes, successful[6]);
        self.conveyor_name_bytes
            .grow(&mut capacities.conveyor_name_bytes, deployment);
        self.line_custom_name_bytes
            .grow(&mut capacities.line_custom_name_bytes, deployment);
        self.license_records
            .grow(&mut capacities.rows[16], successful[16]);
        self.license_handles
            .grow(&mut capacities.license_handles, successful[16]);
    }

    /// Acquire non-variant tables. Each table requests its own consistency
    /// reads; variants use the independently acquired registry census.
    /// Durations are QPC ticks and include comparison work.
    #[allow(clippy::too_many_arguments)]
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        divisions: Sources<'_, cequipment_variant_pool::DivisionSource>,
        _hour: i32,
        _force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Hourly production rest (copy/decode split): every hour the game
        // thread stages raw bytes via `copy_production_rest_refresh` and the
        // decode worker fills lines, stockpiles, licences, deployment and
        // licence records before the ready handoff, so the typed walks below
        // skip here. Matched flags and costs come from the raw compare,
        // applied by the caller after this returns.
        if !self.rest_raw_mode {
            let begin = qpc();
            matched[0] = capture_rows(&mut self.lines, needed[0], |rows| {
                cproduction_status::walk_lines(memory, base, game_state, rows)
            });
            ticks[0] = elapsed(begin);
            let begin = qpc();
            matched[1] = capture_rows(&mut self.stockpiles, needed[1], |rows| {
                cequipment_variant_pool::walk_stockpiles(memory, base, game_state, rows)
            });
            ticks[1] = elapsed(begin);
        }
        // Divisions group (copy/decode split): on hours with a staged raw
        // payload the decode worker fills equipment and upgrade rows before
        // the ready handoff, so the typed detail walks are skipped here,
        // leaving the reset outcomes for the worker. Matched flags and costs
        // come from the raw compare, applied by the caller after this
        // returns. Lines, stockpiles, licences and deployment below always
        // run game-side (independent walks).
        if !self.divisions_raw_mode {
            let begin = qpc();
            let first_sources = complete_source(divisions.first, divisions.first_outcome);
            let second_sources = complete_source(divisions.second, divisions.second_outcome);
            self.division_equipment.checked = needed[2];
            self.division_equipment.outcome = match first_sources {
                Ok(source) => cequipment_variant_pool::walk_divisions(
                    memory,
                    base,
                    source,
                    &mut self.division_equipment.rows,
                ),
                Err(reason) => Outcome::Unavailable { reason },
            };
            if needed[2] {
                let second = match second_sources {
                    Ok(source) => cequipment_variant_pool::walk_divisions(
                        memory,
                        base,
                        source,
                        &mut self.division_equipment.second,
                    ),
                    Err(reason) => Outcome::Unavailable { reason },
                };
                matched[2] = self.division_equipment.compare(second);
                if sources_changed(first_sources, second_sources) {
                    self.division_equipment.mismatches += 1;
                    matched[2] = false;
                }
            }
            ticks[2] = elapsed(begin);
            let begin = qpc();
            let result = match first_sources {
                Ok(sources) => carmy_upgrades_status::walk(
                    memory,
                    base,
                    sources,
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
            self.groups.checked = needed[7];
            self.requests.checked = needed[8];
            self.deliveries.checked = needed[9];
            self.produced.checked = needed[10];
            if needed[7..11].iter().any(|&needed| needed) {
                let result = match second_sources {
                    Ok(sources) => carmy_upgrades_status::walk(
                        memory,
                        base,
                        sources,
                        &mut self.groups.second,
                        &mut self.requests.second,
                        &mut self.deliveries.second,
                        &mut self.produced.second,
                    ),
                    Err(reason) => carmy_upgrades_status::Outcomes::unavailable(reason),
                };
                if needed[7] {
                    matched[7] = self.groups.compare(result.groups);
                }
                if needed[8] {
                    matched[8] = self.requests.compare(result.requests);
                }
                if needed[9] {
                    matched[9] = self.deliveries.compare(result.deliveries);
                }
                if needed[10] {
                    matched[10] = self.produced.compare(result.produced);
                }
                if sources_changed(first_sources, second_sources) {
                    for (index, mismatches) in [
                        (7, &mut self.groups.mismatches),
                        (8, &mut self.requests.mismatches),
                        (9, &mut self.deliveries.mismatches),
                        (10, &mut self.produced.mismatches),
                    ] {
                        if needed[index] {
                            *mismatches += 1;
                            matched[index] = false;
                        }
                    }
                }
            }
            ticks[7] = elapsed(begin);
        }

        if !self.rest_raw_mode {
            let begin = qpc();
            matched[11] = capture_rows(&mut self.licenses, needed[11], |rows| {
                cproduction_status::walk_licenses(memory, base, game_state, rows)
            });
            let first = cproduction_status::walk_license_records(
                memory,
                base,
                game_state,
                &mut self.license_records.rows,
                &mut self.license_handles.first,
            );
            self.license_records.outcome = first.outcome;
            self.license_handles.len = first.handles_len;
            self.license_handles.required = first.required_handles;
            self.license_records.checked = needed[16];
            if needed[16] {
                let second = cproduction_status::walk_license_records(
                    memory,
                    base,
                    game_state,
                    &mut self.license_records.second,
                    &mut self.license_handles.second,
                );
                // A changed produced-handle arena is a mismatch even when every
                // fixed field of every record is equal.
                let handles_matched = self
                    .license_handles
                    .compare(second.handles_len, second.required_handles);
                let rows_matched = self.license_records.compare(second.outcome);
                if !handles_matched {
                    self.license_records.mismatches += 1;
                }
                matched[16] = rows_matched && handles_matched;
            }
            ticks[11] = elapsed(begin);

            let begin = qpc();
            let deployment_needed = needed[12..16].iter().any(|needed| *needed);
            let first = self.deployment(memory, base, game_state, false);
            self.conveyors.outcome = first.conveyors;
            self.deployment_lines.outcome = first.lines;
            self.deployment_manpower.outcome = first.manpower;
            self.deployment_equipment.outcome = first.equipment;
            self.conveyor_name_bytes.len = first.name_bytes;
            self.conveyor_name_bytes.required = first.required_name_bytes;
            self.line_custom_name_bytes.len = first.custom_name_bytes;
            self.line_custom_name_bytes.required = first.required_custom_name_bytes;
            self.conveyors.checked = needed[12];
            self.deployment_lines.checked = needed[13];
            self.deployment_manpower.checked = needed[14];
            self.deployment_equipment.checked = needed[15];
            if deployment_needed {
                let second = self.deployment(memory, base, game_state, true);
                // A changed name arena is a mismatch for every deployment table,
                // including ones whose rows happen to be equal.
                let names_matched = self
                    .conveyor_name_bytes
                    .compare(second.name_bytes, second.required_name_bytes);
                let custom_matched = self
                    .line_custom_name_bytes
                    .compare(second.custom_name_bytes, second.required_custom_name_bytes);
                let arenas_matched = names_matched && custom_matched;
                let check = |index: usize, rows: &mut dyn DeploymentRows, outcome| {
                    let matched_rows = rows.compare(outcome);
                    if !arenas_matched {
                        rows.add_mismatch();
                    }
                    needed[index] && matched_rows && arenas_matched
                };
                matched[12] = check(12, &mut self.conveyors, second.conveyors);
                matched[13] = check(13, &mut self.deployment_lines, second.lines);
                matched[14] = check(14, &mut self.deployment_manpower, second.manpower);
                matched[15] = check(15, &mut self.deployment_equipment, second.equipment);
            }
            ticks[12] = elapsed(begin);
        }
        (matched, ticks)
    }

    /// Decode-worker interpretation of a raw divisions-group payload, in
    /// dependency order: equipment first, then the upgrade graph, both
    /// against the decoded division sources. Outcomes and rows match the
    /// typed walks exactly because the identical walks run against the
    /// recorded spans; checked/mismatches set by the raw compare survive.
    /// Clears `divisions_raw_mode` so the next capture starts clean.
    pub(crate) fn decode_division_detail(
        &mut self,
        base: usize,
        sources: &Sources<'_, cequipment_variant_pool::DivisionSource>,
        recorded: &crate::raw::Recorded<'_>,
        decode_ticks: &mut [u64; 6],
    ) {
        let started = qpc();
        let first_sources = complete_source(sources.first, sources.first_outcome);
        self.division_equipment.outcome = match first_sources {
            Ok(source) => cequipment_variant_pool::walk_divisions(
                recorded,
                base,
                source,
                &mut self.division_equipment.rows,
            ),
            Err(reason) => Outcome::Unavailable { reason },
        };
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        let result = match first_sources {
            Ok(source) => carmy_upgrades_status::walk(
                recorded,
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
        decode_ticks[2] += elapsed(started);
        self.divisions_raw_mode = false;
    }

    /// One shared traversal fills all four deployment buffers. The second pass
    /// writes the comparison buffers, which are empty once the window closes.
    fn deployment(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        second: bool,
    ) -> cdeployment_status::Outcomes {
        let (conveyors, lines, manpower, equipment, names, custom) = if second {
            (
                &mut self.conveyors.second,
                &mut self.deployment_lines.second,
                &mut self.deployment_manpower.second,
                &mut self.deployment_equipment.second,
                &mut self.conveyor_name_bytes.second,
                &mut self.line_custom_name_bytes.second,
            )
        } else {
            (
                &mut self.conveyors.rows,
                &mut self.deployment_lines.rows,
                &mut self.deployment_manpower.rows,
                &mut self.deployment_equipment.rows,
                &mut self.conveyor_name_bytes.first,
                &mut self.line_custom_name_bytes.first,
            )
        };
        cdeployment_status::walk(
            memory, base, game_state, conveyors, lines, manpower, equipment, names, custom,
        )
    }

    /// Decode-worker interpretation of a raw hourly production-rest payload.
    /// Replays the existing lines, stockpile, licence, deployment and licence-
    /// record walks against the recorded spans, installing identical rows,
    /// arenas, truncation counts and failure reasons into the frame's pooled
    /// buffers. Checked/mismatches set by the raw compare survive. Clears
    /// `rest_raw_mode` so the next capture starts clean.
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
// Hourly production-rest raw-copy split (ProductionLines, Stockpiles,
// ProductionLicenses, DeploymentConveyors, DeploymentLines,
// DeploymentManpower, DeploymentEquipment, ProductionLicenseRecords).
//
// Zero-engine-change replay (Wars/Divisions/Focus/Session precedent for
// census plus per-country walks): the game thread copies raw bytes only and
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
// Fault attribution is conservative like the pilot: shared census spans carry
// all eight bits, production-manager spans the four production bits, and
// deployment spans all four deployment bits, so any live failure blocks its
// whole shared domain even when both copies agree. Per-table leaves use
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

/// One hour of raw production-rest payload. Flat pools recycle capacities
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
        let mut job = Self {
            // Late-campaign worst case is about 768 lines, 1.6k stockpiles,
            // licences plus deployment queues with names; both pools start
            // above that so steady state never overflows.
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
pub(crate) struct ProductionRestCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in family
    /// order [lines, stockpiles, licences, conveyors, lines, manpower,
    /// equipment, records].
    pub matched: [bool; 8],
    pub copy_ticks: [u64; 8],
    pub raw_bytes: [u64; 8],
}

/// Game-thread copy parameters, bundled to keep the tick entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ProductionRestCopyContext {
    pub base: usize,
    pub game_state: usize,
    pub needed: [bool; 8],
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
/// seven scattered field spans per line plus variant identities.
fn copy_production_lines(log: &mut RawLog<8>, memory: &impl Memory, manager: usize) {
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
        // Seven scattered spans exactly like `read_row`'s `copy_scattered`.
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
        ];
        let mut variant = None;
        for (addr, len) in takes {
            if let Some(start) = pspan(log, memory, Some(addr), len, M_P_LINES)
                && addr
                    == pointer
                        .checked_add(cequipment_production_line::EQUIPMENT_VARIANT)
                        .unwrap_or(0)
            {
                variant = usize::try_from(p_pool_u64(log, start))
                    .ok()
                    .filter(|&p| p != 0);
            }
        }
        if let Some(v) = variant {
            copy_variant_identity(log, memory, v, M_P_LINES);
        }
        if log.overflowed {
            return;
        }
    }
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
        let mut head = [0u8; 0x58];
        head.copy_from_slice(&log.bytes[head_start..head_start + 0x58]);
        let parent = usize::try_from(u64::from_le_bytes(
            head[cequipment_production_license::PARENT..cequipment_production_license::PARENT + 8]
                .try_into()
                .unwrap(),
        ))
        .ok()
        .filter(|&p| p != 0);
        if let Some(p) = parent {
            copy_variant_identity(log, memory, p, M_P_REC);
        }
        let equipment = Array::from_header(
            head[cequipment_production_license::EQUIPMENT
                ..cequipment_production_license::EQUIPMENT + 16]
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
        let mut head = [0u8; 0x9c];
        head.copy_from_slice(&log.bytes[head_start..head_start + 0x9c]);
        let template = usize::try_from(u64::from_le_bytes(
            head[cmilitary_deployment_conveyor::TEMPLATE
                ..cmilitary_deployment_conveyor::TEMPLATE + 8]
                .try_into()
                .unwrap(),
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
            head[cmilitary_deployment_conveyor::LINES..cmilitary_deployment_conveyor::LINES + 16]
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
    let mut head = [0u8; 0x40];
    head.copy_from_slice(&log.bytes[head_start..head_start + 0x40]);
    let name = usize::try_from(u64::from_le_bytes(
        head[cmilitary_deployment_line::NAME..cmilitary_deployment_line::NAME + 8]
            .try_into()
            .unwrap(),
    ))
    .ok()
    .filter(|&p| p != 0);
    let deployment = usize::try_from(u64::from_le_bytes(
        head[cmilitary_deployment_line::DEPLOYMENT..cmilitary_deployment_line::DEPLOYMENT + 8]
            .try_into()
            .unwrap(),
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
    let mut chead = [0u8; 32];
    chead.copy_from_slice(&log.bytes[chead_start..chead_start + 32]);
    let len = match usize::try_from(u64::from_le_bytes(
        chead[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    )) {
        Ok(l) => l,
        Err(_) => return,
    };
    let capacity = u64::from_le_bytes(chead[msvc_string::CAPACITY..].try_into().unwrap());
    if len as u64 <= capacity && capacity >= msvc_string::INLINE_CAPACITY {
        let source = if capacity == msvc_string::INLINE_CAPACITY {
            string
        } else {
            match usize::try_from(u64::from_le_bytes(chead[..8].try_into().unwrap()))
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

impl Captured {
    /// Game-thread raw copy for an hourly refresh. Copies `first` always and
    /// `second` while any rest table still needs double-read checks, compares
    /// the raw logs, and records copy-only costs plus the existing
    /// checked/mismatches bookkeeping. Returns whether the payload is
    /// complete (`raw_ok`); on staging overflow the caller falls back to the
    /// typed walk and enqueues nothing.
    pub(crate) fn copy_production_rest_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: ProductionRestCopyContext,
        first: &mut ProductionRestJob,
        mut second: Option<&mut ProductionRestJob>,
    ) -> ProductionRestCopyReport {
        let started = qpc();
        first.clear();
        first.game_state = ctx.game_state;
        copy_production_rest_pass(memory, ctx.base, ctx.game_state, first);
        let mut ticks = [0u64; 8];
        ticks[0] = elapsed(started);
        let mut matched = [true; 8];
        if let Some(second_job) = second.as_deref_mut() {
            let second_started = qpc();
            second_job.clear();
            second_job.game_state = ctx.game_state;
            copy_production_rest_pass(memory, ctx.base, ctx.game_state, second_job);
            ticks[0] += elapsed(second_started);
            // Per-table partitions: shared roots gate their dependents,
            // private spans only their own table.
            let raw_ok_pair = !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                let equal = raw_ok_pair && first.log.partition_equal(&second_job.log, slot);
                *matched = equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        // Conservative checked bookkeeping mirroring the typed paths: shared
        // name/handle arenas block every member table on change.
        self.lines.checked = ctx.needed[0];
        self.lines.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.stockpiles.checked = ctx.needed[1];
        self.stockpiles.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.licenses.checked = ctx.needed[2];
        self.licenses.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        self.conveyors.checked = ctx.needed[3];
        self.conveyors.mismatches = usize::from(ctx.needed[3] && !matched[3]);
        self.deployment_lines.checked = ctx.needed[4];
        self.deployment_lines.mismatches = usize::from(ctx.needed[4] && !matched[4]);
        self.deployment_manpower.checked = ctx.needed[5];
        self.deployment_manpower.mismatches = usize::from(ctx.needed[5] && !matched[5]);
        self.deployment_equipment.checked = ctx.needed[6];
        self.deployment_equipment.mismatches = usize::from(ctx.needed[6] && !matched[6]);
        self.license_records.checked = ctx.needed[7];
        self.license_records.mismatches = usize::from(ctx.needed[7] && !matched[7]);
        let raw_ok = !first.log.overflowed && second.is_none_or(|job| !job.log.overflowed);
        ProductionRestCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }
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
    for index in 0..job.census.len() {
        let country = job.census[index];
        if let Some(manager) = copy_production_manager(&mut job.log, memory, base, &country) {
            copy_production_lines(&mut job.log, memory, manager);
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
    /// Read variant fields from complete, independently acquired registry
    /// snapshots. No consumer-table union or discovery pass is needed.
    pub fn finish_variants(
        &mut self,
        memory: &impl Memory,
        base: usize,
        needed: &[bool],
        variants: Sources<'_, cequipment_variant::RegistryRow>,
        hour: i32,
        force_full: bool,
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let mut matched = [false; N];
        let mut ticks = [0; N];
        // Tier-4 variants share slot3; off-hours skip the whole walk.
        let walk = replay::cadence::should_walk(
            replay::rows::TableId::VariantDefinitions,
            hour,
            force_full,
        );

        if !walk {
            let skipped = Outcome::Unavailable {
                reason: replay::cadence::SKIPPED_REASON,
            };
            self.definitions.outcome = skipped;
            self.modules.outcome = skipped;
            self.stats.outcome = skipped;
            self.upgrades.outcome = skipped;
            return (matched, ticks);
        }
        let begin = qpc();
        let first = complete_source(variants.first, variants.first_outcome);
        let second = complete_source(variants.second, variants.second_outcome);
        matched[3] = capture_text(
            &mut self.definitions,
            &mut self.definition_bytes,
            needed[3],
            first,
            second,
            |source, rows, arena| {
                cequipment_variant::walk_definitions(memory, base, source, rows, arena)
            },
        );
        matched[4] = capture_text(
            &mut self.modules,
            &mut self.module_bytes,
            needed[4],
            first,
            second,
            |source, rows, arena| {
                equipment_module_container::walk(memory, base, source, rows, arena)
            },
        );
        self.stats.checked = needed[5];
        self.stats.outcome = match first {
            Ok(source) => cequipment_variant::walk_stats(memory, source, &mut self.stats.rows),
            Err(reason) => Outcome::Unavailable { reason },
        };
        if needed[5] {
            let second = match second {
                Ok(source) => {
                    cequipment_variant::walk_stats(memory, source, &mut self.stats.second)
                }
                Err(reason) => Outcome::Unavailable { reason },
            };
            matched[5] = self.stats.compare(second);
        }
        matched[6] = capture_text(
            &mut self.upgrades,
            &mut self.upgrade_bytes,
            needed[6],
            first,
            second,
            |source, rows, arena| {
                cequipment_upgrades_instance::walk(memory, base, source, rows, arena)
            },
        );
        if sources_changed(first, second) {
            // A changed parent census is a mismatch even when both child tables
            // are empty (for example, two variants without installed modules).
            for (index, mismatches) in [
                (3, &mut self.definitions.mismatches),
                (4, &mut self.modules.mismatches),
                (5, &mut self.stats.mismatches),
                (6, &mut self.upgrades.mismatches),
            ] {
                if needed[index] {
                    *mismatches += 1;
                    matched[index] = false;
                }
            }
        }
        ticks[3] = elapsed(begin);

        (matched, ticks)
    }

    pub fn outcomes(&self) -> [Outcome; N] {
        [
            self.lines.outcome,
            self.stockpiles.outcome,
            self.division_equipment.outcome,
            self.definitions.outcome,
            self.modules.outcome,
            self.stats.outcome,
            self.upgrades.outcome,
            self.groups.outcome,
            self.requests.outcome,
            self.deliveries.outcome,
            self.produced.outcome,
            self.licenses.outcome,
            outcome(&self.conveyors),
            outcome(&self.deployment_lines),
            outcome(&self.deployment_manpower),
            outcome(&self.deployment_equipment),
            outcome(&self.license_records),
        ]
    }

    pub fn checked_mismatches(&self) -> [usize; N] {
        [
            checked_mismatches(&self.lines),
            checked_mismatches(&self.stockpiles),
            checked_mismatches(&self.division_equipment),
            checked_mismatches(&self.definitions),
            checked_mismatches(&self.modules),
            checked_mismatches(&self.stats),
            checked_mismatches(&self.upgrades),
            checked_mismatches(&self.groups),
            checked_mismatches(&self.requests),
            checked_mismatches(&self.deliveries),
            checked_mismatches(&self.produced),
            checked_mismatches(&self.licenses),
            checked_mismatches(&self.conveyors),
            checked_mismatches(&self.deployment_lines),
            checked_mismatches(&self.deployment_manpower),
            checked_mismatches(&self.deployment_equipment),
            checked_mismatches(&self.license_records),
        ]
    }

    pub fn tables(&self) -> [TableView; N] {
        [
            table::<tables::ProductionLines, _>(outcome(&self.lines), &self.lines.rows, |r| {
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
                }
            }),
            table::<tables::Stockpiles, _>(outcome(&self.stockpiles), &self.stockpiles.rows, |r| {
                rows::StockpileRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                }
            }),
            table::<tables::DivisionEquipment, _>(
                outcome(&self.division_equipment),
                &self.division_equipment.rows,
                |r| rows::DivisionEquipmentRow {
                    division: r.division,
                    template: r.template,
                    country: r.country,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                },
            ),
            definitions(&self.definitions, &self.definition_bytes),
            modules(&self.modules, &self.module_bytes),
            table::<tables::VariantStats, _>(outcome(&self.stats), &self.stats.rows, |r| {
                rows::VariantStatsRow {
                    variant: r.variant,
                    values: r.values,
                }
            }),
            upgrades(&self.upgrades, &self.upgrade_bytes),
            table::<tables::ArmyUpgradeGroups, _>(outcome(&self.groups), &self.groups.rows, |r| {
                rows::ArmyUpgradeGroupRow {
                    country: r.country,
                    army: r.army,
                    priority: r.priority,
                }
            }),
            table::<tables::ArmyUpgradeRequests, _>(
                outcome(&self.requests),
                &self.requests.rows,
                |r| rows::ArmyUpgradeRequestRow {
                    country: r.country,
                    army: r.army,
                    ordinal: r.ordinal,
                    equipment: r.equipment,
                    request_raw: r.request_raw,
                    total_raw: r.total_raw,
                },
            ),
            table::<tables::ArmyUpgradeDeliveries, _>(
                outcome(&self.deliveries),
                &self.deliveries.rows,
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
            table::<tables::ArmyUpgradeProduced, _>(
                outcome(&self.produced),
                &self.produced.rows,
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
                outcome(&self.licenses),
                &self.licenses.rows,
                |r| rows::ProductionLicenseRow {
                    country: r.country,
                    ordinal: r.ordinal,
                    variant: r.variant,
                },
            ),
            conveyors(&self.conveyors, &self.conveyor_name_bytes),
            deployment_lines(&self.deployment_lines, &self.line_custom_name_bytes),
            table::<tables::DeploymentManpower, _>(
                outcome(&self.deployment_manpower),
                &self.deployment_manpower.rows,
                |r| rows::DeploymentManpowerRow {
                    line: r.line,
                    side: r.side,
                    ordinal: r.ordinal,
                    source_country: r.source_country,
                    amount: r.amount,
                },
            ),
            table::<tables::DeploymentEquipment, _>(
                outcome(&self.deployment_equipment),
                &self.deployment_equipment.rows,
                |r| rows::DeploymentEquipmentRow {
                    line: r.line,
                    ordinal: r.ordinal,
                    variant_handle: r.variant_handle,
                    amount_raw: r.amount_raw,
                },
            ),
            license_records(&self.license_records, &self.license_handles),
        ]
    }
}

/// Produced-variant handles live in a shared arena; a span outside it fails the
/// table rather than publishing a shortened or borrowed list.
fn license_records(
    rows: &CaptureRows<cproduction_status::LicenseRecordRow>,
    arena: &Arena<u64>,
) -> TableView {
    let result = outcome(rows);
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

/// Uniform access to the four deployment buffers, which hold different rows but
/// share one traversal, one consistency window and one name arena.
trait DeploymentRows {
    fn compare(&mut self, second: Outcome) -> bool;
    fn add_mismatch(&mut self);
}

impl<R: Default + Clone + PartialEq> DeploymentRows for CaptureRows<R> {
    fn compare(&mut self, second: Outcome) -> bool {
        CaptureRows::compare(self, second)
    }
    fn add_mismatch(&mut self) {
        self.mismatches += 1;
    }
}

/// Conveyor names live in a shared arena; a span outside it fails the table
/// rather than publishing a truncated or borrowed-from-elsewhere name.
fn conveyors(rows: &CaptureRows<cdeployment_status::ConveyorRow>, arena: &Arena<u8>) -> TableView {
    let result = outcome(rows);
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
    let result = outcome(rows);
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

fn sources_changed<T: PartialEq>(
    first: Result<&[T], &'static str>,
    second: Result<&[T], &'static str>,
) -> bool {
    matches!((first, second), (Ok(first), Ok(second)) if first != second)
}

fn capture_rows<R: Default + Clone + PartialEq>(
    rows: &mut CaptureRows<R>,
    needed: bool,
    mut walk: impl FnMut(&mut [R]) -> Outcome,
) -> bool {
    rows.checked = needed;
    rows.outcome = walk(&mut rows.rows);
    if !needed {
        return false;
    }
    let second = walk(&mut rows.second);
    rows.compare(second)
}

fn capture_text<R: Default + Clone + PartialEq>(
    rows: &mut CaptureRows<R>,
    arena: &mut Arena<u8>,
    needed: bool,
    first: Result<&[cequipment_variant::RegistryRow], &'static str>,
    second: Result<&[cequipment_variant::RegistryRow], &'static str>,
    mut walk: impl FnMut(
        &[cequipment_variant::RegistryRow],
        &mut [R],
        &mut [u8],
    ) -> cequipment_variant::TextOutcome,
) -> bool {
    rows.checked = needed;
    let result = match first {
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
    if !needed {
        return false;
    }
    let result = match second {
        Ok(source) => walk(source, &mut rows.second, &mut arena.second),
        Err(reason) => cequipment_variant::TextOutcome {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        },
    };
    rows.compare(result.outcome);
    rows.mismatches += usize::from(!arena.compare(result.bytes_len, result.required_bytes));
    full_match(rows.outcome, result.outcome, rows.mismatches)
}

fn checked_mismatches<T>(rows: &CaptureRows<T>) -> usize {
    if rows.checked { rows.mismatches } else { 0 }
}

fn outcome<T>(rows: &CaptureRows<T>) -> Outcome {
    if rows.checked && rows.mismatches != 0 {
        Outcome::Unavailable {
            reason: "production table changed during double read",
        }
    } else {
        rows.outcome
    }
}

fn text(arena: &[u8], span: cequipment_variant::TextSpan) -> Option<&[u8]> {
    arena.get(span.start..span.start.checked_add(span.len)?)
}

fn checked_text<'a, T, const N: usize>(
    rows: &'a CaptureRows<T>,
    arena: &'a Arena<u8>,
    spans: impl Fn(&T) -> [cequipment_variant::TextSpan; N],
) -> Result<&'a [u8], TableView> {
    let result = outcome(rows);
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
        [r.display_name, r.variant_name, r.equipment_name]
    }) {
        Ok(bytes) => bytes,
        Err(failure) => return failure,
    };
    table::<tables::VariantDefinitions, _>(outcome(rows), &rows.rows, |r| {
        rows::VariantDefinitionRow {
            id: r.identity.id,
            equipment_token: r.identity.equipment_token,
            creator_link: r.identity.creator_link,
            origin_link: r.identity.origin_link,
            append_variant_name: r.identity.append_variant_name,
            obsolete: r.identity.obsolete,
            classifier_flags: r.identity.classifier_flags,
            mission_mask: r.identity.mission_mask,
            display_name: text(bytes, r.display_name).unwrap().to_vec(),
            variant_name: text(bytes, r.variant_name).unwrap().to_vec(),
            equipment_name: text(bytes, r.equipment_name).unwrap().to_vec(),
        }
    })
}

fn modules(rows: &CaptureRows<equipment_module_container::Row>, arena: &Arena<u8>) -> TableView {
    let bytes = match checked_text(rows, arena, |r| [r.definition_name]) {
        Ok(bytes) => bytes,
        Err(failure) => return failure,
    };
    table::<tables::VariantModules, _>(outcome(rows), &rows.rows, |r| rows::VariantModuleRow {
        variant: r.variant,
        source_ordinal: r.source_ordinal,
        slot: r.slot,
        definition_token: r.definition_token,
        definition_active: r.definition_active,
        definition_name: text(bytes, r.definition_name).unwrap().to_vec(),
    })
}

fn upgrades(rows: &CaptureRows<cequipment_upgrades_instance::Row>, arena: &Arena<u8>) -> TableView {
    let bytes = match checked_text(rows, arena, |r| [r.definition_name]) {
        Ok(bytes) => bytes,
        Err(failure) => return failure,
    };
    table::<tables::VariantUpgrades, _>(outcome(rows), &rows.rows, |r| rows::VariantUpgradeRow {
        variant: r.variant,
        source_ordinal: r.source_ordinal,
        definition_token: r.definition_token,
        level: r.level,
        definition_name: text(bytes, r.definition_name).unwrap().to_vec(),
    })
}

/// Shared synthetic address space for the hourly production-rest
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
    const VARIANT: usize = 0x7700;
    const PLINE: usize = 0x11000;
    const PDATA: usize = 0x11100;
    const SDATA: usize = 0x11200;
    const LDATA: usize = 0x11300;
    const RDATA: usize = 0x11400;
    const RECORD: usize = 0x11500;
    const RLIST: usize = 0x11600;

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
            module_bytes: 64,
            upgrade_bytes: 64,
            conveyor_name_bytes: 64,
            line_custom_name_bytes: 64,
            license_handles: 64,
        }
    }

    pub(crate) fn direct(memory: &impl engine::Memory, capacities: &Capacities) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let divisions = Sources {
            first: &[],
            first_outcome: Outcome::Rows { len: 0 },
            second: &[],
            second_outcome: Outcome::Rows { len: 0 },
        };
        let _ = captured.acquire(memory, 0, BASE, &[true; N], divisions, 0, true);
        captured.rest_raw_mode = false;
        captured
    }

    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, ProductionRestCopyReport, ProductionRestJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: BASE,
            needed: [true; 8],
        };
        let mut first = ProductionRestJob::new();
        let mut second = ProductionRestJob::new();
        let report =
            captured.copy_production_rest_refresh(memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        captured.rest_raw_mode = report.raw_ok;
        // Typed walks skip while staged; the worker fills the buffers.
        let divisions = Sources {
            first: &[],
            first_outcome: Outcome::Rows { len: 0 },
            second: &[],
            second_outcome: Outcome::Rows { len: 0 },
        };
        let _ = captured.acquire(memory, 0, BASE, &[true; N], divisions, 0, true);
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
            module_bytes: 16,
            upgrade_bytes: 16,
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
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
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
        captured.stats.outcome = Outcome::Rows { len: 1 };
        captured.stats.rows[0] = cequipment_variant::StatsRow {
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
            captured.stats.rows[0].values
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
    }

    #[test]
    fn licences_and_deployment_conversion_keep_order_names_and_signed_extrema() {
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
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
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
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

    /// The four deployment tables share one traversal, one consistency window
    /// and one name arena, so a queue that moves between the two passes must
    /// fail all four and a changed name alone must fail them too.
    #[test]
    fn a_changed_deployment_queue_fails_all_four_tables_and_a_changed_name_counts() {
        use engine::{
            carmy_manpower_values, ccountry, cdeployment_status as deployment, cequipment_variant,
            cequipment_variant_pool, cgame_state, cmilitary_deployment,
            cmilitary_deployment_conveyor as conveyor_layout, cmilitary_deployment_line,
            cname_group_member, creferenced_division_template, memory::SliceMemory, msvc_string,
        };
        use std::cell::Cell;

        const BASE: usize = 0x1000;
        const COUNTRY: usize = 0x3000;
        const STATUS: usize = 0x4000;
        const QUEUE: usize = 0x4100;
        const CONVEYOR: usize = 0x5000;
        const LINE_ARRAY: usize = 0x5200;
        const LINE: usize = 0x6000;
        const NAME_OBJECT: usize = 0x6200;
        const DEPLOYMENT: usize = 0x7000;
        const TEMPLATE: usize = 0x7600;

        fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
            bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
        }
        fn array(bytes: &mut [u8], header: usize, data: usize, count: u32) {
            put(bytes, header, &(data as u64).to_le_bytes());
            put(bytes, header + 8, &count.to_le_bytes());
            put(bytes, header + 12, &count.to_le_bytes());
        }

        let mut bytes = vec![0_u8; 0x8000];
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
            COUNTRY + ccountry::DEPLOYMENT_MANAGER,
            &(STATUS as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS,
            &(deployment::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATUS + deployment::OWNER,
            &(COUNTRY as u64).to_le_bytes(),
        );
        array(&mut bytes, STATUS + deployment::CONVEYORS, QUEUE, 1);
        put(&mut bytes, QUEUE, &(CONVEYOR as u64).to_le_bytes());
        put(
            &mut bytes,
            CONVEYOR,
            &(conveyor_layout::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + conveyor_layout::TYPE,
            &0x0000_0001_0000_0040_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + conveyor_layout::TEMPLATE,
            &(TEMPLATE as u64).to_le_bytes(),
        );
        put(&mut bytes, CONVEYOR + conveyor_layout::NAME, b"Panzer\0");
        put(
            &mut bytes,
            CONVEYOR + conveyor_layout::NAME + msvc_string::LENGTH,
            &6_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            CONVEYOR + conveyor_layout::NAME + msvc_string::CAPACITY,
            &15_u64.to_le_bytes(),
        );
        array(&mut bytes, CONVEYOR + conveyor_layout::LINES, LINE_ARRAY, 1);
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
            &0x0000_0002_0000_003f_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            LINE + cmilitary_deployment_line::NAME,
            &(NAME_OBJECT as u64).to_le_bytes(),
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
            NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::LENGTH,
            &0_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            NAME_OBJECT + cname_group_member::OVERRIDE_NAME + msvc_string::CAPACITY,
            &msvc_string::INLINE_CAPACITY.to_le_bytes(),
        );
        put(&mut bytes, NAME_OBJECT + cname_group_member::FLAG_A, &[1]);
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
        put(
            &mut bytes,
            DEPLOYMENT + cmilitary_deployment::OWNING_LINE,
            &(LINE as u64).to_le_bytes(),
        );
        for offset in [
            cmilitary_deployment::MANPOWER_HAVE,
            cmilitary_deployment::MANPOWER_NEED,
        ] {
            put(
                &mut bytes,
                DEPLOYMENT + offset,
                &(carmy_manpower_values::VTABLE_RVA as u64).to_le_bytes(),
            );
        }
        let _ = cequipment_variant::VTABLE_RVA;

        /// Flips one byte of the conveyor's training word on the second pass.
        struct ChangingTraining<'a> {
            bytes: &'a [u8],
            copies: Cell<usize>,
        }
        impl Memory for ChangingTraining<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if !(SliceMemory {
                    base: BASE,
                    bytes: self.bytes,
                })
                .copy(address, output)
                {
                    return false;
                }
                if address == DEPLOYMENT && output.len() >= 8 {
                    let copies = self.copies.get();
                    self.copies.set(copies + 1);
                }
                if address == DEPLOYMENT + cmilitary_deployment::TRAINING && self.copies.get() > 1 {
                    output[0] ^= 1;
                }
                true
            }
        }

        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
        let stable = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let (matched, _) = captured.acquire(
            &stable,
            0,
            BASE,
            &[true; N],
            Sources {
                first: &[],
                first_outcome: Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: Outcome::Rows { len: 0 },
            },
            0,
            true,
        );
        assert_eq!(&matched[12..16], &[true; 4]);
        assert_eq!(captured.conveyors.outcome, Outcome::Rows { len: 1 });

        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
        let changing = ChangingTraining {
            bytes: &bytes,
            copies: Cell::new(0),
        };
        let (matched, _) = captured.acquire(
            &changing,
            0,
            BASE,
            &[true; N],
            Sources {
                first: &[],
                first_outcome: Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: Outcome::Rows { len: 0 },
            },
            0,
            true,
        );
        // Only the line row's own word moved, so the shared traversal reports the
        // mismatch on the line table alone and leaves the other three matched.
        assert_eq!(&matched[12..16], &[true, false, true, true]);
        assert_eq!(&captured.checked_mismatches()[12..16], &[0, 1, 0, 0]);
        assert!(matches!(
            captured.tables()[13],
            TableView::Unavailable { .. }
        ));
        for index in [12, 14, 15] {
            assert!(
                matches!(captured.tables()[index], TableView::Complete(_)),
                "{index}"
            );
        }

        // A conveyor array whose header is torn between the two passes fails
        // every dependent table rather than publishing a shorter queue.
        let mut broken = bytes.clone();
        put(
            &mut broken,
            STATUS + deployment::CONVEYORS + 12,
            &2_u32.to_le_bytes(),
        );
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
        let (matched, _) = captured.acquire(
            &SliceMemory {
                base: BASE,
                bytes: &broken,
            },
            0,
            BASE,
            &[true; N],
            Sources {
                first: &[],
                first_outcome: Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: Outcome::Rows { len: 0 },
            },
            0,
            true,
        );
        assert_eq!(&matched[12..16], &[false; 4]);
        for index in 12..16 {
            assert!(
                matches!(captured.outcomes()[index], Outcome::Unavailable { .. }),
                "{index}"
            );
        }
    }

    #[test]
    fn a_conveyor_name_outside_the_arena_fails_only_its_own_table() {
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
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
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
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
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
        captured.definition_bytes.first[..4].copy_from_slice(&[0xff, 0, 0x80, b'A']);
        captured.definition_bytes.len = 4;
        captured.definitions.outcome = Outcome::Rows { len: 1 };
        captured.definitions.rows[0] = cequipment_variant::DefinitionRow {
            identity: cequipment_variant::Row {
                id: u64::MAX,
                equipment_token: u32::MAX,
                append_variant_name: 0xff,
                obsolete: 0x80,
                classifier_flags: u64::MAX,
                ..Default::default()
            },
            display_name: TextSpan { start: 0, len: 2 },
            variant_name: TextSpan { start: 2, len: 0 },
            equipment_name: TextSpan { start: 2, len: 2 },
        };
        captured.module_bytes.first[..2].copy_from_slice(&[0xff, 0]);
        captured.module_bytes.len = 2;
        captured.modules.outcome = Outcome::Rows { len: 2 };
        captured.modules.rows[0] = equipment_module_container::Row {
            variant: u64::MAX,
            source_ordinal: 7,
            slot: 3,
            definition_token: u32::MAX,
            definition_active: 0xfe,
            definition_name: TextSpan { start: 0, len: 2 },
        };
        captured.modules.rows[1] = equipment_module_container::Row {
            source_ordinal: 1,
            slot: 9,
            ..captured.modules.rows[0]
        };
        captured.upgrade_bytes.first[0] = 0xff;
        captured.upgrade_bytes.len = 1;
        captured.upgrades.outcome = Outcome::Rows { len: 1 };
        captured.upgrades.rows[0] = cequipment_upgrades_instance::Row {
            variant: u64::MAX,
            source_ordinal: u32::MAX,
            definition_token: i32::MIN,
            level: u8::MAX,
            definition_name: TextSpan { start: 0, len: 1 },
        };
        let views = captured.tables();
        let definition = &complete::<tables::VariantDefinitions>(&views[3])[0];
        assert_eq!(
            (&definition.display_name[..], &definition.equipment_name[..]),
            (&[0xff, 0][..], &[0x80, b'A'][..])
        );
        assert!(definition.variant_name.is_empty());
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
        assert_eq!(modules[0].definition_name, [0xff, 0]);
        let upgrade = &complete::<tables::VariantUpgrades>(&views[6])[0];
        assert_eq!(
            (upgrade.definition_token, upgrade.level),
            (i32::MIN, u8::MAX)
        );
        assert_eq!(upgrade.definition_name, [0xff]);
        captured.modules.rows[0].definition_name = TextSpan {
            start: usize::MAX,
            len: 2,
        };
        assert!(matches!(
            captured.tables()[4],
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
    fn complete_census_is_distinct_from_a_readable_prefix_or_a_changed_empty_parent_set() {
        let destination = [10usize, 20];
        let truncated = Outcome::Truncated {
            len: 2,
            unread_rows: 1,
        };
        assert_eq!(
            complete_source(&destination, truncated),
            Err("source census incomplete")
        );
        assert_eq!(
            complete_source(&destination, Outcome::Rows { len: 3 }),
            Err("source row count exceeds acquired buffer")
        );
        assert!(sources_changed(Ok(&[10usize][..]), Ok(&[20usize][..])));
        assert!(!sources_changed(Ok(&[] as &[usize]), Ok(&[])));
        assert!(complete_source(&destination, truncated).is_err());
        assert_eq!(
            complete_source(&destination, Outcome::Rows { len: 0 }),
            Ok(&[][..])
        );
        let mut rows = CaptureRows::<u32>::new(2, 0);
        let mut arena = Arena::new(2, 0);
        let mut calls = 0;
        assert!(!capture_text(
            &mut rows,
            &mut arena,
            true,
            Err("failed census"),
            Err("failed census"),
            |_, _, _| {
                calls += 1;
                unreachable!()
            }
        ));
        assert_eq!(calls, 0);
        assert_eq!(
            outcome(&rows),
            Outcome::Unavailable {
                reason: "failed census"
            }
        );
    }

    #[test]
    fn upgrade_tables_require_complete_stable_division_sources_independently() {
        let mut bytes = vec![0_u8; 0x3000];
        let army = 0x1000;
        let group = 0x2000;
        for (address, value) in [
            (
                army + engine::carmy::REQUESTS,
                (group - carmy_upgrades_requests::IN_ARMY_REQUESTS) as u64,
            ),
            (group, carmy_upgrades_requests::VTABLE_RVA as u64),
            (
                group + carmy_upgrades_requests::PERSISTENT_SUBOBJECT,
                carmy_upgrades_requests::PERSISTENT_VTABLE_RVA as u64,
            ),
            (group + carmy_upgrades_requests::ARMY, army as u64),
        ] {
            bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = engine::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let first = [cequipment_variant_pool::DivisionSource {
            pointer: army,
            division: 0x1234_0000_001a,
            template: 1,
            country: 7,
        }];
        let mut second = first;
        second[0].template = 2;
        let mut captured = Captured::with_capacities(&capacities(), &[0; N]);
        let mut needed = [false; N];
        needed[7..11].fill(true);
        for (failed, reason) in [
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
        ] {
            captured.reset();
            let (matched, _) = captured.acquire(
                &memory,
                0,
                0,
                &needed,
                Sources {
                    first: &first,
                    first_outcome: failed,
                    second: &first,
                    second_outcome: failed,
                },
                0,
                true,
            );
            assert_eq!(&matched[7..11], &[false; 4]);
            assert_eq!(
                &captured.outcomes()[7..11],
                &[Outcome::Unavailable { reason }; 4]
            );
        }
        // Equipment has finished checking. Each upgrade table still rejects
        // a changed parent census even when its own emitted children are empty.
        for table in 7..11 {
            captured.reset();
            needed.fill(false);
            needed[table] = true;
            let (matched, _) = captured.acquire(
                &memory,
                0,
                0,
                &needed,
                Sources {
                    first: &first,
                    first_outcome: Outcome::Rows { len: 1 },
                    second: &second,
                    second_outcome: Outcome::Rows { len: 1 },
                },
                0,
                true,
            );
            assert!(!matched[table]);
            assert_eq!(captured.checked_mismatches()[table], 1);
            assert!(matches!(
                captured.tables()[table],
                TableView::Unavailable { .. }
            ));
            assert_eq!(captured.checked_mismatches()[2], 0);
            for other in (7..11).filter(|&other| other != table) {
                assert_eq!(captured.checked_mismatches()[other], 0);
                assert!(matches!(captured.outcomes()[other], Outcome::Rows { .. }));
            }
        }
    }

    #[test]
    fn changed_text_fails_consistency_and_truncation_grows_off_thread() {
        let mut rows = CaptureRows::<u32>::new(2, 0);
        let mut arena = Arena::new(2, 0);
        let mut value = 0;
        assert!(!capture_text(
            &mut rows,
            &mut arena,
            true,
            Ok(&[]),
            Ok(&[]),
            |_, rows, arena| {
                rows[0] = 7;
                arena[0] = value;
                value += 1;
                TextOutcome {
                    outcome: Outcome::Rows { len: 1 },
                    bytes_len: 1,
                    required_bytes: 1,
                }
            }
        ));
        assert_eq!(checked_mismatches(&rows), 1);
        assert!(matches!(outcome(&rows), Outcome::Unavailable { .. }));
        let mut capacities = capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        captured.stockpiles.outcome = Outcome::Truncated {
            len: 2,
            unread_rows: 5,
        };
        captured.stockpiles.rows[0].ordinal = 0;
        captured.stockpiles.rows[1].ordinal = 1;
        captured.module_bytes.required = 33;
        assert!(matches!(
            captured.tables()[1],
            TableView::Truncated { missing: 5, .. }
        ));
        captured.grow(&mut capacities, &[0; N]);
        assert!(captured.stockpiles.rows.len() >= 7);
        assert!(captured.stockpiles.second.len() >= 7);
        assert!(captured.module_bytes.first.len() >= 33);
        let allocated = captured.allocated_bytes();
        captured.grow(&mut capacities, &[crate::sampling::CONSISTENCY_TARGET; N]);
        assert!(captured.groups.second.is_empty() && captured.produced.second.is_empty());
        assert!(captured.module_bytes.second.is_empty());
        assert!(captured.allocated_bytes() < allocated);
    }

    #[test]
    fn each_table_stops_comparison_reads_without_waiting_for_variant_checks() {
        let mut rows = CaptureRows::<u64>::new(2, 0);
        let mut calls = 0;
        assert!(!capture_rows(&mut rows, false, |rows| {
            calls += 1;
            rows[0] = 70;
            Outcome::Rows { len: 1 }
        }));
        assert_eq!(calls, 1);
        assert!(!rows.checked);
        let mut capacities = capacities();
        let mut successful = [crate::sampling::CONSISTENCY_TARGET; N];
        successful[4] -= 1;
        successful[8] -= 1;
        let mut captured = Captured::with_capacities(&capacities, &successful);
        assert!(captured.lines.second.is_empty() && captured.stockpiles.second.is_empty());
        assert!(captured.division_equipment.second.is_empty());
        assert!(!captured.requests.second.is_empty() && !captured.groups.second.is_empty());
        assert!(!captured.modules.second.is_empty());
        successful[8] += 1;
        captured.grow(&mut capacities, &successful);
        assert!(captured.requests.second.is_empty() && captured.groups.second.is_empty());
        assert!(!captured.modules.second.is_empty());
        successful[4] += 1;
        captured.grow(&mut capacities, &successful);
        assert!(captured.modules.second.is_empty());
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
        let (split, report, _) = kit::split(&memory, &capacities);
        assert_eq!(report.matched, [true; 8]);
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
                module_bytes: 0,
                upgrade_bytes: 0,
                conveyor_name_bytes: 64,
                line_custom_name_bytes: 64,
                license_handles: 64,
            };
            let direct = kit::direct(&memory, &capacities);
            let mut captured = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = ProductionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
                needed: [true; 8],
            };
            let mut first = ProductionRestJob::new();
            let mut second = ProductionRestJob::new();
            let report =
                captured.copy_production_rest_refresh(&memory, ctx, &mut first, Some(&mut second));
            assert!(report.raw_ok, "rows={rows:?}");
            assert_eq!(report.matched, [true; 8], "rows={rows:?}");
            captured.rest_raw_mode = true;
            let divisions = Sources {
                first: &[],
                first_outcome: Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: Outcome::Rows { len: 0 },
            };
            let _ = captured.acquire(&memory, 0, kit::BASE, &[true; N], divisions, 0, true);
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
            module_bytes: 0,
            upgrade_bytes: 0,
            conveyor_name_bytes: 2,
            line_custom_name_bytes: 0,
            license_handles: 0,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = {
            let mut captured = Captured::with_capacities(&capacities, &[0; N]);
            let ctx = ProductionRestCopyContext {
                base: 0,
                game_state: kit::BASE,
                needed: [true; 8],
            };
            let mut first = ProductionRestJob::new();
            let mut second = ProductionRestJob::new();
            let report =
                captured.copy_production_rest_refresh(&memory, ctx, &mut first, Some(&mut second));
            assert!(report.raw_ok);
            captured.rest_raw_mode = true;
            let divisions = Sources {
                first: &[],
                first_outcome: Outcome::Rows { len: 0 },
                second: &[],
                second_outcome: Outcome::Rows { len: 0 },
            };
            let _ = captured.acquire(&memory, 0, kit::BASE, &[true; N], divisions, 0, true);
            let mut ticks = [0u64; 8];
            captured.decode_production_rest_payload(0, kit::BASE, &first, &mut ticks);
            (captured, report, first)
        };
        assert_eq!(report.matched, [true; 8]);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn production_rest_double_copy_compare_blocks_checks_on_tearing() {
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [true; 8],
        };
        let mut first = ProductionRestJob::new();
        let mut second = ProductionRestJob::new();
        let report =
            captured.copy_production_rest_refresh(&memory, ctx, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 8]);
        // A torn conveyor amount between the passes changes the log.
        let mut torn = bytes.clone();
        torn[0x5000 + engine::cmilitary_deployment_conveyor::AMOUNT - kit::BASE] ^= 0xff;
        let torn_memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &torn,
        };
        let mut after = ProductionRestJob::new();
        let mut after_torn = ProductionRestJob::new();
        {
            let mut log = crate::raw::RawLog::<8>::new(1 << 17, 8 << 20);
            let _ = &mut log;
        }
        let ctx_none = ProductionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [false; 8],
        };
        let mut c2 = Captured::with_capacities(&capacities, &[0; N]);
        let r1 = c2.copy_production_rest_refresh(&memory, ctx_none, &mut after, None);
        assert!(r1.raw_ok);
        let r2 = c2.copy_production_rest_refresh(&torn_memory, ctx_none, &mut after_torn, None);
        assert!(r2.raw_ok);
        assert!(!after.log.logs_equal(&after_torn.log));
    }

    #[test]
    fn production_rest_staging_overflow_falls_back_to_typed_walk() {
        use super::production_rest_test_kit as kit;
        let bytes = kit::fixture();
        let memory = engine::memory::SliceMemory {
            base: kit::BASE,
            bytes: &bytes,
        };
        let capacities = kit::capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let ctx = ProductionRestCopyContext {
            base: 0,
            game_state: kit::BASE,
            needed: [true; 8],
        };
        let mut tiny = ProductionRestJob {
            log: crate::raw::RawLog::new(8, 8),
            game_state: 0,
            census: Vec::new(),
            capacity_charge: crate::raw_memory::Charge::default(),
        };
        let report = captured.copy_production_rest_refresh(&memory, ctx, &mut tiny, None);
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
