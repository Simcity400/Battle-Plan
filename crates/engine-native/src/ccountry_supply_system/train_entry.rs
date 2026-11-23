use engine_native_memory::{Memory, read};
use crate::cequipment_variant;
use engine_native_parts::creference_registry;
pub const STRIDE: usize = 0x18;
pub const VARIANT: usize = 0;
pub const COUNT: usize = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub variant: u64,
    pub count: i32,
}

pub(crate) fn decode(memory: &impl Memory, base: usize, bytes: &[u8]) -> Result<Row, &'static str> {
    let variant = u64::from_le_bytes(bytes[VARIANT..VARIANT + 8].try_into().unwrap());
    let count = i32::from_le_bytes(bytes[COUNT..COUNT + 4].try_into().unwrap());
    if variant != 0 {
        if variant as u32 != cequipment_variant::REFERENCE_TYPE {
            return Err("allocated train reference is not a variant");
        }
        let pointer = creference_registry::resolve(memory, base, variant)?
            .ok_or("allocated train variant unresolved")?;
        if engine_native_parts::equipment_variant_identity::identity(memory, base, pointer)? != variant {
            return Err("allocated train variant identity mismatch");
        }
        if read::<u64>(memory, pointer + cequipment_variant::DEFINITION).is_none_or(|p| p == 0) {
            return Err("allocated train variant definition unreadable or null");
        }
    }
    Ok(Row { variant, count })
}
