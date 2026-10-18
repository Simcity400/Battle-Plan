use super::*;
use replay_data::rows::TABLE_COUNT;
use replay_data::{
    rows::CampaignRow, rows::ClockRow, rows::CountryRow, rows::DivisionStatusRow, rows::Recorded,
    rows::TableRows,
};
use replay_frame::{frame::Metrics, frame::SharedView};

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
            .complete::<replay_data::rows::tables::Countries>()
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
fn resume_reader_uses_and_validates_the_stored_selection() {
    let source = frame();
    let position = Position {
        block: 0,
        next_frame: 1,
    };
    let bytes = encode_checkpoint(&source, position, SELECTED).unwrap();
    let restored = decode_resume(bytes.as_slice()).unwrap();
    assert_eq!(restored.frame, source);
    assert_eq!(restored.position, position);
    assert!(restored.selects(TableId::DivisionStatus));
    assert!(!restored.selects(TableId::AirAces));
    let mut changed = bytes.clone();
    changed[38 + TableId::Countries.index()] = 0;
    assert!(decode_resume(changed.as_slice()).is_err());
    let mut trailing = bytes;
    trailing.push(0);
    assert!(decode_resume(trailing.as_slice()).is_err());
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
        // The Operatives map mode's native icon inputs.
        TableId::OperativeMissions,
        TableId::StrategicOperatives,
        TableId::AgencyOperationBuffers,
        TableId::Operations,
        TableId::CountryStabilityInputs,
        // The resources map mode and the trade routes.
        TableId::ConvoyClients,
        TableId::CountryOilFactors,
        TableId::RegionalConvoyEfficiency,
        TableId::ResourceExchanges,
        TableId::ResourceOrigins,
        TableId::ResourceRights,
        TableId::StateTemporaryResources,
        TableId::TradeRoutes,
    ] {
        assert!(selected.contains(&table));
    }
    assert!(!selected.contains(&TableId::DivisionLandInputs));
    assert!(selected.len() < TableId::ALL.len());
}

#[test]
fn gap_intervals_keep_overlap_order_duplicates_and_maximum_hour() {
    use replay_frame::branches::Gap;
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

#[test]
fn sparse_map_checkpoints_equal_hourly_bytes_across_short_blocks_gaps_branches_and_resume() {
    use crate::test_fixtures::{CAMPAIGN, continuation_records, records, scale_frame};
    use replay::{container::Replay, container::ReplayStream, container::test_work::table_decodes};
    let observed = |branch, hour| {
        let mut frame = scale_frame(branch, hour, 2);
        frame.metrics.table_ns[TableId::Countries.index()] = 1;
        frame.tables[TableId::DivisionStatus.index()] = match hour {
            104 => TableView::Truncated {
                rows: TableRows::DivisionStatus(vec![DivisionStatusRow {
                    division: 1,
                    ..Default::default()
                }]),
                missing: 7,
            },
            105 => TableView::Unavailable {
                reason: "synthetic failed census".into(),
            },
            _ => TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: 1,
                maximum_hit_points: i64::from(hour) * 1000,
                ..Default::default()
            }])),
        }
        .into();
        frame
    };
    let frames = |branch, hours: &[i32]| hours.iter().map(|&hour| observed(branch, hour)).collect();
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1, &[100, 101]))]).unwrap();
    let mut replay = first
        .append(&[continuation_records(
            &first,
            1,
            frames(1, &[102, 104, 105, 106]),
        )])
        .unwrap();
    replay = replay.append(&[records(2, frames(2, &[20, 21]))]).unwrap();
    // A new session starts a new branch instead of re-entering session 1.
    replay = replay
        .append(&[records(3, frames(3, &[108, 109, 110]))])
        .unwrap();
    let appended = replay
        .append(&[continuation_records(
            &replay,
            3,
            frames(3, &[111, 112, 113]),
        )])
        .unwrap();
    let control = crate::prepared::series::tests::Fixture::new();
    let sparse = crate::prepared::series::tests::Fixture::new();
    let stride = NonZeroU32::new(3).unwrap();
    let mut full_map = Builder::new(stride, SELECTED, Progress::default()).unwrap();
    let mut sparse_map = Builder::new(stride, SELECTED, Progress::default()).unwrap();
    let mut full = ReplayStream::new(SELECTED).unwrap();
    let mut narrow = ReplayStream::new(&[TableId::Campaign]).unwrap();
    for block in 0..appended.index.len() {
        // The appended tail is first opened after accepting the earlier revision.
        let replay = if block < replay.index.len() {
            &replay
        } else {
            &appended
        };
        let mut writer = control.writer();
        let mut count = 0;
        full.next_block_with(
            replay,
            |raw, frame| {
                count += 1;
                full_map.accept(
                    &mut writer,
                    &raw,
                    frame,
                    Position {
                        block,
                        next_frame: count,
                    },
                )
            },
            || Ok(()),
        )
        .unwrap();
        writer.commit().unwrap();
        let mut writer = sparse.writer();
        let mut count = 0;
        let (_, decodes) = table_decodes(|| {
            narrow
                .next_block_with(
                    replay,
                    |raw, frame| {
                        count += 1;
                        sparse_map.accept_sparse(
                            &mut writer,
                            replay,
                            &raw,
                            frame,
                            Position {
                                block,
                                next_frame: count,
                            },
                            || Ok(()),
                        )
                    },
                    || Ok(()),
                )
                .unwrap()
        });
        writer.commit().unwrap();
        assert_eq!(
            sparse.payloads(),
            control.payloads(),
            "block {block}: every compressed checkpoint byte"
        );
        assert_eq!(
            serde_json::to_vec(sparse_map.progress()).unwrap(),
            serde_json::to_vec(full_map.progress()).unwrap()
        );
        let hours: Vec<_> = decodes
            .iter()
            .filter(|(_, _, table, _)| *table == TableId::DivisionStatus)
            .map(|(_, _, _, hours)| *hours)
            .collect();
        assert_eq!(
            hours,
            if block == 1 { vec![2] } else { vec![1] },
            "block {block}: only checkpoint prefixes decoded"
        );
        let progress =
            serde_json::from_slice(&serde_json::to_vec(sparse_map.progress()).unwrap()).unwrap();
        sparse_map = Builder::new(stride, SELECTED, progress).unwrap();
        narrow = ReplayStream::resume(
            &[TableId::Campaign],
            narrow.cursor().clone(),
            narrow.accumulator().cloned(),
        )
        .unwrap();
    }
}
