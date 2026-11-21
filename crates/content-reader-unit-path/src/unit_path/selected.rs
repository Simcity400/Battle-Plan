
use super::{Army, Country, Fact, MoveWorld, Mover};
use crate::adjacency_rules::AdjacencyRule;

mod advance_warning;
mod area;
pub mod cursor;
pub mod transport;
pub use area::AreaNode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommonClass {
    Army,
    RailwayGun,
}

pub struct CommonUnit<'a> {
    pub class: CommonClass,
    pub handle: u64,
    pub logical_owner_raw: u32,
    pub combat_count: u32,
    pub army: &'a Army,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommonCombat {
    pub combat: u64,
    pub kind: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeArmyOccupant {
    pub handle: u64,
    pub logical_owner_raw: u32,
}

pub trait CommonMoveWorld: MoveWorld {
    fn canonical_country(&self, raw: u32) -> Fact<Country>;
    fn actual_country_raw(&self, country: Country) -> Fact<u32>;
    fn current_subject_access(&self, _receiver: Country, _target: Country) -> Fact<bool> {
        Err("selected current subject status inputs not recorded".into())
    }
    fn controller_raw(&self, province: u32) -> Fact<u32>;
    fn state_owner_raw(&self, province: u32) -> Fact<u32>;
    fn province_state(&self, province: u32) -> Fact<Option<u32>>;
    fn controller_area_equal(&self, from: u32, to: u32) -> Fact<bool>;
    fn directed_war(&self, receiver: Country, target: Country) -> Fact<bool>;
    fn owner_area_equal(&self, from: u32, to: u32) -> Fact<bool> {
        Err(format!(
            "selected owner-area identities {from}->{to} unavailable"
        ))
    }
    fn controller_area(&self, province: u32) -> Fact<u32> {
        Err(format!(
            "selected controller-area identity {province} unavailable"
        ))
    }
    fn area_node(&self, label: u32) -> Fact<AreaNode> {
        Err(format!("selected area {label} graph unavailable"))
    }
    fn area_nonsea_count(&self, label: u32) -> Fact<u32> {
        Err(format!("selected area {label} nonsea domain unavailable"))
    }
    fn unit_exiled_raw(&self, unit: &CommonUnit<'_>) -> Fact<u8> {
        Err(format!(
            "selected unit {:#x} raw exile unavailable",
            unit.handle
        ))
    }
    fn railway_path_owner_admits(
        &self,
        owner_raw: u32,
        controller_raw: u32,
        exiled: u8,
    ) -> Fact<bool> {
        Err(format!(
            "selected railway path country predicate {owner_raw}->{controller_raw} exile {exiled} unavailable"
        ))
    }
    fn full_unit_combats(&self, unit: &CommonUnit<'_>) -> Fact<Vec<CommonCombat>>;
    fn combat_has_unit(&self, combat: CommonCombat, unit: u64, attacking: bool) -> Fact<bool>;
    fn combat_minimum_duration(&self, combat: CommonCombat) -> Fact<bool>;
    fn unit_route_first(&self, unit: &CommonUnit<'_>) -> Fact<Option<u32>>;
    fn primary_path_combat(&self, unit: &CommonUnit<'_>, province: u32) -> Fact<bool>;
    fn province_units(&self, province: u32) -> Fact<&[u64]>;
    fn army_occupant(&self, handle: u64, province: u32) -> Fact<Option<NativeArmyOccupant>>;
    fn occupant_retreating(&self, handle: u64) -> Fact<bool>;
    fn occupant_strength(&self, handle: u64) -> Fact<i64>;
    fn unit_orders_group(&self, unit: &CommonUnit<'_>) -> Fact<u64>;
    fn ordered_enemy_raws(&self, country: Country) -> Fact<Vec<u32>>;
    fn move_strength_threshold(&self) -> Fact<i64>;
    fn move_organisation_threshold(&self) -> Fact<i64>;
    fn maximum_organisation(&self, unit: &CommonUnit<'_>) -> Fact<i64>;
    fn unit_exiled(&self, unit: &CommonUnit<'_>) -> Fact<bool>;
    fn unit_exfiltrates(&self, unit: &CommonUnit<'_>) -> Fact<bool>;
    fn unit_retreat_ban(&self, unit: &CommonUnit<'_>, attacking: bool) -> Fact<bool>;
    fn railway_retreat_edge(&self, source: u32, target: u32) -> Fact<bool>;
    fn combat_attacker_at(&self, combat: CommonCombat, target: u32) -> Fact<bool>;
    fn selected_path(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool>;
    fn selected_transport(
        &self,
        unit: &CommonUnit<'_>,
        target: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool>;
    fn railway_cooldown(&self, province: u32) -> Fact<Option<i32>>;
    fn railway_accessible(&self, province: u32) -> Fact<bool>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MoveOptions {
    pub transport_fallback: bool,
    pub retreat_bypass: bool,
    pub avoid_enemy: bool,
}

pub struct Silent;

impl CommonMoveText for Silent {
    fn plain(&self, _: &str) -> Fact<String> {
        Ok(String::new())
    }
    fn state(&self, _: &str, _: u32) -> Fact<String> {
        Ok(String::new())
    }
    fn owner(&self, _: &str, _: u32) -> Fact<String> {
        Ok(String::new())
    }
    fn rule(&self, _: &str, _: &AdjacencyRule) -> Fact<String> {
        Ok(String::new())
    }
    fn warning(&self, _: &str, _: i64) -> Fact<String> {
        Ok(String::new())
    }
    fn days(&self, _: &str, _: i32) -> Fact<String> {
        Ok(String::new())
    }
    fn transport_stock(&self, _: i32, _: i32) -> Fact<String> {
        Ok(String::new())
    }
    fn transport_risk(&self, _: i64) -> Fact<String> {
        Ok(String::new())
    }
    fn transport_description(&self, _: i32, _: bool) -> Fact<String> {
        Ok(String::new())
    }
}

pub trait CommonMoveText {
    fn plain(&self, key: &str) -> Fact<String>;
    fn state(&self, key: &str, state: u32) -> Fact<String>;
    fn owner(&self, key: &str, raw_owner: u32) -> Fact<String>;
    fn rule(&self, key: &str, rule: &AdjacencyRule) -> Fact<String>;
    fn warning(&self, key: &str, raw: i64) -> Fact<String>;
    fn days(&self, key: &str, days: i32) -> Fact<String>;
    fn transport_stock(&self, available: i32, required: i32) -> Fact<String>;
    fn transport_risk(&self, risk: i64) -> Fact<String>;
    fn transport_description(&self, convoys: i32, icon: bool) -> Fact<String>;
}

impl<W: CommonMoveWorld + ?Sized> Mover<'_, W> {
    pub fn selected_army_path(&self, unit: &CommonUnit<'_>, to: u32) -> Fact<bool> {
        let world = self.world;
        let from = unit.army.location;
        let same_owner = world.owner_area_equal(from, to)?;
        let access = |province| {
            self.common_has_province_access(
                world.canonical_country(unit.logical_owner_raw)?,
                province,
            )
        };
        if same_owner && access(to)? {
            return Ok(true);
        }
        let mut mode = world.unit_exiled_raw(unit)? ^ 1;
        let at = self
            .graph
            .dense(from)
            .ok_or("selected army source undefined")?;
        for edge in self
            .graph
            .neighbours(at)
            .ok_or("selected army source neighbours unavailable")?
        {
            if edge.crossing != super::NONE
                && self
                    .graph
                    .crossings
                    .get(edge.crossing as usize)
                    .ok_or("selected army crossing undefined")?
                    .impassable
            {
                continue;
            }
            if edge.to == super::NONE {
                continue;
            }
            let next = *self
                .graph
                .ids
                .get(edge.to as usize)
                .ok_or("selected army neighbour index outside graph")?;
            if self.graph.kind(next)? & 1 == 0 {
                continue;
            }
            if next == to && access(next)? {
                return Ok(true);
            }
            if access(next)? {
                mode = 1;
            }
        }
        if mode == 0 {
            return Ok(world.has_naval_base(to)? && same_owner);
        }
        let start = world.controller_area(from)?;
        if start == 0 {
            return Ok(false);
        }
        let target = world.controller_area(to)?;
        if target == 0 {
            return Ok(false);
        }
        if start == target {
            return Ok(true);
        }
        area::areas_connect(
            start,
            target,
            world.area_nonsea_count(start)?,
            |label| world.area_node(label),
            |node| {
                area::area_cost(
                    node.land_count.clone(),
                    unit.logical_owner_raw as i32,
                    || Ok(world.state_owner_raw(node.first_land_province.clone()?)? as i32),
                    || access(node.first_land_province.clone()?),
                    |a, b| self.raw_same(a as u32, b as u32),
                )
            },
        )
    }

    pub fn selected_railway_path(&self, unit: &CommonUnit<'_>, to: u32) -> Fact<bool> {
        use crate::province_graph::{heap_pop, heap_push};
        let graph = self.graph;
        let source = graph
            .dense(unit.army.location)
            .ok_or("selected rail source undefined")?;
        let target = graph.dense(to).ok_or("selected rail target undefined")?;
        if graph.kind(unit.army.location)? & 1 == 0 || graph.kind(to)? & 1 == 0 {
            return Err("selected railway route endpoints require land provinces".into());
        }
        self.world.canonical_country(unit.logical_owner_raw)?;
        let exile = self.world.unit_exiled_raw(unit)?;
        let [tx, ty] =
            graph.centroids[target].ok_or("selected rail target map position unavailable")?;
        let mut closed = vec![false; graph.ids.len()];
        let mut nodes = vec![None::<(i64, i64)>; graph.ids.len()];
        let mut heap = Vec::new();
        nodes[source] = Some((0, 0));
        closed[source] = true;
        heap_push(&mut heap, (0, source as u32));
        while !heap.is_empty() {
            let at = heap_pop(&mut heap) as usize;
            if at == target {
                return Ok(true);
            }
            closed[at] = true;
            let from = graph.ids[at];
            let neighbours = graph
                .neighbours(at)
                .ok_or("selected rail neighbours unavailable")?;
            for edge in neighbours {
                let next = edge.to as usize;
                let province = *graph
                    .ids
                    .get(next)
                    .ok_or("selected rail null or undefined neighbour")?;
                if closed[next] {
                    continue;
                }
                if !self.world.railway_retreat_edge(from, province)? {
                    continue;
                }
                if !self.world.railway_path_owner_admits(
                    unit.logical_owner_raw,
                    self.world.controller_raw(province)?,
                    exile,
                )? {
                    continue;
                }
                if self.world.demilitarized(province)? && !self.world.demilitarized(from)? {
                    continue;
                }
                let cost = neighbours
                    .iter()
                    .find(|first| first.to == edge.to)
                    .unwrap()
                    .distance
                    .ok_or("selected rail adjacency distance unavailable")?;
                if cost < 0 {
                    continue;
                }
                let g = nodes[at].unwrap().0.wrapping_add(cost);
                if g >= nodes[next].map_or(RAILWAY_UNSEEN_COST, |(old, _)| old) {
                    continue;
                }
                let h = if let Some((old_g, old_f)) = nodes[next] {
                    old_f.wrapping_sub(old_g)
                } else {
                    let [x, y] =
                        graph.centroids[next].ok_or("selected rail map position unavailable")?;
                    let mut dx = tx.wrapping_sub(x);
                    if dx < -(graph.width / 2) {
                        dx = dx.wrapping_add(graph.width);
                    }
                    if dx > graph.width / 2 {
                        dx = dx.wrapping_sub(graph.width);
                    }
                    let dy = ty.wrapping_sub(y);
                    let square = dx.wrapping_mul(dx).wrapping_add(dy.wrapping_mul(dy));
                    railway_heuristic(square)
                };
                let f = g.wrapping_add(h);
                nodes[next] = Some((g, f));
                heap_push(&mut heap, (f, next as u32));
            }
        }
        Ok(false)
    }

    fn raw_same(&self, first: u32, second: u32) -> Fact<bool> {
        if first == second {
            return Ok(true);
        }
        Ok(first != 0
            && second != 0
            && self.world.canonical_country(first)? == self.world.canonical_country(second)?)
    }

    fn raw_at_war(&self, receiver: Country, target_raw: u32) -> Fact<bool> {
        let actual = self.world.actual_country_raw(receiver)?;
        if self.raw_same(actual, target_raw)? || actual as i32 <= 0 {
            return Ok(false);
        }
        self.world
            .directed_war(receiver, self.world.canonical_country(target_raw)?)
    }

    fn common_class_access(&self, unit: &CommonUnit<'_>, to: u32) -> Fact<bool> {
        let army = unit.army;
        let world = self.world;
        let kind = self.graph.kind(to)?;
        if kind & 3 == 0 {
            return Ok(army.naval_transfer);
        }
        if kind & 2 != 0 || (kind & 1 != 0 && world.owner_area_missing(to)?) {
            return Ok(false);
        }
        if world.border_conflict(to)?
            && !(army.location != 0 && world.border_conflict(army.location)?)
            && !world.border_conflict_admits(to, world.unit_orders_group(unit)?)?
        {
            return Ok(false);
        }
        if world.demilitarized(to)? || world.impassable(to)? {
            return Ok(false);
        }
        if world.controller_area_equal(army.location, to)? {
            return Ok(true);
        }
        if world.unit_exiled(unit)?
            && self.raw_same(
                world.state_owner_raw(to)?,
                world.state_owner_raw(army.location)?,
            )?
        {
            return Ok(true);
        }
        if self.raw_same(
            world.controller_raw(to)?,
            world.controller_raw(army.location)?,
        )? {
            return Ok(true);
        }
        self.common_has_province_access(world.canonical_country(unit.logical_owner_raw)?, to)
    }

    fn common_has_province_access(&self, owner: Country, province: u32) -> Fact<bool> {
        let world = self.world;
        let kind = self.graph.kind(province)?;
        if kind & 2 != 0 {
            return Ok(false);
        }
        let state_owner = world.state_owner_raw(province)?;
        if state_owner as i32 <= 0 {
            return Ok(true);
        }
        let own_raw = world.actual_country_raw(owner)?;
        if self.raw_same(own_raw, state_owner)? {
            return Ok(true);
        }
        let controller_raw = world.controller_raw(province)?;
        if self.raw_same(own_raw, controller_raw)? {
            return Ok(true);
        }
        let controller = world.canonical_country(controller_raw)?;
        if world.directed_war(owner, controller)? || world.military_access(owner, controller)? {
            return Ok(true);
        }
        if kind & 3 == 0 {
            return Ok(false);
        }
        self.faction_or_overlord(owner, controller)
    }

    fn common_combats(&self, unit: &CommonUnit<'_>) -> Fact<Vec<CommonCombat>> {
        if (unit.combat_count as i32) <= 0 {
            return Ok(Vec::new());
        }
        let combats = self.world.full_unit_combats(unit)?;
        if combats.len() != unit.combat_count as usize {
            return Err("selected full combat reference count differs".into());
        }
        Ok(combats)
    }

    fn common_disengages(&self, unit: &CommonUnit<'_>, combats: &[CommonCombat]) -> Fact<bool> {
        for attacking in [false, true] {
            if self.world.unit_retreat_ban(unit, attacking)? {
                for &combat in combats {
                    if self.world.combat_has_unit(combat, unit.handle, attacking)? {
                        return Ok(false);
                    }
                }
            }
        }
        for &combat in combats {
            if self.world.combat_minimum_duration(combat)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn common_retreats(&self, unit: &CommonUnit<'_>, to: u32) -> Fact<bool> {
        if unit.class == CommonClass::RailwayGun
            && !self.world.railway_retreat_edge(unit.army.location, to)?
        {
            return Ok(false);
        }
        let army = unit.army;
        let world = self.world;
        if to == army.location || self.passable_adjacency(army.location, to) {
            let controller = world.canonical_country(world.controller_raw(to)?)?;
            if !self.raw_at_war(controller, unit.logical_owner_raw)? || army.organisation > 99_999 {
                let mut enemy = 0_i64;
                for &handle in world.province_units(to)? {
                    let Some(occupant) = world.army_occupant(handle, to)? else {
                        continue;
                    };
                    let owner = world.canonical_country(occupant.logical_owner_raw)?;
                    if self.raw_at_war(owner, unit.logical_owner_raw)? {
                        enemy = enemy.wrapping_add(world.occupant_strength(handle)?);
                    }
                }
                return Ok(((enemy / 100_000) as i32) <= 0);
            }
            return Ok(false);
        }
        Ok(world.has_naval_base(to)?
            && (world.has_naval_base(army.location)? || world.unit_exfiltrates(unit)?))
    }

    fn common_advance(
        &self,
        unit: &CommonUnit<'_>,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        if unit.class == CommonClass::RailwayGun {
            if unit.army.naval_transfer {
                return Ok(true);
            }
            text.push_str(&content.plain("NO_ADVANCE_ALLOWED")?);
            return Ok(false);
        }
        advance_warning::can_advance(
            text,
            Ok(unit.army.strength),
            self.world.move_strength_threshold(),
            || Ok(unit.army.organisation),
            || self.world.move_organisation_threshold(),
            || self.world.maximum_organisation(unit),
            |key, raw| content.warning(key, raw),
        )
    }

    fn common_enemy_army(&self, unit: &CommonUnit<'_>, to: u32) -> Fact<bool> {
        for &handle in self.world.province_units(to)? {
            let Some(occupant) = self.world.army_occupant(handle, to)? else {
                continue;
            };
            if self.world.occupant_retreating(handle)? {
                continue;
            }
            if self.raw_at_war(
                self.world.canonical_country(occupant.logical_owner_raw)?,
                unit.logical_owner_raw,
            )? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn common_class_enter(
        &self,
        unit: &CommonUnit<'_>,
        to: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        if unit.class == CommonClass::RailwayGun
            && !unit.army.naval_transfer
            && self.graph.kind(unit.army.location)? & 1 != 0
        {
            if let Some(days) = self.world.railway_cooldown(unit.army.location)?
                && days > 0
            {
                text.push_str(&content.days("NO_RAILWAY_ACCESS_STANDING_COOLDOWN", days)?);
                return Ok(false);
            }
            if !self.world.railway_accessible(to)? && !self.world.has_naval_base(to)? {
                let days = self.world.railway_cooldown(to)?.unwrap_or(0);
                text.push_str(&if days > 0 {
                    content.days("NO_RAILWAY_ACCESS_TO_TARGET_COOLDOWN", days)?
                } else {
                    content.plain("NO_RAILWAY_ACCESS_TO_TARGET")?
                });
                return Ok(false);
            }
            if self.raw_at_war(
                self.world.canonical_country(unit.logical_owner_raw)?,
                self.world.controller_raw(to)?,
            )? {
                text.push_str(&content.plain("NO_ADVANCE_ALLOWED")?);
                return Ok(false);
            }
            if self.common_enemy_army(unit, to)? {
                text.push_str(&content.plain("NO_SAFE_RAILWAY_ACCESS")?);
                return Ok(false);
            }
        }
        self.common_enter_message(unit, to, text, content)
    }

    pub fn selected_move_message(
        &self,
        unit: &CommonUnit<'_>,
        to: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let options = MoveOptions {
            transport_fallback: true,
            retreat_bypass: true,
            avoid_enemy: false,
        };
        self.selected_move(unit, to, options, text, content)
    }

    pub fn selected_move(
        &self,
        unit: &CommonUnit<'_>,
        to: u32,
        options: MoveOptions,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let army = unit.army;
        let world = self.world;
        if army.location != to {
            if army.retreating && !options.retreat_bypass {
                text.push_str(&content.plain("NO_MOVE_RETREAT")?);
                return Ok(false);
            }
            if (unit.combat_count as i32) > 0
                && !self.common_disengages(unit, &self.common_combats(unit)?)?
                && !self.raw_at_war(
                    world.canonical_country(world.controller_raw(to)?)?,
                    unit.logical_owner_raw,
                )?
            {
                text.push_str(&content.plain("NO_RETREAT_ALLOWED")?);
                return Ok(false);
            }
            if !self.common_class_enter(unit, to, text, content)? {
                return Ok(false);
            }
            if self.graph.kind(army.location)? & 3 == 0
                && self.graph.kind(to)? & 1 != 0
                && (unit.class == CommonClass::RailwayGun
                    || !self.passable_adjacency(to, army.location))
            {
                return Ok(false);
            }
        }
        if unit.class == CommonClass::Army {
            for combat in self.common_combats(unit)? {
                if combat.kind != 2
                    && world.combat_has_unit(combat, unit.handle, false)?
                    && world.combat_attacker_at(combat, to)?
                {
                    text.push_str(&content.plain("NO_COUNTER_ATTACK_IN_COMBAT")?);
                    return Ok(false);
                }
            }
        }
        if army.naval_transfer
            || (unit.class == CommonClass::Army
                && self.graph.kind(army.location)? & 3 == 0
                && self.raw_at_war(
                    world.canonical_country(world.controller_raw(to)?)?,
                    unit.logical_owner_raw,
                )?)
        {
            text.push_str(&content.plain("TRANSPORT_ACTIVE_CANT_MOVE")?);
            return Ok(false);
        }
        if unit.class == CommonClass::RailwayGun
            && self.graph.kind(army.location)? & 3 == 0
            && self.graph.kind(to)? & 1 != 0
        {
            return Ok(false);
        }
        if unit.class == CommonClass::Army {
            if !self.common_advance(unit, text, content)? && self.common_enemy_army(unit, to)? {
                return Ok(false);
            }
            if options.avoid_enemy
                && self.raw_at_war(
                    world.canonical_country(unit.logical_owner_raw)?,
                    world.controller_raw(to)?,
                )?
            {
                return Ok(false);
            }
        }
        if world.selected_path(unit, to, text, content)? {
            return Ok(true);
        }
        if !options.transport_fallback {
            return Ok(false);
        }
        world.selected_transport(unit, to, text, content)
    }

    pub fn common_enter_message(
        &self,
        unit: &CommonUnit<'_>,
        to: u32,
        text: &mut String,
        content: &impl CommonMoveText,
    ) -> Fact<bool> {
        let world = self.world;
        let army = unit.army;
        if !self.common_class_access(unit, to)? {
            text.push_str(&content.plain("NO_ACCESS_TO_TARGET")?);
            return Ok(false);
        }
        if (unit.combat_count as i32) > 0 {
            let controller = world.canonical_country(world.controller_raw(to)?)?;
            if !self.raw_at_war(controller, unit.logical_owner_raw)?
                && !self.common_retreats(unit, to)?
            {
                text.push_str(&content.plain("NO_RETREAT_ALLOWED")?);
                return Ok(false);
            }
        }
        if (unit.combat_count as i32) > 0
            && let Some(first) = world.unit_route_first(unit)?
            && world.primary_path_combat(unit, first)?
            && !self.common_disengages(unit, &self.common_combats(unit)?)?
        {
            if world.province_state(to)?.is_some() {
                text.push_str(&content.plain("NO_ADVANCE_IN_COMBAT")?);
            }
            return Ok(false);
        }
        if world.demilitarized(to)? {
            let state = world
                .province_state(to)?
                .ok_or("demilitarized province has no state")?;
            text.push_str(&content.state("UNIT_DEMILITARIZED", state)?);
            return Ok(false);
        }
        if world.border_conflict(to)?
            && !world.border_conflict_admits(to, world.unit_orders_group(unit)?)?
        {
            let state = world
                .province_state(to)?
                .ok_or("border-conflict warning has no state")?;
            text.push_str(&content.state("UNIT_BORDER_WAR", state)?);
            return Ok(false);
        }
        if world.impassable(to)? {
            let state = world
                .province_state(to)?
                .ok_or("impassable province has no state")?;
            text.push_str(&content.state("UNIT_IMPASSABLE", state)?);
            return Ok(false);
        }
        if !army.retreating {
            for &handle in world.province_units(to)? {
                let Some(occupant) = world.army_occupant(handle, to)? else {
                    continue;
                };
                if world.occupant_retreating(handle)? {
                    continue;
                }
                let owner = world.canonical_country(occupant.logical_owner_raw)?;
                if self.raw_at_war(owner, unit.logical_owner_raw)? {
                    if !self.common_advance(unit, text, content)? {
                        return Ok(false);
                    }
                    break;
                }
            }
        }
        let controller_raw = world.controller_raw(to)?;
        if !self.raw_same(unit.logical_owner_raw, controller_raw)?
            && world.directed_war(
                world.canonical_country(unit.logical_owner_raw)?,
                world.canonical_country(controller_raw)?,
            )?
        {
            let owner = world.canonical_country(unit.logical_owner_raw)?;
            let state_owner_raw = world.state_owner_raw(to)?;
            let state_owner = world.canonical_country(state_owner_raw)?;
            if !self.common_discloses(owner, state_owner_raw, false)?
                && !world.military_access(owner, state_owner)?
                && !self.common_occupies(owner, state_owner_raw)?
            {
                text.push_str(
                    &content.owner("UNIT_CANT_MOVE_TO_OCCUPIED_NEUTRAL", state_owner_raw)?,
                );
                return Ok(false);
            }
        }
        if let Some(rule) = self
            .graph
            .crossing(army.location, to)
            .and_then(|c| c.rule.as_ref())
        {
            let rule = self.rule(rule)?;
            let verdict =
                self.common_rule_verdict(rule, world.canonical_country(unit.logical_owner_raw)?)?;
            if !rule.permits(verdict, crate::adjacency_rules::ARMY) {
                text.push_str(&content.rule("UNIT_ADJECENCY_RULE_BLOCKED", rule)?);
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn common_rule_verdict(
        &self,
        rule: &AdjacencyRule,
        owner: Country,
    ) -> Fact<Option<crate::adjacency_rules::Verdict>> {
        Ok(self.common_rule_relation(rule, owner)?.verdict())
    }

    pub fn common_rule_relation(
        &self,
        rule: &AdjacencyRule,
        owner: Country,
    ) -> Fact<super::RuleReason> {
        self.relation_with(rule, owner, &|receiver, target, friendly| {
            let target_raw = self.world.actual_country_raw(target)?;
            if friendly {
                self.common_discloses(receiver, target_raw, true)
            } else {
                self.raw_at_war(receiver, target_raw)
            }
        })
    }

    fn common_discloses(&self, owner: Country, target_raw: u32, access: bool) -> Fact<bool> {
        let world = self.world;
        if self.raw_same(world.actual_country_raw(owner)?, target_raw)? {
            return Ok(true);
        }
        if world.directed_war(owner, world.canonical_country(target_raw)?)? {
            return Ok(false);
        }
        let target = world.canonical_country(target_raw)?;
        if access && world.military_access(owner, target)? {
            return Ok(true);
        }
        if let Some(faction) = world.faction(owner)?
            && world.faction(target)? == Some(faction)
        {
            return Ok(true);
        }
        world.current_subject_access(owner, target)
    }

    fn common_occupies(&self, owner: Country, target_raw: u32) -> Fact<bool> {
        let world = self.world;
        if self.raw_at_war(owner, target_raw)? {
            return Ok(true);
        }
        if !world.can_occupy_non_war(owner)? {
            return Ok(false);
        }
        let theirs = world.ordered_enemy_raws(world.canonical_country(target_raw)?)?;
        for ours in world.ordered_enemy_raws(owner)? {
            for &theirs in &theirs {
                if self.raw_same(ours, theirs)? {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests;

const RAILWAY_UNSEEN_COST: i64 = 0x53e2_d620_fb40;

fn railway_heuristic(square: i32) -> i64 {
    let mut root = square;
    let mut next = square / 2;
    if square > 1 {
        loop {
            root = next;
            next = (square / root).wrapping_add(root) / 2;
            if next >= root {
                break;
            }
        }
    }
    i64::from(root).wrapping_mul(100_000)
}
