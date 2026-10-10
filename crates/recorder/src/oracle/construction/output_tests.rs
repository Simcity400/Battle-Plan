use super::*;
use replay::{
    frame::TableView,
    rows::{BuildingConstructionRow, BuildingRow, RailwayConstructionRow, TableRows},
};
fn frame(hour: i32, amount: i64, produced: i64, level: i16, railway: bool) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| TableView::Complete(TableRows::empty(TableId::ALL[i])))
            .map(Into::into),
    };
    frame.metrics.table_ns[TableId::BuildingConstruction.index()] = 1;
    *frame.tables[TableId::ConstructionLineOutput.index()] =
        TableView::Complete(TableRows::ConstructionLineOutput(vec![
            ConstructionLineOutputRow {
                line: 1,
                construction_raw: amount,
                railway,
                since_hour: 100,
                exact: true,
                ..Default::default()
            },
        ]));
    if railway {
        *frame.tables[TableId::RailwayConstruction.index()] =
            TableView::Complete(TableRows::RailwayConstruction(vec![
                RailwayConstructionRow {
                    id: 1,
                    cost: 1_000_000,
                    produced,
                    route: vec![7, 8, 9],
                    ..Default::default()
                },
            ]));
    } else {
        *frame.tables[TableId::BuildingConstruction.index()] =
            TableView::Complete(TableRows::BuildingConstruction(vec![
                BuildingConstructionRow {
                    id: 1,
                    cost: 1_000_000,
                    amount: 3 - i32::from(level),
                    produced,
                    building_type: 9,
                    target_state: 7,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::StateBuildings.index()] =
            TableView::Complete(TableRows::StateBuildings(vec![BuildingRow {
                location: 7,
                building_type: 9,
                level,
                healthy_level: level,
                ..Default::default()
            }]));
    }
    frame
}
fn output(frame: &mut DecodedFrame) -> &mut ConstructionLineOutputRow {
    let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
        &mut *frame.tables[TableId::ConstructionLineOutput.index()]
    else {
        unreachable!()
    };
    &mut rows[0]
}
fn replay_of(frames: Vec<DecodedFrame>) -> Replay {
    use replay::{
        branches::{BranchReason, BranchStart, Parentage},
        frame::FrameEncoder,
        identity::{Identity, Roster, SessionStart, Verification},
        spool::Record,
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
            sample_boundary: replay::identity::SampleBoundary::EndOfHour,
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
        }),
        Record::BranchStart(BranchStart {
            id: 1,
            session: 1,
            loaded_hour: Some(frames[0].hour - 1),
            first_hour: frames[0].hour,
            parent: Parentage::Unknown,
            reason: BranchReason::SessionStart,
        }),
    ];
    let mut encoder = FrameEncoder::new();
    for mut frame in frames {
        for table in TableId::ALL {
            if !OUTPUT_TABLES.contains(&table) {
                *frame.tables[table.index()] = TableView::Unavailable {
                    reason: "outside the construction output fixture".into(),
                };
            }
        }
        *frame.tables[TableId::Clock.index()] =
            TableView::Complete(TableRows::Clock(vec![replay::rows::ClockRow {
                hour: frame.hour,
                ..Default::default()
            }]));
        *frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![replay::rows::CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(b"campaign".to_vec(), &[records]).unwrap()
}

fn checked(frames: &[DecodedFrame]) -> Report {
    let mut check = OutputCheck::default();
    let mut report = Report::default();
    for f in frames {
        check.observe(f, &mut report).unwrap();
    }
    check.finish(&mut report);
    // The current encoder requires every current field; historical suffix
    // absence is exercised on decoded rows and by replay's prefix fixtures.
    if frames.iter().any(|frame| {
        super::complete::<tables::ConstructionLineOutput>(frame)
            .unwrap()
            .iter()
            .any(|row| matches!(row.count_reset, replay::rows::Recorded::NotRecorded))
    }) {
        return report;
    }
    let replay = replay_of(frames.to_vec());
    let mut encoded = Report::default();
    compare_output(&replay, frames.last().unwrap(), &mut encoded).unwrap();
    assert_eq!(encoded.differences, report.differences);
    report
}
#[test]
fn construction_output_does_not_charge_a_lines_spending_for_external_building_growth() {
    let mut frames = [frame(100, 0, 100, 1, false), frame(124, 350, 450, 2, false)];
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frames[1].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].amount = 2;
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
    output(&mut frames[1]).construction_raw += 1;
    assert_eq!(checked(&frames).differences.len(), 1);
}

#[test]
fn quantity_decreases_without_target_growth_or_ownership_transfer_fail() {
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 0, 0, 1, false)];
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frames[1].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].amount -= 1;
    let report = checked(&frames);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("unexplained remaining-count decrease"));
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frames[1].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].amount += 1;
    assert_eq!(
        checked(&frames).compared_tables["ConstructionLineOutput"],
        1
    );
    output(&mut frames[1]).construction_raw += 1;
    assert_eq!(checked(&frames).differences.len(), 1);
}

#[test]
fn ownership_transfer_verifies_count_reset_and_unexplained_decrease_fails() {
    use replay::rows::{CountryLinkRow, StateRow};
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 0, 0, 1, false)];
    for (index, f) in frames.iter_mut().enumerate() {
        let TableView::Complete(TableRows::BuildingConstruction(rows)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        rows[0].country = 11;
        rows[0].amount = if index == 0 { 1 } else { 0 };
        output(f).count_reset = replay::rows::Recorded::Value(index as i64);
        *f.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 7,
                owner: if index == 0 { 12 } else { 21 },
                controller: if index == 0 { 12 } else { 21 },
                ..Default::default()
            }]));
        *f.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 12,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 21,
                    canonical_slot: 2,
                },
            ]));
    }
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ConstructionLineOutput ownership transfer count resets"],
        1
    );
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
    let original = frames.clone();
    // SetOwner@1409D1710 resets the count before 1409D26D0 can lower
    // shared levels. Independent script growth can also surround the reset.
    for level in [0, 1, 3] {
        for completed in [0, 1] {
            let mut frames = original.clone();
            let TableView::Complete(TableRows::StateBuildings(rows)) =
                &mut *frames[1].tables[TableId::StateBuildings.index()]
            else {
                unreachable!()
            };
            rows[0].level = level;
            for (index, f) in frames.iter_mut().enumerate() {
                let TableView::Complete(TableRows::BuildingConstruction(rows)) =
                    &mut *f.tables[TableId::BuildingConstruction.index()]
                else {
                    unreachable!()
                };
                rows[0].amount = if index == 0 { 1 + completed } else { 0 };
                output(f).count_reset = replay::rows::Recorded::Value(index as i64);
            }
            output(&mut frames[1]).construction_raw = i64::from(completed) * 1_000_000;
            let report = checked(&frames);
            assert!(
                report.differences.is_empty(),
                "level {level}, completed {completed}: {:?}",
                report.differences
            );
            assert_eq!(
                report.compared_tables["ConstructionLineOutput ownership transfer count resets"],
                1
            );
            for extra in [1, 1_000_000] {
                let mut bad = frames.clone();
                output(&mut bad[1]).construction_raw += extra;
                assert_eq!(checked(&bad).differences.len(), 1);
            }
            // Readable historical prefixes still check exact native progress
            // and cost debits, including completion followed by a reset.
            for f in &mut frames {
                output(f).count_reset = replay::rows::Recorded::NotRecorded;
            }
            assert!(checked(&frames).differences.is_empty());
            output(&mut frames[1]).construction_raw += 1;
            assert_eq!(checked(&frames).differences.len(), 1);
        }
    }
    output(&mut frames[1]).construction_raw = 1;
    assert_eq!(checked(&frames).differences.len(), 1);
    output(&mut frames[1]).construction_raw = 0;
    let mut historical = original.clone();
    for f in &mut historical {
        output(f).count_reset = replay::rows::Recorded::NotRecorded;
    }
    for owner in [11, 12, 0, u32::MAX, 999] {
        let mut frames = historical.clone();
        let TableView::Complete(TableRows::States(rows)) =
            &mut *frames[1].tables[TableId::States.index()]
        else {
            unreachable!()
        };
        rows[0].owner = owner;
        assert_eq!(checked(&frames).differences.len(), 1, "owner {owner}");
    }
    let mut frames = historical.clone();
    let TableView::Complete(TableRows::States(rows)) =
        &mut *frames[0].tables[TableId::States.index()]
    else {
        unreachable!()
    };
    rows[0].owner = 21;
    assert_eq!(checked(&frames).differences.len(), 1);
    for table in [
        TableId::States,
        TableId::CountryLinks,
        TableId::StateBuildings,
    ] {
        let mut frames = historical.clone();
        *frames[1].tables[table.index()] = TableView::Unavailable {
            reason: "missing ownership-reset witness".into(),
        };
        assert_eq!(
            checked(&frames).differences.len(),
            1,
            "{table:?}: historical ownership inputs are required"
        );
    }
    let mut frames = historical;
    for f in &mut frames {
        let TableView::Complete(TableRows::BuildingConstruction(rows)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        rows[0].amount += 1;
    }
    assert_eq!(
        checked(&frames).differences.len(),
        1,
        "a historical ownership reset must clear the remaining count"
    );
}

#[test]
fn new_building_completions_and_railway_progress_pass_and_wrong_amount_fails() {
    for rail in [false, true] {
        let (amount, level) = if rail { (350, 1) } else { (1_000_350, 2) };
        let mut frames = [
            frame(100, 0, 100, 1, rail),
            frame(124, amount, 450, level, rail),
        ];
        let report = checked(&frames);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        output(&mut frames[1]).construction_raw += 1;
        assert_eq!(checked(&frames).differences.len(), 1);
    }
}

#[test]
fn script_target_growth_does_not_spend_the_waiting_lines_ic() {
    // A focus reward adds two levels while the queue has zero factories and
    // unchanged progress; an independent gift can also accompany paid work.
    for amount in [0, 350, 1_000_350, 2_000_350] {
        let progress = if amount == 0 { 0 } else { 350 };
        let mut frames = [
            frame(100, 0, 0, 2, false),
            frame(124, amount, progress, 4, false),
        ];
        for (index, frame) in frames.iter_mut().enumerate() {
            let TableView::Complete(TableRows::BuildingConstruction(rows)) =
                &mut *frame.tables[TableId::BuildingConstruction.index()]
            else {
                unreachable!()
            };
            rows[0].amount = 3 - if index == 0 {
                0
            } else {
                (amount / 1_000_000) as i32
            };
        }
        let report = checked(&frames);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        assert_eq!(
            report.compared_tables["ConstructionLineOutput external target growth bounds"],
            usize::from(amount < 2_000_350)
        );
        // A fractional missing debit and an extra whole level remain defects.
        output(&mut frames[1]).construction_raw += 1;
        assert_eq!(checked(&frames).differences.len(), 1);
        if amount >= 1_000_000 {
            output(&mut frames[1]).construction_raw = amount - 1_000_000;
            assert_eq!(checked(&frames).differences.len(), 1);
        }
        output(&mut frames[1]).construction_raw = 3_000_000 + progress;
        assert_eq!(checked(&frames).differences.len(), 1);
    }
}
#[test]
fn output_reads_follow_the_daily_slot_and_forced_refresh_not_the_timing_anchor() {
    let slot = match replay::cadence::cadence(TableId::ConstructionLineOutput) {
        replay::cadence::Cadence::Daily(slot) => i32::try_from(slot).unwrap(),
        _ => panic!("construction output must share the construction census slot"),
    };
    for rail in [false, true] {
        let mut frames = [
            frame(120 + slot, 0, 100, 1, rail),
            frame(144 + slot, 350, 450, 1, rail),
        ];
        for f in &mut frames {
            f.metrics.table_ns.fill(0);
            // A shared read can charge an hourly anchor instead of the census.
            f.metrics.table_ns[TableId::ProvinceSupply.index()] = 1;
        }
        let report = checked(&frames);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        output(&mut frames[1]).construction_raw += 1;
        assert_eq!(checked(&frames).differences.len(), 1);

        // Between census reads the rows are carried; an hourly anchor's
        // timing alone cannot date a new publication.
        let mut carried = frame(121 + slot, 900, 999, 1, rail);
        carried.metrics.table_ns.fill(0);
        carried.metrics.table_ns[TableId::ProvinceSupply.index()] = 1;
        output(&mut frames[1]).construction_raw -= 1;
        let report = checked(&[frames[0].clone(), carried.clone(), frames[1].clone()]);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);

        // An off-slot full refresh is dated by a timed table of the slot.
        carried.metrics.table_ns[TableId::BuildingConstruction.index()] = 1;
        output(&mut carried).construction_raw = 899;
        let report = checked(&[frames[0].clone(), carried.clone()]);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        output(&mut carried).construction_raw += 1;
        let report = checked(&[frames[0].clone(), carried]);
        assert_eq!(report.differences.len(), 1);
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
    }
}
#[test]
fn bank_is_not_factory_output_and_carry_is_only_a_discard_bound() {
    let mut frames = [frame(100, 0, 0, 1, true), frame(124, 900, 950, 1, true)];
    output(&mut frames[1]).bank_raw = 100;
    output(&mut frames[1]).carry_raw = 80;
    assert!(checked(&frames).differences.is_empty());
    output(&mut frames[1]).carry_raw = 40;
    assert_eq!(checked(&frames).differences.len(), 1);
}
#[test]
fn a_linked_secondary_mirrors_progress_without_a_spend() {
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 200, 200, 1, false)];
    output(&mut frames[1]).linked = true;
    for f in &mut frames {
        let TableView::Complete(TableRows::BuildingConstruction(queue)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        queue.push(BuildingConstructionRow {
            id: 2,
            country: 9,
            // The province operand is unused by this state target.
            target_location: 44,
            ..queue[0].clone()
        });
        let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
            &mut *f.tables[TableId::ConstructionLineOutput.index()]
        else {
            unreachable!()
        };
        rows.push(ConstructionLineOutputRow {
            line: 2,
            construction_raw: 0,
            linked: true,
            exact: false,
            ticks: 0,
            ..rows[0].clone()
        });
    }
    let report = checked(&frames);
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
    assert!(report.limitations.keys().any(|r| r.contains("linked")));
}
fn add_railway(frame: &mut DecodedFrame, route: Vec<u32>) {
    let TableView::Complete(TableRows::RailwayConstruction(lines)) =
        &mut *frame.tables[TableId::RailwayConstruction.index()]
    else {
        unreachable!()
    };
    lines.push(RailwayConstructionRow {
        id: 2,
        country: 9,
        route,
        ..Default::default()
    });
    let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
        &mut *frame.tables[TableId::ConstructionLineOutput.index()]
    else {
        unreachable!()
    };
    rows.push(ConstructionLineOutputRow {
        line: 2,
        railway: true,
        since_hour: 100,
        exact: true,
        ..Default::default()
    });
}
#[test]
fn railway_overlap_in_either_read_excludes_a_shared_progress_gain() {
    for reads in [&[0][..], &[1][..], &[0, 1][..]] {
        let mut frames = [frame(100, 0, 0, 1, true), frame(124, 0, 200, 1, true)];
        // Reversed order and another country's queue still share progress.
        for &read in reads {
            add_railway(&mut frames[read], vec![10, 9, 8]);
        }
        let report = checked(&frames);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput railways"], 0);
        assert_eq!(
            report.limitations["shares a segment with another railway line"],
            [format!("ConstructionLineOutput: {}", reads.len())]
        );
    }
}
#[test]
fn railway_with_only_a_shared_province_or_repeated_own_segment_is_compared() {
    let mut frames = [frame(100, 0, 0, 1, true), frame(124, 200, 200, 1, true)];
    for f in &mut frames {
        add_railway(f, vec![9, 10]);
        let TableView::Complete(TableRows::RailwayConstruction(lines)) =
            &mut *f.tables[TableId::RailwayConstruction.index()]
        else {
            unreachable!()
        };
        lines[0].route = vec![7, 8, 7, 8, 9];
    }
    let report = checked(&frames);
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["ConstructionLineOutput railways"], 2);
    assert!(
        !report
            .limitations
            .contains_key("shares a segment with another railway line")
    );
    output(&mut frames[1]).construction_raw += 1;
    assert_eq!(checked(&frames).differences.len(), 1);
}

#[test]
fn repair_necessary_check_accepts_factory_and_zero_ic_advances_and_rejects_missing_tick() {
    for factory in [false, true] {
        let mut frames = [frame(100, 0, 0, 2, false), frame(124, 0, 0, 2, false)];
        for f in &mut frames {
            let TableView::Complete(TableRows::BuildingConstruction(rows)) =
                &mut *f.tables[TableId::BuildingConstruction.index()]
            else {
                unreachable!()
            };
            rows[0].repair_levels = 1;
            let TableView::Complete(TableRows::StateBuildings(rows)) =
                &mut *f.tables[TableId::StateBuildings.index()]
            else {
                unreachable!()
            };
            rows[0].healthy_level = 1;
            rows[0].partial_health = if f.hour == 100 { 10 } else { 20 };
        }
        output(&mut frames[1]).repair_ticks = 1;
        output(&mut frames[1]).repair_raw = if factory { 100 } else { 0 };
        let report = checked(&frames);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let key = if factory {
            "ConstructionLineOutput repair necessary-check"
        } else {
            "ConstructionLineOutput natural repair necessary-check"
        };
        assert_eq!(report.compared_tables[key], 1);
        output(&mut frames[1]).repair_ticks = 0;
        assert_eq!(checked(&frames).differences.len(), 1);
    }
}
#[test]
fn linked_instant_and_missing_repair_operands_are_named_limitations() {
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 200, 999, 1, false)];
    output(&mut frames[1]).linked = true;
    assert!(checked(&frames).differences.is_empty());
    output(&mut frames[1]).linked = false;
    output(&mut frames[1]).exact = false;
    assert!(checked(&frames).differences.is_empty());
    output(&mut frames[1]).exact = true;
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frames[1].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].repair_levels = 1;
    *frames[1].tables[TableId::StateBuildings.index()] = TableView::Unavailable {
        reason: "fixture missing".into(),
    };
    let report = checked(&frames);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|r| r.contains("health/level operands missing"))
    );
}

#[test]
fn native_completion_self_check_refuses_live_or_future_outcomes() {
    use replay::rows::{ConstructionCompletion, Recorded};
    let mut f = frame(124, 35, 35, 1, true);
    output(&mut f).ended = true;
    output(&mut f).completion = Recorded::Value(Some(ConstructionCompletion {
        hour: 120,
        cancelled: true,
    }));
    assert_eq!(checked(&[f.clone()]).differences.len(), 1); // still in queue
    *f.tables[TableId::RailwayConstruction.index()] =
        TableView::Complete(TableRows::RailwayConstruction(Vec::new()));
    let report = checked(&[f.clone()]);
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["ConstructionLineOutput native cancellation consistency"],
        1
    );
    output(&mut f).completion = Recorded::Value(Some(ConstructionCompletion {
        hour: 125,
        cancelled: false,
    }));
    assert_eq!(checked(&[f]).differences.len(), 1);
}

#[test]
fn signed_construction_and_native_progress_wrap_are_exact_self_checks() {
    let signed = [frame(100, 0, 100, 1, false), frame(124, -25, 75, 1, false)];
    assert!(checked(&signed).differences.is_empty());
    let wrapped = [
        frame(100, 0, i64::MAX, 1, false),
        frame(124, 1, i64::MIN, 1, false),
    ];
    assert!(checked(&wrapped).differences.is_empty());
    let mut bank = [
        frame(100, 0, 0, 1, false),
        frame(124, i64::MAX, i64::MIN, 1, false),
    ];
    output(&mut bank[1]).bank_raw = 1;
    assert!(checked(&bank).differences.is_empty());
    output(&mut bank[1]).bank_raw = 2;
    assert_eq!(checked(&bank).differences.len(), 1);
}
