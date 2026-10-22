use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    observer_tests,
};

fn prepared_step(scan: &mut Scan, input: &DecodedFrame, tracker: &mut FrameChangeTracker) {
    let changes = tracker.observe(input, scan.format);
    scan.prepare_changed(input, &changes).unwrap();
}

#[test]
fn division_shared_driver_parity_keeps_equal_read_dates_changes_gaps_and_resume() {
    parity(3);
}

fn parity(divisions: u32) {
    use dashboard_base::prepared::series::tests::Fixture;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let mut held = frame(100, divisions);
    held.metrics.table_ns[TableId::DivisionTemplates.index()] = 1;
    let mut inputs = vec![held.clone()];
    for hour in 101..=103 {
        held.hour = hour;
        inputs.push(held.clone());
    }
    // Fresh equal template reads advance only supplying dates, not operands.
    let mut renamed = held.clone();
    renamed.hour = 104;
    if let TableView::Complete(TableRows::DivisionStatus(rows)) =
        &mut *renamed.tables[TableId::DivisionStatus.index()]
    {
        rows[0].name = Recorded::Value(Some(b"renamed synthetic division".to_vec()));
    }
    inputs.push(renamed.clone());
    renamed.hour = 105;
    witness(&mut renamed, Disposition::Merged, None);
    inputs.push(renamed.clone());
    let mut unavailable = renamed.clone();
    unavailable.hour = 106;
    unavailable.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "synthetic unavailable census".into(),
    }
    .into();
    unavailable.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic unavailable identity".into(),
    }
    .into();
    inputs.push(unavailable);
    renamed.hour = 108; // a gap forces the same join as the oracle
    witness(
        &mut renamed,
        Disposition::Loss,
        Some(Cause::DivisionDestroyed),
    );
    destroyed(&mut renamed);
    inputs.push(renamed.clone());
    renamed.hour = 109;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"synthetic renamed country".to_vec();
    }
    witness(&mut renamed, Disposition::VolunteerTransferred, None);
    inputs.push(renamed);

    let mut replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(
            1,
            vec![inputs[0].clone()],
        )],
    )
    .unwrap();
    for input in &inputs[1..] {
        replay = replay
            .append(&[dashboard_base::test_fixtures::continuation_records(
                &replay,
                1,
                vec![input.clone()],
            )])
            .unwrap();
    }
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "division-changes".into(),
            revision: "1".into(),
        },
        "division-changes.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut prepared = Scan::new(source.replay_ref().format);
    let mut oracle = Scan::new(source.replay_ref().format);
    let mut tracker = FrameChangeTracker::new();
    for (ordinal, input) in inputs.iter().enumerate() {
        prepared_step(&mut prepared, input, &mut tracker);
        oracle.prepare(input).unwrap();
        observer_tests::assert_reply_parity(&prepared.events, &oracle.events).unwrap();
        observer_tests::assert_reply_parity(
            &prepared.warnings.iter().collect::<Vec<_>>(),
            &oracle.warnings.iter().collect::<Vec<_>>(),
        )
        .unwrap();
        let mut writer = fixture.writer();
        prepared
            .publish_block(&source, &context, &mut writer, 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
        if ordinal == 3 || ordinal == 6 {
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            prepared.configure(source.replay_ref().format);
            prepared.resume(input);
            tracker.invalidate();
        }
        let params = DivisionLogParams {
            branch_id: "1".into(),
            window: ordinal,
        };
        let actual = prepared::query(
            &source,
            "division-log",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        observer_tests::assert_reply_parity(
            &actual,
            &oracle.reply(&source, &params, &context, &flag).unwrap(),
        )
        .unwrap();
        let params = DivisionLogTotalsParams {
            branch_id: "1".into(),
            first_hour: 100,
            last_hour: input.hour,
            countries: LandBattleCountryFilter {
                tags: None,
                membership: None,
                unobserved: vec![],
            },
            templates: None,
        };
        let index = totals_index(&oracle, &params, "oracle".into(), &flag).unwrap();
        let expected =
            totals_reply(index.counts[index.reads.partition_point(|read| *read <= input.hour)])
                .unwrap();
        let actual = prepared::query(
            &source,
            "division-log-totals",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        observer_tests::assert_reply_parity(&actual, &expected).unwrap();
    }
    // A witness after held equal reads must borrow the latest complete census.
    assert_eq!(prepared.events[0].row.presentation_hour, Some(104));
    assert_eq!(prepared.events[0].row.template_hour, Some(104));
    assert_eq!(prepared.events[1].row.presentation_hour, Some(105));
}
