
use engine_native_memory::{Memory, cpdx_array::Array, read, read_slice, walk::Outcome};
use engine_native::ccountry;
use crate::{fuel_consumer_data as consumer, fuel_history_entry as entry};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a0_7068);

pub const CURRENT: usize = 0x08;
pub const MAXIMUM: usize = 0x10;
pub const GAIN: usize = 0x18;
pub const GAIN_FROM_STATES: usize = 0x20;
pub const LEND_LEASE_GAIN: usize = 0x28;
pub const LEND_LEASE_CONSUMPTION: usize = 0x30;
pub const COST: usize = 0x38;
pub const GAIN_PER_OIL: usize = 0x40;
pub const CONSUMERS: usize = 0x48;
pub const MAX_CONSUMERS: u32 = 3;
pub const PENDING_HISTORY: usize = 0x60;
pub const HISTORY_RING: usize = 0xb0;
pub const HISTORY_RING_ENTRIES: u32 = 24;
pub const HISTORY_INDEX: usize = 0xc8;
pub const REMAINING_HOURS: usize = 0xcc;
pub const COUNTRY: usize = 0xe0;
pub use engine_executable::native_values::fuel_status::QUANTITY_SCALE;
pub use engine_executable::native_values::fuel_status::MODIFIER_SCALE;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub country_instance: Option<u32>,
    pub current_raw: i64,
    pub maximum_raw: i64,
    pub lend_lease_gain_raw: i64,
    pub ring_gain_raw: i64,
    pub ring_consumed_raw: i64,
    pub ring_other_raw: i64,
    pub ring_received_raw: [i64; entry::CONSUMER_VALUES],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerRow {
    pub country: u32,
    pub index: u32,
    pub priority: i32,
    pub requested_raw: i64,
    pub received_raw: i64,
}

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

pub fn walk(
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
        engine_native_memory::walk::Buffer::Growing(rows),
        engine_native_memory::walk::Buffer::Growing(consumers),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: engine_native_memory::walk::Buffer<'_, Row>,
    consumers: engine_native_memory::walk::Buffer<'_, ConsumerRow>,
) -> Outcomes {
    let mut rows = Sink::new(rows);
    let mut consumers = Sink::new(consumers);
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let vtable = engine_sites::at(base, VTABLE_RVA).ok_or("fuel module address overflow")?;
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
    if let Err(reason) = result {
        rows.fail(reason);
        consumers.fail(reason);
    }
    Outcomes {
        fuel: rows.finish(),
        consumers: consumers.finish(),
    }
}

struct Status {
    row: Row,
    consumers: Result<Array, &'static str>,
    ring: Result<Array, &'static str>,
}

fn read_status(
    memory: &impl Memory,
    vtable: usize,
    country: engine_native_memory::country_identity::Country,
) -> Result<Status, &'static str> {
    let status = country
        .pointer
        .checked_add(ccountry::FUEL_STATUS)
        .and_then(|p| read::<u64>(memory, p))
        .and_then(|p| usize::try_from(p).ok())
        .filter(|&p| p != 0)
        .ok_or("fuel status pointer unreadable or null")?;
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

struct Sink<'a, R> {
    rows: engine_native_memory::walk::Buffer<'a, R>,
    len: usize,
    unread_rows: u32,
    failure: Option<&'static str>,
}

impl<'a, R: Default> Sink<'a, R> {
    fn new(rows: engine_native_memory::walk::Buffer<'a, R>) -> Self {
        Self {
            rows,
            len: 0,
            unread_rows: 0,
            failure: None,
        }
    }

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
