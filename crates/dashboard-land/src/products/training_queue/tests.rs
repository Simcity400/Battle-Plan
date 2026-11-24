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

#[test]
fn unread_country_names_leave_the_queue_standing_with_the_unnamed_contributor_missing() {
    use super::{TrainingQueue, TrainingQueueParams};
    use dashboard_base::products::{LoadContext, at_hour::{At, AtHour, Owner}};
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use dashboard_follow::live::Cells;
    use dashboard_base::products::Country;
    use dashboard_query::identity::Holders;
    use replay_data::rows::{CountryLinkRow, DeploymentConveyorRow, DeploymentLineRow, DeploymentManpowerRow, TableId, TableRows};
    use replay_frame::frame::{SharedView, TableView};
    let mut frame = source(1, 10, Some(1));
    let sampled = |view: TableView| SharedView::from(view).with_sampled_at(Some(10));
    frame.tables[TableId::CountryLinks.index()] = sampled(TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }])));
    let countries = frame.tables[TableId::Countries.index()].clone();
    let TableView::Complete(rows) = &*countries else { unreachable!() };
    frame.tables[TableId::Countries.index()] = sampled(TableView::Truncated { rows: rows.clone(), missing: 1 });
    frame.tables[TableId::DeploymentConveyors.index()] = sampled(TableView::Complete(TableRows::DeploymentConveyors(vec![DeploymentConveyorRow { handle: 1, country: 9001, template: 5, ..Default::default() }])));
    frame.tables[TableId::DeploymentLines.index()] = sampled(TableView::Complete(TableRows::DeploymentLines(vec![DeploymentLineRow { handle: 10, conveyor: 1, country: 9001, ..Default::default() }])));
    frame.tables[TableId::DeploymentManpower.index()] = sampled(TableView::Complete(TableRows::DeploymentManpower(vec![
        DeploymentManpowerRow { line: 10, side: 0, ordinal: 0, source_country: 9001, amount: 400 },
        DeploymentManpowerRow { line: 10, side: 0, ordinal: 1, source_country: 9002, amount: 100 },
    ])));
    let replay = replay::campaign::Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, vec![source(1, 10, Some(1))])]).unwrap().into_timelines(&Default::default()).unwrap();
    let held = dashboard_base::Source::from_replay(dashboard_base::SourceKey { source_id: "training".into(), revision: "1".into() }, "training.battleplan".into(), std::sync::Arc::new(replay), &std::sync::atomic::AtomicBool::new(false)).unwrap();
    let content = TrainingQueue::context(&held, &LoadContext { content: None }).unwrap();
    let (holders, live, cancelled) = (Holders::default(), Cells::default(), std::sync::atomic::AtomicBool::new(false));
    let at = At { owner: Some(Owner { slot: 7, id: 9001 }), holders: &holders, hour: 10, context: &content, cancelled: &cancelled, live: &live };
    let reply = TrainingQueue::reply(&frame, &TrainingQueueParams { branch_id: "1".into(), country: Country::from("GER") }, &at).expect("the queue stands without every country's name");
    assert_eq!((reply.missing, reply.in_training), (None, Some(1)));
    let division = &reply.orders[0].divisions[0];
    assert_eq!(division.manpower, Some(500));
    assert_eq!(division.contributors.iter().map(|row| (row.tag.as_deref(), row.amount)).collect::<Vec<_>>(), [(Some("GER"), 400), (None, 100)], "the witnessed country is named; the unread one is the missing part");
    assert!(reply.warnings.iter().any(|line| line.starts_with("Country names unavailable")), "{:?}", reply.warnings);
}
