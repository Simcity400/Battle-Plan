//! The sampled families (air force and navy size, stability's parts,
//! equipment losses and the party history) prepared on
//! their own, before the hourly walk, publish and save exactly what hourly
//! preparation does, across commits, resume, reload, append, a gap and a
//! second branch, with one and several workers; refuse where hourly
//! preparation refuses, at its hour and with its text; observe every
//! recorded hour once on their own thread from blocks every one of which a
//! pre-pass worker read; and are no longer observed by the hourly walk.
use super::family_lanes::{assert_payloads_equal, assert_versions, payloads};
use super::*;
use crate::prepared::prepass;
use replay_data::{
    rows::CountryStabilityLineRow, rows::LexicalTokenRow, rows::ResourceDefinitionRow,
    rows::ResourceExchangeRow, rows::ShipRow, rows::SliceReadRow,
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
/// and ships move every hour (the force fixture's). The first country's ship
/// is of a second class from +8 (inside the second block) to +13 (the third
/// block's first two hours), and the second country's of a third at +3 and +4 (inside the
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
const PARTS: Sampled = Sampled {
    name: "parts",
    field: Some("parts"),
    gated: "stability-parts",
    observed: "parts.observed",
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

    /// Sections only one mode writes: the checkpoint (which holds the families'
    /// progress), the versions page (checked against each run's plan), the
    /// family's own coverage pages, its own continuation and the branch
    /// coverage pages (which hold the refusals of the hourly walk alone);
    /// branch states are compared without the family's state.
    fn mode_specific(self, name: &str) -> bool {
        name == "preparation-checkpoint-v1"
            || name == "preparation-versions-v1"
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
    assert_versions(actual, label);
    assert_versions(hourly, label);
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
    let modes: [PrepareReloadMode; 4] = [
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
            assert_eq!(page.refused[family.name].hour, hour, "{label}");
            assert_eq!(page.refused[family.name].hour, expected.hour, "{label}");
            assert_eq!(page.refused[family.name].error, expected.error, "{label}");
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
            lanes::with_test_coordinator_families(|| {
                lanes::with_test_lanes(2, Some(hook), || {
                    prepass::with_test_prepass(2, None, || plain(&source))
                })
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

/// A sampled family whose own thread observes every block has no pre-pass
/// lane: prepared alone, no block is read on the pre-pass pool for it (a lane
/// reading each block's campaign identity and keeping nothing did before),
/// and it publishes and covers what hourly preparation does.
#[test]
fn sampled_families_observing_every_block_have_no_prepass_lane() {
    for family in [PARTS, EQUIPMENT_LOSSES, TRADE, TERRITORY, PARTIES] {
        let replay = scenario(3, true);
        let mut fixture = Fixture::new(&replay);
        let revision = format!("{}-unlaned", family.name);
        let hourly = hourly(&mut fixture, &revision, plain);
        let source = fixture.source(&revision);
        let (reads, sequential) = (
            prepass::noted(prepass::PREPASS_READ),
            prepass::noted(&format!("{}.sequential", family.name)),
        );
        prepass::with_test_prepass(2, None, || {
            // Isolate this family's lack of a lane from the walk's own lanes.
            independent::with_test_one_after_another(|| {
                lanes::with_test_lanes(4, None, || family.own(|| plain(&source)))
            })
        });
        assert_eq!(
            prepass::noted(prepass::PREPASS_READ),
            reads,
            "{}: no block read on the pool",
            family.name
        );
        assert_eq!(
            prepass::noted(&format!("{}.sequential", family.name)),
            sequential,
            "{}: no block counted as observed for want of facts",
            family.name
        );
        let own = payloads(&source, None);
        assert_same(family, &own, &hourly, family.name, true);
        assert_covered(family, &own, family.name);
        clear(source);
    }
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
    PARTS,
    stability_parts_prepared_on_their_own_publish_and_save_what_hourly_preparation_does,
    stability_parts_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation,
    stability_parts_prepared_on_their_own_refuse_where_hourly_preparation_does,
    stability_parts_are_observed_every_hour_on_their_own_thread_and_skipped_by_the_walk
);
sampled_tests!(
    EQUIPMENT_LOSSES,
    equipment_losses_prepared_on_their_own_publish_and_save_what_hourly_preparation_does,
    equipment_losses_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation,
    equipment_losses_prepared_on_their_own_refuse_where_hourly_preparation_does,
    equipment_losses_are_observed_every_hour_on_their_own_thread_and_skipped_by_the_walk
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
        assert_eq!(&page.refused["territory"], expected);
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
        independent::with_test_one_after_another(|| {
            family.own(|| prepass::with_test_prepass(2, None, || plain(&source)))
        })
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
    folds_every_block_from(family, replay, revision, replay.index.len());
}

/// [`folds_every_block`] where `carried` hours in all are observed on the
/// carried state (each block's hours through its worker's tail), and every
/// other hour is published from a worker's rows.
fn folds_every_block_from(family: Sampled, replay: &Replay, revision: &str, carried: usize) {
    let applied_name = family.applied.expect("a family folded from workers' facts");
    let total = hours(replay);
    let blocks = carried;
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
        "{label}: only each block's hours through its worker's tail observed on the carried state"
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

/// [`sampled_frame`] whose wings' equipment, variant 1, the variant table
/// names. The table is read in its daily slot and, here, three hours into
/// each six-hour block, never at a block's first hour, as a recording's
/// blocks and daily reads fall.
fn named_variant_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = sampled_frame(branch, hour);
    set(
        &mut observed,
        TableRows::VariantDefinitions(vec![replay_data::rows::VariantDefinitionRow {
            id: 1,
            equipment_token: 1,
            ..Default::default()
        }]),
    );
    let slot = replay_data::cadence::cadence(TableId::VariantDefinitions);
    for table in TableId::ALL {
        if replay_data::cadence::cadence(table) == slot {
            observed.metrics.table_ns[table.index()] = 0;
        }
    }
    if (hour - BASE).rem_euclid(6) == 3 {
        // A slot-mate walked: the read was due.
        observed.metrics.table_ns[TableId::VariantModules.index()] = 1;
    }
    observed
}

/// Variant names hold from the read that gave them, in the variant table's
/// daily slot, which a block's first hour need not be: there a census
/// carried from the block before names its wings' equipment, while a
/// worker's, starting afresh, names none until the block reads them. Air's
/// worker takes its tail at that read, its fold observes each block through
/// it on the carried state, and every later hour is published from the
/// worker's rows, as hourly preparation publishes them; navy, which names no
/// variants, still folds from each block's first hour.
#[test]
fn force_folds_continue_from_the_blocks_variant_read_where_its_first_hour_names_none() {
    use dashboard_base::products::equipment::{VariantCatalogue, VariantHour};
    let replay = scenario_of(5, false, named_variant_frame);
    let format = replay.format;
    let mut carried = VariantCatalogue::default();
    let mut through = 0;
    let mut named = 0;
    for (number, index) in replay.index.iter().enumerate() {
        let mut first = true;
        let mut settled = false;
        replay
            .read_block_alone(
                index,
                &VariantCatalogue::TABLES,
                |_, frame| {
                    let read = VariantCatalogue::hour_of(format, frame);
                    if first && number > 0 {
                        // A census starting afresh here names nothing yet.
                        assert_eq!(read, VariantHour::Carried { complete: true });
                        let mut fresh = VariantCatalogue::default();
                        fresh.observe(format, frame);
                        assert!(fresh.current(1).is_none());
                    }
                    carried.observe(format, frame);
                    if first && carried.current(1).is_some_and(|raw| !raw.token.is_empty()) {
                        named += 1;
                    }
                    first = false;
                    if !settled {
                        through += 1;
                        settled = read != VariantHour::Carried { complete: true };
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
        assert!(settled, "block {number} reads its variants");
    }
    // The carried names hold into each block after an adjacent one (the
    // second, third and fifth); the gap before the fourth ends them.
    assert_eq!(named, 3);
    assert!(through > replay.index.len());
    folds_every_block_from(AIR, &replay, "variant-reads", through);
    folds_every_block(NAVY, &replay, "variant-reads");
}

/// [`named_variant_frame`] where designs made after a block's variant read
/// are flown before the next: the first wing flies variant 2 from +4 and the
/// second variant 3 from +10, each named by the mutation ledger at the hour
/// it is made, as the collector names designs between daily reads.
fn designed_variant_frame(branch: u64, hour: i32) -> DecodedFrame {
    let at = hour - BASE;
    let mut observed = named_variant_frame(branch, hour);
    let designs: [(u64, i32); 2] = [(2, 4), (3, 10)];
    *observed.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(
            std::iter::once((1, i32::MIN))
                .chain(designs)
                .filter(|&(_, made)| at >= made)
                .map(|(id, _)| replay_data::rows::VariantDefinitionRow {
                    id,
                    equipment_token: id as u32,
                    ..Default::default()
                })
                .collect(),
        ));
    set(
        &mut observed,
        TableRows::VariantChanges(
            designs
                .into_iter()
                .filter(|&(_, made)| at == made)
                .map(|(variant, made)| replay_data::rows::VariantChangeRow {
                    sequence: made as u64,
                    hour,
                    variant,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    if let TableView::Complete(TableRows::WingEquipment(rows)) =
        &mut *observed.tables[TableId::WingEquipment.index()]
    {
        for (row, (variant, made)) in rows.iter_mut().zip(designs) {
            if at >= made {
                row.variant = variant;
            }
        }
    }
    observed
}

/// A design the mutation ledger names after the worker's tail, flown before
/// the next variant read, is named in the rows the fold publishes from the
/// worker, as hourly preparation names it.
#[test]
fn air_folds_name_designs_made_after_the_workers_tail_as_hourly_preparation_does() {
    use crate::products::{air_force::prepared::Air, parts_race_prepared::Family};
    use dashboard_base::products::equipment::{VariantCatalogue, VariantHour};
    for table in VariantCatalogue::TABLES {
        assert!(Air::SAMPLE_TABLES.contains(&table), "{table:?}");
    }
    let replay = scenario_of(5, false, designed_variant_frame);
    let format = replay.format;
    let mut through = 0;
    for index in &replay.index {
        let mut settled = false;
        replay
            .read_block_alone(
                index,
                &VariantCatalogue::TABLES,
                |_, frame| {
                    if !settled {
                        through += 1;
                        settled = VariantCatalogue::hour_of(format, frame)
                            != VariantHour::Carried { complete: true };
                    }
                    Ok(())
                },
                || Ok(()),
            )
            .unwrap();
    }
    folds_every_block_from(AIR, &replay, "designs-after-tail", through);
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
                .map(|&index| replay_data::rows::TaskForceRow {
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
        &[records(
            1,
            (100..=102).map(returning_country_frame).collect(),
        )],
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
                            "from":100,"to":105,"width":100,"requireParts":true}),
                        &context,
                        &flag,
                    )
                    .unwrap();
                    actual.as_object_mut().unwrap().remove("summaries");
                    // Completed adapters may omit their empty preview marker;
                    // neither shape may claim an approximate hour here.
                    if let Some(approximate) = actual.get("approximate") {
                        assert_eq!(approximate, &serde_json::json!([]));
                    }
                    actual.as_object_mut().unwrap().remove("approximate");
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
        // Query the coverage written by this preparation mode. The hourly
        // reference has branch coverage, while the independent navy has its
        // own coverage; outside this scope the default registry would ask
        // the hourly reference for an independent family's absent page.
        let compare = || {
            assert!(
                product_coverage(&source, None, 1, "navy-size", &flag)
                    .unwrap()
                    .expect("navy preparation publishes coverage")
                    .complete,
                "{label}: navy preparation covers the complete recording"
            );
            matches_native(&source, label);
        };
        if own {
            NAVY.own(compare);
        } else {
            independent::with_test_hourly(compare);
        }
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
        "parts",
        "equipment_losses",
        "parties",
        "trade",
        "territory",
    ];
    // The hourly walk is cancelled at its first commit, having accepted no
    // block, while every sampled family committed every block.
    let flag = AtomicBool::new(false);
    let mut commits = 0;
    let error = independent::with_test_only(&families, || {
        independent::with_test_one_after_another(|| {
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
                "stability-parts",
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
    use replay_data::{rows::FactionMemberRow, rows::FactionRow};
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
    let coverage_page = |source: &Source, section: &str, last_hour: i32, complete: bool| {
        let (store, key) = super::super::super::files(source, None).unwrap();
        let mut writer = store.extend(&key).unwrap();
        json(
            &mut writer,
            section,
            &CoveragePage {
                coverage: Coverage {
                    schema: SCHEMA,
                    first_hour: BASE,
                    last_hour,
                    complete,
                },
                refused: Default::default(),
            },
        )
        .unwrap();
        writer.commit().unwrap();
    };
    let expected: Vec<Vec<Value>> = {
        let source = fixture.source("territory-staggered-hourly");
        independent::with_test_hourly(|| measured(&source));
        // Keep the complete identity supplier while moving the hourly
        // reference's other coverage back. Faction totals require the whole
        // identity history, which the independent setup has already prepared.
        coverage_page(
            &source,
            &independent::coverage_section("identities", 1),
            BASE + 31,
            true,
        );
        let expected = independent::with_test_only(&["identities"], || {
            // The first three requests read territory and its accepted prefix.
            // Give their hourly reference that same prefix, keeping its pages
            // ahead. Political power reads no territory pages, so its reference
            // retains complete branch coverage, as the independent setup does.
            coverage_page(&source, &coverage_section(1), inside, false);
            let mut within: Vec<_> = requests(inside)
                .iter()
                .take(3)
                .map(|(product, params)| query(&source, product, params))
                .collect();
            coverage_page(&source, &coverage_section(1), BASE + 31, true);
            let (product, params) = requests(inside).into_iter().nth(3).unwrap();
            within.push(query(&source, product, &params));
            vec![within, replies(&source, past)]
        });
        clear(source);
        expected
    };
    for (hour, replies) in [inside, past].iter().zip(&expected) {
        assert!(
            replies.iter().all(|reply| !reply.is_null()),
            "hourly preparation answers every request at {hour}"
        );
    }
    let territory_page = |source: &Source, last_hour, complete| {
        coverage_page(
            source,
            &independent::coverage_section("territory", 1),
            last_hour,
            complete,
        );
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
        let mut actual = replies(&source, inside);
        let mut complete = expected[0].clone();
        let extent = serde_json::json!({"schema":SCHEMA,"firstHour":BASE,
            "lastHour":inside,"complete":false});
        for (index, reply) in actual.iter_mut().take(3).enumerate() {
            assert_eq!(reply["coverage"], extent, "request {index}");
            reply.as_object_mut().unwrap().remove("coverage");
            complete[index].as_object_mut().unwrap().remove("coverage");
            if index != 0 {
                assert_eq!(reply["pending"], serde_json::json!([]));
            }
        }
        assert_eq!(
            actual, complete,
            "within territory's coverage, as hourly preparation"
        );
        let waiting = replies(&source, past);
        assert!(waiting[0].is_null(), "territory history still waits");
        for index in 1..3 {
            // A moment scoreboard keeps its known identity rows while the
            // territory cells wait; an index cannot rank those unknown cells.
            let mut pending = expected[1][index].clone();
            pending["coverage"] = extent.clone();
            pending["pending"] = serde_json::json!([
                {"firstHour":inside+1,"lastHour":BASE+17},
                {"firstHour":BASE+20,"lastHour":past}
            ]);
            pending["preparing"] = serde_json::json!(["territory"]);
            if index == 1 {
                pending["data"]["series"] = serde_json::json!([]);
                pending["maxima"] = serde_json::json!({});
            } else {
                for row in pending["data"]["series"].as_array_mut().unwrap() {
                    for column in row["columns"].as_array_mut().unwrap() {
                        column["runs"] = serde_json::json!([]);
                        column["readRuns"] = serde_json::json!([]);
                    }
                }
            }
            assert_eq!(
                waiting[index], pending,
                "request {index} waits for territory"
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

/// Signed drains change only their amounts most hours. The log prunes an
/// earlier owner's rows, holds two raw owners of one slot, and also records
/// an unresolved owner. Truncated log reads retain their supplied rows.
fn loss_frame(branch: u64, hour: i32) -> DecodedFrame {
    use replay_data::{
        rows::CombatLogEquipmentRow, rows::CountryLinkRow, rows::EquipmentDestructionCause,
        rows::EquipmentDestructionRow,
    };
    let mut frame = sampled_frame(branch, hour);
    let at = hour - BASE - if branch == 2 { 100 } else { 0 };
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.insert(
            0,
            CountryLinkRow {
                id: 42,
                canonical_slot: 0,
            },
        );
    }
    set(
        &mut frame,
        TableRows::EquipmentDestruction(
            (0..2)
                .map(|index| EquipmentDestructionRow {
                    country: FIRST + index,
                    cause: EquipmentDestructionCause::LandCombat,
                    variant: 17_u64 << 32
                        | u64::from(engine_native::cequipment_variant::REFERENCE_TYPE),
                    amount_raw: if at % 2 == 0 {
                        100 + i64::from(at)
                    } else {
                        -50
                    },
                    ..Default::default()
                })
                .collect(),
        ),
    );
    let mut log = Vec::new();
    for country in [42, FIRST, FIRST + 1, 999_999] {
        for group in 0..3 {
            if country == FIRST && group == 0 && at >= 8 {
                continue;
            }
            log.push(CombatLogEquipmentRow {
                country,
                group,
                variant: 17,
                amount_raw: 100 + i64::from(at / 3),
                ..Default::default()
            });
        }
    }
    set(&mut frame, TableRows::CombatLogEquipment(log.clone()));
    if at == 13 {
        *frame.tables[TableId::CombatLogEquipment.index()] = TableView::Truncated {
            rows: TableRows::CombatLogEquipment(log),
            missing: 1,
        };
    }
    let mut reads = frame
        .complete::<replay_data::rows::tables::SliceReads>()
        .unwrap()
        .to_vec();
    reads.extend([42, FIRST, FIRST + 1, 999_999].map(|owner| SliceReadRow {
        table: TableId::CombatLogEquipment.index() as u32,
        owner,
        sample_hour: hour - at.rem_euclid(3),
    }));
    reads.sort_by_key(SliceReadRow::key);
    set(&mut frame, TableRows::SliceReads(reads));
    frame
}

#[test]
fn loss_owner_ranges_and_amount_only_drains_publish_and_resume_as_hourly_preparation() {
    let first = scenario_of(3, false, loss_frame);
    let grown = scenario_of(5, true, loss_frame);
    let mut fixture = Fixture::new(&first);
    let both = |fixture: &mut Fixture| {
        fixture.replace(&first);
        let source = fixture.source("loss-owner-ranges");
        measured(&source);
        drop(source);
        fixture.replace(&grown);
        let source = fixture.source("loss-owner-ranges");
        measured(&source);
        let pages = payloads(&source, None);
        clear(source);
        pages
    };
    let expected = independent::with_test_hourly(|| both(&mut fixture));
    let actual =
        EQUIPMENT_LOSSES.own(|| prepass::with_test_prepass(3, None, || both(&mut fixture)));
    assert_same(
        EQUIPMENT_LOSSES,
        &actual,
        &expected,
        "loss owner ranges, resume and append",
        true,
    );
}

#[test]
fn occupied_territory_channels_match_hourly_payloads_after_resume_append_and_clear() {
    use replay_data::rows::{ProvinceRow, StateRow};
    fn occupied_frame(branch: u64, hour: i32) -> DecodedFrame {
        let mut read = sampled_frame(branch, hour);
        let at = hour - BASE - if branch == 2 { 100 } else { 0 };
        let owner = if (8..=13).contains(&at) {
            FIRST + 1
        } else {
            FIRST
        };
        let controller = if (3..=5).contains(&at) {
            FIRST
        } else {
            FIRST + 1
        };
        set(
            &mut read,
            TableRows::States(vec![StateRow {
                id: 1,
                owner,
                controller,
                ..Default::default()
            }]),
        );
        set(
            &mut read,
            TableRows::Provinces(vec![
                ProvinceRow {
                    id: 1,
                    state: 1,
                    owner,
                    controller,
                    victory_points: at - 9,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 2,
                    state: 1,
                    owner: FIRST + 1,
                    controller: FIRST,
                    victory_points: 0,
                    ..Default::default()
                },
                ProvinceRow {
                    id: 3,
                    owner: 0,
                    controller: FIRST,
                    victory_points: 19,
                    ..Default::default()
                },
            ]),
        );
        for table in match at {
            12 => vec![TableId::States],
            15 => vec![TableId::Provinces],
            _ => vec![],
        } {
            *read.tables[table.index()] = TableView::Unavailable {
                reason: "independent occupied read".into(),
            };
        }
        read
    }
    let first = scenario_of(3, false, occupied_frame);
    let grown = scenario_of(5, true, occupied_frame);
    let mut fixture = Fixture::new(&first);
    let prepare = |fixture: &mut Fixture| {
        fixture.replace(&first);
        let source = fixture.source("occupied-resume");
        measured(&source);
        drop(source);
        fixture.replace(&grown);
        let source = fixture.source("occupied-resume");
        measured(&source);
        let pages = payloads(&source, None);
        for channel in crate::products::territory::prepared::VALUE_CHANNELS {
            assert!(
                pages
                    .keys()
                    .any(|(name, _)| name == &format!("territory-values-1-{channel}"))
            );
            assert!(
                pages
                    .keys()
                    .any(|(name, _)| name == &format!("territory-ranks-1-{channel}"))
            );
        }
        assert!(
            pages.keys().any(|(name, _)| name == "territory-values-1-8"),
            "native owned states remain separate"
        );
        clear(source);
        pages
    };
    let hourly = independent::with_test_hourly(|| prepare(&mut fixture));
    let own = TERRITORY.own(|| prepare(&mut fixture));
    assert_same(TERRITORY, &own, &hourly, "occupied resume and append", true);
    assert_covered(TERRITORY, &own, "occupied resume and append");
}
