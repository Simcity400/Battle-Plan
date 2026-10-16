//! Country-owned radar coverage group containing three arrays and producer
//! state. Without a vtable, authentication uses allocator alignment, valid vector
//! headers and the backlink to the owning radar pool, whose vtable is checked first.

use crate::{
    Memory, ccountry,
    cpdx_array::{Array, copy_u32s},
    cradars_pool, cstate, read,
    walk::{Buffer, Outcome},
};

/// Province-ID array pointer.
pub const PROVINCES: usize = 0;
/// Province-ID array u32 capacity.
pub const PROVINCES_CAPACITY: usize = 0x08;
/// Province-ID array u32 count.
pub const PROVINCES_COUNT: usize = 0x0c;
/// Auxiliary pointer array; pointee meaning is unknown.
pub const AUXILIARY_ARRAY_18: usize = 0x18;
/// Auxiliary pointer-array signed i32 count.
pub const AUXILIARY_ARRAY_18_COUNT: usize = 0x24;
/// Auxiliary signed i32 array of associated-object coverage counts.
pub const AUXILIARY_ARRAY_30: usize = 0x30;
/// Auxiliary integer-array signed i32 count.
pub const AUXILIARY_ARRAY_30_COUNT: usize = 0x3c;
/// Host state pointer selected from controlled states.
pub const STATE: usize = 0x48;
/// Signed raw i64 scalar using fixed-point scale 100000.
pub const SCALAR_50: usize = 0x50;
/// Signed i32 aggregate of signed i16 state operands; semantic label is unknown.
pub const AGGREGATE_58: usize = 0x58;
/// Owning radar-pool pointer.
pub const PARENT: usize = 0x60;
/// Radar-group allocation size.
pub const SIZE: usize = 0x70;
/// Coverage update using game-state province indices.
pub const UPDATE_RVA: usize = 0x0108_ec70;
/// Group creation per radar-carrying controlled state.
pub const CREATE_RVA: usize = 0x0109_3140;
/// Building-change producer creating or reusing a group before updating coverage.
pub const BUILDING_CHANGE_RVA: usize = 0x0109_3790;
/// Province-controller-change producer selecting by state and swap-removing groups.
pub const CONTROLLER_CHANGE_RVA: usize = 0x0109_3ac0;
/// Fog consumer of radar coverage.
pub const FOG_CONSUMER_RVA: usize = 0x0160_6390;

/// One country's radar group in vector order. Its ordinal is a position,
/// not an identity: swap-removal can move coverage between hours. Host state
/// is the closest stable name retained by the record.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw CCountry link id of the owning country.
    pub country: u32,
    pub ordinal: u32,
    /// Host CState id; a null `+0x48` pointer is recorded as absent, not zero.
    pub state: Option<u32>,
    pub provinces_start: usize,
    pub provinces_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub provinces_len: usize,
    /// Full native province demand, including whole rows omitted for space.
    pub required_provinces: usize,
}

/// Read every canonical country's groups and each group's full province-ID
/// vector. Null or misaligned groups, invalid owner backlinks or state identities,
/// and zero province IDs refuse the whole table. Capacity omits a whole group row
/// and reports remaining demand.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    provinces: &mut [u32],
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        Buffer::Fixed(output),
        Buffer::Fixed(provinces),
    )
}

/// Reserve from each original group/coverage header before copying its body.
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    provinces: &mut Vec<u32>,
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        Buffer::Growing(provinces),
    )
}

fn walk_buffers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
    mut provinces: Buffer<'_, u32>,
) -> Walk {
    let mut provinces_len = 0;
    let mut required_provinces = 0_usize;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let pool = country
                .pointer
                .checked_add(ccountry::RADARS_POOL)
                .ok_or("radar pool address overflow")?;
            let expected = base
                .checked_add(cradars_pool::VTABLE_RVA)
                .ok_or("module address overflow")?;
            let mut primary = [0; 8];
            let mut owner = [0; 4];
            let mut header = [0; 16];
            // Read only the three independent native fields in one guard. The
            // checks retain the former field and owner-resolution precedence.
            let completed = memory.copy_scattered(
                [
                    Some(pool),
                    country.pointer.checked_add(ccountry::RADARS_POOL_COUNTRY),
                    country.pointer.checked_add(ccountry::RADAR_GROUPS),
                ]
                .map(|at| at.unwrap_or(0)),
                &mut [&mut primary, &mut owner, &mut header],
            );
            if completed == 0 || u64::from_le_bytes(primary) != expected as u64 {
                return Err("radar pool vtable mismatch");
            }
            if completed < 2 {
                return Err("radar pool country reference unreadable");
            }
            // A pool may retain a native country-id alias after slot reuse;
            // resolution must still name this exact authenticated object.
            if countries.resolve(u32::from_le_bytes(owner))?.pointer != country.pointer {
                return Err("radar pool country differs from its owner");
            }
            let groups = (completed == 3)
                .then(|| Array::from_header(header, 8))
                .flatten()
                .ok_or("radar group vector unreadable or invalid")?;
            // Use the complete group count and each complete coverage count.
            output.ensure(
                len.checked_add(groups.count as usize)
                    .ok_or("radar row demand overflow")?,
            )?;
            for ordinal in 0..groups.count {
                let group = groups
                    .element(ordinal, 8)
                    .and_then(|at| read::<u64>(memory, at))
                    .and_then(|pointer| usize::try_from(pointer).ok())
                    .ok_or("radar group pointer unreadable or address overflow")?;
                if group == 0 {
                    return Err("counted radar group vector contains a null group");
                }
                // Eight-byte allocator alignment and the owning-pool backlink authenticate this nonpolymorphic record.
                if group % 8 != 0 {
                    return Err("radar group pointer is not eight-aligned");
                }
                let mut parent = [0; 8];
                let mut state = [0; 8];
                let mut header = [0; 16];
                let completed = memory.copy_scattered(
                    [PARENT, STATE, PROVINCES].map(|offset| group.checked_add(offset).unwrap_or(0)),
                    &mut [&mut parent, &mut state, &mut header],
                );
                if completed == 0 || u64::from_le_bytes(parent) != pool as u64 {
                    return Err("radar group parent differs from its country pool");
                }
                let state = (completed >= 2)
                    .then(|| usize::try_from(u64::from_le_bytes(state)).ok())
                    .flatten()
                    .ok_or("radar group state pointer unreadable")?;
                let state = if state == 0 {
                    None
                } else {
                    let expected = base
                        .checked_add(cstate::VTABLE_RVA)
                        .ok_or("module address overflow")?;
                    if read::<u64>(memory, state) != Some(expected as u64) {
                        return Err("radar group state vtable mismatch");
                    }
                    Some(
                        state
                            .checked_add(cstate::ID)
                            .and_then(|at| read::<u32>(memory, at))
                            .ok_or("radar group state id unreadable")?,
                    )
                };
                let covered = (completed == 3)
                    .then(|| Array::from_header(header, 4))
                    .flatten()
                    .ok_or("radar coverage vector unreadable or invalid")?;
                let count = covered.count as usize;
                required_provinces = required_provinces
                    .checked_add(count)
                    .ok_or("radar coverage demand overflow")?;
                provinces.ensure(required_provinces)?;
                if len == output.len() || count > provinces.len() - provinces_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("radar group omitted count overflow")?;
                    continue;
                }
                let destination = &mut provinces[provinces_len..provinces_len + count];
                copy_u32s(memory, covered, destination, "radar coverage unreadable")?;
                if destination.contains(&0) {
                    return Err("radar coverage contains province id zero");
                }
                output[len] = Row {
                    country: country.id,
                    ordinal,
                    state,
                    provinces_start: provinces_len,
                    provinces_len: count,
                };
                len += 1;
                provinces_len += count;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    Walk {
        outcome: result.unwrap_or_else(|reason| Outcome::Unavailable { reason }),
        provinces_len,
        required_provinces,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    pub(crate) const BASE: usize = 0x1000;
    const COUNTRY: usize = 0x3000;
    const OTHER_COUNTRY: usize = 0x3800;
    const GROUP: usize = 0x5000;
    const SECOND_GROUP: usize = 0x5400;
    const STATE_A: usize = 0x6000;
    const COVERED: usize = 0x7000;

    pub(crate) fn put(bytes: &mut [u8], address: usize, value: &[u8]) {
        bytes[address - BASE..address - BASE + value.len()].copy_from_slice(value);
    }

    /// One country with two groups; the second has a null state and no coverage.
    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRIES,
            &0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRIES_CAPACITY,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRIES_COUNT,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRY_LINKS,
            &0x2100_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRY_LINKS_CAPACITY,
            &4_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + crate::cgame_state::COUNTRY_LINKS_COUNT,
            &4_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x2008, &(COUNTRY as u64).to_le_bytes());
        put(&mut bytes, 0x2010, &(OTHER_COUNTRY as u64).to_le_bytes());
        // Link slot 1 and the first country's own id 2 resolve to slot 1; link
        // slot 2 and the second country's id 3 resolve to slot 2.
        put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
        put(&mut bytes, 0x2108, &2_u32.to_le_bytes());
        put(&mut bytes, 0x210c, &2_u32.to_le_bytes());
        // Link slot 1 names the first country and slots 2 and 3 the second.
        put(
            &mut bytes,
            OTHER_COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            OTHER_COUNTRY + ccountry::ID,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            OTHER_COUNTRY + ccountry::RADARS_POOL_COUNTRY,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY,
            &(ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        for country in [COUNTRY, OTHER_COUNTRY] {
            put(
                &mut bytes,
                country + ccountry::RADARS_POOL,
                &(cradars_pool::VTABLE_RVA as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            COUNTRY + ccountry::RADARS_POOL_COUNTRY,
            &1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            COUNTRY + ccountry::RADAR_GROUPS,
            &0x4000_u64.to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + 0x1128, &2_u32.to_le_bytes());
        put(
            &mut bytes,
            COUNTRY + ccountry::RADAR_GROUPS_COUNT,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x4000, &(GROUP as u64).to_le_bytes());
        put(&mut bytes, 0x4008, &(SECOND_GROUP as u64).to_le_bytes());
        let parent = (COUNTRY + ccountry::RADARS_POOL) as u64;
        for group in [GROUP, SECOND_GROUP] {
            put(&mut bytes, group + PARENT, &parent.to_le_bytes());
        }
        put(&mut bytes, GROUP + STATE, &(STATE_A as u64).to_le_bytes());
        put(
            &mut bytes,
            GROUP + PROVINCES,
            &(COVERED as u64).to_le_bytes(),
        );
        put(&mut bytes, GROUP + PROVINCES_CAPACITY, &4_u32.to_le_bytes());
        put(&mut bytes, GROUP + PROVINCES_COUNT, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            STATE_A,
            &(cstate::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, STATE_A + cstate::ID, &77_u32.to_le_bytes());
        for (index, id) in [u32::MAX, 1, 13_413_u32].into_iter().enumerate() {
            put(&mut bytes, COVERED + index * 4, &id.to_le_bytes());
        }
        bytes
    }

    fn walked(bytes: &[u8], rows: usize, arena: usize) -> (Walk, Vec<Row>, Vec<u32>) {
        let memory = SliceMemory { base: BASE, bytes };
        let mut output = vec![Row::default(); rows];
        let mut provinces = vec![0; arena];
        let walk = walk(&memory, 0, BASE, &mut output, &mut provinces);
        (walk, output, provinces)
    }

    #[test]
    fn native_group_order_state_and_covered_ids_survive_exactly() {
        let (walk, rows, provinces) = walked(&fixture(), 4, 8);
        assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
        assert_eq!((walk.provinces_len, walk.required_provinces), (3, 3));
        assert_eq!(
            rows[0],
            Row {
                country: 1,
                ordinal: 0,
                state: Some(77),
                provinces_start: 0,
                provinces_len: 3,
            }
        );
        assert_eq!(
            rows[1],
            Row {
                country: 1,
                ordinal: 1,
                state: None,
                provinces_start: 3,
                provinces_len: 0,
            }
        );
        assert_eq!(&provinces[..3], [u32::MAX, 1, 13_413]);
    }

    #[test]
    fn capacity_omits_whole_groups_and_reports_the_full_demand() {
        let (walk, rows, _) = walked(&fixture(), 1, 2);
        assert_eq!(
            walk.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        // The first group does not fit the arena, so the empty second one lands.
        assert_eq!(rows[0].ordinal, 1);
        assert_eq!((walk.provinces_len, walk.required_provinces), (0, 3));
        let (walk, _, _) = walked(&fixture(), 0, 8);
        assert_eq!(
            walk.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(walk.required_provinces, 3);
    }

    #[test]
    fn torn_headers_null_groups_and_zero_provinces_refuse_the_table() {
        for (address, value, reason) in [
            (
                0x4008,
                0_u64,
                "counted radar group vector contains a null group",
            ),
            (0x4008, 0x5401, "radar group pointer is not eight-aligned"),
            (
                SECOND_GROUP + PARENT,
                0x1234,
                "radar group parent differs from its country pool",
            ),
            (GROUP + STATE, 0x6008, "radar group state vtable mismatch"),
            (
                COUNTRY + ccountry::RADARS_POOL,
                0x1234,
                "radar pool vtable mismatch",
            ),
            (
                COUNTRY + ccountry::RADAR_GROUPS,
                0,
                "radar group vector unreadable or invalid",
            ),
        ] {
            let mut bytes = fixture();
            put(&mut bytes, address, &value.to_le_bytes());
            let (walk, _, _) = walked(&bytes, 4, 8);
            assert_eq!(walk.outcome, Outcome::Unavailable { reason });
        }
        let mut bytes = fixture();
        put(&mut bytes, COVERED + 4, &0_u32.to_le_bytes());
        let (walk, _, _) = walked(&bytes, 4, 8);
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "radar coverage contains province id zero"
            }
        );
        let mut bytes = fixture();
        // A count above its own capacity is a torn vector header, not a census.
        put(&mut bytes, GROUP + PROVINCES_COUNT, &9_u32.to_le_bytes());
        let (walk, _, _) = walked(&bytes, 4, 8);
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "radar coverage vector unreadable or invalid"
            }
        );
        let mut bytes = fixture();
        // The record now names the other authenticated country, which is a
        // recycled or torn reference rather than this country's coverage.
        put(
            &mut bytes,
            COUNTRY + ccountry::RADARS_POOL_COUNTRY,
            &3_u32.to_le_bytes(),
        );
        let (walk, _, _) = walked(&bytes, 4, 8);
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "radar pool country differs from its owner"
            }
        );
    }

    #[test]
    fn an_empty_world_reports_no_rows_without_inventing_coverage() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            COUNTRY + ccountry::RADAR_GROUPS_COUNT,
            &0_u32.to_le_bytes(),
        );
        let (walk, _, _) = walked(&bytes, 4, 8);
        assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
        assert_eq!((walk.provinces_len, walk.required_provinces), (0, 0));
    }
}
