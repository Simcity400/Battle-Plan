use std::collections::{BTreeMap, BTreeSet};
#[cfg(not(test))]
use std::env;
#[cfg(not(test))]
use std::fs;

pub const DOMAIN_ID: u16 = 20;
pub const DOMAIN_NAME: &str = "divisions";
pub const STREAM_MAGIC: &[u8; 4] = b"H4CR";
pub const STREAM_HEADER_LEN: usize = 32;
pub const STREAM_VERSION: u16 = 1;
pub const RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const RECORD_TYPE_FRAME_END: u16 = 2;
pub const RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const PAYLOAD_HEADER_LEN: usize = 32;
pub const ROW_LEN: usize = 88;
pub const FIXED_SCALE_RAW: u32 = 100_000;
pub const DIVISION_TYPE: u32 = 4_713;

pub const STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const ROW_FLAG_MOVING: u32 = 1;
pub const ROW_FLAG_PROGRESS: u32 = 1 << 1;
pub const ROW_FLAG_MOVE_TARGET: u32 = 1 << 2;
pub const ROW_FLAG_MOVE_SOURCE: u32 = 1 << 3;
pub const ROW_FLAG_IN_COMBAT: u32 = 1 << 4;
pub const ROW_FLAG_OWNER: u32 = 1 << 5;
pub const ROW_FLAG_ARMY: u32 = 1 << 6;
pub const ROW_FLAG_EXPERIENCE: u32 = 1 << 7;
pub const ROW_FLAG_MAX_SUPPLY: u32 = 1 << 8;
pub const ROW_FLAG_CURRENT_SUPPLY: u32 = 1 << 9;
pub const ROW_FLAG_SUPPLY_GAIN: u32 = 1 << 10;
pub const ROW_FLAG_FUEL: u32 = 1 << 11;
pub const ROW_FLAG_DELETED: u32 = 1 << 12;
pub const ROW_FLAG_TEMPLATE: u32 = 1 << 13;
pub const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_MOVING
    | ROW_FLAG_PROGRESS
    | ROW_FLAG_MOVE_TARGET
    | ROW_FLAG_MOVE_SOURCE
    | ROW_FLAG_IN_COMBAT
    | ROW_FLAG_OWNER
    | ROW_FLAG_ARMY
    | ROW_FLAG_EXPERIENCE
    | ROW_FLAG_MAX_SUPPLY
    | ROW_FLAG_CURRENT_SUPPLY
    | ROW_FLAG_SUPPLY_GAIN
    | ROW_FLAG_FUEL
    | ROW_FLAG_DELETED
    | ROW_FLAG_TEMPLATE;

pub const MAX_PROVINCE_ID: u32 = 1_000_000;
pub const MAX_COMBAT_REF_COUNT: u32 = 1_000;
pub const MAX_ORGANISATION_RAW: u32 = 100_000_000;
pub const MAX_STRENGTH_RAW: u32 = 1_000_000_000;
pub const MAX_OWNER_TAG_DB_ID: u32 = 2_048;
pub const MAX_ARMY_ID: u32 = i32::MAX as u32;
pub const MAX_TEMPLATE_ID: u32 = i32::MAX as u32;
pub const MAX_SUPPLY_RAW: u32 = 100_000_000;
pub const MAX_SUPPLY_GAIN_RAW: u32 = 500_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DivisionStateHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub root_rows: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DivisionStateRow {
    pub div_id: u32,
    pub deleted: bool,
    pub moving: bool,
    pub location: u32,
    pub move_target_province: Option<u32>,
    pub movement_source_province: Option<u32>,
    pub raw_movement_progress: Option<u32>,
    pub progress_bucket: Option<u32>,
    pub raw_organisation: u32,
    pub organisation_bucket: u32,
    pub raw_strength: u32,
    pub strength_bucket: u32,
    pub combat_ref_count: u32,
    pub template_id: Option<u32>,
    pub owner_tag_db_id: Option<u32>,
    pub army_id: Option<u32>,
    pub raw_experience: Option<u32>,
    pub raw_max_supply: Option<u32>,
    pub raw_current_supply: Option<u32>,
    pub raw_supply_gain: Option<u32>,
    pub raw_fuel: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain20GateSummary {
    pub stream_bytes: usize,
    pub record_count: u64,
    pub frame_end_record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub status_flags_or: u32,
    pub domains_seen: BTreeSet<u16>,
    pub unique_hours: BTreeSet<i32>,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub max_payload_len: usize,
    pub max_read_time_us: u64,
    pub ordering_errors: Vec<String>,
    pub parse_errors: Vec<String>,
}

#[derive(Default)]
struct LifecycleTracker {
    initialized: bool,
    live_divisions: BTreeSet<u32>,
    baseline_rows: u64,
    created_rows: u64,
    updated_rows: u64,
    deleted_rows: u64,
}

impl Domain20GateSummary {
    pub fn merge_domain_payload(&mut self, other: Domain20GateSummary) {
        self.data_rows_total = self.data_rows_total.saturating_add(other.data_rows_total);
        self.data_changed_rows_total = self
            .data_changed_rows_total
            .saturating_add(other.data_changed_rows_total);
        self.status_flags_or |= other.status_flags_or;
        for (key, value) in other.data_units_total {
            add_unit(self, key, value as usize);
        }
        self.parse_errors.extend(other.parse_errors);
    }

    pub fn has_failures(&self) -> bool {
        !self.parse_errors.is_empty()
            || !self.ordering_errors.is_empty()
            || (self.status_flags_or
                & (STATUS_WORKER_PANIC
                    | STATUS_BRIDGE_UNAVAILABLE
                    | STATUS_SOURCE_UNAVAILABLE
                    | STATUS_PARTIAL_DATA))
                != 0
    }

    pub fn to_json(&self) -> String {
        let units = self
            .data_units_total
            .iter()
            .map(|(key, value)| format!("\"{}\":{}", json_escape(key), value))
            .collect::<Vec<_>>()
            .join(",");
        let domains = self
            .domains_seen
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let errors = self
            .parse_errors
            .iter()
            .chain(self.ordering_errors.iter())
            .map(|value| format!("\"{}\"", json_escape(value)))
            .collect::<Vec<_>>()
            .join(",");
        format!(
            concat!(
                "{{",
                "\"domain_id\":{},",
                "\"domain_name\":\"{}\",",
                "\"stream_bytes\":{},",
                "\"record_count\":{},",
                "\"frame_end_record_count\":{},",
                "\"domain_perf_record_count\":{},",
                "\"domain_data_record_count\":{},",
                "\"data_rows_total\":{},",
                "\"data_changed_rows_total\":{},",
                "\"status_flags_or\":{},",
                "\"has_failures\":{},",
                "\"domains_seen\":[{}],",
                "\"unique_game_hour_count\":{},",
                "\"max_payload_len\":{},",
                "\"max_read_time_us\":{},",
                "\"data_units_total\":{{{}}},",
                "\"errors\":[{}]",
                "}}"
            ),
            DOMAIN_ID,
            DOMAIN_NAME,
            self.stream_bytes,
            self.record_count,
            self.frame_end_record_count,
            self.domain_perf_record_count,
            self.domain_data_record_count,
            self.data_rows_total,
            self.data_changed_rows_total,
            self.status_flags_or,
            self.has_failures(),
            domains,
            self.unique_hours.len(),
            self.max_payload_len,
            self.max_read_time_us,
            units,
            errors
        )
    }
}

pub fn encode_payload(header: &DivisionStateHeader, rows: &[DivisionStateRow]) -> Vec<u8> {
    let mut payload =
        Vec::with_capacity(PAYLOAD_HEADER_LEN.saturating_add(rows.len().saturating_mul(ROW_LEN)));
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&header.root_rows.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        let flags = row_flags(row);
        payload.extend_from_slice(&row.div_id.to_le_bytes());
        payload.extend_from_slice(&DIVISION_TYPE.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.location.to_le_bytes());
        payload.extend_from_slice(&row.move_target_province.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.movement_source_province.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_movement_progress.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.progress_bucket.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_organisation.to_le_bytes());
        payload.extend_from_slice(&row.organisation_bucket.to_le_bytes());
        payload.extend_from_slice(&row.raw_strength.to_le_bytes());
        payload.extend_from_slice(&row.strength_bucket.to_le_bytes());
        payload.extend_from_slice(&row.combat_ref_count.to_le_bytes());
        payload.extend_from_slice(&row.template_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.army_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_experience.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_max_supply.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_current_supply.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_supply_gain.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.raw_fuel.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain20GateSummary {
    let mut summary = Domain20GateSummary::default();
    if payload.len() < PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions payload is {} byte(s), expected at least {PAYLOAD_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let reserved0 = read_u32_le(payload, 28);
    summary.status_flags_or |= status_flags;
    if scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions scale {scale_raw} != expected {FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions header reserved field is nonzero"
        ));
    }
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }
    let delta_data = (status_flags & STATUS_DELTA_DATA) != 0;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions delta changed_rows {changed_rows} != row_count {row_count}"
        ));
    }
    if !delta_data && row_count > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row_count {row_count} > root_rows {root_rows}"
        ));
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "division_rows", row_count);
    add_unit(&mut summary, "division_root_rows", root_rows);

    let mut cursor = PAYLOAD_HEADER_LEN;
    let mut seen_divisions = BTreeSet::new();
    let mut deleted_rows = 0usize;
    let mut moving_rows = 0usize;
    let mut in_combat_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut army_rows = 0usize;
    let mut template_rows = 0usize;
    let mut org_strength_rows = 0usize;
    let mut experience_rows = 0usize;
    let mut max_supply_rows = 0usize;
    let mut current_supply_rows = 0usize;
    let mut supply_gain_rows = 0usize;
    let mut fuel_rows = 0usize;
    let mut delta_upsert_rows = 0usize;
    let mut reserved_tail_zero_rows = 0usize;

    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let div_type = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let location = read_u32_le(payload, cursor + 12);
        let move_target = read_u32_le(payload, cursor + 16);
        let move_source = read_u32_le(payload, cursor + 20);
        let movement_progress = read_u32_le(payload, cursor + 24);
        let progress_bucket = read_u32_le(payload, cursor + 28);
        let organisation_raw = read_u32_le(payload, cursor + 32);
        let organisation_bucket = read_u32_le(payload, cursor + 36);
        let strength_raw = read_u32_le(payload, cursor + 40);
        let strength_bucket = read_u32_le(payload, cursor + 44);
        let combat_ref_count = read_u32_le(payload, cursor + 48);
        let template_id = read_u32_le(payload, cursor + 52);
        let owner_tag_db_id = read_u32_le(payload, cursor + 56);
        let army_id = read_u32_le(payload, cursor + 60);
        let experience_raw = read_u32_le(payload, cursor + 64);
        let max_supply_raw = read_u32_le(payload, cursor + 68);
        let current_supply_raw = read_u32_le(payload, cursor + 72);
        let supply_gain_raw = read_u32_le(payload, cursor + 76);
        let fuel_raw = read_u32_le(payload, cursor + 80);
        let reserved1 = read_u32_le(payload, cursor + 84);
        cursor += ROW_LEN;
        reserved_tail_zero_rows = reserved_tail_zero_rows.saturating_add((reserved1 == 0) as usize);

        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert(div_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} duplicates div_id {div_id}"
            ));
        }
        if div_type != DIVISION_TYPE {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has div_type {div_type}"
            ));
        }
        if flags & !KNOWN_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has unknown flags {flags}"
            ));
        }

        let deleted = has_flag(flags, ROW_FLAG_DELETED);
        let moving = has_flag(flags, ROW_FLAG_MOVING);
        let has_progress = has_flag(flags, ROW_FLAG_PROGRESS);
        let has_target = has_flag(flags, ROW_FLAG_MOVE_TARGET);
        let has_source = has_flag(flags, ROW_FLAG_MOVE_SOURCE);
        let in_combat = has_flag(flags, ROW_FLAG_IN_COMBAT);
        let has_owner = has_flag(flags, ROW_FLAG_OWNER);
        let has_army = has_flag(flags, ROW_FLAG_ARMY);
        let has_experience = has_flag(flags, ROW_FLAG_EXPERIENCE);
        let has_max_supply = has_flag(flags, ROW_FLAG_MAX_SUPPLY);
        let has_current_supply = has_flag(flags, ROW_FLAG_CURRENT_SUPPLY);
        let has_supply_gain = has_flag(flags, ROW_FLAG_SUPPLY_GAIN);
        let has_fuel = has_flag(flags, ROW_FLAG_FUEL);
        let has_template = has_flag(flags, ROW_FLAG_TEMPLATE);

        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} has non-delete flags"
                ));
            }
            if !zero_filled_deleted_row(
                location,
                move_target,
                move_source,
                movement_progress,
                progress_bucket,
                organisation_raw,
                organisation_bucket,
                strength_raw,
                strength_bucket,
                combat_ref_count,
                template_id,
                owner_tag_db_id,
                army_id,
                experience_raw,
                max_supply_raw,
                current_supply_raw,
                supply_gain_raw,
                fuel_raw,
                reserved1,
            ) {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} is not zero-filled"
                ));
            }
            continue;
        }

        if delta_data {
            delta_upsert_rows = delta_upsert_rows.saturating_add(1);
        }
        org_strength_rows = org_strength_rows.saturating_add(1);
        moving_rows = moving_rows.saturating_add(moving as usize);
        in_combat_rows = in_combat_rows.saturating_add(in_combat as usize);
        owner_rows = owner_rows.saturating_add(has_owner as usize);
        army_rows = army_rows.saturating_add(has_army as usize);
        template_rows = template_rows.saturating_add(has_template as usize);
        experience_rows = experience_rows.saturating_add(has_experience as usize);
        max_supply_rows = max_supply_rows.saturating_add(has_max_supply as usize);
        current_supply_rows = current_supply_rows.saturating_add(has_current_supply as usize);
        supply_gain_rows = supply_gain_rows.saturating_add(has_supply_gain as usize);
        fuel_rows = fuel_rows.saturating_add(has_fuel as usize);

        if !valid_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid location {location}"
            ));
        }
        validate_optional_province(
            &mut summary,
            offset,
            row_index,
            has_target,
            move_target,
            "move_target",
        );
        validate_optional_province(
            &mut summary,
            offset,
            row_index,
            has_source,
            move_source,
            "move_source",
        );
        if !moving && (has_progress || has_target || has_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has movement fields while not moving"
            ));
        }
        if has_progress && movement_progress == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero movement progress"
            ));
        }
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_progress,
            progress_bucket,
            0,
            u32::MAX,
            "progress_bucket",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            organisation_raw,
            0,
            MAX_ORGANISATION_RAW,
            "organisation_raw",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            strength_raw,
            0,
            MAX_STRENGTH_RAW,
            "strength_raw",
        );
        validate_required_u32(
            &mut summary,
            offset,
            row_index,
            combat_ref_count,
            0,
            MAX_COMBAT_REF_COUNT,
            "combat_ref_count",
        );
        if in_combat != (combat_ref_count > 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} combat flag does not match combat_ref_count"
            ));
        }
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_template,
            template_id,
            1,
            MAX_TEMPLATE_ID,
            "template_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_owner,
            owner_tag_db_id,
            1,
            MAX_OWNER_TAG_DB_ID,
            "owner_tag_db_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_army,
            army_id,
            1,
            MAX_ARMY_ID,
            "army_id",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_experience,
            experience_raw,
            0,
            u32::MAX,
            "experience_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_max_supply,
            max_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "max_supply_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_current_supply,
            current_supply_raw,
            0,
            MAX_SUPPLY_RAW,
            "current_supply_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_supply_gain,
            supply_gain_raw,
            0,
            MAX_SUPPLY_GAIN_RAW,
            "supply_gain_raw",
        );
        validate_optional_u32(
            &mut summary,
            offset,
            row_index,
            has_fuel,
            fuel_raw,
            0,
            u32::MAX,
            "fuel_raw",
        );
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} reserved field is nonzero"
            ));
        }
    }

    add_unit(&mut summary, "division_deleted_rows", deleted_rows);
    add_unit(&mut summary, "division_moving_rows", moving_rows);
    add_unit(&mut summary, "division_in_combat_rows", in_combat_rows);
    add_unit(&mut summary, "division_owner_rows", owner_rows);
    add_unit(&mut summary, "division_army_rows", army_rows);
    add_unit(&mut summary, "division_template_rows", template_rows);
    add_unit(
        &mut summary,
        "division_org_strength_rows",
        org_strength_rows,
    );
    add_unit(&mut summary, "division_experience_rows", experience_rows);
    add_unit(&mut summary, "division_max_supply_rows", max_supply_rows);
    add_unit(
        &mut summary,
        "division_current_supply_rows",
        current_supply_rows,
    );
    add_unit(&mut summary, "division_supply_gain_rows", supply_gain_rows);
    add_unit(&mut summary, "division_fuel_rows", fuel_rows);
    add_unit(
        &mut summary,
        "division_delta_upsert_rows",
        delta_upsert_rows,
    );
    add_unit(
        &mut summary,
        "division_reserved_tail_zero_rows",
        reserved_tail_zero_rows,
    );
    summary
}

pub fn decode_stream(raw: &[u8]) -> Domain20GateSummary {
    let mut summary = Domain20GateSummary {
        stream_bytes: raw.len(),
        ..Domain20GateSummary::default()
    };
    let mut offset = 0usize;
    let mut previous_sequence: Option<u64> = None;
    let mut lifecycle = LifecycleTracker::default();
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < STREAM_HEADER_LEN {
            summary
                .parse_errors
                .push(format!("offset {offset}: truncated stream header"));
            break;
        }
        let header = &raw[offset..offset + STREAM_HEADER_LEN];
        if &header[0..4] != STREAM_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid stream magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        if version != STREAM_VERSION || header_len != STREAM_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported stream header version {version} len {header_len}"
            ));
            break;
        }
        let record_len = STREAM_HEADER_LEN.saturating_add(payload_len);
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining bytes {remaining}"
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
        summary.record_count = summary.record_count.saturating_add(1);
        summary.domains_seen.insert(domain_id);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);

        let payload = &raw[offset + STREAM_HEADER_LEN..offset + record_len];
        match record_type {
            RECORD_TYPE_FRAME_END => {
                summary.frame_end_record_count = summary.frame_end_record_count.saturating_add(1);
                summary.unique_hours.insert(game_hour_raw);
            }
            RECORD_TYPE_DOMAIN_PERF if domain_id == DOMAIN_ID => {
                summary.domain_perf_record_count =
                    summary.domain_perf_record_count.saturating_add(1);
                decode_domain_perf(offset, payload, &mut summary);
            }
            RECORD_TYPE_DOMAIN_DATA if domain_id == DOMAIN_ID => {
                summary.domain_data_record_count =
                    summary.domain_data_record_count.saturating_add(1);
                let payload_summary = decode_payload(offset, payload);
                summary.merge_domain_payload(payload_summary);
                update_lifecycle_from_payload(offset, payload, &mut lifecycle, &mut summary);
            }
            _ => {}
        }
        offset = offset.saturating_add(record_len);
    }
    add_unit(
        &mut summary,
        "division_lifecycle_baseline_rows",
        lifecycle.baseline_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_created_rows",
        lifecycle.created_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_updated_rows",
        lifecycle.updated_rows.min(usize::MAX as u64) as usize,
    );
    add_unit(
        &mut summary,
        "division_lifecycle_deleted_rows",
        lifecycle.deleted_rows.min(usize::MAX as u64) as usize,
    );
    summary
}

fn update_lifecycle_from_payload(
    offset: usize,
    payload: &[u8],
    tracker: &mut LifecycleTracker,
    summary: &mut Domain20GateSummary,
) {
    if payload.len() < PAYLOAD_HEADER_LEN {
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let expected_len = PAYLOAD_HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        return;
    }
    let mut cursor = PAYLOAD_HEADER_LEN;
    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 8);
        cursor += ROW_LEN;
        if div_id == 0 {
            continue;
        }
        let deleted = has_flag(flags, ROW_FLAG_DELETED);
        if !tracker.initialized {
            if deleted {
                summary.parse_errors.push(format!(
                    "offset {offset}: baseline divisions row {row_index} is a delete row"
                ));
            } else if tracker.live_divisions.insert(div_id) {
                tracker.baseline_rows = tracker.baseline_rows.saturating_add(1);
            }
            continue;
        }
        if deleted {
            if tracker.live_divisions.remove(&div_id) {
                tracker.deleted_rows = tracker.deleted_rows.saturating_add(1);
            } else {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} deletes unknown div_id {div_id}"
                ));
            }
        } else if tracker.live_divisions.insert(div_id) {
            tracker.created_rows = tracker.created_rows.saturating_add(1);
        } else {
            tracker.updated_rows = tracker.updated_rows.saturating_add(1);
        }
    }
    tracker.initialized = true;
}

fn decode_domain_perf(offset: usize, payload: &[u8], summary: &mut Domain20GateSummary) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: domain perf payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let read_time_us = read_u64_le(payload, 8);
    let status_flags = read_u32_le(payload, 32);
    summary.status_flags_or |= status_flags;
    summary.max_read_time_us = summary.max_read_time_us.max(read_time_us);
}

fn row_flags(row: &DivisionStateRow) -> u32 {
    if row.deleted {
        return ROW_FLAG_DELETED;
    }
    let mut flags = 0u32;
    if row.moving {
        flags |= ROW_FLAG_MOVING;
    }
    if row.raw_movement_progress.is_some() {
        flags |= ROW_FLAG_PROGRESS;
    }
    if row.move_target_province.is_some() {
        flags |= ROW_FLAG_MOVE_TARGET;
    }
    if row.movement_source_province.is_some() {
        flags |= ROW_FLAG_MOVE_SOURCE;
    }
    if row.combat_ref_count > 0 {
        flags |= ROW_FLAG_IN_COMBAT;
    }
    if row.owner_tag_db_id.is_some() {
        flags |= ROW_FLAG_OWNER;
    }
    if row.army_id.is_some() {
        flags |= ROW_FLAG_ARMY;
    }
    if row.raw_experience.is_some() {
        flags |= ROW_FLAG_EXPERIENCE;
    }
    if row.raw_max_supply.is_some() {
        flags |= ROW_FLAG_MAX_SUPPLY;
    }
    if row.raw_current_supply.is_some() {
        flags |= ROW_FLAG_CURRENT_SUPPLY;
    }
    if row.raw_supply_gain.is_some() {
        flags |= ROW_FLAG_SUPPLY_GAIN;
    }
    if row.raw_fuel.is_some() {
        flags |= ROW_FLAG_FUEL;
    }
    if row.template_id.is_some() {
        flags |= ROW_FLAG_TEMPLATE;
    }
    flags
}

fn zero_filled_deleted_row(
    values: u32,
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    e: u32,
    f: u32,
    g: u32,
    h: u32,
    i: u32,
    j: u32,
    k: u32,
    l: u32,
    m: u32,
    n: u32,
    o: u32,
    p: u32,
    q: u32,
    r: u32,
) -> bool {
    [values, a, b, c, d, e, f, g, h, i, j, k, l, m, n, o, p, q, r]
        .into_iter()
        .all(|value| value == 0)
}

fn validate_required_u32(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    value: u32,
    min_value: u32,
    max_value: u32,
    label: &str,
) {
    if value < min_value || value > max_value {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} {label} {value} outside {min_value}..{max_value}"
        ));
    }
}

fn validate_optional_u32(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    has_value: bool,
    value: u32,
    min_value: u32,
    max_value: u32,
    label: &str,
) {
    if has_value {
        validate_required_u32(
            summary, offset, row_index, value, min_value, max_value, label,
        );
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {label} without flag"
        ));
    }
}

fn validate_optional_province(
    summary: &mut Domain20GateSummary,
    offset: usize,
    row_index: usize,
    has_value: bool,
    value: u32,
    label: &str,
) {
    if has_value {
        if !valid_province_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid {label} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {label} without flag"
        ));
    }
}

fn valid_province_id(value: u32) -> bool {
    value > 0 && value <= MAX_PROVINCE_ID
}

fn has_flag(flags: u32, flag: u32) -> bool {
    (flags & flag) != 0
}

fn add_unit(summary: &mut Domain20GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
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

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

#[cfg(not(test))]
fn main() {
    let args = env::args().collect::<Vec<_>>();
    if args.len() < 2 || args.len() > 3 {
        eprintln!("usage: domain20_compact <collector_stream.bin> [proof_summary.json]");
        std::process::exit(2);
    }
    let raw = fs::read(&args[1]).unwrap_or_else(|err| {
        eprintln!("failed to read {}: {err}", args[1]);
        std::process::exit(2);
    });
    let summary = decode_stream(&raw);
    let json = summary.to_json();
    if let Some(out_path) = args.get(2) {
        fs::write(out_path, json.as_bytes()).unwrap_or_else(|err| {
            eprintln!("failed to write {out_path}: {err}");
            std::process::exit(2);
        });
    } else {
        println!("{json}");
    }
    if summary.has_failures() || summary.domain_data_record_count == 0 {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_and_decodes_full_state_rows() {
        let rows = vec![
            DivisionStateRow {
                div_id: 101,
                moving: true,
                location: 12_345,
                move_target_province: Some(12_345),
                movement_source_province: Some(12_300),
                raw_movement_progress: Some(42_000),
                progress_bucket: Some(42),
                raw_organisation: 8_500_000,
                organisation_bucket: 8_500,
                raw_strength: 96_250_000,
                strength_bucket: 96_250,
                combat_ref_count: 2,
                template_id: Some(3_101),
                owner_tag_db_id: Some(1),
                army_id: Some(77),
                raw_experience: Some(1_234_000),
                raw_max_supply: Some(2_000_000),
                raw_current_supply: Some(1_750_000),
                raw_supply_gain: Some(10_000),
                raw_fuel: Some(88_000),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 102,
                location: 99,
                raw_organisation: 10_000_000,
                organisation_bucket: 10_000,
                raw_strength: 100_000_000,
                strength_bucket: 100_000,
                owner_tag_db_id: Some(2),
                template_id: Some(4_001),
                raw_experience: Some(0),
                raw_current_supply: Some(2_000_000),
                ..DivisionStateRow::default()
            },
        ];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 250,
            status_flags: 0,
            changed_rows: rows.len() as u32,
            root_rows: rows.len() as u32,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(32, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 2);
        assert_eq!(
            summary
                .data_units_total
                .get("division_moving_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_in_combat_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_current_supply_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary.data_units_total.get("division_fuel_rows").copied(),
            Some(1)
        );
    }

    #[test]
    fn accepts_zero_filled_deleted_rows() {
        let rows = vec![DivisionStateRow {
            div_id: 404,
            deleted: true,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 250,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            root_rows: 10,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_deleted_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_movement_fields_when_not_moving() {
        let rows = vec![DivisionStateRow {
            div_id: 5,
            location: 55,
            move_target_province: Some(56),
            raw_organisation: 1,
            raw_strength: 1,
            ..DivisionStateRow::default()
        }];
        let header = DivisionStateHeader {
            elapsed_wall_ms: 0,
            status_flags: 0,
            changed_rows: 1,
            root_rows: 1,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("movement fields while not moving")));
    }

    #[test]
    fn decodes_stream_with_domain20_data_record() {
        let rows = vec![DivisionStateRow {
            div_id: 7,
            moving: true,
            location: 70,
            move_target_province: Some(70),
            raw_movement_progress: Some(1_000),
            progress_bucket: Some(1),
            raw_organisation: 5_000_000,
            organisation_bucket: 5_000,
            raw_strength: 75_000_000,
            strength_bucket: 75_000,
            owner_tag_db_id: Some(1),
            template_id: Some(9),
            raw_current_supply: Some(100_000),
            ..DivisionStateRow::default()
        }];
        let payload = encode_payload(
            &DivisionStateHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 1,
                root_rows: 1,
            },
            &rows,
        );
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            123,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &payload,
        );
        append_record(&mut stream, 2, 123, 1000, RECORD_TYPE_FRAME_END, &[0; 32]);
        let summary = decode_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.record_count, 2);
        assert_eq!(summary.domain_data_record_count, 1);
        assert_eq!(summary.frame_end_record_count, 1);
        assert_eq!(summary.unique_hours.len(), 1);
    }

    #[test]
    fn derives_lifecycle_from_ordered_domain20_rows() {
        let baseline = vec![
            DivisionStateRow {
                div_id: 7,
                location: 70,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 8,
                location: 80,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
        ];
        let delta = vec![
            DivisionStateRow {
                div_id: 7,
                moving: true,
                location: 71,
                move_target_province: Some(71),
                raw_movement_progress: Some(1_000),
                progress_bucket: Some(1),
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(9),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 9,
                location: 90,
                raw_organisation: 5_000_000,
                organisation_bucket: 5_000,
                raw_strength: 75_000_000,
                strength_bucket: 75_000,
                owner_tag_db_id: Some(1),
                template_id: Some(10),
                ..DivisionStateRow::default()
            },
            DivisionStateRow {
                div_id: 8,
                deleted: true,
                ..DivisionStateRow::default()
            },
        ];
        let mut stream = Vec::new();
        append_record(
            &mut stream,
            1,
            100,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(
                &DivisionStateHeader {
                    elapsed_wall_ms: 0,
                    status_flags: 0,
                    changed_rows: 2,
                    root_rows: 2,
                },
                &baseline,
            ),
        );
        append_record(
            &mut stream,
            2,
            101,
            DOMAIN_ID,
            RECORD_TYPE_DOMAIN_DATA,
            &encode_payload(
                &DivisionStateHeader {
                    elapsed_wall_ms: 100,
                    status_flags: STATUS_DELTA_DATA,
                    changed_rows: 3,
                    root_rows: 3,
                },
                &delta,
            ),
        );
        let summary = decode_stream(&stream);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_baseline_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_updated_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_created_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_lifecycle_deleted_rows")
                .copied(),
            Some(1)
        );
    }

    fn append_record(
        out: &mut Vec<u8>,
        sequence: u64,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload: &[u8],
    ) {
        out.extend_from_slice(STREAM_MAGIC);
        out.extend_from_slice(&STREAM_VERSION.to_le_bytes());
        out.extend_from_slice(&(STREAM_HEADER_LEN as u16).to_le_bytes());
        out.extend_from_slice(&sequence.to_le_bytes());
        out.extend_from_slice(&game_hour_raw.to_le_bytes());
        out.extend_from_slice(&domain_id.to_le_bytes());
        out.extend_from_slice(&record_type.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(payload);
    }
}
