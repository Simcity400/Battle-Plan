use super::fresh_reads_tests::{Native, source};
use super::*;
use engine_native_memory::{Memory, walk::Outcome};
use replay_data::rows::*;
use replay_frame::frame::TableInput;
use std::cell::RefCell;

struct Reads<'a, M> {
    memory: &'a M,
    addresses: RefCell<Vec<usize>>,
}
impl<M: Memory> Memory for Reads<'_, M> {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        self.addresses.borrow_mut().push(at);
        self.memory.copy(at, output)
    }
}

const SHIP_BASE: usize = 0x1000_0000;

struct ShipWorld(Native);
impl Memory for ShipWorld {
    fn copy(&self, at: usize, output: &mut [u8]) -> bool {
        let pointer = if at == SHIP_BASE + engine_native::cgame_state::SINGLETON_RVA.0 {
            Some(0x10000_u64)
        } else if at == SHIP_BASE + engine_native_parts::cequipment_database::SINGLETON_RVA.0 {
            Some(0x15000_u64)
        } else {
            None
        };
        if let Some(pointer) = pointer.filter(|_| output.len() == 8) {
            output.copy_from_slice(&pointer.to_le_bytes());
            return true;
        }
        self.0.copy(at, output)
    }
}

#[test]
fn failed_ship_observer_rejects_carry_and_publishes_changed_native_models() {
    use collector_ship_models::ship_model_changes::Changes;
    use collector_ship_models::ship_models::{CaptureContext, Captured};
    use engine_native::{
        ccountry,
        cequipment_variant::{self, pool as cequipment_variant_pool},
        cgame_state,
        cship,
        ctask_force,
        cunit,
    };
    use engine_native_parts::{cequipment_database, cequipment_type, csub_unit_definition};
    use engine_native_memory::msvc_string;
    if !contract_tests::isolated(
        "hook_failures::fresh_producer_tests::failed_ship_observer_rejects_carry_and_publishes_changed_native_models",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_ship_models::ship_model_changes::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    assert!(failed(Family::ShipModels));
    assert!(failed(Family::AirBaseCarry));
    assert_eq!(
        collector_ship_models::ship_model_changes::Observer.quiet(),
        None
    );
    let (state, country, task, doctrine, experience, database, variant, definition, ship) = (
        0x10000, 0x12000, 0x13000, 0x14000, 0x14100, 0x15000, 0x16000, 0x17000, 0x18000,
    );
    let census = cship::Row {
        id: (42_u64 << 32) | u64::from(cship::HANDLE_TYPE),
        task_force: (17_u64 << 32) | u64::from(ctask_force::DEFAULT_HANDLE_TYPE),
        class_id: 7,
        ..Default::default()
    };
    let mut world = ShipWorld(Native(vec![0; 0x20000]));
    let memory = &mut world.0;
    memory.array(state + cgame_state::COUNTRIES, 0x11000, 2);
    memory.array(state + cgame_state::COUNTRY_LINKS, 0x11100, 2);
    memory.word(0x11008, country);
    memory.put(0x11104, &1_u32.to_le_bytes());
    memory.word(country, SHIP_BASE + ccountry::VTABLE_RVA.0);
    memory.put(country + ccountry::ID, &1_u32.to_le_bytes());
    memory.word(task, SHIP_BASE + ctask_force::VTABLE_RVA.0);
    memory.put(task + cunit::HANDLE, &census.task_force.to_le_bytes());
    memory.put(task + cunit::ROSTER_OWNER, &1_u32.to_le_bytes());
    memory.word(task + 0x640, doctrine);
    memory.word(doctrine, doctrine);
    memory.put(doctrine + 0x19, &[1]);
    memory.word(variant, SHIP_BASE + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0);
    memory.word(
        variant + cequipment_variant::TYPE,
        (1_usize << 32) | cequipment_variant::REFERENCE_TYPE as usize,
    );
    memory.put(
        variant + cequipment_variant::EQUIPMENT_TOKEN,
        &11_u32.to_le_bytes(),
    );
    memory.word(variant + cequipment_variant::DEFINITION, definition);
    memory.word(variant + cequipment_variant::CLASSIFIER_FLAGS, 1 << 6);
    memory.word(definition, SHIP_BASE + cequipment_type::VTABLE_RVA.0);
    memory.put(definition + cequipment_type::TOKEN, &11_u32.to_le_bytes());
    memory.put(
        definition + cequipment_type::ARCHETYPE_ID,
        &1_i32.to_le_bytes(),
    );
    memory.word(
        definition + cequipment_type::HULL_SPRITE + msvc_string::CAPACITY,
        msvc_string::INLINE_CAPACITY as usize,
    );
    memory.word(database, SHIP_BASE + cequipment_database::VTABLE_RVA.0);
    memory.array(database + 0x68, 0x15100, 2);
    memory.word(0x15108, definition);
    memory.word(ship, SHIP_BASE + cship::VTABLE_RVA.0);
    memory.put(ship + cship::HANDLE, &census.id.to_le_bytes());
    memory.word(ship + cship::TASK_FORCE, task);
    memory.word(
        ship + cship::EQUIPMENT_POOL,
        SHIP_BASE + cequipment_variant_pool::VTABLE_RVA.0,
    );
    memory.array(
        ship + cship::EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        ship + 0x980,
        1,
    );
    memory.word(ship + 0x980, variant);
    memory.word(ship + 0x988, 5);
    memory.word(
        ship + cship::SUBUNIT_DEFINITION,
        SHIP_BASE + csub_unit_definition::VTABLE_RVA.0,
    );
    memory.put(
        ship + cship::CLASS_DATABASE_ID,
        &census.class_id.to_le_bytes(),
    );
    memory.put(ship + cship::DEFINITION_VALID, &[1]);
    memory.word(
        ship + cship::FALLBACK_SPRITE + msvc_string::CAPACITY,
        msvc_string::INLINE_CAPACITY as usize,
    );
    memory.word(ship + cship::MAXIMUM_STRENGTH, 5_000_000);
    memory.word(ship + 0x8f8, experience);
    let mut capture = Captured::vacant();
    for (hour, value) in [(10, 13_i32), (11, 29)] {
        world
            .0
            .put(ship + cship::MODEL_NUMBER, &value.to_le_bytes());
        world.0.put(
            ship + cship::BASE_ORGANISATION,
            &(i64::from(value) * 1_000_000).to_le_bytes(),
        );
        let reads = Reads {
            memory: &world,
            addresses: RefCell::default(),
        };
        capture.begin_sources();
        capture.visit_source(ship, &census);
        capture.finish_sources(Outcome::Rows { len: 1 });
        capture.capture_owned(&reads, CaptureContext::new(SHIP_BASE));
        assert_eq!(capture.outcomes(), [Outcome::Rows { len: 1 }]);
        assert!(
            reads
                .addresses
                .borrow()
                .iter()
                .any(|at| (ship..ship + 0x900).contains(at))
        );
        assert_eq!(
            collector_ship_models::ship_model_changes::Observer.sequence(ship),
            None
        );
        let mut frame = source(hour);
        frame.tables[TableId::ShipModels.index()] =
            capture.table_inputs(None).into_iter().next().unwrap();
        let expected = frame.tables[TableId::ShipModels.index()].clone();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[TableId::ShipModels.index()], expected);
        let TableRows::ShipModels(rows) = complete(&frame.tables[TableId::ShipModels.index()])
        else {
            panic!("wrong ship table")
        };
        assert_eq!(rows[0].model_number, value);
    }
    world.0.word(ship, 0);
    capture.begin_sources();
    capture.visit_source(ship, &census);
    capture.finish_sources(Outcome::Rows { len: 1 });
    capture.capture_owned(&world, CaptureContext::new(SHIP_BASE));
    let mut frame = source(12);
    frame.tables[TableId::ShipModels.index()] =
        capture.table_inputs(None).into_iter().next().unwrap();
    let refused = frame.tables[TableId::ShipModels.index()].clone();
    assert!(!matches!(
        refused,
        TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_)
    ));
    apply(&mut frame).unwrap();
    assert_eq!(frame.tables[TableId::ShipModels.index()], refused);
}

fn complete(input: &TableInput) -> &TableRows {
    match input {
        TableInput::Sampled(TableView::Complete(rows)) => rows,
        TableInput::SharedComplete(rows) => rows,
        other => panic!("expected fresh complete rows, got {other:?}"),
    }
}

#[test]
fn failed_design_observer_reacquires_changed_native_snapshots_and_publishes_them() {
    use collector_production::production::{self, Captured, Sources};
    use engine_equipment::{cequipment_upgrades_instance, equipment_module_container};
    use engine_native_parts::{cequipment_module, cequipment_type, cequipment_upgrade};
    use engine_native::cequipment_variant as variant;
    use engine_native_memory::msvc_string;
    use engine_native_politics::equipment_upgrade_entry;
    if !contract_tests::isolated(
        "hook_failures::fresh_producer_tests::failed_design_observer_reacquires_changed_native_snapshots_and_publishes_them",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_production::production::design_changes::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    assert!(failed(Family::Designs));
    assert!(!collector_production::production::design_changes::enabled());
    let mut memory = Native(vec![0; 0x9000]);
    let (object, definition, module, upgrade, bonus) = (0x1000, 0x2000, 0x3000, 0x4000, 0x5000);
    memory.word(object, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0);
    memory.word(
        object + variant::TYPE,
        (7_usize << 32) | variant::REFERENCE_TYPE as usize,
    );
    memory.put(object + variant::EQUIPMENT_TOKEN, &71_u32.to_le_bytes());
    memory.word(object + variant::DEFINITION, definition);
    memory.word(definition, cequipment_type::VTABLE_RVA.0);
    memory.put(definition + cequipment_type::TOKEN, &71_u32.to_le_bytes());
    memory.word(definition + cequipment_type::ARCHETYPE, definition);
    for field in [
        variant::DISPLAY_NAME,
        variant::VARIANT_NAME,
        variant::EXPLICIT_ENTITY,
        variant::ICON_OVERRIDE_TOKEN,
    ] {
        memory.word(
            object + field + msvc_string::CAPACITY,
            msvc_string::INLINE_CAPACITY as usize,
        );
    }
    memory.put(object + variant::ICON_OVERRIDE_KIND, &1_u32.to_le_bytes());
    memory.word(
        object + variant::UPGRADES,
        cequipment_upgrades_instance::VTABLE_RVA.0,
    );
    memory.array(
        object + variant::UPGRADES + cequipment_upgrades_instance::ENTRIES,
        0x6100,
        1,
    );
    memory.word(0x6100 + equipment_upgrade_entry::DEFINITION, upgrade);
    memory.word(upgrade, cequipment_upgrade::VTABLE_RVA.0);
    memory.put(upgrade + 8, &73_i32.to_le_bytes());
    memory.array(
        object + variant::MODULES + equipment_module_container::ENTRIES_CANDIDATE,
        0x6000,
        1,
    );
    memory.word(0x6008, module);
    memory.word(module, cequipment_module::VTABLE_RVA.0);
    memory.put(module + 8, &72_u32.to_le_bytes());
    memory.put(module + cequipment_module::ACTIVE, &[1]);
    memory.array(object + variant::BONUS_SOURCES, 0x6200, 1);
    memory.word(0x6200, bonus);
    memory.word(bonus, variant::SPECIFIC_BONUS_VTABLE_RVA.0);
    memory.array(bonus + variant::BONUS_STATS, 0x6300, 1);
    memory.put(0x6300, &1_u32.to_le_bytes());
    let registry = [variant::RegistryRow::read(&memory, 0, object).unwrap()];
    let mut capture = Captured::vacant();
    let snapshots = [
        TableId::VariantDefinitions,
        TableId::VariantModules,
        TableId::VariantStats,
        TableId::VariantUpgrades,
        TableId::VariantBonusSnapshots,
    ];
    let mut previous = None;
    for (hour, value, full) in [(22, 13_i32, false), (46, 29, false), (47, 31, true)] {
        memory.put(object + variant::MANPOWER, &value.to_le_bytes());
        memory.put(0x6000, &(value as u32).to_le_bytes());
        memory.put(
            object + variant::MAXIMUM_SPEED_STAT,
            &i64::from(value).to_le_bytes(),
        );
        memory.put(
            object + variant::ORGANISATION_TRAIT_COUNT,
            &value.to_le_bytes(),
        );
        memory.put(
            0x6100 + equipment_upgrade_entry::LEVEL_STORAGE,
            &[value as u8],
        );
        memory.put(0x6308, &i64::from(value).to_le_bytes());
        let reads = Reads {
            memory: &memory,
            addresses: RefCell::default(),
        };
        let mut variants = capture.take_variants();
        variants.capture(
            &reads,
            0,
            Sources {
                rows: &registry,
                outcome: Outcome::Rows { len: 1 },
            },
            hour,
            full,
        );
        capture.restore_variants(variants);
        assert!(
            reads
                .addresses
                .borrow()
                .iter()
                .any(|at| (bonus..bonus + variant::BONUS_IDEA).contains(at))
        );
        let mut frame = source(hour);
        for (table, input) in production::TABLES
            .into_iter()
            .zip(capture.table_inputs(None))
        {
            frame.tables[table.index()] = input;
        }
        let fresh = snapshots.map(|table| frame.tables[table.index()].clone());
        for input in &fresh {
            assert!(!complete(input).is_empty());
        }
        if let Some(previous) = previous {
            for (before, after) in std::iter::zip(previous, &fresh) {
                assert_ne!(&before, after, "every native snapshot must change");
            }
        }
        apply(&mut frame).unwrap();
        for (table, expected) in snapshots.into_iter().zip(&fresh) {
            assert_eq!(&frame.tables[table.index()], expected);
        }
        let TableRows::VariantDefinitions(definitions) =
            complete(&frame.tables[TableId::VariantDefinitions.index()])
        else {
            panic!("wrong definition table")
        };
        let Recorded::Value(design) = &definitions[0].design else {
            panic!("native design state missing")
        };
        assert_eq!(design.manpower, value);
        let TableRows::VariantStats(stats) = complete(&frame.tables[TableId::VariantStats.index()])
        else {
            panic!("wrong stat table")
        };
        assert_eq!(stats[0].values, [None; 10]);
        assert_eq!(stats[0].organisation_trait_count, value);
        let TableRows::VariantModules(modules) =
            complete(&frame.tables[TableId::VariantModules.index()])
        else {
            panic!("wrong module table")
        };
        assert_eq!(modules[0].slot, value as u32);
        let TableRows::VariantUpgrades(upgrades) =
            complete(&frame.tables[TableId::VariantUpgrades.index()])
        else {
            panic!("wrong upgrade table")
        };
        assert_eq!(upgrades[0].level, value as u8);
        let TableRows::VariantBonusSnapshots(bonuses) =
            complete(&frame.tables[TableId::VariantBonusSnapshots.index()])
        else {
            panic!("wrong bonus table")
        };
        assert!(
            bonuses
                .iter()
                .any(|bonus| bonus.source.stats[0].value == i64::from(value))
        );
        for table in contract_tests::expected_tables(Family::Designs) {
            assert!(matches!(
                &frame.tables[table.index()],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
        previous = Some(fresh);
    }
    memory.word(object + variant::DEFINITION, 0);
    let mut variants = capture.take_variants();
    variants.capture(
        &memory,
        0,
        Sources {
            rows: &registry,
            outcome: Outcome::Rows { len: 1 },
        },
        70,
        false,
    );
    capture.restore_variants(variants);
    let mut frame = source(70);
    for (table, input) in production::TABLES
        .into_iter()
        .zip(capture.table_inputs(None))
    {
        frame.tables[table.index()] = input;
    }
    let refused = frame.tables[TableId::VariantDefinitions.index()].clone();
    assert!(!matches!(
        refused,
        TableInput::Sampled(TableView::Complete(_)) | TableInput::SharedComplete(_)
    ));
    apply(&mut frame).unwrap();
    assert_eq!(frame.tables[TableId::VariantDefinitions.index()], refused);
}

#[test]
fn failed_army_observer_rejects_carry_and_reads_changed_native_operands() {
    use collector_division_parts::attack_stats::{ArmyStatStore, Captured};
    use engine_native::carmy::DivisionSeed;
    use engine_native_parts::csub_unit_definition as stats;
    if !contract_tests::isolated(
        "hook_failures::fresh_producer_tests::failed_army_observer_rejects_carry_and_reads_changed_native_operands",
    ) {
        return;
    }
    contract_tests::reset_session();
    FamilyHooks::<collector_division_parts::army_stat_changes::Hooks>::construct(0, || {
        Err("synthetic relay allocation failure")
    })
    .unwrap();
    let (army, block) = (0x1000, 0x3000);
    let seeds = [DivisionSeed {
        pointer: army,
        division: (7_u64 << 32) | 1,
        stat_block: Some(block),
        ..Default::default()
    }];
    let mut memory = Native(vec![0; 0x8000]);
    memory.word(block, stats::VTABLE_RVA.0);
    let mut store = ArmyStatStore::new();
    let mut capture = Captured::default();
    for (hour, value) in [(10, 13_i64), (11, 29)] {
        memory.put(block + stats::ATTACK_NIGHT, &value.to_le_bytes());
        let reads = Reads {
            memory: &memory,
            addresses: RefCell::default(),
        };
        capture.certify(
            &reads,
            0,
            &seeds,
            Outcome::Rows { len: 1 },
            Some(&mut store),
            &collector_division_parts::army_stat_changes::Observer,
        );
        assert!(!capture.carryable);
        assert_eq!(capture.epoch(), None);
        assert_eq!(capture.held_maxima, [None]);
        capture.capture(
            &reads,
            0,
            hour,
            &seeds,
            Outcome::Rows { len: 1 },
            Some(&mut store),
            |_| None,
        );
        assert!(
            reads
                .addresses
                .borrow()
                .contains(&(block + stats::ATTACK_NIGHT))
        );
        let fresh = capture
            .stats(seeds[0].division)
            .expect("native attack operands");
        assert_eq!(fresh.night, value);
        let mut frame = source(hour);
        frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: seeds[0].division,
                attack_stats: Recorded::Value(Some(fresh)),
                movement_stats: Recorded::Value(capture.movement(seeds[0].division)),
                ..Default::default()
            }]))
            .into();
        let expected = frame.tables[TableId::DivisionStatus.index()].clone();
        apply(&mut frame).unwrap();
        assert_eq!(frame.tables[TableId::DivisionStatus.index()], expected);
    }
}
