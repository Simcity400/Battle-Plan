use super::*;
use crate::products::decryption::tests::{DEFINES, frame};
use dashboard_base::prepared::series::tests::Fixture;
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::frame::TableView;

fn reference(scan: &Scan) -> DecryptionReply {
    DecryptionReply {
        branch_id: "1".into(),
        pairs: scan
            .pairs
            .values()
            .map(|state| state.pair.clone())
            .collect(),
        coverage: scan.coverage.clone(),
        warnings: scan.refusals.diagnostic_lines("cipher entries affected"),
    }
}

fn assert_reply(fixture: &Fixture, scan: &Scan) {
    let params = Params {
        branch_id: "1".into(),
        pairs: Vec::new(),
        first_hour: None,
        last_hour: None,
    };
    let actual = query(
        &fixture.store,
        &fixture.key,
        1,
        &params,
        &AtomicBool::new(false),
    )
    .unwrap()
    .unwrap();
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(reference(scan)).unwrap()
    );
}

#[test]
fn prepared_cipher_query_rejects_a_different_parameter_branch() {
    let fixture = Fixture::new();
    for branch_id in ["2", "invalid"] {
        let params = Params {
            branch_id: branch_id.into(),
            pairs: Vec::new(),
            first_hour: None,
            last_hour: None,
        };
        let error = query(
            &fixture.store,
            &fixture.key,
            1,
            &params,
            &AtomicBool::new(false),
        )
        .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("dispatched branch"));
    }
}

#[test]
fn prepared_cipher_reply_matches_after_commits_resume_and_append() {
    let mut fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Accumulator::default();
    let mut old = Scan::default();
    for phase in 0..3 {
        let mut writer = if phase == 2 {
            fixture.append_writer()
        } else {
            fixture.writer()
        };
        for hour in 100 + phase * 48..148 + phase * 48 {
            let amount = 9_007_199_254_740_993 + i64::from((hour - 100) / 24) * 100_000;
            let mut input = frame(hour, 2, amount, 1);
            if hour == 157 {
                input.tables[TableId::Cryptology.index()] = TableView::Unavailable {
                    reason: "synthetic cipher gap".into(),
                }
                .into();
            }
            if hour >= 160 {
                let TableView::Complete(TableRows::Countries(rows)) =
                    &mut *input.tables[TableId::Countries.index()]
                else {
                    panic!("countries fixture");
                };
                rows[1].tag = b"Mod alias | opaque".to_vec();
            }
            old.frame(&input, Ok(DEFINES)).unwrap();
            prepared.observe_with_defines(&input, Ok(DEFINES)).unwrap();
        }
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        assert_reply(&fixture, &old);
        prepared = series::read_reply(&fixture.store, &fixture.key, "decryption-1", 0)
            .unwrap()
            .unwrap();
        assert!(
            prepared
                .scan
                .pairs
                .values()
                .all(|state| state.pair.runs.len() == 1 && state.pair.tag_runs.len() == 1)
        );
        assert_eq!(prepared.scan.coverage.len(), 1);
    }
    let params = Params {
        branch_id: "1".into(),
        pairs: vec![("1".into(), "2".into())],
        first_hour: Some(242),
        last_hour: Some(242),
    };
    let actual = query(&fixture.store, &fixture.key, 1, &params, &flag)
        .unwrap()
        .unwrap();
    let expected = old.pairs[&(1, 2)].pair.runs.last().unwrap();
    assert_eq!(
        actual.pairs[0].runs[0].amount_at(242),
        expected.amount_at(242)
    );
    assert_eq!(actual.pairs[0].runs[0].started, expected.started);
}

#[test]
fn prepared_cipher_checkpoint_retains_open_rows_without_daily_history() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Accumulator::default();
    let mut short = 0;
    for day in 0..100 {
        for hour in day * 24..(day + 1) * 24 {
            prepared
                .observe_with_defines(&frame(hour, 2, i64::from(day) * 100_000, 1), Ok(DEFINES))
                .unwrap();
        }
        let mut writer = fixture.writer();
        prepared.flush(&mut writer, 1, &flag).unwrap();
        writer.commit().unwrap();
        if day == 1 {
            short = serde_json::to_vec(&prepared).unwrap().len();
        }
    }
    let long = serde_json::to_vec(&prepared).unwrap().len();
    eprintln!("cipher checkpoint bytes: 2 days={short}, 100 days={long}");
    assert!(long < short * 2);
    assert_eq!(prepared.scan.pairs[&(1, 2)].pair.runs.len(), 1);
    assert_eq!(prepared.scan.coverage.len(), 1);
}

#[test]
fn intel_shared_driver_cipher_equal_rows_are_flat_and_exact() {
    use crate::products::{intelligence_prepared::measured, operative_rows::CountryInputs};
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let scaled = observer_tests::scaled_work(8, |hours| {
        let first = frame(100, 2, 400_000, 1);
        let format = replay_data::format::FormatVersion::default();
        let mut fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut state = Accumulator::default();
        let mut countries = CountryInputs::default();
        let changes = FrameChangeTracker::new().observe(&first, format);
        countries.update(&first, Some(&changes));
        state.observe_bound(&first, &changes, &countries, Ok(DEFINES), &[])?;
        let mut old = Scan::default();
        old.frame(&first, Ok(DEFINES))?;
        for offset in 1..=hours {
            let mut input = first.clone();
            input.hour += offset as i32;
            old.frame(&input, Ok(DEFINES))?;
        }
        let delta = observer_tests::unchanged_frames(
            measured(),
            &first,
            hours as i32,
            format,
            |input, changes| {
                let restored = if input.hour == first.hour + (hours / 2) as i32 {
                    let mut writer = fixture.writer();
                    state.flush(&mut writer, 1, &flag)?;
                    writer.commit()?;
                    state = series::read_reply(&fixture.store, &fixture.key, "decryption-1", 0)?
                        .unwrap();
                    countries = CountryInputs::default();
                    Some(FrameChangeTracker::new().observe(input, format))
                } else {
                    None
                };
                let changes = restored.as_ref().unwrap_or(changes);
                countries.update(input, Some(changes));
                state.observe_bound(input, changes, &countries, Ok(DEFINES), &[])?;
                Ok(measured())
            },
        )?;
        let mut writer = fixture.append_writer();
        state.flush(&mut writer, 1, &flag)?;
        writer.commit()?;
        let actual = query(
            &fixture.store,
            &fixture.key,
            1,
            &Params {
                branch_id: "1".into(),
                pairs: Vec::new(),
                first_hour: None,
                last_hour: None,
            },
            &flag,
        )?
        .unwrap();
        observer_tests::assert_reply_parity(&actual, &reference(&old))?;
        Ok(delta)
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 3);
    assert_eq!(scaled[0].history_visits, 1);
}

#[test]
fn intel_shared_driver_cipher_one_row_ignores_ten_times_closed_pairs() {
    use crate::products::intelligence_prepared::{measured, work};
    use dashboard_base::prepared::observer_tests;
    let scaled = observer_tests::scaled_work(8, |history| {
        let mut state = Accumulator::default();
        let mut input = frame(100, 2, 400_000, 1);
        for index in 0..history {
            input.hour = 100 + index as i32;
            let slot = index as u32 + 2;
            let TableView::Complete(TableRows::Countries(rows)) =
                &mut *input.tables[TableId::Countries.index()]
            else {
                unreachable!()
            };
            rows[1].canonical_slot = slot;
            let TableView::Complete(TableRows::CountryLinks(rows)) =
                &mut *input.tables[TableId::CountryLinks.index()]
            else {
                unreachable!()
            };
            for row in rows.iter_mut().filter(|row| row.id >= 3) {
                row.canonical_slot = slot;
            }
            state.observe_with_defines(&input, Ok(DEFINES))?;
        }
        assert_eq!(state.scan.pairs.len(), history);
        input.hour += 1;
        let TableView::Complete(TableRows::Cryptology(rows)) =
            &mut *input.tables[TableId::Cryptology.index()]
        else {
            unreachable!()
        };
        rows[0].amount_raw += 1;
        work(|work| *work = Default::default());
        state.observe_with_defines(&input, Ok(DEFINES))?;
        Ok(measured())
    })
    .unwrap();
    observer_tests::assert_flat(scaled);
    assert_eq!(scaled[0].row_visits, 1);
    assert_eq!(scaled[0].emitted, 1);
}

#[test]
fn unflushed_cipher_checkpoint_continues_without_losing_or_repeating_observations() {
    use dashboard_base::prepared::series::tests::assert_payloads_equal;
    let run = |resume: bool| {
        let fixture = Fixture::new();
        let flag = AtomicBool::new(false);
        let mut state = Accumulator::default();
        let mut oracle = Scan::default();
        let mut snapshots = Vec::new();
        for hours in [100..148, 148..196] {
            for hour in hours {
                let amount = 9_007_199_254_740_993 + i64::from((hour - 100) / 24) * 100_000;
                let input = frame(hour, 2, amount, 1);
                oracle.frame(&input, Ok(DEFINES)).unwrap();
                state.observe_with_defines(&input, Ok(DEFINES)).unwrap();
                if resume && matches!(hour, 112 | 135 | 160 | 183) {
                    assert!(state.buffered_bytes() > 0);
                    let bytes = series::compress(&serde_json::to_vec(&state).unwrap(), 3).unwrap();
                    state = Accumulator::read(bytes.as_slice()).unwrap();
                }
            }
            let mut writer = fixture.writer();
            state.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
            assert_reply(&fixture, &oracle);
            snapshots.push(fixture.payloads());
        }
        snapshots
    };
    for (actual, expected) in run(true).iter().zip(run(false).iter()) {
        assert_payloads_equal(actual, expected, "unflushed decryption resume");
    }
}
