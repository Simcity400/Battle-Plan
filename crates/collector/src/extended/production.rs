//! Production/equipment acquisition. Only construction/growth and conversion
//! allocate; the callback fills typed buffers and measures shared graph work.

use crate::{
    sampling::{full_match, qpc},
    spooling::{table, unavailable},
    table_capture::{Arena, CaptureRows},
};
use engine::{
    Memory, army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status,
    cdeployment_status, cequipment_production_line, cequipment_upgrades_instance,
    cequipment_variant, cequipment_variant_pool, cproduction_status, cupgrade_delivery,
    equipment_module_container, walk::Outcome,
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
    pub license_records: CaptureRows<cproduction_status::LicenseRecordRow>,
    pub license_handles: Arena<u64>,
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
            license_records: CaptureRows::new(capacities.rows[16], successful[16]),
            license_handles: Arena::new(capacities.license_handles, successful[16]),
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
        self.license_records
            .grow(&mut capacities.rows[16], successful[16]);
        self.license_handles
            .grow(&mut capacities.license_handles, successful[16]);
    }

    /// Acquire non-variant tables. Each table requests its own consistency
    /// reads; variants use the independently acquired registry census.
    /// Durations are QPC ticks and include comparison work.
    pub fn acquire(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        needed: &[bool],
        divisions: Sources<'_, cequipment_variant_pool::DivisionSource>,
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
            let check = |index: usize, rows: &mut dyn DeploymentRows, outcome| {
                let matched_rows = rows.compare(outcome);
                if !names_matched {
                    rows.add_mismatch();
                }
                needed[index] && matched_rows && names_matched
            };
            matched[12] = check(12, &mut self.conveyors, second.conveyors);
            matched[13] = check(13, &mut self.deployment_lines, second.lines);
            matched[14] = check(14, &mut self.deployment_manpower, second.manpower);
            matched[15] = check(15, &mut self.deployment_equipment, second.equipment);
        }
        ticks[12] = elapsed(begin);
        (matched, ticks)
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
        let (conveyors, lines, manpower, equipment, names) = if second {
            (
                &mut self.conveyors.second,
                &mut self.deployment_lines.second,
                &mut self.deployment_manpower.second,
                &mut self.deployment_equipment.second,
                &mut self.conveyor_name_bytes.second,
            )
        } else {
            (
                &mut self.conveyors.rows,
                &mut self.deployment_lines.rows,
                &mut self.deployment_manpower.rows,
                &mut self.deployment_equipment.rows,
                &mut self.conveyor_name_bytes.first,
            )
        };
        cdeployment_status::walk(
            memory, base, game_state, conveyors, lines, manpower, equipment, names,
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
    ) -> ([bool; N], [u64; N]) {
        assert_eq!(needed.len(), N);
        let mut matched = [false; N];
        let mut ticks = [0; N];
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
            table::<tables::DeploymentLines, _>(
                outcome(&self.deployment_lines),
                &self.deployment_lines.rows,
                |r| rows::DeploymentLineRow {
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
                },
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
    })
}

fn name(arena: &[u8], start: usize, len: usize) -> Option<&[u8]> {
    arena.get(start..start.checked_add(len)?)
}

fn elapsed(begin: i64) -> u64 {
    qpc().saturating_sub(begin).max(0) as u64
}

fn complete_source<T>(source: &[T], outcome: Outcome) -> Result<&[T], &'static str> {
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
        };
        captured.deployment_lines.outcome = Outcome::Rows { len: 1 };
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
}
