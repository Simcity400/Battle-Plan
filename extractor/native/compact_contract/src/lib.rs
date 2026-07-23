//! Shared compact-stream contract: the single source of truth for compact
//! domain descriptors, H4CR record framing constants, stream status flags,
//! and the per-build offset/sentinel profile.
//!
//! The collector DLL, the recorder host, and every lane crate must consume
//! these definitions instead of keeping local copies. Domain IDs are assigned
//! in `extractor/documents/DOMAIN_REGISTRY.md`; the `ASSIGNED_COMPACT_DOMAINS`
//! table below must stay in sync with that registry.

pub mod bound_trips;
pub mod build_profile;
pub mod building_registry;
pub mod calendar;
pub mod campaign_identity;
pub mod campaign_rollover;
pub mod cid1_lifecycle_evidence;
pub mod control_transport;
pub mod dynamic_modifiers;
pub mod early_render_content_manifest;
pub mod early_render_control;
pub mod early_render_runtime_dlc;
pub mod early_render_vfs;
pub mod early_render_vfs_payload;
pub mod first_compact_ack;
pub mod identity_control;
pub mod national_focus_presentation;
pub mod naval_history;
pub mod preferred_tactic;
pub mod pov_visibility;
pub mod spotted_units;
pub mod tactic_weight;
pub mod rail_replay;
pub mod raid_instances;
pub mod record_header;
pub mod record_semantics;
pub mod sink_failure;
pub mod status_flags_base;
pub mod status_origins;
pub mod stream_validator;
pub mod tech_sharing_groups;

pub use record_header::{
    parse_compact_record_header, CompactRecordHeader, RecordFramingError, COMPACT_MAX_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION,
};
pub use record_semantics::*;
pub use status_flags_base::*;

pub type CompactDomainId = u16;

// --- H4CR compact record framing -------------------------------------------

/// Bounded control-plane roster capacity. The production profile currently
/// contains more than the historical 64-domain ceiling after the reviewed
/// exact, fail-closed lanes were promoted; retain explicit headroom without
/// making the parser unbounded.
pub const MAX_SCHEDULED_DOMAINS: usize = 96;

// H4CR parsing is implemented in the dependency-free `record_header` module
// and envelope semantics in `record_semantics`; both are re-exported above so
// Cargo and direct-rustc consumers share one source.

// --- Compact stream status flags (record header `flags` field) -------------

/// d17 diplomacy: the war-relations SEGMENT (per-country `at_war` +
/// casualties) was NOT captured this frame because no well-formed
/// `war_relation_bridge` was supplied (production presets pass none). Without
/// this bit the absence was SILENT — per-row `at_war: None` is ambiguous with
/// "not at war", and only the positive `WAR_BRIDGE_APPLIED` bit existed
/// (absence detectable only by a missing bit, i.e. not first-class). This is
/// explicit segment ACCOUNTING (zero-silent-gap law), NOT a failure bit: the
/// rest of the diplomacy frame is complete, d53 carries wars natively in
/// production, and the SourceDisposition design (O8) will type this segment
/// properly. Never used to mask or excuse anything.

// Domain 51 `scripted_state_flags_variables_live` carries lane-local badges in
// the shared status field for legacy wire compatibility. These bits intentionally
// overlap global bits and must be masked before global status interpretation.

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

/// Classifies the known v1 failure vocabulary.
///
/// This is not a status-validity predicate. Callers handling untrusted status
/// words must first reject a non-zero [`compact_status_unknown_flags_v1`]
/// result; otherwise an unassigned bit has no defined classification.
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
        reason: "division_equipment: only a sampled-root shortfall is tolerated; counts are serialized but omitted division identities are not, so the sparse patch aligns the retained accumulator and the domain/hour remains unknown",
    },
    CompactExpectedPartialTolerance {
        domain_id: 27,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "army_orders: only invalid army-root identity or an unreadable army-order walk is tolerated; the collector freezes identity/generation state, and the domain/hour remains unknown because omitted order identities are not serialized",
    },
    CompactExpectedPartialTolerance {
        domain_id: 28,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "oob_command: only invalid army-root or naval task-force-root rows are tolerated; the collector freezes additions/deletes/linkage changes, and the domain/hour remains unknown because omitted command identities are not serialized",
    },
    CompactExpectedPartialTolerance {
        domain_id: 54,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "army_battleplans_depth_live: inherited d27/d28 partials or counted non-exact/linkage/identity/active/path omissions are diagnostic only; no omitted-order identity list exists, so the domain/hour remains unknown",
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
pub const STATIC_ARTIFACT_TECHNOLOGY_SHARING_STATIC: &str = "technology_sharing_static";
pub const STATIC_ARTIFACT_FOCUS_DEFS_STATIC: &str = "focus_defs_static";

pub const REQUIRED_STATIC_ARTIFACTS: &[&str] = &[
    STATIC_ARTIFACT_TEMPLATES_EQUIPMENT_CATALOG,
    STATIC_ARTIFACT_LOCALISATION_JOIN,
    STATIC_ARTIFACT_DECISIONS_STATIC_DEFINITIONS,
    STATIC_ARTIFACT_IDEA_TECH_LOC_XREF_STATIC,
    STATIC_ARTIFACT_SHIP_DEFINITIONS_STATIC,
    STATIC_ARTIFACT_GFX_ICON_MANIFEST,
    STATIC_ARTIFACT_TECHNOLOGY_SHARING_STATIC,
    STATIC_ARTIFACT_FOCUS_DEFS_STATIC,
];

pub const PRODUCTION_PROFILE_RULE: &str =
    "mandatory schedulable compact domains must have status Existing, ProductionComplete, or ProductionCompleteBase; validated/partial/open-gap domains are never admitted to the mandatory production profile";

/// Feature-local companions which are intentionally not part of the
/// single-machine production contract. They remain individually schedulable
/// for explicit evidence/debug captures, but production neither requests nor
/// substitutes for them.
///
/// D57's multiplayer-only arm is excluded until CID1 M3 can be exercised on a
/// second physical machine. This exception must stay explicit and domain-local.
pub const OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS: &[CompactDomainId] = &[57, 78, 84];

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

/// Returns true only for statuses which may enter the mandatory production
/// recording profile. `Existing` is the legacy spelling for the original
/// production-complete compact domains.
pub fn status_in_production_profile(status: CompactDomainStatus) -> bool {
    matches!(
        status,
        CompactDomainStatus::Existing
            | CompactDomainStatus::ProductionComplete
            | CompactDomainStatus::ProductionCompleteBase
    )
}

pub fn is_optional_fail_closed_companion(id: CompactDomainId) -> bool {
    OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS.contains(&id)
}

/// Mandatory-by-default is deliberate: adding a schedulable domain can never
/// silently broaden production with a partial/open-gap contract. A domain must
/// either be production-complete or be reviewed into the explicit optional
/// companion list.
pub fn mandatory_profile_domains_not_production_complete() -> Vec<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && !status_in_production_profile(descriptor.status)
        })
        .collect()
}

pub fn production_profile_domains() -> Vec<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && status_in_production_profile(descriptor.status)
        })
        .collect()
}

pub fn production_profile_scheduled_names() -> Vec<&'static str> {
    production_profile_domains()
        .into_iter()
        .map(|descriptor| descriptor.wire_name)
        .collect()
}

/// Capture profile: one coherent preset cluster selected with a single
/// `--profile production|debug` switch instead of hand-assembling the individual
/// collector flags.
///
/// Both profiles run the IDENTICAL compact domain readers, the IDENTICAL
/// DOMAIN_DATA schemas, and the IDENTICAL fold. Only emission/diagnostics
/// verbosity differs. `stream_format=compact` in BOTH — the JSON stream format
/// is a divergent dead path that bypasses production readers and must never be a
/// profile.
///
/// This preset is the SINGLE SOURCE OF TRUTH shared by the recorder host (which
/// resolves it into the individual `collector_control.json` fields) and the
/// collector DLL (which re-applies it as a fallback for any preset field a
/// control file leaves unset — e.g. the Python dev harness). In BOTH hosts an
/// explicitly-set field always wins over the preset value.
///
/// Fail-closed data-loss detection is NOT a profile choice and is therefore not
/// represented here: skipped-hour accounting, per-domain SOURCE_UNAVAILABLE /
/// partial-data signaling, and the per-hour DOMAIN_PERF coverage heartbeat are
/// identical and always-on in both profiles. Only instrumentation COST (per-read
/// RC counters, phase-timing sidecar logs, full country-frame provenance) is
/// toggled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureProfile {
    /// Lean production capture: instrumentation cost stripped, data-loss
    /// detection fully preserved, strict acceptance gate.
    Production,
    /// Full-diagnostics capture: per-read RC counters, phase-timing sidecar logs,
    /// and full per-country provenance on; relaxed acceptance gate.
    Debug,
}

/// The resolved preset field cluster for a [`CaptureProfile`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureProfilePreset {
    /// Compact typed stream in both profiles (never JSON).
    pub stream_format_compact: bool,
    /// Expand the scheduled set to the production domain profile.
    pub use_production_domains: bool,
    /// Compact country frame (true) vs full per-country provenance (false).
    pub compact_country_frame: bool,
    /// Bounded per-frame reader fan-out (0 = unbounded / legacy).
    pub reader_concurrency: u32,
    /// Per-read RC counters + the `EXTRACTION_PERF_GATE` status marker. This is
    /// pure instrumentation cost; the DOMAIN_PERF coverage records that carry the
    /// fail-closed per-domain status flags are emitted regardless of this bit.
    pub extraction_perf_gate: bool,
    /// Phase-timing sidecar logs (`collector_attach.jsonl`,
    /// `collector_compact_phase_timing.jsonl`).
    pub sidecar_log: bool,
    /// Skip the root-frame country-identity scan. INERT for the production domain
    /// set — the scheduled country domains (fuel/production/focus/tech/politics/
    /// decisions/diplomacy/construction/supply/...) force the scan via
    /// `lightweight_root_needs_country_identity`, and the identity rows re-key
    /// downstream DOMAIN_DATA by TAG — so it is OFF in both profiles.
    pub lightweight_root_frame: bool,
    /// Acceptance minimum of advanced game-hours.
    pub min_game_hours: i32,
}

impl CaptureProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureProfile::Production => "production",
            CaptureProfile::Debug => "debug",
        }
    }

    pub fn parse(value: &str) -> Option<CaptureProfile> {
        match value {
            "production" => Some(CaptureProfile::Production),
            "debug" => Some(CaptureProfile::Debug),
            _ => None,
        }
    }

    pub fn preset(self) -> CaptureProfilePreset {
        match self {
            CaptureProfile::Production => CaptureProfilePreset {
                stream_format_compact: true,
                use_production_domains: true,
                compact_country_frame: true,
                reader_concurrency: 12,
                extraction_perf_gate: false,
                sidecar_log: false,
                lightweight_root_frame: false,
                min_game_hours: 48,
            },
            CaptureProfile::Debug => CaptureProfilePreset {
                stream_format_compact: true,
                use_production_domains: true,
                compact_country_frame: false,
                reader_concurrency: 0,
                extraction_perf_gate: true,
                sidecar_log: true,
                lightweight_root_frame: false,
                min_game_hours: 12,
            },
        }
    }
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
        CompactDomainStatus::ProductionComplete,
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
        CompactDomainStatus::ProductionComplete,
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
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        42,
        "air_combat_losses_live",
        "air_combat_losses_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        43,
        "market_trade_live",
        "market_trade_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        44,
        "supply_logistics_attrition_live",
        "supply_logistics_attrition_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        45,
        "focus_research_decisions_depth_live",
        "focus_research_decisions_depth_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        46,
        "air_variant_archetypes_live",
        "air_variant_archetypes",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        47,
        "characters_leaders_cabinet_live",
        "characters_leaders_cabinet_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        48,
        "construction_buildings_state_damage_live",
        "construction_buildings_state_damage_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        49,
        "naval_combat_lifecycle_live",
        "naval_combat_lifecycle_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        50,
        "naval_combat_members_live",
        "naval_combat_members_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        51,
        "scripted_state_flags_variables_live",
        "scripted_state_flags_variables_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        52,
        "occupation_resistance_compliance_live",
        "occupation_resistance_compliance_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        53,
        "diplomacy_intel_operatives_live",
        "diplomacy_intel_operatives_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        54,
        "army_battleplans_depth_live",
        "army_battleplans_depth_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        55,
        "land_combat_detail_events_live",
        "land_combat_detail_events_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        56,
        "naval_missions_status_live",
        "naval_missions_status_live",
        CompactDomainStatus::ProductionComplete,
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
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // C1 (2026-07-05): per-country CUMULATIVE air-combat loss statistics (statistic id 23,
    // the 89-field CLoopHistory record). The corrected yearly-window chain is
    // D04 value-validated: 23/23 countries, 2047/2047 fields, 0 mismatches.
    // All 24 strategic-air details channels are engine-table/localisation proven and emitted.
    domain(
        61,
        "air_loss_stats_live",
        "air_loss_stats_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // P1/A1: exact node-level supply-hub motorization from the persistent
    // CCountrySupplySystem settings object. D04 validates 223/223 outer rows,
    // 1338/1338 fields, and 483/483 map entries; schema v2 also carries the
    // proven inner actor attribution and exact levels 1/2 (XN07 joint gate).
    domain(
        62,
        "hub_motorization_live",
        "hub_motorization_live",
        CompactDomainStatus::ProductionComplete,
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
        CompactDomainStatus::ProductionComplete,
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
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // W3 (2026-07-05): per-country OWNER-JOINED special-project program
    // facilities, ROOTED walk of CCountry+0xFA8 -> CProgramStatus (array of up
    // to 3 records, stride 0x160, each record's own vtable == mb+0x295AC88) ->
    // record+0x20 vector<CProgram*> begin, record+0x28 packed u64 (low32=count,
    // high32=capacity). Per CProgram (vtable mb+0x295AC30): program_id (+0x0C),
    // duplicated serialized owner (+0x18/+0x170), duplicated province
    // (+0x1C/+0x174), and unread_prototype_rewards (+0x178). The row is
    // admitted only when both duplicated pairs agree inside two equal coherent
    // object snapshots and the owner exists in the validated country table.
    // Stale cross-country traversal aliases therefore emit once under the
    // object's intrinsic owner. Distinct from committed d41 projects_live.
    domain(
        65,
        "special_project_programs_live",
        "special_project_programs_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // W4 (2026-07-05): per-occupied-state occupation EXTRAS, a NEW rooted
    // per-state domain that keeps the forbidden d52
    // occupation_resistance_compliance_live lane byte-identical. Two fields
    // (agent D5, validated vs the D04 melt oracle):
    //   * occupation_law_token_id: the state's occupation law FNV1a32 token,
    //     read from the controller's CCountryOccupationData+0xB0 PDX
    //     open-addressing hashmap (0x18-byte buckets: state_id u32 @+0x08,
    //     law* u64 @+0x10 -> CBuildingOccupationLaw vtable mb+0x2928218, PDX SSO
    //     token @+0x10). Same FNV scheme as the committed d52 country
    //     default_law token. D5: 279/279 live entries correct, 0 wrong; negative
    //     control at bucket+0x00 gave 0/1155. This is the LIVE/EFFECTIVE law per
    //     state (a same-value residual vs the serialized occupation_law_list was
    //     flagged at discovery but showed 0 contradiction on the oracle), hence
    //     validated_slice_open_gaps.
    //   * garrison_manpower_source_tag_db_id: CStateGarrisonData+0x70 sub-map
    //     +0x00 u32 supplying-country tag_db_id (who supplies garrison manpower;
    //     usually the controller, occasionally not). D5: 276/276 tag match. The
    //     adjacent +0x04 value is redundant with the committed d52
    //     army_manpower_need and is NOT re-emitted here.
    // Each field carries its own presence bit; a bucket/garrison that fails any
    // guard is badged ABSENT (never fabricated). A row is emitted when EITHER
    // field is present and the (controller, occupied_state) key is valid.
    domain(
        66,
        "occupation_state_extras_live",
        "occupation_state_extras_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // W5 (2026-07-05): per-country DYNAMIC MODIFIER id set, a NEW rooted
    // per-country domain carrying the country's OWN active dynamic-modifier
    // definition ids (agent D2). ROOTED walk of the inline
    // CDynamicModifierContainer at CCountry+0xE58 (own vtable
    // mb+0x27BFDE8; +0x08 backref u64 == the owning CCountry, hard guard),
    // entry array ptr @+0x28, u32 count @+0x34, entry stride 0x40, each entry
    // +0x18 -> CDynamicModifier (mb+0x27BFD48), and the modifier definition id
    // is that def's MSVC std::string @+0x28. D2 discovery validation vs the D04
    // melt oracle: 6/6 countries exact multiset (GER 9, SOV 7, USA 1, ENG 1,
    // ITA 5, JAP 10) plus a database-wide cross-check (159 tokens / 47 countries
    // / 144 distinct ids, ABDA_supreme_command x5, CHI_warlord_army_dynamic_modifier
    // x6) exactly matching the melt. This is a VARIABLE-LENGTH per-country string
    // set that does not fit the fixed-width d63 country_core_scalars row, hence a
    // new rooted domain (mirrors the d64 delayed_events_live precedent for a
    // variable-length per-country repeated collection). Per-instance
    // value/expiry is UNRESOLVED-BY-ABSENCE in D04 (no timed country modifier)
    // and is intentionally not emitted; only the membership id set is carried.
    // 390/437 countries have count==0 (the array is never walked when count==0).
    domain(
        67,
        "country_dynamic_modifiers_live",
        "country_dynamic_modifiers_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // W6 (2026-07-05): TOP-LEVEL session header, a NEW rooted top-level
    // (non-per-country) domain snapshotting the mostly-static save/session
    // header off the CGameState singleton (agent D3, every field live-validated
    // vs the D04 melt oracle at discovery). Root gs = u64[mb+0x3316E90]. Fields
    // (each presence-flagged, fail-closed): player tag_db_id (CHuman(gs+0xA8)
    // +0x70; melt player="D04"), ideology (CIdeologyGroup(gs+0xC8)+0x18 MSVC;
    // melt fascism), cosmetic_tag (gs+0x70 MSVC; melt "FRA_VICHY"), dlcs (gs+0xD4
    // u32; melt 257535), start_date (gs+0x4A8 u32 hour-count; melt 1936.1.1.12),
    // save_version (gs+0x58 u32; melt 33; provisional), session (gs+0x490 u32;
    // melt 43695; provisional), version (module .rdata RVA 0x2701108 banner;
    // best-effort, not a save field). Emitted as a full record every frame;
    // the fold is last-value-wins (static fields never change; player/ideology/
    // cosmetic_tag are change-detected downstream from the last emitted value).
    // DISTINCT from d57 session_multiplayer_metadata_live (player ROSTER + pause
    // event), which is left untouched — d68 carries the header scalars/strings
    // d57 does not.
    domain(
        68,
        "session_header_live",
        "session_header_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // W6 (2026-07-05): TOP-LEVEL tech-sharing groups, a NEW rooted top-level
    // domain (agent D8, 9/9 groups + membership multisets validated vs the D04
    // melt oracle at discovery). Root gs = u64[mb+0x3316E90]; CGameState+0x3A0 ->
    // tech-sharing manager holding 9 inline pointer slots (+0x00..+0x40 stride 8),
    // each -> CTechnologySharingGroup (vtable RVA 0x270B720). Per group: id token
    // (+0x08 u32), member tag_db_ids (begin +0x18, count +0x20 u32, stride 4). A
    // full-heap vtable census found EXACTLY 9 objects == the melt's 9 groups
    // (commonwealth_research{6}, CHI_technological_cooperation{10}, SOV_soviet
    // _academies{2}, HUN_joint_air{7}, BUL_italian{2}, FIN_joint{1}, MEX_USA{2},
    // BUL_axis{2}, VIC_learn_from_the_enemy{2}). The static group id STRING lives
    // behind the +0x48 template ptr and is NOT decoded here (only the id token +
    // member tag_db_id set); the replay resolves the token->name via installed content.
    domain(
        69,
        "tech_sharing_groups_live",
        "tech_sharing_groups_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A21/B4 (2026-07-17): global sunk-convoy history. The authoritative root
    // is CGameState+0x5A8 (pointer vector; count +0x5B4), whose 0x18-byte
    // CSunkConvoyInfo rows store month/count/killer/owner at +0x08..+0x14.
    // This is a full persistent history snapshot and is intentionally distinct
    // from d49 active naval combats and CShip cumulative counters. Static law
    // and the fail-closed SCH1/v3 integration are complete. The paused
    // raw/save/production equality receipt is optional evidence and does not
    // gate the ordinary exact reader.
    domain(
        70,
        "sunk_convoy_history_live",
        "sunk_convoy_history_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A28/XUI03: exact passive Air-selection and paired MIO-history controller
    // traces. Hook install is executable-hash and sentinel gated; callback
    // observations are clock-bound and fail closed. The heterogeneous MIO
    // aggregate-bonus producer is bound to the same D71 session. Technology
    // canvas remains omitted until its selected-Present producer is exact.
    domain(
        71,
        "dynamic_collection_trace_live",
        "dynamic_collection_trace_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A26/XUI05: the ordinary collector owns the exact-a729, sentinel-gated
    // tooltip-controller hook and seals its passive observations against the
    // selected Present. TTI2 preserves exact final post-layout text,
    // controller-owned geometry and authored identities. Missing installation,
    // observation, clock, or Present emits typed unavailable and can never be
    // interpreted as measured-hidden. Visibility matrices remain optional
    // regression coverage rather than a production activation prerequisite.
    domain(
        72,
        "resolved_tooltip_instances_live",
        "resolved_tooltip_instances_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // Retired A24 semantic decomposition. Production politics state is already
    // carried by d14/d15, exact visible tooltip text/geometry by scheduled d72,
    // and all XUI11 pixels by AuthenticatedUiPass. PTI1 is retained only for
    // backwards-compatible forensic decoding; it is deliberately unscheduled
    // because it has no production consumer or independently visible pixels.
    domain(
        73,
        "politics_tooltip_inputs_live",
        "politics_tooltip_inputs_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // A37/XUI18: optional typed power-balance semantic companion. Exact visible
    // product pixels are owned by the mandatory authenticated selected-Present
    // UI pass, not reconstructed from d74. PBP2 deliberately remains reserved:
    // its open-panel-only contract cannot represent ordinary passive closed
    // hours and it must never substitute for the selected-Present pixel path.
    domain(
        74,
        "power_balance_presentation_live",
        "power_balance_presentation_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // XUI21: authenticated fired-event hook stream, intentionally distinct
    // from d64 pending delayed-event polling. The collector owns the hash-gated
    // game-file registry bridge, exact a729 install/uninstall lifecycle, hourly
    // FEV1 emission, and typed incomplete-frame fallback. That ordinary
    // production route is complete and fail-closed; broader popup-class
    // coverage remains optional regression evidence, not an activation gate.
    domain(
        75,
        "fired_event_popups_live",
        "fired_event_popups_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A25/XUI06 (2026-07-17): exact runtime-localised text. The d76 carrier
    // binds the authenticated fixed-window/textbox registration hook to the
    // final a729 text setter, retaining final bytes and exact authored
    // view/element identity. Scheduling remains explicit until recorder fold
    // integration is enabled as one atomic promotion.
    domain(
        76,
        "runtime_text_inputs_live",
        "runtime_text_inputs_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // XN12/XN13/XN14/XN15: exact post-simulation non-UI render submissions.
    // The neutral carrier, recorder fold, standalone ingest, and fail-closed
    // observer seams are implemented, but no live current-process descriptor
    // catalog builder exists. Gameplay domains and prebuilt pointer catalogs
    // are explicitly not substitutes, so d77 remains omitted from production.
    domain(
        77,
        "production_render_submits_live",
        "production_render_submits_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // A27/XUI13: exact equipment-designer view state captured after the
    // visibility-triggered a729 refresh. Pointer-free d78 contains current
    // country, raw selected-tag vector, filter toggles, family, and final
    // accepted design-id order. It is distinct from d59's static catalog.
    domain(
        78,
        "design_catalog_context_live",
        "design_catalog_context_live",
        CompactDomainStatus::ValidatedSlice,
        true,
        true,
        &[],
    ),
    // Runtime proof for the four fixed HUD controller roots and the optional
    // active A27 designer family. Pointer-free d79 is consumed only to select
    // exact post-capture effective-VFS dependency closures.
    domain(
        79,
        "hud_surface_activation_live",
        "hud_surface_activation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // Exact pointer-free effective-VFS content observations for the 11
    // production-map semantic consumers. d80 is deliberately separate from
    // d77's ProductionRenderFrame codec. The fixed filename classifier was
    // removed; keep omitted until normal --production builds the semantic-role
    // closures itself from the admitted install/profile/effective-VFS inputs.
    domain(
        80,
        "production_map_content_snapshot_live",
        "production_map_content_snapshot_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    // Exact CRailwayGfx producer carrier for 1.19.2.0.a729. This snapshots
    // per-province engine-selected anchors plus the ordered RailwayInfo
    // law is known but its complete live owner root remains unavailable, so
    // that independent sub-scope is explicitly fail-closed in schema v1.
    domain(
        81,
        "rail_replay_state_live",
        "rail_replay_state_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A39/XUI17 (2026-07-20): exact pointer-free observations from the
    // CUnitLeaderTraitTree row vectors/state producer and the owner-qualified
    // CAcesView refresh/bind portrait route. The ordinary collector installs
    // the authenticated hook, emits CPR1 or typed source-unavailable, then
    // emits its owned d76 dependency in the enforced same-hour order. Carrier,
    // fold and replay admission are complete; mixed-state observations remain
    // optional regression coverage and are not an activation prerequisite.
    domain(
        82,
        "character_presentation_live",
        "character_presentation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A17 (2026-07-20): exact pointer-free per-country CNuke rows carrying
    // amount_raw and nukes_ready. Exact20 function/data authority is bound to
    // the collector's double-stable, vtable/tag-checked current-process country
    // roster. Source failure emits explicit unavailable; it never certifies an
    // empty stockpile. d63's established 48-byte row remains unchanged. The
    // complete fail-closed source, fold, replay and topbar consumer are
    // production-authorized; a positive fixture is optional regression
    // evidence and is not an activation prerequisite.
    domain(
        83,
        "nuke_stockpile_live",
        "nuke_stockpile_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A38/XUI22 (2026-07-20): exact engine-materialised scripted-GUI runtime
    // results in CScriptedWindowManager updater order, bound to the exact
    // effective-VFS winner and authenticated static producer receipt. The d84
    // carrier, recorder fold, replay typed view, and generic UI consumer are
    // implemented. Production now requests every effective scripted-GUI
    // source payload through H4CT, compiles the exact registry in-process, and
    // emits only the post-owner frame; missing/ambiguous consumer joins remain
    // explicit unavailable values rather than inferred defaults.
    domain(
        84,
        "scripted_gui_runtime_live",
        "scripted_gui_runtime_live",
        CompactDomainStatus::ValidatedSlice,
        true,
        false,
        &[],
    ),
    // A40/XUI08 (2026-07-20): pointer-free selected designer identity and
    // exact controller-published row order across division, equipment, tank,
    // plane, ship, and variant-upgrade families. The d85 codec, isolated
    // collector seam, recorder fold, and replay typed view are implemented.
    // Producer plus split classifier/thunk receipts are bound. The ordinary
    // collector owns the passive hook lifecycle and emits typed unavailable
    // until a complete transition has actually been observed; the six-family
    // fixture remains regression coverage rather than a scheduling gate.
    domain(
        85,
        "selected_designer_identity_live",
        "selected_designer_identity_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A33/XUI20 (2026-07-20): exact sunk-ship writer observations joined to
    // the validated d49/d50 CFEXMember combat epoch. The pointer-free d86
    // carrier is receipt-bound and fail-closed. Missing hook/baseline authority
    // emits typed unavailable; the live sink fixture is optional evidence.
    domain(
        86,
        "sunk_ship_history_live",
        "sunk_ship_history_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A34/XUI19 (2026-07-23): exact ordered decision-panel materialization,
    // evaluated text, alert membership and engine interface state. The ordinary
    // collector owns the passive epoch, selected-Present seal, authenticated
    // effective decision catalog and same-hour country decision join. Missing
    // or ambiguous leaves emit typed unavailable; no semantic values are
    // fabricated.
    domain(
        87,
        "decision_panel_live",
        "decision_panel_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A36/XUI23 (2026-07-20): exact selected-context active-command-ability
    // rows wrapped in a receipt-bound ABA2 envelope. The nested ABL1 payload
    // preserves source order, engine-evaluated core state/timers, target and
    // affected-division identities. Evaluated cost/final text are optional
    // leaves and remain absent unless observed exactly; they do not gate core
    // row scheduling.
    domain(
        88,
        "active_command_abilities_live",
        "active_command_abilities_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A31/XUI10 (2026-07-20): final CNationalFocusView item order and exact
    // per-item presentation leaves, with every process address replaced by a
    // stable focus-definition id. Static exact20 authority and the d89
    // codec/fold/replay/UI route are complete. The ordinary collector owns a
    // continuously re-armed passive epoch, exact inline leaves and the
    // selected-Present seal; absent refreshes remain typed unavailable.
    domain(
        89,
        "national_focus_presentation_live",
        "national_focus_presentation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // H-CONVOY (2026-07-20): exact pre-collapse terminal convoy event with
    // complete two-word client identity, generic-reference producer join,
    // serializer-owned endpoints, combat epoch and same-call D70 mutation.
    // CTA2 includes cumulative hook/drop/queue/publication custody. Fresh
    // exact-a729 static authority pins the callback, reference resolver,
    // serializers, and same-call d70 mutation proof. Runtime custody remains
    // fail closed on any hook, resolver, queue, or decode loss.
    domain(
        90,
        "convoy_transport_attack_live",
        "convoy_transport_attack_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // B7 (2026-07-20): RID1 preserves the intended CGameState+0x3F0 inline
    // CCountryRaidStatus order, each +0x18 child-vector order, the unique
    // type-84 reference identity, dummy parent and lifecycle scalars. The
    // collector owns the authenticated current-process owner walk: exact
    // vtables, bounded/double-stable producer vectors and rows, roster-bound
    // country ids, pointer erasure, and typed unavailable on any torn read.
    domain(
        91,
        "raid_instances_live",
        "raid_instances_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // E25 (2026-07-21): exact CCountry+0x15D0 preferred-tactic definition,
    // reduced to the authenticated database id and stable definition token.
    // PTC1 is a complete pointer-free country snapshot with exact none
    // semantics and embedded executable/static-evidence receipts.
    domain(
        92,
        "country_preferred_tactic_live",
        "country_preferred_tactic_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // E20 exact-a729 CCountry+0xF80 CArmyUpgradesStatus snapshot. AUR1
    // authenticates the executable plus request/delivery/pool/CArmy evidence,
    // uses CArmy persistent IDs as group identity, and preserves all fixed-5
    // request, total, delivery-progress, and produced-amount integers.
    domain(
        93,
        "country_army_upgrade_requests_live",
        "country_army_upgrade_requests_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // G3 (2026-07-21): exact current-selected-POV province-intel vector.
    // FOW1 preserves the complete raw u8 vector indexed by province database
    // ID. Unit spotting remains a separate identity domain and is not
    // fabricated from these graded province bytes.
    domain(
        94,
        "pov_province_visibility_live",
        "pov_province_visibility_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // G3 (2026-07-21): exact current-POV land and task-force map producer
    // frames. SPU1 retains the engine's pointer-free CUnit reference pair and
    // raw graded intel byte; air-wing spotting is deliberately not inferred.
    domain(
        95,
        "pov_spotted_units_live",
        "pov_spotted_units_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // E25 (2026-07-21): exact selected-tactic probability inputs. PTW1 joins
    // the current-hour D18 roster and D92 selection snapshot, preserving the
    // signed fixed-five country factor and per-country modifier accumulator
    // result used by the land-combat weight consumer. Runtime pointers never
    // enter the carrier; tactic database id plus token is portable identity.
    domain(
        96,
        "country_preferred_tactic_weight_live",
        "country_preferred_tactic_weight_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    // A40/XUI08 exact peace-conference refresh presence. PCS1 records the
    // registered two-word conference reference plus proven turn/completion
    // scalars. Allocation/result identity is deliberately snapshot-scoped;
    // v1 leaves the unproven semantic collections typed unavailable.
    domain(
        97,
        "peace_conference_live",
        "peace_conference_live",
        CompactDomainStatus::ProductionComplete,
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

    fn header_bytes(
        version: u16,
        header_len: u16,
        game_hour_raw: i32,
        domain_id: u16,
        record_type: u16,
        payload_len: u32,
        flags: u32,
    ) -> [u8; COMPACT_RECORD_HEADER_LEN] {
        let mut b = [0u8; COMPACT_RECORD_HEADER_LEN];
        b[0..4].copy_from_slice(COMPACT_RECORD_MAGIC);
        b[4..6].copy_from_slice(&version.to_le_bytes());
        b[6..8].copy_from_slice(&header_len.to_le_bytes());
        b[8..16].copy_from_slice(&7u64.to_le_bytes());
        b[16..20].copy_from_slice(&game_hour_raw.to_le_bytes());
        b[20..22].copy_from_slice(&domain_id.to_le_bytes());
        b[22..24].copy_from_slice(&record_type.to_le_bytes());
        b[24..28].copy_from_slice(&payload_len.to_le_bytes());
        b[28..32].copy_from_slice(&flags.to_le_bytes());
        b
    }

    #[test]
    fn record_header_round_trips_and_rejects_malformed() {
        let ok = header_bytes(
            COMPACT_RECORD_VERSION,
            COMPACT_RECORD_HEADER_LEN as u16,
            123,
            53,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            9,
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
        );
        let h = parse_compact_record_header(&ok).expect("valid header");
        assert_eq!(h.sequence, 7);
        assert_eq!(h.game_hour_raw, 123);
        assert_eq!(h.domain_id, 53);
        assert_eq!(h.record_type, COMPACT_RECORD_TYPE_DOMAIN_DATA);
        assert_eq!(h.payload_len, 9);
        assert_eq!(h.record_flags, COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2);
        assert_eq!(h.record_len(), COMPACT_RECORD_HEADER_LEN + 9);

        let mut bad_magic = ok;
        bad_magic[0] = b'X';
        assert_eq!(
            parse_compact_record_header(&bad_magic),
            Err(RecordFramingError::BadMagic)
        );

        let bad_version = header_bytes(2, COMPACT_RECORD_HEADER_LEN as u16, 0, 1, 2, 0, 0);
        assert_eq!(
            parse_compact_record_header(&bad_version),
            Err(RecordFramingError::UnsupportedVersion(2))
        );

        let bad_hlen = header_bytes(COMPACT_RECORD_VERSION, 40, 0, 1, 2, 0, 0);
        assert_eq!(
            parse_compact_record_header(&bad_hlen),
            Err(RecordFramingError::UnsupportedHeaderLen(40))
        );
    }

    #[test]
    fn record_header_flag_policy_is_domain_and_record_type_specific() {
        assert_eq!(
            compact_record_allowed_flags(45, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(67, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(10, COMPACT_RECORD_TYPE_DOMAIN_PERF),
            COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER
        );
        assert_eq!(
            compact_record_allowed_flags(10, COMPACT_RECORD_TYPE_DOMAIN_DATA),
            0
        );
        assert_eq!(
            compact_record_rejected_flags(
                67,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            ),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
        assert_eq!(
            compact_record_allowed_flags(1000, COMPACT_RECORD_TYPE_FRAME_END),
            0
        );
        assert_eq!(
            compact_record_rejected_flags(
                45,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 | (1 << 29),
            ),
            1 << 29
        );
        assert_eq!(
            compact_record_rejected_flags(
                45,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
            ),
            COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2
        );
    }

    #[test]
    fn capture_profile_round_trips_and_presets_are_coherent() {
        for profile in [CaptureProfile::Production, CaptureProfile::Debug] {
            assert_eq!(CaptureProfile::parse(profile.as_str()), Some(profile));
            let preset = profile.preset();
            // Both profiles stay compact and run the production domain set.
            assert!(preset.stream_format_compact);
            assert!(preset.use_production_domains);
            // lightweight_root_frame is inert for the production domain set: off in both.
            assert!(!preset.lightweight_root_frame);
        }
        assert_eq!(CaptureProfile::parse("json"), None);
        assert_eq!(CaptureProfile::parse(""), None);

        let production = CaptureProfile::Production.preset();
        assert!(production.compact_country_frame);
        assert_eq!(production.reader_concurrency, 12);
        assert!(!production.extraction_perf_gate);
        assert!(!production.sidecar_log);
        assert_eq!(production.min_game_hours, 48);

        let debug = CaptureProfile::Debug.preset();
        assert!(!debug.compact_country_frame);
        assert_eq!(debug.reader_concurrency, 0);
        assert!(debug.extraction_perf_gate);
        assert!(debug.sidecar_log);
        assert_eq!(debug.min_game_hours, 12);
    }

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
            55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 75, 76,
            77, 78, 79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97,
            1000,
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
        assert_eq!(id_for_scheduled_name("sunk_convoy_history_live"), Some(70));
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
    fn production_profile_contains_only_complete_mandatory_domains() {
        let profile = production_profile_domains();
        let profile_ids: std::collections::BTreeSet<CompactDomainId> =
            profile.iter().map(|descriptor| descriptor.id).collect();
        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            let expected = descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && status_in_production_profile(descriptor.status);
            assert_eq!(
                profile_ids.contains(&descriptor.id),
                expected,
                "domain {} mandatory profile membership should require a complete status",
                descriptor.id
            );
        }

        let incomplete = mandatory_profile_domains_not_production_complete();
        assert!(
            incomplete.is_empty(),
            "mandatory schedulable domains are not production-complete: {:?}",
            incomplete
                .iter()
                .map(|descriptor| (descriptor.id, descriptor.wire_name, descriptor.status))
                .collect::<Vec<_>>()
        );

        for optional in OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS {
            assert!(
                !profile_ids.contains(optional),
                "optional fail-closed companion domain {optional} must stay out of production"
            );
        }
        for excluded in [1, 58, 59, 73, 74, 76, 77, 80, 1000] {
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
                STATIC_ARTIFACT_TECHNOLOGY_SHARING_STATIC,
                STATIC_ARTIFACT_FOCUS_DEFS_STATIC,
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
        assert_eq!(domains, vec![26, 27, 28, 54]);
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
        for domain_id in [29, 41, 45, 47, 48, 49, 50, 53, 56, 65] {
            assert_eq!(expected_partial_tolerance(domain_id), None);
        }
        for domain_id in [26, 27, 28, 54] {
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
        assert_eq!(
            COMPACT_STATUS_FAILURE_MASK & !COMPACT_STATUS_V1_KNOWN_MASK,
            0,
            "all classified failure bits must belong to the v1 status vocabulary"
        );
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
            COMPACT_STATUS_WAR_SEGMENT_ABSENT,
        ] {
            assert!(
                !compact_status_is_failure(flag),
                "flag {flag:#x} should be informational"
            );
        }

        let unknown = 1_u32 << 31;
        assert_eq!(compact_status_unknown_flags_v1(unknown), unknown);
        assert!(
            !compact_status_is_failure(unknown),
            "failure classification remains known-vocabulary-only; callers reject unknown bits first"
        );
    }
}
