
use engine_native_memory::{Memory, walk::Outcome};
use engine_native::cgame_state;

pub fn rows(outcome: Outcome) -> Result<usize, &'static str> {
    match outcome {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("table storage demand overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

pub fn demand_game_state(memory: &impl Memory, base: usize) -> Result<Option<usize>, &'static str> {
    let game_state = engine_sites::at(base, cgame_state::SINGLETON_RVA)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .ok_or("input demand game state unreadable or address overflow")?;
    let state =
        usize::try_from(game_state).map_err(|_| "input demand game state address overflow")?;
    if state == 0 {
        return Ok(None);
    }
    let countries = state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand country array unreadable or invalid")?;
    Ok((countries.count >= 2).then_some(state))
}

pub fn demand_air_game_state(
    memory: &impl Memory,
    base: usize,
) -> Result<Option<usize>, &'static str> {
    let Some(state) = demand_game_state(memory, base)? else {
        return Ok(None);
    };
    let manager = state
        .checked_add(cgame_state::STRATEGIC_AIR_MANAGER)
        .and_then(|at| engine_native_memory::read::<u64>(memory, at))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("input demand strategic air manager unreadable")?;
    if manager == 0 {
        return Ok(None);
    }
    let slots = manager
        .checked_add(engine_native_air::cstrategic_air_manager::COUNTRY_AIR)
        .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand strategic air country roster unreadable or invalid")?;
    let countries = state
        .checked_add(cgame_state::COUNTRIES)
        .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
        .ok_or("input demand country array unreadable or invalid")?;
    Ok((countries.count >= 2 && slots.count >= countries.count).then_some(state))
}
