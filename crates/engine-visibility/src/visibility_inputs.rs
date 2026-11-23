
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources, read_slice},
    read,
    walk::{Buffer, Outcome},
};
use engine_native::{ccountry, cgame_state, cstate};
use crate::dlc_feature_state;
use engine_native_parts::modifier_accumulator;
use engine_sites::Rva;

pub const UNIT_MODIFIER: u32 = crate::modifier_keys::INTEL_FROM_COMBAT_FACTOR;
pub const NETWORK_MODIFIER: u32 = crate::modifier_keys::LOCAL_INTEL_TO_ENEMIES;

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

pub fn count_countries(memory: &impl Memory, base: usize, game_state: usize) -> (Outcome, usize) {
    countries_buffer(memory, base, game_state, Buffer::Count)
}

pub fn countries(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CountryRow>,
) -> (Outcome, usize) {
    countries_buffer(memory, base, game_state, Buffer::Growing(output))
}

pub fn countries_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: Buffer<'_, CountryRow>,
) -> (Outcome, usize) {
    countries_buffer_with(memory, base, game_state, output, &mut |_, accumulator| {
        modifier_accumulator::lookup_at(memory, accumulator, UNIT_MODIFIER)
    })
}

pub fn countries_buffer_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, CountryRow>,
    lookup: &mut impl FnMut(
        engine_native_memory::country_identity::Country,
        usize,
    ) -> Result<engine_native_memory::modifier_cache::Value, &'static str>,
) -> (Outcome, usize) {
    let mut slots = 0;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        slots = countries.count() as usize;
        output.ensure(slots)?;
        let (mut len, mut unread) = (0, 0);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let accumulator = country
                .pointer
                .checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)
                .ok_or("country modifier address overflow")?;
            let raw = lookup(country, accumulator)?;
            let value = if raw.present {
                engine_native_parts::modifier_definition::effective(memory, base, UNIT_MODIFIER, raw.raw)?
            } else {
                0
            };
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

pub fn count_states(memory: &impl Memory, base: usize, game_state: usize) -> Outcome {
    states_buffer(memory, base, game_state, Buffer::Count)
}

pub fn states(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<StateRow>,
) -> Outcome {
    states_buffer(memory, base, game_state, Buffer::Growing(output))
}

pub fn states_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: Buffer<'_, StateRow>,
) -> Outcome {
    states_buffer_extent(memory, base, game_state, output, &mut None)
}

pub fn states_buffer_extent(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, StateRow>,
    extent: &mut Option<u32>,
) -> Outcome {
    *extent = None;
    let result = (|| {
        let array = game_state
            .checked_add(cgame_state::STATES)
            .and_then(|address| Array::read(memory, address, 8))
            .filter(|array| array.count > 1)
            .ok_or("state array unreadable or empty")?;
        *extent = Some(array.count);
        output.ensure(array.count as usize)?;
        let vtable =
            engine_sites::at(base, cstate::VTABLE_RVA).ok_or("module address overflow")? as u64;
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

pub const HUMAN_AI_RVA: Rva = Rva(0x033d_daa9);

pub const INSTANT_DECISIONS_RVA: Rva = Rva(0x033d_daa3);

pub fn instant_decisions(memory: &impl Memory, base: usize) -> Result<u8, &'static str> {
    engine_sites::at(base, INSTANT_DECISIONS_RVA)
        .and_then(|address| read(memory, address))
        .ok_or("instant decisions byte unreadable")
}

pub fn human_ai(memory: &impl Memory, base: usize) -> Result<u8, &'static str> {
    engine_sites::at(base, HUMAN_AI_RVA)
        .and_then(|address| read(memory, address))
        .ok_or("human_ai byte unreadable")
}

pub fn dlc_mask(memory: &impl Memory, base: usize) -> Result<u32, &'static str> {
    engine_sites::at(base, dlc_feature_state::MASK_RVA)
        .and_then(|address| read(memory, address))
        .ok_or("simulation DLC mask unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

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
                &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
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
        let mut rows = vec![StateRow::default(); 2];
        assert_eq!(
            states(&memory, 0, STATE, &mut rows),
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn scripted_state_extent_reuses_the_census_header_and_keeps_null_holes() {
        let mut bytes = states_fixture(&[(1, 1), (1, 1), (1, 1)]);
        put(&mut bytes, ARRAY + 2 * 8, &0_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut extent = None;
        assert_eq!(
            states_buffer_extent(&memory, 0, STATE, Buffer::Count, &mut extent),
            Outcome::Rows { len: 0 }
        );
        assert_eq!(extent, Some(4));
    }

    #[test]
    fn an_occupied_state_with_an_unreadable_modifier_fails_the_table() {
        let bytes = states_fixture(&[(1, 2)]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![StateRow::default(); 1];
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
            states(&memory, 0, STATE, &mut Vec::new()),
            Outcome::Unavailable {
                reason: "state vtable mismatch"
            }
        );
    }
}
