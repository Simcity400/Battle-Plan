use super::*;
use replay_data::{
    rows::CountryLinkRow, rows::CountryRow, rows::DivisionDeletionDisposition,
    rows::DivisionEquipmentDeletionRow, rows::DivisionRow, rows::EquipmentDestructionRow,
    rows::TableId, rows::TableRows, rows::VariantDefinitionRow,
};
use replay_frame::{frame::Metrics, frame::TableView};

const OWNER: u32 = 4999;
const ALIAS: u32 = 2_000_001;
const DESIGN: u64 = 0xf00100000046;
const DIVISION: u64 = 7_u64 << 32 | 987_654;

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour: 2,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "fixture absent".into(),
            }
            .into()
        }),
    };
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: OWNER,
                canonical_slot: 17,
            },
            CountryLinkRow {
                id: ALIAS,
                canonical_slot: 17,
            },
        ]));
    *frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(vec![CountryRow {
            id: OWNER,
            canonical_slot: 17,
            tag: b"X_custom_4999".to_vec(),
            ..Default::default()
        }]));
    *frame.tables[TableId::VariantDefinitions.index()] =
        TableView::Complete(TableRows::VariantDefinitions(vec![VariantDefinitionRow {
            id: DESIGN,
            ..Default::default()
        }]));
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![]));
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] =
        TableView::Complete(TableRows::DivisionEquipmentDeletions(vec![]));
    *frame.tables[TableId::Divisions.index()] = TableView::Complete(TableRows::Divisions(vec![]));
    frame
}

fn amount(cause: EquipmentDestructionCause) -> EquipmentDestructionRow {
    EquipmentDestructionRow {
        country: OWNER,
        cause,
        variant: DESIGN,
        amount_raw: 7,
        ..Default::default()
    }
}

fn deletion(cause: EquipmentDestructionCause) -> DivisionEquipmentDeletionRow {
    DivisionEquipmentDeletionRow {
        country: OWNER,
        disposition: DivisionDeletionDisposition::Loss,
        cause: Some(cause),
        division_handle: DIVISION,
        ..Default::default()
    }
}

fn save(units: &str) -> Save {
    let text = format!(
        "HOI4txt\ndate=\"1936.1.1.2\" game_unique_id=\"synthetic\" version=\"synthetic\" game_unique_seed=1 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13\ncountries={{ X_custom_4999={{ {units} }} }}\nstates={{}}\nprovinces={{}}\n"
    );
    content_reader_save::parse(text.as_bytes()).unwrap()
}

fn deletion_rows(frame: &mut DecodedFrame, rows: Vec<DivisionEquipmentDeletionRow>) {
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] =
        TableView::Complete(TableRows::DivisionEquipmentDeletions(rows));
}

#[test]
fn all_stable_causes_check_designs_without_claiming_save_amount_agreement() {
    let mut frame = frame();
    for cause in (0..=u8::MAX).filter_map(|value| EquipmentDestructionCause::try_from(value).ok()) {
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![amount(cause)]));
        let mut report = Report::default();
        compare(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["EquipmentDestruction"], 1);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("Saves serialize remaining pools"))
        );
        if cause == EquipmentDestructionCause::Unknown {
            assert!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|subject| subject.contains("1 unknown aggregate rows"))
            );
        }
    }
}

#[test]
fn invalid_owners_designs_amounts_and_unavailable_intervals_refuse() {
    let mut frame = frame();
    let row = amount(EquipmentDestructionCause::Attrition);
    for bad in [
        EquipmentDestructionRow {
            country: 5000,
            ..row.clone()
        },
        EquipmentDestructionRow {
            variant: DESIGN + (1 << 32),
            ..row.clone()
        },
        EquipmentDestructionRow {
            variant: 0,
            ..row.clone()
        },
        EquipmentDestructionRow {
            amount_raw: 0,
            ..row
        },
    ] {
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![bad]));
        assert!(compare(&frame, &mut Report::default()).is_err());
    }
    for reason in ["first/branch interval partial", "overflow"] {
        *frame.tables[TableId::EquipmentDestruction.index()] = TableView::Unavailable {
            reason: reason.into(),
        };
        assert!(
            compare(&frame, &mut Report::default())
                .unwrap_err()
                .to_string()
                .contains(reason)
        );
    }
    let partial_marker = EquipmentDestructionRow {
        coverage: EquipmentDestructionCoverage::PartialInterval.into(),
        unavailable_reason: "reworded startup interval".to_owned().into(),
        ..Default::default()
    };
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![partial_marker.clone()]));
    assert!(
        compare(&frame, &mut Report::default())
            .unwrap_err()
            .to_string()
            .contains("reworded startup interval")
    );
    *frame.tables[TableId::EquipmentDestruction.index()] = TableView::Complete(
        TableRows::EquipmentDestruction(vec![partial_marker, amount(EquipmentDestructionCause::Attrition)]),
    );
    assert!(compare(&frame, &mut Report::default()).is_err());
}

#[test]
fn native_pool_gain_rows_are_counted_and_signed_sums_preserve_corrections() {
    for (loss, gain, expected) in [(7, -12, -5), (7, -7, 0), (i64::MAX, i64::MIN, -1)] {
        let mut frame = frame();
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![
                EquipmentDestructionRow {
                    amount_raw: loss,
                    ..amount(EquipmentDestructionCause::LandCombat)
                },
                EquipmentDestructionRow {
                    amount_raw: gain,
                    ..amount(EquipmentDestructionCause::ScriptedEquipmentFraction)
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["EquipmentDestruction"], 2);
        assert_eq!(report.limitations["native pool gains are recorded as negative net removals, so these rows correct earlier totals rather than counting losses"].len(), 1);
        assert_eq!(
            report.limitations["native pool gains are recorded as negative net removals, so these rows correct earlier totals rather than counting losses"]
                [0],
            "EquipmentDestruction.amount_raw: 1 negative aggregate rows are native gain corrections"
        );
        assert!(report.limitations.values().flatten().any(|subject| {
            subject
                == &format!(
                    "EquipmentDestruction.amount_raw: 2 aggregate rows; signed net {expected} raw"
                )
        }));
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("signed net removals have no exact save amount comparator")
        }));
    }
}

#[test]
fn every_deletion_cause_checks_exact_save_absence_and_prior_full_handle() {
    let mut current = frame();
    let mut previous = frame();
    previous.hour -= 1;
    *previous.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            roster_owner: ALIAS,
            ..Default::default()
        }]));
    let saved = save("");
    for cause in (0..=u8::MAX)
        .filter_map(|value| EquipmentDestructionCause::try_from(value).ok())
        .filter(|cause| cause.is_division_deletion())
    {
        deletion_rows(&mut current, vec![deletion(cause)]);
        let mut report = Report::default();
        compare_deletions(&current, &saved, &mut report).unwrap();
        compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["DivisionEquipmentDeletions"], 1);
        assert_eq!(
            report.compared_tables["DivisionEquipmentDeletions.previous_census"],
            1
        );
        if cause == EquipmentDestructionCause::Unknown {
            assert!(
                report
                    .limitations
                    .values()
                    .flatten()
                    .any(|subject| subject.contains("1 unknown deletion witnesses"))
            );
        }
    }
}

#[test]
fn opaque_kind_zero_keeps_full_identity_for_save_absence_and_census_join() {
    let handle = 7_u64 << 32;
    let mut current = frame();
    deletion_rows(
        &mut current,
        vec![DivisionEquipmentDeletionRow {
            division_handle: handle,
            ..deletion(EquipmentDestructionCause::DivisionDestroyed)
        }],
    );
    let mut previous = frame();
    previous.hour -= 1;
    *previous.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: handle,
            roster_owner: OWNER,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_deletions(&current, &save(""), &mut report).unwrap();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["DivisionEquipmentDeletions"], 1);
    assert_eq!(
        report.compared_tables["DivisionEquipmentDeletions.previous_census"],
        1
    );
    let saved = save(
        "units={ division={ id={id=7 type=0} logical_country=X_custom_4999 strength=1 organisation=1 } }",
    );
    let mut report = Report::default();
    compare_deletions(&current, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("still present in the save"));
}

#[test]
fn a_recorded_deletion_still_saved_is_a_difference() {
    let mut frame = frame();
    deletion_rows(
        &mut frame,
        vec![deletion(EquipmentDestructionCause::DivisionDestroyed)],
    );
    let saved = save(
        "units={ division={ id={id=7 type=987654} logical_country=X_custom_4999 strength=1 organisation=1 } }",
    );
    assert!(saved.divisions.contains_key(&DIVISION));
    let mut report = Report::default();
    compare_deletions(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("still present in the save"));
    assert!(report.outcome().is_err());
}

#[test]
fn a_recorded_deletion_still_in_current_census_is_a_difference() {
    let mut frame = frame();
    deletion_rows(
        &mut frame,
        vec![deletion(EquipmentDestructionCause::DivisionDestroyed)],
    );
    *frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            ..Default::default()
        }]));
    let mut report = Report::default();
    compare_deletions(&frame, &save(""), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("still present in the current Divisions census"));
    assert!(report.outcome().is_err());
    *frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "live census failed".into(),
    };
    let mut report = Report::default();
    compare_deletions(&frame, &save(""), &mut report).unwrap();
    assert!(
        !report
            .compared_tables
            .contains_key("DivisionEquipmentDeletions.current_census")
    );
    assert!(report.limitations.values().flatten().any(|subject| {
        subject.contains("1 witnesses; hour 2 Divisions is unavailable: live census failed")
    }));
}

#[test]
fn empty_discarded_pool_still_has_a_save_absence_witness() {
    let mut frame = frame();
    deletion_rows(
        &mut frame,
        vec![deletion(EquipmentDestructionCause::DivisionDeleted)],
    );
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    compare_deletions(&frame, &save(""), &mut report).unwrap();
    assert_eq!(report.compared_tables["EquipmentDestruction"], 0);
    assert_eq!(report.compared_tables["DivisionEquipmentDeletions"], 1);
    assert!(report.differences.is_empty());
}

#[test]
fn owner_aliases_resolve_opaque_tags_and_missing_saved_country_fails() {
    let mut frame = frame();
    deletion_rows(
        &mut frame,
        vec![DivisionEquipmentDeletionRow {
            country: ALIAS,
            ..deletion(EquipmentDestructionCause::DivisionScripted)
        }],
    );
    let mut saved = save("");
    let mut report = Report::default();
    compare_deletions(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    saved.countries.clear();
    let mut report = Report::default();
    compare_deletions(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
}

#[test]
fn invalid_deletion_owner_identity_causes_and_partial_intervals_refuse() {
    let mut frame = frame();
    let row = deletion(EquipmentDestructionCause::DivisionDeleted);
    for bad in [
        DivisionEquipmentDeletionRow {
            country: 0,
            ..row.clone()
        },
        DivisionEquipmentDeletionRow {
            country: 5000,
            ..row.clone()
        },
        DivisionEquipmentDeletionRow {
            division_handle: 987_654,
            ..row.clone()
        },
        DivisionEquipmentDeletionRow {
            disposition: DivisionDeletionDisposition::Loss,
            cause: Some(EquipmentDestructionCause::ParadropDisruption),
            ..row.clone()
        },
        DivisionEquipmentDeletionRow {
            disposition: DivisionDeletionDisposition::Loss,
            cause: Some(EquipmentDestructionCause::LandCombat),
            ..row.clone()
        },
    ] {
        deletion_rows(&mut frame, vec![bad]);
        assert!(compare_deletions(&frame, &save(""), &mut Report::default()).is_err());
    }
    deletion_rows(&mut frame, vec![row.clone(), row]);
    assert!(compare_deletions(&frame, &save(""), &mut Report::default()).is_err());
    for reason in ["first/branch interval partial", "pre-free hook uninstalled"] {
        *frame.tables[TableId::DivisionEquipmentDeletions.index()] = TableView::Unavailable {
            reason: reason.into(),
        };
        assert!(
            compare_deletions(&frame, &save(""), &mut Report::default())
                .unwrap_err()
                .to_string()
                .contains(reason)
        );
    }
}

#[test]
fn missing_prior_census_is_counted_and_never_invented() {
    let mut current = frame();
    deletion_rows(
        &mut current,
        vec![deletion(EquipmentDestructionCause::DivisionScripted)],
    );
    let mut previous = frame();
    previous.hour -= 1;
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert_eq!(
        report.compared_tables["DivisionEquipmentDeletions.previous_census"],
        0
    );
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 deletion witnesses not previously censused"))
    );
    let mut report = Report::default();
    compare_previous_census(&current, None, &save(""), "branch gap", &mut report).unwrap();
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 witnesses; preceding frame: branch gap"))
    );
    *previous.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "census read failed".into(),
    };
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert!(report.limitations.values().flatten().any(|subject| {
        subject.contains("1 witnesses; hour 1 Divisions is unavailable: census read failed")
    }));
    previous.branch += 1;
    assert!(
        compare_previous_census(
            &current,
            Some(&previous),
            &save(""),
            "",
            &mut Report::default()
        )
        .is_err()
    );
}

fn previous_with_division() -> DecodedFrame {
    let mut previous = frame();
    previous.hour -= 1;
    *previous.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: DIVISION,
            roster_owner: OWNER,
            ..Default::default()
        }]));
    previous
}

#[test]
fn deletion_owner_identity_survives_renames_and_detects_reused_tags() {
    for expeditionary in [false, true] {
        let mut previous = previous_with_division();
        if expeditionary {
            let TableView::Complete(TableRows::Divisions(rows)) =
                &mut *previous.tables[TableId::Divisions.index()]
            else {
                unreachable!()
            };
            rows[0].expeditionary_owner = ALIAS;
        }
        let mut current = frame();
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *current.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        rows[0].tag = b"Renamed_custom_country".to_vec();
        deletion_rows(
            &mut current,
            vec![DivisionEquipmentDeletionRow {
                country: ALIAS,
                expeditionary_owner: if expeditionary { OWNER } else { 0 },
                ..deletion(EquipmentDestructionCause::DivisionDestroyed)
            }],
        );
        let mut report = Report::default();
        compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(
            report.compared_tables["DivisionEquipmentDeletions.previous_owner"],
            1
        );
    }

    let previous = previous_with_division();
    let mut current = frame();
    let TableView::Complete(TableRows::Countries(rows)) =
        &mut *current.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    rows[0].id = OWNER + 1;
    let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *current.tables[TableId::CountryLinks.index()]
    else {
        unreachable!()
    };
    rows.push(CountryLinkRow {
        id: OWNER + 1,
        canonical_slot: 17,
    });
    deletion_rows(
        &mut current,
        vec![deletion(EquipmentDestructionCause::DivisionDestroyed)],
    );
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("previous_owner"));
}

#[test]
fn unwitnessed_disappearance_is_a_difference_even_when_event_table_is_empty() {
    let current = frame();
    let previous = previous_with_division();
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("without a witness"));
    assert_eq!(
        report.compared_tables["DivisionEquipmentDeletions.disappearances"],
        1
    );
    assert!(report.outcome().is_err());
    let saved = save(
        "units={ division={ id={id=7 type=987654} logical_country=X_custom_4999 strength=1 organisation=1 } }",
    );
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &saved, "", &mut report).unwrap();
    assert!(report.differences.is_empty());
}

#[test]
fn all_transfer_dispositions_witness_disappearance_without_fabricating_amounts() {
    let previous = previous_with_division();
    for disposition in [
        DivisionDeletionDisposition::Returned,
        DivisionDeletionDisposition::Merged,
        DivisionDeletionDisposition::HqTransferred,
    ] {
        let mut current = frame();
        deletion_rows(
            &mut current,
            vec![DivisionEquipmentDeletionRow {
                disposition,
                cause: None,
                ..deletion(EquipmentDestructionCause::Unknown)
            }],
        );
        let mut report = Report::default();
        compare(&current, &mut report).unwrap();
        compare_deletions(&current, &save(""), &mut report).unwrap();
        compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
        assert_eq!(report.compared_tables["EquipmentDestruction"], 0);
        assert_eq!(
            report.compared_tables["DivisionEquipmentDeletions.disappearances"],
            1
        );
        assert!(report.differences.is_empty());
        let saved = save(
            "units={ division={ id={id=7 type=987654} logical_country=X_custom_4999 strength=1 organisation=1 } }",
        );
        let mut report = Report::default();
        compare_deletions(&current, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }
}

fn roster_transfer(disposition: DivisionDeletionDisposition) -> DivisionEquipmentDeletionRow {
    DivisionEquipmentDeletionRow {
        disposition,
        cause: None,
        ..deletion(EquipmentDestructionCause::Unknown)
    }
}

#[test]
fn volunteer_and_exile_transfers_span_intervals_without_false_disappearance() {
    for disposition in [
        DivisionDeletionDisposition::VolunteerTransferred,
        DivisionDeletionDisposition::ExileTransferred,
    ] {
        let previous = previous_with_division();
        let mut transferred = frame();
        deletion_rows(&mut transferred, vec![roster_transfer(disposition)]);
        let mut report = Report::default();
        compare_deletions(&transferred, &save(""), &mut report).unwrap();
        compare_previous_census(&transferred, Some(&previous), &save(""), "", &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(
            report.compared_tables["DivisionEquipmentDeletions.disappearances"],
            1
        );
        let mut history = DivisionHistory::default();
        history.observe(&previous).unwrap();
        history.observe(&transferred).unwrap();
        assert_eq!(history.transfers[&DIVISION], disposition);
        let mut later = frame();
        later.hour = 3;
        let mut report = Report::default();
        compare_previous_census_with_history(
            &later,
            Some(&transferred),
            &save(""),
            "",
            &history,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("1 transfer-held divisions"))
        );
        history.observe(&later).unwrap();
        assert!(history.transfers.contains_key(&DIVISION));
    }
}

#[test]
fn roster_transfer_can_reappear_in_its_interval_and_reappearance_retires_provenance() {
    let previous = previous_with_division();
    let mut current = frame();
    deletion_rows(
        &mut current,
        vec![roster_transfer(
            DivisionDeletionDisposition::VolunteerTransferred,
        )],
    );
    current.tables[TableId::Divisions.index()] =
        previous.tables[TableId::Divisions.index()].clone();
    let saved = save(
        "units={ division={ id={id=7 type=987654} logical_country=X_custom_4999 strength=1 organisation=1 } }",
    );
    let mut report = Report::default();
    compare_deletions(&current, &saved, &mut report).unwrap();
    compare_previous_census(&current, Some(&previous), &saved, "", &mut report).unwrap();
    assert!(report.differences.is_empty());
    let mut history = DivisionHistory::default();
    history.observe(&previous).unwrap();
    history.observe(&current).unwrap();
    assert!(!history.transfers.contains_key(&DIVISION));
    let mut later = frame();
    later.hour = 3;
    let mut report = Report::default();
    compare_previous_census_with_history(
        &later,
        Some(&current),
        &save(""),
        "",
        &history,
        &mut report,
    )
    .unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("without a witness"));
}

#[test]
fn transfer_held_orphan_deletion_retires_active_transfer() {
    let mut held = frame();
    deletion_rows(
        &mut held,
        vec![roster_transfer(
            DivisionDeletionDisposition::ExileTransferred,
        )],
    );
    let mut history = DivisionHistory::default();
    history.observe(&held).unwrap();
    let mut deleted = frame();
    deleted.hour = 3;
    deletion_rows(
        &mut deleted,
        vec![deletion(EquipmentDestructionCause::Unknown)],
    );
    let mut report = Report::default();
    compare_previous_census_with_history(
        &deleted,
        Some(&held),
        &save(""),
        "",
        &history,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("deleted as a transfer-held orphan"))
    );
    assert!(
        !report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("transfer-held divisions"))
    );
    history.observe(&deleted).unwrap();
    assert!(history.transfers.is_empty());
}

#[test]
fn transferred_division_can_return_and_be_wiped_before_next_census() {
    let mut earlier = previous_with_division();
    earlier.hour = 0;
    let mut held = frame();
    held.hour = 1;
    deletion_rows(
        &mut held,
        vec![roster_transfer(
            DivisionDeletionDisposition::VolunteerTransferred,
        )],
    );
    let mut history = DivisionHistory::default();
    history.observe(&earlier).unwrap();
    history.observe(&held).unwrap();
    for cause in [
        EquipmentDestructionCause::DivisionDestroyed,
        EquipmentDestructionCause::DivisionOverrun,
    ] {
        let mut current = frame();
        deletion_rows(&mut current, vec![deletion(cause)]);
        let mut report = Report::default();
        compare_previous_census_with_history(
            &current,
            Some(&held),
            &save(""),
            "",
            &history,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("return and be deleted before a roster census"))
        );
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("1 transfer-held deletion witnesses"))
        );
    }
}

#[test]
fn transfer_history_gaps_are_named_and_counted_without_inventing_reappearance() {
    let mut held = frame();
    held.hour = 0;
    deletion_rows(
        &mut held,
        vec![roster_transfer(
            DivisionDeletionDisposition::ExileTransferred,
        )],
    );
    let mut gap = frame();
    gap.hour = 1;
    *gap.tables[TableId::DivisionEquipmentDeletions.index()] = TableView::Unavailable {
        reason: "missed publication".into(),
    };
    *gap.tables[TableId::Divisions.index()] = TableView::Unavailable {
        reason: "incomplete census".into(),
    };
    let mut history = DivisionHistory::default();
    history.observe(&held).unwrap();
    history.observe(&gap).unwrap();
    let mut previous = frame();
    previous.hour = 2;
    history.observe(&previous).unwrap();
    let mut current = frame();
    current.hour = 3;
    let mut report = Report::default();
    compare_previous_census_with_history(
        &current,
        Some(&previous),
        &save(""),
        "",
        &history,
        &mut report,
    )
    .unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report.limitations.values().flatten().any(|subject| subject
            .contains("1 active transfers; 1 incomplete witness intervals; 1 incomplete censuses"))
    );
}

fn history_replay(branches: Vec<Vec<(DecodedFrame, bool)>>) -> Replay {
    use replay_capture_spool::spool::Record;
    use replay_data::{
        identity::Identity, identity::Roster, identity::SessionStart, identity::Verification,
    };
    use replay_frame::{
        branches::BranchReason, branches::BranchStart, branches::Gap, branches::Parentage,
        frame::FrameEncoder,
    };
    let mut records = vec![Record::SessionStart(SessionStart {
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
    })];
    let mut encoder = FrameEncoder::new();
    for (index, frames) in branches.into_iter().enumerate() {
        let first = &frames[0].0;
        records.push(Record::BranchStart(BranchStart {
            id: first.branch,
            session: 1,
            loaded_hour: None,
            first_hour: first.hour,
            parent: Parentage::Unknown,
            reason: if index == 0 {
                BranchReason::SessionStart
            } else {
                BranchReason::LoadSignal
            },
        }));
        let mut next_hour = first.hour;
        for (mut frame, keyframe) in frames {
            if frame.hour > next_hour {
                records.push(Record::Gap(Gap {
                    branch: frame.branch,
                    first_hour: next_hour,
                    last_hour: frame.hour - 1,
                    reason: "synthetic missed publication".into(),
                }));
            }
            next_hour = frame.hour + 1;
            *frame.tables[TableId::Clock.index()] =
                TableView::Complete(TableRows::Clock(vec![replay_data::rows::ClockRow {
                    hour: frame.hour,
                    ..Default::default()
                }]));
            *frame.tables[TableId::Campaign.index()] =
                TableView::Complete(TableRows::Campaign(vec![replay_data::rows::CampaignRow {
                    id: b"synthetic-history".to_vec(),
                }]));
            records.push(Record::Frame(encoder.encode(frame, keyframe).unwrap()));
        }
    }
    Replay::from_records(recorder_derive::writers(), b"synthetic-history".to_vec(), &[records]).unwrap()
}

#[test]
fn selected_block_history_retains_transfer_across_keyframes_and_gap_then_retires_it() {
    let mut frames = Vec::new();
    for hour in [0, 1, 2, 4, 5, 6, 7] {
        let mut source = frame();
        source.hour = hour;
        if hour == 0 || hour == 6 {
            source.tables[TableId::Divisions.index()] =
                previous_with_division().tables[TableId::Divisions.index()].clone();
        }
        if hour == 1 {
            deletion_rows(
                &mut source,
                vec![roster_transfer(
                    DivisionDeletionDisposition::VolunteerTransferred,
                )],
            );
        }
        frames.push((source, matches!(hour, 2 | 4 | 6)));
    }
    let replay = history_replay(vec![frames]);
    assert_eq!(replay.index.len(), 2);
    let SeekResult::Frame(held) = replay.seek(1, 5).unwrap() else {
        panic!("missing held hour")
    };
    let history = division_history(&replay, &held).unwrap();
    assert_eq!(
        history.transfers[&DIVISION],
        DivisionDeletionDisposition::VolunteerTransferred
    );
    assert!(history.ever_censused.contains(&DIVISION));
    assert_eq!(history.unrecorded_hours, 1);
    let mut report = Report::default();
    compare_deletion_history(&replay, &held, &save(""), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 unrecorded hours"))
    );
    let SeekResult::Frame(after_return) = replay.seek(1, 7).unwrap() else {
        panic!("missing return hour")
    };
    let history = division_history(&replay, &after_return).unwrap();
    assert!(history.transfers.is_empty());
    let mut report = Report::default();
    compare_deletion_history(&replay, &after_return, &save(""), &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("without a witness"));
}

#[test]
fn selected_block_history_excludes_prior_branches_and_future_census_identity() {
    let mut prior = previous_with_division();
    prior.hour = 0;
    let mut transfer = frame();
    transfer.hour = 1;
    deletion_rows(
        &mut transfer,
        vec![roster_transfer(
            DivisionDeletionDisposition::ExileTransferred,
        )],
    );
    let mut frames = Vec::new();
    for hour in 10..=13 {
        let mut source = frame();
        source.branch = 2;
        source.hour = hour;
        if hour == 12 {
            deletion_rows(
                &mut source,
                vec![deletion(EquipmentDestructionCause::DivisionOverrun)],
            );
        } else if hour == 13 {
            source.tables[TableId::Divisions.index()] =
                previous_with_division().tables[TableId::Divisions.index()].clone();
        }
        frames.push((source, hour == 12));
    }
    let replay = history_replay(vec![vec![(prior, false), (transfer, false)], frames]);
    let SeekResult::Frame(current) = replay.seek(2, 12).unwrap() else {
        panic!("missing branch hour")
    };
    let history = division_history(&replay, &current).unwrap();
    assert!(!history.ever_censused.contains(&DIVISION));
    assert!(history.transfers.is_empty());
    let mut report = Report::default();
    compare_deletion_history(&replay, &current, &save(""), &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("before its first complete census"))
    );
}

#[test]
fn combat_wipe_and_overrun_never_censused_have_counted_creation_limits() {
    let mut previous = frame();
    previous.hour -= 1;
    for cause in [
        EquipmentDestructionCause::DivisionDestroyed,
        EquipmentDestructionCause::DivisionOverrun,
    ] {
        let mut current = frame();
        deletion_rows(&mut current, vec![deletion(cause)]);
        let mut report = Report::default();
        compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("before its first complete census"))
        );
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("1 deletion witnesses not previously censused"))
        );
    }
}

#[test]
fn combat_wipe_and_overrun_seen_earlier_but_missing_previous_remain_differences() {
    let mut earlier = previous_with_division();
    earlier.hour = 0;
    let mut previous = frame();
    previous.hour = 1;
    let mut history = DivisionHistory::default();
    history.observe(&earlier).unwrap();
    history.observe(&previous).unwrap();
    for cause in [
        EquipmentDestructionCause::DivisionDestroyed,
        EquipmentDestructionCause::DivisionOverrun,
    ] {
        let mut current = frame();
        deletion_rows(&mut current, vec![deletion(cause)]);
        let mut report = Report::default();
        compare_previous_census_with_history(
            &current,
            Some(&previous),
            &save(""),
            "",
            &history,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("earlier complete census"));
    }
}

#[test]
fn between_census_events_and_transfer_orphan_cleanup_have_named_limits() {
    let mut previous = frame();
    previous.hour -= 1;
    for (disposition, cause) in [
        (
            DivisionDeletionDisposition::Loss,
            Some(EquipmentDestructionCause::DivisionScripted),
        ),
        (
            DivisionDeletionDisposition::Loss,
            Some(EquipmentDestructionCause::Unknown),
        ),
        (
            DivisionDeletionDisposition::Loss,
            Some(EquipmentDestructionCause::DivisionDeleted),
        ),
        (DivisionDeletionDisposition::Returned, None),
        (DivisionDeletionDisposition::Merged, None),
        (DivisionDeletionDisposition::HqTransferred, None),
    ] {
        let mut current = frame();
        let row = DivisionEquipmentDeletionRow {
            disposition,
            cause,
            ..deletion(EquipmentDestructionCause::Unknown)
        };
        let name = row.event_name();
        deletion_rows(&mut current, vec![row]);
        let mut report = Report::default();
        compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains(&format!(
                    "{name}: 1 deletion witnesses not previously censused"
                )))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("orphaned"))
        );
    }
}

#[test]
fn fake_intel_nonforces_are_named_counted_census_exclusions() {
    let current = frame();
    let mut previous = previous_with_division();
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *previous.tables[TableId::Divisions.index()]
    {
        rows[0].reveal_skip = 1;
    }
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["DivisionEquipmentDeletions.disappearances"],
        0
    );
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 fake-intel census entries excluded"))
    );
}

fn second_country(frame: &mut DecodedFrame) {
    if let TableView::Complete(TableRows::CountryLinks(rows)) =
        &mut *frame.tables[TableId::CountryLinks.index()]
    {
        rows.push(CountryLinkRow {
            id: 5000,
            canonical_slot: 18,
        });
    }
    if let TableView::Complete(TableRows::Countries(rows)) =
        &mut *frame.tables[TableId::Countries.index()]
    {
        rows.push(CountryRow {
            id: 5000,
            canonical_slot: 18,
            tag: b"Another_custom_tag".to_vec(),
            ..Default::default()
        });
    }
}

#[test]
fn owner_matches_previous_census_through_aliases_when_expeditionary_status_is_unchanged() {
    let mut current = frame();
    let mut previous = previous_with_division();
    second_country(&mut current);
    second_country(&mut previous);
    if let TableView::Complete(TableRows::Divisions(rows)) =
        &mut *previous.tables[TableId::Divisions.index()]
    {
        rows[0].roster_owner = 5000;
        rows[0].expeditionary_owner = OWNER;
    }
    let correct = DivisionEquipmentDeletionRow {
        country: ALIAS,
        expeditionary_owner: ALIAS,
        ..deletion(EquipmentDestructionCause::DivisionOverrun)
    };
    deletion_rows(&mut current, vec![correct.clone()]);
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(
        report.compared_tables["DivisionEquipmentDeletions.previous_owner"],
        1
    );
    deletion_rows(
        &mut current,
        vec![DivisionEquipmentDeletionRow {
            country: 5000,
            ..correct.clone()
        }],
    );
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("previous_owner"));
    deletion_rows(
        &mut current,
        vec![DivisionEquipmentDeletionRow {
            country: 5000,
            expeditionary_owner: 0,
            ..correct
        }],
    );
    let mut report = Report::default();
    compare_previous_census(&current, Some(&previous), &save(""), "", &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("1 witnesses with changed expeditionary status"))
    );
}

#[test]
fn historical_amount_checks_keep_identity_and_report_absent_coverage() {
    let mut frame = frame();
    let mut historical = amount(EquipmentDestructionCause::LandCombat);
    historical.coverage = Recorded::NotRecorded;
    historical.unavailable_reason = Recorded::NotRecorded;
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![historical]));
    let mut report = Report::default();
    report.record_tables(&frame);
    compare(&frame, &mut report).unwrap();
    assert_eq!(report.compared_tables["EquipmentDestruction"], 1);
    assert!(report.differences.is_empty());
    for field in ["coverage", "unavailable_reason"] {
        assert!(
            report
                .limitations
                .contains_key(&format!("EquipmentDestruction.{field} not recorded"))
        );
    }
}

#[test]
fn independent_hook_failure_retains_signed_amount_checks_and_omitted_causes_marker() {
    let mut frame = frame();
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![
            EquipmentDestructionRow {
                coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
                unavailable_reason: "delete sentinel differs".to_owned().into(),
                ..Default::default()
            },
            EquipmentDestructionRow {
                amount_raw: -7,
                ..amount(EquipmentDestructionCause::LandCombat)
            },
        ]));
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty());
    assert_eq!(report.compared_tables["EquipmentDestruction"], 1);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("coverage: delete sentinel differs"))
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("signed net removals omit supplemental causes"))
    );
    *frame.tables[TableId::DivisionEquipmentDeletions.index()] = TableView::Unavailable {
        reason: "delete sentinel differs".into(),
    };
    assert!(compare_deletions(&frame, &save(""), &mut Report::default()).is_err());
}

#[test]
fn malformed_coverage_markers_refuse_instead_of_becoming_silent_empty_intervals() {
    let mut frame = frame();
    let marker = EquipmentDestructionRow {
        coverage: EquipmentDestructionCoverage::SupplementalUnavailable.into(),
        unavailable_reason: "merge hook unavailable".to_owned().into(),
        ..Default::default()
    };
    *frame.tables[TableId::EquipmentDestruction.index()] =
        TableView::Complete(TableRows::EquipmentDestruction(vec![marker.clone()]));
    let mut report = Report::default();
    compare(&frame, &mut report).unwrap();
    assert_eq!(report.compared_tables["EquipmentDestruction"], 0);
    assert!(
        report
            .limitations
            .values()
            .flatten()
            .any(|subject| subject.contains("coverage: merge hook unavailable"))
    );
    for bad in [
        EquipmentDestructionRow {
            country: OWNER,
            ..marker.clone()
        },
        EquipmentDestructionRow {
            cause: EquipmentDestructionCause::DivisionDestroyed,
            ..marker.clone()
        },
        EquipmentDestructionRow {
            variant: DESIGN,
            ..marker.clone()
        },
        EquipmentDestructionRow {
            amount_raw: 7,
            ..marker.clone()
        },
        EquipmentDestructionRow {
            unavailable_reason: String::new().into(),
            ..marker.clone()
        },
        EquipmentDestructionRow {
            coverage: EquipmentDestructionCoverage::Complete.into(),
            ..marker
        },
    ] {
        *frame.tables[TableId::EquipmentDestruction.index()] =
            TableView::Complete(TableRows::EquipmentDestruction(vec![bad]));
        assert!(compare(&frame, &mut Report::default()).is_err());
    }
}
