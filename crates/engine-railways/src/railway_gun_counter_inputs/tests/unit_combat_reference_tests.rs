use super::*;

const LAND_REFERENCE: u64 = 0xffff_fffe_0000_1b59;
const BORDER_REFERENCE: u64 = 0x8000_0001_0000_1b5a;
const NAVAL_REFERENCE: u64 = 0x7fff_ffff_0000_1b5b;
const NAVAL: usize = 0x9700;

fn full_reference_fixture() -> EtaFixture {
    let mut bytes = standing();
    array(&mut bytes, GUN + cunit::COMBAT_REFERENCE_DATA, COMBATS, 4);
    for (index, pointer) in [COMBAT, COMBAT_TWO, NAVAL, COMBAT].into_iter().enumerate() {
        put64(&mut bytes, COMBATS + index * 8, pointer as u64);
    }
    for (pointer, primary, secondary, handle) in [
        (
            COMBAT,
            cland_combat::VTABLE_RVA.0,
            cland_combat::BODY_VTABLE_RVA.0,
            LAND_REFERENCE,
        ),
        (
            COMBAT_TWO,
            engine_native_parts::cland_border_war_combat::VTABLE_RVA.0,
            engine_native_parts::cland_border_war_combat::BODY_VTABLE_RVA.0,
            BORDER_REFERENCE,
        ),
        (
            NAVAL,
            engine_native::cnaval_combat::VTABLE_RVA.0,
            0x02a5_6e70,
            NAVAL_REFERENCE,
        ),
    ] {
        put64(&mut bytes, pointer, primary as u64);
        put64(&mut bytes, pointer + 0x10, secondary as u64);
        put64(&mut bytes, pointer + 0x18, handle);
    }
    bytes[BUCKETS..BUCKETS + 14 * reference_bucket::STRIDE].fill(0);
    registry(
        &mut bytes,
        TABLE,
        BUCKETS,
        &[
            (GUN_HANDLE, GUN + 0x10),
            (LAND_REFERENCE, COMBAT + 0x10),
            (BORDER_REFERENCE, COMBAT_TWO + 0x10),
            (NAVAL_REFERENCE, NAVAL + 0x10),
        ],
    );
    EtaFixture {
        memory: MemoryFixture { bytes, fault: None },
        reads: Default::default(),
    }
}

fn reference_capture(
    image: &EtaFixture,
    hour: Option<i32>,
    enabled: bool,
    entries: &mut Vec<cunit::MapCombatReference>,
) -> (Walk, Row, usize, usize) {
    let mut rows = vec![Row::default(); 1];
    let mut paths = vec![0; 3];
    let mut roots = vec![(0, 0); 1];
    let mut observations = vec![FreshEtaObservations::default(); 1];
    let mut names = NameArena {
        bytes: engine_native_memory::walk::Buffer::Count,
        used: 0,
        required: 0,
        failures: 0,
    };
    let mut layout = None;
    let mut scratch = TooltipScratch {
        guns: engine_native_memory::walk::Buffer::Growing(&mut roots),
        fresh_observations: engine_native_memory::walk::Buffer::Growing(&mut observations),
        sample_hour: hour,
        named: crate::railway_gun_speed::NamedBonuses {
            pointers: engine_native_memory::walk::Buffer::Count,
            required: 0,
        },
        layout: &mut layout,
        layout_buffer: &mut [],
        supply_origins: engine_native_memory::walk::Buffer::Count,
        supply_origins_len: 0,
        required_supply_origins: 0,
    };
    let mut arena = CombatReferencesArena {
        entries: Buffer::Growing(entries),
        used: 0,
        required: 0,
    };
    let walk = if enabled {
        walk_with_tooltip_and_combat_references(
            image,
            0,
            STATE,
            &mut rows,
            &mut paths,
            &mut cunit::FirstCombatMemo::new(1),
            &mut names,
            &mut scratch,
            &mut arena,
        )
    } else {
        walk_impl(image, 0, STATE, engine_native_memory::walk::Buffer::Growing(&mut rows), engine_native_memory::walk::Buffer::Growing(&mut paths), &mut cunit::FirstCombatMemo::new(1), Some(&mut names), Some(&mut scratch), None)
    };
    (walk, rows[0], arena.used, arena.required)
}

#[test]
fn railway_full_references_share_motion_prefix_without_later_role_demands() {
    let image = full_reference_fixture();
    let (expected_walk, expected_row, _, _) = reference_capture(&image, Some(15), false, &mut Vec::new());
    image.reads.borrow_mut().clear();
    let mut entries = Vec::new();
    let (walk, mut row, used, required) = reference_capture(&image, Some(15), true, &mut entries);
    assert_eq!(walk, expected_walk);
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (
            row.combat_references,
            row.combat_references_failure,
            used,
            required
        ),
        (Some((0, 4)), None, 4, 4)
    );
    assert_eq!(
        entries.iter().map(|entry| (entry.combat, entry.kind)).collect::<Vec<_>>(),
        [
            (LAND_REFERENCE, cunit::COMBAT_LAND),
            (BORDER_REFERENCE, cunit::COMBAT_BORDER_WAR),
            (NAVAL_REFERENCE, cunit::COMBAT_NAVAL),
            (LAND_REFERENCE, cunit::COMBAT_LAND),
        ]
    );
    row.combat_references = None;
    assert_eq!(row, expected_row);
    let reads = image.reads.borrow();
    for index in 0..4 {
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, len)| address == COMBATS + index * 8 && len == 8)
                .count(),
            1
        );
    }
    for object in [COMBAT_TWO, NAVAL] {
        assert!(
            !reads
                .iter()
                .any(|&(address, _)| address == object + cland_combat::ATTACKER
                    || address == object + cland_combat::PROVINCE)
        );
    }
}

#[test]
fn railway_tail_fault_preserves_every_existing_field() {
    for fault in [Some(COMBATS + 8), Some(COMBAT_TWO)] {
        let mut image = full_reference_fixture();
        image.memory.fault = fault;
        let (expected_walk, expected_row, _, _) =
            reference_capture(&image, Some(15), false, &mut Vec::new());
        let mut entries = Vec::new();
        let (walk, mut row, used, required) =
            reference_capture(&image, Some(15), true, &mut entries);
        assert_eq!(walk, expected_walk);
        assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
        assert_eq!((row.combat_references, used, required), (None, 0, 4));
        assert!(row.combat_references_failure.take().is_some());
        assert_eq!(row, expected_row);
    }
}

#[test]
fn railway_missing_hour_and_authenticated_empty_are_distinct() {
    let image = EtaFixture {
        memory: MemoryFixture {
            bytes: standing(),
            fault: None,
        },
        reads: Default::default(),
    };
    let (_, missing, used, required) = reference_capture(&image, None, true, &mut Vec::new());
    assert_eq!(
        (
            missing.combat_references,
            missing.combat_references_failure,
            used,
            required
        ),
        (None, None, 0, 0)
    );
    let (walk, empty, used, required) = reference_capture(&image, Some(i32::MIN), true, &mut Vec::new());
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(
        (
            empty.combat_references,
            empty.combat_references_failure,
            used,
            required
        ),
        (Some((0, 0)), None, 0, 0)
    );
}

#[test]
fn railway_header_hint_never_walks_the_vector_and_retains_independent_demand() {
    let mut image = full_reference_fixture();
    image.memory.fault = Some(COMBATS);
    let demand = name_demand(&image, 0, STATE).unwrap();
    assert_eq!((demand.rows, demand.combat_references), (1, 4));
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| (COMBATS..COMBATS + 32).contains(&address))
    );
    image.memory.fault = Some(GUN + cunit::COMBAT_REFERENCE_DATA);
    let refused_hint = name_demand(&image, 0, STATE).unwrap();
    assert_eq!((refused_hint.rows, refused_hint.combat_references), (1, 0));
    assert_eq!(
        (
            refused_hint.bytes,
            refused_hint.named_bonuses,
            refused_hint.supply_origins
        ),
        (demand.bytes, demand.named_bonuses, demand.supply_origins)
    );
}
