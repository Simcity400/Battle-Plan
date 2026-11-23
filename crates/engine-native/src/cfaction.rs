pub mod system;

use engine_sites::Rva;
use crate::{
    Memory,
    cfaction::system as cfaction_system,
    cfaction_template,
    memory::{RecordField, RecordLayout, RecordSources},
    msvc_string,
    read,
    read_slice,
    walk::Outcome,
};

pub const VTABLE_RVA: Rva = Rva(0x029e_2b58);

pub const REFERENCE: usize = 0x08;
pub const REFERENCE_TYPE: u32 = 88;
pub const NAME: usize = 0x18;
pub const MEMBERS: usize = 0x58;
pub const MEMBER_CAPACITY: usize = 0x60;
pub const MEMBER_COUNT: usize = 0x64;
pub const NAME_FALLBACK_HOLDER: usize = 0x568;
pub const TEMPLATE: usize = NAME_FALLBACK_HOLDER;
pub const COLOR: usize = 0x9d0;
pub const COLOR_COMPONENTS: usize = 0x10;
pub const COLOR_VTABLE_RVA: Rva = Rva(0x0279_6208);
pub const COLOR_OVERRIDDEN: usize = 0x9c0;
pub const ICON: usize = 0x548;
pub const THEATER_MANAGER: usize = 0x9f8;

pub const UPGRADE_STATUS: usize = 0x838;
pub const ADVISOR_SLOTS: usize = UPGRADE_STATUS + 0x98;
pub const ADVISOR_SLOT_STRIDE: usize = 0x1a8;

pub const POWER_PROJECTION_TOTAL: usize = 0x818;
pub const INFLUENCE_TOTAL: usize = 0x820;
pub const EXTRACTED_RESOURCES: usize = 0x910;
pub const VARIABLES: usize = 0xa40;
pub const MANPOWER_POOL: usize = 0x8f0;
pub const RESEARCH_UNLOCKED: usize = 0xa38;
#[cfg(any(test, feature = "test-support"))]
pub const MANPOWER_UNLOCKED: usize = 0xa39;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ManpowerContribution {
    pub country: u32,
    pub value: i32,
}

pub fn manpower_into(
    memory: &impl Memory,
    base: usize,
    faction: usize,
    output: &mut Vec<ManpowerContribution>,
) -> Result<(), &'static str> {
    let pool = faction
        .checked_add(MANPOWER_POOL)
        .ok_or("faction manpower pool address overflow")?;
    let expected = engine_sites::at(base, crate::carmy_manpower_values::VTABLE_RVA)
        .ok_or("faction manpower vtable address overflow")? as u64;
    if read::<u64>(memory, pool) != Some(expected) {
        return Err("faction manpower pool vtable mismatch");
    }
    let source = pool
        .checked_add(crate::carmy_manpower_values::DATA)
        .and_then(|address| crate::cpdx_array::Array::read(memory, address, 8))
        .ok_or("faction manpower contribution array unreadable or invalid")?;
    crate::allocation::resize_default(output, source.count as usize)
        .map_err(|_| "faction manpower contribution allocation failed")?;
    for (ordinal, contribution) in output.iter_mut().enumerate() {
        let address = source
            .element(ordinal as u32, 8)
            .ok_or("faction manpower contribution address overflow")?;
        let bytes =
            read::<[u8; 8]>(memory, address).ok_or("faction manpower contribution unreadable")?;
        *contribution = ManpowerContribution {
            country: u32::from_le_bytes(bytes[..4].try_into().unwrap()),
            value: i32::from_le_bytes(bytes[4..].try_into().unwrap()),
        };
    }
    Ok(())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AdvisorRow {
    pub faction: u64,
    pub ordinal: u32,
    pub token_start: usize,
    pub token_len: usize,
}

pub fn count_advisors(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_advisors_buffer(memory, base, game_state, crate::walk::Buffer::Count, crate::walk::Buffer::Count)
}

pub fn walk_advisors(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<AdvisorRow>,
    bytes: &mut Vec<u8>,
) -> NameWalk {
    walk_advisors_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(bytes),
    )
}

pub fn walk_advisors_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, AdvisorRow>,
    mut bytes: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    let result = (|| {
        let roster = cfaction_system::roster(memory, base, game_state)?;
        let advisor_vtable = engine_sites::at(base, crate::cadvisor::VTABLE_RVA)
            .ok_or("module address overflow")? as u64;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        for index in 0..roster.count {
            let (pointer, id) = cfaction_system::faction(memory, base, roster, index)?;
            let slots = pointer
                .checked_add(ADVISOR_SLOTS)
                .and_then(|address| {
                    crate::cpdx_array::Array::read(memory, address, ADVISOR_SLOT_STRIDE)
                })
                .ok_or("faction advisor slots unreadable or invalid")?;
            for ordinal in 0..slots.count {
                let advisor = slots
                    .element(ordinal, ADVISOR_SLOT_STRIDE)
                    .and_then(|slot| read::<u64>(memory, slot))
                    .and_then(|advisor| usize::try_from(advisor).ok())
                    .ok_or("faction advisor slot unreadable")?;
                if advisor == 0 {
                    continue;
                }
                if read::<u64>(memory, advisor) != Some(advisor_vtable) {
                    return Err("faction advisor vtable mismatch");
                }
                let string = advisor
                    .checked_add(crate::cadvisor::IDEA_TOKEN)
                    .ok_or("faction advisor address overflow")?;
                let literal = crate::ccharacter::Literal::read(memory, string)?;
                crate::ccharacter::Literal::validate(memory, [&literal])?;
                let end = bytes_len
                    .checked_add(literal.len)
                    .ok_or("faction advisor byte count overflow")?;
                output.ensure(len.checked_add(1).ok_or("faction advisor count overflow")?)?;
                bytes.ensure(end)?;
                if unread_rows != 0 || len == output.len() || end > bytes.len() {
                    unread_rows += 1;
                    required_bytes = required_bytes
                        .checked_add(literal.len)
                        .ok_or("faction advisor byte count overflow")?;
                    continue;
                }
                crate::ccharacter::Literal::copy_all(
                    memory,
                    [&literal],
                    &mut bytes[bytes_len..end],
                )?;
                output[len] = AdvisorRow {
                    faction: id,
                    ordinal,
                    token_start: bytes_len,
                    token_len: literal.len,
                };
                len += 1;
                bytes_len = end;
            }
        }
        Ok(NameWalk {
            outcome: if unread_rows == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated { len, unread_rows }
            },
            bytes_len,
            required_bytes: required_bytes
                .checked_add(bytes_len)
                .ok_or("faction advisor byte count overflow")?,
        })
    })();
    result.unwrap_or_else(|reason| NameWalk {
        outcome: Outcome::Unavailable { reason },
        bytes_len: 0,
        required_bytes: 0,
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub name_start: usize,
    pub name_len: usize,
    pub template_token: Option<u32>,
    pub color: [u32; 4],
    pub color_overridden: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NameWalk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> NameWalk {
    walk_buffer(memory, base, game_state, crate::walk::Buffer::Count, crate::walk::Buffer::Count)
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
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(names),
    )
}

fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    names: crate::walk::Buffer<'_, u8>,
) -> NameWalk {
    walk_buffer_with(memory, base, game_state, output, names, |_, _| {}).0
}

pub fn walk_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
    visit: impl FnMut(usize, u64),
) -> (NameWalk, bool) {
    walk_buffer_with(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(names),
        visit,
    )
}

fn walk_buffer_with(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut names: crate::walk::Buffer<'_, u8>,
    mut visit: impl FnMut(usize, u64),
) -> (NameWalk, bool) {
    let mut roster_complete = false;
    let result = (|| {
        let roster = cfaction_system::roster(memory, base, game_state)?;
        output.ensure(roster.count as usize)?;
        let mut len = 0_usize;
        let mut bytes_len = 0_usize;
        let mut required_bytes = 0_usize;
        let mut unread_rows = 0_u32;
        let mut name_failure = None;
        for index in 0..roster.count {
            let (pointer, id) = match cfaction_system::faction(memory, base, roster, index) {
                Ok(value) => value,
                Err(reason) => return Err(name_failure.unwrap_or(reason)),
            };
            visit(pointer, id);
            if name_failure.is_some() {
                continue;
            }
            let named = (|| -> Result<(), &'static str> {
                let string = pointer
                    .checked_add(NAME)
                    .ok_or("faction address overflow")?;
                let (source, name_len) = literal(memory, string)?;
                required_bytes = required_bytes
                    .checked_add(name_len)
                    .ok_or("faction name byte count overflow")?;
                names.ensure(required_bytes)?;
                if len == output.len() || name_len > names.len() - bytes_len {
                    unread_rows += 1;
                    return Ok(());
                }
                if !read_slice(memory, source, &mut names[bytes_len..bytes_len + name_len]) {
                    return Err("faction name bytes unreadable");
                }
                let (template_token, color, color_overridden) =
                    presentation(memory, base, pointer)?;
                output[len] = Row {
                    id,
                    name_start: bytes_len,
                    name_len,
                    template_token,
                    color,
                    color_overridden,
                };
                bytes_len += name_len;
                len += 1;
                Ok(())
            })();
            if let Err(reason) = named {
                name_failure = Some(reason);
            }
        }
        roster_complete = true;
        if let Some(reason) = name_failure {
            return Err(reason);
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
    (
        result.unwrap_or_else(|reason| NameWalk {
            outcome: Outcome::Unavailable { reason },
            bytes_len: 0,
            required_bytes: 0,
        }),
        roster_complete,
    )
}

pub(crate) fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<u64, &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("module address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("faction vtable mismatch");
    }
    let id = pointer
        .checked_add(REFERENCE)
        .and_then(|p| read::<u64>(memory, p))
        .ok_or("faction handle unreadable")?;
    if id == 0 {
        return Err("non-null faction has null handle");
    }
    Ok(id)
}

const COLOR_BYTES: usize = COLOR_COMPONENTS + 16;
const RECORD_COLOR: usize = 8;
const RECORD_OVERRIDDEN: usize = RECORD_COLOR + COLOR_BYTES;
const RECORD_BYTES: usize = RECORD_OVERRIDDEN + 1;
const PRESENTATION_FIELDS: [RecordField; 3] = [
    RecordField {
        source_offset: TEMPLATE,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: COLOR,
        destination_offset: RECORD_COLOR,
        len: COLOR_BYTES,
    },
    RecordField {
        source_offset: COLOR_OVERRIDDEN,
        destination_offset: RECORD_OVERRIDDEN,
        len: 1,
    },
];
const PRESENTATION: RecordLayout<'static> =
    RecordLayout::new(&PRESENTATION_FIELDS, RECORD_BYTES).expect("valid presentation layout");
const TEMPLATE_BYTES: usize = cfaction_template::IDENTIFIER_TOKEN + 4;

fn presentation(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
) -> Result<(Option<u32>, [u32; 4], u8), &'static str> {
    let mut record = [0_u8; RECORD_BYTES];
    if memory.copy_records(
        RecordSources::Pointers(&[pointer]),
        &PRESENTATION,
        &mut record,
    ) != PRESENTATION_FIELDS.len()
    {
        return Err("faction presentation unreadable");
    }
    let qword = |bytes: &[u8], at: usize| u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
    let dword = |bytes: &[u8], at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let module = |key: Rva| {
        engine_sites::at(base, key)
            .map(|address| address as u64)
            .ok_or("module address overflow")
    };
    let template =
        usize::try_from(qword(&record, 0)).map_err(|_| "faction template pointer unreadable")?;
    let template_token = if template == 0 {
        None
    } else {
        let mut span = [0_u8; TEMPLATE_BYTES];
        if !read_slice(memory, template, &mut span) {
            return Err("faction template unreadable");
        }
        if qword(&span, 0) != module(cfaction_template::VTABLE_RVA)? {
            return Err("faction template vtable mismatch");
        }
        Some(dword(&span, cfaction_template::IDENTIFIER_TOKEN))
    };
    if qword(&record, RECORD_COLOR) != module(COLOR_VTABLE_RVA)? {
        return Err("faction colour vtable mismatch");
    }
    let color =
        std::array::from_fn(|index| dword(&record, RECORD_COLOR + COLOR_COMPONENTS + index * 4));
    Ok((template_token, color, record[RECORD_OVERRIDDEN]))
}

fn literal(memory: &impl Memory, pointer: usize) -> Result<(usize, usize), &'static str> {
    let at = |offset| {
        pointer
            .checked_add(offset)
            .ok_or("faction name address overflow")
    };
    let len =
        read::<u64>(memory, at(msvc_string::LENGTH)?).ok_or("faction name length unreadable")?;
    let capacity = read::<u64>(memory, at(msvc_string::CAPACITY)?)
        .ok_or("faction name capacity unreadable")?;
    if len > capacity || capacity < msvc_string::INLINE_CAPACITY {
        return Err("faction name header invalid");
    }
    let len = usize::try_from(len).map_err(|_| "faction name length overflow")?;
    let source = if capacity == msvc_string::INLINE_CAPACITY {
        pointer
    } else {
        read::<u64>(memory, pointer)
            .and_then(|p| usize::try_from(p).ok())
            .filter(|&p| p != 0)
            .ok_or("faction name pointer unreadable or null")?
    };
    let terminator = source
        .checked_add(len)
        .ok_or("faction name address overflow")?;
    if read::<u8>(memory, terminator) != Some(0) {
        return Err("faction name terminator unreadable or invalid");
    }
    Ok((source, len))
}

#[cfg(test)]
mod advisor_tests {
    use super::*;
    use crate::{cadvisor, cgame_state, memory::SliceMemory};

    const GS: usize = 0x1000;
    const SYSTEM: usize = 0x1800;
    const ROSTER: usize = 0x1900;
    const FACTIONS: [(usize, u64); 2] = [(0x2000, (1 << 32) | 88), (0x3000, (2 << 32) | 88)];
    const SLOTS: usize = 0x4000;
    const ADVISORS: [usize; 2] = [0x5000, 0x5200];
    const HEAP: usize = 0x6000;
    const LONG: &[u8] = b"GER_wilhelm_canaris_spymaster";

    fn p64(bytes: &mut [u8], at: usize, value: u64) {
        bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
    }
    fn array(bytes: &mut [u8], at: usize, data: usize, count: u32) {
        p64(bytes, at, data as u64);
        bytes[at + 8..at + 12].copy_from_slice(&count.to_le_bytes());
        bytes[at + 12..at + 16].copy_from_slice(&count.to_le_bytes());
    }
    fn string(bytes: &mut [u8], at: usize, value: &[u8]) {
        if value.len() <= 15 {
            bytes[at..at + value.len()].copy_from_slice(value);
            p64(bytes, at + 24, 15);
        } else {
            p64(bytes, at, HEAP as u64);
            bytes[HEAP..HEAP + value.len()].copy_from_slice(value);
            p64(bytes, at + 24, 31);
        }
        p64(bytes, at + 16, value.len() as u64);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x7000];
        p64(&mut bytes, GS + cgame_state::FACTION_SYSTEM, SYSTEM as u64);
        p64(&mut bytes, SYSTEM, cfaction_system::VTABLE_RVA.0 as u64);
        array(&mut bytes, SYSTEM + cfaction_system::FACTIONS, ROSTER, 2);
        for (index, (faction, id)) in FACTIONS.into_iter().enumerate() {
            p64(&mut bytes, ROSTER + index * 8, faction as u64);
            p64(&mut bytes, faction, VTABLE_RVA.0 as u64);
            p64(&mut bytes, faction + REFERENCE, id);
        }
        array(&mut bytes, FACTIONS[0].0 + ADVISOR_SLOTS, SLOTS, 3);
        p64(&mut bytes, SLOTS, ADVISORS[0] as u64);
        p64(
            &mut bytes,
            SLOTS + 2 * ADVISOR_SLOT_STRIDE,
            ADVISORS[1] as u64,
        );
        for (advisor, token) in [
            (ADVISORS[0], b"GER_theorist".as_slice()),
            (ADVISORS[1], LONG),
        ] {
            p64(&mut bytes, advisor, cadvisor::VTABLE_RVA.0 as u64);
            string(&mut bytes, advisor + cadvisor::IDEA_TOKEN, token);
        }
        bytes
    }

    fn run(bytes: &[u8], rows: usize, text: usize) -> (NameWalk, Vec<AdvisorRow>, Vec<u8>) {
        let mut output = vec![AdvisorRow::default(); rows];
        let mut arena = vec![0; text];
        let walk = walk_advisors(
            &SliceMemory { base: 0, bytes },
            0,
            GS,
            &mut output,
            &mut arena,
        );
        (walk, output, arena)
    }

    #[test]
    fn occupied_slots_keep_their_slot_ordinal_and_exact_token() {
        let (walk, rows, arena) = run(&fixture(), 4, 64);
        assert_eq!(walk.outcome, Outcome::Rows { len: 2 });
        assert_eq!(walk.bytes_len, walk.required_bytes);
        assert_eq!(
            rows[..2]
                .iter()
                .map(|row| (row.faction, row.ordinal))
                .collect::<Vec<_>>(),
            [(FACTIONS[0].1, 0), (FACTIONS[0].1, 2)]
        );
        let token = |row: &AdvisorRow| &arena[row.token_start..row.token_start + row.token_len];
        assert_eq!(token(&rows[0]), b"GER_theorist");
        assert_eq!(token(&rows[1]), LONG);
    }

    #[test]
    fn the_advisor_count_probe_counts_rows_and_bytes_and_empty_slots_have_no_rows() {
        let bytes = fixture();
        let walk = count_advisors(&SliceMemory { base: 0, bytes: &bytes }, 0, GS);
        assert_eq!(
            walk.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(walk.required_bytes, 12 + LONG.len());
        let mut bytes = fixture();
        array(&mut bytes, FACTIONS[0].0 + ADVISOR_SLOTS, 0, 0);
        assert_eq!(run(&bytes, 4, 64).0.outcome, Outcome::Rows { len: 0 });
    }

    #[test]
    fn unauthenticated_faction_advisors_fail_the_table() {
        for (address, value) in [
            (ADVISORS[1], 7_u64),
            (FACTIONS[1].0, 7),
            (FACTIONS[0].0 + ADVISOR_SLOTS, 0x9_0000),
        ] {
            let mut bytes = fixture();
            p64(&mut bytes, address, value);
            let (walk, ..) = run(&bytes, 4, 64);
            assert!(
                matches!(walk.outcome, Outcome::Unavailable { .. }),
                "{address:#x}: {walk:?}"
            );
        }
        let mut bytes = fixture();
        bytes[HEAP + LONG.len()] = b'!';
        assert!(matches!(
            run(&bytes, 4, 64).0.outcome,
            Outcome::Unavailable { .. }
        ));
    }
}
