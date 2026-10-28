//! Retained naval-air result snapshot, 0x88 bytes. It copies variant,
//! counts, country and name with three transferred hit arrays.
use crate::capture_text::Value;
use crate::{
    Memory,
    cnaval_combat_ship_entry::{HitBuffers, capture_hits},
    cnaval_combatant::field,
};

/// Naval-combat air-entry primary vtable.
pub const VTABLE_RVA: usize = 0x029d_a5a0;
pub const SIZE: usize = 0x88;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DetailsRow {
    pub variant: u64,
    /// Variant-summary operands.
    pub variant_summary_inputs: [i64; 2],
    pub initial_planes: i32,
    pub current_planes: i32,
    pub lost_planes: i32,
    pub country: u32,
    pub name: crate::capture_text::Owned,
    pub naval_hits: [usize; 2],
    pub air_hits: [[usize; 2]; 2],
}
impl DetailsRow {
    pub fn allocated_bytes(&self) -> usize {
        self.name.allocated_bytes()
    }
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
        name: crate::capture_text::Owned::read(
            memory,
            pointer
                .checked_add(0x20)
                .ok_or("naval result air name overflow")?,
            false,
        )
        .map_err(|reason| {
            if reason == "string allocation failed" {
                reason
            } else {
                "naval result air name invalid or unreadable"
            }
        })?,
        naval_hits,
        air_hits,
    })
}
