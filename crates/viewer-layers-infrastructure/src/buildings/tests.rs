use std::{collections::BTreeSet, sync::Arc};
use super::{Buildings, BuildingsFed, Catalogue};
use viewer_layer_core::{layers::{Inputs, Layer, Upstream, View}, test_support::rail_recording::{frames_until, recorded, side_of}, vision::{self as sight, Vision as Sight}, playhead_host::{MapSide, open, units::Units}};
use content_reader_key_names::buildings::{BuildingType, Buildings as Types};
use content_reader_map::positions::BuildingPosition;
use playhead::{Settings, Timeline};
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use viewer_foundation::perspective::{Choice, VISION_TABLES, Vision};
use viewer_buildings::map_buildings::{CityTerrain, Defines, MapBuildingAssets};

const GAP: std::ops::Range<i32> = 40..48;
const FOGGED: i32 = 52;

struct Stub {
    assets: MapBuildingAssets,
    city: Vec<bool>,
}

impl Catalogue for Stub {
    fn assets(&self) -> &MapBuildingAssets {
        &self.assets
    }

    fn city(&self) -> CityTerrain<'_> {
        CityTerrain { width: 1, height: 1, spawn_city: &self.city }
    }

    fn sprawl_cutoff(&self) -> f32 {
        viewer_gpu::camera::MapDefines::test().sprawl_cutoff
    }
}

fn kind(name: &str, shown: i32, edit: impl FnOnce(&mut BuildingType)) -> BuildingType {
    let mut kind = BuildingType { name: name.to_owned(), show_on_map: Some(shown), show_on_map_meshes: 1, ..Default::default() };
    edit(&mut kind);
    kind
}

fn at(state: u32, kind: &str, x: f32, adjacent_sea: u32) -> BuildingPosition {
    BuildingPosition { state, kind: kind.to_owned(), x, y: 0.0, z: x, rotation: 0.5, adjacent_sea }
}

fn catalogue() -> Arc<dyn Catalogue> {
    let kinds = [
        kind("infrastructure", 1, |_| {}),
        kind("industrial_complex", 2, |_| {}),
        kind("bunker", 1, |kind| {
            kind.province_max = Some(10);
            kind.need_detection = true;
        }),
        kind("supply_node", 1, |kind| {
            kind.province_max = Some(1);
            kind.supply_node = true;
        }),
    ];
    let types = Types {
        declaration_order: kinds.iter().map(|kind| kind.name.clone()).collect(),
        types: kinds.into_iter().map(|kind| (kind.name.clone(), kind)).collect(),
        ..Default::default()
    };
    let defines = Defines { shrink_distance: 180.0, destroyed: 6_000_000, recovered: 8_000_000, damaged: 80_000, max_levels: 10, popup_speed: 5.0 };
    let slots = vec![
        (100, "industrial_complex".to_owned(), at(100, "industrial_complex", 1.0, 0), Some(1)),
        (100, "industrial_complex".to_owned(), at(100, "industrial_complex", 2.0, 0), Some(2)),
        (100, "infrastructure".to_owned(), at(100, "infrastructure", 3.0, 0), Some(1)),
        (3, "bunker".to_owned(), at(200, "bunker", 4.0, 0), Some(3)),
        (4, "supply_node".to_owned(), at(200, "supply_node", 5.0, 0), Some(4)),
    ];
    let entities = ["infrastructure", "industrial_complex", "bunker", "supply_node"].into_iter().enumerate().map(|(mesh, name)| ((name.to_owned(), 0), mesh)).collect();
    let assets = MapBuildingAssets::stub(&types, defines, slots, entities, vec![(at(100, "floating_harbor", 6.0, 9), 1)], Some(4));
    Arc::new(Stub { assets, city: vec![false] })
}

fn vision(fogged: bool) -> Arc<Vision> {
    let mut vision = Vision::sharing(1, []);
    vision.set_visibility((1..=9).map(|province| (province, if fogged && province == 1 { 0 } else { 255 })));
    Arc::new(vision)
}

fn key(location: u32, token: u32) -> u128 {
    u128::from(location) << 32 | u128::from(token)
}

fn row(store: &mut Store, table: TableId, slot: u32, at: u128, flags: u32, country: u32, level: i16, healthy: i16) {
    let mut changes = store.table(table.index()).unwrap();
    changes.insert_at(slot, at).unwrap();
    changes.write(slot, 1, Cell::U32(flags)).unwrap();
    changes.write(slot, 2, Cell::U32(country)).unwrap();
    changes.write(slot, 3, Cell::I16(level)).unwrap();
    changes.write(slot, 4, Cell::I16(healthy)).unwrap();
    changes.write(slot, 5, Cell::I64(0)).unwrap();
}

fn side(name: &str) -> MapSide {
    let strings = |at: u32| match at {
        0 => ["infrastructure", "industrial_complex", "bunker", "supply_node", "nonsense", "\u{2}"].map(str::to_owned).to_vec(),
        _ => Vec::new(),
    };
    let tables = [
        TableId::StateBuildings,
        TableId::ProvinceBuildings,
        TableId::LexicalTokens,
        TableId::NavalBases,
        TableId::ProvinceSupply,
        TableId::FloatingHarborDeployments,
        TableId::RaidTargetStatuses,
        TableId::Provinces,
        TableId::States,
        TableId::CountryLinks,
    ];
    let pages = recorded(80, strings, |at, store| {
        let gap = GAP.contains(&(at as i32));
        for table in tables {
            let status = if gap && table == TableId::StateBuildings { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        let level = |store: &mut Store, slot: u32, level: i16| {
            let mut changes = store.table(TableId::StateBuildings.index()).unwrap();
            changes.write(slot, 3, Cell::I16(level)).unwrap();
            changes.write(slot, 4, Cell::I16(level)).unwrap();
        };
        match at {
            0 => {
                let mut lexical = store.table(TableId::LexicalTokens.index()).unwrap();
                for token in 1..=5u32 {
                    lexical.insert_at(token - 1, u128::from(token)).unwrap();
                    lexical.write(token - 1, 0, Cell::Bytes(token - 1)).unwrap();
                }
                row(store, TableId::StateBuildings, 0, key(100, 1), 0, 0, 5, 5);
                row(store, TableId::StateBuildings, 1, key(100, 2), 0, 0, 4, 4);
                row(store, TableId::ProvinceBuildings, 0, key(3, 3), 1, 0, 1, 1);
                row(store, TableId::ProvinceBuildings, 1, key(4, 4), 0, 0, 1, 1);
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for (slot, (province, state, controller)) in [(1u32, 100u32, 10u32), (2, 100, 10), (3, 200, 20), (4, 200, 20)].into_iter().enumerate() {
                    provinces.insert_at(slot as u32, u128::from(province)).unwrap();
                    provinces.write(slot as u32, 0, Cell::U32(state)).unwrap();
                    provinces.write(slot as u32, 2, Cell::U32(controller)).unwrap();
                }
                let mut states = store.table(TableId::States.index()).unwrap();
                for (slot, (state, controller)) in [(100u32, 10u32), (200, 20)].into_iter().enumerate() {
                    states.insert_at(slot as u32, u128::from(state)).unwrap();
                    states.write(slot as u32, 1, Cell::U32(controller)).unwrap();
                }
                let mut links = store.table(TableId::CountryLinks.index()).unwrap();
                for (slot, (raw, canonical)) in [(10u32, 1u32), (20, 2)].into_iter().enumerate() {
                    links.insert_at(slot as u32, u128::from(raw)).unwrap();
                    links.write(slot as u32, 0, Cell::U32(canonical)).unwrap();
                }
                let mut supply = store.table(TableId::ProvinceSupply.index()).unwrap();
                supply.insert_at(0, 9).unwrap();
                supply.write(0, 0, Cell::U16(1)).unwrap();
                supply.write(0, 4, Cell::U8(1)).unwrap();
                let mut deployments = store.table(TableId::FloatingHarborDeployments.index()).unwrap();
                deployments.insert_at(0, key(1, 0)).unwrap();
                deployments.write(0, 0, Cell::I32(9)).unwrap();
                deployments.write(0, 1, Cell::U8(1)).unwrap();
            }
            9 => level(store, 1, 3),
            10 => level(store, 1, 7),
            12 => row(store, TableId::StateBuildings, 2, key(200, 5), 0, 0, 3, 3),
            16 => {
                let mut targets = store.table(TableId::RaidTargetStatuses.index()).unwrap();
                targets.insert_at(0, 0).unwrap();
                for (column, value) in [(5, 3), (6, 3), (7, 1)] {
                    targets.write(0, column, Cell::U32(value)).unwrap();
                }
                targets.write(0, 8, Cell::I32(0)).unwrap();
                targets.write(0, 10, Cell::Bytes(5)).unwrap();
            }
            20 => {
                store.table(TableId::StateBuildings.index()).unwrap().delete(1).unwrap();
                row(store, TableId::StateBuildings, 1, key(100, 6), 0, 0, 1, 1);
            }
            24 => store.table(TableId::StateBuildings.index()).unwrap().delete(1).unwrap(),
            26 => store.table(TableId::RaidTargetStatuses.index()).unwrap().delete(0).unwrap(),
            28 => store.table(TableId::Provinces.index()).unwrap().write(2, 2, Cell::U32(10)).unwrap(),
            30 => store.table(TableId::NavalBases.index()).unwrap().insert_at(0, 4).unwrap(),
            32 => store.table(TableId::ProvinceSupply.index()).unwrap().write(0, 4, Cell::U8(0)).unwrap(),
            36 => store.table(TableId::LexicalTokens.index()).unwrap().write(4, 0, Cell::Bytes(0)).unwrap(),
            _ => {}
        }
    });
    let layer = Buildings::new(catalogue());
    layer.set_vision(Choice::default(), vision(false));
    side_of(pages, Box::new(layer), name)
}

fn buildings(side: &MapSide) -> &Buildings {
    side.parts().unwrap().1[0].any().downcast_ref::<Buildings>().unwrap()
}

#[test]
fn buildings_settled_every_step_equal_composed_and_follow_rows_tokens_gates_vision_and_gaps() {
    let mut side = side("buildings-steps");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut seen = Vec::new();
    let mut fogged = false;
    frames_until(&mut side, |side, frame| {
        if frame.steps == 0 || side.composing() {
            return false;
        }
        let hour = frame.applied.unwrap();
        if hour >= FOGGED && !fogged {
            fogged = true;
            buildings(side).set_vision(Choice::default(), vision(true));
            return false;
        }
        let (mirrors, _) = side.parts().unwrap();
        let layer = buildings(side);
        assert_eq!(layer.carried().entities, 0, "a 3 ms settling settles a step of this world");
        assert_eq!(layer.refused(), None);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.differ(&fresh), (0, None), "settled at hour {hour} equals composed");
        let drawn: Vec<bool> = layer.slots().iter().map(|slot| slot.has_entity()).collect();
        let refused_token = layer.warnings().iter().any(|warning| warning.contains("token 5 (nonsense) names no authored building type"));
        let health: Vec<(u32, Option<i64>)> = layer.health().collect();
        if GAP.contains(&hour) {
            assert_eq!(&drawn[2..5], [false; 3], "StateBuildings not recorded draws no state building at hour {hour}");
            assert!(layer.city_refused().is_some_and(|why| why.contains("StateBuildings is not recorded")), "{:?}", layer.city_refused());
            assert!(health.is_empty());
        } else if hour >= 12 && hour < 20 {
            assert_eq!(&drawn[2..5], [true, true, true], "the complex written twice in a step shows level 7's two slots at hour {hour}");
            assert!(refused_token, "the unknown token is warned of at hour {hour}");
            assert_eq!(health, vec![(1, Some(100_000)), (2, Some(100_000)), (3, Some(100_000)), (4, Some(100_000))]);
        }
        if (16..26).contains(&hour) {
            assert!(drawn[0], "the raid target detects the bunker at hour {hour}");
        }
        if (26..28).contains(&hour) {
            assert!(!drawn[0], "with the raid target deleted the foreign bunker is undetected at hour {hour}");
        }
        if hour >= 32 {
            assert!(drawn[0], "the viewer controls the bunker's province at hour {hour}");
            assert!(!drawn[1], "a harbour takes the hub's province at hour {hour}");
        }
        if (20..24).contains(&hour) {
            assert_eq!(&drawn[2..4], [false, false], "the complex is deleted at hour {hour}");
            assert_eq!(layer.city_refused(), Some("token 6 is not in lexical tokens"), "an unnamed token refuses the city damage at hour {hour}");
        }
        if hour < 32 {
            assert!(drawn[5], "the floating harbour is deployed at hour {hour}");
        } else {
            assert!(!drawn[5], "the floating harbour is withdrawn at hour {hour}");
        }
        if hour >= 40 && !GAP.contains(&hour) {
            assert!(!refused_token, "token 5 is renamed to infrastructure at hour {hour}");
            assert_eq!(layer.city_refused(), None);
        }
        if fogged {
            assert_eq!((drawn[2], drawn[4]), (false, false), "province 1 fogged hides its buildings at hour {hour}");
        } else if hour >= 48 {
            assert!(drawn[4], "infrastructure over province 1 at hour {hour}");
        }
        seen.push(hour);
        hour >= 76
    });
    assert!(seen.iter().any(|hour| GAP.contains(hour)) && seen.iter().any(|&hour| hour >= FOGGED), "{seen:?}");
    assert_eq!(buildings(&side).applied_reads(), 0, "its applies only mark: every row written, gone or come is read at the settle");
}

#[test]
fn buildings_without_vision_refuse_by_name_then_draw() {
    let layer = Buildings::new(catalogue());
    let pages = recorded(8, |_| Vec::new(), |at, store| {
        for table in [TableId::CountryLinks, TableId::Provinces] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
    });
    let mut side = side_of(pages, Box::new(layer), "buildings-no-vision");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    assert!(buildings(&side).refused().is_some_and(|why| why.contains("no vision")));
    assert_eq!(buildings(&side).carried().entities, 0, "refused, a settle can do none of its marked work, so it carries none");
    buildings(&side).set_vision(Choice::default(), vision(false));
    frames_until(&mut side, |side, _| !side.composing() && buildings(side).refused().is_none());
    assert_eq!(buildings(&side).slots().len(), 6);
}

fn column(table: TableId, name: &str) -> usize {
    (0..64).find(|&at| table.field_name(at) == Some(name)).unwrap_or_else(|| panic!("{table:?}.{name}"))
}

const TAKEN: i32 = 13;

fn vision_side(layer: impl Layer + 'static, name: &str) -> MapSide {
    let strings = |at: u32| match at {
        0 => ["infrastructure", "industrial_complex", "bunker", "supply_node", "AAA", "BBB"].map(str::to_owned).to_vec(),
        _ => Vec::new(),
    };
    let mut tables: Vec<TableId> = VISION_TABLES.to_vec();
    for table in [TableId::StateBuildings, TableId::ProvinceBuildings, TableId::LexicalTokens, TableId::NavalBases, TableId::ProvinceSupply, TableId::FloatingHarborDeployments, TableId::RaidTargetStatuses, TableId::States, TableId::CountryLinks] {
        if !tables.contains(&table) {
            tables.push(table);
        }
    }
    let pages = recorded(44, strings, |at, store| {
        for &table in &tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        let write = |store: &mut Store, table: TableId, slot: u32, field: &str, cell: Cell<'_>| store.table(table.index()).unwrap().write(slot, column(table, field), cell).unwrap();
        let insert = |store: &mut Store, table: TableId, slot: u32, key: u128| store.table(table.index()).unwrap().insert_at(slot, key).unwrap();
        if at == 0 {
            for token in 1..=4u32 {
                insert(store, TableId::LexicalTokens, token - 1, u128::from(token));
                write(store, TableId::LexicalTokens, token - 1, "literal", Cell::Bytes(token - 1));
            }
            row(store, TableId::StateBuildings, 0, key(100, 1), 0, 0, 5, 5);
            row(store, TableId::StateBuildings, 1, key(100, 2), 0, 0, 4, 4);
            row(store, TableId::ProvinceBuildings, 0, key(3, 3), 1, 0, 1, 1);
            row(store, TableId::ProvinceBuildings, 1, key(4, 4), 0, 0, 1, 1);
            for (slot, raw) in [101u32, 102].into_iter().enumerate() {
                let slot = slot as u32;
                insert(store, TableId::CountryLinks, slot, u128::from(raw));
                write(store, TableId::CountryLinks, slot, "canonical_slot", Cell::U32(slot + 1));
                insert(store, TableId::Countries, slot, u128::from(raw));
                write(store, TableId::Countries, slot, "canonical_slot", Cell::U32(slot + 1));
                write(store, TableId::Countries, slot, "tag", Cell::Bytes(4 + slot));
            }
            insert(store, TableId::LocalPlayer, 0, 0);
            write(store, TableId::LocalPlayer, 0, "country", Cell::U32(101));
            for (slot, (province, state, controller)) in [(1u32, 100u32, 101u32), (2, 100, 101), (3, 200, 102), (4, 200, 102), (9, 0, 0)].into_iter().enumerate() {
                insert(store, TableId::Provinces, slot as u32, u128::from(province));
                write(store, TableId::Provinces, slot as u32, "state", Cell::U32(state));
                write(store, TableId::Provinces, slot as u32, "controller", Cell::U32(controller));
            }
            insert(store, TableId::Divisions, 0, 900);
            for (field, value) in [("roster_owner", 101), ("logical_owner", 101), ("province", 1)] {
                write(store, TableId::Divisions, 0, field, Cell::U32(value));
            }
            write(store, TableId::Divisions, 0, "reveal_skip", Cell::U8(0));
            insert(store, TableId::WarRelations, 0, 101 << 32 | 102);
        }
        if at as i32 == TAKEN {
            write(store, TableId::Provinces, 3, "controller", Cell::U32(101));
        }
    });
    let content = {
        use viewer_foundation::visibility::{FogContent, Topology, test_defines};
        let neighbors = [(1, vec![2]), (2, vec![1, 3]), (3, vec![2, 4]), (4, vec![3, 9]), (9, vec![4])].into_iter().collect();
        Arc::new(sight::Content::new(Ok(FogContent { topology: Topology { land: (1..=4).collect(), sea: BTreeSet::from([9]), neighbors }, defines: test_defines() })))
    };
    let timeline = Timeline::whole(&*pages).unwrap();
    let units = Units { name: name.to_owned(), pages, timeline, kinds: Arc::new(replay_recording::schema::schema().unwrap()) };
    let settings = Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0, ..Settings::default() };
    let entry = open(units, settings).unwrap();
    let layers: Vec<Box<dyn Layer>> = vec![Box::new(Sight::new(content)), Box::new(layer)];
    let side = MapSide::new(Arc::clone(&entry), layers, View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    side
}

#[test]
fn buildings_with_the_vision_layer_upstream_settle_after_a_fog_decision_moves_and_equal_composed() {
    const SETTLES: usize = 8;
    for reading in [false, true] {
        let layer = if reading { Buildings::reading_vision(catalogue()) } else { Buildings::new(catalogue()) };
        let mut side = vision_side(layer, &format!("buildings-vision-upstream-{reading}"));
        let carried = |side: &MapSide| side.parts().unwrap().1.iter().map(|layer| (layer.name(), layer.carried().entities)).filter(|&(_, entities)| entities > 0).collect::<Vec<_>>();
        let (mut applied, mut seen) = (None::<i32>, Vec::new());
        for _ in 0..50_000_000u64 {
            let newest = side.entry().playhead().newest().unwrap_or(0);
            let head = applied.map_or(newest, |hour| newest.min(hour + 4));
            let frame = side.frame(head).unwrap();
            applied = frame.applied.or(applied);
            if frame.steps == 0 || side.composing() {
                if frame.steps == 0 && frame.swapped.is_none() {
                    std::thread::yield_now();
                }
                continue;
            }
            let hour = frame.applied.unwrap();
            side.settle_all().unwrap();
            if !reading {
                let (_, layers) = side.parts().unwrap();
                let vision = layers[0].any().downcast_ref::<Sight>().unwrap();
                buildings_at(&side, 1).set_vision(vision.choice().clone(), Arc::new(vision.vision().expect("the vision layer draws")));
                side.settle_all().unwrap();
            }
            let mut settles = 1;
            while !carried(&side).is_empty() && settles < SETTLES {
                side.settle_all().unwrap();
                settles += 1;
            }
            assert_eq!(carried(&side), Vec::new(), "reading the layer {reading}: {SETTLES} whole settles leave nothing carried at hour {hour}");
            let (mirrors, layers) = side.parts().unwrap();
            let sight = layers[0].any().downcast_ref::<Sight>().unwrap();
            assert!(sight.fog_drawn(), "a fog is drawn at hour {hour}");
            assert_eq!(sight.province_byte(4), Some(if hour >= TAKEN { 255 } else { 0 }), "province 4's fog at hour {hour}");
            let layer = buildings_at(&side, 1);
            assert_eq!(layer.refused(), None, "reading the layer {reading} at hour {hour}");
            let mut fresh = layer.fresh();
            if reading {
                fresh.compose_with(mirrors, &Upstream::new(&layers[..1])).unwrap();
            } else {
                fresh.compose(mirrors).unwrap();
            }
            assert_eq!(layer.differ(&fresh), (0, None), "reading the layer {reading}: settled at hour {hour} equals composed");
            assert_eq!(layer.slots()[1].has_entity(), hour >= TAKEN, "reading the layer {reading}: the hub over province 4 is seen once the viewer takes it, at hour {hour}");
            seen.push(hour);
            if hour >= 40 {
                break;
            }
        }
        assert!(seen.iter().any(|&hour| hour < TAKEN) && seen.iter().any(|&hour| (TAKEN..40).contains(&hour)) && seen.last().is_some_and(|&hour| hour >= 40), "reading the layer {reading}: {seen:?}");
    }
}

fn buildings_at(side: &MapSide, at: usize) -> &Buildings {
    side.parts().unwrap().1[at].any().downcast_ref::<Buildings>().unwrap()
}

fn fed(side: &MapSide) -> &BuildingsFed {
    side.parts().unwrap().1[1].any().downcast_ref::<BuildingsFed>().unwrap()
}

#[test]
fn on_their_worker_the_buildings_reading_the_vision_layer_equal_composed_at_each_hour() {
    let mut side = vision_side(BuildingsFed::new(Buildings::reading_vision(catalogue())), "buildings-fed");
    let (mut applied, mut seen) = (None::<i32>, Vec::new());
    for _ in 0..50_000_000u64 {
        let newest = side.entry().playhead().newest().unwrap_or(0);
        let head = applied.map_or(newest, |hour| newest.min(hour + 4));
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        if frame.steps == 0 || side.composing() {
            if frame.steps == 0 && frame.swapped.is_none() {
                std::thread::yield_now();
            }
            continue;
        }
        let hour = frame.applied.unwrap();
        side.settle_all().unwrap();
        assert_eq!(Layer::published(fed(&side)), viewer_layer_core::layers::Published::Current, "hour {hour}");
        let (mirrors, layers) = side.parts().unwrap();
        let mut fresh = fed(&side).fresh();
        fresh.compose_with(mirrors, &Upstream::new(&layers[..1])).unwrap();
        let worker = fed(&side).inspect().expect("the buildings' worker answers");
        assert_eq!(worker.refused, None, "hour {hour}");
        assert_eq!(worker.differ(&fresh.inspect()), (0, None), "hour {hour}: the worker's buildings against those composed from the frame's mirrors and vision");
        assert_eq!(worker.slots[1].has_entity(), hour >= TAKEN, "the hub over province 4 is seen once the viewer takes it, at hour {hour}");
        seen.push(hour);
        if hour >= 40 {
            break;
        }
    }
    assert!(seen.iter().any(|&hour| hour < TAKEN) && seen.iter().any(|&hour| (TAKEN..40).contains(&hour)), "{seen:?}");
}

fn hidden() -> View {
    let mut view = View::default();
    (view.graphics.buildings, view.graphics.city_sprawl) = (false, false);
    view
}

#[test]
fn buildings_hidden_by_the_view_declare_nothing_and_either_switch_declares_every_table() {
    let kinds = replay_recording::schema::schema().unwrap();
    let mut layer = Buildings::new(catalogue());
    let mut inputs = Inputs::new(&kinds, "buildings");
    layer.inputs(&hidden(), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "buildings and city sprawl hidden declare nothing");
    for (buildings, sprawl) in [(false, true), (true, false), (true, true)] {
        let mut view = hidden();
        (view.graphics.buildings, view.graphics.city_sprawl) = (buildings, sprawl);
        let mut inputs = Inputs::new(&kinds, "buildings");
        layer.inputs(&view, &mut inputs).unwrap();
        assert_eq!(inputs.into_tables().len(), 10, "buildings {buildings}, city sprawl {sprawl}: every table read is declared");
    }
    let placed = |height: f32, target: Option<f32>| {
        let mut view = hidden();
        (view.graphics.buildings, view.graphics.city_sprawl) = (true, true);
        view.camera_height = height;
        view.camera = Some(viewer_gpu::camera::Placement::map([0.0, 0.0], height));
        view.target = target.map(|height| viewer_gpu::camera::Placement::map([0.0, 0.0], height));
        view
    };
    for (height, target, tables) in [(110.0, None, 10), (2844.0, None, 0), (2844.0, Some(110.0), 10), (200.0, None, 10), (300.0, None, 0)] {
        let mut inputs = Inputs::new(&kinds, "buildings");
        layer.inputs(&placed(height, target), &mut inputs).unwrap();
        assert_eq!(inputs.into_tables().len(), tables, "at height {height} towards {target:?}");
        assert_eq!(layer.placed(&placed(height, target)), Some(u64::from(tables > 0)), "the camera decides the declaration at {height}");
    }
}

#[test]
fn buildings_the_city_index_after_its_moves_draws_as_the_index_made_whole() {
    use map_display_data::value_index::ValueIndex;
    let thresholds = (80_000, 30_000);
    assert_eq!([None, Some(None), Some(Some(80_000)), Some(Some(79_999)), Some(Some(30_000)), Some(Some(29_999))].map(|health| super::city_class(health, thresholds)), [0, 1, 1, 2, 2, 3]);
    let mut layer = Buildings::new(catalogue());
    for (province, state) in [(5, 2), (6, 2), (7, 0), (8, 3)] {
        layer.set_state(province, Some(state));
    }
    layer.city[0].insert(2, 90_000);
    layer.city[0].insert(3, 50_000);
    let mut held = layer.health_index(Some(thresholds)).unwrap();
    layer.city_whole = false;
    assert!(layer.index_due.is_empty(), "made whole, nothing is due");
    let drawn = |index: &ValueIndex<u32, Option<i64>>| (0..=9u32).map(|province| super::city_class(index.get(&province).copied(), thresholds)).collect::<Vec<_>>();
    let moves: [(u32, i64, &[u32]); 6] = [(2, 85_000, &[]), (2, 70_000, &[5, 6]), (2, 75_000, &[]), (3, 40_000, &[]), (3, 20_000, &[8]), (2, 95_000, &[5, 6])];
    for (state, health, due) in moves {
        layer.city[0].insert(state, health);
        layer.city_moved(false, state);
        assert_eq!(layer.index_due.iter().copied().collect::<Vec<_>>(), due, "state {state} at {health}");
        let patched = layer.uploads[3];
        let index = layer.health_index(Some(thresholds)).unwrap();
        assert_eq!(index.ptr_eq(&held), due.is_empty(), "the index is kept unless a drawn state moved");
        assert_eq!(layer.uploads[3] - patched, due.len() as u64, "the due provinces alone are patched");
        held = index;
        let whole = ValueIndex::try_from_iter(layer.health()).unwrap();
        assert_eq!(drawn(&held), drawn(&whole), "draws as the index made whole after state {state} at {health}");
        assert!(held.keys().eq(whole.keys()));
    }
    layer.set_state(9, Some(3));
    assert_eq!(layer.index_due.iter().copied().collect::<Vec<_>>(), [9], "a province gaining its entry is due");
}

#[test]
fn buildings_a_write_to_what_the_slots_do_not_read_marks_nothing() {
    let strings = |at: u32| if at == 0 { ["infrastructure", "industrial_complex", "bunker", "supply_node"].map(str::to_owned).to_vec() } else { Vec::new() };
    let tables = [
        TableId::StateBuildings,
        TableId::ProvinceBuildings,
        TableId::LexicalTokens,
        TableId::NavalBases,
        TableId::ProvinceSupply,
        TableId::FloatingHarborDeployments,
        TableId::RaidTargetStatuses,
        TableId::Provinces,
        TableId::States,
        TableId::CountryLinks,
    ];
    let pages = recorded(30, strings, |at, store| {
        for table in tables {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        match at {
            0 => {
                let mut lexical = store.table(TableId::LexicalTokens.index()).unwrap();
                for token in 1..=4u32 {
                    lexical.insert_at(token - 1, u128::from(token)).unwrap();
                    lexical.write(token - 1, 0, Cell::Bytes(token - 1)).unwrap();
                }
                row(store, TableId::StateBuildings, 0, key(100, 1), 0, 0, 5, 5);
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for (slot, province) in [1u32, 2].into_iter().enumerate() {
                    provinces.insert_at(slot as u32, u128::from(province)).unwrap();
                    provinces.write(slot as u32, 0, Cell::U32(100)).unwrap();
                    provinces.write(slot as u32, 2, Cell::U32(10)).unwrap();
                }
                let mut states = store.table(TableId::States.index()).unwrap();
                states.insert_at(0, 100).unwrap();
                states.write(0, 1, Cell::U32(10)).unwrap();
                let mut links = store.table(TableId::CountryLinks.index()).unwrap();
                links.insert_at(0, 10).unwrap();
                links.write(0, 0, Cell::U32(1)).unwrap();
                let mut supply = store.table(TableId::ProvinceSupply.index()).unwrap();
                supply.insert_at(0, 9).unwrap();
                supply.write(0, 0, Cell::U16(1)).unwrap();
                supply.write(0, 4, Cell::U8(1)).unwrap();
            }
            4..=20 if at % 2 == 0 => store.table(TableId::StateBuildings.index()).unwrap().write(0, 5, Cell::I64(i64::from(at) * 10)).unwrap(),
            5..=21 => store.table(TableId::ProvinceSupply.index()).unwrap().write(0, 4, Cell::U8(if at % 4 == 1 { 3 } else { 1 })).unwrap(),
            24 => store.table(TableId::ProvinceSupply.index()).unwrap().write(0, 4, Cell::U8(0)).unwrap(),
            _ => {}
        }
    });
    let layer = Buildings::new(catalogue());
    layer.set_vision(Choice::default(), vision(false));
    let mut side = viewer_layer_core::test_support::rail_recording::side_at_pace(pages, Box::new(layer), "buildings-unread-writes", 1.0);
    crate::rails::tests::settle_at(&mut side, 3);
    let before = buildings(&side).causes;
    let rows = buildings(&side).marked[0];
    crate::rails::tests::settle_at(&mut side, 22);
    let after = buildings(&side).causes;
    assert!(after[super::HELD] > before[super::HELD], "the partial health writes were read and held: {after:?}");
    assert_eq!((buildings(&side).marked[0], after[super::ROW], after[super::HARBOUR]), (rows, before[super::ROW], before[super::HARBOUR]), "no row, slot or harbour marked by writes nothing reads");
    crate::rails::tests::settle_at(&mut side, 26);
    assert_eq!(buildings(&side).causes[super::HARBOUR], after[super::HARBOUR] + 1, "the deployment bit marks the harbour on sea 9");
}
