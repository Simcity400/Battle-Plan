
use engine_sites::Rva;
use engine_native_memory::{Memory, read};

pub const VTABLE_RVA: Rva = Rva(0x0286_85f0);
pub const DEFINITION: usize = 0x08;
pub const DAYS: usize = 0x10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Record {
    pub definition: usize,
    pub days: i32,
}

pub(crate) fn record(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<Record, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
    let bytes = read::<[u8; 20]>(memory, pointer).ok_or("decision cooldown unreadable")?;
    if u64::from_le_bytes(bytes[..8].try_into().unwrap()) != expected as u64 {
        return Err("decision cooldown vtable mismatch");
    }
    let definition = usize::try_from(u64::from_le_bytes(bytes[8..16].try_into().unwrap()))
        .map_err(|_| "decision definition pointer overflow")?;
    Ok(Record {
        definition,
        days: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
    })
}
