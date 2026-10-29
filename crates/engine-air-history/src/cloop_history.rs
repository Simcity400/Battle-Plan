//! CLoopHistory layout and air-statistics channel schema.
//!
//! The 24 named channels are a subset of the 89-field history record.
//! Values remain signed i64; display divisors do not justify narrowing or
//! rounding recorded values.

use crate::{
    Memory,
    cpdx_array::Array,
    cstrategic_air, loop_history_record_fields, loop_history_window,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
    walk::Outcome,
};

/// One region-table slot pointer, gathered for a whole batch of slots.
const SLOT_FIELD: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: 8,
}];
const SLOT_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&SLOT_FIELD, 8).expect("valid record layout");

/// CLoopHistory primary vtable.
pub const VTABLE_RVA: usize = 0x02a4_bc68;
/// Embedded history queue, followed by its three window pointers.
pub const QUEUE: usize = 0x08;
/// CLoopHistory daily window pointer.
pub const DAILY_WINDOW: usize = 0x10;
/// Records in the daily window.
pub const DAILY_WINDOW_CAPACITY: u32 = 30;
/// The third history window advances when the twelve-record second window wraps.
pub const WINDOW: usize = 0x20;
/// History self-reference pointer, required to equal its object.
pub const SELF_REFERENCE: usize = 0x28;
/// History roll-up mode, signed i32: zero sums wrapped records,
/// one keeps the latest record.
pub const ROLL_UP: usize = 0x30;
/// History field-count u32; air schema requires 89 fields.
pub const FIELD_COUNT: usize = 0x34;

/// A country's current annual-ring air-statistics bucket in one air region.
/// This ring advances after twelve thirty-day buckets, not calendar years.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub region: u32,
    pub year_slot: u32,
    pub year_window_wrapped: u8,
    /// Native i64 values, in `air_stats::CHANNELS` order, without display scaling.
    pub channels: [i64; 24],
}

impl Row {
    pub fn key(self) -> u64 {
        (u64::from(self.country) << 32) | u64::from(self.region)
    }
}

/// Reads every allocated region's current annual-ring bucket for each country.
/// An unallocated country air manager, history table or region slot is absent.
/// A malformed allocated object makes the table unavailable. Capacity bounds
/// row storage; remaining allocated histories are counted without field reads.
pub fn walk(memory: &impl Memory, base: usize, game_state: usize, output: &mut [Row]) -> Outcome {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
) -> Outcome {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    match walk_inner(memory, base, game_state, output) {
        Ok((len, 0)) => Outcome::Rows { len },
        Ok((len, unread_rows)) => Outcome::Truncated { len, unread_rows },
        Err(reason) => Outcome::Unavailable { reason },
    }
}

fn walk_inner(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Result<(usize, u32), &'static str> {
    let mut len = 0_usize;
    let mut unread_rows = 0_u32;
    // The region-indexed history table is a contiguous pointer vector. One
    // guarded round per batch replaces one read per region slot; each slot is
    // still decoded in order, and the first unreadable slot keeps its position.
    const BATCH: usize = 64;
    let mut slots = [0_usize; BATCH];
    let mut slot_bytes = [0_u8; BATCH * 8];
    cstrategic_air::visit(memory, base, game_state, |root| {
        let histories = array(memory, root.address, cstrategic_air::HISTORY_TABLE, 8)?;
        let mut first = 0;
        while first < histories.count {
            let count = ((histories.count - first) as usize).min(BATCH);
            let mut overflow = None;
            for (index, slot) in slots[..count].iter_mut().enumerate() {
                match histories.element(first + index as u32, 8) {
                    Some(element) => *slot = element,
                    None => {
                        overflow = Some(index);
                        break;
                    }
                }
            }
            let staged = overflow.unwrap_or(count);
            let copied = memory.copy_records(
                RecordSources::Pointers(&slots[..staged]),
                &SLOT_LAYOUT,
                &mut slot_bytes[..staged * 8],
            );
            for index in 0..count {
                let region = first + index as u32;
                if overflow == Some(index) {
                    return Err("air history region address overflow");
                }
                if index >= copied {
                    return Err("air history field unreadable or address overflow");
                }
                let history = usize::try_from(u64::from_le_bytes(
                    slot_bytes[index * 8..index * 8 + 8].try_into().unwrap(),
                ))
                .map_err(|_| "air history pointer outside address space")?;
                if history == 0 {
                    continue;
                }
                output.ensure(len.checked_add(1).ok_or("air history row count overflow")?)?;
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("air history row count overflow")?;
                    continue;
                }
                output[len] = read_current(memory, base, history, root.country, region)?;
                len += 1;
            }
            first += count as u32;
        }
        Ok(())
    })?;
    Ok((len, unread_rows))
}

fn read_current(
    memory: &impl Memory,
    base: usize,
    history: usize,
    country: u32,
    region: u32,
) -> Result<Row, &'static str> {
    authenticate(memory, base, history, VTABLE_RVA)?;
    if field::<u32>(memory, history, FIELD_COUNT)? != air_stats::FIELD_COUNT {
        return Err("air history field count differs from pinned schema");
    }
    if pointer(memory, history, SELF_REFERENCE)? != history {
        return Err("air history self reference mismatch");
    }
    let window = pointer(memory, history, WINDOW)?;
    authenticate(memory, base, window, loop_history_window::VTABLE_RVA)?;
    let records = array(memory, window, loop_history_window::RECORDS, 8)?;
    let capacity = field::<u32>(memory, window, loop_history_window::CAPACITY)?;
    let year_slot = field::<u32>(memory, window, loop_history_window::CURRENT_SLOT)?;
    let year_window_wrapped = field::<u8>(memory, window, loop_history_window::WRAPPED)?;
    // Existing air histories remain intact. The original signed ring
    // capacity supplies its bound, without a five-slot gate.
    if capacity as i32 <= 0
        || records.count != capacity
        || year_slot >= capacity
        || year_window_wrapped > 1
    {
        return Err("air history yearly window is invalid");
    }
    if pointer(memory, window, loop_history_window::HISTORY)? != history {
        return Err("air history window owner mismatch");
    }
    let record = pointer(memory, window, loop_history_window::RECORD_FIELDS)?;
    let slot = records
        .element(year_slot, 8)
        .ok_or("air history current slot address overflow")?;
    if record == 0 || pointer(memory, slot, 0)? != record {
        return Err("air history current record disagrees with circular slot");
    }
    if pointer(memory, record, loop_history_record_fields::HISTORY)? != history {
        return Err("air history record owner mismatch");
    }
    let values = array(memory, record, loop_history_record_fields::VALUES, 8)?;
    if values.count != air_stats::FIELD_COUNT {
        return Err("air history record field count differs from pinned schema");
    }
    Ok(Row {
        country,
        region,
        year_slot,
        year_window_wrapped,
        channels: read_channels(memory, values)?,
    })
}

/// Nine exact contiguous spans cover the 24 recorded channels. One guarded
/// scatter copy avoids 24 SEH entries while leaving all other channels unread.
fn read_channels(memory: &impl Memory, values: Array) -> Result<[i64; 24], &'static str> {
    let mut head = [0_u8; 32];
    let mut bombers = [0_u8; 8];
    let mut cas = [0_u8; 8];
    let mut transports = [0_u8; 8];
    let mut middle = [0_u8; 72];
    let mut enemy_bombers = [0_u8; 8];
    let mut enemy_cas = [0_u8; 8];
    let mut enemy_transports = [0_u8; 8];
    let mut tail = [0_u8; 40];
    let mut addresses = [0; 9];
    for (address, index) in addresses
        .iter_mut()
        .zip([0, 10, 17, 24, 36, 51, 58, 65, 77])
    {
        *address = values
            .element(index, loop_history_record_fields::VALUE_STRIDE)
            .ok_or("air history channel address overflow")?;
    }
    if memory.copy_scattered(
        addresses,
        &mut [
            &mut head,
            &mut bombers,
            &mut cas,
            &mut transports,
            &mut middle,
            &mut enemy_bombers,
            &mut enemy_cas,
            &mut enemy_transports,
            &mut tail,
        ],
    ) != addresses.len()
    {
        return Err("air history channel unreadable");
    }
    let mut channels = [0; 24];
    let mut fields = channels.iter_mut();
    for span in [
        head.as_slice(),
        &bombers,
        &cas,
        &transports,
        &middle,
        &enemy_bombers,
        &enemy_cas,
        &enemy_transports,
        &tail,
    ] {
        for bytes in span.chunks_exact(8) {
            if let Some(field) = fields.next() {
                *field = i64::from_le_bytes(bytes.try_into().expect("eight-byte channel"));
            }
        }
    }
    Ok(channels)
}

fn authenticate(
    memory: &impl Memory,
    base: usize,
    object: usize,
    rva: usize,
) -> Result<(), &'static str> {
    let expected = base
        .checked_add(rva)
        .ok_or("air history vtable address overflow")?;
    if object == 0 || read::<u64>(memory, object) != Some(expected as u64) {
        return Err("air history object vtable unreadable or mismatched");
    }
    Ok(())
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| read::<T>(memory, address))
        .ok_or("air history field unreadable or address overflow")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "air history pointer outside address space")
}

fn array(
    memory: &impl Memory,
    object: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    object
        .checked_add(offset)
        .and_then(|address| Array::read(memory, address, stride))
        .ok_or("air history array unreadable or invalid")
}

/// Air-statistics semantics attached to a history instance.
pub mod air_stats {
    /// Air-statistics field count 89.
    pub const FIELD_COUNT: u32 = 89;
    /// Non-building air-statistics display divisor.
    pub const DISPLAY_DIVISOR: i64 = 100_000;
    /// Building-bombing display divisor for fields zero and 41.
    pub const BUILDINGS_BOMBED_DISPLAY_DIVISOR: i64 = 1_000;

    /// CLoopHistory air field 0, our buildings bombed.
    pub const OUR_BUILDINGS_BOMBED: usize = 0;
    /// CLoopHistory air field 1, our troops bombed.
    pub const OUR_TROOPS_BOMBED: usize = 1;
    /// CLoopHistory air field 2, our ships bombed.
    pub const OUR_SHIPS_BOMBED: usize = 2;
    /// CLoopHistory air field 3, our fighters lost.
    pub const OUR_FIGHTERS_LOST: usize = 3;
    /// CLoopHistory air field 10, our bombers lost.
    pub const OUR_BOMBERS_LOST: usize = 10;
    /// Air history field 17: friendly close-air-support aircraft lost.
    pub const OUR_CAS_LOST: usize = 17;
    /// CLoopHistory air field 24, our transports lost.
    pub const OUR_TRANSPORTS_LOST: usize = 24;
    /// CLoopHistory air field 36, our trains lost.
    pub const OUR_TRAINS_LOST: usize = 36;
    /// CLoopHistory air field 37, our trucks lost.
    pub const OUR_TRUCKS_LOST: usize = 37;
    /// CLoopHistory air field 38, our railway guns lost.
    pub const OUR_RAILWAY_GUNS_LOST: usize = 38;
    /// CLoopHistory air field 39, our bombers disrupted.
    pub const OUR_BOMBERS_DISRUPTED: usize = 39;
    /// CLoopHistory air field 40, our missiles lost.
    pub const OUR_MISSILES_LOST: usize = 40;
    /// CLoopHistory air field 41, enemy buildings bombed.
    pub const ENEMY_BUILDINGS_BOMBED: usize = 41;
    /// CLoopHistory air field 42, enemy troops bombed.
    pub const ENEMY_TROOPS_BOMBED: usize = 42;
    /// CLoopHistory air field 43, enemy ships sunk.
    pub const ENEMY_SHIPS_SUNK: usize = 43;
    /// CLoopHistory air field 44, enemy fighters lost.
    pub const ENEMY_FIGHTERS_LOST: usize = 44;
    /// CLoopHistory air field 51, enemy bombers lost.
    pub const ENEMY_BOMBERS_LOST: usize = 51;
    /// Air history field 58: enemy close-air-support aircraft lost.
    pub const ENEMY_CAS_LOST: usize = 58;
    /// CLoopHistory air field 65, enemy transports lost.
    pub const ENEMY_TRANSPORTS_LOST: usize = 65;
    /// CLoopHistory air field 77, enemy trains lost.
    pub const ENEMY_TRAINS_LOST: usize = 77;
    /// CLoopHistory air field 78, enemy trucks lost.
    pub const ENEMY_TRUCKS_LOST: usize = 78;
    /// CLoopHistory air field 79, enemy railway guns lost.
    pub const ENEMY_RAILWAY_GUNS_LOST: usize = 79;
    /// CLoopHistory air field 80, enemy bombers disrupted.
    pub const ENEMY_BOMBERS_DISRUPTED: usize = 80;
    /// CLoopHistory air field 81, enemy missiles lost.
    pub const ENEMY_MISSILES_LOST: usize = 81;

    /// Stored air-history channel ordering.
    pub const CHANNELS: [usize; 24] = [
        OUR_BUILDINGS_BOMBED,
        OUR_TROOPS_BOMBED,
        OUR_SHIPS_BOMBED,
        OUR_FIGHTERS_LOST,
        OUR_BOMBERS_LOST,
        OUR_CAS_LOST,
        OUR_TRANSPORTS_LOST,
        OUR_TRAINS_LOST,
        OUR_TRUCKS_LOST,
        OUR_RAILWAY_GUNS_LOST,
        OUR_BOMBERS_DISRUPTED,
        OUR_MISSILES_LOST,
        ENEMY_BUILDINGS_BOMBED,
        ENEMY_TROOPS_BOMBED,
        ENEMY_SHIPS_SUNK,
        ENEMY_FIGHTERS_LOST,
        ENEMY_BOMBERS_LOST,
        ENEMY_CAS_LOST,
        ENEMY_TRANSPORTS_LOST,
        ENEMY_TRAINS_LOST,
        ENEMY_TRUCKS_LOST,
        ENEMY_RAILWAY_GUNS_LOST,
        ENEMY_BOMBERS_DISRUPTED,
        ENEMY_MISSILES_LOST,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ccountry, cgame_state, cstrategic_air_manager};
    use std::collections::BTreeMap;

    const BASE: usize = 0x0001_4000_0000;
    const GAME: usize = 0x10000;
    const COUNTRY: usize = 0x20000;
    const MANAGER: usize = 0x30000;
    const AIR: usize = 0x40000;
    const HISTORY: usize = 0x50000;
    const WINDOW_OBJECT: usize = 0x60000;
    const RECORD: usize = 0x70000;
    const VALUES: usize = 0x80000;

    #[derive(Default)]
    struct Fixture(BTreeMap<usize, u8>);

    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (offset, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + offset)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    impl Fixture {
        fn bytes(&mut self, address: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, &b)| (address + i, b)));
        }

        fn u32(&mut self, address: usize, value: u32) {
            self.bytes(address, &value.to_le_bytes());
        }

        fn pointer(&mut self, address: usize, value: usize) {
            self.bytes(address, &(value as u64).to_le_bytes());
        }

        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.pointer(address, data);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }

        fn new() -> Self {
            let mut f = Self::default();
            f.array(GAME + cgame_state::COUNTRIES, 0x11000, 3);
            f.pointer(0x11000, 0);
            f.pointer(0x11008, COUNTRY);
            f.pointer(0x11010, COUNTRY);
            f.array(GAME + cgame_state::COUNTRY_LINKS, 0x12000, 4);
            for (index, target) in [0, 1, 1, 1].into_iter().enumerate() {
                f.u32(0x12000 + index * 4, target);
            }
            f.pointer(COUNTRY, BASE + ccountry::VTABLE_RVA);
            f.u32(COUNTRY + ccountry::ID, 3);
            f.pointer(GAME + cgame_state::STRATEGIC_AIR_MANAGER, MANAGER);
            f.pointer(MANAGER, BASE + cstrategic_air_manager::VTABLE_RVA);
            f.array(MANAGER + cstrategic_air_manager::COUNTRY_AIR, 0x31000, 3);
            f.pointer(0x31000, 0);
            f.pointer(0x31008, AIR);
            f.pointer(0x31010, 0);
            f.pointer(AIR, BASE + cstrategic_air::VTABLE_RVA);
            f.u32(AIR + cstrategic_air::COUNTRY, 2);
            f.array(AIR + cstrategic_air::HISTORY_TABLE, 0x41000, 24);
            for region in 0..24 {
                f.pointer(0x41000 + region * 8, 0);
            }
            f.pointer(0x41000 + 23 * 8, HISTORY);
            f.pointer(HISTORY, BASE + VTABLE_RVA);
            f.pointer(HISTORY + SELF_REFERENCE, HISTORY);
            f.u32(HISTORY + FIELD_COUNT, air_stats::FIELD_COUNT);
            f.pointer(HISTORY + WINDOW, WINDOW_OBJECT);
            f.pointer(WINDOW_OBJECT, BASE + loop_history_window::VTABLE_RVA);
            f.array(WINDOW_OBJECT + loop_history_window::RECORDS, 0x61000, 5);
            f.u32(WINDOW_OBJECT + loop_history_window::CAPACITY, 5);
            f.u32(WINDOW_OBJECT + loop_history_window::CURRENT_SLOT, 2);
            f.bytes(WINDOW_OBJECT + loop_history_window::WRAPPED, &[1]);
            f.pointer(WINDOW_OBJECT + loop_history_window::HISTORY, HISTORY);
            f.pointer(WINDOW_OBJECT + loop_history_window::RECORD_FIELDS, RECORD);
            f.pointer(0x61000 + 2 * 8, RECORD);
            f.array(RECORD, VALUES, air_stats::FIELD_COUNT);
            f.pointer(RECORD + loop_history_record_fields::HISTORY, HISTORY);
            for channel in 0..air_stats::FIELD_COUNT as usize {
                f.bytes(VALUES + channel * 8, &(-(channel as i64) - 1).to_le_bytes());
            }
            f
        }
    }

    #[test]
    fn annual_bucket_preserves_channel_order_extremes_and_country_aliases() {
        let mut f = Fixture::new();
        f.bytes(VALUES, &i64::MIN.to_le_bytes());
        f.bytes(VALUES + 81 * 8, &i64::MAX.to_le_bytes());
        // An unrecorded channel need not be readable.
        f.0.remove(&(VALUES + 4 * 8));
        let mut output = [Row::default(); 2];
        assert_eq!(walk(&f, BASE, GAME, &mut output), Outcome::Rows { len: 1 });
        let row = output[0];
        assert_eq!(
            (
                row.country,
                row.region,
                row.year_slot,
                row.year_window_wrapped
            ),
            (2, 23, 2, 1)
        );
        for (ordinal, channel) in air_stats::CHANNELS.into_iter().enumerate() {
            let expected = match channel {
                0 => i64::MIN,
                81 => i64::MAX,
                _ => -(channel as i64) - 1,
            };
            assert_eq!(row.channels[ordinal], expected);
        }
    }

    /// The region-indexed slots are gathered per batch. An unreadable slot in
    /// the middle of that batch must keep its own reason and stop the walk
    /// there, rather than skipping to the live slot staged after it.
    #[test]
    fn a_mid_batch_history_slot_fault_keeps_its_reason_and_stops_the_walk() {
        let mut f = Fixture::new();
        f.0.remove(&(0x41000 + 10 * 8 + 3));
        let mut output = [Row::default(); 2];
        assert_eq!(
            walk(&f, BASE, GAME, &mut output),
            Outcome::Unavailable {
                reason: "air history field unreadable or address overflow"
            }
        );
        assert_eq!(output[0], Row::default());
    }

    #[test]
    fn allocated_history_capacity_and_absence_are_distinct_from_read_failure() {
        let mut f = Fixture::new();
        assert_eq!(
            walk(&f, BASE, GAME, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        f.pointer(0x41000 + 23 * 8, 0);
        assert_eq!(walk(&f, BASE, GAME, &mut []), Outcome::Rows { len: 0 });
        f.0.remove(&(0x41000 + 23 * 8));
        assert!(matches!(
            walk(&f, BASE, GAME, &mut []),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn every_allocated_region_is_addressed_without_a_special_region_23() {
        for region in [0_u32, 7, 22, 23] {
            let mut f = Fixture::new();
            f.pointer(0x41000 + 23 * 8, 0);
            f.pointer(0x41000 + region as usize * 8, HISTORY);
            let mut output = [Row::default(); 1];
            assert_eq!(walk(&f, BASE, GAME, &mut output), Outcome::Rows { len: 1 });
            assert_eq!(output[0].region, region);
            assert_eq!(output[0].key(), (2_u64 << 32) | u64::from(region));
        }
    }

    #[test]
    fn circular_identity_and_record_schema_corruption_are_unavailable() {
        let mut output = [Row::default(); 1];
        let corruptions = [
            (
                WINDOW_OBJECT + loop_history_window::CURRENT_SLOT,
                5_u32.to_le_bytes().to_vec(),
            ),
            (WINDOW_OBJECT + loop_history_window::WRAPPED, vec![2]),
            (
                WINDOW_OBJECT + loop_history_window::HISTORY,
                0_u64.to_le_bytes().to_vec(),
            ),
            (0x61000 + 2 * 8, 0_u64.to_le_bytes().to_vec()),
            (
                RECORD + loop_history_record_fields::COUNT,
                88_u32.to_le_bytes().to_vec(),
            ),
            (
                RECORD + loop_history_record_fields::HISTORY,
                0_u64.to_le_bytes().to_vec(),
            ),
        ];
        for (address, bytes) in corruptions {
            let mut f = Fixture::new();
            f.bytes(address, &bytes);
            assert!(matches!(
                walk(&f, BASE, GAME, &mut output),
                Outcome::Unavailable { .. }
            ));
        }
        let mut f = Fixture::new();
        f.0.remove(&(VALUES + 81 * 8 + 7));
        assert!(matches!(
            walk(&f, BASE, GAME, &mut output),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn unexpected_or_unreadable_manager_slots_cannot_hide_histories() {
        let mut output = [Row::default(); 2];
        let mut f = Fixture::new();
        f.pointer(0x31010, AIR);
        assert_eq!(
            walk(&f, BASE, GAME, &mut output),
            Outcome::Unavailable {
                reason: "strategic air owner differs from canonical country slot",
            }
        );
        f.pointer(0x31010, 0);
        f.0.remove(&0x31010);
        assert_eq!(
            walk(&f, BASE, GAME, &mut output),
            Outcome::Unavailable {
                reason: "strategic air country pointer unreadable or address overflow",
            }
        );
    }

    #[test]
    fn growing_air_statistics_follows_changed_region_demand_without_country_growth() {
        let mut memory = Fixture::new();
        let mut output = Vec::new();
        for count in [5_000_u32, 12_000] {
            memory.array(AIR + cstrategic_air::HISTORY_TABLE, 0x100000, count);
            for index in 0..count as usize {
                memory.pointer(0x100000 + index * 8, HISTORY);
            }
            let mut fixed = [Row::default(); 1];
            assert_eq!(
                walk(&memory, BASE, GAME, &mut fixed),
                Outcome::Truncated {
                    len: 1,
                    unread_rows: count - 1
                }
            );
            assert_eq!(
                walk_growing(&memory, BASE, GAME, &mut output),
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(output[count as usize - 1].region, count - 1);
        }
    }

    #[test]
    fn current_air_statistics_uses_original_yearly_capacity_and_slot() {
        let mut output = Vec::new();
        for count in [6_u32, 97, 5_000, 12_000] {
            let mut memory = Fixture::new();
            memory.array(
                WINDOW_OBJECT + loop_history_window::RECORDS,
                0x100000,
                count,
            );
            memory.u32(WINDOW_OBJECT + loop_history_window::CAPACITY, count);
            memory.u32(WINDOW_OBJECT + loop_history_window::CURRENT_SLOT, count - 1);
            memory.pointer(0x100000 + (count as usize - 1) * 8, RECORD);
            assert_eq!(
                walk_growing(&memory, BASE, GAME, &mut output),
                Outcome::Rows { len: 1 }
            );
            assert_eq!(
                (output[0].year_slot, output[0].year_window_wrapped),
                (count - 1, 1)
            );
            assert_eq!(
                output[0].channels,
                air_stats::CHANNELS.map(|channel| -(channel as i64) - 1)
            );
            let mut fixed = [Row::default(); 1];
            assert_eq!(
                walk(&memory, BASE, GAME, &mut fixed),
                Outcome::Rows { len: 1 }
            );
            assert_eq!(fixed, output[..1]);
        }
        for count in [0, i32::MAX as u32 + 1, u32::MAX] {
            let mut memory = Fixture::new();
            memory.u32(WINDOW_OBJECT + loop_history_window::CAPACITY, count);
            assert!(matches!(
                walk_growing(&memory, BASE, GAME, &mut output),
                Outcome::Unavailable { .. }
            ));
        }
    }
}
