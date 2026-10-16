use super::*;
use std::{io::Read, sync::Mutex, time::Duration};

// Enumerate the documented manifest header, but open every registered payload
// through the real store reader. UUIDs and physical file names are not payloads.
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
    let mut manifest = std::fs::File::open(directory.join("manifest")).unwrap();
    let mut length = [0; 8];
    manifest.read_exact(&mut length).unwrap();
    let header: serde_json::Value =
        serde_json::from_reader(manifest.take(u64::from_le_bytes(length))).unwrap();
    let mut result = BTreeMap::new();
    for section in header["sections"].as_array().unwrap() {
        let name = section["name"].as_str().unwrap();
        for range in 0..section["count"].as_u64().unwrap() {
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
    use replay::rows::{CountryLinkRow, FactionMemberRow, FactionRow};
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
    // Territory/history/supply/groups run on the coordinator; subject/tension
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
        lanes::with_test_lanes(parallelism, Some(hook), || {
            run(&source, None, &AtomicBool::new(false), |_| {})
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
            assert_eq!(*workers, 20, "all 20 numeric jobs see the same fact");
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
    let (resource_send, resource_receive) = mpsc::sync_channel(1);
    let (stability_send, stability_receive) = mpsc::sync_channel(1);
    let resource_receive = Mutex::new(resource_receive);
    let stability_receive = Mutex::new(stability_receive);
    let entered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let count = Arc::clone(&entered);
    let resource_frame = std::sync::atomic::AtomicUsize::new(0);
    let stability_frame = std::sync::atomic::AtomicUsize::new(0);
    let hook: lanes::Hook = Arc::new(move |family, observed, _changes| {
        if observed.hour == 10 && family == "resources" {
            resource_frame.store(observed as *const DecodedFrame as usize, Ordering::SeqCst);
            count.fetch_add(1, Ordering::SeqCst);
            resource_send.send(()).unwrap();
            stability_receive
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .expect("stability must enter while resources is active");
            assert_eq!(
                resource_frame.load(Ordering::SeqCst),
                stability_frame.load(Ordering::SeqCst),
                "lanes must share the same frame allocation"
            );
        } else if observed.hour == 10 && family == "stability" {
            stability_frame.store(observed as *const DecodedFrame as usize, Ordering::SeqCst);
            count.fetch_add(1, Ordering::SeqCst);
            stability_send.send(()).unwrap();
            resource_receive
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .expect("resources must enter while stability is active");
        } else if observed.hour == 11 && matches!(family, "resources" | "stability") {
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
    lanes::with_test_lanes(4, Some(hook), || {
        run(&source, None, &AtomicBool::new(false), |_| {})
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
                FrameChangeTracker::new().observe(&first, replay::format::FormatVersion::CURRENT),
            )
            .unwrap();
        assert_eq!(
            workers
                .begin(
                    &next,
                    FrameChangeTracker::new()
                        .observe(&next, replay::format::FormatVersion::CURRENT)
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
                FrameChangeTracker::new().observe(&next, replay::format::FormatVersion::CURRENT),
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
                        .observe(&first, replay::format::FormatVersion::CURRENT),
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
                    .observe(&observed, replay::format::FormatVersion::CURRENT),
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
                if family != "resources" || frame.hour != hour {
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
    engine::executable_tables::initialize_for_tests()
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
