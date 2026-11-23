
use engine_native_memory::Memory;

pub const TAG: usize = 0x20;
pub const NAME: usize = 0x28;
pub const LONG_NAME: usize = 0x48;
pub const NAME_DEFAULT: usize = 0x68;
pub const COUNTRY_LEADER_CAPACITY: usize = 0x78;
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
            engine_native_memory::walk::Buffer::Count,
            0,
            engine_native_memory::walk::Buffer::Count,
            0,
            &mut crate::cpolitical_status::FreshParties,
        );
    let rows = match walked.outcome {
        engine_native_memory::walk::Outcome::Rows { len } => len,
        engine_native_memory::walk::Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("political party sizing row count overflow")?,
        engine_native_memory::walk::Outcome::Unavailable { reason } => return Err(reason),
    };
    Ok(Demand {
        rows,
        bytes: walked.required_bytes,
    })
}
