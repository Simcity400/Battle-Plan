//! Current HQ cooldown for the counter's commander (`1418C00D0`).
//!
//! CArmyLeader slot26 (`140C09F40`) combines the tracked date with deployment
//! cooldown (`14159DC20` -> `14159DDE0`). The latter follows the leader's own
//! formation reference; group+190 is a referenced HQ division template,
//! established by `140BE6E60`, and template+190 is its manpower operand.
//! All reads are passive and callback-local; no rendered text or UI cache is
//! copied. The existing UnitLeaders name at +40 supplies the tooltip's name.

use super::LEADER;
use crate::{
    Memory, carmy_leader, cgame_date, cgame_state, corders_group,
    creference_registry::HeaderResolver, creferenced_division_template, cunit_leader, fixed5, read,
};

// 140C09F40 / 140C0A130: tracked modifier kind and CGameDate end.
const COOLDOWN_TYPE: usize = 0xe84;
const COOLDOWN_END: usize = 0xea0;
// 140C09F40's formation+19A selects the pending deployment contribution.
const DEPLOYMENT_PENDING: usize = 0x19a;
// 140BDFEF0 resolves group+190; 140BE6E60 writes referenced template+8.
const HQ_TEMPLATE: usize = 0x190;
const TEMPLATE_MANPOWER: usize = 0x190;
// 14159DC20 loads signed dword defines; 14159DDE0 loads the qword pair.
const DEPLOY_DAYS_RVA: usize = 0x0331_e4ec;
const DEPLOY_MIN_RVA: usize = 0x0331_e56c;
const REFERENCE_MANPOWER_RVA: usize = 0x0331_e760;
const MANPOWER_EXPONENT_RVA: usize = 0x0331_e7e8;

fn at_counter(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("HQ cooldown address overflow")
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at_counter(pointer, offset)?).ok_or("HQ cooldown operand unreadable")
}

fn date(memory: &impl Memory, base: usize, pointer: usize) -> Result<i32, &'static str> {
    if field::<u64>(memory, pointer, 0)? != at_counter(base, cgame_date::VTABLE_RVA)? as u64 {
        return Err("HQ cooldown date identity mismatch");
    }
    field(memory, pointer, cgame_date::DATE_BASE)
}

fn deployment_days(memory: &impl Memory, base: usize, manpower: i32) -> Result<i32, &'static str> {
    let days = field::<i32>(memory, base, DEPLOY_DAYS_RVA)?;
    let minimum = field::<i32>(memory, base, DEPLOY_MIN_RVA)?;
    if days < 1 {
        return Ok(0);
    }
    if manpower <= 0 {
        return Ok(days);
    }
    let reference = field::<i64>(memory, base, REFERENCE_MANPOWER_RVA)?;
    if reference <= 0 {
        return Ok(days);
    }
    let ratio = i64::from(manpower)
        .wrapping_mul(10_000_000_000)
        .checked_div(reference)
        .ok_or("HQ cooldown manpower division traps")?;
    let factor = fixed5::power(ratio, field(memory, base, MANPOWER_EXPONENT_RVA)?)?;
    let value = i64::from(days).wrapping_mul(factor);
    let rounded = value.wrapping_add(if value < 0 { -50_000 } else { 50_000 }) / fixed5::SCALE;
    Ok((rounded as i32).max(minimum))
}

pub(super) fn direct_leader(memory: &impl Memory, group: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, group, LEADER)?)
        .map_err(|_| "HQ cooldown leader pointer overflow")
}

pub(super) fn authenticate_leader(
    memory: &impl Memory,
    base: usize,
    leader: usize,
) -> Result<Option<u64>, &'static str> {
    if leader == 0 {
        return Ok(None);
    }
    let (kind, reference_type, id) = cunit_leader::identity(memory, base, leader)?;
    if kind != cunit_leader::KIND_ARMY || id == 0 {
        return Err("HQ cooldown needs an authenticated army leader");
    }
    let handle = u64::from(reference_type) | (u64::from(id) << 32);
    if HeaderResolver::default().resolve(memory, base, handle)? != Some(leader) {
        return Err("HQ cooldown leader registry identity mismatch");
    }
    Ok(Some(handle))
}

pub(super) fn leader_enroute_days(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    group: usize,
) -> Result<i32, &'static str> {
    let pointer = direct_leader(memory, group);
    let leader = pointer.and_then(|pointer| authenticate_leader(memory, base, pointer));
    leader_enroute_observed(memory, base, game_state, pointer, leader)
}

pub(super) fn leader_enroute_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    pointer: Result<usize, &'static str>,
    observed: Result<Option<u64>, &'static str>,
) -> Result<i32, &'static str> {
    let leader = pointer?;
    if observed?.is_none() {
        return Ok(0);
    }
    let mut registry = HeaderResolver::default();
    let mut days = if field::<i32>(memory, leader, COOLDOWN_TYPE)? == 0 {
        0
    } else {
        if game_state == 0 {
            return Err("HQ cooldown current game date absent");
        }
        let end = date(memory, base, at_counter(leader, COOLDOWN_END)?)?;
        let now = date(
            memory,
            base,
            at_counter(game_state, cgame_state::CURRENT_DATE)?,
        )?;
        end.wrapping_add(23_i32.wrapping_sub(now)) / 24
    };
    let formation = field::<u64>(memory, leader, carmy_leader::FORMATION_REFERENCE)?;
    let Some(formation_pointer) = registry.resolve(memory, base, formation)? else {
        return Ok(days);
    };
    if formation as u32 != corders_group::REFERENCE_TYPE
        || formation >> 32 == 0
        || corders_group::plan_pointer_handle(memory, base, formation_pointer)? != formation
    {
        return Err("HQ cooldown formation identity mismatch");
    }
    if field::<u8>(memory, formation_pointer, DEPLOYMENT_PENDING)? != 0 {
        let template_handle = field::<u64>(memory, formation_pointer, HQ_TEMPLATE)?;
        let manpower = if let Some(template) = registry.resolve(memory, base, template_handle)? {
            if template_handle as u32 != creferenced_division_template::REFERENCE_TYPE
                || template_handle >> 32 == 0
                || field::<u64>(memory, template, 0)?
                    != at_counter(base, creferenced_division_template::VTABLE_RVA)? as u64
                || field::<u64>(memory, template, creferenced_division_template::TYPE)?
                    != template_handle
            {
                return Err("HQ cooldown division template identity mismatch");
            }
            field::<i32>(memory, template, TEMPLATE_MANPOWER)?
        } else {
            0
        };
        days = days.wrapping_add(deployment_days(memory, base, manpower)?);
    }
    Ok(days)
}

#[cfg(test)]
mod tests;
