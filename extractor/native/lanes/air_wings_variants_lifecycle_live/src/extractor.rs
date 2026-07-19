use crate::domain46_air_ace_roster_live::{
    AirAceRosterHeader, AirAceRosterRow, ACE_STATUS_PARTIAL_DATA,
    ACE_STATUS_WING_ASSIGNMENT_BLOCKED, SOURCE_DEV_SAVE_LAYOUT_VALIDATION,
    SOURCE_LIVE_ACE_DEATH_RECORD, SOURCE_LIVE_ACE_LIFECYCLE_RECORD, SOURCE_LIVE_ACE_OBJECT_LAYOUT,
    SOURCE_LIVE_COUNTRY_ACE_VECTOR, SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT,
    SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED,
};
use crate::domain46_air_variant_archetypes_live::{
    AirVariantArchetypeHeader, AirVariantArchetypeRow, FIXED_SCALE_RAW, SOURCE_LIVE_STATS,
    SOURCE_LIVE_TOKEN_DICTIONARY, SOURCE_STATIC_HEURISTIC, SOURCE_STATIC_SOURCE_ENUM,
    STATUS_DELTA_DATA,
};
use std::collections::BTreeMap;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LiveAceRosterInput {
    pub tag_db_id: u32,
    pub ace_id: u32,
    pub country_ace_index: u32,
    pub wing_id: Option<u32>,
    pub identity_layout_validated: bool,
    pub save_layout_validated: bool,
    pub wing_assignment_validated: bool,
    pub assignment_blocked: bool,
    pub name: Option<String>,
    pub surname: Option<String>,
    pub callsign: Option<String>,
    /// Validated alive flag (+0x158): `Some(true)`=alive, `Some(false)`=dead,
    /// `None`=unknown (torn/out-of-domain byte) — never defaulted.
    pub alive: Option<bool>,
    /// Serializer-owned handled flag (+0x16C). Unknown when the lifecycle
    /// tuple is torn or out of domain.
    pub handled: Option<bool>,
    /// Serializer-owned raw kill type (+0x15C). Kept numeric because the
    /// retained corpus does not prove an exhaustive enum mapping.
    pub kill_type_raw: Option<u32>,
    /// Killer ace id (+0x164); only meaningful on a dead ace and
    /// independently present (two D04 dead aces have none).
    pub killer_ace_id: Option<u32>,
    /// Killer country tag-db-id (+0x168); only meaningful on a dead ace.
    pub killer_country_tag_db_id: Option<u32>,
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

/// Build the `ACE1` trailer bytes for the domain-46 data record.
///
/// Fail-closed replace contract (the fold FULL-REPLACES its ace state only on
/// a clean, complete frame; PARTIAL and SOURCE_UNAVAILABLE both RETAIN prior
/// state so a read failure is never written as ground-truth absence):
/// - `source_unavailable`: the country scalar source was missing entirely
///   (0 rows carried, absent — never zero-filled) => SOURCE_UNAVAILABLE.
/// - `roster_incomplete`: at least one country root/ace that DOES exist could
///   not be read this frame (rejected root, torn vector, wrong vtable,
///   invalid id), so the emitted full-replace set is missing real rows =>
///   PARTIAL. The surviving rows are still carried for observability.
/// - `assignment_scan_blocked`: the wing walk failed/was empty, so no
///   assignments could be joined => WING_ASSIGNMENT_BLOCKED. This is an
///   assignment-only anomaly (identity is complete); it does NOT force a
///   retain on its own.
///
/// `skipped_rows` is an informational counter (identity + ambiguous-
/// assignment skips) and may be nonzero on a complete frame when only
/// ambiguous wing claims were dropped; PARTIAL is driven by `roster_incomplete`,
/// not by `skipped_rows`.
#[allow(clippy::too_many_arguments)]
pub fn build_air_ace_roster_trailer(
    observed_country_rows: u32,
    observed_ace_rows: u32,
    skipped_rows: u32,
    assignment_scan_blocked: bool,
    source_unavailable: bool,
    roster_incomplete: bool,
    aces: &[LiveAceRosterInput],
) -> Vec<u8> {
    let rows = aces
        .iter()
        .map(build_air_ace_roster_row)
        .collect::<Vec<_>>();
    let wing_assignment_rows = rows
        .iter()
        .filter(|row| row.wing_id.is_some())
        .count()
        .min(u32::MAX as usize) as u32;
    let dead_rows = rows
        .iter()
        .filter(|row| row.alive == Some(false))
        .count()
        .min(u32::MAX as usize) as u32;
    let mut status_flags = 0u32;
    if roster_incomplete {
        status_flags |= ACE_STATUS_PARTIAL_DATA;
    }
    if assignment_scan_blocked {
        status_flags |= ACE_STATUS_WING_ASSIGNMENT_BLOCKED;
    }
    if source_unavailable {
        status_flags |= crate::domain46_air_ace_roster_live::ACE_STATUS_SOURCE_UNAVAILABLE;
    }
    let header = AirAceRosterHeader {
        status_flags,
        observed_country_rows,
        observed_ace_rows,
        wing_assignment_rows,
        dead_rows,
        skipped_rows,
    };
    crate::domain46_air_ace_roster_live::encode_trailer(&header, &rows)
}

pub fn build_air_ace_roster_row(input: &LiveAceRosterInput) -> AirAceRosterRow {
    let mut source_mask = SOURCE_LIVE_COUNTRY_ACE_VECTOR;
    if input.identity_layout_validated {
        source_mask |= SOURCE_LIVE_ACE_OBJECT_LAYOUT;
    }
    if input.save_layout_validated {
        source_mask |= SOURCE_DEV_SAVE_LAYOUT_VALIDATION;
    }
    let wing_id = if input.wing_assignment_validated {
        if input.wing_id.is_some() {
            source_mask |= SOURCE_LIVE_WING_ACE_ID_ASSIGNMENT;
        }
        input.wing_id
    } else {
        None
    };
    if input.assignment_blocked {
        source_mask |= SOURCE_WING_ASSIGNMENT_SCAN_BLOCKED;
    }
    if input.alive.is_some() {
        source_mask |= SOURCE_LIVE_ACE_DEATH_RECORD;
    }
    if input.handled.is_some() && input.kill_type_raw.is_some() {
        source_mask |= SOURCE_LIVE_ACE_LIFECYCLE_RECORD;
    }
    // Killer fields are only trusted on a confirmed-dead ace (fail closed:
    // their content on living/unknown aces was never validated).
    let dead = input.alive == Some(false);

    AirAceRosterRow {
        tag_db_id: input.tag_db_id,
        ace_id: input.ace_id,
        country_ace_index: input.country_ace_index,
        wing_id,
        alive: input.alive,
        handled: input.handled,
        kill_type_raw: input.kill_type_raw,
        killer_ace_id: if dead { input.killer_ace_id } else { None },
        killer_country_tag_db_id: if dead {
            input.killer_country_tag_db_id
        } else {
            None
        },
        source_mask,
        name: input.name.clone(),
        surname: input.surname.clone(),
        callsign: input.callsign.clone(),
    }
}

pub fn build_air_variant_archetype_payload_with_token_dictionary(
    elapsed_wall_ms: u64,
    observed_variant_rows: u32,
    variants: &[LiveVariantInput],
    token_dictionary: &BTreeMap<u32, String>,
) -> Vec<u8> {
    let resolved = variants
        .iter()
        .map(|input| {
            let mut row = input.clone();
            if row.equipment_token_name.is_none() {
                row.equipment_token_name = token_dictionary.get(&row.equipment_token_raw).cloned();
            }
            row
        })
        .collect::<Vec<_>>();
    build_air_variant_archetype_payload(elapsed_wall_ms, observed_variant_rows, &resolved)
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
        if matches_airframe_rule(token, rule.needle) {
            return Some(ArchetypeClosure {
                archetype_token: Some(token.to_string()),
                base_airframe_token: Some(rule.base_airframe.to_string()),
                role_token: rule.role.map(str::to_string),
            });
        }
    }
    None
}

fn matches_airframe_rule(token: &str, needle: &str) -> bool {
    if token == needle {
        return true;
    }
    if let Some(suffix) = token.strip_prefix(needle) {
        return suffix.starts_with('_') && suffix[1..].chars().all(|ch| ch.is_ascii_digit());
    }
    false
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
    AirframeRule {
        needle: "cv_CAS_equipment",
        base_airframe: "cv_small_plane_cas_airframe",
        role: Some("cas"),
    },
    AirframeRule {
        needle: "CAS_equipment",
        base_airframe: "small_plane_cas_airframe",
        role: Some("cas"),
    },
    AirframeRule {
        needle: "cv_nav_bomber_equipment",
        base_airframe: "cv_small_plane_naval_bomber_airframe",
        role: Some("naval_bomber"),
    },
    AirframeRule {
        needle: "nav_bomber_equipment",
        base_airframe: "small_plane_naval_bomber_airframe",
        role: Some("naval_bomber"),
    },
    AirframeRule {
        needle: "cv_fighter_equipment",
        base_airframe: "cv_small_plane_airframe",
        role: Some("fighter"),
    },
    AirframeRule {
        needle: "fighter_equipment",
        base_airframe: "small_plane_airframe",
        role: Some("fighter"),
    },
    AirframeRule {
        needle: "jet_fighter_equipment",
        base_airframe: "small_plane_airframe",
        role: Some("fighter"),
    },
    AirframeRule {
        needle: "supersonic_fighter_equipment",
        base_airframe: "small_plane_airframe",
        role: Some("fighter"),
    },
    AirframeRule {
        needle: "heavy_fighter_equipment",
        base_airframe: "medium_plane_fighter_airframe",
        role: Some("heavy_fighter"),
    },
    AirframeRule {
        needle: "tac_bomber_equipment",
        base_airframe: "medium_plane_airframe",
        role: Some("tactical_bomber"),
    },
    AirframeRule {
        needle: "jet_tac_bomber_equipment",
        base_airframe: "medium_plane_airframe",
        role: Some("tactical_bomber"),
    },
    AirframeRule {
        needle: "strat_bomber_equipment",
        base_airframe: "large_plane_airframe",
        role: Some("strategic_bomber"),
    },
    AirframeRule {
        needle: "jet_strat_bomber_equipment",
        base_airframe: "large_plane_airframe",
        role: Some("strategic_bomber"),
    },
    AirframeRule {
        needle: "scout_plane_equipment",
        base_airframe: "medium_plane_scout_plane_airframe",
        role: Some("scout"),
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain46_air_ace_roster_live::decode_trailer as decode_ace_trailer;
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
        assert_eq!(closure.role_token.as_deref(), Some("maritime_patrol_plane"));
    }

    #[test]
    fn classifies_legacy_equipment_tokens() {
        let fighter = classify_equipment_token("fighter_equipment_0").unwrap();
        assert_eq!(
            fighter.base_airframe_token.as_deref(),
            Some("small_plane_airframe")
        );
        assert_eq!(fighter.role_token.as_deref(), Some("fighter"));

        let carrier_cas = classify_equipment_token("cv_CAS_equipment_2").unwrap();
        assert_eq!(
            carrier_cas.base_airframe_token.as_deref(),
            Some("cv_small_plane_cas_airframe")
        );
        assert_eq!(carrier_cas.role_token.as_deref(), Some("cas"));

        let tactical_bomber = classify_equipment_token("tac_bomber_equipment_3").unwrap();
        assert_eq!(
            tactical_bomber.base_airframe_token.as_deref(),
            Some("medium_plane_airframe")
        );
        assert_eq!(
            tactical_bomber.role_token.as_deref(),
            Some("tactical_bomber")
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
    fn token_dictionary_adapter_removes_raw_fallback() {
        let mut dictionary = BTreeMap::new();
        dictionary.insert(
            20_378,
            "large_plane_maritime_patrol_plane_airframe_1".to_string(),
        );
        let payload = build_air_variant_archetype_payload_with_token_dictionary(
            100,
            1,
            &[LiveVariantInput {
                variant_id: 5085,
                equipment_token_raw: 20_378,
                equipment_type_token_raw: 2,
                interface_type_raw: 27,
                air_superiority_weight_raw: None,
                equipment_token_name: None,
            }],
            &dictionary,
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
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_static_heuristic_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_base_airframe_string_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_role_string_rows")
                .copied(),
            Some(1)
        );
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

    fn ace_input(alive: Option<bool>) -> LiveAceRosterInput {
        LiveAceRosterInput {
            tag_db_id: 1,
            ace_id: 17514,
            country_ace_index: 10,
            wing_id: Some(290),
            identity_layout_validated: true,
            save_layout_validated: true,
            wing_assignment_validated: true,
            assignment_blocked: false,
            name: Some("Adler".to_string()),
            surname: Some("Driess".to_string()),
            callsign: Some("Fighter of Libau".to_string()),
            alive,
            handled: alive.map(|value| !value),
            kill_type_raw: alive.map(|value| if value { 0 } else { 1 }),
            killer_ace_id: None,
            killer_country_tag_db_id: None,
        }
    }

    #[test]
    fn builds_ace_roster_trailer_with_validated_wing_assignment() {
        let trailer =
            build_air_ace_roster_trailer(439, 1, 0, false, false, false, &[ace_input(Some(true))]);
        let summary = decode_ace_trailer(0, &trailer, 0);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_identity_rows")
                .copied(),
            Some(1)
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
                .get("air_ace_roster_alive_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn ace_roster_trailer_drops_unvalidated_wing_assignment() {
        let mut input = ace_input(Some(true));
        input.wing_assignment_validated = false;
        input.assignment_blocked = true;
        let trailer = build_air_ace_roster_trailer(439, 1, 0, true, false, false, &[input]);
        let summary = decode_ace_trailer(0, &trailer, 0);
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
            Some(0)
        );
        assert_eq!(
            summary.status_flags_or,
            crate::domain46_air_ace_roster_live::ACE_STATUS_WING_ASSIGNMENT_BLOCKED
        );
    }

    #[test]
    fn ace_roster_row_drops_killer_fields_unless_dead() {
        let mut living = ace_input(Some(true));
        living.killer_ace_id = Some(6265);
        living.killer_country_tag_db_id = Some(92);
        let row = build_air_ace_roster_row(&living);
        assert_eq!(row.killer_ace_id, None);
        assert_eq!(row.killer_country_tag_db_id, None);

        let mut unknown = ace_input(None);
        unknown.killer_ace_id = Some(6265);
        let row = build_air_ace_roster_row(&unknown);
        assert_eq!(row.alive, None);
        assert_eq!(row.killer_ace_id, None);

        let mut dead = ace_input(Some(false));
        dead.killer_ace_id = Some(6265);
        dead.killer_country_tag_db_id = Some(92);
        let row = build_air_ace_roster_row(&dead);
        assert_eq!(row.killer_ace_id, Some(6265));
        assert_eq!(row.killer_country_tag_db_id, Some(92));
    }

    #[test]
    fn archetype_payload_with_ace_trailer_decodes_both_slices() {
        let base = build_air_variant_archetype_payload(
            120,
            1,
            &[LiveVariantInput {
                variant_id: 77,
                equipment_token_raw: 20378,
                equipment_type_token_raw: 20378,
                interface_type_raw: 3,
                air_superiority_weight_raw: None,
                equipment_token_name: Some("small_plane_airframe_2".to_string()),
            }],
        );
        let trailer =
            build_air_ace_roster_trailer(439, 1, 0, false, false, false, &[ace_input(Some(false))]);
        let mut payload = base;
        payload.extend_from_slice(&trailer);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_variant_archetype_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary.data_units_total.get("air_ace_roster_rows").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("air_ace_roster_dead_rows")
                .copied(),
            Some(1)
        );
    }
}
