//! Combat-side division death before unit deletion, without game calls or allocation.
//!
//! Lifecycle processing follows removal of the triggering combat when retreat
//! is impossible or strength falls below the content-loaded threshold. It logs
//! the victim's remaining pool and shares leader experience among opposing units
//! without selecting a killer division. Tail-jump deletion bypasses the separate
//! selected-killer destruction route.
//!
//! The later leader on-action chooses its own country; it does not change casualty attribution.
//! Attribution uses the opposite side's first raw country. An empty side selects
//! the first raw link to reserved slot zero, or zero when no link matches. Keep
//! the triggering combat separate from remaining references because it is already
//! removed. Observation begins after land leader-experience effects and before
//! country selection; intervening unit and reserved-link accessors run no scripts.
//! No selected killer was observed, so killer attribution is unavailable rather
//! than a known null. This layout does not establish live hook coverage or cost.

use crate::{
    Memory, carmy, cgame_state, cland_border_war_combat, cland_border_war_combatant,
    cland_combat as combat, cland_combatant as side, cnaval_combatant, cpdx_array::Array,
    cprovince, cunit, read,
};

pub const RETURN_RVA: usize = 0x147f616;
/// Base lifecycle after land leader-experience effects. Naval combatants
/// share this virtual path and their vtable is excluded below.
pub const LIFECYCLE_RVA: usize = 0x147d070;
pub const LIFECYCLE_SENTINEL: [u8; 16] = [
    0x40, 0x53, 0x41, 0x57, 0x48, 0x83, 0xec, 0x58, 0x48, 0x8b, 0x02, 0x4c, 0x8b, 0xfa, 0x48, 0x89,
];
/// CPdxArray<u32>; base lifecycle loads its data and signed count.
const COUNTRY_ARRAY: usize = 0x68;

#[derive(Clone, Copy)]
pub struct WipePlan {
    pub words: usize,
    header: [u64; 9],
    trigger_pointer: usize,
    remaining: Array,
}

impl WipePlan {
    pub fn read(
        memory: &impl Memory,
        base: usize,
        combatant: usize,
        army: usize,
    ) -> Result<Option<Self>, &'static str> {
        let side_vtable = field::<u64>(memory, combatant, 0)?;
        if side_vtable == address(base, cnaval_combatant::VTABLE_RVA)? as u64 {
            return Ok(None);
        }
        let border = side_vtable == address(base, cland_border_war_combatant::VTABLE_RVA)? as u64;
        if !border && side_vtable != address(base, side::VTABLE_RVA)? as u64 {
            return Err("combat wipe class unsupported");
        }
        let expected_side = if border {
            cland_border_war_combatant::VTABLE_RVA
        } else {
            side::VTABLE_RVA
        };
        identity(memory, base, army, carmy::VTABLE_RVA)?;
        // Authenticate the division primary class discriminator.
        if field::<u32>(memory, army, cunit::CLASS)? != 0 {
            return Err("combat wipe unit class unsupported");
        }
        let parent = field::<u64>(memory, combatant, side::COMBAT)? as usize;
        identity(
            memory,
            base,
            parent,
            if border {
                cland_border_war_combat::VTABLE_RVA
            } else {
                combat::VTABLE_RVA
            },
        )?;
        let attacker = field::<u64>(memory, parent, combat::ATTACKER)? as usize;
        let defender = field::<u64>(memory, parent, combat::DEFENDER)? as usize;
        let flag = field::<u8>(memory, combatant, side::IS_ATTACKER)?;
        let opposite = match (combatant == attacker, combatant == defender, flag) {
            (true, false, 1) => defender,
            (false, true, 0) => attacker,
            _ => return Err("combat wipe side discriminator differs"),
        };
        identity(memory, base, opposite, expected_side)?;
        if field::<u64>(memory, opposite, side::COMBAT)? != parent as u64 {
            return Err("combat wipe opposite backlink differs");
        }
        let state = read::<u64>(memory, address(base, cgame_state::SINGLETON_RVA)?)
            .ok_or("combat wipe game state unreadable")? as usize;
        let hour = field::<i32>(memory, state, cgame_state::CURRENT_HOUR)?;
        let countries = Array::read(memory, address(opposite, COUNTRY_ARRAY)?, 4)
            .ok_or("combat wipe countries invalid")?;
        let killer = if countries.count == 0 {
            null_country(memory, state)?
        } else {
            field::<u32>(memory, countries.data, 0)?
        };
        let province = field::<u64>(memory, army, cunit::LOCATION)? as usize;
        let destination = field::<u64>(memory, parent, combat::PROVINCE)? as usize;
        identity(memory, base, province, cprovince::VTABLE_RVA)?;
        identity(memory, base, destination, cprovince::VTABLE_RVA)?;
        let remaining = Array::read(memory, address(army, cunit::COMBAT_REFERENCE_DATA)?, 8)
            .ok_or("combat wipe remaining references invalid")?;
        Ok(Some(Self {
            words: 10 + remaining.count as usize,
            header: [
                7,
                hour as u64,
                field(memory, army, cunit::HANDLE)?,
                u64::from(field::<u32>(memory, army, cunit::LOGICAL_OWNER)?),
                u64::from(killer),
                u64::from(field::<u32>(memory, province, cprovince::ID)?),
                u64::from(field::<u32>(memory, destination, cprovince::ID)?),
                field(memory, parent, combat::HANDLE)?,
                u64::from(remaining.count),
            ],
            trigger_pointer: parent,
            remaining,
        }))
    }

    /// Words include the caller's framing-size word, as for DestructionPlan.
    pub fn copy(
        self,
        memory: &impl Memory,
        base: usize,
        mut put: impl FnMut(u64),
    ) -> Result<(), &'static str> {
        for word in self.header {
            put(word);
        }
        for n in 0..self.remaining.count {
            let pointer = field::<u64>(
                memory,
                self.remaining
                    .element(n, 8)
                    .ok_or("combat wipe remaining offset overflow")?,
                0,
            )? as usize;
            if pointer == self.trigger_pointer {
                return Err("combat wipe triggering reference was not removed");
            }
            let vtable = field::<u64>(memory, pointer, 0)?;
            if vtable != address(base, combat::VTABLE_RVA)? as u64
                && vtable != address(base, cland_border_war_combat::VTABLE_RVA)? as u64
            {
                return Err("combat wipe remaining class unsupported");
            }
            put(field(memory, pointer, combat::HANDLE)?);
        }
        Ok(())
    }
}

fn null_country(memory: &impl Memory, state: usize) -> Result<u32, &'static str> {
    let links = Array::read(memory, address(state, cgame_state::COUNTRY_LINKS)?, 4)
        .ok_or("combat wipe null-country links invalid")?;
    for raw in 0..links.count {
        let at = links
            .element(raw, 4)
            .ok_or("combat wipe null-country offset overflow")?;
        if field::<u32>(memory, at, 0)? == 0 {
            return Ok(raw);
        }
    }
    Ok(0) // Preserve the reserved-link search's empty and no-match fallback.
}

fn address(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("combat wipe address overflow")
}
fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    if pointer == 0 {
        return Err("combat wipe null pointer");
    }
    read(memory, address(pointer, offset)?).ok_or("combat wipe field unreadable")
}
fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    if field::<u64>(memory, pointer, 0)? != address(base, vtable)? as u64 {
        return Err("combat wipe pointer identity differs");
    }
    Ok(())
}

#[cfg(test)]
#[path = "land_wipe_tests.rs"]
mod tests;
