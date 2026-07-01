use std::collections::{BTreeMap, BTreeSet};
use std::convert::TryInto;

pub const DOMAIN_ID: u16 = 52;
pub const DOMAIN_NAME: &str = "occupation_resistance_compliance_live";
pub const HEADER_LEN: usize = 52;
pub const ROW_LEN: usize = 64;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const ROW_FLAG_OWNER: u32 = 1;
pub const ROW_FLAG_CONTROLLER: u32 = 1 << 1;
pub const ROW_FLAG_RESISTANCE: u32 = 1 << 2;
pub const ROW_FLAG_COMPLIANCE: u32 = 1 << 3;
pub const ROW_FLAG_OCCUPIED_STATE: u32 = 1 << 4;
pub const ROW_FLAG_OCCUPIED_TAG_DERIVED: u32 = 1 << 5;
pub const ROW_FLAG_OCCUPATION_LAW_TOKEN: u32 = 1 << 6;
pub const ROW_FLAG_GARRISON_TEMPLATE: u32 = 1 << 7;
pub const ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE: u32 = 1 << 8;
pub const ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE: u32 = 1 << 9;
pub const ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE: u32 = 1 << 10;
pub const ROW_FLAG_RESISTANCE_SPEED: u32 = 1 << 11;
pub const ROW_FLAG_COMPLIANCE_SPEED: u32 = 1 << 12;
pub const ROW_FLAG_OCCUPATION_STATUS_ROOT: u32 = 1 << 13;
pub const ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN: u32 = 1 << 14;

pub const MAX_STATE_ID: u32 = 65_535;
pub const MAX_TAG_DB_ID: u32 = 1_024;
pub const MAX_ABS_FIXED_RAW: i32 = 100_000_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_state_rows: u32,
    pub occupied_state_rows: u32,
    pub wall_badge_rows: u32,
    pub law_token_rows: u32,
    pub garrison_template_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationStateRow {
    pub state_id: u32,
    pub owner_tag_db_id: u32,
    pub controller_tag_db_id: u32,
    pub occupied_country_tag_db_id: u32,
    pub resistance_raw: i32,
    pub compliance_raw: i32,
    pub resistance_speed_raw: i32,
    pub compliance_speed_raw: i32,
    pub garrison_need_raw: i32,
    pub garrison_equilibrium_raw: i32,
    pub garrison_progress_raw: i32,
    pub occupation_law_token_id: u32,
    pub garrison_template_id: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain44GateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(header: &OccupationHeader, rows: &[OccupationStateRow]) -> Vec<u8> {
    let mut payload =
        Vec::with_capacity(HEADER_LEN.saturating_add(rows.len().saturating_mul(ROW_LEN)));
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.observed_state_rows.to_le_bytes());
    payload.extend_from_slice(&header.occupied_state_rows.to_le_bytes());
    payload.extend_from_slice(&header.wall_badge_rows.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&header.law_token_rows.to_le_bytes());
    payload.extend_from_slice(&header.garrison_template_rows.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.state_id.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.controller_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.occupied_country_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.resistance_raw.to_le_bytes());
        payload.extend_from_slice(&row.compliance_raw.to_le_bytes());
        payload.extend_from_slice(&row.resistance_speed_raw.to_le_bytes());
        payload.extend_from_slice(&row.compliance_speed_raw.to_le_bytes());
        payload.extend_from_slice(&row.garrison_need_raw.to_le_bytes());
        payload.extend_from_slice(&row.garrison_equilibrium_raw.to_le_bytes());
        payload.extend_from_slice(&row.garrison_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.occupation_law_token_id.to_le_bytes());
        payload.extend_from_slice(&row.garrison_template_id.to_le_bytes());
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain44GateSummary {
    let mut summary = Domain44GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation resistance compliance data payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_state_rows = read_u32_le(payload, 20) as usize;
    let occupied_state_rows = read_u32_le(payload, 24) as usize;
    let wall_badge_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let law_token_rows = read_u32_le(payload, 36) as usize;
    let garrison_template_rows = read_u32_le(payload, 40) as usize;
    let reserved0 = read_u32_le(payload, 44);
    let reserved1 = read_u32_le(payload, 48);
    summary.status_flags_or |= status_flags;

    if scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation fixed scale {scale_raw} != {FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation header reserved field is nonzero"
        ));
    }
    let expected_len = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }
    if row_count > observed_state_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation row_count {row_count} > observed_state_rows {observed_state_rows}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if occupied_state_rows > row_count
        || wall_badge_rows > row_count
        || law_token_rows > row_count
        || garrison_template_rows > row_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation counters exceed row_count {row_count}"
        ));
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "occupation_state_rows", row_count);
    add_unit(&mut summary, "occupation_observed_state_rows", observed_state_rows);
    add_unit(&mut summary, "occupation_occupied_state_rows", occupied_state_rows);
    add_unit(&mut summary, "occupation_wall_badge_rows", wall_badge_rows);
    add_unit(&mut summary, "occupation_law_token_rows", law_token_rows);
    add_unit(
        &mut summary,
        "occupation_garrison_template_rows",
        garrison_template_rows,
    );

    let mut cursor = HEADER_LEN;
    let mut seen_states = BTreeSet::new();
    let mut decoded_occupied = 0usize;
    let mut decoded_wall_badges = 0usize;
    let mut decoded_law_tokens = 0usize;
    let mut decoded_garrison_templates = 0usize;
    for row_index in 0..row_count {
        let state_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let controller_tag_db_id = read_u32_le(payload, cursor + 8);
        let occupied_country_tag_db_id = read_u32_le(payload, cursor + 12);
        let resistance_raw = read_i32_le(payload, cursor + 16);
        let compliance_raw = read_i32_le(payload, cursor + 20);
        let resistance_speed_raw = read_i32_le(payload, cursor + 24);
        let compliance_speed_raw = read_i32_le(payload, cursor + 28);
        let garrison_need_raw = read_i32_le(payload, cursor + 32);
        let garrison_equilibrium_raw = read_i32_le(payload, cursor + 36);
        let garrison_progress_raw = read_i32_le(payload, cursor + 40);
        let occupation_law_token_id = read_u32_le(payload, cursor + 44);
        let garrison_template_id = read_u32_le(payload, cursor + 48);
        let flags = read_u32_le(payload, cursor + 52);
        let reserved1 = read_u32_le(payload, cursor + 56);
        let reserved2 = read_u32_le(payload, cursor + 60);
        cursor += ROW_LEN;

        if state_id == 0 || state_id > MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has invalid state_id {state_id}"
            ));
        } else if !seen_states.insert(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} duplicates state_id {state_id}"
            ));
        }
        validate_tag_field(
            offset,
            &mut summary,
            "owner",
            row_index,
            owner_tag_db_id,
            flags & ROW_FLAG_OWNER != 0,
        );
        validate_tag_field(
            offset,
            &mut summary,
            "controller",
            row_index,
            controller_tag_db_id,
            flags & ROW_FLAG_CONTROLLER != 0,
        );
        validate_tag_field(
            offset,
            &mut summary,
            "occupied country",
            row_index,
            occupied_country_tag_db_id,
            flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0,
        );
        validate_fixed(offset, &mut summary, "resistance", row_index, resistance_raw);
        validate_fixed(offset, &mut summary, "compliance", row_index, compliance_raw);
        validate_fixed(
            offset,
            &mut summary,
            "resistance_speed",
            row_index,
            resistance_speed_raw,
        );
        validate_fixed(
            offset,
            &mut summary,
            "compliance_speed",
            row_index,
            compliance_speed_raw,
        );
        validate_fixed(offset, &mut summary, "garrison_need", row_index, garrison_need_raw);
        validate_fixed(
            offset,
            &mut summary,
            "garrison_equilibrium",
            row_index,
            garrison_equilibrium_raw,
        );
        validate_fixed(
            offset,
            &mut summary,
            "garrison_progress",
            row_index,
            garrison_progress_raw,
        );
        if flags & ROW_FLAG_OCCUPIED_STATE != 0 {
            decoded_occupied = decoded_occupied.saturating_add(1);
        }
        if flags
            & (ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE)
            != 0
        {
            decoded_wall_badges = decoded_wall_badges.saturating_add(1);
        }
        if flags & ROW_FLAG_OCCUPATION_LAW_TOKEN != 0 {
            decoded_law_tokens = decoded_law_tokens.saturating_add(1);
            if occupation_law_token_id == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: occupation row {row_index} has law-token flag without token"
                ));
            }
        } else if occupation_law_token_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has law token without flag"
            ));
        }
        if flags & ROW_FLAG_GARRISON_TEMPLATE != 0 {
            decoded_garrison_templates = decoded_garrison_templates.saturating_add(1);
            if garrison_template_id == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: occupation row {row_index} has garrison-template flag without id"
                ));
            }
        } else if garrison_template_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has garrison template id without flag"
            ));
        }
        if flags
            & !(ROW_FLAG_OWNER
                | ROW_FLAG_CONTROLLER
                | ROW_FLAG_RESISTANCE
                | ROW_FLAG_COMPLIANCE
                | ROW_FLAG_OCCUPIED_STATE
                | ROW_FLAG_OCCUPIED_TAG_DERIVED
                | ROW_FLAG_OCCUPATION_LAW_TOKEN
                | ROW_FLAG_GARRISON_TEMPLATE
                | ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE
                | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE
                | ROW_FLAG_RESISTANCE_SPEED
                | ROW_FLAG_COMPLIANCE_SPEED
                | ROW_FLAG_OCCUPATION_STATUS_ROOT
                | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has unknown flags {flags}"
            ));
        }
        if flags & ROW_FLAG_RESISTANCE == 0 && resistance_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has resistance without flag"
            ));
        }
        if flags & ROW_FLAG_COMPLIANCE == 0 && compliance_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has compliance without flag"
            ));
        }
        if flags & ROW_FLAG_RESISTANCE_SPEED == 0 && resistance_speed_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has resistance speed without flag"
            ));
        }
        if flags & ROW_FLAG_COMPLIANCE_SPEED == 0 && compliance_speed_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} has compliance speed without flag"
            ));
        }
        if flags & ROW_FLAG_RESISTANCE_SPEED_UNAVAILABLE != 0
            && flags & (ROW_FLAG_RESISTANCE_SPEED | ROW_FLAG_COMPLIANCE_SPEED) != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} marks speed both measured and unavailable"
            ));
        }
        if flags & ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE != 0
            && (garrison_need_raw != 0
                || garrison_equilibrium_raw != 0
                || garrison_progress_raw != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} marks garrison signals unavailable with nonzero garrison values"
            ));
        }
        if flags & ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN != 0
            && flags & ROW_FLAG_OCCUPATION_LAW_TOKEN == 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} marks default law token without law-token flag"
            ));
        }
        if flags & ROW_FLAG_OCCUPIED_TAG_DERIVED != 0
            && !(flags & ROW_FLAG_OWNER != 0 && flags & ROW_FLAG_CONTROLLER != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} derives occupied tag without owner/controller"
            ));
        }
        if reserved1 != 0 || reserved2 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation row {row_index} reserved field is nonzero"
            ));
        }
    }
    if decoded_occupied != occupied_state_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation occupied_state_rows {occupied_state_rows} != decoded {decoded_occupied}"
        ));
    }
    if decoded_wall_badges != wall_badge_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation wall_badge_rows {wall_badge_rows} != decoded {decoded_wall_badges}"
        ));
    }
    if decoded_law_tokens != law_token_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation law_token_rows {law_token_rows} != decoded {decoded_law_tokens}"
        ));
    }
    if decoded_garrison_templates != garrison_template_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation garrison_template_rows {garrison_template_rows} != decoded {decoded_garrison_templates}"
        ));
    }
    summary
}

fn validate_tag_field(
    offset: usize,
    summary: &mut Domain44GateSummary,
    name: &str,
    row_index: usize,
    value: u32,
    flag_present: bool,
) {
    if value > MAX_TAG_DB_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation row {row_index} has invalid {name} tag_db_id {value}"
        ));
    }
    if !flag_present && value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation row {row_index} has {name} tag without flag"
        ));
    }
    if flag_present && value == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation row {row_index} has {name} flag without tag"
        ));
    }
}

fn validate_fixed(
    offset: usize,
    summary: &mut Domain44GateSummary,
    name: &str,
    row_index: usize,
    value: i32,
) {
    if value.abs() > MAX_ABS_FIXED_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation row {row_index} has invalid {name} raw value {value}"
        ));
    }
}

fn add_unit(summary: &mut Domain44GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(payload[offset..offset + 4].try_into().expect("u32 range"))
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(payload[offset..offset + 4].try_into().expect("i32 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn occupied_row() -> OccupationStateRow {
        OccupationStateRow {
            state_id: 64,
            owner_tag_db_id: 1,
            controller_tag_db_id: 2,
            occupied_country_tag_db_id: 1,
            resistance_raw: 123_456,
            compliance_raw: 5_000_000,
            resistance_speed_raw: -450,
            compliance_speed_raw: 571,
            garrison_need_raw: 0,
            garrison_equilibrium_raw: 0,
            garrison_progress_raw: 0,
            occupation_law_token_id: 0xA11C_E001,
            garrison_template_id: 0,
            flags: ROW_FLAG_OWNER
                | ROW_FLAG_CONTROLLER
                | ROW_FLAG_RESISTANCE
                | ROW_FLAG_COMPLIANCE
                | ROW_FLAG_OCCUPIED_STATE
                | ROW_FLAG_OCCUPIED_TAG_DERIVED
                | ROW_FLAG_OCCUPATION_LAW_TOKEN
                | ROW_FLAG_RESISTANCE_SPEED
                | ROW_FLAG_COMPLIANCE_SPEED
                | ROW_FLAG_OCCUPATION_STATUS_ROOT
                | ROW_FLAG_COUNTRY_DEFAULT_LAW_TOKEN
                | ROW_FLAG_GARRISON_SIGNALS_UNAVAILABLE
                | ROW_FLAG_OCCUPATION_LAW_UNAVAILABLE,
        }
    }

    #[test]
    fn decodes_live_state_rows_with_wall_badges() {
        let rows = vec![occupied_row()];
        let header = OccupationHeader {
            elapsed_wall_ms: 10,
            status_flags: 0,
            changed_rows: 1,
            observed_state_rows: 1081,
            occupied_state_rows: 1,
            wall_badge_rows: 1,
            law_token_rows: 1,
            garrison_template_rows: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(32, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.data_rows_total, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("occupation_occupied_state_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("occupation_wall_badge_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn accepts_measured_zero_unoccupied_baseline() {
        let rows = vec![OccupationStateRow {
            state_id: 1,
            owner_tag_db_id: 1,
            controller_tag_db_id: 1,
            occupied_country_tag_db_id: 0,
            resistance_raw: 0,
            compliance_raw: 0,
            resistance_speed_raw: 0,
            compliance_speed_raw: 0,
            garrison_need_raw: 0,
            garrison_equilibrium_raw: 0,
            garrison_progress_raw: 0,
            occupation_law_token_id: 0,
            garrison_template_id: 0,
            flags: ROW_FLAG_OWNER
                | ROW_FLAG_CONTROLLER
                | ROW_FLAG_RESISTANCE
                | ROW_FLAG_COMPLIANCE
                | ROW_FLAG_RESISTANCE_SPEED
                | ROW_FLAG_COMPLIANCE_SPEED
                | ROW_FLAG_OCCUPATION_STATUS_ROOT,
        }];
        let payload = encode_payload(
            &OccupationHeader {
                elapsed_wall_ms: 0,
                status_flags: STATUS_DELTA_DATA,
                changed_rows: 1,
                observed_state_rows: 1,
                occupied_state_rows: 0,
                wall_badge_rows: 0,
                law_token_rows: 0,
                garrison_template_rows: 0,
            },
            &rows,
        );
        let summary = decode_payload(0, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(summary.status_flags_or, STATUS_DELTA_DATA);
    }

    #[test]
    fn rejects_nonzero_garrison_values_when_badged_unavailable() {
        let mut row = occupied_row();
        row.garrison_need_raw = 10;
        let payload = encode_payload(
            &OccupationHeader {
                elapsed_wall_ms: 10,
                status_flags: 0,
                changed_rows: 1,
                observed_state_rows: 1,
                occupied_state_rows: 1,
                wall_badge_rows: 1,
                law_token_rows: 1,
                garrison_template_rows: 0,
            },
            &[row],
        );
        let summary = decode_payload(32, &payload);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|error| error.contains("garrison signals unavailable")),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn accepts_validated_garrison_template_rows() {
        let mut row = occupied_row();
        row.garrison_template_id = 4_183;
        row.flags |= ROW_FLAG_GARRISON_TEMPLATE;
        let payload = encode_payload(
            &OccupationHeader {
                elapsed_wall_ms: 10,
                status_flags: 0,
                changed_rows: 1,
                observed_state_rows: 1,
                occupied_state_rows: 1,
                wall_badge_rows: 1,
                law_token_rows: 1,
                garrison_template_rows: 1,
            },
            &[row],
        );
        let summary = decode_payload(32, &payload);
        assert!(summary.parse_errors.is_empty(), "{:?}", summary.parse_errors);
        assert_eq!(
            summary
                .data_units_total
                .get("occupation_garrison_template_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_bad_counter_and_flag_contracts() {
        let mut row = occupied_row();
        row.flags &= !ROW_FLAG_OCCUPIED_TAG_DERIVED;
        let mut payload = encode_payload(
            &OccupationHeader {
                elapsed_wall_ms: 0,
                status_flags: 0,
                changed_rows: 1,
                observed_state_rows: 1,
                occupied_state_rows: 0,
                wall_badge_rows: 1,
                law_token_rows: 0,
                garrison_template_rows: 0,
            },
            &[row],
        );
        payload[32..36].copy_from_slice(&1u32.to_le_bytes());
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("fixed scale 1")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("occupied country tag without flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("occupied_state_rows 0 != decoded 1")));
    }
}
