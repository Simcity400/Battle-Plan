//! Air superiority prepared on its own, with its own read of the country
//! groups, beside the hourly walk, from blocks read by their changes: it
//! publishes and saves exactly what hourly preparation does across quiet
//! spans, day boundaries and a war's recorded end inside them, scores moving
//! every hour of an air war, fields it does not read changing every hour (war
//! casualties, plane counts, roster ordinals, balances), a gap, two branches,
//! append, resume and refusals (its own and the groups'), with one and
//! several pre-pass workers, alone or with the groups prepared on their own;
//! its own read of the groups is the groups' hourly read; its queries answer
//! while the hourly walk has accepted nothing; and its quiet hours build no
//! frame.
use super::family_lanes::{assert_payloads_equal, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::air_superiority::block;
use dashboard_base::prepared::kept::counted;
use independent::Own;
use replay::rows::{
    AirTheatreGroupRow, AirTheatreMemberRow, AirTheatreRow, AirWingRow, FactionMemberRow,
    FactionRow, HumanControllerRow, RaidAirScoreRow, WarRelationRow,
};

type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;
const D: u32 = 100_003;

/// Branch 1's last hour.
const LAST: i32 = 216;

/// The groups' and air superiority's tables at `hour` of `branch`, over the
/// four countries A to D (`frame`), whose balances move every hour.
///
/// Faction 5 holds A and B, and C from 130 (inside a block); A is human. A
/// and C are at war from 110 until the war's recorded end at 140 (inside a
/// block, an hour no field changes), B and D from 150; every war's
/// casualties move every hour. C is renamed at 145. A's score in region
/// 7000 moves once a day, and every hour from 150 to 160 (an air war); C
/// scores in 7000 while at war; B scores in 7100 from 160. C's wing moves its
/// mission from 7000 to 7100 at 125; plane counts and the theatre roster's
/// ordinals move every hour; the theatres are read at hour 13 of each day.
fn air_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 4);
    if hour >= 145
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"RNM".to_vec();
    }
    let set = |observed: &mut DecodedFrame, rows: TableRows| {
        let table = rows.table();
        *observed.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        &mut observed,
        TableRows::Factions(vec![FactionRow {
            id: 5,
            name: b"mod_pact".to_vec(),
            ..Default::default()
        }]),
    );
    let mut members = vec![
        FactionMemberRow {
            country: A,
            faction: 5,
            ordinal: 0,
        },
        FactionMemberRow {
            country: B,
            faction: 5,
            ordinal: 1,
        },
    ];
    if hour >= 130 {
        members.push(FactionMemberRow {
            country: C,
            faction: 5,
            ordinal: 2,
        });
    }
    set(&mut observed, TableRows::FactionMembers(members));
    set(
        &mut observed,
        TableRows::HumanControllers(vec![HumanControllerRow {
            country: A,
            count: 1,
        }]),
    );
    let war = |first_country, second_country, start_hour, end_hour| WarRelationRow {
        first_country,
        second_country,
        start_hour,
        end_hour,
        first_casualties: i64::from(hour),
        ..Default::default()
    };
    let wars = (hour >= 110)
        .then(|| war(A, C, 110, 140))
        .into_iter()
        .chain((hour >= 150).then(|| war(B, D, 150, 0)))
        .collect();
    set(&mut observed, TableRows::WarRelations(wars));
    set(&mut observed, TableRows::Capitulation(Vec::new()));
    set(&mut observed, TableRows::LexicalTokens(Vec::new()));
    let score = |country, region, friendly, enemy| RaidAirScoreRow {
        country,
        region,
        friendly,
        enemy,
        impact: Default::default(),
    };
    let a = if (150..=160).contains(&hour) {
        100_000 + i64::from(hour) * 100
    } else {
        100_000 + i64::from(hour.div_euclid(24)) * 1_000
    };
    let mut scores = vec![score(A, 7000, a, 50_000)];
    if hour >= 160 {
        scores.push(score(B, 7100, 40_000, 0));
    }
    if (110..=140).contains(&hour) {
        scores.push(score(C, 7000, 20_000, 80_000));
    }
    set(&mut observed, TableRows::RaidAirScores(scores));
    let wing = |id, owner, region| AirWingRow {
        id,
        owner,
        planes: hour,
        mission_region: Some(region),
        ..Default::default()
    };
    set(
        &mut observed,
        TableRows::AirWings(vec![
            wing(1, A, 7000),
            wing(2, C, if hour < 125 { 7000 } else { 7100 }),
        ]),
    );
    set(
        &mut observed,
        TableRows::AirTheatres(vec![AirTheatreRow {
            id: 700,
            owner: A,
            ordinal: 0,
            name: b"Synthetic theatre".to_vec(),
        }]),
    );
    set(
        &mut observed,
        TableRows::AirTheatreGroups(vec![AirTheatreGroupRow {
            id: 701,
            theatre: 700,
            ..Default::default()
        }]),
    );
    set(
        &mut observed,
        TableRows::AirTheatreMembers(vec![AirTheatreMemberRow {
            wing: 1,
            group: 701,
            ordinal: hour.rem_euclid(3) as u32,
        }]),
    );
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
        hours.iter().map(|&hour| air_frame(1, hour)).collect()
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
                (20..=25).map(|hour| air_frame(2, hour)).collect(),
            )])
            .unwrap();
    }
    replay
}

fn superiority() -> &'static dyn Own {
    &independent::air_superiority::AIR_SUPERIORITY
}

/// Air superiority alone prepared on its own (the walk reads the groups),
/// and with the groups prepared on their own too (the walk reads neither),
/// by registered name.
fn registries() -> [&'static [&'static str]; 2] {
    [&["air_superiority"], &["groups", "air_superiority"]]
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

/// Sections only one mode writes: the checkpoint (which holds the families'
/// progress), their own coverage pages, the groups' change publisher state
/// while they are prepared on their own, air superiority's own read of the
/// groups ([`assert_groups_read`] compares it), and each branch's state
/// ([`assert_same`] compares it decoded).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name.starts_with("preparation-family-")
        || name.starts_with("preparation-branch-")
        || name.starts_with("country-group-state-")
        || name.starts_with("air-superiority-group")
}

fn branch_state(bytes: &[u8]) -> serde_json::Value {
    serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
}

/// Every section both modes publish is byte-identical (with `refusals`, but
/// for each branch's coverage page, whose refusals the families keep in their
/// own pages), air superiority's and the groups' among them. Each branch's
/// state is hourly preparation's, but for the groups' change publisher,
/// which the walk keeps only while it reads the groups; without refusals,
/// air superiority's own read of the groups is the walk's.
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
        "air-superiority-1",
        "air-superiority-1-values",
        "air-superiority-1-operands",
        "air-superiority-1-hours",
        "air-superiority-2",
        "country-groups-1",
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
        let mut own = branch_state(
            actual
                .get(&(name.clone(), *range))
                .unwrap_or_else(|| panic!("{label}: no {name}")),
        );
        let mut expected = branch_state(bytes);
        own.as_object_mut().unwrap().remove("group_changes");
        expected.as_object_mut().unwrap().remove("group_changes");
        assert_eq!(own, expected, "{label}: {name}");
        states += 1;
    }
    assert!(states > 0, "{label}: no branch state");
    if !refusals {
        assert_groups_read(actual, hourly, label);
    }
}

/// Air superiority's own read of the groups saves the groups' scan, their
/// change publisher's pages and its continuation exactly as hourly
/// preparation's groups read does, under air superiority's names.
fn assert_groups_read(actual: &Payloads, hourly: &Payloads, label: &str) {
    let mut compared = 0;
    for branch in [1, 2] {
        let scan = format!("country-groups-{branch}");
        let own = format!("air-superiority-groups-{branch}");
        assert_eq!(
            actual.get(&(own, 0)),
            hourly.get(&(scan, 0)),
            "{label}: branch {branch}'s groups read"
        );
        let state = actual
            .get(&(format!("air-superiority-group-state-{branch}"), 0))
            .unwrap_or_else(|| panic!("{label}: no group change state of branch {branch}"));
        let state: serde_json::Value = serde_json::from_slice(state).unwrap();
        let saved = branch_state(&hourly[&(format!("preparation-branch-{branch}"), 0)]);
        assert_eq!(
            state, saved["group_changes"],
            "{label}: branch {branch}'s group change publisher"
        );
        compared += 1;
    }
    for ((name, range), bytes) in hourly {
        let Some(rest) = name.strip_prefix("country-group-changes-") else {
            continue;
        };
        assert_eq!(
            actual.get(&(format!("air-superiority-group-changes-{rest}"), *range)),
            Some(bytes),
            "{label}: {name} range {range}"
        );
        compared += 1;
    }
    assert!(compared > 2, "{label}: no group change pages");
}

/// Blocks the family observed hour by hour, and folds that fell back.
fn notes() -> (usize, usize) {
    (
        prepass::noted("air_superiority.sequential"),
        prepass::noted("air_superiority.fallback"),
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

/// The family reads every table and field the groups read, and its own
/// field list names fields the schema has: a misspelt field would read every
/// block whole.
#[test]
fn air_superiority_family_reads_the_groups_tables_and_declares_its_fields() {
    for table in crate::products::country_groups::prepared::TABLES {
        assert!(block::TABLES.contains(table), "{table:?}");
    }
    let fields = (block::FAMILY.fields.unwrap())().unwrap();
    let groups = (crate::products::country_groups::prepared::FAMILY
        .fields
        .unwrap())()
    .unwrap();
    for watched in groups {
        assert!(fields.contains(&watched), "{:?}", watched.table);
    }
    for watched in &fields {
        assert!(
            block::TABLES.contains(&watched.table),
            "{:?}",
            watched.table
        );
    }
}

#[test]
fn air_superiority_prepared_on_its_own_publishes_and_saves_what_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "superiority-parity", &plain));
    for families in registries() {
        for workers in [1, 3] {
            for together in [true, false] {
                let label = format!(
                    "{} families, {workers} workers, at once {together}",
                    families.len()
                );
                let before = notes();
                let own = prepass::with_test_prepass(workers, None, || {
                    independent::with_test_only(families, || {
                        if together {
                            prepared(&mut fixture, "superiority-parity", &plain)
                        } else {
                            independent::with_test_one_after_another(|| {
                                prepared(&mut fixture, "superiority-parity", &plain)
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
}

#[test]
fn air_superiority_prepared_on_its_own_commits_resumes_reloads_and_appends_as_hourly_preparation() {
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
            let source = fixture.source("superiority-commits");
            prepare(&source);
            drop(source);
            fixture.replace(&grown);
            let source = fixture.source("superiority-commits");
            prepare(&source);
            super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        };
        let hourly = independent::with_test_hourly(|| both(&mut fixture));
        for families in registries() {
            let label = format!("{mode}, {} families", families.len());
            let own = prepass::with_test_prepass(2, None, || {
                independent::with_test_only(families, || both(&mut fixture))
            });
            assert_same(&own, &hourly, &label, false);
        }
    }
}

/// Air superiority's refusal of each branch, from the checkpoint in hourly
/// preparation, or from its own coverage page.
fn superiority_refusals(source: &Source, own: bool) -> BTreeMap<u64, Option<Refusal>> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    let (state, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    [1, 2]
        .into_iter()
        .map(|branch| {
            let refusal = if own {
                store
                    .read(
                        &key,
                        &independent::coverage_section("air_superiority", branch),
                        0,
                    )
                    .unwrap()
                    .and_then(|file| {
                        serde_json::from_reader::<_, CoveragePage>(std::io::BufReader::new(file))
                            .unwrap()
                            .refused
                            .0
                            .get("air_superiority")
                            .cloned()
                    })
            } else {
                state
                    .refused
                    .get(&branch)
                    .and_then(|refused| refused.0.get("air_superiority").cloned())
            };
            (branch, refusal)
        })
        .collect()
}

/// Its own refusals, and the groups' reaching it, at a block's first hour
/// and at an hour inside a block that no field either reads changes: at the
/// hour, with the error and the family it came from that hourly preparation
/// records.
#[test]
fn air_superiority_prepared_on_its_own_refuses_where_hourly_preparation_does() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let flag = AtomicBool::new(false);
    for (family, hour) in [
        ("air_superiority", 124),
        ("air_superiority", 128),
        ("groups", 124),
        ("groups", 128),
    ] {
        let refusals = [(family, hour)];
        let (hourly, hourly_refused) = independent::with_test_hourly(|| {
            lanes::with_test_refusals(&refusals, || {
                let source = fixture.source("superiority-refusal");
                measured(&source);
                let result = (
                    payloads(&source, None),
                    superiority_refusals(&source, false),
                );
                clear(source);
                result
            })
        });
        let expected = hourly_refused[&1].clone().expect("hourly refusal");
        assert_eq!(expected.hour, hour);
        assert_eq!(
            expected.from.as_deref(),
            (family == "groups").then_some("groups")
        );
        for families in registries() {
            let label = format!("{family} refusing at {hour}, {} families", families.len());
            let source = fixture.source("superiority-refusal");
            let sequential = prepass::noted("air_superiority.sequential");
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || {
                    independent::with_test_only(families, || measured(&source))
                })
            });
            if family == "air_superiority" {
                assert_eq!(
                    prepass::noted("air_superiority.sequential") - sequential,
                    1,
                    "{label}: the refusing block is observed hour by hour"
                );
            }
            assert_same(&payloads(&source, None), &hourly, &label, true);
            let own_refused = superiority_refusals(&source, true);
            assert_eq!(own_refused, hourly_refused, "{label}");
            assert!(own_refused[&2].is_none(), "{label}: branch 2 goes on");
            independent::with_test_only(families, || {
                let error = refusal(&source, None, 1, "air-superiority", &flag).unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{label}");
                assert_eq!(
                    error.to_string(),
                    format!("Injected refusal of {family} at hour {hour}"),
                    "{label}"
                );
                refusal(&source, None, 2, "air-superiority", &flag).unwrap();
            });
            clear(source);
        }
    }
}

fn query(source: &Source, params: serde_json::Value) -> serde_json::Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: "air-superiority".into(),
        params,
    };
    let reply =
        super::super::super::query::query(source, &request, None, &AtomicBool::new(false)).unwrap();
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    reply["data"].take()
}

/// Both widgets' default request (the choices over the branch), and the
/// timeline and moment views they draw from it, for `branch` from `first` to
/// `last`.
fn requests(branch: &str, first: i32, last: i32) -> Vec<serde_json::Value> {
    ["choices", "timeline", "moment"]
        .into_iter()
        .map(|view| {
            serde_json::json!({"branchId": branch, "view": view,
                "firstHour": if view == "moment" { last } else { first },
                "lastHour": last})
        })
        .collect()
}

#[test]
fn air_superiority_answers_from_its_own_coverage_while_the_hourly_walk_accepts_nothing() {
    let replay = scenario(LAST, true);
    let mut fixture = Fixture::new(&replay);
    let branches = [("1", 100, LAST), ("2", 20, 25)];
    // The replies of hourly preparation, with every block accepted.
    let expected: Vec<Vec<serde_json::Value>> = independent::with_test_hourly(|| {
        let source = fixture.source("superiority-hourly-replies");
        plain(&source);
        let replies = branches
            .iter()
            .map(|&(branch, first, last)| {
                requests(branch, first, last)
                    .into_iter()
                    .map(|params| query(&source, params))
                    .collect()
            })
            .collect();
        clear(source);
        replies
    });
    for families in registries() {
        let label = format!("{} families", families.len());
        independent::with_test_only(families, || {
            let source = fixture.source("superiority-stalled");
            let flag = AtomicBool::new(false);
            assert!(
                !query(&source, requests("1", 100, LAST).remove(0))["approximate"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{label}: a cold choices reply samples the blocks"
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
            assert_eq!(error.kind(), io::ErrorKind::Interrupted, "{label}");
            assert_eq!(commits, 1, "{label}");
            flag.store(false, Ordering::Release);
            assert_eq!(
                unprepared_blocks(&source, None).unwrap(),
                Some(replay.index.len()),
                "{label}: the hourly walk accepted no block"
            );
            for (position, &(branch, first, last)) in branches.iter().enumerate() {
                let id = branch.parse().unwrap();
                assert!(
                    coverage(&source, None, id, &flag).unwrap().is_none(),
                    "{label}: branch {branch}: the hourly walk covers nothing"
                );
                assert!(
                    complete(&source, None, id, "air-superiority", &flag).unwrap(),
                    "{label}: branch {branch}: air superiority covers the branch on its own"
                );
                for (index, params) in requests(branch, first, last).into_iter().enumerate() {
                    let reply = query(&source, params);
                    assert!(!reply.is_null(), "{label}: branch {branch}: view {index}");
                    assert_eq!(
                        reply, expected[position][index],
                        "{label}: branch {branch}: view {index} answers as hourly preparation does"
                    );
                }
            }
            clear(source);
        });
    }
}

/// Read by its changes, air superiority alone builds frames only at the
/// hours a field it or its groups read changes, at each day's first hour, at
/// the war's recorded end and at each block's first hour: fewer than half the
/// recorded hours although wars, planes and rosters move every hour, every
/// one kept and observed once on the carried state. Read whole, every hour
/// is a frame. Both publish what hourly preparation does.
#[test]
fn air_superiority_quiet_hours_build_no_frame() {
    let replay = scenario(LAST, true);
    let recorded = hours(&replay);
    let mut fixture = Fixture::new(&replay);
    let hourly =
        independent::with_test_hourly(|| prepared(&mut fixture, "superiority-frames", &plain));
    for whole in [true, false] {
        let label = format!("read whole {whole}");
        let before = (counted("air_superiority"), notes());
        let source = fixture.source("superiority-frames");
        let read = || {
            prepass::with_test_prepass(2, None, || {
                independent::with_test_families(&[superiority()], || plain(&source))
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
        let (observed, frames) = counted("air_superiority");
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

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "synthetic content and first-view air presentation"
)]
fn cached_air_reply_refreshes_labels_when_language_changes_without_a_publication() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    independent::with_test_hourly(|| {
        let mut fixture = Fixture::new(&scenario(111, false));
        let game = fixture.root.join("game");
        for (path, text) in [
            ("hoi4.exe", "test"),
            (
                "user/dlc_load.json",
                r#"{"enabled_mods":[],"disabled_dlcs":[]}"#,
            ),
            (
                "user/settings.txt",
                "language=l_english force_pow2_textures=no",
            ),
            (
                "map/strategicregions/synthetic.txt",
                "strategic_region={id=7000 name=REGION provinces={1}}",
            ),
            (
                "localisation/english/synthetic_l_english.yml",
                "\u{feff}l_english:\n REGION:0 \"English region\"\n",
            ),
            (
                "localisation/french/synthetic_l_french.yml",
                "\u{feff}l_french:\n REGION:0 \"R\u{e9}gion fran\u{e7}aise\"\n",
            ),
        ] {
            let path = game.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let (content, identity) = dashboard_base::test_fixtures::content_identity(&game);
        let mut wire = records(1, vec![air_frame(1, 100), air_frame(1, 101)]);
        let replay::spool::Record::SessionStart(session) = &mut wire[0] else {
            unreachable!()
        };
        session.identity = identity;
        fixture.replace(&Replay::from_records(CAMPAIGN.into(), &[wire]).unwrap());
        let source = fixture.source("air-reply-language");
        let flag = AtomicBool::new(false);
        let request = crate::transport::ProductRequest {
            source_id: source.key().source_id.clone(),
            revision: source.key().revision.clone(),
            product: "air-superiority".into(),
            params: serde_json::json!({"branchId":"1", "view":"moment", "firstHour":101, "lastHour":101, "mode":"sides"}),
        };
        let ask = || -> serde_json::Value {
            serde_json::from_str(
                &crate::prepared::query::query(&source, &request, Some(&content), &flag).unwrap(),
            )
            .unwrap()
        };
        let english = ask();
        assert_eq!(english["data"]["regions"][0]["name"], "English region");
        assert_eq!(ask(), english);
        let computed = dashboard_base::prepared::query::observations::usage(
            &source,
            "queries:air-superiority",
        )
        .2;
        assert_eq!(computed, 1);
        let (_, key) = super::super::super::files(&source, Some(&content)).unwrap();
        let epoch = crate::prepared::store_epoch(&source);
        std::fs::write(
            game.join("user/settings.txt"),
            "language=l_french force_pow2_textures=no",
        )
        .unwrap();
        assert_eq!(
            super::super::super::files(&source, Some(&content))
                .unwrap()
                .1,
            key
        );
        assert_eq!(crate::prepared::store_epoch(&source), epoch);
        let french = ask();
        assert_eq!(
            french["data"]["regions"][0]["name"],
            "R\u{e9}gion fran\u{e7}aise"
        );
        assert_eq!(french["data"]["coverage"], english["data"]["coverage"]);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &source,
                "queries:air-superiority",
            )
            .2,
            computed + 1
        );
        assert_eq!(ask(), french);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &source,
                "queries:air-superiority",
            )
            .2,
            computed + 1
        );
    });
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "cold, prefix and complete first-view preparation parity"
)]
fn air_first_views_cache_the_tail_and_replace_complete_days_with_prepared_values() {
    let replay = scenario(LAST, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        let source = fixture.source("air-sampled-first-view");
        let params = serde_json::json!({"branchId":"1", "view":"timeline", "firstHour":100, "lastHour":LAST, "mode":"sides", "top":10});
        let cold = query(&source, params.clone());
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &source,
                "queries:air-superiority",
            )
            .2,
            1
        );
        assert!(
            cold["coverage"]["firstHour"]
                .as_i64()
                .is_some_and(|first| cold["coverage"]["lastHour"].as_i64() == first.checked_sub(1))
                && cold["coverage"]["complete"] == false
        );
        assert_eq!(cold["rowsApproximate"], true);
        assert!(!cold["rows"].as_array().unwrap().is_empty());
        assert_eq!(cold["pending"], serde_json::json!([]));
        let (again, refused) =
            dashboard_base::prepared::query::progressive::without_block_decodes(|| {
                query(&source, params.clone())
            });
        assert!(!refused, "a re-ask must reuse every block sample");
        assert_eq!(again, cold);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(
                &source,
                "queries:air-superiority",
            )
            .2,
            1,
            "an unchanged publication reuses the complete reply projection"
        );
        let (bytes, bound, computed) =
            dashboard_base::prepared::query::observations::usage(&source, "air-superiority");
        assert!(bytes > 0 && bytes <= bound);
        assert_eq!(
            computed,
            source.replay_ref().index.len(),
            "one shared observation per block"
        );
        let narrow = serde_json::json!({"branchId":"1", "view":"timeline", "firstHour":124, "lastHour":LAST, "mode":"sides", "top":1});
        query(&source, narrow);
        assert_eq!(
            dashboard_base::prepared::query::observations::usage(&source, "air-superiority").2,
            computed,
            "range and row selection reuse the same block observations"
        );

        run_measured(
            &source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            |_| {},
        )
        .unwrap();
        run_measured(
            &source,
            None,
            &AtomicBool::new(false),
            true,
            Duration::ZERO,
            |_| {},
        )
        .unwrap();
        let partial = query(&source, params.clone());
        assert!(
            dashboard_base::prepared::query::observations::usage(
                &source,
                "queries:air-superiority",
            )
            .2 > 1,
            "a moved supplier must invalidate the reply"
        );
        let daily = dashboard_base::prepared::query::observations::usage(
            &source,
            "air-superiority:daily-values",
        )
        .2;
        assert!(daily > 0);
        assert_eq!(partial["coverage"]["lastHour"], 123);
        assert_eq!(partial["rowsApproximate"], true);
        plain(&source);
        let complete = query(&source, params);
        let (bytes, bound, all_days) = dashboard_base::prepared::query::observations::usage(
            &source,
            "air-superiority:daily-values",
        );
        assert!(bytes > 0 && bytes <= bound);
        assert!(
            all_days > daily,
            "only newly reached daily blocks add extractions"
        );
        assert!(complete.get("approximate").is_none());
        assert!(complete.get("rowsApproximate").is_none());
        // Only whole prepared days are exact when a daily cell crosses coverage.
        assert_eq!(partial["days"][0], complete["days"][0]);
        assert_eq!(air_day_values(&partial, 0), air_day_values(&complete, 0));
        clear(source);
    });
}

fn air_day_values(reply: &serde_json::Value, day: usize) -> serde_json::Value {
    let sides = reply["sides"].as_array().unwrap();
    let mut rows = BTreeMap::new();
    for row in reply["rows"].as_array().unwrap() {
        let mut through = 0;
        for run in row["cells"].as_array().unwrap() {
            through += run[0].as_u64().unwrap() as usize;
            if through <= day {
                continue;
            }
            let mut cell = run[1].clone();
            let remap_scores = |scores: &mut serde_json::Value| {
                if let Some(scores) = scores.as_array_mut() {
                    for score in scores.iter_mut() {
                        score[0] = sides[score[0].as_u64().unwrap() as usize]["key"].clone();
                    }
                    scores.sort_by_key(|score| score[0].as_str().unwrap().to_owned());
                }
            };
            if cell.is_array() {
                remap_scores(&mut cell);
            } else if cell.is_object() {
                for field in ["scores", "viewpoints"] {
                    remap_scores(&mut cell[field]);
                }
                for field in ["unknown", "noAirObject"] {
                    if let Some(indices) = cell[field].as_array_mut() {
                        for index in indices.iter_mut() {
                            *index = sides[index.as_u64().unwrap() as usize]["key"].clone();
                        }
                        indices.sort_by_key(|index| index.as_str().unwrap().to_owned());
                    }
                }
            }
            rows.insert(row["region"].as_u64().unwrap(), cell);
            break;
        }
    }
    serde_json::to_value(rows).unwrap()
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "5,000-country sampled air comparison"
)]
fn sampled_air_comparison_keeps_five_thousand_countries() {
    let mut first = frame(1, 120, 5000);
    for rows in [
        TableRows::Factions(vec![]),
        TableRows::FactionMembers(vec![]),
        TableRows::WarRelations(vec![]),
        TableRows::HumanControllers(vec![]),
        TableRows::Capitulation(vec![]),
        TableRows::AirWings(vec![]),
        TableRows::AirTheatres(vec![]),
        TableRows::AirTheatreGroups(vec![]),
        TableRows::AirTheatreMembers(vec![]),
    ] {
        let table = rows.table();
        *first.tables[table.index()] = TableView::Complete(rows);
    }
    *first.tables[TableId::RaidAirScores.index()] = TableView::Complete(TableRows::RaidAirScores(
        (0..5000)
            .map(|id| RaidAirScoreRow {
                country: 100_000 + id,
                region: 7000,
                friendly: i64::from(id) + 7,
                enemy: 3,
                ..Default::default()
            })
            .collect(),
    ));
    let mut last = first.clone();
    last.hour = 121;
    let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![first, last])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("air-sample-5000");
    let params = serde_json::json!({"branchId":"1", "view":"timeline", "firstHour":120,"lastHour":121, "mode":"sides", "regions":[7000]});
    let cold = query(&source, params.clone());
    assert_eq!(cold["sides"].as_array().unwrap().len(), 5000);
    plain(&source);
    let complete = query(&source, params);
    assert_eq!(air_day_values(&cold, 0), air_day_values(&complete, 0));
    clear(source);
}
