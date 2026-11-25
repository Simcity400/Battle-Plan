use super::*;
use crate::fuel_awards::Entry;
use engine_fuel::{fuel_awards::Recipient, fuel_awards::Settled, fuel_awards::Started};
use replay_data::rows::CountryFuelRow;

fn input(count: u32) -> Vec<TableInput> {
    let mut tables = (0..replay_data::rows::TABLE_COUNT)
        .map(|_| TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(Vec::new()))))
        .collect::<Vec<_>>();
    tables[TableId::CountryFuel.index()] =
        TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(
            (1..=count)
                .map(|country| CountryFuelRow {
                    country,
                    current_raw: country as i64,
                    country_instance: Recorded::Value(Some(1)),
                    ..Default::default()
                })
                .collect(),
        )));
    tables
}
fn capture(first: u64, entries: Vec<Entry>) -> Captured {
    let copied = entries.len();
    let mut roots = Vec::new();
    let entries = entries
        .into_iter()
        .map(|entry| CompactEntry::from_entry(entry, &mut roots).unwrap())
        .collect();
    Captured {
        entries,
        roots,
        names: b"retained".to_vec(),
        drained: Some(Ok((
            Drained {
                epoch: 1,
                first,
                cursor: first + copied as u64,
                copied,
                pending: copied,
            },
            8,
            8,
        ))),
    }
}
fn entries(hour: i32, count: u32) -> Vec<Entry> {
    (1..=count)
        .flat_map(|country| {
            let amount = if country == 1 { 0 } else { -(country as i64) };
            let root = hour as u64 * 10000 + country as u64;
            [
                Entry {
                    root,
                    country,
                    recipient: Some(Recipient {
                        kind: 0,
                        reference: (country as u64) << 32 | 29,
                        owner: country + 100,
                        host: 0,
                    }),
                    amount,
                    name: Some((0, 8)),
                    ..Default::default()
                },
                Entry {
                    root,
                    country,
                    started: Started {
                        country,
                        status: country as usize * 0x1000,
                        country_instance: 1,
                        pending_gain: 91,
                        remaining_hours: 7,
                        ..Default::default()
                    },
                    settled: Settled {
                        gains: [2, -3, 4, 0],
                        own_gain: 3,
                        received: [amount, 0, 0],
                        ..Default::default()
                    },
                    ..Default::default()
                },
            ]
        })
        .collect()
}

#[test]
fn same_pointer_and_zero_instance_reset_withdraws_only_its_old_window() {
    let mut windows = Windows::default();
    let mut tables = input(2);
    let mut cursor = 0;
    if let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) =
        &mut tables[TableId::CountryFuel.index()]
    {
        for row in rows {
            row.country_instance = Recorded::Value(Some(0));
        }
    }
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=48 {
        let mut events = entries(hour, 2);
        events[1].started.country_instance = 0;
        events[3].started.country_instance = 0;
        if hour == 25 {
            events.insert(
                0,
                Entry {
                    reset: true,
                    started: Started {
                        status: 0x1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
        }
        let captured = capture(cursor, events);
        cursor = captured.drained.unwrap().unwrap().0.cursor;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
        if hour == 25 {
            let Recorded::Value(reset) = &rows(&tables)[0].award_day else {
                panic!()
            };
            assert!(!reset.reason.is_empty());
            assert!(reset.recipients.is_empty() && reset.own_gain_raw == 0);
            assert_eq!(reset.continuity_start_hour, Some(25));
        }
    }
    let assembled = assembled(&tables, 48);
    let Recorded::Value(reset) = &assembled[0].award_day else {
        panic!()
    };
    let Recorded::Value(peer) = &assembled[1].award_day else {
        panic!()
    };
    assert!(reset.reason.is_empty());
    assert_eq!(reset.last_hour, FuelAwardDayHour::Host);
    assert_eq!(reset.last_hour.at(48), 48);
    assert_eq!(
        (
            reset.recipients[0].first_hour,
            reset.recipients[0].last_hour
        ),
        (25, 48)
    );
    assert_eq!(reset.continuity_start_hour, Some(25));
    assert_eq!(peer.continuity_start_hour, Some(1));
    assert!(peer.reason.is_empty());
    assert_eq!(peer.own_gain_raw, 24 * 3);
    assert_eq!(rows(&tables)[0].current_raw, 1);
    assert!(
        rows(&tables)
            .iter()
            .all(|row| matches!(&row.award_day, Recorded::Value(day) if day.recipients.is_empty()))
    );
    assert!(
        rows(&tables)
            .iter()
            .all(|row| row.awards == Recorded::Value(replay_data::rows::Retired))
    );
}

#[test]
fn repeated_roots_in_one_sample_cannot_certify_two_continuity_segments() {
    let mut windows = Windows::default();
    let mut tables = input(2);
    let mut cursor = 0;
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=49 {
        let mut events = entries(hour, 2);
        if hour == 25 {
            let mut repeated = events[..2].to_vec();
            for event in &mut repeated {
                event.root += 500;
            }
            repeated.insert(
                0,
                Entry {
                    reset: true,
                    started: Started {
                        status: 0x1000,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            events.extend(repeated);
        }
        let captured = capture(cursor, events);
        cursor = captured.drained.unwrap().unwrap().0.cursor;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
        if hour == 25 {
            let Recorded::Value(first) = &rows(&tables)[0].award_day else {
                panic!()
            };
            assert!(first.continuity_start_hour.is_none());
            assert!(!first.reason.is_empty());
        }
    }
    let Recorded::Value(first) = &rows(&tables)[0].award_day else {
        panic!()
    };
    let Recorded::Value(peer) = &rows(&tables)[1].award_day else {
        panic!()
    };
    assert!(first.reason.is_empty());
    assert_eq!(first.continuity_start_hour, Some(26));
    assert_eq!(peer.continuity_start_hour, Some(1));
}

#[test]
fn continuity_start_is_scoped_by_actual_branch_and_fresh_reattach_observation() {
    let mut tracker = replay_frame::branches::BranchTracker::new();
    tracker.begin_session(7).unwrap();
    let mut windows = Windows::default();
    let mut tables = input(1);
    let mut cursor = 0;
    let first = tracker.observe(None, 0, Some(1), &[]).unwrap().branch;
    tracker.record_hour(first, 0).unwrap();
    windows
        .assemble(first, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=24 {
        let branch = tracker.observe(None, hour, Some(1), &[]).unwrap().branch;
        tracker.record_hour(branch, hour).unwrap();
        let captured = capture(cursor, entries(hour, 1));
        cursor += 2;
        windows
            .assemble(branch, hour, &captured, &mut tables)
            .unwrap();
    }
    let mut previous = first;
    for (boundary, signal) in [(24, 1), (0, 1), (25, 2)] {
        let observation = tracker.observe(None, boundary, Some(signal), &[]).unwrap();
        assert_ne!(observation.branch, previous);
        previous = observation.branch;
        tracker.record_hour(previous, boundary).unwrap();
        windows
            .assemble(
                previous,
                boundary,
                &capture(cursor, Vec::new()),
                &mut tables,
            )
            .unwrap();
        for hour in boundary + 1..=boundary + 24 {
            assert_eq!(
                tracker
                    .observe(None, hour, Some(signal), &[])
                    .unwrap()
                    .branch,
                previous
            );
            tracker.record_hour(previous, hour).unwrap();
            let captured = capture(cursor, entries(hour, 1));
            cursor += 2;
            windows
                .assemble(previous, hour, &captured, &mut tables)
                .unwrap();
        }
        let Recorded::Value(window) = &rows(&tables)[0].award_day else {
            panic!()
        };
        assert!(window.reason.is_empty());
        assert_eq!(window.continuity_start_hour, Some(boundary + 1));
    }
    let mut resumed = replay_frame::branches::BranchTracker::new();
    resumed
        .seed_recorded_branch(
            tracker.branch_start(previous).unwrap().clone(),
            tracker.recorded_hours(previous).unwrap().iter().copied(),
        )
        .unwrap();
    resumed.resume_session(7, previous, 49, Some(2)).unwrap();
    let mut windows = Windows::default();
    cursor = 0;
    assert_eq!(
        resumed.observe(None, 50, Some(2), &[]).unwrap().branch,
        previous
    );
    windows
        .assemble(previous, 50, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 51..=74 {
        assert_eq!(
            resumed.observe(None, hour, Some(2), &[]).unwrap().branch,
            previous
        );
        let captured = capture(cursor, entries(hour, 1));
        cursor += 2;
        windows
            .assemble(previous, hour, &captured, &mut tables)
            .unwrap();
    }
    let Recorded::Value(window) = &rows(&tables)[0].award_day else {
        panic!()
    };
    assert!(window.reason.is_empty());
    assert_eq!(window.continuity_start_hour, Some(51));
}
fn rows(tables: &[TableInput]) -> &[CountryFuelRow] {
    let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(rows))) =
        &tables[TableId::CountryFuel.index()]
    else {
        panic!()
    };
    rows
}
fn assembled(tables: &[TableInput], hour: i32) -> Vec<CountryFuelRow> {
    let mut fuel = rows(tables).to_vec();
    let TableInput::Sampled(TableView::Complete(TableRows::FuelAwardRecipients(recipients))) =
        &tables[TableId::FuelAwardRecipients.index()]
    else {
        panic!()
    };
    replay_data::rows::assemble_award_recipients(&mut fuel, Ok(recipients), hour);
    fuel
}
#[test]
fn writer_window_keeps_corrections_names_owner_and_local_refusals() {
    let mut windows = Windows::default();
    let mut tables = input(10);
    let mut cursor = 0;
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=24 {
        let mut events = entries(hour, 10);
        if hour == 24 {
            events[(7 - 1) * 2 + 1].refused = true;
        }
        let captured = capture(cursor, events);
        cursor = captured.drained.unwrap().unwrap().0.cursor;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
    }
    let rows = assembled(&tables, 24);
    for row in &rows {
        let Recorded::Value(day) = &row.award_day else {
            panic!()
        };
        assert_eq!(row.current_raw, row.country as i64);
        if row.country == 7 {
            assert!(!day.reason.is_empty());
            continue;
        }
        assert!(day.reason.is_empty());
        assert_eq!(day.last_hour, FuelAwardDayHour::Host);
        assert_eq!(day.last_hour.at(24), 24);
        assert_eq!(day.gains_raw, [48, -72, 96, 0]);
        assert_eq!(day.own_gain_raw, 72);
        assert_eq!(
            day.own_gain_quantity_raw,
            24 * engine_fuel::fuel_awards::quantity(3)
        );
        assert_eq!(day.recipients.len(), 1);
        let part = &day.recipients[0];
        assert_eq!(part.name.as_deref(), Some(&b"retained"[..]));
        assert_eq!(part.owner, row.country + 100);
        assert_eq!((part.first_hour, part.last_hour), (1, 24));
        assert_eq!(
            part.amount_raw,
            if row.country == 1 {
                0
            } else {
                -24 * row.country as i64
            }
        );
        assert_eq!(day.received_raw, [part.amount_raw, 0, 0]);
    }
}

#[test]
fn a_day_whose_sums_leave_their_native_width_is_refused_by_name() {
    let mut windows = Windows::default();
    let mut tables = input(1);
    let mut cursor = 0;
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    let before = super::day_sum_refusals();
    for hour in 1..=24 {
        let mut events = entries(hour, 1);
        events[1].settled.own_gain = i64::MAX;
        let captured = capture(cursor, events);
        cursor = captured.drained.unwrap().unwrap().0.cursor;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
    }
    let Recorded::Value(day) = &rows(&tables)[0].award_day else {
        panic!()
    };
    assert!(String::from_utf8_lossy(&day.reason).contains("exceeds its native i64"));
    assert!(day.recipients.is_empty() && day.own_gain_raw == 0);
    assert_eq!(day.continuity_start_hour, Some(1));
    assert!(super::day_sum_refusals() > before);
}
#[test]
fn missing_interval_writer_epoch_and_country_replacement_do_not_fabricate_zero() {
    let mut windows = Windows::default();
    let mut tables = input(2);
    let mut cursor = 0;
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=24 {
        let captured = capture(cursor, entries(hour, 2));
        cursor += 4;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
    }
    let TableInput::Sampled(TableView::Complete(TableRows::CountryFuel(fuel))) =
        &mut tables[TableId::CountryFuel.index()]
    else {
        panic!()
    };
    fuel[0].country = 3;
    fuel[0].current_raw = 3;
    for hour in 25..=48 {
        let mut changed = entries(hour, 2);
        for entry in &mut changed[..2] {
            entry.country = 3;
            entry.started.country = 3;
            if let Some(recipient) = &mut entry.recipient {
                recipient.reference = (3_u64 << 32) | 29;
                recipient.owner = 103;
            }
        }
        let captured = capture(cursor, changed);
        cursor += 4;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
        assert!(!windows.countries.contains_key(&1));
        assert!(!windows.identities.contains_key(&1));
        assert_eq!(windows.status_owners.get(&0x1000), Some(&3));
        assert_eq!(windows.countries[&3].1.len(), (hour - 24) as usize);
        let assembled = assembled(&tables, hour);
        let Recorded::Value(first) = &assembled[0].award_day else {
            panic!()
        };
        let Recorded::Value(second) = &assembled[1].award_day else {
            panic!()
        };
        assert_eq!(first.continuity_start_hour, Some(25));
        assert!(second.reason.is_empty());
        assert_eq!(second.continuity_start_hour, Some(1));
        if hour < 48 {
            assert!(!first.reason.is_empty());
            assert!(first.recipients.is_empty());
        } else {
            assert!(first.reason.is_empty());
            assert_eq!(first.last_hour, FuelAwardDayHour::Host);
            assert_eq!(first.last_hour.at(48), 48);
            assert_eq!(first.own_gain_raw, 72);
            assert_eq!(first.gains_raw, [48, -72, 96, 0]);
            assert_eq!(first.recipients.len(), 1);
            assert_eq!(first.recipients[0].owner, 103);
            assert_eq!(
                (
                    first.recipients[0].first_hour,
                    first.recipients[0].last_hour
                ),
                (25, 48)
            );
        }
    }
    let captured = capture(cursor, entries(50, 2));
    cursor += 4;
    windows.assemble(1, 50, &captured, &mut tables).unwrap();
    assert!(
        rows(&tables)
            .iter()
            .all(|r| matches!(&r.award_day,Recorded::Value(w) if !w.reason.is_empty()))
    );
    let mut restarted = capture(cursor, entries(51, 2));
    restarted
        .drained
        .as_mut()
        .unwrap()
        .as_mut()
        .unwrap()
        .0
        .epoch = 2;
    windows.assemble(1, 51, &restarted, &mut tables).unwrap();
    assert!(
        rows(&tables)
            .iter()
            .all(|r| matches!(&r.award_day,Recorded::Value(w) if w.recipients.is_empty() && !w.reason.is_empty()))
    );
}

#[test]
fn surrender_target_changes_keep_the_exact_day_and_record_the_latest_raw_target() {
    let mut windows = Windows::default();
    let mut tables = input(1);
    let mut cursor = 0;
    windows
        .assemble(1, 0, &capture(0, Vec::new()), &mut tables)
        .unwrap();
    for hour in 1..=25 {
        let mut events = entries(hour, 1);
        events[1].started.country_instance = if hour < 24 { 0 } else { 7 };
        let captured = capture(cursor, events);
        cursor += 2;
        windows.assemble(1, hour, &captured, &mut tables).unwrap();
        if hour >= 24 {
            let Recorded::Value(day) = &rows(&tables)[0].award_day else {
                panic!()
            };
            assert!(day.reason.is_empty());
            assert_eq!(day.country_instance, 7);
            assert_eq!(day.continuity_start_hour, Some(1));
            assert_eq!(day.own_gain_raw, 72);
        }
    }
}
