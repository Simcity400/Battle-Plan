pub mod bound_trips;
pub mod build_profile;
pub mod building_registry;
pub mod calendar;
pub mod campaign_identity;
pub mod campaign_rollover;
pub mod capture_termination;
pub mod cid1_lifecycle_evidence;
pub mod clock_unreadable_interval;
pub mod content_identity_census;
pub mod control_transport;
pub mod country_tag;
pub mod country_tag_db;
pub mod diagnostic_channel;
pub mod division_combat_stats;
pub mod domain_gap;
pub mod domain_outcome;
pub mod dynamic_modifiers;
pub mod early_render_content_manifest;
pub mod early_render_control;
pub mod early_render_runtime_dlc;
pub mod early_render_startup_failure;
pub mod early_render_vfs;
pub mod early_render_vfs_payload;
pub mod first_compact_ack;
pub mod frame_degraded;
pub mod frame_refusal;
pub mod freeze_census_record;
pub mod gap_record;
pub mod identity_control;
pub mod national_focus_presentation;
pub mod naval_history;
pub mod occupation_robin_hood;
pub mod passive_ready;
pub mod payload_span_delta;
pub mod pending_producer;
pub mod pov_visibility;
pub mod preferred_tactic;
pub mod raid_instances;
pub mod rail_replay;
pub mod record_header;
pub mod record_semantics;
pub mod sink_failure;
pub mod spotted_units;
pub mod state_topology_census;
pub mod status_flags_base;
pub mod status_origins;
pub mod stream_validator;
pub mod tactic_weight;
pub mod tech_sharing_groups;
pub mod untrusted_clock_recovery;
pub mod worker_error_record;
pub mod map_furniture_victory_points;
pub mod xmap07_snow_mud;

pub use domain_gap::{
    DomainGapError, DomainGapPart, DomainGapReason, DomainGapRecordV1, DomainGapWitnessProvenance,
};
pub use gap_record::{
    decode_gap_record, encode_gap_record, validate_gap_records, validate_gap_records_hour_ordered,
    GapCounter, GapReason, GapRecord, GapRecordCodecError, GapRecordValidationError, GapWitness,
    GAP_COUNTER_CODE_COALESCED_HOURS, GAP_COUNTER_CODE_QUEUE_OVERRUN_HOURS,
    GAP_COUNTER_CODE_SINK_BACKPRESSURE_HOURS, GAP_REASON_CODE_BACKPRESSURE,
    GAP_RECORD_LEGACY_VERSION, GAP_RECORD_MAGIC, GAP_RECORD_VERSION, MAX_GAP_DOMAIN_COUNT,
};
pub use record_header::{
    parse_compact_record_header, CompactRecordHeader, RecordFramingError, COMPACT_MAX_PAYLOAD_LEN,
    COMPACT_RECORD_HEADER_LEN, COMPACT_RECORD_MAGIC, COMPACT_RECORD_VERSION,
};
pub use record_semantics::*;
pub use status_flags_base::*;

pub type CompactDomainId = u16;

pub const MAX_SCHEDULED_DOMAINS: usize = 96;

pub const DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED: u32 = 1 << 11;
pub const DOMAIN49_LANE_LOCAL_STATUS_MASK: u32 = DOMAIN49_STATUS_LOCATION_CANDIDATE_UNVALIDATED;

pub const DOMAIN53_STATUS_OPERATIONS_DETAIL_RESISTED: u32 = 1 << 16;
pub const DOMAIN53_STATUS_INTEL_MAP_RESISTED: u32 = 1 << 17;
pub const DOMAIN53_STATUS_WAR_ROOT_UNVALIDATED: u32 = 1 << 18;
pub const DOMAIN53_STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED: u32 = 1 << 19;
pub const DOMAIN53_STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED: u32 = 1 << 20;
pub const DOMAIN53_STATUS_FACTION_GROUP_UNVALIDATED: u32 = 1 << 21;
pub const DOMAIN53_STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED: u32 = 1 << 22;
pub const DOMAIN53_LANE_LOCAL_STATUS_MASK: u32 = DOMAIN53_STATUS_OPERATIONS_DETAIL_RESISTED
    | DOMAIN53_STATUS_INTEL_MAP_RESISTED
    | DOMAIN53_STATUS_WAR_ROOT_UNVALIDATED
    | DOMAIN53_STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
    | DOMAIN53_STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED
    | DOMAIN53_STATUS_FACTION_GROUP_UNVALIDATED
    | DOMAIN53_STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;

pub const DOMAIN60_STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR: u32 = 1 << 16;
pub const DOMAIN60_STATUS_WAR_RESIDUALS_EXACT: u32 = 1 << 17;
pub const DOMAIN60_LANE_LOCAL_STATUS_MASK: u32 =
    DOMAIN60_STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR | DOMAIN60_STATUS_WAR_RESIDUALS_EXACT;

pub const fn lane_local_status_mask(domain_id: CompactDomainId) -> u32 {
    match domain_id {
        49 => DOMAIN49_LANE_LOCAL_STATUS_MASK,
        51 => DOMAIN51_LANE_LOCAL_STATUS_MASK,
        53 => DOMAIN53_LANE_LOCAL_STATUS_MASK,
        60 => DOMAIN60_LANE_LOCAL_STATUS_MASK,
        99 => crate::division_combat_stats::DOMAIN99_LANE_LOCAL_STATUS_MASK,
        _ => 0,
    }
}

pub const fn compact_status_for_global_interpretation(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> u32 {
    status_flags & !lane_local_status_mask(domain_id)
}

pub const fn compact_status_unassigned_flags(domain_id: CompactDomainId, status_flags: u32) -> u32 {
    compact_status_unknown_flags_v1(compact_status_for_global_interpretation(
        domain_id,
        status_flags,
    ))
}

pub fn compact_status_unassigned_bit_indices(unassigned_flags: u32) -> Vec<u32> {
    (0..u32::BITS)
        .filter(|bit| (unassigned_flags & (1_u32 << bit)) != 0)
        .collect()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactBaselineClearAdmission {
    pub domain_id: CompactDomainId,
    pub reason: &'static str,
}

pub const COMPACT_BASELINE_CLEAR_ADMISSIONS: &[CompactBaselineClearAdmission] =
    &[CompactBaselineClearAdmission {
        domain_id: 31,
        reason: "air_region_activity: a bucket row is an AGGREGATE over the whole live wing \
                 walk, and the reader badges PARTIAL_DATA when any wing row was invalid, so an \
                 absent bucket on a clean record means the bucket is empty rather than \
                 unmeasured. There is no per-key `None` on this domain and no unmeasured-entity \
                 case for a clean record to hide",
    }];

pub fn compact_baseline_clear_admissible(domain_id: CompactDomainId) -> bool {
    COMPACT_BASELINE_CLEAR_ADMISSIONS
        .iter()
        .any(|admission| admission.domain_id == domain_id)
}

pub fn compact_baseline_clear_requested(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> Result<bool, &'static str> {
    if status_flags & COMPACT_STATUS_BASELINE_CLEAR == 0 {
        return Ok(false);
    }
    if compact_status_baseline_clear_is_contradictory(status_flags) {
        return Err("BASELINE_CLEAR with DELTA_DATA: a patch cannot be a replacement");
    }
    if compact_status_is_failure(status_flags) {
        return Err(
            "BASELINE_CLEAR on a failure-class record: a read that did not complete cannot \
             describe the whole extent it asks the consumer to replace",
        );
    }
    if !compact_baseline_clear_admissible(domain_id) {
        return Err(
            "BASELINE_CLEAR from a domain that is not on COMPACT_BASELINE_CLEAR_ADMISSIONS: a \
             clean record of that domain is not a complete claim about its whole extent",
        );
    }
    Ok(true)
}

pub const fn compact_status_is_failure(status_flags: u32) -> bool {
    (status_flags & COMPACT_STATUS_FAILURE_MASK) != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactStatusFlagSpec {
    pub bit: u32,
    pub name: &'static str,
}

impl CompactStatusFlagSpec {
    pub const fn is_failure(&self) -> bool {
        (self.bit & COMPACT_STATUS_FAILURE_MASK) != 0
    }
}

pub const COMPACT_STATUS_V1_FLAG_SPECS: &[CompactStatusFlagSpec] = &[
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_STATE_UNAVAILABLE,
        name: "state_unavailable",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_PROFILE_MISMATCH,
        name: "profile_mismatch",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_NO_CLOCK,
        name: "no_clock",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_NO_SCALAR_SOURCE,
        name: "no_scalar_source",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_WAR_BRIDGE_APPLIED,
        name: "war_bridge_applied",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_EXTRACTION_PERF_GATE,
        name: "extraction_perf_gate",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_WORKER_PANIC,
        name: "worker_panic",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_BRIDGE_UNAVAILABLE,
        name: "bridge_unavailable",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_SOURCE_UNAVAILABLE,
        name: "source_unavailable",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_PARTIAL_DATA,
        name: "partial_data",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_DELTA_DATA,
        name: "delta_data",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_TRANSITION_BASELINE_RESET,
        name: "transition_baseline_reset",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_CONTROL_INVALID,
        name: "control_invalid",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_WAR_SEGMENT_ABSENT,
        name: "war_segment_absent",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_SOURCE_EMPTY,
        name: "source_empty",
    },
    CompactStatusFlagSpec {
        bit: COMPACT_STATUS_BASELINE_CLEAR,
        name: "baseline_clear",
    },
];

pub fn compact_status_flag_names(status_flags: u32) -> Vec<&'static str> {
    COMPACT_STATUS_V1_FLAG_SPECS
        .iter()
        .filter(|spec| (status_flags & spec.bit) != 0)
        .map(|spec| spec.name)
        .collect()
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
        domain_id: 42,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "air_combat_losses_live: only a producer-declared air-region combat-history gap is tolerated, and the declaration IS the payload spelling — the DATA record ends at its legacy length with NO region trailer exactly when the attribution walk refused unreadably after one re-walk and a second agreeing wing pass, with the refusal named by stage/predicate/address on the diagnostic channel; the fold admits nothing else. The wing rows carried are exact measurements whose delta baseline advances; the omitted attribution is absent, never zeros, so no combat is fabricated and no deletion is invented; every other failing word (SOURCE_UNAVAILABLE, inherited frame failures, rejected wing rows, scale mismatches — all serialized WITH their region section) keeps the pre-tolerance verdict: hour unknown, rows discarded",
    },
    CompactExpectedPartialTolerance {
        domain_id: 47,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "characters_leaders_cabinet_live: only a payload-declared per-family gap is tolerated, and the header's family-presence mask names every family the frame did not observe with the decoder refusing any other spelling — a bare PARTIAL_DATA without the declaration, a declaration without PARTIAL_DATA, rows from a declared-absent family, or a unit-leader trait count asserted while the trait family was unobserved. The rows carried are exact and whole families are the only unit of omission, so the sparse patch fabricates no deletion; row-level loss inside an observed family keeps SOURCE_UNAVAILABLE and stays outside this tolerance, and the domain/hour remains unknown for every family the mask declares absent",
    },
    CompactExpectedPartialTolerance {
        domain_id: 54,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "army_battleplans_depth_live: inherited d27/d28 partials or counted non-exact/linkage/identity/active/path omissions are diagnostic only; no omitted-order identity list exists, so the domain/hour remains unknown",
    },
    CompactExpectedPartialTolerance {
        domain_id: 86,
        tolerated_flags: COMPACT_STATUS_PARTIAL_DATA,
        reason: "sunk_ship_history_live: only a producer-declared partial history is tolerated, and the SHP1 payload names every lost callback per cause with a cross-checked total; the rows carried are exact and the history is append-only keyed by event_sequence, so the sparse patch fabricates no deletion, and the domain/hour remains unknown because the omitted sinkings' identities are not serialized",
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

pub fn compact_delta_pin_may_advance(domain_id: CompactDomainId, status_flags: u32) -> bool {
    let failure_flags = compact_status_for_global_interpretation(domain_id, status_flags)
        & COMPACT_STATUS_FAILURE_MASK;
    if failure_flags == 0 {
        return true;
    }
    if expected_partial_tolerance_applies(domain_id, status_flags).is_some() {
        return true;
    }
    compact_transition_baseline_reset_is_applied(domain_id, status_flags)
}

pub fn compact_transition_baseline_reset_is_applied(
    domain_id: CompactDomainId,
    status_flags: u32,
) -> bool {
    if domain_id != COMPACT_DOMAIN_TRANSITION_BASELINE_RESET_TOLERATED {
        return false;
    }
    if (status_flags & COMPACT_STATUS_TRANSITION_BASELINE_RESET) == 0 {
        return false;
    }
    let failure_flags = compact_status_for_global_interpretation(domain_id, status_flags)
        & COMPACT_STATUS_FAILURE_MASK;
    failure_flags == COMPACT_STATUS_PARTIAL_DATA
}

pub const COMPACT_DOMAIN_TRANSITION_BASELINE_RESET_TOLERATED: CompactDomainId = 55;

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

pub const OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS: &[CompactDomainId] =
    &[57, 74, 78, 84, 98, 99, 100, 101];

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

pub fn status_in_production_profile(status: CompactDomainStatus) -> bool {
    matches!(
        status,
        CompactDomainStatus::Existing
            | CompactDomainStatus::ProductionComplete
            | CompactDomainStatus::ProductionCompleteBase
    )
}

pub const PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS: &[CompactDomainId] = &[71, 82, 87];

pub fn passive_production_admits(id: CompactDomainId) -> bool {
    !PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS.contains(&id)
}

pub fn is_optional_fail_closed_companion(id: CompactDomainId) -> bool {
    OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS.contains(&id)
}

pub const DESCHEDULED_CAPTURE_DECISION_DATE: &str = "2026-08-03";

pub const DESCHEDULED_CAPTURE_DECISION_REF: &str = "PROJECT.md";

pub const DESCHEDULED_CAPTURE_REASON: &str =
    "POV-local presentation: what one client's screen had already resolved. The replay is an \
     observer-mode model and derives these panels from authoritative captured domains at render \
     time, so capturing them adds nothing.";

pub const DESCHEDULED_CAPTURE_REASON_D85: &str =
    "Client UI selection state has no clean observation when no designer is open; the source \
     yields no observation, which the collector can only encode as SOURCE_UNAVAILABLE, so the \
     domain cannot satisfy a production-complete passive capture contract.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescheduledCaptureRuling {
    pub id: CompactDomainId,
    pub decided_on: &'static str,
    pub reason: &'static str,
}

pub const DESCHEDULED_CAPTURE_RULINGS: &[DescheduledCaptureRuling] = &[
    DescheduledCaptureRuling {
        id: 72,
        decided_on: "2026-08-03",
        reason: DESCHEDULED_CAPTURE_REASON,
    },
    DescheduledCaptureRuling {
        id: 75,
        decided_on: "2026-08-03",
        reason: DESCHEDULED_CAPTURE_REASON,
    },
    DescheduledCaptureRuling {
        id: 79,
        decided_on: "2026-08-03",
        reason: DESCHEDULED_CAPTURE_REASON,
    },
    DescheduledCaptureRuling {
        id: 85,
        decided_on: "2026-08-13",
        reason: DESCHEDULED_CAPTURE_REASON_D85,
    },
    DescheduledCaptureRuling {
        id: 88,
        decided_on: "2026-08-03",
        reason: DESCHEDULED_CAPTURE_REASON,
    },
    DescheduledCaptureRuling {
        id: 89,
        decided_on: "2026-08-03",
        reason: DESCHEDULED_CAPTURE_REASON,
    },
];

pub fn descheduled_capture_ruling(
    id: CompactDomainId,
) -> Option<&'static DescheduledCaptureRuling> {
    DESCHEDULED_CAPTURE_RULINGS
        .iter()
        .find(|ruling| ruling.id == id)
}

pub const DESCHEDULED_CAPTURE_DOMAIN_IDS: &[CompactDomainId] = &[72, 75, 79, 85, 88, 89];

pub fn is_descheduled_from_capture(id: CompactDomainId) -> bool {
    DESCHEDULED_CAPTURE_DOMAIN_IDS.contains(&id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DescheduledDomainHookPrerequisite {
    pub descheduled: CompactDomainId,
    pub held_by: CompactDomainId,
    pub sites: &'static [&'static str],
    pub reason: &'static str,
    pub citation: &'static str,
}

pub const DESCHEDULED_DOMAIN_HOOK_PREREQUISITES: &[DescheduledDomainHookPrerequisite] =
    &[DescheduledDomainHookPrerequisite {
        descheduled: 75,
        held_by: 64,
        sites: &[
            "gameplay.d75.common_fire",
            "gameplay.d75.option_command",
            "gameplay.d75.timeout_resolution",
        ],
        reason: "d75's XUI21 producer edge registers the A19 delayed-event dispatch observer d64 reads through; d64's installer refuses when that observer is absent, and the two are one install unit so the edge closes before d64 is drained",
        citation: "collector/src/attach_log.rs PRODUCTION_INSTALL_UNITS (the Xui21ProducerEdge + D64 unit) and its doc comment",
    }];

/// a domain leaving the shipped roster releases every hold it had.
pub fn is_in_passive_production_roster(id: CompactDomainId) -> bool {
    passive_production_roster()
        .into_iter()
        .any(|descriptor| descriptor.id == id)
}

pub fn descheduled_hook_site_prerequisite(
    logical_name: &str,
) -> Option<&'static DescheduledDomainHookPrerequisite> {
    DESCHEDULED_DOMAIN_HOOK_PREREQUISITES.iter().find(|hold| {
        hold.sites.contains(&logical_name) && is_in_passive_production_roster(hold.held_by)
    })
}

pub const OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES: &[&str] = &[];

pub fn production_hook_site_installs(logical_name: &str, producer_domain: CompactDomainId) -> bool {
    if is_descheduled_from_capture(producer_domain)
        && descheduled_hook_site_prerequisite(logical_name).is_none()
    {
        return false;
    }
    if is_optional_fail_closed_companion(producer_domain) {
        return OPTIONAL_COMPANION_SITES_DEFERRED_TO_ACTIVE_LANES.contains(&logical_name);
    }
    true
}

pub fn descheduled_capture_domains() -> Vec<CompactDomainDescriptor> {
    let mut domains: Vec<CompactDomainDescriptor> = ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| is_descheduled_from_capture(descriptor.id))
        .collect();
    domains.sort_by_key(|descriptor| descriptor.id);
    domains
}

pub fn descheduled_capture_domain_names() -> Vec<&'static str> {
    descheduled_capture_domains()
        .into_iter()
        .map(|descriptor| descriptor.wire_name)
        .collect()
}

pub fn mandatory_profile_domains_not_production_complete() -> Vec<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| {
            descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && !is_descheduled_from_capture(descriptor.id)
                && !status_in_production_profile(descriptor.status)
        })
        .collect()
}

pub fn production_profile_domains() -> Vec<CompactDomainDescriptor> {
    ASSIGNED_COMPACT_DOMAINS
        .iter()
        .copied()
        .filter(|descriptor| in_production_profile(*descriptor))
        .collect()
}

pub fn in_production_profile(descriptor: CompactDomainDescriptor) -> bool {
    descriptor.schedulable
        && !is_optional_fail_closed_companion(descriptor.id)
        && !is_descheduled_from_capture(descriptor.id)
        && status_in_production_profile(descriptor.status)
}

pub fn production_profile_scheduled_names() -> Vec<&'static str> {
    production_profile_domains()
        .into_iter()
        .map(|descriptor| descriptor.wire_name)
        .collect()
}

pub fn passive_production_roster() -> Vec<CompactDomainDescriptor> {
    production_profile_domains()
        .into_iter()
        .filter(|descriptor| passive_production_admits(descriptor.id))
        .collect()
}

// Domain 100 rides every capture by owner ruling 2026-08-21: snow/mud is
// recorded always at its tile-delta cost (~40 KB/h) so the map picture never
// needs a special capture setting.
// Domain 101 rides every capture for the same reason domain 100 does, and at a
// far smaller price: the victory-point census is DELTA-emitted, so after the
// first hour it costs only the provinces a scripted effect actually changed.
// The map picture must not need a special capture setting to show its victory
// points.
pub const PASSIVE_CAPTURE_ONLY_DOMAIN_IDS: &[CompactDomainId] = &[57, 100, 101];

pub fn passive_capture_roster() -> Vec<CompactDomainDescriptor> {
    let mut roster = passive_production_roster();
    for id in PASSIVE_CAPTURE_ONLY_DOMAIN_IDS {
        if roster.iter().any(|descriptor| descriptor.id == *id) {
            continue;
        }
        let descriptor = assigned_descriptor_by_id(*id)
            .expect("every passive-capture-only domain has an assigned descriptor");
        assert!(
            descriptor.schedulable && !is_descheduled_from_capture(*id),
            "a passive-capture-only domain must be schedulable and not descheduled"
        );
        roster.push(descriptor);
    }
    roster.sort_by_key(|descriptor| descriptor.id);
    roster
}

pub fn is_in_passive_capture_roster(id: CompactDomainId) -> bool {
    passive_capture_roster()
        .into_iter()
        .any(|descriptor| descriptor.id == id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactOwnedEmittedDependency {
    pub owner: CompactDomainId,
    pub dependency: CompactDomainId,
    pub reason: &'static str,
    pub collector_citation: &'static str,
}

pub const OWNED_EMITTED_DOMAIN_DEPENDENCIES: &[CompactOwnedEmittedDependency] = &[
    CompactOwnedEmittedDependency {
        owner: 45,
        dependency: 12,
        reason: "focus_research_decisions_depth_live derives its focus depth from the same-frame country focus bundle, so enabling d45 pushes the Focus country-domain spec into the frame's enabled set",
        collector_citation: "attach_log.rs:24801-24816 (Focus spec push), :27566-27589 + :27607 (per-spec perf/data emission), :27563 (emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 45,
        dependency: 13,
        reason: "focus_research_decisions_depth_live derives research-slot state from the same-frame country tech bundle",
        collector_citation: "attach_log.rs:24801-24816 (Tech spec push), :27566-27589 + :27615 (per-spec perf/data emission)",
    },
    CompactOwnedEmittedDependency {
        owner: 45,
        dependency: 16,
        reason: "focus_research_decisions_depth_live derives decision status from the same-frame country decisions bundle",
        collector_citation: "attach_log.rs:24801-24816 (Decisions spec push), :27566-27589 (per-spec perf/data emission)",
    },
    CompactOwnedEmittedDependency {
        owner: 47,
        dependency: 14,
        reason: "characters_leaders_cabinet_live resolves cabinet/advisor slots against the same-frame country politics bundle",
        collector_citation: "attach_log.rs:24817-24828 (Politics spec push), :27566-27589 + :27623 (per-spec perf/data emission)",
    },
    CompactOwnedEmittedDependency {
        owner: 82,
        dependency: 47,
        reason: "character_presentation_live runs its d47 character-root dependency in the same frame; no prior-hour identity cache is used",
        collector_citation: "attach_log.rs:24773-24775 (comment), :24790-24796 (enable), :29052-29076 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 82,
        dependency: 46,
        reason: "character_presentation_live needs the same-frame air variant archetype classification to resolve designer/variant presentation rows",
        collector_citation: "attach_log.rs:24855-24857 (enable), :28933-28965 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 88,
        dependency: 47,
        reason: "active_command_abilities_live attributes abilities to characters read by the same-frame d47 cabinet walk",
        collector_citation: "attach_log.rs:24790-24796 (enable), :29052-29076 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 88,
        dependency: 20,
        reason: "active_command_abilities_live binds abilities to the same-frame division roster",
        collector_citation: "attach_log.rs:24829-24830 (enable), :28511-28548 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 54,
        dependency: 27,
        reason: "army_battleplans_depth_live derives generation/order rows from the same-frame army_orders read",
        collector_citation: "attach_log.rs:24839-24842 (enable), :28598-28655 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 54,
        dependency: 28,
        reason: "army_battleplans_depth_live requires exact same-frame assignment and order-linkage status from oob_command",
        collector_citation: "attach_log.rs:24839-24844 (enable), :28657-28708 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 55,
        dependency: 21,
        reason: "land_combat_detail_events_live derives transition events from the same in-frame domain-21 combat snapshot and its logical generation tracker",
        collector_citation: "attach_log.rs:25027-25030 (comment + enable), :30292-30322 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 99,
        dependency: 21,
        reason: "division_combat_stats_live keys every row on the D21 participant key, and `generation` is D21's own epoch counter rather than engine state, so a captured combat round cannot be keyed without the same in-frame domain-21 snapshot",
        collector_citation: "attach_log.rs:31358-31362 (comment + enable), :34915-34943 (perf/data emission + emitted_domains)",
    },
    CompactOwnedEmittedDependency {
        owner: 96,
        dependency: 92,
        reason: "country_preferred_tactic_weight_live is a trailer of the d92 preferred-tactic read; the weight payload only exists when that read runs, and the d92 record is appended unconditionally alongside it",
        collector_citation: "attach_log.rs:24962-24969 (enable), :28373-28401 (d92 perf/data emission + emitted_domains), :28402-28430 (d96 trailer emission)",
    },
    CompactOwnedEmittedDependency {
        owner: 92,
        dependency: 18,
        reason: "country_preferred_tactic_live resolves its per-country rows against the same-frame country_identity roster",
        collector_citation: "attach_log.rs:24970-24973 (enable), :27722-27747 (perf/data emission + emitted_domains)",
    },
];

pub const DEFAULT_SCHEDULED_DOMAIN_IDS: &[CompactDomainId] = &[10, 11, 12, 13, 14, 15, 16, 17];

pub fn owned_emitted_dependency_participant_ids() -> Vec<CompactDomainId> {
    let mut ids = Vec::new();
    for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
        for id in [edge.owner, edge.dependency] {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids.sort_unstable();
    ids
}

pub fn owned_emitted_dependencies_for_owner(
    owner: CompactDomainId,
) -> Vec<&'static CompactOwnedEmittedDependency> {
    OWNED_EMITTED_DOMAIN_DEPENDENCIES
        .iter()
        .filter(|edge| edge.owner == owner)
        .collect()
}

pub fn emitted_domain_ids_for_scheduled_ids(
    scheduled_ids: &[CompactDomainId],
) -> Vec<CompactDomainId> {
    let mut emitted = scheduled_ids.to_vec();
    loop {
        let before = emitted.len();
        for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
            if emitted.contains(&edge.owner) && !emitted.contains(&edge.dependency) {
                emitted.push(edge.dependency);
            }
        }
        if emitted.len() == before {
            break;
        }
    }
    emitted.sort_unstable();
    emitted.dedup();
    emitted
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureProfile {
    Production,
    Debug,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptureProfilePreset {
    pub stream_format_compact: bool,
    pub use_production_domains: bool,
    pub compact_country_frame: bool,
    pub reader_concurrency: u32,
    pub extraction_perf_gate: bool,
    pub sidecar_log: bool,
    pub lightweight_root_frame: bool,
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
        false,
        &[],
    ),
    domain(
        59,
        "templates_equipment_catalog_live",
        "templates_equipment_catalog_live",
        CompactDomainStatus::ValidatedSlice,
        false,
        false,
        &[],
    ),
    domain(
        60,
        "capitulation_war_state_live",
        "capitulation_war_state_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        61,
        "air_loss_stats_live",
        "air_loss_stats_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        62,
        "hub_motorization_live",
        "hub_motorization_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        63,
        "country_core_scalars_live",
        "country_core_scalars_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        64,
        "delayed_events_live",
        "delayed_events_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        65,
        "special_project_programs_live",
        "special_project_programs_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        66,
        "occupation_state_extras_live",
        "occupation_state_extras_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        67,
        "country_dynamic_modifiers_live",
        "country_dynamic_modifiers_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        68,
        "session_header_live",
        "session_header_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        69,
        "tech_sharing_groups_live",
        "tech_sharing_groups_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        70,
        "sunk_convoy_history_live",
        "sunk_convoy_history_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        71,
        "dynamic_collection_trace_live",
        "dynamic_collection_trace_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        72,
        "resolved_tooltip_instances_live",
        "resolved_tooltip_instances_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        74,
        "power_balance_state_live",
        "power_balance_state_live",
        CompactDomainStatus::ValidatedSlice,
        true,
        true,
        &[],
    ),
    domain(
        75,
        "fired_event_popups_live",
        "fired_event_popups_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        77,
        "production_render_submits_live",
        "production_render_submits_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        78,
        "design_catalog_context_live",
        "design_catalog_context_live",
        CompactDomainStatus::ValidatedSlice,
        true,
        true,
        &[],
    ),
    domain(
        79,
        "hud_surface_activation_live",
        "hud_surface_activation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        80,
        "production_map_content_snapshot_live",
        "production_map_content_snapshot_live",
        CompactDomainStatus::Reserved,
        false,
        true,
        &[],
    ),
    domain(
        81,
        "rail_replay_state_live",
        "rail_replay_state_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        82,
        "character_presentation_live",
        "character_presentation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        83,
        "nuke_stockpile_live",
        "nuke_stockpile_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        84,
        "scripted_gui_runtime_live",
        "scripted_gui_runtime_live",
        CompactDomainStatus::ValidatedSlice,
        true,
        false,
        &[],
    ),
    domain(
        85,
        "selected_designer_identity_live",
        "selected_designer_identity_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        86,
        "sunk_ship_history_live",
        "sunk_ship_history_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        87,
        "decision_panel_live",
        "decision_panel_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        88,
        "active_command_abilities_live",
        "active_command_abilities_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        89,
        "national_focus_presentation_live",
        "national_focus_presentation_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        90,
        "convoy_transport_attack_live",
        "convoy_transport_attack_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        91,
        "raid_instances_live",
        "raid_instances_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        92,
        "country_preferred_tactic_live",
        "country_preferred_tactic_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        93,
        "country_army_upgrade_requests_live",
        "country_army_upgrade_requests_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        94,
        "pov_province_visibility_live",
        "pov_province_visibility_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        95,
        "pov_spotted_units_live",
        "pov_spotted_units_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
    domain(
        96,
        "country_preferred_tactic_weight_live",
        "country_preferred_tactic_weight_live",
        CompactDomainStatus::ProductionComplete,
        true,
        true,
        &[],
    ),
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
        98,
        "theatre_selector_live",
        "theatre_selector_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        99,
        "division_combat_stats_live",
        "division_combat_stats_live",
        CompactDomainStatus::ValidatedSliceOpenGaps,
        true,
        true,
        &[],
    ),
    domain(
        100,
        "xmap07_snow_mud_live",
        "xmap07_snow_mud_live",
        CompactDomainStatus::ActiveIntegration,
        true,
        true,
        &[],
    ),
    domain(
        101,
        "map_furniture_victory_points_live",
        "map_furniture_victory_points_live",
        CompactDomainStatus::ActiveIntegration,
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
    fn owned_emitted_dependency_table_is_well_formed() {
        for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
            assert_ne!(
                edge.owner, edge.dependency,
                "domain {} cannot be its own owned dependency",
                edge.owner
            );
            let owner = assigned_descriptor_by_id(edge.owner)
                .unwrap_or_else(|| panic!("owner {} is not an assigned domain", edge.owner));
            let dependency = assigned_descriptor_by_id(edge.dependency).unwrap_or_else(|| {
                panic!("dependency {} is not an assigned domain", edge.dependency)
            });
            assert!(
                dependency.requires_data_record,
                "dependency {} ({}) does not emit a DOMAIN_DATA record and therefore is not an owned EMITTED dependency",
                dependency.id, dependency.wire_name
            );
            assert!(
                owner.schedulable,
                "owner {} ({}) is not schedulable, so it can never force a dependency",
                owner.id, owner.wire_name
            );
            assert!(
                !edge.reason.is_empty() && !edge.collector_citation.is_empty(),
                "edge {} -> {} must carry a reason and a collector citation",
                edge.owner,
                edge.dependency
            );
        }
        let mut seen = Vec::new();
        for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
            let key = (edge.owner, edge.dependency);
            assert!(
                !seen.contains(&key),
                "duplicate owned-dependency edge {} -> {}",
                edge.owner,
                edge.dependency
            );
            seen.push(key);
        }
    }

    #[test]
    fn d45_expands_to_its_three_country_bundle_dependencies() {
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[45]),
            vec![12, 13, 16, 45]
        );
    }

    #[test]
    fn owned_dependency_expansion_matches_the_verified_collector_edges() {
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[47]),
            vec![14, 47],
            "d47 pulls the politics country bundle"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[54]),
            vec![27, 28, 54],
            "d54 pulls army_orders and oob_command"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[55]),
            vec![21, 55],
            "d55 forces the domain-21 snapshot it derives transitions from"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[82]),
            vec![14, 46, 47, 82],
            "d82 pulls d47 (which pulls d14) and d46"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[88]),
            vec![14, 20, 47, 88],
            "d88 pulls d20 and d47 (which pulls d14)"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[96]),
            vec![18, 92, 96],
            "d96 pulls d92, which pulls d18"
        );
        assert_eq!(
            emitted_domain_ids_for_scheduled_ids(&[92]),
            vec![18, 92],
            "d92 alone still pulls d18"
        );
    }

    #[test]
    fn owned_dependency_expansion_is_idempotent_and_canonical() {
        let once = emitted_domain_ids_for_scheduled_ids(&[88, 45, 88]);
        let twice = emitted_domain_ids_for_scheduled_ids(&once);
        assert_eq!(once, twice);
        assert!(once.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn reader_only_couplings_are_not_encoded_as_emissions() {
        for owner in [86u16, 90u16] {
            assert_eq!(
                emitted_domain_ids_for_scheduled_ids(&[owner]),
                vec![owner],
                "domain {owner} runs the d49/d50 reader but emits neither"
            );
        }
        for dependency in [49u16, 50u16] {
            assert!(
                OWNED_EMITTED_DOMAIN_DEPENDENCIES
                    .iter()
                    .all(|edge| edge.dependency != dependency),
                "domain {dependency} is a reader dependency, never an owned emission"
            );
        }
    }

    #[test]
    fn owned_dependency_participants_cover_the_table() {
        let participants = owned_emitted_dependency_participant_ids();
        for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
            assert!(participants.contains(&edge.owner));
            assert!(participants.contains(&edge.dependency));
        }
        assert!(participants.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            owned_emitted_dependencies_for_owner(45)
                .iter()
                .map(|edge| edge.dependency)
                .collect::<Vec<_>>(),
            vec![12, 13, 16]
        );
        assert!(owned_emitted_dependencies_for_owner(34).is_empty());
    }

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
            assert!(preset.stream_format_compact);
            assert!(preset.use_production_domains);
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
        let expected: &[CompactDomainId] = &[
            1, 10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32,
            33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54,
            55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 74, 75, 77, 78,
            79, 80, 81, 82, 83, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,
            100, 101, 1000,
        ];
        for id in expected {
            assert!(
                assigned_descriptor_by_id(*id).is_some(),
                "domain id {id} missing from ASSIGNED_COMPACT_DOMAINS"
            );
        }
        assert_eq!(ASSIGNED_COMPACT_DOMAINS.len(), expected.len());
    }

    const RULED_OUT_OF_CAPTURE_SCOPE: &[(CompactDomainId, &str, &str)] = &[
        (72, "resolved_tooltip_instances_live", "2026-08-03"),
        (75, "fired_event_popups_live", "2026-08-03"),
        (79, "hud_surface_activation_live", "2026-08-03"),
        (85, "selected_designer_identity_live", "2026-08-13"),
        (88, "active_command_abilities_live", "2026-08-03"),
        (89, "national_focus_presentation_live", "2026-08-03"),
    ];

    #[test]
    fn descheduled_domains_are_absent_from_every_production_roster() {
        let profile_ids: Vec<CompactDomainId> = production_profile_domains()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();
        let scheduled_names = production_profile_scheduled_names();

        for (id, wire_name, _decided_on) in RULED_OUT_OF_CAPTURE_SCOPE {
            assert!(
                !profile_ids.contains(id),
                "d{id} {wire_name} was ruled out of capture scope but is still in the \
                 production profile; every roster path derives from it"
            );
            assert!(
                !scheduled_names.contains(wire_name),
                "d{id} {wire_name} was ruled out of capture scope but is still scheduled"
            );
        }
    }

    #[test]
    fn descheduling_keeps_the_producers_and_the_reason() {
        for (id, wire_name, _decided_on) in RULED_OUT_OF_CAPTURE_SCOPE {
            let descriptor = assigned_descriptor_by_id(*id)
                .unwrap_or_else(|| panic!("d{id} must keep its descriptor"));
            assert_eq!(descriptor.wire_name, *wire_name);
            assert!(
                status_in_production_profile(descriptor.status),
                "d{id} {wire_name} should keep its production status; descheduling is not a \
                 status downgrade"
            );
            assert!(
                descriptor.schedulable,
                "d{id} {wire_name} must stay individually schedulable for evidence work"
            );
            assert_eq!(
                id_for_scheduled_name(wire_name),
                Some(*id),
                "d{id} {wire_name} must still resolve when explicitly scheduled"
            );
        }

        let published = descheduled_capture_domain_names();
        for (id, wire_name, decided_on) in RULED_OUT_OF_CAPTURE_SCOPE {
            assert!(
                published.contains(wire_name),
                "d{id} {wire_name} is unscheduled but nothing publishes it as a decision"
            );
            let ruling = descheduled_capture_ruling(*id)
                .unwrap_or_else(|| panic!("d{id} {wire_name} publishes no ruling"));
            assert_eq!(
                ruling.decided_on, *decided_on,
                "d{id} {wire_name} must publish the date of the ruling that actually covered it"
            );
            assert!(
                ruling.reason.len() > 40,
                "d{id} {wire_name}'s published reason must actually explain the decision"
            );
        }
        assert_eq!(published.len(), RULED_OUT_OF_CAPTURE_SCOPE.len());
        assert_eq!(
            DESCHEDULED_CAPTURE_RULINGS.len(),
            RULED_OUT_OF_CAPTURE_SCOPE.len()
        );
        assert_ne!(
            descheduled_capture_ruling(85).expect("d85 ruling").reason,
            descheduled_capture_ruling(72).expect("d72 ruling").reason,
            "d85 was descheduled for its own reason and must not inherit the 2026-08-03 one"
        );
        assert!(
            DESCHEDULED_CAPTURE_REASON.len() > 40,
            "the published reason must actually explain the decision"
        );
        assert_eq!(
            DESCHEDULED_CAPTURE_DECISION_DATE, "2026-08-03",
            "this constant is the MECHANISM's date, not every ruling's"
        );
        assert_eq!(DESCHEDULED_CAPTURE_DECISION_REF, "PROJECT.md");
    }

    #[test]
    fn descheduling_an_owner_does_not_silently_drop_its_dependencies() {
        let scheduled_ids: std::collections::BTreeSet<CompactDomainId> =
            production_profile_domains()
                .into_iter()
                .map(|descriptor| descriptor.id)
                .collect();

        for edge in OWNED_EMITTED_DOMAIN_DEPENDENCIES {
            if !is_descheduled_from_capture(edge.owner) {
                continue;
            }
            assert!(
                scheduled_ids.contains(&edge.dependency),
                "d{} is descheduled but d{} reached the wire only through it, and d{} is not \
                 scheduled on its own",
                edge.owner,
                edge.dependency,
                edge.dependency
            );
        }
    }

    #[test]
    fn descheduling_does_not_reach_the_domains_it_excluded() {
        for (id, wire_name) in [
            (95u16, "pov_spotted_units_live"),
            (97, "peace_conference_live"),
        ] {
            assert!(
                !is_descheduled_from_capture(id),
                "d{id} {wire_name} is outside the deschedule ruling"
            );
            assert!(
                production_profile_scheduled_names().contains(&wire_name),
                "d{id} {wire_name} must stay scheduled"
            );
        }

        for id in DESCHEDULED_CAPTURE_DOMAIN_IDS {
            assert!(
                !PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS.contains(id),
                "d{id} cannot be both an owner deschedule and a transport refusal"
            );
        }
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
            assert_eq!(
                domain
                    .get("capture_scheduled")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true),
                !is_descheduled_from_capture(descriptor.id),
                "json capture_scheduled drift for domain {}",
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
    fn passive_capture_roster_schedules_the_domain_that_names_the_players() {
        let capture: Vec<CompactDomainId> = passive_capture_roster()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();
        let mandatory: Vec<CompactDomainId> = passive_production_roster()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect();

        assert!(capture.contains(&57), "d57 must be scheduled by a capture");
        assert!(
            !mandatory.contains(&57),
            "d57 must stay out of the mandatory production profile"
        );
        assert!(is_in_passive_capture_roster(57));
        for id in &mandatory {
            assert!(capture.contains(id), "capture roster dropped domain {id}");
        }
        assert_eq!(
            capture.len(),
            mandatory.len() + PASSIVE_CAPTURE_ONLY_DOMAIN_IDS.len()
        );
        let mut sorted = capture.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted, capture, "capture roster must be sorted and unique");
    }

    #[test]
    fn production_profile_contains_only_complete_mandatory_domains() {
        let profile = production_profile_domains();
        let profile_ids: std::collections::BTreeSet<CompactDomainId> =
            profile.iter().map(|descriptor| descriptor.id).collect();
        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            let expected = descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && !is_descheduled_from_capture(descriptor.id)
                && status_in_production_profile(descriptor.status);
            assert_eq!(
                profile_ids.contains(&descriptor.id),
                expected,
                "domain {} mandatory profile membership should require a complete status and \
                 a domain the owner has not ruled out of capture scope",
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
        for excluded in [1, 58, 59, 77, 80, 1000] {
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
    fn the_passive_roster_is_the_production_profile_minus_the_passive_refusals() {
        let roster = passive_production_roster();
        let roster_ids: std::collections::BTreeSet<CompactDomainId> =
            roster.iter().map(|descriptor| descriptor.id).collect();

        assert_eq!(
            roster,
            production_profile_domains()
                .into_iter()
                .filter(|descriptor| passive_production_admits(descriptor.id))
                .collect::<Vec<_>>(),
            "passive_production_roster must be production_profile_domains() filtered by \
             passive_production_admits, and nothing else"
        );

        for descriptor in ASSIGNED_COMPACT_DOMAINS {
            let expected = descriptor.schedulable
                && !is_optional_fail_closed_companion(descriptor.id)
                && !is_descheduled_from_capture(descriptor.id)
                && status_in_production_profile(descriptor.status)
                && passive_production_admits(descriptor.id);
            assert_eq!(
                roster_ids.contains(&descriptor.id),
                expected,
                "domain {} passive roster membership disagrees with the composed clauses",
                descriptor.id
            );
        }

        for id in OPTIONAL_FAIL_CLOSED_COMPANION_DOMAIN_IDS {
            assert!(
                !roster_ids.contains(id),
                "optional fail-closed companion domain {id} must stay out of the passive roster"
            );
            let promoted = ASSIGNED_COMPACT_DOMAINS
                .iter()
                .find(|descriptor| descriptor.id == *id)
                .map(|descriptor| CompactDomainDescriptor {
                    status: CompactDomainStatus::ProductionComplete,
                    ..*descriptor
                })
                .expect("every companion id has a descriptor");
            assert!(
                status_in_production_profile(promoted.status),
                "the promoted fixture must actually clear the status clause, or this arm proves \
                 nothing about the companion clause"
            );
            assert!(
                !in_production_profile(promoted),
                "domain {id} carrying a production status must still be held out by the companion \
                 clause; otherwise a one-word status edit widens production"
            );
        }

        let free_id = ASSIGNED_COMPACT_DOMAINS
            .iter()
            .find(|descriptor| {
                !is_optional_fail_closed_companion(descriptor.id)
                    && !is_descheduled_from_capture(descriptor.id)
            })
            .expect("some domain is neither a companion nor descheduled")
            .id;
        let synthetic = |schedulable, status| CompactDomainDescriptor {
            id: free_id,
            canonical_name: "synthetic_clause_probe",
            wire_name: "synthetic_clause_probe",
            status,
            schedulable,
            requires_data_record: true,
            aliases: &[],
        };
        assert!(
            in_production_profile(synthetic(true, CompactDomainStatus::ProductionComplete)),
            "the control must be admitted, or the two refusals below prove nothing"
        );
        assert!(
            !in_production_profile(synthetic(false, CompactDomainStatus::ProductionComplete)),
            "an unschedulable domain must never enter the production profile, whatever its status"
        );
        assert!(
            !in_production_profile(synthetic(true, CompactDomainStatus::ValidatedSliceOpenGaps)),
            "an open-gap domain must never enter the production profile, however schedulable \
             (PRODUCTION_PROFILE_RULE)"
        );

        assert!(
            roster.len() <= production_profile_domains().len(),
            "the passive roster can only ever be a subset of the production profile"
        );
        for id in PASSIVE_PRODUCTION_REFUSED_DOMAIN_IDS {
            assert!(
                !roster_ids.contains(id),
                "the passive collector refuses domain {id} at attach, so no roster may name it"
            );
        }
    }

    #[test]
    fn d98_is_schedulable_by_name_and_out_of_production_by_profile_policy() {
        let descriptor =
            assigned_descriptor_by_id(98).expect("d98 has a compact_contract descriptor");
        assert_eq!(descriptor.wire_name, "theatre_selector_live");
        assert!(descriptor.schedulable, "d98 is individually schedulable");
        assert!(descriptor.requires_data_record);

        assert_eq!(id_for_scheduled_name("theatre_selector_live"), Some(98));
        assert!(
            emitted_domain_ids_for_scheduled_ids(&[98]).contains(&98),
            "scheduling d98 must put d98 on the wire"
        );
        assert!(
            passive_production_admits(98),
            "d98 is not in the roster-side passive refusal set, so the derived roster is not \
             what excludes it"
        );
        assert!(
            !is_descheduled_from_capture(98),
            "d98 was not ruled out of capture scope by the owner"
        );

        assert!(
            !status_in_production_profile(descriptor.status),
            "the status clause holds d98 out"
        );
        assert!(
            is_optional_fail_closed_companion(98),
            "the companion clause holds d98 out too, so a status promotion alone does not admit it"
        );
        assert!(
            !in_production_profile(descriptor),
            "d98 is out of the mandatory production profile"
        );
        assert!(
            !passive_production_roster()
                .iter()
                .any(|candidate| candidate.id == 98),
            "and therefore out of the roster a production capture schedules"
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
    fn d53_production_status_word_classifies_instead_of_rejecting() {
        let observed = COMPACT_STATUS_PARTIAL_DATA
            | DOMAIN53_STATUS_OPERATIONS_DETAIL_RESISTED
            | DOMAIN53_STATUS_INTEL_MAP_RESISTED
            | DOMAIN53_STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED
            | DOMAIN53_STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
        assert_eq!(observed, 0x0053_0200, "the measured batch-3a word");
        let global = compact_status_for_global_interpretation(53, observed);
        assert_eq!(global, COMPACT_STATUS_PARTIAL_DATA);
        assert_eq!(
            compact_status_unknown_flags_v1(global),
            0,
            "a registered lane badge is not an unassigned bit"
        );
        assert_eq!(
            compact_status_failure_flags_after_expected_partial_tolerance(53, observed),
            COMPACT_STATUS_PARTIAL_DATA
        );
        for bit in 16..=22u32 {
            assert_eq!(
                compact_status_for_global_interpretation(
                    53,
                    COMPACT_STATUS_PARTIAL_DATA | (1 << bit)
                ),
                COMPACT_STATUS_PARTIAL_DATA,
                "d53 badge bit {bit} must strip"
            );
        }
        assert_eq!(
            compact_status_unknown_flags_v1(compact_status_for_global_interpretation(52, observed)),
            0x0053_0000
        );
    }

    #[test]
    fn d60_war_badges_classify_instead_of_rejecting() {
        let observed = DOMAIN60_STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR
            | DOMAIN60_STATUS_WAR_RESIDUALS_EXACT;
        let global = compact_status_for_global_interpretation(60, observed);
        assert_eq!(global, 0);
        assert_eq!(compact_status_unknown_flags_v1(global), 0);
        assert!(!compact_status_is_failure(global));
        assert_eq!(
            compact_status_unknown_flags_v1(compact_status_for_global_interpretation(61, observed)),
            observed,
            "the d60 badges stay unassigned for every other domain"
        );
    }

    #[test]
    fn lane_local_status_registry_is_exactly_the_declared_domains() {
        for domain_id in 0..=u16::MAX {
            let expected: u32 = match domain_id {
                49 => 1 << 11,
                51 => (1 << 11) | (1 << 12) | (1 << 13),
                53 => 0x007F_0000,
                60 => 0x0003_0000,
                99 => 0x0007_0000,
                _ => 0,
            };
            assert_eq!(
                lane_local_status_mask(domain_id),
                expected,
                "domain {domain_id}"
            );
        }
        assert_eq!(DOMAIN53_STATUS_OPERATIONS_DETAIL_RESISTED, 1 << 16);
        assert_eq!(DOMAIN53_STATUS_INTEL_MAP_RESISTED, 1 << 17);
        assert_eq!(DOMAIN53_STATUS_WAR_ROOT_UNVALIDATED, 1 << 18);
        assert_eq!(DOMAIN53_STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED, 1 << 19);
        assert_eq!(DOMAIN53_STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED, 1 << 20);
        assert_eq!(DOMAIN53_STATUS_FACTION_GROUP_UNVALIDATED, 1 << 21);
        assert_eq!(DOMAIN53_STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED, 1 << 22);
        assert_eq!(DOMAIN60_STATUS_WAR_CASUALTIES_RUNNING_ACCUMULATOR, 1 << 16);
        assert_eq!(DOMAIN60_STATUS_WAR_RESIDUALS_EXACT, 1 << 17);
        assert_eq!(
            crate::division_combat_stats::DOMAIN99_STATUS_TORN_ROWS_PRESENT,
            1 << 16
        );
        assert_eq!(
            crate::division_combat_stats::DOMAIN99_STATUS_STAT_BLOCK_NULL_PRESENT,
            1 << 17
        );
        assert_eq!(
            crate::division_combat_stats::DOMAIN99_STATUS_INCOMPLETE_OBSERVATION,
            1 << 18
        );
        assert_eq!(
            DOMAIN49_LANE_LOCAL_STATUS_MASK & !COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
        assert_eq!(
            DOMAIN51_LANE_LOCAL_STATUS_MASK & !COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
        assert_eq!(
            DOMAIN53_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
        assert_eq!(
            DOMAIN60_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
        assert_eq!(
            crate::division_combat_stats::DOMAIN99_LANE_LOCAL_STATUS_MASK
                & COMPACT_STATUS_V1_KNOWN_MASK,
            0
        );
        assert_eq!(
            DOMAIN53_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_FAILURE_MASK,
            0
        );
        assert_eq!(
            DOMAIN60_LANE_LOCAL_STATUS_MASK & COMPACT_STATUS_FAILURE_MASK,
            0
        );
        assert_eq!(
            crate::division_combat_stats::DOMAIN99_LANE_LOCAL_STATUS_MASK
                & COMPACT_STATUS_FAILURE_MASK,
            0
        );
    }

    #[test]
    fn expected_partial_tolerances_are_exactly_documented_domains() {
        let domains = COMPACT_EXPECTED_PARTIAL_TOLERANCES
            .iter()
            .map(|tolerance| tolerance.domain_id)
            .collect::<Vec<_>>();
        assert_eq!(domains, vec![26, 27, 28, 42, 47, 54, 86]);
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
        for domain_id in [29, 41, 45, 48, 49, 50, 53, 56, 65] {
            assert_eq!(expected_partial_tolerance(domain_id), None);
        }
        for domain_id in [26, 27, 28, 42, 47, 54, 86] {
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
            COMPACT_STATUS_SOURCE_EMPTY,
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

    #[test]
    fn v1_flag_specs_cover_exactly_the_known_mask() {
        let mut covered = 0u32;
        for spec in COMPACT_STATUS_V1_FLAG_SPECS {
            assert_eq!(
                spec.bit.count_ones(),
                1,
                "spec {} must name a single bit, found {:#x}",
                spec.name,
                spec.bit
            );
            assert_eq!(
                covered & spec.bit,
                0,
                "bit {:#x} is named twice (second name {})",
                spec.bit,
                spec.name
            );
            covered |= spec.bit;
        }
        assert_eq!(covered, COMPACT_STATUS_V1_KNOWN_MASK);
        assert_eq!(
            COMPACT_STATUS_V1_FLAG_SPECS.len(),
            COMPACT_STATUS_V1_KNOWN_MASK.count_ones() as usize
        );

        let mut names: Vec<&'static str> = COMPACT_STATUS_V1_FLAG_SPECS
            .iter()
            .map(|spec| spec.name)
            .collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "flag names must be distinct");
    }

    #[test]
    fn spec_failure_class_is_read_from_the_mask_not_a_hand_written_list() {
        for spec in COMPACT_STATUS_V1_FLAG_SPECS {
            assert_eq!(
                spec.is_failure(),
                compact_status_is_failure(spec.bit),
                "spec {} disagrees with the failure mask",
                spec.name
            );
        }
        let war_segment_absent = COMPACT_STATUS_V1_FLAG_SPECS
            .iter()
            .find(|spec| spec.bit == COMPACT_STATUS_WAR_SEGMENT_ABSENT)
            .expect("war_segment_absent is an assigned v1 bit");
        assert_eq!(war_segment_absent.name, "war_segment_absent");
        assert!(!war_segment_absent.is_failure());
    }

    #[test]
    fn flag_names_report_set_bits_in_ascending_order_and_ignore_unassigned() {
        assert_eq!(compact_status_flag_names(0), Vec::<&str>::new());
        assert_eq!(
            compact_status_flag_names(
                COMPACT_STATUS_WORKER_PANIC | COMPACT_STATUS_NO_CLOCK | COMPACT_STATUS_PARTIAL_DATA,
            ),
            vec!["no_clock", "worker_panic", "partial_data"]
        );
        assert_eq!(
            compact_status_flag_names(COMPACT_STATUS_WAR_SEGMENT_ABSENT),
            vec!["war_segment_absent"]
        );
        assert_eq!(compact_status_flag_names(1 << 31), Vec::<&str>::new());
        assert_eq!(
            compact_status_flag_names(COMPACT_STATUS_V1_KNOWN_MASK).len(),
            COMPACT_STATUS_V1_FLAG_SPECS.len()
        );
    }

    #[test]
    fn unassigned_flags_are_undeclared_bits_only() {
        for spec in COMPACT_STATUS_V1_FLAG_SPECS {
            for domain_id in [10u16, 17, 49, 51, 53, 55, 60, 99] {
                assert_eq!(
                    compact_status_unassigned_flags(domain_id, spec.bit),
                    0,
                    "domain {domain_id}: assigned bit {} is defined",
                    spec.name
                );
            }
        }

        for (domain_id, mask) in [
            (49u16, DOMAIN49_LANE_LOCAL_STATUS_MASK),
            (51, DOMAIN51_LANE_LOCAL_STATUS_MASK),
            (53, DOMAIN53_LANE_LOCAL_STATUS_MASK),
            (60, DOMAIN60_LANE_LOCAL_STATUS_MASK),
            (
                99,
                crate::division_combat_stats::DOMAIN99_LANE_LOCAL_STATUS_MASK,
            ),
        ] {
            assert_eq!(compact_status_unassigned_flags(domain_id, mask), 0);
            let outside_v1 = mask & !COMPACT_STATUS_V1_KNOWN_MASK;
            if outside_v1 != 0 {
                assert_eq!(
                    compact_status_unassigned_flags(10, outside_v1),
                    outside_v1,
                    "d10 has declared no lane-local badge, so {outside_v1:#x} is undefined for it"
                );
            }
        }

        assert_eq!(lane_local_status_mask(10), 0);
        assert_eq!(
            compact_status_unassigned_flags(10, COMPACT_STATUS_BASELINE_CLEAR),
            0
        );
        for bit in 16..u32::BITS {
            let flag = 1_u32 << bit;
            assert_eq!(compact_status_unassigned_flags(10, flag), flag);
        }

        assert_eq!(
            compact_status_unassigned_flags(17, COMPACT_STATUS_PARTIAL_DATA | (1 << 20)),
            1 << 20
        );
    }

    #[test]
    fn unassigned_bit_indices_render_every_offending_bit_ascending() {
        assert_eq!(compact_status_unassigned_bit_indices(0), Vec::<u32>::new());
        assert_eq!(
            compact_status_unassigned_bit_indices((1 << 15) | (1 << 20) | (1 << 31)),
            vec![15, 20, 31]
        );
        assert_eq!(compact_status_unassigned_bit_indices(u32::MAX).len(), 32);
    }
}
