use super::*;
use replay::{frame::TableView, rows::TableRows};

fn fixture(owner: u32, controller: u32) -> (DecodedFrame, Save, save::PartialMapMembership) {
    let (mut frame, mut save, membership, _) = super::super::tests::fixture();
    *frame.tables[TableId::States.index()] =
        TableView::Complete(TableRows::States(vec![StateRow {
            id: 1,
            owner,
            controller,
            manpower_available: -5,
            custom_name: b"kept name".to_vec(),
            ..Default::default()
        }]));
    let saved = save.states.get_mut(&1).unwrap();
    saved.owner = None;
    saved.controller = None;
    saved.manpower_available = Some(-5);
    saved.name = Some(b"kept name".to_vec());
    (frame, save, membership)
}

fn run(frame: &DecodedFrame, save: &Save, membership: &save::PartialMapMembership) -> Report {
    let context = super::super::country_context(frame).unwrap();
    let mut report = Report::default();
    report.attempt("States", |report| {
        compare_states(frame, save, membership, &context.links, report)
    });
    report
}

#[test]
fn nonpositive_owner_class_never_infers_raw_zero_or_controller() {
    for owner in [0, u32::MAX, i32::MIN as u32] {
        for controller in [0, 101, 999, u32::MAX] {
            let (frame, save, membership) = fixture(owner, controller);
            let report = run(&frame, &save, &membership);
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(report.failed_comparisons.is_empty());
            assert_eq!(report.states, 1);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("not raw0 or the exact negative magnitude"))
            );
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("no controller value can be inferred"))
            );
        }
    }
}

#[test]
fn every_positive_raw_owner_fails_the_omission_class_without_a_resolver() {
    for owner in [101, 999, i32::MAX as u32] {
        let (frame, save, membership) = fixture(owner, 999);
        let report = run(&frame, &save, &membership);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("owner positive class"));
        assert!(report.failed_comparisons.is_empty());
    }
}

#[test]
fn nonpositive_states_still_compare_every_saved_name_and_available_word() {
    let (mut frame, mut save, membership) = fixture(0, 999);
    let saved = save.states.get_mut(&1).unwrap();
    saved.manpower_available = Some(-6);
    saved.name = Some(b"different name".to_vec());
    let report = run(&frame, &save, &membership);
    assert_eq!(report.differences.len(), 2);
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("manpower_available"))
    );
    assert!(
        report
            .differences
            .iter()
            .any(|difference| difference.contains("custom_name"))
    );
    let saved = save.states.get_mut(&1).unwrap();
    saved.manpower_available = None;
    saved.name = None;
    let report = run(&frame, &save, &membership);
    assert_eq!(report.differences.len(), 1);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("available is not independently saved"))
    );
    let TableView::Complete(TableRows::States(states)) =
        &mut *frame.tables[TableId::States.index()]
    else {
        unreachable!()
    };
    states[0].custom_name.clear();
    assert!(run(&frame, &save, &membership).differences.is_empty());
}

#[test]
fn positive_saved_state_retains_exact_owner_controller_and_link_rules() {
    let (frame, mut save, membership) = fixture(0, 101);
    save.states.get_mut(&1).unwrap().owner = Some(b"GER".to_vec());
    assert!(
        run(&frame, &save, &membership)
            .differences
            .iter()
            .any(|difference| difference.contains(".owner"))
    );
    let (frame, mut save, membership) = fixture(u32::MAX, 101);
    save.states.get_mut(&1).unwrap().owner = Some(b"GER".to_vec());
    assert!(
        run(&frame, &save, &membership)
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("unresolved country reference"))
    );
    let (frame, mut save, membership) = fixture(101, 0);
    save.states.get_mut(&1).unwrap().owner = Some(b"GER".to_vec());
    assert!(
        run(&frame, &save, &membership)
            .differences
            .iter()
            .any(|difference| difference.contains(".controller"))
    );
}

#[test]
fn a_saved_controller_in_the_omitted_owner_arm_refuses() {
    let (frame, mut save, membership) = fixture(0, 101);
    save.states.get_mut(&1).unwrap().controller = Some(b"GER".to_vec());
    assert!(
        run(&frame, &save, &membership)
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("controller without a positive owner"))
    );
}

#[test]
fn nonpositive_census_missing_duplicate_and_unavailable_states_remain_failures() {
    let (mut frame, save, membership) = fixture(0, 999);
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![]));
    assert!(
        run(&frame, &save, &membership)
            .differences
            .iter()
            .any(|difference| difference.contains("state 1"))
    );
    *frame.tables[TableId::States.index()] = TableView::Complete(TableRows::States(vec![
        StateRow {
            id: 1,
            ..Default::default()
        };
        2
    ]));
    assert!(
        run(&frame, &save, &membership)
            .failed_comparisons
            .values()
            .any(|reason| reason.contains("duplicate state row"))
    );
    *frame.tables[TableId::States.index()] = TableView::Unavailable {
        reason: "fixture unavailable".into(),
    };
    assert!(
        !run(&frame, &save, &membership)
            .failed_comparisons
            .is_empty()
    );
}

#[test]
fn unowned_state_province_census_compares_every_province_and_rejects_positive_owner() {
    let (mut frame, mut save, membership) = fixture(0, 0);
    save.provinces.clear();
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    for row in rows.iter_mut() {
        row.owner = 0;
        row.controller = 0;
    }
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &save, &membership, &context.links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.provinces,
        membership.provinces.keys().filter(|&&id| id != 0).count()
    );
    drop(context);
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[0].owner = 101;
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &save, &membership, &context.links, &mut report).unwrap();
    assert!(report.differences.iter().any(|row| row.contains(".owner")));
    drop(context);
    for raw in [u32::MAX, i32::MIN as u32] {
        let TableView::Complete(TableRows::Provinces(rows)) =
            &mut *frame.tables[TableId::Provinces.index()]
        else {
            unreachable!()
        };
        for row in rows {
            row.owner = raw;
            row.controller = raw;
        }
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        compare_provinces(&frame, &save, &membership, &context.links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.provinces,
            membership.provinces.keys().filter(|&&id| id != 0).count()
        );
    }
    let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    else {
        unreachable!()
    };
    rows[0].controller = 999;
    let context = super::super::country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &save, &membership, &context.links, &mut report).unwrap();
    assert!(
        report
            .differences
            .iter()
            .any(|row| row.contains(".controller"))
    );
    assert_eq!(
        report.provinces,
        membership.provinces.keys().filter(|&&id| id != 0).count()
    );
}
