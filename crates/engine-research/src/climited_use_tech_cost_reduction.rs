//! Limited-use technology cost reduction. Serialization writes its name,
//! id and signed fixed5 reduction. Selection adds ids to the same applied-id
//! list as speed/ahead bonuses; both kinds share the status id counter.
use crate::{
    Memory, climited_use_tech_bonus::Value, msvc_string::Descriptor, read, research_inputs::Text,
};

pub const VTABLE_RVA: usize = 0x0297_50f0;
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
    growing: bool,
) -> Result<(Value, i64), &'static str> {
    let descriptor = Descriptor::read(
        memory,
        pointer
            .checked_add(NAME)
            .ok_or("cost bonus address overflow")?,
    )?;
    let name = Text::from_descriptor(memory, descriptor, growing)?;
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
