//! CIntelligenceAgency, reached through CCountry's agency unique pointer.
//!
//! Native scalar state; roster, cryptology and operative rows are separate walks.
//! Scripted localisation uses the stored name and created byte. The reader
//! preserves the save gates. Names use validated stored string
//! lengths and a caller-sized arena. Capacity shortages omit whole rows with
//! exact demand; a bad header or terminator refuses the table.

use crate::{Memory, ccountry, cpdx_array::Array, memory::Raw, read, walk::Outcome};

/// CIntelligenceAgency primary vtable.
pub const VTABLE_RVA: usize = 0x0296_b338;
/// CIntelligenceAgency inline COperativeRecruitment.
pub const RECRUITMENT: usize = 0x08;
/// CIntelligenceAgency owning CCountry pointer.
pub const OWNER_COUNTRY: usize = 0x78;
/// CIntelligenceAgency name std::string.
pub const NAME: usize = 0x80;
/// CIntelligenceAgency icon std::string.
pub const ICON: usize = 0xa0;
/// Agency-created byte. Every country has an agency object independently
/// of this flag. Nonzero selects its stored name; zero selects the display
/// fallback. Serialization writes the created flag only when nonzero.
pub const CREATED: usize = 0xc0;
/// CIntelligenceAgency signed i64 progress /100000.
pub const UPGRADE_PROGRESS: usize = 0xc8;
/// CIntelligenceAgency COperativeLeader pointer-vector data.
pub const OPERATIVES: usize = 0xd8;
/// CIntelligenceAgency active-vector u32 capacity.
pub const OPERATIVE_CAPACITY: usize = 0xe0;
/// CIntelligenceAgency active-vector i32 size.
pub const OPERATIVE_COUNT: usize = 0xe4;
/// Unidentified agency dword initialized with +0xF4, read as u32.
/// It does not establish available operative slots.
pub const UNKNOWN_F0: usize = 0xf0;
/// Signed operative-admission ceiling; ordinary admission requires count below it.
pub const OPERATIVE_ADMISSION_CEILING: usize = 0xf4;
/// CIntelligenceAgency CCapturedOperativeReference inline-array data.
pub const CAPTURED_OPERATIVE_REFERENCES: usize = 0x108;
/// Signed CPdxArray count.
pub const CAPTURED_OPERATIVE_REFERENCE_COUNT: usize = 0x114;
/// CIntelligenceAgency CCryptology pointer.
pub const CRYPTOLOGY: usize = 0x120;
/// CIntelligenceAgency signed i64 defense /100000.
pub const DEFENSE: usize = 0x128;
/// CIntelligenceAgency constructor RVA.
pub const CONSTRUCTOR_RVA: usize = 0x00fc_1d70;
/// CIntelligenceAgency AddOperative RVA.
pub const ADD_OPERATIVE_RVA: usize = 0x00fc_21d0;
/// Agency operative removal erases active membership.
pub const REMOVE_OPERATIVE_RVA: usize = 0x00fc_9120;
/// Agency operative capture appends a captured reference without
/// removing active or recruitment membership.
pub const CAPTURE_OPERATIVE_RVA: usize = 0x00fc_5830;
/// Operative-admission override byte; nonzero permits count above ceiling.
pub const ADMISSION_OVERRIDE_RVA: usize = 0x0331_725e;
/// CIntelligenceAgency container loader.
pub const CONTAINER_LOADER_RVA: usize = 0x00fc_8750;
/// CIntelligenceAgency operative construct helper.
pub const OPERATIVE_CONSTRUCT_RVA: usize = 0x00bf_c8d0;
/// CIntelligenceAgency operative token.
pub const TOKEN_OPERATIVE: u32 = 0x3d13;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub country: u32,
    pub upgrade_progress_raw: i64,
    pub defense_raw: i64,
    pub active_count: i32,
    pub admission_ceiling: i32,
    /// The serializer preserves this dword; its gameplay meaning is unresolved.
    pub unknown_f0: u32,
    /// [`CREATED`], the raw byte.
    pub created: u8,
    pub name: crate::csubject_relation::Name,
}

/// Reads the minimum agency scalars without rounding or calling engine code.
/// Null agency pointers mean no object. A non-null invalid object is unavailable.
/// The name is copied at its native length; icon, upgrade and operative records have separate readers.
pub fn walk(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
) -> crate::csubject_relation::NameWalk {
    walk_shared(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(names),
        |_, _| {},
    )
}

pub struct UpgradeWalk {
    pub agencies: crate::csubject_relation::NameWalk,
    pub upgrades: crate::agency_upgrade_levels::Walk,
}

/// Entries and scalar rows share one country census and authenticated
/// agency root. The created byte filters neither table.
pub fn walk_with_upgrades(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [Row],
    names: &mut [u8],
    upgrades: &mut [crate::agency_upgrade_levels::Row],
    catalog: &mut crate::agency_upgrade_levels::Catalog,
) -> UpgradeWalk {
    walk_with_upgrades_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Fixed(output),
        crate::walk::Buffer::Fixed(names),
        crate::walk::Buffer::Fixed(upgrades),
        catalog,
    )
}

pub fn walk_with_upgrades_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<Row>,
    names: &mut Vec<u8>,
    upgrades: &mut Vec<crate::agency_upgrade_levels::Row>,
    catalog: &mut crate::agency_upgrade_levels::Catalog,
) -> UpgradeWalk {
    walk_with_upgrades_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
        crate::walk::Buffer::Growing(names),
        crate::walk::Buffer::Growing(upgrades),
        catalog,
    )
}

pub fn walk_with_upgrades_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: crate::walk::Buffer<'_, Row>,
    names: crate::walk::Buffer<'_, u8>,
    upgrades: crate::walk::Buffer<'_, crate::agency_upgrade_levels::Row>,
    catalog: &mut crate::agency_upgrade_levels::Catalog,
) -> UpgradeWalk {
    let mut capture = crate::agency_upgrade_levels::Capture::new_buffer(upgrades, catalog);
    let agencies = walk_shared(
        memory,
        base,
        game_state,
        output,
        names,
        |country, agency| {
            capture.agency(memory, base, country, agency);
        },
    );
    if let Outcome::Unavailable { reason } = agencies.outcome {
        capture.failed(reason);
    }
    UpgradeWalk {
        agencies,
        upgrades: capture.finish(memory, base),
    }
}

fn walk_shared(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, Row>,
    mut names: crate::walk::Buffer<'_, u8>,
    mut each: impl FnMut(u32, usize),
) -> crate::csubject_relation::NameWalk {
    use crate::{
        ccharacter::Literal,
        csubject_relation::{Name, NameWalk},
    };
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut len, mut bytes_len, mut required_bytes) = (0_usize, 0_usize, 0_usize);
        let mut unread_rows = 0_u32;
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(pointer) = root(memory, base, country)? else {
                continue;
            };
            each(country.id, pointer);
            // Created agencies use their stored name verbatim, even when empty.
            // Serialization writes the same bytes; never localise them again.
            let name = Literal::read(memory, at(pointer, NAME)?)?;
            Literal::validate(memory, [&name])?;
            required_bytes = required_bytes
                .checked_add(name.len)
                .ok_or("agency name size overflow")?;
            output.ensure(len.checked_add(1).ok_or("agency row count overflow")?)?;
            names.ensure(required_bytes)?;
            if len == output.len() || name.len > names.len() - bytes_len {
                unread_rows = unread_rows
                    .checked_add(1)
                    .ok_or("agency row count overflow")?;
                continue;
            }
            let active = Array::read(memory, at(pointer, OPERATIVES)?, 8)
                .ok_or("agency active roster unreadable or invalid")?;
            Literal::copy_all(memory, [&name], &mut names[bytes_len..bytes_len + name.len])?;
            output[len] = Row {
                country: country.id,
                upgrade_progress_raw: field(memory, pointer, UPGRADE_PROGRESS)?,
                defense_raw: field(memory, pointer, DEFENSE)?,
                active_count: active.count as i32,
                admission_ceiling: field(memory, pointer, OPERATIVE_ADMISSION_CEILING)?,
                unknown_f0: field(memory, pointer, UNKNOWN_F0)?,
                created: field(memory, pointer, CREATED)?,
                name: Name {
                    start: bytes_len,
                    len: name.len,
                },
            };
            bytes_len += name.len;
            len += 1;
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
    result.unwrap_or_else(crate::csubject_relation::unavailable)
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CapturedOperativeRow {
    pub country: u32,
    pub ordinal: u32,
    pub operative: u64,
    /// None only when operative resolution returns null.
    pub owner: Option<u32>,
}

/// Live roster count only, summed through the same guarded header walk.
pub fn captured_operative_demand(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    match walk_captured_operatives(memory, base, game_state, &mut []) {
        Outcome::Rows { len } => Ok(len),
        Outcome::Truncated { len, unread_rows } => len
            .checked_add(unread_rows as usize)
            .ok_or("captured operative row count overflow"),
        Outcome::Unavailable { reason } => Err(reason),
    }
}

/// Walk the captured-operative array at +0x108/count +0x114, stride 0x38,
/// resolve each +0xC handle and read the operative's raw owner at +0x120.
/// Never call the game.
pub fn walk_captured_operatives(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut [CapturedOperativeRow],
) -> Outcome {
    walk_captured_operatives_buffer(memory, base, game_state, crate::walk::Buffer::Fixed(output))
}

pub fn walk_captured_operatives_growing(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    output: &mut Vec<CapturedOperativeRow>,
) -> Outcome {
    walk_captured_operatives_buffer(
        memory,
        base,
        game_state,
        crate::walk::Buffer::Growing(output),
    )
}

pub fn walk_captured_operatives_buffer(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    mut output: crate::walk::Buffer<'_, CapturedOperativeRow>,
) -> Outcome {
    use crate::{
        ccaptured_operative_reference as captured, coperative_leader, creference_registry,
    };
    let result = (|| {
        let countries = ccountry::Reader::new(memory, base, game_state)?;
        let (mut len, mut unread_rows) = (0_usize, 0_u32);
        for slot in 1..countries.count() {
            let Some(country) = countries.at(slot)? else {
                continue;
            };
            let Some(agency) = root(memory, base, country)? else {
                continue;
            };
            let array = Array::read(
                memory,
                at(agency, CAPTURED_OPERATIVE_REFERENCES)?,
                captured::STRIDE,
            )
            .ok_or("captured operative roster unreadable or invalid")?;
            output.ensure(
                len.checked_add(array.count as usize)
                    .ok_or("captured operative row demand overflow")?,
            )?;
            for ordinal in 0..array.count {
                if len == output.len() {
                    unread_rows = unread_rows
                        .checked_add(array.count - ordinal)
                        .ok_or("captured operative row count overflow")?;
                    break;
                }
                let pointer = array
                    .element(ordinal, captured::STRIDE)
                    .ok_or("captured operative entry address overflow")?;
                let header = read::<[u8; 0x14]>(memory, pointer)
                    .ok_or("captured operative entry unreadable")?;
                if u64::from_le_bytes(header[..8].try_into().unwrap())
                    != base
                        .checked_add(captured::VTABLE_RVA)
                        .ok_or("module address overflow")? as u64
                {
                    return Err("captured operative entry vtable mismatch");
                }
                let operative = u64::from_le_bytes(header[0xC..0x14].try_into().unwrap());
                let owner = match creference_registry::resolve(memory, base, operative)? {
                    None => None,
                    Some(leader) => {
                        let identity = read::<[u8; 16]>(memory, leader)
                            .ok_or("captured operative referent unreadable")?;
                        if u64::from_le_bytes(identity[..8].try_into().unwrap())
                            != base
                                .checked_add(coperative_leader::VTABLE_RVA)
                                .ok_or("module address overflow")?
                                as u64
                            || u64::from_le_bytes(identity[8..].try_into().unwrap()) != operative
                        {
                            return Err("captured operative referent identity mismatch");
                        }
                        Some(
                            read::<u32>(memory, at(leader, coperative_leader::OWNER_COUNTRY)?)
                                .ok_or("captured operative owner unreadable")?,
                        )
                    }
                };
                output[len] = CapturedOperativeRow {
                    country: country.id,
                    ordinal,
                    operative,
                    owner,
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
    result.unwrap_or_else(|reason| Outcome::Unavailable { reason })
}

pub(crate) fn root(
    memory: &impl Memory,
    base: usize,
    country: ccountry::Country,
) -> Result<Option<usize>, &'static str> {
    let pointer = usize::try_from(field::<u64>(
        memory,
        country.pointer,
        ccountry::INTELLIGENCE_AGENCY,
    )?)
    .map_err(|_| "agency pointer overflow")?;
    if pointer == 0 {
        return Ok(None);
    }
    let expected = base
        .checked_add(VTABLE_RVA)
        .ok_or("agency vtable address overflow")?;
    if read::<u64>(memory, pointer) != Some(expected as u64) {
        return Err("agency vtable mismatch");
    }
    if field::<u64>(memory, pointer, OWNER_COUNTRY)? != country.pointer as u64 {
        return Err("agency owner pointer mismatch");
    }
    Ok(Some(pointer))
}

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer.checked_add(offset).ok_or("agency address overflow")
}

fn field<T: Raw>(memory: &impl Memory, pointer: usize, offset: usize) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or("agency field unreadable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{cgame_state, memory::SliceMemory};

    fn walk(memory: &impl Memory, base: usize, state: usize, rows: &mut [Row]) -> Outcome {
        super::walk(memory, base, state, rows, &mut []).outcome
    }

    fn put<const N: usize>(bytes: &mut [u8], offset: usize, value: [u8; N]) {
        bytes[offset..offset + N].copy_from_slice(&value);
    }

    fn fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x6000];
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES,
            0x2000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRIES + 12,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS,
            0x2100_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS + 8,
            2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x100 + cgame_state::COUNTRY_LINKS + 12,
            2_u32.to_le_bytes(),
        );
        put(&mut bytes, 0x2008, 0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x2104, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000,
            (ccountry::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(&mut bytes, 0x3008, 1_u32.to_le_bytes());
        put(
            &mut bytes,
            0x3000 + ccountry::INTELLIGENCE_AGENCY,
            0x5000_u64.to_le_bytes(),
        );
        put(&mut bytes, 0x5000, (VTABLE_RVA as u64).to_le_bytes());
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3000_u64.to_le_bytes());
        put(&mut bytes, 0x5000 + NAME + 24, 15_u64.to_le_bytes());
        bytes
    }

    #[test]
    fn growing_agency_name_arena_tracks_later_names_without_country_growth() {
        let mut bytes = fixture();
        let mut rows = Vec::new();
        let mut names = Vec::new();
        let mut upgrades = Vec::new();
        let mut catalog = crate::agency_upgrade_levels::Catalog::new(0);
        for count in [3_usize, 5_000] {
            const PAYLOAD: usize = 0x7000;
            bytes.resize(PAYLOAD + count + 1, 0);
            let header = 0x5000 + NAME;
            put(&mut bytes, header, (PAYLOAD as u64).to_le_bytes());
            put(&mut bytes, header + 16, (count as u64).to_le_bytes());
            put(
                &mut bytes,
                header + 24,
                (count.max(16) as u64).to_le_bytes(),
            );
            bytes[PAYLOAD..PAYLOAD + count].fill(0xfe);
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let mut expected_rows = [Row::default(); 1];
            let mut expected_names = vec![0; count];
            let expected = super::walk(&memory, 0, 0x100, &mut expected_rows, &mut expected_names);
            assert_eq!(expected.outcome, Outcome::Rows { len: 1 });
            let actual = walk_with_upgrades_growing(
                &memory,
                0,
                0x100,
                &mut rows,
                &mut names,
                &mut upgrades,
                &mut catalog,
            );
            assert_eq!(actual.agencies, expected);
            assert_eq!(&rows[..1], &expected_rows);
            assert_eq!(&names[..count], expected_names.as_slice());
        }
    }

    #[test]
    fn captured_roster_preserves_nulls_order_and_resolved_owner_without_active_membership() {
        use crate::{
            ccaptured_operative_reference as captured, coperative_leader,
            creference_registry as registry, reference_bucket,
        };
        let handle = (17_u64 << 32) | 54;
        let mut bytes = fixture();
        bytes.resize(0x7000, 0);
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES,
            0x6000_u64.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES + 8,
            3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + CAPTURED_OPERATIVE_REFERENCES + 12,
            3_i32.to_le_bytes(),
        );
        for ordinal in 0..3 {
            put(
                &mut bytes,
                0x6000 + ordinal * captured::STRIDE,
                (captured::VTABLE_RVA as u64).to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            0x6000 + captured::STRIDE + captured::OPERATIVE_HANDLE,
            handle.to_le_bytes(),
        );
        put(
            &mut bytes,
            0x6000 + 2 * captured::STRIDE + captured::OPERATIVE_HANDLE,
            (handle + (1 << 32)).to_le_bytes(),
        );
        let mut memory = registry::tests::Fixture::default();
        memory.put(0, &bytes);
        memory.put(registry::LOW_ROOTS_RVA + 54 * 8, &0x8000_u64.to_le_bytes());
        memory.put(0x8000 + registry::ENTRIES, &0x9000_u64.to_le_bytes());
        memory.put(0x8000 + registry::COUNT, &1_u32.to_le_bytes());
        memory.put(0x8000 + registry::MASK, &7_u32.to_le_bytes());
        memory.put(0x8000 + registry::OVERFLOW_DISTANCE, &[5]);
        memory.put(0x9000, &[0; 14 * reference_bucket::STRIDE]);
        memory.put(
            0x9000 + 13 * reference_bucket::STRIDE + reference_bucket::DISTANCE,
            &[255],
        );
        let index = registry::tests::Fixture::index(handle);
        let bucket = 0x9000 + index * reference_bucket::STRIDE;
        memory.put(bucket + reference_bucket::DISTANCE, &[1]);
        memory.put(bucket + reference_bucket::TYPE, &handle.to_le_bytes());
        memory.put(bucket + reference_bucket::OBJECT, &0xa000_u64.to_le_bytes());
        memory.put(
            0xa000,
            &(coperative_leader::VTABLE_RVA as u64).to_le_bytes(),
        );
        memory.put(0xa008, &handle.to_le_bytes());
        memory.put(
            0xa000 + coperative_leader::OWNER_COUNTRY,
            &5000_u32.to_le_bytes(),
        );
        let mut rows = [CapturedOperativeRow::default(); 3];
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows),
            Outcome::Rows { len: 3 }
        );
        assert_eq!(rows.map(|row| row.owner), [None, Some(5000), None]);
        assert_eq!(rows.map(|row| row.ordinal), [0, 1, 2]);
        assert_eq!(rows[1].operative, handle);
        assert_eq!(captured_operative_demand(&memory, 0, 0x100), Ok(3));
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows[..1]),
            Outcome::Truncated {
                len: 1,
                unread_rows: 2
            }
        );
        memory.put(0xa008, &0_u64.to_le_bytes());
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows),
            Outcome::Unavailable {
                reason: "captured operative referent identity mismatch"
            }
        );
    }

    #[test]
    fn captured_operative_demand_and_rows_follow_5000_live_countries() {
        const COUNT: usize = 5_000;
        const OBJECTS: usize = 0x20000;
        const STRIDE: usize = 0x1400;
        let mut bytes = vec![0; OBJECTS + COUNT * STRIDE];
        for (offset, data, stride) in [
            (cgame_state::COUNTRIES, 0x1000, 8),
            (cgame_state::COUNTRY_LINKS, 0xb000, 4),
        ] {
            put(&mut bytes, 0x100 + offset, (data as u64).to_le_bytes());
            put(
                &mut bytes,
                0x100 + offset + 8,
                ((COUNT + 1) as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                0x100 + offset + 12,
                ((COUNT + 1) as i32).to_le_bytes(),
            );
            for slot in 1..=COUNT {
                if stride == 8 {
                    put(
                        &mut bytes,
                        data + slot * stride,
                        ((OBJECTS + (slot - 1) * STRIDE) as u64).to_le_bytes(),
                    );
                } else {
                    put(
                        &mut bytes,
                        data + slot * stride,
                        (slot as u32).to_le_bytes(),
                    );
                }
            }
        }
        put(
            &mut bytes,
            0x18000,
            (crate::ccaptured_operative_reference::VTABLE_RVA as u64).to_le_bytes(),
        );
        for slot in 1..=COUNT {
            let country = OBJECTS + (slot - 1) * STRIDE;
            let agency = country + 0x1000;
            put(
                &mut bytes,
                country,
                (ccountry::VTABLE_RVA as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::ID,
                (slot as u32).to_le_bytes(),
            );
            put(
                &mut bytes,
                country + ccountry::INTELLIGENCE_AGENCY,
                (agency as u64).to_le_bytes(),
            );
            put(&mut bytes, agency, (VTABLE_RVA as u64).to_le_bytes());
            put(
                &mut bytes,
                agency + OWNER_COUNTRY,
                (country as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES,
                0x18000_u64.to_le_bytes(),
            );
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES + 8,
                1_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES + 12,
                1_i32.to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(captured_operative_demand(&memory, 0, 0x100), Ok(COUNT));
        let mut rows = Vec::new();
        assert_eq!(
            walk_captured_operatives_growing(&memory, 0, 0x100, &mut rows),
            Outcome::Rows { len: COUNT }
        );
        assert_eq!(
            rows[COUNT - 1],
            CapturedOperativeRow {
                country: COUNT as u32,
                ordinal: 0,
                operative: 0,
                owner: None
            }
        );
        let data = 0x19000;
        for ordinal in 0..10 {
            put(
                &mut bytes,
                data + ordinal * crate::ccaptured_operative_reference::STRIDE,
                (crate::ccaptured_operative_reference::VTABLE_RVA as u64).to_le_bytes(),
            );
        }
        for slot in 1..=COUNT {
            let agency = OBJECTS + (slot - 1) * STRIDE + 0x1000;
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES,
                (data as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES + 8,
                10_u32.to_le_bytes(),
            );
            put(
                &mut bytes,
                agency + CAPTURED_OPERATIVE_REFERENCES + 12,
                10_i32.to_le_bytes(),
            );
        }
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(captured_operative_demand(&memory, 0, 0x100), Ok(COUNT * 10));
        assert_eq!(
            walk_captured_operatives(&memory, 0, 0x100, &mut rows[..COUNT]),
            Outcome::Truncated {
                len: COUNT,
                unread_rows: (COUNT * 9) as u32
            }
        );
        assert_eq!(
            walk_captured_operatives_growing(&memory, 0, 0x100, &mut rows),
            Outcome::Rows { len: COUNT * 10 }
        );
        for (index, row) in rows[..COUNT * 10].iter().enumerate() {
            assert_eq!(row.country as usize, index / 10 + 1);
            assert_eq!(row.ordinal as usize, index % 10);
            assert_eq!((row.operative, row.owner), (0, None));
        }
    }

    #[test]
    fn signed_qwords_and_capacity_are_preserved() {
        let mut bytes = fixture();
        put(
            &mut bytes,
            0x5000 + UPGRADE_PROGRESS,
            (i64::MAX - 7).to_le_bytes(),
        );
        put(&mut bytes, 0x5000 + DEFENSE, (i64::MIN + 9).to_le_bytes());
        put(
            &mut bytes,
            0x5000 + OPERATIVE_ADMISSION_CEILING,
            (-3_i32).to_le_bytes(),
        );
        put(&mut bytes, 0x5000 + UNKNOWN_F0, u32::MAX.to_le_bytes());
        // The raw byte, not a normalised boolean.
        put(&mut bytes, 0x5000 + CREATED, [0x7f]);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut rows = [Row::default(); 1];
        assert_eq!(walk(&memory, 0, 0x100, &mut rows), Outcome::Rows { len: 1 });
        assert_eq!(
            rows[0],
            Row {
                country: 1,
                upgrade_progress_raw: i64::MAX - 7,
                defense_raw: i64::MIN + 9,
                active_count: 0,
                admission_ceiling: -3,
                unknown_f0: u32::MAX,
                created: 0x7f,
                name: Default::default(),
            }
        );
        assert_eq!(
            walk(&memory, 0, 0x100, &mut []),
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
    }

    #[test]
    fn null_agency_is_empty_but_bad_owner_or_partial_scalar_is_unavailable() {
        let mut bytes = fixture();
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3008_u64.to_le_bytes());
        assert_eq!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes
                },
                0,
                0x100,
                &mut [Row::default()]
            ),
            Outcome::Unavailable {
                reason: "agency owner pointer mismatch"
            }
        );
        put(&mut bytes, 0x5000 + OWNER_COUNTRY, 0x3000_u64.to_le_bytes());
        assert!(matches!(
            walk(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes[..0x5000 + DEFENSE + 7]
                },
                0,
                0x100,
                &mut [Row::default()]
            ),
            Outcome::Unavailable { .. }
        ));
        put(
            &mut bytes,
            0x3000 + ccountry::INTELLIGENCE_AGENCY,
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
                &mut [Row::default()]
            ),
            Outcome::Rows { len: 0 }
        );
    }
    #[test]
    fn heap_name_demand_capacity_and_terminator_are_exact() {
        let mut bytes = fixture();
        let text = vec![b'x'; 40_000];
        bytes.resize(0x6000 + text.len() + 1, 0);
        bytes[0x6000..0x6000 + text.len()].copy_from_slice(&text);
        put(&mut bytes, 0x5000 + NAME, 0x6000_u64.to_le_bytes());
        put(
            &mut bytes,
            0x5000 + NAME + 16,
            (text.len() as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            0x5000 + NAME + 24,
            (text.len() as u64).to_le_bytes(),
        );
        let mut rows = [Row::default()];
        let mut names = vec![0; text.len()];
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
        assert_eq!(read.outcome, Outcome::Rows { len: 1 });
        assert_eq!(read.required_bytes, text.len());
        assert_eq!(names, text);
        let short = super::walk(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            0,
            0x100,
            &mut rows,
            &mut names[..39_999],
        );
        assert_eq!(
            short.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 1
            }
        );
        assert_eq!((short.bytes_len, short.required_bytes), (0, text.len()));
        bytes[0x6000 + text.len()] = 1;
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
}
