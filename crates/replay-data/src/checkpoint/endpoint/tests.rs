use super::*;
use crate::codec::Decoder;
use std::io;
use crate::rows::{self, CountryRow, EndpointRows, Recorded, TableId, TableRows};

fn canonical(rows: &TableRows) -> Vec<u8> {
    let mut out = Vec::new();
    rows.encode_checkpoint_rows(&mut out).unwrap();
    out
}


#[test]
fn sparse_masks_follow_the_recording_schema_for_every_registered_table() {
    for table in TableId::ALL {
        let source =
            rows::sample_extended_table(table, 7).unwrap_or_else(|| TableRows::one_default(table));
        source.check_endpoint_sparse_masks();
    }
}

#[test]
fn endpoint_fields_cover_every_registered_table_and_every_field() {
    for table in TableId::ALL {
        let mut source =
            rows::sample_extended_table(table, 7).unwrap_or_else(|| TableRows::one_default(table));
        if let TableRows::EventFeedHistory(rows) = &mut source {
            for row in rows {
                row.coverage = crate::event_feed_rows::EventFeedCoverage::Partial;
            }
        }
        source.round_to_display();
        assert!(!source.is_empty(), "{table:?}");
        assert!(table.schema().len() <= 64, "{table:?}");
        let expected = canonical(&source);
        let edits: Vec<_> = source
            .iter()
            .map(|row| (row.key(), full_mask(table.schema().len()), true))
            .collect();
        let mut bytes = Vec::new();
        source.encode_endpoint_rows(&edits, &mut bytes).unwrap();
        let mut input = Decoder::new(&bytes);
        let edits = table.decode_endpoint_rows(&mut input).unwrap();
        input.finish().unwrap();
        let actual = TableRows::empty(table)
            .apply_endpoint_rows(edits, &[])
            .unwrap();
        assert_eq!(
            canonical(&actual),
            expected,
            "{table:?} full typed insertion"
        );
        for field in 0..table.schema().len() {
            assert!(
                source.endpoint_field_round_trip(1u64 << field).unwrap(),
                "{table:?} field {field} installation"
            );
            let edits: Vec<_> = source
                .iter()
                .map(|row| (row.key(), 1u64 << field, false))
                .collect();
            bytes.clear();
            source.encode_endpoint_rows(&edits, &mut bytes).unwrap();
            let mut input = Decoder::new(&bytes);
            let edits = table.decode_endpoint_rows(&mut input).unwrap();
            input.finish().unwrap();
            let actual = source
                .try_clone_validated()
                .unwrap()
                .apply_endpoint_rows(edits, &[])
                .unwrap();
            assert_eq!(canonical(&actual), expected, "{table:?} field {field}");
        }
    }
}

#[test]
fn endpoint_assignment_is_absolute_preserves_recorded_absence_and_held_rows() {
    let old = CountryRow {
        id: u32::MAX,
        canonical_slot: 4,
        tag: vec![0, 255],
        command_power_raw: i64::MAX,
        border_land_doctrine_level: Recorded::Value(None),
        ..Default::default()
    };
    let next = CountryRow {
        tag: vec![255, 0, 3],
        command_power_raw: i64::MIN,
        border_land_doctrine_level: Recorded::NotRecorded,
        ..old
    };
    let mask = (1 << 1) | (1 << 3) | (1 << 9);
    let current = TableRows::Countries(vec![next.clone()]);
    let mut bytes = Vec::new();
    current
        .encode_endpoint_rows(&[(u128::from(next.id), mask, false)], &mut bytes)
        .unwrap();
    let edits = TableId::Countries
        .decode_endpoint_rows(&mut Decoder::new(&bytes))
        .unwrap();
    let baseline = TableRows::Countries(vec![old.clone()]);
    let actual = baseline
        .try_clone_validated()
        .unwrap()
        .apply_endpoint_rows(edits, &[])
        .unwrap();
    assert_eq!(actual, current);
    assert_eq!(baseline, TableRows::Countries(vec![old]));
    assert_eq!(actual, TableRows::Countries(vec![next]));
}

#[test]
fn endpoint_field_refusals_are_bounded_and_validate_membership_masks_and_keys() {
    let table = TableId::Countries;
    let row = CountryRow {
        id: 1,
        canonical_slot: 1,
        ..Default::default()
    };
    let rows = TableRows::Countries(vec![row.clone()]);
    for (edit, message) in [
        (
            (1, 0, false),
            "endpoint field mask is empty or has unknown bits",
        ),
        (
            (1, 1 << 63, false),
            "endpoint field mask is empty or has unknown bits",
        ),
        ((1, 1, true), "endpoint insertion requires every field"),
        ((1 << 32, 1, false), "endpoint key exceeds table width"),
    ] {
        assert_eq!(
            rows.encode_endpoint_rows(&[edit], &mut Vec::new())
                .unwrap_err()
                .to_string(),
            message
        );
    }
    let edit = |key| Edit {
        key,
        mask: 1,
        replace: false,
    };
    for (edits, deleted, message) in [
        (
            vec![edit(1), edit(1)],
            vec![],
            "endpoint keys must be unique and ascending",
        ),
        (
            vec![edit(1)],
            vec![1],
            "endpoint changed and deleted keys overlap",
        ),
        (vec![edit(2)], vec![], "endpoint patch has no row baseline"),
        (
            vec![],
            vec![1, 1],
            "endpoint deleted keys must be unique and ascending",
        ),
        (
            vec![],
            vec![1 << 32],
            "endpoint deleted key exceeds table width",
        ),
    ] {
        assert_eq!(
            rows.try_clone_validated()
                .unwrap()
                .apply_endpoint_rows(
                    EndpointRows {
                        rows: TableRows::Countries(vec![row.clone(); edits.len()]),
                        edits,
                    },
                    &deleted,
                )
                .unwrap_err()
                .to_string(),
            message
        );
    }
    let mut bytes = Vec::new();
    rows.encode_endpoint_rows(&[(1, full_mask(table.schema().len()), true)], &mut bytes)
        .unwrap();
    for cut in 0..bytes.len() {
        assert!(
            table
                .decode_endpoint_rows(&mut Decoder::new(&bytes[..cut]))
                .is_err(),
            "cut {cut}"
        );
    }
    assert!(
        table
            .decode_endpoint_rows(&mut Decoder::new(&u32::MAX.to_le_bytes()))
            .is_err()
    );
}

#[test]
fn endpoint_install_refuses_a_replacement_with_a_different_row_key() {
    let before = TableRows::Countries(vec![CountryRow {
        id: 1,
        ..Default::default()
    }]);
    let edits = EndpointRows {
        rows: TableRows::Countries(vec![CountryRow {
            id: 3,
            ..Default::default()
        }]),
        edits: vec![Edit {
            key: 2,
            mask: full_mask(TableId::Countries.schema().len()),
            replace: true,
        }],
    };
    assert_eq!(
        before
            .apply_endpoint_rows(edits, &[])
            .unwrap_err()
            .to_string(),
        "endpoint row key differs from edit key"
    );
}

#[test]
fn endpoint_owned_payload_validates_dispatch_and_header_count() {
    fn send_sync<T: Send + Sync>() {}
    send_sync::<EndpointRows>();
    let rows = TableRows::Countries(vec![CountryRow {
        id: 1,
        ..Default::default()
    }]);
    let mut bytes = Vec::new();
    rows.encode_endpoint_rows(
        &[(1, full_mask(TableId::Countries.schema().len()), true)],
        &mut bytes,
    )
    .unwrap();
    let edits = TableId::Countries
        .decode_endpoint_rows(&mut Decoder::new(&bytes))
        .unwrap();
    assert_eq!(edits.len(), 1);
    assert!(!edits.is_empty());
    assert_eq!(
        TableRows::empty(TableId::Divisions)
            .apply_endpoint_rows(edits, &[])
            .unwrap_err()
            .to_string(),
        "endpoint row table mismatch",
    );
    let edits = EndpointRows {
        rows,
        edits: Vec::new(),
    };
    assert_eq!(
        TableRows::empty(TableId::Countries)
            .apply_endpoint_rows(edits, &[])
            .unwrap_err()
            .to_string(),
        "endpoint row and edit counts differ",
    );
}

#[test]
fn endpoint_model_witness_masks_install_only_the_named_current_schema_field() {
    use crate::rows::{DivisionCamouflage, DivisionModelInputRow};
    let before = DivisionModelInputRow {
        division: u64::MAX,
        model_number: None,
        camouflage: None,
        exercising: None,
    };
    let next = DivisionModelInputRow {
        division: u64::MAX,
        model_number: Some(i32::MIN),
        camouflage: Some(DivisionCamouflage {
            category: Some(u32::MAX),
        }),
        exercising: Some(u8::MAX),
    };
    for field in 0..3 {
        let mut expected = before.clone();
        match field {
            0 => expected.model_number = next.model_number,
            1 => expected.camouflage = next.camouflage.clone(),
            2 => expected.exercising = next.exercising,
            _ => unreachable!(),
        }
        let mut bytes = Vec::new();
        TableRows::DivisionModelInputs(vec![next.clone()])
            .encode_endpoint_rows(
                &[(u128::from(next.division), 1 << field, false)],
                &mut bytes,
            )
            .unwrap();
        let mut input = Decoder::new(&bytes);
        let edits = TableId::DivisionModelInputs
            .decode_endpoint_rows(&mut input)
            .unwrap();
        input.finish().unwrap();
        let actual = TableRows::DivisionModelInputs(vec![before.clone()])
            .apply_endpoint_rows(edits, &[])
            .unwrap();
        assert_eq!(
            actual,
            TableRows::DivisionModelInputs(vec![expected]),
            "field {field}"
        );
    }
}

#[test]
fn endpoint_delete_refuses_a_key_absent_from_the_emitted_baseline() {
    for baseline in [
        Vec::new(),
        vec![CountryRow {
            id: 1,
            ..Default::default()
        }],
    ] {
        let edits = EndpointRows {
            rows: TableRows::Countries(Vec::new()),
            edits: Vec::new(),
        };
        let error = TableRows::Countries(baseline)
            .apply_endpoint_rows(edits, &[2])
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "endpoint deletes an absent key");
    }
}

#[test]
fn endpoint_missing_delete_is_not_hidden_by_a_valid_delete_and_patch() {
    let baseline = TableRows::Countries(vec![
        CountryRow {
            id: 1,
            ..Default::default()
        },
        CountryRow {
            id: 3,
            ..Default::default()
        },
    ]);
    let source = TableRows::Countries(vec![CountryRow {
        id: 3,
        command_power_raw: 77,
        ..Default::default()
    }]);
    let mut bytes = Vec::new();
    source
        .encode_endpoint_rows(&[(3, 1 << 3, false)], &mut bytes)
        .unwrap();
    let mut input = Decoder::new(&bytes);
    let edits = TableId::Countries.decode_endpoint_rows(&mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(
        baseline
            .apply_endpoint_rows(edits, &[1, 2])
            .unwrap_err()
            .to_string(),
        "endpoint deletes an absent key"
    );
}
