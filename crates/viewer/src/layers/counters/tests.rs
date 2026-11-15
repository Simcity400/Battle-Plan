use super::{Content, Counters};
use crate::{
    layers::{Layer, View},
    playhead_host::{MapSide, open, units::Units},
};
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
use replay_data::rows::TableId;
use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Duration,
};
use viewer_counters::map::counters::Member;
use viewer_foundation::map::perspective::{Choice, Country, Sight, Vision};
use viewer_world::map::map_mode::{MapMode, Stock};

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

/// The tables the layer reads: what it declares, one list with its declaration.
fn read() -> Vec<TableId> {
    super::declared().collect()
}

fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap_or_else(|| panic!("{table:?}.{field}"))
}

/// The recording's schema, the layer's tables written over `hours` hours from hour 0 by `hour`,
/// every table complete unless `status` says otherwise; hour `n` names string `n`. Steps race
/// ahead of the first compose, so tests change things from hour 200.
fn record(hours: u32, status: impl Fn(u32, TableId) -> Status, mut hour: impl FnMut(u32, &mut Store)) -> Units {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    for at in 0..hours {
        let text = format!("text {at}");
        store.begin_hour(at as i32, &[text.as_bytes()]).unwrap();
        for table in read() {
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
    // The host shares one playhead among callers of the same name; each recording is its own.
    static MADE: AtomicU32 = AtomicU32::new(0);
    Units { name: format!("counters {}", MADE.fetch_add(1, Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) }
}

fn write(store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
}

fn insert(store: &mut Store, table: TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

/// A division at `slot`, keyed `id`, owned through link `owner`, standing in `province`; its
/// strength and organisation in units of their display steps (tenths).
fn division(store: &mut Store, slot: u32, id: u64, owner: u32, province: u32, hit_points: i64) {
    insert(store, TableId::Divisions, slot, u128::from(id));
    for (field, value) in [("roster_owner", owner), ("logical_owner", owner), ("expeditionary_owner", 0), ("province", province)] {
        write(store, TableId::Divisions, slot, field, Cell::U32(value));
    }
    write(store, TableId::Divisions, slot, "template", Cell::U64(7));
    write(store, TableId::Divisions, slot, "hit_points", Cell::I64(hit_points));
    write(store, TableId::Divisions, slot, "organisation", Cell::I64(4));
    write(store, TableId::Divisions, slot, "orders_group", Cell::U64(0));
}

/// Two linked countries; divisions 100 and 101 in province 5 (its array: 101, 100), 102 in 6;
/// a status and panel row for 100.
fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        insert(store, TableId::CountryLinks, slot, key);
        write(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot));
        insert(store, TableId::Countries, slot, key);
    }
    division(store, 0, 100, 10, 5, 5);
    division(store, 1, 101, 11, 5, 6);
    division(store, 2, 102, 10, 6, 7);
    order(store, 0, 5, &[101, 100]);
    order(store, 1, 6, &[102]);
    insert(store, TableId::DivisionStatus, 0, 100);
    for (field, value) in [("entrenchment", 3), ("entrenchment_cap", 9), ("attrition_value", 5), ("attrition_threshold", 2), ("strength_factor", 100_000)] {
        write(store, TableId::DivisionStatus, 0, field, Cell::I64(value));
    }
    write(store, TableId::DivisionStatus, 0, "supply_ratio", Cell::OptionalI64(Some(80)));
    write(store, TableId::DivisionStatus, 0, "exercising", Cell::U8(1));
    insert(store, TableId::DivisionPanelStats, 0, 100);
    write(store, TableId::DivisionPanelStats, 0, "panel_stats[0]", Cell::OptionalI64(Some(100_000)));
    write(store, TableId::DivisionPanelStats, 0, "panel_stats[1]", Cell::OptionalI64(Some(80_000)));
}

fn order(store: &mut Store, slot: u32, province: u32, units: &[u64]) {
    if store.table(TableId::ProvinceUnitOrder.index()).unwrap().slot(u128::from(province)).is_none() {
        insert(store, TableId::ProvinceUnitOrder, slot, u128::from(province));
    }
    write(store, TableId::ProvinceUnitOrder, slot, "mixed", Cell::U64List(units));
}

/// Four hours a step, a frame a second.
fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }
}

fn fog_off() -> View {
    View { fog: false, ..View::default() }
}

/// Viewed as country slot 0 with fog off: every division seen.
fn layer_seeing_all() -> Counters {
    let layer = Counters::new(Arc::new(Content::bare()));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer.with_relations(Arc::new(viewer_counters::map::counters::type_tint::Index::default()))
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

fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &crate::playhead_host::Frame) -> bool) {
    for _ in 0..50_000_000u64 {
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

/// Frames with the head held within three hours past `hour` (steps stay four hours) until the
/// mirrors are at or past it and nothing is composing.
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

/// Frames with the head at `hour` until the playhead has built steps to it, whatever is declared.
fn hold(side: &mut MapSide, hour: i32) {
    while side.entry().playhead().newest().is_none_or(|newest| newest < hour) {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    side.frame(hour).unwrap();
}

fn layer(side: &MapSide) -> &Counters {
    side.parts().unwrap().1[0].any().downcast_ref::<Counters>().unwrap()
}

/// The layer settled whole equals itself composed anew from the same mirrors.
fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn members(side: &MapSide) -> Vec<((u32, u32), Vec<Member>)> {
    layer(side).sources().map(|(source, members)| (source, members.cloned().collect())).collect()
}

fn member(side: &MapSide, id: u64) -> Option<((u32, u32), Member)> {
    members(side).into_iter().find_map(|(source, members)| members.into_iter().find(|member| member.id == id).map(|member| (source, member)))
}

fn ids(side: &MapSide) -> Vec<((u32, u32), Vec<u64>)> {
    members(side).into_iter().map(|(source, members)| (source, members.into_iter().map(|member| member.id).collect())).collect()
}

#[test]
fn a_division_changed_in_a_step_is_marked_once_and_settles_to_the_composed_value() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::Divisions, 0, "hit_points", Cell::I64(2));
            write(store, TableId::Divisions, 0, "organisation", Cell::I64(2));
            write(store, TableId::DivisionStatus, 0, "entrenchment", Cell::I64(4));
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    assert_eq!(ids(&side), vec![((5, 0), vec![101, 100]), ((6, 0), vec![102])], "occupant order, not id order");
    let (_, first) = member(&side, 100).unwrap();
    assert_eq!((first.ordinal, first.owner, first.sight, first.bars, first.status.entrenchment, first.status.attrition), (1, 0, Sight::Full, true, 3, true));
    assert_eq!((first.shared_vision, first.status.own), (Ok(true), true));
    assert_eq!(member(&side, 101).unwrap().1.shared_vision, Ok(false));
    assert_eq!(first.status.supply_ratio, 80_000, "stored in units of its display step, a thousandth");
    assert!(!member(&side, 101).unwrap().1.bars, "no status row: no bars");
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    assert_eq!(layer(&side).marked() - marked, 1, "three cells of one division in two tables mark it once");
    settled_equals_composed(&mut side);
    let (_, changed) = member(&side, 100).unwrap();
    assert_eq!(changed.status.entrenchment, 4);
    assert_ne!((changed.hp, changed.org), (first.hp, first.org));
}

#[test]
fn a_division_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::Divisions, 1, "hit_points", Cell::I64(1)),
        201 => {
            write(store, TableId::Divisions, 1, "province", Cell::U32(6));
            order(store, 0, 5, &[100]);
            order(store, 1, 6, &[102, 101]);
        }
        202 => write(store, TableId::Divisions, 1, "hit_points", Cell::I64(3)),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    // Both changes marked before any settle; then, with no budget, a unit a frame to the newest.
    side.set_budget(Duration::ZERO);
    let settles = side.hold_settles();
    reach(&mut side, 204);
    crate::layers::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    assert_eq!(ids(&side), vec![((5, 0), vec![100]), ((6, 0), vec![102, 101])]);
    let (_, moved) = member(&side, 101).unwrap();
    assert_eq!(moved.ordinal, 1);
    let mirrors = side.parts().unwrap().0;
    let divisions = mirrors.get(TableId::Divisions).unwrap();
    assert_eq!(divisions.raw_cell(column(TableId::Divisions, "hit_points"), divisions.slot(101).unwrap()), Some(3));
}

#[test]
fn a_division_deleted_and_another_inserted_in_its_slot_updates_every_source_naming_them() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            store.table(TableId::Divisions.index()).unwrap().delete(2).unwrap();
            order(store, 1, 6, &[]);
        }
        201 => {
            division(store, 2, 103, 11, 5, 9);
            order(store, 0, 5, &[101, 103, 100]);
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert_eq!(ids(&side), vec![((5, 0), vec![101, 100])], "the deleted division's source is gone with it");
    reach(&mut side, 201);
    settled_equals_composed(&mut side);
    assert_eq!(ids(&side), vec![((5, 0), vec![101, 103, 100])], "its slot taken by another");
    assert_eq!(member(&side, 103).unwrap().1.owner, 1);
    assert!(member(&side, 102).is_none());
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = record(BLOCK, |at, table| match (at, table) {
        (240..=247, TableId::ProvinceUnitOrder) => Status::Truncated { missing: 1 },
        (260..=267, TableId::DivisionStatus) => Status::Unavailable { reason: 1 },
        (280..=287, TableId::CountryLinks) => Status::NotRecorded,
        _ => Status::Complete,
    }, |at, store| match at {
        0 => world(store),
        236 => order(store, 0, 5, &[101]),
        250 => order(store, 0, 5, &[101, 100]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 236);
    settled_equals_composed(&mut side);
    assert!(member(&side, 100).is_none(), "absent from its province's array: no counter");
    reach(&mut side, 244);
    settled_equals_composed(&mut side);
    let (source, unordered) = member(&side, 100).expect("without a whole occupant order every division stands, at ordinal zero");
    assert_eq!((source, unordered.ordinal), ((5, 0), 0));
    assert_eq!(ids(&side)[0], ((5, 0), vec![100, 101]), "ties in id order");
    reach(&mut side, 264);
    settled_equals_composed(&mut side);
    let (_, unavailable) = member(&side, 100).unwrap();
    assert!(!unavailable.bars && unavailable.status.exercising.is_none(), "an unavailable status reads as a missing row, never the last one");
    reach(&mut side, 284);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("CountryLinks")), "{:?}", layer(&side).refused());
    assert!(members(&side).is_empty(), "nothing is drawn from the last rows");
    reach(&mut side, 292);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert_eq!(member(&side, 100).unwrap().1.ordinal, 1, "after the gap, the arrays again");
}

#[test]
fn a_mode_without_land_counters_declares_nothing_and_a_mode_with_them_composes() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::Divisions, 0, "province", Cell::U32(6));
            order(store, 0, 5, &[101]);
            order(store, 1, 6, &[102, 100]);
        }
        _ => {}
    });
    let resources = View { map_mode: MapMode::Stock(Stock::Resources), ..fog_off() };
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut inputs = crate::layers::Inputs::new(&kinds, "counters");
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
    assert!(members(&side).is_empty(), "nothing is kept while hidden");
    settled_equals_composed(&mut side);
    side.set_view(fog_off()).unwrap();
    // Its tables left the mirrors while hidden and come back as a reset, which it composes from.
    frames_until(&mut side, |side, _| !side.composing() && side.draws("counters") && layer(side).refused().is_none());
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert!(!members(&side).is_empty(), "composed again from the mirrors it is given");
}

#[test]
fn a_new_vision_takes_every_division_s_sight_again() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| {
        if at == 0 {
            world(store);
        }
    });
    let mut side = side_for(units);
    assert!(member(&side, 101).is_some());
    let fogged = Arc::new(Vision::sharing(0, []));
    layer(&side).set_vision(Choice { country: Country::Recorded, fog: true }, fogged);
    frames_until(&mut side, |side, frame| frame.steps > 0 && layer(side).carried().entities == 0);
    settled_equals_composed(&mut side);
    assert!(member(&side, 101).is_none(), "another country's division, undetected under fog, has no counter");
    assert_eq!(member(&side, 100).unwrap().1.sight, Sight::Full, "our own stays in full");
}

#[test]
fn the_layer_declares_against_this_schema() {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut inputs = crate::layers::Inputs::new(&kinds, "counters");
    layer_seeing_all().inputs(&fog_off(), &mut inputs).unwrap();
    let declared: Vec<usize> = inputs.into_tables().into_keys().collect();
    let mut read: Vec<usize> = read().into_iter().map(|table| table.index()).collect();
    read.sort_unstable();
    read.dedup();
    assert_eq!(declared, read, "the tables the layer declares are its list, each once");
}




#[test]
fn outside_the_view_a_change_stays_marked_and_inside_it_is_made() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::Divisions, 0, "hit_points", Cell::I64(2));
            write(store, TableId::Divisions, 2, "hit_points", Cell::I64(1));
        }
        _ => {}
    });
    let entry = open(units, quick()).unwrap();
    let mut counters = layer_seeing_all();
    counters.admit_only(|province| province == 5);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(counters)], fog_off()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    reach(&mut side, 150);
    side.settle_all().unwrap();
    assert_eq!(ids(&side), vec![((5, 0), vec![101, 100])], "only the admitted province's members are made");
    let before = member(&side, 100).unwrap().1.hp;
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_ne!(member(&side, 100).unwrap().1.hp, before, "inside: made");
    assert!(layer(&side).outside() > 0, "outside: kept marked");
    assert_eq!(layer(&side).carried().entities, 0, "what waits outside is not late");
    settled_equals_composed(&mut side);
}

#[test]
fn a_destination_changing_hands_gives_only_the_division_going_there_its_action_again() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => {
            world(store);
            insert(store, TableId::DivisionMovement, 0, 102);
            write(store, TableId::DivisionMovement, 0, "path_total", Cell::U32(1));
            write(store, TableId::DivisionMovement, 0, "path", Cell::U32List(&[7]));
            insert(store, TableId::Provinces, 0, 7);
            write(store, TableId::Provinces, 0, "controller", Cell::U32(10));
        }
        200 => write(store, TableId::Provinces, 0, "controller", Cell::U32(11)),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let action = |side: &MapSide, id: u64| layer(side).sources_acting().flat_map(|(_, members)| members.map(|(member, action)| (member.id, action)).collect::<Vec<_>>()).find(|&(held, _)| held == id).map(|(_, action)| action);
    assert_eq!(action(&side, 102), Some(Ok(4)), "its own controller's province: an ordinary move");
    assert_eq!(action(&side, 101), Some(Err("DivisionMovement division")), "not the viewer's: no action, as today");
    assert_eq!(action(&side, 100), Some(Err("DivisionMovement division")), "the viewer's, without a movement row");
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).marked() - marked, 1, "only the division going there");
    assert_eq!(action(&side, 102), Some(Err("action destination directed-war receiver unavailable")), "another controller asks the relations layer's wars, which no layer gives here");
    settled_equals_composed(&mut side);
}

/// The counters reading the vision layer registered before them, nothing given to either: countries
/// 101 (slot 1, the recorded client's) and 102 (slot 2) at war under fog; provinces 1 and 2 are
/// 101's, 3 and 4 are 102's, sea 9 beyond 4. Division 901 (101's) stands in province 1, 905 (102's) in
/// province 4: each country sees its own and not the other's, two provinces away.
fn reading_side() -> MapSide {
    reading_side_with(false, |_, _| {})
}

/// [`reading_side`] with `more` written after each hour's rows, and the counters the window's
/// layer, made on its worker ([`super::fed::Fed`]), when `fed`.
fn reading_side_with(fed: bool, more: impl Fn(u32, &mut Store)) -> MapSide {
    use viewer_foundation::map::{perspective::VISION_TABLES, visibility::{FogContent, Topology, test_defines}};
    let mut tables: Vec<TableId> = read();
    tables.extend(VISION_TABLES.iter().copied().filter(|table| !tables.contains(table)).collect::<Vec<_>>());
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut store = Store::new(kinds.clone(), vec![false; kinds.len()]);
    store.attach(Encoder::new(&kinds, Writing::at(3)).unwrap());
    let mut memory = Memory::default();
    store.begin_run(0);
    let tags: [&[u8]; 2] = [b"AAA", b"BBB"];
    for at in 0..BLOCK {
        store.begin_hour(at as i32, if at == 0 { &tags[..] } else { &[] }).unwrap();
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        if at == 0 {
            for (slot, raw) in [101u32, 102].into_iter().enumerate() {
                let slot = slot as u32;
                insert(&mut store, TableId::CountryLinks, slot, u128::from(raw));
                write(&mut store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot + 1));
                insert(&mut store, TableId::Countries, slot, u128::from(raw));
                write(&mut store, TableId::Countries, slot, "canonical_slot", Cell::U32(slot + 1));
                write(&mut store, TableId::Countries, slot, "tag", Cell::Bytes(slot));
            }
            insert(&mut store, TableId::LocalPlayer, 0, 0);
            write(&mut store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
            for (slot, (province, state, controller)) in [(1u32, 100u32, 101u32), (2, 100, 101), (3, 200, 102), (4, 200, 102), (9, 0, 0)].into_iter().enumerate() {
                insert(&mut store, TableId::Provinces, slot as u32, u128::from(province));
                write(&mut store, TableId::Provinces, slot as u32, "state", Cell::U32(state));
                write(&mut store, TableId::Provinces, slot as u32, "controller", Cell::U32(controller));
            }
            insert(&mut store, TableId::WarRelations, 0, 101 << 32 | 102);
            for (slot, (id, owner, province)) in [(901u64, 101u32, 1u32), (905, 102, 4)].into_iter().enumerate() {
                division(&mut store, slot as u32, id, owner, province, 5);
                write(&mut store, TableId::Divisions, slot as u32, "reveal_skip", Cell::U8(0));
                order(&mut store, slot as u32, province, &[id]);
            }
        }
        more(at, &mut store);
        store.end_hour(&mut memory).unwrap();
    }
    store.end_block();
    let recording = Arc::new(Recording { bytes: memory.bytes, runs: vec![Run::new(0, memory.pages).unwrap()], kinds: kinds.clone() });
    let pages: Arc<dyn Pages> = recording;
    let timeline = Timeline::whole(&*pages).unwrap();
    static MADE: AtomicU32 = AtomicU32::new(0);
    let units = Units { name: format!("counters reading the vision layer {}", MADE.fetch_add(1, Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) };
    let content = {
        let neighbors = [(1, vec![2]), (2, vec![1, 3]), (3, vec![2, 4]), (4, vec![3, 9]), (9, vec![4])].into_iter().collect();
        Arc::new(crate::layers::vision::Content::new(Ok(FogContent { topology: Topology { land: (1..=4).collect(), sea: std::collections::BTreeSet::from([9]), neighbors }, defines: test_defines() })))
    };
    let counters = Counters::new(Arc::new(Content::bare())).with_relations(Arc::new(viewer_counters::map::counters::type_tint::Index::default())).reading_vision();
    let entry = open(units, quick()).unwrap();
    let counters: Box<dyn Layer> = if fed { Box::new(super::fed::Fed::new(counters)) } else { Box::new(counters) };
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(crate::layers::vision::Vision::new(content)), counters];
    let mut side = MapSide::new(Arc::clone(&entry), layers, View { fog: true, perspective: Choice { country: Country::Recorded, fog: true }, ..View::default() }).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    reach(&mut side, 2);
    side
}

fn reading(side: &MapSide) -> &Counters {
    side.parts().unwrap().1[1].any().downcast_ref::<Counters>().unwrap()
}

/// Whole settles, at most eight (the vision layer's waits for its worker), until no layer carries
/// anything.
fn reading_settled(side: &mut MapSide) {
    for _ in 0..8 {
        side.settle_all().unwrap();
        if side.parts().unwrap().1.iter().all(|layer| layer.carried().entities == 0) {
            return;
        }
    }
    panic!("eight whole settles left work carried");
}

fn reading_member(side: &MapSide, id: u64) -> Option<Member> {
    reading(side).sources().flat_map(|(_, members)| members.cloned().collect::<Vec<_>>()).find(|member| member.id == id)
}

/// The reading layer against itself composed anew from the same mirrors and vision layer.
fn reading_equals_composed(side: &MapSide) {
    let (mirrors, layers) = side.parts().unwrap();
    let mut fresh = reading(side).fresh();
    fresh.compose_with(mirrors, &crate::layers::Upstream::new(&layers[..1])).unwrap();
    assert_eq!(reading(side).differ(&fresh), (0, None));
}

#[test]
fn the_counters_reading_the_vision_layer_follow_a_change_of_the_viewing_country_with_nothing_given() {
    let mut side = reading_side();
    reading_settled(&mut side);
    assert_eq!(reading(&side).refused(), None);
    assert!(reading(&side).perspective().is_none(), "no vision is given to the layer");
    assert!(reading_member(&side, 901).is_some(), "the viewer's own division");
    assert!(reading_member(&side, 905).is_none(), "the enemy's, two provinces away under fog, has no counter");
    reading_equals_composed(&side);
    side.set_view(View { fog: true, perspective: Choice { country: Country::Tag("BBB".into()), fog: true }, ..View::default() }).unwrap();
    reach(&mut side, 6);
    reading_settled(&mut side);
    assert!(reading(&side).perspective().is_none(), "still nothing given");
    assert!(reading_member(&side, 905).is_some(), "viewed as 102, its own division has its counter");
    assert!(reading_member(&side, 901).is_none(), "and 101's, two provinces away, none");
    reading_equals_composed(&side);
}

fn phase(side: &MapSide, name: &str) -> u64 {
    layer(side).phases().find(|(held, _, _)| *held == name).map_or(0, |(_, entities, _)| entities)
}

/// Division 100 changed in three steps (its strength, its status, then its place in its province's
/// array) and 102, outside the view, in one, while every settle is held: `apply` reads no row (on the
/// base it read each division's province to admit it, and each status row's key) and makes nothing,
/// the work is carried, and released with no budget, a unit a frame makes 100 once, from its newest
/// values.
#[test]
fn steps_mark_their_divisions_reading_no_row_and_a_division_changed_in_several_is_made_once() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::Divisions, 0, "hit_points", Cell::I64(2));
            write(store, TableId::Divisions, 2, "hit_points", Cell::I64(1));
        }
        204 => write(store, TableId::DivisionStatus, 0, "entrenchment", Cell::I64(5)),
        208 => {
            write(store, TableId::Divisions, 0, "hit_points", Cell::I64(3));
            order(store, 0, 5, &[100, 101]);
        }
        _ => {}
    });
    let entry = open(units, quick()).unwrap();
    let mut counters = layer_seeing_all();
    counters.admit_only(|province| province == 5);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(counters)], fog_off()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let made = (phase(&side, "whole members"), phase(&side, "strength"));
    side.set_budget(Duration::ZERO);
    let settles = side.hold_settles();
    reach(&mut side, 208);
    assert_eq!(layer(&side).applied_reads(), 0, "apply marks; it reads no row");
    assert_eq!((phase(&side, "whole members"), phase(&side, "strength")), made, "nothing is made while the settles are held");
    crate::layers::recorded::released_unit_by_unit(&mut side, settles);
    assert_eq!(phase(&side, "strength") - made.1, 1, "division 100, changed in three steps, is made once");
    assert_eq!(phase(&side, "whole members"), made.0, "and nothing whole: 102 is outside the view");
    assert_eq!(ids(&side), vec![((5, 0), vec![100, 101])], "its newest place in the array");
    let mirrors = side.parts().unwrap().0;
    let divisions = mirrors.get(TableId::Divisions).unwrap();
    assert_eq!(divisions.raw_cell(column(TableId::Divisions, "hit_points"), divisions.slot(100).unwrap()), Some(3));
    assert_eq!(member(&side, 100).unwrap().1.status.entrenchment, 5);
    settled_equals_composed(&mut side);
}

/// A recording's executable tables, none of them decoded, put in scope only where scoped: never the
/// process's, which every thread would see.
fn scoped_tables() -> engine_executable::executable_tables::Tables {
    let nowhere = std::env::temp_dir().join("battle-plan no game here");
    Box::leak(Box::new(engine_executable::executable_tables::decode(engine_executable::build::ExecutableIdentity::of(&[]), Vec::new(), &nowhere)))
}

/// After a seek the host composes its layers on its own worker thread, where no recording's
/// executable tables are in scope. The strategic layout validates against them; on the base it was
/// validated there without them ("executable metadata unavailable"), so a layer composed after a
/// seek withheld every strategic redeployment (`division strategic action fixed layout`) that
/// today and the layer composed on the frame thread draw, until its next compose. Every validation
/// must run with the recording's tables in scope. Run it by name, alone: a test that loads an
/// installed game's tables puts them in scope on every thread of its process.
#[test]
fn a_compose_on_the_hosts_worker_validates_the_strategic_layout_with_the_recordings_tables() {
    let tables = scoped_tables();
    engine_executable::executable_tables::enter(Some(tables));
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| {
        if at == 0 {
            world(store);
        }
    });
    let entry = open(units, quick()).unwrap();
    let counters = Counters::new(Arc::new(Content { tables: Some(tables), ..Content::bare() }));
    counters.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    let counters = counters.with_relations(Arc::new(viewer_counters::map::counters::type_tint::Index::default()));
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(counters)], fog_off()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    reach(&mut side, 150);
    let [before, _] = layer(&side).validated();
    entry.playhead().seek(100);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.settle_all().unwrap();
    let [scoped, unscoped] = layer(&side).validated();
    engine_executable::executable_tables::enter(None);
    assert!(scoped > before, "the seek's compose validated the strategic layout");
    assert_eq!(unscoped, 0, "and every validation, on whatever thread, had the recording's tables in scope");
    settled_equals_composed(&mut side);
}

/// The reading side with the window's counters layer and 902, 101's, in province 2 besides: the
/// viewer sees two stacks, 901's in province 1 and 902's. `strength` gives the strength written at
/// an hour to the divisions at slots 0 (901) and 2 (902), when it moves. Both have a status and
/// maxima: without them a member's strength is not recorded and moves with no hit points.
fn fed_side(strength: impl Fn(u32, u32) -> Option<i64>) -> MapSide {
    reading_side_with(true, move |at, store| {
        if at == 0 {
            division(store, 2, 902, 101, 2, 5);
            write(store, TableId::Divisions, 2, "reveal_skip", Cell::U8(0));
            order(store, 2, 2, &[902]);
            for (slot, id) in [(0, 901u128), (1, 902)] {
                insert(store, TableId::DivisionStatus, slot, id);
                write(store, TableId::DivisionStatus, slot, "strength_factor", Cell::I64(100_000));
                insert(store, TableId::DivisionPanelStats, slot, id);
                write(store, TableId::DivisionPanelStats, slot, "panel_stats[0]", Cell::OptionalI64(Some(100_000)));
                write(store, TableId::DivisionPanelStats, slot, "panel_stats[1]", Cell::OptionalI64(Some(80_000)));
            }
        }
        for slot in [0, 2] {
            if let Some(hit_points) = strength(at, slot) {
                write(store, TableId::Divisions, slot, "hit_points", Cell::I64(hit_points));
            }
        }
    })
}

fn fed(side: &MapSide) -> &super::fed::Fed {
    side.parts().unwrap().1[1].any().downcast_ref::<super::fed::Fed>().unwrap()
}

/// The window's product, current at the hour applied, against the counters composed anew on this
/// thread from the same mirrors and vision layer.
fn fed_equals_composed(side: &MapSide) {
    use crate::layers::{Budget, Published, Upstream};
    let (mirrors, layers) = side.parts().unwrap();
    let layer = fed(side);
    assert!(matches!(layer.published(), Published::Current), "settled whole, the product is the hour applied's");
    assert_eq!(layer.refused(), None);
    let upstream = Upstream::new(&layers[..1]);
    let mut fresh = layer.fresh();
    fresh.compose_with(mirrors, &upstream).unwrap();
    fresh.settle_with(mirrors, &upstream, &mut Budget::unlimited()).unwrap();
    fresh.publish(&mut Budget::unlimited());
    assert_eq!(**layer.stacks().unwrap(), *fresh.published);
}

#[test]
fn steps_moving_two_stacks_are_made_on_the_worker_and_equal_a_compose_at_their_hour() {
    let mut side = fed_side(|at, _| (at >= 200).then_some(3 + i64::from(at % 5)));
    reach(&mut side, 196);
    reading_settled(&mut side);
    fed_equals_composed(&side);
    let mut product = Arc::clone(fed(&side).stacks().unwrap());
    assert_eq!(product.len(), 2, "901's stack and 902's");
    for hour in (200..=240).step_by(4) {
        let made = super::made_here();
        reach(&mut side, hour);
        reading_settled(&mut side);
        assert_eq!(super::made_here(), made, "at hour {hour} the frame thread made a stack");
        let now = Arc::clone(fed(&side).stacks().unwrap());
        assert!(!Arc::ptr_eq(&now, &product), "at hour {hour} the step moved both stacks: a new product");
        assert!(now.values().zip(product.values()).all(|(now, before)| !Arc::ptr_eq(now, before)), "and both its sources anew");
        fed_equals_composed(&side);
        product = now;
    }
}

/// Frames with the head moved one step (four hours) past the hour applied whenever the frame before
/// applied it, `each` after every frame with the hour applied before it and the frame, until `steps`
/// steps are applied.
fn step_a_frame(side: &mut MapSide, steps: u32, mut each: impl FnMut(&MapSide, i32, &crate::playhead_host::Frame)) {
    let mut before = fed(side).applied_hour().unwrap();
    let mut target = before + 4;
    let mut applied_steps = 0;
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(target);
        let frame = side.frame(head).unwrap();
        let applied = fed(side).applied_hour().unwrap();
        each(side, before, &frame);
        if applied > before {
            assert_eq!(applied - before, 4, "one step a frame");
            applied_steps += 1;
            target = applied + 4;
            if applied_steps == steps {
                return;
            }
        } else if frame.steps == 0 {
            std::thread::yield_now();
        }
        before = applied;
    }
    panic!("the map side applied {applied_steps} steps of {steps}");
}

#[test]
fn with_the_worker_answering_within_a_frame_the_counters_show_the_step_before() {
    let mut side = fed_side(|at, _| (at >= 200).then_some(3 + i64::from(at % 5)));
    reach(&mut side, 196);
    reading_settled(&mut side);
    side.set_budget(Duration::ZERO);
    let blocked = fed(&side).blocked();
    step_a_frame(&mut side, 40, |side, before, frame| {
        let applied = fed(side).applied_hour().unwrap();
        let shown = frame.layers[1].shown.expect("the counters show an hour");
        assert!(shown >= before && shown <= applied, "applied {applied}, the frame before {before}: the counters show {shown}");
        // The worker answers before the next frame: what a frame's time gives it.
        while !fed(side).answered() {
            std::thread::yield_now();
        }
    });
    assert_eq!(fed(&side).blocked(), blocked, "the frame thread never waited for the worker");
}

#[test]
fn with_the_worker_held_the_frame_never_waits_shows_the_product_it_has_and_catches_up_in_one_take() {
    use crate::layers::Published;
    let mut side = fed_side(|at, _| (at >= 200).then_some(3 + i64::from(at % 5)));
    reach(&mut side, 196);
    reading_settled(&mut side);
    side.set_budget(Duration::ZERO);
    let had = fed(&side).applied_hour().unwrap();
    let product = Arc::clone(fed(&side).stacks().unwrap());
    let (blocked, late) = (fed(&side).blocked(), fed(&side).tally().late);
    fed(&side).hold_worker(true);
    let held_steps = 6;
    step_a_frame(&mut side, held_steps, |side, _, frame| {
        assert_eq!(frame.layers[1].shown, Some(had), "held, the counters show the hour of the product they have");
        let layer = fed(side);
        if layer.applied_hour() > Some(had) {
            assert_eq!(layer.published(), Published::At(had), "never current while its steps are with the worker");
        }
        assert!(Arc::ptr_eq(layer.stacks().unwrap(), &product));
    });
    assert_eq!(fed(&side).blocked(), blocked, "the frame thread returned at once every frame");
    assert!(fed(&side).tally().late - late >= u64::from(held_steps) - 1, "each frame after the first counted late");
    let applied = fed(&side).applied_hour().unwrap();
    assert_eq!(applied, had + 4 * held_steps as i32);
    let taken = fed(&side).taken();
    fed(&side).hold_worker(false);
    while !fed(&side).answered() {
        std::thread::yield_now();
    }
    let frame = side.frame(applied).unwrap();
    assert_eq!(fed(&side).taken(), taken + 1, "the steps queued while it was held are one product, taken at once");
    assert_eq!(frame.layers[1].shown, Some(applied), "and the counters show the hour applied");
    assert_eq!(fed(&side).published(), Published::Current);
    assert_eq!(fed(&side).blocked(), blocked);
    fed_equals_composed(&side);
}

#[test]
fn a_product_whose_content_changed_has_a_new_address_and_only_the_changed_source_a_new_allocation() {
    let mut side = fed_side(|at, slot| (at == 250 && slot == 0).then_some(2));
    reach(&mut side, 240);
    reading_settled(&mut side);
    let first = Arc::clone(fed(&side).stacks().unwrap());
    assert_eq!(first.len(), 2, "901's stack and 902's");
    reach(&mut side, 244);
    reading_settled(&mut side);
    assert!(fed(&side).applied_hour().unwrap() < 250);
    assert!(Arc::ptr_eq(fed(&side).stacks().unwrap(), &first), "a step changing no stack keeps the product's address");
    reach(&mut side, 252);
    reading_settled(&mut side);
    let changed = Arc::clone(fed(&side).stacks().unwrap());
    assert!(!Arc::ptr_eq(&changed, &first), "a step changing 901's stack: a new product");
    assert_eq!(changed.keys().collect::<Vec<_>>(), first.keys().collect::<Vec<_>>());
    for (source, members) in changed.iter() {
        let held = &first[source];
        if source.0 == 1 {
            assert!(!Arc::ptr_eq(members, held) && members != held, "901's source, changed, is a new allocation");
        } else {
            assert!(Arc::ptr_eq(members, held), "source {source:?}, not changed, keeps its allocation");
        }
    }
    fed_equals_composed(&side);
}
