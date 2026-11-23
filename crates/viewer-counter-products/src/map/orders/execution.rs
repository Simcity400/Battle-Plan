
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
};

use replay_data::{
    rows::ArmyOrderPlanRow, rows::ArmyOrderRow, rows::DivisionMovementRow,
    rows::DivisionOrderTransportInputs, rows::DivisionRow, rows::OrderExecutionInputs,
    rows::OrdersGroupRow, rows::Recorded, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use super::{OrderKey, TYPE_INVASION, TYPE_PARADROP, paradrop_counts, transport_counts};

const ONE: i64 = 100_000;

use viewer_gpu::ribbon_content::{ExecutionDefines as Defines, ExecutionState as State};

type Evaluation = Result<bool, String>;

pub(super) struct Context<'a, 'c> {
    frame: &'a DecodedFrame,
    defines: &'c Defines,
    orders: &'c BTreeMap<OrderKey, &'a ArmyOrderRow>,
    plans: &'c BTreeMap<OrderKey, &'a ArmyOrderPlanRow>,
    groups: &'c BTreeMap<u64, &'a OrdersGroupRow>,
    transports: &'c paradrop_counts::Context<'a>,
    results: RefCell<BTreeMap<OrderKey, Evaluation>>,
    roots: RefCell<BTreeMap<u64, Result<Option<OrderKey>, String>>>,
}

impl<'a, 'c> Context<'a, 'c> {
    pub(super) fn new(
        frame: &'a DecodedFrame,
        defines: &'c Defines,
        orders: &'c BTreeMap<OrderKey, &'a ArmyOrderRow>,
        plans: &'c BTreeMap<OrderKey, &'a ArmyOrderPlanRow>,
        groups: &'c BTreeMap<u64, &'a OrdersGroupRow>,
        transports: &'c paradrop_counts::Context<'a>,
    ) -> Self {
        Self {
            frame,
            defines,
            orders,
            plans,
            groups,
            transports,
            results: RefCell::default(),
            roots: RefCell::default(),
        }
    }

    pub(super) fn callback(&self, order: &ArmyOrderRow) -> Result<State, String> {
        if order.active == 0 {
            return Ok(State::default());
        }
        self.frame.complete::<tables::ArmyOrders>().map_err(error)?;
        self.frame
            .complete::<tables::ArmyOrderPlans>()
            .map_err(error)?;
        ordered_conditions(
            (order.owner_group, order.instance),
            self.orders,
            self.plans,
            &mut self.results.borrow_mut(),
            |order, plan| self.local(order, plan),
        )
        .map(|prerequisites| State {
            active: true,
            prerequisites,
        })
    }

    fn local(&self, order: &ArmyOrderRow, plan: &ArmyOrderPlanRow) -> Evaluation {
        match order.order_type {
            TYPE_INVASION => self.naval(order, plan),
            TYPE_PARADROP => self.paradrop(order, plan),
            _ => {
                if matches!(plan.execution, Recorded::Value(Some(_))) {
                    Err("execution operands on an order outside type3/4".into())
                } else {
                    Ok(true)
                }
            }
        }
    }

    fn group(&self, owner: u64) -> Result<&OrdersGroupRow, String> {
        self.frame
            .complete::<tables::OrdersGroups>()
            .map_err(error)?;
        self.groups
            .get(&owner)
            .copied()
            .ok_or_else(|| format!("execution owner {owner:#x} has no OrdersGroups row"))
    }

    fn unit(&self, handle: u64) -> Result<&DivisionRow, String> {
        let rows = self.frame.complete::<tables::Divisions>().map_err(error)?;
        rows.binary_search_by_key(&handle, |row| row.id)
            .ok()
            .map(|at| &rows[at])
            .ok_or_else(|| format!("assigned execution unit {handle:#x} has no Divisions row"))
    }

    fn movement(&self, handle: u64) -> Result<&DivisionMovementRow, String> {
        let rows = self
            .frame
            .complete::<tables::DivisionMovement>()
            .map_err(error)?;
        rows.binary_search_by_key(&handle, |row| row.division)
            .ok()
            .map(|at| &rows[at])
            .ok_or_else(|| {
                format!("assigned execution unit {handle:#x} has no DivisionMovement row")
            })
    }

    fn naval(&self, order: &ArmyOrderRow, plan: &ArmyOrderPlanRow) -> Evaluation {
        let mut demand = 0_i32;
        for &handle in &plan.assigned_divisions {
            self.unit(handle)?;
            let movement = self.movement(handle)?;
            if movement.naval_transfer != 0 {
                continue;
            }
            let weight = unit_transport(movement)?
                .naval_weight
                .ok_or("DivisionMovement.order_transport naval weight is unavailable")?;
            demand = demand.wrapping_add(naval_count(weight));
        }
        let allocated = transport_counts::invasion(plan)?;
        let rows = self
            .frame
            .complete::<tables::InvasionPreparations>()
            .map_err(error)?;
        let key = (order.owner_group, order.instance);
        let row = rows
            .binary_search_by_key(&key, |row| (row.owner_group, row.instance))
            .ok()
            .map(|at| &rows[at])
            .ok_or("the invasion execution preparation row is unavailable")?;
        let group = self.group(order.owner_group)?;
        self.validate_relation(group, row.target, row.relation_term)?;
        if row.leader_term.is_some() && (group.leader == 0 || group.hq_provider == 0)
            || row.leader_applies != 0 && row.leader_term.is_none()
        {
            return Err("invasion execution leader term is outside its native gate".into());
        }
        let days = self.defines.naval_days.as_ref().map_err(Clone::clone)?;
        let hours = dashboard_text::triggers::required_invasion_hours(row, *days);
        let prepared = (row.progress / ONE) as i32 >= hours;
        let inputs = execution_inputs(plan)?;
        if inputs.paradrop.is_some() {
            return Err("naval order has paradrop execution operands".into());
        }
        let route = inputs
            .route_is_ok
            .ok_or("ArmyOrderPlans.execution route_is_ok is unavailable")?;
        Ok(allocated >= demand && prepared && route != 0)
    }

    fn paradrop(&self, order: &ArmyOrderRow, plan: &ArmyOrderPlanRow) -> Evaluation {
        let inputs = execution_inputs(plan)?;
        let route = inputs
            .route_is_ok
            .ok_or("ArmyOrderPlans.execution route_is_ok is unavailable")?;
        if route == 0 {
            if inputs.paradrop.is_some() {
                return Err("paradrop preparation operands outside a nonzero route gate".into());
            }
            return Ok(false);
        }
        let preparation = inputs
            .paradrop
            .as_ref()
            .ok_or("ArmyOrderPlans.execution paradrop progress is unavailable")?;
        let terms = preparation
            .terms
            .as_ref()
            .ok_or("ArmyOrderPlans.execution paradrop preparation terms are unavailable")?;
        let group = self.group(order.owner_group)?;
        self.validate_relation(group, terms.target, terms.relation_term)?;
        if terms.leader_term.is_some() && group.leader == 0 {
            return Err("paradrop preparation leader term has a null owning-group leader".into());
        }
        let days = self.defines.air_days.as_ref().map_err(Clone::clone)?;
        let required = preparation_hours(
            *days,
            terms.country_term,
            terms.relation_term.unwrap_or(0),
            terms.leader_term.unwrap_or(0),
            terms.days_term,
        );
        if ((preparation.progress / ONE) as i32) < required {
            return Ok(false);
        }
        if (order.paradrop_target as i32) < 1 {
            return Ok(true);
        }
        let mut demand = 0_i64;
        for &handle in &plan.assigned_divisions {
            let unit = self.unit(handle)?;
            if self.primary_root(unit)? != Some((order.owner_group, order.instance)) {
                continue;
            }
            let movement = self.movement(handle)?;
            let pack = unit_transport(movement)?;
            let eligible = pack
                .paradrop_eligible
                .ok_or("DivisionMovement.order_transport paradrop eligibility is unavailable")?;
            if eligible == 0 || unit.province != order.paradrop_target {
                continue;
            }
            let modifiers = pack
                .paradrop_modifiers
                .as_ref()
                .ok_or("DivisionMovement.order_transport paradrop modifiers are unavailable")?;
            let base = self.defines.unit_weight.as_ref().map_err(Clone::clone)?;
            demand = demand.wrapping_add(transport_weight(
                *base,
                modifiers.country_term,
                modifiers.commander_term.unwrap_or(0),
            ));
        }
        let totals = self.transports.totals(order, Some(group))?;
        let minimum = self.defines.minimum_planes.as_ref().map_err(Clone::clone)?;
        let capacity = if totals.plane_count >= *minimum {
            totals.capacity_raw
        } else {
            0
        };
        Ok(missing_transports(demand, capacity) < 1)
    }

    fn validate_relation(
        &self,
        group: &OrdersGroupRow,
        target: u32,
        term: Option<i64>,
    ) -> Result<(), String> {
        if term.is_none() {
            return Ok(());
        }
        let own_slot = self.slot(group.country)?;
        let native_id = if own_slot == 0 {
            0
        } else {
            self.frame
                .complete::<tables::Countries>()
                .map_err(error)?
                .iter()
                .find(|row| row.canonical_slot == own_slot)
                .map(|row| row.id)
                .ok_or("the execution country has no Countries identity")?
        };
        if !relation_gate(target, native_id, |raw| self.slot(raw))? {
            return Err(
                "execution relation contribution is outside its native country gate".into(),
            );
        }
        Ok(())
    }

    fn slot(&self, raw: u32) -> Result<u32, String> {
        if raw == 0 {
            return Ok(0);
        }
        let rows = self
            .frame
            .complete::<tables::CountryLinks>()
            .map_err(error)?;
        rows.binary_search_by_key(&raw, |row| row.id)
            .ok()
            .map(|at| rows[at].canonical_slot)
            .ok_or_else(|| format!("execution country {raw} has no CountryLinks row"))
    }

    fn primary_root(&self, unit: &DivisionRow) -> Result<Option<OrderKey>, String> {
        if let Some(result) = self.roots.borrow().get(&unit.id) {
            return result.clone();
        }
        let result = self.find_primary_root(unit);
        self.roots.borrow_mut().insert(unit.id, result.clone());
        result
    }

    fn find_primary_root(&self, unit: &DivisionRow) -> Result<Option<OrderKey>, String> {
        if unit.orders_group == 0 {
            return Ok(None);
        }
        let group = self.group(unit.orders_group)?;
        let roots = self
            .frame
            .complete::<tables::OrdersGroupPlans>()
            .map_err(error)?;
        let plans = self
            .frame
            .complete::<tables::ArmyOrderPlans>()
            .map_err(error)?;
        let orders = self.frame.complete::<tables::ArmyOrders>().map_err(error)?;
        for owner in [group.handle, group.parent] {
            if owner == 0 {
                break;
            }
            self.group(owner)?;
            let row = roots
                .binary_search_by_key(&owner, |row| row.group)
                .ok()
                .map(|at| &roots[at])
                .ok_or("execution member's group has no primary-root row")?;
            for &instance in &row.primary_orders {
                let key = (owner, instance);
                let plan = plans
                    .binary_search_by_key(&key, |row| (row.owner_group, row.instance))
                    .ok()
                    .map(|at| &plans[at])
                    .ok_or("execution primary root has no plan row")?;
                if orders
                    .binary_search_by_key(&key, |row| (row.owner_group, row.instance))
                    .is_err()
                {
                    return Err("execution primary root has no ArmyOrders row".into());
                }
                if plan.assigned_divisions.contains(&unit.id) {
                    return Ok(Some(key));
                }
            }
        }
        Ok(None)
    }
}

fn execution_inputs(plan: &ArmyOrderPlanRow) -> Result<&OrderExecutionInputs, String> {
    plan.execution
        .require("ArmyOrderPlans.execution")?
        .as_ref()
        .ok_or_else(|| "ArmyOrderPlans.execution operands are unavailable".into())
}

fn unit_transport(row: &DivisionMovementRow) -> Result<&DivisionOrderTransportInputs, String> {
    row.order_transport
        .require("DivisionMovement.order_transport")?
        .as_ref()
        .ok_or_else(|| "DivisionMovement.order_transport class0 operands are unavailable".into())
}

fn relation_gate(
    target: u32,
    native_id: u32,
    slot: impl Fn(u32) -> Result<u32, String>,
) -> Evaluation {
    Ok((target as i32) > 0
        && target != native_id
        && (native_id == 0 || slot(target)? != slot(native_id)?))
}

fn ordered_conditions(
    root: OrderKey,
    orders: &BTreeMap<OrderKey, &ArmyOrderRow>,
    plans: &BTreeMap<OrderKey, &ArmyOrderPlanRow>,
    memo: &mut BTreeMap<OrderKey, Evaluation>,
    mut local: impl FnMut(&ArmyOrderRow, &ArmyOrderPlanRow) -> Evaluation,
) -> Evaluation {
    let mut stack: Vec<(OrderKey, usize)> = Vec::new();
    let mut path = BTreeSet::new();
    let mut next = Some(root);
    loop {
        let result = if let Some(key) = next.take() {
            if let Some(result) = memo.get(&key) {
                Some(result.clone())
            } else if !path.insert(key) {
                Some(Err(format!(
                    "execution successor graph cycles at {:#x}:{}",
                    key.0, key.1
                )))
            } else {
                let result = orders
                    .get(&key)
                    .ok_or_else(|| {
                        format!(
                            "execution successor {:#x}:{} has no order row",
                            key.0, key.1
                        )
                    })
                    .and_then(|order| {
                        plans
                            .get(&key)
                            .ok_or_else(|| {
                                format!(
                                    "execution successor {:#x}:{} has no plan row",
                                    key.0, key.1
                                )
                            })
                            .and_then(|plan| local(order, plan))
                    });
                if result == Ok(true) {
                    stack.push((key, 0));
                    None
                } else {
                    path.remove(&key);
                    memo.insert(key, result.clone());
                    Some(result)
                }
            }
        } else {
            None
        };
        if let Some(result) = result {
            if result != Ok(true) {
                for &(key, _) in &stack {
                    memo.insert(key, result.clone());
                }
                return result;
            }
            if stack.is_empty() {
                return result;
            }
        }
        let Some((key, child)) = stack.last_mut() else {
            return Ok(true);
        };
        if let Some(&instance) = plans[key].successors.get(*child) {
            *child += 1;
            next = Some((key.0, instance));
        } else {
            let (key, _) = stack.pop().expect("the current execution frame exists");
            path.remove(&key);
            memo.insert(key, Ok(true));
            if stack.is_empty() {
                return Ok(true);
            }
        }
    }
}

fn naval_count(weight: i64) -> i32 {
    (ceil_fixed(weight) / ONE) as i32
}

fn ceil_fixed(value: i64) -> i64 {
    let whole = value / ONE;
    if value == whole * ONE {
        value
    } else {
        whole.wrapping_add(i64::from(value > 0)).wrapping_mul(ONE)
    }
}

fn preparation_hours(days: i32, country: i64, relation: i64, leader: i64, days_term: i64) -> i32 {
    let factor = ONE
        .wrapping_add(country)
        .wrapping_add(relation)
        .wrapping_add(leader);
    let days = i64::from(days).wrapping_mul(ONE).wrapping_add(days_term);
    ((factor.wrapping_mul(days) / 10_000_000_000) as i32).wrapping_mul(24)
}

fn transport_weight(base: i64, country: i64, commander: i64) -> i64 {
    ONE.wrapping_add(country)
        .wrapping_add(commander)
        .wrapping_mul(base)
        / ONE
}

fn missing_transports(demand: i64, capacity: i64) -> i32 {
    let delta = demand.wrapping_sub(capacity);
    let fixed = if matches!(delta, i64::MIN | i64::MAX) || delta == i64::MIN + 1 {
        i64::from(u32::MAX)
    } else {
        delta.wrapping_mul(ONE) / 10_000_000
    };
    (ceil_fixed(fixed) / ONE) as i32
}

fn error(error: std::io::Error) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests;
