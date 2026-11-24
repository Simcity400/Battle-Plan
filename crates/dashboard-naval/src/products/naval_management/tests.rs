use super::*;
use replay_data::rows::{
    CountryLinkRow, CountryRow, NavalBaseRow, NavalRefitShipRow, NavalRepairBaseRow, NavalRepairCountryRow, NavalRepairQueueRow, NavalRepairTaskForceRow, ShipRow,
    TableRows, TableSchema, TaskForceRow,
};
use replay_frame::frame::{Metrics, TableView};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 1, metrics: Metrics::default(), tables: std::array::from_fn(|index| TableView::Complete(TableRows::empty(TableId::ALL[index])).into()) };
    set::<tables::Countries>(&mut frame, vec![CountryRow { id: 101, canonical_slot: 7, tag: b"MOD".to_vec(), ..Default::default() }]);
    set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 11, canonical_slot: 7 }, CountryLinkRow { id: 101, canonical_slot: 7 }]);
    frame
}

fn lines(reply: &NavalManagementReply) -> impl Iterator<Item = &NavalManagementLine> {
    reply.sections.iter().flat_map(|section| &section.lines)
}

#[test]
fn a_rejoin_order_shows_only_while_its_destination_task_force_lives() {
    let (task, destination) = (61 | (3_u64 << 32), 61 | (4_u64 << 32));
    let mut frame = frame();
    set::<tables::NavalRepairTaskForces>(
        &mut frame,
        vec![NavalRepairTaskForceRow { task_force: task, merge_after_repair: destination, composition_token: engine_native_naval::naval_management::NO_AI_COMPOSITION_TOKEN, ..Default::default() }],
    );
    set::<tables::TaskForces>(&mut frame, vec![TaskForceRow { id: task, fleet: 1, roster_owner: 101, ..Default::default() }, TaskForceRow { id: destination, fleet: 1, ..Default::default() }]);
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(lines(&reply).any(|line| line.text == "Rejoin task force 4 after repair"));
    set::<tables::TaskForces>(&mut frame, vec![TaskForceRow { id: task, fleet: 2, roster_owner: 101, ..Default::default() }]);
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(!lines(&reply).any(|line| line.text.starts_with("Rejoin task force")), "a retired destination is not rejoined");
}

#[test]
fn a_refit_shows_its_hourly_progress_as_a_live_amount_and_its_daily_line_apart() {
    let (task, ship, line) = (3_u64 << 32, 4_u64 << 32, 5_u64 << 32);
    let mut frame = frame();
    set::<tables::TaskForces>(&mut frame, vec![TaskForceRow { id: task, roster_owner: 101, ..Default::default() }]);
    set::<tables::Ships>(&mut frame, vec![ShipRow { id: ship, task_force: task, ..Default::default() }]);
    set::<tables::NavalRefitShips>(&mut frame, vec![NavalRefitShipRow { ship, line }]);
    set::<tables::NavalRefitProgress>(&mut frame, vec![NavalRefitProgressRow { line, progress_raw: 123_456 }]);
    for production in [TableView::Complete(TableRows::ProductionLines(vec![])), TableView::Unavailable { reason: "synthetic production fault".into() }] {
        *frame.tables[TableId::ProductionLines.index()] = production;
        let cells = Cells::default();
        let reply = project(&frame, 7, None, &cells).unwrap();
        let progress = lines(&reply).find(|item| item.text == "Ship 4 refit progress: ").expect("the progress line");
        assert_eq!(progress.amount.as_ref().map(|amount| (amount.row.as_str(), amount.column.as_str(), amount.raw.as_deref())), Some((line.to_string().as_str(), "progress", Some("123456"))));
        assert!(!cells.finish().is_empty(), "the progress is a live cell");
        let metadata = lines(&reply).find(|item| item.text.starts_with("Last recorded target:")).unwrap();
        assert!(metadata.text.contains("cost: Unavailable"));
    }
    *frame.tables[TableId::ProductionLines.index()] = TableView::Complete(TableRows::ProductionLines(vec![ProductionLineRow { handle: line, cost_raw: 234_567, active_factories: 2, ..Default::default() }]));
    set::<tables::NavalRefitProgress>(&mut frame, vec![NavalRefitProgressRow { line: line + 1, progress_raw: 1 }]);
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(!lines(&reply).any(|item| item.text.starts_with("Ship 4 refit") || item.text.starts_with("Last recorded target:")));
    *frame.tables[TableId::NavalRefitProgress.index()] = TableView::Unavailable { reason: "synthetic progress fault".into() };
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(lines(&reply).any(|item| item.text == "Ship 4 refit progress unavailable" && item.amount.is_none()));
    assert!(lines(&reply).any(|item| item.text == "Last recorded target: None; cost: 2.34567 IC; dockyard allocation: 2 dockyards"));
}

#[test]
fn only_the_owners_ports_show_and_a_queued_ship_without_a_row_has_no_hull() {
    let mut frame = frame();
    set::<tables::NavalRepairCountries>(&mut frame, vec![NavalRepairCountryRow { country: 101, max_factories_for_repair: -1, max_allowed_dockyards_to_repair: 3, num_used_dockyards: 2 }]);
    set::<tables::NavalBases>(&mut frame, vec![NavalBaseRow { province: 9, country: 11, ..Default::default() }, NavalBaseRow { province: 10, country: 12, ..Default::default() }]);
    set::<tables::NavalRepairBases>(&mut frame, vec![NavalRepairBaseRow { province: 9, priority: -3, allocated_dockyards: 2 }, NavalRepairBaseRow { province: 10, priority: 0, allocated_dockyards: 1 }]);
    set::<tables::NavalRepairQueue>(&mut frame, vec![NavalRepairQueueRow { province: 9, ordinal: 0, ship: u64::MAX }]);
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(reply.sections.iter().find(|section| section.key == "budgets").unwrap().lines[1].text.ends_with("-1"));
    let port = reply.sections.iter().find(|section| section.key == "base:9").expect("the owner's port");
    assert_eq!(port.lines[1].amount.as_ref().map(|amount| amount.raw.clone()), Some(None), "no row: no hull, never zero");
    assert!(!reply.sections.iter().any(|section| section.key == "base:10"));
    *frame.tables[TableId::NavalRepairQueue.index()] = TableView::Unavailable { reason: "synthetic queue fault".into() };
    let reply = project(&frame, 7, None, &Cells::default()).unwrap();
    assert!(reply.warnings.iter().any(|line| line.contains("synthetic queue fault")));
    assert!(reply.sections.iter().any(|section| section.key == "budgets"));
    assert_eq!(fixed(i64::MIN), "-92233720368547.75808");
}

