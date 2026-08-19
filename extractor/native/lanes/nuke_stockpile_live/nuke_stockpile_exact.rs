pub const COUNTRY_NUKE_ARRAY_OFF: u64 = 0x1310;
pub const COUNTRY_NUKE_CAPACITY_OFF: u64 = 0x1318;
pub const COUNTRY_NUKE_COUNT_OFF: u64 = 0x131c;
pub const NUKE_STRIDE: u64 = 0x48;
pub const NUKE_OWNER_OFF: u64 = 0x10;
pub const NUKE_AMOUNT_RAW_OFF: u64 = 0x18;
pub const NUKE_TYPE_OFF: u64 = 0x38;
pub const NUKE_READY_OFF: u64 = 0x3c;
pub const NUKE_VTABLE_RVA: u64 = 0x27d0da0;
pub const NUKE_TYPE_COUNT: i32 = 2;
pub const MAX_NUKE_TYPE_ROWS: i32 = NUKE_TYPE_COUNT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NukeEntrySnapshot {
    pub nuke_type_raw: u32,
    pub amount_raw: i64,
    pub nukes_ready: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountryNukeSnapshot {
    pub container_capacity: i32,
    pub entries: Vec<NukeEntrySnapshot>,
}

impl CountryNukeSnapshot {
    pub fn built_total(&self) -> i32 {
        self.entries.iter().fold(0i32, |total, entry| {
            total.wrapping_add(built_count_from_raw(entry.amount_raw))
        })
    }

    pub fn ready_total(&self) -> u64 {
        self.entries.iter().fold(0u64, |total, entry| {
            total.saturating_add(u64::from(entry.nukes_ready))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Memory(u64),
    InvalidCountry,
    InvalidCapacity(i32),
    InvalidCount(i32),
    InvalidArray(u64),
    AddressOverflow,
    EntryVtable {
        index: i32,
        address: u64,
        actual: u64,
        expected: u64,
    },
    EntryOwner {
        index: i32,
        address: u64,
        actual: u64,
        expected: u64,
    },
    EntryType {
        index: i32,
        address: u64,
        actual: u32,
        expected: u32,
    },
    TornField {
        address: u64,
    },
}

pub trait NukeMemory {
    fn read_u32(&self, address: u64) -> Result<u32, ReadError>;
    fn read_u64(&self, address: u64) -> Result<u64, ReadError>;
}

fn add(base: u64, offset: u64) -> Result<u64, ReadError> {
    base.checked_add(offset).ok_or(ReadError::AddressOverflow)
}

fn read_container_header<M: NukeMemory>(
    memory: &M,
    country: u64,
) -> Result<(u64, i32, i32), ReadError> {
    let begin_addr = add(country, COUNTRY_NUKE_ARRAY_OFF)?;
    let capacity_count_addr = add(country, COUNTRY_NUKE_CAPACITY_OFF)?;
    let begin0 = memory.read_u64(begin_addr)?;
    let capacity_count0 = memory.read_u64(capacity_count_addr)?;
    let begin1 = memory.read_u64(begin_addr)?;
    let capacity_count1 = memory.read_u64(capacity_count_addr)?;
    if begin0 != begin1 || capacity_count0 != capacity_count1 {
        return Err(ReadError::TornField {
            address: if begin0 != begin1 {
                begin_addr
            } else {
                capacity_count_addr
            },
        });
    }
    let capacity = capacity_count0 as u32 as i32;
    let count = (capacity_count0 >> 32) as u32 as i32;
    Ok((begin0, capacity, count))
}

pub fn built_count_from_raw(raw: i64) -> i32 {
    let condition = (raw as u64).wrapping_add(0x7fff_ffff_ffff_fffe) < 0xffff_ffff_ffff_fffd;
    if condition {
        raw.wrapping_mul(100_000).wrapping_div(10_000_000_000_000) as i32
    } else {
        0xa7c5
    }
}

pub fn read_country_nukes<M: NukeMemory>(
    memory: &M,
    country: u64,
    module_base: u64,
) -> Result<CountryNukeSnapshot, ReadError> {
    if country == 0 || module_base == 0 {
        return Err(ReadError::InvalidCountry);
    }
    let (begin0, capacity0, count0) = read_container_header(memory, country)?;
    if capacity0 < NUKE_TYPE_COUNT {
        return Err(ReadError::InvalidCapacity(capacity0));
    }
    if count0 != NUKE_TYPE_COUNT {
        return Err(ReadError::InvalidCount(count0));
    }
    if begin0 == 0 {
        return Err(ReadError::InvalidArray(begin0));
    }
    let expected_vtable = add(module_base, NUKE_VTABLE_RVA)?;
    let mut entries = Vec::with_capacity(count0 as usize);
    for index in 0..count0 {
        let entry = begin0
            .checked_add(
                (index as u64)
                    .checked_mul(NUKE_STRIDE)
                    .ok_or(ReadError::AddressOverflow)?,
            )
            .ok_or(ReadError::AddressOverflow)?;
        let actual_vtable = memory.read_u64(entry)?;
        if actual_vtable != expected_vtable {
            return Err(ReadError::EntryVtable {
                index,
                address: entry,
                actual: actual_vtable,
                expected: expected_vtable,
            });
        }
        let owner = memory.read_u64(add(entry, NUKE_OWNER_OFF)?)?;
        if owner != country {
            return Err(ReadError::EntryOwner {
                index,
                address: add(entry, NUKE_OWNER_OFF)?,
                actual: owner,
                expected: country,
            });
        }
        let type_addr = add(entry, NUKE_TYPE_OFF)?;
        let type0 = memory.read_u32(type_addr)?;
        let type1 = memory.read_u32(type_addr)?;
        if type0 != type1 {
            return Err(ReadError::TornField { address: type_addr });
        }
        let expected_type = index as u32;
        if type0 != expected_type {
            return Err(ReadError::EntryType {
                index,
                address: type_addr,
                actual: type0,
                expected: expected_type,
            });
        }
        let amount_addr = add(entry, NUKE_AMOUNT_RAW_OFF)?;
        let raw0 = memory.read_u64(amount_addr)? as i64;
        let raw1 = memory.read_u64(amount_addr)? as i64;
        if raw0 != raw1 {
            return Err(ReadError::TornField {
                address: amount_addr,
            });
        }
        let ready_addr = add(entry, NUKE_READY_OFF)?;
        let ready0 = memory.read_u32(ready_addr)?;
        let ready1 = memory.read_u32(ready_addr)?;
        if ready0 != ready1 {
            return Err(ReadError::TornField {
                address: ready_addr,
            });
        }
        if memory.read_u64(entry)? != actual_vtable
            || memory.read_u64(add(entry, NUKE_OWNER_OFF)?)? != owner
            || memory.read_u32(type_addr)? != type0
        {
            return Err(ReadError::TornField { address: entry });
        }
        entries.push(NukeEntrySnapshot {
            nuke_type_raw: type0,
            amount_raw: raw0,
            nukes_ready: ready0,
        });
    }
    if read_container_header(memory, country)? != (begin0, capacity0, count0) {
        return Err(ReadError::TornField {
            address: add(country, COUNTRY_NUKE_ARRAY_OFF)?,
        });
    }
    Ok(CountryNukeSnapshot {
        container_capacity: capacity0,
        entries,
    })
}

pub fn read_country_built_nukes<M: NukeMemory>(
    memory: &M,
    country: u64,
    module_base: u64,
) -> Result<i32, ReadError> {
    Ok(read_country_nukes(memory, country, module_base)?.built_total())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Mem(BTreeMap<u64, u64>);
    impl NukeMemory for Mem {
        fn read_u32(&self, address: u64) -> Result<u32, ReadError> {
            self.0
                .get(&address)
                .copied()
                .map(|v| v as u32)
                .ok_or(ReadError::Memory(address))
        }
        fn read_u64(&self, address: u64) -> Result<u64, ReadError> {
            self.0
                .get(&address)
                .copied()
                .ok_or(ReadError::Memory(address))
        }
    }

    fn put_header(mem: &mut Mem, country: u64, begin: u64, capacity: i32, count: i32) {
        mem.0.insert(country + COUNTRY_NUKE_ARRAY_OFF, begin);
        mem.0.insert(
            country + COUNTRY_NUKE_CAPACITY_OFF,
            u64::from(capacity as u32) | (u64::from(count as u32) << 32),
        );
    }

    #[test]
    fn finite_fixed_point_leaf_matches_engine_count() {
        assert_eq!(built_count_from_raw(0), 0);
        assert_eq!(built_count_from_raw(8_000), 0);
        assert_eq!(built_count_from_raw(99_999_999), 0);
        assert_eq!(built_count_from_raw(100_000_000), 1);
        assert_eq!(built_count_from_raw(250_000_000), 2);
    }

    #[test]
    fn engine_sentinel_pair_is_preserved() {
        assert_eq!(built_count_from_raw(i64::MIN), 0xa7c5);
        assert_eq!(built_count_from_raw(i64::MIN + 1), 0xa7c5);
    }

    #[test]
    fn coherent_two_type_array_sums_exact_built_count() {
        let country = 0x2000_0000;
        let module = 0x1400_0000_0;
        let begin = 0x3000_0000;
        let vtable = module + NUKE_VTABLE_RVA;
        let mut mem = Mem::default();
        put_header(&mut mem, country, begin, 2, 2);
        for (index, raw) in [250_000_000u64, 100_000_000].into_iter().enumerate() {
            let entry = begin + index as u64 * NUKE_STRIDE;
            mem.0.insert(entry, vtable);
            mem.0.insert(entry + NUKE_OWNER_OFF, country);
            mem.0.insert(entry + NUKE_AMOUNT_RAW_OFF, raw);
            mem.0.insert(entry + NUKE_TYPE_OFF, index as u64);
            mem.0.insert(entry + NUKE_READY_OFF, index as u64 + 1);
        }
        assert_eq!(read_country_built_nukes(&mem, country, module), Ok(3));
        let snapshot = read_country_nukes(&mem, country, module).unwrap();
        assert_eq!(snapshot.ready_total(), 3);
        assert_eq!(snapshot.container_capacity, 2);
        assert_eq!(snapshot.entries[1].nuke_type_raw, 1);
        assert_eq!(snapshot.entries[0].amount_raw, 250_000_000);
    }

    #[test]
    fn wrong_owner_fails_closed() {
        let country = 0x2000_0000;
        let module = 0x1400_0000_0;
        let begin = 0x3000_0000;
        let mut mem = Mem::default();
        put_header(&mut mem, country, begin, 2, 2);
        mem.0.insert(begin, module + NUKE_VTABLE_RVA);
        mem.0.insert(begin + NUKE_OWNER_OFF, 0xdead);
        mem.0.insert(begin + NUKE_AMOUNT_RAW_OFF, 100_000_000);
        mem.0.insert(begin + NUKE_TYPE_OFF, 0);
        mem.0.insert(begin + NUKE_READY_OFF, 0);
        assert!(matches!(
            read_country_built_nukes(&mem, country, module),
            Err(ReadError::EntryOwner { .. })
        ));
    }

    #[test]
    fn constructor_invariants_reject_torn_empty_and_wrong_type() {
        let country = 0x2000_0000;
        let module = 0x1400_0000_0;
        let begin = 0x3000_0000;
        let mut mem = Mem::default();
        put_header(&mut mem, country, begin, 2, 0);
        assert_eq!(
            read_country_nukes(&mem, country, module),
            Err(ReadError::InvalidCount(0))
        );

        put_header(&mut mem, country, begin, 1, 2);
        assert_eq!(
            read_country_nukes(&mem, country, module),
            Err(ReadError::InvalidCapacity(1))
        );

        put_header(&mut mem, country, begin, 2, 2);
        for index in 0..2u64 {
            let entry = begin + index * NUKE_STRIDE;
            mem.0.insert(entry, module + NUKE_VTABLE_RVA);
            mem.0.insert(entry + NUKE_OWNER_OFF, country);
            mem.0.insert(entry + NUKE_AMOUNT_RAW_OFF, 0);
            mem.0.insert(entry + NUKE_TYPE_OFF, 1);
            mem.0.insert(entry + NUKE_READY_OFF, 0);
        }
        assert!(matches!(
            read_country_nukes(&mem, country, module),
            Err(ReadError::EntryType {
                index: 0,
                actual: 1,
                expected: 0,
                ..
            })
        ));
    }

    struct TornContainerHeader {
        country: u64,
        begin: u64,
        capacity_count_reads: Cell<u32>,
    }

    impl NukeMemory for TornContainerHeader {
        fn read_u32(&self, address: u64) -> Result<u32, ReadError> {
            Err(ReadError::Memory(address))
        }

        fn read_u64(&self, address: u64) -> Result<u64, ReadError> {
            if address == self.country + COUNTRY_NUKE_ARRAY_OFF {
                return Ok(self.begin);
            }
            if address == self.country + COUNTRY_NUKE_CAPACITY_OFF {
                let read = self.capacity_count_reads.get();
                self.capacity_count_reads.set(read + 1);
                return Ok(if read == 0 {
                    2u64 | (2u64 << 32)
                } else {
                    3u64 | (2u64 << 32)
                });
            }
            Err(ReadError::Memory(address))
        }
    }

    #[test]
    fn capacity_count_pair_must_be_double_stable_before_row_walk() {
        let country = 0x2000_0000;
        let memory = TornContainerHeader {
            country,
            begin: 0x3000_0000,
            capacity_count_reads: Cell::new(0),
        };
        assert!(matches!(
            read_country_nukes(&memory, country, 0x1400_0000_0),
            Err(ReadError::TornField { .. })
        ));
    }

    #[test]
    fn retained_a729_positive_amount_is_progress_not_a_built_nuke() {
        let retained = NukeEntrySnapshot {
            nuke_type_raw: 0,
            amount_raw: 8_000,
            nukes_ready: 0,
        };
        assert_eq!(built_count_from_raw(retained.amount_raw), 0);
    }
}
