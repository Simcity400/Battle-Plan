use super::*;

#[test]
fn production_daily_groups_skip_native_reads_outside_their_slots() {
    use production_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
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
        reason: replay_data::cadence::SKIPPED_REASON,
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
                        engine_native_memory::memory::ReadCounts::ZERO
                    );
                }
            }
        }
        assert_eq!(counts[TABLES[0].index()].spans > 0, rest_active);
        assert_eq!(counts[TABLES[2].index()].spans > 0, divisions_active);
    }
}

#[test]
fn variant_census_reads_only_its_daily_slot_or_an_explicit_recovery() {
    use production_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::CountedMemory::new(engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    });
    let registry = [cequipment_variant::RegistryRow::read(&memory.0, 0, kit::VARIANT).unwrap()];
    let mut captured = Captured::with_capacities(&kit::capacities());
    let skipped = Outcome::Unavailable {
        reason: replay_data::cadence::SKIPPED_REASON,
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
        assert_eq!(
            captured.variants.definitions.outcome != skipped,
            details_active
        );
        assert_eq!(counts[TABLES[3].index()].spans > 0, details_active);
        assert_eq!(counts[TABLES[4].index()].spans > 0, details_active);
        for (index, table) in TABLES.iter().enumerate().take(7).skip(3) {
            if details_active {
                assert_ne!(captured.outcomes()[index], skipped);
            } else {
                assert_eq!(captured.outcomes()[index], skipped);
                assert_eq!(ticks[index], 0);
                assert_eq!(
                    counts[table.index()],
                    engine_native_memory::memory::ReadCounts::ZERO
                );
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
    let _hint = crate::country_scale::hint_guard();
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
    assert_eq!(grown, initial, "unused census capacity is not live demand");
    assert_eq!(grown, expected(&job));
    job.clear();
    assert_eq!(LOCAL.snapshot().current.auxiliary_vectors, grown);
    drop(job);
    assert_eq!(LOCAL.snapshot().current, Capacity::default());
}

use cequipment_variant::{TextOutcome, TextSpan};
use replay_data::rows::TableSchema;

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
        values: std::array::from_fn(|i| Some(if i == 0 { i64::MIN } else { i as i64 })),
        ..Default::default()
    };
    let views = captured.tables();
    let line = &complete::<tables::ProductionLines>(&views[0])[0];
    assert_eq!(
        (line.handle, line.variant, line.mio),
        (u64::MAX, variant, u64::MAX - 1)
    );
    assert_eq!(
        (line.cost_raw, line.speed_raw, line.produced_raw),
        (i64::MIN, -100_001, replay_data::rows::RetiredI64::default())
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

    let mut baseline = std::array::from_fn(|_| unavailable("not part of this fixture"));
    for (id, view) in TABLES.into_iter().zip(views) {
        baseline[id.index()] = view;
    }
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    encoder
        .encode(
            replay_frame::frame::DecodedFrame {
                branch: 1,
                hour: 0,
                tables: baseline.map(Into::into),
                metrics: replay_frame::frame::Metrics::default(),
            },
            false,
        )
        .unwrap();
    let scratch = crate::projection::ProjectionScratch::default();
    let mut projection = Projection::new(&encoder, 1, 1, false, &scratch);
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
        version: u8::MAX,
        design: Default::default(),
        archetype_pointer: 0,
        organisation_pointer: None,
        source_links: None,
    };
    captured.variants.modules.outcome = Outcome::Rows { len: 2 };
    captured.variants.modules.rows[0] = equipment_module_container::Row {
        variant: u64::MAX,
        source_ordinal: 7,
        slot: 3,
        definition_token: u32::MAX,
        definition_active: 0xfe,
        definition_pointer: 0,
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
        definition_pointer: usize::MAX,
    };
    let views = captured.tables();
    let definition = &complete::<tables::VariantDefinitions>(&views[3])[0];
    assert_eq!(
        definition
            .version
            .require("VariantDefinitions.version")
            .unwrap(),
        &u8::MAX
    );
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
    captured.variants.dictionary = Some(std::sync::Arc::new(
        rows::TableRows::VariantBonusSnapshots(Vec::new()),
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
        captured.stockpiles.rows.len() >= 7 && captured.variants.definition_bytes.first.len() >= 33
    );
    let allocated = captured.allocated_bytes();
    captured.reset();
    captured.grow(&mut capacities);
    assert_eq!(captured.allocated_bytes(), allocated);
}

#[test]
fn variant_loan_retains_buffers_and_adopts_worker_observations() {
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
    variants.definitions.rows.reserve(5000);
    variants.definition_bytes.first.reserve(5000);
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
    assert_eq!(capacities.rows[3], 2);
    assert_eq!(capacities.definition_bytes, 16);
    assert_eq!(captured.allocated_bytes(), allocated);
    let mut variants = captured.take_variants();
    variants.definitions.outcome = Outcome::Rows { len: 33 };
    variants.definition_bytes.len = 65;
    captured.restore_variants(variants);
    captured.grow(&mut capacities);
    assert!(capacities.rows[3] >= 33 && capacities.rows[3] <= 128);
    assert!(capacities.definition_bytes >= 65 && capacities.definition_bytes <= 256);
    let mut variants = captured.take_variants();
    variants.fail("variant worker stopped", 0, false);
    captured.restore_variants(variants);
    for (index, table) in TABLES.iter().enumerate().take(7).skip(3) {
        assert_eq!(
            captured.outcomes()[index],
            Outcome::Unavailable {
                reason: if replay_data::cadence::should_walk(*table, 0, false) {
                    "variant worker stopped"
                } else {
                    replay_data::cadence::SKIPPED_REASON
                }
            }
        );
    }
}

#[test]
fn production_rest_split_copy_matches_direct_walk() {
    use super::production_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
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

#[test]
fn production_rest_split_copy_skips_reserved_aliases_and_copies_each_country_once() {
    use super::production_rest_test_kit as kit;
    const RESERVED: usize = 0x2800;
    let mut bytes = kit::fixture();
    let mut put = |address: usize, value: &[u8]| {
        bytes[address - kit::BASE..address - kit::BASE + value.len()].copy_from_slice(value);
    };
    for header in [cgame_state::COUNTRIES, cgame_state::COUNTRY_LINKS] {
        put(kit::BASE + header + 8, &4_u32.to_le_bytes());
        put(kit::BASE + header + 12, &4_u32.to_le_bytes());
    }
    for slot in [0, 3] {
        put(0x2000 + slot * 8, &(RESERVED as u64).to_le_bytes());
    }
    put(RESERVED, &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes());
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    let capacities = kit::capacities();
    let direct = kit::direct(&memory, &capacities);
    let (split, _, job) = kit::split(&memory, &capacities);
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(split.lines.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.deployment_lines.outcome, Outcome::Rows { len: 1 });
    assert_eq!(split.license_records.outcome, Outcome::Rows { len: 1 });
    assert_eq!(job.census.len(), 1);
}

struct DefinedMemory<'a>(&'a [u8]);

impl Memory for DefinedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        if let Some(index) = cequipment_production_line::MAX_FACTORY_DEFINE_RVAS
            .iter()
            .position(|&rva| rva.0 == address)
        {
            destination.copy_from_slice(&(index as i32 + 3).to_le_bytes());
            return true;
        }
        engine_native_memory::memory::SliceMemory {
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
    let (outcome, line) = row(&bytes);
    assert_eq!(outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (line.max_factories, line.efficiency_raw),
        (150, (40..52).sum::<i64>() * 32_768 / 12)
    );
    let flag = at(kit::EQUIPMENT_TYPE + engine_native_parts::cequipment_type::MAX_MIL_FACTORIES + 4);
    bytes[flag] = 0;
    let (_, line) = row(&bytes);
    assert_eq!(line.max_factories, 3);
    assert_eq!(line.efficiency_raw, (40 + 41 + 42) * 32_768 / 3);
    let vtable = at(kit::PLINE);
    bytes[vtable..vtable + 8].copy_from_slice(
        &(engine_native_parts::cnaval_production_line::VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    let flags = at(kit::VARIANT + cequipment_variant::CLASSIFIER_FLAGS);
    bytes[flags..flags + 8]
        .copy_from_slice(&cequipment_variant::CAPITAL_SHIP_CLASSIFIER_MASK.to_le_bytes());
    let (_, line) = row(&bytes);
    assert_eq!(line.max_factories, 7);
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
        engine_native_memory::memory::SliceMemory {
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
    let mut job = production_rest_test_kit::job();
    let initial = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    assert!(copy_production_rest_refresh(&initial, fixture_plan_context(), &mut job).raw_ok);
    assert!(!job.census.is_empty());
    job.log.prepare_read_plan();
    let cost = kit::PLINE + cproduction_line::COST - kit::BASE;
    bytes[cost..cost + 8].copy_from_slice(&999_i64.to_le_bytes());
    let memory = engine_native_memory::memory::CountedMemory::new(PlanFixtureMemory {
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
    expected_counts[TABLES[0].index()] = engine_native_memory::memory::ReadCounts {
        plan_calls: 1,
        spans: planned_calls.iter().filter(|&&(_, len)| len != 0).count() as u64,
        bytes: planned_calls.iter().map(|&(_, len)| len as u64).sum(),
        ..engine_native_memory::memory::ReadCounts::ZERO
    };
    assert!(expected_counts[TABLES[0].index()].spans > 0);
    assert_eq!(planned_counts, expected_counts);
    let mut canonical = production_rest_test_kit::job();
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
    let mut job = production_rest_test_kit::job();
    let initial = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    assert!(copy_production_rest_refresh(&initial, fixture_plan_context(), &mut job).raw_ok);
    job.log.prepare_read_plan();
    let next_variant = kit::BASE + bytes.len();
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
        let mut canonical = production_rest_test_kit::job();
        let fresh = copy_production_rest_refresh(&memory, fixture_plan_context(), &mut canonical);
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
    use super::production_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
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
        let mut first = production_rest_test_kit::job();
        let report = crate::production::copy_production_rest_refresh(&memory, ctx, &mut first);
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
    let bytes = kit::many_rows_fixture();
    let memory = engine_native_memory::memory::SliceMemory {
        base: kit::BASE,
        bytes: &bytes,
    };
    for cap in [0, 1, kit::MANY - 1, kit::MANY, kit::MANY + 1, 64] {
        let capacities = Capacities {
            rows: [cap; N],
            definition_bytes: 0,
            conveyor_name_bytes: 64,
            line_custom_name_bytes: 64,
            license_handles: 64,
        };
        let direct = kit::direct(&memory, &capacities);
        let (split, _, _) = kit::split(&memory, &capacities);
        kit::assert_matches_direct(&split, &direct);
        let expected = Outcome::Rows { len: kit::MANY };
        assert_eq!((split.lines.outcome, split.stockpiles.outcome), (expected, expected), "every row is kept from an initial capacity of {cap}");
        let len = expected.len();
        let produced: Vec<i64> = split.lines.rows[..len].iter().map(|row| row.produced_raw).collect();
        assert_eq!(produced, (0..len).map(kit::line_produced).collect::<Vec<_>>(), "lines keep their order");
        let amounts: Vec<i64> = split.stockpiles.rows[..len].iter().map(|row| row.amount_raw).collect();
        assert_eq!(amounts, (0..len).map(kit::stockpile_amount).collect::<Vec<_>>(), "stockpiles keep their order");
    }
}

#[test]
fn production_rest_split_copy_matches_direct_walk_under_truncation() {
    use super::production_rest_test_kit as kit;
    let bytes = kit::fixture();
    let memory = engine_native_memory::memory::SliceMemory {
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
        let mut first = production_rest_test_kit::job();
        let report = crate::production::copy_production_rest_refresh(&memory, ctx, &mut first);
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
fn production_daily_groups_keep_design_certificates_between_scheduled_reads() {
    let mut variants = Variants::vacant();
    variants.organisations_complete = true;
    for hour in 0..24 {
        if !replay_data::cadence::should_walk(TableId::VariantStats, hour, false) {
            variants.fail(replay_data::cadence::SKIPPED_REASON, hour, false);
            assert!(variants.organisations_complete, "skipped hour {hour}");
        }
    }
    let slot = replay_data::cadence::refresh_slot(TableId::VariantStats).unwrap() as i32;
    variants.fail("scheduled read failed", slot, false);
    assert!(!variants.organisations_complete);
    variants.organisations_complete = true;
    variants.fail("requested full read failed", 0, true);
    assert!(!variants.organisations_complete);
}
