//! Robin-hood table used by occupation status, laws and garrisons.
//! Bucket extent combines the stored span input and maximum probe distance,
//! independently of live entry count.

/// Bucket-array pointer.
pub const DATA: usize = 0x00;
/// Live u32 entry count.
pub const SIZE: usize = 0x08;
/// Stored u32 bucket-span input.
pub const CAPACITY_OR_MASK: usize = 0x0c;
/// Maximum u8 probe distance.
pub const MAX_DISTANCE: usize = 0x10;

use crate::{Memory, pdx_robin_hood_bucket as bucket, read};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub key: u32,
    pub value: usize,
}

/// Visits the native occupied buckets once. Empty buckets may retain stale
/// pointers; those pointers are never followed. No heap storage or retry occurs.
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
    use crate::memory::SliceMemory;

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x100];
        bytes[..8].copy_from_slice(&0x1040_u64.to_le_bytes());
        bytes[8..12].copy_from_slice(&1_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&2_u32.to_le_bytes());
        // Empty bucket retains a stale pointer and key; neither is admitted.
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
