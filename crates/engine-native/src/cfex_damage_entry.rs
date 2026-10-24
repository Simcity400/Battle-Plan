//! Unnamed damage-attribution entry in the member's damage array (`cfex_member::DAMAGE_ENTRIES`).

/// CFEXMember damage-entry stride.
pub const STRIDE: usize = 0x20;
/// CFEXMember damage-entry source reference (two u32 words).
pub const SOURCE_HANDLE: usize = 0x08;
/// CFEXMember damage-entry country id u32.
pub const COUNTRY: usize = 0x10;
/// CFEXMember damage-entry amount i64.
pub const DAMAGE: usize = 0x18;

/// Received-damage entry primary vtable.
pub const VTABLE_RVA: usize = 0x02a2_0f38;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source: u64,
    pub country: u32,
    pub damage: i64,
}

pub(crate) fn read(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<Row, &'static str> {
    use crate::cnaval_combatant::field;
    if field::<u64>(memory, pointer, 0)?
        != base
            .checked_add(VTABLE_RVA)
            .ok_or("naval damage module overflow")? as u64
    {
        return Err("naval damage entry vtable mismatch");
    }
    Ok(Row {
        source: field(memory, pointer, SOURCE_HANDLE)?,
        country: field(memory, pointer, COUNTRY)?,
        damage: field(memory, pointer, DAMAGE)?,
    })
}
