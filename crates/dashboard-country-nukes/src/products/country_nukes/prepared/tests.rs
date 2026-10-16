use super::*;
#[cfg(feature = "slow-tests")]
use dashboard_base::test_fixtures::continuation_records;
use dashboard_base::test_fixtures::{CAMPAIGN, records, scale_frame};
use replay::{frame::TableView, rows::TableRows};

fn frame(hour: i32, count: u32) -> DecodedFrame {
    let mut frame = scale_frame(1, hour, count);
    *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
        (0..count)
            .flat_map(|index| {
                (0..=1).map(move |ordinal| NukeRow {
                    country: 100_000 + index,
                    nuke_type: ordinal,
                    amount_raw: (i64::from(index) + i64::from(hour) - i64::from(ordinal))
                        * 100_000_000,
                    ..Default::default()
                })
            })
            .collect(),
    ));
    frame
}

/// Hours whose keys, links, tokens and stockpile rows are the hour before's
/// step in constant time to the state observing them reaches (each dates its
/// read), and on to the same state at the next changed hour. A step that
/// observing would take otherwise (after a gap, with another read flag, read
/// back from a continuation, or a read of a stockpile table that is not
/// complete, whose reason is the frame's) declines and changes nothing.
#[test]
fn a_quiet_hour_steps_to_the_state_observing_it_reaches() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let first = frame(100, 3);
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first.clone()])]).unwrap();
    let state = |prepared: &Prepared| serde_json::to_value(prepared).unwrap();
    // Each later hour shares every table of `from`: nothing changed.
    let at = |from: &DecodedFrame, hour: i32| {
        let mut frame = from.clone();
        frame.hour = hour;
        frame
    };
    let read = |hour: i32| slot_read(replay.format, &first.metrics, TableId::Nukes, hour);
    let mut tracker = FrameChangeTracker::new();
    let changes = tracker.observe(&first, replay.format);
    let (mut observed, mut stepped) = (Prepared::default(), Prepared::default());
    observed.observe(&first, &replay, &changes).unwrap();
    stepped.observe(&first, &replay, &changes).unwrap();
    for hour in 101..=105 {
        let frame = at(&first, hour);
        let changes = tracker.observe(&frame, replay.format);
        assert!(
            !changes.rows_changed[TableId::Nukes.index()],
            "hour {hour} is quiet"
        );
        observed.observe(&frame, &replay, &changes).unwrap();
        assert!(
            stepped.advance_quiet(hour, read(hour)).unwrap(),
            "hour {hour} steps"
        );
        assert_eq!(state(&stepped), state(&observed), "hour {hour}");
    }
    assert_eq!(
        state(&stepped)["feed_reads"],
        serde_json::json!([{"firstHour": 100, "lastHour": 105}]),
        "every quiet read is dated"
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
    let mut changed = at(&first, 106);
    if let TableView::Complete(TableRows::Nukes(rows)) =
        &mut *changed.tables[TableId::Nukes.index()]
    {
        rows[0].amount_raw += 200_000_000;
    }
    let changes = tracker.observe(&changed, replay.format);
    observed.observe(&changed, &replay, &changes).unwrap();
    stepped.observe(&changed, &replay, &changes).unwrap();
    assert_eq!(state(&stepped), state(&observed));
    // A stockpile table read but not complete names its reason from the
    // frame: the hours after it are observed.
    let mut unread = at(&changed, 107);
    *unread.tables[TableId::Nukes.index()] = TableView::Unavailable {
        reason: "Stockpiles not read".into(),
    };
    let changes = tracker.observe(&unread, replay.format);
    stepped.observe(&unread, &replay, &changes).unwrap();
    assert!(read(108) && !stepped.advance_quiet(108, true).unwrap());
}

#[test]
#[cfg(feature = "slow-tests")]
fn progressive_nukes_sampler_matches_observer_on_every_key_at_5000_countries() {
    let frames = [frame(0, 5000), frame(1, 5000)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let mut observer = Prepared::default();
    for frame in &frames {
        let samples = sample(frame, None).unwrap();
        assert_eq!(samples.len(), 15_000);
        observer
            .observe(
                frame,
                &replay,
                &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        for ((tag, field), (identity, amount)) in &samples {
            assert_eq!(
                *identity,
                observer.scan.tracks[tag].last.map(|(identity, _)| identity)
            );
            let key = observer.keys[&series::field_key(tag, field)];
            let values = if field.is_empty() {
                &observer.values
            } else {
                &observer.parts
            };
            assert_eq!(
                values.state(key).unwrap().last.unwrap().value,
                amount.unwrap()
            );
        }
    }
}

#[test]
fn progressive_nukes_sampler_preserves_prefix_holes_duplicates_and_absence() {
    let mut frame = frame(0, 2);
    if let TableView::Complete(TableRows::Nukes(rows)) = &mut *frame.tables[TableId::Nukes.index()]
    {
        rows[0].nuke_type = 3;
        rows.push(rows[2].clone());
    }
    let tags: Vec<_> = (0..2)
        .map(|index| String::from_utf8(dashboard_base::test_fixtures::scale_tag(index)).unwrap())
        .collect();
    let requested = vec![
        (tags[0].clone(), String::new()),
        (tags[0].clone(), "3".into()),
        (tags[1].clone(), String::new()),
        ("tail absent".into(), String::new()),
    ];
    let samples = sample(&frame, Some(&requested)).unwrap();
    assert_eq!(samples.len(), requested.len());
    assert_eq!(samples[&requested[0]], (Some((0, 100_000)), None));
    assert_eq!(samples[&requested[1]], (Some((0, 100_000)), Some(0)));
    assert_eq!(samples[&requested[2]], (Some((1, 100_001)), None));
    assert_eq!(samples[&requested[3]], (None, None));
    assert_eq!(
        supplying_read(&frame, replay::format::FormatVersion::CURRENT),
        Some(false),
        "a refused array cannot prove a complete empty census"
    );
}

#[test]
#[cfg(feature = "slow-tests")]
fn global_nuclear_increases_match_prepared_mixtures_after_two_commits_resume_and_append_at_5000_countries()
 {
    let make = |hour| {
        let amount_hour = [0, 1, 1, 3, 2, 4][hour as usize];
        let mut frame = frame(amount_hour, 5000);
        frame.hour = hour;
        if let TableView::Complete(TableRows::Nukes(rows)) =
            &mut *frame.tables[TableId::Nukes.index()]
        {
            rows.retain(|row| row.nuke_type == 0);
        }
        frame
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, (0..=3).map(make).collect())]).unwrap();
    let flag = AtomicBool::new(false);
    let mut fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    let rows = |reader: &mut Reader<'_>, keys: &[u64]| {
        let mut rows = Vec::new();
        let mut cursor = 0;
        let mut pages = 0;
        loop {
            pages += 1;
            assert!(
                pages <= (5000usize * 6).div_ceil(1024),
                "six supplying frames bound all page reads at 5,000 countries"
            );
            let page = reader.events(keys, 0, 5, cursor, 1024).unwrap();
            rows.extend(page.rows);
            let Some(next) = page.cursor else { break };
            assert!(
                next > cursor,
                "a supplying page cannot repeat the same cursor"
            );
            cursor = next;
        }
        rows
    };
    for (stage, hours) in [(0, 0..=1), (1, 2..=3), (2, 4..=5)] {
        if stage == 2 {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    (4..=5).map(make).collect(),
                )])
                .unwrap();
        }
        for hour in hours {
            prepared
                .observe(
                    &make(hour),
                    &replay,
                    &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&make(hour), replay.format),
                )
                .unwrap();
        }
        let mut writer = if stage == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        if stage == 0 {
            continue;
        }
        let inverse: BTreeMap<_, _> = prepared
            .scan
            .tracks
            .keys()
            .map(|tag| (prepared.keys[&series::field_key(tag, "")], tag.clone()))
            .collect();
        let mut mixtures =
            Reader::new(&fixture.store, &fixture.key, "nukes-mixes-1".into(), &flag).unwrap();
        let mut by_tag = BTreeMap::<String, Vec<NukeRun>>::new();
        for row in rows(&mut mixtures, &inverse.keys().copied().collect::<Vec<_>>()) {
            by_tag
                .entry(inverse[&row.key].clone())
                .or_default()
                .push(serde_json::from_value(row.row).unwrap());
        }
        let mut expected = Vec::new();
        for (tag, runs) in &mut by_tag {
            runs.sort_by_key(|run| run.first_hour);
            for pair in runs.windows(2) {
                if !pair[1].starts_segment
                    && let (Some(before), Some(after)) = (pair[0].total, pair[1].total)
                {
                    let delta = i64::from(after) - i64::from(before);
                    if delta > 0 {
                        expected.push((
                            tag.clone(),
                            pair[1].first_hour,
                            pair[1].first_hour,
                            delta as u32,
                        ));
                    }
                }
            }
        }
        let mut increases = Reader::new(
            &fixture.store,
            &fixture.key,
            format!("{INCREASES}-1"),
            &flag,
        )
        .unwrap();
        let mut actual: Vec<_> = rows(&mut increases, &[INCREASE_KEY])
            .into_iter()
            .map(|row| {
                assert_eq!(row.key, INCREASE_KEY);
                assert_eq!(row.hour, row.row["readHour"].as_i64().unwrap() as i32);
                assert!(row.row["typeOrdinal"].is_null());
                assert_eq!(row.row["exact"], false);
                assert_eq!(row.row["types"][0]["ordinal"], 0);
                assert_eq!(row.row["types"][0]["supported"], true);
                (
                    row.row["tag"].as_str().unwrap().to_owned(),
                    row.row["hour"].as_i64().unwrap() as i32,
                    row.row["readHour"].as_i64().unwrap() as i32,
                    row.row["amount"].as_u64().unwrap() as u32,
                )
            })
            .collect();
        expected.sort();
        actual.sort();
        assert_eq!(actual.len(), 5000 * if stage == 1 { 2 } else { 3 });
        assert_eq!(
            actual, expected,
            "one global event key must reproduce prepared mixture increases across publication boundaries"
        );
    }
}

#[test]
fn global_nuclear_increases_keep_the_feed_total_rule_unknowns_segments_and_metadata() {
    let make = |hour, counts: &[(u32, i32)]| {
        let mut frame = frame(hour, 1);
        *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(
            counts
                .iter()
                .map(|&(nuke_type, amount)| NukeRow {
                    country: 100_000,
                    nuke_type,
                    amount_raw: i64::from(amount) * 100_000_000,
                    ..Default::default()
                })
                .collect(),
        ));
        let mut lexical = vec![
            replay::rows::LexicalTokenRow {
                token: NUCLEAR_BOMB_TOKEN,
                literal: b"synthetic counter".to_vec(),
            },
            replay::rows::LexicalTokenRow {
                token: THERMONUCLEAR_BOMB_TOKEN,
                literal: b"synthetic alternative".to_vec(),
            },
        ];
        lexical.sort_by_key(|row| row.token);
        *frame.tables[TableId::LexicalTokens.index()] =
            TableView::Complete(TableRows::LexicalTokens(lexical));
        frame
    };
    let frames = vec![
        make(0, &[(0, 0), (1, 2)]),
        make(1, &[(0, 2), (1, 0)]),
        make(2, &[(0, 1), (1, 0)]),
        make(3, &[(0, 3), (1, 0)]),
        make(4, &[(0, 3), (2, 5)]),
        make(5, &[(0, 5), (1, 0)]),
        make(6, &[(0, -5), (1, 0)]),
        make(7, &[(0, 5), (1, 0)]),
        make(9, &[(0, 6), (1, 0)]),
        make(10, &[(0, 7), (1, 0)]),
    ];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for (first, last) in [(0, 3), (4, 10)] {
        for frame in frames
            .iter()
            .filter(|frame| frame.hour >= first && frame.hour <= last)
        {
            prepared
                .observe(
                    frame,
                    &replay,
                    &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(frame, replay.format),
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        format!("{INCREASES}-1"),
        &flag,
    )
    .unwrap();
    let page = reader.events(&[INCREASE_KEY], 0, 10, 0, 100).unwrap();
    assert_eq!(
        page.total, 3,
        "part-only increases, decreases, unknown totals and segment starts emit no total increase"
    );
    assert_eq!(
        page.rows
            .iter()
            .map(|row| (row.hour, row.row["amount"].as_u64().unwrap()))
            .collect::<Vec<_>>(),
        vec![(3, 2), (7, 10), (10, 1)]
    );
    for row in page.rows {
        let increase: StockpileIncrease = serde_json::from_value(row.row).unwrap();
        assert_eq!(
            increase.tag,
            String::from_utf8(dashboard_base::test_fixtures::scale_tag(0)).unwrap()
        );
        assert_eq!(increase.slot, 0);
        assert_eq!(increase.country_id, 100_000);
        assert_eq!(increase.hour, increase.read_hour);
        assert_eq!(increase.read_hour, row.hour);
        assert!(!increase.exact);
        assert!(increase.type_ordinal.is_none());
        assert_eq!(
            increase.types[0].token.as_deref(),
            Some("synthetic counter")
        );
        assert!(increase.types[0].supported);
        assert!(
            increase.types[0].name.is_none(),
            "names resolve from the recorded content at request time"
        );
        if row.hour >= 7 {
            assert_eq!(increase.types[2].ordinal, 2);
            assert!(!increase.types[2].supported);
        }
    }
    assert_eq!(
        reader
            .events(&[INCREASE_KEY], 3, 7, 0, 100)
            .unwrap()
            .rows
            .len(),
        2,
        "the shared event reader serves a clipped global page"
    );
    assert!(page.cursor.is_none());
}

#[test]
#[cfg(feature = "slow-tests")]
fn global_nuclear_increases_widen_the_native_signed_total_difference_to_u32() {
    let make = |hour, target: i32| {
        let mut frame = scale_frame(1, hour, 1);
        let mut remaining = i64::from(target);
        let mut rows = Vec::new();
        while remaining != 0 {
            let amount = remaining.clamp(-900_000, 900_000);
            rows.push(NukeRow {
                country: 100_000,
                nuke_type: rows.len() as u32,
                amount_raw: amount * 100_000_000,
                ..Default::default()
            });
            remaining -= amount;
        }
        *frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(rows));
        frame
    };
    let frames = [make(0, i32::MIN), make(1, i32::MAX)];
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap();
    let flag = AtomicBool::new(false);
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let mut prepared = Prepared::default();
    for (frame, expected) in frames.iter().zip([i32::MIN, i32::MAX]) {
        prepared
            .observe(
                frame,
                &replay,
                &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                    .observe(frame, replay.format),
            )
            .unwrap();
        assert_eq!(
            prepared
                .scan
                .tracks
                .values()
                .next()
                .unwrap()
                .runs
                .last()
                .unwrap()
                .total,
            Some(expected),
            "synthetic arrays must produce the real native signed total"
        );
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut reader = Reader::new(
        &fixture.store,
        &fixture.key,
        format!("{INCREASES}-1"),
        &flag,
    )
    .unwrap();
    let page = reader.events(&[INCREASE_KEY], 0, 1, 0, 1).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.rows[0].row["amount"], u32::MAX);
}

#[test]
fn nuclear_feed_catalogue_preserves_strike_only_types_reads_and_refusals_after_two_commits_and_resume()
 {
    let make = |hour| {
        let mut frame = scale_frame(1, hour, 1);
        *frame.tables[TableId::Nukes.index()] = match hour {
            0 | 1 => TableView::Unavailable {
                reason: "synthetic stockpile census unavailable".into(),
            },
            2 => TableView::Truncated {
                rows: TableRows::Nukes(Vec::new()),
                missing: 2,
            },
            _ => TableView::Complete(TableRows::Nukes(vec![NukeRow {
                country: 100_000,
                nuke_type: 2,
                amount_raw: 0,
                ..Default::default()
            }])),
        };
        if hour == 0 {
            let mut tokens = vec![
                replay::rows::LexicalTokenRow {
                    token: NUCLEAR_BOMB_TOKEN,
                    literal: b"synthetic strike type".to_vec(),
                },
                replay::rows::LexicalTokenRow {
                    token: THERMONUCLEAR_BOMB_TOKEN,
                    literal: b"synthetic strike alternative".to_vec(),
                },
            ];
            tokens.sort_by_key(|row| row.token);
            *frame.tables[TableId::LexicalTokens.index()] =
                TableView::Complete(TableRows::LexicalTokens(tokens));
        }
        *frame.tables[TableId::NuclearStrikes.index()] =
            TableView::Complete(TableRows::NuclearStrikes(vec![
                replay::rows::NuclearStrikeRow {
                    id: 7,
                    hour: 0,
                    launcher: replay::event_feed_rows::EventFeedCountry {
                        reference: 100_000,
                        id: 100_000,
                        instance: 0,
                    },
                    nuke_type: 1,
                    province: 13_000,
                    state: 2,
                    owner: replay::event_feed_rows::EventFeedCountry {
                        reference: 100_000,
                        id: 100_000,
                        instance: 0,
                    },
                    controller: 100_000,
                    ..Default::default()
                },
            ]));
        frame
    };
    let frames = (0..=5).map(make).collect::<Vec<_>>();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
    let fixture = dashboard_base::prepared::series::tests::Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for (stage, hours) in [(0, 0..=3), (1, 4..=5)] {
        for hour in hours {
            prepared
                .observe(
                    &frames[hour as usize],
                    &replay,
                    &dashboard_base::prepared::frame_changes::FrameChangeTracker::new()
                        .observe(&frames[hour as usize], replay.format),
                )
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        let (keys, catalogue) =
            series::read_catalogue::<IncreaseCatalogue>(&fixture.store, &fixture.key, INCREASES, 1)
                .unwrap()
                .expect(
                    "strike-only observations publish their feed catalogue without an increase",
                );
        assert!(
            keys.is_empty(),
            "the global event key needs no tag catalogue"
        );
        assert_eq!(catalogue.types[0].ordinal, 0);
        assert_eq!(
            catalogue.types[0].token.as_deref(),
            Some("synthetic strike type")
        );
        assert_eq!(catalogue.types[1].ordinal, 1);
        assert_eq!(
            catalogue.types[1].token.as_deref(),
            Some("synthetic strike alternative")
        );
        assert!(
            catalogue.types[..2]
                .iter()
                .all(|kind| kind.supported && kind.name.is_none())
        );
        assert_eq!(
            catalogue.refusals["synthetic stockpile census unavailable"],
            vec![HourSpan {
                first_hour: 0,
                last_hour: 1
            }],
            "equal refusal reasons coalesce across reads"
        );
        assert_eq!(
            catalogue.read_spans,
            vec![HourSpan {
                first_hour: 3,
                last_hour: if stage == 0 { 3 } else { 5 }
            }],
            "complete-read availability coalesces across commits and survives resume"
        );
        assert_eq!(
            catalogue.refusals["Nukes is truncated (2 missing rows)"],
            vec![HourSpan {
                first_hour: 2,
                last_hour: 2
            }]
        );
        assert_eq!(
            catalogue.refused_types,
            BTreeMap::from([(2, if stage == 0 { 1 } else { 3 })])
        );
        assert_eq!(catalogue.types[2].ordinal, 2);
        assert!(!catalogue.types[2].supported);
        assert!(catalogue.types[2].token.is_none());
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            format!("{INCREASES}-1"),
            &flag,
        )
        .unwrap();
        assert_eq!(
            reader.events(&[INCREASE_KEY], 0, 5, 0, 10).unwrap().total,
            0,
            "strike-only campaigns never manufacture stockpile increase rows"
        );
    }
}

#[test]
fn phase_b_unchanged_hours_and_retained_holders_are_flat_and_one_row_is_bounded() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let measure = |history: usize, hours: i32| -> io::Result<observer_tests::Work> {
        let mut first = frame(100, 128);
        if let TableView::Complete(TableRows::Nukes(rows)) =
            &mut *first.tables[TableId::Nukes.index()]
        {
            rows.truncate(1);
        }
        let replay = Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(
                1,
                vec![first.clone()],
            )],
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
        if let TableView::Complete(TableRows::Nukes(rows)) =
            &mut *first.tables[TableId::Nukes.index()]
        {
            rows[0].amount_raw += 100_000_000;
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
            changed.row_visits <= 1 && changed.history_visits <= 1 && changed.emitted <= 8,
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
            *next.tables[TableId::Nukes.index()] = TableView::Unavailable {
                reason: "synthetic refusal".into(),
            };
        }
        oracle.observe(&next, replay::format::FormatVersion::CURRENT);
        lazy.observe_changes(
            &next,
            replay::format::FormatVersion::CURRENT,
            Some(&tracker.observe(&next, replay::format::FormatVersion::CURRENT)),
        );
        if hour == 103 || hour == 108 {
            lazy.seal();
            lazy = serde_json::from_slice(&serde_json::to_vec(&lazy).unwrap()).unwrap();
            tracker.invalidate();
        }
    }
    observer_tests::assert_reply_parity(&lazy.finish(), &oracle.finish()).unwrap();
}
