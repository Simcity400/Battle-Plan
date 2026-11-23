
use engine_native_memory::{
    Memory,
    cpdx_array::{Array, copy_u32s},
    read,
    walk::{Buffer, Outcome},
};
use engine_native::{ccountry, cstate};
use engine_native_parts::cradars_pool;
use engine_sites::Rva;

pub const PROVINCES: usize = 0;
#[cfg(test)]
pub const PROVINCES_CAPACITY: usize = 0x08;
pub const PROVINCES_COUNT: usize = 0x0c;
pub const STATE: usize = 0x48;
pub const SCALAR_50: usize = 0x50;
pub const COVERAGE_RADIUS: usize = SCALAR_50;
pub const PARENT: usize = 0x60;
pub const SIZE: usize = 0x70;
pub const UPDATE_RVA: Rva = Rva(0x0112_b630);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub ordinal: u32,
    pub state: Option<u32>,
    pub symbol: Option<Symbol>,
    pub provinces_start: usize,
    pub provinces_len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Symbol {
    pub radius: i64,
}

pub fn radius(memory: &impl Memory, group: usize) -> Option<i64> {
    group
        .checked_add(COVERAGE_RADIUS)
        .and_then(|at| read::<i64>(memory, at))
}

pub fn symbol(memory: &impl Memory, group: usize, state: usize) -> Option<Symbol> {
    (state != 0)
        .then(|| radius(memory, group))
        .flatten()
        .map(|radius| Symbol { radius })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub provinces_len: usize,
    pub required_provinces: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffers(memory, base, game_state, Buffer::Count, Buffer::Count, |_, _| None)
}

pub fn walk(
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
        |_, _| None,
    )
}

pub fn walk_with_symbols(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    provinces: &mut Vec<u32>,
    symbols: impl FnMut(usize, usize) -> Option<Symbol>,
) -> Walk {
    walk_buffers(
        memory,
        base,
        game_state,
        Buffer::Growing(output),
        Buffer::Growing(provinces),
        symbols,
    )
}

fn walk_buffers(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: Buffer<'_, Row>,
    mut provinces: Buffer<'_, u32>,
    mut symbols: impl FnMut(usize, usize) -> Option<Symbol>,
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
            let expected = engine_sites::at(base, cradars_pool::VTABLE_RVA)
                .ok_or("module address overflow")?;
            let mut primary = [0; 8];
            let mut owner = [0; 4];
            let mut header = [0; 16];
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
            if countries.resolve(u32::from_le_bytes(owner))?.pointer != country.pointer {
                return Err("radar pool country differs from its owner");
            }
            let groups = (completed == 3)
                .then(|| Array::from_header(header, 8))
                .flatten()
                .ok_or("radar group vector unreadable or invalid")?;
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
                let host = state;
                let state = if state == 0 {
                    None
                } else {
                    let expected = engine_sites::at(base, cstate::VTABLE_RVA)
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
                    symbol: symbols(group, host),
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
    use engine_native_memory::memory::SliceMemory;

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

    pub(crate) fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRIES,
            &0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRIES_CAPACITY,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRIES_COUNT,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRY_LINKS,
            &0x2100_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRY_LINKS_CAPACITY,
            &4_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + engine_native::cgame_state::COUNTRY_LINKS_COUNT,
            &4_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x2008, &(COUNTRY as u64).to_le_bytes());
        put(&mut bytes, 0x2010, &(OTHER_COUNTRY as u64).to_le_bytes());
        put(&mut bytes, 0x2104, &1_u32.to_le_bytes());
        put(&mut bytes, 0x2108, &2_u32.to_le_bytes());
        put(&mut bytes, 0x210c, &2_u32.to_le_bytes());
        put(
            &mut bytes,
            OTHER_COUNTRY,
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, COUNTRY + ccountry::ID, &1_u32.to_le_bytes());
        for country in [COUNTRY, OTHER_COUNTRY] {
            put(
                &mut bytes,
                country + ccountry::RADARS_POOL,
                &(cradars_pool::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(cstate::VTABLE_RVA.0 as u64).to_le_bytes(),
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
                ..Default::default()
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
                ..Default::default()
            }
        );
        assert_eq!(&provinces[..3], [u32::MAX, 1, 13_413]);
    }

    #[test]
    fn symbol_inputs_read_only_the_demanded_cached_radius() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            GROUP + COVERAGE_RADIUS,
            &123_456_i64.to_le_bytes(),
        );
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert_eq!(
            symbol(&memory, GROUP, STATE_A),
            Some(Symbol { radius: 123_456 })
        );
        assert_eq!(symbol(&memory, GROUP, 0), None);
    }

    #[test]
    fn the_count_probe_reports_every_group_and_the_full_demand() {
        let walk = count(&SliceMemory { base: BASE, bytes: &fixture() }, 0, BASE);
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
        put(&mut bytes, GROUP + PROVINCES_COUNT, &9_u32.to_le_bytes());
        let (walk, _, _) = walked(&bytes, 4, 8);
        assert_eq!(
            walk.outcome,
            Outcome::Unavailable {
                reason: "radar coverage vector unreadable or invalid"
            }
        );
        let mut bytes = fixture();
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
