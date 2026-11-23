
use engine_sites::Rva;
use engine_native_memory::{Memory, read};

pub const VTABLE_RVA: Rva = Rva(0x029b_cc48);
pub const PREPARATION_TIME: usize = 0x10;
pub const TOKEN_ID: usize = 0x08;

pub fn definition(memory: &impl Memory, base: usize, pointer: usize) -> Result<u32, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("operation module address overflow")?;
    let head =
        read::<[u8; 12]>(memory, pointer).ok_or("operation definition identity unreadable")?;
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != expected as u64 {
        return Err("operation definition vtable mismatch");
    }
    Ok(u32::from_le_bytes(head[TOKEN_ID..].try_into().unwrap()))
}
