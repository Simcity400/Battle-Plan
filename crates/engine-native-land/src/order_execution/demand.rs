use super::*;

fn rows(outcome: Outcome) -> Result<usize, &'static str> {
    match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("execution sizing count overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Demand {
    pub capacity: Capacity,
    pub failure: Option<&'static str>,
}

pub fn demand(memory: &impl Memory, base: usize, state: usize) -> Demand {
    engine_allocation::advisory(|| demand_inner(memory, base, state))
}

fn demand_inner(memory: &impl Memory, base: usize, state: usize) -> Demand {
    let mut demand = Demand::default();
    match rows(carmy::walk(memory, base, state, &mut [])) {
        Ok(count) => demand.capacity.units = count,
        Err(reason) => demand.failure = Some(reason),
    }
    if let Err(reason) = collect(memory, base, state, &mut demand.capacity) {
        demand.failure.get_or_insert(reason);
    }
    demand
}

pub(super) fn collect(
    memory: &impl Memory,
    base: usize,
    state: usize,
    capacity: &mut Capacity,
) -> Result<(), &'static str> {
    let count = rows(corders_group::count_roots(memory, base))?;
    capacity.groups = count;
    let mut roots =
        engine_allocation::default_vec(count).map_err(|_| "execution sizing allocation failed")?;
    let root_source = corders_group::walk_roots(memory, base, &mut roots);
    complete(root_source, count)?;
    let (mut orders, mut geometry, mut visits) = (Vec::new(), Vec::new(), Vec::new());
    let mut successors = order::SuccessorLists::default();
    let census = order::walk_listing_successors_of(
        memory,
        base,
        &mut orders,
        &mut geometry,
        &roots,
        root_source,
        &mut visits,
        &mut successors,
    );
    capacity.orders = census.visited;
    complete(census.outcome, census.outcome.len())?;
    orders.truncate(census.outcome.len());
    geometry.truncate(census.geometry_len);
    let (mut plans, mut assigned, mut links, mut names) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut plan_order = order::PlanOrder::default();
    let mut hosts = AssignedSources::default();
    let report = order::walk_plans_ordered_with_sources(
        memory,
        base,
        &roots,
        &visits,
        &successors,
        census.outcome,
        &mut plans,
        &mut assigned,
        &mut links,
        &mut names,
        &mut plan_order,
        Some(&mut hosts),
    );
    capacity.memberships = report.required_assigned;
    complete(report.outcome, orders.len())?;
    plans.truncate(report.outcome.len());
    assigned.truncate(report.assigned);
    complete(hosts.outcome(), assigned.len())?;
    let mut groups =
        engine_allocation::default_vec(count).map_err(|_| "execution sizing allocation failed")?;
    let (mut primary, mut group_names) = (Vec::new(), Vec::new());
    let group = corders_group::walk_plans(
        memory,
        base,
        &roots,
        root_source,
        &visits,
        census.outcome,
        &mut groups,
        &mut primary,
        &mut group_names,
    );
    complete(group.outcome, count)?;
    primary.truncate(group.orders);
    let count = capacity.units;
    let mut unit_rows =
        engine_allocation::default_vec(count).map_err(|_| "execution sizing allocation failed")?;
    let mut seeds =
        engine_allocation::vec(count).map_err(|_| "execution sizing allocation failed")?;
    let unit_source = carmy::walk_with(memory, base, state, &mut unit_rows, |pointer, row| {
        seeds.push(carmy::DivisionSeed {
            pointer,
            division: row.id,
            ..Default::default()
        });
    });
    complete(unit_source, count)?;
    carmy::capture_division_seeds(memory, &mut seeds);
    let slots = engine_native::cgame_state::read_country_registry_size(memory, base)
        .ok_or("execution sizing country registry unavailable")?
        .slots();
    let mut country_memo = ccountry::NativeReferenceMemo::try_with_capacity(slots, slots)
        .map_err(|_| "execution sizing allocation failed")?;
    country_memo.reset(base, state);
    let countries = country_memo.view(memory, base, state);
    let mut prepared = Prepared::try_new(Capacity {
        units: count,
        groups: roots.len(),
        orders: orders.len(),
        memberships: assigned.len(),
        ..Default::default()
    })
    .map_err(|_| "execution sizing allocation failed")?;
    let orders = Orders {
        roots: &roots,
        root_source,
        visits: &visits,
        rows: &orders,
        source: census.outcome,
        geometry: &geometry,
        plan_rows: plan_order.rows(),
        census: successors.census(),
    };
    prepared.capture_orders(
        memory,
        base,
        state,
        orders,
        &mut plans,
        report.outcome,
        &countries,
    );
    prepared.capture_units(
        memory,
        base,
        state,
        &seeds,
        unit_source,
        orders,
        SourceView {
            plans: &plans,
            source: report.outcome,
            groups: &groups,
            group_source: group.outcome,
            primary: &primary,
            assigned: &assigned,
            assigned_source: hosts.outcome(),
            hosts: &hosts,
        },
        &countries,
    );
    if let Some(reason) = prepared.index_error.or(prepared.membership_error) {
        return Err(reason);
    }
    if let Outcome::Unavailable { reason } = prepared.units.outcome {
        return Err(reason);
    }
    *capacity = prepared.required_capacity();
    Ok(())
}
