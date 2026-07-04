use std::collections::{BTreeMap, BTreeSet};

pub const DOMAIN_ID: u16 = 46;
pub const DOMAIN_NAME: &str = "air_variant_archetypes_live";
pub const HEADER_LEN: usize = 48;
pub const ROW_LEN: usize = 32;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_DELTA_DATA: u32 = 1 << 10;

pub const ROW_FLAG_DELETED: u32 = 1;
pub const ROW_FLAG_ARCHETYPE: u32 = 1 << 1;
pub const ROW_FLAG_BASE_AIRFRAME: u32 = 1 << 2;
pub const ROW_FLAG_ROLE: u32 = 1 << 3;
pub const ROW_FLAG_AIR_SUPERIORITY_WEIGHT: u32 = 1 << 4;

pub const SOURCE_LIVE_STATS: u32 = 1;
pub const SOURCE_LIVE_DEFINITION: u32 = 1 << 1;
pub const SOURCE_STATIC_CATALOG: u32 = 1 << 2;
pub const SOURCE_DEV_SAVE_JOIN_REJECTED: u32 = 1 << 3;
pub const SOURCE_STATIC_HEURISTIC: u32 = 1 << 4;
pub const SOURCE_LIVE_TOKEN_DICTIONARY: u32 = 1 << 5;
pub const SOURCE_STATIC_SOURCE_ENUM: u32 = 1 << 6;

pub const MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const MAX_EQUIPMENT_TOKEN_RAW: u32 = 1_000_000;
pub const MAX_EQUIPMENT_TYPE_TOKEN_RAW: u32 = 1_000_000;
pub const MAX_INTERFACE_TYPE_RAW: u32 = 1_000_000;
pub const MAX_STRING_LEN: usize = 160;
pub const MAX_AIR_SUPERIORITY_WEIGHT_RAW: i32 = 10 * FIXED_SCALE_RAW as i32;

const KNOWN_ROW_FLAGS: u32 = ROW_FLAG_DELETED
    | ROW_FLAG_ARCHETYPE
    | ROW_FLAG_BASE_AIRFRAME
    | ROW_FLAG_ROLE
    | ROW_FLAG_AIR_SUPERIORITY_WEIGHT;
const KNOWN_SOURCE_MASK: u32 = SOURCE_LIVE_STATS
    | SOURCE_LIVE_DEFINITION
    | SOURCE_STATIC_CATALOG
    | SOURCE_DEV_SAVE_JOIN_REJECTED
    | SOURCE_STATIC_HEURISTIC
    | SOURCE_LIVE_TOKEN_DICTIONARY
    | SOURCE_STATIC_SOURCE_ENUM;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirVariantArchetypeHeader {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: u32,
    pub observed_variant_rows: u32,
    pub classified_rows: u32,
    pub unresolved_rows: u32,
    pub static_catalog_hash: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AirVariantArchetypeRow {
    pub variant_id: u32,
    pub deleted: bool,
    pub equipment_token_raw: u32,
    pub equipment_type_token_raw: u32,
    pub interface_type_raw: u32,
    pub air_superiority_weight_raw: Option<i32>,
    pub source_mask: u32,
    pub archetype_token: Option<String>,
    pub base_airframe_token: Option<String>,
    pub role_token: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AirVariantArchetypeGateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(
    header: &AirVariantArchetypeHeader,
    rows: &[AirVariantArchetypeRow],
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(
        HEADER_LEN
            .saturating_add(rows.len().saturating_mul(ROW_LEN))
            .saturating_add(rows.len().saturating_mul(24)),
    );
    payload.extend_from_slice(&header.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&header.changed_rows.to_le_bytes());
    payload.extend_from_slice(&header.status_flags.to_le_bytes());
    payload.extend_from_slice(&header.observed_variant_rows.to_le_bytes());
    payload.extend_from_slice(&header.classified_rows.to_le_bytes());
    payload.extend_from_slice(&header.unresolved_rows.to_le_bytes());
    payload.extend_from_slice(&header.static_catalog_hash.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());

    for row in rows {
        let mut flags = 0u32;
        if row.deleted {
            flags |= ROW_FLAG_DELETED;
        }
        if row.archetype_token.is_some() {
            flags |= ROW_FLAG_ARCHETYPE;
        }
        if row.base_airframe_token.is_some() {
            flags |= ROW_FLAG_BASE_AIRFRAME;
        }
        if row.role_token.is_some() {
            flags |= ROW_FLAG_ROLE;
        }
        if row.air_superiority_weight_raw.is_some() {
            flags |= ROW_FLAG_AIR_SUPERIORITY_WEIGHT;
        }
        payload.extend_from_slice(&row.variant_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.equipment_token_raw.to_le_bytes());
        payload.extend_from_slice(&row.equipment_type_token_raw.to_le_bytes());
        payload.extend_from_slice(&row.interface_type_raw.to_le_bytes());
        payload.extend_from_slice(&row.air_superiority_weight_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.source_mask.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }

    for row in rows {
        append_compact_string(&mut payload, row.archetype_token.as_deref());
        append_compact_string(&mut payload, row.base_airframe_token.as_deref());
        append_compact_string(&mut payload, row.role_token.as_deref());
    }

    payload
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> AirVariantArchetypeGateSummary {
    let mut summary = AirVariantArchetypeGateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let classified_rows = read_u32_le(payload, 24) as usize;
    let unresolved_rows = read_u32_le(payload, 28) as usize;
    let static_catalog_hash = read_u32_le(payload, 32);
    let fixed_scale_raw = read_u32_le(payload, 36);
    let reserved0 = read_u32_le(payload, 40);
    let reserved1 = read_u32_le(payload, 44);
    summary.status_flags_or |= status_flags;

    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if classified_rows.saturating_add(unresolved_rows) != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes classified+unresolved {} != row_count {row_count}",
            classified_rows.saturating_add(unresolved_rows)
        ));
    }
    if fixed_scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes fixed scale {fixed_scale_raw} != {FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes header reserved fields are nonzero"
        ));
    }

    let rows_end = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if payload.len() < rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes payload is {} byte(s), expected at least {rows_end}",
            payload.len()
        ));
        return summary;
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "air_variant_archetype_rows", row_count);
    add_unit(
        &mut summary,
        "air_variant_archetype_observed_variant_rows",
        observed_variant_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_classified_rows",
        classified_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_unresolved_rows",
        unresolved_rows,
    );
    if static_catalog_hash != 0 {
        add_unit(
            &mut summary,
            "air_variant_archetype_static_catalog_hash_present",
            1,
        );
    }

    let mut cursor = HEADER_LEN;
    let mut seen_variants = BTreeSet::new();
    let mut row_headers = Vec::with_capacity(row_count);
    let mut decoded_classified_rows = 0usize;
    let mut decoded_unresolved_rows = 0usize;
    let mut static_catalog_rows = 0usize;
    let mut save_rejected_rows = 0usize;
    let mut static_heuristic_rows = 0usize;
    let mut live_token_dictionary_rows = 0usize;
    let mut static_source_enum_rows = 0usize;
    for row_index in 0..row_count {
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let equipment_token_raw = read_u32_le(payload, cursor + 8);
        let equipment_type_token_raw = read_u32_le(payload, cursor + 12);
        let interface_type_raw = read_u32_le(payload, cursor + 16);
        let air_superiority_weight_raw = read_i32_le(payload, cursor + 20);
        let source_mask = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += ROW_LEN;

        let deleted = flags & ROW_FLAG_DELETED != 0;
        if variant_id == 0 || variant_id > MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        if flags & !KNOWN_ROW_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} has unknown flags {flags}"
            ));
        }
        if source_mask == 0 && !deleted {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} has empty source_mask"
            ));
        }
        if source_mask & !KNOWN_SOURCE_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} has unknown source_mask {source_mask}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} reserved field is nonzero"
            ));
        }
        if deleted {
            if equipment_token_raw != 0
                || equipment_type_token_raw != 0
                || interface_type_raw != 0
                || air_superiority_weight_raw != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant archetypes deleted row {row_index} carries live fields"
                ));
            }
        } else {
            if equipment_token_raw == 0 || equipment_token_raw > MAX_EQUIPMENT_TOKEN_RAW {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant archetypes row {row_index} has invalid equipment_token_raw {equipment_token_raw}"
                ));
            }
            if equipment_type_token_raw == 0
                || equipment_type_token_raw > MAX_EQUIPMENT_TYPE_TOKEN_RAW
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant archetypes row {row_index} has invalid equipment_type_token_raw {equipment_type_token_raw}"
                ));
            }
            if interface_type_raw > MAX_INTERFACE_TYPE_RAW {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant archetypes row {row_index} has invalid interface_type_raw {interface_type_raw}"
                ));
            }
        }
        if flags & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0 {
            if !(0..=MAX_AIR_SUPERIORITY_WEIGHT_RAW).contains(&air_superiority_weight_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant archetypes row {row_index} has invalid air_superiority_weight_raw {air_superiority_weight_raw}"
                ));
            }
        } else if air_superiority_weight_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes row {row_index} has air_superiority_weight without flag"
            ));
        }

        if source_mask & SOURCE_STATIC_CATALOG != 0 {
            static_catalog_rows = static_catalog_rows.saturating_add(1);
        }
        if source_mask & SOURCE_DEV_SAVE_JOIN_REJECTED != 0 {
            save_rejected_rows = save_rejected_rows.saturating_add(1);
        }
        if source_mask & SOURCE_STATIC_HEURISTIC != 0 {
            static_heuristic_rows = static_heuristic_rows.saturating_add(1);
        }
        if source_mask & SOURCE_LIVE_TOKEN_DICTIONARY != 0 {
            live_token_dictionary_rows = live_token_dictionary_rows.saturating_add(1);
        }
        if source_mask & SOURCE_STATIC_SOURCE_ENUM != 0 {
            static_source_enum_rows = static_source_enum_rows.saturating_add(1);
        }
        row_headers.push((flags, deleted));
    }

    let mut archetype_string_rows = 0usize;
    let mut base_airframe_string_rows = 0usize;
    let mut role_string_rows = 0usize;
    for (row_index, (flags, deleted)) in row_headers.iter().enumerate() {
        let archetype_len =
            match skip_compact_string_len(payload, &mut cursor, "air variant archetype token") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return summary;
                }
            };
        let base_len = match skip_compact_string_len(
            payload,
            &mut cursor,
            "air variant base airframe token",
        ) {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return summary;
            }
        };
        let role_len = match skip_compact_string_len(payload, &mut cursor, "air variant role token")
        {
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
            "archetype",
            archetype_len,
            *flags,
            ROW_FLAG_ARCHETYPE,
        );
        validate_flagged_string(
            &mut summary,
            offset,
            row_index,
            "base_airframe",
            base_len,
            *flags,
            ROW_FLAG_BASE_AIRFRAME,
        );
        validate_flagged_string(
            &mut summary,
            offset,
            row_index,
            "role",
            role_len,
            *flags,
            ROW_FLAG_ROLE,
        );
        if *deleted && (archetype_len != 0 || base_len != 0 || role_len != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant archetypes deleted row {row_index} carries strings"
            ));
        }
        if archetype_len > 0 {
            archetype_string_rows = archetype_string_rows.saturating_add(1);
        }
        if base_len > 0 {
            base_airframe_string_rows = base_airframe_string_rows.saturating_add(1);
        }
        if role_len > 0 {
            role_string_rows = role_string_rows.saturating_add(1);
        }
        if archetype_len > 0
            || base_len > 0
            || role_len > 0
            || flags & ROW_FLAG_AIR_SUPERIORITY_WEIGHT != 0
        {
            decoded_classified_rows = decoded_classified_rows.saturating_add(1);
        } else {
            decoded_unresolved_rows = decoded_unresolved_rows.saturating_add(1);
        }
    }
    if cursor != payload.len() {
        // Wave-4 ace lifecycle: bytes after the self-delimiting base payload
        // are the domain-46 `ACE1` air-ace-roster trailer (MTD1 precedent).
        // Anything else is a parse error, exactly as before. The trailer's
        // status flags are lane-scoped and are surfaced as data units, NOT
        // OR-ed into the record status (they must not feed the global
        // compact-status interpretation).
        let ace_summary =
            crate::domain46_air_ace_roster_live::decode_trailer(offset, payload, cursor);
        summary.parse_errors.extend(ace_summary.parse_errors);
        for (key, value) in ace_summary.data_units_total {
            let entry = summary.data_units_total.entry(key).or_default();
            *entry = entry.saturating_add(value);
        }
        // Per-status frame counters (sums stay meaningful across records).
        if ace_summary.status_flags_or
            & crate::domain46_air_ace_roster_live::ACE_STATUS_SOURCE_UNAVAILABLE
            != 0
        {
            add_unit(&mut summary, "air_ace_roster_source_unavailable_frames", 1);
        }
        if ace_summary.status_flags_or
            & crate::domain46_air_ace_roster_live::ACE_STATUS_PARTIAL_DATA
            != 0
        {
            add_unit(&mut summary, "air_ace_roster_partial_frames", 1);
        }
        if ace_summary.status_flags_or
            & crate::domain46_air_ace_roster_live::ACE_STATUS_WING_ASSIGNMENT_BLOCKED
            != 0
        {
            add_unit(&mut summary, "air_ace_roster_assignment_blocked_frames", 1);
        }
    }
    if decoded_classified_rows != classified_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes classified_rows {classified_rows} != decoded {decoded_classified_rows}"
        ));
    }
    if decoded_unresolved_rows != unresolved_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes unresolved_rows {unresolved_rows} != decoded {decoded_unresolved_rows}"
        ));
    }

    add_unit(
        &mut summary,
        "air_variant_archetype_archetype_string_rows",
        archetype_string_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_base_airframe_string_rows",
        base_airframe_string_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_role_string_rows",
        role_string_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_static_catalog_rows",
        static_catalog_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_dev_save_join_rejected_rows",
        save_rejected_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_static_heuristic_rows",
        static_heuristic_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_live_token_dictionary_rows",
        live_token_dictionary_rows,
    );
    add_unit(
        &mut summary,
        "air_variant_archetype_static_source_enum_rows",
        static_source_enum_rows,
    );

    summary
}

fn validate_flagged_string(
    summary: &mut AirVariantArchetypeGateSummary,
    offset: usize,
    row_index: usize,
    label: &str,
    len: usize,
    flags: u32,
    flag: u32,
) {
    if len > MAX_STRING_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes row {row_index} {label} string is too long ({len})"
        ));
    }
    if len == 0 && flags & flag != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes row {row_index} has {label} flag with empty string"
        ));
    }
    if len > 0 && flags & flag == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant archetypes row {row_index} has {label} string without flag"
        ));
    }
}

fn add_unit(summary: &mut AirVariantArchetypeGateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_static_archetype_rows() {
        let rows = vec![
            AirVariantArchetypeRow {
                variant_id: 1375,
                deleted: false,
                equipment_token_raw: 20_335,
                equipment_type_token_raw: 1,
                interface_type_raw: 5,
                air_superiority_weight_raw: Some(FIXED_SCALE_RAW as i32),
                source_mask: SOURCE_LIVE_STATS
                    | SOURCE_LIVE_DEFINITION
                    | SOURCE_STATIC_CATALOG
                    | SOURCE_LIVE_TOKEN_DICTIONARY
                    | SOURCE_STATIC_SOURCE_ENUM,
                archetype_token: Some("small_plane_cas_airframe_0".to_string()),
                base_airframe_token: Some("small_plane_airframe".to_string()),
                role_token: Some("cas".to_string()),
            },
            AirVariantArchetypeRow {
                variant_id: 1325,
                deleted: false,
                equipment_token_raw: 20_213,
                equipment_type_token_raw: 1,
                interface_type_raw: 0,
                air_superiority_weight_raw: None,
                source_mask: SOURCE_LIVE_STATS | SOURCE_DEV_SAVE_JOIN_REJECTED,
                archetype_token: None,
                base_airframe_token: None,
                role_token: None,
            },
        ];
        let header = AirVariantArchetypeHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: rows.len() as u32,
            observed_variant_rows: 91,
            classified_rows: 1,
            unresolved_rows: 1,
            static_catalog_hash: 0xCAFE_BABE,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(32, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(summary.status_flags_or, STATUS_DELTA_DATA);
        assert_eq!(summary.data_rows_total, 2);
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_classified_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_dev_save_join_rejected_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_live_token_dictionary_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_static_source_enum_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_string_without_matching_flag() {
        let rows = vec![AirVariantArchetypeRow {
            variant_id: 1375,
            deleted: false,
            equipment_token_raw: 20_335,
            equipment_type_token_raw: 1,
            interface_type_raw: 5,
            air_superiority_weight_raw: Some(FIXED_SCALE_RAW as i32),
            source_mask: SOURCE_LIVE_STATS | SOURCE_STATIC_CATALOG,
            archetype_token: Some("small_plane_cas_airframe_0".to_string()),
            base_airframe_token: None,
            role_token: None,
        }];
        let header = AirVariantArchetypeHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_variant_rows: 1,
            classified_rows: 1,
            unresolved_rows: 0,
            static_catalog_hash: 1,
        };
        let mut payload = encode_payload(&header, &rows);
        let flags_offset = HEADER_LEN + 4;
        let mut flags = read_u32_le(&payload, flags_offset);
        flags &= !ROW_FLAG_ARCHETYPE;
        payload[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("archetype string without flag")));
    }

    #[test]
    fn rejects_bad_source_mask() {
        let rows = vec![AirVariantArchetypeRow {
            variant_id: 1375,
            deleted: false,
            equipment_token_raw: 20_335,
            equipment_type_token_raw: 1,
            interface_type_raw: 5,
            air_superiority_weight_raw: None,
            source_mask: 1 << 31,
            archetype_token: None,
            base_airframe_token: None,
            role_token: None,
        }];
        let header = AirVariantArchetypeHeader {
            elapsed_wall_ms: 100,
            status_flags: STATUS_DELTA_DATA,
            changed_rows: 1,
            observed_variant_rows: 1,
            classified_rows: 0,
            unresolved_rows: 1,
            static_catalog_hash: 0,
        };
        let payload = encode_payload(&header, &rows);
        let summary = decode_payload(7, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown source_mask")));
    }
}
