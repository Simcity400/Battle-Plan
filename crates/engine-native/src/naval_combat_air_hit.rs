//! Retained naval aircraft-loss entry with a complete 0x18-byte layout.
use crate::{Memory, cequipment_variant, cnaval_combatant::field};
use engine_sites::Rva;

/// Aircraft-loss entry primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a5_6fd8);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub count: i32,
    pub country: u32,
}

/// The hit's fields [`read`] reads (vtable, variant, count, country), one
/// record per hit for a run of hits (`cfex_member::append_entries`).
pub const RECORD: [crate::memory::RecordField; 1] = [crate::memory::RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 0x18,
}];

pub fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA).ok_or("naval air hit module withheld")? as u64
    {
        return Err("naval air hit vtable mismatch");
    }
    let variant = field::<u64>(memory, pointer, 8)? as usize;
    if variant == 0
        || field::<u64>(memory, variant, 0)?
            != engine_sites::at(base, cequipment_variant::VTABLE_RVA)
                .ok_or("naval air hit variant module withheld")? as u64
    {
        return Err("naval air hit variant identity mismatch");
    }
    Ok(Row {
        variant: field(memory, variant, 8)?,
        count: field(memory, pointer, 0x10)?,
        country: field(memory, pointer, 0x14)?,
    })
}
