use super::*;
use engine_native_parts::equipment_entry;
use engine_native_memory::memory::SliceMemory;
use std::cell::{Cell, RefCell};

const BASE: usize = 0x1000;
const PARENT: usize = 0x1000;
const VARIANTS: [usize; 2] = [0x2000, 0x2500];
const DEFINITIONS: [usize; 2] = [0x3000, 0x3800];
const BANK: usize = 0x4000;
const HEAP: usize = 0x5000;
const SHIP_ID: u64 = (42 << 32) | HANDLE_TYPE as u64;
const PARENT_ID: u64 = (17 << 32) | ctask_force::DEFAULT_HANDLE_TYPE as u64;

fn word(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn dword(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn inline(bytes: &mut [u8], at: usize, text: &[u8]) {
    bytes[at..at + text.len()].copy_from_slice(text);
    word(bytes, at + 16, text.len() as u64);
    word(bytes, at + 24, 15);
}

fn fixture() -> Vec<u8> {
    let mut bytes = vec![0; 0x6000];
    word(&mut bytes, 0, VTABLE_RVA.0 as u64);
    word(&mut bytes, HANDLE, SHIP_ID);
    word(&mut bytes, TASK_FORCE, (BASE + PARENT) as u64);
    word(&mut bytes, PARENT, ctask_force::VTABLE_RVA.0 as u64);
    word(&mut bytes, PARENT + cunit::HANDLE, PARENT_ID);
    word(
        &mut bytes,
        EQUIPMENT_POOL,
        cequipment_variant_pool::VTABLE_RVA.0 as u64,
    );
    word(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        (BASE + BANK) as u64,
    );
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::CAPACITY,
        2,
    );
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        2,
    );
    inline(&mut bytes, FALLBACK_SPRITE, b"destroyer");
    word(&mut bytes, FORMATION_FLAGS, u64::MAX);
    dword(&mut bytes, MODEL_NUMBER, i32::MIN as u32);
    for index in 0..2 {
        let variant = VARIANTS[index];
        let definition = DEFINITIONS[index];
        word(&mut bytes, variant, engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64);
        word(&mut bytes, variant + 8, ((index as u64 + 1) << 32) | 70);
        dword(
            &mut bytes,
            variant + cequipment_variant::EQUIPMENT_TOKEN,
            index as u32 + 11,
        );
        word(
            &mut bytes,
            variant + cequipment_variant::DEFINITION,
            (BASE + definition) as u64,
        );
        word(&mut bytes, definition, cequipment_type::VTABLE_RVA.0 as u64);
        dword(
            &mut bytes,
            definition + cequipment_type::TOKEN,
            index as u32 + 11,
        );
        inline(
            &mut bytes,
            definition + cequipment_type::HULL_SPRITE,
            &[b'A' + index as u8],
        );
        word(
            &mut bytes,
            BANK + index * equipment_entry::STRIDE,
            (BASE + variant) as u64,
        );
        word(
            &mut bytes,
            BANK + index * equipment_entry::STRIDE + 8,
            i64::MIN as u64,
        );
    }
    bytes
}

fn model(bytes: &[u8]) -> Result<ModelRow, &'static str> {
    read_model(
        &SliceMemory { base: BASE, bytes },
        0,
        BASE,
        &Row {
            id: SHIP_ID,
            task_force: PARENT_ID,
            ..Row::default()
        },
        &mut ModelMemo::default(),
    )
}

#[test]
fn model_selects_first_signed_maximum_and_preserves_bits_and_native_bytes() {
    let mut bytes = fixture();
    let row = model(&bytes).unwrap();
    assert_eq!(row.ship, SHIP_ID);
    assert_eq!(row.variant, (1 << 32) | 70);
    assert_eq!(row.hull_sprite.as_bytes(), b"A");
    assert_eq!(row.fallback_sprite.as_bytes(), b"destroyer");
    assert_eq!(row.model_number, i32::MIN);
    assert_eq!(row.formation_flags, u64::MAX);
    word(&mut bytes, BANK + 8, -1_i64 as u64);
    word(&mut bytes, BANK + equipment_entry::STRIDE + 8, 0);
    inline(
        &mut bytes,
        DEFINITIONS[1] + cequipment_type::HULL_SPRITE,
        &[0xff, 0, b'X'],
    );
    let row = model(&bytes).unwrap();
    assert_eq!(row.variant, (2 << 32) | 70);
    assert_eq!(row.hull_sprite.as_bytes(), &[0xff, 0, b'X']);
    word(&mut bytes, BANK + 8, i64::MAX as u64);
    assert_eq!(model(&bytes).unwrap().variant, (1 << 32) | 70);
}

#[test]
fn empty_and_selected_null_pools_preserve_no_variant_without_invented_hull() {
    let mut bytes = fixture();
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        0,
    );
    word(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        0,
    );
    let row = model(&bytes).unwrap();
    assert_eq!(row.variant, 0);
    assert!(row.hull_sprite.as_bytes().is_empty());
    assert_eq!(row.fallback_sprite.as_bytes(), b"destroyer");
    let mut bytes = fixture();
    word(&mut bytes, BANK, 0);
    word(&mut bytes, BANK + 8, i64::MAX as u64);
    assert_eq!(
        model(&bytes),
        Err("positive ship equipment has null variant")
    );
    word(&mut bytes, BANK + 8, 0);
    assert_eq!(model(&bytes).unwrap().variant, 0);
}

#[test]
fn counter_flags_include_all_positive_variants_and_preserve_raw_maximum_and_airbase() {
    let mut bytes = fixture();
    word(&mut bytes, BANK + 8, 5);
    word(&mut bytes, BANK + 24, 1);
    word(
        &mut bytes,
        VARIANTS[0] + cequipment_variant::CLASSIFIER_FLAGS,
        1 << 6,
    );
    word(
        &mut bytes,
        VARIANTS[1] + cequipment_variant::CLASSIFIER_FLAGS,
        (1 << 37) | (1 << 63),
    );
    bytes[DEFINITION_VALID] = 255;
    word(&mut bytes, MAXIMUM_STRENGTH, i64::MIN as u64);
    word(&mut bytes, CARRIER_AIR_BASE, (BASE + 0x5800) as u64);
    word(&mut bytes, ANTI_AIR_BASE, i64::MIN as u64);
    word(&mut bytes, 0x5800, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
    word(&mut bytes, 0x5808, (7 << 32) | 65);
    word(&mut bytes, 0x5860, SHIP_ID);
    let row = model(&bytes).unwrap();
    assert_eq!(row.variant, (1 << 32) | 70);
    assert_eq!(
        row.positive_equipment_flags,
        (1 << 6) | (1 << 37) | (1 << 63)
    );
    assert_eq!(row.definition_valid, 255);
    assert_eq!(row.maximum_strength, i64::MIN);
    assert_eq!(row.anti_air_base, i64::MIN);
    assert_eq!(row.carrier_air_base, (7 << 32) | 65);
    word(&mut bytes, BANK + 24, 0);
    assert_eq!(model(&bytes).unwrap().positive_equipment_flags, 1 << 6);
    word(&mut bytes, 0x5860, 0);
    assert!(model(&bytes).is_err());
}

#[test]
fn model_rejects_changed_census_identity_bad_pool_and_mismatched_variant_definition() {
    for at in [
        0,
        HANDLE,
        TASK_FORCE,
        PARENT,
        PARENT + cunit::HANDLE,
        EQUIPMENT_POOL,
        VARIANTS[0],
        VARIANTS[0] + cequipment_variant::TYPE,
        VARIANTS[0] + cequipment_variant::DEFINITION,
        DEFINITIONS[0],
        DEFINITIONS[0] + cequipment_type::TOKEN,
    ] {
        let mut bytes = fixture();
        bytes[at] ^= 1;
        assert!(model(&bytes).is_err(), "bad field at {at:x}");
    }
    let mut bytes = fixture();
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        3,
    );
    assert!(model(&bytes).is_err());
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        u32::MAX,
    );
    assert!(model(&bytes).is_err());
}

#[test]
fn model_strings_reject_corruption_and_unreadable_bytes_and_grow_without_truncation() {
    let mut bytes = fixture();
    word(&mut bytes, FALLBACK_SPRITE + 16, 16);
    assert!(model(&bytes).is_err());
    let mut bytes = fixture();
    word(&mut bytes, FALLBACK_SPRITE, (BASE + HEAP) as u64);
    word(&mut bytes, FALLBACK_SPRITE + 16, MODEL_TEXT_CAPACITY as u64);
    word(&mut bytes, FALLBACK_SPRITE + 24, 511);
    bytes[HEAP..HEAP + MODEL_TEXT_CAPACITY].fill(b'X');
    assert_eq!(
        model(&bytes).unwrap().fallback_sprite.as_bytes(),
        &[b'X'; MODEL_TEXT_CAPACITY]
    );
    word(
        &mut bytes,
        FALLBACK_SPRITE + 16,
        MODEL_TEXT_CAPACITY as u64 + 1,
    );
    bytes[HEAP + MODEL_TEXT_CAPACITY] = b'Y';
    let mut long = vec![b'X'; MODEL_TEXT_CAPACITY];
    long.push(b'Y');
    assert_eq!(model(&bytes).unwrap().fallback_sprite.as_bytes(), long);
    word(&mut bytes, FALLBACK_SPRITE + 16, MODEL_TEXT_CAPACITY as u64);
    bytes[HEAP + MODEL_TEXT_CAPACITY] = 1;
    assert!(model(&bytes).is_err());
    bytes[HEAP + MODEL_TEXT_CAPACITY] = 0;
    assert!(model(&bytes[..HEAP + MODEL_TEXT_CAPACITY]).is_err());
    word(&mut bytes, FALLBACK_SPRITE, 0);
    assert!(model(&bytes).is_err());
}

#[test]
fn pool_selection_crosses_bounded_batch_boundary_and_checks_all_amounts() {
    let mut bytes = fixture();
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        65,
    );
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::CAPACITY,
        65,
    );
    for index in 0..65 {
        word(
            &mut bytes,
            BANK + index * equipment_entry::STRIDE,
            (BASE + VARIANTS[0]) as u64,
        );
        word(
            &mut bytes,
            BANK + index * equipment_entry::STRIDE + 8,
            index as u64,
        );
    }
    word(
        &mut bytes,
        BANK + 64 * equipment_entry::STRIDE,
        (BASE + VARIANTS[1]) as u64,
    );
    assert_eq!(model(&bytes).unwrap().variant, (2 << 32) | 70);
    assert!(model(&bytes[..BANK + 65 * equipment_entry::STRIDE - 1]).is_err());
}

#[test]
fn mod_sized_model_pool_has_no_chosen_limit_and_preserves_original_read_faults() {
    let mut bytes = fixture();
    for count in [5_000, 12_001] {
        bytes.resize(BANK + count * equipment_entry::STRIDE, 0);
        dword(
            &mut bytes,
            EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
            count as u32,
        );
        dword(
            &mut bytes,
            EQUIPMENT_POOL + cequipment_variant_pool::CAPACITY,
            count as u32,
        );
        for slot in 0..count {
            word(
                &mut bytes,
                BANK + slot * equipment_entry::STRIDE,
                (BASE + VARIANTS[0]) as u64,
            );
            word(
                &mut bytes,
                BANK + slot * equipment_entry::STRIDE + 8,
                slot as u64,
            );
        }
        word(
            &mut bytes,
            BANK + (count - 1) * equipment_entry::STRIDE,
            (BASE + VARIANTS[1]) as u64,
        );
        assert_eq!(model(&bytes).unwrap().variant, (2 << 32) | 70);
        assert_eq!(
            model(&bytes[..BANK]),
            Err("equipment model pool entries unreadable")
        );
    }
}

struct Scalar<'a, M>(&'a M);

impl<M: Memory> Memory for Scalar<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }
}

struct Trace<'a, M> {
    source: &'a M,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<(usize, usize)>,
    change: Option<(usize, usize, u8)>,
    prefetched: Cell<usize>,
}

impl<'a, M> Trace<'a, M> {
    fn new(source: &'a M) -> Self {
        Self {
            source,
            reads: RefCell::new(Vec::new()),
            fault: None,
            change: None,
            prefetched: Cell::new(0),
        }
    }
}

impl<M: Memory> Memory for Trace<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        if let Some((failed, partial)) = self.fault
            && ordinal == failed
        {
            let len = partial.min(output.len());
            if len != 0 {
                self.source.copy(address, &mut output[..len]);
            }
            return false;
        }
        if !self.source.copy(address, output) {
            return false;
        }
        if let Some((changed, byte, xor)) = self.change
            && ordinal == changed
        {
            output[byte] ^= xor;
        }
        true
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        Scalar(self).copy_scattered(addresses, destinations)
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.prefetched.set(self.prefetched.get() + 1);
        self.copy_scattered(addresses, destinations)
    }
}

#[test]
fn subunit_index_preserves_signed_words_and_authenticates_the_embedded_copy() {
    for (token, valid, index) in [
        (0, 1, i32::MIN),
        (0, 1, -1),
        (0x165, 0, 0),
        (0, 1, 17),
        (u32::MAX, 255, i32::MAX),
    ] {
        let mut bytes = fixture();
        word(
            &mut bytes,
            SUBUNIT_DEFINITION,
            engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64,
        );
        dword(&mut bytes, CLASS_DATABASE_ID, token);
        bytes[DEFINITION_VALID] = valid;
        dword(&mut bytes, SUBUNIT_INDEX, index as u32);
        let source = Row {
            class_id: token as i32,
            ..Default::default()
        };
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let trace = Trace::new(&memory);
        assert_eq!(
            read_model_subunit_index(&trace, 0, BASE, &source),
            Ok(index)
        );
        assert_eq!(
            *trace.reads.borrow(),
            [(BASE + SUBUNIT_DEFINITION, 12), (BASE + SUBUNIT_INDEX, 4)]
        );
        assert_eq!(model(&bytes).unwrap().subunit_index, None);
        for at in [SUBUNIT_DEFINITION, CLASS_DATABASE_ID] {
            bytes[at] ^= 1;
            let memory = SliceMemory {
                base: BASE,
                bytes: &bytes,
            };
            let trace = Trace::new(&memory);
            assert_eq!(
                read_model_subunit_index(&trace, 0, BASE, &source),
                Err("ship subunit identity differs from census")
            );
            assert_eq!(trace.reads.borrow().len(), 1);
            bytes[at] ^= 1;
        }
    }
}

#[test]
fn subunit_index_guard_faults_never_retry_or_return_partial_words() {
    let mut bytes = fixture();
    word(
        &mut bytes,
        SUBUNIT_DEFINITION,
        engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64,
    );
    dword(&mut bytes, SUBUNIT_INDEX, i32::MIN as u32);
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let source = Row::default();
    for (field, len, reason) in [
        (0, 12, "ship subunit identity unreadable"),
        (1, 4, "ship subunit index unreadable"),
    ] {
        for partial in 0..=len {
            let mut trace = Trace::new(&memory);
            trace.fault = Some((field, partial));
            assert_eq!(
                read_model_subunit_index(&trace, 0, BASE, &source),
                Err(reason)
            );
            assert_eq!(trace.reads.borrow().len(), field + 1);
        }
    }
    let trace = Trace::new(&memory);
    assert_eq!(
        read_model_subunit_index(&trace, 0, 0, &source),
        Err("ship subunit address overflow")
    );
    assert_eq!(
        read_model_subunit_index(&trace, 0, usize::MAX - SUBUNIT_DEFINITION + 1, &source),
        Err("ship subunit address overflow")
    );
    assert!(trace.reads.borrow().is_empty());
    let trace = Trace::new(&memory);
    assert_eq!(
        read_model_subunit_index(&trace, usize::MAX, BASE, &source),
        Err("ship subunit module overflow")
    );
    assert_eq!(trace.reads.borrow().len(), 1);
}

struct CounterSource(usize);

impl Memory for CounterSource {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        for (offset, bytes) in [
            (DEFINITION_VALID, [255; 8]),
            (MAXIMUM_STRENGTH, i64::MIN.to_le_bytes()),
            (CARRIER_AIR_BASE, u64::MAX.to_le_bytes()),
            (ANTI_AIR_BASE, (i64::MIN + 1).to_le_bytes()),
        ] {
            if self.0.checked_add(offset) == Some(address) {
                output.copy_from_slice(&bytes[..output.len()]);
                return true;
            }
        }
        panic!("unexpected counter source {address:#x}");
    }
}

fn scalar_model_counters(
    memory: &impl Memory,
    pointer: usize,
) -> Result<(u8, i64, u64, i64), &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("ship model address overflow")
    };
    let valid =
        read(memory, at(DEFINITION_VALID)?).ok_or("ship definition-valid byte unreadable")?;
    let strength = read(memory, at(MAXIMUM_STRENGTH)?).ok_or("ship maximum strength unreadable")?;
    let air = read(memory, at(CARRIER_AIR_BASE)?).ok_or("ship carrier air base unreadable")?;
    let anti_air = read(memory, at(ANTI_AIR_BASE)?).ok_or("ship anti-air base unreadable")?;
    Ok((valid, strength, air, anti_air))
}

#[test]
fn model_counter_gather_matches_scalar_faults_partial_copies_and_address_overflow() {
    let faults: Vec<_> = std::iter::once(None)
        .chain(
            [1, 8, 8, 8]
                .into_iter()
                .enumerate()
                .flat_map(|(field, len)| (0..=len).map(move |partial| Some((field, partial)))),
        )
        .collect();
    for pointer in [
        0,
        BASE,
        usize::MAX - DEFINITION_VALID + 1,
        usize::MAX - DEFINITION_VALID,
        usize::MAX - MAXIMUM_STRENGTH + 1,
        usize::MAX - MAXIMUM_STRENGTH,
        usize::MAX - CARRIER_AIR_BASE + 1,
        usize::MAX - CARRIER_AIR_BASE,
        usize::MAX - CARRIER_AIR_BASE - 7,
        usize::MAX - CARRIER_AIR_BASE - 8,
        usize::MAX - ANTI_AIR_BASE + 1,
        usize::MAX - ANTI_AIR_BASE,
    ] {
        let source = CounterSource(pointer);
        for &fault in &faults {
            let mut scalar = Trace::new(&source);
            scalar.fault = fault;
            let mut gathered = Trace::new(&source);
            gathered.fault = fault;
            assert_eq!(
                read_model_counters(&gathered, pointer),
                scalar_model_counters(&scalar, pointer),
                "pointer {pointer:#x}, fault {fault:?}"
            );
            assert_eq!(
                *gathered.reads.borrow(),
                *scalar.reads.borrow(),
                "pointer {pointer:#x}, fault {fault:?}"
            );
        }
    }
}

#[test]
fn model_counter_gather_retains_four_observations_and_twenty_five_bytes() {
    let source = CounterSource(BASE);
    let census = engine_native_memory::read_census::ReadCensus::new(&source);
    let expected = Ok((255, i64::MIN, u64::MAX, i64::MIN + 1));
    assert_eq!(scalar_model_counters(&census, BASE), expected);
    assert_eq!((census.calls(), census.spans(), census.bytes()), (4, 4, 25));
    census.reset();
    assert_eq!(read_model_counters(&census, BASE), expected);
    assert_eq!((census.calls(), census.spans(), census.bytes()), (1, 4, 25));
}

#[test]
fn organisation_pairs_keep_aliases_partial_faults_and_lazy_invalid_address_order() {
    struct Source;
    impl Memory for Source {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            bytes.fill(address as u8);
            true
        }
    }
    fn check<const A: usize, const B: usize>() {
        use crate::cnaval_combatant::field;
        for (first, second) in [
            ((BASE, 8), (BASE, 0)),
            ((BASE, 8), (BASE, 8)),
            ((0, 8), (BASE, 0)),
            ((BASE, 8), (0, 0)),
            ((usize::MAX - 4, 8), (BASE, 0)),
            ((BASE, 8), (usize::MAX - 4, 8)),
            ((usize::MAX - A + 1, 0), (BASE, 0)),
            ((BASE, 8), (usize::MAX - B + 1, 0)),
        ] {
            for fault in std::iter::once(None).chain(
                [A, B]
                    .into_iter()
                    .enumerate()
                    .flat_map(|(field, len)| (0..=len).map(move |partial| Some((field, partial)))),
            ) {
                let mut scalar = Trace::new(&Source);
                scalar.fault = fault;
                let expected: Result<([u8; A], [u8; B]), &'static str> = (|| {
                    Ok((
                        field::<[u8; A]>(&scalar, first.0, first.1)?,
                        field::<[u8; B]>(&scalar, second.0, second.1)?,
                    ))
                })();
                let mut gathered = Trace::new(&Source);
                gathered.fault = fault;
                assert_eq!(
                    organisation_pair::<A, B>(&gathered, first, second),
                    expected
                );
                assert_eq!(*gathered.reads.borrow(), *scalar.reads.borrow());
            }
        }
        let census = engine_native_memory::read_census::ReadCensus::new(&Source);
        assert!(organisation_pair::<A, B>(&census, (BASE, 8), (BASE, 0)).is_ok());
        assert_eq!(
            (census.calls(), census.spans(), census.bytes()),
            (1, 2, A + B)
        );
    }
    check::<8, 4>();
    check::<8, 8>();
    check::<4, 8>();
}

const AIR: usize = 0x5800;

fn full_model_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    word(&mut bytes, BANK + 8, 5);
    word(&mut bytes, BANK + 24, 1);
    word(
        &mut bytes,
        VARIANTS[0] + cequipment_variant::CLASSIFIER_FLAGS,
        1 << 6,
    );
    word(
        &mut bytes,
        VARIANTS[1] + cequipment_variant::CLASSIFIER_FLAGS,
        1 << 37,
    );
    bytes[DEFINITION_VALID] = 255;
    word(&mut bytes, MAXIMUM_STRENGTH, i64::MIN as u64);
    word(&mut bytes, CARRIER_AIR_BASE, (BASE + AIR) as u64);
    word(&mut bytes, AIR, engine_native_parts::cair_base_layout::VTABLE_RVA.0 as u64);
    word(&mut bytes, AIR + engine_native_parts::cair_base_layout::HANDLE, (7 << 32) | 65);
    word(&mut bytes, AIR + engine_native_parts::cair_base_layout::HOST_REFERENCE, SHIP_ID);
    bytes
}

fn traced_model(memory: &impl Memory) -> Result<ModelRow, &'static str> {
    traced_model_memo(memory, &mut ModelMemo::default())
}

fn traced_model_memo(memory: &impl Memory, memo: &mut ModelMemo) -> Result<ModelRow, &'static str> {
    read_model(
        memory,
        0,
        BASE,
        &Row {
            id: SHIP_ID,
            task_force: PARENT_ID,
            ..Row::default()
        },
        memo,
    )
}

fn full_model_reads() -> Vec<(usize, usize, &'static str)> {
    let ship = "ship model fields unreadable";
    let parent = "ship model task force identity unreadable";
    let variant = "ship model variant fields unreadable";
    [
        (0, 16, ship),
        (TASK_FORCE, 8, ship),
        (FORMATION_FLAGS, 8, ship),
        (MODEL_NUMBER, 4, ship),
        (PARENT, 8, parent),
        (PARENT + cunit::HANDLE, 8, parent),
        (
            EQUIPMENT_POOL,
            8,
            "equipment pool vtable unreadable or mismatch",
        ),
        (
            EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
            16,
            "equipment pool array unreadable",
        ),
        (BANK, 32, "equipment model pool entries unreadable"),
        (
            VARIANTS[0],
            16,
            "ship equipment variant identity unreadable",
        ),
        (
            VARIANTS[0] + cequipment_variant::CLASSIFIER_FLAGS,
            8,
            "ship equipment flags unreadable",
        ),
        (
            VARIANTS[1],
            16,
            "ship equipment variant identity unreadable",
        ),
        (
            VARIANTS[1] + cequipment_variant::CLASSIFIER_FLAGS,
            8,
            "ship equipment flags unreadable",
        ),
        (FALLBACK_SPRITE, 32, "string header unreadable"),
        (DEFINITION_VALID, 1, "ship definition-valid byte unreadable"),
        (MAXIMUM_STRENGTH, 8, "ship maximum strength unreadable"),
        (CARRIER_AIR_BASE, 8, "ship carrier air base unreadable"),
        (ANTI_AIR_BASE, 8, "ship anti-air base unreadable"),
        (AIR, 8, "ship carrier air base identity invalid"),
        (
            AIR + engine_native_parts::cair_base_layout::HANDLE,
            8,
            "ship air base handle unreadable",
        ),
        (
            AIR + engine_native_parts::cair_base_layout::HOST_REFERENCE,
            8,
            "ship carrier air base has null handle",
        ),
        (VARIANTS[0], 16, variant),
        (
            VARIANTS[0] + cequipment_variant::EQUIPMENT_TOKEN,
            4,
            variant,
        ),
        (VARIANTS[0] + cequipment_variant::DEFINITION, 8, variant),
        (
            DEFINITIONS[0],
            12,
            "ship model equipment definition identity unreadable",
        ),
        (
            DEFINITIONS[0] + cequipment_type::HULL_SPRITE,
            32,
            "string header unreadable",
        ),
    ]
    .map(|(offset, len, reason)| (BASE + offset, len, reason))
    .to_vec()
}

#[test]
fn model_hull_reuse_retains_exact_bytes_and_bounded_native_savings() {
    for (heap, len, extra_calls, extra_bytes) in
        [(false, 3, 0, 0), (true, 0, 1, 1), (true, 256, 2, 257)]
    {
        let mut bytes = full_model_fixture();
        let sprite = DEFINITIONS[0] + cequipment_type::HULL_SPRITE;
        bytes[sprite..sprite + 32].fill(0);
        let payload: Vec<u8> = (0..len).map(|i| [0xff, 0, b'X'][i % 3]).collect();
        if heap {
            word(&mut bytes, sprite, (BASE + HEAP) as u64);
            word(&mut bytes, sprite + 16, len as u64);
            word(&mut bytes, sprite + 24, 511);
            bytes[HEAP..HEAP + len].copy_from_slice(&payload);
        } else {
            inline(&mut bytes, sprite, &payload);
        }
        let source = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let census = engine_native_memory::read_census::ReadCensus::new(&source);
        let mut memo = ModelMemo::default();
        let cold = traced_model_memo(&census, &mut memo).unwrap();
        assert_eq!(cold.hull_sprite.as_bytes(), payload);
        assert_eq!(memo.hulls.len(), 1);
        let cached_sprite = &memo.hulls.values().next().unwrap().sprite;
        assert_eq!(cached_sprite.as_bytes(), payload);
        assert_eq!(cached_sprite.allocated_bytes(), 0);
        assert_eq!(
            (census.calls(), census.spans(), census.bytes()),
            (17 + extra_calls, 26 + extra_calls, 309 + extra_bytes)
        );
        census.reset();
        assert_eq!(traced_model_memo(&census, &mut memo), Ok(cold.clone()));
        assert_eq!(
            (census.calls(), census.spans(), census.bytes()),
            (15, 24, 265)
        );
        let reads: Vec<_> = full_model_reads()[..24]
            .iter()
            .map(|&(a, n, _)| (a, n))
            .collect();
        let trace = Trace::new(&source);
        assert_eq!(traced_model_memo(&trace, &mut memo), Ok(cold));
        assert_eq!(*trace.reads.borrow(), reads);
        for (ordinal, &(_, len, reason)) in full_model_reads()[..24].iter().enumerate() {
            let mut trace = Trace::new(&source);
            trace.fault = Some((ordinal, len / 2));
            assert_eq!(traced_model_memo(&trace, &mut memo), Err(reason));
            assert_eq!(*trace.reads.borrow(), reads[..=ordinal]);
        }
    }
}

#[test]
fn model_hull_reuse_keeps_other_ship_and_selected_variant_inputs_fresh() {
    let mut bytes = full_model_fixture();
    let other = 0x6000;
    bytes.resize(0x7000, 0);
    bytes.copy_within(..SIZE, other);
    let other_id = SHIP_ID + (1 << 32);
    word(&mut bytes, other + HANDLE, other_id);
    word(&mut bytes, other + CARRIER_AIR_BASE, 0);
    inline(&mut bytes, other + FALLBACK_SPRITE, b"different");
    dword(&mut bytes, other + MODEL_NUMBER, 71);
    let mut memo = ModelMemo::default();
    assert!(
        traced_model_memo(
            &SliceMemory {
                base: BASE,
                bytes: &bytes
            },
            &mut memo
        )
        .is_ok()
    );

    word(&mut bytes, BANK + 24, 6);
    dword(
        &mut bytes,
        VARIANTS[1] + cequipment_variant::EQUIPMENT_TOKEN,
        11,
    );
    word(
        &mut bytes,
        VARIANTS[1] + cequipment_variant::DEFINITION,
        (BASE + DEFINITIONS[0]) as u64,
    );
    word(
        &mut bytes,
        VARIANTS[1] + cequipment_variant::CLASSIFIER_FLAGS,
        1 << 63,
    );
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let census = engine_native_memory::read_census::ReadCensus::new(&source);
    let row_source = Row {
        id: other_id,
        task_force: PARENT_ID,
        roster_ordinal: 9,
        ..Row::default()
    };
    let warm = read_model(&census, 0, BASE + other, &row_source, &mut memo).unwrap();
    let warm_counts = (census.calls(), census.spans(), census.bytes());
    census.reset();
    let cold = read_model(
        &census,
        0,
        BASE + other,
        &row_source,
        &mut ModelMemo::default(),
    )
    .unwrap();
    assert_eq!(warm, cold);
    assert_eq!(warm.variant, (2 << 32) | 70);
    assert_eq!(warm.fallback_sprite.as_bytes(), b"different");
    assert_eq!(warm.model_number, 71);
    assert_eq!(warm.roster_ordinal, 9);
    assert_eq!(warm.positive_equipment_flags, (1 << 6) | (1 << 63));
    assert_eq!(
        warm_counts,
        (census.calls() - 2, census.spans() - 2, census.bytes() - 44)
    );
}

#[test]
fn model_hull_changed_token_pointer_and_capture_reacquire_complete_definition() {
    let mut bytes = full_model_fixture();
    let mut memo = ModelMemo::default();
    assert!(
        traced_model_memo(
            &SliceMemory {
                base: BASE,
                bytes: &bytes
            },
            &mut memo
        )
        .is_ok()
    );
    dword(
        &mut bytes,
        VARIANTS[0] + cequipment_variant::EQUIPMENT_TOKEN,
        99,
    );
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let trace = Trace::new(&source);
    assert_eq!(
        traced_model_memo(&trace, &mut memo),
        Err("ship model equipment definition identity mismatch")
    );
    assert_eq!(trace.reads.borrow().len(), 25);
    assert_eq!(memo.hulls.len(), 1);
    assert!(memo.hulls.keys().all(|key| key.2 == 11_u32.to_le_bytes()));
    for (definition, reset, text) in [
        (DEFINITIONS[0], false, b'B'),
        (DEFINITIONS[1], false, b'C'),
        (DEFINITIONS[1], true, b'D'),
    ] {
        word(
            &mut bytes,
            VARIANTS[0] + cequipment_variant::DEFINITION,
            (BASE + definition) as u64,
        );
        dword(&mut bytes, definition + cequipment_type::TOKEN, 99);
        inline(
            &mut bytes,
            definition + cequipment_type::HULL_SPRITE,
            &[text],
        );
        if reset {
            memo = ModelMemo::default();
        }
        let source = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let census = engine_native_memory::read_census::ReadCensus::new(&source);
        assert_eq!(
            traced_model_memo(&census, &mut memo)
                .unwrap()
                .hull_sprite
                .as_bytes(),
            &[text]
        );
        assert_eq!(
            (census.calls(), census.spans(), census.bytes()),
            (17, 26, 309)
        );
    }
}

#[test]
fn model_memo_reuses_every_completed_hull_definition_of_the_capture() {
    let mut bytes = full_model_fixture();
    let first = BASE + DEFINITIONS[0]..BASE + DEFINITIONS[1];
    let mut memo = ModelMemo::default();
    for (second, sprite) in [(false, b'A'), (true, b'B')] {
        word(&mut bytes, BANK + 24, if second { 6 } else { 1 });
        let source = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let row = traced_model_memo(&source, &mut memo).unwrap();
        assert_eq!(row.hull_sprite.as_bytes(), &[sprite]);
    }
    word(&mut bytes, BANK + 24, 1);
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let trace = Trace::new(&source);
    let row = traced_model_memo(&trace, &mut memo).unwrap();
    assert_eq!(row, traced_model(&source).unwrap());
    let definition_reads = trace
        .reads
        .borrow()
        .iter()
        .filter(|(address, _)| first.contains(address))
        .count();
    assert_eq!(definition_reads, 0);
    memo.reset();
    assert!(memo.hulls.is_empty());
}

#[test]
fn model_hull_failures_keep_original_prefix_and_never_publish_partial_text() {
    let mut bytes = full_model_fixture();
    let sprite = DEFINITIONS[0] + cequipment_type::HULL_SPRITE;
    word(&mut bytes, sprite, (BASE + HEAP) as u64);
    word(&mut bytes, sprite + 16, 17);
    word(&mut bytes, sprite + 24, 31);
    bytes[HEAP..HEAP + 17].fill(0xff);
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut expected = full_model_reads();
    expected.extend([
        (
            BASE + HEAP + 17,
            1,
            "string terminator unreadable or invalid",
        ),
        (BASE + HEAP, 17, "string bytes unreadable"),
    ]);
    let reads: Vec<_> = expected.iter().map(|&(a, n, _)| (a, n)).collect();
    for (ordinal, &(_, len, reason)) in expected.iter().enumerate().skip(24) {
        for partial in [0, len / 2, len] {
            let mut memo = ModelMemo::default();
            let mut trace = Trace::new(&source);
            trace.fault = Some((ordinal, partial));
            assert_eq!(traced_model_memo(&trace, &mut memo), Err(reason));
            assert_eq!(*trace.reads.borrow(), reads[..=ordinal]);
            assert!(memo.hulls.is_empty());
            let trace = Trace::new(&source);
            assert!(traced_model_memo(&trace, &mut memo).is_ok());
            assert_eq!(*trace.reads.borrow(), reads);
        }
    }
}

#[test]
fn model_primary_identity_gates_keep_the_first_read_prefix() {
    let bytes = full_model_fixture();
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let reads: Vec<_> = full_model_reads().iter().map(|&(a, n, _)| (a, n)).collect();
    for (ordinal, byte, xor, end, reason) in [
        (0, 0, 1, 4, "ship model identity differs from census"),
        (4, 0, 1, 6, "ship model task force differs from census"),
        (9, 0, 1, 10, "variant vtable or reference type mismatch"),
        (18, 0, 1, 19, "ship carrier air base identity invalid"),
        (19, 4, 7, 20, "ship carrier air base has null handle"),
        (19, 0, 1, 20, "ship carrier air base has null handle"),
        (20, 0, 1, 21, "ship carrier air base has null handle"),
        (21, 0, 1, 24, "variant vtable or reference type mismatch"),
        (
            24,
            0,
            1,
            25,
            "ship model equipment definition identity mismatch",
        ),
    ] {
        let mut memory = Trace::new(&source);
        memory.change = Some((ordinal, byte, xor));
        assert_eq!(traced_model(&memory), Err(reason), "invalid copy {ordinal}");
        assert_eq!(
            *memory.reads.borrow(),
            reads[..end],
            "invalid copy {ordinal}"
        );
    }
    let mut bytes = bytes;
    word(&mut bytes, CARRIER_AIR_BASE, 0);
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let memory = Trace::new(&source);
    assert_eq!(traced_model(&memory).unwrap().carrier_air_base, 0);
    let without_air: Vec<_> = reads
        .into_iter()
        .enumerate()
        .filter(|(index, _)| ![18, 19, 20].contains(index))
        .map(|(_, read)| read)
        .collect();
    assert_eq!(*memory.reads.borrow(), without_air);
}

#[test]
fn empty_selected_pool_has_no_companion_and_the_memo_keeps_current_selectors() {
    let mut bytes = fixture();
    let source = Row {
        id: SHIP_ID,
        task_force: PARENT_ID,
        ..Default::default()
    };
    let mut memo = ModelMemo::default();
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (first, selected) =
        read_model_with_selection(&memory, 0, BASE, &source, &mut memo).unwrap();
    assert_eq!(selected.unwrap().handle, first.variant);
    word(
        &mut bytes,
        VARIANTS[0] + cequipment_variant::TYPE,
        (9 << 32) | 70,
    );
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (current, selected) =
        read_model_with_selection(&memory, 0, BASE, &source, &mut memo).unwrap();
    let selected = selected.unwrap();
    assert_eq!(selected.handle, current.variant);
    assert_eq!(selected.handle, (9 << 32) | 70);
    assert_eq!(selected.definition, BASE + DEFINITIONS[0]);
    dword(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::COUNT,
        0,
    );
    word(
        &mut bytes,
        EQUIPMENT_POOL + cequipment_variant_pool::ITEMS,
        0,
    );
    let memory = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let (empty, selected) =
        read_model_with_selection(&memory, 0, BASE, &source, &mut memo).unwrap();
    assert_eq!(empty.variant, 0);
    assert!(selected.is_none());
}

struct NativeNullModelMemory<'a> {
    bytes: &'a [u8],
    singleton: u64,
}

impl Memory for NativeNullModelMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if address == hull_variant::NULL_DEFINITION_SINGLETON_RVA.0 && output.len() == 8 {
            output.copy_from_slice(&self.singleton.to_le_bytes());
            true
        } else {
            SliceMemory {
                base: BASE,
                bytes: self.bytes,
            }
            .copy(address, output)
        }
    }
}

fn native_null_model_fixture() -> Vec<u8> {
    let mut bytes = fixture();
    word(
        &mut bytes,
        DEFINITIONS[0],
        hull_variant::NULL_DEFINITION_VTABLE_RVA.0 as u64,
    );
    dword(
        &mut bytes,
        DEFINITIONS[0] + cequipment_type::TOKEN,
        hull_variant::NULL_DEFINITION_TOKEN,
    );
    dword(
        &mut bytes,
        VARIANTS[0] + cequipment_variant::EQUIPMENT_TOKEN,
        hull_variant::NULL_DEFINITION_TOKEN,
    );
    dword(
        &mut bytes,
        DEFINITIONS[0] + cequipment_type::ARCHETYPE_ID,
        (-1_i32) as u32,
    );
    bytes[DEFINITIONS[0] + cequipment_type::HULL_SPRITE
        ..DEFINITIONS[0] + cequipment_type::HULL_SPRITE + engine_native_memory::msvc_string::STRIDE]
        .fill(0);
    inline(
        &mut bytes,
        DEFINITIONS[0] + cequipment_type::HULL_SPRITE,
        b"",
    );
    bytes
}

#[test]
fn registered_variant_null_type_keeps_handle_and_reads_its_actual_empty_sprite() {
    let bytes = native_null_model_fixture();
    let memory = NativeNullModelMemory {
        bytes: &bytes,
        singleton: (BASE + DEFINITIONS[0]) as u64,
    };
    let trace = Trace::new(&memory);
    let source = Row {
        id: SHIP_ID,
        task_force: PARENT_ID,
        ..Default::default()
    };
    let (row, selected) =
        read_model_with_selection(&trace, 0, BASE, &source, &mut ModelMemo::default()).unwrap();
    assert_eq!(
        row.variant,
        (1 << 32) | cequipment_variant::REFERENCE_TYPE as u64
    );
    assert_eq!(row.hull_sprite.as_bytes(), b"");
    assert_eq!(row.fallback_sprite.as_bytes(), b"destroyer");
    assert_eq!(selected.unwrap().handle, row.variant);
    let spans = trace.reads.into_inner();
    let singleton = spans
        .iter()
        .position(|&(at, _)| at == hull_variant::NULL_DEFINITION_SINGLETON_RVA.0)
        .unwrap();
    assert_eq!(
        spans[singleton + 1],
        (BASE + DEFINITIONS[0] + cequipment_type::ACTIVE, 1)
    );
    assert_eq!(
        spans[singleton + 2],
        (
            BASE + DEFINITIONS[0] + cequipment_type::HULL_SPRITE,
            engine_native_memory::msvc_string::STRIDE
        )
    );
    for ordinal in singleton..spans.len() {
        let mut trace = Trace::new(&memory);
        trace.fault = Some((ordinal, 0));
        assert!(
            read_model_with_selection(&trace, 0, BASE, &source, &mut ModelMemo::default()).is_err()
        );
        assert_eq!(*trace.reads.borrow(), spans[..=ordinal]);
    }
}

#[test]
fn null_type_impostors_and_unregistered_variants_do_not_widen_model_admission() {
    for change in 0..4 {
        let mut bytes = native_null_model_fixture();
        let mut singleton = (BASE + DEFINITIONS[0]) as u64;
        match change {
            0 => singleton += 8,
            1 => bytes[DEFINITIONS[0] + cequipment_type::ACTIVE] = 1,
            2 => dword(&mut bytes, DEFINITIONS[0] + cequipment_type::TOKEN, 0x166),
            3 => word(
                &mut bytes,
                VARIANTS[0] + 8,
                cequipment_variant::REFERENCE_TYPE as u64,
            ),
            _ => unreachable!(),
        }
        let memory = NativeNullModelMemory {
            bytes: &bytes,
            singleton,
        };
        assert_eq!(
            read_model(
                &memory,
                0,
                BASE,
                &Row {
                    id: SHIP_ID,
                    task_force: PARENT_ID,
                    ..Default::default()
                },
                &mut ModelMemo::default()
            )
            .unwrap_err(),
            "ship model equipment definition identity mismatch"
        );
    }
}

#[test]
fn a_warm_null_sprite_memo_still_requires_each_selected_variant_to_be_registered() {
    let mut bytes = native_null_model_fixture();
    let source = Row {
        id: SHIP_ID,
        task_force: PARENT_ID,
        ..Default::default()
    };
    let mut memo = ModelMemo::default();
    let memory = NativeNullModelMemory {
        bytes: &bytes,
        singleton: (BASE + DEFINITIONS[0]) as u64,
    };
    assert!(read_model_with_selection(&memory, 0, BASE, &source, &mut memo).is_ok());
    word(
        &mut bytes,
        VARIANTS[0] + 8,
        cequipment_variant::REFERENCE_TYPE as u64,
    );
    let memory = NativeNullModelMemory {
        bytes: &bytes,
        singleton: (BASE + DEFINITIONS[0]) as u64,
    };
    let trace = Trace::new(&memory);
    assert_eq!(
        read_model_with_selection(&trace, 0, BASE, &source, &mut memo).err(),
        Some("ship model equipment definition identity mismatch")
    );
    assert!(
        !trace
            .reads
            .borrow()
            .iter()
            .any(|&(at, _)| at == hull_variant::NULL_DEFINITION_SINGLETON_RVA.0)
    );
}

#[test]
fn model_text_keeps_mod_sized_sprites_and_original_hull_memo() {
    let mut bytes = fixture();
    word(&mut bytes, BANK + 8, 1);
    word(&mut bytes, BANK + equipment_entry::STRIDE + 8, 0);
    let (fallback, hull) = (0x10000, 0x30000);
    let source = Row {
        id: SHIP_ID,
        task_force: PARENT_ID,
        ..Row::default()
    };
    for (length, occurrences) in [(8_193, 5_000), (32_769, 1)] {
        bytes.resize(hull + length + 1, 0);
        for (descriptor, payload, byte) in [
            (FALLBACK_SPRITE, fallback, 0xff),
            (DEFINITIONS[0] + cequipment_type::HULL_SPRITE, hull, b'X'),
        ] {
            word(&mut bytes, descriptor, (BASE + payload) as u64);
            word(&mut bytes, descriptor + 16, length as u64);
            word(&mut bytes, descriptor + 24, length as u64);
            bytes[payload..payload + length].fill(byte);
        }
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut memo = ModelMemo::default();
        for _ in 0..occurrences {
            let row = read_model(&memory, 0, BASE, &source, &mut memo).unwrap();
            assert_eq!(row.fallback_sprite.as_bytes(), vec![0xff; length]);
            assert_eq!(row.hull_sprite.as_bytes(), vec![b'X'; length]);
        }
    }
}

#[test]
fn gathered_model_reads_equal_the_direct_reader_at_every_fault() {
    use super::gathered::FaultAt;
    use engine_native_memory::read_census::ReadCensus;
    fn through_fields(memory: &impl Memory) -> Result<ModelRow, &'static str> {
        let mut fields = ShipFields::new(0);
        fields.visit(memory, BASE);
        traced_model(&fields.memory(memory))
    }
    let bytes = full_model_fixture();
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let direct = FaultAt::new(&source, None);
    let healthy = traced_model(&direct);
    assert!(healthy.is_ok());
    let gathered = FaultAt::new(&source, None);
    assert_eq!(through_fields(&gathered), healthy);
    let mut edges = direct.edges();
    edges.extend(gathered.edges());
    edges.sort_unstable();
    edges.dedup();
    for fault in edges {
        let direct = FaultAt::new(&source, Some(fault));
        let gathered = FaultAt::new(&source, Some(fault));
        assert_eq!(
            through_fields(&gathered),
            traced_model(&direct),
            "fault at {fault:#x}"
        );
        assert!(!gathered.retried(), "fault at {fault:#x}");
    }
    let direct = ReadCensus::new(&source);
    assert_eq!(traced_model(&direct), healthy);
    let gathered = ReadCensus::new(&source);
    assert_eq!(through_fields(&gathered), healthy);
    assert_eq!((direct.calls(), gathered.calls()), (17, 11));
    let census = ReadCensus::new(&source);
    let mut fields = ShipFields::new(0);
    fields.visit(&census, BASE);
    census.reset();
    fields.visit(&census, BASE);
    assert_eq!(census.calls(), 2, "the ship and its pool entries");
    assert_eq!(traced_model(&fields.memory(&census)), healthy);
}

#[test]
fn held_ships_serve_their_own_objects_without_reading_them() {
    use engine_native_memory::read_census::ReadCensus;
    let bytes = full_model_fixture();
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let mut fields = ShipFields::new(0);
    fields.visit(&source, BASE);
    let healthy = traced_model(&fields.memory(&source));
    assert!(healthy.is_ok());
    let held = fields.held();
    let census = ReadCensus::new(&source);
    let mut fields = ShipFields::new(0);
    fields.visit_held(&census, &held);
    assert_eq!(
        census.calls(),
        4,
        "the game singleton, the task force and the two variants"
    );
    assert_eq!(traced_model(&fields.memory(&census)), healthy);
    assert_eq!(held.task_force(), BASE + PARENT);
    let mut again = ShipFields::new(0);
    again.visit(&source, BASE);
    assert!(again.held().same(&held));
    let mut changed = bytes.clone();
    dword(&mut changed, MODEL_NUMBER, 7);
    let mut fresh = ShipFields::new(0);
    fresh.visit(
        &SliceMemory {
            base: BASE,
            bytes: &changed,
        },
        BASE,
    );
    assert!(!fresh.held().same(&held));
}

#[test]
fn gathered_heap_sprites_equal_the_direct_reader_at_every_fault() {
    use super::gathered::FaultAt;
    use engine_native_memory::read_census::ReadCensus;
    fn through_fields(memory: &impl Memory) -> Result<ModelRow, &'static str> {
        let mut fields = ShipFields::new(0);
        fields.visit(memory, BASE);
        traced_model(&fields.memory(memory))
    }
    let mut bytes = full_model_fixture();
    let text = b"a_heap_fallback_sprite_name";
    word(&mut bytes, FALLBACK_SPRITE, (BASE + HEAP) as u64);
    word(&mut bytes, FALLBACK_SPRITE + 16, text.len() as u64);
    word(&mut bytes, FALLBACK_SPRITE + 24, 31);
    bytes[HEAP..HEAP + text.len()].copy_from_slice(text);
    bytes[HEAP + text.len()] = 0;
    let source = SliceMemory {
        base: BASE,
        bytes: &bytes,
    };
    let direct = FaultAt::new(&source, None);
    let healthy = traced_model(&direct);
    assert_eq!(
        healthy
            .as_ref()
            .map(|row| row.fallback_sprite.as_bytes().to_vec()),
        Ok(text.to_vec())
    );
    let gathered = FaultAt::new(&source, None);
    assert_eq!(through_fields(&gathered), healthy);
    let mut edges = direct.edges();
    edges.extend(gathered.edges());
    edges.sort_unstable();
    edges.dedup();
    for fault in edges {
        let direct = FaultAt::new(&source, Some(fault));
        let gathered = FaultAt::new(&source, Some(fault));
        assert_eq!(
            through_fields(&gathered),
            traced_model(&direct),
            "fault at {fault:#x}"
        );
        assert!(!gathered.retried(), "fault at {fault:#x}");
    }
    let direct = ReadCensus::new(&source);
    assert_eq!(traced_model(&direct), healthy);
    let gathered = ReadCensus::new(&source);
    assert_eq!(through_fields(&gathered), healthy);
    assert_eq!((direct.calls(), gathered.calls()), (19, 11));
}
