
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x029f_f3b0);
pub const STRIDE: usize = 0x58;
pub const PROTOTYPE_REWARD_HANDLE: usize = 0x08;
pub const OPTION_HANDLE: usize = 0x0c;
pub const CONTEXT: usize = 0x10;
pub const DATE: usize = 0x48;

use engine_native_memory::{Memory, read, walk::Outcome};
use crate::{nproject_cproject, nproject_sproject_context};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub owner: u32,
    pub project: u32,
    pub ordinal: u32,
    pub prototype_reward_handle: u32,
    pub option_handle: u32,
    pub date: i32,
    pub context: nproject_sproject_context::Row,
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut Vec<Row>) -> Outcome {
    nproject_cproject::walk_with_history_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Growing(output),
    )
    .1
}

pub fn read_row(
    memory: &impl Memory,
    base: usize,
    address: usize,
    owner: u32,
    project: u32,
    ordinal: u32,
) -> Result<Row, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("project history vtable overflow")?;
    if read::<u64>(memory, address) != Some(expected as u64) {
        return Err("project history vtable unreadable or mismatched");
    }
    let field = |offset| {
        address
            .checked_add(offset)
            .ok_or("project history address overflow")
    };
    let mut handles = [0; 8];
    let mut date = [0; 4];
    if memory.copy_scattered(
        [field(PROTOTYPE_REWARD_HANDLE)?, field(DATE)?],
        &mut [&mut handles, &mut date],
    ) != 2
    {
        return Err("project history fields unreadable");
    }
    Ok(Row {
        owner,
        project,
        ordinal,
        prototype_reward_handle: u32::from_le_bytes(handles[..4].try_into().unwrap()),
        option_handle: u32::from_le_bytes(handles[4..].try_into().unwrap()),
        date: i32::from_le_bytes(date),
        context: nproject_sproject_context::read_row(memory, base, field(CONTEXT)?)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn history_keeps_owner_project_ordinal_and_checks_both_vtables() {
        let mut bytes = [0; STRIDE];
        bytes[..8].copy_from_slice(&(0x1000_u64 + VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[8..16].fill(0xff);
        bytes[CONTEXT..CONTEXT + 8].copy_from_slice(
            &(0x1000_u64 + nproject_sproject_context::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        bytes[DATE..DATE + 4].copy_from_slice(&i32::MIN.to_le_bytes());
        let memory = SliceMemory {
            base: 0x2000,
            bytes: &bytes,
        };
        let row = read_row(&memory, 0x1000, 0x2000, 3, 4, 5).unwrap();
        assert_eq!((row.owner, row.project, row.ordinal), (3, 4, 5));
        assert_eq!(
            (row.prototype_reward_handle, row.option_handle),
            (u32::MAX, u32::MAX)
        );
        assert_eq!(row.date, i32::MIN);
        bytes[CONTEXT] ^= 1;
        assert_eq!(
            read_row(
                &SliceMemory {
                    base: 0x2000,
                    bytes: &bytes
                },
                0x1000,
                0x2000,
                3,
                4,
                5
            ),
            Err("project context vtable unreadable or mismatched")
        );
    }
}
