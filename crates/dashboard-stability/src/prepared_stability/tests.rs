use super::*;
use dashboard_base::{
    prepared::series::tests::Fixture,
    prepared_measure::query::query,
    test_fixtures::{CAMPAIGN, records},
};
use replay::container::Replay;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;
use serde_json::json;

mod experience {
    use dashboard_base::products::country_experience::{
        UNITS, scan,
        test_fixtures::{experience_frame, renamed_frame, slot_base},
    };
    use dashboard_base::test_fixtures::{CAMPAIGN, records};
    use replay::container::Replay;
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    use std::sync::atomic::AtomicBool;

    #[test]
    fn prepared_experience_matches_native_reads_alias_reuse_gaps_and_resumed_append() {
        use crate::prepared_stability::ExperienceAccumulator;
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
        let mut accumulator = ExperienceAccumulator::default();
        for frame in &frames {
            accumulator.observe(frame, replay.format).unwrap();
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
                            .complete::<replay_data::rows::tables::Countries>()
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

    // regression: 585b7093a3 perf(dashboard): prepare native experience reads and window charts
    #[test]
    fn prepared_experience_preserves_equal_read_boundaries_and_wide_gains() {
        use crate::prepared_stability::ExperienceAccumulator;
        use dashboard_base::{prepared::series::tests::Fixture, prepared_measure::query::query};
        use replay_data::{rows::CountryExperienceGainRow, rows::CountryExperienceRow};
        use serde_json::json;
        const COUNTRIES: u32 = 3;
        let flag = AtomicBool::new(false);
        let mut frame = experience_frame(0, false, 1);
        let countries: Vec<_> = (1..=COUNTRIES)
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
        let mut accumulator = ExperienceAccumulator::default();
        for hour in 0..100 {
            frame.hour = slot_base() + hour;
            *frame.tables[TableId::CountryExperienceGains.index()] =
                TableView::Complete(TableRows::CountryExperienceGains(
                    (1..=COUNTRIES)
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
                .observe(&frame, replay_data::format::FormatVersion::CURRENT)
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
        let key = serde_json::to_string(&(format!("country / {COUNTRIES}"), COUNTRIES, COUNTRIES))
            .unwrap();
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
        assert_eq!(catalogue.len(), 10 * COUNTRIES as usize);
        assert_eq!(runs, 44 * u64::from(COUNTRIES));
        assert_eq!(events, runs);
        // The catalogue is its base plus a short chain of patches.
        let catalogue = fixture
            .store
            .range_count(&fixture.key, "experience-1-catalogue")
            .unwrap()
            .unwrap();
        assert!((1..=4).contains(&catalogue));
        // Ranges the manifest lists, whichever packs hold them.
        let ranges = fixture.payloads().len() as u64;
        assert_eq!(ranges, 3 + catalogue);
        let summary = query("series-summary",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-banked"},"tags":[key],"from":slot_base(),"to":slot_base()+99,"width":100}),&fixture.store,&fixture.key, None,&flag).unwrap();
        assert_eq!(
            summary["series"][0]["summary"]["runs"]
                .as_array()
                .unwrap()
                .len(),
            5
        );
        let dense = query("series-summary",json!({"branchId":"1","family":{"kind":"country-experience","measure":"army-gain"},"tags":[key],"from":slot_base(),"to":slot_base()+99,"width":1}),&fixture.store,&fixture.key, None,&flag).unwrap();
        let dense = serde_json::from_value::<dashboard_base::prepared_chart::ChartSummary>(
            dense["series"][0]["summary"].clone(),
        )
        .unwrap();
        assert_eq!(
            dense.buckets[0].map(|bucket| bucket.min_value),
            Some((i128::from(i64::MAX) * 24) as f64)
        );
    }
}

/// Experience prepared on its own observes its banked reads and the hours its
/// tables change, and advances over the hours between: the same continuation
/// and published values as observing every hour, across quiet spans of most
/// of a day (through which the day of gains a read sums rolls), a gain change
/// between reads, and a country whose gain interval is never complete.
#[test]
fn experience_quiet_spans_advance_as_observing_each_hour_does() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::CountryExperienceGainRow, rows::CountryExperienceRow};
    let format = replay_data::format::FormatVersion::CURRENT;
    let replay_data::cadence::Cadence::Daily(slot) =
        replay_data::cadence::cadence(TableId::CountryExperience)
    else {
        panic!("CountryExperience is read in a daily slot");
    };
    let read = |day: i32| day * replay_data::cadence::STAGGER_PERIOD + slot as i32;
    let (first, last) = (read(1000) + 1, read(1002) + 5);
    let gains = |army_raw: i64| -> replay_frame::frame::SharedView {
        TableView::Complete(TableRows::CountryExperienceGains(vec![
            CountryExperienceGainRow {
                country: 11,
                instance: 1,
                complete: 1,
                army_raw,
                ..Default::default()
            },
            CountryExperienceGainRow {
                country: 21,
                instance: 2,
                ..Default::default()
            },
        ]))
        .into()
    };
    let banked = |army_raw: i64| -> replay_frame::frame::SharedView {
        TableView::Complete(TableRows::CountryExperience(vec![CountryExperienceRow {
            country: 11,
            army_raw,
            ..Default::default()
        }]))
        .into()
    };
    // Unchanged tables keep their allocation hour after hour, as decoded.
    let mut frame = dashboard_base::test_fixtures::source(1, first, Some(1));
    frame.tables[TableId::CountryExperience.index()] = banked(100);
    frame.tables[TableId::CountryExperienceGains.index()] = gains(3);
    let mut frames = Vec::new();
    for hour in first..=last {
        frame.hour = hour;
        if hour % replay_data::cadence::STAGGER_PERIOD == slot as i32 {
            frame.tables[TableId::CountryExperience.index()] = banked(100 + i64::from(hour));
        }
        if hour == read(1001) + 9 {
            frame.tables[TableId::CountryExperienceGains.index()] = gains(7);
        }
        frames.push(frame.clone());
    }
    let mut hourly = ExperienceAccumulator::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        let changes = tracker.observe(frame, format);
        hourly.observe_changes(frame, format, &changes).unwrap();
    }
    let watched = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryExperience,
        TableId::CountryExperienceGains,
    ];
    let mut quiet = ExperienceAccumulator::default();
    let mut tracker = FrameChangeTracker::new();
    let (mut span, mut longest, mut observed) = (None::<(i32, i32)>, 0, 0);
    for frame in &frames {
        let changes = tracker.observe(frame, format);
        let due = changes.slot_read[TableId::CountryExperience.index()]
            || watched
                .iter()
                .any(|table| changes.rows_changed[table.index()]);
        if !due {
            span = Some(span.map_or((frame.hour, frame.hour), |(from, _)| (from, frame.hour)));
            continue;
        }
        if let Some((from, to)) = span.take() {
            assert!(quiet.advance_quiet(from, to).unwrap(), "{from}..={to}");
            longest = longest.max(to - from + 1);
        }
        quiet.observe_changes(frame, format, &changes).unwrap();
        observed += 1;
    }
    if let Some((from, to)) = span {
        assert!(quiet.advance_quiet(from, to).unwrap(), "{from}..={to}");
        longest = longest.max(to - from + 1);
    }
    // Banked experience is read daily, so a quiet span is at most the rest of
    // a day: the day of gains a read sums rolls through it.
    assert!(
        longest >= replay_data::cadence::STAGGER_PERIOD - 2,
        "{longest}"
    );
    assert!(
        observed < frames.len(),
        "{observed} of {} hours",
        frames.len()
    );
    let flag = AtomicBool::new(false);
    let fixtures = [Fixture::new(), Fixture::new()];
    for (accumulator, fixture) in [&mut hourly, &mut quiet].into_iter().zip(&fixtures) {
        let mut writer = fixture.writer();
        accumulator.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
    }
    assert_eq!(
        serde_json::to_value(&quiet).unwrap(),
        serde_json::to_value(&hourly).unwrap(),
        "the continuation"
    );
    let tag = serde_json::to_string(&("GER", 7, 9001)).unwrap();
    for measure in ["army-banked", "army-gain", "army-change", "navy-banked"] {
        for hour in first..=last {
            let request = json!({
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": measure },
                "reads": [{ "tag": tag, "hour": hour }],
            });
            let [own, expected] = [&fixtures[1], &fixtures[0]].map(|fixture| {
                query(
                    "series-state",
                    request.clone(),
                    &fixture.store,
                    &fixture.key,
                    None,
                    &flag,
                )
                .unwrap()
            });
            assert_eq!(own, expected, "{measure} at {hour}");
        }
    }
}
/// Experience observed from each hour's recorded changes joins again only the
/// slots whose gain rows changed (a few countries an hour, one gaining
/// through two instances at times, one whose row comes and goes) and compares
/// no country key while only balances move. It publishes and saves exactly
/// what observing each hour's whole tables does, across two blocks.
#[test]
fn experience_observes_an_hours_changed_gains_as_its_whole_tables() {
    use dashboard_base::{prepared::frame_changes::FrameChangeTracker, test_fixtures};
    use replay_data::{rows::CountryExperienceGainRow, rows::CountryExperienceRow};
    use std::sync::Arc;
    const COUNT: u32 = 48;
    let format = replay_data::format::FormatVersion::CURRENT;
    let base = 1000 * 24;
    let frame = |hour: i32| {
        let mut frame = test_fixtures::scale_frame(1, hour, COUNT);
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            for row in rows {
                row.political_power_raw = Some(i64::from(hour));
            }
        }
        // Banked experience grows at each daily read.
        let day = i64::from((hour - 17).div_euclid(24));
        *frame.tables[TableId::CountryExperience.index()] =
            TableView::Complete(TableRows::CountryExperience(
                (0..COUNT)
                    .map(|index| CountryExperienceRow {
                        country: 100_000 + index,
                        army_raw: 1_000 * day + i64::from(index),
                        ..Default::default()
                    })
                    .collect(),
            ));
        let mut gains = Vec::new();
        for index in 0..COUNT {
            // The fifth country's gains are recorded half the time.
            if index == 5 && hour.rem_euclid(10) < 5 {
                continue;
            }
            let country = 100_000 + index;
            gains.push(CountryExperienceGainRow {
                country,
                instance: 1,
                complete: 1,
                army_raw: if (hour + index as i32).rem_euclid(12) == 0 {
                    2
                } else {
                    1
                },
                ..Default::default()
            });
            // The seventh gains through a second instance at times.
            if index == 7 && hour.rem_euclid(8) < 3 {
                gains.push(CountryExperienceGainRow {
                    country,
                    instance: 2,
                    complete: 1,
                    army_raw: 3,
                    ..Default::default()
                });
            }
        }
        *frame.tables[TableId::CountryExperienceGains.index()] =
            TableView::Complete(TableRows::CountryExperienceGains(gains));
        frame
    };
    let frames = |hours: std::ops::Range<i32>| -> Vec<replay_frame::frame::DecodedFrame> {
        hours.map(|offset| frame(base + offset)).collect()
    };
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0..36))]).unwrap();
    let replay = first
        .append(&[test_fixtures::continuation_records(
            &first,
            1,
            frames(36..72),
        )])
        .unwrap();
    let observe = |updates: bool, whole: bool| {
        let mut accumulator = ExperienceAccumulator {
            whole_experience: whole,
            ..Default::default()
        };
        for index in &replay.index {
            let mut tracker = FrameChangeTracker::new();
            replay
                .read_block_alone(
                    index,
                    &TableId::ALL,
                    |raw, frame| {
                        let changes = if updates {
                            tracker.observe_updates(frame, Arc::new(raw), format)
                        } else {
                            tracker.observe(frame, format)
                        };
                        accumulator.observe_changes(frame, format, &changes)
                    },
                    || Ok(()),
                )
                .unwrap();
        }
        let visits = accumulator.scan.work().row_visits;
        let fixture = Fixture::new();
        let mut writer = fixture.writer();
        accumulator
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        writer.commit().unwrap();
        let state = serde_json::to_value(&accumulator).unwrap();
        (fixture.payloads(), state, visits, accumulator.projections)
    };
    let (whole_pages, whole_state, whole_visits, whole_projections) = observe(false, true);
    let (pages, state, visits, projections) = observe(true, false);
    assert!(
        whole_pages.keys().any(|(name, _)| name == "experience-1"),
        "experience published"
    );
    assert!(
        projections < whole_projections,
        "held reads do not reproject ten channels"
    );
    assert_eq!(pages, whole_pages, "pages");
    assert_eq!(state, whole_state, "saved state");
    assert!(
        visits * 3 < whole_visits,
        "{visits} rows visited from the changes, {whole_visits} from the whole tables"
    );
}
