use super::{TrainingNeed, comparison_missing, equipment_fill, withheld};

fn need(archetype: &str, need_raw: i64, held_raw: i64) -> TrainingNeed {
    TrainingNeed { archetype: archetype.to_owned(), name: None, need_raw: need_raw.to_string(), held_raw: held_raw.to_string() }
}

#[test]
fn fill_counts_each_type_up_to_its_need() {
    let needs = [need("infantry_equipment", 1_000, 1_500), need("artillery_equipment", 1_000, 500)];
    assert_eq!(equipment_fill(&needs), Some(75_000));
}

#[test]
fn fill_ignores_types_not_needed_and_negative_holdings() {
    let needs = [need("infantry_equipment", 400, -20), need("support_equipment", 0, 900)];
    assert_eq!(equipment_fill(&needs), Some(0));
}

#[test]
fn fill_is_unknown_when_nothing_is_needed() {
    assert_eq!(equipment_fill(&[need("infantry_equipment", 0, 10)]), None);
    assert_eq!(equipment_fill(&[]), None);
}

#[test]
fn the_queue_stands_without_its_manpower_or_equipment_and_held_amounts_are_not_compared_when_equipment_was_not_read_whole() {
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::{DecodedFrame, Metrics, TableView};
    let mut frame = DecodedFrame { branch: 1, hour: 10, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::DeploymentConveyors.index()] = TableView::Complete(TableRows::DeploymentConveyors(Vec::new()));
    *frame.tables[TableId::DeploymentLines.index()] = TableView::Complete(TableRows::DeploymentLines(Vec::new()));
    *frame.tables[TableId::DeploymentManpower.index()] = TableView::Unavailable { reason: "reader fault".into() };
    *frame.tables[TableId::DeploymentEquipment.index()] = TableView::Unavailable { reason: "reader fault".into() };
    assert_eq!(withheld(&frame), None, "orders and divisions in training are witnessed without manpower or equipment");
    *frame.tables[TableId::DeploymentLines.index()] = TableView::Unavailable { reason: "reader fault".into() };
    assert!(withheld(&frame).is_some());
    assert_eq!(comparison_missing(None, Some("Held equipment not read whole"), false).as_deref(), Some("Held equipment not read whole"), "an exact held figure would leave out what was not read");
    assert_eq!(comparison_missing(None, None, true).as_deref(), Some("A held design's equipment type is unresolved"));
    assert_eq!(comparison_missing(None, None, false), None);
}
