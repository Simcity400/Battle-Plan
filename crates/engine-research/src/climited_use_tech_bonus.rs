use engine_sites::Rva;
use engine_native_memory::{Memory, read};
use crate::research_inputs::Text;

pub const VTABLE_RVA: Rva = Rva(0x029f_1918);
pub const NAME: usize = 0x10;
pub const ID: usize = 0x30;
pub const USES: usize = 0x08;
pub const CLAIMS: usize = 0x0c;
pub const TECHNOLOGIES: usize = 0x38;
pub const CATEGORIES: usize = 0x50;
pub const AHEAD_REDUCTION: usize = 0x68;
pub const SPEED_BONUS: usize = 0x70;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Value {
    pub id: i32,
    pub name: Text,
    pub ahead_reduction_raw: i64,
    pub speed_bonus_raw: i64,
}

pub(crate) fn value(
    memory: &impl Memory,
    pointer: usize,
    id: i32,
) -> Result<Value, &'static str> {
    let descriptor = engine_native_memory::msvc_string::Descriptor::read(
        memory,
        pointer.checked_add(NAME).ok_or("bonus address overflow")?,
    )?;
    let name = Text::from_descriptor(memory, descriptor)?;
    let operands = read::<[u8; 16]>(
        memory,
        pointer
            .checked_add(AHEAD_REDUCTION)
            .ok_or("bonus address overflow")?,
    )
    .ok_or("research bonus values unreadable")?;
    Ok(Value {
        id,
        name,
        ahead_reduction_raw: i64::from_le_bytes(operands[..8].try_into().unwrap()),
        speed_bonus_raw: i64::from_le_bytes(operands[8..].try_into().unwrap()),
    })
}
