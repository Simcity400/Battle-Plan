#[test]
fn selected_parsing_still_rejects_malformed_omitted_updates() {
    let frame = FrameEncoder::new().encode(source(1, 0), true).unwrap();
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let selected = [TableId::Countries];
    let projected = Frame::decode_selected(&mut Decoder::new(&bytes), Some(&selected)).unwrap();
    assert_eq!(
        projected.tables[TableId::Countries.index()],
        frame.tables[TableId::Countries.index()]
    );
    assert!(matches!(
        projected.tables[TableId::Clock.index()],
        TableUpdate::Unavailable { .. }
    ));
    // Walk the shared frame header to corrupt the first (omitted) update.
    let mut header = Decoder::new(&bytes);
    header.u64().unwrap();
    header.i32().unwrap();
    header.bool().unwrap();
    header.u64().unwrap();
    for _ in 0..TABLE_COUNT {
        header.u64().unwrap();
    }
    header.u64().unwrap();
    header.u64().unwrap();
    let offset = bytes.len() - header.remaining();
    bytes[offset] = u8::MAX;
    assert!(Frame::decode_selected(&mut Decoder::new(&bytes), Some(&selected)).is_err());
}

#[test]
fn selected_reconstruction_matches_full_decoder_through_baseline_loss() {
    let mut encoder = super::FrameEncoder::new();
    let mut full = super::FrameDecoder::new();
    let selected = [
        crate::rows::TableId::Countries,
        crate::rows::TableId::Campaign,
    ];
    let mut projected = super::FrameDecoder::for_tables(&selected);
    for hour in 0..8 {
        let mut frame = encoder.encode(source(1, hour), hour == 6).unwrap();
        if hour == 2 {
            frame.tables[crate::rows::TableId::Countries.index()] =
                super::TableUpdate::Unavailable {
                    reason: "lost baseline".into(),
                };
        }
        let expected = full.advance(&frame).unwrap();
        let actual = projected.advance(&frame).unwrap();
        for table in selected {
            assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        }
        assert!(matches!(
            *actual.tables[crate::rows::TableId::Clock.index()],
            super::TableView::Unavailable { .. }
        ));
    }
    let mut invalid = encoder.encode(source(1, 8), true).unwrap();
    invalid.tables[crate::rows::TableId::Clock.index()] =
        super::TableUpdate::Unchanged { baseline: 7 };
    assert!(
        projected.advance(&invalid).is_err(),
        "omitted tables still obey frame-wide validation"
    );
}

use super::*;
use crate::rows::{
    CampaignRow, ClockRow, CountryLinkRow, CountryRow, DivisionRow, FactionMemberRow, FactionRow,
    ProvinceRow, RowCodec, StateRow, WarRelationRow, tables,
};

const FACTION_ID: u64 = 0xfedc_ba98_0000_004d;

/// An hour that leaves a table unchanged hands on the same allocation; a
/// delta copies the baseline first while an earlier hour still holds it,
/// so that hour keeps its own rows.
#[test]
fn unchanged_tables_are_shared_and_a_delta_copies_on_write() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let original = source(1, 0);
    let first = decoder
        .apply(&encoder.encode(original.clone(), true).unwrap())
        .unwrap();
    let mut next = source(1, 1);
    change_country(&mut next, 5);
    let frame = encoder.encode(next.clone(), false).unwrap();
    let countries = TableId::Countries.index();
    let campaign = TableId::Campaign.index();
    assert!(matches!(
        frame.tables[campaign],
        TableUpdate::Unchanged { .. }
    ));
    assert!(matches!(frame.tables[countries], TableUpdate::Delta { .. }));
    let second = decoder.apply(&frame).unwrap();
    assert!(SharedView::ptr_eq(
        &first.tables[campaign],
        &second.tables[campaign]
    ));
    assert!(!SharedView::ptr_eq(
        &first.tables[countries],
        &second.tables[countries]
    ));
    assert_eq!(first.tables[countries], original.tables[countries]);
    assert_eq!(second.tables[countries], next.tables[countries]);
}

fn shared_input(source: DecodedFrame) -> FrameInput {
    let mut input: FrameInput = source.into();
    input.tables = input.tables.map(|input| match input {
        TableInput::Sampled(TableView::Complete(rows)) => {
            TableInput::SharedComplete(Arc::new(rows))
        }
        other => other,
    });
    input
}

// Exercise both newly converted allocations and the exact allocation held
// by the encoder. Fixture equality stands in for producer conversion proof.
fn reuse_committed_rows(input: &mut FrameInput, encoder: &FrameEncoder) {
    for (index, input) in input.tables.iter_mut().enumerate() {
        if let TableInput::SharedComplete(current) = input
            && let Some((_, BaselineRows::Shared(previous))) = &encoder.state.tables[index]
            && current.as_ref() == previous.as_ref()
        {
            *current = Arc::clone(previous);
        }
    }
}

#[test]
fn shared_complete_rows_match_owned_wire_and_adopt_each_validated_refresh() {
    for reuse_allocation in [false, true] {
        let mut shared = FrameEncoder::new();
        let mut owned = FrameEncoder::new();
        let mut decoder = FrameDecoder::new();
        let mut previous_rows = None;
        // Includes equal/changed refreshes, a gap, scheduled and forced
        // keyframes, and a branch round trip with reused hour values.
        for (branch, hour, value, force) in [
            (1, 100, 1, false),
            (1, 101, 1, false),
            (1, 107, 2, false),
            (1, 108, 2, false),
            (1, 820, 2, false),
            (1, 821, 2, true),
            (2, 100, 3, false),
            (1, 100, 4, false),
        ] {
            let mut source = source(branch, hour);
            change_country(&mut source, value);
            let mut input = shared_input(source.clone());
            if reuse_allocation {
                reuse_committed_rows(&mut input, &shared);
            }
            let allocations: [_; TABLE_COUNT] = std::array::from_fn(|index| {
                let TableInput::SharedComplete(rows) = &input.tables[index] else {
                    panic!("fixture has complete rows")
                };
                Arc::clone(rows)
            });
            let expected = owned.encode(source.clone(), force).unwrap();
            let actual = shared.encode_input(input, force).unwrap();
            assert_eq!(wire(&actual), wire(&expected));
            assert_eq!(decoder.apply(&actual).unwrap(), source);
            for (index, allocation) in allocations.iter().enumerate() {
                let (baseline_hour, BaselineRows::Shared(baseline)) =
                    shared.state.tables[index].as_ref().unwrap()
                else {
                    panic!("complete shared rows must stay shared")
                };
                assert_eq!(*baseline_hour, hour);
                assert!(Arc::ptr_eq(baseline, allocation));
                assert_eq!(Arc::strong_count(allocation), 2);
            }
            if let Some(previous) = previous_rows.take() {
                if std::sync::Weak::ptr_eq(
                    &previous,
                    &Arc::downgrade(&allocations[TableId::Countries.index()]),
                ) {
                    assert!(reuse_allocation);
                    assert!(std::sync::Weak::upgrade(&previous).is_some());
                } else {
                    assert!(std::sync::Weak::upgrade(&previous).is_none());
                }
            }
            previous_rows = Some(Arc::downgrade(&allocations[TableId::Countries.index()]));
        }
    }
}

#[test]
fn shared_baselines_survive_cadence_and_owned_equal_inputs_without_reallocation() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let table = TableId::HiredAdvisors;
    let first = shared_input(source(1, 0));
    let TableInput::SharedComplete(rows) = &first.tables[table.index()] else {
        panic!()
    };
    let allocation = Arc::clone(rows);
    decoder
        .apply(&encoder.encode_input(first, false).unwrap())
        .unwrap();
    for hour in 1..4 {
        let expected = source(1, hour);
        let mut input: FrameInput = expected.clone().into();
        if hour != 2 {
            input.tables[table.index()] = TableInput::Sampled(TableView::Unavailable {
                reason: crate::cadence::SKIPPED_REASON.into(),
            });
        }
        let frame = encoder.encode_input(input, false).unwrap();
        assert_eq!(
            frame.tables[table.index()],
            TableUpdate::Unchanged { baseline: hour - 1 }
        );
        assert_eq!(decoder.apply(&frame).unwrap(), expected);
        let (_, BaselineRows::Shared(baseline)) =
            encoder.state.tables[table.index()].as_ref().unwrap()
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(baseline, &allocation));
    }
}

#[test]
fn shared_partial_and_unavailable_clear_baselines_until_a_complete_snapshot() {
    let table = TableId::Characters;
    for partial in [false, true] {
        let mut encoder = FrameEncoder::new();
        let initial = shared_input(source(1, 0));
        let TableInput::SharedComplete(rows) = &initial.tables[table.index()] else {
            panic!()
        };
        let allocation = Arc::clone(rows);
        encoder.encode_input(initial, false).unwrap();
        let mut failed = shared_input(source(1, 1));
        failed.tables[table.index()] = TableInput::Sampled(if partial {
            TableView::Truncated {
                rows: TableRows::empty(table),
                missing: 1,
            }
        } else {
            TableView::Unavailable {
                reason: "fresh read failed".into(),
            }
        });
        encoder.encode_input(failed, false).unwrap();
        assert!(encoder.state.tables[table.index()].is_none());
        assert_eq!(Arc::strong_count(&allocation), 1);
        let mut restored = shared_input(source(1, 2));
        restored.tables[table.index()] = TableInput::SharedComplete(Arc::clone(&allocation));
        let restored = encoder.encode_input(restored, false).unwrap();
        assert!(matches!(
            restored.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
}

#[test]
fn shared_validation_is_atomic_and_preserves_existing_comparison_tokens() {
    let mut encoder = FrameEncoder::new();
    encoder
        .encode_input(shared_input(variable_source(1, 0, 1)), false)
        .unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let (_, BaselineRows::Shared(rows)) = encoder.state.tables[TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap()
    else {
        panic!()
    };
    let allocation = Arc::clone(rows);
    let duplicate = TableRows::Countries(vec![
        variable_source(1, 1, 1)
            .complete::<tables::Countries>()
            .unwrap()[0]
            .clone();
        2
    ]);
    for (table, rows) in [
        (TableId::Countries, duplicate),
        (TableId::CountryIntel, TableRows::Clock(Vec::new())),
        (
            TableId::Clock,
            TableRows::Clock(vec![ClockRow {
                hour: 99,
                ..Default::default()
            }]),
        ),
    ] {
        let mut invalid = shared_input(variable_source(1, 1, 2));
        invalid.tables[table.index()] = TableInput::SharedComplete(Arc::new(rows));
        reuse_committed_rows(&mut invalid, &encoder);
        assert!(encoder.encode_input(invalid, false).is_err());
        assert_eq!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
                .unwrap()
                .0,
            token
        );
        let (_, BaselineRows::Shared(rows)) = encoder.state.tables
            [TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap() else {
            panic!()
        };
        assert!(Arc::ptr_eq(rows, &allocation));
        assert_eq!(encoder.state.last_hour, Some(0));
        assert!(
            encoder
                .state
                .tables
                .iter()
                .flatten()
                .all(|(hour, _)| *hour == 0)
        );
    }
    let mut compared = shared_input(variable_source(1, 1, 1));
    compared.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    let frame = encoder.encode_input(compared, false).unwrap();
    assert_eq!(
        frame.tables[TableId::ScriptVariables.index()],
        TableUpdate::Unchanged { baseline: 0 }
    );
    let (_, BaselineRows::Shared(rows)) = encoder.state.tables[TableId::ScriptVariables.index()]
        .as_ref()
        .unwrap()
    else {
        panic!()
    };
    assert!(Arc::ptr_eq(rows, &allocation));
}

#[test]
fn shared_committed_identity_checks_clock_mutation_table_and_encoder_reset() {
    let mut encoder = FrameEncoder::new();
    let first = shared_input(source(1, 0));
    let TableInput::SharedComplete(clock) = &first.tables[TableId::Clock.index()] else {
        panic!()
    };
    let clock = Arc::clone(clock);
    let TableInput::SharedComplete(countries) = &first.tables[TableId::Countries.index()] else {
        panic!()
    };
    let countries = Arc::clone(countries);
    encoder.encode_input(first, false).unwrap();
    let revision = encoder.revision;
    let mut mutated = Arc::clone(&countries);
    let rows = Arc::make_mut(&mut mutated)
        .rows_mut::<tables::Countries>()
        .unwrap();
    rows.push(rows[0].clone());
    assert!(!Arc::ptr_eq(&mutated, &countries));
    for (table, rows, reason) in [
        (
            TableId::Clock,
            clock,
            "clock row hour differs from frame hour",
        ),
        (
            TableId::Countries,
            mutated,
            "table rows must have matching types and unique ascending keys",
        ),
        (
            TableId::CountryIntel,
            Arc::clone(&countries),
            "table rows must have matching types and unique ascending keys",
        ),
    ] {
        let mut invalid = shared_input(source(1, 1));
        reuse_committed_rows(&mut invalid, &encoder);
        invalid.tables[table.index()] = TableInput::SharedComplete(rows);
        assert_eq!(
            encoder
                .encode_input(invalid, false)
                .unwrap_err()
                .to_string(),
            reason
        );
        assert_eq!(encoder.revision, revision);
        assert_eq!(encoder.state.last_hour, Some(0));
        let (_, BaselineRows::Shared(previous)) = encoder.state.tables[TableId::Countries.index()]
            .as_ref()
            .unwrap()
        else {
            panic!()
        };
        assert!(Arc::ptr_eq(previous, &countries));
    }
    // A producer may retain this allocation through a failed spool write,
    // but a replacement encoder has no proof and must emit a full snapshot.
    encoder = FrameEncoder::new();
    let mut recovered = shared_input(source(1, 1));
    recovered.tables[TableId::Countries.index()] =
        TableInput::SharedComplete(Arc::clone(&countries));
    let recovered = encoder.encode_input(recovered, false).unwrap();
    assert!(recovered.keyframe);
    assert_eq!(
        recovered.tables[TableId::Countries.index()],
        TableUpdate::Snapshot((*countries).clone())
    );
}

fn variable_source(branch: BranchId, hour: Hour, value: i64) -> DecodedFrame {
    let mut frame = source(branch, hour);
    *frame.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 7,
            entries: vec![rows::ScriptVariableEntry {
                name: vec![0, 0xff],
                value_raw: value,
            }],
        }]));
    frame
}

#[test]
fn compared_variables_encode_identically_without_replacing_owned_baseline_rows() {
    let mut ordinary = FrameEncoder::new();
    let mut compared = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut previous_name = None;
    for hour in 0..4 {
        let value = if hour < 2 { i64::MIN } else { i64::MAX };
        let source = variable_source(1, hour, value);
        let mut input: FrameInput = source.clone().into();
        if hour == 1 || hour == 3 {
            let (token, previous) = compared
                .comparison_baseline::<tables::ScriptVariables>(1, hour, false)
                .unwrap();
            assert_eq!(
                previous,
                source.complete::<tables::ScriptVariables>().unwrap()
            );
            previous_name = Some(previous[0].entries[0].name.as_ptr());
            input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
        }
        let expected = ordinary.encode(source.clone(), false).unwrap();
        let actual = compared.encode_input(input, false).unwrap();
        assert_eq!(wire(&actual), wire(&expected));
        assert_eq!(decoder.apply(&actual).unwrap(), source);
        if let Some(pointer) = previous_name.take() {
            let (_, baseline) = compared
                .comparison_baseline::<tables::ScriptVariables>(1, hour + 1, false)
                .unwrap();
            assert_eq!(baseline[0].entries[0].name.as_ptr(), pointer);
        }
    }
}

#[test]
fn compared_variables_require_this_encoder_current_branch_hour_and_non_keyframe() {
    let mut encoder = FrameEncoder::new();
    assert!(
        encoder
            .comparison_baseline::<tables::ScriptVariables>(1, 0, false)
            .is_none()
    );
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    assert!(
        encoder
            .comparison_baseline::<tables::Clock>(1, 1, false)
            .is_none()
    );
    for (branch, hour, force) in [
        (2, 1, false),
        (1, 0, false),
        (1, 2, false),
        (1, 1, true),
        (1, 720, false),
    ] {
        assert!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(branch, hour, force)
                .is_none()
        );
    }
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let input = |branch, hour| {
        let mut input: FrameInput = variable_source(branch, hour, 1).into();
        input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
        input
    };
    assert!(encoder.encode_input(input(1, 1), true).is_err());
    assert!(encoder.encode_input(input(2, 1), false).is_err());
    assert!(encoder.encode_input(input(1, 2), false).is_err());
    let mut wrong_table = input(1, 1);
    wrong_table.tables[TableId::Clock.index()] = TableInput::ComparedEqual(token);
    assert!(encoder.encode_input(wrong_table, false).is_err());
    let mut other = FrameEncoder::new();
    other.encode(variable_source(1, 0, 99), false).unwrap();
    assert!(
        other.encode_input(input(1, 1), false).is_err(),
        "another encoder has the same hours but different rows"
    );
    assert!(
        FrameEncoder::new()
            .encode_input(input(1, 1), false)
            .is_err()
    );
    assert!(encoder.encode_input(input(1, 1), false).is_ok());
}

#[test]
fn compared_variables_noncomplete_baselines_clear_and_resume_with_snapshot() {
    for partial in [false, true] {
        let mut encoder = FrameEncoder::new();
        encoder.encode(variable_source(1, 0, 1), false).unwrap();
        let mut source = variable_source(1, 1, 1);
        *source.tables[TableId::ScriptVariables.index()] = if partial {
            TableView::Truncated {
                rows: TableRows::ScriptVariables(Vec::new()),
                missing: 1,
            }
        } else {
            TableView::Unavailable {
                reason: "current read failed".into(),
            }
        };
        encoder.encode(source, false).unwrap();
        assert!(
            encoder
                .comparison_baseline::<tables::ScriptVariables>(1, 2, false)
                .is_none()
        );
        let restored = encoder.encode(variable_source(1, 2, 1), false).unwrap();
        assert!(matches!(
            restored.tables[TableId::ScriptVariables.index()],
            TableUpdate::Snapshot(_)
        ));
    }
}

#[test]
fn compared_variables_token_does_not_survive_branch_round_trip_with_identical_hours() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    encoder.encode(variable_source(2, 0, 2), false).unwrap();
    encoder.encode(variable_source(1, 0, 3), false).unwrap();
    let mut stale: FrameInput = variable_source(1, 1, 1).into();
    stale.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    assert!(encoder.encode_input(stale, false).is_err());
    let fresh = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    assert_ne!(fresh, token);
    let mut current: FrameInput = variable_source(1, 1, 3).into();
    current.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(fresh);
    assert!(encoder.encode_input(current, false).is_ok());
}

#[test]
fn compared_variables_do_not_commit_before_later_table_validation_succeeds() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(variable_source(1, 0, 1), false).unwrap();
    let token = encoder
        .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
        .unwrap()
        .0;
    let mut input: FrameInput = variable_source(1, 1, 1).into();
    input.tables[TableId::ScriptVariables.index()] = TableInput::ComparedEqual(token);
    let mut invalid = input.clone();
    invalid.tables[TableId::CountryIntel.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::Clock(Vec::new())));
    assert!(encoder.encode_input(invalid, false).is_err());
    assert_eq!(
        encoder
            .comparison_baseline::<tables::ScriptVariables>(1, 1, false)
            .unwrap()
            .0,
        token
    );
    assert_eq!(
        encoder.encode_input(input, false).unwrap().tables[TableId::ScriptVariables.index()],
        TableUpdate::Unchanged { baseline: 0 }
    );
}

#[test]
fn row_replacements_match_full_frames_and_keep_untouched_allocations() {
    let mut rows: Vec<_> = (0..4)
        .map(|index| rows::SupplyNodeRow {
            country: 7,
            index,
            path: vec![index, 0, index, u32::MAX],
            ..Default::default()
        })
        .collect();
    let current = |hour, rows: &[rows::SupplyNodeRow]| {
        let mut frame = source(1, hour);
        *frame.tables[TableId::SupplyNodes.index()] =
            TableView::Complete(TableRows::SupplyNodes(rows.to_vec()));
        frame
    };
    let mut sparse = FrameEncoder::new();
    let mut ordinary = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = sparse.encode(current(0, &rows), false).unwrap();
    ordinary.encode(current(0, &rows), false).unwrap();
    decoder.apply(&first).unwrap();
    let (_, original) = sparse
        .comparison_baseline::<tables::SupplyNodes>(1, 1, false)
        .unwrap();
    let allocation = original.as_ptr();
    let untouched_path = original[3].path.as_ptr();
    for hour in 1..=4 {
        let indices = match hour {
            1 => {
                rows[1].available = i64::MIN;
                rows[1].path.push(99);
                vec![1]
            }
            2 => vec![1], // An equal replacement still emits Unchanged.
            3 => {
                rows[0].consumed = i64::MAX;
                rows[2].path.reverse();
                vec![0, 2]
            }
            _ => vec![],
        };
        let (token, _) = sparse
            .comparison_baseline::<tables::SupplyNodes>(1, hour, false)
            .unwrap();
        assert!(token.allows_row_updates());
        let complete = current(hour, &rows);
        let mut input: FrameInput = complete.clone().into();
        input.tables[TableId::SupplyNodes.index()] = TableInput::ComparedChanges {
            token,
            rows: TableRows::SupplyNodes(
                indices.iter().map(|&index| rows[index].clone()).collect(),
            ),
            indices,
        };
        let actual = sparse.encode_bytes(input, false).unwrap();
        let expected = ordinary
            .encode_bytes(complete.clone().into(), false)
            .unwrap();
        assert_eq!(actual.bytes, expected.bytes);
        assert_eq!(decoder.apply(&actual.decode().unwrap()).unwrap(), complete);
        let (_, retained) = sparse
            .comparison_baseline::<tables::SupplyNodes>(1, hour + 1, false)
            .unwrap();
        assert_eq!(retained.as_ptr(), allocation);
        assert_eq!(retained[3].path.as_ptr(), untouched_path);
    }
}

#[test]
fn row_replacements_reject_bad_positions_tokens_and_later_errors_atomically() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0), false).unwrap();
    let (token, old) = encoder
        .comparison_baseline::<tables::Provinces>(1, 1, false)
        .unwrap();
    let original = old.to_vec();
    let mut changed = original[0].clone();
    changed.controller = u32::MAX;
    let patch = |indices, rows| {
        let mut input: FrameInput = source(1, 1).into();
        input.tables[TableId::Provinces.index()] = TableInput::ComparedChanges {
            token,
            indices,
            rows,
        };
        input
    };
    let valid = patch(vec![0], TableRows::Provinces(vec![changed.clone()]));
    let mut cases = vec![
        patch(vec![], TableRows::Provinces(vec![changed.clone()])),
        patch(vec![0, 0], TableRows::Provinces(vec![changed.clone(); 2])),
        patch(
            vec![1, 0],
            TableRows::Provinces(vec![original[1].clone(), changed.clone()]),
        ),
        patch(vec![2], TableRows::Provinces(vec![changed.clone()])),
        patch(vec![1], TableRows::Provinces(vec![changed.clone()])),
        patch(vec![0], TableRows::States(vec![])),
    ];
    let mut later_error = valid.clone();
    let later = TableId::ALL.into_iter().next_back().unwrap();
    assert!(later.index() > TableId::Provinces.index());
    later_error.tables[later.index()] = TableView::Complete(TableRows::Clock(Vec::new())).into();
    cases.push(later_error);
    let mut wrong_branch = valid.clone();
    wrong_branch.branch += 1;
    cases.push(wrong_branch);
    let mut wrong_hour = valid.clone();
    wrong_hour.hour += 1;
    cases.push(wrong_hour);
    for invalid in cases {
        assert!(encoder.encode_input(invalid, false).is_err());
        let (current, retained) = encoder
            .comparison_baseline::<tables::Provinces>(1, 1, false)
            .unwrap();
        assert_eq!(current, token);
        assert_eq!(retained, original);
    }
    assert!(encoder.encode_input(valid.clone(), true).is_err());
    let mut other = FrameEncoder::new();
    other.encode(source(1, 0), false).unwrap();
    assert!(other.encode_input(valid.clone(), false).is_err());
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![changed, original[1].clone()]));
    let actual = encoder.encode_input(valid.clone(), false).unwrap();
    let expected = other.encode(expected, false).unwrap();
    assert_eq!(actual, expected);
    let mut stale = valid;
    stale.hour = 2;
    assert!(encoder.encode_input(stale, false).is_err());
}

#[test]
fn row_replacements_cannot_edit_shared_or_cross_row_validated_baselines() {
    for shared in [false, true] {
        let mut encoder = FrameEncoder::new();
        let first = source(1, 0);
        encoder
            .encode_input(
                if shared {
                    shared_input(first)
                } else {
                    first.into()
                },
                false,
            )
            .unwrap();
        let (token, rows) = encoder
            .comparison_baseline::<tables::Countries>(1, 1, false)
            .unwrap();
        assert!(!token.allows_row_updates());
        let mut input: FrameInput = source(1, 1).into();
        input.tables[TableId::Countries.index()] = TableInput::ComparedChanges {
            token,
            indices: vec![0],
            rows: TableRows::Countries(rows.to_vec()),
        };
        assert!(encoder.encode_input(input, false).is_err());
        let (token, rows) = encoder
            .comparison_baseline::<tables::Provinces>(1, 1, false)
            .unwrap();
        assert_eq!(token.allows_row_updates(), !shared);
        if shared {
            let mut input: FrameInput = source(1, 1).into();
            input.tables[TableId::Provinces.index()] = TableInput::ComparedChanges {
                token,
                indices: vec![0],
                rows: TableRows::Provinces(vec![rows[0].clone()]),
            };
            assert!(encoder.encode_input(input, false).is_err());
        }
        // A staggered table's baseline takes no row replacements.
        let (token, _) = encoder
            .comparison_baseline::<tables::DivisionTemplates>(1, 1, false)
            .unwrap();
        assert!(!token.allows_row_updates());
    }
}

#[test]
fn cadence_marker_carries_identical_validated_rows_without_input_clones() {
    let mut ordinary = FrameEncoder::new();
    let mut carried = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..8 {
        let full = source(1, hour);
        let mut compact = full.clone();
        if hour != 0 {
            for table in TableId::ALL {
                if crate::cadence::is_staggered(table) {
                    assert!(carried.can_carry(table, 1, hour, false));
                    *compact.tables[table.index()] = TableView::Unavailable {
                        reason: crate::cadence::SKIPPED_REASON.into(),
                    };
                }
            }
        }
        let expected = ordinary.encode(full.clone(), false).unwrap();
        let actual = carried.encode(compact, false).unwrap();
        assert_eq!(actual, expected, "hour {hour}");
        assert_eq!(decoder.apply(&actual).unwrap(), full);
    }
}

#[test]
fn cadence_carry_requires_complete_same_branch_non_keyframe_baseline() {
    let mut encoder = FrameEncoder::new();
    let table = TableId::HiredAdvisors;
    assert!(!encoder.can_carry(table, 1, 1, false));
    encoder.encode(source(1, 0), false).unwrap();
    assert!(encoder.can_carry(table, 1, 1, false));
    for (branch, hour, force) in [(2, 1, false), (1, 0, false), (1, 1, true), (1, 720, false)] {
        assert!(!encoder.can_carry(table, branch, hour, force));
    }
    assert!(!encoder.can_carry(TableId::Clock, 1, 1, false));
    let mut missing = source(1, 1);
    *missing.tables[table.index()] = TableView::Unavailable {
        reason: "fault".into(),
    };
    encoder.encode(missing, false).unwrap();
    assert!(!encoder.can_carry(table, 1, 2, false));
    let mut marker = source(1, 2);
    *marker.tables[table.index()] = TableView::Unavailable {
        reason: crate::cadence::SKIPPED_REASON.into(),
    };
    assert!(matches!(
        encoder.encode(marker, false).unwrap().tables[table.index()],
        TableUpdate::Unavailable { .. }
    ));
    let mut partial = source(1, 3);
    let TableView::Complete(rows) = (*partial.tables[table.index()]).clone() else {
        unreachable!()
    };
    *partial.tables[table.index()] = TableView::Truncated { rows, missing: 1 };
    encoder.encode(partial, false).unwrap();
    assert!(!encoder.can_carry(table, 1, 4, false));
}

#[test]
fn cadence_marker_cannot_turn_a_missing_hourly_capture_into_fresh_rows() {
    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0), false).unwrap();
    let mut skipped = source(1, 1);
    for table in [TableId::Characters, TableId::Divisions, TableId::Countries] {
        assert!(!crate::cadence::is_staggered(table));
        assert!(!encoder.can_carry(table, 1, 1, false));
        *skipped.tables[table.index()] = TableView::Unavailable {
            reason: crate::cadence::SKIPPED_REASON.into(),
        };
    }
    let frame = encoder.encode(skipped, false).unwrap();
    for table in [TableId::Characters, TableId::Divisions, TableId::Countries] {
        assert!(matches!(
            &frame.tables[table.index()],
            TableUpdate::Unavailable { reason } if reason == crate::cadence::SKIPPED_REASON
        ));
        assert!(encoder.state.tables[table.index()].is_none());
    }
}

fn source(branch: BranchId, hour: Hour) -> DecodedFrame {
    DecodedFrame {
        branch,
        hour,
        metrics: Metrics {
            total_ns: 4_567_890,
            table_ns: std::array::from_fn(|index| (index as u64 + 1) * 100),
            pool_bytes: 123_456,
            dropped_frames: 2,
        },
        tables: {
            let initial = [
                TableView::Complete(TableRows::Clock(vec![ClockRow {
                    hour,
                    play_time: u64::MAX,
                    play_session_started_at: 123_456,
                    seed: -789,
                    speed: 4,
                    debug_current_ref_id: Some(u32::MAX),
                    unit: Some(4012),
                    order_index: Some(7693),
                    front_index: Some(8230),
                    theatre_index: Some(112),
                    theater_group_index: Some(924),
                    military_deployment_line_index: Some(7841),
                    military_deployment_conveyor_index: Some(744),
                    equipment_variant_index: Some(6295),
                    country_leader_index: Some(1),
                    navy_id: Some(8263),
                    land_combat_id: Some(15_359),
                    multiplayer_random_count: Some(-1_333_963_091),
                }])),
                TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                    id: b"campaign".to_vec(),
                }])),
                TableView::Complete(TableRows::Countries(vec![CountryRow {
                    id: 7,
                    canonical_slot: 1,
                    tag: b"GER".to_vec(),
                    political_power_raw: Some(-9_007_199_254_740_993),
                    command_power_raw: 1,
                    stability_raw: 100_001,
                    war_support_raw: 99_999,
                    convoys_raw: None,
                    peace_conference_participant: 0,
                    manpower_residual: 0,
                }])),
                TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
                    id: 2,
                    canonical_slot: 1,
                }])),
                TableView::Complete(TableRows::Provinces(vec![province(1, 7), province(2, 2)])),
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 7,
                    controller: 2,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }])),
                TableView::Complete(TableRows::Divisions(vec![division(
                    0x1234_5678_0000_0035,
                    300_000,
                    100_001,
                )])),
                TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                    first_country: 2,
                    second_country: 7,
                    start_hour: -12,
                    end_hour: i32::MIN,
                    cancel: 0x80,
                    first_casualties: 9_007_199_254_740_993,
                    second_casualties: i64::MAX,
                    first_unknown_casualties: i64::MIN,
                    second_unknown_casualties: -1,
                    first_was_instigator: 0xfe,
                }])),
                TableView::Complete(TableRows::Factions(vec![faction(FACTION_ID, b"Faction")])),
                TableView::Complete(TableRows::FactionMembers(vec![
                    faction_member(2, FACTION_ID, 1),
                    faction_member(7, FACTION_ID, 0),
                ])),
                TableView::Complete(TableRows::Players(Vec::new())),
                TableView::Complete(TableRows::HumanControllers(Vec::new())),
                TableView::Complete(TableRows::LandCombats(Vec::new())),
                TableView::Complete(TableRows::Fleets(Vec::new())),
                TableView::Complete(TableRows::TaskForces(Vec::new())),
                TableView::Complete(TableRows::Ships(Vec::new())),
                TableView::Complete(TableRows::NavalCombats(Vec::new())),
                TableView::Complete(TableRows::NavalCombatParticipants(Vec::new())),
                TableView::Complete(TableRows::AirWings(Vec::new())),
                TableView::Complete(TableRows::ArmyOrders(Vec::new())),
            ];
            let mut tables = std::array::from_fn(|index| {
                let table = TableId::ALL[index];
                TableView::Complete(
                    rows::sample_extended_table(table, 0)
                        .unwrap_or_else(|| TableRows::empty(table)),
                )
            });
            for (target, source) in tables.iter_mut().zip(initial) {
                *target = source;
            }
            tables.map(Into::into)
        },
    }
}

fn province(id: u32, controller: u32) -> ProvinceRow {
    ProvinceRow {
        id,
        state: 8,
        owner: 7,
        controller,
        victory_points: 50,
        fort_positive: 0,
        move_gate: 0,
        port_sea: 0,
    }
}

fn division(id: u64, hit_points: i64, organisation: i64) -> DivisionRow {
    DivisionRow {
        id,
        roster_owner: 7,
        logical_owner: 2,
        expeditionary_owner: 0,
        province: 1,
        template: 0xfedc_ba98_0000_002b,
        hit_points,
        organisation,
        orders_group: 0x0000_0039_0000_0035,
        reveal_skip: 0,
    }
}

fn faction(id: u64, name: &[u8]) -> FactionRow {
    FactionRow {
        id,
        name: name.to_vec(),
        ..FactionRow::default()
    }
}

fn faction_member(country: u32, faction: u64, ordinal: u32) -> FactionMemberRow {
    FactionMemberRow {
        country,
        faction,
        ordinal,
    }
}

fn change_country(frame: &mut DecodedFrame, value: i64) {
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    let country = &mut rows[0];
    country.command_power_raw = value;
}

fn wire(frame: &Frame) -> Frame {
    let mut bytes = Vec::new();
    frame.encode(&mut bytes).unwrap();
    let mut decoder = Decoder::new(&bytes);
    let result = Frame::decode(&mut decoder).unwrap();
    decoder.finish().unwrap();
    assert_eq!(&result, frame);
    result
}

fn unavailable(view: &TableView, expected: &str) {
    let TableView::Unavailable { reason } = view else {
        panic!("expected unavailable: {view:?}")
    };
    assert!(
        reason.contains(expected),
        "{reason:?} did not contain {expected:?}"
    );
}

#[test]
fn padded_snapshot_counts_are_rejected_before_allocating_or_reading_rows() {
    let claimed = 16_384_u32;
    assert_eq!(TableId::Divisions.minimum_row_bytes(), 50);
    assert_eq!(TableId::Campaign.minimum_row_bytes(), 5);
    for table in TableId::ALL {
        let padding = claimed as usize * table.minimum_row_bytes() - 1;
        let mut bytes = claimed.to_le_bytes().to_vec();
        bytes.resize(bytes.len() + padding, 0);
        // A key-only bound accepts this count before reserving all rows.
        assert_eq!(Decoder::new(&bytes).count(1).unwrap(), claimed as usize);
        let mut input = Decoder::new(&bytes);
        let error = decode_rows(table, &mut input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "collection length exceeds remaining payload"
        );
        assert_eq!(
            input.remaining(),
            padding,
            "read a row before rejecting {table:?}"
        );
    }
}

#[test]
fn changed_row_counts_include_a_selected_value_and_singletons_never_reserve_many_rows() {
    let claimed = 16_384_u32;
    let padding = claimed as usize * 2;
    let mut bytes = claimed.to_le_bytes().to_vec();
    bytes.resize(bytes.len() + padding, 0);
    assert_eq!(Decoder::new(&bytes).count(2).unwrap(), claimed as usize);
    assert_eq!(TableId::Divisions.minimum_delta_bytes(), 4);
    assert_eq!(TableId::Campaign.minimum_delta_bytes(), 6);
    let mut input = Decoder::new(&bytes);
    let error = decode_row_count(
        TableId::Divisions,
        &mut input,
        TableId::Divisions.minimum_delta_bytes(),
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "collection length exceeds remaining payload"
    );
    assert_eq!(input.remaining(), padding);

    for table in [TableId::Clock, TableId::Campaign] {
        // Enough payload for two rows, so the singleton rule is what refuses it.
        let payload = 2 * table.minimum_row_bytes();
        let mut bytes = 2_u32.to_le_bytes().to_vec();
        bytes.resize(4 + payload, 0);
        let mut input = Decoder::new(&bytes);
        let error = decode_rows(table, &mut input).unwrap_err();
        assert_eq!(
            error.to_string(),
            "singleton table cannot contain multiple rows"
        );
        assert_eq!(input.remaining(), payload);
    }
}

#[test]
fn unchanged_advances_each_baseline_and_missing_hours_do_not_clear_it() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in [100, 101, 107, 108] {
        let mut expected = source(1, hour);
        if hour >= 107 {
            change_country(&mut expected, i64::MAX);
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            100 => assert!(
                frame
                    .tables
                    .iter()
                    .all(|table| matches!(table, TableUpdate::Snapshot(_)))
            ),
            101 => assert_eq!(
                frame.tables[TableId::Countries.index()],
                TableUpdate::Unchanged { baseline: 100 }
            ),
            107 => {
                assert!(matches!(
                    frame.tables[TableId::Countries.index()],
                    TableUpdate::Delta { baseline: 101, .. }
                ));
                assert_eq!(
                    frame.tables[TableId::Provinces.index()],
                    TableUpdate::Unchanged { baseline: 101 }
                );
            }
            108 => assert_eq!(
                frame.tables[TableId::Countries.index()],
                TableUpdate::Unchanged { baseline: 107 }
            ),
            _ => unreachable!(),
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn borrowed_advances_and_owned_results_keep_independent_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let initial = source(1, 100);
    let frame = wire(&encoder.encode(initial.clone(), false).unwrap());
    assert_eq!(decoder.advance(&frame).unwrap(), &initial);

    let mut expected = source(1, 101);
    change_country(&mut expected, i64::MAX);
    let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
    let mut owned = decoder.apply(&frame).unwrap();
    assert_eq!(owned, expected);
    for table in [TableId::Campaign, TableId::Countries, TableId::Factions] {
        let TableView::Complete(rows) = &mut *owned.tables[table.index()] else {
            panic!()
        };
        match rows {
            TableRows::Campaign(rows) => rows[0].id.clear(),
            TableRows::Countries(rows) => {
                let row = &mut rows[0];
                row.tag.clear();
                row.political_power_raw = None;
            }
            TableRows::Factions(rows) => rows[0].name.clear(),
            _ => unreachable!(),
        }
    }
    for hour in [107, 108] {
        let mut expected = source(1, hour);
        change_country(&mut expected, i64::MAX);
        let frame = wire(&encoder.encode(expected.clone(), false).unwrap());
        assert_eq!(
            frame.tables[TableId::Countries.index()],
            TableUpdate::Unchanged {
                baseline: if hour == 107 { 101 } else { 107 }
            }
        );
        assert_eq!(decoder.advance(&frame).unwrap(), &expected);
    }
}

#[test]
fn consuming_encoder_moves_vectors_and_decoder_keeps_unchanged_string_allocations() {
    let mut encoder = FrameEncoder::new();
    let first = source(1, 100);
    let input_rows = first.complete::<tables::Countries>().unwrap().as_ptr();
    let initial = encoder.encode(first, false).unwrap();
    let baseline = encoder.state.tables[TableId::Countries.index()]
        .as_ref()
        .unwrap();
    assert_eq!(
        baseline
            .1
            .as_ref()
            .rows::<tables::Countries>()
            .unwrap()
            .as_ptr(),
        input_rows
    );

    let mut decoder = FrameDecoder::new();
    let country = &decoder
        .advance(&initial)
        .unwrap()
        .complete::<tables::Countries>()
        .unwrap()[0];
    let tag = country.tag.as_ptr();
    let mut next = source(1, 101);
    change_country(&mut next, 2);
    let input_rows = next.complete::<tables::Countries>().unwrap().as_ptr();
    let delta = encoder.encode(next, false).unwrap();
    let baseline = encoder.state.tables[TableId::Countries.index()]
        .as_ref()
        .unwrap();
    assert_eq!(
        baseline
            .1
            .as_ref()
            .rows::<tables::Countries>()
            .unwrap()
            .as_ptr(),
        input_rows
    );
    let country = &decoder
        .advance(&delta)
        .unwrap()
        .complete::<tables::Countries>()
        .unwrap()[0];
    assert_eq!(country.command_power_raw, 2);
    assert_eq!(country.tag.as_ptr(), tag);

    let mut unchanged = source(1, 102);
    change_country(&mut unchanged, 2);
    let update = encoder.encode(unchanged, false).unwrap();
    assert!(matches!(
        update.tables[TableId::Countries.index()],
        TableUpdate::Unchanged { .. }
    ));
    let baseline = encoder.state.tables[TableId::Countries.index()]
        .as_ref()
        .unwrap();
    assert_eq!(
        baseline
            .1
            .as_ref()
            .rows::<tables::Countries>()
            .unwrap()
            .as_ptr(),
        input_rows
    );
}

#[test]
fn unavailable_and_truncated_tables_require_their_own_next_snapshot() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = source(1, 0);
    assert_eq!(
        decoder
            .apply(&encoder.encode(first.clone(), false).unwrap())
            .unwrap(),
        first
    );
    let mut failed = source(1, 1);
    *failed.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "political status unreadable".into(),
    };
    *failed.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![province(1, 2)]),
        missing: 42,
    };
    let frame = encoder.encode(failed.clone(), false).unwrap();
    assert!(!frame.keyframe);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), failed);

    let recovered = source(1, 2);
    let frame = encoder.encode(recovered.clone(), false).unwrap();
    for table in [TableId::Countries, TableId::Provinces] {
        assert!(matches!(
            frame.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
    assert!(matches!(
        frame.tables[TableId::Clock.index()],
        TableUpdate::Delta { baseline: 1, .. }
    ));
    assert_eq!(
        frame.tables[TableId::Campaign.index()],
        TableUpdate::Unchanged { baseline: 1 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
    let frame = encoder.encode(source(1, 3).clone(), false).unwrap();
    assert_eq!(
        frame.tables[TableId::Provinces.index()],
        TableUpdate::Unchanged { baseline: 2 }
    );
}

#[test]
fn state_control_deltas_and_failures_keep_separate_baselines_from_provinces() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let state_index = TableId::States.index();
    for hour in 0..=6 {
        let mut expected = source(1, hour);
        if hour > 0 {
            *expected.tables[state_index] =
                TableView::Complete(TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 0,
                    controller: u32::MAX,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }]));
        }
        if hour == 2 {
            *expected.tables[state_index] = TableView::Unavailable {
                reason: "state table unreadable".into(),
            };
        } else if hour == 4 {
            *expected.tables[state_index] = TableView::Truncated {
                rows: TableRows::States(vec![StateRow {
                    id: 8,
                    owner: 0,
                    controller: 2,
                    movement_infrastructure: 0,
                    ..StateRow::default()
                }]),
                missing: 10,
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            0 | 3 | 5 => assert!(matches!(
                frame.tables[state_index],
                TableUpdate::Snapshot(_)
            )),
            1 => {
                let TableUpdate::Delta {
                    baseline,
                    changed,
                    deleted,
                } = &frame.tables[state_index]
                else {
                    panic!("state ownership change did not produce a delta")
                };
                assert_eq!(*baseline, 0);
                assert_eq!(changed.len(), 1);
                assert_eq!(changed[0].mask, 3);
                assert!(deleted.is_empty());
            }
            6 => assert_eq!(
                frame.tables[state_index],
                TableUpdate::Unchanged { baseline: 5 }
            ),
            _ => {}
        }
        if hour > 0 {
            assert_eq!(
                frame.tables[TableId::Provinces.index()],
                TableUpdate::Unchanged { baseline: hour - 1 }
            );
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn division_handles_and_native_health_survive_insert_update_delete_and_dropped_hours() {
    // Equal instance halves with different type halves must remain distinct keys.
    let first_id = 0xfedc_ba98_0000_0035;
    let second_id = 0xfedc_ba98_0000_0036;
    let inserted_id = 0xffff_ffff_0000_0035;
    let index = TableId::Divisions.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 10);
    *first.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(first_id, 300_000, 100_001),
        division(second_id, 300_000, 100_001),
    ]));
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut next = source(1, 11);
    *next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(second_id, i64::MAX, i64::MIN),
        division(inserted_id, 9_007_199_254_740_993, -100_001),
    ]));
    let frame = encoder.encode(next.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[index]
    else {
        panic!("division changes did not produce a delta")
    };
    assert_eq!(*baseline, 10);
    assert_eq!(deleted, &[u128::from(first_id)]);
    assert_eq!(changed.len(), 2);
    assert_eq!(
        (changed[0].key, changed[0].mask),
        (u128::from(second_id), 0b110_0000)
    );
    assert_eq!(
        changed[0]
            .apply(Some(division(second_id, 300_000, 100_001)))
            .unwrap(),
        division(second_id, i64::MAX, i64::MIN)
    );
    assert_eq!(
        (changed[1].key, changed[1].mask),
        (u128::from(inserted_id), TableId::Divisions.full_mask())
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

    let mut resting = source(1, 12);
    resting.tables[index] = next.tables[index].clone();
    let frame = encoder.encode(resting.clone(), false).unwrap();
    assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 11 });
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), resting);

    // The spool's gap control record explains the missing hours; surviving
    // table deltas still refer to the last successfully recorded baseline.
    let mut after_gap = source(1, 19);
    *after_gap.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(second_id, i64::MAX, i64::MIN + 1),
        division(inserted_id, 9_007_199_254_740_993, -100_001),
    ]));
    let frame = encoder.encode(after_gap.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[index]
    else {
        panic!("organisation change did not produce a delta")
    };
    assert_eq!(*baseline, 12);
    assert!(deleted.is_empty());
    assert_eq!(changed.len(), 1);
    assert_eq!(
        (changed[0].key, changed[0].mask),
        (u128::from(second_id), 0b100_0000)
    );
    assert_eq!(changed[0].values, [2]); // Zigzag(+1), without rounding the native value.
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), after_gap);
}

#[test]
fn division_failures_and_partial_keyframes_clear_only_their_own_baseline() {
    let index = TableId::Divisions.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=6 {
        let mut expected = source(1, hour);
        if matches!(hour, 0 | 4) {
            *expected.tables[index] = TableView::Truncated {
                rows: TableRows::Divisions(vec![division(0x1234_5678_0000_0035, 300_000, 100_001)]),
                missing: 17,
            };
        } else if hour == 2 {
            *expected.tables[index] = TableView::Unavailable {
                reason: "division reference unreadable".into(),
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        match hour {
            1 | 3 | 5 => assert!(matches!(frame.tables[index], TableUpdate::Snapshot(_))),
            6 => assert_eq!(frame.tables[index], TableUpdate::Unchanged { baseline: 5 }),
            _ => {}
        }
        if hour > 0 {
            assert_eq!(
                frame.tables[TableId::Provinces.index()],
                TableUpdate::Unchanged { baseline: hour - 1 }
            );
        }
        let actual = decoder.apply(&wire(&frame)).unwrap();
        if hour == 0 {
            unavailable(&actual.tables[index], "17 rows not read");
            expected.tables[index] = actual.tables[index].clone();
        }
        assert_eq!(actual, expected);
    }

    let expected = source(1, 7);
    let mut invalid = encoder.encode(expected.clone(), false).unwrap();
    invalid.tables[index] = TableUpdate::Unchanged { baseline: 999 };
    let actual = decoder.apply(&wire(&invalid)).unwrap();
    unavailable(&actual.tables[index], "baseline at hour 999");
    for table in TableId::ALL
        .into_iter()
        .filter(|table| *table != TableId::Divisions)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let later = encoder.encode(source(1, 8).clone(), false).unwrap();
    unavailable(
        &decoder.apply(&wire(&later)).unwrap().tables[index],
        "baseline at hour 7",
    );
    let recovered = source(1, 9);
    let frame = encoder.encode(recovered.clone(), true).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), recovered);
}

#[test]
fn war_faction_and_membership_failures_recover_independently() {
    let tables = [
        TableId::WarRelations,
        TableId::Factions,
        TableId::FactionMembers,
    ];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=4 {
        let mut expected = source(1, hour);
        if (1..=3).contains(&hour) {
            let table = tables[hour as usize - 1];
            *expected.tables[table.index()] = if table == TableId::Factions {
                TableView::Truncated {
                    rows: TableRows::Factions(vec![faction(FACTION_ID, b"Incomplete name walk")]),
                    missing: 2,
                }
            } else {
                TableView::Unavailable {
                    reason: format!("{table:?} walk unreadable"),
                }
            };
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        for (offset, table) in tables.into_iter().enumerate() {
            let failure_hour = offset as i32 + 1;
            if hour == 0 || hour == failure_hour + 1 {
                assert!(matches!(
                    frame.tables[table.index()],
                    TableUpdate::Snapshot(_)
                ));
            } else if hour != failure_hour {
                assert_eq!(
                    frame.tables[table.index()],
                    TableUpdate::Unchanged { baseline: hour - 1 },
                    "{table:?} lost its baseline when another table failed"
                );
            }
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }

    // A segment's first frame cannot use partial rows from any of these
    // related tables as a baseline, even when all three walks failed.
    let mut partial = source(1, 5);
    for table in tables {
        let TableView::Complete(rows) = &*partial.tables[table.index()] else {
            panic!()
        };
        *partial.tables[table.index()] = TableView::Truncated {
            rows: rows.clone(),
            missing: 3,
        };
    }
    let frame = encoder.encode(partial.clone(), true).unwrap();
    assert!(frame.keyframe);
    let actual = decoder.apply(&wire(&frame)).unwrap();
    for table in tables {
        unavailable(&actual.tables[table.index()], "3 rows not read");
        partial.tables[table.index()] = actual.tables[table.index()].clone();
    }
    assert_eq!(actual, partial);
    let expected = source(1, 6);
    let frame = encoder.encode(expected.clone(), false).unwrap();
    assert!(!frame.keyframe);
    for table in tables {
        assert!(matches!(
            frame.tables[table.index()],
            TableUpdate::Snapshot(_)
        ));
    }
    assert_eq!(
        frame.tables[TableId::Countries.index()],
        TableUpdate::Unchanged { baseline: 5 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn faction_leader_reordering_changes_only_member_ordinals_before_insert_and_delete() {
    let factions = TableId::Factions.index();
    let members = TableId::FactionMembers.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let first = source(1, 0);
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut reordered = source(1, 1);
    *reordered.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
        faction_member(2, FACTION_ID, 0),
        faction_member(7, FACTION_ID, 1),
    ]));
    let frame = encoder.encode(reordered.clone(), false).unwrap();
    assert_eq!(
        frame.tables[factions],
        TableUpdate::Unchanged { baseline: 0 }
    );
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[members]
    else {
        panic!("leader reordering did not produce membership deltas")
    };
    assert_eq!(*baseline, 0);
    assert!(deleted.is_empty());
    assert_eq!(
        changed
            .iter()
            .map(|row| (row.key, row.mask))
            .collect::<Vec<_>>(),
        [(2, 2), (7, 2)]
    );
    assert_eq!(changed[0].values, [1]); // Zigzag(-1): country 2 becomes first.
    assert_eq!(changed[1].values, [2]); // Zigzag(+1): country 7 becomes second.
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), reordered);

    // Dissolve the old faction and form one with a different full-width
    // identity. An empty engine name remains a valid faction row.
    let new_faction = u64::MAX;
    let mut replaced = source(1, 2);
    *replaced.tables[factions] =
        TableView::Complete(TableRows::Factions(vec![faction(new_faction, b"")]));
    *replaced.tables[members] = TableView::Complete(TableRows::FactionMembers(vec![
        faction_member(2, new_faction, 1),
        faction_member(9, new_faction, 0),
    ]));
    let frame = encoder.encode(replaced.clone(), false).unwrap();
    for (table, expected_changes, expected_deleted) in [
        (
            TableId::Factions,
            vec![(u128::from(new_faction), TableId::Factions.full_mask())],
            vec![u128::from(FACTION_ID)],
        ),
        (
            TableId::FactionMembers,
            vec![(2, 3), (9, TableId::FactionMembers.full_mask())],
            vec![7],
        ),
    ] {
        let TableUpdate::Delta {
            baseline,
            changed,
            deleted,
        } = &frame.tables[table.index()]
        else {
            panic!("faction replacement did not produce a {table:?} delta")
        };
        assert_eq!(*baseline, 1);
        assert_eq!(
            changed
                .iter()
                .map(|row| (row.key, row.mask))
                .collect::<Vec<_>>(),
            expected_changes
        );
        assert_eq!(deleted, &expected_deleted);
    }
    assert_eq!(
        frame.tables[TableId::WarRelations.index()],
        TableUpdate::Unchanged { baseline: 1 }
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), replaced);
}

#[test]
fn empty_diplomacy_tables_are_complete_baselines_for_insertions_and_deletions() {
    let tables = [
        TableId::WarRelations,
        TableId::Factions,
        TableId::FactionMembers,
    ];
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for hour in 0..=5 {
        let mut expected = source(1, hour);
        if matches!(hour, 0 | 1 | 3 | 4) {
            for table in tables {
                *expected.tables[table.index()] = TableView::Complete(TableRows::empty(table));
            }
        }
        let frame = encoder.encode(expected.clone(), false).unwrap();
        for table in tables {
            let update = &frame.tables[table.index()];
            match hour {
                0 => assert_eq!(update, &TableUpdate::Snapshot(TableRows::empty(table))),
                1 | 4 => assert_eq!(update, &TableUpdate::Unchanged { baseline: hour - 1 }),
                2 | 3 | 5 => {
                    let TableUpdate::Delta {
                        baseline,
                        changed,
                        deleted,
                    } = update
                    else {
                        panic!("{table:?} lost its complete empty baseline")
                    };
                    assert_eq!(*baseline, hour - 1);
                    let populated = source(1, hour);
                    let TableView::Complete(rows) = &*populated.tables[table.index()] else {
                        panic!()
                    };
                    if hour == 3 {
                        assert!(changed.is_empty());
                        assert_eq!(
                            deleted,
                            &rows.iter().map(|row| row.key()).collect::<Vec<_>>()
                        );
                    } else {
                        assert!(deleted.is_empty());
                        assert_eq!(changed.len(), rows.len());
                        assert!(changed.iter().all(|row| row.mask == table.full_mask()));
                    }
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn key_difference_contexts_restart_for_changed_deleted_and_each_frame() {
    let table = TableId::Divisions;
    let index = table.index();
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 0);
    *first.tables[index] = TableView::Complete(TableRows::Divisions(
        [0, 1, u64::MAX - 1, u64::MAX]
            .into_iter()
            .map(|key| division(key, 300_000, 100_001))
            .collect(),
    ));
    let frame = encoder.encode(first.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), first);

    let mut next = source(1, 1);
    *next.tables[index] = TableView::Complete(TableRows::Divisions(vec![
        division(0, 300_001, 100_001),
        division(2, 300_000, 100_001),
        division(u64::MAX, 300_000, 100_002),
    ]));
    let frame = encoder.encode(next.clone(), false).unwrap();
    let mut bytes = Vec::new();
    let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
    let header_size = bytes.len() - sizes.iter().sum::<usize>();
    let table_start = header_size + sizes[..index].iter().sum::<usize>();
    let mut input = Decoder::new(&bytes[table_start..table_start + sizes[index]]);
    assert_eq!(input.u8().unwrap(), 1);
    assert_eq!(input.i32().unwrap(), 0);
    assert_eq!(input.count(1 + table.mask_width()).unwrap(), 3);
    for (previous_key, expected_key) in [(0, 0), (0, 2), (2, u128::from(u64::MAX))] {
        assert_eq!(
            rows::decode_delta(table, previous_key, &mut input)
                .unwrap()
                .key,
            expected_key
        );
    }
    assert_eq!(input.count(1).unwrap(), 2);
    // Deletions start again at zero, after the changed list reached u64::MAX.
    assert_eq!(input.varint().unwrap(), 1);
    assert_eq!(input.varint().unwrap(), u128::from(u64::MAX - 2));
    input.finish().unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), next);

    let mut empty = source(1, 2);
    *empty.tables[index] = TableView::Complete(TableRows::empty(table));
    let frame = encoder.encode(empty.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), empty);
    let mut inserted = source(1, 3);
    *inserted.tables[index] =
        TableView::Complete(TableRows::Divisions(vec![division(0, i64::MIN, i64::MAX)]));
    let frame = encoder.encode(inserted.clone(), false).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), inserted);
}

#[test]
fn malformed_key_differences_cannot_wrap_snapshot_or_deleted_keys() {
    let table = TableId::Divisions;
    let mut bytes = Vec::new();
    put_count(&mut bytes, 2).unwrap();
    rows::encode_row(&division(u64::MAX, 1, 2), 0, &mut bytes).unwrap();
    // This independently encoded +1 follows u64::MAX in the snapshot list.
    rows::encode_row(&division(1, 3, 4), 0, &mut bytes).unwrap();
    assert!(decode_rows(table, &mut Decoder::new(&bytes)).is_err());

    let mut encoder = FrameEncoder::new();
    encoder.encode(source(1, 0).clone(), false).unwrap();
    let frame = encoder.encode(source(1, 1).clone(), false).unwrap();
    bytes.clear();
    let sizes = frame.encode_with_sizes(&mut bytes).unwrap();
    let table_start =
        bytes.len() - sizes.iter().sum::<usize>() + sizes[..table.index()].iter().sum::<usize>();
    bytes.truncate(table_start);
    bytes.push(1); // Delta.
    bytes.extend_from_slice(&0_i32.to_le_bytes());
    put_count(&mut bytes, 0).unwrap();
    put_count(&mut bytes, 2).unwrap();
    crate::codec::put_varint(&mut bytes, u128::from(u64::MAX));
    crate::codec::put_varint(&mut bytes, 1);
    assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
}

#[test]
fn direct_delta_application_rejects_narrow_key_overflow_and_nonzero_singleton_keys() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0).clone(), false).unwrap();
    let expected = source(1, 1);
    let next = encoder.encode(expected.clone(), false).unwrap();
    for (table, invalid_key) in [
        (TableId::Provinces, u128::from(u32::MAX) + 1),
        (TableId::Campaign, 1),
    ] {
        for delete in [false, true] {
            let mut bad = next.clone();
            let TableView::Complete(values) = &*expected.tables[table.index()] else {
                panic!()
            };
            let mut change = match values {
                TableRows::Provinces(rows) => rows[0].delta(None),
                TableRows::Campaign(rows) => rows[0].delta(None),
                _ => unreachable!(),
            }
            .unwrap()
            .unwrap();
            change.key = invalid_key;
            bad.tables[table.index()] = TableUpdate::Delta {
                baseline: 0,
                changed: if delete { vec![] } else { vec![change] },
                deleted: if delete { vec![invalid_key] } else { vec![] },
            };
            let reason = table.check_key(invalid_key).unwrap_err().to_string();
            assert_eq!(bad.encode(&mut Vec::new()).unwrap_err().to_string(), reason);
            let mut decoder = FrameDecoder::new();
            decoder.apply(&first).unwrap();
            let actual = decoder.apply(&bad).unwrap();
            unavailable(&actual.tables[table.index()], &reason);
            for other in TableId::ALL.into_iter().filter(|other| *other != table) {
                assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
            }
            let later = FrameEncoder::new()
                .encode(source(1, 2).clone(), false)
                .unwrap();
            assert_eq!(decoder.apply(&later).unwrap(), source(1, 2));
        }
    }
}

#[test]
fn truncated_keyframe_names_missing_rows_and_never_supplies_a_baseline() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut expected = source(1, 0);
    *expected.tables[TableId::Provinces.index()] = TableView::Truncated {
        rows: TableRows::Provinces(vec![province(1, 7)]),
        missing: 12,
    };
    let frame = encoder.encode(expected.clone(), false).unwrap();
    assert!(frame.keyframe);
    assert!(frame.tables.iter().all(|t| matches!(
        t,
        TableUpdate::Snapshot(_) | TableUpdate::Unavailable { .. }
    )));
    let actual = decoder.apply(&wire(&frame)).unwrap();
    unavailable(
        &actual.tables[TableId::Provinces.index()],
        "12 rows not read",
    );
    for table in TableId::ALL
        .into_iter()
        .filter(|t| *t != TableId::Provinces)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let available = source(1, 1);
    let frame = encoder.encode(available.clone(), false).unwrap();
    assert!(matches!(
        frame.tables[TableId::Provinces.index()],
        TableUpdate::Snapshot(_)
    ));
    assert_eq!(decoder.apply(&frame).unwrap(), available);
}

#[test]
fn keyframes_follow_elapsed_hours_and_reset_on_segments_and_branches() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, keyframe) in [
        (-10, true),
        (709, false),
        (710, true),
        (1429, false),
        (1430, true),
    ] {
        let expected = source(1, hour);
        let frame = encoder.encode(expected.clone(), false).unwrap();
        assert_eq!(frame.keyframe, keyframe, "hour {hour}");
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
        if keyframe {
            assert_eq!(FrameDecoder::new().apply(&frame).unwrap(), expected);
        }
    }
    let expected = source(1, 1431);
    let segment = encoder.encode(expected.clone(), true).unwrap();
    assert!(segment.keyframe);
    assert_eq!(
        FrameDecoder::new().apply(&wire(&segment)).unwrap(),
        expected
    );
    assert_eq!(decoder.apply(&segment).unwrap(), expected);
    for (branch, hour) in [(2, 1431), (3, -50)] {
        let mut expected = source(branch, hour);
        change_country(&mut expected, -55);
        let frame = encoder.encode(expected.clone(), false).unwrap();
        assert!(frame.keyframe);
        assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
    }
}

#[test]
fn missing_baseline_isolates_one_table_and_a_snapshot_restores_it() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    decoder
        .apply(&encoder.encode(source(1, 0).clone(), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    change_country(&mut expected, -123_456_789);
    let mut frame = encoder.encode(expected.clone(), false).unwrap();
    frame.tables[TableId::Provinces.index()] = TableUpdate::Unchanged { baseline: 999 };
    let actual = decoder.apply(&wire(&frame)).unwrap();
    unavailable(
        &actual.tables[TableId::Provinces.index()],
        "baseline at hour 999",
    );
    for table in TableId::ALL
        .into_iter()
        .filter(|t| *t != TableId::Provinces)
    {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
    }
    let frame = encoder.encode(source(1, 2).clone(), false).unwrap();
    unavailable(
        &decoder.apply(&frame).unwrap().tables[TableId::Provinces.index()],
        "baseline at hour 1",
    );
    let expected = source(1, 3);
    let frame = encoder.encode(expected.clone(), true).unwrap();
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn insertion_deletion_and_field_updates_reconstruct_the_direct_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    decoder
        .apply(&encoder.encode(source(1, 0).clone(), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(2, 99), province(3, 0)]));
    let frame = encoder.encode(expected.clone(), false).unwrap();
    let TableUpdate::Delta {
        baseline,
        changed,
        deleted,
    } = &frame.tables[TableId::Provinces.index()]
    else {
        panic!()
    };
    assert_eq!(*baseline, 0);
    assert_eq!(deleted, &[1]);
    assert_eq!(
        changed.iter().map(|r| (r.key, r.mask)).collect::<Vec<_>>(),
        [(2, 4), (3, TableId::Provinces.full_mask())]
    );
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn sorted_delta_merge_handles_insertions_around_deleted_and_retained_rows() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut first = source(1, 0);
    *first.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(2, 2),
        province(4, 4),
        province(6, 6),
    ]));
    decoder
        .apply(&encoder.encode(first.clone(), false).unwrap())
        .unwrap();
    let mut expected = source(1, 1);
    *expected.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(vec![
        province(1, 1),
        province(4, 99),
        province(5, 5),
        province(7, 7),
    ]));
    let frame = encoder.encode(expected.clone(), false).unwrap();
    let TableUpdate::Delta {
        changed, deleted, ..
    } = &frame.tables[TableId::Provinces.index()]
    else {
        panic!()
    };
    assert_eq!(
        changed.iter().map(|row| row.key).collect::<Vec<_>>(),
        [1, 4, 5, 7]
    );
    assert_eq!(deleted, &[2, 6]);
    assert_eq!(decoder.apply(&wire(&frame)).unwrap(), expected);
}

#[test]
fn invalid_delta_operations_do_not_damage_other_tables_or_retain_partial_rows() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0).clone(), false).unwrap();
    let next_source = source(1, 1);
    let next = encoder.encode(next_source.clone(), false).unwrap();
    let later = encoder.encode(source(1, 2).clone(), false).unwrap();
    let change = province(2, 99)
        .delta(Some(&province(2, 2)))
        .unwrap()
        .unwrap();
    let cases = [
        (vec![change.clone()], vec![2], "changes and deletes"),
        (
            vec![change.clone(), change.clone()],
            vec![],
            "unique and ascending",
        ),
        (vec![], vec![1, 1], "unique and ascending"),
        (vec![], vec![1, 999], "absent key"),
        (
            vec![DeltaRow {
                key: 3,
                mask: 4,
                values: vec![198, 1], // Zigzag(+99).
            }],
            vec![1],
            "requires every field",
        ),
        (
            vec![DeltaRow {
                mask: 1 << TableId::Provinces.field_count(),
                ..change
            }],
            vec![],
            "unknown bits",
        ),
    ];
    for (changed, deleted, reason) in cases {
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        let mut bad = next.clone();
        bad.tables[TableId::Provinces.index()] = TableUpdate::Delta {
            baseline: 0,
            changed,
            deleted,
        };
        let actual = decoder.apply(&bad).unwrap();
        unavailable(&actual.tables[TableId::Provinces.index()], reason);
        for table in TableId::ALL
            .into_iter()
            .filter(|t| *t != TableId::Provinces)
        {
            assert_eq!(
                actual.tables[table.index()],
                next_source.tables[table.index()]
            );
        }
        let actual = decoder.apply(&later).unwrap();
        unavailable(
            &actual.tables[TableId::Provinces.index()],
            "baseline at hour 1",
        );
    }
    let mut empty_reason = next;
    empty_reason.tables[TableId::Provinces.index()] = TableUpdate::Unavailable {
        reason: String::new(),
    };
    let mut decoder = FrameDecoder::new();
    decoder.apply(&first).unwrap();
    unavailable(
        &decoder.apply(&empty_reason).unwrap().tables[TableId::Provinces.index()],
        "needs a reason",
    );
}

#[test]
fn invalid_input_and_keyframes_are_rejected_without_advancing_state() {
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let initial = encoder.encode(source(1, 0).clone(), false).unwrap();
    decoder.apply(&initial).unwrap();
    assert!(encoder.encode(source(1, 0).clone(), false).is_err());
    assert!(decoder.apply(&initial).is_err());
    let mut invalid_source = source(1, 1);
    change_country(&mut invalid_source, i64::MAX);
    *invalid_source.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(2, 2), province(1, 7)]));
    assert!(encoder.encode(invalid_source.clone(), false).is_err());
    let mut invalid_last_source = source(1, 1);
    change_country(&mut invalid_last_source, i64::MIN);
    *invalid_last_source.tables[TableId::Provinces.index()] =
        TableView::Complete(TableRows::Provinces(vec![province(1, 7)]));
    *invalid_last_source.tables[TableId::ArmyOrders.index()] =
        TableView::Complete(TableRows::ArmyOrders(vec![
            crate::rows::ArmyOrderRow::default(),
            crate::rows::ArmyOrderRow::default(),
        ]));
    assert!(encoder.encode(invalid_last_source.clone(), false).is_err());
    let expected = source(1, 1);
    let valid = encoder.encode(expected.clone(), false).unwrap();
    for table in TableId::ALL.into_iter().filter(|id| *id != TableId::Clock) {
        assert_eq!(
            valid.tables[table.index()],
            TableUpdate::Unchanged { baseline: 0 },
            "{table:?} changed its baseline before the final table was validated"
        );
    }
    let mut invalid_keyframe = valid.clone();
    invalid_keyframe.keyframe = true;
    assert!(invalid_keyframe.encode(&mut Vec::new()).is_err());
    assert!(decoder.apply(&invalid_keyframe).is_err());
    let mut invalid_last_table = FrameEncoder::new().encode(expected.clone(), false).unwrap();
    invalid_last_table.tables[TableId::ArmyOrders.index()] = TableUpdate::Unchanged { baseline: 0 };
    assert!(decoder.apply(&invalid_last_table).is_err());
    let mut bytes = Vec::new();
    valid.encode(&mut bytes).unwrap();
    bytes[12] = 1; // The keyframe byte follows the u64 branch and i32 hour.
    assert!(Frame::decode(&mut Decoder::new(&bytes)).is_err());
    assert_eq!(decoder.apply(&wire(&valid)).unwrap(), expected);
}

#[test]
fn complete_singletons_and_clock_hours_are_validated_without_losing_other_tables() {
    let expected = source(1, 0);
    let valid = FrameEncoder::new().encode(expected.clone(), false).unwrap();
    let wrong_clock = ClockRow {
        hour: 99,
        ..ClockRow::default()
    };
    for (table, rows, reason) in [
        (TableId::Clock, TableRows::Clock(vec![]), "exactly one row"),
        (
            TableId::Campaign,
            TableRows::Campaign(vec![]),
            "exactly one row",
        ),
        (
            TableId::SunkShips,
            TableRows::SunkShips(vec![]),
            "exactly one row",
        ),
        (
            TableId::Clock,
            TableRows::Clock(vec![wrong_clock]),
            "hour differs",
        ),
    ] {
        let mut bad_source = expected.clone();
        *bad_source.tables[table.index()] = TableView::Complete(rows.clone());
        assert!(
            FrameEncoder::new()
                .encode(bad_source.clone(), false)
                .is_err()
        );
        let mut bad_frame = valid.clone();
        bad_frame.tables[table.index()] = TableUpdate::Snapshot(rows);
        assert!(bad_frame.encode(&mut Vec::new()).is_err());
        let actual = FrameDecoder::new().apply(&bad_frame).unwrap();
        unavailable(&actual.tables[table.index()], reason);
        for other in TableId::ALL.into_iter().filter(|other| *other != table) {
            assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
        }
    }
}

#[test]
fn singleton_invariants_apply_after_deltas_and_unchanged_updates() {
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(source(1, 0).clone(), false).unwrap();
    let expected = source(1, 1);
    let next = encoder.encode(expected.clone(), false).unwrap();
    for (table, update, reason) in [
        (
            TableId::Clock,
            TableUpdate::Delta {
                baseline: 0,
                changed: vec![DeltaRow {
                    key: 0,
                    mask: 1,
                    values: vec![198, 1], // Zigzag(+99) from the hour-zero baseline.
                }],
                deleted: vec![],
            },
            "hour differs",
        ),
        (
            TableId::Clock,
            TableUpdate::Unchanged { baseline: 0 },
            "hour differs",
        ),
        (
            TableId::Campaign,
            TableUpdate::Delta {
                baseline: 0,
                changed: vec![],
                deleted: vec![0],
            },
            "exactly one row",
        ),
    ] {
        let mut decoder = FrameDecoder::new();
        decoder.apply(&first).unwrap();
        let mut bad = next.clone();
        bad.tables[table.index()] = update;
        let actual = decoder.apply(&wire(&bad)).unwrap();
        unavailable(&actual.tables[table.index()], reason);
        for other in TableId::ALL.into_iter().filter(|other| *other != table) {
            assert_eq!(actual.tables[other.index()], expected.tables[other.index()]);
        }
    }
}

#[test]
fn duplicate_canonical_country_slots_are_ambiguous_in_snapshots_and_deltas() {
    let first_source = source(1, 0);
    let TableView::Complete(TableRows::Countries(countries)) =
        &*first_source.tables[TableId::Countries.index()]
    else {
        panic!()
    };
    let country = &countries[0];
    let mut alias = country.clone();
    alias.id = 9;
    let duplicate = TableRows::Countries(vec![countries[0].clone(), alias.clone()]);
    for view in [
        TableView::Complete(duplicate.clone()),
        TableView::Truncated {
            rows: duplicate,
            missing: 1,
        },
    ] {
        let mut bad = first_source.clone();
        *bad.tables[TableId::Countries.index()] = view;
        assert!(FrameEncoder::new().encode(bad.clone(), false).is_err());
    }
    let mut encoder = FrameEncoder::new();
    let first = encoder.encode(first_source.clone(), false).unwrap();
    let expected = source(1, 1);
    let mut next = encoder.encode(expected.clone(), false).unwrap();
    next.tables[TableId::Countries.index()] = TableUpdate::Delta {
        baseline: 0,
        changed: vec![alias.delta(None).unwrap().unwrap()],
        deleted: vec![],
    };
    let mut decoder = FrameDecoder::new();
    decoder.apply(&first).unwrap();
    let actual = decoder.apply(&wire(&next)).unwrap();
    unavailable(
        &actual.tables[TableId::Countries.index()],
        "duplicate canonical slots",
    );
    assert_eq!(
        actual.tables[TableId::CountryLinks.index()],
        expected.tables[TableId::CountryLinks.index()]
    );
    assert_eq!(
        actual.tables[TableId::Provinces.index()],
        expected.tables[TableId::Provinces.index()]
    );
}
