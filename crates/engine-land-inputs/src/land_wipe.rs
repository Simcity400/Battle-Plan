
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{
    carmy,
    cgame_state,
    cland_combat as combat,
    cland_combatant as side,
    cnaval_combatant,
    cprovince,
    cunit,
};
use engine_native_parts::{cland_border_war_combat, cland_border_war_combatant};
use engine_sites::Rva;

pub const RETURN_RVA: Rva = Rva(0x147fa46);
pub const LIFECYCLE_RVA: Rva = Rva(0x147d4a0);
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
        let side_vtable = field::<u64>(memory, combatant, 0)? as usize;
        let classes = [
            cnaval_combatant::VTABLE_RVA,
            cland_border_war_combatant::VTABLE_RVA,
            side::VTABLE_RVA,
        ];
        let border = match engine_sites::which(base, side_vtable, &classes) {
            Some(cnaval_combatant::VTABLE_RVA) => return Ok(None),
            Some(cland_border_war_combatant::VTABLE_RVA) => true,
            Some(_) => false,
            None => return Err("combat wipe class unsupported"),
        };
        let expected_side = if border {
            cland_border_war_combatant::VTABLE_RVA
        } else {
            side::VTABLE_RVA
        };
        identity(memory, base, army, carmy::VTABLE_RVA)?;
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
        let state = read::<u64>(memory, module(base, cgame_state::SINGLETON_RVA)?)
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
            let vtable = field::<u64>(memory, pointer, 0)? as usize;
            let classes = [combat::VTABLE_RVA, cland_border_war_combat::VTABLE_RVA];
            if engine_sites::which(base, vtable, &classes).is_none() {
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
    Ok(0)
}

fn address(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("combat wipe address overflow")
}
fn module(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("combat wipe address overflow")
}
fn field<T: engine_native_memory::memory::Raw>(
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
    vtable: Rva,
) -> Result<(), &'static str> {
    if field::<u64>(memory, pointer, 0)? != module(base, vtable)? as u64 {
        return Err("combat wipe pointer identity differs");
    }
    Ok(())
}

#[cfg(test)]
#[path = "land_wipe_tests.rs"]
mod tests;
