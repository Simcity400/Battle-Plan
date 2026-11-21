#![allow(clippy::too_many_arguments)]
use super::*;
fn apply(
    state: &mut DecisionState,
    operation: Operation,
    container: u8,
    id: Option<u32>,
    token: &[u8],
    days: Option<i32>,
    lifecycle: Option<i32>,
    flags: u8,
    ordinal: u32,
) {
    reduce(
        state, operation, container, id, token, days, lifecycle, flags, ordinal,
    )
    .unwrap();
}
fn reduce(
    state: &mut DecisionState,
    operation: Operation,
    container: u8,
    id: Option<u32>,
    token: &[u8],
    days: Option<i32>,
    lifecycle: Option<i32>,
    flags: u8,
    ordinal: u32,
) -> io::Result<()> {
    let change = DecisionChange {
        operation,
        container,
        flags,
        ordinal,
        political_power: None,
    };
    let script = EventFeedScript {
        instance: id,
        token_len: token.len() as u32,
        days,
        lifecycle,
        change: Some(EventFeedScriptChange::Decision(change)),
        ..Default::default()
    };
    let row = EventFeedHistoryRow {
        read_hour: 7,
        script_tokens: token.to_vec(),
        ..Default::default()
    };
    let event = EventFeedEvent {
        sequence: 1,
        generation: 1,
        hour: 7,
        source: EventFeedSource::ScriptState,
        actor: EventFeedCountry {
            reference: 1,
            id: 2,
            instance: 3,
        },
        target: Default::default(),
        cause: Recorded::Value(Default::default()),
        script: Some(script),
    };
    change.validate(script)?;
    state.observe(&row, &event, script, change)
}
fn marker(state: &mut DecisionState, operation: Operation, container: u8, ordinal: u32) {
    apply(
        state, operation, container, None, b"", None, None, 0, ordinal,
    );
}
fn store(
    state: &mut DecisionState,
    container: u8,
    id: u32,
    token: &[u8],
    days: i32,
    lifecycle: i32,
    flags: u8,
) {
    apply(
        state,
        Operation::Store,
        container,
        Some(id),
        token,
        Some(days),
        Some(lifecycle),
        flags,
        0,
    );
}
fn baseline() -> DecisionState {
    let mut state = DecisionState::default();
    marker(&mut state, Operation::Baseline, 0, 0);
    state
}
#[test]
fn compact_decision_operations_roundtrip_and_reject_invented_operands() {
    for operation in [
        Operation::Baseline,
        Operation::Ready,
        Operation::Invalidate,
        Operation::Store,
        Operation::Erase,
        Operation::Update,
        Operation::PassBegin,
        Operation::PassCut,
        Operation::PassEnd,
        Operation::RandomSet,
        Operation::RandomIncrement,
        Operation::Rebuild,
        Operation::Candidate,
        Operation::Ignore,
        Operation::Refused,
    ] {
        let value = EventFeedScriptChange::Decision(DecisionChange {
            operation,
            container: 5,
            flags: 127,
            ordinal: u32::MAX,
            political_power: None,
        });
        let mut bytes = Vec::new();
        value.encode(&mut bytes).unwrap();
        assert_eq!(value.encoded_bytes().unwrap(), bytes.len());
        assert_eq!(
            EventFeedScriptChange::decode(&mut Decoder::new(&bytes)).unwrap(),
            value
        );
    }
    let change = DecisionChange::new(Operation::Store, TIMED);
    assert!(change.validate(EventFeedScript::default()).is_err());
    let mut script = EventFeedScript {
        instance: Some(1),
        ..Default::default()
    };
    change.validate(script).unwrap();
    script.option = Some(0);
    assert!(change.validate(script).is_err());
}
#[test]
fn taken_duplicates_preserve_swap_ordinal_and_stable_all_removal() {
    let mut state = baseline();
    for (id, token) in [(1, b"a"), (2, b"b"), (3, b"a"), (4, b"c")] {
        store(&mut state, TAKEN, id, token, 0, 0, 0);
    }
    marker(&mut state, Operation::Ready, 0, 0);
    apply(
        &mut state,
        Operation::Erase,
        TAKEN,
        Some(0),
        b"a",
        None,
        None,
        SWAP | ORDINAL,
        2,
    );
    assert_eq!(
        state
            .members(TAKEN)
            .unwrap()
            .iter()
            .map(|row| row.token.as_slice())
            .collect::<Vec<_>>(),
        [b"a", b"b", b"c"]
    );
    apply(
        &mut state,
        Operation::Erase,
        TAKEN,
        Some(0),
        b"a",
        None,
        None,
        ALL,
        0,
    );
    assert_eq!(
        state
            .members(TAKEN)
            .unwrap()
            .iter()
            .map(|row| row.instance)
            .collect::<Vec<_>>(),
        [2, 4]
    );
}
#[test]
fn quiet_pass_decrements_are_signed_gated_and_do_not_require_visit_rows() {
    let mut state = baseline();
    for id in 1..=5000 {
        store(
            &mut state,
            REENABLE,
            id,
            b"cool",
            i32::MIN,
            0,
            if id % 2 == 0 { PERMANENT } else { 0 },
        );
    }
    marker(&mut state, Operation::Ready, 0, 0);
    marker(&mut state, Operation::PassBegin, REENABLE, 5000);
    assert!(!state.complete());
    marker(&mut state, Operation::PassEnd, REENABLE, 0);
    let rows = state.members(REENABLE).unwrap();
    assert_eq!(rows[0].days, i32::MAX);
    assert_eq!(rows[1].days, i32::MIN);
    marker(&mut state, Operation::PassBegin, REENABLE, 5000);
    marker(&mut state, Operation::PassEnd, REENABLE, 0);
    assert_eq!(state.members(REENABLE).unwrap()[0].days, i32::MAX - 1);
}
#[test]
fn sparse_cut_precedes_effect_mutation_and_transfer_preserves_revisit_order() {
    let mut state = baseline();
    for (id, days) in [(1, 10), (2, 1), (3, 5)] {
        store(&mut state, REMOVAL, id, b"x", days, 0, 0);
    }
    marker(&mut state, Operation::Ready, 0, 0);
    marker(&mut state, Operation::PassBegin, REMOVAL, 3);
    apply(
        &mut state,
        Operation::PassCut,
        REMOVAL,
        Some(2),
        b"x",
        Some(0),
        None,
        0,
        0,
    );
    apply(
        &mut state,
        Operation::Update,
        REMOVAL,
        Some(3),
        b"x",
        Some(15),
        None,
        0,
        0,
    );
    store(&mut state, REENABLE, 2, b"x", 20, 0, MOVED | ITERATION);
    marker(&mut state, Operation::PassEnd, REMOVAL, 0);
    assert_eq!(
        state
            .members(REMOVAL)
            .unwrap()
            .iter()
            .map(|row| (row.instance, row.days))
            .collect::<Vec<_>>(),
        [(1, 9), (3, 14)]
    );
    assert_eq!(state.members(REENABLE).unwrap()[0].days, 20);
}
#[test]
fn targeted_reactivation_selection_and_random_count_survive_between_censuses() {
    let mut state = baseline();
    store(&mut state, TARGETED_TIMED, 1, b"mod_mission", 10, 0, 0);
    apply(
        &mut state,
        Operation::RandomSet,
        0,
        None,
        b"mod_mission",
        Some(i32::MAX),
        None,
        0,
        17,
    );
    marker(&mut state, Operation::Ready, 0, 0);
    apply(
        &mut state,
        Operation::Update,
        TARGETED_TIMED,
        Some(1),
        b"mod_mission",
        Some(15),
        Some(0),
        0,
        0,
    );
    apply(
        &mut state,
        Operation::Update,
        TARGETED_TIMED,
        Some(1),
        b"mod_mission",
        Some(-1),
        Some(1),
        0,
        0,
    );
    assert_eq!(
        (
            state.members(TARGETED_TIMED).unwrap()[0].days,
            state.members(TARGETED_TIMED).unwrap()[0].lifecycle
        ),
        (-1, 1)
    );
    apply(
        &mut state,
        Operation::RandomIncrement,
        0,
        None,
        b"mod_mission",
        None,
        None,
        0,
        17,
    );
    assert_eq!(state.random_counts[0].count, i32::MIN);
    apply(
        &mut state,
        Operation::Erase,
        TARGETED_TIMED,
        Some(1),
        b"",
        None,
        None,
        0,
        0,
    );
    assert!(state.members(TARGETED_TIMED).unwrap().is_empty());
    marker(&mut state, Operation::Invalidate, 0, 0);
    assert!(state.members(TARGETED_TIMED).is_err());
    marker(&mut state, Operation::Baseline, 0, 0);
    marker(&mut state, Operation::Ready, 0, 0);
    assert!(state.random_counts.is_empty());
}
#[test]
fn every_conditional_lifecycle_branch_and_genuine_loss_withdraws_carry() {
    for (container, states) in [
        (
            TIMED,
            vec![(0, true), (1, true), (2, true), (3, false), (4, true)],
        ),
        (TARGETED, vec![(0, false), (1, true), (2, true), (3, false)]),
        (
            TARGETED_TIMED,
            vec![(0, true), (1, true), (2, true), (3, true)],
        ),
    ] {
        for (lifecycle, decrements) in states {
            let mut state = baseline();
            store(&mut state, container, 1, b"x", 10, lifecycle, 0);
            marker(&mut state, Operation::Ready, 0, 0);
            marker(&mut state, Operation::PassBegin, container, 1);
            marker(&mut state, Operation::PassEnd, container, 0);
            assert_eq!(
                state.members(container).unwrap()[0].days,
                10 - i32::from(decrements)
            );
        }
    }
    let mut state = baseline();
    store(&mut state, TIMED, 1, b"x", 10, 1, 0);
    marker(&mut state, Operation::Ready, 0, 0);
    marker(&mut state, Operation::PassBegin, TIMED, 2);
    assert!(!state.complete());
}
#[test]
fn candidate_filter_and_ignore_marks_are_owner_local_and_clear_exactly() {
    let mut owner = baseline();
    marker(&mut owner, Operation::Ready, 0, 0);
    apply(
        &mut owner,
        Operation::Candidate,
        2,
        None,
        b"mod_target",
        None,
        None,
        0,
        17,
    );
    apply(
        &mut owner,
        Operation::Candidate,
        3,
        None,
        b"mod_state",
        None,
        None,
        0,
        42,
    );
    apply(
        &mut owner,
        Operation::Ignore,
        0,
        None,
        b"mod_target",
        None,
        None,
        IGNORED,
        0,
    );
    assert_eq!(owner.candidates.len(), 2);
    assert!(owner.ignored.contains(b"mod_target".as_slice()));
    apply(
        &mut owner,
        Operation::Ignore,
        0,
        None,
        b"mod_target",
        None,
        None,
        0,
        0,
    );
    assert!(owner.ignored.is_empty());
    marker(&mut owner, Operation::Rebuild, 3, 0);
    assert!(owner.complete());
    apply(
        &mut owner,
        Operation::Candidate,
        2,
        None,
        b"",
        None,
        None,
        ALL,
        0,
    );
    assert_eq!(owner.candidates.len(), 1);
    assert!(DecisionState::default().candidates.is_empty());
}

#[test]
fn effect_erasure_does_not_rewind_the_enclosing_native_cursor_or_fixed_limit() {
    let mut state = baseline();
    store(&mut state, TIMED, 1, b"one", 1, 0, 0);
    store(&mut state, TIMED, 2, b"two", 9, 0, 0);
    marker(&mut state, Operation::Ready, 0, 0);
    marker(&mut state, Operation::PassBegin, TIMED, 2);
    apply(
        &mut state,
        Operation::PassCut,
        TIMED,
        Some(1),
        b"one",
        Some(0),
        Some(0),
        0,
        0,
    );
    store(&mut state, TIMED, 3, b"appended", 5, 0, 0);
    apply(
        &mut state,
        Operation::Erase,
        TIMED,
        Some(1),
        b"",
        None,
        None,
        SWAP,
        0,
    );
    marker(&mut state, Operation::PassEnd, TIMED, 0);
    assert_eq!(
        state
            .members(TIMED)
            .unwrap()
            .iter()
            .map(|row| (row.instance, row.days))
            .collect::<Vec<_>>(),
        [(3, 5), (2, 8)]
    );
}

#[test]
fn nested_native_passes_keep_the_outer_anchor_and_decrement_each_real_visit() {
    let mut state = baseline();
    store(&mut state, TIMED, 1, b"one", 10, 0, 0);
    store(&mut state, TIMED, 2, b"two", 20, 0, 0);
    marker(&mut state, Operation::Ready, 0, 0);
    marker(&mut state, Operation::PassBegin, TIMED, 2);
    apply(
        &mut state,
        Operation::PassCut,
        TIMED,
        Some(1),
        b"one",
        Some(9),
        Some(0),
        0,
        0,
    );
    marker(&mut state, Operation::PassBegin, TIMED, 2);
    marker(&mut state, Operation::PassEnd, TIMED, 0);
    assert!(!state.complete());
    marker(&mut state, Operation::PassEnd, TIMED, 0);
    assert_eq!(
        state
            .members(TIMED)
            .unwrap()
            .iter()
            .map(|row| row.days)
            .collect::<Vec<_>>(),
        [8, 18]
    );
}

#[test]
fn one_bulk_candidate_clear_mark_matches_all_native_vector_stores() {
    let mut state = baseline();
    marker(&mut state, Operation::Ready, 0, 0);
    for kind in 0..4 {
        apply(
            &mut state,
            Operation::Candidate,
            kind,
            None,
            b"target",
            None,
            None,
            0,
            17,
        );
    }
    apply(
        &mut state,
        Operation::Candidate,
        0,
        None,
        b"",
        None,
        None,
        ALL | CLEAR_ALL_CANDIDATES,
        0,
    );
    assert!(state.candidates.is_empty());
}

#[test]
fn ordered_duplicate_random_items_increment_only_the_native_first_match() {
    let mut state = baseline();
    for count in [7, 11] {
        apply(
            &mut state,
            Operation::RandomSet,
            0,
            None,
            b"mod_counter",
            Some(count),
            None,
            0,
            17,
        );
    }
    marker(&mut state, Operation::Ready, 0, 0);
    apply(
        &mut state,
        Operation::RandomIncrement,
        0,
        None,
        b"mod_counter",
        None,
        None,
        0,
        17,
    );
    assert_eq!(
        state
            .random_counts
            .iter()
            .map(|row| row.count)
            .collect::<Vec<_>>(),
        [8, 11]
    );
}

#[test]
fn state_only_ordinary_reactivation_carries_duration() {
    let mut state = baseline();
    store(&mut state, TARGETED, 17, b"ordinary", -5, 2, 0);
    marker(&mut state, Operation::Ready, 0, 0);
    apply(
        &mut state,
        Operation::Update,
        TARGETED,
        Some(17),
        b"ordinary",
        None,
        Some(1),
        0,
        0,
    );
    let member = &state.members(TARGETED).unwrap()[0];
    assert_eq!((member.days, member.lifecycle), (-5, 1));
}
#[test]
fn rebuild_balance_codec_preserves_signed_fixed5_and_other_ops_have_no_extra_bytes() {
    for power in [None, Some(i64::MIN), Some(100 * 100_000), Some(i64::MAX)] {
        let mut change = DecisionChange::new(Operation::Rebuild, 3);
        change.political_power = power;
        let mut bytes = Vec::new();
        change.encode(&mut bytes).unwrap();
        assert_eq!(
            DecisionChange::decode(&mut Decoder::new(&bytes)).unwrap(),
            change
        );
        assert_eq!(change.encoded_bytes().unwrap(), bytes.len());
        change.validate(EventFeedScript::default()).unwrap();
    }
    let change = DecisionChange::new(Operation::Ready, 0);
    assert_eq!(change.encoded_bytes().unwrap(), 4);
}

#[test]
fn owner_refusal_needs_its_reason_and_retires_a_certified_or_open_baseline() {
    let refusal = DecisionChange::new(Operation::Refused, 0);
    let reason = EventFeedScript {
        token_len: 6,
        ..Default::default()
    };
    refusal.validate(reason).unwrap();
    for script in [
        EventFeedScript::default(),
        EventFeedScript {
            token_unrecorded: true,
            ..Default::default()
        },
        EventFeedScript {
            instance: Some(1),
            ..reason
        },
        EventFeedScript {
            native_event: Some(EventFeedEventIdentity { kind: 1, id: 1 }),
            ..reason
        },
        EventFeedScript {
            scope_len: 1,
            ..reason
        },
        EventFeedScript {
            scope_unrecorded: true,
            ..reason
        },
    ] {
        assert!(refusal.validate(script).is_err(), "{script:?}");
    }
    for (container, flags, ordinal) in [(TIMED, 0, 0), (0, IGNORED, 0), (0, 0, 1)] {
        let change = DecisionChange {
            container,
            flags,
            ordinal,
            ..refusal
        };
        assert!(change.validate(reason).is_err());
    }
    let mut certified = baseline();
    store(&mut certified, TIMED, 1, b"x", 10, 0, 0);
    marker(&mut certified, Operation::Ready, 0, 0);
    assert!(certified.complete());
    apply(
        &mut certified,
        Operation::Refused,
        0,
        None,
        b"reason",
        None,
        None,
        0,
        0,
    );
    assert!(certified.members(TIMED).is_err());
    let mut open = baseline();
    store(&mut open, TIMED, 1, b"x", 10, 0, 0);
    apply(
        &mut open,
        Operation::Refused,
        0,
        None,
        b"reason",
        None,
        None,
        0,
        0,
    );
    assert!(reduce(&mut open, Operation::Ready, 0, None, b"", None, None, 0, 0).is_err());
    assert!(!open.complete());
    marker(&mut open, Operation::Baseline, 0, 0);
    marker(&mut open, Operation::Ready, 0, 0);
    assert!(open.members(TIMED).unwrap().is_empty());
}
