
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};
use engine_native::ccountry;
use engine_sites::Rva;

pub const VTABLE_RVA: Rva = Rva(0x0285_be00);
pub const REFERENCE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const AIRCRAFT_TYPE_MASK: usize = 0x18;
pub const MODIFIERS: usize = 0x30;
pub const OWNER_COUNTRY: usize = 0xe0;
pub const ASSIGNED_WING_REFERENCE: usize = 0xe8;
pub const NAME: usize = 0xf0;
pub const SURNAME: usize = 0x110;
pub const CALLSIGN: usize = 0x130;
pub const PORTRAIT_GENDER: usize = 0x150;
#[cfg(any(test, feature = "test-support"))]
pub const PORTRAIT_POOL_INDEX: usize = 0x154;
pub const ALIVE: usize = 0x158;
pub const KILL_TYPE: usize = 0x15c;
pub const KILLER_ACE_REFERENCE: usize = 0x160;
pub const KILLER_COUNTRY: usize = 0x168;
pub const HANDLED: usize = 0x16c;
pub const TYPE_DEFINITION: usize = 0x170;
pub const TYPE_NAME: usize = 0x08;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TextRange {
    pub start: usize,
    pub len: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Death {
    pub killer_ace: u64,
    pub killer_country: u32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Presentation {
    pub gender: u8,
    pub pool_index: i32,
    pub definition: Option<TextRange>,
    pub portrait: Option<Portrait>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Portrait {
    pub cached: Option<TextRange>,
    pub culture: Option<TextRange>,
    pub fallback: Option<TextRange>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub country: u32,
    pub ordinal: u32,
    pub aircraft_type_mask: u32,
    pub assigned_wing: u64,
    pub alive: u8,
    pub handled: u8,
    pub kill_type: u32,
    pub death: Option<Death>,
    pub name: TextRange,
    pub surname: TextRange,
    pub callsign: TextRange,
    pub presentation: Option<Presentation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_buffer(memory, base, game_state, engine_native_memory::walk::Buffer::Count, engine_native_memory::walk::Buffer::Count)
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> NameWalk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut found_country = false;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            found_country = true;
            let roster = country
                .pointer
                .checked_add(ccountry::ACES)
                .and_then(|p| Array::read(memory, p, 8))
                .ok_or("ace roster unreadable or invalid")?;
            output.ensure(
                len.checked_add(roster.count as usize)
                    .ok_or("ace row count overflow")?,
            )?;
            for ordinal in 0..roster.count {
                let pointer = roster
                    .element(ordinal, 8)
                    .and_then(|p| read::<u64>(memory, p))
                    .and_then(|p| usize::try_from(p).ok())
                    .filter(|&p| p != 0)
                    .ok_or("ace roster pointer unreadable or null")?;
                let mut headers = [0_u8; 96];
                let mut row = read_row(memory, base, country, pointer, ordinal, &mut headers)?;
                let strings = [
                    literal(&headers[..32], pointer.checked_add(NAME))?,
                    literal(&headers[32..64], pointer.checked_add(SURNAME))?,
                    literal(&headers[64..], pointer.checked_add(CALLSIGN))?,
                ];
                let mut presentation = presentation(memory, base, country, pointer).ok();
                let row_bytes = strings.iter().try_fold(0_usize, |sum, string| {
                    sum.checked_add(string.len)
                        .ok_or("ace name byte count overflow")
                })?;
                let extra_bytes = if let Some((_, names)) = &presentation {
                    names
                        .iter()
                        .flatten()
                        .try_fold(0_usize, |sum, name| sum.checked_add(name.len))
                        .ok_or("ace name byte count overflow")?
                } else {
                    0
                };
                let total_bytes = row_bytes
                    .checked_add(extra_bytes)
                    .ok_or("ace name byte count overflow")?;
                required_bytes = required_bytes
                    .checked_add(total_bytes)
                    .ok_or("ace name byte count overflow")?;
                names.ensure(required_bytes)?;
                if len == output.len() || row_bytes > names.len() - bytes_len {
                    unread_rows = unread_rows
                        .checked_add(1)
                        .ok_or("ace unread row count overflow")?;
                    continue;
                }
                for (index, (string, range)) in strings
                    .iter()
                    .zip([&mut row.name, &mut row.surname, &mut row.callsign])
                    .enumerate()
                {
                    let end = bytes_len + string.len;
                    if string.inline {
                        names[bytes_len..end]
                            .copy_from_slice(&headers[index * 32..index * 32 + string.len]);
                    } else if read::<u8>(memory, string.source + string.len) != Some(0)
                        || !read_slice(memory, string.source, &mut names[bytes_len..end])
                    {
                        return Err("ace name bytes or terminator unreadable or invalid");
                    }
                    *range = TextRange {
                        start: bytes_len,
                        len: string.len,
                    };
                    bytes_len = end;
                }
                if extra_bytes <= names.len() - bytes_len
                    && let Some((mut fields, extra_names)) = presentation.take()
                {
                    let mut ranges = [None; 4];
                    let mut complete = true;
                    for (index, source) in extra_names.into_iter().enumerate() {
                        let Some(source) = source else {
                            continue;
                        };
                        let end = bytes_len + source.len;
                        if source.copy(memory, &mut names[bytes_len..end]).is_err() {
                            if index == 3 {
                                continue;
                            }
                            complete = false;
                            break;
                        }
                        ranges[index] = Some(TextRange {
                            start: bytes_len,
                            len: source.len,
                        });
                        bytes_len = end;
                    }
                    if complete {
                        fields.definition = ranges[0];
                        if let Some(portrait) = &mut fields.portrait {
                            portrait.cached = ranges[1];
                            portrait.culture = ranges[2];
                            portrait.fallback = ranges[3];
                        }
                        row.presentation = Some(fields);
                    }
                }
                output[len] = row;
                len += 1;
            }
        }
        if !found_country {
            return Err("country table has no readable countries");
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

fn presentation(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    pointer: usize,
) -> Result<(Presentation, [Option<msvc_string::Descriptor>; 4]), &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("ace presentation address overflow")
    };
    let mut portrait = [0; 8];
    let mut selected = [0; 8];
    if memory.copy_scattered(
        [at(PORTRAIT_GENDER)?, at(TYPE_DEFINITION)?],
        &mut [&mut portrait, &mut selected],
    ) != 2
    {
        return Err("ace presentation operands unreadable");
    }
    let pointer = usize::try_from(u64::from_le_bytes(selected))
        .map_err(|_| "ace definition pointer overflow")?;
    let definition = if pointer == 0 {
        None
    } else {
        Some(msvc_string::Descriptor::read(
            memory,
            pointer
                .checked_add(TYPE_NAME)
                .ok_or("ace definition name address overflow")?,
        )?)
    };
    let gender = portrait[0];
    let pool_index = i32::from_le_bytes(portrait[4..].try_into().unwrap());
    let sources = portrait_sources(memory, base, country, gender, pool_index).ok();
    let (portrait, extra) = match sources {
        Some(names) => (Some(Portrait::default()), names),
        None => (None, [None, None, None]),
    };
    let [cached, culture, fallback] = extra;
    Ok((
        Presentation {
            gender,
            pool_index,
            definition: None,
            portrait,
        },
        [definition, cached, culture, fallback],
    ))
}

pub fn portrait_sources(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    gender: u8,
    index: i32,
) -> Result<[Option<msvc_string::Descriptor>; 3], &'static str> {
    let gui = read::<u64>(
        memory,
        engine_sites::at(base, engine_native_politics::cgui::SINGLETON_RVA)
            .ok_or("ace GUI singleton address overflow")?,
    )
    .and_then(|p| usize::try_from(p).ok())
    .filter(|p| *p != 0)
    .ok_or("ace GUI not available")?;
    if read::<u64>(memory, gui)
        != engine_sites::at(base, engine_native_politics::cgame_gui::VTABLE_RVA).map(|p| p as u64)
    {
        return Err("ace GUI identity mismatch");
    }
    let offset = if gender == 0 { 0xc98 } else { 0xcb0 };
    let countries = Array::read(
        memory,
        gui.checked_add(offset).ok_or("ace pool address overflow")?,
        engine_native_memory::cpdx_array::STRIDE,
    )
    .ok_or("ace country pools unreadable or invalid")?;
    let pool_at = countries
        .element(country.canonical_slot, engine_native_memory::cpdx_array::STRIDE)
        .ok_or("ace country outside portrait pools")?;
    let pool = Array::read(memory, pool_at, msvc_string::STRIDE)
        .ok_or("ace country portrait pool unreadable or invalid")?;
    let selected = |pool: Array| -> Result<Option<msvc_string::Descriptor>, &'static str> {
        if pool.count == 0 {
            return Ok(None);
        }
        let count = i32::try_from(pool.count).map_err(|_| "ace portrait pool count overflow")?;
        let ordinal = index % count;
        let ordinal = u32::try_from(ordinal)
            .map_err(|_| "ace portrait index has negative native remainder")?;
        Ok(Some(msvc_string::Descriptor::read(
            memory,
            pool.element(ordinal, msvc_string::STRIDE)
                .ok_or("ace selected portrait address overflow")?,
        )?))
    };
    if pool.count != 0 {
        return Ok([selected(pool)?, None, None]);
    }
    let culture = msvc_string::Descriptor::read(
        memory,
        country
            .pointer
            .checked_add(ccountry::GRAPHICAL_CULTURE_2D)
            .ok_or("ace culture address overflow")?,
    )?;
    let fallback = countries
        .element(0, engine_native_memory::cpdx_array::STRIDE)
        .and_then(|p| Array::read(memory, p, msvc_string::STRIDE))
        .and_then(|pool| selected(pool).ok().flatten());
    Ok([None, Some(culture), fallback])
}

fn read_row(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
    pointer: usize,
    ordinal: u32,
    headers: &mut [u8; 96],
) -> Result<Row, &'static str> {
    let at = |offset| pointer.checked_add(offset).ok_or("ace address overflow");
    let mut identity = [0_u8; 16];
    let mut mask = [0_u8; 4];
    let mut owner_wing = [0_u8; 16];
    let mut alive = [0_u8; 1];
    let mut kill_type = [0_u8; 4];
    let mut handled = [0_u8; 1];
    let copied = memory.copy_scattered(
        [
            pointer,
            at(AIRCRAFT_TYPE_MASK)?,
            at(OWNER_COUNTRY)?,
            at(NAME)?,
            at(ALIVE)?,
            at(KILL_TYPE)?,
            at(HANDLED)?,
        ],
        &mut [
            &mut identity,
            &mut mask,
            &mut owner_wing,
            headers,
            &mut alive,
            &mut kill_type,
            &mut handled,
        ],
    );
    if copied == 0
        || le64(&identity, 0)
            != engine_sites::at(base, VTABLE_RVA).ok_or("module address overflow")? as u64
    {
        return Err("ace vtable unreadable or mismatch");
    }
    let id = le64(&identity, REFERENCE);
    if id == 0 {
        return Err("non-null ace has null reference");
    }
    if copied < 7 {
        return Err("ace fields unreadable");
    }
    if le64(&owner_wing, 0) != country.pointer as u64 {
        return Err("ace owner differs from roster country");
    }
    if alive[0] > 1 || handled[0] > 1 {
        return Err("ace alive or handled flag invalid");
    }
    let death = if alive[0] == 0 {
        let fields = read::<[u8; 12]>(memory, at(KILLER_ACE_REFERENCE)?)
            .ok_or("dead ace killer fields unreadable")?;
        Some(Death {
            killer_ace: le64(&fields, 0),
            killer_country: le32(&fields, 8),
        })
    } else {
        None
    };
    Ok(Row {
        id,
        country: country.id,
        ordinal,
        aircraft_type_mask: u32::from_le_bytes(mask),
        assigned_wing: le64(&owner_wing, 8),
        alive: alive[0],
        handled: handled[0],
        kill_type: u32::from_le_bytes(kill_type),
        death,
        ..Row::default()
    })
}

struct Literal {
    source: usize,
    len: usize,
    inline: bool,
}

fn literal(header: &[u8], address: Option<usize>) -> Result<Literal, &'static str> {
    let address = address.ok_or("ace name address overflow")?;
    let len = le64(header, msvc_string::LENGTH);
    let capacity = le64(header, msvc_string::CAPACITY);
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("ace name header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "ace name length overflow")?;
    let inline = capacity == msvc_string::INLINE_CAPACITY;
    let source = if inline {
        if header[len] != 0 {
            return Err("ace inline name terminator invalid");
        }
        address
    } else {
        usize::try_from(le64(header, msvc_string::DATA))
            .ok()
            .filter(|&p| p != 0)
            .ok_or("ace name pointer null or overflow")?
    };
    source.checked_add(len).ok_or("ace name address overflow")?;
    Ok(Literal {
        source,
        len,
        inline,
    })
}

fn le64(bytes: &[u8], start: usize) -> u64 {
    u64::from_le_bytes(std::array::from_fn(|i| bytes[start + i]))
}

fn le32(bytes: &[u8], start: usize) -> u32 {
    u32::from_le_bytes(std::array::from_fn(|i| bytes[start + i]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use std::collections::BTreeMap;

    const GS: usize = 0x1000;
    const COUNTRIES: usize = 0x2000;
    const LINKS: usize = 0x2100;
    const COUNTRY: usize = 0x4000;
    const ROSTER: usize = 0x5800;
    const ACE: [usize; 2] = [0x6000, 0x7000];
    const IDS: [u64; 2] = [0x8000_0004_ffff_ffff, 0x0000_0006_0000_000a];

    #[derive(Default)]
    struct Synthetic(BTreeMap<usize, u8>);

    impl Memory for Synthetic {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            for (index, byte) in destination.iter_mut().enumerate() {
                let Some(value) = self.0.get(&(address + index)) else {
                    return false;
                };
                *byte = *value;
            }
            true
        }
    }

    impl Synthetic {
        fn bytes(&mut self, address: usize, bytes: &[u8]) {
            self.0
                .extend(bytes.iter().enumerate().map(|(i, b)| (address + i, *b)));
        }
        fn u32(&mut self, address: usize, value: u32) {
            self.bytes(address, &value.to_le_bytes());
        }
        fn u64(&mut self, address: usize, value: u64) {
            self.bytes(address, &value.to_le_bytes());
        }
        fn array(&mut self, address: usize, data: usize, count: u32) {
            self.u64(address, data as u64);
            self.u32(address + 8, count);
            self.u32(address + 12, count);
        }
        fn inline(&mut self, address: usize, value: &[u8]) {
            self.bytes(address, &[0; 32]);
            self.bytes(address, value);
            self.u64(address + 16, value.len() as u64);
            self.u64(address + 24, 15);
        }
    }

    fn fixture() -> Synthetic {
        let mut memory = Synthetic::default();
        memory.array(GS + cgame_state::COUNTRIES, COUNTRIES, 3);
        memory.array(GS + cgame_state::COUNTRY_LINKS, LINKS, 3);
        memory.u64(COUNTRIES + 8, COUNTRY as u64);
        memory.u64(COUNTRIES + 16, COUNTRY as u64);
        memory.u32(LINKS + 4, 1);
        memory.u32(LINKS + 8, 1);
        memory.u64(COUNTRY, ccountry::VTABLE_RVA.0 as u64);
        memory.u32(COUNTRY + ccountry::ID, 2);
        memory.array(COUNTRY + ccountry::ACES, ROSTER, 2);
        for (index, pointer) in ACE.into_iter().enumerate() {
            memory.u64(ROSTER + index * 8, pointer as u64);
            memory.u64(pointer, VTABLE_RVA.0 as u64);
            memory.u64(pointer + REFERENCE, IDS[index]);
            memory.u32(pointer + AIRCRAFT_TYPE_MASK, 0xf123_4567);
            memory.u64(pointer + OWNER_COUNTRY, COUNTRY as u64);
            memory.u64(pointer + ASSIGNED_WING_REFERENCE, 0xffff_ffff_0000_0038);
            memory.bytes(pointer + ALIVE, &[u8::from(index == 0)]);
            memory.u32(pointer + KILL_TYPE, u32::MAX);
            memory.bytes(pointer + HANDLED, &[u8::from(index == 0)]);
            memory.inline(pointer + NAME, &[b'J', 0xff]);
            memory.inline(pointer + SURNAME, b"");
            memory.inline(pointer + CALLSIGN, b"One");
        }
        memory.u64(ACE[1] + KILLER_ACE_REFERENCE, u64::MAX);
        memory.u32(ACE[1] + KILLER_COUNTRY, u32::MAX);
        memory.u64(ACE[1] + NAME, 0x8000);
        memory.u64(ACE[1] + NAME + 16, 40);
        memory.u64(ACE[1] + NAME + 24, 47);
        memory.bytes(0x8000, &[0xfe; 40]);
        memory.bytes(0x8028, &[0]);
        memory
    }

    #[test]
    fn canonical_roster_preserves_complete_references_unknown_death_and_exact_names() {
        let memory = fixture();
        let mut rows = vec![Row::default(); 2];
        let mut names = vec![0_u8; 64];
        let result = walk(&memory, 0, GS, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: 2 });
        assert_eq!(result.bytes_len, 48);
        assert_eq!(result.required_bytes, 48);
        let dead = rows.iter().find(|row| row.ordinal == 1).unwrap();
        let living = rows.iter().find(|row| row.ordinal == 0).unwrap();
        assert_eq!(living.id, IDS[0]);
        assert_eq!(living.country, 2);
        assert_eq!(living.aircraft_type_mask, 0xf123_4567);
        assert_eq!(living.assigned_wing, 0xffff_ffff_0000_0038);
        assert_eq!(living.kill_type, u32::MAX);
        assert_eq!(living.death, None);
        assert_eq!(dead.handled, 0);
        assert_eq!(
            dead.death,
            Some(Death {
                killer_ace: u64::MAX,
                killer_country: u32::MAX
            })
        );
        assert_eq!(
            &names[living.name.start..living.name.start + living.name.len],
            &[b'J', 0xff]
        );
        assert_eq!(living.surname.len, 0);
        assert_eq!(
            &names[dead.name.start..dead.name.start + dead.name.len],
            &[0xfe; 40]
        );
    }

    #[test]
    fn the_count_probe_reports_exact_growth_counts() {
        let memory = fixture();
        let result = count(&memory, 0, GS);
        assert_eq!(
            result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(result.required_bytes, 48);
    }

    #[test]
    fn unreadable_dead_fields_strings_and_invalid_identity_refuse_the_whole_table() {
        for address in [
            ACE[1] + KILLER_ACE_REFERENCE,
            0x8003,
            ACE[1] + NAME + 16,
            ROSTER + 8,
        ] {
            let mut memory = fixture();
            memory.0.remove(&address);
            let result = walk(&memory, 0, GS, &mut vec![Row::default(); 2], &mut vec![0; 64]);
            assert!(matches!(result.outcome, Outcome::Unavailable { .. }));
            assert_eq!(result.bytes_len, 0);
        }
        for (address, bytes) in [
            (ACE[0], 0_u64.to_le_bytes().to_vec()),
            (ACE[0] + OWNER_COUNTRY, 0_u64.to_le_bytes().to_vec()),
            (ACE[0] + REFERENCE, 0_u64.to_le_bytes().to_vec()),
            (ACE[0] + ALIVE, vec![2]),
            (ACE[0] + HANDLED, vec![2]),
            (0x8028, vec![1]),
        ] {
            let mut memory = fixture();
            memory.bytes(address, &bytes);
            assert!(matches!(
                walk(&memory, 0, GS, &mut vec![Row::default(); 2], &mut vec![0; 64]).outcome,
                Outcome::Unavailable { .. }
            ));
        }
    }

    #[test]
    fn empty_rosters_and_repeated_members_preserve_native_order_but_null_is_unavailable() {
        let mut memory = fixture();
        memory.array(COUNTRY + ccountry::ACES, 0, 0);
        assert_eq!(
            walk(&memory, 0, GS, &mut Vec::new(), &mut Vec::new()).outcome,
            Outcome::Rows { len: 0 }
        );
        let mut memory = fixture();
        memory.u64(ROSTER + 8, ACE[0] as u64);
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(&memory, 0, GS, &mut rows, &mut vec![0; 64]).outcome,
            Outcome::Rows { len: 2 }
        );
        assert_eq!([rows[0].id, rows[1].id], [IDS[0], IDS[0]]);
        assert_eq!([rows[0].ordinal, rows[1].ordinal], [0, 1]);
        memory.u64(ROSTER + 8, 0);
        assert!(matches!(
            walk(&memory, 0, GS, &mut rows, &mut vec![0; 64]).outcome,
            Outcome::Unavailable { .. }
        ));
    }
    #[test]
    fn presentation_keeps_selected_definition_and_retained_canonical_gender_pool() {
        const GUI: usize = 0xa000;
        const POOLS: usize = 0xb000;
        const STRINGS: usize = 0xc000;
        const DEFINITION: usize = 0x9000;
        let mut memory = fixture();
        memory.array(COUNTRY + ccountry::ACES, ROSTER, 1);
        memory.bytes(ACE[0] + PORTRAIT_GENDER, &[7, 0, 0, 0, 3, 0, 0, 0]);
        memory.u64(ACE[0] + TYPE_DEFINITION, DEFINITION as u64);
        memory.inline(DEFINITION + TYPE_NAME, b"custommod_type");
        memory.u64(engine_native_politics::cgui::SINGLETON_RVA.0, GUI as u64);
        memory.u64(GUI, engine_native_politics::cgame_gui::VTABLE_RVA.0 as u64);
        memory.array(GUI + 0xcb0, POOLS, 2);
        memory.array(POOLS + engine_native_memory::cpdx_array::STRIDE, STRINGS, 2);
        memory.inline(STRINGS, b"GFX_other_ace");
        memory.inline(STRINGS + msvc_string::STRIDE, b"GFX_stale_ace");
        let mut rows = vec![Row::default(); 1];
        let mut names = vec![0; 64];
        let result = walk(&memory, 0, GS, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        let fields = rows[0].presentation.unwrap();
        assert_eq!((fields.gender, fields.pool_index), (7, 3));
        let bytes = |range: TextRange| &names[range.start..range.start + range.len];
        assert_eq!(bytes(fields.definition.unwrap()), b"custommod_type");
        let portrait = fields.portrait.unwrap();
        assert_eq!(bytes(portrait.cached.unwrap()), b"GFX_stale_ace");
        assert_eq!((portrait.culture, portrait.fallback), (None, None));
        assert_eq!(result.required_bytes, 5 + 14 + 13);
        memory.u64(DEFINITION + TYPE_NAME, 0xd000);
        memory.u64(DEFINITION + TYPE_NAME + 16, 32);
        memory.u64(DEFINITION + TYPE_NAME + 24, 47);
        memory.bytes(0xd000, &[0xff; 32]);
        memory.bytes(0xd020, &[0]);
        let result = walk(&memory, 0, GS, &mut rows, &mut names);
        let definition = rows[0].presentation.unwrap().definition.unwrap();
        assert_eq!(
            &names[definition.start..definition.start + definition.len],
            &[0xff; 32]
        );
        assert_eq!(result.required_bytes, 5 + 32 + 13);
        let mut memory = Synthetic(memory.0.clone());
        memory.0.remove(&(DEFINITION + TYPE_NAME + 16));
        let result = walk(&memory, 0, GS, &mut rows, &mut names);
        assert_eq!(result.outcome, Outcome::Rows { len: 1 });
        assert_eq!(rows[0].presentation, None);
        assert_eq!(rows[0].id, IDS[0]);
    }

    #[test]
    fn growing_aces_preserve_all_mod_rows_and_text_on_first_and_later_samples() {
        let mut memory = fixture();
        let prototype: Vec<_> = memory
            .0
            .range(ACE[0]..ACE[0] + 0x180)
            .map(|(&at, &byte)| (at - ACE[0], byte))
            .collect();
        let mut rows = Vec::new();
        let mut names = Vec::new();
        for count in [50_u32, 120] {
            memory.array(COUNTRY + ccountry::ACES, 0x80000, count);
            for index in 0..count as usize {
                let pointer = 0x100000 + index * 0x200;
                for &(offset, byte) in &prototype {
                    memory.bytes(pointer + offset, &[byte]);
                }
                memory.u64(pointer + REFERENCE, ((index as u64 + 1) << 32) | 10);
                memory.u64(0x80000 + index * 8, pointer as u64);
            }
            let walked = walk(&memory, 0, GS, &mut rows, &mut names);
            assert_eq!(
                walked.outcome,
                Outcome::Rows {
                    len: count as usize
                }
            );
            assert_eq!(walked.bytes_len, count as usize * 5);
            assert_eq!(rows[count as usize - 1].ordinal, count - 1);
        }
    }
}
