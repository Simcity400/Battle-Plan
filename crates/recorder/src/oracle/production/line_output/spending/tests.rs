use super::*;
use replay::{frame::TableView, rows::TableRows};

fn frame(hour: Hour, progress: i64, spent: i64, units: i64, kind: u8) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow {
            handle: 1,
            country: 11,
            variant: 9,
            kind,
            cost_raw: 300_000,
            produced_raw: progress,
            ..Default::default()
        }]));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(vec![
            ProductionLineOutputRow {
                line: 1,
                country: 11,
                variant: 9,
                units,
                mio_credit: units,
                spent_raw: Recorded::Value(Some(spent)),
                instant_raw: Recorded::Value(Some(0)),
                since_hour: 100,
                observed_since_creation: true,
                exact: true,
                ..Default::default()
            },
        ]));
    frame
}

fn output(frame: &mut DecodedFrame) -> &mut ProductionLineOutputRow {
    let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
        &mut *frame.tables[TableId::ProductionLineOutput.index()]
    else {
        unreachable!()
    };
    &mut rows[0]
}

fn check(frames: &[DecodedFrame]) -> Report {
    let mut check = SpendingCheck::new(100);
    let mut report = Report::default();
    for frame in frames {
        check.observe(frame, &mut report).unwrap();
    }
    check.finish(&mut report);
    report
}

#[test]
fn all_four_surviving_producer_paths_compare_and_an_operand_mutation_is_a_difference() {
    for kind in [
        engine::cequipment_production_line::KIND_MILITARY,
        engine::cequipment_production_line::KIND_NAVAL,
        engine::cequipment_production_line::KIND_RAILWAY_GUN,
        engine::cequipment_production_line::KIND_SHIP_REFIT,
    ] {
        let before = frame(100, 40_000, 70_000, 0, kind);
        let after = frame(124, 90_000, 120_000, 0, kind);
        let report = check(&[before.clone(), after.clone()]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["ProductionLineOutput spending progress self-check"],
            1
        );
        let mut wrong = after;
        output(&mut wrong).spent_raw = Recorded::Value(Some(120_001));
        assert_eq!(check(&[before, wrong]).differences.len(), 1);
    }
}

#[test]
fn signed_input_instant_input_delivery_and_native_progress_wrap_keep_exact_effects() {
    let kind = engine::cequipment_production_line::KIND_MILITARY;
    let before = frame(100, 100_000, 0, 0, kind);
    let mut after = frame(124, 80_000, 0, 1, kind);
    output(&mut after).instant_raw = Recorded::Value(Some(280_000));
    assert!(check(&[before, after]).differences.is_empty());
    let before = frame(100, 40_000, 0, 0, kind);
    let after = frame(124, 30_000, -10_000, 0, kind);
    assert!(check(&[before, after]).differences.is_empty());
    let before = frame(100, i64::MAX, 0, 0, kind);
    let after = frame(124, i64::MIN, 1, 0, kind);
    assert!(check(&[before, after]).differences.is_empty());
}

#[test]
fn missing_suffix_refusal_and_naval_discard_never_pass_as_zero_spending() {
    let kind = engine::cequipment_production_line::KIND_NAVAL;
    let before = frame(100, 250_000, 0, 0, kind);
    let mut blocked = frame(124, 300_000, 100_000, 0, kind);
    output(&mut blocked).mio_credit = 1;
    let report = check(&[before.clone(), blocked]);
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("naval no-deploy"))
    );
    assert_eq!(
        report.compared_tables["ProductionLineOutput spending progress self-check"],
        0
    );
    for value in [Recorded::NotRecorded, Recorded::Value(None)] {
        let mut after = frame(124, 270_000, 0, 0, kind);
        output(&mut after).spent_raw = value;
        let report = check(&[before.clone(), after]);
        assert_eq!(
            report.compared_tables["ProductionLineOutput spending progress self-check"],
            0
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|key| key.contains("spent_raw"))
        );
    }
}
