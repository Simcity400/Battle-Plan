
use engine_native_memory::Memory;
use crate::sair_wing_combat_data::Key;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a9_9980);
pub const STRIDE: usize = 0x18;
pub const CAUSE_TYPE: usize = 0x08;
pub const AMOUNT: usize = 0x10;
pub const UPDATE_RVA: Rva = Rva(0x019e_fa30);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub record: Key,
    pub channel: u8,
    pub cause: u32,
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
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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
