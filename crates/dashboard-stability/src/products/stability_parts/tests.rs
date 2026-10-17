use dashboard_base::test_fixtures::{CAMPAIGN, records, stability_frame, stability_inputs};
use replay::{
    container::Replay,
    frame::{DecodedFrame, TableView},
    rows::{TableId, TableRows},
};

#[test]
fn appearance_projection_visits_only_changed_naming_rows() {
    use super::*;
    use dashboard_base::prepared::frame_changes::field_mask;
    use replay::rows::{CountryAppearanceRow, DeltaRow};
    let mut scan = Scan {
        appearances_read: true,
        appearances_complete: true,
        appearances: (0..200)
            .map(|country| {
                (
                    country,
                    CountryAppearanceRow {
                        country,
                        cosmetic_tag: b"original".to_vec(),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        ..Default::default()
    };
    let mut frame = appearance_frame(100, true, None);
    let mut rows: Vec<_> = scan.appearances.values().cloned().collect();
    rows[7].cosmetic_tag = b"changed".to_vec();
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(rows));
    let row = DeltaRow {
        key: 7,
        mask: field_mask(TableId::CountryAppearance, &["cosmetic_tag"]),
        format: replay::format::FormatVersion::CURRENT.version(),
        recorded_fields: 0,
        values: Vec::new(),
    };
    let mut dirty = BTreeSet::new();
    assert!(scan.apply_appearances(&frame, Some((&[row], &[])), &mut dirty));
    assert_eq!(dirty, BTreeSet::from([7]));
    assert_eq!(scan.native.work.row_visits, 1);
    assert_eq!(scan.appearances[&7].cosmetic_tag, b"changed");
    dirty.clear();
    assert!(scan.apply_appearances(&frame, Some((&[], &[8])), &mut dirty));
    assert_eq!(dirty, BTreeSet::from([8]));
    assert!(!scan.appearances.contains_key(&8));
    scan.appearances_complete = false;
    assert!(!scan.apply_appearances(&frame, Some((&[], &[])), &mut dirty));
}

#[test]
fn stability_parts_add_up_to_the_value_and_are_known_only_where_recorded() {
    use crate::products::stability_parts::scan_parts;
    use dashboard_base::products::country_groups::HourSpan;
    use replay::rows::CountryStabilityLineRow as Line;
    let line = |line, kind, identity: &[u8], value, text: &[u8]| Line {
        country: 9001,
        line,
        key: 0x8d,
        kind,
        identity: identity.to_vec(),
        value,
        text: text.to_vec(),
        ..Default::default()
    };
    // stability_factor is 30 000: the two lines explain 25 000, the rest is a remainder.
    let lines = vec![
        line(0, Line::IDEA, b"idea_a", 10_000, b""),
        line(2, Line::UNPAIRED, b"", 15_000, b"Decision: X"),
    ];
    // A read whose lines table was not recorded: its parts are not known.
    let mut unlined = stability_frame(103, stability_inputs(), 103, vec![]);
    *unlined.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(100, stability_inputs(), 100, lines.clone()),
                // Carried from the hour-100 read.
                stability_frame(101, stability_inputs(), 100, lines),
                // A read with no lines: the whole modifier term is unexplained.
                stability_frame(102, stability_inputs(), 102, vec![]),
                unlined,
            ],
        )],
    )
    .unwrap();
    let (stability, war_support, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    let observed = [HourSpan {
        first_hour: 100,
        last_hour: 102,
    }];
    assert_eq!(stability.observed, observed);
    assert_eq!(war_support.observed, observed);
    let at = |parts: &crate::products::stability_parts::MeasureParts, hour: i32| -> i64 {
        parts
            .parts
            .iter()
            .flat_map(|part| &part.runs)
            .filter(|run| run.first_hour <= hour && hour <= run.last_hour)
            .map(|run| run.value_raw.parse::<i64>().unwrap())
            .sum()
    };
    // The parts add up to the displayed values (79.249% and the war support total).
    assert_eq!(at(&stability, 100), 79_249);
    assert_eq!(at(&stability, 101), 79_249);
    assert_eq!(at(&stability, 102), 79_249);
    let remainder = stability
        .parts
        .iter()
        .find(|part| part.id == "remainder")
        .unwrap();
    assert_eq!(
        remainder
            .runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "5000"), (102, 102, "30000")]
    );
    let displayed = crate::products::country_stability::formula::war_support(&stability_inputs());
    assert_eq!(at(&war_support, 100), displayed.value.unwrap());
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    for id in [
        "term:base",
        "line:idea:idea_a:0:0",
        "unpaired:Decision: X",
        "remainder",
    ] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let idea = stability
        .parts
        .iter()
        .find(|part| part.id == "line:idea:idea_a:0:0")
        .unwrap();
    assert_eq!(
        idea.runs
            .iter()
            .map(|run| (run.first_hour, run.last_hour, run.value_raw.as_str()))
            .collect::<Vec<_>>(),
        [(100, 101, "10000")]
    );
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("1 reads recorded no modifier lines"))
    );
    // Lines missing at one hour and recorded again at the next hour of the same
    // read: the parts are known again there, not lost until the next read.
    let mut missing = stability_frame(200, stability_inputs(), 200, vec![]);
    *missing.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                missing,
                stability_frame(201, stability_inputs(), 200, vec![]),
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 201,
            last_hour: 201
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
    // Parts carried from a read are not reused for an hour whose lines table is missing.
    let mut dropped = stability_frame(301, stability_inputs(), 300, vec![]);
    *dropped.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![
                stability_frame(300, stability_inputs(), 300, vec![]),
                dropped,
            ],
        )],
    )
    .unwrap();
    let (stability, _, warnings) = scan_parts(&replay, 1, "GER").unwrap();
    assert_eq!(
        stability.observed,
        [HourSpan {
            first_hour: 300,
            last_hour: 300
        }]
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn stability_parts_name_a_read_s_characters_as_recorded_at_that_read() {
    use crate::products::stability_parts::scan_parts;
    use replay::rows::CountryStabilityLineRow as Line;
    let leader = vec![Line {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: Line::COUNTRY_LEADER,
        identity_id: 7,
        value: 30_000,
        ..Default::default()
    }];
    let named = |mut frame: DecodedFrame, saved: &str| {
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
                id: 7,
                token: saved.as_bytes().to_vec(),
                name_token_id: -1,
                ..Default::default()
            }]));
        frame
    };
    // Read 100's lines are missing at hour 100 and arrive at 101, after a rename.
    let mut first = named(
        stability_frame(100, stability_inputs(), 100, leader.clone()),
        "Old Name",
    );
    *first.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "fixture".into(),
    };
    let second = named(
        stability_frame(101, stability_inputs(), 100, leader),
        "New Name",
    );
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, second])]).unwrap();
    let (stability, _, _) = scan_parts(&replay, 1, "GER").unwrap();
    let ids: Vec<_> = stability
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    assert!(ids.iter().any(|id| id.ends_with(":Old Name")), "{ids:?}");
    assert!(!ids.iter().any(|id| id.ends_with(":New Name")), "{ids:?}");
}

pub(super) fn prepared_source(frames: Vec<DecodedFrame>) -> dashboard_base::source::Source {
    use std::sync::{Arc, atomic::AtomicBool};
    dashboard_base::source::Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "parts fixture".into(),
            revision: "1".into(),
        },
        std::env::temp_dir().join("parts-fixture.battleplan"),
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}
#[test]
fn prepared_parts_match_the_product_with_missing_lines_character_reads_and_resume() {
    use super::*;
    use dashboard_base::prepared::series::tests::Fixture;
    use serde_json::json;
    let leader = replay::rows::CountryStabilityLineRow {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: replay::rows::CountryStabilityLineRow::COUNTRY_LEADER,
        identity_id: 7,
        value: 30_000,
        ..Default::default()
    };
    let make = |hour, read, lines: Vec<_>, name: &str| {
        let mut frame = stability_frame(hour, stability_inputs(), read, lines);
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![replay::rows::CharacterRow {
                id: 7,
                token: name.as_bytes().to_vec(),
                name_token_id: -1,
                ..Default::default()
            }]));
        frame
    };
    let mut first = make(0, 0, vec![leader.clone()], "Old name");
    *first.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "delayed lines".into(),
    };
    let mut absent = make(3, 2, vec![], "New name");
    *absent.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
        reason: "missing lines".into(),
    };
    let mut replaced = make(7, 7, vec![], "Replacement");
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *replaced.tables[TableId::Countries.index()]
    {
        rows.iter_mut()
            .find(|row| row.tag == b"GER")
            .unwrap()
            .canonical_slot = 8;
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *replaced.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 9001)
            .unwrap()
            .canonical_slot = 8;
    }
    let frames = vec![
        first,
        make(1, 0, vec![leader], "New name"),
        make(2, 2, vec![], "New name"),
        absent,
        make(4, 2, vec![], "New name"),
        make(6, 6, vec![], "New name"),
        replaced,
    ];
    let source = prepared_source(frames.clone());
    let (stability, war, warnings) = scan_parts(&source.replay(), 1, "GER").unwrap();
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut sampled = Prepared {
        sampled_projection: true,
        ..Default::default()
    };
    let mut sampled_fixture = Fixture::new();
    for (index, frame) in frames.iter().enumerate() {
        let calls = progressive::SAMPLE_CALLS.with(std::cell::Cell::get);
        prepared.observe(frame).unwrap();
        assert_eq!(
            progressive::SAMPLE_CALLS.with(std::cell::Cell::get),
            calls,
            "preparation consumes parts directly"
        );
        sampled.observe(frame).unwrap();
        if [0, 3, 6].contains(&index) {
            let mut writer = if index == 6 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            let mut sampled_writer = if index == 6 {
                sampled_fixture.append_writer()
            } else {
                sampled_fixture.writer()
            };
            sampled.flush(&mut sampled_writer, 1, &flag).unwrap();
            sampled_writer.commit().unwrap();
            dashboard_base::prepared::series::tests::assert_payloads_equal(
                &fixture.payloads(),
                &sampled_fixture.payloads(),
                "direct parts projection",
            );
            let saved = serde_json::to_vec(&prepared).unwrap();
            assert_eq!(saved, serde_json::to_vec(&sampled).unwrap());
            prepared = serde_json::from_slice(&saved).unwrap();
            sampled = serde_json::from_slice(&saved).unwrap();
            sampled.sampled_projection = true;
        }
    }
    assert_eq!(prepared.calculated_reads(), 4);
    let accepted = dashboard_base::prepared::coverage::Coverage {
        schema: 0,
        first_hour: 0,
        last_hour: 7,
        complete: true,
    };
    for (measure, expected) in [("stability", stability), ("warSupport", war)] {
        let family = json!({"kind":"stability-parts","measure":measure});
        let summary = prepared_query(
            &source,
            None,
            "series-summary",
            json!({"branchId":"1","family":family,"tags":["GER"],"from":0,"to":7,"width":100}),
            &fixture.store,
            &fixture.key,
            Some(&accepted),
            &flag,
        )
        .unwrap();
        assert_eq!(summary["warnings"], json!(warnings));
        assert_eq!(
            summary["parts"]["GER"]["parts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|part| part["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected
                .parts
                .iter()
                .map(|part| part.id.as_str())
                .collect::<Vec<_>>()
        );
        for hour in 0..=7 {
            let state = prepared_query(
                &source,
                None,
                "series-state",
                json!({"branchId":"1","family":family,"reads":[{"tag":"GER","hour":hour}]}),
                &fixture.store,
                &fixture.key,
                Some(&accepted),
                &flag,
            )
            .unwrap();
            let known = expected
                .observed
                .iter()
                .any(|span| span.first_hour <= hour && hour <= span.last_hour);
            let coverage = state["states"]
                .as_array()
                .unwrap()
                .iter()
                .find(|state| state["tag"] == serde_json::to_string(&("GER", "")).unwrap())
                .unwrap();
            assert_eq!(!coverage["run"].is_null(), known, "{measure} {hour}");
            for part in &expected.parts {
                let actual = state["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|state| {
                        state["tag"] == serde_json::to_string(&("GER", &part.id)).unwrap()
                    })
                    .unwrap();
                let run = part
                    .runs
                    .iter()
                    .find(|run| run.first_hour <= hour && hour <= run.last_hour);
                assert_eq!(
                    actual["run"]["valueRaw"].as_str(),
                    run.map(|run| run.value_raw.as_str()),
                    "{} {hour}",
                    part.id
                );
                if let Some(run) = run {
                    assert_eq!(actual["run"]["startsSegment"], run.starts_segment);
                }
            }
        }
        let dense = prepared_query(
            &source,
            None,
            "series-summary",
            json!({"branchId":"1","family":family,"tags":["GER"],"from":0,"to":7,"width":1}),
            &fixture.store,
            &fixture.key,
            Some(&accepted),
            &flag,
        )
        .unwrap();
        for part in &expected.parts {
            let line = dense["series"]
                .as_array()
                .unwrap()
                .iter()
                .find(|line| line["tag"] == serde_json::to_string(&("GER", &part.id)).unwrap())
                .unwrap();
            let total: u128 = line["summary"]["buckets"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|bucket| !bucket.is_null())
                .map(|bucket| {
                    bucket["absoluteTotal"]
                        .as_str()
                        .unwrap()
                        .parse::<u128>()
                        .unwrap()
                })
                .sum();
            let expected: u128 = part
                .runs
                .iter()
                .map(|run| {
                    run.value_raw.parse::<i64>().unwrap().unsigned_abs() as u128
                        * (run.last_hour - run.first_hour + 1) as u128
                })
                .sum();
            assert_eq!(total, expected);
        }
        let nav=prepared_query(&source,None,"series-next-change",json!({"branchId":"1","family":family,"tags":["GER"],"within":{"GER":[{"firstHour":6,"lastHour":7}]},"key":"Home","firstHour":0,"lastHour":7}),&fixture.store,&fixture.key,Some(&accepted),&flag).unwrap();
        assert_eq!(nav["hour"].as_f64(), Some(6.0));
        let events = prepared_query(
            &source,
            None,
            "events-page",
            json!({"branchId":"1","family":family,"tags":["GER"],"pageSize":1}),
            &fixture.store,
            &fixture.key,
            Some(&accepted),
            &flag,
        )
        .unwrap();
        assert_eq!(events["rows"].as_array().unwrap().len(), 1);
        assert!(events["cursor"].is_string());
    }
}

#[test]
fn recent_naming_views_are_saved_once_and_stay_shared_after_resume() {
    use super::*;
    let frame = stability_frame(0, stability_inputs(), 0, vec![]);
    let mut scan = Scan::default();
    for hour in 0..20 {
        let mut frame = frame.clone();
        frame.hour = hour;
        scan.step(&frame, None, None, |_, _, _, _, _, _| Ok(()))
            .unwrap();
    }
    let json = serde_json::to_value(&scan).unwrap();
    assert_eq!(json["recent_names"][0].as_array().unwrap().len(), 20);
    assert_eq!(json["recent_names"][1].as_array().unwrap().len(), 2);
    let resumed: Scan = serde_json::from_value(json).unwrap();
    assert_eq!(resumed.recent_names, scan.recent_names);
    assert!(SharedView::ptr_eq(
        &resumed.recent_names[0].1,
        &resumed.recent_names[19].1
    ));
    assert!(SharedView::ptr_eq(
        &resumed.recent_names[0].2,
        &resumed.recent_names[19].2
    ));
}

#[test]
fn dynamic_part_naming_keeps_hour_dependent_native_refusals_on_held_tables() {
    use super::*;
    let mut frame = stability_frame(
        100,
        stability_inputs(),
        100,
        vec![replay::rows::CountryStabilityLineRow {
            country: 9001,
            line: 0,
            key: 0x8d,
            kind: replay::rows::CountryStabilityLineRow::IDEA,
            identity: b"mod_a".to_vec(),
            value: 10_000,
            ..Default::default()
        }],
    );
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            replay::rows::CountryAppearanceRow {
                country: 9001,
                name_tag: b"DYN".to_vec(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCivilWars.index()] =
        TableView::Complete(TableRows::CountryCivilWars(vec![
            replay::rows::CountryCivilWarRow {
                country: 9001,
                target: 44,
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![]));
    let mut prepared = Prepared::default();
    for hour in 100..=120 {
        frame.hour = hour;
        prepared.observe(&frame).unwrap();
        let country = &frame.complete::<tables::Countries>().unwrap()[0];
        let expected = NamingCountry::read(&frame, country);
        let parts: Vec<_> = prepared.countries["GER"].measures[0]
            .parts
            .values()
            .filter(|part| part.line.is_some())
            .collect();
        assert!(!parts.is_empty(), "exercise an actual recorded idea line");
        for part in parts {
            assert_eq!(part.country, expected);
            assert!(
                part.country
                    .look
                    .as_ref()
                    .unwrap()
                    .civil_war
                    .as_ref()
                    .unwrap_err()
                    .contains(&format!("hour {hour}"))
            );
        }
    }
    assert_eq!(
        prepared.scan.naming_ticks, 20,
        "only reached dynamic naming retains clock work"
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_parts_visit_no_unchanged_native_rows_or_history_at_scale() {
    use super::*;
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{self, Work},
    };
    let work = observer_tests::scaled_work(2, |history| {
        let spans = observer_tests::scaled_work(10, |hours| {
            let mut frame = dashboard_base::test_fixtures::scale_frame(1, 100, 5_000);
            *frame.tables[TableId::CountryStabilityInputs.index()] =
                TableView::Complete(TableRows::CountryStabilityInputs(
                    (0..5_000)
                        .map(|id| {
                            let mut row = stability_inputs();
                            row.country = 100_000 + id;
                            row
                        })
                        .collect(),
                ));
            *frame.tables[TableId::CountryStabilityLines.index()] =
                TableView::Complete(TableRows::CountryStabilityLines(vec![]));
            let format = replay::format::FormatVersion::CURRENT;
            let mut tracker = FrameChangeTracker::new();
            let mut prepared = Prepared::default();
            for _ in 0..=history {
                *frame.tables[TableId::SliceReads.index()] =
                    TableView::Complete(TableRows::SliceReads(
                        (0..5_000)
                            .map(|id| replay::rows::SliceReadRow {
                                owner: 100_000 + id,
                                table: TableId::CountryStabilityInputs as u32,
                                sample_hour: frame.hour,
                            })
                            .collect(),
                    ));
                let changes = tracker.observe(&frame, format);
                prepared.observe_changes(&frame, &changes)?;
                frame.hour += 1;
            }
            frame.hour -= 1;
            let before = prepared.scan.native.work;
            let work = observer_tests::unchanged_frames(
                before,
                &frame,
                hours as i32,
                format,
                |frame, changes| {
                    prepared.observe_changes(frame, changes)?;
                    Ok(prepared.scan.native.work)
                },
            )?;
            assert_eq!(work, Work::default());
            let fixture = dashboard_base::prepared::series::tests::Fixture::new();
            let mut writer = fixture.writer();
            prepared.flush(&mut writer, 1, &AtomicBool::new(false))?;
            writer.commit()?;
            let restored: Prepared = serde_json::from_slice(&serde_json::to_vec(&prepared)?)?;
            assert_eq!(
                serde_json::to_value(&restored)?,
                serde_json::to_value(&prepared)?
            );
            Ok(work)
        })?;
        observer_tests::assert_flat(spans);
        Ok(spans[1])
    })
    .unwrap();
    observer_tests::assert_flat(work);
}

#[cfg(feature = "slow-tests")]
#[test]
fn five_thousand_country_continuation_saves_equal_fresh_naming_reads_once() {
    use super::*;
    use replay::frame::{FrameEncoder, READ_TIME_NOT_RECORDED};
    let mut frame = dashboard_base::test_fixtures::scale_frame(1, 0, 5_000);
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(
            (0..5_000)
                .map(|id| {
                    let mut row = stability_inputs();
                    row.country = 100_000 + id;
                    row
                })
                .collect(),
        ));
    *frame.tables[TableId::CountryStabilityLines.index()] =
        TableView::Complete(TableRows::CountryStabilityLines(vec![]));
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(
        (0..5_000)
            .map(|id| replay::rows::SliceReadRow {
                owner: 100_000 + id,
                table: TableId::CountryStabilityInputs as u32,
                sample_hour: 0,
            })
            .collect(),
    ));
    *frame.tables[TableId::Characters.index()] = TableView::Complete(TableRows::Characters(
        (0..5_000)
            .map(|id| replay::rows::CharacterRow {
                id: 100_000 + id,
                token: format!("Saved character / {id}").into_bytes(),
                name_token_id: -1,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        (0..5_000)
            .map(|id| replay::rows::LexicalTokenRow {
                token: id,
                literal: format!("Recorded character script / {id}").into_bytes(),
            })
            .collect(),
    ));
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    let mut accumulator = crate::prepared_stability::StabilityAccumulator::default();
    let mut retained = VecDeque::new();
    for hour in 0..NAMED_HOURS as i32 {
        frame.hour = hour;
        for table in [TableId::Characters, TableId::LexicalTokens] {
            frame.metrics.table_ns[table.index()] = READ_TIME_NOT_RECORDED;
        }
        let encoded = encoder.encode(frame.clone(), hour == 0).unwrap();
        let decoded = decoder.advance(&encoded).unwrap();
        accumulator.observe(decoded).unwrap();
        retained.push_back((
            hour,
            decoded.tables[TableId::Characters.index()].clone(),
            decoded.tables[TableId::LexicalTokens.index()].clone(),
        ));
    }
    assert!(SharedView::ptr_eq(
        &retained[0].1,
        &retained[NAMED_HOURS - 1].1
    ));
    assert_ne!(
        retained[0].1.sampled_at(),
        retained[NAMED_HOURS - 1].1.sampled_at()
    );
    let after = serde_json::to_vec(&accumulator).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&after).unwrap();
    let archive = &json["parts"]["scan"]["recent_names"];
    assert_eq!(archive[1].as_array().unwrap().len(), 2);

    // Count the previous serializer's actual JSON bytes without retaining its
    // many repeated tables. Every supplying date formerly saved a checkpoint.
    let mut hours = Vec::new();
    let mut old_rows_bytes = 0;
    let mut count = 0;
    for (hour, characters, lexical) in &retained {
        let first = count;
        for (table, view) in [
            (TableId::Characters, characters),
            (TableId::LexicalTokens, lexical),
        ] {
            let mut packet = frame.clone();
            packet.hour = *hour;
            packet.tables = std::array::from_fn(|_| {
                TableView::Unavailable {
                    reason: "Unused naming input".into(),
                }
                .into()
            });
            packet.tables[table.index()] = view.clone();
            let mut bytes = Vec::new();
            packet.branch = 0;
            packet.metrics = Default::default();
            packet.encode_checkpoint(&mut bytes).unwrap();
            old_rows_bytes += serde_json::to_vec(&(table.index(), bytes)).unwrap().len();
            count += 1;
        }
        hours.push((*hour, first, first + 1));
    }
    let old_archive_bytes = serde_json::to_vec(&hours).unwrap().len() + old_rows_bytes + count + 4;
    let new_archive_bytes = serde_json::to_vec(archive).unwrap().len();
    let before_bytes = after.len() - new_archive_bytes + old_archive_bytes;
    eprintln!(
        "5000-country StabilityAccumulator JSON: before={before_bytes} after={}",
        after.len()
    );
    assert!(new_archive_bytes * 10 < old_archive_bytes);
    assert!(after.len() * 2 < before_bytes);
    let resumed: crate::prepared_stability::StabilityAccumulator =
        serde_json::from_slice(&after).unwrap();
    assert_eq!(serde_json::to_vec(&resumed).unwrap(), after);
}

/// GER's one idea line, carried from the hour-100 read, at `hour`: its lines
/// recorded or not, and its CountryAppearance row with this cosmetic tag (None:
/// no row).
fn appearance_frame(hour: i32, lined: bool, cosmetic: Option<&str>) -> DecodedFrame {
    let idea = replay::rows::CountryStabilityLineRow {
        country: 9001,
        line: 0,
        key: 0x8d,
        kind: replay::rows::CountryStabilityLineRow::IDEA,
        identity: b"idea_a".to_vec(),
        value: 10_000,
        ..Default::default()
    };
    let mut frame = stability_frame(hour, stability_inputs(), 100, vec![idea]);
    if !lined {
        *frame.tables[TableId::CountryStabilityLines.index()] = TableView::Unavailable {
            reason: "fixture".into(),
        };
    }
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(
            cosmetic
                .into_iter()
                .map(|cosmetic| replay::rows::CountryAppearanceRow {
                    country: 9001,
                    cosmetic_tag: cosmetic.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

/// Prepares `frames` in blocks `..4` and `4..`, one copy continued in memory
/// and one restored from its saved continuation, as preparation does.
fn assert_parts_restore_continues(frames: &[DecodedFrame]) {
    use super::*;
    let flag = AtomicBool::new(false);
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        replay::format::FormatVersion::CURRENT,
        &[&frames[..4], &frames[4..]],
        Prepared::default,
        |prepared, frame, changes, _| prepared.observe_changes(frame, changes).unwrap(),
        |prepared, writer, _| {
            prepared.flush(writer, 1, &flag).unwrap();
            writer
                .write_at(
                    "parts-continuation-1",
                    0,
                    serde_json::to_vec(&*prepared).unwrap().as_slice(),
                )
                .unwrap();
        },
        |fixture| {
            serde_json::from_reader::<_, Prepared>(
                fixture
                    .store
                    .read(&fixture.key, "parts-continuation-1", 0)
                    .unwrap()
                    .unwrap(),
            )
            .unwrap()
        },
    );
}

#[test]
fn restored_parts_rename_a_country_whose_appearance_row_is_gone_at_the_first_hour_after_a_commit() {
    // GER's CountryAppearance row is gone from hour 104, the first hour after a commit.
    let frames: Vec<_> = (100..108)
        .map(|hour| appearance_frame(hour, true, (hour < 104).then_some("GER_X")))
        .collect();
    assert_parts_restore_continues(&frames);
}

#[test]
fn parts_rename_a_country_whose_appearance_changed_at_an_hour_without_lines() {
    use super::*;
    // GER's appearance changes at hour 103, whose lines were not recorded; the
    // commit falls after it and hour 104 records the lines again.
    let frames: Vec<_> = (100..108)
        .map(|hour| {
            let cosmetic = if hour < 103 { "GER_X" } else { "GER_Y" };
            appearance_frame(hour, hour != 103, Some(cosmetic))
        })
        .collect();
    assert_parts_restore_continues(&frames);
    // Continued alone, hour 104's part is named with hour 104's appearance.
    let mut prepared = Prepared::default();
    for frame in &frames[..5] {
        prepared.observe(frame).unwrap();
    }
    let country = &frames[4].complete::<tables::Countries>().unwrap()[0];
    assert_eq!(
        prepared.countries["GER"].measures[0].parts["line:idea:idea_a:0:0"].country,
        NamingCountry::read(&frames[4], country)
    );
}
