//! Shared compact-stream contract: the single source of truth for compact
//! domain descriptors, H4CR record framing constants, stream status flags,
//! and the per-build offset/sentinel profile.
//!
//! The collector DLL, the recorder host, and every lane crate must consume
//! these definitions instead of keeping local copies. Domain IDs are assigned
//! in `extractor/documents/DOMAIN_REGISTRY.md`; the `ASSIGNED_COMPACT_DOMAINS`
//! table below must stay in sync with that registry.

pub mod build_profile;
pub mod calendar;

pub type CompactDomainId = u16;

// --- H4CR compact record framing -------------------------------------------

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;

pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;

pub const MAX_SCHEDULED_DOMAINS: usize = 64;

// --- Compact stream status flags (record header `flags` field) -------------

pub const COMPACT_STATUS_STATE_UNAVAILABLE: u32 = 1 << 0;
pub const COMPACT_STATUS_PROFILE_MISMATCH: u32 = 1 << 1;
pub const COMPACT_STATUS_NO_CLOCK: u32 = 1 << 2;
pub const COMPACT_STATUS_NO_SCALAR_SOURCE: u32 = 1 << 3;
pub const COMPACT_STATUS_WAR_BRIDGE_APPLIED: u32 = 1 << 4;
pub const COMPACT_STATUS_EXTRACTION_PERF_GATE: u32 = 1 << 5;
pub const COMPACT_STATUS_WORKER_PANIC: u32 = 1 << 6;
pub const COMPACT_STATUS_BRIDGE_UNAVAILABLE: u32 = 1 << 7;
pub const COMPACT_STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const COMPACT_STATUS_DELTA_DATA: u32 = 1 << 10;
pub const COMPACT_STATUS_TRANSITION_BASELINE_RESET: u32 = 1 << 11;
pub const COMPACT_STATUS_CONTROL_INVALID: u32 = 1 << 12;

pub const COMPACT_STATUS_FAILURE_MASK: u32 = COMPACT_STATUS_STATE_UNAVAILABLE
    | COMPACT_STATUS_PROFILE_MISMATCH
    | COMPACT_STATUS_NO_CLOCK
    | COMPACT_STATUS_NO_SCALAR_SOURCE
    | COMPACT_STATUS_WORKER_PANIC
    | COMPACT_STATUS_BRIDGE_UNAVAILABLE
    | COMPACT_STATUS_SOURCE_UNAVAILABLE
    | COMPACT_STATUS_PARTIAL_DATA
    | COMPACT_STATUS_CONTROL_INVALID;

pub const fn compact_status_is_failure(status_flags: u32) -> bool {
    (status_flags & COMPACT_STATUS_FAILURE_MASK) != 0
}

// --- Production profile ------------------------------------------------------

pub const STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG: &str = "templates_equipment_catalog";
pub const STATIC_ARTIFACT_LOCALISATION_JOIN: &str = "localisation_join";

pub const REQUIRED_STATIC_ARTIFACTS: &[&str] = &[
    STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
    STATIC_ARTIFACT_LOCALISATION_JOIN,
];

pub const PRODUCTION_PROFILE_RULE: &str =
    "schedulable compact domains whose descriptor status is Existing, ProductionComplete, ProductionCompleteBase, ValidatedSlice, ValidatedPartialSlice, or ValidatedSliceOpenGaps; validated slice/open-gap members are profile-scheduled for batched live proof and are not PRODUCTION_SLICE_DONE until that proof passes";

// --- Domain descriptors -----------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactDomainStatus {
    Existing,
    ProductionComplete,
    ProductionCompleteBase,
    ValidatedSlice,
    ValidatedPartialSlice,
    ValidatedSliceOpenGaps,
    ActiveIntegration,
    Reserved,
    Infrastructure,
}

pub const fn domain_status_registry_key(status: CompactDomainStatus) -> &'static str {
    match status {
        CompactDomainStatus::Existing => "existing",
        CompactDomainStatus::ProductionComplete => "production_complete",
        CompactDomainStatus::ProductionCompleteBase => "production_complete_base",
        CompactDomainStatus::ValidatedSlice => "validated_slice",
        CompactDomainStatus::ValidatedPartialSlice => "validated_partial_slice",
        CompactDomainStatus::ValidatedSliceOpenGaps => "validated_slice_open_gaps",
        CompactDomainStatus::ActiveIntegration => "active_integration",
        CompactDomainStatus::Reserved => "reserved",
        CompactDomainStatus::Infrastructure => "infrastructure",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactDomainDescriptor {
    pub id: CompactDomainId,
    pub canonical_name: &'static str,
    pub wire_name: &'static str,
    pub status: CompactDomainStatus,
    pub schedulable: bool,
    pub requires_data_record: bool,
    pub aliases: &'static [&'static str],
}

impl CompactDomainDescriptor {
    pub fn matches_any_name(&self, name: &str) -> bool {
        self.canonical_name == name
            || self.wire_name == name
            || self.aliases.iter().any(|alias| *alias == name)
    }

    pub fn matches_scheduled_name(&self, name: &str) -> bool {
        self.schedulable
            && (self.wire_name == name || self.aliases.iter().any(|alias| *alias == name))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainRegistryError {
    InvalidId {
        id: CompactDomainId,
    },
    InvalidName {
        name: &'static str,
    },
    DuplicateId {
        id: CompactDomainId,
        existing_name: &'static str,
        new_name: &'static str,
    },
    DuplicateName {
        name: &'static str,
        existing_id: CompactDomainId,
        new_id: CompactDomainId,
    },
}

pub fn validate_descriptor(descriptor: CompactDomainDescriptor) -> Result<(), DomainRegistryError> {
    if descriptor.id == 0 {
        return Err(DomainRegistryError::InvalidId { id: descriptor.id });
    }
    for name in descriptor_names(descriptor) {
        if !is_valid_domain_name(name) {
            return Err(DomainRegistryError::InvalidName { name });
        }
    }
    Ok(())
}

pub fn descriptor_names(descriptor: CompactDomainDescriptor) -> Vec<&'static str> {
    let mut names = Vec::with_capacity(2 + descriptor.aliases.len());
    names.push(descriptor.canonical_name);
    names.push(descriptor.wire_name);
    names.extend(descriptor.aliases.iter().copied());
    names
}

pub fn is_valid_domain_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub fn assigned_descriptor_by_id(id: CompactDomainId) -> Option<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.id == id)
}

pub fn assigned_descriptor_by_name(name: &str) -> Option<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.matches_any_name(name))
}

pub fn id_for_scheduled_name(name: &str) -> Option<CompactDomainId> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .find(|descriptor| descriptor.matches_scheduled_name(name))
        .map(|descriptor| descriptor.id)
}

pub fn wire_name_for_id(id: CompactDomainId) -> Option<&'static str> {
    assigned_descriptor_by_id(id).map(|descriptor| descriptor.wire_name)
}

pub fn requires_data_record(id: CompactDomainId) -> bool {
    assigned_descriptor_by_id(id)
        .map(|descriptor| descriptor.requires_data_record)
        .unwrap_or(false)
}

/// Returns true for descriptor statuses scheduled by the production recording
/// profile. `Existing` is included because it is the legacy status for the base
/// production compact domains. `ProductionComplete`/`ProductionCompleteBase`
/// are fully accepted production statuses. `ValidatedSlice`,
/// `ValidatedPartialSlice`, and `ValidatedSliceOpenGaps` are included so they
/// are captured in batched production-profile runs; those rows remain pending
/// proof at their documented scope and are not `PRODUCTION_SLICE_DONE` merely by
/// being profile-scheduled. `ActiveIntegration`, `Reserved`, and
/// `Infrastructure` are excluded from the profile, even when schedulable.
pub fn status_in_production_profile(status: CompactDomainStatus) -> bool {
    matches!(
        status,
        CompactDomainStatus::Existing
            | CompactDomainStatus::ProductionComplete
            | CompactDomainStatus::ProductionCompleteBase
            | CompactDomainStatus::ValidatedSlice
            | CompactDomainStatus::ValidatedPartialSlice
            | CompactDomainStatus::ValidatedSliceOpenGaps
    )
}

pub fn production_profile_domains() -> Vec<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.schedulable && status_in_production_profile(descriptor.status)
        })
        .collect()
}

pub fn production_profile_scheduled_names() -> Vec<&'static str> {
    production_profile_domains()
        .into_iter()
        .map(|descriptor| descriptor.wire_name)
        .collect()
}

pub const fn domain(
    id: CompactDomainId,
    canonical_name: &'static str,
    wire_name: &'static str,
    status: CompactDomainStatus,
    schedulable: bool,
    requires_data_record: bool,
    aliases: &'static [&'static str],
) -> CompactDomainDescriptor {
    CompactDomainDescriptor {
        id,
        canonical_name,
        wire_name,
        status,
        schedulable,
        requires_data_record,
        aliases,
    }
}

/// Every assigned compact domain ID. This descriptor table is the executable
/// registry; `extractor/native/domain_registry.json` is a derived mirror.
pub const ASSIGNED_COMPACT_DOMAINS: &[CompactDomainDescriptor] = &[
    domain(
        1,
        "clock",
        "clock",
        CompactDomainStatus::Infrastructure,
        false,
        false,
        &[],
    ),
    domain(
        10,
        "fuel_live",
        "fuel_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        11,
        "production_live",
        "production_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        12,
        "focus_live",
        "focus_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        13,
        "tech_live",
        "tech_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        14,
        "politics_live",
        "politics_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        15,
        "politics_residuals",
        "politics_residuals",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        16,
        "decisions_live",
        "decisions_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        17,
        "diplomacy_live",
        "diplomacy_live",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        18,
        "country_identity",
        "country_identity",
        CompactDomainStatus::ActiveIntegration,
        true,
        true,
        &[],
    ),
    domain(
        20,
        "divisions_live",
        "divisions",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        21,
        "land_combats_live",
        "land_combats",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        22,
        "map_control_live",
        "map_control",
        CompactDomainStatus::Existing,
        true,
        true,
        &["state_ownership", "state_control", "province_control"],
    ),
    domain(
        23,
        "map_detail_live",
        "map_detail",
        CompactDomainStatus::Existing,
        true,
        true,
        &["state_detail", "buildings_live"],
    ),
    domain(
        24,
        "weather_live",
        "weather_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        26,
        "division_equipment_live",
        "division_equipment",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        27,
        "army_orders_live",
        "army_orders",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        28,
        "oob_command_live",
        "oob_command",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        29,
        "ships_live",
        "ships",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        30,
        "air_wings_live",
        "air_wings",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        31,
        "air_region_activity_live",
        "air_region_activity",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        32,
        "air_wing_loadouts_live",
        "air_wing_loadouts",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        33,
        "air_variant_defs",
        "air_variant_defs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        34,
        "air_base_status_live",
        "air_base_status",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        35,
        "air_variant_stats",
        "air_variant_stats",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        36,
        "air_variant_reserves",
        "air_variant_reserves",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        37,
        "stockpile_variant_defs",
        "stockpile_variant_defs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        38,
        "air_superiority_inputs",
        "air_superiority_inputs",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        39,
        "air_variant_upgrades",
        "air_variant_upgrades",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        40,
        "stockpile_variant_upgrades",
        "stockpile_variant_upgrades",
        CompactDomainStatus::Existing,
        true,
        true,
        &[],
    ),
    domain(
        41,
        "projects_live",
        "projects_live",
        CompactDomainStatus::ValidatedPartialSlice,
        true,
        true,
        &[],
    ),
    domain(
        42,
        "air_combat_losses_live",
        "air_combat_losses_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        43,
        "market_trade_live",
        "market_trade_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        44,
        "supply_logistics_attrition_live",
        "supply_logistics_attrition_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        45,
        "focus_research_decisions_depth_live",
        "focus_research_decisions_depth_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        46,
        "air_variant_archetypes_live",
        "air_variant_archetypes",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        47,
        "characters_leaders_cabinet_live",
        "characters_leaders_cabinet_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        48,
        "construction_buildings_state_damage_live",
        "construction_buildings_state_damage_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        49,
        "naval_combat_lifecycle_live",
        "naval_combat_lifecycle_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        50,
        "naval_combat_members_live",
        "naval_combat_members_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        51,
        "scripted_state_flags_variables_live",
        "scripted_state_flags_variables_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        52,
        "occupation_resistance_compliance_live",
        "occupation_resistance_compliance_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        53,
        "diplomacy_intel_operatives_live",
        "diplomacy_intel_operatives_live",
        CompactDomainStatus::ActiveIntegration,
        false,
        true,
        &[],
    ),
    domain(
        54,
        "army_battleplans_depth_live",
        "army_battleplans_depth_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        55,
        "land_combat_detail_events_live",
        "land_combat_detail_events_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        56,
        "naval_missions_status_live",
        "naval_missions_status_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        57,
        "session_multiplayer_metadata_live",
        "session_multiplayer_metadata_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        58,
        "country_economy_counters_live",
        "country_economy_counters_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // Static definition artifact packed at replay-pack time; never scheduled
    // in the hot loop and never emits per-frame data records.
    domain(
        59,
        "templates_equipment_catalog_live",
        "templates_equipment_catalog_live",
        CompactDomainStatus::Reserved,
        false,
        false,
        &[],
    ),
    domain(
        1000,
        "frame",
        "frame",
        CompactDomainStatus::Infrastructure,
        false,
        false,
        &[],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_has_no_duplicate_ids_or_names() {
        let mut seen_ids = std::collections::BTreeSet::new();
        let mut seen_names = std::collections::BTreeSet::new();
        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            validate_descriptor(*descriptor).expect("descriptor valid");
            assert!(
                seen_ids.insert(descriptor.id),
                "duplicate id {}",
                descriptor.id
            );
            let mut names: Vec<&str> = descriptor_names(*descriptor);
            names.sort_unstable();
            names.dedup();
            for name in names {
                assert!(
                    seen_names.insert(name),
                    "name {name} used by more than one domain"
                );
            }
        }
    }

    #[test]
    fn all_registry_document_ids_are_assigned() {
        // extractor/documents/DOMAIN_REGISTRY.md active ID map.
        let expected: &[CompactDomainId] = &[
            1, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32,
            33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
            55, 56, 57, 58, 59, 1000,
        ];
        for id in expected {
            assert!(
                assigned_descriptor_by_id(*id).is_some(),
                "domain id {id} missing from ASSIGNED_COMPACT_DOMAINS"
            );
        }
        assert_eq!(ASSIGNED_COMPACT_DOMAINS.len(), expected.len());
    }

    #[test]
    fn native_json_registry_is_derived_from_descriptors() {
        let registry: serde_json::Value =
            serde_json::from_str(include_str!("../../domain_registry.json"))
                .expect("domain_registry.json parses");
        assert_eq!(
            registry
                .get("derived_from")
                .and_then(serde_json::Value::as_str),
            Some("compact_contract::ASSIGNED_COMPACT_DOMAINS")
        );

        let domains = registry
            .get("domains")
            .and_then(serde_json::Value::as_array)
            .expect("domains array");
        assert_eq!(
            domains.len(),
            ASSIGNED_COMPACT_DOMAINS.len(),
            "domain_registry.json must list every compact descriptor"
        );

        let mut by_id = std::collections::BTreeMap::new();
        for domain in domains {
            let id = domain
                .get("id")
                .and_then(serde_json::Value::as_u64)
                .and_then(|id| CompactDomainId::try_from(id).ok())
                .expect("domain id");
            assert!(
                by_id.insert(id, domain).is_none(),
                "duplicate json domain id {id}"
            );
        }

        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            let domain = by_id
                .get(&descriptor.id)
                .unwrap_or_else(|| panic!("json missing domain {}", descriptor.id));
            assert_eq!(
                domain.get("name").and_then(serde_json::Value::as_str),
                Some(descriptor.canonical_name),
                "json name drift for domain {}",
                descriptor.id
            );
            assert_eq!(
                domain.get("status").and_then(serde_json::Value::as_str),
                Some(domain_status_registry_key(descriptor.status)),
                "json status drift for domain {}",
                descriptor.id
            );
            assert_eq!(
                domain
                    .get("schedulable")
                    .and_then(serde_json::Value::as_bool),
                Some(descriptor.schedulable),
                "json schedulability drift for domain {}",
                descriptor.id
            );
            assert_eq!(
                domain
                    .get("requires_data_record")
                    .and_then(serde_json::Value::as_bool),
                Some(descriptor.requires_data_record),
                "json data-record requirement drift for domain {}",
                descriptor.id
            );
        }
    }

    #[test]
    fn scheduled_lookup_preserves_wire_names_and_aliases() {
        assert_eq!(id_for_scheduled_name("fuel_live"), Some(10));
        assert_eq!(id_for_scheduled_name("state_ownership"), Some(22));
        assert_eq!(id_for_scheduled_name("air_variant_archetypes"), Some(46));
        assert_eq!(id_for_scheduled_name("diplomacy_intel_operatives_live"), None);
        assert_eq!(id_for_scheduled_name("air_variant_archetypes_live"), None);
        assert_eq!(
            wire_name_for_id(53),
            Some("diplomacy_intel_operatives_live")
        );
        assert_eq!(wire_name_for_id(46), Some("air_variant_archetypes"));
        assert!(!requires_data_record(1000));
        assert!(requires_data_record(53));
        assert!(requires_data_record(46));
    }

    #[test]
    fn production_profile_is_derived_from_schedulable_validated_descriptors() {
        let profile = production_profile_domains();
        let profile_ids: std::collections::BTreeSet<CompactDomainId> =
            profile.iter().map(|descriptor| descriptor.id).collect();
        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            let expected =
                descriptor.schedulable && status_in_production_profile(descriptor.status);
            assert_eq!(
                profile_ids.contains(&descriptor.id),
                expected,
                "domain {} profile membership should be derived from descriptor status/schedulability",
                descriptor.id
            );
        }

        assert!(
            profile_ids.contains(&55),
            "domain 55 is validated/schedulable"
        );
        assert!(
            !profile_ids.contains(&53),
            "domain 53 stays out of production until live root re-validation"
        );
        assert!(
            profile_ids.contains(&56),
            "domain 56 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&48),
            "domain 48 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&47),
            "domain 47 is a validated slice with explicit open gaps"
        );
        for excluded in [1, 18, 49, 50, 59, 1000] {
            assert!(
                !profile_ids.contains(&excluded),
                "domain {excluded} must stay out of the production profile"
            );
        }

        assert_eq!(
            production_profile_scheduled_names(),
            profile
                .iter()
                .map(|descriptor| descriptor.wire_name)
                .collect::<Vec<_>>()
        );
        assert!(
            profile.len() <= MAX_SCHEDULED_DOMAINS,
            "production profile has {} domains but collector control accepts at most {}",
            profile.len(),
            MAX_SCHEDULED_DOMAINS
        );
    }

    #[test]
    fn required_static_artifacts_are_the_production_gate_artifacts() {
        assert_eq!(
            REQUIRED_STATIC_ARTIFACTS,
            &[
                STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
                STATIC_ARTIFACT_LOCALISATION_JOIN,
            ]
        );
    }

    #[test]
    fn failure_status_predicate_covers_failure_vocabulary_only() {
        for flag in [
            COMPACT_STATUS_STATE_UNAVAILABLE,
            COMPACT_STATUS_PROFILE_MISMATCH,
            COMPACT_STATUS_NO_CLOCK,
            COMPACT_STATUS_NO_SCALAR_SOURCE,
            COMPACT_STATUS_WORKER_PANIC,
            COMPACT_STATUS_BRIDGE_UNAVAILABLE,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            COMPACT_STATUS_PARTIAL_DATA,
            COMPACT_STATUS_CONTROL_INVALID,
        ] {
            assert!(
                compact_status_is_failure(flag),
                "flag {flag:#x} should fail"
            );
        }
        for flag in [
            COMPACT_STATUS_WAR_BRIDGE_APPLIED,
            COMPACT_STATUS_EXTRACTION_PERF_GATE,
            COMPACT_STATUS_DELTA_DATA,
            COMPACT_STATUS_TRANSITION_BASELINE_RESET,
        ] {
            assert!(
                !compact_status_is_failure(flag),
                "flag {flag:#x} should be informational"
            );
        }
    }
}
