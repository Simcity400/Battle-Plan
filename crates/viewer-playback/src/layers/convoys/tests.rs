use super::{Convoys, Provinces, Read, Route, Routes};
use crate::{
    layers::{
        Layer, View,
        recorded::{self, column, frames_until, through},
    },
    playhead_host::{MapSide, open},
};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use viewer_foundation::map::perspective::{Choice, Vision};
use viewer_world::map::trade_routes::{self, Countries, ExchangeOrder, Stops, TradeRoutes};

/// Today's lines over the values, their convoys summed into the report, and each route's stops
/// asked for and counted; no content geometry.
#[derive(Default)]
struct Lines {
    searches: AtomicUsize,
}

impl Routes for Lines {
    fn stops(&self, _route: &Route, _reverse: bool, _provinces: &Provinces) -> Result<Option<Stops>, String> {
        self.searches.fetch_add(1, Ordering::Relaxed);
        Ok(None)
    }

    fn build(
        &self,
        _ours: u32,
        tables: Read<'_>,
        countries: &Countries<'_>,
        _exchanges: &BTreeMap<u32, ExchangeOrder>,
        _culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
        provinces: Result<&Provinces, String>,
        stops: &mut dyn FnMut(&Route, bool) -> Result<Option<Stops>, String>,
        _emit: &mut trade_routes::Emit<'_>,
    ) -> TradeRoutes {
        let mut out = TradeRoutes::default();
        match (provinces, tables) {
            (Err(why), _) | (_, Err(why)) => out.report.unavailable = Some(why),
            (Ok(_), Ok(tables)) => {
                let lines = trade_routes::lines(&tables, countries, &mut out.report);
                for line in &lines {
                    let _ = stops(line.route, line.reverse);
                    out.report.convoys += line.convoys_assigned as usize;
                }
                out.report.drawn = lines.len();
            }
        }
        out
    }
}

const TABLES: [TableId; 7] = [
    TableId::ResourceOrigins,
    TableId::ConvoyClients,
    TableId::TradeRoutes,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::CountryLinks,
    TableId::Provinces,
];
const HOURS: u32 = 320;

fn client(store: &mut Store, slot: u32, handle: u32, convoys: i32, ordinal: u32) {
    let mut clients = store.table(TableId::ConvoyClients.index()).unwrap();
    let c = |field| column(TableId::ConvoyClients, field);
    clients.insert_at(slot, u128::from(handle)).unwrap();
    for (field, value) in [("owner", 7), ("other", 8), ("route_country", 7), ("route_other", 8)] {
        clients.write(slot, c(field), Cell::U32(value)).unwrap();
    }
    clients.write(slot, c("category"), Cell::U8(trade_routes::IMPORT)).unwrap();
    clients.write(slot, c("route_class"), Cell::U8(1)).unwrap();
    clients.write(slot, c("bad"), Cell::Bool(false)).unwrap();
    clients.write(slot, c("convoys_assigned"), Cell::I32(convoys)).unwrap();
    clients.write(slot, c("owner_ordinal"), Cell::OptionalU32(Some(ordinal))).unwrap();
    clients.write(slot, c("other_ordinal"), Cell::OptionalU32(Some(ordinal))).unwrap();
}

fn route(store: &mut Store, slot: u32, key: (u32, u8, u32), path: &[u32]) {
    let mut routes = store.table(TableId::TradeRoutes.index()).unwrap();
    let c = |field| column(TableId::TradeRoutes, field);
    routes.insert_at(slot, u128::from(key.0) << 40 | u128::from(key.1) << 32 | u128::from(key.2)).unwrap();
    routes.write(slot, c("kind"), Cell::U8(trade_routes::NAVAL_ROUTE)).unwrap();
    routes.write(slot, c("from"), Cell::U32(1)).unwrap();
    routes.write(slot, c("to"), Cell::U32(2)).unwrap();
    routes.write(slot, c("path"), Cell::U32List(path)).unwrap();
    routes.write(slot, c("source_state"), Cell::OptionalU32(Some(10))).unwrap();
    routes.write(slot, c("destination_state"), Cell::OptionalU32(Some(20))).unwrap();
}

/// The viewed country (link 7, slot 3) with a resource origin and an import from link 8, their two
/// routes, an exchange and a sea region's efficiency; the import's convoys change twice in two
/// hours, the client is replaced by another, and ConvoyClients is not recorded for a stretch.
fn campaign(at: u32, store: &mut Store) {
    for table in TABLES {
        let status = if table == TableId::ConvoyClients && (180..220).contains(&at) { Status::NotRecorded } else { Status::Complete };
        store.status(table.index(), status, true, Some(at as i32), true).unwrap();
    }
    match at {
        0 => {
            let mut links = store.table(TableId::CountryLinks.index()).unwrap();
            for (slot, (key, canonical)) in [(7u32, 3u32), (8, 4)].into_iter().enumerate() {
                links.insert_at(slot as u32, u128::from(key)).unwrap();
                links.write(slot as u32, column(TableId::CountryLinks, "canonical_slot"), Cell::U32(canonical)).unwrap();
            }
            let mut origins = store.table(TableId::ResourceOrigins.index()).unwrap();
            let c = |field| column(TableId::ResourceOrigins, field);
            origins.insert_at(0, 50u128 << 32 | 7).unwrap();
            origins.write(0, c("list"), Cell::U8(trade_routes::OWN_ORIGINS)).unwrap();
            origins.write(0, c("ordinal"), Cell::U32(0)).unwrap();
            origins.write(0, c("bad"), Cell::Bool(false)).unwrap();
            origins.write(0, c("convoys_assigned"), Cell::I32(1)).unwrap();
            route(store, 0, (7, trade_routes::ORIGIN_ROUTE_CLASS, 50), &[1, 2]);
            route(store, 1, (7, 1, 8), &[3]);
            client(store, 0, 1, 2, 0);
            let mut regional = store.table(TableId::RegionalConvoyEfficiency.index()).unwrap();
            regional.insert_at(0, 7u128 << 32 | 1).unwrap();
            regional.write(0, column(TableId::RegionalConvoyEfficiency, "efficiency"), Cell::I64(50_000)).unwrap();
            let mut exchanges = store.table(TableId::ResourceExchanges.index()).unwrap();
            exchanges.insert_at(0, 1u128 << 32 | 5).unwrap();
            for field in ["resource", "import_ordinal", "export_ordinal"] {
                exchanges.write(0, column(TableId::ResourceExchanges, field), Cell::U32(0)).unwrap();
            }
            let mut provinces = store.table(TableId::Provinces.index()).unwrap();
            provinces.insert_at(0, 1).unwrap();
            provinces.write(0, column(TableId::Provinces, "state"), Cell::U32(10)).unwrap();
            provinces.write(0, column(TableId::Provinces, "port_sea"), Cell::I32(0)).unwrap();
        }
        50 | 51 => {
            let mut clients = store.table(TableId::ConvoyClients.index()).unwrap();
            clients.write(0, column(TableId::ConvoyClients, "convoys_assigned"), Cell::I32(if at == 50 { 3 } else { 4 })).unwrap();
        }
        130 => {
            store.table(TableId::ConvoyClients.index()).unwrap().delete(0).unwrap();
            client(store, 1, 2, 6, 1);
        }
        _ => {}
    }
}

fn side_seeing(name: &str, seeing: bool) -> (MapSide, Arc<Lines>) {
    let entry = open(recorded::record(name, HOURS, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let routes = Arc::new(Lines::default());
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Convoys::new(Arc::clone(&routes) as Arc<dyn Routes>))], supply()).unwrap();
    if seeing {
        layer(&side).set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    }
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    (side, routes)
}

/// A view in the supply map mode, which draws trade routes.
fn supply() -> View {
    View { map_mode: viewer_world::map::map_mode::MapMode::parse("supply").unwrap(), ..View::default() }
}

fn side(name: &str) -> (MapSide, Arc<Lines>) {
    side_seeing(name, true)
}

fn layer(side: &MapSide) -> &Convoys {
    side.parts().unwrap().1[0].any().downcast_ref::<Convoys>().unwrap()
}

fn composed_equal(side: &MapSide) {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

/// Frames with the head never past `cap` until `done` holds: the playhead decodes on, the map does
/// not take a step ending after `cap`.
fn frames_within(side: &mut MapSide, cap: i32, mut done: impl FnMut(&MapSide, &crate::playhead_host::Frame) -> bool) {
    for _ in 0..50_000_000u64 {
        let head = side.entry().playhead().newest().unwrap_or(0).min(cap);
        let frame = side.frame(head).unwrap();
        if done(side, &frame) {
            return;
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    panic!("the map side never reached what the test waits for with the head at most {cap}");
}

/// The viewed country's lines and the convoys they carry.
fn drawn(side: &MapSide) -> (usize, usize) {
    let report = &layer(side).product().report;
    (report.drawn, report.convoys)
}

#[test]
fn a_row_changed_in_a_step_is_taken_and_the_product_equals_the_composed_one() {
    let (mut side, _) = side("convoys-once");
    assert_eq!(drawn(&side), (2, 3), "the origin's line and the import's");
    composed_equal(&side);
    through(&mut side, 64);
    side.settle_all().unwrap();
    assert_eq!(drawn(&side), (2, 5));
    composed_equal(&side);
}

#[test]
fn a_row_changed_twice_before_the_layer_settles_shows_its_newest_values() {
    let (mut side, _) = side("convoys-twice");
    through(&mut side, 30);
    let settles = side.hold_settles();
    // Both changes (hours 50 and 51) marked before any settle; the head stays at hour 56 from here,
    // far before the client replaced at hour 130. Then, with no settling, a unit a frame.
    frames_within(&mut side, 56, |_, frame| frame.applied.is_some_and(|applied| applied >= 51));
    crate::layers::recorded::released_unit_by_unit(&mut side, settles);
    // The product is made off the frame: it arrives a frame or more after it is asked for.
    frames_within(&mut side, 56, |side, _| layer(side).carried().entities == 0 && layer(side).published() == crate::layers::Published::Current);
    let at = side.parts().unwrap().0.get(TableId::ConvoyClients).and_then(crate::playhead_host::mirror::Mirror::head).expect("a head");
    assert!((51..=56).contains(&at), "the mirrors at hour {at}");
    assert_eq!(drawn(&side), (2, 5), "the newest convoys");
    composed_equal(&side);
}

#[test]
fn a_row_deleted_or_inserted_updates_the_lines_that_name_it_and_stops_are_kept() {
    let (mut side, routes) = side("convoys-churn");
    through(&mut side, 134);
    side.settle_all().unwrap();
    assert_eq!(drawn(&side), (2, 7), "the new client's convoys");
    composed_equal(&side);
    assert_eq!(routes.searches.load(Ordering::Relaxed), 2 + 2, "each route's stops searched once per layer: this one and the composed copy");
}

#[test]
fn a_table_not_recorded_is_drawn_as_today_draws_it() {
    let (mut side, _) = side("convoys-gap");
    through(&mut side, 184);
    side.settle_all().unwrap();
    let unavailable = layer(&side).product().report.unavailable.clone();
    assert!(unavailable.as_deref().is_some_and(|why| why.contains("ConvoyClients") && why.contains("not recorded")), "{unavailable:?}");
    assert_eq!(drawn(&side), (0, 0), "never the last rows");
    composed_equal(&side);
    through(&mut side, 244);
    side.settle_all().unwrap();
    assert_eq!(drawn(&side), (2, 7));
    composed_equal(&side);
}

#[test]
fn a_layer_given_no_vision_is_refused_by_name_and_composes_whole_once_given_one() {
    let (mut side, _) = side_seeing("convoys-vision", false);
    side.settle_all().unwrap();
    assert!(layer(&side).refused().is_some_and(|why| why.contains("convoys") && why.contains("vision")));
    through(&mut side, 64);
    layer(&side).set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    side.settle_all().unwrap();
    assert_eq!(layer(&side).refused(), None);
    assert_eq!(drawn(&side), (2, 5));
    composed_equal(&side);
}

#[test]
fn a_map_mode_that_draws_no_trade_routes_declares_nothing_and_one_that_does_composes() {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut hidden = Convoys::new(Arc::new(Lines::default()));
    let mut inputs = crate::layers::Inputs::new(&kinds, "convoys");
    hidden.inputs(&View::default(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the default mode draws no trade route");
    let (mut side, _) = side("convoys-mode");
    through(&mut side, 30);
    side.settle_all().unwrap();
    // Hidden while the convoys changed at hours 50 and 51 wait for a product.
    through(&mut side, 52);
    side.set_view(View::default()).unwrap();
    through(&mut side, 140);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).carried().entities, 0, "a view that draws nothing carries nothing, though a step changed its tables");
    assert_eq!(layer(&side).held(), Convoys::new(Arc::new(Lines::default())).held(), "and holds what an empty layer holds");
    assert_eq!(drawn(&side), (0, 0), "it holds nothing");
    assert!(layer(&side).product().convoys.is_empty());
    composed_equal(&side);
    side.set_view(supply()).unwrap();
    // Its tables come back as resets in a step built once their lanes are prepared, which may be
    // past the recording's end: until then it is not drawn.
    let last = HOURS as i32 - 1;
    frames_until(&mut side, |side, frame| (!side.composing() && side.parts().is_some_and(|_| layer(side).refused().is_none())) || frame.applied == Some(last));
    if layer(&side).refused().is_none() {
        side.settle_all().unwrap();
        composed_equal(&side);
    } else {
        assert!(!side.draws("convoys"), "a layer whose tables have not come back is not drawn");
    }
}

/// Today's lines, held on the worker until the test opens the gate (a minute at most, after which
/// the build goes on and the test's assertions fail by name).
#[derive(Default)]
struct HeldLines {
    lines: Lines,
    open: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

impl HeldLines {
    fn set(&self, open: bool) {
        *self.open.lock().unwrap() = open;
        self.changed.notify_all();
    }
}

impl Routes for HeldLines {
    fn stops(&self, route: &Route, reverse: bool, provinces: &Provinces) -> Result<Option<Stops>, String> {
        self.lines.stops(route, reverse, provinces)
    }

    fn build(
        &self,
        ours: u32,
        tables: Read<'_>,
        countries: &Countries<'_>,
        exchanges: &BTreeMap<u32, ExchangeOrder>,
        culture: &dyn Fn(&Countries<'_>) -> Result<String, String>,
        provinces: Result<&Provinces, String>,
        stops: &mut dyn FnMut(&Route, bool) -> Result<Option<Stops>, String>,
        emit: &mut trade_routes::Emit<'_>,
    ) -> TradeRoutes {
        let until = std::time::Instant::now() + Duration::from_secs(60);
        let mut open = self.open.lock().unwrap();
        while !*open {
            let Some(left) = until.checked_duration_since(std::time::Instant::now()) else { break };
            open = self.changed.wait_timeout(open, left).unwrap().0;
        }
        drop(open);
        self.lines.build(ours, tables, countries, exchanges, culture, provinces, stops, emit)
    }
}

/// While a job is out (the convoys changed at hours 50 and 51) the layer carries nothing for it and
/// publishes the hour its product stands for, which the host shows; answered, it is current again.
/// On the base the job out was carried work and the layer published `Current`, so the host kept the
/// hour it last carried nothing at.
#[test]
fn a_job_out_is_not_carried_and_the_layer_publishes_the_hour_its_product_stands_for() {
    use crate::layers::Published;
    let entry = open(recorded::record("convoys-job-out", HOURS, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let routes = Arc::new(HeldLines::default());
    routes.set(true);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Convoys::new(Arc::clone(&routes) as Arc<dyn Routes>))], supply()).unwrap();
    layer(&side).set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    through(&mut side, 30);
    side.settle_all().unwrap();
    assert_eq!((layer(&side).published(), layer(&side).carried().entities), (Published::Current, 0));
    routes.set(false);
    let at = |side: &MapSide| side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::ConvoyClients).and_then(crate::playhead_host::mirror::Mirror::head));
    frames_within(&mut side, 56, |side, _| at(side).is_some_and(|hour| hour >= 51) && layer(side).in_flight > 0 && layer(side).carried().entities == 0);
    let stands = layer(&side).product_hour.expect("a product");
    assert!((30..50).contains(&stands), "the product stands for the rows before the change, at {stands}");
    assert_eq!(layer(&side).published(), Published::At(stands));
    assert_eq!(side.last_frame().layers[0].shown, Some(stands), "the host shows the hour the product stands for");
    routes.set(true);
    frames_within(&mut side, 56, |side, _| layer(side).in_flight == 0 && layer(side).carried().entities == 0);
    assert_eq!(layer(&side).published(), Published::Current);
    assert_eq!(drawn(&side), (2, 5), "the newest convoys");
}
