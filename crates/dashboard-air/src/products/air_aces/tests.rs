use super::*;
use replay_data::rows::{AceDeath, AirWingRow, Recorded, TableRows, WingEquipmentRow};
use replay_frame::frame::{Metrics, TableView};

fn dates(runs: &[(i32, i32, Option<i64>)], reads: &[(i32, i32)]) -> Dates {
    let mut fold = Fold::default();
    for &(first, last, alive) in runs {
        fold.run(first, alive, last_read(reads, first, last));
    }
    fold.dates
}

#[test]
fn an_aces_dates_are_its_first_value_and_the_reads_around_its_death() {
    let reads = [(201, 201), (225, 225), (249, 290), (321, 321), (345, 345)];
    assert_eq!(last_read(&reads, 226, 260), Some(260));
    assert_eq!(last_read(&reads, 202, 224), None);
    assert_eq!(last_read(&reads, 300, 400), Some(345));
    let held = dates(&[(100, 200, None), (201, 300, Some(1)), (301, 320, None), (321, 400, Some(0))], &reads);
    assert_eq!(held.first_seen, Some(201));
    assert_eq!(held.deaths, vec![Death { from: 321, last_alive: Some(290), dead_by: Some(321) }]);
    assert_eq!(held.death_at(400), Some(held.deaths[0]));
    let dead = dates(&[(50, 60, Some(0)), (61, 70, None), (71, 80, Some(0))], &[(50, 50), (71, 71)]);
    assert_eq!(dead.deaths, vec![Death { from: 50, last_alive: None, dead_by: None }], "first seen dead: no interval, and one death");
    let twice = dates(&[(10, 19, Some(1)), (20, 29, Some(0)), (30, 39, Some(1)), (40, 49, Some(0))], &[(10, 49)]);
    assert_eq!(twice.deaths.iter().map(|death| (death.from, death.last_alive, death.dead_by)).collect::<Vec<_>>(), [(20, Some(19), Some(20)), (40, Some(39), Some(40))]);
    assert_eq!(twice.death_at(45).map(|death| death.from), Some(40));
    assert_eq!(twice.death_at(25).map(|death| death.from), Some(20));
}

fn frame(hour: i32) -> DecodedFrame {
    DecodedFrame {
        branch: 1,
        hour,
        metrics: Metrics::default(),
        tables: std::array::from_fn(|_| TableView::Unavailable { reason: "Table not selected".into() }.into()),
    }
}

fn history() -> History {
    History {
        aces: [
            (11, Dates { first_seen: Some(100), deaths: Vec::new() }),
            (12, Dates { first_seen: Some(150), deaths: vec![Death { from: 300, last_alive: Some(280), dead_by: Some(300) }] }),
        ]
        .into(),
        slots: [(1, vec![("JAP".to_owned(), 0, 1_000)]), (2, vec![("CHI".to_owned(), 0, 1_000)])].into(),
    }
}

#[test]
fn the_roster_names_and_dates_each_ace_from_its_row_and_history() {
    let mut frame = frame(310);
    *frame.tables[TableId::AirAces.index()] = TableView::Complete(TableRows::AirAces(vec![
        AirAceRow { id: 11, country: 7, assigned_wing: 40, alive: 1, callsign: b"Lightning".to_vec(), ..Default::default() },
        AirAceRow { id: 12, country: 8, alive: 0, death: Some(AceDeath { killer_ace: 11, killer_country: 7 }), ..Default::default() },
    ]));
    *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![CountryLinkRow { id: 7, canonical_slot: 1 }, CountryLinkRow { id: 8, canonical_slot: 2 }]));
    *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![AirWingRow { id: 40, name: Recorded::Value(Some(b"1. Hikodan".to_vec())), ..Default::default() }]));
    *frame.tables[TableId::WingEquipment.index()] = TableView::Complete(TableRows::WingEquipment(vec![
        WingEquipmentRow { wing: 40, ordinal: 1, variant: 902, amount_raw: 0 },
        WingEquipmentRow { wing: 40, ordinal: 0, variant: 901, amount_raw: 0 },
        WingEquipmentRow { wing: 41, ordinal: 0, variant: 999, amount_raw: 0 },
    ]));
    let mut warnings = Vec::new();
    let (missing, aces) = roster(&frame, &history(), None, &mut warnings);
    assert_eq!(missing, None);
    let (alive, dead) = (&aces[0], &aces[1]);
    assert_eq!((alive.tag.as_deref(), alive.callsign.as_deref(), alive.first_seen, alive.death.as_ref()), (Some("JAP"), Some("Lightning"), Some(100), None));
    assert_eq!(alive.name, None, "no content: the name order is unknown, never guessed");
    assert_eq!((alive.wing.as_deref(), alive.wing_name.as_deref()), (Some("40"), Some("1. Hikodan")));
    let aircraft = alive.aircraft.as_ref().expect("the wing's equipment is recorded");
    assert_eq!(aircraft.iter().map(|plane| plane.id.as_str()).collect::<Vec<_>>(), ["901", "902"]);
    assert!(aircraft.iter().all(|plane| plane.name.is_none() && plane.missing.as_deref().is_some_and(|why| why.starts_with("Aircraft names are not recorded at this hour"))));
    assert_eq!((dead.tag.as_deref(), dead.wing.as_ref(), dead.aircraft.as_deref()), (Some("CHI"), None, Some(&[][..])));
    let death = dead.death.as_ref().expect("a dead ace has a death");
    assert_eq!((death.last_alive, death.by_hour, death.killer_ace.as_deref(), death.killer_tag.as_deref()), (Some(280), Some(300), Some("11"), Some("JAP")));
    assert!(warnings.iter().any(|warning| warning.contains("ACE_FULL_NAME content unavailable")));

    let mut renamed = history();
    renamed.slots.insert(1, vec![("JAP".to_owned(), 0, 304), ("MAN".to_owned(), 305, 1_000)]);
    let (_, aces) = roster(&frame, &renamed, None, &mut Vec::new());
    let death = aces[1].death.as_ref().expect("still dead");
    assert_eq!((aces[0].tag.as_deref(), death.seen_dead, death.killer_tag.as_deref()), (Some("MAN"), Some(300), Some("JAP")), "a later tag change does not rewrite who killed it");

    *frame.tables[TableId::WingEquipment.index()] = TableView::Unavailable { reason: "wing equipment was not read".into() }.into();
    let (_, aces) = roster(&frame, &history(), None, &mut Vec::new());
    assert_eq!(aces[0].aircraft, None, "a wing's equipment not recorded is not an empty wing");

    *frame.tables[TableId::AirAces.index()] = TableView::Unavailable { reason: "air aces was not read".into() }.into();
    let (missing, aces) = roster(&frame, &history(), None, &mut Vec::new());
    assert!(aces.is_empty() && missing.is_some_and(|why| why.contains("air aces was not read")));
}
