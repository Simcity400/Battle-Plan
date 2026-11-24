
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0279_63d0);
pub const PERSISTENT_VTABLE_RVA: Rva = Rva(0x0279_6400);
pub const GREGORIAN_BASE: usize = 0;
pub const DATE_BASE: usize = 0x08;
pub const PERSISTENT_BASE: usize = 0x10;

pub fn read_raw(
    memory: &impl crate::Memory,
    base: usize,
    address: usize,
) -> Result<i32, &'static str> {
    let primary = engine_sites::at(base, VTABLE_RVA)
        .ok_or("game date vtable address overflow")? as u64;
    let persistent = engine_sites::at(base, PERSISTENT_VTABLE_RVA)
        .ok_or("game date vtable address overflow")? as u64;
    if crate::read::<u64>(memory, address) != Some(primary)
        || address
            .checked_add(PERSISTENT_BASE)
            .and_then(|at| crate::read::<u64>(memory, at))
            != Some(persistent)
    {
        return Err("game date vtable mismatch or unreadable");
    }
    address
        .checked_add(DATE_BASE)
        .and_then(|at| crate::read(memory, at))
        .ok_or("game date raw hour unreadable")
}
pub const NULL_RAW_HOUR: i32 = 43_808_760;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn an_unlocated_vtable_site_is_withheld_not_an_address_overflow() {
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &[0; 0x18],
        };
        for key in [VTABLE_RVA, PERSISTENT_VTABLE_RVA] {
            assert_eq!(
                engine_sites::with_synthetic_withheld(key, || read_raw(&memory, 0, 0x1000)),
                Err("game date vtable withheld")
            );
        }
    }
}
