//! Country groups prepared on their own beside the hourly walk, publish and
//! save exactly what hourly preparation does (air superiority included), and
//! answer their queries while the hourly walk has accepted nothing.
use super::family_lanes::{assert_payloads_equal, assert_versions, payloads};
use super::*;
use crate::prepared::prepass;
use crate::products::country_groups::prepared::observed_hours;
use replay_data::{
    rows::CapitulationRow, rows::FactionMemberRow, rows::FactionRow, rows::HumanControllerRow,
    rows::LexicalTokenRow, rows::WarRelationRow,
};

pub(super) type Payloads = BTreeMap<(String, u64), Vec<u8>>;

const A: u32 = 100_000;
const B: u32 = 100_001;
const C: u32 = 100_002;

/// Three countries' groups at `hour`. B is human from 104 (a block's first
/// hour) to 109; C leaves faction 5 at 106 (inside a block) and joins the
/// templated faction 6 at 109; B joins faction 5 at 108 (a block's first
/// hour). A and B are at war 101 to 104, A and C from 102 until the war's
/// recorded end at 116 (inside a block); the war rows' casualties change every
/// hour without changing the war. The dictionary grows at 115 (inside a block)
/// and 118 (a block's first hour), when C is renamed. C capitulates at 119.
/// Every hour's country rows change without changing their keys.
pub(super) fn groups_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 3);
    if hour >= 118
        && let TableView::Complete(TableRows::Countries(rows)) =
            &mut *observed.tables[TableId::Countries.index()]
    {
        rows[2].tag = b"ZZZ".to_vec();
    }
    let humans: Vec<_> = [A]
        .into_iter()
        .chain((104..110).contains(&hour).then_some(B))
        .map(|country| HumanControllerRow { country, count: 1 })
        .collect();
    *observed.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(humans));
    let mut factions = vec![FactionRow {
        id: 5,
        name: b"Axis".to_vec(),
        ..Default::default()
    }];
    let mut rosters: Vec<(u64, Vec<u32>)> = vec![(
        5,
        [A].into_iter()
            .chain((hour < 106).then_some(C))
            .chain((hour >= 108).then_some(B))
            .collect(),
    )];
    if hour >= 109 {
        factions.push(FactionRow {
            id: 6,
            name: Vec::new(),
            template_token: Some(77),
            ..Default::default()
        });
        rosters.push((6, vec![C]));
    }
    *observed.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(factions));
    let mut members: Vec<_> = rosters
        .iter()
        .flat_map(|(faction, countries)| {
            countries
                .iter()
                .enumerate()
                .map(move |(ordinal, &country)| FactionMemberRow {
                    country,
                    faction: *faction,
                    ordinal: ordinal as u32,
                })
        })
        .collect();
    members.sort_by_key(|member| member.country);
    *observed.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(members));
    let war = |second_country, start_hour, end_hour| WarRelationRow {
        first_country: A,
        second_country,
        start_hour,
        end_hour,
        first_casualties: i64::from(hour),
        ..Default::default()
    };
    let wars = (101..105)
        .contains(&hour)
        .then(|| war(B, 101, 0))
        .into_iter()
        .chain((hour >= 102).then(|| war(C, 102, 116)))
        .collect();
    *observed.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(wars));
    *observed.tables[TableId::Capitulation.index()] = TableView::Complete(TableRows::Capitulation(
        [A, B, C]
            .into_iter()
            .map(|country| {
                let capitulated = country == C && hour >= 119;
                CapitulationRow {
                    country,
                    capitulated: u8::from(capitulated),
                    capitulated_hour: if capitulated { 119 } else { 0 },
                    ..Default::default()
                }
            })
            .collect(),
    ));
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(
            [
                (77, "TEMPLATE_PACT"),
                (78, "SECOND_PACT"),
                (79, "THIRD_PACT"),
            ]
            .into_iter()
            .take(1 + usize::from(hour >= 115) + usize::from(hour >= 118))
            .map(|(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
        ));
    observed
}

/// Branch 1's first `blocks` blocks of six (hours 112 and 113 are a gap
/// between the third and fourth), then branch 2's one block when `second`.
pub(super) fn scenario(blocks: usize, second: bool) -> Replay {
    let hours: [&[i32]; 6] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[108, 109, 110, 111],
        &[114, 115, 116, 117],
        &[118, 119, 120, 121],
        &[122, 123, 124, 125],
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| groups_frame(1, hour)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(hours[0]))]).unwrap();
    for &hours in &hours[1..blocks] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    if second {
        replay = replay
            .append(&[records(
                2,
                (20..=23).map(|hour| groups_frame(2, hour)).collect(),
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

fn refused(source: &Source) -> BTreeMap<u64, Refused> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    checkpoint(&store, &key, None).unwrap().unwrap().0.refused
}

pub(super) fn query(source: &Source, product: &str, branch: &str) -> io::Result<serde_json::Value> {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params: serde_json::json!({ "branchId": branch }),
    };
    let reply = super::super::super::query::query(source, &request, None, &AtomicBool::new(false))?;
    let mut reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    Ok(reply["data"].take())
}

/// Sections only one mode writes: the checkpoint (which holds the groups'
/// progress), the versions page (checked against each run's plan), the groups'
/// own coverage pages and change publisher state, and air superiority's own
/// read of the groups (and territory's of the country identities, also prepared
/// on their own). The walk's branch state is compared field by field
/// ([`assert_same`]).
fn mode_specific(name: &str) -> bool {
    name == "preparation-checkpoint-v1"
        || name == "preparation-versions-v1"
        || name.starts_with("preparation-branch-")
        || name.starts_with("preparation-family-")
        || name.starts_with("country-group-state-")
        || name.starts_with("air-superiority-group")
        || name.starts_with("territory-identities-")
}

/// Every section both modes publish is byte-identical, air superiority's
/// first. With `continuation`, the groups' change publisher state is the one
/// hourly preparation saves in its branch state.
pub(super) fn assert_same(actual: &Payloads, hourly: &Payloads, label: &str, continuation: bool) {
    assert_versions(actual, label);
    assert_versions(hourly, label);
    let shared = |payloads: &Payloads, air: bool| -> Payloads {
        payloads
            .iter()
            .filter(|((name, _), _)| {
                !mode_specific(name) && (!air || name.starts_with("air-superiority"))
            })
            .map(|(key, bytes)| (key.clone(), bytes.clone()))
            .collect()
    };
    assert!(
        !shared(hourly, true).is_empty(),
        "{label}: air superiority published nothing"
    );
    assert_payloads_equal(
        &shared(actual, true),
        &shared(hourly, true),
        &format!("{label}: air superiority"),
    );
    assert_payloads_equal(&shared(actual, false), &shared(hourly, false), label);
    // The walk's branch state is hourly preparation's, apart from the state
    // of the families prepared on their own, which it leaves out.
    let decoded = |bytes: &[u8]| -> serde_json::Value {
        serde_json::from_slice(&zstd::stream::decode_all(bytes).unwrap()).unwrap()
    };
    let mut states = 0;
    for ((name, range), bytes) in hourly
        .iter()
        .filter(|((name, _), _)| name.starts_with("preparation-branch-"))
    {
        let own = decoded(&actual[&(name.clone(), *range)]);
        let mut expected = decoded(bytes);
        expected
            .as_object_mut()
            .unwrap()
            .retain(|field, _| own.get(field).is_some());
        assert_eq!(own, expected, "{label}: {name}");
        states += 1;
    }
    assert!(states > 0, "{label}: no branch state");
    assert!(
        actual
            .keys()
            .any(|(name, _)| name.starts_with("country-groups-")),
        "{label}: the groups published nothing"
    );
    if !continuation {
        return;
    }
    let mut states = 0;
    for ((name, _), bytes) in actual {
        let Some(branch) = name.strip_prefix("country-group-state-") else {
            continue;
        };
        states += 1;
        let state: serde_json::Value = serde_json::from_slice(bytes).unwrap();
        let saved = &hourly[&(format!("preparation-branch-{branch}"), 0)];
        let saved: serde_json::Value =
            serde_json::from_slice(&zstd::stream::decode_all(saved.as_slice()).unwrap()).unwrap();
        assert_eq!(
            state, saved["group_changes"],
            "{label}: branch {branch}'s change publisher"
        );
    }
    assert!(states > 0, "{label}: no change publisher state");
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
fn country_groups_prepared_on_their_own_publish_and_save_what_hourly_preparation_does() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        let replay = scenario(6, true);
        let mut fixture = Fixture::new(&replay);
        let hourly = independent::with_test_hourly(|| {
            let source = fixture.source("groups-parity");
            run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        });
        for (workers, parallelism) in [(1, 2), (3, 4)] {
            let source = fixture.source("groups-parity");
            let before = (
                prepass::noted("groups.sequential"),
                prepass::noted("groups.fallback"),
                observed_hours(),
            );
            prepass::with_test_prepass(workers, None, || {
                lanes::with_test_lanes(parallelism, None, || {
                    run(&source, None, &AtomicBool::new(false), |_| {})
                })
            })
            .unwrap();
            let label = format!("{workers} workers, {parallelism} processors");
            assert_same(&payloads(&source, None), &hourly, &label, true);
            assert_eq!(
                (
                    prepass::noted("groups.sequential") - before.0,
                    prepass::noted("groups.fallback") - before.1
                ),
                (0, 0),
                "{label}: every block folded from its extract"
            );
            // Unchanged hours are not observed on the coordinator.
            let observed = observed_hours() - before.2;
            assert!(
                observed >= replay.index.len() && observed < hours(&replay),
                "{label}: {observed} of {} hours observed",
                hours(&replay)
            );
            clear(source);
        }
    });
}

#[test]
fn country_groups_prepared_on_their_own_commit_resume_reload_and_append_as_hourly_preparation() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        let first = scenario(3, false);
        let grown = scenario(6, true);
        let plain = |source: &Source| {
            run(source, None, &AtomicBool::new(false), |_| {}).unwrap();
        };
        // Every block committed on its own.
        let measured = |source: &Source| {
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
        };
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
                let source = fixture.source("groups-commits");
                prepare(&source);
                drop(source);
                fixture.replace(&grown);
                let source = fixture.source("groups-commits");
                prepare(&source);
                super::super::RELOAD_FLUSHED.with(|flag| flag.set(false));
                let payloads = payloads(&source, None);
                clear(source);
                payloads
            };
            let hourly = independent::with_test_hourly(|| both(&mut fixture));
            let own = both(&mut fixture);
            assert_same(&own, &hourly, mode, true);
        }
    });
}

#[test]
fn country_groups_prepared_on_their_own_refuse_where_hourly_preparation_does() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        let replay = scenario(6, true);
        // At a block's first hour, and inside a block.
        for hour in [108, 115] {
            let refusals = [("groups", hour)];
            let measured = |source: &Source| {
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
            };
            let mut fixture = Fixture::new(&replay);
            let (hourly, hourly_refused) = independent::with_test_hourly(|| {
                lanes::with_test_refusals(&refusals, || {
                    let source = fixture.source("groups-refusal");
                    measured(&source);
                    let result = (payloads(&source, None), refused(&source));
                    clear(source);
                    result
                })
            });
            let source = fixture.source("groups-refusal");
            let sequential = prepass::noted("groups.sequential");
            lanes::with_test_refusals(&refusals, || {
                prepass::with_test_prepass(2, None, || measured(&source))
            });
            let label = format!("refusal at {hour}");
            assert_eq!(
                prepass::noted("groups.sequential") - sequential,
                1,
                "{label}: the refusing block is observed hour by hour"
            );
            assert_same(&payloads(&source, None), &hourly, &label, false);
            // Air superiority refuses with the groups at the same hour.
            let text = format!("Injected refusal of groups at hour {hour}");
            assert_eq!(refused(&source), hourly_refused, "{label}");
            let branch = &hourly_refused[&1].0;
            assert_eq!(branch["groups"].hour, hour);
            assert_eq!(branch["air_superiority"].hour, hour);
            assert_eq!(branch["air_superiority"].from.as_deref(), Some("groups"));
            let error = query(&source, "country-groups", "1").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), text);
            assert!(!query(&source, "country-groups", "2").unwrap().is_null());
            clear(source);
        }
    });
}

#[test]
fn country_groups_answer_from_their_own_coverage_while_the_hourly_walk_accepts_nothing() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        use crate::products::{
            LoadContext, Product,
            country_groups::{CountryGroupParams, CountryGroups},
        };
        let replay = scenario(6, true);
        let mut fixture = Fixture::new(&replay);
        let source = fixture.source("groups-stalled");
        let flag = AtomicBool::new(false);
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
        for branch in ["1", "2"] {
            let oracle = CountryGroups::load(
                &source,
                &CountryGroupParams {
                    branch_id: branch.into(),
                },
                &LoadContext { content: None },
                &flag,
            )
            .unwrap();
            assert_eq!(
                query(&source, "country-groups", branch).unwrap(),
                serde_json::to_value(oracle).unwrap(),
                "branch {branch}: the groups answer from their own coverage"
            );
            // A product only the hourly walk prepares still waits for it.
            let waiting = registry::FAMILIES
                .iter()
                .filter(|family| !independent::registered(family.id.name()))
                .flat_map(|family| family.queries.iter().copied())
                .find(|product| independent::gating(product).is_none())
                .expect("a product of the hourly walk");
            assert!(
                product_coverage(&source, None, branch.parse().unwrap(), waiting, &flag)
                    .unwrap()
                    .is_none(),
                "branch {branch}: {waiting} still waits for the hourly walk"
            );
        }
        // The hourly walk then resumes from its first block and the groups stay.
        let accepted = run(&source, None, &flag, |_| {}).unwrap();
        assert_eq!(accepted.new_blocks, replay.index.len());
        let ready = accepted
            .sections
            .iter()
            .find(|section| section.id == "country-groups")
            .unwrap();
        assert_eq!(ready.state, State::Ready);
        clear(source);
    });
}

use crate::products::country_groups::prepared::built_frames;

/// `groups_frame` with every war row's unwatched fields moving every hour:
/// its cancel byte and both sides' casualties. The countries' balances move
/// every hour already.
fn noisy_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = groups_frame(branch, hour);
    if let TableView::Complete(TableRows::WarRelations(rows)) =
        &mut *observed.tables[TableId::WarRelations.index()]
    {
        for row in rows {
            row.cancel = u8::from(hour % 2 == 1);
            row.second_casualties = 3 * i64::from(hour);
            row.second_unknown_casualties = i64::from(hour % 5);
        }
    }
    observed
}

/// `scenario(6, true)` of noisy frames.
fn noisy_scenario() -> Replay {
    let hours: [&[i32]; 6] = [
        &[100, 101, 102, 103],
        &[104, 105, 106, 107],
        &[108, 109, 110, 111],
        &[114, 115, 116, 117],
        &[118, 119, 120, 121],
        &[122, 123, 124, 125],
    ];
    let frames = |hours: &[i32]| -> Vec<DecodedFrame> {
        hours.iter().map(|&hour| noisy_frame(1, hour)).collect()
    };
    let mut replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, frames(hours[0]))]).unwrap();
    for &hours in &hours[1..] {
        replay = replay
            .append(&[continuation_records(&replay, 1, frames(hours))])
            .unwrap();
    }
    replay
        .append(&[records(
            2,
            (20..=23).map(|hour| noisy_frame(2, hour)).collect(),
        )])
        .unwrap()
}

/// Read by their changes, the groups keep exactly the hours a whole read of
/// each block keeps, and publish and save what hourly preparation does: the
/// fields they do not watch change every hour without making an hour
/// observed, and the war ending at 116 inside the quiet hours of its block is
/// observed there. The extract is given frames only at the hours read and at
/// that war's end.
#[test]
fn country_groups_read_by_their_changes_keep_the_hours_a_whole_read_keeps() {
    // The groups alone prepared on their own; every other family hourly.
    independent::with_test_only(&["groups"], || {
        let replay = noisy_scenario();
        let mut fixture = Fixture::new(&replay);
        let hourly = independent::with_test_hourly(|| {
            let source = fixture.source("groups-changes");
            run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
            let payloads = payloads(&source, None);
            clear(source);
            payloads
        });
        let mut kept = Vec::new();
        for whole in [true, false] {
            let label = if whole {
                "read whole"
            } else {
                "read by changes"
            };
            let source = fixture.source("groups-changes");
            let before = (
                observed_hours(),
                built_frames(),
                prepass::noted("groups.sequential"),
                prepass::noted("groups.fallback"),
            );
            let read = || {
                prepass::with_test_prepass(2, None, || {
                    run(&source, None, &AtomicBool::new(false), |_| {})
                })
            };
            if whole {
                prepass::with_test_alone(read)
            } else {
                read()
            }
            .unwrap();
            assert_same(&payloads(&source, None), &hourly, label, true);
            assert_eq!(
                (
                    prepass::noted("groups.sequential") - before.2,
                    prepass::noted("groups.fallback") - before.3
                ),
                (0, 0),
                "{label}: every block folded from its extract"
            );
            let observed = observed_hours() - before.0;
            let frames = built_frames() - before.1;
            if whole {
                assert_eq!(frames, hours(&replay), "{label}: a frame every hour");
            } else {
                assert_eq!(
                    frames, observed,
                    "{label}: frames only at the hours that change the groups"
                );
                assert!(frames < hours(&replay), "{label}: {frames} frames");
            }
            kept.push(observed);
            clear(source);
        }
        assert_eq!(kept[0], kept[1], "both reads keep the same hours");
    });
}

/// Three countries in faction 5, named by the dictionary's template 77, at
/// `hour`: the faction's colour moves at 102 and 103, and nothing else the
/// groups read changes.
fn dictionary_frame(hour: i32) -> DecodedFrame {
    let mut observed = frame(1, hour, 3);
    *observed.tables[TableId::HumanControllers.index()] =
        TableView::Complete(TableRows::HumanControllers(vec![HumanControllerRow {
            country: A,
            count: 1,
        }]));
    *observed.tables[TableId::Factions.index()] =
        TableView::Complete(TableRows::Factions(vec![FactionRow {
            id: 5,
            name: Vec::new(),
            template_token: Some(77),
            color_red: hour.clamp(101, 103) as u32,
            ..Default::default()
        }]));
    *observed.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(
            [A, B, C]
                .into_iter()
                .enumerate()
                .map(|(ordinal, country)| FactionMemberRow {
                    country,
                    faction: 5,
                    ordinal: ordinal as u32,
                })
                .collect(),
        ));
    *observed.tables[TableId::WarRelations.index()] =
        TableView::Complete(TableRows::WarRelations(Vec::new()));
    *observed.tables[TableId::Capitulation.index()] = TableView::Complete(TableRows::Capitulation(
        [A, B, C]
            .into_iter()
            .map(|country| CapitulationRow {
                country,
                ..Default::default()
            })
            .collect(),
    ));
    *observed.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(dictionary()));
    observed
}

fn dictionary() -> Vec<LexicalTokenRow> {
    vec![LexicalTokenRow {
        token: 77,
        literal: b"TEMPLATE_PACT".to_vec(),
    }]
}

/// The dictionary's equal snapshot at 101, recorded without a read time,
/// is a quiet hour its extract observes on a frame built for it, with the
/// dictionary at a new allocation as hourly preparation's there. The faction
/// colour changes at 102 and 103 are read with the dictionary unchanged: the
/// extract keeps the allocation built for it at both, as hourly preparation
/// keeps its own, so neither hour reads the dictionary again and the groups
/// publish and save what hourly preparation does.
#[test]
fn country_groups_read_by_their_changes_keep_a_built_dictionary_until_it_rebuilds() {
    let mut recorded = records(1, (100..=104).map(dictionary_frame).collect());
    let replay::spool::Record::Frame(snapshot) = &mut recorded[3] else {
        panic!("the second record after the branch start is a frame");
    };
    assert_eq!(snapshot.hour, 101);
    assert!(matches!(
        snapshot.tables[TableId::LexicalTokens.index()],
        replay_frame::frame::TableUpdate::Unchanged { .. }
    ));
    assert_eq!(snapshot.metrics.table_ns[TableId::LexicalTokens.index()], 0);
    snapshot.tables[TableId::LexicalTokens.index()] =
        replay_frame::frame::TableUpdate::Snapshot(TableRows::LexicalTokens(dictionary()));
    let replay = Replay::from_records(CAMPAIGN.into(), &[recorded]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let hourly = independent::with_test_hourly(|| {
        let source = fixture.source("groups-dictionary");
        run(&source, None, &AtomicBool::new(false), |_| {}).unwrap();
        let payloads = payloads(&source, None);
        clear(source);
        payloads
    });
    let source = fixture.source("groups-dictionary");
    let before = (
        prepass::noted("groups.sequential"),
        prepass::noted("groups.fallback"),
    );
    independent::with_test_only(&["groups"], || {
        prepass::with_test_prepass(2, None, || {
            run(&source, None, &AtomicBool::new(false), |_| {})
        })
    })
    .unwrap();
    assert_eq!(
        (
            prepass::noted("groups.sequential") - before.0,
            prepass::noted("groups.fallback") - before.1
        ),
        (0, 0),
        "the block folded from its extract"
    );
    assert_same(&payloads(&source, None), &hourly, "read by changes", true);
    clear(source);
}

#[test]
fn country_groups_cold_single_hour_query_keeps_the_sampled_name_read() {
    let replay =
        Replay::from_records(CAMPAIGN.into(), &[records(1, vec![groups_frame(1, 100)])]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("groups-single-sample");
    let data = query(&source, "country-groups", "1").unwrap();
    assert_eq!(data["coverage"]["lastHour"], 99);
    assert_eq!(data["factions"][0]["name"], "Axis");
    assert_eq!(data["factions"][0]["names"][0]["firstHour"], 100);
    assert_eq!(data["factions"][0]["names"][0]["lastHour"], 100);
    assert_eq!(
        data["approximate"],
        serde_json::json!([{"firstHour":100,"lastHour":100}])
    );
}

#[test]
fn cold_country_groups_merge_explicit_gaps_with_unsaved_block_discontinuities() {
    let replay = scenario(3, false);
    let mut continuation = continuation_records(
        &replay,
        1,
        (114..=117).map(|hour| groups_frame(1, hour)).collect(),
    );
    // The explicit witness covers only the first of two hours with no block.
    continuation.push(replay::spool::Record::Gap(replay_frame::branches::Gap {
        branch: 1,
        first_hour: 112,
        last_hour: 112,
        reason: "synthetic incomplete gap witness".into(),
    }));
    let replay = replay.append(&[continuation]).unwrap();
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("groups-cold-gaps");
    let data = query(&source, "country-groups", "1").unwrap();
    assert_eq!(data["coverage"]["lastHour"], 99);
    for list in ["humans", "factions", "wars"] {
        assert_eq!(
            data["unobserved"][list],
            serde_json::json!([{"firstHour": 112, "lastHour": 113}]),
            "{list}: every unrecorded hour is unobserved"
        );
    }
    assert_eq!(
        data["approximate"],
        serde_json::json!([
            {"firstHour": 100, "lastHour": 111},
            {"firstHour": 114, "lastHour": 117}
        ])
    );
    assert_eq!(data["pending"], serde_json::json!([]));
    clear(source);
}

#[test]
fn prepared_country_groups_query_refuses_missing_and_empty_name_histories() {
    use crate::products::country_groups::Scan;
    use serde_json::json;

    independent::with_test_hourly(|| {
        for empty_entry in [false, true] {
            let frame = groups_frame(1, 100);
            let replay =
                Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]).unwrap();
            let mut scan = Scan::new(&replay, 1);
            scan.observe(&frame).unwrap();
            scan.finish(&replay, 1);
            let (mut state, dictionaries) = Scan::saved_parts(&scan.saved_bytes().unwrap()).unwrap();
            // Keep the recorded roster and literal, but remove its supplying read.
            state["continuation"]["names"] = if empty_entry {
                json!({"5":[]})
            } else {
                json!({})
            };
            let saved = Scan::saved_from_parts(&state, &dictionaries).unwrap();
            let mut fixture = Fixture::new(&replay);
            let source = fixture.source("groups-missing-name");
            let (store, key) = super::super::super::files(&source, None).unwrap();
            let mut writer = store.replace(key).unwrap();
            writer
                .write_at("country-groups-1", 0, saved.as_slice())
                .unwrap();
            super::super::json(
                &mut writer,
                &coverage_section(1),
                &Coverage {
                    schema: SCHEMA,
                    first_hour: 100,
                    last_hour: 100,
                    complete: true,
                },
            )
            .unwrap();
            writer.commit().unwrap();

            let data = query(&source, "country-groups", "1").unwrap();
            assert!(data["factions"][0]["name"].is_null());
            assert_eq!(
                data["factions"][0]["nameReason"],
                "Faction has no recorded name read"
            );
            assert_eq!(data["factions"][0]["names"], json!([]));
            assert_eq!(data["factions"][0]["recordedName"], "Axis");
            assert!(
                !data["factions"][0]["members"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(data["warnings"].as_array().unwrap().iter().any(|warning|
                warning == "1 faction names are not shown: Faction has no recorded name read; faction 5"
            ));
        }
    });
}

/// Before groups preparation covers the branch, the country groups answer
/// their prepared prefix instead of null, carried on by each later block's
/// keyframe: those hours are approximate, the hours after the prefix are
/// neither unobserved nor pending once sampled (the recorded gap stays
/// unobserved), faction ids keep their lanes across samples, and the dated
/// event lists stay the prefix's. Once complete it is hourly preparation's
/// reply, with nothing approximate.
#[test]
fn country_groups_answer_their_prepared_prefix_carried_on_by_later_keyframes() {
    let replay = scenario(6, false);
    let mut fixture = Fixture::new(&replay);
    independent::with_test_hourly(|| {
        use serde_json::json;
        let source = fixture.source("groups-prefix");
        let flag = AtomicBool::new(false);
        let cold = query(&source, "country-groups", "1").unwrap();
        assert_eq!(
            cold["coverage"]["lastHour"].as_i64().unwrap() + 1,
            cold["coverage"]["firstHour"].as_i64().unwrap()
        );
        assert!(cold["pending"].is_array());
        while coverage(&source, None, 1, &flag)
            .unwrap()
            .is_none_or(|coverage| coverage.last_hour < 103)
        {
            run_measured(&source, None, &flag, true, Duration::ZERO, |_| {}).unwrap();
        }
        let accepted = coverage(&source, None, 1, &flag).unwrap().unwrap();
        assert!(!accepted.complete && accepted.last_hour < 108);
        let first = accepted.last_hour;
        let prefix = query(&source, "country-groups", "1").unwrap();
        assert_eq!(prefix["coverage"], serde_json::to_value(&accepted).unwrap());
        assert_eq!(
            prefix["eventsApproximate"],
            json!([{"firstHour": accepted.first_hour, "lastHour": accepted.last_hour}])
        );
        assert_eq!(
            prefix["approximate"],
            json!([{"firstHour": first + 1, "lastHour": 111}, {"firstHour": 114, "lastHour": 125}])
        );
        assert_eq!(prefix["pending"], json!([]), "every later block is sampled");
        for list in ["humans", "factions", "wars"] {
            assert_eq!(
                prefix["unobserved"][list],
                json!([{"firstHour": 112, "lastHour": 113}]),
                "{list}: only the recorded gap is unobserved"
            );
        }
        run(&source, None, &flag, |_| {}).unwrap();
        let complete = query(&source, "country-groups", "1").unwrap();
        assert!(complete.get("approximate").is_none() && complete.get("pending").is_none());
        let ids = |reply: &serde_json::Value| -> Vec<serde_json::Value> {
            let mut ids: Vec<_> = reply["factions"]
                .as_array()
                .unwrap()
                .iter()
                .map(|faction| faction["id"].clone())
                .collect();
            ids.sort_by_key(|id| id.to_string());
            ids
        };
        assert_eq!(
            ids(&prefix),
            ids(&complete),
            "recorded faction ids keep their lanes"
        );
        // Members first seen inside the prefix start there exactly.
        for faction in complete["factions"].as_array().unwrap() {
            let sampled = prefix["factions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|other| other["id"] == faction["id"])
                .unwrap();
            for run in faction["members"].as_array().unwrap() {
                if run["firstHour"].as_i64().unwrap() <= i64::from(first) {
                    assert!(
                        sampled["members"].as_array().unwrap().iter().any(|other| {
                            other["tag"] == run["tag"] && other["firstHour"] == run["firstHour"]
                        }),
                        "{run} in {sampled}"
                    );
                }
            }
        }
        let through = |list: &str| -> Vec<serde_json::Value> {
            complete[list]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["hour"].as_i64().unwrap() <= i64::from(first))
                .cloned()
                .collect()
        };
        for list in ["warStarts", "capitulations", "identityChanges"] {
            assert_eq!(prefix[list].as_array().unwrap(), &through(list), "{list}");
        }
        assert!(!complete["capitulations"].as_array().unwrap().is_empty());
        clear(source);
    });
}
