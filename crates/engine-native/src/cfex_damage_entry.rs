
use engine_sites::Rva;
pub const STRIDE: usize = 0x20;
pub const SOURCE_HANDLE: usize = 0x08;
pub const COUNTRY: usize = 0x10;
pub const DAMAGE: usize = 0x18;

pub const VTABLE_RVA: Rva = Rva(0x02a9_a208);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source: u64,
    pub country: u32,
    pub damage: i64,
}

pub fn read(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: usize,
) -> Result<Row, &'static str> {
    use crate::cnaval_combatant::field;
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA)
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
