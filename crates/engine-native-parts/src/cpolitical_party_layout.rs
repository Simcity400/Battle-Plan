use engine_sites::Rva;
use engine_native_memory::{Memory, read};
use crate::cideology_group;

pub const VTABLE_RVA: Rva = Rva(0x02a1_a6e0);

pub const ID: usize = 0x08;

pub const IDEOLOGY_GROUP: usize = 0x18;

pub const COUNTRY_LEADERS: usize = 0x70;

pub const COUNTRY_LEADER_COUNT: usize = 0x7c;

pub fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(u32, usize), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let header = read::<[u8; 12]>(memory, pointer).ok_or("political party identity unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("political party vtable mismatch");
    }
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("political party address overflow")
    };
    let id = u32::from_le_bytes(header[ID..].try_into().unwrap());
    let ideology = read::<u64>(memory, at(IDEOLOGY_GROUP)?)
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("political party ideology pointer unreadable or null")?;
    let expected =
        engine_sites::at(base, cideology_group::VTABLE_RVA).ok_or("module address overflow")?;
    let header = read::<[u8; 12]>(memory, ideology).ok_or("ideology identity unreadable")?;
    if u64::from_le_bytes(header[..8].try_into().unwrap()) != expected as u64 {
        return Err("political party ideology vtable mismatch");
    }
    let ideology_id = u32::from_le_bytes(header[cideology_group::ID..].try_into().unwrap());
    if ideology_id != id {
        return Err("political party and ideology ids differ");
    }
    Ok((id, ideology))
}
