use engine_sites::Rva;
use engine_native_memory::Memory;
use engine_native::cnaval_combatant::field;
pub const VTABLE_RVA: Rva = Rva(0x02a9_a168);
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub present: u8,
    pub is_convoy: u8,
    pub target_group_member_count: i32,
}
pub(crate) fn read(memory: &impl Memory, base: usize, pointer: usize) -> Result<Row, &'static str> {
    let present = field::<u8>(memory, pointer, 0x10)?;
    if present == 0 {
        return Ok(Row::default());
    }
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA)
            .ok_or("last-target module overflow")? as u64
    {
        return Err("last-target identity mismatch");
    }
    let row = Row {
        present,
        is_convoy: field(memory, pointer, 8)?,
        target_group_member_count: field(memory, pointer, 0xc)?,
    };
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_optional_does_not_read_inactive_storage_and_signed_count_survives() {
        let mut bytes = [0u8; 24];
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        assert_eq!(read(&census, 0, 0x1000), Ok(Row::default()));
        assert_eq!((census.calls(), census.bytes()), (1, 1));
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8] = 0xfe;
        bytes[0xc..0x10].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[0x10] = 1;
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let census = engine_native_memory::read_census::ReadCensus::new(&memory);
        assert_eq!(
            read(&census, 0, 0x1000),
            Ok(Row {
                present: 1,
                is_convoy: 0xfe,
                target_group_member_count: i32::MIN
            })
        );
        assert_eq!((census.calls(), census.bytes()), (4, 14));
    }
}
