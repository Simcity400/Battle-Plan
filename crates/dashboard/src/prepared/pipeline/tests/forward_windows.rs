use super::*;
use crate::products::{
    LoadContext, Product,
    land_battles::{
        LandBattles, LandBattlesManifest, LandBattlesManifestParams, LandBattlesParams,
    },
    naval_battles::{
        NavalBattles, NavalBattlesManifest, NavalBattlesManifestParams, NavalBattlesParams,
    },
};
use replay_data::rows::{
    LandCombatRow, LandCombatSideRow, NavalCombatRow, NavalResultRow, NavalResultSourceRow,
};
use serde_json::{Value, json};
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

fn envelope(source: &Source, flag: &AtomicBool, product: &str, params: Value) -> Value {
    let request = crate::transport::ProductRequest {
        source_id: source.key().source_id.clone(),
        revision: source.key().revision.clone(),
        product: product.into(),
        params,
    };
    serde_json::from_str(&crate::prepared::query::query(source, &request, None, flag).unwrap())
        .unwrap()
}

fn assert_pending_tail(
    source: &Source,
    flag: &AtomicBool,
    family: &str,
    branch: &str,
    window: usize,
) {
    for (product, params) in [
        (
            format!("{family}-battles-manifest"),
            json!({"branchId":branch}),
        ),
        (
            format!("{family}-battles"),
            json!({"branchId":branch,"window":window}),
        ),
    ] {
        let reply = envelope(source, flag, &product, params.clone());
        assert_eq!(reply["gate"], json!(["walk"]));
        let data = &reply["data"];
        assert!(!data["pending"].as_array().unwrap().is_empty(), "{reply}");
        assert!(data.get("exact").is_none());
        assert_eq!(
            crate::readiness::classify(&product, &params, data, false),
            crate::readiness::Outcome::Prefix
        );
        if params.get("window").is_some() {
            assert_eq!(data["rows"], json!([]));
            assert_eq!(data["observed"], json!([]));
            let last = source
                .block_windows(branch.parse().unwrap())
                .last()
                .unwrap()
                .last_hour;
            assert!(
                data["pending"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|span| span["lastHour"] == last)
            );
        }
    }
}

#[test]
fn battle_windows_wait_for_the_forward_frontier_and_resume_to_the_recording_oracle() {
    independent::with_test_hourly(|| {
        for (family, frame) in [
            ("land", land_frame as fn(u64, i32) -> DecodedFrame),
            ("naval", naval_frame),
        ] {
            let replay = gapped(frame);
            let mut fixture = Fixture::new(&replay);
            let source = fixture.source(&format!("forward-{family}"));
            let flag = AtomicBool::new(false);
            for (branch, last) in [("1", 3), ("2", 1)] {
                assert_pending_tail(&source, &flag, family, branch, last);
            }
            let error = run(&source, None, &flag, |accepted| {
                assert!(
                    accepted
                        .sections
                        .iter()
                        .all(|section| !section.id.ends_with("-newest"))
                );
                assert!(
                    accepted
                        .families
                        .keys()
                        .all(|name| !name.starts_with("newest."))
                );
                if accepted
                    .coverage
                    .get("1")
                    .is_some_and(|coverage| coverage.last_hour >= 7)
                {
                    flag.store(true, Ordering::Release);
                }
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Interrupted);
            flag.store(false, Ordering::Release);
            for (branch, last) in [("1", 3), ("2", 1)] {
                assert_pending_tail(&source, &flag, family, branch, last);
            }
            let prefix = envelope(
                &source,
                &flag,
                &format!("{family}-battles"),
                json!({"branchId":"1","window":0}),
            );
            assert_eq!(prefix["data"]["observed"], json!([[6, 7]]));
            assert_eq!(prefix["data"]["coverage"]["lastHour"], 7);
            run(&source, None, &flag, |_| {}).unwrap();
            let context = LoadContext { content: None };
            for (branch, last) in [("1", 3), ("2", 1)] {
                let manifest_params = json!({"branchId":branch});
                let window_params = json!({"branchId":branch,"window":last});
                let (manifest, window) = if family == "land" {
                    (
                        serde_json::to_value(
                            LandBattlesManifest::load(
                                &source,
                                &LandBattlesManifestParams {
                                    branch_id: branch.into(),
                                },
                                &context,
                                &flag,
                            )
                            .unwrap(),
                        )
                        .unwrap(),
                        serde_json::to_value(
                            LandBattles::load(
                                &source,
                                &LandBattlesParams {
                                    branch_id: branch.into(),
                                    window: last,
                                },
                                &context,
                                &flag,
                            )
                            .unwrap(),
                        )
                        .unwrap(),
                    )
                } else {
                    (
                        serde_json::to_value(
                            NavalBattlesManifest::load(
                                &source,
                                &NavalBattlesManifestParams {
                                    branch_id: branch.into(),
                                },
                                &context,
                                &flag,
                            )
                            .unwrap(),
                        )
                        .unwrap(),
                        serde_json::to_value(
                            NavalBattles::load(
                                &source,
                                &NavalBattlesParams {
                                    branch_id: branch.into(),
                                    window: last,
                                },
                                &context,
                                &flag,
                            )
                            .unwrap(),
                        )
                        .unwrap(),
                    )
                };
                for (product, params, expected) in [
                    (
                        format!("{family}-battles-manifest"),
                        manifest_params,
                        manifest,
                    ),
                    (format!("{family}-battles"), window_params, window),
                ] {
                    let reply = envelope(&source, &flag, &product, params);
                    assert_eq!(reply["data"], expected);
                    assert!(reply["gate"].is_null());
                    if branch == "1" && product == "naval-battles" {
                        assert!(
                            reply["data"]["rows"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|row| row["id"] == "result:5"),
                            "branch 1's completed last naval window holds the retained result"
                        );
                    }
                }
            }
            let pages = super::family_lanes::payloads(&source, None);
            assert!(pages.keys().all(|(section, _)| !section.contains("newest")));
        }
    });
}

#[test]
fn battle_reply_gates_only_name_the_forward_publishers() {
    for product in [
        "land-battles",
        "land-battles-manifest",
        "land-battles-ranking",
        "land-battle-course",
    ] {
        assert_eq!(gate(product), ["walk"]);
    }
    for product in [
        "naval-battles",
        "naval-battles-manifest",
        "naval-battles-ranking",
        "naval-battle-details",
    ] {
        assert_eq!(gate(product), ["naval"]);
        independent::with_test_hourly(|| assert_eq!(gate(product), ["walk"]));
    }
}
