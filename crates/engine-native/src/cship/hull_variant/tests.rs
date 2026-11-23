use super::*;
use std::cell::RefCell;

const BASE: usize = 0x10_0000;
const VARIANT: usize = 0x20_0000;
const DEFINITION: usize = 0x30_0000;
const DATABASE: usize = 0x40_0000;
const LIST: usize = 0x50_0000;
const HANDLE: u64 = (17 << 32) | cequipment_variant::REFERENCE_TYPE as u64;
const TOKEN: u32 = 0x4321;

struct World {
    regions: Vec<(usize, Vec<u8>)>,
    reads: RefCell<Vec<(usize, usize)>>,
    fault: Option<(usize, usize)>,
}

impl World {
    fn fixture() -> Self {
        let mut world = Self {
            regions: vec![
                (VARIANT, vec![0; 0x500]),
                (DEFINITION, vec![0; 0x500]),
                (DATABASE, vec![0; 0x80]),
                (LIST, vec![0; 16]),
                (BASE + cequipment_database::SINGLETON_RVA.0, vec![0; 8]),
            ],
            reads: RefCell::new(Vec::new()),
            fault: None,
        };
        world.put(
            VARIANT,
            &(BASE as u64 + engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        world.put(VARIANT + 8, &HANDLE.to_le_bytes());
        world.put(
            VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
            &TOKEN.to_le_bytes(),
        );
        world.put(
            VARIANT + cequipment_variant::DEFINITION,
            &(DEFINITION as u64).to_le_bytes(),
        );
        world.put(
            DEFINITION,
            &(BASE as u64 + cequipment_type::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        world.put(DEFINITION + cequipment_type::TOKEN, &TOKEN.to_le_bytes());
        world.put(DEFINITION + DEFINITION_INDEX, &1_i32.to_le_bytes());
        world.put(
            BASE + cequipment_database::SINGLETON_RVA.0,
            &(DATABASE as u64).to_le_bytes(),
        );
        world.put(
            DATABASE,
            &(BASE as u64 + cequipment_database::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        world.put(DATABASE + DEFINITIONS, &(LIST as u64).to_le_bytes());
        world.put(DATABASE + DEFINITIONS + 8, &2_u32.to_le_bytes());
        world.put(DATABASE + DEFINITIONS + 12, &2_u32.to_le_bytes());
        world.put(LIST + 8, &(DEFINITION as u64).to_le_bytes());
        world.put(VARIANT + MODULE_COUNT, &i32::MIN.to_le_bytes());
        world.put(VARIANT + UPGRADE_LEVEL_SUM, &(-1_i32).to_le_bytes());
        world.put(VARIANT + MODULE_PRIORITY_SUM, &i32::MAX.to_le_bytes());
        world.put(VARIANT + cequipment_variant::VERSION, &[u8::MAX]);
        world.put(VARIANT + ORGANISATION_MARKER, &i32::MIN.to_le_bytes());
        world
    }

    fn put(&mut self, address: usize, bytes: &[u8]) {
        let (base, region) = self
            .regions
            .iter_mut()
            .find(|(base, region)| {
                address >= *base && address - *base <= region.len().saturating_sub(bytes.len())
            })
            .unwrap();
        let start = address - *base;
        region[start..start + bytes.len()].copy_from_slice(bytes);
    }

    fn source(&self) -> SelectedModelVariant {
        SelectedModelVariant {
            pointer: VARIANT,
            handle: HANDLE,
            definition: DEFINITION,
            token: TOKEN.to_le_bytes(),
        }
    }
}

impl Memory for World {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ordinal = self.reads.borrow().len();
        self.reads.borrow_mut().push((address, output.len()));
        let source = self.regions.iter().find_map(|(base, region)| {
            let start = address.checked_sub(*base)?;
            region.get(start..start.checked_add(output.len())?)
        });
        let Some(source) = source else { return false };
        if let Some((failed, partial)) = self.fault
            && ordinal == failed
        {
            let len = partial.min(output.len());
            output[..len].copy_from_slice(&source[..len]);
            return false;
        }
        output.copy_from_slice(source);
        true
    }
}

fn expected() -> HullVariantInputs {
    HullVariantInputs {
        definition_index: Some(1),
        module_count: Some(i32::MIN),
        upgrade_level_sum: Some(-1),
        module_priority_sum: Some(i32::MAX),
        version: Some(u8::MAX),
        organisation_marker: Some(i32::MIN),
    }
}

#[test]
fn current_tuple_preserves_signed_words_unsigned_version_and_fixed_budget() {
    let world = World::fixture();
    assert_eq!(
        read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default()),
        Some(expected())
    );
    let reads = world.reads.borrow();
    assert_eq!(
        (reads.len(), reads.iter().map(|(_, len)| len).sum::<usize>()),
        (14, 101)
    );
    assert_eq!(reads.iter().filter(|(at, _)| *at == LIST + 8).count(), 1);
    assert!(!reads.iter().any(|(at, _)| *at == LIST));
}

#[test]
fn short_optional_spans_keep_other_terms_without_retry_or_partial_values() {
    let complete = World::fixture();
    read_hull_variant(
        &complete,
        BASE,
        &complete.source(),
        &mut ModelMemo::default(),
    )
    .unwrap();
    let spans = complete.reads.into_inner();
    for (ordinal, &(address, len)) in spans.iter().enumerate() {
        for partial in 0..=len {
            let mut world = World::fixture();
            world.fault = Some((ordinal, partial));
            let actual =
                read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default());
            if ordinal < 3 {
                assert_eq!(actual, None);
            } else {
                let mut wanted = expected();
                match ordinal {
                    3..=8 => wanted.definition_index = None,
                    9 => wanted.module_count = None,
                    10 => wanted.upgrade_level_sum = None,
                    11 => wanted.module_priority_sum = None,
                    12 => wanted.version = None,
                    13 => wanted.organisation_marker = None,
                    _ => unreachable!(),
                }
                assert_eq!(actual, Some(wanted), "span{ordinal}, partial{partial}");
            }
            assert_eq!(
                world
                    .reads
                    .borrow()
                    .iter()
                    .filter(|(at, _)| *at == address)
                    .count(),
                1
            );
            assert!(world.reads.borrow().len() <= 14);
        }
    }
}

#[test]
fn changed_full_handle_token_or_selected_definition_refuses_the_whole_packet() {
    for (address, bytes) in [
        (VARIANT, 0_u64.to_le_bytes().to_vec()),
        (VARIANT + 8, (HANDLE + (1 << 32)).to_le_bytes().to_vec()),
        (
            VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
            (TOKEN + 1).to_le_bytes().to_vec(),
        ),
        (
            VARIANT + cequipment_variant::DEFINITION,
            (DEFINITION as u64 + 8).to_le_bytes().to_vec(),
        ),
    ] {
        let mut world = World::fixture();
        world.put(address, &bytes);
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default()),
            None
        );
        assert!(world.reads.borrow().len() <= 3);
    }
}

#[test]
fn definition_alias_and_database_faults_withhold_only_the_ordinal() {
    for (address, bytes) in [
        (DEFINITION, 0_u64.to_le_bytes().to_vec()),
        (
            DEFINITION + cequipment_type::TOKEN,
            (TOKEN + 1).to_le_bytes().to_vec(),
        ),
        (DEFINITION + DEFINITION_INDEX, 0_i32.to_le_bytes().to_vec()),
        (
            DEFINITION + DEFINITION_INDEX,
            (-2_i32).to_le_bytes().to_vec(),
        ),
        (
            DEFINITION + DEFINITION_INDEX,
            i32::MAX.to_le_bytes().to_vec(),
        ),
        (
            BASE + cequipment_database::SINGLETON_RVA.0,
            0_u64.to_le_bytes().to_vec(),
        ),
        (DATABASE, 0_u64.to_le_bytes().to_vec()),
        (DATABASE + DEFINITIONS + 12, 3_u32.to_le_bytes().to_vec()),
        (LIST + 8, (DEFINITION as u64 + 8).to_le_bytes().to_vec()),
    ] {
        let mut world = World::fixture();
        world.put(address, &bytes);
        let mut wanted = expected();
        wanted.definition_index = None;
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default()),
            Some(wanted)
        );
    }
}

#[test]
fn null_ordinal_is_not_admitted_as_a_real_definition_and_other_terms_survive() {
    let mut world = World::fixture();
    world.put(DEFINITION + DEFINITION_INDEX, &(-1_i32).to_le_bytes());
    world.put(LIST, &(DEFINITION as u64).to_le_bytes());
    let mut wanted = expected();
    wanted.definition_index = None;
    assert_eq!(
        read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default()),
        Some(wanted)
    );
}

fn native_null_world() -> World {
    let mut world = World::fixture();
    world.regions.push((
        BASE + NULL_DEFINITION_SINGLETON_RVA.0,
        (DEFINITION as u64).to_le_bytes().to_vec(),
    ));
    world.put(
        DEFINITION,
        &(BASE as u64 + NULL_DEFINITION_VTABLE_RVA.0 as u64).to_le_bytes(),
    );
    world.put(
        DEFINITION + cequipment_type::TOKEN,
        &NULL_DEFINITION_TOKEN.to_le_bytes(),
    );
    world.put(
        VARIANT + cequipment_variant::EQUIPMENT_TOKEN,
        &NULL_DEFINITION_TOKEN.to_le_bytes(),
    );
    world.put(DEFINITION + DEFINITION_INDEX, &(-1_i32).to_le_bytes());
    world
}

fn native_null_source(world: &World) -> SelectedModelVariant {
    SelectedModelVariant {
        token: NULL_DEFINITION_TOKEN.to_le_bytes(),
        ..world.source()
    }
}

#[test]
fn registered_variant_with_actual_null_definition_keeps_minus_one_and_raw_terms() {
    let world = native_null_world();
    let mut wanted = expected();
    wanted.definition_index = Some(-1);
    assert_eq!(
        read_hull_variant(
            &world,
            BASE,
            &native_null_source(&world),
            &mut ModelMemo::default()
        ),
        Some(wanted)
    );
    let reads = world.reads.borrow();
    assert_eq!(
        (reads.len(), reads.iter().map(|(_, len)| len).sum::<usize>()),
        (12, 70)
    );
    assert!(
        !reads
            .iter()
            .any(|(at, _)| *at == BASE + cequipment_database::SINGLETON_RVA.0 || *at == LIST)
    );
}

#[test]
fn native_null_alias_activity_ordinal_and_partial_faults_withhold_only_the_ordinal() {
    for (address, bytes) in [
        (
            BASE + NULL_DEFINITION_SINGLETON_RVA.0,
            (DEFINITION as u64 + 8).to_le_bytes().to_vec(),
        ),
        (DEFINITION + cequipment_type::ACTIVE, vec![1]),
        (DEFINITION + DEFINITION_INDEX, 0_i32.to_le_bytes().to_vec()),
        (DEFINITION + DEFINITION_INDEX, 1_i32.to_le_bytes().to_vec()),
        (
            DEFINITION + DEFINITION_INDEX,
            (-2_i32).to_le_bytes().to_vec(),
        ),
    ] {
        let mut world = native_null_world();
        world.put(address, &bytes);
        let mut wanted = expected();
        wanted.definition_index = None;
        assert_eq!(
            read_hull_variant(
                &world,
                BASE,
                &native_null_source(&world),
                &mut ModelMemo::default()
            ),
            Some(wanted)
        );
    }
    let complete = native_null_world();
    read_hull_variant(
        &complete,
        BASE,
        &native_null_source(&complete),
        &mut ModelMemo::default(),
    )
    .unwrap();
    let spans = complete.reads.into_inner();
    for (ordinal, &(_, len)) in spans.iter().enumerate().skip(3).take(4) {
        for partial in 0..=len {
            let mut world = native_null_world();
            world.fault = Some((ordinal, partial));
            let mut wanted = expected();
            wanted.definition_index = None;
            assert_eq!(
                read_hull_variant(
                    &world,
                    BASE,
                    &native_null_source(&world),
                    &mut ModelMemo::default()
                ),
                Some(wanted)
            );
        }
    }
    let mut world = native_null_world();
    world.put(
        VARIANT + 8,
        &(cequipment_variant::REFERENCE_TYPE as u64).to_le_bytes(),
    );
    let source = SelectedModelVariant {
        handle: cequipment_variant::REFERENCE_TYPE as u64,
        ..native_null_source(&world)
    };
    let mut wanted = expected();
    wanted.definition_index = None;
    assert_eq!(
        read_hull_variant(&world, BASE, &source, &mut ModelMemo::default()),
        Some(wanted)
    );
}

#[test]
fn consecutive_aliases_retain_failed_fields_and_common_failure_until_reset() {
    for failed in [0, 4, 9, 12, 13] {
        let mut world = World::fixture();
        world.fault = Some((failed, 1));
        let mut memo = ModelMemo::default();
        let first = read_hull_variant(&world, BASE, &world.source(), &mut memo);
        let count = world.reads.borrow().len();
        world.fault = None;
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut memo),
            first
        );
        assert_eq!(world.reads.borrow().len(), count);
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut ModelMemo::default()),
            Some(expected())
        );
    }
}

#[test]
fn a_distinct_authenticated_source_replaces_only_the_fixed_last_observation() {
    let mut world = World::fixture();
    let mut memo = ModelMemo::default();
    assert_eq!(
        read_hull_variant(&world, BASE, &world.source(), &mut memo),
        Some(expected())
    );
    let count = world.reads.borrow().len();
    let mut source = world.source();
    source.handle += 1 << 32;
    world.put(VARIANT + 8, &source.handle.to_le_bytes());
    world.put(VARIANT + MODULE_COUNT, &7_i32.to_le_bytes());
    let mut wanted = expected();
    wanted.module_count = Some(7);
    assert_eq!(
        read_hull_variant(&world, BASE, &source, &mut memo),
        Some(wanted)
    );
    assert_eq!(world.reads.borrow().len(), count + 14);
    let count = world.reads.borrow().len();
    assert_eq!(
        read_hull_variant(&world, BASE, &source, &mut memo),
        Some(wanted)
    );
    assert_eq!(world.reads.borrow().len(), count);
}

#[test]
fn a_successful_null_or_unrequested_other_model_source_breaks_adjacency() {
    for intervening_is_null in [false, true] {
        let mut world = World::fixture();
        world.fault = Some((0, 0));
        let mut memo = ModelMemo::default();
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut memo),
            None
        );
        let count = world.reads.borrow().len();
        let mut other = world.source();
        other.handle += 1 << 32;
        memo.hull_variant
            .retain_source(BASE, (!intervening_is_null).then_some(&other));
        assert_eq!(world.reads.borrow().len(), count);
        world.fault = None;
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut memo),
            Some(expected())
        );
        assert_eq!(world.reads.borrow().len(), count + 14);
    }
}

#[test]
fn invalid_or_overflowing_addresses_never_escape_guarded_spans() {
    for pointer in [0, usize::MAX, usize::MAX - 15] {
        let world = World::fixture();
        let mut source = world.source();
        source.pointer = pointer;
        assert_eq!(
            read_hull_variant(&world, BASE, &source, &mut ModelMemo::default()),
            None
        );
        assert!(world.reads.borrow().is_empty());
    }
    let world = World::fixture();
    assert_eq!(
        read_hull_variant(
            &world,
            usize::MAX,
            &world.source(),
            &mut ModelMemo::default()
        ),
        None
    );
    assert!(world.reads.borrow().is_empty());
    let mut world = World::fixture();
    let mut source = world.source();
    source.definition = usize::MAX - 3;
    world.put(
        VARIANT + cequipment_variant::DEFINITION,
        &(source.definition as u64).to_le_bytes(),
    );
    let mut wanted = expected();
    wanted.definition_index = None;
    assert_eq!(
        read_hull_variant(&world, BASE, &source, &mut ModelMemo::default()),
        Some(wanted)
    );
    assert!(
        !world
            .reads
            .borrow()
            .iter()
            .any(|(at, _)| *at == source.definition)
    );
}

#[test]
fn each_complete_packet_of_the_capture_is_reused_but_never_a_refused_one() {
    const OTHER: usize = 0x60_0000;
    let mut world = World::fixture();
    let copy = world.regions[0].1.clone();
    world.regions.push((OTHER, copy));
    let other = || SelectedModelVariant {
        pointer: OTHER,
        handle: HANDLE + (1 << 32),
        definition: DEFINITION,
        token: TOKEN.to_le_bytes(),
    };
    world.put(OTHER + 8, &other().handle.to_le_bytes());
    let mut memo = ModelMemo::default();
    world.reads.borrow_mut().clear();
    for source in [world.source(), other()] {
        assert_eq!(
            read_hull_variant(&world, BASE, &source, &mut memo),
            Some(expected())
        );
    }
    let count = world.reads.borrow().len();
    assert_eq!(count, 28);
    assert_eq!(
        read_hull_variant(&world, BASE, &world.source(), &mut memo),
        Some(expected())
    );
    assert_eq!(world.reads.borrow().len(), count);
    memo.reset();
    assert!(memo.hull_variant.complete.is_empty());
    let count = world.reads.borrow().len();
    assert_eq!(
        read_hull_variant(&world, BASE, &other(), &mut memo),
        Some(expected())
    );
    assert_eq!(world.reads.borrow().len(), count + 14);

    for failed in [0, 9] {
        let mut world = World::fixture();
        let copy = world.regions[0].1.clone();
        world.regions.push((OTHER, copy));
        world.put(OTHER + 8, &other().handle.to_le_bytes());
        world.fault = Some((failed, 0));
        let mut memo = ModelMemo::default();
        let refused = read_hull_variant(&world, BASE, &world.source(), &mut memo);
        assert_ne!(refused, Some(expected()));
        assert_eq!(
            read_hull_variant(&world, BASE, &other(), &mut memo),
            Some(expected())
        );
        let count = world.reads.borrow().len();
        assert_eq!(
            read_hull_variant(&world, BASE, &world.source(), &mut memo),
            Some(expected())
        );
        assert_eq!(world.reads.borrow().len(), count + 14);
    }
}
