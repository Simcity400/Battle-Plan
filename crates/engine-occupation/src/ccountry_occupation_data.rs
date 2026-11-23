
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02a0_0dc8);
pub const PARENT_STATUS: usize = 0x08;
pub const OCCUPIED_COUNTRY: usize = 0x10;
pub const OCCUPIED_COUNTRY_POINTER: usize = 0x18;
pub const GROUP_LAW: usize = 0xa0;
pub const LAW_TABLE: usize = 0xb0;
pub const GARRISON_TABLE: usize = 0xd8;
pub const GARRISON_CAPACITY_OR_MASK: usize = 0xe4;

use crate::ccountry_occupation_status::Rows;
use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native::{ccountry, cgame_state, cstate};
use crate::{coccupation_law, pdx_robin_hood_table};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LawRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub name_start: usize,
    pub name_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GroupLawRow {
    pub controller: u32,
    pub occupied: u32,
    pub scope_country: Option<u32>,
    pub present: u8,
    pub name_start: usize,
    pub name_len: usize,
}

pub(crate) fn group_law(
    memory: &impl Memory,
    base: usize,
    scope: &Scope,
    output: &mut Rows<'_, GroupLawRow>,
    names: &mut coccupation_law::Names<'_>,
) -> Result<(), &'static str> {
    let pointer = scope
        .pointer
        .checked_add(GROUP_LAW)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("occupied-country law pointer unreadable")?;
    let scope_country = scope
        .pointer
        .checked_add(OCCUPIED_COUNTRY_POINTER)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| {
            engine_sites::at(base, ccountry::VTABLE_RVA)
                .is_some_and(|v| read::<u64>(memory, p) == Some(v as u64))
        })
        .and_then(|p| p.checked_add(ccountry::ID))
        .and_then(|p| read::<u32>(memory, p));
    let mut row = GroupLawRow {
        controller: scope.controller,
        occupied: scope.occupied,
        scope_country,
        ..Default::default()
    };
    if pointer != 0 {
        let token = coccupation_law::literal(memory, base, pointer)?;
        let Some(start) = names.copy(memory, token, output.fits()?)? else {
            return output.omit();
        };
        row.present = 1;
        row.name_start = start;
        row.name_len = token.len;
    }
    output.push(row)
}

pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    parent: usize,
    entry: pdx_robin_hood_table::Entry,
    countries: &ccountry::Reader<'_, impl Memory>,
) -> Result<u32, &'static str> {
    let fields =
        read::<[u8; 20]>(memory, entry.value).ok_or("occupation data identity unreadable")?;
    let vtable = u64::from_le_bytes(fields[..8].try_into().unwrap());
    let backref = u64::from_le_bytes(fields[PARENT_STATUS..PARENT_STATUS + 8].try_into().unwrap());
    let occupied = u32::from_le_bytes(fields[OCCUPIED_COUNTRY..].try_into().unwrap());
    if engine_sites::at(base, VTABLE_RVA).map(|p| p as u64) != Some(vtable) {
        return Err("occupation data vtable mismatch");
    }
    if backref != parent as u64 {
        return Err("occupation data parent mismatch");
    }
    if countries.resolve(occupied)?.canonical_slot != entry.key {
        return Err("occupation bucket key differs from occupied country slot");
    }
    Ok(occupied)
}

pub(crate) fn state_array(memory: &impl Memory, game_state: usize) -> Result<Array, &'static str> {
    let array = game_state
        .checked_add(cgame_state::STATES)
        .and_then(|p| Array::read(memory, p, 8))
        .ok_or("occupation state array invalid")?;
    if array.count < 2 {
        return Err("occupation state array has no state slots");
    }
    Ok(array)
}

pub(crate) fn state_pointer(
    memory: &impl Memory,
    base: usize,
    states: Array,
    id: u32,
) -> Result<usize, &'static str> {
    if id == 0 {
        return Err("occupation state id is zero");
    }
    let pointer = states
        .element(id, 8)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .ok_or("occupation state slot unreadable")?;
    let expected =
        engine_sites::at(base, cstate::VTABLE_RVA).ok_or("occupation state vtable mismatch")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("occupation state vtable mismatch");
    }
    if pointer
        .checked_add(cstate::ID)
        .and_then(|p| read::<u32>(memory, p))
        != Some(id)
    {
        return Err("occupation state id differs from slot");
    }
    Ok(pointer)
}

pub(crate) struct Scope {
    pub controller: u32,
    pub occupied: u32,
    pub pointer: usize,
}

pub(crate) fn laws(
    memory: &impl Memory,
    base: usize,
    scope: &Scope,
    output: &mut Rows<'_, LawRow>,
    names: &mut coccupation_law::Names<'_>,
) -> Result<(), &'static str> {
    let table = scope
        .pointer
        .checked_add(LAW_TABLE)
        .ok_or("occupation law table address overflow")?;
    pdx_robin_hood_table::visit(memory, table, |entry| {
        let token = coccupation_law::literal(memory, base, entry.value)?;
        match names.copy(memory, token, output.fits()?)? {
            Some(name_start) => output.push(LawRow {
                controller: scope.controller,
                occupied: scope.occupied,
                state: entry.key,
                name_start,
                name_len: token.len,
            }),
            None => output.omit(),
        }
    })
}
