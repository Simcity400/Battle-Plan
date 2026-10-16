use super::*;
use dashboard_base::{
    prepared::series::tests::Fixture,
    prepared_measure::query::query,
    test_fixtures::{CAMPAIGN, records, stability_frame, stability_inputs},
};
use replay::{
    container::Replay,
    frame::TableView,
    rows::{TableId, TableRows},
};
use serde_json::{Value, json};

fn publish(accumulator: &mut StabilityAccumulator, fixture: &Fixture) {
    let mut writer = fixture.writer();
    accumulator
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
}
fn read(fixture: &Fixture, family: Value, hour: i32) -> Value {
    query(
        "series-state",
        json!({"branchId":"1", "family":family, "reads":[{"tag":"GER", "hour":hour}]}),
        &fixture.store,
        &fixture.key,
        None,
        &AtomicBool::new(false),
    )
    .unwrap()
}

#[cfg(feature = "slow-tests")]
#[test]
fn experience_native_work_is_flat_and_hourly_gain_queues_remain_separate() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    use replay::rows::{CountryExperienceGainRow, CountryExperienceRow};
    let format = replay::format::FormatVersion::CURRENT;
    for complete in [0, 1] {
        let work = observer_tests::scaled_work(2, |history| {
            let spans = observer_tests::scaled_work(10, |hours| {
                let mut frame = dashboard_base::test_fixtures::scale_frame(1, 100, 5_000);
                *frame.tables[TableId::CountryExperience.index()] =
                    TableView::Complete(TableRows::CountryExperience(
                        (0..5_000)
                            .map(|id| CountryExperienceRow {
                                country: 100_000 + id,
                                army_raw: 100,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                *frame.tables[TableId::CountryExperienceGains.index()] =
                    TableView::Complete(TableRows::CountryExperienceGains(
                        (0..5_000)
                            .map(|id| CountryExperienceGainRow {
                                country: 100_000 + id,
                                instance: 1,
                                complete,
                                army_raw: 1,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                let mut tracker = FrameChangeTracker::new();
                let mut data = StabilityAccumulator::default();
                for _ in 0..=history {
                    frame.metrics.table_ns[TableId::CountryExperience.index()] = 1;
                    let changes = tracker.observe(&frame, format);
                    data.observe_experience_changes(&frame, format, &changes)?;
                    frame.hour += 1;
                }
                frame.metrics.table_ns[TableId::CountryExperience.index()] = 0;
                let changes = tracker.observe(&frame, format);
                data.observe_experience_changes(&frame, format, &changes)?;
                let native_before = data.experience.work();
                let before = Work {
                    emitted: 0,
                    ..native_before
                };
                let ticks = data.experience.clock_ticks();
                let work = observer_tests::unchanged_frames(
                    before,
                    &frame,
                    hours as i32,
                    format,
                    |frame, changes| {
                        data.observe_experience_changes(frame, format, changes)?;
                        Ok(Work {
                            emitted: 0,
                            ..data.experience.work()
                        })
                    },
                )?;
                assert_eq!(work, Work::default());
                let supplying_reads = (1..=hours as i32)
                    .filter(|offset| {
                        dashboard_text::metadata::slot_read(
                            format,
                            &frame.metrics,
                            TableId::CountryExperience,
                            frame.hour + offset,
                        )
                    })
                    .count();
                assert_eq!(
                    data.experience.work().emitted - native_before.emitted,
                    5_000 * supplying_reads,
                    "native daily supplying reads retain their provenance even when rows are equal"
                );
                assert_eq!(
                    data.experience.clock_ticks() - ticks,
                    if complete == 1 { 5_000 * hours } else { 0 }
                );
                frame.hour += hours as i32 + 1;
                let mut tracker = FrameChangeTracker::new();
                let mut previous = frame.clone();
                previous.hour -= 1;
                tracker.observe(&previous, format);
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
                {
                    rows[0].command_power_raw += 1;
                }
                let before = data.experience.work();
                let changes = tracker.observe(&frame, format);
                data.observe_experience_changes(&frame, format, &changes)?;
                let after = data.experience.work();
                assert_eq!(
                    after.row_visits - before.row_visits,
                    5_000,
                    "balance changes read only country operands"
                );
                assert_eq!(after.emitted, before.emitted);
                let fixture = Fixture::new();
                publish(&mut data, &fixture);
                let resumed: StabilityAccumulator =
                    serde_json::from_slice(&serde_json::to_vec(&data)?).unwrap();
                assert_eq!(
                    serde_json::to_value(&resumed)?,
                    serde_json::to_value(&data)?
                );
                Ok(work)
            })?;
            observer_tests::assert_flat(spans);
            Ok(spans[1])
        })
        .unwrap();
        observer_tests::assert_flat(work);
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn stability_native_work_is_flat_over_hours_and_history_and_one_changed_row() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{Work, scaled_work, unchanged_frames},
    };
    let measured = scaled_work(2, |history| {
        let hours = scaled_work(10, |hours| {
            let mut frame = dashboard_base::test_fixtures::scale_frame(1, 100, 5_000);
            *frame.tables[TableId::CountryStabilityInputs.index()] =
                TableView::Complete(TableRows::CountryStabilityInputs(
                    (0..5_000)
                        .map(|index| {
                            let mut row = stability_inputs();
                            row.country = 100_000 + index;
                            row
                        })
                        .collect(),
                ));
            *frame.tables[TableId::SliceReads.index()] =
                TableView::Complete(TableRows::SliceReads(
                    (0..5_000)
                        .map(|index| replay::rows::SliceReadRow {
                            owner: 100_000 + index,
                            table: TableId::CountryStabilityInputs as u32,
                            sample_hour: 100,
                        })
                        .collect(),
                ));
            let format = replay::format::FormatVersion::CURRENT;
            let mut tracker = FrameChangeTracker::new();
            let mut data = StabilityAccumulator::default();
            for _ in 0..=history {
                if let TableView::Complete(TableRows::CountryStabilityInputs(rows)) =
                    &mut *frame.tables[TableId::CountryStabilityInputs.index()]
                {
                    rows[0].stability_factor = rows[0].stability_factor.map(|raw| raw + 100);
                }
                let changes = tracker.observe(&frame, format);
                data.observe_timed(&frame, &changes, &mut None)?;
                frame.hour += 1;
            }
            frame.hour -= 1;
            let before = data.stability.work();
            let work = unchanged_frames(before, &frame, hours as i32, format, |frame, changes| {
                data.observe_timed(frame, changes, &mut None)?;
                Ok(data.stability.work())
            })?;
            assert_eq!(
                work,
                Work::default(),
                "no numeric join, formula or emission on carried hours"
            );
            frame.hour += hours as i32;
            let mut tracker = FrameChangeTracker::new();
            tracker.observe(&frame, format);
            frame.hour += 1;
            if let TableView::Complete(TableRows::CountryStabilityInputs(rows)) =
                &mut *frame.tables[TableId::CountryStabilityInputs.index()]
            {
                rows[0].stability_factor = rows[0].stability_factor.map(|raw| raw + 100);
            }
            let before = data.stability.work();
            let changes = tracker.observe(&frame, format);
            data.observe_timed(&frame, &changes, &mut None)?;
            let after = data.stability.work();
            let delta = Work {
                row_visits: after.row_visits - before.row_visits,
                emitted: after.emitted - before.emitted,
                ..Default::default()
            };
            assert_eq!(
                delta.row_visits, 5_000,
                "only the changed input table is visited"
            );
            assert_eq!(delta.emitted, 1, "only one native holder changed");
            let fixture = Fixture::new();
            publish(&mut data, &fixture);
            let flag = AtomicBool::new(false);
            let (mut reader, actor) = (
                dashboard_base::prepared::series::Reader::new(
                    &fixture.store,
                    &fixture.key,
                    "stability-1".into(),
                    &flag,
                )?,
                &data.actors[&series::field_key("000", "stability")],
            );
            let value = reader.value_at(actor.key, frame.hour)?.unwrap();
            assert_eq!(value.last_hour, frame.hour);
            assert_eq!(
                value.value,
                i128::from(
                    country_stability::formula::stability(
                        &frame
                            .complete::<replay::rows::tables::CountryStabilityInputs>()
                            .unwrap()[0]
                    )
                    .value
                    .unwrap()
                )
            );
            Ok(delta)
        })?;
        assert_eq!(hours[0], hours[1]);
        Ok(hours[1])
    })
    .unwrap();
    assert_eq!(measured[0], measured[1]);
}

#[test]
fn production_scan_and_prepared_values_reads_gaps_and_resume_agree() {
    let inputs = stability_inputs();
    let mut unknown = inputs.clone();
    unknown.stability_factor = None;
    unknown.keys[0] = u32::MAX;
    let mut dropped = stability_frame(3, inputs.clone(), 2, vec![]);
    *dropped.tables[TableId::CountryStabilityInputs.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut unlined = stability_frame(4, inputs.clone(), 4, vec![]);
    *unlined.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let mut frames = vec![
        stability_frame(0, inputs.clone(), 0, vec![]),
        stability_frame(1, inputs.clone(), 0, vec![]),
        stability_frame(2, unknown, 2, vec![]),
        dropped,
        unlined,
        stability_frame(5, inputs.clone(), 4, vec![]),
        stability_frame(7, inputs.clone(), 7, vec![]),
    ];
    let mut replacement = stability_frame(9, inputs.clone(), 9, vec![]);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replacement.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.tag == b"GER").unwrap().id = 9100;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *replacement.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut().find(|row| row.id == 9001).unwrap().id = 9100;
    }
    if let TableView::Complete(TableRows::CountryStabilityInputs(rows)) =
        &mut *replacement.tables[TableId::CountryStabilityInputs.index()]
    {
        rows[0].country = 9100;
    }
    if let TableView::Complete(TableRows::SliceReads(rows)) =
        &mut *replacement.tables[TableId::SliceReads.index()]
    {
        rows.iter_mut().for_each(|row| row.owner = 9100);
    }
    let mut renamed = replacement.clone();
    renamed.hour = 10;
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *renamed.tables[TableId::Countries.index()]
    {
        rows.iter_mut().find(|row| row.tag == b"GER").unwrap().tag = b"renamed".to_vec();
    }
    frames.extend([replacement, renamed]);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let (oracle, _) =
        country_stability::scan(&replay, 1, &["GER"], None, &AtomicBool::new(false)).unwrap();
    let fixture = Fixture::new();
    let mut accumulator = StabilityAccumulator::default();
    for (index, frame) in frames.iter().enumerate() {
        accumulator.observe(frame).unwrap();
        if index == 1 || index == 4 || index == frames.len() - 1 {
            publish(&mut accumulator, &fixture);
            accumulator =
                serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
        }
    }
    for (measure, expected) in [
        ("stability", &oracle["GER"].stability),
        ("warSupport", &oracle["GER"].war_support),
        ("maxCommandPower", &oracle["GER"].max_command_power),
    ] {
        for hour in 0..=10 {
            let reply = read(
                &fixture,
                json!({"kind":"country-stability", "measure":measure}),
                hour,
            );
            let actual = &reply["states"][0]["run"];
            let run = expected
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour);
            assert_eq!(
                actual["valueRaw"].as_str(),
                run.map(|run| run.value_raw.as_str()),
                "{measure} {hour}"
            );
            if let Some(run) = run {
                assert_eq!(actual["firstHour"], run.first_hour);
                assert_eq!(actual["lastHour"], run.last_hour);
                assert_eq!(actual["startsSegment"], run.starts_segment);
                let read = oracle["GER"]
                    .carried
                    .iter()
                    .find(|read| read.first_hour <= hour && hour <= read.last_hour)
                    .map_or(hour, |read| read.read_hour);
                assert_eq!(actual["readHour"], read);
                assert_eq!(reply["states"][0]["readHour"], read);
            }
        }
    }
    let masked = query("series-next-change", json!({"branchId":"1","family":{"kind":"country-stability","measure":"stability"},
        "tags":["GER"],"within":{"GER":[{"firstHour":5,"lastHour":5}]},"key":"Home","firstHour":0,"lastHour":10}),
        &fixture.store, &fixture.key, None, &AtomicBool::new(false)).unwrap();
    assert_eq!(masked["hour"].as_f64(), Some(5.0));
    let summary = query("series-summary", json!({"branchId":"1","family":{"kind":"country-stability","measure":"stability"},"tags":["GER"],
        "from":0,"to":10,"width":100,"within":{"GER":[{"firstHour":5,"lastHour":5}]}}),&fixture.store,&fixture.key, None,&AtomicBool::new(false)).unwrap();
    let runs = summary["series"][0]["summary"]["runs"].as_array().unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0]["firstHour"], 5);
    assert_eq!(runs[0]["lastHour"], 5);
}

#[test]
fn navigation_unions_requested_measures_with_each_tags_membership_after_resume() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut accumulator = StabilityAccumulator::default();
    for (first, last) in [(0, 4), (5, 10)] {
        for hour in first..=last {
            for (tag, measure, value) in [
                ("alpha", "stability", i128::from(hour >= 7)),
                ("alpha", "warSupport", i128::from(hour >= 5)),
                ("beta", "stability", i128::from(hour >= 6)),
            ] {
                record(
                    &mut accumulator.actors,
                    &mut accumulator.batch,
                    tag,
                    measure,
                    hour,
                    Some(value),
                    hour != 0,
                    (1, 1, 0),
                    false,
                    None,
                )
                .unwrap();
            }
        }
        let mut writer = fixture.writer();
        flush(
            &accumulator.actors,
            &mut accumulator.batch,
            &mut writer,
            ("stability", 1, last, 5),
            &flag,
        )
        .unwrap();
        writer.commit().unwrap();
        accumulator = serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
    }
    let family = json!({"kind":"country-stability", "measure":"stability"});
    for (key, from, expected) in [
        ("Home", None, 5.0),
        ("End", None, 7.0),
        ("ArrowRight", Some(5.0), 7.0),
        ("ArrowLeft", Some(7.0), 5.0),
    ] {
        let reply = query(
            "series-next-change",
            json!({
                "branchId":"1", "family":family, "tags":["alpha"],
                "measures":["stability", "warSupport"],
                "within":{"alpha":[{"firstHour":5,"lastHour":8}]},
                "key":key, "from":from, "firstHour":0,"lastHour":10,
            }),
            &fixture.store,
            &fixture.key,
            None,
            &flag,
        )
        .unwrap();
        assert_eq!(reply["hour"].as_f64(), Some(expected), "{key} {from:?}");
    }
    for (measures, within, expected) in [
        (json!([]), json!({}), 7.0),
        (json!(["warSupport"]), json!({}), 5.0),
        (json!(["stability", "warSupport"]), json!({"alpha":[]}), 0.0),
    ] {
        let reply = query(
            "series-next-change",
            json!({
                "branchId":"1", "family":family, "tags":["alpha"], "measures":measures,
                "within":within, "key":"ArrowRight", "from":0, "firstHour":0,"lastHour":10,
            }),
            &fixture.store,
            &fixture.key,
            None,
            &flag,
        )
        .unwrap();
        assert_eq!(reply["hour"].as_f64(), Some(expected));
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn five_thousand_countries_keep_runs_and_files_proportional_to_changes_and_batches() {
    let mut fixture = Fixture::new();
    let mut accumulator = StabilityAccumulator::default();
    let mut frame = stability_frame(0, stability_inputs(), 0, vec![]);
    let countries: Vec<_> = (1..=5000)
        .map(|id| replay::rows::CountryRow {
            id,
            canonical_slot: id,
            tag: format!("country / {id}").into_bytes(),
            ..Default::default()
        })
        .collect();
    let links = countries
        .iter()
        .map(|row| replay::rows::CountryLinkRow {
            id: row.id,
            canonical_slot: row.canonical_slot,
        })
        .collect();
    let inputs = countries
        .iter()
        .map(|row| {
            let mut input = stability_inputs();
            input.country = row.id;
            input
        })
        .collect();
    let reads = countries
        .iter()
        .map(|row| replay::rows::SliceReadRow {
            owner: row.id,
            table: TableId::CountryStabilityInputs as u32,
            sample_hour: 0,
        })
        .collect();
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(links));
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(inputs));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(reads));
    for hour in 0..100 {
        frame.hour = hour;
        accumulator.observe(&frame).unwrap();
        if hour == 32 || hour == 65 || hour == 99 {
            if hour == 99 {
                let mut writer = fixture.append_writer();
                accumulator
                    .flush(&mut writer, 1, &AtomicBool::new(false))
                    .unwrap();
                writer.commit().unwrap();
            } else {
                publish(&mut accumulator, &fixture);
            }
            accumulator =
                serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
        }
    }
    let runs: u64 = accumulator
        .actors
        .values()
        .map(|actor| {
            accumulator
                .batch
                .state(actor.key)
                .map_or(0, |state| state.runs)
        })
        .sum();
    let events: u64 = accumulator
        .actors
        .values()
        .map(|actor| {
            accumulator
                .batch
                .state(actor.key)
                .map_or(0, |state| state.events)
        })
        .sum();
    assert!(runs <= accumulator.actors.len() as u64);
    assert_eq!(events, runs);
    let flag = AtomicBool::new(false);
    let mut reader = dashboard_base::prepared::series::Reader::new(
        &fixture.store,
        &fixture.key,
        "stability-1".into(),
        &flag,
    )
    .unwrap();
    let actor =
        &accumulator.actors[&serde_json::to_string(&("country / 5000", "stability")).unwrap()];
    assert_eq!(
        reader.value_at(actor.key, 99).unwrap().unwrap().last_hour,
        99
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "stability-1")
            .unwrap(),
        Some(3)
    );
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "stability-1-catalogue")
            .unwrap(),
        Some(1)
    );
    let files = std::fs::read_dir(&fixture.directory)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "bin")
        })
        .count();
    assert_eq!(accumulator.parts.calculated_reads(), 5000);
    assert_eq!(
        fixture.store.range_count(&fixture.key, "parts-1").unwrap(),
        Some(3)
    );
    let (_, catalogue) = dashboard_base::prepared::series::read_catalogue::<Vec<MeasureActor>>(
        &fixture.store,
        &fixture.key,
        "parts",
        1,
    )
    .unwrap()
    .unwrap();
    let part_runs: u64 = catalogue
        .iter()
        .map(|actor| actor.run_count.parse::<u64>().unwrap())
        .sum();
    let part_events: u64 = catalogue
        .iter()
        .map(|actor| actor.drop_count.parse::<u64>().unwrap())
        .sum();
    assert_eq!(part_runs, catalogue.len() as u64);
    assert_eq!(part_events, part_runs);
    println!(
        "Parts: 500000 country-hours / 5000 computed reads, {part_runs} canonical runs, {part_events} provenance events; {files} combined physical files / 9 active (4 stability, 5 parts)"
    );
    assert_eq!(files, 12);
    println!(
        "5000 countries x 100 hours: stability {runs} canonical runs, {events} provenance events"
    );
}

// Experience as the prepared stability accumulator observes it, against the native scan.
mod experience {
    use dashboard_base::products::country_experience::{
        UNITS, scan,
        test_fixtures::{experience_frame, renamed_frame, slot_base},
    };
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    use replay::{
        container::Replay,
        frame::TableView,
        rows::{CountryLinkRow, CountryRow, TableId, TableRows},
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_experience_matches_native_reads_alias_reuse_gaps_and_resumed_append() {
        use crate::prepared_stability::StabilityAccumulator;
        use dashboard_base::{prepared::series::tests::Fixture, prepared_measure::query::query};
        use serde_json::json;
        let frames: Vec<_> = (0..=96)
            .filter(|hour| ![16, 54].contains(hour))
            .map(renamed_frame)
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
        let flag = AtomicBool::new(false);
        let oracle = scan(&replay, 1, &["opaque!tag", "renamed", "third"], &flag).unwrap();
        let mut fixture = Fixture::new();
        let mut accumulator = StabilityAccumulator::default();
        for frame in &frames {
            accumulator
                .observe_experience(frame, replay.format)
                .unwrap();
            if [24, 48, 96].contains(&(frame.hour - slot_base())) {
                let mut writer = if frame.hour == slot_base() + 48 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                accumulator.flush(&mut writer, 1, &flag).unwrap();
                writer.commit().unwrap();
                accumulator =
                    serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
            }
        }
        for tag in ["opaque!tag", "renamed", "third"] {
            for offset in 0..=96 {
                let hour = slot_base() + offset;
                let expected = oracle[tag]
                    .reads
                    .iter()
                    .find(|read| read.first_hour <= hour && hour <= read.last_hour);
                let country = frames
                    .iter()
                    .find(|frame| frame.hour == hour)
                    .and_then(|frame| {
                        frame
                            .complete::<replay::rows::tables::Countries>()
                            .unwrap()
                            .iter()
                            .find(|country| country.tag == tag.as_bytes())
                    });
                let key = serde_json::to_string(&(
                    tag,
                    country.map_or(0, |row| row.canonical_slot),
                    country.map_or(0, |row| row.id),
                ))
                .unwrap();
                for (measure, values) in [
                    (
                        "banked",
                        expected.map(|read| {
                            [
                                read.army_raw.as_str(),
                                read.navy_raw.as_str(),
                                read.air_raw.as_str(),
                            ]
                        }),
                    ),
                    (
                        "gain",
                        expected.and_then(|read| read.gain.as_ref()).map(|gain| {
                            [
                                gain.army_raw.as_str(),
                                gain.navy_raw.as_str(),
                                gain.air_raw.as_str(),
                            ]
                        }),
                    ),
                    (
                        "change",
                        expected
                            .and_then(|read| read.change.as_ref())
                            .map(|change| {
                                [
                                    change.army_raw.as_str(),
                                    change.navy_raw.as_str(),
                                    change.air_raw.as_str(),
                                ]
                            }),
                    ),
                ] {
                    for (index, branch) in ["army", "navy", "air"].iter().enumerate() {
                        let reply = query("series-state",json!({"branchId":"1","family":{"kind":"country-experience","measure":format!("{branch}-{measure}")},"reads":[{"tag":key,"hour":hour}]}),&fixture.store,&fixture.key, None,&flag).unwrap();
                        assert_eq!(reply["scale"], UNITS);
                        assert_eq!(reply["dayHours"], 24);
                        assert_eq!(
                            reply["states"][0]["run"]["valueRaw"].as_str(),
                            values.map(|values| values[index]),
                            "{tag} {hour} {branch}-{measure}"
                        );
                        assert_eq!(
                            reply["states"][0]["experience"],
                            serde_json::to_value(expected).unwrap(),
                            "{tag} {hour}"
                        );
                    }
                }
            }
        }
        let key = serde_json::to_string(&("third", 7, 9001)).unwrap();
        let reply = query("events-page",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-banked"},"tags":[key],"from":slot_base(),"to":slot_base()+96,"pageSize":100}),&fixture.store,&fixture.key, None,&flag).unwrap();
        assert!(
            reply["rows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row["row"]["experience"].is_object())
        );
        let summary = query("series-summary",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-banked"},"tags":[key],"from":slot_base(),"to":slot_base()+96,"width":100}),&fixture.store,&fixture.key, None,&flag).unwrap();
        assert!(summary["series"][0]["summary"]["runs"].is_array());
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "experience-1")
                .unwrap(),
            Some(3)
        );
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn prepared_experience_five_thousand_countries_preserves_equal_read_boundaries_and_wide_gains()
    {
        use crate::prepared_stability::StabilityAccumulator;
        use dashboard_base::{prepared::series::tests::Fixture, prepared_measure::query::query};
        use replay::rows::{CountryExperienceGainRow, CountryExperienceRow};
        use serde_json::json;
        let flag = AtomicBool::new(false);
        let mut frame = experience_frame(0, false, 1);
        let countries: Vec<_> = (1..=5000)
            .map(|id| CountryRow {
                id,
                canonical_slot: id,
                tag: format!("country / {id}").into_bytes(),
                ..Default::default()
            })
            .collect();
        *frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(
                countries
                    .iter()
                    .map(|row| CountryLinkRow {
                        id: row.id,
                        canonical_slot: row.canonical_slot,
                    })
                    .collect(),
            ));
        *frame.tables[TableId::CountryExperience.index()] =
            TableView::Complete(TableRows::CountryExperience(
                countries
                    .iter()
                    .map(|row| CountryExperienceRow {
                        country: row.id,
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::Countries.index()] =
            TableView::Complete(TableRows::Countries(countries));
        let mut fixture = Fixture::new();
        let mut accumulator = StabilityAccumulator::default();
        for hour in 0..100 {
            frame.hour = slot_base() + hour;
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(
                    (1..=5000)
                        .map(|id| CountryExperienceGainRow {
                            country: id,
                            instance: 77,
                            complete: u8::from(hour > 0),
                            army_raw: i64::MAX,
                            navy_raw: 0,
                            air_raw: 0,
                        })
                        .collect(),
                ));
            accumulator
                .observe_experience(&frame, replay::format::FormatVersion::CURRENT)
                .unwrap();
            if [32, 65, 99].contains(&hour) {
                let mut writer = if hour == 99 {
                    fixture.append_writer()
                } else {
                    fixture.writer()
                };
                accumulator.flush(&mut writer, 1, &flag).unwrap();
                writer.commit().unwrap();
                accumulator =
                    serde_json::from_slice(&serde_json::to_vec(&accumulator).unwrap()).unwrap();
            }
        }
        let key = serde_json::to_string(&("country / 5000", 5000, 5000)).unwrap();
        let reply = query("series-state",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-gain"},"reads":[{"tag":key,"hour":slot_base()+99}]}),&fixture.store,&fixture.key, None,&flag).unwrap();
        let gained = (i128::from(i64::MAX) * 24).to_string();
        assert_eq!(reply["states"][0]["run"]["valueRaw"], gained);
        assert_eq!(reply["states"][0]["experience"]["gain"]["armyRaw"], gained);
        let (_, catalogue) = dashboard_base::prepared::series::read_catalogue::<
            Vec<dashboard_base::prepared_measure::MeasureActor>,
        >(&fixture.store, &fixture.key, "experience", 1)
        .unwrap()
        .unwrap();
        let runs: u64 = catalogue
            .iter()
            .map(|actor| actor.run_count.parse::<u64>().unwrap())
            .sum();
        let events: u64 = catalogue
            .iter()
            .map(|actor| actor.drop_count.parse::<u64>().unwrap())
            .sum();
        assert_eq!(catalogue.len(), 50000);
        assert_eq!(runs, 220000);
        assert_eq!(events, runs);
        let files = std::fs::read_dir(&fixture.directory)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "bin")
            })
            .count();
        assert_eq!(files, 5);
        let summary = query("series-summary",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-banked"},"tags":[key],"from":slot_base(),"to":slot_base()+99,"width":100}),&fixture.store,&fixture.key, None,&flag).unwrap();
        assert_eq!(
            summary["series"][0]["summary"]["runs"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let dense = query("series-summary",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-gain"},"tags":[key],"from":slot_base(),"to":slot_base()+99,"width":1}),&fixture.store,&fixture.key, None,&flag).unwrap();
        assert_eq!(
            dense["series"][0]["summary"]["buckets"][0]["minValue"].as_f64(),
            Some((i128::from(i64::MAX) * 24) as f64)
        );
        println!(
            "5000 countries x 100 hours: {runs} supplying runs, {events} provenance events, {files} physical files / 4 active (3 batches + catalogue); zero interpreter calls"
        );
    }
}
