use super::spending::Scanned as Seen;
use super::*;
fn observe(
    frame: &DecodedFrame,
    tag: &str,
    reply: &mut PoliticsLedgerReply,
    scan: &mut Seen,
) -> io::Result<()> {
    super::spending::observe_all(frame, scan, None)?;
    *reply = super::spending::present(scan, frame.branch, tag, None, &AtomicBool::new(false))?;
    Ok(())
}
use replay_data::rows::TABLE_COUNT;
use replay_frame::frame::TableView;
fn frame(hour: i32, tag: &[u8], instance: u32) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| {
            TableView::Unavailable {
                reason: "fixture".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let rows = [
        TableRows::Countries(vec![CountryRow {
            id: 5001,
            canonical_slot: 1,
            tag: tag.to_vec(),
            ..Default::default()
        }]),
        TableRows::CountryLinks(vec![CountryLinkRow {
            id: 5001,
            canonical_slot: 1,
        }]),
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 5001,
            instance,
            epoch: 1,
            since_hour: hour - 1,
            through_hour: hour,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    ];
    for row in rows {
        let index = row.table().index();
        frame.tables[index] = TableView::Complete(row).into();
    }
    frame
}
fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    frame.tables[index] = TableView::Complete(rows).into();
}
#[test]
fn events_deduplicate_keep_integer_precision_and_never_claim_missing_coverage() {
    let mut frame = frame(11, b"AAA", 9);
    let event = CountryPowerChangeRow {
        country: 5001,
        instance: 9,
        epoch: 1,
        sequence: u64::MAX,
        hour: 10,
        balance_before_raw: i64::MIN,
        balance_after_raw: i64::MAX,
        requested_delta_raw: Some(i64::MIN),
        cause: PowerCause::Decision,
        ..Default::default()
    };
    set(&mut frame, TableRows::CountryPowerChanges(vec![event]));
    let mut reply = PoliticsLedgerReply::default();
    let mut seen = Seen::default();
    observe(&frame, "AAA", &mut reply, &mut seen).unwrap();
    observe(&frame, "AAA", &mut reply, &mut seen).unwrap();
    assert_eq!(reply.mutations.len(), 1);
    assert!(reply.mutations[0].covered);
    assert_eq!(
        reply.mutations[0].realized_delta_raw,
        "18446744073709551615"
    );
    assert_eq!(reply.mutations[0].sequence, u64::MAX.to_string());
    let mut frame = frame.clone();
    frame.hour = 12;
    frame.tables[TableId::PoliticsCaptureCoverage.index()] = TableView::Unavailable {
        reason: "loss".into(),
    }
    .into();
    set(
        &mut frame,
        TableRows::CountryPowerChanges(vec![CountryPowerChangeRow {
            country: 5001,
            instance: 9,
            epoch: 1,
            sequence: 8,
            hour: 11,
            ..Default::default()
        }]),
    );
    observe(&frame, "AAA", &mut reply, &mut seen).unwrap();
    assert!(!reply.mutations[1].covered);
    assert!(reply.coverage.last().unwrap().reason.is_some());
    assert!(
        reply
            .warnings
            .iter()
            .any(|warning| warning.contains("GovernmentChanges unavailable"))
    );
}

#[test]
fn tag_changes_date_mutations_governments_and_daily_rows_to_their_native_hour() {
    let mut seen = Seen::default();
    let mut reply = PoliticsLedgerReply::default();
    let mut first = frame(24, b"AAA", 9);
    let day = CountryPowerDailyRow {
        country: 5001,
        instance: 9,
        epoch: 1,
        day_hour: 24,
        cp_gain_raw: Some(3),
        cp_realized_delta_raw: Some(3),
        cp_sequence: Some(1),
        ..Default::default()
    };
    set(&mut first, TableRows::CountryPowerDaily(vec![day.clone()]));
    observe(&first, "AAA", &mut reply, &mut seen).unwrap();
    let mut renamed = frame(25, b"BBB", 9);
    set(&mut renamed, TableRows::CountryPowerDaily(vec![day]));
    set(
        &mut renamed,
        TableRows::CountryPowerChanges(
            [24, 25]
                .into_iter()
                .map(|hour| CountryPowerChangeRow {
                    country: 5001,
                    instance: 9,
                    epoch: 1,
                    sequence: hour as u64,
                    hour,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut renamed,
        TableRows::GovernmentChanges(vec![GovernmentChangeRow {
            country: 5001,
            instance: 9,
            epoch: 1,
            sequence: 26,
            hour: 25,
            ..Default::default()
        }]),
    );
    observe(&renamed, "AAA", &mut reply, &mut seen).unwrap();
    assert_eq!(reply.mutations.len(), 1);
    assert_eq!(reply.mutations[0].hour, 24);
    assert_eq!(reply.daily.len(), 1);
    assert!(reply.governments.is_empty());
    reply = super::spending::present(&seen, 1, "BBB", None, &AtomicBool::new(false)).unwrap();
    assert_eq!(reply.mutations.len(), 1);
    assert_eq!(reply.mutations[0].hour, 25);
    assert_eq!(reply.governments.len(), 1);
    assert!(reply.daily.is_empty());
}

#[test]
fn reused_raw_country_does_not_reassign_old_events_and_daily_batches_can_finish_later() {
    let mut seen = Seen::default();
    let mut reply = PoliticsLedgerReply::default();
    let mut first = frame(24, b"AAA", 9);
    set(
        &mut first,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 5001,
            instance: 9,
            epoch: 1,
            day_hour: 24,
            ..Default::default()
        }]),
    );
    observe(&first, "AAA", &mut reply, &mut seen).unwrap();
    set(
        &mut first,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 5001,
            instance: 9,
            epoch: 1,
            day_hour: 24,
            cp_gain_raw: Some(3),
            cp_realized_delta_raw: Some(3),
            cp_sequence: Some(4),
            ..Default::default()
        }]),
    );
    first.hour = 25;
    observe(&first, "AAA", &mut reply, &mut seen).unwrap();
    assert_eq!(reply.daily.len(), 1);
    assert_eq!(reply.daily[0].cp_income_raw.as_deref(), Some("3"));
    assert_eq!(reply.daily[0].read_hour, 25);
    let mut later = frame(26, b"BBB", 10);
    set(
        &mut later,
        TableRows::CountryPowerChanges(vec![
            CountryPowerChangeRow {
                country: 5001,
                instance: 9,
                epoch: 1,
                sequence: 4,
                hour: 24,
                ..Default::default()
            },
            CountryPowerChangeRow {
                country: 5001,
                instance: 10,
                epoch: 1,
                sequence: 8,
                hour: 25,
                ..Default::default()
            },
        ]),
    );
    observe(&later, "AAA", &mut reply, &mut seen).unwrap();
    assert_eq!(reply.mutations.len(), 1);
    assert_eq!(reply.mutations[0].instance, 9);
    assert!(!reply.mutations[0].covered);
    let mut reused = frame(27, b"CCC", 9);
    set(
        &mut reused,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 5001,
            instance: 9,
            epoch: 2,
            since_hour: 26,
            through_hour: 27,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    set(
        &mut reused,
        TableRows::CountryPowerChanges(vec![
            CountryPowerChangeRow {
                country: 5001,
                instance: 9,
                epoch: 1,
                sequence: 12,
                hour: 24,
                ..Default::default()
            },
            CountryPowerChangeRow {
                country: 5001,
                instance: 9,
                epoch: 2,
                sequence: 12,
                hour: 26,
                ..Default::default()
            },
        ]),
    );
    observe(&reused, "AAA", &mut reply, &mut seen).unwrap();
    assert_eq!(reply.mutations.len(), 2);
    assert_eq!(reply.mutations[1].epoch, "1");
    assert!(!reply.mutations[1].covered);
}
