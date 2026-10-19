//! The division log and the naval family prepared on their own, before the
//! hourly walk, publish and save exactly what hourly preparation does, across
//! commits, resume, reload, append, a gap and a second branch, with one and
//! several workers; refuse where hourly preparation refuses; answer their
//! queries while the hourly walk has accepted nothing; and the division log
//! observes only each block's first hour on its own thread, the naval family
//! every recorded hour once.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use independent::windowed;
use replay_data::{
    rows::DestroyedDivisionsRow, rows::DivisionEquipmentDeletionRow, rows::DivisionRow,
    rows::DivisionStatusRow, rows::DivisionTemplateRow, rows::LandEventCoverageRow,
    rows::NameOperands, rows::NavalCombatParticipantRow, rows::NavalCombatRow, rows::Recorded,
    rows::SunkShipEntry, rows::SunkShipsRow,
};
use serde_json::Value;

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

/// Day 1000's first hour.
const BASE: i32 = 1000 * 24;
/// The first country of `frame`: raw id, canonical slot 0.
const FIRST: u32 = 100_000;
/// The second country: raw id, canonical slot 1.
const SECOND: u32 = 100_001;

fn set(observed: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *observed.tables[index] = TableView::Complete(rows);
}

fn handle(id: u32) -> u64 {
    u64::from(id) << 32 | 4713
}

/// Divisions live at `hour` of `branch`, by offset from the branch's first
/// hour: division 1 throughout; 2 at +3..=+8 and again from +21 (a later
/// block, after the gap); 3 from +13 until its destruction at +27. Branch
/// 2's division 4 appears at +2.
fn divisions(branch: u64, at: i32) -> Vec<u32> {
    let mut live = vec![1];
    if branch == 1 {
        if (3..=8).contains(&at) || at >= 21 {
            live.push(2);
        }
        if (13..=27).contains(&at) {
            live.push(3);
        }
    } else if at >= 2 {
        live.push(4);
    }
    live
}

/// The windowed logs' tables at `hour` of `branch`. Every division moves and
/// its status changes every hour; division 3 is destroyed at +27 (branch 1).
/// A naval combat at province 5 is open at +12..=+16 (branch 1), and a ship
/// sunk at +14 is retained from then on; both countries' balances (in
/// `frame`) change every hour.
fn windowed_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let start = BASE + if branch == 2 { 100 } else { 0 };
    let at = hour - start;
    let live = divisions(branch, at);
    let template = |id: u32| u64::from(id % 2) << 32 | 4714;
    set(
        &mut observed,
        TableRows::Divisions(
            live.iter()
                .map(|&id| DivisionRow {
                    id: handle(id),
                    roster_owner: if id % 2 == 0 { SECOND } else { FIRST },
                    logical_owner: FIRST,
                    expeditionary_owner: u32::MAX,
                    province: id * 1000 + at as u32,
                    template: template(id),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::DivisionStatus(
            live.iter()
                .map(|&id| DivisionStatusRow {
                    division: handle(id),
                    supply_ratio: i64::from(at),
                    naming: Recorded::Value(Some(NameOperands {
                        custom: format!("Unit {id}").into_bytes(),
                        ..Default::default()
                    })),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        &mut observed,
        TableRows::DivisionTemplates(
            (0..2)
                .map(|id| DivisionTemplateRow {
                    template: template(id),
                    owner: FIRST,
                    name: format!("Template {id}").into_bytes(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(&mut observed, TableRows::DivisionTemplateIcons(Vec::new()));
    let destroyed = branch == 1 && at == 27;
    set(
        &mut observed,
        TableRows::DivisionEquipmentDeletions(if destroyed {
            vec![DivisionEquipmentDeletionRow {
                division_handle: handle(3),
                country: FIRST,
                expeditionary_owner: u32::MAX,
                disposition: replay_data::rows::DivisionDeletionDisposition::Loss,
                cause: Some(replay_data::rows::EquipmentDestructionCause::DivisionDestroyed),
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableRows::DestroyedDivisions(if destroyed {
            vec![DestroyedDivisionsRow {
                id: 1,
                division: handle(3),
                owner: FIRST,
                killer_country: SECOND,
                killer_division: Some(handle(9)),
                province: 17,
                hour,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableRows::LandEventCoverage(vec![LandEventCoverageRow {
            from_hour: hour - 1,
            to_hour: hour,
            complete: true,
            ..Default::default()
        }]),
    );
    let fighting = branch == 1 && (12..=16).contains(&at);
    set(
        &mut observed,
        TableRows::NavalCombats(if fighting {
            vec![NavalCombatRow {
                id: 1,
                province: 5,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        &mut observed,
        TableRows::NavalCombatParticipants(if fighting {
            (0..2)
                .map(|side| NavalCombatParticipantRow {
                    combat: 1,
                    side,
                    battle_slot: 0,
                    ship: 10 + u64::from(side),
                    owner: if side == 0 { FIRST } else { SECOND },
                    // The defender's ship is sunk at +14.
                    state: if side == 1 && at >= 14 { 0x20 } else { 0 },
                })
                .collect()
        } else {
            Vec::new()
        }),
    );
    for rows in [
        TableRows::NavalCombatDetails(Vec::new()),
        TableRows::NavalResults(Vec::new()),
        TableRows::NavalResultMembers(Vec::new()),
        TableRows::NavalResultCountries(Vec::new()),
        TableRows::NavalCombatResultDetails(Vec::new()),
        TableRows::NavalResultSources(Vec::new()),
    ] {
        set(&mut observed, rows);
    }
    let sunk = branch == 1 && at >= 14;
    set(
        &mut observed,
        TableRows::SunkShips(vec![SunkShipsRow {
            entries: if sunk {
                vec![SunkShipEntry {
                    ship_name: b"synthetic retained sinking".to_vec(),
                    victim_country: SECOND,
                    attacker_country: FIRST,
                    sink_hour: start + 14,
                    battle: 1,
                    ..Default::default()
                }]
            } else {
                Vec::new()
            },
        }]),
    );
    observed.metrics.table_ns[TableId::SunkShips.index()] = 1;
    observed
}

/// Branch 1's first `blocks` blocks of six hours (+18 and +19 are a gap
/// between the third and fourth), then branch 2's one block when `second`.
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
            .map(|&offset| windowed_frame(1, BASE + offset))
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
                    .map(|offset| windowed_frame(2, BASE + offset))
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

/// One of the windowed families under test: its registry name, the field of
/// hourly preparation's branch state that holds its scan, the section its
/// own preparation saves the scan in, and a section prefix it publishes.
#[derive(Clone, Copy)]
struct Windowed {
    name: &'static str,
    field: &'static str,
    state: fn(u64) -> String,
    /// The prefix of [`Self::state`]'s sections.
    saved: &'static str,
    published: &'static str,
}

const DIVISIONS: Windowed = Windowed {
    name: "divisions",
    field: "divisions",
    state: windowed::divisions_state,
    saved: "division-log-state-",
    published: "division-log-counts-",
};

const NAVAL: Windowed = Windowed {
    name: "naval",
    field: "naval",
    state: windowed::naval_state,
    saved: "naval-battles-state-",
    published: "naval-battle-windows-",
};

impl Windowed {
    /// The family alone prepared on its own; every other family in the walk.
    fn own<T>(self, run: impl FnOnce() -> T) -> T {
        independent::with_test_only(&[self.name], run)
    }

    /// Sections only one mode writes: the checkpoint (which holds the
    /// families' progress), the family's own coverage pages, its own scan
    /// section and the branch coverage pages (which hold the refusals of the
    /// hourly walk alone); branch states are compared without the family's
    /// scan.
    fn mode_specific(self, name: &str) -> bool {
        name == "preparation-checkpoint-v1"
            || name.starts_with("preparation-family-")
            || name.starts_with("preparation-branch-")
            || name.starts_with("preparation-coverage-")
            || name.starts_with(self.saved)
    }
}

fn decompressed_json(bytes: &[u8]) -> Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical, every branch state
/// equal but for the family's scan and, with `continuation`, the family's
/// own saved scan is the one hourly preparation saves in its branch state.
fn assert_same(
    family: Windowed,
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
    assert!(
        actual
            .keys()
            .any(|(name, _)| name.starts_with(family.published)),
        "{label}: nothing published as {}",
        family.published
    );
    assert_payloads_equal(&shared(actual), &shared(hourly), label);
    let mut states = 0;
    for ((name, range), bytes) in hourly
        .iter()
        .filter(|((name, _), _)| name.starts_with("preparation-branch-"))
    {
        let mut expected = decompressed_json(bytes);
        let mut found = decompressed_json(&actual[&(name.clone(), *range)]);
        let saved = expected
            .as_object_mut()
            .unwrap()
            .remove(family.field)
            .unwrap();
        assert!(
            found
                .as_object_mut()
                .unwrap()
                .remove(family.field)
                .is_none(),
            "{label}: {name} leaves out the {} scan",
            family.name
        );
        assert_eq!(found, expected, "{label}: {name}");
        if continuation {
            let branch: u64 = name
                .strip_prefix("preparation-branch-")
                .unwrap()
                .parse()
                .unwrap();
            let own = &actual[&((family.state)(branch), 0)];
            assert_eq!(
                decompressed_json(own),
                saved,
                "{label}: branch {branch}'s {} scan",
                family.name
            );
            states += 1;
        }
    }
    assert!(!continuation || states > 0, "{label}: no saved scan");
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

fn parity(family: Windowed) {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let revision = format!("{}-parity", family.name);
    let hourly = hourly(&mut fixture, &revision, plain);
    for (workers, parallelism) in [(1, 2), (3, 4)] {
        let source = fixture.source(&revision);
        let sequential = prepass::noted(family_sequential(family));
        prepass::with_test_prepass(workers, None, || {
            lanes::with_test_lanes(parallelism, None, || family.own(|| plain(&source)))
        });
        let label = format!(
            "{}: {workers} workers, {parallelism} processors",
            family.name
        );
        assert_same(family, &payloads(&source, None), &hourly, &label, true);
        assert_eq!(
            prepass::noted(family_sequential(family)),
            sequential,
            "{label}: every block was read by a pre-pass worker"
        );
        clear(source);
    }
}

fn family_sequential(family: Windowed) -> &'static str {
    match family.name {
        "divisions" => "divisions.sequential",
        _ => "naval.sequential",
    }
}

#[test]
fn division_log_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    parity(DIVISIONS);
}

#[test]
fn naval_family_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    parity(NAVAL);
}

fn commits(family: Windowed) {
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

#[test]
fn division_log_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    commits(DIVISIONS);
}

#[test]
fn naval_family_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
    commits(NAVAL);
}

fn refusals(family: Windowed) {
    let replay = scenario(5, true);
    // At a block's first hour, inside a block (while divisions move and the
    // combat is open), and at a block's last hour (where its window is
    // published).
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
            let gated = if family.name == "divisions" {
                "division-log-manifest"
            } else {
                "naval-battles-manifest"
            };
            let error = refusal(&source, None, 1, gated, &flag).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
            assert_eq!(error.to_string(), expected.error, "{label}");
            refusal(&source, None, 2, gated, &flag).unwrap();
        });
        clear(source);
    }
}

#[test]
fn division_log_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    refusals(DIVISIONS);
}

#[test]
fn naval_family_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    refusals(NAVAL);
}

fn query(source: &Source, product: &str, params: Value) -> io::Result<Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

#[test]
fn windowed_logs_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("windowed-stalled");
    // The hourly walk is cancelled at its first commit, having accepted no
    // block, while both families committed every block.
    let flag = AtomicBool::new(false);
    let families = ["divisions", "naval"];
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
                "division-log-manifest",
                "naval-battles-manifest",
                "ships-sunk-manifest",
            ] {
                let covered = product_coverage(&source, None, branch, product, &flag)?
                    .unwrap_or_else(|| panic!("{product} covers branch {branch}"));
                assert!(covered.complete, "{product}, branch {branch}");
            }
            assert!(
                coverage(&source, None, branch, &flag)?.is_none_or(|branch| !branch.complete),
                "branch {branch} itself is not covered"
            );
            let id = branch.to_string();
            let last = if branch == 1 { 4 } else { 0 };
            for (product, params) in [
                (
                    "division-log-manifest",
                    serde_json::json!({ "branchId": id }),
                ),
                (
                    "division-log",
                    serde_json::json!({ "branchId": id, "window": last }),
                ),
                (
                    "naval-battles-manifest",
                    serde_json::json!({ "branchId": id }),
                ),
                (
                    "naval-battles",
                    serde_json::json!({ "branchId": id, "window": last }),
                ),
            ] {
                assert!(
                    !query(&source, product, params)?.is_null(),
                    "branch {branch}: {product} answers"
                );
            }
            assert!(
                !query(&source, "subjects", serde_json::json!({ "branchId": id }))?.is_null(),
                "branch {branch}: a family of the hourly walk answers its pending prefix"
            );
        }
        Ok(())
    })
    .unwrap();
    clear(source);
}

/// Every block's first hour continues as its extract's did (each block's
/// census, removal witnesses and country tables are read completely there),
/// so the division log's own thread observes only those hours, and publishes
/// and saves hourly preparation's pages and scan.
#[test]
fn division_log_prepared_on_its_own_observes_only_each_blocks_first_hour() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "divisions-hours", plain);
    let source = fixture.source("divisions-hours");
    let observed = prepass::noted(crate::products::division_log::block::OBSERVED);
    let fallback = prepass::noted("divisions.fallback");
    // One family alone prepares on this thread, where it counts.
    DIVISIONS.own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    assert_eq!(
        prepass::noted(crate::products::division_log::block::OBSERVED) - observed,
        replay.index.len(),
        "one hour observed on the carried scan per block"
    );
    assert_eq!(prepass::noted("divisions.fallback"), fallback);
    assert!(hours(&replay) > replay.index.len());
    assert_same(
        DIVISIONS,
        &payloads(&source, None),
        &hourly,
        "first hours",
        true,
    );
    clear(source);
}

/// The naval family commits where the hourly walk does (here after every
/// two blocks of a branch, and at each branch's end), saving its scan once
/// per commit rather than once per block, and its pages, published at each
/// block's end into pages its commit writes, are hourly preparation's.
#[test]
fn naval_family_commits_where_the_hourly_walk_does() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let mut walk = Vec::new();
    let hourly = super::super::with_test_batch_blocks(2, || {
        independent::with_test_hourly(|| {
            let source = fixture.source("naval-commit-points");
            run_checked(
                &source,
                None,
                &AtomicBool::new(false),
                |_| {},
                |next| {
                    walk.push(next);
                    Ok(())
                },
            )
            .unwrap();
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        })
    });
    let source = fixture.source("naval-commit-points");
    windowed::take_naval_saves();
    super::super::with_test_batch_blocks(2, || {
        NAVAL.own(|| prepass::with_test_prepass(2, None, || plain(&source)))
    });
    let saves = windowed::take_naval_saves();
    assert_eq!(saves, walk, "the naval family's commits are the walk's");
    assert!(
        saves.len() < replay.index.len(),
        "fewer commits than blocks: {saves:?}"
    );
    assert_same(
        NAVAL,
        &payloads(&source, None),
        &hourly,
        "two-block batches",
        true,
    );
    clear(source);
}

/// The naval family's own thread observes each recorded hour once, from
/// blocks every one of which a pre-pass worker read.
#[test]
fn naval_family_prepared_on_its_own_observes_every_recorded_hour_once() {
    let replay = scenario(5, true);
    let total = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "naval-hours", plain);
    let source = fixture.source("naval-hours");
    let observed = prepass::noted(windowed::NAVAL_OBSERVED);
    let sequential = prepass::noted("naval.sequential");
    NAVAL.own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    assert_eq!(
        prepass::noted(windowed::NAVAL_OBSERVED) - observed,
        total,
        "every recorded hour observed once"
    );
    assert_eq!(prepass::noted("naval.sequential"), sequential);
    assert_same(NAVAL, &payloads(&source, None), &hourly, "every hour", true);
    clear(source);
}

/// The naval family's own thread reads the trigger kit at each block's hours
/// through the last at which its sinking scan can first find a ship (the
/// block's first read of the sunk ships, and the read after their rows
/// change), and the naval tables alone after it; it publishes and saves what
/// hourly preparation does, observing every recorded hour once.
#[test]
fn naval_family_reads_the_trigger_kit_only_through_each_blocks_last_sinking_read() {
    let replay = scenario(5, true);
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "naval-places", plain);
    for workers in [1, 3] {
        let source = fixture.source("naval-places");
        let places = prepass::noted(windowed::NAVAL_PLACES);
        let observed = prepass::noted(windowed::NAVAL_OBSERVED);
        NAVAL.own(|| prepass::with_test_prepass(workers, None, || plain(&source)));
        // Each block's first hour; in the third, through the sinking's first
        // read at +14.
        assert_eq!(
            prepass::noted(windowed::NAVAL_PLACES) - places,
            1 + 1 + 3 + 1 + 1 + 1,
            "{workers} workers: hours read with the trigger kit"
        );
        assert_eq!(
            prepass::noted(windowed::NAVAL_OBSERVED) - observed,
            hours(&replay),
            "{workers} workers: every recorded hour observed once"
        );
        assert_same(
            NAVAL,
            &payloads(&source, None),
            &hourly,
            &format!("{workers} workers: the kit through each block's last sinking read"),
            true,
        );
        clear(source);
    }
}

/// Branch 1's first three blocks, the third's sunk ships holding from +12 one
/// entry dated +15, after the reads at +12, +13 and +14.
fn dated_scenario() -> Replay {
    let frames = |offsets: std::ops::RangeInclusive<i32>| -> Vec<DecodedFrame> {
        offsets
            .map(|offset| {
                let mut observed = windowed_frame(1, BASE + offset);
                if offset >= 12 {
                    set(
                        &mut observed,
                        TableRows::SunkShips(vec![SunkShipsRow {
                            entries: vec![SunkShipEntry {
                                ship_name: b"synthetic dated sinking".to_vec(),
                                victim_country: SECOND,
                                attacker_country: FIRST,
                                sink_hour: BASE + 15,
                                battle: 1,
                                ..Default::default()
                            }],
                        }]),
                    );
                }
                observed
            })
            .collect()
    };
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0..=5))]).unwrap();
    for offsets in [6..=11, 12..=17] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(offsets))])
            .unwrap();
    }
    replay
}

/// The sinking scan first finds a ship dated after the read of its rows at
/// the read it comes due at, although the rows did not change: the naval
/// family reads the trigger kit through that hour, so it titles the ship
/// from the frame hourly preparation titles it from.
#[test]
fn naval_family_reads_the_trigger_kit_where_a_dated_sinking_comes_due() {
    let replay = dated_scenario();
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "naval-places-due", plain);
    let source = fixture.source("naval-places-due");
    let places = prepass::noted(windowed::NAVAL_PLACES);
    NAVAL.own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    // Each block's first hour; in the third, every read until +15.
    assert_eq!(prepass::noted(windowed::NAVAL_PLACES) - places, 1 + 1 + 4);
    assert_same(
        NAVAL,
        &payloads(&source, None),
        &hourly,
        "a dated sinking comes due",
        true,
    );
    clear(source);
}

/// Branch 1's first three blocks, every hour retaining one result whose
/// member row names side 2, which a full read of the results refuses.
fn invalid_member_scenario() -> Replay {
    let frames = |offsets: std::ops::RangeInclusive<i32>| -> Vec<DecodedFrame> {
        offsets
            .map(|offset| {
                let mut observed = windowed_frame(1, BASE + offset);
                set(
                    &mut observed,
                    TableRows::NavalResults(vec![replay_data::rows::NavalResultRow {
                        id: 50,
                        province: 7,
                        result_hour: BASE,
                        winning_side: 1,
                        losing_side: 2,
                        ..Default::default()
                    }]),
                );
                set(
                    &mut observed,
                    TableRows::NavalResultMembers(vec![replay_data::rows::NavalResultMemberRow {
                        result: 50,
                        member: 1,
                        side: 2,
                        ..Default::default()
                    }]),
                );
                observed
            })
            .collect()
    };
    let mut replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames(0..=5))]).unwrap();
    for offsets in [6..=11, 12..=17] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(offsets))])
            .unwrap();
    }
    replay
}

/// After the last hour a block reads the trigger kit at, its naval tables
/// come from a read of their own; the observation is handed the views the
/// first read held for the tables not rebuilt since, so the unchanged
/// retained results are not read again and their refusal is not counted
/// again, as hourly preparation counts it once per block.
#[test]
fn naval_family_reads_its_tables_after_the_kit_as_one_read() {
    let replay = invalid_member_scenario();
    let mut fixture = Fixture::new(&replay);
    let hourly = hourly(&mut fixture, "naval-places-one-read", plain);
    let source = fixture.source("naval-places-one-read");
    let places = prepass::noted(windowed::NAVAL_PLACES);
    NAVAL.own(|| prepass::with_test_prepass(2, None, || plain(&source)));
    // Each block's first hour; in the third, through the sinking at +14.
    assert_eq!(prepass::noted(windowed::NAVAL_PLACES) - places, 1 + 1 + 3);
    assert_same(
        NAVAL,
        &payloads(&source, None),
        &hourly,
        "unchanged tables across the kit's last hour",
        true,
    );
    clear(source);
}

#[test]
fn sinking_and_loss_queries_answer_prefixes_before_the_branch_is_prepared() {
    let replay = scenario(5, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        let source = fixture.source("naval-prefix-first-views");
        let manifest = serde_json::json!({"branchId":"1"});
        let losses = serde_json::json!({"branchId":"1", "tags":[], "top":10, "kind":"both", "attribution":"lost", "period":"running"});
        for (product, params) in [
            ("ships-sunk-manifest", manifest.clone()),
            ("naval-losses", losses.clone()),
        ] {
            let cold = query(&source, product, params).unwrap();
            assert!(
                cold["coverage"]["firstHour"].as_i64().is_some_and(|first| {
                    cold["coverage"]["lastHour"].as_i64() == first.checked_sub(1)
                }) && cold["coverage"]["complete"] == false
            );
            assert!(!cold["pending"].as_array().unwrap().is_empty());
        }
        run_measured(
            &source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            |_| {},
        )
        .unwrap();
        let prefix = query(&source, "ships-sunk-manifest", manifest.clone()).unwrap();
        assert_eq!(prefix["coverage"]["lastHour"], BASE + 5);
        assert!(!prefix["pending"].as_array().unwrap().is_empty());
        let approximate = serde_json::json!([{"firstHour": BASE, "lastHour": BASE + 5}]);
        assert_eq!(prefix["approximate"], approximate);
        let window = serde_json::json!({"branchId":"1", "hour":BASE + 5});
        let page = query(&source, "ships-sunk", window.clone()).unwrap();
        assert_eq!(page["window"], 0);
        assert_eq!(page["approximate"], approximate);
        let unreached = query(
            &source,
            "ships-sunk",
            serde_json::json!({"branchId":"1", "hour":BASE + 11}),
        )
        .unwrap();
        assert_eq!(unreached["window"], 1);
        assert_eq!(unreached["indices"], serde_json::json!([]));
        assert!(!unreached["pending"].as_array().unwrap().is_empty());
        let mut reached = losses.clone();
        reached["window"] = serde_json::json!(0);
        let values = query(&source, "naval-losses", reached.clone()).unwrap();
        assert!(!values.is_null());
        assert_eq!(values["coverage"]["complete"], false);
        assert_eq!(values["pending"], serde_json::json!([]));
        assert_eq!(values["approximate"], approximate);
        // The prefix uses the same saved rows and numerical values as direct
        // reads, without waiting for the branch's full identity history.
        let (store, key) = super::super::super::files(&source, None).unwrap();
        let flag = AtomicBool::new(false);
        let context = crate::products::LoadContext { content: None };
        let mut bare = page;
        for field in ["coverage", "pending", "approximate"] {
            bare.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(
            bare,
            crate::products::ships_sunk::prepared::query(
                &source,
                "ships-sunk",
                window,
                &store,
                &key,
                &flag
            )
            .unwrap()
        );
        let mut bare = values;
        for field in ["coverage", "pending", "approximate"] {
            bare.as_object_mut().unwrap().remove(field);
        }
        assert_eq!(
            bare,
            crate::products::naval_losses::prepared::query(
                &source,
                "naval-losses",
                &reached,
                &context,
                &store,
                &key,
                &flag
            )
            .unwrap()
        );
        drop(store);
        run(&source, None, &flag, |_| {}).unwrap();
        for (product, params) in [("ships-sunk-manifest", manifest), ("naval-losses", losses)] {
            let full = query(&source, product, params).unwrap();
            assert_eq!(full["coverage"]["complete"], true);
            assert_eq!(full["pending"], serde_json::json!([]));
            assert_eq!(full["approximate"], serde_json::json!([]));
        }
        clear(source);
    });
}
