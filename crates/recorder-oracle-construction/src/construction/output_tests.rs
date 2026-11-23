use super::*;
use replay_data::{
    rows::BuildingConstructionRow, rows::BuildingRow, rows::RailwayConstructionRow, rows::Recorded,
    rows::TableRows,
};
use replay_frame::frame::TableView;
fn frame(hour: i32, amount: i64, produced: i64, level: i16, railway: bool) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|i| TableView::Complete(TableRows::empty(TableId::ALL[i])))
            .map(Into::into),
    };
    construction_walk(&mut frame, true);
    *frame.tables[TableId::ConstructionLineOutput.index()] =
        TableView::Complete(TableRows::ConstructionLineOutput(vec![
            ConstructionLineOutputRow {
                line: 1,
                construction_raw: amount,
                railway,
                since_hour: 100,
                exact: true,
                country: Recorded::Value(Some(0)),
                building_type: Recorded::Value((!railway).then_some(9)),
                target_flags: Recorded::Value((!railway).then_some(0)),
                target_location: Recorded::Value((!railway).then_some(0)),
                target_state: Recorded::Value((!railway).then_some(7)),
                target_country: Recorded::Value((!railway).then_some(0)),
                route_first: Recorded::Value(railway.then_some(7)),
                route_last: Recorded::Value(railway.then_some(9)),
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
fn construction_walk(frame: &mut DecodedFrame, read: bool) {
    let slot = replay_data::cadence::cadence(TableId::ConstructionLineOutput);
    frame.metrics.table_ns.fill(0);
    frame.metrics.table_ns[TableId::ProvinceSupply.index()] = 1;
    for table in TableId::ALL {
        if replay_data::cadence::cadence(table) == slot {
            frame.metrics.table_ns[table.index()] = u64::from(read);
        }
    }
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
    use replay_capture_spool::spool::Record;
    use replay_data::{
        identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Parentage, frame::FrameEncoder,
    };
    let mut records = vec![
        Record::SessionStart(SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 10_000_000,
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
            TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
                hour: frame.hour,
                ..Default::default()
            }]));
        *frame.tables[TableId::Campaign.index()] =
            TableView::Complete(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
                id: b"campaign".to_vec(),
            }]));
        records.push(Record::Frame(encoder.encode(frame, false).unwrap()));
    }
    Replay::from_records(recorder_derive::writers(), b"campaign".to_vec(), &[records]).unwrap()
}

fn checked(frames: &[DecodedFrame]) -> Report {
    let mut check = OutputCheck::default();
    let mut report = Report::default();
    for f in frames {
        check.observe(f, &mut report).unwrap();
    }
    check.finish(&mut report, frames.last().map_or(0, |f| f.hour));
    if frames.iter().any(|frame| {
        super::complete::<tables::ConstructionLineOutput>(frame)
            .unwrap()
            .iter()
            .any(|row| matches!(row.count_reset, replay_data::rows::Recorded::NotRecorded))
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
fn a_branch_whose_last_line_retired_still_checks_its_earlier_publications() {
    let mut frames = [
        frame(100, 0, 100, 1, false),
        frame(124, 351, 450, 2, false),
        frame(148, 0, 0, 2, false),
    ];
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frames[1].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].amount = 2;
    for table in [
        TableId::ConstructionLineOutput,
        TableId::BuildingConstruction,
    ] {
        *frames[2].tables[table.index()] = TableView::Complete(TableRows::empty(table));
    }
    let report = checked(&frames);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
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
    use replay_data::{rows::CountryLinkRow, rows::StateRow};
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 0, 0, 1, false)];
    for (index, f) in frames.iter_mut().enumerate() {
        let TableView::Complete(TableRows::BuildingConstruction(rows)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        rows[0].country = 11;
        rows[0].amount = if index == 0 { 1 } else { 0 };
        output(f).count_reset = replay_data::rows::Recorded::Value(index as i64);
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
                output(f).count_reset = replay_data::rows::Recorded::Value(index as i64);
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
            for f in &mut frames {
                output(f).count_reset = replay_data::rows::Recorded::NotRecorded;
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
        output(f).count_reset = replay_data::rows::Recorded::NotRecorded;
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
fn output_reads_are_the_hours_the_recording_marks_it_read() {
    let slot = slot();
    for rail in [false, true] {
        let mut frames = [
            frame(120 + slot, 0, 100, 1, rail),
            frame(144 + slot, 350, 450, 1, rail),
        ];
        let report = checked(&frames);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        output(&mut frames[1]).construction_raw += 1;
        assert_eq!(checked(&frames).differences.len(), 1);

        let mut carried = frame(121 + slot, 900, 999, 1, rail);
        construction_walk(&mut carried, false);
        output(&mut frames[1]).construction_raw -= 1;
        let report = checked(&[frames[0].clone(), carried.clone(), frames[1].clone()]);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);

        let mut forced = carried;
        construction_walk(&mut forced, true);
        output(&mut forced).construction_raw = 899;
        let report = checked(&[frames[0].clone(), forced.clone()]);
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
        output(&mut forced).construction_raw += 1;
        let report = checked(&[frames[0].clone(), forced]);
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
            country: Recorded::Value(Some(9)),
            target_location: Recorded::Value(Some(44)),
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
    let endpoints = route.first().copied().zip(route.last().copied());
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
        country: Recorded::Value(Some(9)),
        route_first: Recorded::Value(endpoints.map(|(first, _)| first)),
        route_last: Recorded::Value(endpoints.map(|(_, last)| last)),
        since_hour: 100,
        exact: true,
        ..Default::default()
    });
}
#[test]
fn railway_overlap_in_either_read_excludes_a_shared_progress_gain() {
    for reads in [&[0][..], &[1][..], &[0, 1][..]] {
        let mut frames = [frame(100, 0, 0, 1, true), frame(124, 0, 200, 1, true)];
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
fn railway_progress_without_a_spend_advance_is_not_this_lines_output() {
    let mut frames = [frame(100, 0, 0, 1, true), frame(124, 0, 1_000_000, 1, true)];
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput railways"], 0);
    assert_eq!(
        report
            .limitations
            .keys()
            .filter(|r| r.contains("no spend advance"))
            .count(),
        1
    );
    output(&mut frames[1]).ticks = 1;
    assert_eq!(checked(&frames).differences.len(), 1);
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
    use replay_data::{rows::ConstructionCompletion, rows::Recorded};
    let mut f = frame(124, 35, 35, 1, true);
    output(&mut f).ended = true;
    output(&mut f).completion = Recorded::Value(Some(ConstructionCompletion {
        hour: 120,
        cancelled: true,
    }));
    assert_eq!(checked(&[f.clone()]).differences.len(), 2);
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
fn a_queued_line_without_a_live_output_row_is_a_difference() {
    let mut frames = [frame(100, 0, 0, 1, false), frame(124, 0, 0, 1, false)];
    for f in &mut frames {
        let TableView::Complete(TableRows::BuildingConstruction(queue)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        queue.push(BuildingConstructionRow {
            id: 2,
            ..queue[0].clone()
        });
    }
    let report = checked(&frames);
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .all(|difference| difference.contains("ConstructionLineOutput 2 is queued"))
    );
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
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

fn set_amount(frame: &mut DecodedFrame, value: i32) {
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *frame.tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows[0].amount = value;
}

fn checked_at(frames: &[DecodedFrame], compared: Hour) -> Report {
    let mut check = OutputCheck::default();
    let mut report = Report::default();
    for f in frames {
        check.observe(f, &mut report).unwrap();
    }
    check.finish(&mut report, compared);
    report
}

#[test]
fn a_completion_beside_a_count_addition_is_its_exact_cost_debit_under_the_level_rise() {
    let mut frames = [
        frame(100, 0, 900_000, 1, false),
        frame(124, 200_350, 100_350, 2, false),
    ];
    set_amount(&mut frames[0], 2);
    set_amount(&mut frames[1], 2);
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
    assert_eq!(
        report.compared_tables["ConstructionLineOutput completions beside count additions"],
        1
    );
    let report = checked_at(&frames, 148);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("unwitnessed count addition 100..124"));
    let mut ended = frames.to_vec();
    ended.push(frame(148, 200_350, 100_350, 2, false));
    let TableView::Complete(TableRows::BuildingConstruction(rows)) =
        &mut *ended[2].tables[TableId::BuildingConstruction.index()]
    else {
        unreachable!()
    };
    rows.clear();
    output(&mut ended[2]).ended = true;
    let report = checked(&ended);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.limitations[ENDED_ADDITION],
        ["ConstructionLineOutput 1 completion beside an unwitnessed count addition 100..124"]
    );
    let mut vanished = ended.clone();
    let TableView::Complete(TableRows::ConstructionLineOutput(rows)) =
        &mut *vanished[2].tables[TableId::ConstructionLineOutput.index()]
    else {
        unreachable!()
    };
    rows.clear();
    assert!(
        checked(&vanished)
            .differences
            .iter()
            .any(|difference| difference.contains("unwitnessed count addition 100..124"))
    );
    let mut later = frames.to_vec();
    later.push(frame(148, 200_350, 100_350, 2, false));
    set_amount(&mut later[2], 1);
    assert!(
        checked(&later)
            .differences
            .iter()
            .any(|difference| difference.contains("unexplained remaining-count decrease")),
    );
    set_amount(&mut later[2], 2);
    assert!(checked(&later).differences.is_empty());
    output(&mut frames[1]).construction_raw += 1;
    assert_eq!(checked(&frames).differences.len(), 1);
    output(&mut frames[1]).construction_raw -= 1;
    let mut flat = frames.clone();
    let TableView::Complete(TableRows::StateBuildings(rows)) =
        &mut *flat[1].tables[TableId::StateBuildings.index()]
    else {
        unreachable!()
    };
    rows[0].level = 1;
    rows[0].healthy_level = 1;
    assert_eq!(checked(&flat).differences.len(), 1);
}

fn slot() -> i32 {
    match replay_data::cadence::cadence(TableId::ConstructionLineOutput) {
        replay_data::cadence::Cadence::Daily(slot) => i32::try_from(slot).unwrap(),
        _ => panic!("construction output must share the construction census slot"),
    }
}

#[test]
fn a_refreshed_compared_hour_off_the_slot_certifies_a_count_addition_by_its_own_read() {
    let compared = 144 + slot() + 7;
    assert!(!replay_data::cadence::is_refresh(
        TableId::ConstructionLineOutput,
        compared
    ));
    let mut frames = [
        frame(120 + slot(), 0, 900_000, 1, false),
        frame(144 + slot(), 200_350, 100_350, 2, false),
        frame(compared, 200_350, 100_350, 2, false),
    ];
    for frame in &mut frames {
        set_amount(frame, 2);
    }
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ConstructionLineOutput completions beside count additions"],
        1
    );
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 2);
}

#[test]
fn a_forced_construction_read_dates_its_tables_last_read() {
    let compared = 144 + slot() + 7;
    let frames = vec![
        frame(120 + slot(), 0, 100, 1, false),
        frame(compared, 350, 450, 1, false),
    ];
    let hours =
        recorder_oracle_support::report::last_read_hours(&replay_of(frames), 1, compared).unwrap();
    for table in [
        TableId::ConstructionLineOutput,
        TableId::BuildingConstruction,
        TableId::StateBaseResources,
    ] {
        assert_eq!(hours.get(&table), Some(&compared), "{table:?}");
    }
}

#[test]
fn a_completion_beside_a_net_count_rise_is_inferred_and_certified_the_same_way() {
    let mut frames = [
        frame(100, 0, 900_000, 1, false),
        frame(124, 200_350, 100_350, 2, false),
    ];
    set_amount(&mut frames[0], 2);
    set_amount(&mut frames[1], 3);
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ConstructionLineOutput completions beside count additions"],
        1
    );
    assert_eq!(checked_at(&frames, 148).differences.len(), 1);
    output(&mut frames[1]).construction_raw += 1_000_000;
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
    let TableView::Complete(TableRows::StateBuildings(rows)) =
        &mut *frames[1].tables[TableId::StateBuildings.index()]
    else {
        unreachable!()
    };
    rows[0].level = 3;
    rows[0].healthy_level = 3;
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 1);
    let mut rise = [
        frame(100, 0, 900_000, 1, false),
        frame(124, 50, 900_050, 1, false),
    ];
    set_amount(&mut rise[0], 2);
    set_amount(&mut rise[1], 3);
    let report = checked(&rise);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
}

#[test]
fn no_count_addition_is_inferred_beside_a_count_reset_or_an_ownership_transfer() {
    use replay_data::{rows::CountryLinkRow, rows::StateRow};
    let base = || {
        let mut frames = [
            frame(100, 0, 900_000, 1, false),
            frame(124, 200_350, 100_350, 2, false),
        ];
        set_amount(&mut frames[0], 2);
        set_amount(&mut frames[1], 2);
        frames
    };
    let mut frames = base();
    for (index, f) in frames.iter_mut().enumerate() {
        output(f).count_reset = replay_data::rows::Recorded::Value(index as i64);
    }
    let report = checked(&frames);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert!(report.differences[0].contains("count_reset"));
    set_amount(&mut frames[1], 3);
    let report = checked(&frames);
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["ConstructionLineOutput"], 0);
    let mut frames = base();
    for (index, f) in frames.iter_mut().enumerate() {
        let TableView::Complete(TableRows::BuildingConstruction(rows)) =
            &mut *f.tables[TableId::BuildingConstruction.index()]
        else {
            unreachable!()
        };
        rows[0].country = 11;
        *f.tables[TableId::States.index()] =
            TableView::Complete(TableRows::States(vec![StateRow {
                id: 7,
                owner: if index == 0 { 11 } else { 21 },
                controller: if index == 0 { 11 } else { 21 },
                ..Default::default()
            }]));
        *f.tables[TableId::CountryLinks.index()] =
            TableView::Complete(TableRows::CountryLinks(vec![
                CountryLinkRow {
                    id: 11,
                    canonical_slot: 1,
                },
                CountryLinkRow {
                    id: 21,
                    canonical_slot: 2,
                },
            ]));
    }
    let report = checked(&frames);
    assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ConstructionLineOutput completions beside count additions"],
        0
    );
    assert_eq!(
        report.compared_tables["ConstructionLineOutput ownership transfer count resets"],
        0
    );
}
