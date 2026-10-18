//! Production/equipment acquisition. Only construction/growth and conversion
//! allocate; worker-owned variants grow during their original acquisition.

use crate::{
    clock::qpc,
    projection::{Projection, scalar_table},
    raw::RawLog,
    read_accounting,
    table_capture::{Arena, CaptureRows},
    table_conversion::{table, unavailable},
};
use engine_allocation::{copy_vec, owned_vec};
use engine_deployment::{
    army_upgrade_request_row, carmy_upgrades_requests, carmy_upgrades_status, cdeployment_status,
    cmilitary_deployment, cmilitary_deployment_conveyor, cmilitary_deployment_line,
    cupgrade_delivery, manpower_value_entry,
};
use engine_equipment::{cequipment_upgrades_instance, equipment_module_container};
use engine_native::{
    Memory, carmy_manpower_values, ccountry, cequipment_production_license,
    cequipment_production_line, cequipment_variant, cequipment_variant_pool, cgame_state,
    cname_group_member, cpdx_array::Array, cproduction_line, cproduction_status, equipment_entry,
    msvc_string, walk::Outcome,
};
use replay_data::{rows, rows::TableId, rows::tables};
use replay_frame::{frame::TableInput, frame::TableView};

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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Capacities {
    pub rows: [usize; N],
    pub definition_bytes: usize,
    pub conveyor_name_bytes: usize,
    pub line_custom_name_bytes: usize,
    pub license_handles: usize,
}

impl Capacities {
    /// Advisory attach/refresh demand includes the embedded pool, army-owned
    /// requests and every delivery, including groups absent from the active
    /// country registry.
    pub fn presize(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
    ) -> Result<(), &'static str> {
        let required = |outcome| match outcome {
            Outcome::Rows { len } => Ok(len),
            Outcome::Truncated { len, unread_rows } => len
                .checked_add(unread_rows as usize)
                .ok_or("production sizing row demand overflow"),
            Outcome::Unavailable { reason } => Err(reason),
        };
        for (index, outcome) in [
            (
                0,
                cproduction_status::walk_lines(memory, base, state, &mut []),
            ),
            (
                1,
                cequipment_variant_pool::walk_stockpiles(memory, base, state, &mut []),
            ),
            (
                11,
                cproduction_status::walk_licenses(memory, base, state, &mut []),
            ),
        ] {
            crate::table_capture::grow_capacity(&mut self.rows[index], required(outcome)?);
        }
        let licenses =
            cproduction_status::walk_license_records(memory, base, state, &mut [], &mut []);
        crate::table_capture::grow_capacity(&mut self.rows[16], required(licenses.outcome)?);
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut demand = [0_usize; 5];
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let roster = country
                .pointer
                .checked_add(ccountry::DIVISIONS)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("division detail sizing roster unreadable or invalid")?;
            engine_native::cpdx_array::pointers(
                memory,
                roster,
                "division detail sizing pointer unreadable",
                |_, pointer| {
                    if pointer == 0 {
                        return Ok(());
                    }
                    let at = |offset| {
                        pointer
                            .checked_add(offset)
                            .ok_or("division detail sizing address overflow")
                    };
                    if engine_native::read::<u64>(memory, pointer)
                        != base
                            .checked_add(engine_native::carmy::VTABLE_RVA)
                            .map(|value| value as u64)
                        || engine_native::read::<u64>(
                            memory,
                            at(engine_native::carmy::SECONDARY_BASE)?,
                        ) != base
                            .checked_add(engine_native::carmy::SECONDARY_VTABLE_RVA)
                            .map(|value| value as u64)
                    {
                        return Err("division detail sizing army identity invalid");
                    }
                    let division =
                        engine_native::read::<u64>(memory, at(engine_native::cunit::HANDLE)?)
                            .filter(|value| *value != 0)
                            .ok_or("division detail sizing handle unreadable or null")?;
                    let source = [cequipment_variant_pool::DivisionSource {
                        pointer,
                        division,
                        country: country.id,
                        template: 0,
                    }];
                    let equipment =
                        cequipment_variant_pool::walk_divisions(memory, base, &source, &mut []);
                    let upgrades = carmy_upgrades_status::walk(
                        memory,
                        base,
                        &source,
                        &mut [],
                        &mut [],
                        &mut [],
                        &mut [],
                    );
                    for (total, outcome) in demand.iter_mut().zip([
                        equipment,
                        upgrades.groups,
                        upgrades.requests,
                        upgrades.deliveries,
                        upgrades.produced,
                    ]) {
                        let rows = match outcome {
                            Outcome::Rows { len } => len,
                            Outcome::Truncated { len, unread_rows } => len
                                .checked_add(unread_rows as usize)
                                .ok_or("division detail sizing overflow")?,
                            Outcome::Unavailable { reason } => return Err(reason),
                        };
                        *total = total
                            .checked_add(rows)
                            .ok_or("division detail sizing overflow")?;
                    }
                    Ok(())
                },
            )?;
        }
        for (index, rows) in [2, 7, 8, 9, 10].into_iter().zip(demand) {
            self.rows[index] = self.rows[index].max(rows);
        }
        Ok(())
    }
}

/// Variant leaves loaned to the worker-only acquisition job.
pub struct Variants {
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

    pub fn vacant() -> Self {
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

    pub fn allocated_bytes(&self) -> usize {
        self.definitions.allocated_bytes()
            + self.modules.allocated_bytes()
            + self.stats.allocated_bytes()
            + self.upgrades.allocated_bytes()
            + self.definition_bytes.allocated_bytes()
    }

    fn grow(&mut self, capacities: &mut Capacities) {
        capacities.rows[3] = capacities.rows[3].max(self.definitions.outcome.len());
        self.definitions.grow(&mut capacities.rows[3]);
        capacities.rows[4] = capacities.rows[4].max(self.modules.outcome.len());
        self.modules.grow(&mut capacities.rows[4]);
        capacities.rows[5] = capacities.rows[5].max(self.stats.outcome.len());
        self.stats.grow(&mut capacities.rows[5]);
        capacities.rows[6] = capacities.rows[6].max(self.upgrades.outcome.len());
        self.upgrades.grow(&mut capacities.rows[6]);
        capacities.definition_bytes = capacities.definition_bytes.max(self.definition_bytes.len);
        self.definition_bytes.grow(&mut capacities.definition_bytes);
    }

    pub fn fail(&mut self, reason: &'static str, hour: i32, force_full: bool) {
        self.reset();
        for (outcome, table) in [
            (&mut self.definitions.outcome, TABLES[3]),
            (&mut self.modules.outcome, TABLES[4]),
            (&mut self.stats.outcome, TABLES[5]),
            (&mut self.upgrades.outcome, TABLES[6]),
        ] {
            *outcome = Outcome::Unavailable {
                reason: if replay_data::cadence::should_walk(table, hour, force_full) {
                    reason
                } else {
                    replay_data::cadence::SKIPPED_REASON
                },
            };
        }
    }

    /// Read variant fields from the current complete registry census.
    pub fn capture(
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
        if !replay_data::cadence::should_walk(TableId::VariantModules, hour, force_full) {
            let skipped = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
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
    pub divisions_raw_mode: bool,
    /// Raw-copy split gate for scheduled production rest (lines 0,
    /// stockpiles 1, licences 11, deployment 12..15, licence records 16).
    /// Set when an acquisition job returns a valid raw payload; `acquire`
    /// then skips those typed walks and the decode worker fills the rows
    /// (plus the licence/name arenas) before the ready handoff. Preserved
    /// across `reset` (set per hour, cleared by the worker after decode).
    pub rest_raw_mode: bool,
    /// The acquisition's selected detail activity survives through decode failures.
    division_details_active: bool,
}

impl Captured {
    pub fn vacant() -> Self {
        Self::with_capacities(&Capacities {
            rows: [0; N],
            definition_bytes: 0,
            conveyor_name_bytes: 0,
            line_custom_name_bytes: 0,
            license_handles: 0,
        })
    }

    pub fn fail_final_capture(&mut self, hour: i32, force_full: bool, reason: &'static str) {
        if replay_data::cadence::should_walk(TableId::ProductionLines, hour, force_full) {
            self.fail_production_rest_capture(reason);
        }
        if replay_data::cadence::should_walk(TableId::DivisionEquipment, hour, force_full) {
            let outcome = Outcome::Unavailable { reason };
            self.division_equipment.fail(outcome);
            self.groups.fail(outcome);
            self.requests.fail(outcome);
            self.deliveries.fail(outcome);
            self.produced.fail(outcome);
        }
    }

    pub fn take_variants(&mut self) -> Variants {
        std::mem::replace(&mut self.variants, Variants::vacant())
    }

    pub fn restore_variants(&mut self, variants: Variants) {
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
        capacities.rows[7] = capacities.rows[7].max(self.groups.outcome.len());
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

    pub fn division_details_active(&self) -> bool {
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
        self.acquire_buffers(memory, base, game_state, divisions, hour, force_full, true)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn acquire_fixed(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        divisions: Sources<'_, cequipment_variant_pool::DivisionSource>,
        hour: i32,
        force_full: bool,
    ) -> [u64; N] {
        self.acquire_buffers(memory, base, game_state, divisions, hour, force_full, false)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The fixed reference and worker use identical native readers"
    )]
    fn acquire_buffers(
        &mut self,
        memory: &impl Memory,
        base: usize,
        game_state: usize,
        divisions: Sources<'_, cequipment_variant_pool::DivisionSource>,
        hour: i32,
        force_full: bool,
        growing: bool,
    ) -> [u64; N] {
        let mut ticks = [0; N];
        let rest_active =
            replay_data::cadence::should_walk(TableId::ProductionLines, hour, force_full);
        let divisions_active =
            replay_data::cadence::should_walk(TableId::DivisionEquipment, hour, force_full);
        self.division_details_active = divisions_active;
        if !rest_active {
            self.fail_production_rest_capture(replay_data::cadence::SKIPPED_REASON);
            self.conveyor_name_bytes.reset();
            self.line_custom_name_bytes.reset();
            self.license_handles.reset();
        }
        if !divisions_active {
            let skipped = Outcome::Unavailable {
                reason: replay_data::cadence::SKIPPED_REASON,
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
                cproduction_status::walk_lines_buffer(
                    memory,
                    base,
                    game_state,
                    engine_native::walk::Buffer::for_worker(rows, growing),
                )
            });
            ticks[0] = elapsed(begin);
            reads.select(TABLES[1]);
            let begin = qpc();
            capture_rows(&mut self.stockpiles, |rows| {
                cequipment_variant_pool::walk_stockpiles_buffer(
                    memory,
                    base,
                    game_state,
                    engine_native::walk::Buffer::for_worker(rows, growing),
                )
            });
            ticks[1] = elapsed(begin);
        }
        if divisions_active && !self.divisions_raw_mode {
            let mut reads = read_accounting::table(TABLES[2]);
            let begin = qpc();
            let source = complete_source(divisions.rows, divisions.outcome);
            self.division_equipment.outcome = match source {
                Ok(source) if growing => cequipment_variant_pool::walk_divisions_growing(
                    memory,
                    base,
                    source,
                    &mut self.division_equipment.rows,
                ),
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
                Ok(source) if growing => carmy_upgrades_status::walk_growing(
                    memory,
                    base,
                    source,
                    &mut self.groups.rows,
                    &mut self.requests.rows,
                    &mut self.deliveries.rows,
                    &mut self.produced.rows,
                ),
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
                cproduction_status::walk_licenses_buffer(
                    memory,
                    base,
                    game_state,
                    engine_native::walk::Buffer::for_worker(rows, growing),
                )
            });
            let records = cproduction_status::walk_license_records_buffer(
                memory,
                base,
                game_state,
                engine_native::walk::Buffer::for_worker(&mut self.license_records.rows, growing),
                engine_native::walk::Buffer::for_worker(&mut self.license_handles.first, growing),
            );
            self.license_records.outcome = records.outcome;
            self.license_handles.len = records.handles_len;
            self.license_handles.required = records.required_handles;
            ticks[11] = elapsed(begin);
            reads.select(TABLES[12]);
            let begin = qpc();
            let deployment = self.deployment(memory, base, game_state, growing);
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
        growing: bool,
    ) -> cdeployment_status::Outcomes {
        cdeployment_status::walk_buffer(
            memory,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(&mut self.conveyors.rows, growing),
            engine_native::walk::Buffer::for_worker(&mut self.deployment_lines.rows, growing),
            engine_native::walk::Buffer::for_worker(&mut self.deployment_manpower.rows, growing),
            engine_native::walk::Buffer::for_worker(&mut self.deployment_equipment.rows, growing),
            engine_native::walk::Buffer::for_worker(&mut self.conveyor_name_bytes.first, growing),
            engine_native::walk::Buffer::for_worker(
                &mut self.line_custom_name_bytes.first,
                growing,
            ),
        )
    }

    /// Decode-worker interpretation of a scheduled production-rest payload.
    /// Replays the existing lines, stockpile, licence, deployment and licence-
    /// record walks against the recorded spans, installing identical rows,
    /// arenas, truncation counts and failure reasons into the frame's pooled
    /// buffers. Clears `rest_raw_mode` so the next capture starts clean.
    pub fn decode_production_rest_payload(
        &mut self,
        base: usize,
        game_state: usize,
        job: &ProductionRestJob,
        decode_ticks: &mut [u64; 8],
    ) {
        let started = qpc();
        let recorded = crate::raw::Recorded::build(&job.log);
        self.lines.outcome = cproduction_status::walk_lines_buffer(
            &recorded,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(
                &mut self.lines.rows,
                job.log.worker_growth_enabled(),
            ),
        );
        decode_ticks[0] += elapsed(started);
        let started = qpc();
        self.stockpiles.outcome = cequipment_variant_pool::walk_stockpiles_buffer(
            &recorded,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(
                &mut self.stockpiles.rows,
                job.log.worker_growth_enabled(),
            ),
        );
        decode_ticks[1] += elapsed(started);
        let started = qpc();
        self.licenses.outcome = cproduction_status::walk_licenses_buffer(
            &recorded,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(
                &mut self.licenses.rows,
                job.log.worker_growth_enabled(),
            ),
        );
        let result = cproduction_status::walk_license_records_buffer(
            &recorded,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(
                &mut self.license_records.rows,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.license_handles.first,
                job.log.worker_growth_enabled(),
            ),
        );
        self.license_records.outcome = result.outcome;
        self.license_handles.len = result.handles_len;
        self.license_handles.required = result.required_handles;
        decode_ticks[2] += elapsed(started);
        let started = qpc();
        let walked = cdeployment_status::walk_buffer(
            &recorded,
            base,
            game_state,
            engine_native::walk::Buffer::for_worker(
                &mut self.conveyors.rows,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.deployment_lines.rows,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.deployment_manpower.rows,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.deployment_equipment.rows,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.conveyor_name_bytes.first,
                job.log.worker_growth_enabled(),
            ),
            engine_native::walk::Buffer::for_worker(
                &mut self.line_custom_name_bytes.first,
                job.log.worker_growth_enabled(),
            ),
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

pub mod registry;
mod rest;
#[cfg(any(test, feature = "test-support"))]
pub use rest::copy_production_rest_refresh;
pub use rest::{
    ProductionRestCopyContext, ProductionRestCopyReport, ProductionRestJob,
    copy_production_rest_refresh_growing,
};

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

    #[cfg(any(test, feature = "test-support"))]
    pub fn tables(&self) -> [TableView; N] {
        self.table_inputs(None).map(|input| match input {
            TableInput::Sampled(view) => view,
            _ => unreachable!("projection without a baseline always samples rows"),
        })
    }

    /// Fresh owned division payloads compare directly with the encoder's exact
    /// complete baseline before allocating replay rows. Other production tables
    /// retain their existing arena validation and conversion paths.
    pub fn table_inputs(&self, mut projection: Option<&mut Projection<'_>>) -> [TableInput; N] {
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
                    produced_raw: replay_data::rows::RetiredI64::default(),
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
            equipment: owned_vec(copy_vec(
                span(handles, r.equipment_start, r.equipment_len).unwrap(),
            )),
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
        name: owned_vec(copy_vec(name(bytes, r.name_start, r.name_len).unwrap())),
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
        custom_name: owned_vec(copy_vec(
            name(bytes, r.custom_name_start, r.custom_name_len).unwrap(),
        )),
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

pub fn complete_source<T>(source: &[T], outcome: Outcome) -> Result<&[T], &'static str> {
    match outcome {
        Outcome::Rows { len } => source
            .get(..len)
            .ok_or("source row count exceeds acquired buffer"),
        Outcome::Unavailable { reason } => Err(reason),
        Outcome::Truncated { .. } => Err("source census incomplete"),
    }
}

fn capture_rows<R>(rows: &mut CaptureRows<R>, mut walk: impl FnMut(&mut Vec<R>) -> Outcome) {
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
            display_name: owned_vec(copy_vec(text(bytes, r.display_name).unwrap())),
            variant_name: owned_vec(copy_vec(text(bytes, r.variant_name).unwrap())),
            explicit_entity: owned_vec(copy_vec(text(bytes, r.explicit_entity).unwrap())),
            icon_active: r.icon_active,
            icon_kind: r.icon_kind,
            icon: owned_vec(copy_vec(text(bytes, r.icon).unwrap())),
            version: rows::Recorded::Value(r.version),
        }
    })
}

/// Shared synthetic address space for the scheduled production-rest
/// copy/decode split: one country with one line, one stockpile entry,
/// one licence, one licence record with one handle, and a one-conveyor
/// deployment queue with one line, manpower and equipment.
#[cfg(any(test, feature = "test-support"))]
pub mod production_rest_test_kit;

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "variants_scale_tests.rs"]
mod variant_scale_tests;
