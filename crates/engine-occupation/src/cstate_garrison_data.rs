//! State-garrison operands retain full i64 fixed-point scalars. Assigned
//! manpower is an embedded container: its entry-array field is not a
//! pointer to a separate manpower pool.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x02a0_0d78);
/// Garrison allocation size.
pub const SIZE: usize = 0x108;
/// CStateGarrisonData child pointer vector.
pub const CHILDREN: usize = 0x30;
/// Owning state pointer.
pub const STATE: usize = 0x50;
/// Owning occupation-data pointer authenticated against containing table.
pub const OCCUPATION_DATA: usize = 0x58;
/// Serialized token-0x2eb7 byte; gameplay meaning unresolved.
pub const SERIALIZED_FLAG_CANDIDATE: usize = 0x60;
/// Embedded assigned-manpower container.
pub const ASSIGNED_MANPOWER: usize = 0x68;
/// Required manpower, signed i32.
pub const REQUIRED_MANPOWER: usize = 0x88;
/// CStateGarrisonData local equipment allocation pool.
pub const LOCAL_EQUIPMENT: usize = 0x90;
/// CStateGarrisonData required-equipment pool.
pub const REQUIRED_EQUIPMENT: usize = 0xd0;
/// Strength ratio, i64/100000.
pub const STRENGTH_RATIO: usize = 0xf0;
/// Required garrison, i64/100000.
pub const GARRISON_REQUIRED: usize = 0xf8;
/// Fixed-point denominator for both scalar qwords.
pub const FIXED_POINT_SCALE: i64 = 100_000;

use crate::{
    Memory, ccountry_occupation_data, ccountry_occupation_status::Rows, cpdx_array::Array,
    manpower_value_entry, pdx_robin_hood_table, read,
};

pub mod equipment;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub required_manpower: i32,
    pub strength_ratio: i64,
    pub garrison_required: i64,
    /// Stored byte indicating whether required strength is negative.
    pub serialized_flag: u8,
    /// Owned arena span; None refuses this state's equipment only.
    pub equipment: Option<(usize, usize)>,
    pub equipment_failure: Option<&'static str>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ManpowerRow {
    pub controller: u32,
    pub occupied: u32,
    pub state: u32,
    pub ordinal: u32,
    pub source_country: u32,
    pub amount: i32,
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    states: Array,
    parent: usize,
    entry: pdx_robin_hood_table::Entry,
) -> Result<(), &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("state garrison vtable mismatch")?;
    if read::<u64>(memory, entry.value) != Some(expected as u64) {
        return Err("state garrison vtable mismatch");
    }
    let references = entry
        .value
        .checked_add(STATE)
        .and_then(|p| read::<[u8; 16]>(memory, p))
        .ok_or("state garrison references unreadable")?;
    let state_pointer = u64::from_le_bytes(references[..8].try_into().unwrap());
    let occupation_data = u64::from_le_bytes(references[8..].try_into().unwrap());
    if occupation_data != parent as u64 {
        return Err("state garrison parent mismatch");
    }
    if ccountry_occupation_data::state_pointer(memory, base, states, entry.key)? as u64
        != state_pointer
    {
        return Err("state garrison pointer differs from its state slot");
    }
    Ok(())
}

fn scalars(
    memory: &impl Memory,
    pointer: usize,
    scope: &ccountry_occupation_data::Scope,
    state: u32,
) -> Result<Row, &'static str> {
    let mut flag = [0; 1];
    let mut manpower = [0; 4];
    let mut ratios = [0; 16];
    let addresses = [SERIALIZED_FLAG_CANDIDATE, REQUIRED_MANPOWER, STRENGTH_RATIO]
        .map(|offset| pointer.checked_add(offset));
    let [
        Some(flag_address),
        Some(manpower_address),
        Some(ratio_address),
    ] = addresses
    else {
        return Err("state garrison scalar address overflow");
    };
    if memory.copy_scattered(
        [flag_address, manpower_address, ratio_address],
        &mut [&mut flag, &mut manpower, &mut ratios],
    ) != 3
    {
        return Err("state garrison scalars unreadable");
    }
    Ok(Row {
        controller: scope.controller,
        occupied: scope.occupied,
        state,
        required_manpower: i32::from_le_bytes(manpower),
        strength_ratio: i64::from_le_bytes(ratios[..8].try_into().unwrap()),
        garrison_required: i64::from_le_bytes(ratios[8..].try_into().unwrap()),
        serialized_flag: flag[0],
        equipment: None,
        equipment_failure: None,
    })
}

pub(crate) fn walk_data(
    memory: &impl Memory,
    base: usize,
    states: Array,
    scope: &ccountry_occupation_data::Scope,
    rows: &mut Rows<'_, Row>,
    manpower: &mut Rows<'_, ManpowerRow>,
    equipment: &mut equipment::Arena<'_>,
) -> Result<(), &'static str> {
    let table = scope
        .pointer
        .checked_add(ccountry_occupation_data::GARRISON_TABLE)
        .ok_or("state garrison table address overflow")?;
    pdx_robin_hood_table::visit(memory, table, |entry| {
        authenticate(memory, base, states, scope.pointer, entry)?;
        if rows.fits()? {
            match scalars(memory, entry.value, scope, entry.key) {
                Ok(mut row) => {
                    match equipment.capture(memory, base, entry.value) {
                        Ok(Some(span)) => row.equipment = Some(span),
                        Ok(None) => {
                            row.equipment_failure = Some("garrison equipment arena truncated")
                        }
                        Err(reason) => row.equipment_failure = Some(reason),
                    }
                    rows.push(row)?;
                }
                Err(reason) => rows.fail(reason),
            }
        } else {
            rows.omit()?;
        }
        let result = entry
            .value
            .checked_add(ASSIGNED_MANPOWER)
            .ok_or("state garrison manpower address overflow")
            .and_then(|container| {
                manpower_value_entry::visit(
                    memory,
                    base,
                    container,
                    manpower.remaining(),
                    |ordinal, source_country, amount| {
                        manpower.push(ManpowerRow {
                            controller: scope.controller,
                            occupied: scope.occupied,
                            state: entry.key,
                            ordinal,
                            source_country,
                            amount,
                        })
                    },
                )
            });
        match result {
            Ok(unread) => manpower.omit_n(unread)?,
            Err(reason) => manpower.fail(reason),
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    #[test]
    fn garrison_native_widths_are_never_narrowed_or_normalized() {
        let mut bytes = [0; SIZE];
        bytes[SERIALIZED_FLAG_CANDIDATE] = 0xfe;
        bytes[REQUIRED_MANPOWER..REQUIRED_MANPOWER + 4].copy_from_slice(&i32::MIN.to_le_bytes());
        bytes[STRENGTH_RATIO..STRENGTH_RATIO + 8].copy_from_slice(&i64::MAX.to_le_bytes());
        bytes[GARRISON_REQUIRED..GARRISON_REQUIRED + 8].copy_from_slice(&i64::MIN.to_le_bytes());
        let scope = ccountry_occupation_data::Scope {
            controller: 12,
            occupied: 34,
            pointer: 0x7000,
        };
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes,
        };
        assert_eq!(
            scalars(&memory, 0x1000, &scope, 9),
            Ok(Row {
                controller: 12,
                occupied: 34,
                state: 9,
                required_manpower: i32::MIN,
                strength_ratio: i64::MAX,
                garrison_required: i64::MIN,
                serialized_flag: 0xfe,
                equipment: None,
                equipment_failure: None,
            })
        );
        assert!(
            scalars(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..GARRISON_REQUIRED + 7]
                },
                0x1000,
                &scope,
                9
            )
            .is_err()
        );
    }
}
