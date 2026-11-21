
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CounterClass {
    #[default]
    Army,
    Railway(u64),
    TaskForce(u64),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterRequest {
    pub class: CounterClass,
    pub strategic: bool,
    pub local_option: bool,
}

impl<W: MoveWorld + ?Sized> Mover<'_, W> {
    pub fn counter_route(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        request: CounterRequest,
    ) -> Fact<Vec<u32>> {
        if from != army.location {
            return Err("counter path source differs from its unit location".into());
        }
        let mut route = self
            .counter_attempt(army, from, to, request, false)?
            .unwrap_or_default();
        if !route.is_empty() {
            return Ok(route);
        }
        route = self
            .counter_attempt(army, from, to, request, true)?
            .unwrap_or_default();
        if request.local_option
            && !request.strategic
            && let Some(&first) = route.first()
            && self.is_sea(first)?
            && (!self.world.has_naval_base(from)? || !self.world.has_naval_base(to)?)
        {
            route.clear();
        }
        Ok(route)
    }

    fn counter_attempt(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        request: CounterRequest,
        forced_transfer: bool,
    ) -> Fact<Option<Vec<u32>>> {
        let mode_two = request.strategic || request.local_option;
        let route =
            self.counter_search(army, from, to, request.class, mode_two, forced_transfer)?;
        if mode_two && !request.strategic && route.is_none() {
            self.counter_search(army, from, to, request.class, false, forced_transfer)
        } else {
            Ok(route)
        }
    }

    fn counter_search(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        class: CounterClass,
        mode_two: bool,
        forced_transfer: bool,
    ) -> Fact<Option<Vec<u32>>> {
        if !self.counter_precheck(army, from, to, class, mode_two, forced_transfer)? {
            return Ok(None);
        }
        self.native_route(army, from, to, None, |from, to| {
            if !self.counter_edge(army, from, to, class, forced_transfer)? {
                return Ok(false);
            }
            if mode_two {
                self.counter_node_admissible(army, to, class)
            } else {
                Ok(true)
            }
        })
    }

    fn counter_precheck(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        class: CounterClass,
        mode_two: bool,
        forced_transfer: bool,
    ) -> Fact<bool> {
        if from == 0 || to == 0 {
            return Ok(false);
        }
        if !army.exiled && !self.counter_access(army, to, class, forced_transfer)? {
            return Ok(false);
        }
        if (matches!(class, CounterClass::TaskForce(_))
            || (!forced_transfer && !army.naval_transfer))
            && self.is_land(from)?
            && self.is_land(to)?
            && self.graph.land_mass(from)? != self.graph.land_mass(to)?
        {
            return Ok(false);
        }
        if mode_two && !self.counter_node_admissible(army, to, class)? {
            return Ok(false);
        }
        Ok(true)
    }

    fn counter_edge(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        class: CounterClass,
        forced_transfer: bool,
    ) -> Fact<bool> {
        let crossing = self.graph.crossing(from, to);
        if from == to || crossing.is_some_and(|crossing| crossing.impassable) {
            return Ok(false);
        }
        let [from_land, to_land] = [self.is_land(from)?, self.is_land(to)?];
        if from_land && to_land && self.crosses_other_war(army.owner, from, to)? {
            return Ok(false);
        }
        if let Some(rule) = crossing.and_then(|crossing| crossing.rule.as_ref())
            && !self.counter_rule_admits(self.rule(rule)?, army.owner, class)?
        {
            return Ok(false);
        }
        let transfer =
            matches!(class, CounterClass::TaskForce(_)) || forced_transfer || army.naval_transfer;
        let class_admits = if transfer {
            match [from_land, to_land] {
                [true, true] => false,
                [true, false] => self.world.port_sea(from)? == to,
                [false, true] => self.world.port_sea(to)? == from,
                [false, false] => true,
            }
        } else {
            to_land
                && (!matches!(class, CounterClass::Railway(_))
                    || self.world.counter_rail_connected(from, to)?)
        };
        if !class_admits {
            return Ok(false);
        }
        if !army.exiled
            && let Some(crossing) = crossing
        {
            match &crossing.strait {
                Strait::Through(through)
                    if self.world.strait_fleet(*through, army.owner)? == StraitFleet::Visible =>
                {
                    return Ok(false);
                }
                Strait::Invalid(reason) => return Err(reason.clone()),
                _ => {}
            }
        }
        if army.exiled {
            return Ok(!from_land || to_land);
        }
        let access = if matches!(class, CounterClass::TaskForce(_)) {
            self.naval_move_access(army.owner, to)?
        } else {
            self.move_access(army.owner, to)?
        };
        if !access {
            return Ok(false);
        }
        self.counter_access(army, to, class, forced_transfer)
    }

    fn counter_access(
        &self,
        army: &Army,
        to: u32,
        class: CounterClass,
        forced_transfer: bool,
    ) -> Fact<bool> {
        match class {
            CounterClass::Army => self.province_access_for_move(army, to, forced_transfer),
            CounterClass::Railway(handle) => {
                self.province_access_for_move_with_group(army, to, forced_transfer, || {
                    self.world.counter_railway_orders_group(handle)
                })
            }
            CounterClass::TaskForce(handle) => self.world.counter_task_force_access(handle, to),
        }
    }

    fn counter_node_admissible(&self, army: &Army, to: u32, class: CounterClass) -> Fact<bool> {
        match class {
            CounterClass::Army => self.node_admissible(army, to),
            CounterClass::Railway(_) => {
                Ok(!self.world.at_war(army.owner, self.world.controller(to)?)?
                    && !self.world.land_combat(to)?)
            }
            CounterClass::TaskForce(handle) => {
                self.world.counter_task_force_hostile_allows(handle, to)
            }
        }
    }

    fn counter_rule_admits(
        &self,
        rule: &AdjacencyRule,
        owner: Country,
        class: CounterClass,
    ) -> Fact<bool> {
        let permission = match class {
            CounterClass::Army | CounterClass::Railway(_) => adjacency_rules::ARMY,
            CounterClass::TaskForce(handle) => {
                if self.world.counter_task_force_submarine(handle)? {
                    adjacency_rules::SUBMARINE
                } else {
                    adjacency_rules::NAVY
                }
            }
        };
        Ok(rule.permits(self.verdict(rule, owner)?, permission))
    }
}
