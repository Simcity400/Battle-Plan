use super::*;
use crate::memory::SliceMemory;
use std::cell::{Cell, RefCell};

#[test]
fn portrait_tag_memo_holds_one_cell_per_country_and_expires_each_walk() {
    let country = |slot| crate::ccountry::Country {
        pointer: 0x1000 + slot as usize * 0x20,
        canonical_slot: slot,
        id: slot,
    };
    let tag = crate::msvc_string::Text::<64>::default();
    let mut memo = PortraitTagMemo::default();
    // An unsized memo holds nothing but records the live table's demand.
    memo.begin(5_001);
    assert!(memo.cell(4_999).is_none());
    assert_eq!(memo.required(), 5_001);
    memo.grow().unwrap();
    memo.begin(5_001);
    // Slots that shared a cell in the old 64-entry memo stay separate.
    for slot in [1, 65, 4_097, 5_000] {
        *memo.cell(slot).unwrap() = Some((country(slot), slot, tag));
    }
    for slot in [1, 65, 4_097, 5_000] {
        assert_eq!(memo.cell(slot).unwrap().unwrap().0, country(slot));
    }
    // The next walk never sees the previous walk's tags.
    memo.begin(5_001);
    assert!(
        [1, 65, 4_097, 5_000]
            .iter()
            .all(|&slot| memo.cell(slot).unwrap().is_none())
    );
}

const IMAGE: usize = 0x1_4000_0000;
const ADDRESS: usize = 0x1000;
const SKILL_AT: usize = 0x2000;
const FIRST_TRAIT: usize = 0x2800;
const SECOND_TRAIT: usize = 0x3400;
const TRAIT_ARRAY: usize = 0x6000;
const XP_ARRAY: usize = 0x7000;

fn u64_at(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn u32_at(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn inline(bytes: &mut [u8], at: usize, value: &[u8]) {
    assert!(value.len() <= 15);
    bytes[at..at + 32].fill(0);
    bytes[at..at + value.len()].copy_from_slice(value);
    u64_at(bytes, at + msvc_string::LENGTH, value.len() as u64);
    u64_at(bytes, at + msvc_string::CAPACITY, 15);
}

fn vector(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    u64_at(bytes, at, (ADDRESS + data) as u64);
    u32_at(bytes, at + 8, count);
    u32_at(bytes, at + 12, count);
}

fn fixture(vtable: usize) -> Vec<u8> {
    let mut bytes = vec![0; 0x9000];
    u64_at(&mut bytes, 0, (IMAGE + vtable) as u64);
    u32_at(&mut bytes, TYPE, REFERENCE_TYPE);
    u32_at(&mut bytes, ID, 0x8000_0001);
    u32_at(&mut bytes, CHARACTER_TYPE, ccharacter::REFERENCE_TYPE);
    u32_at(&mut bytes, CHARACTER_ID, 0xf000_0001);
    u32_at(&mut bytes, CATEGORY, (-2_i32) as u32);
    u64_at(&mut bytes, EXPERIENCE, (-9_000_000_123_i64) as u64);
    u64_at(&mut bytes, SKILL, (ADDRESS + SKILL_AT) as u64);
    u64_at(
        &mut bytes,
        SKILL_AT,
        (IMAGE + cunit_leader_skill::VTABLE_RVA) as u64,
    );
    bytes[SKILL_AT + cunit_leader_skill::VALID] = 1;
    u32_at(&mut bytes, SKILL_AT + cunit_leader_skill::LEVEL, 300);
    u32_at(
        &mut bytes,
        SKILL_AT + cunit_leader_skill::REQUIRED_EXPERIENCE,
        (-33_i32) as u32,
    );
    u32_at(&mut bytes, SKILL_AT + cunit_leader_skill::CATEGORY, 4);
    let offsets = if vtable == cnavy_leader::VTABLE_RVA {
        [
            cnavy_leader::ATTACK,
            cnavy_leader::DEFENSE,
            cnavy_leader::MANEUVERING,
            cnavy_leader::COORDINATION,
        ]
    } else {
        [
            carmy_leader::ATTACK,
            carmy_leader::DEFENSE,
            carmy_leader::PLANNING,
            carmy_leader::LOGISTICS,
        ]
    };
    for (offset, value) in offsets.into_iter().zip([256_i32, -9, i32::MAX, i32::MIN]) {
        u32_at(&mut bytes, offset, value as u32);
    }
    inline(&mut bytes, NAME, b"Name");
    inline(&mut bytes, TOKEN, &[0xff, 0, b'x']);
    for (offset, token, id, ordinal) in [
        (FIRST_TRAIT, b"first".as_slice(), 0xffff_fffe, 70000),
        (SECOND_TRAIT, b"other".as_slice(), 27, 2),
    ] {
        u64_at(
            &mut bytes,
            offset,
            (IMAGE + cunit_leader_trait::VTABLE_RVA) as u64,
        );
        u32_at(&mut bytes, offset + cunit_leader_trait::TOKEN_ID, id);
        u32_at(&mut bytes, offset + cunit_leader_trait::ORDINAL, ordinal);
        inline(&mut bytes, offset + cunit_leader_trait::TOKEN, token);
    }
    vector(&mut bytes, TRAIT_DATA, TRAIT_ARRAY, 2);
    u64_at(&mut bytes, TRAIT_ARRAY, (ADDRESS + FIRST_TRAIT) as u64);
    u64_at(&mut bytes, TRAIT_ARRAY + 8, (ADDRESS + SECOND_TRAIT) as u64);
    vector(&mut bytes, TRAIT_EXPERIENCE_DATA, XP_ARRAY, 2);
    u64_at(&mut bytes, XP_ARRAY, (ADDRESS + SECOND_TRAIT) as u64);
    u64_at(&mut bytes, XP_ARRAY + 8, i64::MIN as u64);
    u64_at(&mut bytes, XP_ARRAY + 16, (ADDRESS + FIRST_TRAIT) as u64);
    u64_at(&mut bytes, XP_ARRAY + 24, 9_000_000_321);
    bytes
}

fn memory(bytes: &[u8]) -> SliceMemory<'_> {
    SliceMemory {
        base: ADDRESS,
        bytes,
    }
}

struct TracedMemory<'a> {
    memory: SliceMemory<'a>,
    guards: Cell<usize>,
    spans: RefCell<Vec<(usize, usize)>>,
    fail: Option<usize>,
}

impl TracedMemory<'_> {
    fn span(&self, address: usize, destination: &mut [u8]) -> bool {
        if destination.is_empty() {
            return true;
        }
        self.spans.borrow_mut().push((address, destination.len()));
        if self.fail == Some(address) {
            return false;
        }
        self.memory.copy(address, destination)
    }
}

impl Memory for TracedMemory<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.guards.set(self.guards.get() + 1);
        self.span(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.guards.set(self.guards.get() + 1);
        for (index, (address, destination)) in addresses.into_iter().zip(destinations).enumerate() {
            if !self.span(address, destination) {
                return index;
            }
        }
        N
    }
}

fn traced(bytes: &[u8], fail: Option<usize>) -> TracedMemory<'_> {
    TracedMemory {
        memory: memory(bytes),
        guards: Cell::new(0),
        spans: RefCell::new(Vec::new()),
        fail,
    }
}

fn single_heap_xp_fixture() -> Vec<u8> {
    let mut bytes = fixture(carmy_leader::VTABLE_RVA);
    vector(&mut bytes, TRAIT_EXPERIENCE_DATA, XP_ARRAY, 1);
    let token = SECOND_TRAIT + cunit_leader_trait::TOKEN;
    u64_at(&mut bytes, token, (ADDRESS + 0x8000) as u64);
    u64_at(&mut bytes, token + msvc_string::LENGTH, 20);
    u64_at(&mut bytes, token + msvc_string::CAPACITY, 31);
    bytes[0x8000..0x8014].fill(0xff);
    bytes
}

#[test]
fn trait_batches_keep_exact_read_ranges_and_reduce_guard_entries() {
    let bytes = single_heap_xp_fixture();
    let source = traced(&bytes, None);
    let mut rows = [TraitExperienceRow::default(); 1];
    let mut arena = [0; 20];
    let result = walk_trait_experience(&source, IMAGE, ADDRESS, 0x8000_0001, &mut rows, &mut arena);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(source.guards.get(), 5); // Two root reads and three for one heap-backed entry.
    assert_eq!(
        *source.spans.borrow(),
        [
            (ADDRESS, 16),
            (ADDRESS + TRAIT_EXPERIENCE_DATA, 16),
            (ADDRESS + XP_ARRAY, 16),
            (ADDRESS + SECOND_TRAIT, 12),
            (ADDRESS + SECOND_TRAIT + cunit_leader_trait::ORDINAL, 4),
            (ADDRESS + SECOND_TRAIT + cunit_leader_trait::TOKEN, 32),
            (ADDRESS + 0x8014, 1),
            (ADDRESS + 0x8000, 20),
        ]
    );
    assert_eq!(arena, [0xff; 20]);
    assert_eq!(rows[0].experience, i64::MIN);
    let inline_bytes = fixture(carmy_leader::VTABLE_RVA);
    let inline_source = traced(&inline_bytes, None);
    assert_eq!(
        walk_trait_experience(
            &inline_source,
            IMAGE,
            ADDRESS,
            0x8000_0001,
            &mut [TraitExperienceRow::default(); 2],
            &mut [0; 10],
        )
        .outcome,
        Outcome::Rows { len: 2 },
    );
    assert_eq!(inline_source.guards.get(), 6); // Two root reads, two per inline entry.
    let source = traced(&bytes, None);
    assert!(read(&source, IMAGE, ADDRESS, 0xf000_0001).is_ok());
    // Identity, state, skill, components, then owner and visibility.
    assert_eq!(source.guards.get(), 5);
    assert_eq!(
        source
            .spans
            .borrow()
            .iter()
            .map(|span| span.1)
            .sum::<usize>(),
        81 + 4 + 8 + 1 + 1
    );
}

/// The owner, both visibility inputs and, for an army leader only, the
/// captured byte come from one more scattered read.
#[test]
fn owner_visibility_and_the_army_captured_byte_are_read() {
    let mut bytes = fixture(carmy_leader::VTABLE_RVA);
    u32_at(&mut bytes, OWNER, 7);
    u64_at(&mut bytes, VISIBILITY_CHOICE, 0x1234);
    bytes[TEMPLATE_VISIBILITY] = 1;
    bytes[carmy_leader::CAPTURED] = 1;
    let row = read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).unwrap();
    assert_eq!(
        (
            row.owner,
            row.visibility_choice,
            row.template_visibility,
            row.captured
        ),
        (7, 1, 1, 1)
    );
    let mut navy = fixture(cnavy_leader::VTABLE_RVA);
    u32_at(&mut navy, OWNER, 7);
    navy[carmy_leader::CAPTURED] = 1;
    let row = read(&memory(&navy), IMAGE, ADDRESS, 0xf000_0001).unwrap();
    assert_eq!((row.owner, row.visibility_choice, row.captured), (7, 0, 0));
}

#[test]
fn trait_batch_faults_preserve_prefix_stops_and_failure_precedence() {
    let mut bytes = single_heap_xp_fixture();
    let definition = ADDRESS + SECOND_TRAIT;
    let spans = [
        definition,
        definition + cunit_leader_trait::ORDINAL,
        definition + cunit_leader_trait::TOKEN,
        ADDRESS + 0x8014,
        ADDRESS + 0x8000,
    ];
    for (index, fail) in spans.into_iter().enumerate() {
        let source = traced(&bytes, Some(fail));
        let mut rows = [TraitExperienceRow::default(); 1];
        let mut arena = [0; 20];
        let result =
            walk_trait_experience(&source, IMAGE, ADDRESS, 0x8000_0001, &mut rows, &mut arena);
        let reason = match index {
            0..=2 => "unit leader trait identity unreadable",
            3 => "unit leader string terminator invalid or unreadable",
            _ => "unit leader trait token bytes unreadable",
        };
        assert_eq!(result.outcome, Outcome::Unavailable { reason });
        assert_eq!(result.bytes_len, 0);
        for later in &spans[index + 1..] {
            assert!(!source.spans.borrow().iter().any(|span| span.0 == *later));
        }
    }
    // A readable invalid identity wins over an unreadable later scalar and
    // never permits traversal of the string pointer from that object.
    u64_at(&mut bytes, SECOND_TRAIT, 0);
    let source = traced(&bytes, Some(definition + cunit_leader_trait::ORDINAL));
    let result = walk_trait_experience(
        &source,
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut [TraitExperienceRow::default(); 1],
        &mut [0; 20],
    );
    assert_eq!(
        result.outcome,
        Outcome::Unavailable {
            reason: "unit leader trait vtable mismatch"
        }
    );
    assert!(
        !source
            .spans
            .borrow()
            .iter()
            .any(|span| span.0 == ADDRESS + 0x8000)
    );
}

#[test]
fn native_widths_and_four_class_layouts_remain_distinct() {
    for (vtable, kind) in [
        (VTABLE_RVA, KIND_BASE),
        (carmy_leader::VTABLE_RVA, KIND_ARMY),
        (cnavy_leader::VTABLE_RVA, KIND_NAVY),
        (coperative_leader::VTABLE_RVA, KIND_OPERATIVE),
    ] {
        let bytes = fixture(vtable);
        let row = read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).unwrap();
        assert_eq!(
            (row.reference_type, row.id, row.character_id),
            (4713, 0x8000_0001, 0xf000_0001)
        );
        assert_eq!(
            (row.kind, row.category, row.experience),
            (kind, -2, -9_000_000_123)
        );
        assert_eq!(
            (
                row.skill_status,
                row.skill_valid,
                row.skill_level,
                row.skill_required_experience,
                row.skill_category
            ),
            (2, 1, 300, -33, 4)
        );
        assert_eq!(
            row.components,
            if kind == KIND_ARMY || kind == KIND_NAVY {
                [256, -9, i32::MAX, i32::MIN]
            } else {
                [0; 4]
            }
        );
    }
}

#[test]
fn absent_and_native_invalid_skill_are_distinct_from_read_failure() {
    let mut bytes = fixture(carmy_leader::VTABLE_RVA);
    bytes[SKILL_AT + cunit_leader_skill::VALID] = 0;
    let row = read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).unwrap();
    assert_eq!((row.skill_status, row.skill_level), (1, 300));
    u64_at(&mut bytes, SKILL, 0);
    let row = read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).unwrap();
    assert_eq!((row.skill_status, row.skill_level), (0, 0));
    u64_at(&mut bytes, SKILL, usize::MAX as u64);
    assert!(read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).is_err());
}

#[test]
fn wrong_role_identity_backlinks_and_incomplete_skill_refuse_the_row() {
    for offset in [0, TYPE, CHARACTER_TYPE, CHARACTER_ID, SKILL_AT] {
        let mut bytes = fixture(carmy_leader::VTABLE_RVA);
        u32_at(&mut bytes, offset, 0);
        assert!(read(&memory(&bytes), IMAGE, ADDRESS, 0xf000_0001).is_err());
    }
    let bytes = fixture(carmy_leader::VTABLE_RVA);
    assert!(read(&memory(&bytes), IMAGE, ADDRESS, 7).is_err());
    assert!(
        read(
            &memory(&bytes[..SKILL_AT + cunit_leader_skill::CATEGORY + 3]),
            IMAGE,
            ADDRESS,
            0xf000_0001
        )
        .is_err()
    );
    assert!(read(&memory(&bytes), IMAGE, usize::MAX - 3, 0xf000_0001).is_err());
}

#[test]
fn text_preserves_long_heap_bytes_and_omits_whole_rows_when_short() {
    let mut bytes = fixture(carmy_leader::VTABLE_RVA);
    let name = vec![0xfe; 4097];
    u64_at(&mut bytes, NAME, (ADDRESS + 0x4000) as u64);
    u64_at(&mut bytes, NAME + msvc_string::LENGTH, name.len() as u64);
    u64_at(&mut bytes, NAME + msvc_string::CAPACITY, name.len() as u64);
    bytes[0x4000..0x4000 + name.len()].copy_from_slice(&name);
    let mut arena = vec![0; name.len() + 3];
    let result = read_text(&memory(&bytes), ADDRESS, &mut arena);
    assert_eq!(result.outcome, Outcome::Rows { len: 1 });
    assert_eq!(&arena[..name.len()], &name);
    assert_eq!(&arena[name.len()..], &[0xff, 0, b'x']);
    let mut short = [0x99; 5];
    let result = read_text(&memory(&bytes), ADDRESS, &mut short);
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 1
        }
    );
    assert_eq!((result.bytes_len, result.required_bytes), (0, 4100));
    assert_eq!(short, [0x99; 5]);
    bytes[0x4000 + name.len()] = 1;
    assert!(matches!(
        read_text(&memory(&bytes), ADDRESS, &mut arena).outcome,
        Outcome::Unavailable { .. }
    ));
}

#[test]
fn assigned_traits_preserve_order_and_report_both_capacities_exactly() {
    let bytes = fixture(carmy_leader::VTABLE_RVA);
    let mut rows = [TraitRow::default(); 2];
    let mut arena = [0; 10];
    let result = walk_traits(
        &memory(&bytes),
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut rows,
        &mut arena,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (
            rows[0].ordinal,
            rows[0].token_id,
            rows[0].definition_ordinal
        ),
        (0, -2, 70000)
    );
    assert_eq!((rows[1].ordinal, rows[1].token_id), (1, 27));
    assert_eq!(&arena, b"firstother");
    let result = walk_traits(
        &memory(&bytes),
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut rows[..1],
        &mut arena,
    );
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    assert_eq!((result.bytes_len, result.required_bytes), (5, 5));
    let result = walk_traits(
        &memory(&bytes),
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut rows,
        &mut arena[..4],
    );
    assert_eq!(
        result.outcome,
        Outcome::Truncated {
            len: 0,
            unread_rows: 2
        }
    );
    assert_eq!((result.bytes_len, result.required_bytes), (0, 5));
}

#[test]
fn trait_experience_is_independent_ordered_native_state() {
    let bytes = fixture(coperative_leader::VTABLE_RVA);
    let mut rows = [TraitExperienceRow::default(); 2];
    let mut arena = [0; 10];
    let result = walk_trait_experience(
        &memory(&bytes),
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut rows,
        &mut arena,
    );
    assert_eq!(result.outcome, Outcome::Rows { len: 2 });
    assert_eq!(
        (
            rows[0].trait_row.ordinal,
            rows[0].trait_row.token_id,
            rows[0].experience
        ),
        (0, 27, i64::MIN)
    );
    assert_eq!(
        (
            rows[1].trait_row.ordinal,
            rows[1].trait_row.token_id,
            rows[1].experience
        ),
        (1, -2, 9_000_000_321)
    );
    assert_eq!(&arena, b"otherfirst");
}

#[test]
fn trait_faults_never_certify_prefixes_and_empty_vectors_are_available() {
    let mut bytes = fixture(carmy_leader::VTABLE_RVA);
    let mut rows = [TraitRow::default(); 2];
    let mut arena = [0; 10];
    u64_at(&mut bytes, TRAIT_ARRAY + 8, 0);
    let result = walk_traits(
        &memory(&bytes),
        IMAGE,
        ADDRESS,
        0x8000_0001,
        &mut rows,
        &mut arena,
    );
    assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
    assert_eq!(result.bytes_len, 0);
    // The same bad pointer after capacity is not visited or certified.
    assert_eq!(
        walk_traits(
            &memory(&bytes),
            IMAGE,
            ADDRESS,
            0x8000_0001,
            &mut rows[..1],
            &mut arena
        )
        .outcome,
        Outcome::Truncated {
            len: 1,
            unread_rows: 1
        }
    );
    u32_at(&mut bytes, TRAIT_COUNT, (-1_i32) as u32);
    assert!(matches!(
        walk_traits(
            &memory(&bytes),
            IMAGE,
            ADDRESS,
            0x8000_0001,
            &mut rows,
            &mut arena
        )
        .outcome,
        Outcome::Unavailable { .. }
    ));
    bytes[TRAIT_DATA..TRAIT_DATA + 16].fill(0);
    assert_eq!(
        walk_traits(
            &memory(&bytes),
            IMAGE,
            ADDRESS,
            0x8000_0001,
            &mut rows,
            &mut arena
        )
        .outcome,
        Outcome::Rows { len: 0 }
    );
    assert!(matches!(
        walk_traits(&memory(&bytes), IMAGE, ADDRESS, 1, &mut rows, &mut arena).outcome,
        Outcome::Unavailable { .. }
    ));
}
