use super::*;
use recorder_derive_framework::{
    forces::testing::{cells, rows, store},
    framework::Writers,
};
use engine_executable::modifier_keys as keys;
use replay_columnar::{manifest::Status, store::Store};
use replay_data::rows::{
    CountryLinkRow, CountryProductionModifiersRow, CountryRow, FactionMemberRow, ModifierBreakdownNode, NavalBaseRow, NavalRepairBaseRow, NavalRepairCountryRow, NavalRepairQueueRow, ProductionModifierTree, ProvinceRow,
    Recorded, RelationLinkRow, RowCodec, ShipCounterInputs, ShipModelRow, ShipRow, SubjectRow, TaskForceRow, WarRelationRow,
};

const TEXTS: &[&[u8]] = &[b""];
const FIRST: u64 = 0x0000_0011_0000_0003;
const SECOND: u64 = 0x0000_0012_0000_0003;
const OUTPUT: i64 = 250_000;

fn fill<R: RowCodec>(store: &mut Store, hour: i32, table: TableId, rows: &[R], status: Status) {
    store.status(table.index(), status, true, Some(hour), true).unwrap();
    let keys: Vec<u128> = rows.iter().map(RowCodec::key).collect();
    for (key, _) in recorder_derive_framework::testing::rows(store, table) {
        if !keys.contains(&key) {
            recorder_derive_framework::testing::remove(store, table, key).unwrap();
        }
    }
    for row in rows {
        recorder_derive_framework::testing::put(store, table, row, TEXTS).unwrap();
    }
}

#[derive(Clone)]
struct World {
    used: Vec<(u32, i32)>,
    modifiers: Status,
    allocated: i32,
    queue: Vec<(u32, u64)>,
    ships: Vec<(u64, i64)>,
    province: u32,
    war: Option<i64>,
}

impl Default for World {
    fn default() -> Self {
        Self { used: vec![(10, 2)], modifiers: Status::Complete, allocated: 2, queue: vec![(0, 100), (1, 101)], ships: vec![(100, 50), (101, 50)], province: 5, war: None }
    }
}

fn modifiers() -> CountryProductionModifiersRow {
    let tree = |key: u32| ProductionModifierTree { key, nodes: vec![ModifierBreakdownNode { children: 0, value: 0, name: Vec::new() }], failure: Vec::new() };
    CountryProductionModifiersRow {
        country: 10,
        power_raw: 100_000,
        factory_speed: [0, 0, 0, OUTPUT],
        trees: vec![tree(keys::OUT_OF_POWER_IMPACT_FACTOR), tree(keys::INDUSTRIAL_CAPACITY_DOCKYARD), tree(keys::INDUSTRIAL_CAPACITY_DOCKYARD_POWERED)],
        ..Default::default()
    }
}

fn feed(store: &mut Store, hour: i32, world: &World) {
    store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
    let complete = Status::Complete;
    let countries: Vec<CountryRow> = [(10, 1), (20, 2)].into_iter().map(|(id, slot)| CountryRow { id, canonical_slot: slot, ..Default::default() }).collect();
    fill(store, hour, TableId::Countries, &countries, complete);
    let links: Vec<CountryLinkRow> = [(10, 1), (20, 2)].into_iter().map(|(id, canonical_slot)| CountryLinkRow { id, canonical_slot }).collect();
    fill(store, hour, TableId::CountryLinks, &links, complete);
    let used: Vec<NavalRepairCountryRow> = world.used.iter().map(|&(country, num_used_dockyards)| NavalRepairCountryRow { country, num_used_dockyards, ..Default::default() }).collect();
    fill(store, hour, TableId::NavalRepairCountries, &used, complete);
    fill(store, hour, TableId::CountryProductionModifiers, &[modifiers()], world.modifiers);
    fill(store, hour, TableId::NavalBases, &[NavalBaseRow { province: 5, country: 10, ..Default::default() }], complete);
    fill(store, hour, TableId::NavalRepairBases, &[NavalRepairBaseRow { province: 5, allocated_dockyards: world.allocated, ..Default::default() }], complete);
    let queue: Vec<NavalRepairQueueRow> = world.queue.iter().map(|&(ordinal, ship)| NavalRepairQueueRow { province: 5, ordinal, ship }).collect();
    fill(store, hour, TableId::NavalRepairQueue, &queue, complete);
    let ships: Vec<ShipRow> = world.ships.iter().map(|&(id, strength)| ShipRow { id, task_force: 1000, strength, ..Default::default() }).collect();
    fill(store, hour, TableId::Ships, &ships, complete);
    let model = |ship: u64, variant: u64| ShipModelRow {
        ship,
        variant,
        counter: Some(ShipCounterInputs { maximum_strength: 1000, ..Default::default() }),
        has_criticals: Recorded::Value(Some(0)),
        ..Default::default()
    };
    fill(store, hour, TableId::ShipModels, &[model(100, FIRST), model(101, SECOND)], complete);
    fill(store, hour, TableId::TaskForces, &[TaskForceRow { id: 1000, roster_owner: 10, logical_owner: 10, province: world.province, ..Default::default() }], complete);
    let provinces: Vec<ProvinceRow> = [5, 6].into_iter().map(|id| ProvinceRow { id, owner: 10, controller: 10, ..Default::default() }).collect();
    fill(store, hour, TableId::Provinces, &provinces, complete);
    fill::<FactionMemberRow>(store, hour, TableId::FactionMembers, &[], complete);
    let wars: Vec<WarRelationRow> = world.war.iter().map(|&casualties| WarRelationRow { first_country: 10, second_country: 20, first_casualties: casualties, ..Default::default() }).collect();
    fill(store, hour, TableId::WarRelations, &wars, complete);
    fill::<SubjectRow>(store, hour, TableId::Subjects, &[], complete);
    fill::<RelationLinkRow>(store, hour, TableId::RelationLinks, &[], complete);
    store.settle();
}

fn writers() -> Writers {
    let mut writers = Writers::new(vec![Box::new(DockyardRepairs::default())]);
    writers.start_run();
    writers
}

fn rate(store: &Store, part: u8, design: u64) -> Option<Vec<Option<i64>>> {
    cells(store, TableId::DockyardRepairRates, rate_key(10, part, design))
}

fn part(value: Option<i64>) -> Option<Vec<Option<i64>>> {
    Some(vec![value, None, None, None])
}

#[test]
fn repair_rates_follow_the_bases_selections_and_say_what_is_not_known() {
    let mut hours = vec![World::default(); 5];
    for world in &mut hours[1..] {
        world.ships[1].1 = 1000;
    }
    for world in &mut hours[2..] {
        world.province = 6;
    }
    for world in &mut hours[3..] {
        world.used = vec![(10, 3)];
    }
    hours[4].modifiers = Status::Unavailable { reason: 0 };
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        let total = rate(&store, Rate::TOTAL, 0);
        let (first, second) = (rate(&store, Rate::DESIGN, FIRST), rate(&store, Rate::DESIGN, SECOND));
        let unattributed = rate(&store, Rate::UNATTRIBUTED, 0);
        let expected = match hour {
            0 => (Some(vec![Some(2 * OUTPUT), Some(2), Some(OUTPUT), Some(1)]), part(Some(OUTPUT)), part(Some(OUTPUT)), part(Some(0))),
            1 => (Some(vec![Some(2 * OUTPUT), Some(2), Some(OUTPUT), Some(1)]), part(Some(OUTPUT)), None, part(Some(OUTPUT))),
            2 => (Some(vec![Some(2 * OUTPUT), Some(2), Some(OUTPUT), Some(1)]), None, None, part(Some(2 * OUTPUT))),
            3 => (Some(vec![Some(3 * OUTPUT), Some(3), Some(OUTPUT), Some(0)]), None, None, part(None)),
            _ => (Some(vec![None, Some(3), None, Some(0)]), None, None, part(None)),
        };
        assert_eq!((total, first, second, unattributed), expected, "hour {hour}");
        assert!(cells(&store, TableId::DockyardRepairRates, rate_key(20, Rate::TOTAL, 0)).is_none(), "a country with no dockyard row has none");
    }
}

#[test]
fn no_dockyard_in_use_is_a_zero_rate_and_a_gone_row_takes_its_rates() {
    let mut hours = vec![World::default(); 3];
    hours[1].used = vec![(10, 0)];
    hours[1].allocated = 0;
    hours[2].used = Vec::new();
    let mut store = store();
    let mut writers = writers();
    for (hour, world) in hours.iter().enumerate() {
        feed(&mut store, hour as i32, world);
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        match hour {
            1 => {
                assert_eq!(rate(&store, Rate::TOTAL, 0), Some(vec![Some(0), Some(0), Some(OUTPUT), Some(1)]));
                assert_eq!(rate(&store, Rate::UNATTRIBUTED, 0), part(Some(0)));
                assert_eq!(rate(&store, Rate::DESIGN, FIRST), None);
            }
            2 => assert!(rows(&store, TableId::DockyardRepairRates).is_empty()),
            _ => {}
        }
    }
}

#[test]
fn a_resume_writes_what_a_run_that_never_stopped_writes() {
    let mut hours = vec![World::default(); 6];
    hours[1].ships[0].1 = 1000;
    hours[2].queue.push((2, 102));
    hours[2].ships.push((102, 10));
    hours[3].province = 6;
    hours[4].used = vec![(10, 1)];
    hours[4].allocated = 1;
    hours[5].modifiers = Status::Unavailable { reason: 0 };
    let unbroken = |resume: Option<usize>| {
        let mut store = store();
        let mut writers = writers();
        let mut seen = Vec::new();
        for (hour, world) in hours.iter().enumerate() {
            feed(&mut store, hour as i32, world);
            writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
            seen.push(rows(&store, TableId::DockyardRepairRates));
            if resume == Some(hour) {
                let saved = writers.save().unwrap();
                writers = Writers::new(vec![Box::new(DockyardRepairs::default())]);
                writers.load(&saved, &mut store).unwrap();
            }
        }
        seen
    };
    let whole = unbroken(None);
    for resume in 0..hours.len() - 1 {
        assert_eq!(unbroken(Some(resume)), whole, "resumed after hour {resume}");
    }
}

#[test]
fn only_a_war_coming_or_going_moves_the_relations_the_bases_read() {
    let hours = [Some(10), Some(25), Some(25), None];
    let mut store = store();
    let mut writers = writers();
    for (hour, war) in hours.into_iter().enumerate() {
        feed(&mut store, hour as i32, &World { war, ..World::default() });
        let moved = inputs::relations_moved(&Hour::new(&store, 1, hour as i32, hour == 0, hour == 0, &[]));
        if hour > 0 {
            assert_eq!(moved, hour == 3, "hour {hour}");
        }
        writers.hour(1, hour as i32, hour == 0, &mut store).unwrap();
        assert_eq!(rate(&store, Rate::TOTAL, 0), Some(vec![Some(2 * OUTPUT), Some(2), Some(OUTPUT), Some(1)]), "hour {hour}");
    }
}

#[test]
fn rate_keys_are_the_row_codec_s() {
    let key = rate_key(u32::MAX, Rate::DESIGN, u64::MAX);
    assert_eq!(key, Rate { country: u32::MAX, part: Rate::DESIGN, design: u64::MAX, ..Default::default() }.key());
    assert_eq!(rate_parts(key), (u32::MAX, Rate::DESIGN, u64::MAX));
}
