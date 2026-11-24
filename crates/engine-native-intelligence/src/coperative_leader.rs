use engine_native_parts::coperative_leader_layout::VTABLE_RVA;

use engine_sites::Rva;
use engine_native_memory::{Memory, cpdx_array::Array, memory::Raw, read, walk::Outcome};
use engine_native_memory::msvc_string::Literal;
use engine_native::{ccountry, crelation::subject::{Name, NameWalk}};
use crate::{
    cintelligence_agency,
    coperation_instance,
    coperative_recruitment,
    operative_action,
    operative_action_implementation,
};
use engine_native_parts::{
    creference_registry,
    cunit_leader_skill,
    null_unit_leader_skill,
};

pub const IDENTITY: usize = 0x08;
pub const TYPE: usize = 0x08;
pub const ID: usize = 0x0c;
pub const HANDLE_TYPE: u32 = 4713;
pub const NAME: usize = 0x20;
pub(crate) const NAME_MAX_LENGTH: u64 = 0x7fff_ffff_ffff_ffff;
#[cfg(test)]
pub const NAME_DUPLICATE: usize = 0x40;
pub const PORTRAIT_GFX: usize = 0x100;
pub const OWNER_COUNTRY: usize = 0x120;
pub const TRAITS: usize = 0xdc8;
pub const TRAIT_CAPACITY: usize = 0xdd0;
pub const TRAIT_COUNT: usize = 0xdd4;
pub const SKILL_OBJECT: usize = 0xe60;
pub const EXPERIENCE: usize = 0xe68;
pub const COOLDOWN_KIND: usize = 0xe84;
pub const COOLDOWN_START_HOUR: usize = 0xe90;
pub const COOLDOWN_END_HOUR: usize = 0xea8;
pub const COOLDOWN_ASSIGNMENT: u32 = 1;
pub const NATIONALITIES: usize = 0xf68;
pub const MODIFIER_ACCUMULATOR: usize = 0x418;
pub const OPERATION_REFERENCE: usize = 0xf80;
pub const PENDING_MISSION: usize = 0xf88;
pub const PENDING_MISSION_PRESENT: usize = 0xfa8;
pub const CAPTURER_COUNTRY: usize = 0xfb0;
pub const CAPTURE_HOUR: usize = 0xfc0;
pub const ASSIGNMENT: usize = 0x1080;
pub const ACTION: usize = 0x1088;
pub const MISSION_ASSIGNMENT: u32 = 3;
pub const OPERATION_ASSIGNMENT: u32 = 4;
pub const SET_OWNER_RVA: Rva = Rva(0x00c7_bc30);
#[cfg(test)]
pub const SAVE_RVA: usize = 0x00c7_ebf0;
pub const TOKEN_ASSIGNMENT: u32 = 0x01b7;
pub const ASSIGNMENT_LITERAL_IDLE: u32 = 0x4a51;
pub const ASSIGNMENT_LITERAL_TRAINING: u32 = 0x4a52;
pub const ASSIGNMENT_LITERAL_HIDING: u32 = 0x4a53;
pub const ASSIGNMENT_LITERAL_MISSION: u32 = 0x394c;
pub const ASSIGNMENT_LITERAL_OPERATION: u32 = 0x4a50;
pub const ASSIGNMENT_LITERAL_CAPTURED: u32 = 0x336c;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub owner_country: u32,
    pub roster_country: u32,
    pub roster_kind: u8,
    pub ordinal: u32,
    pub assignment: u32,
    pub experience_raw: i64,
    pub skill: Option<i32>,
    pub capturer_country: u32,
    pub captured_hour: i32,
    pub action_variant: Option<u32>,
    pub target_country: Option<u32>,
    pub operation: Option<u64>,
    pub name: Name,
    pub cooldown_kind: Option<u32>,
    pub cooldown_start_hour: Option<i32>,
    pub cooldown_end_hour: Option<i32>,
}

pub struct Walk {
    pub outcome: Outcome,
    pub bytes_len: usize,
    pub required_bytes: usize,
    pub failed_membership: Option<(u64, u32, u8)>,
    pub refused_rows: u32,
    pub first_refused: Option<Refusal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub id: u64,
    pub roster_country: u32,
    pub roster_kind: u8,
    pub reason: &'static str,
}

enum ReadRow {
    Row(Row),
    Refused(&'static str),
}

pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Growing(output),
        engine_native_memory::walk::Buffer::Growing(names),
    )
}

pub fn count(memory: &impl Memory, base: usize, game_state: usize) -> Walk {
    walk_buffer(
        memory,
        base,
        game_state,
        engine_native_memory::walk::Buffer::Count,
        engine_native_memory::walk::Buffer::Count,
    )
}

pub fn walk_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: engine_native_memory::walk::Buffer<'_, Row>,
    mut names: engine_native_memory::walk::Buffer<'_, u8>,
) -> Walk {
    let mut failed_membership = None;
    let mut refused_rows = 0_u32;
    let mut first_refused = None;
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let mut mission_vtables = operative_action_implementation::Vtables::new(base);
        let mut len = 0_usize;
        let mut unread_rows = 0_u32;
        let (mut bytes_len, mut required_bytes) = (0_usize, 0_usize);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(agency) = cintelligence_agency::root(memory, base, country)? else {
                continue;
            };
            let recruitment = at(agency, cintelligence_agency::RECRUITMENT)?;
            if read::<u64>(memory, recruitment)
                != Some(
                    engine_sites::at(base, coperative_recruitment::VTABLE_RVA)
                        .ok_or("operative recruitment vtable overflow")? as u64,
                )
            {
                return Err("operative recruitment vtable mismatch");
            }
            let rosters = [
                at(agency, cintelligence_agency::OPERATIVES)?,
                at(recruitment, coperative_recruitment::POOL_A)?,
                at(recruitment, coperative_recruitment::RECRUITABLE_OPERATIVES)?,
                at(recruitment, coperative_recruitment::NOT_TO_SPY_MASTER)?,
            ];
            for (kind, header) in rosters.into_iter().enumerate() {
                let array = Array::read(memory, header, 8)
                    .ok_or("operative roster unreadable or invalid")?;
                output.ensure(
                    len.checked_add(array.count as usize)
                        .ok_or("operative row demand overflow")?,
                )?;
                let roster_start = len;
                for index in 0..array.count {
                    let pointer = array
                        .element(index, 8)
                        .and_then(|p| read::<u64>(memory, p))
                        .and_then(|p| usize::try_from(p).ok())
                        .filter(|&p| p != 0)
                        .ok_or("operative roster pointer unreadable or null")?;
                    let expected = engine_sites::at(base, VTABLE_RVA)
                        .ok_or("operative vtable address overflow")?;
                    if read::<u64>(memory, pointer) != Some(expected as u64) {
                        return Err("operative vtable mismatch");
                    }
                    let id = field::<u64>(memory, pointer, IDENTITY)?;
                    if id == 0 {
                        return Err("operative has a null handle");
                    }
                    failed_membership = Some((id, country.id, kind as u8));
                    let name_pointer = at(pointer, NAME)?;
                    let name_header = read::<[u8; 32]>(memory, name_pointer)
                        .ok_or("operative saved name header unreadable")?;
                    if u64::from_le_bytes(name_header[16..24].try_into().unwrap()) > NAME_MAX_LENGTH
                        || u64::from_le_bytes(name_header[24..32].try_into().unwrap())
                            > NAME_MAX_LENGTH
                    {
                        return Err("operative saved name exceeds native string maximum");
                    }
                    let name = Literal::from_header(name_pointer, name_header)
                        .map_err(|_| "operative saved name unreadable or invalid")?;
                    Literal::validate(memory, [&name])
                        .map_err(|_| "operative saved name terminator unreadable or invalid")?;
                    let mut row = match read_row(
                        memory,
                        base,
                        pointer,
                        id,
                        country.id,
                        kind as u8,
                        &countries,
                        &mut mission_vtables,
                    )? {
                        ReadRow::Row(row) => row,
                        ReadRow::Refused(reason) => {
                            refused_rows = refused_rows
                                .checked_add(1)
                                .ok_or("operative refused row count overflow")?;
                            unread_rows = unread_rows
                                .checked_add(1)
                                .ok_or("operative row count overflow")?;
                            first_refused.get_or_insert(Refusal {
                                id,
                                roster_country: country.id,
                                roster_kind: kind as u8,
                                reason,
                            });
                            failed_membership = None;
                            continue;
                        }
                    };
                    required_bytes = required_bytes
                        .checked_add(name.len)
                        .ok_or("operative name size overflow")?;
                    names.ensure(required_bytes)?;
                    if len == output.len() || name.len > names.len() - bytes_len {
                        unread_rows = unread_rows
                            .checked_add(1)
                            .ok_or("operative row count overflow")?;
                        failed_membership = None;
                        continue;
                    }
                    row.ordinal = index;
                    if output[roster_start..len].iter().any(|old| old.id == row.id) {
                        return Err("duplicate operative handle within holding vector");
                    }
                    Literal::copy_all(memory, [&name], &mut names[bytes_len..bytes_len + name.len])
                        .map_err(|_| "operative saved name bytes unreadable")?;
                    row.name = Name {
                        start: bytes_len,
                        len: name.len,
                    };
                    bytes_len += name.len;
                    output[len] = row;
                    len += 1;
                    failed_membership = None;
                }
            }
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
    let names = result.unwrap_or_else(engine_native::crelation::subject::unavailable);
    Walk {
        outcome: names.outcome,
        bytes_len: names.bytes_len,
        required_bytes: names.required_bytes,
        failed_membership,
        refused_rows,
        first_refused,
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "Native membership identity and caller-owned lookup state remain explicit"
)]
fn read_row<M: Memory>(
    memory: &M,
    base: usize,
    pointer: usize,
    id: u64,
    roster_country: u32,
    roster_kind: u8,
    countries: &ccountry::Reader<'_, M>,
    mission_vtables: &mut operative_action_implementation::Vtables,
) -> Result<ReadRow, &'static str> {
    let owner_country = field::<u32>(memory, pointer, OWNER_COUNTRY)?;
    let capturer_country = field::<u32>(memory, pointer, CAPTURER_COUNTRY)?;
    for raw in [owner_country, capturer_country] {
        if raw != 0 {
            countries.resolve(raw)?;
        }
    }
    let assignment = field::<u32>(memory, pointer, ASSIGNMENT)?;
    let (cooldown_kind, cooldown_start_hour, cooldown_end_hour) =
        if assignment == COOLDOWN_ASSIGNMENT {
            (
                Some(field(memory, pointer, COOLDOWN_KIND)?),
                Some(field(memory, pointer, COOLDOWN_START_HOUR)?),
                Some(field(memory, pointer, COOLDOWN_END_HOUR)?),
            )
        } else {
            (None, None, None)
        };
    let mission = at(pointer, ACTION)?;
    let variant = field::<u32>(memory, mission, operative_action::VARIANT)?;
    let action_variant = (variant != 0).then_some(variant);
    let mission_country = action_variant
        .map(|variant| mission_vtables.country(memory, mission, variant))
        .transpose()?;
    let operation = field::<u64>(memory, pointer, OPERATION_REFERENCE)?;
    let operation = (operation != 0).then_some(operation);
    let target_country = match assignment {
        MISSION_ASSIGNMENT => mission_country,
        OPERATION_ASSIGNMENT => {
            let Some(handle) = operation else {
                return Ok(ReadRow::Refused(
                    "active operative operation reference is null",
                ));
            };
            let Some(instance) = creference_registry::resolve(memory, base, handle)? else {
                return Ok(ReadRow::Refused(
                    "active operative operation reference is stale",
                ));
            };
            if field::<u64>(memory, instance, 0)?
                != engine_sites::at(base, coperation_instance::VTABLE_RVA)
                    .ok_or("operative operation vtable overflow")? as u64
            {
                return Err("operative operation instance vtable mismatch");
            }
            if field::<u64>(memory, instance, coperation_instance::HANDLE)? != handle {
                return Err("operative operation instance identity mismatch");
            }
            Some(field::<u32>(
                memory,
                instance,
                coperation_instance::TARGET_COUNTRY_ID,
            )?)
        }
        _ => None,
    }
    .filter(|&raw| raw != 0);
    if let Some(raw) = target_country {
        countries.resolve(raw)?;
    }
    let skill_pointer = usize::try_from(field::<u64>(memory, pointer, SKILL_OBJECT)?)
        .map_err(|_| "operative skill pointer overflow")?;
    let skill = if skill_pointer == 0 {
        None
    } else {
        let vtable = field::<u64>(memory, skill_pointer, 0)?;
        let regular = engine_sites::at(base, cunit_leader_skill::VTABLE_RVA)
            .ok_or("operative skill vtable overflow")? as u64;
        let null = engine_sites::at(base, null_unit_leader_skill::VTABLE_RVA)
            .ok_or("operative null skill vtable overflow")? as u64;
        if vtable != regular && vtable != null {
            return Err("operative skill vtable mismatch");
        }
        if field::<u8>(memory, skill_pointer, cunit_leader_skill::VALID)? == 0 {
            None
        } else {
            Some(field::<i32>(
                memory,
                skill_pointer,
                cunit_leader_skill::LEVEL,
            )?)
        }
    };
    Ok(ReadRow::Row(Row {
        id,
        owner_country,
        roster_country,
        roster_kind,
        ordinal: 0,
        assignment,
        experience_raw: field(memory, pointer, EXPERIENCE)?,
        skill,
        capturer_country,
        captured_hour: field(memory, pointer, CAPTURE_HOUR)?,
        action_variant,
        target_country,
        operation,
        name: Name::default(),
        cooldown_kind,
        cooldown_start_hour,
        cooldown_end_hour,
    }))
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("operative address overflow")
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("operative field unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native::cgame_state;
    use engine_native_memory::memory::SliceMemory;
    fn walk(memory: &impl Memory, base: usize, state: usize, rows: &mut Vec<Row>) -> Outcome {
        super::walk(memory, base, state, rows, &mut Vec::new()).outcome
    }
    fn count(memory: &impl Memory, base: usize, state: usize) -> Outcome {
        walk_buffer(
            memory,
            base,
            state,
            engine_native_memory::walk::Buffer::Count,
            engine_native_memory::walk::Buffer::Count,
        )
        .outcome
    }
    fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }
    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x9000];
        for (offset, data) in [
            (cgame_state::COUNTRIES, 0x2000_u64),
            (cgame_state::COUNTRY_LINKS, 0x2100),
        ] {
            put(&mut bytes, 0x100 + offset, data.to_le_bytes());
            put(&mut bytes, 0x100 + offset + 8, 2_u32.to_le_bytes());
            put(&mut bytes, 0x100 + offset + 12, 2_u32.to_le_bytes());
        }
        put(&mut bytes, 0x2008, 0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000,
            (ccountry::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x3008, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + ccountry::INTELLIGENCE_AGENCY,
            0x5000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000,
            (cintelligence_agency::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OWNER_COUNTRY,
            0x3000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5008,
            (coperative_recruitment::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        for (header, data) in [
            (0x5000 + cintelligence_agency::OPERATIVES, 0x6000_u64),
            (
                0x5008 + coperative_recruitment::RECRUITABLE_OPERATIVES,
                0x6010,
            ),
        ] {
            put(&mut bytes, header, data.to_le_bytes());
            put(&mut bytes, header + 8, 1_u32.to_le_bytes());
            put(&mut bytes, header + 12, 1_u32.to_le_bytes());
            put(&mut bytes, data as usize, 0x7000_u64.to_le_bytes());
        }
        put(&mut bytes, 0x7000, (VTABLE_RVA.0 as u64).to_le_bytes());
        put(&mut bytes, 0x7000 + NAME + 24, 15_u64.to_le_bytes());
        put(
            &mut bytes,
            0x7000 + IDENTITY,
            0xffff_ffff_8000_1269_u64.to_le_bytes(),
        );
        put(&mut bytes, 0x7000 + OWNER_COUNTRY, 1_u32.to_le_bytes());
        put(&mut bytes, 0x7000 + 0x70, [0xfe]);
        put(
            &mut bytes,
            0x7000 + EXPERIENCE,
            (i64::MAX - 2).to_le_bytes(),
        );
        put(&mut bytes, 0x7000 + CAPTURER_COUNTRY, 1_u32.to_le_bytes());
        put(&mut bytes, 0x7000 + CAPTURE_HOUR, i32::MIN.to_le_bytes());
        bytes
    }
    #[test]
    fn growing_operative_rosters_and_names_capture_new_memberships_before_publication() {
        let mut bytes = fixture();
        let name = 0x7000 + NAME;
        bytes[name..name + 3].copy_from_slice(&[0xfe, 0, b'x']);
        put(&mut bytes, name + 16, 3_u64.to_le_bytes());
        let mut rows = Vec::new();
        let mut names = Vec::new();
        let mut expected_rows = vec![Row::default(); 2];
        let mut expected_names = vec![0; 6];
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let expected = super::walk(&memory, 0, 0x100, &mut expected_rows, &mut expected_names);
        assert_eq!(expected.outcome, Outcome::Rows { len: 2 });
        let actual = super::walk(&memory, 0, 0x100, &mut rows, &mut names);
        assert_eq!(
            (
                actual.outcome,
                actual.bytes_len,
                actual.required_bytes,
                actual.failed_membership,
                actual.refused_rows,
                actual.first_refused
            ),
            (
                expected.outcome,
                expected.bytes_len,
                expected.required_bytes,
                expected.failed_membership,
                expected.refused_rows,
                expected.first_refused
            )
        );
        assert_eq!(&rows[..2], &expected_rows);
        assert_eq!(&names[..6], &expected_names);
        const COUNT: usize = 5_000;
        const ROSTER: usize = 0x10000;
        const OBJECTS: usize = 0x20000;
        const STRIDE: usize = 0x1100;
        let template = bytes[0x7000..0x7000 + STRIDE].to_vec();
        bytes.resize(OBJECTS + COUNT * STRIDE, 0);
        let header = 0x5000 + cintelligence_agency::OPERATIVES;
        put(&mut bytes, header, (ROSTER as u64).to_le_bytes());
        put(&mut bytes, header + 8, (COUNT as u32).to_le_bytes());
        put(&mut bytes, header + 12, (COUNT as u32).to_le_bytes());
        for index in 0..COUNT {
            let object = OBJECTS + index * STRIDE;
            bytes[object..object + STRIDE].copy_from_slice(&template);
            put(
                &mut bytes,
                object + IDENTITY,
                (((index as u64 + 1) << 32) | u64::from(HANDLE_TYPE)).to_le_bytes(),
            );
            put(
                &mut bytes,
                ROSTER + index * 8,
                (object as u64).to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let actual = super::walk(&memory, 0, 0x100, &mut rows, &mut names);
        assert_eq!(actual.outcome, Outcome::Rows { len: COUNT + 1 });
        assert_eq!(
            (actual.bytes_len, actual.required_bytes),
            ((COUNT + 1) * 3, (COUNT + 1) * 3)
        );
        assert!(
            rows[..COUNT]
                .iter()
                .enumerate()
                .all(|(index, row)| row.ordinal as usize == index && row.roster_kind == 0)
        );
        assert_eq!(rows[COUNT].roster_kind, 2);
        assert!(
            names[..actual.bytes_len]
                .chunks(3)
                .all(|value| value == [0xfe, 0, b'x'])
        );
    }

    #[test]
    fn native_memberships_and_scalar_widths_are_preserved() {
        let bytes = fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 2];
        assert_eq!(walk(&memory, 0, 0x100, &mut rows), Outcome::Rows { len: 2 });
        assert_eq!(rows[0].id, 0xffff_ffff_8000_1269);
        assert_eq!(rows[0].experience_raw, i64::MAX - 2);
        assert_eq!(rows[0].captured_hour, i32::MIN);
        assert_eq!(rows[0].roster_kind, 0);
        assert_eq!(rows[1].roster_kind, 2);
        assert_eq!(
            rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
            [0, 0]
        );
        assert_eq!(
            count(&memory, 0, 0x100),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
    }

    #[test]
    fn localized_description_length_is_not_read_as_detection() {
        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let obsolete = 0x7000 + 0x70;
                if address <= obsolete && obsolete < address.saturating_add(destination.len()) {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, destination)
            }
        }
        let bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(&Fault(&bytes), 0, 0x100, &mut rows),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].captured_hour, i32::MIN);
    }
    #[test]
    fn operative_positions_follow_each_holding_vector_after_removal() {
        let mut bytes = fixture();
        let other = 0x9000;
        bytes.resize(0xb000, 0);
        let first = bytes[0x7000..0x7000 + 0x1100].to_vec();
        bytes[other..other + first.len()].copy_from_slice(&first);
        put(
            &mut bytes,
            other + IDENTITY,
            0x8000_0001_0000_1269_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OPERATIVES + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OPERATIVES + 12,
            2_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x6008, (other as u64).to_le_bytes());
        let mut rows = vec![Row::default(); 3];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                0x100,
                &mut rows,
            ),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(
            rows.iter()
                .map(|row| (row.roster_kind, row.ordinal))
                .collect::<Vec<_>>(),
            [(0, 0), (0, 1), (2, 0)]
        );
        put(&mut bytes, 0x6000, (other as u64).to_le_bytes());
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OPERATIVES + 12,
            1_u32.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                0x100,
                &mut rows,
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].ordinal, 0);
        assert_eq!(rows[0].id, 0x8000_0001_0000_1269);
    }
    #[test]
    fn action_variant_uses_its_discriminator_and_skill_has_native_signed_width() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            MISSION_ASSIGNMENT.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + ACTION + operative_action::VARIANT,
            0_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x7000 + SKILL_OBJECT, 0x8200_u64.to_le_bytes());
        put(
            &mut bytes,
            0x8200,
            (cunit_leader_skill::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8200 + cunit_leader_skill::VALID, [1]);
        put(
            &mut bytes,
            0x8200 + cunit_leader_skill::LEVEL,
            i32::MIN.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].skill, Some(i32::MIN));
        assert_eq!(rows[0].action_variant, None);
        put(&mut bytes, 0x7000 + ASSIGNMENT, 9_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].action_variant, None);
        put(
            &mut bytes,
            0x7000 + ACTION + operative_action::VARIANT,
            0_u32.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].action_variant, None);
    }
    #[test]
    fn every_mission_authenticates_its_concrete_vtable_and_reads_the_stored_country() {
        let mut bytes = fixture();
        bytes.resize(
            operative_action_implementation::VTABLE_RVAS
                .iter()
                .max()
                .unwrap()
                .0
                + 0x58,
            0,
        );
        for rva in operative_action_implementation::VTABLE_RVAS {
            put(&mut bytes, rva.0 + 0x50, 0x01ac_42f0u64.to_le_bytes());
        }
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            MISSION_ASSIGNMENT.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + ACTION + operative_action::IMPLEMENTATION,
            0x8500_u64.to_le_bytes(),
        );
        put(&mut bytes, 0x8518, u64::MAX.to_le_bytes());
        let mut rows = vec![Row::default(); 2];
        for (index, rva) in operative_action_implementation::VTABLE_RVAS
            .into_iter()
            .enumerate()
        {
            let variant = index as u32 + 1;
            put(
                &mut bytes,
                0x7000 + ACTION + operative_action::VARIANT,
                variant.to_le_bytes(),
            );
            put(&mut bytes, 0x8500, (rva.0 as u64).to_le_bytes());
            put(&mut bytes, 0x8510, 1_u32.to_le_bytes());
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    0,
                    0x100,
                    &mut rows
                ),
                Outcome::Rows { len: 2 }
            );
            assert!(rows.iter().all(|row| row.target_country == Some(1) && row.action_variant == Some(variant)));
            put(&mut bytes, 0x8500, 0_u64.to_le_bytes());
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    0,
                    0x100,
                    &mut rows
                ),
                Outcome::Unavailable {
                    reason: "unsupported operative mission implementation vtable"
                }
            );
        }
        put(
            &mut bytes,
            0x7000 + ACTION + operative_action::VARIANT,
            9_u32.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "unsupported operative mission variant"
            }
        );
        put(
            &mut bytes,
            0x7000 + ACTION + operative_action::VARIANT,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x8500,
            (operative_action_implementation::VTABLE_RVAS[0].0 as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x8510, 0_u32.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(rows[0].target_country, None);
        put(&mut bytes, 0x8510, 1_u32.to_le_bytes());
        for assignment in [0_u32, 1, 2, 5] {
            put(&mut bytes, 0x7000 + ASSIGNMENT, assignment.to_le_bytes());
            assert_eq!(
                walk(
                    &SliceMemory {
                        base: 0,
                        bytes: &bytes
                    },
                    0,
                    0x100,
                    &mut rows
                ),
                Outcome::Rows { len: 2 }
            );
            assert_eq!(rows[0].target_country, None);
            assert_eq!(rows[0].action_variant, Some(1));
        }
    }

    #[test]
    fn own_operation_reference_is_independent_of_reservation_and_resolved_for_assignment_four() {
        use engine_native_parts::reference_bucket as bucket;
        let mut bytes = fixture();
        bytes.resize(creference_registry::HIGH_ROOT_RVA.0 + 8, 0);
        let handle = (99_u64 << 32) | 4713;
        put(
            &mut bytes,
            0x7000 + OPERATION_REFERENCE,
            handle.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (rows[0].operation, rows[0].target_country),
            (Some(handle), None)
        );
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            OPERATION_ASSIGNMENT.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2,
            }
        );
        put(
            &mut bytes,
            creference_registry::HIGH_ROOT_RVA.0,
            0x9000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x9000 + creference_registry::ENTRIES,
            0xa000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x9000 + creference_registry::COUNT,
            1_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x9000 + creference_registry::MASK,
            7_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x9000 + creference_registry::OVERFLOW_DISTANCE,
            [5],
        );
        put(
            &mut bytes,
            0xa000 + 13 * bucket::STRIDE + bucket::DISTANCE,
            [255],
        );
        let hash = handle
            .rotate_left(32)
            .wrapping_mul(creference_registry::HASH_MULTIPLIER);
        let index = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
        let cell = 0xa000 + index * bucket::STRIDE;
        put(&mut bytes, cell + bucket::DISTANCE, [1]);
        put(&mut bytes, cell + bucket::TYPE, handle.to_le_bytes());
        put(&mut bytes, cell + bucket::OBJECT, 0xb000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xb000,
            (coperation_instance::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb000 + coperation_instance::HANDLE,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            0xb000 + coperation_instance::TARGET_COUNTRY_ID,
            1_u32.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert_eq!(
            (rows[1].operation, rows[1].target_country),
            (Some(handle), Some(1))
        );
        put(
            &mut bytes,
            0xb000 + coperation_instance::HANDLE,
            (handle + 1).to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "operative operation instance identity mismatch"
            }
        );
        put(
            &mut bytes,
            0x7000 + OPERATION_REFERENCE,
            0_u64.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Truncated {
                len: 0,
                unread_rows: 2,
            }
        );
    }

    #[test]
    fn stale_active_operation_refuses_only_its_named_membership_without_growth_demand() {
        let mut bytes = fixture();
        bytes.resize(creference_registry::HIGH_ROOT_RVA.0 + 8, 0);
        let healthy = bytes[0x7000..0x7000 + 0x1100].to_vec();
        bytes[0xc000..0xc000 + healthy.len()].copy_from_slice(&healthy);
        put(&mut bytes, 0x6010, 0xc000_u64.to_le_bytes());
        put(
            &mut bytes,
            0xc000 + IDENTITY,
            ((88_u64 << 32) | 4713).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            OPERATION_ASSIGNMENT.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + OPERATION_REFERENCE,
            ((99_u64 << 32) | 4713).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = vec![Row::default(); 2];
        let read = super::walk(&memory, 0, 0x100, &mut rows, &mut Vec::new());
        assert_eq!(
            read.outcome,
            Outcome::Truncated {
                len: 1,
                unread_rows: 1
            }
        );
        assert_eq!(rows[0].id, (88 << 32) | 4713);
        assert_eq!(read.refused_rows, 1);
        assert_eq!(
            read.first_refused,
            Some(Refusal {
                id: 0xffff_ffff_8000_1269,
                roster_country: 1,
                roster_kind: 0,
                reason: "active operative operation reference is stale",
            })
        );
        assert_eq!(read.required_bytes, 0);
        let sizing = super::count(&memory, 0, 0x100);
        assert_eq!(
            sizing.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        );
        assert_eq!(sizing.refused_rows, 1);
        assert_eq!(sizing.required_bytes, 0);
    }

    #[test]
    fn name_length_above_native_string_maximum_is_refused_before_payload_access() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x7000 + NAME + 16,
            (NAME_MAX_LENGTH + 1).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + NAME + 24,
            (NAME_MAX_LENGTH + 1).to_le_bytes(),
        );
        let read = super::walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x100,
            &mut vec![Row::default(); 2],
            &mut Vec::new(),
        );
        assert_eq!(
            read.outcome,
            Outcome::Unavailable {
                reason: "operative saved name exceeds native string maximum"
            }
        );
        assert_eq!(read.failed_membership, Some((0xffff_ffff_8000_1269, 1, 0)));
    }

    #[test]
    fn names_have_exact_native_demand_and_never_use_the_localised_cache() {
        let mut bytes = fixture();
        let name = vec![0xfe; 40_000];
        bytes.resize(0x9000 + name.len() + 1, 0);
        bytes[0x9000..0x9000 + name.len()].copy_from_slice(&name);
        put(&mut bytes, 0x7000 + NAME, 0x9000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x7000 + NAME + 16,
            (name.len() as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + NAME + 24,
            (name.len() as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + NAME_DUPLICATE + 24,
            u64::MAX.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 2];
        let mut names = vec![0; 2 * name.len()];
        let read = super::walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x100,
            &mut rows,
            &mut names,
        );
        assert_eq!(read.outcome, Outcome::Rows { len: 2 });
        assert_eq!((read.bytes_len, read.required_bytes), (80_000, 80_000));
        assert_eq!(&names[..40_000], &name);
        assert_eq!(
            rows[1].name,
            Name {
                start: 40_000,
                len: 40_000
            }
        );
        bytes[0x9000 + name.len()] = 1;
        assert!(matches!(
            super::walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows,
                &mut names
            )
            .outcome,
            Outcome::Unavailable { .. }
        ));
    }

    #[test]
    fn invalid_headers_and_read_faults_never_publish_a_complete_prefix() {
        let mut bytes = fixture();
        let mut rows = vec![Row::default(); 2];
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OPERATIVE_COUNT,
            (-1_i32).to_le_bytes(),
        );
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Unavailable { .. }
        ));
        put(
            &mut bytes,
            0x5000 + cintelligence_agency::OPERATIVE_COUNT,
            1_u32.to_le_bytes(),
        );
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x7000 + ASSIGNMENT + 3]
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Unavailable { .. }
        ));
        put(&mut bytes, 0x6000, 0_u64.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Unavailable {
                reason: "operative roster pointer unreadable or null"
            }
        );
    }
    #[test]
    fn cooldown_assignment_captures_both_raw_dates_in_every_membership() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            COOLDOWN_ASSIGNMENT.to_le_bytes(),
        );
        put(&mut bytes, 0x7000 + COOLDOWN_KIND, 3_u32.to_le_bytes());
        put(
            &mut bytes,
            0x7000 + COOLDOWN_START_HOUR,
            (-24_i32).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x7000 + COOLDOWN_END_HOUR,
            i32::MAX.to_le_bytes(),
        );
        let mut rows = vec![Row::default(); 2];
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        for row in &rows {
            assert_eq!(row.cooldown_kind, Some(3));
            assert_eq!(row.cooldown_start_hour, Some(-24));
            assert_eq!(row.cooldown_end_hour, Some(i32::MAX));
        }
        put(
            &mut bytes,
            0x7000 + ASSIGNMENT,
            MISSION_ASSIGNMENT.to_le_bytes(),
        );
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut rows
            ),
            Outcome::Rows { len: 2 }
        );
        assert!(rows.iter().all(|row| row.cooldown_kind.is_none()
            && row.cooldown_start_hour.is_none()
            && row.cooldown_end_hour.is_none()));
    }
}
