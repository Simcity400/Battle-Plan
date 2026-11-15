use super::{Naval, Roster, Ships};
use crate::{
    layers::{
        Layer, Published, View,
        recorded::{self, column, frames_until, through},
    },
    playhead_host::{
        MapSide, open,
        rows::{Recording, Rows},
    },
};
use playhead::Settings;
use replay_columnar::{manifest::Status, state::Cell, store::Store};
use replay_data::rows::TableId;
use replay_frame::frame::DecodedFrame;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use viewer_foundation::{
    map::{
        camera::{MapDefines, MapSize, Placement},
        counter_move::UnitRoute,
        perspective::{Choice, Vision},
    },
    unit_class::CounterClass,
};
use viewer_world::map::{
    equipment_graphics::Decided,
    unit_history::naval::{Candidate, Formation, Hull},
    unit_models::{Force, ForcePart, ResolvedHull, ResolvedRoster},
};

/// Province `p`'s box ten units wide at x `100 p`; ships drawn below a camera height of 500. A
/// figure is the task force's values as a candidate, and its roster as a formation; a roster is
/// one hull whose flags count the times that task force's roster was asked (203 has no ship).
/// With an `epoch`, every roster decides one graphics pool, whose limit holds from that hour on.
struct Fake {
    bounds: BTreeMap<u32, [f32; 4]>,
    asked: Mutex<BTreeMap<u64, u64>>,
    epoch: Option<i32>,
}

fn fake() -> Arc<Fake> {
    fake_with(None)
}

fn fake_with(epoch: Option<i32>) -> Arc<Fake> {
    Arc::new(Fake { bounds: [10, 20, 30, 2000].into_iter().map(|p| (p, [p as f32 * 100.0, 1000.0, p as f32 * 100.0 + 10.0, 1010.0])).collect(), asked: Mutex::new(BTreeMap::new()), epoch })
}

impl Fake {
    fn asked(&self, force: u64) -> u64 {
        self.asked.lock().unwrap().get(&force).copied().unwrap_or(0)
    }
}

impl Naval for Fake {
    fn drawn_at(&self, height: f32) -> bool {
        height < 500.0
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        &self.bounds
    }

    fn map(&self) -> (MapSize, MapDefines) {
        (MapSize::TEST, MapDefines::test())
    }

    fn region(&self, province: u32) -> Option<u32> {
        Some(province / 10)
    }

    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, _vision: &Vision, _skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, _speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart {
        let route = route.map(|route| route.display_path().to_vec()).unwrap_or_default();
        let candidate = Candidate {
            province: force.province,
            identity: force.id,
            ordinal: ordinal.unwrap_or(u32::MAX),
            slot: 0,
            priority: link(force.logical_owner).map_or(9, |slot| slot as u8),
            ours: false,
            class: CounterClass::TaskForce,
            land_eligible: false,
            sample_count: 0,
            next_arc: 0.0,
            route,
            speed: 0.0,
            min_arc: 0.0,
            arrival_allowed: false,
            day,
        };
        let formation = match roster() {
            Some(Ok(roster)) => Some(Formation {
                identity: force.id,
                province: force.province,
                slot: 0,
                roster: roster.hulls.iter().filter_map(|hull| match hull {
                    ResolvedHull::Hull { flags, graphics, .. } => Some(Hull { flags: *flags, graphics: graphics.clone(), targets: [None; 4] }),
                    ResolvedHull::Skipped(_) => None,
                }).collect(),
            }),
            _ => None,
        };
        ForcePart { candidate: Some(candidate), position: None, formation, report: Default::default() }
    }

    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)> {
        let mut asked = self.asked.lock().unwrap();
        let rosters = forces
            .iter()
            .map(|&force| {
                let count = asked.entry(force).or_default();
                *count += 1;
                (force, (force != 203).then(|| Ok(ResolvedRoster { first_flags: None, slots: Vec::new(), hulls: vec![ResolvedHull::Hull { ship: force, flags: *count, graphics: Vec::new(), slot: Ok(None) }] })))
            })
            .collect();
        let decided = self.epoch.map(|epoch| (0, (Arc::from(Vec::new()), Ok(frame.hour >= epoch)))).into_iter().collect();
        Ok((rosters, decided))
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        decided.values().all(|(_, was)| self.epoch.map(|epoch| frame.hour >= epoch) == was.clone().ok())
    }
}

const TABLES: [TableId; 11] = [
    TableId::TaskForces,
    TableId::TaskForceMapMotion,
    TableId::TaskForceCounterInputs,
    TableId::TaskForceMissions,
    TableId::ProvinceUnitOrder,
    TableId::Ships,
    TableId::ShipModels,
    TableId::VariantDefinitions,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::Clock,
];
const HOURS: u32 = 320;

fn task_force(store: &mut Store, slot: u32, id: u64, province: u32, owner: u32) {
    let mut forces = store.table(TableId::TaskForces.index()).unwrap();
    forces.insert_at(slot, u128::from(id)).unwrap();
    for (field, value) in [("province", province), ("logical_owner", owner), ("roster_owner", owner), ("expeditionary_owner", 0), ("mission", 0)] {
        forces.write(slot, column(TableId::TaskForces, field), Cell::U32(value)).unwrap();
    }
    let mut motion = store.table(TableId::TaskForceMapMotion.index()).unwrap();
    motion.insert_at(slot, u128::from(id)).unwrap();
    motion.write(slot, column(TableId::TaskForceMapMotion, "target"), Cell::U32(0)).unwrap();
}

fn ship(store: &mut Store, slot: u32, id: u64, force: u64, variant: u64) {
    let mut ships = store.table(TableId::Ships.index()).unwrap();
    ships.insert_at(slot, u128::from(id)).unwrap();
    ships.write(slot, column(TableId::Ships, "task_force"), Cell::U64(force)).unwrap();
    ships.write(slot, column(TableId::Ships, "class_id"), Cell::I32(5)).unwrap();
    let mut models = store.table(TableId::ShipModels.index()).unwrap();
    models.insert_at(slot, u128::from(id)).unwrap();
    models.write(slot, column(TableId::ShipModels, "variant"), Cell::U64(variant)).unwrap();
}

fn order(store: &mut Store, slot: u32, province: u32, mixed: &[u64]) {
    let mut order = store.table(TableId::ProvinceUnitOrder.index()).unwrap();
    order.insert_at(slot, u128::from(province)).unwrap();
    order.write(slot, column(TableId::ProvinceUnitOrder, "mixed"), Cell::U64List(mixed)).unwrap();
}

/// Three task forces of two countries in three provinces, one where no box stands; a task force
/// sails to another province, a ship's model changes twice in two hours, a task force is deleted
/// and another inserted with a ship, and ShipModels is not recorded for a stretch.
fn campaign(at: u32, store: &mut Store) {
    for table in TABLES {
        let status = if table == TableId::ShipModels && (180..220).contains(&at) { Status::NotRecorded } else { Status::Complete };
        store.status(table.index(), status, true, Some(at as i32), true).unwrap();
    }
    match at {
        0 => {
            let mut links = store.table(TableId::CountryLinks.index()).unwrap();
            for (slot, (key, canonical)) in [(7u32, 3u32), (8, 4)].into_iter().enumerate() {
                links.insert_at(slot as u32, u128::from(key)).unwrap();
                links.write(slot as u32, column(TableId::CountryLinks, "canonical_slot"), Cell::U32(canonical)).unwrap();
            }
            let mut countries = store.table(TableId::Countries.index()).unwrap();
            for (slot, key) in [3u32, 4].into_iter().enumerate() {
                countries.insert_at(slot as u32, u128::from(key)).unwrap();
            }
            let mut clock = store.table(TableId::Clock.index()).unwrap();
            clock.insert_at(0, 0).unwrap();
            clock.write(0, column(TableId::Clock, "speed"), Cell::I32(1)).unwrap();
            for (slot, (id, province, owner)) in [(201, 10, 7), (202, 20, 8), (203, 2000, 7)].into_iter().enumerate() {
                task_force(store, slot as u32, id, province, owner);
            }
            for (slot, (id, force, variant)) in [(301, 201, 1), (302, 201, 1), (303, 202, 1)].into_iter().enumerate() {
                ship(store, slot as u32, id, force, variant);
            }
            order(store, 0, 10, &[201]);
            order(store, 1, 20, &[202]);
        }
        50 => {
            store.table(TableId::TaskForces.index()).unwrap().write(0, column(TableId::TaskForces, "province"), Cell::U32(30)).unwrap();
            store.table(TableId::ProvinceUnitOrder.index()).unwrap().delete(0).unwrap();
            order(store, 2, 30, &[201]);
        }
        100 | 101 => {
            let variant = if at == 100 { 2 } else { 3 };
            store.table(TableId::ShipModels.index()).unwrap().write(2, column(TableId::ShipModels, "variant"), Cell::U64(variant)).unwrap();
        }
        130 => {
            store.table(TableId::TaskForces.index()).unwrap().delete(2).unwrap();
            store.table(TableId::TaskForceMapMotion.index()).unwrap().delete(2).unwrap();
            task_force(store, 3, 204, 20, 7);
            ship(store, 3, 304, 204, 7);
            store.table(TableId::ProvinceUnitOrder.index()).unwrap().write(1, column(TableId::ProvinceUnitOrder, "mixed"), Cell::U64List(&[202, 204])).unwrap();
        }
        _ => {}
    }
}

fn side_at(name: &str, view: View) -> MapSide {
    side_with(name, view, fake())
}

fn side_with(name: &str, view: View, naval: Arc<Fake>) -> MapSide {
    side_of(name, view, naval, campaign)
}

fn side_of(name: &str, view: View, naval: Arc<dyn Naval>, campaign: fn(u32, &mut Store)) -> MapSide {
    let units = recorded::record(name, HOURS, campaign);
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 1.0 }).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Ships::new(naval, false).unwrap())], view).unwrap();
    layer(&side).set_source(Arc::clone(&entry), Recording { branch: 1 }).unwrap();
    layer(&side).set_vision(Choice::default(), Arc::new(Vision::default()));
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_until(&mut side, |side, frame| frame.swapped.is_some() && !side.composing());
    settled(&mut side);
    side
}

fn side(name: &str) -> MapSide {
    side_at(name, View::default())
}

fn layer(side: &MapSide) -> &Ships {
    side.parts().unwrap().1[0].any().downcast_ref::<Ships>().unwrap()
}

/// The rosters asked answered at the head and taken: every figure made. While a reader's tables
/// load (they join the playhead past its head) the head moves on a step, as the window's would.
fn settled(side: &mut MapSide) {
    frames_until(side, |side, _| !side.composing());
    if !layer(side).drawn() {
        return;
    }
    for _ in 0..200 {
        if side.parts().is_none() {
            frames_until(side, |side, _| !side.composing());
        }
        let hour = side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::TaskForces).and_then(|mirror| mirror.head())).unwrap_or(0);
        let _ = layer(side).rosters().wait(hour, Duration::from_millis(200));
        side.settle_all().unwrap();
        let composed = layer(side).refused().is_none() || layer(side).refused().is_some_and(|why| why.contains("not wholly recorded"));
        if composed && layer(side).parts().values().all(Option::is_some) && !layer(side).rosters().pending() {
            return;
        }
        if !composed || layer(side).rosters().loading() {
            frames_until(side, |side, frame| frame.steps > 0 && !side.composing());
        }
    }
    panic!("the rosters were never answered");
}

fn to(side: &mut MapSide, hour: i32) {
    through(side, hour);
    settled(side);
}

fn composed_equal(side: &MapSide) {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh().unwrap();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

/// Each task force's province, ordinal and hull flags, by id.
fn drawn(side: &MapSide) -> Vec<(u64, u32, u32, Vec<u64>)> {
    let (sources, _) = layer(side).sources();
    sources
        .candidates
        .iter()
        .map(|candidate| {
            let hulls = sources.formations.iter().find(|formation| formation.identity == candidate.identity).map(|formation| formation.roster.iter().map(|hull| hull.flags).collect()).unwrap_or_default();
            (candidate.identity, candidate.province, candidate.ordinal, hulls)
        })
        .collect()
}

#[test]
fn a_task_force_changed_in_a_step_is_marked_once_and_settled_to_the_composed_value() {
    let mut side = side("ships-once");
    assert_eq!(drawn(&side), vec![(201, 10, 0, vec![1]), (202, 20, 0, vec![1]), (203, 2000, u32::MAX, vec![])]);
    composed_equal(&side);
    to(&mut side, 30);
    let before = layer(&side).marked();
    side.set_budget(Duration::ZERO);
    through(&mut side, 50);
    assert_eq!(layer(&side).marked()[0] - before[0], 1, "the task force that sailed is marked once");
    assert!(layer(&side).carried().entities >= 1, "nothing settles without budget");
    side.set_budget(Duration::from_secs(1));
    to(&mut side, 64);
    assert_eq!(layer(&side).carried().entities, 0);
    assert_eq!(drawn(&side)[0], (201, 30, 0, vec![1]), "a task force that sails keeps its roster: none is asked again");
    composed_equal(&side);
}

#[test]
fn a_ship_changed_twice_before_it_settles_shows_its_newest_values() {
    let naval = fake();
    let mut side = side_with("ships-twice", View::default(), Arc::clone(&naval));
    to(&mut side, 90);
    side.set_budget(Duration::ZERO);
    through(&mut side, 102);
    side.set_budget(Duration::from_secs(1));
    to(&mut side, 120);
    assert_eq!(layer(&side).carried().entities, 0);
    let asked = naval.asked(202);
    assert!(asked >= 2, "the roster of the task force whose ship changed is asked again");
    assert_eq!(drawn(&side)[1], (202, 20, 0, vec![asked]), "the figure takes the newest answer");
    assert_eq!(naval.asked(201), 1, "no other roster is asked again");
    composed_equal(&side);
}

#[test]
fn a_task_force_deleted_or_inserted_updates_every_array_that_names_it() {
    let naval = fake();
    let mut side = side_with("ships-rows", View::default(), Arc::clone(&naval));
    to(&mut side, 140);
    assert_eq!(drawn(&side), vec![(201, 30, 0, vec![1]), (202, 20, 0, vec![naval.asked(202)]), (204, 20, 1, vec![naval.asked(204)])], "refused {:?}, counts {:?}, carried {:?}", layer(&side).refused(), layer(&side).counts(), layer(&side).carried());
    assert!(naval.asked(204) >= 1);
    composed_equal(&side);
}

#[test]
fn a_table_not_recorded_draws_no_ship_as_today() {
    let mut side = side("ships-gap");
    to(&mut side, 190);
    assert!(layer(&side).refused().is_some_and(|why| why.contains("ShipModels is not wholly recorded")), "{:?}", layer(&side).refused());
    assert!(drawn(&side).is_empty());
    to(&mut side, 230);
    assert_eq!(layer(&side).refused(), None);
    assert_eq!(drawn(&side).len(), 3);
    composed_equal(&side);
}

fn over(province: u32, height: f32) -> View {
    let camera = Placement::map([province as f32 * 100.0 + 5.0, 1005.0], height);
    View { camera: Some(camera), camera_height: height, viewport: [2048, 1152], ..View::default() }
}

#[test]
fn a_view_that_draws_no_ship_declares_nothing_and_one_that_does_composes() {
    let kinds = replay::columnar_frames::schema().unwrap();
    let mut far = Ships::new(fake(), false).unwrap();
    let mut inputs = crate::layers::Inputs::new(&kinds, "ships");
    far.inputs(&over(10, 600.0), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "above the cutoff the layer reads nothing");
    let mut side = side("ships-far");
    to(&mut side, 60);
    side.set_view(over(10, 600.0)).unwrap();
    through(&mut side, 64);
    assert!(side.refusals().next().is_none());
    assert!(!layer(&side).drawn());
    assert_eq!(layer(&side).counts(), [0, 0], "it holds nothing");
    assert_eq!((layer(&side).carried().entities, layer(&side).published()), (0, Published::Current), "and carries and owes nothing");
    let rows = Rows::of(side.entry(), Recording { branch: 1 }).unwrap();
    assert!(rows.released(TableId::ShipModels, Instant::now() + Duration::from_secs(30)), "closed, the rosters' reader lets its tables go");
    // Drawn again, the rosters declare their tables anew and they arrive.
    side.set_view(View::default()).unwrap();
    frames_until(&mut side, |side, frame| frame.applied.is_some_and(|hour| hour >= 72) && !side.composing());
    settled(&mut side);
    assert_eq!(drawn(&side).len(), 3);
    composed_equal(&side);
}

#[test]
fn a_task_force_outside_the_view_gets_its_figure_when_the_camera_admits_it() {
    let mut side = side_at("ships-admitted", over(10, 100.0));
    assert_eq!(layer(&side).counts(), [3, 1]);
    assert_eq!(drawn(&side), vec![(201, 10, 0, vec![1])]);
    to(&mut side, 64);
    assert!(drawn(&side).is_empty(), "it sailed out of view");
    side.set_view(over(30, 100.0)).unwrap();
    to(&mut side, 72);
    assert_eq!(drawn(&side), vec![(201, 30, 0, vec![1])]);
    composed_equal(&side);
}

#[test]
fn a_task_force_whose_roster_is_answered_before_it_is_read_still_gets_its_figure() {
    let mut side = side("ships-born");
    to(&mut side, 120);
    side.set_budget(Duration::ZERO);
    through(&mut side, 130);
    // The worker's acknowledgement: the roster of the task force born at 130 is answered, told on
    // its condvar, before the layer, held by its zero budget, reads it. The worker changes what it
    // answered under the lock it notifies after, so no answer is missed.
    let shared = &layer(&side).rosters().shared;
    let mut asked = shared.lock();
    while !(asked.known.contains(&204) && asked.wanted.is_empty()) {
        asked = shared.answered.wait(asked).unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    drop(asked);
    assert!(!layer(&side).rosters().pending() && !layer(&side).rosters().loading());
    side.set_budget(Duration::from_secs(1));
    side.settle_all().unwrap();
    let parts = layer(&side).parts();
    assert!(parts.get(&204).is_some_and(Option::is_some), "the task force born at 130 has its figure: {:?}", parts.iter().map(|(id, part)| (*id, part.is_some())).collect::<Vec<_>>());
}

/// The campaign with a land occupant (999) beside task force 202 in province 20 from hour 60 to 119.
fn contested(at: u32, store: &mut Store) {
    campaign(at, store);
    let mixed: &[u64] = match at {
        60 => &[202, 999],
        120 => &[202],
        _ => return,
    };
    store.table(TableId::ProvinceUnitOrder.index()).unwrap().write(1, column(TableId::ProvinceUnitOrder, "mixed"), Cell::U64List(mixed)).unwrap();
}

/// The window's hand-over: drawn, the layer stands in and its ships are what the window's unit models
/// carry (today's made without ships, the layer's joined); while a land occupant competes in a task
/// force's province it does not, so today's ships are made again; hidden, or refused where today's
/// builder draws nothing, it stands in with nothing drawn and the refusal named.
#[test]
fn the_window_draws_the_layers_ships_and_todays_where_a_land_occupant_competes() {
    use viewer_world::map::unit_models::{PreparedModels, naval_differ};
    let mut side = side_of("ships-handover", View::default(), fake(), contested);
    assert!(layer(&side).stands_in(), "drawn and equal to today's, it stands in ({:?})", layer(&side).needs_today());
    let first = layer(&side).seq();
    assert!(first > 0, "what it draws is published");
    let today = PreparedModels::drawn_by_layers();
    assert!(!today.naval(), "today's models made without ships hold none");
    let (sources, report) = layer(&side).handover();
    assert_eq!(layer(&side).warnings(), report.warnings(), "the kept lines are the handover's");
    let joined = today.with_ships(sources, &report);
    assert!(joined.naval());
    let (_, differing, why) = naval_differ(joined.ships(), &layer(&side).sources().0);
    assert_eq!((differing, why), (0, None), "the window's ships are the layer's");
    assert_eq!(joined.ships().candidates.len(), 3);

    to(&mut side, 64);
    assert!(!layer(&side).stands_in(), "a land occupant competes beside 202: today's ships are made again");
    assert!(layer(&side).needs_today().is_some_and(|why| why.contains("province 20")), "{:?}", layer(&side).needs_today());

    to(&mut side, 124);
    assert!(layer(&side).stands_in(), "the occupant left: the layer stands in from the next hour ({:?})", layer(&side).needs_today());
    assert!(layer(&side).seq() > first);

    side.set_view(over(10, 600.0)).unwrap();
    through(&mut side, 128);
    assert!(!layer(&side).drawn());
    assert!(layer(&side).stands_in(), "hidden, it draws nothing, as today's draws nothing there");
    assert!(layer(&side).handover().0.candidates.is_empty(), "and hands over no ship");

    side.set_view(View::default()).unwrap();
    to(&mut side, 190);
    assert!(layer(&side).refused().is_some_and(|why| why.contains("ShipModels is not wholly recorded")), "{:?}", layer(&side).refused());
    assert!(layer(&side).stands_in(), "refused where today's builder draws nothing, it stands in");
    let (sources, report) = layer(&side).handover();
    assert!(sources.candidates.is_empty());
    assert!(report.warnings().iter().any(|line| line.contains("ShipModels is not wholly recorded")), "the refusal is named where the window shows its model warnings: {:?}", report.warnings());
    assert_eq!(layer(&side).warnings(), report.warnings(), "kept as the handover words them, the refusal named");
}

/// [`Fake`]'s rosters, held on the worker while the test holds the gate (a minute at most, after
/// which the batch goes on and the test's assertions fail by name).
struct HeldRosters {
    fake: Arc<Fake>,
    open: Mutex<bool>,
    changed: std::sync::Condvar,
}

impl HeldRosters {
    fn set(&self, open: bool) {
        *self.open.lock().unwrap() = open;
        self.changed.notify_all();
    }
}

impl Naval for HeldRosters {
    fn drawn_at(&self, height: f32) -> bool {
        self.fake.drawn_at(height)
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        self.fake.bounds()
    }

    fn map(&self) -> (MapSize, MapDefines) {
        self.fake.map()
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.fake.region(province)
    }

    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, vision: &Vision, skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart {
        self.fake.figure(force, route, link, vision, skip, ordinal, speed, day, roster)
    }

    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)> {
        let until = Instant::now() + Duration::from_secs(60);
        let mut open = self.open.lock().unwrap();
        while !*open {
            let Some(left) = until.checked_duration_since(Instant::now()) else { break };
            open = self.changed.wait_timeout(open, left).unwrap().0;
        }
        drop(open);
        self.fake.rosters(frame, forces)
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        self.fake.decide_the_same(frame, decided)
    }
}

/// What a wait in the roster tests saw last: the hour applied, a compose, and the layer's carried,
/// waiting, asked and published.
fn roster_state(side: &MapSide) -> String {
    let layer = side.parts().map(|_| layer(side)).map(|layer| (layer.carried(), layer.waiting(), layer.rosters().pending(), layer.published()));
    format!("applied {:?}, composing {}, carried, waiting, asked and published {layer:?}", side.applied(), side.composing())
}

/// Frames with the head at `hour` until it is applied, every roster asked is answered and the layer
/// carries nothing and waits for nothing.
fn settled_at(side: &mut MapSide, hour: i32) {
    loop {
        let frame = side.frame(hour).unwrap();
        if side.applied() == Some(hour) && !side.composing() {
            // Settled once the rosters' rows stand at the hour: until then the layer says the hour
            // they stand at, never current.
            let answered = layer(side).rosters().wait(hour, Duration::from_millis(200)).is_ok();
            side.settle_all().unwrap();
            let held = layer(side);
            if answered && held.carried().entities == 0 && held.waiting() == 0 && !held.rosters().pending() {
                return;
            }
        }
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
}

/// While a task force waits for its roster on the worker (held by the test: task force 202's ship
/// changes model at hours 100 and 101) the layer carries nothing for it and publishes the hour its
/// parts stood for before the change, which the host shows; answered, it is current again. On the
/// base the waiting task force was carried and the layer published `Current`, so the host kept the
/// hour it last carried nothing at. The head moves a step at a time, so no step past the change is
/// taken before the worker is held.
#[test]
fn a_roster_out_on_the_worker_is_not_carried_and_the_hour_the_parts_stand_for_is_shown() {
    let held = Arc::new(HeldRosters { fake: fake(), open: Mutex::new(true), changed: std::sync::Condvar::new() });
    let mut side = side_of("ships-roster-out", View::default(), Arc::clone(&held) as Arc<dyn Naval>, campaign);
    let before = 96;
    let applied = side.applied().expect("composed");
    assert!(applied < before, "the side opened at hour {applied}, past {before}");
    for hour in (applied + 1..=before).filter(|hour| hour % 4 == 0) {
        settled_at(&mut side, hour);
    }
    assert_eq!((layer(&side).carried().entities, layer(&side).published()), (0, Published::Current));
    held.set(false);
    let head = |side: &MapSide| side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::TaskForces).and_then(|mirror| mirror.head()));
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let frame = side.frame(before + 8).unwrap();
        if !side.composing() && head(&side).is_some_and(|hour| hour >= 101) && layer(&side).waiting() > 0 && layer(&side).carried().entities == 0 {
            break;
        }
        assert!(Instant::now() < deadline, "task force 202 was not waiting for its roster at hour 101 within 120 s: {}", roster_state(&side));
        if frame.steps == 0 && frame.swapped.is_none() {
            std::thread::yield_now();
        }
    }
    assert_eq!(layer(&side).published(), Published::At(before), "a roster out publishes the hour the parts stand for, the last before the change at 100");
    assert_eq!(side.last_frame().layers[0].shown, Some(before), "the host shows that hour");
    held.set(true);
    settled_at(&mut side, before + 8);
    assert_eq!(layer(&side).published(), Published::Current, "answered, current again");
    assert!(drawn(&side)[1].3[0] >= 2, "task force 202 takes its newest roster");
}

/// No task force waits for its roster, but the rosters drawn were answered from rows that stand
/// behind the hour applied (the rows' worker held by the test while the map takes the step from 88
/// to 92, which changes no ship): the layer says the hour those rows stand at, never current;
/// released, the rows stand at the hour applied and it is current. (Steps end at hours a multiple
/// of four, and the world changes a ship at hour 100.) On the base it said current whenever no
/// task force waited, whatever the age of the rows its rosters came from.
#[test]
fn rosters_drawn_from_rows_behind_the_hour_applied_publish_their_hour_and_never_current() {
    let mut side = side("ships-rows-behind");
    let before = 88;
    settled_at(&mut side, before);
    assert_eq!(layer(&side).published(), Published::Current);
    let rows = Rows::of(side.entry(), Recording { branch: 1 }).unwrap();
    let pause = rows.hold_before_take();
    assert!(pause.holding(Instant::now() + Duration::from_secs(60)), "the rows' worker was not held");
    let deadline = Instant::now() + Duration::from_secs(120);
    while side.applied() != Some(before + 4) || side.composing() {
        side.frame(before + 4).unwrap();
        assert!(Instant::now() < deadline, "hour {} was not applied within 120 s: {}", before + 4, roster_state(&side));
        std::thread::yield_now();
    }
    side.settle_all().unwrap();
    assert_eq!(layer(&side).waiting(), 0, "no task force waits for its roster");
    assert_eq!(layer(&side).published(), Published::Cadence(before), "the rosters' rows stand at {before}, the map applied {}", before + 4);
    pause.release();
    settled_at(&mut side, before + 4);
    let deadline = Instant::now() + Duration::from_secs(120);
    while layer(&side).published() != Published::Current {
        side.frame(before + 4).unwrap();
        assert!(Instant::now() < deadline, "the rosters' rows never stood at the hour applied: {}", roster_state(&side));
        std::thread::yield_now();
    }
}

/// [`campaign`] with the Clock's hour written every hour, as the recorder writes it.
fn clocked(at: u32, store: &mut Store) {
    campaign(at, store);
    store.table(TableId::Clock.index()).unwrap().write(0, column(TableId::Clock, "hour"), Cell::I32(at as i32)).unwrap();
}

/// A graphics pool whose limit compares the date (it holds from hour 70) decides the rosters, and
/// no table they read changes between hours 64 and 76: past 70 they are resolved again, at the hour
/// the pool decides otherwise. On the base only a changed table resolved a known roster again, and
/// task force 202 kept the roster the pool chose before hour 70.
#[test]
fn a_roster_whose_graphics_limit_compares_the_date_is_resolved_again_when_it_decides_otherwise() {
    let naval = fake_with(Some(70));
    let mut side = side_of("ships-dated-limit", View::default(), Arc::clone(&naval) as Arc<dyn Naval>, clocked);
    settled_at(&mut side, 64);
    let before = naval.asked(202);
    settled_at(&mut side, 68);
    assert_eq!(naval.asked(202), before, "the pool decides the same before hour 70: nothing resolved again");
    settled_at(&mut side, 76);
    assert!(naval.asked(202) > before, "the pool decides otherwise past hour 70: task force 202's roster resolved again");
    assert_eq!(drawn(&side)[1].3, vec![naval.asked(202)], "and drawn from that answer");
}
