
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::Outcome};
use engine_native::{cgame_state, cstate};
use engine_native_parts::cstate_template;
use engine_sites::Rva;

pub const STRIDE: usize = 0x10;
pub const AMOUNT: usize = 0x00;
#[cfg(test)]
pub const NO_RESOURCE_ID: u32 = u32::MAX;
pub const FIXED_POINT_SCALE: i64 = 100_000;
pub const CONSTRUCTOR_RVA: Rva = Rva(0x00c2_0880);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub state: u32,
    pub resource: u32,
    pub ordinal: u32,
    pub amount: i64,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let states = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|p| Array::read(memory, p, 8))
            .ok_or("resource state array unreadable or invalid")?;
        if states.count < 2 {
            return Err("resource state array has no state slots");
        }
        let state_vtable =
            engine_sites::at(base, cstate::VTABLE_RVA).ok_or("module address overflow")? as u64;
        let template_vtable = engine_sites::at(base, cstate_template::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for state in 1..states.count {
            let pointer = states
                .element(state, 8)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .ok_or("resource state pointer unreadable")?;
            if pointer == 0 {
                continue;
            }
            if read::<u64>(memory, pointer) != Some(state_vtable) {
                return Err("resource state vtable mismatch");
            }
            if pointer
                .checked_add(cstate::ID)
                .and_then(|p| read::<u32>(memory, p))
                != Some(state)
            {
                return Err("resource state id differs from slot");
            }
            let template = pointer
                .checked_add(cstate::TEMPLATE)
                .and_then(|p| read::<u64>(memory, p))
                .and_then(|p| usize::try_from(p).ok())
                .filter(|&p| p != 0)
                .ok_or("resource state template unreadable or null")?;
            if read::<u64>(memory, template) != Some(template_vtable) {
                return Err("resource state template vtable mismatch");
            }
            if template
                .checked_add(cstate_template::ID)
                .and_then(|p| read::<u32>(memory, p))
                != Some(state)
            {
                return Err("resource state template id mismatch");
            }
            let resources = template
                .checked_add(cstate_template::RESOURCES)
                .and_then(|p| Array::read(memory, p, STRIDE))
                .ok_or("state resource vector unreadable or invalid")?;
            output.ensure(
                len.checked_add(resources.count as usize)
                    .ok_or("state resource row count overflow")?,
            )?;
            let state_start = len;
            for ordinal in 0..resources.count {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("state resource omitted row count overflow")?;
                    continue;
                }
                let entry = resources
                    .element(ordinal, STRIDE)
                    .ok_or("state resource address overflow")?;
                let bytes =
                    read::<[u8; 12]>(memory, entry).ok_or("state resource entry unreadable")?;
                let amount = i64::from_le_bytes(bytes[..8].try_into().unwrap());
                let resource = u32::from_le_bytes(bytes[8..].try_into().unwrap());
                if output[state_start..len]
                    .iter()
                    .any(|row| row.resource == resource)
                {
                    return Err("duplicate resource id in state vector");
                }
                output[len] = Row {
                    state,
                    resource,
                    ordinal,
                    amount,
                };
                len += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    fn put<T: AsRef<[u8]>>(bytes: &mut [u8], at: usize, value: T) {
        bytes[at..at + value.as_ref().len()].copy_from_slice(value.as_ref());
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0xb00];
        put(&mut bytes, cgame_state::STATES, 0x1300_u64.to_le_bytes());
        put(&mut bytes, cgame_state::STATES + 8, 2_i32.to_le_bytes());
        put(&mut bytes, cgame_state::STATES + 12, 2_i32.to_le_bytes());
        put(&mut bytes, 0x308, 0x1400_u64.to_le_bytes());
        put(
            &mut bytes,
            0x400,
            (0x1000 + cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x400 + cstate::ID, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x400 + cstate::TEMPLATE,
            0x1800_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x800,
            (0x1000 + cstate_template::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x800 + cstate_template::ID, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCES,
            0x1a00_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCE_CAPACITY,
            3_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCE_COUNT,
            2_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xa00,
            (i64::from(i32::MAX) + 987_654).to_le_bytes(),
        );
        put(&mut bytes, 0xa08, 55_u32.to_le_bytes());
        put(&mut bytes, 0xa10, (-5_000_000_001_i64).to_le_bytes());
        put(&mut bytes, 0xa18, NO_RESOURCE_ID.to_le_bytes());
        put(&mut bytes, 0xa28, 55_u32.to_le_bytes());
        bytes
    }

    #[test]
    fn native_width_order_sentinel_and_live_count_are_preserved() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        let mut output = vec![Row::default(); 3];
        assert_eq!(
            walk(&memory, 0x1000, 0x1000, &mut output),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            output[0],
            Row {
                state: 1,
                resource: 55,
                ordinal: 0,
                amount: i64::from(i32::MAX) + 987_654
            }
        );
        assert_eq!(
            output[1],
            Row {
                state: 1,
                resource: NO_RESOURCE_ID,
                ordinal: 1,
                amount: -5_000_000_001
            }
        );
    }

    #[test]
    fn allocated_empty_is_empty_and_invalid_membership_is_not_a_prefix() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCE_COUNT,
            0_i32.to_le_bytes(),
        );
        let mut output = vec![Row::default(); 3];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                0x1000,
                &mut output
            ),
            Outcome::Rows { len: 0 }
        );
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCE_COUNT,
            2_i32.to_le_bytes(),
        );
        put(&mut bytes, 0xa18, 55_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                0x1000,
                &mut output
            ),
            Outcome::Unavailable {
                reason: "duplicate resource id in state vector"
            }
        );
        put(&mut bytes, 0xa18, 56_u32.to_le_bytes());
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..0xa18]
                },
                0x1000,
                0x1000,
                &mut output
            ),
            Outcome::Unavailable { .. }
        ));
        put(
            &mut bytes,
            0x800 + cstate_template::RESOURCE_COUNT,
            4_i32.to_le_bytes(),
        );
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                0x1000,
                &mut output
            ),
            Outcome::Unavailable { .. }
        ));
    }
}
