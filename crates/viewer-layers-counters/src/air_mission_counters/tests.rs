use super::{AirMissionCounters, Content, Passes, TABLES};
use viewer_layer_core::{layers::{Layer, Published, View}, playhead_host::{MapSide, open, units::Units}};
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
use replay_data::{
    BranchId, Hour,
    rows::{AirCombatDamage, AirCombatRecord, AirSupplySummary, Field, NativeValue, Opaque, TableId},
};
use std::{
    io,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU32, Ordering},
    },
};
use viewer_counters_naval::air_missions::{AirPasses, Ground, counts_withheld};
use viewer_foundation::perspective::{Choice, Country, Vision};
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
        let strings: &[&[u8]] = if at == 0 { &[b"not captured"] } else { &[] };
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
    Units { name: format!("air missions {}", MADE.fetch_add(1, Ordering::Relaxed)), pages, timeline, kinds: Arc::new(kinds) }
}

fn write(store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>) {
    store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
}

fn insert(store: &mut Store, table: TableId, slot: u32, key: u128) {
    store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
}

fn delete(store: &mut Store, table: TableId, slot: u32) {
    store.table(table.index()).unwrap().delete(slot).unwrap();
}

fn opaque<T: NativeValue>(value: T) -> Vec<u8> {
    let Field::Opaque(Opaque::Native(value)) = value.field() else { panic!("an opaque value") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

const INTERCEPT: u32 = 1 << 9;

fn wing(store: &mut Store, slot: u32, id: u64, owner: u32, region: u32, planes: i32, mask: u32, ordinal: u32) {
    insert(store, TableId::AirWings, slot, u128::from(id));
    write(store, TableId::AirWings, slot, "owner", Cell::U32(owner));
    write(store, TableId::AirWings, slot, "planes", Cell::I32(planes));
    write(store, TableId::AirWings, slot, "mission_mask", Cell::U32(mask));
    write(store, TableId::AirWings, slot, "mission_region", Cell::OptionalU32(Some(region)));
    insert(store, TableId::AirRegionalMissions, slot, u128::from(id));
    write(store, TableId::AirRegionalMissions, slot, "ordinal", Cell::U32(ordinal));
    insert(store, TableId::AirMissionInputs, slot, u128::from(id));
    write(store, TableId::AirMissionInputs, slot, "interception_alert_gate", Cell::OptionalU32(Some(1)));
}

fn summary(severity: i32) -> AirSupplySummary {
    AirSupplySummary { severity, needed: 10, supplied: 5, deficit: 5 }
}

fn write_supply(store: &mut Store, slot: u32, summary: AirSupplySummary) {
    write(store, TableId::AirGroundCrews, slot, "supply_severity", Cell::OptionalI32(Some(summary.severity)));
    write(store, TableId::AirGroundCrews, slot, "supply_needed", Cell::OptionalI64(Some(summary.needed)));
    write(store, TableId::AirGroundCrews, slot, "supply_supplied", Cell::OptionalI64(Some(summary.supplied)));
    write(store, TableId::AirGroundCrews, slot, "supply_deficit_extra", Cell::OptionalI64(Some(summary.deficit_extra())));
}

fn records(dates: &[i32]) -> Vec<u8> {
    let records: Vec<AirCombatRecord> = dates
        .iter()
        .map(|&date| AirCombatRecord { date, planes: 1, mission_mask: INTERCEPT, classification: 1, second_damage: vec![AirCombatDamage { cause: 9, amount: 100 }], ..Default::default() })
        .collect();
    opaque(Arc::new(records))
}

fn holder(store: &mut Store, dates: &[i32]) {
    if store.table(TableId::AirRegionCombat.index()).unwrap().slot(10 << 32 | 3).is_none() {
        insert(store, TableId::AirRegionCombat, 0, 10 << 32 | 3);
        write(store, TableId::AirRegionCombat, 0, "foreign", Cell::Opaque(&records(&[])));
    }
    write(store, TableId::AirRegionCombat, 0, "own", Cell::Opaque(&records(dates)));
}

fn world(store: &mut Store) {
    for (slot, key) in [(0, 10u128), (1, 11)] {
        insert(store, TableId::CountryLinks, slot, key);
        write(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot));
        insert(store, TableId::Countries, slot, key);
        write(store, TableId::Countries, slot, "canonical_slot", Cell::U32(slot));
        write(store, TableId::Countries, slot, "command_power_raw", Cell::I64(500_000));
    }
    wing(store, 0, 900, 10, 3, 50, INTERCEPT, 0);
    wing(store, 1, 901, 10, 3, 20, INTERCEPT | 1 << 2, 1);
    wing(store, 2, 902, 11, 3, 7, 1, 0);
    insert(store, TableId::AirGroundCrews, 0, u128::from(u32::MAX));
    write_supply(store, 0, summary(2));
    insert(store, TableId::AirGroundCrews, 1, 3);
    write(store, TableId::AirGroundCrews, 1, "enabled", Cell::OptionalU32(Some(0)));
    write_supply(store, 1, summary(1));
}

fn quick() -> Settings {
    Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }
}

fn air_view() -> View {
    View { map_mode: MapMode::Stock(Stock::StrategicAir), fog: false, ..View::default() }
}

struct Held {
    open: Mutex<bool>,
    signal: Condvar,
    visited: bool,
}

impl Held {
    fn new(open: bool, visited: bool) -> Arc<Self> {
        Arc::new(Self { open: Mutex::new(open), signal: Condvar::new(), visited })
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.signal.notify_all();
    }
}

impl AirPasses for Held {
    fn visited(&self, _branch: BranchId, _pass: Hour, _slot: u32) -> Result<bool, String> {
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.signal.wait(open).unwrap();
        }
        Ok(self.visited)
    }
}

fn layer_with(timeout: i32, passes: Passes) -> AirMissionCounters {
    let layer = AirMissionCounters::new(Arc::new(Content::of(Ok(timeout), Ok((100_000, 5)))));
    layer.set_vision(Choice { country: Country::Recorded, fog: false }, Arc::new(Vision::sharing(0, []).without_fog()));
    layer.set_passes(passes, 0);
    layer
}

fn side_with(units: Units, layer: AirMissionCounters) -> MapSide {
    let entry = open(units, quick()).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], air_view()).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn side_for(units: Units) -> MapSide {
    side_with(units, layer_with(1000, Held::new(true, false)))
}

fn frames_until(side: &mut MapSide, mut done: impl FnMut(&MapSide, &viewer_layer_core::playhead_host::Frame) -> bool) {
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

fn reach_within(side: &mut MapSide, hour: i32, cap: i32) -> Hour {
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(cap);
        let frame = side.frame(head).unwrap();
        if !side.composing() && frame.applied.is_some_and(|applied| applied >= hour) {
            let at = side.parts().unwrap().0.get(TableId::AirRegionalMissions).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head).expect("a head");
            assert!((hour..=cap).contains(&at), "the mirrors at hour {at}, outside {hour} to {cap}");
            return at;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached hour {hour} with the head at most {cap}");
}

fn hold(side: &mut MapSide, hour: i32) {
    while side.entry().playhead().newest().is_none_or(|newest| newest < hour) {
        let head = side.entry().playhead().newest().unwrap_or(0).min(hour);
        side.frame(head).unwrap();
        std::thread::yield_now();
    }
    side.frame(hour).unwrap();
}

fn layer(side: &MapSide) -> &AirMissionCounters {
    side.parts().unwrap().1[0].any().downcast_ref::<AirMissionCounters>().unwrap()
}

fn settled_equals_composed(side: &mut MapSide) {
    side.settle_all().unwrap();
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn counts(side: &MapSide, region: u32) -> Option<[i32; 18]> {
    layer(side).values().get(&region).map(|value| value.counts)
}

fn phase(side: &MapSide, name: &str) -> u64 {
    layer(side).phases().find(|(held, _, _)| *held == name).map_or(0, |(_, entities, _)| entities)
}

#[test]
fn a_region_changed_in_a_step_is_marked_once_and_settles_to_the_composed_value() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            write(store, TableId::AirWings, 0, "planes", Cell::I32(30));
            write(store, TableId::AirWings, 1, "planes", Cell::I32(40));
        }
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    settled_equals_composed(&mut side);
    let first = counts(&side, 3).expect("region 3 counts our wings");
    assert_eq!((first[9], first[2], first[10], first[0]), (70, 20, 0, -1), "the other country's wing is not ours; low supply counts the supply entry");
    let value = layer(&side).values()[&3].clone();
    assert_eq!((value.alerts, value.uncertain.clone(), value.supply.map(|supply| supply.severity)), (0, vec![], Some(1)), "no holder: no alert, nothing withheld");
    assert_eq!(value.ground, Some(Ok(Ground { frame: 1, enabled: true })), "not active, with the power to pay");
    let marked = layer(&side).marked();
    reach(&mut side, 204);
    assert_eq!(layer(&side).marked() - marked, 1, "two wings of one region mark it once");
    settled_equals_composed(&mut side);
    assert_eq!((counts(&side, 3).unwrap()[9], counts(&side, 3).unwrap()[2]), (70, 40));
}

#[test]
fn a_region_changed_twice_before_it_settles_shows_its_newest_values() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::AirWings, 0, "planes", Cell::I32(1)),
        202 => write(store, TableId::AirWings, 0, "planes", Cell::I32(2)),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.settle_all().unwrap();
    let settles = side.hold_settles();
    reach(&mut side, 204);
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    settled_equals_composed(&mut side);
    assert_eq!(counts(&side, 3).unwrap()[9], 22);
}

#[test]
fn a_registration_deleted_and_another_inserted_updates_every_region_naming_them() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => delete(store, TableId::AirRegionalMissions, 1),
        204 => wing(store, 3, 903, 10, 4, 9, 1 << 2, 2),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 200);
    settled_equals_composed(&mut side);
    assert_eq!((counts(&side, 3).unwrap()[9], counts(&side, 3).unwrap()[2]), (50, -1), "the unregistered wing counts nowhere");
    reach(&mut side, 208);
    settled_equals_composed(&mut side);
    assert_eq!(counts(&side, 4).unwrap()[2], 9, "a new region of ours");
    assert_eq!(layer(&side).values()[&4].supply, None, "a region without its supply row");
}

#[test]
fn the_first_refused_registration_in_order_withholds_the_counts_whatever_its_owner() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => {
            insert(store, TableId::AirRegionalMissions, 3, 905);
            write(store, TableId::AirRegionalMissions, 3, "ordinal", Cell::U32(5));
            wing(store, 4, 906, 99, 3, 1, 1, 2);
        }
        210 => delete(store, TableId::AirRegionalMissions, 4),
        220 => delete(store, TableId::AirRegionalMissions, 3),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 204);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).withheld(), Some(counts_withheld("registered wing 906 has an unresolved owner").as_str()), "the earlier of two refusals, another owner's");
    reach(&mut side, 212);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).withheld(), Some(counts_withheld("registered wing 905 has no AirWings row").as_str()));
    reach(&mut side, 224);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).withheld(), None);
}

#[test]
fn tables_not_wholly_recorded_draw_as_today_draws_them() {
    let units = record(BLOCK, |at, table| match (at, table) {
        (240..=247, TableId::AirWings) => Status::Truncated { missing: 1 },
        (260..=267, TableId::AirRegionCombat) => Status::Unavailable { reason: 0 },
        (280..=287, TableId::AirGroundCrews) => Status::NotRecorded,
        _ => Status::Complete,
    }, |at, store| match at {
        0 => world(store),
        200 => holder(store, &[200]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 244);
    settled_equals_composed(&mut side);
    assert!(layer(&side).withheld().is_some_and(|why| why.contains("AirWings is truncated (1 missing rows)")), "{:?}", layer(&side).withheld());
    reach(&mut side, 264);
    settled_equals_composed(&mut side);
    let value = layer(&side).values()[&3].clone();
    assert!(value.uncertain.contains(&(9, "the regional combat census is incomplete")), "{:?}", value.uncertain);
    let first = reach_within(&mut side, 280, 283);
    settled_equals_composed(&mut side);
    let value = layer(&side).values()[&3].clone();
    assert_eq!(layer(&side).supply(), (false, false), "no supply census, no supply");
    assert_eq!((value.counts[10], value.supply), (-1, None));
    assert!(matches!(&value.ground, Some(Err(why)) if why.contains("AirGroundCrews")), "{:?}", value.ground);
    let head = reach_within(&mut side, first + 1, 287);
    settled_equals_composed(&mut side);
    let value = layer(&side).values()[&3].clone();
    assert!(matches!(&value.ground, Some(Err(why)) if why.starts_with(&format!("hour {head} AirGroundCrews"))), "at hour {head}: {:?}", value.ground);
    reach(&mut side, 292);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values()[&3].ground, Some(Ok(Ground { frame: 1, enabled: true })), "after the gap, the control again");
    assert!(layer(&side).hourly.is_empty(), "the table whole again, no region names the hour");
}

#[test]
fn the_alert_follows_the_window_and_the_hour_touches_only_regions_with_a_holder() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => {
            world(store);
            wing(store, 3, 903, 10, 4, 9, INTERCEPT, 2);
        }
        205 => holder(store, &[205]),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 212);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values()[&3].alerts, INTERCEPT, "a record of the last day proves the alert");
    assert_eq!(layer(&side).values()[&4].alerts, 0, "a region without a holder has none");
    reach(&mut side, 232);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values()[&3].alerts, 0, "the record left the window at hour 230, its row unchanged");
    let hourly = phase(&side, "by the hour");
    reach(&mut side, 300);
    settled_equals_composed(&mut side);
    assert_eq!(phase(&side, "by the hour") - hourly, 3, "only the region with a holder, once at each of the passes at 240, 264 and 288");
}

#[test]
fn an_alert_waiting_for_the_daily_pass_is_undecided_and_settles_when_it_is_answered() {
    let passes = Held::new(false, true);
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        230 => holder(store, &[230]),
        _ => {}
    });
    let mut side = side_with(units, layer_with(0, Arc::clone(&passes) as Passes));
    reach(&mut side, 244);
    let value = layer(&side).values()[&3].clone();
    assert!(value.uncertain.contains(&(9, "whether the latest daily air pass pruned the regional combat records is unrecorded")), "undecided while the worker reads: {:?}", value.uncertain);
    assert!(!layer(&side).waiting.is_empty(), "the region waits for the answer");
    passes.release();
    frames_until(&mut side, |side, frame| frame.steps > 0 && layer(side).waiting.is_empty() && layer(side).carried().entities == 0);
    settled_equals_composed(&mut side);
    let value = layer(&side).values()[&3].clone();
    assert_eq!((value.alerts, value.uncertain), (0, vec![]), "the pass visited us and pruned the record");
}

#[test]
fn a_view_that_hides_the_layer_declares_nothing_and_shown_again_composes() {
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write(store, TableId::AirWings, 0, "planes", Cell::I32(3)),
        _ => {}
    });
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "air_mission_counters");
    layer_with(1000, Held::new(true, false)).inputs(&View::default(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the default mode draws no air missions");
    let mut side = side_for(units);
    reach(&mut side, 150);
    side.set_view(View::default()).unwrap();
    assert_eq!(side.refusals().count(), 0, "hidden, not refused");
    hold(&mut side, 206);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though steps changed its tables");
    assert_eq!(Layer::published(layer(&side)), Published::Current, "and owes nothing");
    assert_eq!(layer(&side).held(), layer_with(1000, Held::new(true, false)).held(), "and holds what an empty layer holds");
    assert!(layer(&side).values().is_empty(), "nothing is kept while hidden");
    assert!(layer(&side).published().is_none(), "and the window is given no air mission counter to draw");
    settled_equals_composed(&mut side);
    side.set_view(air_view()).unwrap();
    frames_until(&mut side, |side, frame| frame.steps > 0 && !side.composing() && side.parts().is_some_and(|(mirrors, _)| mirrors.get(TableId::AirWings).is_some()));
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).refused(), None);
    assert!(counts(&side, 3).is_some(), "composed again from the mirrors it is given");
    let published = layer(&side).published().expect("shown again, the window is given its values");
    assert_eq!(published.regions.len(), layer(&side).values().len(), "the regions the window draws are the layer's");
    assert_eq!(published.regions.get(&3), layer(&side).values().get(&3).map(|value| &value.counts), "with the layer's counts");
}

fn within(side: &mut MapSide, cap: i32, what: &str, mut done: impl FnMut(&MapSide) -> bool) {
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(cap);
        let frame = side.frame(head).unwrap();
        if done(side) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    let shown = side.last_frame().layers.first().and_then(|part| part.shown);
    let applied = side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::AirRegionalMissions).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head));
    let layer = layer(side);
    panic!(
        "never {what}: the layer published {:?}, carried {}, {} regions waiting; the host showed {shown:?} for it, the mirrors applied to {applied:?}",
        Layer::published(layer),
        layer.carried().entities,
        layer.waiting.values().map(std::collections::BTreeSet::len).sum::<usize>(),
    );
}

#[test]
fn a_question_out_on_the_daily_pass_worker_is_not_carried_and_the_hour_its_counts_stand_for_is_shown() {
    let passes = Held::new(false, true);
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        230 => holder(store, &[230]),
        _ => {}
    });
    let mut side = side_with(units, layer_with(0, Arc::clone(&passes) as Passes));
    reach(&mut side, 244);
    assert!(!layer(&side).waiting.is_empty() && layer(&side).carried().entities == 0, "a question out with nothing carried");
    let stands = match Layer::published(layer(&side)) {
        Published::At(hour) => hour,
        other => panic!("a question out publishes the hour the counts stand for, not {other:?}"),
    };
    assert!(stands < 240, "the counts stand for the rows before the pass at 240, at {stands}");
    assert_eq!(side.last_frame().layers[0].shown, Some(stands), "the host shows that hour");
    passes.release();
    within(&mut side, 247, "the question answered, current, nothing carried", |side| layer(side).waiting.is_empty() && layer(side).carried().entities == 0 && Layer::published(layer(side)) == Published::Current);
    settled_equals_composed(&mut side);
}

#[test]
fn scalar_air_supply_updates_reconstruct_mixed_surplus_deficit_on_step() {
    let mixed = AirSupplySummary { severity: 0, needed: 100, supplied: 120, deficit: 15 };
    let units = record(BLOCK, |_, _| Status::Complete, |at, store| match at {
        0 => world(store),
        200 => write_supply(store, 1, mixed),
        _ => {}
    });
    let mut side = side_for(units);
    reach(&mut side, 150);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values()[&3].supply, Some(summary(1)));
    reach(&mut side, 204);
    settled_equals_composed(&mut side);
    assert_eq!(layer(&side).values()[&3].supply, Some(mixed));
}
