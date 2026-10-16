//! Anonymous robin-hood bucket keyed by (province id, node type).
//!
//! Values retain full signed i64 fixed5 precision without a narrower wire range.
//! This key-pair instantiation differs from the companion's single-key map.

use crate::{
    Memory, ccountry_supply_system as country_supply, csupply_system, read, walk::Outcome,
};

/// Disrupted-supply bucket stride.
pub const STRIDE: usize = 0x18;
/// Disrupted-supply u8 DistancePlus1 marker.
pub const DISTANCE_PLUS_ONE: usize = 0x04;
/// Disrupted-supply empty bucket marker.
pub const EMPTY: u8 = 0;
/// Disrupted-supply end sentinel, never a live row.
pub const END: u8 = 0xff;
/// Disrupted-supply key u32 province id.
pub const PROVINCE: usize = 0x08;
/// Disrupted-supply key u32 node type.
pub const NODE_TYPE: usize = 0x0c;
/// Disrupted-supply signed i64 fixed5 value.
pub const VALUE: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    /// Native bucket position, which can change when the table rehashes.
    pub bucket_index: u32,
    pub province: u32,
    pub node_type: u32,
    /// Signed fixed5 value, preserving all i64 bits.
    pub value: i64,
}

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
    mut output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let systems = csupply_system::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..systems.count() {
            let Some(system) = systems.at(slot)? else {
                continue;
            };
            match read_country_buffer(
                memory,
                system.pointer,
                system.country.id,
                output.suffix(len),
            ) {
                Outcome::Rows { len: added } => len += added,
                Outcome::Truncated {
                    len: added,
                    unread_rows: omitted,
                } => {
                    len += added;
                    unread_rows = unread_rows
                        .checked_add(omitted)
                        .ok_or("disrupted supply row count overflow")?;
                }
                Outcome::Unavailable { reason } => return Err(reason),
            }
        }
        Ok(outcome(len, unread_rows))
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

/// The caller authenticates the CCountrySupplySystem and its country backlink.
/// Empty tables have no semantic buckets, even when their allocation is retained.
/// Capacity limits storage; all live markers and keys remain validated so the
/// omitted count is exact and a broken tail cannot masquerade as truncation.
#[cfg(test)]
pub(crate) fn read_country(
    memory: &impl Memory,
    pointer: usize,
    country: u32,
    output: &mut [Row],
) -> Outcome {
    read_country_buffer(memory, pointer, country, crate::walk::Buffer::Fixed(output))
}

pub(crate) fn read_country_buffer(
    memory: &impl Memory,
    pointer: usize,
    country: u32,
    mut output: crate::walk::Buffer<'_, Row>,
) -> Outcome {
    let result = (|| {
        let header = pointer
            .checked_add(country_supply::DISRUPTED_BUCKETS)
            .and_then(|p| read::<[u8; 17]>(memory, p))
            .ok_or("disrupted supply header unreadable")?;
        let buckets = usize::try_from(u64::from_le_bytes(header[..8].try_into().unwrap()))
            .map_err(|_| "disrupted supply pointer overflow")?;
        let occupied = u32::from_le_bytes(header[8..12].try_into().unwrap());
        let mask = u32::from_le_bytes(header[12..16].try_into().unwrap());
        let max_distance = header[16];
        let bucket_count = mask
            .checked_add(1)
            .filter(|count| count.is_power_of_two())
            .ok_or("disrupted supply bucket mask invalid")?;
        if country == 0 || buckets == 0 || buckets % 8 != 0 || occupied > bucket_count {
            return Err("disrupted supply header invalid");
        }
        if occupied == 0 {
            return Ok(Outcome::Rows { len: 0 });
        }
        if max_distance == 0 {
            return Err("disrupted supply probe distance invalid");
        }
        let scan_count = bucket_count
            .checked_add(u32::from(max_distance))
            .filter(|&count| count <= i32::MAX as u32)
            .ok_or("disrupted supply bucket extent invalid")?;
        let end = buckets
            .checked_add(
                (scan_count as usize)
                    .checked_mul(STRIDE)
                    .ok_or("disrupted supply address overflow")?,
            )
            .and_then(|p| p.checked_add(DISTANCE_PLUS_ONE))
            .ok_or("disrupted supply address overflow")?;
        if read::<u8>(memory, end) != Some(END) {
            return Err("disrupted supply end sentinel invalid");
        }
        output.ensure(occupied as usize)?;
        let mut seen = 0_u32;
        let mut len = 0_usize;
        for bucket_index in 0..scan_count {
            let address = buckets + bucket_index as usize * STRIDE;
            let marker = read::<u8>(memory, address + DISTANCE_PLUS_ONE)
                .ok_or("disrupted supply bucket marker unreadable")?;
            if marker == EMPTY {
                continue;
            }
            if marker == END || marker > max_distance {
                return Err("disrupted supply bucket marker invalid");
            }
            let key = read::<[u8; 8]>(memory, address + PROVINCE)
                .ok_or("disrupted supply key unreadable")?;
            let province = u32::from_le_bytes(key[..4].try_into().unwrap());
            let node_type = u32::from_le_bytes(key[4..].try_into().unwrap());
            if province == 0 {
                return Err("disrupted supply province is zero");
            }
            // Use this stored hash geometry and a forward-only lookup.
            // Reaching this bucket first proves that its key is not duplicated,
            // including when the output buffer could not fit the preceding row.
            let home = province.wrapping_mul(13).wrapping_add(node_type) & mask;
            if bucket_index
                .checked_sub(home)
                .and_then(|v| v.checked_add(1))
                != Some(u32::from(marker))
            {
                return Err("disrupted supply key probe distance differs");
            }
            for prior in home..bucket_index {
                let probe = read::<[u8; 12]>(
                    memory,
                    buckets + prior as usize * STRIDE + DISTANCE_PLUS_ONE,
                )
                .ok_or("disrupted supply key probe unreadable")?;
                if u32::from(probe[0]) < prior - home + 1 {
                    return Err("disrupted supply key is unreachable");
                }
                if probe[4..12] == key {
                    return Err("duplicate disrupted supply key");
                }
            }
            seen = seen
                .checked_add(1)
                .ok_or("disrupted supply row count overflow")?;
            if seen > occupied {
                return Err("disrupted supply occupied count differs");
            }
            if len == output.len() {
                continue;
            }
            let value =
                read::<i64>(memory, address + VALUE).ok_or("disrupted supply value unreadable")?;
            output[len] = Row {
                country,
                bucket_index,
                province,
                node_type,
                value,
            };
            len += 1;
        }
        if seen != occupied {
            return Err("disrupted supply occupied count differs");
        }
        Ok(outcome(len, occupied - len as u32))
    })();
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

fn outcome(len: usize, unread_rows: u32) -> Outcome {
    if unread_rows == 0 {
        Outcome::Rows { len }
    } else {
        Outcome::Truncated { len, unread_rows }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const COUNTRY: usize = 0x1000;
    const BUCKETS: usize = 0x1400;

    fn memory() -> Vec<u8> {
        let mut bytes = vec![0; 0x600];
        bytes[country_supply::DISRUPTED_BUCKETS..country_supply::DISRUPTED_BUCKETS + 8]
            .copy_from_slice(&(BUCKETS as u64).to_le_bytes());
        bytes[country_supply::DISRUPTED_COUNT..country_supply::DISRUPTED_COUNT + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        bytes[country_supply::DISRUPTED_MASK..country_supply::DISRUPTED_MASK + 4]
            .copy_from_slice(&3_u32.to_le_bytes());
        bytes[country_supply::DISRUPTED_MAX_DISTANCE] = 2;
        for (index, province, value) in [(1, 1_u32, i64::MAX), (2, 5_u32, i64::MIN)] {
            let offset = BUCKETS - COUNTRY + index * STRIDE;
            bytes[offset + DISTANCE_PLUS_ONE] = index as u8;
            bytes[offset + PROVINCE..offset + NODE_TYPE].copy_from_slice(&province.to_le_bytes());
            bytes[offset + VALUE..offset + STRIDE].copy_from_slice(&value.to_le_bytes());
        }
        bytes[BUCKETS - COUNTRY + 6 * STRIDE + DISTANCE_PLUS_ONE] = END;
        bytes
    }

    fn capture(bytes: &[u8], output: &mut [Row]) -> Outcome {
        read_country(
            &SliceMemory {
                base: COUNTRY,
                bytes,
            },
            COUNTRY,
            19,
            output,
        )
    }

    #[test]
    fn collision_chain_preserves_signed_i64_and_exact_omitted_count() {
        let bytes = memory();
        let mut rows = [Row::default(); 2];
        assert_eq!(capture(&bytes, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(
            (rows[0].country, rows[0].province, rows[0].value),
            (19, 1, i64::MAX)
        );
        assert_eq!(
            (rows[1].bucket_index, rows[1].province, rows[1].value),
            (2, 5, i64::MIN)
        );
        assert_eq!(
            capture(&bytes, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(
            capture(&bytes, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn malformed_chain_duplicate_key_and_count_fail_even_after_capacity() {
        let mut bytes = memory();
        let key = BUCKETS - COUNTRY + 2 * STRIDE + PROVINCE;
        bytes[key..key + 4].copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut []),
            Outcome::Unavailable {
                reason: "duplicate disrupted supply key"
            }
        );
        bytes[key..key + 4].copy_from_slice(&2_u32.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut []),
            Outcome::Unavailable {
                reason: "disrupted supply key probe distance differs"
            }
        );
        bytes = memory();
        bytes[country_supply::DISRUPTED_COUNT..country_supply::DISRUPTED_COUNT + 4]
            .copy_from_slice(&1_u32.to_le_bytes());
        assert_eq!(
            capture(&bytes, &mut []),
            Outcome::Unavailable {
                reason: "disrupted supply occupied count differs"
            }
        );
        bytes = memory();
        bytes[BUCKETS - COUNTRY + 6 * STRIDE + DISTANCE_PLUS_ONE] = EMPTY;
        assert_eq!(
            capture(&bytes, &mut []),
            Outcome::Unavailable {
                reason: "disrupted supply end sentinel invalid"
            }
        );
    }

    #[test]
    fn retained_empty_table_does_not_touch_old_buckets_and_fault_is_unavailable() {
        let mut bytes = memory();
        bytes[country_supply::DISRUPTED_COUNT..country_supply::DISRUPTED_COUNT + 4].fill(0);
        assert_eq!(capture(&bytes[..0x200], &mut []), Outcome::Rows { len: 0 });
        assert_eq!(
            capture(&bytes[..country_supply::DISRUPTED_MAX_DISTANCE], &mut []),
            Outcome::Unavailable {
                reason: "disrupted supply header unreadable"
            }
        );
        bytes[country_supply::DISRUPTED_COUNT..country_supply::DISRUPTED_COUNT + 4]
            .copy_from_slice(&2_u32.to_le_bytes());
        assert_eq!(
            capture(&bytes[..0x200], &mut []),
            Outcome::Unavailable {
                reason: "disrupted supply end sentinel invalid"
            }
        );
    }
}
