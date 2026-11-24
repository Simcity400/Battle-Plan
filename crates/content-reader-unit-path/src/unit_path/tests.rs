
use super::*;
use crate::province_graph::Crossing as Authored;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default)]
struct World {
    cost_friendly: BTreeSet<(Country, Country)>,
    controllers: BTreeMap<u32, Country>,
    state_owners: BTreeMap<u32, Country>,
    wars: BTreeSet<(Country, Country)>,
    access: BTreeSet<(Country, Country)>,
    docking: BTreeSet<(Country, Country)>,
    factions: BTreeMap<Country, u64>,
    overlords: BTreeMap<Country, Country>,
    occupy_non_war: BTreeSet<Country>,
    enemies: BTreeMap<Country, Vec<Country>>,
    bases: BTreeMap<u32, bool>,
    ports: BTreeMap<u32, u32>,
    armies: BTreeMap<u32, Vec<Occupant>>,
    combats: BTreeSet<u32>,
    conflicts: BTreeMap<u32, Vec<u64>>,
    straits: BTreeMap<u32, StraitFleet>,
    regions: BTreeMap<u32, u32>,
    modes: BTreeMap<(Country, u32), u8>,
    unknown: BTreeSet<u32>,
    raw_positive: Option<BTreeSet<u32>>,
    raw_owner_positive: Option<BTreeSet<u32>>,
    rail_connections: BTreeSet<(u32, u32)>,
    railway_groups: BTreeMap<u64, u64>,
    task_force_access: BTreeSet<(u64, u32)>,
    submarines: BTreeSet<u64>,
    task_force_hostile_allows: BTreeSet<(u64, u32)>,
}

impl MoveWorld for World {
    fn path_friendly(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.cost_friendly.contains(&(a, b)) || self.at_war(a, b)?)
    }
    fn at_war(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(a != 0 && a != b && self.wars.contains(&(a.min(b), a.max(b))))
    }
    fn military_access(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.access.contains(&(a, b)))
    }
    fn docking_rights(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.docking.contains(&(a, b)))
    }
    fn subject_tie(&self, a: Country, b: Country) -> Fact<bool> {
        Ok(self.overlords.get(&a) == Some(&b) || self.overlords.get(&b) == Some(&a))
    }
    fn faction(&self, country: Country) -> Fact<Option<u64>> {
        Ok(self.factions.get(&country).copied())
    }
    fn overlord(&self, country: Country) -> Fact<Country> {
        Ok(self.overlords.get(&country).copied().unwrap_or(0))
    }
    fn can_occupy_non_war(&self, country: Country) -> Fact<bool> {
        Ok(self.occupy_non_war.contains(&country))
    }
    fn enemies(&self, country: Country) -> Fact<Vec<Country>> {
        Ok(self.enemies.get(&country).cloned().unwrap_or_default())
    }
    fn controller(&self, province: u32) -> Fact<Country> {
        if self.unknown.contains(&province) {
            return Err(format!("province {province} is not recorded"));
        }
        Ok(self.controllers.get(&province).copied().unwrap_or(0))
    }
    fn controller_raw_positive(&self, province: u32) -> Fact<bool> {
        if let Some(positive) = &self.raw_positive {
            return Ok(positive.contains(&province));
        }
        Ok(self.controller(province)? as i32 > 0)
    }
    fn state_owner(&self, province: u32) -> Fact<Country> {
        Ok(self.state_owners.get(&province).copied().unwrap_or(0))
    }
    fn state_owner_raw_positive(&self, province: u32) -> Fact<bool> {
        if let Some(positive) = &self.raw_owner_positive {
            return Ok(positive.contains(&province));
        }
        Ok(self.state_owner(province)? as i32 > 0)
    }
    fn demilitarized(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn impassable(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn owner_area_missing(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn border_conflict(&self, province: u32) -> Fact<bool> {
        Ok(self.conflicts.contains_key(&province))
    }
    fn border_conflict_admits(&self, province: u32, group: u64) -> Fact<bool> {
        Ok(group != 0
            && self
                .conflicts
                .get(&province)
                .is_some_and(|g| g.contains(&group)))
    }
    fn has_naval_base(&self, province: u32) -> Fact<bool> {
        Ok(self.bases.contains_key(&province))
    }
    fn naval_base_active(&self, province: u32) -> Fact<bool> {
        Ok(self.bases.get(&province).copied().unwrap_or(false))
    }
    fn counter_rail_connected(&self, from: u32, to: u32) -> Fact<bool> {
        Ok(self.rail_connections.contains(&(from, to)))
    }
    fn counter_railway_orders_group(&self, handle: u64) -> Fact<u64> {
        self.railway_groups
            .get(&handle)
            .copied()
            .ok_or_else(|| format!("railway gun {handle} orders group is unavailable"))
    }
    fn counter_task_force_access(&self, handle: u64, to: u32) -> Fact<bool> {
        Ok(self.task_force_access.contains(&(handle, to)))
    }
    fn counter_task_force_submarine(&self, handle: u64) -> Fact<bool> {
        Ok(self.submarines.contains(&handle))
    }
    fn counter_task_force_hostile_allows(&self, handle: u64, to: u32) -> Fact<bool> {
        Ok(self.task_force_hostile_allows.contains(&(handle, to)))
    }
    fn port_sea(&self, province: u32) -> Fact<u32> {
        Ok(self.ports.get(&province).copied().unwrap_or(0))
    }
    fn armies(&self, province: u32) -> Fact<Vec<Occupant>> {
        Ok(self.armies.get(&province).cloned().unwrap_or_default())
    }
    fn land_combat(&self, province: u32) -> Fact<bool> {
        Ok(self.combats.contains(&province))
    }
    fn strait_fleet(&self, through: u32, _: Country) -> Fact<StraitFleet> {
        Ok(self
            .straits
            .get(&through)
            .copied()
            .unwrap_or(StraitFleet::None))
    }
    fn strategic_region(&self, province: u32) -> Option<u32> {
        self.regions.get(&province).copied()
    }
    fn region_access(&self, country: Country, region: u32) -> Fact<u8> {
        Ok(self.modes.get(&(country, region)).copied().unwrap_or(0))
    }
    fn has_children(&self, triggers: &[Item]) -> Fact<bool> {
        Ok(!triggers.is_empty())
    }
    fn evaluate(&self, triggers: &[Item], _: Country) -> Fact<bool> {
        let mut result = true;
        for trigger in triggers {
            let name = String::from_utf8_lossy(trigger.key.as_deref().unwrap_or_default());
            match (name.as_ref(), trigger.value.scalar_str()) {
                ("always", Some(value)) => result &= value == "yes",
                (name, _) => return Err(format!("{name} is not in the test world")),
            }
        }
        Ok(result)
    }
}

const RULES: &[u8] = b"adjacency_rule = { name = OPEN is_disabled = { always = no } }
    adjacency_rule = { name = SHUT is_disabled = { always = yes } }
    adjacency_rule = { name = R required_provinces = { 2 } }
    adjacency_rule = { name = N required_provinces = { 2 } is_neutral = { always = no } }
    adjacency_rule = { name = C required_provinces = { 2 4 } contested = { army = no } }
    adjacency_rule = { name = BAD enemy = { tanks = no } }";

#[test]
fn invasion_costs_keep_the_non_admissible_heuristic_and_zero_slot() {
    let geometry = ProvinceGeometry {
        width: 100,
        centroids: BTreeMap::from([(1, [8, 0]), (2, [4, 0]), (3, [0, 0]), (4, [0, 0])]),
        ..Default::default()
    };
    let graph = ProvinceGraph {
        edges: BTreeMap::from([
            (1, vec![(2, Some(100_000)), (3, Some(150_000))]),
            (2, vec![(4, Some(100_000))]),
            (3, vec![(4, Some(250_000))]),
            (4, vec![]),
        ]),
        ..Default::default()
    };
    let graph = MoveGraph::new(&graph, &geometry);
    let mut world = World::default();
    let army = Army {
        owner: 1,
        ..Default::default()
    };
    let route = |world: &World| {
        Mover {
            graph: &graph,
            world,
        }
        .invasion_route(&army, Some(1), 1, 4)
    };
    assert_eq!(route(&world), Ok(vec![2, 4]));
    world.cost_friendly.insert((1, 0));
    assert_eq!(route(&world), Ok(vec![3, 4]));
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .invasion_route(&army, Some(1), 3, 3),
        Ok(vec![3])
    );
}

#[test]
fn invasion_unsupported_content_is_refused_before_destination_admission() {
    let (mut graph, mut geometry) = map();
    let mut world = world();
    world.unknown.insert(2);
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .invasion_route(&army(), None, 99, 2),
        Err("unit path source 99 is undefined".into())
    );
    geometry.centroids.insert(65_536, [0, 0]);
    graph.edges.insert(65_536, vec![]);
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .invasion_route(&army(), None, 1, 2),
        Err("unit path province id exceeds the u16 node representation".into())
    );
}

#[test]
fn invasion_forces_port_links_but_ignores_region_avoidance_and_base_level() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.insert(6, vec![(5, Some(50_000))]);
    graph.edges.get_mut(&5).unwrap().push((6, Some(50_000)));
    let mut world = world();
    world.ports.insert(1, 5);
    world.ports.insert(3, 5);
    world.regions.extend([(5, 1), (6, 2)]);
    world.modes.insert((1, 2), 2);
    let moves = MoveGraph::new(&graph, &geometry);
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    let army = army();
    assert_eq!(mover.invasion_sea(1), Ok(Some(5)));
    assert_eq!(mover.invasion_sea(4), Ok(None));
    assert_eq!(mover.invasion_route(&army, Some(1), 5, 6), Ok(vec![6]));
    graph.edges.get_mut(&5).unwrap().last_mut().unwrap().1 = Some(0);
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .invasion_route(&army, Some(1), 5, 6),
        Ok(vec![])
    );
    graph.edges.get_mut(&5).unwrap().last_mut().unwrap().1 = Some(-1);
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .invasion_route(&army, Some(1), 5, 6),
        Ok(vec![6])
    );
}

#[test]
fn invasion_sea_fallback_keeps_neighbour_order_and_named_unknowns() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.get_mut(&1).unwrap().insert(0, (6, Some(10)));
    graph.edges.insert(6, vec![(5, None)]);
    let world = world();
    let moves = MoveGraph::new(&graph, &geometry);
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert_eq!(mover.invasion_sea(1), Ok(Some(6)));
    assert!(
        mover
            .invasion_route(&army(), Some(1), 6, 5)
            .unwrap_err()
            .contains("6->5 distance")
    );
    let mut world = world;
    world.ports.insert(1, 5);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .invasion_sea(1),
        Ok(Some(5))
    );
}

#[test]
fn invasion_search_obeys_rules_visible_strait_fleets_and_request_country_stop() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.insert(5, vec![(6, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    let mut world = world();
    let search = |graph: &ProvinceGraph, world: &World| {
        Mover {
            graph: &MoveGraph::new(graph, &geometry),
            world,
        }
        .invasion_route(&army(), Some(1), 5, 6)
    };
    for (rule, expected) in [("OPEN", vec![6]), ("SHUT", vec![])] {
        cross(&mut graph, 5, 6, ruled(rule));
        assert_eq!(search(&graph, &world), Ok(expected));
    }
    cross(&mut graph, 5, 6, ruled("BAD"));
    assert!(search(&graph, &world).unwrap_err().contains("BAD"));
    cross(
        &mut graph,
        5,
        6,
        Authored {
            kind: SEA_CROSSING,
            through: 5,
            rule: None,
        },
    );
    world.straits.insert(5, StraitFleet::Visible);
    assert_eq!(search(&graph, &world), Ok(vec![]));
    world.straits.insert(5, StraitFleet::Unseen);
    assert_eq!(search(&graph, &world), Ok(vec![6]));
    world.state_owners.insert(5, 1);
    assert_eq!(search(&graph, &world), Ok(vec![]));
    assert_eq!(
        Mover {
            graph: &MoveGraph::new(&graph, &geometry),
            world: &world,
        }
        .invasion_route(&army(), None, 5, 6),
        Ok(vec![6])
    );
}

fn map() -> (ProvinceGraph, ProvinceGeometry) {
    let mut graph = ProvinceGraph {
        rules: crate::adjacency_rules::parse(RULES).unwrap(),
        ..ProvinceGraph::default()
    };
    let edges: [(u32, &[u32]); 5] = [
        (1, &[2, 5]),
        (2, &[1, 3]),
        (3, &[2, 4, 5]),
        (4, &[3]),
        (5, &[1, 3]),
    ];
    for (id, next) in edges {
        graph
            .edges
            .insert(id, next.iter().map(|&to| (to, Some(50_000))).collect());
    }
    let mut geometry = ProvinceGeometry {
        width: 1000,
        height: 10,
        ..Default::default()
    };
    for id in 1..=5 {
        geometry.centroids.insert(id, [id as i32 * 10, 5]);
    }
    geometry.land.extend([1, 2, 3, 4]);
    (graph, geometry)
}

fn world() -> World {
    let mut world = World::default();
    for id in 1..=4 {
        world.controllers.insert(id, 1);
        world.state_owners.insert(id, 1);
    }
    world
}

fn army() -> Army {
    Army {
        location: 1,
        owner: 1,
        navy_country: 1,
        strength: 100_000,
        organisation: 500_000,
        ..Army::default()
    }
}

#[test]
fn counter_route_excludes_source_and_retains_equal_endpoint_success() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let world = world();
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert_eq!(
        mover.counter_route(&army(), 1, 4, CounterRequest::default()),
        Ok(vec![2, 3, 4])
    );
    assert_eq!(
        mover.native_route(&army(), 1, 1, None, |_, _| Ok(true)),
        Ok(Some(vec![]))
    );
    assert_eq!(
        mover.counter_route(&army(), 1, 1, CounterRequest::default()),
        Ok(vec![])
    );
}

#[test]
fn local_counter_mode_falls_back_from_rail_admission_but_strategic_does_not() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.controllers.insert(2, 2);
    world.wars.insert((1, 2));
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert_eq!(
        mover.counter_route(
            &army(),
            1,
            2,
            CounterRequest {
                local_option: true,
                strategic: false,
                ..Default::default()
            }
        ),
        Ok(vec![2])
    );
    assert_eq!(
        mover.counter_route(
            &army(),
            1,
            2,
            CounterRequest {
                local_option: false,
                strategic: true,
                ..Default::default()
            }
        ),
        Ok(vec![])
    );
}

#[test]
fn forced_counter_route_sea_postfilter_uses_base_presence_not_active_level() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    geometry.land.insert(6);
    graph.edges.get_mut(&5).unwrap().push((6, Some(50_000)));
    graph.edges.insert(6, vec![(5, Some(50_000))]);
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.controllers.insert(6, 1);
    world.state_owners.insert(6, 1);
    world.ports.extend([(1, 5), (6, 5)]);
    let request = CounterRequest {
        local_option: true,
        strategic: false,
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&army(), 1, 6, CounterRequest::default()),
        Ok(vec![5, 6])
    );
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&army(), 1, 6, request),
        Ok(vec![])
    );
    world.bases.extend([(1, false), (6, false)]);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&army(), 1, 6, request),
        Ok(vec![5, 6])
    );
}

#[test]
fn exiled_counter_landing_skips_access_and_visible_fleet_operands() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    geometry.land.insert(6);
    graph.edges.insert(5, vec![(6, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    cross(
        &mut graph,
        5,
        6,
        Authored {
            kind: SEA_CROSSING,
            through: 5,
            rule: None,
        },
    );
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.unknown.insert(6);
    world.straits.insert(5, StraitFleet::Visible);
    let army = Army {
        location: 5,
        exiled: true,
        ..army()
    };
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&army, 5, 6, CounterRequest::default()),
        Ok(vec![6])
    );
}

#[test]
fn land_mass_ignores_cost_and_rules_but_excludes_kind_four() {
    let (mut graph, geometry) = map();
    graph.edges.get_mut(&2).unwrap()[1].1 = None;
    graph.edges.get_mut(&3).unwrap()[0].1 = Some(0);
    cross(&mut graph, 2, 3, ruled("SHUT"));
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(moves.land_mass(1), moves.land_mass(4));
    cross(
        &mut graph,
        2,
        3,
        Authored {
            kind: IMPASSABLE,
            through: -1,
            rule: None,
        },
    );
    let moves = MoveGraph::new(&graph, &geometry);
    assert_ne!(moves.land_mass(1).unwrap(), moves.land_mass(4).unwrap());
}

#[test]
fn raw_positive_controller_linked_to_zero_still_reaches_war_gate() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.controllers.extend([(1, 2), (2, 0)]);
    world.wars.insert((0, 1));
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .crosses_other_war(1, 1, 2),
        Ok(false)
    );
    world.raw_positive = Some(BTreeSet::from([1, 2]));
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .crosses_other_war(1, 1, 2),
        Ok(true)
    );
}

#[test]
fn positive_raw_state_owner_linked_to_zero_does_not_exempt_foreign_access() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.state_owners.insert(2, 0);
    world.controllers.insert(2, 3);
    world.raw_owner_positive = Some(BTreeSet::from([2]));
    let access = |world: &World| {
        let mover = Mover {
            graph: &moves,
            world,
        };
        (
            mover.has_province_access(1, 2),
            mover.move_access(1, 2),
            mover.naval_move_access(1, 2),
        )
    };
    assert_eq!(access(&world), (Ok(false), Ok(false), Ok(false)));
    world.docking.insert((1, 3));
    assert_eq!(access(&world), (Ok(false), Ok(false), Ok(true)));
    world.docking.clear();
    world.raw_owner_positive = Some(BTreeSet::new());
    world.unknown.insert(2);
    assert_eq!(access(&world), (Ok(true), Ok(true), Ok(true)));
}

#[test]
fn railway_counter_requires_directed_connections_but_transfer_uses_port_links() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    let request = CounterRequest {
        class: CounterClass::Railway(77),
        ..Default::default()
    };
    let route = |world: &World, unit: &Army, from, to| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(unit, from, to, request)
    };
    assert_eq!(route(&world, &army(), 1, 4), Ok(vec![]));
    world.rail_connections.extend([(1, 2), (2, 3), (3, 4)]);
    assert_eq!(route(&world, &army(), 1, 4), Ok(vec![2, 3, 4]));
    let reverse = Army {
        location: 4,
        ..army()
    };
    assert_eq!(route(&world, &reverse, 4, 1), Ok(vec![]));
    world.rail_connections.clear();
    world.ports.insert(1, 5);
    assert_eq!(route(&world, &army(), 1, 5), Ok(vec![5]));
}

#[test]
fn railway_counter_reads_actual_group_only_at_the_border_conflict_gate() {
    let (graph, geometry) = map();
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.rail_connections.insert((1, 2));
    let request = CounterRequest {
        class: CounterClass::Railway(77),
        ..Default::default()
    };
    let route = |world: &World| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(&army(), 1, 2, request)
    };
    assert_eq!(route(&world), Ok(vec![2]));
    world.conflicts.insert(2, vec![8]);
    assert_eq!(
        route(&world),
        Err("railway gun 77 orders group is unavailable".into())
    );
    world.railway_groups.insert(77, 0);
    assert_eq!(route(&world), Ok(vec![]));
    world.railway_groups.insert(77, 8);
    assert_eq!(route(&world), Ok(vec![2]));
    world.railway_groups.clear();
    world.conflicts.insert(1, vec![]);
    assert_eq!(route(&world), Ok(vec![2]));
}

#[test]
fn railway_mode_two_skips_army_region_policy_but_keeps_war_and_combat_gates() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.insert(5, vec![(6, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.regions.extend([(5, 1), (6, 2)]);
    world.modes.insert((1, 2), 2);
    let unit = Army {
        location: 5,
        naval_transfer: true,
        ..army()
    };
    let route = |world: &World, class| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(
            &unit,
            5,
            6,
            CounterRequest {
                class,
                strategic: true,
                local_option: false,
            },
        )
    };
    assert_eq!(route(&world, CounterClass::Army), Ok(vec![]));
    assert_eq!(route(&world, CounterClass::Railway(77)), Ok(vec![6]));
    world.combats.insert(6);
    assert_eq!(route(&world, CounterClass::Railway(77)), Ok(vec![]));
    world.combats.clear();
    world.controllers.insert(6, 2);
    world.wars.insert((1, 2));
    assert_eq!(route(&world, CounterClass::Railway(77)), Ok(vec![]));
}

#[test]
fn task_force_counter_uses_naval_output_and_directional_docking_rights() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph
        .edges
        .insert(5, vec![(6, Some(50_000)), (3, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.controllers.extend([(3, 3), (6, 3)]);
    world.state_owners.extend([(3, 3), (6, 3)]);
    world.ports.insert(3, 5);
    world.task_force_access.extend([(77, 3), (77, 6)]);
    let unit = Army {
        location: 5,
        ..army()
    };
    let request = CounterRequest {
        class: CounterClass::TaskForce(77),
        ..Default::default()
    };
    let route = |world: &World, to| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(&unit, 5, to, request)
    };
    assert_eq!(route(&world, 6), Ok(vec![6]));
    assert_eq!(route(&world, 3), Ok(vec![]));
    world.occupy_non_war.insert(1);
    world.enemies.extend([(1, vec![9]), (3, vec![9])]);
    world.docking.insert((3, 1));
    assert_eq!(route(&world, 3), Ok(vec![]));
    world.docking.insert((1, 3));
    assert_eq!(route(&world, 3), Ok(vec![3]));
    geometry.lake.insert(6);
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&unit, 5, 6, request),
        Ok(vec![])
    );
}

#[test]
fn task_force_land_mass_exclusion_survives_forced_and_active_transfer() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    geometry.land.insert(6);
    graph.edges.insert(6, vec![(5, Some(50_000))]);
    graph.edges.get_mut(&5).unwrap().push((6, Some(50_000)));
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.controllers.insert(6, 1);
    world.state_owners.insert(6, 1);
    world.ports.extend([(1, 5), (6, 5)]);
    world.task_force_access.extend([(77, 5), (77, 6)]);
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert_eq!(
        mover.counter_route(&army(), 1, 6, CounterRequest::default()),
        Ok(vec![5, 6])
    );
    for naval_transfer in [false, true] {
        let unit = Army {
            naval_transfer,
            ..army()
        };
        assert_eq!(
            mover.counter_route(
                &unit,
                1,
                6,
                CounterRequest {
                    class: CounterClass::TaskForce(77),
                    ..Default::default()
                }
            ),
            Ok(vec![])
        );
    }
    world.task_force_access.insert((77, 2));
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(
            &army(),
            1,
            2,
            CounterRequest {
                class: CounterClass::TaskForce(77),
                ..Default::default()
            }
        ),
        Ok(vec![])
    );
}

#[test]
fn task_force_counter_rule_selects_submarines_and_exile_keeps_class_gates() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.insert(5, vec![(6, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    graph.rules.extend(
        crate::adjacency_rules::parse(
            b"adjacency_rule = {
        name = SUB_ONLY is_friend = { always = yes }
        friend = { army = no navy = no submarine = yes }
    }",
        )
        .unwrap(),
    );
    cross(&mut graph, 5, 6, ruled("SUB_ONLY"));
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.task_force_access.insert((77, 6));
    let unit = Army {
        location: 5,
        ..army()
    };
    let request = CounterRequest {
        class: CounterClass::TaskForce(77),
        ..Default::default()
    };
    let route = |world: &World| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(&unit, 5, 6, request)
    };
    assert_eq!(route(&world), Ok(vec![]));
    world.submarines.insert(77);
    assert_eq!(route(&world), Ok(vec![6]));
    cross(
        &mut graph,
        5,
        6,
        Authored {
            kind: SEA_CROSSING,
            through: 5,
            rule: None,
        },
    );
    let moves = MoveGraph::new(&graph, &geometry);
    world.straits.insert(5, StraitFleet::Visible);
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&unit, 5, 6, request),
        Ok(vec![])
    );
    world.task_force_access.clear();
    let exiled = Army {
        exiled: true,
        ..unit
    };
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&exiled, 5, 6, request),
        Ok(vec![6])
    );
    world.ports.insert(1, 5);
    let exiled = Army {
        exiled: true,
        ..army()
    };
    assert_eq!(
        Mover {
            graph: &moves,
            world: &world
        }
        .counter_route(&exiled, 1, 5, request),
        Ok(vec![])
    );
}

#[test]
fn task_force_mode_two_uses_hostile_gate_and_preserves_local_fallback() {
    let (mut graph, mut geometry) = map();
    geometry.centroids.insert(6, [60, 5]);
    graph.edges.insert(5, vec![(6, Some(50_000))]);
    graph.edges.insert(6, vec![]);
    let moves = MoveGraph::new(&graph, &geometry);
    let mut world = world();
    world.task_force_access.insert((77, 6));
    world.controllers.insert(6, 3);
    world.state_owners.insert(6, 3);
    world.wars.insert((1, 3));
    world.combats.insert(6);
    world.regions.extend([(5, 1), (6, 2)]);
    world.modes.insert((1, 2), 2);
    let unit = Army {
        location: 5,
        ..army()
    };
    let route = |world: &World, strategic| {
        Mover {
            graph: &moves,
            world,
        }
        .counter_route(
            &unit,
            5,
            6,
            CounterRequest {
                class: CounterClass::TaskForce(77),
                strategic,
                local_option: !strategic,
            },
        )
    };
    assert_eq!(route(&world, true), Ok(vec![]));
    assert_eq!(route(&world, false), Ok(vec![6]));
    world.task_force_hostile_allows.insert((77, 6));
    assert_eq!(route(&world, true), Ok(vec![6]));
    world.task_force_access.clear();
    assert_eq!(route(&world, true), Ok(vec![]));
}

fn reaches(
    graph: &ProvinceGraph,
    geometry: &ProvinceGeometry,
    world: &World,
    army: &Army,
    to: u32,
) -> Fact<bool> {
    let moves = MoveGraph::new(graph, geometry);
    let mover = Mover {
        graph: &moves,
        world,
    };
    let single = mover.army_reaches(army, to, true);
    let full = mover.reach(army);
    let shared = mover.army_reaches_in(army, to, true, || &full);
    assert_eq!(single, shared, "early stop and full reach agree");
    let reachable = mover.reachable(army.exfiltrates);
    for (order, ahead) in [false, true]
        .into_iter()
        .flat_map(|order| [0, 1, 2, 3].map(|ahead| (order, ahead)))
    {
        let mut search = mover.search(army);
        let mut provinces: Vec<u32> = moves.ids.clone();
        if order {
            provinces.reverse();
        }
        for province in provinces {
            if ahead > 0 {
                mover.advance_by(army, &mut search, ahead);
            }
            assert_eq!(
                mover.army_reaches_resumed(army, province, true, &mut search, || &reachable),
                mover.army_reaches_in(army, province, true, || &full),
                "province {province} resumed, {ahead} edges ahead"
            );
        }
        if ahead > 0 {
            while !mover.advance_by(army, &mut search, ahead) {}
            assert_eq!(
                format!("{:?}", search.reach),
                format!("{:?}", full),
                "run ahead {ahead} edges at a time to its end"
            );
        }
    }
    single
}

fn cross(graph: &mut ProvinceGraph, a: u32, b: u32, crossing: Authored) {
    graph.crossings.insert((a, b), crossing.clone());
    graph.crossings.insert((b, a), crossing);
}

fn ruled(name: &str) -> Authored {
    Authored {
        kind: 0,
        through: -1,
        rule: Some(name.into()),
    }
}

#[test]
fn an_open_province_behind_an_unknown_fact_does_not_end_the_search_early() {
    let (mut graph, mut geometry) = map();
    graph
        .edges
        .insert(1, vec![(3, Some(50_000)), (6, Some(50_000))]);
    geometry.centroids.insert(6, [60, 5]);
    geometry.land.insert(6);
    let mut world = world();
    world.controllers.insert(6, 1);
    world.state_owners.insert(6, 1);
    world.unknown.extend([3, 6]);
    let answer = reaches(&graph, &geometry, &world, &army(), 4);
    assert_eq!(answer, Err("province 3 is not recorded".into()));
}

#[test]
fn an_island_without_ports_is_apart_and_answers_no() {
    let (mut graph, mut geometry) = map();
    graph.edges.insert(
        5,
        vec![(1, Some(50_000)), (3, Some(50_000)), (6, Some(50_000))],
    );
    graph.edges.insert(6, vec![(5, Some(50_000))]);
    geometry.centroids.insert(6, [60, 5]);
    geometry.land.insert(6);
    let mut world = world();
    world.controllers.insert(6, 1);
    world.state_owners.insert(6, 1);
    let moves = MoveGraph::new(&graph, &geometry);
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert!(mover.reachable(false).apart(&moves, 1, 6));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 6), Ok(false));
    world.ports.extend([(1, 5), (6, 5)]);
    world.bases.extend([(1, true), (6, true)]);
    let mover = Mover {
        graph: &moves,
        world: &world,
    };
    assert!(!mover.reachable(false).apart(&moves, 1, 6));
    reaches(&graph, &geometry, &world, &army(), 6).ok();
}

#[test]
fn an_army_walks_own_land_and_stops_at_enemy_or_closed_land() {
    let (graph, geometry) = map();
    let mut world = world();
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(true));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 1), Ok(true));
    world.controllers.insert(2, 2);
    world.wars.insert((1, 2));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
    world.wars.clear();
    world.controllers.insert(2, 3);
    world.state_owners.insert(2, 3);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
    world.access.insert((1, 3));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(true));
}

#[test]
fn embarking_needs_an_active_port_unless_the_army_exfiltrates() {
    let (mut graph, geometry) = map();
    graph.edges.insert(2, vec![]);
    graph.edges.insert(1, vec![(5, Some(50_000))]);
    graph
        .edges
        .insert(3, vec![(5, Some(50_000)), (4, Some(50_000))]);
    let mut world = world();
    world.ports.extend([(1, 5), (3, 5)]);
    world.bases.insert(3, true);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
    let marines = Army {
        exfiltrates: true,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &marines, 3), Ok(true));
    world.bases.insert(3, false);
    assert_eq!(reaches(&graph, &geometry, &world, &marines, 3), Ok(false));
}

#[test]
fn a_disabled_or_refused_adjacency_rule_closes_or_withholds_the_crossing() {
    let (mut graph, geometry) = map();
    let world = world();
    for (rule, expected) in [
        ("SHUT", Ok(false)),
        ("OPEN", Ok(true)),
        ("BAD", Err("adjacency rule BAD is refused".to_owned())),
    ] {
        cross(&mut graph, 1, 2, ruled(rule));
        let answer = reaches(&graph, &geometry, &world, &army(), 3)
            .map_err(|reason| reason[..reason.find(':').unwrap_or(reason.len())].to_owned());
        assert_eq!(answer, expected, "{rule}");
    }
}

#[test]
fn admission_refuses_retreat_transfer_and_an_enemy_the_army_cannot_attack() {
    let (graph, geometry) = map();
    let mut world = world();
    let retreating = Army {
        retreating: true,
        ..army()
    };
    assert_eq!(
        reaches(&graph, &geometry, &world, &retreating, 3),
        Ok(false)
    );
    let shipping = Army {
        naval_transfer: true,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &shipping, 3), Ok(false));
    world.wars.insert((1, 2));
    world.armies.insert(
        3,
        vec![Occupant {
            owner: 2,
            retreating: false,
            strength: 50_000,
        }],
    );
    let weak = Army {
        organisation: 50_000,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &weak, 3), Ok(false));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(true));
}

#[test]
fn a_strait_is_closed_only_by_a_visible_enemy_fleet_in_a_sea_through() {
    let (mut graph, mut geometry) = map();
    graph.edges.insert(1, vec![(2, Some(50_000))]);
    graph.edges.insert(2, vec![(1, Some(50_000))]);
    let strait = Authored {
        kind: SEA_CROSSING,
        through: 5,
        rule: None,
    };
    cross(&mut graph, 1, 2, strait);
    let mut world = world();
    for (fleet, expected) in [
        (StraitFleet::Visible, false),
        (StraitFleet::Unseen, true),
        (StraitFleet::None, true),
    ] {
        world.straits.insert(5, fleet);
        assert_eq!(
            reaches(&graph, &geometry, &world, &army(), 2),
            Ok(expected),
            "{fleet:?}"
        );
    }
    world.straits.insert(5, StraitFleet::Visible);
    geometry.land.insert(5);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 2), Ok(true));
    geometry.centroids.remove(&5);
    assert_eq!(
        reaches(&graph, &geometry, &world, &army(), 2),
        Err("province 5 is not defined".into())
    );
}

#[test]
fn an_avoided_or_blocked_sea_region_closes_to_the_search() {
    let (mut graph, geometry) = map();
    graph.edges.insert(1, vec![(5, Some(50_000))]);
    graph.edges.insert(2, vec![]);
    let mut world = world();
    world.ports.extend([(1, 5), (3, 5)]);
    world.bases.extend([(1, true), (3, true)]);
    world.regions.extend([(1, 7), (5, 8)]);
    let navy = Army {
        navy_country: 9,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &navy, 3), Ok(true));
    for (mode, expected) in [(1, false), (2, false), (3, true)] {
        world.modes.insert((9, 8), mode);
        assert_eq!(
            reaches(&graph, &geometry, &world, &navy, 3),
            Ok(expected),
            "mode {mode}"
        );
    }
    world.modes.insert((9, 8), 2);
    world.regions.insert(1, 8);
    assert_eq!(reaches(&graph, &geometry, &world, &navy, 3), Ok(true));
}

#[test]
fn an_exiled_army_moves_without_access_through_its_state_owner() {
    let (graph, geometry) = map();
    let mut world = world();
    for id in 2..=3 {
        world.controllers.insert(id, 3);
        world.state_owners.insert(id, 3);
    }
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
    let exiled = Army {
        exiled: true,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &exiled, 3), Ok(false));
    world.state_owners.insert(1, 3);
    assert_eq!(reaches(&graph, &geometry, &world, &exiled, 3), Ok(true));
}

#[test]
fn a_border_conflict_admits_only_its_orders_groups() {
    let (graph, geometry) = map();
    let mut world = world();
    world.conflicts.insert(2, vec![40]);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
    let grouped = Army {
        orders_group: 40,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &grouped, 3), Ok(true));
    world.conflicts.insert(1, vec![]);
    world.conflicts.insert(3, vec![]);
    let inside = Army {
        orders_group: 40,
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &inside, 2), Ok(true));
    assert_eq!(reaches(&graph, &geometry, &world, &inside, 3), Ok(false));
}

#[test]
fn an_army_in_combat_disengages_and_retreats_by_the_rules() {
    let (graph, geometry) = map();
    let mut world = world();
    let combat = ArmyCombat {
        province: 1,
        land: true,
        attacking: true,
        minimum_duration: false,
        ..ArmyCombat::default()
    };
    let fighting = Army {
        combats: vec![combat.clone()],
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &fighting, 2), Ok(false));
    let long = Army {
        combats: vec![ArmyCombat {
            minimum_duration: true,
            ..combat.clone()
        }],
        ..army()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &long, 2), Ok(true));
    let held = Army {
        no_retreat_attacking: true,
        ..long.clone()
    };
    assert_eq!(reaches(&graph, &geometry, &world, &held, 2), Ok(false));
    world.wars.insert((1, 2));
    world.armies.insert(
        2,
        vec![Occupant {
            owner: 2,
            retreating: true,
            strength: 100_000,
        }],
    );
    assert_eq!(reaches(&graph, &geometry, &world, &long, 2), Ok(false));
    world.armies.get_mut(&2).unwrap()[0].strength = 99_999;
    assert_eq!(reaches(&graph, &geometry, &world, &long, 2), Ok(true));
    let defending = Army {
        combats: vec![ArmyCombat {
            attacking: false,
            defending: true,
            attacker_locations: vec![2],
            ..combat
        }],
        ..long
    };
    assert_eq!(reaches(&graph, &geometry, &world, &defending, 2), Ok(false));
}

#[test]
fn an_enemy_occupied_neutral_state_needs_disclosure_access_or_a_shared_enemy() {
    let (graph, geometry) = map();
    let mut world = world();
    world.controllers.insert(2, 2);
    world.state_owners.insert(2, 3);
    world.wars.insert((1, 2));
    world.factions.extend([(1, 7), (3, 8)]);
    let admits = |world: &World| {
        let moves = MoveGraph::new(&graph, &geometry);
        Mover {
            graph: &moves,
            world,
        }
        .check_move_admission(&army(), 2, false)
    };
    assert_eq!(admits(&world), Ok(false));
    world.occupy_non_war.insert(1);
    world.enemies.insert(1, vec![5]);
    world.enemies.insert(3, vec![5]);
    assert_eq!(admits(&world), Ok(true));
    world.enemies.insert(3, vec![6]);
    assert_eq!(admits(&world), Ok(false));
    world.access.insert((1, 3));
    assert_eq!(admits(&world), Ok(true));
    world.access.clear();
    world.factions.insert(3, 7);
    assert_eq!(admits(&world), Ok(true));
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 2), Ok(false));
}

fn verdict(
    graph: &ProvinceGraph,
    world: &World,
    rule: &str,
    country: Country,
) -> Fact<Option<Verdict>> {
    let geometry = map().1;
    let moves = MoveGraph::new(graph, &geometry);
    let rule = crate::adjacency_rules::find(moves.rules(), rule).unwrap();
    Mover {
        graph: &moves,
        world,
    }
    .verdict(rule, country)
}

fn relation(graph: &ProvinceGraph, world: &World, rule: &str, country: Country) -> Fact<RuleReason> {
    let geometry = map().1;
    let moves = MoveGraph::new(graph, &geometry);
    let rule = crate::adjacency_rules::find(moves.rules(), rule).unwrap();
    Mover {
        graph: &moves,
        world,
    }
    .relation(rule, country)
}

#[test]
fn the_fallback_verdict_reads_war_and_disclosure() {
    let (graph, _) = map();
    let mut world = world();
    assert_eq!(verdict(&graph, &world, "R", 1), Ok(Some(Verdict::Friend)));
    assert_eq!(verdict(&graph, &world, "R", 3), Ok(Some(Verdict::Neutral)));
    assert_eq!(verdict(&graph, &world, "N", 3), Ok(Some(Verdict::Enemy)));
    world.wars.insert((1, 3));
    assert_eq!(verdict(&graph, &world, "R", 3), Ok(Some(Verdict::Enemy)));
    assert_eq!(relation(&graph, &world, "R", 3), Ok(RuleReason::Enemy(1)));
    assert_eq!(verdict(&graph, &world, "SHUT", 3), Ok(None));
    assert_eq!(relation(&graph, &world, "SHUT", 3), Ok(RuleReason::Disabled));
}

#[test]
fn required_provinces_at_war_contest_the_rule() {
    let (mut graph, geometry) = map();
    let mut world = world();
    world.controllers.insert(4, 2);
    assert_eq!(verdict(&graph, &world, "C", 1), Ok(Some(Verdict::Friend)));
    world.wars.insert((1, 2));
    assert_eq!(
        verdict(&graph, &world, "C", 3),
        Ok(Some(Verdict::Contested))
    );
    assert_eq!(relation(&graph, &world, "C", 3), Ok(RuleReason::Contested(1, 2)));
    cross(&mut graph, 1, 2, ruled("C"));
    world.controllers.insert(4, 1);
    world.controllers.insert(2, 1);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(true));
    world.controllers.insert(4, 2);
    assert_eq!(reaches(&graph, &geometry, &world, &army(), 3), Ok(false));
}

#[test]
fn overlord_chains_decide_the_fallback_and_a_loop_is_unknown() {
    let (graph, _) = map();
    let mut world = world();
    world.overlords.extend([(3, 4)]);
    world.wars.insert((1, 4));
    assert_eq!(verdict(&graph, &world, "R", 3), Ok(Some(Verdict::Enemy)));
    world.wars.clear();
    world.overlords.insert(4, 1);
    assert_eq!(verdict(&graph, &world, "R", 3), Ok(Some(Verdict::Friend)));
    assert_eq!(relation(&graph, &world, "R", 3), Ok(RuleReason::Friend(1)));
    world.overlords.insert(1, 3);
    assert!(
        verdict(&graph, &world, "R", 3)
            .unwrap_err()
            .contains("loops")
    );
}

struct Random(u32);

impl Random {
    fn next(&mut self, below: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 % below
    }
}

fn flood(mover: &Mover<'_, World>, army: &Army, unknown: bool) -> BTreeSet<u32> {
    let graph = mover.graph;
    let start = graph.dense(army.location).unwrap();
    let mut seen = BTreeSet::from([start]);
    let mut queue = vec![start];
    while let Some(at) = queue.pop() {
        for edge in graph.neighbours(at).unwrap_or_default() {
            let mut memo = Memo {
                nodes: vec![None; graph.ids.len()],
                rules: vec![None; graph.rules.len()],
            };
            let admitted = mover.admit(army, &mut memo, at, edge).unwrap_or(unknown);
            if admitted && seen.insert(edge.to as usize) {
                queue.push(edge.to as usize);
            }
        }
    }
    seen.into_iter().map(|at| graph.ids[at]).collect()
}

#[test]
fn the_reach_equals_a_plain_flood_fill_on_random_graphs() {
    let mut random = Random(0x9e37_79b9);
    let mut outcomes = [0_usize; 3];
    for case in 0..300 {
        let count = 2 + random.next(24);
        let mut graph = ProvinceGraph {
            rules: crate::adjacency_rules::parse(RULES).unwrap(),
            ..ProvinceGraph::default()
        };
        let mut geometry = ProvinceGeometry::default();
        let mut world = World::default();
        for id in 1..=count {
            geometry.centroids.insert(id, [0, 0]);
            if random.next(4) != 0 {
                geometry.land.insert(id);
            }
            let controller = 1 + random.next(3);
            world.controllers.insert(id, controller);
            world.state_owners.insert(
                id,
                [0, controller, 1 + random.next(3)][random.next(3) as usize],
            );
            if random.next(8) == 0 {
                world.unknown.insert(id);
            }
            if random.next(3) == 0 {
                world.ports.insert(id, 1 + random.next(count));
                world.bases.insert(id, random.next(2) == 0);
            }
        }
        for _ in 0..random.next(3) {
            let (a, b) = (1 + random.next(4), 1 + random.next(4));
            world.wars.insert((a.min(b), a.max(b)));
        }
        for _ in 0..random.next(4) {
            world.access.insert((1, 1 + random.next(4)));
        }
        for id in 1..=count {
            let mut edges = Vec::new();
            for _ in 0..1 + random.next(5) {
                let to = 1 + random.next(count);
                let cost = match random.next(10) {
                    0 => Some(0),
                    1 => None,
                    _ => Some(i64::from(random.next(90_000))),
                };
                edges.push((to, cost));
                match random.next(8) {
                    0 => cross(
                        &mut graph,
                        id,
                        to,
                        Authored {
                            kind: IMPASSABLE,
                            through: -1,
                            rule: None,
                        },
                    ),
                    1 => cross(
                        &mut graph,
                        id,
                        to,
                        ruled(["OPEN", "SHUT", "R", "BAD"][random.next(4) as usize]),
                    ),
                    _ => {}
                }
            }
            graph.edges.insert(id, edges);
        }
        let moves = MoveGraph::new(&graph, &geometry);
        let mover = Mover {
            graph: &moves,
            world: &world,
        };
        let army = Army {
            location: 1 + random.next(count),
            owner: 1,
            navy_country: 1,
            exfiltrates: random.next(2) == 0,
            ..army()
        };
        let reach = mover.reach(&army);
        let (sure, maybe) = (flood(&mover, &army, false), flood(&mover, &army, true));
        for id in 1..=count {
            let expected = if sure.contains(&id) {
                Some(true)
            } else if !maybe.contains(&id) {
                Some(false)
            } else {
                None
            };
            outcomes[match expected {
                Some(true) => 0,
                Some(false) => 1,
                None => 2,
            }] += 1;
            let answer = reach.answer(&moves, id);
            assert_eq!(answer.ok(), expected, "case {case} province {id}");
            let mut search = mover.search(&army);
            mover.advance(&army, &mut search, moves.dense(id));
            assert_eq!(
                search.reach.answer(&moves, id).ok(),
                expected,
                "case {case} early stop at {id}"
            );
        }
    }
    assert!(outcomes.iter().all(|&count| count > 100), "{outcomes:?}");
}

#[test]
fn selected_first_neighbour_does_not_demand_distance_or_other_crossing_fields() {
    let (mut graph, geometry) = map();
    graph
        .edges
        .insert(1, vec![(2, None), (3, Some(1)), (2, Some(99))]);
    graph.crossings.insert(
        (1, 2),
        Authored {
            kind: 3,
            ..Default::default()
        },
    );
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        moves.selected_neighbour(1, 2),
        Ok(Some(SelectedNeighbour { through: Ok(None) }))
    );
    assert_eq!(moves.selected_neighbour(1, 99), Ok(None));
    assert!(moves.selected_neighbour(99, 2).is_err());
    assert_eq!(moves.selected_neighbour_index(1, 2), Ok(Some(0)));
    assert_eq!(moves.selected_neighbour_index(1, 3), Ok(Some(1)));
    assert_eq!(moves.selected_neighbour_index(1, 99), Ok(None));
    assert!(moves.selected_neighbour_index(99, 2).is_err());
}

#[test]
fn selected_strait_sea_through_is_shared_and_invalid_through_is_refused() {
    let (mut graph, geometry) = map();
    graph.crossings.insert(
        (1, 2),
        Authored {
            kind: SEA_CROSSING,
            through: 5,
            ..Default::default()
        },
    );
    let moves = MoveGraph::new(&graph, &geometry);
    assert_eq!(
        moves.selected_neighbour(1, 2),
        Ok(Some(SelectedNeighbour {
            through: Ok(Some(5))
        }))
    );
    graph.crossings.get_mut(&(1, 2)).unwrap().through = -3;
    let moves = MoveGraph::new(&graph, &geometry);
    let selected = moves.selected_neighbour(1, 2).unwrap().unwrap();
    assert!(selected.through.is_err());
}

#[test]
fn selected_non_sea_or_land_lake_through_never_requests_fleet() {
    let (mut graph, geometry) = map();
    for (kind, through) in [(2, 5), (SEA_CROSSING, 0), (SEA_CROSSING, 1)] {
        graph.crossings.insert(
            (1, 2),
            Authored {
                kind,
                through,
                ..Default::default()
            },
        );
        let moves = MoveGraph::new(&graph, &geometry);
        assert_eq!(
            moves.selected_neighbour(1, 2).unwrap().unwrap().through,
            Ok(None)
        );
    }
}
