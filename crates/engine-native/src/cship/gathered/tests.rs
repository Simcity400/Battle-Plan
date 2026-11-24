use super::*;
use engine_native_memory::{memory::{RecordField, SliceMemory}, read_census::ReadCensus};

const IMAGE: usize = 0x1000_0000;
const LEN: usize = 0x6000;
const SHIP_A: usize = IMAGE + 0x1000;
const SHIP_B: usize = IMAGE + 0x2000;
const SHIP_C: usize = IMAGE + 0x3000;
const TASK_A: usize = IMAGE + 0x4000;
const TASK_B: usize = IMAGE + 0x4800;
const PROVINCE: usize = IMAGE + 0x5000;
const SINGLETON: usize = IMAGE + 0x5800;
const MODULE: usize = SINGLETON - cgame_state::SINGLETON_RVA.0;

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
fn model_comparison_excludes_only_organisation_inputs_and_requires_complete_observations() {
    let kept = HeldShip {
        ship: Object {
            pointer: SHIP_A,
            completed: SHIP.len(),
            ..Object::NONE
        },
        heap: Heap::NONE,
        variants: [Object::NONE; POOL_ENTRIES],
        naval_variants: [Object::NONE; POOL_ENTRIES],
    };
    assert!(kept.same_model(&kept, 0));
    assert!(
        !kept.same_model(&kept, 1),
        "current census class still authenticates"
    );
    for (index, &(offset, len)) in SHIP.iter().enumerate() {
        for byte in 0..len {
            let mut fresh = kept;
            fresh.ship.bytes[index][byte] ^= 1;
            let organisation_only =
                matches!(offset, WEATHER_FACTOR | MODIFIERS | EXPERIENCE | CRITICALS)
                    || offset == BASE_ORGANISATION
                        && byte < super::super::MAXIMUM_STRENGTH - BASE_ORGANISATION;
            assert_eq!(
                fresh.same_model(&kept, 0),
                organisation_only,
                "field {index}, byte {byte}"
            );
        }
    }
    let mut fresh = kept;
    fresh.ship.completed -= 1;
    assert!(!fresh.same_model(&kept, 0));
    assert!(!kept.same_model(&fresh, 0));
    fresh = kept;
    fresh.heap = Heap {
        completed: 1,
        spans: [(IMAGE, 8), (0, 0)],
        ..Heap::NONE
    };
    let mut changed = fresh;
    changed.heap.pool[0] = 1;
    assert!(
        !fresh.same_model(&changed, 0),
        "pool payloads are model inputs"
    );
    changed = fresh;
    changed.heap.spans[0].0 += 8;
    assert!(
        !fresh.same_model(&changed, 0),
        "pool allocation identity still matches"
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
    let mut outside = [0; 8];
    assert!(memory.copy(SHIP_A + 0x20, &mut outside));
    assert_eq!(outside, at(&bytes, SHIP_A + 0x20, 8));
    let mut across = [0; 16];
    assert!(memory.copy(SHIP_A + TASK_FORCE - 8, &mut across));
    assert_eq!(across, at(&bytes, SHIP_A + TASK_FORCE - 8, 16));
    assert_eq!(census.calls(), 2);
}

#[test]
fn indexed_field_lookup_matches_linear_for_all_gather_prefixes_and_boundaries() {
    fn check<const N: usize>(fields: [(usize, usize); N]) {
        let mut object = Object::<N> {
            pointer: IMAGE,
            ..Object::NONE
        };
        for (index, bytes) in object.bytes.iter_mut().enumerate() {
            bytes.fill(index as u8 + 1);
        }
        for completed in 0..=N {
            object.completed = completed;
            for offset in 0..=fields.last().map_or(4, |&(at, len)| at + len + 4) {
                for len in 0..=WIDTH + 1 {
                    let expected = if len == 0 {
                        Lookup::Source
                    } else {
                        fields
                            .iter()
                            .enumerate()
                            .find_map(|(index, &(start, width))| {
                                (start <= offset && offset + len <= start + width).then(|| {
                                    match index.cmp(&completed) {
                                        Ordering::Less => Lookup::Served(
                                            &object.bytes[index]
                                                [offset - start..offset + len - start],
                                        ),
                                        Ordering::Equal if offset == start && len == width => {
                                            Lookup::Failed
                                        }
                                        _ => Lookup::Source,
                                    }
                                })
                            })
                            .unwrap_or(Lookup::Source)
                    };
                    let observed = object.lookup(&fields, IMAGE + offset, len);
                    match (expected, observed) {
                        (Lookup::Served(old), Lookup::Served(new)) => assert_eq!(old, new),
                        (Lookup::Failed, Lookup::Failed) | (Lookup::Source, Lookup::Source) => {}
                        _ => panic!("prefix {completed}, offset {offset}, len {len}"),
                    }
                }
            }
        }
        assert!(matches!(
            object.lookup(&fields, IMAGE - 1, 1),
            Lookup::Source
        ));
        assert!(matches!(
            object.lookup(&fields, usize::MAX, usize::MAX),
            Lookup::Source
        ));
        object.pointer = 0;
        assert!(matches!(object.lookup(&fields, IMAGE, 1), Lookup::Source));
    }
    check([]);
    check([(4, 8)]);
    check(std::array::from_fn::<_, 31, _>(|index| {
        (4 + index * 40, 1 + index % WIDTH)
    }));
}

#[test]
fn a_failed_field_fails_again_without_a_read_and_later_fields_read_on_first_use() {
    let bytes = image();
    let slice = SliceMemory {
        base: IMAGE,
        bytes: &bytes,
    };
    let source = FaultAt::new(&slice, Some(SHIP_A + FORMATION_FLAGS + 6));
    let mut fields = ShipFields::new(MODULE);
    fields.visit(&source, SHIP_A);
    let reads = || source.reads.borrow().len();
    let gathered = reads();
    let memory = fields.memory(&source);
    let mut flags = [0; 8];
    assert!(!memory.copy(SHIP_A + FORMATION_FLAGS, &mut flags));
    assert_eq!(reads(), gathered, "the failed field is not read again");
    let mut low = [0; 4];
    assert!(memory.copy(SHIP_A + FORMATION_FLAGS, &mut low));
    assert_eq!(low, at(&bytes, SHIP_A + FORMATION_FLAGS, 4));
    assert_eq!(reads(), gathered + 1);
    let mut subunit = [0; 4];
    assert!(memory.copy(SHIP_A + SUBUNIT_INDEX, &mut subunit));
    assert_eq!(reads(), gathered + 1);
    let mut model = [0; 4];
    assert!(memory.copy(SHIP_A + MODEL_NUMBER, &mut model));
    assert_eq!(model, at(&bytes, SHIP_A + MODEL_NUMBER, 4));
    let mut task = [0; 8];
    assert!(memory.copy(TASK_A, &mut task));
    assert_eq!(reads(), gathered + 3);
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
    assert_eq!(
        memory.copy_records(
            RecordSources::Pointers(&[SHIP_A, IMAGE + LEN]),
            &layout,
            &mut output
        ),
        1
    );
}

#[test]
fn worker_power_fields_invalidate_variant_reuse_without_widening_ordinary_gathers() {
    let pointer = 0x1000;
    let mut bytes = vec![0; 0x2000];
    bytes[pointer..pointer + 8]
        .copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
    bytes[pointer + 8..pointer + 16].copy_from_slice(
        &((1_u64 << 32) | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
    );
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut ordinary = ShipFields::new(0);
    let slot = ordinary.variant(&memory, pointer);
    assert_eq!(ordinary.naval_variants[slot].pointer, 0);
    let mut first = ShipFields::new(0);
    first.enable_naval_power();
    let slot = first.variant(&memory, pointer);
    first.naval_variant(&memory, slot);
    let held = HeldVariant(first.variants[slot], first.naval_variants[slot]);
    bytes[pointer + super::super::naval_power::CARRIER_SIZE
        ..pointer + super::super::naval_power::CARRIER_SIZE + 8]
        .copy_from_slice(&123_i64.to_le_bytes());
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut current = ShipFields::new(0);
    current.enable_naval_power();
    assert!(!current.held_variant_unchanged(&memory, &held));
}

#[test]
fn power_companion_demands_only_the_selected_variant_and_selected_carrier_word() {
    let mut bytes = vec![0; 0x5000];
    let first = 0x1000_usize;
    let selected = 0x2000_usize;
    for (pointer, flags) in [
        (first, 0),
        (selected, super::super::naval_power::CARRIER_SIZE_SELECTOR),
    ] {
        bytes[pointer..pointer + 8]
            .copy_from_slice(&(engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[pointer + 8..pointer + 16].copy_from_slice(
            &((pointer as u64) << 32 | u64::from(cequipment_variant::REFERENCE_TYPE)).to_le_bytes(),
        );
        bytes[pointer + cequipment_variant::CLASSIFIER_FLAGS
            ..pointer + cequipment_variant::CLASSIFIER_FLAGS + 8]
            .copy_from_slice(&flags.to_le_bytes());
    }
    let mut heap = Heap {
        spans: [(0x4000, 2 * equipment_entry::STRIDE), (0, 0)],
        completed: 1,
        ..Heap::NONE
    };
    for (index, (pointer, amount)) in [(first, 3_i64), (selected, 7)].into_iter().enumerate() {
        let at = index * equipment_entry::STRIDE;
        heap.pool[at..at + 8].copy_from_slice(&(pointer as u64).to_le_bytes());
        heap.pool[at + 8..at + 16].copy_from_slice(&amount.to_le_bytes());
    }
    let slice = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let census = ReadCensus::new(&slice);
    let mut ordinary = ShipFields::new(0);
    ordinary.visit_variants_of(&census, &heap);
    let ordinary_bytes = census.bytes();
    let ordinary_calls = census.calls();
    census.reset();
    let mut fields = ShipFields::new(0);
    fields.enable_naval_power();
    fields.visit_variants_of(&census, &heap);
    assert_eq!(
        census.bytes() - ordinary_bytes,
        32,
        "one heavy attack, speed/range and selected carrier word"
    );
    assert_eq!(
        census.calls() - ordinary_calls,
        1,
        "only the selected variant adds a gather"
    );
    let held = fields.current_variants();
    let powers = fields.current_naval_variants();
    assert_eq!(powers[0].pointer, 0);
    assert_eq!(powers[1].pointer, selected);
    let selected_fields = naval_power_fields(&held[1]).unwrap();
    assert_eq!(
        selected_fields[2],
        (super::super::naval_power::CARRIER_SIZE + 8, 8)
    );
    let unselected = HeldVariant(held[0], powers[0]);
    let chosen = HeldVariant(held[1], powers[1]);
    bytes[first + super::super::naval_power::CARRIER_SIZE
        ..first + super::super::naval_power::CARRIER_SIZE + 8]
        .copy_from_slice(&77_i64.to_le_bytes());
    bytes[selected + super::super::naval_power::CARRIER_SIZE
        ..selected + super::super::naval_power::CARRIER_SIZE + 8]
        .copy_from_slice(&88_i64.to_le_bytes());
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut later = ShipFields::new(0);
    later.enable_naval_power();
    assert!(later.held_variant_unchanged(&memory, &unselected));
    assert!(
        later.held_variant_unchanged(&memory, &chosen),
        "the unselected carrier word is not an input"
    );
    bytes[selected + super::super::naval_power::CARRIER_SIZE + 8
        ..selected + super::super::naval_power::CARRIER_SIZE + 16]
        .copy_from_slice(&99_i64.to_le_bytes());
    let memory = SliceMemory {
        base: 0,
        bytes: &bytes,
    };
    let mut later = ShipFields::new(0);
    later.enable_naval_power();
    assert!(!later.held_variant_unchanged(&memory, &chosen));
}

