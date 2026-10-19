use super::super::tests::{fuel_frame, fuel_slot};
use super::*;
use dashboard_base::prepared::series::tests::Fixture;
use dashboard_base::test_fixtures::{CAMPAIGN, records};
use replay::container::Replay;
use replay_data::rows::TableRows;
use replay_frame::frame::TableView;

#[test]
fn preassembled_demand_preserves_fuel_state_and_named_part_failures() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::{rows::FuelDemandRead, rows::FuelDemandUnit, rows::Recorded};
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut direct = Prepared::default();
    let mut extracted = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for day in 0..3 {
        let hour = fuel_slot(1000 + day);
        let mut frame = fuel_frame(hour, true, 9001, (100, 200, 3, 4));
        let (header, parts) = FuelDemandRead {
            divisions: vec![FuelDemandUnit {
                reference: 17,
                rate_raw: Some(500 + i64::from(day)),
                ..Default::default()
            }],
            ..Default::default()
        }
        .split(11)
        .unwrap();
        if let TableView::Complete(TableRows::CountryFuel(rows)) =
            &mut *frame.tables[TableId::CountryFuel.index()]
        {
            rows[0].demand = Recorded::Value(header);
        }
        *frame.tables[TableId::FuelDemandParts.index()] = match day {
            0 => TableView::Complete(TableRows::FuelDemandParts(parts)),
            1 => TableView::Truncated {
                rows: TableRows::FuelDemandParts(parts),
                missing: 1,
            },
            _ => TableView::Unavailable {
                reason: "synthetic missing parts".into(),
            },
        };
        let changes = tracker.observe(&frame, format);
        let demand = replay_frame::rows::with_demand_parts(
            frame.complete::<tables::CountryFuel>().unwrap(),
            &frame.tables[TableId::FuelDemandParts.index()],
            format,
        )
        .unwrap();
        direct
            .observe_changes(&frame, format, &changes, &flag)
            .unwrap();
        *frame.tables[TableId::FuelDemandParts.index()] = TableView::Unavailable {
            reason: "parts released after assembly".into(),
        };
        extracted
            .observe_demand(&frame, format, &changes, Some(&demand), &flag)
            .unwrap();
        assert_eq!(
            serde_json::to_vec(&extracted).unwrap(),
            serde_json::to_vec(&direct).unwrap(),
            "day {day}"
        );
    }
}

#[test]
fn fuel_country_keys_skip_balance_only_recorded_deltas() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let format = replay_data::format::FormatVersion::CURRENT;
    let mut frame = dashboard_base::test_fixtures::scale_frame(1, 100, 200);
    let mut encoder = replay_frame::frame::FrameEncoder::new();
    let mut decoder = replay_frame::frame::FrameDecoder::new();
    let mut tracker = FrameChangeTracker::new();
    let mut scan = Scan::default();
    for hour in 100..=102 {
        frame.hour = hour;
        if let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        {
            rows[100].political_power_raw = Some(i64::from(hour));
        }
        let wire = Arc::new(encoder.encode(frame.clone(), hour == 100).unwrap());
        let decoded = decoder.advance(&wire).unwrap();
        let changes = tracker.observe_updates(decoded, wire, format);
        let before = scan
            .country_keys
            .as_ref()
            .map_or(0, |keys| keys.row_visits());
        scan.observe_changes(decoded, format, &changes, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(scan.country_keys.as_ref().unwrap().row_visits(), before);
    }
}

fn observer_work(prepared: &Prepared) -> dashboard_base::prepared::observer_tests::Work {
    use dashboard_base::prepared::observer_tests::Work;
    let scan = prepared.scan.work;
    let current = prepared.work;
    Work {
        content_loads: scan.content_loads + current.content_loads,
        row_visits: scan.row_visits + current.row_visits,
        history_visits: scan.history_visits + current.history_visits,
        emitted: scan.emitted + current.emitted,
        interpreter_calls: scan.interpreter_calls + current.interpreter_calls,
        dictionary_interpretations: scan.dictionary_interpretations
            + current.dictionary_interpretations,
    }
}

fn phase_b_scaled_work(count: u32) {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        observer_tests::{assert_flat, scaled_work, unchanged_frames},
    };
    use replay_data::{rows::FuelDemandHour, rows::FuelDemandRead, rows::Recorded};
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    for refused in [false, true] {
        let histories = scaled_work(8, |history| {
            let hours = scaled_work(2, |hours| {
                let mut prepared = Prepared::default();
                let mut tracker = FrameChangeTracker::new();
                // Real earlier registrations and unnamed diagnostics are retained
                // after this roster disappears; the measured active roster is fixed.
                let mut old = dashboard_base::test_fixtures::scale_frame(
                    1,
                    fuel_slot(97),
                    (history * 2) as u32,
                );
                if let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *old.tables[TableId::Countries.index()]
                {
                    for (index, row) in rows.iter_mut().enumerate() {
                        row.tag = if index < history {
                            format!("old-{index}").into_bytes()
                        } else {
                            vec![0xff, index as u8]
                        };
                    }
                }
                prepared.observe_changes(&old, format, &tracker.observe(&old, format), &flag)?;
                let mut first =
                    dashboard_base::test_fixtures::scale_frame(1, fuel_slot(100), count);
                first.metrics.table_ns.fill(0);
                *first.tables[TableId::FuelDemandParts.index()] =
                    TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
                *first.tables[TableId::CountryFuel.index()] = if refused {
                    TableView::Unavailable {
                        reason: "synthetic fuel refusal".into(),
                    }
                } else {
                    TableView::Complete(TableRows::CountryFuel(
                        (0..count)
                            .map(|index| CountryFuelRow {
                                country: 100_000 + index,
                                current_raw: i64::from(index),
                                maximum_raw: i64::from(count),
                                country_instance: Recorded::Value(Some(9)),
                                demand: Recorded::Value(FuelDemandRead {
                                    hour: FuelDemandHour::Stamped(first.hour),
                                    country_instance: Some(9),
                                    ..Default::default()
                                }),
                                ..Default::default()
                            })
                            .collect(),
                    ))
                };
                prepared.observe_changes(
                    &first,
                    format,
                    &tracker.observe(&first, format),
                    &flag,
                )?;
                assert_eq!(prepared.scan.unnamed.len(), history);
                assert!(
                    prepared.metadata.unnamed.is_empty(),
                    "diagnostics materialize only at publication"
                );
                let warm = observer_work(&prepared);
                assert_eq!(
                    warm.content_loads, 0,
                    "fuel preparation has no content database binding"
                );
                let work =
                    unchanged_frames(warm, &first, hours as i32, format, |frame, changes| {
                        prepared.observe_changes(frame, format, changes, &flag)?;
                        Ok(observer_work(&prepared))
                    })?;
                assert_eq!(
                    (work.row_visits, work.history_visits, work.emitted),
                    (0, 0, 0)
                );
                assert_eq!(prepared.scan.active.len(), count as usize);
                assert!(prepared.metadata.unnamed.is_empty());
                // One changed fuel row uses the shared sample and visits the
                // changed table, but emits exactly its one changed value.
                let mut changed = first.clone();
                changed.hour += hours as i32 + 1;
                let mut last = first.clone();
                last.hour += hours as i32;
                tracker.observe(&last, format);
                if refused {
                    if let TableView::Complete(TableRows::Countries(rows)) =
                        &mut *changed.tables[TableId::Countries.index()]
                    {
                        rows[0].political_power_raw = Some(17);
                    }
                } else if let TableView::Complete(TableRows::CountryFuel(rows)) =
                    &mut *changed.tables[TableId::CountryFuel.index()]
                {
                    rows[0].current_raw += 17;
                }
                let before = observer_work(&prepared);
                prepared.observe_changes(
                    &changed,
                    format,
                    &tracker.observe(&changed, format),
                    &flag,
                )?;
                let after = observer_work(&prepared);
                assert_eq!(after.emitted - before.emitted, usize::from(!refused));
                assert_eq!(after.row_visits - before.row_visits, count as usize);
                assert_eq!(
                    after.history_visits - before.history_visits,
                    if refused { 0 } else { count as usize }
                );
                assert_eq!(after.content_loads, 0);
                Ok(work)
            })?;
            assert_flat(hours);
            Ok(hours[1])
        })
        .unwrap();
        assert_flat(histories);
    }
}

#[test]
fn phase_b_fuel_work_is_flat_over_unchanged_hours_and_retained_history() {
    phase_b_scaled_work(100);
}

#[test]
fn phase_b_fuel_work_is_flat_at_five_thousand_countries() {
    phase_b_scaled_work(5000);
}

#[test]
fn phase_b_link_changes_rebind_retained_operands_without_country_state_or_fuel_row_visits() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let first = fuel_slot(100);
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let base = fuel_frame(first, false, 9001, (11, 19, 23, 29));
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe_changes(&base, format, &tracker.observe(&base, format), &flag)
        .unwrap();
    let before = observer_work(&prepared);
    let owners = prepared.scan.ownership_rows;
    let mut changed = base.clone();
    changed.hour += 1;
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        rows.iter_mut()
            .find(|row| row.id == 11)
            .unwrap()
            .canonical_slot = 8;
    }
    let changes = tracker.observe(&changed, format);
    assert!(!changes.rows_changed[TableId::Countries.index()]);
    assert!(!changes.rows_changed[TableId::CountryFuel.index()]);
    assert!(!changes.slot_read[TableId::CountryFuel.index()]);
    prepared
        .observe_changes(&changed, format, &changes, &flag)
        .unwrap();
    assert_eq!(
        observer_work(&prepared).row_visits - before.row_visits,
        3,
        "only the changed link table visits native rows"
    );
    assert_eq!(prepared.scan.ownership_rows, owners);
    let expected = sample(&changed, None).unwrap();
    let fixture = Fixture::new();
    let mut writer = fixture.writer();
    prepared.flush(&mut writer, 1, format, &flag).unwrap();
    writer.commit().unwrap();
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    for ((tag, field), (_, value)) in expected {
        let key = reads.keys[&series::field_key(&tag, &field)];
        assert_eq!(
            reads
                .current
                .value_at(key, changed.hour)
                .unwrap()
                .map(|run| run.value),
            value
        );
    }
}

#[test]
fn phase_b_equal_amounts_reopen_after_unknowns_across_two_commits_resume_and_append() {
    use replay_data::{rows::FuelDemandHour, rows::FuelDemandRead, rows::Recorded};
    let first = fuel_slot(100);
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    for consumption in [false, true] {
        let mut base = fuel_frame(first, false, 9001, (11, 19, 23, 29));
        base.metrics.table_ns.fill(0);
        *base.tables[TableId::FuelDemandParts.index()] =
            TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
        if let TableView::Complete(TableRows::CountryFuel(rows)) =
            &mut *base.tables[TableId::CountryFuel.index()]
        {
            for row in rows {
                row.country_instance = Recorded::Value(Some(9));
                row.demand = Recorded::Value(FuelDemandRead {
                    hour: FuelDemandHour::Host,
                    country_instance: Some(9),
                    ..Default::default()
                });
            }
        }
        let frames: Vec<_> = (0..=2)
            .map(|offset| {
                let mut frame = base.clone();
                frame.hour += offset;
                if consumption {
                    frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
                    if offset == 1
                        && let TableView::Complete(TableRows::CountryFuel(rows)) =
                            &mut *frame.tables[TableId::CountryFuel.index()]
                    {
                        let Recorded::Value(pack) = &mut rows
                            .iter_mut()
                            .find(|row| row.country == 11)
                            .unwrap()
                            .demand
                        else {
                            unreachable!()
                        };
                        pack.army_reason = b"synthetic army refusal".to_vec();
                    }
                } else if offset == 1 {
                    *frame.tables[TableId::CountryFuel.index()] =
                        TableView::Complete(TableRows::CountryFuel(Vec::new()));
                }
                frame
            })
            .collect();
        let mut fixture = Fixture::new();
        let mut prepared = Prepared::default();
        for (index, frame) in frames.iter().enumerate() {
            prepared.observe(frame, format, &flag).unwrap();
            let mut writer = if index == 2 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            prepared.flush(&mut writer, 1, format, &flag).unwrap();
            writer.commit().unwrap();
            let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
                .unwrap()
                .unwrap();
            for field in if consumption {
                vec!["consumption-0"]
            } else {
                vec!["fuel", "capacity"]
            } {
                let key = reads.keys[&series::field_key("GER", field)];
                assert_eq!(
                    reads.current.value_at(key, frame.hour).unwrap().is_some(),
                    index != 1,
                    "{field} membership at publication {index}"
                );
            }
            compare(&fixture, &frames[..=index], &flag);
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
        let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
            .unwrap()
            .unwrap();
        for (field, amount) in if consumption {
            vec![("consumption-0", 0)]
        } else {
            vec![("fuel", 11), ("capacity", 19)]
        } {
            let key = reads.keys[&series::field_key("GER", field)];
            assert_eq!(
                reads.current.value_at(key, first).unwrap().unwrap().value,
                amount
            );
            assert!(reads.current.value_at(key, first + 1).unwrap().is_none());
            let returned = reads.current.value_at(key, first + 2).unwrap().unwrap();
            assert_eq!(returned.value, amount);
            assert!(
                returned.break_before,
                "equal values retain the intervening unknown interval"
            );
        }
    }
}

#[test]
fn phase_b_retained_diagnostics_count_identities_and_reads_after_two_commits_resume_and_append() {
    use replay_data::{rows::CountryLinkRow, rows::CountryRow};
    let first = fuel_slot(100);
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut base = fuel_frame(first, false, 9001, (11, 19, 23, 29));
    base.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *base.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 9999,
            canonical_slot: 9,
            tag: vec![0xff],
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *base.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 9999,
            canonical_slot: 9,
        });
        rows.sort_by_key(|row| row.id);
    }
    let frames: Vec<_> = (0..=49)
        .map(|offset| {
            let mut frame = base.clone();
            frame.hour += offset;
            frame
        })
        .collect();
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    for range in [0..5, 5..26, 26..frames.len()] {
        for frame in &frames[range.clone()] {
            prepared.observe(frame, format, &flag).unwrap();
        }
        let mut writer = if range.start == 26 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &frames[..range.end], &flag);
        let reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
            .unwrap()
            .unwrap();
        assert_eq!(reads.metadata.unnamed, BTreeSet::from([9999]));
        if range.end >= 26 {
            assert_eq!(
                reads.metadata.countries["GER"].left_out,
                [0, 0, 1],
                "one unproved supplying day is not a held-hour warning count"
            );
        }
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
}

#[test]
fn phase_b_shared_views_keep_fresh_reads_gaps_and_dormant_ownership_after_resume_and_append() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay_data::rows::Recorded;
    let first = fuel_slot(100);
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut base = fuel_frame(first, false, 9001, (11, 19, 23, 29));
    base.metrics.table_ns.fill(0);
    if let TableView::Complete(TableRows::CountryFuel(rows)) =
        &mut *base.tables[TableId::CountryFuel.index()]
    {
        for row in rows {
            row.country_instance = Recorded::Value(Some(9));
        }
    }
    let frames: Vec<_> = (0..=72)
        .filter(|offset| *offset != 36)
        .map(|offset| {
            let mut frame = base.clone();
            frame.hour += offset;
            // Fresh equal acquisitions share their rows and still supply new dates.
            if offset == 5 || offset == 47 {
                frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
            }
            if ((8..=10).contains(&offset) || (25..=30).contains(&offset))
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.retain(|row| row.tag != b"GER");
            }
            if offset == 10 || offset == 29 {
                *frame.tables[TableId::States.index()] = TableView::Unavailable {
                    reason: "synthetic ownership refusal".into(),
                };
            }
            if offset == 12 {
                *frame.tables[TableId::CountryFuel.index()] = TableView::Unavailable {
                    reason: "synthetic fuel refusal".into(),
                };
            }
            if offset == 17
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows[0].political_power_raw = Some(17);
            }
            if offset >= 49
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.tag == b"GER").unwrap().tag = b"RENAMED".to_vec();
            }
            frame
        })
        .collect();
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    for range in [0..8, 8..32, 32..frames.len()] {
        for frame in &frames[range.clone()] {
            let changes = tracker.observe(frame, format);
            let before = observer_work(&prepared);
            let owners = prepared.scan.ownership_rows;
            prepared
                .observe_changes(frame, format, &changes, &flag)
                .unwrap();
            if frame.hour == first + 5 {
                assert!(!changes.rows_changed[TableId::CountryFuel.index()]);
                assert!(changes.slot_read[TableId::CountryFuel.index()]);
                assert_eq!(
                    prepared.scan.countries["GER"]
                        .result
                        .reads
                        .last()
                        .unwrap()
                        .first_hour,
                    frame.hour
                );
                assert_eq!(
                    prepared.scan.ownership_rows, owners,
                    "fresh equal fuel does not rejoin unchanged ownership"
                );
            }
            if frame.hour == first + 17 {
                let after = observer_work(&prepared);
                assert_eq!(
                    after.row_visits - before.row_visits,
                    2,
                    "balance-only countries use the shared key projection"
                );
                assert_eq!(after.emitted, before.emitted);
                assert_eq!(prepared.scan.ownership_rows, owners);
            }
        }
        let mut writer = if range.start == 32 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &frames[..range.end], &flag);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        tracker.invalidate();
    }
    let (replay, _) = reference(&frames);
    let expected = scan_branch(&replay, 1, &flag).unwrap();
    let day = expected.countries["GER"]
        .reads
        .iter()
        .find(|read| read.first_hour == first + 48)
        .unwrap();
    assert_eq!(
        day.left_out,
        Some(DayLeftOut::NotUpdated),
        "the dormant tag's known absent hours outweigh missing ownership"
    );
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let actor = actor_binding("11", 9);
    let actor_key = reads.keys[&series::field_key(&actor, "actorSegment")];
    assert!(
        reads
            .current
            .value_at(actor_key, first + 8)
            .unwrap()
            .is_none()
    );
    assert!(
        reads
            .current
            .value_at(actor_key, first + 36)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        reads
            .consumption("GER", first + 6)
            .unwrap()
            .unwrap()
            .first_hour,
        first + 5
    );
}

#[test]
fn progressive_fuel_sampler_matches_observer_on_every_key_at_5000_countries() {
    let mut prepared = Prepared::default();
    let flag = AtomicBool::new(false);
    for hour in 0..=1 {
        let mut frame = dashboard_base::test_fixtures::scale_frame(1, hour, 5000);
        frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
        *frame.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
            (0..5000)
                .map(|index| CountryFuelRow {
                    country: 100_000 + index,
                    current_raw: i64::from(index) - i64::from(hour),
                    maximum_raw: i64::from(index) * 2,
                    ..Default::default()
                })
                .collect(),
        ));
        let samples = sample(&frame, None).unwrap();
        assert_eq!(samples.len(), 10_000);
        prepared
            .observe(&frame, replay_data::format::FormatVersion::CURRENT, &flag)
            .unwrap();
        let identities: BTreeMap<_, _> = frame
            .complete::<tables::Countries>()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    String::from_utf8(row.tag.clone()).unwrap(),
                    (row.canonical_slot, row.id),
                )
            })
            .collect();
        for ((tag, field), (identity, value)) in samples {
            assert_eq!(identity, Some(identities[&tag]));
            let key = prepared.keys[&series::field_key(&tag, &field)];
            assert_eq!(
                prepared.current.state(key).unwrap().last.unwrap().value,
                value.unwrap()
            );
        }
        let requested = [("tail absent".into(), "fuel".into())];
        assert_eq!(
            sample(&frame, Some(&requested)).unwrap()[&requested[0]],
            (None, None)
        );
    }
}

fn reference(frames: &[replay_frame::frame::DecodedFrame]) -> (Arc<Replay>, Source) {
    let replay =
        Arc::new(Replay::from_records(CAMPAIGN.into(), &[records(1, frames.to_vec())]).unwrap());
    let source = Source::from_replay(
        dashboard_base::source::SourceKey {
            source_id: "fuel-prepared".into(),
            revision: "1".into(),
        },
        "memory.battleplan".into(),
        Arc::clone(&replay),
        &AtomicBool::new(false),
    )
    .unwrap();
    (replay, source)
}
fn compare(
    fixture: &Fixture,
    frames: &[replay_frame::frame::DecodedFrame],
    cancelled: &AtomicBool,
) {
    let (replay, source) = reference(frames);
    let expected = scan_branch(&replay, 1, cancelled).unwrap();
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, cancelled)
        .unwrap()
        .unwrap();
    let oracle = Fixture::new();
    let mut batch = series::Batch::default();
    let mut states = BTreeMap::<u64, series::KeyState>::new();
    let mut keys = BTreeMap::new();
    batch.through(frames[0].hour);
    batch.through(frames.last().unwrap().hour);
    for (tag, country) in &expected.countries {
        for read in &country.reads {
            for (measure, value) in day_values(read) {
                let key = key(&mut keys, tag, &measure);
                batch.push(
                    key,
                    states.entry(key).or_default(),
                    Run::new(
                        read.first_hour,
                        if measure.starts_with("strip") {
                            read.first_hour
                        } else {
                            read.last_hour
                        },
                        value,
                        read.starts_segment,
                    ),
                );
            }
        }
    }
    let mut writer = oracle.writer();
    batch.write(&mut writer, "oracle", 0, cancelled).unwrap();
    writer.commit().unwrap();
    let mut oracle_reader =
        Reader::new(&oracle.store, &oracle.key, "oracle".into(), cancelled).unwrap();
    let bounds = (frames[0].hour, frames.last().unwrap().hour);
    for (tag, country) in &expected.countries {
        for frame in frames {
            let expected = country
                .reads
                .iter()
                .find(|read| read.first_hour <= frame.hour && read.last_hour >= frame.hour)
                .map(FuelRead::from);
            assert_eq!(
                reads.fuel(tag, frame.hour).unwrap(),
                expected,
                "{tag} at {}",
                frame.hour
            );
            let expected = country
                .consumption
                .iter()
                .find(|read| read.first_hour <= frame.hour && read.last_hour >= frame.hour)
                .map(demand::FuelConsumptionRead::from);
            dashboard_base::prepared::observer_tests::assert_reply_parity(
                &reads.consumption(tag, frame.hour).unwrap(),
                &expected,
            )
            .unwrap();
        }
        let expected = super::super::reply(
            &source,
            &CountryParams {
                branch_id: "1".into(),
                tag: tag.clone(),
            },
            &LoadContext { content: None },
            &expected,
            country,
        )
        .unwrap();
        let actual = presentation(
            &source,
            &LoadContext { content: None },
            1,
            tag,
            &reads.metadata,
        )
        .unwrap();
        assert_eq!(actual.warnings, expected.warnings);
        let window = reads
            .fuel_window(tag, frames[0].hour, frames.last().unwrap().hour, cancelled)
            .unwrap();
        assert_eq!(window, expected.reads);
        for measure in [
            "stripGain",
            "stripBurn",
            "consumer-0",
            "gain-0",
            "other",
            "conversion",
        ] {
            let tail = tail_runs(measure, tag, &reads.metadata).unwrap();
            let actual_key = reads
                .keys
                .get(&series::field_key(tag, measure))
                .copied()
                .unwrap_or(u64::MAX);
            let expected_key = keys
                .get(&series::field_key(tag, measure))
                .copied()
                .unwrap_or(u64::MAX);
            for view in [
                (f64::from(bounds.0), f64::from(bounds.1), 1),
                (f64::from(bounds.0), f64::from(bounds.1), 200),
                (f64::from(bounds.1 - 2), f64::from(bounds.1), 100),
            ] {
                assert_eq!(
                    reads
                        .days
                        .summary_with_tail(actual_key, bounds, view, 0, &tail, None)
                        .unwrap(),
                    oracle_reader
                        .summary(expected_key, bounds, view, 0)
                        .unwrap(),
                    "{tag} {measure} at {view:?}"
                );
            }
        }
    }
}
#[test]
fn prepared_append_suppresses_refresh_across_commit_after_two_checkpoints_and_resume() {
    let slot = fuel_slot(1000);
    let frames: Vec<_> = (slot - 25..=slot + 25)
        .map(|hour| fuel_frame(hour, hour == slot - 1, 9001, (11, 19, 23, 29)))
        .collect();
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let format = reference(&frames).0.format;
    let mut prepared = Prepared::default();
    for range in [0..10, 10..25] {
        for frame in &frames[range.clone()] {
            prepared.observe(frame, format, &flag).unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &frames[..range.end], &flag);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    assert!(reads.fuel("GER", slot - 1).unwrap().unwrap().day.is_some());
    drop(reads);
    // The slot is appended after the commit that accepted the refresh.
    for frame in &frames[25..] {
        prepared.observe(frame, format, &flag).unwrap();
    }
    let mut writer = fixture.append_writer();
    prepared.flush(&mut writer, 1, format, &flag).unwrap();
    writer.commit().unwrap();
    compare(&fixture, &frames, &flag);
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    let refresh = reads.fuel("GER", slot - 1).unwrap().unwrap();
    assert_eq!(refresh.left_out, Some(DayLeftOut::Overlaps));
    assert!(refresh.day.is_none());
    assert!(reads.fuel("GER", slot).unwrap().unwrap().day.is_some());
    assert_eq!(prepared.scan.decoded_frames, frames.len());
    assert_eq!(prepared.scan.ownership_rows, frames.len() * 2);
    for country in prepared.scan.countries.values() {
        assert!(
            country
                .result
                .reads
                .iter()
                .filter(|read| read.first_hour > prepared.metadata.settled_until.unwrap())
                .all(|read| read.first_hour > frames.last().unwrap().hour - 24)
        );
    }
}

#[test]
fn prepared_gaps_identity_and_aliases_match_reads_after_reopen() {
    let slot = fuel_slot(1000);
    let frames: Vec<_> = (slot..=slot + 96)
        .filter(|hour| *hour != slot + 37)
        .map(|hour| {
            let mut frame = fuel_frame(
                hour,
                hour % 5 == 0,
                if hour < slot + 49 { 9001 } else { 9002 },
                (hour.into(), 37, 41, 43),
            );
            if hour >= slot + 11
                && hour < slot + 49
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.tag == b"GER").unwrap().tag = b"RENAMED".to_vec();
            }
            frame
        })
        .collect();
    let format = reference(&frames).0.format;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for chunk in frames.chunks(24) {
        for frame in chunk {
            prepared.observe(frame, format, &flag).unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    compare(&fixture, &frames, &flag);
}

#[test]
fn a_gap_settles_country_dates_in_one_shared_batch_after_resume() {
    use super::super::tests::award_row;
    let first = fuel_slot(40);
    let frames: Vec<_> = [first, first + 1, first + 25]
        .into_iter()
        .map(|hour| {
            let mut frame = fuel_frame(hour, hour == first + 1, 9001, (1, 2, 3, 4));
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                for country in countries {
                    country.tag = if country.id == 9001 {
                        b"Z".to_vec()
                    } else {
                        b"A".to_vec()
                    };
                }
            }
            let mut row = award_row(if hour == first { first } else { first + 1 });
            if let replay_data::rows::Recorded::Value(day) = &mut row.award_day {
                day.continuity_start_hour = Some(first - (DAY_HOURS - 1));
            }
            if hour != first {
                row.country = 21;
            }
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![row]));
            frame
        })
        .collect();
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let format = reference(&frames).0.format;
    let mut prepared = Prepared::default();
    for (index, frame) in frames.iter().enumerate() {
        prepared.observe(frame, format, &flag).unwrap();
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        compare(&fixture, &frames[..=index], &flag);
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    assert!(reads.fuel("Z", first).unwrap().unwrap().day.is_some());
    assert!(reads.fuel("A", first + 1).unwrap().unwrap().day.is_some());
}

#[test]
fn five_thousand_countries_share_each_frame_and_keep_bounded_continuations() {
    use dashboard_base::test_fixtures::scale_frame;
    use replay_data::{
        rows::CountryFuelRow, rows::FuelDemandHour, rows::FuelDemandRead, rows::FuelDemandUnit,
        rows::Recorded,
    };
    let count = 5000_u32;
    let slot = fuel_slot(1000);
    let format = replay_data::format::FormatVersion::CURRENT;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for hour in slot..=slot + 48 {
        let mut frame = scale_frame(1, hour, count);
        *frame.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
            (0..count)
                .map(|index| CountryFuelRow {
                    country: 100_000 + index,
                    current_raw: i64::from(index),
                    maximum_raw: i64::from(count),
                    country_instance: Recorded::Value(Some(9)),
                    demand: Recorded::Value(FuelDemandRead {
                        hour: FuelDemandHour::Stamped(hour),
                        country_instance: Some(9),
                        divisions: vec![FuelDemandUnit {
                            reference: u64::from(index) << 32 | 29,
                            rate_raw: Some(i64::from(index)),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .collect(),
        ));
        prepared.observe(&frame, format, &flag).unwrap();
        if hour == slot + 24 || hour == slot + 48 {
            let mut writer = fixture.writer();
            prepared.flush(&mut writer, 1, format, &flag).unwrap();
            writer.commit().unwrap();
            prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
        }
    }
    let mut reads = Reads::open(&fixture.store, &fixture.key, 1, &flag)
        .unwrap()
        .unwrap();
    for index in 0..count {
        let tag = String::from_utf8(dashboard_base::test_fixtures::scale_tag(index)).unwrap();
        assert_eq!(
            reads.fuel(&tag, slot + 48).unwrap().unwrap().fuel_raw,
            index.to_string()
        );
        assert_eq!(
            reads
                .consumption(&tag, slot + 48)
                .unwrap()
                .unwrap()
                .known_daily_raw[0],
            (i64::from(index) * 24).to_string()
        );
    }
    assert_eq!(prepared.scan.decoded_frames, 49);
    assert_eq!(prepared.scan.ownership_rows, 49 * count as usize);
    assert_eq!(prepared.scan.countries.len(), count as usize);
    assert!(prepared.scan.countries.values().all(|country|country.result.reads.len()<=2 && country.result.consumption.len()==1));
    eprintln!(
        "fuel 5000-country continuation bytes: {}",
        serde_json::to_vec(&prepared).unwrap().len()
    );
}

struct FileFixture {
    root: std::path::PathBuf,
    path: std::path::PathBuf,
    stores: Vec<Arc<store::Store>>,
}
impl FileFixture {
    fn new(replay: &Replay) -> Self {
        let root = std::env::temp_dir().join(format!("fuel-details-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("campaign.battleplan");
        std::fs::write(&path, replay.bytes()).unwrap();
        Self {
            root,
            path,
            stores: Vec::new(),
        }
    }
    fn source(&mut self) -> Source {
        let source = Source::open(
            dashboard_base::source::SourceKey {
                source_id: "fuel-file".into(),
                revision: dashboard_core::protocol::replay_revision(&self.path).unwrap(),
            },
            self.path.clone(),
            &AtomicBool::new(false),
        )
        .unwrap();
        self.stores
            .push(dashboard_base::prepared::files(&source, None).unwrap().0);
        source
    }
}
impl Drop for FileFixture {
    fn drop(&mut self) {
        for store in &self.stores {
            store.clear().unwrap();
        }
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[test]
fn detail_products_use_summaries_after_two_commits_resume_and_file_append_without_branch_scan() {
    use super::super::tests::award_row;
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::FuelDemandHour, rows::FuelDemandRead,
        rows::FuelDemandUnit, rows::Recorded,
    };
    let first = fuel_slot(30);
    let frames: Vec<_> = (first..=first + 850)
        .filter(|hour| ![first + 750, first + 751].contains(hour))
        .map(|hour| {
            let mut frame = fuel_frame(hour, (hour - first) % 24 == 0, 9001, (1, 2, 3, 4));
            let instance = if hour < first + 800 { 9 } else { 10 };
            let mut own = award_row(hour);
            own.country_instance = Recorded::Value(Some(instance));
            if let Recorded::Value(day) = &mut own.award_day {
                day.country_instance = instance;
                day.continuity_start_hour = Some(if hour < first + 780 {
                    first - 23
                } else {
                    first + 780
                });
            }
            own.demand = Recorded::Value(FuelDemandRead {
                hour: FuelDemandHour::Stamped(hour),
                country_instance: Some(instance),
                divisions: vec![FuelDemandUnit {
                    reference: 7 << 32 | 29,
                    owners: Some([11; 3]),
                    rate_raw: Some(i64::from(hour) * 100),
                    name: Some(Vec::new()),
                    ..Default::default()
                }],
                ..Default::default()
            });
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![own]));
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                countries
                    .iter_mut()
                    .find(|country| country.id == 9001)
                    .unwrap()
                    .tag = if hour < first + 24 {
                    b"X".to_vec()
                } else {
                    b"Y".to_vec()
                };
                if hour >= first + 24 {
                    countries.push(CountryRow {
                        id: 9002,
                        canonical_slot: 9,
                        tag: b"X".to_vec(),
                        ..Default::default()
                    });
                    if let TableView::Complete(TableRows::CountryLinks(links)) =
                        &mut *frame.tables[TableId::CountryLinks.index()]
                    {
                        links.push(CountryLinkRow {
                            id: 9002,
                            canonical_slot: 9,
                        });
                        links.sort_by_key(|link| link.id);
                    }
                }
            }
            frame
        })
        .collect();
    let prefix_len = 701;
    let prefix = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames[..prefix_len].to_vec())],
    )
    .unwrap();
    let mut fixture = FileFixture::new(&prefix);
    let source = fixture.source();
    let (store, previous_key) = dashboard_base::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for (index, chunk) in frames[..prefix_len].chunks(351).enumerate() {
        for frame in chunk {
            prepared.observe(frame, prefix.format, &flag).unwrap();
        }
        let mut writer = if index == 0 {
            store.replace(previous_key.clone()).unwrap()
        } else {
            store.extend(&previous_key).unwrap()
        };
        prepared
            .flush(&mut writer, 1, prefix.format, &flag)
            .unwrap();
        dashboard_base::prepared::coverage::publish_coverage(
            &mut writer,
            1,
            first,
            chunk.last().unwrap().hour,
            chunk.last().unwrap().hour == frames[prefix_len - 1].hour,
        )
        .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let (_, oracle) = reference(&frames[..prefix_len]);
    assert_eq!(
        detail::oracle::replies(&source, first, &[1, 2]),
        detail::oracle::replies(&oracle, first, &[1, 2])
    );
    let navigation = query(
        &source,
        None,
        "series-next-change",
        serde_json::json!({
            "branchId": "1", "family": { "kind": "country-fuel" }, "tags": ["X"],
            "key": "Home", "firstHour": first, "lastHour": first + 700,
            "within": { "X": [{ "firstHour": first + 5, "lastHour": first + 9 }] }
        }),
        &store,
        &previous_key,
        &flag,
    )
    .unwrap();
    assert_eq!(
        navigation["hour"].as_f64(),
        Some(f64::from(first + 5)),
        "fuel navigation keeps the clipped membership edge"
    );
    assert!(source.state::<Cache>().lock().unwrap().branches.is_empty());
    drop(source);
    let appended = prefix
        .append(&[dashboard_base::test_fixtures::continuation_records(
            &prefix,
            1,
            frames[prefix_len..].to_vec(),
        )])
        .unwrap();
    let pending = fixture.root.join("append");
    std::fs::write(&pending, appended.bytes()).unwrap();
    replay::publication::replace_file(&pending, &fixture.path).unwrap();
    let source = fixture.source();
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    for frame in &frames[prefix_len..] {
        prepared.observe(frame, appended.format, &flag).unwrap();
    }
    let mut writer = store.append(&previous_key, key).unwrap();
    prepared
        .flush(&mut writer, 1, appended.format, &flag)
        .unwrap();
    dashboard_base::prepared::coverage::publish_coverage(
        &mut writer,
        1,
        first,
        frames.last().unwrap().hour,
        true,
    )
    .unwrap();
    writer.commit().unwrap();
    let (_, oracle) = reference(&frames);
    assert_eq!(
        detail::oracle::replies(&source, first, &[1, 2]),
        detail::oracle::replies(&oracle, first, &[1, 2])
    );
    assert!(source.state::<Cache>().lock().unwrap().branches.is_empty());
    assert_eq!(prepared.scan.decoded_frames, frames.len());
}

#[test]
fn a_conflicting_alias_outside_the_requested_window_keeps_both_histories_refused() {
    use super::super::tests::award_row;
    use replay_data::{rows::CountryLinkRow, rows::CountryRow, rows::Recorded};
    let first = fuel_slot(30);
    let mut frames: Vec<_> = [first, first + detail::DETAIL_HOURS]
        .into_iter()
        .map(|hour| {
            let mut frame = fuel_frame(hour, true, 9001, (1, 2, 3, 4));
            let mut row = award_row(hour);
            if let Recorded::Value(day) = &mut row.award_day {
                day.continuity_start_hour = Some(first - (DAY_HOURS - 1));
            }
            row.demand = Recorded::Value(replay_data::rows::FuelDemandRead {
                hour: replay_data::rows::FuelDemandHour::Stamped(hour),
                country_instance: Some(9),
                ..Default::default()
            });
            *frame.tables[TableId::CountryFuel.index()] =
                TableView::Complete(TableRows::CountryFuel(vec![row]));
            if let TableView::Complete(TableRows::Countries(countries)) =
                &mut *frame.tables[TableId::Countries.index()]
            {
                countries
                    .iter_mut()
                    .find(|country| country.id == 9001)
                    .unwrap()
                    .tag = b"X".to_vec();
            }
            frame
        })
        .collect();
    let clean_frames = frames.clone();
    // Defensive observer coverage for a census the encoder refuses.
    if let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frames[0].tables[TableId::Countries.index()]
    {
        countries.push(CountryRow {
            id: 9999,
            canonical_slot: 7,
            tag: b"Z".to_vec(),
            ..Default::default()
        });
    }
    if let TableView::Complete(TableRows::CountryLinks(links)) =
        &mut *frames[0].tables[TableId::CountryLinks.index()]
    {
        links.push(CountryLinkRow {
            id: 9999,
            canonical_slot: 7,
        });
        links.sort_by_key(|link| link.id);
    }
    let (replay, _) = reference(&clean_frames);
    let mut fixture = FileFixture::new(&replay);
    let source = fixture.source();
    let (store, key) = dashboard_base::prepared::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for (index, frame) in frames.iter().enumerate() {
        prepared.observe(frame, replay.format, &flag).unwrap();
        let mut writer = if index == 0 {
            store.replace(key.clone()).unwrap()
        } else {
            store.extend(&key).unwrap()
        };
        prepared
            .flush(&mut writer, 1, replay.format, &flag)
            .unwrap();
        writer.commit().unwrap();
        prepared = serde_json::from_slice(&serde_json::to_vec(&prepared).unwrap()).unwrap();
    }
    let context = LoadContext { content: None };
    let basis = FuelAwardsParams {
        branch_id: "1".into(),
        tag: "X".into(),
        hour: first,
        country: "11".into(),
        country_instance: 9,
    };
    let awards = CountryFuelAwards::load(&source, &basis, &context, &flag).unwrap();
    let params = detail::FuelAwardHistoryParams {
        branch_id: "1".into(),
        tag: "X".into(),
        hour: first,
        country: "11".into(),
        country_instance: 9,
        window: 2,
        recipient_key: awards.awards.recipients[0].key.clone(),
    };
    let actual =
        detail::CountryFuelAwardHistory::load(&source, &params, &context, &flag).unwrap_err();
    assert!(actual.to_string().contains("Conflicting aliases"));
    assert!(
        actor_consumption(&source, &basis, 2, &context, &flag)
            .unwrap()
            .is_empty()
    );
    assert!(source.state::<Cache>().lock().unwrap().branches.is_empty());
}

/// Stepping each span of hours at which nothing fuel reads changed and no
/// fuel read is dated at once ([`Prepared::advance_quiet`]) publishes and
/// saves what observing each of those hours does: across four days of daily
/// reads, with an off-slot read left out by the slot read after it, a state
/// ceded inside a day and a flush inside a span, the days settled, left out
/// and retired inside the spans, and the proofs and open reads carried, are
/// the same. Fewer days are settled than hours observed.
#[test]
fn quiet_spans_settle_and_carry_fuel_as_observing_every_hour() {
    use dashboard_base::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
    use replay_data::rows::StateRow;
    use replay_frame::frame::DecodedFrame;
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let slot = fuel_slot(1000);
    // Germany's fuel grows at each daily slot.
    let fuel = |hour: i32| {
        let day = i64::from((hour - slot).div_euclid(24));
        (
            655_360_000 + 65_536 * day,
            655_360_000 * 2,
            1_000_000 + day,
            65_536,
        )
    };
    let (first, last) = (slot - 5, slot + 100);
    // A read off the clock (another table of the slot timed), Germany's
    // state ceded to Italy, and a flush, each inside a day.
    let (off_slot, ceded, flushed) = (slot + 30, slot + 40, slot + 50);
    // Each hour's frame shares every table the hour before holds unchanged,
    // as decoded frames do, so only changed tables are changed facts.
    let mut frames: Vec<DecodedFrame> = Vec::new();
    for hour in first..=last {
        let fresh = fuel_frame(hour, hour == off_slot, 9001, fuel(hour));
        let frame = match frames.last() {
            None => fresh,
            Some(before) => {
                let mut frame = before.clone();
                frame.hour = hour;
                frame.metrics = fresh.metrics.clone();
                if fuel(hour) != fuel(hour - 1) {
                    frame.tables[TableId::CountryFuel.index()] =
                        fresh.tables[TableId::CountryFuel.index()].clone();
                }
                if hour == ceded {
                    let state = |id, owner| StateRow {
                        id,
                        owner,
                        controller: owner,
                        ..Default::default()
                    };
                    *frame.tables[TableId::States.index()] =
                        TableView::Complete(TableRows::States(vec![state(1, 21), state(2, 21)]));
                }
                frame
            }
        };
        frames.push(frame);
    }
    let mut tracker = FrameChangeTracker::new();
    let changes: Vec<FrameChanges> = frames
        .iter()
        .map(|frame| tracker.observe(frame, format))
        .collect();
    // An hour fuel observes: one whose read is dated or whose watched tables
    // changed (every one, the first).
    let watched = [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::States,
        TableId::CountryFuel,
        TableId::FuelDemandParts,
    ];
    let kept = |index: usize| {
        let changes = &changes[index];
        index == 0
            || changes.slot_read[TableId::CountryFuel.index()]
            || watched
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    };
    let publish = |prepared: &mut Prepared, fixture: &Fixture| {
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
        serde_json::to_value(&*prepared).unwrap()
    };
    // Every hour observed.
    let (hourly_fixture, mut hourly) = (
        Fixture::new(),
        Prepared {
            hourly_settlement: true,
            ..Default::default()
        },
    );
    let mut hourly_states = Vec::new();
    for (frame, changes) in frames.iter().zip(&changes) {
        hourly
            .observe_changes(frame, format, changes, &flag)
            .unwrap();
        if frame.hour == flushed || frame.hour == last {
            hourly_states.push(publish(&mut hourly, &hourly_fixture));
        }
    }
    // Kept hours observed, the spans between stepped at once, split where
    // the flush is.
    let (fixture, mut stepped) = (Fixture::new(), Prepared::default());
    let mut states = Vec::new();
    let mut observed = 0;
    let mut spans = 0;
    let mut index = 0;
    while index < frames.len() {
        let hour = frames[index].hour;
        if kept(index) {
            stepped
                .observe_changes(&frames[index], format, &changes[index], &flag)
                .unwrap();
            observed += 1;
            index += 1;
        } else {
            let mut end = index;
            while end + 1 < frames.len() && !kept(end + 1) && frames[end].hour != flushed {
                end += 1;
            }
            assert!(
                stepped
                    .advance_quiet(hour, frames[end].hour, format)
                    .unwrap(),
                "the hours from {hour} step"
            );
            spans += 1;
            index = end + 1;
        }
        if frames[index - 1].hour == flushed || frames[index - 1].hour == last {
            states.push(publish(&mut stepped, &fixture));
        }
    }
    assert_eq!(states, hourly_states, "saved states");
    assert_eq!(fixture.payloads(), hourly_fixture.payloads(), "pages");
    assert!(
        hourly_fixture
            .payloads()
            .keys()
            .any(|(name, _)| name == "fuel-days-1"),
        "days were settled"
    );
    // The first hour, the five slot reads, the off-slot read and the
    // cession; nine spans between them and the flush.
    assert_eq!(observed, 8, "hours observed");
    assert_eq!(spans, 9, "spans stepped");
    // An off-slot read overlapping the next slot read is left out.
    let left_out = &stepped.metadata.countries["GER"].left_out;
    assert!(left_out[0] > 0, "an overlapping day left out: {left_out:?}");
}

fn check_deferred_fuel_settlement_work(count: u32) {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker, series::tests::assert_payloads_equal,
    };
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut frame = dashboard_base::test_fixtures::scale_frame(1, fuel_slot(1000), count);
    frame.metrics.table_ns.fill(0);
    frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
    *frame.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
        (0..count)
            .map(|index| CountryFuelRow {
                country: 100_000 + index,
                current_raw: i64::from(index),
                maximum_raw: 10_000,
                ring_gain_raw: 19,
                ring_consumed_raw: 7,
                ..Default::default()
            })
            .collect(),
    ));
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut states = [
        Prepared::default(),
        Prepared {
            hourly_settlement: true,
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    // Complete preceding ownership evidence makes the first supplying day
    // eligible, so the post-flush continuation really has day rows to charge.
    let slot = frame.hour;
    for hour in slot - DAY_HOURS..slot {
        frame.hour = hour;
        frame.metrics.table_ns.fill(0);
        for (state, tracker) in states.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe(&frame, format);
            state
                .observe_changes(&frame, format, &changes, &flag)
                .unwrap();
        }
    }
    frame.hour = slot;
    frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
    for offset in 0..=25 {
        if offset > 0 {
            frame.hour += 1;
            frame.metrics.table_ns.fill(0);
        }
        let mut visits = [0; 2];
        for (index, (state, tracker)) in states.iter_mut().zip(&mut trackers).enumerate() {
            let changes = tracker.observe(&frame, format);
            let before = state.settlement_visits;
            state
                .observe_changes(&frame, format, &changes, &flag)
                .unwrap();
            visits[index] = state.settlement_visits - before;
        }
        assert_eq!(
            states[0].buffered_bytes(),
            states[1].buffered_bytes(),
            "fuel publication budget at offset {offset}"
        );
        if offset == 1 {
            assert_eq!(
                visits,
                [0, count as usize],
                "quiet hours decide no fuel reads"
            );
        }
        if offset == 23 {
            for (state, fixture) in states.iter_mut().zip(&fixtures) {
                let mut writer = fixture.writer();
                state.flush(&mut writer, 1, format, &flag).unwrap();
                writer.commit().unwrap();
            }
        }
    }
    for (state, fixture) in states.iter_mut().zip(&fixtures) {
        let mut writer = fixture.writer();
        state.flush(&mut writer, 1, format, &flag).unwrap();
        writer.commit().unwrap();
    }
    assert_payloads_equal(
        &fixtures[0].payloads(),
        &fixtures[1].payloads(),
        "deferred fuel settlement",
    );
    assert_eq!(
        serde_json::to_vec(&states[0]).unwrap(),
        serde_json::to_vec(&states[1]).unwrap()
    );
}

#[test]
fn quiet_fuel_settlement_does_no_per_country_work_between_decisions() {
    check_deferred_fuel_settlement_work(64);
}

#[test]
fn quiet_fuel_settlement_carries_5000_countries_without_hourly_decisions() {
    check_deferred_fuel_settlement_work(5_000);
}

#[test]
fn deferred_fuel_settlement_matches_hourly_bytes_through_gaps_restore_and_append() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker, series::tests::assert_payloads_equal,
    };
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let slot = fuel_slot(1000);
    let frames: Vec<_> = (slot - 25..=slot + 80)
        .filter(|hour| ![slot + 36, slot + 37].contains(hour))
        .map(|hour| {
            let mut frame = fuel_frame(
                hour,
                [slot + 4, slot + 30].contains(&hour),
                9001,
                (i64::from(hour), 10_000, 19, 7),
            );
            if hour == slot + 50 {
                *frame.tables[TableId::CountryFuel.index()] = TableView::Unavailable {
                    reason: "synthetic unavailable fuel".into(),
                };
            }
            if hour >= slot + 60
                && let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *frame.tables[TableId::Countries.index()]
            {
                rows.iter_mut().find(|row| row.id == 9001).unwrap().tag =
                    b"changed fuel alias".to_vec();
            }
            frame
        })
        .collect();
    let split = frames.partition_point(|frame| frame.hour <= slot + 27);
    let first =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..split].to_vec())]).unwrap();
    let replay = first
        .append(&[dashboard_base::test_fixtures::continuation_records(
            &first,
            1,
            frames[split..].to_vec(),
        )])
        .unwrap();
    let mut fixtures = [Fixture::new(), Fixture::new()];
    let mut states = [
        Prepared::default(),
        Prepared {
            hourly_settlement: true,
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    for block in &replay.index {
        replay
            .read_block_alone(
                block,
                &TableId::ALL,
                |raw, frame| {
                    for (state, tracker) in states.iter_mut().zip(&mut trackers) {
                        let changes = tracker.observe_updates(frame, Arc::new(raw.clone()), format);
                        state
                            .observe_changes(frame, format, &changes, &flag)
                            .unwrap();
                    }
                    assert_eq!(
                        states[0].buffered_bytes(),
                        states[1].buffered_bytes(),
                        "fuel hourly budget at {}",
                        frame.hour
                    );
                    if ![
                        slot - 20,
                        slot + 10,
                        slot + 27,
                        slot + 49,
                        slot + 65,
                        slot + 80,
                    ]
                    .contains(&frame.hour)
                    {
                        return Ok(());
                    }
                    for (state, fixture) in states.iter_mut().zip(&mut fixtures) {
                        let mut writer = if frame.hour > slot + 27 {
                            fixture.append_writer()
                        } else {
                            fixture.writer()
                        };
                        state.flush(&mut writer, 1, format, &flag).unwrap();
                        writer.commit().unwrap();
                    }
                    assert_payloads_equal(
                        &fixtures[0].payloads(),
                        &fixtures[1].payloads(),
                        "fuel decision deadlines",
                    );
                    assert_eq!(
                        serde_json::to_vec(&states[0]).unwrap(),
                        serde_json::to_vec(&states[1]).unwrap(),
                        "fuel continuation at {}",
                        frame.hour
                    );
                    if [slot + 27, slot + 65].contains(&frame.hour) {
                        for (index, (state, tracker)) in
                            states.iter_mut().zip(&mut trackers).enumerate()
                        {
                            *state = serde_json::from_slice(&serde_json::to_vec(&*state).unwrap())
                                .unwrap();
                            state.hourly_settlement = index == 1;
                            tracker.invalidate();
                        }
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
}

#[test]
fn deferred_fuel_uses_hourly_publication_budget_after_flush() {
    use dashboard_base::prepared::{
        frame_changes::FrameChangeTracker,
        pages::{BATCH_BYTES, publication_due},
        series::tests::assert_payloads_equal,
    };
    let format = replay_data::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let slot = fuel_slot(1000);
    let mut frame = fuel_frame(slot, true, 9001, (100, 10_000, 19, 7));
    let mut fixtures = [Fixture::new(), Fixture::new()];
    let mut states = [
        Prepared::default(),
        Prepared {
            hourly_settlement: true,
            ..Default::default()
        },
    ];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut publications: [Vec<i32>; 2] = Default::default();
    for offset in -DAY_HOURS..=25 {
        frame.hour = slot + offset;
        frame.metrics.table_ns.fill(0);
        if offset == 0 {
            frame.metrics.table_ns[TableId::CountryFuel.index()] = 1;
        }
        for (state, tracker) in states.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe(&frame, format);
            state
                .observe_changes(&frame, format, &changes, &flag)
                .unwrap();
        }
        if offset < 23 {
            continue;
        }
        if offset == 24 {
            assert!(
                states[1].days.bytes() > 0,
                "hourly settlement charges carried day rows after the flush"
            );
        }
        // Held pages leave exactly enough room for the hourly row charge.
        // Missing the first carried day rows incorrectly defers this commit.
        let held = BATCH_BYTES - states[1].buffered_bytes();
        for (index, (state, fixture)) in states.iter_mut().zip(&mut fixtures).enumerate() {
            if !publication_due(offset != 25, false, offset == 23, || {
                (state.buffered_bytes(), held)
            }) {
                continue;
            }
            publications[index].push(offset);
            let mut writer = if offset > 23 {
                fixture.append_writer()
            } else {
                fixture.writer()
            };
            state.flush(&mut writer, 1, format, &flag).unwrap();
            writer.commit().unwrap();
        }
    }
    assert_eq!(publications[1], [23, 24, 25]);
    assert_eq!(
        publications[0], publications[1],
        "fuel budget-selected publication hours"
    );
    assert_payloads_equal(
        &fixtures[0].payloads(),
        &fixtures[1].payloads(),
        "fuel budget-selected pages and ranges",
    );
    assert_eq!(
        serde_json::to_vec(&states[0]).unwrap(),
        serde_json::to_vec(&states[1]).unwrap()
    );
}
