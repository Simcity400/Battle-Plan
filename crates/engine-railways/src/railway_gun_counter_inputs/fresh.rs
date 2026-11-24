
use super::{FreshEta, FreshEtaObservations, Row, at, eta, pointer_at};
use engine_native_memory::Memory;
use engine_native::{carmy, corders_group};
use engine_native_parts::creference_registry::HeaderResolver;
use crate::{fixed_layout, railway_gun_speed};
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

#[expect(
    clippy::too_many_arguments,
    reason = "the gun, row and sample hour identify one estimate, and the named bonuses, registry, layout, buffer and observations are caller-owned"
)]
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
