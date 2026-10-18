//! A branch's newest land and naval battle windows answer before forward
//! preparation has accepted any block, as the recording products answer
//! them, and forward preparation later answers them the same.
use super::*;
use crate::products::{
    LoadContext, Product, land_battles,
    land_battles::{
        LandBattles, LandBattlesManifest, LandBattlesManifestParams, LandBattlesParams,
    },
    naval_battles,
    naval_battles::{
        NavalBattles, NavalBattlesManifest, NavalBattlesManifestParams, NavalBattlesParams,
    },
};
use replay::rows::{
    LandCombatRow, LandCombatSideRow, NavalCombatRow, NavalResultRow, NavalResultSourceRow,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

/// An hour of `branch`: combat 1 is open from hour 11 in branch 1, combat 2
/// from hour 21 in branch 2 (of [`gapped`]).
fn land_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let open: Vec<(u64, i32)> = match branch {
        1 if hour >= 11 => vec![(1, 11)],
        2 if hour >= 21 => vec![(2, 21)],
        _ => Vec::new(),
    };
    *observed.tables[TableId::LandCombats.index()] = TableView::Complete(TableRows::LandCombats(
        open.iter()
            .map(|&(id, zero)| LandCombatRow {
                id,
                province: id as u32,
                day: 0,
                duration: hour - zero,
                attackers: vec![id << 32 | 1],
                defenders: vec![id << 32 | 2],
                ..Default::default()
            })
            .collect(),
    ));
    *observed.tables[TableId::LandCombatSides.index()] =
        TableView::Complete(TableRows::LandCombatSides(
            open.iter()
                .map(|&(id, _)| LandCombatSideRow {
                    combat: id,
                    ..Default::default()
                })
                .collect(),
        ));
    observed
}

fn query(source: &Source, flag: &AtomicBool, product: &str, params: Value) -> Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    let mut data = serde_json::from_str::<Value>(
        &crate::prepared::query::query(source, &request, None, flag).unwrap(),
    )
    .unwrap()["data"]
        .clone();
    // Compare observed facts with the recording oracle, independently of the
    // prefix metadata tested by cold_defaults. Only a proof's diagnostics
    // marker identifies the diagnostics publication this suite waits for.
    if let Some(fields) = data.as_object_mut() {
        if fields.contains_key("coverage") && !fields.contains_key("exact") {
            fields.remove("warningsComplete");
        }
        for field in ["coverage", "pending", "approximate", "exact"] {
            fields.remove(field);
        }
    }
    data
}

#[test]
fn newest_land_window_answers_while_forward_preparation_has_accepted_nothing() {
    let replay = gapped(land_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-land");
    let flag = AtomicBool::new(false);
    // Forward preparation stalls: it is cancelled at its first commit.
    let error = independent::with_test_newest_first(|| {
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    flag.store(false, Ordering::Release);
    assert_eq!(
        unprepared_blocks(&source, None).unwrap(),
        Some(replay.index.len()),
        "forward preparation accepted no block"
    );
    let context = LoadContext { content: None };
    let mut answered = Vec::new();
    for (branch, last) in [("1", 3), ("2", 1)] {
        let manifest = query(
            &source,
            &flag,
            "land-battles-manifest",
            json!({ "branchId": branch }),
        );
        assert_eq!(
            manifest,
            serde_json::to_value(
                LandBattlesManifest::load(
                    &source,
                    &LandBattlesManifestParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap(),
            "branch {branch}: the manifest answers from the newest window"
        );
        let params = LandBattlesParams {
            branch_id: branch.into(),
            window: last,
        };
        let window = query(
            &source,
            &flag,
            "land-battles",
            serde_json::to_value(&params).unwrap(),
        );
        assert_eq!(
            window,
            serde_json::to_value(LandBattles::load(&source, &params, &context, &flag).unwrap())
                .unwrap(),
            "branch {branch}: the newest window answers as the recording product"
        );
        answered.push((branch, params, manifest, window));
    }
    assert!(
        query(
            &source,
            &flag,
            "land-battles",
            json!({ "branchId": "1", "window": 0 })
        )
        .is_object(),
        "an earlier window answers its pending prefix"
    );
    run(&source, None, &flag, |_| {}).unwrap();
    for (branch, params, manifest, window) in answered {
        assert_eq!(
            query(
                &source,
                &flag,
                "land-battles-manifest",
                json!({ "branchId": branch })
            ),
            manifest
        );
        assert_eq!(
            query(
                &source,
                &flag,
                "land-battles",
                serde_json::to_value(&params).unwrap()
            ),
            window,
            "branch {branch}: forward preparation answers the newest window the same"
        );
    }
}

/// An hour of `branch` with every naval census table complete: combat 1 is
/// open from hour 11 through 13 and its source at hour 14 creates result 5,
/// retained from then on in branch 1, and from hour 20 in branch 2 (of
/// [`gapped`]).
fn naval_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    let mut set = |table: TableId, rows: TableRows| {
        *observed.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        TableId::NavalCombats,
        TableRows::NavalCombats(if (11..=13).contains(&hour) {
            vec![NavalCombatRow {
                id: 1,
                province: 1,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        TableId::NavalResults,
        TableRows::NavalResults(if hour >= if branch == 1 { 14 } else { 20 } {
            vec![NavalResultRow {
                id: 5,
                province: 1,
                result_hour: 14,
                winning_side: 1,
                losing_side: 2,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        TableId::NavalResultSources,
        TableRows::NavalResultSources(if hour == 14 {
            vec![NavalResultSourceRow {
                id: 1,
                game_hour: 14,
                result: 5,
                combat: 1,
                ..Default::default()
            }]
        } else {
            Vec::new()
        }),
    );
    set(
        TableId::NavalCombatParticipants,
        TableRows::NavalCombatParticipants(Vec::new()),
    );
    set(
        TableId::NavalCombatDetails,
        TableRows::NavalCombatDetails(Vec::new()),
    );
    set(
        TableId::NavalResultMembers,
        TableRows::NavalResultMembers(Vec::new()),
    );
    set(
        TableId::NavalResultCountries,
        TableRows::NavalResultCountries(Vec::new()),
    );
    set(
        TableId::NavalCombatResultDetails,
        TableRows::NavalCombatResultDetails(Vec::new()),
    );
    observed
}

/// The naval manifest and newest window of each branch answer from the
/// newest window while forward preparation has accepted nothing, equal to
/// the recording products, and forward preparation later answers them the
/// same.
fn naval_answers_first(name: &str) {
    let replay = gapped(naval_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source(name);
    let flag = AtomicBool::new(false);
    // Forward preparation stalls: it is cancelled at its first commit.
    let error = independent::with_test_newest_first(|| {
        run_checked(
            &source,
            None,
            &flag,
            |_| {},
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    flag.store(false, Ordering::Release);
    assert_eq!(
        unprepared_blocks(&source, None).unwrap(),
        Some(replay.index.len()),
        "forward preparation accepted no block"
    );
    let context = LoadContext { content: None };
    let mut answered = Vec::new();
    for (branch, last) in [("1", 3), ("2", 1)] {
        let manifest = query(
            &source,
            &flag,
            "naval-battles-manifest",
            json!({ "branchId": branch }),
        );
        assert_eq!(
            manifest,
            serde_json::to_value(
                NavalBattlesManifest::load(
                    &source,
                    &NavalBattlesManifestParams {
                        branch_id: branch.into()
                    },
                    &context,
                    &flag
                )
                .unwrap()
            )
            .unwrap(),
            "branch {branch}: the manifest answers from the newest window"
        );
        let params = NavalBattlesParams {
            branch_id: branch.into(),
            window: last,
        };
        let window = query(
            &source,
            &flag,
            "naval-battles",
            serde_json::to_value(&params).unwrap(),
        );
        assert_eq!(
            window,
            serde_json::to_value(NavalBattles::load(&source, &params, &context, &flag).unwrap())
                .unwrap(),
            "branch {branch}: the newest window answers as the recording product"
        );
        answered.push((branch, params, manifest, window));
    }
    assert!(
        answered[0].3["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == "result:5"),
        "the newest window holds the retained result"
    );
    assert!(
        query(
            &source,
            &flag,
            "naval-battles",
            json!({ "branchId": "1", "window": 0 })
        )
        .is_object(),
        "an earlier window answers its pending prefix"
    );
    run(&source, None, &flag, |_| {}).unwrap();
    for (branch, params, manifest, window) in answered {
        assert_eq!(
            query(
                &source,
                &flag,
                "naval-battles-manifest",
                json!({ "branchId": branch })
            ),
            manifest
        );
        assert_eq!(
            query(
                &source,
                &flag,
                "naval-battles",
                serde_json::to_value(&params).unwrap()
            ),
            window,
            "branch {branch}: forward preparation answers the newest window the same"
        );
    }
}

#[test]
fn newest_naval_window_answers_while_forward_preparation_has_accepted_nothing() {
    // The naval family prepared on its own, and in the hourly walk.
    naval_answers_first("newest-naval");
    independent::with_test_hourly(|| naval_answers_first("newest-naval-hourly"));
}

/// An hour of `branch` with both the land battles of `land_frame` and the
/// naval battles of `naval_frame`.
fn battles_frame(branch: u64, hour: i32) -> DecodedFrame {
    let mut observed = naval_frame(branch, hour);
    let land = land_frame(branch, hour);
    for table in [TableId::LandCombats, TableId::LandCombatSides] {
        observed.tables[table.index()] = land.tables[table.index()].clone();
    }
    observed
}

/// The newest windows' pages after a preparation cancelled at its first
/// commit, the newest windows' own.
fn newest_pages(source: &Source) -> std::collections::BTreeMap<(String, u64), Vec<u8>> {
    let flag = AtomicBool::new(false);
    let error = independent::with_test_newest_first(|| {
        run_checked(
            source,
            None,
            &flag,
            |_| {},
            |_| {
                flag.store(true, Ordering::Release);
                Ok(())
            },
        )
    })
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    super::family_lanes::payloads(source, None)
        .into_iter()
        .filter(|((section, _), _)| section.contains("newest"))
        .collect()
}

/// The newest land and naval windows, prepared at once, publish in their
/// commit exactly the pages of preparing them one family after the other.
#[test]
fn newest_land_and_naval_windows_prepared_at_once_publish_what_in_turn_does() {
    let replay = gapped(battles_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-together");
    let in_turn = super::super::newest::with_test_in_turn(|| newest_pages(&source));
    super::super::super::files(&source, None)
        .unwrap()
        .0
        .clear()
        .unwrap();
    let together = newest_pages(&source);
    let sections: Vec<_> = together
        .keys()
        .map(|(section, _)| section.as_str())
        .collect();
    for section in [
        "land-battles-newest-1",
        "land-battles-newest-2",
        "naval-battles-newest-1",
        "naval-battles-newest-2",
    ] {
        assert!(
            sections.contains(&section),
            "{section} published: {sections:?}"
        );
    }
    super::family_lanes::assert_payloads_equal(&together, &in_turn, "newest windows at once");
}

/// The newest land window answers from the commit that proves it, before its
/// diagnostics (every earlier block's naming tables) are read: its reply then
/// says its warnings are not complete. Once forward preparation covers the
/// branch, the page's rows are forward preparation's.
#[test]
fn newest_land_window_answers_from_its_proof_before_its_diagnostics() {
    let replay = gapped(land_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-land-proof");
    let flag = AtomicBool::new(false);
    let params = json!({ "branchId": "1", "window": 3 });
    let mut proven = None;
    let observed = Arc::new((Mutex::new(false), Condvar::new()));
    let signal = Arc::clone(&observed);
    let waited = Arc::new(Mutex::new(false));
    let outcome = Arc::clone(&waited);
    super::super::newest::with_test_stall(
        move |family, branch| {
            if family != "newest.land.diagnostics" || branch != 1 {
                return;
            }
            let (done, changed) = &*signal;
            let (guard, _) = changed
                .wait_timeout_while(done.lock().unwrap(), Duration::from_secs(30), |done| !*done)
                .unwrap();
            *outcome.lock().unwrap() = *guard;
        },
        || {
            run(&source, None, &flag, |_| {
                // Family and naval commits can precede this branch's proof.
                if proven.is_none() {
                    let reply = query(
                        &source,
                        &AtomicBool::new(false),
                        "land-battles",
                        params.clone(),
                    );
                    if reply["warningsComplete"] == false {
                        proven = Some(reply);
                        let (done, changed) = &*observed;
                        *done.lock().unwrap() = true;
                        changed.notify_all();
                    }
                }
            })
        },
    )
    .unwrap();
    assert!(
        *waited.lock().unwrap(),
        "the proof answered before the diagnostics started"
    );
    let proven = proven.unwrap();
    assert_eq!(proven["warningsComplete"], false, "{proven}");
    let complete = query(&source, &flag, "land-battles", params);
    assert!(complete.get("warningsComplete").is_none());
    let mut page = proven.clone();
    page.as_object_mut().unwrap().remove("warningsComplete");
    page["contentWarnings"] = complete["contentWarnings"].clone();
    assert_eq!(page, complete);
}

/// Each branch's blocks of `frame`, the first behind a recorded gap, so its
/// newest windows' look-backs start after the gap, short of the branch's
/// first block: branch 1's hours 6 and 7, then 10 to 15 in three blocks;
/// branch 2's hours 16 and 17, then 20 and 21.
fn gapped(frame: fn(u64, i32) -> DecodedFrame) -> Replay {
    let blocks: [(u64, [i32; 2]); 6] = [
        (1, [6, 7]),
        (1, [10, 11]),
        (1, [12, 13]),
        (1, [14, 15]),
        (2, [16, 17]),
        (2, [20, 21]),
    ];
    let mut replay: Option<Replay> = None;
    for (branch, hours) in blocks {
        let frames = hours.iter().map(|&hour| frame(branch, hour)).collect();
        replay = Some(match replay {
            None => Replay::from_records(CAMPAIGN.into(), &[records(branch, frames)]).unwrap(),
            Some(replay) if replay.branches.iter().any(|start| start.id == branch) => replay
                .append(&[continuation_records(&replay, branch, frames)])
                .unwrap(),
            Some(replay) => replay.append(&[records(branch, frames)]).unwrap(),
        });
    }
    let replay = replay.unwrap();
    assert_eq!(replay.index.len(), 6, "one replay block per fixture block");
    replay
}

/// Waits up to `limit` for `ready`, polling.
fn wait_for(limit: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < limit {
        if ready() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    ready()
}

/// The families prepared on their own commit while a newest land window is
/// still being prepared: their first commits are not ordered after the
/// newest windows. The land look-back waits for a family's commit before it
/// starts; preparing the newest windows first, it would wait in vain.
#[test]
fn families_commit_while_a_newest_land_window_prepares() {
    let replay = gapped(battles_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-stalled-land");
    let flag = AtomicBool::new(false);
    let committed = Arc::new((Mutex::new(false), Condvar::new()));
    let waited = Arc::new(Mutex::new(Vec::new()));
    let (signal, outcomes) = (Arc::clone(&committed), Arc::clone(&waited));
    super::super::newest::with_test_stall(
        move |family, branch| {
            if family != "newest.land" {
                return;
            }
            let (done, changed) = &*signal;
            let guard = done.lock().unwrap();
            let (guard, _) = changed
                .wait_timeout_while(guard, Duration::from_secs(30), |done| !*done)
                .unwrap();
            outcomes.lock().unwrap().push((branch, *guard));
        },
        || {
            run(&source, None, &flag, |accepted| {
                // Only a family's commit counts blocks before the walk's.
                if accepted.sections.iter().any(|section| section.done != "0") {
                    let (done, changed) = &*committed;
                    *done.lock().unwrap() = true;
                    changed.notify_all();
                }
            })
        },
    )
    .unwrap();
    assert_eq!(
        *waited.lock().unwrap(),
        vec![(1, true), (2, true)],
        "a family committed while each newest land window waited"
    );
    let (store, key) = super::super::super::files(&source, None).unwrap();
    for branch in [1, 2] {
        assert!(
            land_battles::newest::published(&store, &key, &replay, branch).unwrap(),
            "branch {branch}: the land window is still published"
        );
    }
}

/// Each family's newest windows are published in commits of their own as
/// soon as they are proven: the naval windows are published while the land
/// look-back waits, and the land windows after it.
#[test]
fn each_familys_newest_windows_publish_on_their_own() {
    let replay = Arc::new(gapped(battles_frame));
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-each-on-its-own");
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let flag = AtomicBool::new(false);
    let cold_gates: Vec<_> = ["land-battles-manifest", "naval-battles-manifest"]
        .into_iter()
        .map(|product| {
            let request = crate::transport::ProductRequest {
                source_id: source.key().source_id.clone(),
                revision: source.key().revision.clone(),
                product: product.into(),
                params: json!({"branchId":"1"}),
            };
            serde_json::from_str::<Value>(
                &crate::prepared::query::query(&source, &request, None, &flag).unwrap(),
            )
            .unwrap()["gate"]
                .clone()
        })
        .collect();
    let mut moved = BTreeSet::new();

    let seen = Arc::new(Mutex::new(Vec::new()));
    let (outcomes, reading, read) = (Arc::clone(&seen), Arc::clone(&store), key.clone());
    let recorded = Arc::clone(&replay);
    super::super::newest::with_test_stall(
        move |family, branch| {
            if family != "newest.land" {
                return;
            }
            let naval = wait_for(Duration::from_secs(30), || {
                [1, 2].into_iter().all(|branch| {
                    naval_battles::newest::published(&reading, &read, &recorded, branch).unwrap()
                })
            });
            let land = land_battles::newest::published(&reading, &read, &recorded, branch).unwrap();
            outcomes.lock().unwrap().push((branch, naval, land));
        },
        || {
            run(&source, None, &flag, |accepted| {
                if !accepted.coverage.is_empty() {
                    return;
                }
                for (index, family) in ["newest.land", "newest.naval"].into_iter().enumerate() {
                    if accepted
                        .families
                        .get(family)
                        .is_some_and(|branches| branches.contains_key("1"))
                    {
                        assert!(
                            cold_gates[index]
                                .as_array()
                                .unwrap()
                                .contains(&json!(family)),
                            "the cold reply follows the window's publication gate"
                        );
                        moved.insert(family);
                    }
                }
            })
        },
    )
    .unwrap();
    assert_eq!(
        moved,
        BTreeSet::from(["newest.land", "newest.naval"]),
        "newest commits advance mounted requests before the walk accepts an hour"
    );
    let restored = restore(&source, None, &flag)
        .unwrap()
        .expect("window gates survive checkpoint restore");
    for family in ["newest.land", "newest.naval"] {
        assert!(restored.families[family].contains_key("1"));
        assert!(restored.families[family].contains_key("2"));
    }
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(1, true, false), (2, true, false)],
        "every naval window was published before each land window was prepared"
    );
    for branch in [1, 2] {
        assert!(
            land_battles::newest::published(&store, &key, &replay, branch).unwrap(),
            "branch {branch}: the land window is published in its own commit"
        );
    }
}

/// What each branch's newest window did is counted in the family's timing
/// rows and reported in a status row of the family's own while preparation
/// runs: every branch proven, from the block after its gap.
#[test]
fn newest_windows_report_their_look_backs_and_outcomes() {
    let replay = gapped(battles_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-reported");
    let mut rows = BTreeMap::new();
    let (_, timings) = run_measured_through(
        &source,
        None,
        &AtomicBool::new(false),
        true,
        Duration::ZERO,
        Some(u64::MAX),
        |_| Ok(()),
        |accepted| {
            for section in &accepted.sections {
                if section.id.ends_with("-newest") {
                    rows.insert(section.id.clone(), section.clone());
                }
            }
        },
    )
    .unwrap();
    for id in ["land-battles-newest", "naval-battles-newest"] {
        let row = rows
            .get(id)
            .unwrap_or_else(|| panic!("{id} reported: {rows:?}"));
        assert_eq!(row.state, State::Ready, "{id}");
        assert_eq!(
            (row.done.as_str(), row.total.as_deref()),
            ("2", Some("2")),
            "{id}"
        );
        let text = row.error.as_deref().unwrap_or_default();
        for line in [
            "Branch 1: proven from block 1 of 4",
            "Branch 2: proven from block 1 of 2",
        ] {
            assert!(text.contains(line), "{id}: {text}");
        }
    }
    let families = timings.families.expect("measured timings");
    for (row, calls) in [("newest.land.proven", 2), ("newest.naval.proven", 2)] {
        assert_eq!(families.get(row).map(|row| row.calls), Some(calls), "{row}");
    }
    assert_eq!(
        families.get("newest.naval.earlier").map(|row| row.calls),
        Some(3 + 1),
        "each branch's blocks before its last, read by their changes"
    );
    for row in ["newest.land.look-back", "newest.naval.look-back"] {
        assert!(
            families.get(row).is_some_and(|row| row.calls > 0),
            "{row}: {:?}",
            families.keys().collect::<Vec<_>>()
        );
    }
}

/// A proven page is not the end of its side job: diagnostics still have to
/// publish even if every forward publisher finishes before cancellation.
#[test]
fn cancelled_newest_diagnostics_resume_after_forward_completion() {
    let replay = gapped(land_frame);
    let mut fixture = Fixture::new(&replay);
    let source = fixture.source("newest-diagnostics-outstanding");
    let flag = AtomicBool::new(false);
    let (store, key) = super::super::super::files(&source, None).unwrap();
    let finished = Arc::new((Mutex::new(false), Condvar::new()));
    let signal = Arc::clone(&finished);
    let waited = Arc::new(AtomicBool::new(false));
    let observed = Arc::clone(&waited);
    let result = super::super::newest::with_test_stall(
        move |family, branch| {
            if family != "newest.land.diagnostics" || branch != 1 {
                return;
            }
            let (done, changed) = &*signal;
            let (guard, _) = changed
                .wait_timeout_while(done.lock().unwrap(), Duration::from_secs(10), |done| !*done)
                .unwrap();
            observed.store(*guard, Ordering::Release);
        },
        || {
            run(&source, None, &flag, |_| {
                let Some((saved, _)) = checkpoint(&store, &key, None).unwrap() else {
                    return;
                };
                if saved.forward_complete(replay.index.len()) {
                    flag.store(true, Ordering::Release);
                    let (done, changed) = &*finished;
                    *done.lock().unwrap() = true;
                    changed.notify_all();
                }
            })
        },
    );
    assert_eq!(
        result.err().map(|error| error.kind()),
        Some(io::ErrorKind::Interrupted)
    );
    assert!(
        waited.load(Ordering::Acquire),
        "diagnostics stayed outstanding through forward completion"
    );
    let (saved, _) = checkpoint(&store, &key, None).unwrap().unwrap();
    assert!(saved.forward_complete(replay.index.len()));
    let proven = land_battles::newest::query(
        &source,
        "land-battles",
        1,
        &json!({"branchId":"1", "window":3}),
        &store,
        &key,
    )
    .unwrap()
    .expect("the proven page survives cancellation");
    assert_eq!(proven["warningsComplete"], false);
    assert!(!land_battles::newest::published(&store, &key, &replay, 1).unwrap());
    let diagnostics = "land-battle-newest-diagnostics-1";
    assert!(store.read(&key, diagnostics, 0).unwrap().is_none());
    flag.store(false, Ordering::Release);
    let reopened = fixture.source("newest-diagnostics-resumed");
    assert!(restore(&reopened, None, &flag).unwrap().is_none());
    let mut walked = Vec::new();
    run_checked(
        &reopened,
        None,
        &flag,
        |_| {},
        |next| {
            walked.push(next);
            Ok(())
        },
    )
    .unwrap();
    assert!(
        walked.is_empty(),
        "completed forward prefixes were retained"
    );
    assert!(
        store.read(&key, diagnostics, 0).unwrap().is_some(),
        "pending diagnostics were scheduled"
    );
    assert!(restore(&reopened, None, &flag).unwrap().is_some());
}
