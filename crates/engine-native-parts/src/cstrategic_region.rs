
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029e_a078);
pub const SECONDARY_VTABLE_RVA: Rva = Rva(0x029e_a0c8);
pub const SECONDARY: usize = 0x08;
pub const ID: usize = 0x58;
pub const RUNTIME_SLOT: usize = 0x60;
pub const PROVINCES: usize = 0x18;
pub const PROVINCES_COUNT: usize = 0x24;
pub const ROUTE_WEIGHT: usize = 0xe0;
pub const NAVAL_DOMINANCE: usize = 0xe8;
pub const AIR_PHASE: usize = 0x140;
pub const TEMPLATE: usize = 0x30;
pub const TEMPLATE_VTABLE_RVA: Rva = Rva(0x02a5_5348);
pub const TEMPLATE_MODIFIERS_CAPACITY: usize = 0x140;
pub const TEMPLATE_MODIFIERS_COUNT: usize = 0x144;

pub fn id(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: usize,
) -> Result<Option<u32>, &'static str> {
    if pointer == 0 {
        return Ok(None);
    }
    let expected = engine_sites::at(base, VTABLE_RVA)
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
