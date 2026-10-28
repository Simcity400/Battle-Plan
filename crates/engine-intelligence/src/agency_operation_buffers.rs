//! The client's retained agency operation buckets, exactly as state map icons visit them.
//!
//! This is one shared client buffer, populated for the clock's viewing country, not a
//! per-country saved roster. A perspective change may leave its previous contents intact.
//! Keep that snapshot, including empty lists and unresolved references; never rebuild it.

use crate::{
    Memory, ccountry, cgame_state, coperation_instance,
    cpdx_array::Array,
    read,
    walk::{Buffer, Outcome},
};
use engine_native::{cprovince, creference_registry, cstate};

/// Inline agency UI operation buffer in the authenticated executable image.
pub const BUFFER_RVA: usize = 0x033e_b190;
/// Completed, running, ready preparation, unready preparation: native visit order.
pub const BUCKET_OFFSETS: [usize; 4] = [0x00, 0x48, 0x18, 0x30];
/// UI buffer publication counter; used only to reject a changing observation.
pub const REVISION: usize = 0xa8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub entries_start: usize,
    pub counts: [usize; 4],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub operation: u64,
    /// None for an unresolved reference; Some(0) for a resolved operation with no state.
    pub state: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub entries_len: usize,
    pub required_entries: usize,
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut [Row],
    entries: &mut [Entry],
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        Buffer::Fixed(rows),
        Buffer::Fixed(entries),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut Vec<Row>,
    entries: &mut Vec<Entry>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        Buffer::Growing(rows),
        Buffer::Growing(entries),
    )
}

fn word<T: engine_native::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("agency operation buffer field unreadable")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: usize,
) -> Result<(), &'static str> {
    if base.checked_add(vtable).map(|v| v as u64) != read::<u64>(memory, object) {
        return Err("agency operation buffer object vtable mismatch");
    }
    Ok(())
}

fn target_state(
    memory: &impl Memory,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    operation: usize,
) -> Result<u32, &'static str> {
    let province = word::<u64>(memory, operation, coperation_instance::TARGET_PROVINCE)? as usize;
    let states = Array::read(
        memory,
        state
            .checked_add(cgame_state::STATES)
            .ok_or("agency operation state root overflow")?,
        8,
    )
    .ok_or("agency operation state roster invalid")?;
    let pointer = if province != 0 {
        authenticate(memory, base, province, cprovince::VTABLE_RVA)?;
        word::<u64>(memory, province, cprovince::STATE)? as usize
    } else {
        let target = word::<u32>(memory, operation, coperation_instance::TARGET_COUNTRY_ID)?;
        if target == 0 {
            return Ok(0);
        }
        let country = countries.resolve(target)?;
        let capital = word::<i32>(memory, country.pointer, ccountry::CAPITAL)?;
        if capital <= 0 || capital as u32 >= states.count {
            return Ok(0);
        }
        states
            .element(capital as u32, 8)
            .and_then(|at| read::<u64>(memory, at))
            .ok_or("agency operation capital state unreadable")? as usize
    };
    if pointer == 0 {
        return Ok(0);
    }
    authenticate(memory, base, pointer, cstate::VTABLE_RVA)?;
    let id = word::<u32>(memory, pointer, cstate::ID)?;
    if states.element(id, 8).and_then(|at| read::<u64>(memory, at)) != Some(pointer as u64) {
        return Err("agency operation target state identity mismatch");
    }
    Ok(id)
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut rows: Buffer<'_, Row>,
    mut entries: Buffer<'_, Entry>,
) -> Walk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, state)?;
        let player = cgame_state::local_player(memory, state)?;
        let viewing_country = if (player[0] as i32) > 0 {
            player[0]
        } else {
            player[1]
        };
        if viewing_country != 0 {
            countries.resolve(viewing_country)?;
        }
        let buffer = base
            .checked_add(BUFFER_RVA)
            .ok_or("agency operation buffer root overflow")?;
        let revision = word::<u32>(memory, buffer, REVISION)?;
        let mut lists = [Array::default(); 4];
        let mut counts = [0; 4];
        let mut required = 0usize;
        for bucket in 0..4 {
            lists[bucket] = Array::read(
                memory,
                buffer
                    .checked_add(BUCKET_OFFSETS[bucket])
                    .ok_or("agency operation bucket overflow")?,
                8,
            )
            .ok_or("agency operation bucket header invalid")?;
            counts[bucket] = lists[bucket].count as usize;
            required = required
                .checked_add(counts[bucket])
                .ok_or("agency operation buffer demand overflow")?;
        }
        rows.ensure(1)?;
        entries.ensure(required)?;
        let fits = !rows.is_empty() && entries.len() >= required;
        let mut resolver = creference_registry::ScopedResolver::new(base);
        let mut written = 0;
        for list in lists {
            for ordinal in 0..list.count {
                let operation = list
                    .element(ordinal, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .ok_or("agency operation buffer reference unreadable")?;
                let target = match resolver.resolve(memory, operation)? {
                    None => None,
                    Some(pointer) => {
                        authenticate(memory, base, pointer, coperation_instance::VTABLE_RVA)?;
                        if word::<u64>(memory, pointer, coperation_instance::HANDLE)? != operation {
                            return Err("agency operation buffer handle mismatch");
                        }
                        Some(target_state(memory, base, state, &countries, pointer)?)
                    }
                };
                if fits {
                    entries[written] = Entry {
                        operation,
                        state: target,
                    };
                }
                written += 1;
            }
        }
        if word::<u32>(memory, buffer, REVISION)? != revision
            || cgame_state::local_player(memory, state)? != player
        {
            return Err("agency operation buffer changed during capture");
        }
        for (offset, expected) in BUCKET_OFFSETS.into_iter().zip(lists) {
            if Array::read(memory, buffer + offset, 8) != Some(expected) {
                return Err("agency operation bucket changed during capture");
            }
        }
        if fits {
            rows[0] = Row {
                entries_start: 0,
                counts,
            };
        }
        Ok(Walk {
            outcome: if fits {
                Outcome::Rows { len: 1 }
            } else {
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 1,
                }
            },
            entries_len: if fits { written } else { 0 },
            required_entries: required,
        })
    })();
    result.unwrap_or_else(|reason| Walk {
        outcome: Outcome::Unavailable { reason },
        entries_len: 0,
        required_entries: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::{ccountry::synthetic::World, creference_registry::test_support::Fixture};

    fn fixture(buckets: [&[u64]; 4]) -> (World, Fixture) {
        let mut world = World::new(0, 1, ccountry::CAPITAL + 8);
        let state = world.game_state();
        world.u32(state + cgame_state::LOCAL_PLAYER_COUNTRY, 1);
        let mut memory = Fixture::new(coperation_instance::REFERENCE_TYPE);
        for (offset, entries) in BUCKET_OFFSETS.into_iter().zip(buckets) {
            let data = world.alloc(entries.len() * 8);
            for (i, handle) in entries.iter().enumerate() {
                world.u64(data + i * 8, *handle);
            }
            memory.put(BUFFER_RVA + offset, &(data as u64).to_le_bytes());
            memory.put(
                BUFFER_RVA + offset + 8,
                &(entries.len() as u32).to_le_bytes(),
            );
            memory.put(
                BUFFER_RVA + offset + 12,
                &(entries.len() as u32).to_le_bytes(),
            );
        }
        memory.put(BUFFER_RVA + REVISION, &3_u32.to_le_bytes());
        (world, memory)
    }

    fn capture(world: &World, memory: &mut Fixture) -> (Walk, Vec<Row>, Vec<Entry>) {
        memory.put(engine_native::ccountry::synthetic::START, &world.bytes);
        let mut rows = Vec::new();
        let mut entries = Vec::new();
        let result = walk_growing(memory, 0, world.game_state(), &mut rows, &mut entries);
        (result, rows, entries)
    }

    #[test]
    fn empty_is_a_snapshot_and_stale_references_keep_native_bucket_order() {
        let (world, mut memory) = fixture([&[], &[], &[], &[]]);
        let (result, rows, entries) = capture(&world, &mut memory);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows, [Row::default()]);
        assert!(entries.is_empty());
        let a = (9_u64 << 32) | u64::from(coperation_instance::REFERENCE_TYPE);
        let b = (3_u64 << 32) | u64::from(coperation_instance::REFERENCE_TYPE);
        let (world, mut memory) = fixture([&[a, 0], &[b], &[], &[a]]);
        let (result, rows, entries) = capture(&world, &mut memory);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].counts, [2, 1, 0, 1]);
        assert_eq!(
            entries,
            [a, 0, b, a].map(|operation| Entry {
                operation,
                state: None
            })
        );
        let short = walk(
            &memory,
            0,
            world.game_state(),
            &mut [Row::default()],
            &mut [],
        );
        assert_eq!(short.required_entries, 4);
        assert_eq!(
            short.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn resolved_capital_is_authenticated_and_bad_headers_are_unavailable() {
        let handle = (9_u64 << 32) | u64::from(coperation_instance::REFERENCE_TYPE);
        let (mut world, mut memory) = fixture([&[handle], &[], &[], &[]]);
        let operation = world.alloc(coperation_instance::TARGET_PROVINCE + 8);
        world.u64(operation, coperation_instance::VTABLE_RVA as u64);
        world.u64(operation + coperation_instance::HANDLE, handle);
        world.u32(operation + coperation_instance::TARGET_COUNTRY_ID, 1);
        let capital = world.alloc(cstate::ID + 4);
        world.u64(capital, cstate::VTABLE_RVA as u64);
        world.u32(capital + cstate::ID, 1);
        let states = world.alloc(16);
        world.u64(states + 8, capital as u64);
        world.array(world.game_state() + cgame_state::STATES, states, 2);
        world.u32(world.object(1) + ccountry::CAPITAL, 1);
        memory.entry(Fixture::index(handle), 1, handle, operation as u64);
        let (result, _, entries) = capture(&world, &mut memory);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(
            entries,
            [Entry {
                operation: handle,
                state: Some(1)
            }]
        );
        // The first out-of-range positive index, and larger indices, return
        // no state without reading a missing slot or withholding other buckets.
        for index in [2, i32::MAX] {
            world.u32(world.object(1) + ccountry::CAPITAL, index as u32);
            let (result, _, entries) = capture(&world, &mut memory);
            assert_eq!(result.outcome, Outcome::Rows { len: 1 });
            assert_eq!(entries[0].state, Some(0));
        }
        world.u32(world.object(1) + ccountry::CAPITAL, 1);
        world.u64(capital, 0);
        assert!(matches!(
            capture(&world, &mut memory).0.outcome,
            Outcome::Unavailable { .. }
        ));
        memory.put(BUFFER_RVA + 12, &2_u32.to_le_bytes());
        assert_eq!(
            capture(&world, &mut memory).0.outcome,
            Outcome::Unavailable {
                reason: "agency operation bucket header invalid"
            }
        );
    }
}
