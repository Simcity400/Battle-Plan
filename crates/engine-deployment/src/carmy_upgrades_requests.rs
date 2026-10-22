//! Army-owned upgrade requests, including request rows and delivery pointers.
//! The owning army pointer authenticates membership independently of active
//! country registration.

/// Primary requests vtable.
pub const VTABLE_RVA: usize = 0x29bb380;
/// Priority, u32; serialization omits the default value one.
pub const PRIORITY: usize = 0x08;
/// Persistent subobject displacement.
pub const PERSISTENT_SUBOBJECT: usize = 0x18;
/// Persistent subobject vtable.
pub const PERSISTENT_VTABLE_RVA: usize = 0x29bb3b8;
/// Owning CArmy pointer requiring both vtables and persistent-id authentication.
pub const ARMY: usize = 0x20;
/// CArmyUpgradesStatus back-reference checked against the owning status.
pub const STATUS: usize = 0x28;
/// Request-row vector data.
pub const REQUESTS_DATA: usize = 0x30;
/// Request-row vector capacity, i32.
#[cfg(test)]
pub const REQUESTS_CAPACITY: usize = 0x38;
/// CUpgradeDelivery pointer-vector data.
pub const DELIVERIES_DATA: usize = 0x48;
/// Delivery-vector entry width; entries are CUpgradeDelivery pointers.
pub const DELIVERY_ENTRY_STRIDE: usize = 0x08;
/// Primary requests subobject, recovered from the serialized persistent base.
pub const IN_ARMY_REQUESTS: usize = 0x70;

use crate::{Memory, cequipment_variant_pool::DivisionSource, read};

/// An upgrade group exists even when both child vectors are empty. The army
/// handle is the engine's packed reference, with type low and id high.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Country whose authenticated division roster contains the owning army.
    pub country: u32,
    pub army: u64,
    pub priority: u32,
}

/// The source division was authenticated by shared acquisition. Its owned
/// group is present independently of the country's active-request registry.
pub(crate) fn row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    source: &DivisionSource,
) -> Result<Row, &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    let expected_persistent = base
        .checked_add(PERSISTENT_VTABLE_RVA)
        .ok_or("module address overflow")?;
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
