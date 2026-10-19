//! Opt-in native movement-speed samples, taken on the stopped game thread.
use super::*;

/// The movement-speed virtual getter and its supported implementation.
pub const SPEED_GETTER_SLOT: usize = 0xd8;
pub const SPEED_GETTER_RVA: usize = 0x00c7_2ee0;

/// Calls the game's getter for this mover's actual first edge. This module is
/// absent unless the diagnostic feature is enabled; ordinary readers never call
/// game methods. No diagnostic description list is requested.
///
/// # Safety
/// The caller owns the supported game's hourly capture callback, on its game
/// thread, with simulation writers stopped for the entire call. `base`, `state`
/// and the authenticated census seed belong to that resident process.
pub unsafe fn first_edge_speed(
    memory: &impl Memory,
    base: usize,
    state: usize,
    seed: &DivisionSeed,
) -> Result<i64, &'static str> {
    if seed.primary_vtable != base.checked_add(VTABLE_RVA).map(|p| p as u64)
        || seed.handle != Some(seed.division)
        || seed.class != Some(0)
    {
        return Err("movement witness division identity refused");
    }
    let vtable = seed
        .primary_vtable
        .ok_or("movement witness vtable missing")? as usize;
    let getter = presentation_field::<u64>(memory, vtable, SPEED_GETTER_SLOT)? as usize;
    if Some(getter) != base.checked_add(SPEED_GETTER_RVA) {
        return Err("movement witness speed getter identity refused");
    }
    let route = seed
        .ordinary_route
        .and_then(|h| Array::from_header(h, 4))
        .ok_or("movement witness route unreadable")?;
    let from = seed
        .location
        .filter(|p| *p != 0)
        .ok_or("movement witness location unreadable")?;
    movement_definition(memory, base, from)?;
    let to = movement_route_province(memory, base, state, route, 0)?;
    if to == 0 {
        return Err("movement witness destination missing");
    }
    type Getter = unsafe extern "system" fn(usize, *mut i64, usize, usize) -> *mut i64;
    // SAFETY: the supported virtual target was checked above; caller guarantees
    // the live game-thread boundary and resident authenticated arguments.
    let getter: Getter = unsafe { std::mem::transmute(getter) };
    let mut result = 0_i64;
    unsafe {
        getter(seed.pointer, &mut result, from, to);
    }
    Ok(result)
}
