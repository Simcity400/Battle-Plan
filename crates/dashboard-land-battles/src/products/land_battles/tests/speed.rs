use super::*;
use dashboard_base::prepared::{
    frame_changes::FrameChangeTracker,
    observer_tests::{self, Work},
};

fn step(census: &mut Census, input: &DecodedFrame, tracker: &mut FrameChangeTracker) {
    let changes = tracker.observe(input, census.format);
    census.observe_prepared(input, &changes).unwrap();
}

/// A census that paged out earlier hours answers from memory only for those it holds.
fn assert_replies(prepared: &Census, oracle: &Census, first: i32, last: i32) {
    let first = prepared
        .paged_before
        .map_or(first, |before| first.max(before));
    observer_tests::assert_reply_parity(
        &prepared
            .reply(&params(Some(0)), &windows(first, last))
            .unwrap(),
        &oracle
            .reply(&params(Some(0)), &windows(first, last))
            .unwrap(),
    )
    .unwrap();
    for measure in ["Length", "Peak size", "Casualties"] {
        let params = ranking_params(first, last, 5, measure);
        observer_tests::assert_reply_parity(
            &prepared.ranked(&params, &AtomicBool::new(false)).unwrap(),
            &oracle.ranked(&params, &AtomicBool::new(false)).unwrap(),
        )
        .unwrap();
    }
    // A battle that paged out runs or reads has its whole course only in the store.
    for episode in prepared
        .episodes
        .values()
        .filter(|episode| episode.history.is_none())
        .take(3)
    {
        observer_tests::assert_reply_parity(
            &prepared.course(&episode.id).unwrap(),
            &oracle.course(&episode.id).unwrap(),
        )
        .unwrap();
    }
}

fn unchanged_work(combats: u32, hours: usize, unknown: bool) -> io::Result<Work> {
    let mut first = frame(100, 5, combats);
    if unknown
        && let TableView::Complete(TableRows::LandCombatSides(rows)) =
            &mut *first.tables[TableId::LandCombatSides.index()]
    {
        rows[0].attacker_primary = vec![90000, 90000];
    }
    let mut prepared = Census::default();
    let mut oracle = Census::default();
    let mut tracker = FrameChangeTracker::new();
    step(&mut prepared, &first, &mut tracker);
    oracle.observe(&first)?;
    let work = observer_tests::unchanged_frames(
        prepared.work,
        &first,
        hours as i32,
        prepared.format,
        |input, changes| {
            prepared.observe_prepared(input, changes)?;
            oracle.observe(input)?;
            Ok(prepared.work)
        },
    )?;
    // These read open state without forcing the stored census to expand.
    assert_replies(&prepared, &oracle, 100, 100 + hours as i32);
    assert_eq!(prepared.episodes[&0].runs.len(), 1);
    assert_eq!(prepared.deferred_through, Some(100 + hours as i32));
    prepared.materialize()?;
    assert_replies(&prepared, &oracle, 100, 100 + hours as i32);
    observer_tests::assert_reply_parity(&prepared.episodes, &oracle.episodes)?;
    assert_eq!(prepared.warnings, oracle.warnings);
    Ok(work)
}

#[test]
fn land_shared_driver_unchanged_census_work_stays_flat_with_exact_counter_runs_and_warnings() {
    for unknown in [false, true] {
        let work =
            observer_tests::scaled_work(4, |hours| unchanged_work(3, hours, unknown)).unwrap();
        observer_tests::assert_flat(work);
        assert_eq!(work, [Work::default(); 2]);
    }
}

#[test]
fn land_shared_driver_counter_steps_match_existing_oracle_after_skipped_spans() {
    let mut input = frame(100, 5, 1);
    let mut prepared = Census::default();
    let mut oracle = Census::default();
    let mut tracker = FrameChangeTracker::new();
    for (hour, duration) in [
        (100, 50),
        (101, 51),
        (102, 53),
        (103, 53),
        (104, 54),
        (105, 54),
        (106, 54),
        (107, 55),
    ] {
        input.hour = hour;
        let previous = input.complete::<tables::LandCombats>().unwrap()[0].duration;
        if previous != duration
            && let TableView::Complete(TableRows::LandCombats(rows)) =
                &mut *input.tables[TableId::LandCombats.index()]
        {
            rows[0].duration = duration;
        }
        step(&mut prepared, &input, &mut tracker);
        oracle.observe(&input).unwrap();
        assert_replies(&prepared, &oracle, 100, hour);
    }
    prepared.materialize().unwrap();
    observer_tests::assert_reply_parity(&prepared.episodes, &oracle.episodes).unwrap();
}

#[test]
fn land_shared_driver_unavailable_inputs_materialize_old_frontier_and_keep_diagnostics_exact() {
    let original = frame(100, 5, 3);
    let mut input = original.clone();
    let mut prepared = Census::default();
    let mut oracle = Census::default();
    let mut tracker = FrameChangeTracker::new();
    for hour in 100..=111 {
        input.hour = hour;
        if hour == 103 {
            input.tables[TableId::LandCombatSides.index()] = TableView::Unavailable {
                reason: "synthetic participants unavailable".into(),
            }
            .into();
        }
        if hour == 106 {
            input.tables[TableId::LandCombats.index()] = TableView::Unavailable {
                reason: "synthetic live census unavailable".into(),
            }
            .into();
        }
        if hour == 108 {
            input.tables[TableId::LandCombats.index()] =
                original.tables[TableId::LandCombats.index()].clone();
            input.tables[TableId::LandCombatSides.index()] =
                original.tables[TableId::LandCombatSides.index()].clone();
            input.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
                reason: "synthetic country links unavailable".into(),
            }
            .into();
        }
        if hour == 110 {
            input.tables[TableId::CountryLinks.index()] =
                original.tables[TableId::CountryLinks.index()].clone();
        }
        step(&mut prepared, &input, &mut tracker);
        oracle.observe(&input).unwrap();
        assert_replies(&prepared, &oracle, 100, hour);
    }
    prepared.materialize().unwrap();
    observer_tests::assert_reply_parity(&prepared.episodes, &oracle.episodes).unwrap();
    assert_eq!(prepared.warnings, oracle.warnings);
    assert_eq!(prepared.episodes[&0].runs.last().unwrap().last_hour, 105);
    assert_eq!(prepared.episodes[&0].end.as_ref().unwrap().0, 106);
}

fn fixture_inputs(combats: u32) -> Vec<Vec<DecodedFrame>> {
    let mut input = frame(100, 5, combats);
    let mut groups = vec![vec![input.clone()]];
    for hour in 101..=107 {
        input.hour = hour;
        if hour == 103
            && let TableView::Complete(TableRows::LandCombats(rows)) =
                &mut *input.tables[TableId::LandCombats.index()]
        {
            rows[0].duration += 1;
        }
        if hour == 104
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(1);
        }
        if hour == 105
            && let TableView::Complete(TableRows::LandCombatSides(rows)) =
                &mut *input.tables[TableId::LandCombatSides.index()]
        {
            rows[0].attacker_primary = vec![90000, 90000];
        }
        if hour == 106 {
            set_progress(
                &mut input,
                vec![LandCombatProgressRow {
                    combat: 1,
                    attacker_manpower_loss: Recorded::Value(7),
                    defender_manpower_loss: Recorded::Value(9),
                    attacker_equipment_losses: Recorded::Value(vec![]),
                    defender_equipment_losses: Recorded::Value(vec![]),
                    ..Default::default()
                }],
            );
        }
        if hour == 103 || hour == 106 {
            groups.push(vec![]);
        }
        groups.last_mut().unwrap().push(input.clone());
    }
    groups.push(vec![]);
    for hour in 110..=112 {
        input.hour = hour;
        if hour == 111
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
        {
            rows[1].tag = b"mod country renamed".to_vec();
        }
        groups.last_mut().unwrap().push(input.clone());
    }
    let mut finalizer = result_frame(113, 3);
    if let TableView::Complete(TableRows::LandCombatResults(rows)) =
        &mut *finalizer.tables[TableId::LandCombatResults.index()]
    {
        rows[0].end_hour = 112;
    }
    if let TableView::Complete(TableRows::LandEventCoverage(rows)) =
        &mut *finalizer.tables[TableId::LandEventCoverage.index()]
    {
        rows[0].from_hour = 111;
    }
    groups.push(vec![finalizer]);
    groups
}

fn publication_parity(combats: u32) {
    use dashboard_base::prepared::series::tests::Fixture;
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let inputs = fixture_inputs(combats);
    let mut replay = Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, inputs[0].clone())],
    )
    .unwrap();
    for group in &inputs[1..] {
        replay = replay
            .append(&[dashboard_base::test_fixtures::continuation_records(
                &replay,
                1,
                group.clone(),
            )])
            .unwrap();
    }
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "land-classes".into(),
            revision: "1".into(),
        },
        "land-classes.battleplan".into(),
        Arc::new(replay),
        &flag,
    )
    .unwrap();
    let fixture = Fixture::new();
    let mut prepared = Census::new(source.replay_ref().format);
    let mut oracle = Census::new(source.replay_ref().format);
    let mut tracker = FrameChangeTracker::new();
    prepared.configure(&source, &context);
    oracle.configure(&source, &context);
    for (ordinal, group) in inputs.iter().enumerate() {
        for input in group {
            step(&mut prepared, input, &mut tracker);
            oracle.observe(input).unwrap();
            assert_replies(&prepared, &oracle, 100, input.hour);
            if input.hour == 101 {
                assert_eq!(prepared.deferred_through, Some(101));
                // Accumulator batches are publication buffers, not continuation.
                // Persist the read already emitted at 100 while leaving the
                // native carry outstanding, just as a checkpoint flush does.
                let mut writer = fixture.writer();
                prepared
                    .reads
                    .flush(&mut writer, "land-battle-reads-1", 0, &flag)
                    .unwrap();
                writer.commit().unwrap();
                prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
                prepared.configure(&source, &context);
                // A reply immediately after restoration sees the persisted frontier.
                assert_replies(&prepared, &oracle, 100, 101);
                tracker.invalidate();
            }
        }
        let mut writer = fixture.writer();
        prepared
            .publish_block(&mut writer, source.replay_ref(), 1, ordinal, &flag)
            .unwrap();
        writer.commit().unwrap();
        assert!(prepared.deferred_through.is_none());
        assert_held(&prepared, &oracle);
        if ordinal == 1 || ordinal == 2 {
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            prepared.configure(&source, &context);
            tracker.invalidate();
        }
    }
    let windows = super::super::windows(source.replay_ref(), 1);
    for ordinal in 0..windows.len() {
        let params = params(Some(ordinal));
        let actual = prepared::query(
            &source,
            "land-battles",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap()
        .into_value()
        .unwrap();
        observer_tests::assert_reply_parity(&actual, &oracle.reply(&params, &windows).unwrap())
            .unwrap();
    }
    for measure in ["Length", "Peak size", "Casualties"] {
        for (first, last) in [
            (100, 100),
            (100, 101),
            (101, 102),
            (102, 107),
            (107, 109),
            (110, 113),
        ] {
            let params = ranking_params(first, last, 5, measure);
            let actual = prepared::query(
                &source,
                "land-battles-ranking",
                serde_json::to_value(&params).unwrap(),
                &fixture.store,
                &fixture.key,
                &flag,
            )
            .unwrap()
        .into_value()
        .unwrap();
            observer_tests::assert_reply_parity(&actual, &oracle.ranked(&params, &flag).unwrap())
                .unwrap();
        }
    }
    for episode in oracle.episodes.values().take(3) {
        let params = LandBattleCourseParams {
            branch_id: "1".into(),
            id: episode.id.clone(),
        };
        let actual = prepared::query(
            &source,
            "land-battle-course",
            serde_json::to_value(&params).unwrap(),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap()
        .into_value()
        .unwrap();
        observer_tests::assert_reply_parity(&actual, &oracle.course(&episode.id).unwrap()).unwrap();
    }
}

#[test]
fn land_shared_driver_prepared_replies_match_oracle_after_skips_two_commits_resume_append_and_gap()
{
    publication_parity(3);
}
