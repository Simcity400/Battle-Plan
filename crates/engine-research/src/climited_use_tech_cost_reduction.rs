use engine_sites::Rva;
use engine_native_memory::{Memory, msvc_string::Descriptor, read};
use crate::{climited_use_tech_bonus::Value, research_inputs::Text};

pub const VTABLE_RVA: Rva = Rva(0x029f_1968);
pub const NAME: usize = 0x30;
pub const ID: usize = 0x50;
pub const REDUCTION: usize = 0x08;
pub const TECHNOLOGIES: usize = 0x10;
pub const USES: usize = 0x28;
pub const CATEGORIES: usize = 0x58;

pub(crate) fn value(
    memory: &impl Memory,
    pointer: usize,
    id: i32,
) -> Result<(Value, i64), &'static str> {
    let descriptor = Descriptor::read(
        memory,
        pointer
            .checked_add(NAME)
            .ok_or("cost bonus address overflow")?,
    )?;
    let name = Text::from_descriptor(memory, descriptor)?;
    let reduction = read::<i64>(
        memory,
        pointer
            .checked_add(REDUCTION)
            .ok_or("cost bonus address overflow")?,
    )
    .ok_or("cost reduction unreadable")?;
    Ok((
        Value {
            id,
            name,
            ..Default::default()
        },
        reduction,
    ))
}
