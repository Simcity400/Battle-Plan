//! CNavalBase layout.

/// CNavalBase constructor.
pub const CONSTRUCT_RVA: usize = 0x00e8_a080;
/// Native object allocation size.
pub const SIZE: usize = 0x58;
/// Owning country-holder pointer.
pub const HOLDER: usize = 0x08;
/// Native 32-bit province id.
pub const PROVINCE_ID: usize = 0x10;
/// Native signed i32 healthy port level sum.
pub const HEALTHY_LEVEL: usize = 0x14;
/// Native signed i32 total port level sum.
pub const LEVEL: usize = 0x18;
/// Rebuild cached port levels.
pub const UPDATE_LEVELS_RVA: usize = 0x00e9_cc60;

/// CNavalBase vtable.
pub const VTABLE_RVA: usize = 0x0295_c5a0;
/// Per-base country list: pointer, count and one u32 tag per entry. Membership supplies the
/// prohibition overlay for the viewer.
pub const COUNTRY_LIST: usize = 0x40;
/// Holder's country tag used by known, foreign and enabled branches.
pub const HOLDER_COUNTRY: usize = 0x10;

use crate::{
    Memory, cgame_state,
    cpdx_array::{Array, copy_u32s},
    rail_anchor_selector as map, read,
    walk::Outcome,
};

/// Spell every raw country-list word directly: nonzero indexes live
/// tag strings without link translation; zero uses the database sentinel string.
pub fn country_text(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    country: u32,
) -> Result<Vec<u8>, &'static str> {
    let array = if country == 0 {
        base.checked_add(crate::ccountry_database::SINGLETON_RVA)
            .and_then(|address| read::<u64>(memory, address))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .and_then(|pointer| {
                Array::read(
                    memory,
                    pointer + crate::ccountry_database::TAGS,
                    crate::msvc_string::STRIDE,
                )
            })
    } else {
        game_state
            .checked_add(cgame_state::COUNTRY_TAGS)
            .and_then(|address| Array::read(memory, address, crate::msvc_string::STRIDE))
    }
    .ok_or("naval base raw country tag array unreadable")?;
    let address = array
        .element(country, crate::msvc_string::STRIDE)
        .ok_or("naval base raw country tag is outside its string array")?;
    let mut bytes = Vec::new();
    crate::msvc_string::append_growing(memory, address, &mut bytes, &mut 0)?;
    Ok(bytes)
}

/// Hash entry stride.
const ENTRY_STRIDE: usize = 0x18;
/// Entry occupied marker, its probe distance; the terminal cell holds 0xff.
const ENTRY_DISTANCE: usize = 0x04;
/// Entry key, the province id.
const ENTRY_KEY: usize = 0x08;
/// Entry value, the CNavalBase pointer.
const ENTRY_VALUE: usize = 0x10;
/// The probe-distance sentinel that ends a bucket chain.
const TERMINAL_DISTANCE: u8 = 0xff;
/// Buckets read per bulk copy.
const CHUNK: usize = 512;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub province: u32,
    /// The holder's raw country reference; join it through the country links.
    pub country: u32,
    pub level: i32,
    pub healthy_level: i32,
    /// [`COUNTRY_LIST`] as raw country references, in native order: this many
    /// entries of the walk's country arena from `countries_start`.
    pub countries_start: usize,
    pub countries_len: u32,
    /// The list header held a positive count with a null data pointer, which
    /// the game never writes: this base's list is unknown, not empty.
    pub countries_unreadable: bool,
}

impl Row {
    pub fn key(self) -> u128 {
        u128::from(self.province)
    }
}

/// Rows written, and the country arena's use and complete demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub countries: usize,
    pub required_countries: usize,
}

/// Walk the province-to-naval-base hash held by the strategic navy manager
/// (`cgame_state::STRATEGIC_NAVY_MANAGER`). The hash holds its buckets, mask and maximum probe
/// distance, with 0x18-byte entries selected by set probe bytes. Authenticate each base vtable and
/// require its province id to match the key.
///
/// Keep each complete [`COUNTRY_LIST`] in `countries`, bounded by live count.
/// Short row/list storage omits the base whole and counts full list demand.
/// Read its final entry before requesting growth, so an unreadable list fails
/// the table instead of supplying demand.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut [Row],
    countries: &mut [u32],
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(rows),
        crate::walk::Buffer::Fixed(countries),
    )
}

pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    rows: &mut Vec<Row>,
    countries: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(rows),
        crate::walk::Buffer::Growing(countries),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut rows: crate::walk::Buffer<'_, Row>,
    mut countries: crate::walk::Buffer<'_, u32>,
) -> Walk {
    let mut used = 0_usize;
    let mut required = 0_usize;
    let result = (|| {
        let table = game_state
            .checked_add(cgame_state::STRATEGIC_NAVY_MANAGER)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("naval base map unreadable or null")?;
        let buckets = table
            .checked_add(map::BUCKETS)
            .and_then(|at| read::<u64>(memory, at))
            .and_then(|pointer| usize::try_from(pointer).ok())
            .filter(|&pointer| pointer != 0)
            .ok_or("naval base bucket array unreadable or null")?;
        let mask = table
            .checked_add(map::MASK)
            .and_then(|at| read::<u32>(memory, at))
            .ok_or("naval base map mask unreadable")?;
        let probe = table
            .checked_add(map::MAX_PROBE)
            .and_then(|at| read::<u8>(memory, at))
            .ok_or("naval base map probe unreadable")?;
        let capacity = mask.checked_add(1).ok_or("naval base map mask overflow")?;
        if !capacity.is_power_of_two() || probe == TERMINAL_DISTANCE {
            return Err("naval base map header invalid");
        }
        // The chain ends at one terminal cell past the probe window.
        let cells = capacity as usize + usize::from(probe) + 1;
        let expected = base
            .checked_add(VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut buffer = [0_u8; CHUNK * ENTRY_STRIDE];
        let mut len = 0_usize;
        let mut unread = 0_u32;
        let mut first = 0;
        while first < cells {
            let count = CHUNK.min(cells - first);
            let at = buckets
                .checked_add(first * ENTRY_STRIDE)
                .ok_or("naval base bucket address overflow")?;
            let span = &mut buffer[..count * ENTRY_STRIDE];
            if !memory.copy(at, span) {
                return Err("naval base bucket chunk unreadable");
            }
            for index in 0..count {
                let entry = &span[index * ENTRY_STRIDE..(index + 1) * ENTRY_STRIDE];
                let distance = entry[ENTRY_DISTANCE];
                if distance == 0 || distance == TERMINAL_DISTANCE {
                    continue;
                }
                let province =
                    u32::from_le_bytes(entry[ENTRY_KEY..ENTRY_KEY + 4].try_into().unwrap());
                if province == 0 {
                    // The probe byte marks a chain, not only a resident, so a
                    // cell can carry a distance with no key of its own. Province
                    // zero is not a province, and the payload check below
                    // catches the rest.
                    continue;
                }
                let pointer = usize::try_from(u64::from_le_bytes(
                    entry[ENTRY_VALUE..ENTRY_VALUE + 8].try_into().unwrap(),
                ))
                .map_err(|_| "naval base address overflow")?;
                if pointer == 0 {
                    // A key with no payload is the map's own; it draws nothing.
                    continue;
                }
                if read::<u64>(memory, pointer) != Some(expected) {
                    return Err("naval base vtable mismatch");
                }
                let (mut row, list) = read_row(memory, pointer, province)?;
                required = required
                    .checked_add(list.count as usize)
                    .ok_or("naval base country demand overflows")?;
                let end = used
                    .checked_add(list.count as usize)
                    .ok_or("naval base country demand overflows")?;
                rows.ensure(len.checked_add(1).ok_or("naval base row count overflow")?)?;
                countries.ensure(end)?;
                if len < rows.len() && end <= countries.len() {
                    copy_u32s(
                        memory,
                        list,
                        &mut countries[used..end],
                        "naval base country list data unreadable",
                    )?;
                    row.countries_start = used;
                    row.countries_len = list.count;
                    used = end;
                    rows[len] = row;
                    len += 1;
                } else {
                    if let Some(last) = list.count.checked_sub(1)
                        && list
                            .element(last, 4)
                            .and_then(|at| read::<u32>(memory, at))
                            .is_none()
                    {
                        return Err("naval base country list data unreadable");
                    }
                    unread += 1;
                }
            }
            first += count;
        }
        rows[..len].sort_unstable_by_key(|row| row.province);
        if rows[..len]
            .windows(2)
            .any(|p| p[0].province == p[1].province)
        {
            return Err("duplicate naval base province");
        }
        Ok(if unread == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated {
                len,
                unread_rows: unread,
            }
        })
    })();
    match result {
        Ok(outcome) => Walk {
            outcome,
            countries: used,
            required_countries: required,
        },
        Err(reason) => Walk {
            outcome: Outcome::Unavailable { reason },
            countries: 0,
            required_countries: 0,
        },
    }
}

/// The base's row and its country list; an empty list for a count that is not
/// positive, which the game's country-list membership check never walks. A null list with a
/// positive count marks only this base's list unreadable.
fn read_row(
    memory: &impl Memory,
    pointer: usize,
    province: u32,
) -> Result<(Row, Array), &'static str> {
    let field = |offset: usize| {
        pointer
            .checked_add(offset)
            .ok_or("naval base field address overflow")
    };
    let value = |offset: usize| -> Result<u32, &'static str> {
        read::<u32>(memory, field(offset)?).ok_or("naval base field unreadable")
    };
    if value(PROVINCE_ID)? != province {
        return Err("naval base province differs from its map key");
    }
    let holder = read::<u64>(memory, field(HOLDER)?)
        .and_then(|pointer| usize::try_from(pointer).ok())
        .filter(|&pointer| pointer != 0)
        .ok_or("naval base holder unreadable or null")?;
    let country = holder
        .checked_add(HOLDER_COUNTRY)
        .and_then(|at| read::<u32>(memory, at))
        .ok_or("naval base holder country unreadable")?;
    let row = Row {
        province,
        country,
        level: value(LEVEL)? as i32,
        healthy_level: value(HEALTHY_LEVEL)? as i32,
        ..Row::default()
    };
    // The list is a CPdxArray: data pointer, signed capacity and count. A count
    // that is not positive is empty, as the game's membership check reads it;
    // any other count must pass the shared header check (within its capacity
    // and the address space), so a torn header can never size the arena.
    let header = read::<[u8; 16]>(memory, field(COUNTRY_LIST)?)
        .ok_or("naval base country list unreadable")?;
    let empty = Array { data: 0, count: 0 };
    let count = i32::from_le_bytes(header[12..16].try_into().unwrap());
    if count <= 0 {
        return Ok((row, empty));
    }
    if header[..8] == [0; 8] {
        let row = Row {
            countries_unreadable: true,
            ..row
        };
        return Ok((row, empty));
    }
    let list =
        Array::from_header(header, 4).ok_or("naval base country list header is inconsistent")?;
    Ok((row, list))
}

#[cfg(test)]
mod naval_base_tests {
    use super::*;

    #[test]
    fn serialized_country_text_uses_raw_slots_and_database_zero_at_scale() {
        const STATE: usize = 0x1000;
        const DATABASE: usize = 0x3000;
        const TAGS: usize = 0x5000;
        const COUNT: usize = 5_001;
        struct Tags(Vec<u8>);
        impl Memory for Tags {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == crate::ccountry_database::SINGLETON_RVA && output.len() == 8 {
                    output.copy_from_slice(&(DATABASE as u64).to_le_bytes());
                    return true;
                }
                let Some(source) = address
                    .checked_add(output.len())
                    .and_then(|end| self.0.get(address..end))
                else {
                    return false;
                };
                output.copy_from_slice(source);
                true
            }
        }
        let mut memory = Tags(vec![0; TAGS + COUNT * crate::msvc_string::STRIDE]);
        for (header, data, count) in [
            (STATE + cgame_state::COUNTRY_TAGS, TAGS, COUNT),
            (DATABASE, 0x4000, 1),
        ] {
            memory.0[header..header + 8].copy_from_slice(&(data as u64).to_le_bytes());
            memory.0[header + 8..header + 12].copy_from_slice(&(count as i32).to_le_bytes());
            memory.0[header + 12..header + 16].copy_from_slice(&(count as i32).to_le_bytes());
        }
        for index in 0..COUNT {
            let tag = format!("RAW{index}");
            let at = TAGS + index * crate::msvc_string::STRIDE;
            memory.0[at..at + tag.len()].copy_from_slice(tag.as_bytes());
            memory.0[at + 16..at + 24].copy_from_slice(&(tag.len() as u64).to_le_bytes());
            memory.0[at + 24..at + 32].copy_from_slice(&15_u64.to_le_bytes());
        }
        memory.0[0x4000..0x4008].copy_from_slice(b"DATABASE");
        memory.0[0x4010..0x4018].copy_from_slice(&8_u64.to_le_bytes());
        memory.0[0x4018..0x4020].copy_from_slice(&15_u64.to_le_bytes());
        assert_eq!(country_text(&memory, 0, STATE, 0).unwrap(), b"DATABASE");
        for index in 1..COUNT {
            assert_eq!(
                country_text(&memory, 0, STATE, index as u32).unwrap(),
                format!("RAW{index}").as_bytes()
            );
        }
        assert!(country_text(&memory, 0, STATE, COUNT as u32).is_err());
    }
    use crate::memory::SliceMemory;

    const GS: usize = 0x1000;
    const MAP: usize = 0x2000;
    const BUCKETS: usize = 0x3000;
    const BASE: usize = 0x5000;
    const HOLDER_AT: usize = 0x5200;
    const LIST: usize = 0x5300;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x6000];
        put(
            &mut bytes,
            GS + cgame_state::STRATEGIC_NAVY_MANAGER,
            &(MAP as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MAP + map::BUCKETS,
            &(BUCKETS as u64).to_le_bytes(),
        );
        put(&mut bytes, MAP + map::MASK, &3_u32.to_le_bytes());
        put(&mut bytes, MAP + map::MAX_PROBE, &[2]);
        // One live entry in bucket 1.
        let entry = BUCKETS + ENTRY_STRIDE;
        put(&mut bytes, entry + ENTRY_DISTANCE, &[1]);
        put(&mut bytes, entry + ENTRY_KEY, &158_u32.to_le_bytes());
        put(
            &mut bytes,
            entry + ENTRY_VALUE,
            &(BASE as u64).to_le_bytes(),
        );
        put(&mut bytes, BASE, &(VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, BASE + HOLDER, &(HOLDER_AT as u64).to_le_bytes());
        put(&mut bytes, BASE + PROVINCE_ID, &158_u32.to_le_bytes());
        put(&mut bytes, BASE + LEVEL, &10_u32.to_le_bytes());
        put(&mut bytes, BASE + HEALTHY_LEVEL, &9_u32.to_le_bytes());
        put(
            &mut bytes,
            HOLDER_AT + HOLDER_COUNTRY,
            &102_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + COUNTRY_LIST,
            &(LIST as u64).to_le_bytes(),
        );
        put(&mut bytes, BASE + COUNTRY_LIST + 0x08, &2_u32.to_le_bytes());
        put(&mut bytes, BASE + COUNTRY_LIST + 0x0c, &2_u32.to_le_bytes());
        put(&mut bytes, LIST, &55_u32.to_le_bytes());
        put(&mut bytes, LIST + 4, &77_u32.to_le_bytes());
        bytes
    }

    fn run(bytes: &[u8], rows: &mut [Row]) -> Outcome {
        walk(&SliceMemory { base: 0, bytes }, 0, GS, rows, &mut [0; 64]).outcome
    }

    #[test]
    fn a_live_entry_yields_its_base_levels_and_country_list() {
        let mut rows = [Row::default(); 4];
        let mut countries = [0; 4];
        let bytes = fixture();
        let walked = walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut rows,
            &mut countries,
        );
        assert_eq!(walked.outcome, Outcome::Rows { len: 1 });
        assert_eq!((walked.countries, walked.required_countries), (2, 2));
        assert_eq!(rows[0].province, 158);
        assert_eq!(rows[0].country, 102);
        assert_eq!((rows[0].level, rows[0].healthy_level), (10, 9));
        assert_eq!((rows[0].countries_start, rows[0].countries_len), (0, 2));
        assert_eq!(countries[..2], [55, 77]);
    }

    #[test]
    fn a_list_count_above_its_capacity_is_refused_not_sized() {
        let mut bytes = fixture();
        // A torn header: two entries claimed in a one-entry allocation.
        put(&mut bytes, BASE + COUNTRY_LIST + 0x08, &1_u32.to_le_bytes());
        assert_eq!(
            run(&bytes, &mut [Row::default(); 2]),
            Outcome::Unavailable {
                reason: "naval base country list header is inconsistent"
            }
        );
    }

    #[test]
    fn a_null_list_with_a_positive_count_marks_only_that_base() {
        let mut bytes = fixture();
        put(&mut bytes, BASE + COUNTRY_LIST, &0_u64.to_le_bytes());
        let mut rows = [Row::default(); 2];
        assert_eq!(run(&bytes, &mut rows), Outcome::Rows { len: 1 });
        assert_eq!((rows[0].level, rows[0].healthy_level), (10, 9));
        assert_eq!(rows[0].countries_len, 0);
        assert!(rows[0].countries_unreadable);
    }

    #[test]
    fn empty_cells_and_the_terminal_are_skipped_and_a_full_buffer_truncates() {
        let mut bytes = fixture();
        // Clearing the probe byte makes the cell empty, as the native lookup reads it.
        put(&mut bytes, BUCKETS + ENTRY_STRIDE + ENTRY_DISTANCE, &[0]);
        assert_eq!(
            run(&bytes, &mut [Row::default(); 2]),
            Outcome::Rows { len: 0 }
        );
        assert_eq!(
            run(&fixture(), &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn a_wrong_object_or_a_key_disagreement_is_refused() {
        let mut rows = [Row::default(); 2];
        let mut bytes = fixture();
        put(&mut bytes, BASE, &7_u64.to_le_bytes());
        assert!(matches!(
            run(&bytes, &mut rows),
            Outcome::Unavailable { .. }
        ));
        let mut bytes = fixture();
        put(&mut bytes, BASE + PROVINCE_ID, &159_u32.to_le_bytes());
        assert!(matches!(
            run(&bytes, &mut rows),
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn a_key_with_no_payload_draws_nothing() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            BUCKETS + ENTRY_STRIDE + ENTRY_VALUE,
            &0_u64.to_le_bytes(),
        );
        assert_eq!(
            run(&bytes, &mut [Row::default(); 2]),
            Outcome::Rows { len: 0 }
        );
    }

    /// The whole live list is read, however long; a list the arena cannot hold
    /// omits its base and keeps the exact demand, and a list whose entries
    /// cannot be read fails the table instead of demanding growth.
    #[test]
    fn a_long_country_list_is_read_whole_and_arena_pressure_keeps_its_demand() {
        const LONG: usize = 20_000;
        const LONG_LIST: usize = 0x6000;
        let mut bytes = fixture();
        bytes.resize(LONG_LIST + LONG * 4, 0);
        put(
            &mut bytes,
            BASE + COUNTRY_LIST,
            &(LONG_LIST as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + COUNTRY_LIST + 0x08,
            &(LONG as u32).to_le_bytes(),
        );
        put(
            &mut bytes,
            BASE + COUNTRY_LIST + 0x0c,
            &(LONG as u32).to_le_bytes(),
        );
        for index in 0..LONG {
            put(
                &mut bytes,
                LONG_LIST + index * 4,
                &(index as u32 * 3 + 1).to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [Row::default(); 2];
        let mut countries = vec![0; LONG];
        let walked = walk(&memory, 0, GS, &mut rows, &mut countries);
        assert_eq!(walked.outcome, Outcome::Rows { len: 1 });
        assert_eq!((walked.countries, walked.required_countries), (LONG, LONG));
        assert_eq!(rows[0].countries_len, LONG as u32);
        assert!(
            countries
                .iter()
                .enumerate()
                .all(|(index, raw)| *raw == index as u32 * 3 + 1)
        );

        let walked = walk(&memory, 0, GS, &mut rows, &mut countries[..LONG - 1]);
        assert_eq!(
            walked.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!((walked.countries, walked.required_countries), (0, LONG));

        let short = &bytes[..bytes.len() - 4];
        let memory = SliceMemory {
            base: 0,
            bytes: short,
        };
        for arena in [LONG, 0] {
            let walked = walk(&memory, 0, GS, &mut rows, &mut countries[..arena]);
            assert_eq!(
                walked.outcome,
                Outcome::Unavailable {
                    reason: "naval base country list data unreadable"
                }
            );
        }
        // A count that is not positive is an empty list, as the native test reads it.
        put(
            &mut bytes,
            BASE + COUNTRY_LIST + 0x0c,
            &(-3_i32).to_le_bytes(),
        );
        let walked = walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            GS,
            &mut rows,
            &mut [],
        );
        assert_eq!(walked.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].countries_len, 0);
    }

    #[test]
    fn growing_naval_base_rows_and_country_lists_cover_mod_sized_maps() {
        let mut rows = Vec::new();
        let mut countries = Vec::new();
        for (count, list_len) in [(5000_usize, 2_usize), (5000, 32), (12000, 32)] {
            let capacity = count.next_power_of_two();
            let buckets = 0x10000;
            let bases = buckets + (capacity + 1) * ENTRY_STRIDE;
            let list = bases + count * SIZE;
            let mut bytes = fixture();
            bytes.resize(list + list_len * 4, 0);
            put(
                &mut bytes,
                MAP + map::BUCKETS,
                &(buckets as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                MAP + map::MASK,
                &((capacity - 1) as u32).to_le_bytes(),
            );
            put(&mut bytes, MAP + map::MAX_PROBE, &[0]);
            for index in 0..list_len {
                put(
                    &mut bytes,
                    list + index * 4,
                    &(index as u32 + 1).to_le_bytes(),
                );
            }
            for index in 0..count {
                let base = bases + index * SIZE;
                let entry = buckets + index * ENTRY_STRIDE;
                put(&mut bytes, entry + ENTRY_DISTANCE, &[1]);
                put(
                    &mut bytes,
                    entry + ENTRY_KEY,
                    &(index as u32 + 1).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    entry + ENTRY_VALUE,
                    &(base as u64).to_le_bytes(),
                );
                put(&mut bytes, base, &(VTABLE_RVA as u64).to_le_bytes());
                put(&mut bytes, base + HOLDER, &(HOLDER_AT as u64).to_le_bytes());
                put(
                    &mut bytes,
                    base + PROVINCE_ID,
                    &(index as u32 + 1).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    base + COUNTRY_LIST,
                    &(list as u64).to_le_bytes(),
                );
                for offset in [8, 12] {
                    put(
                        &mut bytes,
                        base + COUNTRY_LIST + offset,
                        &(list_len as u32).to_le_bytes(),
                    );
                }
            }
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            assert_eq!(
                walk(&memory, 0, GS, &mut [], &mut []).outcome,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: count as u32
                }
            );
            let captured = walk_growing(&memory, 0, GS, &mut rows, &mut countries);
            assert_eq!(captured.outcome, Outcome::Rows { len: count });
            assert_eq!(captured.countries, count * list_len);
            assert!(
                rows[..count]
                    .iter()
                    .all(|row| row.countries_len == list_len as u32)
            );
            assert!(
                countries[..captured.countries]
                    .chunks_exact(list_len)
                    .all(|list| list
                        .iter()
                        .enumerate()
                        .all(|(index, raw)| *raw == index as u32 + 1))
            );
        }
    }
}
