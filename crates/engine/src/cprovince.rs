use crate::{
    Memory, cgame_state, cstate,
    pdx::Array,
    read,
    walk::{self, Outcome},
};

/// CProvince vtable, ported from old build_compact_map_control_topology.
pub const VTABLE_RVA: usize = 0x0295_af68;
/// CProvince id, checked against its array slot by the old topology walker.
pub const ID: usize = 0xa4;
/// CProvince CState pointer; the old topology walker follows this for ownership.
pub const STATE: usize = 0xc0;
/// CProvince raw controller tag id, u32 in the old topology walk; zero is preserved.
pub const CONTROLLER: usize = 0x188;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u32,
    pub state: u32,
    pub owner: u32,
    pub controller: u32,
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    let Some(array) = game_state
        .checked_add(cgame_state::PROVINCES)
        .and_then(|p| Array::read(memory, p, 8))
    else {
        return Outcome::Unavailable {
            reason: "province array unreadable or invalid",
        };
    };
    if array.count < 2 {
        return Outcome::Unavailable {
            reason: "province table has no province slots",
        };
    }
    let outcome = walk::pointers(memory, array, output, |index, pointer| {
        let at = |offset| {
            pointer
                .checked_add(offset)
                .ok_or("province address overflow")
        };
        let vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")?;
        if read::<u64>(memory, pointer) != Some(vtable as u64) {
            return Err("province vtable mismatch");
        }
        let id = read::<u32>(memory, at(ID)?).ok_or("province id unreadable")?;
        if id != index {
            return Err("province id differs from slot");
        }
        let state = read::<u64>(memory, at(STATE)?).ok_or("province state unreadable")?;
        let controller =
            read::<u32>(memory, at(CONTROLLER)?).ok_or("province controller unreadable")?;
        let (state, owner) = if state == 0 {
            (0, 0)
        } else {
            let state = usize::try_from(state).map_err(|_| "state pointer overflow")?;
            let vtable = base
                .checked_add(cstate::VTABLE_RVA)
                .ok_or("module address overflow")?;
            if read::<u64>(memory, state) != Some(vtable as u64) {
                return Err("state vtable mismatch");
            }
            let id = state
                .checked_add(cstate::ID)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("state id unreadable")?;
            if id == 0 {
                return Err("non-null state has zero id");
            }
            let owner = state
                .checked_add(cstate::OWNER)
                .and_then(|p| read::<u32>(memory, p))
                .ok_or("state owner unreadable")?;
            (id, owner)
        };
        Ok(Row {
            id,
            state,
            owner,
            controller,
        })
    });
    if outcome == (Outcome::Rows { len: 0 }) {
        Outcome::Unavailable {
            reason: "province table has no readable provinces",
        }
    } else {
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn ownership_comes_from_the_state_and_raw_controller_zero_is_preserved() {
        let mut bytes = vec![0_u8; 0x800];
        let put = |bytes: &mut [u8], offset: usize, data: &[u8]| {
            bytes[offset..offset + data.len()].copy_from_slice(data)
        };
        put(&mut bytes, 0x2b0, &0x1300_u64.to_le_bytes());
        put(&mut bytes, 0x2b8, &2_u32.to_le_bytes());
        put(&mut bytes, 0x2bc, &2_u32.to_le_bytes());
        put(&mut bytes, 0x308, &0x1400_u64.to_le_bytes());
        put(&mut bytes, 0x400, &(VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, 0x4a4, &1_u32.to_le_bytes());
        put(&mut bytes, 0x4c0, &0x1600_u64.to_le_bytes());
        put(
            &mut bytes,
            0x600,
            &(cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x658, &7_u32.to_le_bytes());
        put(&mut bytes, 0x6c8, &42_u32.to_le_bytes());
        let mut rows = [Row::default(); 2];
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Rows { len: 1 }
        );
        assert_eq!(
            rows[0],
            Row {
                id: 1,
                state: 7,
                owner: 42,
                controller: 0
            }
        );
        bytes[0x400] ^= 1;
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "province vtable mismatch"
            }
        );
        bytes[0x400] ^= 1;
        bytes[0x308..0x310].fill(0);
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "province table has no readable provinces"
            }
        );
        bytes[0x2bc..0x2c0].copy_from_slice(&1_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            walk(&memory, 0, 0x1000, &mut rows),
            Outcome::Unavailable {
                reason: "province table has no province slots"
            }
        );
    }
}
