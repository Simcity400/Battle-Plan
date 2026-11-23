
use engine_sites::Rva;
use super::LEADER;
use engine_native_memory::{Memory, cgame_date, fixed5, read};
use crate::{
    carmy_leader,
    cgame_state,
    corders_group,
    creferenced_division_template,
    cunit_leader,
};
use engine_native_parts::creference_registry::HeaderResolver;

const COOLDOWN_TYPE: usize = 0xe84;
const COOLDOWN_END: usize = 0xea0;
const DEPLOYMENT_PENDING: usize = 0x19a;
const HQ_TEMPLATE: usize = 0x190;
const TEMPLATE_MANPOWER: usize = 0x190;
const DEPLOY_DAYS_RVA: Rva = Rva(0x033e_1fb8);
const DEPLOY_MIN_RVA: Rva = Rva(0x033e_2024);
const REFERENCE_MANPOWER_RVA: Rva = Rva(0x033e_22a8);
const MANPOWER_EXPONENT_RVA: Rva = Rva(0x033e_2338);

fn at_counter(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("HQ cooldown address overflow")
}

fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("HQ cooldown address overflow")
}

fn define<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    field(memory, site(base, key)?, 0)
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    read(memory, at_counter(pointer, offset)?).ok_or("HQ cooldown operand unreadable")
}

fn date(memory: &impl Memory, base: usize, pointer: usize) -> Result<i32, &'static str> {
    if field::<u64>(memory, pointer, 0)? != site(base, cgame_date::VTABLE_RVA)? as u64 {
        return Err("HQ cooldown date identity mismatch");
    }
    field(memory, pointer, cgame_date::DATE_BASE)
}

fn deployment_days(memory: &impl Memory, base: usize, manpower: i32) -> Result<i32, &'static str> {
    let days = define::<i32>(memory, base, DEPLOY_DAYS_RVA)?;
    let minimum = define::<i32>(memory, base, DEPLOY_MIN_RVA)?;
    if days < 1 {
        return Ok(0);
    }
    if manpower <= 0 {
        return Ok(days);
    }
    let reference = define::<i64>(memory, base, REFERENCE_MANPOWER_RVA)?;
    if reference <= 0 {
        return Ok(days);
    }
    let ratio = i64::from(manpower)
        .wrapping_mul(10_000_000_000)
        .checked_div(reference)
        .ok_or("HQ cooldown manpower division traps")?;
    let factor = fixed5::power(ratio, define(memory, base, MANPOWER_EXPONENT_RVA)?)?;
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
    registry: &mut HeaderResolver,
) -> Result<Option<u64>, &'static str> {
    if leader == 0 {
        return Ok(None);
    }
    let (kind, reference_type, id) = cunit_leader::identity(memory, base, leader)?;
    if kind != cunit_leader::KIND_ARMY || id == 0 {
        return Err("HQ cooldown needs an authenticated army leader");
    }
    let handle = u64::from(reference_type) | (u64::from(id) << 32);
    if registry.resolve(memory, base, handle)? != Some(leader) {
        return Err("HQ cooldown leader registry identity mismatch");
    }
    Ok(Some(handle))
}

#[cfg(test)]
pub(super) fn leader_enroute_days(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    group: usize,
) -> Result<i32, &'static str> {
    let mut registry = HeaderResolver::default();
    let pointer = direct_leader(memory, group);
    let leader =
        pointer.and_then(|pointer| authenticate_leader(memory, base, pointer, &mut registry));
    leader_enroute_observed(memory, base, game_state, pointer, leader, &mut registry)
}

pub(super) fn leader_enroute_observed(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    pointer: Result<usize, &'static str>,
    observed: Result<Option<u64>, &'static str>,
    registry: &mut HeaderResolver,
) -> Result<i32, &'static str> {
    let leader = pointer?;
    if observed?.is_none() {
        return Ok(0);
    }
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
                    != site(base, creferenced_division_template::VTABLE_RVA)? as u64
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
