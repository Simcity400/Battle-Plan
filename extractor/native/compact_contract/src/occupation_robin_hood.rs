use crate::build_profile as bp;

pub trait RobinHoodMemory {
    fn read_u8(&self, address: u64) -> Option<u8>;
    fn read_u32(&self, address: u64) -> Option<u32>;
    fn read_u64(&self, address: u64) -> Option<u64>;

    fn read_u64_stable(&self, address: u64) -> Option<u64> {
        let first = self.read_u64(address)?;
        let second = self.read_u64(address)?;
        if first == second {
            Some(first)
        } else {
            None
        }
    }

    fn is_heap_pointer(&self, value: u64) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RobinHoodFault {
    ReadUnavailable,
    InvalidHeader,
    InvalidPointer,
    AddressOverflow,
    CountMismatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RobinHoodEntry {
    pub key: u32,
    pub value: u64,
    pub bucket: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RobinHoodTable {
    Empty,
    Entries(Vec<RobinHoodEntry>),
}

pub fn read_robin_hood_table<M: RobinHoodMemory + ?Sized>(
    mem: &M,
    header_addr: u64,
) -> Result<RobinHoodTable, (RobinHoodFault, u64)> {
    let Some(size) = header_addr
        .checked_add(bp::DOMAIN66_ROBIN_HOOD_SIZE_OFF as u64)
        .and_then(|addr| mem.read_u32(addr))
    else {
        return Err((RobinHoodFault::ReadUnavailable, header_addr));
    };
    let Some(cap) = header_addr
        .checked_add(bp::DOMAIN66_ROBIN_HOOD_CAP_OFF as u64)
        .and_then(|addr| mem.read_u32(addr))
    else {
        return Err((RobinHoodFault::ReadUnavailable, header_addr));
    };
    let Some(max_distance) = header_addr
        .checked_add(bp::DOMAIN66_ROBIN_HOOD_MAX_DISTANCE_OFF as u64)
        .and_then(|addr| mem.read_u8(addr))
    else {
        return Err((RobinHoodFault::ReadUnavailable, header_addr));
    };
    if size > cap || cap > bp::DOMAIN66_MAX_ROBIN_HOOD_CAP {
        return Err((RobinHoodFault::InvalidHeader, header_addr));
    }
    let Some(bucket_data) = mem.read_u64_stable(header_addr) else {
        return Err((RobinHoodFault::ReadUnavailable, header_addr));
    };
    if bucket_data == 0 || !mem.is_heap_pointer(bucket_data) {
        if size == 0 {
            let stable_zero = header_addr
                .checked_add(bp::DOMAIN66_ROBIN_HOOD_SIZE_OFF as u64)
                .and_then(|addr| mem.read_u32(addr));
            return match stable_zero {
                Some(0) => Ok(RobinHoodTable::Empty),
                Some(_) => Err((RobinHoodFault::CountMismatch, header_addr)),
                None => Err((RobinHoodFault::ReadUnavailable, header_addr)),
            };
        }
        return Err((RobinHoodFault::InvalidPointer, bucket_data));
    }
    let Some(scan_ceiling) = u64::from(cap)
        .checked_add(u64::from(max_distance))
        .and_then(|value| value.checked_add(1))
    else {
        return Err((RobinHoodFault::InvalidHeader, header_addr));
    };
    let mut entries: Vec<RobinHoodEntry> = Vec::new();
    for index in 0..scan_ceiling {
        let Some(bucket) = index
            .checked_mul(bp::DOMAIN66_ROBIN_HOOD_BUCKET_STRIDE as u64)
            .and_then(|off| bucket_data.checked_add(off))
        else {
            return Err((RobinHoodFault::AddressOverflow, bucket_data));
        };
        let Some(distance_plus_1) = bucket
            .checked_add(bp::DOMAIN66_ROBIN_HOOD_DISTANCE_PLUS_1_OFF as u64)
            .and_then(|addr| mem.read_u8(addr))
        else {
            return Err((RobinHoodFault::ReadUnavailable, bucket));
        };
        if distance_plus_1 == 0 {
            continue;
        }
        if distance_plus_1 == bp::DOMAIN66_ROBIN_HOOD_ITERATOR_SENTINEL_DISTANCE {
            return Err((RobinHoodFault::InvalidHeader, bucket));
        }
        let Some(key) = bucket
            .checked_add(bp::DOMAIN66_ROBIN_HOOD_KEY_OFF as u64)
            .and_then(|addr| mem.read_u32(addr))
        else {
            return Err((RobinHoodFault::ReadUnavailable, bucket));
        };
        let Some(value) = bucket
            .checked_add(bp::DOMAIN66_ROBIN_HOOD_VALUE_OFF as u64)
            .and_then(|addr| mem.read_u64(addr))
        else {
            return Err((RobinHoodFault::ReadUnavailable, bucket));
        };
        entries.push(RobinHoodEntry { key, value, bucket });
        if entries.len() as u64 > u64::from(size) {
            return Err((RobinHoodFault::CountMismatch, header_addr));
        }
    }
    if entries.len() as u64 != u64::from(size) {
        return Err((RobinHoodFault::CountMismatch, header_addr));
    }
    if entries.is_empty() {
        return Ok(RobinHoodTable::Empty);
    }
    Ok(RobinHoodTable::Entries(entries))
}

pub fn bucket_still_live<M: RobinHoodMemory + ?Sized>(mem: &M, entry: &RobinHoodEntry) -> bool {
    let Some(distance_plus_1) = entry
        .bucket
        .checked_add(bp::DOMAIN66_ROBIN_HOOD_DISTANCE_PLUS_1_OFF as u64)
        .and_then(|addr| mem.read_u8(addr))
    else {
        return false;
    };
    if distance_plus_1 == 0 || distance_plus_1 == bp::DOMAIN66_ROBIN_HOOD_ITERATOR_SENTINEL_DISTANCE
    {
        return false;
    }
    entry
        .bucket
        .checked_add(bp::DOMAIN66_ROBIN_HOOD_VALUE_OFF as u64)
        .and_then(|addr| mem.read_u64(addr))
        == Some(entry.value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const HEAP_LO: u64 = 0x10000000000;
    const HEAP_HI: u64 = 0x7FF000000000;

    #[derive(Default)]
    struct FakeMem {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMem {
        fn write(&mut self, address: u64, data: &[u8]) {
            for (offset, byte) in data.iter().enumerate() {
                self.bytes.insert(address + offset as u64, *byte);
            }
        }

        fn write_u8(&mut self, address: u64, value: u8) {
            self.write(address, &[value]);
        }

        fn write_u32(&mut self, address: u64, value: u32) {
            self.write(address, &value.to_le_bytes());
        }

        fn write_u64(&mut self, address: u64, value: u64) {
            self.write(address, &value.to_le_bytes());
        }

        fn read(&self, address: u64, len: usize) -> Option<Vec<u8>> {
            (0..len as u64)
                .map(|offset| self.bytes.get(&(address + offset)).copied())
                .collect()
        }

        fn table(&mut self, header: u64, buckets: u64, size: u32, cap: u32, max_distance: u8) {
            self.write_u64(header, buckets);
            self.write_u32(header + 0x08, size);
            self.write_u32(header + 0x0C, cap);
            self.write_u8(header + 0x10, max_distance);
            let ceiling = u64::from(cap) + u64::from(max_distance) + 1;
            for index in 0..=ceiling {
                let bucket = buckets + index * 0x18;
                self.write(bucket, &[0u8; 0x18]);
            }
            self.write_u8(buckets + ceiling * 0x18 + 0x04, 0xFF);
        }

        fn fill_bucket(&mut self, buckets: u64, index: u64, key: u32, value: u64) {
            let bucket = buckets + index * 0x18;
            self.write_u8(bucket + 0x04, 1);
            self.write_u32(bucket + 0x08, key);
            self.write_u64(bucket + 0x10, value);
        }

        fn destroy_bucket(&mut self, buckets: u64, index: u64) {
            self.write_u8(buckets + index * 0x18 + 0x04, 0);
        }
    }

    impl RobinHoodMemory for FakeMem {
        fn read_u8(&self, address: u64) -> Option<u8> {
            Some(self.read(address, 1)?[0])
        }
        fn read_u32(&self, address: u64) -> Option<u32> {
            Some(u32::from_le_bytes(self.read(address, 4)?.try_into().ok()?))
        }
        fn read_u64(&self, address: u64) -> Option<u64> {
            Some(u64::from_le_bytes(self.read(address, 8)?.try_into().ok()?))
        }
        fn is_heap_pointer(&self, value: u64) -> bool {
            (HEAP_LO..HEAP_HI).contains(&value) && value % 8 == 0
        }
    }

    const HEADER: u64 = 0x20000000000;
    const BUCKETS: u64 = 0x20000010000;
    const LIVE_RECORD: u64 = 0x20000020000;
    const DEAD_RECORD: u64 = 0x20000030000;

    #[test]
    fn a_destroyed_bucket_ahead_of_the_live_one_is_never_returned() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 0, 11, DEAD_RECORD);
        mem.destroy_bucket(BUCKETS, 0);
        mem.fill_bucket(BUCKETS, 3, 22, LIVE_RECORD);

        let table = read_robin_hood_table(&mem, HEADER).expect("header and array agree");
        let RobinHoodTable::Entries(entries) = table else {
            panic!("expected one live entry, got Empty");
        };
        assert_eq!(entries.len(), 1, "only the occupied bucket may be returned");
        assert_eq!(entries[0].key, 22);
        assert_eq!(entries[0].value, LIVE_RECORD);
        assert!(
            entries.iter().all(|entry| entry.value != DEAD_RECORD),
            "the destroyed record's pointer survives in the value slot and must \
             never be published; only _DistancePlus1 decides occupancy"
        );
    }

    #[test]
    fn every_bucket_of_the_declared_range_is_walked() {
        let mut mem = FakeMem::default();
        let cap = 64u32;
        let max_distance = 5u8;
        mem.table(HEADER, BUCKETS, 2, cap, max_distance);
        mem.fill_bucket(BUCKETS, 0, 1, LIVE_RECORD);
        let last = u64::from(cap) + u64::from(max_distance);
        mem.fill_bucket(BUCKETS, last, 2, LIVE_RECORD + 8);

        let RobinHoodTable::Entries(entries) = read_robin_hood_table(&mem, HEADER).unwrap() else {
            panic!("expected entries");
        };
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].key, 2);
        assert!(
            last * 0x18 >= 0x300,
            "the far bucket must sit beyond the 0x300 window this replaces \
             (offset {:#x})",
            last * 0x18
        );
    }

    #[test]
    fn a_count_that_disagrees_with_the_array_refuses() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 0, 11, LIVE_RECORD);
        mem.fill_bucket(BUCKETS, 4, 12, LIVE_RECORD + 8);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::CountMismatch, HEADER))
        );
    }

    #[test]
    fn a_count_higher_than_the_live_buckets_refuses() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 2, 8, 2);
        mem.fill_bucket(BUCKETS, 0, 11, LIVE_RECORD);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::CountMismatch, HEADER))
        );
    }

    #[test]
    fn the_sentinel_inside_the_declared_range_refuses() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 0, 11, LIVE_RECORD);
        mem.write_u8(BUCKETS + 5 * 0x18 + 0x04, 0xFF);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::InvalidHeader, BUCKETS + 5 * 0x18))
        );
    }

    #[test]
    fn a_zero_count_over_a_heap_array_is_still_proven_not_assumed() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 0, 8, 2);
        mem.fill_bucket(BUCKETS, 1, 11, LIVE_RECORD);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::CountMismatch, HEADER)),
            "a zero count must be checked against the array, not taken on trust"
        );
    }

    #[test]
    fn the_static_empty_singleton_reads_as_empty() {
        let mut mem = FakeMem::default();
        mem.write_u64(HEADER, 0x143099fd0);
        mem.write_u32(HEADER + 0x08, 0);
        mem.write_u32(HEADER + 0x0C, 0);
        mem.write_u8(HEADER + 0x10, 0);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Ok(RobinHoodTable::Empty)
        );
    }

    #[test]
    fn a_bucket_emptied_after_the_walk_is_no_longer_live() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 3, 22, LIVE_RECORD);
        let RobinHoodTable::Entries(entries) = read_robin_hood_table(&mem, HEADER).unwrap() else {
            panic!("expected entries");
        };
        let entry = entries[0];
        assert!(bucket_still_live(&mem, &entry));

        mem.destroy_bucket(BUCKETS, 3);
        assert!(
            !bucket_still_live(&mem, &entry),
            "the walk's own sample is stale; the commit-time check is what sees it"
        );
    }

    #[test]
    fn a_bucket_whose_value_was_replaced_is_no_longer_live() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 3, 22, LIVE_RECORD);
        let RobinHoodTable::Entries(entries) = read_robin_hood_table(&mem, HEADER).unwrap() else {
            panic!("expected entries");
        };
        let entry = entries[0];
        mem.write_u64(BUCKETS + 3 * 0x18 + 0x10, DEAD_RECORD);
        assert!(!bucket_still_live(&mem, &entry));
    }

    #[test]
    fn a_torn_zero_count_over_a_non_heap_array_is_not_reported_as_empty() {
        struct TearingCount {
            inner: FakeMem,
            reads: std::cell::Cell<u32>,
        }
        impl RobinHoodMemory for TearingCount {
            fn read_u8(&self, address: u64) -> Option<u8> {
                self.inner.read_u8(address)
            }
            fn read_u32(&self, address: u64) -> Option<u32> {
                if address == HEADER + 0x08 {
                    let seen = self.reads.get();
                    self.reads.set(seen + 1);
                    return Some(if seen == 0 { 0 } else { 1 });
                }
                self.inner.read_u32(address)
            }
            fn read_u64(&self, address: u64) -> Option<u64> {
                self.inner.read_u64(address)
            }
            fn is_heap_pointer(&self, value: u64) -> bool {
                self.inner.is_heap_pointer(value)
            }
        }

        let mut inner = FakeMem::default();
        inner.write_u64(HEADER, 0x143099fd0);
        inner.write_u32(HEADER + 0x0C, 8);
        inner.write_u8(HEADER + 0x10, 2);
        let mem = TearingCount {
            inner,
            reads: std::cell::Cell::new(0),
        };
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::CountMismatch, HEADER))
        );
    }

    #[test]
    fn a_non_heap_array_under_a_live_count_refuses() {
        let mut mem = FakeMem::default();
        mem.write_u64(HEADER, 0x143099fd0);
        mem.write_u32(HEADER + 0x08, 1);
        mem.write_u32(HEADER + 0x0C, 8);
        mem.write_u8(HEADER + 0x10, 2);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::InvalidPointer, 0x143099fd0))
        );
    }

    #[test]
    fn a_capacity_below_the_count_refuses() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 9, 8, 2);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::InvalidHeader, HEADER))
        );
    }

    #[test]
    fn a_capacity_past_the_ceiling_refuses() {
        let mut mem = FakeMem::default();
        mem.write_u64(HEADER, BUCKETS);
        mem.write_u32(HEADER + 0x08, 0);
        mem.write_u32(HEADER + 0x0C, bp::DOMAIN66_MAX_ROBIN_HOOD_CAP + 1);
        mem.write_u8(HEADER + 0x10, 2);
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::InvalidHeader, HEADER))
        );
    }

    #[test]
    fn an_unreadable_bucket_refuses_against_that_bucket() {
        let mut mem = FakeMem::default();
        mem.table(HEADER, BUCKETS, 1, 8, 2);
        mem.fill_bucket(BUCKETS, 0, 11, LIVE_RECORD);
        mem.bytes.remove(&(BUCKETS + 6 * 0x18 + 0x04));
        assert_eq!(
            read_robin_hood_table(&mem, HEADER),
            Err((RobinHoodFault::ReadUnavailable, BUCKETS + 6 * 0x18))
        );
    }
}
