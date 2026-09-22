//! CNavalCombatAirEntry retained result snapshot.140CD2A60 constructs0x88 bytes
//! and copies CFEXAir variant/counts/country/name plus three transferred hit lists.
use crate::{
    Memory,
    cnaval_combat_ship_entry::{HitBuffers, capture_hits},
    cnaval_combatant::field,
};

///140CD2FB7 LEA RIP+1C744E2 resolves to1429474A0 in the pinned executable.
pub const VTABLE_RVA: usize = 0x0294_74a0;
pub const SIZE: usize = 0x88;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DetailsRow {
    pub variant: u64,
    ///141DF1400 uses these exact variant+2A0/+2A8 operands in its summary.
    pub variant_summary_inputs: [i64; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub lost_planes: i32,
    pub country: u32,
    pub name: crate::msvc_string::Text<256>,
    pub naval_hits: [usize; 2],
    pub air_hits: [[usize; 2]; 2],
}

pub fn capture_details(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    buffers: &mut HitBuffers<'_>,
) -> Result<DetailsRow, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("naval result air module overflow")? as u64
    {
        return Err("naval result air vtable mismatch");
    }
    let variant = field::<u64>(memory, pointer, 8)? as usize;
    if variant == 0
        || field::<u64>(memory, variant, 0)?
            != base
                .checked_add(crate::cequipment_variant::VTABLE_RVA)
                .ok_or("naval result air variant module overflow")? as u64
    {
        return Err("naval result air variant identity mismatch");
    }
    let variant_handle = field::<u64>(memory, variant, 8)?;
    if variant_handle == 0 {
        return Err("naval result air variant handle is null");
    }
    let (naval_hits, air_hits) = capture_hits(memory, base, pointer, [0x40, 0x58, 0x70], buffers)?;
    Ok(DetailsRow {
        variant: variant_handle,
        variant_summary_inputs: [
            field(memory, variant, 0x2a0)?,
            field(memory, variant, 0x2a8)?,
        ],
        initial_planes: field(memory, pointer, 0x10)?,
        current_planes: field(memory, pointer, 0x14)?,
        lost_planes: field(memory, pointer, 0x18)?,
        country: field(memory, pointer, 0x1c)?,
        name: crate::msvc_string::read_text(
            memory,
            pointer
                .checked_add(0x20)
                .ok_or("naval result air name overflow")?,
        )
        .ok_or("naval result air name invalid or exceeds capacity")?,
        naval_hits,
        air_hits,
    })
}
