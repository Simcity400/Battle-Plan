use crate::domain46_air_variant_archetypes_live::{
    AirVariantArchetypeHeader, AirVariantArchetypeRow, FIXED_SCALE_RAW, SOURCE_LIVE_STATS,
    SOURCE_LIVE_TOKEN_DICTIONARY, SOURCE_STATIC_HEURISTIC, SOURCE_STATIC_SOURCE_ENUM,
    STATUS_DELTA_DATA,
};

pub const CGAME_STATE_STRATEGIC_AIR_MANAGER_OFF: u64 = 0x690;
pub const STRATEGIC_AIR_OWNER_CONTAINERS_PTR_OFF: u64 = 0x30;
pub const STRATEGIC_AIR_OWNER_CONTAINERS_COUNT_OFF: u64 = 0x3C;
pub const OWNER_AIR_WING_POOL_PTR_TABLE_OFF: u64 = 0x58;
pub const OWNER_AIR_WING_POOL_COUNT_OFF: u64 = 0x64;
pub const AIR_WING_POOL_OBJECT_TABLE_OFF: u64 = 0x28;
pub const AIR_WING_OBJECT_BODY_OFF: u64 = 0x10;
pub const AIR_WING_ID_OFF: u64 = 0x0C;
pub const AIR_WING_COUNT_OFF: u64 = 0x6C;
pub const AIR_WING_MISSION_MASK_OFF: u64 = 0x90;
pub const AIR_WING_MISSION_REGION_OFF: u64 = 0xB0;
pub const AIR_WING_EFFECTIVE_PLANES_OFF: u64 = 0x160;
pub const AIR_WING_EQUIPMENT_BANK_OFF: u64 = 0x1E8;

pub const VARIANT_EQUIPMENT_TOKEN_RAW_OFF: u64 = 0x18;
pub const VARIANT_EQUIPMENT_TYPE_TOKEN_RAW_OFF: u64 = 0x1C;
pub const VARIANT_INTERFACE_TYPE_RAW_OFF: u64 = 0x38;

pub const COUNTRY_FLAG_MANAGER_OFF: u64 = 0x230;
pub const FLAG_MANAGER_TOKEN_RECORD_VTABLE_OFF: u64 = 0x18;
pub const TOKEN_RECORD_ID_OFF: u64 = 0x08;
pub const TOKEN_RECORD_TEXT_PTR_OFF: u64 = 0x10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveVariantInput {
    pub variant_id: u32,
    pub equipment_token_raw: u32,
    pub equipment_type_token_raw: u32,
    pub interface_type_raw: u32,
    pub air_superiority_weight_raw: Option<i32>,
    pub equipment_token_name: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchetypeClosure {
    pub archetype_token: Option<String>,
    pub base_airframe_token: Option<String>,
    pub role_token: Option<String>,
}

pub fn build_air_variant_archetype_payload(
    elapsed_wall_ms: u64,
    observed_variant_rows: u32,
    variants: &[LiveVariantInput],
) -> Vec<u8> {
    let rows = variants
        .iter()
        .map(build_air_variant_archetype_row)
        .collect::<Vec<_>>();
    let classified_rows = rows
        .iter()
        .filter(|row| {
            row.archetype_token.is_some()
                || row.base_airframe_token.is_some()
                || row.role_token.is_some()
                || row.air_superiority_weight_raw.is_some()
        })
        .count()
        .min(u32::MAX as usize) as u32;
    let row_count = rows.len().min(u32::MAX as usize) as u32;
    let header = AirVariantArchetypeHeader {
        elapsed_wall_ms,
        status_flags: STATUS_DELTA_DATA,
        changed_rows: row_count,
        observed_variant_rows,
        classified_rows,
        unresolved_rows: row_count.saturating_sub(classified_rows),
        static_catalog_hash: 0,
    };
    crate::domain46_air_variant_archetypes_live::encode_payload(&header, &rows)
}

pub fn build_air_variant_archetype_row(input: &LiveVariantInput) -> AirVariantArchetypeRow {
    let closure = close_or_badge_equipment_token(
        input.equipment_token_raw,
        input.equipment_token_name.as_deref(),
    );
    let mut source_mask = SOURCE_LIVE_STATS;
    if input.equipment_token_name.is_some() {
        source_mask |= SOURCE_LIVE_TOKEN_DICTIONARY;
    }
    if input.equipment_token_name.is_none() {
        source_mask |= SOURCE_STATIC_HEURISTIC;
    }
    if closure.base_airframe_token.is_some() {
        source_mask |= SOURCE_STATIC_SOURCE_ENUM;
    }

    AirVariantArchetypeRow {
        variant_id: input.variant_id,
        deleted: false,
        equipment_token_raw: input.equipment_token_raw,
        equipment_type_token_raw: input.equipment_type_token_raw,
        interface_type_raw: input.interface_type_raw,
        air_superiority_weight_raw: input.air_superiority_weight_raw,
        source_mask,
        archetype_token: closure.archetype_token,
        base_airframe_token: closure.base_airframe_token,
        role_token: closure.role_token,
    }
}

pub fn close_or_badge_equipment_token(
    equipment_token_raw: u32,
    token_name: Option<&str>,
) -> ArchetypeClosure {
    if let Some(token) = token_name {
        return classify_equipment_token(token).unwrap_or(ArchetypeClosure {
            archetype_token: Some(token.to_string()),
            base_airframe_token: None,
            role_token: None,
        });
    }
    ArchetypeClosure {
        archetype_token: Some(format!("equipment_token_raw_{equipment_token_raw}")),
        base_airframe_token: None,
        role_token: None,
    }
}

pub fn classify_equipment_token(token: &str) -> Option<ArchetypeClosure> {
    for rule in BASE_AIRFRAME_RULES {
        if token.contains(rule.needle) {
            return Some(ArchetypeClosure {
                archetype_token: Some(token.to_string()),
                base_airframe_token: Some(rule.base_airframe.to_string()),
                role_token: rule.role.map(str::to_string),
            });
        }
    }
    None
}

pub fn fixed_one_raw() -> i32 {
    FIXED_SCALE_RAW as i32
}

struct AirframeRule {
    needle: &'static str,
    base_airframe: &'static str,
    role: Option<&'static str>,
}

const BASE_AIRFRAME_RULES: &[AirframeRule] = &[
    AirframeRule {
        needle: "cv_small_plane_naval_bomber_airframe",
        base_airframe: "cv_small_plane_naval_bomber_airframe",
        role: Some("naval_bomber"),
    },
    AirframeRule {
        needle: "small_plane_naval_bomber_airframe",
        base_airframe: "small_plane_naval_bomber_airframe",
        role: Some("naval_bomber"),
    },
    AirframeRule {
        needle: "medium_plane_scout_plane_airframe",
        base_airframe: "medium_plane_scout_plane_airframe",
        role: Some("scout"),
    },
    AirframeRule {
        needle: "medium_plane_fighter_airframe",
        base_airframe: "medium_plane_fighter_airframe",
        role: Some("heavy_fighter"),
    },
    AirframeRule {
        needle: "large_plane_maritime_patrol_plane_airframe",
        base_airframe: "large_plane_maritime_patrol_plane_airframe",
        role: Some("maritime_patrol_plane"),
    },
    AirframeRule {
        needle: "cv_small_plane_cas_airframe",
        base_airframe: "cv_small_plane_cas_airframe",
        role: Some("cas"),
    },
    AirframeRule {
        needle: "small_plane_cas_airframe",
        base_airframe: "small_plane_cas_airframe",
        role: Some("cas"),
    },
    AirframeRule {
        needle: "cv_small_plane_airframe",
        base_airframe: "cv_small_plane_airframe",
        role: None,
    },
    AirframeRule {
        needle: "small_plane_airframe",
        base_airframe: "small_plane_airframe",
        role: None,
    },
    AirframeRule {
        needle: "medium_plane_airframe",
        base_airframe: "medium_plane_airframe",
        role: None,
    },
    AirframeRule {
        needle: "large_plane_airframe",
        base_airframe: "large_plane_airframe",
        role: None,
    },
    AirframeRule {
        needle: "transport_plane_equipment",
        base_airframe: "transport_plane_equipment",
        role: Some("transport"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain46_air_variant_archetypes_live::{
        decode_payload, SOURCE_LIVE_TOKEN_DICTIONARY, SOURCE_STATIC_SOURCE_ENUM,
    };

    #[test]
    fn classifies_active_token_dictionary_airframe() {
        let closure = classify_equipment_token("small_plane_cas_airframe_2").unwrap();
        assert_eq!(
            closure.archetype_token.as_deref(),
            Some("small_plane_cas_airframe_2")
        );
        assert_eq!(
            closure.base_airframe_token.as_deref(),
            Some("small_plane_cas_airframe")
        );
        assert_eq!(closure.role_token.as_deref(), Some("cas"));
    }

    #[test]
    fn classifies_transport_airframe() {
        let closure = classify_equipment_token("transport_plane_equipment_1").unwrap();
        assert_eq!(
            closure.base_airframe_token.as_deref(),
            Some("transport_plane_equipment")
        );
        assert_eq!(closure.role_token.as_deref(), Some("transport"));
    }

    #[test]
    fn classifies_maritime_patrol_airframe() {
        let closure =
            classify_equipment_token("large_plane_maritime_patrol_plane_airframe_1").unwrap();
        assert_eq!(
            closure.base_airframe_token.as_deref(),
            Some("large_plane_maritime_patrol_plane_airframe")
        );
        assert_eq!(
            closure.role_token.as_deref(),
            Some("maritime_patrol_plane")
        );
    }

    #[test]
    fn unresolved_token_stays_fail_closed() {
        let row = build_air_variant_archetype_row(&LiveVariantInput {
            variant_id: 120,
            equipment_token_raw: 20_000,
            equipment_type_token_raw: 1,
            interface_type_raw: 0,
            air_superiority_weight_raw: None,
            equipment_token_name: Some("experimental_unknown_plane".to_string()),
        });
        assert_eq!(
            row.archetype_token.as_deref(),
            Some("experimental_unknown_plane")
        );
        assert_eq!(row.base_airframe_token, None);
        assert_ne!(row.source_mask & SOURCE_LIVE_TOKEN_DICTIONARY, 0);
        assert_eq!(row.source_mask & SOURCE_STATIC_SOURCE_ENUM, 0);
    }

    #[test]
    fn missing_token_name_gets_raw_fail_closed_badge() {
        let row = build_air_variant_archetype_row(&LiveVariantInput {
            variant_id: 5085,
            equipment_token_raw: 20_378,
            equipment_type_token_raw: 2,
            interface_type_raw: 27,
            air_superiority_weight_raw: None,
            equipment_token_name: None,
        });
        assert_eq!(
            row.archetype_token.as_deref(),
            Some("equipment_token_raw_20378")
        );
        assert_eq!(row.base_airframe_token, None);
        assert_ne!(row.source_mask & SOURCE_STATIC_HEURISTIC, 0);
        assert_eq!(row.source_mask & SOURCE_LIVE_TOKEN_DICTIONARY, 0);
    }

    #[test]
    fn builds_payload_that_passes_lane_gate() {
        let payload = build_air_variant_archetype_payload(
            250,
            2,
            &[
                LiveVariantInput {
                    variant_id: 120,
                    equipment_token_raw: 20_000,
                    equipment_type_token_raw: 1,
                    interface_type_raw: 4,
                    air_superiority_weight_raw: Some(fixed_one_raw()),
                    equipment_token_name: Some("small_plane_airframe_0".to_string()),
                },
                LiveVariantInput {
                    variant_id: 121,
                    equipment_token_raw: 20_001,
                    equipment_type_token_raw: 1,
                    interface_type_raw: 0,
                    air_superiority_weight_raw: None,
                    equipment_token_name: Some("experimental_unknown_plane".to_string()),
                },
            ],
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_live_token_dictionary_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_static_source_enum_rows")
                .copied(),
            Some(1)
        );
    }
}
