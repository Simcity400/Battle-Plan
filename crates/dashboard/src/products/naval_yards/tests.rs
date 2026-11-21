use super::*;
use replay_data::rows::{CountryLinkRow, NavalBaseRow, NavalRefitProgressRow, NavalRefitShipRow, NavalRepairQueueRow, ProductionLineRow, TableSchema};
use replay_frame::frame::{Metrics, TableView};

fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn frame() -> DecodedFrame {
    let mut frame = DecodedFrame { branch: 1, hour: 10, tables: std::array::from_fn(|_| replay_frame::frame::not_selected()), metrics: Metrics::default() };
    set::<tables::CountryLinks>(&mut frame, [(11, 7), (12, 8), (9001, 7)].map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).to_vec());
    set::<tables::NavalBases>(&mut frame, [(100, 11), (101, 12), (102, 9001)].map(|(province, country)| NavalBaseRow { province, country, ..Default::default() }).to_vec());
    set::<tables::NavalRepairQueue>(
        &mut frame,
        vec![
            NavalRepairQueueRow { province: 100, ordinal: 0, ship: 5 },
            NavalRepairQueueRow { province: 100, ordinal: 1, ship: 6 },
            NavalRepairQueueRow { province: 100, ordinal: 2, ship: 5 },
            NavalRepairQueueRow { province: 101, ordinal: 0, ship: 9 },
        ],
    );
    set::<tables::ProductionLines>(&mut frame, [(70, 9001), (71, 12), (72, 11)].map(|(handle, country)| ProductionLineRow { handle, country, ..Default::default() }).to_vec());
    set::<tables::NavalRefitProgress>(&mut frame, [70, 71].map(|line| NavalRefitProgressRow { line, progress_raw: 5 }).to_vec());
    set::<tables::NavalRefitShips>(&mut frame, [(1, 70), (2, 71), (3, 70), (4, 72)].map(|(ship, line)| NavalRefitShipRow { ship, line }).to_vec());
    frame
}

#[test]
fn a_countrys_ports_are_the_bases_its_links_hold_and_queues_keep_the_games_order_and_repeats() {
    let frame = frame();
    let raws = raws(&frame, 7);
    assert_eq!(raws, BTreeSet::from([11, 9001]));
    let ports = owned_ports(&frame, &raws);
    assert_eq!(ports, BTreeSet::from([100, 102]));
    let queues = queues(&frame, &ports);
    assert_eq!(queues.get(&100), Some(&vec![(0, 5), (1, 6), (2, 5)]), "a ship queued twice stays twice");
    assert!(!queues.contains_key(&101), "another country's queue is not this one's");
}

#[test]
fn a_refit_line_is_the_owners_by_its_metadata_and_live_only_while_its_progress_row_lives() {
    let lines = owned_lines(&frame(), &BTreeSet::from([11, 9001]));
    assert_eq!(lines.keys().copied().collect::<Vec<_>>(), [70], "line 72 has metadata but no progress row: not live");
    assert_eq!(lines[&70], [1, 3]);
    assert!(finite(5) && !finite(i64::MAX) && !finite(i64::MIN) && !finite(i64::MIN + 1));
}

#[test]
fn a_ships_override_names_it_only_when_set_on_it_or_its_design_names_no_group() {
    let naming = |override_name: &[u8], programmatic: u8| ShipNamingOperands { kind: 1, order: 2, ordered: 1, override_name: override_name.to_vec(), programmatic, equipment: None };
    let grouped: Result<Option<Vec<u8>>, String> = Ok(Some(b"GROUP".to_vec()));
    assert!(override_applies(&naming(b"Bismarck", 0), &grouped));
    assert!(!override_applies(&naming(b"Old name", 0xfe), &grouped), "an override the game made is kept only without a group");
    assert!(override_applies(&naming(b"Old name", 0xfe), &Ok(None)));
    assert!(!override_applies(&naming(b"", 0), &grouped));
    assert_eq!(naming_variant(&ShipNamingOperands { equipment: Some(7), ..naming(b"", 0) }, None), Some(7));
    let model = replay_data::rows::ShipModelRow { variant: 9, ..Default::default() };
    assert_eq!(naming_variant(&naming(b"", 0), Some(&model)), Some(9), "no design of its own: its model's");
    assert_eq!(naming_variant(&naming(b"", 0), None), None);
    assert_eq!(policy_key(true, -3), "NAVY_DOCKYARD_REPAIR_POLICY_-3");
    assert_eq!(policy_key(false, 2), "NAVY_REPAIR_POLICY_2");
}

#[test]
fn refits_whose_progress_is_unavailable_are_missing_by_name_not_refused() {
    assert!(refits_unrecorded(&frame()).is_none());
    let mut frame = frame();
    *frame.tables[TableId::NavalRefitProgress.index()] = TableView::Unavailable { reason: "naval repair writer observation unavailable".into() };
    let reply = refits_unrecorded(&frame).unwrap();
    assert!(reply.lines.is_empty() && !reply.census_complete && reply.ships.is_none() && reply.dockyards.is_none());
    let missing = reply.missing.unwrap();
    assert_eq!(missing, "Refit lines are not recorded at this hour: naval refit progress: not available (naval repair writer observation unavailable)");
}
