use super::*;
use engine_governance_tests as kit;
use engine_research::{research_inputs, research_modifiers};
#[expect(dead_code, reason = "these tests use only part of the serial reference copy of drift")]
#[path = "drift_serial.rs"]
mod serial;
use kit::{GAME, SLOT, TECH, Trace, World};

struct Full {
    inputs: Vec<research_inputs::Row>,
    bonuses: Vec<research_inputs::BonusRow>,
    modifiers: Vec<research_modifiers::Row>,
    technologies: Vec<ctechnology::Row>,
    pointers: Vec<usize>,
    slots: Vec<cresearch_slot::Row>,
}
fn full(world: &World, id: u32) -> Full {
    let mut rows = Full {
        inputs: Vec::new(),
        bonuses: Vec::new(),
        modifiers: Vec::new(),
        technologies: Vec::new(),
        pointers: Vec::new(),
        slots: Vec::new(),
    };
    let country = World::country(id);
    let inputs = research_inputs::walk_country(
        world,
        0,
        country,
        id,
        research_inputs::Context::read(world, 0),
        &mut rows.inputs,
        &mut rows.bonuses,
    );
    assert_eq!(inputs, (Outcome::Rows { len: 3 }, Outcome::Rows { len: 2 }));
    let countries = ccountry::Reader::new(world, 0, GAME).unwrap();
    let modifiers = research_modifiers::walk_country(
        world,
        0,
        &countries,
        engine_native_memory::country_identity::Country {
            pointer: country,
            id,
            canonical_slot: id,
        },
        research_modifiers::Context::read(world, 0),
        &rows.inputs,
        &mut rows.modifiers,
    );
    assert!(matches!(modifiers, Outcome::Rows { .. }));
    rows.modifiers.truncate(modifiers.len());
    let mut entries = Vec::new();
    let mut names = Vec::new();
    let mut definitions = ctechnology_template::Dictionary::new(&mut entries, &mut names);
    let technology = ctechnology_status::walk_technologies(
        world,
        0,
        country,
        id,
        (&mut rows.technologies, &mut rows.pointers),
        0,
        &mut definitions,
    );
    assert_eq!(technology, Outcome::Rows { len: 2 });
    assert_eq!(
        ctechnology_status::walk_slots(world, 0, country, id, &mut rows.slots),
        Outcome::Rows { len: 3 }
    );
    rows
}
fn hold(store: &mut drift::Store, serial: &mut serial::Store, world: &World, id: u32, rows: &Full) {
    let country = World::country(id);
    store.hold_research(
        world,
        0,
        id,
        country,
        (&rows.inputs, &rows.bonuses, &rows.modifiers),
    );
    serial.hold_research(
        world,
        0,
        id,
        country,
        (&rows.inputs, &rows.bonuses, &rows.modifiers),
    );
    store.hold_technologies(
        world,
        0,
        id,
        country,
        &rows.technologies,
        &rows.pointers,
        Some(&rows.slots),
    );
    serial.hold_technologies(
        world,
        0,
        id,
        country,
        &rows.technologies,
        &rows.pointers,
        Some(&rows.slots),
    );
}
#[derive(Debug, PartialEq, Eq)]
struct ResearchResult {
    outcomes: [Outcome; 3],
    inputs: Vec<research_inputs::Row>,
    bonuses: Vec<research_inputs::BonusRow>,
    modifiers: Vec<research_modifiers::Row>,
}
fn research(store: &drift::Store, memory: &Trace<'_, World>, id: u32) -> ResearchResult {
    let mut inputs = Vec::new();
    let mut bonuses = Vec::new();
    let mut modifiers = Vec::new();
    let outcome = store.carry_research(
        memory,
        0,
        id,
        World::country(id),
        engine_native_memory::walk::Buffer::Growing(&mut inputs),
        engine_native_memory::walk::Buffer::Growing(&mut bonuses),
        engine_native_memory::walk::Buffer::Growing(&mut modifiers),
    );
    let outcomes = match outcome {
        drift::Carried::Rows(lens) => lens.map(|len| Outcome::Rows { len }),
        drift::Carried::Failed(reason) => [Outcome::Unavailable { reason }; 3],
        drift::Carried::Miss => panic!("expected held country"),
    };
    inputs.truncate(outcomes[0].len());
    bonuses.truncate(outcomes[1].len());
    modifiers.truncate(outcomes[2].len());
    ResearchResult {
        outcomes,
        inputs,
        bonuses,
        modifiers,
    }
}
fn serial_research(store: &serial::Store, memory: &Trace<'_, World>, id: u32) -> ResearchResult {
    let mut inputs = Vec::new();
    let mut bonuses = Vec::new();
    let mut modifiers = Vec::new();
    let outcome = store.carry_research(
        memory,
        0,
        id,
        World::country(id),
        engine_native_memory::walk::Buffer::Growing(&mut inputs),
        engine_native_memory::walk::Buffer::Growing(&mut bonuses),
        engine_native_memory::walk::Buffer::Growing(&mut modifiers),
    );
    let outcomes = match outcome {
        serial::Carried::Rows(lens) => lens.map(|len| Outcome::Rows { len }),
        serial::Carried::Failed(reason) => [Outcome::Unavailable { reason }; 3],
        serial::Carried::Miss => panic!("expected held country"),
    };
    inputs.truncate(outcomes[0].len());
    bonuses.truncate(outcomes[1].len());
    modifiers.truncate(outcomes[2].len());
    ResearchResult {
        outcomes,
        inputs,
        bonuses,
        modifiers,
    }
}
fn technologies(
    store: &drift::Store,
    memory: &Trace<'_, World>,
    id: u32,
) -> (Outcome, Vec<ctechnology::Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut pointers = Vec::new();
    let outcome = match store.carry_technologies(
        memory,
        0,
        id,
        World::country(id),
        &mut rows,
        &mut pointers,
        0,
    ) {
        drift::Carried::Rows(len) => Outcome::Rows { len },
        drift::Carried::Failed(reason) => Outcome::Unavailable { reason },
        drift::Carried::Miss => panic!("expected held country"),
    };
    rows.truncate(outcome.len());
    pointers.truncate(outcome.len());
    (outcome, rows, pointers)
}
fn serial_technologies(
    store: &serial::Store,
    memory: &Trace<'_, World>,
    id: u32,
) -> (Outcome, Vec<ctechnology::Row>, Vec<usize>) {
    let mut rows = Vec::new();
    let mut pointers = Vec::new();
    let outcome = match store.carry_technologies(
        memory,
        0,
        id,
        World::country(id),
        &mut rows,
        &mut pointers,
        0,
    ) {
        serial::Carried::Rows(len) => Outcome::Rows { len },
        serial::Carried::Failed(reason) => Outcome::Unavailable { reason },
        serial::Carried::Miss => panic!("expected held country"),
    };
    rows.truncate(outcome.len());
    pointers.truncate(outcome.len());
    (outcome, rows, pointers)
}

#[test]
fn governance_batch_carried_persistent_and_one_shot_every_serial_operand() {
    let world = World::new(1);
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    hold(&mut store, &mut serial, &world, 1, &full(&world, 1));
    let before = Trace::new(&world, "Research drift serial census");
    serial_research(&serial, &before, 1);
    let research_addresses = kit::addresses(&before);
    before.clear();
    serial_technologies(&serial, &before, 1);
    let technology_addresses = kit::addresses(&before);
    for address in research_addresses {
        for once in [false, true] {
            let before = Trace::fault(&world, "Research drift serial", address, once);
            let after = Trace::fault(&world, "Research drift batch", address, once);
            let old = serial_research(&serial, &before, 1);
            let new = research(&store, &after, 1);
            if once {
                for (old, new) in old.outcomes.into_iter().zip(new.outcomes) {
                    kit::never_harsher(old, new);
                }
            } else {
                assert_eq!(new, old, "persistent {address:#x}");
            }
            if !once {
                assert!(matches!(new.outcomes[0], Outcome::Unavailable { .. }));
            }
            assert!(after.fallbacks.witnesses().len() <= 1);
            let is_drift = address >= World::country(1) + SLOT
                && address < World::country(1) + TECH + 0x400 + ctechnology::LIVE_PROGRESS + 8;
            assert_eq!(after.fallbacks.witnesses().len(), usize::from(is_drift));
        }
    }
    for address in technology_addresses {
        for once in [false, true] {
            let before = Trace::fault(&world, "Technology drift serial", address, once);
            let after = Trace::fault(&world, "Technology drift batch", address, once);
            let old = serial_technologies(&serial, &before, 1);
            let new = technologies(&store, &after, 1);
            if once {
                kit::never_harsher(old.0, new.0);
            } else {
                assert_eq!(new, old, "persistent {address:#x}");
            }
            if !once {
                assert!(matches!(new.0, Outcome::Unavailable { .. }));
            }
            assert!(after.fallbacks.witnesses().len() <= 1);
        }
    }
}
#[test]
fn governance_batch_carried_gather_only_fault_publishes_serial_fallback() {
    let mut world = World::new(1);
    let country = World::country(1);
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    hold(&mut store, &mut serial, &world, 1, &full(&world, 1));
    for position in 0..3 {
        world.put(
            country + SLOT + position * 0x80 + cresearch_slot::BANKED_DAYS,
            (200_011_i64 + position as i64).to_le_bytes(),
        );
    }
    for position in 0..2 {
        world.put(
            country + TECH + position * 0x400 + ctechnology::LIVE_PROGRESS,
            (600_013_i64 + position as i64).to_le_bytes(),
        );
    }
    for (address, calls, once_read) in [
        (
            country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS,
            8,
            vec![country + SLOT + cresearch_slot::BANKED_DAYS],
        ),
        (
            country + TECH + 0x400 + ctechnology::LIVE_PROGRESS,
            6,
            vec![
                country + SLOT + cresearch_slot::BANKED_DAYS,
                country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS,
                country + SLOT + 2 * 0x80 + cresearch_slot::BANKED_DAYS,
                country + TECH + ctechnology::LIVE_PROGRESS,
            ],
        ),
    ] {
        let before = Trace::gather_fault(&world, "Research serial", address);
        let after = Trace::gather_fault(&world, "Research batch", address);
        let old = serial_research(&serial, &before, 1);
        assert!(
            old.outcomes
                .iter()
                .all(|o| matches!(o, Outcome::Rows { .. }))
        );
        assert_eq!(research(&store, &after, 1), old);
        assert_eq!(before.remaining.get(), Some(1));
        assert_eq!(after.remaining.get(), Some(0));
        assert_eq!(&*after.fallbacks.witnesses(), &["research_drift"]);
        assert_eq!(after.count(), calls);
        for kept in once_read {
            assert_eq!(
                after
                    .operands
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == kept)
                    .count(),
                1,
                "completed operand {kept:#x} was reread"
            );
        }
        assert_eq!(
            after
                .operands
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == address)
                .count(),
            2
        );
    }
    let address = country + TECH + 0x400 + ctechnology::LIVE_PROGRESS;
    let before = Trace::gather_fault(&world, "Technology serial", address);
    let after = Trace::gather_fault(&world, "Technology batch", address);
    let old = serial_technologies(&serial, &before, 1);
    assert_eq!(old.0, Outcome::Rows { len: 2 });
    assert_eq!(technologies(&store, &after, 1), old);
    assert_eq!(before.remaining.get(), Some(1));
    assert_eq!(after.remaining.get(), Some(0));
    assert_eq!(&*after.fallbacks.witnesses(), &["technology_drift"]);
    assert_eq!(after.count(), 5);
    assert_eq!(
        after
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == country + TECH + ctechnology::LIVE_PROGRESS)
            .count(),
        1
    );
    assert_eq!(
        after
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == address)
            .count(),
        2
    );
}
#[test]
fn governance_batch_carried_persistent_drift_fault_keeps_serial_failure() {
    let world = World::new(1);
    let country = World::country(1);
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    hold(&mut store, &mut serial, &world, 1, &full(&world, 1));
    for (address, once_read) in [
        (
            country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS,
            vec![country + SLOT + cresearch_slot::BANKED_DAYS],
        ),
        (
            country + TECH + 0x400 + ctechnology::LIVE_PROGRESS,
            vec![
                country + SLOT + cresearch_slot::BANKED_DAYS,
                country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS,
                country + SLOT + 2 * 0x80 + cresearch_slot::BANKED_DAYS,
                country + TECH + ctechnology::LIVE_PROGRESS,
            ],
        ),
    ] {
        let before = Trace::fault(&world, "Research serial", address, false);
        let after = Trace::fault(&world, "Research batch", address, false);
        let old = serial_research(&serial, &before, 1);
        assert!(
            old.outcomes
                .iter()
                .all(|o| matches!(o, Outcome::Unavailable { .. }))
        );
        assert_eq!(research(&store, &after, 1), old);
        assert_eq!(&*after.fallbacks.witnesses(), &["research_drift"]);
        for kept in once_read {
            assert_eq!(
                after
                    .operands
                    .borrow()
                    .iter()
                    .filter(|&&(at, _)| at == kept)
                    .count(),
                1,
                "completed operand {kept:#x} was reread"
            );
        }
        assert_eq!(
            after
                .operands
                .borrow()
                .iter()
                .filter(|&&(at, _)| at == address)
                .count(),
            2
        );
    }
    let address = country + TECH + 0x400 + ctechnology::LIVE_PROGRESS;
    let before = Trace::fault(&world, "Technology serial", address, false);
    let after = Trace::fault(&world, "Technology batch", address, false);
    let old = serial_technologies(&serial, &before, 1);
    assert!(matches!(old.0, Outcome::Unavailable { .. }));
    assert_eq!(technologies(&store, &after, 1), old);
    assert_eq!(&*after.fallbacks.witnesses(), &["technology_drift"]);
    assert_eq!(
        after
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == country + TECH + ctechnology::LIVE_PROGRESS)
            .count(),
        1
    );
    assert_eq!(
        after
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == address)
            .count(),
        2
    );
}
#[test]
fn governance_batch_carried_research_aliased_technologies_do_not_reread_completed_prefix() {
    let mut world = World::new(1);
    let country = World::country(1);
    world.pointer(
        country + SLOT + 0x80 + cresearch_slot::TECHNOLOGY,
        country + TECH + 0x400,
    );
    world.put(
        country + SLOT + 0x80 + cresearch_slot::TECHNOLOGY_HANDLE,
        8_u32.to_le_bytes(),
    );
    world.pointer(
        country + SLOT + 2 * 0x80 + cresearch_slot::TECHNOLOGY,
        country + TECH,
    );
    world.put(
        country + SLOT + 2 * 0x80 + cresearch_slot::TECHNOLOGY_HANDLE,
        7_u32.to_le_bytes(),
    );
    world.array(
        country + TECH + 0x400 + ctechnology::APPLIED_BONUS_IDS,
        country + 0x4104,
        0,
    );
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    hold(&mut store, &mut serial, &world, 1, &full(&world, 1));
    let tech_a = country + TECH + ctechnology::LIVE_PROGRESS;
    let tech_b = country + TECH + 0x400 + ctechnology::LIVE_PROGRESS;
    world.put(tech_a, 777_001_i64.to_le_bytes());
    world.put(tech_b, 888_002_i64.to_le_bytes());

    let trace = Trace::fault(&world, "Research alias B one-shot", tech_b, true);
    let result = research(&store, &trace, 1);
    assert!(
        result
            .outcomes
            .iter()
            .all(|o| matches!(o, Outcome::Rows { .. }))
    );
    assert_eq!(result.inputs[0].progress_raw, 777_001);
    assert_eq!(result.inputs[1].progress_raw, 888_002);
    assert_eq!(result.inputs[2].progress_raw, 777_001);
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_a)
            .count(),
        1
    );
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_b)
            .count(),
        2
    );

    let trace = Trace::fault(&world, "Research alias A one-shot", tech_a, true);
    let result = research(&store, &trace, 1);
    assert!(
        result
            .outcomes
            .iter()
            .all(|o| matches!(o, Outcome::Rows { .. }))
    );
    assert_eq!(result.inputs[0].progress_raw, 777_001);
    assert_eq!(result.inputs[1].progress_raw, 888_002);
    assert_eq!(result.inputs[2].progress_raw, 777_001);
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_a)
            .count(),
        2
    );
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_b)
            .count(),
        1
    );

    let banked_b = country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS;
    let trace = Trace::fault(&world, "Research alias banked one-shot", banked_b, true);
    let mut inputs = vec![research_inputs::Row::default(); 3];
    let mut bonuses = vec![research_inputs::BonusRow::default(); 2];
    let mut modifiers = vec![research_modifiers::Row::default(); 64];
    let growths = engine_allocation::growths();
    let outcome = store.carry_research(
        &trace,
        0,
        1,
        country,
        engine_native_memory::walk::Buffer::Growing(&mut inputs),
        engine_native_memory::walk::Buffer::Growing(&mut bonuses),
        engine_native_memory::walk::Buffer::Growing(&mut modifiers),
    );
    assert_eq!(engine_allocation::growths() - growths, 2);
    assert!(matches!(outcome, drift::Carried::Rows([3, 2, _])));
    assert_eq!(inputs[0].progress_raw, 777_001);
    assert_eq!(inputs[1].progress_raw, 888_002);
    assert_eq!(inputs[2].progress_raw, 777_001);
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_a)
            .count(),
        1
    );
    assert_eq!(
        trace
            .operands
            .borrow()
            .iter()
            .filter(|&&(at, _)| at == tech_b)
            .count(),
        1
    );
}
#[test]
fn governance_batch_carried_multiple_persistent_faults_keep_serial_first_reason() {
    let world = World::new(1);
    let country = World::country(1);
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    hold(&mut store, &mut serial, &world, 1, &full(&world, 1));
    let inner = Trace::fault(
        &world,
        "second banked fault",
        country + SLOT + 0x80 + cresearch_slot::BANKED_DAYS,
        false,
    );
    let outer = Trace::fault(
        &inner,
        "first assigned fault",
        country + TECH + ctechnology::LIVE_PROGRESS,
        false,
    );
    let mut inputs = Vec::new();
    let mut bonuses = Vec::new();
    let mut modifiers = Vec::new();
    let outcome = store.carry_research(
        &outer,
        0,
        1,
        country,
        engine_native_memory::walk::Buffer::Growing(&mut inputs),
        engine_native_memory::walk::Buffer::Growing(&mut bonuses),
        engine_native_memory::walk::Buffer::Growing(&mut modifiers),
    );
    assert!(matches!(
        outcome,
        drift::Carried::Failed("assigned technology unreadable")
    ));
    assert_eq!(&*outer.fallbacks.witnesses(), &["research_drift"]);
    let original = serial.carry_research(
        &outer,
        0,
        1,
        country,
        engine_native_memory::walk::Buffer::Growing(&mut inputs),
        engine_native_memory::walk::Buffer::Growing(&mut bonuses),
        engine_native_memory::walk::Buffer::Growing(&mut modifiers),
    );
    assert!(matches!(
        original,
        serial::Carried::Failed("assigned technology unreadable")
    ));
}
#[test]
fn governance_batch_rehold_refills_storage_without_earlier_rows() {
    let world = World::new(1);
    let country = World::country(1);
    let mut store = drift::Store::default();
    let mut serial = serial::Store::default();
    let mut rows = full(&world, 1);
    hold(&mut store, &mut serial, &world, 1, &rows);
    rows.inputs[0].cost_raw += 1;
    rows.bonuses.truncate(1);
    rows.modifiers.truncate(1);
    rows.technologies[0].ahead_reduction_years_raw += 1;
    hold(&mut store, &mut serial, &world, 1, &rows);
    let new = research(&store, &Trace::new(&world, "Research rehold batch"), 1);
    let old = serial_research(&serial, &Trace::new(&world, "Research rehold serial"), 1);
    assert_eq!(new, old);
    assert_eq!(
        (new.bonuses.len(), new.modifiers.len()),
        (1, rows.modifiers.len())
    );
    assert_eq!(new.inputs[0].cost_raw, rows.inputs[0].cost_raw);
    let new = technologies(&store, &Trace::new(&world, "Technology rehold batch"), 1);
    let old = serial_technologies(&serial, &Trace::new(&world, "Technology rehold serial"), 1);
    assert_eq!(new, old);
    assert_eq!(
        new.1[0].ahead_reduction_years_raw,
        rows.technologies[0].ahead_reduction_years_raw
    );
    rows.inputs.pop();
    store.hold_research(
        &world,
        0,
        1,
        country,
        (&rows.inputs, &rows.bonuses, &rows.modifiers),
    );
    let outcome = store.carry_research(
        &Trace::new(&world, "Research dropped hold"),
        0,
        1,
        country,
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
        engine_native_memory::walk::Buffer::Growing(&mut Vec::new()),
    );
    assert!(matches!(outcome, drift::Carried::Miss));
}
