
use engine_sites::Rva;
use engine_native_memory::{Memory, read};

pub const VTABLE_RVA: Rva = Rva(0x029b_9b20);
pub const TOKEN: usize = 8;
pub const WAR_NAME: usize = 0x2a8;

pub fn token(memory: &impl Memory, base: usize, pointer: usize) -> Result<u32, &'static str> {
    if pointer == 0
        || read::<u64>(memory, pointer) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
    {
        return Err("war goal type vtable mismatch");
    }
    pointer
        .checked_add(TOKEN)
        .and_then(|p| read(memory, p))
        .ok_or("war goal type token unreadable")
}
