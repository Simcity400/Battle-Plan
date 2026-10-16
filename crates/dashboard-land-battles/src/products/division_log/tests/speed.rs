use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    observer_tests::{self, Work},
};

fn prepared_step(scan: &mut Scan, input: &DecodedFrame, tracker: &mut FrameChangeTracker) {
    let changes = tracker.observe(input, scan.format);
    scan.prepare_changed(input, &changes).unwrap();
}

fn unchanged_work(divisions: u32, hours: usize) -> io::Result<Work> {
    let first = frame(100, divisions);
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    prepared_step(&mut scan, &first, &mut tracker);
    observer_tests::unchanged_frames(
        scan.work,
        &first,
        hours as i32,
        scan.format,
        |input, changes| {
            scan.prepare_changed(input, changes)?;
            Ok(scan.work)
        },
    )
}

#[test]
fn division_shared_driver_unchanged_frames_skip_native_and_presentation_work() {
    let work = observer_tests::scaled_work(4, |hours| unchanged_work(3, hours)).unwrap();
    observer_tests::assert_flat(work);
    assert_eq!(work, [Work::default(); 2]);
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "5,000 divisions")]
fn division_shared_driver_5000_divisions_keep_unchanged_work_flat() {
    let work = [
        unchanged_work(3, 8).unwrap(),
        unchanged_work(5000, 8).unwrap(),
    ];
    observer_tests::assert_flat(work);
    assert_eq!(work, [Work::default(); 2]);
}

fn after_history(hours: usize, changed: bool) -> io::Result<Work> {
    let mut input = frame(100, 3);
    let mut scan = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    prepared_step(&mut scan, &input, &mut tracker);
    for at in 0..hours {
        input.hour += 1;
        witness(
            &mut input,
            if at % 2 == 0 {
                Disposition::Merged
            } else {
                Disposition::VolunteerTransferred
            },
            None,
        );
        prepared_step(&mut scan, &input, &mut tracker);
    }
    assert_eq!(scan.events.len(), hours);
    scan.work = Work::default();
    if changed {
        input.hour += 1;
        witness(
            &mut input,
            Disposition::Loss,
            Some(Cause::DivisionDestroyed),
        );
        destroyed(&mut input);
        prepared_step(&mut scan, &input, &mut tracker);
        assert_eq!(scan.events.len(), hours + 1);
        Ok(scan.work)
    } else {
        observer_tests::unchanged_frames(scan.work, &input, 8, scan.format, |input, changes| {
            scan.prepare_changed(input, changes)?;
            Ok(scan.work)
        })
    }
}

#[test]
fn division_shared_driver_unchanged_and_one_witness_work_stay_flat_after_ten_times_history() {
    let unchanged = observer_tests::scaled_work(10, |hours| after_history(hours, false)).unwrap();
    observer_tests::assert_flat(unchanged);
    assert_eq!(unchanged, [Work::default(); 2]);
    let changed = observer_tests::scaled_work(10, |hours| after_history(hours, true)).unwrap();
    observer_tests::assert_flat(changed);
    assert_eq!(changed[0].emitted, 1);
    assert_eq!(changed[0].row_visits, 2); // one witness and its one destruction
    assert_eq!(changed[0].history_visits, 0);
}

#[test]
fn division_shared_driver_parity_keeps_equal_read_dates_changes_gaps_and_resume() {
    parity(3);
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "5,000 divisions and countries")]
fn division_shared_driver_5000_divisions_match_oracle_after_commits_resume_and_append() {
    parity(5000);
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
