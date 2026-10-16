use super::*;
use crate::test_fixtures::{CAMPAIGN, records};
use replay::{frame::TableView, rows::TableRows};

#[test]
fn supply_phase_b_defines_cache_success_and_named_failure_before_observation() {
    for declared in [true, false] {
        let files = if declared {
            vec![(
                "common/defines/synthetic.lua",
                "NDefines={NMilitary={SUPPLY_GRACE=11,LOW_SUPPLY=.63},NGraphics={SUPPLY_UNIT_COUNTER_VERY_LOW_THRESHOLD=.17,SUPPLY_UNIT_COUNTER_LOW_THRESHOLD=.42}}",
            )]
        } else {
            Vec::new()
        };
        let (catalogue, _folder) = crate::catalogue::tests::folder_catalogue(&files);
        let layers = &catalogue.content.layers;
        let binding = layers.derived::<crate::catalogue::CachedResult<Defines>>();
        let configured = content_defines(layers);
        assert_eq!(configured.is_ok(), declared);
        for _ in 0..10 {
            match (&configured, content_defines(layers)) {
                (Ok(expected), Ok(actual)) => assert_eq!(*expected, actual),
                (Err(expected), Err(actual)) => {
                    assert_eq!(expected.kind(), actual.kind());
                    assert_eq!(expected.to_string(), actual.to_string());
                }
                _ => panic!("fixed content outcome changed"),
            }
        }
        assert_eq!(binding.loads(), 1);
    }
}

#[test]
fn supply_phase_b_refused_configuration_observes_without_loading_or_visiting_rows() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let (_, folder) = crate::catalogue::tests::folder_catalogue(&[]);
    let frame = crate::products::supply_health::tests::frame(100);
    let (source, context) =
        crate::test_fixtures::source_with_content(vec![frame.clone()], &folder.0);
    let content = source
        .content(&context.content.as_ref().unwrap().options)
        .unwrap();
    let binding = content
        .layers
        .derived::<crate::catalogue::CachedResult<Defines>>();
    let mut prepared = Prepared::default();
    prepared
        .configure(&source, context.content.as_ref())
        .unwrap();
    assert!(
        prepared
            .failure
            .as_ref()
            .is_some_and(|reason| reason.contains("Supply health refused"))
    );
    prepared
        .observe(
            &frame,
            &FrameChangeTracker::new().observe(&frame, source.replay_ref().format),
        )
        .unwrap();
    let snapshot = |state: &Prepared| observer_tests::Work {
        content_loads: binding.loads(),
        ..state.scan.work
    };
    let work = observer_tests::unchanged_frames(
        snapshot(&prepared),
        &frame,
        80,
        source.replay_ref().format,
        |frame, changes| {
            prepared.observe(frame, changes)?;
            Ok(snapshot(&prepared))
        },
    )
    .unwrap();
    assert_eq!(work, observer_tests::Work::default());
    assert_eq!(binding.loads(), 1);
}

#[test]
fn supply_phase_b_work_is_flat_over_unchanged_hours_and_retained_aliases() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use crate::products::parts_race_prepared::progressive_tests::Fixture;
    let fixture = Fixture::new();
    let content = fixture
        .source
        .content(&fixture.context.content.as_ref().unwrap().options)
        .unwrap();
    let binding = content
        .layers
        .derived::<crate::catalogue::CachedResult<Defines>>();
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut state = Prepared::default();
        state.configure(&fixture.source, fixture.context.content.as_ref())?;
        let mut frame = crate::products::supply_health::tests::frame(100);
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            frame.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("history / {index}").into_bytes();
            }
            state.observe(
                &frame,
                &tracker.observe(&frame, fixture.source.replay_ref().format),
            )?;
        }
        let snapshot = |state: &Prepared| observer_tests::Work {
            content_loads: binding.loads(),
            ..state.scan.work
        };
        let work = observer_tests::unchanged_frames(
            snapshot(&state),
            &frame,
            hours,
            fixture.source.replay_ref().format,
            |frame, changes| {
                state.observe(frame, changes)?;
                Ok(snapshot(&state))
            },
        )?;
        assert_eq!(work, observer_tests::Work::default());
        let before = snapshot(&state);
        frame.hour += hours;
        tracker.observe(&frame, fixture.source.replay_ref().format);
        frame.hour += 1;
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *frame.tables[TableId::DivisionStatus.index()]
        {
            rows[0].supply_ratio = 99_999;
        }
        state.observe(
            &frame,
            &tracker.observe(&frame, fixture.source.replay_ref().format),
        )?;
        let after = snapshot(&state);
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 8 && changed.history_visits <= 2 && changed.emitted <= 8,
            "{changed:?}"
        );
        Ok(changed)
    };
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |hours| measure(1, hours as i32)).unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |history| measure(history, 8)).unwrap(),
    );
}

#[test]
fn supply_phase_b_deferred_runs_match_oracle_after_two_commits_resume_append_and_gap() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let defines = Defines {
        grace: 10,
        low: 60_000,
        very_low_icon: 15_000,
        low_icon: 40_000,
    };
    let first = crate::products::supply_health::tests::frame(100);
    let mut frames = (100..=105)
        .map(|hour| {
            let mut frame = first.clone();
            frame.hour = hour;
            frame
        })
        .collect::<Vec<_>>();
    let mut renamed = first.clone();
    renamed.hour = 108;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows[0].tag = b"renamed holder".to_vec();
    }
    frames.push(renamed.clone());
    renamed.hour = 109;
    frames.push(renamed);
    let mut fixture = crate::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared {
        defines: Some(defines),
        ..Default::default()
    };
    let mut oracle = Scan::new(&["GER".into(), "renamed holder".into()]);
    let mut tracker = FrameChangeTracker::new();
    for (batch, range) in [(0, 0..3), (1, 3..6), (2, 6..8)] {
        let through = frames[range.end - 1].hour;
        for frame in &frames[range] {
            oracle.observe(frame, defines).unwrap();
            prepared
                .observe(
                    frame,
                    &tracker.observe(frame, replay::format::FormatVersion::CURRENT),
                )
                .unwrap();
        }
        let mut writer = if batch == 0 {
            fixture.writer()
        } else {
            fixture.append_writer()
        };
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        // Prepared continuation prunes closed runs; the open authenticated
        // endpoint and its supplying boundary still match the native builder.
        for series in &prepared.scan.series {
            let expected = &oracle.series[oracle.selected[&series.tag]];
            if let Some(run) = series.runs.last() {
                observer_tests::assert_reply_parity(run, expected.runs.last().unwrap()).unwrap();
            }
        }
        let flag = AtomicBool::new(false);
        let mut counts =
            Reader::new(&fixture.store, &fixture.key, "supply-runs-1".into(), &flag).unwrap();
        for series in &oracle.series {
            if let Some(&key) = prepared.scan.selected.get(&series.tag) {
                for hour in 100..=through {
                    let expected = series
                        .runs
                        .iter()
                        .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                    let actual = counts.value_at(key as u64, hour).unwrap();
                    assert_eq!(
                        actual.is_some(),
                        expected.is_some(),
                        "{} at {hour}",
                        series.tag
                    );
                    if let Some(expected) = expected {
                        let (_, event) = counts.latest_event(key as u64, hour).unwrap().unwrap();
                        assert_eq!(
                            (
                                event["noSupply"].as_u64(),
                                event["strained"].as_u64(),
                                event["inSupply"].as_u64()
                            ),
                            (
                                Some(u64::from(expected.no_supply)),
                                Some(u64::from(expected.strained)),
                                Some(u64::from(expected.in_supply))
                            )
                        );
                    }
                }
            }
        }
        let saved = serde_json::to_value(&prepared).unwrap();
        for joined in [
            "identities",
            "owners",
            "country_keys",
            "roster_keys",
            "members",
            "counts",
        ] {
            assert!(
                saved["scan"].get(joined).is_none(),
                "serialized runtime join {joined}"
            );
        }
        prepared = serde_json::from_value(saved).unwrap();
        tracker.invalidate();
    }
}

#[test]
fn share_summaries_preserve_denominator_peaks_and_exact_zero_counts() {
    let mut frames = Vec::new();
    for (offset, counts) in [[1, 1, 1], [5, 5, 2], [10, 100, 50], [0, 0, 0]]
        .into_iter()
        .enumerate()
    {
        let mut frame = crate::products::supply_health::tests::frame(100 + offset as i32);
        let mut divisions = Vec::new();
        let mut land = Vec::new();
        let mut status = Vec::new();
        for (bucket, count) in counts.into_iter().enumerate() {
            for _ in 0..count {
                let id = divisions.len() as u64 + 1;
                divisions.push(DivisionRow {
                    id,
                    roster_owner: 11,
                    ..Default::default()
                });
                land.push(DivisionLandInputsRow {
                    division: id,
                    ..Default::default()
                });
                status.push(DivisionStatusRow {
                    division: id,
                    supply_ratio: [0, 30_000, 100_000][bucket],
                    supply_grace_maximum: Recorded::Value(1_000_000),
                    supply_grace_remaining: Recorded::Value(0),
                    ..Default::default()
                });
            }
        }
        *frame.tables[TableId::Divisions.index()] =
            TableView::Complete(TableRows::Divisions(divisions));
        *frame.tables[TableId::DivisionLandInputs.index()] =
            TableView::Complete(TableRows::DivisionLandInputs(land));
        *frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(status));
        frames.push(frame);
    }
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let source = Source::from_replay(
        crate::SourceKey {
            source_id: "shares".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut changes = crate::prepared::frame_changes::FrameChangeTracker::new();
    let mut prepared = Prepared {
        defines: Some(Defines {
            grace: 10,
            low: 60_000,
            very_low_icon: 15_000,
            low_icon: 40_000,
        }),
        ..Default::default()
    };
    for frame in &frames {
        prepared
            .observe(frame, &changes.observe(frame, source.replay_ref().format))
            .unwrap();
    }
    let fixture = crate::prepared::series::tests::Fixture::new();
    let mut writer = fixture.writer();
    let flag = AtomicBool::new(false);
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    let mut shares = Reader::new(
        &fixture.store,
        &fixture.key,
        "supply-values-1-0".into(),
        &flag,
    )
    .unwrap();
    let summary = shares
        .summary(0, (100, 102), (100.0, 102.0, 1), SHARE_DECIMALS)
        .unwrap();
    assert_eq!(summary.buckets[0].unwrap().max_hour, 101);
    assert!(
        shares.value_at(0, 103).unwrap().is_none(),
        "zero divisions have no defined share"
    );
    let mut counts =
        Reader::new(&fixture.store, &fixture.key, "supply-runs-1".into(), &flag).unwrap();
    assert!(
        counts.value_at(0, 103).unwrap().is_some(),
        "zero divisions are a known count"
    );
    let (_, zero) = counts.latest_event(0, 103).unwrap().unwrap();
    assert_eq!(
        (
            zero["noSupply"].as_u64(),
            zero["strained"].as_u64(),
            zero["inSupply"].as_u64()
        ),
        (Some(0), Some(0), Some(0))
    );
}

#[test]
#[cfg(feature = "slow-tests")]
fn supply_progressive_sampler_matches_observer_every_key_at_5000_countries() {
    use crate::products::parts_race_prepared::progressive_tests::{Fixture, frame, tag};
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    prepared
        .configure(&fixture.source, fixture.context.content.as_ref())
        .unwrap();
    let mut changes = crate::prepared::frame_changes::FrameChangeTracker::new();
    let defines = query_defines(&fixture.source, &fixture.context, &mut Vec::new()).unwrap();
    assert_eq!(
        defines.low, 63_000,
        "the recording's mod Defines supply the classifier"
    );
    for hour in [6, 7] {
        let frame = frame(hour, 5000);
        prepared
            .observe(
                &frame,
                &changes.observe(&frame, fixture.source.replay_ref().format),
            )
            .unwrap();
        let sampled = sample(&frame, None, Some(defines)).unwrap();
        assert_eq!(sampled.len(), 5000 * 6);
        assert_eq!(sampled, sample_rows(&prepared.scan, None).unwrap());
        for ((tag, field), (identity, value)) in &sampled {
            assert!(identity.is_some());
            if let Ok(bucket) = field.parse::<usize>() {
                let index = prepared.scan.selected[tag];
                assert_eq!(
                    *value,
                    prepared.values[bucket]
                        .state(index as u64)
                        .and_then(|state| state.last)
                        .filter(|run| run.last_hour == hour)
                        .map(|run| run.value)
                );
            }
        }
        if hour == 7 {
            assert_eq!(sampled[&(tag(0), "count-0".into())].1, None);
            assert_eq!(sampled[&(tag(1), "count-2".into())].1, Some(1));
        }
    }
    assert_eq!(
        sample(
            &frame(6, 1),
            Some(&[("absent".into(), "0".into())]),
            Some(defines)
        )
        .unwrap()
        .get(&("absent".into(), "0".into())),
        Some(&(None, None))
    );
    let mut zero = frame(6, 1);
    *zero.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(Vec::new()));
    *zero.tables[TableId::DivisionLandInputs.index()] =
        TableView::Complete(TableRows::DivisionLandInputs(Vec::new()));
    *zero.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(Vec::new()));
    let sampled = sample(&zero, None, Some(defines)).unwrap();
    assert_eq!(sampled[&(tag(0), "0".into())].1, None);
    assert_eq!(sampled[&(tag(0), "count-0".into())].1, Some(0));
    assert!(
        sample(&zero, None, None)
            .unwrap()
            .values()
            .all(|(_, amount)| amount.is_none()),
        "absent land inputs need no content thresholds or fabricated zeros"
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn supply_progressive_queries_join_prepared_prefix_and_tail_then_zoom_exactly_without_writing() {
    use crate::products::parts_race_prepared::progressive_tests::{Fixture, frame, tag};
    use serde_json::json;
    let fixture = Fixture::new();
    let params =
        |from| json!({"branchId":"1","tags":[tag(0),tag(1)],"from":from,"to":31,"width":32});
    let cold = fixture.query("supply-health", params(0));
    assert!(
        !cold["summaries"][0]["values"][0]["approximate"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        cold["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1))
    );
    let after_rename = fixture.query(
        "supply-health",
        json!({"branchId":"1", "tags":[tag(0)], "from":26,"to":31,"width":6}),
    );
    assert!(
        after_rename["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first"
                && !row["runs"].as_array().unwrap().is_empty())
    );
    assert!(fixture.store.saved_key().unwrap().is_none());
    let mut prepared = Prepared::default();
    prepared
        .configure(&fixture.source, fixture.context.content.as_ref())
        .unwrap();
    let mut changes = crate::prepared::frame_changes::FrameChangeTracker::new();
    for hour in 0..2 {
        let frame = frame(hour, 1);
        prepared
            .observe(
                &frame,
                &changes.observe(&frame, fixture.source.replay_ref().format),
            )
            .unwrap();
        let mut writer = if hour == 0 {
            fixture.store.replace(fixture.key.clone()).unwrap()
        } else {
            fixture.store.extend(&fixture.key).unwrap()
        };
        prepared
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        fixture.identities(&mut writer, hour);
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let ranges = fixture
        .store
        .range_count(&fixture.key, "supply-runs-1")
        .unwrap();
    let rough = fixture.query("supply-health", params(0));
    assert_eq!(rough["coverage"]["lastHour"], 1);
    let exact = fixture.query("supply-health", params(6));
    for row in exact["summaries"].as_array().unwrap() {
        for summary in row["values"].as_array().unwrap() {
            assert_eq!(summary["approximate"], json!([]));
        }
    }
    let primary = exact["series"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["tag"] == tag(0))
        .unwrap();
    assert!(
        primary["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"] == 9 && run["inSupply"] == 1)
    );
    assert!(
        !primary["runs"]
            .as_array()
            .unwrap()
            .iter()
            .any(|run| run["firstHour"].as_i64().unwrap() <= 7
                && run["lastHour"].as_i64().unwrap() >= 7),
        "the missing companion is a local unknown gap"
    );
    let aliases = fixture.query(
        "supply-health",
        json!({"branchId":"1","tags":[tag(0)],"from":0,"to":31,"width":32}),
    );
    assert!(
        aliases["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == "renamed / first")
    );
    assert!(
        !aliases["series"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["tag"] == tag(1))
    );
    let navigation = fixture.query("series-next-change", json!({"branchId":"1","family":{"kind":"supply-health"},"tags":[tag(0)],"key":"ArrowRight","from":8,"firstHour":6,"lastHour":31}));
    assert_eq!(navigation["hour"].as_f64(), Some(9.0));
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "supply-runs-1")
            .unwrap(),
        ranges
    );
    for hour in 2..4 {
        let frame = frame(hour, 2);
        prepared
            .observe(
                &frame,
                &changes.observe(&frame, fixture.source.replay_ref().format),
            )
            .unwrap();
    }
    let mut writer = fixture.store.extend(&fixture.key).unwrap();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    fixture.identities(&mut writer, 3);
    writer.commit().unwrap();
    assert_eq!(
        fixture.query("supply-health", params(0))["coverage"]["lastHour"],
        3
    );
}

#[test]
fn phase_b_supply_one_status_row_does_not_visit_empty_countries_or_retired_holders() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut frame = crate::products::parts_race_prepared::progressive_tests::frame(100, 128);
        for table in [
            TableId::Divisions,
            TableId::DivisionLandInputs,
            TableId::DivisionStatus,
        ] {
            match &mut *frame.tables[table.index()] {
                TableView::Complete(TableRows::Divisions(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::DivisionLandInputs(rows)) => rows.truncate(1),
                TableView::Complete(TableRows::DivisionStatus(rows)) => rows.truncate(1),
                _ => unreachable!(),
            }
        }
        let mut state = Prepared {
            defines: Some(Defines {
                grace: 10,
                low: 60_000,
                very_low_icon: 15_000,
                low_icon: 40_000,
            }),
            ..Default::default()
        };
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            frame.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("retired / {index}").into_bytes();
            }
            state.observe(
                &frame,
                &tracker.observe(&frame, replay::format::FormatVersion::CURRENT),
            )?;
        }
        let work = observer_tests::unchanged_frames(
            state.scan.work,
            &frame,
            hours,
            replay::format::FormatVersion::CURRENT,
            |frame, changes| {
                state.observe(frame, changes)?;
                Ok(state.scan.work)
            },
        )?;
        assert_eq!(work, observer_tests::Work::default());
        frame.hour += hours;
        tracker.observe(&frame, replay::format::FormatVersion::CURRENT);
        frame.hour += 1;
        if let TableView::Complete(TableRows::DivisionStatus(rows)) =
            &mut *frame.tables[TableId::DivisionStatus.index()]
        {
            rows[0].supply_ratio = 100_000;
        }
        let before = state.scan.work;
        state.observe(
            &frame,
            &tracker.observe(&frame, replay::format::FormatVersion::CURRENT),
        )?;
        let after = state.scan.work;
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 4,
            "{changed:?}"
        );
        Ok(changed)
    };
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |hours| measure(1, hours as i32)).unwrap(),
    );
    observer_tests::assert_flat(
        observer_tests::scaled_work(8, |history| measure(history, 8)).unwrap(),
    );
}
