use super::super::tests::{frame, operative};
use super::*;
use crate::{prepared::series::tests::Fixture, test_fixtures};
use replay::{container::Replay, frame::TableView, rows::TableRows};

#[test]
fn prepared_placements_query_rejects_a_different_parameter_branch() {
    let fixture = Fixture::new();
    for branch_id in ["2", "invalid"] {
        let params = Params {
            branch_id: branch_id.into(),
            first_hour: None,
            last_hour: None,
        };
        let error = query(
            &fixture.store,
            &fixture.key,
            1,
            &params,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("dispatched branch"));
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_placements_keep_five_thousand_countries_and_dated_read_anchors() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let input = super::super::tests::scale_frame(10, true);
    let source = source(
        Replay::from_records(
            test_fixtures::CAMPAIGN.into(),
            &[test_fixtures::records(1, vec![input.clone()])],
        )
        .unwrap(),
    );
    let context = LoadContext { content: None };
    let mut state = Prepared::default();
    let mut old = Scan::default();
    for hours in [10..34, 34..58] {
        for hour in hours {
            let mut input = input.clone();
            input.hour = hour;
            input.metrics.table_ns[TableId::OperativeMemberships.index()] =
                u64::from((hour - 10) % 24 == 0);
            state.observe(&source, &context, &input).unwrap();
            old.observe(&source, &context, &input).unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    let params = Params {
        branch_id: "1".into(),
        first_hour: None,
        last_hour: None,
    };
    assert_eq!(
        serde_json::to_value(
            query(&fixture.store, &fixture.key, 1, &params, &flag)
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        serde_json::to_value(old.reply(1)).unwrap()
    );
    let params = Params {
        branch_id: "1".into(),
        first_hour: Some(40),
        last_hour: Some(57),
    };
    let reply = query(&fixture.store, &fixture.key, 1, &params, &flag)
        .unwrap()
        .unwrap();
    assert_eq!(reply.pairs.len(), 5_000);
    assert_eq!(reply.read_hours, [34]);
    assert!(reply.pairs.iter().all(|pair| pair.runs[0].first_hour == 34));
}

fn source(replay: Replay) -> Source {
    Source::from_replay(
        crate::SourceKey {
            source_id: "prepared-placements".into(),
            revision: "synthetic".into(),
        },
        "fixtures/operative-placements.battleplan".into(),
        std::sync::Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[test]
fn prepared_placements_match_commits_resume_append_and_identity_gaps() {
    let mut fixture = Fixture::new();
    let mut state = Prepared::default();
    let mut old = Scan::default();
    let flag = AtomicBool::new(false);
    let source = source(
        Replay::from_records(
            test_fixtures::CAMPAIGN.into(),
            &[test_fixtures::records(
                1,
                vec![frame(10, true, vec![operative(1)])],
            )],
        )
        .unwrap(),
    );
    let context = LoadContext { content: None };
    for phase in 0..3 {
        let mut writer = if phase == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        for hour in 10 + phase * 48..58 + phase * 48 {
            let mut input = frame(hour, (hour - 10) % 24 == 0, vec![operative(1)]);
            if hour == 63 {
                *input.tables[TableId::OperativeMemberships.index()] = TableView::Unavailable {
                    reason: "synthetic missed membership".into(),
                };
            }
            if hour >= 90 {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                else {
                    panic!("countries");
                };
                rows[1].tag = b"Mod alias / opaque".to_vec();
            }
            old.observe(&source, &context, &input).unwrap();
            state.observe(&source, &context, &input).unwrap();
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let params = Params {
            branch_id: "1".into(),
            first_hour: None,
            last_hour: None,
        };
        let actual = query(&fixture.store, &fixture.key, 1, &params, &flag)
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(old.reply(1)).unwrap()
        );
        state = series::read_reply(&fixture.store, &fixture.key, "operative-placements-1", 0)
            .unwrap()
            .unwrap();
        assert!(state.scan.history.reads.len() <= 1);
        assert!(
            state
                .scan
                .history
                .pairs
                .values()
                .all(|pair| pair.runs.len() == 1)
        );
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_placement_checkpoint_does_not_retain_daily_provenance() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let source = source(
        Replay::from_records(
            test_fixtures::CAMPAIGN.into(),
            &[test_fixtures::records(
                1,
                vec![frame(10, true, vec![operative(1)])],
            )],
        )
        .unwrap(),
    );
    let context = LoadContext { content: None };
    let mut state = Prepared::default();
    let mut short = 0;
    for day in 0..100 {
        for hour in day * 24..(day + 1) * 24 {
            state
                .observe(
                    &source,
                    &context,
                    &frame(hour, hour % 24 == 0, vec![operative(1)]),
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if day == 1 {
            short = serde_json::to_vec(&state).unwrap().len();
        }
    }
    let long = serde_json::to_vec(&state).unwrap().len();
    eprintln!("placement checkpoint bytes: 2 days={short}, 100 days={long}");
    assert!(long < short * 2);
    assert_eq!(state.scan.history.reads.len(), 1);
    assert_eq!(state.scan.history.periods.len(), 1);
}

#[test]
fn intel_shared_driver_placements_equal_reads_are_flat_and_exact() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use crate::products::{
        intelligence_prepared::{measured, work},
        operative_rows::CountryInputs,
    };
    let scaled = observer_tests::scaled_work(8, |hours| {
        let first = frame(100, true, vec![operative(1)]);
        let frames: Vec<_> = (0..=hours)
            .map(|offset| {
                let mut frame = first.clone();
                frame.hour += offset as i32;
                frame
            })
            .collect();
        let source = source(Replay::from_records(
            test_fixtures::CAMPAIGN.into(),
            &[test_fixtures::records(1, frames.clone())],
        )?);
        let context = LoadContext { content: None };
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut state = Prepared::default();
        let mut countries = CountryInputs::default();
        let changes = FrameChangeTracker::new().observe(&first, source.replay_ref().format);
        countries.update(&first, Some(&changes));
        state.observe_bound(&first, &changes, &countries, &Ok(None))?;
        let baseline = measured();
        let delta = observer_tests::unchanged_frames(
            baseline,
            &first,
            hours as i32,
            source.replay_ref().format,
            |frame, changes| {
                let restored = if frame.hour == first.hour + (hours / 2) as i32 {
                    let mut writer = fixture.writer();
                    state.flush(&mut writer, 1, &flag)?;
                    writer.commit()?;
                    state = series::read_reply(
                        &fixture.store,
                        &fixture.key,
                        "operative-placements-1",
                        0,
                    )?
                    .unwrap();
                    countries = CountryInputs::default();
                    Some(FrameChangeTracker::new().observe(frame, source.replay_ref().format))
                } else {
                    None
                };
                let changes = restored.as_ref().unwrap_or(changes);
                countries.update(frame, Some(changes));
                state.observe_bound(frame, changes, &countries, &Ok(None))?;
                Ok(measured())
            },
        )?;
        let mut writer = fixture.append_writer();
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        let mut old = Scan::default();
        for frame in &frames {
            old.observe(&source, &context, frame)?;
        }
        let actual = query(
            &fixture.store,
            &fixture.key,
            1,
            &Params {
                branch_id: "1".into(),
                first_hour: None,
                last_hour: None,
            },
            &flag,
        )?
        .unwrap();
        observer_tests::assert_reply_parity(&actual, &old.reply(1))?;
        work(|work| *work = Default::default());
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].history_visits, 1);
}

#[test]
fn intel_shared_driver_placements_one_row_ignores_ten_times_closed_pairs() {
    use crate::prepared::observer_tests;
    use crate::products::intelligence_prepared::{measured, work};
    let scaled = observer_tests::scaled_work(8, |history| {
        let initial = frame(100, true, vec![operative(1)]);
        let source = source(Replay::from_records(
            test_fixtures::CAMPAIGN.into(),
            &[test_fixtures::records(1, vec![initial.clone()])],
        )?);
        let context = LoadContext { content: None };
        let mut state = Prepared::default();
        let mut input = initial;
        for index in 0..history {
            input.hour = 100 + index as i32;
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            rows[1].tag = format!("target-{index}").into_bytes();
            state.observe(&source, &context, &input)?;
        }
        assert_eq!(state.scan.history.pairs.len(), history);
        input.hour += 1;
        let TableView::Complete(TableRows::OperativeMemberships(rows)) =
            &mut *input.tables[TableId::OperativeMemberships.index()]
        else {
            unreachable!()
        };
        rows[0].name = b"Changed one operative".to_vec().into();
        work(|work| *work = Default::default());
        state.observe(&source, &context, &input)?;
        Ok(measured())
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].emitted, 1);
}
