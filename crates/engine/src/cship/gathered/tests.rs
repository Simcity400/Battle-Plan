use super::*;
use crate::{
    memory::{RecordField, SliceMemory},
    read_census::ReadCensus,
};

/// One image holding three ships (A and B in task force A, C in task force
/// B), the province both task forces are at and the game singleton slot.
const IMAGE: usize = 0x1000_0000;
const LEN: usize = 0x6000;
const SHIP_A: usize = IMAGE + 0x1000;
const SHIP_B: usize = IMAGE + 0x2000;
const SHIP_C: usize = IMAGE + 0x3000;
const TASK_A: usize = IMAGE + 0x4000;
const TASK_B: usize = IMAGE + 0x4800;
const PROVINCE: usize = IMAGE + 0x5000;
const SINGLETON: usize = IMAGE + 0x5800;
const MODULE: usize = SINGLETON - cgame_state::SINGLETON_RVA;

fn image() -> Vec<u8> {
    let mut bytes: Vec<u8> = (0..LEN).map(|index| (index * 7 + 3) as u8).collect();
    for (at, value) in [
        (SHIP_A + TASK_FORCE, TASK_A),
        (SHIP_B + TASK_FORCE, TASK_A),
        (SHIP_C + TASK_FORCE, TASK_B),
        (TASK_A + cunit::LOCATION, PROVINCE),
        (TASK_B + cunit::LOCATION, PROVINCE),
    ] {
        bytes[at - IMAGE..at - IMAGE + 8].copy_from_slice(&(value as u64).to_le_bytes());
    }
    bytes
}

fn at(bytes: &[u8], address: usize, len: usize) -> &[u8] {
    &bytes[address - IMAGE..address - IMAGE + len]
}

#[test]
fn every_object_lists_ascending_disjoint_fields_no_wider_than_its_storage() {
    fn check(fields: &[(usize, usize)]) {
        for pair in fields.windows(2) {
            assert!(pair[0].0 + pair[0].1 <= pair[1].0, "{pair:x?}");
        }
        assert!(
            fields
                .iter()
                .all(|&(_, width)| (1..=WIDTH).contains(&width))
        );
    }
    check(&SHIP);
    check(&TASK);
    check(&LOCATION);
    check(&GLOBALS);
    check(&VARIANT);
    assert_eq!(
        SHIP[SHIP_POOL],
        (EQUIPMENT_POOL, 8),
        "the pool's vtable field"
    );
    assert_eq!(
        SHIP[SHIP_POOL_ITEMS],
        (EQUIPMENT_POOL + cequipment_variant_pool::ITEMS, 16),
        "the pool's array header field"
    );
    assert_eq!(SHIP[SHIP_TASK_FORCE].0, TASK_FORCE);
    assert_eq!(TASK[TASK_LOCATION].0, cunit::LOCATION);
    // Every field lies inside its object.
    assert!(
        SHIP.iter()
            .all(|&(start, width)| start + width <= crate::cship::SIZE)
    );
}

#[test]
fn gathered_fields_serve_every_read_inside_them_without_the_source() {
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&slice);
    let mut fields = ShipFields::new(MODULE);
    fields.visit(&census, SHIP_A);
    // The ship, the game singleton, the task force and its province.
    assert_eq!(census.calls(), 4);
    census.reset();
    let memory = fields.memory(&census);
    for (address, len) in [
        (SHIP_A, 16),
        (SHIP_A, 8),
        (SHIP_A + 8, 8),
        (SHIP_A + SUBUNIT_DEFINITION, 12),
        (SHIP_A + crate::cship::CLASS_DATABASE_ID, 4),
        (SHIP_A + FALLBACK_SPRITE, msvc_string::STRIDE),
        (SHIP_A + BASE_ORGANISATION, 8),
        (SHIP_A + crate::cship::MAXIMUM_STRENGTH, 8),
        (SHIP_A + TASK_FORCE, 8),
        (SHIP_A + crate::cship::CARRIER_AIR_BASE, 8),
        (SHIP_A + MODIFIERS, 16),
        (SHIP_A + CRITICALS, 16),
        (TASK_A, 8),
        (TASK_A + cunit::HANDLE, 8),
        (TASK_A + cunit::ROSTER_OWNER, 4),
        (TASK_A + ctask_force::FLEET, 8),
        (PROVINCE, 8),
        (PROVINCE + PROVINCE_DEFINITION, 8),
        (SINGLETON, 8),
    ] {
        let mut served = vec![0; len];
        assert!(memory.copy(address, &mut served), "{address:#x}");
        assert_eq!(served, at(&bytes, address, len), "{address:#x}");
    }
    assert_eq!(
        memory.read_scalar::<u32>(SHIP_A + MODEL_NUMBER),
        Some(u32::from_le_bytes(
            at(&bytes, SHIP_A + MODEL_NUMBER, 4).try_into().unwrap()
        ))
    );
    assert_eq!(census.calls(), 0, "served reads never reach the source");
    // A read outside every field, or across two, goes to the source.
    let mut outside = [0; 8];
    assert!(memory.copy(SHIP_A + 0x20, &mut outside));
    assert_eq!(outside, at(&bytes, SHIP_A + 0x20, 8));
    let mut across = [0; 16];
    assert!(memory.copy(SHIP_A + TASK_FORCE - 8, &mut across));
    assert_eq!(across, at(&bytes, SHIP_A + TASK_FORCE - 8, 16));
    assert_eq!(census.calls(), 2);
}

#[test]
fn a_failed_field_fails_again_without_a_read_and_later_fields_read_on_first_use() {
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    // The fault lies in the formation flags' second half.
    let source = FaultAt::new(&slice, Some(SHIP_A + FORMATION_FLAGS + 6));
    let mut fields = ShipFields::new(MODULE);
    fields.visit(&source, SHIP_A);
    let reads = || source.reads.borrow().len();
    let gathered = reads();
    let memory = fields.memory(&source);
    let mut flags = [0; 8];
    assert!(!memory.copy(SHIP_A + FORMATION_FLAGS, &mut flags));
    assert_eq!(reads(), gathered, "the failed field is not read again");
    // Part of the failed field is another observation, from the source.
    let mut low = [0; 4];
    assert!(memory.copy(SHIP_A + FORMATION_FLAGS, &mut low));
    assert_eq!(low, at(&bytes, SHIP_A + FORMATION_FLAGS, 4));
    assert_eq!(reads(), gathered + 1);
    // A field before the failure is served; one after it reads the source,
    // and so does the task force, whose pointer the gather did not reach.
    let mut subunit = [0; 4];
    assert!(memory.copy(SHIP_A + SUBUNIT_INDEX, &mut subunit));
    assert_eq!(reads(), gathered + 1);
    let mut model = [0; 4];
    assert!(memory.copy(SHIP_A + MODEL_NUMBER, &mut model));
    assert_eq!(model, at(&bytes, SHIP_A + MODEL_NUMBER, 4));
    let mut task = [0; 8];
    assert!(memory.copy(TASK_A, &mut task));
    assert_eq!(reads(), gathered + 3);
    // A scattered read stops at the failed field, its earlier live span read.
    let (mut a, mut b, mut c, mut d) = ([0; 16], [0; 8], [0xaa; 8], [0xaa; 4]);
    assert_eq!(
        memory.copy_scattered(
            [
                SHIP_A,
                IMAGE + 0x10,
                SHIP_A + FORMATION_FLAGS,
                SHIP_A + MODEL_NUMBER
            ],
            &mut [&mut a[..], &mut b[..], &mut c[..], &mut d[..]],
        ),
        2
    );
    assert_eq!(a, at(&bytes, SHIP_A, 16));
    assert_eq!(b, at(&bytes, IMAGE + 0x10, 8));
    assert_eq!((c, d), ([0xaa; 8], [0xaa; 4]));
    assert_eq!(reads(), gathered + 4);
    assert!(!source.retried());
}

#[test]
fn scattered_reads_keep_their_live_spans_in_one_source_call() {
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&slice);
    let mut fields = ShipFields::new(MODULE);
    fields.visit(&census, SHIP_A);
    census.reset();
    let memory = fields.memory(&census);
    let addresses = [
        SHIP_A + FORMATION_FLAGS,
        SHIP_A + 0x20,
        SHIP_A + TASK_FORCE,
        IMAGE + 0x10,
    ];
    let mut spans = [[0_u8; 8]; 4];
    let [a, b, c, d] = &mut spans;
    assert_eq!(
        memory.copy_scattered_prefetched(
            addresses,
            &mut [&mut a[..], &mut b[..], &mut c[..], &mut d[..]]
        ),
        4
    );
    assert_eq!(
        (census.calls(), census.spans()),
        (1, 2),
        "one source call for the two live spans"
    );
    for (span, address) in spans.iter().zip(addresses) {
        assert_eq!(span[..], *at(&bytes, address, 8));
    }
    // Every span served: no source call.
    census.reset();
    let (mut handle, mut model) = ([0; 8], [0; 4]);
    assert_eq!(
        memory.copy_scattered(
            [SHIP_A + 8, SHIP_A + MODEL_NUMBER],
            &mut [&mut handle[..], &mut model[..]]
        ),
        2
    );
    assert_eq!(census.calls(), 0);
    assert_eq!(handle, at(&bytes, SHIP_A + 8, 8));
    // A live span that fails stops the read; a later served span stays
    // untouched.
    let (mut first, mut unreadable, mut later) = ([0; 8], [0xaa; 8], [0xaa; 4]);
    assert_eq!(
        memory.copy_scattered(
            [SHIP_A, IMAGE + LEN, SHIP_A + MODEL_NUMBER],
            &mut [&mut first[..], &mut unreadable[..], &mut later[..]]
        ),
        1
    );
    assert_eq!(first, at(&bytes, SHIP_A, 8));
    assert_eq!(later, [0xaa; 4]);
}

#[test]
fn task_forces_and_provinces_are_gathered_once_per_run_of_ships() {
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&slice);
    let mut fields = ShipFields::new(MODULE);
    let mut calls = Vec::new();
    for (ship, task) in [
        (SHIP_A, TASK_A),
        (SHIP_B, TASK_A),
        (SHIP_C, TASK_B),
        (SHIP_A, TASK_A),
    ] {
        census.reset();
        fields.visit(&census, ship);
        calls.push(census.calls());
        census.reset();
        let memory = fields.memory(&census);
        for (address, len) in [
            (ship, 16),
            (task, 8),
            (task + cunit::LOCATION, 8),
            (PROVINCE + PROVINCE_DEFINITION, 8),
            (SINGLETON, 8),
        ] {
            let mut served = vec![0; len];
            assert!(memory.copy(address, &mut served));
            assert_eq!(served, at(&bytes, address, len));
        }
        assert_eq!(census.calls(), 0, "{ship:#x}");
    }
    // A: ship, singleton, task force and province; B: ship; C: ship and
    // task force, at the same province; A again: ship and task force.
    assert_eq!(calls, [4, 1, 2, 2]);
}

#[test]
fn record_gathers_go_to_the_source_unless_they_touch_a_gathered_object() {
    const FIELD: [RecordField; 1] = [RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    }];
    let layout = RecordLayout::new(&FIELD, 8).unwrap();
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&slice);
    let mut fields = ShipFields::new(MODULE);
    fields.visit(&census, SHIP_A);
    let memory = fields.memory(&census);
    let mut output = [0; 16];
    census.reset();
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[IMAGE + 0x100, IMAGE + 0x200]),
            &layout,
            &mut output
        ),
        2
    );
    assert_eq!(census.calls(), 1, "one source gather");
    // The ship's vtable is served; the other record reads the source.
    census.reset();
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[SHIP_A, IMAGE + 0x200]),
            &layout,
            &mut output
        ),
        2
    );
    assert_eq!(census.calls(), 1);
    assert_eq!(output[..8], *at(&bytes, SHIP_A, 8));
    assert_eq!(output[8..], *at(&bytes, IMAGE + 0x200, 8));
    // A record past the image fails at its field, as the source's would.
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[SHIP_A, IMAGE + LEN]),
            &layout,
            &mut output
        ),
        1
    );
}
