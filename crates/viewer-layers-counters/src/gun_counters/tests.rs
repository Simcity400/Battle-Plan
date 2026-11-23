use super::{Content, GunCounters, Hostility, TABLES};
use viewer_layer_core::{layers::{Layer, View}, playhead_host::{MapSide, open, units::Units}};
use playhead::{
    Settings, Timeline,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    state::{Cell, Kind},
    store::Store,
    unit::{BLOCK, BlockPage, Encoder, PartSink, Settings as Writing, reader::Extents},
};
use replay_data::rows::{Field, NativeValue, Opaque, RailwaySelectionInputs, TableId};
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
};
use viewer_counters_land::gun_members::{Gun, GunMember};
use viewer_foundation::perspective::{Choice, Country, Vision};
use dashboard_text::table_access::TokenKeys;
use viewer_map_modes::map_mode::{MapMode, Stock};

#[derive(Default)]
struct Memory {
    bytes: Vec<u8>,
    pages: Vec<BlockPage>,
}

impl PartSink for Memory {
    fn frame(&mut self, stored: &[u8]) -> io::Result<u64> {
        let offset = self.bytes.len() as u64;
        self.bytes.extend_from_slice(stored);
        Ok(offset)
    }

    fn block(&mut self, page: BlockPage) -> io::Result<()> {
        self.pages.push(page);
        Ok(())
    }

    fn read_back(&mut self, offset: u64, stored: u32, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        out.extend_from_slice(&self.bytes[offset as usize..offset as usize + stored as usize]);
        Ok(())
    }
}

struct Recording {
    bytes: Vec<u8>,
    runs: Vec<Run>,
    kinds: Vec<Vec<Kind>>,
}

impl Pages for Recording {
    fn extents(&self, _run: usize) -> &dyn Extents {
        &self.bytes
    }

    fn runs(&self) -> &[Run] {
        &self.runs
    }

    fn kinds(&self, table: usize) -> Option<&[Kind]> {
        self.kinds.get(table).map(Vec::as_slice)
    }
}

fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap_or_else(|| panic!("{table:?}.{field}"))
}

fn record(hours: u32, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> Units {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        let strings: &[&[u8]] = if at == 0 { &[b"Railway_Gun", b"Other"] } else { &[] };
        store.begin_hour(at as i32, strings).unwrap();
        for table in TABLES {
            store.status(table.index(), status(at, table), true, Some(at as i32), true).unwrap();
        }
        hour(at, &mut store);
        store.end_hour(&mut memory).unwrap();
        if at % BLOCK == BLOCK - 1 {
            store.end_block();
        }
    }
    if !hours.is_multiple_of(BLOCK) {
        store.end_run(&mut memory).unwrap();
    }
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds: kinds.clone() });
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    static MADE: AtomicU32 = AtomicU32::new(0);
    Units { name: format!("gun counters {}", MADE.fetch_add(1, Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) }
}

fn write(store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
}

fn insert(store: &mut Store, table: TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn opaque<T: NativeValue>(value: T) -> Vec<u8> {
    let Field::Opaque(Opaque::Native(value)) = value.field() else { panic!("an opaque value") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn gun(store: &mut Store, slot: u32, id: u64, owner: u32, province: u32, token: u32) {
    let table = TableId::RailwayGunCounterInputs;
    insert(store, table, slot, u128::from(id));
    for (field, value) in [("roster_owner", owner), ("expeditionary_owner", 0), ("logical_owner", owner), ("province", province), ("subunit_token", token), ("target", 0), ("combat_province", 0), ("combat_count", 0)] {
        write(store, table, slot, field, Cell::U32(value));
    }
    for field in ["naval_transfer", "transfer_action_status", "exiled", "retreat", "strategic_redeployment", "combat_role", "attacking"] {
        write(store, table, slot, field, Cell::U8(0));
    }
    write(store, table, slot, "reference_entry_count", Cell::I32(0));
    write(store, table, slot, "assignee_group", Cell::U64(0));
    write(store, table, slot, "current_strength", Cell::I64(100_000));
    write(store, table, slot, "maximum_strength", Cell::I64(100_000));
    write(store, table, slot, "received_supply_ratio", Cell::I64(50_000));
    write(store, table, slot, "railway_building_level", Cell::Opaque(&opaque(Some(2i16))));
    write(store, table, slot, "ordinary_path", Cell::U32List(&[]));
    write(store, table, slot, "alternate_path", Cell::U32List(&[]));
}

fn order(store: &mut Store, slot: u32, province: u32, units: &[u64]) {
    if store.table(TableId::ProvinceUnitOrder.index()).unwrap().slot(u128::from(province)).is_none() {
        insert(store, TableId::ProvinceUnitOrder, slot, u128::from(province));
    }
    write(store, TableId::ProvinceUnitOrder, slot, "mixed", Cell::U64List(units));
}

fn rail(store: &mut Store, slot: u32, province: u32, hour: i32, pairs: i32) {
    if store.table(TableId::RailCooldowns.index()).unwrap().slot(u128::from(province)).is_none() {
        insert(store, TableId::RailCooldowns, slot, u128::from(province));
        write(store, TableId::RailCooldowns, slot, "cooldown_ticks", Cell::I32(0));
    }
    let selection = Some(RailwaySelectionInputs { pair_count: Some(pairs), ..Default::default() });
    write(store, TableId::RailCooldowns, slot, "selection", Cell::Opaque(&opaque(selection)));
    write(store, TableId::RailCooldowns, slot, "selection_hour", Cell::OptionalI32(Some(hour)));
}

fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        insert(store, TableId::CountryLinks, slot, key);
        write(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot));
        insert(store, TableId::Countries, slot, key);
    }
    for (slot, token, string) in [(0, 3u128, 0), (1, 4, 1)] {
        insert(store, TableId::LexicalTokens, slot, token);
        write(store, TableId::LexicalTokens, slot, "literal", Cell::Bytes(string));
    }
    gun(store, 0, 500, 10, 5, 3);
    gun(store, 1, 501, 11, 6, 3);
    order(store, 0, 5, &[500]);
    order(store, 1, 6, &[600, 501]);
}

fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

fn fog_off() -> View {
    View { fog: false, ..View::default() }
}

fn layer_seeing_all() -> GunCounters {
    let subunits = TokenKeys::new(&["railway_gun".to_owned()]);
    let layer = GunCounters::new(Arc::new(Content::bare(subunits, |province| (5..10).contains(&province))));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer.set_relations(Arc::new(viewer_counter_kit::type_tint::Index::default()));
    layer.set_hostility(Arc::new(Hostility::new()));
    layer
}

fn side_for(units: Units) -> MapSide {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer_seeing_all())], fog_off()).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &viewer_layer_core::playhead_host::Frame) -> bool) {
    for _ in 0..50_000_000u64 {
        viewer_layer_core::test_support::recorded::reads_landed(side);
        let head = side.entry().playhead().newest().unwrap_or(0);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached what the test waits for");
}

fn reach(side: &mut MapSide, hour: i32) {
    for _ in 0..50_000_000u64 {
        viewer_layer_core::test_support::recorded::reads_landed(side);
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

fn hold(side: &mut MapSide, hour: i32) {
    while side.entry().playhead().newest().is_none_or(|newest| newest < hour) {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    side.frame(hour).unwrap();
}

fn layer(side: &MapSide) -> &GunCounters {
    side.parts().unwrap().1[0].any().downcast_ref::<GunCounters>().unwrap()
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn gun_of(side: &MapSide, id: u64) -> Option<Gun> {
    layer(side).guns().find(|(held, _)| *held == id).map(|(_, gun)| gun.clone())
}

fn member(side: &MapSide, id: u64) -> GunMember {
    match gun_of(side, id) {
        Some(Gun::Member(member)) => member,
        other => panic!("gun {id} is no member: {other:?}"),
    }
}

fn phase(side: &MapSide, name: &str) -> u64 {
    layer(side).phases().find(|(held, _, _)| *held == name).map_or(0, |(_, entities, _)| entities)
}

#[test]
fn a_gun_changed_in_a_step_is_marked_once_and_settles_to_the_composed_value() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::RailwayGunCounterInputs, 0, "current_strength", Cell::I64(25_000));
            write(store, TableId::RailwayGunCounterInputs, 0, "received_supply_ratio", Cell::I64(10_000));
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let first = member(&side, 500);
    assert_eq!((first.province, first.ordinal, first.owner, first.own, first.hp, first.name.as_str()), (5, 0, 0, true, 100_000, "railway_gun"), "the token's spelling names the authored key under the fold");
    assert_eq!(member(&side, 501).ordinal, 1, "its place in its province's mixed array");
    assert_eq!((first.join_action, first.action, first.location_warning), (1, Ok(1), None), "no rail row for its province: the warning is undecided");
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    assert_eq!(layer(&side).marked() - marked, 1, "two cells of one gun mark it once");
    settled_equals_composed(&mut side);
    let changed = member(&side, 500);
    assert_eq!((changed.hp, changed.supply_ratio), (25_000, 10_000));
}

#[test]
fn a_gun_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::RailwayGunCounterInputs, 1, "current_strength", Cell::I64(10_000)),
        201 => {
            write(store, TableId::RailwayGunCounterInputs, 1, "province", Cell::U32(5));
            order(store, 0, 5, &[501, 500]);
            order(store, 1, 6, &[600]);
        }
        202 => write(store, TableId::RailwayGunCounterInputs, 1, "current_strength", Cell::I64(30_000)),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let settles = side.hold_settles();
    reach(&mut side, 204);
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    let moved = member(&side, 501);
    assert_eq!((moved.province, moved.ordinal, moved.hp), (5, 0, 30_000));
    assert_eq!(member(&side, 500).ordinal, 1);
}

#[test]
fn a_gun_deleted_and_another_inserted_in_its_slot_updates_every_array_naming_them() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            store.table(TableId::RailwayGunCounterInputs.index()).unwrap().delete(0).unwrap();
            order(store, 0, 5, &[]);
        }
        201 => {
            gun(store, 0, 502, 11, 7, 3);
            order(store, 2, 7, &[502]);
        }
        220 => rail(store, 0, 5, 0, 2),
        230 => rail(store, 1, 7, 0, 2),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert!(gun_of(&side, 500).is_none(), "the deleted gun is gone");
    reach(&mut side, 212);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 502).province, 7, "its slot taken by another");
    let marked = layer(&side).marked();
    reach(&mut side, 224);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).marked(), marked, "the old gun's province no longer names its slot");
    reach(&mut side, 232);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).marked() - marked, 1, "the new one's does");
    settled_equals_composed(&mut side);
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = record(BLOCK, |at, table| match (at, table) {
        (240..=247, TableId::RailwayGunCounterInputs) => Status::Truncated { missing: 1 },
        (260..=267, TableId::RailCooldowns) => Status::Unavailable { reason: 1 },
        (280..=287, TableId::CountryLinks) => Status::NotRecorded,
        _ => Status::Complete,
    }, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::RailwayGunCounterInputs, 1, "subunit_token", Cell::U32(8)),
        210 => write(store, TableId::RailwayGunCounterInputs, 0, "subunit_token", Cell::U32(4)),
        at if at >= 220 => {
            write(store, TableId::RailwayGunCounterInputs, 0, "subunit_token", Cell::U32(3));
            rail(store, 0, 5, at as i32, 2);
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 204);
    settled_equals_composed(&mut side);
    assert_eq!(gun_of(&side, 501), Some(Gun::Unnamed("token 8 is not in lexical tokens".into())));
    reach(&mut side, 212);
    settled_equals_composed(&mut side);
    assert_eq!(gun_of(&side, 500), Some(Gun::Unnamed("sub-unit Other is not authored".into())));
    reach(&mut side, 232);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 500).location_warning, Some(false), "a current sample with neighbours: no warning");
    reach(&mut side, 244);
    settled_equals_composed(&mut side);
    assert_eq!((layer(&side).census(), layer(&side).guns().count()), (Some(2), 0), "a partial census decides no gun");
    reach(&mut side, 264);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 500).location_warning, None, "unavailable cooldowns leave the warning undecided, never the last rows'");
    reach(&mut side, 284);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("CountryLinks")), "{:?}", layer(&side).refused());
    assert_eq!(layer(&side).guns().count(), 0, "nothing is drawn from the last rows");
    reach(&mut side, 292);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert_eq!(member(&side, 500).location_warning, Some(false), "after the gap, the guns again");
}

#[test]
fn the_rail_sample_follows_the_hour_and_the_hour_touches_only_guns_on_a_rail_row() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        at @ 200..=219 => rail(store, 0, 5, at as i32, 0),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 210);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 500).location_warning, Some(true), "a current sample without neighbour pairs warns");
    assert_eq!(member(&side, 501).location_warning, None, "no rail row for its province decides nothing");
    reach(&mut side, 232);
    settled_equals_composed(&mut side);
    assert_eq!(member(&side, 500).location_warning, None, "its sample went stale with the hour, its row unchanged");
    assert_eq!(layer(&side).on_rail(), 1, "only the gun standing on a rail row follows the hour");
    let (marked, hourly) = (layer(&side).marked(), phase(&side, "on rail"));
    reach(&mut side, 300);
    settled_equals_composed(&mut side);
    let (marked, hourly) = (layer(&side).marked() - marked, phase(&side, "on rail") - hourly);
    assert!(hourly > 0, "the gun on a rail row is marked as the hour moves");
    assert_eq!(marked, hourly as usize, "and nothing else is marked: the other gun is never touched by the hour");
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::RailwayGunCounterInputs, 0, "province", Cell::U32(6)),
        _ => {}
    });
    let resources = View { map_mode: MapMode::Stock(Stock::Resources), ..fog_off() };
    let mut guns_off = fog_off();
    guns_off.overlay.counters.railway_guns = false;
    let kinds = replay_recording::schema::schema().unwrap();
    for view in [&resources, &guns_off] {
        let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "gun_counters");
        layer_seeing_all().inputs(view, &mut inputs).unwrap();
        assert!(inputs.into_tables().is_empty(), "nothing is declared where no gun counter is drawn");
    }
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(resources).unwrap();
    assert_eq!(side.refusals().count(), 0, "hidden, not refused");
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(layer(&side).held(), layer_seeing_all().held(), "and holds what an empty layer holds");
    assert_eq!(layer(&side).guns().count(), 0, "nothing is kept while hidden");
    assert!(layer(&side).published().is_none(), "and the window is given no gun to draw");
    settled_equals_composed(&mut side);
    side.set_view(fog_off()).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing() && side.parts().is_some_and(|(mirrors, _)| mirrors.get(TableId::RailwayGunCounterInputs).is_some()));
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert!(gun_of(&side, 500).is_some(), "composed again from the mirrors it is given");
    let mut members: Vec<u64> = layer(&side).guns().filter_map(|(id, gun)| matches!(gun, Gun::Member(_)).then_some(*id)).collect();
    members.sort_unstable();
    let published = layer(&side).published().expect("shown again, the window is given its guns");
    assert!(published.complete, "the census is whole");
    assert_eq!(published.members.iter().map(|placed| placed.member.id).collect::<Vec<_>>(), members, "the guns the window draws are the layer's members, each with its route");
}

#[test]
fn the_layer_declares_against_this_schema() {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "gun_counters");
    layer_seeing_all().inputs(&fog_off(), &mut inputs).unwrap();
    assert_eq!(inputs.into_tables().len(), TABLES.len());
}
