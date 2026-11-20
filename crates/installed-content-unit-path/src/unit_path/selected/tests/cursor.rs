use super::super::cursor::{
    CursorFacts, CursorRequest, FirstSelected, OrderCursor, TASK_FORCE_ENTRY_UNRECORDED,
};
use super::*;
use crate::unit_path::{Cost, Edge, NONE};

const NOT_RECORDED: &str = "not recorded";

/// Land 1 (the unit), land 2 (the target), lake 3, sea 4 beside 2, land 5 and 6.
fn map() -> MoveGraph {
    let mut graph = MoveGraph {
        index: vec![NONE, 0, 1, 2, 3, 4, 5],
        ids: vec![1, 2, 3, 4, 5, 6],
        kinds: vec![Some(1), Some(1), Some(2), Some(0), Some(1), Some(1)],
        ranges: vec![Some((0, 0)); 6],
        ..Default::default()
    };
    graph.ranges[1] = Some((0, 1));
    graph.edges.push(Edge {
        to: 3,
        cost: Cost::Moving,
        crossing: NONE,
        distance: Some(1),
    });
    graph
}

struct Facts {
    queued: Fact<bool>,
    routes: [Fact<Option<u32>>; 2],
    theatre: Fact<bool>,
    truce: Fact<bool>,
    owners: BTreeMap<u64, u32>,
    state: Vec<u32>,
    reveal_skip: bool,
}

impl Default for Facts {
    fn default() -> Self {
        Self {
            queued: Err(NOT_RECORDED.into()),
            routes: [Ok(None), Ok(None)],
            theatre: Err(NOT_RECORDED.into()),
            truce: Err(NOT_RECORDED.into()),
            owners: BTreeMap::new(),
            state: Vec::new(),
            reveal_skip: false,
        }
    }
}

impl CursorFacts for Facts {
    fn queued_actions(&self, _: &CommonUnit<'_>) -> Fact<bool> {
        self.queued.clone()
    }
    fn route_end(&self, _: &CommonUnit<'_>, alternate: bool) -> Fact<Option<u32>> {
        self.routes[usize::from(alternate)].clone()
    }
    fn theatre_holds_area(&self, _: u32, _: u32) -> Fact<bool> {
        self.theatre.clone()
    }
    fn truce(&self, _: Country, _: Country) -> Fact<bool> {
        self.truce.clone()
    }
    fn roster_owner(&self, handle: u64, _: u32) -> Fact<u32> {
        self.owners
            .get(&handle)
            .copied()
            .ok_or_else(|| "occupant missing".into())
    }
    fn state_provinces(&self, _: u32) -> Fact<Vec<u32>> {
        Ok(self.state.clone())
    }
    fn reveal_skip(&self, _: &CommonUnit<'_>) -> Fact<bool> {
        Ok(self.reveal_skip)
    }
}

fn areas(pairs: &[(u32, u32)]) -> BTreeMap<u32, Fact<u32>> {
    pairs
        .iter()
        .map(|&(province, area)| (province, Ok(area)))
        .collect()
}

fn bases(provinces: &[u32]) -> BTreeMap<u32, Fact<bool>> {
    provinces
        .iter()
        .map(|&province| (province, Ok(true)))
        .collect()
}

fn land(
    world: &World,
    facts: &Facts,
    viewer_raw: u32,
    army: &Army,
    class: CommonClass,
    target: u32,
    shift: bool,
) -> Fact<OrderCursor> {
    let graph = map();
    let mut selected = unit(army);
    selected.class = class;
    let request = CursorRequest {
        viewer_raw,
        first: FirstSelected::Unit(selected),
        target,
        shift,
    };
    Mover {
        graph: &graph,
        world,
    }
    .order_cursor(&request, facts)
}

fn division(world: &World, facts: &Facts, target: u32, shift: bool) -> Fact<OrderCursor> {
    land(world, facts, 1, &army(), CommonClass::Army, target, shift)
}

fn naval(world: &World, facts: &Facts, target: u32) -> Fact<OrderCursor> {
    let graph = map();
    let request = CursorRequest {
        viewer_raw: 1,
        first: FirstSelected::TaskForce,
        target,
        shift: false,
    };
    Mover {
        graph: &graph,
        world,
    }
    .order_cursor(&request, facts)
}

#[test]
fn a_task_force_reads_sea_occupants_and_snaps_land_to_the_states_first_naval_base() {
    let facts = Facts {
        owners: BTreeMap::from([(50, 1), (51, 2)]),
        ..Default::default()
    };
    let hostile = World {
        occupants: Ok(vec![50, 51]),
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        ..Default::default()
    };
    assert_eq!(naval(&hostile, &facts, 4), Ok(OrderCursor::AttackMove));
    let ours_at_war = World {
        occupants: Ok(vec![50, 51]),
        wars: BTreeMap::from([((1, 2), Ok(true))]),
        ..Default::default()
    };
    assert_eq!(
        naval(&ours_at_war, &facts, 4),
        Ok(OrderCursor::FriendlyMove),
        "the war is read from the occupant's owner to the viewer"
    );
    assert_eq!(
        naval(&World::default(), &facts, 4),
        Ok(OrderCursor::FriendlyMove)
    );

    assert_eq!(naval(&World::default(), &facts, 3), Ok(OrderCursor::NoMove));
    let impassable = World {
        impassable: vec![2],
        ..Default::default()
    };
    assert_eq!(naval(&impassable, &facts, 2), Ok(OrderCursor::NoMove));

    let state = Facts {
        state: vec![2, 5, 6],
        ..Default::default()
    };
    assert_eq!(
        naval(&World::default(), &state, 2),
        Ok(OrderCursor::NoMove),
        "no naval base in the state"
    );
    let first_base_ours = World {
        bases: bases(&[5, 6]),
        state_owners: BTreeMap::from([(5, Ok(1)), (6, Ok(2))]),
        ..Default::default()
    };
    assert_eq!(
        naval(&first_base_ours, &state, 2),
        Ok(OrderCursor::FriendlyMove)
    );
    let foreign_base = World {
        bases: bases(&[6]),
        state_owners: BTreeMap::from([(6, Ok(2))]),
        ..Default::default()
    };
    assert_eq!(
        naval(&foreign_base, &state, 2),
        Err(TASK_FORCE_ENTRY_UNRECORDED.into())
    );
    let own_base = World {
        bases: bases(&[2]),
        state_owners: BTreeMap::from([(2, Ok(1))]),
        ..Default::default()
    };
    assert_eq!(
        naval(&own_base, &Facts::default(), 2),
        Ok(OrderCursor::FriendlyMove)
    );
}

#[test]
fn from_the_sea_a_unit_shows_add_units_without_the_transport_fallback() {
    let at_sea = Army {
        location: 4,
        ..army()
    };
    let facts = Facts::default();
    let world = World {
        transport: Err("unreached transport".into()),
        ..Default::default()
    };
    assert_eq!(
        land(&world, &facts, 1, &at_sea, CommonClass::Army, 2, false),
        Ok(OrderCursor::AddUnits)
    );
    let refused = World {
        path: Ok(false),
        transport: Err("unreached transport".into()),
        ..Default::default()
    };
    assert_eq!(
        land(&refused, &facts, 1, &at_sea, CommonClass::Army, 2, false),
        Ok(OrderCursor::NoMove)
    );
    let enemy_controller = World {
        controllers: BTreeMap::from([(2, Ok(2))]),
        wars: BTreeMap::from([((1, 2), Ok(true))]),
        ..Default::default()
    };
    assert_eq!(
        land(
            &enemy_controller,
            &facts,
            1,
            &at_sea,
            CommonClass::Army,
            2,
            false
        ),
        Ok(OrderCursor::AddUnits)
    );
    let avoiding = Facts {
        reveal_skip: true,
        ..Default::default()
    };
    assert_eq!(
        land(
            &enemy_controller,
            &avoiding,
            1,
            &at_sea,
            CommonClass::Army,
            2,
            false
        ),
        Ok(OrderCursor::NoMove),
        "a reveal-skip template avoids the enemy controller"
    );
}

#[test]
fn over_our_own_ground_the_cursor_follows_areas_bases_and_shift() {
    let facts = Facts::default();
    let same_area = World {
        d0: areas(&[(1, 10), (2, 10)]),
        ..Default::default()
    };
    assert_eq!(
        division(&same_area, &facts, 2, false),
        Ok(OrderCursor::FriendlyMove),
        "answered without the unrecorded queued actions"
    );
    assert_eq!(
        division(&same_area, &facts, 2, true),
        Err(NOT_RECORDED.into())
    );
    let unqueued = Facts {
        queued: Ok(false),
        ..Default::default()
    };
    assert_eq!(
        division(&same_area, &unqueued, 2, true),
        Ok(OrderCursor::ChainMove)
    );

    assert_eq!(
        division(&World::default(), &facts, 3, false),
        Ok(OrderCursor::NoMove)
    );
    let impassable = World {
        impassable: vec![2],
        ..Default::default()
    };
    assert_eq!(
        division(&impassable, &facts, 2, false),
        Ok(OrderCursor::NoMove)
    );
    let unreachable = World {
        path: Ok(false),
        transport: Ok(false),
        ..Default::default()
    };
    assert_eq!(
        division(&unreachable, &facts, 2, false),
        Ok(OrderCursor::NoMove)
    );
    let retired = World {
        path: Ok(false),
        transport: Err("transport retired".into()),
        ..Default::default()
    };
    assert_eq!(
        division(&retired, &facts, 2, false),
        Err("transport retired".into())
    );

    let ports = World {
        d0: areas(&[(1, 10), (2, 20), (6, 20)]),
        bases: bases(&[1, 2]),
        ..Default::default()
    };
    assert_eq!(
        division(&ports, &facts, 2, false),
        Ok(OrderCursor::FriendlyMove)
    );
    assert_eq!(
        division(&ports, &unqueued, 2, true),
        Ok(OrderCursor::ChainMoveNaval)
    );
    let queued_to = |end: u32| Facts {
        queued: Ok(true),
        routes: [Ok(Some(1)), Ok(Some(end))],
        ..Default::default()
    };
    assert_eq!(
        division(&ports, &queued_to(6), 2, true),
        Ok(OrderCursor::ChainMove)
    );
    let far = World {
        d0: areas(&[(1, 10), (2, 20), (6, 30)]),
        bases: bases(&[1, 2, 6]),
        ..Default::default()
    };
    assert_eq!(
        division(&far, &queued_to(6), 2, true),
        Ok(OrderCursor::ChainMoveNaval)
    );
    let far_inland = World {
        d0: areas(&[(1, 10), (2, 20), (6, 30)]),
        bases: bases(&[1, 2]),
        ..Default::default()
    };
    assert_eq!(
        division(&far_inland, &queued_to(6), 2, true),
        Ok(OrderCursor::NoMove)
    );
    let queued_without_routes = Facts {
        queued: Ok(true),
        ..Default::default()
    };
    assert_eq!(
        division(&ports, &queued_without_routes, 2, true),
        Ok(OrderCursor::NoMove)
    );
    let ordinary_only = Facts {
        queued: Ok(true),
        routes: [Ok(Some(6)), Ok(None)],
        ..Default::default()
    };
    assert_eq!(
        division(&ports, &ordinary_only, 2, true),
        Ok(OrderCursor::ChainMove),
        "an empty alternate route falls back to the ordinary route's end"
    );
}

#[test]
fn another_land_area_of_ours_asks_the_units_theatres() {
    let world = World {
        d0: areas(&[(1, 10), (2, 20)]),
        ..Default::default()
    };
    let held = Facts {
        theatre: Ok(true),
        ..Default::default()
    };
    assert_eq!(
        division(&world, &held, 2, false),
        Ok(OrderCursor::FriendlyMove)
    );
    let missed = Facts {
        theatre: Ok(false),
        ..Default::default()
    };
    assert_eq!(division(&world, &missed, 2, false), Ok(OrderCursor::NoMove));
    assert_eq!(
        division(&world, &Facts::default(), 2, false),
        Err(NOT_RECORDED.into())
    );
    let exiled = World {
        d0: areas(&[(1, 10), (2, 20)]),
        exile_raw: Ok(1),
        ..Default::default()
    };
    assert_eq!(
        division(&exiled, &Facts::default(), 2, false),
        Ok(OrderCursor::FriendlyMove),
        "an exiled unit needs no theatre"
    );
    let shift_unqueued = Facts {
        queued: Ok(false),
        theatre: Ok(false),
        ..Default::default()
    };
    assert_eq!(
        division(&world, &shift_unqueued, 2, true),
        Ok(OrderCursor::NoMove),
        "an empty route to another area takes the theatre test"
    );
}

#[test]
fn foreign_ground_shows_attack_or_the_state_owners_access() {
    let viewer_at_war = World {
        controllers: BTreeMap::from([(2, Ok(2))]),
        wars: BTreeMap::from([((9, 2), Ok(true))]),
        ..Default::default()
    };
    let facts = Facts::default();
    assert_eq!(
        land(
            &viewer_at_war,
            &facts,
            9,
            &army(),
            CommonClass::Army,
            2,
            false
        ),
        Ok(OrderCursor::AttackMove),
        "the viewer's war decides, not the unit owner's"
    );
    let gun = Army {
        organisation: 10_000_000,
        ..army()
    };
    assert_eq!(
        land(
            &viewer_at_war,
            &facts,
            9,
            &gun,
            CommonClass::RailwayGun,
            2,
            false
        ),
        Ok(OrderCursor::FriendlyMove),
        "a railway gun skips the attack cursor"
    );

    let unowned = |access: bool| World {
        controllers: BTreeMap::from([(2, Ok(2))]),
        state_owners: BTreeMap::from([(2, Ok(0))]),
        access: Ok(access),
        ..Default::default()
    };
    assert_eq!(
        division(&unowned(true), &facts, 2, false),
        Ok(OrderCursor::FriendlyMove)
    );
    assert_eq!(
        division(&unowned(false), &facts, 2, false),
        Err(NOT_RECORDED.into())
    );
    let truce = Facts {
        truce: Ok(true),
        ..Default::default()
    };
    assert_eq!(
        division(&unowned(false), &truce, 2, false),
        Ok(OrderCursor::FriendlyMove)
    );
    let none = Facts {
        truce: Ok(false),
        ..Default::default()
    };
    assert_eq!(
        division(&unowned(false), &none, 2, false),
        Ok(OrderCursor::MoveToEnemyProvince)
    );
    let owned = World {
        controllers: BTreeMap::from([(2, Ok(2))]),
        state_owners: BTreeMap::from([(2, Ok(2))]),
        ..Default::default()
    };
    assert_eq!(
        division(&owned, &facts, 2, false),
        Ok(OrderCursor::FriendlyMove),
        "a positive state owner answers without the unrecorded truce"
    );
}

#[test]
fn shift_admits_a_retreating_unit_and_the_tooltip_always_does() {
    let retreating = Army {
        retreating: true,
        ..army()
    };
    let world = World {
        d0: areas(&[(1, 10), (2, 10)]),
        ..Default::default()
    };
    let facts = Facts {
        queued: Ok(false),
        ..Default::default()
    };
    assert_eq!(
        land(&world, &facts, 1, &retreating, CommonClass::Army, 2, false),
        Ok(OrderCursor::NoMove)
    );
    assert_eq!(
        land(&world, &facts, 1, &retreating, CommonClass::Army, 2, true),
        Ok(OrderCursor::ChainMove)
    );

    let graph = map();
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let options = MoveOptions {
        transport_fallback: true,
        retreat_bypass: false,
        avoid_enemy: false,
    };
    let mut text = String::new();
    assert_eq!(
        mover.selected_move(&unit(&retreating), 2, options, &mut text, &Text::default()),
        Ok(false)
    );
    assert_eq!(text, "[NO_MOVE_RETREAT]");
    let mut text = String::new();
    assert_eq!(
        mover.selected_move_message(&unit(&retreating), 2, &mut text, &Text::default()),
        Ok(true)
    );
    assert!(text.is_empty());
}
