//! NNavalCombatHit::SNavalHit. Constructor1415b38b0 and tooltip1415b46d0
//! preserve target slot/name, convoy flag, accumulated damage, denominator and
//! final-hit flag. The denominator is not a hit counter.
use crate::{Memory, cnaval_combatant::field, msvc_string};

/// Constructor instruction1415b3b10.
pub const VTABLE_RVA: usize = 0x029c_7148;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub target_slot: u32,
    pub target_name: msvc_string::Text<256>,
    pub convoy: u8,
    pub damage: i64,
    pub denominator: i64,
    pub last_hit: u8,
}

pub(crate) fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("naval hit module overflow")? as u64
    {
        return Err("naval hit vtable mismatch");
    }
    Ok(Row {
        target_slot: field(memory, pointer, 8)?,
        target_name: msvc_string::read_text(
            memory,
            pointer.checked_add(0x10).ok_or("naval hit text overflow")?,
        )
        .ok_or("naval hit target name invalid or exceeds capacity")?,
        convoy: field(memory, pointer, 0x30)?,
        damage: field(memory, pointer, 0x38)?,
        denominator: field(memory, pointer, 0x40)?,
        last_hit: field(memory, pointer, 0x48)?,
    })
}
