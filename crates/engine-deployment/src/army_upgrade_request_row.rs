//! Request-vector entry in army upgrade requests; its concrete type is unknown.

/// Request-row stride.
pub const STRIDE: usize = 0x18;
/// Equipment-variant pointer, authenticated by its vtable.
pub const EQUIPMENT_VARIANT: usize = 0x00;
/// Request amount, signed i64 fixed point.
pub const REQUEST: usize = 0x08;
/// Total amount, signed i64 fixed point.
pub const TOTAL: usize = 0x10;
/// Fixed-point denominator.
pub const FIXED_POINT_SCALE: i64 = 100_000;

use crate::{Memory, cequipment_variant, read};

/// One ordered request in its army's current upgrade group. The engine does
/// not assign a persistent identity to a request entry; ordinal is local to
/// the group and is retained rather than inferred from equipment identity.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub army: u64,
    pub ordinal: u32,
    pub equipment: u64,
    pub request_raw: i64,
    pub total_raw: i64,
}

pub(crate) fn row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    country: u32,
    army: u64,
    ordinal: u32,
) -> Result<Row, &'static str> {
    let bytes = read::<[u8; STRIDE]>(memory, address).ok_or("army upgrade request unreadable")?;
    let pointer = usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
        .map_err(|_| "army upgrade equipment pointer overflow")?;
    Ok(Row {
        country,
        army,
        ordinal,
        equipment: cequipment_variant::identity(memory, base, pointer)?,
        request_raw: i64::from_le_bytes(bytes[REQUEST..TOTAL].try_into().unwrap()),
        total_raw: i64::from_le_bytes(bytes[TOTAL..STRIDE].try_into().unwrap()),
    })
}
