use super::*;
use replay::frame::TableView;
use replay::rows::{AirReferenceWitnessRow, AirRegionalMissionRow, TableRows};

fn set(frame: &mut DecodedFrame, rows: TableRows) {
    let index = rows.table().index();
    *frame.tables[index] = TableView::Complete(rows);
}

#[test]
fn idle_pool_witnesses_match_capture_with_retained_regions_and_registration() {
    for region in [None, Some(7)] {
        for registered in [false, true] {
            let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
            let wing = (1_u64 << 32) | 54;
            let base = (2_u64 << 32) | 63;
            set(
                &mut frame,
                TableRows::AirWings(vec![AirWingRow {
                    id: wing,
                    air_base: base,
                    mission_region: region,
                    mission_mask: 0,
                    ..Default::default()
                }]),
            );
            set(&mut frame, TableRows::AirBases(vec![]));
            set(
                &mut frame,
                TableRows::AirRegionalMissions(if registered {
                    vec![AirRegionalMissionRow { wing, ordinal: 0 }]
                } else {
                    vec![]
                }),
            );
            let witness = AirReferenceWitnessRow {
                handle: base,
                base_kind: Some(1),
                state: Some(1),
                ..Default::default()
            };
            set(
                &mut frame,
                TableRows::AirReferenceWitnesses(vec![witness.clone()]),
            );
            saved.air.bases = vec![installed_content_save::air::AirBase {
                id: base,
                state: Some(1),
                host: None,
                capacity: 0,
                kind: 1,
            }];
            let mut report = Report::default();
            witnesses(&frame, &saved, &mut report).unwrap();
            assert!(report.differences.is_empty(), "{report:?}");
            assert_eq!(report.compared_tables["AirReferenceWitnesses"], 1);

            set(&mut frame, TableRows::AirReferenceWitnesses(vec![]));
            let mut report = Report::default();
            witnesses(&frame, &saved, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                1,
                "a saved idle base needs its witness"
            );
            set(
                &mut frame,
                TableRows::AirReferenceWitnesses(vec![
                    witness.clone(),
                    AirReferenceWitnessRow {
                        handle: base + (1 << 32),
                        ..witness
                    },
                ]),
            );
            let mut report = Report::default();
            witnesses(&frame, &saved, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1);
            assert!(report.differences[0].contains("unprobed"));
        }
    }
}
