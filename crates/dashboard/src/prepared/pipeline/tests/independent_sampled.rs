//! The sampled families (air force and navy size, stability and its parts,
//! fuel, equipment losses, supply health and the party history) prepared on
//! their own, before the hourly walk, publish and save exactly what hourly
//! preparation does, across commits, resume, reload, append, a gap and a
//! second branch, with one and several workers; refuse where hourly
//! preparation refuses, at its hour and with its text; observe every
//! recorded hour once on their own thread from blocks every one of which a
//! pre-pass worker read; and are no longer observed by the hourly walk.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use replay::rows::{
    CountryFuelRow, CountryStabilityLineRow, LexicalTokenRow, ResourceDefinitionRow,
    ResourceExchangeRow, ShipRow, SliceReadRow,
};
use serde_json::Value;
use std::sync::Mutex;

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour.
const BASE: i32 = 1000 * 24;
/// The first country of `frame`: raw id, canonical slot 0.
const FIRST: u32 = 100_000;

fn set(observed: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *observed.tables[index] = TableView::Complete(rows);
    observed.metrics.table_ns[index] = 1;
}

/// Every sampled family's tables at `hour` of `branch`, two countries. Wings
/// and ships move every hour (the force fixture's), as does every country's
/// fuel; fuel capacity grows at +9. The first country's ship is of a second
/// class from +8 (inside the second block) to +13 (the third block's first
/// two hours), and the second country's of a third at +3 and +4 (inside the
/// first block of each branch), each spelled by its own token: so a census
/// starting afresh at the third block numbers the classes in another order
/// than one carried from the first, and classes are first seen inside
/// blocks. Stability inputs are read every hour and
/// change every fourth hour; the first country's stability line appears at
/// +13 and goes at +22. The first country sells the second a resource from +3
/// to +27, delivering more every hour.
fn sampled_frame(branch: u64, hour: i32) -> DecodedFrame {
    let at = hour - BASE - if branch == 2 { 100 } else { 0 };
    let mut observed = frame(branch, hour, 2);
    let forces = crate::products::parts_race_prepared::test_fixtures::frame(hour, 2);
    for table in [
        TableId::AirWings,
        TableId::WingEquipment,
        TableId::TaskForces,
        TableId::Ships,
        TableId::ShipModels,
        TableId::LexicalTokens,
        TableId::Divisions,
        TableId::DivisionStatus,
        TableId::DivisionLandInputs,
        TableId::DivisionCensus,
        TableId::DivisionTemplateIcons,
    ] {
        observed.tables[table.index()] = forces.tables[table.index()].clone();
        observed.metrics.table_ns[table.index()] = forces.metrics.table_ns[table.index()];
    }
    set(
        &mut observed,
        TableRows::Ships(
            (0..2u32)
                .map(|index| ShipRow {
                    id: u64::from(index) + 1,
                    task_force: u64::from(index) + 1,
                    class_id: match (index, at) {
                        (0, 8..=13) => 3,
                        (1, 3..=4) => 4,
                        _ => 2,
                    },
                    strength: 100_000 + i64::from(hour),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::LexicalTokens(
            [
                (1, "synthetic battalion"),
                (2, "synthetic ship"),
                (3, "synthetic cruiser"),
                (4, "synthetic carrier"),
            ]
            .into_iter()
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::CountryFuel(
            (0..2)
                .map(|index| CountryFuelRow {
                    country: FIRST + index,
                    current_raw: i64::from(at) * 10 + i64::from(index),
                    maximum_raw: 32_768 + if at >= 9 { 1_000 } else { 0 },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut inputs = crate::test_fixtures::stability_inputs();
    inputs.stability_raw = 50_000 + i64::from(at / 4) * 1_000;
    let lines = if (13..22).contains(&at) {
        vec![CountryStabilityLineRow {
            country: FIRST,
            key: 0x8d,
            kind: CountryStabilityLineRow::IDEA,
            identity: b"mod/sampled".to_vec(),
            value: 10_000,
            ..Default::default()
        }]
    } else {
        Vec::new()
    };
    let stability = crate::test_fixtures::stability_frame(hour, inputs.clone(), hour, Vec::new());
    observed.tables[TableId::ModifierDefinitions.index()] =
        stability.tables[TableId::ModifierDefinitions.index()].clone();
    set(
        &mut observed,
        TableRows::CountryStabilityInputs(
            (0..2)
                .map(|index| {
                    let mut row = inputs.clone();
                    row.country = FIRST + index;
                    row
                })
                .collect(),
        ),
    );
    set(&mut observed, TableRows::CountryStabilityLines(lines));
    set(
        &mut observed,
        TableRows::SliceReads(
            [
                TableId::CountryStabilityInputs,
                TableId::CountryStabilityLines,
            ]
            .into_iter()
            .flat_map(|table| {
                (0..2).map(move |index| SliceReadRow {
                    table: table.index() as u32,
                    owner: FIRST + index,
                    sample_hour: hour,
                })
            })
            .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::ResourceDefinitions(vec![ResourceDefinitionRow {
            id: 0,
            token: 1,
            active: 1,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableRows::ResourceExchanges(if (3..=27).contains(&at) {
            vec![ResourceExchangeRow {
                id: 1,
                exporter: FIRST,
                importer: FIRST + 1,
                resource: 0,
                delivered: i64::from(at) * 100_000,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    observed
}

/// [`sampled_frame`] with a third country that holds no wings and no ships.
/// Its countries and links are the same at every hour, so the join a census
/// carries holds at each block's first hour after an adjacent block.
fn idle_country_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = sampled_frame(branch, hour);
    let countries = frame(branch, hour, 3);
    for table in [TableId::Countries, TableId::CountryLinks] {
        observed.tables[table.index()] = countries.tables[table.index()].clone();
        observed.metrics.table_ns[table.index()] = countries.metrics.table_ns[table.index()];
    }
    observed
}

/// Branch 1's first `blocks` blocks of six hours (+18 and +19 are a gap
/// between the third and fourth), then branch 2's one block when `second`.
fn scenario(blocks: usize, second: bool) -> Replay {
    scenario_of(blocks, second, sampled_frame)
}

/// [`scenario`] with each hour's frame from `sampled`.
fn scenario_of(blocks: usize, second: bool, sampled: fn(u64, i32) -> DecodedFrame) -> Replay {
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
            .map(|&offset| sampled(1, BASE + offset))
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
                    .map(|offset| sampled(2, BASE + offset))
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

/// One sampled family under test: its registry name, the field of hourly
/// preparation's branch state that holds its state (none for the party
/// history, whose saved page is its state in both), the product whose
/// refusal it answers, and the count of hours its own thread observes.
#[derive(Clone, Copy)]
struct Sampled {
    name: &'static str,
    field: Option<&'static str>,
    gated: &'static str,
    observed: &'static str,
    /// For a family folded from workers' facts, the count of later hours it
    /// published from their rows.
    applied: Option<&'static str>,
    /// A section prefix only its own preparation writes besides its
    /// continuation: territory's own read of the identity history, which the
    /// hourly walk publishes as the identities' own.
    own_read: Option<&'static str>,
}

const AIR: Sampled = Sampled {
    name: "air",
    field: Some("air"),
    gated: "air-force-size",
    observed: "air.observed",
    applied: Some(independent::sampled::AIR_APPLIED),
    own_read: None,
};
const NAVY: Sampled = Sampled {
    name: "navy",
    field: Some("navy"),
    gated: "navy-size",
    observed: "navy.observed",
    applied: Some(independent::sampled::NAVY_APPLIED),
    own_read: None,
};
const STABILITY: Sampled = Sampled {
    name: "stability",
    field: Some("stability"),
    gated: "country-stability",
    observed: "stability.observed",
    applied: None,
    own_read: None,
};
const FUEL: Sampled = Sampled {
    name: "fuel",
    field: Some("fuel"),
    gated: "country-fuel",
    observed: "fuel.observed",
    applied: None,
    own_read: None,
};
const EQUIPMENT_LOSSES: Sampled = Sampled {
    name: "equipment_losses",
    field: Some("equipment_losses"),
    gated: "equipment-losses",
    observed: "equipment_losses.observed",
    applied: None,
    own_read: None,
};
const SUPPLY: Sampled = Sampled {
    name: "supply",
    field: Some("supply"),
    gated: "supply-health",
    observed: "supply.observed",
    applied: None,
    own_read: None,
};
const TRADE: Sampled = Sampled {
    name: "trade",
    field: Some("trade"),
    gated: "trade-flows",
    observed: "trade.observed",
    applied: None,
    own_read: None,
};
const TERRITORY: Sampled = Sampled {
    name: "territory",
    field: Some("territory"),
    gated: "territory",
    observed: "territory.observed",
    applied: None,
    own_read: Some("territory-identities-"),
};
const PARTIES: Sampled = Sampled {
    name: "parties",
    field: None,
    gated: "party-history",
    observed: "parties.observed",
    applied: None,
    own_read: None,
};

impl Sampled {
    /// The family alone prepared on its own; every other family in the walk.
    fn own<T>(self, run: impl FnOnce() -> T) -> T {
        independent::with_test_only(&[self.name], run)
    }

    /// Its own continuation of `branch`.
    fn continuation(self, branch: u64) -> String {
        format!("{}-continuation-{branch}", self.name)
    }

    /// Sections only one mode writes: the checkpoint (which holds the
    /// families' progress), the family's own coverage pages, its own
    /// continuation and the branch coverage pages (which hold the refusals of
    /// the hourly walk alone); branch states are compared without the
    /// family's state.
    fn mode_specific(self, name: &str) -> bool {
        name == "preparation-checkpoint-v1"
            || name.starts_with("preparation-family-")
            || name.starts_with("preparation-branch-")
            || name.starts_with("preparation-coverage-")
            || name.starts_with(&format!("{}-continuation-", self.name))
            || self.own_read.is_some_and(|prefix| name.starts_with(prefix))
    }
}

fn decompressed_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical, every branch state
/// equal but for the family's state, and, with `continuation`, the family's
/// own continuation is the state hourly preparation saves in its branch
/// state.
fn assert_same(
    family: Sampled,
    actual: &Payloads,
    hourly: &Payloads,
    label: &str,
    continuation: bool,
) {
    let shared = |payloads: &Payloads| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| !family.mode_specific(name))
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    let mut states = 0;
    for ((name, range), bytes) in hourly
        .iter()
        .filter(|((name, _), _)| name.starts_with("preparation-branch-"))
    {
        let mut expected = decompressed_json(bytes);
        let mut found = decompressed_json(&actual[&(name.clone(), *range)]);
        let Some(field) = family.field else {
            assert_eq!(found, expected, "{label}: {name}");
            continue;
        };
        let saved = expected.as_object_mut().unwrap().remove(field).unwrap();
        assert!(
            found.as_object_mut().unwrap().remove(field).is_none(),
            "{label}: {name} leaves out the {} state",
            family.name
        );
        assert_eq!(found, expected, "{label}: {name}");
        if continuation {
            let branch: u64 = name
                .strip_prefix("preparation-branch-")
                .unwrap()
                .parse()
                .unwrap();
            let own = &actual[&(family.continuation(branch), 0)];
            assert_eq!(
                decompressed_json(own),
                saved,
                "{label}: branch {branch}'s {} state",
                family.name
            );
            states += 1;
        }
    }
    assert!(
        !continuation || family.field.is_none() || states > 0,
        "{label}: no saved state"
    );
    // Territory's own read of the identity history is the history hourly
    // preparation publishes.
    if continuation && family.own_read.is_some() {
        for ((name, range), bytes) in hourly
            .iter()
            .filter(|((name, _), _)| name.starts_with("country-identities-"))
        {
            let branch = name.strip_prefix("country-identities-").unwrap();
            assert_eq!(
                &actual[&(format!("territory-identities-{branch}"), *range)],
                bytes,
                "{label}: branch {branch}'s identity read"
            );
        }
    }
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

/// The family's own coverage of both branches is complete.
fn assert_covered(family: Sampled, own: &Payloads, label: &str) {
    for branch in [1, 2] {
        let page: CoveragePage =
            serde_json::from_slice(&own[&(independent::coverage_section(family.name, branch), 0)])
                .unwrap();
        assert!(page.coverage.complete, "{label}: branch {branch}");
    }
}

fn parity(family: Sampled) {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let revision = format!("{}-parity", family.name);
    let hourly = hourly(&mut fixture, &revision, plain);
    let sequential = format!("{}.sequential", family.name);
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source(&revision);
        let before = prepass::noted(&sequential);
        prepass::with_test_prepass(workers, None, || {
            lanes::with_test_lanes(parallelism, None, || family.own(|| plain(&source)))
        });
        let label = format!(
            "{}: {workers} workers, {parallelism} processors",
            family.name
        );
        let own = payloads(&source, None);
        assert_same(family, &own, &hourly, &label, true);
        assert_covered(family, &own, &label);
        assert_eq!(
            prepass::noted(&sequential),
            before,
            "{label}: every block was read by a pre-pass worker"
        );
        clear(source);
    }
}

fn commits(family: Sampled) {
    let first = scenario(3, false);
    let grown = scenario(5, true);
    // The hourly walk prepares one block per run; the family, every block of
    // the recording at the run's start, resuming after the append.
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
    let revision = format!("{}-commits", family.name);
    for (mode, prepare, reload) in modes {
        let both = |fixture: &mut Fixture| -> Payloads {
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(reload));
            fixture.replace(&first);
            let source = fixture.source(&revision);
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source(&revision);
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        let own = family.own(|| prepass::with_test_prepass(2, None, || both(&mut fixture)));
        assert_same(
            family,
            &own,
            &hourly,
            &format!("{}: {mode}", family.name),
            true,
        );
    }
}

fn refusals(family: Sampled) {
    let replay = scenario(5, true);
    // At a block's first hour, inside a block and at a block's last hour.
    for hour in [BASE + 6, BASE + 14, BASE + 17] {
        let refusals = [(family.name, hour)];
        let mut fixture = Fixture::new(&replay);
        let revision = format!("{}-refusal", family.name);
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source(&revision);
                measured(&source);
                let (store, key) = super::super::super::files(&source, None).unwrap();
                let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
                let result = (payloads(&source, None), refused);
                clear(source);
                result
            })
        });
        let source = fixture.source(&revision);
        family.own(|| {
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            });
            let label = format!("{} refused at {hour}", family.name);
            let own = payloads(&source, None);
            assert_same(family, &own, &hourly, &label, false);
            // The refusal is the family's own, at the hourly hour and text.
            let expected = &hourly_refused[&1].0[family.name];
            let page: CoveragePage =
                serde_json::from_slice(&own[&(independent::coverage_section(family.name, 1), 0)])
                    .unwrap();
            assert_eq!(page.refused.0[family.name].hour, hour, "{label}");
            assert_eq!(page.refused.0[family.name].hour, expected.hour, "{label}");
            assert_eq!(page.refused.0[family.name].error, expected.error, "{label}");
            assert!(page.coverage.complete, "{label}");
            let flag = AtomicBool::new(false);
            let error = refusal(&source, None, 1, family.gated, &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(error.to_string(), expected.error, "{label}");
            refusal(&source, None, 2, family.gated, &flag).unwrap();
        });
        clear(source);
    }
}

/// The family's own thread observes each recorded hour once, from blocks
/// every one of which a pre-pass worker read, and the hourly walk observes it
/// at no hour, as it does at every hour while every family is prepared hour
/// by hour.
fn observed_on_its_own(family: Sampled) {
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let walked = |own: bool, fixture: &mut Fixture| -> (usize, Payloads) {
        let seen = Arc::new(Mutex::new(0usize));
        let trace = Arc::clone(&seen);
        let name = family.name;
        let hook: lanes::Hook = Arc::new(move |job, _observed, _changes| {
            if job == name {
                *trace.lock().unwrap() += 1;
            }
            lanes::Fault::Continue
        });
        let source = fixture.source("sampled-hours");
        let prepare = || {
            lanes::with_test_lanes(2, Some(hook), || {
                prepass::with_test_prepass(2, None, || plain(&source))
            })
        };
        if own {
            family.own(prepare);
        } else {
            independent::with_test_hourly(prepare);
        }
        let payloads = payloads(&source, None);
        clear(source);
        let seen = *seen.lock().unwrap();
        (seen, payloads)
    };
    let (walked_hourly, hourly) = walked(false, &mut fixture);
    assert_eq!(
        walked_hourly, total,
        "{}: the walk observes every hour while hourly",
        family.name
    );
    let observed = prepass::noted(family.observed);
    let applied = family.applied.map(prepass::noted);
    let fallback = prepass::noted(&format!("{}.fallback", family.name));
    let sequential = prepass::noted(&format!("{}.sequential", family.name));
    let (walked_own, own) = walked(true, &mut fixture);
    assert_eq!(walked_own, 0, "{}: the walk skips it", family.name);
    let blocks = replay.index.len();
    match (family.applied, applied) {
        (Some(name), Some(applied)) => {
            assert_eq!(
                prepass::noted(family.observed) - observed,
                blocks,
                "{}: only each block's first hour observed on the carried state",
                family.name
            );
            assert_eq!(
                prepass::noted(name) - applied,
                total - blocks,
                "{}: every later hour published from a worker's rows",
                family.name
            );
            assert_eq!(
                prepass::noted(&format!("{}.fallback", family.name)),
                fallback,
                "{}: no block fell back",
                family.name
            );
        }
        _ => assert_eq!(
            prepass::noted(family.observed) - observed,
            total,
            "{}: every recorded hour observed once on its own thread",
            family.name
        ),
    }
    assert_eq!(
        prepass::noted(&format!("{}.sequential", family.name)),
        sequential
    );
    assert_same(
        family,
        &own,
        &hourly,
        &format!("{}: hours", family.name),
        true,
    );
}

macro_rules! sampled_tests {
    ($family:ident, $parity:ident, $commits:ident, $refusals:ident, $hours:ident) => {
        #[test]
        fn $parity() {
            parity($family);
        }

        #[test]
        fn $commits() {
            commits($family);
        }

        #[test]
        fn $refusals() {
            refusals($family);
        }

        #[test]
        fn $hours() {
            observed_on_its_own($family);
        }
    };
}

sampled_tests!(
    AIR,
    air_force_size_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    air_force_size_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    air_force_size_prepared_on_its_own_refuses_where_hourly_preparation_does,
    air_force_size_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    NAVY,
    navy_size_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    navy_size_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    navy_size_prepared_on_its_own_refuses_where_hourly_preparation_does,
    navy_size_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    STABILITY,
    stability_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    stability_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    stability_prepared_on_its_own_refuses_where_hourly_preparation_does,
    stability_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    FUEL,
    fuel_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    fuel_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    fuel_prepared_on_its_own_refuses_where_hourly_preparation_does,
    fuel_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    EQUIPMENT_LOSSES,
    equipment_losses_prepared_on_their_own_publish_and_save_what_hourly_preparation_does,
    equipment_losses_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation,
    equipment_losses_prepared_on_their_own_refuse_where_hourly_preparation_does,
    equipment_losses_are_observed_every_hour_on_their_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    SUPPLY,
    supply_health_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    supply_health_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    supply_health_prepared_on_its_own_refuses_where_hourly_preparation_does,
    supply_health_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    PARTIES,
    party_history_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    party_history_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    party_history_prepared_on_its_own_refuses_where_hourly_preparation_does,
    party_history_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);

sampled_tests!(
    TERRITORY,
    territory_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does,
    territory_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation,
    territory_prepared_on_its_own_refuses_where_hourly_preparation_does,
    territory_is_observed_every_hour_on_its_own_thread_and_skipped_by_the_walk
);

/// A refusal of the identity history refuses territory, prepared on its own
/// with its own read of the history, at the hour and with the text hourly
/// preparation records for territory, as coming from the identities.
#[test]
fn territory_records_an_identities_refusal_as_hourly_preparation_does() {
    let replay = scenario(5, true);
    let hour = BASE + 14;
    let refusals = [("identities", hour)];
    let mut fixture = Fixture::new(&replay);
    let hourly_refused = independent::with_test_hourly(|| {
        lanes::with_test_refusals(&refusals, || {
            let source = fixture.source("territory-identities-refusal");
            measured(&source);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let refused = checkpoint(&store, &key, None).unwrap().unwrap().0.refused;
            clear(source);
            refused
        })
    });
    let expected = &hourly_refused[&1].0["territory"];
    assert_eq!(expected.from.as_deref(), Some("identities"));
    let source = fixture.source("territory-identities-refusal");
    TERRITORY.own(|| {
        lanes::with_test_refusals(&refusals, || {
            prepass::with_test_prepass(2, None, || measured(&source))
        });
        let own = payloads(&source, None);
        let page: CoveragePage =
            serde_json::from_slice(&own[&(independent::coverage_section("territory", 1), 0)])
                .unwrap();
        assert_eq!(&page.refused.0["territory"], expected);
    });
    clear(source);
}

sampled_tests!(
    TRADE,
    trade_flows_prepared_on_their_own_publish_and_save_what_hourly_preparation_does,
    trade_flows_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation,
    trade_flows_prepared_on_their_own_refuse_where_hourly_preparation_does,
    trade_flows_are_observed_every_hour_on_their_own_thread_and_skipped_by_the_walk
);

/// Where the carried census of a force family folded from workers' facts
/// does not continue as a worker's, the fold observes the rest of each block
/// hour by hour on the carried state, and publishes and saves what hourly
/// preparation does: every recorded hour is observed once and no later hour
/// is published from a worker's rows.
fn observes_the_rest_where_its_census_does_not_continue(family: Sampled) {
    let applied_name = family.applied.expect("a family folded from workers' facts");
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let revision = format!("{}-fallback", family.name);
    let hourly = hourly(&mut fixture, &revision, plain);
    let source = fixture.source(&revision);
    let observed = prepass::noted(family.observed);
    let applied = prepass::noted(applied_name);
    let fallback = prepass::noted(&format!("{}.fallback", family.name));
    independent::sampled::with_test_fallback(|| {
        family.own(|| prepass::with_test_prepass(2, None, || plain(&source)))
    });
    assert_eq!(prepass::noted(family.observed) - observed, total);
    assert_eq!(prepass::noted(applied_name), applied);
    assert_eq!(
        prepass::noted(&format!("{}.fallback", family.name)) - fallback,
        replay.index.len()
    );
    assert_same(family, &payloads(&source, None), &hourly, &revision, true);
    clear(source);
}

#[test]
fn air_force_size_observes_the_rest_of_a_block_where_its_census_does_not_continue() {
    observes_the_rest_where_its_census_does_not_continue(AIR);
}

#[test]
fn navy_size_observes_the_rest_of_a_block_where_its_census_does_not_continue() {
    observes_the_rest_where_its_census_does_not_continue(NAVY);
}

/// A force family folded from workers' facts, prepared on its own over
/// `replay`: each block's first hour observed on the carried state, every
/// later hour published from a worker's rows and no block falling back,
/// publishing and saving what hourly preparation does.
fn folds_every_block(family: Sampled, replay: &Replay, revision: &str) {
    let applied_name = family.applied.expect("a family folded from workers' facts");
    let total = hours(replay);
    let blocks = replay.index.len();
    let mut fixture = Fixture::new(replay);
    let hourly = hourly(&mut fixture, revision, plain);
    let source = fixture.source(revision);
    let observed = prepass::noted(family.observed);
    let applied = prepass::noted(applied_name);
    let fallback = prepass::noted(&format!("{}.fallback", family.name));
    family.own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    let label = format!("{}: {revision}", family.name);
    assert_eq!(
        prepass::noted(family.observed) - observed,
        blocks,
        "{label}: only each block's first hour observed on the carried state"
    );
    assert_eq!(
        prepass::noted(applied_name) - applied,
        total - blocks,
        "{label}: every later hour published from a worker's rows"
    );
    assert_eq!(
        prepass::noted(&format!("{}.fallback", family.name)),
        fallback,
        "{label}: no block fell back"
    );
    assert_same(family, &payloads(&source, None), &hourly, &label, true);
    clear(source);
}

/// Where a block follows an adjacent one with the same countries and links,
/// the carried census finds its join held at the block's first hour, while
/// a worker's census, starting afresh, joins anew; and a country holding no
/// forces is not observed there by the carried census, which extends its
/// history later. Neither decides the block's later hours, so the force
/// folds still continue at every block.
#[test]
fn force_folds_continue_where_the_country_join_holds_and_a_country_holds_no_forces() {
    let replay = scenario_of(5, false, idle_country_frame);
    // The join a census carries holds at the first hour of each block after
    // an adjacent one (the second, third and fifth).
    let mut countries = dashboard_base::products::force_census::Countries::default();
    let mut last: Option<i32> = None;
    let mut held = 0;
    for index in &replay.index {
        let mut first = true;
        replay
            .read_block_alone(
                index,
                &[TableId::Countries, TableId::CountryLinks],
                |_, frame| {
                    let changed = countries.update(frame)?;
                    if first && last.is_some_and(|at| at + 1 == frame.hour) {
                        assert!(!changed, "the join holds at {}", frame.hour);
                        held += 1;
                    }
                    first = false;
                    last = Some(frame.hour);
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    assert_eq!(held, 3);
    folds_every_block(AIR, &replay, "held-joins");
    folds_every_block(NAVY, &replay, "held-joins");
}

/// Three countries with a task force and a ship each at `hour`, but for the
/// second country, which with its task force and ship is away at 102 and 103.
fn returning_country_frame(hour: i32) -> DecodedFrame {
    let away = |id: u32| (102..=103).contains(&hour) && id == FIRST + 1;
    let mut observed = frame(1, hour, 3);
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *observed.tables[TableId::Countries.index()]
    {
        rows.retain(|row| !away(row.id));
    }
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *observed.tables[TableId::CountryLinks.index()]
    {
        rows.retain(|row| !away(row.id));
    }
    let present: Vec<u32> = (0..3u32).filter(|index| !away(FIRST + index)).collect();
    set(
        &mut observed,
        TableRows::TaskForces(
            present
                .iter()
                .map(|&index| replay::rows::TaskForceRow {
                    id: u64::from(index) + 1,
                    roster_owner: FIRST + index,
                    logical_owner: FIRST + index,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::Ships(
            present
                .iter()
                .map(|&index| ShipRow {
                    id: u64::from(index) + 1,
                    task_force: u64::from(index) + 1,
                    class_id: 2,
                    strength: 100_000,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 2,
            literal: b"synthetic ship".to_vec(),
        }]),
    );
    observed
}

/// A country that leaves the recorded countries and later returns had no
/// navy while it was away: the native census ends its runs and identity
/// segment where it leaves and starts them again where it returns, never
/// spanning the hours it was away; navy size prepared hour by hour and on
/// its own publishes what the native census answers; and the fold continues
/// at every block, the country leaving inside the first block and
/// returning inside the second, away at its first hour.
#[test]
fn navy_size_ends_a_country_where_it_leaves_and_starts_it_again_where_it_returns() {
    use crate::products::{
        LoadContext, Product,
        navy::{NavyGrouping, NavyMeasure, NavyOwnership, NavyParams, NavySize},
    };
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(1, (100..=102).map(returning_country_frame).collect())],
    )
    .unwrap();
    let replay = first
        .append(&[continuation_records(
            &first,
            1,
            (103..=105).map(returning_country_frame).collect(),
        )])
        .unwrap();
    let tags: Vec<String> = (0..3)
        .map(|index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap())
        .collect();
    let context = LoadContext { content: None };
    let flag = AtomicBool::new(false);
    let params = |measure, ownership, grouping| NavyParams {
        branch_id: "1".into(),
        tags: tags.clone(),
        top: None,
        measure,
        ownership,
        grouping,
    };
    // Prepared navy answers what the native census does, for every view.
    let matches_native = |source: &Source, label: &str| {
        for measure in [NavyMeasure::Ships, NavyMeasure::Strength] {
            for ownership in [NavyOwnership::Roster, NavyOwnership::Logical] {
                for grouping in [NavyGrouping::Class, NavyGrouping::Type] {
                    let params = params(measure, ownership, grouping);
                    let mut actual = crate::products::navy::prepared::query(
                        source,
                        "series-summary",
                        &serde_json::json!({"branchId":"1","tags":params.tags,
                            "family":{"kind":"navy-size","measure":measure,
                                "ownership":ownership,"grouping":grouping,"top":null},
                            "from":100,"to":105,"width":100}),
                        &context,
                        &flag,
                    )
                    .unwrap();
                    actual.as_object_mut().unwrap().remove("summaries");
                    let expected = NavySize::load(source, &params, &context, &flag).unwrap();
                    assert_eq!(
                        actual,
                        serde_json::to_value(expected).unwrap(),
                        "{label}: {measure:?} {ownership:?} {grouping:?}"
                    );
                }
            }
        }
    };
    let mut fixture = Fixture::new(&replay);
    for (label, own) in [("hourly", false), ("own", true)] {
        let source = fixture.source(&format!("navy-returning-{label}"));
        let prepare = || prepass::with_test_prepass(2, None, || plain(&source));
        if own {
            NAVY.own(prepare);
        } else {
            independent::with_test_hourly(prepare);
        }
        if !own {
            // The native census ends the country's runs and segment at 101
            // and starts them again at 104.
            let reply = NavySize::load(
                &source,
                &params(
                    NavyMeasure::Ships,
                    NavyOwnership::Roster,
                    NavyGrouping::Class,
                ),
                &context,
                &flag,
            )
            .unwrap();
            let returning = reply
                .series
                .iter()
                .find(|series| series.tag == tags[1])
                .expect("the returning country's series");
            assert_eq!(
                returning
                    .runs
                    .iter()
                    .map(|run| (run.first_hour, run.last_hour, run.starts_segment, run.total))
                    .collect::<Vec<_>>(),
                [(100, 101, true, Some(1)), (104, 105, true, Some(1))]
            );
            assert_eq!(
                returning
                    .segments
                    .iter()
                    .map(|segment| (segment.first_hour, segment.last_hour))
                    .collect::<Vec<_>>(),
                [(100, 101), (104, 105)]
            );
        }
        matches_native(&source, label);
        clear(source);
    }
    folds_every_block(NAVY, &replay, "navy-returning");
}

/// Air force size answers from its own coverage while the hourly walk has
/// accepted nothing, and a family of the hourly walk still waits.
#[test]
fn sampled_families_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("sampled-stalled");
    let families = [
        "air",
        "navy",
        "stability",
        "fuel",
        "equipment_losses",
        "supply",
        "parties",
        "trade",
        "territory",
    ];
    // The hourly walk is cancelled at its first commit, having accepted no
    // block, while every sampled family committed every block.
    let flag = AtomicBool::new(false);
    let mut commits = 0;
    let error = independent::with_test_only(&families, || {
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
    let flag = AtomicBool::new(false);
    independent::with_test_only(&families, || -> io::Result<()> {
        assert_eq!(
            unprepared_blocks(&source, None)?,
            Some(replay.index.len()),
            "the hourly walk accepted no block"
        );
        for branch in [1, 2] {
            for product in [
                "air-force-size",
                "navy-size",
                "country-stability",
                "stability-parts",
                "country-fuel",
                "party-history",
                "trade-flows",
            ] {
                let covered = product_coverage(&source, None, branch, product, &flag)?
                    .unwrap_or_else(|| panic!("{product} covers branch {branch}"));
                assert!(covered.complete, "{product}, branch {branch}");
            }
            assert!(
                coverage(&source, None, branch, &flag)?.is_none_or(|branch| !branch.complete),
                "branch {branch} itself is not covered"
            );
        }
        Ok(())
    })
    .unwrap();
    clear(source);
}

/// [`frame`]'s two countries in one recorded faction, with the tables the
/// faction rosters read.
fn faction_frame(branch: u64, hour: i32) -> DecodedFrame {
    use replay::rows::{FactionMemberRow, FactionRow};
    let mut observed = frame(branch, hour, 2);
    set(
        &mut observed,
        TableRows::Factions(vec![FactionRow {
            id: 42,
            name: b"Recorded pact".to_vec(),
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableRows::FactionMembers(
            (0..2)
                .map(|ordinal| FactionMemberRow {
                    faction: 42,
                    country: FIRST + ordinal,
                    ordinal,
                })
                .collect(),
        ),
    );
    for rows in [
        TableRows::HumanControllers(Vec::new()),
        TableRows::WarRelations(Vec::new()),
        TableRows::Capitulation(Vec::new()),
    ] {
        set(&mut observed, rows);
    }
    observed
}

/// The queries that read territory's pages (territory faction totals and
/// the scoreboard's territory column) wait for territory's own coverage,
/// not the branch's: with the hourly walk, the faction rosters and the
/// identity history covering the branch and territory one batch (its first
/// block), they answer as hourly preparation does within that batch and wait
/// past it, while a scoreboard column that reads no territory page answers
/// there. Once territory covers the branch, they answer past it too.
#[test]
fn territory_readers_wait_for_territory_coverage_while_the_branch_is_ahead() {
    let replay = scenario_of(5, false, faction_frame);
    let mut fixture = Fixture::new(&replay);
    let tags: Vec<String> = (0..2)
        .map(|index| String::from_utf8(crate::test_fixtures::scale_tag(index)).unwrap())
        .collect();
    // The first block's last hour, and an hour of the fourth.
    let (inside, past) = (BASE + 5, BASE + 22);
    let query = |source: &Source, product: &str, params: &Value| -> Value {
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: product.into(),
            params: params.clone(),
        };
        let reply =
            super::super::super::query::query(source, &request, None, &AtomicBool::new(false))
                .unwrap();
        let mut reply: Value = serde_json::from_str(&reply).unwrap();
        reply["data"].take()
    };
    let scoreboard = |column: &str, hour: i32, index: bool| -> Value {
        let selection = if index {
            serde_json::json!({"branchId": "1", "columns": [column], "tags": [],
                "top": {"count": 2, "sort": column, "descending": true}})
        } else {
            serde_json::json!({"branchId": "1", "columns": [column], "tags": tags,
                "top": null})
        };
        serde_json::json!({"selection": selection, "hour": hour, "index": index,
            "bars": index, "sort": column, "descending": true})
    };
    // Territory faction totals at the hour, the scoreboard's territory index
    // and page, then its political power page (no territory page read).
    let requests = |hour: i32| -> Vec<(&'static str, Value)> {
        vec![
            (
                "series-state",
                serde_json::json!({
                    "branchId": "1",
                    "family": {"kind": "faction-totals", "selection": {
                        "branchId": "1", "membership": "hour", "top": 5,
                        "source": {"kind": "territory", "measure": "states", "rule": "owned"},
                    }},
                    "reads": [{"hour": hour}],
                }),
            ),
            ("scoreboard", scoreboard("territory", hour, true)),
            ("scoreboard", scoreboard("territory", hour, false)),
            ("scoreboard", scoreboard("politicalPower", hour, false)),
        ]
    };
    let replies = |source: &Source, hour: i32| -> Vec<Value> {
        requests(hour)
            .iter()
            .map(|(product, params)| query(source, product, params))
            .collect()
    };
    let expected: Vec<Vec<Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("territory-staggered-hourly");
        measured(&source);
        let expected = [inside, past].map(|hour| replies(&source, hour)).to_vec();
        clear(source);
        expected
    });
    for (hour, replies) in [inside, past].iter().zip(&expected) {
        assert!(
            replies.iter().all(|reply| !reply.is_null()),
            "hourly preparation answers every request at {hour}"
        );
    }
    let territory_page = |source: &Source, last_hour: i32, complete: bool| {
        let (store, key) = super::super::super::files(source, None).unwrap();
        let mut writer = store.extend(&key).unwrap();
        json(
            &mut writer,
            &independent::coverage_section("territory", 1),
            &CoveragePage {
                coverage: Coverage {
                    schema: SCHEMA,
                    first_hour: BASE,
                    last_hour,
                    complete,
                },
                refused: Refused::default(),
            },
        )
        .unwrap();
        writer.commit().unwrap();
    };
    independent::with_test_only(&["groups", "identities", "territory"], || {
        let source = fixture.source("territory-staggered");
        let flag = AtomicBool::new(false);
        measured(&source);
        // Territory has committed its first block alone.
        territory_page(&source, inside, false);
        assert!(
            coverage(&source, None, 1, &flag).unwrap().unwrap().complete,
            "the hourly walk covers the branch"
        );
        for product in [
            "country-groups",
            crate::products::country_history::IDENTITIES_PRODUCT,
        ] {
            assert!(
                product_coverage(&source, None, 1, product, &flag)
                    .unwrap()
                    .unwrap()
                    .complete,
                "{product} covers the branch"
            );
        }
        assert_eq!(
            replies(&source, inside),
            expected[0],
            "within territory's coverage, as hourly preparation"
        );
        let waiting = replies(&source, past);
        for (index, reply) in waiting.iter().take(3).enumerate() {
            assert!(
                reply.is_null(),
                "request {index} past territory's coverage waits: {reply}"
            );
        }
        assert_eq!(
            waiting[3], expected[1][3],
            "a column reading no territory page answers past territory's coverage"
        );
        // Territory covers the branch.
        territory_page(&source, BASE + 31, true);
        assert_eq!(
            replies(&source, past),
            expected[1],
            "once territory covers the branch, as hourly preparation"
        );
        clear(source);
    });
}
