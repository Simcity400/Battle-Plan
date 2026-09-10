//! Native request-vector entry in CArmyUpgradesRequests; engine type name unknown.

/// CArmyUpgradesRequests request-row stride; old D93 lib.rs:29 and read_request_rows use 0x18 bytes.
pub const STRIDE: usize = 0x18;
/// Upgrade request row's CEquipmentVariant pointer; old D93 read_request_rows reads the first qword and checks its vtable.
pub const EQUIPMENT_VARIANT: usize = 0x00;
/// Upgrade request amount, i64 in engine fixed point; old D93 read_request_rows reads +0x08 without narrowing.
pub const REQUEST: usize = 0x08;
/// Upgrade request total, i64 in engine fixed point; old D93 read_request_rows reads +0x10 without narrowing.
pub const TOTAL: usize = 0x10;
/// Upgrade request numeric scale; old D93 lib.rs:65 records native fixed point with denominator 100,000.
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
