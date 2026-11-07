use super::*;
use crate::products::naval_losses::prepared::tests::{Harness, reloaded};
use replay_data::{
    rows::CountryLinkRow, rows::SunkConvoyRow, rows::SunkShipEntry, rows::SunkShipsRow,
    rows::TableId, rows::TableRows, rows::tables,
};
use replay_frame::{frame::DecodedFrame, frame::TableView};

fn samples(reply: &NavalLossesReply, hour: i32) -> BTreeMap<String, (Option<i64>, Option<i64>)> {
    reply
        .series
        .iter()
        .filter_map(|series| {
            series
                .runs
                .iter()
                .find(|run| run.first_hour <= hour && hour <= run.last_hour)
                .map(|run| (series.tag.clone(), (run.ships, run.convoys)))
        })
        .collect()
}
fn inputs(countries: usize, first_ship_step: usize) -> Vec<DecodedFrame> {
    let first = month_start(1940 * 12 + 1).unwrap() - 4;
    [0, 1, 2, 4, 5, 6, 7, 8]
        .into_iter()
        .enumerate()
        .map(|(step, offset)| {
            let mut frame =
                dashboard_base::test_fixtures::scale_frame(1, first + offset, countries as u32);
            let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
            if step == 0 || step >= 4 {
                links.push(CountryLinkRow {
                    id: 200000,
                    canonical_slot: 0,
                });
            }
            if step >= 6 {
                let mut rows = frame.complete::<tables::Countries>().unwrap().to_vec();
                rows[0].tag = b"synthetic successor".to_vec();
                frame.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(rows)).into();
            }
            links.sort_by_key(|row| row.id);
            frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(links)).into();
            let mut entries: Vec<_> = (0..countries)
                .map(|index| SunkShipEntry {
                    ship_name: format!("synthetic ship {index}").into_bytes(),
                    victim_country: 100000 + index as u32,
                    attacker_country: 100001,
                    class_token: 77,
                    sink_hour: first,
                    cause_record: u8::from(index % 5 == 0),
                    assisted_kill: u8::from(index % 7 == 0),
                    ..Default::default()
                })
                .collect();
            if step >= 1 {
                let duplicate = SunkShipEntry {
                    ship_name: b"bridge duplicates".to_vec(),
                    victim_country: 200000,
                    attacker_country: 100001,
                    sink_hour: first + 1,
                    class_token: 77,
                    ..Default::default()
                };
                entries.extend([duplicate.clone(), duplicate]);
            }
            if step >= 5 {
                entries.clear();
            }
            frame.tables[TableId::SunkShips.index()] = if step < first_ship_step || step == 3 {
                TableView::Unavailable {
                    reason: "synthetic ship read gap".into(),
                }
            } else {
                TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }]))
            }
            .into();
            frame.tables[TableId::SunkConvoys.index()] = if step == 3 {
                TableView::Unavailable {
                    reason: "synthetic convoy read gap".into(),
                }
            } else {
                TableView::Complete(TableRows::SunkConvoys(vec![SunkConvoyRow {
                    month: 1940 * 12,
                    owner_country: 200000,
                    killer_country: 100001,
                    convoys: [-3, 1, i32::MAX, i32::MAX, i32::MIN, 0, -2, 3][step],
                    ordinal: 0,
                }]))
            }
            .into();
            frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
            frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
            frame
        })
        .collect()
}
fn parity(countries: usize, baseline_step: usize) {
    let flag = AtomicBool::new(false);
    let frames = inputs(countries, baseline_step);
    let first = frames[0].hour;
    let mut harness = Harness::new(first);
    let mut history = sinkings::ShipHistory::new(first);
    let mut prepared = Prepared::default();
    let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
    let retention = sinkings::Retention {
        ships: true,
        convoys: true,
        ship_loss: vec![],
        convoy_loss: vec![],
        warnings: vec![],
    };
    for (step, frame) in frames.iter().enumerate() {
        let changes = history.prepare(
            replay_data::format::FormatVersion::default(),
            frame,
            &tracker.observe(frame, replay_data::format::FormatVersion::default()),
        );
        prepared
            .observe_changes(
                &history.scan,
                &changes,
                history.prepared_holders(),
                history.prepared_scope_changed(),
                &retention,
                &flag,
            )
            .unwrap();
        history.finish_prepared(frame.hour);
        harness.hour(&mut prepared, frame.hour);
        harness.publish(&mut prepared, &history.scan, frame.hour);
        (history, prepared) = reloaded(&(&history, &prepared));
        history.resume(frame);
        tracker.invalidate();
        if step == 0 {
            continue;
        }
        let replay = replay::campaign::Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(
                1,
                frames[..=step].to_vec(),
            )],
        )
        .unwrap();
        let oracle = sinkings::scan(&replay, 1, &flag).unwrap();
        let root = std::str::from_utf8(&frames[0].complete::<tables::Countries>().unwrap()[0].tag)
            .unwrap();
        for channel in 0..12 {
            for top in [None, Some(10)] {
                let mut params = selection(1, channel);
                params.top = top;
                if top.is_none() {
                    params.tags = vec![root.to_owned()];
                }
                let expected = reply_with_histories(
                    &oracle,
                    &params,
                    &flag,
                    (true, true),
                    (&[], &[]),
                    vec![],
                    Some(&oracle.histories),
                );
                let actual = harness.read(channel, |lines, actors, metadata| {
                    window_reply(
                        lines,
                        actors,
                        metadata,
                        &oracle.histories,
                        &params,
                        Some((first, frame.hour)),
                        &flag,
                    )
                });
                match (actual, expected) {
                    (Err(actual), Err(expected)) => {
                        assert_eq!(actual.to_string(), expected.to_string())
                    }
                    (Ok(actual), Ok(expected)) => {
                        dashboard_base::prepared::observer_tests::assert_reply_parity(
                            &actual, &expected,
                        )
                        .unwrap();
                        assert_eq!(
                            actual.warnings, expected.warnings,
                            "channel {channel}, step {step}"
                        );
                        assert_eq!(actual.first_read, expected.first_read);
                        assert_eq!(actual.ship_reads, expected.ship_reads);
                        assert_eq!(actual.convoy_reads, expected.convoy_reads);
                        assert_eq!(actual.available, expected.available);
                        for hour in first..=frame.hour {
                            assert_eq!(
                                samples(&actual, hour),
                                samples(&expected, hour),
                                "channel {channel}, step {step}, hour {hour}, top {top:?}, baseline {baseline_step}"
                            );
                        }
                    }
                    (actual, expected) => panic!(
                        "mismatched refusal channel {channel}, step {step}: {actual:?} / {expected:?}"
                    ),
                }
            }
        }
    }
    assert!(history.scan.ships.is_empty());
    assert!(
        prepared
            .channels
            .iter()
            .all(|channel| channel.incremental.pending.is_empty()),
        "resolved projections leave only cumulative counters"
    );
}
#[test]
fn changed_occurrences_match_oracle_after_two_commits_resume_append_alias_gaps_months_duplicates_and_wrap()
 {
    for baseline in [0, 1, 4] {
        parity(3, baseline);
    }
}
#[test]
fn changed_contribution_arithmetic_refuses_overflow() {
    let mut channel = Channel::default();
    let frame = inputs(3, 0).remove(0);
    let mut history = sinkings::ShipHistory::new(frame.hour);
    let changes = history.prepare(
        replay_data::format::FormatVersion::default(),
        &frame,
        &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(&frame, replay_data::format::FormatVersion::default()),
    );
    let params = selection(1, 0);
    let change = changes
        .iter()
        .find(|change| {
            change
                .ship
                .as_ref()
                .is_some_and(|ship| ship.entry.victim_country == 100001)
        })
        .unwrap();
    channel.totals.insert(
        "null".into(),
        BTreeMap::from([(
            actor((1, 100001)),
            Delta {
                ships: i64::MAX,
                ..Default::default()
            },
        )]),
    );
    let error = channel
        .apply(
            &history.scan,
            change,
            &params,
            contribution(&history.scan, change, &params, &AtomicBool::new(false)).unwrap(),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Naval losses count exceeds i64 arithmetic"
    );
    let mut delta = Delta {
        convoys: i64::MIN,
        ..Default::default()
    };
    assert!(
        delta
            .checked_add(
                Delta {
                    convoys: 1,
                    ..Default::default()
                },
                false
            )
            .is_err()
    );
}

#[test]
fn changed_contributions_preserve_missing_country_precedence_over_prefix_overflow() {
    let frame = inputs(3, 0).remove(0);
    let mut history = sinkings::ShipHistory::new(frame.hour);
    let mut changes = history.prepare(
        replay_data::format::FormatVersion::default(),
        &frame,
        &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
            .observe(&frame, replay_data::format::FormatVersion::default()),
    );
    changes.push(sinkings::Change {
        key: (false, 100),
        ship: Some(sinkings::ShipEvent {
            entry: SunkShipEntry {
                victim_country: 999999,
                sink_hour: frame.hour,
                ..Default::default()
            },
            read_hour: frame.hour,
            victim: None,
            attacker: None,
            victim_as_of: None,
            attacker_as_of: None,
        }),
        convoy: None,
        possible: Default::default(),
        pending: true,
    });
    let mut prepared = Prepared::default();
    prepared.channels[0].incremental.totals.insert(
        "null".into(),
        BTreeMap::from([(
            actor((1, 100001)),
            Delta {
                ships: i64::MAX,
                ..Default::default()
            },
        )]),
    );
    let error = prepared
        .observe_changes(
            &history.scan,
            &changes,
            history.prepared_holders(),
            history.prepared_scope_changed(),
            &sinkings::Retention {
                ships: true,
                convoys: true,
                ship_loss: vec![],
                convoy_loss: vec![],
                warnings: vec![],
            },
            &AtomicBool::new(false),
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Naval losses cannot identify possible holders of ship country reference 999999"
    );
}

#[test]
fn restored_losses_close_a_holder_that_leaves_at_the_first_hour_after_a_commit() {
    // Country 2 loses a ship to country 1 at the first hour. Its country and
    // link are gone from the fifth hour, the first hour after the commit.
    const VICTIM: Identity = (2, 100_002);
    type Pair = (sinkings::ShipHistory, Prepared);
    let flag = AtomicBool::new(false);
    let format = replay_data::format::FormatVersion::default();
    let first = month_start(1940 * 12 + 1).unwrap() + 100;
    let frames: Vec<DecodedFrame> = (0..8)
        .map(|offset| {
            let mut frame = dashboard_base::test_fixtures::scale_frame(1, first + offset, 3);
            if offset >= 4 {
                let mut countries = frame.complete::<tables::Countries>().unwrap().to_vec();
                countries.truncate(2);
                frame.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(countries)).into();
                let mut links = frame.complete::<tables::CountryLinks>().unwrap().to_vec();
                links.truncate(2);
                frame.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(links)).into();
            }
            frame.tables[TableId::SunkShips.index()] =
                TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow {
                    entries: vec![SunkShipEntry {
                        ship_name: b"synthetic lost ship".to_vec(),
                        victim_country: 100_002,
                        attacker_country: 100_001,
                        class_token: 77,
                        sink_hour: first,
                        ..Default::default()
                    }],
                }]))
                .into();
            frame.tables[TableId::SunkConvoys.index()] =
                TableView::Complete(TableRows::SunkConvoys(Vec::new())).into();
            frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
            frame.metrics.table_ns[TableId::SunkConvoys.index()] = 1;
            frame
        })
        .collect();
    let blocks = [&frames[..4], &frames[4..]];
    let retention = sinkings::Retention {
        ships: true,
        convoys: true,
        ship_loss: vec![],
        convoy_loss: vec![],
        warnings: vec![],
    };
    dashboard_base::prepared::observer_tests::assert_restore_continues(
        format,
        &blocks,
        || -> Pair { (sinkings::ShipHistory::new(first), Prepared::default()) },
        |pair: &mut Pair, frame, changes, _| {
            let (history, prepared) = pair;
            let changes = history.prepare(format, frame, changes);
            if frame.hour == first + 4 {
                assert!(history.prepared_scope_changed());
                assert!(
                    history
                        .prepared_holders()
                        .iter()
                        .all(|(_, identity)| *identity != VICTIM),
                    "the victim no longer holds a tag"
                );
            }
            prepared
                .observe_changes(
                    &history.scan,
                    &changes,
                    history.prepared_holders(),
                    history.prepared_scope_changed(),
                    &retention,
                    &flag,
                )
                .unwrap();
        },
        |pair: &mut Pair, writer, ordinal| {
            let (history, prepared) = pair;
            history.finish_prepared(blocks[ordinal].last().unwrap().hour);
            if ordinal == 0 {
                // Ships lost, running: the victim's loss is open at the commit.
                let key = field_key(prepared.actors[&actor(VICTIM)], 0);
                let lost = &prepared.channels[0].values;
                assert!(lost.active().any(|open| open == key));
                assert_eq!(lost.value(key), Some(1));
            }
            // What its series is handed: every write since the last commit, by channel.
            let written: Vec<_> = prepared
                .channels
                .iter_mut()
                .map(|channel| std::mem::take(&mut channel.values.written))
                .collect();
            dashboard_base::prepared::series::write_reply(
                writer,
                "naval-losses-written",
                ordinal as u64,
                &written,
            )
            .unwrap();
            dashboard_base::prepared::series::write_reply(
                writer,
                "naval-losses-continuation",
                0,
                &(&*history, &*prepared),
            )
            .unwrap();
        },
        |fixture| {
            let (mut history, prepared): Pair = dashboard_base::prepared::series::read_reply(
                &fixture.store,
                &fixture.key,
                "naval-losses-continuation",
                0,
            )
            .unwrap()
            .unwrap();
            // As preparation does after every restore: the last frame observed.
            let last = frames
                .iter()
                .find(|frame| frame.hour == history.scan.last_hour)
                .unwrap();
            history.resume(last);
            (history, prepared)
        },
    );
}

/// One batch over the inputs' gap publishes what a cut after every hour does: nothing is held
/// across the gap, and only the hours handed are known.
#[test]
fn a_batch_over_a_gap_answers_as_cuts_at_every_hour_do() {
    let flag = AtomicBool::new(false);
    let frames = inputs(3, 0);
    let first = frames[0].hour;
    assert!(
        frames
            .windows(2)
            .any(|pair| pair[0].hour + 1 != pair[1].hour),
        "the inputs skip an hour"
    );
    let retention = sinkings::Retention {
        ships: true,
        convoys: true,
        ship_loss: vec![],
        convoy_loss: vec![],
        warnings: vec![],
    };
    let mut harnesses = [Harness::new(first), Harness::new(first)];
    let mut histories = [
        sinkings::ShipHistory::new(first),
        sinkings::ShipHistory::new(first),
    ];
    let mut prepared = [Prepared::default(), Prepared::default()];
    let last = frames.last().unwrap().hour;
    for (at, (harness, (history, prepared))) in harnesses
        .iter_mut()
        .zip(histories.iter_mut().zip(prepared.iter_mut()))
        .enumerate()
    {
        let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
        for frame in &frames {
            let changes = history.prepare(
                replay_data::format::FormatVersion::default(),
                frame,
                &tracker.observe(frame, replay_data::format::FormatVersion::default()),
            );
            prepared
                .observe_changes(
                    &history.scan,
                    &changes,
                    history.prepared_holders(),
                    history.prepared_scope_changed(),
                    &retention,
                    &flag,
                )
                .unwrap();
            history.finish_prepared(frame.hour);
            harness.hour(prepared, frame.hour);
            if at == 0 || frame.hour == last {
                harness.publish(prepared, &history.scan, frame.hour);
            }
        }
    }
    let replay = replay::campaign::Replay::from_records(
        dashboard_base::test_fixtures::CAMPAIGN.into(),
        &[dashboard_base::test_fixtures::records(1, frames.clone())],
    )
    .unwrap();
    let oracle = sinkings::scan(&replay, 1, &flag).unwrap();
    for channel in 0..12 {
        let mut params = selection(1, channel);
        params.top = Some(10);
        let [cut, batched] = [&harnesses[0], &harnesses[1]].map(|harness| {
            harness.read(channel, |lines, actors, metadata| {
                let reply = window_reply(
                    lines,
                    actors,
                    metadata,
                    &oracle.histories,
                    &params,
                    Some((first, last)),
                    &flag,
                )
                .map(|reply| serde_json::to_value(reply).unwrap())
                .map_err(|error| error.to_string());
                let summary = series_query(
                    "series-summary",
                    &json!({"from": first, "to": last, "width": 4}),
                    &params,
                    actors,
                    metadata,
                    &oracle.histories,
                    lines,
                    &flag,
                )
                .map_err(|error| error.to_string());
                (reply, summary)
            })
        });
        assert_eq!(batched, cut, "channel {channel}");
    }
}

#[test]
fn a_value_after_none_starts_a_segment_and_a_gap_ends_every_value() {
    let mut values = Values::default();
    values.observe(1, 10, Some(4), false);
    values.observe(1, 11, Some(4), false);
    values.observe(1, 12, None, false);
    values.observe(1, 13, Some(4), false);
    values.observe(2, 13, Some(0), false);
    values.gap(13);
    values.observe(2, 16, Some(0), false);
    assert_eq!(
        values.written,
        vec![
            (1, 10, Some(4), true),
            (1, 12, None, false),
            (1, 13, Some(4), true),
            (2, 13, Some(0), true),
            (1, 14, None, false),
            (2, 14, None, false),
            (2, 16, Some(0), true),
        ]
    );
    assert!(values.held(1) && values.value(1).is_none());
}
