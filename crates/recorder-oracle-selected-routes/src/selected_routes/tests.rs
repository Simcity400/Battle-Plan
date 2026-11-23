use super::*;
use replay_data::{
    rows::ControllerAreaNode, rows::CountryWarRow, rows::OrderedEnemyInputs, rows::ProvinceRow,
};

fn fixture() -> (DecodedFrame, Save) {
    let (frame, saved, _, _) = super::super::fixtures::fixture();
    (frame, saved)
}

#[test]
fn ordered_enemies_accept_reserved_zero_repeats_and_native_order_without_save_credit() {
    let (mut frame, _) = fixture();
    *frame.tables[TableId::CountryWars.index()] =
        TableView::Complete(TableRows::CountryWars(vec![
            CountryWarRow {
                country: 0,
                enemies_read: 0,
                enemies: vec![],
                ordered: Some(OrderedEnemyInputs {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    entries: vec![77, 0, 77, 9],
                })
                .into(),
                ..Default::default()
            },
            CountryWarRow {
                country: 77,
                enemies_read: 1,
                enemies: vec![77, 0, 9, 77],
                ordered: Some(OrderedEnemyInputs {
                    sample_hour: replay_data::rows::PacketHour::Host,
                    entries: vec![77, 0, 9, 77],
                })
                .into(),
                ..Default::default()
            },
        ]));
    let mut report = Report::default();
    enemies(&frame, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{report:?}");
    assert!(report.compared_tables.is_empty());
    assert_eq!(report.scalar_checks, 0);
    assert!(
        report
            .limitations
            .keys()
            .any(|reason| reason.contains("not this vector"))
    );

    let TableView::Complete(TableRows::CountryWars(rows)) =
        &mut *frame.tables[TableId::CountryWars.index()]
    else {
        panic!("enemies");
    };
    rows[1].enemies.swap(1, 3);
    let mut report = Report::default();
    enemies(&frame, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("legacy shared vector"));
}

#[test]
fn ordered_enemy_absence_refusal_and_empty_current_packet_are_distinct() {
    let (mut frame, _) = fixture();
    for (field, differences, historical) in [
        (Recorded::NotRecorded, 0, true),
        (Recorded::Value(None), 1, false),
        (
            Some(OrderedEnemyInputs {
                sample_hour: replay_data::rows::PacketHour::Host,
                entries: vec![],
            })
            .into(),
            0,
            false,
        ),
    ] {
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
                country: 0,
                ordered: field,
                ..Default::default()
            }]));
        let mut report = Report::default();
        enemies(&frame, &mut report).unwrap();
        assert_eq!(report.differences.len(), differences, "{report:?}");
        assert_eq!(
            report
                .limitations
                .contains_key("CountryWars.ordered not recorded"),
            historical
        );
        assert!(report.compared_tables.is_empty());
        assert_eq!(report.scalar_checks, 0);
    }
}

#[test]
fn whole_frame_historical_suffixes_are_counted_once_without_comparison_credit() {
    let (mut frame, _) = fixture();
    *frame.tables[TableId::CountryWars.index()] =
        TableView::Complete(TableRows::CountryWars(vec![CountryWarRow {
            country: 0,
            ordered: Recorded::NotRecorded,
            ..Default::default()
        }]));
    let mut report = Report::default();
    report.record_tables(&frame);
    let retired = |report: &Report| {
        report
            .limitations
            .iter()
            .filter(|(reason, _)| {
                ["Provinces.selected_areas", "DivisionMovement.selected"]
                    .iter()
                    .any(|field| reason.contains(field))
            })
            .map(|(reason, labels)| (reason.clone(), labels.len()))
            .collect::<Vec<_>>()
    };
    let gaps = retired(&report);
    let saved = fixture().1;
    compare(
        &frame,
        &saved,
        Err("no geometry in the fixture".into()),
        &mut report,
    );
    assert!(report.differences.is_empty(), "{report:?}");
    assert_eq!(
        report.limitations["CountryWars.ordered not recorded"].len(),
        1
    );
    assert_eq!(retired(&report), gaps);
    assert!(report.compared_tables.is_empty());
    assert_eq!(report.scalar_checks, 0);
}

fn area(id: u32, controller: u32, owner: u32, node: Option<ControllerAreaNode>) -> ProvinceRow {
    ProvinceRow {
        id,
        controller_area: Recorded::Value(Some(controller)),
        owner_area: Recorded::Value(Some(owner)),
        area_node: Recorded::Value(node),
        ..Default::default()
    }
}

fn node(first_land: u32, neighbours: &[u32]) -> Option<ControllerAreaNode> {
    Some(ControllerAreaNode {
        first_land: Some(first_land),
        neighbours: Some(neighbours.to_vec()),
    })
}

fn areas_frame(rows: Vec<ProvinceRow>) -> DecodedFrame {
    let (mut frame, _) = fixture();
    *frame.tables[TableId::Provinces.index()] = TableView::Complete(TableRows::Provinces(rows));
    frame
}

fn consistent() -> Vec<ProvinceRow> {
    vec![
        area(1, 1, 1, node(2, &[3])),
        area(2, 1, 1, None),
        area(3, 3, 1, node(3, &[1])),
        area(4, 0, 0, None),
    ]
}

fn check(rows: Vec<ProvinceRow>) -> Report {
    let geometry = content_reader_map::province_geometry::ProvinceGeometry {
        nonsea_indices: (1..=3).map(|id| (id, id as i32)).collect(),
        ..Default::default()
    };
    let mut report = Report::default();
    areas(&areas_frame(rows), Ok(&geometry), &mut report).unwrap();
    report
}

#[test]
fn recorded_areas_are_checked_for_smallest_member_labels_roots_nodes_and_contacts() {
    let report = check(consistent());
    assert!(report.differences.is_empty(), "{report:?}");
    assert_eq!(report.compared_tables["Provinces selected route areas"], 4);
    for (rows, field) in [
        (
            {
                let mut rows = consistent();
                rows[0].owner_area = Recorded::Value(Some(2));
                rows[1].owner_area = Recorded::Value(Some(2));
                rows[2].owner_area = Recorded::Value(Some(2));
                rows
            },
            "Provinces 1.owner_area",
        ),
        (
            {
                let mut rows = consistent();
                rows[1].area_node = Recorded::Value(node(2, &[3]));
                rows
            },
            "Provinces 2.area_node",
        ),
        (
            {
                let mut rows = consistent();
                rows[0].area_node = Recorded::Value(node(3, &[3]));
                rows
            },
            "Provinces 1.area_node.first_land",
        ),
        (
            {
                let mut rows = consistent();
                rows[0].area_node = Recorded::Value(node(2, &[3, 2]));
                rows
            },
            "Provinces 1.area_node.neighbours[1]",
        ),
        (
            {
                let mut rows = consistent();
                rows[3].controller_area = Recorded::Value(None);
                rows
            },
            "Provinces 4.areas",
        ),
    ] {
        let report = check(rows);
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.starts_with(field)),
            "{field}: {:?}",
            report.differences
        );
    }
}

#[test]
fn division_exile_compares_with_the_saved_truth() {
    use replay_data::rows::DivisionStatusRow;
    let (mut frame, mut saved, _, _) = super::super::fixtures::division_fixture();
    let id = *saved.divisions.keys().next().unwrap();
    let status = |division: u64, exiled: bool| DivisionStatusRow {
        division,
        move_flags: if exiled {
            DivisionStatusRow::MOVE_EXILED
        } else {
            0
        },
        ..Default::default()
    };
    let run = |frame: &DecodedFrame, saved: &Save| {
        let mut report = Report::default();
        exile(frame, saved, &mut report).unwrap();
        report
    };
    for saved_exile in [false, true] {
        saved.divisions.get_mut(&id).unwrap().presentation.exile = saved_exile;
        for recorded in [false, true] {
            *frame.tables[TableId::DivisionStatus.index()] =
                TableView::Complete(TableRows::DivisionStatus(vec![status(id, recorded)]));
            let report = run(&frame, &saved);
            assert_eq!(
                report.differences.len(),
                usize::from(recorded != saved_exile),
                "{report:?}"
            );
            assert_eq!(
                report.compared_tables["DivisionStatus exile saved truth"],
                1
            );
            assert_eq!(report.scalar_checks, 1);
        }
    }
    *frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![status(id + 1, false)]));
    assert_eq!(run(&frame, &saved).differences.len(), 1);
}
