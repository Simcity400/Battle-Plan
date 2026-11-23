use engine_native_memory::Memory;
use crate::cnaval_combatant::field;
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x02ab_2d48);
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub speed: i64,
    pub strength: i64,
    pub organisation: i64,
    pub maximum_strength: i64,
    pub maximum_organisation: i64,
}
pub fn read(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    handle: u64,
    wants_speed: bool,
) -> Result<Row, &'static str> {
    if field::<u64>(memory, pointer, 0)?
        != engine_sites::at(base, VTABLE_RVA).ok_or("convoy ship module overflow")? as u64
        || field::<u64>(memory, pointer, 8)? != handle
    {
        return Err("convoy ship identity mismatch");
    }
    let variant = field::<u64>(memory, pointer, 0x18)? as usize;
    if field::<u64>(memory, variant, 0)?
        != engine_sites::at(base, engine_native_parts::equipment_variant_identity::VTABLE_RVA)
            .ok_or("convoy variant module overflow")? as u64
    {
        return Err("convoy ship variant identity mismatch");
    }
    let row = Row {
        speed: if wants_speed {
            field(memory, variant, 0x208)?
        } else {
            0
        },
        strength: field(memory, pointer, 0x20)?,
        organisation: field(memory, pointer, 0x28)?,
        maximum_strength: field(memory, variant, 0x2e8)?,
        maximum_organisation: field(memory, variant, 0x2e0)?,
    };
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn convoy_speed_comes_from_stored_variant_at_208() {
        let mut bytes = [0_u8; 0x500];
        let put = |bytes: &mut [u8], at: usize, value: u64| {
            bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        };
        put(&mut bytes, 0x80, VTABLE_RVA.0 as u64);
        put(&mut bytes, 0x88, 0x9876);
        put(&mut bytes, 0x98, 0x200);
        put(
            &mut bytes,
            0x200,
            engine_native_parts::equipment_variant_identity::VTABLE_RVA.0 as u64,
        );
        put(&mut bytes, 0x408, 321_000);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(read(&memory, 0, 0x80, 0x9876, true).unwrap().speed, 321_000);
        assert_eq!(read(&memory, 0, 0x80, 0x9876, false).unwrap().speed, 0);
        put(&mut bytes, 0x200, 0);
        assert_eq!(
            read(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x80,
                0x9876,
                true
            ),
            Err("convoy ship variant identity mismatch")
        );
    }
}
