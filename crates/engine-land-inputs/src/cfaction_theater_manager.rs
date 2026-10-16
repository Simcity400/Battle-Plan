//! A faction's theatres: the political map draws one pin on each theatre's
//! first strategic region for every faction member whose tag is outside the
//! theatre's hidden set. The pin number counts its countries, its sprite uses
//! the faction miniature icon and its anchor averages all listed regions.

use crate::{
    Memory, cfaction, cfaction_system,
    cpdx_array::{self, Array},
    msvc_string, read,
    walk::Outcome,
};

/// Manager primary vtable.
pub const VTABLE_RVA: usize = 0x02a0_86c0;
/// Owning CFaction pointer.
pub const OWNER: usize = 0x08;
/// Theatre array header: data +0, capacity +8, count +0xC.
/// Serialization and pin visibility use the stored count.
pub const THEATRES: usize = 0x10;
/// Theatre entry stride; theatre serialization and every pin reader step by 0xA8.
pub const THEATRE_STRIDE: usize = 0xa8;
/// Theatre raw-country-tag array; pins use only its count.
pub const COUNTRIES: usize = 0x30;
/// Theatre strategic-region id array, signed i32; first-region lookup
/// compares the initial entry when present.
pub const REGIONS: usize = 0x48;
/// Theatre hidden-pin country tags, sorted by canonical slot.
pub const HIDDEN_FOR: usize = 0x88;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Full native CFaction reference, as in the Factions walk.
    pub faction: u64,
    /// Native index in the theatre array.
    pub ordinal: u32,
    /// Faction icon bytes in the icon arena, shared by the faction's rows.
    pub icon_start: usize,
    pub icon_len: usize,
    /// Raw i32 region ids, bit-preserved, in the region arena.
    pub regions_start: usize,
    pub regions_len: usize,
    pub country_count: u32,
    /// Raw CCountryTag links in the hidden arena, in native order.
    pub hidden_start: usize,
    pub hidden_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub icons_len: usize,
    pub regions_len: usize,
    pub hidden_len: usize,
    /// Demand including rows omitted for any capacity. A faction's icon
    /// counts once, and only when it has a theatre.
    pub required_rows: usize,
    pub required_icons: usize,
    pub required_regions: usize,
    pub required_hidden: usize,
}

impl Walk {
    fn empty(outcome: Outcome) -> Self {
        Self {
            outcome,
            icons_len: 0,
            regions_len: 0,
            hidden_len: 0,
            required_rows: 0,
            required_icons: 0,
            required_regions: 0,
            required_hidden: 0,
        }
    }
}

/// Copies every faction's theatres into preallocated storage without an
/// engine call or allocation. A row that does not fit the row buffer or any
/// arena is omitted whole and counted; its headers remain validated.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    icons: &mut [u8],
    regions: &mut [u32],
    hidden: &mut [u32],
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(icons),
        crate::walk::Buffer::Fixed(regions),
        crate::walk::Buffer::Fixed(hidden),
    )
}

/// Worker storage grows from the original native headers before publication.
pub fn walk_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    icons: &mut Vec<u8>,
    regions: &mut Vec<u32>,
    hidden: &mut Vec<u32>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(icons),
        crate::walk::Buffer::Growing(regions),
        crate::walk::Buffer::Growing(hidden),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut icons: crate::walk::Buffer<'_, u8>,
    mut regions: crate::walk::Buffer<'_, u32>,
    mut hidden: crate::walk::Buffer<'_, u32>,
) -> Walk {
    let mut walk = Walk::empty(Outcome::Rows { len: 0 });
    let result = (|| {
        let roster = cfaction_system::roster(memory, base, game_state)?;
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        for index in 0..roster.count {
            let (faction, id) = cfaction_system::faction(memory, base, roster, index)?;
            let theatres = manager(memory, base, faction)?;
            if theatres.count == 0 {
                continue;
            }
            let icon = faction
                .checked_add(cfaction::ICON)
                .ok_or("faction address overflow")
                .and_then(|at| msvc_string::Descriptor::read(memory, at))
                .map_err(|_| "faction icon string unreadable or invalid")?;
            walk.required_icons = walk
                .required_icons
                .checked_add(icon.len)
                .ok_or("faction icon byte count overflow")?;
            let mut icon_span = None;
            for ordinal in 0..theatres.count {
                let entry = theatres
                    .element(ordinal, THEATRE_STRIDE)
                    .and_then(|at| read::<[u8; THEATRE_STRIDE]>(memory, at))
                    .ok_or("faction theatre unreadable")?;
                let array = |offset: usize| {
                    let header = entry[offset..offset + 16].try_into().expect("16 bytes");
                    Array::from_header(header, 4).ok_or("faction theatre array header invalid")
                };
                let countries = array(COUNTRIES)?;
                let region_ids = array(REGIONS)?;
                let hidden_tags = array(HIDDEN_FOR)?;
                let (region_count, hidden_count) =
                    (region_ids.count as usize, hidden_tags.count as usize);
                walk.required_rows += 1;
                walk.required_regions += region_count;
                walk.required_hidden += hidden_count;
                output.ensure(
                    len.checked_add(1)
                        .ok_or("faction theatre row count overflow")?,
                )?;
                regions.ensure(
                    walk.regions_len
                        .checked_add(region_count)
                        .ok_or("faction theatre region count overflow")?,
                )?;
                hidden.ensure(
                    walk.hidden_len
                        .checked_add(hidden_count)
                        .ok_or("faction theatre hidden count overflow")?,
                )?;
                if icon_span.is_none() {
                    icons.ensure(
                        walk.icons_len
                            .checked_add(icon.len)
                            .ok_or("faction icon byte count overflow")?,
                    )?;
                }
                let fits = len < output.len()
                    && region_count <= regions.len() - walk.regions_len
                    && hidden_count <= hidden.len() - walk.hidden_len
                    && (icon_span.is_some() || icon.len <= icons.len() - walk.icons_len);
                if !fits {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("faction theatre unread count overflow")?;
                    continue;
                }
                let (icon_start, icon_len) = match icon_span {
                    Some(span) => span,
                    None => {
                        let start = walk.icons_len;
                        icon.copy(memory, &mut icons[start..])
                            .map_err(|_| "faction icon bytes unreadable")?;
                        walk.icons_len += icon.len;
                        *icon_span.insert((start, icon.len))
                    }
                };
                let regions_start = walk.regions_len;
                cpdx_array::copy_u32s(
                    memory,
                    region_ids,
                    &mut regions[regions_start..regions_start + region_count],
                    "faction theatre regions unreadable",
                )?;
                walk.regions_len += region_count;
                let hidden_start = walk.hidden_len;
                cpdx_array::copy_u32s(
                    memory,
                    hidden_tags,
                    &mut hidden[hidden_start..hidden_start + hidden_count],
                    "faction theatre hidden tags unreadable",
                )?;
                walk.hidden_len += hidden_count;
                output[len] = Row {
                    faction: id,
                    ordinal,
                    icon_start,
                    icon_len,
                    regions_start,
                    regions_len: region_count,
                    country_count: countries.count,
                    hidden_start,
                    hidden_len: hidden_count,
                };
                len += 1;
            }
        }
        Ok(if unread_rows == 0 {
            Outcome::Rows { len }
        } else {
            Outcome::Truncated { len, unread_rows }
        })
    })();
    match result {
        Ok(outcome) => Walk { outcome, ..walk },
        Err(reason) => Walk::empty(Outcome::Unavailable { reason }),
    }
}

/// The manager is embedded in its faction: authenticate its class and owner
/// before trusting the theatre array header.
fn manager(memory: &impl Memory, base: usize, faction: usize) -> Result<Array, &'static str> {
    let manager = faction
        .checked_add(cfaction::THEATER_MANAGER)
        .ok_or("faction address overflow")?;
    let header = read::<[u8; 0x20]>(memory, manager).ok_or("faction theatre manager unreadable")?;
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().expect("8 bytes"));
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("module address overflow")?;
    if word(0) != expected as u64 {
        return Err("faction theatre manager vtable mismatch");
    }
    if word(OWNER) != faction as u64 {
        return Err("faction theatre manager owner differs from faction");
    }
    Array::from_header(
        header[THEATRES..THEATRES + 16]
            .try_into()
            .expect("16 bytes"),
        THEATRE_STRIDE,
    )
    .ok_or("faction theatre array header invalid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    const GS: usize = 0x1000;
    const SYSTEM: usize = 0x1200;
    const FACTION_POINTERS: usize = 0x1300;
    const FIRST: usize = 0x2000;
    const SECOND: usize = 0x3000;
    const THIRD: usize = 0x4000;
    const THEATRES_A: usize = 0x5000;
    const LISTS: usize = 0x6000;
    const HEAP_ICON: usize = 0x7000;
    const FIRST_ID: u64 = (2_u64 << 32) | 88;
    const SECOND_ID: u64 = (5_u64 << 32) | 88;
    const THIRD_ID: u64 = (9_u64 << 32) | 88;

    fn put32(bytes: &mut [u8], address: usize, value: u32) {
        bytes[address..address + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn put64(bytes: &mut [u8], address: usize, value: u64) {
        bytes[address..address + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn array(bytes: &mut [u8], address: usize, data: usize, capacity: u32, count: u32) {
        put64(bytes, address, data as u64);
        put32(bytes, address + 8, capacity);
        put32(bytes, address + 12, count);
    }

    fn icon(bytes: &mut [u8], faction: usize, value: &[u8]) {
        let string = faction + cfaction::ICON;
        let source = if value.len() <= 15 {
            put64(bytes, string + msvc_string::CAPACITY, 15);
            string
        } else {
            put64(bytes, string, HEAP_ICON as u64);
            put64(bytes, string + msvc_string::CAPACITY, value.len() as u64);
            HEAP_ICON
        };
        put64(bytes, string + msvc_string::LENGTH, value.len() as u64);
        bytes[source..source + value.len()].copy_from_slice(value);
        bytes[source + value.len()] = 0;
    }

    /// Writes a u32 list at `at` and its header at `header`.
    fn list(bytes: &mut [u8], header: usize, at: usize, values: &[u32]) {
        array(
            bytes,
            header,
            at,
            values.len() as u32 + 1,
            values.len() as u32,
        );
        for (index, value) in values.iter().enumerate() {
            put32(bytes, at + index * 4, *value);
        }
    }

    /// Three factions: the first has two theatres (the second with a hidden
    /// set and no regions), the second none, the third one with a heap icon.
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        put64(&mut bytes, GS + cgame_state::FACTION_SYSTEM, SYSTEM as u64);
        put64(&mut bytes, SYSTEM, cfaction_system::VTABLE_RVA as u64);
        array(
            &mut bytes,
            SYSTEM + cfaction_system::FACTIONS,
            FACTION_POINTERS,
            4,
            3,
        );
        for (slot, (faction, id)) in [(FIRST, FIRST_ID), (SECOND, SECOND_ID), (THIRD, THIRD_ID)]
            .into_iter()
            .enumerate()
        {
            put64(&mut bytes, FACTION_POINTERS + slot * 8, faction as u64);
            put64(&mut bytes, faction, cfaction::VTABLE_RVA as u64);
            put64(&mut bytes, faction + cfaction::REFERENCE, id);
            let manager = faction + cfaction::THEATER_MANAGER;
            put64(&mut bytes, manager, VTABLE_RVA as u64);
            put64(&mut bytes, manager + OWNER, faction as u64);
        }
        icon(&mut bytes, FIRST, b"GFX_a");
        icon(&mut bytes, SECOND, b"unused");
        icon(&mut bytes, THIRD, b"GFX_faction_logo_long_name");
        let theatres = [
            (FIRST, THEATRES_A, 2),
            (SECOND, 0, 0),
            (THIRD, THEATRES_A + 2 * THEATRE_STRIDE, 1),
        ];
        for (faction, data, count) in theatres {
            array(
                &mut bytes,
                faction + cfaction::THEATER_MANAGER + THEATRES,
                data,
                count + 1,
                count,
            );
        }
        let entry = |ordinal: usize| THEATRES_A + ordinal * THEATRE_STRIDE;
        list(&mut bytes, entry(0) + COUNTRIES, LISTS, &[11, 12, 13]);
        list(&mut bytes, entry(0) + REGIONS, LISTS + 0x40, &[5, 1, 208]);
        list(&mut bytes, entry(1) + COUNTRIES, LISTS + 0x80, &[14]);
        list(&mut bytes, entry(1) + REGIONS, LISTS + 0xc0, &[]);
        list(
            &mut bytes,
            entry(1) + HIDDEN_FOR,
            LISTS + 0x100,
            &[3, 0x8000_0007],
        );
        list(
            &mut bytes,
            entry(2) + REGIONS,
            LISTS + 0x140,
            &[u32::MAX, 76],
        );
        bytes
    }

    fn run(
        bytes: &[u8],
        rows: &mut [Row],
        icons: &mut [u8],
        regions: &mut [u32],
        hidden: &mut [u32],
    ) -> Walk {
        walk(
            &SliceMemory { base: 0, bytes },
            0,
            GS,
            rows,
            icons,
            regions,
            hidden,
        )
    }

    #[test]
    fn worker_grows_every_theatre_arena_before_first_and_later_publication() {
        let mut output = Vec::new();
        let mut icons = Vec::new();
        let mut regions = Vec::new();
        let mut hidden = Vec::new();
        for count in [5_000_u32, 10_000] {
            let mut bytes = fixture();
            let template =
                bytes[THEATRES_A + THEATRE_STRIDE..THEATRES_A + 2 * THEATRE_STRIDE].to_vec();
            let source = bytes.len();
            for _ in 0..count {
                bytes.extend_from_slice(&template);
            }
            array(
                &mut bytes,
                FIRST + cfaction::THEATER_MANAGER + THEATRES,
                source,
                count,
                count,
            );
            array(
                &mut bytes,
                THIRD + cfaction::THEATER_MANAGER + THEATRES,
                0,
                0,
                0,
            );
            let walk = walk_growing(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                GS,
                &mut output,
                &mut icons,
                &mut regions,
                &mut hidden,
            );
            assert_eq!(
                walk.outcome,
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(walk.hidden_len, count as usize * 2);
            assert_eq!(walk.icons_len, 5);
            assert_eq!(output[count as usize - 1].ordinal, count - 1);
            assert_eq!(
                &hidden[..walk.hidden_len],
                [3, 0x8000_0007].repeat(count as usize)
            );
            assert!(
                matches!(run(&bytes, &mut [], &mut [], &mut [], &mut []).outcome, Outcome::Truncated { unread_rows, .. } if unread_rows == count)
            );
        }
    }

    #[test]
    fn theatres_keep_native_order_raw_ids_counts_and_one_icon_per_faction() {
        let bytes = fixture();
        let mut rows = [Row::default(); 3];
        let mut icons = [0; 31];
        let mut regions = [0; 5];
        let mut hidden = [0; 2];
        let walk = run(&bytes, &mut rows, &mut icons, &mut regions, &mut hidden);
        assert_eq!(
            walk,
            Walk {
                outcome: Outcome::Rows { len: 3 },
                icons_len: 31,
                regions_len: 5,
                hidden_len: 2,
                required_rows: 3,
                required_icons: 31,
                required_regions: 5,
                required_hidden: 2,
            }
        );
        assert_eq!(&icons, b"GFX_aGFX_faction_logo_long_name");
        assert_eq!(regions, [5, 1, 208, u32::MAX, 76]);
        assert_eq!(hidden, [3, 0x8000_0007]);
        assert_eq!(
            rows,
            [
                Row {
                    faction: FIRST_ID,
                    ordinal: 0,
                    icon_start: 0,
                    icon_len: 5,
                    regions_start: 0,
                    regions_len: 3,
                    country_count: 3,
                    hidden_start: 0,
                    hidden_len: 0,
                },
                Row {
                    faction: FIRST_ID,
                    ordinal: 1,
                    icon_start: 0,
                    icon_len: 5,
                    regions_start: 3,
                    regions_len: 0,
                    country_count: 1,
                    hidden_start: 0,
                    hidden_len: 2,
                },
                Row {
                    faction: THIRD_ID,
                    ordinal: 0,
                    icon_start: 5,
                    icon_len: 26,
                    regions_start: 3,
                    regions_len: 2,
                    country_count: 0,
                    hidden_start: 2,
                    hidden_len: 0,
                },
            ]
        );
    }

    #[test]
    fn any_short_buffer_omits_whole_rows_and_reports_full_demand() {
        let bytes = fixture();
        let demand = (3, 31, 5, 2);
        for (rows, icons, regions, hidden, len) in [
            (2, 31, 5, 2, 2),
            (3, 30, 5, 2, 2),
            (3, 4, 5, 2, 0),
            (3, 31, 4, 2, 2),
            (3, 31, 5, 1, 2),
            (0, 0, 0, 0, 0),
        ] {
            let mut row_buffer = vec![Row::default(); rows];
            let walk = run(
                &bytes,
                &mut row_buffer,
                &mut vec![0; icons],
                &mut vec![0; regions],
                &mut vec![0; hidden],
            );
            assert_eq!(
                walk.outcome,
                Outcome::Truncated {
                    len,
                    unread_rows: 3 - len as u32
                },
                "{rows} {icons} {regions} {hidden}"
            );
            assert_eq!(
                (
                    walk.required_rows,
                    walk.required_icons,
                    walk.required_regions,
                    walk.required_hidden
                ),
                demand
            );
            // Every published row stays inside the published arena lengths.
            for row in &row_buffer[..len] {
                assert!(row.icon_start + row.icon_len <= walk.icons_len);
                assert!(row.regions_start + row.regions_len <= walk.regions_len);
                assert!(row.hidden_start + row.hidden_len <= walk.hidden_len);
            }
        }
    }

    #[test]
    fn empty_rosters_and_factions_without_theatres_are_complete_and_empty() {
        let mut bytes = fixture();
        for faction in [FIRST, THIRD] {
            put32(
                &mut bytes,
                faction + cfaction::THEATER_MANAGER + THEATRES + 12,
                0,
            );
            // A faction without theatres does not read its icon.
            put64(
                &mut bytes,
                faction + cfaction::ICON + msvc_string::CAPACITY,
                0,
            );
        }
        let walk = run(&bytes, &mut [], &mut [], &mut [], &mut []);
        assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
        assert_eq!(walk.required_icons, 0);
        array(&mut bytes, SYSTEM + cfaction_system::FACTIONS, 0, 0, 0);
        assert_eq!(
            run(&bytes, &mut [], &mut [], &mut [], &mut []).outcome,
            Outcome::Rows { len: 0 }
        );
    }

    #[test]
    fn wrong_manager_bad_headers_and_unreadable_lists_are_unavailable() {
        let manager = FIRST + cfaction::THEATER_MANAGER;
        let entry = THEATRES_A + THEATRE_STRIDE;
        for (at, value) in [
            (manager, 0),
            (manager + OWNER, SECOND as u64),
            // Capacity one below the count of two.
            (manager + THEATRES + 8, (2_u64 << 32) | 1),
            (manager + THEATRES, u64::MAX - 8),
            (entry + REGIONS + 8, 1_u64 << 32),
            (entry + HIDDEN_FOR, 0),
            (FIRST + cfaction::ICON + msvc_string::LENGTH, 16),
            (FACTION_POINTERS + 16, FIRST as u64),
        ] {
            let mut bytes = fixture();
            put64(&mut bytes, at, value);
            // Headers are checked even when no row fits.
            for size in [0, 8] {
                let walk = run(
                    &bytes,
                    &mut vec![Row::default(); size],
                    &mut vec![0; size * 8],
                    &mut vec![0; size],
                    &mut vec![0; size],
                );
                assert!(
                    matches!(walk.outcome, Outcome::Unavailable { .. }),
                    "case {at:x} size {size}: {walk:?}"
                );
                assert_eq!(walk.required_rows, 0);
            }
        }
        // An unreadable list is found only when its row is copied.
        let mut bytes = fixture();
        put64(&mut bytes, entry + HIDDEN_FOR, 0x10_0000);
        assert_eq!(
            run(&bytes, &mut [], &mut [], &mut [], &mut []).outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 3
            }
        );
        assert!(matches!(
            run(
                &bytes,
                &mut [Row::default(); 3],
                &mut [0; 31],
                &mut [0; 5],
                &mut [0; 2]
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
