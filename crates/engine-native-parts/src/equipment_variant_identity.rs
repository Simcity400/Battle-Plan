use engine_native_memory::{Memory, read};
use engine_sites::Rva;
use engine_executable::native_values::equipment_variant::REFERENCE_TYPE;

pub const VTABLE_RVA: Rva = Rva(0x029c_e250);
pub fn identity(memory: &impl Memory, base: usize, pointer: usize) -> Result<u64, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("variant module address overflow")?;
    let bytes = read::<[u8; 16]>(memory, pointer).ok_or("variant identity unreadable")?;
    identity_header(expected, bytes)
}

pub fn identity_from_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    identity_header(expected, bytes)
}

pub fn identity_header(expected: usize, bytes: [u8; 16]) -> Result<u64, &'static str> {
    let vtable = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    let handle = u64::from_le_bytes(bytes[8..].try_into().unwrap());
    if vtable != expected as u64 || handle as u32 != REFERENCE_TYPE {
        return Err("variant vtable or reference type mismatch");
    }
    Ok(handle)
}
