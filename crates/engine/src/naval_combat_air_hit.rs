//! Retained naval aircraft-loss entry with a complete 0x18-byte layout.
use crate::{Memory, cequipment_variant, cnaval_combatant::field};

/// Aircraft-loss entry primary vtable.
pub const VTABLE_RVA: usize = 0x029c_7198;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub count: i32,
    pub country: u32,
}

pub(crate) fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("naval air hit module overflow")? as u64
    {
        return Err("naval air hit vtable mismatch");
    }
    let variant = field::<u64>(memory, pointer, 8)? as usize;
    if variant == 0
        || field::<u64>(memory, variant, 0)?
            != base
                .checked_add(cequipment_variant::VTABLE_RVA)
                .ok_or("naval air hit variant module overflow")? as u64
    {
        return Err("naval air hit variant identity mismatch");
    }
    Ok(Row {
        variant: field(memory, variant, 8)?,
        count: field(memory, pointer, 0x10)?,
        country: field(memory, pointer, 0x14)?,
    })
}
