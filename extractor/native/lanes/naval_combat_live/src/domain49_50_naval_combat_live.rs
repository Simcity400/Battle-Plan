use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const DOMAIN_NAVAL_COMBAT_LIFECYCLE: u16 = 49;
pub const DOMAIN_NAVAL_COMBAT_LIFECYCLE_NAME: &str = "naval_combat_lifecycle_live";
pub const DOMAIN_NAVAL_COMBAT_MEMBERS: u16 = 50;
pub const DOMAIN_NAVAL_COMBAT_MEMBERS_NAME: &str = "naval_combat_members_live";

pub const SCHEMA_VERSION: u16 = 1;
pub const PAYLOAD_HEADER_LEN: usize = 28;
pub const LIFECYCLE_ROW_LEN: usize = 72;
pub const MEMBER_ROW_LEN: usize = 88;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const FAILURE_STATUS_MASK: u32 = STATUS_SOURCE_UNAVAILABLE | STATUS_PARTIAL_DATA;

pub const LIFECYCLE_ROW_DELETED: u32 = 1 << 0;
pub const LIFECYCLE_LOCATION_PRESENT: u32 = 1 << 1;
pub const LIFECYCLE_ATTACKER_COUNT_PRESENT: u32 = 1 << 2;
pub const LIFECYCLE_DEFENDER_COUNT_PRESENT: u32 = 1 << 3;
pub const LIFECYCLE_ATTACKER_POSITIONING_PRESENT: u32 = 1 << 4;
pub const LIFECYCLE_DEFENDER_POSITIONING_PRESENT: u32 = 1 << 5;
pub const LIFECYCLE_REGION_PRESENT: u32 = 1 << 6;

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

pub const MAX_COMBAT_ID: u32 = 0x7fff_ffff;
pub const MAX_SHIP_INSTANCE_ID: u32 = 4_000_000;
pub const MAX_OWNER_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_GROUP_INDEX: u32 = 64;
pub const MAX_MEMBER_INDEX: u32 = 1_000;
pub const MAX_BATTLE_SLOT_INDEX: u32 = 4_096;
pub const MAX_LOCATION_ID: u32 = 1_000_000;
pub const MAX_REGION_ID: u32 = 100_000;
pub const MAX_DESIGNED_STRENGTH_RAW: u32 = 1_000_000_000;
pub const MAX_FIXED_POINT_RAW: u32 = 1_000_000_000;
pub const MAX_SUNK_CONVOYS: u32 = 1_000_000;
pub const MAX_OOB_ID: u32 = 4_000_000;
pub const CSHIP_TYPE_TAG: u32 = 51;

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
    pub attacker_positioning_milli: Option<i32>,
    pub defender_positioning_milli: Option<i32>,
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

#[derive(Debug)]
struct DecodedLifecycleRow {
    combat_id: u32,
    generation: u64,
    flags: u32,
    location_id: u32,
    region_id: u32,
    attacker_member_count: u32,
    defender_member_count: u32,
    attacker_positioning_milli: i32,
    defender_positioning_milli: i32,
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
        if row.attacker_positioning_milli.is_some() {
            flags |= LIFECYCLE_ATTACKER_POSITIONING_PRESENT;
        }
        if row.defender_positioning_milli.is_some() {
            flags |= LIFECYCLE_DEFENDER_POSITIONING_PRESENT;
        }
        push_u32(&mut payload, row.combat_id);
        push_u64(&mut payload, row.generation);
        push_u64(&mut payload, row.body_ptr);
        push_u32(&mut payload, flags);
        push_u32(&mut payload, row.location_id.unwrap_or(0));
        push_u32(&mut payload, row.region_id.unwrap_or(0));
        push_u32(&mut payload, row.attacker_member_count.unwrap_or(0));
        push_u32(&mut payload, row.defender_member_count.unwrap_or(0));
        push_i32(&mut payload, row.attacker_positioning_milli.unwrap_or(0));
        push_i32(&mut payload, row.defender_positioning_milli.unwrap_or(0));
        push_u64(&mut payload, 0);
        push_u64(&mut payload, 0);
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
    let mut lifecycle_keys = BTreeSet::new();
    let mut seen_lifecycle_keys = BTreeSet::new();
    let mut member_sides: BTreeMap<(u32, u64), BTreeSet<CombatSide>> = BTreeMap::new();

    for record in records {
        if let Some(previous) = last_sequence {
            if record.sequence <= previous {
                errors.push(format!(
                    "offset {}: sequence {} did not advance past {}",
                    record.offset, record.sequence, previous
                ));
            }
        }
        last_sequence = Some(record.sequence);
        if record.record_type != COMPACT_RECORD_TYPE_DOMAIN_DATA {
            errors.push(format!(
                "offset {}: unexpected record type {}",
                record.offset, record.record_type
            ));
            continue;
        }
        summary.domains_seen.insert(record.domain_id);
        match record.domain_id {
            DOMAIN_NAVAL_COMBAT_LIFECYCLE => {
                let (header, rows) =
                    match decode_lifecycle_payload(record.offset, record.payload, &mut errors) {
                        Some(decoded) => decoded,
                        None => continue,
                    };
                record_domain_data(&mut summary, record.domain_id, &header, rows.len());
                summary.lifecycle_row_count = summary
                    .lifecycle_row_count
                    .saturating_add(rows.len() as u64);
                for (row_index, row) in rows.into_iter().enumerate() {
                    validate_decoded_lifecycle_row(record.offset, row_index, &row, &mut errors);
                    if row.flags & LIFECYCLE_LOCATION_PRESENT != 0 {
                        summary.lifecycle_location_row_count =
                            summary.lifecycle_location_row_count.saturating_add(1);
                    }
                    if row.flags & LIFECYCLE_REGION_PRESENT != 0 {
                        summary.lifecycle_region_row_count =
                            summary.lifecycle_region_row_count.saturating_add(1);
                    }
                    if row.flags & LIFECYCLE_ROW_DELETED == 0 {
                        let key = (row.combat_id, row.generation);
                        lifecycle_keys.insert(key);
                        if seen_lifecycle_keys.insert(key) {
                            summary.lifecycle_started_row_count =
                                summary.lifecycle_started_row_count.saturating_add(1);
                        } else {
                            summary.lifecycle_updated_row_count =
                                summary.lifecycle_updated_row_count.saturating_add(1);
                        }
                    } else {
                        let key = (row.combat_id, row.generation);
                        summary.lifecycle_ended_row_count =
                            summary.lifecycle_ended_row_count.saturating_add(1);
                        if !seen_lifecycle_keys.contains(&key) {
                            errors.push(format!(
                                "offset {}: lifecycle end row for combat {}:{} was not preceded by an active row",
                                record.offset, key.0, key.1
                            ));
                        }
                    }
                }
            }
            DOMAIN_NAVAL_COMBAT_MEMBERS => {
                let (header, rows) =
                    match decode_member_payload(record.offset, record.payload, &mut errors) {
                        Some(decoded) => decoded,
                        None => continue,
                    };
                record_domain_data(&mut summary, record.domain_id, &header, rows.len());
                summary.member_row_count =
                    summary.member_row_count.saturating_add(rows.len() as u64);
                for (row_index, row) in rows.into_iter().enumerate() {
                    validate_decoded_member_row(record.offset, row_index, &row, &mut errors);
                    if row.flags & MEMBER_IS_CONVOY != 0 {
                        summary.convoy_member_count = summary.convoy_member_count.saturating_add(1);
                    }
                    if row.flags & MEMBER_ROW_DELETED != 0 {
                        summary.member_deleted_row_count =
                            summary.member_deleted_row_count.saturating_add(1);
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
                    if row.flags & (MEMBER_STRENGTH_PRESENT | MEMBER_ORGANISATION_PRESENT) != 0 {
                        summary.hull_telemetry_member_count =
                            summary.hull_telemetry_member_count.saturating_add(1);
                    }
                    if row.flags & MEMBER_SUNK_CONVOYS_PRESENT != 0 {
                        summary.sunk_convoy_telemetry_member_count =
                            summary.sunk_convoy_telemetry_member_count.saturating_add(1);
                    }
                    if row.flags & MEMBER_ROW_DELETED == 0 {
                        if let Some(side) = CombatSide::from_wire_id(row.side) {
                            member_sides
                                .entry((row.combat_id, row.generation))
                                .or_default()
                                .insert(side);
                        }
                    }
                }
            }
            other => errors.push(format!(
                "offset {}: unexpected domain {other}",
                record.offset
            )),
        }
    }

    summary.status_flags_or = summary
        .per_domain
        .values()
        .fold(0u32, |acc, domain| acc | domain.status_flags_or);
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
    if lifecycle_keys.is_empty() {
        errors.push("no active lifecycle rows".to_string());
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
    if summary.member_row_count > 0 && summary.cship_validated_member_count == 0 {
        errors.push("member rows exist but no non-convoy CShip join was validated".to_string());
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
        require(data, cursor, COMPACT_RECORD_HEADER_LEN, "compact header")?;
        if &data[cursor..cursor + 4] != COMPACT_RECORD_MAGIC {
            return Err(format!("offset {cursor}: bad compact magic"));
        }
        let version = read_u16(data, cursor + 4);
        let header_len = read_u16(data, cursor + 6) as usize;
        let sequence = read_u64(data, cursor + 8);
        let domain_id = read_u16(data, cursor + 20);
        let record_type = read_u16(data, cursor + 22);
        let payload_len = read_u32(data, cursor + 24) as usize;
        let reserved = read_u32(data, cursor + 28);
        if version != COMPACT_RECORD_VERSION {
            return Err(format!(
                "offset {cursor}: unsupported compact version {version}"
            ));
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            return Err(format!(
                "offset {cursor}: unsupported compact header length {header_len}"
            ));
        }
        if reserved != 0 {
            return Err(format!(
                "offset {cursor}: compact header reserved field is nonzero"
            ));
        }
        let payload_start = cursor + COMPACT_RECORD_HEADER_LEN;
        require(data, payload_start, payload_len, "compact payload")?;
        records.push(CompactRecord {
            offset: cursor,
            sequence,
            domain_id,
            record_type,
            payload: &data[payload_start..payload_start + payload_len],
        });
        cursor = payload_start + payload_len;
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
            attacker_positioning_milli: read_i32(payload, cursor + 40),
            defender_positioning_milli: read_i32(payload, cursor + 44),
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
    if row.flags & LIFECYCLE_LOCATION_PRESENT == 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} is missing required location"
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
    if row.flags & LIFECYCLE_REGION_PRESENT == 0 {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} is missing required region"
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
    if row.flags & LIFECYCLE_ATTACKER_POSITIONING_PRESENT == 0
        && row.attacker_positioning_milli != 0
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has attacker positioning without flag"
        ));
    }
    if row.flags & LIFECYCLE_DEFENDER_POSITIONING_PRESENT == 0
        && row.defender_positioning_milli != 0
    {
        errors.push(format!(
            "offset {offset}: lifecycle row {row_index} has defender positioning without flag"
        ));
    }
}

fn validate_decoded_member_row(
    offset: usize,
    row_index: usize,
    row: &DecodedMemberRow,
    errors: &mut Vec<String>,
) {
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
    if row.flags & MEMBER_IS_CONVOY == 0 && row.unit_type_tag != CSHIP_TYPE_TAG {
        errors.push(format!(
            "offset {offset}: member row {row_index} non-convoy unit_type_tag {} != {CSHIP_TYPE_TAG}",
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
    if row.flags & MEMBER_IS_CONVOY != 0 && row.flags & MEMBER_CSHIP_JOIN_VALIDATED != 0 {
        errors.push(format!(
            "offset {offset}: member row {row_index} convoy row must not carry CShip validation"
        ));
    }
    if row.flags & MEMBER_IS_CONVOY == 0
        && row.flags & MEMBER_SHIP_INSTANCE_PRESENT != 0
        && row.flags & MEMBER_CSHIP_JOIN_VALIDATED == 0
    {
        errors.push(format!(
            "offset {offset}: member row {row_index} non-convoy ship lacks CShip validation"
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
    if !row.is_convoy && row.ship_instance_id.is_some() && !row.cship_join_validated {
        return Err("non-convoy ship member requires CShip join validation".to_string());
    }
    if row.is_convoy && row.cship_join_validated {
        return Err("convoy member cannot carry CShip join validation".to_string());
    }
    if let Some(owner) = row.owner_tag_db_id {
        if owner > MAX_OWNER_TAG_DB_ID {
            return Err("invalid owner_tag_db_id".to_string());
        }
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

    fn passing_stream() -> Vec<u8> {
        let lifecycle = [LifecycleRow {
            combat_id: 240_580,
            generation: 1,
            body_ptr: 0x22aa_0004_000,
            deleted: false,
            location_id: Some(2799),
            region_id: Some(18),
            attacker_member_count: Some(1),
            defender_member_count: Some(1),
            attacker_positioning_milli: Some(812),
            defender_positioning_milli: Some(765),
        }];
        let members = [
            MemberRow {
                combat_id: 240_580,
                generation: 1,
                deleted: false,
                side: CombatSide::Attacker,
                group_index: 0,
                member_index: 0,
                unit_type_tag: CSHIP_TYPE_TAG,
                ship_instance_id: Some(1001),
                cship_join_validated: true,
                is_convoy: false,
                owner_tag_db_id: Some(17),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 0,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(1_000),
                task_force_id: Some(477),
                fleet_id: Some(88),
                strength_raw: Some(9_500_000),
                organisation_raw: Some(8_750_000),
                experience_raw: Some(100_000),
                sunk_convoys: Some(0),
            },
            MemberRow {
                combat_id: 240_580,
                generation: 1,
                deleted: false,
                side: CombatSide::Defender,
                group_index: 0,
                member_index: 0,
                unit_type_tag: 0,
                ship_instance_id: None,
                cship_join_validated: false,
                is_convoy: true,
                owner_tag_db_id: Some(18),
                designed_max_strength_raw: Some(10_000_000),
                battle_slot_index_raw: 1,
                state_bitmask: Some(1),
                engagement_weight_raw: Some(500),
                task_force_id: None,
                fleet_id: None,
                strength_raw: None,
                organisation_raw: None,
                experience_raw: None,
                sunk_convoys: None,
            },
        ];
        let lifecycle_payload = encode_lifecycle_payload(42, STATUS_DELTA_DATA, &lifecycle);
        let member_payload = encode_member_payload(42, STATUS_DELTA_DATA, &members);
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
        stream
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
        assert_eq!(result.summary.member_deleted_row_count, 0);
        assert_eq!(result.summary.cship_validated_member_count, 1);
        assert_eq!(result.summary.task_force_linked_member_count, 1);
        assert_eq!(result.summary.fleet_linked_member_count, 1);
        assert_eq!(result.summary.hull_telemetry_member_count, 1);
        assert_eq!(result.summary.sunk_convoy_telemetry_member_count, 1);
        assert_eq!(result.summary.convoy_member_count, 1);
    }

    #[test]
    fn per_domain_payload_decoders_summarize_recorder_units() {
        let stream = passing_stream();
        let records = decode_records(&stream).expect("records decode");
        let lifecycle = records
            .iter()
            .find(|record| record.domain_id == DOMAIN_NAVAL_COMBAT_LIFECYCLE)
            .expect("lifecycle record");
        let members = records
            .iter()
            .find(|record| record.domain_id == DOMAIN_NAVAL_COMBAT_MEMBERS)
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
            attacker_positioning_milli: None,
            defender_positioning_milli: None,
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
