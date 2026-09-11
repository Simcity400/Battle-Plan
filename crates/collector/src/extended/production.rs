//! Production/equipment acquisition. Only construction/growth and conversion
//! allocate; the callback fills typed buffers and measures shared graph work.

use crate::{
    raw::RawLog,
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status,
    cdeployment_status, cequipment_module, cequipment_production_line, cequipment_type,
    cequipment_upgrade, cequipment_upgrades_instance, cequipment_variant, cequipment_variant_pool,
    cpdx_array::Array, cproduction_status, cupgrade_delivery, equipment_module_container,
    equipment_module_entry, equipment_upgrade_entry, msvc_string, walk::Outcome,
};
use replay::{
    frame::{TableView},
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
    /// Raw-copy split gate for the Tier-4 slot-3 variant tables (3..7).
    /// Set by the game thread on refresh hours with a valid raw payload;
    /// `finish_variants` then skips the typed detail walks and the decode
    /// worker fills these rows before the ready handoff. Preserved across
    /// `reset` (set per refresh, cleared by the worker after decode). The
    /// registry census itself always runs game-side and is never gated.
    pub(crate) raw_mode: bool,
    /// Raw-copy split gate for the hourly divisions group (equipment table
    /// 2 plus the upgrade graph 7..11). Set by the game thread every hour
    /// with a valid raw payload; `acquire` then skips those typed detail
    /// walks and the decode worker fills the rows before the ready handoff.
    /// Preserved across `reset` (set per hour, cleared by the worker after
    /// decode). Lines, stockpiles, licences and deployment stay game-side.
    pub(crate) divisions_raw_mode: bool,
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
            raw_mode: false,
            divisions_raw_mode: false,
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
            let custom_matched = self.line_custom_name_bytes.compare(
                second.custom_name_bytes,
                second.required_custom_name_bytes,
            );
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
        // Raw-copy split: on refresh hours the registry census still runs
        // game-side (production consumes it here), but the per-variant
        // detail walks move to the decode worker via `copy_variants_refresh`.
        // Matched flags and costs come from the raw compare, applied by the
        // caller after this returns.
        let walk = replay::cadence::should_walk(
            replay::rows::TableId::VariantDefinitions,
            hour,
            force_full,
        ) && !self.raw_mode;
        if self.raw_mode {
            // Refresh payload staged: leave the reset outcomes for the decode
            // worker, which installs identical rows, outcomes and arenas.
            // The registry census above already ran game-side as always.
            return (matched, ticks);
        }
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
            deployment_lines(
                &self.deployment_lines,
                &self.line_custom_name_bytes,
            ),
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
fn conveyors(
    rows: &CaptureRows<cdeployment_status::ConveyorRow>,
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

// ===========================================================================
// Tier-4 slot-3 raw-copy split (VariantDefinitions, VariantModules,
// VariantStats, VariantUpgrades; all other production tables stay typed).
//
// The registry census always runs game-side (production consumes it here,
// per the coordinator ruling — no duplicated census). On refresh hours the
// game thread additionally copies per-variant detail bytes — definition
// fields, equipment identity, module/upgrade entries and strings — into the
// job, keyed by the census's own pointers. The single FIFO decode worker
// replays the existing detail walks against the recorded spans
// (`crate::raw::Recorded`) with the frame's pooled registry rows as the
// source, so rows, ordinals, skips, truncation and failure reasons are
// identical by construction (see `crate::raw` for the mirror rules).
//
// The mirror traverses structurally and records every discoverable span,
// never aborting except on overflow: the replay derives every outcome,
// precedence and error scope from the same recorded spans, so later extras
// are harmless.
// ===========================================================================

/// Fault-attribution bits, one per variant table in family order.
const M_DEFINITIONS: u32 = 0x01;
const M_MODULES: u32 = 0x02;
const M_STATS: u32 = 0x04;
const M_UPGRADES: u32 = 0x08;

/// One refresh of raw variant detail payload. Flat pools recycle capacities
/// and the steady state allocates on neither thread; growth happens on the
/// decode side only (see `crate::decode::recycle_variants_job`).
#[derive(Debug)]
pub(crate) struct VariantsJob {
    pub log: RawLog<4>,
}

impl VariantsJob {
    pub fn new() -> Self {
        Self {
            // Late-campaign worst case is about 6.4k variants with modules,
            // stats and upgrades; both pools start above that so steady
            // state never overflows.
            log: RawLog::new(1 << 20, 32 << 20),
        }
    }

    /// Reset lengths for reuse, keeping every capacity. No allocation.
    pub fn clear(&mut self) {
        self.log.clear();
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
pub(crate) struct VariantsCopyReport {
    /// False when staging overflowed: the caller falls back to the typed walk
    /// (identical rows through the existing path) and enqueues no payload.
    pub raw_ok: bool,
    /// Whole-log raw equality gated by live-fault conservatism, in
    /// [definitions, modules, stats, upgrades] order.
    pub matched: [bool; 4],
    pub copy_ticks: [u64; 4],
    pub raw_bytes: [u64; 4],
}

/// Game-thread copy parameters, bundled to keep the refresh entry small.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VariantsCopyContext {
    pub needed: [bool; 4],
}

fn pool_u64(log: &RawLog<4>, start: usize) -> u64 {
    u64::from_le_bytes(log.bytes[start..start + 8].try_into().unwrap())
}

/// One mirrored span read. `None` addresses (arithmetic overflow) record
/// nothing because the typed walk fails locally without reads.
fn vspan(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    addr: Option<usize>,
    len: usize,
    mask: u32,
) -> Option<usize> {
    log.take(memory, addr?, len, mask)
}

/// Parsed equipment string header: payload length, source and inline flag.
/// Mirrors the engine literal rules; `None` means the typed walk fails the
/// enclosing row from the same header bytes.
struct VLit {
    len: usize,
    source: usize,
    inline: bool,
}

fn parse_variant_literal(header: &[u8; 32], header_addr: usize) -> Option<VLit> {
    let len = u64::from_le_bytes(header[16..24].try_into().unwrap());
    let capacity = u64::from_le_bytes(header[24..32].try_into().unwrap());
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    if inline {
        if header[len] != 0 {
            return None;
        }
        Some(VLit {
            len,
            source: header_addr,
            inline: true,
        })
    } else {
        let source = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .ok()
            .filter(|&source| source != 0)?;
        source.checked_add(len)?;
        Some(VLit {
            len,
            source,
            inline: false,
        })
    }
}

/// Heap payload and terminator takes for one parsed literal. Inline strings
/// ride in the already-copied header span.
fn copy_variant_literal(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    literal: &VLit,
    mask: u32,
) {
    if literal.inline {
        return;
    }
    if literal.len != 0 {
        let _ = vspan(log, memory, Some(literal.source), literal.len, mask);
    }
    let _ = vspan(log, memory, literal.source.checked_add(literal.len), 1, mask);
}

/// Mirror of `walk_definitions`' read set for the registry census: fixed
/// field gathers, equipment identity and the three string literals per
/// variant. Takes are unconditional (a superset); the replay derives the
/// batch precedence and every outcome from the same recorded spans.
fn copy_variant_definitions(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    variants: &[cequipment_variant::RegistryRow],
) {
    for variant in variants {
        let pointer = variant.pointer();
        let token = vspan(log, memory, pointer.checked_add(cequipment_variant::EQUIPMENT_TOKEN), 13, M_DEFINITIONS);
        let definition = vspan(log, memory, pointer.checked_add(cequipment_variant::DEFINITION), 8, M_DEFINITIONS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::OBSOLETE), 1, M_DEFINITIONS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::CLASSIFIER_FLAGS), 12, M_DEFINITIONS);
        let display = vspan(log, memory, pointer.checked_add(cequipment_variant::DISPLAY_NAME), 64, M_DEFINITIONS);
        let (Some(_), Some(definition), Some(display)) = (token, definition, display) else {
            continue;
        };
        let mut headers = [0u8; 64];
        headers.copy_from_slice(&log.bytes[display..display + 64]);
        let equipment = usize::try_from(pool_u64(log, definition)).ok().filter(|&e| e != 0);
        let Some(equipment) = equipment else {
            continue;
        };
        let identity = vspan(log, memory, Some(equipment), 12, M_DEFINITIONS);
        let name = vspan(log, memory, equipment.checked_add(cequipment_type::NAME), 32, M_DEFINITIONS);
        let (Some(_), Some(name)) = (identity, name) else {
            continue;
        };
        let mut name_header = [0u8; 32];
        name_header.copy_from_slice(&log.bytes[name..name + 32]);
        let display_a: &[u8; 32] = headers[..32].try_into().unwrap();
        let display_b: &[u8; 32] = headers[32..].try_into().unwrap();
        for header in [display_a, display_b] {
            if let Some(literal) = parse_variant_literal(header, 0) {
                copy_variant_literal(log, memory, &literal, M_DEFINITIONS);
            }
        }
        if let Some(literal) = parse_variant_literal(&name_header, 0) {
            copy_variant_literal(log, memory, &literal, M_DEFINITIONS);
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `equipment_module_container::walk`'s read set: array header,
/// contiguous entry block, definition fields and the name literal per entry.
fn copy_variant_modules(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    variants: &[cequipment_variant::RegistryRow],
) {
    for variant in variants {
        let pointer = variant.pointer();
        let header = vspan(
            log,
            memory,
            pointer.checked_add(cequipment_variant::MODULES + equipment_module_container::ENTRIES_CANDIDATE),
            16,
            M_MODULES,
        );
        let Some(header) = header else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header..header + 16]);
        let array = match Array::from_header(raw, equipment_module_entry::STRIDE) {
            Some(array) => array,
            None => continue,
        };
        let block = (array.count as usize).checked_mul(equipment_module_entry::STRIDE);
        let entries = block.and_then(|len| {
            if len == 0 {
                Some(0)
            } else {
                vspan(log, memory, Some(array.data), len, M_MODULES)
            }
        });
        let Some(entries) = entries else {
            continue;
        };
        for index in 0..array.count as usize {
            let entry = entries + index * equipment_module_entry::STRIDE;
            let definition = usize::try_from(pool_u64(log, entry + 8)).ok().filter(|&d| d != 0);
            let Some(definition) = definition else {
                continue;
            };
            let identity = vspan(log, memory, Some(definition), 12, M_MODULES);
            let _ = vspan(log, memory, definition.checked_add(cequipment_module::ACTIVE), 1, M_MODULES);
            let name = vspan(log, memory, definition.checked_add(cequipment_module::NAME), 32, M_MODULES);
            let (Some(_), Some(name)) = (identity, name) else {
                continue;
            };
            let mut name_header = [0u8; 32];
            name_header.copy_from_slice(&log.bytes[name..name + 32]);
            if let Some(literal) = parse_variant_literal(&name_header, 0) {
                copy_variant_literal(log, memory, &literal, M_MODULES);
            }
        }
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `walk_stats`' read set: the five fixed stat-cache gathers per
/// variant. No discovery beyond the census pointers.
fn copy_variant_stats(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    variants: &[cequipment_variant::RegistryRow],
) {
    for variant in variants {
        let pointer = variant.pointer();
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::SPEED_CACHE_CANDIDATE), 32, M_STATS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::MAXIMUM_SPEED_CACHE_CANDIDATE), 8, M_STATS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::RELIABILITY_CACHE_CANDIDATE), 8, M_STATS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::AIR_DEFENSE_CACHE_CANDIDATE), 24, M_STATS);
        let _ = vspan(log, memory, pointer.checked_add(cequipment_variant::BUILD_COST_CACHE_CANDIDATE), 8, M_STATS);
        if log.overflowed {
            return;
        }
    }
}

/// Mirror of `cequipment_upgrades_instance::walk`'s read set: instance
/// header, entry records, definition fields and the name literal.
fn copy_variant_upgrades(
    log: &mut RawLog<4>,
    memory: &impl Memory,
    variants: &[cequipment_variant::RegistryRow],
) {
    for variant in variants {
        let pointer = variant.pointer();
        let header = vspan(log, memory, pointer.checked_add(cequipment_variant::UPGRADES), 24, M_UPGRADES);
        let Some(header) = header else {
            continue;
        };
        let mut raw = [0u8; 16];
        raw.copy_from_slice(&log.bytes[header + 8..header + 24]);
        let array = match Array::from_header(raw, equipment_upgrade_entry::STRIDE) {
            Some(array) => array,
            None => continue,
        };
        for index in 0..array.count as usize {
            let entry_addr = (index.checked_mul(equipment_upgrade_entry::STRIDE))
                .and_then(|offset| array.data.checked_add(offset));
            let entry = vspan(log, memory, entry_addr, 9, M_UPGRADES);
            let Some(entry) = entry else {
                continue;
            };
            let definition = usize::try_from(pool_u64(log, entry)).ok().filter(|&d| d != 0);
            let Some(definition) = definition else {
                continue;
            };
            let identity = vspan(log, memory, Some(definition), 12, M_UPGRADES);
            let name = vspan(log, memory, definition.checked_add(cequipment_upgrade::NAME), 32, M_UPGRADES);
            let (Some(_), Some(name)) = (identity, name) else {
                continue;
            };
            let mut name_header = [0u8; 32];
            name_header.copy_from_slice(&log.bytes[name..name + 32]);
            if let Some(literal) = parse_variant_literal(&name_header, 0) {
                copy_variant_literal(log, memory, &literal, M_UPGRADES);
            }
        }
        if log.overflowed {
            return;
        }
    }
}

impl Captured {
    /// Game-thread raw copy for a slot-3 refresh hour. The registry census
    /// already ran game-side and feeds both production (as always) and this
    /// copy. Copies `first` always and `second` while any variant table still
    /// needs double-read checks, compares the raw logs, and records
    /// copy-only costs plus the existing checked/mismatches bookkeeping.
    /// Returns whether the payload is complete (`raw_ok`); on staging
    /// overflow the caller falls back to the typed walk and enqueues nothing.
    pub(crate) fn copy_variants_refresh(
        &mut self,
        memory: &impl Memory,
        ctx: VariantsCopyContext,
        variants: &Sources<'_, cequipment_variant::RegistryRow>,
        first: &mut VariantsJob,
        second: Option<&mut VariantsJob>,
    ) -> VariantsCopyReport {
        let started = qpc();
        // A failed or incomplete census blocks every detail check exactly
        // like the typed walk: without a comparable source there is no
        // evidence of stability, even when both detail logs agree empty.
        let first_ok = complete_source(variants.first, variants.first_outcome).is_ok();
        if let Ok(source) = complete_source(variants.first, variants.first_outcome) {
            copy_variant_definitions(&mut first.log, memory, source);
            if !first.log.overflowed {
                copy_variant_modules(&mut first.log, memory, source);
            }
            if !first.log.overflowed {
                copy_variant_stats(&mut first.log, memory, source);
            }
            if !first.log.overflowed {
                copy_variant_upgrades(&mut first.log, memory, source);
            }
        }
        let mut ticks = [0u64; 4];
        ticks[0] = elapsed(started);
        let mut matched = [true; 4];
        if let Some(second_job) = second {
            let second_started = qpc();
            if let Ok(source) = complete_source(variants.second, variants.second_outcome) {
                copy_variant_definitions(&mut second_job.log, memory, source);
                if !second_job.log.overflowed {
                    copy_variant_modules(&mut second_job.log, memory, source);
                }
                if !second_job.log.overflowed {
                    copy_variant_stats(&mut second_job.log, memory, source);
                }
                if !second_job.log.overflowed {
                    copy_variant_upgrades(&mut second_job.log, memory, source);
                }
            }
            ticks[0] += elapsed(second_started);
            let equal =
                first.log.logs_equal(&second_job.log) && !first.log.overflowed && !second_job.log.overflowed;
            for (slot, matched) in matched.iter_mut().enumerate() {
                *matched =
                    first_ok && equal && !first.log.fault[slot] && !second_job.log.fault[slot];
            }
        }
        self.definitions.checked = ctx.needed[0];
        self.definitions.mismatches = usize::from(ctx.needed[0] && !matched[0]);
        self.modules.checked = ctx.needed[1];
        self.modules.mismatches = usize::from(ctx.needed[1] && !matched[1]);
        self.stats.checked = ctx.needed[2];
        self.stats.mismatches = usize::from(ctx.needed[2] && !matched[2]);
        self.upgrades.checked = ctx.needed[3];
        self.upgrades.mismatches = usize::from(ctx.needed[3] && !matched[3]);
        if sources_changed(
            complete_source(variants.first, variants.first_outcome),
            complete_source(variants.second, variants.second_outcome),
        ) {
            // A changed parent census is a mismatch even when both child
            // tables are empty, exactly like the typed walk.
            for (slot, mismatches) in [
                (0, &mut self.definitions.mismatches),
                (1, &mut self.modules.mismatches),
                (2, &mut self.stats.mismatches),
                (3, &mut self.upgrades.mismatches),
            ] {
                if ctx.needed[slot] {
                    *mismatches += 1;
                    matched[slot] = false;
                }
            }
        }
        let raw_ok = !first.log.overflowed;
        VariantsCopyReport {
            raw_ok,
            matched,
            copy_ticks: ticks,
            raw_bytes: first.log.raw_bytes,
        }
    }

    /// Decode-worker interpretation of a raw refresh payload into the pooled
    /// typed rows. Outcomes and arena bytes match the typed walks exactly
    /// because the identical walks run against the recorded spans with the
    /// frame's pooled registry rows as the source; checked/mismatches set by
    /// the raw compare survive. Clears `raw_mode` so the next capture starts
    /// clean.
    pub(crate) fn decode_variants_payload(
        &mut self,
        base: usize,
        source_rows: &[cequipment_variant::RegistryRow],
        source_outcome: Outcome,
        job: &VariantsJob,
        decode_ticks: &mut [u64; 4],
    ) {
        use cequipment_variant::TextOutcome;
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        let source: Result<&[cequipment_variant::RegistryRow], &'static str> =
            match source_outcome {
                Outcome::Rows { len } => source_rows
                    .get(..len)
                    .ok_or("source row count exceeds acquired buffer"),
                Outcome::Unavailable { reason } => Err(reason),
                Outcome::Truncated { .. } => Err("source census incomplete"),
            };
        let result = match source {
            Ok(source) => cequipment_variant::walk_definitions(
                &recorded,
                base,
                source,
                &mut self.definitions.rows,
                &mut self.definition_bytes.first,
            ),
            Err(reason) => TextOutcome {
                outcome: Outcome::Unavailable { reason },
                bytes_len: 0,
                required_bytes: 0,
            },
        };
        self.definitions.outcome = result.outcome;
        self.definition_bytes.len = result.bytes_len;
        self.definition_bytes.required = result.required_bytes;
        let result = match source {
            Ok(source) => equipment_module_container::walk(
                &recorded,
                base,
                source,
                &mut self.modules.rows,
                &mut self.module_bytes.first,
            ),
            Err(reason) => TextOutcome {
                outcome: Outcome::Unavailable { reason },
                bytes_len: 0,
                required_bytes: 0,
            },
        };
        self.modules.outcome = result.outcome;
        self.module_bytes.len = result.bytes_len;
        self.module_bytes.required = result.required_bytes;
        self.stats.outcome = match source {
            Ok(source) => {
                cequipment_variant::walk_stats(&recorded, source, &mut self.stats.rows)
            }
            Err(reason) => Outcome::Unavailable { reason },
        };
        let result = match source {
            Ok(source) => cequipment_upgrades_instance::walk(
                &recorded,
                base,
                source,
                &mut self.upgrades.rows,
                &mut self.upgrade_bytes.first,
            ),
            Err(reason) => TextOutcome {
                outcome: Outcome::Unavailable { reason },
                bytes_len: 0,
                required_bytes: 0,
            },
        };
        self.upgrades.outcome = result.outcome;
        self.upgrade_bytes.len = result.bytes_len;
        self.upgrade_bytes.required = result.required_bytes;
        decode_ticks[0] += elapsed(started);
        self.raw_mode = false;
    }
}

pub(crate) fn complete_source<T>(source: &[T], outcome: Outcome) -> Result<&[T], &'static str> {    match outcome {
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

/// Shared synthetic address space for the variants copy/decode split
/// harness: two registry variants with definitions, modules, stats,
/// upgrades and mixed inline/heap strings.
#[cfg(test)]
pub(crate) mod variants_test_kit {
    use super::{
        Capacities, Captured, Sources, VariantsCopyContext, VariantsCopyReport, VariantsJob, N,
    };
    use engine::{
        cequipment_module, cequipment_type, cequipment_upgrade, cequipment_upgrades_instance,
        cequipment_variant, cgame_state, equipment_module_container, msvc_string, walk::Outcome,
    };

    pub(crate) const BASE: usize = 0x140000000;
    /// Game-state object lives at fixture offset GSTATE in the image.
    pub(crate) const GAME_STATE: usize = 0x11000;
    const ADDRESS: usize = 0x10000;

    const GSTATE: usize = 0x1000;
    const VARR: usize = 0x2000;
    pub(crate) const V1: usize = 0x3000;
    const V2: usize = 0x4000;
    const EQ1: usize = 0x5000;
    const MOD_ENTRY: usize = 0x6100;
    const MOD_DEF: usize = 0x6200;
    const UPG_ENTRY: usize = 0x6400;
    const UPG_DEF: usize = 0x6500;
    pub(crate) const HEAP: usize = 0x7000;

    pub(crate) fn memory(bytes: &[u8]) -> KitMemory<'_> {
        KitMemory {
            inner: engine::memory::SliceMemory { base: ADDRESS, bytes },
            fault: None,
        }
    }

    /// Routes the image-base singleton read to the game-state value; every
    /// other read lands in the small fixture image. An optional fault range
    /// fails overlapping reads for double-read divergence tests.
    pub(crate) struct KitMemory<'a> {
        inner: engine::memory::SliceMemory<'a>,
        pub(crate) fault: Option<(usize, usize)>,
    }

    impl engine::Memory for KitMemory<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if let Some((start, len)) = self.fault {
                let end = address.checked_add(destination.len());
                if end.is_some_and(|end| address < start + len && end > start) {
                    return false;
                }
            }
            if address == BASE + cgame_state::SINGLETON_RVA && destination.len() == 8 {
                destination.copy_from_slice(&(GAME_STATE as u64).to_le_bytes());
                return true;
            }
            self.inner.copy(address, destination)
        }
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

    fn heap_string(bytes: &mut [u8], header: usize, payload: usize, value: &[u8]) {
        assert!(value.len() > msvc_string::INLINE_CAPACITY as usize);
        put(bytes, header, ((ADDRESS + payload) as u64).to_le_bytes());
        put(bytes, header + 16, (value.len() as u64).to_le_bytes());
        put(bytes, header + 24, (value.len() as u64).to_le_bytes());
        bytes[payload..payload + value.len()].copy_from_slice(value);
        bytes[payload + value.len()] = 0;
    }

    fn inline_string(bytes: &mut [u8], header: usize, value: &[u8]) {
        assert!(value.len() <= msvc_string::INLINE_CAPACITY as usize);
        bytes[header..header + 32].fill(0);
        bytes[header..header + value.len()].copy_from_slice(value);
        put(bytes, header + 16, (value.len() as u64).to_le_bytes());
        put(bytes, header + 24, 15_u64.to_le_bytes());
    }

    fn identity(bytes: &mut [u8], object: usize, id: u64) {
        put(
            bytes,
            object,
            ((BASE + cequipment_variant::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(bytes, object + 8, id.to_le_bytes());
    }

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        array(&mut bytes, GSTATE + cgame_state::EQUIPMENT_VARIANTS, VARR, 2);
        pointer(&mut bytes, VARR, V1);
        pointer(&mut bytes, VARR + 8, V2);
        for (variant, id) in [
            (V1, 0x1100_0000_0046_u64),
            (V2, 0x2200_0000_0046_u64),
        ] {
            // Low u32 equals the reference type for the census.
            identity(&mut bytes, variant, id);
            put(
                &mut bytes,
                variant + cequipment_variant::EQUIPMENT_TOKEN,
                0x1234_5678_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                variant + cequipment_variant::EQUIPMENT_TOKEN + 4,
                0x1111_2222_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                variant + cequipment_variant::EQUIPMENT_TOKEN + 8,
                0x3333_4444_u32.to_le_bytes(),
            );
            bytes[variant + cequipment_variant::EQUIPMENT_TOKEN + 12] = 1;
            // Both variants share the equipment definition; V2 exercises
            // shared definitions with inline-only strings.
            pointer(&mut bytes, variant + cequipment_variant::DEFINITION, EQ1);
            bytes[variant + cequipment_variant::OBSOLETE] = 0;
            put(
                &mut bytes,
                variant + cequipment_variant::CLASSIFIER_FLAGS,
                0x0102030405060708_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                variant + cequipment_variant::CLASSIFIER_FLAGS + 8,
                0x99aa_bbcc_u32.to_le_bytes(),
            );
        }
        inline_string(&mut bytes, V1 + cequipment_variant::DISPLAY_NAME, b"disp");
        heap_string(
            &mut bytes,
            V1 + cequipment_variant::DISPLAY_NAME + 32,
            HEAP,
            b"variant-heap-string!",
        );
        inline_string(&mut bytes, V2 + cequipment_variant::DISPLAY_NAME, b"d2");
        inline_string(&mut bytes, V2 + cequipment_variant::DISPLAY_NAME + 32, b"v2");
        // Equipment type for V1 with a heap name.
        put(
            &mut bytes,
            EQ1,
            ((BASE + cequipment_type::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, EQ1 + 8, 0x1234_5678_u32.to_le_bytes());
        heap_string(&mut bytes, EQ1 + cequipment_type::NAME, HEAP + 64, b"equipment-heap-name!");
        // One module on V1.
        array(
            &mut bytes,
            V1 + cequipment_variant::MODULES + equipment_module_container::ENTRIES_CANDIDATE,
            MOD_ENTRY,
            1,
        );
        put(&mut bytes, MOD_ENTRY, 0xdead_beef_u32.to_le_bytes());
        pointer(&mut bytes, MOD_ENTRY + 8, MOD_DEF);
        put(
            &mut bytes,
            MOD_DEF,
            ((BASE + cequipment_module::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, MOD_DEF + 8, 0x5555_6666_u32.to_le_bytes());
        bytes[MOD_DEF + cequipment_module::ACTIVE] = 1;
        inline_string(&mut bytes, MOD_DEF + cequipment_module::NAME, b"mod");
        // Empty module array on V2.
        array(
            &mut bytes,
            V2 + cequipment_variant::MODULES + equipment_module_container::ENTRIES_CANDIDATE,
            0,
            0,
        );
        // Stat caches: distinct first qwords per variant.
        for (variant, base_value) in [(V1, 100_i64), (V2, 200_i64)] {
            let mut value = base_value;
            for (offset, len) in [
                (cequipment_variant::SPEED_CACHE_CANDIDATE, 32),
                (cequipment_variant::MAXIMUM_SPEED_CACHE_CANDIDATE, 8),
                (cequipment_variant::RELIABILITY_CACHE_CANDIDATE, 8),
                (cequipment_variant::AIR_DEFENSE_CACHE_CANDIDATE, 24),
                (cequipment_variant::BUILD_COST_CACHE_CANDIDATE, 8),
            ] {
                for chunk in (0..len).step_by(8) {
                    put(
                        &mut bytes,
                        variant + offset + chunk,
                        value.to_le_bytes(),
                    );
                    value += 1;
                }
            }
        }
        // One upgrade on V1, none on V2.
        put(
            &mut bytes,
            V1 + cequipment_variant::UPGRADES,
            ((BASE + cequipment_upgrades_instance::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(&mut bytes, V1 + cequipment_variant::UPGRADES + 8, UPG_ENTRY, 1);
        pointer(&mut bytes, UPG_ENTRY, UPG_DEF);
        put(
            &mut bytes,
            UPG_DEF,
            ((BASE + cequipment_upgrade::VTABLE_RVA) as u64).to_le_bytes(),
        );
        put(&mut bytes, UPG_DEF + 8, (-5_i32).to_le_bytes());
        inline_string(&mut bytes, UPG_DEF + cequipment_upgrade::NAME, b"upg");
        put(
            &mut bytes,
            V2 + cequipment_variant::UPGRADES,
            ((BASE + cequipment_upgrades_instance::VTABLE_RVA) as u64).to_le_bytes(),
        );
        array(&mut bytes, V2 + cequipment_variant::UPGRADES + 8, 0, 0);
        bytes
    }

    pub(crate) fn registry(
        memory: &impl engine::Memory,
    ) -> (Vec<cequipment_variant::RegistryRow>, Outcome) {
        let mut rows = vec![cequipment_variant::RegistryRow::default(); 8];
        let outcome = cequipment_variant::walk_registry(memory, BASE, GAME_STATE, &mut rows);
        (rows, outcome)
    }

    fn sources<'a>(
        rows: &'a [cequipment_variant::RegistryRow],
        outcome: Outcome,
    ) -> Sources<'a, cequipment_variant::RegistryRow> {
        Sources {
            first: rows,
            first_outcome: outcome,
            second: rows,
            second_outcome: outcome,
        }
    }

    /// Reference typed detail walks with identical pooled buffers.
    pub(crate) fn direct(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> Captured {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let (rows, outcome) = registry(memory);
        let sources = sources(&rows, outcome);
        let needed = [true; N];
        let _ = captured.finish_variants(memory, BASE, &needed, sources, 3, true);
        captured
    }

    /// Raw-copy split: registry census feeds both production and the copy;
    /// two detail passes compared, then the first payload decoded.
    pub(crate) fn split(
        memory: &impl engine::Memory,
        capacities: &Capacities,
    ) -> (Captured, VariantsCopyReport, VariantsJob) {
        let mut captured = Captured::with_capacities(capacities, &[0; N]);
        let (rows, outcome) = registry(memory);
        let sources = sources(&rows, outcome);
        let ctx = VariantsCopyContext { needed: [true; 4] };
        let mut first = VariantsJob::new();
        let mut second = VariantsJob::new();
        let report =
            captured.copy_variants_refresh(memory, ctx, &sources, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        let mut ticks = [0u64; 4];
        captured.decode_variants_payload(BASE, &rows, outcome, &first, &mut ticks);
        (captured, report, first)
    }

    pub(crate) fn assert_matches_direct(split: &Captured, direct: &Captured) {
        assert_eq!(
            split.definitions.outcome, direct.definitions.outcome,
            "definitions outcome"
        );
        assert_eq!(split.modules.outcome, direct.modules.outcome, "modules outcome");
        assert_eq!(split.stats.outcome, direct.stats.outcome, "stats outcome");
        assert_eq!(split.upgrades.outcome, direct.upgrades.outcome, "upgrades outcome");
        assert_eq!(
            &split.definitions.rows[..split.definitions.outcome.len()],
            &direct.definitions.rows[..direct.definitions.outcome.len()],
            "definition rows"
        );
        assert_eq!(
            &split.modules.rows[..split.modules.outcome.len()],
            &direct.modules.rows[..direct.modules.outcome.len()],
            "module rows"
        );
        assert_eq!(
            &split.stats.rows[..split.stats.outcome.len()],
            &direct.stats.rows[..direct.stats.outcome.len()],
            "stat rows"
        );
        assert_eq!(
            &split.upgrades.rows[..split.upgrades.outcome.len()],
            &direct.upgrades.rows[..direct.upgrades.outcome.len()],
            "upgrade rows"
        );
        assert_eq!(
            &split.definition_bytes.first[..split.definition_bytes.len],
            &direct.definition_bytes.first[..direct.definition_bytes.len],
            "definition arena"
        );
        assert_eq!(
            &split.module_bytes.first[..split.module_bytes.len],
            &direct.module_bytes.first[..direct.module_bytes.len],
            "module arena"
        );
        assert_eq!(
            &split.upgrade_bytes.first[..split.upgrade_bytes.len],
            &direct.upgrade_bytes.first[..direct.upgrade_bytes.len],
            "upgrade arena"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert_eq!((manpower[0].amount, manpower[1].amount), (i32::MIN, i32::MAX));
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
            put(&mut bytes, 0x2000 + slot * 8, &(COUNTRY as u64).to_le_bytes());
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
        put(
            &mut bytes,
            NAME_OBJECT + cname_group_member::FLAG_A,
            &[1],
        );
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
                if address == DEPLOYMENT + cmilitary_deployment::TRAINING
                    && self.copies.get() > 1
                {
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
        put(&mut broken, STATUS + deployment::CONVEYORS + 12, &2_u32.to_le_bytes());
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

    fn split_capacities() -> Capacities {
        Capacities {
            rows: [256; N],
            definition_bytes: 8192,
            module_bytes: 8192,
            upgrade_bytes: 8192,
            conveyor_name_bytes: 64,
            line_custom_name_bytes: 64,
            license_handles: 64,
        }
    }

    #[test]
    fn split_copy_matches_direct_walk() {
        use super::variants_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        kit::assert_matches_direct(&split, &direct);
        // Spot-checks: definition identity and heap strings, module and
        // upgrade rows, and stat values.
        assert_eq!(split.definitions.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.definitions.rows[0].identity.equipment_token, 0x1234_5678);
        assert_eq!(
            &split.definition_bytes.first[split.definitions.rows[0].variant_name.start
                ..split.definitions.rows[0].variant_name.start
                    + split.definitions.rows[0].variant_name.len],
            b"variant-heap-string!"
        );
        assert_eq!(split.modules.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.modules.rows[0].definition_token, 0x5555_6666);
        assert_eq!(split.stats.outcome, Outcome::Rows { len: 2 });
        assert_eq!(split.stats.rows[0].values[0], 100);
        assert_eq!(split.stats.rows[1].values[0], 200);
        assert_eq!(split.upgrades.outcome, Outcome::Rows { len: 1 });
        assert_eq!(split.upgrades.rows[0].definition_token, -5);
    }

    #[test]
    fn split_copy_matches_direct_walk_under_truncation() {
        use super::variants_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = Capacities {
            rows: [1; N],
            definition_bytes: 8,
            module_bytes: 8,
            upgrade_bytes: 8,
            conveyor_name_bytes: 0,
            line_custom_name_bytes: 0,
            license_handles: 0,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, report, _) = kit::split(&memory, &capacities);
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        kit::assert_matches_direct(&split, &direct);
    }

    #[test]
    fn split_copy_matches_direct_walk_on_fault_matrix() {
        use super::variants_test_kit as kit;
        // (fault offset from the image base, expected raw-compare verdicts in
        // family order). Outcomes including reason strings must match the
        // typed walk exactly; verdicts follow the fault-attribution masks.
        for (offset, matched) in [
            (0x1708, [false, false, false, false]),
            (kit::V1, [false, false, false, false]),
            (
                kit::V1 + cequipment_variant::DEFINITION,
                [false, true, true, true],
            ),
            (kit::HEAP, [false, true, true, true]),
            (
                kit::V1 + cequipment_variant::MODULES + 8,
                [true, false, true, true],
            ),
            (
                kit::V1 + cequipment_variant::SPEED_CACHE_CANDIDATE,
                [true, true, false, true],
            ),
            (
                kit::V1 + cequipment_variant::UPGRADES,
                [true, true, true, false],
            ),
        ] {
            let bytes = kit::fixture();
            // Fixture offsets address the image base directly.
            let fault = 0x10000 + offset;
            let mut memory = kit::memory(&bytes);
            memory.fault = Some((fault, 1));
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
        use super::variants_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let (rows, outcome) = kit::registry(&memory);
        let sources = Sources {
            first: rows.as_slice(),
            first_outcome: outcome,
            second: rows.as_slice(),
            second_outcome: outcome,
        };
        let ctx = VariantsCopyContext {
            needed: [true; 4],
        };
        let mut first = VariantsJob::new();
        let mut second = VariantsJob::new();
        let report =
            captured.copy_variants_refresh(&memory, ctx, &sources, &mut first, Some(&mut second));
        assert!(report.raw_ok);
        assert_eq!(report.matched, [true; 4]);
        // Same-hour tearing between the passes changes the log.
        let mut torn = bytes.clone();
        torn[kit::V1 + cequipment_variant::OBSOLETE] ^= 0xff;
        let torn_memory = kit::memory(&torn);
        let (torn_rows, torn_outcome) = kit::registry(&torn_memory);
        let torn_sources = Sources {
            first: torn_rows.as_slice(),
            first_outcome: torn_outcome,
            second: torn_rows.as_slice(),
            second_outcome: torn_outcome,
        };
        let ctx = VariantsCopyContext {
            needed: [false; 4],
        };
        let mut after = VariantsJob::new();
        let after_report =
            captured.copy_variants_refresh(&memory, ctx, &sources, &mut after, None);
        assert!(after_report.raw_ok);
        let mut after_torn = VariantsJob::new();
        let torn_report =
            captured.copy_variants_refresh(&torn_memory, ctx, &torn_sources, &mut after_torn, None);
        assert!(torn_report.raw_ok);
        assert!(!after.log.logs_equal(&after_torn.log));
        let mut repeat = VariantsJob::new();
        let repeat_report =
            captured.copy_variants_refresh(&memory, ctx, &sources, &mut repeat, None);
        assert!(repeat_report.raw_ok);
        assert!(after.log.logs_equal(&repeat.log));
    }

    #[test]
    fn staging_overflow_falls_back_to_typed_walk() {
        use super::variants_test_kit as kit;
        let bytes = kit::fixture();
        let memory = kit::memory(&bytes);
        let capacities = split_capacities();
        let mut captured = Captured::with_capacities(&capacities, &[0; N]);
        let (rows, outcome) = kit::registry(&memory);
        let sources = Sources {
            first: rows.as_slice(),
            first_outcome: outcome,
            second: rows.as_slice(),
            second_outcome: outcome,
        };
        let ctx = VariantsCopyContext {
            needed: [true; 4],
        };
        let mut tiny = VariantsJob {
            log: crate::raw::RawLog::new(8, 8),
        };
        let report = captured.copy_variants_refresh(&memory, ctx, &sources, &mut tiny, None);
        assert!(!report.raw_ok);
        assert!(tiny.log.overflowed);
        assert!(tiny.grow_for_next());
    }
}
