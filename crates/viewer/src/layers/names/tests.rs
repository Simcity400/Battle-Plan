use super::{Atlas, Names, NameLayer, NameLayers};
use crate::layers::{
    Layer, Published, View,
    rails::tests::{frames_until, recorded, side_of},
};
use installed_content_localisation::localisation::Localisation;
use map_display_data::landscape::{Placements, WorldName};
use replay_columnar::{manifest::Status, state::Cell};
use replay_data::rows::TableId;
use std::{
    collections::BTreeSet,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use viewer_world::map::map_mode::{MapMode, Stock};

/// Provinces 1 to 6; connected areas are runs of consecutive ids; a name is its tag, text and area.
struct Stub(Arc<Localisation>);

impl Atlas for Stub {
    fn provinces(&self) -> Vec<u32> {
        (1..=6).collect()
    }

    fn localisation(&self) -> Arc<Localisation> {
        Arc::clone(&self.0)
    }

    fn components(&self, members: &BTreeSet<u32>) -> Vec<Vec<u32>> {
        let mut out: Vec<Vec<u32>> = Vec::new();
        for &id in members {
            match out.last_mut() {
                Some(run) if run.last() == Some(&(id - 1)) => run.push(id),
                _ => out.push(vec![id]),
            }
        }
        out
    }

    /// Today's rule in small: the factions mode writes faction names, the terrain mode none, every
    /// other stock mode the countries' own; a scripted mode is not declared.
    fn name_layers(&self, mode: &MapMode) -> Option<NameLayers> {
        let layers = |far, near| Some(NameLayers { far, near });
        match mode {
            MapMode::Stock(Stock::Factions) => layers(NameLayer::Factions, NameLayer::Factions),
            MapMode::Stock(Stock::Terrain) => layers(NameLayer::Nothing, NameLayer::Nothing),
            MapMode::Stock(_) => layers(NameLayer::Countries, NameLayer::Nothing),
            MapMode::Scripted(_) => None,
        }
    }

    /// One name, keyed by the mode and the hour of the rows it was made from.
    fn mode_names(&self, frame: &replay_frame::frame::DecodedFrame, mode: &MapMode) -> std::io::Result<Arc<Placements>> {
        let name = WorldName { key: format!("{} at {}", mode.token(), frame.hour), ..self.place("", "", &[]).unwrap() };
        Ok(Arc::new(Placements { country: vec![name], generation: viewer_foundation::map::name_draw::next_generation(), ..Placements::default() }))
    }

    fn executable(&self) -> Option<engine_executable::executable_tables::Tables> {
        None
    }

    fn place(&self, tag: &str, text: &str, component: &[u32]) -> Option<WorldName> {
        Some(WorldName {
            key: tag.to_owned(),
            strategic_provinces: None,
            text: text.to_owned(),
            pages: Vec::new(),
            corners: component.iter().map(|&id| [id as f32, 0.0, 0.0]).collect(),
            uvs: Vec::new(),
            aabb: [0.0; 4],
            wrapped: false,
            font_size: 1.0,
            spaces: 0,
            alpha: 1.0,
        })
    }
}

const GAP: std::ops::Range<i32> = 40..48;

fn key(high: u32, low: u32) -> u128 {
    u128::from(high) << 32 | u128::from(low)
}

/// Two countries and six provinces: a province changing hands twice inside one step, a tag renamed,
/// a ruling party naming a group, a province's row deleted, a third country inserted, and Provinces
/// not recorded for a stretch; the Clock, which a mode's whole names are made at.
fn side(name: &str) -> crate::playhead_host::MapSide {
    side_of(pages(), Box::new(Names::new(Arc::new(stub()))), name)
}

fn stub() -> Stub {
    let localisation: Localisation = [("AAA", "Alpha"), ("CCC", "Gamma"), ("AAA_fascism", "Fascist Alpha")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Stub(Arc::new(localisation))
}

fn pages() -> Arc<dyn playhead::pages::Pages> {
    let strings = |at: u32| match at {
        0 => ["AAA", "BBB", "fascism"].map(str::to_owned).to_vec(),
        20 => vec!["CCC".to_owned()],
        _ => Vec::new(),
    };
    let (aaa, bbb, fascism, ccc) = (0, 1, 2, 3);
    let pages = recorded(80, strings, |at, store| {
        let gap = GAP.contains(&(at as i32));
        if at == 0 {
            store.table(TableId::Clock.index()).unwrap().insert_at(0, 0).unwrap();
            store.table(TableId::Clock.index()).unwrap().write(0, crate::layers::recorded::column(TableId::Clock, "speed"), Cell::I32(1)).unwrap();
        }
        store.status(TableId::Clock.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        for table in [TableId::Provinces, TableId::CountryLinks, TableId::Countries, TableId::CountryAppearance, TableId::Politics, TableId::PoliticalParties, TableId::Autonomy] {
            let status = if gap && table == TableId::Provinces { Status::NotRecorded } else { Status::Complete };
            store.status(table.index(), status, true, Some(at as i32), true).unwrap();
        }
        let country = |store: &mut replay_columnar::store::Store, slot: u32, native: u32, canonical: u32, tag: u32| {
            let mut countries = store.table(TableId::Countries.index()).unwrap();
            countries.insert_at(slot, u128::from(native)).unwrap();
            countries.write(slot, 0, Cell::U32(canonical)).unwrap();
            countries.write(slot, 1, Cell::Bytes(tag)).unwrap();
            let mut links = store.table(TableId::CountryLinks.index()).unwrap();
            links.insert_at(slot, u128::from(native)).unwrap();
            links.write(slot, 0, Cell::U32(canonical)).unwrap();
            let mut appearance = store.table(TableId::CountryAppearance.index()).unwrap();
            appearance.insert_at(slot, u128::from(native)).unwrap();
            appearance.write(slot, 7, Cell::I32(0)).unwrap();
            let mut politics = store.table(TableId::Politics.index()).unwrap();
            politics.insert_at(slot, u128::from(native)).unwrap();
            politics.write(slot, 4, Cell::OptionalU32(None)).unwrap();
        };
        let controller = |store: &mut replay_columnar::store::Store, province: u32, native: u32| {
            store.table(TableId::Provinces.index()).unwrap().write(province - 1, 2, Cell::U32(native)).unwrap();
        };
        match at {
            0 => {
                country(store, 0, 10, 1, aaa);
                country(store, 1, 20, 2, bbb);
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for province in 1..=6u32 {
                    provinces.insert_at(province - 1, u128::from(province)).unwrap();
                    provinces.write(province - 1, 2, Cell::U32(if province <= 3 { 10 } else { 20 })).unwrap();
                }
            }
            9 | 11 => controller(store, 3, 20),
            10 => controller(store, 3, 10),
            20 => store.table(TableId::Countries.index()).unwrap().write(1, 1, Cell::Bytes(ccc)).unwrap(),
            25 => {
                store.table(TableId::Politics.index()).unwrap().write(0, 4, Cell::OptionalU32(Some(7))).unwrap();
                let mut parties = store.table(TableId::PoliticalParties.index()).unwrap();
                parties.insert_at(0, key(10, 7)).unwrap();
                parties.write(0, 2, Cell::Bytes(fascism)).unwrap();
            }
            30 => {
                store.table(TableId::Provinces.index()).unwrap().delete(5).unwrap();
                country(store, 2, 30, 3, aaa);
                controller(store, 5, 30);
            }
            _ => {}
        }
    });
    pages
}

fn names(side: &crate::playhead_host::MapSide) -> &Names {
    home(side).expect("the names layer is read while the map is not composing")
}

/// The layer, or none while the map's layers are away composing (a view whose tables changed).
fn home(side: &crate::playhead_host::MapSide) -> Option<&Names> {
    side.parts().map(|(_, layers)| layers[0].any().downcast_ref::<Names>().unwrap())
}

/// What a wait saw last: the hour applied, a compose, and the layer's carried, published and shown.
fn state(side: &crate::playhead_host::MapSide) -> String {
    let layer = home(side).map(|layer| (layer.carried(), layer.published(), layer.shown().country.iter().map(|name| name.key.clone()).collect::<Vec<_>>()));
    format!("applied {:?}, composing {}, carried, published and shown {layer:?}", side.applied(), side.composing())
}

/// Steps of four hours, each taken once the layer has settled the last and published it (its
/// placement worker included); `each` sees every settled hour until it returns true, within 120 s.
fn settled_steps(side: &mut crate::playhead_host::MapSide, mut each: impl FnMut(&crate::playhead_host::MapSide, i32) -> bool) {
    let idle = |side: &crate::playhead_host::MapSide| !side.composing() && side.parts().is_some_and(|(_, layers)| layers[0].carried().entities == 0 && layers[0].published() == Published::Current);
    let (mut applied, mut checked) = (None::<i32>, None::<i32>);
    let deadline = Instant::now() + Duration::from_secs(120);
    while Instant::now() < deadline {
        let newest = side.entry().playhead().newest().unwrap_or(0);
        let head = match applied {
            Some(hour) if idle(side) && checked == Some(hour) => newest.min(hour + 4),
            Some(hour) => hour,
            None => newest,
        };
        let frame = side.frame(head).unwrap();
        applied = frame.applied.or(applied);
        if let Some(hour) = applied.filter(|&hour| idle(side) && checked != Some(hour)) {
            checked = Some(hour);
            if each(side, hour) {
                return;
            }
        } else if frame.steps == 0 {
            std::thread::yield_now();
        }
    }
    let layer = side.parts().map(|(_, layers)| (layers[0].carried(), layers[0].published()));
    panic!("the names layer did not settle what the test waits for within 120 s: applied {applied:?}, checked {checked:?}, composing {}, carried and published {layer:?}", side.composing());
}

#[test]
fn names_settled_every_step_equal_composed_and_follow_owners_operands_rows_and_gaps() {
    let mut side = side("names-steps");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut seen = Vec::new();
    settled_steps(&mut side, |side, hour| {
        let (mirrors, _) = side.parts().unwrap();
        let layer = names(side);
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.differ(&fresh), (0, None), "settled at hour {hour} equals composed");
        let area = |slot: u32| layer.areas().get(&slot);
        if GAP.contains(&hour) {
            assert!(layer.refused().is_some() && layer.placements().country.is_empty(), "Provinces not recorded draws no names at hour {hour}");
        } else if hour >= 12 {
            assert!(layer.refused().is_none());
            assert!(area(2).unwrap().members.contains(&3) && !area(1).unwrap().members.contains(&3), "a province changed twice in a step shows its newest owner at hour {hour}");
            if hour >= 24 {
                assert_eq!((area(2).unwrap().tag.as_deref(), &area(2).unwrap().text), (Some("CCC"), &Ok("Gamma".to_owned())), "a renamed tag at hour {hour}");
            }
            if hour >= 28 {
                assert_eq!(area(1).unwrap().text, Ok("Fascist Alpha".to_owned()), "a ruling party's group at hour {hour}");
            }
            if hour >= 32 {
                assert!(layer.areas().values().all(|area| !area.members.contains(&6)), "a deleted province is in no area at hour {hour}");
                assert_eq!(area(3).map(|area| area.members.iter().copied().collect::<Vec<_>>()), Some(vec![5]), "an inserted country's area at hour {hour}");
                let placed = layer.placements();
                assert_eq!(placed.country.iter().map(|name| name.key.as_str()).collect::<Vec<_>>(), ["AAA", "CCC", "AAA"]);
            }
        }
        seen.push(hour);
        hour >= 76
    });
    assert!(seen.iter().any(|hour| GAP.contains(hour)) && seen.iter().any(|&hour| hour >= 48), "{seen:?}");
    let latency = names(&side).latency();
    assert!(latency.worker_areas > 0 && latency.buckets.iter().sum::<u64>() == latency.worker_areas, "placed on the worker: {latency:?}");
}

/// The stub's placement, of `held`'s tag only, held at a gate the test opens: `None` lets every call
/// through, `Some(n)` n more. Counts the calls of that tag that reached the gate.
struct Gated {
    stub: Stub,
    held: &'static str,
    gate: Arc<(Mutex<(Option<u64>, u64)>, Condvar)>,
}

impl Atlas for Gated {
    fn provinces(&self) -> Vec<u32> {
        self.stub.provinces()
    }

    fn localisation(&self) -> Arc<Localisation> {
        self.stub.localisation()
    }

    fn components(&self, members: &BTreeSet<u32>) -> Vec<Vec<u32>> {
        self.stub.components(members)
    }

    fn name_layers(&self, mode: &MapMode) -> Option<NameLayers> {
        self.stub.name_layers(mode)
    }

    fn mode_names(&self, frame: &replay_frame::frame::DecodedFrame, mode: &MapMode) -> std::io::Result<Arc<Placements>> {
        self.stub.mode_names(frame, mode)
    }

    fn executable(&self) -> Option<engine_executable::executable_tables::Tables> {
        None
    }

    fn place(&self, tag: &str, text: &str, component: &[u32]) -> Option<WorldName> {
        if tag != self.held {
            return self.stub.place(tag, text, component);
        }
        let (lock, wake) = &*self.gate;
        let mut gate = lock.lock().unwrap();
        gate.1 += 1;
        wake.notify_all();
        while gate.0 == Some(0) {
            gate = wake.wait(gate).unwrap();
        }
        if let Some(permits) = gate.0.as_mut() {
            *permits -= 1;
        }
        drop(gate);
        self.stub.place(tag, text, component)
    }
}

/// The stepped harness settles once without a limit and then needs the hour whole: an area placed
/// again on that step is on screen, not still on the worker.
#[test]
fn names_settled_without_a_limit_carry_nothing_left_on_the_worker() {
    let mut side = side("names-unlimited");
    side.catch_up(8).unwrap();
    // Out of budget, the frames leave every change to the settle without a limit, which places them.
    side.set_budget(Duration::ZERO);
    side.catch_up(12).unwrap();
    let layer = names(&side);
    assert_eq!((layer.carried().entities, layer.published()), (0, Published::Current), "the hour province 3 changed hands in is placed whole: {:?}", layer.carried());
    assert!(layer.areas()[&2].members.contains(&3) && layer.areas()[&2].placed.iter().any(|(component, _)| component.contains(&3)), "placed with its newest owner");
}

/// Opens the gate when dropped, so a failed assertion does not leave the worker waiting at it while
/// the layer joins it.
struct Opens(Arc<(Mutex<(Option<u64>, u64)>, Condvar)>);

impl Drop for Opens {
    fn drop(&mut self) {
        let (lock, wake) = &*self.0;
        lock.lock().unwrap_or_else(std::sync::PoisonError::into_inner).0 = None;
        wake.notify_all();
    }
}

/// The worker makes a step's product once every area the step changed is placed. Alpha's
/// placement is held at the gate from its change in the step from hour 9, in which Beta changes too:
/// the frames go on without waiting, and the names show the product of hour 8, Beta's old placement
/// included, never anything of the step held. Released, the step's product is shown whole: Alpha's
/// and Beta's new placements, equal to the names composed there.
#[test]
fn names_with_a_placement_held_show_the_last_product_whole_and_never_part_of_the_step_held() {
    let pages = recorded(104, |at| if at == 0 { ["AAA", "BBB", "CCC"].map(str::to_owned).to_vec() } else { Vec::new() }, |at, store| {
        for table in [TableId::Provinces, TableId::CountryLinks, TableId::Countries, TableId::CountryAppearance, TableId::Politics, TableId::PoliticalParties, TableId::Autonomy] {
            store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
        }
        match at {
            0 => {
                for (slot, native, tag) in [(0, 10u32, 0), (1, 20, 1), (2, 30, 2)] {
                    for table in [TableId::Countries, TableId::CountryLinks, TableId::CountryAppearance, TableId::Politics] {
                        store.table(table.index()).unwrap().insert_at(slot, u128::from(native)).unwrap();
                    }
                    store.table(TableId::Countries.index()).unwrap().write(slot, 0, Cell::U32(slot + 1)).unwrap();
                    store.table(TableId::Countries.index()).unwrap().write(slot, 1, Cell::Bytes(tag)).unwrap();
                    store.table(TableId::CountryLinks.index()).unwrap().write(slot, 0, Cell::U32(slot + 1)).unwrap();
                    store.table(TableId::CountryAppearance.index()).unwrap().write(slot, 7, Cell::I32(0)).unwrap();
                    store.table(TableId::Politics.index()).unwrap().write(slot, 4, Cell::OptionalU32(None)).unwrap();
                }
                let mut provinces = store.table(TableId::Provinces.index()).unwrap();
                for province in 1..=6u32 {
                    provinces.insert_at(province - 1, u128::from(province)).unwrap();
                    provinces.write(province - 1, 2, Cell::U32(10 * province.div_ceil(2))).unwrap();
                }
            }
            9 => store.table(TableId::Provinces.index()).unwrap().write(1, 2, Cell::U32(20)).unwrap(),
            _ => {}
        }
    });
    let localisation: Localisation = [("AAA", "Alpha"), ("BBB", "Beta"), ("CCC", "Gamma")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    let gate = Arc::new((Mutex::new((None::<u64>, 0u64)), Condvar::new()));
    let atlas = Gated { stub: Stub(Arc::new(localisation)), held: "AAA", gate: Arc::clone(&gate) };
    let mut side = side_of(pages, Box::new(Names::new(Arc::new(atlas))), "names-held");
    let _opened_before_the_side_drops = Opens(Arc::clone(&gate));
    let (alpha, beta) = (1, 2);
    let deadline = Instant::now() + Duration::from_secs(120);
    let placed = |layer: &Names, slot: u32| layer.areas().get(&slot).map(|area| area.placed.iter().map(|(component, _)| component.clone()).collect::<Vec<_>>()).unwrap_or_default();
    let state = |side: &crate::playhead_host::MapSide| format!("applied {:?}, composing {}, published {:?}, gate {:?}", side.applied(), side.composing(), names(side).published(), *gate.0.lock().unwrap());
    let frames_to = |side: &mut crate::playhead_host::MapSide, hour: i32, what: &str, reached: &dyn Fn(&Names) -> bool| loop {
        let frame = side.frame(hour).unwrap();
        if side.applied() == Some(hour) && !side.composing() && reached(names(side)) {
            return;
        }
        assert!(Instant::now() < deadline, "frames at hour {hour} did not reach {what} within 120 s: {}", state(side));
        if frame.steps == 0 {
            std::thread::yield_now();
        }
    };
    side.catch_up(0).unwrap();
    let entered = gate.0.lock().unwrap().1;
    {
        let (lock, wake) = &*gate;
        lock.lock().unwrap().0 = Some(0);
        wake.notify_all();
    }
    frames_to(&mut side, 8, "hour 8 shown", &|layer| layer.published() == Published::Current);
    frames_to(&mut side, 12, "the step from hour 9 applied", &|_| true);
    {
        let (lock, wake) = &*gate;
        let mut held = lock.lock().unwrap();
        while held.1 <= entered {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "Alpha's placement did not reach the gate within 120 s");
            held = wake.wait_timeout(held, left).unwrap().0;
        }
    }
    for _ in 0..20 {
        side.frame(12).unwrap();
        let layer = names(&side);
        assert_eq!(layer.published(), Published::At(8), "the step from 9 is held: the product of 8 is shown: {}", state(&side));
        assert_eq!((placed(layer, alpha), placed(layer, beta)), (vec![vec![1, 2]], vec![vec![3, 4]]), "nothing of the step held is shown");
    }
    {
        let (lock, wake) = &*gate;
        lock.lock().unwrap().0 = None;
        wake.notify_all();
    }
    frames_to(&mut side, 12, "the step's product, released", &|layer| layer.published() == Published::Current);
    let layer = names(&side);
    assert_eq!((placed(layer, alpha), placed(layer, beta)), (vec![vec![1]], vec![vec![2, 3, 4]]), "Alpha's and Beta's placements of the step, together");
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer.fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer.differ(&fresh), (0, None), "at hour 12 equals composed");
}

#[test]
fn names_hidden_by_another_map_mode_are_refused_by_name_and_composed_again_when_shown() {
    let mut side = side("names-view");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side.set_view(View { map_mode: MapMode::Stock(Stock::Factions), ..View::default() }).unwrap();
    let refused: Vec<String> = side.refusals().map(|(name, why)| format!("{name}: {why}")).collect();
    assert!(refused.iter().any(|line| line.starts_with("names") && line.contains("map mode")), "{refused:?}");
    frames_until(&mut side, |side, frame| !side.composing() && frame.applied.is_some_and(|hour| hour >= 28));
    side.set_view(View::default()).unwrap();
    assert_eq!(side.refusals().count(), 0, "shown again, the layer is declared again");
    let mut composed = false;
    frames_until(&mut side, |side, frame| {
        composed |= frame.swapped.is_some();
        composed && !side.composing()
    });
    assert_eq!(names(&side).published(), Published::Current, "composed whole from the mirrors when shown again");
    settled_steps(&mut side, |side, hour| {
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = names(side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(names(side).differ(&fresh), (0, None), "shown again, settled at hour {hour} equals composed");
        hour >= 36
    });
}

/// A stand-in for the scene's names held by area: each area's names, and what each call wrote;
/// and the whole placements given their glyph buffers.
#[derive(Default)]
struct Held {
    generation: Option<u64>,
    areas: std::collections::BTreeMap<u32, Vec<WorldName>>,
    begun: usize,
    sets: Vec<u32>,
    prepared: Vec<u64>,
}

impl super::Target for Held {
    fn keyed(&self) -> Option<u64> {
        self.generation
    }

    fn begin(&mut self, generation: u64) {
        (self.generation, self.begun) = (Some(generation), self.begun + 1);
        self.areas.clear();
    }

    fn set(&mut self, generation: u64, slot: u32, names: &[&WorldName]) -> std::io::Result<()> {
        self.generation = Some(generation);
        self.sets.push(slot);
        if names.is_empty() {
            self.areas.remove(&slot);
        } else {
            self.areas.insert(slot, names.iter().map(|&name| name.clone()).collect());
        }
        Ok(())
    }

    fn install(&mut self, placements: &Placements) -> std::io::Result<()> {
        self.prepared.push(placements.generation);
        self.generation = None;
        Ok(())
    }

    fn holds(&self, placements: &Placements) -> bool {
        placements.country.is_empty() || self.prepared.last() == Some(&placements.generation) || self.generation == Some(placements.generation)
    }
}

impl Held {
    /// The scene's own precondition for drawing `placements` (`NamePass::upload`): no names, or
    /// glyph buffers prepared for them, whole or by area.
    fn takes(&self, placements: &Placements) -> bool {
        placements.country.is_empty() || self.prepared.contains(&placements.generation) || self.generation == Some(placements.generation)
    }

    /// The names drawn, in the order the pass draws them: area after area.
    fn drawn(&self) -> Vec<WorldName> {
        self.areas.values().flatten().cloned().collect()
    }
}

const AREAS: u32 = 200;

/// `AREAS` areas, each one country's single connected area with its name placed.
fn areas() -> Names {
    let empty = || Arc::new(std::iter::empty::<(String, String)>().collect::<Localisation>());
    let stub = Stub(empty());
    let mut layer = Names::new(Arc::new(Stub(empty())));
    for slot in 1..=AREAS {
        let tag = format!("T{slot:03}");
        let name = stub.place(&tag, "text", &[slot]).map(Arc::new);
        layer.areas.insert(slot, super::Area { members: BTreeSet::from([slot]), tag: Some(tag.clone()), text: Ok("text".into()), placed: vec![(vec![slot], name)], placed_as: Some((tag, "text".into())) });
    }
    layer
}

/// One area placed again: the upload writes that area's names alone, not every area's, and the
/// scene draws the layer's whole placements. Before, an upload cloned every name into new
/// placements and the scene built every glyph buffer again.
#[test]
fn names_upload_writes_only_the_area_placed_again() {
    let (mut layer, mut scene) = (areas(), Held::default());
    layer.push(&mut scene, &mut crate::layers::Budget::unlimited()).unwrap();
    assert_eq!((scene.begun, scene.sets.len()), (1, AREAS as usize), "composed, every area is written once");
    assert_eq!(scene.drawn(), layer.placements().country);
    assert!(layer.uploaded() && layer.drawn.generation == scene.generation.unwrap(), "the window's placements carry the scene's generation");

    let moved = Arc::new(WorldName { corners: vec![[100.0, 5.0, 0.0]], ..WorldName::clone(layer.areas[&100].placed[0].1.as_ref().unwrap()) });
    layer.areas.get_mut(&100).unwrap().placed[0].1 = Some(moved);
    layer.owe(100);
    assert!(!layer.uploaded());
    let sets = scene.sets.len();
    layer.push(&mut scene, &mut crate::layers::Budget::unlimited()).unwrap();
    assert_eq!(&scene.sets[sets..], [100], "one area written, of {AREAS}");
    assert_eq!(scene.begun, 1);
    assert_eq!(scene.drawn(), layer.placements().country, "the scene draws today's placements after the change");
    assert!(layer.uploaded() && layer.drawn.generation == scene.generation.unwrap());
}

/// Nothing changed: nothing is written and the window's placements stay the same.
#[test]
fn names_upload_with_no_area_changed_writes_nothing() {
    let (mut layer, mut scene) = (areas(), Held::default());
    layer.push(&mut scene, &mut crate::layers::Budget::unlimited()).unwrap();
    let (drawn, sets) = (Arc::clone(&layer.drawn), scene.sets.len());
    layer.push(&mut scene, &mut crate::layers::Budget::unlimited()).unwrap();
    assert_eq!(scene.sets.len(), sets, "nothing written");
    assert!(Arc::ptr_eq(&drawn, &layer.drawn), "the window's placements are the same");
}

/// The scene holding names the layer did not send (another placement drew there): every area is
/// sent again, an area an upload while the budget is spent, and the pieces end as the whole. The
/// window's placements follow the scene's generation after every piece.
#[test]
fn names_sent_again_whole_are_carried_across_spent_budgets_and_end_equal_to_whole() {
    let (mut layer, mut scene) = (areas(), Held::default());
    layer.push(&mut scene, &mut crate::layers::Budget::unlimited()).unwrap();
    scene.generation = Some(0);
    let mut uploads = 0;
    loop {
        let sets = scene.sets.len();
        layer.push(&mut scene, &mut crate::layers::Budget::new(Duration::ZERO).first_free()).unwrap();
        uploads += 1;
        assert!(scene.sets.len() - sets <= 1, "upload {uploads} writes one area at most");
        assert_eq!(Some(layer.drawn.generation), scene.generation, "the window draws what the scene holds after upload {uploads}");
        if layer.uploaded() {
            break;
        }
        assert!(uploads <= AREAS as usize + 1, "the upload never finished");
    }
    assert_eq!((scene.begun, uploads), (2, AREAS as usize), "begun again with the first area, then an area an upload");
    assert_eq!(scene.drawn(), layer.placements().country, "the pieces draw today's placements");
}

/// Plays a step a frame, as at 1,440 hours a second: between two steps, frames at the hour applied
/// (taking none) hand the layer the steps built past the head until its worker has made the next
/// step's product; then the frame that takes the step, which `each` sees until it returns true.
fn ahead_steps(side: &mut crate::playhead_host::MapSide, mut each: impl FnMut(&mut crate::playhead_host::MapSide, &crate::playhead_host::Frame, i32) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let next = side.applied().unwrap() + 4;
        loop {
            let applied = side.applied().unwrap();
            side.frame(applied).unwrap();
            if side.entry().playhead().newest() >= Some(next) && names(side).made_through() >= Some(next) {
                break;
            }
            assert!(Instant::now() < deadline, "the worker did not make hour {next} within 120 s");
            std::thread::yield_now();
        }
        let frame = side.frame(next).unwrap();
        assert_eq!((frame.applied, frame.steps), (Some(next), 1), "a step of four hours a frame");
        if each(side, &frame, next) {
            return;
        }
    }
}

/// Fed the steps ahead, the names' worker settles and places each step before the frame applies
/// it: in the frame that takes a step, the names show that step's hour (the product was ready),
/// equal to the names composed from the mirrors there, through owners, renames, operands, a deleted
/// row and a stretch the core is not recorded. Before, the frame settled the step and asked the
/// placements of it, and the names showed the hour before until they came back.
#[test]
fn names_fed_ahead_show_the_hour_applied_in_the_frame_that_applies_it() {
    let mut side = side("names-ahead");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let mut seen = 0;
    ahead_steps(&mut side, |side, frame, hour| {
        assert_eq!(frame.layers[0].ready_at_hour, Some(true), "the names of hour {hour} were ready in the frame that applied it");
        assert_eq!(names(side).published(), Published::Current, "the names show hour {hour}");
        side.settle_all().unwrap();
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = names(side).fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(names(side).differ(&fresh), (0, None), "at hour {hour} equals composed");
        seen += 1;
        hour >= 76
    });
    assert!(seen >= 18, "steps seen: {seen}");
}

/// A seek while products are made ahead restarts the worker from the mirrors at the hour sought: no
/// product made before it is shown. Sought back to hour 12 after the worker made the hours past the
/// rename at 20, the names show Beta's tag, not the rename's, until the step that renames it, and
/// equal the names composed at every step.
#[test]
fn names_sought_back_restart_from_the_mirrors_and_show_nothing_made_before() {
    let mut side = side("names-sought");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    ahead_steps(&mut side, |_, _, hour| hour >= 24);
    let applied = side.applied().unwrap();
    for _ in 0..50_000_000u64 {
        side.frame(applied).unwrap();
        if names(&side).made_through() >= Some(applied + 8) {
            break;
        }
        std::thread::yield_now();
    }
    assert_eq!(names(&side).areas()[&2].tag.as_deref(), Some("CCC"), "renamed at hour 20");
    side.entry().playhead().seek(12);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    assert_eq!(side.applied(), Some(12));
    let check = |side: &mut crate::playhead_host::MapSide, hour: i32| {
        side.settle_all().unwrap();
        let layer = names(side);
        let (mirrors, _) = side.parts().unwrap();
        let mut fresh = layer.fresh();
        fresh.compose(mirrors).unwrap();
        assert_eq!(layer.differ(&fresh), (0, None), "after the seek, at hour {hour} equals composed");
        let tag = if hour >= 20 { "CCC" } else { "BBB" };
        assert_eq!(layer.areas()[&2].tag.as_deref(), Some(tag), "Beta's tag at hour {hour}: nothing made before the seek is shown");
    };
    check(&mut side, 12);
    ahead_steps(&mut side, |side, _, hour| {
        check(side, hour);
        hour >= 28
    });
}

/// A naming string the recording does not name (one made before the recorder named every string at
/// a seek): the worker's read refuses it by name, as the mirror's read did. The area keeps its tag,
/// its name is the refusal, and the window's warnings carry it; never a name made as if the cell
/// were empty. On `8abb44618d` the cell read as empty and the area was named as if it had no
/// cosmetic tag.
#[test]
fn names_whose_naming_string_is_not_named_are_refused_by_name() {
    let mut side = side("names-unnamed");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    let (mirrors, _) = side.parts().unwrap();
    let layer = names(&side);
    let columns = layer.columns.unwrap();
    let mut tables = crate::layers::owned::Tables::from_mirrors(mirrors, &columns.shapes());
    let appearance = tables.get_mut(TableId::CountryAppearance).unwrap();
    let row = appearance.slot(10).expect("AAA's appearance row");
    appearance.unname(columns.cosmetic, row, 99);
    let atlas: Arc<dyn Atlas> = Arc::clone(&layer.content);
    let mut model = super::Model::new(Arc::clone(&atlas), columns, tables);
    let placers = super::Placers::new(&atlas);
    let placed = model.settle(&placers);
    let mut shown = layer.fresh();
    shown.take(model.made(0, placed));
    let area = &shown.areas()[&1];
    assert_eq!(area.tag.as_deref(), Some("AAA"), "the area keeps its tag");
    let why = area.text.as_ref().expect_err("refused, not named from an empty string");
    assert!(why.contains("cosmetic tag: string 99 is not named at the head"), "the mirror read's words: {why}");
    let warnings = shown.placements().warnings;
    assert!(warnings.iter().any(|line| line.contains("AAA: ") && line.contains("cosmetic tag: string 99 is not named at the head")), "the window's warning names it: {warnings:?}");
}

fn view(mode: Stock) -> View {
    View { map_mode: MapMode::Stock(mode), ..View::default() }
}

/// The layer as the window makes it, building a mode's whole names from the rows of its playhead,
/// at the default mode and composed.
fn windowed(name: &str) -> crate::playhead_host::MapSide {
    let pages = pages();
    let kinds = Arc::new((0..TableId::ALL.len()).map(|table| pages.kinds(table).map(<[replay_columnar::state::Kind]>::to_vec).unwrap_or_default()).collect());
    let timeline = playhead::Timeline::whole(&*pages).unwrap();
    let units = crate::playhead_host::units::Units { name: name.to_owned(), pages, timeline, kinds };
    let entry = crate::playhead_host::open(units, playhead::Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let layer = Names::new(Arc::new(stub())).in_window(Arc::clone(&entry), crate::playhead_host::rows::Recording { branch: 1 }).unwrap();
    let mut side = crate::playhead_host::MapSide::new(Arc::clone(&entry), vec![Box::new(layer)], View::default()).unwrap();
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    side
}

/// The names equal those composed from the mirrors now.
fn equal_composed(side: &crate::playhead_host::MapSide, what: &str) {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = names(side).fresh();
    fresh.compose(mirrors).unwrap();
    assert_eq!(names(side).differ(&fresh), (0, None), "{what}: equal to composed");
}

/// Today's rule decides what a mode draws. At the default mode and at another that writes the
/// countries' own names (resources here) the layer declares its tables and draws the names composed
/// for the mode, the same names, with no restart between them. At a mode that writes names over
/// areas that are not the countries' (factions here) it shows nothing of the old mode at once, then
/// today's names of that mode made from the rows at the head, and never a country name meanwhile.
/// Back at the default, nothing of the factions' names is shown, and the countries' names are
/// composed anew and equal the mirrors'. Before, every mode but the default was refused and drew
/// no names at all.
#[test]
fn names_at_each_mode_draw_what_todays_rule_writes_there_and_never_the_old_modes() {
    let mut side = windowed("names-modes");
    settled_steps(&mut side, |_, hour| hour >= 8);
    assert!(side.parts().unwrap().0.get(TableId::Provinces).is_some(), "the country names declare their tables");
    assert!(!names(&side).areas().is_empty());
    equal_composed(&side, "at the default mode");

    let before = names(&side).shown().generation;
    side.set_view(view(Stock::Resources)).unwrap();
    assert_eq!(side.refusals().count(), 0, "resources writes the countries' names: drawn, not refused");
    assert_eq!(names(&side).shown().generation, before, "the same names: nothing dropped, nothing restarted");
    settled_steps(&mut side, |_, hour| hour >= 12);
    assert!(side.parts().unwrap().0.get(TableId::Provinces).is_some());
    equal_composed(&side, "at the resources mode");

    side.set_view(view(Stock::Factions)).unwrap();
    assert_eq!(side.refusals().count(), 0, "the factions mode writes names: drawn, not refused");
    assert!(names(&side).shown().country.is_empty() && names(&side).shown().generation != before, "the old mode's names are dropped at once");
    assert!(names(&side).areas().is_empty());
    // The mode declares other tables: the map composes, and the layer is away until it swaps in.
    // A mode's whole names reach the window only through the upload, once the scene holds them.
    let mut scene = Held::default();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let applied = side.applied().unwrap();
        side.frame(applied).unwrap();
        if !side.composing() {
            side.upload_each(|layer, budget| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Names>()) {
                Some(names) => names.push_any(&mut scene, budget),
                None => Ok(()),
            })
            .unwrap();
        }
        if let Some(layer) = home(&side) {
            let shown = layer.shown();
            assert!(shown.country.iter().all(|name| name.key.starts_with("factions at ")), "only the factions mode's names are shown: {:?}", shown.country.iter().map(|name| &name.key).collect::<Vec<_>>());
            if !shown.country.is_empty() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "no names of the factions mode within 120 s: {}", state(&side));
        std::thread::yield_now();
    }
    assert!(side.parts().unwrap().0.get(TableId::Clock).is_some(), "a mode's whole names read the rows at the head");
    let modes = names(&side).modes.as_ref().unwrap();
    let built = modes.product().unwrap();
    assert_eq!(names(&side).shown().country[0].key, format!("factions at {}", built.hour));
    let again = modes.again(built).unwrap().unwrap();
    assert_eq!(again.country, names(&side).shown().country, "today's names of the mode, composed from the same rows");
    assert_eq!(names(&side).published(), Published::Cadence(built.hour), "made on the fill layer's cadence, at the hour of its rows");

    side.set_view(View::default()).unwrap();
    assert!(names(&side).shown().country.is_empty(), "nothing of the factions' names is shown at the default mode");
    // The clock ran on through the factions loop, so the hour reached here may be in GAP, where
    // Provinces are not recorded and today's rule draws no names.
    settled_steps(&mut side, |side, hour| {
        let layer = names(side);
        assert!(layer.shown().country.is_empty(), "back at the default mode no mode's names are shown at hour {hour}: {:?}; {}", layer.shown().country.iter().map(|name| &name.key).collect::<Vec<_>>(), state(side));
        if GAP.contains(&hour) {
            assert!(layer.refused().is_some() && layer.areas().is_empty(), "Provinces not recorded draws no names at hour {hour}");
        }
        hour >= 20 && !GAP.contains(&hour)
    });
    assert!(!names(&side).areas().is_empty(), "back at the default mode the countries' areas are named at a recorded hour: {}", state(&side));
    equal_composed(&side, "back at the default mode");
}

/// A mode's whole names handed to the window are installed again when the scene no longer holds
/// them (a new scene, as the window's arrives after the layer's first install, or another install
/// since): the window never draws names the scene did not prepare. Before, the layer installed a
/// product once and handed it on, and a scene that had not prepared it refused it by name.
#[test]
fn a_modes_whole_names_are_installed_again_in_a_scene_that_does_not_hold_them() {
    let mut side = windowed("names-whole-again");
    settled_steps(&mut side, |_, hour| hour >= 8);
    side.set_view(view(Stock::Factions)).unwrap();
    let mut scene = Held::default();
    let upload = |side: &mut crate::playhead_host::MapSide, scene: &mut Held| {
        side.upload_each(|layer, budget| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Names>()) {
            Some(names) => names.push_any(scene, budget),
            None => Ok(()),
        })
        .unwrap();
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let applied = side.applied().unwrap();
        side.frame(applied).unwrap();
        if !side.composing() {
            upload(&mut side, &mut scene);
        }
        if home(&side).is_some_and(|layer| !layer.shown().country.is_empty()) {
            break;
        }
        assert!(Instant::now() < deadline, "no names of the factions mode within 120 s: {}", state(&side));
        std::thread::yield_now();
    }
    let shown = Arc::clone(names(&side).shown());
    assert!(scene.takes(&shown), "the scene the layer installed into holds the names it hands on");
    let mut fresh = Held::default();
    assert!(!fresh.takes(&shown), "a new scene holds none of them");
    let applied = side.applied().unwrap();
    side.frame(applied).unwrap();
    upload(&mut side, &mut fresh);
    assert!(fresh.takes(names(&side).shown()), "the next upload gives the new scene the names the window is handed: {:?} prepared", fresh.prepared);
}

/// At a mode whose rule writes no names (terrain here) the layer declares no table, holds nothing,
/// hands the window no name and holds no hour back.
#[test]
fn names_at_a_mode_that_writes_none_declare_and_hold_nothing() {
    let mut side = windowed("names-none");
    settled_steps(&mut side, |_, hour| hour >= 8);
    side.set_view(view(Stock::Terrain)).unwrap();
    for _ in 0..4 {
        let applied = side.applied().unwrap();
        side.frame(applied).unwrap();
    }
    assert_eq!(side.refusals().count(), 0, "drawn, writing nothing; not refused");
    let (mirrors, _) = side.parts().unwrap();
    assert!(mirrors.get(TableId::Provinces).is_none() && mirrors.get(TableId::Clock).is_none(), "it declares no table");
    assert_eq!(names(&side).held(), 0, "it holds nothing");
    assert!(names(&side).shown().country.is_empty() && names(&side).areas().is_empty(), "it hands the window no name");
    assert_eq!(names(&side).published(), Published::Current);
}

/// The picture the window is handed after the upload, at every kind of mode (the countries' own
/// names, a mode's whole names, none), meets the scene's precondition for drawing it: no names, or
/// glyph buffers the scene prepared. On round 7 a mode's whole names were handed to the window
/// without being prepared, and the window stopped before its first frame at every such mode.
#[test]
fn names_hand_the_window_only_pictures_the_scene_has_prepared_at_every_kind_of_mode() {
    let mut side = windowed("names-prepared");
    let mut scene = Held::default();
    let upload = |side: &mut crate::playhead_host::MapSide, scene: &mut Held, what: &str| {
        let applied = side.applied().unwrap();
        side.frame(applied).unwrap();
        // While the map composes (a mode whose tables differ) the window is handed nothing new.
        if side.composing() {
            return false;
        }
        side.upload_each(|layer, budget| match layer.any_mut().and_then(|layer| layer.downcast_mut::<Names>()) {
            Some(names) => names.push_any(scene, budget),
            None => Ok(()),
        })
        .unwrap();
        let shown = names(side).shown();
        assert!(scene.takes(shown), "{what}: the window is handed names the scene has not prepared ({} names, generation {})", shown.country.len(), shown.generation);
        true
    };
    let deadline = Instant::now() + Duration::from_secs(120);
    for (mode, what) in [(View::default(), "the countries' own names"), (view(Stock::Factions), "a mode's whole names"), (view(Stock::Terrain), "a mode of none"), (view(Stock::Factions), "whole names again after none")] {
        side.set_view(mode).unwrap();
        let mut seen = false;
        while !seen {
            let uploaded = upload(&mut side, &mut scene, what);
            seen = uploaded && (what.contains("none") || !names(&side).shown().country.is_empty() || !scene.areas.is_empty());
            assert!(Instant::now() < deadline, "{what}: no names handed within 120 s: {}", state(&side));
            std::thread::yield_now();
        }
        let mut uploads = 0;
        while uploads < 3 {
            uploads += usize::from(upload(&mut side, &mut scene, what));
            assert!(Instant::now() < deadline, "{what}: three uploads after the names were handed not made within 120 s: {}", state(&side));
        }
    }
}

/// The names' worker had placed the four-hour steps built ahead, the rename at hour 20 among them,
/// when the pace fell to an hour a step at hour 16: those steps are built again an hour each, the
/// worker starts over from the mirrors at hour 16, and at every hour after, a step of one hour a
/// frame, the names equal those composed from the mirrors there and Beta keeps its tag until the
/// hour that renames it: nothing made from a step that was void is shown.
#[test]
fn names_after_a_slower_pace_start_over_and_equal_composed_at_each_hour() {
    let mut side = side("names-pace");
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    ahead_steps(&mut side, |_, _, hour| hour >= 16);
    let applied = side.applied().unwrap();
    let deadline = Instant::now() + Duration::from_secs(120);
    while names(&side).made_through() < Some(applied + 8) {
        assert!(Instant::now() < deadline, "the worker did not place hour {} within 120 s", applied + 8);
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
                assert_eq!(frame.steps, 1, "one step of one hour to hour {hour}");
                break;
            }
            std::thread::yield_now();
        }
        side.settle_all().unwrap();
        assert_eq!(names(&side).published(), Published::Current, "the names show hour {hour}: a restart stands for the hour applied");
        equal_composed(&side, &format!("hour {hour} after the slower pace"));
        let tag = if hour >= 20 { "CCC" } else { "BBB" };
        assert_eq!(names(&side).areas()[&2].tag.as_deref(), Some(tag), "Beta's tag at hour {hour}");
    }
    assert!(voided >= 1, "the steps the names placed ahead were voided and the worker started over");
}
