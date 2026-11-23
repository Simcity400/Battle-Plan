use super::*;
use dashboard_follow::live::GroupState;
use replay_data::rows::TableRows;
use replay_frame::frame::{Metrics, TableView, not_selected};

fn counts(slot: u32, region: u32, wings: i64, recorded: &[(usize, i32)]) -> AirMissionCountRow {
    let mut cells = [Recorded::NotRecorded; MISSIONS];
    for &(bit, value) in recorded {
        cells[bit] = Recorded::Value(value);
    }
    let [mission_0, mission_1, mission_2, mission_3, mission_4, mission_5, mission_6, mission_7, mission_8, mission_9, mission_10, mission_11, mission_12, mission_13, mission_14, mission_15, mission_16, mission_17] = cells;
    AirMissionCountRow {
        slot, region, wings, mission_0, mission_1, mission_2, mission_3, mission_4, mission_5, mission_6, mission_7, mission_8, mission_9, mission_10, mission_11,
        mission_12, mission_13, mission_14, mission_15, mission_16, mission_17,
    }
}

fn names() -> Names {
    let mut missions: Vec<Result<String, String>> = (0..MISSIONS).map(|bit| Ok(format!("Mission {bit}"))).collect();
    missions[2] = Err("Localisation AIRWING_MISSION_TYPE_INTERCEPTION is absent from the recorded content stack".to_owned());
    Names {
        catalogue: Err("no content in this test".to_owned()),
        regions: Ok([(10, Ok("North Sea".to_owned())), (11, Err("Region 11 name unsupported".to_owned()))].into()),
        missions,
        strip: Ok(("GFX_strip".to_owned(), None)),
        warnings: Vec::new(),
    }
}

#[test]
fn a_region_s_counts_are_live_cells_and_only_the_missions_it_records_are_named() {
    let rows = [counts(7, 10, 1, &[(0, 24), (2, 0)]), counts(7, 11, 2, &[(1, 50)]), counts(8, 10, 5, &[(0, 9)])];
    let cells = Cells::default();
    let reply = reply(Ok(&rows), 7, Some(40), Ok(Vec::new()), &names(), &cells).unwrap();
    assert_eq!(reply.read_hour, Some(40));
    assert_eq!(reply.missing, None);
    let regions: Vec<_> = reply.regions.iter().map(|region| (region.region.as_str(), region.missions.clone())).collect();
    assert_eq!(regions, [("10", vec!["0".to_owned(), "2".to_owned()]), ("11", vec!["1".to_owned()])], "the owner's rows alone; a zero stays recorded");
    assert_eq!((reply.regions[0].name.as_deref(), reply.regions[1].missing.as_deref()), (Some("North Sea"), Some("Region 11 name unsupported")));
    let missions: Vec<_> = reply.missions.iter().map(|mission| (mission.id.as_str(), mission.name.as_str(), mission.icon.as_ref().map(|icon| icon.frame))).collect();
    assert_eq!(missions, [("0", "Mission 0", Some(0)), ("1", "Mission 1", Some(1)), ("2", "Mission name unavailable", Some(2))]);
    assert!(reply.warnings.iter().any(|warning| warning.contains("AIRWING_MISSION_TYPE_INTERCEPTION")), "{:?}", reply.warnings);
    assert_eq!(reply.theatres, Some(Vec::new()));
    let live = cells.finish();
    let mut frame = DecodedFrame { branch: 1, hour: 41, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    *frame.tables[TableId::AirMissionCounts.index()] = TableView::Complete(TableRows::AirMissionCounts(rows.to_vec()));
    let json = live.json(&live.values(&frame).into_iter().map(GroupState::Values).collect::<Vec<_>>());
    assert_eq!(json["rows"], serde_json::json!(["10", "11"]));
    let kinds: BTreeSet<_> = json["kinds"].as_array().unwrap().iter().map(|kind| kind["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(kinds, ["m0", "m1", "m2", "wings"].map(str::to_owned).into());
    let values: Vec<i64> = json["groups"].as_array().unwrap().iter().flat_map(|group| group["values"].as_array().unwrap().iter().filter_map(|value| value.as_i64())).collect();
    for expected in [24, 0, 50, 1, 2] {
        assert!(values.contains(&expected), "{expected} in {values:?}");
    }
    assert!(!values.contains(&9) && !values.contains(&5), "another owner's counts are not named");
}

#[test]
fn missions_or_theatres_not_recorded_at_the_hour_are_missing_not_empty() {
    let cells = Cells::default();
    let reply = reply(Err(io::Error::other("air mission counts was withheld")), 7, None, Err("Air-theatre owner is unresolved".to_owned()), &names(), &cells).unwrap();
    assert!(reply.regions.is_empty() && reply.missions.is_empty());
    assert!(cells.finish().is_empty(), "no region, no cell");
    assert!(reply.missing.unwrap().contains("air mission counts was withheld"));
    assert_eq!((reply.theatres, reply.theatres_missing.as_deref()), (None, Some("Air-theatre owner is unresolved")));
}

fn theatre(id: u64, owner: u32, ordinal: u32) -> AirTheatreRow {
    AirTheatreRow { id, owner, ordinal, name: Vec::new() }
}

fn wing(id: u64, owner: u32, region: Option<u32>) -> AirWingRow {
    AirWingRow { id, owner, mission_region: region, planes: 10, mission_mask: 1, ..Default::default() }
}

#[test]
fn theatres_are_the_owner_s_in_order_with_their_member_wings_regions() {
    let links: BTreeMap<u32, u32> = [(3, 7), (4, 7), (5, 8)].into();
    let rows = [theatre(100, 3, 2), theatre(101, 4, 1), theatre(102, 5, 0)];
    let groups = [AirTheatreGroupRow { id: 1, theatre: 100, ..Default::default() }, AirTheatreGroupRow { id: 2, theatre: 101, ..Default::default() }, AirTheatreGroupRow { id: 3, theatre: 102, ..Default::default() }];
    let members = [
        AirTheatreMemberRow { group: 1, wing: 50, ordinal: 0 },
        AirTheatreMemberRow { group: 1, wing: 51, ordinal: 1 },
        AirTheatreMemberRow { group: 1, wing: 52, ordinal: 2 },
        AirTheatreMemberRow { group: 3, wing: 53, ordinal: 0 },
    ];
    let wings = [wing(50, 3, Some(11)), wing(51, 3, Some(10)), wing(52, 3, None)];
    let name = |row: &AirTheatreRow| format!("Theatre {}", row.id);
    let found = theatres(&rows, &groups, &members, &wings, &links, 7, name).unwrap();
    let shown: Vec<_> = found.iter().map(|theatre| (theatre.id.as_str(), theatre.regions.clone())).collect();
    assert_eq!(shown, [("101", Vec::new()), ("100", vec!["10".to_owned(), "11".to_owned()])], "game order; an empty theatre is known empty; a wing with no region adds none");
    assert!(theatres(&rows, &groups, &members, &wings[..1], &links, 7, name).unwrap_err().contains("wing is not recorded"));
    assert!(theatres(&rows, &groups[..1], &members, &wings, &links, 7, name).unwrap_err().contains("group is not recorded"));
    assert!(theatres(&rows, &groups, &members, &wings, &[(3, 7)].into(), 7, name).unwrap_err().contains("owner is unresolved"));
}

#[test]
fn a_region_s_wings_are_the_owner_s_registrations_each_once_in_registration_order() {
    let links: BTreeMap<u32, u32> = [(3, 7), (4, 7), (5, 8)].into();
    let wings = [wing(60, 3, Some(10)), wing(61, 4, Some(10)), wing(62, 3, Some(11)), wing(63, 5, Some(10))];
    let registrations = [
        AirRegionalMissionRow { wing: 60, ordinal: 4, ..Default::default() },
        AirRegionalMissionRow { wing: 61, ordinal: 1, ..Default::default() },
        AirRegionalMissionRow { wing: 62, ordinal: 2, ..Default::default() },
        AirRegionalMissionRow { wing: 63, ordinal: 3, ..Default::default() },
    ];
    let found: Vec<u64> = region_wings(&registrations, &wings, &links, 7, 10).unwrap().iter().map(|wing| wing.id).collect();
    assert_eq!(found, [61, 60], "both raw ids of the owner, by ordinal; other regions and owners left out");
    assert_eq!(region_wings(&registrations, &wings[..3], &links, 7, 10).unwrap_err(), "registered wing 63 has no AirWings row", "a registration that does not join withholds the region");
}

#[test]
fn a_region_s_wings_from_its_table_are_the_raw_rows_roster() {
    use recorder_derive_air::air_missions::AirMissions;
    use recorder_derive_framework::{framework::Writers, testing};
    use replay_columnar::manifest::Status;
    use replay_data::rows::AirMissionWingRow;
    let texts: &[&[u8]] = &[b"x"];
    let links: BTreeMap<u32, u32> = [(3, 7), (4, 7), (5, 8)].into();
    let wings = [wing(60, 3, Some(10)), wing(61, 4, Some(10)), wing(62, 3, Some(11)), wing(63, 5, Some(10)), wing(64, 4, Some(10)), wing(65, 3, Some(10))];
    let registrations = [
        AirRegionalMissionRow { wing: 60, ordinal: 4, ..Default::default() },
        AirRegionalMissionRow { wing: 61, ordinal: 1, ..Default::default() },
        AirRegionalMissionRow { wing: 62, ordinal: 2, ..Default::default() },
        AirRegionalMissionRow { wing: 63, ordinal: 3, ..Default::default() },
        AirRegionalMissionRow { wing: 64, ordinal: 0, ..Default::default() },
    ];
    let mut store = testing::store();
    store.begin_hour(0, texts).unwrap();
    for table in [TableId::AirRegionalMissions, TableId::AirWings, TableId::CountryLinks] {
        store.status(table.index(), Status::Complete, true, Some(0), true).unwrap();
    }
    for (&id, &canonical_slot) in &links {
        testing::put(&mut store, TableId::CountryLinks, &CountryLinkRow { id, canonical_slot }, texts).unwrap();
    }
    for row in &wings {
        testing::put(&mut store, TableId::AirWings, row, texts).unwrap();
    }
    for row in &registrations {
        testing::put(&mut store, TableId::AirRegionalMissions, row, texts).unwrap();
    }
    store.settle();
    let mut writers = Writers::new(vec![Box::new(AirMissions::default())]);
    writers.start_run();
    writers.hour(1, 0, true, &mut store).unwrap();
    let view = store.view(TableId::AirMissionWings.index());
    let mut held: Vec<AirMissionWingRow> = view
        .rows()
        .map(|slot| {
            let key = view.key(slot).unwrap();
            AirMissionWingRow { slot: (key >> 96) as u32, region: (key >> 64) as u32, wing: key as u64, ordinal: view.number(0, slot).unwrap() as u32 }
        })
        .collect();
    held.sort_unstable_by_key(|row| (row.slot, row.region, row.wing));
    let mut frame = DecodedFrame { branch: 1, hour: 0, tables: std::array::from_fn(|_| not_selected()), metrics: Metrics::default() };
    frame.tables[TableId::AirMissionWings.index()] = TableView::Complete(TableRows::AirMissionWings(held)).into();
    frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(wings.to_vec())).into();
    for (slot, region) in [(7, 10), (7, 11), (8, 10), (8, 11), (9, 10)] {
        let raw: Vec<u64> = region_wings(&registrations, &wings, &links, slot, region).unwrap().iter().map(|wing| wing.id).collect();
        let table: Vec<u64> = wings_in(&frame, slot, region).unwrap().iter().map(|wing| wing.id).collect();
        assert_eq!(table, raw, "slot {slot}, region {region}");
    }
    assert_eq!(wings_in(&frame, 7, 10).unwrap().iter().map(|wing| wing.id).collect::<Vec<_>>(), [64, 61, 60], "registration order; the unregistered wing 65 left out");
    frame.tables[TableId::AirMissionWings.index()] = TableView::Unavailable { reason: "AirMissions: a registered wing has no air wings row".into() }.into();
    assert!(wings_in(&frame, 7, 10).unwrap_err().contains("has no air wings row"), "the region missing by the writer's reason");
}
