use super::{Convoys, Provinces, Read, Route, Routes};
use viewer_layer_core::{layers::{Layer, View}, test_support::recorded::{self, column, frames_until, through}, playhead_host::{MapSide, open}};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use viewer_foundation::perspective::{Choice, Vision};
use viewer_unit_models::trade_routes::{self, Countries, ExchangeOrder, Stops, TradeRoutes};

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

fn supply() -> View {
    View { map_mode: viewer_map_modes::map_mode::MapMode::parse("supply").unwrap(), ..View::default() }
}

fn side(name: &str) -> (MapSide, Arc<Lines>) {
    side_seeing(name, true)
}

fn layer(side: &MapSide) -> &Convoys {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<Convoys>()).unwrap()
}

fn vision_layer(side: &MapSide) -> &viewer_layer_core::vision::Vision {
    side.parts().unwrap().1.iter().find_map(|layer| layer.any().downcast_ref::<viewer_layer_core::vision::Vision>()).unwrap()
}

fn window_side(name: &str, routes: Arc<dyn Routes>, refused_hours: std::ops::Range<u32>) -> MapSide {
    use viewer_foundation::perspective::VISION_TABLES;
    let recording = recorded::record_naming(name, HOURS, &[b"AAA", b"BBB"], |at, store| {
        for &table in VISION_TABLES {
            let status = if table == TableId::Countries && refused_hours.contains(&at) { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        campaign(at, store);
        if at == 0 {
            let mut countries = store.table(TableId::Countries.index()).unwrap();
            for (slot, (raw, canonical)) in [(7u32, 3u32), (8, 4)].into_iter().enumerate() {
                let slot = slot as u32;
                countries.insert_at(slot, u128::from(raw)).unwrap();
                countries.write(slot, column(TableId::Countries, "canonical_slot"), Cell::U32(canonical)).unwrap();
                countries.write(slot, column(TableId::Countries, "tag"), Cell::Bytes(slot)).unwrap();
            }
            let mut player = store.table(TableId::LocalPlayer.index()).unwrap();
            player.insert_at(0, 0).unwrap();
            player.write(0, column(TableId::LocalPlayer, "country"), Cell::U32(7)).unwrap();
        }
    });
    let entry = open(recording, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let content = Arc::new(viewer_layer_core::vision::Content::new(Err("no fog content".into())));
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(viewer_layer_core::vision::Vision::new(content)), Box::new(Convoys::new(routes).in_window())];
    let mut side = MapSide::new(Arc::clone(&entry), layers, supply()).unwrap();
    assert_eq!(side.refusals().count(), 0, "{:?}", side.refusals().collect::<Vec<_>>());
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

fn composed_equal(side: &MapSide) {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

fn frames_within(side: &mut MapSide, cap: i32, mut done: impl FnMut(&MapSide, &viewer_layer_core::playhead_host::Frame) -> bool) {
    let cap = cap.min(side.entry().playhead().timeline().last());
    loop {
        assert!(side.entry().playhead().error().is_none(), "the playhead stopped before cap {cap}: {:?}", side.entry().playhead().error());
        recorded::reads_landed(side);
        let before = side.entry().playhead().step_state();
        let head = before.0.unwrap_or(0).min(cap);
        let frame = if side.composing() { side.frame_waiting(head) } else { side.frame(head) }.unwrap();
        if done(side, &frame) {
            return;
        }
        if side.composing() || frame.steps > 0 || frame.swapped.is_some() || frame.superseded {
            continue;
        }
        let more = side
            .wait_step_after(before, cap)
            .unwrap_or_else(|error| panic!("the playhead stopped before cap {cap}: {error}"));
        assert!(
            more,
            "the map side reached hour {:?} (cap {cap}) without reaching what the test waits for: {}",
            side.applied(),
            recorded::seen(side),
        );
    }
}

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
    frames_within(&mut side, 56, |_, frame| frame.applied.is_some_and(|applied| applied >= 51));
    viewer_layer_core::test_support::recorded::released_unit_by_unit(&mut side, settles);
    side.settle_all().unwrap();
    assert_eq!((layer(&side).carried().entities, layer(&side).published()), (0, viewer_layer_core::layers::Published::Current));
    let at = side.parts().unwrap().0.get(TableId::ConvoyClients).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head).expect("a head");
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
    let kinds = replay_recording::schema::schema().unwrap();
    let mut hidden = Convoys::new(Arc::new(Lines::default()));
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "convoys");
    hidden.inputs(&View::default(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "the default mode draws no trade route");
    let (mut side, _) = side("convoys-mode");
    through(&mut side, 30);
    side.settle_all().unwrap();
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
    let last = HOURS as i32 - 1;
    frames_until(&mut side, |side, frame| (!side.composing() && side.parts().is_some_and(|_| layer(side).refused().is_none())) || frame.applied == Some(last));
    if layer(&side).refused().is_none() {
        side.settle_all().unwrap();
        composed_equal(&side);
    } else {
        assert!(!side.draws("convoys"), "a layer whose tables have not come back is not drawn");
    }
}

#[derive(Default)]
struct HeldLines {
    lines: Lines,
    open: std::sync::Mutex<bool>,
    changed: std::sync::Condvar,
}

struct ReleaseRoutes(Arc<HeldLines>);

impl Drop for ReleaseRoutes {
    fn drop(&mut self) {
        self.0.set(true);
    }
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
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.changed.wait(open).unwrap();
        }
        drop(open);
        self.lines.build(ours, tables, countries, exchanges, culture, provinces, stops, emit)
    }
}

#[test]
fn a_job_out_is_not_carried_and_the_layer_publishes_the_hour_its_product_stands_for() {
    use viewer_layer_core::layers::Published;
    let entry = open(recorded::record("convoys-job-out", HOURS, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let routes = Arc::new(HeldLines::default());
    let _release = ReleaseRoutes(Arc::clone(&routes));
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
    let at = |side: &MapSide| side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::ConvoyClients).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head));
    frames_within(&mut side, 56, |side, _| at(side).is_some_and(|hour| hour >= 51) && layer(side).in_flight > 0 && layer(side).carried().entities == 0);
    let stands = layer(&side).product_hour.expect("a product");
    assert!((30..50).contains(&stands), "the product stands for the rows before the change, at {stands}");
    assert_eq!(layer(&side).published(), Published::At(stands));
    assert_eq!(side.last_frame().layers[0].shown, Some(stands), "the host shows the hour the product stands for");
    let waited = layer(&side).blocking_waits;
    side.frame_still(side.applied().expect("a step applied")).unwrap();
    assert_eq!(layer(&side).blocking_waits, waited, "a still publication waits on no convoy job");
    assert!(layer(&side).in_flight > 0, "the job out joins a later publication");
    routes.set(true);
    layer(&side).wait_replies_ready();
    side.frame(side.applied().expect("a step applied")).unwrap();
    assert_eq!(layer(&side).blocking_waits, waited, "an ordinary playing frame takes the ready reply via try_recv without blocking");
    assert_eq!((layer(&side).in_flight, layer(&side).carried().entities), (0, 0));
    assert_eq!(layer(&side).published(), Published::Current);
    assert_eq!(drawn(&side), (2, 5), "the newest convoys");
}

#[test]
fn a_viewing_country_change_withholds_the_lines_made_for_the_country_before() {
    use viewer_foundation::perspective::Country;
    let entry = open(recorded::record("convoys-viewer-change", HOURS, campaign), Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let routes = Arc::new(HeldLines::default());
    let _release = ReleaseRoutes(Arc::clone(&routes));
    routes.set(true);
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Convoys::new(Arc::clone(&routes) as Arc<dyn Routes>))], supply()).unwrap();
    layer(&side).set_vision(Choice::default(), Arc::new(Vision::sharing(3, [])));
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    through(&mut side, 30);
    side.settle_all().unwrap();
    assert!(layer(&side).shown().is_some_and(|(_, lines)| lines.report.drawn == 2), "slot 3's lines are shown for slot 3");
    routes.set(false);
    let at = |side: &MapSide| side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::ConvoyClients).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head));
    frames_within(&mut side, 56, |side, _| at(side).is_some_and(|hour| hour >= 51) && layer(side).in_flight > 0);
    layer(&side).set_vision(Choice { country: Country::Tag("other".into()), ..Choice::default() }, Arc::new(Vision::sharing(7, [])));
    side.frame(side.applied().expect("a step applied")).unwrap();
    assert_eq!(layer(&side).vision.as_ref().map(|&(_, ours)| ours), Some(7));
    assert!(layer(&side).shown().is_none(), "slot 3's lines are withheld for slot 7 while its own are made");
    routes.set(true);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).in_flight, 0);
    assert_eq!(layer(&side).product_for, Some(7));
    assert!(layer(&side).shown().is_some(), "slot 7's own lines are shown");
}

#[test]
fn a_refused_vision_withholds_the_lines_made_for_the_country_before() {
    let routes = Arc::new(Lines::default());
    let mut side = window_side("convoys-refused-viewer", Arc::clone(&routes) as Arc<dyn Routes>, 60..146);
    through(&mut side, 30);
    side.settle_all().unwrap();
    assert_eq!(vision_layer(&side).ours(), Some(3));
    assert!(layer(&side).shown().is_some(), "slot 3's lines are shown for slot 3");
    frames_within(&mut side, 68, |side, _| !side.composing() && side.applied().is_some_and(|hour| hour >= 64));
    side.settle_all().unwrap();
    assert_eq!(vision_layer(&side).ours(), None, "a refused vision has no viewing country");
    assert!(layer(&side).shown().is_none(), "slot 3's lines are withheld once the vision has no viewing country");
}

#[test]
fn a_refused_vision_carries_nothing_and_the_next_vision_reads_every_row_again() {
    use viewer_layer_core::layers::Published;
    let routes = Arc::new(HeldLines::default());
    let _release = ReleaseRoutes(Arc::clone(&routes));
    routes.set(true);
    let mut side = window_side("convoys-refused-settled", Arc::clone(&routes) as Arc<dyn Routes>, 60..146);
    through(&mut side, 30);
    side.settle_all().unwrap();
    routes.set(false);
    let at = |side: &MapSide| side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::ConvoyClients).and_then(viewer_layer_core::playhead_host::mirror::Mirror::head));
    frames_within(&mut side, 56, |side, _| at(side).is_some_and(|hour| hour >= 51) && layer(side).in_flight > 0);
    frames_within(&mut side, 140, |side, _| at(side).is_some_and(|hour| hour >= 134));
    side.frame(140).unwrap();
    assert_eq!(vision_layer(&side).ours(), None);
    assert!(layer(&side).refused().is_some_and(|why| why.contains("vision")), "{:?}", layer(&side).refused());
    assert_eq!((layer(&side).carried().entities, layer(&side).published(), layer(&side).in_flight), (0, Published::Current, 0), "a refused layer carries and publishes no work");
    assert!(side.on_screen().is_some_and(|on| on.settled), "the held map settles while the vision is refused");
    assert!(layer(&side).shown().is_none());
    frames_within(&mut side, 152, |side, _| !side.composing() && side.applied().is_some_and(|hour| hour >= 148));
    assert_eq!(vision_layer(&side).ours(), Some(3));
    assert_eq!(layer(&side).refused(), None);
    assert!(layer(&side).in_flight > 0, "the replacement build is still held while the vision recovers");
    assert_eq!(layer(&side).product_for, None, "pre-refusal lines are cleared when vision is refused");
    assert!(layer(&side).shown().is_none(), "stale pre-refusal lines do not reappear while the replacement job is pending");
    routes.set(true);
    side.settle_all().unwrap();
    assert_eq!(layer(&side).in_flight, 0);
    assert_eq!(layer(&side).product_for, Some(3));
    assert!(layer(&side).shown().is_some());
    assert_eq!(drawn(&side), (2, 7), "the client that replaced the one before, read again");
    composed_equal(&side);
}
