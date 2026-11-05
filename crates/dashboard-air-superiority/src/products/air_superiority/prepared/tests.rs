use super::super::tests::{air, groups, params, recording};
use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_core::test_fixtures::scale_frame;
use replay_data::{rows::RaidAirScoreRow, rows::TableRows, rows::WarRelationRow};
use replay_frame::frame::TableView;

fn score(country: u32, region: u32, friendly: i64, enemy: i64) -> RaidAirScoreRow {
    RaidAirScoreRow {
        country,
        region,
        friendly,
        enemy,
        impact: Default::default(),
        friendly_missions: 0,
        enemy_missions: 0,
        air_spotting_scale: Some(0),
        menu: None,
    }
}

fn reads<'a>(fixture: &'a Fixture, flag: &'a AtomicBool) -> reader::Reads<'a> {
    reader::Reads {
        source: None,
        store: &fixture.store,
        key: &fixture.key,
        branch: 1,
        cancelled: flag,
    }
}

/// Every view the facts compose answers as classifying each recorded frame
/// does: the side choices, moments, and timelines of the sides, a side and
/// a country, unfiltered and filtered to a side, ranked and every region,
/// through each day's end and part way into it.
fn parity(fixture: &Fixture, frames: &[DecodedFrame], theatres: Option<Vec<String>>) {
    let replay = Arc::new(recording(frames.to_vec()));
    let groups = groups(&replay);
    // Queries read the groups the walk publishes beside the family's pages.
    let mut writer = fixture.writer();
    writer
        .write_at(
            "country-groups-1",
            0,
            groups.saved_bytes().unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
    let flag = AtomicBool::new(false);
    let reads = reads(fixture, &flag);
    let daily = reader::Daily::read(&reads).unwrap();
    let expected = scan::scan_days(&replay, 1, &groups, None, &flag).unwrap();
    let choices = |sides: &[AirSide]| {
        sides
            .iter()
            .map(|side| {
                let mut side = side.clone();
                if let Some((_, name, leader, tag, count, factions, colour)) = side.reads.last() {
                    side.name = name.clone();
                    side.leader = leader.clone();
                    side.tag = tag.clone();
                    side.co_belligerents = *count;
                    side.factions = factions.clone();
                    side.colour_faction = colour.clone();
                }
                side.reads.clear();
                side
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(choices(&daily.dictionary), choices(&expected.sides));
    assert_eq!(daily.state.regions, expected.regions);
    assert_eq!(daily.state.countries, expected.countries);
    assert_eq!(daily.state.theatres, expected.theatres);
    let first = frames.first().unwrap().hour;
    let last = frames.last().unwrap().hour;
    let views = [(None, false), (Some("f:9"), false), (Some("MOD"), true)];
    let view = |request: &mut AirParams, viewpoint: Option<&str>, country: bool| {
        request.regions = None;
        request.theatres = theatres.clone();
        request.top = Some(5000);
        request.mode = Some(if viewpoint.is_some() {
            AirMode::Viewpoint
        } else {
            AirMode::Sides
        });
        request.viewpoint = viewpoint.map(str::to_owned);
        request.viewpoint_country = Some(country);
    };
    for (viewpoint, country) in views {
        let mut request = params(AirView::Moment, first, last);
        view(&mut request, viewpoint, country);
        let state = Prepared::read_query(&fixture.store, &fixture.key, 1).unwrap();
        let mut sides = vec![];
        let samples = state
            .samples(
                &fixture.store,
                &fixture.key,
                1,
                &request,
                &std::sync::OnceLock::new(),
                &groups,
                &daily.anchors,
                &mut sides,
                &flag,
            )
            .unwrap();
        let actual = moment_reply(samples, sides, &groups, &request, &daily.anchors).unwrap();
        let expected = scan_moment(&replay, 1, &groups, &request, &flag).unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    // Each recorded hour and each recorded day's middle.
    let mut ends = BTreeSet::new();
    for frame in frames {
        ends.insert(frame.hour);
        ends.insert(frame.hour.div_euclid(DAY_HOURS) * DAY_HOURS + DAY_HOURS / 2);
    }
    let ends: Vec<i32> = ends
        .into_iter()
        .filter(|end| (first..=last).contains(end))
        .collect();
    for (viewpoint, country) in views {
        let cold = scan::scan_days_kind(&replay, 1, &groups, viewpoint, country, &flag).unwrap();
        for &end in &ends {
            let mut cold = cold.clone();
            if cold
                .day(end.div_euclid(DAY_HOURS))
                .is_some_and(|sample| sample.hour > end)
            {
                cold.partial = scan::partial_day(&replay, 1, &groups, &mut cold, end, &flag)
                    .unwrap()
                    .map(|sample| (end, Arc::new(sample)));
            }
            for start in [first, first.max(end - 30)] {
                for selected in [None, Some(vec!["f:9".to_owned()])] {
                    for top in [5000, 1] {
                        let mut request = params(AirView::Timeline, start, end);
                        view(&mut request, viewpoint, country);
                        request.sides = selected.clone();
                        request.top = Some(top);
                        let actual = reader::timeline(&daily, &reads, &request, i32::MAX).unwrap();
                        assert_eq!(
                            serde_json::to_value(actual).unwrap(),
                            serde_json::to_value(timeline(&cold, &request).unwrap()).unwrap(),
                            "{viewpoint:?} {start}..{end} {selected:?} top {top}"
                        );
                    }
                }
            }
        }
    }
}

/// Days with a zero crossing, a member that is no side's representative, a
/// duplicate owner, a whole missing day and a gap inside a day, theatres
/// recorded and not, a leaderless faction, lone countries at war, a
/// truncated score table, an invalid leader score, and a leaderless
/// coalition on the open last day.
fn mixed_frames() -> Vec<DecodedFrame> {
    use replay_data::rows::{
        AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, FactionMemberRow,
    };
    let theatres = |frame: &mut DecodedFrame| {
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 17,
                owner: 9001,
                mission_region: Some(7000),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirTheatres.index()] =
            TableView::Complete(TableRows::AirTheatres(vec![AirTheatreRow {
                id: 700,
                owner: 9001,
                name: b"Synthetic theatre".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
                id: 701,
                theatre: 700,
                ..Default::default()
            }]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
                wing: 17,
                group: 701,
                ..Default::default()
            }]));
        frame.metrics.table_ns[TableId::AirTheatres.index()] = 1;
    };
    let members = |frame: &mut DecodedFrame, rows: &[(u32, u64, u32)]| {
        *frame.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(
                rows.iter()
                    .map(|&(country, faction, ordinal)| FactionMemberRow {
                        country,
                        faction,
                        ordinal,
                    })
                    .collect(),
            ));
    };
    let wars = |frame: &mut DecodedFrame, pairs: &[(u32, u32)]| {
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(
                pairs
                    .iter()
                    .map(|&(first_country, second_country)| WarRelationRow {
                        first_country,
                        second_country,
                        ..Default::default()
                    })
                    .collect(),
            ));
    };
    let scores = |frame: &mut DecodedFrame, mut rows: Vec<RaidAirScoreRow>, truncated: bool| {
        rows.sort_by_key(|row| (row.country, row.region));
        *frame.tables[TableId::RaidAirScores.index()] = if truncated {
            TableView::Truncated {
                missing: 1,
                rows: TableRows::RaidAirScores(rows),
            }
        } else {
            TableView::Complete(TableRows::RaidAirScores(rows))
        };
    };
    (0..=130)
        .filter(|hour| !(48..=71).contains(hour) && !(80..=84).contains(hour))
        .map(|hour| {
            let friendly = 200_000 + i64::from(hour) * 1_000;
            let mut frame = air(hour, friendly);
            match hour.div_euclid(DAY_HOURS) {
                0 => scores(
                    &mut frame,
                    vec![
                        score(44, 7000, 100_000, friendly),
                        score(9001, 7000, friendly, 100_000),
                        score(55, 7999, 0, 0),
                        score(66, 7999, 0, 0),
                        score(66, 7100, 5_000, 7_000),
                        score(44, 7200, 3_000, 0),
                        score(11, 7200, 4_000, 1),
                        score(9001, 7200, 6_000, 2),
                    ],
                    false,
                ),
                1 => {
                    theatres(&mut frame);
                    scores(
                        &mut frame,
                        vec![
                            score(44, 7000, 100_000, friendly),
                            score(9001, 7000, 0, 100_000),
                            score(55, 7999, 0, 0),
                            score(44, 7200, 3_000, 0),
                        ],
                        false,
                    );
                }
                3 | 4 => {
                    theatres(&mut frame);
                    members(&mut frame, &[(44, 10, 0), (55, 9, 1), (66, 9, 2)]);
                    wars(&mut frame, &[(44, 9001)]);
                    let invalid = (110..=119).contains(&hour);
                    scores(
                        &mut frame,
                        vec![
                            score(9001, 7000, 150_000, 90_000),
                            score(44, 7000, if invalid { -1 } else { 90_000 }, 150_000),
                            score(55, 7100, 1_000, 0),
                            score(66, 7100, 2_000, 0),
                        ],
                        hour == 119,
                    );
                }
                _ => {
                    members(&mut frame, &[(44, 10, 0)]);
                    wars(&mut frame, &[(44, 55), (44, 66)]);
                    scores(
                        &mut frame,
                        vec![
                            score(55, 7100, 3_000, 100),
                            score(66, 7100, 2_500, 100),
                            score(44, 7100, 100, 3_000),
                            score(9001, 7000, 1_000, 0),
                        ],
                        false,
                    );
                }
            }
            frame
        })
        .collect()
}

#[test]
fn facts_compose_every_view_as_the_frame_classifier_across_two_publications() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let frames = mixed_frames();
    let replay = recording(frames.clone());
    let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
    let mut state = Prepared::default();
    let split = frames.partition_point(|frame| frame.hour < 72);
    for chunk in [&frames[..split], &frames[split..]] {
        for frame in chunk {
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
    }
    parity(&fixture, &frames, None);
    parity(&fixture, &frames, Some(vec!["700".into()]));
}

/// Reading only the group runs active at an hour assigns what walking every
/// group run assigns, at every hour and for every set of countries.
#[test]
fn indexed_rosters_assign_what_walking_the_group_history_assigns() {
    let replay = recording(mixed_frames());
    let groups = groups(&replay);
    let index = assignment::GroupIndex::new(&groups);
    let all: BTreeSet<String> = ["GER", "FRA", "CTL", "MOD"].map(String::from).into();
    let some: BTreeSet<String> = ["GER", "CTL"].map(String::from).into();
    let read = |sides: Option<Vec<assignment::AssignedSide>>| {
        sides.map(|sides| {
            let named: Vec<_> = sides
                .iter()
                .map(|side| (side.name.clone(), side.leader.clone(), side.members.clone()))
                .collect();
            (daily::roster(&sides), named)
        })
    };
    for hour in -1..=131 {
        for countries in [&all, &some] {
            assert_eq!(
                read(assignment::indexed_roster(&groups, &index, hour, countries)),
                read(assignment::native_roster(&groups, hour, countries)),
                "{hour}"
            );
        }
    }
}

/// A day a request cuts short assigns its roster from the group runs active
/// then, and finds identities and faction names by halving their histories,
/// however much war, rename and tag history the branch holds.
#[test]
fn a_cut_short_day_reads_only_the_group_runs_active_then() {
    let visited = |days: i32| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (0..days * DAY_HOURS)
            .map(|hour| {
                let day = hour.div_euclid(DAY_HOURS);
                let mut frame = air(hour, 200000);
                // A war, the two tags swapped and a renamed faction on every other day.
                if day % 2 == 0 {
                    *frame.tables[TableId::WarRelations.index()] =
                        TableView::Complete(TableRows::WarRelations(vec![WarRelationRow {
                            first_country: 44,
                            second_country: 55,
                            ..Default::default()
                        }]));
                    if let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *frame.tables[TableId::Countries.index()]
                    {
                        for row in rows.iter_mut() {
                            match row.id {
                                55 => row.tag = b"MOD".to_vec(),
                                66 => row.tag = b"CTL".to_vec(),
                                _ => {}
                            }
                        }
                    }
                }
                if let TableView::Complete(TableRows::Factions(rows)) =
                    &mut *frame.tables[TableId::Factions.index()]
                {
                    rows[0].name = format!("pact {}", day % 2).into_bytes();
                }
                frame
            })
            .collect();
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        for frame in &frames {
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer
            .write_at(
                "country-groups-1",
                0,
                groups.saved_bytes().unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let reads = reads(&fixture, &flag);
        let daily = reader::Daily::read(&reads).unwrap();
        let last = days * DAY_HOURS - 1;
        let before = (
            assignment::VISITED.with(std::cell::Cell::get),
            dashboard_base::products::country_groups::history_steps(),
        );
        let reply = reader::timeline(
            &daily,
            &reads,
            &params(AirView::Timeline, last - 23, last - 12),
            i32::MAX,
        )
        .unwrap();
        assert_eq!(reply.days, [Some(last - 12)], "the day is read live");
        let steps = dashboard_base::products::country_groups::history_steps();
        (
            assignment::VISITED.with(std::cell::Cell::get) - before.0,
            steps.0 - before.1.0,
            steps.1 - before.1.1,
        )
    };
    let (runs, lookups, steps) = visited(4);
    let (long_runs, long_lookups, long_steps) = visited(24);
    assert!(runs > 0 && lookups > 0);
    assert_eq!(
        runs, long_runs,
        "the group runs visited do not grow with the history"
    );
    assert_eq!(lookups, long_lookups, "nor do the history lookups");
    // Halving six times the runs takes at most three more steps a lookup.
    assert!(
        long_steps <= steps + 3 * lookups,
        "{steps} then {long_steps} steps"
    );
}

#[test]
fn superiority_shared_driver_equal_scores_keep_daily_facts_and_native_work_flat() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measured = observer_tests::scaled_work(8, |hours| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut first = scale_frame(1, 25, 24);
        for rows in [
            TableRows::Factions(vec![]),
            TableRows::FactionMembers(vec![]),
            TableRows::WarRelations(vec![]),
            TableRows::HumanControllers(vec![]),
            TableRows::Capitulation(vec![]),
            TableRows::AirWings(vec![]),
        ] {
            let table = rows.table();
            first.tables[table.index()] = TableView::Complete(rows).into();
        }
        first.tables[TableId::RaidAirScores.index()] =
            TableView::Complete(TableRows::RaidAirScores(
                (0..24)
                    .map(|country| score(100000 + country, 7000, i64::from(country) + 1, 99))
                    .collect(),
            ))
            .into();
        let frames: Vec<_> = (0..=25 + hours)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour as i32;
                frame
            })
            .collect();
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[..=25] {
            groups.observe(frame)?;
            state.observe_changed(
                frame,
                &replay,
                &mut groups,
                &tracker.observe(frame, replay.format),
            )?;
        }
        let delta = observer_tests::unchanged_frames(
            state.work,
            &first,
            hours as i32,
            replay.format,
            |frame, changes| {
                groups.observe(frame)?;
                state.observe_changed(frame, &replay, &mut groups, changes)?;
                Ok(state.work)
            },
        )?;
        assert_eq!(delta, observer_tests::Work::default());
        groups.finish(&replay, 1);
        state.publish_day(&groups, true)?;
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        parity(&fixture, &frames, None);
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(measured);
}

#[test]
fn superiority_shared_driver_one_score_change_does_not_walk_historical_regions() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measured = observer_tests::scaled_work(20, |history| {
        let mut frame = air(0, 100000);
        let replay = recording(vec![frame.clone()]);
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for hour in 0..history {
            frame.hour = hour as i32;
            if let TableView::Complete(TableRows::RaidAirScores(rows)) =
                &mut *frame.tables[TableId::RaidAirScores.index()]
            {
                rows[0].region = 1000 + hour as u32;
            }
            groups.observe(&frame)?;
            state.observe_changed(
                &frame,
                &replay,
                &mut groups,
                &tracker.observe(&frame, replay.format),
            )?;
        }
        let end = (frame.hour.div_euclid(DAY_HOURS) + 1) * DAY_HOURS + 1;
        while frame.hour < end {
            frame.hour += 1;
            groups.observe(&frame)?;
            state.observe_changed(
                &frame,
                &replay,
                &mut groups,
                &tracker.observe(&frame, replay.format),
            )?;
        }
        assert!(state.regions.len() >= history);
        let before = state.work;
        let delta = observer_tests::unchanged_frames(
            before,
            &frame,
            10,
            replay.format,
            |frame, changes| {
                groups.observe(frame)?;
                state.observe_changed(frame, &replay, &mut groups, changes)?;
                Ok(state.work)
            },
        )?;
        assert_eq!(delta, observer_tests::Work::default());
        let mut previous = frame.clone();
        previous.hour += 10;
        tracker.observe(&previous, replay.format);
        frame.hour += 11;
        if let TableView::Complete(TableRows::RaidAirScores(rows)) =
            &mut *frame.tables[TableId::RaidAirScores.index()]
        {
            rows[0].friendly += 1;
        }
        groups.observe(&frame)?;
        state.observe_changed(
            &frame,
            &replay,
            &mut groups,
            &tracker.observe(&frame, replay.format),
        )?;
        let changed = observer_tests::Work {
            row_visits: state.work.row_visits - before.row_visits,
            history_visits: state.work.history_visits - before.history_visits,
            emitted: state.work.emitted - before.emitted,
            ..Default::default()
        };
        assert!(changed.row_visits < 100 && changed.history_visits < 10);
        Ok(changed)
    })
    .unwrap();
    observer_tests::assert_flat(measured);
}

/// A held identity's score falling to zero hides its side the next day, as
/// classifying that day's frame does.
#[test]
fn superiority_days_follow_held_identity_scores_crossing_zero() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let first = air(0, 200000);
    let mut zero = first.clone();
    if let TableView::Complete(TableRows::RaidAirScores(rows)) =
        &mut *zero.tables[TableId::RaidAirScores.index()]
    {
        rows.iter_mut()
            .find(|row| row.country == 9001)
            .unwrap()
            .friendly = 0;
    }
    let frames: Vec<_> = (0..=49)
        .map(|hour| {
            let mut frame = if hour < 24 {
                first.clone()
            } else {
                zero.clone()
            };
            frame.hour = hour;
            frame
        })
        .collect();
    let replay = recording(frames.clone());
    let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
    let mut state = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for frame in &frames {
        groups.observe(frame).unwrap();
        state
            .observe_changed(
                frame,
                &replay,
                &mut groups,
                &tracker.observe(frame, replay.format),
            )
            .unwrap();
    }
    groups.finish(&replay, 1);
    state.publish_day(&groups, true).unwrap();
    let mut writer = fixture.writer();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    parity(&fixture, &frames, None);
    let reads = reads(&fixture, &flag);
    let daily = reader::Daily::read(&reads).unwrap();
    let reply =
        reader::timeline(&daily, &reads, &params(AirView::Timeline, 0, 47), i32::MAX).unwrap();
    assert_eq!(
        reply.rows[0].cells.len(),
        2,
        "zero scores hide a side even when its identity is held"
    );
}

#[test]
fn superiority_equal_theatre_reads_publish_dates_without_copying_the_dictionary() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    for hours in [8, 80] {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut first = air(25, 200000);
        first.metrics.table_ns[TableId::AirTheatres.index()] = 1;
        first.tables[TableId::AirTheatres.index()] = TableView::Complete(TableRows::AirTheatres(
            (0..128)
                .map(|id| replay_data::rows::AirTheatreRow {
                    id,
                    name: format!("theatre-{id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ))
        .into();
        first.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![])).into();
        first.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![])).into();
        let frames: Vec<_> = (0..=25 + hours)
            .map(|hour| {
                let mut frame = first.clone();
                frame.hour = hour;
                frame
            })
            .collect();
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        for frame in &frames[..=25] {
            groups.observe(frame).unwrap();
            state
                .observe_changed(
                    frame,
                    &replay,
                    &mut groups,
                    &tracker.observe(frame, replay.format),
                )
                .unwrap();
        }
        let work = observer_tests::unchanged_frames(
            state.work,
            &first,
            hours,
            replay.format,
            |frame, changes| {
                groups.observe(frame)?;
                state.observe_changed(frame, &replay, &mut groups, changes)?;
                Ok(state.work)
            },
        )
        .unwrap();
        assert_eq!(
            work,
            observer_tests::Work {
                emitted: hours as usize,
                ..Default::default()
            },
            "the only hourly work is its small supplying-date event"
        );
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            "air-superiority-1-operands".into(),
            &flag,
        )
        .unwrap();
        assert_eq!(reader.event_count_through(1, i32::MAX).unwrap(), 1);
        let (_, date) = reader.latest_value(2, 25 + hours).unwrap().unwrap();
        assert_eq!(date, serde_json::json!(25 + hours));
        parity(&fixture, &frames, None);
    }
}

#[test]
fn prepared_air_superiority_two_publications_resume_append_and_final_side_keys_match() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut frames = vec![
        air(23, 200000),
        air(24, i64::MAX),
        air(47, 400000),
        air(48, 500000),
        air(49, 600000),
    ];
    for frame in &mut frames {
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![replay_data::rows::AirWingRow {
                id: 17,
                owner: 9001,
                mission_region: Some(if frame.hour >= 48 { 7999 } else { 7000 }),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirTheatres.index()] =
            TableView::Complete(TableRows::AirTheatres(vec![
                replay_data::rows::AirTheatreRow {
                    id: 700,
                    owner: 9001,
                    name: b"Synthetic theatre".to_vec(),
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::AirTheatreGroups.index()] =
            TableView::Complete(TableRows::AirTheatreGroups(vec![
                replay_data::rows::AirTheatreGroupRow {
                    id: 701,
                    theatre: 700,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::AirTheatreMembers.index()] =
            TableView::Complete(TableRows::AirTheatreMembers(vec![
                replay_data::rows::AirTheatreMemberRow {
                    wing: 17,
                    group: 701,
                    ..Default::default()
                },
            ]));
        frame.metrics.table_ns[TableId::AirTheatres.index()] = 1;
    }
    if let TableView::Complete(TableRows::Factions(rows)) =
        &mut *frames[2].tables[TableId::Factions.index()]
    {
        rows[0].name = b"Renamed synthetic pact".to_vec();
    }
    for frame in &mut frames[3..] {
        *frame.tables[TableId::WarRelations.index()] =
            TableView::Complete(TableRows::WarRelations(vec![
                WarRelationRow {
                    first_country: 44,
                    second_country: 66,
                    ..Default::default()
                },
                WarRelationRow {
                    first_country: 9001,
                    second_country: 66,
                    ..Default::default()
                },
            ]));
    }
    let replay = recording(frames.clone());
    let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
    let mut state = Prepared::default();
    for chunk in [&frames[..1], &frames[1..3]] {
        let mut writer = fixture.writer();
        for frame in chunk {
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        // The restored observer resumes from the same roster.
        let roster = |state: &Prepared| {
            (
                state.roster_dirty,
                serde_json::to_string(&state.roster).unwrap(),
                state.war_ends.clone(),
            )
        };
        let continued = roster(&state);
        state = Prepared::read(&fixture.store, &fixture.key, None, 1).unwrap();
        assert_eq!(roster(&state), continued);
    }
    parity(&fixture, &frames[..3], None);
    parity(&fixture, &frames[..3], Some(vec!["700".into()]));
    let previous = fixture.key.clone();
    let mut writer = fixture.append_writer();
    state = Prepared::read(&fixture.store, &fixture.key, Some(&previous), 1).unwrap();
    for frame in &frames[3..] {
        groups.observe(frame).unwrap();
        state.observe(frame, &replay, &mut groups).unwrap();
    }
    groups.finish(&replay, 1);
    state.publish_day(&groups, true).unwrap();
    state.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    parity(&fixture, &frames, None);
    parity(&fixture, &frames, Some(vec!["700".into()]));
    let reads = reads(&fixture, &flag);
    let daily = reader::Daily::read(&reads).unwrap();
    let reply =
        reader::timeline(&daily, &reads, &params(AirView::Timeline, 0, 49), i32::MAX).unwrap();
    assert_eq!(
        reply.days,
        [Some(23), Some(47), Some(49)],
        "the finished days' samples and the open day's last hour"
    );
}

#[test]
fn prepared_air_superiority_continuation_grows_with_distinct_operands_and_not_days() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let replay = recording(vec![air(0, 200000)]);
    let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
    let mut state = Prepared::default();
    let mut sizes = vec![];
    for end in [100, 10000] {
        let start = state.previous_hour.map_or(0, |hour| hour + 1);
        let mut writer = fixture.writer();
        for hour in start..end {
            let frame = air(hour, 200000);
            groups.observe(&frame).unwrap();
            state.observe(&frame, &replay, &mut groups).unwrap();
        }
        state.publish_day(&groups, true).unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        sizes.push(serde_json::to_vec(&state).unwrap().len());
        assert!(state.facts.checkpoints().is_some());
    }
    eprintln!("air-superiority continuation bytes after 100 and 10000 hours: {sizes:?}");
    assert!(sizes[1] < sizes[0] + 4096);
}

/// A timeline request composes only its rows' cells, once per run of equal
/// facts, and reads only a day it cuts short, however many days are kept.
#[test]
fn a_timeline_request_composes_its_rows_and_not_every_kept_day() {
    let composed = |days: i32| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let frames: Vec<_> = (0..days * DAY_HOURS)
            .map(|hour| air(hour, 200000))
            .collect();
        let replay = recording(frames.clone());
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared::default();
        for frame in &frames {
            groups.observe(frame).unwrap();
            state.observe(frame, &replay, &mut groups).unwrap();
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer
            .write_at(
                "country-groups-1",
                0,
                groups.saved_bytes().unwrap().as_slice(),
            )
            .unwrap();
        writer.commit().unwrap();
        let reads = reads(&fixture, &flag);
        let daily = reader::Daily::read(&reads).unwrap();
        let last = days * DAY_HOURS - 1;
        let mut counts = Vec::new();
        for (first, last) in [(0, last), (0, last - 12), (last - 47, last)] {
            let before = reader::COMPOSED.with(std::cell::Cell::get);
            let reply = reader::timeline(
                &daily,
                &reads,
                &params(AirView::Timeline, first, last),
                i32::MAX,
            )
            .unwrap();
            assert_eq!(
                reply.days.len() as i32,
                last.div_euclid(DAY_HOURS) - first.div_euclid(DAY_HOURS) + 1
            );
            counts.push(reader::COMPOSED.with(std::cell::Cell::get) - before);
        }
        counts
    };
    let short = composed(4);
    assert_eq!(
        short,
        composed(24),
        "per-request work does not grow with the kept days"
    );
    assert!(short.iter().all(|count| *count <= 4), "{short:?}");
}

/// Held key ids and the workers' pair diffs publish, byte for byte, what
/// naming every field anew and walking every pair publishes, the daily facts
/// among them: scores changing every hour of an air war, a score's sign
/// flipping, a pair appearing, going and coming back, a pair unchanged for
/// hours, a new region, a wing moving, a gap, two day boundaries and the
/// worker's projections starting again at block boundaries.
#[test]
fn superiority_held_keys_and_pair_diffs_publish_what_plain_observation_does() {
    use super::super::block::ScoreTracker;
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::AirWingRow;
    let frames: Vec<_> = (0..60)
        .filter(|hour| *hour != 33)
        .map(|hour| {
            let mut frame = air(hour, 200_000 + i64::from(hour) * 1_000);
            let germany = if hour % 7 == 3 {
                -500
            } else {
                1_000 + i64::from(hour)
            };
            let mut scores = vec![
                score(44, 7000, 100_000, 200_000 + i64::from(hour) * 1_000),
                score(9001, 7000, germany, 100_000),
            ];
            if (10..20).contains(&hour) || (30..40).contains(&hour) {
                scores.push(score(55, 7999, 3_000 + i64::from(hour % 3), 0));
            }
            if hour >= 25 {
                scores.push(score(66, 7100, 4_000, 1_000));
            }
            scores.sort_by_key(|row| (row.country, row.region));
            *frame.tables[TableId::RaidAirScores.index()] =
                TableView::Complete(TableRows::RaidAirScores(scores));
            if hour >= 5 {
                *frame.tables[TableId::AirWings.index()] =
                    TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                        id: 1,
                        owner: 44,
                        mission_region: Some(if hour < 50 { 7000 } else { 7200 }),
                        ..Default::default()
                    }]));
            }
            frame
        })
        .collect();
    let replay = recording(frames.clone());
    let flag = AtomicBool::new(false);
    let run = |plain: bool| -> (BTreeMap<(String, u64), Vec<u8>>, Prepared) {
        let fixture = Fixture::new();
        let mut groups = dashboard_base::products::country_groups::Scan::new(&replay, 1);
        let mut state = Prepared {
            plain,
            ..Default::default()
        };
        state.measure();
        let mut changes = FrameChangeTracker::new();
        let mut worker = ScoreTracker::default();
        for frame in &frames {
            // The pre-pass reads each block alone: its worker starts
            // again at each block's first hour.
            if [20, 45].contains(&frame.hour) {
                worker = ScoreTracker::default();
            }
            let changes = changes.observe(frame, replay.format);
            groups.observe(frame).unwrap();
            if plain {
                state
                    .observe_changed(frame, &replay, &mut groups, &changes)
                    .unwrap();
            } else {
                let scored = worker.scored(frame, &changes).unwrap();
                state
                    .observe_scored(frame, &replay, &mut groups, &changes, scored)
                    .unwrap();
            }
        }
        groups.finish(&replay, 1);
        state.publish_day(&groups, true).unwrap();
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        (fixture.payloads(), state)
    };
    let (plain, _) = run(true);
    let (held, mut state) = run(false);
    for name in [
        "air-superiority-1",
        "air-superiority-1-values",
        "air-superiority-1-operands",
        "air-superiority-1-hours",
        "air-superiority-1-catalogue",
    ] {
        assert!(
            plain.keys().any(|(section, _)| section == name),
            "{name} was not written"
        );
    }
    assert!(
        plain
            .keys()
            .any(|(section, _)| section.starts_with(dashboard_base::prepared::facts::SECTIONS)),
        "the daily facts were written"
    );
    assert_eq!(
        held.keys().collect::<Vec<_>>(),
        plain.keys().collect::<Vec<_>>()
    );
    for (name, bytes) in &plain {
        assert_eq!(&held[name], bytes, "{name:?}");
    }
    let parts: BTreeMap<_, _> = state
        .take_parts()
        .into_iter()
        .map(|(name, _, times)| (name, times))
        .collect();
    assert!(
        parts
            .get("air_superiority.fold.diffed")
            .is_some_and(|times| *times > 20),
        "most war hours observe only their changed pairs: {parts:?}"
    );
    assert!(
        parts.contains_key("air_superiority.fold.pairs"),
        "block starts and the gap walk every pair: {parts:?}"
    );
}

#[test]
fn prepared_air_superiority_window_list_reads_no_scan_state() {
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "prepared-air-windows".into(),
            revision: "1".into(),
        },
        "synthetic.battleplan".into(),
        Arc::new(recording(vec![air(0, 100000)])),
        &AtomicBool::new(false),
    )
    .unwrap();
    let mut request = params(AirView::Choices, 0, 0);
    request.windows_only = Some(true);
    let reply = query(
        &source,
        serde_json::to_value(request).unwrap(),
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap();
    assert!(!reply["windows"].as_array().unwrap().is_empty());
    assert!(
        source
            .append_state::<AirCache>()
            .lock()
            .unwrap()
            .branches
            .is_empty()
    );
}
