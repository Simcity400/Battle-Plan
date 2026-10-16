use super::*;
use replay::frame::{Metrics, SharedView, TABLE_COUNT};
use replay::rows::{CampaignRow, ClockRow, CountryRow, DivisionStatusRow, Recorded, TableRows};

const SELECTED: &[TableId] = &[
    TableId::Campaign,
    TableId::Clock,
    TableId::Countries,
    TableId::DivisionStatus,
];

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 71,
        hour: 300,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "Table not selected for reconstruction".into(),
            }
            .into()
        }),
    };
    frame.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(vec![CampaignRow { id: vec![0, 255] }])).into();
    frame.tables[TableId::Clock.index()] = TableView::Complete(TableRows::Clock(vec![ClockRow {
        hour: 300,
        ..Default::default()
    }]))
    .into();
    frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(
        (1..=5_000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: id.to_string().into_bytes(),
                political_power_raw: Some(i64::MAX - i64::from(id)),
                ..Default::default()
            })
            .collect(),
    ))
    .into();
    frame.tables[TableId::DivisionStatus.index()] = TableView::Truncated {
        rows: TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: u64::MAX,
            maximum_hit_points: i64::MAX,
            maximum_organisation: i64::MIN,
            border_combat_stats: Recorded::NotRecorded,
            name: Recorded::Value(Some(vec![0, 255, 1])),
            ..Default::default()
        }]),
        missing: 50_001,
    }
    .into();
    let mut samples = [None; TABLE_COUNT];
    samples[TableId::Countries.index()] = Some(273);
    frame.restore_samples(samples).unwrap();
    frame
}

#[test]
fn checkpoint_retains_partial_census_old_fields_bytes_and_five_thousand_countries() {
    let source = frame();
    let position = Position {
        block: 91,
        next_frame: 7,
    };
    let bytes = encode_checkpoint(&source, position, SELECTED).unwrap();
    assert_eq!(header(bytes.as_slice()).unwrap(), (71, 300));
    let restored = decode_checkpoint(bytes.as_slice(), SELECTED).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(restored.position, position);
    assert_eq!(
        restored.frame.tables[TableId::Countries.index()].sampled_at(),
        Some(273)
    );
    assert!(matches!(
        &*restored.frame.tables[TableId::DivisionStatus.index()],
        TableView::Truncated {
            missing: 50_001,
            ..
        }
    ));
    // A recording-style keyframe would turn this partial census into a refusal;
    // the prepared snapshot must retain every reached row and its count instead.
    assert_eq!(
        restored
            .frame
            .complete::<replay::rows::tables::Countries>()
            .unwrap()
            .len(),
        5_000
    );
}

#[test]
fn checkpoint_preserves_unavailable_and_known_empty_without_inventing_reads() {
    let mut source = frame();
    source.tables[TableId::DivisionStatus.index()] = TableView::Unavailable {
        reason: "division status read failed twice".into(),
    }
    .into();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert_eq!(
        decode_checkpoint(bytes.as_slice(), SELECTED).unwrap().frame,
        source
    );
    source.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(Vec::new())).into();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    let restored = decode_checkpoint(bytes.as_slice(), SELECTED).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(
        restored.frame.tables[TableId::DivisionStatus.index()].sampled_at(),
        None
    );
    assert!(!SharedView::ptr_eq(
        &source.tables[TableId::Countries.index()],
        &restored.frame.tables[TableId::Countries.index()]
    ));
}

#[test]
fn checkpoints_refuse_changed_selection_contract_and_trailing_payload() {
    let source = frame();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert!(decode_checkpoint(bytes.as_slice(), &[TableId::Campaign]).is_err());
    let mut changed = bytes.clone();
    changed[36..38].copy_from_slice(&0u16.to_le_bytes());
    assert!(decode_checkpoint(changed.as_slice(), SELECTED).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_checkpoint(trailing.as_slice(), SELECTED).is_err());
    assert!(validate_selection(&[TableId::DivisionStatus]).is_err());
    assert!(validate_selection(&[TableId::Campaign, TableId::Campaign]).is_err());
}

#[test]
fn checkpoint_compresses_native_state_and_rejects_corrupt_or_concatenated_frames() {
    let source = frame();
    let mut raw = Vec::new();
    source.encode_checkpoint(&mut raw).unwrap();
    let bytes = encode_checkpoint(
        &source,
        Position {
            block: 0,
            next_frame: 1,
        },
        SELECTED,
    )
    .unwrap();
    assert!(
        bytes.len() < raw.len() / 2,
        "the 5,000-country checkpoint must be compressed"
    );
    let payload = 38 + TableId::ALL.len();
    let mut corrupt = bytes.clone();
    let end = corrupt.len() - 1;
    corrupt[end] ^= 1;
    assert!(decode_checkpoint(corrupt.as_slice(), SELECTED).is_err());
    let mut concatenated = bytes.clone();
    concatenated.extend_from_slice(&bytes[payload + 8..]);
    assert!(decode_checkpoint(concatenated.as_slice(), SELECTED).is_err());
    let mut wrong_length = bytes;
    wrong_length[payload..payload + 8].copy_from_slice(&1u64.to_le_bytes());
    assert!(decode_checkpoint(wrong_length.as_slice(), SELECTED).is_err());
}

#[test]
fn map_selection_keeps_every_authored_trigger_operand_and_omits_product_only_inputs() {
    let selected = selected_tables();
    validate_selection(selected).unwrap();
    for &table in crate::triggers::RECORDED_TABLES {
        assert!(
            selected.contains(&table),
            "authored trigger input {table:?} is required"
        );
    }
    for table in [
        TableId::BattleSounds,
        TableId::Clock,
        TableId::DivisionMovement,
        TableId::MapEvents,
        TableId::NuclearStrikes,
        TableId::TaskForceMapMotion,
    ] {
        assert!(selected.contains(&table));
    }
    assert!(!selected.contains(&TableId::DivisionLandInputs));
    assert!(selected.len() < TableId::ALL.len());
}

#[test]
fn gap_intervals_keep_overlap_order_duplicates_and_maximum_hour() {
    use replay::branches::Gap;
    let gaps = [
        Gap {
            branch: 1,
            first_hour: 20,
            last_hour: 40,
            reason: "first".into(),
        },
        Gap {
            branch: 1,
            first_hour: 10,
            last_hour: 30,
            reason: "second".into(),
        },
        Gap {
            branch: 1,
            first_hour: 20,
            last_hour: 20,
            reason: "first".into(),
        },
        Gap {
            branch: 1,
            first_hour: Hour::MAX,
            last_hour: Hour::MAX,
            reason: "end".into(),
        },
    ];
    assert_eq!(
        gap_intervals(&gaps.iter().collect::<Vec<_>>()).unwrap(),
        vec![
            (10, 19, "second".into()),
            (20, 20, "first; second; first".into()),
            (21, 30, "first; second".into()),
            (31, 40, "first".into()),
            (Hour::MAX, Hour::MAX, "end".into()),
        ]
    );
}
