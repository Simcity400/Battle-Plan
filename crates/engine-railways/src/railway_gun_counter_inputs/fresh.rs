//! Class-13 fresh-route common operands after the established capture pass.
//!
//! Speed uses fresh variant stat 0x3e, separate from the gun's cached speed. The land branch
//! applies the fresh supply factor acquired by `eta.rs`. A nonnull naval transfer selects transport
//! independently of fresh endpoints. Orders-group access uses `carmy::ORDERS_GROUP`; the rail
//! assignee (`crailway_gun::ASSIGNEE`) is different. No game function, map entity, client path
//! cache or duration is acquired.

use super::{FreshEta, FreshEtaObservations, Row, at, eta, pointer_at};
use crate::{
    Memory, carmy, corders_group, creference_registry::HeaderResolver, fixed_layout,
    railway_gun_speed,
};
use std::cell::OnceCell;

fn orders_group(
    memory: &impl Memory,
    base: usize,
    gun: usize,
    registry: &mut HeaderResolver,
) -> Result<u64, &'static str> {
    let pointer = pointer_at(memory, at(gun, carmy::ORDERS_GROUP)?)
        .ok_or("railway gun genuine orders-group pointer unreadable")?;
    if pointer == 0 {
        return Ok(0);
    }
    let handle = corders_group::plan_pointer_handle(memory, base, pointer)?;
    if handle >> 32 == 0 || registry.resolve(memory, base, handle)? != Some(pointer) {
        return Err("railway gun genuine orders-group registry identity mismatch");
    }
    Ok(handle)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn capture(
    memory: &impl Memory,
    base: usize,
    state: usize,
    gun: usize,
    row: &Row,
    sample_hour: i32,
    named: &mut railway_gun_speed::NamedBonuses<'_>,
    registry: &mut HeaderResolver,
    layout: &OnceCell<Result<fixed_layout::Report, &'static str>>,
    layout_buffer: &mut [u8],
    observations: &mut FreshEtaObservations,
) -> (FreshEta, [Option<&'static str>; 3]) {
    let mut failures = [None; 3];
    let mut inputs = FreshEta {
        sample_hour,
        ..Default::default()
    };
    if !row.naval_transfer {
        let maximum = eta::maximum_speed(
            memory,
            base,
            state,
            gun,
            named,
            registry,
            layout,
            layout_buffer,
            observations,
        );
        inputs.maximum_speed = maximum.ok();
        failures[0] = maximum.err();
        let supply = eta::land_supply_factor(memory, base, gun, row.exiled, observations);
        inputs.land_supply_factor = supply.ok();
        failures[1] = supply.err();
    }
    let group = orders_group(memory, base, gun, registry);
    inputs.orders_group = group.ok();
    failures[2] = group.err();
    (inputs, failures)
}
