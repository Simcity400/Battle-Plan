use super::*;
use replay_data::{
    rows::DivisionDeletionDisposition, rows::EquipmentDestructionCause,
    rows::EquipmentDestructionCoverage,
};

#[test]
fn scripted_division_and_stockpile_reductions_never_aggregate_together() {
    let division = producer::Entry {
        country: 4999,
        cause: engine_events::equipment_destruction::Cause::ScriptedEquipmentFraction,
        variant: 0xf00100000046,
        amount_raw: 11,
        ..Default::default()
    };
    let stockpile = producer::Entry {
        cause: engine_events::equipment_destruction::Cause::ScriptedStockpileFraction,
        amount_raw: 17,
        ..division
    };
    let rows = aggregate(&[stockpile, division, stockpile]).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].cause,
        EquipmentDestructionCause::ScriptedEquipmentFraction
    );
    assert_eq!(rows[0].amount_raw, 11);
    assert_eq!(
        rows[1].cause,
        EquipmentDestructionCause::ScriptedStockpileFraction
    );
    assert_eq!(rows[1].amount_raw, 34);
    assert!(deletion_rows(&[stockpile, division]).unwrap().is_empty());
}

#[test]
fn hourly_signed_totals_preserve_gains_and_omit_exact_cancellations() {
    let event = producer::Entry {
        country: 4999,
        cause: engine_events::equipment_destruction::Cause::LandCombat,
        variant: 0xf00100000046,
        amount_raw: 9,
        ..Default::default()
    };
    let gain = producer::Entry {
        amount_raw: -17,
        ..event
    };
    let rows = aggregate(&[event, gain]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].amount_raw, -8);
    assert!(
        aggregate(&[
            event,
            producer::Entry {
                amount_raw: -9,
                ..event
            }
        ])
        .unwrap()
        .is_empty()
    );
    assert_eq!(
        aggregate(&[
            producer::Entry {
                amount_raw: i64::MIN,
                ..event
            },
            producer::Entry {
                amount_raw: -1,
                ..event
            },
        ])
        .unwrap_err(),
        "equipment destruction hourly total overflow"
    );
}

#[test]
fn hourly_groups_preserve_all_owners_causes_designs_and_deletion_witnesses() {
    let event = producer::Entry {
        country: 4999,
        cause: engine_events::equipment_destruction::Cause::Attrition,
        division_handle: 0,
        variant: 0xf00100000046,
        amount_raw: 9,
        ..Default::default()
    };
    let marker = producer::Entry {
        country: 4999,
        cause: engine_events::equipment_destruction::Cause::DivisionDeleted,
        division_handle: 0xf00200004b01,
        variant: 0,
        amount_raw: 0,
        ..Default::default()
    };
    let rows = aggregate(&[
        event,
        producer::Entry {
            country: 1500,
            ..event
        },
        event,
        producer::Entry {
            cause: engine_events::equipment_destruction::Cause::Unknown,
            ..event
        },
        marker,
    ])
    .unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].country, 1500);
    assert_eq!(rows[1].cause, EquipmentDestructionCause::Unknown);
    assert_eq!(rows[2].cause, EquipmentDestructionCause::Attrition);
    assert_eq!(rows[2].amount_raw, 18);
    let deleted = deletion_rows(&[marker, event]).unwrap();
    assert_eq!(deleted.len(), 1);
    assert_eq!(deleted[0].division_handle, marker.division_handle);
    assert_eq!(deleted[0].country, marker.country);
    assert_eq!(deleted[0].disposition, DivisionDeletionDisposition::Loss);
    assert_eq!(
        deleted[0].cause,
        Some(EquipmentDestructionCause::DivisionDeleted)
    );
    assert!(deletion_rows(&[marker, marker]).is_err());
    assert!(
        deletion_rows(&[producer::Entry {
            amount_raw: 1,
            ..marker
        }])
        .is_err()
    );
    assert!(
        aggregate(&[
            producer::Entry {
                amount_raw: i64::MAX,
                ..event
            },
            event
        ])
        .is_err()
    );
    assert!(
        aggregate(&[producer::Entry {
            amount_raw: 0,
            ..event
        }])
        .is_err()
    );
    assert!(aggregate(&[]).unwrap().is_empty());
    assert!(deletion_rows(&[]).unwrap().is_empty());
}

#[test]
fn nonloss_deletion_dispositions_publish_witnesses_without_equipment_totals() {
    let events: Vec<_> = [
        engine_events::equipment_destruction::Disposition::Returned,
        engine_events::equipment_destruction::Disposition::Merged,
        engine_events::equipment_destruction::Disposition::HqTransferred,
        engine_events::equipment_destruction::Disposition::VolunteerTransferred,
        engine_events::equipment_destruction::Disposition::ExileTransferred,
    ]
    .into_iter()
    .enumerate()
    .map(|(index, disposition)| producer::Entry {
        country: 1500,
        cause: engine_events::equipment_destruction::Cause::Unknown,
        disposition,
        expeditionary_owner: 1500,
        division_handle: ((index as u64 + 1) << 32) | 0x4b01,
        ..Default::default()
    })
    .collect();
    assert!(aggregate(&events).unwrap().is_empty());
    let rows = deletion_rows(&events).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.disposition).collect::<Vec<_>>(),
        [
            DivisionDeletionDisposition::Returned,
            DivisionDeletionDisposition::Merged,
            DivisionDeletionDisposition::HqTransferred,
            DivisionDeletionDisposition::VolunteerTransferred,
            DivisionDeletionDisposition::ExileTransferred,
        ]
    );
    assert!(
        rows.iter().all(|row| row.cause.is_none()
            && row.country == 1500
            && row.expeditionary_owner == 1500)
    );
    for event in events {
        assert!(
            deletion_rows(&[producer::Entry {
                amount_raw: 1,
                ..event
            }])
            .is_err()
        );
        assert!(
            deletion_rows(&[producer::Entry {
                variant: 0xf00100000046,
                ..event
            }])
            .is_err()
        );
    }
}

#[test]
fn same_hour_transfer_then_disposal_keeps_the_final_disposition() {
    let transfer = producer::Entry {
        country: 4999,
        division_handle: 0xf00200004b01,
        disposition: engine_events::equipment_destruction::Disposition::VolunteerTransferred,
        ..Default::default()
    };
    let deletion = producer::Entry {
        cause: engine_events::equipment_destruction::Cause::DivisionScripted,
        disposition: engine_events::equipment_destruction::Disposition::Loss,
        ..transfer
    };
    let rows = deletion_rows(&[transfer, transfer, deletion]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].disposition, DivisionDeletionDisposition::Loss);
    assert_eq!(
        rows[0].cause,
        Some(EquipmentDestructionCause::DivisionScripted)
    );
    assert!(deletion_rows(&[deletion, transfer]).is_err());
}

#[test]
fn refused_supplemental_group_marks_observed_signed_totals_incomplete_even_when_empty() {
    for amount_raw in [0, 9, -9] {
        let mut captured = Captured::with_capacities(&1);
        let len = usize::from(amount_raw != 0);
        if len != 0 {
            captured.rows.rows[0] = producer::Entry {
                country: 4999,
                cause: engine_events::equipment_destruction::Cause::LandCombat,
                variant: 0xf00100000046,
                amount_raw,
                ..Default::default()
            };
        }
        captured.rows.outcome = Outcome::Rows { len };
        captured.deletion_unavailable = Some("division_delete_scope sentinel differs");
        let [
            TableView::Complete(TableRows::EquipmentDestruction(rows)),
            TableView::Unavailable { reason },
        ] = captured.tables()
        else {
            panic!("observed signed totals must retain explicit supplemental-unavailable coverage");
        };
        assert_eq!(rows.len(), len + 1);
        assert_eq!(
            rows[0].coverage,
            EquipmentDestructionCoverage::SupplementalUnavailable.into()
        );
        assert_eq!(
            rows[0].unavailable_reason.require("reason").unwrap(),
            &reason
        );
        assert!(reason.contains("sentinel"));
        if len != 0 {
            assert_eq!(
                rows[1].coverage,
                EquipmentDestructionCoverage::Complete.into()
            );
            assert_eq!(rows[1].amount_raw, amount_raw);
            assert_eq!(rows[1].cause, EquipmentDestructionCause::LandCombat);
        }
        let [
            Outcome::Rows { len: outcome_len },
            Outcome::Unavailable { reason },
        ] = captured.outcomes()
        else {
            panic!("independent availability must be visible before conversion");
        };
        assert_eq!(outcome_len, len);
        assert!(reason.contains("sentinel"));
    }
}

#[test]
fn startup_and_branch_partial_intervals_encode_structured_marker_distinct_from_withheld_refusals() {
    let mut partial = Captured::vacant();
    partial.capture_with(|_| Ok(producer::Drain::Partial("reworded startup interval")));
    assert_eq!(
        partial.outcomes(),
        [
            Outcome::Rows { len: 1 },
            Outcome::Unavailable {
                reason: "reworded startup interval"
            },
        ]
    );
    let [
        TableView::Complete(TableRows::EquipmentDestruction(rows)),
        TableView::Unavailable { reason },
    ] = partial.tables()
    else {
        panic!("startup/branch partial interval must emit structured PartialInterval marker");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].key(), 0);
    assert_eq!(rows[0].amount_raw, 0);
    assert_eq!(
        rows[0].coverage,
        EquipmentDestructionCoverage::PartialInterval.into()
    );
    assert_eq!(
        rows[0].unavailable_reason.require("reason").unwrap(),
        "reworded startup interval"
    );
    assert_eq!(reason, "reworded startup interval");

    let mut refused = Captured::vacant();
    refused.capture_with(|_| Err("drain overflowed"));
    assert_eq!(
        refused.outcomes(),
        [
            Outcome::Unavailable {
                reason: "drain overflowed"
            },
            Outcome::Unavailable {
                reason: "drain overflowed"
            },
        ]
    );
    assert!(matches!(
        refused.tables(),
        [
            TableView::Unavailable { .. },
            TableView::Unavailable { .. }
        ]
    ));
}
