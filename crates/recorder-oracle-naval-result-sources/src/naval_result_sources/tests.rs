use super::*;
use replay_capture_spool::spool::Record;
use replay_data::{
    identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
    rows::CampaignRow, rows::ClockRow, rows::NavalCombatRow, rows::NavalResultRow, rows::TableRows,
};
use replay_frame::{
    branches::BranchReason, branches::BranchStart, branches::Gap, branches::Parentage,
    frame::FrameEncoder, frame::TableView,
};

fn frame(
    hour: Hour,
    combats: &[u64],
    results: &[(u64, Hour)],
    events: Vec<NavalResultSourceRow>,
) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| {
            TableView::Complete(TableRows::empty(TableId::ALL[i])).into()
        }),
    };
    *frame.tables[TableId::NavalCombats.index()] = TableView::Complete(TableRows::NavalCombats(
        combats
            .iter()
            .map(|&id| NavalCombatRow {
                id,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::NavalResults.index()] = TableView::Complete(TableRows::NavalResults(
        results
            .iter()
            .map(|&(id, result_hour)| NavalResultRow {
                id,
                result_hour,
                ..Default::default()
            })
            .collect(),
    ));
    *frame.tables[TableId::NavalResultSources.index()] =
        TableView::Complete(TableRows::NavalResultSources(events));
    frame
}

fn event(id: u64, combat: u64, result: u64) -> NavalResultSourceRow {
    NavalResultSourceRow {
        id,
        game_hour: 101,
        combat,
        result,
        ..Default::default()
    }
}

fn merged_event(id: u64, result: u64) -> NavalResultSourceRow {
    NavalResultSourceRow {
        merged: true,
        transient: true,
        combat_created_hour: Some(101),
        ..event(id, 0, result)
    }
}

fn checked(frames: &[DecodedFrame]) -> Report {
    let mut check = SourceCheck::default();
    let mut report = Report::default();
    for frame in frames {
        check.observe(frame, &mut report);
    }
    check.finish(&mut report);
    report
}

fn has_failure(report: &Report, name: &str) {
    assert!(
        report
            .differences
            .iter()
            .any(|line| line.contains(&format!("NavalResultSources.{name}"))),
        "{:?}",
        report.differences
    );
}

fn has_unobserved_result(report: &Report, event: u64, result: u64) {
    assert!(report.limitations.iter().any(|(reason, subjects)| {
        reason.contains("native pre-read retirement")
            && subjects.iter().any(|subject| {
                subject.contains(&format!("result_unobserved event {event} result {result}"))
            })
    }));
}

fn replay_of(frames: &[DecodedFrame]) -> Replay {
    let mut records = vec![Record::SessionStart(SessionStart {
        id: 1,
        process_id: 1,
        process_start_time: 1,
        qpc_frequency: 1,
        sample_boundary: replay_data::identity::SampleBoundary::EndOfHour,
        capture_mode: replay_data::identity::CaptureMode::Full,
        identity: Identity {
            game_version: "synthetic".into(),
            exe_hash: [1; 32],
            exe_size: 1,
            vanilla_hash: [2; 32],
            dlc: Vec::new(),
            mods: Vec::new(),
            coverage_policy: 1,
            language: "l_english".into(),
            recorder_version: "test".into(),
            collector_hash: [3; 32],
            verification: Verification::Verified {
                launcher: Roster::default(),
                memory: Roster::default(),
            },
        },
    })];
    let mut branches = BTreeSet::new();
    let mut encoder = FrameEncoder::new();
    for source in frames {
        let mut frame = source.clone();
        for table in TableId::ALL {
            if !TABLES.contains(&table) {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "outside the naval source fixture".into(),
                };
            }
        }
        if branches.insert(frame.branch) {
            records.push(Record::BranchStart(BranchStart {
                id: frame.branch,
                session: 1,
                loaded_hour: Some(frame.hour - 1),
                first_hour: frame.hour,
                parent: Parentage::Unknown,
                reason: if branches.len() == 1 {
                    BranchReason::SessionStart
                } else {
                    BranchReason::LoadSignal
                },
            }));
        }
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![ClockRow {
                hour: frame.hour,
                ..Default::default()
            }]));
        *frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap()
}

#[test]
fn new_and_merged_sources_round_trip_across_the_branch() {
    let frames = [
        frame(100, &[11], &[(21, 100)], vec![]),
        frame(
            101,
            &[],
            &[(21, 101), (22, 101), (23, 101)],
            vec![
                event(1, 11, 23),
                NavalResultSourceRow {
                    merged: false,
                    ..merged_event(2, 22)
                },
                merged_event(3, 22),
                merged_event(4, 21),
            ],
        ),
        frame(102, &[], &[(21, 101), (22, 101), (23, 101)], vec![]),
    ];
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["NavalResultSources"], 4);
    let mut encoded = Report::default();
    compare(&replay_of(&frames), &frames[2], &mut encoded).unwrap();
    assert_eq!(encoded.compared_tables["NavalResultSources"], 4);
    assert!(encoded.differences.is_empty());
    assert!(
        encoded
            .limitations
            .keys()
            .any(|reason| reason.contains("no source-combat reference"))
    );
}

#[test]
fn explicit_between_read_sources_and_skipped_creation_pass() {
    let born = NavalResultSourceRow {
        combat_created_hour: Some(100),
        game_hour: 100,
        ..event(1, 11, 21)
    };
    let transient = NavalResultSourceRow {
        transient: true,
        combat_created_hour: Some(101),
        ..event(2, 0, 22)
    };
    let frames = [
        frame(100, &[], &[], vec![]),
        frame(101, &[], &[(21, 100), (22, 101)], vec![born, transient]),
        frame(102, &[], &[(21, 100), (22, 101)], vec![]),
    ];
    assert!(checked(&frames).differences.is_empty());
    assert_eq!(
        checked(&[frame(100, &[], &[], vec![]), frame(101, &[], &[], vec![])]).compared_tables["NavalResultSources"],
        0
    );
}

#[test]
fn every_provenance_failure_is_named() {
    let before = frame(100, &[11], &[], vec![]);
    for (row, name) in [
        (event(0, 11, 21), "event_identity"),
        (
            NavalResultSourceRow {
                game_hour: 99,
                ..event(1, 11, 21)
            },
            "event_hour",
        ),
        (event(1, 12, 21), "combat_missing"),
        (
            NavalResultSourceRow {
                combat_created_hour: Some(99),
                ..event(1, 12, 21)
            },
            "combat_missing",
        ),
        (
            NavalResultSourceRow {
                combat_created_hour: Some(102),
                ..event(1, 11, 21)
            },
            "event_hour",
        ),
        (
            NavalResultSourceRow {
                merged: true,
                ..event(1, 11, 21)
            },
            "merge_without_prior_result",
        ),
        (
            NavalResultSourceRow {
                merged: true,
                ..event(1, 11, 21)
            },
            "persistent_merge",
        ),
        (
            NavalResultSourceRow {
                transient: true,
                ..event(1, u64::MAX, 21)
            },
            "transient_source",
        ),
        (
            NavalResultSourceRow {
                transient: true,
                combat_created_hour: Some(101),
                ..event(1, 11, 21)
            },
            "transient_source",
        ),
    ] {
        has_failure(
            &checked(&[before.clone(), frame(101, &[], &[(21, 101)], vec![row])]),
            name,
        );
    }
    let absent = checked(&[before.clone(), frame(101, &[], &[], vec![event(1, 11, 21)])]);
    assert!(absent.differences.is_empty());
    has_unobserved_result(&absent, 1, 21);
    has_failure(
        &checked(&[
            frame(100, &[11], &[(21, 100)], vec![]),
            frame(101, &[], &[(21, 101)], vec![event(1, 11, 21)]),
        ]),
        "new_result_already_existed",
    );
    let absent_merge = checked(&[
        frame(100, &[], &[(21, 100)], vec![]),
        frame(101, &[], &[], vec![merged_event(1, 21)]),
    ]);
    assert!(absent_merge.differences.is_empty());
    has_unobserved_result(&absent_merge, 1, 21);
    has_failure(
        &checked(&[
            before,
            frame(
                101,
                &[],
                &[(21, 101), (22, 101)],
                vec![event(1, 11, 21), event(2, 11, 22)],
            ),
        ]),
        "duplicate_combat",
    );
}

#[test]
fn observed_result_transitions_require_exact_creation_and_merge_coverage() {
    let before = frame(100, &[11, 12], &[(21, 100)], vec![]);
    let report = checked(&[
        before.clone(),
        frame(101, &[], &[(21, 100), (22, 101)], vec![]),
    ]);
    has_failure(&report, "result_unsourced");
    assert_eq!(report.differences.len(), 1);
    has_failure(
        &checked(&[
            before.clone(),
            frame(
                101,
                &[],
                &[(21, 100), (22, 101)],
                vec![event(1, 11, 22), event(2, 12, 22)],
            ),
        ]),
        "result_unsourced",
    );
    let changed = frame(101, &[], &[(21, 101)], vec![]);
    let report = checked(&[before.clone(), changed]);
    has_failure(&report, "merge_unsourced");
    assert_eq!(report.differences.len(), 1);
    let merged = frame(101, &[], &[(21, 101)], vec![merged_event(1, 21)]);
    assert!(checked(&[before, merged]).differences.is_empty());
}

#[test]
fn unavailable_previous_results_suppress_dependent_transition_failures() {
    let mut before = frame(100, &[], &[(21, 100), (22, 100)], vec![]);
    *before.tables[TableId::NavalResults.index()] = TableView::Unavailable {
        reason: "synthetic result read failure".into(),
    };
    let created = NavalResultSourceRow {
        transient: true,
        combat_created_hour: Some(101),
        ..event(2, 0, 22)
    };
    let report = checked(&[
        before,
        frame(
            101,
            &[],
            &[(21, 101), (22, 101)],
            vec![merged_event(1, 21), created],
        ),
    ]);
    has_failure(&report, "input_unavailable");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn unavailable_previous_combats_suppress_dependent_membership_failures() {
    let mut before = frame(100, &[11, 12], &[], vec![]);
    *before.tables[TableId::NavalCombats.index()] = TableView::Unavailable {
        reason: "synthetic combat read failure".into(),
    };
    let frames = [
        before,
        frame(
            101,
            &[],
            &[(21, 101), (22, 101)],
            vec![event(1, 11, 21), event(2, 12, 22)],
        ),
    ];
    let report = checked(&frames);
    has_failure(&report, "input_unavailable");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

    let after = frame(
        102,
        &[],
        &[(21, 101), (22, 101), (23, 101)],
        vec![event(3, 13, 23)],
    );
    let report = checked(&[frames[0].clone(), frames[1].clone(), after]);
    has_failure(&report, "combat_missing");
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

#[test]
fn a_gap_suppresses_dependent_combat_failures_but_the_first_read_still_checks() {
    let mut row = event(1, 11, 21);
    row.game_hour = 102;
    let report = checked(&[
        frame(100, &[11], &[], vec![]),
        frame(102, &[], &[(21, 102)], vec![row]),
    ]);
    has_failure(&report, "gap");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

    let report = checked(&[frame(101, &[], &[(21, 101)], vec![event(1, 11, 21)])]);
    has_failure(&report, "combat_missing");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn later_result_read_resolves_pending_sources_after_an_unavailable_read() {
    let mut unavailable = frame(101, &[], &[], vec![event(1, 11, 21)]);
    *unavailable.tables[TableId::NavalResults.index()] = TableView::Unavailable {
        reason: "synthetic result read failure".into(),
    };
    let report = checked(&[
        frame(100, &[11], &[], vec![]),
        unavailable,
        frame(102, &[], &[(21, 101)], vec![]),
    ]);
    has_failure(&report, "input_unavailable");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
}

#[test]
fn declared_gaps_are_reported_once_and_unlabelled_missing_hours_remain_differences() {
    for (last_hour, expected) in [(102, 1), (104, 2)] {
        let frames = [
            frame(100, &[], &[(21, 100)], vec![]),
            frame(last_hour, &[], &[(21, 101), (22, 101)], vec![]),
        ];
        let mut replay = replay_of(&frames);
        replay.gaps = vec![Gap {
            branch: 1,
            first_hour: 101,
            last_hour: 101,
            reason: "synthetic lost hour".into(),
        }];
        let mut report = Report::default();
        compare(&replay, &frames[1], &mut report).unwrap();
        has_failure(&report, "gap");
        assert_eq!(
            report.differences.len(),
            expected,
            "{:?}",
            report.differences
        );
        assert_eq!(
            report
                .differences
                .iter()
                .filter(|line| line.contains("101..=101"))
                .count(),
            1,
        );
    }
}

#[test]
fn labelled_partial_recovery_does_not_waive_the_lost_interval() {
    let mut lost = frame(101, &[], &[], vec![]);
    *lost.tables[TableId::NavalResultSources.index()] = TableView::Unavailable {
        reason: "naval result sources drain lost".into(),
    };
    let mut recovery = frame(102, &[], &[], vec![]);
    *recovery.tables[TableId::NavalResultSources.index()] = TableView::Unavailable {
        reason: "naval result sources recovery interval is partial".into(),
    };
    let report = checked(&[
        frame(100, &[], &[], vec![]),
        lost,
        recovery,
        frame(103, &[], &[], vec![]),
    ]);
    has_failure(&report, "unavailable_interval");
    assert_eq!(report.differences.len(), 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("after a loss is partial"))
    );
    assert!(report.outcome().is_err());
}

#[test]
fn lost_intervals_and_prerequisite_reads_are_differences() {
    let mut unavailable = frame(101, &[], &[], vec![]);
    *unavailable.tables[TableId::NavalResultSources.index()] = TableView::Unavailable {
        reason: "overlapping native callers".into(),
    };
    has_failure(
        &checked(&[frame(100, &[], &[], vec![]), unavailable.clone()]),
        "unavailable_interval",
    );
    has_failure(&checked(&[unavailable.clone()]), "unavailable_interval");
    *unavailable.tables[TableId::NavalResultSources.index()] = TableView::Unavailable {
        reason: "naval result sources first/branch interval is partial".into(),
    };
    let first = checked(&[unavailable.clone()]);
    assert!(first.differences.is_empty());
    assert!(
        first
            .limitations
            .keys()
            .any(|reason| reason.contains("first event interval is partial"))
    );
    let mut at_first = first;
    at_first.record_tables(&unavailable);
    assert!(at_first.outcome().is_err());
    has_failure(
        &checked(&[frame(100, &[], &[], vec![]), unavailable]),
        "unavailable_interval",
    );
    for table in [TableId::NavalResults, TableId::NavalCombats] {
        let mut frame = frame(101, &[], &[], vec![]);
        *frame.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic read failure".into(),
        };
        has_failure(&checked(&[frame]), "input_unavailable");
    }
    has_failure(
        &checked(&[frame(100, &[], &[], vec![]), frame(102, &[], &[], vec![])]),
        "gap",
    );
}

#[test]
fn a_reattach_first_interval_needs_its_recorded_event_loss() {
    let mut reattached = frame(101, &[], &[], vec![]);
    *reattached.tables[TableId::NavalResultSources.index()] = TableView::Unavailable {
        reason: "naval result sources first/branch interval is partial".into(),
    };
    let frames = [
        frame(100, &[], &[], vec![]),
        reattached,
        frame(102, &[], &[], vec![]),
    ];
    let with = |event_losses: Vec<(Hour, Hour)>| {
        let mut check = SourceCheck {
            event_losses,
            ..SourceCheck::default()
        };
        let mut report = Report::default();
        for frame in &frames {
            check.observe(frame, &mut report);
        }
        check.finish(&mut report);
        report
    };
    let recorded = with(vec![(100, 101)]);
    assert!(
        recorded.differences.is_empty(),
        "{:?}",
        recorded.differences
    );
    assert!(
        recorded
            .limitations
            .keys()
            .any(|reason| reason.contains("after a recorded event loss"))
    );
    for event_losses in [vec![], vec![(99, 100)], vec![(101, 102)]] {
        has_failure(&with(event_losses), "unavailable_interval");
    }
}

#[test]
fn another_branch_cannot_certify_a_result() {
    let mut other = frame(101, &[], &[(21, 101)], vec![]);
    other.branch = 2;
    let frames = [
        frame(100, &[11], &[], vec![]),
        frame(101, &[], &[], vec![event(1, 11, 21)]),
        other,
    ];
    let mut report = Report::default();
    compare(&replay_of(&frames), &frames[1], &mut report).unwrap();
    assert!(report.differences.is_empty());
    has_unobserved_result(&report, 1, 21);
}

#[test]
fn final_retained_date_matches_the_last_ordered_native_emission() {
    let before = frame(100, &[], &[(21, 99)], vec![]);
    let earlier = NavalResultSourceRow {
        game_hour: 100,
        combat_created_hour: Some(100),
        ..merged_event(1, 21)
    };
    let mismatched = frame(101, &[], &[(21, 101)], vec![earlier.clone()]);
    let frames = [before.clone(), mismatched];
    let mut report = Report::default();
    compare(&replay_of(&frames), &frames[1], &mut report).unwrap();
    has_failure(&report, "result_hour");
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

    let matched = frame(101, &[], &[(21, 100)], vec![earlier.clone()]);
    assert!(checked(&[before.clone(), matched]).differences.is_empty());
    let latest = frame(101, &[], &[(21, 101)], vec![merged_event(2, 21), earlier]);
    assert!(checked(&[before, latest]).differences.is_empty());
}
