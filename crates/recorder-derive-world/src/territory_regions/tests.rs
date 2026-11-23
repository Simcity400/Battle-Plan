use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, put, rows, store},
    framework::{Definitions, ProvinceRegions, Writers},
};
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{CountryLinkRow, ProvinceRow, StateRow};

#[derive(Clone)]
struct World {
    provinces: Vec<(u32, u32, u32, u32, i32)>,
    states: Vec<(u32, u32, u32)>,
    province_status: Status,
}

fn world() -> World {
    World {
        provinces: vec![(100, 1, 10, 20, 5), (101, 1, 10, 20, 0), (102, 1, 10, 10, 1), (103, 0, 0, 0, 3)],
        states: vec![(1, 10, 20), (2, 10, 10)],
        province_status: Status::Complete,
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { &[b"x"] } else { &[] }).unwrap();
    put(store, hour, TableId::CountryLinks, &[CountryLinkRow { id: 10, canonical_slot: 1 }, CountryLinkRow { id: 20, canonical_slot: 2 }], Status::Complete);
    let provinces: Vec<ProvinceRow> = world
        .provinces
        .iter()
        .map(|&(id, state, owner, controller, victory_points)| ProvinceRow { id, state, owner, controller, victory_points, ..Default::default() })
        .collect();
    put(store, hour, TableId::Provinces, &provinces, world.province_status);
    let states: Vec<StateRow> = world.states.iter().map(|&(id, owner, controller)| StateRow { id, owner, controller, ..Default::default() }).collect();
    put(store, hour, TableId::States, &states, Status::Complete);
    store.settle();
}

fn definitions() -> Definitions {
    let areas: Vec<i64> = [0; 100].into_iter().chain([7, 8, 9]).collect();
    Definitions::read_none().with_areas(Ok(areas)).with_regions(Ok(ProvinceRegions::new([(7, &[100, 101, 104][..]), (8, &[102, 104][..])])))
}

fn writers(definitions: Definitions) -> Writers {
    let mut writers = Writers::new(vec![Box::new(TerritoryRegions::default())]);
    writers.define(definitions);
    writers.start_run();
    writers
}

fn row(store: &Store, holder: u32, region: u32) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::TerritoryRegionTotals, TerritoryRegionTotalRow::key_of(holder, region))
}

fn known(values: [i64; 10]) -> Option<Vec<Option<i64>>> {
    Some(values.into_iter().map(Some).collect())
}

fn status(store: &Store) -> Status {
    store.view(TableId::TerritoryRegionTotals.index()).status()
}

#[test]
fn rows_are_each_holders_places_by_region_at_each_read_hour_and_unknown_where_a_link_is_not() {
    let (mut tables, mut derive) = (store(), writers(definitions()));
    let mut hours = vec![world(); 4];
    hours[1].provinces[1].3 = 10;
    hours[2].provinces[1].3 = 10;
    hours[2].states[0].1 = 99;
    hours[3].provinces[1].3 = 10;
    hours[3].states[0].1 = 20;
    hours[3].provinces[2] = (102, 1, 20, 20, 1);
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut tables, hour as i32, world);
        derive.hour(1, hour as i32, hour == 0, &mut tables).unwrap();
        assert_eq!(status(&tables), Status::Complete, "hour {hour}");
        match hour {
            0 => {
                assert_eq!(row(&tables, 1, 7), known([0, 0, 0, 0, 1, 2, 5, 15, 2, 3]), "GER owns two of state 1's three provinces there and controls none");
                assert_eq!(row(&tables, 2, 7), known([2, 5, 15, 2, 3, 0, 0, 0, 0, 1]), "the occupier controls them and their part of the state");
                assert_eq!(row(&tables, 1, 8), known([1, 1, 9, 0, 1, 1, 1, 9, 1, 3]));
                assert_eq!(row(&tables, 2, 8), known([0, 0, 0, 1, 3, 0, 0, 0, 0, 1]), "state 1's third in region 8 is the occupier's to control");
                assert_eq!(row(&tables, 1, NO_REGION), known([0, 0, 0, 1, 1, 0, 0, 0, 1, 1]), "a state no province names is whole and in no region");
                let nobody = row(&tables, NO_COUNTRY, NO_REGION).unwrap();
                assert_eq!(nobody, [Some(1), Some(3), None, Some(0), Some(1), Some(1), Some(3), None, Some(0), Some(1)], "a province past the areas has none, never zero");
                assert_eq!(rows(&tables, TableId::TerritoryRegionTotals).len(), 6);
            }
            1 => {
                assert_eq!(row(&tables, 2, 7), known([1, 5, 7, 2, 3, 0, 0, 0, 0, 1]));
                assert_eq!(row(&tables, 1, 7), known([1, 0, 8, 0, 1, 2, 5, 15, 2, 3]));
                assert_eq!(row(&tables, 1, 8), known([1, 1, 9, 0, 1, 1, 1, 9, 1, 3]), "a row the hour did not move is as it was");
            }
            2 => {
                let held = row(&tables, 1, 8).unwrap();
                assert_eq!(held[..5], [Some(1), Some(1), Some(9), Some(0), Some(1)], "controlled stays known");
                assert!(held[5..].iter().all(Option::is_none), "an owner no link resolves leaves every owned cell unknown: {held:?}");
                assert!(row(&tables, NO_COUNTRY, NO_REGION).unwrap()[5..].iter().all(Option::is_none));
            }
            _ => {
                assert_eq!(row(&tables, 1, 8), None, "GER holds nothing in region 8 under either rule");
                assert_eq!(row(&tables, 2, 8), known([1, 1, 9, 1, 3, 1, 1, 9, 1, 3]));
                assert_eq!(row(&tables, 2, 7), known([1, 5, 7, 2, 3, 0, 0, 0, 2, 3]), "owned is known again");
            }
        }
    }
}

#[test]
fn an_unread_input_or_absent_regions_withhold_the_table_and_absent_areas_only_area() {
    let (mut tables, mut derive) = (store(), writers(definitions()));
    let mut unread = world();
    unread.province_status = Status::Truncated { missing: 1 };
    for (hour, world) in [world(), unread.clone(), world()].iter().enumerate() {
        feed(&mut tables, hour as i32, world);
        derive.hour(1, hour as i32, hour == 0, &mut tables).unwrap();
    }
    assert_eq!(status(&tables), Status::Complete);
    assert_eq!(row(&tables, 2, 7), known([2, 5, 15, 2, 3, 0, 0, 0, 0, 1]), "read whole again after the unread hour");
    let (mut tables, mut derive) = (store(), writers(definitions()));
    feed(&mut tables, 0, &unread);
    derive.hour(1, 0, true, &mut tables).unwrap();
    assert!(matches!(status(&tables), Status::Unavailable { .. }), "{:?}", status(&tables));

    let (mut tables, mut derive) = (store(), writers(Definitions::read_none().with_areas(Ok(vec![0; 103]))));
    feed(&mut tables, 0, &world());
    derive.hour(1, 0, true, &mut tables).unwrap();
    let Status::Unavailable { reason } = status(&tables) else { panic!("{:?}", status(&tables)) };
    let reason = String::from_utf8_lossy(tables.view(TableId::TerritoryRegionTotals.index()).string(reason).unwrap()).into_owned();
    assert!(reason.contains("strategic regions"), "{reason}");

    let regions = ProvinceRegions::new([(7, &[100, 101][..]), (8, &[102][..])]);
    let (mut tables, mut derive) = (store(), writers(Definitions::read_none().with_regions(Ok(regions))));
    feed(&mut tables, 0, &world());
    derive.hour(1, 0, true, &mut tables).unwrap();
    let held = row(&tables, 1, 7).unwrap();
    assert_eq!((held[2], held[7]), (None, None), "areas not held: area unknown under both rules");
    assert_eq!((held[5], held[6], held[8], held[9]), (Some(2), Some(5), Some(2), Some(3)), "the rest stays known");
}

#[test]
fn states_sum_exactly_reduced_and_unknown_past_63_bits() {
    assert_eq!(states_sum(&[]), Some((0, 1)));
    assert_eq!(states_sum(&[(3, 3)]), Some((1, 1)));
    assert_eq!(states_sum(&[(3, 2), (5, 2)]), Some((16, 15)));
    assert_eq!(states_sum(&[(4, 2), (6, 3)]), Some((1, 1)));
    assert_eq!(states_sum(&[(4_294_967_291, 1), (4_294_967_279, 1), (4_294_967_231, 1)]), None);
}

#[test]
fn its_state_is_its_tables_so_a_resume_reads_them_again() {
    let mut writer = TerritoryRegions::default();
    let mut blob = Bytes::new();
    writer.save(&mut blob).unwrap();
    assert!(blob.bytes().is_empty());
    writer.load(&[]).unwrap();
    assert!(writer.rebuild);
    assert!(writer.load(&[1]).is_err());
}
