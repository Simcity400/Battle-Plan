use super::*;
use replay_frame::{frame::FrameDecoder, frame::FrameEncoder, frame::FrameInput};
use std::sync::Barrier;

fn hour_events(country: u32, hour: i32) -> Vec<Event> {
    let before = 100 - 5 * i64::from(hour - 23) + i64::from(hour > 24) * 7;
    let mut events = vec![
        Event {
            before,
            after: before - 5,
            requested: Some(-5),
            cause: PowerCause::Decision,
            site: PowerSite::AddPoliticalPower,
            ..Default::default()
        },
        Event {
            currency: PowerCurrency::Command,
            before: 200,
            after: 200,
            requested: Some(7),
            site: PowerSite::AddCommandPower,
            ..Default::default()
        },
        Event {
            kind: Kind::Government,
            parties: [Some((hour - 22) as u32), Some((hour - 21) as u32)],
            government_cause: GovernmentCause::SetPoliticsEffect,
            ..Default::default()
        },
    ];
    if hour.rem_euclid(24) == 0 {
        for component in [
            PowerDailyCostKind::PoliticalPowerCostModifier,
            PowerDailyCostKind::Relation,
            PowerDailyCostKind::MioResearch,
        ] {
            events.push(Event {
                kind: Kind::Cost,
                component,
                cost: 1,
                ..Default::default()
            });
        }
        events.extend([
            Event {
                kind: Kind::Daily,
                gross: 10,
                cost: 3,
                net: 7,
                ..Default::default()
            },
            Event {
                before: before - 5,
                after: before + 2,
                requested: Some(7),
                cause: PowerCause::DailyGain,
                site: PowerSite::AddPoliticalPower,
                ..Default::default()
            },
            Event {
                currency: PowerCurrency::Command,
                before: 200,
                after: 200,
                requested: Some(9),
                cause: PowerCause::DailyGain,
                site: PowerSite::AddCommandPower,
                ..Default::default()
            },
        ]);
    }
    for event in &mut events {
        event.country = country;
        event.instance = 1;
        event.hour = hour;
    }
    events
}

#[test]
fn politics_tfix2_multilane_hours_encode_alias_census_and_midnight_without_loss() {
    let _guard = hooks::staging_test();
    let ids = [
        361, 362, 363, 364, 440, 441, 442, 368, 369, 370, 371, 372, 373, 374, 375, 376,
    ];
    let staging = hooks::StagingFixture::new(16 * 16, 16);
    let mut captured = Captured::with_capacities(&Capacities::default());
    captured.cursors = vec![hooks::LaneCursor::default(); 16];
    let mut countries: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(slot, &id)| engine_native::ccountry::Row {
            id,
            canonical_slot: 361 + slot as u32,
            politics_witness: Some((1, Some(1))),
            political_power_raw: Some(100),
            command_power_raw: 200,
            ..Default::default()
        })
        .collect();
    let mut operatives: Vec<_> = ids
        .iter()
        .enumerate()
        .map(|(n, &country)| OperativeMembershipRow {
            id: 1000 - n as u64,
            roster_country: country,
            owner_country: country,
            ordinal: Recorded::Value(n as u32),
            ..Default::default()
        })
        .collect();
    let mut shared = operatives[0].clone();
    shared.roster_country = ids[1];
    operatives.push(shared);
    let operative_view = crate::table_conversion::table::<tables::OperativeMemberships, _>(
        Outcome::Rows {
            len: operatives.len(),
        },
        &operatives,
        Clone::clone,
    );
    let mut ledger = Ledger::default();
    let mut frames = Vec::new();
    let census = Outcome::Rows {
        len: countries.len(),
    };
    // SAFETY: test owns `staging` and `captured` with no concurrent producer.
    captured.handoff = Some(unsafe { staging.handoff(&mut captured.cursors) });
    let mut tables = (0..replay_data::rows::TABLE_COUNT)
        .map(|_| TableInput::Sampled(unavailable("fixture")))
        .collect::<Vec<_>>();
    tables[TableId::OperativeMemberships.index()] = TableInput::Sampled(operative_view);
    // SAFETY: test is the sole spool consumer of `captured.handoff`.
    unsafe { ledger.assemble_owned(1, 22, &captured, &countries, census, &mut tables) }.unwrap();
    frames.push((22, tables.clone()));
    let hour_gate = Barrier::new(17);
    let event_gate = Barrier::new(16);
    let mut drained_sequences = Vec::new();
    std::thread::scope(|scope| {
        for &country in &ids {
            let (staging, hour_gate, event_gate) = (&staging, &hour_gate, &event_gate);
            scope.spawn(move || {
                let memory = engine_native_memory::memory::SliceMemory {
                    base: 0,
                    bytes: &[],
                };
                for hour in 23..=27 {
                    hour_gate.wait();
                    for event in hour_events(country, hour) {
                        staging.observe(event, &memory);
                        event_gate.wait();
                    }
                    hour_gate.wait();
                    hour_gate.wait();
                }
            });
        }
        for hour in 23..=27 {
            hour_gate.wait();
            hour_gate.wait();
            for country in &mut countries {
                country.political_power_raw =
                    Some(100 - 5 * i64::from(hour - 22) + i64::from(hour >= 24) * 7);
                country.politics_witness = Some((1, Some((hour - 21) as u32)));
            }
            // SAFETY: producer threads are paused at `hour_gate` during this handoff.
            captured.handoff = Some(unsafe { staging.handoff(&mut captured.cursors) });
            // SAFETY: test is the sole spool consumer of `captured.handoff`.
            unsafe { ledger.assemble_owned(1, hour, &captured, &countries, census, &mut tables) }
                .unwrap();
            let scratch = ledger.scratch.as_ref().unwrap();
            let drain = scratch.drained.unwrap().unwrap();
            drained_sequences.extend(scratch.events[..drain.copied].iter().map(|e| e.sequence));
            frames.push((hour, tables.clone()));
            hour_gate.wait();
        }
    });
    assert_eq!(staging.registered_lanes(), 16);
    assert_eq!(drained_sequences, (1..=336).collect::<Vec<_>>());
    let handoff = captured.handoff.unwrap().unwrap();
    // SAFETY: all producer threads have joined and test is the sole consumer.
    let repeated = unsafe { handoff.drain(&captured.cursors, &mut [Event::default(); 336], &mut []) }
        .unwrap();
    assert_eq!((repeated.copied, repeated.pending), (0, 0));
    let mut encoder = FrameEncoder::new();
    let mut decoder = FrameDecoder::new();
    for (hour, tables) in frames {
        let source = FrameInput {
            branch: 1,
            hour,
            tables: tables.clone().try_into().unwrap(),
            metrics: Default::default(),
        };
        let encoded = encoder
            .encode_bytes(source, false)
            .unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let decoded = decoder.apply(&encoded.decode().unwrap()).unwrap();
        for table in TABLES.into_iter().chain([TableId::OperativeMemberships]) {
            let expected = rows(&tables[table.index()]).unwrap();
            let TableView::Complete(actual) = &*decoded.tables[table.index()] else {
                panic!()
            };
            assert_eq!(actual, expected, "hour {hour}, table {table:?}");
        }
        let Some(TableRows::CountryPowerChanges(changes)) =
            rows(&tables[TableId::CountryPowerChanges.index()])
        else {
            panic!()
        };
        assert_eq!(
            changes.len(),
            if hour == 22 {
                0
            } else if hour == 24 {
                96
            } else {
                48
            }
        );
        let Some(TableRows::GovernmentChanges(governments)) =
            rows(&tables[TableId::GovernmentChanges.index()])
        else {
            panic!()
        };
        assert_eq!(governments.len(), if hour == 22 { 0 } else { 16 });
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(coverage.len(), 16);
        let mut expected_ids = ids;
        expected_ids.sort_unstable();
        assert_eq!(
            coverage.iter().map(|row| row.country).collect::<Vec<_>>(),
            expected_ids
        );
        for row in coverage {
            let expected = if hour == 22 {
                0
            } else if hour == 23 {
                PoliticsCaptureCoverageRow::PP
                    | PoliticsCaptureCoverageRow::CP
                    | PoliticsCaptureCoverageRow::GOVERNMENT
            } else {
                PoliticsCaptureCoverageRow::ALL_STREAMS
            };
            assert_eq!(
                row.streams, expected,
                "hour {hour}, country {}",
                row.country
            );
        }
        let Some(TableRows::CountryPowerDaily(daily)) =
            rows(&tables[TableId::CountryPowerDaily.index()])
        else {
            panic!()
        };
        let Some(TableRows::CountryPowerDailyCosts(costs)) =
            rows(&tables[TableId::CountryPowerDailyCosts.index()])
        else {
            panic!()
        };
        assert_eq!(daily.len(), if hour < 24 { 0 } else { 16 });
        assert_eq!(costs.len(), if hour < 24 { 0 } else { 48 });
        for day in daily {
            assert_eq!(
                (day.day_hour, day.pp_net_gain_raw, day.cp_gain_raw),
                (24, Some(7), Some(9))
            );
            if hour == 24 {
                for sequence in [day.pp_sequence.unwrap(), day.cp_sequence.unwrap()] {
                    assert!(
                        changes
                            .iter()
                            .any(|row| { row.country == day.country && row.sequence == sequence })
                    );
                }
            }
        }
        for cost in costs {
            let day = daily
                .iter()
                .find(|row| row.country == cost.country)
                .unwrap();
            assert_eq!(Some(cost.pp_sequence), day.pp_sequence);
            assert_eq!(cost.cost_raw, 1);
            assert_eq!(
                cost.component,
                [
                    PowerDailyCostKind::PoliticalPowerCostModifier,
                    PowerDailyCostKind::Relation,
                    PowerDailyCostKind::MioResearch,
                ][cost.ordinal as usize]
            );
        }
        for correction in changes
            .iter()
            .filter(|row| row.operation == PowerOperation::Clamp)
        {
            let native = changes
                .iter()
                .find(|row| {
                    row.country == correction.country && row.sequence + 1 == correction.sequence
                })
                .unwrap();
            assert_eq!(correction.cause, PowerCause::CapCorrection);
            assert_eq!(
                correction.requested_delta_raw,
                native.requested_delta_raw.map(|amount| -amount)
            );
            assert_eq!(
                (correction.balance_before_raw, correction.balance_after_raw),
                (200, 200)
            );
        }
        let Some(TableRows::OperativeMemberships(memberships)) =
            rows(&tables[TableId::OperativeMemberships.index()])
        else {
            panic!()
        };
        assert_eq!(memberships.len(), 17);
        assert_eq!(memberships.iter().filter(|row| row.id == 1000).count(), 2);
    }
}

#[test]
fn politics_tfix2_inactive_alias_census_encodes_every_country() {
    let count = 6;
    let mut captured = Captured::worker(&Capacities {
        countries: count,
        entries: 0,
        bytes: 0,
    })
    .unwrap();
    for (slot, balance) in captured.balances.iter_mut().enumerate() {
        *balance = BalanceSnapshot {
            country: (count - slot) as u32,
            instance: slot as u32 + 1,
            pp: (slot % 2 == 0).then_some(100),
            cp: 200,
            party: None,
        };
    }
    captured.census = Outcome::Rows { len: count };
    captured.drained = Some(Ok(Drained {
        epoch: 1,
        first: 0,
        cursor: 0,
        copied: 0,
        bytes: 0,
        pending: 0,
        keys_required: 0,
    }));
    let mut tables = (0..replay_data::rows::TABLE_COUNT)
        .map(|_| TableInput::Sampled(unavailable("fixture")))
        .collect::<Vec<_>>();
    let mut ledger = Ledger::default();
    let mut encoder = FrameEncoder::new();
    for hour in [23, 24] {
        ledger.assemble(1, hour, &captured, &mut tables);
        let source = FrameInput {
            branch: 1,
            hour,
            tables: tables.clone().try_into().unwrap(),
            metrics: Default::default(),
        };
        encoder.encode_bytes(source, false).unwrap();
        let Some(TableRows::PoliticsCaptureCoverage(coverage)) =
            rows(&tables[TableId::PoliticsCaptureCoverage.index()])
        else {
            panic!()
        };
        assert_eq!(coverage.len(), count);
        for (n, row) in coverage.iter().enumerate() {
            assert_eq!(row.country, n as u32 + 1);
            let original = &captured.balances[count - n - 1];
            assert_eq!(row.instance, original.instance);
            let mut streams = 0;
            if hour == 24 {
                streams = PoliticsCaptureCoverageRow::CP | PoliticsCaptureCoverageRow::GOVERNMENT;
                if original.pp.is_some() {
                    streams |= PoliticsCaptureCoverageRow::PP;
                }
            }
            assert_eq!(row.streams, streams);
        }
    }
}

#[test]
fn politics_tfix2_extreme_clamp_chunks_cannot_collide_with_next_observation() {
    let last = (u64::MAX - (EVENT_SEQUENCE_STRIDE - 1)) / EVENT_SEQUENCE_STRIDE;
    assert_eq!(event_sequence(last), last * EVENT_SEQUENCE_STRIDE);
    assert!(std::panic::catch_unwind(|| event_sequence(last + 1)).is_err());
    let mut captured = Captured::worker(&Capacities {
        countries: 1,
        entries: 4,
        bytes: 0,
    })
    .unwrap();
    captured.balances[0] = BalanceSnapshot {
        country: 5001,
        instance: 1,
        pp: None,
        cp: i64::MIN,
        party: None,
    };
    captured.census = Outcome::Rows { len: 1 };
    captured.drained = Some(Ok(Drained {
        epoch: 1,
        first: 0,
        cursor: 0,
        copied: 0,
        bytes: 0,
        pending: 0,
        keys_required: 0,
    }));
    let mut tables = (0..replay_data::rows::TABLE_COUNT)
        .map(|_| TableInput::Sampled(unavailable("fixture")))
        .collect::<Vec<_>>();
    let mut ledger = Ledger::default();
    let mut encoder = FrameEncoder::new();
    ledger.assemble(1, 23, &captured, &mut tables);
    encoder
        .encode_bytes(
            FrameInput {
                branch: 1,
                hour: 23,
                tables: tables.clone().try_into().unwrap(),
                metrics: Default::default(),
            },
            false,
        )
        .unwrap();
    for (n, (before, after, requested)) in [
        (i64::MIN, i64::MAX, i64::MIN),
        (i64::MAX, i64::MAX, 0),
        (i64::MAX, i64::MIN, i64::MAX),
        (i64::MIN, i64::MIN, 0),
    ]
    .into_iter()
    .enumerate()
    {
        captured.events[n] = Event {
            country: 5001,
            instance: 1,
            epoch: 1,
            sequence: n as u64 + 1,
            hour: 24,
            currency: PowerCurrency::Command,
            site: PowerSite::AddCommandPower,
            before,
            after,
            requested: Some(requested),
            ..Default::default()
        };
    }
    captured.drained = Some(Ok(Drained {
        cursor: 4,
        copied: 4,
        pending: 4,
        ..captured.drained.unwrap().unwrap()
    }));
    ledger.assemble(1, 24, &captured, &mut tables);
    let encoded = encoder
        .encode_bytes(
            FrameInput {
                branch: 1,
                hour: 24,
                tables: tables.clone().try_into().unwrap(),
                metrics: Default::default(),
            },
            false,
        )
        .unwrap();
    encoded.decode().unwrap();
    let Some(TableRows::CountryPowerChanges(changes)) =
        rows(&tables[TableId::CountryPowerChanges.index()])
    else {
        panic!()
    };
    assert_eq!(changes.len(), 11);
    for native in changes
        .iter()
        .filter(|row| row.operation != PowerOperation::Clamp)
    {
        let corrections = changes
            .iter()
            .filter(|row| {
                row.sequence > native.sequence
                    && row.sequence < native.sequence + EVENT_SEQUENCE_STRIDE
                    && row.operation == PowerOperation::Clamp
            })
            .collect::<Vec<_>>();
        let expected =
            native.realized_delta_raw() - i128::from(native.requested_delta_raw.unwrap());
        assert_eq!(
            corrections
                .iter()
                .map(|row| {
                    assert_eq!(row.cause, PowerCause::CapCorrection);
                    assert_eq!(row.realized_delta_raw(), 0);
                    i128::from(row.requested_delta_raw.unwrap())
                })
                .sum::<i128>(),
            expected
        );
        if native.balance_before_raw == i64::MIN && native.balance_after_raw == i64::MAX {
            assert_eq!(corrections.len(), 4);
        }
    }
}
