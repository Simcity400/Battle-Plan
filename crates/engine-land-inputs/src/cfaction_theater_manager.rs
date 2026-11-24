
use engine_native_memory::{
    Memory,
    cpdx_array::{self, Array},
    msvc_string,
    read,
    walk::Outcome,
};
use engine_native::cfaction::{self, system as cfaction_system};
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x02a9_8778);
pub const OWNER: usize = 0x08;
pub const THEATRES: usize = 0x10;
pub const THEATRE_STRIDE: usize = 0xa8;
pub const NAME: usize = 0x00;
pub const THEATRE_OWNER: usize = 0x20;
pub const COMMANDER: usize = 0x24;
pub const COUNTRIES: usize = 0x30;
pub const REGIONS: usize = 0x48;
pub const HIDDEN_FOR: usize = 0x88;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub faction: u64,
    pub ordinal: u32,
    pub icon_start: usize,
    pub icon_len: usize,
    pub name_start: usize,
    pub name_len: usize,
    pub owner: u32,
    pub commander: u64,
    pub countries_start: usize,
    pub countries_len: usize,
    pub regions_start: usize,
    pub regions_len: usize,
    pub country_count: u32,
    pub hidden_start: usize,
    pub hidden_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub outcome: Outcome,
    pub icons_len: usize,
    pub names_len: usize,
    pub countries_len: usize,
    pub regions_len: usize,
    pub hidden_len: usize,
    pub required_rows: usize,
    pub required_icons: usize,
    pub required_names: usize,
    pub required_countries: usize,
    pub required_regions: usize,
    pub required_hidden: usize,
}

impl Walk {
    fn empty(outcome: Outcome) -> Self {
        Self {
            outcome,
            icons_len: 0,
            names_len: 0,
            countries_len: 0,
            regions_len: 0,
            hidden_len: 0,
            required_rows: 0,
            required_icons: 0,
            required_names: 0,
            required_countries: 0,
            required_regions: 0,
            required_hidden: 0,
        }
    }
}

pub struct Arenas<'a> {
    pub rows: engine_native_memory::walk::Buffer<'a, Row>,
    pub icons: engine_native_memory::walk::Buffer<'a, u8>,
    pub names: engine_native_memory::walk::Buffer<'a, u8>,
    pub countries: engine_native_memory::walk::Buffer<'a, u32>,
    pub regions: engine_native_memory::walk::Buffer<'a, u32>,
    pub hidden: engine_native_memory::walk::Buffer<'a, u32>,
}

pub struct Storage<'a> {
    pub rows: &'a mut Vec<Row>,
    pub icons: &'a mut Vec<u8>,
    pub names: &'a mut Vec<u8>,
    pub countries: &'a mut Vec<u32>,
    pub regions: &'a mut Vec<u32>,
    pub hidden: &'a mut Vec<u32>,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    use engine_native_memory::walk::Buffer::Count;
    walk_buffer(
        memory,
        base,
        game_state,
        Arenas {
            rows: Count,
            icons: Count,
            names: Count,
            countries: Count,
            regions: Count,
            hidden: Count,
        },
    )
}

pub fn walk(memory: &impl Memory, base: usize, game_state: usize, storage: Storage<'_>) -> Walk {
    use engine_native_memory::walk::Buffer::Growing;
    walk_buffer(
        memory,
        base,
        game_state,
        Arenas {
            rows: Growing(storage.rows),
            icons: Growing(storage.icons),
            names: Growing(storage.names),
            countries: Growing(storage.countries),
            regions: Growing(storage.regions),
            hidden: Growing(storage.hidden),
        },
    )
}

pub fn walk_buffer(memory: &impl Memory, base: usize, game_state: usize, arenas: Arenas<'_>) -> Walk {
    let Arenas {
        rows: mut output,
        mut icons,
        mut names,
        mut countries,
        mut regions,
        mut hidden,
    } = arenas;
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
                let at = theatres
                    .element(ordinal, THEATRE_STRIDE)
                    .ok_or("faction theatre unreadable")?;
                let entry =
                    read::<[u8; THEATRE_STRIDE]>(memory, at).ok_or("faction theatre unreadable")?;
                let array = |offset: usize| {
                    let header = entry[offset..offset + 16].try_into().expect("16 bytes");
                    Array::from_header(header, 4).ok_or("faction theatre array header invalid")
                };
                let word = |offset: usize| {
                    u32::from_le_bytes(entry[offset..offset + 4].try_into().expect("4 bytes"))
                };
                let name = msvc_string::Descriptor::from_header(
                    memory,
                    at.checked_add(NAME).ok_or("faction theatre address overflow")?,
                    entry[NAME..NAME + msvc_string::STRIDE]
                        .try_into()
                        .expect("string header"),
                )
                .map_err(|_| "faction theatre name unreadable or invalid")?;
                let owner = word(THEATRE_OWNER);
                let commander = u64::from(word(COMMANDER)) | u64::from(word(COMMANDER + 4)) << 32;
                let member_tags = array(COUNTRIES)?;
                let region_ids = array(REGIONS)?;
                let hidden_tags = array(HIDDEN_FOR)?;
                let (name_len, country_count, region_count, hidden_count) = (
                    name.len,
                    member_tags.count as usize,
                    region_ids.count as usize,
                    hidden_tags.count as usize,
                );
                walk.required_rows += 1;
                walk.required_names = walk
                    .required_names
                    .checked_add(name_len)
                    .ok_or("faction theatre name byte count overflow")?;
                walk.required_countries += country_count;
                walk.required_regions += region_count;
                walk.required_hidden += hidden_count;
                output.ensure(
                    len.checked_add(1)
                        .ok_or("faction theatre row count overflow")?,
                )?;
                names.ensure(
                    walk.names_len
                        .checked_add(name_len)
                        .ok_or("faction theatre name byte count overflow")?,
                )?;
                countries.ensure(
                    walk.countries_len
                        .checked_add(country_count)
                        .ok_or("faction theatre country count overflow")?,
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
                    && name_len <= names.len() - walk.names_len
                    && country_count <= countries.len() - walk.countries_len
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
                let name_start = walk.names_len;
                name.copy(memory, &mut names[name_start..name_start + name_len])
                    .map_err(|_| "faction theatre name bytes unreadable")?;
                walk.names_len += name_len;
                let countries_start = walk.countries_len;
                cpdx_array::copy_u32s(
                    memory,
                    member_tags,
                    &mut countries[countries_start..countries_start + country_count],
                    "faction theatre countries unreadable",
                )?;
                walk.countries_len += country_count;
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
                    name_start,
                    name_len,
                    owner,
                    commander,
                    countries_start,
                    countries_len: country_count,
                    regions_start,
                    regions_len: region_count,
                    country_count: member_tags.count,
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

fn manager(memory: &impl Memory, base: usize, faction: usize) -> Result<Array, &'static str> {
    let manager = faction
        .checked_add(cfaction::THEATER_MANAGER)
        .ok_or("faction address overflow")?;
    let header = read::<[u8; 0x20]>(memory, manager).ok_or("faction theatre manager unreadable")?;
    let word = |at: usize| u64::from_le_bytes(header[at..at + 8].try_into().expect("8 bytes"));
    let expected = engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")?;
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
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;

    const GS: usize = 0x1000;
    const SYSTEM: usize = 0x1200;
    const FACTION_POINTERS: usize = 0x1300;
    const FIRST: usize = 0x2000;
    const SECOND: usize = 0x3000;
    const THIRD: usize = 0x4000;
    const THEATRES_A: usize = 0x5000;
    const LISTS: usize = 0x6000;
    const HEAP_ICON: usize = 0x7000;
    const HEAP_NAME: usize = 0x7100;
    const FIRST_ID: u64 = (2_u64 << 32) | 88;
    const SECOND_ID: u64 = (5_u64 << 32) | 88;
    const THIRD_ID: u64 = (9_u64 << 32) | 88;
    const COMMANDER_REF: u64 = (844_u64 << 32) | 4713;

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

    fn string(bytes: &mut [u8], string: usize, heap: usize, value: &[u8]) {
        let source = if value.len() <= 15 {
            put64(bytes, string + msvc_string::CAPACITY, 15);
            string
        } else {
            put64(bytes, string, heap as u64);
            put64(bytes, string + msvc_string::CAPACITY, value.len() as u64);
            heap
        };
        put64(bytes, string + msvc_string::LENGTH, value.len() as u64);
        bytes[source..source + value.len()].copy_from_slice(value);
        bytes[source + value.len()] = 0;
    }

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

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x8000];
        put64(&mut bytes, GS + cgame_state::FACTION_SYSTEM, SYSTEM as u64);
        put64(&mut bytes, SYSTEM, cfaction_system::VTABLE_RVA.0 as u64);
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
            put64(&mut bytes, faction, cfaction::VTABLE_RVA.0 as u64);
            put64(&mut bytes, faction + cfaction::REFERENCE, id);
            let manager = faction + cfaction::THEATER_MANAGER;
            put64(&mut bytes, manager, VTABLE_RVA.0 as u64);
            put64(&mut bytes, manager + OWNER, faction as u64);
        }
        string(&mut bytes, FIRST + cfaction::ICON, HEAP_ICON, b"GFX_a");
        string(&mut bytes, SECOND + cfaction::ICON, HEAP_ICON, b"unused");
        string(
            &mut bytes,
            THIRD + cfaction::ICON,
            HEAP_ICON,
            b"GFX_faction_logo_long_name",
        );
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
        string(&mut bytes, entry(0) + NAME, HEAP_NAME, b"Western Front");
        string(&mut bytes, entry(1) + NAME, HEAP_NAME, b"");
        string(
            &mut bytes,
            entry(2) + NAME,
            HEAP_NAME,
            b"Pacific Ocean Theatre",
        );
        put32(&mut bytes, entry(0) + THEATRE_OWNER, 11);
        put32(&mut bytes, entry(1) + THEATRE_OWNER, 14);
        put32(&mut bytes, entry(2) + THEATRE_OWNER, 0x8000_0002);
        put32(&mut bytes, entry(0) + COMMANDER, 4713);
        put32(&mut bytes, entry(0) + COMMANDER + 4, 844);
        put32(&mut bytes, entry(2) + COMMANDER, 4713);
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

    #[derive(Default)]
    struct Out {
        rows: Vec<Row>,
        icons: Vec<u8>,
        names: Vec<u8>,
        countries: Vec<u32>,
        regions: Vec<u32>,
        hidden: Vec<u32>,
    }

    impl Out {
        fn sized(size: [usize; 6]) -> Self {
            Self {
                rows: vec![Row::default(); size[0]],
                icons: vec![0; size[1]],
                names: vec![0; size[2]],
                countries: vec![0; size[3]],
                regions: vec![0; size[4]],
                hidden: vec![0; size[5]],
            }
        }

        fn run(&mut self, bytes: &[u8]) -> Walk {
            walk(
                &SliceMemory { base: 0, bytes },
                0,
                GS,
                Storage {
                    rows: &mut self.rows,
                    icons: &mut self.icons,
                    names: &mut self.names,
                    countries: &mut self.countries,
                    regions: &mut self.regions,
                    hidden: &mut self.hidden,
                },
            )
        }
    }

    #[test]
    fn worker_grows_every_theatre_arena_before_first_and_later_publication() {
        let mut out = Out::default();
        for count in [5_000_u32, 10_000] {
            let mut bytes = fixture();
            let template =
                bytes[THEATRES_A + THEATRE_STRIDE..THEATRES_A + 2 * THEATRE_STRIDE].to_vec();
            let source = bytes.len();
            for _ in 0..count {
                bytes.extend_from_slice(&template);
            }
            for copy in 0..count as usize {
                string(
                    &mut bytes,
                    source + copy * THEATRE_STRIDE + NAME,
                    0,
                    b"North",
                );
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
            let walk = out.run(&bytes);
            assert_eq!(
                walk.outcome,
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(walk.hidden_len, count as usize * 2);
            assert_eq!(walk.countries_len, count as usize);
            assert_eq!(walk.names_len, count as usize * 5);
            assert_eq!(walk.icons_len, 5);
            assert_eq!(out.rows[count as usize - 1].ordinal, count - 1);
            assert_eq!(
                &out.hidden[..walk.hidden_len],
                [3, 0x8000_0007].repeat(count as usize)
            );
            assert_eq!(&out.countries[..walk.countries_len], [14].repeat(count as usize));
            assert_eq!(&out.names[..walk.names_len], b"North".repeat(count as usize));
            assert!(
                matches!(super::count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS).outcome, Outcome::Truncated { unread_rows, .. } if unread_rows == count)
            );
        }
    }

    #[test]
    fn theatres_keep_native_order_raw_ids_counts_and_one_icon_per_faction() {
        let bytes = fixture();
        let mut out = Out::sized([3, 31, 34, 4, 5, 2]);
        let walk = out.run(&bytes);
        assert_eq!(
            walk,
            Walk {
                outcome: Outcome::Rows { len: 3 },
                icons_len: 31,
                names_len: 34,
                countries_len: 4,
                regions_len: 5,
                hidden_len: 2,
                required_rows: 3,
                required_icons: 31,
                required_names: 34,
                required_countries: 4,
                required_regions: 5,
                required_hidden: 2,
            }
        );
        assert_eq!(&out.icons, b"GFX_aGFX_faction_logo_long_name");
        assert_eq!(&out.names, b"Western FrontPacific Ocean Theatre");
        assert_eq!(out.countries, [11, 12, 13, 14]);
        assert_eq!(out.regions, [5, 1, 208, u32::MAX, 76]);
        assert_eq!(out.hidden, [3, 0x8000_0007]);
        assert_eq!(
            out.rows,
            [
                Row {
                    faction: FIRST_ID,
                    ordinal: 0,
                    icon_start: 0,
                    icon_len: 5,
                    name_start: 0,
                    name_len: 13,
                    owner: 11,
                    commander: COMMANDER_REF,
                    countries_start: 0,
                    countries_len: 3,
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
                    name_start: 13,
                    name_len: 0,
                    owner: 14,
                    commander: 0,
                    countries_start: 3,
                    countries_len: 1,
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
                    name_start: 13,
                    name_len: 21,
                    owner: 0x8000_0002,
                    commander: 4713,
                    countries_start: 4,
                    countries_len: 0,
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
    fn empty_rosters_and_factions_without_theatres_are_complete_and_empty() {
        let mut bytes = fixture();
        for faction in [FIRST, THIRD] {
            put32(
                &mut bytes,
                faction + cfaction::THEATER_MANAGER + THEATRES + 12,
                0,
            );
            put64(
                &mut bytes,
                faction + cfaction::ICON + msvc_string::CAPACITY,
                0,
            );
        }
        let walk = Out::default().run(&bytes);
        assert_eq!(walk.outcome, Outcome::Rows { len: 0 });
        assert_eq!(walk.required_icons, 0);
        array(&mut bytes, SYSTEM + cfaction_system::FACTIONS, 0, 0, 0);
        assert_eq!(Out::default().run(&bytes).outcome, Outcome::Rows { len: 0 });
    }

    #[test]
    fn wrong_manager_bad_headers_and_unreadable_lists_are_unavailable() {
        let manager = FIRST + cfaction::THEATER_MANAGER;
        let entry = THEATRES_A + THEATRE_STRIDE;
        for (at, value) in [
            (manager, 0),
            (manager + OWNER, SECOND as u64),
            (manager + THEATRES + 8, (2_u64 << 32) | 1),
            (manager + THEATRES, u64::MAX - 8),
            (entry + REGIONS + 8, 1_u64 << 32),
            (entry + HIDDEN_FOR, 0),
            (entry + COUNTRIES, 0),
            (entry + NAME + msvc_string::CAPACITY, 3),
            (THEATRES_A + 2 * THEATRE_STRIDE + NAME, 0x10_0000),
            (FIRST + cfaction::ICON + msvc_string::LENGTH, 16),
            (FACTION_POINTERS + 16, FIRST as u64),
        ] {
            let mut bytes = fixture();
            put64(&mut bytes, at, value);
            for size in [0, 8] {
                let walk = Out::sized([size, size * 8, size * 8, size, size, size]).run(&bytes);
                assert!(
                    matches!(walk.outcome, Outcome::Unavailable { .. }),
                    "case {at:x} size {size}: {walk:?}"
                );
                assert_eq!(walk.required_rows, 0);
            }
        }
        for list in [HIDDEN_FOR, COUNTRIES] {
            let mut bytes = fixture();
            put64(&mut bytes, entry + list, 0x10_0000);
            assert_eq!(
                count(&SliceMemory { base: 0, bytes: &bytes }, 0, GS).outcome,
                Outcome::Truncated {
                    len: 0,
                    unread_rows: 3
                }
            );
            assert!(matches!(
                Out::sized([3, 31, 34, 4, 5, 2]).run(&bytes).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }
}
