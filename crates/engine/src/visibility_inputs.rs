//! Stored simulation inputs of the map visibility laws that the game computes
//! per viewer on demand: `GetUnitIntelLevel@1416068C0` and the province fog
//! rebuild `radar_group::fog_consumer@141606390`. No viewpoint, fog switch or
//! presentation buffer is read, and no engine function is called.

use crate::{
    Memory, ccountry, cgame_state,
    cpdx_array::Array,
    cstate, dlc_feature_state,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    modifier_accumulator, read,
    walk::Outcome,
};

/// `GetUnitIntelLevel@1416068C0` scales a foreign unit's byte by the viewer's
/// own modifier 0x24B (`modifier_accumulator::value@140551820` on `CCountry+0x5B8`).
pub const UNIT_MODIFIER: u32 = 0x24b;
/// `fog_consumer@141606390` asks `CState::GetModifierForCountry@1409CC140`
/// for key 0x4B on each owned state another country controls.
pub const NETWORK_MODIFIER: u32 = 0x4b;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CountryRow {
    pub country: u32,
    pub unit_modifier: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateRow {
    pub state: u32,
    pub country: u32,
    pub network_modifier: i64,
}

fn outcome(len: usize, unread: usize) -> Outcome {
    match u32::try_from(unread) {
        Ok(0) => Outcome::Rows { len },
        Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
        Err(_) => Outcome::Unavailable {
            reason: "visibility row count overflow",
        },
    }
}

/// Every country with a nonzero effective unit modifier; absence is zero.
/// Also returns the live country-slot count, for sizing off the game thread.
pub fn countries(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [CountryRow],
) -> (Outcome, usize) {
    let mut slots = 0;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        slots = countries.count() as usize;
        let (mut len, mut unread) = (0, 0);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let accumulator = country
                .pointer
                .checked_add(ccountry::MODIFIER_ACCUMULATOR)
                .ok_or("country modifier address overflow")?;
            let value = modifier_accumulator::lookup_effective_at(
                memory,
                base,
                accumulator,
                UNIT_MODIFIER,
            )?
            .effective;
            if value == 0 {
                continue;
            }
            match output.get_mut(len) {
                Some(row) => {
                    *row = CountryRow {
                        country: country.id,
                        unit_modifier: value,
                    };
                    len += 1;
                }
                None => unread += 1,
            }
        }
        Ok(outcome(len, unread))
    })();
    (
        result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        slots,
    )
}

const STATE_FIELDS: [RecordField; 4] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: cstate::ID,
        destination_offset: 8,
        len: 4,
    },
    RecordField {
        source_offset: cstate::OWNER,
        destination_offset: 12,
        len: 4,
    },
    RecordField {
        source_offset: cstate::CONTROLLER,
        destination_offset: 16,
        len: 4,
    },
];
const STATE_BYTES: usize = 20;
const STATE_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&STATE_FIELDS, STATE_BYTES).expect("valid state layout");
const BATCH: usize = 64;

/// Every state whose raw owner and controller differ and whose owner's
/// network modifier is nonzero; the fog rebuild adds nothing for the rest.
pub fn states(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [StateRow],
) -> Outcome {
    let result = (|| {
        let array = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))
            .filter(|array| array.count > 1)
            .ok_or("state array unreadable or empty")?;
        let vtable = base
            .checked_add(cstate::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let (mut len, mut unread) = (0, 0);
        let mut slots = [0_u8; BATCH * 8];
        let mut pointers = [0_usize; BATCH];
        let mut records = [0_u8; BATCH * STATE_BYTES];
        let mut first = 1;
        while first < array.count {
            let count = ((array.count - first) as usize).min(BATCH);
            let address = array
                .element(first, 8)
                .ok_or("state array address overflow")?;
            if !read_slice(memory, address, &mut slots[..count * 8]) {
                return Err("state pointers unreadable");
            }
            let mut live = 0;
            for chunk in slots[..count * 8].chunks_exact(8) {
                let pointer = usize::try_from(u64::from_le_bytes(chunk.try_into().unwrap()))
                    .map_err(|_| "state pointer overflow")?;
                if pointer != 0 {
                    pointers[live] = pointer;
                    live += 1;
                }
            }
            let copied = memory.copy_records(
                RecordSources::Pointers(&pointers[..live]),
                &STATE_LAYOUT,
                &mut records[..live * STATE_BYTES],
            );
            if copied < live * STATE_FIELDS.len() {
                return Err("state fields unreadable");
            }
            for (&pointer, record) in pointers[..live]
                .iter()
                .zip(records.chunks_exact(STATE_BYTES))
            {
                let word = |at: usize| u32::from_le_bytes(record[at..at + 4].try_into().unwrap());
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != vtable {
                    return Err("state vtable mismatch");
                }
                let (state, owner, controller) = (word(8), word(12), word(16));
                if owner == controller || owner == 0 {
                    continue;
                }
                let network_modifier = cstate::movement_modifier(
                    memory,
                    base,
                    game_state,
                    pointer,
                    owner,
                    NETWORK_MODIFIER,
                )?;
                if network_modifier == 0 {
                    continue;
                }
                match output.get_mut(len) {
                    Some(row) => {
                        *row = StateRow {
                            state,
                            country: owner,
                            network_modifier,
                        };
                        len += 1;
                    }
                    None => unread += 1,
                }
            }
            first += count as u32;
        }
        Ok(outcome(len, unread))
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// The simulation's DLC feature mask.
pub fn dlc_mask(memory: &impl Memory, base: usize) -> Result<u32, &'static str> {
    base.checked_add(dlc_feature_state::MASK_RVA)
        .and_then(|address| read(memory, address))
        .ok_or("simulation DLC mask unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const STATE: usize = 0x100;
    const ARRAY: usize = 0x200;
    const ROWS: usize = 0x400;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn states_fixture(owner_controller: &[(u32, u32)]) -> Vec<u8> {
        let mut bytes = vec![0; 0x4000];
        let count = owner_controller.len() as u32 + 1;
        put(
            &mut bytes,
            STATE + cgame_state::STATES,
            &(ARRAY as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            STATE + cgame_state::STATES + 8,
            &count.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATE + cgame_state::STATES + 12,
            &count.to_le_bytes(),
        );
        for (index, &(owner, controller)) in owner_controller.iter().enumerate() {
            let state = ROWS + index * 0x1000;
            put(
                &mut bytes,
                ARRAY + (index + 1) * 8,
                &(state as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                state,
                &(cstate::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                state + cstate::ID,
                &(index as u32 + 1).to_le_bytes(),
            );
            put(&mut bytes, state + cstate::OWNER, &owner.to_le_bytes());
            put(
                &mut bytes,
                state + cstate::CONTROLLER,
                &controller.to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn controlled_own_states_are_skipped_without_modifier_reads() {
        let bytes = states_fixture(&[(1, 1), (0, 2)]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [StateRow::default(); 2];
        assert_eq!(
            states(&memory, 0, STATE, &mut rows),
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn an_occupied_state_with_an_unreadable_modifier_fails_the_table() {
        let bytes = states_fixture(&[(1, 2)]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [StateRow::default(); 1];
        assert!(matches!(
            states(&memory, 0, STATE, &mut rows),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn a_foreign_state_vtable_fails_the_table() {
        let mut bytes = states_fixture(&[(1, 1)]);
        put(&mut bytes, ROWS, &7_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            states(&memory, 0, STATE, &mut []),
            Outcome::Unavailable {
                reason: "state vtable mismatch"
            }
        );
    }
}
