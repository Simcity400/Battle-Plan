//! Exact a729 `SDynamicModifierEntry` value/duration slice (A18).
//!
//! The container/definition identity walk remains owned by domain 67. This
//! module decodes only an already-validated 0x40-byte entry, preserving raw
//! fixed-point values and the serialized remaining-day counter. The engine
//! decrements that counter on its daily country tick and removes zero-valued
//! entries; the tick phase is not encoded here, so this slice deliberately
//! does not invent an absolute expiry hour. Any incoherent vector rejects the
//! whole entry.

pub const ENTRY_STRIDE: u64 = 0x40;
pub const ENTRY_DAYS_REMAINING_OFF: u64 = 0x10;
pub const ENTRY_DEF_PTR_OFF: u64 = 0x18;
pub const ENTRY_ENABLED_OFF: u64 = 0x20;
pub const ENTRY_VALUE_BEGIN_OFF: u64 = 0x28;
pub const ENTRY_VALUE_CAPACITY_OFF: u64 = 0x30;
pub const ENTRY_VALUE_COUNT_OFF: u64 = 0x34;
pub const VALUE_STRIDE: u64 = 8;
pub const VALUE_SCALE: i64 = 100_000;
pub const MAX_VALUE_COUNT: u32 = 256;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimedModifierInstance {
    pub enabled: bool,
    pub values_raw: Vec<i64>,
    pub value_scale: i64,
    pub remaining_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Memory(u64),
    AddressOverflow,
    InvalidEnabled(u8),
    InvalidDays(i32),
    InvalidValueHeader { begin: u64, capacity: u32, count: u32 },
    TornHeader,
}

pub trait EntryMemory {
    fn read_u8(&self, address: u64) -> Result<u8, ReadError>;
    fn read_u32(&self, address: u64) -> Result<u32, ReadError>;
    fn read_u64(&self, address: u64) -> Result<u64, ReadError>;
}

fn add(base: u64, offset: u64) -> Result<u64, ReadError> {
    base.checked_add(offset).ok_or(ReadError::AddressOverflow)
}

fn read_header<M: EntryMemory>(memory: &M, entry: u64) -> Result<(u64, u32, u32), ReadError> {
    Ok((
        memory.read_u64(add(entry, ENTRY_VALUE_BEGIN_OFF)?)?,
        memory.read_u32(add(entry, ENTRY_VALUE_CAPACITY_OFF)?)?,
        memory.read_u32(add(entry, ENTRY_VALUE_COUNT_OFF)?)?,
    ))
}

pub fn read_timed_instance<M: EntryMemory>(
    memory: &M,
    entry: u64,
) -> Result<TimedModifierInstance, ReadError> {
    let enabled = match memory.read_u8(add(entry, ENTRY_ENABLED_OFF)?)? {
        0 => false,
        1 => true,
        other => return Err(ReadError::InvalidEnabled(other)),
    };
    let days_raw = memory.read_u32(add(entry, ENTRY_DAYS_REMAINING_OFF)?)? as i32;
    let remaining_days = match days_raw {
        -1 => None,
        value if value >= 0 => Some(value as u32),
        value => return Err(ReadError::InvalidDays(value)),
    };

    let header0 = read_header(memory, entry)?;
    let (begin, capacity, count) = header0;
    if count > capacity || count > MAX_VALUE_COUNT || (count != 0 && begin == 0) {
        return Err(ReadError::InvalidValueHeader { begin, capacity, count });
    }
    let mut values_raw = Vec::with_capacity(count as usize);
    for index in 0..count as u64 {
        let address = begin
            .checked_add(index.checked_mul(VALUE_STRIDE).ok_or(ReadError::AddressOverflow)?)
            .ok_or(ReadError::AddressOverflow)?;
        let value0 = memory.read_u64(address)? as i64;
        let value1 = memory.read_u64(address)? as i64;
        if value0 != value1 {
            return Err(ReadError::TornHeader);
        }
        values_raw.push(value0);
    }
    if read_header(memory, entry)? != header0 {
        return Err(ReadError::TornHeader);
    }
    Ok(TimedModifierInstance {
        enabled,
        values_raw,
        value_scale: VALUE_SCALE,
        remaining_days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Mem(BTreeMap<u64, u64>);
    impl EntryMemory for Mem {
        fn read_u8(&self, address: u64) -> Result<u8, ReadError> {
            self.0.get(&address).copied().map(|v| v as u8).ok_or(ReadError::Memory(address))
        }
        fn read_u32(&self, address: u64) -> Result<u32, ReadError> {
            self.0.get(&address).copied().map(|v| v as u32).ok_or(ReadError::Memory(address))
        }
        fn read_u64(&self, address: u64) -> Result<u64, ReadError> {
            self.0.get(&address).copied().ok_or(ReadError::Memory(address))
        }
    }

    fn fixture(days: i32) -> (Mem, u64) {
        let entry = 0x2000_0000;
        let values = 0x3000_0000;
        let mut mem = Mem::default();
        mem.0.insert(entry + ENTRY_ENABLED_OFF, 1);
        mem.0.insert(entry + ENTRY_DAYS_REMAINING_OFF, days as u32 as u64);
        mem.0.insert(entry + ENTRY_VALUE_BEGIN_OFF, values);
        mem.0.insert(entry + ENTRY_VALUE_CAPACITY_OFF, 4);
        mem.0.insert(entry + ENTRY_VALUE_COUNT_OFF, 3);
        mem.0.insert(values, 15_000);
        mem.0.insert(values + 8, 10_000);
        mem.0.insert(values + 16, 0);
        (mem, entry)
    }

    #[test]
    fn timed_entry_preserves_raw_values_and_remaining_daily_ticks() {
        let (mem, entry) = fixture(716);
        let row = read_timed_instance(&mem, entry).unwrap();
        assert_eq!(row.values_raw, vec![15_000, 10_000, 0]);
        assert_eq!(row.value_scale, 100_000);
        assert_eq!(row.remaining_days, Some(716));
    }

    #[test]
    fn permanent_entry_uses_minus_one_sentinel() {
        let (mem, entry) = fixture(-1);
        let row = read_timed_instance(&mem, entry).unwrap();
        assert_eq!(row.remaining_days, None);
    }

    #[test]
    fn impossible_count_fails_closed() {
        let (mut mem, entry) = fixture(1);
        mem.0.insert(entry + ENTRY_VALUE_COUNT_OFF, 257);
        assert!(matches!(read_timed_instance(&mem, entry), Err(ReadError::InvalidValueHeader { .. })));
    }

    #[test]
    fn negative_non_sentinel_duration_fails_closed() {
        let (mem, entry) = fixture(-2);
        assert_eq!(read_timed_instance(&mem, entry), Err(ReadError::InvalidDays(-2)));
    }
}
