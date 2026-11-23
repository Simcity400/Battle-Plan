use super::{PlanRows, Trains, Yard};
use viewer_layer_core::{layers::{Layer, Published, Upstream, View}, test_support::rail_recording::{frames_until, recorded}, playhead_host::{MapSide, open, units::Units}};
use crate::rails::{Rails, test_support::Stub as Ground};
use playhead::{Settings, Timeline};
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::{Field, NativeValue, Opaque, SparseBytes, SupplyNodeRow, TableId, TrainAllocation};
use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use viewer_rail::{
    railways::{RailNetwork, TrainRoute},
    trains::{Curve, PlanCache, TrainAssets, TrainPlan, TrainRows},
};

fn settled_at(side: &mut MapSide, hour: i32) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let frame = side.frame(hour).unwrap();
        let state: Vec<(&str, usize, Published)> = side.parts().map(|(_, layers)| layers.iter().map(|layer| (layer.name(), layer.carried().entities, layer.published())).collect()).unwrap_or_default();
        if side.applied() == Some(hour) && !side.composing() && !state.is_empty() && state.iter().all(|&(_, entities, published)| entities == 0 && published == Published::Current) {
            return;
        }
        if Instant::now() >= deadline {
            let owed = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Trains>())).map(Trains::owed);
            panic!("hour {hour} was not applied and made within 120 s: applied {:?}, composing {}, carried and published {state:?}; {owed:?}", side.applied(), side.composing());
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Seen {
    head: i32,
    country: Option<u32>,
    complete: bool,
    nodes: Vec<(u32, u32, i64, i32)>,
    fallback: Result<bool, String>,
    allowed: [bool; 3],
    rail: [bool; 2],
}

#[derive(Default)]
struct Probe(Mutex<Vec<Seen>>);

impl Yard for Probe {
    fn cutoff(&self) -> f32 {
        300.0
    }

    fn plan(&self, rows: &PlanRows<'_>, ours: u32, _curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>, _cache: &mut PlanCache) -> io::Result<TrainPlan> {
        let country = rows.first_country(ours)?;
        let access = SparseBytes::from_dense(&[0, 1])?;
        let seen = Seen {
            head: rows.head(TableId::SupplyNodes).unwrap_or(-1),
            country,
            complete: rows.complete()?,
            nodes: rows.nodes(10)?.iter().map(|node| (node.index, node.province, node.available, node.parent)).collect(),
            fallback: rows.fallback().and_then(|fallback| fallback.emits(1, 2)).map_err(|error| error.to_string()),
            allowed: [1, 2, 4].map(|province| rows.allowed(province, &access)),
            rail: [rows.rail(2, 1), rows.rail(3, 2)],
        };
        self.0.lock().unwrap().push(seen);
        Ok(TrainPlan::default())
    }

    fn curve(&self, _path: &[u32], _network: &RailNetwork) -> Curve {
        None
    }
}

const VERTEX: u32 = SupplyNodeRow::VERTEX;

fn key(high: u32, low: u32) -> u128 {
    u128::from(high) << 32 | u128::from(low)
}

fn side(name: &str, probe: &Arc<Probe>) -> MapSide {
    let tables = [
        TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::Provinces, TableId::ReservedCountryNaming,
        TableId::SupplyNodes, TableId::TrainInventory, TableId::VariantDefinitions, TableId::RailEdges, TableId::RailCooldowns, TableId::ProvinceSupply, TableId::NavalBases,
    ];
    let pages = recorded(60, |at| if at == 0 { vec!["AAA".to_owned(), "BBB".to_owned()] } else { Vec::new() }, |at, store: &mut Store| {
        for table in tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        let country = |store: &mut Store, slot: u32, native: u32, canonical: u32, tag: u32| {
            let mut countries = store.table(TableId::Countries.index()).unwrap();
            countries.insert_at(slot, u128::from(native)).unwrap();
            countries.write(slot, 0, Cell::U32(canonical)).unwrap();
            countries.write(slot, 1, Cell::Bytes(tag)).unwrap();
            let mut links = store.table(TableId::CountryLinks.index()).unwrap();
            links.insert_at(slot, u128::from(native)).unwrap();
            links.write(slot, 0, Cell::U32(canonical)).unwrap();
        };
        let node = |store: &mut Store, slot: u32, key: u128, province: u32, kind: u32, parent: i32| {
            let mut nodes = store.table(TableId::SupplyNodes.index()).unwrap();
            nodes.insert_at(slot, key).unwrap();
            for (column, cell) in [(0, Cell::U32(province)), (1, Cell::U32(kind)), (2, Cell::U8(0)), (3, Cell::I64(500_000)), (4, Cell::I64(0)), (5, Cell::I32(parent)), (6, Cell::U16(0)), (7, Cell::U16(0)), (8, Cell::U8(0))] {
                nodes.write(slot, column, cell).unwrap();
            }
            nodes.write(slot, 9, Cell::U32List(&[])).unwrap();
        };
        let available = |store: &mut Store, value: i64| store.table(TableId::SupplyNodes.index()).unwrap().write(1, 3, Cell::I64(value)).unwrap();
        match at {
            0 => {
                country(store, 0, 10, 1, 0);
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for province in 1..=4u32 {
                    provinces.insert_at(province - 1, u128::from(province)).unwrap();
                    provinces.write(province - 1, 2, Cell::U32(if province == 4 { 0 } else { 10 })).unwrap();
                }
                node(store, 0, key(10, 1), 1, 2, 0);
                node(store, 1, key(10, 2), 2, 3, 1);
                node(store, 2, key(10, 3 | VERTEX), 3, 2, 2);
                node(store, 3, key(20, 1), 4, 4, 0);
                let mut edges = store.table(TableId::RailEdges.index()).unwrap();
                edges.insert_at(0, key(1, 2)).unwrap();
                edges.write(0, 0, Cell::I32(1)).unwrap();
                let mut cooldowns = store.table(TableId::RailCooldowns.index()).unwrap();
                cooldowns.insert_at(0, 2).unwrap();
                cooldowns.write(0, 0, Cell::I32(5)).unwrap();
            }
            9 => available(store, 1),
            10 => {
                available(store, 2);
                let mut edges = store.table(TableId::RailEdges.index()).unwrap();
                edges.insert_at(1, key(2, 3)).unwrap();
                edges.write(1, 0, Cell::I32(1)).unwrap();
            }
            11 => available(store, 3),
            12 => store.table(TableId::RailCooldowns.index()).unwrap().write(0, 0, Cell::I32(0)).unwrap(),
            20 => country(store, 1, 20, 2, 1),
            30 => {
                store.table(TableId::SupplyNodes.index()).unwrap().delete(0).unwrap();
            }
            _ => {}
        }
    });
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[replay_columnar::state::Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    let units = Units { name: name.to_owned(), pages, timeline, kinds };
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let yard: Arc<dyn Yard> = Arc::clone(probe) as _;
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Rails::new(Arc::new(Ground))), Box::new(Trains::new(yard, Some(1)))];
    let side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    side
}

fn trains(side: &MapSide) -> &Trains {
    side.parts().unwrap().1[1].any().downcast_ref::<Trains>().unwrap()
}

#[test]
fn trains_read_nodes_census_access_and_rails_at_the_head_and_build_only_when_they_change() {
    let probe = Arc::new(Probe::default());
    let mut side = side("trains-steps", &probe);
    settled_at(&mut side, 0);
    let mut building = Vec::new();
    for hour in (4..=56).step_by(4) {
        let before = trains(&side).marked()[0];
        settled_at(&mut side, hour);
        let built = trains(&side).marked()[0] - before;
        assert!(built <= 1, "the step to hour {hour} built {built} plans");
        if built == 1 {
            building.push(hour);
        }
    }
    let seen = probe.0.lock().unwrap().clone();
    let at = |hour: i32| seen.iter().find(|seen| seen.head >= hour).cloned().unwrap_or_else(|| panic!("no plan built from hour {hour}: {seen:?}"));
    let first = at(0);
    assert_eq!((first.country, first.complete), (Some(10), true));
    assert_eq!(first.nodes.iter().map(|node| node.0).collect::<Vec<_>>(), [1, 2, 3 | VERTEX], "our nodes in key order, the vertex among them");
    assert!(first.fallback.as_ref().is_err_and(|why| why.contains("country absent")), "country 20's node refuses the census: {:?}", first.fallback);
    assert_eq!(first.allowed, [true, false, false], "a cooling province and an uncontrolled one with no access are refused");
    assert_eq!(first.rail, [true, false]);
    let later = at(12);
    assert_eq!(later.nodes[1].2, 3, "a node written twice in a step shows its newest supply");
    assert_eq!((later.allowed, later.rail), ([true, true, false], [true, true]), "the cooldown ended and an edge was inserted");
    assert_eq!(at(20).fallback, Ok(true), "with country 20 recorded the census holds both kind 2 and 3 nodes");
    assert_eq!(at(30).nodes.iter().map(|node| node.0).collect::<Vec<_>>(), [2, 3 | VERTEX], "a deleted node is gone");
    assert_eq!(building, [12, 20, 32], "plans built only in the steps that change what they read (built, quiet settles, curves, weights only: {:?})", trains(&side).marked());
}

#[test]
fn trains_above_the_cutoff_declare_nothing_and_build_again_when_shown() {
    let probe = Arc::new(Probe::default());
    let mut side = side("trains-view", &probe);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_view(View { camera_height: 1000.0, ..View::default() }).unwrap();
    let kinds = replay_recording::schema::schema().unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "trains");
    let mut hidden = Trains::new(Arc::new(Probe::default()), Some(1));
    hidden.inputs(&View { camera_height: 1000.0, ..View::default() }, &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "hidden, the layer declares nothing");
    frames_until(&mut side, |side, frame| !side.composing() && frame.applied.is_some_and(|hour| hour >= 24));
    let before = probe.0.lock().unwrap().len();
    side.set_view(View::default()).unwrap();
    frames_until(&mut side, |side, frame| (side.draws("trains") && !side.composing()) || frame.applied == Some(59));
    let seen = probe.0.lock().unwrap().clone();
    if side.draws("trains") {
        assert!(seen.len() > before, "shown again, the plan is built anew from the mirrors: {:?}", seen.last());
    } else {
        let trains = side.parts().unwrap().1[1].any().downcast_ref::<Trains>().unwrap();
        assert!(trains.plan().counts() == [0; 3], "not drawn again yet, it holds no plan");
    }
}

#[derive(Default)]
struct Laid(TrainAssets);

impl Yard for Laid {
    fn cutoff(&self) -> f32 {
        300.0
    }

    fn plan(&self, rows: &PlanRows<'_>, ours: u32, curve: &mut dyn FnMut(&[u32]) -> io::Result<Curve>, cache: &mut PlanCache) -> io::Result<TrainPlan> {
        TrainPlan::build_from(rows, 0, 0, ours, &self.0, &self.0.cultures, curve, Some(cache))
    }

    fn curve(&self, path: &[u32], _network: &RailNetwork) -> Curve {
        let points: Vec<[f32; 3]> = path.iter().map(|&province| [province as f32 * 10.0, 0.0, 0.0]).collect();
        let distance = points.iter().scan(None::<[f32; 3]>, |last, point| {
            let step = last.map_or(0.0, |last: [f32; 3]| (point[0] - last[0]).abs());
            *last = Some(*point);
            Some(step)
        });
        let distance: Vec<f32> = distance.scan(0.0, |sum, step| {
            *sum += step;
            Some(*sum)
        }).collect();
        Some(Arc::new((TrainRoute { points, open_end: false }, distance)))
    }
}

fn column(table: TableId, field: &str) -> usize {
    (0..64).find(|&column| table.field_name(column) == Some(field)).unwrap_or_else(|| panic!("{table:?}.{field}"))
}

fn opaque<T: NativeValue>(value: T) -> Vec<u8> {
    let Field::Opaque(Opaque::Native(value)) = value.field() else { panic!("an opaque value") };
    let mut bytes = Vec::new();
    value.encode_value(&mut bytes).unwrap();
    bytes
}

fn routes_side(name: &str, hours: u32, churn: bool) -> MapSide {
    routes_side_with(name, hours, Changes { churn, ..Changes::default() })
}

#[derive(Clone, Copy, Default)]
struct Changes {
    churn: bool,
    rails: bool,
    grows: bool,
    crossing: bool,
    waking: bool,
}

fn routes_side_with(name: &str, hours: u32, changes: Changes) -> MapSide {
    let Changes { churn, rails, grows, crossing, waking } = changes;
    let tables = [
        TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::LexicalTokens, TableId::Provinces, TableId::ReservedCountryNaming,
        TableId::SupplyNodes, TableId::TrainInventory, TableId::VariantDefinitions, TableId::RailEdges, TableId::RailCooldowns, TableId::ProvinceSupply, TableId::NavalBases,
    ];
    let pages = recorded(hours, |at| if at == 0 { vec!["AAA".to_owned()] } else { Vec::new() }, |at, store: &mut Store| {
        for table in tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        let node = |store: &mut Store, slot: u32, index: u32, province: u32, available: i64, parent: i32, path: &[u32]| {
            let mut nodes = store.table(TableId::SupplyNodes.index()).unwrap();
            nodes.insert_at(slot, key(10, index)).unwrap();
            let route = u16::from(parent > 0);
            for (column, cell) in [(0, Cell::U32(province)), (1, Cell::U32(2)), (2, Cell::U8(0)), (3, Cell::I64(available)), (4, Cell::I64(0)), (5, Cell::I32(parent)), (6, Cell::U16(route)), (7, Cell::U16(0)), (8, Cell::U8(0))] {
                nodes.write(slot, column, cell).unwrap();
            }
            nodes.write(slot, 9, Cell::U32List(path)).unwrap();
        };
        if rails && (at == 14 || at == 18) {
            store.table(TableId::RailEdges.index()).unwrap().write(1, 0, Cell::I32(i32::from(at == 18))).unwrap();
        }
        if crossing && at == 10 {
            store.table(TableId::SupplyNodes.index()).unwrap().write(2, 3, Cell::I64(900_000)).unwrap();
        }
        if crossing && at == 12 {
            store.table(TableId::SupplyNodes.index()).unwrap().delete(2).unwrap();
            node(store, 70, 3, 3, 900_000, 1, &[3, 1]);
        }
        if crossing && at == 15 {
            store.table(TableId::SupplyNodes.index()).unwrap().write(70, 3, Cell::I64(200_000)).unwrap();
        }
        if grows && at == 20 {
            node(store, 3, 4, 4, 500_000, 1, &[4, 1]);
            store.table(TableId::TrainInventory.index()).unwrap().write(0, column(TableId::TrainInventory, "node_count"), Cell::U32(4)).unwrap();
        }
        match at {
            0 => {
                let mut countries = store.table(TableId::Countries.index()).unwrap();
                countries.insert_at(0, 10).unwrap();
                countries.write(0, 0, Cell::U32(1)).unwrap();
                countries.write(0, 1, Cell::Bytes(0)).unwrap();
                let mut links = store.table(TableId::CountryLinks.index()).unwrap();
                links.insert_at(0, 10).unwrap();
                links.write(0, 0, Cell::U32(1)).unwrap();
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for province in 1..=4u32 {
                    provinces.insert_at(province - 1, u128::from(province)).unwrap();
                    provinces.write(province - 1, 2, Cell::U32(10)).unwrap();
                }
                node(store, 0, 1, 1, 0, 0, &[]);
                node(store, 1, 2, 2, 500_000, 1, &[2, 1]);
                node(store, 2, 3, 3, 500_000, 1, &[3, 1]);
                let inventory = TableId::TrainInventory;
                let mut trains = store.table(inventory.index()).unwrap();
                trains.insert_at(0, 10).unwrap();
                trains.write(0, column(inventory, "node_count"), Cell::U32(3)).unwrap();
                trains.write(0, column(inventory, "allocations"), Cell::Opaque(&opaque(vec![TrainAllocation { variant: 70, count: 5 }]))).unwrap();
                trains.write(0, column(inventory, "controller_access"), Cell::Opaque(&opaque(SparseBytes::from_dense(&[0, 1]).unwrap()))).unwrap();
                let mut edges = store.table(TableId::RailEdges.index()).unwrap();
                for (slot, (from, to)) in [(1, 2), (1, 3), (1, 4), (4, 2)].into_iter().enumerate() {
                    edges.insert_at(slot as u32, key(from, to)).unwrap();
                    edges.write(slot as u32, 0, Cell::I32(1)).unwrap();
                }
                if waking {
                    node(store, 3, 4, 4, 0, 1, &[4, 1]);
                    store.table(inventory.index()).unwrap().write(0, column(inventory, "node_count"), Cell::U32(4)).unwrap();
                }
            }
            6 => store.table(TableId::SupplyNodes.index()).unwrap().write(1, 9, Cell::U32List(&[2, 4, 1])).unwrap(),
            10 if waking => store.table(TableId::SupplyNodes.index()).unwrap().write(3, 3, Cell::I64(500_000)).unwrap(),
            _ if churn && at >= 8 => {
                let mut nodes = store.table(TableId::SupplyNodes.index()).unwrap();
                nodes.write(2, 7, Cell::U16((at % 2) as u16)).unwrap();
                if at % 4 == 0 {
                    nodes.write(2, 3, Cell::I64(500_000 + i64::from(at))).unwrap();
                }
            }
            _ => {}
        }
    });
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[replay_columnar::state::Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = Timeline::whole(&*pages).unwrap();
    let units = Units { name: name.to_owned(), pages, timeline, kinds };
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() }).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Rails::new(Arc::new(Ground))), Box::new(Trains::new(Arc::new(Laid::default()), Some(1)))];
    let side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    side
}

fn equals_composed(side: &MapSide, what: &str) {
    let (mirrors, layers) = side.parts().unwrap();
    let mut fresh = trains(side).fresh();
    fresh.compose_with(mirrors, &Upstream::new(&layers[..1])).unwrap();
    assert_eq!(trains(side).differ(&fresh), (0, None), "{what}: the plan equals one composed");
}

#[test]
fn trains_a_change_to_one_route_is_folded_on_the_worker_and_remakes_that_route_alone() {
    let mut side = routes_side("trains-routes", 12, false);
    settled_at(&mut side, 4);
    let layer = trains(&side);
    assert_eq!(layer.refused(), None);
    let kept = Arc::clone(layer.plan().route_geometry(&[1, 3]).expect("the route from 1 to 3"));
    assert!(layer.plan().route_geometry(&[1, 2]).is_some(), "the route from 1 to 2");
    let built = layer.marked()[0];

    settled_at(&mut side, 8);
    let layer = trains(&side);
    assert_eq!(layer.applied_reads(), 0, "apply read nothing from the mirrors");
    assert_eq!(layer.carried().entities, 0, "the frame holds no row of it");
    assert_eq!(layer.marked()[0], built + 1, "one plan made");
    assert!(layer.plan().route_geometry(&[1, 2]).is_none() && layer.plan().route_geometry(&[1, 4, 2]).is_some(), "the changed route is remade");
    assert!(Arc::ptr_eq(layer.plan().route_geometry(&[1, 3]).unwrap(), &kept), "the unchanged route keeps its curve");
    equals_composed(&side, "settled");
}

#[test]
fn trains_a_row_changed_in_a_thousand_steps_is_folded_and_the_plan_follows_the_head() {
    const HOURS: u32 = 4040;
    let mut side = routes_side("trains-bounded", HOURS, true);
    settled_at(&mut side, 8);
    let built = trains(&side).marked()[0];
    let (mut steps, mut carried, mut behind) = (0u64, 0usize, 0i32);
    frames_until(&mut side, |side, frame| {
        steps += u64::from(frame.steps);
        let layer = side.parts().unwrap().1[1].any().downcast_ref::<Trains>().unwrap();
        carried = carried.max(layer.carried().entities);
        if let (Some(applied), Published::At(shown)) = (frame.applied, layer.published()) {
            behind = behind.max(applied - shown);
        }
        frame.applied.is_some_and(|hour| hour >= HOURS as i32 - 8)
    });
    assert!(steps >= 1000, "a thousand steps taken: {steps}");
    let layer = trains(&side);
    assert_eq!(carried, 0, "the frame carried no row in {steps} steps");
    assert_eq!(layer.applied_reads(), 0, "apply read nothing from the mirrors");
    let made = layer.marked()[0] + layer.marked()[1] + layer.marked()[3] - built;
    assert!((made as u64) <= steps + 1, "at most a plan a step: {made} makings for {steps} steps");
    assert!(layer.stats.plan_short <= 0, "a plan stood {} hours before the hour applied when its work began (the shown plan lagged {behind} hours at the worst)", layer.stats.plan_short);

    let hour = side.applied().unwrap();
    settled_at(&mut side, hour);
    equals_composed(&side, &format!("settled at hour {hour} after {steps} steps"));
}

#[test]
fn trains_at_a_head_driven_hour_by_hour_and_settled_whole_show_the_plan_of_each_hour() {
    let mut side = routes_side("trains-driven", 48, true);
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    for hour in 1..=40 {
        side.catch_up(hour).unwrap();
        let layer = trains(&side);
        assert_eq!(layer.published(), Published::Current, "driven to hour {hour}, settled whole: {}", layer.owed());
        assert_eq!(layer.carried().entities, 0, "hour {hour}: no curve left to make");
        equals_composed(&side, &format!("driven to hour {hour}"));
    }
}

#[test]
fn trains_with_the_rail_network_changing_mid_run_settle_whole_each_hour_with_no_frame_budget() {
    let mut side = routes_side_with("trains-network", 30, Changes { churn: true, rails: true, ..Changes::default() });
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    for hour in 1..=24 {
        side.catch_up(hour).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let layer = trains(&side);
        assert_eq!(layer.published(), Published::Current, "hour {hour}: {}", layer.owed());
        assert!(layer.stats.plan_short <= 0, "hour {hour}: a plan missed a step it owes");
        equals_composed(&side, &format!("the network changing, hour {hour}"));
    }
}

#[test]
fn trains_show_an_hour_before_a_change_their_plan_does_not_hold_yet() {
    let mut side = routes_side("trains-published", 40, true);
    settled_at(&mut side, 4);
    assert_eq!(trains(&side).published(), Published::Current);
    for hour in (8..=36).step_by(4) {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let frame = side.frame(hour).unwrap();
            let layer = trains(&side);
            let published = layer.published();
            if let Published::At(shown) = published {
                assert!(shown < hour, "the hour shown, {shown}, is before the head {hour}");
            }
            if side.applied() == Some(hour) && !side.composing() && layer.carried().entities == 0 && published == Published::Current {
                break;
            }
            assert!(Instant::now() < deadline, "hour {hour} was not made within 120 s: applied {:?}, {published:?}; {}", side.applied(), layer.owed());
            if frame.steps == 0 && frame.swapped.is_none() {
                std::thread::yield_now();
            }
        }
        equals_composed(&side, &format!("at hour {hour}"));
    }
}

fn ahead_steps(side: &mut MapSide, mut each: impl FnMut(i32) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let next = side.applied().unwrap() + 4;
        loop {
            let applied = side.applied().unwrap();
            side.frame(applied).unwrap();
            if side.entry().playhead().newest() >= Some(next) && trains(side).made_through() >= Some(next) {
                break;
            }
            assert!(Instant::now() < deadline, "the trains' worker did not make hour {next} within 120 s: {}", trains(side).owed());
            std::thread::yield_now();
        }
        let mut stepped = None;
        for _ in 0..50_000_000u64 {
            let frame = side.frame(next).unwrap();
            if frame.steps > 0 {
                stepped = Some(frame);
                break;
            }
            std::thread::yield_now();
        }
        let frame = stepped.unwrap_or_else(|| panic!("no frame took the step to hour {next}"));
        assert_eq!((frame.applied, frame.steps), (Some(next), 1), "a step of four hours a frame");
        if each(next) {
            return;
        }
    }
}

#[test]
fn trains_after_a_slower_pace_start_over_and_hold_no_row_of_a_void_step() {
    let mut side = routes_side_with("trains-pace", 48, Changes { grows: true, ..Changes::default() });
    settled_at(&mut side, 4);
    ahead_steps(&mut side, |hour| hour >= 16);
    let applied = side.applied().unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while trains(&side).made_through() < Some(applied + 8) {
        assert!(Instant::now() < deadline, "the trains' worker did not make hour {} within 120 s: {}", applied + 8, trains(&side).owed());
        side.frame(applied).unwrap();
        std::thread::yield_now();
    }
    side.entry().playhead().play(1.0);
    let mut voided = 0;
    while side.entry().playhead().held().2.resteps == 0 {
        assert!(Instant::now() < deadline, "the steps ahead were not built again within 120 s");
        voided += side.frame(applied).unwrap().voided;
        std::thread::yield_now();
    }
    for hour in applied + 1..=applied + 8 {
        loop {
            assert!(Instant::now() < deadline, "no step of one hour to hour {hour} within 120 s");
            let frame = side.frame(hour).unwrap();
            voided += frame.voided;
            if frame.applied == Some(hour) {
                break;
            }
            std::thread::yield_now();
        }
        side.settle_all().unwrap();
        assert_eq!(trains(&side).published(), Published::Current, "the trains show hour {hour}: {}", trains(&side).owed());
        equals_composed(&side, &format!("hour {hour} after the slower pace"));
    }
    assert!(voided >= 1, "the steps the trains folded ahead were voided and the worker started over");
}

#[test]
fn trains_a_supply_change_alone_moves_the_weights_at_its_hour() {
    let mut side = routes_side_with("trains-supply", 24, Changes { crossing: true, ..Changes::default() });
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    for hour in 1..=20 {
        side.catch_up(hour).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let layer = trains(&side);
        assert_eq!(layer.published(), Published::Current, "hour {hour}: {}", layer.owed());
        equals_composed(&side, &format!("hour {hour}, the supply alone moved"));
    }
    let weights: Vec<i64> = trains(&side).shown_nodes().iter().map(|node| node.weight).collect();
    assert!(weights.contains(&200_000), "the fall at hour 15 is in the plan: {weights:?}");
}

#[test]
fn trains_a_node_weighted_by_its_supply_alone_brings_its_route_at_that_hour() {
    let mut side = routes_side_with("trains-waking", 16, Changes { waking: true, ..Changes::default() });
    side.anchor(0, false, 0.0, true);
    side.catch_up(0).unwrap();
    assert!(trains(&side).plan().route_geometry(&[1, 4]).is_none(), "a node with no supply has no route");
    for hour in 1..=14 {
        side.catch_up(hour).unwrap_or_else(|error| panic!("hour {hour}: {error}"));
        let layer = trains(&side);
        assert_eq!(layer.published(), Published::Current, "hour {hour}: {}", layer.owed());
        equals_composed(&side, &format!("hour {hour}"));
    }
    assert!(trains(&side).plan().route_geometry(&[1, 4]).is_some(), "the node weighted by its supply alone has its route");
}
