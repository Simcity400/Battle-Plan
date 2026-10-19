use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

fn values() -> Defines {
    Defines {
        recruitment: RecruitmentDefines {
            speed: 0,
            noncore_base: 2_000,
            noncore_max: 100_000,
        },
        yearly_growth: 1_500,
        contribution: 10_000,
    }
}
fn fixture() -> DecodedFrame {
    let mut frame = crate::test_fixtures::source(1, 96, Some(0));
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 3,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 5,
                canonical_slot: 3,
            },
            CountryLinkRow {
                id: 8,
                canonical_slot: 8,
            },
            CountryLinkRow {
                id: 9,
                canonical_slot: 3,
            },
        ]));
    *frame.tables[TableId::FactionAccounts.index()] =
        TableView::Complete(TableRows::FactionAccounts(vec![FactionAccountRow {
            account: 3,
            country: 5,
            contribution_subject_gain: Some(0),
            population_growth: Some(0),
            ..Default::default()
        }]));
    *frame.tables[TableId::CountryCounterInputs.index()] =
        TableView::Complete(TableRows::CountryCounterInputs(vec![
            CountryCounterInputRow {
                country: 9,
                // The controlled state belongs to another country. The owned state
                // below must not enter the controller's prediction.
                controlled_states: Some(vec![2]).into(),
                owned_states: Some(vec![1]).into(),
                recruitment_modifiers: Some([None; 4]).into(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::CountryStabilityInputs.index()] =
        TableView::Complete(TableRows::CountryStabilityInputs(vec![
            CountryStabilityInputRow {
                country: 9,
                recruitment_ratio: Some(500_000).into(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::SliceReads.index()] =
        TableView::Complete(TableRows::SliceReads(vec![SliceReadRow {
            table: TableId::CountryStabilityInputs.index() as u32,
            owner: 9,
            sample_hour: 96,
        }]));
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            owner: 9,
            controller: 8,
            manpower_available: 0,
            manpower_locked: 0.into(),
            population: 2_000_000_000.into(),
            ..Default::default()
        },
        StateRow {
            id: 2,
            owner: 8,
            controller: 5,
            manpower_available: 50_000,
            manpower_locked: 950_000.into(),
            population: 1_000_000.into(),
            ..Default::default()
        },
    ]));
    *frame.tables[TableId::StateResourceInputs.index()] =
        TableView::Complete(TableRows::StateResourceInputs(vec![
            StateResourceInputRow {
                state: 2,
                local_manpower: Some(0).into(),
                local_noncore_manpower: Some(0).into(),
                recruitment_add: Some(0).into(),
                recruitment_factor: Some(0).into(),
                ..Default::default()
            },
        ]));
    *frame.tables[TableId::StateCores.index()] =
        TableView::Complete(TableRows::StateCores(vec![StateCoreRow {
            state: 2,
            cores: vec![9],
            ..Default::default()
        }]));
    frame
}
#[test]
fn zero_subject_scaler_proves_zero_without_growth_content_or_state_operands() {
    let mut frame = crate::test_fixtures::source(1, 96, Some(0));
    for gain in [-100_000, -200_000] {
        *frame.tables[TableId::FactionAccounts.index()] =
            TableView::Complete(TableRows::FactionAccounts(vec![FactionAccountRow {
                account: 3,
                contribution_subject_gain: Some(gain),
                population_growth: None,
                ..Default::default()
            }]));
        assert_eq!(predict(&frame, 3, 50_000, values()).unwrap(), 0);
    }
}
#[test]
fn zero_scaler_is_presented_without_a_blanket_refusal_or_supplying_read() {
    let mut frame = fixture();
    *frame.tables[TableId::FactionMembers.index()] =
        TableView::Complete(TableRows::FactionMembers(vec![FactionMemberRow {
            faction: 7,
            country: 9,
            ordinal: 0,
        }]));
    let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *frame.tables[TableId::FactionAccounts.index()]
    else {
        panic!()
    };
    rows[0].contribution_subject_gain = Some(-100_000);
    rows[0].population_growth = None;
    *frame.tables[TableId::SliceReads.index()] = TableView::Complete(TableRows::SliceReads(vec![]));
    let root = std::env::temp_dir().join(format!("faction-zero-manpower-{}", uuid::Uuid::new_v4()));
    let (source, _) = crate::test_fixtures::source_with_content(
        vec![crate::test_fixtures::source(1, 96, Some(0))],
        &root,
    );
    let mut reply = FactionDetailReply::default();
    attach(
        &source,
        1,
        &AtomicBool::new(false),
        &frame,
        7,
        None,
        &mut reply,
    )
    .unwrap();
    assert!(reply.refusals.is_empty(), "{:?}", reply.refusals);
    let result = reply
        .presentations
        .iter()
        .find(|row| row.kind == "Expected manpower contribution")
        .unwrap();
    assert!(result.text.contains("expected manpower contribution 0"));
    assert!(result.text.contains("supplying hour 96"));
    assert!(result.reasons.is_empty());
    drop(source);
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn prediction_uses_controlled_roster_and_controller_aliases_at_one_supply_hour() {
    let mut frame = fixture();
    assert_eq!(predict(&frame, 3, 50_000, values()).unwrap(), 310_000);
    // Membership and account retain distinct raw references to canonical slot 3.
    assert_eq!(supplying_hour(&frame, 3).unwrap(), 96);
    frame.hour = 97;
    assert_eq!(supplying_hour(&frame, 3).unwrap(), 96);
    assert!(
        predict(&frame, 3, 50_000, values())
            .unwrap_err()
            .contains("does not supply")
    );
}
#[test]
fn prediction_refuses_only_reached_missing_operands_and_reuses_core_gate() {
    let mut frame = fixture();
    let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
        &mut *frame.tables[TableId::CountryCounterInputs.index()]
    else {
        panic!()
    };
    rows[0].recruitment_modifiers = Recorded::NotRecorded;
    let TableView::Complete(TableRows::StateResourceInputs(rows)) =
        &mut *frame.tables[TableId::StateResourceInputs.index()]
    else {
        panic!()
    };
    rows[0].local_noncore_manpower = Recorded::NotRecorded;
    assert_eq!(predict(&frame, 3, 50_000, values()).unwrap(), 310_000);
    let TableView::Complete(TableRows::StateCores(rows)) =
        &mut *frame.tables[TableId::StateCores.index()]
    else {
        panic!()
    };
    rows[0].cores.clear();
    assert!(
        predict(&frame, 3, 50_000, values())
            .unwrap_err()
            .contains("controller recruitment modifiers")
    );
    let TableView::Complete(TableRows::FactionAccounts(rows)) =
        &mut *frame.tables[TableId::FactionAccounts.index()]
    else {
        panic!()
    };
    rows[0].population_growth = None;
    assert!(
        predict(&frame, 3, 50_000, values())
            .unwrap_err()
            .contains("population_growth")
    );
    // A disabled selected programme reaches no state or growth input.
    assert_eq!(predict(&frame, 3, 0, values()).unwrap(), 0);
}
#[test]
fn population_rounding_locked_reconciliation_and_contribution_truncation_are_native() {
    assert_eq!(monthly_growth(1_500, 0), 125);
    assert_eq!(monthly_growth(1_500, 100_000), 250);
    assert_eq!(
        contributors(50_000, 950_000, 1_000_000, 5_000, 125, 50_000),
        31
    );
    // Half a person rounds away from zero. Minimum distribution and the held
    // locked pool decide whether the new available person remains recruitable.
    assert_eq!(contributors(10, 390, 400, 2_500, 125, 100_000), 1);
    assert_eq!(contributors(10, 0, 400, 2_500, 125, 100_000), 0);
    // A negative partial product floors instead of truncating toward zero.
    assert_eq!(contributors(100, 0, 400, 2_500, 125, 50_001), -46);
    assert_eq!(expected(31, 10_000, 150_000), 465_000);
}
#[test]
fn noncore_prediction_uses_controller_ratio_and_recorded_noncore_inputs() {
    let mut frame = fixture();
    let TableView::Complete(TableRows::StateCores(rows)) =
        &mut *frame.tables[TableId::StateCores.index()]
    else {
        panic!()
    };
    rows[0].cores = vec![8]; // Owner's core is not the controller's core.
    let TableView::Complete(TableRows::States(rows)) = &mut *frame.tables[TableId::States.index()]
    else {
        panic!()
    };
    rows[1].manpower_available = 1_000;
    rows[1].manpower_locked = 999_000.into();
    assert_eq!(predict(&frame, 3, 100_000, values()).unwrap(), 10_000);
}
