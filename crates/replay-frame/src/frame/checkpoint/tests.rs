use super::*;
use crate::rows::*;

fn frame(hour: Hour, keyframe: bool) -> Frame {
    Frame {
        branch: 19,
        hour,
        keyframe,
        tables: std::array::from_fn(|_| TableUpdate::Unavailable {
            reason: "not acquired".into(),
        }),
        metrics: Metrics {
            total_ns: u64::MAX,
            pool_bytes: u64::MAX - 1,
            dropped_frames: 7,
            ..Default::default()
        },
    }
}

fn stored(source: &Frame) -> Frame {
    let mut bytes = Vec::new();
    source.encode(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let decoded = Frame::decode(&mut input).unwrap();
    input.finish().unwrap();
    decoded
}

fn checkpoint(source: &DecodedFrame) -> (DecodedFrame, Vec<u8>) {
    let mut bytes = Vec::new();
    source.encode_checkpoint(&mut bytes).unwrap();
    let mut input = Decoder::new(&bytes);
    let restored = DecodedFrame::decode_checkpoint(&mut input).unwrap();
    input.finish().unwrap();
    assert_eq!(&restored, source);
    for table in TableId::ALL {
        assert_eq!(
            restored.tables[table.index()].sampled_at(),
            source.tables[table.index()].sampled_at(),
            "{table:?} sample origin"
        );
    }
    (restored, bytes)
}

#[test]
fn checkpoint_current_rows_round_trip_after_ordinary_wire_decoding() {
    for tick in 0..4 {
        let mut source = frame(tick, true);
        for table in TableId::ALL {
            if let Some(mut rows) = crate::rows::sample_extended_table(table, tick as usize) {
                rows.round_to_display();
                source.tables[table.index()] = TableUpdate::Snapshot(rows);
                source.metrics.table_ns[table.index()] = super::super::READ_TIME_NOT_RECORDED;
            }
        }
        let decoded = FrameDecoder::new().apply(&stored(&source)).unwrap();
        checkpoint(&decoded);
    }
}

#[test]
fn checkpoint_resume_matches_uninterrupted_delta_and_preserves_partial_tables() {
    let country = CountryRow {
        id: 3,
        canonical_slot: 8,
        tag: vec![0, 255],
        ..Default::default()
    };
    let mut baseline = frame(72, true);
    baseline.tables[TableId::Countries.index()] =
        TableUpdate::Snapshot(TableRows::Countries(vec![country.clone()]));
    baseline.tables[TableId::CountryLinks.index()] =
        TableUpdate::Snapshot(TableRows::CountryLinks(Vec::new()));
    baseline.metrics.table_ns[TableId::Countries.index()] = super::super::READ_TIME_NOT_RECORDED;
    let mut uninterrupted = FrameDecoder::new();
    uninterrupted.apply(&stored(&baseline)).unwrap();

    let mut partial = frame(73, false);
    partial.tables[TableId::Countries.index()] = TableUpdate::Unchanged { baseline: 72 };
    partial.tables[TableId::CountryLinks.index()] = TableUpdate::Unchanged { baseline: 72 };
    partial.tables[TableId::DivisionStatus.index()] = TableUpdate::Truncated {
        rows: TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: 9,
            supply: Recorded::NotRecorded,
            ..Default::default()
        }]),
        missing: u32::MAX,
    };
    let source = uninterrupted.apply(&stored(&partial)).unwrap();
    assert_eq!(
        source.tables[TableId::Countries.index()].sampled_at(),
        Some(72)
    );
    assert_eq!(
        source.tables[TableId::DivisionStatus.index()].sampled_at(),
        None
    );
    let (restored, _) = checkpoint(&source);
    let selected = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::DivisionStatus,
    ];
    let mut resumed = FrameDecoder::resume_for_tables(&selected, restored).unwrap();
    assert_eq!(resumed.current().unwrap().hour, 73);

    let next_country = CountryRow {
        manpower_residual: -7,
        ..country.clone()
    };
    let mut next = frame(74, false);
    next.tables[TableId::Countries.index()] = TableUpdate::Delta {
        baseline: 73,
        changed: vec![next_country.delta(Some(&country)).unwrap().unwrap()],
        deleted: Vec::new(),
    };
    next.tables[TableId::CountryLinks.index()] = TableUpdate::Unchanged { baseline: 73 };
    next.tables[TableId::DivisionStatus.index()] = TableUpdate::Unchanged { baseline: 73 };
    next.metrics.table_ns[TableId::Countries.index()] = super::super::READ_TIME_NOT_RECORDED;
    let next = stored(&next);
    let expected = uninterrupted.apply(&next).unwrap();
    let actual = resumed.apply(&next).unwrap();
    for table in selected {
        assert_eq!(actual.tables[table.index()], expected.tables[table.index()]);
        assert_eq!(
            actual.tables[table.index()].sampled_at(),
            expected.tables[table.index()].sampled_at()
        );
    }
    assert_eq!(actual.metrics, expected.metrics);
    assert_eq!(
        actual.tables[TableId::Countries.index()].sampled_at(),
        Some(74)
    );
    assert!(
        matches!(&*actual.tables[TableId::DivisionStatus.index()], TableView::Unavailable { reason } if reason.contains("missing table baseline"))
    );
    assert!(
        resumed.apply(&next).is_err(),
        "resume retains the frame-hour guard"
    );
}

#[test]
fn checkpoint_preserves_large_census_and_raw_decoded_nested_steps() {
    let mut baseline = frame(72, true);
    baseline.tables[TableId::Countries.index()] = TableUpdate::Snapshot(TableRows::Countries(
        (0..5_000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: vec![0, 255],
                ..Default::default()
            })
            .collect(),
    ));
    baseline.tables[TableId::DivisionStatus.index()] =
        TableUpdate::Snapshot(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: u64::MAX,
            supply: Recorded::Value(Some(DivisionSupplyFooter {
                asked: 120_000,
                received: 110_000,
                origins: Some(vec![UnitSupplyOrigin {
                    province: -3,
                    requested: 120_000,
                    received: 110_000,
                }]),
                ..Default::default()
            })),
            ..Default::default()
        }]));
    let mut source = FrameDecoder::new().apply(&stored(&baseline)).unwrap();
    let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *source.tables[TableId::DivisionStatus.index()]
    else {
        panic!("complete division status");
    };
    let Recorded::Value(Some(footer)) = &mut rows[0].supply else {
        panic!("recorded footer");
    };
    footer.asked = 123_457;
    footer.received = -117_911;
    footer.origins.as_mut().unwrap()[0].requested = 123_457;
    checkpoint(&source);
}

#[test]
fn checkpoint_schema_corruption_and_invalid_state_are_refused_atomically() {
    let source = FrameDecoder::new()
        .apply(&stored(&frame(72, true)))
        .unwrap();
    let (_, bytes) = checkpoint(&source);
    for end in 0..bytes.len() {
        assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&bytes[..end])).is_err());
    }
    for offset in [0, 4, 6] {
        let mut unknown = bytes.clone();
        unknown[offset] ^= 1;
        assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&unknown)).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    let mut input = Decoder::new(&trailing);
    DecodedFrame::decode_checkpoint(&mut input).unwrap();
    assert!(input.finish().is_err());

    let mut invalid = source.clone();
    invalid.tables[TableId::Countries.index()].1 = Some(invalid.hour);
    let mut output = vec![1, 2, 3];
    assert!(invalid.encode_checkpoint(&mut output).is_err());
    assert_eq!(output, [1, 2, 3]);
    assert!(FrameDecoder::resume_for_tables(&[], invalid).is_err());
    let mut samples = [None; TABLE_COUNT];
    samples[TableId::Countries.index()] = Some(source.hour);
    let mut restored = source.clone();
    assert!(restored.restore_samples(samples).is_err());
    for (before, after) in source.tables.iter().zip(&restored.tables) {
        assert_eq!(before.sampled_at(), after.sampled_at());
    }

    let mut empty_singleton = source;
    empty_singleton.tables[TableId::Campaign.index()] =
        TableView::Complete(TableRows::Campaign(Vec::new())).into();
    assert!(empty_singleton.encode_checkpoint(&mut output).is_err());
    assert_eq!(output, [1, 2, 3]);
    assert!(FrameDecoder::resume_for_tables(&[], empty_singleton).is_err());
}

#[test]
fn checkpoint_payload_rejects_bad_status_keys_counts_and_clock_hour() {
    let mut baseline = frame(72, true);
    baseline.tables[TableId::Clock.index()] =
        TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
            hour: 72,
            ..Default::default()
        }]));
    let source = FrameDecoder::new().apply(&stored(&baseline)).unwrap();
    let (_, bytes) = checkpoint(&source);
    let first_table = 44 + 8 * TABLE_COUNT;
    let count = first_table + 2;
    let key = count + 4;
    let clock_hour = key + 16;
    let mut status = bytes.clone();
    status[first_table + 1] = 255;
    assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&status)).is_err());
    let mut hostile_count = bytes.clone();
    hostile_count[count..count + 4].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&hostile_count)).is_err());
    let mut nonzero_singleton_key = bytes.clone();
    nonzero_singleton_key[key] = 1;
    assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&nonzero_singleton_key)).is_err());
    let mut different_hour = bytes;
    different_hour[clock_hour..clock_hour + 4].copy_from_slice(&71_i32.to_le_bytes());
    assert!(DecodedFrame::decode_checkpoint(&mut Decoder::new(&different_hour)).is_err());
}
