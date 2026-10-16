//! Unit leaders prepared on their own, before the hourly walk, from blocks
//! read by their changes: they publish and save exactly what hourly
//! preparation does across a war whose combat operands, task force
//! positions, mission fuel and country balances move every hour while
//! commanders hold, a commander swap, combats starting and ending with their
//! results, a command reassigned, a skill rise, a renamed country, a gap,
//! two branches, append, resume, cancellation and refusals, with one and
//! several pre-pass workers and with blocks read whole; their queries answer
//! while the hourly walk has accepted nothing; and the fold observes only
//! the hours at which something leaders read changed.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use dashboard_base::prepared::kept::counted;
use independent::Own;

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Leaders (one per country) of the scenario.
const COUNTRIES: u32 = 8;

/// Branch 1's last hour.
const LAST: i32 = 216;

fn leader_frame(branch: u64, hour: i32, countries: u32) -> DecodedFrame {
    crate::products::unit_leaders::prepared::test_fixtures::war_frame(branch, hour, countries)
}

/// Branch 1's hours from 100 through `last` of the war
/// (`test_fixtures::war_frame`), without 136 and 137 (a gap inside the
/// second combat), in blocks of 12, then branch 2's one block (hours 20 to
/// 25) when `second`.
fn scenario_of(last: i32, second: bool, countries: u32) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&hour| leader_frame(1, hour, countries))
            .collect()
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
                (20..=25)
                    .map(|hour| leader_frame(2, hour, countries))
                    .collect(),
            )])
            .unwrap();
    }
    replay
}

fn scenario(last: i32, second: bool) -> Replay {
    scenario_of(last, second, COUNTRIES)
}

fn leaders() -> &'static dyn Own {
    &independent::leaders::LEADERS
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

/// Sections only one mode writes, or writes differently: the checkpoint
/// (which holds the family's progress), the families' own coverage pages
/// and each branch's state ([`assert_same`] compares it decoded).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
}

fn branch_state(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the family keeps in its
/// own page), the leaders' events, catalogue and continuation among them,
/// and each branch's state is hourly preparation's: the leaders' state is
/// saved in their own continuation in both modes, never in the branch state.
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
    for published in [
        "unit-leaders-events-1",
        "unit-leaders-1-catalogue",
        "unit-leaders-1",
    ] {
        assert!(
            actual.keys().any(|(name, _)| name == published),
            "{label}: nothing published as {published}"
        );
    }
    let mut states = 0;
    for ((name, range), bytes) in hourly
        .iter()
        .filter(|((name, _), _)| name.starts_with("preparation-branch-"))
    {
        let own = actual
            .get(&(name.clone(), *range))
            .unwrap_or_else(|| panic!("{label}: no {name}"));
        assert_eq!(branch_state(own), branch_state(bytes), "{label}: {name}");
        states += 1;
    }
    assert!(states > 0, "{label}: no branch state");
}

/// Blocks the family observed hour by hour, and folds that fell back.
fn notes() -> (usize, usize) {
    (
        prepass::noted("leaders.sequential"),
        prepass::noted("leaders.fallback"),
    )
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
fn leaders_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "leaders-parity", &plain));
    // Alone, and at once with the identity history.
    let registries: [Vec<&'static dyn Own>; 2] =
        [vec![leaders()], vec![leaders(), &independent::IDENTITIES]];
    for families in registries {
        for workers in [1, 3] {
            for together in [true, false] {
                let label = format!(
                    "{} families, {workers} workers, at once {together}",
                    families.len()
                );
                let before = notes();
                let own = prepass::with_test_prepass(workers, None, || {
                    independent::with_test_families(&families, || {
                        if together {
                            prepared(&mut fixture, "leaders-parity", &plain)
                        } else {
                            independent::with_test_one_after_another(|| {
                                prepared(&mut fixture, "leaders-parity", &plain)
                            })
                        }
                    })
                });
                // The identity history's own sections are its own.
                let own: Payloads = own
                    .into_iter()
                    .filter(|((name, _), _)| !name.starts_with("territory-identities-"))
                    .collect();
                let expected: Payloads = hourly
                    .iter()
                    .filter(|((name, _), _)| !name.starts_with("territory-identities-"))
                    .map(|(key, bytes)| (key.clone(), bytes.clone()))
                    .collect();
                assert_same(&own, &expected, &label, false);
                assert_eq!(
                    notes(),
                    before,
                    "{label}: every block folded from its extract"
                );
            }
        }
    }
}

#[test]
fn leaders_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = scenario(161, false);
    let grown = scenario(LAST, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    // Cancelled at the first accepted report, then prepared again; every
    // block commits on its own in both runs.
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
            let source = fixture.source("leaders-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("leaders-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&[leaders()], || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, false);
    }
}

/// The leaders' refusal of each branch, from the checkpoint in hourly
/// preparation, or from their own coverage page.
fn leader_refusals(source: &Source, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    [1, 2]
        .into_iter()
        .map(|branch| {
            let refusal = if own {
                store
                    .read(&key, &independent::coverage_section("leaders", branch), 0)
                    .unwrap()
                    .and_then(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                            .refused
                            .0
                            .get("leaders")
                            .cloned()
                    })
            } else {
                state
                    .refused
                    .get(&branch)
                    .and_then(|refused| refused.0.get("leaders").cloned())
            };
            (branch, refusal)
        })
        .collect()
}

#[test]
fn leaders_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // At a block's first hour, and at an hour inside a block whose leaders
    // carry on.
    for hour in [124, 128] {
        let label = format!("refusing at {hour}");
        let refusals = [("leaders", hour)];
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("leaders-refusal");
                measured(&source);
                let result = (payloads(&source, None), leader_refusals(&source, false));
                clear(source);
                result
            })
        });
        let source = fixture.source("leaders-refusal");
        let sequential = prepass::noted("leaders.sequential");
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&[leaders()], || measured(&source))
            })
        });
        assert_eq!(
            prepass::noted("leaders.sequential") - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        let own_refused = leader_refusals(&source, true);
        assert_eq!(own_refused, hourly_refused, "{label}");
        assert_eq!(own_refused[&1].as_ref().unwrap().hour, hour, "{label}");
        assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
        independent::with_test_families(&[leaders()], || {
            let error = refusal(&source, None, 1, "unit-leaders", &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(
                error.to_string(),
                format!("Injected refusal of leaders at hour {hour}"),
                "{label}"
            );
            refusal(&source, None, 2, "unit-leaders", &flag).unwrap();
        });
        clear(source);
    }
}

fn query(source: &Source, product: &str, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// The generals leaderboard's default request, the careers, and one
/// general's career, on `branch`.
fn requests(branch: &str) -> Vec<serde_json::Value> {
    let reference = crate::products::unit_leaders::prepared::test_fixtures::reference(2);
    vec![
        serde_json::json!({"branchId": branch, "selection": {"kind": "all"},
            "view": "leaderboard", "leaders": "all"}),
        serde_json::json!({"branchId": branch, "selection": {"kind": "all"},
            "view": "career"}),
        serde_json::json!({"branchId": branch,
            "selection": {"kind": "leader", "reference": reference.to_string()},
            "view": "career"}),
    ]
}

#[test]
fn leaders_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let branches = ["1", "2"];
    // The replies of hourly preparation, with every block accepted.
    let expected: Vec<Vec<serde_json::Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("leaders-hourly-replies");
        plain(&source);
        let replies = branches
            .iter()
            .map(|branch| {
                requests(branch)
                    .into_iter()
                    .map(|params| query(&source, "unit-leaders", params))
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    independent::with_test_families(&[leaders()], || {
        let source = fixture.source("leaders-stalled");
        let flag = AtomicBool::new(false);
        assert!(
            query(&source, "unit-leaders", requests("1").remove(0)).is_null(),
            "nothing is prepared yet"
        );
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
        for (position, branch) in branches.iter().enumerate() {
            let id = branch.parse().unwrap();
            assert!(
                coverage(&source, None, id, &flag).unwrap().is_none(),
                "branch {branch}: the hourly walk covers nothing"
            );
            assert!(
                complete(&source, None, id, "unit-leaders", &flag).unwrap(),
                "branch {branch}: the leaders cover the branch on their own"
            );
            for (index, params) in requests(branch).into_iter().enumerate() {
                let reply = query(&source, "unit-leaders", params);
                assert!(!reply.is_null(), "branch {branch}: request {index} answers");
                assert_eq!(
                    reply, expected[position][index],
                    "branch {branch}: request {index} answers as hourly preparation does"
                );
            }
        }
        clear(source);
    });
}

/// Read by their changes, the leaders' fold observes only each block's first
/// hour and the hours at which something leaders read changed: fewer than
/// half the recorded hours, although the combats, task forces, missions and
/// countries change every hour. Read whole, every hour is a frame and the
/// fold still observes only those hours. Both publish what hourly
/// preparation does, and no block falls back.
#[test]
fn leaders_observe_only_the_hours_whose_inputs_changed() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "leaders-frames", &plain));
    for whole in [true, false] {
        let label = format!("read whole {whole}");
        let before = (counted("leaders"), notes());
        let source = fixture.source("leaders-frames");
        let read = || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&[leaders()], || plain(&source))
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
        let (observed, frames) = counted("leaders");
        let (observed, frames) = (observed - before.0.0, frames - before.0.1);
        if whole {
            assert_eq!(frames, recorded, "{label}: a frame every hour");
        }
        assert!(
            observed * 2 < recorded,
            "{label}: {observed} hours observed of {recorded}"
        );
        assert!(observed <= frames, "{label}: {observed} of {frames} frames");
    }
}

/// Five thousand leaders of as many countries at war publish what hourly
/// preparation does.
#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn leaders_prepared_on_their_own_match_hourly_preparation_at_5000_countries() {
    let replay = scenario_of(160, false, 5000);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "leaders-scale", &plain));
    let own = prepass::with_test_prepass(3, None, || {
        independent::with_test_families(&[leaders()], || {
            prepared(&mut fixture, "leaders-scale", &plain)
        })
    });
    assert_same(&own, &hourly, "5000 countries", false);
}
