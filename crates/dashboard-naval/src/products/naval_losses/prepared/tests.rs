use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::{
    rows::CountryLinkRow, rows::SunkConvoyRow, rows::SunkShipEntry, rows::SunkShipsRow,
    rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::frame::TableView;

fn sample(reply: &NavalLossesReply, hour: i32) -> BTreeMap<String, (Option<i64>, Option<i64>)> {
    reply
        .series
        .iter()
        .filter_map(|series| {
            series
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && run.last_hour >= hour)
                .map(|run| (series.tag.clone(), (run.ships, run.convoys)))
        })
        .collect()
}
#[test]
fn prepared_naval_losses_match_oracle_after_two_commits_resume_append_and_bridge() {
    let flag = AtomicBool::new(false);
    let first = month_start(1940 * 12).unwrap();
    let mut inputs = Vec::new();
    for step in 0..5 {
        let hour = first + step;
        let mut frame = dashboard_base::test_fixtures::scale_frame(1, hour, 12);
        let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
        if step == 0 || step >= 3 {
            links.push(CountryLinkRow {
                id: 200000,
                canonical_slot: 0,
            });
        }
        if step == 4 {
            let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
            countries[0].id = 200123;
            countries.sort_by_key(|row| row.id);
            frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(countries)).into();
            links.push(CountryLinkRow {
                id: 200123,
                canonical_slot: 0,
            });
        }
        links.sort_by_key(|row| row.id);
        frame.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(links)).into();
        let mut entries: Vec<_> = (0..120)
            .map(|index| SunkShipEntry {
                ship_name: format!("synthetic mod ship {index}").into_bytes(),
                sink_hour: first,
                class_token: 77,
                victim_country: 100000 + index % 12,
                attacker_country: 100001,
                cause_record: u8::from(index % 5 == 0),
                assisted_kill: u8::from(index % 7 == 0),
                ..Default::default()
            })
            .collect();
        if step >= 1 {
            entries.push(SunkShipEntry {
                ship_name: b"late alias bridge sinking".to_vec(),
                sink_hour: first + 1,
                victim_country: 200000,
                attacker_country: 100001,
                class_token: 77,
                ..Default::default()
            });
        }
        if step == 4 {
            entries.clear();
        }
        frame.tables[TableId::SunkShips.index()] =
            TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }])).into();
        if step == 2 {
            frame.tables[TableId::SunkShips.index()] = TableView::Unavailable {
                reason: "synthetic retention read gap".into(),
            }
            .into();
        }
        frame.tables[TableId::SunkConvoys.index()] =
            TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
                month: 1940 * 12,
                owner_country: 200000,
                killer_country: 100001,
                convoys: [-3, 1, i32::MAX, i32::MIN, 0][step as usize],
                ordinal: 0,
            }]))
            .into();
        frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
        frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
        inputs.push(frame);
    }
    let mut fixture = Fixture::new();
    let mut history = sinkings::ShipHistory::new(first);
    let mut prepared = Prepared::default();
    for (step, frame) in inputs.iter().enumerate() {
        history.observe(replay_data::format::FormatVersion::default(), frame);
        history.finish(frame.hour);
        let retention = sinkings::Retention {
            ships: true,
            convoys: true,
            ship_loss: vec![],
            convoy_loss: vec![],
            warnings: vec![],
        };
        prepared.observe(&history.scan, &retention, &flag).unwrap();
        let mut writer = if step == 4 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared
            .flush(&mut writer, 1, &history.scan, &flag)
            .unwrap();
        history.compact();
        series::write_reply(&mut writer, "resume", 0, &(&history, &prepared)).unwrap();
        writer.commit().unwrap();
        (history, prepared) = series::read_reply(&fixture.store, &fixture.key, "resume", 0)
            .unwrap()
            .unwrap();
        history.resume(frame);
        if step >= 1 {
            let replay = replay::container::Replay::from_records(
                dashboard_base::test_fixtures::CAMPAIGN.into(),
                &[dashboard_base::test_fixtures::records(
                    1,
                    inputs[..=step].to_vec(),
                )],
            )
            .unwrap();
            let oracle = sinkings::scan(&replay, 1, &flag).unwrap();
            let (actors, metadata) =
                series::read_catalogue::<Metadata>(&fixture.store, &fixture.key, "naval-losses", 1)
                    .unwrap()
                    .unwrap();
            let root =
                std::str::from_utf8(&inputs[0].complete::<tables::Countries>().unwrap()[0].tag)
                    .unwrap()
                    .to_owned();
            for channel in 0..12 {
                for top in [None, Some(10)] {
                    let mut params = selection(1, channel);
                    params.top = top;
                    if top.is_none() {
                        params.tags = vec![root.clone()];
                    }
                    let expected = reply_with_histories(
                        &oracle,
                        &params,
                        &flag,
                        (true, true),
                        (&[], &[]),
                        vec![],
                        Some(&oracle.histories),
                    )
                    .unwrap();
                    let actual = window_reply(
                        &fixture.store,
                        &fixture.key,
                        &actors,
                        &metadata,
                        &oracle.histories,
                        &params,
                        Some((first, frame.hour)),
                        &flag,
                    )
                    .unwrap();
                    assert_eq!(
                        actual.warnings, expected.warnings,
                        "channel {channel}, step {step}"
                    );
                    assert_eq!(actual.first_read, expected.first_read);
                    assert_eq!(actual.available, expected.available);
                    assert_eq!(actual.ship_reads, expected.ship_reads);
                    assert_eq!(actual.convoy_reads, expected.convoy_reads);
                    for hour in first..=frame.hour {
                        assert_eq!(
                            sample(&actual, hour),
                            sample(&expected, hour),
                            "channel {channel}, step {step}, hour {hour}, top {top:?}"
                        );
                    }
                }
            }
            if step == 3 {
                let params = selection(1, 8);
                let exact = window_reply(
                    &fixture.store,
                    &fixture.key,
                    &actors,
                    &metadata,
                    &oracle.histories,
                    &NavalLossesParams {
                        tags: vec![root.clone()],
                        ..params.clone()
                    },
                    Some((first + 1, first + 1)),
                    &flag,
                )
                .unwrap();
                assert!(
                    sample(&exact, first + 1)
                        .values()
                        .all(|(ships, convoys)| ships.is_some() && convoys.is_some())
                );
                let summary = series_query(
                    "series-summary",
                    &json!({"from":first,"to":frame.hour,"width":1,"parts":true}),
                    &NavalLossesParams {
                        tags: vec![root.clone()],
                        ..params
                    },
                    &actors,
                    &metadata,
                    &oracle.histories,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap();
                assert!(summary["sampled"]["series"].as_array().is_some());
                let expected = reply_with_histories(
                    &oracle,
                    &NavalLossesParams {
                        tags: vec![root.clone()],
                        ..selection(1, 8)
                    },
                    &flag,
                    (true, true),
                    (&[], &[]),
                    vec![],
                    Some(&oracle.histories),
                )
                .unwrap();
                let actual: dashboard_base::prepared_chart::ChartSummary =
                    serde_json::from_value(summary["series"][0]["summary"].clone()).unwrap();
                let bucket = actual.buckets[0].unwrap();
                let known: Vec<_> = expected
                    .series
                    .iter()
                    .flat_map(|series| &series.runs)
                    .filter_map(|run| amount(run, NavalKind::Both).map(|value| (run, value)))
                    .collect();
                assert_eq!(
                    bucket.min_value,
                    known.iter().map(|(_, value)| *value).min().unwrap() as f64
                );
                assert_eq!(
                    bucket.max_value,
                    known.iter().map(|(_, value)| *value).max().unwrap() as f64
                );
                assert_eq!(
                    bucket.absolute_total,
                    known
                        .iter()
                        .map(|(run, value)| i128::from(*value).unsigned_abs()
                            * (i64::from(run.last_hour) - i64::from(run.first_hour) + 1) as u128)
                        .sum::<u128>()
                );
                let before_bridge = first + 1;
                let blocked = series_query(
                    "series-next-change",
                    &json!({"key":"Home","from":null,"firstHour":first,"lastHour":frame.hour,
                        "within":{(root.clone()):[]}}),
                    &NavalLossesParams {
                        tags: vec![root.clone()],
                        ..selection(1, 8)
                    },
                    &actors,
                    &metadata,
                    &oracle.histories,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap();
                assert!(blocked["hour"].is_null());
                let bound = series_query(
                    "series-next-change",
                    &json!({"key":"Home","from":null,"firstHour":first,"lastHour":frame.hour,
                        "within":{(root.clone()):[[root.clone(),before_bridge,before_bridge,true]]}}),
                    &NavalLossesParams { tags:vec![root.clone()], ..selection(1,8) },
                    &actors, &metadata, &oracle.histories, &fixture.store, &fixture.key, &flag,
                ).unwrap();
                assert_eq!(bound["hour"].as_f64(), Some(f64::from(before_bridge)));
                // A dense Top 1 draws each line only where the exact, corrected ranking
                // puts its country first, the late-resolved alias included.
                assert!(!metadata.corrected.is_empty(), "the fixture corrects a late alias");
                let top = NavalLossesParams {
                    top: Some(1),
                    ..selection(1, 8)
                };
                let dense = series_query(
                    "series-summary",
                    &json!({"from":first,"to":frame.hour,"width":1}),
                    &top,
                    &actors,
                    &metadata,
                    &oracle.histories,
                    &fixture.store,
                    &fixture.key,
                    &flag,
                )
                .unwrap();
                let ranked = reply_with_histories(
                    &oracle,
                    &top,
                    &flag,
                    (true, true),
                    (&[], &[]),
                    vec![],
                    Some(&oracle.histories),
                )
                .unwrap();
                for line in dense["series"].as_array().unwrap() {
                    let tag = line["tag"].as_str().unwrap();
                    let membership = ranked
                        .series
                        .iter()
                        .find(|series| series.tag == tag)
                        .and_then(|series| series.membership.clone())
                        .unwrap_or_default();
                    let summary: dashboard_base::prepared_chart::ChartSummary =
                        serde_json::from_value(line["summary"].clone()).unwrap();
                    for bucket in summary.buckets.iter().flatten() {
                        for hour in [bucket.first_hour, bucket.last_hour] {
                            assert!(
                                membership.iter().any(|span| {
                                    span.first_hour <= hour && hour <= span.last_hour
                                }),
                                "{tag} is drawn at hour {hour}, outside its Top 1 hours {membership:?}"
                            );
                        }
                    }
                }
            }
        }
    }
    assert!(history.scan.ships.is_empty());
    assert!(prepared.channels.iter().all(|channel| {
        channel
            .seed
            .values()
            .all(|events| events.ships.len() <= 1 && events.convoys.is_empty())
    }));
}
#[test]
fn shifted_signed_summaries_preserve_absolute_totals_and_refuse_unsafe_values() {
    let bucket = dashboard_base::prepared_chart::ChartBucket {
        absolute_total: 5,
        first_hour: 0,
        last_hour: 1,
        first_value: 2.0,
        last_value: 3.0,
        last_value_hour: 1,
        min_hour: 0,
        min_value: 2.0,
        max_hour: 1,
        max_value: 3.0,
        first_break: false,
        has_break: false,
    };
    let changed = shifted(bucket, -4).unwrap();
    assert_eq!(changed.absolute_total, 3);
    assert_eq!((changed.min_value, changed.max_value), (-2.0, -1.0));
    assert!(shifted(bucket, JS_MAX_SAFE_INTEGER).is_err());
}
