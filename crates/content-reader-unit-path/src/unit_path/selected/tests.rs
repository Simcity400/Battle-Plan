use super::*;
use crate::{
    parser::Item,
    unit_path::{MoveGraph, Occupant, StraitFleet},
};
use std::{cell::RefCell, collections::BTreeMap};

struct World {
    links: BTreeMap<u32, u32>,
    actual: BTreeMap<u32, u32>,
    controller: u32,
    state_owner: u32,
    state: Option<u32>,
    areas: Fact<bool>,
    wars: BTreeMap<(u32, u32), Fact<bool>>,
    access: Fact<bool>,
    current_subject: Fact<bool>,
    faction: Fact<Option<u64>>,
    overlords: BTreeMap<Country, Fact<Country>>,
    occupants: Fact<Vec<u64>>,
    armies: BTreeMap<u64, Fact<Option<NativeArmyOccupant>>>,
    retreats: BTreeMap<u64, Fact<bool>>,
    strengths: BTreeMap<u64, Fact<i64>>,
    combats: Fact<Vec<CommonCombat>>,
    primary: Fact<bool>,
    thresholds: [Fact<i64>; 2],
    maximum: Fact<i64>,
    bans: [Fact<bool>; 2],
    membership: BTreeMap<(u64, bool), Fact<bool>>,
    minimum: BTreeMap<u64, Fact<bool>>,
    first: Fact<Option<u32>>,
    path: Fact<bool>,
    transport: Fact<bool>,
    attacker_at: bool,
    rail_cooldown: Option<i32>,
    rail_accessible: Fact<bool>,
    rail_retreat_edge: Fact<bool>,
    owner_areas: Fact<bool>,
    d0: BTreeMap<u32, Fact<u32>>,
    nodes: BTreeMap<u32, Fact<AreaNode>>,
    nonsea_count: Fact<u32>,
    exile_raw: Fact<u8>,
    canonical_failure: Option<String>,
    controllers: BTreeMap<u32, Fact<u32>>,
    state_owners: BTreeMap<u32, Fact<u32>>,
    bases: BTreeMap<u32, Fact<bool>>,
    demilitarized: BTreeMap<u32, Fact<bool>>,
    impassable: Vec<u32>,
    rail_edges: BTreeMap<(u32, u32), Fact<bool>>,
    rail_owners: BTreeMap<(u32, u32, u8), Fact<bool>>,
    route_trace: RefCell<Vec<String>>,
    trace: RefCell<Vec<String>>,
}
impl Default for World {
    fn default() -> Self {
        Self {
            links: BTreeMap::new(),
            actual: BTreeMap::new(),
            controller: 1,
            state_owner: 1,
            state: Some(7),
            areas: Ok(true),
            wars: BTreeMap::new(),
            access: Ok(false),
            current_subject: Ok(false),
            faction: Ok(None),
            overlords: Default::default(),
            occupants: Ok(Vec::new()),
            armies: Default::default(),
            retreats: Default::default(),
            strengths: Default::default(),
            combats: Ok(Vec::new()),
            primary: Ok(false),
            thresholds: [Ok(2), Ok(3)],
            maximum: Ok(100),
            bans: [Ok(false), Ok(false)],
            membership: Default::default(),
            minimum: Default::default(),
            first: Ok(None),
            path: Ok(true),
            transport: Ok(false),
            attacker_at: false,
            rail_cooldown: None,
            rail_accessible: Ok(true),
            rail_retreat_edge: Ok(true),
            owner_areas: Ok(false),
            d0: Default::default(),
            nodes: Default::default(),
            nonsea_count: Ok(8),
            exile_raw: Ok(0),
            canonical_failure: None,
            controllers: Default::default(),
            state_owners: Default::default(),
            bases: Default::default(),
            demilitarized: Default::default(),
            impassable: Vec::new(),
            rail_edges: Default::default(),
            rail_owners: Default::default(),
            route_trace: Default::default(),
            trace: Default::default(),
        }
    }
}
impl MoveWorld for World {
    fn path_friendly(&self, _: Country, _: Country) -> Fact<bool> {
        panic!("unreached path query")
    }
    fn at_war(&self, _: Country, _: Country) -> Fact<bool> {
        panic!("canonical war must not replace raw selected predicate")
    }
    fn military_access(&self, _: Country, _: Country) -> Fact<bool> {
        self.access.clone()
    }
    fn subject_tie(&self, _: Country, _: Country) -> Fact<bool> {
        panic!("historical subject dates cannot certify selected disclosure")
    }
    fn faction(&self, _: Country) -> Fact<Option<u64>> {
        self.faction.clone()
    }
    fn overlord(&self, country: Country) -> Fact<Country> {
        self.overlords.get(&country).cloned().unwrap_or(Ok(0))
    }
    fn can_occupy_non_war(&self, _: Country) -> Fact<bool> {
        Ok(false)
    }
    fn enemies(&self, _: Country) -> Fact<Vec<Country>> {
        panic!("derived enemy sets must not replace the ordered enemy vector")
    }
    fn controller(&self, _: u32) -> Fact<Country> {
        self.canonical_country(self.controller)
    }
    fn controller_raw_positive(&self, _: u32) -> Fact<bool> {
        Ok(self.controller as i32 > 0)
    }
    fn state_owner(&self, _: u32) -> Fact<Country> {
        self.canonical_country(self.state_owner)
    }
    fn state_owner_raw_positive(&self, _: u32) -> Fact<bool> {
        Ok(self.state_owner as i32 > 0)
    }
    fn demilitarized(&self, province: u32) -> Fact<bool> {
        self.route_trace
            .borrow_mut()
            .push(format!("demilitarized:{province}"));
        self.demilitarized
            .get(&province)
            .cloned()
            .unwrap_or(Ok(false))
    }
    fn impassable(&self, province: u32) -> Fact<bool> {
        Ok(self.impassable.contains(&province))
    }
    fn owner_area_missing(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn border_conflict(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn border_conflict_admits(&self, _: u32, _: u64) -> Fact<bool> {
        Ok(false)
    }
    fn has_naval_base(&self, province: u32) -> Fact<bool> {
        self.route_trace
            .borrow_mut()
            .push(format!("base:{province}"));
        self.bases.get(&province).cloned().unwrap_or(Ok(false))
    }
    fn naval_base_active(&self, _: u32) -> Fact<bool> {
        Ok(false)
    }
    fn port_sea(&self, _: u32) -> Fact<u32> {
        Ok(0)
    }
    fn armies(&self, _: u32) -> Fact<Vec<Occupant>> {
        panic!("unordered/canonical occupants must not replace the recorded array")
    }
    fn land_combat(&self, _: u32) -> Fact<bool> {
        panic!("unreached land combat query")
    }
    fn strait_fleet(&self, _: u32, _: Country) -> Fact<StraitFleet> {
        panic!("unreached strait")
    }
    fn strategic_region(&self, _: u32) -> Option<u32> {
        None
    }
    fn region_access(&self, _: Country, _: u32) -> Fact<u8> {
        panic!("unreached region access")
    }
    fn evaluate(&self, body: &[Item], _: Country) -> Fact<bool> {
        Ok(body
            .iter()
            .all(|item| item.value.scalar_str() == Some("yes")))
    }
    fn has_children(&self, body: &[Item]) -> Fact<bool> {
        Ok(!body.is_empty())
    }
}
impl CommonMoveWorld for World {
    fn current_subject_access(&self, _: Country, _: Country) -> Fact<bool> {
        self.current_subject.clone()
    }
    fn canonical_country(&self, raw: u32) -> Fact<Country> {
        self.route_trace
            .borrow_mut()
            .push(format!("canonical:{raw}"));
        if let Some(reason) = &self.canonical_failure {
            return Err(reason.clone());
        }
        Ok(*self.links.get(&raw).unwrap_or(&raw))
    }
    fn actual_country_raw(&self, country: Country) -> Fact<u32> {
        Ok(*self.actual.get(&country).unwrap_or(&country))
    }
    fn controller_raw(&self, province: u32) -> Fact<u32> {
        self.route_trace
            .borrow_mut()
            .push(format!("controller:{province}"));
        self.controllers
            .get(&province)
            .cloned()
            .unwrap_or(Ok(self.controller))
    }
    fn state_owner_raw(&self, province: u32) -> Fact<u32> {
        self.route_trace
            .borrow_mut()
            .push(format!("state-owner:{province}"));
        self.state_owners
            .get(&province)
            .cloned()
            .unwrap_or(Ok(self.state_owner))
    }
    fn province_state(&self, _: u32) -> Fact<Option<u32>> {
        Ok(self.state)
    }
    fn controller_area_equal(&self, _: u32, _: u32) -> Fact<bool> {
        self.areas.clone()
    }
    fn owner_area_equal(&self, from: u32, to: u32) -> Fact<bool> {
        self.route_trace
            .borrow_mut()
            .push(format!("D8:{from}->{to}"));
        self.owner_areas.clone()
    }
    fn controller_area(&self, province: u32) -> Fact<u32> {
        self.route_trace.borrow_mut().push(format!("D0:{province}"));
        self.d0
            .get(&province)
            .cloned()
            .unwrap_or(Err("D0 capture unavailable".into()))
    }
    fn area_node(&self, label: u32) -> Fact<AreaNode> {
        self.route_trace.borrow_mut().push(format!("area:{label}"));
        self.nodes
            .get(&label)
            .cloned()
            .unwrap_or(Err("area graph unavailable".into()))
    }
    fn area_nonsea_count(&self, _: u32) -> Fact<u32> {
        self.nonsea_count.clone()
    }
    fn unit_exiled_raw(&self, _: &CommonUnit<'_>) -> Fact<u8> {
        self.route_trace.borrow_mut().push("raw-exile".into());
        self.exile_raw.clone()
    }
    fn railway_path_owner_admits(&self, owner: u32, controller: u32, exile: u8) -> Fact<bool> {
        self.route_trace
            .borrow_mut()
            .push(format!("rail-owner:{owner}->{controller}:{exile}"));
        self.rail_owners
            .get(&(owner, controller, exile))
            .cloned()
            .unwrap_or(Ok(true))
    }
    fn directed_war(&self, receiver: Country, target: Country) -> Fact<bool> {
        self.trace
            .borrow_mut()
            .push(format!("war:{receiver}->{target}"));
        self.wars
            .get(&(receiver, target))
            .cloned()
            .unwrap_or(Ok(false))
    }
    fn full_unit_combats(&self, _: &CommonUnit<'_>) -> Fact<Vec<CommonCombat>> {
        self.combats.clone()
    }
    fn primary_path_combat(&self, _: &CommonUnit<'_>, _: u32) -> Fact<bool> {
        self.primary.clone()
    }
    fn province_units(&self, _: u32) -> Fact<&[u64]> {
        self.trace.borrow_mut().push("occupants".into());
        self.occupants.as_deref().map_err(Clone::clone)
    }
    fn army_occupant(&self, handle: u64, _: u32) -> Fact<Option<NativeArmyOccupant>> {
        self.armies
            .get(&handle)
            .cloned()
            .unwrap_or(Err("occupant class held".into()))
    }
    fn occupant_retreating(&self, handle: u64) -> Fact<bool> {
        self.retreats.get(&handle).cloned().unwrap_or(Ok(false))
    }
    fn occupant_strength(&self, handle: u64) -> Fact<i64> {
        self.strengths.get(&handle).cloned().unwrap_or(Ok(0))
    }
    fn unit_orders_group(&self, _: &CommonUnit<'_>) -> Fact<u64> {
        Ok(0)
    }
    fn ordered_enemy_raws(&self, _: Country) -> Fact<Vec<u32>> {
        panic!("unreached ordered enemies")
    }
    fn move_strength_threshold(&self) -> Fact<i64> {
        self.trace.borrow_mut().push("threshold:0".into());
        self.thresholds[0].clone()
    }
    fn move_organisation_threshold(&self) -> Fact<i64> {
        self.trace.borrow_mut().push("threshold:1".into());
        self.thresholds[1].clone()
    }
    fn maximum_organisation(&self, _: &CommonUnit<'_>) -> Fact<i64> {
        self.maximum.clone()
    }
    fn unit_exiled(&self, _: &CommonUnit<'_>) -> Fact<bool> {
        Ok(false)
    }
    fn unit_exfiltrates(&self, _: &CommonUnit<'_>) -> Fact<bool> {
        Ok(false)
    }
    fn unit_retreat_ban(&self, _: &CommonUnit<'_>, attacking: bool) -> Fact<bool> {
        self.trace.borrow_mut().push(format!("ban:{attacking}"));
        self.bans[usize::from(attacking)].clone()
    }
    fn combat_has_unit(&self, combat: CommonCombat, _: u64, attacking: bool) -> Fact<bool> {
        self.trace
            .borrow_mut()
            .push(format!("side:{}:{attacking}", combat.combat));
        self.membership
            .get(&(combat.combat, attacking))
            .cloned()
            .unwrap_or(Ok(false))
    }
    fn combat_minimum_duration(&self, combat: CommonCombat) -> Fact<bool> {
        self.trace
            .borrow_mut()
            .push(format!("duration:{}", combat.combat));
        self.minimum
            .get(&combat.combat)
            .cloned()
            .unwrap_or(Ok(false))
    }
    fn unit_route_first(&self, _: &CommonUnit<'_>) -> Fact<Option<u32>> {
        self.first.clone()
    }
    fn railway_retreat_edge(&self, from: u32, to: u32) -> Fact<bool> {
        self.trace.borrow_mut().push("railway-retreat-edge".into());
        self.route_trace
            .borrow_mut()
            .push(format!("rail-edge:{from}->{to}"));
        self.rail_edges
            .get(&(from, to))
            .cloned()
            .unwrap_or_else(|| self.rail_retreat_edge.clone())
    }
    fn combat_attacker_at(&self, combat: CommonCombat, _: u32) -> Fact<bool> {
        self.trace
            .borrow_mut()
            .push(format!("attacker-location:{}", combat.combat));
        Ok(self.attacker_at)
    }
    fn selected_path(
        &self,
        _: &CommonUnit<'_>,
        _: u32,
        _: &mut String,
        _: &impl CommonMoveText,
    ) -> Fact<bool> {
        self.trace.borrow_mut().push("selected-path".into());
        self.path.clone()
    }
    fn selected_transport(
        &self,
        _: &CommonUnit<'_>,
        _: u32,
        _: &mut String,
        _: &impl CommonMoveText,
    ) -> Fact<bool> {
        self.trace.borrow_mut().push("selected-transport".into());
        self.transport.clone()
    }
    fn railway_cooldown(&self, _: u32) -> Fact<Option<i32>> {
        Ok(self.rail_cooldown)
    }
    fn railway_accessible(&self, _: u32) -> Fact<bool> {
        self.rail_accessible.clone()
    }
}

#[derive(Default)]
struct Text {
    calls: RefCell<Vec<(String, String)>>,
    empty: Option<&'static str>,
}
impl CommonMoveText for Text {
    fn transport_stock(&self, available: i32, required: i32) -> Fact<String> {
        Ok(format!(
            "[TRANSPORT_NOT_ENOUGH:FREE={available}:NEED={required}]"
        ))
    }
    fn transport_risk(&self, risk: i64) -> Fact<String> {
        Ok(format!("[NAVAL_TRANSPORT_INTERCEPT_WARNING:ATTR={risk}]"))
    }
    fn transport_description(&self, convoys: i32, icon: bool) -> Fact<String> {
        Ok(format!("[transport:CONVOYS={convoys}:icon={icon}]"))
    }
    fn days(&self, key: &str, days: i32) -> Fact<String> {
        self.calls
            .borrow_mut()
            .push((key.into(), format!("DAYS={days}")));
        Ok(format!("[{key}:DAYS={days}]"))
    }
    fn plain(&self, key: &str) -> Fact<String> {
        self.calls.borrow_mut().push((key.into(), String::new()));
        Ok(if self.empty == Some(key) {
            String::new()
        } else {
            format!("[{key}]")
        })
    }
    fn state(&self, key: &str, state: u32) -> Fact<String> {
        self.calls
            .borrow_mut()
            .push((key.into(), format!("STATE={state}")));
        Ok(format!("[{key}:STATE={state}]"))
    }
    fn owner(&self, key: &str, owner: u32) -> Fact<String> {
        self.calls
            .borrow_mut()
            .push((key.into(), format!("OWNER={owner}")));
        Ok(format!("[{key}:OWNER={owner}]"))
    }
    fn rule(&self, key: &str, rule: &AdjacencyRule) -> Fact<String> {
        self.calls
            .borrow_mut()
            .push((key.into(), format!("RULE_NAME={}", rule.name)));
        Ok(format!("[{key}:RULE_NAME={}]", rule.name))
    }
    fn warning(&self, key: &str, value: i64) -> Fact<String> {
        self.calls
            .borrow_mut()
            .push((key.into(), format!("VAL={value}")));
        Ok(format!("[{key}:VAL={value}]"))
    }
}
fn graph() -> MoveGraph {
    MoveGraph {
        index: vec![super::super::NONE, 0, 1],
        ids: vec![1, 2],
        kinds: vec![Some(1), Some(1)],
        ranges: vec![Some((0, 0)), Some((0, 0))],
        ..Default::default()
    }
}
fn army() -> Army {
    Army {
        location: 1,
        owner: 1,
        strength: 10,
        organisation: 10,
        ..Default::default()
    }
}
fn unit(army: &Army) -> CommonUnit<'_> {
    CommonUnit {
        class: CommonClass::Army,
        handle: 77,
        logical_owner_raw: 1,
        combat_count: 0,
        army,
    }
}

#[test]
fn selected_transfer_and_counterattack_gates_precede_class_path() {
    let graph = graph();
    let world = World {
        path: Err("unreached area graph".into()),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let transferred = Army {
        naval_transfer: true,
        ..army()
    };
    let mut text = String::new();
    assert_eq!(
        mover.selected_move_message(&unit(&transferred), 1, &mut text, &Text::default()),
        Ok(false)
    );
    assert_eq!(text, "[TRANSPORT_ACTIVE_CANT_MOVE]");
    assert!(world.trace.borrow().is_empty());

    let world = World {
        combats: Ok(vec![CommonCombat { combat: 8, kind: 1 }]),
        membership: BTreeMap::from([((8, false), Ok(true))]),
        attacker_at: true,
        path: Err("unreached area graph".into()),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let army = army();
    let mut selected = unit(&army);
    selected.combat_count = 1;
    let mut text = String::new();
    assert_eq!(
        mover.selected_move_message(&selected, 1, &mut text, &Text::default()),
        Ok(false)
    );
    assert_eq!(text, "[NO_COUNTER_ATTACK_IN_COMBAT]");
    assert_eq!(
        *world.trace.borrow(),
        ["side:8:false", "attacker-location:8"]
    );
}

#[test]
fn selected_allows_retreat_and_falls_back_only_after_a_known_failed_path() {
    let graph = graph();
    let world = World {
        path: Ok(false),
        transport: Ok(true),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let army = Army {
        retreating: true,
        ..army()
    };
    let mut text = String::new();
    assert_eq!(
        mover.selected_move_message(&unit(&army), 2, &mut text, &Text::default()),
        Ok(true)
    );
    assert!(text.is_empty());
    assert!(
        world
            .trace
            .borrow()
            .ends_with(&["selected-path".into(), "selected-transport".into()])
    );
    let world = World {
        path: Err("area graph missing".into()),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(
        mover.selected_move_message(&unit(&army), 2, &mut text, &Text::default()),
        Err("area graph missing".into())
    );
    assert!(
        !world
            .trace
            .borrow()
            .iter()
            .any(|call| call == "selected-transport")
    );
}

#[test]
fn selected_railway_cooldown_message_precedes_common_access_and_route() {
    let graph = graph();
    let world = World {
        rail_cooldown: Some(4),
        areas: Err("unreached D0".into()),
        rail_accessible: Err("unreached railway pair count".into()),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let army = army();
    let mut selected = unit(&army);
    selected.class = CommonClass::RailwayGun;
    let mut text = String::new();
    assert_eq!(
        mover.selected_move_message(&selected, 2, &mut text, &Text::default()),
        Ok(false)
    );
    assert_eq!(text, "[NO_RAILWAY_ACCESS_STANDING_COOLDOWN:DAYS=4]");
    assert!(world.trace.borrow().is_empty());
}

#[test]
fn railway_source_gate_precedes_inherited_retreat_operands() {
    let mut graph = graph();
    graph.ranges[0] = Some((0, 1));
    graph.edges.push(super::super::Edge {
        to: 1,
        cost: super::super::Cost::Moving,
        crossing: super::super::NONE,
        distance: Some(1),
    });
    let army = Army {
        organisation: 10_000_000,
        ..army()
    };
    let mut selected = unit(&army);
    selected.class = CommonClass::RailwayGun;
    for gate in [Ok(false), Err("neighbor level missing".into())] {
        let world = World {
            rail_retreat_edge: gate.clone(),
            occupants: Err("unreached occupants".into()),
            wars: BTreeMap::from([((1, 1), Err("unreached war".into()))]),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .common_retreats(&selected, 2),
            gate
        );
        assert_eq!(*world.trace.borrow(), ["railway-retreat-edge"]);
    }
    let world = World {
        controller: 2,
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_retreats(&selected, 2),
        Ok(true)
    );
    assert_eq!(
        *world.trace.borrow(),
        ["railway-retreat-edge", "war:2->1", "occupants"]
    );
}

#[test]
fn selected_keeps_advance_warning_even_when_no_enemy_allows_the_path() {
    let graph = graph();
    let world = World::default();
    let army = Army {
        strength: 1,
        ..army()
    };
    let mut text = String::new();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_move_message(&unit(&army), 1, &mut text, &Text::default()),
        Ok(true)
    );
    assert_eq!(text, "[NO_STRENGTH_TO_FIGHT:VAL=2]");
    assert_eq!(
        *world.trace.borrow(),
        ["threshold:0", "occupants", "selected-path"]
    );
}

#[test]
fn false_access_appends_before_missing_combats_and_occupants() {
    let mut graph = graph();
    graph.kinds[1] = Some(0);
    let world = World {
        occupants: Err("occupants held".into()),
        combats: Err("combat held".into()),
        areas: Err("D0 held".into()),
        ..Default::default()
    };
    let army = army();
    let mut unit = unit(&army);
    unit.combat_count = 9;
    let mut output = "prior".to_owned();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit, 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "prior[NO_ACCESS_TO_TARGET]");
    assert!(world.trace.borrow().is_empty());
}
#[test]
fn empty_access_localisation_still_rejects_without_clearing_prior_text() {
    let mut graph = graph();
    graph.kinds[1] = Some(0);
    let world = World::default();
    let army = army();
    let mut output = "prior".to_owned();
    let text = Text {
        empty: Some("NO_ACCESS_TO_TARGET"),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "prior");
}
#[test]
fn unknown_d0_is_not_controller_equality_or_no_access_text() {
    let graph = graph();
    let world = World {
        areas: Err("D0 held".into()),
        ..Default::default()
    };
    let army = army();
    let mut output = "prior".to_owned();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &text),
        Err("D0 held".into())
    );
    assert_eq!(output, "prior");
    assert!(text.calls.borrow().is_empty());
}
#[test]
fn low_strength_replaces_prior_bytes_without_maximum_org_demand() {
    let graph = graph();
    let world = World {
        maximum: Err("max held".into()),
        thresholds: [Ok(7), Err("org threshold held".into())],
        occupants: Ok(vec![22]),
        armies: BTreeMap::from([(
            22,
            Ok(Some(NativeArmyOccupant {
                handle: 22,
                logical_owner_raw: 2,
            })),
        )]),
        strengths: BTreeMap::from([(22, Err("strength must be undemanded".into()))]),
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        ..Default::default()
    };
    let mut army = army();
    army.strength = 6;
    let mut output = "prior".to_owned();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "[NO_STRENGTH_TO_FIGHT:VAL=7]");
    assert_eq!(
        &*world.trace.borrow(),
        &["occupants", "war:2->1", "threshold:0"]
    );
}
#[test]
fn retreating_occupant_is_skipped_before_first_enemy_advance() {
    let graph = graph();
    let world = World {
        occupants: Ok(vec![99, 22]),
        armies: BTreeMap::from([
            (
                99,
                Ok(Some(NativeArmyOccupant {
                    handle: 99,
                    logical_owner_raw: 9,
                })),
            ),
            (
                22,
                Ok(Some(NativeArmyOccupant {
                    handle: 22,
                    logical_owner_raw: 2,
                })),
            ),
        ]),
        retreats: BTreeMap::from([(99, Ok(true))]),
        wars: BTreeMap::from([((9, 1), Err("must be skipped".into())), ((2, 1), Ok(true))]),
        ..Default::default()
    };
    let mut army = army();
    army.strength = 1;
    let mut output = String::new();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(
        &*world.trace.borrow(),
        &["occupants", "war:2->1", "threshold:0"]
    );
}
#[test]
fn canonical_zero_alias_does_not_bypass_actual_zero_direct_war_gate() {
    let graph = graph();
    let world = World {
        controller: 2,
        state_owner: 3,
        wars: BTreeMap::from([((0, 2), Ok(true))]),
        ..Default::default()
    };
    let mut army = army();
    army.owner = 0;
    army.retreating = true;
    let mut unit = unit(&army);
    unit.logical_owner_raw = 0;
    let mut output = "prior".to_owned();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit, 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "prior[UNIT_CANT_MOVE_TO_OCCUPIED_NEUTRAL:OWNER=3]");
    assert_eq!(&*world.trace.borrow(), &["war:0->2", "war:0->3"]);
}
#[test]
fn province_access_actual_zero_active_war_skips_later_operands() {
    let graph = graph();
    let world = World {
        links: BTreeMap::from([(99, 0), (100, 1), (200, 2)]),
        controller: 200,
        state_owner: 200,
        areas: Ok(false),
        controllers: BTreeMap::from([(1, Ok(100)), (2, Ok(200))]),
        wars: BTreeMap::from([((0, 2), Ok(true))]),
        access: Err("military access must remain unreached".into()),
        faction: Err("faction must remain unreached".into()),
        overlords: BTreeMap::from([
            (0, Err("receiver overlord must remain unreached".into())),
            (2, Err("controller overlord must remain unreached".into())),
        ]),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    let army = Army { owner: 0, ..army() };
    let mut selected = unit(&army);
    for raw in [0, 99] {
        selected.logical_owner_raw = raw;
        assert_eq!(mover.common_class_access(&selected, 2), Ok(true));
    }
    assert_eq!(*world.trace.borrow(), ["war:0->2", "war:0->2"]);
}

#[test]
fn province_access_actual_zero_clear_war_demands_military_access() {
    let graph = graph();
    let mut world = World {
        links: BTreeMap::from([(200, 2)]),
        controller: 200,
        state_owner: 200,
        wars: BTreeMap::from([((0, 2), Ok(false))]),
        overlords: BTreeMap::from([(0, Ok(0)), (2, Ok(3))]),
        ..Default::default()
    };
    for access in [
        Ok(false),
        Ok(true),
        Err("military access capture refused".into()),
    ] {
        world.access = access.clone();
        world.trace.borrow_mut().clear();
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .common_has_province_access(0, 2),
            access
        );
        assert_eq!(*world.trace.borrow(), ["war:0->2"]);
    }
}

#[test]
fn province_access_actual_zero_missing_war_refuses_before_known_access() {
    let graph = graph();
    let world = World {
        links: BTreeMap::from([(200, 2)]),
        controller: 200,
        state_owner: 200,
        wars: BTreeMap::from([((0, 2), Err("directed war cell omitted".into()))]),
        access: Ok(true),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_has_province_access(0, 2),
        Err("directed war cell omitted".into())
    );
    assert_eq!(*world.trace.borrow(), ["war:0->2"]);
}

#[test]
fn raw_self_and_nonpositive_f3a00_gates_do_not_demand_direct_packet() {
    let graph = graph();
    let world = World {
        links: BTreeMap::from([(99, 0)]),
        wars: BTreeMap::from([((0, 0), Err("unreached direct packet".into()))]),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(mover.raw_at_war(0, 99), Ok(false));
    assert_eq!(mover.raw_at_war(1, 1), Ok(false));
    assert!(world.trace.borrow().is_empty());
    assert_eq!(mover.raw_same(0, 99), Ok(false));
}
#[test]
fn blocked_primary_with_known_no_state_returns_false_with_unchanged_bytes() {
    let graph = graph();
    let world = World {
        controller: 2,
        state: None,
        primary: Ok(true),
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        combats: Ok(vec![CommonCombat {
            combat: 88,
            kind: 0,
        }]),
        first: Ok(Some(2)),
        ..Default::default()
    };
    let mut army = army();
    army.route_first = Some(2);
    let mut unit = unit(&army);
    unit.combat_count = 1;
    let mut output = "prior".to_owned();
    let text = Text::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit, 2, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "prior");
    assert!(text.calls.borrow().is_empty());
}
#[test]
fn full_combat_count_mismatch_is_unknown_not_empty_rejection() {
    let graph = graph();
    let world = World {
        controller: 2,
        primary: Ok(true),
        first: Ok(Some(2)),
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        ..Default::default()
    };
    let mut army = army();
    army.route_first = Some(2);
    let mut unit = unit(&army);
    unit.combat_count = 1;
    let mut output = String::new();
    let text = Text::default();
    assert!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit, 2, &mut output, &text)
        .is_err()
    );
    assert!(output.is_empty());
}
#[test]
fn commander_ban_reads_full_side_membership_including_nonland_combats() {
    let graph = graph();
    let world = World {
        bans: [Ok(true), Err("unreached attacker ban".into())],
        membership: BTreeMap::from([
            ((88, false), Ok(true)),
            ((99, false), Err("later side held".into())),
        ]),
        ..Default::default()
    };
    let army = army();
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(
        mover.common_disengages(
            &unit(&army),
            &[
                CommonCombat {
                    combat: 88,
                    kind: 2
                },
                CommonCombat {
                    combat: 99,
                    kind: 1
                }
            ]
        ),
        Ok(false)
    );
    assert_eq!(&*world.trace.borrow(), &["ban:false", "side:88:false"]);
}
#[test]
fn railway_selection_uses_transport_presence_and_plain_advance_key() {
    let graph = graph();
    let world = World {
        thresholds: [Err("not army".into()), Err("not army".into())],
        ..Default::default()
    };
    let mut army = army();
    let mut output = "prior".to_owned();
    let text = Text::default();
    let mut request = unit(&army);
    request.class = CommonClass::RailwayGun;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_advance(&request, &mut output, &text),
        Ok(false)
    );
    assert_eq!(output, "prior[NO_ADVANCE_ALLOWED]");
    army.naval_transfer = true;
    let mut request = unit(&army);
    request.class = CommonClass::RailwayGun;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_advance(&request, &mut output, &text),
        Ok(true)
    );
    assert_eq!(text.calls.borrow().len(), 1);
}

#[test]
fn combat_duration_first_hit_does_not_demand_later_unknown_combat() {
    let graph = graph();
    let world = World {
        minimum: BTreeMap::from([(88, Ok(true)), (99, Err("later duration held".into()))]),
        ..Default::default()
    };
    let army = army();
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(
        mover.common_disengages(
            &unit(&army),
            &[
                CommonCombat {
                    combat: 88,
                    kind: 0
                },
                CommonCombat {
                    combat: 99,
                    kind: 2
                }
            ]
        ),
        Ok(true)
    );
    assert_eq!(
        &*world.trace.borrow(),
        &["ban:false", "ban:true", "duration:88"]
    );
}
#[test]
fn zero_combats_do_not_demand_missing_route() {
    let graph = graph();
    let world = World {
        first: Err("route held".into()),
        ..Default::default()
    };
    let army = army();
    let mut output = String::new();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &Text::default()),
        Ok(true)
    );
    assert!(output.is_empty());
}

#[test]
fn first_enemy_advance_rejects_before_later_unknown_occupant() {
    let graph = graph();
    let world = World {
        occupants: Ok(vec![22, 99]),
        armies: BTreeMap::from([
            (
                22,
                Ok(Some(NativeArmyOccupant {
                    handle: 22,
                    logical_owner_raw: 2,
                })),
            ),
            (99, Err("later class held".into())),
        ]),
        wars: BTreeMap::from([((2, 1), Ok(true))]),
        ..Default::default()
    };
    let mut army = army();
    army.strength = 1;
    let mut output = String::new();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&unit(&army), 2, &mut output, &Text::default()),
        Ok(false)
    );
    assert!(output.starts_with("[NO_STRENGTH_TO_FIGHT:"));
}

#[test]
fn signed_nonpositive_combat_count_does_not_demand_route_or_full_vector() {
    let graph = graph();
    let world = World {
        first: Err("route held".into()),
        combats: Err("full refs held".into()),
        primary: Err("primary held".into()),
        ..Default::default()
    };
    let mut army = army();
    army.retreating = true;
    let mut request = unit(&army);
    request.combat_count = u32::MAX;
    let mut output = "prior".to_owned();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_enter_message(&request, 2, &mut output, &Text::default()),
        Ok(true)
    );
    assert_eq!(output, "prior");
    assert!(world.trace.borrow().is_empty());
}

fn selected_rule() -> AdjacencyRule {
    crate::adjacency_rules::parse(b"adjacency_rule = { name = fixture required_provinces = { 2 } }")
        .unwrap()
        .remove(0)
}

#[test]
fn common_adjacency_omitted_direct_cell_stays_unknown() {
    let graph = graph();
    let world = World {
        controller: 2,
        wars: BTreeMap::from([(
            (1, 2),
            Err("directed target outside observed domain".into()),
        )]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_rule_verdict(&selected_rule(), 1),
        Err("directed target outside observed domain".into())
    );
    assert_eq!(&*world.trace.borrow(), &["war:1->2"]);
}

#[test]
fn common_adjacency_active_direction_returns_enemy_without_symmetric_query() {
    let graph = graph();
    let world = World {
        controller: 2,
        wars: BTreeMap::from([
            ((1, 2), Ok(true)),
            ((2, 1), Err("reverse is not queried".into())),
        ]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_rule_verdict(&selected_rule(), 1),
        Ok(Some(crate::adjacency_rules::Verdict::Enemy))
    );
    assert_eq!(&*world.trace.borrow(), &["war:1->2"]);
}

#[test]
fn common_adjacency_friend_disclosure_has_no_positive_receiver_gate() {
    let graph = graph();
    let world = World {
        controller: 2,
        wars: BTreeMap::from([((0, 2), Err("actual-zero direct cell unavailable".into()))]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .common_rule_verdict(&selected_rule(), 0),
        Err("actual-zero direct cell unavailable".into())
    );
    assert_eq!(&*world.trace.borrow(), &["war:0->2"]);
}

#[test]
fn common_disclosure_access_is_reached_only_after_clear_direct_war() {
    let graph = graph();
    let world = World {
        access: Err("access unavailable".into()),
        wars: BTreeMap::from([((1, 2), Ok(false)), ((1, 3), Ok(true))]),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(mover.common_discloses(1, 2, false), Ok(false));
    assert_eq!(
        mover.common_discloses(1, 2, true),
        Err("access unavailable".into())
    );
    assert_eq!(mover.common_discloses(1, 3, true), Ok(false));
}

#[test]
fn selected_disclosure_uses_current_subject_inputs_and_never_historical_dates() {
    let graph = graph();
    for subject in [
        Ok(false),
        Ok(true),
        Err("current subject capture refused".into()),
    ] {
        let world = World {
            current_subject: subject.clone(),
            wars: BTreeMap::from([((1, 2), Ok(false))]),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .common_discloses(1, 2, false),
            subject
        );
    }
    let world = World {
        current_subject: Err("unreached current subject".into()),
        access: Ok(true),
        wars: BTreeMap::from([((1, 2), Ok(false)), ((1, 3), Ok(true))]),
        ..Default::default()
    };
    let mover = Mover {
        graph: &graph,
        world: &world,
    };
    assert_eq!(mover.common_discloses(1, 2, true), Ok(true));
    assert_eq!(mover.common_discloses(1, 3, true), Ok(false));
    assert_eq!(mover.common_discloses(1, 1, false), Ok(true));
}

fn route_graph(count: u32, neighbours: &[(u32, u32, Option<i64>, bool)]) -> MoveGraph {
    use super::super::{Cost, Crossing, Edge, NONE, Strait};
    let mut edges = Vec::new();
    let mut crossings = Vec::new();
    let mut ranges = Vec::new();
    for from in 1..=count {
        let start = edges.len() as u32;
        for &(_, to, distance, impassable) in neighbours.iter().filter(|edge| edge.0 == from) {
            let crossing = if impassable {
                let index = crossings.len() as u32;
                crossings.push(Crossing {
                    impassable,
                    strait: Strait::None,
                    rule: None,
                });
                index
            } else {
                NONE
            };
            edges.push(Edge {
                to: to - 1,
                distance,
                crossing,
                cost: match distance {
                    Some(0) => Cost::Zero,
                    Some(_) => Cost::Moving,
                    None => Cost::Unknown,
                },
            });
        }
        ranges.push(Some((start, edges.len() as u32)));
    }
    MoveGraph {
        width: 100,
        index: std::iter::once(NONE).chain(0..count).collect(),
        ids: (1..=count).collect(),
        kinds: vec![Some(1); count as usize],
        centroids: vec![Some([0, 0]); count as usize],
        edges,
        crossings,
        ranges,
        ..Default::default()
    }
}

fn railway_unit(army: &Army) -> CommonUnit<'_> {
    CommonUnit {
        class: CommonClass::RailwayGun,
        ..unit(army)
    }
}

#[test]
fn army_route_d8_access_and_d0_identity_keep_their_independent_early_returns() {
    let graph = route_graph(2, &[]);
    let army = army();
    let world = World {
        owner_areas: Ok(true),
        exile_raw: Err("unreached exile".into()),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(true)
    );
    assert_eq!(
        &*world.route_trace.borrow(),
        &["D8:1->2", "canonical:1", "state-owner:2"]
    );

    let world = World {
        d0: BTreeMap::from([(1, Ok(9)), (2, Ok(9))]),
        canonical_failure: Some("unreached country resolution".into()),
        nonsea_count: Err("unreached graph domain".into()),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(true)
    );
    assert_eq!(
        &*world.route_trace.borrow(),
        &["D8:1->2", "raw-exile", "D0:1", "D0:2"]
    );
}

#[test]
fn army_route_raw_exile_xor_keeps_nonboolean_bytes_and_skips_unreached_graph() {
    let graph = route_graph(2, &[]);
    let army = Army {
        exiled: true,
        ..army()
    };
    for (exile, expected) in [(1, false), (2, true)] {
        let world = World {
            exile_raw: Ok(exile),
            d0: BTreeMap::from([(1, Ok(8)), (2, Ok(8))]),
            bases: BTreeMap::from([(2, Ok(true))]),
            canonical_failure: Some("unreached country resolution".into()),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .selected_army_path(&unit(&army), 2),
            Ok(expected)
        );
        if exile == 1 {
            assert_eq!(
                &*world.route_trace.borrow(),
                &["D8:1->2", "raw-exile", "base:2"]
            );
        } else {
            assert_eq!(
                &*world.route_trace.borrow(),
                &["D8:1->2", "raw-exile", "D0:1", "D0:2"]
            );
        }
    }
}

#[test]
fn army_route_zero_negative_and_unknown_distances_are_not_kind4() {
    let army = army();
    for distance in [Some(0), Some(-100), None] {
        let graph = route_graph(2, &[(1, 2, distance, false)]);
        let world = World {
            exile_raw: Ok(1),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .selected_army_path(&unit(&army), 2),
            Ok(true)
        );
        assert!(
            !world
                .route_trace
                .borrow()
                .iter()
                .any(|call| call.starts_with("D0:"))
        );
    }
    let graph = route_graph(2, &[(1, 2, Some(100), true)]);
    let world = World {
        exile_raw: Ok(1),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(false)
    );
    assert_eq!(
        &*world.route_trace.borrow(),
        &["D8:1->2", "raw-exile", "base:2"]
    );
}

#[test]
fn army_route_skips_known_null_and_kind4_before_indexing_but_refuses_invalid_indices() {
    let mut graph = route_graph(2, &[(1, 2, Some(0), false)]);
    let army = army();
    let world = World {
        exile_raw: Ok(1),
        ..Default::default()
    };
    graph.edges[0].to = super::super::NONE;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(false)
    );
    graph.edges[0].to = 99;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Err("selected army neighbour index outside graph".into())
    );
    let mut graph = route_graph(2, &[(1, 2, Some(100), true)]);
    graph.edges[0].to = 99;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(false)
    );
}

#[test]
fn army_route_adjacent_target_returns_before_later_undefined_land_kind() {
    let mut graph = route_graph(3, &[(1, 2, Some(0), false), (1, 3, Some(100), false)]);
    graph.kinds[2] = None;
    let army = army();
    let world = World::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 2),
        Ok(true)
    );
    assert_eq!(
        &*world.route_trace.borrow(),
        &["D8:1->2", "raw-exile", "canonical:1", "state-owner:2"]
    );
}

#[test]
fn army_route_runs_d0_area_graph_and_discards_only_later_target_callback_failure() {
    let graph = route_graph(3, &[]);
    let army = army();
    let node = |label, first, index, neighbours| AreaNode {
        label,
        first_land_province: Ok(first),
        nonsea_index: Ok(index),
        land_count: Ok(1),
        neighbors: Ok(neighbours),
    };
    let mut target = node(30, 3, 2, vec![]);
    target.land_count = Err("overwritten later target callback operand".into());
    let world = World {
        d0: BTreeMap::from([(1, Ok(10)), (3, Ok(30))]),
        nodes: BTreeMap::from([
            (10, Ok(node(10, 1, 0, vec![20]))),
            (20, Ok(node(20, 2, 1, vec![30]))),
            (30, Ok(target)),
        ]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 3),
        Ok(true)
    );
    assert!(
        world
            .route_trace
            .borrow()
            .ends_with(&["area:20".into(), "area:30".into()])
    );
    let mut world = world;
    world
        .nodes
        .get_mut(&10)
        .unwrap()
        .as_mut()
        .unwrap()
        .neighbors = Ok(vec![30]);
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_army_path(&unit(&army), 3),
        Err("overwritten later target callback operand".into())
    );
}

#[test]
fn railway_route_passes_current_raw_exile_and_keeps_predicate_gate_order() {
    let graph = route_graph(2, &[(1, 2, Some(100), false)]);
    let army = Army {
        exiled: true,
        ..army()
    };
    for (exile, expected) in [(0, false), (1, true), (2, true)] {
        let world = World {
            exile_raw: Ok(exile),
            controller: 5,
            rail_owners: BTreeMap::from([((1, 5, exile), Ok(expected))]),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .selected_railway_path(&railway_unit(&army), 2),
            Ok(expected)
        );
        let mut expected_calls = vec![
            "canonical:1".to_owned(),
            "raw-exile".to_owned(),
            "rail-edge:1->2".to_owned(),
            "controller:2".to_owned(),
            format!("rail-owner:1->5:{exile}"),
        ];
        if expected {
            expected_calls.push("demilitarized:2".into());
        }
        assert_eq!(*world.route_trace.borrow(), expected_calls);
    }
    let world = World {
        rail_retreat_edge: Ok(false),
        controllers: BTreeMap::from([(2, Err("unreached controller".into()))]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 2),
        Ok(false)
    );
    assert_eq!(
        &*world.route_trace.borrow(),
        &["canonical:1", "raw-exile", "rail-edge:1->2"]
    );
}

#[test]
fn railway_route_demilitarized_destination_demands_source_only_when_reached() {
    let graph = route_graph(2, &[(1, 2, Some(100), false)]);
    let army = army();
    let world = World {
        demilitarized: BTreeMap::from([(1, Err("unreached source state".into())), (2, Ok(false))]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 2),
        Ok(true)
    );
    for source in [false, true] {
        let world = World {
            demilitarized: BTreeMap::from([(1, Ok(source)), (2, Ok(true))]),
            ..Default::default()
        };
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .selected_railway_path(&railway_unit(&army), 2),
            Ok(source)
        );
        assert!(
            world
                .route_trace
                .borrow()
                .ends_with(&["demilitarized:2".into(), "demilitarized:1".into()])
        );
    }
}

#[test]
fn railway_route_uses_first_repeated_adjacency_and_accepts_zero_distance() {
    let army = army();
    for (first, second, expected) in [(0, None, true), (-1, Some(100), false)] {
        let graph = route_graph(2, &[(1, 2, Some(first), false), (1, 2, second, false)]);
        let world = World::default();
        assert_eq!(
            Mover {
                graph: &graph,
                world: &world
            }
            .selected_railway_path(&railway_unit(&army), 2),
            Ok(expected)
        );
        assert_eq!(
            world
                .route_trace
                .borrow()
                .iter()
                .filter(|call| *call == "rail-edge:1->2")
                .count(),
            2
        );
    }
    let graph = route_graph(2, &[(1, 2, None, false), (1, 2, Some(100), false)]);
    assert_eq!(
        Mover {
            graph: &graph,
            world: &World::default()
        }
        .selected_railway_path(&railway_unit(&army), 2),
        Err("selected rail adjacency distance unavailable".into())
    );
}

#[test]
fn railway_route_heap_bubbles_equal_cost_later_neighbour_before_lower_id() {
    let army = army();
    let world = World {
        rail_edges: BTreeMap::from([((3, 4), Err("equal-cost first branch".into()))]),
        ..Default::default()
    };
    let graph = route_graph(
        4,
        &[
            (1, 2, Some(100), false),
            (1, 3, Some(100), false),
            (2, 4, Some(0), false),
            (3, 4, Some(0), false),
        ],
    );
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 4),
        Err("equal-cost first branch".into())
    );
    let graph = route_graph(
        4,
        &[
            (1, 3, Some(100), false),
            (1, 2, Some(100), false),
            (2, 4, Some(0), false),
            (3, 4, Some(0), false),
        ],
    );
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 4),
        Ok(true)
    );
}

#[test]
fn railway_route_wraps_map_position_before_integer_heuristic() {
    let mut graph = route_graph(
        4,
        &[
            (1, 2, Some(0), false),
            (1, 3, Some(0), false),
            (2, 4, Some(0), false),
            (3, 4, Some(0), false),
        ],
    );
    graph.centroids[1] = Some([99, 0]);
    graph.centroids[2] = Some([3, 0]);
    let army = army();
    let world = World {
        rail_edges: BTreeMap::from([((3, 4), Err("unreached unwrapped branch".into()))]),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 4),
        Ok(true)
    );
}

#[test]
fn railway_route_unseen_cost_bound_rejects_before_intermediate_position() {
    let army = army();
    for (cost, expected) in [
        (RAILWAY_UNSEEN_COST - 1, true),
        (RAILWAY_UNSEEN_COST, false),
        (RAILWAY_UNSEEN_COST + 1, false),
    ] {
        let graph = route_graph(2, &[(1, 2, Some(cost), false)]);
        assert_eq!(
            Mover {
                graph: &graph,
                world: &World::default()
            }
            .selected_railway_path(&railway_unit(&army), 2),
            Ok(expected)
        );
    }
    let mut graph = route_graph(3, &[(1, 2, Some(RAILWAY_UNSEEN_COST), false)]);
    graph.centroids[1] = None;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &World::default()
        }
        .selected_railway_path(&railway_unit(&army), 3),
        Ok(false)
    );
}

#[test]
fn railway_route_equal_endpoint_resolves_owner_before_exile_and_target_position() {
    let mut graph = route_graph(2, &[]);
    graph.centroids[0] = None;
    let army = army();
    let world = World {
        canonical_failure: Some("owner resolution unavailable".into()),
        exile_raw: Err("later raw exile unavailable".into()),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("owner resolution unavailable".into())
    );
    assert_eq!(&*world.route_trace.borrow(), &["canonical:1"]);

    let world = World {
        exile_raw: Err("later raw exile unavailable".into()),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("later raw exile unavailable".into())
    );
    assert_eq!(&*world.route_trace.borrow(), &["canonical:1", "raw-exile"]);

    let world = World::default();
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("selected rail target map position unavailable".into())
    );
    assert_eq!(&*world.route_trace.borrow(), &["canonical:1", "raw-exile"]);
}

#[test]
fn railway_route_equal_endpoint_still_requires_current_exile_and_null_neighbour_refuses() {
    let graph = route_graph(2, &[]);
    let army = army();
    let world = World {
        exile_raw: Err("current raw exile unavailable".into()),
        ..Default::default()
    };
    assert_eq!(
        Mover {
            graph: &graph,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("current raw exile unavailable".into())
    );
    assert_eq!(
        Mover {
            graph: &graph,
            world: &World::default()
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Ok(true)
    );
    let mut no_position = graph;
    no_position.centroids[0] = None;
    assert_eq!(
        Mover {
            graph: &no_position,
            world: &World::default()
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("selected rail target map position unavailable".into())
    );
    assert_eq!(
        Mover {
            graph: &no_position,
            world: &world
        }
        .selected_railway_path(&railway_unit(&army), 1),
        Err("current raw exile unavailable".into())
    );
    let mut graph = route_graph(2, &[(1, 2, Some(0), false)]);
    graph.edges[0].to = super::super::NONE;
    assert_eq!(
        Mover {
            graph: &graph,
            world: &World::default()
        }
        .selected_railway_path(&railway_unit(&army), 2),
        Err("selected rail null or undefined neighbour".into())
    );
}

#[test]
fn railway_integer_newton_heuristic_keeps_truncation_and_wrapped_negative_square() {
    for (square, expected) in [
        (0, 0),
        (1, 100_000),
        (2, 100_000),
        (15, 300_000),
        (24, 400_000),
        (i32::MAX, 4_634_000_000),
        (-1, -100_000),
        (i32::MIN, -214_748_364_800_000),
    ] {
        assert_eq!(railway_heuristic(square), expected);
    }
}

mod cursor;
