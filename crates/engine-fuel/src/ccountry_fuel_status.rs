//! Country fuel state. Quantities use signed i64 fixed15; gain and cost
//! modifiers use fixed5 and are converted before changing fuel. Serialization
//! retains scalars, consumers, history cursor, pending entry and history ring.
//! Capture records scalars, cursor, consumers and ring sums, excluding pending
//! history. Element layouts are in fuel_consumer_data and fuel_history_entry.
//!
//! The tables share one guarded scatter of scalars, headers, cursor and owner,
//! then another of consumers and history. Synthetic coverage includes truncation,
//! unreadable rings, overflow, invalid headers and owner mismatch. Ring sums
//! remain unverified live.

use crate::{
    Memory, ccountry, cpdx_array::Array, fuel_consumer_data as consumer,
    fuel_history_entry as entry, read, read_slice, walk::Outcome,
};

/// Fuel-state primary vtable.
pub const VTABLE_RVA: usize = 0x0298_b3a8;

/// Current fuel, i64 fixed15.
pub const CURRENT: usize = 0x08;
/// Maximum fuel, i64 fixed15.
pub const MAXIMUM: usize = 0x10;
/// Base gain, i64 fixed5.
pub const GAIN: usize = 0x18;
/// State fuel gain, i64 fixed5, added to base gain.
pub const GAIN_FROM_STATES: usize = 0x20;
/// Lend-lease gain, i64 fixed15, added to current fuel.
pub const LEND_LEASE_GAIN: usize = 0x28;
/// Lend-lease consumption, i64 fixed15, subtracted from current fuel.
pub const LEND_LEASE_CONSUMPTION: usize = 0x30;
/// Flat fuel cost, i64 fixed5, converted to fixed15 before subtraction.
pub const COST: usize = 0x38;
/// Fuel gain per oil, i64 fixed5. Oil-demand division substitutes the
/// unsigned u32 sentinel when this divisor is zero.
pub const GAIN_PER_OIL: usize = 0x40;
/// Consumer array header. Serialization retains every counted entry and
/// its index; the allocator follows the header.
pub const CONSUMERS: usize = 0x48;
/// Constructed consumer count. Simulation indexes its three entries
/// unconditionally; loading stores by index without resizing.
pub const MAX_CONSUMERS: u32 = 3;
/// Pending history entry, excluded from this reader. Hourly ring writes
/// clear it; serialization and loading identify it with index -1.
pub const PENDING_HISTORY: usize = 0x60;
/// History-ring header. Construction sizes the fixed ring, loading bounds
/// indices by count, and serialization reads exactly the constructed extent
/// from its data pointer. Capture refuses any other ring header.
pub const HISTORY_RING: usize = 0xb0;
/// Constructed history-ring extent, independent of capture capacity.
pub const HISTORY_RING_ENTRIES: u32 = 24;
/// Next ring slot, i32, advanced modulo the ring extent after copying pending history.
pub const HISTORY_INDEX: usize = 0xc8;
/// Hourly countdown, i32, resetting to one after writing the ring.
pub const REMAINING_HOURS: usize = 0xcc;
/// Owning CCountry pointer checked against the caller's country.
pub const COUNTRY: usize = 0xe0;
/// Fuel quantity denominator.
pub const QUANTITY_SCALE: i64 = 32_768;
/// Fuel gain and cost modifier denominator.
pub const MODIFIER_SCALE: i64 = 100_000;

/// One country's fuel status. The `ring_` fields are exact sums of the raw
/// values of the 24 history ring entries, which the game writes one per
/// hourly fuel update: they cover the 24 updates before the read, whatever
/// its hour, not a calendar day.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    /// Raw surrender target, optional without withholding the prefix.
    pub country_instance: Option<u32>,
    pub current_raw: i64,
    pub maximum_raw: i64,
    pub lend_lease_gain_raw: i64,
    /// Sum of the ring entries' fuel gain, fixed5.
    pub ring_gain_raw: i64,
    /// Sum of the ring entries' consumed fuel, /32768.
    pub ring_consumed_raw: i64,
    /// Sum of the ring entries' other fuel, /32768.
    pub ring_other_raw: i64,
    /// Per-consumer sums of the ring entries' received fuel, fixed5.
    pub ring_received_raw: [i64; entry::CONSUMER_VALUES],
}

/// One consumer entry; `index` is its array position, which the save writes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerRow {
    pub country: u32,
    pub index: u32,
    pub priority: i32,
    pub requested_raw: i64,
    pub received_raw: i64,
}

/// Per-table outcomes of one fuel walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outcomes {
    pub fuel: Outcome,
    pub consumers: Outcome,
}

impl Outcomes {
    pub fn unavailable(reason: &'static str) -> Self {
        Self {
            fuel: Outcome::Unavailable { reason },
            consumers: Outcome::Unavailable { reason },
        }
    }
}

/// Reads each authenticated canonical country's fuel status once for both
/// tables, retaining its raw country reference. Dormant countries can retain
/// the native -32768 initial quantity; it is preserved, never mistaken for a
/// failed read or zero fuel.
///
/// A status that cannot be read or authenticated makes both tables
/// unavailable. The history ring feeds only the fuel rows: an invalid or
/// unreadable ring, or a ring sum that overflows i64, withholds the fuel table
/// and leaves the consumers. A consumer array failure withholds only the
/// consumers. A failed table keeps its first failure, even when a later
/// failure stops the walk, and reads nothing more. A
/// country's rows that do not fit are counted unread and their entries are not
/// read; its consumers are stored all or none.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [Row],
    consumers: &mut [ConsumerRow],
) -> Outcomes {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(rows),
        crate::walk::Buffer::Fixed(consumers),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    consumers: &mut Vec<ConsumerRow>,
) -> Outcomes {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(rows),
        crate::walk::Buffer::Growing(consumers),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: crate::walk::Buffer<'_, Row>,
    consumers: crate::walk::Buffer<'_, ConsumerRow>,
) -> Outcomes {
    let mut rows = Sink::new(rows);
    let mut consumers = Sink::new(consumers);
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let vtable = base
            .checked_add(VTABLE_RVA)
            .ok_or("fuel module address overflow")?;
        let mut found = false;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found = true;
            let status = read_status(memory, vtable, country)?;
            let ring = rows.admit(status.ring.map(|_| 1)).and(status.ring.ok());
            let consumer_array = consumers
                .admit(status.consumers.map(|array| array.count as usize))
                .and(status.consumers.ok());
            let mut consumer_bytes = [0; MAX_CONSUMERS as usize * consumer::STRIDE];
            let mut ring_bytes = [0; HISTORY_RING_ENTRIES as usize * entry::STRIDE];
            let (consumers_read, ring_read) = read_entries(
                memory,
                consumer_array.map(|array| {
                    (
                        array.data,
                        &mut consumer_bytes[..array.count as usize * consumer::STRIDE],
                    )
                }),
                ring.map(|ring| (ring.data, &mut ring_bytes[..])),
            );
            if ring.is_some() {
                let row = if ring_read {
                    ring_sums(status.row, &ring_bytes)
                } else {
                    Err("fuel history ring unreadable")
                };
                match row {
                    Ok(row) => rows.push(row),
                    Err(reason) => rows.fail(reason),
                }
            }
            if let Some(array) = consumer_array {
                if !consumers_read {
                    consumers.fail("fuel consumer entries unreadable");
                } else {
                    let entries = consumer_bytes[..array.count as usize * consumer::STRIDE]
                        .chunks_exact(consumer::STRIDE);
                    for (index, bytes) in (0..).zip(entries) {
                        consumers.push(ConsumerRow {
                            country: country.id,
                            index,
                            priority: i32_at(bytes, consumer::PRIORITY),
                            requested_raw: i64_at(bytes, consumer::REQUESTED),
                            received_raw: i64_at(bytes, consumer::RECEIVED),
                        });
                    }
                }
            }
        }
        if found {
            Ok(())
        } else {
            Err("fuel country table has no countries")
        }
    })();
    // A failure that stops the walk withholds both tables, but a table that
    // already failed keeps its own first failure.
    if let Err(reason) = result {
        rows.fail(reason);
        consumers.fail(reason);
    }
    Outcomes {
        fuel: rows.finish(),
        consumers: consumers.finish(),
    }
}

/// One status's owned scalars and its validated array headers, from one
/// guarded scatter.
struct Status {
    row: Row,
    consumers: Result<Array, &'static str>,
    ring: Result<Array, &'static str>,
}

fn read_status(
    memory: &impl Memory,
    vtable: usize,
    country: ccountry::Country,
) -> Result<Status, &'static str> {
    let status = country
        .pointer
        .checked_add(ccountry::FUEL_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("fuel status pointer unreadable or null")?;
    // These spans contain only the vtable and requested fields: the scalars
    // with the consumer header, the ring header and the owner; the pending
    // entry, the allocator words and the retired history cursor are not
    // read. All buffers are stack-owned, and one guarded scatter avoids
    // per-field SEH calls.
    let mut head = [0; CONSUMERS + 16];
    let mut ring = [0; 16];
    let mut owner = [0; 8];
    let copied = memory.copy_scattered(
        [
            status,
            status.checked_add(HISTORY_RING).unwrap_or(0),
            status.checked_add(COUNTRY).unwrap_or(0),
        ],
        &mut [&mut head, &mut ring, &mut owner],
    );
    if copied == 0 {
        return Err("fuel status quantities unreadable");
    }
    if u64::from_le_bytes(head[..8].try_into().unwrap()) != vtable as u64 {
        return Err("fuel status vtable mismatch");
    }
    if copied < 2 {
        return Err("fuel status history ring header unreadable");
    }
    if copied < 3 {
        return Err("fuel status country pointer unreadable");
    }
    if u64::from_le_bytes(owner) != country.pointer as u64 {
        return Err("fuel status country mismatch");
    }
    let consumers = Array::from_header(head[CONSUMERS..].try_into().unwrap(), consumer::STRIDE)
        .ok_or("fuel consumer array header invalid")
        .and_then(|array| {
            if array.count <= MAX_CONSUMERS {
                Ok(array)
            } else {
                Err("fuel consumer count exceeds the engine's three consumers")
            }
        });
    let ring = Array::from_header(ring, entry::STRIDE)
        .filter(|ring| ring.count == HISTORY_RING_ENTRIES)
        .ok_or("fuel history ring header is not the constructed 24-entry ring");
    Ok(Status {
        row: Row {
            country: country.id,
            country_instance: country
                .pointer
                .checked_add(ccountry::SURRENDER_TARGET)
                .and_then(|p| read(memory, p)),
            current_raw: i64_at(&head, CURRENT),
            maximum_raw: i64_at(&head, MAXIMUM),
            lend_lease_gain_raw: i64_at(&head, LEND_LEASE_GAIN),
            ..Row::default()
        },
        consumers,
        ring,
    })
}

/// Copies one country's admitted consumer entries and history ring. When both
/// are wanted they share one guarded scatter; a failed consumer span stops the
/// scatter before the ring, which is then read once on its own.
fn read_entries(
    memory: &impl Memory,
    consumers: Option<(usize, &mut [u8])>,
    ring: Option<(usize, &mut [u8])>,
) -> (bool, bool) {
    match (consumers, ring) {
        (Some((consumers, consumer_bytes)), Some((ring, ring_bytes)))
            if !consumer_bytes.is_empty() =>
        {
            match memory.copy_scattered([consumers, ring], &mut [consumer_bytes, ring_bytes]) {
                0 => (false, read_slice(memory, ring, ring_bytes)),
                1 => (true, false),
                _ => (true, true),
            }
        }
        (consumers, ring) => (
            consumers.is_none_or(|(address, bytes)| read_slice(memory, address, bytes)),
            ring.is_none_or(|(address, bytes)| read_slice(memory, address, bytes)),
        ),
    }
}

/// The row with the exact sums of the ring entries' raw values. Each sum is
/// accumulated in i128, which 24 i64 values cannot overflow, so only the final
/// sum is checked: one outside i64 is refused rather than wrapped or saturated,
/// while an in-range sum whose partial sums leave i64 is kept.
fn ring_sums(mut row: Row, ring: &[u8]) -> Result<Row, &'static str> {
    let offsets = [
        entry::GAIN,
        entry::CONSUMED,
        entry::OTHER,
        entry::RECEIVED,
        entry::RECEIVED + 8,
        entry::RECEIVED + 16,
    ];
    let mut sums = [0_i128; 6];
    for bytes in ring.chunks_exact(entry::STRIDE) {
        for (sum, offset) in sums.iter_mut().zip(offsets) {
            *sum += i128::from(i64_at(bytes, offset));
        }
    }
    let [gain, consumed, other, received_0, received_1, received_2] = sums.map(i64::try_from);
    let overflow = |_| "fuel history ring sum exceeds i64";
    row.ring_gain_raw = gain.map_err(overflow)?;
    row.ring_consumed_raw = consumed.map_err(overflow)?;
    row.ring_other_raw = other.map_err(overflow)?;
    row.ring_received_raw = [
        received_0.map_err(overflow)?,
        received_1.map_err(overflow)?,
        received_2.map_err(overflow)?,
    ];
    Ok(row)
}

fn i64_at(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
}

fn i32_at(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

/// One table's rows, unread demand and first failure.
struct Sink<'a, R> {
    rows: crate::walk::Buffer<'a, R>,
    len: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl<'a, R: Default> Sink<'a, R> {
    fn new(rows: crate::walk::Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread_rows: 0,
            failure: None,
        }
    }

    /// Admits one country's rows: `None` when the table has failed, when
    /// `count` is an error (the table fails with it) or when the rows do not
    /// all fit (they are counted unread).
    fn admit(&mut self, count: Result<usize, &'static str>) -> Option<usize> {
        if self.failure.is_some() {
            return None;
        }
        if let Ok(count) = count {
            let growth = self
                .len
                .checked_add(count)
                .ok_or("fuel row count overflow")
                .and_then(|required| self.rows.ensure(required));
            if let Err(reason) = growth {
                self.fail(reason);
                return None;
            }
        }
        match count {
            Err(reason) => self.fail(reason),
            Ok(count) if count <= self.rows.len() - self.len => return Some(count),
            Ok(count) => {
                match u32::try_from(count)
                    .ok()
                    .and_then(|count| self.unread_rows.checked_add(count))
                {
                    Some(total) => self.unread_rows = total,
                    None => self.fail("fuel unread row count exceeds u32"),
                }
            }
        }
        None
    }

    /// Stores an admitted row.
    fn push(&mut self, row: R) {
        self.rows[self.len] = row;
        self.len += 1;
    }

    fn fail(&mut self, reason: &'static str) {
        self.failure.get_or_insert(reason);
    }

    fn finish(self) -> Outcome {
        match (self.failure, self.unread_rows) {
            (Some(reason), _) => Outcome::Unavailable { reason },
            (None, 0) => Outcome::Rows { len: self.len },
            (None, unread_rows) => Outcome::Truncated {
                len: self.len,
                unread_rows,
            },
        }
    }
}

#[cfg(test)]
mod tests;
