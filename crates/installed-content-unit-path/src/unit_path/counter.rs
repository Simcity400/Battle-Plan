//! Fresh army, railway-gun and task-force counter routes, with no client
//! cache. The A* loop is shared with invasions; requests change prechecks,
//! edge admission and mode/forced-transfer fallback.

use super::*;

/// Unit class, with the recorded handle for class-specific facts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CounterClass {
    #[default]
    Army,
    Railway(u64),
    TaskForce(u64),
}

/// Viewer-owned route preference and this unit's simulation rail mode.
/// `local_option` supplies the interface route option; the interface's
/// retained path cache is not a recorded operand.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CounterRequest {
    pub class: CounterClass,
    pub strategic: bool,
    pub local_option: bool,
}

impl<W: MoveWorld + ?Sized> Mover<'_, W> {
    /// One mode-0 forced-transfer search, with region checks enabled and
    /// request exile, other flags, counter fallback and interface postfilter
    /// disabled. The passed source may differ from the actual location.
    pub fn transport_risk_route(
        &self,
        unit: &Army,
        from: u32,
        to: u32,
        class: CounterClass,
    ) -> Fact<Option<Vec<u32>>> {
        // The request's exile flag is independent of the unit's stored exile flag.
        let mut request = unit.clone();
        request.exiled = false;
        self.counter_search(&request, from, to, class, false, true)
    }

    /// Counter cache-miss routing. Source excluded, destination included;
    /// empty routes include failed searches and equal-endpoint success.
    /// The request leaves rail-speed cost division disabled.
    pub fn counter_route(
        &self,
        army: &Army,
        from: u32,
        to: u32,
        request: CounterRequest,
    ) -> Fact<Vec<u32>> {
        // Mode-2 region admission compares against the request source.
        // Counter callers pass this unit's actual location.
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
        // Definition flags with both low bits clear mean sea. The filter
        // checks naval-base-map presence, not positive building level.
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
        // Retry on search failure, independently of route length. Some(empty)
        // retains equal-endpoint success at this inner boundary.
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
            // Evaluate class-specific edge access before the mode-2 node predicate.
            if mode_two {
                self.counter_node_admissible(army, to, class)
            } else {
                Ok(true)
            }
        })
    }

    /// Request precheck with only region checks enabled.
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

    /// Class-specific counter admission. Transport classification reads
    /// stored port-sea links rather than active naval-base levels.
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
            // Classification byte 6: sea/sea or an exact port/sea pair.
            match [from_land, to_land] {
                [true, true] => false,
                [true, false] => self.world.port_sea(from)? == to,
                [false, true] => self.world.port_sea(to)? == from,
                [false, false] => true,
            }
        } else {
            // Classification byte 7: land/land or sea/land, without a port
            // or active naval-base requirement on the latter.
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
            // Request exile bypasses fleet, move-access and task-force access,
            // but refuses embarkation even on a forced-transfer route.
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

    /// Choose the class's permission before evaluating the rule verdict.
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
