
use crate::{Memory, cideology_group, read};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a1_a6e0);
pub const ID: usize = 0x08;
pub const IDEOLOGY_GROUP: usize = 0x18;
pub const TAG: usize = 0x20;
pub const NAME: usize = 0x28;
pub const LONG_NAME: usize = 0x48;
pub const NAME_DEFAULT: usize = 0x68;
pub const COUNTRY_LEADERS: usize = 0x70;
pub const COUNTRY_LEADER_CAPACITY: usize = 0x78;
pub const COUNTRY_LEADER_COUNT: usize = 0x7c;
pub const POPULARITY: usize = 0x88;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LeaderRow {
    pub country: u32,
    pub party: u32,
    pub ordinal: u32,
    pub character: Option<u32>,
    pub role: Option<i32>,
    pub leader_id: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub popularity: i64,
    pub is_ruling: u8,
    pub token_start: usize,
    pub token_len: usize,
    pub tag: u32,
    pub name_default: u8,
    pub name_start: usize,
    pub name_len: usize,
    pub long_name_start: usize,
    pub long_name_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub rows: usize,
    pub bytes: usize,
}

pub fn demand(memory: &impl Memory, base: usize, country: usize) -> Result<Demand, &'static str> {
    let walked =
        crate::cpolitical_status::Reader::new(memory, base, country)?.parties_carried(
            0,
            crate::walk::Buffer::Count,
            0,
            crate::walk::Buffer::Count,
            0,
            &mut crate::cpolitical_status::FreshParties,
        );
    let rows = match walked.outcome {
        crate::walk::Outcome::Rows { len } => len,
        crate::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("political party sizing row count overflow")?,
        crate::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    Ok(Demand {
        rows,
        bytes: walked.required_bytes,
    })
}

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
