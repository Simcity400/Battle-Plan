//! Construction, equipment stockpiles, production, country experience and
//! fuel prepared on their own beside the hourly walk, publish and save
//! exactly what hourly preparation does, refuse where it refuses, answer
//! their queries while the hourly walk has accepted nothing, and build no
//! frame for the quiet hours between their changes and daily reads.
use super::family_lanes::{assert_payloads_equal, assert_versions, payloads};
use super::*;
use crate::prepared::prepass;
use independent::economy::{self, CONSTRUCTION, EXPERIENCE, FUEL, PRODUCTION, STOCKPILE};
use replay_data::{
    rows::BuildingConstructionRow, rows::BuildingRow, rows::ConstructionLineOutputRow,
    rows::CountryAppearanceRow, rows::CountryExperienceGainRow, rows::CountryExperienceRow,
    rows::CountryFuelRow, rows::LexicalTokenRow, rows::OutputPurpose,
    rows::ProductionLineOutputRow, rows::ProductionLineRow, rows::ProvinceRow, rows::Recorded,
    rows::StateRow, rows::StockpileRow, rows::VariantDefinitionRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour: LogisticsEquipment's daily slot. The construction
/// queues read at +3, stockpiles, lines and output at +8, banked experience
/// and fuel at +17 and variants at +22 of each day.
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
const LEDGERS: [(&str, &str); 5] = [
    ("construction", "construction"),
    ("stockpile", "stockpile"),
    ("production", "production"),
    ("experience", "experience"),
    ("fuel", "fuel"),
];

fn ledgers() -> [&'static dyn independent::Own; 5] {
    [&CONSTRUCTION, &STOCKPILE, &PRODUCTION, &EXPERIENCE, &FUEL]
}

/// The day of `hour` as read at daily slot `slot`: it advances at the slot.
fn day(hour: i32, slot: i32) -> i64 {
    i64::from((hour - BASE - slot).div_euclid(24))
}

/// Every ledger's tables at `hour`. The countries' balances (in `frame`) and
/// the building's partial health change every hour without being read. The
/// daily tables change at their reads: the queue's line progresses and its
/// output grows at +3 until the line completes at the second day's read; the
/// stock, line efficiency and output at +8; banked experience and fuel at
/// +17; a second variant appears at +22. The building is bombed at +14
/// (inside a block) and repaired at the second day's queue read. The first
/// country's hourly experience award grows at +10 (inside a block) and is
/// otherwise equal hour after hour. The states' manpower changes every hour
/// without being read, and the second country's state passes to the first
/// at +23 (inside a block).
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
                target_flags: engine_native::cbuilding_status::STATE_FLAG,
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
            units: 3 * (line_day + 1),
            since_hour: BASE - 100,
            observed_since_creation: true,
            exact: true,
            spent_raw: Recorded::Value(Some(50_000 * line_day)),
            purpose: Recorded::Value(OutputPurpose::New.raw()),
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableId::RocketProductionOutput,
        TableRows::RocketProductionOutput(vec![replay_data::rows::RocketProductionOutputRow {
            since_hour: BASE - 100,
            exact: true,
            units_exact: true,
            ..Default::default()
        }]),
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
    // The quiet baseline has no writer-observed changes between variant reads.
    set(
        &mut observed,
        TableId::VariantChanges,
        TableRows::VariantChanges(Vec::new()),
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
    fuel_tables(&mut observed, hour, BASE + 23);
    // Off the clock, a table of each ledger's daily slot is read at +13: the
    // queues', the lines' and banked experience's and fuel's slots, though
    // none of the ledgers' own tables. Their reads are dated there with no
    // frame read.
    if hour == BASE + 13 {
        for table in [
            TableId::StateGarrisons,
            TableId::DeploymentLines,
            TableId::HubMotorization,
        ] {
            observed.metrics.table_ns[table.index()] = 1;
        }
    }
    observed
}

/// Fuel's tables at `hour`: both countries' fuel, which grows at each daily
/// read (+17), and no demand parts; each country owns a state, whose manpower
/// moves every hour without being read, and the second country's state passes
/// to the first at `ceded`.
fn fuel_tables(observed: &mut DecodedFrame, hour: i32, ceded: i32) {
    let fuel_day = day(hour, 17);
    *observed.tables[TableId::CountryFuel.index()] = TableView::Complete(TableRows::CountryFuel(
        [COUNTRY, OTHER]
            .into_iter()
            .enumerate()
            .map(|(index, country)| CountryFuelRow {
                country,
                current_raw: 10_000 + 500 * fuel_day + index as i64,
                maximum_raw: 50_000,
                ring_gain_raw: 600 + index as i64,
                ring_consumed_raw: 100,
                ..Default::default()
            })
            .collect(),
    ));
    *observed.tables[TableId::FuelDemandParts.index()] =
        TableView::Complete(TableRows::FuelDemandParts(Vec::new()));
    *observed.tables[TableId::States.index()] = TableView::Complete(TableRows::States(
        [COUNTRY, OTHER]
            .into_iter()
            .enumerate()
            .map(|(index, owner)| StateRow {
                id: index as u32 + 1,
                owner: if hour >= ceded { COUNTRY } else { owner },
                controller: owner,
                manpower_available: hour,
                ..Default::default()
            })
            .collect(),
    ));
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

/// Sections only one mode writes: the checkpoint, the versions page (checked
/// against each run's plan), the ledgers' coverage pages and continuations, and
/// the hourly branch state, which holds the ledgers' continuations in hourly
/// preparation (compared apart).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name == "preparation-versions-v1"
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
    assert_versions(actual, label);
    assert_versions(hourly, label);
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
fn stockpile_variant_changes_and_missing_intervals_publish_and_save_as_hourly_preparation() {
    use replay_data::rows::{VariantChangeRow, VariantScalarChange};
    for rename in [true, false] {
        let label = if rename { "rename" } else { "unavailable" };
        let renamed = b"Renamed equipment";
        let frames = (0..32)
            .map(|offset| {
                let mut frame = ledger_frame(1, BASE + offset);
                if rename
                    && offset >= 25
                    && let TableView::Complete(TableRows::VariantDefinitions(rows)) =
                        &mut *frame.tables[TableId::VariantDefinitions.index()]
                {
                    rows[0].display_name = renamed.to_vec();
                }
                if offset == 25 {
                    *frame.tables[TableId::VariantChanges.index()] = if rename {
                        TableView::Complete(TableRows::VariantChanges(vec![VariantChangeRow {
                            sequence: 1,
                            hour: frame.hour,
                            variant: 10,
                            scalar: Some(VariantScalarChange {
                                display_name: Some(renamed.to_vec()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }]))
                    } else {
                        TableView::Unavailable {
                            reason: "synthetic variant change interval unavailable".into(),
                        }
                    };
                }
                frame
            })
            .collect();
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
        let mut fixture = Fixture::new(&replay);
        let expected = hourly(&mut fixture, "stockpile-variant-changes", plain);
        let source = fixture.source("stockpile-variant-changes");
        independent::with_test_families(&ledgers(), || {
            prepass::with_test_prepass(2, None, || plain(&source))
        });
        let actual = payloads(&source, None);
        clear(source);
        assert_same(&actual, &expected, label, true);
        if rename {
            let continuation = decoded(&actual[&("stockpile-continuation-1".into(), 0)]);
            assert!(
                continuation["names"]["versions"]["10"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|version| version["first_hour"] == BASE + 25
                        && version["raw"]["display_name"] == "Renamed equipment"),
                "a writer-observed rename is dated between the daily variant reads"
            );
        }
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
    let modes: [PrepareReloadMode; 4] = [
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
        ("fuel", "country-fuel"),
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
                assert_eq!(page.refused[family].hour, hour, "{label}");
                assert_eq!(page.refused[family].hour, expected.hour, "{label}");
                assert_eq!(page.refused[family].error, expected.error, "{label}");
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
        independent::with_test_one_after_another(|| {
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
                "country-fuel",
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
            let pending =
                query(&source, "subjects", serde_json::json!({ "branchId": id })).unwrap();
            assert_cold_prefix(&source, branch, &pending);
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

/// Before production's coverage completes, a selection after the prepared
/// prefix answers at the prefix's last hour, as the complete pages answer
/// that hour, and names the later recorded hours pending; the old selection
/// waited for complete coverage.
#[test]
fn production_selections_answer_the_prepared_prefix_with_the_hours_still_pending() {
    use crate::prepared_production::ProductionAccumulator;
    independent::with_test_hourly(|| {
        let replay = scenario(5, false);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("production-prefix");
        let flag = AtomicBool::new(false);
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let format = source.replay_ref().format;
        let mut state = ProductionAccumulator::new(format.version(), 1, BASE);
        // Commits through the second block, then through the branch's end.
        let mut commit = |writer: &mut store::Writer, offsets: &[i32], last: i32, complete| {
            for &offset in offsets {
                let mut frame = ledger_frame(1, BASE + offset);
                // This comparison needs type identities supplied in the prefix,
                // at its first output read, before the native variant slot (+22).
                // Later classifications deliberately revise provisional keys.
                if offset == 8 {
                    frame.metrics.table_ns[TableId::VariantDefinitions.index()] = 1;
                }
                state.ingest(format, &frame, &flag).unwrap();
            }
            state.flush(writer, &flag).unwrap();
            json(
                writer,
                &coverage_section(1),
                &Coverage {
                    schema: SCHEMA,
                    first_hour: BASE,
                    last_hour: BASE + last,
                    complete,
                },
            )
            .unwrap();
        };
        let selection = |hour: i32| {
            serde_json::json!({"hour": hour, "selection": {"branchId": "1", "tags": [], "top": 10,
                "measure": "total", "count": "units", "grouping": "type", "included": [], "refits": false},
                "offset": "0", "limit": 10, "ranked": true})
        };
        let mut writer = store.replace(key.clone()).unwrap();
        commit(&mut writer, &(0..=11).collect::<Vec<_>>(), 11, false);
        writer.commit().unwrap();
        let cold = query(&source, "production-select", selection(BASE + 31)).unwrap();
        let mut writer = store.extend(&key).unwrap();
        let rest: Vec<i32> = (12..=17).chain(20..=31).collect();
        commit(&mut writer, &rest, 31, true);
        writer.commit().unwrap();
        assert!(!cold.is_null(), "a prefix selection before completion");
        assert_eq!(cold["coverage"]["lastHour"], BASE + 11);
        assert_eq!(cold["coverage"]["complete"], false);
        assert_eq!(
            cold["approximate"],
            serde_json::json!([{"firstHour": BASE, "lastHour": BASE + 11}])
        );
        assert_eq!(
            cold["pending"],
            serde_json::json!([{"firstHour": BASE + 12, "lastHour": BASE + 17},
                {"firstHour": BASE + 20, "lastHour": BASE + 31}]),
            "the recorded gap between blocks is not pending"
        );
        assert!(cold["firstHour"].as_i64().unwrap() <= i64::from(BASE + 11));
        assert!(cold["lastHour"].as_i64().unwrap() >= i64::from(BASE + 11));
        assert!(
            !cold["countries"].as_array().unwrap().is_empty(),
            "the prefix delivered equipment: {cold}"
        );
        assert_eq!(
            cold["countries"][0]["values"][0]["key"], ":infantry_equipment_1",
            "the prefix already includes the supplying design identity"
        );
        // The prefix's countries are those the complete pages give at its last hour.
        let complete = query(&source, "production-select", selection(BASE + 11)).unwrap();
        assert_eq!(complete["coverage"]["complete"], true);
        assert_eq!(complete["pending"], serde_json::json!([]));
        assert_eq!(cold["countries"], complete["countries"]);
        assert_eq!(cold["totalCountries"], complete["totalCountries"]);
        assert_eq!(complete["approximate"], serde_json::json!([]));
        clear(source);
    });
}

/// Commits experience prepared from `offsets` (after BASE) of branch 1's
/// ledger frames, with the hourly walk's coverage through `last`.
fn commit_experience(
    writer: &mut store::Writer,
    accumulator: &mut crate::prepared_stability::ExperienceAccumulator,
    format: replay_data::format::FormatVersion,
    offsets: impl IntoIterator<Item = i32>,
    last: i32,
    complete: bool,
) {
    for offset in offsets {
        accumulator
            .observe(&ledger_frame(1, BASE + offset), format)
            .unwrap();
    }
    accumulator
        .flush(writer, 1, &AtomicBool::new(false))
        .unwrap();
    json(
        writer,
        &coverage_section(1),
        &Coverage {
            schema: SCHEMA,
            first_hour: BASE,
            last_hour: BASE + last,
            complete,
        },
    )
    .unwrap();
}

/// The first country's banked army experience at `hours`, as series states.
fn banked_states(source: &Source, hours: &[i32]) -> serde_json::Value {
    let tag = serde_json::to_string(&("000", 0, COUNTRY)).unwrap();
    query(
        source,
        "series-state",
        serde_json::json!({
            "branchId": "1",
            "family": { "kind": "country-experience", "measure": "army-banked" },
            "reads": hours
                .iter()
                .map(|hour| serde_json::json!({ "tag": tag, "hour": BASE + hour }))
                .collect::<Vec<_>>(),
        }),
    )
    .unwrap()
}

/// Branch 1 as fourteen blocks of six hours from BASE, without gaps: daily
/// reads of banked experience fall at +13 and +17, +41 and +65.
fn daily_scenario() -> Replay {
    let frames = |block: i32| -> Vec<DecodedFrame> {
        (block * 6..block * 6 + 6)
            .map(|offset| ledger_frame(1, BASE + offset))
            .collect()
    };
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0))]).unwrap();
    for block in 1..14 {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(block))])
            .unwrap();
    }
    replay
}

#[test]
fn exact_banked_experience_keeps_its_cadence_read_across_one_hour_blocks() {
    independent::with_test_hourly(|| {
        let observed = |offset| {
            let mut observed = ledger_frame(1, BASE + offset);
            if let TableView::Complete(TableRows::CountryExperience(rows)) =
                &mut *observed.tables[TableId::CountryExperience.index()]
            {
                rows[0].army_raw = if offset < 41 { 100 } else { 200 };
            }
            observed
        };
        let mut replay =
            Replay::from_records(CAMPAIGN.into(), &[records(1, vec![observed(0)])]).unwrap();
        for hour in 1..24 {
            replay = replay
                .append(&[continuation_records(&replay, 1, vec![observed(hour)])])
                .unwrap();
        }
        replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                (24..48).map(observed).collect(),
            )])
            .unwrap();
        assert_eq!(replay.index.len(), 25);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("experience-one-hour-predecessors");
        let hours = [24, 40, 41, 42, 47];
        let cold = banked_states(&source, &hours);
        for (index, hour) in hours.iter().enumerate() {
            let state = &cold["states"][index];
            assert_eq!(state["approximate"], false, "hour {hour}");
            assert_eq!(
                state["run"]["valueRaw"],
                if *hour < 41 { "100" } else { "200" },
                "hour {hour}: the latest supplying read, not the block's keyframe"
            );
            assert_eq!(
                state["readHour"],
                BASE + if *hour < 41 { 17 } else { 41 },
                "hour {hour}: {state}"
            );
        }
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let mut writer = store.replace(key).unwrap();
        let mut accumulator = crate::prepared_stability::ExperienceAccumulator::default();
        for hour in 0..48 {
            accumulator.observe(&observed(hour), replay.format).unwrap();
        }
        accumulator
            .flush(&mut writer, 1, &AtomicBool::new(false))
            .unwrap();
        json(
            &mut writer,
            &coverage_section(1),
            &Coverage {
                schema: SCHEMA,
                first_hour: BASE,
                last_hour: BASE + 47,
                complete: true,
            },
        )
        .unwrap();
        writer.commit().unwrap();
        let complete = banked_states(&source, &hours);
        assert_eq!(complete["coverage"]["complete"], true);
        for index in 0..hours.len() {
            assert_eq!(
                cold["states"][index]["run"]["valueRaw"],
                complete["states"][index]["run"]["valueRaw"]
            );
            for field in ["readHour", "approximate"] {
                assert_eq!(
                    cold["states"][index][field],
                    complete["states"][index][field]
                );
            }
        }
        clear(source);
    });
}

/// Before experience's coverage completes, banked experience after the
/// prepared prefix is sampled from the recorded blocks as preparation reads
/// it: a short tail exactly, the read before it carried into its first
/// hours, each state dated by its read. Gains still wait. The old reply
/// waited for coverage.
#[test]
fn banked_experience_after_the_prepared_prefix_is_read_as_preparation_reads_it() {
    independent::with_test_hourly(|| {
        let replay = daily_scenario();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("experience-prefix");
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let format = source.replay_ref().format;
        let mut accumulator = crate::prepared_stability::ExperienceAccumulator::default();
        let hours = [18, 30, 40, 41, 83];
        // Through the third block, whose daily read the tail carries.
        let mut writer = store.replace(key.clone()).unwrap();
        commit_experience(&mut writer, &mut accumulator, format, 0..=17, 17, false);
        writer.commit().unwrap();
        let cold = banked_states(&source, &hours);
        let tag = serde_json::to_string(&("000", 0, COUNTRY)).unwrap();
        let gain = query(
            &source,
            "series-state",
            serde_json::json!({
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": "army-gain" },
                "reads": [{ "tag": tag, "hour": BASE + 20 }],
            }),
        )
        .unwrap();
        assert!(gain.is_null(), "daily gains need every hour of a day");
        let mut writer = store.extend(&key).unwrap();
        commit_experience(&mut writer, &mut accumulator, format, 18..=83, 83, true);
        writer.commit().unwrap();
        let complete = banked_states(&source, &hours);
        assert!(
            !cold.is_null(),
            "banked experience answers before completion"
        );
        assert_eq!(complete["coverage"]["complete"], true);
        assert!(
            cold["states"][0]["run"]["valueRaw"].is_string(),
            "the read before the tail carries into its first hour"
        );
        for (index, hour) in hours.iter().enumerate() {
            let (cold, complete) = (&cold["states"][index], &complete["states"][index]);
            assert_eq!(cold["approximate"], false, "a short tail is read exactly");
            assert_eq!(
                cold["run"]["valueRaw"], complete["run"]["valueRaw"],
                "{hour}"
            );
            assert_eq!(cold["readHour"], complete["readHour"], "{hour}");
        }
        clear(source);
    });
}

/// Summaries publish the exact accepted prefix without reading the recording.
/// Reached values remain independent; preparation supplies the remaining chart.
#[test]
fn banked_experience_summaries_draw_the_prefix_without_decoding_the_tail() {
    independent::with_test_hourly(|| {
        let replay = daily_scenario();
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("experience-long-tail");
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let format = source.replay_ref().format;
        let mut accumulator = crate::prepared_stability::ExperienceAccumulator::default();
        let tag = serde_json::to_string(&("000", 0, COUNTRY)).unwrap();
        let params = |measure: &str| {
            serde_json::json!({
                "branchId": "1",
                "family": { "kind": "country-experience", "measure": measure },
                "tags": [tag],
                "from": BASE,
                "to": BASE + 83,
                "width": 84,
                "withMarks": false,
            })
        };
        let summary = |measure: &str| {
            let (reply, decoded) =
                crate::prepared::query::progressive::without_block_decodes(|| {
                    query(&source, "series-summary", params(measure))
                });
            assert!(!decoded, "a summary must not wait for tail decodes");
            reply.unwrap()
        };
        let span = |first: i32, last: i32| serde_json::json!({ "firstHour": BASE + first, "lastHour": BASE + last });
        for measure in crate::products::country_experience::SAMPLED_MEASURES {
            let cold = summary(measure);
            assert_eq!(cold["coverage"]["lastHour"], BASE - 1);
            assert_eq!(cold["pending"], serde_json::json!([span(0, 83)]));
            let chart = &cold["series"][0]["summary"];
            assert_eq!(chart["runs"], serde_json::json!([]));
            assert_eq!(chart["buckets"]["skip"], serde_json::json!([]));
        }
        let mut writer = store.replace(key.clone()).unwrap();
        commit_experience(&mut writer, &mut accumulator, format, 0..=23, 23, false);
        writer.commit().unwrap();
        for measure in crate::products::country_experience::SAMPLED_MEASURES {
            let prefix = summary(measure);
            assert_eq!(prefix["coverage"]["lastHour"], BASE + 23);
            assert_eq!(prefix["pending"], serde_json::json!([span(24, 83)]));
            let expected = crate::prepared_measure::query::query(
                "series-summary",
                params(measure),
                &store,
                &key,
                None,
                &AtomicBool::new(false),
            )
            .unwrap();
            let chart = &prefix["series"][0]["summary"];
            let prepared = &expected["series"][0]["summary"];
            assert_eq!(chart["runs"], prepared["runs"], "exact prefix values");
            let known = |chart: &serde_json::Value| {
                serde_json::from_value::<crate::prepared_chart::ChartSummary>(chart.clone())
                    .unwrap()
                    .buckets
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            };
            assert_eq!(known(chart), known(prepared));
            assert_eq!(chart["approximate"], serde_json::json!([]));
        }
        assert!(
            summary("army-gain").is_null(),
            "daily gains need every hour of a day"
        );
        let mut writer = store.extend(&key).unwrap();
        commit_experience(&mut writer, &mut accumulator, format, 24..=83, 83, true);
        writer.commit().unwrap();
        for measure in crate::products::country_experience::SAMPLED_MEASURES {
            let complete = summary(measure);
            let mut expected = crate::prepared_measure::query::query(
                "series-summary",
                params(measure),
                &store,
                &key,
                None,
                &AtomicBool::new(false),
            )
            .unwrap();
            expected["coverage"] = complete["coverage"].clone();
            assert_eq!(complete, expected, "the complete reply is unchanged");
            assert_eq!(complete["coverage"]["complete"], true);
        }
        clear(source);
    });
}

/// Before the stockpile's coverage completes, a table at one hour answers
/// from the recording without a prepared page: each holder's last stock, line
/// and logistics reads carry to the hour as preparation carries them, looking
/// back over earlier blocks within a day. Its values at the hour are those
/// the complete reply gives. The old reply waited for complete coverage.
#[test]
fn a_stockpile_table_at_an_hour_answers_from_the_recording_before_coverage_completes() {
    independent::with_test_hourly(|| {
        let replay = daily_scenario();
        let mut fixture = Fixture::new(&replay);
        let table = |source: &Source, hour: i32| {
            query(
                source,
                "equipment-stockpile",
                serde_json::json!({ "branchId": "1", "tag": "000", "from": BASE + hour,
                    "to": BASE + hour, "width": 1, "included": [] }),
            )
            .unwrap()
        };
        // Before the first reads, inside a block after a read, after reads
        // in earlier blocks of the day before, and at the branch's end.
        let hours = [6, 10, 40, 83];
        let source = fixture.source("stockpile-at-hour");
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let mut writer = store.replace(key.clone()).unwrap();
        json(
            &mut writer,
            &coverage_section(1),
            &Coverage {
                schema: SCHEMA,
                first_hour: BASE,
                last_hour: BASE + 5,
                complete: false,
            },
        )
        .unwrap();
        writer.commit().unwrap();
        let cold: Vec<_> = hours.iter().map(|&hour| table(&source, hour)).collect();
        clear(source);
        let source = fixture.source("stockpile-at-hour");
        plain(&source);
        let at = |runs: &serde_json::Value, hour: i32| {
            runs.as_array()
                .unwrap()
                .iter()
                .find(|run| {
                    run["firstHour"].as_i64().unwrap() <= i64::from(BASE + hour)
                        && i64::from(BASE + hour) <= run["lastHour"].as_i64().unwrap()
                })
                .map(|run| run["amountRaw"].clone())
        };
        for (&hour, cold) in hours.iter().zip(&cold) {
            assert!(!cold.is_null(), "a table at {hour} before completion");
            assert_eq!(cold["coverage"]["complete"], false);
            assert_eq!(cold["pending"], serde_json::json!([]));
            assert_eq!(
                cold["approximate"],
                serde_json::json!([{"firstHour": BASE + hour, "lastHour": BASE + hour}])
            );
            let complete = table(&source, hour);
            assert!(complete["prepared"].is_object());
            for (list, key, fields) in [
                (
                    "variants",
                    "id",
                    &["stock", "daily", "dailyIc", "unitCostLow", "unitCostHigh"][..],
                ),
                ("logistics", "key", &["balance", "stock", "available"][..]),
            ] {
                for row in complete[list].as_array().unwrap() {
                    let sampled = cold[list]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|other| other[key] == row[key]);
                    for field in fields {
                        let expected = at(&row[*field], hour);
                        let actual = sampled.and_then(|sampled| at(&sampled[*field], hour));
                        assert_eq!(actual, expected, "{list} {field} at {hour}");
                    }
                }
            }
        }
        assert!(
            at(&cold[2]["variants"][0]["stock"], 40).is_some(),
            "the stock read of the day before carries to the hour"
        );
        clear(source);
    });
}

/// Fuel's frame at `hour` for its days: the fuel tables alone, with an
/// off-slot read of the fuel table at +30 (a block's first hour, a day after
/// +17 and inside a day of +41) and the second country's state ceded at +47
/// (inside a block).
fn fuel_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    fuel_tables(&mut observed, hour, BASE + 47);
    if hour == BASE + 30 {
        observed.metrics.table_ns[TableId::CountryFuel.index()] = 1;
    }
    observed
}

/// Branch 1's blocks of thirty hours (+90 and +91 a gap before the fourth),
/// then branch 2's one block. A day's read is settled 23 hours after it and
/// the read before it retires the hour before that, so with daily reads at
/// +17, +41, +65, +89 and +113, the reads at +17, +30 and +41 are settled or
/// retired at +52, +53, +63 and +87, inside the second and third blocks'
/// quiet hours, and +40, +64 and +88 end those blocks' quiet spans.
fn fuel_scenario() -> Replay {
    let hours: [Vec<i32>; 4] = [
        (0..=29).collect(),
        (30..=59).collect(),
        (60..=89).collect(),
        (92..=121).collect(),
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours
            .iter()
            .map(|&offset| fuel_frame(1, BASE + offset))
            .collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(&hours[0]))]).unwrap();
    for hours in &hours[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    replay
        .append(&[records(
            2,
            (200..=229)
                .map(|offset| fuel_frame(2, BASE + offset))
                .collect(),
        )])
        .unwrap()
}

/// Read by its changes, fuel builds a frame only at its reads, a state's
/// cession and each block's first hour, and steps every hour between without
/// one, settling the days that fall due there (a read decided, the read
/// before it retired, an off-slot read left out by the slot read after it)
/// at those hours: its pages and continuation are hourly preparation's, run
/// whole, committed at every block, and reloaded from each commit.
#[test]
fn fuel_read_by_its_changes_settles_its_days_inside_quiet_hours_as_hourly_preparation() {
    let replay = fuel_scenario();
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let modes: [PrepareReloadMode; 3] = [
        ("run", &plain, false),
        ("measured", &measured, false),
        ("measured, reloaded", &measured, true),
    ];
    let fuel = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| name.starts_with("fuel-") && !mode_specific(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    for (mode, prepare, reload) in modes {
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
        let hourly = hourly(&mut fixture, "fuel-days", prepare);
        let source = fixture.source("fuel-days");
        let before = economy::counts("fuel");
        let fallback = prepass::noted("fuel.fallback");
        // Fuel alone folds on this thread, where it counts.
        independent::with_test_families(&[&FUEL], || {
            prepass::with_test_prepass(2, None, || prepare(&source))
        });
        super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
        let after = economy::counts("fuel");
        let own = payloads(&source, None);
        clear(source);
        let (frames, observed, quiet) = (
            after.frames - before.frames,
            after.observed - before.observed,
            after.quiet - before.quiet,
        );
        assert!(
            hourly.keys().any(|(name, _)| name == "fuel-days-1"),
            "{mode}: days were settled"
        );
        assert_payloads_equal(&fuel(&own), &fuel(&hourly), mode);
        let mut states = 0;
        for ((name, _), bytes) in &hourly {
            let Some(branch) = name.strip_prefix("preparation-branch-") else {
                continue;
            };
            let saved = decoded(bytes)["fuel"].clone();
            assert!(!saved.is_null(), "{mode}: branch {branch}'s hourly fuel");
            assert_eq!(
                decoded(&own[&(format!("fuel-continuation-{branch}"), 0)]),
                saved,
                "{mode}: branch {branch}'s fuel continuation"
            );
            states += 1;
        }
        assert!(states >= 2, "{mode}: both branches' states");
        assert_eq!(observed + quiet, total, "{mode}: every hour folded once");
        assert!(quiet > total / 2, "{mode}: {quiet} of {total} hours quiet");
        assert!(
            frames < total / 2,
            "{mode}: {frames} frames of {total} hours"
        );
        assert_eq!(
            prepass::noted("fuel.fallback"),
            fallback,
            "{mode}: no block left to hourly observation"
        );
    }
}

#[test]
fn experience_gain_runs_publish_and_save_as_hourly_preparation_across_quiet_hours_and_append() {
    let frames = |from: i32, to: i32| -> Vec<DecodedFrame> {
        (from..to)
            .map(|at| {
                let mut frame = ledger_frame(1, BASE + at);
                *frame.tables[TableId::CountryExperienceGains.index()] =
                    TableView::Complete(TableRows::CountryExperienceGains(
                        [COUNTRY, OTHER]
                            .into_iter()
                            .enumerate()
                            .map(|(index, country)| CountryExperienceGainRow {
                                country,
                                instance: if index == 0 && at >= 45 { 2 } else { 1 },
                                complete: u8::from(index != 0 || at != 37),
                                army_raw: if index == 0 { i64::from(at / 8) } else { 7 },
                                air_raw: i64::MAX,
                                ..Default::default()
                            })
                            .collect(),
                    ));
                frame
            })
            .collect()
    };
    let first = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0, 32))]).unwrap();
    let grown = first
        .append(&[continuation_records(&first, 1, frames(32, 64))])
        .unwrap();
    let grown = grown
        .append(&[continuation_records(&grown, 1, frames(64, 96))])
        .unwrap();
    let mut fixture = Fixture::new(&first);
    let both = |fixture: &mut Fixture| {
        fixture.replace(&first);
        let source = fixture.source("experience-gain-runs");
        measured(&source);
        drop(source);
        fixture.replace(&grown);
        let source = fixture.source("experience-gain-runs");
        measured(&source);
        let pages = payloads(&source, None);
        clear(source);
        pages
    };
    let expected = independent::with_test_hourly(|| both(&mut fixture));
    let actual = independent::with_test_families(&ledgers(), || {
        prepass::with_test_prepass(3, None, || both(&mut fixture))
    });
    assert_same(
        &actual,
        &expected,
        "experience gain runs, resume and append",
        true,
    );
}
