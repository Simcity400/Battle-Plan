use super::*;
use replay_data::{rows::ProductionLineOutputRow, rows::Recorded, rows::TableRows};
use replay_frame::frame::TableView;

/// A publication of `lines` in the census and of output rows `(line, ended)`.
fn frame(hour: Hour, read: bool, lines: &[u64], output: &[(u64, bool)]) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        metrics: Default::default(),
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index]))
        })
        .map(Into::into),
    };
    if read {
        frame.metrics.table_ns[TableId::ProductionLines.index()] = 1;
    }
    *frame.tables[TableId::ProductionLines.index()] =
        TableView::Complete(TableRows::ProductionLines(
            lines
                .iter()
                .map(|&handle| ProductionLineRow {
                    handle,
                    ..Default::default()
                })
                .collect(),
        ));
    *frame.tables[TableId::ProductionLineOutput.index()] =
        TableView::Complete(TableRows::ProductionLineOutput(
            output
                .iter()
                .map(|&(line, ended)| ProductionLineOutputRow {
                    line,
                    since_hour: 100,
                    ended,
                    exact: true,
                    purpose: Recorded::Value(OutputPurpose::New.raw()),
                    ..Default::default()
                })
                .collect(),
        ));
    frame
}

fn check(frames: &[DecodedFrame]) -> io::Result<Report> {
    let mut check = CensusCheck::new(100);
    let mut report = Report::default();
    for frame in frames {
        check.observe(frame, &mut report)?;
    }
    check.finish(&mut report);
    Ok(report)
}

#[test]
fn each_publication_pairs_live_output_with_its_census() {
    let report = check(&[
        frame(100, true, &[1, 2], &[(1, false), (2, false)]),
        // Carried: not a publication, whatever its view holds.
        frame(101, false, &[1], &[(3, false)]),
        // Line 2 ended: its row has left the census.
        frame(124, true, &[1], &[(1, false), (2, true)]),
    ])
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(
        report.compared_tables["ProductionLineOutput census self-check"],
        3
    );
    // A census line without output and a live row outside the census.
    let report = check(&[frame(124, true, &[1, 2], &[(1, false), (3, false)])]).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(report.differences[0].starts_with("ProductionLineOutput 2 at hour 124"));
    assert!(report.differences[1].starts_with("ProductionLineOutput 3 at hour 124"));
}

#[test]
fn restarted_observation_duplicates_and_unavailable_reads_are_not_passed() {
    let mut restarted = frame(124, true, &[1], &[(1, false)]);
    let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
        &mut *restarted.tables[TableId::ProductionLineOutput.index()]
    else {
        unreachable!()
    };
    rows[0].since_hour = 124;
    assert!(check(&[restarted]).is_err());
    assert!(check(&[frame(124, true, &[1], &[(1, false), (1, false)])]).is_err());
    let mut unavailable = frame(124, true, &[1], &[(1, false)]);
    *unavailable.tables[TableId::ProductionLines.index()] = TableView::Unavailable {
        reason: "read failed".into(),
    };
    let report = check(&[unavailable]).unwrap();
    assert_eq!(
        report.compared_tables["ProductionLineOutput census self-check"],
        0
    );
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("unavailable at a read"))
    );
}

/// Each live row's purpose must be one its census line's kind produces; an
/// unclassified row is a counted limitation, not a pass.
#[test]
fn live_row_purposes_follow_their_line_kind() {
    use engine_native::cequipment_production_line::{
        KIND_MILITARY, KIND_NAVAL, KIND_RAILWAY_GUN, KIND_SHIP_REFIT,
    };
    let with = |kind: u8, purpose: OutputPurpose| {
        let mut publication = frame(124, true, &[1], &[(1, false)]);
        let TableView::Complete(TableRows::ProductionLines(lines)) =
            &mut *publication.tables[TableId::ProductionLines.index()]
        else {
            unreachable!()
        };
        lines[0].kind = kind;
        let TableView::Complete(TableRows::ProductionLineOutput(rows)) =
            &mut *publication.tables[TableId::ProductionLineOutput.index()]
        else {
            unreachable!()
        };
        rows[0].purpose = Recorded::Value(purpose.raw());
        check(&[publication]).unwrap()
    };
    for (kind, purpose, passes) in [
        (KIND_MILITARY, OutputPurpose::New, true),
        (KIND_MILITARY, OutputPurpose::Converted, true),
        (KIND_MILITARY, OutputPurpose::Refit, false),
        (KIND_NAVAL, OutputPurpose::New, true),
        (KIND_NAVAL, OutputPurpose::Converted, false),
        (KIND_RAILWAY_GUN, OutputPurpose::Converted, false),
        (KIND_SHIP_REFIT, OutputPurpose::Refit, true),
        (KIND_SHIP_REFIT, OutputPurpose::New, false),
    ] {
        let report = with(kind, purpose);
        assert_eq!(
            report.differences.is_empty(),
            passes,
            "{kind} {purpose:?}: {:?}",
            report.differences
        );
    }
    let report = with(KIND_SHIP_REFIT, OutputPurpose::Unknown);
    assert!(report.differences.is_empty());
    assert!(
        report
            .limitations
            .keys()
            .any(|key| key.contains("could not classify"))
    );
}
