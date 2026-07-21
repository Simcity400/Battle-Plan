use std::collections::{BTreeMap, BTreeSet};
use std::convert::TryInto;

pub const DOMAIN_ID: u16 = 44;
pub const DOMAIN_NAME: &str = "supply_logistics_attrition_live";
pub const HEADER_LEN_LEGACY: usize = 48;
pub const HEADER_LEN: usize = 52;
pub const ROW_LEN: usize = 40;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_COMPACT_HEALTH_OK: u32 = 0;

pub const ROW_KIND_PROVINCE_LOGISTICS: u32 = 1;
pub const ROW_KIND_DIVISION_SUPPLY: u32 = 2;
pub const ROW_KIND_COMPUTED_BADGE: u32 = 3;
pub const ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 = 4;
pub const ROW_KIND_COUNTRY_DISRUPTED_SUPPLY: u32 = 5;
// Keep this lane-local, explicitly non-production candidate on a disjoint id.
pub const ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE: u32 = 6;

pub const ROW_FLAG_RAIL: u32 = 1;
pub const ROW_FLAG_SUPPLY_NODE: u32 = 1 << 1;
pub const ROW_FLAG_NAVAL_SUPPLY_HUB: u32 = 1 << 2;
pub const ROW_FLAG_INFRASTRUCTURE: u32 = 1 << 3;
pub const ROW_FLAG_LIVE_BUILDING_VECTOR: u32 = 1 << 4;
pub const ROW_FLAG_DIVISION_CURRENT_SUPPLY: u32 = 1 << 5;
pub const ROW_FLAG_DIVISION_MAX_SUPPLY: u32 = 1 << 6;
pub const ROW_FLAG_DIVISION_SUPPLY_GAIN: u32 = 1 << 7;
pub const ROW_FLAG_DIVISION_OWNER: u32 = 1 << 8;
pub const ROW_FLAG_EXACT_SUPPLY_WALL: u32 = 1 << 9;
pub const ROW_FLAG_ATTRITION_WALL: u32 = 1 << 10;
pub const ROW_FLAG_PROXY_VALUE: u32 = 1 << 11;
pub const ROW_FLAG_DIVISION_SUPPLY_DEFICIT: u32 = 1 << 12;
pub const ROW_FLAG_DIVISION_FUEL: u32 = 1 << 13;
pub const ROW_FLAG_MOTORIZATION_WALL: u32 = 1 << 14;
pub const ROW_FLAG_CONVOY_PRESSURE_WALL: u32 = 1 << 15;
pub const ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED: u32 = 1 << 16;
pub const ROW_FLAG_CSUPPLY_EFFICIENCY: u32 = 1 << 17;
pub const ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 = 1 << 18;
pub const ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT: u32 = 1 << 19;
pub const ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: u32 = 1 << 20;
pub const ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE: u32 = 1 << 21;
pub const ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT: u32 = 1 << 22;
pub const ROW_ID_DELETED: u32 = 1 << 31;

pub const BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT: i32 = 1;
pub const BADGE_ATTRITION_RUNTIME_COMPUTED: i32 = 2;
pub const BADGE_MOTORIZATION_SETTINGS_UNPINNED: i32 = 3;
pub const BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED: i32 = 4;
pub const BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE: i32 = 5;
pub const BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: i32 = 6;

pub const MAX_PROVINCE_ID: u32 = 13_413;
pub const MAX_STATE_ID: u32 = 1_081;
pub const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_DIVISION_ID: u32 = i32::MAX as u32;
pub const MAX_SUPPLY_RAW: i32 = 100_000_000;
pub const MAX_SUPPLY_GAIN_RAW: i32 = 500_000;
pub const MAX_DIVISION_FUEL_RAW: i32 = i32::MAX;
pub const MAX_BUILDING_LEVEL: i32 = 1_000;
pub const MAX_COUNTRY_DISRUPTED_SUPPLY_RAW: i32 = FIXED_SCALE_RAW as i32;

const VALID_ROW_FLAGS: u32 = ROW_FLAG_RAIL
    | ROW_FLAG_SUPPLY_NODE
    | ROW_FLAG_NAVAL_SUPPLY_HUB
    | ROW_FLAG_INFRASTRUCTURE
    | ROW_FLAG_LIVE_BUILDING_VECTOR
    | ROW_FLAG_DIVISION_CURRENT_SUPPLY
    | ROW_FLAG_DIVISION_MAX_SUPPLY
    | ROW_FLAG_DIVISION_SUPPLY_GAIN
    | ROW_FLAG_DIVISION_OWNER
    | ROW_FLAG_EXACT_SUPPLY_WALL
    | ROW_FLAG_ATTRITION_WALL
    | ROW_FLAG_PROXY_VALUE
    | ROW_FLAG_DIVISION_SUPPLY_DEFICIT
    | ROW_FLAG_DIVISION_FUEL
    | ROW_FLAG_MOTORIZATION_WALL
    | ROW_FLAG_CONVOY_PRESSURE_WALL
    | ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED
    | ROW_FLAG_CSUPPLY_EFFICIENCY
    | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY
    | ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT
    | ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE
    | ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE
    | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupplyLogisticsHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub province_rows: u32,
    pub division_rows: u32,
    pub badge_rows: u32,
    pub observed_province_roots: u32,
    pub observed_division_roots: u32,
    pub csupply_input_rows: u32,
    pub pressure_candidate_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SupplyLogisticsRow {
    pub row_kind: u32,
    pub entity_id: u32,
    pub join_id: u32,
    pub owner_tag_db_id: u32,
    pub flags: u32,
    pub value_a_raw: i32,
    pub value_b_raw: i32,
    pub value_c_raw: i32,
    pub value_d_raw: i32,
    pub aux_raw: i32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain44GateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub province_rows_total: u64,
    pub division_rows_total: u64,
    pub badge_rows_total: u64,
    pub csupply_input_rows_total: u64,
    pub country_disrupted_supply_rows_total: u64,
    pub country_disrupted_supply_tombstones_total: u64,
    pub country_disrupted_source_available: bool,
    pub pressure_candidate_rows_total: u64,
    pub observed_province_roots: u64,
    pub observed_division_roots: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(header: &SupplyLogisticsHeader, rows: &[SupplyLogisticsRow]) -> Vec<u8> {
    let mut payload =
        Vec::with_capacity(HEADER_LEN.saturating_add(rows.len().saturating_mul(ROW_LEN)));
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&header.province_rows.to_le_bytes());
    payload.extend_from_slice(&header.division_rows.to_le_bytes());
    payload.extend_from_slice(&header.badge_rows.to_le_bytes());
    payload.extend_from_slice(&header.observed_province_roots.to_le_bytes());
    payload.extend_from_slice(&header.observed_division_roots.to_le_bytes());
    payload.extend_from_slice(&header.csupply_input_rows.to_le_bytes());
    payload.extend_from_slice(&header.pressure_candidate_rows.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.row_kind.to_le_bytes());
        payload.extend_from_slice(&row.entity_id.to_le_bytes());
        payload.extend_from_slice(&row.join_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.value_a_raw.to_le_bytes());
        payload.extend_from_slice(&row.value_b_raw.to_le_bytes());
        payload.extend_from_slice(&row.value_c_raw.to_le_bytes());
        payload.extend_from_slice(&row.value_d_raw.to_le_bytes());
        payload.extend_from_slice(&row.aux_raw.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain44GateSummary {
    let mut summary = Domain44GateSummary::default();
    if payload.len() < HEADER_LEN_LEGACY {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics data payload is {} byte(s), expected at least {HEADER_LEN_LEGACY}",
            payload.len()
        ));
        return summary;
    }
    let header_len =
        if payload.len() >= HEADER_LEN && payload.len().saturating_sub(HEADER_LEN) % ROW_LEN == 0 {
            HEADER_LEN
        } else {
            HEADER_LEN_LEGACY
        };

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let province_rows = read_u32_le(payload, 24) as usize;
    let division_rows = read_u32_le(payload, 28) as usize;
    let badge_rows = read_u32_le(payload, 32) as usize;
    let observed_province_roots = read_u32_le(payload, 36) as usize;
    let observed_division_roots = read_u32_le(payload, 40) as usize;
    let csupply_input_rows = read_u32_le(payload, 44) as usize;
    let pressure_candidate_rows = if header_len >= HEADER_LEN {
        read_u32_le(payload, 48) as usize
    } else {
        0
    };
    summary.status_flags_or |= status_flags;
    summary.observed_province_roots = observed_province_roots as u64;
    summary.observed_division_roots = observed_division_roots as u64;

    if scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics scale_raw {scale_raw} != {FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if province_rows + division_rows + badge_rows + csupply_input_rows + pressure_candidate_rows
        != row_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row counters {}+{}+{}+{}+{} != row_count {row_count}",
            province_rows, division_rows, badge_rows, csupply_input_rows, pressure_candidate_rows
        ));
    }
    let expected_len = header_len.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }

    let mut decoded_province_rows = 0usize;
    let mut decoded_division_rows = 0usize;
    let mut decoded_badge_rows = 0usize;
    let mut decoded_csupply_input_rows = 0usize;
    let mut decoded_country_disrupted_supply_rows = 0usize;
    let mut decoded_country_disrupted_supply_tombstones = 0usize;
    let mut decoded_pressure_candidate_rows = 0usize;
    let mut seen_provinces = BTreeSet::new();
    let mut seen_divisions = BTreeSet::new();
    let mut seen_csupply_input_provinces = BTreeSet::new();
    let mut seen_country_disrupted_supply = BTreeSet::new();
    let mut seen_pressure_candidates = BTreeSet::new();
    let mut has_exact_supply_wall = false;
    let mut has_attrition_wall = false;
    let mut has_motorization_wall = false;
    let mut has_convoy_pressure_wall = false;
    let mut has_csupply_candidate_rejected = false;
    let mut has_country_disrupted_source_available = false;
    let mut cursor = header_len;
    for row_index in 0..row_count {
        let row_kind = read_u32_le(payload, cursor);
        let entity_id = read_u32_le(payload, cursor + 4);
        let join_id = read_u32_le(payload, cursor + 8);
        let owner_tag_db_id = read_u32_le(payload, cursor + 12);
        let flags = read_u32_le(payload, cursor + 16);
        let value_a_raw = read_i32_le(payload, cursor + 20);
        let value_b_raw = read_i32_le(payload, cursor + 24);
        let value_c_raw = read_i32_le(payload, cursor + 28);
        let value_d_raw = read_i32_le(payload, cursor + 32);
        let aux_raw = read_i32_le(payload, cursor + 36);
        cursor += ROW_LEN;

        if flags & !VALID_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: supply logistics row {row_index} has unknown flags {flags}"
            ));
        }
        match row_kind {
            ROW_KIND_PROVINCE_LOGISTICS => {
                decoded_province_rows = decoded_province_rows.saturating_add(1);
                if entity_id == 0 || entity_id > MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has invalid province_id {entity_id}"
                    ));
                } else if !seen_provinces.insert(entity_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} duplicates province_id {entity_id}"
                    ));
                }
                if join_id == 0 || join_id > MAX_STATE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has invalid state_id {join_id}"
                    ));
                }
                if owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has nonzero owner_tag_db_id"
                    ));
                }
                if flags & ROW_FLAG_LIVE_BUILDING_VECTOR == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} missing live building vector flag"
                    ));
                }
                for (label, value, flag) in [
                    ("rail", value_a_raw, ROW_FLAG_RAIL),
                    ("supply_node", value_b_raw, ROW_FLAG_SUPPLY_NODE),
                    ("naval_supply_hub", value_c_raw, ROW_FLAG_NAVAL_SUPPLY_HUB),
                    ("infrastructure", value_d_raw, ROW_FLAG_INFRASTRUCTURE),
                ] {
                    validate_nonnegative_value(
                        offset,
                        &mut summary,
                        row_index,
                        label,
                        value,
                        MAX_BUILDING_LEVEL,
                    );
                    validate_presence_flag(
                        offset,
                        &mut summary,
                        row_index,
                        label,
                        value,
                        flags,
                        flag,
                    );
                }
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "building_count",
                    aux_raw,
                    MAX_BUILDING_LEVEL,
                );
                add_unit(&mut summary, "province_logistics_rows", 1);
            }
            ROW_KIND_DIVISION_SUPPLY => {
                decoded_division_rows = decoded_division_rows.saturating_add(1);
                if entity_id == 0 || entity_id > MAX_DIVISION_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has invalid division_id {entity_id}"
                    ));
                } else if !seen_divisions.insert(entity_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} duplicates division_id {entity_id}"
                    ));
                }
                if join_id == 0 || join_id > MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has invalid province_id {join_id}"
                    ));
                }
                if flags & ROW_FLAG_PROXY_VALUE == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} missing proxy flag"
                    ));
                }
                if flags & ROW_FLAG_DIVISION_OWNER != 0 {
                    if owner_tag_db_id == 0 || owner_tag_db_id > MAX_TAG_DB_ID {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics division row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                        ));
                    }
                } else if owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has owner without owner flag"
                    ));
                }
                for (label, value, max_value, flag) in [
                    (
                        "current_supply_raw",
                        value_a_raw,
                        MAX_SUPPLY_RAW,
                        ROW_FLAG_DIVISION_CURRENT_SUPPLY,
                    ),
                    (
                        "max_supply_raw",
                        value_b_raw,
                        MAX_SUPPLY_RAW,
                        ROW_FLAG_DIVISION_MAX_SUPPLY,
                    ),
                    (
                        "supply_gain_raw",
                        value_c_raw,
                        MAX_SUPPLY_GAIN_RAW,
                        ROW_FLAG_DIVISION_SUPPLY_GAIN,
                    ),
                ] {
                    validate_nonnegative_value(
                        offset,
                        &mut summary,
                        row_index,
                        label,
                        value,
                        max_value,
                    );
                    validate_presence_flag(
                        offset,
                        &mut summary,
                        row_index,
                        label,
                        value,
                        flags,
                        flag,
                    );
                }
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "supply_deficit_raw",
                    value_d_raw,
                    MAX_SUPPLY_RAW,
                );
                let expected_deficit = value_b_raw.saturating_sub(value_a_raw).max(0);
                if value_d_raw != expected_deficit {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has supply_deficit_raw {value_d_raw} but expected {expected_deficit}"
                    ));
                }
                validate_presence_flag(
                    offset,
                    &mut summary,
                    row_index,
                    "supply_deficit_raw",
                    value_d_raw,
                    flags,
                    ROW_FLAG_DIVISION_SUPPLY_DEFICIT,
                );
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "fuel_raw",
                    aux_raw,
                    MAX_DIVISION_FUEL_RAW,
                );
                validate_presence_flag(
                    offset,
                    &mut summary,
                    row_index,
                    "fuel_raw",
                    aux_raw,
                    flags,
                    ROW_FLAG_DIVISION_FUEL,
                );
                if flags & ROW_FLAG_DIVISION_SUPPLY_DEFICIT != 0 {
                    add_unit(&mut summary, "division_supply_deficit_rows", 1);
                }
                if flags & ROW_FLAG_DIVISION_FUEL != 0 {
                    add_unit(&mut summary, "division_fuel_rows", 1);
                }
                add_unit(&mut summary, "division_supply_rows", 1);
            }
            ROW_KIND_CSUPPLY_PROVINCE_INPUT => {
                decoded_csupply_input_rows = decoded_csupply_input_rows.saturating_add(1);
                if entity_id == 0 || entity_id > MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} has invalid province_id {entity_id}"
                    ));
                } else if !seen_csupply_input_provinces.insert(entity_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} duplicates province_id {entity_id}"
                    ));
                }
                if join_id != 0 || owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} has nonzero join/owner fields"
                    ));
                }
                if flags & (ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY)
                    != (ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY)
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} missing required csupply flags"
                    ));
                }
                if flags & !(ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY) != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} has non-csupply flags"
                    ));
                }
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "csupply_efficiency_raw",
                    value_a_raw,
                    FIXED_SCALE_RAW as i32,
                );
                if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply input row {row_index} has nonzero reserved value fields"
                    ));
                }
                add_unit(&mut summary, "csupply_efficiency_rows", 1);
            }
            ROW_KIND_COUNTRY_DISRUPTED_SUPPLY => {
                decoded_country_disrupted_supply_rows =
                    decoded_country_disrupted_supply_rows.saturating_add(1);
                let deleted_id = (entity_id & ROW_ID_DELETED != 0)
                    .then_some(entity_id & !ROW_ID_DELETED);
                let province_id = deleted_id.unwrap_or(entity_id);
                if province_id == 0 || province_id > MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has invalid province_id {province_id}"
                    ));
                }
                if owner_tag_db_id == 0 || owner_tag_db_id > MAX_TAG_DB_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                    ));
                }
                if !seen_country_disrupted_supply.insert((
                    owner_tag_db_id,
                    province_id,
                    join_id,
                )) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} duplicates owner/province/node_type identity"
                    ));
                }
                if deleted_id.is_some() {
                    decoded_country_disrupted_supply_tombstones =
                        decoded_country_disrupted_supply_tombstones.saturating_add(1);
                    if flags != 0
                        || value_a_raw != 0
                        || value_b_raw != 0
                        || value_c_raw != 0
                        || value_d_raw != 0
                        || aux_raw != 0
                    {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics disrupted tombstone row {row_index} has live fields"
                        ));
                    }
                } else {
                    if flags != ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics disrupted row {row_index} has invalid flags {flags:#x}"
                        ));
                    }
                    validate_nonnegative_value(
                        offset,
                        &mut summary,
                        row_index,
                        "country_disrupted_supply_raw",
                        value_a_raw,
                        MAX_COUNTRY_DISRUPTED_SUPPLY_RAW,
                    );
                    if value_b_raw != 0
                        || value_c_raw != 0
                        || value_d_raw != 0
                        || aux_raw != 0
                    {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics disrupted row {row_index} has nonzero reserved value fields"
                        ));
                    }
                    add_unit(&mut summary, "country_disrupted_supply_rows", 1);
                }
            }
            ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE => {
                decoded_pressure_candidate_rows = decoded_pressure_candidate_rows.saturating_add(1);
                if entity_id == 0 || entity_id > MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} has invalid province_id {entity_id}"
                    ));
                }
                if owner_tag_db_id == 0 || owner_tag_db_id > MAX_TAG_DB_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                    ));
                }
                if join_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} has nonzero join_id {join_id}"
                    ));
                }
                if flags & (ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT)
                    != (ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT)
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} missing required pressure flags"
                    ));
                }
                if flags
                    & !(ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT)
                    != 0
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} has non-pressure flags"
                    ));
                }
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "foreign_homebase_supply_raw",
                    value_a_raw,
                    MAX_SUPPLY_RAW,
                );
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "foreign_homebase_penalty_raw",
                    value_b_raw,
                    FIXED_SCALE_RAW as i32,
                );
                if value_c_raw != entity_id as i32 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} duplicate province {value_c_raw} != entity_id {entity_id}"
                    ));
                }
                validate_nonnegative_value(
                    offset,
                    &mut summary,
                    row_index,
                    "pressure_record_offset",
                    value_d_raw,
                    1_048_576,
                );
                if aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} has nonzero reserved aux_raw"
                    ));
                }
                if !seen_pressure_candidates.insert((owner_tag_db_id, entity_id, value_d_raw)) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics pressure candidate row {row_index} duplicates owner/province/offset tuple"
                    ));
                }
                add_unit(&mut summary, "foreign_homebase_pressure_candidate_rows", 1);
            }
            ROW_KIND_COMPUTED_BADGE => {
                decoded_badge_rows = decoded_badge_rows.saturating_add(1);
                if entity_id != 0 || join_id != 0 || owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} has nonzero identity fields"
                    ));
                }
                if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} has nonzero reserved value fields"
                    ));
                }
                let badge_flags = flags
                    & (ROW_FLAG_EXACT_SUPPLY_WALL
                        | ROW_FLAG_ATTRITION_WALL
                        | ROW_FLAG_MOTORIZATION_WALL
                        | ROW_FLAG_CONVOY_PRESSURE_WALL
                        | ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED
                        | ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE);
                if badge_flags.count_ones() != 1 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} must carry exactly one badge flag"
                    ));
                }
                if flags != badge_flags {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} has non-badge flags"
                    ));
                }
                if flags & ROW_FLAG_EXACT_SUPPLY_WALL != 0 {
                    has_exact_supply_wall = true;
                    if value_a_raw != BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics exact-supply badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                if flags & ROW_FLAG_ATTRITION_WALL != 0 {
                    has_attrition_wall = true;
                    if value_a_raw != BADGE_ATTRITION_RUNTIME_COMPUTED {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics attrition badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                if flags & ROW_FLAG_MOTORIZATION_WALL != 0 {
                    has_motorization_wall = true;
                    if value_a_raw != BADGE_MOTORIZATION_SETTINGS_UNPINNED {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics motorization badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                if flags & ROW_FLAG_CONVOY_PRESSURE_WALL != 0 {
                    has_convoy_pressure_wall = true;
                    if value_a_raw != BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics convoy-pressure badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                if flags & ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED != 0 {
                    has_csupply_candidate_rejected = true;
                    if value_a_raw != BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics csupply candidate badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                if flags & ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE != 0 {
                    has_country_disrupted_source_available = true;
                    if value_a_raw != BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics country-disrupted source badge row {row_index} has invalid code {value_a_raw}"
                        ));
                    }
                }
                add_unit(&mut summary, "computed_badge_rows", 1);
            }
            other => summary.parse_errors.push(format!(
                "offset {offset}: supply logistics row {row_index} has unknown row_kind {other}"
            )),
        }
    }

    if decoded_province_rows != province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics province_rows {province_rows} != decoded {decoded_province_rows}"
        ));
    }
    if decoded_division_rows != division_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics division_rows {division_rows} != decoded {decoded_division_rows}"
        ));
    }
    if decoded_badge_rows != badge_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics badge_rows {badge_rows} != decoded {decoded_badge_rows}"
        ));
    }
    if decoded_csupply_input_rows + decoded_country_disrupted_supply_rows != csupply_input_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply_input_rows {csupply_input_rows} != decoded kind4+kind5 {}+{}",
            decoded_csupply_input_rows, decoded_country_disrupted_supply_rows
        ));
    }
    if decoded_pressure_candidate_rows != pressure_candidate_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics pressure_candidate_rows {pressure_candidate_rows} != decoded {decoded_pressure_candidate_rows}"
        ));
    }
    if decoded_country_disrupted_supply_rows > 0
        && !has_country_disrupted_source_available
    {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics disrupted rows lack exact-source availability badge"
        ));
    }
    for (label, present) in [
        ("exact supply wall", has_exact_supply_wall),
        ("attrition wall", has_attrition_wall),
        ("motorization wall", has_motorization_wall),
        ("convoy pressure wall", has_convoy_pressure_wall),
        (
            "csupply candidate rejection",
            has_csupply_candidate_rejected,
        ),
    ] {
        if !present {
            summary.parse_errors.push(format!(
                "offset {offset}: supply logistics missing {label} badge row"
            ));
        }
    }

    summary.data_rows_total = row_count as u64;
    summary.province_rows_total = decoded_province_rows as u64;
    summary.division_rows_total = decoded_division_rows as u64;
    summary.badge_rows_total = decoded_badge_rows as u64;
    summary.csupply_input_rows_total = decoded_csupply_input_rows as u64;
    summary.country_disrupted_supply_rows_total =
        (decoded_country_disrupted_supply_rows - decoded_country_disrupted_supply_tombstones) as u64;
    summary.country_disrupted_supply_tombstones_total =
        decoded_country_disrupted_supply_tombstones as u64;
    summary.country_disrupted_source_available = has_country_disrupted_source_available;
    summary.pressure_candidate_rows_total = decoded_pressure_candidate_rows as u64;
    add_unit(
        &mut summary,
        "observed_province_roots",
        observed_province_roots,
    );
    add_unit(
        &mut summary,
        "observed_division_roots",
        observed_division_roots,
    );
    summary
}

fn add_unit(summary: &mut Domain44GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn validate_nonnegative_value(
    offset: usize,
    summary: &mut Domain44GateSummary,
    row_index: usize,
    label: &str,
    value: i32,
    max_value: i32,
) {
    if !(0..=max_value).contains(&value) {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} has invalid {label} {value}"
        ));
    }
}

fn validate_presence_flag(
    offset: usize,
    summary: &mut Domain44GateSummary,
    row_index: usize,
    label: &str,
    value: i32,
    flags: u32,
    flag: u32,
) {
    if value != 0 && flags & flag == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} has nonzero {label} without presence flag"
        ));
    }
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

#[cfg(not(test))]
fn main() {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: domain44_supply_logistics_attrition <domain44_payload.bin>");
        std::process::exit(2);
    };
    let payload = match std::fs::read(&path) {
        Ok(payload) => payload,
        Err(err) => {
            eprintln!("failed to read {:?}: {err}", path);
            std::process::exit(2);
        }
    };
    let summary = decode_payload(0, &payload);
    println!("domain_id={DOMAIN_ID}");
    println!("domain_name={DOMAIN_NAME}");
    println!("data_rows_total={}", summary.data_rows_total);
    println!("province_rows_total={}", summary.province_rows_total);
    println!("division_rows_total={}", summary.division_rows_total);
    println!("badge_rows_total={}", summary.badge_rows_total);
    println!(
        "csupply_input_rows_total={}",
        summary.csupply_input_rows_total
    );
    println!(
        "country_disrupted_supply_rows_total={}",
        summary.country_disrupted_supply_rows_total
    );
    println!(
        "country_disrupted_supply_tombstones_total={}",
        summary.country_disrupted_supply_tombstones_total
    );
    println!(
        "country_disrupted_source_available={}",
        summary.country_disrupted_source_available
    );
    println!(
        "pressure_candidate_rows_total={}",
        summary.pressure_candidate_rows_total
    );
    println!(
        "observed_province_roots={}",
        summary.observed_province_roots
    );
    println!(
        "observed_division_roots={}",
        summary.observed_division_roots
    );
    println!("status_flags_or={:#x}", summary.status_flags_or);
    println!("data_units_total={:?}", summary.data_units_total);
    if summary.parse_errors.is_empty() {
        println!("parse_errors=0");
        return;
    }
    for err in &summary.parse_errors {
        eprintln!("{err}");
    }
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_rows() -> Vec<SupplyLogisticsRow> {
        vec![
            SupplyLogisticsRow {
                row_kind: ROW_KIND_PROVINCE_LOGISTICS,
                entity_id: 2068,
                join_id: 64,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_RAIL
                    | ROW_FLAG_SUPPLY_NODE
                    | ROW_FLAG_INFRASTRUCTURE
                    | ROW_FLAG_LIVE_BUILDING_VECTOR,
                value_a_raw: 5,
                value_b_raw: 2,
                value_c_raw: 0,
                value_d_raw: 4,
                aux_raw: 3,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_DIVISION_SUPPLY,
                entity_id: 42,
                join_id: 2068,
                owner_tag_db_id: 1,
                flags: ROW_FLAG_DIVISION_CURRENT_SUPPLY
                    | ROW_FLAG_DIVISION_MAX_SUPPLY
                    | ROW_FLAG_DIVISION_SUPPLY_GAIN
                    | ROW_FLAG_DIVISION_OWNER
                    | ROW_FLAG_DIVISION_SUPPLY_DEFICIT
                    | ROW_FLAG_DIVISION_FUEL
                    | ROW_FLAG_PROXY_VALUE,
                value_a_raw: 950_000,
                value_b_raw: 1_200_000,
                value_c_raw: 25_000,
                value_d_raw: 250_000,
                aux_raw: 99_000,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COUNTRY_DISRUPTED_SUPPLY,
                entity_id: 7081,
                join_id: 3,
                owner_tag_db_id: 47,
                flags: ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT,
                value_a_raw: 4017,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_EXACT_SUPPLY_WALL,
                value_a_raw: BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE,
                value_a_raw: BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_ATTRITION_WALL,
                value_a_raw: BADGE_ATTRITION_RUNTIME_COMPUTED,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_MOTORIZATION_WALL,
                value_a_raw: BADGE_MOTORIZATION_SETTINGS_UNPINNED,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_CONVOY_PRESSURE_WALL,
                value_a_raw: BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_COMPUTED_BADGE,
                entity_id: 0,
                join_id: 0,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED,
                value_a_raw: BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 0,
                aux_raw: 0,
            },
        ]
    }

    #[test]
    fn decodes_province_division_and_badge_rows() {
        let rows = fixture_rows();
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 150,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: rows.len() as u32,
            province_rows: 1,
            division_rows: 1,
            badge_rows: 6,
            observed_province_roots: 13_414,
            observed_division_roots: 1_600,
            csupply_input_rows: 1,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(32, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 9);
        assert_eq!(summary.province_rows_total, 1);
        assert_eq!(summary.division_rows_total, 1);
        assert_eq!(summary.badge_rows_total, 6);
        assert_eq!(summary.country_disrupted_supply_rows_total, 1);
        assert!(summary.country_disrupted_source_available);
        assert_eq!(summary.observed_province_roots, 13_414);
        assert_eq!(
            summary
                .data_units_total
                .get("division_supply_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("division_supply_deficit_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.data_units_total.get("division_fuel_rows").copied(),
            Some(1)
        );
    }

    #[test]
    fn decodes_composite_disrupted_tombstone_and_requires_source_witness() {
        let mut rows = fixture_rows();
        let disrupted = rows
            .iter_mut()
            .find(|row| row.row_kind == ROW_KIND_COUNTRY_DISRUPTED_SUPPLY)
            .expect("fixture disrupted row");
        disrupted.entity_id |= ROW_ID_DELETED;
        disrupted.flags = 0;
        disrupted.value_a_raw = 0;
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: rows.len() as u32,
            province_rows: 1,
            division_rows: 1,
            badge_rows: 6,
            observed_province_roots: 1,
            observed_division_roots: 1,
            csupply_input_rows: 1,
            pressure_candidate_rows: 0,
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.country_disrupted_supply_rows_total, 0);
        assert_eq!(summary.country_disrupted_supply_tombstones_total, 1);

        rows.retain(|row| {
            row.flags != ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE
        });
        let header = SupplyLogisticsHeader {
            changed_rows: rows.len() as u32,
            badge_rows: 5,
            ..header
        };
        let summary = decode_payload(0, &encode_payload(&header, &rows));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("lack exact-source availability badge")));
    }

    #[test]
    fn rejects_header_counter_mismatch_and_missing_wall_badge() {
        let rows = vec![fixture_rows().remove(0)];
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: 1,
            province_rows: 1,
            division_rows: 1,
            badge_rows: 0,
            observed_province_roots: 1,
            observed_division_roots: 0,
            csupply_input_rows: 0,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("row counters")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("missing exact supply wall badge row")));
    }

    #[test]
    fn rejects_invalid_division_supply_values() {
        let rows = vec![SupplyLogisticsRow {
            row_kind: ROW_KIND_DIVISION_SUPPLY,
            entity_id: 7,
            join_id: 9_999_999,
            owner_tag_db_id: 0,
            flags: ROW_FLAG_DIVISION_OWNER,
            value_a_raw: -1,
            value_b_raw: MAX_SUPPLY_RAW + 1,
            value_c_raw: MAX_SUPPLY_GAIN_RAW + 1,
            value_d_raw: MAX_SUPPLY_RAW + 1,
            aux_raw: -1,
        }];
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            province_rows: 0,
            division_rows: 1,
            badge_rows: 0,
            observed_province_roots: 0,
            observed_division_roots: 1,
            csupply_input_rows: 0,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("invalid province_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("invalid owner_tag_db_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("current_supply_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("supply_gain_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("supply_deficit_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("fuel_raw")));
    }

    #[test]
    fn rejects_inconsistent_deficit_and_missing_value_flags() {
        let rows = vec![SupplyLogisticsRow {
            row_kind: ROW_KIND_DIVISION_SUPPLY,
            entity_id: 77,
            join_id: 2068,
            owner_tag_db_id: 1,
            flags: ROW_FLAG_DIVISION_OWNER | ROW_FLAG_PROXY_VALUE,
            value_a_raw: 500_000,
            value_b_raw: 900_000,
            value_c_raw: 0,
            value_d_raw: 250_000,
            aux_raw: 10_000,
        }];
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            province_rows: 0,
            division_rows: 1,
            badge_rows: 0,
            observed_province_roots: 0,
            observed_division_roots: 1,
            csupply_input_rows: 0,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("expected 400000")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero supply_deficit_raw without presence flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero fuel_raw without presence flag")));
    }

    #[test]
    fn rejects_missing_province_and_division_contract_flags() {
        let rows = vec![
            SupplyLogisticsRow {
                row_kind: ROW_KIND_PROVINCE_LOGISTICS,
                entity_id: 2068,
                join_id: 64,
                owner_tag_db_id: 99,
                flags: 0,
                value_a_raw: 1,
                value_b_raw: 0,
                value_c_raw: 0,
                value_d_raw: 3,
                aux_raw: 1,
            },
            SupplyLogisticsRow {
                row_kind: ROW_KIND_DIVISION_SUPPLY,
                entity_id: 42,
                join_id: 2068,
                owner_tag_db_id: 0,
                flags: ROW_FLAG_DIVISION_CURRENT_SUPPLY,
                value_a_raw: 100_000,
                value_b_raw: 200_000,
                value_c_raw: 10_000,
                value_d_raw: 100_000,
                aux_raw: 5_000,
            },
        ];
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: rows.len() as u32,
            province_rows: 1,
            division_rows: 1,
            badge_rows: 0,
            observed_province_roots: 1,
            observed_division_roots: 1,
            csupply_input_rows: 0,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero owner_tag_db_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("missing live building vector flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero rail without presence flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("missing proxy flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero max_supply_raw without presence flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero supply_gain_raw without presence flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero supply_deficit_raw without presence flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero fuel_raw without presence flag")));
    }

    #[test]
    fn rejects_malformed_badge_rows() {
        let rows = vec![SupplyLogisticsRow {
            row_kind: ROW_KIND_COMPUTED_BADGE,
            entity_id: 1,
            join_id: 2,
            owner_tag_db_id: 3,
            flags: ROW_FLAG_EXACT_SUPPLY_WALL | ROW_FLAG_ATTRITION_WALL | ROW_FLAG_PROXY_VALUE,
            value_a_raw: 999,
            value_b_raw: 1,
            value_c_raw: 0,
            value_d_raw: 0,
            aux_raw: 0,
        }];
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: 1,
            province_rows: 0,
            division_rows: 0,
            badge_rows: 1,
            observed_province_roots: 0,
            observed_division_roots: 0,
            csupply_input_rows: 0,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero identity fields")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero reserved value fields")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("exactly one badge flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("non-badge flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("exact-supply badge")));
    }

    #[test]
    fn decodes_and_rejects_csupply_input_rows() {
        let valid = SupplyLogisticsRow {
            row_kind: ROW_KIND_CSUPPLY_PROVINCE_INPUT,
            entity_id: 3324,
            join_id: 0,
            owner_tag_db_id: 0,
            flags: ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_CSUPPLY_PROVINCE_ARRAY,
            value_a_raw: 95_000,
            value_b_raw: 0,
            value_c_raw: 0,
            value_d_raw: 0,
            aux_raw: 0,
        };
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: 1,
            province_rows: 0,
            division_rows: 0,
            badge_rows: 0,
            observed_province_roots: 0,
            observed_division_roots: 0,
            csupply_input_rows: 1,
            pressure_candidate_rows: 0,
        };
        let payload = encode_payload(&header, &[valid.clone()]);
        let summary = decode_payload(0, &payload);
        assert!(
            summary
                .parse_errors
                .iter()
                .all(|err| err.contains("missing") && err.contains("badge row")),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.csupply_input_rows_total, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("csupply_efficiency_rows")
                .copied(),
            Some(1)
        );

        let invalid = SupplyLogisticsRow {
            join_id: 64,
            flags: ROW_FLAG_CSUPPLY_EFFICIENCY | ROW_FLAG_PROXY_VALUE,
            value_a_raw: FIXED_SCALE_RAW as i32 + 1,
            value_b_raw: 7,
            ..valid
        };
        let payload = encode_payload(&header, &[invalid]);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero join/owner")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("missing required csupply flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("non-csupply flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("csupply_efficiency_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero reserved value fields")));
    }

    #[test]
    fn decodes_and_rejects_pressure_candidate_rows() {
        let mut rows = fixture_rows();
        rows.push(SupplyLogisticsRow {
            row_kind: ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE,
            entity_id: 4948,
            join_id: 0,
            owner_tag_db_id: 30,
            flags: ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PRESSURE_CANDIDATE_LAYOUT,
            value_a_raw: 25_000,
            value_b_raw: 84_000,
            value_c_raw: 4948,
            value_d_raw: 0x2A0,
            aux_raw: 0,
        });
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: rows.len() as u32,
            province_rows: 1,
            division_rows: 1,
            badge_rows: 6,
            observed_province_roots: 1,
            observed_division_roots: 1,
            csupply_input_rows: 1,
            pressure_candidate_rows: 1,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.pressure_candidate_rows_total, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("foreign_homebase_pressure_candidate_rows")
                .copied(),
            Some(1)
        );

        let invalid = SupplyLogisticsRow {
            row_kind: ROW_KIND_FOREIGN_HOMEBASE_PRESSURE_CANDIDATE,
            entity_id: MAX_PROVINCE_ID + 1,
            join_id: 7,
            owner_tag_db_id: 0,
            flags: ROW_FLAG_FOREIGN_HOMEBASE_PRESSURE | ROW_FLAG_PROXY_VALUE,
            value_a_raw: -1,
            value_b_raw: FIXED_SCALE_RAW as i32 + 1,
            value_c_raw: 123,
            value_d_raw: -1,
            aux_raw: 1,
        };
        let header = SupplyLogisticsHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_COMPACT_HEALTH_OK,
            changed_rows: 1,
            province_rows: 0,
            division_rows: 0,
            badge_rows: 0,
            observed_province_roots: 0,
            observed_division_roots: 0,
            csupply_input_rows: 0,
            pressure_candidate_rows: 1,
        };
        let payload = encode_payload(&header, &[invalid]);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("invalid province_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("invalid owner_tag_db_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero join_id")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("missing required pressure flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("non-pressure flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("foreign_homebase_supply_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("foreign_homebase_penalty_raw")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("duplicate province")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("pressure_record_offset")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("nonzero reserved aux_raw")));
    }
}
