use super::*;
use crate::prepared::series::{Reader, tests::Fixture};
use crate::products::country_history::HistoryScan;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

fn frame(hour: i32, tag: &[u8], slot: u32, id: u32, raw: Option<i64>) -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, hour, raw);
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            canonical_slot: slot,
            id,
            tag: tag.to_vec(),
            political_power_raw: raw,
            command_power_raw: raw.unwrap_or(i64::MAX),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow {
            id,
            canonical_slot: slot,
        }]));
    frame
}

#[test]
fn runs_drops_identities_unknowns_and_resume_match_history_exactly() {
    let fixture = Fixture::new();
    let frames = [
        frame(0, b"opaque / tag", 7, 11, Some(i64::MAX)),
        frame(1, b"opaque / tag", 7, 11, Some(i64::MAX - 1)),
        frame(2, b"opaque / tag", 7, 11, Some(i64::MIN)),
        frame(3, b"opaque / tag", 7, 11, None),
        frame(4, b"opaque / tag", 7, 11, Some(0)),
        frame(6, b"opaque / tag", 7, 11, Some(-5)),
        frame(7, b"opaque / tag", 8, 12, Some(-7)),
        frame(8, b"renamed", 8, 12, Some(-9)),
        frame(9, b"renamed", 8, 12, Some(-9)),
        frame(10, b"renamed", 8, 12, Some(-9)),
    ];
    let measures = [Measure::PoliticalPower, Measure::CommandPower];
    let mut oracle = HistoryScan::new(false, &measures);
    let mut prepared = MeasureAccumulator::new(0);
    let flag = AtomicBool::new(false);
    for (index, frame) in frames.iter().enumerate() {
        oracle.observe(frame);
        prepared.observe(frame).unwrap();
        if index == 3 || index == frames.len() - 1 {
            let mut writer = fixture.writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
            prepared.validate_checkpoint().unwrap();
        }
    }
    let oracle = oracle.finish();
    for actor in prepared.catalogue() {
        let expected = &oracle.countries[&actor.tag];
        let expected = match actor.measure.as_str() {
            "politicalPower" => &expected.political_power,
            "commandPower" => &expected.command_power,
            _ => unreachable!(),
        };
        let section = format!(
            "measure-{}-1",
            if actor.measure == "politicalPower" {
                "pp"
            } else {
                "cp"
            }
        );
        let mut reader = Reader::new(&fixture.store, &fixture.key, section, &flag).unwrap();
        let key = actor.actor.parse().unwrap();
        assert_eq!(
            actor.run_count.parse::<usize>().unwrap(),
            expected.runs.len()
        );
        for run in &expected.runs {
            let actual = reader.value_at(key, run.first_hour).unwrap().unwrap();
            assert_eq!(
                (
                    actual.first_hour,
                    actual.last_hour,
                    actual.value,
                    actual.break_before
                ),
                (
                    run.first_hour,
                    run.last_hour,
                    i128::from(run.value),
                    run.starts_segment
                )
            );
        }
        let summary = reader
            .summary(key, (0, 10), (0.25, 9.5, 100), MEASURE_DECIMALS)
            .unwrap();
        let wanted: Vec<_> = expected
            .runs
            .iter()
            .filter(|run| run.last_hour >= 0 && run.first_hour <= 9)
            .map(|run| crate::prepared_chart::ChartRun {
                first_hour: run.first_hour,
                last_hour: run.last_hour,
                value: run.value as f64 / 100_000.0,
                break_before: run.starts_segment,
            })
            .collect();
        assert_eq!(
            summary.runs.unwrap(),
            wanted,
            "unclipped legacy supplying runs, including indistinguishable plotted values"
        );
        for hour in 0..11 {
            let actual = reader.value_at(key, hour).unwrap();
            let expected = expected
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .map(|run| i128::from(run.value));
            assert_eq!(
                actual.map(|run| run.value),
                expected,
                "{} {hour}",
                actor.tag
            );
        }
        let drops = reader.events(&[key], i32::MIN, i32::MAX, 0, 100).unwrap();
        assert_eq!(drops.rows.len(), expected.drops.len());
        for (actual, expected) in drops.rows.iter().zip(&expected.drops) {
            assert_eq!(
                &actual.row,
                &serde_json::json!({"hour":expected.hour,"beforeRaw":expected.before.to_string(),"afterRaw":expected.after.to_string()})
            );
        }
    }
}

#[test]
fn five_thousand_opaque_countries_and_mismatched_links_have_no_cap() {
    let fixture = Fixture::new();
    let rows: Vec<_> = (0..5000)
        .map(|id| CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country / {id}").into_bytes(),
            political_power_raw: Some(i64::from(id)),
            command_power_raw: -i64::from(id),
            ..Default::default()
        })
        .collect();
    let mut links: Vec<_> = (0..5000)
        .map(|id| CountryLinkRow {
            id,
            canonical_slot: id,
        })
        .collect();
    let mut accumulator = MeasureAccumulator::new(0);
    accumulator.observe_rows(0, Some((&rows, &links))).unwrap();
    assert_eq!(accumulator.catalogue().len(), 10_000);
    links[17].canonical_slot = 18;
    accumulator.observe_rows(1, Some((&rows, &links))).unwrap();
    assert!(accumulator.unresolved_countries > 0);
    let actor = accumulator.countries["country / 17"].political.actor;
    assert!(
        accumulator.batches[0]
            .state(actor)
            .and_then(|state| state.last)
            .is_none_or(|run| run.last_hour < 1)
    );
    let flag = AtomicBool::new(false);
    let mut writer = fixture.writer();
    accumulator.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "measure-pp-1")
            .unwrap(),
        Some(1)
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "measure-cp-1")
            .unwrap(),
        Some(1)
    );
    let actor = accumulator
        .catalogue()
        .into_iter()
        .find(|actor| actor.tag == "country / 4999" && actor.measure == "commandPower")
        .unwrap();
    let mut reader =
        Reader::new(&fixture.store, &fixture.key, "measure-cp-1".into(), &flag).unwrap();
    let run = reader
        .value_at(actor.actor.parse().unwrap(), 1)
        .unwrap()
        .unwrap();
    assert_eq!((run.first_hour, run.last_hour, run.value), (0, 1, -4999));
}

#[test]
fn stateless_sample_and_observer_share_values_for_all_five_thousand_countries() {
    let mut frame = crate::test_fixtures::scale_frame(1, 0, 5000);
    if let replay_frame::frame::TableView::Complete(replay_data::rows::TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for (index, row) in rows.iter_mut().enumerate() {
            row.political_power_raw = Some(index as i64 - 2500);
        }
    }
    let samples = sample(&frame, None).unwrap();
    assert_eq!(samples.len(), 10_000);
    let mut observer = MeasureAccumulator::new(0);
    observer.observe(&frame).unwrap();
    for ((tag, label), (identity, value)) in samples {
        let country = &observer.countries[&tag];
        let (batch, actor) = if label == "politicalPower" {
            (&observer.batches[0], country.political.actor)
        } else {
            (&observer.batches[1], country.command.actor)
        };
        assert_eq!(
            batch.state(actor).unwrap().last.unwrap().value,
            value.unwrap()
        );
        assert_eq!(observer.holders.get(&tag).copied(), identity);
    }
    let selected = [("not in catalogue".into(), "politicalPower".into())];
    assert_eq!(
        sample(&frame, Some(&selected)).unwrap()[&selected[0]],
        (None, None)
    );
}

fn stable_balances(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = crate::test_fixtures::scale_frame(1, hour, count);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        for (index, row) in rows.iter_mut().enumerate() {
            row.political_power_raw = Some(1000 + index as i64);
            row.command_power_raw = 2000 + index as i64;
        }
    }
    for table in [TableId::Countries, TableId::CountryLinks] {
        frame.metrics.table_ns[table.index()] = 1;
    }
    frame
}

fn measure_scaled_work(countries: u32) {
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let histories = observer_tests::scaled_work(2, |history| {
        let hours = observer_tests::scaled_work(3, |hours| {
            let mut observer = MeasureAccumulator::new(-(history as i32));
            for retired in 0..history {
                observer.observe(&frame(
                    retired as i32 - history as i32,
                    format!("retired/{retired}").as_bytes(),
                    7,
                    11,
                    Some(7),
                ))?;
            }
            let first = stable_balances(0, countries);
            observer.observe(&first)?;
            assert_eq!(observer.countries.len(), history + countries as usize);
            let unchanged = observer_tests::unchanged_frames(
                observer.work,
                &first,
                hours as i32,
                replay_data::format::FormatVersion::CURRENT,
                |frame, changes| {
                    assert!(changes.table_read[TableId::Countries.index()]);
                    observer.observe_changes(frame, changes)?;
                    Ok(observer.work)
                },
            )?;
            assert_eq!(
                unchanged,
                Work::default(),
                "unchanged native inputs do no keyed work"
            );
            let tag = std::str::from_utf8(&first.complete::<tables::Countries>()?[0].tag).unwrap();
            let actor = observer.countries[tag].political.actor;
            assert_eq!(
                observer.batches[0]
                    .state(actor)
                    .unwrap()
                    .last
                    .unwrap()
                    .last_hour,
                0,
                "held endpoints are materialized only at a change/publication"
            );
            let mut previous = first.clone();
            previous.hour = hours as i32;
            let mut tracker = FrameChangeTracker::new();
            tracker.observe(&previous, replay_data::format::FormatVersion::CURRENT);
            let mut unrelated = previous.clone();
            unrelated.hour += 1;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *unrelated.tables[TableId::Countries.index()]
            {
                rows[0].stability_raw += 1;
            }
            let before = observer.work;
            let changes = tracker.observe(&unrelated, replay_data::format::FormatVersion::CURRENT);
            observer.observe_changes(&unrelated, &changes)?;
            assert_eq!(
                observer.work.emitted, before.emitted,
                "unrelated Country fields never emit balances"
            );
            assert_eq!(observer.work.history_visits, before.history_visits);
            previous = unrelated;
            let mut changed = previous.clone();
            changed.hour += 1;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *changed.tables[TableId::Countries.index()]
            {
                rows[0].political_power_raw = Some(1001);
            }
            let changes = tracker.observe(&changed, replay_data::format::FormatVersion::CURRENT);
            let before = observer.work;
            observer.observe_changes(&changed, &changes)?;
            let work = Work {
                row_visits: observer.work.row_visits - before.row_visits,
                history_visits: observer.work.history_visits - before.history_visits,
                emitted: observer.work.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(
                work.row_visits,
                countries as usize * 2,
                "shared CountryKeys and numeric row projection only"
            );
            assert_eq!(
                work.history_visits, 0,
                "equal country keys and links require no retained-roster sweep"
            );
            assert_eq!(
                work.emitted, 1,
                "one value change emits one update, no other keys"
            );
            let fixture = Fixture::new();
            let flag = AtomicBool::new(false);
            let mut writer = fixture.writer();
            observer.flush(&mut writer, 1, &flag)?;
            writer.commit()?;
            let mut requests = 0;
            for (label, section) in [("politicalPower", "pp"), ("commandPower", "cp")] {
                let mut keys = Vec::new();
                let mut expected = Vec::new();
                for ((tag, field), (_, value)) in sample(&changed, None)? {
                    if field != label {
                        continue;
                    }
                    let country = &observer.countries[&tag];
                    keys.push(if section == "pp" {
                        country.political.actor
                    } else {
                        country.command.actor
                    });
                    expected.push(value);
                }
                let mut reader = Reader::new(
                    &fixture.store,
                    &fixture.key,
                    format!("measure-{section}-1"),
                    &flag,
                )?;
                requests += 1;
                assert!(
                    requests <= 2,
                    "one bulk read per field, no per-country request loop"
                );
                assert_eq!(reader.values_at(&keys, changed.hour)?, expected);
            }
            let mut reader =
                Reader::new(&fixture.store, &fixture.key, "measure-pp-1".into(), &flag)?;
            let prior = reader.value_at(actor, previous.hour)?.unwrap();
            assert_eq!(
                (prior.first_hour, prior.last_hour, prior.value),
                (0, previous.hour, 1000)
            );
            let actor = observer.countries[std::str::from_utf8(
                &first.complete::<tables::Countries>()?[countries as usize - 1].tag,
            )
            .unwrap()]
            .political
            .actor;
            let held = reader.value_at(actor, changed.hour)?.unwrap();
            assert_eq!((held.first_hour, held.last_hour), (0, changed.hour));
            let mut removed = changed.clone();
            removed.hour += 1;
            if let TableView::Complete(TableRows::Countries(rows)) =
                &mut *removed.tables[TableId::Countries.index()]
            {
                rows.remove(0);
            }
            let before = observer.work;
            let changes = tracker.observe(&removed, replay_data::format::FormatVersion::CURRENT);
            observer.observe_changes(&removed, &changes)?;
            assert_eq!(
                observer.work.emitted - before.emitted,
                2,
                "removing one holder closes only its two measure keys"
            );
            assert_eq!(
                observer.work.history_visits - before.history_visits,
                countries as usize
            );
            assert!(observer.work.row_visits - before.row_visits <= countries as usize * 2);
            assert_eq!(observer.countries[tag].political.value, None);
            assert_eq!(observer.countries[tag].command.value, None);
            Ok(work)
        })?;
        observer_tests::assert_flat(hours);
        Ok(hours[1])
    })
    .unwrap();
    observer_tests::assert_flat(histories);
}

#[test]
fn measure_shared_driver_keeps_unchanged_frames_and_one_change_flat_over_tenfold_history() {
    measure_scaled_work(3);
}

#[cfg(feature = "slow-tests")]
#[test]
fn measure_shared_driver_keeps_5000_country_work_flat_over_tenfold_hours_and_history() {
    measure_scaled_work(5000);
}

#[test]
fn measure_shared_facts_keep_hourly_diagnostics_without_rejoining_equal_fresh_or_carried_inputs() {
    use crate::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let measured = observer_tests::scaled_work(3, |hours| {
        let mut first = frame(0, b"opaque / tag", 7, 11, Some(100));
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *first.tables[TableId::Countries.index()]
        {
            rows.extend([
                CountryRow {
                    id: 12,
                    canonical_slot: 8,
                    tag: b"unresolved".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 13,
                    canonical_slot: 9,
                    tag: vec![255],
                    ..Default::default()
                },
                CountryRow {
                    id: 14,
                    canonical_slot: 10,
                    tag: vec![255],
                    ..Default::default()
                },
            ]);
        }
        if let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *first.tables[TableId::CountryLinks.index()]
        {
            rows.extend([
                CountryLinkRow {
                    id: 13,
                    canonical_slot: 9,
                },
                CountryLinkRow {
                    id: 14,
                    canonical_slot: 10,
                },
            ]);
        }
        let mut observer = MeasureAccumulator::new(0);
        let mut oracle = HistoryScan::new(false, &[Measure::PoliticalPower, Measure::CommandPower]);
        observer.observe(&first)?;
        oracle.observe(&first);
        let work = observer_tests::unchanged_frames(
            observer.work,
            &first,
            hours as i32,
            replay_data::format::FormatVersion::CURRENT,
            |frame, changes| {
                observer.observe_changes(frame, changes)?;
                oracle.observe(frame);
                Ok(observer.work)
            },
        )?;
        assert_eq!(work, Work::default());
        // Both timed equal acquisitions and a carried hour retain these numeric
        // operands; country balances do not add supplying-read boundaries.
        let mut carried = first.clone();
        carried.hour = hours as i32 + 1;
        carried.metrics.table_ns[TableId::Countries.index()] = 0;
        carried.metrics.table_ns[TableId::CountryLinks.index()] = 0;
        let before = observer.work;
        let mut tracker = FrameChangeTracker::new();
        let mut previous = first.clone();
        previous.hour = carried.hour - 1;
        tracker.observe(&previous, replay_data::format::FormatVersion::CURRENT);
        let changes = tracker.observe(&carried, replay_data::format::FormatVersion::CURRENT);
        assert!(!changes.table_read[TableId::Countries.index()]);
        observer.observe_changes(&carried, &changes)?;
        oracle.observe(&carried);
        assert_eq!(observer.work, before);
        let mut changed = carried.clone();
        changed.hour += 1;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *changed.tables[TableId::Countries.index()]
        {
            rows[0].political_power_raw = Some(99);
        }
        let changes = tracker.observe(&changed, replay_data::format::FormatVersion::CURRENT);
        observer.observe_changes(&changed, &changes)?;
        oracle.observe(&changed);
        assert_eq!(observer.countries["opaque / tag"].political.value, Some(99));
        let oracle = oracle.finish();
        assert_eq!(
            observer.unresolved_countries,
            oracle.unresolved_countries as u64
        );
        assert_eq!(observer.unnamed_countries, oracle.unnamed as u64);
        assert_eq!(observer.unresolved_countries, hours as u64 + 3);
        assert_eq!(
            observer.unnamed_countries,
            hours as u64 + 3,
            "a duplicate raw tag counts only its resolved row"
        );
        Ok(work)
    })
    .unwrap();
    observer_tests::assert_flat(measured);
}

fn assert_measure_oracle(
    fixture: &Fixture,
    observer: &MeasureAccumulator,
    frames: &[DecodedFrame],
) {
    use crate::products::country_measure::{FixedRun, MeasureSeries};
    let flag = AtomicBool::new(false);
    let mut oracle = HistoryScan::new(false, &[Measure::PoliticalPower, Measure::CommandPower]);
    for frame in frames {
        oracle.observe(frame);
    }
    let oracle = oracle.finish();
    assert_eq!(
        observer.unresolved_countries,
        oracle.unresolved_countries as u64
    );
    assert_eq!(observer.unnamed_countries, oracle.unnamed as u64);
    for actor in observer.catalogue() {
        let track = &oracle.countries[&actor.tag];
        let (track, section) = if actor.measure == "politicalPower" {
            (&track.political_power, "pp")
        } else {
            (&track.command_power, "cp")
        };
        let expected = MeasureSeries {
            tag: actor.tag.clone(),
            runs: track
                .runs
                .iter()
                .map(|run| FixedRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value_raw: run.value.to_string(),
                    starts_segment: run.starts_segment,
                })
                .collect(),
            drops: track
                .drops
                .iter()
                .map(|drop| BalanceDrop {
                    hour: drop.hour,
                    before_raw: drop.before.to_string(),
                    after_raw: drop.after.to_string(),
                })
                .collect(),
        };
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            format!("measure-{section}-1"),
            &flag,
        )
        .unwrap();
        let key = actor.actor.parse().unwrap();
        let events = reader
            .events(&[key], i32::MIN, i32::MAX, 0, u32::MAX)
            .unwrap();
        assert!(events.cursor.is_none());
        let actual = MeasureSeries {
            tag: actor.tag,
            runs: reader
                .runs(key, i32::MIN, i32::MAX)
                .unwrap()
                .into_iter()
                .map(|run| FixedRun {
                    first_hour: run.first_hour,
                    last_hour: run.last_hour,
                    value_raw: run.value.to_string(),
                    starts_segment: run.break_before,
                })
                .collect(),
            drops: events
                .rows
                .into_iter()
                .map(|row| serde_json::from_value(row.row).unwrap())
                .collect(),
        };
        crate::prepared::observer_tests::assert_reply_parity(&actual, &expected).unwrap();
        assert_eq!(actual.runs.len().to_string(), actor.run_count);
        assert_eq!(actual.drops.len().to_string(), actor.drop_count);
    }
}

#[test]
fn measure_shared_facts_match_oracle_across_commits_discard_resume_append_gaps_and_aliases() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut observer = MeasureAccumulator::new(0);
    let mut seen = Vec::new();
    let base = frame(0, b"opaque / tag", 7, 11, Some(100));
    let decreased = frame(2, b"opaque / tag", 7, 11, Some(90));
    for mut observed in [base.clone(), base, decreased.clone(), decreased] {
        observed.hour = seen.len() as i32;
        observer.observe(&observed).unwrap();
        seen.push(observed);
    }
    let mut writer = fixture.writer();
    observer.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_measure_oracle(&fixture, &observer, &seen);
    let checkpoint = serde_json::to_vec(&observer).unwrap();
    observer
        .observe(&frame(4, b"opaque / tag", 7, 11, Some(-999)))
        .unwrap();
    let mut discarded = fixture.writer();
    observer.flush(&mut discarded, 1, &flag).unwrap();
    drop(discarded);
    observer = serde_json::from_slice(&checkpoint).unwrap();
    observer.validate_checkpoint().unwrap();
    let missing_value = frame(4, b"opaque / tag", 7, 11, None);
    let recovered = frame(5, b"opaque / tag", 7, 11, Some(100));
    let mut unavailable = recovered.clone();
    unavailable.hour = 6;
    *unavailable.tables[TableId::CountryLinks.index()] = TableView::Unavailable {
        reason: "synthetic refusal".into(),
    };
    let mut carried_refusal = unavailable.clone();
    carried_refusal.hour = 7;
    for observed in [missing_value, recovered, unavailable, carried_refusal] {
        observer.observe(&observed).unwrap();
        seen.push(observed);
    }
    let mut writer = fixture.writer();
    observer.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_measure_oracle(&fixture, &observer, &seen);
    observer = serde_json::from_slice(&serde_json::to_vec(&observer).unwrap()).unwrap();
    let alias = frame(11, b"recorded alias", 8, 12, Some(-6));
    let mut held_alias = alias.clone();
    held_alias.hour = 12;
    for observed in [
        frame(9, b"opaque / tag", 7, 11, Some(0)),
        frame(10, b"opaque / tag", 8, 12, Some(-5)),
        alias,
        held_alias,
    ] {
        observer.observe(&observed).unwrap();
        seen.push(observed);
    }
    let mut writer = fixture.append_writer();
    observer.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_measure_oracle(&fixture, &observer, &seen);
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "measure-pp-1")
            .unwrap(),
        Some(3)
    );
    let mut reader =
        Reader::new(&fixture.store, &fixture.key, "measure-pp-1".into(), &flag).unwrap();
    let key = observer.countries["opaque / tag"].political.actor;
    assert!(reader.value_at(key, 8).unwrap().is_none());
}
