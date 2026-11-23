use engine_native_memory::{Memory, cpdx_array::Array, read};
use engine_native_parts::cequipment_type as ty;
use engine_native::cequipment_variant::pool as pool;
use crate::{garrison_equipment_pool, required_equipment_row};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub equipment_token: u32,
    pub archetype_token: u32,
    pub archetype_active: u8,
    pub required_raw: i64,
    pub required_lookup_raw: i64,
    pub allocated_raw: i64,
    pub weight_raw: i64,
}

pub struct Arena<'a> {
    pub rows: engine_native_memory::walk::Buffer<'a, Row>,
    pub len: usize,
    pub required: usize,
}

impl<'a> Arena<'a> {
    pub fn new_buffer(rows: engine_native_memory::walk::Buffer<'a, Row>) -> Self {
        Self {
            rows,
            len: 0,
            required: 0,
        }
    }

    pub fn capture(
        &mut self,
        memory: &impl Memory,
        base: usize,
        garrison: usize,
    ) -> Result<Option<(usize, usize)>, &'static str> {
        let required_pool = garrison
            .checked_add(super::REQUIRED_EQUIPMENT)
            .ok_or("garrison required equipment address overflow")?;
        let requirements = required_entries(memory, base, required_pool)?;
        let count = requirements.count as usize;
        self.required = self
            .required
            .checked_add(count)
            .ok_or("garrison equipment demand overflow")?;
        self.rows.ensure(self.required)?;
        if count > self.rows.len() - self.len {
            return Ok(None);
        }
        if count == 0 {
            return Ok(Some((self.len, 0)));
        }
        let allocated_pool = garrison
            .checked_add(super::LOCAL_EQUIPMENT)
            .ok_or("garrison allocated equipment address overflow")?;
        let allocations = pool::allocation_entries(memory, base, allocated_pool)?;
        let start = self.len;
        for ordinal in 0..requirements.count {
            let entry = pair(memory, requirements, ordinal)?;
            let equipment = entry.0;
            authenticate(memory, base, equipment)?;
            let archetype = field::<u64>(memory, equipment, ty::ARCHETYPE)? as usize;
            authenticate(memory, base, archetype)?;
            let active = field::<u8>(memory, archetype, ty::ACTIVE)?;
            let target = field::<i32>(memory, archetype, ty::ARCHETYPE_ID)?;
            let mut allocated = 0;
            for index in 0..allocations.count {
                let address = allocations
                    .element(index, engine_native_parts::allocation_equipment_row::STRIDE)
                    .ok_or("garrison allocation entry address overflow")?;
                let row = engine_native_parts::allocation_equipment_row::read(memory, base, address, index)?;
                if target <= row.archetype_id {
                    if target == row.archetype_id {
                        allocated = row.amount_raw;
                    }
                    break;
                }
            }
            let mut required = 0;
            if active != 0 {
                for index in 0..requirements.count {
                    let candidate = pair(memory, requirements, index)?;
                    if candidate.0 == archetype {
                        required = candidate.1;
                        break;
                    }
                }
            }
            self.rows[start + ordinal as usize] = Row {
                equipment_token: field(memory, equipment, ty::TOKEN)?,
                archetype_token: field(memory, archetype, ty::TOKEN)?,
                archetype_active: active,
                required_raw: entry.1,
                required_lookup_raw: required,
                allocated_raw: allocated,
                weight_raw: field(memory, equipment, ty::BUILD_COST_IC)?,
            };
        }
        self.len += count;
        Ok(Some((start, count)))
    }
}

pub fn required_entries(
    memory: &impl Memory,
    base: usize,
    pool: usize,
) -> Result<Array, &'static str> {
    let expected = engine_sites::at(base, garrison_equipment_pool::VTABLE_RVA)
        .ok_or("garrison required equipment pool vtable mismatch")?;
    if read::<u64>(memory, pool) != Some(expected as u64) {
        return Err("garrison required equipment pool vtable mismatch");
    }
    let header = pool
        .checked_add(garrison_equipment_pool::DATA)
        .ok_or("garrison required equipment header address overflow")?;
    Array::read(memory, header, required_equipment_row::STRIDE)
        .ok_or("garrison required equipment array invalid or unreadable")
}

fn pair(memory: &impl Memory, array: Array, ordinal: u32) -> Result<(usize, i64), &'static str> {
    let address = array
        .element(ordinal, required_equipment_row::STRIDE)
        .ok_or("garrison requirement entry address overflow")?;
    let bytes = read::<[u8; 16]>(memory, address).ok_or("garrison requirement entry unreadable")?;
    Ok((
        u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize,
        i64::from_le_bytes(bytes[8..].try_into().unwrap()),
    ))
}

fn authenticate(memory: &impl Memory, base: usize, pointer: usize) -> Result<(), &'static str> {
    let expected =
        engine_sites::at(base, ty::VTABLE_RVA).ok_or("garrison equipment type vtable mismatch")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("garrison equipment type vtable mismatch");
    }
    Ok(())
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .and_then(|address| read(memory, address))
        .ok_or("garrison equipment operand unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;
    const START: usize = 0x1000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at - START..at - START + value.len()].copy_from_slice(value);
    }
    fn header(bytes: &mut [u8], at: usize, pointer: usize, count: usize) {
        put(bytes, at, &(pointer as u64).to_le_bytes());
        put(bytes, at + 8, &(count as i32).to_le_bytes());
        put(bytes, at + 12, &(count as i32).to_le_bytes());
    }
    fn fixture(count: usize) -> (Vec<u8>, usize, usize) {
        let required = 0x2000;
        let allocated = required + count * 16;
        let equipment = allocated + count * 24 + 0x100;
        let mut bytes = vec![0; equipment + 0x600 - START];
        put(
            &mut bytes,
            START + super::super::REQUIRED_EQUIPMENT,
            &(garrison_equipment_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        header(
            &mut bytes,
            START + super::super::REQUIRED_EQUIPMENT + 8,
            required,
            count,
        );
        put(
            &mut bytes,
            START + super::super::LOCAL_EQUIPMENT,
            &(pool::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        header(
            &mut bytes,
            START + super::super::LOCAL_EQUIPMENT + 8,
            allocated,
            count,
        );
        put(
            &mut bytes,
            equipment,
            &(ty::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, equipment + ty::TOKEN, &700_u32.to_le_bytes());
        put(&mut bytes, equipment + ty::ACTIVE, &[2]);
        put(
            &mut bytes,
            equipment + ty::ARCHETYPE,
            &(equipment as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            equipment + ty::ARCHETYPE_ID,
            &(-2_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            equipment + ty::BUILD_COST_IC,
            &200_003_i64.to_le_bytes(),
        );
        for ordinal in 0..count {
            put(
                &mut bytes,
                required + ordinal * 16,
                &(equipment as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                required + ordinal * 16 + 8,
                &(100_000 + ordinal as i64).to_le_bytes(),
            );
            put(
                &mut bytes,
                allocated + ordinal * 24,
                &(equipment as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                allocated + ordinal * 24 + 16,
                &(50_000 + ordinal as i64).to_le_bytes(),
            );
        }
        (bytes, required, equipment)
    }

    #[test]
    fn initial_equipment_arena_keeps_all_mod_sized_requirements() {
        let (bytes, _, _) = fixture(5000);
        let memory = SliceMemory {
            base: START,
            bytes: &bytes,
        };
        let mut rows = Vec::new();
        let mut arena = Arena::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut rows));
        assert_eq!(arena.capture(&memory, 0, START), Ok(Some((0, 5000))));
        assert_eq!(arena.len, 5000);
        assert_eq!(arena.required, 5000);
        assert_eq!(arena.rows[4999].required_raw, 104999);
        assert_eq!(Arena::new_buffer(engine_native_memory::walk::Buffer::Count).capture(&memory, 0, START), Ok(None));
    }

    #[test]
    fn empty_inactive_and_partial_reads_never_fabricate_operands() {
        let (mut bytes, _, equipment) = fixture(1);
        put(&mut bytes, equipment + ty::ACTIVE, &[0]);
        let mut output = vec![Row::default(); 1];
        let mut arena = Arena::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut output));
        assert_eq!(
            arena.capture(
                &SliceMemory {
                    base: START,
                    bytes: &bytes
                },
                0,
                START
            ),
            Ok(Some((0, 1)))
        );
        assert_eq!(arena.rows[0].required_lookup_raw, 0);
        assert_eq!(arena.rows[0].required_raw, 100_000);
        let mut arena = Arena::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut output));
        assert!(
            arena
                .capture(
                    &SliceMemory {
                        base: START,
                        bytes: &bytes[..equipment + ty::BUILD_COST_IC + 7 - START]
                    },
                    0,
                    START
                )
                .is_err()
        );
        assert_eq!(arena.len, 0);
        let (bytes, _, _) = fixture(0);
        assert_eq!(
            Arena::new_buffer(engine_native_memory::walk::Buffer::Count).capture(
                &SliceMemory {
                    base: START,
                    bytes: &bytes[..super::super::REQUIRED_EQUIPMENT + 24]
                },
                0,
                START
            ),
            Ok(Some((0, 0)))
        );
    }

    #[test]
    fn required_lookup_uses_pointer_identity_and_the_entrys_own_stat_weight() {
        let (mut bytes, required, equipment) = fixture(2);
        let clone = equipment + 0x600;
        let archetype = clone + 0x600;
        bytes.resize(archetype + 0x600 - START, 0);
        for (pointer, token) in [(clone, 701_u32), (archetype, 702)] {
            put(
                &mut bytes,
                pointer,
                &(ty::VTABLE_RVA.0 as u64).to_le_bytes(),
            );
            put(&mut bytes, pointer + ty::TOKEN, &token.to_le_bytes());
            put(&mut bytes, pointer + ty::ACTIVE, &[1]);
            put(
                &mut bytes,
                pointer + ty::ARCHETYPE_ID,
                &(-2_i32).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            clone + ty::ARCHETYPE,
            &(archetype as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            clone + ty::BUILD_COST_IC,
            &300_007_i64.to_le_bytes(),
        );
        put(&mut bytes, required + 16, &(clone as u64).to_le_bytes());
        let mut output = vec![Row::default(); 2];
        let mut arena = Arena::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut output));
        assert_eq!(
            arena.capture(
                &SliceMemory {
                    base: START,
                    bytes: &bytes
                },
                0,
                START
            ),
            Ok(Some((0, 2)))
        );
        assert_eq!(arena.rows[1].required_lookup_raw, 0);
        assert_eq!(arena.rows[1].allocated_raw, 50_000);
        assert_eq!(arena.rows[1].weight_raw, 300_007);
        put(
            &mut bytes,
            clone + ty::ARCHETYPE,
            &(equipment as u64).to_le_bytes(),
        );
        let mut arena = Arena::new_buffer(engine_native_memory::walk::Buffer::Growing(&mut output));
        assert_eq!(
            arena.capture(
                &SliceMemory {
                    base: START,
                    bytes: &bytes
                },
                0,
                START
            ),
            Ok(Some((0, 2)))
        );
        assert_eq!(arena.rows[1].required_lookup_raw, 100_000);
        assert_eq!(arena.rows[1].required_raw, 100_001);
    }
}
