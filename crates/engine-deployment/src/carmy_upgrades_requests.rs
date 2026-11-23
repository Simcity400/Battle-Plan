
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x2a4b9a0);
pub const PRIORITY: usize = 0x08;
pub const PERSISTENT_SUBOBJECT: usize = 0x18;
pub const PERSISTENT_VTABLE_RVA: Rva = Rva(0x2a4b9d8);
pub const ARMY: usize = 0x20;
pub const STATUS: usize = 0x28;
pub const REQUESTS_DATA: usize = 0x30;
#[cfg(test)]
pub const REQUESTS_CAPACITY: usize = 0x38;
pub const DELIVERIES_DATA: usize = 0x48;
pub const DELIVERY_ENTRY_STRIDE: usize = 0x08;
pub const IN_ARMY_REQUESTS: usize = 0x70;

use engine_native_memory::{Memory, read};
use engine_native::cequipment_variant::pool::DivisionSource;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub army: u64,
    pub priority: u32,
}

pub(crate) fn row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    source: &DivisionSource,
) -> Result<Row, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let expected_persistent =
        engine_sites::at(base, PERSISTENT_VTABLE_RVA).ok_or("module address overflow")?;
    let mut primary = [0_u8; 12];
    let mut persistent = [0_u8; 16];
    let persistent_at = address
        .checked_add(PERSISTENT_SUBOBJECT)
        .ok_or("army upgrade group address overflow")?;
    if memory.copy_scattered(
        [address, persistent_at],
        &mut [&mut primary, &mut persistent],
    ) != 2
    {
        return Err("army upgrade group identity unreadable");
    }
    if u64::from_le_bytes(primary[..8].try_into().unwrap()) != expected as u64
        || u64::from_le_bytes(persistent[..8].try_into().unwrap()) != expected_persistent as u64
    {
        return Err("army upgrade group vtable mismatch");
    }
    if source.pointer == 0
        || source.division == 0
        || u64::from_le_bytes(persistent[8..].try_into().unwrap()) != source.pointer as u64
    {
        return Err("army upgrade group army back-reference mismatch");
    }
    Ok(Row {
        country: source.country,
        army: source.division,
        priority: u32::from_le_bytes(primary[PRIORITY..].try_into().unwrap()),
    })
}

pub(crate) fn pointer(memory: &impl Memory, address: usize) -> Result<usize, &'static str> {
    read::<u64>(memory, address)
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("army upgrade graph pointer unreadable or null")
}
