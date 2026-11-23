
use engine_native_memory::{Memory, msvc_string, read};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x0285_3e78);
pub const ID: usize = 0x28;
pub const VALUE_COUNT: usize = 0x1c4;

pub fn value_count(memory: &impl Memory, definition: usize) -> Result<u32, &'static str> {
    let count = definition
        .checked_add(VALUE_COUNT)
        .and_then(|p| read::<i32>(memory, p))
        .filter(|count| *count >= 0)
        .ok_or("dynamic modifier definition value count unreadable or invalid")?;
    Ok(count as u32)
}

pub fn token(
    memory: &impl Memory,
    base: usize,
    definition: usize,
) -> Result<(usize, usize), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    if read::<u64>(memory, definition) != Some(expected as u64) {
        return Err("dynamic modifier definition vtable mismatch");
    }
    literal(
        memory,
        definition
            .checked_add(ID)
            .ok_or("dynamic modifier token address overflow")?,
    )
}

pub fn literal(
    memory: &impl Memory,
    address: usize,
) -> Result<(usize, usize), &'static str> {
    let bytes = read::<[u8; 32]>(memory, address).ok_or("definition token header unreadable")?;
    let len = u64::from_le_bytes(
        bytes[msvc_string::LENGTH..msvc_string::CAPACITY]
            .try_into()
            .unwrap(),
    );
    let capacity = u64::from_le_bytes(bytes[msvc_string::CAPACITY..].try_into().unwrap());
    if len == 0 || len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("definition token header invalid");
    }
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        address
    } else {
        usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .ok()
            .filter(|p| *p != 0)
            .ok_or("definition token pointer invalid")?
    };
    let len = usize::try_from(len).map_err(|_| "definition token length overflow")?;
    let terminator = source
        .checked_add(len)
        .ok_or("definition token end overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("definition token terminator unreadable or invalid");
    }
    Ok((source, len))
}
