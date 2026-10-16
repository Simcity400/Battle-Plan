use super::family_lanes::{assert_payloads_equal, payloads, serial_control};
use super::*;
use crate::products::{
    LoadContext, Product,
    military_air_ledger::{MilitaryAirFamily, MilitaryAirLedger, MilitaryAirParams},
};
use replay::rows::AirLossEventRow;
use std::{sync::Mutex, time::Duration};

const REFUSAL: &str = "military air event ordinal repeated or moved backwards";

/// A frame whose air loss events carry these native ordinals.
fn air_frame(branch: u64, hour: i32, ids: &[u64]) -> DecodedFrame {
    let mut observed = frame(branch, hour, 2);
    *observed.tables[TableId::AirLossEvents.index()] =
        TableView::Complete(TableRows::AirLossEvents(
            ids.iter()
                .map(|&id| AirLossEventRow {
                    id,
                    game_hour: hour,
                    country: 100_000,
                    region: 40001,
                    native_cause: 1,
                    family_base: 3,
                    amount_raw: 100_000,
                    ..Default::default()
                })
                .collect(),
        ));
    observed
}

/// Branch 1's second block restarts its air loss ordinals at hour 12, as the
/// recorded ledger did within one branch; branch 2 follows. The control keeps
/// every other recorded row and only continues the ordinals.
fn air_recording(restart: bool) -> Replay {
    let first = Replay::from_records(
        CAMPAIGN.into(),
        &[records(
            1,
            vec![air_frame(1, 10, &[0]), air_frame(1, 11, &[1])],
        )],
    )
    .unwrap();
    let (at_12, at_13) = if restart { (0, 1) } else { (2, 3) };
    let second = first
        .append(&[continuation_records(
            &first,
            1,
            vec![air_frame(1, 12, &[at_12]), air_frame(1, 13, &[at_13])],
        )])
        .unwrap();
    second
        .append(&[records(
            2,
            vec![air_frame(2, 20, &[0]), air_frame(2, 21, &[1])],
        )])
        .unwrap()
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

fn ledger_oracle(
    source: &Source,
    branch: &str,
    family: MilitaryAirFamily,
) -> io::Result<serde_json::Value> {
    MilitaryAirLedger::load(
        source,
        &MilitaryAirParams {
            branch_id: branch.into(),
            family,
            regions: Vec::new(),
            causes: Vec::new(),
            from_hour: None,
            top: None,
        },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .map(|reply| serde_json::to_value(reply).unwrap())
}

fn refused(source: &Source) -> BTreeMap<u64, Refused> {
    let (store, key) = super::super::super::files(source, None).unwrap();
    checkpoint(&store, &key, None).unwrap().unwrap().0.refused
}

fn refusal(hour: i32, error: &str, from: Option<&str>) -> Refusal {
    Refusal {
        hour,
        error: error.into(),
        from: from.map(str::to_owned),
    }
}

fn section<'a>(sections: &'a [Section], id: &str) -> &'a Section {
    sections.iter().find(|section| section.id == id).unwrap()
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn air_ledger_refusal_stops_only_its_branch_family_and_its_queries_answer_the_oracle_refusal() {
    // The air ledger is prepared on its own; its refusals here are the hourly
    // walk's, with every family in it.
    independent::with_test_hourly(|| {
        let mut fixture = Fixture::new(&air_recording(true));
        let source = fixture.source("air-refusal");
        let flag = AtomicBool::new(false);
        let mut progress = Vec::new();
        // The previous pipeline returned the ledger's InvalidData from `run`, so no
        // family reached hour 12 and nothing after the first batch was prepared.
        let accepted = run(&source, None, &flag, |accepted| {
            progress.push(accepted.sections.clone())
        })
        .unwrap();
        assert_eq!(accepted.next_block, 3);
        assert!(accepted.coverage["1"].complete && accepted.coverage["2"].complete);
        assert_eq!(
            refused(&source),
            BTreeMap::from([(
                1,
                Refused(BTreeMap::from([(
                    "air_ledger".to_owned(),
                    refusal(12, REFUSAL, None)
                )]))
            )])
        );

        // Progress and status name the family and its reason through the existing rows.
        let reason = format!("Branch 1 refused from hour 12: {REFUSAL}");
        let preparing = progress
            .iter()
            .rev()
            .find(|sections| section(sections, "country-measure").state == State::Preparing)
            .unwrap();
        for sections in [preparing, &accepted.sections] {
            let ledger = section(sections, "military-air-ledger");
            assert_eq!(ledger.state, State::Failed);
            assert_eq!(ledger.error.as_deref(), Some(reason.as_str()));
        }
        let mut control = Fixture::new(&air_recording(false));
        let control_source = control.source("air-refusal");
        let control_accepted = run(&control_source, None, &flag, |_| {}).unwrap();
        for (actual, expected) in accepted.sections.iter().zip(&control_accepted.sections) {
            if actual.id != "military-air-ledger" {
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        }

        // Every branch-1 ledger query answers with exactly the product route's refusal.
        for family in [MilitaryAirFamily::AirLosses, MilitaryAirFamily::Bombing] {
            let oracle = ledger_oracle(&source, "1", family).unwrap_err();
            assert_eq!(oracle.to_string(), REFUSAL);
            for params in [
                serde_json::json!({"branchId": "1", "family": family}),
                serde_json::json!({"branchId": "1", "family": family, "availability": true}),
            ] {
                let prepared = query(&source, "military-air-ledger", params).unwrap_err();
                assert_eq!(prepared.kind(), io::ErrorKind::InvalidData);
                assert_eq!(prepared.to_string(), oracle.to_string());
            }
            // The refusal is branch-scoped: branch 2 serves its oracle reply.
            assert_eq!(
                query(
                    &source,
                    "military-air-ledger",
                    serde_json::json!({"branchId": "2", "family": family}),
                )
                .unwrap(),
                ledger_oracle(&source, "2", family).unwrap()
            );
        }

        // Every other family's pages equal the control's, whose families match their
        // oracles; recording stamps, the map and the ledger's own branch differ.
        let differs = |name: &str| {
            [
                "military-air-ledger-1",
                "military-air-losses-1",
                "military-bombing-1",
                "map.",
                "source-metadata",
                "preparation-checkpoint",
                "preparation-coverage-1",
            ]
            .iter()
            .any(|prefix| name.starts_with(prefix))
        };
        let families = |source: &Source| {
            payloads(source, None)
                .into_iter()
                .filter(|((name, _), _)| !differs(name))
                .collect::<BTreeMap<_, _>>()
        };
        assert_payloads_equal(
            &families(&source),
            &families(&control_source),
            "families other than the refused ledger branch",
        );
        assert_prepared_identities(&source, 1);

        // The ledger never published a branch-1 page, and the accepted preparation
        // still restores and resumes without rebuilding.
        assert!(restore(&source, None, &flag).unwrap().is_some());
        let source = fixture.source("air-refusal-restart");
        assert_eq!(run(&source, None, &flag, |_| {}).unwrap().new_blocks, 0);
        assert_eq!(
            query(
                &source,
                "military-air-ledger",
                serde_json::json!({"branchId": "1", "family": "airLosses"}),
            )
            .unwrap_err()
            .to_string(),
            REFUSAL
        );
    });
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn refusals_in_a_lane_family_and_a_coordinator_family_give_byte_identical_serial_and_lane_stores() {
    // The air ledger is prepared on its own; its refusals here are the hourly
    // walk's, with every family in it.
    independent::with_test_hourly(|| {
        // A lane family refuses through its recorded data.
        let mut fixture = Fixture::new(&air_recording(true));
        let expected = serial_control(&mut fixture, "refusal-lanes", None);
        let parallel = fixture.source("refusal-lanes");
        lanes::with_test_lanes(4, None, || {
            run(&parallel, None, &AtomicBool::new(false), |_| {})
        })
        .unwrap();
        assert_payloads_equal(
            &payloads(&parallel, None),
            &expected,
            "lane family refusal: lanes vs serial",
        );
        assert!(refused(&parallel)[&1].0.contains_key("air_ledger"));

        // A coordinator family refuses, together with its dependent.
        lanes::with_test_refusals(&[("groups", 11)], || {
            let mut fixture = Fixture::new(&recording());
            let expected = serial_control(&mut fixture, "refusal-lanes", None);
            let parallel = fixture.source("refusal-lanes");
            lanes::with_test_lanes(4, None, || {
                run(&parallel, None, &AtomicBool::new(false), |_| {})
            })
            .unwrap();
            assert_payloads_equal(
                &payloads(&parallel, None),
                &expected,
                "coordinator family refusal: lanes vs serial",
            );
            let refused = refused(&parallel);
            assert!(refused[&1].0.contains_key("groups"));
            assert!(refused[&1].0.contains_key("air_superiority"));
        });
    });
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: runs in the full suite (--features slow-tests)"
)]
fn air_ledger_refusal_persists_across_commit_resume_restart_and_append_without_observing_again() {
    // The air ledger is prepared on its own; its refusals here are the hourly
    // walk's, with every family in it.
    independent::with_test_hourly(|| {
        let replay = Replay::from_records(
            CAMPAIGN.into(),
            &[records(
                1,
                vec![air_frame(1, 10, &[0]), air_frame(1, 11, &[1])],
            )],
        )
        .unwrap();
        let replay = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![air_frame(1, 12, &[0]), air_frame(1, 13, &[1])],
            )])
            .unwrap();
        let grown = replay
            .append(&[continuation_records(
                &replay,
                1,
                vec![air_frame(1, 14, &[2]), air_frame(1, 15, &[3])],
            )])
            .unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let trace = Arc::clone(&observed);
        let hook: lanes::Hook = Arc::new(move |family, frame, _| {
            if family == "air_ledger" {
                trace.lock().unwrap().push(frame.hour);
            }
            lanes::Fault::Continue
        });
        let flag = AtomicBool::new(false);
        let mut fixture = Fixture::new(&replay);
        for parallelism in [1, 4] {
            // Commit the first block, before the refusing hour.
            let source = fixture.source("refusal-resume");
            let (accepted, _) =
                lanes::with_test_lanes(parallelism, Some(Arc::clone(&hook)), || {
                    run_measured_through(
                        &source,
                        None,
                        &flag,
                        parallelism == 1,
                        Duration::ZERO,
                        Some(2),
                        |_| Ok(()),
                        |_| {},
                    )
                })
                .unwrap();
            assert_eq!(accepted.next_block, 1);
            assert!(refused(&source).is_empty());

            // Resume after a restart: the family refuses at 12 and is never observed again.
            let source = fixture.source("refusal-resume");
            lanes::with_test_lanes(parallelism, Some(Arc::clone(&hook)), || {
                run(&source, None, &flag, |_| {})
            })
            .unwrap();
            let expected = BTreeMap::from([(
                1,
                Refused(BTreeMap::from([(
                    "air_ledger".to_owned(),
                    refusal(12, REFUSAL, None),
                )])),
            )]);
            assert_eq!(refused(&source), expected);
            assert_eq!(*observed.lock().unwrap(), [10, 11, 12]);

            // Append: only the new block is prepared, and the refused family stays stopped.
            fixture.replace(&grown);
            let source = fixture.source("refusal-resume");
            let accepted = lanes::with_test_lanes(parallelism, Some(Arc::clone(&hook)), || {
                run(&source, None, &flag, |_| {})
            })
            .unwrap();
            assert_eq!(accepted.new_blocks, 1);
            assert_eq!(*observed.lock().unwrap(), [10, 11, 12]);
            assert_eq!(refused(&source), expected);
            assert_eq!(
                section(&accepted.sections, "military-air-ledger")
                    .error
                    .as_deref(),
                Some(format!("Branch 1 refused from hour 12: {REFUSAL}").as_str())
            );
            assert_eq!(
                query(
                    &source,
                    "military-air-ledger",
                    serde_json::json!({"branchId": "1", "family": "bombing"}),
                )
                .unwrap_err()
                .to_string(),
                REFUSAL
            );

            super::super::super::files(&source, None)
                .unwrap()
                .0
                .clear()
                .unwrap();
            drop(source);
            fixture.replace(&replay);
            observed.lock().unwrap().clear();
        }
    });
}

#[test]
fn a_family_error_other_than_invalid_data_still_stops_preparation() {
    // The air ledger is prepared on its own; its refusals here are the hourly
    // walk's, with every family in it.
    independent::with_test_hourly(|| {
        for parallelism in [1, 4] {
            let mut fixture = Fixture::new(&air_recording(false));
            let source = fixture.source("refusal-other-error");
            let hook: lanes::Hook = Arc::new(|family, frame, _| {
                if family == "air_ledger" && frame.hour == 12 {
                    lanes::Fault::Error
                } else {
                    lanes::Fault::Continue
                }
            });
            let error = lanes::with_test_lanes(parallelism, Some(hook), || {
                run(&source, None, &AtomicBool::new(false), |_| {})
            })
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other);
            let (store, key) = super::super::super::files(&source, None).unwrap();
            assert!(
                checkpoint(&store, &key, None)
                    .unwrap()
                    .is_none_or(|(state, _)| state.refused.is_empty())
            );
        }
    });
}

#[test]
fn a_groups_refusal_reaches_air_superiority_with_its_source_and_their_queries_refuse() {
    // The hourly walk's own read: groups prepared on their own reach air
    // superiority through its own read (independent_superiority).
    independent::with_test_hourly(|| {
        let mut fixture = Fixture::new(&recording());
        let source = fixture.source("refusal-dependent");
        let flag = AtomicBool::new(false);
        let accepted =
            lanes::with_test_refusals(&[("groups", 11)], || run(&source, None, &flag, |_| {}))
                .unwrap();
        let text = "Injected refusal of groups at hour 11";
        assert_eq!(
            refused(&source),
            BTreeMap::from([(
                1,
                Refused(BTreeMap::from([
                    ("groups".to_owned(), refusal(11, text, None)),
                    (
                        "air_superiority".to_owned(),
                        refusal(11, text, Some("groups"))
                    ),
                ]))
            )])
        );
        assert_eq!(
            section(&accepted.sections, "country-groups")
                .error
                .as_deref(),
            Some(format!("Branch 1 refused from hour 11: {text}").as_str())
        );
        let superiority = section(&accepted.sections, "air-superiority");
        assert_eq!(superiority.state, State::Failed);
        assert_eq!(
            superiority.error.as_deref(),
            Some(format!("Branch 1 refused from hour 11: country-groups refused: {text}").as_str())
        );
        for (product, params) in [
            ("country-groups", serde_json::json!({"branchId": "1"})),
            ("air-superiority", serde_json::json!({"branchId": "1"})),
            ("event-feed", serde_json::json!({"branchId": "1"})),
            (
                "series-summary",
                serde_json::json!({"branchId": "1", "family": {"kind": "faction-totals"}}),
            ),
        ] {
            let error = query(&source, product, params).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(error.to_string(), text);
        }
        // Other families, and the same families on another branch, are unaffected.
        for (branch, family) in [(1, "raids"), (1, "country-measure"), (2, "country-groups")] {
            super::super::refusal(&source, None, branch, family, &flag).unwrap();
        }
    })
}

#[test]
fn a_scoreboard_refuses_when_one_of_its_column_families_refused() {
    let mut fixture = Fixture::new(&recording());
    let source = fixture.source("refusal-composite");
    let flag = AtomicBool::new(false);
    lanes::with_test_refusals(&[("measure", 20)], || run(&source, None, &flag, |_| {})).unwrap();
    let text = "Injected refusal of measure at hour 20";
    assert_eq!(
        refused(&source),
        BTreeMap::from([(
            2,
            Refused(BTreeMap::from([(
                "measure".to_owned(),
                refusal(20, text, None)
            )]))
        )])
    );
    for (product, params) in [
        (
            "scoreboard",
            serde_json::json!({"selection": {"branchId": "2"}}),
        ),
        (
            "series-state",
            serde_json::json!({"branchId": "2", "family": {"kind": "country-measure"}}),
        ),
    ] {
        let error = query(&source, product, params).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), text);
    }
    for (branch, family) in [(1, "scoreboard"), (2, "faction-totals")] {
        super::super::refusal(&source, None, branch, family, &flag).unwrap();
    }
}
