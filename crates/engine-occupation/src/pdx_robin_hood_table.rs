
pub const DATA: usize = 0x00;
pub const SIZE: usize = 0x08;
pub const CAPACITY_OR_MASK: usize = 0x0c;
pub const MAX_DISTANCE: usize = 0x10;
const COUNTRY_KEY_HASH_MULTIPLIER: u32 = 0x045d_9f3b;

use engine_native_memory::{Memory, read};
use crate::pdx_robin_hood_bucket as bucket;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: u32,
    pub value: usize,
}

pub fn find(
    memory: &impl Memory,
    address: usize,
    key: u32,
) -> Result<Option<Entry>, &'static str> {
    let header = read::<[u8; 17]>(memory, address).ok_or("occupation table header unreadable")?;
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| "occupation table pointer overflow")?;
    let size = u32::from_le_bytes(header[SIZE..SIZE + 4].try_into().unwrap());
    let mask = u32::from_le_bytes(
        header[CAPACITY_OR_MASK..CAPACITY_OR_MASK + 4]
            .try_into()
            .unwrap(),
    );
    if size > mask {
        return Err("occupation table size exceeds capacity");
    }
    if data == 0 {
        return if size == 0 {
            Ok(None)
        } else {
            Err("occupation live table has null storage")
        };
    }
    let mix = |value: u32| (value ^ (value >> 16)).wrapping_mul(COUNTRY_KEY_HASH_MULTIPLIER);
    let hash = mix(mix(key));
    let first = ((hash ^ (hash >> 16)) & mask) as usize;
    let span = (mask as usize)
        .checked_add(usize::from(header[MAX_DISTANCE]))
        .and_then(|n| n.checked_add(1))
        .ok_or("occupation table span overflow")?;
    for (distance, index) in (1usize..).zip(first..span) {
        let at = index
            .checked_mul(bucket::STRIDE)
            .and_then(|n| data.checked_add(n))
            .ok_or("occupation bucket address overflow")?;
        let value = read::<[u8; 24]>(memory, at).ok_or("occupation bucket unreadable")?;
        let stored = usize::from(value[bucket::DISTANCE_PLUS_ONE]);
        if stored == usize::from(bucket::ITERATOR_SENTINEL_DISTANCE) {
            return Err("occupation iterator sentinel inside lookup probe");
        }
        if stored < distance {
            return Ok(None);
        }
        if u32::from_le_bytes(value[bucket::KEY..bucket::KEY + 4].try_into().unwrap()) == key {
            let value = usize::try_from(u64::from_le_bytes(
                value[bucket::VALUE..].try_into().unwrap(),
            ))
            .map_err(|_| "occupation bucket pointer overflow")?;
            if value == 0 {
                return Err("occupation live bucket has null value");
            }
            return Ok(Some(Entry { key, value }));
        }
    }
    Ok(None)
}

pub(crate) fn visit(
    memory: &impl Memory,
    address: usize,
    mut visit_entry: impl FnMut(Entry) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let header = read::<[u8; 17]>(memory, address).ok_or("occupation table header unreadable")?;
    let data = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
        .map_err(|_| "occupation table pointer overflow")?;
    let size = u32::from_le_bytes(header[SIZE..SIZE + 4].try_into().unwrap());
    let capacity = u32::from_le_bytes(
        header[CAPACITY_OR_MASK..CAPACITY_OR_MASK + 4]
            .try_into()
            .unwrap(),
    );
    if size > capacity {
        return Err("occupation table size exceeds capacity");
    }
    if data == 0 {
        return if size == 0 {
            Ok(())
        } else {
            Err("occupation live table has null storage")
        };
    }
    let span = (capacity as usize)
        .checked_add(usize::from(header[MAX_DISTANCE]))
        .and_then(|n| n.checked_add(1))
        .ok_or("occupation table span overflow")?;
    data.checked_add(
        span.checked_mul(bucket::STRIDE)
            .ok_or("occupation table span overflow")?,
    )
    .ok_or("occupation table address overflow")?;
    let mut observed = 0_u32;
    for index in 0..span {
        let value = read::<[u8; 24]>(memory, data + index * bucket::STRIDE)
            .ok_or("occupation bucket unreadable")?;
        match value[bucket::DISTANCE_PLUS_ONE] {
            bucket::EMPTY_DISTANCE => continue,
            bucket::ITERATOR_SENTINEL_DISTANCE => {
                return Err("occupation iterator sentinel inside table");
            }
            _ => {}
        }
        observed = observed
            .checked_add(1)
            .ok_or("occupation live count overflow")?;
        if observed > size {
            return Err("occupation bucket count differs from header");
        }
        let key = u32::from_le_bytes(value[bucket::KEY..bucket::KEY + 4].try_into().unwrap());
        let value = usize::try_from(u64::from_le_bytes(
            value[bucket::VALUE..].try_into().unwrap(),
        ))
        .map_err(|_| "occupation bucket pointer overflow")?;
        if value == 0 {
            return Err("occupation live bucket has null value");
        }
        visit_entry(Entry { key, value })?;
    }
    if observed != size {
        return Err("occupation bucket count differs from header");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::memory::SliceMemory;

    #[test]
    fn occupation_key_lookup_probes_collisions_and_never_follows_an_empty_stale_value() {
        let mut bytes = vec![0; 0x100];
        bytes[..8].copy_from_slice(&0x1040u64.to_le_bytes());
        bytes[SIZE..SIZE + 4].copy_from_slice(&2u32.to_le_bytes());
        bytes[CAPACITY_OR_MASK..CAPACITY_OR_MASK + 4].copy_from_slice(&3u32.to_le_bytes());
        bytes[MAX_DISTANCE] = 2;
        for (index, key, distance, value) in [
            (0, 2u32, 1, 0x2000u64),
            (1, 11, 2, 0x3000),
            (2, 10, 0, u64::MAX),
        ] {
            let at = 0x40 + index * bucket::STRIDE;
            bytes[at + bucket::DISTANCE_PLUS_ONE] = distance;
            bytes[at + bucket::KEY..at + bucket::KEY + 4].copy_from_slice(&key.to_le_bytes());
            bytes[at + bucket::VALUE..at + bucket::VALUE + 8].copy_from_slice(&value.to_le_bytes());
        }
        let memory = SliceMemory {
            base: 0x1000,
            bytes: &bytes[..0x88],
        };
        assert_eq!(
            find(&memory, 0x1000, 11),
            Ok(Some(Entry {
                key: 11,
                value: 0x3000
            }))
        );
        assert_eq!(find(&memory, 0x1000, 10), Ok(None));
        assert!(
            find(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..0x60]
                },
                0x1000,
                11
            )
            .is_err()
        );
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x100];
        bytes[..8].copy_from_slice(&0x1040_u64.to_le_bytes());
        bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
        bytes[0x48..0x50].fill(0xff);
        bytes[0x50..0x58].fill(0xff);
        bytes[0x5c] = 1;
        bytes[0x60..0x64].copy_from_slice(&7_u32.to_le_bytes());
        bytes[0x68..0x70].copy_from_slice(&0x3000_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn only_occupied_buckets_are_members_and_live_counts_must_match() {
        let mut bytes = fixture();
        let mut entries = Vec::new();
        visit(
            &SliceMemory {
                base: 0x1000,
                bytes: &bytes,
            },
            0x1000,
            |entry| {
                entries.push(entry);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            entries,
            [Entry {
                key: 7,
                value: 0x3000
            }]
        );
        bytes[8..12].fill(0);
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                |_| Ok(())
            )
            .is_err()
        );
        bytes[8..12].copy_from_slice(&2_u32.to_le_bytes());
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                |_| Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn empty_roots_faults_null_live_values_and_sentinels_are_distinct() {
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &[0; 17]
                },
                0x1000,
                |_| Ok(())
            )
            .is_ok()
        );
        let mut bytes = fixture();
        bytes[0x68..0x70].fill(0);
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                |_| Ok(())
            )
            .is_err()
        );
        bytes[0x5c] = 0xff;
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes
                },
                0x1000,
                |_| Ok(())
            )
            .is_err()
        );
        assert!(
            visit(
                &SliceMemory {
                    base: 0x1000,
                    bytes: &bytes[..16]
                },
                0x1000,
                |_| Ok(())
            )
            .is_err()
        );
    }
}
