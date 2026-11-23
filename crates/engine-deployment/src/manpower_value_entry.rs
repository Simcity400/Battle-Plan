
pub const STRIDE: usize = 0x08;
pub const SOURCE_COUNTRY_LINK: usize = 0x00;
pub const COUNT: usize = 0x04;

use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_parts::carmy_manpower_values;

pub fn visit(
    memory: &impl Memory,
    base: usize,
    container: usize,
    capacity: usize,
    mut entry: impl FnMut(u32, u32, i32) -> Result<(), &'static str>,
) -> Result<u32, &'static str> {
    let expected = engine_sites::at(base, carmy_manpower_values::VTABLE_RVA)
        .ok_or("assigned manpower container vtable mismatch")?;
    if read::<u64>(memory, container) != Some(expected as u64) {
        return Err("assigned manpower container vtable mismatch");
    }
    let array = container
        .checked_add(carmy_manpower_values::DATA)
        .and_then(|p| Array::read(memory, p, STRIDE))
        .ok_or("assigned manpower array invalid")?;
    let copied = (array.count as usize).min(capacity) as u32;
    for ordinal in 0..copied {
        let value = array
            .element(ordinal, STRIDE)
            .and_then(|p| read::<[u8; 8]>(memory, p))
            .ok_or("assigned manpower entry unreadable")?;
        let source = u32::from_le_bytes(value[..4].try_into().unwrap());
        let amount = i32::from_le_bytes(value[COUNT..].try_into().unwrap());
        entry(ordinal, source, amount)?;
    }
    Ok(array.count - copied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn every_native_source_and_signed_amount_is_retained_with_capacity_count() {
        let mut bytes = [0; 0x40];
        bytes[..8].copy_from_slice(&(carmy_manpower_values::VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..16].copy_from_slice(&0x1020_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&2_u32.to_le_bytes());
        bytes[20..24].copy_from_slice(&2_u32.to_le_bytes());
        bytes[32..36].copy_from_slice(&0x8000_0001_u32.to_le_bytes());
        bytes[36..40].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[40..44].copy_from_slice(&0_u32.to_le_bytes());
        bytes[44..48].copy_from_slice(&i32::MAX.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut rows = Vec::new();
        assert_eq!(
            visit(&memory, 0, 0x1000, 2, |n, source, amount| {
                rows.push((n, source, amount));
                Ok(())
            }),
            Ok(0)
        );
        assert_eq!(rows, [(0, 0x8000_0001, i32::MIN), (1, 0, i32::MAX)]);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..40],
        };
        assert_eq!(visit(&memory, 0, 0x1000, 1, |_, _, _| Ok(())), Ok(1));
        assert!(visit(&memory, 0, 0x1000, 2, |_, _, _| Ok(())).is_err());
    }
}
