use super::*;
use engine_native_parts::{creference_registry as registry, reference_bucket};
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

const UNIT: usize = 0x1000;
const VECTOR: usize = 0x2000;
const OBJECTS: [usize; 3] = [0x3000, 0x3400, 0x3800];
const TABLE: usize = 0x6000;
const BUCKETS: usize = 0x6100;
const HANDLES: [u64; 3] = [
    0xffff_fffe_0000_1b59,
    0x8000_0001_0000_1b5a,
    0x7fff_ffff_0000_1b5b,
];

struct Image {
    bytes: Vec<u8>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<(usize, usize)>,
    changed_header: bool,
}
impl Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        if self.fault == Some((address, output.len())) {
            return false;
        }
        if address == registry::HIGH_ROOT_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&(TABLE as u64).to_le_bytes());
            return true;
        }
        let copied = SliceMemory {
            base: 0,
            bytes: &self.bytes,
        }
        .copy(address, output);
        if copied
            && self.changed_header
            && address == UNIT + COMBAT_REFERENCE_DATA
            && output.len() == 16
        {
            output[12] ^= 1;
        }
        copied
    }
}

fn word(bytes: &mut [u8], address: usize, value: u64) {
    bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
}
fn header(bytes: &mut [u8], count: u32) -> [u8; 16] {
    let mut value = [0; 16];
    value[..8].copy_from_slice(&(if count == 0 { 0 } else { VECTOR as u64 }).to_le_bytes());
    value[8..12].copy_from_slice(&count.to_le_bytes());
    value[12..16].copy_from_slice(&count.to_le_bytes());
    bytes[UNIT + COMBAT_REFERENCE_DATA..UNIT + COMBAT_REFERENCE_DATA + 16].copy_from_slice(&value);
    value
}
fn fixture() -> (Image, [u8; 16]) {
    let mut bytes = vec![0; 0x7000];
    word(&mut bytes, TABLE + registry::ENTRIES, BUCKETS as u64);
    bytes[TABLE + registry::COUNT..TABLE + registry::COUNT + 4]
        .copy_from_slice(&3_u32.to_le_bytes());
    bytes[TABLE + registry::MASK..TABLE + registry::MASK + 4].copy_from_slice(&7_u32.to_le_bytes());
    bytes[TABLE + registry::OVERFLOW_DISTANCE] = 5;
    bytes[BUCKETS + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE] = 255;
    let classes = [
        (
            crate::cland_combat::VTABLE_RVA.0,
            crate::cland_combat::BODY_VTABLE_RVA.0,
        ),
        (
            engine_native_parts::cland_border_war_combat::VTABLE_RVA.0,
            engine_native_parts::cland_border_war_combat::BODY_VTABLE_RVA.0,
        ),
        (crate::cnaval_combat::VTABLE_RVA.0, 0x02a5_6e70),
    ];
    for ((pointer, handle), (primary, secondary)) in OBJECTS.into_iter().zip(HANDLES).zip(classes) {
        word(&mut bytes, pointer, primary as u64);
        word(&mut bytes, pointer + 0x10, secondary as u64);
        word(&mut bytes, pointer + 0x18, handle);
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(registry::HASH_MULTIPLIER);
        let slot =
            BUCKETS + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE;
        assert_eq!(
            bytes[slot + reference_bucket::DISTANCE],
            0,
            "fixture keys occupy distinct slots"
        );
        bytes[slot + reference_bucket::DISTANCE] = 1;
        word(&mut bytes, slot + reference_bucket::TYPE, handle);
        word(
            &mut bytes,
            slot + reference_bucket::OBJECT,
            (pointer + 0x10) as u64,
        );
    }
    for (index, pointer) in [OBJECTS[0], OBJECTS[1], OBJECTS[2], OBJECTS[0]]
        .into_iter()
        .enumerate()
    {
        word(&mut bytes, VECTOR + index * 8, pointer as u64);
    }
    let value = header(&mut bytes, 4);
    (
        Image {
            bytes,
            reads: Default::default(),
            fault: None,
            changed_header: false,
        },
        value,
    )
}
fn capture(
    image: &Image,
    value: [u8; 16],
    output: &mut Vec<MapCombatReference>,
) -> Result<usize, &'static str> {
    let mut resolver = registry::HeaderResolver::default();
    read_unit_combat_references_with(image, UNIT, value, OBJECTS[0], output,
        0, |pointer| {
        super::super::map_combat_identity(image, 0, pointer, &mut resolver)
    })
}

#[test]
fn ordered_classes_raw_handles_repeats_and_first_pointer_reuse() {
    let (image, value) = fixture();
    let mut output = vec![MapCombatReference::default(); 4];
    assert_eq!(capture(&image, value, &mut output), Ok(4));
    assert_eq!(
        output,
        [
            MapCombatReference {
                combat: HANDLES[0],
                kind: COMBAT_LAND
            },
            MapCombatReference {
                combat: HANDLES[1],
                kind: COMBAT_BORDER_WAR
            },
            MapCombatReference {
                combat: HANDLES[2],
                kind: COMBAT_NAVAL
            },
            MapCombatReference {
                combat: HANDLES[0],
                kind: COMBAT_LAND
            },
        ]
    );
    let reads = image.reads.borrow();
    assert!(
        !reads.iter().any(|&(address, _)| address == VECTOR),
        "legacy first pointer is supplied, never reread"
    );
    for index in 1..4 {
        assert_eq!(
            reads
                .iter()
                .filter(|&&(address, len)| address == VECTOR + index * 8 && len == 8)
                .count(),
            1
        );
    }
    assert_eq!(
        reads
            .iter()
            .filter(|&&(address, len)| address == OBJECTS[0] && len == 32)
            .count(),
        2,
        "repeated occurrence remains an independent identity observation"
    );
}

#[test]
fn unreadable_tail_keeps_first_refusal_and_never_claims_complete() {
    let (mut image, value) = fixture();
    image.fault = Some((VECTOR + 8, 8));
    let mut output = vec![MapCombatReference::default(); 4];
    assert_eq!(
        capture(&image, value, &mut output),
        Err("unit combat-reference pointer unreadable or null")
    );
    assert_eq!(output[0].combat, HANDLES[0]);
    assert!(
        !image
            .reads
            .borrow()
            .iter()
            .any(|&(address, _)| address == VECTOR + 16)
    );
}

#[test]
fn later_identity_and_registry_faults_do_not_become_empty_or_class_guesses() {
    for fault in [(OBJECTS[1], 32), (registry::HIGH_ROOT_RVA.0, 8)] {
        let (mut image, value) = fixture();
        image.fault = Some(fault);
        let mut output = vec![MapCombatReference::default(); 4];
        assert!(capture(&image, value, &mut output).is_err());
    }
    for mutation in [0_u64, 0x1234] {
        let (mut image, value) = fixture();
        word(&mut image.bytes, OBJECTS[1] + 0x10, mutation);
        assert_eq!(
            capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
            Err("map combat secondary identity invalid")
        );
    }
    let (mut image, value) = fixture();
    let handle = HANDLES[1];
    let hash = handle
        .rotate_left(32)
        .wrapping_mul(registry::HASH_MULTIPLIER);
    let slot =
        BUCKETS + (((hash >> 32) as u32 ^ hash as u32) & 7) as usize * reference_bucket::STRIDE;
    word(
        &mut image.bytes,
        slot + reference_bucket::OBJECT,
        (OBJECTS[1] + 0x20) as u64,
    );
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Err("map combat registry identity mismatch")
    );
}

#[test]
fn final_header_change_withholds_the_complete_suffix() {
    let (mut image, value) = fixture();
    image.changed_header = true;
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Err("unit combat-reference vector changed or unreadable")
    );
    image.changed_header = false;
    image.fault = Some((UNIT + COMBAT_REFERENCE_DATA, 16));
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Err("unit combat-reference vector changed or unreadable")
    );
}

#[test]
fn pointer_null_and_unsupported_class_keep_independent_static_refusals() {
    let (mut image, value) = fixture();
    word(&mut image.bytes, VECTOR + 8, 0);
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Err("unit combat-reference pointer unreadable or null")
    );
    let (mut image, value) = fixture();
    word(
        &mut image.bytes,
        OBJECTS[1],
        crate::carmy::VTABLE_RVA.0 as u64,
    );
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Err("map combat class unsupported")
    );
}

#[test]
fn worker_combat_reference_arena_grows_from_the_original_count() {
    let (mut image, _) = fixture();
    let mut rows = Vec::new();
    for count in [50, 200] {
        let mut capture =
            CombatReferencesCapture::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut rows));
        capture.begin(count);
        assert!(capture.failure.is_none());
        for _ in 0..count {
            capture.observe(
                &image,
                0,
                OBJECTS[0],
                &mut registry::HeaderResolver::default(),
            );
        }
        let value = header(&mut image.bytes, count);
        capture.finish(&image, UNIT, value);
        assert_eq!(capture.result(), Ok(count as usize));
        assert!(
            capture.entries[..count as usize]
                .iter()
                .all(|row| row.combat == HANDLES[0])
        );
    }
}

struct PartialFault<'a> {
    image: &'a Image,
    byte: Cell<Option<usize>>,
    once: bool,
}
impl Memory for PartialFault<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self
            .byte
            .get()
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            if self.once {
                self.byte.set(None);
            }
            output.fill(0xff);
            return false;
        }
        self.image.copy(address, output)
    }
}

#[test]
fn movement_memo_matches_direct_values_and_first_failures_including_partial_records() {
    let (image, value) = fixture();
    assert_eq!(
        capture(&image, value, &mut vec![MapCombatReference::default(); 4]),
        Ok(4)
    );
    let spans = image.reads.borrow().clone();
    for once in [false, true] {
        for fault in std::iter::once(None).chain(
            spans
                .iter()
                .flat_map(|&(at, len)| [Some(at), Some(at + len / 2), Some(at + len - 1)]),
        ) {
            let actual = PartialFault {
                image: &image,
                byte: Cell::new(fault),
                once,
            };
            let direct = PartialFault {
                image: &image,
                byte: Cell::new(fault),
                once,
            };
            let mut memo = MapCombatIdentityMemo::default();
            memo.reserve(4).unwrap();
            let mut resolver = registry::HeaderResolver::default();
            for _ in 0..3 {
                let mut output = vec![MapCombatReference::default(); 4];
                let mut expected = output.clone();
                let result = read_unit_combat_references(
                    &actual,
                    0,
                    UNIT,
                    value,
                    OBJECTS[0],
                    &mut output,
                    0,
                    &mut memo,
                );
                let reference = read_unit_combat_references_with(
                    &direct,
                    UNIT,
                    value,
                    OBJECTS[0],
                    &mut expected,
                    0,
                    |pointer| super::super::map_combat_identity(&direct, 0, pointer, &mut resolver),
                );
                assert_eq!(
                    (result, output),
                    (reference, expected),
                    "fault {fault:?}, once {once}"
                );
                assert_eq!(actual.byte.get(), direct.byte.get());
            }
        }
    }
}

#[test]
fn movement_memo_keeps_header_rechecks_and_uncached_fallback_refusals() {
    for mutation in 0..5 {
        let (mut image, value) = fixture();
        match mutation {
            0 => image.changed_header = true,
            1 => word(
                &mut image.bytes,
                OBJECTS[1],
                crate::carmy::VTABLE_RVA.0 as u64,
            ),
            2 => word(&mut image.bytes, OBJECTS[1] + 0x10, 0),
            3 => word(&mut image.bytes, VECTOR + 8, 0),
            _ => word(&mut image.bytes, OBJECTS[1] + 0x18, 0),
        }
        for capacity in [0, 4] {
            let mut memo = MapCombatIdentityMemo::default();
            memo.reserve(capacity).unwrap();
            let allocated = memo.allocated_bytes();
            for _ in 0..2 {
                let mut actual = vec![MapCombatReference::default(); 4];
                let mut expected = actual.clone();
                let result = read_unit_combat_references(
                    &image,
                    0,
                    UNIT,
                    value,
                    OBJECTS[0],
                    &mut actual,
                    0,
                    &mut memo,
                );
                let reference = capture(&image, value, &mut expected);
                assert_eq!((result, actual), (reference, expected));
                assert_eq!(memo.allocated_bytes(), allocated);
            }
        }
    }
}

