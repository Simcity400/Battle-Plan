//! Strategic-region fields relative to the primary object.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0296_d558;
/// Secondary vtable (`SECONDARY`).
pub const SECONDARY_VTABLE_RVA: usize = 0x0296_d5a8;
/// Secondary-subobject displacement; authenticate both vtables.
pub const SECONDARY: usize = 0x08;
/// Strategic-region ID, u32.
pub const ID: usize = 0x58;
/// Runtime roster slot supplied during world setup.
pub const RUNTIME_SLOT: usize = 0x60;
/// Province-pointer vector used by hourly context samples.
pub const PROVINCES: usize = 0x18;
/// Province count, i32, read before weather aggregation.
pub const PROVINCES_COUNT: usize = 0x24;
/// Naval-dominance container pointer, authenticated before use.
pub const NAVAL_DOMINANCE: usize = 0xe8;
/// Hourly air-update phase, signed i32, initialized to -1. Province admission
/// sets runtime slot modulo twice the loaded post-combat delay; hourly updates
/// compare it to the current hour.
pub const AIR_PHASE: usize = 0x140;
/// Strategic-region template pointer.
pub const TEMPLATE: usize = 0x30;
/// CStrategicRegionTemplate vtable, authenticated before the custom sea walk.
pub const TEMPLATE_VTABLE_RVA: usize = 0x029d_be50;
/// Template custom-modifier vector with array capacity and count.
pub const TEMPLATE_MODIFIERS_CAPACITY: usize = 0x140;
pub const TEMPLATE_MODIFIERS_COUNT: usize = 0x144;

/// Authenticates a nullable region pointer and reads its stored id (`ID`).
pub fn id(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<u32>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("strategic region module address overflow")? as u64;
    let address = pointer
        .checked_add(ID)
        .ok_or("strategic region address overflow")?;
    let mut identity = [0; 8];
    let mut id = [0; 4];
    if memory.copy_scattered([pointer, address], &mut [&mut identity, &mut id]) != 2 {
        return Err("strategic region identity or id unreadable");
    }
    if u64::from_le_bytes(identity) != expected {
        return Err("strategic region vtable mismatch");
    }
    Ok(Some(u32::from_le_bytes(id)))
}
