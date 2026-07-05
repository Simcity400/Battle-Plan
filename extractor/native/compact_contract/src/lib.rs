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

/// H4CR record-header `flags` bit reserved for payload-schema selection.
///
/// This is framing metadata, not a compact status flag. Domain 45
/// `focus_research_decisions_depth_live` uses it to select the schema-v2
/// payload shape from outside the payload so legacy elapsed-time bytes cannot
/// collide with an in-payload magic marker.
pub const COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2: u32 = 1 << 31;

/// H4CR record-header `flags` bit announcing the optional domain-perf
/// diagnostic-unit trailer after the base 40-byte perf payload.
///
/// Presence is decided from this framing bit, not by scanning payload bytes, so
/// future perf payload extensions cannot collide with the trailer magic.
pub const COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER: u32 = 1 << 30;

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

// Domain 51 `scripted_state_flags_variables_live` carries lane-local badges in
// the shared status field for legacy wire compatibility. These bits intentionally
// overlap global bits and must be masked before global status interpretation.
pub const DOMAIN51_STATUS_FLAG_NAMES_PARTIAL: u32 = 1 << 11;
pub const DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED: u32 = 1 << 12;
pub const DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED: u32 = 1 << 13;
pub const DOMAIN51_LANE_LOCAL_STATUS_MASK: u32 = DOMAIN51_STATUS_FLAG_NAMES_PARTIAL
    | DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
    | DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED;

// Domain-49 (naval_combat_lifecycle_live) lane-local payload status bit: a
// lifecycle row is missing the promoted location/region pair (fail-closed
// missing-field badge, see the naval_combat_live lane). The bit value overlaps
// the global COMPACT_STATUS_TRANSITION_BASELINE_RESET bit and must be masked
// before global status interpretation.
pub const DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED: u32 = 1 << 11;
pub const DOMAIN49_LANE_LOCAL_STATUS_MASK: u32 = DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED;

pub const fn lane_local_status_mask(domain_id: CompactDomainId) -> u32 {
    match domain_id {
        49 => DOMAIN49_LANE_LOCAL_STATUS_MASK,
        51 => DOMAIN51_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    }
}

pub const fn compact_status_for_global_interpretation(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> u32 {
    status_flags & !lane_local_status_mask(domain_id)
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactExpectedPartialTolerance {
    pub domain_id: CompactDomainId,
    pub tolerated_flags: u32,
    pub reason: &'static str,
}

pub const COMPACT_EXPECTED_PARTIAL_TOLERANCES: &[CompactExpectedPartialTolerance] = &[
    CompactExpectedPartialTolerance {
        domain_id: 26,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "division_equipment: transient live-mutation row skips (observed 0.025% of roots), skip-counted; sparse fold retains prior equipment for skipped divisions",
    },
    CompactExpectedPartialTolerance {
        domain_id: 27,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "army_orders: unsupported/validation-failed order-target rows skip-counted (documented fixture-blocked scope)",
    },
    CompactExpectedPartialTolerance {
        domain_id: 28,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "oob_command: invalid root/task-force rows skip-counted (documented fixture-blocked scope)",
    },
    CompactExpectedPartialTolerance {
        domain_id: 41,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "projects_live: project lifecycle/reward sub-scopes are contract FIXTURE_BLOCKED; affected rows skip-counted",
    },
    CompactExpectedPartialTolerance {
        domain_id: 45,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "focus_research_decisions_depth_live: decision-target rows outside the validated exact slice are omitted and badged (decision_target_omitted_rows), documented fixture-blocked scope",
    },
    CompactExpectedPartialTolerance {
        domain_id: 48,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "construction_buildings_state_damage_live: transient live-mutation construction-line row skips (invalid line pointer/type/factory/progress), skip-counted; valid construction/building/resource rows retained and sparse fold preserves omitted siblings",
    },
    CompactExpectedPartialTolerance {
        domain_id: 49,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "naval_combat_lifecycle_live: transient live-mutation walk anomalies on an unpaused frame (torn CNavalCombat body/side/front reads, unreadable CFEXGroup owning-side back-refs) fail the whole frame closed with empty rows; the delta baseline is preserved and rows re-emit on the next clean frame",
    },
    CompactExpectedPartialTolerance {
        domain_id: 50,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "naval_combat_members_live: transient live-mutation member-walk anomalies on an unpaused frame (torn CFEXGroup member arrays, unreadable CFEXMember handles/slots, duplicate ship ids from a stale group, missing CShip OOB joins) fail the whole frame closed with empty rows; the delta baseline is preserved and rows re-emit on the next clean frame",
    },
    CompactExpectedPartialTolerance {
        domain_id: 54,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "army_battleplans_depth_live: documented fixture-blocked scopes (redraw/toggle lifecycle, activation/execution, stance) fail closed per-row",
    },
    CompactExpectedPartialTolerance {
        domain_id: 65,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "special_project_programs_live: CProgram facilities reachable from >1 country's CProgramStatus records (aliased stale category records) cannot be owner-attributed without a validated CProgram owner-tag offset; they are dropped fail-closed and counted (dropped_ambiguous), so the owner-joined set is intentionally incomplete when any ambiguity is present",
    },
];

pub fn expected_partial_tolerance(
    domain_id: CompactDomainId,
) -> Option<&'static CompactExpectedPartialTolerance> {
    COMPACT_EXPECTED_PARTIAL_TOLERANCES
        .iter()
        .find(|tolerance| tolerance.domain_id == domain_id)
}

pub fn expected_partial_tolerance_applies(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> Option<&'static CompactExpectedPartialTolerance> {
    let tolerance = expected_partial_tolerance(domain_id)?;
    let failure_flags = compact_status_for_global_interpretation(domain_id, status_flags)
        & COMPACT_STATUS_FAILURE_MASK;
    ((failure_flags & tolerance.tolerated_flags) != 0
        && (failure_flags & !tolerance.tolerated_flags) == 0)
        .then_some(tolerance)
}

pub fn compact_status_failure_flags_after_expected_partial_tolerance(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> u32 {
    let failure_flags = compact_status_for_global_interpretation(domain_id, status_flags)
        & COMPACT_STATUS_FAILURE_MASK;
    match expected_partial_tolerance_applies(domain_id, status_flags) {
        Some(tolerance) => failure_flags & !tolerance.tolerated_flags,
        None => failure_flags,
    }
}

// --- Production profile ------------------------------------------------------

pub const STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG: &str = "templates_equipment_catalog";
pub const STATIC_ARTIFACT_LOCALISATION_JOIN: &str = "localisation_join";
pub const STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS: &str = "decisions_static_definitions";
pub const STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC: &str = "idea_tech_loc_xref_static";
pub const STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC: &str = "ship_definitions_static";
pub const STATIC_ARTIFACT_GFX_ICON_MANIFEST: &str = "gfx_icon_asset_manifest_static";

pub const REQUIRED_STATIC_ARTIFACTS: &[&str] = &[
    STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
    STATIC_ARTIFACT_LOCALISATION_JOIN,
    STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS,
    STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC,
    STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC,
    STATIC_ARTIFACT_GFX_ICON_MANIFEST,
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
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        50,
        "naval_combat_members_live",
        "naval_combat_members_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
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
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        54,
        "army_battleplans_depth_live",
        "army_battleplans_depth_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
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
        CompactDomainStatus::ValidatedPartialSlice,
        true,
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
    // E1 (2026-07-04): per-country capitulation state + global war pool.
    // Independent of domain 53 by design — the d53 base extract is fail-closed
    // on 870ebf and all-or-nothing, while these families are fully validated
    // (FINAL_SWEEP §T1 + runs/e1_wire_capitulation_war).
    domain(
        60,
        "capitulation_war_state_live",
        "capitulation_war_state_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    // C1 (2026-07-05): per-country CUMULATIVE air-combat loss statistics (statistic id 23,
    // the 89-field CLoopHistory record). The corrected yearly-window chain is
    // D04 value-validated: 23/23 countries, 2047/2047 fields, 0 mismatches.
    // Only the six value-pinned channels are labeled; other channels remain
    // unpromoted.
    domain(
        61,
        "air_loss_stats_live",
        "air_loss_stats_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    // P1 (2026-07-05): node-level supply-hub motorization membership from the
    // persistent CCountrySupplySystem settings object. D04 value validation:
    // 223/223 rows, 1338/1338 emitted fields, 483/483 map entries, 0 mismatches.
    // Inner per-tag level attribution remains intentionally unpromoted.
    domain(
        62,
        "hub_motorization_live",
        "hub_motorization_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    // W1 (2026-07-05): per-country CORE scalars lane — manpower_pool
    // (CCountry+0x338), convoys_destroyed (+0x1334), divisions_lost
    // (*(CCountry+0x70)+0x54 post_mortem count), and the 7-channel
    // experience_status (+0x1588 -> CCountryExperienceStatus, i32/32768). D04
    // live value validation: manpower 436/436, convoys 16/16 nonzero + 420/420
    // absent-zero, experience 40/40 scalars (437/437 CES ptrs valid),
    // divisions_lost 436/436. Distinct from the reserved economy-counter BADGE
    // lane (domain 58): this carries live per-country scalar data.
    domain(
        63,
        "country_core_scalars_live",
        "country_core_scalars_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    // W3 (2026-07-05): per-country delayed (scripted) events, ROOTED walk of the
    // CCountry+0x1290 vector<CDelayedEvent*> (count +0x1298 low32). Per event:
    // event_id (+0x08 -> CEvent+0x0C), due_remaining_hours (+0xC4, RELATIVE
    // countdown = months*720+days*24+hours; the replay derives the absolute date
    // = clock + remaining), originator tag_db_id (+0xC0). D04 discovery
    // validation (agent D6): 13/13 countries, 17/17 events, ZERO mismatch vs the
    // melt oracle; no heap scan. Distinct from the forbidden d51 scripted-state
    // lane, whose delayed-event path is a heap-scan diagnostic BADGE that does
    // not emit per-event rows; d64 is the authoritative rooted per-event source
    // and d51 is left untouched.
    domain(
        64,
        "delayed_events_live",
        "delayed_events_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    // W3 (2026-07-05): per-country OWNER-JOINED special-project program
    // facilities, ROOTED walk of CCountry+0xFA8 -> CProgramStatus (array of up
    // to 3 records, stride 0x160, each record's own vtable == mb+0x295AC88) ->
    // record+0x20 vector<CProgram*> begin, record+0x28 packed u64 (low32=count,
    // high32=capacity). Per CProgram (vtable mb+0x295AC30): program_id (+0x0C),
    // province (+0x174), unread_prototype_rewards (+0x178). D04 discovery
    // validation (agent D6): 26/26 facilities exact vs melt (unread + province
    // both unique full-population offsets). FAIL-CLOSED owner attribution: a
    // CProgram pointer reachable from >1 country's records (stale category
    // records aliasing another country's vector) is DROPPED as ambiguous (no
    // production owner oracle exists), yielding the 21 single-owner facilities;
    // the 5 ambiguous facilities are badged/counted. Distinct from the committed
    // d41 projects_live flat (non-owner-joined) facility list, which is left
    // byte-identical.
    domain(
        65,
        "special_project_programs_live",
        "special_project_programs_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
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
            55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 1000,
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
        assert_eq!(
            id_for_scheduled_name("diplomacy_intel_operatives_live"),
            Some(53)
        );
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
            profile_ids.contains(&53),
            "domain 53 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&56),
            "domain 56 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&57),
            "domain 57 is a validated partial slice"
        );
        assert!(
            profile_ids.contains(&61),
            "domain 61 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&54),
            "domain 54 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&48),
            "domain 48 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&47),
            "domain 47 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&49),
            "domain 49 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&50),
            "domain 50 is a validated slice with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&63),
            "domain 63 (country_core_scalars_live) is a validated slice \
             with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&64),
            "domain 64 (delayed_events_live) is a validated slice \
             with explicit open gaps"
        );
        assert!(
            profile_ids.contains(&65),
            "domain 65 (special_project_programs_live) is a validated slice \
             with explicit open gaps"
        );
        for excluded in [1, 18, 58, 59, 1000] {
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
                STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS,
                STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC,
                STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC,
                STATIC_ARTIFACT_GFX_ICON_MANIFEST,
            ]
        );
    }

    #[test]
    fn domain51_lane_local_status_mask_is_domain_scoped() {
        assert_eq!(
            lane_local_status_mask(51),
            DOMAIN51_STATUS_FLAG_NAMES_PARTIAL
                | DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
                | DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED
        );
        assert_eq!(lane_local_status_mask(50), 0);
        assert_eq!(
            lane_local_status_mask(49),
            DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED
        );
        assert_eq!(
            compact_status_for_global_interpretation(
                49,
                DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED | COMPACT_STATUS_DELTA_DATA,
            ),
            COMPACT_STATUS_DELTA_DATA
        );
        assert_eq!(
            compact_status_for_global_interpretation(
                51,
                DOMAIN51_STATUS_DELAYED_EVENTS_ROOT_UNRESOLVED
                    | DOMAIN51_STATUS_SCRIPTED_GUI_ROOT_UNRESOLVED
                    | COMPACT_STATUS_PARTIAL_DATA,
            ),
            COMPACT_STATUS_PARTIAL_DATA
        );
    }

    #[test]
    fn expected_partial_tolerances_are_exactly_documented_domains() {
        let domains = COMPACT_EXPECTED_PARTIAL_TOLERANCES
            .iter()
            .map(|tolerance| tolerance.domain_id)
            .collect::<Vec<_>>();
        assert_eq!(domains, vec![26, 27, 28, 41, 45, 48, 49, 50, 54, 65]);
    }

    #[test]
    fn expected_partial_tolerances_only_grant_partial_data() {
        for tolerance in COMPACT_EXPECTED_PARTIAL_TOLERANCES {
            assert_eq!(
                tolerance.tolerated_flags, COMPACT_STATUS_PARTIAL_DATA,
                "domain {} grants unexpected tolerance flags {:#x}",
                tolerance.domain_id, tolerance.tolerated_flags
            );
            assert_eq!(tolerance.tolerated_flags & !COMPACT_STATUS_PARTIAL_DATA, 0);
        }
    }

    #[test]
    fn expected_partial_tolerance_lookup_rejects_unaccepted_partial_domains() {
        for domain_id in [29, 47, 56] {
            assert_eq!(expected_partial_tolerance(domain_id), None);
        }
        for domain_id in [26, 27, 28, 41, 45, 48, 49, 50, 54, 65] {
            assert_eq!(
                expected_partial_tolerance(domain_id).map(|tolerance| tolerance.tolerated_flags),
                Some(COMPACT_STATUS_PARTIAL_DATA)
            );
        }
    }

    #[test]
    fn expected_partial_tolerance_applies_only_without_untolerated_failure_bits() {
        assert_eq!(
            expected_partial_tolerance_applies(27, COMPACT_STATUS_PARTIAL_DATA)
                .map(|tolerance| tolerance.domain_id),
            Some(27)
        );
        assert_eq!(
            expected_partial_tolerance_applies(
                27,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_DELTA_DATA,
            )
            .map(|tolerance| tolerance.domain_id),
            Some(27)
        );
        assert_eq!(
            expected_partial_tolerance_applies(
                27,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE,
            ),
            None
        );
        assert_eq!(
            expected_partial_tolerance_applies(26, COMPACT_STATUS_PARTIAL_DATA),
            expected_partial_tolerance(26)
        );
    }

    #[test]
    fn expected_partial_tolerance_failure_flags_keep_untolerated_partials() {
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                27,
                COMPACT_STATUS_PARTIAL_DATA,
            ),
            0
        );
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(
                27,
                COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE,
            ),
            COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE
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
