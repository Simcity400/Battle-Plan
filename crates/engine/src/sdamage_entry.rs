//! SDamageEntry, pinned a729; damage remains native i64.

use crate::{Memory, sair_wing_combat_data::Key};

/// SDamageEntry primary vtable; executable RTTI confirms old AIR_REGION_DAMAGE_ENTRY identity.
pub const VTABLE_RVA: usize = 0x02a0_9990;
/// SDamageEntry array stride; sample_compact_air_region_combat_native advances by 0x18 bytes.
pub const STRIDE: usize = 0x18;
/// SDamageEntry u32 cause type; sample_compact_air_region_combat_native reads entry+8.
pub const CAUSE_TYPE: usize = 0x08;
/// SDamageEntry i64 raw amount; sample_compact_air_region_combat_native reads entry+10 without narrowing.
pub const AMOUNT: usize = 0x10;
pub const UPDATE_RVA: usize = 0x0194_f430;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub record: Key,
    /// Native first or second damage vector, preserved without relabelling.
    pub channel: u8,
    pub cause: u32,
    /// Full signed engine representation; no display conversion or clamping.
    pub amount: i64,
}

pub(crate) fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    record: Key,
    channel: u8,
) -> Result<Row, &'static str> {
    let mut head = [0; 12];
    let mut amount = [0; 8];
    let amount_address = address
        .checked_add(AMOUNT)
        .ok_or("air damage entry address overflow")?;
    if memory.copy_scattered([address, amount_address], &mut [&mut head, &mut amount]) != 2 {
        return Err("air damage entry fields unreadable");
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("air damage entry vtable mismatch");
    }
    Ok(Row {
        record,
        channel,
        cause: u32::from_le_bytes(head[CAUSE_TYPE..CAUSE_TYPE + 4].try_into().unwrap()),
        amount: i64::from_le_bytes(amount),
    })
}
