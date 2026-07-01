use std::collections::BTreeSet;
use std::fmt;

use crate::domain47_characters_leaders_cabinet_live as compact;

pub const DOMAIN_ID: u16 = compact::DOMAIN_ID;
pub const DOMAIN_NAME: &str = compact::DOMAIN_NAME;

pub const CCHARACTER_RTTI_RVA: u32 = 0x2967E30;
pub const CARMY_LEADER_RTTI_RVA: u32 = 0x293F538;
pub const CNAVY_LEADER_RTTI_RVA: u32 = 0x293F650;
pub const CCOUNTRY_LEADER_RTTI_RVA: u32 = 0x296A720;
pub const CADVISOR_RTTI_RVA: u32 = 0x2990E60;
pub const CUNIT_LEADER_TRAIT_RTTI_RVA: u32 = 0x292F160;

pub const CCOUNTRY_POLITICAL_STATUS_OFF: u64 = 0xF90;
pub const CPOLITICAL_STATUS_RULING_PARTY_OFF: u64 = 0xD0;
pub const CPOLITICAL_STATUS_IDEAS_DATA_OFF: u64 = 0x50;
pub const CPOLITICAL_STATUS_IDEAS_SIZE_OFF: u64 = 0x5C;
pub const CPOLITICAL_PARTY_LEADER_VEC_OFF: u64 = 0x70;

pub const CCHARACTER_TYPE_OFF: u64 = 0x08;
pub const CCHARACTER_ID_OFF: u64 = 0x0C;
pub const CCHARACTER_NAME_OFF: u64 = 0x28;
pub const CCHARACTER_TOKEN_OFF: u64 = 0x48;
pub const CCHARACTER_COUNTRY_TAG_DB_ID_OFF: u64 = 0x68;
pub const CCHARACTER_UNIT_LEADER_PTR_OFF: u64 = 0xA8;
pub const CCHARACTER_TYPE_ID: u32 = 73;

pub const CCOUNTRY_LEADER_CHARACTER_PTR_OFF: u64 = 0x08;

pub const CADVISOR_CHARACTER_PTR_OFF: u64 = 0x10;
pub const CADVISOR_IDEA_TOKEN_OFF: u64 = 0x30;
pub const CADVISOR_SLOT_OFF: u64 = 0x80;

pub const CUNIT_LEADER_TYPE_OFF: u64 = 0x08;
pub const CUNIT_LEADER_ID_OFF: u64 = 0x0C;
pub const CUNIT_LEADER_TOKEN_OFF: u64 = 0x20;
pub const CUNIT_LEADER_NAME_OFF: u64 = 0x40;
pub const CUNIT_LEADER_CHARACTER_ID_COPY_OFF: u64 = 0xF4C;
pub const CUNIT_LEADER_SKILL_SOURCE_PTR_OFF: u64 = 0xE60;
pub const CUNIT_LEADER_SKILL_VALUE_OFF: u64 = 0x1B8;
pub const CUNIT_LEADER_TRAIT_DATA_OFF: u64 = 0xDC8;
pub const CUNIT_LEADER_TRAIT_CAPACITY_OFF: u64 = 0xDD0;
pub const CUNIT_LEADER_TRAIT_SIZE_OFF: u64 = 0xDD4;
pub const CUNIT_LEADER_TYPE_ID: u32 = 4713;
pub const MAX_UNIT_LEADER_TRAITS_HOT: u32 = 64;

pub const CUNIT_LEADER_TRAIT_TOKEN_OFF: u64 = 0x10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentBuildRttiRoots {
    pub ccharacter: u32,
    pub carmy_leader: u32,
    pub cnavy_leader: u32,
    pub ccountry_leader: u32,
    pub cadvisor: u32,
    pub cunit_leader_trait: u32,
}

pub const CURRENT_BUILD_RTTI_ROOTS: CurrentBuildRttiRoots = CurrentBuildRttiRoots {
    ccharacter: CCHARACTER_RTTI_RVA,
    carmy_leader: CARMY_LEADER_RTTI_RVA,
    cnavy_leader: CNAVY_LEADER_RTTI_RVA,
    ccountry_leader: CCOUNTRY_LEADER_RTTI_RVA,
    cadvisor: CADVISOR_RTTI_RVA,
    cunit_leader_trait: CUNIT_LEADER_TRAIT_RTTI_RVA,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedTypeRoots {
    pub ccharacter_vtable: u64,
    pub carmy_leader_vtable: u64,
    pub cnavy_leader_vtable: u64,
    pub ccountry_leader_vtable: u64,
    pub cadvisor_vtable: u64,
    pub cunit_leader_trait_vtable: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRecord {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderRecord {
    pub country_tag_db_id: u32,
    pub unit_leader_id: u32,
    pub char_id: u32,
    pub leader_kind: u8,
    pub skill: Option<u32>,
    pub experience_raw: Option<i32>,
    pub attack: Option<u8>,
    pub defense: Option<u8>,
    pub planning: Option<u8>,
    pub logistics: Option<u8>,
    pub maneuvering: Option<u8>,
    pub coordination: Option<u8>,
    pub token: Option<String>,
    pub name: Option<String>,
    pub trait_count: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderTraitRecord {
    pub unit_leader_id: u32,
    pub ordinal: u16,
    pub trait_token: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryLeaderRoleRecord {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub name: Option<String>,
    pub token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveRulerRecord {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub ruling_party: Option<String>,
    pub name: Option<String>,
    pub token: Option<String>,
    pub leader_vec_size: u16,
    pub ambiguous: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRecord {
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub slot: Option<String>,
    pub ledger: Option<String>,
    pub idea_token: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaneFrame {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub schema_flags: u32,
    pub characters: Vec<CharacterRecord>,
    pub unit_leaders: Vec<UnitLeaderRecord>,
    pub unit_leader_traits: Vec<UnitLeaderTraitRecord>,
    pub country_leader_roles: Vec<CountryLeaderRoleRecord>,
    pub active_rulers: Vec<ActiveRulerRecord>,
    pub advisor_roster: Vec<AdvisorRecord>,
    pub appointed_advisors: Vec<AdvisorRecord>,
}

impl LaneFrame {
    pub fn new(elapsed_wall_ms: u64) -> Self {
        Self {
            elapsed_wall_ms,
            schema_flags: compact::SCHEMA_ROOTS_VALIDATED
                | compact::SCHEMA_STATIC_NAME_CLOSURE
                | compact::SCHEMA_APPOINTED_FROM_ACTIVE_IDEAS,
            ..Self::default()
        }
    }

    pub fn compact_rows(&self) -> Vec<compact::CharacterLaneRow<'_>> {
        let mut rows = Vec::with_capacity(
            self.characters.len()
                + self.unit_leaders.len()
                + self.unit_leader_traits.len()
                + self.country_leader_roles.len()
                + self.active_rulers.len()
                + self.advisor_roster.len()
                + self.appointed_advisors.len(),
        );
        rows.extend(self.characters.iter().map(|row| {
            compact::CharacterLaneRow::Character(compact::CharacterRow {
                country_tag_db_id: row.country_tag_db_id,
                char_id: row.char_id,
                name: row.name.as_deref(),
                token: row.token.as_deref(),
            })
        }));
        rows.extend(self.unit_leaders.iter().map(|row| {
            compact::CharacterLaneRow::UnitLeader(compact::UnitLeaderRow {
                country_tag_db_id: row.country_tag_db_id,
                unit_leader_id: row.unit_leader_id,
                char_id: row.char_id,
                leader_kind: row.leader_kind,
                skill: row.skill,
                experience_raw: row.experience_raw,
                attack: row.attack,
                defense: row.defense,
                planning: row.planning,
                logistics: row.logistics,
                maneuvering: row.maneuvering,
                coordination: row.coordination,
                token: row.token.as_deref(),
                name: row.name.as_deref(),
                trait_count: row.trait_count,
            })
        }));
        rows.extend(self.unit_leader_traits.iter().map(|row| {
            compact::CharacterLaneRow::UnitLeaderTrait(compact::UnitLeaderTraitRow {
                unit_leader_id: row.unit_leader_id,
                ordinal: row.ordinal,
                trait_token: row.trait_token.as_str(),
            })
        }));
        rows.extend(self.country_leader_roles.iter().map(|row| {
            compact::CharacterLaneRow::CountryLeaderRole(compact::CountryLeaderRoleRow {
                country_tag_db_id: row.country_tag_db_id,
                char_id: row.char_id,
                name: row.name.as_deref(),
                token: row.token.as_deref(),
            })
        }));
        rows.extend(self.active_rulers.iter().map(|row| {
            compact::CharacterLaneRow::ActiveRuler(compact::ActiveRulerRow {
                country_tag_db_id: row.country_tag_db_id,
                char_id: row.char_id,
                ruling_party: row.ruling_party.as_deref(),
                name: row.name.as_deref(),
                token: row.token.as_deref(),
                leader_vec_size: row.leader_vec_size,
                ambiguous: row.ambiguous,
            })
        }));
        rows.extend(self.advisor_roster.iter().map(|row| {
            compact::CharacterLaneRow::AdvisorRoster(compact::AdvisorRow {
                country_tag_db_id: row.country_tag_db_id,
                char_id: row.char_id,
                slot: row.slot.as_deref(),
                ledger: row.ledger.as_deref(),
                idea_token: row.idea_token.as_str(),
            })
        }));
        rows.extend(self.appointed_advisors.iter().map(|row| {
            compact::CharacterLaneRow::AppointedAdvisor(compact::AdvisorRow {
                country_tag_db_id: row.country_tag_db_id,
                char_id: row.char_id,
                slot: row.slot.as_deref(),
                ledger: row.ledger.as_deref(),
                idea_token: row.idea_token.as_str(),
            })
        }));
        rows
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CabinetBucket {
    PoliticalAdvisor,
    ArmyChief,
    NavyChief,
    AirChief,
    HighCommand,
    Theorist,
    Unknown,
}

impl CabinetBucket {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PoliticalAdvisor => "political_advisor",
            Self::ArmyChief => "army_chief",
            Self::NavyChief => "navy_chief",
            Self::AirChief => "air_chief",
            Self::HighCommand => "high_command",
            Self::Theorist => "theorist",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_slot(value: Option<&str>) -> Self {
        match value.unwrap_or_default() {
            "political_advisor" => Self::PoliticalAdvisor,
            "army_chief" => Self::ArmyChief,
            "navy_chief" => Self::NavyChief,
            "air_chief" => Self::AirChief,
            "high_command" => Self::HighCommand,
            "theorist" => Self::Theorist,
            _ => Self::Unknown,
        }
    }
}

pub fn advisor_bucket(slot: Option<&str>, ledger: Option<&str>, idea_token: &str) -> CabinetBucket {
    let direct = CabinetBucket::from_slot(slot.or(ledger));
    if direct != CabinetBucket::Unknown {
        return direct;
    }
    for suffix in [
        "political_advisor",
        "army_chief",
        "navy_chief",
        "air_chief",
        "high_command",
        "theorist",
    ] {
        if idea_token.ends_with(suffix) {
            return CabinetBucket::from_slot(Some(suffix));
        }
    }
    CabinetBucket::Unknown
}

pub fn appointed_advisors_from_active_ideas(
    roster: &[AdvisorRecord],
    active_idea_tokens: &[&str],
) -> Vec<AdvisorRecord> {
    let active: BTreeSet<&str> = active_idea_tokens.iter().copied().collect();
    let mut seen = BTreeSet::new();
    roster
        .iter()
        .filter(|row| active.contains(row.idea_token.as_str()))
        .filter(|row| {
            seen.insert((
                row.country_tag_db_id,
                row.char_id,
                row.idea_token.as_str().to_string(),
            ))
        })
        .cloned()
        .collect()
}

pub fn encode_frame_payload(frame: &LaneFrame) -> Vec<u8> {
    let rows = frame.compact_rows();
    compact::encode_payload(
        frame.elapsed_wall_ms,
        frame.status_flags,
        frame.schema_flags,
        &rows,
    )
}

pub fn encode_frame_record(sequence: u64, game_hour_raw: i32, frame: &LaneFrame) -> Vec<u8> {
    let payload = encode_frame_payload(frame);
    compact::encode_compact_data_record(sequence, game_hour_raw, &payload)
}

pub fn decode_frame_payload(offset: usize, payload: &[u8]) -> compact::Domain44GateSummary {
    compact::decode_payload(offset, payload)
}

pub trait LaneMemory {
    fn read_u8(&self, addr: u64) -> Option<u8>;

    fn read_u16_le(&self, addr: u64) -> Option<u16> {
        let bytes = self.read_array::<2>(addr)?;
        Some(u16::from_le_bytes(bytes))
    }

    fn read_u32_le(&self, addr: u64) -> Option<u32> {
        let bytes = self.read_array::<4>(addr)?;
        Some(u32::from_le_bytes(bytes))
    }

    fn read_i32_le(&self, addr: u64) -> Option<i32> {
        let bytes = self.read_array::<4>(addr)?;
        Some(i32::from_le_bytes(bytes))
    }

    fn read_u64_le(&self, addr: u64) -> Option<u64> {
        let bytes = self.read_array::<8>(addr)?;
        Some(u64::from_le_bytes(bytes))
    }

    fn read_array<const N: usize>(&self, addr: u64) -> Option<[u8; N]> {
        let mut out = [0u8; N];
        for (idx, byte) in out.iter_mut().enumerate() {
            *byte = self.read_u8(addr.checked_add(idx as u64)?)?;
        }
        Some(out)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRawFields {
    pub character_addr: u64,
    pub country_tag_db_id: u32,
    pub char_id: u32,
    pub unit_leader_ptr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdvisorRawFields {
    pub advisor_addr: u64,
    pub character_addr: u64,
    pub country_tag_db_id: u32,
    pub char_id: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnitLeaderTraitVector {
    pub data_ptr: u64,
    pub capacity: u32,
    pub size: u32,
    pub trait_ptrs: Vec<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    RegistryMismatch(String),
    AddressOverflow(&'static str),
    NullPointer(&'static str),
    MissingRead {
        addr: u64,
        field: &'static str,
    },
    BadVtable {
        addr: u64,
        field: &'static str,
        expected: u64,
        observed: u64,
    },
    BadTypeId {
        addr: u64,
        field: &'static str,
        expected: u32,
        observed: u32,
    },
    BadVector {
        field: &'static str,
        begin: u64,
        end: u64,
        size: u64,
    },
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RegistryMismatch(msg) => write!(f, "{msg}"),
            Self::AddressOverflow(field) => write!(f, "address overflow while reading {field}"),
            Self::NullPointer(field) => write!(f, "null pointer while reading {field}"),
            Self::MissingRead { addr, field } => {
                write!(f, "missing read for {field} at 0x{addr:X}")
            }
            Self::BadVtable {
                addr,
                field,
                expected,
                observed,
            } => write!(
                f,
                "bad vtable for {field} at 0x{addr:X}: observed 0x{observed:X}, expected 0x{expected:X}"
            ),
            Self::BadTypeId {
                addr,
                field,
                expected,
                observed,
            } => write!(
                f,
                "bad type id for {field} at 0x{addr:X}: observed {observed}, expected {expected}"
            ),
            Self::BadVector {
                field,
                begin,
                end,
                size,
            } => write!(
                f,
                "bad vector for {field}: begin=0x{begin:X}, end=0x{end:X}, size={size}"
            ),
        }
    }
}

impl std::error::Error for ExtractError {}

pub fn validate_registry_assignment(registry_json: &str) -> Result<(), ExtractError> {
    let id_anchor = "\"id\": 47";
    let Some(id_pos) = registry_json.find(id_anchor) else {
        return Err(ExtractError::RegistryMismatch(
            "domain registry does not assign id 47".to_string(),
        ));
    };
    let window = &registry_json[id_pos..registry_json.len().min(id_pos + 220)];
    if !window.contains("\"name\": \"characters_leaders_cabinet_live\"") {
        return Err(ExtractError::RegistryMismatch(
            "domain registry id 47 is not characters_leaders_cabinet_live".to_string(),
        ));
    }
    if !(window.contains("\"status\": \"reserved\"")
        || window.contains("\"status\": \"active_integration\"")
        || window.contains("\"status\": \"production_complete\""))
    {
        return Err(ExtractError::RegistryMismatch(
            "domain registry id 47 is not in an accepted integration status".to_string(),
        ));
    }
    Ok(())
}

pub fn read_character_raw_fields<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    character_addr: u64,
) -> Result<CharacterRawFields, ExtractError> {
    require_vtable(mem, character_addr, roots.ccharacter_vtable, "CCharacter")?;
    require_type_id(
        mem,
        checked_add(character_addr, CCHARACTER_TYPE_OFF, "CCharacter.type")?,
        CCHARACTER_TYPE_ID,
        "CCharacter.type",
    )?;
    let char_id = read_u32(mem, character_addr, CCHARACTER_ID_OFF, "CCharacter.char_id")?;
    let country_tag_db_id = read_u32(
        mem,
        character_addr,
        CCHARACTER_COUNTRY_TAG_DB_ID_OFF,
        "CCharacter.country_tag_db_id",
    )?;
    let unit_leader_ptr = read_u64(
        mem,
        character_addr,
        CCHARACTER_UNIT_LEADER_PTR_OFF,
        "CCharacter.unit_leader_ptr",
    )?;
    Ok(CharacterRawFields {
        character_addr,
        country_tag_db_id,
        char_id,
        unit_leader_ptr,
    })
}

pub fn read_advisor_raw_fields<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    advisor_addr: u64,
) -> Result<AdvisorRawFields, ExtractError> {
    require_vtable(mem, advisor_addr, roots.cadvisor_vtable, "CAdvisor")?;
    let character_addr = read_nonzero_u64(
        mem,
        advisor_addr,
        CADVISOR_CHARACTER_PTR_OFF,
        "CAdvisor.character",
    )?;
    let character = read_character_raw_fields(mem, roots, character_addr)?;
    Ok(AdvisorRawFields {
        advisor_addr,
        character_addr,
        country_tag_db_id: character.country_tag_db_id,
        char_id: character.char_id,
    })
}

pub fn read_active_ruler_from_country<M: LaneMemory>(
    mem: &M,
    roots: &ResolvedTypeRoots,
    country_tag_db_id: u32,
    country_addr: u64,
    ruling_party: Option<String>,
    name: Option<String>,
    token: Option<String>,
) -> Result<ActiveRulerRecord, ExtractError> {
    let status = read_nonzero_u64(
        mem,
        country_addr,
        CCOUNTRY_POLITICAL_STATUS_OFF,
        "CCountry.political_status",
    )?;
    let party = read_nonzero_u64(
        mem,
        status,
        CPOLITICAL_STATUS_RULING_PARTY_OFF,
        "CPoliticalStatus.ruling_party",
    )?;
    let begin = read_nonzero_u64(
        mem,
        party,
        CPOLITICAL_PARTY_LEADER_VEC_OFF,
        "CPoliticalParty.country_leaders.begin",
    )?;
    let end = read_u64(
        mem,
        party,
        CPOLITICAL_PARTY_LEADER_VEC_OFF + 8,
        "CPoliticalParty.country_leaders.end",
    )?;
    if end < begin || (end - begin) % 8 != 0 {
        return Err(ExtractError::BadVector {
            field: "CPoliticalParty.country_leaders",
            begin,
            end,
            size: 0,
        });
    }
    let size = (end - begin) / 8;
    if size == 0 || size > u16::MAX as u64 {
        return Err(ExtractError::BadVector {
            field: "CPoliticalParty.country_leaders",
            begin,
            end,
            size,
        });
    }
    let country_leader = read_nonzero_u64_at(mem, begin, "CCountryLeader[0]")?;
    require_vtable(
        mem,
        country_leader,
        roots.ccountry_leader_vtable,
        "CCountryLeader",
    )?;
    let character_addr = read_nonzero_u64(
        mem,
        country_leader,
        CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
        "CCountryLeader.character",
    )?;
    let character = read_character_raw_fields(mem, roots, character_addr)?;
    Ok(ActiveRulerRecord {
        country_tag_db_id,
        char_id: character.char_id,
        ruling_party,
        name,
        token,
        leader_vec_size: size as u16,
        ambiguous: size != 1,
    })
}

pub fn read_unit_leader_trait_vector<M: LaneMemory>(
    mem: &M,
    leader_addr: u64,
) -> Result<UnitLeaderTraitVector, ExtractError> {
    let data_ptr = read_u64(
        mem,
        leader_addr,
        CUNIT_LEADER_TRAIT_DATA_OFF,
        "CUnitLeader.traits.data",
    )?;
    let capacity = read_u32(
        mem,
        leader_addr,
        CUNIT_LEADER_TRAIT_CAPACITY_OFF,
        "CUnitLeader.traits.capacity",
    )?;
    let size = read_u32(
        mem,
        leader_addr,
        CUNIT_LEADER_TRAIT_SIZE_OFF,
        "CUnitLeader.traits.size",
    )?;
    if size > capacity || size > MAX_UNIT_LEADER_TRAITS_HOT {
        return Err(ExtractError::BadVector {
            field: "CUnitLeader.assigned_traits",
            begin: data_ptr,
            end: data_ptr.saturating_add((size as u64).saturating_mul(8)),
            size: size as u64,
        });
    }
    if size > 0 && data_ptr == 0 {
        return Err(ExtractError::NullPointer("CUnitLeader.traits.data"));
    }
    let mut trait_ptrs = Vec::with_capacity(size as usize);
    for ordinal in 0..size {
        trait_ptrs.push(read_nonzero_u64_at(
            mem,
            checked_add(
                data_ptr,
                (ordinal as u64).saturating_mul(8),
                "CUnitLeader.traits.element",
            )?,
            "CUnitLeader.traits.element",
        )?);
    }
    Ok(UnitLeaderTraitVector {
        data_ptr,
        capacity,
        size,
        trait_ptrs,
    })
}

pub fn read_unit_leader_skill<M: LaneMemory>(
    mem: &M,
    leader_addr: u64,
) -> Result<Option<u32>, ExtractError> {
    let skill_source = read_u64(
        mem,
        leader_addr,
        CUNIT_LEADER_SKILL_SOURCE_PTR_OFF,
        "CUnitLeader.skill_source",
    )?;
    if skill_source == 0 {
        return Ok(None);
    }
    Ok(Some(read_u32(
        mem,
        skill_source,
        CUNIT_LEADER_SKILL_VALUE_OFF,
        "CUnitLeader.skill",
    )?))
}

fn require_vtable<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u64,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadVtable {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn require_type_id<M: LaneMemory>(
    mem: &M,
    addr: u64,
    expected: u32,
    field: &'static str,
) -> Result<(), ExtractError> {
    let observed = mem
        .read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if observed != expected {
        return Err(ExtractError::BadTypeId {
            addr,
            field,
            expected,
            observed,
        });
    }
    Ok(())
}

fn read_u32<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u32, ExtractError> {
    let addr = checked_add(base, offset, field)?;
    mem.read_u32_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })
}

fn read_u64<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u64, ExtractError> {
    let addr = checked_add(base, offset, field)?;
    mem.read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })
}

fn read_nonzero_u64<M: LaneMemory>(
    mem: &M,
    base: u64,
    offset: u64,
    field: &'static str,
) -> Result<u64, ExtractError> {
    let value = read_u64(mem, base, offset, field)?;
    if value == 0 {
        return Err(ExtractError::NullPointer(field));
    }
    Ok(value)
}

fn read_nonzero_u64_at<M: LaneMemory>(
    mem: &M,
    addr: u64,
    field: &'static str,
) -> Result<u64, ExtractError> {
    let value = mem
        .read_u64_le(addr)
        .ok_or(ExtractError::MissingRead { addr, field })?;
    if value == 0 {
        return Err(ExtractError::NullPointer(field));
    }
    Ok(value)
}

fn checked_add(base: u64, offset: u64, field: &'static str) -> Result<u64, ExtractError> {
    base.checked_add(offset)
        .ok_or(ExtractError::AddressOverflow(field))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const CHAR_VT: u64 = 0x7FF6_D000_1000;
    const CL_VT: u64 = 0x7FF6_D000_2000;
    const ADV_VT: u64 = 0x7FF6_D000_3000;

    #[derive(Default)]
    struct FakeMemory {
        bytes: BTreeMap<u64, u8>,
    }

    impl FakeMemory {
        fn write_u32(&mut self, addr: u64, value: u32) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_u64(&mut self, addr: u64, value: u64) {
            self.write_bytes(addr, &value.to_le_bytes());
        }

        fn write_bytes(&mut self, addr: u64, bytes: &[u8]) {
            for (idx, byte) in bytes.iter().enumerate() {
                self.bytes.insert(addr + idx as u64, *byte);
            }
        }
    }

    impl LaneMemory for FakeMemory {
        fn read_u8(&self, addr: u64) -> Option<u8> {
            self.bytes.get(&addr).copied()
        }
    }

    fn roots() -> ResolvedTypeRoots {
        ResolvedTypeRoots {
            ccharacter_vtable: CHAR_VT,
            ccountry_leader_vtable: CL_VT,
            cadvisor_vtable: ADV_VT,
            ..ResolvedTypeRoots::default()
        }
    }

    fn write_character(mem: &mut FakeMemory, addr: u64, char_id: u32, tag_db_id: u32) {
        mem.write_u64(addr, CHAR_VT);
        mem.write_u32(addr + CCHARACTER_TYPE_OFF, CCHARACTER_TYPE_ID);
        mem.write_u32(addr + CCHARACTER_ID_OFF, char_id);
        mem.write_u32(addr + CCHARACTER_COUNTRY_TAG_DB_ID_OFF, tag_db_id);
        mem.write_u64(addr + CCHARACTER_UNIT_LEADER_PTR_OFF, 0);
    }

    #[test]
    fn registry_assigns_domain_47_to_this_lane() {
        let registry = include_str!("../../../domain_registry.json");
        validate_registry_assignment(registry).unwrap();
    }

    #[test]
    fn registry_rejects_wrong_domain_owner() {
        let registry = r#"{ "domains": [{ "id": 47, "name": "wrong", "status": "reserved" }] }"#;
        assert!(validate_registry_assignment(registry).is_err());
    }

    #[test]
    fn active_ruler_path_reads_party_leader_vector() {
        let mut mem = FakeMemory::default();
        let country = 0x1000;
        let status = 0x2000;
        let party = 0x3000;
        let vector = 0x4000;
        let country_leader = 0x5000;
        let character = 0x6000;
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, vector);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF + 8, vector + 8);
        mem.write_u64(vector, country_leader);
        mem.write_u64(country_leader, CL_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        write_character(&mut mem, character, 187, 33);

        let row = read_active_ruler_from_country(
            &mem,
            &roots(),
            33,
            country,
            Some("democratic".to_string()),
            Some("John Curtin".to_string()),
            Some("AST_john_curtin".to_string()),
        )
        .unwrap();

        assert_eq!(row.country_tag_db_id, 33);
        assert_eq!(row.char_id, 187);
        assert_eq!(row.ruling_party.as_deref(), Some("democratic"));
        assert_eq!(row.leader_vec_size, 1);
        assert!(!row.ambiguous);
    }

    #[test]
    fn active_ruler_marks_multi_leader_vector_ambiguous() {
        let mut mem = FakeMemory::default();
        let country = 0x1100;
        let status = 0x2100;
        let party = 0x3100;
        let vector = 0x4100;
        let country_leader = 0x5100;
        let character = 0x6100;
        mem.write_u64(country + CCOUNTRY_POLITICAL_STATUS_OFF, status);
        mem.write_u64(status + CPOLITICAL_STATUS_RULING_PARTY_OFF, party);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF, vector);
        mem.write_u64(party + CPOLITICAL_PARTY_LEADER_VEC_OFF + 8, vector + 16);
        mem.write_u64(vector, country_leader);
        mem.write_u64(vector + 8, country_leader);
        mem.write_u64(country_leader, CL_VT);
        mem.write_u64(
            country_leader + CCOUNTRY_LEADER_CHARACTER_PTR_OFF,
            character,
        );
        write_character(&mut mem, character, 1456, 2);

        let row =
            read_active_ruler_from_country(&mem, &roots(), 2, country, None, None, None).unwrap();
        assert_eq!(row.leader_vec_size, 2);
        assert!(row.ambiguous);
    }

    #[test]
    fn advisor_raw_identity_uses_advisor_character_tag_route() {
        let mut mem = FakeMemory::default();
        let advisor = 0x7000;
        let character = 0x8000;
        mem.write_u64(advisor, ADV_VT);
        mem.write_u64(advisor + CADVISOR_CHARACTER_PTR_OFF, character);
        write_character(&mut mem, character, 1459, 2);

        let row = read_advisor_raw_fields(&mem, &roots(), advisor).unwrap();
        assert_eq!(row.country_tag_db_id, 2);
        assert_eq!(row.char_id, 1459);
    }

    #[test]
    fn assigned_trait_vector_uses_current_dc8_membership_path() {
        let mut mem = FakeMemory::default();
        let leader = 0x9000;
        let data = 0xA000;
        mem.write_u64(leader + CUNIT_LEADER_TRAIT_DATA_OFF, data);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_CAPACITY_OFF, 3);
        mem.write_u32(leader + CUNIT_LEADER_TRAIT_SIZE_OFF, 2);
        mem.write_u64(data, 0xB000);
        mem.write_u64(data + 8, 0xB100);

        let traits = read_unit_leader_trait_vector(&mem, leader).unwrap();
        assert_eq!(traits.size, 2);
        assert_eq!(traits.trait_ptrs, vec![0xB000, 0xB100]);
    }

    #[test]
    fn cabinet_bucket_semantics_are_stable() {
        assert_eq!(
            advisor_bucket(Some("political_advisor"), None, "ENG_maxwell_aitken"),
            CabinetBucket::PoliticalAdvisor
        );
        assert_eq!(
            advisor_bucket(None, None, "generic_army_chief_def_army_chief"),
            CabinetBucket::ArmyChief
        );
        assert_eq!(
            advisor_bucket(None, None, "unclassified_character"),
            CabinetBucket::Unknown
        );
    }

    #[test]
    fn appointed_advisor_transition_fixture_encodes_domain47() {
        let roster = vec![
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 1459,
                slot: Some("political_advisor".to_string()),
                ledger: Some("political_advisor".to_string()),
                idea_token: "ENG_leslie_hore_belisha".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 1458,
                slot: Some("political_advisor".to_string()),
                ledger: Some("political_advisor".to_string()),
                idea_token: "ENG_maxwell_aitken".to_string(),
            },
            AdvisorRecord {
                country_tag_db_id: 2,
                char_id: 1456,
                slot: Some("political_advisor".to_string()),
                ledger: Some("political_advisor".to_string()),
                idea_token: "ENG_philip_kerr".to_string(),
            },
        ];
        let before = appointed_advisors_from_active_ideas(&roster, &[]);
        let after_add = appointed_advisors_from_active_ideas(
            &roster,
            &[
                "ENG_leslie_hore_belisha",
                "ENG_maxwell_aitken",
                "ENG_philip_kerr",
            ],
        );
        let after_remove = appointed_advisors_from_active_ideas(&roster, &["ENG_maxwell_aitken"]);
        assert_eq!(before.len(), 0);
        assert_eq!(after_add.len(), 3);
        assert_eq!(after_remove.len(), 1);

        let mut frame = LaneFrame::new(125);
        frame.advisor_roster = roster;
        frame.appointed_advisors = after_add;
        let payload = encode_frame_payload(&frame);
        let summary = decode_frame_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.advisor_roster_rows, 3);
        assert_eq!(summary.appointed_advisor_rows, 3);
        assert_eq!(summary.schema_flags_or & compact::SCHEMA_ROOTS_VALIDATED, 1);

        let record = encode_frame_record(7, 1946051909, &frame);
        assert_eq!(&record[0..4], compact::COMPACT_RECORD_MAGIC);
        assert_eq!(
            u16::from_le_bytes(record[20..22].try_into().unwrap()),
            DOMAIN_ID
        );
    }
}
