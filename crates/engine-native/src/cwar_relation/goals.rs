use engine_native_parts::cdiplomacy_status_layout as cdiplomacy_status;

use engine_sites::Rva;
use crate::{
    Memory, ccountry,  cpdx_array, cpdx_array::Array, cstate, read, walk::Outcome,
};

pub const VTABLE_RVA: Rva = Rva(0x02a5_9250);
pub const RECIPIENT: usize = 0x3c;
pub const GOAL_TYPE: usize = 0x40;
pub const TYPE_TOKEN: usize = 0x08;
pub const STATES: usize = 0x48;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub recipient: u32,
    pub goal_type: i32,
    pub state_start: usize,
    pub state_count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub states: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub state_len: usize,
    pub required: Demand,
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("owned wargoal field unreadable or address overflow")
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    output: &mut Vec<Row>,
    states: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(states),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut states: crate::walk::Buffer<'_, u32>,
) -> Walk {
    let mut len = 0;
    let mut state_len = 0_usize;
    let mut required = Demand::default();
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let mut found_country = false;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let diplomacy = usize::try_from(field::<u64>(
                memory,
                country.pointer,
                ccountry::DIPLOMACY_STATUS,
            )?)
            .map_err(|_| "owned wargoal diplomacy address overflow")?;
            if diplomacy == 0 {
                return Err("owned wargoal diplomacy root is null");
            }
            if engine_sites::at(base, cdiplomacy_status::VTABLE_RVA)
                != Some(field::<u64>(memory, diplomacy, 0)? as usize)
                || field::<u64>(memory, diplomacy, cdiplomacy_status::COUNTRY)?
                    != country.pointer as u64
            {
                return Err("owned wargoal diplomacy identity mismatch");
            }
            let goals = diplomacy
                .checked_add(cdiplomacy_status::OWNED_WARGOALS)
                .and_then(|at| Array::read(memory, at, 8))
                .ok_or("owned wargoal array unreadable or invalid")?;
            required.rows = required
                .rows
                .checked_add(goals.count as usize)
                .ok_or("owned wargoal row demand overflow")?;
            output.ensure(required.rows)?;
            cpdx_array::pointers(
                memory,
                goals,
                "owned wargoal pointer unreadable",
                |ordinal, goal| {
                    if goal == 0 {
                        return Err("owned wargoal pointer is null");
                    }
                    if engine_sites::at(base, VTABLE_RVA) != Some(field::<u64>(memory, goal, 0)? as usize)
                    {
                        return Err("owned wargoal vtable mismatch");
                    }
                    let source = goal
                        .checked_add(STATES)
                        .and_then(|at| Array::read(memory, at, 8))
                        .ok_or("owned wargoal state array unreadable or invalid")?;
                    required.states = required
                        .states
                        .checked_add(source.count as usize)
                        .ok_or("owned wargoal state demand overflow")?;
                    states.ensure(
                        state_len
                            .checked_add(source.count as usize)
                            .ok_or("owned wargoal state demand overflow")?,
                    )?;
                    if len == output.len()
                        || source.count as usize > states.len().saturating_sub(state_len)
                    {
                        unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("owned wargoal unread row count overflow")?;
                        return Ok(());
                    }
                    let recipient = field::<u32>(memory, goal, RECIPIENT)?;
                    let definition = usize::try_from(field::<u64>(memory, goal, GOAL_TYPE)?)
                        .ok()
                        .filter(|definition| *definition != 0)
                        .ok_or("owned wargoal type pointer is null or invalid")?;
                    let goal_type = field::<i32>(memory, definition, TYPE_TOKEN)?;
                    if goal_type < 0 {
                        return Err("owned wargoal type lexer token is negative");
                    }
                    let start = state_len;
                    cstate::copy_ids(
                        memory,
                        source,
                        &mut states[start..start + source.count as usize],
                        "owned wargoal state pointer unreadable",
                        "owned wargoal state id unreadable",
                    )?;
                    state_len += source.count as usize;
                    output[len] = Row {
                        country: country.id,
                        ordinal,
                        recipient,
                        goal_type,
                        state_start: start,
                        state_count: source.count as usize,
                    };
                    len += 1;
                    Ok(())
                },
            )?;
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    Walk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        state_len,
        required,
    }
}

pub mod inventory;
pub mod relation;

#[cfg(test)]
mod tests;

pub mod mutations;
