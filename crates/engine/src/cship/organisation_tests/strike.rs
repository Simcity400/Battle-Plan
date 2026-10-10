//! Focused optional-lane cases using the existing organisation fixtures.
use super::*;
use crate::cship::{
    TorpedoCooldownMemo, read_maximum_organisation_and_torpedo_cooldown, read_strike_bases,
};

fn prepared_fixture(mut f: Fixture) -> Fixture {
    // Current definition count, not a production constant: this synthetic table
    // has rows through 0x2FF in its already constructed contiguous byte range.
    f.dword(IMAGE + md::TABLE_RVA + 8, 0x300);
    f.dword(IMAGE + md::TABLE_RVA + 12, 0x300);
    // Genuine empty MSVC map: all three sentinel links point to itself.
    f.word(0x8008, 0x8000);
    f.word(0x8010, 0x8000);
    if f.bytes.contains_key(&LEADER) {
        f.word(LEADER + crate::cunit_leader::TYPE, (2_u64 << 32) | 7);
    }
    f
}

fn capture(
    core: &impl Memory,
    suffix: &impl Memory,
    metadata: &mut TorpedoCooldownMemo,
) -> Result<crate::cship::OrganisationAndCooldown, &'static str> {
    read_maximum_organisation_and_torpedo_cooldown(
        core,
        suffix,
        IMAGE,
        SHIP,
        &mut OrganisationMemo::default(),
        metadata,
    )
}

#[test]
fn absent_cooldown_skips_metadata_and_each_base_fault_is_independent() {
    let mut f = prepared_fixture(fixture());
    f.word(SHIP + 0x1e8, i64::MIN as u64);
    f.word(SHIP + 0x1f8, 7);
    f.word(SHIP + 0x210, i64::MAX as u64);
    f.word(SHIP + 0x208, (-13_i64) as u64);
    f.bytes.remove(&(SHIP + 0x1f8 + 7));
    assert_eq!(
        read_strike_bases(&f, SHIP),
        [Some(i64::MIN), None, Some(i64::MAX), Some(-13)]
    );
    // No 0x1BE source is present. Vacant metadata storage remains undemanded.
    let captured = capture(&f, &f, &mut TorpedoCooldownMemo::vacant(IMAGE)).unwrap();
    assert_eq!(
        captured.maximum_organisation,
        maximum_organisation(&f, IMAGE, SHIP).unwrap()
    );
    assert_eq!(captured.torpedo_cooldown_modifier, Ok(0));
}

#[test]
fn cooldown_metadata_fault_is_sticky_but_does_not_poison_maximum() {
    let mut f = prepared_fixture(fixture());
    f.modifier(
        COUNTRY + 0x5b8,
        0x6000,
        &[(0x46, 100000), (0x47, 10000), (0x1be, 123), (0x27e, 20000)],
    );
    let selector = 0x9000 + 0x1be * md::STRIDE + md::FEATURE_SELECTOR;
    let saved = f.bytes.remove(&selector).unwrap();
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    let captured = capture(&f, &f, &mut metadata).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert_eq!(
        captured.torpedo_cooldown_modifier,
        Err("modifier selector unreadable")
    );
    f.bytes.insert(selector, saved);
    let captured = capture(&f, &f, &mut metadata).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert_eq!(
        captured.torpedo_cooldown_modifier,
        Err("modifier selector unreadable")
    );
    metadata.clear();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(123)
    );
}

#[test]
fn present_zero_requires_prepared_metadata_but_not_original_org_admission() {
    let mut f = prepared_fixture(fixture());
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 0)]);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    let captured = capture(&f, &f, &mut TorpedoCooldownMemo::vacant(IMAGE)).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert_eq!(
        captured.torpedo_cooldown_modifier,
        Err(crate::cship::STRIKE_METADATA_CAPACITY)
    );
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(0)
    );
}

#[test]
fn each_source_is_clamped_before_composition_and_disabled_feature_skips_flags() {
    let mut f = prepared_fixture(fixture());
    f.modifier(COUNTRY + 0x5b8, 0x6000, &[(0x1be, 200000)]);
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, -50000)]);
    let row = 0x9000 + 0x1be * md::STRIDE;
    f.dword(row + md::FLAGS, md::CLAMP_MAXIMUM);
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(100000)
    );
    f.dword(row + md::FEATURE_SELECTOR, 4);
    f.word(IMAGE + md::ELIGIBILITY_CALLBACK_RVA, 0);
    f.bytes.remove(&(row + md::FLAGS));
    metadata.clear();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(0)
    );
}

#[test]
fn cooldown_only_doctrine_branch_failure_leaves_original_lazy_org_result() {
    let mut f = prepared_fixture(fixture());
    const NODE: usize = 0x8100;
    const MODIFIER: usize = 0x8200;
    f.word(TASK + 0x648, 1);
    for offset in [0, 8, 0x10] {
        f.word(0x8000 + offset, NODE as u64);
    }
    for offset in [0, 8, 0x10] {
        f.word(NODE + offset, 0x8000);
    }
    f.put(NODE + 0x19, &[0]);
    f.dword(NODE + 0x20, 1);
    f.word(NODE + 0x30, MODIFIER as u64);
    f.modifier(MODIFIER + 0x10, 0x8300, &[(0x1be, 1)]);
    // Native lexer/dynamic-key controls deliberately do not exist. The old
    // four lanes are all zero for this doctrine node, so they skip this arm.
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    let captured = capture(&f, &f, &mut metadata).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert!(captured.torpedo_cooldown_modifier.is_err());
}

#[test]
fn critical_scaling_truncates_each_entry_and_reuses_same_call_severity() {
    let mut f = prepared_fixture(fixture());
    f.modifier(COUNTRY + 0x5b8, 0x6000, &[(0x1bb, 50000)]);
    f.array(SHIP + 0x918, 0x6300, 2);
    for index in 0..2 {
        let definition = 0x7400 + index * 0x200;
        f.word(0x6300 + index * 16, definition as u64);
        f.dword(0x6300 + index * 16 + 8, 1);
        f.modifier(definition + 0x98, 0x6400 + index * 32, &[(0x1be, 1)]);
    }
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(2)
    );
    // Truncating after summing would produce 3, not 2.
    f.dword(0x6308, (-2_i32) as u32);
    f.modifier(0x7400 + 0x98, 0x6400, &[(0x1be, 33333)]);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(-99998)
    );
}

#[test]
fn support_sums_four_definitions_then_takes_maximum_over_influences() {
    let mut f = prepared_fixture(support_fixture());
    f.array(TASK + 0x728, 0x12000, 2);
    f.word(0x12008, 0x12100); // A repeated influence remains an occurrence.
    for index in 0..4 {
        let definition = 0x1a000 + index * 0x200;
        f.modifier(definition + 0xe0, definition + 0x100, &[(0x1be, 5)]);
    }
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(20)
    );
    // A per-definition max would be 5; sum-over-influences would be 40.
}

#[test]
fn pride_and_null_location_preserve_the_native_lazy_source_ladder() {
    let mut f = prepared_fixture(fixture());
    f.modifier(COUNTRY + 0x5b8, 0x6000, &[(0x1be, 20)]);
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 30)]);
    f.modifier(0x7000 + 0x1b0, 0x6200, &[(0x1be, 40)]);
    f.word(COUNTRY + 0x250, (42_u64 << 32) | 51);
    f.word(IMAGE + 0x0331_7cc8, 0x8800);
    f.word(0x8800 + 0xb0, 0x8900);
    f.modifier(0x8910, 0x8a00, &[(0x1be, 50)]);
    // This source must be wholly skipped when location is null.
    f.array(TASK + 0x668, usize::MAX, 1);
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(140)
    );
}

#[test]
fn weather_adjustment_and_explicit_default_leader_use_the_complete_same_ship_chain() {
    let mut f = prepared_fixture(weather_fixture());
    f.array(SHIP + 0x918, 0, 0);
    f.modifier(COUNTRY + 0x5b8, 0x6000, &[(0x1be, 20), (0x248, -25000)]);
    for (address, entries, value) in [
        (TERRAIN_ROWS + 0x38, 0x34000, 3),
        (0x29000 + 0x58, 0x37000, 40),
        (0x2a000 + 0x280, 0x37100, -7),
        (TASK + 0x668, 0x37200, 11),
        (LEADER + 0x418, 0x33000, 5),
        (CLASS_ROWS + 0xd0 + 0x20, 0x35000, 6),
    ] {
        f.modifier(address, entries, &[(0x1be, value)]);
    }
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    // 20 country +3 terrain +30 scaled region -7 province +11 task +5+6 leader.
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(68)
    );
}

#[test]
fn explicit_leader_mismatch_withholds_only_cooldown_before_source_fold() {
    let core = prepared_fixture(shared_sources_fixture());
    let mut suffix = core.clone();
    suffix.word(TASK + ctask_force::FLEET, 0);
    let mut metadata = TorpedoCooldownMemo::try_prepare(&core, IMAGE).unwrap();
    let captured = capture(&core, &suffix, &mut metadata).unwrap();
    assert_eq!(
        captured.maximum_organisation,
        maximum_organisation(&core, IMAGE, SHIP).unwrap()
    );
    assert_eq!(
        captured.torpedo_cooldown_modifier,
        Err("ship strike leader differs from organisation invocation")
    );
}

#[test]
fn original_four_key_read_sequence_is_identical_in_combined_call() {
    struct Trace<'a> {
        source: &'a Fixture,
        reads: RefCell<Vec<(usize, usize)>>,
    }
    impl Memory for Trace<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            self.reads.borrow_mut().push((address, output.len()));
            self.source.copy(address, output)
        }
    }
    let f = prepared_fixture(weather_fixture());
    let old = Trace {
        source: &f,
        reads: RefCell::new(Vec::new()),
    };
    let expected = maximum_organisation(&old, IMAGE, SHIP).unwrap();
    let new = Trace {
        source: &f,
        reads: RefCell::new(Vec::new()),
    };
    let mut metadata = TorpedoCooldownMemo::try_prepare(&f, IMAGE).unwrap();
    let captured = capture(&new, &f, &mut metadata).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert_eq!(*old.reads.borrow(), *new.reads.borrow());
}

// Additional optional cooldown cases use the same fixture.
#[test]
fn count_preparation_is_storage_only_and_current_header_refusal_is_sticky() {
    let mut f = prepared_fixture(fixture());
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 9)]);
    let expected = maximum_organisation(&f, IMAGE, SHIP).unwrap();
    let mut metadata = TorpedoCooldownMemo::try_from_validated_count(0x300).unwrap();
    metadata.begin_capture(IMAGE);
    // Storage is sufficient but the current synthetic table does not admit
    // the reached key. Existing original metadata/read behavior is unchanged.
    f.dword(IMAGE + md::TABLE_RVA + 12, 0x1be);
    let captured = capture(&f, &f, &mut metadata).unwrap();
    assert_eq!(captured.maximum_organisation, expected);
    assert_eq!(
        captured.torpedo_cooldown_modifier,
        Err("ship strike modifier metadata key outside current table")
    );
    f.dword(IMAGE + md::TABLE_RVA + 12, 0x300);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Err("ship strike modifier metadata key outside current table")
    );
    metadata.begin_capture(IMAGE);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(9)
    );
}

#[test]
fn count_hint_growth_and_base_binding_happen_before_the_next_capture() {
    let mut f = prepared_fixture(fixture());
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 0)]);
    let mut metadata = TorpedoCooldownMemo::try_from_validated_count(0).unwrap();
    metadata.begin_capture(IMAGE);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Err(crate::cship::STRIKE_METADATA_CAPACITY)
    );
    // Existing runtime obtains this count from the live validated header;
    // collector grow runs only while this packet is off-thread and idle.
    let count = crate::modifier_definition::table(&f, IMAGE).unwrap().count as usize;
    metadata.try_grow_from_validated_count(count).unwrap();
    metadata.begin_capture(IMAGE);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(0)
    );
}
// Prepared-storage cases use the same strike fixture.
#[test]
fn lower_sizing_hint_never_shrinks_the_prepared_definition_floor() {
    let mut f = prepared_fixture(fixture());
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 17)]);
    let mut metadata = TorpedoCooldownMemo::try_from_validated_count(0x300).unwrap();
    metadata.try_grow_from_validated_count(0x1be).unwrap();
    metadata.begin_capture(IMAGE);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Ok(17)
    );
    // The storage floor is not admission: a lower current native count still
    // independently refuses this reached key, even with ample prepared space.
    f.dword(IMAGE + md::TABLE_RVA + 12, 0x1be);
    metadata.begin_capture(IMAGE);
    assert_eq!(
        capture(&f, &f, &mut metadata)
            .unwrap()
            .torpedo_cooldown_modifier,
        Err("ship strike modifier metadata key outside current table")
    );
}

#[test]
fn worker_cooldown_metadata_keeps_first_capture_and_later_native_table_growth() {
    let mut f = prepared_fixture(fixture());
    f.modifier(SHIP + 0x750, 0x6100, &[(0x1be, 9)]);
    let mut metadata = TorpedoCooldownMemo::vacant(IMAGE);
    metadata.enable_worker_growth();
    for count in [5_003, 50_003] {
        f.dword(IMAGE + md::TABLE_RVA + 8, count);
        f.dword(IMAGE + md::TABLE_RVA + 12, count);
        metadata.begin_capture(IMAGE);
        for _ in 0..5_000 {
            assert_eq!(
                capture(&f, &f, &mut metadata)
                    .unwrap()
                    .torpedo_cooldown_modifier,
                Ok(9)
            );
        }
        assert!(metadata.required_definition_capacity() >= count as usize);
    }
}
