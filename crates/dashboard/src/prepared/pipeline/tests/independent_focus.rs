//! The national focus order prepared on its own, before the hourly walk,
//! from blocks read by their changes: it publishes and saves exactly what
//! hourly preparation does across quiet spans, fields it does not watch
//! changing every hour (country balances, the focus tree), duplicate tags,
//! text that is not UTF-8, a reset, an identity replacement, unread tables,
//! a pause longer than a block, progress changing at a block's first hour,
//! two branches, a gap, append, resume and refusals, with one and several
//! pre-pass workers, at once or one after another; where commits fall never
//! changes a reply; its queries answer while the hourly walk has accepted
//! nothing; and its quiet hours build no frame.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::test_fixtures::scale_tag;
use independent::{Own, focus};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Branch 1's last hour.
const LAST: i32 = 200;

/// Six countries (`scale_tag(0..6)`, ids 100,000 to 100,005), whose balances
/// move every hour, with their focus at `hour` of `branch`. The focus tree
/// field moves every hour and is never read. Every progress and completion
/// change falls on the hours twelve apart from 100 (112 and 124 are blocks'
/// first hours):
/// - A completes `a0`, `a1`, ...; paused from 140 to 165, progress held.
/// - B completes every 24 hours; its list resets to `b_reset` at 125.
/// - Country 5 also holds B's tag from 110 to 121 (the higher id wins).
/// - C is country 2 until 129, then country 4 (an identity replacement).
/// - D lists a token that is not UTF-8 from 115 and has no focus row at 133
///   and 134.
/// - Focus is not read at 150 and 151; CompletedFocuses is truncated at 170.
fn focus_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 6);
    let step = i64::from(hour - 100).max(0);
    let tag = |index: u32| -> Vec<u8> {
        match index {
            2 if hour >= 130 => b"OLD".to_vec(),
            4 if hour >= 130 => scale_tag(2),
            4 => b"NEW".to_vec(),
            5 if (110..=121).contains(&hour) => scale_tag(1),
            _ => scale_tag(index),
        }
    };
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        for (index, row) in rows.iter_mut().enumerate() {
            row.tag = tag(index as u32);
        }
    }
    let mut focus = Vec::new();
    let mut completed = Vec::new();
    for index in 0..6_u32 {
        if index == 3 && (133..=134).contains(&hour) {
            continue;
        }
        let country = 100_000 + index;
        let paused = index == 0 && (140..=165).contains(&hour);
        let held = if paused { 139 } else { i64::from(hour) };
        let (tokens, current): (Vec<Vec<u8>>, Option<&[u8]>) = match index {
            0 => (
                (0..step / 12)
                    .map(|n| format!("a{n}").into_bytes())
                    .collect(),
                Some(&b"a_next"[..]),
            ),
            1 if hour >= 125 => (
                std::iter::once(b"b_reset".to_vec())
                    .chain((0..(step - 25) / 24).map(|n| format!("r{n}").into_bytes()))
                    .collect(),
                Some(&b"b_next"[..]),
            ),
            1 => (
                (0..step / 24)
                    .map(|n| format!("b{n}").into_bytes())
                    .collect(),
                Some(&b"b_next"[..]),
            ),
            3 if hour >= 115 => (vec![b"d0".to_vec(), b"d\xff".to_vec()], Some(&b"d\xfe"[..])),
            _ => (vec![format!("c{index}").into_bytes()], None),
        };
        focus.push(FocusRow {
            country,
            tree: Some(format!("tree{hour}").into_bytes()),
            current: current.map(<[u8]>::to_vec),
            continuous: (index == 2).then(|| b"continuous".to_vec()),
            progress: (held - 100).max(0) / 12 * (i64::from(index) + 1),
            paused: u8::from(paused),
            completed_count: tokens.len() as i32,
        });
        completed.extend(tokens.into_iter().enumerate().map(|(ordinal, token)| {
            CompletedFocusRow {
                country,
                ordinal: ordinal as u32,
                token,
            }
        }));
    }
    *observed.tables[TableId::Focus.index()] = if (150..=151).contains(&hour) {
        TableView::Unavailable {
            reason: "Synthetic focus read unavailable".into(),
        }
    } else {
        TableView::Complete(TableRows::Focus(focus))
    };
    *observed.tables[TableId::CompletedFocuses.index()] = if hour == 170 {
        TableView::Truncated {
            rows: TableRows::CompletedFocuses(Vec::new()),
            missing: 3,
        }
    } else {
        TableView::Complete(TableRows::CompletedFocuses(completed))
    };
    observed
}

/// Branch 1's hours from 100 through `last`, without 136 and 137 (a gap
/// between blocks), in blocks of 12, then branch 2's one block (hours 20 to
/// 31) when `second`.
fn scenario(last: i32, second: bool) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| focus_frame(1, hour)).collect()
    };
    let mut blocks = hours.chunks(12);
    let mut replay = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, frames(blocks.next().unwrap()))],
    )
    .unwrap();
    for block in blocks {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=31).map(|hour| focus_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

fn families() -> Vec<&'static dyn Own> {
    vec![&focus::FOCUS]
}

fn clear(source: Arc<Source>) {
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
}

/// Every block committed on its own.
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

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
}

/// Prepare a fresh source with `prepare` and take its payloads.
fn prepared(fixture: &mut Fixture, name: &str, prepare: &dyn Fn(&Source)) -> Payloads {
    let source = fixture.source(name);
    prepare(&source);
    let payloads = payloads(&source, None);
    clear(source);
    payloads
}

/// Sections only one mode writes, or writes differently: the checkpoint, the
/// family's coverage pages and continuation, and each branch's state, which
/// holds the continuation only in hourly preparation ([`assert_same`]
/// compares both).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.starts_with("focus-state-")
}

fn branch_state(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the family keeps in its
/// own page). Without refusals, the family's continuation is the one hourly
/// preparation keeps in each branch's state as `index`, and the rest of
/// that state is the same.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, refusals: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && !(refusals && name.starts_with("preparation-coverage-"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for published in ["focus-1", "focus-1-catalogue", "focus-completions-1"] {
        assert!(
            actual.keys().any(|(name, _)| name == published),
            "{label}: nothing published as {published}"
        );
    }
    if refusals {
        return;
    }
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        states += 1;
        let mut expected = branch_state(bytes);
        let mut own = branch_state(&actual[&(name.clone(), *range)]);
        let section = actual
            .get(&(focus::focus_state(branch.parse().unwrap()), 0))
            .unwrap_or_else(|| panic!("{label}: no focus continuation of branch {branch}"));
        let continuation: serde_json::Value = serde_json::from_slice(section).unwrap();
        let value = expected.as_object_mut().unwrap().remove("index").unwrap();
        own.as_object_mut().unwrap().remove("index");
        assert_eq!(
            continuation, value,
            "{label}: branch {branch}'s focus state"
        );
        assert_eq!(own, expected, "{label}: branch {branch}'s other state");
    }
    assert!(states > 0, "{label}: no branch state");
}

/// Blocks the family observed hour by hour, and folds that fell back.
fn notes() -> [usize; 2] {
    [
        prepass::noted("focus.sequential"),
        prepass::noted("focus.fallback"),
    ]
}

/// Hours recorded in `replay`.
fn hours(replay: &Replay) -> usize {
    let mut hours = 0;
    for index in &replay.index {
        replay
            .read_block_alone(
                index,
                &[TableId::Campaign],
                |_, _| {
                    hours += 1;
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    hours
}

#[test]
fn focus_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "focus-parity", &plain));
    for workers in [1, 3] {
        for together in [true, false] {
            let label = format!("{workers} workers, at once {together}");
            let before = notes();
            let own = prepass::with_test_prepass(workers, None, || {
                independent::with_test_families(&families(), || {
                    if together {
                        prepared(&mut fixture, "focus-parity", &plain)
                    } else {
                        independent::with_test_one_after_another(|| {
                            prepared(&mut fixture, "focus-parity", &plain)
                        })
                    }
                })
            });
            assert_same(&own, &hourly, &label, false);
            assert_eq!(
                notes(),
                before,
                "{label}: every block folded from its extract"
            );
        }
    }
}

#[test]
fn focus_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    let first = scenario(161, false);
    let grown = scenario(LAST, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    // Cancelled at the first accepted report, then prepared again.
    let cancelled = |source: &Source| {
        let flag = AtomicBool::new(false);
        match run_measured_through(
            source,
            None,
            &flag,
            true,
            Duration::ZERO,
            Some(u64::MAX),
            |_| Ok(()),
            |_| flag.store(true, Ordering::Release),
        ) {
            Ok(_) => {}
            Err(error) => assert_eq!(error.kind(), io::ErrorKind::Interrupted),
        }
        flag.store(false, Ordering::Release);
        measured(source);
    };
    let modes: [(&str, &dyn Fn(&Source), bool); 5] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
        ("cancelled", &cancelled, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("focus-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("focus-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&families(), || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, false);
    }
}

fn query(
    source: &Source,
    product: &str,
    params: serde_json::Value,
) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

/// Every focus reply of every country at every recorded hour of both
/// branches: its hour, its every order page, and the tables' refusals.
fn replies(source: &Source) -> Vec<serde_json::Value> {
    let mut replies = Vec::new();
    for (branch, hours) in [
        ("1", (100..=LAST).collect::<Vec<_>>()),
        ("2", (20..=31).collect()),
    ] {
        for hour in hours {
            replies.push(
                query(
                    source,
                    "focus-unavailable",
                    serde_json::json!({ "branchId": branch, "hour": hour }),
                )
                .unwrap(),
            );
            for index in 0..6 {
                let tag = String::from_utf8(scale_tag(index)).unwrap();
                let row = query(
                    source,
                    "focus-hour",
                    serde_json::json!({ "branchId": branch, "tag": tag, "hour": hour }),
                )
                .unwrap();
                if !row.is_null() {
                    let pages: u64 = row["completed"]["count"].as_str().unwrap().parse().unwrap();
                    for page in 0..pages {
                        replies.push(
                            query(
                                source,
                                "focus-order-page",
                                serde_json::json!({
                                    "branchId": branch,
                                    "tag": tag,
                                    "hour": hour,
                                    "pages": row["completed"],
                                    "page": page.to_string(),
                                }),
                            )
                            .unwrap(),
                        );
                    }
                }
                replies.push(row);
            }
        }
    }
    replies
}

/// Batches end every block, every two blocks, or only early (at powers of two)
/// and at branch ends: every reply, focus hours'
/// first and last hours and progress included, is the same, in both modes,
/// so no run breaks where a commit falls.
#[test]
fn focus_replies_do_not_depend_on_commit_points() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let mut answered = |name: &str, blocks: usize, own: bool| -> Vec<serde_json::Value> {
        super::super::with_test_batch_blocks(blocks, || {
            let prepare = |fixture: &mut Fixture| {
                let source = fixture.source(name);
                plain(&source);
                let replies = replies(&source);
                clear(source);
                replies
            };
            if own {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_families(&families(), || prepare(&mut fixture))
                })
            } else {
                independent::with_test_hourly(|| prepare(&mut fixture))
            }
        })
    };
    let single = answered("focus-batches", 10_000, true);
    assert!(
        single.iter().filter(|reply| !reply.is_null()).count() > 500,
        "the fixture answers"
    );
    for (blocks, own) in [(2, true), (2, false), (10_000, false), (1, true)] {
        assert_eq!(
            answered("focus-batches", blocks, own),
            single,
            "batches of {blocks} blocks, prepared on its own {own}"
        );
    }
}

/// The family's refusals of each branch, from the checkpoint in hourly
/// preparation, or from its own coverage pages.
fn family_refusals(source: &Source, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    let mut refusals = BTreeMap::new();
    for branch in [1, 2] {
        let refused = if own {
            store
                .read(&key, &independent::coverage_section("focus", branch), 0)
                .unwrap()
                .and_then(|file| {
                    serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                        .unwrap()
                        .refused
                        .0
                        .get("focus")
                        .cloned()
                })
        } else {
            state
                .refused
                .get(&branch)
                .and_then(|refused| refused.0.get("focus").cloned())
        };
        refusals.insert(branch, refused);
    }
    refusals
}

#[test]
fn focus_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // At a block's first hour, and at an hour inside a block that no field
    // the family reads changes.
    for hour in [124, 117] {
        let label = format!("focus refusing at {hour}");
        let refusals = [("focus", hour)];
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("focus-refusal");
                measured(&source);
                let result = (payloads(&source, None), family_refusals(&source, false));
                clear(source);
                result
            })
        });
        let source = fixture.source("focus-refusal");
        let sequential = prepass::noted("focus.sequential");
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&families(), || measured(&source))
            })
        });
        assert_eq!(
            prepass::noted("focus.sequential") - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        let own_refused = family_refusals(&source, true);
        assert_eq!(own_refused, hourly_refused, "{label}");
        assert_eq!(own_refused[&1].as_ref().unwrap().hour, hour, "{label}");
        assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
        independent::with_test_families(&families(), || {
            for product in ["focus-hour", "focus-order-page", "focus-unavailable"] {
                let error = refusal(&source, None, 1, product, &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(
                    error.to_string(),
                    format!("Injected refusal of focus at hour {hour}"),
                    "{label}"
                );
                refusal(&source, None, 2, product, &flag).unwrap();
            }
        });
        clear(source);
    }
}

#[test]
fn focus_hour_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let expected = independent::with_test_hourly(|| {
        let source = fixture.source("focus-hourly");
        plain(&source);
        let replies = replies(&source);
        clear(source);
        replies
    });
    independent::with_test_families(&families(), || {
        let source = fixture.source("focus-stalled");
        let flag = AtomicBool::new(false);
        let tag = String::from_utf8(scale_tag(0)).unwrap();
        let cold = query(
            &source,
            "focus-hour",
            serde_json::json!({ "branchId": "1", "tag": tag, "hour": LAST }),
        )
        .unwrap();
        assert!(!cold.is_null());
        assert_eq!(cold["orderFirstHour"], serde_json::Value::Null);
        assert!(
            cold["preparing"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("completedByHour"))
        );
        assert!(!cold["pending"].as_array().unwrap().is_empty());
        // The hourly walk stalls: it is cancelled at its first commit.
        let mut commits = 0;
        let error = run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| {
                commits += 1;
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(commits, 1);
        flag.store(false, Ordering::Release);
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        for branch in [1, 2] {
            assert!(coverage(&source, None, branch, &flag).unwrap().is_none());
            for product in ["focus-hour", "focus-order-page", "focus-unavailable"] {
                assert!(complete(&source, None, branch, product, &flag).unwrap());
            }
        }
        assert_eq!(replies(&source), expected);
        clear(source);
    });
}

/// Read by its changes, the family builds frames only at the hours a field
/// it reads changes and at each block's first hour: fewer than half the
/// recorded hours, though balances and the focus tree change every hour.
/// Read whole, every hour is a frame. Both publish what hourly preparation
/// does.
#[test]
fn quiet_focus_hours_build_no_frame() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "focus-frames", &plain));
    for whole in [true, false] {
        let label = format!("read whole {whole}");
        let before = (crate::prepared_focus::family::frames(), notes());
        let source = fixture.source("focus-frames");
        let read = || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&families(), || plain(&source))
            })
        };
        if whole {
            prepass::with_test_alone(read);
        } else {
            read();
        }
        let payloads = payloads(&source, None);
        clear(source);
        assert_eq!(notes(), before.1, "{label}: every block folded");
        assert_same(&payloads, &hourly, &label, false);
        let frames = crate::prepared_focus::family::frames() - before.0;
        if whole {
            assert_eq!(frames, recorded, "{label}: a frame every hour");
        } else {
            assert!(
                frames * 2 < recorded,
                "{label}: {frames} frames of {recorded} hours"
            );
        }
    }
}

/// 5,000 countries, every one with a focus, across commits: the family's
/// pages and continuation are hourly preparation's.
#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn focus_prepared_on_its_own_matches_hourly_preparation_at_5000_countries() {
    const COUNTRIES: u32 = 5_000;
    let scaled = |hour: i32| {
        let mut observed = frame(1, hour, COUNTRIES);
        if let TableView::Complete(TableRows::Focus(rows)) =
            &mut *observed.tables[TableId::Focus.index()]
        {
            for (index, row) in rows.iter_mut().enumerate() {
                row.progress = i64::from(hour / 4) + index as i64;
                row.completed_count = 1 + i32::from(hour >= 6 && index % 3 == 0);
            }
        }
        *observed.tables[TableId::CompletedFocuses.index()] =
            TableView::Complete(TableRows::CompletedFocuses(
                (0..COUNTRIES)
                    .flat_map(|index| {
                        let more = hour >= 6 && index % 3 == 0;
                        (0..1 + u32::from(more)).map(move |ordinal| CompletedFocusRow {
                            country: 100_000 + index,
                            ordinal,
                            token: format!("mod/completed{ordinal}").into_bytes(),
                        })
                    })
                    .collect(),
            ));
        observed
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, (0..4).map(scaled).collect())]).unwrap();
    for first in [4, 8] {
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                (first..first + 4).map(scaled).collect(),
            )])
            .unwrap();
    }
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "focus-scale", &measured));
    let own = prepass::with_test_prepass(3, None, || {
        independent::with_test_families(&families(), || {
            prepared(&mut fixture, "focus-scale", &measured)
        })
    });
    assert_same(&own, &hourly, "5,000 countries", false);
}
