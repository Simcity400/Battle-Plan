use std::{collections::HashSet, sync::Arc};
use super::{Admits, NavalCounters, TABLES, Waters};
use viewer_layer_core::{layers::{Layer, View}, test_support::recorded::{column, frames_until, record}, playhead_host::{MapSide, open, units::Units}};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::{Field, NativeValue, Opaque, ShipCounterInputs, TableId};
use viewer_counter_kit::type_tint;
use viewer_counters_naval::naval_members::{NavalMember, Port, Sea};
use viewer_gpu::camera::Placement;
use viewer_unit_reads::{counter_move::{Position, UnitRoute}, task_force_visual};
use viewer_foundation::{perspective::{Choice, Country, Vision}, unit_rules::SHIP_IN_PORT};
use viewer_map_modes::map_mode::{MapMode, Stock};

struct Shore {
    land: HashSet<u32>,
}

impl Waters for Shore {
    fn land(&self, province: u32) -> bool {
        self.land.contains(&province)
    }

    fn region(&self, _province: u32) -> Option<u32> {
        None
    }

    fn place(&self, province: u32, route: &UnitRoute<'_>) -> Result<task_force_visual::Placement, &'static str> {
        let kind = if self.land(province) && route.display_path().is_empty() { SHIP_IN_PORT } else { 0 };
        Ok(task_force_visual::Placement { kind, position: Position::still([province as f32, 0.0, 0.0]), animator: None, unplaced_route: None })
    }

    fn port(&self, province: u32) -> Option<[f32; 3]> {
        self.land(province).then_some([province as f32, 1.0, 0.0])
    }

    fn icons_cutoff(&self) -> f32 {
        f32::INFINITY
    }

    fn admits(&self, _placement: Placement, _viewport: [u32; 2]) -> Option<Admits> {
        None
    }
}

fn put(store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
}

fn insert(store: &mut Store, table: TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn counter(maximum: i64) -> Vec<u8> {
    let inputs = Some(ShipCounterInputs { maximum_strength: maximum, maximum_organisation: maximum, ..Default::default() });
    let Field::Opaque(Opaque::Native(value)) = inputs.field() else { panic!("counter operands are opaque") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn force(store: &mut Store, slot: u32, id: u64, owner: u32, fleet: u64, province: u32) {
    insert(store, TableId::TaskForces, slot, u128::from(id));
    put(store, TableId::TaskForces, slot, "fleet", Cell::U64(fleet));
    for (field, value) in [("roster_owner", owner), ("logical_owner", owner), ("expeditionary_owner", 0), ("province", province), ("mission", 0)] {
        put(store, TableId::TaskForces, slot, field, Cell::U32(value));
    }
    put(store, TableId::TaskForces, slot, "route_count", Cell::I32(0));
    put(store, TableId::TaskForces, slot, "combat_references", Cell::I32(0));
    insert(store, TableId::TaskForceMapMotion, slot, u128::from(id));
    put(store, TableId::TaskForceMapMotion, slot, "target", Cell::U32(0));
    put(store, TableId::TaskForceMapMotion, slot, "combat_role", Cell::U8(0));
    put(store, TableId::TaskForceMapMotion, slot, "path", Cell::U32List(&[]));
    put(store, TableId::TaskForceMapMotion, slot, "alternate_path", Cell::U32List(&[]));
    insert(store, TableId::TaskForceCounterInputs, slot, u128::from(id));
    put(store, TableId::TaskForceCounterInputs, slot, "is_in_regions", Cell::U8(0));
    put(store, TableId::TaskForceCounterInputs, slot, "bombardment_region", Cell::U32(0));
}

fn ship(store: &mut Store, slot: u32, id: u64, force: u64, strength: i64) {
    insert(store, TableId::Ships, slot, u128::from(id));
    put(store, TableId::Ships, slot, "task_force", Cell::U64(force));
    put(store, TableId::Ships, slot, "strength", Cell::I64(strength));
    put(store, TableId::Ships, slot, "organisation", Cell::I64(strength));
    insert(store, TableId::ShipModels, slot, u128::from(id));
    put(store, TableId::ShipModels, slot, "counter", Cell::Opaque(&counter(100)));
}

fn order(store: &mut Store, slot: u32, province: u32, mixed: &[u64], naval: &[u64]) {
    if store.table(TableId::ProvinceUnitOrder.index()).unwrap().slot(u128::from(province)).is_none() {
        insert(store, TableId::ProvinceUnitOrder, slot, u128::from(province));
    }
    put(store, TableId::ProvinceUnitOrder, slot, "mixed", Cell::U64List(mixed));
    put(store, TableId::ProvinceUnitOrder, slot, "naval", Cell::U64List(naval));
}

fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        insert(store, TableId::CountryLinks, slot, key);
        put(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot));
        insert(store, TableId::Countries, slot, key);
        put(store, TableId::Countries, slot, "canonical_slot", Cell::U32(slot));
        put(store, TableId::Countries, slot, "peace_conference_participant", Cell::U8(0));
    }
    force(store, 0, 100, 10, 1, 5);
    force(store, 1, 101, 11, 2, 5);
    force(store, 2, 102, 10, 1, 7);
    ship(store, 0, 1000, 100, 50);
    ship(store, 1, 1001, 100, 30);
    ship(store, 2, 1002, 102, 70);
    order(store, 0, 5, &[101, 100], &[]);
    order(store, 1, 7, &[], &[102]);
    insert(store, TableId::NavalBases, 0, 7);
    insert(store, TableId::FleetCounterInputs, 0, 1);
    for field in ["red", "green", "blue", "alpha"] {
        put(store, TableId::FleetCounterInputs, 0, field, Cell::U32(1));
    }
    put(store, TableId::FleetCounterInputs, 0, "task_forces", Cell::U64List(&[100, 102]));
}

fn campaign(name: &str, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> Units {
    record(name, 400, |at, store| {
        for table in TABLES.into_iter().chain([TableId::TaskForceMissions]) {
            store.status(table.index(), status(at, table), true, Some(at as i32), true).unwrap();
        }
        if at == 0 {
            world(store);
        }
        hour(at, store);
    })
}

fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

fn layer_seeing_all() -> NavalCounters {
    let layer = NavalCounters::new(Arc::new(Shore { land: HashSet::from([7]) }));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer.set_relations(Arc::new(type_tint::Index::default()));
    layer
}

fn side_with(units: Units, layer: NavalCounters, view: View) -> MapSide {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], view).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn side_for(units: Units) -> MapSide {
    side_with(units, layer_seeing_all(), View { fog: false, ..View::default() })
}

fn reach(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour + 3);
        let frame = side.frame(head).unwrap();
        if !side.composing() && frame.applied.is_some_and(|applied| applied >= hour) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached hour {hour}");
}

fn layer(side: &MapSide) -> &NavalCounters {
    side.parts().unwrap().1[0].any().downcast_ref::<NavalCounters>().unwrap()
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn member(side: &MapSide, id: u64) -> Option<NavalMember> {
    layer(side).members().find(|member| member.fields.id == id).cloned()
}

#[test]
fn a_task_force_changed_in_a_step_is_marked_once_and_settles_to_the_composed_value() {
    let units = campaign("naval once", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::TaskForces, 0, "mission", Cell::U32(8));
            put(store, TableId::Ships, 1, "strength", Cell::I64(40));
            put(store, TableId::TaskForceCounterInputs, 0, "bombardment_region", Cell::U32(3));
        }
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let first = member(&side, 100).unwrap();
    assert_eq!(first.sea, Sea::Source { kind: 0, ordinal: 1, moving: false }, "its place in the mixed array, not id order");
    assert_eq!(member(&side, 101).unwrap().sea, Sea::Source { kind: 0, ordinal: 0, moving: false });
    assert_eq!(first.unit.map(|unit| (unit.slot, unit.ships, unit.strength)), Some((0, Some(2), 80)));
    assert!(first.ratio.is_some(), "every ship has counter operands");
    assert_eq!(member(&side, 102).unwrap().port, Port::In(0), "docked at its base");
    assert_eq!(layer(&side).fleets().get(&1).map(|fleet| fleet.reserve), Some(Some(false)));
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).marked() - marked, 1, "three tables' cells of one task force mark it once");
    settled_equals_composed(&mut side);
    let changed = member(&side, 100).unwrap();
    assert_eq!(changed.unit.map(|unit| unit.strength), Some(90));
    assert_eq!(changed.fields.mission, 8);
    assert_eq!(layer(&side).fleets().get(&1).map(|fleet| fleet.reserve), Some(Some(true)), "its head's mission makes fleet 1 a reserve fleet");
}

#[test]
fn a_task_force_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = campaign("naval twice", |_, _| Status::Complete, |at, store| match at {
        200 => put(store, TableId::Ships, 0, "strength", Cell::I64(10)),
        201 => {
            put(store, TableId::TaskForces, 1, "province", Cell::U32(6));
            order(store, 0, 5, &[100], &[]);
            order(store, 2, 6, &[101], &[]);
        }
        202 => put(store, TableId::Ships, 0, "strength", Cell::I64(20)),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let settles = side.hold_settles();
    reach(&mut side, 204);
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 100).unwrap().unit.map(|unit| unit.strength), Some(50));
    let moved = member(&side, 101).unwrap();
    assert_eq!((moved.fields.province, moved.sea), (6, Sea::Source { kind: 0, ordinal: 0, moving: false }));
    assert_eq!(member(&side, 100).unwrap().sea, Sea::Source { kind: 0, ordinal: 0, moving: false });
}

#[test]
fn a_task_force_deleted_and_another_inserted_in_its_slot_updates_every_array_naming_them() {
    let units = campaign("naval slots", |_, _| Status::Complete, |at, store| match at {
        200 => {
            for table in [TableId::TaskForces, TableId::TaskForceMapMotion, TableId::TaskForceCounterInputs] {
                store.table(table.index()).unwrap().delete(0).unwrap();
            }
            for slot in [0, 1] {
                store.table(TableId::Ships.index()).unwrap().delete(slot).unwrap();
                store.table(TableId::ShipModels.index()).unwrap().delete(slot).unwrap();
            }
            order(store, 0, 5, &[101], &[]);
        }
        201 => {
            force(store, 0, 103, 11, 2, 5);
            ship(store, 0, 1003, 103, 25);
            order(store, 0, 5, &[103, 101], &[]);
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert!(member(&side, 100).is_none(), "the deleted task force is gone");
    assert_eq!(layer(&side).fleets().get(&1).map(|fleet| fleet.reserve), Some(None), "fleet 1's head has no row: its reserve test is unknown");
    reach(&mut side, 201);
    settled_equals_composed(&mut side);
    let new = member(&side, 103).unwrap();
    assert_eq!((new.sea, new.unit.map(|unit| (unit.slot, unit.ships, unit.strength))), (Sea::Source { kind: 0, ordinal: 0, moving: false }, Some((1, Some(1), 25))));
    assert_eq!(member(&side, 101).unwrap().sea, Sea::Source { kind: 0, ordinal: 1, moving: false });
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = campaign(
        "naval partial",
        |at, table| match (at, table) {
            (240..=247, TableId::TaskForceMapMotion) => Status::Truncated { missing: 1 },
            (240..=247, TableId::Ships) => Status::Truncated { missing: 1 },
            (300..=307, TableId::CountryLinks) => Status::NotRecorded,
            _ => Status::Complete,
        },
        |_, _| {},
    );
    let mut side = side_for(units);
    reach(&mut side, 243);
    settled_equals_composed(&mut side);
    let at_sea = member(&side, 100).unwrap();
    assert_eq!(at_sea.sea, Sea::Refused("task-force motion census incomplete"), "no route census, no place");
    assert_eq!((at_sea.ratio, at_sea.unit.and_then(|unit| unit.ships)), (None, None), "a partial ship census leaves counts and bars unknown");
    assert_eq!(member(&side, 102).unwrap().port, Port::Undecided("task-force motion census incomplete"));
    assert!(layer(&side).ports().undecided.contains_key(&7), "its base's items are undecided");
    reach(&mut side, 303);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("country link")), "{:?}", layer(&side).refused());
    assert_eq!(layer(&side).members().count(), 0, "a refused layer keeps nothing");
    reach(&mut side, 320);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 102).unwrap().port, Port::In(0));
    assert!(layer(&side).ports().undecided.is_empty());
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = campaign("naval hidden", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::TaskForces, 0, "mission", Cell::U32(8));
            put(store, TableId::Ships, 1, "strength", Cell::I64(40));
        }
    });
    let resources = View { fog: false, map_mode: MapMode::Stock(Stock::Resources), ..View::default() };
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "naval_counters");
    layer_seeing_all().inputs(&resources, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the resources mode draws no unit counters");
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(resources).unwrap();
    assert_eq!(side.refusals().count(), 0, "hidden, not refused");
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(layer(&side).held(), layer_seeing_all().held(), "and holds what an empty layer holds");
    assert_eq!(layer(&side).members().count(), 0, "nothing is kept while hidden");
    settled_equals_composed(&mut side);
    side.set_view(View { fog: false, ..View::default() }).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing() && layer(side).refused().is_none());
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert!(member(&side, 100).is_some(), "shown again, it composes");
}

fn hold(side: &mut MapSide, hour: i32) {
    while side.entry().playhead().newest().is_none_or(|newest| newest < hour) {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    side.frame(hour).unwrap();
}

#[test]
fn outside_the_view_a_change_stays_marked_and_a_base_is_always_made() {
    let units = campaign("naval camera", |_, _| Status::Complete, |at, store| {
        if at == 200 {
            put(store, TableId::Ships, 0, "strength", Cell::I64(5));
        }
    });
    let mut layer = layer_seeing_all();
    layer.admit_only(|province| province == 9);
    let mut side = side_with(units, layer, View { fog: false, ..View::default() });
    reach(&mut side, 150);
    side.settle_all().unwrap();
    assert!(member(&side, 100).is_none(), "province 5 is outside the view");
    assert_eq!(member(&side, 102).unwrap().port, Port::In(0), "a base's port items are not the camera's");
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert!(self::layer(&side).outside() > 0, "the change outside is one bit");
    settled_equals_composed(&mut side);
}

