use super::super::tests::frame;
use super::*;
use crate::{prepared::series::tests::Fixture, test_fixtures};
use replay::{
    frame::TableView,
    rows::{OperationCompletionRow, TableRows},
};

fn source(chunks: &[Vec<DecodedFrame>]) -> Source {
    let first = Replay::from_records(
        test_fixtures::CAMPAIGN.into(),
        &[test_fixtures::records(1, chunks[0].clone())],
    )
    .unwrap();
    let mut records = vec![test_fixtures::records(1, chunks[0].clone())];
    records.extend(
        chunks[1..]
            .iter()
            .map(|frames| test_fixtures::continuation_records(&first, 1, frames.clone())),
    );
    Source::from_replay(
        crate::SourceKey {
            source_id: "prepared-espionage".into(),
            revision: "synthetic".into(),
        },
        "fixtures/espionage.battleplan".into(),
        Arc::new(Replay::from_records(test_fixtures::CAMPAIGN.into(), &records).unwrap()),
        &AtomicBool::new(false),
    )
    .unwrap()
}
fn observe(
    state: &mut Prepared,
    source: &Source,
    fixture: &Fixture,
    frame: &DecodedFrame,
    flag: &AtomicBool,
) {
    if state.needs_history(frame) {
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            "espionage-log-1-series".into(),
            flag,
        )
        .unwrap();
        state
            .observe(source, frame, Some(&mut reader), flag)
            .unwrap();
    } else {
        state.observe(source, frame, None, flag).unwrap();
    }
}
fn parity(source: &Source, fixture: &Fixture, old: &Builder, last: i32, flag: &AtomicBool) {
    let raw = old.reply("1".into(), 100, last, source.replay_ref().format.version());
    let count = metadata_windows(100, last).len();
    for window in std::iter::once(None).chain((0..count).map(Some)) {
        let mut expected = metadata_window(&raw, window).unwrap();
        present(source, &LoadContext { content: None }, flag, &mut expected).unwrap();
        let actual = query(
            source,
            &LoadContext { content: None },
            &fixture.store,
            &fixture.key,
            1,
            &EspionageLogParams {
                branch_id: "1".into(),
                window,
            },
            flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap(),
            "window {window:?}, through {last}"
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_espionage_writes_every_short_block_range_across_empty_days_gaps_and_resume() {
    let chunks: Vec<Vec<_>> = (0..48)
        .map(|ordinal| {
            let first = if ordinal < 16 {
                100 + ordinal * 2
            } else {
                172 + (ordinal - 16) * 2
            };
            (first..first + 2)
                .map(|hour| {
                    let mut input = frame(hour, u32::from(hour >= 172));
                    input.metrics.table_ns[TableId::Operations.index()] =
                        u64::from((hour - 100) % 24 == 0);
                    input
                })
                .collect()
        })
        .collect();
    let source = source(&chunks);
    assert_eq!(source.block_windows(1).len(), chunks.len());
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut old = Builder::new(source.replay_ref());
    let mut state = Prepared::default();
    let section = "espionage-log-1-windows";
    for phase in 0..3 {
        let mut writer = fixture.writer();
        for (ordinal, chunk) in chunks.iter().enumerate().skip(phase * 16).take(16) {
            for input in chunk {
                old.observe(input);
                old.observe_completions(input, source.replay_ref(), 1)
                    .unwrap();
                observe(&mut state, &source, &fixture, input, &flag);
            }
            state
                .finish_block(&source, &mut writer, 1, ordinal, &flag)
                .unwrap();
            assert!(
                writer.read_at(section, ordinal as u64).unwrap().is_some(),
                "block {ordinal} must publish a range even when it owns no day"
            );
        }
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert_eq!(
            fixture.store.range_count(&fixture.key, section).unwrap(),
            Some(((phase + 1) * 16) as u64)
        );
        parity(
            &source,
            &fixture,
            &old,
            chunks[(phase + 1) * 16 - 1].last().unwrap().hour,
            &flag,
        );
        state = series::read_reply(&fixture.store, &fixture.key, "espionage-log-1", 0)
            .unwrap()
            .unwrap();
    }
    let empty: Vec<EspionageLogReply> =
        series::read_reply(&fixture.store, &fixture.key, section, 1)
            .unwrap()
            .unwrap();
    assert!(empty.is_empty(), "block 1 supplies no day start");
    for (window, first, last, recorded) in [(0, 100, 123, true), (2, 148, 171, false)] {
        let reply = query(
            &source,
            &LoadContext { content: None },
            &fixture.store,
            &fixture.key,
            1,
            &EspionageLogParams {
                branch_id: "1".into(),
                window: Some(window),
            },
            &flag,
        )
        .unwrap()
        .unwrap();
        assert_eq!((reply.first_hour, reply.last_hour), (first, last));
        assert!(reply.operations.is_empty());
        assert_eq!(
            reply.available,
            if recorded {
                vec![HourSpan {
                    first_hour: first,
                    last_hour: last,
                }]
            } else {
                Vec::new()
            }
        );
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_espionage_packs_days_crossing_blocks_and_repairs_late_completions_after_append() {
    let frames: Vec<_> = (100..=243)
        .map(|hour| {
            let mut input = frame(hour, 1);
            input.metrics.table_ns[TableId::Operations.index()] = u64::from((hour - 100) % 24 == 0);
            let TableView::Complete(TableRows::Operations(rows)) =
                &mut *input.tables[TableId::Operations.index()]
            else {
                panic!("operations");
            };
            if (124..148).contains(&hour) {
                rows[0].duration_days = 2;
                rows[0].start_hour = 124;
            }
            if (148..172).contains(&hour) {
                rows.clear();
            }
            if hour >= 172 {
                rows[0].preparation_current = 50;
            }
            if hour == 161 {
                *input.tables[TableId::Operations.index()] = TableView::Unavailable {
                    reason: "synthetic missed census".into(),
                };
            }
            if hour >= 196 {
                *input.tables[TableId::OperationCompletions.index()] =
                    TableView::Complete(TableRows::OperationCompletions(vec![
                        OperationCompletionRow {
                            id: 1,
                            owner: 1,
                            target: 1,
                            definition_token: 70,
                            sequence: 1,
                            completion_hour: 144,
                            terminal: 1,
                            bonus: 1,
                            detected: 0,
                            effect_description: Vec::new(),
                            description_required: 0,
                            ..Default::default()
                        },
                    ]));
            }
            input
        })
        .collect();
    let chunks = vec![
        frames[..37].to_vec(),
        frames[37..85].to_vec(),
        frames[85..].to_vec(),
    ];
    let source = source(&chunks);
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut old = Builder::new(source.replay_ref());
    let mut state = Prepared::default();
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut writer = if ordinal == 2 {
            // The common pipeline binds accepted prefix ranges to the grown
            // recording before any observer reads earlier series batches.
            fixture.append_writer().commit().unwrap();
            fixture.writer()
        } else {
            fixture.writer()
        };
        for input in chunk {
            old.observe(input);
            old.observe_completions(input, source.replay_ref(), 1)
                .unwrap();
            observe(&mut state, &source, &fixture, input, &flag);
        }
        state
            .finish_block(&source, &mut writer, 1, ordinal, &flag)
            .unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&source, &fixture, &old, chunk.last().unwrap().hour, &flag);
        state = series::read_reply(&fixture.store, &fixture.key, "espionage-log-1", 0)
            .unwrap()
            .unwrap();
        assert!(state.builder.seen_completions.is_empty());
    }
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "espionage-log-1-windows")
            .unwrap(),
        Some(3)
    );
    let first: Vec<EspionageLogReply> =
        series::read_reply(&fixture.store, &fixture.key, "espionage-log-1-windows", 0)
            .unwrap()
            .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|reply| reply.window.unwrap())
            .collect::<Vec<_>>(),
        [0, 1]
    );
    assert_eq!(first[1].last_hour, 147);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn prepared_espionage_retains_terminal_census_runs_when_a_late_drain_moves_the_end() {
    let frames: Vec<_> = (100..=195)
        .map(|hour| {
            let mut input = frame(hour, 1);
            input.metrics.table_ns[TableId::Operations.index()] = u64::from((hour - 100) % 24 == 0);
            let TableView::Complete(TableRows::Operations(rows)) =
                &mut *input.tables[TableId::Operations.index()]
            else {
                panic!("operations");
            };
            rows[0].terminal = u8::from(hour >= 124);
            rows[0].preparation_current = i64::from(hour / 24);
            if hour >= 172 {
                *input.tables[TableId::OperationCompletions.index()] =
                    TableView::Complete(TableRows::OperationCompletions(vec![
                        OperationCompletionRow {
                            id: 1,
                            owner: 1,
                            target: 1,
                            definition_token: 70,
                            sequence: 1,
                            completion_hour: 125,
                            terminal: 1,
                            bonus: 1,
                            detected: 0,
                            effect_description: Vec::new(),
                            description_required: 0,
                            ..Default::default()
                        },
                    ]));
            }
            input
        })
        .collect();
    let chunks = vec![
        frames[..48].to_vec(),
        frames[48..72].to_vec(),
        frames[72..].to_vec(),
    ];
    let source = source(&chunks);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut old = Builder::new(source.replay_ref());
    let mut state = Prepared::default();
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut writer = fixture.writer();
        for input in chunk {
            old.observe(input);
            old.observe_completions(input, source.replay_ref(), 1)
                .unwrap();
            observe(&mut state, &source, &fixture, input, &flag);
        }
        state
            .finish_block(&source, &mut writer, 1, ordinal, &flag)
            .unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&source, &fixture, &old, chunk.last().unwrap().hour, &flag);
        state = series::read_reply(&fixture.store, &fixture.key, "espionage-log-1", 0)
            .unwrap()
            .unwrap();
    }
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_espionage_preserves_five_thousand_countries_and_identity_changes() {
    let chunks: Vec<Vec<_>> = (0..2)
        .map(|day| {
            (100 + day * 24..124 + day * 24)
                .map(|hour| {
                    let mut input = frame(hour, 5_000);
                    input.metrics.table_ns[TableId::Operations.index()] =
                        u64::from((hour - 100) % 24 == 0);
                    if day == 1 {
                        let TableView::Complete(TableRows::Countries(rows)) =
                            &mut *input.tables[TableId::Countries.index()]
                        else {
                            panic!("countries");
                        };
                        rows[0].tag = b"mod alias / opaque".to_vec();
                    }
                    input
                })
                .collect()
        })
        .collect();
    let source = source(&chunks);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut old = Builder::new(source.replay_ref());
    let mut state = Prepared::default();
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut writer = fixture.writer();
        for input in chunk {
            old.observe(input);
            old.observe_completions(input, source.replay_ref(), 1)
                .unwrap();
            observe(&mut state, &source, &fixture, input, &flag);
        }
        state
            .finish_block(&source, &mut writer, 1, ordinal, &flag)
            .unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        parity(&source, &fixture, &old, chunk.last().unwrap().hour, &flag);
        state = series::read_reply(&fixture.store, &fixture.key, "espionage-log-1", 0)
            .unwrap()
            .unwrap();
    }
    assert_eq!(state.builder.open.len(), 5_000);
}

#[cfg(feature = "slow-tests")]
#[test]
fn prepared_espionage_checkpoint_does_not_retain_daily_runs_or_reads() {
    let chunks: Vec<Vec<_>> = (0..100)
        .map(|day| {
            (100 + day * 24..124 + day * 24)
                .map(|hour| {
                    let mut input = frame(hour, 1);
                    input.metrics.table_ns[TableId::Operations.index()] =
                        u64::from((hour - 100) % 24 == 0);
                    input
                })
                .collect()
        })
        .collect();
    let source = source(&chunks);
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut state = Prepared::default();
    let mut short = 0;
    for (ordinal, chunk) in chunks.iter().enumerate() {
        let mut writer = fixture.writer();
        for input in chunk {
            observe(&mut state, &source, &fixture, input, &flag);
        }
        state
            .finish_block(&source, &mut writer, 1, ordinal, &flag)
            .unwrap();
        state.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if ordinal == 1 {
            short = serde_json::to_vec(&state).unwrap().len();
        }
    }
    let long = serde_json::to_vec(&state).unwrap().len();
    eprintln!("espionage checkpoint bytes: 2 days={short}, 100 days={long}");
    assert!(long < short * 2);
    assert!(state.builder.reads.len() <= 2);
    assert!(state.builder.operations.iter().all(|op| op.runs.len() <= 2));
}

#[test]
fn intel_shared_driver_espionage_empty_equal_reads_are_flat_and_exact() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use crate::products::{intelligence_prepared::measured, operative_rows::CountryInputs};
    let scaled = observer_tests::scaled_work(8, |hours| {
        let mut first = frame(100, 1);
        *first.tables[TableId::Operations.index()] =
            TableView::Complete(TableRows::Operations(Vec::new()));
        let frames: Vec<_> = (0..=hours)
            .map(|offset| {
                let mut input = first.clone();
                input.hour += offset as i32;
                input
            })
            .collect();
        let middle = hours / 2;
        let source = source(&[frames[..middle].to_vec(), frames[middle..].to_vec()]);
        let flag = AtomicBool::new(false);
        let mut fixture = Fixture::new();
        let mut state = Prepared::default();
        let mut countries = CountryInputs::default();
        let changes = FrameChangeTracker::new().observe(&first, source.replay_ref().format);
        countries.update(&first, Some(&changes));
        state.observe_changes(&source, &first, &changes, &countries, None, &flag)?;
        let delta = observer_tests::unchanged_frames(
            measured(),
            &first,
            hours as i32,
            source.replay_ref().format,
            |input, changes| {
                let restored = if input.hour == first.hour + (hours / 2) as i32 {
                    let mut writer = fixture.writer();
                    state.finish_block(&source, &mut writer, 1, 0, &flag)?;
                    state.flush(&mut writer, 1, &flag)?;
                    writer.commit()?;
                    state = series::read_reply(&fixture.store, &fixture.key, "espionage-log-1", 0)?
                        .unwrap();
                    countries = CountryInputs::default();
                    Some(FrameChangeTracker::new().observe(input, source.replay_ref().format))
                } else {
                    None
                };
                let changes = restored.as_ref().unwrap_or(changes);
                countries.update(input, Some(changes));
                assert!(!state.needs_history_changes(input, changes));
                state.observe_changes(&source, input, changes, &countries, None, &flag)?;
                Ok(measured())
            },
        )?;
        let mut old = Builder::new(source.replay_ref());
        for input in &frames {
            old.observe(input);
            old.observe_completions(input, source.replay_ref(), 1)?;
        }
        let mut writer = fixture.append_writer();
        state.finish_block(&source, &mut writer, 1, 1, &flag)?;
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        parity(&source, &fixture, &old, frames.last().unwrap().hour, &flag);
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 0);
    assert_eq!(scaled[0].history_visits, 0);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn intel_shared_driver_espionage_one_row_ignores_ten_times_closed_instances() {
    use crate::prepared::observer_tests;
    use crate::products::intelligence_prepared::{measured, work};
    let scaled = observer_tests::scaled_work(8, |history| {
        let frames: Vec<_> = (0..history * 2)
            .map(|index| {
                let mut input = frame(100 + index as i32, 1);
                let TableView::Complete(TableRows::Operations(rows)) =
                    &mut *input.tables[TableId::Operations.index()]
                else {
                    unreachable!()
                };
                rows[0].terminal = u8::from(index % 2 == 0);
                input
            })
            .collect();
        let source = source(std::slice::from_ref(&frames));
        let flag = AtomicBool::new(false);
        let mut state = Prepared::default();
        for input in &frames {
            state.observe(&source, input, None, &flag)?;
        }
        assert_eq!(state.builder.operations.len(), history + 1);
        let mut input = frames.last().unwrap().clone();
        input.hour += 1;
        let TableView::Complete(TableRows::Operations(rows)) =
            &mut *input.tables[TableId::Operations.index()]
        else {
            unreachable!()
        };
        rows[0].preparation_current += 1;
        work(|work| *work = Default::default());
        state.observe(&source, &input, None, &flag)?;
        Ok(measured())
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].history_visits, 2);
}
