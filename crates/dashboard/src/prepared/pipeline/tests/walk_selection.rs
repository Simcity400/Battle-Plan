//! The remaining walk reads only its observers' union, with the all-table
//! walk as a byte oracle for publications, continuations and refusals.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use replay::container::test_work::table_decodes;
use serde_json::Value;

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

fn all_tables<T>(run: impl FnOnce() -> T) -> T {
    struct Reset(bool);
    impl Drop for Reset {
        fn drop(&mut self) {
            TEST_ALL_TABLES.with(|flag| flag.set(self.0));
        }
    }
    let _reset = Reset(TEST_ALL_TABLES.with(|flag| flag.replace(true)));
    run()
}

fn clear(source: &Source) {
    super::super::super::files(source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Only the selected layout and its byte accounting differ. Every published
/// page and branch continuation, including compressed bytes, is compared.
fn assert_same(actual: &Payloads, expected: &Payloads) {
    let pages = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| name != CHECKPOINT && name != "map.resume")
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&pages(actual), &pages(expected), "selected walk");
    let checkpoint = |payloads: &Payloads| {
        let mut value: Value = serde_json::from_slice(&payloads[&(CHECKPOINT.into(), 0)]).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("selected");
        fields.remove("bytes");
        value
    };
    assert_eq!(checkpoint(actual), checkpoint(expected));
    let resume = |payloads: &Payloads| {
        dashboard_base::prepared_map::decode_resume(payloads[&("map.resume".into(), 0)].as_slice())
            .unwrap()
    };
    let actual = resume(actual);
    let mut expected = resume(expected);
    assert_eq!(actual.position, expected.position);
    for table in TableId::ALL {
        if !selected().contains(&table) {
            expected.frame.tables[table.index()] = actual.frame.tables[table.index()].clone();
        }
    }
    let encoded = |frame: &DecodedFrame| {
        let mut bytes = Vec::new();
        frame.encode_checkpoint(&mut bytes).unwrap();
        bytes
    };
    assert_eq!(encoded(&actual.frame), encoded(&expected.frame));
}

fn measured(source: &Source) {
    run_measured_through(
        source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |_| {},
    )
    .unwrap();
}

#[test]
fn walk_decodes_only_its_union_once_per_block_and_preserves_every_selected_hour() {
    let replay = prepass_parity::resources(5, true);
    assert!(selected().len() < TableId::ALL.len());
    for table in [
        TableId::DivisionCensus,
        TableId::DivisionLandInputs,
        TableId::ResourceExchanges,
        TableId::Divisions,
    ] {
        assert!(!selected().contains(&table), "folded input {table:?}");
        assert!(
            block_selected(true, true).contains(&table),
            "fallback input {table:?}"
        );
    }
    for table in [
        TableId::LandCombatProgress,
        TableId::LandCombatResults,
        TableId::LandEventCoverage,
        TableId::CountryGameScoreInputs,
        TableId::AirWingGameScoreStats,
        TableId::GameScoreMultipliers,
        TableId::ReservedCountryNaming,
    ] {
        assert!(selected().contains(&table), "walk input {table:?}");
    }
    assert_eq!(independent::with_test_hourly(selected), &TableId::ALL);
    assert!(!block_selected(false, false).contains(&TableId::Divisions));
    assert!(!block_selected(true, false).contains(&TableId::Divisions));
    assert!(!block_selected(false, true).contains(&TableId::ResourceExchanges));
    assert_eq!(
        independent::with_test_only(&["groups"], selected),
        &TableId::ALL
    );
    for held in [false, true] {
        let fixture = Fixture::new(&replay);
        let held_replay;
        let replay = if held {
            held_replay = Replay::open_path(&fixture.path).unwrap();
            &held_replay
        } else {
            &replay
        };
        let mut full = ReplayStream::new(&TableId::ALL).unwrap();
        let mut narrow = ReplayStream::new(selected()).unwrap();
        for block in &replay.index {
            let mut expected = Vec::new();
            full.next_block(replay, |frame| {
                expected.push(frame.clone());
                Ok(())
            })
            .unwrap();
            let mut hour = 0;
            let (_, decodes) = table_decodes(|| {
                narrow
                    .next_block(replay, |frame| {
                        assert_eq!(frame.metrics, expected[hour].metrics);
                        assert_eq!(frame.hour, expected[hour].hour);
                        for &table in selected() {
                            let actual = &frame.tables[table.index()];
                            let expected = &expected[hour].tables[table.index()];
                            assert_eq!(actual, expected, "{table:?}");
                            assert_eq!(actual.sampled_at(), expected.sampled_at());
                        }
                        hour += 1;
                        Ok(())
                    })
                    .unwrap();
            });
            assert_eq!(hour, expected.len());
            assert_eq!(
                decodes,
                selected()
                    .iter()
                    .map(|&table| (block.branch, block.first_hour, table, expected.len()))
                    .collect::<Vec<_>>(),
                "held={held}: no unused table decoded, no selected table decoded twice"
            );
        }
    }
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow: full preparation parity")]
fn selected_walk_pages_states_refusals_resume_cancel_and_append_equal_the_all_table_walk() {
    let first = prepass_parity::resources(3, false);
    let grown = prepass_parity::resources(5, true);
    let mut fixture = Fixture::new(&first);
    for resumed in [false, true] {
        let prepare = |source: &Source| {
            if resumed {
                while unprepared_blocks(source, None).unwrap() != Some(0) {
                    run_measured(
                        source,
                        None,
                        &AtomicBool::new(false),
                        true,
                        Duration::ZERO,
                        |_| {},
                    )
                    .unwrap();
                }
            } else {
                measured(source);
            }
        };
        let mut reference = None;
        for full in [true, false] {
            fixture.replace(&first);
            let mut work = || {
                let source = fixture.source("walk-append");
                prepare(&source);
                drop(source);
                fixture.replace(&grown);
                let source = fixture.source("walk-append");
                prepare(&source);
                let actual = payloads(&source, None);
                clear(&source);
                actual
            };
            let actual = if full { all_tables(work) } else { work() };
            match &reference {
                None => reference = Some(actual),
                Some(expected) => assert_same(&actual, expected),
            }
        }
    }
    fixture.replace(&grown);
    for (family, hour) in [
        ("measure", 104),
        ("land", 115),
        ("game_score", 111),
        ("autonomy", 118),
    ] {
        let mut reference = None;
        for full in [true, false] {
            let source = fixture.source("walk-refused");
            let work = || lanes::with_test_refusals(&[(family, hour)], || measured(&source));
            if full {
                all_tables(work)
            } else {
                work()
            };
            let actual = payloads(&source, None);
            clear(&source);
            match &reference {
                None => reference = Some(actual),
                Some(expected) => assert_same(&actual, expected),
            }
        }
    }
    let source = fixture.source("walk-cancel");
    measured(&source);
    let expected = payloads(&source, None);
    clear(&source);
    let cancelled = AtomicBool::new(false);
    let Err(error) = run_measured_through(
        &source,
        None,
        &cancelled,
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |accepted| {
            if accepted.next_block == 1 {
                cancelled.store(true, Ordering::Release);
            }
        },
    ) else {
        panic!("a cancelled preparation");
    };
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    cancelled.store(false, Ordering::Release);
    measured(&source);
    assert_payloads_equal(&payloads(&source, None), &expected, "cancel/resume");
}

#[test]
#[cfg_attr(not(feature = "slow-tests"), ignore = "slow: full preparation parity")]
fn selected_walk_matches_hourly_preparation_and_saves_the_same_domain_state() {
    let first = prepass_parity::resources(3, false);
    let grown = prepass_parity::resources(5, true);
    let mut fixture = Fixture::new(&first);
    let mut prepared = Vec::new();
    for hourly in [true, false] {
        fixture.replace(&first);
        let mut prepare = || {
            let resume = |source: &Source| {
                while unprepared_blocks(source, None).unwrap() != Some(0) {
                    run_measured(
                        source,
                        None,
                        &AtomicBool::new(false),
                        true,
                        Duration::ZERO,
                        |_| {},
                    )
                    .unwrap();
                }
            };
            let source = fixture.source("walk-hourly");
            resume(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("walk-hourly");
            resume(&source);
            let pages = payloads(&source, None);
            clear(&source);
            pages
        };
        prepared.push(if hourly {
            independent::with_test_hourly(|| prepass::with_test_sequential(prepare))
        } else {
            prepare()
        });
    }
    let [hourly, actual]: [Payloads; 2] = prepared.try_into().unwrap();
    let walk = |name: &str| {
        [
            "map.",
            "measure-",
            "army-",
            "resource-",
            "land-battle",
            "land-owners",
            "game-score-",
            "autonomy-",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
            && name != "map.resume"
    };
    let pages = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| walk(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&pages(&actual), &pages(&hourly), "hourly walk pages");
    for ((name, range), bytes) in &hourly {
        if !name.starts_with("preparation-branch-") {
            continue;
        }
        let decoded = |bytes: &[u8]| -> Value {
            serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
        };
        let expected = decoded(bytes);
        let actual = decoded(&actual[&(name.clone(), *range)]);
        for field in [
            "schema",
            "measure",
            "land",
            "army",
            "resources",
            "game_score",
        ] {
            assert_eq!(actual[field], expected[field], "{name}: {field}");
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: 5,000-country preparation parity"
)]
fn selected_walk_and_all_table_walk_save_identical_5000_country_pages_and_state() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 100, 5000), frame(1, 101, 5000)])],
    )
    .unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("walk-5000");
    all_tables(|| measured(&source));
    let expected = payloads(&source, None);
    clear(&source);
    measured(&source);
    assert_same(&payloads(&source, None), &expected);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: full preparation with a 720-hour block"
)]
fn walk_reads_map_only_at_checkpoints_and_never_rereads_folded_army_rows() {
    use replay_data::rows::RailCooldownRow;
    let frames = (1000..=1720)
        .map(|hour| {
            let mut frame = frame(1, hour, 2);
            frame.tables[TableId::RailCooldowns.index()] =
                TableView::Complete(TableRows::RailCooldowns(vec![RailCooldownRow {
                    province: 1,
                    cooldown_ticks: hour,
                    ..Default::default()
                }]))
                .into();
            frame
        })
        .collect();
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
    assert_eq!(replay.index.len(), 2);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("sparse-map-work");
    all_tables(|| measured(&source));
    let expected = payloads(&source, None);
    clear(&source);
    let (_, decodes) = table_decodes(|| measured(&source));
    assert_same(&payloads(&source, None), &expected);
    for table in [
        TableId::RailCooldowns,
        TableId::Divisions,
        TableId::ResearchTechnologies,
        TableId::SliceReads,
    ] {
        assert_eq!(
            decodes
                .iter()
                .filter(|(_, _, decoded, _)| *decoded == table)
                .map(|&(branch, first, _, hours)| (branch, first, hours))
                .collect::<Vec<_>>(),
            vec![(1, 1000, 1), (1, 1720, 1)],
            "{table:?}: keyframes only on the walk"
        );
    }
    // Playback still restores the full map at a checkpoint and between them.
    let (store, key, progress) = map_checkpoint_binding(&source, None).unwrap().unwrap();
    let tables = dashboard_base::prepared_map::selected_tables();
    let mut reader =
        dashboard_base::prepared_map::Reader::new(store, key, progress, source.replay(), tables)
            .unwrap();
    for hour in [1000, 1001, 1719, 1720] {
        let Some(dashboard_base::prepared_map::Current::Frame(actual)) =
            reader.seek(1, hour).unwrap()
        else {
            panic!("prepared map frame at {hour}");
        };
        let replay::container::SeekResult::Frame(expected) =
            replay.seek_selected(1, hour, Some(tables)).unwrap()
        else {
            panic!("recorded map frame at {hour}");
        };
        assert_eq!(*actual, *expected, "playback at {hour}");
        for &table in tables {
            assert_eq!(
                actual.tables[table.index()].sampled_at(),
                expected.tables[table.index()].sampled_at(),
                "{table:?} at {hour}"
            );
        }
    }
}
