//! Air-region damage entry retaining signed i64 amounts.

use crate::{Memory, sair_wing_combat_data::Key};

/// Damage-entry vtable.
pub const VTABLE_RVA: usize = 0x02a9_9980;
/// Damage-entry array stride.
pub const STRIDE: usize = 0x18;
/// Cause type, u32.
pub const CAUSE_TYPE: usize = 0x08;
/// Raw signed i64 amount, without narrowing.
pub const AMOUNT: usize = 0x10;
/// Lookup/update matches the full u32 cause and adds to its signed i64 amount.
pub const UPDATE_RVA: usize = 0x019e_fa30;

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
