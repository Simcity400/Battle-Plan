use super::*;
use std::{io::Read, sync::Mutex, time::Duration};

// Enumerate the manifest's sections through the store's listing, and open
// every registered payload through the real store reader. UUIDs and physical
// file names are not payloads.
pub(super) fn payloads(
    source: &Source,
    content: Option<&crate::products::ContentOptions>,
) -> BTreeMap<(String, u64), Vec<u8>> {
    let (store, key) = super::super::super::files(source, content).unwrap();
    let path = std::fs::canonicalize(source.path()).unwrap();
    let hash = blake3::hash(path.as_os_str().as_encoded_bytes());
    let id = uuid::Uuid::from_bytes(hash.as_bytes()[..16].try_into().unwrap());
    let directory = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
        .join("HOI4Replay/prepared")
        .join(id.to_string());
    let listing = store::Listing::open(&directory.join("manifest"))
        .unwrap()
        .unwrap();
    let mut result = BTreeMap::new();
    for (name, count) in listing.sections() {
        for range in 0..count {
            let mut bytes = Vec::new();
            store
                .read(&key, name, range)
                .unwrap()
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            result.insert((name.to_owned(), range), bytes);
        }
    }
    result
}

/// Every section of two stores, byte for byte.
///
/// A family prepared on its own is compared with hourly preparation like
/// with like: its own runs register that family alone
/// (`independent::with_test_only`, or `with_test_families` with its statics)
/// so no other family's own sections, pages or refusals appear in one mode
/// only, and the hourly reference runs under `independent::with_test_hourly`.
/// What still differs by design is mode-specific and compared otherwise: the
/// checkpoint (it holds the family's progress), its `preparation-family-*`
/// pages, the sections it keeps for readers, the walk's branch state (field
/// by field: the walk leaves out the state of families it skips) and the
/// walk's branch coverage page, whose refusals of the family move to the
/// family's own page. A store-level check (`unprepared_blocks`, `checkpoint`)
/// runs under the registry the store was prepared with: another set of
/// families prepared on their own restarts preparation.
pub(super) fn assert_payloads_equal(
    actual: &BTreeMap<(String, u64), Vec<u8>>,
    expected: &BTreeMap<(String, u64), Vec<u8>>,
    label: &str,
) {
    // Compare every byte, but report section/range and first offset rather than
    // dumping retained payloads (especially the 5,000-country fixture) on failure.
    let keys: std::collections::BTreeSet<_> = actual.keys().chain(expected.keys()).collect();
    let mut differences = Vec::new();
    for key in keys {
        let left = actual.get(key);
        let right = expected.get(key);
        if left == right {
            continue;
        }
        let first = left.zip(right).map(|(left, right)| {
            left.iter()
                .zip(right)
                .position(|(left, right)| left != right)
                .unwrap_or(left.len().min(right.len()))
        });
        differences.push(format!(
            "{} range={} lengths={:?}/{:?} first-offset={first:?}",
            key.0,
            key.1,
            left.map(Vec::len),
            right.map(Vec::len)
        ));
    }
    assert!(
        differences.is_empty(),
        "{label}:\n{}",
        differences.join("\n")
    );
}

// Two fresh serial runs are the control for every parallel byte comparison.
// Clearing the real store and dropping Source resets runtime observer state.
pub(super) fn serial_control(
    fixture: &mut Fixture,
    name: &str,
    content: Option<&crate::products::ContentOptions>,
) -> BTreeMap<(String, u64), Vec<u8>> {
    let mut expected = None;
    for _ in 0..2 {
        let source = fixture.source(name);
        lanes::with_test_lanes(2, None, || {
            run(&source, content, &AtomicBool::new(false), |_| {})
        })
        .unwrap();
        let actual = payloads(&source, content);
        if let Some(expected) = &expected {
            assert_payloads_equal(
                &actual,
                expected,
                "fresh serial runs must be byte-identical",
            );
        } else {
            expected = Some(actual);
        }
        super::super::super::files(&source, content)
            .unwrap()
            .0
            .clear()
            .unwrap();
        drop(source);
    }
    expected.unwrap()
}

fn compare_fixture(replay: &Replay) {
    let mut fixture = Fixture::new(replay);
    let expected = serial_control(&mut fixture, "lane-parity", None);
    let parallel = fixture.source("lane-parity");
    lanes::with_test_lanes(4, None, || {
        run(&parallel, None, &AtomicBool::new(false), |_| {})
    })
    .unwrap();
    assert_payloads_equal(&payloads(&parallel, None), &expected, "parallel vs serial");
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_fresh_serial_runs_match_every_section() {
    let mut fixture = Fixture::new(&recording());
    let expected = serial_control(&mut fixture, "serial-control", None);
    assert!(!expected.is_empty());
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_match_every_section_and_keep_the_time_budget() {
    compare_fixture(&recording());
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let same_branch = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    compare_fixture(&same_branch);
    let mut missing = frame(1, 10, 2);
    *missing.tables[TableId::Focus.index()] = TableView::Unavailable {
        reason: "Synthetic focus read unavailable".into(),
    };
    *missing.tables[TableId::Ships.index()] = TableView::Unavailable {
        reason: "Synthetic ship read unavailable".into(),
    };
    compare_fixture(&Replay::from_records(CAMPAIGN.into(), &[records(1, vec![missing])]).unwrap());
}

/// Every lane job, in the coordinator's serial call order, with every family
/// prepared hour by hour (`independent::with_test_hourly`): a family prepared
/// on its own is no lane job otherwise. Resources and army fold their block
/// facts instead (`prepass.rs`).
const LANE_FAMILIES: &[&str] = &[
    "nukes",
    "factories",
    "projects",
    "peace",
    "research",
    "construction",
    "trade",
    "measure",
    "navy",
    "air",
    "supply",
    "stockpile",
    "air_ledger",
    "aces",
    "raids",
    "game_score",
    "land",
    "divisions",
    "naval",
    "subjects",
    "autonomy",
    "tension",
    "wars",
    "ideas",
    "parties",
    "leaders",
    "intelligence",
    "stability",
    "experience",
    "politics_ledger",
    "fuel",
    "occupation",
    "casualties",
    "equipment_losses",
];

/// Families observed by lane jobs whose observation needs no other family's
/// output of the same frame.
const MOVED: &[&str] = &[
    "projects",
    "construction",
    "navy",
    "air",
    "supply",
    "stockpile",
    "raids",
    "land",
    "naval",
    "ideas",
    "leaders",
    "intelligence",
    "politics_ledger",
    "occupation",
    "casualties",
];

#[test]
fn family_lanes_observe_every_job_family_in_serial_order_and_on_lanes() {
    for parallelism in [2, 4] {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let trace = Arc::clone(&seen);
        let hook: lanes::Hook = Arc::new(move |family, observed, _changes| {
            if family != "coordinator" && observed.branch == 1 && observed.hour == 10 {
                trace.lock().unwrap().push(family.to_owned());
            }
            lanes::Fault::Continue
        });
        let mut fixture = Fixture::new(&recording());
        let source = fixture.source("lane-jobs");
        independent::with_test_hourly(|| {
            lanes::with_test_lanes(parallelism, Some(hook), || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap();
        let mut seen = seen.lock().unwrap().clone();
        if parallelism == 2 {
            // Serial observation calls each job where the coordinator called it.
            assert_eq!(seen, LANE_FAMILIES);
        }
        seen.sort_unstable();
        let mut expected = LANE_FAMILIES.to_vec();
        expected.sort_unstable();
        assert_eq!(seen, expected, "lane jobs at parallelism {parallelism}");
        for family in MOVED {
            assert!(LANE_FAMILIES.contains(family), "{family} is not a lane job");
        }
    }
}

// One frame giving several moved families rows that change by hour.
fn moved_families_frame(hour: i32) -> DecodedFrame {
    use replay_data::{
        rows::ActiveIdeaRow, rows::AirWingRow, rows::LexicalTokenRow, rows::ShipCounterInputs,
        rows::ShipModelRow, rows::ShipRow, rows::StateResistanceRow, rows::TaskForceRow,
        rows::WingEquipmentRow,
    };
    let mut frame = frame(1, hour, 3);
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![
        AirWingRow {
            id: 1,
            owner: 100_000,
            country_override: 100_001,
            planes: 10 + hour,
            ..Default::default()
        },
        AirWingRow {
            id: 2,
            owner: 100_001,
            planes: 15,
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::WingEquipment.index()] =
        TableView::Complete(TableRows::WingEquipment(vec![
            WingEquipmentRow {
                wing: 1,
                ordinal: 0,
                variant: 77,
                amount_raw: 1_000_000,
            },
            WingEquipmentRow {
                wing: 2,
                ordinal: 0,
                variant: 78,
                amount_raw: 1_500_000,
            },
        ]));
    *frame.tables[TableId::TaskForces.index()] = TableView::Complete(TableRows::TaskForces(
        (0..3_u32)
            .map(|index| TaskForceRow {
                id: u64::from(index) + 1,
                roster_owner: 100_000 + index,
                logical_owner: 100_000 + (index + 1) % 3,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::Ships.index()] = TableView::Complete(TableRows::Ships(
        (0..3_u32)
            .map(|index| ShipRow {
                id: u64::from(index) + 1,
                task_force: u64::from(index) + 1,
                class_id: 41,
                strength: i64::from(index + 1) * 100_000 + i64::from(hour) * 1_000,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ShipModels.index()] = TableView::Complete(TableRows::ShipModels(
        (0..3_u32)
            .map(|index| ShipModelRow {
                ship: u64::from(index) + 1,
                counter: Some(ShipCounterInputs {
                    maximum_strength: 10_000_000,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 41,
            literal: b"first_ship".to_vec(),
        }]));
    let mut ideas = vec![ActiveIdeaRow {
        country: 100_000,
        ordinal: 0,
        token: b"mod_idea".to_vec(),
    }];
    if hour >= 12 {
        ideas.push(ActiveIdeaRow {
            country: 100_001,
            ordinal: 0,
            token: b"mod_later_idea".to_vec(),
        });
    }
    *frame.tables[TableId::ActiveIdeas.index()] =
        TableView::Complete(TableRows::ActiveIdeas(ideas));
    *frame.tables[TableId::HiredAdvisors.index()] =
        TableView::Complete(TableRows::HiredAdvisors(Vec::new()));
    if hour >= 12
        && let TableView::Complete(TableRows::States(rows)) =
            &mut *frame.tables[TableId::States.index()]
    {
        // The second country occupies the first country's state.
        rows[0].controller = 100_001;
    }
    *frame.tables[TableId::StateResistance.index()] =
        TableView::Complete(TableRows::StateResistance(
            (0..3_u32)
                .map(|index| StateResistanceRow {
                    state: index + 1,
                    country: 100_000 + index,
                    active: 1,
                    level: i64::from(index) + i64::from(hour),
                    compliance: i64::from(index),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

// Project and sinking rows over the single-country source fixture.
fn project_and_sinking_frame(hour: i32) -> DecodedFrame {
    use replay_data::{
        rows::LexicalTokenRow, rows::ProjectRow, rows::ProjectState, rows::SunkShipEntry,
        rows::SunkShipsRow,
    };
    let mut frame = crate::test_fixtures::source(1, hour, Some(i64::from(hour) * 1000));
    frame.metrics.table_ns[TableId::Projects.index()] = 1;
    *frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(
        (1..=2)
            .map(|local_id| ProjectRow {
                owner: 11,
                local_id,
                definition_id: 8,
                started: 1,
                state: ProjectState {
                    current_offset: 0xe8,
                    not_completed: 0,
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::ProjectHistory.index()] =
        TableView::Complete(TableRows::empty(TableId::ProjectHistory));
    *frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 8,
            literal: b"mod_project".to_vec(),
        }]));
    let entries = if hour >= 102 {
        vec![SunkShipEntry {
            ship_name: b"synthetic lane sinking".to_vec(),
            victim_country: 9001,
            attacker_country: 9001,
            sink_hour: 102,
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    *frame.tables[TableId::SunkShips.index()] =
        TableView::Complete(TableRows::SunkShips(vec![SunkShipsRow { entries }]));
    frame.metrics.table_ns[TableId::SunkShips.index()] = 1;
    frame
}

// Two blocks of one branch: the first two frames, then the rest appended.
fn two_blocks(frames: Vec<DecodedFrame>) -> Replay {
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, frames[..2].to_vec())]).unwrap();
    first
        .append(&[continuation_records(&first, 1, frames[2..].to_vec())])
        .unwrap()
}

/// Lanes match serial observation of `replay`, and the comparison covers each
/// of `families`: a lane omitting that family's observations is detected.
fn assert_lane_parity_covers(
    replay: &Replay,
    families: &[&'static str],
    undetected: &mut Vec<&'static str>,
) {
    let mut fixture = Fixture::new(replay);
    let expected = serial_control(&mut fixture, "lane-coverage", None);
    let clear = |source: Arc<Source>| {
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
    };
    let parallel = fixture.source("lane-coverage");
    lanes::with_test_lanes(4, None, || {
        run(&parallel, None, &AtomicBool::new(false), |_| {})
    })
    .unwrap();
    assert_payloads_equal(
        &payloads(&parallel, None),
        &expected,
        "moved families: lanes vs serial",
    );
    clear(parallel);
    for &family in families {
        let source = fixture.source("lane-coverage");
        let hook: lanes::Hook = Arc::new(move |name, _frame, _changes| {
            if name == family {
                lanes::Fault::Skip
            } else {
                lanes::Fault::Continue
            }
        });
        let diverged = lanes::with_test_lanes(4, Some(hook), || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
        .is_err()
            || payloads(&source, None) != expected;
        if !diverged {
            undetected.push(family);
        }
        clear(source);
    }
}

#[test]
fn family_lanes_match_serial_for_every_moved_family_and_detect_a_diverging_lane() {
    let recordings = [
        (
            two_blocks((10..=13).map(moved_families_frame).collect()),
            &[
                "air",
                "navy",
                "ideas",
                "occupation",
                "politics_ledger",
                "supply",
                "raids",
                "intelligence",
                "stockpile",
                "construction",
            ][..],
        ),
        (
            two_blocks(
                crate::products::unit_leaders::prepared::test_fixtures::outcome_frames(false),
            ),
            &["leaders", "land"][..],
        ),
        (
            two_blocks(
                (100..=103)
                    .map(|hour| crate::products::wars::test_fixtures::scaled_recorded(hour, 3))
                    .collect(),
            ),
            &["casualties"][..],
        ),
        (
            two_blocks((100..=103).map(project_and_sinking_frame).collect()),
            &["projects", "naval"][..],
        ),
    ];
    let mut covered: Vec<_> = recordings
        .iter()
        .flat_map(|(_, families)| families.iter().copied())
        .collect();
    covered.sort_unstable();
    let mut moved = MOVED.to_vec();
    moved.sort_unstable();
    assert_eq!(
        covered, moved,
        "every moved family has a covering recording"
    );
    let mut undetected = Vec::new();
    // Every moved family is a lane job of the hourly walk only when no family
    // is prepared on its own.
    for (replay, families) in &recordings {
        independent::with_test_hourly(|| {
            assert_lane_parity_covers(replay, families, &mut undetected)
        });
    }
    assert!(
        undetected.is_empty(),
        "lane parity misses a diverging lane of {undetected:?}"
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_feed_sources_match_serial_sections_and_oracle() {
    use crate::products::event_feed::prepared::tests::{assert_store_pages, frame};
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(100, 3), frame(101, 3)])],
    )
    .unwrap();
    let second = first
        .append(&[continuation_records(&first, 1, vec![frame(102, 3)])])
        .unwrap();
    let replay = second
        .append(&[continuation_records(
            &second,
            1,
            vec![frame(104, 3), frame(105, 3)],
        )])
        .unwrap();
    let mut fixture = Fixture::new(&replay);
    let expected = serial_control(&mut fixture, "lane-feed", None);
    let source = fixture.source("lane-feed");
    lanes::with_test_lanes(4, None, || {
        run(&source, None, &AtomicBool::new(false), |_| {})
    })
    .unwrap();
    assert_payloads_equal(&payloads(&source, None), &expected, "feed lanes vs serial");
    let (store, key) = super::super::super::files(&source, None).unwrap();
    assert_store_pages(&source, &store, &key);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_same_frame_identity_and_membership_change_match_serial() {
    use replay_data::{rows::CountryLinkRow, rows::FactionMemberRow, rows::FactionRow};
    let mut initial = frame(1, 10, 3);
    let mut changed = frame(1, 11, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *changed.tables[TableId::Countries.index()]
    {
        rows[0].id = 200_000;
        rows[0].tag = b"REN".to_vec();
        rows.sort_unstable_by_key(|row| row.id);
    } else {
        unreachable!()
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *changed.tables[TableId::CountryLinks.index()]
    {
        // Retain the old raw alias, now resolving to the replacement holder.
        rows.push(CountryLinkRow {
            id: 200_000,
            canonical_slot: 0,
        });
    } else {
        unreachable!()
    }
    for (observed, country, faction) in [(&mut initial, 100_000, 77), (&mut changed, 200_000, 88)] {
        *observed.tables[TableId::Factions.index()] =
            TableView::Complete(TableRows::Factions(vec![
                FactionRow {
                    id: 77,
                    name: b"First pact".to_vec(),
                    ..Default::default()
                },
                FactionRow {
                    id: 88,
                    name: b"Second pact".to_vec(),
                    ..Default::default()
                },
            ]));
        *observed.tables[TableId::FactionMembers.index()] =
            TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
                country,
                faction,
                ordinal: 0,
            }]));
    }
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![initial, changed])]).unwrap();
    // Territory/history/groups run on the coordinator; subject/tension/supply
    // jobs run on lanes. They must use this immutable frame and their own state,
    // not a coordinator's old or partially updated identity/link cache.
    compare_fixture(&replay);
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn frame_changes_are_computed_once_and_shared_by_coordinator_and_all_lanes() {
    use crate::prepared::frame_changes::{FrameChanges, observation_count};
    for parallelism in [2, 4] {
        let seen = Arc::new(Mutex::new(BTreeMap::new()));
        let trace = Arc::clone(&seen);
        let hook: lanes::Hook = Arc::new(move |family, observed, changes| {
            let mut seen = trace.lock().unwrap();
            let (address, coordinator, workers) = seen
                .entry((observed.branch, observed.hour))
                .or_insert((changes as *const FrameChanges as usize, 0, 0));
            assert_eq!(
                *address, changes as *const FrameChanges as usize,
                "a lane recalculated/copied the shared fact"
            );
            if family == "coordinator" {
                *coordinator += 1;
            } else {
                *workers += 1;
            }
            for table in TableId::ALL {
                assert_eq!(
                    changes.table_read[table.index()],
                    observed.metrics.table_ns[table.index()] != 0
                );
            }
            lanes::Fault::Continue
        });
        let mut fixture = Fixture::new(&recording());
        let source = fixture.source("frame-fact-sharing");
        let before = observation_count();
        // Every family a lane job: none prepared on its own.
        independent::with_test_hourly(|| {
            lanes::with_test_lanes(parallelism, Some(hook), || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap();
        assert_eq!(
            observation_count() - before,
            4,
            "one fact per admitted frame"
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 4);
        for (_, coordinator, workers) in seen.values() {
            assert_eq!(*coordinator, 1);
            assert_eq!(
                *workers,
                LANE_FAMILIES.len(),
                "every lane job sees the same fact"
            );
        }
    }
}

#[test]
fn frame_changes_force_all_on_same_branch_post_publication_restore() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let replay = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    let checked = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&checked);
    let hook: lanes::Hook = Arc::new(move |family, observed, changes| {
        if family == "coordinator" && observed.hour == 12 {
            assert!(changes.rows_changed.iter().all(|changed| *changed));
            flag.store(true, Ordering::Release);
        }
        lanes::Fault::Continue
    });
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("frame-fact-restore");
    let (accepted, _) = lanes::with_test_lanes(4, Some(hook), || {
        run_measured_through(
            &source,
            None,
            &AtomicBool::new(false),
            false,
            Duration::ZERO,
            Some(4),
            |_| Ok(()),
            |_| {},
        )
    })
    .unwrap();
    assert_eq!(accepted.new_blocks, 2);
    assert!(checked.load(Ordering::Acquire));
}

#[test]
fn family_lanes_enter_together_and_never_advance_to_the_next_frame() {
    use std::sync::mpsc;
    // The two heaviest lane jobs, the politics ledger and stability, are
    // always packed on different lanes.
    let (resource_send, resource_receive) = mpsc::sync_channel(1);
    let (ledger_send, ledger_receive) = mpsc::sync_channel(1);
    let resource_receive = Mutex::new(resource_receive);
    let ledger_receive = Mutex::new(ledger_receive);
    let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&entered);
    let resource_frame = std::sync::atomic::AtomicUsize::new(0);
    let ledger_frame = std::sync::atomic::AtomicUsize::new(0);
    let hook: lanes::Hook = Arc::new(move |family, observed, _changes| {
        if observed.hour == 10 && family == "stability" {
            resource_frame.store(observed as *const DecodedFrame as usize, Ordering::SeqCst);
            count.fetch_add(1, Ordering::SeqCst);
            resource_send.send(()).unwrap();
            ledger_receive
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .expect("the politics ledger must enter while stability is active");
            assert_eq!(
                resource_frame.load(Ordering::SeqCst),
                ledger_frame.load(Ordering::SeqCst),
                "lanes must share the same frame allocation"
            );
        } else if observed.hour == 10 && family == "politics_ledger" {
            ledger_frame.store(observed as *const DecodedFrame as usize, Ordering::SeqCst);
            count.fetch_add(1, Ordering::SeqCst);
            ledger_send.send(()).unwrap();
            resource_receive
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .expect("stability must enter while the politics ledger is active");
        } else if observed.hour == 11 && matches!(family, "stability" | "politics_ledger") {
            assert_eq!(
                count.load(Ordering::SeqCst),
                2,
                "next frame arrived before both prior observers completed"
            );
        }
        lanes::Fault::Continue
    });
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("lane-barrier");
    // The politics ledger is prepared on its own; the walk's lane job for it
    // runs only while every family is prepared hour by hour.
    independent::with_test_hourly(|| {
        lanes::with_test_lanes(4, Some(hook), || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    })
    .unwrap();
    assert_eq!(entered.load(Ordering::SeqCst), 2);
}

#[test]
fn family_lanes_reject_an_unjoined_second_frame_and_close_on_coordinator_unwind() {
    let first = frame(1, 10, 2);
    let next = frame(1, 11, 2);
    std::thread::scope(|scope| {
        let jobs = vec![
            lanes::Job::new(&[("one", 0)], 1, |_, _, _, _| Ok(())),
            lanes::Job::new(&[("two", 1)], 1, |_, _, _, _| Ok(())),
        ];
        let mut workers = lanes::Lanes::start(scope, jobs, 4, false, &Refused::default());
        workers
            .begin(
                &first,
                FrameChangeTracker::new()
                    .observe(&first, replay_data::format::FormatVersion::CURRENT),
            )
            .unwrap();
        assert_eq!(
            workers
                .begin(
                    &next,
                    FrameChangeTracker::new()
                        .observe(&next, replay_data::format::FormatVersion::CURRENT)
                )
                .err()
                .expect("second pending frame must be rejected")
                .kind(),
            io::ErrorKind::InvalidInput
        );
        workers
            .finish(Ok(()), 0, &mut None, &mut Refused::default(), 10)
            .unwrap();
        workers
            .begin(
                &next,
                FrameChangeTracker::new()
                    .observe(&next, replay_data::format::FormatVersion::CURRENT),
            )
            .unwrap();
        workers
            .finish(Ok(()), 0, &mut None, &mut Refused::default(), 10)
            .unwrap();
    });
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        std::thread::scope(|scope| {
            let jobs = vec![lanes::Job::new(&[("one", 0)], 1, |_, _, _, _| Ok(()))];
            let mut workers = lanes::Lanes::start(scope, jobs, 4, false, &Refused::default());
            workers
                .begin(
                    &first,
                    FrameChangeTracker::new()
                        .observe(&first, replay_data::format::FormatVersion::CURRENT),
                )
                .unwrap();
            panic!("coordinator unwind with a frame dispatched");
        })
    }));
    assert!(unwound.is_err());
}

#[test]
fn family_lanes_join_all_results_and_select_the_original_call_error() {
    let finished = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    std::thread::scope(|scope| {
        let earlier = Arc::clone(&finished);
        let later = Arc::clone(&finished);
        let jobs = vec![
            lanes::Job::new(&[("earlier", 1)], 1, move |_, _, _, _| {
                earlier.fetch_add(1, Ordering::SeqCst);
                Err(io::Error::other("earlier observer"))
            }),
            lanes::Job::new(&[("later", 9)], 1, move |_, _, _, _| {
                later.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }),
        ];
        let mut workers = lanes::Lanes::start(scope, jobs, 4, false, &Refused::default());
        let observed = frame(1, 10, 2);
        workers
            .begin(
                &observed,
                FrameChangeTracker::new()
                    .observe(&observed, replay_data::format::FormatVersion::CURRENT),
            )
            .unwrap();
        let error = workers
            .finish(
                Err(io::Error::other("coordinator")),
                5,
                &mut None,
                &mut Refused::default(),
                10,
            )
            .unwrap_err();
        assert_eq!(error.to_string(), "earlier observer");
        assert_eq!(
            finished.load(Ordering::SeqCst),
            2,
            "every lane must join even on error"
        );
    });
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_cancel_error_panic_disconnect_discard_the_whole_block_and_resume() {
    independent::with_test_hourly(check_lane_failures);
}

fn check_lane_failures() {
    for failure in 0..4 {
        for hour in [20, 21] {
            let mut fixture = Fixture::new(&recording());
            let expected = serial_control(&mut fixture, "lane-failure", None);
            let source = fixture.source("lane-failure");
            let flag = Arc::new(AtomicBool::new(false));
            // Retain the exact accepted first block as the reference prefix.
            run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
            let prefix = payloads(&source, None);
            let cancellation = Arc::clone(&flag);
            let hook: lanes::Hook = Arc::new(move |family, frame, _changes| {
                if family != "trade" || frame.hour != hour {
                    return lanes::Fault::Continue;
                }
                match failure {
                    0 => {
                        cancellation.store(true, Ordering::Release);
                        lanes::Fault::Continue
                    }
                    1 => lanes::Fault::Error,
                    2 => lanes::Fault::Panic,
                    _ => lanes::Fault::Disconnect,
                }
            });
            let error = lanes::with_test_lanes(4, Some(hook), || run(&source, None, &flag, |_| {}))
                .unwrap_err();
            assert_eq!(
                error.kind(),
                match failure {
                    0 => io::ErrorKind::Interrupted,
                    3 => io::ErrorKind::BrokenPipe,
                    _ => io::ErrorKind::Other,
                }
            );
            assert_payloads_equal(
                &payloads(&source, None),
                &prefix,
                "failed block must not publish",
            );
            assert_eq!(unprepared_blocks(&source, None).unwrap(), Some(1));
            flag.store(false, Ordering::Release);
            lanes::with_test_lanes(4, None, || run(&source, None, &flag, |_| {})).unwrap();
            assert_payloads_equal(
                &payloads(&source, None),
                &expected,
                "resumed lanes vs serial control",
            );
        }
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn family_lanes_append_and_resume_match_every_serial_section() {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 2), frame(1, 11, 2)])],
    )
    .unwrap();
    let appended = first
        .append(&[continuation_records(
            &first,
            1,
            vec![frame(1, 12, 2), frame(1, 13, 2)],
        )])
        .unwrap();
    let mut fixture = Fixture::new(&first);
    let flag = AtomicBool::new(false);
    let mut expected = None;
    for parallelism in [2, 2, 4] {
        fixture.replace(&first);
        let source = fixture.source("lane-append");
        lanes::with_test_lanes(parallelism, None, || run(&source, None, &flag, |_| {})).unwrap();
        fixture.replace(&appended);
        drop(source);
        let source = fixture.source("lane-append");
        lanes::with_test_lanes(parallelism, None, || run(&source, None, &flag, |_| {})).unwrap();
        let actual = payloads(&source, None);
        if let Some(expected) = &expected {
            assert_payloads_equal(
                &actual,
                expected,
                "serial control and lanes must match the same append plan",
            );
        } else {
            expected = Some(actual);
        }
        super::super::super::files(&source, None)
            .unwrap()
            .0
            .clear()
            .unwrap();
        drop(source);
    }
}

#[test]
fn family_lanes_measured_work_overlaps_critical_path_without_double_counting_residual() {
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("lane-timings");
    let (accepted, timings) = lanes::with_test_lanes(4, None, || {
        run_measured(
            &source,
            None,
            &AtomicBool::new(false),
            false,
            Duration::ZERO,
            |_| {},
        )
    })
    .unwrap();
    assert_eq!(accepted.next_block, 1);
    assert_eq!(timings.worker_lanes, 2);
    assert!(timings.worker_observe > Duration::ZERO);
    assert!(timings.observe <= timings.wall);
    assert_eq!(
        family_time(timings.families.as_ref().unwrap()) - timings.worker_observe + timings.residual,
        timings.wall
    );
}

#[cfg(feature = "slow-tests")]
#[test]
fn family_lanes_five_thousand_countries_match_every_section() {
    let replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, vec![frame(1, 10, 5_000), frame(1, 11, 5_000)])],
    )
    .unwrap();
    compare_fixture(&replay);
}

#[cfg(feature = "slow-tests")]
#[test]
fn family_lanes_missing_and_modded_content_match_every_section_and_warning_byte() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation");
    for modded in [false, true] {
        let frames = vec![frame(1, 10, 2), frame(1, 11, 2)];
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames.clone())]).unwrap();
        let mut fixture = Fixture::new(&replay);
        let game = fixture.root.join("game");
        let (seed, context) = crate::test_fixtures::source_with_content(frames.clone(), &game);
        if modded {
            let addon = fixture.root.join("addon");
            std::fs::create_dir_all(addon.join("common/national_focus")).unwrap();
            std::fs::create_dir_all(addon.join("localisation/english")).unwrap();
            std::fs::write(
                addon.join("common/national_focus/fixture.txt"),
                "focus_tree={ id=fixture_tree default=yes focus={ id=mod/focus x=0 y=0 cost=1 } }",
            )
            .unwrap();
            std::fs::write(
                addon.join("localisation/english/fixture_l_english.yml"),
                "\u{feff}l_english:\n mod/focus:0 \"Modded focus\"\n",
            )
            .unwrap();
            std::fs::create_dir_all(game.join("user/mod")).unwrap();
            std::fs::write(
                game.join("user/mod/fixture.mod"),
                format!(
                    "name=\"Lane fixture\" path=\"{}\"",
                    addon.to_string_lossy().replace('\\', "/")
                ),
            )
            .unwrap();
            std::fs::write(
                game.join("user/dlc_load.json"),
                r#"{"enabled_mods":["mod/fixture.mod"],"disabled_dlcs":[]}"#,
            )
            .unwrap();
            let (_, identity) = crate::test_fixtures::content_identity(&game);
            assert_eq!(identity.mods.len(), 1);
            assert_eq!(identity.mods[0].id, "fixture.mod");
            let mut input = records(1, frames);
            let replay::spool::Record::SessionStart(session) = &mut input[0] else {
                unreachable!()
            };
            session.identity = identity;
            fixture.replace(&Replay::from_records(CAMPAIGN.into(), &[input]).unwrap());
        } else {
            fixture.replace(seed.replay_ref());
        }
        drop(seed);
        let flag = AtomicBool::new(false);
        let serial = fixture.source("content-lane-parity");
        if modded {
            assert_eq!(serial.replay_ref().identity.mods.len(), 1);
            assert_eq!(serial.replay_ref().identity.mods[0].id, "fixture.mod");
            let content = serial
                .content(&context.content.as_ref().unwrap().options)
                .unwrap();
            // Folder layers retain canonical roots (verbatim paths on Windows).
            // Compare the same path representation, keeping the mount assertion.
            let addon = std::fs::canonicalize(fixture.root.join("addon")).unwrap();
            assert!(
                content
                    .layers
                    .recorded_source_files()
                    .unwrap()
                    .iter()
                    .any(|path| path.starts_with(&addon))
            );
            assert_eq!(
                content
                    .layers
                    .read("common/national_focus/fixture.txt")
                    .unwrap(),
                std::fs::read(addon.join("common/national_focus/fixture.txt")).unwrap()
            );
        }
        drop(serial);
        let expected = serial_control(
            &mut fixture,
            "content-lane-parity",
            context.content.as_ref(),
        );
        let parallel = fixture.source("content-lane-parity");
        lanes::with_test_lanes(4, None, || {
            run(&parallel, context.content.as_ref(), &flag, |_| {})
        })
        .unwrap();
        assert_payloads_equal(
            &payloads(&parallel, context.content.as_ref()),
            &expected,
            "content lanes vs serial control",
        );
    }
}

/// A branch flushed at a block end and continued in memory into its next block
/// publishes exactly what the branch read back from its saved state publishes.
#[test]
fn flushed_branches_continued_in_memory_publish_as_reloaded_ones() {
    let recordings = [
        two_blocks((10..=13).map(moved_families_frame).collect()),
        two_blocks(crate::products::unit_leaders::prepared::test_fixtures::outcome_frames(false)),
        two_blocks(
            (100..=103)
                .map(|hour| crate::products::wars::test_fixtures::scaled_recorded(hour, 3))
                .collect(),
        ),
        two_blocks((100..=103).map(project_and_sinking_frame).collect()),
    ];
    for replay in recordings {
        let mut fixture = Fixture::new(&replay);
        // A measured run publishes and commits at every block end.
        let mut prepare = |name: &str, reload: bool| {
            let source = fixture.source(name);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            super::super::CONTINUED.with(|continued| continued.set(0));
            run_measured_through(
                &source,
                None,
                &AtomicBool::new(false),
                true,
                Duration::ZERO,
                Some(4),
                |_| Ok(()),
                |_| {},
            )
            .unwrap();
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let published = payloads(&source, None);
            // Both sources open one path, so the next run must not reuse this store.
            super::super::super::files(&source, None)
                .unwrap()
                .0
                .clear()
                .unwrap();
            drop(source);
            (
                published,
                super::super::CONTINUED.with(std::cell::Cell::get),
            )
        };
        let (reloaded, none) = prepare("reloaded", true);
        let (continued, once) = prepare("continued", false);
        assert_eq!((none, once), (0, 1), "the second block continues the first");
        // Name the fields of differing JSON payloads (saved observer states are
        // compressed JSON) before comparing every byte.
        let decoded = |bytes: &[u8]| {
            let plain = zstd::decode_all(bytes).unwrap_or_else(|_| bytes.to_vec());
            serde_json::from_slice::<serde_json::Value>(&plain).ok()
        };
        let mut differences = Vec::new();
        for (key, bytes) in &continued {
            if let Some(other) = reloaded.get(key).filter(|other| *other != bytes)
                && let (Some(left), Some(right)) = (decoded(bytes), decoded(other))
            {
                json_differences(
                    &left,
                    &right,
                    format!("{}#{}", key.0, key.1),
                    &mut differences,
                );
            }
        }
        differences.truncate(40);
        assert!(
            differences.is_empty(),
            "continued vs reloaded fields: {differences:#?}"
        );
        assert_payloads_equal(&continued, &reloaded, "continued vs reloaded");
    }
}

fn json_differences(
    left: &serde_json::Value,
    right: &serde_json::Value,
    path: String,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (left, right) {
        (Value::Object(left), Value::Object(right)) => {
            for key in left
                .keys()
                .chain(right.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                match (left.get(key), right.get(key)) {
                    (Some(l), Some(r)) => json_differences(l, r, format!("{path}.{key}"), out),
                    (l, r) => out.push(format!("{path}.{key}: {l:?} / {r:?}")),
                }
            }
        }
        (Value::Array(l), Value::Array(r)) if l.len() == r.len() => {
            for (index, (l, r)) in l.iter().zip(r).enumerate() {
                json_differences(l, r, format!("{path}[{index}]"), out);
            }
        }
        _ if left != right => {
            let shown = |value: &Value| value.to_string().chars().take(300).collect::<String>();
            out.push(format!("{path}: {} / {}", shown(left), shown(right)));
        }
        _ => {}
    }
}
