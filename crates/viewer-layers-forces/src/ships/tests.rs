use super::{Naval, Roster, Ships};
use viewer_layer_core::{layers::{Layer, Published, View}, test_support::recorded::{self, column}, playhead_host::{MapSide, open, views::Recording}};
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
use viewer_gpu::camera::{MapDefines, MapSize, Placement};
use viewer_unit_reads::counter_move::UnitRoute;
use viewer_foundation::perspective::{Choice, Vision};
use viewer_runtime::unit_class::CounterClass;
use viewer_unit_models::{
    equipment_graphics::Decided,
    unit_models::{Force, ForcePart, ResolvedHull, ResolvedRoster},
};
use viewer_unit_history::unit_history::naval::{Candidate, Formation, Hull};

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
    let entry = open(units, Settings { step_threads: 1, fetch_threads: 1, ahead: 2, ring: 3, speed: 4.0, rate: 4.0, ..Settings::default() }).unwrap();
    let mut side = MapSide::new(Arc::clone(&entry), vec![Box::new(Ships::new(naval).unwrap())], view).unwrap();
    layer(&side).set_source(Arc::clone(&entry), Recording { branch: 1 }).unwrap();
    layer(&side).set_vision(Choice::default(), Arc::new(Vision::default()));
    entry.playhead().play(4.0);
    entry.playhead().seek(0);
    frames_at(&mut side, 0, |side, frame| frame.swapped.is_some() && !side.composing());
    settled(&mut side, 0);
    side
}

fn side(name: &str) -> MapSide {
    side_at(name, View::default())
}

fn layer(side: &MapSide) -> &Ships {
    side.parts().unwrap().1[0].any().downcast_ref::<Ships>().unwrap()
}

fn frames_at(side: &mut MapSide, hour: i32, mut done: impl FnMut(&MapSide, &viewer_layer_core::playhead_host::Frame) -> bool) {
    for _ in 0..50_000_000u64 {
        recorded::reads_landed(side);
        let frame = side.frame(hour).unwrap();
        if done(side, &frame) {
            return;
        }
        std::thread::yield_now();
    }
    panic!("the map side never reached what this test waited for with its head at {hour}: {}", recorded::seen(side));
}

fn applied_to(side: &mut MapSide, hour: i32) {
    frames_at(side, hour, |side, _| side.applied() == Some(hour) && !side.composing());
}

fn settled(side: &mut MapSide, hour: i32) {
    frames_at(side, hour, |side, _| !side.composing());
    if !layer(side).drawn() {
        return;
    }
    for _ in 0..200 {
        if side.parts().is_none() {
            frames_at(side, hour, |side, _| !side.composing());
        }
        let at = side.parts().and_then(|(mirrors, _)| mirrors.get(TableId::TaskForces).and_then(|mirror| mirror.head())).unwrap_or(0);
        let _ = layer(side).rosters().wait(at, Duration::from_millis(200));
        side.settle_all().unwrap();
        let composed = layer(side).refused().is_none() || layer(side).refused().is_some_and(|why| why.contains("not wholly recorded"));
        if composed && layer(side).parts().values().all(Option::is_some) && !layer(side).rosters().pending() {
            return;
        }
        if !composed || layer(side).rosters().loading() {
            frames_at(side, hour, |side, frame| frame.steps == 0 && !side.composing());
        }
    }
    panic!("the rosters were never answered");
}

fn to(side: &mut MapSide, hour: i32) {
    applied_to(side, hour);
    settled(side, hour);
}

fn composed_equal(side: &MapSide) {
    let (mirrors, _) = side.parts().unwrap();
    let mut fresh = layer(side).fresh().unwrap();
    fresh.compose(mirrors).unwrap();
    assert_eq!(layer(side).differ(&fresh), (0, None));
}

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
    assert_eq!(side.applied(), Some(30), "the head is the hour the test drove it to");
    let before = layer(&side).marked();
    {
        let _settles = side.hold_settles();
        applied_to(&mut side, 64);
        assert_eq!(side.applied(), Some(64), "every step through hour 64, the sail at 50 among them, is applied inside this window");
        assert_eq!(layer(&side).marked()[0] - before[0], 1, "the task force that sailed is marked once");
        assert!(layer(&side).carried().entities >= 1, "nothing settles without settling");
    }
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
    applied_to(&mut side, 104);
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
    let kinds = replay_recording::schema::schema().unwrap();
    let mut far = Ships::new(fake()).unwrap();
    let mut inputs = viewer_layer_core::layers::Inputs::new(&kinds, "ships");
    far.inputs(&over(10, 600.0), &mut inputs).unwrap();
    assert!(inputs.into_tables().is_empty(), "above the cutoff the layer reads nothing");
    let mut side = side("ships-far");
    to(&mut side, 60);
    side.set_view(over(10, 600.0)).unwrap();
    applied_to(&mut side, 64);
    assert!(side.refusals().next().is_none());
    assert!(!layer(&side).drawn());
    assert_eq!(layer(&side).counts(), [0, 0], "it holds nothing");
    assert_eq!((layer(&side).carried().entities, layer(&side).published()), (0, Published::Current), "and carries and owes nothing");
    layer(&side).rosters().closed();
    assert!(
        !side.views().held().iter().any(|(table, _)| *table == TableId::ShipModels),
        "closed, the rosters' reader lets its tables go"
    );
    side.set_view(View::default()).unwrap();
    applied_to(&mut side, 72);
    settled(&mut side, 72);
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
    applied_to(&mut side, 132);
    let shared = &layer(&side).rosters().shared;
    let mut asked = shared.lock();
    while !(asked.known.contains(&204) && asked.wanted.is_empty()) {
        asked = shared.answered.wait(asked).unwrap_or_else(std::sync::PoisonError::into_inner);
    }
    drop(asked);
    assert!(!layer(&side).rosters().pending() && !layer(&side).rosters().loading());
    side.settle_all().unwrap();
    let parts = layer(&side).parts();
    assert!(parts.get(&204).is_some_and(Option::is_some), "the task force born at 130 has its figure: {:?}", parts.iter().map(|(id, part)| (*id, part.is_some())).collect::<Vec<_>>());
}

fn contested(at: u32, store: &mut Store) {
    campaign(at, store);
    let mixed: &[u64] = match at {
        60 => &[202, 999],
        120 => &[202],
        _ => return,
    };
    store.table(TableId::ProvinceUnitOrder.index()).unwrap().write(1, column(TableId::ProvinceUnitOrder, "mixed"), Cell::U64List(mixed)).unwrap();
}

#[test]
fn the_window_draws_the_layers_ships_and_todays_where_a_land_occupant_competes() {
    use viewer_unit_models::unit_models::{PreparedModels, naval_differ};
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
    applied_to(&mut side, 128);
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

struct HeldRosters {
    fake: Arc<dyn Naval>,
    permits: Mutex<usize>,
    builds: Mutex<(Vec<i32>, u64)>,
    changed: std::sync::Condvar,
    built: std::sync::Condvar,
}

const OPEN: usize = usize::MAX;

const GIVEN_UP: Duration = Duration::from_secs(30);

impl HeldRosters {
    fn new(fake: Arc<dyn Naval>) -> Arc<Self> {
        Arc::new(Self { fake, permits: Mutex::new(OPEN), builds: Mutex::new((Vec::new(), 0)), changed: std::sync::Condvar::new(), built: std::sync::Condvar::new() })
    }

    fn set(&self, open: bool) {
        *self.permits.lock().unwrap() = if open { OPEN } else { 0 };
        self.changed.notify_all();
    }

    fn let_through(&self, builds: usize) {
        let mut permits = self.permits.lock().unwrap();
        if *permits != OPEN {
            *permits += builds;
        }
        drop(permits);
        self.changed.notify_all();
    }

    fn builds_past(&self, seen: u64, left: bool) {
        let count = |builds: &(Vec<i32>, u64)| if left { builds.1 } else { builds.0.len() as u64 };
        self.waiting_for(&format!("more than {seen} builds to have {} the gate", if left { "left" } else { "come to" }), |builds| count(builds) > seen);
    }

    fn arrived_at(&self, hour: i32) {
        self.waiting_for(&format!("a build over the rows at hour {hour} to come to the gate"), |builds| builds.0.contains(&hour));
    }

    fn waiting_for(&self, what: &str, done: impl Fn(&(Vec<i32>, u64)) -> bool) {
        let until = Instant::now() + GIVEN_UP;
        let mut builds = self.builds.lock().unwrap();
        while !done(&builds) {
            let left = until.checked_duration_since(Instant::now()).unwrap_or_else(|| panic!("this test waited for {what}; the builds that came to the gate were over the rows at {:?} and {} left it", builds.0, builds.1));
            builds = self.built.wait_timeout(builds, left).unwrap().0;
        }
    }

    fn builds(&self) -> (u64, u64) {
        let builds = self.builds.lock().unwrap();
        (builds.0.len() as u64, builds.1)
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
        self.builds.lock().unwrap().0.push(frame.hour);
        self.built.notify_all();
        let until = Instant::now() + GIVEN_UP;
        let mut permits = self.permits.lock().unwrap();
        while *permits == 0 {
            let Some(left) = until.checked_duration_since(Instant::now()) else {
                return Err(io::Error::other(format!("the test left this build of {} rosters over the rows at hour {} at the gate", forces.len(), frame.hour)));
            };
            permits = self.changed.wait_timeout(permits, left).unwrap().0;
        }
        if *permits != OPEN {
            *permits -= 1;
        }
        drop(permits);
        let rosters = self.fake.rosters(frame, forces);
        self.builds.lock().unwrap().1 += 1;
        self.built.notify_all();
        rosters
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        self.fake.decide_the_same(frame, decided)
    }
}

fn roster_state(side: &MapSide) -> String {
    let layer = side.parts().map(|_| layer(side)).map(|layer| (layer.carried(), layer.waiting(), layer.rosters().pending(), layer.published()));
    format!("applied {:?}, composing {}, carried, waiting, asked and published {layer:?}", side.applied(), side.composing())
}

fn settled_at(side: &mut MapSide, hour: i32) {
    loop {
        let frame = side.frame(hour).unwrap();
        if side.applied() == Some(hour) && !side.composing() {
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

#[test]
fn a_roster_out_on_the_worker_is_not_carried_and_the_hour_the_parts_stand_for_is_shown() {
    let held = HeldRosters::new(fake());
    let mut side = side_of("ships-roster-out", View::default(), Arc::clone(&held) as Arc<dyn Naval>, campaign);
    let before = 96;
    let applied = side.applied().expect("composed");
    assert!(applied < before, "the side opened at hour {applied}, past {before}");
    for hour in (applied + 1..=before).filter(|hour| hour % 4 == 0) {
        settled_at(&mut side, hour);
    }
    assert_eq!((layer(&side).carried().entities, layer(&side).published()), (0, Published::Current));
    let (came, _) = held.builds();
    held.set(false);
    applied_to(&mut side, before + 8);
    held.builds_past(came, false);
    side.frame(before + 8).unwrap();
    assert!(layer(&side).waiting() > 0 && layer(&side).carried().entities == 0, "task force 202 waits for the roster the gate holds, and nothing is left to settle: {}", roster_state(&side));
    let Published::At(stands) = layer(&side).published() else { panic!("a roster out publishes the hour the parts stand for: {:?}", layer(&side).published()) };
    let answered = layer(&side).rosters().answered().expect("the worker has answered some rows");
    let owed = layer(&side).rosters().owed().into_iter().map(|(_, at)| at).min().expect("the gate holds a roster the worker owes");
    assert_eq!(stands, answered.min(owed - 1), "a roster out publishes the hour the parts stand for: the rows answered ({answered}), before the change it owes ({owed}), and never the hour of the last settle that owed nothing ({before})");
    assert_eq!(side.last_frame().layers[0].shown, Some(stands), "the host shows that hour");
    held.set(true);
    settled_at(&mut side, before + 8);
    assert_eq!(layer(&side).published(), Published::Current, "answered, current again");
    assert!(drawn(&side)[1].3[0] >= 2, "task force 202 takes its newest roster");
}

fn clocked(at: u32, store: &mut Store) {
    campaign(at, store);
    store.table(TableId::Clock.index()).unwrap().write(0, column(TableId::Clock, "hour"), Cell::I32(at as i32)).unwrap();
}

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

struct FromRows(Arc<Fake>, fn(u64, i32) -> bool);

impl Naval for FromRows {
    fn drawn_at(&self, height: f32) -> bool {
        self.0.drawn_at(height)
    }

    fn bounds(&self) -> &BTreeMap<u32, [f32; 4]> {
        self.0.bounds()
    }

    fn map(&self) -> (MapSize, MapDefines) {
        self.0.map()
    }

    fn region(&self, province: u32) -> Option<u32> {
        self.0.region(province)
    }

    fn figure(&self, force: &Force, route: Result<UnitRoute<'_>, &'static str>, link: &dyn Fn(u32) -> Option<u32>, vision: &Vision, skip: &dyn Fn(u32) -> Option<bool>, ordinal: Option<u32>, speed: Option<i32>, day: i32, roster: &mut dyn FnMut() -> Roster) -> ForcePart {
        self.0.figure(force, route, link, vision, skip, ordinal, speed, day, roster)
    }

    fn rosters(&self, frame: &DecodedFrame, forces: &[u64]) -> io::Result<(Vec<(u64, Roster)>, Decided)> {
        let (mut rosters, decided) = self.0.rosters(frame, forces)?;
        for (force, roster) in &mut rosters {
            if (self.1)(*force, frame.hour) {
                *roster = None;
            }
        }
        Ok((rosters, decided))
    }

    fn decide_the_same(&self, frame: &DecodedFrame, decided: &Decided) -> bool {
        self.0.decide_the_same(frame, decided)
    }
}

fn leaves(force: u64, hour: i32) -> bool {
    force == 202 && hour >= 150
}

fn sought(side: &mut MapSide, hour: i32) {
    sought_alone(side, hour);
    side.settle_all().unwrap();
}

fn sought_alone(side: &mut MapSide, hour: i32) {
    side.entry().playhead().seek(hour);
    while side.applied() != Some(hour) || side.composing() {
        side.frame(hour).unwrap();
        std::thread::yield_now();
    }
}

fn hulls(side: &MapSide, force: u64) -> Option<Vec<u64>> {
    drawn(side).into_iter().find(|(id, ..)| *id == force).map(|(.., hulls)| hulls)
}

#[test]
fn a_roster_asked_after_a_seek_back_is_answered_from_that_seeks_rows_never_the_one_before() {
    let mut side = side_of("ships-seek-back", View::default(), Arc::new(FromRows(fake(), leaves)), campaign);
    sought(&mut side, 160);
    settled_at(&mut side, 160);
    assert_eq!(hulls(&side, 202), Some(Vec::new()), "202 has no ship at 160");
    sought(&mut side, 140);
    let looked = layer(&side).rosters().looks();
    while layer(&side).rosters().looks() <= looked {
        std::thread::yield_now();
    }
    settled_at(&mut side, 140);
    assert!(hulls(&side, 202).is_some_and(|hulls| !hulls.is_empty()), "202 is drawn with its ship at 140: {:?}", hulls(&side, 202));
    composed_equal(&side);
}

#[test]
fn a_roster_build_across_a_seek_back_answers_nothing() {
    let held = HeldRosters::new(Arc::new(FromRows(fake(), leaves)));
    let mut side = side_of("ships-build-across-seek", View::default(), Arc::clone(&held) as Arc<dyn Naval>, campaign);
    held.set(false);
    let (_, left) = held.builds();
    sought_alone(&mut side, 160);
    held.arrived_at(160);
    sought_alone(&mut side, 140);
    held.let_through(1);
    held.builds_past(left, true);
    held.arrived_at(140);
    side.frame(140).unwrap();
    assert!(layer(&side).waiting() > 0 && hulls(&side, 202).is_none(), "202 still waits for its roster at 140: {:?}", hulls(&side, 202));
    held.set(true);
    settled_at(&mut side, 140);
    assert!(hulls(&side, 202).is_some_and(|hulls| !hulls.is_empty()), "202 is drawn with its ship at 140: {:?}", hulls(&side, 202));
    composed_equal(&side);
}

fn apart(at: u32, store: &mut Store) {
    for table in TABLES {
        store.status(table.index(), Status::Complete, true, Some(at as i32), true).unwrap();
    }
    match at {
        0 => campaign(0, store),
        100 => store.table(TableId::ShipModels.index()).unwrap().write(0, column(TableId::ShipModels, "variant"), Cell::U64(2)).unwrap(),
        101 => store.table(TableId::ShipModels.index()).unwrap().write(2, column(TableId::ShipModels, "variant"), Cell::U64(2)).unwrap(),
        _ => {}
    }
}

#[test]
fn the_hour_the_figures_stand_at_is_bounded_by_the_roster_the_worker_owes() {
    let held = HeldRosters::new(fake());
    let mut side = side_of("ships-owed", View::default(), Arc::clone(&held) as Arc<dyn Naval>, apart);
    settled_at(&mut side, 96);
    assert_eq!(layer(&side).published(), Published::Current, "everything drawn stands at the hour applied");
    let (came, left) = held.builds();
    held.set(false);
    applied_to(&mut side, 100);
    held.builds_past(came, false);
    assert!(layer(&side).waiting() > 0, "task force 201 waits for the roster the gate holds");
    applied_to(&mut side, 104);
    assert!(layer(&side).waiting() > 0, "a task force still waits for the roster the gate holds");
    held.let_through(1);
    held.builds_past(left, true);
    held.builds_past(came + 1, false);
    side.frame(104).unwrap();
    assert!(layer(&side).waiting() > 0, "task force 202 waits for the roster the gate holds");
    let Published::At(stands) = layer(&side).published() else {
        panic!("a task force waiting for its roster says the hour its figures stand at: {:?}", layer(&side).published())
    };
    assert_eq!(stands, 100, "it stands at the rows the build answered from, which is before the change at 101 it owes");
    held.set(true);
    settled_at(&mut side, 104);
    assert_eq!(layer(&side).published(), Published::Current, "answered, it stands at the hour applied again");
    composed_equal(&side);
}

