use super::*;
use replay::{
    frame::{Metrics, TABLE_COUNT},
    rows::{
        ArmyOrderPlanRow, InvasionPreparationRow, OrderExecutionInputs, ParadropPreparationInputs,
        Recorded,
    },
};

const GROUP: u64 = (u32::MAX as u64) << 32 | 53;
const INSTANCE: u32 = u32::MAX;

fn fixture(order_type: i32) -> (DecodedFrame, Save, ArmyOrderPlanRow) {
    let (_, mut saved, _, _) = super::super::fixtures::fixture();
    saved.plans.orders.clear();
    saved.plans.orders.insert(
        (GROUP, INSTANCE),
        save::plans::Order {
            order_type,
            has_child: Some(false),
            invasion_transports: (order_type == engine::corder_instance::TYPE_INVASION)
                .then_some(0),
            ..Default::default()
        },
    );
    let frame = DecodedFrame {
        branch: 1,
        hour: 0,
        metrics: Metrics::default(),
        tables: std::array::from_fn::<_, TABLE_COUNT, _>(|_| TableView::Unavailable {
            reason: "fixture".into(),
        })
        .map(Into::into),
    };
    let row = ArmyOrderPlanRow {
        owner_group: GROUP,
        instance: INSTANCE,
        invasion_transports: Recorded::Value(
            (order_type == engine::corder_instance::TYPE_INVASION).then_some(0),
        ),
        execution: Recorded::Value(Some(OrderExecutionInputs {
            route_is_ok: Some(0),
            paradrop: None,
        })),
        ..Default::default()
    };
    (frame, saved, row)
}

fn run(frame: &mut DecodedFrame, saved: &Save, rows: Vec<ArmyOrderPlanRow>) -> Report {
    *frame.tables[TableId::ArmyOrderPlans.index()] =
        TableView::Complete(TableRows::ArmyOrderPlans(rows));
    let mut report = Report::default();
    report.attempt_table(TableId::ArmyOrderPlans, |report| {
        orders(frame, saved, report)
    });
    report
}

#[test]
fn route_truth_is_independent_of_raw_nonboolean_witness_and_type3_progress() {
    for kind in [
        engine::corder_instance::TYPE_INVASION,
        engine::corder_instance::TYPE_PARADROP,
    ] {
        let (mut frame, mut saved, row) = fixture(kind);
        // A saved nonzero time is still outside the type4 capture demand when route_is_ok is zero.
        saved
            .plans
            .orders
            .get_mut(&(GROUP, INSTANCE))
            .unwrap()
            .preparation = Some(Decimal::parse(b"7.5").unwrap());
        for raw in [0_u8, 1, 2, u8::MAX] {
            let expected = saved.plans.orders.get_mut(&(GROUP, INSTANCE)).unwrap();
            expected.route_is_ok = raw != 0;
            let mut actual = row.clone();
            actual.execution = Recorded::Value(Some(OrderExecutionInputs {
                route_is_ok: Some(raw),
                paradrop: (kind == engine::corder_instance::TYPE_PARADROP && raw != 0).then_some(
                    ParadropPreparationInputs {
                        progress: 750_000,
                        terms: None,
                    },
                ),
            }));
            let report = run(&mut frame, &saved, vec![actual.clone()]);
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(report.failed_comparisons.is_empty());
            assert_eq!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("raw byte exceeds one")),
                raw > 1
            );
            saved
                .plans
                .orders
                .get_mut(&(GROUP, INSTANCE))
                .unwrap()
                .route_is_ok = raw == 0;
            let report = run(&mut frame, &saved, vec![actual]);
            assert_eq!(report.differences.len(), 1);
            assert!(report.differences[0].contains("execution.route_is_ok"));
        }
    }
}

#[test]
fn historical_and_current_route_absences_keep_unchanged_comparisons() {
    let (mut frame, saved, row) = fixture(engine::corder_instance::TYPE_PARADROP);
    for (execution, execution_differences, historical) in [
        (Recorded::NotRecorded, 0, true),
        (Recorded::Value(None), 1, false),
        (
            Recorded::Value(Some(OrderExecutionInputs {
                route_is_ok: None,
                paradrop: None,
            })),
            1,
            false,
        ),
    ] {
        let mut actual = row.clone();
        actual.execution = execution;
        actual.blitz = 1;
        let report = run(&mut frame, &saved, vec![actual]);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.differences.len(), execution_differences + 1);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains(".blitz"))
        );
        assert_eq!(
            report
                .limitations
                .contains_key("ArmyOrderPlans.execution not recorded"),
            historical
        );
    }
}

#[test]
fn demanded_paradrop_progress_compares_exactly_with_omitted_zero_and_signed_extremes() {
    let (mut frame, mut saved, row) = fixture(engine::corder_instance::TYPE_PARADROP);
    for (text, expected) in [
        (None, 0_i64),
        (Some("0"), 0),
        (Some("-0.00001"), -1),
        (Some("12.34567"), 1_234_567),
        (Some("-92233720368547.75808"), i64::MIN),
        (Some("92233720368547.75807"), i64::MAX),
    ] {
        let saved_order = saved.plans.orders.get_mut(&(GROUP, INSTANCE)).unwrap();
        saved_order.route_is_ok = true;
        saved_order.preparation = text.map(|text| Decimal::parse(text.as_bytes()).unwrap());
        for (actual_progress, differences) in [
            (Some(expected), 0),
            (Some(expected.wrapping_add(1)), 1),
            (None, 1),
        ] {
            let mut actual = row.clone();
            actual.execution = Recorded::Value(Some(OrderExecutionInputs {
                route_is_ok: Some(1),
                paradrop: actual_progress.map(|progress| ParadropPreparationInputs {
                    progress,
                    terms: None,
                }),
            }));
            let report = run(&mut frame, &saved, vec![actual]);
            assert!(report.failed_comparisons.is_empty());
            assert_eq!(
                report.differences.len(),
                differences,
                "{text:?}: {:?}",
                report.differences
            );
            if differences != 0 {
                assert!(report.differences[0].contains("execution.paradrop.progress"));
            }
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("runtime getter operands"))
            );
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("naval stat weight"))
            );
        }
    }
}

#[test]
fn unsaved_preparation_terms_are_a_limitation_even_when_present() {
    let (mut frame, mut saved, mut row) = fixture(engine::corder_instance::TYPE_PARADROP);
    saved
        .plans
        .orders
        .get_mut(&(GROUP, INSTANCE))
        .unwrap()
        .route_is_ok = true;
    for terms in [
        None,
        Some(replay::rows::ParadropPreparationTerms {
            target: u32::MAX,
            country_term: i64::MIN,
            relation_term: Some(i64::MAX),
            leader_term: None,
            days_term: -1,
        }),
    ] {
        row.execution = Recorded::Value(Some(OrderExecutionInputs {
            route_is_ok: Some(1),
            paradrop: Some(ParadropPreparationInputs { progress: 0, terms }),
        }));
        let report = run(&mut frame, &saved, vec![row.clone()]);
        assert!(report.differences.is_empty());
        assert!(report.limitations.values().any(|subjects| {
            subjects
                .iter()
                .any(|subject| subject == "ArmyOrderPlans.execution.paradrop")
        }));
    }
}

#[test]
fn capture_scope_rejects_preparation_for_unknown_zero_route_or_type3() {
    for (kind, route, differences) in [
        (engine::corder_instance::TYPE_INVASION, Some(1), 1),
        (engine::corder_instance::TYPE_PARADROP, Some(0), 1),
        (engine::corder_instance::TYPE_PARADROP, None, 2),
    ] {
        let (mut frame, mut saved, mut row) = fixture(kind);
        saved
            .plans
            .orders
            .get_mut(&(GROUP, INSTANCE))
            .unwrap()
            .route_is_ok = route.is_some_and(|raw| raw != 0);
        row.execution = Recorded::Value(Some(OrderExecutionInputs {
            route_is_ok: route,
            paradrop: Some(ParadropPreparationInputs {
                progress: 0,
                terms: None,
            }),
        }));
        let report = run(&mut frame, &saved, vec![row]);
        assert_eq!(
            report.differences.len(),
            differences,
            "{:?}",
            report.differences
        );
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("execution.paradrop"))
        );
    }
    let (mut frame, saved, row) = fixture(engine::corder_instance::TYPE_MOVE);
    let report = run(&mut frame, &saved, vec![row.clone()]);
    assert_eq!(report.differences.len(), 1);
    let report = run(
        &mut frame,
        &saved,
        vec![ArmyOrderPlanRow {
            execution: Recorded::Value(None),
            ..row
        }],
    );
    assert!(report.differences.is_empty());
}

#[test]
fn full_owner_and_instance_identity_prevents_cross_group_or_low_word_join() {
    let (mut frame, mut saved, mut row) = fixture(engine::corder_instance::TYPE_PARADROP);
    let second = (7_u64 << 32) | 53;
    saved
        .plans
        .orders
        .get_mut(&(GROUP, INSTANCE))
        .unwrap()
        .route_is_ok = true;
    saved.plans.orders.insert(
        (second, INSTANCE),
        save::plans::Order {
            order_type: engine::corder_instance::TYPE_PARADROP,
            has_child: Some(false),
            ..Default::default()
        },
    );
    row.execution = Recorded::Value(Some(OrderExecutionInputs {
        route_is_ok: Some(1),
        paradrop: Some(ParadropPreparationInputs {
            progress: 0,
            terms: None,
        }),
    }));
    let other = ArmyOrderPlanRow {
        owner_group: second,
        execution: Recorded::Value(Some(OrderExecutionInputs {
            route_is_ok: Some(0),
            paradrop: None,
        })),
        ..row.clone()
    };
    let report = run(&mut frame, &saved, vec![other.clone(), row.clone()]);
    assert!(report.differences.is_empty());
    let report = run(
        &mut frame,
        &saved,
        vec![
            ArmyOrderPlanRow {
                execution: other.execution,
                ..row
            },
            ArmyOrderPlanRow {
                execution: Recorded::Value(Some(OrderExecutionInputs {
                    route_is_ok: Some(1),
                    paradrop: Some(ParadropPreparationInputs {
                        progress: 0,
                        terms: None,
                    }),
                })),
                ..other
            },
        ],
    );
    assert_eq!(report.differences.len(), 2);
}

#[test]
fn duplicate_or_malformed_recorded_identity_fails_instead_of_overwriting() {
    let (mut frame, saved, row) = fixture(engine::corder_instance::TYPE_PARADROP);
    for rows in [
        vec![row.clone(), row.clone()],
        vec![ArmyOrderPlanRow {
            owner_group: 51,
            ..row
        }],
    ] {
        let report = run(&mut frame, &saved, rows);
        assert_eq!(report.failed_comparisons.len(), 1);
        assert!(report.failed_comparisons.contains_key("ArmyOrderPlans"));
    }
}

#[test]
fn invasion_preparations_remain_type3_only_and_reject_duplicate_identities() {
    let (mut frame, mut saved, _) = fixture(engine::corder_instance::TYPE_INVASION);
    saved.plans.orders.insert(
        (GROUP, 1),
        save::plans::Order {
            order_type: engine::corder_instance::TYPE_PARADROP,
            ..Default::default()
        },
    );
    let row = InvasionPreparationRow {
        owner_group: GROUP,
        instance: INSTANCE,
        ..Default::default()
    };
    *frame.tables[TableId::InvasionPreparations.index()] =
        TableView::Complete(TableRows::InvasionPreparations(vec![row.clone()]));
    let mut report = Report::default();
    invasions(&frame, &saved, &mut report).unwrap();
    assert!(report.differences.is_empty());
    *frame.tables[TableId::InvasionPreparations.index()] =
        TableView::Complete(TableRows::InvasionPreparations(vec![
            row.clone(),
            InvasionPreparationRow {
                instance: 1,
                ..row.clone()
            },
        ]));
    let mut report = Report::default();
    invasions(&frame, &saved, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    *frame.tables[TableId::InvasionPreparations.index()] =
        TableView::Complete(TableRows::InvasionPreparations(vec![row.clone(), row]));
    assert!(invasions(&frame, &saved, &mut Report::default()).is_err());
}
