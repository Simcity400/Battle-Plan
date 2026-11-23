use super::*;

const LOCAL: [usize; 4] = [0, 1, 2, 4];

fn inputs(
    current: &Captured,
    shadow: &mut CharacterShadow,
    cached: Option<&[Arc<TableRows>; 4]>,
    branch: BranchId,
) -> [TableInput; N] {
    let cached = std::array::from_fn(|index| cached.map(|rows| &rows[index]));
    current.table_inputs(
        Some(&mut CharacterConversion::new(branch, shadow, cached)),
        None,
    )
}

fn shared(input: &[TableInput; N]) -> [Arc<TableRows>; 4] {
    LOCAL.map(|index| match &input[index] {
        TableInput::SharedComplete(rows) => Arc::clone(rows),
        _ => panic!("expected complete shared table at {index}"),
    })
}

fn assert_canonical(current: &Captured, input: &[TableInput; N]) {
    for (expected, actual) in current.tables().iter().zip(input) {
        match actual {
            TableInput::Sampled(actual) => assert_eq!(actual, expected),
            TableInput::SharedComplete(actual) => {
                assert_eq!(&TableView::Complete((**actual).clone()), expected);
            }
            TableInput::ComparedEqual(_)
            | TableInput::ComparedChanges { .. }
            | TableInput::SliceUpdate { .. }
            | TableInput::UniformOptionalI32 { .. } => {
                panic!("character reuse cannot bypass validation")
            }
        }
    }
}

#[test]
fn character_shadow_exact_hits_keep_current_arcs_and_ignore_unused_pool_tails() {
    let _globals = crate::extended::characters::test_globals();
    let mut shadow = CharacterShadow::default();
    let first = character_shadow_fixture(9);
    let cached = shared(&inputs(&first, &mut shadow, None, 1));
    let pointers = (
        shadow.characters.rows.as_ptr(),
        shadow.characters.bytes.as_ptr(),
    );
    let mut current = character_shadow_fixture(9);
    current
        .characters
        .rows
        .rows
        .push(ccharacter::Row::default());
    current.characters.bytes.first[7] = 42;
    let hit = inputs(&current, &mut shadow, Some(&cached), 1);
    assert_canonical(&current, &hit);
    for (previous, current) in cached.iter().zip(shared(&hit)) {
        assert!(Arc::ptr_eq(previous, &current));
    }
    assert_eq!(
        pointers,
        (
            shadow.characters.rows.as_ptr(),
            shadow.characters.bytes.as_ptr()
        )
    );
    assert_eq!(shadow.characters.rows.len(), 2);
    assert_eq!(shadow.characters.bytes, [9, 0, 0xff, b'b', b'c', b'd']);
    assert!(matches!(
        shadow_input(
            TableId::Characters,
            &current.characters,
            &mut shadow.characters,
            Some(&cached[0]),
            |_, _| panic!("an exact validated hit must not rescan row ranges"),
            || panic!("an exact validated hit must not convert"),
        ),
        TableInput::SharedComplete(_)
    ));
}

#[test]
fn character_shadow_every_consumed_field_and_byte_change_misses_independently() {
    let _globals = crate::extended::characters::test_globals();
    type Change = (usize, fn(&mut Captured));
    let changes: &[Change] = &[
        (0, |c| c.characters.rows.rows[0].id += 20),
        (0, |c| c.characters.rows.rows[0].country += 1),
        (0, |c| c.characters.rows.rows[0].name_token_id -= 1),
        (0, |c| c.characters.rows.rows[0].roster += 1),
        (0, |c| c.characters.rows.rows[0].ordinal += 1),
        (0, |c| c.characters.rows.rows[0].name_start += 1),
        (0, |c| c.characters.rows.rows[0].name_len -= 1),
        (0, |c| c.characters.rows.rows[0].token_start += 1),
        (0, |c| c.characters.rows.rows[0].token_len -= 1),
        (0, |c| c.characters.bytes.first[0] ^= 1),
        (0, |c| c.characters.bytes.first[2] ^= 1),
        (0, |c| c.characters.bytes.required += 1),
        (1, |c| c.leaders.rows.rows[0].character += 20),
        (1, |c| c.leaders.rows.rows[0].country += 1),
        (1, |c| c.leaders.rows.rows[0].ideology -= 1),
        (2, |c| c.advisors.rows.rows[0].character += 20),
        (2, |c| c.advisors.rows.rows[0].country += 1),
        (2, |c| c.advisors.rows.rows[0].ordinal += 1),
        (2, |c| c.advisors.rows.rows[0].key_start += 1),
        (2, |c| c.advisors.rows.rows[0].key_len -= 1),
        (2, |c| c.advisors.rows.rows[0].idea_start += 1),
        (2, |c| c.advisors.rows.rows[0].idea_len -= 1),
        (2, |c| c.advisors.rows.rows[0].slot_start += 1),
        (2, |c| c.advisors.rows.rows[0].slot_len -= 1),
        (2, |c| c.advisors.bytes.first[0] ^= 1),
        (2, |c| c.advisors.bytes.first[1] ^= 1),
        (2, |c| c.advisors.bytes.first[2] ^= 1),
        (2, |c| c.advisors.bytes.required += 1),
        (3, |c| c.traits.rows.rows[0].leader_id += 20),
        (3, |c| c.traits.rows.rows[0].ordinal += 1),
        (3, |c| c.traits.rows.rows[0].token_id -= 1),
        (3, |c| c.traits.rows.rows[0].definition_ordinal -= 1),
        (3, |c| c.traits.rows.rows[0].token_start += 1),
        (3, |c| c.traits.rows.rows[0].token_len -= 1),
        (3, |c| c.traits.bytes.first[0] ^= 1),
        (3, |c| c.traits.bytes.required += 1),
        (0, |c| c.characters.rows.outcome = Outcome::Rows { len: 1 }),
        (1, |c| c.leaders.rows.outcome = Outcome::Rows { len: 1 }),
        (2, |c| c.advisors.rows.outcome = Outcome::Rows { len: 1 }),
        (3, |c| c.traits.rows.outcome = Outcome::Rows { len: 1 }),
        (0, |c| c.characters.rows.rows.swap(0, 1)),
        (1, |c| c.leaders.rows.rows.swap(0, 1)),
        (2, |c| c.advisors.rows.rows.swap(0, 1)),
        (3, |c| c.traits.rows.rows.swap(0, 1)),
    ];
    for &(changed, change) in changes {
        let first = character_shadow_fixture(9);
        let mut shadow = CharacterShadow::default();
        let cached = shared(&inputs(&first, &mut shadow, None, 1));
        let mut current = character_shadow_fixture(9);
        change(&mut current);
        let result = inputs(&current, &mut shadow, Some(&cached), 1);
        assert_canonical(&current, &result);
        for (index, output) in shared(&result).iter().enumerate() {
            assert_eq!(Arc::ptr_eq(&cached[index], output), index != changed);
        }
    }
}

fn fault<R: Clone>(buffer: &mut Buffer<R>, fault: usize) {
    match fault {
        0 => {
            buffer.rows.outcome = Outcome::Unavailable {
                reason: "injected native failure",
            }
        }
        1 => {
            buffer.rows.outcome = Outcome::Truncated {
                len: 1,
                unread_rows: 1,
            }
        }
        2 => {
            buffer.rows.outcome = Outcome::Truncated {
                len: 1,
                unread_rows: 0,
            }
        }
        3 => buffer.rows.outcome = Outcome::Rows { len: usize::MAX },
        5 => buffer.bytes.len = usize::MAX,
        6 => buffer.rows.rows[1] = buffer.rows.rows[0].clone(),
        _ => unreachable!(),
    }
}

#[test]
fn character_shadow_faults_clear_only_current_table_and_recover_via_conversion() {
    let _globals = crate::extended::characters::test_globals();
    for selected in 0..4 {
        for failure in [0, 1, 2, 3, 5, 6] {
            let first = character_shadow_fixture(9);
            let mut shadow = CharacterShadow::default();
            let cached = shared(&inputs(&first, &mut shadow, None, 1));
            let mut current = character_shadow_fixture(9);
            match selected {
                0 => fault(&mut current.characters, failure),
                1 => fault(&mut current.leaders, failure),
                2 => fault(&mut current.advisors, failure),
                3 => fault(&mut current.traits, failure),
                _ => unreachable!(),
            }
            let result = inputs(&current, &mut shadow, Some(&cached), 1);
            assert_canonical(&current, &result);
            assert!(matches!(&result[LOCAL[selected]], TableInput::Sampled(_)));
            let restored = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
            for index in 0..4 {
                assert_eq!(
                    Arc::ptr_eq(&cached[index], &restored[index]),
                    index != selected
                );
            }
        }
    }
    for selected in [0, 2, 3] {
        for overflow in [false, true] {
            let mut shadow = CharacterShadow::default();
            let first = character_shadow_fixture(9);
            let cached = shared(&inputs(&first, &mut shadow, None, 1));
            let mut current = character_shadow_fixture(9);
            let start = if overflow { usize::MAX } else { 7 };
            match selected {
                0 => current.characters.rows.rows[0].name_start = start,
                2 => current.advisors.rows.rows[0].idea_start = start,
                3 => current.traits.rows.rows[0].token_start = start,
                _ => unreachable!(),
            }
            let result = inputs(&current, &mut shadow, Some(&cached), 1);
            assert_canonical(&current, &result);
            assert!(matches!(
                &result[LOCAL[selected]],
                TableInput::Sampled(TableView::Unavailable { .. })
            ));
        }
    }
}

#[test]
fn character_shadow_offhours_branch_and_allocation_identity_are_not_inferred_from_values() {
    let _globals = crate::extended::characters::test_globals();
    let first = character_shadow_fixture(9);
    let mut shadow = CharacterShadow::default();
    let cached = shared(&inputs(&first, &mut shadow, None, 1));
    let mut skipped = character_shadow_fixture(99);
    for outcome in [
        &mut skipped.characters.rows.outcome,
        &mut skipped.leaders.rows.outcome,
        &mut skipped.advisors.rows.outcome,
        &mut skipped.traits.rows.outcome,
    ] {
        *outcome = Outcome::Unavailable {
            reason: replay_data::cadence::SKIPPED_REASON,
        };
    }
    assert_canonical(&skipped, &inputs(&skipped, &mut shadow, Some(&cached), 1));
    let hit = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
    assert!(cached.iter().zip(&hit).all(|(a, b)| Arc::ptr_eq(a, b)));
    let replaced = cached.each_ref().map(|rows| Arc::new((**rows).clone()));
    let fresh = shared(&inputs(&first, &mut shadow, Some(&replaced), 1));
    assert!(replaced.iter().zip(&fresh).all(|(a, b)| !Arc::ptr_eq(a, b)));
    let branch = shared(&inputs(&first, &mut shadow, Some(&fresh), 2));
    assert!(fresh.iter().zip(&branch).all(|(a, b)| !Arc::ptr_eq(a, b)));
    let missing = shared(&inputs(&first, &mut shadow, None, 2));
    assert!(branch.iter().zip(&missing).all(|(a, b)| !Arc::ptr_eq(a, b)));
    let wrong = std::array::from_fn(|_| Arc::clone(&missing[1]));
    let wrong_result = inputs(&first, &mut shadow, Some(&wrong), 2);
    assert_canonical(&first, &wrong_result);
    assert!(!Arc::ptr_eq(&shared(&wrong_result)[0], &wrong[0]));
}

#[test]
fn character_shadow_empty_rows_strings_and_unselected_dynamic_rows_remain_exact() {
    let _globals = crate::extended::characters::test_globals();
    let mut first = character_shadow_fixture(9);
    for row in &mut first.characters.rows.rows {
        row.name_len = 0;
        row.token_len = 0;
    }
    let mut shadow = CharacterShadow::default();
    let cached = shared(&inputs(&first, &mut shadow, None, 1));
    let mut dynamic = character_shadow_fixture(9);
    dynamic
        .characters
        .rows
        .rows
        .clone_from(&first.characters.rows.rows);
    dynamic.units.rows.outcome = Outcome::Rows { len: 1 };
    let result = inputs(&dynamic, &mut shadow, Some(&cached), 1);
    assert_canonical(&dynamic, &result);
    assert!(
        cached
            .iter()
            .zip(shared(&result))
            .all(|(a, b)| Arc::ptr_eq(a, &b))
    );
    assert!(matches!(
        &result[3],
        TableInput::Sampled(TableView::Complete(_))
    ));
    first.characters.rows.outcome = Outcome::Rows { len: 0 };
    first.leaders.rows.outcome = Outcome::Rows { len: 0 };
    first.advisors.rows.outcome = Outcome::Rows { len: 0 };
    first.traits.rows.outcome = Outcome::Rows { len: 0 };
    for bytes in [
        &mut first.characters.bytes,
        &mut first.advisors.bytes,
        &mut first.traits.bytes,
    ] {
        bytes.len = 0;
        bytes.required = 0;
    }
    let empty = shared(&inputs(&first, &mut shadow, Some(&cached), 1));
    let hit = inputs(&first, &mut shadow, Some(&empty), 1);
    assert_canonical(&first, &hit);
    assert!(
        empty
            .iter()
            .zip(shared(&hit))
            .all(|(a, b)| Arc::ptr_eq(a, &b))
    );
}
