use super::super::tests::{event, frame, put};
use super::*;
use crate::prepared::series::tests::Fixture;
use serde_json::json;

fn publish(prepared: &mut Prepared, fixture: &Fixture) {
    let mut writer = fixture.writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer
        .write_at(
            "politics-ledger-state-1",
            0,
            prepared.saved_bytes().unwrap().as_slice(),
        )
        .unwrap();
    writer.commit().unwrap();
}
fn read(prepared: &Fixture, tag: &str, from: i32, hour: i32, view: &str) -> PoliticsLedgerReply {
    serde_json::from_value(
        query::query(
            &json!({"branchId":"1", "tag":tag, "view":view, "from":from, "hour":hour}),
            &prepared.store,
            &prepared.key,
            &AtomicBool::new(false),
        )
        .unwrap(),
    )
    .unwrap()
}
fn spend_total(reply: &PoliticsLedgerReply) -> i128 {
    reply
        .spending
        .iter()
        .filter_map(|row| row.charge_raw.as_deref())
        .map(|raw| raw.parse::<i128>().unwrap())
        .sum::<i128>()
        + reply
            .recurring
            .iter()
            .map(|row| row.known_raw.parse::<i128>().unwrap())
            .sum::<i128>()
}
fn daily(day: i32, read: i32, pp: bool) -> DecodedFrame {
    let mut current = frame(read, 1);
    put(
        &mut current,
        TableRows::CountryPowerDaily(vec![CountryPowerDailyRow {
            country: 1,
            instance: 1,
            epoch: 1,
            day_hour: day,
            pp_gross_gain_raw: pp.then_some(100),
            pp_cost_raw: pp.then_some(7),
            pp_net_gain_raw: pp.then_some(93),
            pp_realized_delta_raw: pp.then_some(80),
            pp_sequence: pp.then_some(day as u64),
            cp_gain_raw: Some(30),
            cp_realized_delta_raw: Some(30),
            cp_sequence: Some(day as u64 + 1),
        }]),
    );
    if pp {
        put(
            &mut current,
            TableRows::CountryPowerDailyCosts(vec![CountryPowerDailyCostRow {
                country: 1,
                instance: 1,
                epoch: 1,
                day_hour: day,
                pp_sequence: day as u64,
                cost_raw: 7,
                cause: PowerCause::DailyGain,
                ..Default::default()
            }]),
        );
    }
    current
}

#[test]
fn prepared_content_success_and_failure_do_not_load_after_configuration() {
    use crate::prepared::observer_tests::{self, Work};
    use replay::format::FormatVersion;

    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for ledger content binding");
    for complete in [true, false] {
        // The store owns only prepared files; its cleanup deletes file entries.
        // Synthetic game directories have their own lifetime outside that store.
        let folder = crate::catalogue::tests::Folder(
            std::env::temp_dir().join(format!("ledger-content-{}", uuid::Uuid::new_v4())),
        );
        let game = &folder.0;
        std::fs::create_dir_all(game.join("common/defines")).unwrap();
        std::fs::write(game.join("common/defines/synthetic.lua"), if complete {
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}}}"
        } else {
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30}}"
        }).unwrap();
        let first = frame(24, 1);
        let (source, context) =
            crate::test_fixtures::source_with_content(vec![first.clone()], game);
        let options = context.content.as_ref();
        let binding = prepared_content(&source, options);
        assert_eq!(binding.is_ok(), complete);
        let refusal = binding.as_ref().err().cloned();
        assert_eq!(
            crate::products::politics_content::content_loads(&source),
            1 + usize::from(complete)
        );
        let repeated = prepared_content(&source, options);
        assert_eq!(repeated.as_ref().err(), refusal.as_ref());
        assert_eq!(
            crate::products::politics_content::content_loads(&source),
            1 + usize::from(complete)
        );
        observer_tests::scaled_work(10, |hours| {
            let mut prepared = Prepared::default();
            prepared.configure(&binding);
            let mut tracker = crate::prepared::frame_changes::FrameChangeTracker::new();
            prepared.observe(
                &first,
                &tracker.observe(&first, FormatVersion::CURRENT),
                &AtomicBool::new(false),
            )?;
            let configured = Work {
                content_loads: crate::products::politics_content::content_loads(&source),
                ..Default::default()
            };
            let work = observer_tests::unchanged_frames(
                configured,
                &first,
                hours as i32,
                FormatVersion::CURRENT,
                |frame, changes| {
                    prepared.observe(frame, changes, &AtomicBool::new(false))?;
                    Ok(Work {
                        content_loads: crate::products::politics_content::content_loads(&source),
                        ..Default::default()
                    })
                },
            )?;
            if let Some(reason) = &refusal {
                assert_eq!(prepared.scan.warnings[&None][reason], 1);
            }
            Ok(work)
        })
        .unwrap();
        drop(repeated);
        drop(binding);
        drop(source);
        drop(context);
        std::fs::remove_dir_all(game).unwrap();
    }
}

#[test]
fn prepared_shared_driver_unchanged_frames_and_closed_history_keep_work_flat() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let unchanged = observer_tests::scaled_work(10, |hours| {
        let first = frame(24, 1);
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        prepared.observe(
            &first,
            &tracker.observe(&first, FormatVersion::CURRENT),
            &flag,
        )?;
        observer_tests::unchanged_frames(
            prepared.measured_work(),
            &first,
            hours as i32,
            FormatVersion::CURRENT,
            |frame, changes| {
                prepared.observe(frame, changes, &flag)?;
                Ok(prepared.measured_work())
            },
        )
    })
    .unwrap();
    observer_tests::assert_flat(unchanged);
    let history = observer_tests::scaled_work(6, |hours| {
        let mut prepared = Prepared::default();
        let mut tracker = FrameChangeTracker::new();
        let mut last = frame(24, 1);
        for offset in 0..hours {
            last = frame(24 + offset as i32, 1);
            let mut countries = last.complete::<tables::Countries>().unwrap().to_vec();
            countries[0].tag = format!("H{offset}").into_bytes();
            put(&mut last, TableRows::Countries(countries));
            let mut payment = event(
                offset as u64 + 1,
                PowerCause::Decision,
                PowerOperation::Subtract,
                Some(-7),
            );
            payment.hour = last.hour;
            put(&mut last, TableRows::CountryPowerChanges(vec![payment]));
            prepared.observe(
                &last,
                &tracker.observe(&last, FormatVersion::CURRENT),
                &flag,
            )?;
        }
        observer_tests::unchanged_frames(
            prepared.measured_work(),
            &last,
            8,
            FormatVersion::CURRENT,
            |frame, changes| {
                prepared.observe(frame, changes, &flag)?;
                Ok(prepared.measured_work())
            },
        )
    })
    .unwrap();
    observer_tests::assert_flat(history);
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_one_changed_row_and_unrelated_country_balances_bound_native_and_projection_work() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let mut first = frame(24, 5000);
    put(
        &mut first,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 1,
            epoch: 1,
            since_hour: 24,
            through_hour: 24,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    let mut prepared = Prepared::default();
    let mut tracker = FrameChangeTracker::new();
    prepared
        .observe(
            &first,
            &tracker.observe(&first, FormatVersion::CURRENT),
            &flag,
        )
        .unwrap();
    first.hour = 25;
    prepared
        .observe(
            &first,
            &tracker.observe(&first, FormatVersion::CURRENT),
            &flag,
        )
        .unwrap();
    let before = prepared.measured_work();
    let mut balance = first.clone();
    balance.hour = 26;
    let mut countries = balance.complete::<tables::Countries>().unwrap().to_vec();
    countries[0].political_power_raw = Some(1);
    put(&mut balance, TableRows::Countries(countries));
    prepared
        .observe(
            &balance,
            &tracker.observe(&balance, FormatVersion::CURRENT),
            &flag,
        )
        .unwrap();
    let after = prepared.measured_work();
    assert_eq!(
        after.row_visits - before.row_visits,
        5000,
        "only the changed Countries projection is visited"
    );
    assert_eq!(after.history_visits, before.history_visits);
    assert_eq!(after.emitted, before.emitted);
    let work = observer_tests::scaled_work(1, |_scale| {
        let before = prepared.measured_work();
        let mut changed = balance.clone();
        changed.hour = prepared.metadata.last.unwrap() + 1;
        let mut payment = event(
            changed.hour as u64,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-7),
        );
        payment.hour = changed.hour;
        put(&mut changed, TableRows::CountryPowerChanges(vec![payment]));
        prepared.observe(
            &changed,
            &tracker.observe(&changed, FormatVersion::CURRENT),
            &flag,
        )?;
        let after = prepared.measured_work();
        assert_eq!(after.row_visits - before.row_visits, 1);
        assert!(
            after.history_visits - before.history_visits <= 1,
            "one payment projects one country"
        );
        Ok(observer_tests::Work {
            row_visits: after.row_visits - before.row_visits,
            history_visits: after.history_visits - before.history_visits,
            emitted: after.emitted - before.emitted,
            ..Default::default()
        })
    })
    .unwrap();
    assert!(
        work.iter()
            .all(|work| work.row_visits == 1 && work.history_visits <= 1 && work.emitted <= 32)
    );
    let before = prepared.measured_work();
    balance.hour = prepared.metadata.last.unwrap() + 1;
    let hour = balance.hour;
    put(
        &mut balance,
        TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
            country: 1,
            instance: 1,
            epoch: 1,
            since_hour: hour,
            through_hour: hour,
            streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
        }]),
    );
    prepared
        .observe(
            &balance,
            &tracker.observe(&balance, FormatVersion::CURRENT),
            &flag,
        )
        .unwrap();
    let after = prepared.measured_work();
    assert_eq!(after.row_visits - before.row_visits, 1);
    assert!(
        after.history_visits - before.history_visits <= 1,
        "one coverage owner must not revisit 5,000 countries"
    );
}

#[test]
fn prepared_bulk_refusal_rates_keep_every_native_alias_and_the_first_naming_read() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::format::FormatVersion;
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut first = frame(24, 1);
    let rejected = CountryRow {
        id: 1,
        canonical_slot: 1,
        tag: b"invalid\0".to_vec(),
        ..Default::default()
    };
    let reason = valid_recorded_tag(&rejected).unwrap_err();
    let alias = CountryRow {
        id: 2,
        ..rejected.clone()
    };
    let alias_reason = valid_recorded_tag(&alias).unwrap_err();
    put(
        &mut first,
        TableRows::Countries(vec![
            rejected.clone(),
            alias,
            CountryRow {
                id: 3,
                canonical_slot: 1,
                tag: b"C1".to_vec(),
                ..Default::default()
            },
        ]),
    );
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let mut tracker = FrameChangeTracker::new();
    for hour in 24..=43 {
        first.hour = hour;
        oracle.observe(&first, None).unwrap();
        prepared
            .observe(
                &first,
                &tracker.observe(&first, FormatVersion::CURRENT),
                &flag,
            )
            .unwrap();
        if hour == 29 || hour == 43 {
            publish(&mut prepared, &fixture);
            observer_tests::assert_reply_parity(
                &read(&fixture, "C1", 24, hour, "raw").warnings,
                &present_view(&oracle, 1, "C1", None, &flag, LedgerView::Raw)
                    .unwrap()
                    .warnings,
            )
            .unwrap();
            assert_eq!(prepared.scan.refused, oracle.refused);
            prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
            prepared.resume(&first);
            tracker.invalidate();
        }
    }
    assert_eq!(prepared.scan.refused[&1].1, 40);
    // Each diagnostic names its own native row; the first read has no known
    // tag for either alias, then each warns once on the remaining 19 reads.
    let warnings = &prepared.scan.warnings[&Some("C1".into())];
    assert_eq!(warnings[&reason], 19);
    assert_eq!(warnings[&alias_reason], 19);
    assert_eq!(warnings[&reason] + warnings[&alias_reason], 38);
}

#[test]
fn prepared_bulk_clock_matches_oracle_refusals_coverage_and_holds_across_publication_resume_append_and_gaps()
 {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let mut tracker = FrameChangeTracker::new();
    let mut current = frame(24, 3);
    let mut census = None;
    for (stage, end) in [(0, 30), (1, 44), (2, 60), (3, 70), (4, 80)] {
        if stage == 1 {
            let mut countries = current.complete::<tables::Countries>().unwrap().to_vec();
            countries[1].tag = b"invalid\0".to_vec();
            countries.pop(); // Keep the absent country's zero hold across resume.
            put(&mut current, TableRows::Countries(countries));
            current.tables[TableId::CountryPowerChanges.index()] =
                replay::frame::TableView::Unavailable {
                    reason: "synthetic missing stream".into(),
                }
                .into();
        }
        if stage == 2 {
            // The first appended read is separated by a recording gap.
            current.hour = 50;
            tracker.invalidate();
        }
        if stage == 3 {
            census = Some(current.tables[TableId::Countries.index()].clone());
            current.tables[TableId::Countries.index()] = replay::frame::TableView::Unavailable {
                reason: "synthetic census gap".into(),
            }
            .into();
        }
        if stage == 4 {
            current.tables[TableId::Countries.index()] = census.take().unwrap();
        }
        while current.hour <= end {
            oracle.observe(&current, None).unwrap();
            prepared
                .observe(
                    &current,
                    &tracker.observe(&current, FormatVersion::CURRENT),
                    &flag,
                )
                .unwrap();
            current.hour += 1;
        }
        if stage < 2 {
            publish(&mut prepared, &fixture);
        } else {
            let mut writer = fixture.append_writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        for tag in ["C1", "C2", "C3"] {
            let actual = read(&fixture, tag, 24, end, "raw");
            let expected = present_view(&oracle, 1, tag, None, &flag, LedgerView::Raw).unwrap();
            observer_tests::assert_reply_parity(&actual.coverage, &expected.coverage).unwrap();
            observer_tests::assert_reply_parity(
                &actual.spending_coverage,
                &expected.spending_coverage,
            )
            .unwrap();
            observer_tests::assert_reply_parity(&actual.command_holds, &expected.command_holds)
                .unwrap();
            observer_tests::assert_reply_parity(&actual.warnings, &expected.warnings).unwrap();
        }
        assert_eq!(prepared.scan.refused, oracle.refused);
        let mut last = current.clone();
        last.hour = end;
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&last);
        tracker.invalidate();
    }
}

#[test]
fn prepared_absent_spending_coverage_ends_when_the_census_is_lost_on_the_read_after_resume() {
    use crate::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let mut tracker = FrameChangeTracker::new();
    let mut current = frame(24, 2);
    for (stage, end) in [(0, 30), (1, 34)] {
        if stage == 1 {
            // Resumed at a complete census, the next read loses it.
            current.tables[TableId::Countries.index()] = replay::frame::TableView::Unavailable {
                reason: "synthetic census gap".into(),
            }
            .into();
        }
        while current.hour <= end {
            if current.hour == 27 {
                // A complete census without C2 proves it made no payments.
                let mut countries = current.complete::<tables::Countries>().unwrap().to_vec();
                countries.pop();
                put(&mut current, TableRows::Countries(countries));
            }
            oracle.observe(&current, None).unwrap();
            prepared
                .observe(
                    &current,
                    &tracker.observe(&current, FormatVersion::CURRENT),
                    &flag,
                )
                .unwrap();
            current.hour += 1;
        }
        if stage == 0 {
            publish(&mut prepared, &fixture);
        } else {
            let mut writer = fixture.append_writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        let actual = read(&fixture, "C2", 24, end, "raw").spending_coverage;
        let expected = present_view(&oracle, 1, "C2", None, &flag, LedgerView::Raw)
            .unwrap()
            .spending_coverage;
        observer_tests::assert_reply_parity(&actual, &expected).unwrap();
        if stage == 0 {
            let mut last = current.clone();
            last.hour = end;
            prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
            prepared.resume(&last);
            tracker.invalidate();
        } else {
            // The lost census proves neither absence nor coverage.
            let last = actual.last().unwrap();
            assert_eq!(
                (
                    last.since_hour,
                    last.through_hour,
                    last.political,
                    last.command,
                    last.reason.as_deref()
                ),
                (
                    Some(31),
                    Some(34),
                    false,
                    false,
                    Some("politics coverage unavailable")
                )
            );
        }
    }
}

#[test]
fn prepared_hold_snapshots_stop_after_census_loss_across_commits_resume_and_append() {
    use crate::prepared::frame_changes::FrameChangeTracker;
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let mut tracker = FrameChangeTracker::new();
    let mut current = frame(24, 3);
    put(
        &mut current,
        TableRows::CountryCommandPower(
            (1..=3)
                .map(|canonical_slot| CountryCommandPowerRow {
                    canonical_slot,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut census = None;
    for (stage, end) in [
        (0, 24),
        (1, 29),
        (2, 34),
        (3, 38),
        (4, 44),
        (5, 50),
        (6, 51),
        (7, 55),
    ] {
        if stage == 1 {
            let mut rows = current.complete::<tables::Countries>().unwrap().to_vec();
            rows[0].tag = b"invalid\0".to_vec();
            rows.remove(1);
            put(&mut current, TableRows::Countries(rows));
            census = Some(current.tables[TableId::Countries.index()].clone());
        }
        if stage == 2 {
            current.hour = 32;
            tracker.invalidate();
        }
        if stage == 3 {
            current.tables[TableId::Countries.index()] = replay::frame::TableView::Unavailable {
                reason: "synthetic census gap".into(),
            }
            .into();
        }
        if stage == 4 {
            current.tables[TableId::Countries.index()] = census.take().unwrap();
        }
        if stage == 5 {
            let mut rows = current.complete::<tables::Countries>().unwrap().to_vec();
            rows.insert(
                1,
                CountryRow {
                    id: 2,
                    canonical_slot: 2,
                    tag: b"C2".to_vec(),
                    ..Default::default()
                },
            );
            put(&mut current, TableRows::Countries(rows));
        }
        if stage == 6 {
            census = Some(current.tables[TableId::Countries.index()].clone());
            let mut rows = current.complete::<tables::Countries>().unwrap().to_vec();
            rows.remove(1);
            put(&mut current, TableRows::Countries(rows));
        }
        if stage == 7 {
            current.tables[TableId::Countries.index()] = census.take().unwrap();
        }
        while current.hour <= end {
            oracle.observe(&current, None).unwrap();
            prepared
                .observe(
                    &current,
                    &tracker.observe(&current, FormatVersion::CURRENT),
                    &flag,
                )
                .unwrap();
            current.hour += 1;
        }
        if stage < 2 {
            publish(&mut prepared, &fixture);
        } else {
            let mut writer = fixture.append_writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        }
        for tag in ["C1", "C2", "C3"] {
            let expected = present_view(&oracle, 1, tag, None, &flag, LedgerView::Raw).unwrap();
            assert_eq!(
                read(&fixture, tag, 24, end, "raw").command_holds,
                expected.command_holds,
                "{tag} through {end}",
            );
            assert_eq!(
                read(&fixture, tag, end, end, "raw").command_holds,
                vec![expected.command_holds.last().unwrap().clone()],
                "{tag} reached at {end}",
            );
            assert_eq!(
                holds::history(&prepared.scan, tag).last(),
                expected.command_holds.last(),
                "{tag} continuation through {end}",
            );
        }
        if stage > 0 {
            assert_eq!(
                holds::history(&oracle, "C1").last().unwrap().through_hour,
                25
            );
            if stage < 5 {
                let last = holds::history(&oracle, "C2").last().unwrap();
                assert_eq!((last.read_hour, last.through_hour), (24, 25));
            }
        }
        let mut last = current.clone();
        last.hour = end;
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&last);
        tracker.invalidate();
    }
}

#[test]
fn prepared_tail_ranges_are_dense_through_empty_blocks_resume_append_and_fresh_store() {
    let flag = AtomicBool::new(false);
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut previous = None;
    for block in 0..16 {
        let day = 24 * (block + 1);
        // Countries arrive after empty blocks, including a later tag which
        // sorts before an existing tag. Most blocks have no government rows.
        let ids = match block {
            0..=1 => vec![],
            2..=7 => vec![2],
            8..=11 => vec![1, 2],
            _ => vec![1, 2, 3],
        };
        for offset in 0..2 {
            let mut current = frame(day + offset, ids.last().copied().unwrap_or(0));
            put(
                &mut current,
                TableRows::Countries(
                    ids.iter()
                        .map(|&id| CountryRow {
                            id,
                            canonical_slot: id,
                            tag: format!("C{id}").into_bytes(),
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            let government = match block {
                4 => Some((2, 1)),
                9 => Some((1, 2)),
                14 => Some((3, 3)),
                _ => None,
            };
            put(
                &mut current,
                TableRows::GovernmentChanges(
                    government
                        .into_iter()
                        .map(|(country, sequence)| GovernmentChangeRow {
                            country,
                            instance: 1,
                            epoch: 1,
                            hour: day,
                            sequence,
                            cause: GovernmentCause::CoupCompletion,
                            ..Default::default()
                        })
                        .collect(),
                ),
            );
            prepared.observe_frame(&current, None, &flag).unwrap();
            previous = Some(current);
        }
        publish(&mut prepared, &fixture);
        let count = ids.len() as u64;
        assert_eq!(
            fixture
                .store
                .range_count(&fixture.key, "politics-ledger-tail-1")
                .unwrap()
                .unwrap_or(0),
            count
        );
        let mut ranges: Vec<_> = prepared.metadata.tail_ranges.values().copied().collect();
        ranges.sort_unstable();
        assert_eq!(ranges, (0..count).collect::<Vec<_>>());
        for &id in &ids {
            let actual = read(&fixture, &format!("C{id}"), 24, day + 1, "governments");
            let expected = match id {
                1 if block >= 9 => Some((240, "2")),
                2 if block >= 4 => Some((120, "1")),
                3 if block >= 14 => Some((360, "3")),
                _ => None,
            };
            assert_eq!(
                actual
                    .governments
                    .iter()
                    .map(|row| (row.hour, row.sequence.as_str()))
                    .collect::<Vec<_>>(),
                expected.into_iter().collect::<Vec<_>>()
            );
        }
        if block == 6 || block == 11 || block == 15 {
            let saved = fixture
                .store
                .read(&fixture.key, "politics-ledger-state-1", 0)
                .unwrap()
                .unwrap();
            prepared = Prepared::read(saved).unwrap();
            prepared.resume(previous.as_ref().unwrap());
        }
    }
    assert_eq!(prepared.metadata.tail_ranges["C2"], 0);
    assert_eq!(prepared.metadata.tail_ranges["C1"], 1);
    assert_eq!(prepared.metadata.tail_ranges["C3"], 2);
    for tag in ["C1", "C2"] {
        let empty = series::read_reply::<BTreeMap<i32, OpenDay>>(
            &fixture.store,
            &fixture.key,
            "politics-ledger-tail-1",
            prepared.metadata.tail_ranges[tag],
        )
        .unwrap()
        .unwrap();
        assert!(empty.is_empty(), "closed tails replace their prior rows");
    }

    let mut current = frame(408, 4);
    let mut countries = current.complete::<tables::Countries>().unwrap().to_vec();
    countries[3].tag = b"A0".to_vec();
    put(&mut current, TableRows::Countries(countries));
    put(
        &mut current,
        TableRows::GovernmentChanges(vec![GovernmentChangeRow {
            country: 4,
            instance: 1,
            epoch: 1,
            hour: 408,
            sequence: 4,
            cause: GovernmentCause::CoupCompletion,
            ..Default::default()
        }]),
    );
    prepared.observe_frame(&current, None, &flag).unwrap();
    let mut writer = fixture.append_writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_eq!(prepared.metadata.tail_ranges["A0"], 3);
    assert_eq!(
        fixture
            .store
            .range_count(&fixture.key, "politics-ledger-tail-1")
            .unwrap(),
        Some(4)
    );
    let actual = read(&fixture, "A0", 24, 408, "governments");
    assert_eq!(actual.governments.len(), 1);
    assert_eq!(actual.governments[0].country, 4);
    assert_eq!(actual.governments[0].sequence, "4");

    // Even a fresh tail section gets every saved country in range order,
    // rather than inheriting the ordinal of a scalar key or skipped block.
    let fresh = Fixture::new();
    publish(&mut prepared, &fresh);
    assert_eq!(
        fresh
            .store
            .range_count(&fresh.key, "politics-ledger-tail-1")
            .unwrap(),
        Some(4)
    );
    let tail = series::read_reply::<BTreeMap<i32, OpenDay>>(
        &fresh.store,
        &fresh.key,
        "politics-ledger-tail-1",
        prepared.metadata.tail_ranges["A0"],
    )
    .unwrap()
    .unwrap();
    assert_eq!(tail[&408].governments[0].1.sequence, "4");
}

#[test]
fn prepared_delayed_native_day_is_visible_only_after_its_supplying_read() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    for current in [daily(24, 24, false), daily(24, 28, false)] {
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 28, "spending")), 0);
    let current = daily(24, 29, true);
    prepared
        .observe_frame(&current, None, &AtomicBool::new(false))
        .unwrap();
    publish(&mut prepared, &fixture);
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 29, "spending")), 7);
    assert_eq!(spend_total(&read(&fixture, "C1", 25, 29, "spending")), 0);
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 28, "spending")), 0);
    prepared
        .observe_frame(&daily(48, 48, true), None, &AtomicBool::new(false))
        .unwrap();
    publish(&mut prepared, &fixture);
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 29, "spending")), 7);
    assert_eq!(spend_total(&read(&fixture, "C1", 25, 29, "spending")), 0);
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 28, "spending")), 0);
}

#[test]
fn prepared_late_attribution_uses_the_native_date_behind_the_closed_frontier() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let late_country = |day, read, valid| {
        let mut current = frame(read, 2);
        put(
            &mut current,
            TableRows::Countries(vec![
                CountryRow {
                    id: 1,
                    canonical_slot: 1,
                    tag: b"C1".to_vec(),
                    ..Default::default()
                },
                CountryRow {
                    id: 2,
                    canonical_slot: 2,
                    tag: if valid {
                        b"NEW".to_vec()
                    } else {
                        b"invalid\0".to_vec()
                    },
                    ..Default::default()
                },
            ]),
        );
        let first = daily(day, read, true);
        let mut days = first
            .complete::<tables::CountryPowerDaily>()
            .unwrap()
            .to_vec();
        let mut old = daily(24, read, valid)
            .complete::<tables::CountryPowerDaily>()
            .unwrap()[0]
            .clone();
        old.country = 2;
        days.push(old);
        put(&mut current, TableRows::CountryPowerDaily(days));
        let mut costs = first
            .complete::<tables::CountryPowerDailyCosts>()
            .unwrap()
            .to_vec();
        let mut cost = CountryPowerDailyCostRow {
            country: 2,
            instance: 1,
            epoch: 1,
            day_hour: 24,
            pp_sequence: 24,
            cost_raw: 7,
            cause: PowerCause::DailyGain,
            ..Default::default()
        };
        cost.ordinal = 1;
        costs.push(cost);
        put(&mut current, TableRows::CountryPowerDailyCosts(costs));
        current
    };
    for current in [late_country(48, 48, false), late_country(72, 72, false)] {
        assert!(valid_recorded_tag(&current.complete::<tables::Countries>().unwrap()[1]).is_err());
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    assert_eq!(prepared.frontier, Some(48));
    let current = late_country(72, 73, true);
    oracle.observe(&current, None).unwrap();
    prepared
        .observe_frame(&current, None, &AtomicBool::new(false))
        .unwrap();
    publish(&mut prepared, &fixture);
    assert_eq!(spend_total(&read(&fixture, "NEW", 24, 73, "spending")), 7);
    assert_eq!(spend_total(&read(&fixture, "NEW", 25, 73, "spending")), 0);
    assert_eq!(spend_total(&read(&fixture, "NEW", 24, 72, "spending")), 0);
    assert_eq!(
        spend_total(&read(&fixture, "NEW", 24, 73, "spending")),
        spend_total(
            &present_view(
                &oracle,
                1,
                "NEW",
                None,
                &AtomicBool::new(false),
                LedgerView::Spending
            )
            .unwrap()
        )
    );
}

#[test]
fn prepared_payments_match_the_oracle_across_two_commits_resume_and_append() {
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let first = engine::cdate::HOUR_EPOCH;
    for day in [first, first + 24] {
        let mut current = daily(day, day, true);
        let mut payment = event(
            day as u64 + 3,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-19),
        );
        payment.hour = day;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    let saved = fixture
        .store
        .read(&fixture.key, "politics-ledger-state-1", 0)
        .unwrap()
        .unwrap();
    prepared = Prepared::read(saved).unwrap();
    prepared.resume(&daily(first + 24, first + 24, true));
    let current = daily(first + 48, first + 48, true);
    oracle.observe(&current, None).unwrap();
    prepared
        .observe_frame(&current, None, &AtomicBool::new(false))
        .unwrap();
    let mut writer = fixture.append_writer();
    prepared
        .flush(&mut writer, 1, &AtomicBool::new(false))
        .unwrap();
    writer.commit().unwrap();
    let expected = present_view(
        &oracle,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Balance,
    )
    .unwrap();
    let actual = read(&fixture, "C1", first, first + 48, "spending");
    assert_eq!(spend_total(&actual), spend_total(&expected));
    assert_eq!(
        actual.spending,
        expected
            .spending
            .iter()
            .filter(|row| !is_recurring(&row.category))
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(actual.spend_items, expected.spend_items);
    assert_eq!(actual.spend_categories, expected.spend_categories);
    assert_eq!(actual.gains[0].gained_raw, "300");
    assert_eq!(actual.gains[0].lost_raw, "39");
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_continuation_bytes_do_not_keep_completed_payments() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut short = 0;
    for hour in 24..=24 * 400 {
        let mut current = frame(hour, 1);
        let mut payment = event(
            hour as u64,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-17),
        );
        payment.hour = hour;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        if hour == 48 || hour == 24 * 400 {
            publish(&mut prepared, &fixture);
            let bytes = prepared.saved_bytes().unwrap().len();
            if short == 0 {
                short = bytes;
            } else {
                assert!(bytes <= short + 512, "short={short}, long={bytes}");
            }
        }
    }
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_observer_and_queries_preserve_five_thousand_countries_without_caps() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    for hour in [24, 25] {
        let mut current = frame(hour, 5000);
        put(
            &mut current,
            TableRows::CountryPowerChanges(
                (1..=5000)
                    .map(|country| {
                        let mut payment = event(
                            hour as u64,
                            PowerCause::Decision,
                            PowerOperation::Subtract,
                            Some(-i64::from(country)),
                        );
                        payment.country = country;
                        payment.hour = hour;
                        payment
                    })
                    .collect(),
            ),
        );
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    assert_eq!(prepared.scan.countries.len(), 5000);
    assert_eq!(
        spend_total(&read(&fixture, "C5000", 24, 25, "spending")),
        10000
    );
    assert_eq!(read(&fixture, "C1", 24, 25, "spending").spending.len(), 2);
}

#[test]
fn prepared_command_upkeep_keeps_intraday_native_dates_and_independent_reads_after_closing() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    for (native, read, sequence, charge) in [(24, 24, 1, 10), (25, 27, 2, 20)] {
        let mut current = daily(24, read, true);
        let mut payment = event(
            sequence,
            PowerCause::AbilityUpkeep,
            PowerOperation::Subtract,
            Some(-charge),
        );
        payment.currency = PowerCurrency::Command;
        payment.hour = native;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    let current = daily(48, 48, true);
    oracle.observe(&current, None).unwrap();
    prepared
        .observe_frame(&current, None, &AtomicBool::new(false))
        .unwrap();
    publish(&mut prepared, &fixture);
    let total = |from, to| {
        read(&fixture, "C1", from, to, "spending")
            .recurring
            .into_iter()
            .filter(|row| row.spend.currency == "Command")
            .map(|row| row.known_raw.parse::<i128>().unwrap())
            .sum::<i128>()
    };
    assert_eq!(total(24, 24), 10);
    assert_eq!(total(24, 26), 10);
    assert_eq!(total(24, 27), 30);
    assert_eq!(total(25, 27), 20);
    assert_eq!(total(26, 27), 0);
    let view = read(&fixture, "C1", 24, 27, "spending");
    let upkeep: Vec<_> = view
        .recurring
        .iter()
        .filter(|row| row.spend.category == "upkeep")
        .collect();
    assert_eq!(upkeep.len(), 1);
    assert_eq!(upkeep[0].count, 1);
    assert_eq!(upkeep[0].spend.occurrences, 2);
    assert_eq!(
        present_view(
            &oracle,
            1,
            "C1",
            None,
            &AtomicBool::new(false),
            LedgerView::Spending
        )
        .unwrap()
        .spending
        .iter()
        .find(|row| row.category == "upkeep")
        .unwrap()
        .charge_raw
        .as_deref(),
        Some("30")
    );
}

#[test]
fn prepared_item_endpoints_match_window_payments_without_reading_daily_applications() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    for (native, read, sequence) in [(24, 24, 1), (25, 28, 2), (26, 29, 3)] {
        let mut current = frame(read, 1);
        let mut payment = event(
            sequence,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-17),
        );
        payment.hour = native;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    for (from, to, expected) in [(24, 29, 51), (25, 29, 34), (26, 29, 17), (25, 27, 0)] {
        let reply: PoliticsLedgerReply = serde_json::from_value(query::query(&json!({"branchId":"1", "tag":"C1", "view":"spending", "from":from, "hour":to, "totals":true}),
            &fixture.store, &fixture.key, &AtomicBool::new(false)).unwrap()).unwrap();
        assert_eq!(spend_total(&reply), expected);
    }
}

#[test]
fn prepared_wide_payment_totals_round_trip_events_and_resume_as_decimal_strings() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for hour in [24, 25] {
        let mut current = frame(hour, 1);
        let mut payment = event(
            hour as u64,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-i64::MAX),
        );
        payment.hour = hour;
        put(&mut current, TableRows::CountryPowerChanges(vec![payment]));
        prepared.observe_frame(&current, None, &flag).unwrap();
        publish(&mut prepared, &fixture);
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&current);
    }
    let expected = i128::from(i64::MAX) * 2;
    assert_eq!(
        spend_total(&read(&fixture, "C1", 24, 25, "spending")),
        expected
    );
    let aggregate = query::query(
        &json!({"branchId":"1", "tag":"C1", "view":"spending", "from":24, "hour":25, "totals":true}),
        &fixture.store,
        &fixture.key,
        &flag,
    ).unwrap();
    let aggregate: PoliticsLedgerReply = serde_json::from_value(aggregate).unwrap();
    assert_eq!(spend_total(&aggregate), expected);
    assert_eq!(aggregate.recurring.len(), 1);
    assert_eq!(aggregate.recurring[0].spend.occurrences, 2);

    // Tagged event deserialization buffers through serde's value representation;
    // both bounds must remain strings there, rather than unsupported i128 values.
    let payment = prepared
        .totals
        .values()
        .next()
        .unwrap()
        .latest
        .clone()
        .unwrap();
    for known in [i128::MIN, i128::MAX] {
        let value = serde_json::to_value(Event::Totals {
            payment: payment.clone(),
            totals: Totals {
                known,
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(value["totals"]["known"], known.to_string());
        let decoded: Event = serde_json::from_value(value.clone()).unwrap();
        let from_bytes: Event =
            serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        for event in [decoded, from_bytes] {
            let Event::Totals { totals, .. } = event else {
                panic!("totals event expected")
            };
            assert_eq!(totals.known, known);
        }
    }
}

#[test]
fn prepared_government_native_marks_obey_independent_reads_and_late_attribution() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let government = |hour, sequence| GovernmentChangeRow {
        country: 1,
        instance: 1,
        epoch: 1,
        hour,
        sequence,
        cause: GovernmentCause::CoupCompletion,
        ..Default::default()
    };
    for (day, read, pp, row) in [
        (24, 24, false, None),
        (24, 28, false, Some(government(24, 1))),
        (24, 29, true, Some(government(25, 2))),
        (48, 48, true, None),
        (48, 49, true, Some(government(24, 3))),
    ] {
        let mut current = daily(day, read, pp);
        put(
            &mut current,
            TableRows::GovernmentChanges(row.into_iter().collect()),
        );
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    assert_eq!(
        read(&fixture, "C1", 24, 27, "governments")
            .governments
            .len(),
        0
    );
    assert_eq!(
        read(&fixture, "C1", 24, 28, "governments")
            .governments
            .iter()
            .map(|row| (row.hour, row.read_hour))
            .collect::<Vec<_>>(),
        [(24, 28)]
    );
    assert_eq!(
        read(&fixture, "C1", 25, 29, "governments")
            .governments
            .iter()
            .map(|row| row.hour)
            .collect::<Vec<_>>(),
        [25]
    );
    let all = read(&fixture, "C1", 24, 49, "governments");
    assert_eq!(
        all.governments
            .iter()
            .map(|row| (row.hour, row.read_hour, row.sequence.as_str()))
            .collect::<Vec<_>>(),
        [(24, 28, "1"), (25, 29, "2"), (24, 49, "3")]
    );
    assert_eq!(
        all.governments,
        present_view(
            &oracle,
            1,
            "C1",
            None,
            &AtomicBool::new(false),
            LedgerView::Governments
        )
        .unwrap()
        .governments
    );
    let marks: PoliticsLedgerReply = serde_json::from_value(query::query(&json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":28, "marks":{"from":24,"to":28,"width":18}}), &fixture.store, &fixture.key, &AtomicBool::new(false)).unwrap()).unwrap();
    let clusters = &marks.government_marks.unwrap().clusters;
    assert_eq!(clusters.len(), 1);
    assert_eq!(
        (
            clusters[0].first_hour,
            clusters[0].last_hour,
            clusters[0].count.as_str()
        ),
        (24, 24, "1")
    );
    assert!(marks.governments.is_empty());
    let marks: PoliticsLedgerReply = serde_json::from_value(query::query(&json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":49, "marks":{"from":24,"to":49,"width":18}}), &fixture.store, &fixture.key, &AtomicBool::new(false)).unwrap()).unwrap();
    assert_eq!(marks.government_marks.unwrap().clusters[0].count, "2");
    assert_eq!(marks.governments.len(), 1);
    assert_eq!(marks.governments[0].sequence, "3");
    assert_eq!(spend_total(&read(&fixture, "C1", 24, 29, "spending")), 7);
    assert_eq!(spend_total(&read(&fixture, "C1", 25, 29, "spending")), 0);
}

#[test]
fn prepared_raw_rows_match_the_accounting_after_two_commits() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    for day in [24, 48, 72] {
        let mut current = daily(day, day, true);
        let mut row = event(
            day as u64,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-19),
        );
        row.hour = day;
        put(&mut current, TableRows::CountryPowerChanges(vec![row]));
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    let expected = present_view(
        &oracle,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Raw,
    )
    .unwrap();
    let actual = read(&fixture, "C1", 24, 72, "raw");
    assert_eq!(actual.mutations, expected.mutations);
    assert_eq!(actual.daily, expected.daily);
    assert_eq!(actual.daily_costs, expected.daily_costs);
    assert_eq!(actual.coverage, expected.coverage);
    assert_eq!(actual.spending_coverage, expected.spending_coverage);
    assert_eq!(actual.warnings, expected.warnings);
}

#[test]
fn prepared_spending_coverage_uses_each_read_after_gaps_across_commit_resume_and_append() {
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let flag = AtomicBool::new(false);
    for hour in [24, 48, 72] {
        // No accepted frames supply the intervening hours. Complete reads on
        // both sides have identical flags and the same coverage reason.
        let current = frame(hour, 1);
        oracle.observe(&current, None).unwrap();
        prepared.observe_frame(&current, None, &flag).unwrap();
        if hour == 72 {
            let mut writer = fixture.append_writer();
            prepared.flush(&mut writer, 1, &flag).unwrap();
            writer.commit().unwrap();
        } else {
            publish(&mut prepared, &fixture);
        }
        let expected = present_view(&oracle, 1, "C1", None, &flag, LedgerView::Raw).unwrap();
        for view in ["raw", "spending", "balance"] {
            let actual = read(&fixture, "C1", 24, hour, view);
            assert_eq!(
                actual.spending_coverage, expected.spending_coverage,
                "{view} through {hour}"
            );
            assert_eq!(
                actual
                    .spending_coverage
                    .iter()
                    .filter(|row| row.political && row.command)
                    .map(|row| (row.read_hour, row.since_hour, row.through_hour))
                    .collect::<Vec<_>>(),
                [24, 48, 72]
                    .into_iter()
                    .filter(|read| *read <= hour)
                    .map(|read| (read, Some(read), Some(read)))
                    .collect::<Vec<_>>()
            );
        }
        let mut reader = Reader::new(
            &fixture.store,
            &fixture.key,
            "politics-ledger-1".into(),
            &flag,
        )
        .unwrap();
        let reason = prepared.keys[&series::field_key("C1", "coverage-reason")];
        assert_eq!(
            reader.event_count_through(reason, hour).unwrap(),
            1,
            "unchanged coverage reasons remain change-proportional"
        );
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&current);
    }
}

#[test]
fn prepared_closed_native_totals_stop_before_the_open_day_across_resume_and_append() {
    let mut fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let flag = AtomicBool::new(false);
    for day in [24, 48, 72] {
        let current = daily(day, day, true);
        prepared.observe_frame(&current, None, &flag).unwrap();
        publish(&mut prepared, &fixture);
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&current);
    }
    assert_eq!(prepared.metadata.native_through, Some(71));
    let check = |fixture: &Fixture, to: i32, days: i128| {
        for view in ["raw", "balance", "spending"] {
            let reply = read(fixture, "C1", 24, to, view);
            assert_eq!(spend_total(&reply), days * 7, "{view} through {to}");
            assert_eq!(
                reply
                    .gains
                    .iter()
                    .find(|row| row.currency == "Political")
                    .unwrap()
                    .gained_raw,
                (days * 100).to_string()
            );
        }
    };
    check(&fixture, 48, 2);
    check(&fixture, 72, 3);
    let current = daily(96, 96, true);
    prepared.observe_frame(&current, None, &flag).unwrap();
    let mut writer = fixture.append_writer();
    prepared.flush(&mut writer, 1, &flag).unwrap();
    writer.commit().unwrap();
    assert_eq!(prepared.metadata.native_through, Some(95));
    check(&fixture, 72, 3);
    check(&fixture, 96, 4);
}

#[test]
fn prepared_capture_identities_keep_raw_tuples_and_refuse_ambiguous_income() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    for read in (24..=29).chain([48]) {
        let instance = if read >= 29 { 2 } else { 1 };
        let day = if read == 48 { 48 } else { 24 };
        let mut current = daily(day, read, true);
        let mut days = current
            .complete::<tables::CountryPowerDaily>()
            .unwrap()
            .to_vec();
        days[0].instance = instance;
        let mut costs = current
            .complete::<tables::CountryPowerDailyCosts>()
            .unwrap()
            .to_vec();
        costs[0].instance = instance;
        put(&mut current, TableRows::CountryPowerDaily(days));
        put(&mut current, TableRows::CountryPowerDailyCosts(costs));
        put(
            &mut current,
            TableRows::PoliticsCaptureCoverage(vec![PoliticsCaptureCoverageRow {
                country: 1,
                instance,
                epoch: 1,
                since_hour: day,
                through_hour: read,
                streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
            }]),
        );
        oracle.observe(&current, None).unwrap();
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    let actual = read(&fixture, "C1", 24, 29, "balance");
    assert_eq!(actual.daily.len(), 2);
    assert_eq!(actual.daily[0].instance, 1);
    assert_eq!(actual.daily[1].instance, 2);
    assert_eq!(
        actual
            .gains
            .iter()
            .find(|row| row.currency == "Political")
            .unwrap()
            .gained_raw,
        "200"
    );
    assert_eq!(spend_total(&actual), 14);
    assert_eq!(read(&fixture, "C1", 24, 28, "balance").daily.len(), 1);
    let summaries: PoliticsLedgerReply = serde_json::from_value(query::query(&json!({"branchId":"1", "tag":"C1", "summary":{"from":24,"to":30,"width":7,"currency":"Political","mode":"income"}}), &fixture.store, &fixture.key, &AtomicBool::new(false)).unwrap()).unwrap();
    let total = &summaries
        .part_summaries
        .iter()
        .find(|part| part.id == "income-total")
        .unwrap()
        .summary;
    assert_eq!(total.runs.as_ref().unwrap()[0].last_hour, 28);
    assert!(
        total
            .runs
            .as_ref()
            .unwrap()
            .iter()
            .all(|run| run.last_hour < 29)
    );
    let expected = present_view(
        &oracle,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Spending,
    )
    .unwrap();
    let actual = read(&fixture, "C1", 24, 48, "spending");
    assert_eq!(actual.spend_items, expected.spend_items);
    assert_eq!(spend_total(&actual), spend_total(&expected));
}

#[test]
fn prepared_government_pages_and_navigation_keep_duplicate_native_dates_and_visibility() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    for (day, read, rows) in [
        (24, 24, vec![(24, 1), (24, 2)]),
        (24, 29, vec![(25, 3)]),
        (48, 48, Vec::new()),
    ] {
        let mut current = daily(day, read, true);
        put(
            &mut current,
            TableRows::GovernmentChanges(
                rows.into_iter()
                    .map(|(hour, sequence)| GovernmentChangeRow {
                        country: 1,
                        instance: 1,
                        epoch: 1,
                        hour,
                        sequence,
                        cause: GovernmentCause::CoupCompletion,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        publish(&mut prepared, &fixture);
    }
    let call = |params| {
        query::query(
            &params,
            &fixture.store,
            &fixture.key,
            &AtomicBool::new(false),
        )
        .unwrap()
    };
    let first = call(
        json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":29, "page":{"from":24,"to":25,"cursor":"0","pageSize":1}}),
    );
    assert_eq!(first["rows"][0]["row"]["sequence"], "1");
    assert_eq!(first["cursor"], "1");
    let second = call(
        json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":29, "page":{"from":24,"to":25,"cursor":"1","pageSize":1}}),
    );
    assert_eq!(second["rows"][0]["row"]["sequence"], "2");
    for (key, from, through, expected) in [
        ("Home", None, 24, 24),
        ("End", None, 28, 24),
        ("End", None, 29, 25),
        ("ArrowRight", Some(24), 29, 25),
    ] {
        let value = call(
            json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":through, "navigation":{"key":key,"from":from,"firstHour":24,"lastHour":through}}),
        );
        assert_eq!(value["hour"], f64::from(expected));
    }
    // A cause native to hour 25 is also reached to suppress its census at 29
    // when an exact viewport starts after that native hour.
    let exact: PoliticsLedgerReply = serde_json::from_value(call(json!({"branchId":"1", "tag":"C1", "view":"governments", "from":28,"hour":29,"matches":true}))).unwrap();
    assert_eq!(exact.governments.len(), 1);
    assert_eq!(
        (exact.governments[0].hour, exact.governments[0].read_hour),
        (25, 29)
    );
}

#[test]
fn prepared_government_navigation_keeps_adjacent_hours_with_equal_supplying_lags() {
    let fixture = Fixture::new();
    let flag = AtomicBool::new(false);
    let mut prepared = Prepared::default();
    for hour in [24_i32, 25, 48] {
        let mut current = daily(hour.div_euclid(24) * 24, hour, true);
        if hour < 48 {
            put(
                &mut current,
                TableRows::GovernmentChanges(vec![GovernmentChangeRow {
                    country: 1,
                    instance: 1,
                    epoch: 1,
                    hour,
                    sequence: hour as u64,
                    cause: GovernmentCause::CoupCompletion,
                    ..Default::default()
                }]),
            );
        }
        prepared.observe_frame(&current, None, &flag).unwrap();
        publish(&mut prepared, &fixture);
        prepared = Prepared::read(prepared.saved_bytes().unwrap().as_slice()).unwrap();
        prepared.resume(&current);
    }
    for (key, from, expected) in [
        ("Home", None, 24),
        ("End", None, 25),
        ("ArrowRight", Some(24), 25),
        ("ArrowLeft", Some(25), 24),
        ("ArrowRight", Some(25), 25),
    ] {
        let reply = query::query(
            &json!({"branchId":"1", "tag":"C1", "view":"governments", "hour":48,
                "navigation":{"key":key, "from":from, "firstHour":24, "lastHour":48}}),
            &fixture.store,
            &fixture.key,
            &flag,
        )
        .unwrap();
        assert_eq!(reply["hour"], f64::from(expected), "{key} from {from:?}");
    }
}

#[test]
fn prepared_month_folds_include_only_partial_selected_months_and_each_category_day_once() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let first = engine::cdate::HOUR_EPOCH;
    for index in 0..=45 {
        let day = first + index * 24;
        let mut current = daily(day, day, true);
        let mut costs = current
            .complete::<tables::CountryPowerDailyCosts>()
            .unwrap()
            .to_vec();
        let mut other = costs[0].clone();
        other.ordinal = 1;
        other.cost_raw = 3;
        costs[0].cost_raw = 4;
        costs.push(other);
        put(&mut current, TableRows::CountryPowerDailyCosts(costs));
        prepared
            .observe_frame(&current, None, &AtomicBool::new(false))
            .unwrap();
        if index == 23 || index == 45 {
            publish(&mut prepared, &fixture);
        }
    }
    let reply = read(
        &fixture,
        "C1",
        first + 15 * 24 + 1,
        first + 40 * 24 + 5,
        "spending",
    );
    assert_eq!(spend_total(&reply), 25 * 7);
    let days: BTreeMap<_, _> = reply
        .recurring_days
        .iter()
        .filter(|row| row.currency == "Political")
        .map(|row| (row.month, row.days))
        .collect();
    assert_eq!(days.get(&Some(0)), Some(&15));
    assert_eq!(days.get(&Some(1)), Some(&10));
    assert_eq!(reply.recurring.iter().map(|row| row.count).sum::<u32>(), 50);
}

#[test]
fn prepared_unresolved_cost_warning_disappears_when_its_application_arrives() {
    let fixture = Fixture::new();
    let mut prepared = Prepared::default();
    let mut oracle = Scanned::default();
    let mut first = daily(24, 24, false);
    put(
        &mut first,
        daily(24, 24, true)
            .complete::<tables::CountryPowerDailyCosts>()
            .map(|rows| TableRows::CountryPowerDailyCosts(rows.to_vec()))
            .unwrap(),
    );
    prepared
        .observe_frame(&first, None, &AtomicBool::new(false))
        .unwrap();
    oracle.observe(&first, None).unwrap();
    publish(&mut prepared, &fixture);
    assert!(
        read(&fixture, "C1", 24, 24, "raw")
            .warnings
            .iter()
            .any(|reason| reason.contains("matched PP application is unavailable"))
    );
    let second = daily(24, 29, true);
    prepared
        .observe_frame(&second, None, &AtomicBool::new(false))
        .unwrap();
    oracle.observe(&second, None).unwrap();
    publish(&mut prepared, &fixture);
    let actual = read(&fixture, "C1", 24, 29, "raw");
    let expected = present_view(
        &oracle,
        1,
        "C1",
        None,
        &AtomicBool::new(false),
        LedgerView::Raw,
    )
    .unwrap();
    assert_eq!(actual.warnings, expected.warnings);
    assert!(
        actual
            .warnings
            .iter()
            .all(|reason| !reason.contains("matched PP application is unavailable"))
    );
}
