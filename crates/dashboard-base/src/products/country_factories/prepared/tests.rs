use super::*;
use crate::test_fixtures::scale_frame;
use crate::test_fixtures::{CAMPAIGN, records};
use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

#[test]
fn factory_point_queries_share_one_reached_observation_without_folding_the_block() {
    use crate::prepared::query::{frames, observations, progressive};
    use crate::test_fixtures::{file_source, replay};
    let reached = frame(31, 2);
    let (source, root) = file_source(replay((0..32).map(|hour| frame(hour, 2)).collect()));
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    let params = json!({"branchId":"1", "tags":[tag], "family":{"kind":"country-factories"},
        "from":31,"to":31,"width":1});
    let cold = query(&source, "series-summary", &params, &context, &flag).unwrap();
    assert_eq!(frames::reads(&source), (0, 1));
    let expected = sample(&reached, None).unwrap();
    let values: Vec<_> = (0..fields().len())
        .map(|index| expected[&(tag.clone(), index.to_string())].1.unwrap() as i32)
        .collect();
    assert_eq!(cold["series"][0]["runs"][0]["values"], json!(values));
    let mut states = params.clone();
    states["reads"] = json!([{"tag":tag,"hour":31}]);
    let (states, refused) = progressive::without_block_decodes(|| {
        query(&source, "series-state", &states, &context, &flag)
    });
    assert!(!refused);
    assert_eq!(
        states.unwrap()["series"][0]["runs"][0]["values"],
        json!(values)
    );
    assert_eq!(
        observations::usage(&source, "country-factories:points").2,
        1
    );
    assert_eq!(frames::reads(&source), (0, 1));
    let _ = std::fs::remove_dir_all(root);
}

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(
            (0..count)
                .map(|index| CountryFactoriesRow {
                    country: 100_000 + index,
                    military_total_raw: (i64::from(index) + i64::from(hour)) * 100_000 + 99_999,
                    civilian_total_raw: -299_999,
                    consumer_goods: (index % 5) as i32,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// Two countries share one alias: X (slot 0) holds `AAA` through hour 4 and
/// `RNM` from 5, when Y (slot 1) takes `AAA` from `BBB`. X's factories change
/// at 2 and 7, Y's at 3 and 8; X always has more.
fn alias_frame(hour: i32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, 2);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        let (x, y): (&[u8], &[u8]) = if hour < 5 {
            (b"AAA", b"BBB")
        } else {
            (b"RNM", b"AAA")
        };
        rows[0].tag = x.to_vec();
        rows[1].tag = y.to_vec();
    }
    let military = |country: u32| match country {
        100_000 if hour >= 7 => 15,
        100_000 if hour >= 2 => 12,
        100_000 => 10,
        _ if hour >= 8 => 8,
        _ if hour >= 3 => 6,
        _ => 4,
    };
    *frame.tables[TableId::CountryFactories.index()] =
        TableView::Complete(TableRows::CountryFactories(
            [100_000, 100_001]
                .into_iter()
                .map(|country| CountryFactoriesRow {
                    country,
                    military_total_raw: military(country) * 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// A factory event page and its clusters count a tag's changes only while it
/// is a member, held by a selected country or ranked in the Top N, inside its
/// `within`; from a partial prefix they name its coverage and the hours pending.
#[test]
fn factory_events_count_only_member_hours_and_name_their_prepared_prefix() {
    use crate::products::country_history::HistoryScan;
    use crate::test_fixtures::{file_source, replay};
    let frames: Vec<_> = (0..=11).map(alias_frame).collect();
    let (source, root) = file_source(replay(frames.clone()));
    let (store, key) = crate::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let context = LoadContext { content: None };
    // Prepared through 9 of the recorded 0 to 11.
    let mut prepared = Prepared::default();
    let mut identities = HistoryScan::new(false, &[]);
    for frame in &frames[..10] {
        prepared
            .observe(
                frame,
                source.replay_ref(),
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, source.replay_ref().format),
            )
            .unwrap();
        identities.observe(frame);
    }
    let mut writer = store.replace(key.clone()).unwrap();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer
        .write_at(
            "country-identities-1",
            0,
            serde_json::to_vec(&identities).unwrap().as_slice(),
        )
        .unwrap();
    crate::prepared::coverage::publish_coverage(&mut writer, 1, 0, 9, false).unwrap();
    writer.commit().unwrap();
    let ask = |product: &str, extra: Value| {
        let mut params = json!({"branchId":"1","family":{"kind":"country-factories"},
            "pageSize":100});
        for (field, value) in extra.as_object().unwrap() {
            params[field] = value.clone();
        }
        query(&source, product, &params, &context, &flag).unwrap()
    };
    let rows = |page: &Value| -> Vec<(String, i64)> {
        page["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["tag"].as_str().unwrap().to_owned(),
                    row["hour"].as_i64().unwrap(),
                )
            })
            .collect()
    };
    let changes = |pairs: &[(&str, i64)]| -> Vec<(String, i64)> {
        pairs
            .iter()
            .map(|&(tag, hour)| (tag.to_owned(), hour))
            .collect()
    };
    let pending = json!([{"firstHour":10,"lastHour":11}]);
    let top = json!({"kind":"country-factories","top":{"count":1,"mode":"built","groups":[]}});
    for selection in [json!({"tags":["RNM"]}), json!({"family":top})] {
        // X's changes under both its tags, never Y's under `AAA` from 5.
        let mut page = selection.clone();
        page["to"] = json!(11);
        let reply = ask("events-page", page.clone());
        assert_eq!(
            rows(&reply),
            changes(&[("AAA", 0), ("AAA", 2), ("RNM", 5), ("RNM", 7)]),
            "{selection}"
        );
        assert_eq!(reply["total"], "4", "{selection}");
        assert_eq!(reply["coverage"]["lastHour"], 9, "{selection}");
        assert_eq!(reply["coverage"]["complete"], false, "{selection}");
        assert_eq!(reply["pending"], pending, "{selection}");
        page["within"] = json!({"AAA":[{"firstHour":0,"lastHour":0}]});
        assert_eq!(
            rows(&ask("events-page", page)),
            changes(&[("AAA", 0), ("RNM", 5), ("RNM", 7)]),
            "{selection}"
        );
        let mut clusters = selection.clone();
        for (field, value) in [
            ("tag", json!("AAA")),
            ("from", json!(0)),
            ("to", json!(11)),
            ("width", json!(18)),
        ] {
            clusters[field] = value;
        }
        let reply = ask("events-clusters", clusters);
        assert_eq!(
            reply["summary"]["clusters"],
            json!([{"firstHour":0,"lastHour":2,"firstOrdinal":"0","count":"2"}]),
            "{selection}"
        );
        assert_eq!(reply["coverage"]["lastHour"], 9, "{selection}");
        assert_eq!(reply["pending"], pending, "{selection}");
    }
    // Without a selection, a tag's clusters count every holder's changes.
    let all = ask("events-clusters", json!({"tag":"AAA","from":0,"to":11,"width":18}));
    assert_eq!(all["summary"]["clusters"][0]["count"], "4");
    // X's last change is at 7: navigation never stops at Y's change under `AAA` at 8.
    let next = ask(
        "series-next-change",
        json!({"tags":["RNM"],"from":7,"firstHour":0,"lastHour":9,"key":"ArrowRight"}),
    );
    assert_eq!(next["hour"], json!(7.0));
    store.clear().unwrap();
    drop(source);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn progressive_factory_sampler_matches_observer_on_every_key() {
    let frames = [frame(0, 24), frame(1, 24)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut observer = Prepared::default();
    for frame in &frames {
        let samples = sample(frame, None).unwrap();
        assert_eq!(samples.len(), 24 * fields().len());
        observer
            .observe(
                frame,
                &replay,
                &crate::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        for ((tag, field), (identity, amount)) in &samples {
            assert_eq!(
                *identity,
                observer.scan.tracks[tag].last.map(|(identity, _)| identity)
            );
            let key = observer.keys[&series::field_key(tag, field)];
            assert_eq!(
                observer.parts.state(key).unwrap().last.unwrap().value,
                amount.unwrap()
            );
        }
        for (tag, track) in &observer.scan.tracks {
            let key = observer.keys[&series::field_key(tag, "")];
            for index in 0..21 {
                assert_eq!(
                    observer.values[index]
                        .state(key)
                        .unwrap()
                        .last
                        .unwrap()
                        .value,
                    i128::from(count_for(
                        &track.runs.last().unwrap().values,
                        &rank_selection(index)
                    ))
                );
            }
        }
    }
}

#[test]
fn progressive_factory_sampler_keeps_absent_and_ambiguous_rows_unknown() {
    let mut frame = frame(0, 2);
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *frame.tables[TableId::CountryFactories.index()]
    {
        rows.push(rows[0].clone());
    }
    let tag = String::from_utf8(crate::test_fixtures::scale_tag(0)).unwrap();
    let requested = vec![
        (tag.clone(), "0".into()),
        ("tail absent".into(), "0".into()),
    ];
    let samples = sample(&frame, Some(&requested)).unwrap();
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[&requested[0]], (Some((0, 100_000)), None));
    assert_eq!(samples[&requested[1]], (None, None));
    assert!(sample(&frame, Some(&[(tag, "31".into())])).is_err());
}

/// Hours whose keys, links and factory rows are the hour before's step in
/// constant time to the state observing them reaches, an unresolved row's
/// count included, and on to the same state at the next changed hour. A step
/// that observing would take otherwise (after a gap, with another read flag,
/// read back from a continuation, or with countries that did not join)
/// declines and changes nothing.
#[test]
fn a_quiet_hour_steps_to_the_state_observing_it_reaches() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    let mut first = frame(100, 4);
    // A row no link names: an unreadable count every hour.
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *first.tables[TableId::CountryFactories.index()]
    {
        rows.push(CountryFactoriesRow {
            country: 999_999,
            ..Default::default()
        });
    }
    let replay = Replay::from_records(
        crate::test_fixtures::CAMPAIGN.into(),
        &[crate::test_fixtures::records(1, vec![first.clone()])],
    )
    .unwrap();
    let state = |prepared: &Prepared| serde_json::to_value(prepared).unwrap();
    // Each later hour shares every table of the first: nothing changed.
    let at = |hour: i32| {
        let mut frame = first.clone();
        frame.hour = hour;
        frame
    };
    let read = |hour: i32| {
        crate::metadata::slot_read(
            replay.format,
            &first.metrics,
            TableId::CountryFactories,
            hour,
        )
    };
    let mut tracker = FrameChangeTracker::new();
    let changes = tracker.observe(&first, replay.format);
    let (mut observed, mut stepped) = (Prepared::default(), Prepared::default());
    observed.observe(&first, &replay, &changes).unwrap();
    stepped.observe(&first, &replay, &changes).unwrap();
    for hour in 101..=105 {
        let frame = at(hour);
        let changes = tracker.observe(&frame, replay.format);
        assert!(
            !changes.rows_changed[TableId::CountryFactories.index()],
            "hour {hour} is quiet"
        );
        observed.observe(&frame, &replay, &changes).unwrap();
        assert!(
            stepped.advance_quiet(hour, read(hour)).unwrap(),
            "hour {hour} steps"
        );
        assert_eq!(state(&stepped), state(&observed), "hour {hour}");
    }
    assert!(
        state(&stepped)["scan"]["result"]["unreadable"]
            .as_u64()
            .unwrap()
            >= 6,
        "the unresolved row counts at every hour"
    );
    // Declined steps change nothing.
    let held = state(&stepped);
    assert!(!stepped.advance_quiet(107, read(107)).unwrap(), "a gap");
    assert!(
        !stepped.advance_quiet(106, !read(106)).unwrap(),
        "another read flag"
    );
    assert_eq!(state(&stepped), held);
    let mut reloaded: Prepared = serde_json::from_value(held).unwrap();
    assert!(
        !reloaded.advance_quiet(106, read(106)).unwrap(),
        "read back from a continuation, the next hour needs its frame"
    );
    // The next changed hour leaves both in the same state.
    let mut changed = at(106);
    if let TableView::Complete(TableRows::CountryFactories(rows)) =
        &mut *changed.tables[TableId::CountryFactories.index()]
    {
        rows[0].military_total_raw += 300_000;
    }
    let changes = tracker.observe(&changed, replay.format);
    observed.observe(&changed, &replay, &changes).unwrap();
    stepped.observe(&changed, &replay, &changes).unwrap();
    assert_eq!(state(&stepped), state(&observed));
    // Countries that did not join leave every later hour to its frame.
    let mut unjoined = first.clone();
    *unjoined.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "Countries not read".into(),
    };
    let mut alone = Prepared::default();
    alone
        .observe(
            &unjoined,
            &replay,
            &FrameChangeTracker::new().observe(&unjoined, replay.format),
        )
        .unwrap();
    assert!(!alone.advance_quiet(101, read(101)).unwrap());
}

#[test]
fn phase_b_unchanged_hours_and_retained_holders_are_flat_and_one_row_is_bounded() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100, 128);
        if let TableView::Complete(TableRows::CountryFactories(rows)) =
            &mut *first.tables[TableId::CountryFactories.index()]
        {
            rows.truncate(1);
        }
        let replay = Replay::from_records(
            crate::test_fixtures::CAMPAIGN.into(),
            &[crate::test_fixtures::records(1, vec![first.clone()])],
        )?;
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for index in 0..history {
            first.hour = 100 + index as i32;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *first.tables[TableId::Countries.index()]
            {
                rows[0].tag = format!("retired / {index}").into_bytes();
            }
            state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        }
        let delta = observer_tests::unchanged_frames(
            state.scan.work,
            &first,
            hours,
            replay.format,
            |frame, changes| {
                state.observe(frame, &replay, changes)?;
                Ok(state.scan.work)
            },
        )?;
        assert_eq!(delta, observer_tests::Work::default());
        first.hour += hours;
        tracker.observe(&first, replay.format);
        first.hour += 1;
        if let TableView::Complete(TableRows::CountryFactories(rows)) =
            &mut *first.tables[TableId::CountryFactories.index()]
        {
            rows[0].military_total_raw += 100_000_000;
        }
        let before = state.scan.work;
        state.observe(&first, &replay, &tracker.observe(&first, replay.format))?;
        let after = state.scan.work;
        let changed = observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        };
        assert!(
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 60,
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
fn phase_b_deferred_native_runs_match_eager_after_two_checkpoints_resume_append_gaps_and_unknowns()
{
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let mut lazy = Scan::default();
    let mut oracle = Scan::default();
    let mut tracker = FrameChangeTracker::new();
    let initial = frame(100, 3);
    for hour in [100, 101, 102, 103, 104, 105, 106, 107, 108, 111, 112, 113] {
        let mut next = initial.clone();
        next.hour = hour;
        if hour >= 106
            && let TableView::Complete(TableRows::Countries(rows)) =
                &mut *next.tables[TableId::Countries.index()]
        {
            rows[0].tag = b"renamed / first".to_vec();
        }
        if hour == 107 {
            *next.tables[TableId::CountryFactories.index()] = TableView::Unavailable {
                reason: "synthetic refusal".into(),
            };
        }
        oracle.observe(&next, replay_data::format::FormatVersion::CURRENT);
        lazy.observe_changes(
            &next,
            replay_data::format::FormatVersion::CURRENT,
            Some(&tracker.observe(&next, replay_data::format::FormatVersion::CURRENT)),
        );
        if hour == 103 || hour == 108 {
            lazy.seal();
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    observer_tests::assert_reply_parity(&lazy.finish(), &oracle.finish()).unwrap();
}
