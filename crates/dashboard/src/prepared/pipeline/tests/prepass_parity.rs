//! Families prepared from blocks extracted ahead publish and save exactly
//! what hourly observation does.
use super::family_lanes::{assert_payloads_equal, payloads, serial_control};
use super::*;
use crate::prepared::frame_changes::{FrameChangeTracker, FrameChanges};
use crate::prepared::prepass;
use crate::products::resource_balance::prepared::{
    block::{self, test_fixtures::scenario},
    observed_hours,
};
use std::sync::Mutex;

/// Branch 1's resource scenario (runs, deals and memberships across block
/// boundaries, a gap, a holder change, growing and unread tables) in the
/// first `blocks` of its five blocks, then branch 2 when `second`.
pub(super) fn resources(blocks: usize, second: bool) -> Replay {
    let hours: [&[i32]; 5] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[110, 111, 112, 113],
        &[114, 115, 116, 117],
        &[118, 119, 121],
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| scenario(hour, None)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(hours[0]))]).unwrap();
    for &hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        let frames = (10..=13)
            .map(|hour| {
                let mut frame = scenario(hour, None);
                frame.branch = 2;
                frame
            })
            .collect();
        replay = replay.append(&[records(2, frames)]).unwrap();
    }
    replay
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// The hourly control, then the pre-pass with one and with three workers,
/// serially and on lanes: every payload, the saved branch state included,
/// must be identical, with no fold falling back.
fn assert_prepass_matches_hourly(replay: &Replay, label: &str) {
    let mut fixture = Fixture::new(replay);
    let expected = prepass::with_test_sequential(|| serial_control(&mut fixture, label, None));
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source(label);
        let noted = || {
            ["resources", "army"].map(|family| {
                (
                    prepass::noted(&format!("{family}.fallback")),
                    prepass::noted(&format!("{family}.sequential")),
                )
            })
        };
        let fallbacks = noted();
        prepass::with_test_prepass(workers, None, || {
            lanes::with_test_lanes(parallelism, None, || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap();
        assert_payloads_equal(
            &payloads(&source, None),
            &expected,
            &format!("{label}: pre-pass, {workers} workers, {parallelism} processors"),
        );
        assert_eq!(
            noted(),
            fallbacks,
            "no fold fell back, no block was observed hourly"
        );
        clear(source);
    }
}

#[test]
fn prepass_publishes_and_saves_every_section_as_hourly_observation() {
    assert_prepass_matches_hourly(&resources(5, true), "prepass-parity");
    assert_prepass_matches_hourly(
        &crate::products::army_size::prepared::block::test_fixtures::recording(),
        "prepass-army",
    );
}

#[test]
fn prepass_parity_detects_a_fold_that_drops_its_recorded_calls() {
    let replay = resources(5, false);
    let mut fixture = Fixture::new(&replay);
    let expected =
        prepass::with_test_sequential(|| serial_control(&mut fixture, "prepass-broken", None));
    let source = fixture.source("prepass-broken");
    block::with_skipped_replay(|| run(&source, None, &AtomicBool::new(false), |_| {})).unwrap();
    assert_ne!(
        payloads(&source, None),
        expected,
        "a fold without its recorded calls must publish differently"
    );
    clear(source);
}

/// Every block committed on its own and continued in memory or read back,
/// prepared a block per run (resume), and grown by an append: each publishes
/// and saves what hourly observation does.
#[test]
fn prepass_commits_resumes_reloads_and_appends_as_hourly_observation() {
    let first = resources(3, false);
    let grown = resources(5, true);
    let measured = |source: &Source, reload: bool| {
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
        let result = run_measured_through(
            source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| {},
        );
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
        result.unwrap();
    };
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    let mut fixture = Fixture::new(&first);
    let mut published = Vec::new();
    for (mode, sequential, reload) in [
        ("hourly", true, false),
        ("pre-pass", false, false),
        ("pre-pass reloaded", false, true),
        ("pre-pass resumed", false, false),
    ] {
        fixture.replace(&first);
        let prepare = |source: &Source| {
            if mode.ends_with("resumed") {
                resumed(source)
            } else {
                measured(source, reload)
            }
        };
        let source = fixture.source("prepass-commits");
        let grow = |fixture: &mut Fixture| {
            fixture.replace(&grown);
            fixture.source("prepass-commits")
        };
        let grown_source = if sequential {
            prepass::with_test_sequential(|| {
                prepare(&source);
                drop(source);
                let grown = grow(&mut fixture);
                prepare(&grown);
                grown
            })
        } else {
            prepare(&source);
            drop(source);
            let grown = grow(&mut fixture);
            prepare(&grown);
            grown
        };
        published.push((mode, payloads(&grown_source, None)));
        clear(grown_source);
    }
    let (_, hourly) = &published[0];
    for (mode, payloads) in &published[1..] {
        if mode.ends_with("resumed") {
            // Resumed runs commit the same blocks one run at a time.
            assert_payloads_equal(payloads, &published[1].1, mode);
        } else {
            assert_payloads_equal(payloads, hourly, mode);
        }
    }
}

/// A family refusal, at a block's first hour or inside one, has the hourly
/// path's hour, text and published and saved state: the refusing block is
/// observed hour by hour.
#[test]
fn prepass_refusals_match_hourly_observation() {
    let replay = resources(5, true);
    for hour in [104, 115] {
        let refusals = [("resources", hour)];
        let mut fixture = Fixture::new(&replay);
        let expected = prepass::with_test_sequential(|| {
            lanes::with_test_refusals(&refusals, || {
                serial_control(&mut fixture, "prepass-refusal", None)
            })
        });
        let source = fixture.source("prepass-refusal");
        let hourly = prepass::noted("resources.sequential");
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
        })
        .unwrap();
        assert_eq!(prepass::noted("resources.sequential") - hourly, 1);
        assert_payloads_equal(
            &payloads(&source, None),
            &expected,
            &format!("refusal at {hour}"),
        );
        clear(source);
    }
}

/// An extract that fails or panics hands its block to hourly observation;
/// cancellation during an extract publishes nothing of the block, and the
/// resumed preparation matches.
#[test]
fn prepass_failed_extracts_observe_hourly_and_cancellation_discards_the_block() {
    let replay = resources(5, false);
    let mut fixture = Fixture::new(&replay);
    let expected =
        prepass::with_test_sequential(|| serial_control(&mut fixture, "prepass-faults", None));
    for fault in [prepass::Fault::Error, prepass::Fault::Panic] {
        let hook: prepass::Hook = Arc::new(move |family, frame| {
            if family == "resources" && frame.hour == 105 {
                fault
            } else {
                prepass::Fault::Continue
            }
        });
        let source = fixture.source("prepass-faults");
        let hourly = prepass::noted("resources.sequential");
        prepass::with_test_prepass(2, Some(hook), || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
        .unwrap();
        assert_eq!(prepass::noted("resources.sequential") - hourly, 1);
        assert_payloads_equal(&payloads(&source, None), &expected, "failed extract");
        clear(source);
    }
    let flag = Arc::new(AtomicBool::new(false));
    // Hourly observation of the same plan: the first block alone, then the rest.
    let control = fixture.source("prepass-faults");
    prepass::with_test_sequential(|| {
        run_measured(&control, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        run(&control, None, &flag, |_| {}).unwrap();
    });
    let expected = payloads(&control, None);
    clear(control);
    let source = fixture.source("prepass-faults");
    // The first block alone is accepted; the pre-pass then starts at the second.
    run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
    let prefix = payloads(&source, None);
    let cancel = Arc::clone(&flag);
    let hook: prepass::Hook = Arc::new(move |family, frame| {
        if family == "resources" && frame.hour == 105 {
            cancel.store(true, Ordering::Release);
        }
        prepass::Fault::Continue
    });
    let error = prepass::with_test_prepass(2, Some(hook), || run(&source, None, &flag, |_| {}))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_payloads_equal(&payloads(&source, None), &prefix, "cancelled block");
    flag.store(false, Ordering::Release);
    run(&source, None, &flag, |_| {}).unwrap();
    assert_payloads_equal(&payloads(&source, None), &expected, "resumed after cancel");
    clear(source);
}

/// Every hour's resources observation runs on a pre-pass worker; the
/// coordinator observes resources once per block (its first hour, on the
/// carried state) where hourly observation observes every hour, and its fold
/// makes no more calls when a block's unchanged hours grow tenfold.
#[test]
fn prepass_extracts_on_workers_and_the_coordinator_folds_once_per_block() {
    let replay = resources(5, true);
    let coordinator = std::thread::current().id();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let trace = Arc::clone(&seen);
    let hook: prepass::Hook = Arc::new(move |family, frame| {
        if family == "resources" {
            trace
                .lock()
                .unwrap()
                .push((std::thread::current().id(), frame.branch, frame.hour));
        }
        prepass::Fault::Continue
    });
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("prepass-threads");
    let before = observed_hours();
    prepass::with_test_prepass(3, Some(hook), || {
        lanes::with_test_lanes(4, None, || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    })
    .unwrap();
    let coordinator_hours = observed_hours() - before;
    clear(source);
    let seen = std::mem::take(&mut *seen.lock().unwrap());
    assert!(seen.iter().all(|(thread, ..)| *thread != coordinator));
    let mut extracted: Vec<_> = seen
        .iter()
        .map(|&(_, branch, hour)| (branch, hour))
        .collect();
    extracted.sort_unstable();
    let mut hours = Vec::new();
    for index in &replay.index {
        replay
            .read_block_alone(
                index,
                &[TableId::Campaign],
                |_, frame| {
                    hours.push((frame.branch, frame.hour));
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    hours.sort_unstable();
    assert_eq!(
        extracted, hours,
        "every hour extracted once, off the coordinator"
    );
    assert_eq!(
        coordinator_hours,
        replay.index.len(),
        "one first hour per block"
    );
    // Hourly observation, serial, observes every hour on the coordinator.
    let source = fixture.source("prepass-threads");
    let before = observed_hours();
    prepass::with_test_sequential(|| run(&source, None, &AtomicBool::new(false), |_| {})).unwrap();
    assert_eq!(observed_hours() - before, hours.len());
    clear(source);
    // Blocks of 10 and of 100 unchanged hours fold the same recorded calls.
    let replayed = |hours: i32| {
        let frames = |from: i32| {
            (from..from + hours)
                .map(|hour| scenario(hour.min(103), None))
                .enumerate()
                .map(|(offset, mut frame)| {
                    frame.hour = from + offset as i32;
                    frame
                })
                .collect::<Vec<_>>()
        };
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, frames(1000))]).unwrap();
        for block in 1..3 {
            replay = replay
                .append(&[continuation_records(
                    &replay,
                    1,
                    frames(1000 + block * hours),
                )])
                .unwrap();
        }
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("prepass-flat");
        let before = block::replayed();
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        clear(source);
        block::replayed() - before
    };
    assert_eq!(replayed(10), replayed(100));
}

/// The change facts a block's frames get from a tracker of that block alone
/// are, hour by hour, those preparation hands its observers.
#[test]
fn block_change_facts_alone_equal_the_facts_preparation_observes() {
    type Facts = (
        Vec<bool>,
        Vec<bool>,
        Vec<bool>,
        Vec<Option<(Vec<(u128, u64)>, Vec<u128>)>>,
    );
    fn facts(changes: &FrameChanges) -> Facts {
        (
            changes.rows_changed.to_vec(),
            changes.table_read.to_vec(),
            changes.slot_read.to_vec(),
            TableId::ALL
                .iter()
                .map(|&table| {
                    changes.rows(table).map(|(changed, deleted)| {
                        (
                            changed.iter().map(|row| (row.key, row.mask)).collect(),
                            deleted.to_vec(),
                        )
                    })
                })
                .collect(),
        )
    }
    let replay = resources(5, true);
    let observed = Arc::new(Mutex::new(BTreeMap::new()));
    let trace = Arc::clone(&observed);
    let hook: lanes::Hook = Arc::new(move |family, frame, changes| {
        if family == "trade" {
            trace
                .lock()
                .unwrap()
                .insert((frame.branch, frame.hour), facts(changes));
        }
        lanes::Fault::Continue
    });
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("prepass-facts");
    independent::with_test_hourly(|| {
        lanes::with_test_lanes(2, Some(hook), || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    })
    .unwrap();
    clear(source);
    let mut alone = BTreeMap::new();
    for index in replay.index.iter().rev() {
        let mut tracker = FrameChangeTracker::new();
        replay
            .read_block_alone(
                index,
                &TableId::ALL,
                |raw, frame| {
                    let changes = tracker.observe_updates(frame, Arc::new(raw), replay.format);
                    alone.insert((frame.branch, frame.hour), facts(&changes));
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    assert_eq!(*observed.lock().unwrap(), alone);
}
