use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 46;
pub const DOMAIN_NAME: &str = "air_variant_archetypes_live";
pub const RECORD_SCOPE: &str = "air_ace_roster_identity";
pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_ACE_ROSTER_DATA: u16 = 0xACE1;
pub const HEADER_LEN: usize = 48;
pub const ROW_LEN: usize = 32;

pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_WING_ASSIGNMENT_BLOCKED: u32 = 1 << 11;

pub const ROW_FLAG_DELETED: u32 = 1;
pub const ROW_FLAG_NAME: u32 = 1 << 1;
pub const ROW_FLAG_SURNAME: u32 = 1 << 2;
pub const ROW_FLAG_CALLSIGN: u32 = 1 << 3;
pub const ROW_FLAG_WING_ID: u32 = 1 << 4;

pub const SOURCE_LIVE_COUNTRY_ACE_VECTOR: u32 = 1;
pub const SOURCE_LIVE_ACE_OBJECT_LAYOUT: u32 = 1 << 1;
pub const SOURCE_DEV_SAVE_LAYOUT_VALIDATION: u32 = 1 << 2;
pub const SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED: u32 = 1 << 3;
pub const SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT: u32 = 1 << 4;

pub const MAX_TAG_DB_ID: u32 = 4096;
pub const MAX_ACE_ID: u32 = i32::MAX as u32;
pub const MAX_COUNTRY_ACE_INDEX: u32 = 4096;
pub const MAX_WING_ID: u32 = i32::MAX as u32;
pub const MAX_STRING_LEN: usize = 96;

const KNOWN_ROW_FLAGS: u32 =
    ROW_FLAG_DELETED | ROW_FLAG_NAME | ROW_FLAG_SURNAME | ROW_FLAG_CALLSIGN | ROW_FLAG_WING_ID;
const KNOWN_SOURCE_MASK: u32 = SOURCE_LIVE_COUNTRY_ACE_VECTOR
    | SOURCE_LIVE_ACE_OBJECT_LAYOUT
    | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
    | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED
    | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirAceRosterHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_country_rows: u32,
    pub observed_ace_rows: u32,
    pub identity_rows: u32,
    pub wing_assignment_rows: u32,
    pub blocked_assignment_rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirAceRosterRow {
    pub tag_db_id: u32,
    pub ace_id: u32,
    pub deleted: bool,
    pub country_ace_index: u32,
    pub wing_id: Option<u32>,
    pub source_mask: u32,
    pub name: Option<String>,
    pub surname: Option<String>,
    pub callsign: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirAceRosterGateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompactRecordHeader {
    pub sequence: u64,
    pub game_hour_raw: i32,
    pub domain_id: u16,
    pub record_type: u16,
    pub payload_len: u32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirAceRosterRecordGateSummary {
    pub record_count: u64,
    pub payload_bytes_total: u64,
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_compact_data_record(sequence: u64, game_hour_raw: i32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(COMPACT_RECORD_HEADER_LEN.saturating_add(payload.len()));
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&DOMAIN_ID.to_le_bytes());
    out.extend_from_slice(&COMPACT_RECORD_TYPE_ACE_ROSTER_DATA.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

pub fn decode_compact_data_record(offset: usize, bytes: &[u8]) -> AirAceRosterRecordGateSummary {
    let mut summary = AirAceRosterRecordGateSummary::default();
    let Some((header, payload)) = parse_compact_record(offset, bytes, &mut summary) else {
        return summary;
    };
    summary.record_count = 1;
    summary.payload_bytes_total = payload.len() as u64;

    if header.domain_id != DOMAIN_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: domain_id {} != {DOMAIN_ID}",
            header.domain_id
        ));
        return summary;
    }
    if header.record_type != COMPACT_RECORD_TYPE_ACE_ROSTER_DATA {
        summary.parse_errors.push(format!(
            "offset {offset}: record_type {} != {COMPACT_RECORD_TYPE_ACE_ROSTER_DATA}",
            header.record_type
        ));
        return summary;
    }

    merge_payload_summary(
        &mut summary,
        decode_payload(offset + COMPACT_RECORD_HEADER_LEN, payload),
    );
    summary
}

pub fn encode_payload(header: &AirAceRosterHeader, rows: &[AirAceRosterRow]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        HEADER_LEN
            .saturating_add(rows.len().saturating_mul(ROW_LEN))
            .saturating_add(rows.len().saturating_mul(32)),
    );
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.observed_country_rows.to_le_bytes());
    payload.extend_from_slice(&header.observed_ace_rows.to_le_bytes());
    payload.extend_from_slice(&header.identity_rows.to_le_bytes());
    payload.extend_from_slice(&header.wing_assignment_rows.to_le_bytes());
    payload.extend_from_slice(&header.blocked_assignment_rows.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    for row in rows {
        let mut flags = 0u32;
        if row.deleted {
            flags |= ROW_FLAG_DELETED;
        }
        if row.name.is_some() {
            flags |= ROW_FLAG_NAME;
        }
        if row.surname.is_some() {
            flags |= ROW_FLAG_SURNAME;
        }
        if row.callsign.is_some() {
            flags |= ROW_FLAG_CALLSIGN;
        }
        if row.wing_id.is_some() {
            flags |= ROW_FLAG_WING_ID;
        }
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.ace_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.source_mask.to_le_bytes());
        payload.extend_from_slice(&row.country_ace_index.to_le_bytes());
        payload.extend_from_slice(&row.wing_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }

    for row in rows {
        append_compact_string(&mut payload, row.name.as_deref());
        append_compact_string(&mut payload, row.surname.as_deref());
        append_compact_string(&mut payload, row.callsign.as_deref());
    }

    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> AirAceRosterGateSummary {
    let mut summary = AirAceRosterGateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_country_rows = read_u32_le(payload, 20) as usize;
    let observed_ace_rows = read_u32_le(payload, 24) as usize;
    let identity_rows = read_u32_le(payload, 28) as usize;
    let wing_assignment_rows = read_u32_le(payload, 32) as usize;
    let blocked_assignment_rows = read_u32_le(payload, 36) as usize;
    let reserved0 = read_u32_le(payload, 40);
    let reserved1 = read_u32_le(payload, 44);
    summary.status_flags_or |= status_flags;

    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if identity_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster identity_rows {identity_rows} > row_count {row_count}"
        ));
    }
    if wing_assignment_rows > row_count || blocked_assignment_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster assignment counters exceed row_count {row_count}"
        ));
    }
    if row_count > observed_ace_rows && (status_flags & STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster row_count {row_count} > observed_ace_rows {observed_ace_rows}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster header reserved fields are nonzero"
        ));
    }

    let rows_end = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() < rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster payload is {} byte(s), expected at least {rows_end}",
            payload.len()
        ));
        return summary;
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "air_ace_roster_rows", row_count);
    add_unit(
        &mut summary,
        "air_ace_roster_observed_country_rows",
        observed_country_rows,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_observed_ace_rows",
        observed_ace_rows,
    );
    add_unit(&mut summary, "air_ace_roster_identity_rows", identity_rows);
    add_unit(
        &mut summary,
        "air_ace_roster_wing_assignment_rows",
        wing_assignment_rows,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_blocked_assignment_rows",
        blocked_assignment_rows,
    );

    let mut cursor = HEADER_LEN;
    let mut seen_aces = BTreeSet::new();
    let mut row_headers = Vec::with_capacity(row_count);
    let mut decoded_identity_rows = 0usize;
    let mut decoded_assignment_rows = 0usize;
    let mut deleted_rows = 0usize;
    let mut layout_validated_rows = 0usize;
    let mut assignment_validated_rows = 0usize;
    let mut assignment_blocked_rows = 0usize;
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let ace_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let source_mask = read_u32_le(payload, cursor + 12);
        let country_ace_index = read_u32_le(payload, cursor + 16);
        let wing_id = read_u32_le(payload, cursor + 20);
        let row_reserved0 = read_u32_le(payload, cursor + 24);
        let row_reserved1 = read_u32_le(payload, cursor + 28);
        cursor += ROW_LEN;

        let deleted = flags & ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
        }
        if flags & !KNOWN_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has unknown flags {flags}"
            ));
        }
        if source_mask == 0 && !deleted {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has empty source_mask"
            ));
        }
        if source_mask & !KNOWN_SOURCE_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has unknown source_mask {source_mask}"
            ));
        }
        if tag_db_id == 0 || tag_db_id > MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if ace_id == 0 || ace_id > MAX_ACE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has invalid ace_id {ace_id}"
            ));
        } else if !seen_aces.insert((tag_db_id, ace_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} duplicates tag_db_id {tag_db_id} ace_id {ace_id}"
            ));
        }
        if country_ace_index > MAX_COUNTRY_ACE_INDEX {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has invalid country_ace_index {country_ace_index}"
            ));
        }
        if flags & ROW_FLAG_WING_ID != 0 {
            if wing_id == 0 || wing_id > MAX_WING_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air ace roster row {row_index} has invalid wing_id {wing_id}"
                ));
            } else {
                decoded_assignment_rows = decoded_assignment_rows.saturating_add(1);
            }
        } else if wing_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} has wing_id without flag"
            ));
        }
        if source_mask & SOURCE_LIVE_ACE_OBJECT_LAYOUT != 0 {
            layout_validated_rows = layout_validated_rows.saturating_add(1);
        }
        if source_mask & SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT != 0 {
            assignment_validated_rows = assignment_validated_rows.saturating_add(1);
        }
        if source_mask & SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED != 0 {
            assignment_blocked_rows = assignment_blocked_rows.saturating_add(1);
        }
        if row_reserved0 != 0 || row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster row {row_index} reserved fields are nonzero"
            ));
        }
        row_headers.push((flags, deleted));
    }

    let mut name_rows = 0usize;
    let mut surname_rows = 0usize;
    let mut callsign_rows = 0usize;
    for (row_index, (flags, deleted)) in row_headers.iter().enumerate() {
        let name_len = match skip_compact_string_len(payload, &mut cursor, "air ace name") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return summary;
            }
        };
        let surname_len = match skip_compact_string_len(payload, &mut cursor, "air ace surname") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return summary;
            }
        };
        let callsign_len = match skip_compact_string_len(payload, &mut cursor, "air ace callsign") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return summary;
            }
        };
        validate_flagged_string(
            &mut summary,
            offset,
            row_index,
            "name",
            name_len,
            *flags,
            ROW_FLAG_NAME,
        );
        validate_flagged_string(
            &mut summary,
            offset,
            row_index,
            "surname",
            surname_len,
            *flags,
            ROW_FLAG_SURNAME,
        );
        validate_flagged_string(
            &mut summary,
            offset,
            row_index,
            "callsign",
            callsign_len,
            *flags,
            ROW_FLAG_CALLSIGN,
        );
        if *deleted && (name_len != 0 || surname_len != 0 || callsign_len != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air ace roster deleted row {row_index} carries strings"
            ));
        }
        if name_len > 0 {
            name_rows = name_rows.saturating_add(1);
        }
        if surname_len > 0 {
            surname_rows = surname_rows.saturating_add(1);
        }
        if callsign_len > 0 {
            callsign_rows = callsign_rows.saturating_add(1);
        }
        if name_len > 0 || surname_len > 0 || callsign_len > 0 {
            decoded_identity_rows = decoded_identity_rows.saturating_add(1);
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    if decoded_identity_rows != identity_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster identity_rows {identity_rows} != decoded {decoded_identity_rows}"
        ));
    }
    if decoded_assignment_rows != wing_assignment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster wing_assignment_rows {wing_assignment_rows} != decoded {decoded_assignment_rows}"
        ));
    }
    if assignment_blocked_rows != blocked_assignment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster blocked_assignment_rows {blocked_assignment_rows} != decoded {assignment_blocked_rows}"
        ));
    }

    add_unit(&mut summary, "air_ace_roster_deleted_rows", deleted_rows);
    add_unit(&mut summary, "air_ace_roster_name_rows", name_rows);
    add_unit(&mut summary, "air_ace_roster_surname_rows", surname_rows);
    add_unit(&mut summary, "air_ace_roster_callsign_rows", callsign_rows);
    add_unit(
        &mut summary,
        "air_ace_roster_live_layout_validated_rows",
        layout_validated_rows,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_live_wing_assignment_validated_rows",
        assignment_validated_rows,
    );
    add_unit(
        &mut summary,
        "air_ace_roster_wing_assignment_scan_blocked_rows",
        assignment_blocked_rows,
    );

    summary
}

fn validate_flagged_string(
    summary: &mut AirAceRosterGateSummary,
    offset: usize,
    row_index: usize,
    label: &str,
    len: usize,
    flags: u32,
    flag: u32,
) {
    if len > MAX_STRING_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster row {row_index} {label} string is too long ({len})"
        ));
    }
    if len == 0 && flags & flag != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster row {row_index} has {label} flag with empty string"
        ));
    }
    if len > 0 && flags & flag == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air ace roster row {row_index} has {label} string without flag"
        ));
    }
}

fn add_unit(summary: &mut AirAceRosterGateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
}

fn parse_compact_record<'a>(
    offset: usize,
    bytes: &'a [u8],
    summary: &mut AirAceRosterRecordGateSummary,
) -> Option<(CompactRecordHeader, &'a [u8])> {
    if bytes.len() < COMPACT_RECORD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: truncated H4CR header with {} byte(s)",
            bytes.len()
        ));
        return None;
    }
    if &bytes[0..4] != COMPACT_RECORD_MAGIC {
        summary
            .parse_errors
            .push(format!("offset {offset}: invalid H4CR magic"));
        return None;
    }
    let version = read_u16_le(bytes, 4);
    if version != COMPACT_RECORD_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported H4CR version {version}"
        ));
    }
    let header_len = read_u16_le(bytes, 6) as usize;
    if header_len != COMPACT_RECORD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported H4CR header length {header_len}"
        ));
        return None;
    }
    let payload_len = read_u32_le(bytes, 24) as usize;
    let expected_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
    if bytes.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: record length {} != header {COMPACT_RECORD_HEADER_LEN} + payload {payload_len}",
            bytes.len()
        ));
        return None;
    }
    let header = CompactRecordHeader {
        sequence: read_u64_le(bytes, 8),
        game_hour_raw: read_i32_le(bytes, 16),
        domain_id: read_u16_le(bytes, 20),
        record_type: read_u16_le(bytes, 22),
        payload_len: payload_len as u32,
        flags: read_u32_le(bytes, 28),
    };
    Some((header, &bytes[COMPACT_RECORD_HEADER_LEN..]))
}

fn merge_payload_summary(
    record: &mut AirAceRosterRecordGateSummary,
    payload: AirAceRosterGateSummary,
) {
    record.status_flags_or |= payload.status_flags_or;
    record.data_rows_total = record
        .data_rows_total
        .saturating_add(payload.data_rows_total);
    record.data_changed_rows_total = record
        .data_changed_rows_total
        .saturating_add(payload.data_changed_rows_total);
    for (key, value) in payload.data_units_total {
        let entry = record.data_units_total.entry(key).or_default();
        *entry = entry.saturating_add(value);
    }
    record.parse_errors.extend(payload.parse_errors);
}

fn append_compact_string(out: &mut Vec<u8>, value: Option<&str>) {
    let bytes = value.unwrap_or("").as_bytes();
    let len = bytes.len().min(u16::MAX as usize);
    out.extend_from_slice(&(len as u16).to_le_bytes());
    out.extend_from_slice(&bytes[..len]);
}

fn skip_compact_string_len(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    if bytes.len().saturating_sub(*cursor) < 2 {
        return Err(format!("{label} string is truncated before length"));
    }
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    if bytes.len().saturating_sub(*cursor) < len {
        return Err(format!(
            "{label} string length {len} exceeds remaining payload"
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(ace_id: u32) -> AirAceRosterRow {
        AirAceRosterRow {
            tag_db_id: 1,
            ace_id,
            deleted: false,
            country_ace_index: ace_id - 5000,
            wing_id: None,
            source_mask: SOURCE_LIVE_COUNTRY_ACE_VECTOR
                | SOURCE_LIVE_ACE_OBJECT_LAYOUT
                | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
                | SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED,
            name: Some("Tobias".to_string()),
            surname: Some("Ochsenhirt".to_string()),
            callsign: Some("Fighter of Libau".to_string()),
        }
    }

    #[test]
    fn decodes_ace_identity_rows_with_blocked_assignment() {
        let rows = vec![sample_row(5440), sample_row(6265)];
        let header = AirAceRosterHeader {
            elapsed_wall_ms: 500,
            status_flags: STATUS_DELTA_DATA | STATUS_WING_ASSIGNMENT_BLOCKED,
            changed_rows: 2,
            observed_country_rows: 439,
            observed_ace_rows: 127,
            identity_rows: 2,
            wing_assignment_rows: 0,
            blocked_assignment_rows: 2,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.data_rows_total, 2);
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_live_layout_validated_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_wing_assignment_scan_blocked_rows")
                .copied(),
            Some(2)
        );
    }

    #[test]
    fn decodes_ace_identity_rows_with_wing_assignment() {
        let mut row = sample_row(5440);
        row.wing_id = Some(290);
        row.source_mask = SOURCE_LIVE_COUNTRY_ACE_VECTOR
            | SOURCE_LIVE_ACE_OBJECT_LAYOUT
            | SOURCE_DEV_SAVE_LAYOUT_VALIDATION
            | SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;
        let header = AirAceRosterHeader {
            elapsed_wall_ms: 750,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_country_rows: 439,
            observed_ace_rows: 147,
            identity_rows: 1,
            wing_assignment_rows: 1,
            blocked_assignment_rows: 0,
        };
        let payload = encode_payload(&header, &[row]);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_wing_assignment_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_live_wing_assignment_validated_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn wraps_and_decodes_compact_data_record() {
        let mut row = sample_row(5440);
        row.wing_id = Some(290);
        row.source_mask |= SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;
        row.source_mask &= !SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED;
        let header = AirAceRosterHeader {
            elapsed_wall_ms: 750,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_country_rows: 439,
            observed_ace_rows: 147,
            identity_rows: 1,
            wing_assignment_rows: 1,
            blocked_assignment_rows: 0,
        };
        let payload = encode_payload(&header, &[row]);
        let record = encode_compact_data_record(7, 1942110101, &payload);
        assert_eq!(&record[0..4], COMPACT_RECORD_MAGIC);
        assert_eq!(
            u16::from_le_bytes(record[20..22].try_into().unwrap()),
            DOMAIN_ID
        );
        let summary = decode_compact_data_record(0, &record);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.record_count, 1);
        assert_eq!(summary.data_rows_total, 1);
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_live_wing_assignment_validated_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn compact_record_rejects_wrong_domain() {
        let row = sample_row(5440);
        let header = AirAceRosterHeader {
            elapsed_wall_ms: 750,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_country_rows: 439,
            observed_ace_rows: 147,
            identity_rows: 1,
            wing_assignment_rows: 0,
            blocked_assignment_rows: 0,
        };
        let payload = encode_payload(&header, &[row]);
        let mut record = encode_compact_data_record(7, 1942110101, &payload);
        record[20..22].copy_from_slice(&45u16.to_le_bytes());
        let summary = decode_compact_data_record(0, &record);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("domain_id 45 != 46")));
    }

    #[test]
    fn rejects_wing_id_without_flag() {
        let mut row = sample_row(5440);
        row.wing_id = Some(357);
        let header = AirAceRosterHeader {
            elapsed_wall_ms: 500,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_country_rows: 1,
            observed_ace_rows: 1,
            identity_rows: 1,
            wing_assignment_rows: 1,
            blocked_assignment_rows: 0,
        };
        let mut payload = encode_payload(&header, &[row]);
        let flags_offset = HEADER_LEN + 8;
        let mut flags = read_u32_le(&payload, flags_offset);
        flags &= !ROW_FLAG_WING_ID;
        payload[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let summary = decode_payload(17, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("wing_id without flag")));
    }
}
