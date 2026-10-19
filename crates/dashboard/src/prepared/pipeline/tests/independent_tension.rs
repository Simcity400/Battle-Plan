//! World tension prepared on its own, before the hourly walk, from blocks
//! read by their changes: it publishes and saves exactly what hourly
//! preparation does across quiet spans (reads every hour with equal rows),
//! daily ramps, a country renamed and one gone that still contributes under
//! its last tag, a source from a country no link names, diagnostics, a gap,
//! two branches, append, resume and refusals, with one and several pre-pass
//! workers; its queries answer while the hourly walk has accepted nothing;
//! and its quiet hours build no frame.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use dashboard_base::prepared::kept::counted;
use engine_stability::cworld_threat;
use independent::Own;
use replay_data::{rows::ThreatCountryAggregateRow, rows::ThreatSourceRow, rows::WorldTensionRow};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;
const D: u32 = 100_003;
/// A raw country reference no link names; its slot names no country row.
const UNLINKED: u32 = 999;
const UNNAMED_SLOT: u32 = 77;

/// Branch 1's last hour.
const LAST: i32 = 216;

/// The tension tables at `hour` of `branch` over the four countries A to D
/// (`frame`), whose balances move every hour. Every table is read every hour
/// (world tension is hourly in the current format), mostly with equal rows.
///
/// A's source ramps once a day (at each day's first hour, inside blocks) and
/// its added value changes at 150; B's source is listed from 120 to 170; a
/// source of a country no link names, counted in a slot no country holds,
/// from 130 to 160; C is renamed at 140 and its source is listed from 140 to
/// 150; D's source is listed throughout, and D's country and link rows are
/// gone from 180, so its slot keeps contributing under its last tag. World
/// tension is not the cap of the sum from 160 to 163 (a diagnostic), and the
/// aggregates are not recorded at 195 and 196 (a segment break).
fn tension_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 4);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        if hour >= 140 {
            rows[2].tag = b"RNM".to_vec();
        }
        if hour >= 180 {
            rows.retain(|row| row.id != D);
        }
    }
    if hour >= 180
        && let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *observed.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| row.id != D);
    }
    let ramp = 50_000 + 10_000 * i64::from((hour - 96).div_euclid(24));
    let mut listed: Vec<(u32, i64, i64, i32, &str)> = vec![(
        A,
        ramp,
        if hour >= 150 { 250_000 } else { 200_000 },
        90,
        "mod_threat_a",
    )];
    let mut aggregates = vec![(0, ramp)];
    if (120..=170).contains(&hour) {
        listed.push((B, 30_000, 30_000, 120, "mod_threat_b"));
        aggregates.push((1, 30_000));
    }
    if (140..=150).contains(&hour) {
        listed.push((C, 7_000, 7_000, 140, "mod_threat_c"));
        aggregates.push((2, 7_000));
    }
    listed.push((D, 5_000, 5_000, 95, "mod_threat_d"));
    aggregates.push((3, 5_000));
    if (130..=160).contains(&hour) {
        listed.push((UNLINKED, 10_000, 10_000, 130, "mod_threat_unlinked"));
        aggregates.push((UNNAMED_SLOT, 10_000));
    }
    let sum: i64 = aggregates.iter().map(|(_, total)| total).sum();
    let mismatch = i64::from((160..=163).contains(&hour));
    *observed.tables[TableId::WorldTension.index()] =
        TableView::Complete(TableRows::WorldTension(vec![WorldTensionRow {
            total_raw: sum.clamp(0, cworld_threat::CLAMP_CEILING) / cworld_threat::SCALAR_DIVISOR
                + mismatch,
            source_count: listed.len() as i32,
            country_aggregate_count: 4,
        }]));
    *observed.tables[TableId::ThreatSources.index()] =
        TableView::Complete(TableRows::ThreatSources(
            listed
                .iter()
                .enumerate()
                .map(
                    |(ordinal, &(country, current_raw, final_raw, date, reason))| ThreatSourceRow {
                        ordinal: ordinal as u32,
                        country,
                        second_country: 0,
                        current_raw,
                        final_raw,
                        daily_raw: 1_000,
                        date,
                        reason: reason.as_bytes().to_vec(),
                    },
                )
                .collect(),
        ));
    *observed.tables[TableId::ThreatCountryAggregates.index()] = if (195..=196).contains(&hour) {
        TableView::Unavailable {
            reason: "Aggregates not read".into(),
        }
    } else {
        TableView::Complete(TableRows::ThreatCountryAggregates(
            aggregates
                .into_iter()
                .map(|(country_slot, total_raw)| ThreatCountryAggregateRow {
                    country_slot,
                    total_raw,
                })
                .collect(),
        ))
    };
    observed
}

/// Branch 1's hours from 100 through `last`, without 136 and 137 (a gap
/// between blocks), in blocks of 12, then branch 2's one block (hours 20 to
/// 25) when `second`.
fn scenario(last: i32, second: bool) -> Replay {
    let hours: Vec<i32> = (100..=last)
        .filter(|hour| !(136..=137).contains(hour))
        .collect();
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| tension_frame(1, hour)).collect()
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
                (20..=25).map(|hour| tension_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

fn tension() -> &'static dyn Own {
    &independent::world_tension::TENSION
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
/// own page), world tension's series and catalogue among them, and each
/// branch's state is hourly preparation's: world tension is saved in its
/// catalogue in both modes, never in the branch state.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, refusals: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !(mode_specific(name) || (refusals && name.starts_with("preparation-coverage-")))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for published in ["tension-series-1", "world-tension-1-catalogue"] {
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
        prepass::noted("tension.sequential"),
        prepass::noted("tension.fallback"),
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
fn world_tension_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "tension-parity", &plain));
    // Alone, and at once with the identity history it is read with.
    let registries: [Vec<&'static dyn Own>; 2] =
        [vec![tension()], vec![tension(), &independent::IDENTITIES]];
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
                            prepared(&mut fixture, "tension-parity", &plain)
                        } else {
                            independent::with_test_one_after_another(|| {
                                prepared(&mut fixture, "tension-parity", &plain)
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
fn world_tension_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
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
    let modes: [PrepareReloadMode; 5] = [
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
            let source = fixture.source("tension-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("tension-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = prepass::with_test_prepass(2, None, || {
            independent::with_test_families(&[tension()], || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, false);
    }
}

/// World tension's refusal of each branch, from the checkpoint in hourly
/// preparation, or from its own coverage page.
fn tension_refusals(source: &Source, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    [1, 2]
        .into_iter()
        .map(|branch| {
            let refusal = if own {
                store
                    .read(&key, &independent::coverage_section("tension", branch), 0)
                    .unwrap()
                    .and_then(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                            .refused
                            .0
                            .get("tension")
                            .cloned()
                    })
            } else {
                state
                    .refused
                    .get(&branch)
                    .and_then(|refused| refused.0.get("tension").cloned())
            };
            (branch, refusal)
        })
        .collect()
}

#[test]
fn world_tension_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    // At a block's first hour, and at an hour inside a block whose rows are
    // the hour before's.
    for hour in [124, 128] {
        let label = format!("refusing at {hour}");
        let refusals = [("tension", hour)];
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("tension-refusal");
                measured(&source);
                let result = (payloads(&source, None), tension_refusals(&source, false));
                clear(source);
                result
            })
        });
        let source = fixture.source("tension-refusal");
        let sequential = prepass::noted("tension.sequential");
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&[tension()], || measured(&source))
            })
        });
        assert_eq!(
            prepass::noted("tension.sequential") - sequential,
            1,
            "{label}: the refusing block is observed hour by hour"
        );
        assert_same(&payloads(&source, None), &hourly, &label, true);
        let own_refused = tension_refusals(&source, true);
        assert_eq!(own_refused, hourly_refused, "{label}");
        assert_eq!(own_refused[&1].as_ref().unwrap().hour, hour, "{label}");
        assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
        independent::with_test_families(&[tension()], || {
            let error = refusal(&source, None, 1, "world-tension", &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(
                error.to_string(),
                format!("Injected refusal of tension at hour {hour}"),
                "{label}"
            );
            refusal(&source, None, 2, "world-tension", &flag).unwrap();
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

/// The widget's default requests (the share at the last hour) and a wider
/// read, for `branch` ending at `last`.
fn requests(branch: &str, last: i32) -> Vec<(&'static str, serde_json::Value)> {
    let family = serde_json::json!({"kind": "world-tension"});
    vec![
        (
            "series-state",
            serde_json::json!({"branchId": branch, "family": family, "share": true,
                "reads": [{"hour": last}]}),
        ),
        (
            "series-state",
            serde_json::json!({"branchId": branch, "family": family,
                "reads": [{"hour": last - 5}, {"hour": last}]}),
        ),
        (
            "series-summary",
            serde_json::json!({"branchId": branch, "family": family,
                "from": last - 60, "to": last, "width": 30}),
        ),
    ]
}

#[test]
fn world_tension_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let branches = [("1", LAST), ("2", 25)];
    // The replies of hourly preparation, with every block accepted.
    let expected: Vec<Vec<serde_json::Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("tension-hourly-replies");
        plain(&source);
        let replies = branches
            .iter()
            .map(|&(branch, last)| {
                requests(branch, last)
                    .into_iter()
                    .map(|(product, params)| query(&source, product, params))
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    // The identity history the replies name contributors with is prepared on
    // its own too.
    independent::with_test_families(&[tension(), &independent::IDENTITIES], || {
        let source = fixture.source("tension-stalled");
        let flag = AtomicBool::new(false);
        let (product, params) = requests("1", LAST).remove(0);
        let cold = query(&source, product, params.clone());
        assert!(
            !cold.is_null(),
            "the current frame answers before any preparation"
        );
        assert_eq!(
            cold["reads"][0]["tensionRaw"],
            expected[0][0]["reads"][0]["tensionRaw"]
        );
        assert!(!cold["preparing"].as_array().unwrap().is_empty());
        let reads = dashboard_base::prepared::query::frames::reads(&source);
        assert_eq!(query(&source, product, params), cold);
        assert_eq!(
            dashboard_base::prepared::query::frames::reads(&source),
            reads
        );
        let summary_params = serde_json::json!({"branchId":"1", "family":{"kind":"world-tension"},
            "from":source.block_windows(1)[0].first_hour, "to":LAST, "width":1});
        let summary = query(&source, "series-summary", summary_params.clone());
        assert_eq!(summary["available"], true);
        assert!(
            !summary["summaries"]["world"]["approximate"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        let samples =
            dashboard_base::prepared::query::observations::usage(&source, "world-tension:keyframe")
                .2;
        assert_eq!(
            samples,
            source
                .replay_ref()
                .index
                .iter()
                .filter(|block| block.branch == 1)
                .count()
        );
        assert_eq!(query(&source, "series-summary", summary_params), summary);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&source, "world-tension:keyframe")
                .2,
            samples
        );
        // The hourly walk stalls: it is cancelled at its first commit.
        let mut commits = 0;
        let error = independent::with_test_one_after_another(|| {
            run_checked(
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
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(commits, 1);
        flag.store(false, Ordering::Release);
        assert_eq!(
            unprepared_blocks(&source, None).unwrap(),
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        for (position, &(branch, last)) in branches.iter().enumerate() {
            let id = branch.parse().unwrap();
            assert!(
                coverage(&source, None, id, &flag).unwrap().is_none(),
                "branch {branch}: the hourly walk covers nothing"
            );
            assert!(
                complete(&source, None, id, "world-tension", &flag).unwrap(),
                "branch {branch}: world tension covers the branch on its own"
            );
            for (index, (product, params)) in requests(branch, last).into_iter().enumerate() {
                let reply = query(&source, product, params);
                assert!(!reply.is_null(), "branch {branch}: {product} answers");
                assert_eq!(
                    reply, expected[position][index],
                    "branch {branch}: {product} answers as hourly preparation does"
                );
            }
        }
        clear(source);
    });
}

/// Read by its changes, world tension alone builds frames only at the hours
/// a row, key or link it reads changes and at each block's first hour: fewer
/// than half the recorded hours although every hour reads its tables, every
/// one kept and observed once on the carried observer. Read whole, every
/// hour is a frame. Both publish what hourly preparation does.
#[test]
fn world_tension_quiet_hours_build_no_frame() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| prepared(&mut fixture, "tension-frames", &plain));
    for whole in [true, false] {
        let label = format!("read whole {whole}");
        let before = (counted("tension"), notes());
        let source = fixture.source("tension-frames");
        let read = || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&[tension()], || plain(&source))
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
        let (observed, frames) = counted("tension");
        let (observed, frames) = (observed - before.0.0, frames - before.0.1);
        if whole {
            assert_eq!(frames, recorded, "{label}: a frame every hour");
        } else {
            assert!(
                frames * 2 < recorded,
                "{label}: {frames} frames of {recorded} hours"
            );
        }
        assert_eq!(
            observed, frames,
            "{label}: every frame kept and observed once"
        );
    }
}
