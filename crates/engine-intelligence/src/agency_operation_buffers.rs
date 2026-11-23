
use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, read, walk::{Buffer, Outcome}};
use engine_native::{ccountry, cgame_state};
use engine_native_intelligence::coperation_instance;
use engine_native_intelligence::coperation;
use engine_native::{cprovince, cstate};
use engine_native_parts::{creference_registry, modifier_definition::NamedKey};

pub const BUFFER_RVA: Rva = Rva(0x033e_b190);
pub const BUCKET_OFFSETS: [usize; 5] = [0x00, 0x48, 0x18, 0x30, 0x60];
pub const REVISION: usize = 0xa8;
pub const SPEED: &[u8] = b"intelligence_operation_speed";

pub const fn speed_key() -> NamedKey {
    NamedKey::new(SPEED)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    pub entries_start: usize,
    pub counts: [usize; 5],
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub definition: u32,
    pub country: u32,
    pub duration: Option<i32>,
    pub operation: u64,
    pub state: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub entries_len: usize,
    pub required_entries: usize,
}

pub fn count(memory: &impl Memory, base: usize, state: usize) -> Walk {
    walk_buffer(memory, base, state, Buffer::Count, Buffer::Count, &mut speed_key())
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    state: usize,
    rows: &mut Vec<Row>,
    entries: &mut Vec<Entry>,
    speed: &mut NamedKey,
) -> Walk {
    walk_buffer(
        memory,
        base,
        state,
        Buffer::Growing(rows),
        Buffer::Growing(entries),
        speed,
    )
}

fn word<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("agency operation buffer field unreadable")
}

fn global<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|at| read(memory, at))
        .ok_or("agency operation buffer field unreadable")
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    vtable: Rva,
) -> Result<(), &'static str> {
    let vtable = engine_sites::at(base, vtable)
        .ok_or("agency operation buffer object vtable mismatch")?;
    if read::<u64>(memory, object) != Some(vtable as u64) {
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

fn offer_state(
    memory: &impl Memory,
    base: usize,
    state: usize,
    countries: &ccountry::Reader<'_, impl Memory>,
    country: u32,
) -> Result<u32, &'static str> {
    let target = countries.resolve(country)?;
    let capital = word::<i32>(memory, target.pointer, ccountry::CAPITAL)?;
    let states = Array::read(memory, state.checked_add(cgame_state::STATES).ok_or("agency offer state root overflow")?, 8).ok_or("agency offer state roster invalid")?;
    if capital <= 0 || capital as u32 >= states.count {
        return Ok(0);
    }
    let pointer = states.element(capital as u32, 8).and_then(|address| read::<u64>(memory, address)).ok_or("agency offer capital unreadable")? as usize;
    if pointer == 0 {
        return Ok(0);
    }
    authenticate(memory, base, pointer, cstate::VTABLE_RVA)?;
    if word::<u32>(memory, pointer, cstate::ID)? != capital as u32 {
        return Err("agency offer capital identity mismatch");
    }
    Ok(capital as u32)
}

const OPERATION_DAYS: usize = 0x148;
const TIME_MULTIPLIER: usize = 0x500;
const HAS_TIME_MULTIPLIER: usize = 0x508;
const SCALE_WITH_ALL_TARGETS: usize = 0x7c9;
const DEFAULT_TIME_MULTIPLIER_RVA: Rva = Rva(0x033e_04f0);
const DEFENSE_COST_FACTOR_RVA: Rva = Rva(0x033e_0120);
const COMPLETION_COUNTS: usize = 0x28;
const COMPLETION_MAP_DATA: usize = 0x10;
const COMPLETION_MAP_MASK: usize = 0x1c;
const COMPLETION_MAP_OVERFLOW: usize = 0x20;
const COMPLETION_ENTRY_STRIDE: usize = 0x30;
const COMPLETION_ENTRY_PROBE: usize = 4;
const COMPLETION_ENTRY_OPERATION: usize = 8;
const COMPLETION_ENTRY_TARGETS: usize = 0x10;
const TARGET_MAP_DATA: usize = 8;
const TARGET_MAP_MASK: usize = 0x14;
const TARGET_MAP_OVERFLOW: usize = 0x18;
const TARGET_ENTRY_STRIDE: usize = 12;
const TARGET_ENTRY_PROBE: usize = 0;
const TARGET_ENTRY_COUNTRY: usize = 4;
const TARGET_ENTRY_COUNT: usize = 8;
const OPERATION_MANAGER_OWNER: usize = 8;

fn completed_count<M: Memory>(memory: &M, countries: &ccountry::Reader<'_, M>, manager: usize, definition: usize, token: u32, country: u32) -> Result<i32, &'static str> {
    let map = manager.checked_add(COMPLETION_COUNTS).ok_or("operation history root overflow")?;
    let data = word::<u64>(memory, map, COMPLETION_MAP_DATA)? as usize;
    let mask = word::<u32>(memory, map, COMPLETION_MAP_MASK)?;
    let overflow = word::<u8>(memory, map, COMPLETION_MAP_OVERFLOW)?;
    let slots = map_extent(mask, overflow)?;
    let mut hash = (token >> 16 ^ token).wrapping_mul(0x45d9f3b);
    hash = (hash >> 16 ^ hash).wrapping_mul(0x45d9f3b);
    let mut slot = (hash >> 16 ^ hash) & mask;
    let mut distance = 1u32;
    let entry = loop {
        if slot as usize >= slots || distance > u32::from(u8::MAX) {
            return Err("operation history probe exceeds storage");
        }
        let entry = data.checked_add((slot as usize).checked_mul(COMPLETION_ENTRY_STRIDE).ok_or("operation history slot overflow")?).ok_or("operation history entry overflow")?;
        let probe = word::<u8>(memory, entry, COMPLETION_ENTRY_PROBE)?;
        if probe == u8::MAX || u32::from(probe) < distance { return Ok(0); }
        if word::<u32>(memory, entry, COMPLETION_ENTRY_OPERATION)? == token { break entry; }
        distance += 1;
        slot = slot.checked_add(1).ok_or("operation history probe overflow")?;
    };
    let targets = entry.checked_add(COMPLETION_ENTRY_TARGETS).ok_or("operation target history overflow")?;
    let data = word::<u64>(memory, targets, TARGET_MAP_DATA)? as usize;
    let mask = word::<u32>(memory, targets, TARGET_MAP_MASK)?;
    let overflow = word::<u8>(memory, targets, TARGET_MAP_OVERFLOW)?;
    let slots = map_extent(mask, overflow)?;
    if word::<u8>(memory, definition, SCALE_WITH_ALL_TARGETS)? != 0 {
        let mut total = 0i32;
        for slot in 0..slots {
            let entry = data.checked_add((slot as usize).checked_mul(TARGET_ENTRY_STRIDE).ok_or("operation target slot overflow")?).ok_or("operation target entry overflow")?;
            if word::<u8>(memory, entry, TARGET_ENTRY_PROBE)? != 0 {
                total = total.wrapping_add(word::<i32>(memory, entry, TARGET_ENTRY_COUNT)?);
            }
        }
        return Ok(total);
    }
    let canonical = |raw| if raw == 0 { Ok(0) } else { countries.resolve(raw).map(|country| country.canonical_slot) };
    let target = canonical(country)?;
    let mut slot = target & mask;
    let mut distance = 1u32;
    loop {
        if slot as usize >= slots || distance > u32::from(u8::MAX) {
            return Err("operation target history probe exceeds storage");
        }
        let entry = data.checked_add((slot as usize).checked_mul(TARGET_ENTRY_STRIDE).ok_or("operation target slot overflow")?).ok_or("operation target entry overflow")?;
        let probe = word::<u8>(memory, entry, TARGET_ENTRY_PROBE)?;
        if probe == u8::MAX || u32::from(probe) < distance { return Ok(0); }
        if canonical(word::<u32>(memory, entry, TARGET_ENTRY_COUNTRY)?)? == target { return word(memory, entry, TARGET_ENTRY_COUNT); }
        distance += 1;
        slot = slot.checked_add(1).ok_or("operation target probe overflow")?;
    }
}

fn map_extent(mask: u32, overflow: u8) -> Result<usize, &'static str> {
    let buckets = mask.checked_add(1).ok_or("operation history mask overflow")?;
    if !buckets.is_power_of_two() {
        return Err("operation history mask invalid");
    }
    (buckets as usize).checked_add(usize::from(overflow))
        .ok_or("operation history extent overflow")
}

fn offer_duration<M: Memory>(memory: &M, base: usize, countries: &ccountry::Reader<'_, M>, owner: u32, definition: usize, token: u32, target: u32, speed_key: u32) -> Result<i32, &'static str> {
    use engine_native_intelligence::cintelligence_agency;
    use engine_native_parts::modifier_accumulator;
    let owner = countries.resolve(owner)?;
    let speed = modifier_accumulator::lookup_effective_at(memory, base, owner.pointer.checked_add(engine_native_memory::country_identity::MODIFIER_ACCUMULATOR).ok_or("operation owner accumulator overflow")?, speed_key)?.effective;
    let multiplier = if word::<u8>(memory, definition, HAS_TIME_MULTIPLIER)? != 0 {
        word::<i64>(memory, definition, TIME_MULTIPLIER)?
    } else {
        global::<i64>(memory, base, DEFAULT_TIME_MULTIPLIER_RVA)?
    };
    let repeated = if multiplier == 0 { 0 } else {
        let manager = word::<u64>(memory, owner.pointer, ccountry::OPERATION_MANAGER)? as usize;
        if manager == 0 || word::<u64>(memory, manager, OPERATION_MANAGER_OWNER)? != owner.pointer as u64 { return Err("operation manager owner mismatch"); }
        i64::from(completed_count(memory, countries, manager, definition, token, target)?).wrapping_mul(multiplier)
    };
    let target = countries.resolve(target)?;
    let agency = cintelligence_agency::root(memory, base, target)?.ok_or("operation target agency unavailable")?;
    let defense = word::<i64>(memory, agency, cintelligence_agency::DEFENSE)?;
    let defense = defense.wrapping_mul(global::<i64>(memory, base, DEFENSE_COST_FACTOR_RVA)?) / 100_000;
    let factor = defense.wrapping_add(100_000).wrapping_mul(repeated.wrapping_add(100_000)) / 100_000;
    let factor = factor.wrapping_mul(100_000i64.wrapping_sub(speed)) / 100_000;
    Ok((factor.wrapping_mul(i64::from(word::<i32>(memory, definition, OPERATION_DAYS)?)) / 100_000) as i32)
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    state: usize,
    mut rows: Buffer<'_, Row>,
    mut entries: Buffer<'_, Entry>,
    speed: &mut NamedKey,
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
        let buffer = engine_sites::at(base, BUFFER_RVA)
            .ok_or("agency operation buffer root overflow")?;
        let revision = word::<u32>(memory, buffer, REVISION)?;
        let mut lists = [Array::default(); 5];
        let mut counts = [0; 5];
        let mut required = 0usize;
        for bucket in 0..5 {
            lists[bucket] = Array::read(
                memory,
                buffer
                    .checked_add(BUCKET_OFFSETS[bucket])
                    .ok_or("agency operation bucket overflow")?,
                if bucket == 4 { 16 } else { 8 },
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
        for (bucket, list) in lists.into_iter().enumerate() {
            for ordinal in 0..list.count {
                if bucket == 4 {
                    let entry = list.element(ordinal, 16).ok_or("agency operation offer address overflow")?;
                    let pointer = word::<u64>(memory, entry, 0)? as usize;
                    let definition = coperation::definition(memory, base, pointer)?;
                    let country = word::<u32>(memory, entry, 8)?;
                    let state = offer_state(memory, base, state, &countries, country)?;
                    if fits {
                        let duration = speed
                            .get(memory, base)
                            .and_then(|key| key.ok_or("operation speed modifier absent"))
                            .and_then(|key| offer_duration(memory, base, &countries, viewing_country, pointer, definition, country, key))
                            .ok();
                        entries[written] = Entry { definition, country, duration, operation: 0, state: Some(state) };
                    }
                    written += 1;
                    continue;
                }
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
                        definition: 0,
                        country: 0,
                        duration: None,
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
        for (bucket, (offset, expected)) in BUCKET_OFFSETS.into_iter().zip(lists).enumerate() {
            if Array::read(memory, buffer + offset, if bucket == 4 { 16 } else { 8 }) != Some(expected) {
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
    use engine_native::ccountry::synthetic::World;
    use engine_native_parts::creference_registry::test_support::Fixture;

    fn fixture(buckets: [&[u64]; 4]) -> (World, Fixture) {
        let mut world = World::new(0, 1, ccountry::CAPITAL + 8);
        let state = world.game_state();
        world.u32(state + cgame_state::LOCAL_PLAYER_COUNTRY, 1);
        let mut memory = Fixture::new(coperation_instance::REFERENCE_TYPE);
        memory.put(BUFFER_RVA.0 + BUCKET_OFFSETS[4], &[0; 16]);
        for (offset, entries) in BUCKET_OFFSETS.into_iter().zip(buckets) {
            let data = world.alloc(entries.len() * 8);
            for (i, handle) in entries.iter().enumerate() {
                world.u64(data + i * 8, *handle);
            }
            memory.put(BUFFER_RVA.0 + offset, &(data as u64).to_le_bytes());
            memory.put(
                BUFFER_RVA.0 + offset + 8,
                &(entries.len() as u32).to_le_bytes(),
            );
            memory.put(
                BUFFER_RVA.0 + offset + 12,
                &(entries.len() as u32).to_le_bytes(),
            );
        }
        memory.put(BUFFER_RVA.0 + REVISION, &3_u32.to_le_bytes());
        (world, memory)
    }

    fn capture(world: &World, memory: &mut Fixture) -> (Walk, Vec<Row>, Vec<Entry>) {
        memory.put(engine_native::ccountry::synthetic::START, &world.bytes);
        let mut rows = Vec::new();
        let mut entries = Vec::new();
        let result = walk(memory, 0, world.game_state(), &mut rows, &mut entries, &mut speed_key());
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
        assert_eq!(rows[0].counts, [2, 1, 0, 1, 0]);
        assert_eq!(
            entries,
            [a, 0, b, a].map(|operation| Entry {
                operation,
                state: None,
                ..Entry::default()
            })
        );
    }

    #[test]
    fn resolved_capital_is_authenticated_and_bad_headers_are_unavailable() {
        let handle = (9_u64 << 32) | u64::from(coperation_instance::REFERENCE_TYPE);
        let (mut world, mut memory) = fixture([&[handle], &[], &[], &[]]);
        let operation = world.alloc(coperation_instance::TARGET_PROVINCE + 8);
        world.u64(operation, coperation_instance::VTABLE_RVA.0 as u64);
        world.u64(operation + coperation_instance::HANDLE, handle);
        world.u32(operation + coperation_instance::TARGET_COUNTRY_ID, 1);
        let capital = world.alloc(cstate::ID + 4);
        world.u64(capital, cstate::VTABLE_RVA.0 as u64);
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
                state: Some(1),
                ..Entry::default()
            }]
        );
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
        memory.put(BUFFER_RVA.0 + 12, &2_u32.to_le_bytes());
        assert_eq!(
            capture(&world, &mut memory).0.outcome,
            Outcome::Unavailable {
                reason: "agency operation bucket header invalid"
            }
        );
    }
}
