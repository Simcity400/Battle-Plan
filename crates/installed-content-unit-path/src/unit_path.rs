
use std::{
    collections::{BTreeMap, HashMap},
    ops::Deref,
};

use crate::{
    adjacency_rules::{self, AdjacencyRule, Verdict},
    parser::Item,
    province_geometry::ProvinceGeometry,
    province_graph::{IMPASSABLE, ProvinceGraph, SEA_CROSSING},
};

mod counter;
mod invasion;
mod selected;
pub use counter::{CounterClass, CounterRequest};
pub use selected::transport as selected_transport;
pub use selected::cursor as order_cursor;
pub use selected::{
    AreaNode, CommonClass, CommonCombat, CommonMoveText, CommonMoveWorld, CommonUnit, MoveOptions,
    NativeArmyOccupant, Silent,
};

pub type Fact<T> = Result<T, String>;

pub type Country = u32;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArmyCombat {
    pub province: u32,
    pub land: bool,
    pub attacking: bool,
    pub defending: bool,
    pub attacker_locations: Vec<u32>,
    pub minimum_duration: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Army {
    pub location: u32,
    pub owner: Country,
    pub navy_country: Country,
    pub orders_group: u64,
    pub retreating: bool,
    pub naval_transfer: bool,
    pub exiled: bool,
    pub exfiltrates: bool,
    pub strength: i64,
    pub organisation: i64,
    pub route_first: Option<u32>,
    pub combats: Vec<ArmyCombat>,
    pub no_retreat_defending: bool,
    pub no_retreat_attacking: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ReachKey {
    pub location: u32,
    pub owner: Country,
    pub navy_country: Country,
    pub orders_group: u64,
    pub naval_transfer: bool,
    pub exiled: bool,
    pub exfiltrates: bool,
}

impl Army {
    pub fn reach_key(&self) -> ReachKey {
        ReachKey {
            location: self.location,
            owner: self.owner,
            navy_country: self.navy_country,
            orders_group: self.orders_group,
            naval_transfer: self.naval_transfer,
            exiled: self.exiled,
            exfiltrates: self.exfiltrates,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occupant {
    pub owner: Country,
    pub retreating: bool,
    pub strength: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StraitFleet {
    Visible,
    Unseen,
    None,
}

pub trait MoveWorld {
    fn path_friendly(&self, owner: Country, target: Country) -> Fact<bool>;
    fn at_war(&self, a: Country, b: Country) -> Fact<bool>;
    fn military_access(&self, a: Country, b: Country) -> Fact<bool>;
    fn docking_rights(&self, a: Country, b: Country) -> Fact<bool> {
        Err(format!("counter docking rights {a}->{b} are unavailable"))
    }
    fn subject_tie(&self, a: Country, b: Country) -> Fact<bool>;
    fn faction(&self, country: Country) -> Fact<Option<u64>>;
    fn overlord(&self, country: Country) -> Fact<Country>;
    fn can_occupy_non_war(&self, country: Country) -> Fact<bool>;
    fn enemies(&self, country: Country) -> Fact<Vec<Country>>;
    fn controller(&self, province: u32) -> Fact<Country>;
    fn controller_raw_positive(&self, province: u32) -> Fact<bool>;
    fn state_owner(&self, province: u32) -> Fact<Country>;
    fn state_owner_raw_positive(&self, province: u32) -> Fact<bool>;
    fn demilitarized(&self, province: u32) -> Fact<bool>;
    fn impassable(&self, province: u32) -> Fact<bool>;
    fn owner_area_missing(&self, province: u32) -> Fact<bool>;
    fn border_conflict(&self, province: u32) -> Fact<bool>;
    fn border_conflict_admits(&self, province: u32, orders_group: u64) -> Fact<bool>;
    fn has_naval_base(&self, province: u32) -> Fact<bool>;
    fn naval_base_active(&self, province: u32) -> Fact<bool>;
    fn counter_rail_connected(&self, from: u32, to: u32) -> Fact<bool> {
        Err(format!(
            "counter railway connection {from}->{to} is unavailable"
        ))
    }
    fn counter_railway_orders_group(&self, handle: u64) -> Fact<u64> {
        Err(format!(
            "counter railway gun {handle} orders group is unavailable"
        ))
    }
    fn counter_task_force_access(&self, handle: u64, to: u32) -> Fact<bool> {
        Err(format!(
            "counter task force {handle} access to {to} is unavailable"
        ))
    }
    fn counter_task_force_submarine(&self, handle: u64) -> Fact<bool> {
        Err(format!(
            "counter task force {handle} submarine predicate is unavailable"
        ))
    }
    fn counter_task_force_hostile_allows(&self, handle: u64, to: u32) -> Fact<bool> {
        Err(format!(
            "counter task force {handle} hostile strength at {to} is unavailable"
        ))
    }
    fn port_sea(&self, province: u32) -> Fact<u32>;
    fn armies(&self, province: u32) -> Fact<Vec<Occupant>>;
    fn land_combat(&self, province: u32) -> Fact<bool>;
    fn strait_fleet(&self, through: u32, owner: Country) -> Fact<StraitFleet>;
    fn strategic_region(&self, province: u32) -> Option<u32>;
    fn region_access(&self, country: Country, region: u32) -> Fact<u8>;
    fn evaluate(&self, triggers: &[Item], country: Country) -> Fact<bool>;
    fn has_children(&self, triggers: &[Item]) -> Fact<bool>;
}

fn and(first: Fact<bool>, second: impl FnOnce() -> Fact<bool>) -> Fact<bool> {
    match first {
        Ok(false) => Ok(false),
        Ok(true) => second(),
        Err(reason) => match second() {
            Ok(false) => Ok(false),
            _ => Err(reason),
        },
    }
}

fn or(first: Fact<bool>, second: impl FnOnce() -> Fact<bool>) -> Fact<bool> {
    match first {
        Ok(true) => Ok(true),
        Ok(false) => second(),
        Err(reason) => match second() {
            Ok(true) => Ok(true),
            _ => Err(reason),
        },
    }
}

pub fn discloses<W: MoveWorld + ?Sized>(
    world: &W,
    country: Country,
    other: Country,
    access: bool,
) -> Fact<bool> {
    if country == other {
        return Ok(true);
    }
    if world.at_war(country, other)? {
        return Ok(false);
    }
    if access && world.military_access(country, other)? {
        return Ok(true);
    }
    if let Some(faction) = world.faction(country)?
        && world.faction(other)? == Some(faction)
    {
        return Ok(true);
    }
    world.subject_tie(country, other)
}

const NONE: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cost {
    Zero,
    Moving,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Strait {
    None,
    Through(u32),
    Invalid(String),
}

#[derive(Clone, Debug)]
struct Crossing {
    impassable: bool,
    strait: Strait,
    rule: Option<Result<usize, String>>,
}

#[derive(Clone, Copy, Debug)]
struct Edge {
    to: u32,
    cost: Cost,
    crossing: u32,
    distance: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectedNeighbour {
    pub through: Fact<Option<u32>>,
}

#[derive(Clone, Debug, Default)]
pub struct MoveGraph {
    width: i32,
    centroids: Vec<Option<[i32; 2]>>,
    index: Vec<u32>,
    ids: Vec<u32>,
    kinds: Vec<Option<u8>>,
    land_masses: Vec<u16>,
    land_mass_refusal: Option<String>,
    ranges: Vec<Option<(u32, u32)>>,
    edges: Vec<Edge>,
    crossings: Vec<Crossing>,
    pairs: HashMap<(u32, u32), u32>,
    rules: Vec<AdjacencyRule>,
    unavailable: BTreeMap<(u32, u32), String>,
}

impl MoveGraph {
    pub fn selected_land(&self, province: u32) -> Fact<bool> {
        Ok(self.kind(province)? & 1 != 0)
    }
    pub fn selected_neighbour_index(&self, from: u32, to: u32) -> Fact<Option<usize>> {
        let source = self
            .dense(from)
            .ok_or_else(|| format!("province {from} has no neighbour domain"))?;
        let neighbours = self
            .neighbours(source)
            .ok_or_else(|| format!("province {from} has no neighbour domain"))?;
        Ok(neighbours
            .iter()
            .position(|edge| self.ids[edge.to as usize] == to))
    }

    pub fn selected_neighbour(&self, from: u32, to: u32) -> Fact<Option<SelectedNeighbour>> {
        let source = self
            .dense(from)
            .ok_or_else(|| format!("province {from} has no neighbour domain"))?;
        let neighbours = self
            .neighbours(source)
            .ok_or_else(|| format!("province {from} has no neighbour domain"))?;
        let Some(edge) = neighbours
            .iter()
            .find(|edge| self.ids[edge.to as usize] == to)
        else {
            return Ok(None);
        };
        let through = if edge.crossing == NONE {
            Ok(None)
        } else {
            match &self.crossings[edge.crossing as usize].strait {
                Strait::None => Ok(None),
                Strait::Through(id) => Ok(Some(*id)),
                Strait::Invalid(reason) => Err(reason.clone()),
            }
        };
        Ok(Some(SelectedNeighbour { through }))
    }

    pub fn debug_values(&self) -> impl std::fmt::Debug + '_ {
        struct View<'a>(&'a MoveGraph);
        impl std::fmt::Debug for View<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct("MoveGraph")
                    .field("width", &self.0.width)
                    .field("centroids", &self.0.centroids)
                    .field("index", &self.0.index)
                    .field("ids", &self.0.ids)
                    .field("kinds", &self.0.kinds)
                    .field("land_masses", &self.0.land_masses)
                    .field("land_mass_refusal", &self.0.land_mass_refusal)
                    .field("ranges", &self.0.ranges)
                    .field("edges", &self.0.edges)
                    .field("crossings", &self.0.crossings)
                    .field("pairs", &self.0.pairs.iter().collect::<BTreeMap<_, _>>())
                    .field("rules", &self.0.rules)
                    .field("unavailable", &self.0.unavailable)
                    .finish()
            }
        }
        View(self)
    }

    pub fn new(graph: &ProvinceGraph, geometry: &ProvinceGeometry) -> Self {
        let mut ids: Vec<u32> = geometry
            .centroids
            .keys()
            .copied()
            .chain(graph.edges.keys().copied())
            .chain(graph.edges.values().flatten().map(|&(to, _)| to))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let mut index = vec![NONE; ids.last().map_or(0, |&id| id as usize + 1)];
        for (at, &id) in ids.iter().enumerate() {
            index[id as usize] = at as u32;
        }
        let kinds = ids.iter().map(|&id| geometry.kind_flags(id)).collect();
        let mut pairs = HashMap::new();
        let mut crossings = Vec::new();
        for (&pair, crossing) in &graph.crossings {
            let strait = if crossing.kind != SEA_CROSSING || crossing.through == 0 {
                Strait::None
            } else {
                match u32::try_from(crossing.through) {
                    Err(_) => Strait::Invalid(format!(
                        "strait through province {} is not a province",
                        crossing.through
                    )),
                    Ok(through) => match geometry.kind_flags(through) {
                        None => Strait::Invalid(format!("province {through} is not defined")),
                        Some(kind) if kind & 3 == 0 => Strait::Through(through),
                        Some(_) => Strait::None,
                    },
                }
            };
            let rule = crossing.rule.as_deref().map(|name| {
                graph
                    .rules
                    .iter()
                    .position(|rule| rule.name == name)
                    .ok_or_else(|| format!("adjacency rule {name} is not loaded"))
            });
            pairs.insert(pair, crossings.len() as u32);
            crossings.push(Crossing {
                impassable: crossing.kind == IMPASSABLE,
                strait,
                rule,
            });
        }
        let mut ranges = vec![None; ids.len()];
        let mut edges = Vec::new();
        for (&from, neighbours) in &graph.edges {
            let start = edges.len() as u32;
            for &(to, cost) in neighbours {
                edges.push(Edge {
                    distance: cost,
                    to: index[to as usize],
                    cost: match cost {
                        Some(0) => Cost::Zero,
                        Some(_) => Cost::Moving,
                        None => Cost::Unknown,
                    },
                    crossing: pairs.get(&(from, to)).copied().unwrap_or(NONE),
                });
            }
            ranges[index[from as usize] as usize] = Some((start, edges.len() as u32));
        }
        let mut result = Self {
            width: geometry.width,
            centroids: ids
                .iter()
                .map(|id| geometry.centroids.get(id).copied())
                .collect(),
            index,
            ids,
            kinds,
            land_masses: Vec::new(),
            land_mass_refusal: None,
            ranges,
            edges,
            crossings,
            pairs,
            rules: graph.rules.clone(),
            unavailable: graph.unavailable.clone(),
        };
        match result.derive_land_masses() {
            Ok(labels) => result.land_masses = labels,
            Err(reason) => result.land_mass_refusal = Some(reason),
        }
        result
    }

    fn derive_land_masses(&self) -> Fact<Vec<u16>> {
        let mut labels = vec![0; self.ids.len()];
        let mut seen = vec![false; self.ids.len()];
        let mut label = 0_u16;
        let mut pending = Vec::new();
        for at in 0..self.ids.len() {
            let id = self.ids[at];
            if id == 0 || seen[at] || self.kind(id)? & 1 == 0 {
                continue;
            }
            label = label.wrapping_add(1);
            pending.push(at);
            seen[at] = true;
            while let Some(current) = pending.pop() {
                labels[current] = label;
                for edge in self.neighbours(current).ok_or_else(|| {
                    format!("land mass province {} has no neighbours", self.ids[current])
                })? {
                    let next = edge.to as usize;
                    if seen[next]
                        || self.kind(self.ids[next])? & 1 == 0
                        || (edge.crossing != NONE
                            && self.crossings[edge.crossing as usize].impassable)
                    {
                        continue;
                    }
                    seen[next] = true;
                    pending.push(next);
                }
            }
        }
        Ok(labels)
    }

    fn land_mass(&self, province: u32) -> Fact<u16> {
        if let Some(reason) = &self.land_mass_refusal {
            return Err(reason.clone());
        }
        self.dense(province)
            .and_then(|at| self.land_masses.get(at).copied())
            .ok_or_else(|| format!("province {province} has no land mass"))
    }

    fn dense(&self, province: u32) -> Option<usize> {
        self.index
            .get(province as usize)
            .filter(|&&at| at != NONE)
            .map(|&at| at as usize)
    }

    fn kind(&self, province: u32) -> Fact<u8> {
        self.dense(province)
            .and_then(|at| self.kinds[at])
            .ok_or_else(|| format!("province {province} is not defined"))
    }

    fn neighbours(&self, at: usize) -> Option<&[Edge]> {
        self.ranges[at].map(|(start, end)| &self.edges[start as usize..end as usize])
    }

    fn crossing(&self, from: u32, to: u32) -> Option<&Crossing> {
        self.pairs
            .get(&(from, to))
            .map(|&at| &self.crossings[at as usize])
    }

    pub fn rules(&self) -> &[AdjacencyRule] {
        &self.rules
    }
}

const UNSEEN: u8 = 0;
const POSSIBLE: u8 = 1;
const REACHED: u8 = 2;

#[derive(Clone, Debug)]
pub struct Reach {
    key: ReachKey,
    state: Vec<u8>,
    unknown: Option<String>,
    open: Option<String>,
}

impl Reach {
    fn note(&mut self, reason: String) {
        self.unknown.get_or_insert(reason);
    }

    fn answer(&self, graph: &MoveGraph, province: u32) -> Fact<bool> {
        let state = graph.dense(province).map_or(UNSEEN, |at| self.state[at]);
        match state {
            REACHED => Ok(true),
            POSSIBLE => Err(self
                .unknown
                .clone()
                .unwrap_or_else(|| "a path fact is unknown".into())),
            _ => self.open.clone().map_or(Ok(false), Err),
        }
    }
}

struct Memo {
    nodes: Vec<Option<Fact<bool>>>,
    rules: Vec<Option<Fact<bool>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleReason {
    Disabled,
    Contested(Country, Country),
    Held(Verdict),
    Excluded(Verdict),
    Enemy(Country),
    NotEnemy,
    Friend(Country),
    NotFriendly,
    Neutral,
    NotNeutral,
}

impl RuleReason {
    pub fn verdict(self) -> Option<Verdict> {
        Some(match self {
            Self::Disabled => return None,
            Self::Contested(..) => Verdict::Contested,
            Self::Held(verdict) | Self::Excluded(verdict) => verdict,
            Self::Enemy(_) | Self::NotNeutral => Verdict::Enemy,
            Self::Friend(_) => Verdict::Friend,
            Self::NotEnemy | Self::NotFriendly | Self::Neutral => Verdict::Neutral,
        })
    }
}

pub struct Mover<'a, W: MoveWorld + ?Sized> {
    pub graph: &'a MoveGraph,
    pub world: &'a W,
}

impl<'a, W: MoveWorld + ?Sized> Mover<'a, W> {
    fn is_sea(&self, province: u32) -> Fact<bool> {
        Ok(self.graph.kind(province)? & 3 == 0)
    }

    fn is_land(&self, province: u32) -> Fact<bool> {
        Ok(self.graph.kind(province)? & 1 != 0)
    }

    #[cfg(test)]
    fn army_reaches(&self, army: &Army, source: u32, avoid_enemy: bool) -> Fact<bool> {
        let reachable = self.reachable(army.exfiltrates);
        self.army_reaches_resumed(army, source, avoid_enemy, &mut self.search(army), || {
            &reachable
        })
    }

    pub fn reach(&self, army: &Army) -> Reach {
        let mut search = self.search(army);
        self.advance(army, &mut search, None);
        search.reach
    }

    pub fn army_reaches_resumed<'r>(
        &self,
        army: &Army,
        source: u32,
        avoid_enemy: bool,
        search: &mut Search,
        reachable: impl FnOnce() -> &'r Reachable,
    ) -> Fact<bool> {
        match self.gate(army, source, avoid_enemy)? {
            Some(answer) => Ok(answer),
            None => {
                if search.reach.key != army.reach_key() {
                    return Err("the reach was searched for another army".into());
                }
                if reachable().apart(self.graph, army.location, source) {
                    return Ok(false);
                }
                self.advance(army, search, self.graph.dense(source));
                search.reach.answer(self.graph, source)
            }
        }
    }

    pub fn reachable(&self, exfiltrates: bool) -> Reachable {
        let graph = self.graph;
        let mut parent: Vec<u32> = (0..graph.ids.len() as u32).collect();
        fn root(parent: &mut [u32], mut at: u32) -> u32 {
            while parent[at as usize] != at {
                parent[at as usize] = parent[parent[at as usize] as usize];
                at = parent[at as usize];
            }
            at
        }
        for from in 0..graph.ids.len() {
            for edge in graph.neighbours(from).into_iter().flatten() {
                let to = edge.to as usize;
                let refused = edge.to == NONE
                    || to == from
                    || edge.cost == Cost::Zero
                    || graph
                        .crossings
                        .get(edge.crossing as usize)
                        .is_some_and(|crossing| crossing.impassable)
                    || self.class_refused(exfiltrates, from, to);
                if !refused {
                    let (a, b) = (root(&mut parent, from as u32), root(&mut parent, edge.to));
                    parent[a as usize] = b;
                }
            }
        }
        let component: Vec<u32> = (0..graph.ids.len() as u32)
            .map(|at| root(&mut parent, at))
            .collect();
        let mut open = vec![false; graph.ids.len()];
        for (at, range) in graph.ranges.iter().enumerate() {
            if range.is_none() {
                open[component[at] as usize] = true;
            }
        }
        Reachable { component, open }
    }

    fn class_refused(&self, exfiltrates: bool, from: usize, to: usize) -> bool {
        let world = self.world;
        let (from, to) = (self.graph.ids[from], self.graph.ids[to]);
        let (Ok(from_land), Ok(to_land)) = (self.is_land(from), self.is_land(to)) else {
            return false;
        };
        let admits = match (from_land, to_land) {
            (true, false) => (|| {
                Ok::<_, String>(
                    world.port_sea(from)? == to
                        && (exfiltrates || world.naval_base_active(from)?),
                )
            })(),
            (false, true) => {
                (|| Ok::<_, String>(world.port_sea(to)? == from && world.naval_base_active(to)?))()
            }
            _ => Ok(true),
        };
        admits == Ok(false)
    }

    pub fn army_reaches_in<R: Deref<Target = Reach>>(
        &self,
        army: &Army,
        source: u32,
        avoid_enemy: bool,
        reach: impl FnOnce() -> R,
    ) -> Fact<bool> {
        match self.gate(army, source, avoid_enemy)? {
            Some(answer) => Ok(answer),
            None => {
                let reach = reach();
                if reach.key != army.reach_key() {
                    return Err("the reach was searched for another army".into());
                }
                reach.answer(self.graph, source)
            }
        }
    }

    fn gate(&self, army: &Army, source: u32, avoid_enemy: bool) -> Fact<Option<bool>> {
        if !self.check_move_admission(army, source, avoid_enemy)? {
            return Ok(Some(false));
        }
        if army.location == 0 {
            return Ok(Some(false));
        }
        if army.location == source {
            return Ok(Some(true));
        }
        if self.is_sea(army.location)? {
            return Ok(Some(false));
        }
        if !self.province_access_for_move(army, source, false)?
            || !self.node_admissible(army, source)?
        {
            return Ok(Some(false));
        }
        Ok(None)
    }

    pub fn check_move_admission(&self, army: &Army, to: u32, avoid_enemy: bool) -> Fact<bool> {
        let world = self.world;
        if !self.can_move_to_province(army, to)? {
            return Ok(false);
        }
        for combat in army.combats.iter().filter(|combat| combat.land) {
            if combat.defending && combat.attacker_locations.contains(&to) {
                return Ok(false);
            }
        }
        if army.naval_transfer {
            return Ok(false);
        }
        let controller = world.controller(to)?;
        if self.is_sea(army.location)? && world.at_war(controller, army.owner)? {
            return Ok(false);
        }
        if self.can_advance(army) || !self.enemy_army_in(army, to)? {
            return Ok(!(avoid_enemy && world.at_war(army.owner, controller)?));
        }
        Ok(false)
    }

    fn can_move_to_province(&self, army: &Army, to: u32) -> Fact<bool> {
        if to == army.location {
            return Ok(true);
        }
        if army.retreating {
            return Ok(false);
        }
        if !army.combats.is_empty()
            && !self.can_disengage(army)
            && !self.world.at_war(self.world.controller(to)?, army.owner)?
        {
            return Ok(false);
        }
        if !self.can_enter_province(army, to)? {
            return Ok(false);
        }
        if self.is_sea(army.location)? && self.is_land(to)? {
            return Ok(self.passable_adjacency(to, army.location));
        }
        Ok(true)
    }

    fn passable_adjacency(&self, of: u32, other: u32) -> bool {
        let graph = self.graph;
        let Some((start, end)) = graph.dense(of).and_then(|at| graph.ranges[at]) else {
            return false;
        };
        graph.edges[start as usize..end as usize]
            .iter()
            .any(|edge| edge.to != NONE && graph.ids[edge.to as usize] == other)
            && graph.crossing(of, other).is_none_or(|c| !c.impassable)
    }

    fn can_disengage(&self, army: &Army) -> bool {
        if army.no_retreat_defending && army.combats.iter().any(|combat| combat.defending) {
            return false;
        }
        if army.no_retreat_attacking && army.combats.iter().any(|combat| combat.attacking) {
            return false;
        }
        army.combats.iter().any(|combat| combat.minimum_duration)
    }

    fn can_advance(&self, army: &Army) -> bool {
        army.strength >= 10_000 && army.organisation >= 100_000
    }

    fn enemy_army_in(&self, army: &Army, province: u32) -> Fact<bool> {
        for occupant in self.world.armies(province)? {
            if !occupant.retreating && self.world.at_war(occupant.owner, army.owner)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn can_enter_province(&self, army: &Army, to: u32) -> Fact<bool> {
        let world = self.world;
        if !self.province_access_for_move(army, to, false)? {
            return Ok(false);
        }
        let controller = world.controller(to)?;
        if !army.combats.is_empty()
            && !world.at_war(controller, army.owner)?
            && !self.can_retreat_to(army, to)?
        {
            return Ok(false);
        }
        if let Some(first) = army.route_first
            && !army.combats.is_empty()
            && army.combats.iter().any(|combat| combat.province == first)
            && !self.can_disengage(army)
        {
            return Ok(false);
        }
        if world.demilitarized(to)? {
            return Ok(false);
        }
        if world.border_conflict(to)? && !world.border_conflict_admits(to, army.orders_group)? {
            return Ok(false);
        }
        if world.impassable(to)? {
            return Ok(false);
        }
        if !army.retreating {
            for occupant in world.armies(to)? {
                if !occupant.retreating && world.at_war(occupant.owner, army.owner)? {
                    if !self.can_advance(army) {
                        return Ok(false);
                    }
                    break;
                }
            }
        }
        if army.owner != controller && world.at_war(army.owner, controller)? {
            let state_owner = world.state_owner(to)?;
            if !self.discloses(army.owner, state_owner, false)?
                && !world.military_access(army.owner, state_owner)?
                && !self.occupies_non_war(army.owner, state_owner)?
            {
                return Ok(false);
            }
        }
        match self
            .graph
            .crossing(army.location, to)
            .and_then(|c| c.rule.as_ref())
        {
            Some(rule) => self.rule_admits_army(self.rule(rule)?, army.owner),
            None => Ok(true),
        }
    }

    fn can_retreat_to(&self, army: &Army, to: u32) -> Fact<bool> {
        let world = self.world;
        if to == army.location || self.passable_adjacency(army.location, to) {
            if !world.at_war(world.controller(to)?, army.owner)? || army.organisation > 99_999 {
                let mut enemy: i64 = 0;
                for occupant in world.armies(to)? {
                    if world.at_war(occupant.owner, army.owner)? {
                        enemy = enemy.wrapping_add(occupant.strength);
                    }
                }
                return Ok(((enemy / 100_000) as i32) <= 0);
            }
            return Ok(false);
        }
        Ok(world.has_naval_base(to)? && (world.has_naval_base(army.location)? || army.exfiltrates))
    }

    fn province_access_for_move(&self, army: &Army, to: u32, naval: bool) -> Fact<bool> {
        self.province_access_for_move_with_group(army, to, naval, || Ok(army.orders_group))
    }

    fn province_access_for_move_with_group(
        &self,
        army: &Army,
        to: u32,
        naval: bool,
        group: impl FnOnce() -> Fact<u64>,
    ) -> Fact<bool> {
        let world = self.world;
        let kind = self.graph.kind(to)?;
        if kind & 3 == 0 {
            return Ok(army.naval_transfer || naval);
        }
        if kind & 2 != 0 || world.owner_area_missing(to)? {
            return Ok(false);
        }
        if world.border_conflict(to)?
            && !(army.location != 0 && world.border_conflict(army.location)?)
            && !world.border_conflict_admits(to, group()?)?
        {
            return Ok(false);
        }
        if world.demilitarized(to)? || world.impassable(to)? {
            return Ok(false);
        }
        let (here, there) = (world.controller(army.location)?, world.controller(to)?);
        if here != there {
            if army.exiled && world.state_owner(to)? == world.state_owner(army.location)? {
                return Ok(true);
            }
            return self.has_province_access(army.owner, to);
        }
        Ok(true)
    }

    fn has_province_access(&self, country: Country, province: u32) -> Fact<bool> {
        let world = self.world;
        let kind = self.graph.kind(province)?;
        if kind & 2 != 0 {
            return Ok(false);
        }
        if !world.state_owner_raw_positive(province)? {
            return Ok(true);
        }
        if world.state_owner(province)? == country {
            return Ok(true);
        }
        let controller = world.controller(province)?;
        if controller == country {
            return Ok(true);
        }
        if world.at_war(country, controller)? || world.military_access(country, controller)? {
            return Ok(true);
        }
        if kind & 3 == 0 {
            return Ok(false);
        }
        self.faction_or_overlord(country, controller)
    }

    fn faction_or_overlord(&self, country: Country, other: Country) -> Fact<bool> {
        let world = self.world;
        if let Some(faction) = world.faction(country)?
            && world.faction(other)? == Some(faction)
        {
            return Ok(true);
        }
        if world.overlord(other)? == country {
            return Ok(true);
        }
        Ok(world.overlord(country)? == other)
    }

    fn move_access(&self, country: Country, province: u32) -> Fact<bool> {
        self.move_access_output(country, province, false)
    }

    fn naval_move_access(&self, country: Country, province: u32) -> Fact<bool> {
        self.move_access_output(country, province, true)
    }

    fn move_access_output(&self, country: Country, province: u32, naval: bool) -> Fact<bool> {
        let world = self.world;
        let kind = self.graph.kind(province)?;
        if kind & 2 != 0 {
            return Ok(false);
        }
        if !world.state_owner_raw_positive(province)? {
            return Ok(true);
        }
        if world.state_owner(province)? == country {
            return Ok(true);
        }
        let controller = world.controller(province)?;
        if controller == country
            || world.at_war(country, controller)?
            || world.military_access(country, controller)?
        {
            return Ok(true);
        }
        if kind & 3 == 0 {
            return Ok(naval);
        }
        if self.faction_or_overlord(country, controller)? {
            return Ok(true);
        }
        if naval {
            world.docking_rights(country, controller)
        } else {
            Ok(false)
        }
    }

    fn discloses(&self, country: Country, other: Country, access: bool) -> Fact<bool> {
        discloses(self.world, country, other, access)
    }

    fn occupies_non_war(&self, country: Country, other: Country) -> Fact<bool> {
        let world = self.world;
        if country != other && country != 0 && world.at_war(country, other)? {
            return Ok(true);
        }
        if !world.can_occupy_non_war(country)? {
            return Ok(false);
        }
        let theirs = world.enemies(other)?;
        Ok(world
            .enemies(country)?
            .iter()
            .any(|enemy| theirs.contains(enemy)))
    }

    fn rule(&self, rule: &Result<usize, String>) -> Fact<&'a AdjacencyRule> {
        let rule = &self.graph.rules[*rule.as_ref().map_err(Clone::clone)?];
        match &rule.unsupported {
            Some(reason) => Err(format!("adjacency rule {} is refused: {reason}", rule.name)),
            None => Ok(rule),
        }
    }

    fn rule_admits_army(&self, rule: &AdjacencyRule, country: Country) -> Fact<bool> {
        Ok(rule.permits(self.verdict(rule, country)?, adjacency_rules::ARMY))
    }

    pub fn verdict(&self, rule: &AdjacencyRule, country: Country) -> Fact<Option<Verdict>> {
        Ok(self.relation(rule, country)?.verdict())
    }

    pub fn relation(&self, rule: &AdjacencyRule, country: Country) -> Fact<RuleReason> {
        self.relation_with(rule, country, &|first, second, friendly| {
            if friendly {
                self.discloses(first, second, true)
            } else {
                self.world.at_war(first, second)
            }
        })
    }

    pub(crate) fn relation_with(
        &self,
        rule: &AdjacencyRule,
        country: Country,
        matches: &impl Fn(Country, Country, bool) -> Fact<bool>,
    ) -> Fact<RuleReason> {
        let world = self.world;
        let holds = |triggers: &[Item]| -> Fact<bool> {
            Ok(world.has_children(triggers)? && world.evaluate(triggers, country)?)
        };
        if holds(&rule.is_disabled)? {
            return Ok(RuleReason::Disabled);
        }
        if let Some((first, second)) = self.contested(rule, matches)? {
            return Ok(RuleReason::Contested(first, second));
        }
        if holds(&rule.is_enemy)? {
            return Ok(RuleReason::Held(Verdict::Enemy));
        }
        if holds(&rule.is_friend)? {
            return Ok(RuleReason::Held(Verdict::Friend));
        }
        let (enemy, friend, neutral) = (
            self.world.has_children(&rule.is_enemy)?,
            self.world.has_children(&rule.is_friend)?,
            self.world.has_children(&rule.is_neutral)?,
        );
        if enemy && friend {
            return Ok(RuleReason::Excluded(Verdict::Neutral));
        }
        if holds(&rule.is_neutral)? {
            return Ok(RuleReason::Held(Verdict::Neutral));
        }
        Ok(if enemy && neutral {
            RuleReason::Excluded(Verdict::Friend)
        } else if friend && neutral {
            RuleReason::Excluded(Verdict::Enemy)
        } else {
            self.fallback_relation(rule, country, matches)?
        })
    }

    fn contested(
        &self,
        rule: &AdjacencyRule,
        matches: &impl Fn(Country, Country, bool) -> Fact<bool>,
    ) -> Fact<Option<(Country, Country)>> {
        let provinces = &rule.required_provinces;
        for (index, &first) in provinces.iter().enumerate() {
            let country = self.world.controller(first)?;
            for &second in &provinces[index + 1..] {
                if let Some(pair) =
                    self.chain_witness(country, self.world.controller(second)?, false, matches)?
                {
                    return Ok(Some(pair));
                }
            }
        }
        Ok(None)
    }

    fn fallback_relation(
        &self,
        rule: &AdjacencyRule,
        country: Country,
        matches: &impl Fn(Country, Country, bool) -> Fact<bool>,
    ) -> Fact<RuleReason> {
        let mut friend = None;
        for &province in &rule.required_provinces {
            let controller = self.world.controller(province)?;
            if let Some((_, enemy)) = self.chain_witness(country, controller, false, matches)? {
                return Ok(if !self.world.has_children(&rule.is_enemy)? {
                    RuleReason::Enemy(enemy)
                } else {
                    RuleReason::NotEnemy
                });
            }
            if friend.is_none() {
                friend = self
                    .chain_witness(country, controller, true, matches)?
                    .map(|(_, friend)| friend);
            }
        }
        Ok(match friend {
            Some(friend) if !self.world.has_children(&rule.is_friend)? => {
                RuleReason::Friend(friend)
            }
            Some(_) => RuleReason::NotFriendly,
            None if !self.world.has_children(&rule.is_neutral)? => RuleReason::Neutral,
            None => RuleReason::NotNeutral,
        })
    }

    fn chain_witness(
        &self,
        country: Country,
        other: Country,
        friendly: bool,
        matches: &impl Fn(Country, Country, bool) -> Fact<bool>,
    ) -> Fact<Option<(Country, Country)>> {
        let world = self.world;
        let mut theirs = vec![other];
        let mut next = world.overlord(other)?;
        while next != 0 {
            if theirs.contains(&next) {
                return Err(format!("country {next}'s overlord chain loops"));
            }
            theirs.push(next);
            next = world.overlord(next)?;
        }
        let mut current = country;
        let mut seen = Vec::new();
        loop {
            for &candidate in &theirs {
                if matches(current, candidate, friendly)? {
                    return Ok(Some((current, candidate)));
                }
            }
            seen.push(current);
            let overlord = world.overlord(current)?;
            if overlord == 0 {
                return Ok(None);
            }
            if seen.contains(&overlord) {
                return Err(format!("country {overlord}'s overlord chain loops"));
            }
            current = overlord;
        }
    }

    fn node_admissible(&self, army: &Army, to: u32) -> Fact<bool> {
        let world = self.world;
        if self.is_sea(to)?
            && let Some(region) = world.strategic_region(to)
            && world.strategic_region(army.location) != Some(region)
        {
            let mode = world.region_access(army.navy_country, region)?;
            if matches!(mode, 1 | 2) {
                return Ok(false);
            }
        }
        if world.at_war(army.owner, world.controller(to)?)? {
            return Ok(false);
        }
        Ok(!world.land_combat(to)?)
    }

    fn destination(&self, army: &Army, to: u32) -> Fact<bool> {
        and(self.node_admissible(army, to), || {
            and(
                or(Ok(army.exiled), || self.move_access(army.owner, to)),
                || self.province_access_for_move(army, to, true),
            )
        })
    }

    fn crosses_other_war(&self, owner: Country, from: u32, to: u32) -> Fact<bool> {
        let world = self.world;
        if !world.controller_raw_positive(from)? || !world.controller_raw_positive(to)? {
            return Ok(false);
        }
        let (here, there) = (world.controller(from)?, world.controller(to)?);
        Ok(here != there
            && world.at_war(owner, there)?
            && !world.at_war(here, there)?
            && !world.at_war(owner, here)?)
    }

    fn class_admits(&self, army: &Army, from: u32, to: u32, lands: [bool; 2]) -> Fact<bool> {
        let world = self.world;
        match lands {
            [true, true] | [false, false] => Ok(true),
            [false, true] => Ok(world.port_sea(to)? == from && world.naval_base_active(to)?),
            [true, false] => Ok(world.port_sea(from)? == to
                && (army.exfiltrates || world.naval_base_active(from)?)),
        }
    }

    fn admit(&self, army: &Army, memo: &mut Memo, from: usize, edge: &Edge) -> Fact<bool> {
        let graph = self.graph;
        if edge.cost == Cost::Zero {
            return Ok(false);
        }
        let crossing = graph.crossings.get(edge.crossing as usize);
        if crossing.is_some_and(|c| c.impassable) || edge.to as usize == from {
            return Ok(false);
        }
        let (from_id, to_id) = (graph.ids[from], graph.ids[edge.to as usize]);
        let mut answer = match edge.cost {
            Cost::Unknown => Err(format!(
                "edge {from_id}->{to_id} distance is unavailable: {}",
                graph
                    .unavailable
                    .get(&(from_id.min(to_id), from_id.max(to_id)))
                    .map_or("its terrain cost is unavailable", String::as_str)
            )),
            _ => Ok(true),
        };
        let node = memo.nodes[edge.to as usize]
            .get_or_insert_with(|| self.destination(army, to_id))
            .clone();
        answer = and(answer, || node);
        if answer == Ok(false) {
            return answer;
        }
        answer = and(answer, || {
            let [from_land, to_land] = [self.is_land(from_id)?, self.is_land(to_id)?];
            if from_land && to_land && self.crosses_other_war(army.owner, from_id, to_id)? {
                return Ok(false);
            }
            self.class_admits(army, from_id, to_id, [from_land, to_land])
        });
        let Some(crossing) = crossing else {
            return answer;
        };
        if let Some(rule) = &crossing.rule {
            let index = rule.as_ref().ok().copied();
            let permits = match index.and_then(|at| memo.rules[at].clone()) {
                Some(permits) => permits,
                None => {
                    let permits = self
                        .rule(rule)
                        .and_then(|rule| self.rule_admits_army(rule, army.owner));
                    if let Some(at) = index {
                        memo.rules[at] = Some(permits.clone());
                    }
                    permits
                }
            };
            answer = and(answer, || permits);
        }
        and(answer, || match &crossing.strait {
            Strait::None => Ok(true),
            Strait::Through(through) => {
                Ok(self.world.strait_fleet(*through, army.owner)? != StraitFleet::Visible)
            }
            Strait::Invalid(reason) => Err(reason.clone()),
        })
    }

    pub fn search(&self, army: &Army) -> Search {
        let graph = self.graph;
        let mut search = Search {
            reach: Reach {
                key: army.reach_key(),
                state: vec![UNSEEN; graph.ids.len()],
                unknown: None,
                open: None,
            },
            memo: Memo {
                nodes: vec![None; graph.ids.len()],
                rules: vec![None; graph.rules.len()],
            },
            queue: Vec::new(),
            head: 0,
            edge: 0,
            possible: Vec::new(),
            unproved: false,
            expanding: None,
        };
        match graph.dense(army.location) {
            Some(start) => {
                search.reach.state[start] = REACHED;
                search.queue.push(start);
            }
            None => {
                search.reach.open =
                    Some(format!("province {} has no neighbour table", army.location));
            }
        }
        search
    }

    pub fn advance_by(&self, army: &Army, search: &mut Search, edges: usize) -> bool {
        let mut budget = edges;
        self.advance_within(army, search, None, &mut budget)
    }

    fn advance(&self, army: &Army, search: &mut Search, target: Option<usize>) {
        let mut unbounded = usize::MAX;
        self.advance_within(army, search, target, &mut unbounded);
    }

    fn advance_within(
        &self,
        army: &Army,
        search: &mut Search,
        target: Option<usize>,
        budget: &mut usize,
    ) -> bool {
        let graph = self.graph;
        let open = |reach: &mut Reach, at: usize| {
            reach.open.get_or_insert_with(|| {
                format!("province {} has no neighbour table", graph.ids[at])
            });
        };
        let Search {
            reach,
            memo,
            queue,
            head,
            edge,
            possible,
            unproved,
            expanding,
        } = search;
        while let Some(&at) = queue.get(*head) {
            let Some(edges) = graph.neighbours(at) else {
                open(reach, at);
                (*head, *edge) = (*head + 1, 0);
                continue;
            };
            while let Some(next_edge) = edges.get(*edge) {
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                *edge += 1;
                let next = next_edge.to as usize;
                if next_edge.to == NONE || reach.state[next] == REACHED {
                    continue;
                }
                match self.admit(army, memo, at, next_edge) {
                    Ok(false) => {}
                    Ok(true) => {
                        reach.state[next] = REACHED;
                        queue.push(next);
                        if target == Some(next) {
                            return false;
                        }
                    }
                    Err(reason) => {
                        reach.note(reason);
                        if reach.state[next] == UNSEEN {
                            reach.state[next] = POSSIBLE;
                            possible.push(next);
                        }
                    }
                }
            }
            (*head, *edge) = (*head + 1, 0);
        }
        if !*unproved {
            *unproved = true;
            if reach.open.is_some() {
                possible.clear();
            }
        }
        if target.is_some_and(|target| reach.state[target] != UNSEEN) {
            return false;
        }
        loop {
            let (at, from) = match expanding.take() {
                Some(current) => current,
                None => match possible.pop() {
                    Some(at) if reach.state[at] == POSSIBLE => (at, 0),
                    Some(_) => continue,
                    None => return true,
                },
            };
            let Some(edges) = graph.neighbours(at) else {
                open(reach, at);
                continue;
            };
            for (index, next_edge) in edges.iter().enumerate().skip(from) {
                if *budget == 0 {
                    *expanding = Some((at, index));
                    return false;
                }
                *budget -= 1;
                let next = next_edge.to as usize;
                if next_edge.to == NONE || reach.state[next] != UNSEEN {
                    continue;
                }
                if let Ok(false) = self.admit(army, memo, at, next_edge).map_err(|reason| {
                    reach.note(reason);
                }) {
                    continue;
                }
                reach.state[next] = POSSIBLE;
                possible.push(next);
                if target == Some(next) {
                    *expanding = Some((at, index + 1));
                    return false;
                }
            }
        }
    }
}

pub struct Reachable {
    component: Vec<u32>,
    open: Vec<bool>,
}

impl Reachable {
    fn apart(&self, graph: &MoveGraph, from: u32, to: u32) -> bool {
        let Some(from) = graph.dense(from) else {
            return false;
        };
        let component = self.component[from];
        !self.open[component as usize]
            && graph
                .dense(to)
                .is_none_or(|to| self.component[to] != component)
    }
}

pub struct Search {
    reach: Reach,
    memo: Memo,
    queue: Vec<usize>,
    head: usize,
    edge: usize,
    possible: Vec<usize>,
    unproved: bool,
    expanding: Option<(usize, usize)>,
}

#[cfg(test)]
mod tests;
