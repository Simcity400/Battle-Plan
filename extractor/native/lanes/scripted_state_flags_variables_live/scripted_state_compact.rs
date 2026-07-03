use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 51;
pub const DOMAIN_NAME: &str = "scripted_state_flags_variables_live";
pub const SCHEMA_VERSION: u32 = 1;
pub const HEADER_LEN: usize = 64;
pub const ROW_LEN: usize = 48;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
pub const STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const STATUS_FLAG_NAMES_PARTIAL: u32 = compact_contract::DOMAIN51_STATUS_FLAG_NAMES_PARTIAL;
pub const STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED: u32 =
    compact_contract::DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED;
pub const STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED: u32 =
    compact_contract::DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;

pub const SCOPE_GLOBAL: u32 = 1;
pub const SCOPE_COUNTRY: u32 = 2;
pub const SCOPE_STATE: u32 = 3;

pub const VALUE_VARIABLE_FIXED_I64: u32 = 1;
pub const VALUE_FLAG_NAMED: u32 = 2;
pub const VALUE_FLAG_TOKEN_ONLY: u32 = 3;
pub const VALUE_DELAYED_EVENT_OPAQUE: u32 = 4;
pub const VALUE_SCRIPTED_GUI_OPAQUE: u32 = 5;

pub const DELAYED_EVENTS_BADGE_KEY: &str = "root_unresolved:delayed_events";
pub const SCRIPTED_GUI_BADGE_KEY: &str = "root_unresolved:scripted_gui_open_state";

pub const ROW_FLAG_KEY_PRESENT: u32 = 1;
pub const ROW_FLAG_TOKEN_PRESENT: u32 = 1 << 1;
pub const ROW_FLAG_VALUE_PRESENT: u32 = 1 << 2;
pub const ROW_FLAG_DATE_PRESENT: u32 = 1 << 3;
pub const ROW_FLAG_DELETED: u32 = 1 << 4;

pub const MAX_KEY_BYTES: usize = 512;
pub const MAX_KEY_TABLE_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_COUNTRY_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_STATE_ID: u32 = 65_535;

const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_KEY_PRESENT
    | ROW_FLAG_TOKEN_PRESENT
    | ROW_FLAG_VALUE_PRESENT
    | ROW_FLAG_DATE_PRESENT
    | ROW_FLAG_DELETED;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedStateHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_scopes: u32,
    pub measured_scopes: u32,
    pub rejected_scopes: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptedStateRow {
    pub scope_kind: u32,
    pub owner_id: u32,
    pub value_kind: u32,
    pub key: Option<String>,
    pub token_id: Option<u32>,
    pub value_raw: Option<i64>,
    pub date_hour_raw: Option<i32>,
    pub deleted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain51GateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(header: &ScriptedStateHeader, rows: &[ScriptedStateRow]) -> Vec<u8> {
    let mut key_bytes = Vec::new();
    let mut row_keys = Vec::with_capacity(rows.len());
    let mut variable_rows = 0u32;
    let mut flag_rows = 0u32;
    let mut token_only_rows = 0u32;

    for row in rows {
        if row.value_kind == VALUE_VARIABLE_FIXED_I64 {
            variable_rows = variable_rows.saturating_add(1);
        }
        if row.value_kind == VALUE_FLAG_NAMED || row.value_kind == VALUE_FLAG_TOKEN_ONLY {
            flag_rows = flag_rows.saturating_add(1);
        }
        if row.key.is_none() && row.token_id.is_some() {
            token_only_rows = token_only_rows.saturating_add(1);
        }
        if let Some(key) = row.key.as_ref() {
            let offset = key_bytes.len().min(u32::MAX as usize) as u32;
            let len = key.as_bytes().len().min(u32::MAX as usize) as u32;
            key_bytes.extend_from_slice(key.as_bytes());
            row_keys.push((offset, len));
        } else {
            row_keys.push((0, 0));
        }
    }

    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let key_bytes_len = key_bytes.len().min(u32::MAX as usize) as u32;
    let mut payload = Vec::with_capacity(
        HEADER_LEN
            .saturating_add(rows.len().saturating_mul(ROW_LEN))
            .saturating_add(key_bytes.len()),
    );
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&key_bytes_len.to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.observed_scopes.to_le_bytes());
    payload.extend_from_slice(&header.measured_scopes.to_le_bytes());
    payload.extend_from_slice(&header.rejected_scopes.to_le_bytes());
    payload.extend_from_slice(&variable_rows.to_le_bytes());
    payload.extend_from_slice(&flag_rows.to_le_bytes());
    payload.extend_from_slice(&token_only_rows.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    for (row, (key_offset, key_len)) in rows.iter().zip(row_keys.into_iter()) {
        let mut flags = 0u32;
        if key_len != 0 {
            flags |= ROW_FLAG_KEY_PRESENT;
        }
        if row.token_id.is_some() {
            flags |= ROW_FLAG_TOKEN_PRESENT;
        }
        if row.value_raw.is_some() {
            flags |= ROW_FLAG_VALUE_PRESENT;
        }
        if row.date_hour_raw.is_some() {
            flags |= ROW_FLAG_DATE_PRESENT;
        }
        if row.deleted {
            flags |= ROW_FLAG_DELETED;
        }
        payload.extend_from_slice(&row.scope_kind.to_le_bytes());
        payload.extend_from_slice(&row.owner_id.to_le_bytes());
        payload.extend_from_slice(&row.value_kind.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&key_offset.to_le_bytes());
        payload.extend_from_slice(&key_len.to_le_bytes());
        payload.extend_from_slice(&row.token_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.date_hour_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.value_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload.extend_from_slice(&key_bytes);
    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain51GateSummary {
    let mut summary = Domain51GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let key_bytes_len = read_u32_le(payload, 12) as usize;
    let changed_rows = read_u32_le(payload, 16) as usize;
    let status_flags = read_u32_le(payload, 20);
    let observed_scopes = read_u32_le(payload, 24) as usize;
    let measured_scopes = read_u32_le(payload, 28) as usize;
    let rejected_scopes = read_u32_le(payload, 32) as usize;
    let variable_rows = read_u32_le(payload, 36) as usize;
    let flag_rows = read_u32_le(payload, 40) as usize;
    let token_only_rows = read_u32_le(payload, 44) as usize;
    let fixed_scale = read_u32_le(payload, 48);
    let schema_version = read_u32_le(payload, 52);
    let reserved0 = read_u32_le(payload, 56);
    let reserved1 = read_u32_le(payload, 60);
    summary.status_flags_or |= status_flags;

    if fixed_scale != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state fixed scale {fixed_scale} != {FIXED_SCALE_RAW}"
        ));
    }
    if schema_version != SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state schema version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state header reserved fields are nonzero"
        ));
    }
    if key_bytes_len > MAX_KEY_TABLE_BYTES {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state key table is {key_bytes_len} byte(s), max {MAX_KEY_TABLE_BYTES}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state changed_rows {changed_rows} exceeds row_count {row_count}"
        ));
    }
    if measured_scopes > observed_scopes {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state measured_scopes {measured_scopes} exceeds observed_scopes {observed_scopes}"
        ));
    }

    let rows_len = match row_count.checked_mul(ROW_LEN) {
        Some(value) => value,
        None => {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row length overflows"
            ));
            return summary;
        }
    };
    let key_start = match HEADER_LEN.checked_add(rows_len) {
        Some(value) => value,
        None => {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state payload length overflows"
            ));
            return summary;
        }
    };
    let expected_len = match key_start.checked_add(key_bytes_len) {
        Some(value) => value,
        None => {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state key table length overflows"
            ));
            return summary;
        }
    };
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "scripted_state_rows", row_count);
    add_unit(
        &mut summary,
        "scripted_state_observed_scopes",
        observed_scopes,
    );
    add_unit(
        &mut summary,
        "scripted_state_measured_scopes",
        measured_scopes,
    );
    add_unit(
        &mut summary,
        "scripted_state_rejected_scopes",
        rejected_scopes,
    );
    add_unit(&mut summary, "scripted_variable_rows", variable_rows);
    add_unit(&mut summary, "scripted_flag_rows", flag_rows);
    add_unit(
        &mut summary,
        "scripted_token_only_flag_rows",
        token_only_rows,
    );

    let key_table = &payload[key_start..expected_len];
    let mut decoded_variable_rows = 0usize;
    let mut decoded_flag_rows = 0usize;
    let mut decoded_token_only_rows = 0usize;
    let mut decoded_delayed_event_rows = 0usize;
    let mut decoded_scripted_gui_rows = 0usize;
    let mut seen_rows = BTreeSet::new();

    for row_index in 0..row_count {
        let cursor = HEADER_LEN + row_index * ROW_LEN;
        let scope_kind = read_u32_le(payload, cursor);
        let owner_id = read_u32_le(payload, cursor + 4);
        let value_kind = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        let key_offset = read_u32_le(payload, cursor + 16) as usize;
        let key_len = read_u32_le(payload, cursor + 20) as usize;
        let token_id = read_u32_le(payload, cursor + 24);
        let date_hour_raw = read_i32_le(payload, cursor + 28);
        let _value_raw = read_i64_le(payload, cursor + 32);
        let reserved_row0 = read_u32_le(payload, cursor + 40);
        let reserved_row1 = read_u32_le(payload, cursor + 44);

        if flags & !KNOWN_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} has unknown flags {flags}"
            ));
        }
        validate_scope(offset, row_index, scope_kind, owner_id, &mut summary);
        validate_value_kind(offset, row_index, value_kind, flags, token_id, &mut summary);
        if reserved_row0 != 0 || reserved_row1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} reserved fields are nonzero"
            ));
        }
        if flags & ROW_FLAG_KEY_PRESENT != 0 {
            if key_len == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state row {row_index} sets key flag with zero key length"
                ));
            }
        } else if key_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} has key bytes without key flag"
            ));
        }
        if key_len > MAX_KEY_BYTES {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} key is {key_len} byte(s), max {MAX_KEY_BYTES}"
            ));
        }
        let mut key_bytes_for_row = None;
        if key_offset
            .checked_add(key_len)
            .map_or(true, |end| end > key_table.len())
        {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} key range is outside key table"
            ));
        } else if key_len != 0 {
            let key_bytes = &key_table[key_offset..key_offset + key_len];
            key_bytes_for_row = Some(key_bytes);
            if !is_printable_key(key_bytes) {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state row {row_index} key has non-printable bytes"
                ));
            }
        }
        if flags & ROW_FLAG_TOKEN_PRESENT != 0 {
            if token_id == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state row {row_index} sets token flag with token_id 0"
                ));
            }
        } else if token_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} has token_id without token flag"
            ));
        }
        if flags & ROW_FLAG_DATE_PRESENT == 0 && date_hour_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} has date value without date flag"
            ));
        }

        if value_kind == VALUE_VARIABLE_FIXED_I64 {
            decoded_variable_rows = decoded_variable_rows.saturating_add(1);
        }
        if value_kind == VALUE_FLAG_NAMED || value_kind == VALUE_FLAG_TOKEN_ONLY {
            decoded_flag_rows = decoded_flag_rows.saturating_add(1);
        }
        if key_len == 0 && token_id != 0 {
            decoded_token_only_rows = decoded_token_only_rows.saturating_add(1);
        }
        if value_kind == VALUE_DELAYED_EVENT_OPAQUE {
            decoded_delayed_event_rows = decoded_delayed_event_rows.saturating_add(1);
            validate_opaque_badge_key(
                offset,
                row_index,
                "delayed-event",
                key_bytes_for_row,
                DELAYED_EVENTS_BADGE_KEY,
                &mut summary,
            );
        }
        if value_kind == VALUE_SCRIPTED_GUI_OPAQUE {
            decoded_scripted_gui_rows = decoded_scripted_gui_rows.saturating_add(1);
            validate_opaque_badge_key(
                offset,
                row_index,
                "scripted-GUI",
                key_bytes_for_row,
                SCRIPTED_GUI_BADGE_KEY,
                &mut summary,
            );
        }
        let row_key = (
            scope_kind,
            owner_id,
            value_kind,
            key_offset as u32,
            key_len as u32,
            token_id,
        );
        if !seen_rows.insert(row_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted state row {row_index} duplicates scope/value/key identity"
            ));
        }
    }

    if decoded_variable_rows != variable_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state variable_rows {variable_rows} != decoded {decoded_variable_rows}"
        ));
    }
    if decoded_flag_rows != flag_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state flag_rows {flag_rows} != decoded {decoded_flag_rows}"
        ));
    }
    if decoded_token_only_rows != token_only_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state token_only_rows {token_only_rows} != decoded {decoded_token_only_rows}"
        ));
    }
    if token_only_rows > 0 && status_flags & STATUS_FLAG_NAMES_PARTIAL == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state token-only flag rows require STATUS_FLAG_NAMES_PARTIAL"
        ));
    }
    if decoded_delayed_event_rows > 0 && status_flags & STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed-event opaque rows require STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED"
        ));
    }
    if status_flags & STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED != 0 && decoded_delayed_event_rows == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED set without delayed-event opaque badge row"
        ));
    }
    if decoded_scripted_gui_rows > 0 && status_flags & STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted-GUI opaque rows require STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED"
        ));
    }
    if status_flags & STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED != 0 && decoded_scripted_gui_rows == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED set without scripted-GUI opaque badge row"
        ));
    }
    add_unit(
        &mut summary,
        "scripted_delayed_event_opaque_rows",
        decoded_delayed_event_rows,
    );
    add_unit(
        &mut summary,
        "scripted_gui_opaque_rows",
        decoded_scripted_gui_rows,
    );
    summary
}

fn validate_opaque_badge_key(
    offset: usize,
    row_index: usize,
    badge_kind: &str,
    key_bytes: Option<&[u8]>,
    expected_key: &str,
    summary: &mut Domain51GateSummary,
) {
    if key_bytes != Some(expected_key.as_bytes()) {
        summary.parse_errors.push(format!(
            "offset {offset}: scripted state {badge_kind} opaque row {row_index} has wrong badge key"
        ));
    }
}

fn validate_scope(
    offset: usize,
    row_index: usize,
    scope_kind: u32,
    owner_id: u32,
    summary: &mut Domain51GateSummary,
) {
    match scope_kind {
        SCOPE_GLOBAL => {
            if owner_id != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state global row {row_index} has owner_id {owner_id}"
                ));
            }
        }
        SCOPE_COUNTRY => {
            if owner_id == 0 || owner_id > MAX_COUNTRY_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state country row {row_index} has invalid owner_id {owner_id}"
                ));
            }
        }
        SCOPE_STATE => {
            if owner_id == 0 || owner_id > MAX_STATE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state state row {row_index} has invalid owner_id {owner_id}"
                ));
            }
        }
        _ => summary.parse_errors.push(format!(
            "offset {offset}: scripted state row {row_index} has unknown scope_kind {scope_kind}"
        )),
    }
}

fn validate_value_kind(
    offset: usize,
    row_index: usize,
    value_kind: u32,
    flags: u32,
    token_id: u32,
    summary: &mut Domain51GateSummary,
) {
    match value_kind {
        VALUE_VARIABLE_FIXED_I64 => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 || flags & ROW_FLAG_VALUE_PRESENT == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state variable row {row_index} lacks key/value"
                ));
            }
            if token_id != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state variable row {row_index} has token_id {token_id}"
                ));
            }
        }
        VALUE_FLAG_NAMED => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state named flag row {row_index} lacks key"
                ));
            }
        }
        VALUE_FLAG_TOKEN_ONLY => {
            if flags & ROW_FLAG_TOKEN_PRESENT == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state token flag row {row_index} lacks token"
                ));
            }
        }
        VALUE_DELAYED_EVENT_OPAQUE | VALUE_SCRIPTED_GUI_OPAQUE => {
            if flags & ROW_FLAG_KEY_PRESENT == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: scripted state opaque row {row_index} lacks key"
                ));
            }
        }
        _ => summary.parse_errors.push(format!(
            "offset {offset}: scripted state row {row_index} has unknown value_kind {value_kind}"
        )),
    }
}

fn add_unit(summary: &mut Domain51GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn is_printable_key(bytes: &[u8]) -> bool {
    !bytes.is_empty() && bytes.iter().all(|b| (32..=126).contains(b))
}

fn read_u32_le(payload: &[u8], offset: usize) -> u32 {
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&payload[offset..offset + 4]);
    u32::from_le_bytes(arr)
}

fn read_i32_le(payload: &[u8], offset: usize) -> i32 {
    let mut arr = [0u8; 4];
    arr.copy_from_slice(&payload[offset..offset + 4]);
    i32::from_le_bytes(arr)
}

fn read_i64_le(payload: &[u8], offset: usize) -> i64 {
    let mut arr = [0u8; 8];
    arr.copy_from_slice(&payload[offset..offset + 8]);
    i64::from_le_bytes(arr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_country_variables_and_token_flags() {
        let rows = vec![
            ScriptedStateRow {
                scope_kind: SCOPE_COUNTRY,
                owner_id: 1,
                value_kind: VALUE_VARIABLE_FIXED_I64,
                key: Some("usa_congress_senate_support".to_string()),
                token_id: None,
                value_raw: Some(5_250_000),
                date_hour_raw: None,
                deleted: false,
            },
            ScriptedStateRow {
                scope_kind: SCOPE_COUNTRY,
                owner_id: 1,
                value_kind: VALUE_VARIABLE_FIXED_I64,
                key: Some("core_states_at_game_start^0".to_string()),
                token_id: None,
                value_raw: Some(-1_073_741_542),
                date_hour_raw: None,
                deleted: false,
            },
            ScriptedStateRow {
                scope_kind: SCOPE_COUNTRY,
                owner_id: 1,
                value_kind: VALUE_FLAG_TOKEN_ONLY,
                key: None,
                token_id: Some(0xB5E5),
                value_raw: None,
                date_hour_raw: None,
                deleted: false,
            },
        ];
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 8,
            status_flags: STATUS_DELTA_DATA | STATUS_PARTIAL_DATA | STATUS_FLAG_NAMES_PARTIAL,
            changed_rows: 3,
            observed_scopes: 2,
            measured_scopes: 1,
            rejected_scopes: 1,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(128, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.status_flags_or,
            STATUS_DELTA_DATA | STATUS_PARTIAL_DATA | STATUS_FLAG_NAMES_PARTIAL
        );
        assert_eq!(summary.data_rows_total, 3);
        assert_eq!(summary.data_changed_rows_total, 3);
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_variable_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_token_only_flag_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn accepts_empty_measured_frame() {
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 0,
            observed_scopes: 143,
            measured_scopes: 143,
            rejected_scopes: 0,
        };
        let payload = encode_payload(&header, &[]);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 0);
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_state_measured_scopes")
                .copied(),
            Some(143)
        );
    }

    #[test]
    fn rejects_invalid_scope_and_key_bytes() {
        let rows = vec![ScriptedStateRow {
            scope_kind: SCOPE_COUNTRY,
            owner_id: 0,
            value_kind: VALUE_VARIABLE_FIXED_I64,
            key: Some("bad\nkey".to_string()),
            token_id: None,
            value_raw: Some(1),
            date_hour_raw: None,
            deleted: false,
        }];
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            observed_scopes: 1,
            measured_scopes: 1,
            rejected_scopes: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("invalid owner_id 0")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("non-printable bytes")));
    }

    #[test]
    fn rejects_payload_len_mismatch() {
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 0,
            observed_scopes: 0,
            measured_scopes: 0,
            rejected_scopes: 0,
        };
        let mut payload = encode_payload(&header, &[]);
        payload.push(1);
        let summary = decode_payload(9, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("expected 64")));
    }

    #[test]
    fn accepts_fail_closed_badges_and_opaque_rows() {
        let rows = vec![
            ScriptedStateRow {
                scope_kind: SCOPE_GLOBAL,
                owner_id: 0,
                value_kind: VALUE_DELAYED_EVENT_OPAQUE,
                key: Some(DELAYED_EVENTS_BADGE_KEY.to_string()),
                token_id: None,
                value_raw: None,
                date_hour_raw: None,
                deleted: false,
            },
            ScriptedStateRow {
                scope_kind: SCOPE_GLOBAL,
                owner_id: 0,
                value_kind: VALUE_SCRIPTED_GUI_OPAQUE,
                key: Some(SCRIPTED_GUI_BADGE_KEY.to_string()),
                token_id: None,
                value_raw: None,
                date_hour_raw: None,
                deleted: false,
            },
        ];
        let status_flags = STATUS_SOURCE_UNAVAILABLE
            | STATUS_PARTIAL_DATA
            | STATUS_FLAG_NAMES_PARTIAL
            | STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
            | STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 12,
            status_flags,
            changed_rows: 2,
            observed_scopes: 4,
            measured_scopes: 2,
            rejected_scopes: 2,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(44, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.status_flags_or, status_flags);
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_delayed_event_opaque_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("scripted_gui_opaque_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_unresolved_status_without_opaque_badge_row() {
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            changed_rows: 0,
            observed_scopes: 1,
            measured_scopes: 0,
            rejected_scopes: 1,
        };
        let payload = encode_payload(&header, &[]);
        let summary = decode_payload(12, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains(
                "STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED set without delayed-event opaque badge row",
            )
        }));
    }

    #[test]
    fn rejects_opaque_badge_row_without_status_bit() {
        let rows = vec![ScriptedStateRow {
            scope_kind: SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: VALUE_SCRIPTED_GUI_OPAQUE,
            key: Some(SCRIPTED_GUI_BADGE_KEY.to_string()),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
        }];
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            observed_scopes: 1,
            measured_scopes: 0,
            rejected_scopes: 1,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(13, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("scripted-GUI opaque rows require STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED")
        }));
    }

    #[test]
    fn rejects_opaque_badge_row_with_wrong_key() {
        let rows = vec![ScriptedStateRow {
            scope_kind: SCOPE_GLOBAL,
            owner_id: 0,
            value_kind: VALUE_DELAYED_EVENT_OPAQUE,
            key: Some("root_unresolved:wrong".to_string()),
            token_id: None,
            value_raw: None,
            date_hour_raw: None,
            deleted: false,
        }];
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED,
            changed_rows: 1,
            observed_scopes: 1,
            measured_scopes: 0,
            rejected_scopes: 1,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(14, &payload);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("delayed-event opaque row 0 has wrong badge key")));
    }

    #[test]
    fn rejects_token_only_flags_without_partial_names_status() {
        let rows = vec![ScriptedStateRow {
            scope_kind: SCOPE_COUNTRY,
            owner_id: 1,
            value_kind: VALUE_FLAG_TOKEN_ONLY,
            key: None,
            token_id: Some(0xB5E5),
            value_raw: Some(1),
            date_hour_raw: Some(13_443),
            deleted: false,
        }];
        let header = ScriptedStateHeader {
            elapsed_wall_ms: 1,
            status_flags: 0,
            changed_rows: 1,
            observed_scopes: 1,
            measured_scopes: 1,
            rejected_scopes: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(15, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("token-only flag rows require STATUS_FLAG_NAMES_PARTIAL")
        }));
    }
}
