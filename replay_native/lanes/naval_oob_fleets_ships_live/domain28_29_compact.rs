use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::process;

pub const DOMAIN_OOB_COMMAND: u16 = 28;
pub const DOMAIN_SHIPS: u16 = 29;

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const COMPACT_STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const COMPACT_STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const COMPACT_STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const COMPACT_STATUS_DELTA_DATA: u32 = 1 << 10;
pub const COMPACT_STATUS_FAILURE_MASK: u32 = COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA;

pub const COMPACT_FIXED_SCALE_RAW: u32 = 100_000;

pub const COMPACT_OOB_ROW_LEN: usize = 80;
pub const COMPACT_OOB_MEMBER_ROW_LEN: usize = 8;
pub const COMPACT_OOB_KIND_ARMY: u32 = 1;
pub const COMPACT_OOB_KIND_ARMY_GROUP: u32 = 2;
pub const COMPACT_OOB_KIND_FLEET: u32 = 3;
pub const COMPACT_OOB_KIND_TASK_FORCE: u32 = 4;
pub const COMPACT_OOB_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_OOB_ROW_FLAG_PARENT: u32 = 1 << 1;
pub const COMPACT_OOB_ROW_FLAG_OWNER: u32 = 1 << 2;
pub const COMPACT_OOB_ROW_FLAG_LEADER: u32 = 1 << 3;
pub const COMPACT_OOB_ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
pub const COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
pub const COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
pub const COMPACT_OOB_ROW_FLAG_LOCATION: u32 = 1 << 7;
pub const COMPACT_OOB_MAX_ID: u32 = 4_000_000;
pub const COMPACT_OOB_MAX_LOCATION_PROVINCE_ID: u32 = 1_000_000;
pub const COMPACT_OOB_MAX_MEMBER_ROWS: usize = 250_000;
pub const COMPACT_OOB_MAX_LEADER_SKILL: u32 = 15;
pub const COMPACT_OOB_STATUS_OVERFLOW: u32 = 3;

pub const COMPACT_SHIP_ROW_BASE_LEN: usize = 40;
pub const COMPACT_SHIP_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_SHIP_ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
pub const COMPACT_SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
pub const COMPACT_SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;
pub const COMPACT_SHIP_MAX_ID: u32 = 4_000_000;
pub const COMPACT_SHIP_MAX_OWNER_TAG_DB_ID: u32 = 2_048;
pub const COMPACT_SHIP_MAX_CATEGORY: u32 = 128;
pub const COMPACT_SHIP_MAX_STRENGTH_RAW: u32 = 1_000_000_000;
pub const COMPACT_SHIP_MAX_ORGANISATION_RAW: u32 = 100_000_000;
pub const COMPACT_SHIP_MAX_EXPERIENCE_RAW: u32 = 100_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DomainGateSummary {
    pub record_count: u64,
    pub data_record_count: u64,
    pub perf_record_count: u64,
    pub row_count_total: u64,
    pub changed_rows_total: u64,
    pub unit_count_total: u64,
    pub changed_unit_count_total: u64,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub max_read_time_us: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavalOobGateSummary {
    pub record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub frame_end_record_count: u64,
    pub total_payload_bytes: u64,
    pub max_payload_len: usize,
    pub max_record_len: usize,
    pub first_game_hour_raw: Option<i32>,
    pub last_game_hour_raw: Option<i32>,
    pub unique_game_hour_count: usize,
    pub domains_seen: BTreeSet<u16>,
    pub per_domain: BTreeMap<u16, DomainGateSummary>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

impl NavalOobGateSummary {
    pub fn gate_passed(&self) -> bool {
        self.parse_errors.is_empty()
            && self.ordering_errors.is_empty()
            && required_domain_ok(self.per_domain.get(&DOMAIN_OOB_COMMAND))
            && required_domain_ok(self.per_domain.get(&DOMAIN_SHIPS))
            && self
                .per_domain
                .values()
                .all(|domain| domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK == 0)
    }
}

pub fn decode_stream(raw: &[u8]) -> NavalOobGateSummary {
    let mut summary = NavalOobGateSummary::default();
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut hours = BTreeSet::new();

    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: trailing {remaining} byte(s), expected compact record header"
            ));
            break;
        }

        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        if version != COMPACT_RECORD_VERSION {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported record version {version}"
            ));
            break;
        }
        let header_len = read_u16_le(header, 6) as usize;
        if header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported header length {header_len}"
            ));
            break;
        }

        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        summary.first_game_hour_raw.get_or_insert(game_hour_raw);
        summary.last_game_hour_raw = Some(game_hour_raw);
        hours.insert(game_hour_raw);

        let payload_start = offset + COMPACT_RECORD_HEADER_LEN;
        let payload_end = payload_start + payload_len;
        let payload = &raw[payload_start..payload_end];

        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary
            .total_payload_bytes
            .saturating_add(payload_len as u64);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        summary.max_record_len = summary.max_record_len.max(record_len);
        summary.domains_seen.insert(domain_id);

        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
                summary.domain_perf_record_count =
                    summary.domain_perf_record_count.saturating_add(1);
                decode_domain_perf_record(offset, domain_id, payload, &mut summary);
            }
            COMPACT_RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count = summary.frame_end_record_count.saturating_add(1);
            }
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
                summary.domain_data_record_count =
                    summary.domain_data_record_count.saturating_add(1);
                match domain_id {
                    DOMAIN_OOB_COMMAND => {
                        summary
                            .per_domain
                            .entry(domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_oob_command_data_payload(offset, payload, &mut summary);
                    }
                    DOMAIN_SHIPS => {
                        summary
                            .per_domain
                            .entry(domain_id)
                            .or_default()
                            .data_record_count += 1;
                        decode_ships_data_payload(offset, payload, &mut summary);
                    }
                    _ => {}
                }
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: unsupported record type {record_type}"
            )),
        }

        offset = payload_end;
    }

    summary.unique_game_hour_count = hours.len();
    summary
}

fn decode_domain_perf_record(
    offset: usize,
    domain_id: u16,
    payload: &[u8],
    summary: &mut NavalOobGateSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: domain perf payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let read_time_us = read_u64_le(payload, 8);
    let row_count = read_u32_le(payload, 16) as u64;
    let changed_rows = read_u32_le(payload, 20) as u64;
    let unit_count = read_u32_le(payload, 24) as u64;
    let changed_unit_count = read_u32_le(payload, 28) as u64;
    let status_flags = read_u32_le(payload, 32);

    let domain = summary.per_domain.entry(domain_id).or_default();
    domain.record_count = domain.record_count.saturating_add(1);
    domain.perf_record_count = domain.perf_record_count.saturating_add(1);
    domain.row_count_total = domain.row_count_total.saturating_add(row_count);
    domain.changed_rows_total = domain.changed_rows_total.saturating_add(changed_rows);
    domain.unit_count_total = domain.unit_count_total.saturating_add(unit_count);
    domain.changed_unit_count_total = domain
        .changed_unit_count_total
        .saturating_add(changed_unit_count);
    domain.max_read_time_us = domain.max_read_time_us.max(read_time_us);
    domain.status_flags_or |= status_flags;
}

pub fn decode_oob_command_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut NavalOobGateSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_army_group_rows = read_u32_le(payload, 28) as usize;
    let member_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let observed_fleet_rows = read_u32_le(payload, 40) as usize;
    let observed_task_force_rows = read_u32_le(payload, 44) as usize;

    {
        let domain = summary.per_domain.entry(DOMAIN_OOB_COMMAND).or_default();
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }

    if member_count > COMPACT_OOB_MAX_MEMBER_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} exceeds limit {COMPACT_OOB_MAX_MEMBER_ROWS}"
        ));
    }
    let expected_len = 48usize
        .saturating_add(row_count.saturating_mul(COMPACT_OOB_ROW_LEN))
        .saturating_add(member_count.saturating_mul(COMPACT_OOB_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }

    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_command_rows", row_count);
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_command_root_rows", root_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_army_rows",
        observed_army_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_army_group_rows",
        observed_army_group_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_member_rows", member_count);
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_deleted_rows", deleted_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_fleet_rows",
        observed_fleet_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_observed_task_force_rows",
        observed_task_force_rows,
    );

    let mut cursor = 48usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_members = 0usize;
    let mut emitted_army_rows = 0usize;
    let mut emitted_army_group_rows = 0usize;
    let mut emitted_fleet_rows = 0usize;
    let mut emitted_task_force_rows = 0usize;
    let mut member_division_rows = 0usize;
    let mut member_army_rows = 0usize;
    let mut member_task_force_rows = 0usize;
    let mut member_ship_rows = 0usize;
    let mut leader_rows = 0usize;
    let mut leader_skill_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut location_rows = 0usize;
    let mut engine_count_rows = 0usize;
    let mut missing_count_rows = 0usize;

    for row_index in 0..row_count {
        let kind = read_u32_le(payload, cursor);
        let object_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let parent_id = read_u32_le(payload, cursor + 12);
        let owner_tag_db_id = read_u32_le(payload, cursor + 16);
        let leader_id = read_u32_le(payload, cursor + 20);
        let leader_skill = read_u32_le(payload, cursor + 24);
        let member_first = read_u32_le(payload, cursor + 28) as usize;
        let row_member_count = read_u32_le(payload, cursor + 32) as usize;
        let logical_member_count = read_u32_le(payload, cursor + 36) as usize;
        let member_count_engine = read_u32_le(payload, cursor + 40);
        let missing_member_count = read_u32_le(payload, cursor + 44);
        let leader_trait_count = read_u32_le(payload, cursor + 48);
        let parent_status = read_u32_le(payload, cursor + 52);
        let member_status = read_u32_le(payload, cursor + 56);
        let owner_status = read_u32_le(payload, cursor + 60);
        let leader_status = read_u32_le(payload, cursor + 64);
        let leader_trait_status = read_u32_le(payload, cursor + 68);
        let location_province_id = read_u32_le(payload, cursor + 72);
        let row_reserved1 = read_u32_le(payload, cursor + 76);
        cursor += COMPACT_OOB_ROW_LEN;

        let deleted = (flags & COMPACT_OOB_ROW_FLAG_DELETED) != 0;
        match kind {
            COMPACT_OOB_KIND_ARMY => emitted_army_rows = emitted_army_rows.saturating_add(1),
            COMPACT_OOB_KIND_ARMY_GROUP => {
                emitted_army_group_rows = emitted_army_group_rows.saturating_add(1)
            }
            COMPACT_OOB_KIND_FLEET => emitted_fleet_rows = emitted_fleet_rows.saturating_add(1),
            COMPACT_OOB_KIND_TASK_FORCE => {
                emitted_task_force_rows = emitted_task_force_rows.saturating_add(1)
            }
            _ => summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid kind {kind}"
            )),
        }
        if !valid_oob_id(object_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid object_id {object_id}"
            ));
        } else if !seen_rows.insert((kind, object_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} duplicates {kind}:{object_id}"
            ));
        }
        if flags
            & !(COMPACT_OOB_ROW_FLAG_DELETED
                | COMPACT_OOB_ROW_FLAG_PARENT
                | COMPACT_OOB_ROW_FLAG_OWNER
                | COMPACT_OOB_ROW_FLAG_LEADER
                | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT
                | COMPACT_OOB_ROW_FLAG_LOCATION)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted
            && (parent_id != 0
                || owner_tag_db_id != 0
                || leader_id != 0
                || leader_skill != 0
                || row_member_count != 0
                || logical_member_count != 0
                || member_count_engine != 0
                || missing_member_count != 0
                || leader_trait_count != 0
                || location_province_id != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command deleted row {row_index} has live fields"
            ));
        }

        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_PARENT) != 0,
            parent_id,
            "parent_id",
        );
        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0,
            owner_tag_db_id,
            "owner_tag_db_id",
        );
        validate_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0,
            leader_id,
            "leader_id",
        );
        if (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0 {
            leader_rows = leader_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LOCATION) != 0 {
            location_rows = location_rows.saturating_add(1);
            if location_province_id == 0
                || location_province_id > COMPACT_OOB_MAX_LOCATION_PROVINCE_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid location_province_id {location_province_id}"
                ));
            }
        } else if location_province_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has location_province_id without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER_SKILL) != 0 {
            leader_skill_rows = leader_skill_rows.saturating_add(1);
            if leader_skill > COMPACT_OOB_MAX_LEADER_SKILL {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid leader_skill {leader_skill}"
                ));
            }
        } else if leader_skill != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has leader_skill without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE) != 0 {
            engine_count_rows = engine_count_rows.saturating_add(1);
        } else if member_count_engine != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has engine member count without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT) != 0 {
            missing_count_rows = missing_count_rows.saturating_add(1);
        } else if missing_member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has missing member count without flag"
            ));
        }
        for (status_name, status_value) in [
            ("parent_status", parent_status),
            ("member_status", member_status),
            ("owner_status", owner_status),
            ("leader_status", leader_status),
            ("leader_trait_status", leader_trait_status),
        ] {
            if status_value > 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid {status_name} {status_value}"
                ));
            }
        }
        if row_member_count == 0 {
            if member_first != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has member_first without members"
                ));
            }
        } else {
            let Some(member_end) = member_first.checked_add(row_member_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range overflows"
                ));
                continue;
            };
            if member_end > member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range {member_first}..{member_end} exceeds {member_count}"
                ));
            }
            declared_members = declared_members.saturating_add(row_member_count);
            match kind {
                COMPACT_OOB_KIND_ARMY => {
                    member_division_rows = member_division_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_ARMY_GROUP => {
                    member_army_rows = member_army_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_FLEET => {
                    member_task_force_rows = member_task_force_rows.saturating_add(row_member_count)
                }
                COMPACT_OOB_KIND_TASK_FORCE => {
                    member_ship_rows = member_ship_rows.saturating_add(row_member_count)
                }
                _ => {}
            }
        }
        if member_status != COMPACT_OOB_STATUS_OVERFLOW
            && row_member_count != logical_member_count
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} member_count {row_member_count} != logical_member_count {logical_member_count}"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} reserved field is nonzero"
            ));
        }
    }

    if declared_members != member_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} != declared {declared_members}"
        ));
    }

    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_army_rows", emitted_army_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_army_group_rows",
        emitted_army_group_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_fleet_rows", emitted_fleet_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_task_force_rows",
        emitted_task_force_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_division_rows",
        member_division_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_army_rows",
        member_army_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_task_force_rows",
        member_task_force_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_member_ship_rows",
        member_ship_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_leader_rows", leader_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_leader_skill_rows",
        leader_skill_rows,
    );
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_owner_rows", owner_rows);
    add_unit(summary, DOMAIN_OOB_COMMAND, "oob_location_rows", location_rows);
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_engine_member_count_rows",
        engine_count_rows,
    );
    add_unit(
        summary,
        DOMAIN_OOB_COMMAND,
        "oob_missing_member_count_rows",
        missing_count_rows,
    );

    let mut seen_member_rows = BTreeSet::new();
    for row_index in 0..member_count {
        let member_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_OOB_MEMBER_ROW_LEN;
        if !valid_oob_id(member_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} has invalid member_id {member_id}"
            ));
        }
        if !seen_member_rows.insert(row_index) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} duplicates member row index"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} reserved field is nonzero"
            ));
        }
    }
}

pub fn decode_ships_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut NavalOobGateSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_ship_rows = read_u32_le(payload, 20) as usize;
    let observed_task_force_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let reserved0 = read_u32_le(payload, 36);

    {
        let domain = summary.per_domain.entry(DOMAIN_SHIPS).or_default();
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }

    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_ship_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships row_count {row_count} > observed_ship_rows {observed_ship_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: ships scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships header reserved field is nonzero"
        ));
    }

    add_unit(summary, DOMAIN_SHIPS, "ship_rows", row_count);
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_observed_ship_rows",
        observed_ship_rows,
    );
    add_unit(
        summary,
        DOMAIN_SHIPS,
        "ship_observed_task_force_rows",
        observed_task_force_rows,
    );
    add_unit(summary, DOMAIN_SHIPS, "ship_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_rows = BTreeSet::new();
    let mut task_force_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut definition_rows = 0usize;

    for row_index in 0..row_count {
        if cursor.saturating_add(COMPACT_SHIP_ROW_BASE_LEN) > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row needs {COMPACT_SHIP_ROW_BASE_LEN} byte(s), only {} remain",
                payload.len().saturating_sub(cursor)
            ));
            return;
        }

        let instance_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let task_force_id = read_u32_le(payload, cursor + 8);
        let category_enum = read_u32_le(payload, cursor + 12);
        let strength_raw = read_u32_le(payload, cursor + 16);
        let organisation_raw = read_u32_le(payload, cursor + 20);
        let experience_raw = read_u32_le(payload, cursor + 24);
        let sunk_convoys = read_u32_le(payload, cursor + 28);
        let owner_tag_db_id = read_u32_le(payload, cursor + 32);
        let row_reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_SHIP_ROW_BASE_LEN;

        let definition_len = match skip_compact_string(payload, &mut cursor, "ships definition") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };

        let deleted = (flags & COMPACT_SHIP_ROW_FLAG_DELETED) != 0;
        if !valid_ship_id(instance_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid instance_id {instance_id}"
            ));
        } else if !seen_rows.insert((instance_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} duplicates instance_id {instance_id}"
            ));
        }
        if flags
            & !(COMPACT_SHIP_ROW_FLAG_DELETED
                | COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                | COMPACT_SHIP_ROW_FLAG_DEFINITION
                | COMPACT_SHIP_ROW_FLAG_OWNER)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_TASK_FORCE) != 0 {
            task_force_rows = task_force_rows.saturating_add(1);
            if !valid_oob_id(task_force_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid task_force_id {task_force_id}"
                ));
            }
        } else if task_force_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has task_force_id without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_DEFINITION) != 0 {
            definition_rows = definition_rows.saturating_add(1);
            if definition_len == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has definition flag without definition"
                ));
            }
        } else if definition_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has definition without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
            if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_SHIP_MAX_OWNER_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                ));
            }
        } else if owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has owner_tag_db_id without flag"
            ));
        }
        if category_enum > COMPACT_SHIP_MAX_CATEGORY {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid category_enum {category_enum}"
            ));
        }
        if strength_raw > COMPACT_SHIP_MAX_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid strength_raw {strength_raw}"
            ));
        }
        if organisation_raw > COMPACT_SHIP_MAX_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid organisation_raw {organisation_raw}"
            ));
        }
        if experience_raw > COMPACT_SHIP_MAX_EXPERIENCE_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid experience_raw {experience_raw}"
            ));
        }
        if deleted
            && (task_force_id != 0
                || category_enum != 0
                || strength_raw != 0
                || organisation_raw != 0
                || experience_raw != 0
                || sunk_convoys != 0
                || owner_tag_db_id != 0
                || definition_len != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships deleted row {row_index} has live fields"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} reserved field is nonzero"
            ));
        }
    }

    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_unit(summary, DOMAIN_SHIPS, "ship_task_force_rows", task_force_rows);
    add_unit(summary, DOMAIN_SHIPS, "ship_owner_rows", owner_rows);
    add_unit(summary, DOMAIN_SHIPS, "ship_definition_rows", definition_rows);
}

fn add_unit(
    summary: &mut NavalOobGateSummary,
    domain_id: u16,
    key: &'static str,
    value: usize,
) {
    let domain = summary.per_domain.entry(domain_id).or_default();
    *domain.data_units_total.entry(key).or_default() += value as u64;
}

fn required_domain_ok(domain: Option<&DomainGateSummary>) -> bool {
    domain.is_some_and(|domain| {
        domain.data_record_count > 0
            && domain.data_rows_total > 0
            && domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK == 0
    })
}

fn valid_oob_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_OOB_MAX_ID
}

fn valid_ship_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_SHIP_MAX_ID
}

fn validate_oob_optional_id(
    summary: &mut NavalOobGateSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if !valid_oob_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid {field} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command row {row_index} has {field} without flag"
        ));
    }
}

fn skip_compact_string(payload: &[u8], cursor: &mut usize, field: &str) -> Result<usize, String> {
    if cursor.saturating_add(2) > payload.len() {
        return Err(format!("{field} string length exceeds payload"));
    }
    let len = read_u16_le(payload, *cursor) as usize;
    *cursor += 2;
    if cursor.saturating_add(len) > payload.len() {
        return Err(format!(
            "{field} string length {len} exceeds remaining payload"
        ));
    }
    let bytes = &payload[*cursor..*cursor + len];
    if std::str::from_utf8(bytes).is_err() {
        return Err(format!("{field} string is not valid UTF-8"));
    }
    *cursor += len;
    Ok(len)
}

fn read_u16_le(payload: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(payload[offset..offset + 2].try_into().expect("u16 range"))
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

fn read_u64_le(payload: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(payload[offset..offset + 8].try_into().expect("u64 range"))
}

fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("usage: {} <collector_stream.bin>", args[0]);
        process::exit(2);
    }
    let raw = match fs::read(&args[1]) {
        Ok(raw) => raw,
        Err(err) => {
            eprintln!("failed to read {}: {err}", args[1]);
            process::exit(2);
        }
    };
    let summary = decode_stream(&raw);
    println!("{}", summary_json(&summary));
    if !summary.gate_passed() {
        process::exit(1);
    }
}

fn summary_json(summary: &NavalOobGateSummary) -> String {
    let mut out = String::new();
    out.push_str("{\n");
    push_json_bool_field(&mut out, "gate_passed", summary.gate_passed(), true);
    push_json_u64_field(&mut out, "record_count", summary.record_count, true);
    push_json_u64_field(
        &mut out,
        "domain_perf_record_count",
        summary.domain_perf_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "domain_data_record_count",
        summary.domain_data_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "frame_end_record_count",
        summary.frame_end_record_count,
        true,
    );
    push_json_u64_field(
        &mut out,
        "total_payload_bytes",
        summary.total_payload_bytes,
        true,
    );
    push_json_usize_field(&mut out, "max_payload_len", summary.max_payload_len, true);
    push_json_usize_field(&mut out, "max_record_len", summary.max_record_len, true);
    push_json_opt_i32_field(
        &mut out,
        "first_game_hour_raw",
        summary.first_game_hour_raw,
        true,
    );
    push_json_opt_i32_field(
        &mut out,
        "last_game_hour_raw",
        summary.last_game_hour_raw,
        true,
    );
    push_json_usize_field(
        &mut out,
        "unique_game_hour_count",
        summary.unique_game_hour_count,
        true,
    );

    out.push_str("  \"domains_seen\": [");
    for (index, domain_id) in summary.domains_seen.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(&domain_id.to_string());
    }
    out.push_str("],\n");

    out.push_str("  \"per_domain\": {\n");
    for (idx, domain_id) in [DOMAIN_OOB_COMMAND, DOMAIN_SHIPS].iter().enumerate() {
        if idx > 0 {
            out.push_str(",\n");
        }
        out.push_str("    ");
        push_json_str(&mut out, &domain_id.to_string());
        out.push_str(": ");
        if let Some(domain) = summary.per_domain.get(domain_id) {
            out.push_str(&domain_summary_json(*domain_id, domain));
        } else {
            out.push_str("null");
        }
    }
    out.push_str("\n  },\n");

    out.push_str("  \"parse_errors\": ");
    push_json_str_array(&mut out, &summary.parse_errors);
    out.push_str(",\n");
    out.push_str("  \"ordering_errors\": ");
    push_json_str_array(&mut out, &summary.ordering_errors);
    out.push_str("\n}\n");
    out
}

fn domain_summary_json(domain_id: u16, domain: &DomainGateSummary) -> String {
    let mut out = String::new();
    out.push_str("{");
    out.push_str("\"name\":");
    push_json_str(&mut out, domain_name(domain_id));
    out.push_str(",\"record_count\":");
    out.push_str(&domain.record_count.to_string());
    out.push_str(",\"perf_record_count\":");
    out.push_str(&domain.perf_record_count.to_string());
    out.push_str(",\"data_record_count\":");
    out.push_str(&domain.data_record_count.to_string());
    out.push_str(",\"row_count_total\":");
    out.push_str(&domain.row_count_total.to_string());
    out.push_str(",\"changed_rows_total\":");
    out.push_str(&domain.changed_rows_total.to_string());
    out.push_str(",\"unit_count_total\":");
    out.push_str(&domain.unit_count_total.to_string());
    out.push_str(",\"changed_unit_count_total\":");
    out.push_str(&domain.changed_unit_count_total.to_string());
    out.push_str(",\"data_rows_total\":");
    out.push_str(&domain.data_rows_total.to_string());
    out.push_str(",\"data_changed_rows_total\":");
    out.push_str(&domain.data_changed_rows_total.to_string());
    out.push_str(",\"status_flags_or\":");
    out.push_str(&domain.status_flags_or.to_string());
    out.push_str(",\"failure_flags_or\":");
    out.push_str(&(domain.status_flags_or & COMPACT_STATUS_FAILURE_MASK).to_string());
    out.push_str(",\"max_read_time_us\":");
    out.push_str(&domain.max_read_time_us.to_string());
    out.push_str(",\"data_units_total\":{");
    for (idx, (key, value)) in domain.data_units_total.iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        push_json_str(&mut out, key);
        out.push(':');
        out.push_str(&value.to_string());
    }
    out.push_str("}}");
    out
}

fn domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        DOMAIN_OOB_COMMAND => "oob_command",
        DOMAIN_SHIPS => "ships",
        _ => "unknown",
    }
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
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_usize_field(out: &mut String, name: &str, value: usize, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    out.push_str(&value.to_string());
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_opt_i32_field(out: &mut String, name: &str, value: Option<i32>, comma: bool) {
    out.push_str("  ");
    push_json_str(out, name);
    out.push_str(": ");
    match value {
        Some(value) => out.push_str(&value.to_string()),
        None => out.push_str("null"),
    }
    if comma {
        out.push(',');
    }
    out.push('\n');
}

fn push_json_str_array(out: &mut String, values: &[String]) {
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

    #[test]
    fn decodes_oob_command_fleet_and_task_force_members() {
        let payload = test_oob_command_data_payload(COMPACT_STATUS_DELTA_DATA);
        let mut summary = NavalOobGateSummary::default();
        summary
            .per_domain
            .entry(DOMAIN_OOB_COMMAND)
            .or_default()
            .data_record_count = 1;
        decode_oob_command_data_payload(0, &payload, &mut summary);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        let domain = summary.per_domain.get(&DOMAIN_OOB_COMMAND).unwrap();
        assert_eq!(domain.data_rows_total, 4);
        assert_eq!(domain.data_changed_rows_total, 4);
        assert_eq!(domain.status_flags_or, COMPACT_STATUS_DELTA_DATA);
        assert_eq!(domain.data_units_total.get("oob_fleet_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("oob_task_force_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("oob_member_task_force_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("oob_member_ship_rows"), Some(&2));
        assert_eq!(domain.data_units_total.get("oob_location_rows"), Some(&1));
    }

    #[test]
    fn decodes_ship_rows_with_owner_definition_and_delete() {
        let payload = test_ships_data_payload(COMPACT_STATUS_DELTA_DATA);
        let mut summary = NavalOobGateSummary::default();
        summary
            .per_domain
            .entry(DOMAIN_SHIPS)
            .or_default()
            .data_record_count = 1;
        decode_ships_data_payload(0, &payload, &mut summary);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        let domain = summary.per_domain.get(&DOMAIN_SHIPS).unwrap();
        assert_eq!(domain.data_rows_total, 2);
        assert_eq!(domain.data_changed_rows_total, 2);
        assert_eq!(domain.data_units_total.get("ship_task_force_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("ship_owner_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("ship_definition_rows"), Some(&1));
        assert_eq!(domain.data_units_total.get("ship_deleted_rows"), Some(&1));
    }

    #[test]
    fn decodes_compact_stream_and_passes_required_gate() {
        let mut raw = Vec::new();
        raw.extend_from_slice(&compact_record(
            1,
            60787539,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(44_000, 10, 10, 90, 90, COMPACT_STATUS_DELTA_DATA),
        ));
        raw.extend_from_slice(&compact_record(
            2,
            60787539,
            DOMAIN_OOB_COMMAND,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_oob_command_data_payload(COMPACT_STATUS_DELTA_DATA),
        ));
        raw.extend_from_slice(&compact_record(
            3,
            60787539,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            &test_domain_perf_payload(33_000, 2, 2, 2, 2, 0),
        ));
        raw.extend_from_slice(&compact_record(
            4,
            60787539,
            DOMAIN_SHIPS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            &test_ships_data_payload(COMPACT_STATUS_DELTA_DATA),
        ));
        raw.extend_from_slice(&compact_record(
            5,
            60787539,
            0,
            COMPACT_RECORD_TYPE_FRAME_END,
            &[],
        ));

        let summary = decode_stream(&raw);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert!(summary.ordering_errors.is_empty(), "{:?}", summary.ordering_errors);
        assert!(summary.gate_passed(), "{}", summary_json(&summary));
        assert_eq!(summary.record_count, 5);
        assert_eq!(summary.frame_end_record_count, 1);
        assert_eq!(summary.unique_game_hour_count, 1);
    }

    #[test]
    fn rejects_bad_ship_scale() {
        let mut payload = test_ships_data_payload(COMPACT_STATUS_DELTA_DATA);
        payload[32..36].copy_from_slice(&1u32.to_le_bytes());
        let mut summary = NavalOobGateSummary::default();
        decode_ships_data_payload(7, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("scale_raw 1")));
    }

    #[test]
    fn rejects_bad_oob_member_range() {
        let mut payload = test_oob_command_data_payload(COMPACT_STATUS_DELTA_DATA);
        let first_row_member_count_offset = 48 + 32;
        payload[first_row_member_count_offset..first_row_member_count_offset + 4]
            .copy_from_slice(&999u32.to_le_bytes());
        let mut summary = NavalOobGateSummary::default();
        decode_oob_command_data_payload(11, &payload, &mut summary);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("member range")));
    }

    fn test_domain_perf_payload(
        read_time_us: u64,
        row_count: u32,
        changed_rows: u32,
        unit_count: u32,
        changed_unit_count: u32,
        status_flags: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&read_time_us.to_le_bytes());
        payload.extend_from_slice(&row_count.to_le_bytes());
        payload.extend_from_slice(&changed_rows.to_le_bytes());
        payload.extend_from_slice(&unit_count.to_le_bytes());
        payload.extend_from_slice(&changed_unit_count.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload
    }

    fn compact_record(
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
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

    fn test_oob_command_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&4u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&22u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&6u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());

        for (
            kind,
            object_id,
            flags,
            parent_id,
            owner_tag_db_id,
            leader_id,
            leader_skill,
            member_first,
            row_member_count,
            logical_member_count,
            member_count_engine,
            missing_member_count,
            leader_trait_count,
            location_province_id,
            parent_status,
            member_status,
            owner_status,
            leader_status,
            leader_trait_status,
        ) in [
            (
                COMPACT_OOB_KIND_ARMY,
                17u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_OWNER
                    | COMPACT_OOB_ROW_FLAG_LEADER
                    | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT,
                301u32,
                47u32,
                9001u32,
                4u32,
                0u32,
                2u32,
                2u32,
                3u32,
                1u32,
                3u32,
                0u32,
                1u32,
                1u32,
                1u32,
                1u32,
                1u32,
            ),
            (
                COMPACT_OOB_KIND_ARMY_GROUP,
                301u32,
                COMPACT_OOB_ROW_FLAG_OWNER,
                0u32,
                47u32,
                0u32,
                0u32,
                2u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                1u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_FLEET,
                615u32,
                COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE,
                0u32,
                0u32,
                0u32,
                0u32,
                3u32,
                1u32,
                1u32,
                1u32,
                0u32,
                0u32,
                0u32,
                0u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
            (
                COMPACT_OOB_KIND_TASK_FORCE,
                617u32,
                COMPACT_OOB_ROW_FLAG_PARENT
                    | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                    | COMPACT_OOB_ROW_FLAG_LOCATION,
                615u32,
                0u32,
                0u32,
                0u32,
                4u32,
                2u32,
                2u32,
                2u32,
                0u32,
                0u32,
                12389u32,
                1u32,
                1u32,
                2u32,
                0u32,
                0u32,
            ),
        ] {
            payload.extend_from_slice(&kind.to_le_bytes());
            payload.extend_from_slice(&object_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&parent_id.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&leader_id.to_le_bytes());
            payload.extend_from_slice(&leader_skill.to_le_bytes());
            payload.extend_from_slice(&member_first.to_le_bytes());
            payload.extend_from_slice(&row_member_count.to_le_bytes());
            payload.extend_from_slice(&logical_member_count.to_le_bytes());
            payload.extend_from_slice(&member_count_engine.to_le_bytes());
            payload.extend_from_slice(&missing_member_count.to_le_bytes());
            payload.extend_from_slice(&leader_trait_count.to_le_bytes());
            payload.extend_from_slice(&parent_status.to_le_bytes());
            payload.extend_from_slice(&member_status.to_le_bytes());
            payload.extend_from_slice(&owner_status.to_le_bytes());
            payload.extend_from_slice(&leader_status.to_le_bytes());
            payload.extend_from_slice(&leader_trait_status.to_le_bytes());
            payload.extend_from_slice(&location_province_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        for member_id in [101u32, 102u32, 17u32, 617u32, 1001u32, 1002u32] {
            payload.extend_from_slice(&member_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
        }
        payload
    }

    fn test_ships_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&2u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&COMPACT_FIXED_SCALE_RAW.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());

        for (
            instance_id,
            flags,
            task_force_id,
            owner_tag_db_id,
            category_enum,
            strength_raw,
            organisation_raw,
            experience_raw,
            sunk_convoys,
            definition,
        ) in [
            (
                1001u32,
                COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                    | COMPACT_SHIP_ROW_FLAG_DEFINITION
                    | COMPACT_SHIP_ROW_FLAG_OWNER,
                617u32,
                47u32,
                3u32,
                10_000_000u32,
                9_000_000u32,
                500_000u32,
                2u32,
                "hull",
            ),
            (
                1002u32,
                COMPACT_SHIP_ROW_FLAG_DELETED,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                0u32,
                "",
            ),
        ] {
            payload.extend_from_slice(&instance_id.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload.extend_from_slice(&task_force_id.to_le_bytes());
            payload.extend_from_slice(&category_enum.to_le_bytes());
            payload.extend_from_slice(&strength_raw.to_le_bytes());
            payload.extend_from_slice(&organisation_raw.to_le_bytes());
            payload.extend_from_slice(&experience_raw.to_le_bytes());
            payload.extend_from_slice(&sunk_convoys.to_le_bytes());
            payload.extend_from_slice(&owner_tag_db_id.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            append_compact_string(&mut payload, definition);
        }
        payload
    }

    fn append_compact_string(payload: &mut Vec<u8>, value: &str) {
        payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }
}
