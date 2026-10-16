//! Construction, equipment stockpiles, production and country experience
//! prepared on their own, before the hourly walk, publish and save exactly
//! what hourly preparation does, refuse where it refuses, answer their
//! queries while the hourly walk has accepted nothing, and build no frame for
//! the quiet hours between their changes and daily reads.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use independent::economy::{self, CONSTRUCTION, EXPERIENCE, PRODUCTION, STOCKPILE};
use replay::rows::{
    BuildingConstructionRow, BuildingRow, ConstructionLineOutputRow, CountryAppearanceRow,
    CountryExperienceGainRow, CountryExperienceRow, LexicalTokenRow, ProductionLineOutputRow,
    ProductionLineRow, ProvinceRow, Recorded, StockpileRow, VariantDefinitionRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour: LogisticsEquipment's daily slot. The construction
/// queues read at +3, stockpiles, lines and output at +8, banked experience
/// at +17 and variants at +22 of each day.
const BASE: i32 = 1000 * 24;
/// The first country of `frame`: raw id, canonical slot 0, tag "000".
const COUNTRY: u32 = 100_000;
/// The second country, which gains no experience.
const OTHER: u32 = 100_001;
/// A civilian factory's building token.
const FACTORY: u32 = 5;
/// Infantry equipment's token.
const INFANTRY: u32 = 601;

/// The families, under their status and refusal names, with the prefix of
/// every section each publishes and its field in the hourly branch state.
const LEDGERS: [(&str, &str); 4] = [
    ("construction", "construction"),
    ("stockpile", "stockpile"),
    ("production", "production"),
    ("experience", "experience"),
];

fn ledgers() -> [&'static dyn independent::Own; 4] {
    [&CONSTRUCTION, &STOCKPILE, &PRODUCTION, &EXPERIENCE]
}

/// The day of `hour` as read at daily slot `slot`: it advances at the slot.
fn day(hour: i32, slot: i32) -> i64 {
    i64::from((hour - BASE - slot).div_euclid(24))
}

/// Every ledger's tables at `hour`. The countries' balances (in `frame`) and
/// the building's partial health change every hour without being read. The
/// daily tables change at their reads: the queue's line progresses and its
/// output grows at +3 until the line completes at the second day's read; the
/// stock, line efficiency and output at +8; banked experience at +17; a
/// second variant appears at +22. The building is bombed at +14 (inside a
/// block) and repaired at the second day's queue read. The first country's
/// hourly experience award grows at +10 (inside a block) and is otherwise
/// equal hour after hour.
fn ledger_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let set = |observed: &mut DecodedFrame, table: TableId, rows: TableRows| {
        *observed.tables[table.index()] = TableView::Complete(rows);
    };
    // Construction.
    let built = hour >= BASE + 27;
    let levels = if built {
        (2, 1)
    } else if hour >= BASE + 14 {
        (1, 0)
    } else {
        (1, 1)
    };
    set(
        &mut observed,
        TableId::Provinces,
        TableRows::Provinces(vec![ProvinceRow {
            id: 1001,
            state: 40,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::StateBuildings,
        TableRows::StateBuildings(vec![BuildingRow {
            location: 40,
            building_type: FACTORY,
            country: COUNTRY,
            level: levels.0,
            healthy_level: levels.1,
            partial_health: i64::from(hour),
            active: true,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::ProvinceBuildings,
        TableRows::ProvinceBuildings(Vec::new()),
    );
    let queue_day = day(hour, 3);
    set(
        &mut observed,
        TableId::BuildingConstruction,
        TableRows::BuildingConstruction(if built {
            Vec::new()
        } else {
            vec![BuildingConstructionRow {
                id: 7,
                country: COUNTRY,
                active_factories: 10,
                cost: 1_080_000_000,
                speed: 1_080_000_000,
                produced: 100_000 * queue_day,
                amount: 1,
                created_hour: BASE - 100,
                building_type: FACTORY,
                target_flags: engine::cbuilding_status::STATE_FLAG,
                target_location: 40,
                target_state: 40,
                ..Default::default()
            }]
        }),
    );
    set(
        &mut observed,
        TableId::RailwayConstruction,
        TableRows::RailwayConstruction(Vec::new()),
    );
    set(
        &mut observed,
        TableId::ConstructionLineOutput,
        TableRows::ConstructionLineOutput(vec![ConstructionLineOutputRow {
            line: 7,
            construction_raw: 100_000 * queue_day,
            ticks: i32::try_from(queue_day).unwrap(),
            since_hour: BASE - 100,
            ended: built,
            exact: true,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::LexicalTokens,
        TableRows::LexicalTokens(
            [
                (FACTORY, "industrial_complex"),
                (INFANTRY, "infantry_equipment_1"),
            ]
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .to_vec(),
        ),
    );
    // Stockpiles and production.
    let line_day = day(hour, 8);
    set(
        &mut observed,
        TableId::Stockpiles,
        TableRows::Stockpiles(vec![StockpileRow {
            country: COUNTRY,
            ordinal: 0,
            variant_handle: 10,
            amount_raw: 1_000 + 100 * line_day,
        }]),
    );
    set(
        &mut observed,
        TableId::ProductionLines,
        TableRows::ProductionLines(vec![ProductionLineRow {
            handle: 1,
            country: COUNTRY,
            kind: 0,
            variant: 10,
            active_factories: 5,
            requested_factories: 6,
            max_factories: 150,
            cost_raw: 50_000,
            speed_raw: 310_000,
            efficiency_raw: 10_000 + line_day,
            amount: -1,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::ProductionLineOutput,
        TableRows::ProductionLineOutput(vec![ProductionLineOutputRow {
            line: 1,
            ordinal: 0,
            country: COUNTRY,
            variant: 10,
            units: 3 * line_day,
            since_hour: BASE - 100,
            observed_since_creation: true,
            exact: true,
            spent_raw: Recorded::Value(Some(50_000 * line_day)),
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::RocketProductionOutput,
        TableRows::RocketProductionOutput(Vec::new()),
    );
    set(
        &mut observed,
        TableId::LogisticsEquipment,
        TableRows::LogisticsEquipment(Vec::new()),
    );
    let variant = |id| VariantDefinitionRow {
        id,
        equipment_token: INFANTRY,
        creator_link: COUNTRY,
        ..Default::default()
    };
    set(
        &mut observed,
        TableId::VariantDefinitions,
        TableRows::VariantDefinitions(if hour >= BASE + 22 {
            vec![variant(10), variant(11)]
        } else {
            vec![variant(10)]
        }),
    );
    set(
        &mut observed,
        TableId::VariantModules,
        TableRows::VariantModules(Vec::new()),
    );
    set(
        &mut observed,
        TableId::VariantUpgrades,
        TableRows::VariantUpgrades(Vec::new()),
    );
    set(
        &mut observed,
        TableId::CountryAppearance,
        TableRows::CountryAppearance(vec![CountryAppearanceRow {
            country: COUNTRY,
            ..Default::default()
        }]),
    );
    // Experience.
    set(
        &mut observed,
        TableId::CountryExperience,
        TableRows::CountryExperience(vec![CountryExperienceRow {
            country: COUNTRY,
            army_raw: 1_000 * day(hour, 17),
            navy_raw: 5,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::CountryExperienceGains,
        TableRows::CountryExperienceGains(vec![
            CountryExperienceGainRow {
                country: COUNTRY,
                instance: 1,
                complete: 1,
                army_raw: if hour >= BASE + 10 { 7 } else { 3 },
                ..Default::default()
            },
            CountryExperienceGainRow {
                country: OTHER,
                instance: 2,
                ..Default::default()
            },
        ]),
    );
    // Off the clock, a table of each ledger's daily slot is read at +13: the
    // queues', the lines' and banked experience's slots, though none of the
    // ledgers' own tables. Their reads are dated there with no frame read.
    if hour == BASE + 13 {
        for table in [
            TableId::StateGarrisons,
            TableId::DeploymentLines,
            TableId::CountryFuel,
        ] {
            observed.metrics.table_ns[table.index()] = 1;
        }
    }
    observed
}

/// Branch 1's first `blocks` blocks of five (+18 and +19 are a gap between
/// the third and fourth), then branch 2's one block when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    let hours: [Vec<i32>; 5] = [
        (0..=5).collect(),
        (6..=11).collect(),
        (12..=17).collect(),
        (20..=25).collect(),
        (26..=31).collect(),
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&offset| ledger_frame(1, BASE + offset))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(&hours[0]))]).unwrap();
    for hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (100..=105)
                    .map(|offset| ledger_frame(2, BASE + offset))
                    .collect(),
            )])
            .unwrap();
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

/// Sections only one mode writes: the checkpoint, the ledgers' coverage
/// pages and continuations, and the hourly branch state, which holds the
/// ledgers' continuations in hourly preparation (compared apart).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.contains("-continuation-")
}

/// A continuation or branch state page as JSON.
fn decoded(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical; each ledger
/// published; the hourly branch state is the same apart from the ledgers'
/// fields; with `continuation`, each ledger's continuation is the field
/// hourly preparation keeps in its branch state. Without it (a ledger
/// refused), the branch's coverage pages, whose refusals are the hourly
/// walk's own, are compared apart too.
fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, continuation: bool) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && (continuation || !name.starts_with("preparation-coverage-"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    for (prefix, _) in LEDGERS {
        assert!(
            actual
                .keys()
                .any(|(name, _)| name.starts_with(&format!("{prefix}-"))),
            "{label}: {prefix} published nothing"
        );
    }
    let mut states = 0;
    for ((name, range), bytes) in hourly {
        let Some(branch) = name.strip_prefix("preparation-branch-") else {
            continue;
        };
        let mut saved = decoded(bytes);
        let mut own = decoded(&actual[&(name.clone(), *range)]);
        for (family, field) in LEDGERS {
            let state = saved.as_object_mut().unwrap().remove(field).unwrap();
            // The walk saves no state of a family it does not run.
            assert_eq!(
                own.as_object_mut().unwrap().remove(field),
                None,
                "{label}: branch {branch}'s saved state holds {field}"
            );
            if continuation {
                let section = (format!("{family}-continuation-{branch}"), 0);
                let kept = actual
                    .get(&section)
                    .unwrap_or_else(|| panic!("{label}: {family} saved no continuation"));
                assert_eq!(
                    decoded(kept),
                    state,
                    "{label}: branch {branch}'s {family} continuation"
                );
                states += 1;
            }
        }
        assert_eq!(own, saved, "{label}: branch {branch}'s hourly state");
    }
    assert!(!continuation || states > 0, "{label}: no continuation");
}

/// The hourly reference of `prepare`.
fn hourly(fixture: &mut Fixture, revision: &str, prepare: impl Fn(&Source)) -> Payloads {
    independent::with_test_hourly(|| {
        let source = fixture.source(revision);
        prepare(&source);
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    })
}

fn plain(source: &Source) {
    run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
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

#[test]
fn economy_ledgers_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "ledgers-parity", plain);
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source("ledgers-parity");
        let before: Vec<usize> = LEDGERS
            .iter()
            .map(|(family, _)| prepass::noted(&format!("{family}.sequential")))
            .collect();
        prepass::with_test_prepass(workers, None, || {
            lanes::with_test_lanes(parallelism, None, || {
                independent::with_test_families(&ledgers(), || plain(&source))
            })
        });
        let label = format!("{workers} workers, {parallelism} processors");
        assert_same(&payloads(&source, None), &hourly, &label, true);
        for ((family, _), before) in LEDGERS.iter().zip(before) {
            assert_eq!(
                prepass::noted(&format!("{family}.sequential")),
                before,
                "{label}: every {family} block was extracted"
            );
        }
        clear(source);
    }
}

#[test]
fn economy_ledgers_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    let first = scenario(3, false);
    let grown = scenario(5, true);
    // The hourly walk prepares one block per run.
    let resumed = |source: &Source| {
        let flag = AtomicBool::new(false);
        while unprepared_blocks(source, None).unwrap() != Some(0) {
            run_measured(source, None, &flag, false, Duration::ZERO, |_| {}).unwrap();
        }
    };
    let modes: [(&str, &dyn Fn(&Source), bool); 4] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
        ("resumed", &resumed, false),
    ];
    let mut fixture = Fixture::new(&first);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source("ledgers-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("ledgers-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = independent::with_test_families(&ledgers(), || {
            prepass::with_test_prepass(2, None, || both(&mut fixture))
        });
        assert_same(&own, &hourly, mode, true);
    }
}

#[test]
fn economy_ledgers_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    let replay = scenario(5, true);
    let products = [
        ("construction", "construction-log"),
        ("stockpile", "equipment-stockpile"),
        ("production", "production-select"),
        ("experience", "country-experience"),
    ];
    for (family, product) in products {
        // At a block's first hour, and inside a block's quiet hours.
        for hour in [BASE + 6, BASE + 21] {
            let refusals = [(family, hour)];
            let mut fixture = Fixture::new(&replay);
            let (hourly, hourly_refused) = independent::with_test_hourly(|| {
                lanes::with_test_refusals(&refusals, || {
                    let source = fixture.source("ledgers-refusal");
                    measured(&source);
                    let (store, key) = super::super::super::files(&source, None).unwrap();
                    let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
                    let result = (payloads(&source, None), refused);
                    clear(source);
                    result
                })
            });
            let source = fixture.source("ledgers-refusal");
            let sequential = prepass::noted(&format!("{family}.sequential"));
            independent::with_test_families(&ledgers(), || {
                lanes::with_test_refusals(&refusals, || {
                    prepass::with_test_prepass(2, None, || measured(&source))
                });
                let label = format!("{family} refused at {hour}");
                assert_eq!(
                    prepass::noted(&format!("{family}.sequential")) - sequential,
                    1,
                    "{label}: the refusing block is observed hour by hour"
                );
                let own = payloads(&source, None);
                assert_same(&own, &hourly, &label, false);
                // The refusal is the ledger's own, at the hourly hour and text.
                let expected = &hourly_refused[&1].0[family];
                let page: CoveragePage =
                    serde_json::from_slice(&own[&(independent::coverage_section(family, 1), 0)])
                        .unwrap();
                assert_eq!(page.refused.0[family].hour, hour, "{label}");
                assert_eq!(page.refused.0[family].hour, expected.hour, "{label}");
                assert_eq!(page.refused.0[family].error, expected.error, "{label}");
                assert!(page.coverage.complete, "{label}");
                let flag = AtomicBool::new(false);
                let error = refusal(&source, None, 1, product, &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(error.to_string(), expected.error, "{label}");
                refusal(&source, None, 2, product, &flag).unwrap();
            });
            clear(source);
        }
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

/// The hourly walk stalls: cancelled at its first commit, having accepted no
/// block, while the ledgers prepared on their own committed every block.
fn stalled(source: &Source, replay: &Replay) {
    let flag = AtomicBool::new(false);
    let mut commits = 0;
    let error = independent::with_test_families(&ledgers(), || {
        run_checked(
            source,
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
    // Under the registry it prepared with: another set of families prepared
    // on their own restarts preparation, which leaves nothing unprepared.
    assert_eq!(
        independent::with_test_families(&ledgers(), || unprepared_blocks(source, None)).unwrap(),
        Some(replay.index.len()),
        "the hourly walk accepted no block"
    );
}

#[test]
fn economy_ledgers_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("ledgers-stalled");
    stalled(&source, &replay);
    let flag = AtomicBool::new(false);
    independent::with_test_families(&ledgers(), || {
        for branch in [1, 2] {
            for product in [
                "construction-log",
                "construction-log-manifest",
                "construction-spending",
                "equipment-stockpile",
                "production-select",
                "production-spending",
                "country-experience",
            ] {
                let covered = product_coverage(&source, None, branch, product, &flag)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{product} covers branch {branch}"));
                assert!(covered.complete, "{product} on branch {branch}");
            }
            // The branch itself is not covered.
            assert!(
                coverage(&source, None, branch, &flag)
                    .unwrap()
                    .is_none_or(|branch| !branch.complete)
            );
            let id = branch.to_string();
            let manifest = query(
                &source,
                "construction-log-manifest",
                serde_json::json!({ "branchId": id }),
            )
            .unwrap();
            assert!(
                !manifest.is_null(),
                "branch {branch}: the construction manifest answers"
            );
            assert!(
                query(&source, "subjects", serde_json::json!({ "branchId": id }))
                    .unwrap()
                    .is_null(),
                "branch {branch}: a family of the hourly walk still waits"
            );
        }
    });
    clear(source);
}

/// Experience's series route by their family kind, so they answer from the
/// experience family's own coverage too.
#[test]
fn experience_series_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("ledgers-experience-series");
    stalled(&source, &replay);
    independent::with_test_families(&ledgers(), || {
        let tag = serde_json::to_string(&("000", 0, COUNTRY)).unwrap();
        let state = query(
            &source,
            "series-state",
            serde_json::json!({
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": "army-banked" },
                "reads": [{ "tag": tag, "hour": BASE + 20 }],
            }),
        )
        .unwrap();
        assert!(!state.is_null(), "experience answers its series");
    });
    clear(source);
}

/// Read by their changes, the ledgers build a frame only at the hours a
/// field they read changes, a daily read is due or a block starts: every
/// other hour is advanced over without a frame (the countries' balances and
/// the building's health change every hour there), each quiet span inside
/// a block's daily reads included. Their pages are hourly preparation's
/// either way, and the block whose reads are dated off the clock is the one
/// observed hour by hour after its first such hour.
#[test]
fn economy_ledgers_read_by_their_changes_build_no_frame_at_quiet_hours() {
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "ledgers-changes", plain);
    for (family, own) in LEDGERS.iter().map(|(family, _)| *family).zip(ledgers()) {
        let mut seen = Vec::new();
        for whole in [true, false] {
            let label = format!(
                "{family} read {}",
                if whole { "whole" } else { "by changes" }
            );
            let source = fixture.source("ledgers-changes");
            let before = economy::counts(family);
            let fallback = prepass::noted(&format!("{family}.fallback"));
            // One family alone folds on this thread, where it counts.
            independent::with_test_families(&[own], || {
                let read = || prepass::with_test_prepass(2, None, || plain(&source));
                if whole {
                    prepass::with_test_alone(read);
                } else {
                    read();
                }
            });
            let after = economy::counts(family);
            let (frames, observed, quiet) = (
                after.frames - before.frames,
                after.observed - before.observed,
                after.quiet - before.quiet,
            );
            // The family's pages equal hourly preparation's.
            let own_pages: Payloads = payloads(&source, None)
                .into_iter()
                .filter(|((name, _), _)| {
                    name.starts_with(&format!("{family}-")) && !mode_specific(name)
                })
                .collect();
            let hourly_pages: Payloads = hourly
                .iter()
                .filter(|((name, _), _)| {
                    name.starts_with(&format!("{family}-")) && !mode_specific(name)
                })
                .map(|(key, bytes)| (key.clone(), bytes.clone()))
                .collect();
            assert!(!hourly_pages.is_empty(), "{label}");
            assert_payloads_equal(&own_pages, &hourly_pages, &label);
            assert_eq!(observed + quiet, total, "{label}: every hour folded once");
            if whole {
                assert_eq!(frames, total, "{label}: a frame every hour");
                assert_eq!(quiet, 0, "{label}");
                assert_eq!(
                    prepass::noted(&format!("{family}.fallback")),
                    fallback,
                    "{label}"
                );
            } else {
                assert!(frames < total, "{label}: {frames} frames of {total} hours");
                assert!(quiet > 0, "{label}: quiet hours advanced without a frame");
                assert_eq!(
                    prepass::noted(&format!("{family}.fallback")) - fallback,
                    1,
                    "{label}: the block with a read dated off the clock"
                );
            }
            seen.push((frames, quiet));
            clear(source);
        }
        assert!(seen[1].0 < seen[0].0, "{family}: fewer frames by changes");
    }
}
