use super::*;
use engine_native_parts::{creference_registry as registry, reference_bucket};

const COMBAT_REFERENCE: u64 = 0xffff_fffe_0000_1b59;
const TABLE: usize = 0x1b000;
const BUCKETS: usize = 0x1b100;

fn reference_fixture() -> (PresentationMemory, [super::super::DivisionSeed; 3]) {
    let (mut source, sources) = movement_combat_fixture();
    p64(
        &mut source.bytes,
        0x16000 + 0x10,
        crate::cland_combat::BODY_VTABLE_RVA.0 as u64,
    );
    p64(&mut source.bytes, 0x16000 + 0x18, COMBAT_REFERENCE);
    source.globals.insert(
        registry::HIGH_ROOT_RVA.0,
        (TABLE as u64).to_le_bytes().to_vec(),
    );
    p64(&mut source.bytes, TABLE + registry::ENTRIES, BUCKETS as u64);
    p32(&mut source.bytes, TABLE + registry::COUNT, 1);
    p32(&mut source.bytes, TABLE + registry::MASK, 7);
    source.bytes[TABLE + registry::OVERFLOW_DISTANCE] = 5;
    source.bytes[BUCKETS + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
    let hash = COMBAT_REFERENCE
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let cell =
        BUCKETS + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE;
    source.bytes[cell + reference_bucket::DISTANCE] = 1;
    p64(
        &mut source.bytes,
        cell + reference_bucket::TYPE,
        COMBAT_REFERENCE,
    );
    p64(&mut source.bytes, cell + reference_bucket::OBJECT, 0x16010);
    for (index, army) in [A1, A2, A3].into_iter().enumerate() {
        let vector = 0x19000 + index * 0x10;
        array(
            &mut source.bytes,
            army + crate::cunit::COMBAT_REFERENCE_DATA,
            vector,
            2,
        );
        p64(&mut source.bytes, vector + 8, 0x16000);
    }
    let mut seeds = sources.map(|source| super::super::DivisionSeed {
        pointer: source.pointer,
        division: source.division,
        ..Default::default()
    });
    super::super::capture_division_seeds(&source, &mut seeds);
    (source, seeds)
}

fn run(
    memory: &impl engine_native_memory::Memory,
    seeds: &[super::super::DivisionSeed],
    packet: &mut super::super::MovementPacket,
    hour: Option<i32>,
) -> (super::super::MovementWalk, [super::super::MovementRow; 3]) {
    let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
        crate::cprovince::GeometryCapacities::default(),
    );
    geometry.begin(memory, 0, GS).unwrap();
    let result = super::super::capture_movement_inputs_with_combat_references(
        memory,
        0,
        GS,
        seeds,
        Outcome::Rows { len: seeds.len() },
        &mut geometry,
        packet,
        hour,
    );
    let mut output = [super::super::MovementRow::default(); 3];
    assert_eq!(
        super::super::project_movement(packet, &mut output, &mut []),
        result
    );
    (result, output)
}

#[test]
fn movement_reference_growth_preserves_prefixes_order_repeats_and_read_once() {
    let (source, seeds) = reference_fixture();
    let memory = ScalarBatchMemory::new(&source);
    let mut packet = super::super::MovementPacket::new(3, 0);
    packet.reserve_combat_references(4).unwrap();
    let (legacy, expected) = run(&memory, &seeds, &mut packet, None);
    memory.reads.borrow_mut().clear();
    let allocated = packet.allocated_bytes();
    let (walk, rows) = run(&memory, &seeds, &mut packet, Some(7));
    assert_eq!(walk, legacy);
    assert_eq!(walk.outcome, Outcome::Rows { len: 3 });
    assert!(
        packet.allocated_bytes() > allocated,
        "a reference arena short of the hour's demand grows"
    );
    assert_eq!(
        (
            packet.combat_references_required(),
            packet.combat_references_hour()
        ),
        (6, Some(7))
    );
    assert_eq!(
        rows.map(|row| row.combat_references),
        [Some((0, 2)), Some((2, 2)), Some((4, 2))]
    );
    assert!(rows.iter().all(|row| row.combat_references_failure.is_none()));
    for (mut actual, expected) in rows.into_iter().zip(expected) {
        actual.combat_references = None;
        actual.combat_references_failure = None;
        assert_eq!(actual, expected);
    }
    let reads = memory.reads.borrow();
    assert_eq!(
        reads
            .iter()
            .filter(|&&(at, len)| at == 0x16000 && len == 32)
            .count(),
        1,
        "all admitted divisions share this pass's successful combat identity"
    );
    for index in 0..3 {
        let vector = 0x19000 + index * 0x10;
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, len)| address == vector && len == 8)
                .count(),
            1
        );
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, len)| address == vector + 8 && len == 8)
                .count(),
            1
        );
    }
    drop(reads);
    packet
        .reserve_combat_references(packet.combat_references_required())
        .unwrap();
    let (_, grown) = run(&memory, &seeds, &mut packet, Some(8));
    for row in grown {
        assert_eq!(
            packet.combat_references(&row).unwrap(),
            [crate::cunit::MapCombatReference {
                combat: COMBAT_REFERENCE,
                kind: crate::cunit::COMBAT_LAND
            }; 2]
        );
        assert_eq!(row.combat_references_failure, None);
    }
    assert_eq!(packet.combat_references_hour(), Some(8));
    packet.clear();
    assert_eq!(
        (
            packet.combat_references_hour(),
            packet.combat_references_required()
        ),
        (None, 0)
    );
    assert!(
        packet.combat_references(&grown[0]).is_none(),
        "reset invalidates old spans"
    );
}

#[test]
fn movement_optional_registry_failure_preserves_every_established_field() {
    let (source, seeds) = reference_fixture();
    let memory = ScalarBatchMemory::new(&source);
    let mut packet = super::super::MovementPacket::new(3, 0);
    packet.reserve_combat_references(6).unwrap();
    let (legacy, expected) = run(&memory, &seeds, &mut packet, None);
    memory.fault.set(Some(registry::HIGH_ROOT_RVA.0));
    let (walk, rows) = run(&memory, &seeds, &mut packet, Some(9));
    assert_eq!(walk, legacy);
    for (mut actual, expected) in rows.into_iter().zip(expected) {
        assert_eq!(actual.combat_references, None);
        assert!(actual.combat_references_failure.take().is_some());
        assert_eq!(actual, expected);
    }
    assert_eq!(packet.combat_references_required(), 6);
}

#[test]
fn movement_empty_vector_has_own_hour_and_legacy_does_not_acquire_references() {
    let (mut source, _) = reference_fixture();
    let mut seeds = [super::super::DivisionSeed {
        pointer: A1,
        division: handle(9, 9000),
        ..Default::default()
    }];
    array(
        &mut source.bytes,
        A1 + crate::cunit::COMBAT_REFERENCE_DATA,
        0,
        0,
    );
    super::super::capture_division_seeds(&source, &mut seeds);
    let memory = ScalarBatchMemory::new(&source);
    let mut packet = super::super::MovementPacket::new(1, 0);
    let (walk, rows) = run(&memory, &seeds, &mut packet, Some(i32::MIN));
    assert_eq!(walk.outcome, Outcome::Rows { len: 1 });
    assert_eq!(rows[0].combat_references, Some((0, 0)));
    assert_eq!(packet.combat_references(&rows[0]), Some(&[][..]));
    assert_eq!(packet.combat_references_hour(), Some(i32::MIN));
    memory.reads.borrow_mut().clear();
    let (_, legacy) = run(&memory, &seeds, &mut packet, None);
    assert_eq!(legacy[0].combat_references, None);
    assert!(
        !memory.reads.borrow().iter().any(|&(address, len)| address
            == A1 + crate::cunit::COMBAT_REFERENCE_DATA
            && len == 16)
    );
}
