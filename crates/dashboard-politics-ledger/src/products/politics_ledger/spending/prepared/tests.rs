use super::super::tests::{event, frame, put};
use super::*;
use dashboard_base::prepared::series::tests::Fixture;
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
    use dashboard_base::prepared::observer_tests::{self, Work};
    use replay::format::FormatVersion;

    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for ledger content binding");
    for complete in [true, false] {
        // The store owns only prepared files; its cleanup deletes file entries.
        // Synthetic game directories have their own lifetime outside that store.
        let folder = dashboard_text::catalogue::tests::Folder(
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
            dashboard_base::test_fixtures::source_with_content(vec![first.clone()], game);
        let options = context.content.as_ref();
        let binding = prepared_content(&source, options);
        assert_eq!(binding.is_ok(), complete);
        let refusal = binding.as_ref().err().cloned();
        assert_eq!(
            dashboard_base::products::politics_content::content_loads(&source),
            1 + usize::from(complete)
        );
        let repeated = prepared_content(&source, options);
        assert_eq!(repeated.as_ref().err(), refusal.as_ref());
        assert_eq!(
            dashboard_base::products::politics_content::content_loads(&source),
            1 + usize::from(complete)
        );
        observer_tests::scaled_work(10, |hours| {
            let mut prepared = Prepared::default();
            prepared.configure(&binding);
            let mut tracker = dashboard_base::prepared::frame_changes::FrameChangeTracker::new();
            prepared.observe(
                &first,
                &tracker.observe(&first, FormatVersion::CURRENT),
                &AtomicBool::new(false),
            )?;
            let configured = Work {
                content_loads: dashboard_base::products::politics_content::content_loads(&source),
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
                        content_loads: dashboard_base::products::politics_content::content_loads(
                            &source,
                        ),
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
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
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
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

/// The recording codec: each input hour is encoded and decoded as the
/// preparation pipeline reads it, with its recorded updates.
struct Recorded {
    encoder: replay::frame::FrameEncoder,
    decoder: replay::frame::FrameDecoder,
}
impl Recorded {
    fn new() -> Self {
        Self {
            encoder: replay::frame::FrameEncoder::new(),
            decoder: replay::frame::FrameDecoder::new(),
        }
    }
    /// The decoded hour and its updates. Every hourly politics table is
    /// read, as a live recording reads them, whether or not it changed.
    fn next(
        &mut self,
        input: &mut DecodedFrame,
        keyframe: bool,
    ) -> (DecodedFrame, Arc<replay::frame::Frame>) {
        for table in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::PoliticsCaptureCoverage,
            TableId::CountryPowerChanges,
            TableId::GovernmentChanges,
            TableId::CountryCommandPower,
            TableId::Characters,
        ] {
            input.metrics.table_ns[table.index()] = replay::frame::READ_TIME_NOT_RECORDED;
        }
        let wire = self.encoder.encode(input.clone(), keyframe).unwrap();
        let decoded = self.decoder.advance(&wire).unwrap().clone();
        (decoded, Arc::new(wire))
    }
}

fn capture(country: u32, since: i32, through: i32) -> PoliticsCaptureCoverageRow {
    PoliticsCaptureCoverageRow {
        country,
        instance: 1,
        epoch: 1,
        since_hour: since,
        through_hour: through,
        streams: PoliticsCaptureCoverageRow::ALL_STREAMS,
    }
}

fn command_power(count: u32) -> TableRows {
    TableRows::CountryCommandPower(
        (1..=count)
            .map(|canonical_slot| CountryCommandPowerRow {
                canonical_slot,
                ..Default::default()
            })
            .collect(),
    )
}

fn payment(country: u32, sequence: u64, hour: i32) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country,
        hour,
        ..event(
            sequence,
            PowerCause::Decision,
            PowerOperation::Subtract,
            Some(-17),
        )
    }
}

/// Every capture extends to the hour, as a live recording's do.
fn extend_captures(input: &mut DecodedFrame) {
    let hour = input.hour;
    let captures = input
        .complete::<tables::PoliticsCaptureCoverage>()
        .unwrap()
        .iter()
        .map(|row| PoliticsCaptureCoverageRow {
            through_hour: hour,
            ..row.clone()
        })
        .collect();
    put(input, TableRows::PoliticsCaptureCoverage(captures));
}

/// Each observer's countries observed, rows read, countries presented and
/// hours joined from changed rows, over one recorded hour.
type HourWork = (usize, usize, usize, usize);

/// The work of hours whose captures all extend to the hour, with one
/// country's rows changed at a time, joined from changed rows and from the
/// whole tables.
fn assert_hour_observes_only_changed_countries(count: u32) {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let format = replay::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let mut recorded = Recorded::new();
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut observers = [Prepared::default(), Prepared::default()];
    observers[1].scan.whole_tables = true;
    let mut input = frame(24, count);
    put(&mut input, command_power(count));
    let mut step = |observers: &mut [Prepared; 2], input: &mut DecodedFrame| -> [HourWork; 2] {
        extend_captures(input);
        let keyframe = input.hour == 24;
        let (decoded, wire) = recorded.next(input, keyframe);
        let mut work = [(0, 0, 0, 0); 2];
        for ((observer, tracker), work) in observers.iter_mut().zip(&mut trackers).zip(&mut work) {
            let before = (
                observer.scan.country_visits,
                observer.measured_work(),
                observer.scan.row_hours,
            );
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
            observer.observe(&decoded, &changes, &flag).unwrap();
            let after = observer.measured_work();
            *work = (
                observer.scan.country_visits - before.0,
                after.row_visits - before.1.row_visits,
                after.history_visits - before.1.history_visits,
                observer.scan.row_hours - before.2,
            );
        }
        work
    };
    let every = count as usize;
    let [first, whole] = step(&mut observers, &mut input);
    assert_eq!(first.0, every, "the first hour observes every country");
    assert_eq!(whole.0, every);

    input.hour = 25;
    let [rows, whole] = step(&mut observers, &mut input);
    assert_eq!(rows.3, 1, "the hour is joined from its changed rows");
    assert_eq!(
        (rows.0, rows.2),
        (0, 0),
        "captures extended to the hour observe no country"
    );
    assert_eq!(rows.1, every, "only the changed captures are read");
    assert_eq!(whole.0, every, "whole tables observe every country again");

    // One country's payment.
    input.hour = 26;
    let paying = count / 2;
    put(
        &mut input,
        TableRows::CountryPowerChanges(vec![payment(paying, 1, 26)]),
    );
    let [rows, whole] = step(&mut observers, &mut input);
    assert_eq!(rows.3, 1);
    assert_eq!(
        (rows.0, rows.2),
        (1, 1),
        "only the paying country is observed and presented"
    );
    assert_eq!(rows.1, every + 1);
    assert_eq!(whole.0, every);

    // A balance no join reads.
    input.hour = 27;
    let mut countries = input.complete::<tables::Countries>().unwrap().to_vec();
    countries[(count / 3) as usize].political_power_raw = Some(27);
    put(&mut input, TableRows::Countries(countries));
    let [rows, _] = step(&mut observers, &mut input);
    assert_eq!(rows.3, 1);
    assert_eq!((rows.0, rows.2), (0, 0));

    // One capture loses a stream.
    input.hour = 28;
    let changed = count / 4;
    let captures = input
        .complete::<tables::PoliticsCaptureCoverage>()
        .unwrap()
        .iter()
        .map(|row| PoliticsCaptureCoverageRow {
            streams: if row.country == changed {
                PoliticsCaptureCoverageRow::PP
            } else {
                row.streams
            },
            ..row.clone()
        })
        .collect();
    put(&mut input, TableRows::PoliticsCaptureCoverage(captures));
    let [rows, _] = step(&mut observers, &mut input);
    assert_eq!(rows.3, 1);
    assert_eq!(rows.0, 1, "only the changed capture's country is observed");
    assert_eq!(observers[1].scan.row_hours, 0);
}

#[test]
fn prepared_hour_changing_one_country_observes_only_that_country() {
    assert_hour_observes_only_changed_countries(200);
}

#[test]
#[cfg(feature = "slow-tests")]
fn prepared_hour_changing_one_of_5000_countries_observes_only_that_country() {
    assert_hour_observes_only_changed_countries(5000);
}

/// One fixture hour's edits: payments, daily gains of a captured identity
/// and of one no capture names, a balance, a restarted capture, a government, a
/// leader's command power allocation, a capture that stops, is deleted and
/// starts again, and a payment that leaves the published window.
fn edit_hour(
    input: &mut DecodedFrame,
    captures: &mut Vec<PoliticsCaptureCoverageRow>,
    power: &mut Vec<CountryPowerChangeRow>,
) {
    let hour = input.hour;
    match hour {
        26 | 32 => power.push(payment(1, hour as u64, hour)),
        27 => power.push(payment(3, 27, 27)),
        28 => {
            let mut countries = input.complete::<tables::Countries>().unwrap().to_vec();
            countries[0].political_power_raw = Some(28);
            put(input, TableRows::Countries(countries));
        }
        29 | 31 => power.push(CountryPowerChangeRow {
            country: if hour == 29 { 2 } else { 1 },
            instance: if hour == 29 { 1 } else { 9 },
            hour,
            currency: PowerCurrency::Political,
            balance_after_raw: 510,
            ..event(
                hour as u64,
                PowerCause::DailyGain,
                PowerOperation::Add,
                Some(500),
            )
        }),
        // A capture that starts again rather than extending its interval.
        33 => captures[2].since_hour = 33,
        34 => put(
            input,
            TableRows::GovernmentChanges(vec![GovernmentChangeRow {
                country: 1,
                instance: 1,
                epoch: 1,
                sequence: 34,
                hour: 34,
                before_party: Some(1),
                after_party: Some(2),
                ..Default::default()
            }]),
        ),
        37 => {
            power.push(CountryPowerChangeRow {
                country: 2,
                instance: 1,
                epoch: 1,
                hour,
                sequence: 37,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Subtract,
                requested_delta_raw: Some(-300_000),
                cause: PowerCause::LeaderAssignment,
                site: PowerSite::AllocateCommandPower,
                balance_before_raw: 1_000_000,
                balance_after_raw: 700_000,
                identity: PowerIdentity {
                    object_kind: PowerObjectKind::Leader,
                    object_reference: Some((7_u64 << 32) | 2),
                    ..Default::default()
                },
            });
            let mut command = input
                .complete::<tables::CountryCommandPower>()
                .unwrap()
                .to_vec();
            command[1].allocated_command_power_raw = 300_000;
            put(input, TableRows::CountryCommandPower(command));
        }
        40 => captures.retain(|row| row.country != 4),
        41 => captures.push(capture(4, 41, 41)),
        45 => power.retain(|row| (row.country, row.sequence) != (1, 26)),
        // A daily gain leaves the published window too.
        46 => power.retain(|row| (row.country, row.sequence) != (2, 29)),
        _ => {}
    }
    for row in captures.iter_mut() {
        if row.country != 4 || !(39..=40).contains(&hour) {
            row.through_hour = hour;
        }
    }
    power.sort_by_key(|row| (row.country, row.sequence));
    put(input, TableRows::PoliticsCaptureCoverage(captures.clone()));
    put(input, TableRows::CountryPowerChanges(power.clone()));
}

#[test]
fn prepared_changed_rows_match_whole_tables_and_oracle_across_keyframe_gap_and_resume() {
    use dashboard_base::prepared::{frame_changes::FrameChangeTracker, observer_tests};
    let format = replay::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let fixtures = [Fixture::new(), Fixture::new()];
    // Joined from changed rows, and from the whole tables every hour.
    let mut observers = [Prepared::default(), Prepared::default()];
    observers[1].scan.whole_tables = true;
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut recorded = Recorded::new();
    let mut oracle = Scanned::default();
    let mut input = frame(24, 4);
    put(&mut input, command_power(4));
    let mut captures: Vec<_> = (1..=4).map(|country| capture(country, 24, 24)).collect();
    let mut power = Vec::new();
    // Hour 42 is a recording gap; hour 38 a keyframe.
    for hour in (24..=47).filter(|&hour| hour != 42) {
        input.hour = hour;
        edit_hour(&mut input, &mut captures, &mut power);
        let (decoded, wire) = recorded.next(&mut input, hour == 24 || hour == 38);
        oracle.observe(&decoded, None).unwrap();
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
            observer.observe(&decoded, &changes, &flag).unwrap();
        }
        // The events each publishes and their native days are kept by country.
        let [rows, whole] = &observers;
        assert_eq!(
            rows.scan.published_events, whole.scan.published_events,
            "hour {hour}"
        );
        assert_eq!(rows.scan.events, whole.scan.events, "hour {hour}");
        assert_eq!(
            rows.scan.inputs.native_days(),
            whole.scan.inputs.native_days(),
            "hour {hour}"
        );
        if ![35, 44, 47].contains(&hour) {
            continue;
        }
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            publish(observer, fixture);
        }
        for tag in ["C1", "C2", "C3", "C4"] {
            for view in ["raw", "spending", "balance", "governments"] {
                assert_eq!(
                    read(&fixtures[0], tag, 24, hour, view),
                    read(&fixtures[1], tag, 24, hour, view),
                    "{tag} {view} published through hour {hour}"
                );
            }
        }
        if hour == 47 {
            break;
        }
        // Resume both from their checkpoints.
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let (whole_tables, row_hours) = (observer.scan.whole_tables, observer.scan.row_hours);
            *observer = Prepared::read(observer.saved_bytes().unwrap().as_slice()).unwrap();
            observer.scan.whole_tables = whole_tables;
            observer.scan.row_hours = row_hours;
            observer.resume(&decoded);
            tracker.invalidate();
        }
    }
    assert!(
        observers[0].scan.row_hours >= 15,
        "most hours are joined from their changed rows, not {}",
        observers[0].scan.row_hours
    );
    assert_eq!(observers[1].scan.row_hours, 0);
    for tag in ["C1", "C2", "C3", "C4"] {
        let actual = read(&fixtures[0], tag, 24, 47, "raw");
        let expected = present_view(&oracle, 1, tag, None, &flag, LedgerView::Raw).unwrap();
        observer_tests::assert_reply_parity(&actual.coverage, &expected.coverage).unwrap();
        observer_tests::assert_reply_parity(&actual.spending_coverage, &expected.spending_coverage)
            .unwrap();
        observer_tests::assert_reply_parity(&actual.command_holds, &expected.command_holds)
            .unwrap();
        // Preparation presents each country with its balance changes, so its
        // warnings count their reasons too (here the command power hold's
        // unresolved holder item), as the accounting's balance view does.
        let expected = present_view(&oracle, 1, tag, None, &flag, LedgerView::Balance).unwrap();
        observer_tests::assert_reply_parity(&actual.warnings, &expected.warnings).unwrap();
    }
}

/// A ledger preparing from `replay`, as the pipeline configures it.
fn prepared_from(replay: &Arc<replay::container::Replay>) -> Prepared {
    let mut prepared = Prepared::default();
    prepared.decode_previous_from(Arc::clone(replay));
    prepared
}

#[test]
fn prepared_holds_only_the_naming_tables_of_the_previous_read() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    // Each hour's tables are their own allocations.
    let hour_read = |hour: i32| {
        let mut read = frame(hour, 3);
        put(
            &mut read,
            TableRows::Characters(vec![CharacterRow {
                id: 5,
                token: b"Recorded character".to_vec(),
                ..Default::default()
            }]),
        );
        read
    };
    let replay = Arc::new(
        replay::container::Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(
                1,
                vec![hour_read(24), hour_read(25)],
            )],
        )
        .unwrap(),
    );
    let mut prepared = prepared_from(&replay);
    let mut tracker = FrameChangeTracker::new();
    for hour in [24, 25] {
        let current = hour_read(hour);
        prepared
            .observe(
                &current,
                &tracker.observe(&current, FormatVersion::CURRENT),
                &flag,
            )
            .unwrap();
        let views = [
            TableId::Characters,
            TableId::PoliticsCaptureCoverage,
            TableId::Countries,
        ]
        .map(|table| current.tables[table.index()].clone());
        drop(current);
        let [characters, coverage, countries] = views;
        // Only this test holds a table outside the naming set, so the
        // decoder could change it in place for the next read.
        assert!(characters.try_unwrap().is_ok(), "hour {hour}");
        assert!(coverage.try_unwrap().is_ok(), "hour {hour}");
        assert!(
            countries.try_unwrap().is_err(),
            "the next read names its countries from this read"
        );
    }
    assert_eq!(prepared.scan.reads_again, 0);
}

#[test]
fn prepared_payment_dated_at_the_previous_read_is_described_from_that_read_decoded_again() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    use replay::format::FormatVersion;
    let flag = AtomicBool::new(false);
    let mut first = frame(24, 1);
    put(
        &mut first,
        TableRows::Characters(vec![CharacterRow {
            id: 5,
            token: b"Recorded leader".to_vec(),
            ..Default::default()
        }]),
    );
    put(
        &mut first,
        TableRows::UnitLeaders(vec![UnitLeaderRow {
            id: 7,
            reference_type: 2,
            character_id: 5,
            category: 1,
            ..Default::default()
        }]),
    );
    // The next read publishes a payment dated at the first read, after its
    // leader and character left.
    let mut second = frame(25, 1);
    put(&mut second, TableRows::Characters(Vec::new()));
    put(&mut second, TableRows::UnitLeaders(Vec::new()));
    put(
        &mut second,
        TableRows::CountryPowerChanges(vec![CountryPowerChangeRow {
            hour: 24,
            identity: PowerIdentity {
                object_kind: PowerObjectKind::Leader,
                object_reference: Some((7_u64 << 32) | 2),
                ..Default::default()
            },
            ..event(3, PowerCause::Decision, PowerOperation::Subtract, Some(-19))
        }]),
    );
    let replay = Arc::new(
        replay::container::Replay::from_records(
            dashboard_base::test_fixtures::CAMPAIGN.into(),
            &[dashboard_base::test_fixtures::records(
                1,
                vec![first, second],
            )],
        )
        .unwrap(),
    );
    let fixtures = [Fixture::new(), Fixture::new()];
    // Decoding the previous read again, and holding it whole.
    let mut observers = [prepared_from(&replay), Prepared::default()];
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut oracle = Scanned::default();
    for hour in [24, 25] {
        let replay::container::SeekResult::Frame(decoded) = replay.seek(1, hour).unwrap() else {
            panic!("hour {hour} is recorded");
        };
        oracle.observe(&decoded, None).unwrap();
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe(&decoded, FormatVersion::CURRENT);
            observer.observe(&decoded, &changes, &flag).unwrap();
        }
    }
    assert_eq!(observers[0].scan.reads_again, 1);
    assert_eq!(observers[1].scan.reads_again, 0);
    for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
        publish(observer, fixture);
    }
    for view in ["raw", "spending", "balance", "governments"] {
        assert_eq!(
            read(&fixtures[0], "C1", 24, 25, view),
            read(&fixtures[1], "C1", 24, 25, view),
            "{view}"
        );
    }
    let actual = read(&fixtures[0], "C1", 24, 25, "spending");
    assert_eq!(
        actual
            .spend_items
            .iter()
            .map(|item| item.character)
            .collect::<Vec<_>>(),
        [Some(5)],
        "the payment names the leader's character at its dated read"
    );
    let expected = present_view(&oracle, 1, "C1", None, &flag, LedgerView::Balance).unwrap();
    assert_eq!(actual.spend_items, expected.spend_items);
    assert_eq!(actual.spending, expected.spending);
}

/// A leader's command power allocation by `country`, held while allocated.
fn leader_allocation(country: u32, hour: i32) -> CountryPowerChangeRow {
    CountryPowerChangeRow {
        country,
        instance: 1,
        epoch: 1,
        hour,
        sequence: 1,
        currency: PowerCurrency::Command,
        operation: PowerOperation::Subtract,
        requested_delta_raw: Some(-300_000),
        cause: PowerCause::LeaderAssignment,
        site: PowerSite::AllocateCommandPower,
        balance_before_raw: 1_000_000,
        balance_after_raw: 700_000,
        identity: PowerIdentity {
            object_kind: PowerObjectKind::Leader,
            object_reference: Some((u64::from(country) << 32) | 2),
            ..Default::default()
        },
    }
}

/// Each observer's event keys visited to publish and settle, and countries
/// observed again to name their allocators, over one recorded hour.
type PublicationWork = (usize, usize);

/// Observers joining from changed rows and from the whole tables, fed the
/// recorded hours of `count` countries, each publishing payments and holding
/// a leader's allocation.
struct Publications {
    recorded: Recorded,
    trackers: [dashboard_base::prepared::frame_changes::FrameChangeTracker; 2],
    observers: [Prepared; 2],
    input: DecodedFrame,
}
impl Publications {
    fn new(count: u32) -> Self {
        let mut observers = [Prepared::default(), Prepared::default()];
        observers[1].scan.whole_tables = true;
        let mut input = frame(24, count);
        put(
            &mut input,
            TableRows::CountryCommandPower(
                (1..=count)
                    .map(|canonical_slot| CountryCommandPowerRow {
                        canonical_slot,
                        allocated_command_power_raw: 300_000,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        put(
            &mut input,
            TableRows::CountryPowerChanges(
                (1..=count)
                    .flat_map(|country| {
                        [
                            leader_allocation(country, 24),
                            payment(country, 2, 24),
                            payment(country, 3, 24),
                        ]
                    })
                    .collect(),
            ),
        );
        // Characters and the dictionary read complete, as live recordings do.
        put(&mut input, TableRows::Characters(Vec::new()));
        put(&mut input, TableRows::LexicalTokens(Vec::new()));
        // Each allocation's leader, and one leader no allocation names.
        put(
            &mut input,
            TableRows::UnitLeaders(
                (1..=count + 1)
                    .map(|id| UnitLeaderRow {
                        id,
                        reference_type: 2,
                        character_id: 1_000 + id,
                        category: 1,
                        ..Default::default()
                    })
                    .collect(),
            ),
        );
        Self {
            recorded: Recorded::new(),
            trackers: [
                dashboard_base::prepared::frame_changes::FrameChangeTracker::new(),
                dashboard_base::prepared::frame_changes::FrameChangeTracker::new(),
            ],
            observers,
            input,
        }
    }
    fn step(&mut self) -> [PublicationWork; 2] {
        extend_captures(&mut self.input);
        let keyframe = self.input.hour == 24;
        let (decoded, wire) = self.recorded.next(&mut self.input, keyframe);
        let flag = AtomicBool::new(false);
        let mut work = [(0, 0); 2];
        for ((observer, tracker), work) in self
            .observers
            .iter_mut()
            .zip(&mut self.trackers)
            .zip(&mut work)
        {
            let before = (
                observer.scan.publication_visits,
                super::super::holds::named_count(),
            );
            let changes = tracker.observe_updates(
                &decoded,
                Arc::clone(&wire),
                replay::format::FormatVersion::CURRENT,
            );
            observer.observe(&decoded, &changes, &flag).unwrap();
            *work = (
                observer.scan.publication_visits - before.0,
                super::super::holds::named_count() - before.1,
            );
        }
        work
    }
    fn power(&self) -> Vec<CountryPowerChangeRow> {
        self.input
            .complete::<tables::CountryPowerChanges>()
            .unwrap()
            .to_vec()
    }
    fn leaders(&self) -> Vec<UnitLeaderRow> {
        self.input
            .complete::<tables::UnitLeaders>()
            .unwrap()
            .to_vec()
    }
}

#[test]
fn prepared_hour_publishing_one_payment_visits_only_its_country_events() {
    let count = 200;
    let mut hours = Publications::new(count);
    let [first, _] = hours.step();
    assert!(first.0 >= 3 * count as usize);
    hours.input.hour = 25;
    let [rows, whole] = hours.step();
    assert_eq!(rows.0, 0, "no power or government row changed");
    assert!(
        whole.0 >= 6 * count as usize,
        "the whole tables publish again"
    );

    // One country's payment.
    hours.input.hour = 26;
    let mut power = hours.power();
    power.push(payment(count / 2, 4, 26));
    power.sort_by_key(|row| (row.country, row.sequence));
    put(&mut hours.input, TableRows::CountryPowerChanges(power));
    let [rows, whole] = hours.step();
    assert_eq!(rows.0, 3 + 4, "the paying country's old and current events");
    assert!(whole.0 >= 6 * count as usize);

    // One country's government change.
    hours.input.hour = 27;
    put(
        &mut hours.input,
        TableRows::GovernmentChanges(vec![GovernmentChangeRow {
            country: count / 3,
            instance: 1,
            epoch: 1,
            sequence: 9,
            hour: 27,
            before_party: Some(1),
            after_party: Some(2),
            ..Default::default()
        }]),
    );
    let [rows, _] = hours.step();
    assert_eq!(rows.0, 1);
    let [rows, whole] = &hours.observers;
    assert_eq!(rows.scan.published_events, whole.scan.published_events);
    assert_eq!(rows.scan.events, whole.scan.events);
}

#[test]
fn prepared_unit_leader_change_observes_only_countries_whose_allocators_it_names() {
    let count = 40;
    let mut hours = Publications::new(count);
    hours.step();
    hours.input.hour = 25;
    hours.step();
    let held = |hours: &Publications| hours.observers[0].scan.holds.holding();
    assert_eq!(held(&hours), count as usize, "every country holds a leader");

    // A leader no allocation names.
    hours.input.hour = 26;
    let mut leaders = hours.leaders();
    leaders.last_mut().unwrap().skill_level = 3;
    put(&mut hours.input, TableRows::UnitLeaders(leaders));
    let [rows, whole] = hours.step();
    assert_eq!(rows.1, 0, "no allocation is named from the changed leader");
    assert_eq!(whole.1, count as usize, "every allocation is named again");

    // One allocation's leader, then the character it names.
    hours.input.hour = 27;
    let mut leaders = hours.leaders();
    leaders[(count / 2) as usize].skill_level = 4;
    put(&mut hours.input, TableRows::UnitLeaders(leaders));
    let [rows, _] = hours.step();
    assert_eq!(rows.1, 1);
    hours.input.hour = 28;
    put(
        &mut hours.input,
        TableRows::Characters(vec![CharacterRow {
            id: 1_000 + count / 4,
            token: b"Recorded leader".to_vec(),
            ..Default::default()
        }]),
    );
    let [rows, _] = hours.step();
    assert_eq!(rows.1, 1, "only the allocation its leader names");
    assert_eq!(held(&hours), count as usize);

    let fixtures = [Fixture::new(), Fixture::new()];
    for (observer, fixture) in hours.observers.iter_mut().zip(&fixtures) {
        publish(observer, fixture);
    }
    for country in [1, count / 4, count / 2, count] {
        let tag = format!("C{country}");
        let replies = fixtures
            .each_ref()
            .map(|fixture| read(fixture, &tag, 24, 28, "raw"));
        assert_eq!(replies[0].command_holds, replies[1].command_holds, "{tag}");
        assert_eq!(replies[0], replies[1], "{tag}");
    }
}

/// Every section a fixture's store publishes, byte for byte.
fn sections(fixture: &Fixture) -> BTreeMap<(String, u64), Vec<u8>> {
    use std::io::Read;
    let listing = store::Listing::open(&fixture.directory.join("manifest"))
        .unwrap()
        .expect("a published manifest");
    let mut result = BTreeMap::new();
    for (name, count) in listing.sections() {
        for range in 0..count {
            let mut bytes = Vec::new();
            fixture
                .store
                .read(&fixture.key, name, range)
                .unwrap()
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            result.insert((name.to_owned(), range), bytes);
        }
    }
    result
}

/// Each country's daily gain of `day`, its native application and cost,
/// and the political power event it applies.
fn daily_gain(
    country: u32,
    instance: u32,
    day: i32,
) -> (
    CountryPowerDailyRow,
    CountryPowerDailyCostRow,
    CountryPowerChangeRow,
) {
    (
        CountryPowerDailyRow {
            country,
            instance,
            epoch: 1,
            day_hour: day,
            pp_gross_gain_raw: Some(100),
            pp_cost_raw: Some(7),
            pp_net_gain_raw: Some(93),
            pp_realized_delta_raw: Some(80),
            pp_sequence: Some(day as u64),
            cp_gain_raw: Some(30),
            cp_realized_delta_raw: Some(30),
            cp_sequence: Some(day as u64 + 1),
        },
        CountryPowerDailyCostRow {
            country,
            instance,
            epoch: 1,
            day_hour: day,
            pp_sequence: day as u64,
            cost_raw: 7,
            cause: PowerCause::DailyGain,
            ..Default::default()
        },
        CountryPowerChangeRow {
            country,
            instance,
            hour: day,
            currency: PowerCurrency::Political,
            balance_before_raw: 10,
            balance_after_raw: 90,
            ..event(
                day as u64,
                PowerCause::DailyGain,
                PowerOperation::Add,
                Some(93),
            )
        },
    )
}

/// One hour of a campaign whose captures extend to every hour they are
/// read, so that most hours only advance them: a payment at 30 that leaves
/// the published window at 44; the third country's capture, not read from
/// 35 to 37 (stale), starting again at 38 under another instance; a
/// leader's command power allocation at 40; the first country's daily gain,
/// applied with its daily cost, at day 48, leaving the window at 62, and the
/// first and third countries' at day 72; the fourth country's capture
/// deleted at 56 and starting again at 58; and the second country's losing
/// its command power stream at 66.
fn advancing_hour(
    input: &mut DecodedFrame,
    captures: &mut Vec<PoliticsCaptureCoverageRow>,
    power: &mut Vec<CountryPowerChangeRow>,
) {
    let hour = input.hour;
    let days: Vec<(u32, u32, i32)> = if hour >= 72 {
        vec![(1, 1, 72), (3, 2, 72)]
    } else if hour >= 48 {
        vec![(1, 1, 48)]
    } else {
        Vec::new()
    };
    match hour {
        30 => power.push(payment(1, 30, 30)),
        38 => {
            captures[2].instance = 2;
            captures[2].since_hour = 38;
        }
        40 => {
            power.push(CountryPowerChangeRow {
                country: 2,
                instance: 1,
                epoch: 1,
                hour,
                sequence: 40,
                currency: PowerCurrency::Command,
                operation: PowerOperation::Subtract,
                requested_delta_raw: Some(-300_000),
                cause: PowerCause::LeaderAssignment,
                site: PowerSite::AllocateCommandPower,
                balance_before_raw: 1_000_000,
                balance_after_raw: 700_000,
                identity: PowerIdentity {
                    object_kind: PowerObjectKind::Leader,
                    object_reference: Some((7_u64 << 32) | 2),
                    ..Default::default()
                },
            });
            let mut command = input
                .complete::<tables::CountryCommandPower>()
                .unwrap()
                .to_vec();
            command[1].allocated_command_power_raw = 300_000;
            put(input, TableRows::CountryCommandPower(command));
        }
        44 => power.retain(|row| (row.country, row.sequence) != (1, 30)),
        48 | 72 => power.extend(
            days.iter()
                .map(|&(country, instance, day)| daily_gain(country, instance, day).2),
        ),
        56 => captures.retain(|row| row.country != 4),
        58 => captures.push(capture(4, 58, 58)),
        62 => power.retain(|row| (row.country, row.sequence) != (1, 48)),
        66 => captures[1].streams &= !PoliticsCaptureCoverageRow::CP,
        _ => {}
    }
    for row in captures.iter_mut() {
        if row.country != 3 || !(35..=37).contains(&hour) {
            row.through_hour = hour;
        }
    }
    power.sort_by_key(|row| (row.country, row.sequence));
    put(input, TableRows::PoliticsCaptureCoverage(captures.clone()));
    put(input, TableRows::CountryPowerChanges(power.clone()));
    let gains: Vec<_> = days
        .iter()
        .map(|&(country, instance, day)| daily_gain(country, instance, day))
        .collect();
    put(
        input,
        TableRows::CountryPowerDaily(gains.iter().map(|gain| gain.0.clone()).collect()),
    );
    put(
        input,
        TableRows::CountryPowerDailyCosts(gains.iter().map(|gain| gain.1.clone()).collect()),
    );
    // A live recording publishes the daily tables at every hour.
    for table in [TableId::CountryPowerDaily, TableId::CountryPowerDailyCosts] {
        input.metrics.table_ns[table.index()] = replay::frame::READ_TIME_NOT_RECORDED;
    }
}

/// Hours that only advance captures publish and keep exactly what the
/// whole accounting does, byte for byte, through payments, daily gains at
/// day boundaries, a stale capture and one starting again, a deleted
/// capture, a lost stream, a recording gap, a keyframe and two resumes; and
/// they skip the whole accounting, which a dense observer takes at every
/// hour that is not quiet.
#[test]
fn prepared_advance_only_hours_match_the_whole_accounting_byte_for_byte() {
    use dashboard_base::prepared::frame_changes::FrameChangeTracker;
    let format = replay::format::FormatVersion::CURRENT;
    let flag = AtomicBool::new(false);
    let fixtures = [Fixture::new(), Fixture::new()];
    let mut observers = [Prepared::default(), Prepared::default()];
    observers[1].dense = true;
    let mut trackers = [FrameChangeTracker::new(), FrameChangeTracker::new()];
    let mut recorded = Recorded::new();
    let mut input = frame(24, 4);
    put(&mut input, command_power(4));
    let mut captures: Vec<_> = (1..=4).map(|country| capture(country, 24, 24)).collect();
    let mut power = Vec::new();
    let mut advanced = BTreeSet::new();
    // Hours 52 and 53 are a recording gap; hour 70 a keyframe; both
    // observers resume after 60 and 84.
    let hours: Vec<i32> = (24..=95).filter(|hour| !(52..=53).contains(hour)).collect();
    for &hour in &hours {
        input.hour = hour;
        advancing_hour(&mut input, &mut captures, &mut power);
        let (decoded, wire) = recorded.next(&mut input, hour == 24 || hour == 70);
        let before = observers[0].advanced_hours();
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let changes = tracker.observe_updates(&decoded, Arc::clone(&wire), format);
            observer.observe(&decoded, &changes, &flag).unwrap();
        }
        if observers[0].advanced_hours() > before {
            advanced.insert(hour);
        }
        let [fast, dense] = &observers;
        assert_eq!(fast.scan.events, dense.scan.events, "hour {hour}");
        assert_eq!(
            fast.scan.published_events, dense.scan.published_events,
            "hour {hour}"
        );
        assert_eq!(fast.scan.costs, dense.scan.costs, "hour {hour}");
        assert_eq!(
            fast.scan.inputs.deadlines, dense.scan.inputs.deadlines,
            "hour {hour}"
        );
        assert_eq!(
            fast.scan.inputs.witnesses, dense.scan.inputs.witnesses,
            "hour {hour}"
        );
        assert_eq!(
            fast.scan.inputs.by_slot, dense.scan.inputs.by_slot,
            "hour {hour}"
        );
        assert_eq!(
            fast.current_daily_keys, dense.current_daily_keys,
            "hour {hour}"
        );
        // Daily keys read once per allocation are the hour's; cost keys
        // pruned once until the current keys change hold none but current.
        let daily: BTreeSet<_> = decoded
            .complete::<tables::CountryPowerDaily>()
            .unwrap_or_default()
            .iter()
            .map(RowCodec::key)
            .collect();
        assert_eq!(fast.current_daily_keys, daily, "hour {hour}");
        assert!(
            fast.scan
                .costs
                .iter()
                .all(|key| fast.scan.inputs.cost_keys.contains(key)),
            "hour {hour}: a recorded cost key is no current row's"
        );
        assert_eq!(fast.scan.clock.next, dense.scan.clock.next, "hour {hour}");
        assert_eq!(fast.scan.dirty, dense.scan.dirty, "hour {hour}");
        assert_eq!(fast.day_tags, dense.day_tags, "hour {hour}");
        assert_eq!(
            fast.tail.keys().collect::<Vec<_>>(),
            dense.tail.keys().collect::<Vec<_>>(),
            "hour {hour}"
        );
        assert_eq!(fast.frontier, dense.frontier, "hour {hour}");
        if ![44, 60, 84, 95].contains(&hour) {
            continue;
        }
        // The kept state and every published section, byte for byte.
        let saved = observers
            .each_mut()
            .map(|observer| observer.saved_bytes().unwrap());
        assert_eq!(saved[0], saved[1], "state saved at hour {hour}");
        for (observer, fixture) in observers.iter_mut().zip(&fixtures) {
            publish(observer, fixture);
        }
        let published = fixtures.each_ref().map(sections);
        assert!(!published[0].is_empty());
        assert_eq!(
            published[0].keys().collect::<Vec<_>>(),
            published[1].keys().collect::<Vec<_>>(),
            "sections published through hour {hour}"
        );
        for (section, bytes) in &published[0] {
            assert!(
                published[1][section] == *bytes,
                "{section:?} published through hour {hour}"
            );
        }
        for tag in ["C1", "C2", "C3", "C4"] {
            for view in ["raw", "spending", "balance", "governments"] {
                assert_eq!(
                    read(&fixtures[0], tag, 24, hour, view),
                    read(&fixtures[1], tag, 24, hour, view),
                    "{tag} {view} published through hour {hour}"
                );
            }
        }
        if hour == 95 {
            break;
        }
        for (observer, tracker) in observers.iter_mut().zip(&mut trackers) {
            let (dense, paths) = (observer.dense, observer.paths);
            *observer = Prepared::read(observer.saved_bytes().unwrap().as_slice()).unwrap();
            observer.dense = dense;
            observer.paths = paths;
            observer.resume(&decoded);
            tracker.invalidate();
        }
    }
    let [fast, dense] = &observers;
    assert_eq!(
        dense.advanced_hours(),
        0,
        "the dense observer accounts every hour"
    );
    assert_eq!(
        fast.advanced_hours() + fast.accounted_hours(),
        dense.accounted_hours(),
        "each hour not quiet takes one path"
    );
    // Hours whose captures all extend and nothing else changes: after a
    // payment, after a capture went stale or started again, after a leader's
    // allocation, after a daily gain, while a deleted capture is absent.
    for hour in [
        25, 26, 27, 28, 29, 31, 32, 33, 36, 37, 39, 41, 42, 43, 49, 50, 51, 57, 59, 63, 64, 65, 67,
        68, 69, 73, 74, 75, 86, 87, 88,
    ] {
        assert!(
            advanced.contains(&hour),
            "hour {hour} only advanced captures; the advance path took {advanced:?}"
        );
    }
    for hour in [24, 30, 38, 40, 48, 54, 56, 58, 61, 66, 70, 72, 85] {
        assert!(
            !advanced.contains(&hour),
            "hour {hour} changed the ledger; the advance path took it"
        );
    }
    assert!(
        fast.accounted_hours() * 3 < hours.len(),
        "most hours skip the whole accounting: {} of {} took it",
        fast.accounted_hours(),
        hours.len()
    );
}
