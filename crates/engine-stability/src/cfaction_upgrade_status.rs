
use crate::cmodifier;
use engine_native::{
    Memory,
    cadvisor,
    ccharacter,
    cfaction_member_status,
    cgame_date,
    cpdx_array::Array,
    creference_registry,
    read,
};
use engine_native_politics::{cdoctrine_sharing_status, ctechnology_sharing_faction};
use engine_sites::Rva;

pub const STATUS: usize = 0x838;
pub const VTABLE_RVA: Rva = Rva(0x02a3_87d0);
pub const MEMBER_UPGRADES: usize = 0x08;
pub const RESEARCH_SHARING: usize = 0x28;
pub const DOCTRINE_SHARING: usize = 0x78;
pub const SLOTS: usize = 0x98;
pub const OWNER: usize = 0xb0;
pub const UNSET_CHANGE_HOUR_RVA: Rva = Rva(0x30feb68);
pub const SLOT_STRIDE: usize = 0x1a8;
pub const SLOT_ADVISOR: usize = 0;
pub const SLOT_COUNTRY: usize = 0x08;
pub const SLOT_SPYMASTER_DATE: usize = 0x10;
pub const SLOT_SPYMASTER_MODIFIER: usize = 0x28;
pub const SLOT_MODIFIER: usize = 0xe8;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Slot {
    pub owner_country: u32,
    pub advisor_character: Option<u64>,
    pub spymaster_change_hour: i32,
    pub spymaster: cmodifier::Snapshot,
    pub modifier: cmodifier::Snapshot,
}

fn at(address: usize, offset: usize) -> Result<usize, &'static str> {
    address
        .checked_add(offset)
        .ok_or("faction upgrade address overflow")
}
fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("faction upgrade address overflow")
}

fn advisor(memory: &impl Memory, base: usize, slot: usize) -> Result<Option<u64>, &'static str> {
    let pointer = read::<u64>(memory, at(slot, SLOT_ADVISOR)?)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("faction upgrade advisor pointer unreadable")?;
    if pointer == 0 {
        return Ok(None);
    }
    if read::<u64>(memory, pointer) != Some(site(base, cadvisor::VTABLE_RVA)? as u64) {
        return Err("faction upgrade advisor vtable mismatch");
    }
    let character = read::<u64>(memory, at(pointer, cadvisor::CHARACTER)?)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value != 0)
        .ok_or("faction upgrade advisor character unreadable or null")?;
    if read::<u64>(memory, character) != Some(site(base, ccharacter::VTABLE_RVA)? as u64) {
        return Err("faction upgrade advisor character vtable mismatch");
    }
    let handle = read::<u64>(memory, at(character, ccharacter::TYPE)?)
        .filter(|value| *value != 0)
        .ok_or("faction upgrade advisor character handle unreadable or null")?;
    if creference_registry::resolve(memory, base, handle)? != Some(character) {
        return Err("faction upgrade advisor character reference mismatch");
    }
    Ok(Some(handle))
}

#[derive(Clone, Copy)]
pub struct Status {
    address: usize,
    faction: usize,
    base: usize,
}

pub fn authenticate(
    memory: &impl Memory,
    base: usize,
    faction: usize,
) -> Result<Status, &'static str> {
    let status = at(faction, STATUS)?;
    if read::<u64>(memory, status) != Some(site(base, VTABLE_RVA)? as u64) {
        return Err("faction upgrade status vtable mismatch");
    }
    if read::<u64>(memory, at(status, OWNER)?) != Some(faction as u64) {
        return Err("faction upgrade status owner mismatch");
    }
    Ok(Status {
        address: status,
        faction,
        base,
    })
}

impl Status {
    pub fn member_upgrades_into(
        &self,
        memory: &impl Memory,
        output: &mut Vec<cfaction_member_status::Upgrade>,
    ) -> Result<(), &'static str> {
        cfaction_member_status::read_upgrades(
            memory,
            self.base,
            at(self.address, MEMBER_UPGRADES)?,
            output,
        )
    }

    pub fn research_into(
        &self,
        memory: &impl Memory,
        output: &mut ctechnology_sharing_faction::Snapshot,
    ) -> Result<(), &'static str> {
        ctechnology_sharing_faction::read_into(
            memory,
            self.base,
            self.faction,
            at(self.address, RESEARCH_SHARING)?,
            output,
        )
    }

    pub fn doctrine_into(
        &self,
        memory: &impl Memory,
        output: &mut Vec<u32>,
    ) -> Result<(), &'static str> {
        cdoctrine_sharing_status::read_into(
            memory,
            self.base,
            at(self.address, DOCTRINE_SHARING)?,
            output,
        )
    }

    pub fn slots(&self, memory: &impl Memory) -> Result<Array, &'static str> {
        Array::read(memory, at(self.address, SLOTS)?, SLOT_STRIDE)
            .ok_or("faction upgrade slots unreadable or invalid")
    }

    pub fn slot_into(
        &self,
        memory: &impl Memory,
        slots: Array,
        ordinal: u32,
        slot: &mut Slot,
    ) -> Result<(), &'static str> {
        let address = slots
            .element(ordinal, SLOT_STRIDE)
            .ok_or("faction upgrade slot outside its array")?;
        slot.owner_country = read(memory, at(address, SLOT_COUNTRY)?)
            .ok_or("faction upgrade slot owner unreadable")?;
        slot.advisor_character = advisor(memory, self.base, address)?;
        slot.spymaster_change_hour =
            cgame_date::read_raw(memory, self.base, at(address, SLOT_SPYMASTER_DATE)?)?;
        cmodifier::read_into(
            memory,
            self.base,
            at(address, SLOT_SPYMASTER_MODIFIER)?,
            &mut slot.spymaster,
        )?;
        cmodifier::read_into(
            memory,
            self.base,
            at(address, SLOT_MODIFIER)?,
            &mut slot.modifier,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
