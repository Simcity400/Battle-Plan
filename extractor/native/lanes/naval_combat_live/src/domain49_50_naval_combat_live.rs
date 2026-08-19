use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_NAVAL_COMBAT_LIFECYCLE: u16 = 49;
pub const DOMAIN_NAVAL_COMBAT_LIFECYCLE_NAME: &str = "naval_combat_lifecycle_live";
pub const DOMAIN_NAVAL_COMBAT_MEMBERS: u16 = 50;
pub const DOMAIN_NAVAL_COMBAT_MEMBERS_NAME: &str = "naval_combat_members_live";

pub const SCHEMA_VERSION: u16 = 2;
pub const PAYLOAD_HEADER_LEN: usize = 28;
pub const LIFECYCLE_ROW_LEN: usize = 72;
pub const MEMBER_ROW_LEN: usize = 88;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_LOCATION_CANDIDATE_UNVALIDATED: u32 = 1 << 11;
const KNOWN_COMPACT_STATUS_FLAGS: u32 = compact_contract::COMPACT_STATUS_V1_KNOWN_MASK;
pub const FAILURE_STATUS_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;

pub const LIFECYCLE_ROW_DELETED: u32 = 1 << 0;
pub const LIFECYCLE_LOCATION_PRESENT: u32 = 1 << 1;
pub const LIFECYCLE_ATTACKER_COUNT_PRESENT: u32 = 1 << 2;
pub const LIFECYCLE_DEFENDER_COUNT_PRESENT: u32 = 1 << 3;
pub const LIFECYCLE_ATTACKER_POSITIONING_PRESENT: u32 = 1 << 4;
pub const LIFECYCLE_DEFENDER_POSITIONING_PRESENT: u32 = 1 << 5;
pub const LIFECYCLE_REGION_PRESENT: u32 = 1 << 6;
pub const LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT: u32 = 1 << 7;
pub const LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT: u32 = 1 << 8;

pub const MEMBER_ROW_DELETED: u32 = 1 << 0;
pub const MEMBER_SHIP_INSTANCE_PRESENT: u32 = 1 << 1;
pub const MEMBER_OWNER_PRESENT: u32 = 1 << 2;
pub const MEMBER_DESIGNED_MAX_STRENGTH_PRESENT: u32 = 1 << 3;
pub const MEMBER_STATE_BITMASK_PRESENT: u32 = 1 << 4;
pub const MEMBER_ENGAGEMENT_WEIGHT_PRESENT: u32 = 1 << 5;
pub const MEMBER_CSHIP_JOIN_VALIDATED: u32 = 1 << 6;
pub const MEMBER_IS_CONVOY: u32 = 1 << 7;
pub const MEMBER_TASK_FORCE_PRESENT: u32 = 1 << 8;
pub const MEMBER_FLEET_PRESENT: u32 = 1 << 9;
pub const MEMBER_STRENGTH_PRESENT: u32 = 1 << 10;
pub const MEMBER_ORGANISATION_PRESENT: u32 = 1 << 11;
pub const MEMBER_EXPERIENCE_PRESENT: u32 = 1 << 12;
pub const MEMBER_SUNK_CONVOYS_PRESENT: u32 = 1 << 13;
const LIFECYCLE_KNOWN_FLAGS: u32 = LIFECYCLE_ROW_DELETED
    | LIFECYCLE_LOCATION_PRESENT
    | LIFECYCLE_ATTACKER_COUNT_PRESENT
    | LIFECYCLE_DEFENDER_COUNT_PRESENT
    | LIFECYCLE_ATTACKER_POSITIONING_PRESENT
    | LIFECYCLE_DEFENDER_POSITIONING_PRESENT
    | LIFECYCLE_REGION_PRESENT
    | LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT
    | LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT;
const MEMBER_KNOWN_FLAGS: u32 = MEMBER_ROW_DELETED
    | MEMBER_SHIP_INSTANCE_PRESENT
    | MEMBER_OWNER_PRESENT
    | MEMBER_DESIGNED_MAX_STRENGTH_PRESENT
    | MEMBER_STATE_BITMASK_PRESENT
    | MEMBER_ENGAGEMENT_WEIGHT_PRESENT
    | MEMBER_CSHIP_JOIN_VALIDATED
    | MEMBER_IS_CONVOY
    | MEMBER_TASK_FORCE_PRESENT
    | MEMBER_FLEET_PRESENT
    | MEMBER_STRENGTH_PRESENT
    | MEMBER_ORGANISATION_PRESENT
    | MEMBER_EXPERIENCE_PRESENT
    | MEMBER_SUNK_CONVOYS_PRESENT;

pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
pub const MAX_SHIP_INSTANCE_ID: u32 = 4_000_000;
pub const MAX_OWNER_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_GROUP_INDEX: u32 = 64;
pub const MAX_MEMBER_INDEX: u32 = 1_000;
pub const MAX_BATTLE_SLOT_INDEX: u32 = 4_096;
pub const MAX_LOCATION_ID: u32 = (i32::MAX as u32) - 1;
pub const MAX_REGION_ID: u32 = 100_000;
pub const MAX_DESIGNED_STRENGTH_RAW: u32 = 1_000_000_000;
pub const MAX_FIXED_POINT_RAW: u32 = 1_000_000_000;
pub const MAX_POSITIONING_RAW: i32 = 100_000;
pub const MAX_SUNK_CONVOYS: u32 = 1_000_000;
pub const MAX_OOB_ID: u32 = 4_000_000;
pub const CSHIP_TYPE_TAG: u32 = 51;

pub const FEX_STATE_UNINITIALIZED: u32 = 0;
pub const FEX_STATE_WAITING: u32 = 1;
pub const FEX_STATE_FIGHTING: u32 = 2;
pub const FEX_STATE_ON_THE_WAY: u32 = 4;
pub const FEX_STATE_ESCAPING: u32 = 8;
pub const FEX_STATE_RUN_AWAY: u32 = 0x10;
pub const FEX_STATE_SUNK: u32 = 0x20;

pub fn fex_state_name(raw: u32) -> Option<&'static str> {
    match raw {
        FEX_STATE_UNINITIALIZED => Some("FEX_STATE_UNINITIALIZED"),
        FEX_STATE_WAITING => Some("FEX_STATE_WAITING"),
        FEX_STATE_FIGHTING => Some("FEX_STATE_FIGHTING"),
        FEX_STATE_ON_THE_WAY => Some("FEX_STATE_ON_THE_WAY"),
        FEX_STATE_ESCAPING => Some("FEX_STATE_ESCAPING"),
        FEX_STATE_RUN_AWAY => Some("FEX_STATE_RUN_AWAY"),
        FEX_STATE_SUNK => Some("FEX_STATE_SUNK"),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CombatSide {
    Attacker,
    Defender,
}

impl CombatSide {
    pub fn wire_id(self) -> u32 {
        match self {
            Self::Attacker => 1,
            Self::Defender => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Attacker => "attacker",
            Self::Defender => "defender",
        }
    }

    fn from_wire_id(value: u32) -> Option<Self> {
        match value {
            1 => Some(Self::Attacker),
            2 => Some(Self::Defender),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LifecycleRow {
    pub combat_id: u32,
    pub generation: u64,
    pub body_ptr: u64,
    pub deleted: bool,
    pub location_id: Option<u32>,
    pub region_id: Option<u32>,
    pub attacker_member_count: Option<u32>,
    pub defender_member_count: Option<u32>,
    pub attacker_positioning_raw: Option<i32>,
    pub defender_positioning_raw: Option<i32>,
    pub attacker_anti_air_raw: Option<i64>,
    pub defender_anti_air_raw: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub combat_id: u32,
    pub generation: u64,
    pub deleted: bool,
    pub side: CombatSide,
    pub group_index: u32,
    pub member_index: u32,
    pub unit_type_tag: u32,
    pub ship_instance_id: Option<u32>,
    pub cship_join_validated: bool,
    pub is_convoy: bool,
    pub owner_tag_db_id: Option<u32>,
    pub designed_max_strength_raw: Option<u32>,
    pub battle_slot_index_raw: u32,
    pub state_bitmask: Option<u32>,
    pub engagement_weight_raw: Option<i32>,
    pub task_force_id: Option<u32>,
    pub fleet_id: Option<u32>,
    pub strength_raw: Option<u32>,
    pub organisation_raw: Option<u32>,
    pub experience_raw: Option<u32>,
    pub sunk_convoys: Option<u32>,
}

#[derive(Debug)]
struct CompactRecord<'a> {
    offset: usize,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &'a [u8],
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainSummary {
    pub data_record_count: u64,
    pub row_count_total: u64,
    pub status_flags_or: u32,
    pub data_units_total: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateSummary {
    pub record_count: u64,
    pub domains_seen: BTreeSet<u16>,
    pub lifecycle_row_count: u64,
    pub member_row_count: u64,
    pub active_combat_count: u64,
    pub two_sided_combat_count: u64,
    pub lifecycle_location_row_count: u64,
    pub lifecycle_region_row_count: u64,
    pub lifecycle_started_row_count: u64,
    pub lifecycle_updated_row_count: u64,
    pub lifecycle_ended_row_count: u64,
    pub member_deleted_row_count: u64,
    pub cship_validated_member_count: u64,
    pub task_force_linked_member_count: u64,
    pub fleet_linked_member_count: u64,
    pub hull_telemetry_member_count: u64,
    pub sunk_convoy_telemetry_member_count: u64,
    pub convoy_member_count: u64,
    pub status_flags_or: u32,
    pub per_domain: BTreeMap<u16, DomainSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GateResult {
    pub ok: bool,
    pub errors: Vec<String>,
    pub summary: GateSummary,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PayloadDecodeSummary {
    pub domain_id: u16,
    pub status_flags_or: u32,
    pub row_count: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

#[derive(Debug)]
struct PayloadHeader {
    status_flags: u32,
    row_count: usize,
    string_count: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PerfAuthority {
    observed_rows: u32,
    changed_rows: u32,
    unit_count: u32,
    changed_unit_count: u32,
    status_flags: u32,
}

#[derive(Debug)]
struct DecodedLifecycleRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    location_id: u32,
    region_id: u32,
    attacker_member_count: u32,
    defender_member_count: u32,
    attacker_positioning_raw: i32,
    defender_positioning_raw: i32,
    attacker_anti_air_raw: i64,
    defender_anti_air_raw: i64,
    reserved_tail: u64,
}

#[derive(Debug)]
struct DecodedMemberRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    side: u32,
    group_index: u32,
    member_index: u32,
    unit_type_tag: u32,
    ship_instance_id: u32,
    owner_tag_db_id: u32,
    designed_max_strength_raw: u32,
    battle_slot_index_raw: u32,
    state_bitmask: u32,
    engagement_weight_raw: i32,
    reserved: u32,
    task_force_id: u32,
    fleet_id: u32,
    strength_raw: u32,
    organisation_raw: u32,
    sunk_convoys: u32,
    experience_raw: u32,
    reserved_tail: u32,
}

pub fn encode_lifecycle_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[LifecycleRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * LIFECYCLE_ROW_LEN + 2);
    append_payload_header(
        &mut payload,
        LIFECYCLE_ROW_LEN,
        elapsed_wall_ms,
        status_flags,
        rows.len(),
        1,
    );
    for row in rows {
        validate_lifecycle_row(row).expect("invalid lifecycle row");
        let mut flags = 0u32;
        if row.deleted {
            flags |= LIFECYCLE_ROW_DELETED;
        }
        if row.location_id.is_some() {
            flags |= LIFECYCLE_LOCATION_PRESENT;
        }
        if row.region_id.is_some() {
            flags |= LIFECYCLE_REGION_PRESENT;
        }
        if row.attacker_member_count.is_some() {
            flags |= LIFECYCLE_ATTACKER_COUNT_PRESENT;
        }
        if row.defender_member_count.is_some() {
            flags |= LIFECYCLE_DEFENDER_COUNT_PRESENT;
        }
        if row.attacker_positioning_raw.is_some() {
            flags |= LIFECYCLE_ATTACKER_POSITIONING_PRESENT;
        }
        if row.defender_positioning_raw.is_some() {
            flags |= LIFECYCLE_DEFENDER_POSITIONING_PRESENT;
        }
        if row.attacker_anti_air_raw.is_some() {
            flags |= LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT;
        }
        if row.defender_anti_air_raw.is_some() {
            flags |= LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT;
        }
        push_u32(&mut payload, row.combat_id);
        push_u64(&mut payload, row.generation);
        push_u64(&mut payload, row.body_ptr);
        push_u32(&mut payload, flags);
        push_u32(&mut payload, row.location_id.unwrap_or(0));
        push_u32(&mut payload, row.region_id.unwrap_or(0));
        push_u32(&mut payload, row.attacker_member_count.unwrap_or(0));
        push_u32(&mut payload, row.defender_member_count.unwrap_or(0));
        push_i32(&mut payload, row.attacker_positioning_raw.unwrap_or(0));
        push_i32(&mut payload, row.defender_positioning_raw.unwrap_or(0));
        push_i64(&mut payload, row.attacker_anti_air_raw.unwrap_or(0));
        push_i64(&mut payload, row.defender_anti_air_raw.unwrap_or(0));
        push_u64(&mut payload, 0);
    }
    append_empty_string_table(&mut payload);
    payload
}

pub fn encode_member_payload(
    elapsed_wall_ms: u64,
    status_flags: u32,
    rows: &[MemberRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(PAYLOAD_HEADER_LEN + rows.len() * MEMBER_ROW_LEN + 2);
    append_payload_header(
        &mut payload,
        MEMBER_ROW_LEN,
        elapsed_wall_ms,
        status_flags,
        rows.len(),
        1,
    );
    for row in rows {
        validate_member_row(row).expect("invalid member row");
        let mut flags = 0u32;
        if row.deleted {
            flags |= MEMBER_ROW_DELETED;
        }
        if row.ship_instance_id.is_some() {
            flags |= MEMBER_SHIP_INSTANCE_PRESENT;
        }
        if row.cship_join_validated {
            flags |= MEMBER_CSHIP_JOIN_VALIDATED;
        }
        if row.is_convoy {
            flags |= MEMBER_IS_CONVOY;
        }
        if row.owner_tag_db_id.is_some() {
            flags |= MEMBER_OWNER_PRESENT;
        }
        if row.designed_max_strength_raw.is_some() {
            flags |= MEMBER_DESIGNED_MAX_STRENGTH_PRESENT;
        }
        if row.state_bitmask.is_some() {
            flags |= MEMBER_STATE_BITMASK_PRESENT;
        }
        if row.engagement_weight_raw.is_some() {
            flags |= MEMBER_ENGAGEMENT_WEIGHT_PRESENT;
        }
        if row.task_force_id.is_some() {
            flags |= MEMBER_TASK_FORCE_PRESENT;
        }
        if row.fleet_id.is_some() {
            flags |= MEMBER_FLEET_PRESENT;
        }
        if row.strength_raw.is_some() {
            flags |= MEMBER_STRENGTH_PRESENT;
        }
        if row.organisation_raw.is_some() {
            flags |= MEMBER_ORGANISATION_PRESENT;
        }
        if row.experience_raw.is_some() {
            flags |= MEMBER_EXPERIENCE_PRESENT;
        }
        if row.sunk_convoys.is_some() {
            flags |= MEMBER_SUNK_CONVOYS_PRESENT;
        }
        push_u32(&mut payload, row.combat_id);
        push_u64(&mut payload, row.generation);
        push_u32(&mut payload, flags);
        push_u32(&mut payload, row.side.wire_id());
        push_u32(&mut payload, row.group_index);
        push_u32(&mut payload, row.member_index);
        push_u32(&mut payload, row.unit_type_tag);
        push_u32(&mut payload, row.ship_instance_id.unwrap_or(0));
        push_u32(&mut payload, row.owner_tag_db_id.unwrap_or(0));
        push_u32(&mut payload, row.designed_max_strength_raw.unwrap_or(0));
        push_u32(&mut payload, row.battle_slot_index_raw);
        push_u32(&mut payload, row.state_bitmask.unwrap_or(0));
        push_i32(&mut payload, row.engagement_weight_raw.unwrap_or(0));
        push_u32(&mut payload, 0);
        push_u32(&mut payload, row.task_force_id.unwrap_or(0));
        push_u32(&mut payload, row.fleet_id.unwrap_or(0));
        push_u32(&mut payload, row.strength_raw.unwrap_or(0));
        push_u32(&mut payload, row.organisation_raw.unwrap_or(0));
        push_u32(&mut payload, row.sunk_convoys.unwrap_or(0));
        push_u32(&mut payload, row.experience_raw.unwrap_or(0));
        push_u32(&mut payload, 0);
    }
    append_empty_string_table(&mut payload);
    payload
}

pub fn compact_record(
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN + payload.len());
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_stream(data: &[u8]) -> Result<GateSummary, Vec<String>> {
    let result = gate_stream(data);
    if result.ok {
        Ok(result.summary)
    } else {
        Err(result.errors)
    }
}

pub fn decode_lifecycle_rows(payload: &[u8]) -> Result<(u32, Vec<LifecycleRow>), Vec<String>> {
    let mut errors = Vec::new();
    let Some((header, rows)) = decode_lifecycle_payload(0, payload, &mut errors) else {
        return Err(errors);
    };
    for (row_index, row) in rows.iter().enumerate() {
        validate_decoded_lifecycle_row(0, row_index, row, &mut errors);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let decoded = rows
        .into_iter()
        .map(|row| LifecycleRow {
            combat_id: row.combat_id,
            generation: row.generation,
            body_ptr: 0,
            deleted: row.flags & LIFECYCLE_ROW_DELETED != 0,
            location_id: (row.flags & LIFECYCLE_LOCATION_PRESENT != 0).then_some(row.location_id),
            region_id: (row.flags & LIFECYCLE_REGION_PRESENT != 0).then_some(row.region_id),
            attacker_member_count: (row.flags & LIFECYCLE_ATTACKER_COUNT_PRESENT != 0)
                .then_some(row.attacker_member_count),
            defender_member_count: (row.flags & LIFECYCLE_DEFENDER_COUNT_PRESENT != 0)
                .then_some(row.defender_member_count),
            attacker_positioning_raw: (row.flags & LIFECYCLE_ATTACKER_POSITIONING_PRESENT != 0)
                .then_some(row.attacker_positioning_raw),
            defender_positioning_raw: (row.flags & LIFECYCLE_DEFENDER_POSITIONING_PRESENT != 0)
                .then_some(row.defender_positioning_raw),
            attacker_anti_air_raw: (row.flags & LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT != 0)
                .then_some(row.attacker_anti_air_raw),
            defender_anti_air_raw: (row.flags & LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT != 0)
                .then_some(row.defender_anti_air_raw),
        })
        .collect();
    Ok((header.status_flags, decoded))
}

pub fn decode_member_rows(payload: &[u8]) -> Result<(u32, Vec<MemberRow>), Vec<String>> {
    let mut errors = Vec::new();
    let Some((header, rows)) = decode_member_payload(0, payload, &mut errors) else {
        return Err(errors);
    };
    for (row_index, row) in rows.iter().enumerate() {
        validate_decoded_member_row(0, row_index, row, &mut errors);
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut decoded = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(side) = CombatSide::from_wire_id(row.side) else {
            return Err(vec![format!("member row has invalid side {}", row.side)]);
        };
        decoded.push(MemberRow {
            combat_id: row.combat_id,
            generation: row.generation,
            deleted: row.flags & MEMBER_ROW_DELETED != 0,
            side,
            group_index: row.group_index,
            member_index: row.member_index,
            unit_type_tag: row.unit_type_tag,
            ship_instance_id: (row.flags & MEMBER_SHIP_INSTANCE_PRESENT != 0)
                .then_some(row.ship_instance_id),
            cship_join_validated: row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0,
            is_convoy: row.flags & MEMBER_IS_CONVOY != 0,
            owner_tag_db_id: (row.flags & MEMBER_OWNER_PRESENT != 0).then_some(row.owner_tag_db_id),
            designed_max_strength_raw: (row.flags & MEMBER_DESIGNED_MAX_STRENGTH_PRESENT != 0)
                .then_some(row.designed_max_strength_raw),
            battle_slot_index_raw: row.battle_slot_index_raw,
            state_bitmask: (row.flags & MEMBER_STATE_BITMASK_PRESENT != 0)
                .then_some(row.state_bitmask),
            engagement_weight_raw: (row.flags & MEMBER_ENGAGEMENT_WEIGHT_PRESENT != 0)
                .then_some(row.engagement_weight_raw),
            task_force_id: (row.flags & MEMBER_TASK_FORCE_PRESENT != 0)
                .then_some(row.task_force_id),
            fleet_id: (row.flags & MEMBER_FLEET_PRESENT != 0).then_some(row.fleet_id),
            strength_raw: (row.flags & MEMBER_STRENGTH_PRESENT != 0).then_some(row.strength_raw),
            organisation_raw: (row.flags & MEMBER_ORGANISATION_PRESENT != 0)
                .then_some(row.organisation_raw),
            experience_raw: (row.flags & MEMBER_EXPERIENCE_PRESENT != 0)
                .then_some(row.experience_raw),
            sunk_convoys: (row.flags & MEMBER_SUNK_CONVOYS_PRESENT != 0)
                .then_some(row.sunk_convoys),
        });
    }
    Ok((header.status_flags, decoded))
}

pub fn decode_lifecycle_data_payload(offset: usize, payload: &[u8]) -> PayloadDecodeSummary {
    let mut errors = Vec::new();
    let mut summary = PayloadDecodeSummary {
        domain_id: DOMAIN_NAVAL_COMBAT_LIFECYCLE,
        ..PayloadDecodeSummary::default()
    };
    let Some((header, rows)) = decode_lifecycle_payload(offset, payload, &mut errors) else {
        summary.parse_errors = errors;
        return summary;
    };

    summary.status_flags_or |= header.status_flags;
    summary.row_count = rows.len() as u64;
    add_payload_unit(&mut summary, "naval_combat_lifecycle_rows", rows.len());
    let mut location_rows = 0usize;
    let mut region_rows = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        validate_decoded_lifecycle_row(offset, row_index, row, &mut errors);
        if row.flags & LIFECYCLE_LOCATION_PRESENT != 0 {
            location_rows = location_rows.saturating_add(1);
        }
        if row.flags & LIFECYCLE_REGION_PRESENT != 0 {
            region_rows = region_rows.saturating_add(1);
        }
    }
    add_payload_unit(
        &mut summary,
        "naval_combat_lifecycle_location_rows",
        location_rows,
    );
    add_payload_unit(
        &mut summary,
        "naval_combat_lifecycle_region_rows",
        region_rows,
    );
    summary.parse_errors = errors;
    summary
}

pub fn decode_members_data_payload(offset: usize, payload: &[u8]) -> PayloadDecodeSummary {
    let mut errors = Vec::new();
    let mut summary = PayloadDecodeSummary {
        domain_id: DOMAIN_NAVAL_COMBAT_MEMBERS,
        ..PayloadDecodeSummary::default()
    };
    let Some((header, rows)) = decode_member_payload(offset, payload, &mut errors) else {
        summary.parse_errors = errors;
        return summary;
    };

    summary.status_flags_or |= header.status_flags;
    summary.row_count = rows.len() as u64;
    add_payload_unit(&mut summary, "naval_combat_member_rows", rows.len());
    let mut cship_validated_rows = 0usize;
    let mut convoy_rows = 0usize;
    let mut deleted_rows = 0usize;
    let mut task_force_linked_rows = 0usize;
    let mut fleet_linked_rows = 0usize;
    let mut hull_telemetry_rows = 0usize;
    let mut sunk_convoy_telemetry_rows = 0usize;
    for (row_index, row) in rows.iter().enumerate() {
        validate_decoded_member_row(offset, row_index, row, &mut errors);
        if row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 {
            cship_validated_rows = cship_validated_rows.saturating_add(1);
        }
        if row.flags & MEMBER_ROW_DELETED != 0 {
            deleted_rows = deleted_rows.saturating_add(1);
        }
        if row.flags & MEMBER_IS_CONVOY != 0 {
            convoy_rows = convoy_rows.saturating_add(1);
        }
        if row.flags & MEMBER_TASK_FORCE_PRESENT != 0 {
            task_force_linked_rows = task_force_linked_rows.saturating_add(1);
        }
        if row.flags & MEMBER_FLEET_PRESENT != 0 {
            fleet_linked_rows = fleet_linked_rows.saturating_add(1);
        }
        if row.flags & (MEMBER_STRENGTH_PRESENT | MEMBER_ORGANISATION_PRESENT) != 0 {
            hull_telemetry_rows = hull_telemetry_rows.saturating_add(1);
        }
        if row.flags & MEMBER_SUNK_CONVOYS_PRESENT != 0 {
            sunk_convoy_telemetry_rows = sunk_convoy_telemetry_rows.saturating_add(1);
        }
    }
    add_payload_unit(
        &mut summary,
        "naval_combat_cship_validated_member_rows",
        cship_validated_rows,
    );
    add_payload_unit(&mut summary, "naval_combat_convoy_member_rows", convoy_rows);
    add_payload_unit(
        &mut summary,
        "naval_combat_member_deleted_rows",
        deleted_rows,
    );
    add_payload_unit(
        &mut summary,
        "naval_combat_task_force_linked_member_rows",
        task_force_linked_rows,
    );
    add_payload_unit(
        &mut summary,
        "naval_combat_fleet_linked_member_rows",
        fleet_linked_rows,
    );
    add_payload_unit(
        &mut summary,
        "naval_combat_hull_telemetry_member_rows",
        hull_telemetry_rows,
    );
    add_payload_unit(
        &mut summary,
        "naval_combat_sunk_convoy_telemetry_member_rows",
        sunk_convoy_telemetry_rows,
    );
    summary.parse_errors = errors;
    summary
}

pub fn gate_stream(data: &[u8]) -> GateResult {
    let mut errors = Vec::new();
    if let Err(error) =
        compact_contract::stream_validator::validate_freeze_census_stream_placement(data)
    {
        errors.push(format!("stateful stream placement: {error}"));
    }
    let records = match decode_records(data) {
        Ok(records) => records,
        Err(err) => {
            return GateResult {
                ok: false,
                errors: vec![err],
                summary: GateSummary::default(),
            };
        }
    };

    let mut summary = GateSummary {
        record_count: records.len() as u64,
        ..GateSummary::default()
    };
    let mut last_sequence: Option<u64> = None;
    let mut pending_perf_hour: Option<i32> = None;
    let mut open_frame_hour: Option<i32> = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut final_record_is_clean_frame_end = false;
    let mut release_epilogue_seen = false;
    let mut frame_lifecycle_data_records = 0usize;
    let mut frame_member_data_records = 0usize;
    let mut frame_lifecycle_perf_records = 0usize;
    let mut frame_member_perf_records = 0usize;
    let mut frame_emitted_perf_records = 0u32;
    let mut frame_lifecycle_perf = None;
    let mut frame_member_perf = None;
    let mut frame_lifecycle_data_status = None;
    let mut frame_member_data_status = None;
    let mut frame_lifecycle_data_rows = None;
    let mut frame_member_data_rows = None;
    let mut frame_lifecycle_parents = BTreeMap::new();
    let mut frame_member_keys = BTreeSet::new();
    let mut frame_live_ship_ids = BTreeSet::new();
    let mut lifecycle_keys = BTreeSet::new();
    let mut seen_lifecycle_keys = BTreeSet::new();
    let mut retired_lifecycle_keys = BTreeSet::new();
    let mut active_members: BTreeMap<(u32, u64, u32, u32), CombatSide> = BTreeMap::new();
    let mut retired_member_keys = BTreeSet::new();
    let mut non_convoy_member_count = 0u64;
    let mut validated_non_convoy_member_count = 0u64;
    let mut terminal_sunk_unjoined_member_count = 0u64;

    for record in records {
        match last_sequence {
            None if record.sequence != 1 => errors.push(format!(
                "offset {}: sequence {} does not start at 1 (stream prefix is missing)",
                record.offset, record.sequence
            )),
            Some(previous) => match previous.checked_add(1) {
                Some(expected) if record.sequence != expected => errors.push(format!(
                    "offset {}: sequence discontinuity {previous} -> {} (expected {expected})",
                    record.offset, record.sequence
                )),
                None => errors.push(format!(
                    "offset {}: sequence space exhausted after {previous}; got {} (wrapping is not allowed)",
                    record.offset, record.sequence
                )),
                _ => {}
            },
            _ => {}
        }
        last_sequence = Some(record.sequence);
        let previous_record_closed_world = final_record_is_clean_frame_end;
        final_record_is_clean_frame_end = false;

        let known_record_type =
            match compact_contract::KnownRecordType::try_from(record.record_type) {
                Ok(record_type) => record_type,
                Err(unknown) => {
                    errors.push(format!(
                        "offset {}: unknown compact record type {}",
                        record.offset, unknown.0
                    ));
                    continue;
                }
            };
        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf => {
                match open_frame_hour {
                    Some(open) if open != record.game_hour_raw => errors.push(format!(
                        "offset {}: DOMAIN_PERF hour {} does not match open DATA frame hour {open}",
                        record.offset, record.game_hour_raw
                    )),
                    Some(_) => {}
                    None => {
                        validate_next_frame_hour(
                            last_closed_hour,
                            record.game_hour_raw,
                            "DOMAIN_PERF",
                            record.offset,
                            &mut errors,
                        );
                        match pending_perf_hour {
                            Some(pending) if pending != record.game_hour_raw => errors.push(format!(
                                "offset {}: DOMAIN_PERF hour {} conflicts with pending frame hour {pending}",
                                record.offset, record.game_hour_raw
                            )),
                            Some(_) => {}
                            None => pending_perf_hour = Some(record.game_hour_raw),
                        }
                    }
                }

                if record.domain_id != 250 {
                    frame_emitted_perf_records = frame_emitted_perf_records.saturating_add(1);
                }
                match record.domain_id {
                    DOMAIN_NAVAL_COMBAT_LIFECYCLE => {
                        frame_lifecycle_perf_records =
                            frame_lifecycle_perf_records.saturating_add(1);
                        if frame_lifecycle_perf_records > 1 {
                            errors.push(format!(
                                "offset {}: frame hour {} has duplicate domain 49 PERF records",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        let authority = decode_perf_authority(record.payload);
                        validate_status_health(
                            record.domain_id,
                            authority.status_flags,
                            "DOMAIN_PERF",
                            record.offset,
                            &mut errors,
                        );
                        frame_lifecycle_perf.get_or_insert(authority);
                    }
                    DOMAIN_NAVAL_COMBAT_MEMBERS => {
                        frame_member_perf_records = frame_member_perf_records.saturating_add(1);
                        if frame_member_perf_records > 1 {
                            errors.push(format!(
                                "offset {}: frame hour {} has duplicate domain 50 PERF records",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        if frame_lifecycle_perf_records != 1 {
                            errors.push(format!(
                                "offset {}: domain 50 PERF precedes its domain 49 PERF in frame hour {}",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        let authority = decode_perf_authority(record.payload);
                        validate_status_health(
                            record.domain_id,
                            authority.status_flags,
                            "DOMAIN_PERF",
                            record.offset,
                            &mut errors,
                        );
                        frame_member_perf.get_or_insert(authority);
                    }
                    _ => {}
                }
            }
            compact_contract::KnownRecordType::DomainData => {
                match open_frame_hour {
                    Some(open) if open != record.game_hour_raw => errors.push(format!(
                        "offset {}: DOMAIN_DATA hour {} encountered before FRAME_END closed open frame hour {open}",
                        record.offset, record.game_hour_raw
                    )),
                    Some(_) => {}
                    None => {
                        validate_next_frame_hour(
                            last_closed_hour,
                            record.game_hour_raw,
                            "DOMAIN_DATA",
                            record.offset,
                            &mut errors,
                        );
                        if let Some(pending) = pending_perf_hour {
                            if pending != record.game_hour_raw {
                                errors.push(format!(
                                    "offset {}: DOMAIN_DATA hour {} conflicts with pending PERF hour {pending}",
                                    record.offset, record.game_hour_raw
                                ));
                            }
                        }
                        open_frame_hour = Some(record.game_hour_raw);
                    }
                }

                summary.domains_seen.insert(record.domain_id);
                match record.domain_id {
                    DOMAIN_NAVAL_COMBAT_LIFECYCLE => {
                        frame_lifecycle_data_records =
                            frame_lifecycle_data_records.saturating_add(1);
                        if frame_lifecycle_data_records > 1 {
                            errors.push(format!(
                                "offset {}: frame hour {} has duplicate domain 49 DATA records",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        let (header, rows) = match decode_lifecycle_payload(
                            record.offset,
                            record.payload,
                            &mut errors,
                        ) {
                            Some(decoded) => decoded,
                            None => continue,
                        };
                        record_domain_data(&mut summary, record.domain_id, &header, rows.len());
                        validate_status_health(
                            record.domain_id,
                            header.status_flags,
                            "DOMAIN_DATA",
                            record.offset,
                            &mut errors,
                        );
                        frame_lifecycle_data_status.get_or_insert(header.status_flags);
                        frame_lifecycle_data_rows.get_or_insert(rows.len() as u32);
                        if header.status_flags & STATUS_LOCATION_CANDIDATE_UNVALIDATED != 0 {
                            errors.push(format!(
                                "offset {}: domain 49 location/region promotion is incomplete (status bit 11 set)",
                                record.offset
                            ));
                        }
                        summary.lifecycle_row_count = summary
                            .lifecycle_row_count
                            .saturating_add(rows.len() as u64);
                        for (row_index, row) in rows.into_iter().enumerate() {
                            validate_decoded_lifecycle_row(
                                record.offset,
                                row_index,
                                &row,
                                &mut errors,
                            );
                            let key = (row.combat_id, row.generation);
                            let parent_deleted = row.flags & LIFECYCLE_ROW_DELETED != 0;
                            if frame_lifecycle_parents
                                .insert(key, parent_deleted)
                                .is_some()
                            {
                                errors.push(format!(
                                    "offset {}: duplicate/ambiguous domain 49 parent row for combat {}:{} in frame hour {}",
                                    record.offset, key.0, key.1, record.game_hour_raw
                                ));
                            }
                            if !parent_deleted
                                && row.flags
                                    & (LIFECYCLE_LOCATION_PRESENT | LIFECYCLE_REGION_PRESENT)
                                    != (LIFECYCLE_LOCATION_PRESENT | LIFECYCLE_REGION_PRESENT)
                            {
                                errors.push(format!(
                                    "offset {}: lifecycle row {row_index} combat {}:{} lacks promoted location/region",
                                    record.offset, row.combat_id, row.generation
                                ));
                            }
                            let exact_side_scalar_flags = LIFECYCLE_ATTACKER_POSITIONING_PRESENT
                                | LIFECYCLE_DEFENDER_POSITIONING_PRESENT
                                | LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT
                                | LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT;
                            if !parent_deleted
                                && row.flags & exact_side_scalar_flags != exact_side_scalar_flags
                            {
                                errors.push(format!(
                                    "offset {}: lifecycle row {row_index} combat {}:{} lacks a complete exact +0x138/+0x150 side scalar set",
                                    record.offset, row.combat_id, row.generation
                                ));
                            }
                            if row.flags & LIFECYCLE_LOCATION_PRESENT != 0 {
                                summary.lifecycle_location_row_count =
                                    summary.lifecycle_location_row_count.saturating_add(1);
                            }
                            if row.flags & LIFECYCLE_REGION_PRESENT != 0 {
                                summary.lifecycle_region_row_count =
                                    summary.lifecycle_region_row_count.saturating_add(1);
                            }
                            if row.flags & LIFECYCLE_ROW_DELETED == 0 {
                                if retired_lifecycle_keys.contains(&key) {
                                    errors.push(format!(
                                        "offset {}: lifecycle row for retired combat {}:{} attempts same-generation resurrection",
                                        record.offset, key.0, key.1
                                    ));
                                } else {
                                    lifecycle_keys.insert(key);
                                }
                                if seen_lifecycle_keys.insert(key) {
                                    summary.lifecycle_started_row_count =
                                        summary.lifecycle_started_row_count.saturating_add(1);
                                } else {
                                    summary.lifecycle_updated_row_count =
                                        summary.lifecycle_updated_row_count.saturating_add(1);
                                }
                            } else {
                                summary.lifecycle_ended_row_count =
                                    summary.lifecycle_ended_row_count.saturating_add(1);
                                if !lifecycle_keys.remove(&key) {
                                    errors.push(format!(
                                        "offset {}: lifecycle tombstone for combat {}:{} is duplicate or was not preceded by an active row",
                                        record.offset, key.0, key.1
                                    ));
                                }
                                retired_lifecycle_keys.insert(key);
                            }
                        }
                    }
                    DOMAIN_NAVAL_COMBAT_MEMBERS => {
                        if frame_lifecycle_data_records != 1 {
                            errors.push(format!(
                                "offset {}: domain 50 DATA precedes its domain 49 DATA in frame hour {}",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        frame_member_data_records = frame_member_data_records.saturating_add(1);
                        if frame_member_data_records > 1 {
                            errors.push(format!(
                                "offset {}: frame hour {} has duplicate domain 50 DATA records",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        let (header, rows) =
                            match decode_member_payload(record.offset, record.payload, &mut errors)
                            {
                                Some(decoded) => decoded,
                                None => continue,
                            };
                        record_domain_data(&mut summary, record.domain_id, &header, rows.len());
                        validate_status_health(
                            record.domain_id,
                            header.status_flags,
                            "DOMAIN_DATA",
                            record.offset,
                            &mut errors,
                        );
                        frame_member_data_status.get_or_insert(header.status_flags);
                        frame_member_data_rows.get_or_insert(rows.len() as u32);
                        summary.member_row_count =
                            summary.member_row_count.saturating_add(rows.len() as u64);
                        for (row_index, row) in rows.into_iter().enumerate() {
                            validate_decoded_member_row(
                                record.offset,
                                row_index,
                                &row,
                                &mut errors,
                            );
                            let parent = (row.combat_id, row.generation);
                            let is_deleted = row.flags & MEMBER_ROW_DELETED != 0;
                            let parent_is_active = lifecycle_keys.contains(&parent);
                            let parent_delta = frame_lifecycle_parents.get(&parent).copied();
                            if !is_deleted && !parent_is_active {
                                errors.push(format!(
                                    "offset {}: domain 50 member row {row_index} combat {}:{} is orphaned from active domain 49 lifecycle state",
                                    record.offset, parent.0, parent.1
                                ));
                            } else if is_deleted && !parent_is_active && parent_delta != Some(true)
                            {
                                errors.push(format!(
                                    "offset {}: deleted domain 50 member row {row_index} combat {}:{} is orphaned from domain 49 lifecycle history",
                                    record.offset, parent.0, parent.1
                                ));
                            }
                            let membership_key = (
                                row.combat_id,
                                row.generation,
                                row.side,
                                row.battle_slot_index_raw,
                            );
                            if !frame_member_keys.insert(membership_key) {
                                errors.push(format!(
                                    "offset {}: duplicate/ambiguous domain 50 membership for combat {}:{} side {} battle slot {}",
                                    record.offset,
                                    row.combat_id,
                                    row.generation,
                                    row.side,
                                    row.battle_slot_index_raw
                                ));
                            }
                            let is_convoy = row.flags & MEMBER_IS_CONVOY != 0;
                            if is_convoy {
                                summary.convoy_member_count =
                                    summary.convoy_member_count.saturating_add(1);
                            } else {
                                non_convoy_member_count = non_convoy_member_count.saturating_add(1);
                                let terminal_sunk = row.flags & MEMBER_STATE_BITMASK_PRESENT != 0
                                    && row.state_bitmask == FEX_STATE_SUNK;
                                if row.flags & MEMBER_SHIP_INSTANCE_PRESENT == 0 {
                                    errors.push(format!(
                                        "offset {}: non-convoy member row {row_index} combat {}:{} lacks ship-instance identity",
                                        record.offset, row.combat_id, row.generation
                                    ));
                                }
                                if row.flags & MEMBER_CSHIP_JOIN_VALIDATED == 0 && !terminal_sunk {
                                    errors.push(format!(
                                        "offset {}: non-terminal non-convoy member row {row_index} combat {}:{} lacks a validated CShip join",
                                        record.offset, row.combat_id, row.generation
                                    ));
                                }
                                if row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 {
                                    validated_non_convoy_member_count =
                                        validated_non_convoy_member_count.saturating_add(1);
                                } else if terminal_sunk {
                                    terminal_sunk_unjoined_member_count =
                                        terminal_sunk_unjoined_member_count.saturating_add(1);
                                    let cship_derived = MEMBER_TASK_FORCE_PRESENT
                                        | MEMBER_FLEET_PRESENT
                                        | MEMBER_STRENGTH_PRESENT
                                        | MEMBER_ORGANISATION_PRESENT
                                        | MEMBER_EXPERIENCE_PRESENT
                                        | MEMBER_SUNK_CONVOYS_PRESENT;
                                    if row.flags & cship_derived != 0 {
                                        errors.push(format!(
                                            "offset {}: terminal sunk member row {row_index} lacks a live CShip join but carries CShip-derived fields 0x{:08x}",
                                            record.offset,
                                            row.flags & cship_derived
                                        ));
                                    }
                                }
                                if !is_deleted
                                    && row.flags & MEMBER_SHIP_INSTANCE_PRESENT != 0
                                    && !frame_live_ship_ids.insert(row.ship_instance_id)
                                {
                                    errors.push(format!(
                                        "offset {}: duplicate/ambiguous live ship_instance_id {} in frame hour {}",
                                        record.offset, row.ship_instance_id, record.game_hour_raw
                                    ));
                                }
                            }
                            if row.flags & MEMBER_ROW_DELETED != 0 {
                                summary.member_deleted_row_count =
                                    summary.member_deleted_row_count.saturating_add(1);
                                if active_members.remove(&membership_key).is_none() {
                                    errors.push(format!(
                                        "offset {}: deleted domain 50 membership combat {}:{} side {} battle slot {} was not previously active",
                                        record.offset,
                                        row.combat_id,
                                        row.generation,
                                        row.side,
                                        row.battle_slot_index_raw
                                    ));
                                }
                                retired_member_keys.insert(membership_key);
                            } else if let Some(side) = CombatSide::from_wire_id(row.side) {
                                if retired_member_keys.contains(&membership_key) {
                                    errors.push(format!(
                                        "offset {}: domain 50 membership combat {}:{} side {} battle slot {} attempts same-generation resurrection",
                                        record.offset,
                                        row.combat_id,
                                        row.generation,
                                        row.side,
                                        row.battle_slot_index_raw
                                    ));
                                } else {
                                    active_members.insert(membership_key, side);
                                }
                            }
                            if row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 {
                                summary.cship_validated_member_count =
                                    summary.cship_validated_member_count.saturating_add(1);
                            }
                            if row.flags & MEMBER_TASK_FORCE_PRESENT != 0 {
                                summary.task_force_linked_member_count =
                                    summary.task_force_linked_member_count.saturating_add(1);
                            }
                            if row.flags & MEMBER_FLEET_PRESENT != 0 {
                                summary.fleet_linked_member_count =
                                    summary.fleet_linked_member_count.saturating_add(1);
                            }
                            if row.flags & (MEMBER_STRENGTH_PRESENT | MEMBER_ORGANISATION_PRESENT)
                                != 0
                            {
                                summary.hull_telemetry_member_count =
                                    summary.hull_telemetry_member_count.saturating_add(1);
                            }
                            if row.flags & MEMBER_SUNK_CONVOYS_PRESENT != 0 {
                                summary.sunk_convoy_telemetry_member_count =
                                    summary.sunk_convoy_telemetry_member_count.saturating_add(1);
                            }
                        }
                    }
                    _ => {}
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                let mut clean = true;
                let emitted_domains = read_u32(record.payload, 16);
                let scheduled_domains = read_u32(record.payload, 20);
                let frame_status = read_u32(record.payload, 24);
                let reserved = read_u32(record.payload, 28);
                validate_status_health(
                    compact_contract::COMPACT_DOMAIN_FRAME,
                    frame_status,
                    "FRAME_END",
                    record.offset,
                    &mut errors,
                );
                if frame_status & !KNOWN_COMPACT_STATUS_FLAGS != 0
                    || frame_status & compact_contract::COMPACT_STATUS_FAILURE_MASK != 0
                {
                    clean = false;
                }
                if reserved != 0 {
                    clean = false;
                    errors.push(format!(
                        "offset {}: FRAME_END reserved word is nonzero ({reserved:#x})",
                        record.offset
                    ));
                }
                if emitted_domains != frame_emitted_perf_records {
                    clean = false;
                    errors.push(format!(
                        "offset {}: FRAME_END emitted_domains {emitted_domains} != independently observed DOMAIN_PERF count {frame_emitted_perf_records}",
                        record.offset
                    ));
                }
                if scheduled_domains != emitted_domains {
                    clean = false;
                    errors.push(format!(
                        "offset {}: FRAME_END scheduled_domains {scheduled_domains} != emitted_domains {emitted_domains}",
                        record.offset
                    ));
                }
                match open_frame_hour {
                    Some(open) if open == record.game_hour_raw => {}
                    Some(open) => {
                        clean = false;
                        errors.push(format!(
                            "offset {}: FRAME_END hour {} does not close open DATA frame hour {open}",
                            record.offset, record.game_hour_raw
                        ));
                    }
                    None => {
                        clean = false;
                        errors.push(format!(
                            "offset {}: FRAME_END hour {} has no open DATA frame",
                            record.offset, record.game_hour_raw
                        ));
                    }
                }
                if let Some(closed) = last_closed_hour {
                    match closed.checked_add(1) {
                        Some(expected) if expected == record.game_hour_raw => {}
                        Some(expected) => {
                            clean = false;
                            errors.push(format!(
                                "offset {}: FRAME_END hour {} does not follow closed frame hour {closed} (expected {expected})",
                                record.offset, record.game_hour_raw
                            ));
                        }
                        None => {
                            clean = false;
                            errors.push(format!(
                                "offset {}: closed frame hour {closed} exhausted; another frame is forbidden",
                                record.offset
                            ));
                        }
                    }
                }
                if frame_lifecycle_data_records != 1 {
                    clean = false;
                    errors.push(format!(
                        "offset {}: frame hour {} has {} domain 49 DATA records; expected exactly 1",
                        record.offset, record.game_hour_raw, frame_lifecycle_data_records
                    ));
                }
                if frame_member_data_records != 1 {
                    clean = false;
                    errors.push(format!(
                        "offset {}: frame hour {} has {} domain 50 DATA records; expected exactly 1",
                        record.offset, record.game_hour_raw, frame_member_data_records
                    ));
                }
                if frame_lifecycle_perf_records != 1 {
                    clean = false;
                    errors.push(format!(
                        "offset {}: frame hour {} has {} domain 49 PERF records; expected exactly 1",
                        record.offset, record.game_hour_raw, frame_lifecycle_perf_records
                    ));
                }
                if frame_member_perf_records != 1 {
                    clean = false;
                    errors.push(format!(
                        "offset {}: frame hour {} has {} domain 50 PERF records; expected exactly 1",
                        record.offset, record.game_hour_raw, frame_member_perf_records
                    ));
                }
                validate_perf_data_authority(
                    DOMAIN_NAVAL_COMBAT_LIFECYCLE,
                    frame_lifecycle_perf,
                    frame_lifecycle_data_status,
                    frame_lifecycle_data_rows,
                    record.offset,
                    &mut clean,
                    &mut errors,
                );
                validate_perf_data_authority(
                    DOMAIN_NAVAL_COMBAT_MEMBERS,
                    frame_member_perf,
                    frame_member_data_status,
                    frame_member_data_rows,
                    record.offset,
                    &mut clean,
                    &mut errors,
                );
                for (&(combat_id, generation), &deleted) in &frame_lifecycle_parents {
                    if !deleted {
                        continue;
                    }
                    let stale_child_count = active_members
                        .keys()
                        .filter(|(member_combat_id, member_generation, _, _)| {
                            *member_combat_id == combat_id && *member_generation == generation
                        })
                        .count();
                    if stale_child_count != 0 {
                        clean = false;
                        errors.push(format!(
                            "offset {}: domain 49 tombstone for combat {combat_id}:{generation} left {stale_child_count} active domain 50 member(s); complete child teardown is required in the same frame",
                            record.offset
                        ));
                    }
                }
                if clean {
                    open_frame_hour = None;
                    pending_perf_hour = None;
                    last_closed_hour = Some(record.game_hour_raw);
                    frame_lifecycle_data_records = 0;
                    frame_member_data_records = 0;
                    frame_lifecycle_perf_records = 0;
                    frame_member_perf_records = 0;
                    frame_emitted_perf_records = 0;
                    frame_lifecycle_perf = None;
                    frame_member_perf = None;
                    frame_lifecycle_data_status = None;
                    frame_member_data_status = None;
                    frame_lifecycle_data_rows = None;
                    frame_member_data_rows = None;
                    frame_lifecycle_parents.clear();
                    frame_member_keys.clear();
                    frame_live_ship_ids.clear();
                    final_record_is_clean_frame_end = true;
                }
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) =
                    compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(
                        record.payload,
                    )
                {
                    final_record_is_clean_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase()
                        == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => errors.push(format!(
                "offset {}: record type {} is outside the naval-combat proof grammar",
                record.offset,
                known_record_type.canonical_name()
            )),
        }
    }

    if let Some(open) = open_frame_hour {
        errors.push(format!("stream ended with open DATA frame hour {open}"));
    }
    if !final_record_is_clean_frame_end {
        errors.push("stream is not closed by a final canonical FRAME_END record".to_string());
    }

    summary.status_flags_or = summary
        .per_domain
        .values()
        .fold(0u32, |acc, domain| acc | domain.status_flags_or);
    let mut member_sides: BTreeMap<(u32, u64), BTreeSet<CombatSide>> = BTreeMap::new();
    for ((combat_id, generation, _, _), side) in &active_members {
        member_sides
            .entry((*combat_id, *generation))
            .or_default()
            .insert(*side);
    }
    summary.active_combat_count = lifecycle_keys.len() as u64;
    summary.two_sided_combat_count = lifecycle_keys
        .iter()
        .filter(|key| {
            member_sides
                .get(key)
                .map(|sides| {
                    sides.contains(&CombatSide::Attacker) && sides.contains(&CombatSide::Defender)
                })
                .unwrap_or(false)
        })
        .count() as u64;

    if !summary
        .domains_seen
        .contains(&DOMAIN_NAVAL_COMBAT_LIFECYCLE)
    {
        errors.push("missing naval combat lifecycle domain 49".to_string());
    }
    if !summary.domains_seen.contains(&DOMAIN_NAVAL_COMBAT_MEMBERS) {
        errors.push("missing naval combat members domain 50".to_string());
    }
    if summary.status_flags_or & FAILURE_STATUS_MASK != 0 {
        errors.push(format!(
            "failure status flags set: {}",
            summary.status_flags_or & FAILURE_STATUS_MASK
        ));
    }
    for key in lifecycle_keys {
        let two_sided = member_sides
            .get(&key)
            .map(|sides| {
                sides.contains(&CombatSide::Attacker) && sides.contains(&CombatSide::Defender)
            })
            .unwrap_or(false);
        if !two_sided {
            errors.push(format!(
                "combat {}:{} lacks two-sided member rows",
                key.0, key.1
            ));
        }
    }
    if non_convoy_member_count > 0
        && validated_non_convoy_member_count + terminal_sunk_unjoined_member_count
            != non_convoy_member_count
    {
        errors.push(format!(
            "only {} validated-live plus {} terminal-sunk non-convoy members account for {non_convoy_member_count} non-convoy rows",
            validated_non_convoy_member_count, terminal_sunk_unjoined_member_count
        ));
    }
    if summary.cship_validated_member_count > 0
        && summary.task_force_linked_member_count < summary.cship_validated_member_count
    {
        errors.push("not every CShip-validated combat member has task-force linkage".to_string());
    }
    if summary.cship_validated_member_count > 0
        && summary.fleet_linked_member_count < summary.cship_validated_member_count
    {
        errors.push("not every CShip-validated combat member has fleet linkage".to_string());
    }
    if summary.cship_validated_member_count > 0
        && summary.hull_telemetry_member_count < summary.cship_validated_member_count
    {
        errors.push("not every CShip-validated combat member has hull telemetry".to_string());
    }

    GateResult {
        ok: errors.is_empty(),
        errors,
        summary,
    }
}

fn validate_next_frame_hour(
    last_closed_hour: Option<i32>,
    game_hour_raw: i32,
    record_label: &str,
    offset: usize,
    errors: &mut Vec<String>,
) {
    let Some(closed) = last_closed_hour else {
        return;
    };
    match closed.checked_add(1) {
        Some(expected) if game_hour_raw == expected => {}
        Some(expected) => errors.push(format!(
            "offset {offset}: {record_label} hour {game_hour_raw} does not follow closed frame hour {closed} (expected {expected})"
        )),
        None => errors.push(format!(
            "offset {offset}: closed frame hour {closed} exhausted; another record is forbidden"
        )),
    }
}

fn decode_perf_authority(payload: &[u8]) -> PerfAuthority {
    PerfAuthority {
        observed_rows: read_u32(payload, 16),
        changed_rows: read_u32(payload, 20),
        unit_count: read_u32(payload, 24),
        changed_unit_count: read_u32(payload, 28),
        status_flags: read_u32(payload, 32),
    }
}

fn validate_status_health(
    domain_id: u16,
    status_flags: u32,
    record_label: &str,
    offset: usize,
    errors: &mut Vec<String>,
) {
    let unknown = status_flags & !KNOWN_COMPACT_STATUS_FLAGS;
    if unknown != 0 {
        errors.push(format!(
            "offset {offset}: {record_label} domain {domain_id} status {status_flags:#010x} contains unknown bits {unknown:#010x}"
        ));
    }
    let global =
        compact_contract::compact_status_for_global_interpretation(domain_id, status_flags);
    let fatal = global & compact_contract::COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        errors.push(format!(
            "offset {offset}: {record_label} domain {domain_id} status {status_flags:#010x} contains fatal bits {fatal:#010x}"
        ));
    }
    if domain_id == DOMAIN_NAVAL_COMBAT_LIFECYCLE
        && status_flags & STATUS_LOCATION_CANDIDATE_UNVALIDATED != 0
    {
        errors.push(format!(
            "offset {offset}: {record_label} domain 49 location/region promotion is incomplete (status bit 11 set)"
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_perf_data_authority(
    domain_id: u16,
    perf: Option<PerfAuthority>,
    data_status: Option<u32>,
    data_rows: Option<u32>,
    offset: usize,
    clean: &mut bool,
    errors: &mut Vec<String>,
) {
    let (Some(perf), Some(data_status), Some(data_rows)) = (perf, data_status, data_rows) else {
        return;
    };
    if data_status != perf.status_flags {
        *clean = false;
        errors.push(format!(
            "offset {offset}: domain {domain_id} DATA status {data_status:#010x} != PERF status {:#010x}",
            perf.status_flags
        ));
    }
    for (name, actual) in [
        ("changed_rows", perf.changed_rows),
        ("unit_count", perf.unit_count),
        ("changed_unit_count", perf.changed_unit_count),
    ] {
        if actual != data_rows {
            *clean = false;
            errors.push(format!(
                "offset {offset}: domain {domain_id} PERF {name} {actual} != decoded DATA row count {data_rows} (observed_rows={})",
                perf.observed_rows
            ));
        }
    }
}

pub fn result_json(result: &GateResult) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_bool_field(&mut out, "ok", result.ok, true);
    out.push_str("  \"errors\": ");
    push_json_string_array(&mut out, &result.errors);
    out.push_str(",\n");
    out.push_str("  \"summary\": {\n");
    push_json_u64_field(&mut out, "record_count", result.summary.record_count, true);
    push_json_u64_field(
        &mut out,
        "lifecycle_row_count",
        result.summary.lifecycle_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "member_row_count",
        result.summary.member_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "active_combat_count",
        result.summary.active_combat_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "two_sided_combat_count",
        result.summary.two_sided_combat_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "lifecycle_location_row_count",
        result.summary.lifecycle_location_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "lifecycle_region_row_count",
        result.summary.lifecycle_region_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "lifecycle_started_row_count",
        result.summary.lifecycle_started_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "lifecycle_updated_row_count",
        result.summary.lifecycle_updated_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "lifecycle_ended_row_count",
        result.summary.lifecycle_ended_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "member_deleted_row_count",
        result.summary.member_deleted_row_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "cship_validated_member_count",
        result.summary.cship_validated_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "task_force_linked_member_count",
        result.summary.task_force_linked_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "fleet_linked_member_count",
        result.summary.fleet_linked_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "hull_telemetry_member_count",
        result.summary.hull_telemetry_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "sunk_convoy_telemetry_member_count",
        result.summary.sunk_convoy_telemetry_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "convoy_member_count",
        result.summary.convoy_member_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "status_flags_or",
        result.summary.status_flags_or as u64,
        true,
    );
    out.push_str("  \"domains_seen\": [");
    for (idx, domain) in result.summary.domains_seen.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        out.push_str(&domain.to_string());
    }
    out.push_str("]\n");
    out.push_str("  }\n");
    out.push_str("}\n");
    out
}

fn decode_records(data: &[u8]) -> Result<Vec<CompactRecord<'_>>, String> {
    let mut records = Vec::new();
    let mut cursor = 0usize;
    while cursor < data.len() {
        let header = match parse_compact_record_header(&data[cursor..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(_)) => {
                return Err(format!(
                    "compact header overruns buffer at offset {cursor}: need {COMPACT_RECORD_HEADER_LEN}, remaining {}",
                    data.len().saturating_sub(cursor)
                ));
            }
            Err(RecordFramingError::BadMagic) => {
                return Err(format!("offset {cursor}: bad compact magic"));
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                return Err(format!(
                    "offset {cursor}: unsupported compact version {version}"
                ));
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                return Err(format!(
                    "offset {cursor}: unsupported compact header length {header_len}"
                ));
            }
            Err(err @ RecordFramingError::PayloadTooLarge(_)) => {
                return Err(format!("offset {cursor}: {err}"));
            }
        };
        let payload_start = cursor + COMPACT_RECORD_HEADER_LEN;
        require(data, payload_start, header.payload_len, "compact payload")?;
        let payload = &data[payload_start..payload_start + header.payload_len];
        validate_compact_record_semantics(
            header.domain_id,
            header.record_type,
            header.record_flags,
            payload,
        )
        .map_err(|err| format!("offset {cursor}: {err}"))?;
        records.push(CompactRecord {
            offset: cursor,
            sequence: header.sequence,
            game_hour_raw: header.game_hour_raw,
            domain_id: header.domain_id,
            record_type: header.record_type,
            payload,
        });
        cursor = payload_start + header.payload_len;
    }
    Ok(records)
}

fn decode_lifecycle_payload(
    offset: usize,
    payload: &[u8],
    errors: &mut Vec<String>,
) -> Option<(PayloadHeader, Vec<DecodedLifecycleRow>)> {
    let header = match decode_payload_header(offset, payload, LIFECYCLE_ROW_LEN) {
        Ok(header) => header,
        Err(err) => {
            errors.push(err);
            return None;
        }
    };
    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut rows = Vec::with_capacity(header.row_count);
    for _ in 0..header.row_count {
        if let Err(err) = require(payload, cursor, LIFECYCLE_ROW_LEN, "lifecycle row") {
            errors.push(format!("offset {offset}: {err}"));
            return None;
        }
        rows.push(DecodedLifecycleRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            flags: read_u32(payload, cursor + 20),
            location_id: read_u32(payload, cursor + 24),
            region_id: read_u32(payload, cursor + 28),
            attacker_member_count: read_u32(payload, cursor + 32),
            defender_member_count: read_u32(payload, cursor + 36),
            attacker_positioning_raw: read_i32(payload, cursor + 40),
            defender_positioning_raw: read_i32(payload, cursor + 44),
            attacker_anti_air_raw: read_i64(payload, cursor + 48),
            defender_anti_air_raw: read_i64(payload, cursor + 56),
            reserved_tail: read_u64(payload, cursor + 64),
        });
        cursor += LIFECYCLE_ROW_LEN;
    }
    validate_string_table(offset, payload, cursor, header.string_count, errors)?;
    Some((header, rows))
}

fn decode_member_payload(
    offset: usize,
    payload: &[u8],
    errors: &mut Vec<String>,
) -> Option<(PayloadHeader, Vec<DecodedMemberRow>)> {
    let header = match decode_payload_header(offset, payload, MEMBER_ROW_LEN) {
        Ok(header) => header,
        Err(err) => {
            errors.push(err);
            return None;
        }
    };
    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut rows = Vec::with_capacity(header.row_count);
    for _ in 0..header.row_count {
        if let Err(err) = require(payload, cursor, MEMBER_ROW_LEN, "member row") {
            errors.push(format!("offset {offset}: {err}"));
            return None;
        }
        rows.push(DecodedMemberRow {
            combat_id: read_u32(payload, cursor),
            generation: read_u64(payload, cursor + 4),
            flags: read_u32(payload, cursor + 12),
            side: read_u32(payload, cursor + 16),
            group_index: read_u32(payload, cursor + 20),
            member_index: read_u32(payload, cursor + 24),
            unit_type_tag: read_u32(payload, cursor + 28),
            ship_instance_id: read_u32(payload, cursor + 32),
            owner_tag_db_id: read_u32(payload, cursor + 36),
            designed_max_strength_raw: read_u32(payload, cursor + 40),
            battle_slot_index_raw: read_u32(payload, cursor + 44),
            state_bitmask: read_u32(payload, cursor + 48),
            engagement_weight_raw: read_i32(payload, cursor + 52),
            reserved: read_u32(payload, cursor + 56),
            task_force_id: read_u32(payload, cursor + 60),
            fleet_id: read_u32(payload, cursor + 64),
            strength_raw: read_u32(payload, cursor + 68),
            organisation_raw: read_u32(payload, cursor + 72),
            sunk_convoys: read_u32(payload, cursor + 76),
            experience_raw: read_u32(payload, cursor + 80),
            reserved_tail: read_u32(payload, cursor + 84),
        });
        cursor += MEMBER_ROW_LEN;
    }
    validate_string_table(offset, payload, cursor, header.string_count, errors)?;
    Some((header, rows))
}

fn decode_payload_header(
    offset: usize,
    payload: &[u8],
    expected_row_len: usize,
) -> Result<PayloadHeader, String> {
    require(payload, 0, PAYLOAD_HEADER_LEN, "payload header")
        .map_err(|err| format!("offset {offset}: {err}"))?;
    let version = read_u16(payload, 0);
    let row_len = read_u16(payload, 2) as usize;
    let status_flags = read_u32(payload, 12);
    let row_count = read_u32(payload, 16) as usize;
    let string_count = read_u32(payload, 20) as usize;
    let reserved = read_u32(payload, 24);
    if version != SCHEMA_VERSION {
        return Err(format!(
            "offset {offset}: unsupported schema version {version}"
        ));
    }
    if row_len != expected_row_len {
        return Err(format!(
            "offset {offset}: row length {row_len} != expected {expected_row_len}"
        ));
    }
    if reserved != 0 {
        return Err(format!(
            "offset {offset}: payload reserved field is nonzero"
        ));
    }
    if row_count > 100_000 {
        return Err(format!("offset {offset}: row count exceeds limit"));
    }
    if string_count != 1 {
        return Err(format!(
            "offset {offset}: naval combat lane expects exactly one empty string table entry"
        ));
    }
    Ok(PayloadHeader {
        status_flags,
        row_count,
        string_count,
    })
}

fn validate_string_table(
    offset: usize,
    payload: &[u8],
    mut cursor: usize,
    string_count: usize,
    errors: &mut Vec<String>,
) -> Option<()> {
    for index in 0..string_count {
        if let Err(err) = require(payload, cursor, 2, "string length") {
            errors.push(format!("offset {offset}: {err}"));
            return None;
        }
        let len = read_u16(payload, cursor) as usize;
        cursor += 2;
        if let Err(err) = require(payload, cursor, len, "string entry") {
            errors.push(format!("offset {offset}: {err}"));
            return None;
        }
        if index == 0 && len != 0 {
            errors.push(format!(
                "offset {offset}: string table index 0 must be empty"
            ));
        }
        cursor += len;
    }
    if cursor != payload.len() {
        errors.push(format!(
            "offset {offset}: {} trailing payload bytes",
            payload.len().saturating_sub(cursor)
        ));
    }
    Some(())
}

fn validate_decoded_lifecycle_row(
    offset: usize,
    row_index: usize,
    row: &DecodedLifecycleRow,
    errors: &mut Vec<String>,
) {
    if row.flags & !LIFECYCLE_KNOWN_FLAGS != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has unknown flags 0x{:08x}",
            row.flags & !LIFECYCLE_KNOWN_FLAGS
        ));
    }
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} invalid combat_id {}",
            row.combat_id
        ));
    }
    if row.flags & LIFECYCLE_LOCATION_PRESENT != 0
        && (row.location_id == 0 || row.location_id > MAX_LOCATION_ID)
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} invalid location_id {}",
            row.location_id
        ));
    }
    if row.flags & LIFECYCLE_LOCATION_PRESENT == 0 && row.location_id != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has location without flag"
        ));
    }
    if row.flags & LIFECYCLE_REGION_PRESENT != 0
        && (row.region_id == 0 || row.region_id > MAX_REGION_ID)
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} invalid region_id {}",
            row.region_id
        ));
    }
    if row.flags & LIFECYCLE_REGION_PRESENT == 0 && row.region_id != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has region without flag"
        ));
    }
    if row.flags & LIFECYCLE_ATTACKER_COUNT_PRESENT == 0 && row.attacker_member_count != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has attacker count without flag"
        ));
    }
    if row.flags & LIFECYCLE_DEFENDER_COUNT_PRESENT == 0 && row.defender_member_count != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has defender count without flag"
        ));
    }
    if row.flags & LIFECYCLE_ATTACKER_POSITIONING_PRESENT == 0 && row.attacker_positioning_raw != 0
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has attacker positioning without flag"
        ));
    }
    if row.flags & LIFECYCLE_DEFENDER_POSITIONING_PRESENT == 0 && row.defender_positioning_raw != 0
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has defender positioning without flag"
        ));
    }
    if row.flags & LIFECYCLE_ATTACKER_POSITIONING_PRESENT != 0
        && !(0..=MAX_POSITIONING_RAW).contains(&row.attacker_positioning_raw)
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} attacker positioning raw {} is outside 0..={MAX_POSITIONING_RAW}",
            row.attacker_positioning_raw
        ));
    }
    if row.flags & LIFECYCLE_DEFENDER_POSITIONING_PRESENT != 0
        && !(0..=MAX_POSITIONING_RAW).contains(&row.defender_positioning_raw)
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} defender positioning raw {} is outside 0..={MAX_POSITIONING_RAW}",
            row.defender_positioning_raw
        ));
    }
    if row.flags & LIFECYCLE_ATTACKER_ANTI_AIR_PRESENT == 0 && row.attacker_anti_air_raw != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has attacker anti-air without flag"
        ));
    }
    if row.flags & LIFECYCLE_DEFENDER_ANTI_AIR_PRESENT == 0 && row.defender_anti_air_raw != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has defender anti-air without flag"
        ));
    }
    if row.reserved_tail != 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} trailing reserved field is nonzero"
        ));
    }
}

fn validate_decoded_member_row(
    offset: usize,
    row_index: usize,
    row: &DecodedMemberRow,
    errors: &mut Vec<String>,
) {
    if row.flags & !MEMBER_KNOWN_FLAGS != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has unknown flags 0x{:08x}",
            row.flags & !MEMBER_KNOWN_FLAGS
        ));
    }
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        errors.push(format!(
            "offset {offset}: member row {row_index} invalid combat_id {}",
            row.combat_id
        ));
    }
    if CombatSide::from_wire_id(row.side).is_none() {
        errors.push(format!(
            "offset {offset}: member row {row_index} invalid side {}",
            row.side
        ));
    }
    if row.reserved != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} reserved field is nonzero"
        ));
    }
    if row.reserved_tail != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} trailing reserved field is nonzero"
        ));
    }
    if row.group_index > MAX_GROUP_INDEX {
        errors.push(format!(
            "offset {offset}: member row {row_index} group_index {} exceeds {MAX_GROUP_INDEX}",
            row.group_index
        ));
    }
    if row.member_index > MAX_MEMBER_INDEX {
        errors.push(format!(
            "offset {offset}: member row {row_index} member_index {} exceeds {MAX_MEMBER_INDEX}",
            row.member_index
        ));
    }
    if row.battle_slot_index_raw > MAX_BATTLE_SLOT_INDEX {
        errors.push(format!(
            "offset {offset}: member row {row_index} battle_slot_index_raw {} exceeds {MAX_BATTLE_SLOT_INDEX}",
            row.battle_slot_index_raw
        ));
    }
    let is_convoy = row.flags & MEMBER_IS_CONVOY != 0;
    if !is_convoy && row.unit_type_tag != CSHIP_TYPE_TAG {
        errors.push(format!(
            "offset {offset}: member row {row_index} non-convoy unit_type_tag {} != {CSHIP_TYPE_TAG}",
            row.unit_type_tag
        ));
    }
    if is_convoy && row.unit_type_tag != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} convoy unit_type_tag {} is not zero",
            row.unit_type_tag
        ));
    }
    if row.flags & MEMBER_SHIP_INSTANCE_PRESENT != 0 {
        if row.ship_instance_id == 0 || row.ship_instance_id > MAX_SHIP_INSTANCE_ID {
            errors.push(format!(
                "offset {offset}: member row {row_index} invalid ship_instance_id {}",
                row.ship_instance_id
            ));
        }
    } else if row.ship_instance_id != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has ship_instance_id without flag"
        ));
    }
    if row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 && row.flags & MEMBER_SHIP_INSTANCE_PRESENT == 0
    {
        errors.push(format!(
            "offset {offset}: member row {row_index} has CShip validation without ship id"
        ));
    }
    if is_convoy && (row.flags & MEMBER_SHIP_INSTANCE_PRESENT != 0 || row.ship_instance_id != 0) {
        errors.push(format!(
            "offset {offset}: member row {row_index} convoy row must not carry ship-instance identity"
        ));
    }
    if is_convoy && row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} convoy row must not carry CShip validation"
        ));
    }
    if !is_convoy && row.flags & MEMBER_SHIP_INSTANCE_PRESENT == 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} non-convoy ship lacks ship-instance identity"
        ));
    }
    let terminal_sunk =
        row.flags & MEMBER_STATE_BITMASK_PRESENT != 0 && row.state_bitmask == FEX_STATE_SUNK;
    if !is_convoy && row.flags & MEMBER_CSHIP_JOIN_VALIDATED == 0 && !terminal_sunk {
        errors.push(format!(
            "offset {offset}: member row {row_index} non-terminal non-convoy ship lacks CShip validation"
        ));
    }
    let cship_derived = MEMBER_TASK_FORCE_PRESENT
        | MEMBER_FLEET_PRESENT
        | MEMBER_STRENGTH_PRESENT
        | MEMBER_ORGANISATION_PRESENT
        | MEMBER_EXPERIENCE_PRESENT
        | MEMBER_SUNK_CONVOYS_PRESENT;
    if !is_convoy && row.flags & MEMBER_CSHIP_JOIN_VALIDATED == 0 && row.flags & cship_derived != 0
    {
        errors.push(format!(
            "offset {offset}: member row {row_index} without a live CShip join carries CShip-derived flags 0x{:08x}",
            row.flags & cship_derived
        ));
    }
    let convoy_forbidden = MEMBER_CSHIP_JOIN_VALIDATED
        | MEMBER_TASK_FORCE_PRESENT
        | MEMBER_FLEET_PRESENT
        | MEMBER_STRENGTH_PRESENT
        | MEMBER_ORGANISATION_PRESENT
        | MEMBER_EXPERIENCE_PRESENT
        | MEMBER_SUNK_CONVOYS_PRESENT;
    if is_convoy && row.flags & convoy_forbidden != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} convoy row carries CShip-derived flags 0x{:08x}",
            row.flags & convoy_forbidden
        ));
    }
    if row.flags & MEMBER_OWNER_PRESENT != 0 && row.owner_tag_db_id > MAX_OWNER_TAG_DB_ID {
        errors.push(format!(
            "offset {offset}: member row {row_index} owner_tag_db_id {} exceeds {MAX_OWNER_TAG_DB_ID}",
            row.owner_tag_db_id
        ));
    }
    if row.flags & MEMBER_OWNER_PRESENT == 0 && row.owner_tag_db_id != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has owner without flag"
        ));
    }
    if row.flags & MEMBER_DESIGNED_MAX_STRENGTH_PRESENT != 0
        && row.designed_max_strength_raw > MAX_DESIGNED_STRENGTH_RAW
    {
        errors.push(format!(
            "offset {offset}: member row {row_index} designed strength {} exceeds {MAX_DESIGNED_STRENGTH_RAW}",
            row.designed_max_strength_raw
        ));
    }
    if row.flags & MEMBER_DESIGNED_MAX_STRENGTH_PRESENT == 0 && row.designed_max_strength_raw != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has designed strength without flag"
        ));
    }
    if row.flags & MEMBER_STATE_BITMASK_PRESENT == 0 && row.state_bitmask != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has state bitmask without flag"
        ));
    }
    if row.flags & MEMBER_STATE_BITMASK_PRESENT != 0 && fex_state_name(row.state_bitmask).is_none()
    {
        errors.push(format!(
            "offset {offset}: member row {row_index} has unknown CFEX state enum 0x{:x}",
            row.state_bitmask
        ));
    }
    if row.flags & MEMBER_ENGAGEMENT_WEIGHT_PRESENT == 0 && row.engagement_weight_raw != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has engagement weight without flag"
        ));
    }
    if row.flags & MEMBER_TASK_FORCE_PRESENT != 0 {
        if row.task_force_id == 0 || row.task_force_id > MAX_OOB_ID {
            errors.push(format!(
                "offset {offset}: member row {row_index} invalid task_force_id {}",
                row.task_force_id
            ));
        }
    } else if row.task_force_id != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has task_force_id without flag"
        ));
    }
    if row.flags & MEMBER_FLEET_PRESENT != 0 {
        if row.fleet_id == 0 || row.fleet_id > MAX_OOB_ID {
            errors.push(format!(
                "offset {offset}: member row {row_index} invalid fleet_id {}",
                row.fleet_id
            ));
        }
    } else if row.fleet_id != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has fleet_id without flag"
        ));
    }
    if row.flags & MEMBER_STRENGTH_PRESENT != 0 {
        if row.strength_raw > MAX_FIXED_POINT_RAW {
            errors.push(format!(
                "offset {offset}: member row {row_index} strength_raw {} exceeds {MAX_FIXED_POINT_RAW}",
                row.strength_raw
            ));
        }
    } else if row.strength_raw != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has strength_raw without flag"
        ));
    }
    if row.flags & MEMBER_ORGANISATION_PRESENT != 0 {
        if row.organisation_raw > MAX_FIXED_POINT_RAW {
            errors.push(format!(
                "offset {offset}: member row {row_index} organisation_raw {} exceeds {MAX_FIXED_POINT_RAW}",
                row.organisation_raw
            ));
        }
    } else if row.organisation_raw != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has organisation_raw without flag"
        ));
    }
    if row.flags & MEMBER_EXPERIENCE_PRESENT != 0 {
        if row.experience_raw > MAX_FIXED_POINT_RAW {
            errors.push(format!(
                "offset {offset}: member row {row_index} experience_raw {} exceeds {MAX_FIXED_POINT_RAW}",
                row.experience_raw
            ));
        }
    } else if row.experience_raw != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has experience_raw without flag"
        ));
    }
    if row.flags & MEMBER_SUNK_CONVOYS_PRESENT != 0 {
        if row.sunk_convoys > MAX_SUNK_CONVOYS {
            errors.push(format!(
                "offset {offset}: member row {row_index} sunk_convoys {} exceeds {MAX_SUNK_CONVOYS}",
                row.sunk_convoys
            ));
        }
    } else if row.sunk_convoys != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} has sunk_convoys without flag"
        ));
    }
}

fn record_domain_data(
    summary: &mut GateSummary,
    domain_id: u16,
    header: &PayloadHeader,
    row_count: usize,
) {
    let domain = summary.per_domain.entry(domain_id).or_default();
    domain.data_record_count = domain.data_record_count.saturating_add(1);
    domain.row_count_total = domain.row_count_total.saturating_add(row_count as u64);
    domain.status_flags_or |= header.status_flags;
    match domain_id {
        DOMAIN_NAVAL_COMBAT_LIFECYCLE => add_unit(domain, "naval_combat_lifecycle_rows", row_count),
        DOMAIN_NAVAL_COMBAT_MEMBERS => add_unit(domain, "naval_combat_member_rows", row_count),
        _ => {}
    }
}

fn validate_lifecycle_row(row: &LifecycleRow) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err("invalid lifecycle combat_id".to_string());
    }
    if let Some(location_id) = row.location_id {
        if location_id == 0 || location_id > MAX_LOCATION_ID {
            return Err("invalid lifecycle location_id".to_string());
        }
    }
    if let Some(region_id) = row.region_id {
        if region_id == 0 || region_id > MAX_REGION_ID {
            return Err("invalid lifecycle region_id".to_string());
        }
    }
    for positioning in [row.attacker_positioning_raw, row.defender_positioning_raw]
        .into_iter()
        .flatten()
    {
        if !(0..=MAX_POSITIONING_RAW).contains(&positioning) {
            return Err("invalid lifecycle positioning_raw".to_string());
        }
    }
    Ok(())
}

fn validate_member_row(row: &MemberRow) -> Result<(), String> {
    if row.combat_id == 0 || row.combat_id > MAX_COMBAT_ID {
        return Err("invalid member combat_id".to_string());
    }
    if row.group_index > MAX_GROUP_INDEX || row.member_index > MAX_MEMBER_INDEX {
        return Err("invalid member vector index".to_string());
    }
    if let Some(ship_id) = row.ship_instance_id {
        if ship_id == 0 || ship_id > MAX_SHIP_INSTANCE_ID {
            return Err("invalid member ship_instance_id".to_string());
        }
    }
    if row.is_convoy {
        if row.unit_type_tag != 0 || row.ship_instance_id.is_some() {
            return Err("convoy member must carry the zero packed-handle identity".to_string());
        }
        if row.cship_join_validated
            || row.task_force_id.is_some()
            || row.fleet_id.is_some()
            || row.strength_raw.is_some()
            || row.organisation_raw.is_some()
            || row.experience_raw.is_some()
            || row.sunk_convoys.is_some()
        {
            return Err("convoy member cannot carry CShip-derived fields".to_string());
        }
    } else {
        if row.unit_type_tag != CSHIP_TYPE_TAG {
            return Err("non-convoy member must carry the CShip type tag".to_string());
        }
        if row.ship_instance_id.is_none() {
            return Err("non-convoy ship member requires ship-instance identity".to_string());
        }
        let terminal_sunk = row.state_bitmask == Some(FEX_STATE_SUNK);
        if !row.cship_join_validated && !terminal_sunk {
            return Err(
                "non-terminal non-convoy ship member requires CShip join validation".to_string(),
            );
        }
        if !row.cship_join_validated
            && (row.task_force_id.is_some()
                || row.fleet_id.is_some()
                || row.strength_raw.is_some()
                || row.organisation_raw.is_some()
                || row.experience_raw.is_some()
                || row.sunk_convoys.is_some())
        {
            return Err(
                "terminal sunk member without a live CShip join carries CShip-derived fields"
                    .to_string(),
            );
        }
    }
    if let Some(owner) = row.owner_tag_db_id {
        if owner > MAX_OWNER_TAG_DB_ID {
            return Err("invalid owner_tag_db_id".to_string());
        }
    }
    if row
        .state_bitmask
        .is_some_and(|raw| fex_state_name(raw).is_none())
    {
        return Err("invalid CFEX member state enum".to_string());
    }
    if let Some(task_force_id) = row.task_force_id {
        if task_force_id == 0 || task_force_id > MAX_OOB_ID {
            return Err("invalid task_force_id".to_string());
        }
    }
    if let Some(fleet_id) = row.fleet_id {
        if fleet_id == 0 || fleet_id > MAX_OOB_ID {
            return Err("invalid fleet_id".to_string());
        }
    }
    for (label, value) in [
        ("strength_raw", row.strength_raw),
        ("organisation_raw", row.organisation_raw),
        ("experience_raw", row.experience_raw),
    ] {
        if let Some(value) = value {
            if value > MAX_FIXED_POINT_RAW {
                return Err(format!("invalid {label}"));
            }
        }
    }
    if let Some(sunk_convoys) = row.sunk_convoys {
        if sunk_convoys > MAX_SUNK_CONVOYS {
            return Err("invalid sunk_convoys".to_string());
        }
    }
    Ok(())
}

fn append_payload_header(
    out: &mut Vec<u8>,
    row_len: usize,
    elapsed_wall_ms: u64,
    status_flags: u32,
    row_count: usize,
    string_count: usize,
) {
    out.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    out.extend_from_slice(&(row_len as u16).to_le_bytes());
    out.extend_from_slice(&elapsed_wall_ms.to_le_bytes());
    out.extend_from_slice(&status_flags.to_le_bytes());
    out.extend_from_slice(&(row_count as u32).to_le_bytes());
    out.extend_from_slice(&(string_count as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn append_empty_string_table(out: &mut Vec<u8>) {
    out.extend_from_slice(&0u16.to_le_bytes());
}

fn add_unit(domain: &mut DomainSummary, key: &'static str, value: usize) {
    let entry = domain.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn add_payload_unit(summary: &mut PayloadDecodeSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn require(data: &[u8], offset: usize, len: usize, label: &str) -> Result<(), String> {
    if offset.saturating_add(len) > data.len() {
        Err(format!(
            "{label} overruns buffer at offset {offset}: need {len}, remaining {}",
            data.len().saturating_sub(offset)
        ))
    } else {
        Ok(())
    }
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i32(out: &mut Vec<u8>, value: i32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_i64(out: &mut Vec<u8>, value: i64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u64(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_u16(data: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([data[offset], data[offset + 1]])
}

fn read_u32(data: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_i32(data: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ])
}

fn read_i64(data: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn read_u64(data: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ])
}

fn push_json_bool_field(out: &mut String, name: &str, value: bool, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(if value { "true" } else { "false" });
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_u64_field(out: &mut String, name: &str, value: u64, comma: bool) {
    out.push_str("    ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_string_array(out: &mut String, values: &[String]) {
    out.push('[');
    for (idx, value) in values.iter().enumerate() {
        if idx > 0 {
            out.push_str(", ");
        }
        push_json_str(out, value);
    }
    out.push(']');
}

fn push_json_str(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with_flags(
        domain_id: u16,
        record_type: u16,
        record_flags: u32,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut record = compact_record(1, 100, domain_id, record_type, payload);
        record[28..32].copy_from_slice(&record_flags.to_le_bytes());
        record
    }

    fn canonical_perf_payload_with_empty_pdu1() -> Vec<u8> {
        let mut payload = vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        payload.extend_from_slice(compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC);
        payload.extend_from_slice(
            &compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION.to_le_bytes(),
        );
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload
    }

    fn perf_payload(rows: usize, status_flags: u32) -> Vec<u8> {
        let rows = rows as u32;
        let mut payload = Vec::with_capacity(compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN);
        payload.extend_from_slice(&42u64.to_le_bytes());
        payload.extend_from_slice(&7u64.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&rows.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn append_frame_end(stream: &mut Vec<u8>, sequence: u64, game_hour_raw: i32) {
        let mut payload = vec![0u8; compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN];
        payload[16..20].copy_from_slice(&2u32.to_le_bytes());
        payload[20..24].copy_from_slice(&2u32.to_le_bytes());
        stream.extend_from_slice(&compact_record(
            sequence,
            game_hour_raw,
            compact_contract::COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            &payload,
        ));
    }

    fn lifecycle_row(combat_id: u32) -> LifecycleRow {
        LifecycleRow {
            combat_id,
            generation: 1,
            body_ptr: 0x22aa_0004_000 + u64::from(combat_id),
            deleted: false,
            location_id: Some(2_799),
            region_id: Some(18),
            attacker_member_count: Some(1),
            defender_member_count: Some(1),
            attacker_positioning_raw: Some(81_200),
            defender_positioning_raw: Some(76_500),
            attacker_anti_air_raw: Some(123_000),
            defender_anti_air_raw: Some(98_000),
        }
    }

    fn ship_member(
        combat_id: u32,
        side: CombatSide,
        member_index: u32,
        ship_instance_id: u32,
    ) -> MemberRow {
        MemberRow {
            combat_id,
            generation: 1,
            deleted: false,
            side,
            group_index: 0,
            member_index,
            unit_type_tag: CSHIP_TYPE_TAG,
            ship_instance_id: Some(ship_instance_id),
            cship_join_validated: true,
            is_convoy: false,
            owner_tag_db_id: Some(17),
            designed_max_strength_raw: Some(10_000_000),
            battle_slot_index_raw: member_index,
            state_bitmask: Some(1),
            engagement_weight_raw: Some(1_000),
            task_force_id: Some(477),
            fleet_id: Some(88),
            strength_raw: Some(9_500_000),
            organisation_raw: Some(8_750_000),
            experience_raw: Some(100_000),
            sunk_convoys: Some(0),
        }
    }

    fn convoy_member(combat_id: u32, side: CombatSide, member_index: u32) -> MemberRow {
        MemberRow {
            combat_id,
            generation: 1,
            deleted: false,
            side,
            group_index: 0,
            member_index,
            unit_type_tag: 0,
            ship_instance_id: None,
            cship_join_validated: false,
            is_convoy: true,
            owner_tag_db_id: Some(18),
            designed_max_strength_raw: Some(10_000_000),
            battle_slot_index_raw: member_index,
            state_bitmask: Some(1),
            engagement_weight_raw: Some(500),
            task_force_id: None,
            fleet_id: None,
            strength_raw: None,
            organisation_raw: None,
            experience_raw: None,
            sunk_convoys: None,
        }
    }

    fn closed_frame(
        lifecycle_status: u32,
        lifecycle: &[LifecycleRow],
        members: &[MemberRow],
    ) -> Vec<u8> {
        let mut stream = Vec::new();
        append_owned_frame(&mut stream, 1, 512, lifecycle_status, lifecycle, members);
        stream
    }

    fn append_owned_frame(
        stream: &mut Vec<u8>,
        sequence_start: u64,
        game_hour_raw: i32,
        lifecycle_status: u32,
        lifecycle: &[LifecycleRow],
        members: &[MemberRow],
    ) {
        stream.extend_from_slice(&compact_record(
            sequence_start,
            game_hour_raw,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(lifecycle.len(), lifecycle_status),
        ));
        stream.extend_from_slice(&compact_record(
            sequence_start + 1,
            game_hour_raw,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, lifecycle_status, lifecycle),
        ));
        stream.extend_from_slice(&compact_record(
            sequence_start + 2,
            game_hour_raw,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(members.len(), STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            sequence_start + 3,
            game_hour_raw,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, members),
        ));
        append_frame_end(stream, sequence_start + 4, game_hour_raw);
    }

    #[test]
    fn record_flag_policy_accepts_canonical_schema_flag() {
        let schema = record_with_flags(
            45,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );

        assert!(decode_records(&schema).is_ok());
    }

    #[test]
    fn perf_trailer_accepts_valid_announced_pdu1() {
        let payload = canonical_perf_payload_with_empty_pdu1();
        let perf = record_with_flags(
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        assert!(decode_records(&perf).is_ok());
    }

    #[test]
    fn perf_trailer_rejects_missing_announced_pdu1() {
        let payload = vec![0; compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN];
        let perf = record_with_flags(
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        let error = decode_records(&perf).expect_err("missing announced trailer rejected");
        assert!(error.contains("diagnostic trailer header is truncated"));
    }

    #[test]
    fn perf_trailer_rejects_unannounced_pdu1() {
        let payload = canonical_perf_payload_with_empty_pdu1();
        let perf = record_with_flags(
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &payload,
        );

        let error = decode_records(&perf).expect_err("unannounced trailer rejected");
        assert!(error.contains("unannounced trailer byte(s)"));
    }

    #[test]
    fn perf_trailer_rejects_malformed_pdu1() {
        let mut payload = canonical_perf_payload_with_empty_pdu1();
        payload[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN] = b'X';
        let perf = record_with_flags(
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            compact_contract::COMPACT_RECORD_TYPE_DOMAIN_PERF,
            compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
            &payload,
        );

        let error = decode_records(&perf).expect_err("malformed trailer rejected");
        assert!(error.contains("diagnostic trailer magic is invalid"));
    }

    #[test]
    fn record_flag_policy_rejects_wrong_placement_and_unknown_bits() {
        let wrong_placement = record_with_flags(
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            &[],
        );
        let unknown = record_with_flags(
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            compact_contract::COMPACT_RECORD_TYPE_FRAME_END,
            1 << 29,
            &[],
        );

        let wrong_error = decode_records(&wrong_placement).expect_err("wrong placement rejected");
        let unknown_error = decode_records(&unknown).expect_err("unknown flag rejected");
        assert!(wrong_error.contains("rejected bits 0x80000000"));
        assert!(unknown_error.contains("rejected bits 0x20000000"));
    }

    fn passing_stream() -> Vec<u8> {
        let lifecycle = [lifecycle_row(240_580)];
        let members = [
            ship_member(240_580, CombatSide::Attacker, 0, 1001),
            convoy_member(240_580, CombatSide::Defender, 0),
        ];
        closed_frame(STATUS_DELTA_DATA, &lifecycle, &members)
    }

    fn gate_sequences(sequences: &[u64]) -> GateResult {
        let mut stream = Vec::new();
        for &sequence in sequences {
            stream.extend_from_slice(&compact_record(
                sequence,
                512,
                DOMAIN_NAVAL_COMBAT_LIFECYCLE,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                &[],
            ));
        }
        gate_stream(&stream)
    }

    #[test]
    fn rejects_missing_sequence_prefix() {
        let result = gate_sequences(&[2]);

        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("sequence 2 does not start at 1")));
    }

    #[test]
    fn rejects_sequence_gap() {
        let result = gate_sequences(&[1, 3]);

        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 3 (expected 2)")));
    }

    #[test]
    fn rejects_repeated_sequence() {
        let result = gate_sequences(&[1, 1]);

        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 1 -> 1 (expected 2)")));
    }

    #[test]
    fn rejects_sequence_regression() {
        let result = gate_sequences(&[1, 2, 1]);

        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("sequence discontinuity 2 -> 1 (expected 3)")));
    }

    #[test]
    fn rejects_sequence_wrap_after_max() {
        let result = gate_sequences(&[u64::MAX, 0]);

        assert!(result.errors.iter().any(|error| {
            error.contains("sequence space exhausted after 18446744073709551615")
                && error.contains("wrapping is not allowed")
        }));
    }

    #[test]
    fn two_sided_stream_passes_gate() {
        let result = gate_stream(&passing_stream());
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.summary.lifecycle_row_count, 1);
        assert_eq!(result.summary.member_row_count, 2);
        assert_eq!(result.summary.two_sided_combat_count, 1);
        assert_eq!(result.summary.lifecycle_location_row_count, 1);
        assert_eq!(result.summary.lifecycle_region_row_count, 1);
        assert_eq!(
            result.summary.status_flags_or & STATUS_LOCATION_CANDIDATE_UNVALIDATED,
            0
        );
        assert_eq!(result.summary.member_deleted_row_count, 0);
        assert_eq!(result.summary.cship_validated_member_count, 1);
        assert_eq!(result.summary.task_force_linked_member_count, 1);
        assert_eq!(result.summary.fleet_linked_member_count, 1);
        assert_eq!(result.summary.hull_telemetry_member_count, 1);
        assert_eq!(result.summary.sunk_convoy_telemetry_member_count, 1);
        assert_eq!(result.summary.convoy_member_count, 1);
    }

    #[test]
    fn delta_member_update_may_reference_persistent_d49_parent() {
        let mut stream = passing_stream();
        let member_update = [ship_member(240_580, CombatSide::Attacker, 0, 1001)];
        stream.extend_from_slice(&compact_record(
            6,
            513,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            7,
            513,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, STATUS_DELTA_DATA, &[]),
        ));
        stream.extend_from_slice(&compact_record(
            8,
            513,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(member_update.len(), STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            9,
            513,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, &member_update),
        ));
        append_frame_end(&mut stream, 10, 513);

        let result = gate_stream(&stream);
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.summary.active_combat_count, 1);
        assert_eq!(result.summary.two_sided_combat_count, 1);
    }

    #[test]
    fn parent_tombstone_requires_complete_same_frame_child_teardown() {
        let mut stream = passing_stream();
        let mut parent_tombstone = lifecycle_row(240_580);
        parent_tombstone.deleted = true;
        let mut ship_tombstone = ship_member(240_580, CombatSide::Attacker, 0, 1001);
        ship_tombstone.deleted = true;
        append_owned_frame(
            &mut stream,
            6,
            513,
            STATUS_DELTA_DATA,
            &[parent_tombstone],
            &[ship_tombstone],
        );

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result.errors.iter().any(|error| {
            error.contains("left 1 active domain 50 member(s)") && error.contains("same frame")
        }));
    }

    #[test]
    fn complete_same_frame_parent_and_child_teardown_is_terminal_and_valid() {
        let mut stream = passing_stream();
        let mut parent_tombstone = lifecycle_row(240_580);
        parent_tombstone.deleted = true;
        let mut ship_tombstone = ship_member(240_580, CombatSide::Attacker, 0, 1001);
        ship_tombstone.deleted = true;
        let mut convoy_tombstone = convoy_member(240_580, CombatSide::Defender, 0);
        convoy_tombstone.deleted = true;
        append_owned_frame(
            &mut stream,
            6,
            513,
            STATUS_DELTA_DATA,
            &[parent_tombstone],
            &[ship_tombstone, convoy_tombstone],
        );

        let result = gate_stream(&stream);
        assert!(result.ok, "{:?}", result.errors);
        assert_eq!(result.summary.active_combat_count, 0);
        assert_eq!(result.summary.two_sided_combat_count, 0);
    }

    #[test]
    fn duplicate_tombstones_and_same_generation_resurrection_are_rejected() {
        let mut retired_stream = passing_stream();
        let mut parent_tombstone = lifecycle_row(240_580);
        parent_tombstone.deleted = true;
        let mut ship_tombstone = ship_member(240_580, CombatSide::Attacker, 0, 1001);
        ship_tombstone.deleted = true;
        let mut convoy_tombstone = convoy_member(240_580, CombatSide::Defender, 0);
        convoy_tombstone.deleted = true;
        append_owned_frame(
            &mut retired_stream,
            6,
            513,
            STATUS_DELTA_DATA,
            &[parent_tombstone.clone()],
            &[ship_tombstone.clone(), convoy_tombstone.clone()],
        );

        let mut duplicate = retired_stream.clone();
        append_owned_frame(
            &mut duplicate,
            11,
            514,
            STATUS_DELTA_DATA,
            &[parent_tombstone],
            &[ship_tombstone, convoy_tombstone],
        );
        let duplicate_result = gate_stream(&duplicate);
        assert!(duplicate_result
            .errors
            .iter()
            .any(|error| { error.contains("lifecycle tombstone") && error.contains("duplicate") }));
        assert!(duplicate_result.errors.iter().any(|error| {
            error.contains("deleted domain 50 membership")
                && error.contains("was not previously active")
        }));

        let mut resurrected = retired_stream;
        append_owned_frame(
            &mut resurrected,
            11,
            514,
            STATUS_DELTA_DATA,
            &[lifecycle_row(240_580)],
            &[
                ship_member(240_580, CombatSide::Attacker, 0, 1001),
                convoy_member(240_580, CombatSide::Defender, 0),
            ],
        );
        let resurrection_result = gate_stream(&resurrected);
        assert!(resurrection_result
            .errors
            .iter()
            .any(|error| error
                .contains("retired combat 240580:1 attempts same-generation resurrection")));
        assert!(resurrection_result.errors.iter().any(|error| {
            error.contains("domain 50 membership combat 240580:1")
                && error.contains("same-generation resurrection")
        }));
    }

    #[test]
    fn every_global_failure_bit_is_fatal_not_only_partial_and_source() {
        for fatal in [
            compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PROFILE_MISMATCH,
            compact_contract::COMPACT_STATUS_NO_CLOCK,
            compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE,
            compact_contract::COMPACT_STATUS_WORKER_PANIC,
            compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            compact_contract::COMPACT_STATUS_CONTROL_INVALID,
        ] {
            let result = gate_stream(&closed_frame(
                STATUS_DELTA_DATA | fatal,
                &[lifecycle_row(7)],
                &[
                    ship_member(7, CombatSide::Attacker, 0, 1001),
                    convoy_member(7, CombatSide::Defender, 0),
                ],
            ));
            assert!(
                !result.ok
                    && result
                        .errors
                        .iter()
                        .any(|error| error.contains("contains fatal bits")),
                "fatal={fatal:#x}, errors={:?}",
                result.errors
            );
        }
    }

    #[test]
    fn rejects_unknown_data_status_and_perf_data_authority_mismatch() {
        let unknown = gate_stream(&closed_frame(
            STATUS_DELTA_DATA | (1 << 31),
            &[lifecycle_row(7)],
            &[
                ship_member(7, CombatSide::Attacker, 0, 1001),
                convoy_member(7, CombatSide::Defender, 0),
            ],
        ));
        assert!(unknown
            .errors
            .iter()
            .any(|error| error.contains("contains unknown bits")));

        let mut mismatched = passing_stream();
        let perf_changed_rows = COMPACT_RECORD_HEADER_LEN + 20;
        mismatched[perf_changed_rows..perf_changed_rows + 4].copy_from_slice(&0u32.to_le_bytes());
        let mismatch = gate_stream(&mismatched);
        assert!(mismatch
            .errors
            .iter()
            .any(|error| error.contains("PERF changed_rows 0 != decoded DATA row count 1")));
    }

    #[test]
    fn owned_data_and_perf_status_authority_must_match_exactly() {
        let mut stream = passing_stream();
        let perf_status_offset = COMPACT_RECORD_HEADER_LEN + 32;
        let perf_status = read_u32(&stream, perf_status_offset)
            | compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED;
        stream[perf_status_offset..perf_status_offset + 4]
            .copy_from_slice(&perf_status.to_le_bytes());

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result.errors.iter().any(|error| {
            error.contains("domain 49 DATA status") && error.contains("!= PERF status")
        }));
    }

    #[test]
    fn rejects_frame_end_health_and_accounting_corruption() {
        let frame_end_offset = passing_stream().len()
            - COMPACT_RECORD_HEADER_LEN
            - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;

        let mut fatal = passing_stream();
        fatal[frame_end_offset + COMPACT_RECORD_HEADER_LEN + 24
            ..frame_end_offset + COMPACT_RECORD_HEADER_LEN + 28]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_WORKER_PANIC.to_le_bytes());
        assert!(gate_stream(&fatal)
            .errors
            .iter()
            .any(|error| error.contains("FRAME_END") && error.contains("contains fatal bits")));

        let mut wrong_count = passing_stream();
        wrong_count[frame_end_offset + COMPACT_RECORD_HEADER_LEN + 16
            ..frame_end_offset + COMPACT_RECORD_HEADER_LEN + 20]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(gate_stream(&wrong_count)
            .errors
            .iter()
            .any(|error| error.contains("emitted_domains 1") && error.contains("PERF count 2")));

        let mut reserved = passing_stream();
        reserved[frame_end_offset + COMPACT_RECORD_HEADER_LEN + 28
            ..frame_end_offset + COMPACT_RECORD_HEADER_LEN + 32]
            .copy_from_slice(&1u32.to_le_bytes());
        assert!(gate_stream(&reserved)
            .errors
            .iter()
            .any(|error| error.contains("FRAME_END reserved word is nonzero")));
    }

    #[test]
    fn per_hour_cardinality_requires_one_perf_and_one_data_for_both_domains() {
        let lifecycle = [lifecycle_row(7)];
        let members = [
            ship_member(7, CombatSide::Attacker, 0, 1001),
            convoy_member(7, CombatSide::Defender, 0),
        ];
        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(lifecycle.len(), STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle),
        ));
        stream.extend_from_slice(&compact_record(
            3,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, &members),
        ));
        append_frame_end(&mut stream, 4, 512);

        let result = gate_stream(&stream);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("0 domain 50 PERF records; expected exactly 1")));
    }

    #[test]
    fn d50_perf_and_data_cannot_precede_d49_even_when_payloads_are_empty() {
        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, &[]),
        ));
        stream.extend_from_slice(&compact_record(
            3,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(0, STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            4,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, STATUS_DELTA_DATA, &[]),
        ));
        append_frame_end(&mut stream, 5, 512);

        let result = gate_stream(&stream);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("domain 50 PERF precedes its domain 49 PERF")));
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("domain 50 DATA precedes its domain 49 DATA")));
    }

    #[test]
    fn d49_location_region_fail_closed_status_is_lane_fatal() {
        let stream = closed_frame(
            STATUS_DELTA_DATA | STATUS_LOCATION_CANDIDATE_UNVALIDATED,
            &[lifecycle_row(7)],
            &[
                ship_member(7, CombatSide::Attacker, 0, 1001),
                convoy_member(7, CombatSide::Defender, 0),
            ],
        );

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("status bit 11 set")));
    }

    #[test]
    fn d49_rows_cannot_hide_missing_promoted_location_region() {
        let mut lifecycle = lifecycle_row(7);
        lifecycle.location_id = None;
        lifecycle.region_id = None;
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[lifecycle],
            &[
                ship_member(7, CombatSide::Attacker, 0, 1001),
                convoy_member(7, CombatSide::Defender, 0),
            ],
        ));

        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("lacks promoted location/region")));
    }

    #[test]
    fn rejects_d50_members_before_their_d49_parent_rows() {
        let lifecycle = [lifecycle_row(7)];
        let members = [
            ship_member(7, CombatSide::Attacker, 0, 1001),
            convoy_member(7, CombatSide::Defender, 0),
        ];
        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, &members),
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle),
        ));
        append_frame_end(&mut stream, 3, 512);

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("domain 50 DATA precedes its domain 49 DATA")));
    }

    #[test]
    fn rejects_d50_rows_bound_to_a_different_d49_parent() {
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[lifecycle_row(7)],
            &[
                ship_member(8, CombatSide::Attacker, 0, 1001),
                convoy_member(8, CombatSide::Defender, 0),
            ],
        ));

        assert!(!result.ok);
        assert!(result.errors.iter().any(|error| {
            error.contains("combat 8:1 is orphaned")
                && error.contains("active domain 49 lifecycle state")
        }));
    }

    #[test]
    fn rejects_active_members_dangling_under_a_deleted_parent() {
        let mut deleted_parent = lifecycle_row(7);
        deleted_parent.deleted = true;
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[deleted_parent],
            &[
                ship_member(7, CombatSide::Attacker, 0, 1001),
                convoy_member(7, CombatSide::Defender, 0),
            ],
        ));

        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| { error.contains("orphaned from active domain 49 lifecycle state") }));
    }

    #[test]
    fn rejects_duplicate_parent_and_membership_identity() {
        let parent = lifecycle_row(7);
        let ship = ship_member(7, CombatSide::Attacker, 0, 1001);
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[parent.clone(), parent],
            &[ship.clone(), ship],
        ));

        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("duplicate/ambiguous domain 49 parent")));
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("duplicate/ambiguous domain 50 membership")));
    }

    #[test]
    fn rejects_duplicate_live_ship_across_distinct_membership_positions() {
        let mut duplicate = ship_member(7, CombatSide::Defender, 1, 1001);
        duplicate.owner_tag_db_id = Some(18);
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[lifecycle_row(7)],
            &[ship_member(7, CombatSide::Attacker, 0, 1001), duplicate],
        ));

        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("duplicate/ambiguous live ship_instance_id 1001")));
    }

    #[test]
    fn one_valid_ship_cannot_mask_an_unjoined_non_convoy_ship() {
        let lifecycle = [lifecycle_row(7)];
        let members = [
            ship_member(7, CombatSide::Attacker, 0, 1001),
            ship_member(7, CombatSide::Defender, 0, 1002),
        ];
        let lifecycle_payload = encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle);
        let mut member_payload = encode_member_payload(42, STATUS_DELTA_DATA, &members);
        let second_row_flags = PAYLOAD_HEADER_LEN + MEMBER_ROW_LEN + 12;
        let flags = read_u32(&member_payload, second_row_flags)
            & !(MEMBER_SHIP_INSTANCE_PRESENT | MEMBER_CSHIP_JOIN_VALIDATED);
        member_payload[second_row_flags..second_row_flags + 4]
            .copy_from_slice(&flags.to_le_bytes());
        let second_ship_id = PAYLOAD_HEADER_LEN + MEMBER_ROW_LEN + 32;
        member_payload[second_ship_id..second_ship_id + 4].copy_from_slice(&0u32.to_le_bytes());

        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &lifecycle_payload,
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &member_payload,
        ));
        append_frame_end(&mut stream, 3, 512);

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("non-convoy ship lacks ship-instance identity")));
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("non-terminal non-convoy ship lacks CShip validation")));
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("only 1 validated-live plus 0 terminal-sunk")));
    }

    #[test]
    fn terminal_sunk_member_can_outlive_its_cship_join_without_fabricated_telemetry() {
        let mut sunk = ship_member(7, CombatSide::Defender, 0, 1002);
        sunk.state_bitmask = Some(FEX_STATE_SUNK);
        sunk.cship_join_validated = false;
        sunk.task_force_id = None;
        sunk.fleet_id = None;
        sunk.strength_raw = None;
        sunk.organisation_raw = None;
        sunk.experience_raw = None;
        sunk.sunk_convoys = None;
        let result = gate_stream(&closed_frame(
            STATUS_DELTA_DATA,
            &[lifecycle_row(7)],
            &[ship_member(7, CombatSide::Attacker, 0, 1001), sunk],
        ));
        assert!(result.ok, "{:?}", result.errors);
    }

    #[test]
    fn convoy_zero_handle_semantics_reject_cship_identity() {
        let lifecycle = [lifecycle_row(7)];
        let members = [
            ship_member(7, CombatSide::Attacker, 0, 1001),
            convoy_member(7, CombatSide::Defender, 0),
        ];
        let lifecycle_payload = encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle);
        let mut member_payload = encode_member_payload(42, STATUS_DELTA_DATA, &members);
        let convoy_flags_offset = PAYLOAD_HEADER_LEN + MEMBER_ROW_LEN + 12;
        let flags = read_u32(&member_payload, convoy_flags_offset) | MEMBER_SHIP_INSTANCE_PRESENT;
        member_payload[convoy_flags_offset..convoy_flags_offset + 4]
            .copy_from_slice(&flags.to_le_bytes());
        let convoy_type_offset = PAYLOAD_HEADER_LEN + MEMBER_ROW_LEN + 28;
        member_payload[convoy_type_offset..convoy_type_offset + 4]
            .copy_from_slice(&CSHIP_TYPE_TAG.to_le_bytes());
        let convoy_ship_offset = PAYLOAD_HEADER_LEN + MEMBER_ROW_LEN + 32;
        member_payload[convoy_ship_offset..convoy_ship_offset + 4]
            .copy_from_slice(&1002u32.to_le_bytes());

        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &lifecycle_payload,
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &member_payload,
        ));
        append_frame_end(&mut stream, 3, 512);

        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("convoy unit_type_tag 51 is not zero")));
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("convoy row must not carry ship-instance identity")));
    }

    #[test]
    fn unowned_domain_payload_is_opaque_after_envelope_and_boundary_checks() {
        let lifecycle = [lifecycle_row(7)];
        let members = [
            ship_member(7, CombatSide::Attacker, 0, 1001),
            convoy_member(7, CombatSide::Defender, 0),
        ];
        let mut stream = Vec::new();
        stream.extend_from_slice(&compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(lifecycle.len(), STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            2,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle),
        ));
        stream.extend_from_slice(&compact_record(
            3,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &perf_payload(members.len(), STATUS_DELTA_DATA),
        ));
        stream.extend_from_slice(&compact_record(
            4,
            512,
            DOMAIN_NAVAL_COMBAT_MEMBERS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &encode_member_payload(42, STATUS_DELTA_DATA, &members),
        ));
        stream.extend_from_slice(&compact_record(
            5,
            512,
            777,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &[0xff, 0x00, 0x7f],
        ));
        append_frame_end(&mut stream, 6, 512);

        let result = gate_stream(&stream);
        assert!(result.ok, "{:?}", result.errors);
        assert!(result.summary.domains_seen.contains(&777));
    }

    #[test]
    fn rejects_unclosed_tail_and_mismatched_frame_end_hour() {
        let mut unclosed = passing_stream();
        unclosed.truncate(
            unclosed.len()
                - COMPACT_RECORD_HEADER_LEN
                - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN,
        );
        let unclosed_result = gate_stream(&unclosed);
        assert!(unclosed_result
            .errors
            .iter()
            .any(|error| error.contains("final canonical FRAME_END")));

        let mut mismatched = passing_stream();
        let frame_end_offset = mismatched.len()
            - COMPACT_RECORD_HEADER_LEN
            - compact_contract::COMPACT_FRAME_END_PAYLOAD_LEN;
        mismatched[frame_end_offset + 16..frame_end_offset + 20]
            .copy_from_slice(&513i32.to_le_bytes());
        let mismatched_result = gate_stream(&mismatched);
        assert!(mismatched_result
            .errors
            .iter()
            .any(|error| error.contains("does not close open DATA frame hour 512")));
    }

    #[test]
    fn per_domain_payload_decoders_summarize_recorder_units() {
        let stream = passing_stream();
        let records = decode_records(&stream).expect("records decode");
        let lifecycle = records
            .iter()
            .find(|record| {
                record.domain_id == DOMAIN_NAVAL_COMBAT_LIFECYCLE
                    && record.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            })
            .expect("lifecycle record");
        let members = records
            .iter()
            .find(|record| {
                record.domain_id == DOMAIN_NAVAL_COMBAT_MEMBERS
                    && record.record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA
            })
            .expect("member record");

        let lifecycle_summary = decode_lifecycle_data_payload(lifecycle.offset, lifecycle.payload);
        assert_eq!(lifecycle_summary.domain_id, DOMAIN_NAVAL_COMBAT_LIFECYCLE);
        assert!(lifecycle_summary.parse_errors.is_empty());
        assert_eq!(lifecycle_summary.row_count, 1);
        assert_eq!(
            lifecycle_summary
                .data_units_total
                .get("naval_combat_lifecycle_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            lifecycle_summary
                .data_units_total
                .get("naval_combat_lifecycle_location_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            lifecycle_summary
                .data_units_total
                .get("naval_combat_lifecycle_region_rows")
                .copied(),
            Some(1)
        );

        let member_summary = decode_members_data_payload(members.offset, members.payload);
        assert_eq!(member_summary.domain_id, DOMAIN_NAVAL_COMBAT_MEMBERS);
        assert!(member_summary.parse_errors.is_empty());
        assert_eq!(member_summary.row_count, 2);
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_member_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_cship_validated_member_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_convoy_member_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_member_deleted_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_task_force_linked_member_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_fleet_linked_member_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            member_summary
                .data_units_total
                .get("naval_combat_hull_telemetry_member_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn fails_without_member_domain() {
        let lifecycle = [LifecycleRow {
            combat_id: 7,
            generation: 1,
            body_ptr: 0x22aa_0004_000,
            deleted: false,
            location_id: None,
            region_id: None,
            attacker_member_count: Some(1),
            defender_member_count: Some(1),
            attacker_positioning_raw: None,
            defender_positioning_raw: None,
            attacker_anti_air_raw: None,
            defender_anti_air_raw: None,
        }];
        let payload = encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle);
        let stream = compact_record(
            1,
            512,
            DOMAIN_NAVAL_COMBAT_LIFECYCLE,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        let result = gate_stream(&stream);
        assert!(!result.ok);
        assert!(result
            .errors
            .iter()
            .any(|error| error.contains("missing naval combat members domain 50")));
    }

    #[test]
    fn rejects_non_convoy_ship_without_cship_join() {
        let row = MemberRow {
            combat_id: 240_580,
            generation: 1,
            deleted: false,
            side: CombatSide::Attacker,
            group_index: 0,
            member_index: 0,
            unit_type_tag: CSHIP_TYPE_TAG,
            ship_instance_id: Some(1001),
            cship_join_validated: false,
            is_convoy: false,
            owner_tag_db_id: Some(17),
            designed_max_strength_raw: None,
            battle_slot_index_raw: 0,
            state_bitmask: None,
            engagement_weight_raw: None,
            task_force_id: None,
            fleet_id: None,
            strength_raw: None,
            organisation_raw: None,
            experience_raw: None,
            sunk_convoys: None,
        };
        let panic = std::panic::catch_unwind(|| {
            let _ = encode_member_payload(42, STATUS_DELTA_DATA, &[row]);
        });
        assert!(panic.is_err());
    }

    #[test]
    fn registry_ids_match_compact_domain_assignment() {
        let registry = include_str!("../../../domain_registry.json");
        assert!(registry.contains("\"id\": 49"));
        assert!(registry.contains("\"name\": \"naval_combat_lifecycle_live\""));
        assert!(registry.contains("\"id\": 50"));
        assert!(registry.contains("\"name\": \"naval_combat_members_live\""));
    }
}
