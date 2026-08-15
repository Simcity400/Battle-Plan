//! Canonical DATA-payload decoding authority shared by recorder and validators.
//!
//! The implementation in this module is mechanically moved from the recorder
//! binary so every production consumer uses one set of wire decoders.

use air_loss_stats_live_lane as airloss61;
use active_command_abilities_live_lane::production_carrier as abilities88;
use army_battleplans_depth_live_lane as battleplans54;
use capitulation_war_state_live as capwar60;
use character_presentation_live_lane as character82;
use characters_leaders_cabinet_live_lane as domain47;
use construction_buildings_state_damage_live_lane as domain48;
use country_economy_production_stockpile_live_lane as production11;
use design_catalog_context_live_lane as design78;
use design_catalog_context_live_lane::peace_conference_live as peace97;
use theatre_selector_live_lane as theatre98;
use design_catalog_context_live_lane::selected_designer_identity as selected85;
use decision_panel_live_lane as decision87;
use diplomacy_intel_operatives_live as diplomacy53;
use dynamic_collection_trace_live_lane as dynamic71;
use fired_event_popups_live_lane as fired75;
use hub_motorization_live_lane as hub62;
use hud_role_observer_live_lane as hud79;
use land_combat_detail_events_live_lane as land_detail55;
use compact_contract::national_focus_presentation as focus89;
use compact_contract::preferred_tactic as preferred92;
use compact_contract::pov_visibility as visibility94;
use compact_contract::spotted_units as spotted95;
use compact_contract::tactic_weight as weight96;
use country_army_upgrade_requests_live_lane as army_upgrades93;
use naval_combat_live_lane as naval4950;
use naval_missions_status_live_lane as naval56;
use nuke_stockpile_live_lane as nuke83;
use occupation_resistance_compliance_live_lane as occupation52;
use resolved_tooltip_instances_live_lane as tooltip72;
use session_multiplayer_metadata_live as session57;
use scripted_state_flags_variables_live_lane::scripted_gui_runtime_compact as scripted84;
use sunk_ship_history_live_lane as sunk86;
use supply_logistics_attrition_live_lane as supply44;

use crate::campaign_identity::SessionHeaderIdentityObservation;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const A29_PRODUCER_AUTHORITY: &str =
    "a729_observed_research_ahead_selected_present_v3";
pub(crate) const A29_EXECUTABLE_SHA256: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";
pub(crate) const A29_EXACT_ROOT_JOB_ID: &str =
    "20260720T124311Z-w5-a29-effective-producer-roots-e50d79d0";
pub(crate) const A29_EXACT_ROOT_MANIFEST_SHA256: &str =
    "82884a9a122f95298d5068bdb68590806346c2c1ac274ca3b56889bb0e5abcc2";
pub(crate) const A29_SENTINEL_JOB_ID: &str =
    "20260720T130641Z-w5-a29-hook-sentinels-and-substitution-i-9016d012";
pub(crate) const A29_SENTINEL_DUMP_SHA256: &str =
    "3fbca6726f09499e6cf1961969ae4154b9c265f8e5c688f8d939d8397387cee2";
pub(crate) const A29_UI_YEAR_PRODUCER_RVA: u64 = 0x01BC_EA60;
pub(crate) const A29_UI_YEAR_HELPER_RVA: u64 = 0x00AB_EAA0;
pub(crate) const A29_UI_YEAR_ACTION_IDENTITY_COMPARE_RVA: u64 = 0x01BC_F326;
pub(crate) const A29_UI_YEAR_HELPER_RETURN_RVA: u64 = 0x01BC_F384;
pub(crate) const A29_EFFECTIVE_VALUE_PRODUCER_RVA: u64 = 0x00AB_E4C0;
pub(crate) const A29_LOCALIZATION_CONSUMER_RVA: u64 = 0x0223_0F20;
pub(crate) const A29_ACTION_GATE_JOB_ID: &str =
    "20260721T075619Z-wave6-a29-active-ahead-ui-producer-ced43f28";
pub(crate) const A29_ACTION_GATE_MANIFEST_SHA256: &str =
    "8d802c9eb2fe5e78ab52d7512d8d01dd5793b9c29fac8627c17fb6cafcc621ba";
pub(crate) const A29_ACTION_GATE_SENTINEL_JOB_ID: &str =
    "20260721T081803Z-wave6-a29-year-action-gate-sentinels-442109ac";
pub(crate) const A29_ACTION_GATE_SENTINEL_DUMP_SHA256: &str =
    "44169d40d8ccc979f2f78eb7be11e2e7995e28dc4f3c6feb9da48ae928257517";

#[allow(clippy::too_many_arguments)]
pub(crate) fn a29_expected_source_record_sha256(
    callback_sequence: u64,
    callback_thread_id: u32,
    callback_qpc_ticks: u64,
    qpc_frequency_hz: u64,
    producer_game_hour_raw: i32,
    research_slot_index: u32,
    technology_definition_index_raw: u32,
    branch: u32,
    quartet: (i64, i64, i64, i64),
    candidate_count: u32,
    selected_candidate_id: u64,
    selected_ordinal: u32,
    selected_game_hour_raw: i32,
    selected_qpc_ticks: u64,
    selection_cutoff_qpc_ticks: u64,
    candidate_set_sha256: &[u8; 32],
    selected_present_receipt_sha256: &[u8; 32],
) -> [u8; 32] {
    let mut canonical = Vec::with_capacity(512);
    canonical.extend_from_slice(b"the game:a729:a29-observed-producer:selected-present:v3\0");
    canonical.extend_from_slice(A29_EXECUTABLE_SHA256.as_bytes());
    canonical.extend_from_slice(A29_EXACT_ROOT_MANIFEST_SHA256.as_bytes());
    canonical.extend_from_slice(A29_SENTINEL_DUMP_SHA256.as_bytes());
    canonical.extend_from_slice(A29_ACTION_GATE_MANIFEST_SHA256.as_bytes());
    canonical.extend_from_slice(A29_ACTION_GATE_SENTINEL_DUMP_SHA256.as_bytes());
    canonical.extend_from_slice(&A29_UI_YEAR_HELPER_RETURN_RVA.to_le_bytes());
    canonical.extend_from_slice(&callback_sequence.to_le_bytes());
    canonical.extend_from_slice(&callback_thread_id.to_le_bytes());
    canonical.extend_from_slice(&callback_qpc_ticks.to_le_bytes());
    canonical.extend_from_slice(&qpc_frequency_hz.to_le_bytes());
    canonical.extend_from_slice(&producer_game_hour_raw.to_le_bytes());
    canonical.extend_from_slice(&research_slot_index.to_le_bytes());
    canonical.extend_from_slice(&technology_definition_index_raw.to_le_bytes());
    canonical.extend_from_slice(&branch.to_le_bytes());
    for value in [quartet.0, quartet.1, quartet.2, quartet.3] {
        canonical.extend_from_slice(&value.to_le_bytes());
    }
    canonical.extend_from_slice(&candidate_count.to_le_bytes());
    canonical.extend_from_slice(&selected_candidate_id.to_le_bytes());
    canonical.extend_from_slice(&selected_ordinal.to_le_bytes());
    canonical.extend_from_slice(&selected_game_hour_raw.to_le_bytes());
    canonical.extend_from_slice(&selected_qpc_ticks.to_le_bytes());
    canonical.extend_from_slice(&selection_cutoff_qpc_ticks.to_le_bytes());
    canonical.extend_from_slice(snapshot::sha256::to_hex(candidate_set_sha256).as_bytes());
    canonical.extend_from_slice(
        snapshot::sha256::to_hex(selected_present_receipt_sha256).as_bytes(),
    );
    let digest = Sha256::digest(canonical);
    let mut output = [0u8; 32];
    output.copy_from_slice(&digest);
    output
}

pub const SUPPORTED_DOMAIN_DATA_IDS: &[u16] = &[
    10, 11, 12, 13, 14, 15, 16, 17, 18, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35,
    36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54, 55, 56, 57, 60, 61,
    62, 63, 64, 65, 66, 67, 68, 69, 70, 71, 72, 75, 78, 79, 81, 82, 83, 84, 85, 86, 87, 88, 89,
    90, 91, 92, 93, 94, 95, 96, 97,
];

pub const COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2: u32 =
    compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2;
pub const COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER: u32 =
    compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER;
pub const COMPACT_STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
pub const COMPACT_STATUS_BRIDGE_UNAVAILABLE: u32 =
    compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE;
pub const COMPACT_STATUS_SOURCE_UNAVAILABLE: u32 =
    compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE;
/// The measured-zero badge. Informational, so it is deliberately absent from
/// [`COMPACT_STATUS_FAILURE_MASK`] and from the recorder's gap-eligible mask: a
/// domain that measured zero rows produced an ANSWER, and turning that answer
/// into a gap would put the recorder back in the state this badge exists to
/// leave — unable to say whether an hour is missing data or simply has none.
pub const COMPACT_STATUS_SOURCE_EMPTY: u32 = compact_contract::COMPACT_STATUS_SOURCE_EMPTY;
pub const COMPACT_STATUS_PARTIAL_DATA: u32 = compact_contract::COMPACT_STATUS_PARTIAL_DATA;
pub const COMPACT_STATUS_DELTA_DATA: u32 = compact_contract::COMPACT_STATUS_DELTA_DATA;
pub const COMPACT_STATUS_FAILURE_MASK: u32 = compact_contract::COMPACT_STATUS_FAILURE_MASK;
pub const COMPACT_STATUS_TRANSITION_BASELINE_RESET: u32 =
    compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET;
pub const COMPACT_STATUS_CONTROL_INVALID: u32 = compact_contract::COMPACT_STATUS_CONTROL_INVALID;
// Re-exported, not re-declared. A local copy of a wire constant drifts the
// moment the contract moves, and this file held one that silently pinned the
// diagnostic trailer to version 1 after the producer began writing version 2.
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC: &[u8; 4] =
    compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION: u16 =
    compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION;
pub const COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2: u16 =
    compact_contract::record_semantics::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_VERSION_V2;
pub const COMPACT_FUEL_SCALE_RAW: u32 = 32768;
pub const COMPACT_PRODUCTION_SCALE_RAW: u32 = 100000;
pub const COMPACT_TECH_PROGRESS_SCALE_RAW: u32 = 1000;
pub const COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW: u32 = 100000;
pub const COMPACT_FIXED_SCALE_RAW: u32 = 100000;
pub const COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE: u16 = 45;
pub const COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES: u16 = 46;
pub const COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE: u16 = 47;
pub const COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE: u16 = 48;
pub const COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE: u16 = 51;
pub const COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE: u16 = 52;
pub const COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE: u16 = diplomacy53::DOMAIN_ID;
pub const COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE: u16 = battleplans54::DOMAIN_ID;
pub const COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE: u16 = 55;
pub const COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE: u16 = 56;
pub const COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE: u16 =
    naval4950::DOMAIN_NAVAL_COMBAT_LIFECYCLE;
pub const COMPACT_DOMAIN_NAVAL_COMBAT_MEMBERS_LIVE: u16 = naval4950::DOMAIN_NAVAL_COMBAT_MEMBERS;
pub const COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE: u16 =
    naval4950::DOMAIN_CONVOY_TRANSPORT_ATTACK;
pub const COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE: u16 = session57::DOMAIN_ID;
pub const COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE: u16 = capwar60::DOMAIN_ID;
pub const COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE: u16 = airloss61::DOMAIN_ID;
pub const COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE: u16 = hub62::DOMAIN_ID;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_ROW_FLAG_ARCHETYPE: u32 = 1 << 1;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_ROW_FLAG_BASE_AIRFRAME: u32 = 1 << 2;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_ROW_FLAG_ROLE: u32 = 1 << 3;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_SOURCE_LIVE_STATS: u32 = 1;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_SOURCE_LIVE_TOKEN_DICTIONARY: u32 = 1 << 5;
pub const COMPACT_AIR_VARIANT_ARCHETYPE_SOURCE_STATIC_SOURCE_ENUM: u32 = 1 << 6;
pub const COMPACT_FRD_DEPTH_HEADER_LEN_V1: usize = 48;
pub const COMPACT_FRD_DEPTH_HEADER_LEN_V2: usize = 56;
pub const COMPACT_FRD_DEPTH_HEADER_LEN: usize = 60;
pub const COMPACT_FRD_DEPTH_HEADER_LEN_V5: usize = 64;
pub const COMPACT_FRD_DEPTH_SCHEMA_MAGIC: &[u8; 4] = b"FRD2";
pub const COMPACT_FRD_DEPTH_SCHEMA_VERSION_V2: u16 = 2;
pub const COMPACT_FRD_DEPTH_SCHEMA_VERSION_V3: u16 = 3;
pub const COMPACT_FRD_DEPTH_SCHEMA_VERSION: u16 = 4;
pub const COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5: u16 = 5;
pub const COMPACT_FRD_RESEARCH_ETA_ROW_LEN: usize = 80;
pub const COMPACT_FRD_DECISION_STATUS_ROW_LEN: usize = 28;
pub const COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN_V3: usize = 16;
pub const COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN: usize = 28;
pub const COMPACT_FRD_BADGE_ROW_FIXED_LEN: usize = 20;
pub const COMPACT_FRD_STATUS_FOCUS_PAUSE_BADGED: u32 = 1;
pub const COMPACT_FRD_STATUS_DECISION_TARGETS_BADGED: u32 = 1 << 1;
pub const COMPACT_FRD_STATUS_DIRECT_EVENTS_DERIVED: u32 = 1 << 2;
pub const COMPACT_FRD_STATUS_FOCUS_PAUSE_EXACT: u32 = 1 << 3;
pub const COMPACT_FRD_STATUS_DECISION_TARGETS_EXACT: u32 = 1 << 4;
pub const COMPACT_FRD_STATUS_DECISION_COOLDOWNS_EXACT: u32 = 1 << 5;
pub const COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED: u32 = 1 << 6;
/// D45-LOCAL direct-history reset badge (`DELTA_WIRE_DESIGN.md` §7.3.1).
///
/// "This record's `frd_direct_*` extent begins a NEW history; discard the
/// accumulation you hold for it." Its scope is the `frd_direct_*` maps and
/// NOTHING else -- focus, research and decision state is untouched, and the
/// hour it lands in is still typed as whatever it was, because a resync
/// recovers the FUTURE and never the hole.
///
/// It is deliberately NOT the global `COMPACT_STATUS_BASELINE_CLEAR`, and NOT
/// registered in `compact_contract::lane_local_status_mask`: `depth_status_flags`
/// is a separate `COMPACT_FRD_STATUS_*` namespace that never reaches a perf
/// record's `status_flags` (see the collector's
/// `worker_panic_constructor_published_status` census, which lists
/// `depth_status_flags` as exactly that). The mask below is the discipline that
/// actually governs this word, so this is where the bit is registered.
/// `d45_cannot_carry_the_global_resync_bit` records why the global bit could
/// not be used and must keep passing.
pub const COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET: u32 = 1 << 7;
// This remains the v1-v4 depth-authority mask.  The v5 direct-hook bit is a
// distinct trailer/source assertion and is admitted only for a v5 header.
//
// The reset badge is admitted on EVERY schema version rather than v5 only. The
// failure it recovers from has two shapes and one of them is a source
// DOWNGRADE -- the fold refuses an untrailered record once a validated one was
// accepted -- so the first record able to carry the badge is a v4 one. Gating
// the badge on v5 would leave that shape dead until the hooks came back, which
// is the defect rather than the fix.
pub const COMPACT_FRD_STATUS_KNOWN_MASK: u32 = COMPACT_FRD_STATUS_FOCUS_PAUSE_BADGED
    | COMPACT_FRD_STATUS_DECISION_TARGETS_BADGED
    | COMPACT_FRD_STATUS_DIRECT_EVENTS_DERIVED
    | COMPACT_FRD_STATUS_FOCUS_PAUSE_EXACT
    | COMPACT_FRD_STATUS_DECISION_TARGETS_EXACT
    | COMPACT_FRD_STATUS_DECISION_COOLDOWNS_EXACT
    | COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET;
pub const COMPACT_FRD_STATUS_V5_KNOWN_MASK: u32 =
    COMPACT_FRD_STATUS_KNOWN_MASK | COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED;
pub const COMPACT_FRD_BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED: u32 = 1;
pub const COMPACT_FRD_BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED: u32 = 2;
pub const COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE: u32 = 3;
pub const COMPACT_FRD_BADGE_KIND_MAX: u32 = COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE;
pub const COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED: u32 = 1;
pub const COMPACT_FRD_BADGE_FLAG_DERIVED_DETERMINISTIC: u32 = 1 << 1;
pub const COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED: u32 = 1 << 2;
pub const COMPACT_FRD_BADGE_FLAG_KNOWN_MASK: u32 = COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED
    | COMPACT_FRD_BADGE_FLAG_DERIVED_DETERMINISTIC
    | COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED;
pub const COMPACT_FRD_BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH: u32 = 1;
pub const COMPACT_FRD_BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED: u32 = 2;
pub const COMPACT_FRD_BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED: u32 = 3;
pub const COMPACT_FRD_BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED: &str =
    "focus_pause_flag_fail_closed_no_paused_true_ground_truth";
pub const COMPACT_FRD_BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED: &str =
    "decision_target_fields_fail_closed_body_layout_unvalidated";
pub const COMPACT_FRD_BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE: &str =
    "direct_event_hooks_unavailable_transitions_derived_from_compact_frames";
pub const COMPACT_FRD_FOCUS_EVENT_COMPLETED_COUNT_INCREASED: u32 = 1;
pub const COMPACT_FRD_FOCUS_EVENT_CURRENT_STARTED: u32 = 2;
pub const COMPACT_FRD_FOCUS_EVENT_STOPPED: u32 = 3;
pub const COMPACT_FRD_FOCUS_EVENT_CHANGED: u32 = 4;
pub const COMPACT_FRD_FOCUS_EVENT_PROGRESS_REGRESSED: u32 = 5;
pub const COMPACT_FRD_FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES: u32 = 6;
pub const COMPACT_FRD_FOCUS_EVENT_STATE_SNAPSHOT: u32 = 7;
pub const COMPACT_FRD_FOCUS_KIND_CONTINUOUS: u32 = 2;
pub const COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED: u32 = 1;
pub const COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT: u32 = 1 << 1;
pub const COMPACT_FRD_FOCUS_FLAG_PAUSED: u32 = 1 << 2;
pub const COMPACT_FRD_FOCUS_FLAG_PAUSED_PRESENT: u32 = 1 << 3;
pub const COMPACT_FRD_RESEARCH_FLAG_SAME_TECH: u32 = 1;
pub const COMPACT_FRD_RESEARCH_FLAG_POSITIVE_SPEED: u32 = 1 << 1;
pub const COMPACT_FRD_RESEARCH_FLAG_ETA_AVAILABLE: u32 = 1 << 2;
pub const COMPACT_FRD_RESEARCH_FLAG_TECH_ROLLOVER: u32 = 1 << 3;
pub const COMPACT_FRD_RESEARCH_FLAG_NEGATIVE_DELTA: u32 = 1 << 4;
pub const COMPACT_FRD_RESEARCH_FLAG_BASE_PRESENT: u32 = 1 << 5;
pub const COMPACT_FRD_DECISION_EVENT_STARTED: u32 = 1;
pub const COMPACT_FRD_DECISION_EVENT_STOPPED: u32 = 2;
pub const COMPACT_FRD_DECISION_EVENT_DAY_DELTA: u32 = 3;
pub const COMPACT_FRD_DECISION_EVENT_STATE_CHANGED: u32 = 4;
pub const COMPACT_FRD_DECISION_EVENT_STATUS_COUNTS_CHANGED: u32 = 5;
pub const COMPACT_FRD_DECISION_EVENT_RE_ENABLE_COOLDOWN_SNAPSHOT: u32 = 6;
pub const COMPACT_FRD_DECISION_EVENT_REMOVE_COOLDOWN_SNAPSHOT: u32 = 7;
pub const COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED: u32 = 1;
pub const COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED: u32 = 2;
pub const COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG: u32 = 1;
pub const COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20: u32 = 1 << 1;
pub const COMPACT_FRD_DECISION_TARGET_FLAG_NUMERIC_ID: u32 = 1 << 2;
pub const COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X24: u32 = 1 << 3;
pub const COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS: u32 =
    COMPACT_FRD_DECISION_TARGET_FLAG_COUNTRY_TAG | COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X20;
pub const COMPACT_MARKET_TRADE_HEADER_LEN: usize = 48;
pub const COMPACT_MARKET_TRADE_ROW_LEN: usize = 48;
pub const COMPACT_MARKET_TRADE_CONTRACT_TYPE: u32 = 81;
pub const COMPACT_MAX_OWNER_TAG_DB_ID: u32 =
    compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;

/// The D45 numeric decision target is a STATE identity (see the collector and
/// `world_fold`'s matching constant). It was `1_000_000` -- a state ceiling
/// under a role name.
pub const COMPACT_MAX_STATE_ID_FRD: u32 = compact_contract::build_profile::MAX_STATE_ID;
pub const COMPACT_MARKET_TRADE_MAX_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_MARKET_TRADE_MAX_CONTRACT_ID: u32 = i32::MAX as u32;
pub const COMPACT_MARKET_TRADE_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const COMPACT_MARKET_TRADE_ROW_FLAG_BUYER_PTR_VALID: u32 = 1;
pub const COMPACT_MARKET_TRADE_ROW_FLAG_EQUIPMENT_SUBBUF_VALID: u32 = 1 << 1;
pub const COMPACT_MARKET_TRADE_ROW_FLAG_DELIVERY_SUBBUF_VALID: u32 = 1 << 2;
pub const COMPACT_MARKET_TRADE_ROW_FLAG_VARIANT_VALID: u32 = 1 << 3;
pub const COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID: u32 = 1 << 4;
pub const COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC: &[u8; 4] = b"MTD1";
pub const COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN: usize = 32;
pub const COMPACT_MARKET_TRADE_DETAIL_ROW_LEN: usize = 64;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID: u32 = 1;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID: u32 = 1 << 1;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID: u32 = 1 << 2;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID: u32 = 1 << 3;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID: u32 = 1 << 4;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID: u32 = 1 << 5;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID: u32 = 1 << 6;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID: u32 = 1 << 7;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID: u32 = 1 << 8;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID: u32 = 1 << 9;
pub const COMPACT_MARKET_TRADE_DETAIL_FLAG_KNOWN_MASK: u32 =
    COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
        | COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID;
// MTRF: optional market-trade resource-flow trailer (per-country resource
// import/export flow), appended AFTER the MTD1 detail trailer. 32-byte header
// (magic, version u16, header_len u16, row_len u16, reserved u16, row_count u32,
// row_count-dup/country_count u32, scale u32, resource_mask-or-count u32,
// reserved/kind_mask u32); then row_count rows of (tag_db_id u32, flow_kind u32,
// resource_mask-or-count u32, reserved u32, then N x i64 per-resource values).
//
// N is seven in v1 and v2, which is why a mod defining an eighth strategic
// resource could not be published. v3 states the count at `+24` and derives
// `row_len` from it. Mirrors the collector's
// append_compact_market_trade_resource_flow_trailer wire layout.
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC: &[u8; 4] = b"MTRF";
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_LEGACY_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_FIXED7_VERSION: u16 = 2;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_VERSION: u16 = 3;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN: usize = 32;
/// v1/v2 only. A v3 row is `ROW_HEADER_LEN + 8 * resource_count`.
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN: usize = 72;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN: usize = 16;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK: u32 = 0x7F;
pub const COMPACT_MARKET_TRADE_RESOURCE_FLOW_KIND_MASK: u32 = 0x3F;
pub const COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC: &[u8; 4] = b"MTRH";
pub const COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_HEADER_LEN: usize = 24;
pub const COMPACT_MARKET_TRADE_FACTORY_HEADER_ROW_LEN: usize = 16;
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC: &[u8; 4] = b"MTRR";
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_LEGACY_VERSION: u16 = 1;
/// v2 states the resource COUNT; v3 also states each resource's IDENTITY in a
/// fixed-stride block between the header and the rows.
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_COUNT_ONLY_VERSION: u16 = 2;
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_VERSION: u16 = 3;
/// v4 is v3 plus one extra admitted ROW shape: an oil row whose needed-trio
/// could not be derived states that instead of being omitted. The header shape
/// -- including the dense `row_count == country_count * resource_count`
/// invariant this authority checks -- is byte-identical to v3.
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_NEEDED_UNAVAILABLE_VERSION: u16 = 4;
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_HEADER_LEN: usize = 32;
pub const COMPACT_MARKET_TRADE_RESOURCE_ROW_LEN: usize = 64;
/// One `MTRR` v3 identity entry.
pub const COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE: u32 =
    compact_contract::build_profile::COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE;
/// The width v1 `MTRR` and v1/v2 `MTRF` are pinned to.
pub const COMPACT_MARKET_TRADE_FIXED7_RESOURCE_COUNT: u32 = 7;
/// The one shared admission ceiling for a count-prefixed resource vector.
pub const COMPACT_MAX_STRATEGIC_RESOURCE_COUNT: u32 =
    compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_COUNT;
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC: &[u8; 4] = b"MTRD";
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_LEGACY_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_VERSION: u16 = 2;
/// v3 spends the header's last word on the incomplete-owner count and carries
/// that many census keys between the witness block and the rows.
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION: u16 = 3;
/// v4 keeps v3's header exactly and respells the ROW's deal identity at full
/// width: two `u32` country ids at `+0x20`/`+0x24` and a one-based `u32` ordinal
/// at `+0x5C`, in place of v1-v3's single packed `u64`. This decoder is
/// structural — it advances a cursor and counts units — so the only thing the
/// bump changes here is which versions are admitted and which of them carry the
/// incomplete block.
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_WIDE_IDENTITY_VERSION: u16 = 4;
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_HEADER_LEN: usize = 32;
pub const COMPACT_MARKET_TRADE_CLASSIC_DEAL_ROW_LEN: usize = 96;
pub const COMPACT_MARKET_TRADE_ACTIVATION_TRAILER_MAGIC: &[u8; 4] = b"MTRA";
pub const COMPACT_MARKET_TRADE_ACTIVATION_TRAILER_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_ACTIVATION_HEADER_LEN: usize = 32;
pub const COMPACT_MARKET_TRADE_ACTIVATION_FLAGS_KNOWN: u32 = 3;
pub const COMPACT_MARKET_TRADE_ACTIVATION_FLAG_ACTIVE: u32 = 1;
pub const COMPACT_MARKET_TRADE_ACTIVATION_ROOT_COUNTRYTRADEVIEW: u32 = 1;
pub const COMPACT_MARKET_TRADE_ACTIVATION_OPEN_RVA: u32 = 0x015e_c520;
pub const COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_MAGIC: &[u8; 4] = b"MTS1";
pub const COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_VERSION: u16 = 1;
pub const COMPACT_MARKET_TRADE_STOCKPILE_HEADER_LEN: usize = 40;
pub const COMPACT_MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN: usize = 20;
pub const COMPACT_MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN: usize = 24;
pub const COMPACT_MARKET_TRADE_STOCKPILE_GLOBAL_FLAGS: u32 = 3;
pub const COMPACT_MARKET_TRADE_STOCKPILE_COUNTRY_COMPLETE: u32 = 1;
pub const COMPACT_MARKET_TRADE_STOCKPILE_EQUIPMENT_TYPE: u32 = 70;

// --- W1-W6 newly wired domains 63-69: summarizer decode constants ----------
// These mirror the collector emit and the world_fold parse (recorder_host
// world_fold.rs). The summarizer decode below re-validates the wire shape and
// accounts row counts so the manifest carries zero parse_errors for these.
pub const COMPACT_DOMAIN_COUNTRY_CORE_SCALARS_LIVE: u16 = 63;
pub const COMPACT_COUNTRY_CORE_HEADER_LEN: usize = 20;
pub const COMPACT_COUNTRY_CORE_ROW_LEN: usize = 48;
pub const COMPACT_COUNTRY_CORE_FLAG_MANPOWER_POOL: u32 = 1 << 0;
pub const COMPACT_COUNTRY_CORE_FLAG_CONVOYS_DESTROYED: u32 = 1 << 1;
pub const COMPACT_COUNTRY_CORE_FLAG_DIVISIONS_LOST: u32 = 1 << 2;
pub const COMPACT_COUNTRY_CORE_FLAG_EXPERIENCE: u32 = 1 << 3;
pub const COMPACT_COUNTRY_CORE_KNOWN_FLAGS: u32 = COMPACT_COUNTRY_CORE_FLAG_MANPOWER_POOL
    | COMPACT_COUNTRY_CORE_FLAG_CONVOYS_DESTROYED
    | COMPACT_COUNTRY_CORE_FLAG_DIVISIONS_LOST
    | COMPACT_COUNTRY_CORE_FLAG_EXPERIENCE;
pub const COMPACT_COUNTRY_CORE_MAX_MANPOWER_POOL: u32 = 2_000_000_000;
pub const COMPACT_COUNTRY_CORE_MAX_CONVOYS_DESTROYED: u32 = 100_000_000;
pub const COMPACT_COUNTRY_CORE_MAX_DIVISIONS_LOST: u32 = 1_000_000;

pub const COMPACT_DOMAIN_DELAYED_EVENTS_LIVE: u16 = 64;
pub const COMPACT_DELAYED_EVENTS_HEADER_LEN: usize = 20;
pub const COMPACT_DELAYED_EVENT_ROW_LEN: usize = 20;
pub const COMPACT_DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS: u32 = 1 << 0;
pub const COMPACT_DELAYED_EVENT_FLAG_ORIGINATOR: u32 = 1 << 1;
pub const COMPACT_DELAYED_EVENT_KNOWN_FLAGS: u32 =
    COMPACT_DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS | COMPACT_DELAYED_EVENT_FLAG_ORIGINATOR;
pub const COMPACT_DELAYED_EVENT_MAX_DUE_REMAINING_HOURS: u32 = 500_000;
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_MAGIC: &[u8; 4] = b"DEL2";
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_VERSION: u16 = 1;
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN: usize = 48;
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_LEN: usize = 48;
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_FLAG_COMPLETE: u32 = 1 << 0;
pub const COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_FLAG_DUE_AFTER: u32 = 1 << 0;

pub const COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE: u16 = 65;
pub const COMPACT_SPECIAL_PROJECT_PROGRAM_HEADER_LEN: usize = 24;
pub const COMPACT_SPECIAL_PROJECT_PROGRAM_ROW_LEN: usize = 16;

pub const COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE: u16 = 66;
pub const COMPACT_OCCUPATION_STATE_EXTRAS_HEADER_LEN: usize = 20;
pub const COMPACT_OCCUPATION_STATE_EXTRAS_ROW_LEN: usize = 24;
pub const COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_LAW: u32 = 1 << 0;
pub const COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE: u32 = 1 << 1;
pub const COMPACT_OCCUPATION_STATE_EXTRAS_KNOWN_FLAGS: u32 =
    COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_LAW | COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE;

pub const COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE: u16 = 67;
pub const COMPACT_COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN: usize = 20;
pub const COMPACT_COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN: usize = 12;
pub const COMPACT_COUNTRY_DYNAMIC_MODIFIER_MAX_ID_LEN: usize = 128;

pub const COMPACT_DOMAIN_SESSION_HEADER_LIVE: u16 = 68;
pub const COMPACT_SESSION_HEADER_HEADER_LEN: usize = 16;
pub const COMPACT_SESSION_HEADER_SCALAR_BLOCK_LEN: usize = 20;
pub const COMPACT_SESSION_HEADER_FLAG_PLAYER: u32 = 1 << 0;
pub const COMPACT_SESSION_HEADER_FLAG_DLCS: u32 = 1 << 1;
pub const COMPACT_SESSION_HEADER_FLAG_START_DATE: u32 = 1 << 2;
pub const COMPACT_SESSION_HEADER_FLAG_SAVE_VERSION: u32 = 1 << 3;
pub const COMPACT_SESSION_HEADER_FLAG_SESSION: u32 = 1 << 4;
pub const COMPACT_SESSION_HEADER_FLAG_IDEOLOGY: u32 = 1 << 5;
pub const COMPACT_SESSION_HEADER_FLAG_COSMETIC_TAG: u32 = 1 << 6;
pub const COMPACT_SESSION_HEADER_FLAG_VERSION: u32 = 1 << 7;
pub const COMPACT_SESSION_HEADER_KNOWN_FLAGS: u32 = COMPACT_SESSION_HEADER_FLAG_PLAYER
    | COMPACT_SESSION_HEADER_FLAG_DLCS
    | COMPACT_SESSION_HEADER_FLAG_START_DATE
    | COMPACT_SESSION_HEADER_FLAG_SAVE_VERSION
    | COMPACT_SESSION_HEADER_FLAG_SESSION
    | COMPACT_SESSION_HEADER_FLAG_IDEOLOGY
    | COMPACT_SESSION_HEADER_FLAG_COSMETIC_TAG
    | COMPACT_SESSION_HEADER_FLAG_VERSION;
pub const COMPACT_SESSION_HEADER_MAX_STRING_LEN: usize = 128;
pub const COMPACT_SESSION_HEADER_MAX_PLAYER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;

pub const COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE: u16 = 69;
pub const COMPACT_TECH_SHARING_GROUPS_HEADER_LEN: usize = 16;
pub const COMPACT_TECH_SHARING_GROUPS_ROW_LEN: usize = 12;
// Tied to the collector's vector-size ceiling: the reader walks the
// tech-sharing std::vector's active size [0..size), not a fixed 9 slots, so a
// >9-group save emits >9 rows and this decoder cap MUST match (re-gate
// 019f5467 BLOCKER — a mismatched cap rejects a clean >9 frame outright).
pub const COMPACT_TECH_SHARING_GROUP_MAX_GROUPS: usize =
    compact_contract::build_profile::DOMAIN69_TECH_SHARING_MAX_GROUPS as usize;
pub const COMPACT_TECH_SHARING_GROUP_MAX_MEMBERS: usize = 512;
pub const COMPACT_TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE: u16 = compact_contract::naval_history::DOMAIN_ID;
pub const COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE: u16 = compact_contract::rail_replay::DOMAIN_ID;
pub const COMPACT_DOMAIN_RAID_INSTANCES_LIVE: u16 = compact_contract::raid_instances::DOMAIN_ID;
pub const COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE: u16 = preferred92::DOMAIN_ID;
pub const COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE: u16 = army_upgrades93::DOMAIN_ID;
pub const COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE: u16 = weight96::DOMAIN_ID;

pub const COMPACT_SUPPLY_LOGISTICS_HEADER_LEN: usize = supply44::domain44_contract::HEADER_LEN;
pub const COMPACT_SUPPLY_LOGISTICS_ROW_LEN: usize = supply44::domain44_contract::ROW_LEN;
pub const COMPACT_SUPPLY_ROW_KIND_PROVINCE_LOGISTICS: u32 =
    supply44::domain44_contract::ROW_KIND_PROVINCE_LOGISTICS;
pub const COMPACT_SUPPLY_ROW_KIND_DIVISION_SUPPLY: u32 =
    supply44::domain44_contract::ROW_KIND_DIVISION_SUPPLY;
pub const COMPACT_SUPPLY_ROW_KIND_COMPUTED_BADGE: u32 =
    supply44::domain44_contract::ROW_KIND_COMPUTED_BADGE;
pub const COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT: u32 =
    supply44::domain44_contract::ROW_KIND_CSUPPLY_PROVINCE_INPUT;
pub const COMPACT_SUPPLY_ROW_KIND_COUNTRY_DISRUPTED_SUPPLY: u32 =
    supply44::domain44_contract::ROW_KIND_COUNTRY_DISRUPTED_SUPPLY;
pub const COMPACT_SUPPLY_ROW_FLAG_RAIL: u32 = supply44::domain44_contract::ROW_FLAG_RAIL;
pub const COMPACT_SUPPLY_ROW_FLAG_SUPPLY_NODE: u32 =
    supply44::domain44_contract::ROW_FLAG_SUPPLY_NODE;
pub const COMPACT_SUPPLY_ROW_FLAG_NAVAL_SUPPLY_HUB: u32 =
    supply44::domain44_contract::ROW_FLAG_NAVAL_SUPPLY_HUB;
pub const COMPACT_SUPPLY_ROW_FLAG_INFRASTRUCTURE: u32 =
    supply44::domain44_contract::ROW_FLAG_INFRASTRUCTURE;
pub const COMPACT_SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR: u32 =
    supply44::domain44_contract::ROW_FLAG_LIVE_BUILDING_VECTOR;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_CURRENT_SUPPLY;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_MAX_SUPPLY;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_SUPPLY_GAIN;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_OWNER: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_OWNER;
pub const COMPACT_SUPPLY_ROW_FLAG_EXACT_SUPPLY_WALL: u32 =
    supply44::domain44_contract::ROW_FLAG_EXACT_SUPPLY_WALL;
pub const COMPACT_SUPPLY_ROW_FLAG_ATTRITION_WALL: u32 =
    supply44::domain44_contract::ROW_FLAG_ATTRITION_WALL;
pub const COMPACT_SUPPLY_ROW_FLAG_PROXY_VALUE: u32 =
    supply44::domain44_contract::ROW_FLAG_PROXY_VALUE;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_SUPPLY_DEFICIT;
pub const COMPACT_SUPPLY_ROW_FLAG_DIVISION_FUEL: u32 =
    supply44::domain44_contract::ROW_FLAG_DIVISION_FUEL;
pub const COMPACT_SUPPLY_ROW_FLAG_MOTORIZATION_WALL: u32 =
    supply44::domain44_contract::ROW_FLAG_MOTORIZATION_WALL;
pub const COMPACT_SUPPLY_ROW_FLAG_CONVOY_PRESSURE_WALL: u32 =
    supply44::domain44_contract::ROW_FLAG_CONVOY_PRESSURE_WALL;
pub const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED: u32 =
    supply44::domain44_contract::ROW_FLAG_CSUPPLY_CANDIDATE_REJECTED;
pub const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY: u32 =
    supply44::domain44_contract::ROW_FLAG_CSUPPLY_EFFICIENCY;
pub const COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY: u32 =
    supply44::domain44_contract::ROW_FLAG_CSUPPLY_PROVINCE_ARRAY;
pub const COMPACT_SUPPLY_ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT: u32 =
    supply44::domain44_contract::ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT;
pub const COMPACT_SUPPLY_ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: u32 =
    supply44::domain44_contract::ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE;
pub const COMPACT_SUPPLY_ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE: u32 =
    supply44::domain44_contract::ROW_FLAG_FOREIGN_HOMEBASE_SOURCE_AVAILABLE;
pub const COMPACT_SUPPLY_ROW_ID_DELETED: u32 = supply44::domain44_contract::ROW_ID_DELETED;
pub const COMPACT_SUPPLY_ROW_VALID_FLAGS: u32 =
    supply44::domain44_contract::PRODUCTION_VALID_ROW_FLAGS;
pub const COMPACT_SUPPLY_BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT: i32 =
    supply44::domain44_contract::BADGE_EXACT_PROVINCE_SUPPLY_TRANSIENT;
pub const COMPACT_SUPPLY_BADGE_ATTRITION_RUNTIME_COMPUTED: i32 =
    supply44::domain44_contract::BADGE_ATTRITION_RUNTIME_COMPUTED;
pub const COMPACT_SUPPLY_BADGE_MOTORIZATION_SETTINGS_UNPINNED: i32 =
    supply44::domain44_contract::BADGE_MOTORIZATION_SETTINGS_UNPINNED;
pub const COMPACT_SUPPLY_BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED: i32 =
    supply44::domain44_contract::BADGE_CONVOY_LOGISTICS_PRESSURE_UNPINNED;
pub const COMPACT_SUPPLY_BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE: i32 =
    supply44::domain44_contract::BADGE_CSUPPLY_INPUT_ARRAYS_UNSAFE;
pub const COMPACT_SUPPLY_BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE: i32 =
    supply44::domain44_contract::BADGE_COUNTRY_DISRUPTED_SOURCE_AVAILABLE;
pub const COMPACT_SUPPLY_BADGE_FOREIGN_HOMEBASE_SOURCE_AVAILABLE: i32 =
    supply44::domain44_contract::BADGE_FOREIGN_HOMEBASE_SOURCE_AVAILABLE;
// Respelling the number here is exactly the split-brain the comment below
// warns about: the vanilla province count was written out in full in three
// places, so raising the one home would have left this authority still
// discarding every row a mod's provinces produce.
pub const COMPACT_SUPPLY_MAX_PROVINCE_ID: u32 = supply44::MAX_PROVINCE_ID;
// The supply admission bounds have one home in `domain44_contract`; respelling
// them here is what let this authority and the lane gate drift apart.
pub const COMPACT_SUPPLY_MAX_STATE_ID: u32 = supply44::domain44_contract::MAX_STATE_ID;
pub const COMPACT_SUPPLY_MAX_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_SUPPLY_MAX_DIVISION_ID: u32 = i32::MAX as u32;
pub const COMPACT_SUPPLY_MAX_SUPPLY_RAW: i32 = supply44::domain44_contract::MAX_SUPPLY_RAW;
pub const COMPACT_SUPPLY_MAX_SUPPLY_GAIN_RAW: i32 =
    supply44::domain44_contract::MAX_SUPPLY_GAIN_RAW;
pub const COMPACT_SUPPLY_MAX_DIVISION_FUEL_RAW: i32 = i32::MAX;
pub const COMPACT_SUPPLY_MAX_BUILDING_LEVEL: i32 = supply44::domain44_contract::MAX_BUILDING_LEVEL;
/// Deliberately left at the fixed-point scale: the per-province CSupply input
/// is documented as an *efficiency*, for which a 0..=1.0 range is at least
/// plausible, and no measured capture has ever been excluded for it. That is
/// weaker than the evidence that refuted the disrupted-supply ceiling below —
/// this bound is unverified, not proven, and is a candidate for the same
/// treatment if a capture is ever lost to it.
pub const COMPACT_SUPPLY_MAX_CSUPPLY_INPUT_RAW: i32 = COMPACT_FIXED_SCALE_RAW as i32;
pub const COMPACT_SUPPLY_MAX_DISRUPTED_RAW: i32 =
    supply44::domain44_contract::MAX_COUNTRY_DISRUPTED_SUPPLY_RAW;
pub const COMPACT_DIVISION_TYPE: u32 = 4713;
// A15 extends the E4 104-byte row to 120 bytes with exact counter maximum
// organisation, supply-status, and dig-in operands.
pub const COMPACT_DIVISION_ROW_LEN: usize = 120;
pub const LEGACY_COMPACT_DIVISION_ROW_LEN: usize = 104;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
/// Was `1_000_000`, a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_MAX_DIVISION_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_MAX_DIVISION_COMBAT_REF_COUNT: u32 = 1_000;
pub const COMPACT_MAX_DIVISION_ORGANISATION_RAW: u32 = 100_000_000;
pub const COMPACT_MAX_DIVISION_STRENGTH_RAW: u32 = 1_000_000_000;
pub const COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_MAX_ARMY_ID: u32 = i32::MAX as u32;
pub const COMPACT_MAX_DIVISION_EXPERIENCE_RAW: u32 = u32::MAX;
pub const COMPACT_MAX_DIVISION_SUPPLY_RAW: u32 = 100_000_000;
pub const COMPACT_MAX_DIVISION_SUPPLY_GAIN_RAW: u32 = 500_000;
pub const COMPACT_MAX_DIVISION_COUNTER_SUPPLY_STATUS_RAW: u32 = 100_000;
pub const COMPACT_MAX_DIVISION_DIG_IN_RAW: u32 = 100_000_000;
pub const COMPACT_MAX_DIVISION_FUEL_RAW: u32 = u32::MAX;
pub const COMPACT_DIVISION_ROW_FLAG_DELETED: u32 = 1 << 12;
pub const COMPACT_DIVISION_ROW_FLAG_TEMPLATE: u32 = 1 << 13;
/// Wave-3 primary-order linkage flags (mutually exclusive). ORDER_ARMY
/// carries the COrderInstance+0x244 instance id in the row tail u32
/// (previously reserved-zero); ARMY_GROUP/UNRESOLVED badge without an id.
pub const COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY: u32 = 1 << 14;
pub const COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY_GROUP: u32 = 1 << 15;
pub const COMPACT_DIVISION_ROW_FLAG_ORDER_UNRESOLVED: u32 = 1 << 16;
/// Wave-4 disrupted-supply presence (row +88). A present 0 is valid; absent
/// (flag clear) must carry a zero field.
pub const COMPACT_DIVISION_ROW_FLAG_DISRUPTED_SUPPLY: u32 = 1 << 17;
pub const COMPACT_DIVISION_ROW_FLAG_STRATEGIC_REDEPLOYMENT: u32 = 1 << 18;
pub const COMPACT_DIVISION_ROW_FLAG_CURRENT_MANPOWER: u32 = 1 << 19;
pub const COMPACT_DIVISION_ROW_FLAG_CUMULATIVE_LOSSES: u32 = 1 << 20;
pub const COMPACT_DIVISION_ROW_FLAG_MAX_ORGANISATION: u32 = 1 << 21;
pub const COMPACT_DIVISION_ROW_FLAG_COUNTER_SUPPLY_STATUS: u32 = 1 << 22;
pub const COMPACT_DIVISION_ROW_FLAG_DIG_IN: u32 = 1 << 23;
pub const COMPACT_DIVISION_ROW_FLAG_DIG_IN_CAP: u32 = 1 << 24;
pub const COMPACT_DIVISION_ROW_FLAG_TRAINING_OBSERVED: u32 = 1 << 25;
pub const COMPACT_DIVISION_ROW_FLAG_TRAINING_VISIBLE: u32 = 1 << 26;
pub const COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_OBSERVED: u32 = 1 << 27;
pub const COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_VISIBLE: u32 = 1 << 28;
pub const COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_CAPPED: u32 = 1 << 29;
pub const COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_OBSERVED: u32 = 1 << 30;
pub const COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_VISIBLE: u32 = 1 << 31;
/// Sane ceiling for disrupted_supply raw (fixed x100000): a disrupted
/// fraction far above 1.0 is a torn read.
pub const COMPACT_MAX_DIVISION_DISRUPTED_SUPPLY_RAW: u32 = 1_000_000;
/// Shared ceiling for the division primary-order instance id (collector,
/// this summary decoder, and world_fold enforce the same bound).
pub const COMPACT_MAX_DIVISION_ORDER_INSTANCE_ID: u32 = 100_000_000;
pub const COMPACT_MAX_DIVISION_TEMPLATE_ID: u32 = i32::MAX as u32;
pub const COMPACT_MAX_DIVISION_CURRENT_MANPOWER: u32 = 10_000_000;
pub const COMPACT_MAX_DIVISION_CUMULATIVE_LOSSES: u32 = 100_000_000;
pub const COMPACT_DIVISION_EQUIPMENT_ROW_LEN: usize = 24;
pub const COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN: usize = 16;
pub const COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE: u32 = 1;
pub const COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED: u32 = 1 << 1;
pub const COMPACT_MAX_DIVISION_EQUIPMENT_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_MAX_DIVISION_EQUIPMENT_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const COMPACT_LAND_COMBAT_ROW_LEN: usize = 40;
pub const COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN: usize = 24;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
/// Was a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_LAND_COMBAT_MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_LAND_COMBAT_MAX_AGE_DAYS: u32 = 100_000;
pub const COMPACT_ARMY_ORDER_ROW_LEN: usize = 48;
pub const COMPACT_ARMY_ORDER_TARGET_ROW_LEN: usize = 8;
pub const COMPACT_ARMY_ORDER_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES: u32 = 1 << 1;
pub const COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT: u32 = 1 << 2;
pub const COMPACT_ARMY_ORDER_OWNER_KIND_ARMY: u32 = 0;
pub const COMPACT_ARMY_ORDER_OWNER_KIND_ARMY_GROUP: u32 = 1;
pub const COMPACT_ARMY_ORDER_MAX_SLOT_INDEX: u32 = 64;
/// State id space STRUCTURAL width (lane IA, `claim_39c989ce13504504353c`).
/// Was a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_ARMY_ORDER_MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_OOB_ROW_LEN: usize = 80;
pub const COMPACT_OOB_MEMBER_ROW_LEN: usize = 8;
pub const COMPACT_OOB_KIND_ARMY: u32 = 1;
pub const COMPACT_OOB_KIND_ARMY_GROUP: u32 = 2;
pub const COMPACT_OOB_KIND_FLEET: u32 = 3;
pub const COMPACT_OOB_KIND_TASK_FORCE: u32 = 4;
pub const COMPACT_OOB_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_OOB_ROW_FLAG_PARENT: u32 = 1 << 1;
pub const COMPACT_OOB_ROW_FLAG_OWNER: u32 = 1 << 2;
pub const COMPACT_OOB_ROW_FLAG_LEADER: u32 = 1 << 3;
pub const COMPACT_OOB_ROW_FLAG_LEADER_SKILL: u32 = 1 << 4;
pub const COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE: u32 = 1 << 5;
pub const COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT: u32 = 1 << 6;
pub const COMPACT_OOB_ROW_FLAG_LOCATION: u32 = 1 << 7;
pub const COMPACT_OOB_MAX_ID: u32 = 4_000_000;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
/// Was `1_000_000`, a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_OOB_MAX_LOCATION_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_OOB_MAX_MEMBER_ROWS: usize = 250_000;
pub const COMPACT_OOB_MAX_LEADER_SKILL: u32 = 15;
pub const COMPACT_SHIP_ROW_BASE_LEN: usize = 40;
pub const COMPACT_SHIP_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_SHIP_ROW_FLAG_TASK_FORCE: u32 = 1 << 1;
pub const COMPACT_SHIP_ROW_FLAG_DEFINITION: u32 = 1 << 2;
pub const COMPACT_SHIP_ROW_FLAG_OWNER: u32 = 1 << 3;
pub const COMPACT_SHIP_MAX_ID: u32 = 4_000_000;
pub const COMPACT_SHIP_MAX_OWNER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_SHIP_MAX_CATEGORY: u32 = 128;
pub const COMPACT_SHIP_MAX_STRENGTH_RAW: u32 = 1_000_000_000;
pub const COMPACT_SHIP_MAX_ORGANISATION_RAW: u32 = 100_000_000;
pub const COMPACT_SHIP_MAX_EXPERIENCE_RAW: u32 = 100_000_000;
pub const COMPACT_AIR_WING_ROW_LEN: usize = 40;
pub const COMPACT_AIR_WING_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION: u32 = 1 << 1;
pub const COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_AIR_WING_MAX_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_WING_MAX_PLANES: u32 = 10_000_000;
pub const COMPACT_AIR_WING_MAX_MISSION_REGION_ID: u32 = 65_535;
pub const COMPACT_AIR_REGION_ACTIVITY_ROW_LEN: usize = 32;
pub const COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_REGION_ACTIVITY_MAX_REGION_ID: u32 = 65_535;
pub const COMPACT_AIR_REGION_ACTIVITY_MAX_OWNER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_AIR_REGION_ACTIVITY_MAX_WING_COUNT: u32 = 20_000;
pub const COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES: u32 = 10_000_000;
pub const COMPACT_AIR_WING_LOADOUT_ROW_LEN: usize = 32;
pub const COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN: usize = 16;
pub const COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_WING_LOADOUT_MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_WING_LOADOUT_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_WING_LOADOUT_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN: usize = 48;
// Row grew from 68 to 76 bytes with the wave-3 per-wing cumulative scalars
// (air_accidents u32 @68, experience_raw i32 @72).
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_LEN: usize = 76;
pub const COMPACT_AIR_REGION_STATS_TRAILER_MAGIC: u32 = 0x3152_3441;
pub const COMPACT_AIR_REGION_STATS_TRAILER_VERSION: u16 = 2;
pub const COMPACT_AIR_REGION_STATS_TRAILER_LEGACY_VERSION: u16 = 1;
pub const COMPACT_AIR_REGION_STATS_TRAILER_HEADER_LEN: usize = 32;
pub const COMPACT_AIR_REGION_STATS_ROW_LEN: usize = 40;
pub const COMPACT_AIR_REGION_DAMAGE_ROW_LEN: usize = 16;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_LOSS: u32 = 1;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT: u32 = 1 << 1;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_DELETED: u32 = 1 << 2;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION: u32 = 1 << 3;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK: u32 = 1 << 4;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS: u32 = 1 << 5;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE: u32 = 1 << 6;
pub const COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALAR_CHANGE: u32 = 1 << 7;
pub const COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE: i64 = 100_000;
// Wing experience is cumulative XP, fixed-x100000 (not a 0..1 ratio).
pub const COMPACT_AIR_WING_MAX_EXPERIENCE_RAW: i32 = 1_000_000_000;
pub const COMPACT_AIR_VARIANT_DEF_ROW_LEN: usize = 24;
pub const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME: u32 = 1;
pub const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME: u32 = 1 << 1;
pub const COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES: u32 = 1 << 2;
pub const COMPACT_AIR_VARIANT_DEF_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_VARIANT_DEF_MAX_SLOT_HANDLE: u32 = 1_000_000;
pub const COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN: usize = 160;
pub const COMPACT_AIR_VARIANT_STATS_ROW_LEN: usize = 64;
pub const COMPACT_AIR_VARIANT_STATS_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_VARIANT_STATS_MAX_RAW: i32 = 1_000_000_000;
pub const COMPACT_AIR_VARIANT_UPGRADE_ROW_LEN: usize = 16;
pub const COMPACT_AIR_VARIANT_UPGRADE_ENTRY_ROW_LEN: usize = 8;
pub const COMPACT_AIR_VARIANT_UPGRADE_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_VARIANT_UPGRADE_MAX_INDEX: u32 = 256;
pub const COMPACT_AIR_VARIANT_UPGRADE_MAX_LEVEL: u32 = 255;
pub const COMPACT_PROJECT_HEADER_LEN: usize = 64;
pub const COMPACT_PROJECT_HEADER_EXT_ROOT_COUNTERS: u32 = 1;
pub const COMPACT_PROJECT_HEADER_EXT_ROOT_REASON_COUNTERS: u32 = 2;
pub const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1: u32 = 3;
pub const COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2: u32 = 4;
pub const COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1: u32 = 5;
pub const COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1: u32 = 6;
pub const COMPACT_PROJECT_ROOT_COUNTERS_LEN: usize = 24;
pub const COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN: usize = 48;
pub const COMPACT_PROJECT_PROGRESS_COUNTERS_LEN: usize = 16;
pub const COMPACT_PROJECT_FACILITY_ROW_LEN: usize = 32;
pub const COMPACT_PROJECT_ROW_LEN: usize = 24;
pub const COMPACT_PROJECT_SCIENTIST_ROW_LEN: usize = 32;
pub const COMPACT_PROJECT_STATE_ROW_LEN: usize = 48;
pub const COMPACT_PROJECT_BREAKTHROUGH_ROW_LEN: usize = 16;
pub const COMPACT_PROJECT_SOURCE_SCAN: u32 = 1;
pub const COMPACT_PROJECT_SOURCE_BRIDGE: u32 = 2;
pub const COMPACT_PROJECT_FACILITY_ROW_FLAG_ACTIVE: u32 = 1;
pub const COMPACT_PROJECT_FACILITY_ROW_FLAG_SCIENTIST: u32 = 1 << 1;
pub const COMPACT_PROJECT_FACILITY_ROW_FLAG_BUILDING: u32 = 1 << 2;
pub const COMPACT_PROJECT_FACILITY_ROW_FLAG_DELETED: u32 = 1 << 31;
pub const COMPACT_PROJECT_ROW_FLAG_WORKED_AT_FACILITY: u32 = 1;
pub const COMPACT_PROJECT_ROW_FLAG_DELETED: u32 = 1 << 31;
pub const COMPACT_PROJECT_SCIENTIST_ROW_FLAG_NAME_TOKEN: u32 = 1;
pub const COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DISPLAY_NAME: u32 = 1 << 1;
pub const COMPACT_PROJECT_SCIENTIST_ROW_FLAG_TRAIT_COUNT: u32 = 1 << 2;
pub const COMPACT_PROJECT_SCIENTIST_ROW_FLAG_SKILLS: u32 = 1 << 3;
pub const COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DELETED: u32 = 1 << 31;
pub const COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED: u32 = 1;
pub const COMPACT_PROJECT_STATE_ROW_FLAG_DELETED: u32 = 1 << 31;
pub const COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_PRESENT: u32 = 1;
pub const COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_SHIFT: u32 = 1;
pub const COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_MASK: u32 = 0x7 << 1;
pub const COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_SHIFT: u32 = 4;
pub const COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_MASK: u32 = 0x3 << 4;
pub const COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED: u32 = 1 << 31;
pub const COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_KNOWN_FLAGS: u32 =
    COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_PRESENT
        | COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_MASK
        | COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED;
pub const COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_KNOWN_FLAGS: u32 =
    COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_KNOWN_FLAGS
        | COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_MASK;
pub const COMPACT_PROJECT_MAX_ID: u32 = 10_000_000;
/// Province id space STRUCTURAL width (lane IA, `claim_347234c20a542ca79032`).
/// Was a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_PROJECT_MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_PROJECT_MAX_HANDLE: u32 = 10_000_000;
pub const COMPACT_PROJECT_MAX_ITERATIONS: u32 = 1_000_000;
pub const COMPACT_PROJECT_MAX_PROGRESS_RAW: u32 = 1_000_000_000;
pub const COMPACT_PROJECT_MAX_PHASE_TARGET_RAW: u32 = 1_000_000;
pub const COMPACT_PROJECT_MAX_STATE_ENUM: u32 = 64;
pub const COMPACT_PROJECT_MAX_BREAKTHROUGH_POINTS: u32 = 1_000_000_000;
pub const COMPACT_PROJECT_MAX_TRAITS: u32 = 256;
pub const COMPACT_PROJECT_MAX_SKILL_LEVEL: i32 = 1_000;
pub const COMPACT_PROJECT_MAX_BUILDING_LEN: usize = 96;
pub const COMPACT_PROJECT_MAX_SCIENTIST_TOKEN_LEN: usize = 160;
pub const COMPACT_PROJECT_MAX_SCIENTIST_DISPLAY_LEN: usize = 160;
pub const COMPACT_AIR_VARIANT_RESERVE_ROW_LEN: usize = 24;
pub const COMPACT_AIR_VARIANT_RESERVE_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_VARIANT_RESERVE_MAX_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_AIR_VARIANT_RESERVE_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_VARIANT_RESERVE_MAX_AMOUNT_RAW: i64 = 100_000_000_000_000_000;
pub const COMPACT_AIR_SUPERIORITY_INPUT_ROW_LEN: usize = 48;
pub const COMPACT_AIR_SUPERIORITY_INPUT_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_SUPERIORITY_INPUT_MAX_REGION_ID: u32 = 65_535;
pub const COMPACT_AIR_SUPERIORITY_INPUT_MAX_OWNER_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_AIR_SUPERIORITY_INPUT_MAX_VARIANT_ID: u32 = i32::MAX as u32;
pub const COMPACT_AIR_SUPERIORITY_INPUT_MAX_WING_COUNT: u32 = 20_000;
pub const COMPACT_AIR_SUPERIORITY_INPUT_MAX_AMOUNT_RAW: i64 = 1_000_000_000_000_000;
pub const COMPACT_AIR_BASE_STATUS_ROW_LEN: usize = 32;
pub const COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED: u32 = 1;
pub const COMPACT_AIR_BASE_STATUS_MAX_AIR_BASE_ID: u32 = i32::MAX as u32;
/// State id space STRUCTURAL width (lane IA, `claim_39c989ce13504504353c`).
/// Was a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_AIR_BASE_STATUS_MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_AIR_BASE_STATUS_MAX_CAPACITY: u32 = 1_000_000;
pub const COMPACT_AIR_BASE_STATUS_MAX_BUILDING_LEVEL: u32 = 1_000;
pub const COMPACT_AIR_BASE_STATUS_MAX_WING_COUNT: u32 = 20_000;
pub const COMPACT_AIR_BASE_STATUS_MAX_PLANES: u32 = 10_000_000;
pub const COMPACT_MAP_STATE_ROW_LEN: usize = 16;
pub const COMPACT_MAP_PROVINCE_ROW_LEN: usize = 16;
pub const COMPACT_MAP_DETAIL_STATE_ROW_LEN: usize = 28;
pub const COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN: usize = 20;
pub const COMPACT_MAP_DETAIL_ENTRY_ROW_LEN: usize = 12;
pub const COMPACT_WEATHER_REGION_ROW_LEN: usize = 32;
pub const COMPACT_WEATHER_PROVINCE_ROW_LEN: usize = 32;
// The wire-side twins of the collector's 1 000-state / 10 000-province map
// floors are gone. This decoder is STRUCTURAL: the payload length is required
// to equal `32 + state_count * ROW + province_count * ROW` exactly, and every
// row is then checked for a valid, unique id, in-range tags, and flag/value
// agreement. Nothing there needed a lower bound on how many rows a real map
// has -- and asserting one made a total conversion with a smaller map an
// EXCLUDED RECORD on every hour, which is the fail-closed refusal doing exactly
// the wrong thing. Lane GK's residual R3, closed by lane HJ; the producer's
// twelve floors were removed in the same wave.
/// See `world_fold.rs`'s `WEATHER_MAX_REGION_ROWS`: representation envelopes on
/// the two weather array lengths, NOT counts. These were `305` (the vanilla
/// strategic-region count, applied as an equality) and `10_000` (a floor tuned
/// on the vanilla map); either one rejected a modded map's whole weather
/// payload.
pub const COMPACT_WEATHER_MAX_REGION_ROWS: usize = 65_536;
/// The region array's floor: a POLICY on what counts as a resolved weather root,
/// not a claim about the engine. It is 1 rather than 2 because a region row
/// carries a dense 0-based index and so has no vacant slot to spend. It is not 0
/// because a zero-length region array is what an unloaded manager looks like, so
/// admitting it would publish an empty root as a real one -- a deliberate
/// fail-closed choice, not a proof that the vector must be non-empty. See
/// `collector/src/attach_log/offsets.rs`'s `WEATHER_MIN_REGION_COUNT`.
pub const COMPACT_WEATHER_MIN_REGION_ROWS: usize = 1;
/// The province array's floor, which is the engine's own and is NOT the region
/// array's: that array is subscripted BY PROVINCE ID and slot 0 is the vacant
/// location, so a loaded array holds at least that slot plus one real province.
/// It replaces a `10_000` tuned on the vanilla map.
pub const COMPACT_WEATHER_MIN_PROVINCE_ROWS: usize = 2;
pub const COMPACT_WEATHER_MAX_PROVINCE_ROWS: usize = 65_536;
/// State id space STRUCTURAL width (lane IA, `claim_39c989ce13504504353c`).
/// Was a guess; dossier `extractor/native/runs/ia_province_caps/SWEEP.md`.
pub const COMPACT_MAP_MAX_STATE_ID: u32 = (i32::MAX as u32) - 1;
/// Province id space STRUCTURAL width (lane IA). This one was additionally
/// spelled with the SLOT-COUNT value `65_536` rather than an id maximum -- an
/// identity carrying a count's number, which admitted one id past the last
/// addressable slot. Both halves fixed: it is an ID and it is the id space's
/// width (`claim_347234c20a542ca79032`).
pub const COMPACT_MAP_MAX_PROVINCE_ID: u32 = (i32::MAX as u32) - 1;
pub const COMPACT_MAP_MAX_TAG_DB_ID: u32 = COMPACT_MAX_OWNER_TAG_DB_ID;
pub const COMPACT_MAP_MAX_RESOURCE_AMOUNT_RAW: i32 = 100_000_000;
pub const COMPACT_MAP_MAX_BUILDING_LEVEL: u32 = 1_000;
// COMPACT_WEATHER_MAX_REGION_ID (= 304, the vanilla strategic-region count
// minus one) is deliberately gone rather than widened. A region id's ceiling is
// the region table's own length, which the payload header carries in
// `observed_region_rows`; a constant here would be a second, staler answer to a
// question the wire already answers, and re-adding one is how this defect
// returns.
pub const COMPACT_WEATHER_MAX_RAW_ACTIVITY_CANDIDATE: u32 = 16;
pub const COMPACT_WEATHER_MAX_TEMPERATURE_RAW: i32 = 10_000_000;
pub const COMPACT_WEATHER_MAX_TEMP_CHANGE_RAW: i32 = 10_000_000;
// COMPACT_WEATHER_MAX_PRECIP_RAW (= 100_000_000) is deliberately gone rather
// than widened, on the same ground as COMPACT_WEATHER_MAX_REGION_ID above: the
// engine's two precipitation ceilings are `CWeatherManager + 0x1a8` (water) and
// `+ 0x1f8` (snow), both parsed out of `common/weather.txt`, so a constant here
// is game content this crate made up. `100_000_000` is `1000.0` in
// `COMPACT_FIXED_SCALE_RAW` spelling and matched no engine value at all. The
// producer discovers both when the weather root resolves and carries them in the
// payload header's last two words, so this decoder bounds rows by the measured
// envelope. Authority site and engine proof:
// `collector/src/attach_log/offsets.rs::WEATHER_MANAGER_MAX_WATER_RAW_OFF`.
pub const COMPACT_WEATHER_MAX_ACTIVE_MODIFIERS: u32 = 4_096;
pub const COMPACT_WEATHER_REGION_FLAG_RAIN_LIGHT: u32 = 1;
pub const COMPACT_WEATHER_REGION_FLAG_RAIN_HEAVY: u32 = 1 << 1;
pub const COMPACT_WEATHER_REGION_FLAG_SNOW: u32 = 1 << 2;
pub const COMPACT_WEATHER_REGION_FLAG_BLIZZARD: u32 = 1 << 3;
pub const COMPACT_WEATHER_REGION_FLAG_ARCTIC_WATER: u32 = 1 << 4;
pub const COMPACT_WEATHER_REGION_FLAG_ACTIVE_SIGNAL: u32 = 1 << 5;
pub const COMPACT_WEATHER_REGION_FLAG_PHENOMENON_ACTIVE: u32 = 1 << 6;
pub const COMPACT_WEATHER_REGION_FLAG_SANDSTORM: u32 = 1 << 7;
pub const COMPACT_WEATHER_REGION_KNOWN_FLAGS: u32 = COMPACT_WEATHER_REGION_FLAG_RAIN_LIGHT
    | COMPACT_WEATHER_REGION_FLAG_RAIN_HEAVY
    | COMPACT_WEATHER_REGION_FLAG_SNOW
    | COMPACT_WEATHER_REGION_FLAG_BLIZZARD
    | COMPACT_WEATHER_REGION_FLAG_ARCTIC_WATER
    | COMPACT_WEATHER_REGION_FLAG_ACTIVE_SIGNAL
    | COMPACT_WEATHER_REGION_FLAG_PHENOMENON_ACTIVE
    | COMPACT_WEATHER_REGION_FLAG_SANDSTORM;
pub const COMPACT_WEATHER_PROVINCE_FLAG_TEMP_PRESENT: u32 = 1;
pub const COMPACT_WEATHER_PROVINCE_FLAG_TEMP_CHANGE_PRESENT: u32 = 1 << 1;
pub const COMPACT_WEATHER_PROVINCE_FLAG_WATER_PRESENT: u32 = 1 << 2;
pub const COMPACT_WEATHER_PROVINCE_FLAG_SNOW_PRESENT: u32 = 1 << 3;
pub const COMPACT_WEATHER_PROVINCE_FLAG_MUD_PRESENT: u32 = 1 << 4;
pub const COMPACT_WEATHER_PROVINCE_FLAG_MUD_ACTIVE: u32 = 1 << 5;
pub const COMPACT_WEATHER_PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT: u32 = 1 << 6;
pub const COMPACT_WEATHER_PROVINCE_FLAG_STRUCT_VALIDATED: u32 = 1 << 7;
pub const COMPACT_WEATHER_PROVINCE_FLAG_MAP_RECOLOR_DRIVER: u32 = 1 << 8;
pub const COMPACT_WEATHER_PROVINCE_FLAG_DYNAMIC_LUT_DRIVER: u32 = 1 << 9;
pub const COMPACT_WEATHER_PROVINCE_KNOWN_FLAGS: u32 = COMPACT_WEATHER_PROVINCE_FLAG_TEMP_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_TEMP_CHANGE_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_WATER_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_SNOW_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_MUD_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_MUD_ACTIVE
    | COMPACT_WEATHER_PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT
    | COMPACT_WEATHER_PROVINCE_FLAG_STRUCT_VALIDATED
    | COMPACT_WEATHER_PROVINCE_FLAG_MAP_RECOLOR_DRIVER
    | COMPACT_WEATHER_PROVINCE_FLAG_DYNAMIC_LUT_DRIVER;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain57ScalarEvidence {
    pub event_hour_raw: i32,
    pub value_i32: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain57FrameEvidence {
    pub sequence: u64,
    pub header_hour_raw: i32,
    pub payload_hour_raw: i32,
    pub root_evidence_mask: u32,
    pub speed_rows: Vec<Domain57ScalarEvidence>,
    pub pause_rows: Vec<Domain57ScalarEvidence>,
}

#[derive(Debug, Clone, Default)]
pub struct DomainDataAuxiliary {
    pub domain57_frame: Option<Domain57FrameEvidence>,
    pub domain68_session_header_identity: Option<SessionHeaderIdentityObservation>,
}

#[derive(Debug, Clone, Default)]
pub struct DomainDataAuthority {
    pub status_flags: Option<u32>,
    pub emitted_rows: u64,
    pub changed_rows: u64,
    pub data_units: BTreeMap<String, u64>,
    pub problems: BTreeMap<String, u64>,
    pub auxiliary: DomainDataAuxiliary,
}

pub struct DomainDataContext<'a> {
    pub stream_offset: usize,
    pub record_flags: u32,
    pub sequence: u64,
    pub header_hour_raw: i32,
    pub session_header_identity: Option<&'a mut SessionHeaderIdentityObservation>,
}

impl Default for DomainDataContext<'_> {
    fn default() -> Self {
        Self {
            stream_offset: 0,
            record_flags: 0,
            sequence: 0,
            header_hour_raw: 0,
            session_header_identity: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DomainDataError {
    pub errors: Vec<String>,
    pub partial_authority: DomainDataAuthority,
}

impl std::fmt::Display for DomainDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.errors.join("; "))
    }
}

impl std::error::Error for DomainDataError {}

#[derive(Debug, Default)]
struct DomainDataCounts {
    data_rows_total: u64,
    data_changed_rows_total: u64,
    data_units_total: BTreeMap<String, u64>,
    status_flags_or: u32,
    problems: BTreeMap<String, u64>,
}

#[derive(Debug, Default)]
struct DomainDataDecodeSummary {
    domain_id: u16,
    parse_errors: Vec<String>,
    raw_status_flags_or: Option<u32>,
    status_flags_or: u32,
    per_domain: BTreeMap<u16, DomainDataCounts>,
    domain57_frames: Vec<Domain57FrameEvidence>,
    session_header_identity: SessionHeaderIdentityObservation,
}

/// The one place every payload decoder's status word enters this authority.
///
/// 87 call sites reach it; nothing else may write
/// [`DomainDataDecodeSummary::raw_status_flags_or`]. That single-funnel property
/// is what lets [`reject_unassigned_status_bits`] be one gate instead of one
/// per domain, and `every_decoder_status_word_reaches_the_shared_funnel` is what
/// keeps a new decoder from bypassing it.
fn add_decoder_status(summary: &mut DomainDataDecodeSummary, status_flags: u32) {
    summary.raw_status_flags_or = Some(summary.raw_status_flags_or.unwrap_or(0) | status_flags);
    summary.status_flags_or |=
        compact_contract::compact_status_for_global_interpretation(summary.domain_id, status_flags);
}

/// Refuse a `DOMAIN_DATA` record whose status word carries a bit the contract
/// does not assign and this domain has not declared lane-locally.
///
/// # Owner ruling 2026-08-05: reject at the decoder
///
/// The collector DLL and this host ship together, so a status bit neither side
/// defines is not version skew — it is producer/consumer wire disagreement,
/// i.e. corruption or an unregistered producer-side assignment. Before this
/// gate such a bit was INVISIBLE: [`compact_contract::COMPACT_STATUS_FAILURE_MASK`]
/// covers assigned bits only, and no payload decoder called
/// [`compact_contract::compact_status_unknown_flags_v1`], so a record carrying
/// bit 20 folded exactly like a clean one.
///
/// The refusal is one gate at the shared boundary rather than 40 copies:
/// [`add_decoder_status`] is the single funnel every decoder's status word
/// passes through, so testing the accumulated word once here covers every
/// domain [`summarize_domain_data_payload`] dispatches.
///
/// # Why a parse error and not a new outcome type
///
/// A non-empty `parse_errors` makes [`summarize_domain_data_payload`] return
/// [`DomainDataError`], which the two production drivers already turn into an
/// ACCOUNTED gap rather than a silent drop:
/// `stream_fold::drive_fold_compressed_with_identity_mode` records an
/// [`crate::stream_fold::ExcludedDomainDataRecord`] plus a
/// `malformed_payload_domains` entry and calls
/// `WorldFold::record_malformed_domain_payload`, and the manifest path in
/// `recorder_host::main` pushes the same excluded record. The domain-hour
/// therefore reads as refused-with-a-reason (partial data still seals), not as
/// clean. Reusing that machinery is also why the reason is a string: it lands
/// verbatim in `validation_reason`, so the offending bits are what the ledger
/// SAYS (a degraded outcome names its cause).
///
/// Lane-local badges are NOT refused: `compact_status_unassigned_flags` strips
/// this domain's declared mask first, so d53's bits 16..=22 and d60's 16..=17 —
/// production-emitted, outside the v1 vocabulary, and legitimate — pass. d17's
/// `WAR_SEGMENT_ABSENT` is an ASSIGNED informational bit and likewise passes;
/// this gate never re-decides the failure/informational split, only whether the
/// bit has a definition at all.
fn reject_unassigned_status_bits(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    offset: usize,
) {
    let Some(raw_status_flags) = summary.raw_status_flags_or else {
        return;
    };
    let unassigned = compact_contract::compact_status_unassigned_flags(domain_id, raw_status_flags);
    if unassigned == 0 {
        return;
    }
    let bits = compact_contract::compact_status_unassigned_bit_indices(unassigned)
        .into_iter()
        .map(|bit| format!("bit {bit}"))
        .collect::<Vec<_>>()
        .join(", ");
    summary.parse_errors.push(format!(
        "offset {offset}: domain {domain_id} status word {raw_status_flags:#x} carries unassigned \
         status flag(s) {unassigned:#x} ({bits}); compact-stream contract v1 assigns only \
         {:#x} and this domain declares lane-local {:#x}, so producer and consumer disagree \
         about the wire",
        compact_contract::COMPACT_STATUS_V1_KNOWN_MASK,
        compact_contract::lane_local_status_mask(domain_id),
    ));
}

fn add_compact_global_status_flags(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    status_flags: u32,
) {
    debug_assert_eq!(summary.domain_id, domain_id);
    add_decoder_status(summary, status_flags);
}

/// Fold one record's value for `name` into this frame's decode aggregate, by
/// the unit's declared accumulation class.
///
/// A domain can publish more than one record in a frame, so the within-frame
/// accumulation has to obey the same rule as the cross-frame one. Summing two
/// records' copies of an absolute running total is the same defect at a
/// smaller scale. `compact_contract::diagnostic_channel::Accumulation` is the
/// authority; this function is the only arithmetic here.
fn fold_named_unit(aggregate: &mut BTreeMap<String, u64>, name: &str, value: u64) {
    let class = compact_contract::diagnostic_channel::unit_accumulation(name);
    match aggregate.get_mut(name) {
        Some(entry) => *entry = class.fold(Some(*entry), value),
        None => {
            aggregate.insert(name.to_string(), class.fold(None, value));
        }
    }
}

fn add_compact_domain_problem(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    problem: &str,
    count: u64,
) {
    if count == 0 {
        return;
    }
    let domain = summary.per_domain.entry(domain_id).or_default();
    fold_named_unit(&mut domain.problems, problem, count);
}

/// Record a PER-FRAME quantity: a count derived from the payload of the frame
/// this record rode in.
///
/// A value that the producer maintains across frames must NOT come through
/// here. `debug_assert` states that, and the release path still folds by the
/// declared class, so a miscategorised call is a loud test failure rather than
/// a silently inflated manifest figure.
fn add_compact_domain_data_unit(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    name: &str,
    count: usize,
) {
    debug_assert!(
        matches!(
            compact_contract::diagnostic_channel::unit_accumulation(name),
            compact_contract::diagnostic_channel::Accumulation::PerFrame
                | compact_contract::diagnostic_channel::Accumulation::LastValue
                | compact_contract::diagnostic_channel::Accumulation::BitwiseOr
        ),
        "{name} is not a per-frame unit; use add_compact_domain_cumulative_unit"
    );
    // Skipping a zero is only safe when zero is the fold's identity. Adding 0
    // to a sum and OR-ing 0 into a mask both change nothing, so dropping those
    // saves a map entry and nothing else.
    //
    // For a LastValue unit it is NOT safe, and dropping it was a real defect:
    // `air_strength_first_invalid_reason` is submitted on every frame, so a
    // frame that failed with reason 31 followed by a CLEAN frame reporting 0
    // left 31 standing in the manifest. The capture then reported a failure
    // reason for a domain whose last observation was "no failure".
    let class = compact_contract::diagnostic_channel::unit_accumulation(name);
    if count == 0
        && matches!(
            class,
            compact_contract::diagnostic_channel::Accumulation::PerFrame
                | compact_contract::diagnostic_channel::Accumulation::BitwiseOr
                | compact_contract::diagnostic_channel::Accumulation::Undeclared
        )
    {
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        fold_named_unit(&mut domain.data_units_total, name, count as u64);
    }
}

/// Record a value the PRODUCER maintains across frames: an absolute running
/// total, a running maximum, a latching boolean, a session constant, or an
/// instantaneous level.
///
/// Separate from [`add_compact_domain_data_unit`] so the caller states which
/// kind of number it holds at the call site, and so a name that has not been
/// classified in the contract cannot be recorded as cumulative by accident.
///
/// Unlike the per-frame entry point this does NOT drop a zero: for a cumulative
/// unit, zero is the measurement "nothing has happened yet", and dropping it
/// would make "the counter is still 0" indistinguishable from "the domain did
/// not report", which is the [`UnitPresence`] mistake in a different costume.
fn add_compact_domain_cumulative_unit(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    name: &str,
    value: u64,
) {
    debug_assert!(
        matches!(
            compact_contract::diagnostic_channel::unit_accumulation(name),
            compact_contract::diagnostic_channel::Accumulation::Cumulative
                | compact_contract::diagnostic_channel::Accumulation::Gauge
                | compact_contract::diagnostic_channel::Accumulation::LastValue
        ),
        "{name} is not declared cumulative in compact_contract::diagnostic_channel"
    );
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        fold_named_unit(&mut domain.data_units_total, name, value);
    }
}

fn compact_sum_to_usize(value: u128) -> usize {
    value.min(usize::MAX as u128) as usize
}

fn decode_compact_fuel_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    add_decoder_status(summary, status_flags);
    if scale_raw != COMPACT_FUEL_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data scale {scale_raw} != expected {COMPACT_FUEL_SCALE_RAW}"
        ));
    }
    let expected_len = 24usize.saturating_add(row_count.saturating_mul(40));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data payload is {} byte(s), expected {expected_len} for {row_count} row(s)",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: fuel data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&10) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 10, "fuel_country_rows", row_count);
    let mut seen_tag_ids = BTreeSet::new();
    for row_index in 0..row_count {
        let row_offset = 24 + row_index * 40;
        let tag_db_id = read_u32_le(payload, row_offset);
        let row_flags = read_u32_le(payload, row_offset + 4);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} has zero tag_db_id"
            ));
        } else if !seen_tag_ids.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if row_flags & !1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} has unknown flags {row_flags}"
            ));
        }
        let current_raw = read_i64_le(payload, row_offset + 8);
        let max_raw = read_i64_le(payload, row_offset + 16);
        let gain_raw = read_i64_le(payload, row_offset + 24);
        let cost_raw = read_i64_le(payload, row_offset + 32);
        if current_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} current_raw is negative"
            ));
        }
        if max_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: fuel data row {row_index} max_raw is negative"
            ));
        }
        let _ = gain_raw;
        let _ = cost_raw;
    }
}

fn decode_compact_country_identity_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 20 {
        summary.parse_errors.push(format!(
            "offset {offset}: country_identity data payload is {} byte(s), expected at least 20",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: country_identity data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&18) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 18, "country_identity_rows", row_count);
    let mut cursor = 20usize;
    let mut seen_tag_ids = BTreeSet::new();
    let mut cosmetic_present_rows = 0usize;
    let mut cosmetic_null_rows = 0usize;
    let mut cosmetic_absent_rows = 0usize;
    for row_index in 0..row_count {
        if cursor + 4 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_identity data row {row_index} truncated before tag_db_id"
            ));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        let tag = match read_compact_string(payload, &mut cursor, "country_identity tag") {
            Ok(tag) => tag,
            Err(err) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: country_identity data row {row_index}: {err}"
                ));
                return;
            }
        };
        // Wave-4 cosmetic tag: 1-byte marker (0 absent / 1 null / 2 present),
        // then a length-prefixed string only when present.
        if cursor >= payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_identity data row {row_index} truncated before cosmetic marker"
            ));
            return;
        }
        let cosmetic_marker = payload[cursor];
        cursor += 1;
        match cosmetic_marker {
            0 => cosmetic_absent_rows += 1,
            1 => cosmetic_null_rows += 1,
            2 => {
                cosmetic_present_rows += 1;
                match read_compact_string(payload, &mut cursor, "country_identity cosmetic_tag") {
                    Ok(cosmetic) => {
                        if cosmetic.is_empty() {
                            summary.parse_errors.push(format!(
                                "offset {offset}: country_identity data row {row_index} has present cosmetic marker with empty string"
                            ));
                        }
                    }
                    Err(err) => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: country_identity data row {row_index}: {err}"
                        ));
                        return;
                    }
                }
            }
            other => {
                summary.parse_errors.push(format!(
                    "offset {offset}: country_identity data row {row_index} has invalid cosmetic marker {other}"
                ));
                return;
            }
        }
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country_identity data row {row_index} has zero tag_db_id"
            ));
        } else if !seen_tag_ids.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: country_identity data row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if tag.is_empty() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_identity data row {row_index} has empty tag"
            ));
        }
    }
    add_compact_domain_data_unit(
        summary,
        18,
        "country_cosmetic_tag_rows",
        cosmetic_present_rows,
    );
    add_compact_domain_data_unit(
        summary,
        18,
        "country_cosmetic_null_rows",
        cosmetic_null_rows,
    );
    add_compact_domain_data_unit(
        summary,
        18,
        "country_cosmetic_absent_rows",
        cosmetic_absent_rows,
    );
    if cursor < payload.len() {
        let parsed = (|| -> Result<(usize, usize, usize, usize, usize, usize, u32), String> {
            const HEADER_LEN: usize = 40;
            if payload.len().saturating_sub(cursor) < HEADER_LEN
                || &payload[cursor..cursor + 4] != b"CIM1"
                || read_u16_le(payload, cursor + 4) != 1
                || read_u16_le(payload, cursor + 6) as usize != HEADER_LEN
                || read_u32_le(payload, cursor + 36) != 0
            {
                return Err("CIM1 header is invalid".into());
            }
            let claims_status = read_u32_le(payload, cursor + 8);
            let cores_status = read_u32_le(payload, cursor + 12);
            let major_status = read_u32_le(payload, cursor + 16);
            let states = read_u32_le(payload, cursor + 20) as usize;
            let expected_claims = read_u32_le(payload, cursor + 24) as usize;
            let expected_cores = read_u32_le(payload, cursor + 28) as usize;
            let majors = read_u32_le(payload, cursor + 32) as usize;
            let claims_available = !compact_contract::compact_status_is_failure(claims_status);
            let cores_available = !compact_contract::compact_status_is_failure(cores_status);
            let major_available = !compact_contract::compact_status_is_failure(major_status);
            if states > 100_000
                || expected_claims > 1_000_000
                || expected_cores > 1_000_000
                || majors > seen_tag_ids.len()
                || (!claims_available && expected_claims != 0)
                || (!cores_available && expected_cores != 0)
                || (!claims_available && !cores_available && states != 0)
                || (!major_available && majors != 0)
                || (major_available && majors != seen_tag_ids.len())
            {
                return Err("CIM1 status/count contract is invalid".into());
            }
            let mut at = cursor + HEADER_LEN;
            let mut claim_total = 0usize;
            let mut core_total = 0usize;
            let mut previous_state = 0u32;
            for state_index in 0..states {
                if payload.len().saturating_sub(at) < 12 {
                    return Err(format!("CIM1 state row {state_index} is truncated"));
                }
                let state_id = read_u32_le(payload, at);
                let claim_count = read_u32_le(payload, at + 4) as usize;
                let core_count = read_u32_le(payload, at + 8) as usize;
                at += 12;
                if state_id == 0 || (state_index != 0 && state_id <= previous_state) {
                    return Err(format!("CIM1 state row {state_index} is not strictly ordered"));
                }
                previous_state = state_id;
                if (!claims_available && claim_count != 0)
                    || (!cores_available && core_count != 0)
                {
                    return Err(format!("CIM1 unavailable state row {state_id} carries members"));
                }
                let member_bytes = claim_count
                    .checked_add(core_count)
                    .and_then(|count| count.checked_mul(4))
                    .ok_or_else(|| "CIM1 state member size overflow".to_string())?;
                if payload.len().saturating_sub(at) < member_bytes {
                    return Err(format!("CIM1 state row {state_id} members are truncated"));
                }
                let mut previous_tag = 0u32;
                for member_index in 0..claim_count {
                    let tag = read_u32_le(payload, at);
                    at += 4;
                    if tag == 0
                        || (member_index != 0 && tag <= previous_tag)
                        || !seen_tag_ids.contains(&tag)
                    {
                        return Err(format!("CIM1 state {state_id} claim member is invalid"));
                    }
                    previous_tag = tag;
                    claim_total += 1;
                }
                previous_tag = 0;
                for member_index in 0..core_count {
                    let tag = read_u32_le(payload, at);
                    at += 4;
                    if tag == 0
                        || (member_index != 0 && tag <= previous_tag)
                        || !seen_tag_ids.contains(&tag)
                    {
                        return Err(format!("CIM1 state {state_id} core member is invalid"));
                    }
                    previous_tag = tag;
                    core_total += 1;
                }
            }
            if claim_total != expected_claims || core_total != expected_cores {
                return Err("CIM1 membership totals do not match header".into());
            }
            let mut major_true = 0usize;
            let mut major_ids = BTreeSet::new();
            let mut previous_major = 0u32;
            for major_index in 0..majors {
                if payload.len().saturating_sub(at) < 8 {
                    return Err(format!("CIM1 major row {major_index} is truncated"));
                }
                let tag = read_u32_le(payload, at);
                let marker = payload[at + 4];
                if tag == 0
                    || (major_index != 0 && tag <= previous_major)
                    || marker > 1
                    || payload[at + 5..at + 8] != [0, 0, 0]
                    || !seen_tag_ids.contains(&tag)
                {
                    return Err(format!("CIM1 major row {major_index} is invalid"));
                }
                previous_major = tag;
                major_ids.insert(tag);
                major_true += usize::from(marker != 0);
                at += 8;
            }
            if major_available && major_ids != seen_tag_ids {
                return Err("CIM1 major roster is incomplete".into());
            }
            if at != payload.len() {
                return Err(format!("CIM1 has {} trailing byte(s)", payload.len() - at));
            }
            Ok((
                states,
                claim_total,
                core_total,
                majors,
                major_true,
                at,
                claims_status | cores_status | major_status,
            ))
        })();
        match parsed {
            Ok((states, claims, cores, majors, major_true, consumed, cim1_status_or)) => {
                // The three CIM1 sub-statuses are the SAME global vocabulary the
                // record status uses (the collector writes exactly 0 or
                // `SOURCE_UNAVAILABLE | PARTIAL_DATA`), and the production fold
                // gates on them: `claims_available`/`cores_available`/
                // `major_available` decide whether the claims, cores and
                // dynamic-major families are published for this hour AT ALL.
                // They were read here for the local count contract and then
                // dropped, so a d18 record whose base status was clean while its
                // membership read failed certified clean and silently shipped an
                // hour with no claims. Feeding them to the classifier is what
                // makes the status word say what the fold already acted on.
                add_decoder_status(summary, cim1_status_or);
                if let Some(domain) = summary.per_domain.get_mut(&18) {
                    domain.status_flags_or |= cim1_status_or;
                }
                add_compact_domain_data_unit(summary, 18, "country_membership_state_rows", states);
                add_compact_domain_data_unit(summary, 18, "country_claim_memberships", claims);
                add_compact_domain_data_unit(summary, 18, "country_core_memberships", cores);
                add_compact_domain_data_unit(summary, 18, "country_dynamic_major_rows", majors);
                add_compact_domain_data_unit(
                    summary,
                    18,
                    "country_dynamic_major_true_rows",
                    major_true,
                );
                cursor = consumed;
            }
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: country_identity data {error}"
                ));
                return;
            }
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: country_identity data payload has {} trailing byte(s) after {row_count} row(s)",
            payload.len() - cursor
        ));
    }
}

fn decode_compact_production_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: production data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let stockpile_count = read_u32_le(payload, 24) as usize;
    let military_count = read_u32_le(payload, 28) as usize;
    let naval_count = read_u32_le(payload, 32) as usize;
    let construction_count = read_u32_le(payload, 36) as usize;
    add_decoder_status(summary, status_flags);
    if scale_raw != COMPACT_PRODUCTION_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: production data scale {scale_raw} != expected {COMPACT_PRODUCTION_SCALE_RAW}"
        ));
    }
    let line_count = military_count.saturating_add(naval_count);
    let base_len = 40usize
        .saturating_add(stockpile_count.saturating_mul(16))
        .saturating_add(line_count.saturating_mul(32))
        .saturating_add(construction_count.saturating_mul(32));
    // Wave-4 production licensing: the base payload is self-delimiting to
    // base_len; any trailing bytes are the `PLIC` license trailer (parsed
    // after the base rows). A record with no trailer is exactly base_len.
    if payload.len() < base_len {
        summary.parse_errors.push(format!(
            "offset {offset}: production data payload is {} byte(s), expected at least {base_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: production data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&11) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 11, "production_country_rows", country_count);
    add_compact_domain_data_unit(summary, 11, "stockpile_rows", stockpile_count);
    add_compact_domain_data_unit(summary, 11, "military_line_rows", military_count);
    add_compact_domain_data_unit(summary, 11, "naval_line_rows", naval_count);
    add_compact_domain_data_unit(summary, 11, "construction_queue_rows", construction_count);
    let mut cursor = 40usize;
    let mut seen_stockpile = BTreeSet::new();
    for row_index in 0..stockpile_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let variant_id = read_u32_le(payload, cursor + 4);
        let _amount_raw = read_i64_le(payload, cursor + 8);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: production stockpile row {row_index} has zero tag_db_id"
            ));
        }
        if !seen_stockpile.insert((tag_db_id, variant_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: production stockpile row {row_index} duplicates tag_db_id {tag_db_id} variant_id {variant_id}"
            ));
        }
        cursor += 16;
    }
    let mut seen_lines = BTreeSet::new();
    for row_index in 0..line_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let line_id = read_i32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let _cost_raw = read_i32_le(payload, cursor + 16);
        let _speed_raw = read_i32_le(payload, cursor + 20);
        let _produced_raw = read_i32_le(payload, cursor + 24);
        let _amount = read_i32_le(payload, cursor + 28);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has zero tag_db_id"
            ));
        }
        if kind != 1 && kind != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has invalid kind {kind}"
            ));
        }
        if !(0..=100_000).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} has invalid active_factories {active_factories}"
            ));
        }
        if !seen_lines.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: production line row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
        cursor += 32;
    }
    let mut seen_construction = BTreeSet::new();
    for row_index in 0..construction_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let line_id = read_u32_le(payload, cursor + 4);
        let _template_id = read_u32_le(payload, cursor + 8);
        let active_factories = read_i32_le(payload, cursor + 12);
        let cost_raw = read_i32_le(payload, cursor + 16);
        let produced_raw = read_i32_le(payload, cursor + 20);
        let priority = read_u32_le(payload, cursor + 24);
        let reserved = read_u32_le(payload, cursor + 28);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has zero tag_db_id"
            ));
        }
        if !(0..=100_000).contains(&active_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has invalid active_factories {active_factories}"
            ));
        }
        if cost_raw < 0 || produced_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} has negative progress fields"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} reserved field is {reserved}"
            ));
        }
        if !seen_construction.insert((tag_db_id, line_id, priority)) {
            summary.parse_errors.push(format!(
                "offset {offset}: construction row {row_index} duplicates tag_db_id {tag_db_id} line_id {line_id} priority {priority}"
            ));
        }
        cursor += 32;
    }
    // Parse the strict additive D11 trailer chain (`PLIC`, X4-P1 `PVJ1`,
    // X4-P2 `PFR1`, exact-priority `PPR1`, XUI12 `PLS1`/`PEF1`, X4-P5
    // `PMI1`, completeness witnesses, and B2 `PCV1`).
    if cursor != payload.len() {
        decode_compact_production_trailers(
            offset,
            payload,
            cursor,
            country_count,
            &seen_stockpile,
            &seen_lines,
            summary,
        );
    }
}

fn decode_compact_production_trailers(
    offset: usize,
    payload: &[u8],
    mut trailer_offset: usize,
    country_count: usize,
    base_stockpile: &BTreeSet<(u32, u32)>,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) {
    let mut plic_seen = false;
    let mut pvj_seen = false;
    let mut pfr_seen = false;
    let mut ppr_seen = false;
    let mut pls_seen = false;
    let mut pef_seen = false;
    let mut pmi_seen = false;
    let mut pcp_seen = false;
    let mut pcv_seen = false;
    let mut pdq_seen = false;
    while trailer_offset < payload.len() {
        let Some(magic) = payload.get(trailer_offset..trailer_offset.saturating_add(4)) else {
            summary.parse_errors.push(format!(
                "offset {offset}: production trailer at {trailer_offset} has truncated magic"
            ));
            return;
        };
        let next = if magic == b"PLIC" {
            if plic_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PLIC trailer"
                ));
                return;
            }
            plic_seen = true;
            decode_compact_production_license_trailer(offset, payload, trailer_offset, summary)
        } else if magic == b"PVJ1" {
            if pvj_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PVJ1 trailer"
                ));
                return;
            }
            pvj_seen = true;
            decode_compact_production_variant_join_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PFR1" {
            if pfr_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PFR1 trailer"
                ));
                return;
            }
            pfr_seen = true;
            decode_compact_production_requested_factories_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PPR1" {
            if ppr_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PPR1 trailer"
                ));
                return;
            }
            ppr_seen = true;
            decode_compact_production_priority_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PLS1" {
            if pls_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PLS1 trailer"
                ));
                return;
            }
            pls_seen = true;
            decode_compact_production_line_subtype_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PEF1" {
            if pef_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PEF1 trailer"
                ));
                return;
            }
            pef_seen = true;
            decode_compact_production_efficiency_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PMI1" {
            if pmi_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PMI1 trailer"
                ));
                return;
            }
            pmi_seen = true;
            decode_compact_production_mio_join_trailer(
                offset,
                payload,
                trailer_offset,
                base_lines,
                summary,
            )
        } else if magic == b"PCP1" {
            if pcp_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PCP1 trailer"
                ));
                return;
            }
            pcp_seen = true;
            decode_compact_production_completeness_trailer(
                offset,
                payload,
                trailer_offset,
                country_count,
                base_stockpile,
                base_lines,
                summary,
            )
        } else if magic == b"PCV1" {
            if pcv_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PCV1 trailer"
                ));
                return;
            }
            pcv_seen = true;
            decode_compact_country_convoy_trailer(
                offset,
                payload,
                trailer_offset,
                country_count,
                summary,
            )
        } else if magic == b"PDQ1" {
            if pdq_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: production has duplicate PDQ1 trailer"
                ));
                return;
            }
            pdq_seen = true;
            decode_compact_deployment_queue_trailer(
                offset,
                payload,
                trailer_offset,
                country_count,
                summary,
            )
        } else {
            summary.parse_errors.push(format!(
                "offset {offset}: production trailing bytes are not a known D11 trailer"
            ));
            return;
        };
        let Some(next) = next else { return };
        if pdq_seen && next != payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: PDQ1 must be the final D11 trailer"
            ));
            return;
        }
        trailer_offset = next;
    }
}

fn decode_compact_deployment_queue_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_country_count: usize,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PDQ1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 trailer is truncated before its header"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let codec_payload_len = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let Some(expected) = HEADER_LEN.checked_add(codec_payload_len) else {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 segment size overflows usize"
        ));
        return None;
    };
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 header/count/length is invalid"
        ));
        return None;
    }
    let codec_payload = &trailer[HEADER_LEN..segment_len];
    let frame = match production11::decode_deployment_queue_frame(codec_payload) {
        Ok(frame) => frame,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: PDQ1 deployment carrier is invalid: {error}"
            ));
            return None;
        }
    };
    let codec_schema_version = read_u32_le(codec_payload, 20);
    if codec_schema_version != production11::deployment_queue::DEPLOYMENT_QUEUE_SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 requires deployment codec v{}, got v{codec_schema_version}",
            production11::deployment_queue::DEPLOYMENT_QUEUE_SCHEMA_VERSION
        ));
    }
    if frame.elapsed_wall_ms != read_u64_le(payload, 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 elapsed_wall_ms does not match D11"
        ));
    }
    if frame.countries.len() != base_country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 country rows {} != D11 country_count {base_country_count}",
            frame.countries.len()
        ));
    }
    if frame.changed_country_rows as usize != base_country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 changed_country_rows {} != D11 country_count {base_country_count}",
            frame.changed_country_rows
        ));
    }
    let base_changed_country_rows = read_u32_le(payload, 12);
    if frame.changed_country_rows != base_changed_country_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 changed_country_rows {} != D11 changed_rows {base_changed_country_rows}",
            frame.changed_country_rows
        ));
    }
    let base_status_flags = read_u32_le(payload, 16);
    if frame.status_flags != base_status_flags {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 status_flags {:#x} != D11 status_flags {base_status_flags:#x}",
            frame.status_flags
        ));
    }
    let unknown_status = compact_contract::compact_status_unknown_flags_v1(frame.status_flags);
    if unknown_status != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: PDQ1 has unknown status flags {unknown_status:#x}"
        ));
    }
    if compact_contract::compact_status_is_failure(frame.status_flags)
        && frame.countries.iter().any(|country| country.queue_complete)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: failed PDQ1 carrier contains complete country queues"
        ));
    }
    add_decoder_status(summary, frame.status_flags);
    let complete_country_rows = frame
        .countries
        .iter()
        .filter(|country| country.queue_complete)
        .count();
    let unknown_country_rows = frame.countries.len() - complete_country_rows;
    let conveyor_rows = frame
        .countries
        .iter()
        .map(|country| country.conveyors.len())
        .sum::<usize>();
    let line_rows = frame
        .countries
        .iter()
        .flat_map(|country| country.conveyors.iter())
        .map(|conveyor| conveyor.lines.len())
        .sum::<usize>();
    let equipment_rows = frame
        .countries
        .iter()
        .flat_map(|country| country.conveyors.iter())
        .flat_map(|conveyor| conveyor.lines.iter())
        .map(|line| line.equipment_in_transit.len())
        .sum::<usize>();
    add_compact_domain_data_unit(
        summary,
        11,
        "deployment_queue_country_rows",
        frame.countries.len(),
    );
    add_compact_domain_data_unit(
        summary,
        11,
        "deployment_queue_complete_country_rows",
        complete_country_rows,
    );
    add_compact_domain_data_unit(
        summary,
        11,
        "deployment_queue_unknown_country_rows",
        unknown_country_rows,
    );
    add_compact_domain_data_unit(summary, 11, "deployment_queue_conveyor_rows", conveyor_rows);
    add_compact_domain_data_unit(summary, 11, "deployment_queue_line_rows", line_rows);
    add_compact_domain_data_unit(summary, 11, "deployment_queue_equipment_rows", equipment_rows);
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_mio_join_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 16;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PMI1 trailer is truncated before its header"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.checked_add(row_count.checked_mul(ROW_LEN)?)?;
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PMI1 header/count/length is invalid"
        ));
        return None;
    }
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let cursor = HEADER_LEN + row_index * ROW_LEN;
        let key = (
            read_u32_le(trailer, cursor),
            read_u32_le(trailer, cursor + 4),
            read_i32_le(trailer, cursor + 8),
        );
        let _mio_id = read_u32_le(trailer, cursor + 12);
        if key.0 == 0 || (key.1 != 1 && key.1 != 2) || !base_lines.contains(&key) {
            summary.parse_errors.push(format!(
                "offset {offset}: PMI1 row {row_index} references an invalid or absent production line"
            ));
        }
        if !seen.insert(key) {
            summary.parse_errors.push(format!(
                "offset {offset}: PMI1 row {row_index} duplicates line key {:?}",
                key
            ));
        }
    }
    if &seen != base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PMI1 rows are not a bijection over the {} base production lines",
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(summary, 11, "production_mio_join_rows", row_count);
    Some(trailer_offset + segment_len)
}

fn decode_compact_country_convoy_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_country_count: usize,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 28;
    const COUNTRY_ROW_LEN: usize = 40;
    const VARIANT_ROW_LEN: usize = 16;
    const RESERVATION_ROW_LEN: usize = 8;
    const KNOWN_FLAGS: u32 = 0b1_1111;
    const SCALE: i64 = 100_000;
    const OWNED_RAW_CAP: i64 = 0x53E2_D620_FB40;
    let trailer = payload.get(trailer_offset..)?;
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PCV1 trailer is truncated before its header"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let country_count = read_u32_le(trailer, 12) as usize;
    let variant_count = read_u32_le(trailer, 16) as usize;
    let reservation_count = read_u32_le(trailer, 20) as usize;
    let reserved = read_u32_le(trailer, 24);
    let expected = HEADER_LEN
        .checked_add(country_count.checked_mul(COUNTRY_ROW_LEN)?)?
        .checked_add(variant_count.checked_mul(VARIANT_ROW_LEN)?)?
        .checked_add(reservation_count.checked_mul(RESERVATION_ROW_LEN)?)?;
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || country_count < base_country_count
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PCV1 header/count/length is invalid"
        ));
        return None;
    }
    let country_base = HEADER_LEN;
    let variant_base = country_base + country_count * COUNTRY_ROW_LEN;
    let reservation_base = variant_base + variant_count * VARIANT_ROW_LEN;
    let mut variants = Vec::with_capacity(variant_count);
    for row_index in 0..variant_count {
        let cursor = variant_base + row_index * VARIANT_ROW_LEN;
        let variant_id = read_u32_le(trailer, cursor);
        let equip_type = read_u32_le(trailer, cursor + 4);
        let amount_raw = read_i64_le(trailer, cursor + 8);
        if variant_id == 0 || variant_id >= (1 << 24) || equip_type != 70 || amount_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 variant row {row_index} has invalid id/type/amount"
            ));
        }
        variants.push((variant_id, amount_raw));
    }
    let mut reservations = Vec::with_capacity(reservation_count);
    for row_index in 0..reservation_count {
        let cursor = reservation_base + row_index * RESERVATION_ROW_LEN;
        let priority = read_i32_le(trailer, cursor);
        let count = read_i32_le(trailer, cursor + 4);
        if count < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 reservation row {row_index} has negative count"
            ));
        }
        reservations.push((priority, count));
    }

    let mut tags = BTreeSet::new();
    let mut expected_variant_start = 0usize;
    let mut expected_reservation_start = 0usize;
    for row_index in 0..country_count {
        let cursor = country_base + row_index * COUNTRY_ROW_LEN;
        let tag_db_id = read_u32_le(trailer, cursor);
        let flags = read_u32_le(trailer, cursor + 4);
        let owned_raw = read_i64_le(trailer, cursor + 8);
        let assigned = read_i32_le(trailer, cursor + 16);
        let available = read_i32_le(trailer, cursor + 20);
        let variant_start = read_u32_le(trailer, cursor + 24) as usize;
        let variant_len = read_u32_le(trailer, cursor + 28) as usize;
        let reservation_start = read_u32_le(trailer, cursor + 32) as usize;
        let reservation_len = read_u32_le(trailer, cursor + 36) as usize;
        let variant_end = variant_start.checked_add(variant_len);
        let reservation_end = reservation_start.checked_add(reservation_len);
        if tag_db_id == 0 || !tags.insert(tag_db_id) || flags & !KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 country row {row_index} has invalid tag/flags"
            ));
        }
        if variant_start != expected_variant_start
            || reservation_start != expected_reservation_start
            || variant_end.is_none_or(|end| end > variant_count)
            || reservation_end.is_none_or(|end| end > reservation_count)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 country row {row_index} has noncanonical ranges"
            ));
            return None;
        }
        expected_variant_start += variant_len;
        expected_reservation_start += reservation_len;
        let complete = flags & 1 != 0;
        let reservations_complete = flags & (1 << 1) != 0;
        let cache_valid = flags & (1 << 2) != 0;
        let cache_verified = flags & (1 << 3) != 0;
        let classifier_exact = flags & (1 << 4) != 0;
        if !complete {
            if flags != 0
                || owned_raw != 0
                || assigned != 0
                || available != 0
                || variant_len != 0
                || reservation_len != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: PCV1 unreadable country row {row_index} carries data"
                ));
            }
            continue;
        }
        if !classifier_exact
            || cache_valid != cache_verified
            || (!reservations_complete && reservation_len != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 country row {row_index} has inconsistent witnesses"
            ));
        }
        let Some(variant_end) = variant_end else { return None };
        let Some(reservation_end) = reservation_end else {
            return None;
        };
        let mut seen_variants = BTreeSet::new();
        let sum_raw = variants[variant_start..variant_end]
            .iter()
            .try_fold(0i64, |sum, (variant_id, amount_raw)| {
                if !seen_variants.insert(*variant_id) {
                    return None;
                }
                Some(sum.saturating_add(*amount_raw).min(OWNED_RAW_CAP))
            });
        if sum_raw != Some(owned_raw)
            || assigned < 0
            || i64::from(assigned) > owned_raw / SCALE
            || i64::from(available) != owned_raw / SCALE - i64::from(assigned)
            || reservations[reservation_start..reservation_end]
                .iter()
                .any(|(_, count)| *count < 0 || i64::from(*count) > owned_raw / SCALE)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: PCV1 country row {row_index} fails the exact total law"
            ));
        }
    }
    if expected_variant_start != variant_count || expected_reservation_start != reservation_count {
        summary.parse_errors.push(format!(
            "offset {offset}: PCV1 country ranges do not cover all rows"
        ));
    }
    add_compact_domain_data_unit(summary, 11, "country_convoy_rows", country_count);
    add_compact_domain_data_unit(summary, 11, "country_convoy_variant_rows", variant_count);
    add_compact_domain_data_unit(
        summary,
        11,
        "country_convoy_reservation_rows",
        reservation_count,
    );
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_completeness_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    country_count: usize,
    base_stockpile: &BTreeSet<(u32, u32)>,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const KNOWN_FLAGS: u32 = 0b111;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PCP1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PCP1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(8));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || row_count != country_count
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PCP1 header/count/length is invalid"
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut flags_by_tag = BTreeMap::new();
    let mut complete_counts = [0usize; 3];
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let flags = read_u32_le(trailer, cursor + 4);
        cursor += 8;
        if tag_db_id == 0 || flags_by_tag.insert(tag_db_id, flags).is_some() {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 row {row_index} has invalid/duplicate tag_db_id {tag_db_id}"
            ));
        }
        if flags & !KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 row {row_index} has unknown flags {flags:#x}"
            ));
        }
        for (bit, count) in complete_counts.iter_mut().enumerate() {
            if flags & (1 << bit) != 0 {
                *count += 1;
            }
        }
    }
    for &(tag_db_id, _) in base_stockpile {
        if flags_by_tag.get(&tag_db_id).copied().unwrap_or(0) & 1 == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 marks stockpile unreadable for tag {tag_db_id} with base rows"
            ));
        }
    }
    for &(tag_db_id, kind, _) in base_lines {
        let required = if kind == 2 { 1 << 2 } else { 1 << 1 };
        if flags_by_tag.get(&tag_db_id).copied().unwrap_or(0) & required == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PCP1 marks production line kind {kind} unreadable for tag {tag_db_id} with base rows"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 11, "production_completeness_rows", row_count);
    add_compact_domain_data_unit(summary, 11, "stockpile_complete_rows", complete_counts[0]);
    add_compact_domain_data_unit(
        summary,
        11,
        "military_lines_complete_rows",
        complete_counts[1],
    );
    add_compact_domain_data_unit(summary, 11, "naval_lines_complete_rows", complete_counts[2]);
    Some(trailer_offset + segment_len)
}

/// Decode + gate the domain-11 `PLIC` production-license trailer. The trailer
/// must consume the payload to its exact end. License db ids are gated
/// 1..2^24 and per-country id sets are deduped.
fn decode_compact_production_license_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const MAGIC: &[u8; 4] = b"PLIC";
    const HEADER_LEN: usize = 20;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary.parse_errors.push(format!(
            "offset {offset}: production license trailer offset {trailer_offset} beyond payload"
        ));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: production license trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    if &trailer[0..4] != MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: production license trailer magic is invalid"
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let country_rows = read_u32_le(trailer, 8) as usize;
    let id_rows = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    if (version != 1 && version != 2) || header_len != HEADER_LEN || reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: production license trailer header shape invalid (version {version}, header_len {header_len}, reserved {reserved})"
        ));
        return None;
    }
    let item_len = if version == 1 { 4 } else { 16 };
    let expected = HEADER_LEN
        .saturating_add(country_rows.saturating_mul(8))
        .saturating_add(id_rows.saturating_mul(item_len));
    if trailer.len() < expected {
        summary.parse_errors.push(format!(
            "offset {offset}: production license trailer len {} < {expected}",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen_country = BTreeSet::new();
    let mut counts = Vec::with_capacity(country_rows);
    let mut total_ids = 0usize;
    let mut empty_country_rows = 0usize;
    for row_index in 0..country_rows {
        let tag_db_id = read_u32_le(trailer, cursor);
        let count = read_u32_le(trailer, cursor + 4) as usize;
        cursor += 8;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: production license country row {row_index} has zero tag_db_id"
            ));
        }
        if !seen_country.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: production license country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if count == 0 {
            empty_country_rows = empty_country_rows.saturating_add(1);
        }
        total_ids = total_ids.saturating_add(count);
        counts.push((tag_db_id, count));
    }
    if total_ids != id_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: production license id total {total_ids} != id_rows {id_rows}"
        ));
        return None;
    }
    for (tag_db_id, count) in &counts {
        let mut seen_ids = BTreeSet::new();
        for expected_order in 0..*count {
            let variant_id = read_u32_le(trailer, cursor);
            let (visible_order, creator_tag_db_id, origin_tag_db_id) = if version == 2 {
                (
                    read_u32_le(trailer, cursor + 4),
                    read_u32_le(trailer, cursor + 8),
                    read_u32_le(trailer, cursor + 12),
                )
            } else {
                (expected_order as u32, 0, 0)
            };
            cursor += item_len;
            if variant_id == 0 || variant_id >= (1 << 24) {
                summary.parse_errors.push(format!(
                    "offset {offset}: production license id {variant_id} for tag {tag_db_id} out of range"
                ));
            }
            if !seen_ids.insert(variant_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: production license tag {tag_db_id} duplicates variant_id {variant_id}"
                ));
            }
            if version == 2
                // `> i32::MAX` is one short of the id rule (it admits the size
                // ceiling, which no database can index).
                && (visible_order != expected_order as u32
                    || creator_tag_db_id == 0
                    || creator_tag_db_id > COMPACT_MAX_OWNER_TAG_DB_ID
                    || origin_tag_db_id > COMPACT_MAX_OWNER_TAG_DB_ID)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: production license v2 tag {tag_db_id} row {expected_order} has invalid order/creator/origin ({visible_order}, {creator_tag_db_id}, {origin_tag_db_id})"
                ));
            }
        }
    }
    add_compact_domain_data_unit(summary, 11, "production_license_country_rows", country_rows);
    add_compact_domain_data_unit(summary, 11, "production_license_id_rows", id_rows);
    add_compact_domain_data_unit(
        summary,
        11,
        "production_license_empty_country_rows",
        empty_country_rows,
    );
    Some(trailer_offset + expected)
}

fn decode_compact_production_variant_join_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PVJ1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PVJ1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(16));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PVJ1 header shape invalid (version {version}, header_len {header_len}, segment_len {segment_len}, expected {expected}, reserved {reserved}, available {})",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let variant_id = read_u32_le(trailer, cursor + 12);
        cursor += 16;
        let key = (tag_db_id, kind, line_id);
        if tag_db_id == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PVJ1 row {row_index} has invalid tag/kind ({tag_db_id}, {kind})"
            ));
        }
        if variant_id == 0 || variant_id > i32::MAX as u32 {
            summary.parse_errors.push(format!(
                "offset {offset}: PVJ1 row {row_index} has invalid equipment_variant_id {variant_id}"
            ));
        }
        if !seen.insert(key) {
            summary.parse_errors.push(format!(
                "offset {offset}: PVJ1 row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PVJ1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(summary, 11, "production_variant_join_rows", row_count);
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_requested_factories_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PFR1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PFR1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(16));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PFR1 header shape invalid (version {version}, header_len {header_len}, segment_len {segment_len}, expected {expected}, reserved {reserved}, available {})",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let requested_factories = read_i32_le(trailer, cursor + 12);
        cursor += 16;
        let key = (tag_db_id, kind, line_id);
        if tag_db_id == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PFR1 row {row_index} has invalid tag/kind ({tag_db_id}, {kind})"
            ));
        }
        if !(0..=100_000).contains(&requested_factories) {
            summary.parse_errors.push(format!(
                "offset {offset}: PFR1 row {row_index} has invalid requested_factories {requested_factories}"
            ));
        }
        if !seen.insert(key) {
            summary.parse_errors.push(format!(
                "offset {offset}: PFR1 row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PFR1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(
        summary,
        11,
        "production_requested_factories_rows",
        row_count,
    );
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_priority_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 16;
    const MAX_PRIORITY: i32 = 4095;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PPR1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PPR1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PPR1 header shape invalid (version {version}, header_len {header_len}, segment_len {segment_len}, expected {expected}, reserved {reserved}, available {})",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let priority = read_i32_le(trailer, cursor + 12);
        cursor += ROW_LEN;
        if tag_db_id == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PPR1 row {row_index} has invalid tag/kind ({tag_db_id}, {kind})"
            ));
        }
        if !(0..=MAX_PRIORITY).contains(&priority) {
            summary.parse_errors.push(format!(
                "offset {offset}: PPR1 row {row_index} has invalid priority {priority}"
            ));
        }
        if !seen.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PPR1 row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PPR1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(summary, 11, "production_priority_rows", row_count);
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_line_subtype_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 16;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PLS1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PLS1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PLS1 header shape invalid (version {version}, header_len {header_len}, segment_len {segment_len}, expected {expected}, reserved {reserved}, available {})",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut subtype_counts = [0usize; 4];
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let subtype = read_u32_le(trailer, cursor + 12);
        cursor += ROW_LEN;
        let coherent = matches!((kind, subtype), (1, 1) | (2, 2) | (2, 3) | (1, 4));
        if tag_db_id == 0 || !coherent {
            summary.parse_errors.push(format!(
                "offset {offset}: PLS1 row {row_index} has invalid tag/kind/subtype ({tag_db_id}, {kind}, {subtype})"
            ));
        } else {
            subtype_counts[subtype as usize - 1] =
                subtype_counts[subtype as usize - 1].saturating_add(1);
        }
        if !seen.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PLS1 row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PLS1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(summary, 11, "production_line_subtype_rows", row_count);
    for (name, count) in [
        ("production_military_subtype_rows", subtype_counts[0]),
        ("production_naval_subtype_rows", subtype_counts[1]),
        ("production_ship_refit_subtype_rows", subtype_counts[2]),
        ("production_railway_gun_subtype_rows", subtype_counts[3]),
    ] {
        add_compact_domain_data_unit(summary, 11, name, count);
    }
    Some(trailer_offset + segment_len)
}

fn decode_compact_production_efficiency_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    base_lines: &BTreeSet<(u32, u32, i32)>,
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 36;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: PEF1 offset beyond payload"));
        return None;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: PEF1 trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return None;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let segment_len = read_u32_le(trailer, 8) as usize;
    let row_count = read_u32_le(trailer, 12) as usize;
    let reserved = read_u32_le(trailer, 16);
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if version != 1
        || header_len != HEADER_LEN
        || segment_len != expected
        || reserved != 0
        || trailer.len() < segment_len
    {
        summary.parse_errors.push(format!(
            "offset {offset}: PEF1 header shape invalid (version {version}, header_len {header_len}, segment_len {segment_len}, expected {expected}, reserved {reserved}, available {})",
            trailer.len()
        ));
        return None;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let kind = read_u32_le(trailer, cursor + 4);
        let line_id = read_i32_le(trailer, cursor + 8);
        let efficiency_raw = read_i64_le(trailer, cursor + 12);
        let efficiency_cap_raw = read_i64_le(trailer, cursor + 20);
        let efficiency_growth_raw = read_i64_le(trailer, cursor + 28);
        cursor += ROW_LEN;
        if tag_db_id == 0 || (kind != 1 && kind != 2) {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} has invalid tag/kind ({tag_db_id}, {kind})"
            ));
        }
        if efficiency_raw < 0 || efficiency_cap_raw < 0 || efficiency_growth_raw < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} has negative efficiency values ({efficiency_raw}, {efficiency_cap_raw}, {efficiency_growth_raw})"
            ));
        }
        if !seen.insert((tag_db_id, kind, line_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: PEF1 row {row_index} duplicates tag_db_id {tag_db_id} kind {kind} line_id {line_id}"
            ));
        }
    }
    if seen != *base_lines {
        summary.parse_errors.push(format!(
            "offset {offset}: PEF1 line coverage is not bijective: trailer_rows={} base_lines={}",
            seen.len(),
            base_lines.len()
        ));
    }
    add_compact_domain_data_unit(summary, 11, "production_efficiency_rows", row_count);
    Some(trailer_offset + segment_len)
}

fn decode_compact_focus_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 28 {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data payload is {} byte(s), expected at least 28",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let completed_token_count = read_u32_le(payload, 24) as usize;
    add_decoder_status(summary, status_flags);
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&12) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 12, "focus_country_rows", country_count);
    add_compact_domain_data_unit(summary, 12, "completed_focus_tokens", completed_token_count);
    let mut cursor = 28usize;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 16, "focus country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let completed_count = read_u32_le(payload, cursor + 8);
        let progress_raw = read_i32_le(payload, cursor + 12);
        cursor += 16;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        // Bits 6/7 carry the authenticated pause-field presence/value added to
        // the current D12 producer. WorldFold already consumes the same full
        // byte; the summary gate must not reject valid current-schema rows.
        if flags & !0xff != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has unknown flags {flags}"
            ));
        }
        if completed_count != u32::MAX && completed_count > 4096 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has invalid completed_count {completed_count}"
            ));
        }
        if flags & (1 << 4) == 0 && progress_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus country row {row_index} has progress without flag"
            ));
        }
        for field in ["focus tree", "focus current", "focus current continuous"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..completed_token_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "focus completed row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus completed row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "focus completed token") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: focus data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_tech_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    // The structural walk is `d13_grammar`'s, shared with the producer, the fold
    // and the wire port. This decoder's own half is unchanged: it ACCUMULATES
    // diagnostics where the fold returns, and which violations abandon the walk
    // is decided here, at each step.
    //
    // That boundary matters in one direction only: `stream_fold` consults this
    // decoder before applying a payload at all, so a row refused here never
    // reaches `WorldFold`, and a rule that is merely STRICTER here silently
    // deletes the domain.
    let header = match d13_grammar::read_header(payload) {
        Ok(header) => header,
        Err(_) => {
            summary.parse_errors.push(format!(
                "offset {offset}: tech data payload is {} byte(s), expected at least {}",
                payload.len(),
                d13_grammar::HEADER_LEN
            ));
            return;
        }
    };
    let country_count = header.country_count as usize;
    let changed_rows = header.changed_rows as usize;
    let status_flags = header.status_flags;
    let active_count = header.active_count as usize;
    add_decoder_status(summary, status_flags);
    if header.progress_scale_raw != COMPACT_TECH_PROGRESS_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: tech progress scale {} != expected {COMPACT_TECH_PROGRESS_SCALE_RAW}",
            header.progress_scale_raw
        ));
    }
    if header.live_scale_raw != COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: tech live scale {} != expected {COMPACT_TECH_LIVE_PROGRESS_SCALE_RAW}",
            header.live_scale_raw
        ));
    }
    let layout = match d13_grammar::body_layout(payload, &header) {
        Ok(layout) => layout,
        Err(violation) => {
            let minimum_len = match violation.kind {
                d13_grammar::ViolationKind::Need { len, .. } => len,
                _ => payload.len(),
            };
            summary.parse_errors.push(format!(
                "offset {offset}: tech data payload is {} byte(s), expected at least {minimum_len}",
                payload.len()
            ));
            return;
        }
    };
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: tech data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&13) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 13, "tech_country_rows", country_count);
    add_compact_domain_data_unit(summary, 13, "active_research_slot_rows", active_count);
    let mut cursor = layout.countries.start;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        let row = match d13_grammar::read_country_row(payload, cursor) {
            Ok(row) => row,
            Err(_) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: tech country row {row_index} is truncated"
                ));
                return;
            }
        };
        if row.tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(row.tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} duplicates tag_db_id {}",
                row.tag_db_id
            ));
        }
        if row.unknown_flags() != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has unknown flags {}",
                row.flags
            ));
        }
        if row.research_slot_count != u32::MAX && row.research_slot_count > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech country row {row_index} has invalid slot_count {}",
                row.research_slot_count
            ));
        }
        cursor = row.span.end;
    }
    let mut seen_slots = BTreeSet::new();
    for row_index in 0..active_count {
        let (mut row, unknown_flags) = match d13_grammar::ActiveRowCursor::begin(payload, cursor) {
            Ok(begun) => begun,
            Err(_) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: tech active row {row_index} base record is truncated"
                ));
                return;
            }
        };
        let fixed = row.fixed().clone();
        let tag_db_id = fixed.tag_db_id;
        let slot_index = fixed.slot_index;
        let tech_def_index_raw = fixed.tech_def_index_raw;
        let live_progress_raw = fixed.live_progress_word;
        let base_required_points_raw = fixed.base_required_points_word;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has zero tag_db_id"
            ));
        }
        if slot_index > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        // Recorded and walked past: the known bits still describe a walkable row,
        // and refusing where the fold does not would delete the domain.
        if let Some(violation) = unknown_flags {
            summary
                .parse_errors
                .push(tech_authority_message(offset, row_index, &violation));
        }
        if !seen_slots.insert((tag_db_id, slot_index)) {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} duplicates tag_db_id {tag_db_id} slot_index {slot_index}"
            ));
        }
        if !fixed.live_progress_captured() && live_progress_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has live_progress_raw without presence flag"
            ));
        }
        if !fixed.base_required_points_captured() && base_required_points_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech active row {row_index} has base_required_points_raw without presence flag"
            ));
        }
        let mut ahead_quartet = None;
        let row_end = loop {
            let step = match row.next_step() {
                Ok(step) => step,
                // Terminal: the bytes that would find the next boundary are not
                // there, so this decoder abandons the payload exactly as it
                // always has for a truncated row.
                Err(violation) => {
                    summary
                        .parse_errors
                        .push(tech_authority_message(offset, row_index, &violation));
                    return;
                }
            };
            // No wildcard arm: a tail added to the grammar is a compile error
            // here until this decoder decides what it means.
            match step {
                d13_grammar::ActiveStep::Section(section) => match section.tail {
                    d13_grammar::ActiveTail::TechName => {
                        if section.span.is_empty() {
                            summary.parse_errors.push(format!(
                                "offset {offset}: tech active row {row_index} has empty tech_name"
                            ));
                        }
                    }
                    d13_grammar::ActiveTail::AheadQuartet => {
                        let view = match d13_grammar::AheadQuartetView::new(payload, section.span) {
                            Some(view) => view,
                            None => {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: tech active row {row_index} ahead quartet is truncated"
                                ));
                                return;
                            }
                        };
                        let values = view.tuple();
                        ahead_quartet = Some(values);
                        if [values.0, values.1, values.2, values.3]
                            .iter()
                            .any(|value| !(0..=10_000_000_000i64).contains(value))
                        {
                            summary.parse_errors.push(format!(
                                "offset {offset}: tech active row {row_index} has invalid ahead fixed-5 quartet"
                            ));
                        }
                        add_compact_domain_data_unit(summary, 13, "effective_ahead_rows", 1);
                    }
                    d13_grammar::ActiveTail::ObservedAuthority => {
                        let view = match d13_grammar::A29ProvenanceView::new(payload, section.span) {
                            Some(view) => view,
                            None => {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: tech active row {row_index} A29 provenance is truncated"
                                ));
                                return;
                            }
                        };
                        let authority = view.authority();
                        let branch = view.localization_branch();
                        let callback_sequence = view.callback_sequence();
                        let callback_thread_id = view.callback_thread_id();
                        let producer_game_hour_raw = view.producer_game_hour_raw();
                        let callback_qpc_ticks = view.callback_qpc_ticks();
                        let qpc_frequency_hz = view.qpc_frequency_hz();
                        let source_sha = view.source_record_sha256();
                        let ui_action_gate_return_rva = view.ui_action_gate_return_rva();
                        let candidate_count = view.selected_present_candidate_count();
                        let selected_ordinal = view.selected_present_ordinal();
                        let selected_game_hour_raw = view.selected_present_game_hour_raw();
                        let selected_candidate_id = view.selected_present_candidate_id();
                        let selected_qpc_ticks = view.selected_present_qpc_ticks();
                        let selection_cutoff_qpc_ticks = view.selected_present_cutoff_qpc_ticks();
                        let candidate_set_sha256: [u8; 32] = view
                            .selected_present_candidate_set_sha256()
                            .try_into()
                            .unwrap();
                        let selected_present_receipt_sha256: [u8; 32] =
                            view.selected_present_receipt_sha256().try_into().unwrap();
                        let expected_source_sha = ahead_quartet.map(|quartet| {
                            a29_expected_source_record_sha256(
                                callback_sequence,
                                callback_thread_id,
                                callback_qpc_ticks,
                                qpc_frequency_hz,
                                producer_game_hour_raw,
                                slot_index,
                                tech_def_index_raw,
                                branch,
                                quartet,
                                candidate_count,
                                selected_candidate_id,
                                selected_ordinal,
                                selected_game_hour_raw,
                                selected_qpc_ticks,
                                selection_cutoff_qpc_ticks,
                                &candidate_set_sha256,
                                &selected_present_receipt_sha256,
                            )
                        });
                        let computed_selected_present_sha256 =
                            portable_render_stream::SelectedPresentReceipt::sealed(
                                candidate_count,
                                snapshot::sha256::to_hex(&candidate_set_sha256),
                                selected_candidate_id,
                                selected_ordinal,
                                selected_game_hour_raw,
                                selected_qpc_ticks,
                                selection_cutoff_qpc_ticks,
                            )
                            .computed_sha256()
                            .ok();
                        if authority != 3
                            || !matches!(branch, 0 | 1 | 2)
                            || match (branch, ahead_quartet) {
                                (0, Some((years, penalty, reduction, reduced_penalty))) => {
                                    years != 0
                                        || penalty != 0
                                        || reduction < 0
                                        || reduced_penalty != 0
                                }
                                (1, Some((_, penalty, reduction, reduced_penalty))) => {
                                    reduction != 0 || reduced_penalty != penalty
                                }
                                (2, Some((_, _, reduction, _))) => reduction <= 0,
                                _ => true,
                            }
                            || callback_sequence == 0
                            || callback_thread_id == 0
                            || producer_game_hour_raw <= 0
                            || callback_qpc_ticks == 0
                            || qpc_frequency_hz == 0
                            || ui_action_gate_return_rva != A29_UI_YEAR_HELPER_RETURN_RVA as u32
                            || candidate_count == 0
                            || selected_ordinal >= candidate_count
                            || selected_game_hour_raw != producer_game_hour_raw
                            || callback_qpc_ticks > selected_qpc_ticks
                            || selection_cutoff_qpc_ticks < selected_qpc_ticks
                            || candidate_set_sha256.iter().all(|byte| *byte == 0)
                            || selected_present_receipt_sha256.iter().all(|byte| *byte == 0)
                            || computed_selected_present_sha256.as_deref()
                                != Some(
                                    snapshot::sha256::to_hex(&selected_present_receipt_sha256)
                                        .as_str(),
                                )
                            || source_sha.iter().all(|byte| *byte == 0)
                            || expected_source_sha
                                .is_none_or(|expected| source_sha != expected.as_slice())
                        {
                            summary.parse_errors.push(format!(
                                "offset {offset}: tech active row {row_index} A29 provenance is invalid"
                            ));
                        }
                        add_compact_domain_data_unit(
                            summary,
                            13,
                            "observed_ahead_provenance_rows",
                            1,
                        );
                    }
                    // The DERIVED authority: the recorder's own inputs in place
                    // of a producer receipt. This decoder sees a payload without
                    // its record's hour, so the SAME-HOUR gate stays where the
                    // frame hour is known (`world_fold.rs::fold_tech`); what is
                    // checked here is everything that is decidable from the bytes
                    // alone, and nothing beyond it.
                    d13_grammar::ActiveTail::DerivedAuthority => {
                        let view =
                            match d13_grammar::DerivedAuthorityView::new(payload, section.span) {
                                Some(view) => view,
                                None => {
                                    summary.parse_errors.push(format!(
                                        "offset {offset}: tech active row {row_index} is too short for the A29 derived authority"
                                    ));
                                    return;
                                }
                            };
                        if view.authority() != 1
                            || view.producer_game_hour_raw() <= 0
                            || !(0..=10_000_000_000i64).contains(&view.penalty_factor_raw())
                        {
                            summary.parse_errors.push(format!(
                                "offset {offset}: tech active row {row_index} A29 derived authority is invalid"
                            ));
                        }
                        add_compact_domain_data_unit(summary, 13, "derived_ahead_authority_rows", 1);
                    }
                },
                d13_grammar::ActiveStep::Recoverable(violation) => {
                    summary
                        .parse_errors
                        .push(tech_authority_message(offset, row_index, &violation));
                    // One recoverable rule abandons the payload here and always
                    // has: a receipt for a quartet that is not there means the
                    // row's remaining bytes cannot be trusted to be what their
                    // flags claim. The other three are recorded and walked past.
                    if matches!(
                        violation.kind,
                        d13_grammar::ViolationKind::Rule {
                            rule: d13_grammar::RuleViolation::ObservedAuthorityWithoutQuartet,
                            ..
                        }
                    ) {
                        return;
                    }
                }
                d13_grammar::ActiveStep::End(end) => break end,
            }
        };
        cursor = row_end;
    }
    // D13's XUI09 witness tail (G4-X9S1, landed by lane JO). This decoder
    // authenticates the A29 provenance and the row structure; the FOLD is the
    // authority on the witness itself. But the exact-consumption check below
    // cannot tell a tail from corruption, so the tail's length has to be
    // accounted for here or every record carrying one is refused. On the t53
    // live capture that is precisely what happened: 59 D13 records excluded,
    // "trailing byte(s)" counts of 923752 (whole payload) and 2244 (patch),
    // both exactly the tail's own length.
    match crate::world_fold::compact_tech_xui09_tail_len(payload, cursor) {
        Ok(len) => cursor += len,
        Err(error) => summary
            .parse_errors
            .push(format!("offset {offset}: {error}")),
    }
    if let Err(violation) = d13_grammar::exact_consumption(cursor, payload.len()) {
        if let d13_grammar::ViolationKind::TrailingBytes { consumed, total } = violation.kind {
            summary.parse_errors.push(format!(
                "offset {offset}: tech data payload has {} trailing byte(s)",
                total.saturating_sub(consumed)
            ));
        }
    }
}

/// One structural violation, spelled the way THIS decoder has always spelled it:
/// with the record's stream offset and the row's index, because its diagnostics
/// become a sealed manifest's exclusion reasons and a reader needs to find the
/// record they name.
///
/// Exhaustive, no wildcard arm: a new violation kind is a compile error here
/// rather than a silently reworded exclusion.
fn tech_authority_message(
    offset: usize,
    row_index: usize,
    violation: &d13_grammar::Violation,
) -> String {
    use d13_grammar::{FlagWord, RuleViolation, Section, ViolationKind};
    match violation.kind {
        ViolationKind::Need {
            section,
            at,
            len,
            available,
            ..
        } => match section {
            Section::ActiveFixed => format!(
                "offset {offset}: tech active row {row_index} base record is truncated"
            ),
            // `skip_compact_string_len`'s wording, which this decoder used for
            // the name before the walk was shared.
            Section::TechNamePrefix | Section::TechNameBytes => format!(
                "offset {offset}: tech active tech_name overruns payload at cursor {at}: need {len}, remaining {}",
                available.saturating_sub(at)
            ),
            Section::AheadQuartet => format!(
                "offset {offset}: tech active row {row_index} ahead quartet is truncated"
            ),
            Section::ObservedAuthority => format!(
                "offset {offset}: tech active row {row_index} A29 provenance is truncated"
            ),
            Section::DerivedAuthority => format!(
                "offset {offset}: tech active row {row_index} is too short for the A29 derived authority"
            ),
            Section::Header
            | Section::PayloadMinimum
            | Section::CountryTable
            | Section::CountryRow
            | Section::Xui09TailHeader
            | Section::Xui09DictionaryIndex
            | Section::Xui09DictionaryEntry
            | Section::Xui09DictionaryIdPrefix
            | Section::Xui09DictionaryIdBytes
            | Section::Xui09MeasuredTag
            | Section::Xui09Refusal
            | Section::Xui09MeasuredAndRefusalSets
            | Section::Xui09VectorHeader
            | Section::Xui09VectorRows => format!(
                "offset {offset}: tech data payload is truncated at cursor {at}: need {len}, remaining {}",
                available.saturating_sub(at)
            ),
        },
        ViolationKind::LengthOverflow { .. } | ViolationKind::OffsetOverflow { .. } => {
            format!("offset {offset}: tech active row {row_index} length overflows")
        }
        ViolationKind::BadMagic { found, expected } => format!(
            "offset {offset}: tech XUI09 tail magic {found:#010x} is not {expected:#010x}"
        ),
        ViolationKind::UnknownFlags { word, value, .. } => match word {
            FlagWord::ActiveRow => format!(
                "offset {offset}: tech active row {row_index} has unknown flags {value}"
            ),
            FlagWord::CountryRow => format!(
                "offset {offset}: tech country row {row_index} has unknown flags {value}"
            ),
            FlagWord::Xui09Tail | FlagWord::Xui09CountryVector => format!(
                "offset {offset}: tech XUI09 flags {value} are not all assigned"
            ),
        },
        ViolationKind::Rule { rule, .. } => match rule {
            RuleViolation::ObservedAuthorityWithoutQuartet => format!(
                "offset {offset}: tech active row {row_index} has A29 provenance without an ahead quartet"
            ),
            RuleViolation::DerivedAuthorityWithoutQuartet => format!(
                "offset {offset}: tech active row {row_index} A29 derived authority without ahead quartet"
            ),
            RuleViolation::BothAuthorities => format!(
                "offset {offset}: tech active row {row_index} carries both observed and derived A29 authority"
            ),
            RuleViolation::QuartetWithoutAuthority => format!(
                "offset {offset}: tech active row {row_index} has an unauthenticated ahead quartet"
            ),
        },
        ViolationKind::TrailingBytes { consumed, total } => format!(
            "offset {offset}: tech data payload has {} trailing byte(s)",
            total.saturating_sub(consumed)
        ),
        ViolationKind::WalkOutOfOrder => {
            format!("offset {offset}: tech payload walk driven out of order")
        }
    }
}

fn decode_compact_politics_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let popularity_count = read_u32_le(payload, 24) as usize;
    let law_count = read_u32_le(payload, 28) as usize;
    let advisor_count = read_u32_le(payload, 32) as usize;
    let national_spirit_count = read_u32_le(payload, 36) as usize;
    add_decoder_status(summary, status_flags);
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&14) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 14, "politics_country_rows", country_count);
    add_compact_domain_data_unit(summary, 14, "popularity_rows", popularity_count);
    add_compact_domain_data_unit(summary, 14, "law_rows", law_count);
    add_compact_domain_data_unit(summary, 14, "advisor_rows", advisor_count);
    add_compact_domain_data_unit(summary, 14, "national_spirit_rows", national_spirit_count);
    let mut cursor = 40usize;
    let mut seen_countries = BTreeSet::new();
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 28, "politics country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let idea_count = read_u32_le(payload, cursor + 8);
        let election_frequency = read_u32_le(payload, cursor + 12);
        let party_count = read_u32_le(payload, cursor + 16);
        let low_confidence = read_u32_le(payload, cursor + 20);
        let _leader_char_id = read_u32_le(payload, cursor + 24);
        cursor += 28;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11_1111_1111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has unknown flags {flags}"
            ));
        }
        if idea_count != u32::MAX && idea_count > 4096 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid idea_count {idea_count}"
            ));
        }
        if election_frequency != u32::MAX && !(1..=600).contains(&election_frequency) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid election_frequency {election_frequency}"
            ));
        }
        if party_count != u32::MAX && party_count > 128 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid party_count {party_count}"
            ));
        }
        if low_confidence != u32::MAX && low_confidence > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics country row {row_index} has invalid low_confidence {low_confidence}"
            ));
        }
        for field in [
            "politics ruling party",
            "politics leader name",
            "politics leader token",
            "politics leader ideology",
        ] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..popularity_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics popularity row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics popularity row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics popularity ideology")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics popularity row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let raw = read_i32_le(payload, cursor);
        if !(0..=(COMPACT_FIXED_SCALE_RAW as i32 * 100)).contains(&raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics popularity row {row_index} has invalid raw value {raw}"
            ));
        }
        cursor += 4;
    }
    for row_index in 0..law_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics law row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics law row {row_index} has zero tag_db_id"
            ));
        }
        for field in ["politics law group", "politics law token"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..advisor_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 4, "politics advisor row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics advisor row {row_index} has zero tag_db_id"
            ));
        }
        for field in ["politics advisor group", "politics advisor token"] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    for row_index in 0..national_spirit_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 4, "politics national spirit row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics national spirit row {row_index} has zero tag_db_id"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics national spirit") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: politics data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_politics_residuals_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let advisor_detail_count = read_u32_le(payload, 20) as usize;
    add_decoder_status(summary, status_flags);
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&15) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 15, "politics_residual_country_rows", country_count);
    add_compact_domain_data_unit(summary, 15, "advisor_detail_rows", advisor_detail_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_advisors = 0usize;
    for row_index in 0..country_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 16, "politics residuals country row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let elections_allowed = read_u32_le(payload, cursor + 8);
        let advisor_count = read_u32_le(payload, cursor + 12) as usize;
        cursor += 16;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has unknown flags {flags}"
            ));
        }
        if elections_allowed != u32::MAX && elections_allowed > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals country row {row_index} has invalid elections_allowed {elections_allowed}"
            ));
        }
        declared_advisors = declared_advisors.saturating_add(advisor_count);
        if let Err(err) = skip_compact_string(payload, &mut cursor, "politics last election") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if declared_advisors != advisor_detail_count {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals advisor_detail_count {advisor_detail_count} != declared country advisors {declared_advisors}"
        ));
    }
    for row_index in 0..advisor_detail_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 4, "politics residuals advisor row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        cursor += 4;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: politics residuals advisor row {row_index} has zero tag_db_id"
            ));
        }
        for field in [
            "politics advisor idea token",
            "politics advisor slot",
            "politics advisor ledger",
            "politics advisor character",
        ] {
            if let Err(err) = skip_compact_string(payload, &mut cursor, field) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: politics residuals data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_decisions_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let timed_count = read_u32_le(payload, 20) as usize;
    add_decoder_status(summary, status_flags);
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&16) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 16, "decision_country_rows", country_count);
    add_compact_domain_data_unit(summary, 16, "timed_decision_rows", timed_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_timed = 0usize;
    for row_index in 0..country_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 32, "decisions country row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let timed_for_country = read_u32_le(payload, cursor + 28) as usize;
        cursor += 32;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions country row {row_index} has unknown flags {flags}"
            ));
        }
        declared_timed = declared_timed.saturating_add(timed_for_country);
    }
    if declared_timed != timed_count {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions timed_count {timed_count} != declared country timed decisions {declared_timed}"
        ));
    }
    for row_index in 0..timed_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 12, "decisions timed row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let days = read_i32_le(payload, cursor + 4);
        let state = read_i32_le(payload, cursor + 8);
        cursor += 12;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has zero tag_db_id"
            ));
        }
        if !(0..=10_000).contains(&days) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has invalid days {days}"
            ));
        }
        if !(0..=16).contains(&state) {
            summary.parse_errors.push(format!(
                "offset {offset}: decisions timed row {row_index} has invalid state {state}"
            ));
        }
        if let Err(err) = skip_compact_string(payload, &mut cursor, "decisions timed token") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: decisions data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

fn decode_compact_frd_depth_data_payload(
    offset: usize,
    payload: &[u8],
    record_flags: u32,
    summary: &mut DomainDataDecodeSummary,
) {
    let Some(header) = decode_compact_frd_depth_header(offset, payload, record_flags, summary)
    else {
        return;
    };
    if let Some(raw_status_flags) = header.raw_status_flags {
        add_decoder_status(summary, raw_status_flags);
    }
    let decision_target_count = header.decision_target_count;
    let decision_target_row_fixed_len = header.decision_target_row_fixed_len;
    let decision_target_omitted_rows = header.decision_target_omitted_rows;
    let schema_version = header.schema_version;
    let direct_transition_count = header.direct_transition_count;
    let depth_status_flags = header.depth_status_flags;
    let focus_count = header.focus_count;
    let research_count = header.research_count;
    let decision_status_count = header.decision_status_count;
    let decision_event_count = header.decision_event_count;
    let observed_focus_rows = header.observed_focus_rows;
    let observed_research_slot_rows = header.observed_research_slot_rows;
    let observed_decision_rows = header.observed_decision_rows;
    let observed_timed_decision_rows = header.observed_timed_decision_rows;
    let badge_count = header.badge_count;
    let emitted_rows = focus_count
        .saturating_add(research_count)
        .saturating_add(decision_status_count)
        .saturating_add(decision_event_count)
        .saturating_add(decision_target_count)
        .saturating_add(badge_count)
        .saturating_add(direct_transition_count);

    let admitted_depth_status_mask = if schema_version == COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 {
        COMPACT_FRD_STATUS_V5_KNOWN_MASK
    } else {
        COMPACT_FRD_STATUS_KNOWN_MASK
    };
    if depth_status_flags & !admitted_depth_status_mask != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth status_flags has unknown bits: {depth_status_flags}"
        ));
    }
    let direct_source_validated =
        depth_status_flags & COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED != 0;
    if schema_version == COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 && !direct_source_validated {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth schema-v5 payload lacks validated direct-hook source"
        ));
    }
    if schema_version != COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 && direct_source_validated {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth legacy payload claims validated direct-hook source"
        ));
    }
    let exact_status = COMPACT_FRD_STATUS_FOCUS_PAUSE_EXACT
        | COMPACT_FRD_STATUS_DECISION_TARGETS_EXACT
        | COMPACT_FRD_STATUS_DECISION_COOLDOWNS_EXACT;
    let legacy_status =
        COMPACT_FRD_STATUS_FOCUS_PAUSE_BADGED | COMPACT_FRD_STATUS_DECISION_TARGETS_BADGED;
    if depth_status_flags & COMPACT_FRD_STATUS_DIRECT_EVENTS_DERIVED == 0
        || (depth_status_flags & exact_status != exact_status
            && depth_status_flags & legacy_status != legacy_status)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth status_flags lack a complete exact or legacy authority set"
        ));
    }

    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(emitted_rows as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(emitted_rows as u64);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "focus_depth_event_rows",
        focus_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "research_eta_rows",
        research_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "decision_status_rows",
        decision_status_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "decision_event_rows",
        decision_event_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "decision_target_rows",
        decision_target_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "decision_target_omitted_rows",
        decision_target_omitted_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "depth_badge_rows",
        badge_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "direct_transition_rows",
        direct_transition_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "depth_observed_research_slot_rows",
        observed_research_slot_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "depth_observed_timed_decision_rows",
        observed_timed_decision_rows,
    );

    let mut cursor = header.header_len;
    let mut seen_focus = BTreeSet::new();
    for row_index in 0..focus_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 36, "focus depth row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let focus_kind = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        let previous_completed = read_u32_le(payload, cursor + 16);
        let current_completed = read_u32_le(payload, cursor + 20);
        let previous_progress = read_i32_le(payload, cursor + 24);
        let current_progress = read_i32_le(payload, cursor + 28);
        let completed_delta = read_u32_le(payload, cursor + 32);
        cursor += 36;
        let from_len = match skip_compact_string_len(payload, &mut cursor, "focus depth from token")
        {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        let to_len = match skip_compact_string_len(payload, &mut cursor, "focus depth to token") {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        let completed_len =
            match skip_compact_string_len(payload, &mut cursor, "focus depth completed token") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has zero tag_db_id"
            ));
        }
        if !matches!(
            kind,
            COMPACT_FRD_FOCUS_EVENT_COMPLETED_COUNT_INCREASED
                | COMPACT_FRD_FOCUS_EVENT_CURRENT_STARTED
                | COMPACT_FRD_FOCUS_EVENT_STOPPED
                | COMPACT_FRD_FOCUS_EVENT_CHANGED
                | COMPACT_FRD_FOCUS_EVENT_PROGRESS_REGRESSED
                | COMPACT_FRD_FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES
                | COMPACT_FRD_FOCUS_EVENT_STATE_SNAPSHOT
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has invalid kind {kind}"
            ));
        }
        if focus_kind > COMPACT_FRD_FOCUS_KIND_CONTINUOUS {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has invalid focus_kind {focus_kind}"
            ));
        }
        if flags
            & !(COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED
                | COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT
                | COMPACT_FRD_FOCUS_FLAG_PAUSED
                | COMPACT_FRD_FOCUS_FLAG_PAUSED_PRESENT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has unknown flags {flags}"
            ));
        }
        if kind == COMPACT_FRD_FOCUS_EVENT_STATE_SNAPSHOT {
            if flags & COMPACT_FRD_FOCUS_FLAG_PAUSED_PRESENT == 0
                || flags
                    & (COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED
                        | COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT)
                    != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus state snapshot row {row_index} has invalid pause flags {flags}"
                ));
            }
        } else if flags & (COMPACT_FRD_FOCUS_FLAG_PAUSED | COMPACT_FRD_FOCUS_FLAG_PAUSED_PRESENT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: non-snapshot focus row {row_index} carries pause flags {flags}"
            ));
        }
        if kind == COMPACT_FRD_FOCUS_EVENT_COMPLETED_COUNT_INCREASED {
            if current_completed != u32::MAX
                && previous_completed != u32::MAX
                && current_completed <= previous_completed
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus completed row {row_index} did not increase completed_count"
                ));
            }
            if completed_delta == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus completed row {row_index} has zero completed_delta"
                ));
            }
        }
        if kind == COMPACT_FRD_FOCUS_EVENT_PROGRESS_REGRESSED
            && current_progress >= previous_progress
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus progress-regression row {row_index} did not regress"
            ));
        }
        if flags & COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT != 0 && completed_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus row {row_index} has exact completed-token flag without token"
            ));
        }
        // A completed-count-increase row is legitimately token-less when the
        // encoder could not resolve a focus token: the country's completed_count
        // ticked up while it had no active national/continuous focus at either
        // bracketing sample (focus_kind == 0) and completed_tokens surfaced no
        // new token, so neither the EXACT nor INFERRED completed-token flag is
        // set. This is the encoder's `focus_completed_unresolved_rows` state, not
        // corruption: the completion delta and counts are still carried, so
        // accepting it loses no data and fabricates no token. An EXACT/INFERRED
        // completed row without a token stays a hard error (caught here and by the
        // exact-flag check above), and kinds 2..=5 always carry >=1 token, so this
        // exemption stays tight and fail-closed.
        let unresolved_completion = kind == COMPACT_FRD_FOCUS_EVENT_COMPLETED_COUNT_INCREASED
            && flags
                & (COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_EXACT
                    | COMPACT_FRD_FOCUS_FLAG_COMPLETED_TOKEN_INFERRED)
                == 0;
        if from_len == 0
            && to_len == 0
            && completed_len == 0
            && kind != COMPACT_FRD_FOCUS_EVENT_INVALID_BOTH_CURRENT_TYPES
            && kind != COMPACT_FRD_FOCUS_EVENT_STATE_SNAPSHOT
            && !unresolved_completion
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} has no token context"
            ));
        }
        if !seen_focus.insert((
            tag_db_id,
            kind,
            previous_completed,
            current_completed,
            completed_delta,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus depth row {row_index} duplicates focus transition key"
            ));
        }
    }

    let mut seen_research = BTreeSet::new();
    for row_index in 0..research_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_FRD_RESEARCH_ETA_ROW_LEN,
            "research ETA row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let slot_index = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let hour_delta = read_u32_le(payload, cursor + 12);
        let previous_handle = read_u32_le(payload, cursor + 16);
        let current_handle = read_u32_le(payload, cursor + 20);
        let previous_def = read_u32_le(payload, cursor + 24);
        let current_def = read_u32_le(payload, cursor + 28);
        let previous_progress = read_i64_le(payload, cursor + 32);
        let current_progress = read_i64_le(payload, cursor + 40);
        let delta = read_i64_le(payload, cursor + 48);
        let speed = read_i64_le(payload, cursor + 56);
        let base = read_i64_le(payload, cursor + 64);
        let eta = read_i64_le(payload, cursor + 72);
        cursor += COMPACT_FRD_RESEARCH_ETA_ROW_LEN;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has zero tag_db_id"
            ));
        }
        if slot_index > 32 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        if flags
            & !(COMPACT_FRD_RESEARCH_FLAG_SAME_TECH
                | COMPACT_FRD_RESEARCH_FLAG_POSITIVE_SPEED
                | COMPACT_FRD_RESEARCH_FLAG_ETA_AVAILABLE
                | COMPACT_FRD_RESEARCH_FLAG_TECH_ROLLOVER
                | COMPACT_FRD_RESEARCH_FLAG_NEGATIVE_DELTA
                | COMPACT_FRD_RESEARCH_FLAG_BASE_PRESENT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has unknown flags {flags}"
            ));
        }
        if hour_delta == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has zero hour_delta"
            ));
        }
        let same_tech = flags & COMPACT_FRD_RESEARCH_FLAG_SAME_TECH != 0;
        if same_tech && (previous_handle != current_handle || previous_def != current_def) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has same-tech flag but different tech ids"
            ));
        }
        if !same_tech && flags & COMPACT_FRD_RESEARCH_FLAG_TECH_ROLLOVER == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} changed tech without rollover flag"
            ));
        }
        if delta != current_progress.saturating_sub(previous_progress) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} delta does not match progress"
            ));
        }
        if flags & COMPACT_FRD_RESEARCH_FLAG_POSITIVE_SPEED != 0 && speed <= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has positive-speed flag without positive speed"
            ));
        }
        if flags & COMPACT_FRD_RESEARCH_FLAG_NEGATIVE_DELTA != 0 && delta >= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has negative-delta flag without negative delta"
            ));
        }
        if flags & COMPACT_FRD_RESEARCH_FLAG_BASE_PRESENT != 0 && base <= 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has base-present flag without positive base"
            ));
        }
        if flags & COMPACT_FRD_RESEARCH_FLAG_ETA_AVAILABLE != 0 && eta < 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} has negative eta"
            ));
        }
        if !seen_research.insert((
            tag_db_id,
            slot_index,
            current_handle,
            current_def,
            current_progress,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: research ETA row {row_index} duplicates slot/progress key"
            ));
        }
    }

    let mut seen_decision_status = BTreeSet::new();
    let mut targeted_status_count_by_tag = BTreeMap::new();
    let mut targeted_timed_status_count_by_tag = BTreeMap::new();
    for row_index in 0..decision_status_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_FRD_DECISION_STATUS_ROW_LEN,
            "decision status row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let counts = [
            read_u32_le(payload, cursor + 4),
            read_u32_le(payload, cursor + 8),
            read_u32_le(payload, cursor + 12),
            read_u32_le(payload, cursor + 16),
            read_u32_le(payload, cursor + 20),
            read_u32_le(payload, cursor + 24),
        ];
        cursor += COMPACT_FRD_DECISION_STATUS_ROW_LEN;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} has zero tag_db_id"
            ));
        }
        if counts.iter().any(|count| *count > 4096) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} has impossible count"
            ));
        }
        if !seen_decision_status.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision status row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        targeted_status_count_by_tag.insert(tag_db_id, counts[1]);
        targeted_timed_status_count_by_tag.insert(tag_db_id, counts[2]);
    }

    for row_index in 0..decision_event_count {
        if let Err(err) = require_compact_remaining(payload, cursor, 28, "decision event row") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let kind = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let previous_days = read_i32_le(payload, cursor + 12);
        let current_days = read_i32_le(payload, cursor + 16);
        let previous_state = read_i32_le(payload, cursor + 20);
        let current_state = read_i32_le(payload, cursor + 24);
        cursor += 28;
        let decision_len =
            match skip_compact_string_len(payload, &mut cursor, "decision event token") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has zero tag_db_id"
            ));
        }
        if !matches!(
            kind,
            COMPACT_FRD_DECISION_EVENT_STARTED
                | COMPACT_FRD_DECISION_EVENT_STOPPED
                | COMPACT_FRD_DECISION_EVENT_DAY_DELTA
                | COMPACT_FRD_DECISION_EVENT_STATE_CHANGED
                | COMPACT_FRD_DECISION_EVENT_STATUS_COUNTS_CHANGED
                | COMPACT_FRD_DECISION_EVENT_RE_ENABLE_COOLDOWN_SNAPSHOT
                | COMPACT_FRD_DECISION_EVENT_REMOVE_COOLDOWN_SNAPSHOT
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has invalid kind {kind}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has unknown flags {flags}"
            ));
        }
        if kind != COMPACT_FRD_DECISION_EVENT_STATUS_COUNTS_CHANGED && decision_len == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: decision event row {row_index} has no decision token"
            ));
        }
        if kind == COMPACT_FRD_DECISION_EVENT_DAY_DELTA && previous_days == current_days {
            summary.parse_errors.push(format!(
                "offset {offset}: decision day-delta row {row_index} did not change days"
            ));
        }
        if kind == COMPACT_FRD_DECISION_EVENT_STATE_CHANGED && previous_state == current_state {
            summary.parse_errors.push(format!(
                "offset {offset}: decision state-change row {row_index} did not change state"
            ));
        }
    }

    let mut decision_target_rows_by_tag_and_source: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut seen_decision_targets = BTreeSet::new();
    let mut country_target_rows = 0usize;
    for row_index in 0..decision_target_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            decision_target_row_fixed_len,
            "decision target row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let target_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let source_kind = read_u32_le(payload, cursor + 12);
        let detailed = decision_target_row_fixed_len == COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN;
        let lifecycle_state = detailed.then(|| read_i32_le(payload, cursor + 16));
        let ignore_raw = detailed.then(|| read_u32_le(payload, cursor + 20));
        let days = detailed.then(|| read_i32_le(payload, cursor + 24));
        cursor += decision_target_row_fixed_len;
        let decision = match read_compact_string(payload, &mut cursor, "decision target token") {
            Ok(decision) => decision,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };

        if tag_db_id == 0 || tag_db_id > COMPACT_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid owner tag_db_id {tag_db_id}"
            ));
        }
        let country_flags = flags == COMPACT_FRD_DECISION_TARGET_COUNTRY_FLAGS;
        let numeric_flags = flags
            == (COMPACT_FRD_DECISION_TARGET_FLAG_NUMERIC_ID
                | COMPACT_FRD_DECISION_TARGET_FLAG_RECORD_0X24);
        if !country_flags && !numeric_flags {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has unknown or mixed flags {flags:#x}"
            ));
        }
        if target_id == 0
            || (country_flags && target_id > COMPACT_MAX_OWNER_TAG_DB_ID)
            // The numeric target is a STATE id (see the collector's note).
            || (numeric_flags && target_id > COMPACT_MAX_STATE_ID_FRD)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid typed target id {target_id}"
            ));
        }
        if source_kind != COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED
            && (source_kind != COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED || !detailed)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has invalid source_kind {source_kind}"
            ));
        }
        if decision.is_empty() {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} has no decision token"
            ));
        }
        if let (Some(lifecycle_state), Some(ignore_raw), Some(days)) =
            (lifecycle_state, ignore_raw, days)
        {
            if !(0..=4).contains(&lifecycle_state) {
                summary.parse_errors.push(format!(
                    "offset {offset}: decision target row {row_index} has invalid lifecycle_state {lifecycle_state}"
                ));
            }
            if ignore_raw > 1 {
                summary.parse_errors.push(format!(
                    "offset {offset}: decision target row {row_index} has invalid ignore value {ignore_raw}"
                ));
            }
            if !(-100_000..=100_000).contains(&days) {
                summary.parse_errors.push(format!(
                    "offset {offset}: decision target row {row_index} has invalid days {days}"
                ));
            }
        }
        if !seen_decision_targets.insert((tag_db_id, target_id, source_kind, decision.clone())) {
            summary.parse_errors.push(format!(
                "offset {offset}: decision target row {row_index} duplicates owner/target/source key"
            ));
        }
        if country_flags {
            country_target_rows = country_target_rows.saturating_add(1);
        }
        let count = decision_target_rows_by_tag_and_source
            .entry((tag_db_id, source_kind))
            .or_insert(0);
        *count = count.saturating_add(1);
    }
    for ((tag_db_id, source_kind), target_rows) in decision_target_rows_by_tag_and_source {
        let status_count = match source_kind {
            COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED => {
                targeted_status_count_by_tag.get(&tag_db_id).copied()
            }
            COMPACT_FRD_DECISION_TARGET_SOURCE_ACTIVE_TARGETED_TIMED => {
                targeted_timed_status_count_by_tag.get(&tag_db_id).copied()
            }
            _ => None,
        };
        match status_count {
            Some(status_count) if target_rows <= status_count => {}
            Some(status_count) => summary.parse_errors.push(format!(
                "offset {offset}: decision target rows {target_rows} exceed source_kind {source_kind} status count {status_count} for tag_db_id {tag_db_id}"
            )),
            None => summary.parse_errors.push(format!(
                "offset {offset}: decision target rows exist without a decision status row for tag_db_id {tag_db_id}"
            )),
        }
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
        "decision_country_target_rows",
        country_target_rows,
    );

    let mut badge_kinds = BTreeSet::new();
    for row_index in 0..badge_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_FRD_BADGE_ROW_FIXED_LEN,
            "badge row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let kind = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let source_domain_id = read_u32_le(payload, cursor + 8);
        let affected_count = read_u32_le(payload, cursor + 12);
        let evidence_code = read_u32_le(payload, cursor + 16);
        cursor += COMPACT_FRD_BADGE_ROW_FIXED_LEN;
        let detail = match read_compact_string(payload, &mut cursor, "badge detail") {
            Ok(value) => value,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if kind == 0 || kind > COMPACT_FRD_BADGE_KIND_MAX {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has invalid kind {kind}"
            ));
        }
        if flags == 0 || flags & !COMPACT_FRD_BADGE_FLAG_KNOWN_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has invalid flags {flags}"
            ));
        }
        if detail.is_empty() {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} has no detail token"
            ));
        }
        if !badge_kinds.insert(kind) {
            summary.parse_errors.push(format!(
                "offset {offset}: badge row {row_index} duplicates kind {kind}"
            ));
        }
        match kind {
            COMPACT_FRD_BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED => {
                if source_domain_id != 12
                    || evidence_code != COMPACT_FRD_BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH
                    || flags & COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED == 0
                    || detail != COMPACT_FRD_BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus pause badge row {row_index} has inconsistent provenance"
                    ));
                }
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
                    "focus_pause_badge_rows",
                    1,
                );
                if affected_count > observed_focus_rows {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus pause badge row {row_index} affected_count exceeds observed focus rows"
                    ));
                }
            }
            COMPACT_FRD_BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED => {
                if source_domain_id != 16
                    || evidence_code != COMPACT_FRD_BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED
                    || flags & COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED == 0
                    || detail != COMPACT_FRD_BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: decision target badge row {row_index} has inconsistent provenance"
                    ));
                }
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
                    "decision_target_badge_rows",
                    1,
                );
                if observed_decision_rows == 0 && affected_count != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: decision target badge row {row_index} has affected decisions without observed decision rows"
                    ));
                }
            }
            COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE => {
                if source_domain_id != 1000
                    || evidence_code != COMPACT_FRD_BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED
                    || flags & COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED == 0
                    || flags & COMPACT_FRD_BADGE_FLAG_DERIVED_DETERMINISTIC == 0
                    || detail != COMPACT_FRD_BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: direct event badge row {row_index} has inconsistent provenance"
                    ));
                }
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
                    "direct_event_badge_rows",
                    1,
                );
            }
            _ => {}
        }
    }

    for (flag, exact_flag, kind, name) in [
        (
            COMPACT_FRD_STATUS_FOCUS_PAUSE_BADGED,
            COMPACT_FRD_STATUS_FOCUS_PAUSE_EXACT,
            COMPACT_FRD_BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
            "focus pause",
        ),
        (
            COMPACT_FRD_STATUS_DECISION_TARGETS_BADGED,
            COMPACT_FRD_STATUS_DECISION_TARGETS_EXACT,
            COMPACT_FRD_BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED,
            "decision target fields",
        ),
        (
            COMPACT_FRD_STATUS_DIRECT_EVENTS_DERIVED,
            COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED,
            COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE,
            "direct event hooks",
        ),
    ] {
        if depth_status_flags & exact_flag == 0
            && depth_status_flags & flag != 0
            && !badge_kinds.contains(&kind)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth status flag for {name} lacks badge row"
            ));
        }
        if depth_status_flags & exact_flag == 0 && !badge_kinds.contains(&kind) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth missing required {name} badge row"
            ));
        }
    }
    if direct_source_validated
        && badge_kinds.contains(&COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth validated direct-hook source carries unavailable badge"
        ));
    }
    if schema_version == COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 {
        let trailer = &payload[cursor..];
        // A logical stream may begin after the process-wide hook epoch has
        // already drained pre-world or prior-campaign rows. For an empty first
        // drain, the authoritative sequence cursor is therefore the stream's
        // baseline; it is not required to be the hook-epoch origin (1).
        let sequence_start = if direct_transition_count == 0 {
            32
        } else {
            direct_frd_transitions_live::D45_V5_DIRECT_EVENT_TRAILER_HEADER_LEN
        };
        let Some(sequence_bytes) = trailer.get(sequence_start..sequence_start + 8) else {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth v5 direct trailer is too short for its sequence baseline"
            ));
            return;
        };
        let expected_first_sequence = u64::from_le_bytes(
            sequence_bytes
                .try_into()
                .expect("fixed direct-event sequence slice"),
        );
        let rows = match direct_frd_transitions_live::decode_trailer(
            trailer,
            expected_first_sequence,
        ) {
            Ok(rows) => rows,
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus/research/decisions depth v5 direct trailer rejected: {error}"
                ));
                return;
            }
        };
        if rows.len() != direct_transition_count {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth v5 direct trailer row count {} != header {direct_transition_count}",
                rows.len()
            ));
        }
        cursor = payload.len();
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
}

#[derive(Clone, Copy)]
struct CompactFrdDepthHeader {
    schema_version: u16,
    header_len: usize,
    decision_target_row_fixed_len: usize,
    raw_status_flags: Option<u32>,
    decision_target_count: usize,
    decision_target_omitted_rows: usize,
    depth_status_flags: u32,
    focus_count: usize,
    research_count: usize,
    decision_status_count: usize,
    decision_event_count: usize,
    observed_focus_rows: u32,
    observed_research_slot_rows: usize,
    observed_decision_rows: u32,
    observed_timed_decision_rows: usize,
    badge_count: usize,
    direct_transition_count: usize,
}

fn decode_compact_frd_depth_header(
    offset: usize,
    payload: &[u8],
    record_flags: u32,
    summary: &mut DomainDataDecodeSummary,
) -> Option<CompactFrdDepthHeader> {
    if payload.len() < COMPACT_FRD_DEPTH_HEADER_LEN_V1 {
        summary.parse_errors.push(format!(
            "offset {offset}: focus/research/decisions depth payload is {} byte(s), expected at least {COMPACT_FRD_DEPTH_HEADER_LEN_V1}",
            payload.len()
        ));
        return None;
    }
    if record_flags & COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 != 0 {
        if payload.len() < COMPACT_FRD_DEPTH_HEADER_LEN_V2 {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth versioned payload is {} byte(s), expected at least {COMPACT_FRD_DEPTH_HEADER_LEN_V2}",
                payload.len()
            ));
            return None;
        }
        if payload.get(0..4) != Some(COMPACT_FRD_DEPTH_SCHEMA_MAGIC.as_slice()) {
            summary.parse_errors.push(format!(
                "offset {offset}: focus/research/decisions depth versioned payload is missing FRD2 sanity marker"
            ));
            return None;
        }
        let schema_version = read_u16_le(payload, 4);
        let header_len = read_u16_le(payload, 6) as usize;
        let raw_status_flags = match (schema_version, header_len) {
            (COMPACT_FRD_DEPTH_SCHEMA_VERSION_V2, COMPACT_FRD_DEPTH_HEADER_LEN_V2) => None,
            (
                COMPACT_FRD_DEPTH_SCHEMA_VERSION_V3 | COMPACT_FRD_DEPTH_SCHEMA_VERSION,
                COMPACT_FRD_DEPTH_HEADER_LEN,
            ) => {
                if payload.len() < COMPACT_FRD_DEPTH_HEADER_LEN {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus/research/decisions depth v3 payload is {} byte(s), expected at least {COMPACT_FRD_DEPTH_HEADER_LEN}",
                        payload.len()
                    ));
                    return None;
                }
                Some(read_u32_le(payload, 56))
            }
            (COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5, COMPACT_FRD_DEPTH_HEADER_LEN_V5) => {
                if payload.len() < COMPACT_FRD_DEPTH_HEADER_LEN_V5 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: focus/research/decisions depth v5 payload is {} byte(s), expected at least {COMPACT_FRD_DEPTH_HEADER_LEN_V5}",
                        payload.len()
                    ));
                    return None;
                }
                Some(read_u32_le(payload, 56))
            }
            _ => {
                summary.parse_errors.push(format!(
                    "offset {offset}: focus/research/decisions depth versioned header has schema_version {schema_version} and header_len {header_len}"
                ));
                return None;
            }
        };
        return Some(CompactFrdDepthHeader {
            schema_version,
            header_len,
            decision_target_row_fixed_len: if matches!(
                schema_version,
                COMPACT_FRD_DEPTH_SCHEMA_VERSION | COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5
            ) {
                COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN
            } else {
                COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN_V3
            },
            raw_status_flags,
            depth_status_flags: read_u32_le(payload, 8),
            focus_count: read_u32_le(payload, 12) as usize,
            research_count: read_u32_le(payload, 16) as usize,
            decision_status_count: read_u32_le(payload, 20) as usize,
            decision_event_count: read_u32_le(payload, 24) as usize,
            decision_target_count: read_u32_le(payload, 28) as usize,
            observed_focus_rows: read_u32_le(payload, 32),
            observed_research_slot_rows: read_u32_le(payload, 36) as usize,
            observed_decision_rows: read_u32_le(payload, 40),
            observed_timed_decision_rows: read_u32_le(payload, 44) as usize,
            badge_count: read_u32_le(payload, 48) as usize,
            decision_target_omitted_rows: read_u32_le(payload, 52) as usize,
            direct_transition_count: if schema_version == COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 {
                read_u32_le(payload, 60) as usize
            } else {
                0
            },
        });
    }
    Some(CompactFrdDepthHeader {
        schema_version: 1,
        header_len: COMPACT_FRD_DEPTH_HEADER_LEN_V1,
        decision_target_row_fixed_len: COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN_V3,
        raw_status_flags: None,
        decision_target_count: 0,
        decision_target_omitted_rows: 0,
        depth_status_flags: read_u32_le(payload, 8),
        focus_count: read_u32_le(payload, 12) as usize,
        research_count: read_u32_le(payload, 16) as usize,
        decision_status_count: read_u32_le(payload, 20) as usize,
        decision_event_count: read_u32_le(payload, 24) as usize,
        observed_focus_rows: read_u32_le(payload, 28),
        observed_research_slot_rows: read_u32_le(payload, 32) as usize,
        observed_decision_rows: read_u32_le(payload, 36),
        observed_timed_decision_rows: read_u32_le(payload, 40) as usize,
        badge_count: read_u32_le(payload, 44) as usize,
        direct_transition_count: 0,
    })
}

fn decode_compact_diplomacy_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 24 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data payload is {} byte(s), expected at least 24",
            payload.len()
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let relation_count = read_u32_le(payload, 20) as usize;
    add_decoder_status(summary, status_flags);
    let expected_len = 24usize
        .saturating_add(country_count.saturating_mul(24))
        .saturating_add(relation_count.saturating_mul(16));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > country_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy data changed_rows {changed_rows} > country_count {country_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&17) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 17, "diplomacy_country_rows", country_count);
    add_compact_domain_data_unit(summary, 17, "at_war_relation_rows", relation_count);
    let mut cursor = 24usize;
    let mut seen_countries = BTreeSet::new();
    let mut declared_relations = 0usize;
    for row_index in 0..country_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let _faction_group = read_i32_le(payload, cursor + 8);
        let at_war_count = read_u32_le(payload, cursor + 12) as usize;
        let _casualties = read_i64_le(payload, cursor + 16);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} has zero tag_db_id"
            ));
        } else if !seen_countries.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy country row {row_index} has unknown flags {flags}"
            ));
        }
        declared_relations = declared_relations.saturating_add(at_war_count);
        cursor += 24;
    }
    if declared_relations != relation_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy relation_count {relation_count} != declared country relations {declared_relations}"
        ));
    }
    let mut seen_relations = BTreeSet::new();
    for row_index in 0..relation_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let token_len = read_u32_le(payload, cursor + 4) as usize;
        let token = &payload[cursor + 8..cursor + 16];
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has zero tag_db_id"
            ));
        }
        if token_len == 0 || token_len > 8 {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has invalid token_len {token_len}"
            ));
        }
        if token_len <= 8
            && token[..token_len]
                .iter()
                .any(|byte| !(byte.is_ascii_alphanumeric() || *byte == b'_' || *byte == b'-'))
        {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has non-token bytes"
            ));
        }
        if token_len < 8 && token[token_len..].iter().any(|byte| *byte != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} has nonzero padding"
            ));
        }
        if !seen_relations.insert((tag_db_id, token.to_vec())) {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy relation row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        cursor += 16;
    }
}

fn decode_compact_divisions_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let reserved = read_u32_le(payload, 28);
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data reserved field is nonzero"
        ));
    }
    let legacy_expected =
        32usize.saturating_add(row_count.saturating_mul(LEGACY_COMPACT_DIVISION_ROW_LEN));
    let extended_expected =
        32usize.saturating_add(row_count.saturating_mul(COMPACT_DIVISION_ROW_LEN));
    let row_len = if payload.len() == extended_expected {
        COMPACT_DIVISION_ROW_LEN
    } else if payload.len() == legacy_expected {
        LEGACY_COMPACT_DIVISION_ROW_LEN
    } else {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data payload is {} byte(s), expected legacy {legacy_expected} or extended {extended_expected}",
            payload.len()
        ));
        return;
    };
    let extended = row_len == COMPACT_DIVISION_ROW_LEN;
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    if !delta_data && row_count > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data row_count {row_count} > root_rows {root_rows}"
        ));
    }
    if (status_flags & COMPACT_STATUS_BRIDGE_UNAVAILABLE) != 0 && root_rows == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions data root source unavailable"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&20) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 20, "division_rows", row_count);
    add_compact_domain_data_unit(summary, 20, "division_root_rows", root_rows);
    let mut cursor = 32usize;
    let mut seen_divisions = BTreeSet::new();
    let mut deleted_rows = 0usize;
    let mut moving_rows = 0usize;
    let mut combat_rows = 0usize;
    let mut template_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut army_rows = 0usize;
    let mut org_strength_rows = 0usize;
    let mut order_army_rows = 0usize;
    let mut order_group_rows = 0usize;
    let mut order_unresolved_rows = 0usize;
    let mut disrupted_supply_rows = 0usize;
    let mut strategic_redeployment_rows = 0usize;
    let mut current_manpower_rows = 0usize;
    let mut cumulative_losses_rows = 0usize;
    for row_index in 0..row_count {
        let div_id = read_u32_le(payload, cursor);
        let div_type = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let location = read_u32_le(payload, cursor + 12);
        let move_target = read_u32_le(payload, cursor + 16);
        let move_source = read_u32_le(payload, cursor + 20);
        let movement_progress = read_u32_le(payload, cursor + 24);
        let progress_bucket = read_u32_le(payload, cursor + 28);
        let organisation_raw = read_u32_le(payload, cursor + 32);
        let _organisation_bucket = read_u32_le(payload, cursor + 36);
        let strength_raw = read_u32_le(payload, cursor + 40);
        let _strength_bucket = read_u32_le(payload, cursor + 44);
        let combat_ref_count = read_u32_le(payload, cursor + 48);
        let template_id = read_u32_le(payload, cursor + 52);
        let owner_tag_db_id = read_u32_le(payload, cursor + 56);
        let army_id = read_u32_le(payload, cursor + 60);
        let experience_raw = read_u32_le(payload, cursor + 64);
        let max_supply_raw = read_u32_le(payload, cursor + 68);
        let current_supply_raw = read_u32_le(payload, cursor + 72);
        let supply_gain_raw = read_u32_le(payload, cursor + 76);
        let fuel_raw = read_u32_le(payload, cursor + 80);
        let row_reserved2 = read_u32_le(payload, cursor + 84);
        let disrupted_supply_raw = read_u32_le(payload, cursor + 88);
        let strategic_redeployment_raw = read_u32_le(payload, cursor + 92);
        let current_manpower = read_u32_le(payload, cursor + 96);
        let cumulative_losses = read_u32_le(payload, cursor + 100);
        let max_organisation_raw = extended.then(|| read_u32_le(payload, cursor + 104)).unwrap_or(0);
        let counter_supply_status_raw = extended.then(|| read_u32_le(payload, cursor + 108)).unwrap_or(0);
        let dig_in_raw = extended.then(|| read_u32_le(payload, cursor + 112)).unwrap_or(0);
        let dig_in_cap_raw = extended.then(|| read_u32_le(payload, cursor + 116)).unwrap_or(0);
        cursor += row_len;

        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert(div_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} duplicates div_id {div_id}"
            ));
        }
        if div_type != COMPACT_DIVISION_TYPE {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has div_type {div_type}"
            ));
        }
        if flags
            & !(0b1111_1111_1111
                | COMPACT_DIVISION_ROW_FLAG_DELETED
                | COMPACT_DIVISION_ROW_FLAG_TEMPLATE
                | COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY
                | COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY_GROUP
                | COMPACT_DIVISION_ROW_FLAG_ORDER_UNRESOLVED
                | COMPACT_DIVISION_ROW_FLAG_DISRUPTED_SUPPLY
                | COMPACT_DIVISION_ROW_FLAG_STRATEGIC_REDEPLOYMENT
                | COMPACT_DIVISION_ROW_FLAG_CURRENT_MANPOWER
                | COMPACT_DIVISION_ROW_FLAG_CUMULATIVE_LOSSES
                | COMPACT_DIVISION_ROW_FLAG_MAX_ORGANISATION
                | COMPACT_DIVISION_ROW_FLAG_COUNTER_SUPPLY_STATUS
                | COMPACT_DIVISION_ROW_FLAG_DIG_IN
                | COMPACT_DIVISION_ROW_FLAG_DIG_IN_CAP
                | COMPACT_DIVISION_ROW_FLAG_TRAINING_OBSERVED
                | COMPACT_DIVISION_ROW_FLAG_TRAINING_VISIBLE
                | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_OBSERVED
                | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_VISIBLE
                | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_CAPPED
                | COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_OBSERVED
                | COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_VISIBLE)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = (flags & COMPACT_DIVISION_ROW_FLAG_DELETED) != 0;
        let moving = (flags & 1) != 0;
        let has_progress = (flags & (1 << 1)) != 0;
        let has_target = (flags & (1 << 2)) != 0;
        let has_source = (flags & (1 << 3)) != 0;
        let in_combat = (flags & (1 << 4)) != 0;
        let has_owner = (flags & (1 << 5)) != 0;
        let has_army = (flags & (1 << 6)) != 0;
        let has_experience = (flags & (1 << 7)) != 0;
        let has_max_supply = (flags & (1 << 8)) != 0;
        let has_current_supply = (flags & (1 << 9)) != 0;
        let has_supply_gain = (flags & (1 << 10)) != 0;
        let has_fuel = (flags & (1 << 11)) != 0;
        let has_disrupted_supply = (flags & COMPACT_DIVISION_ROW_FLAG_DISRUPTED_SUPPLY) != 0;
        let has_strategic_redeployment =
            (flags & COMPACT_DIVISION_ROW_FLAG_STRATEGIC_REDEPLOYMENT) != 0;
        let has_current_manpower = (flags & COMPACT_DIVISION_ROW_FLAG_CURRENT_MANPOWER) != 0;
        let has_cumulative_losses = (flags & COMPACT_DIVISION_ROW_FLAG_CUMULATIVE_LOSSES) != 0;
        let has_max_organisation =
            (flags & COMPACT_DIVISION_ROW_FLAG_MAX_ORGANISATION) != 0;
        let has_counter_supply_status =
            (flags & COMPACT_DIVISION_ROW_FLAG_COUNTER_SUPPLY_STATUS) != 0;
        let has_dig_in = (flags & COMPACT_DIVISION_ROW_FLAG_DIG_IN) != 0;
        let has_dig_in_cap = (flags & COMPACT_DIVISION_ROW_FLAG_DIG_IN_CAP) != 0;
        let training_observed =
            (flags & COMPACT_DIVISION_ROW_FLAG_TRAINING_OBSERVED) != 0;
        let training_visible =
            (flags & COMPACT_DIVISION_ROW_FLAG_TRAINING_VISIBLE) != 0;
        let dig_in_ui_observed =
            (flags & COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_OBSERVED) != 0;
        let dig_in_ui_visible =
            (flags & COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_VISIBLE) != 0;
        let dig_in_ui_capped =
            (flags & COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_CAPPED) != 0;
        let invalid_orders_observed =
            (flags & COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_OBSERVED) != 0;
        let invalid_orders_visible =
            (flags & COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_VISIBLE) != 0;
        let a15_flags = COMPACT_DIVISION_ROW_FLAG_MAX_ORGANISATION
            | COMPACT_DIVISION_ROW_FLAG_COUNTER_SUPPLY_STATUS
            | COMPACT_DIVISION_ROW_FLAG_DIG_IN
            | COMPACT_DIVISION_ROW_FLAG_DIG_IN_CAP
            | COMPACT_DIVISION_ROW_FLAG_TRAINING_OBSERVED
            | COMPACT_DIVISION_ROW_FLAG_TRAINING_VISIBLE
            | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_OBSERVED
            | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_VISIBLE
            | COMPACT_DIVISION_ROW_FLAG_DIG_IN_UI_CAPPED
            | COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_OBSERVED
            | COMPACT_DIVISION_ROW_FLAG_INVALID_ORDERS_VISIBLE;
        if !extended && flags & a15_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: legacy divisions row {row_index} carries extended A15 flags"
            ));
        }
        let has_template = (flags & COMPACT_DIVISION_ROW_FLAG_TEMPLATE) != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != COMPACT_DIVISION_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} has non-delete flags"
                ));
            }
            let zero_filled = location == 0
                && move_target == 0
                && move_source == 0
                && movement_progress == 0
                && progress_bucket == 0
                && organisation_raw == 0
                && _organisation_bucket == 0
                && strength_raw == 0
                && _strength_bucket == 0
                && combat_ref_count == 0
                && template_id == 0
                && owner_tag_db_id == 0
                && army_id == 0
                && experience_raw == 0
                && max_supply_raw == 0
                && current_supply_raw == 0
                && supply_gain_raw == 0
                && fuel_raw == 0
                && row_reserved2 == 0
                && disrupted_supply_raw == 0
                && strategic_redeployment_raw == 0
                && current_manpower == 0
                && cumulative_losses == 0
                && max_organisation_raw == 0
                && counter_supply_status_raw == 0
                && dig_in_raw == 0
                && dig_in_cap_raw == 0;
            if !zero_filled {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions delete row {row_index} is not zero-filled"
                ));
            }
            continue;
        }
        if training_visible && !training_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible training without an evaluated UI observation"
            ));
        }
        if invalid_orders_visible && !invalid_orders_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible invalid-orders without an evaluated UI observation"
            ));
        }
        if dig_in_ui_visible && !dig_in_ui_observed {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has visible dig-in without an evaluated UI observation"
            ));
        }
        if dig_in_ui_capped && (!dig_in_ui_observed || !dig_in_ui_visible) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has capped dig-in without a visible evaluated UI observation"
            ));
        }
        org_strength_rows = org_strength_rows.saturating_add(1);
        if moving {
            moving_rows = moving_rows.saturating_add(1);
        }
        if in_combat {
            combat_rows = combat_rows.saturating_add(1);
        }
        if has_template {
            template_rows = template_rows.saturating_add(1);
        }
        if has_owner {
            owner_rows = owner_rows.saturating_add(1);
        }
        if has_army {
            army_rows = army_rows.saturating_add(1);
        }
        if !valid_compact_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid location {location}"
            ));
        }
        if has_target && !valid_compact_province_id(move_target) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid move_target {move_target}"
            ));
        }
        if has_source && !valid_compact_province_id(move_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has invalid move_source {move_source}"
            ));
        }
        if !moving && (has_progress || has_target || has_source) {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has movement fields while not moving"
            ));
        }
        if has_progress && movement_progress == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has zero movement progress"
            ));
        }
        if !has_progress && movement_progress != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has progress without flag"
            ));
        }
        if !has_target && move_target != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has target without flag"
            ));
        }
        if !has_source && move_source != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has source without flag"
            ));
        }
        if !has_progress && progress_bucket != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has progress bucket without flag"
            ));
        }
        if organisation_raw > COMPACT_MAX_DIVISION_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} organisation_raw exceeds max"
            ));
        }
        if strength_raw > COMPACT_MAX_DIVISION_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} strength_raw exceeds max"
            ));
        }
        if combat_ref_count > COMPACT_MAX_DIVISION_COMBAT_REF_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} combat_ref_count exceeds max"
            ));
        }
        if !in_combat && combat_ref_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has combat refs while not in combat"
            ));
        }
        if in_combat && combat_ref_count == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} is in combat without refs"
            ));
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_template,
            template_id,
            1,
            COMPACT_MAX_DIVISION_TEMPLATE_ID,
            "template_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_owner,
            owner_tag_db_id,
            1,
            COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID,
            "owner_tag_db_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_army,
            army_id,
            1,
            COMPACT_MAX_ARMY_ID,
            "army_id",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_experience,
            experience_raw,
            0,
            COMPACT_MAX_DIVISION_EXPERIENCE_RAW,
            "experience_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_max_supply,
            max_supply_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_RAW,
            "max_supply_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_current_supply,
            current_supply_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_RAW,
            "current_supply_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_supply_gain,
            supply_gain_raw,
            0,
            COMPACT_MAX_DIVISION_SUPPLY_GAIN_RAW,
            "supply_gain_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_fuel,
            fuel_raw,
            0,
            COMPACT_MAX_DIVISION_FUEL_RAW,
            "fuel_raw",
        );
        // Wave-4 disrupted supply: present value gated 0..=max (a present 0 is
        // valid — 3162/3168 divisions); absent must carry a zero field.
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_disrupted_supply,
            disrupted_supply_raw,
            0,
            COMPACT_MAX_DIVISION_DISRUPTED_SUPPLY_RAW,
            "disrupted_supply_raw",
        );
        if has_disrupted_supply {
            disrupted_supply_rows = disrupted_supply_rows.saturating_add(1);
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_strategic_redeployment,
            strategic_redeployment_raw,
            0,
            1,
            "strategic_redeployment_raw",
        );
        if has_strategic_redeployment {
            strategic_redeployment_rows = strategic_redeployment_rows.saturating_add(1);
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_current_manpower,
            current_manpower,
            0,
            COMPACT_MAX_DIVISION_CURRENT_MANPOWER,
            "current_manpower",
        );
        if has_current_manpower {
            current_manpower_rows = current_manpower_rows.saturating_add(1);
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_cumulative_losses,
            cumulative_losses,
            0,
            COMPACT_MAX_DIVISION_CUMULATIVE_LOSSES,
            "cumulative_losses",
        );
        if has_cumulative_losses {
            cumulative_losses_rows = cumulative_losses_rows.saturating_add(1);
        }
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_max_organisation,
            max_organisation_raw,
            100,
            COMPACT_MAX_DIVISION_ORGANISATION_RAW,
            "max_organisation_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_counter_supply_status,
            counter_supply_status_raw,
            0,
            COMPACT_MAX_DIVISION_COUNTER_SUPPLY_STATUS_RAW,
            "counter_supply_status_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_dig_in,
            dig_in_raw,
            0,
            COMPACT_MAX_DIVISION_DIG_IN_RAW,
            "dig_in_raw",
        );
        validate_optional_compact_u32(
            summary,
            offset,
            row_index,
            has_dig_in_cap,
            dig_in_cap_raw,
            0,
            COMPACT_MAX_DIVISION_DIG_IN_RAW,
            "dig_in_cap_raw",
        );
        let has_order_army = (flags & COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY) != 0;
        let has_order_group = (flags & COMPACT_DIVISION_ROW_FLAG_ORDER_ARMY_GROUP) != 0;
        let has_order_unresolved = (flags & COMPACT_DIVISION_ROW_FLAG_ORDER_UNRESOLVED) != 0;
        if (has_order_army as u32 + has_order_group as u32 + has_order_unresolved as u32) > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} has conflicting order-link flags"
            ));
        }
        if has_order_army {
            order_army_rows = order_army_rows.saturating_add(1);
            if row_reserved2 == 0 || row_reserved2 > COMPACT_MAX_DIVISION_ORDER_INSTANCE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} has invalid order instance id {row_reserved2}"
                ));
            }
        } else {
            if has_order_group {
                order_group_rows = order_group_rows.saturating_add(1);
            }
            if has_order_unresolved {
                order_unresolved_rows = order_unresolved_rows.saturating_add(1);
            }
            if row_reserved2 != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: divisions row {row_index} has order instance id without army-order flag"
                ));
            }
        }
    }
    add_compact_domain_data_unit(summary, 20, "division_deleted_rows", deleted_rows);
    add_compact_domain_data_unit(summary, 20, "division_moving_rows", moving_rows);
    add_compact_domain_data_unit(summary, 20, "division_in_combat_rows", combat_rows);
    add_compact_domain_data_unit(summary, 20, "division_template_rows", template_rows);
    add_compact_domain_data_unit(summary, 20, "division_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 20, "division_army_rows", army_rows);
    add_compact_domain_data_unit(summary, 20, "division_org_strength_rows", org_strength_rows);
    add_compact_domain_data_unit(summary, 20, "division_order_army_rows", order_army_rows);
    add_compact_domain_data_unit(
        summary,
        20,
        "division_order_army_group_rows",
        order_group_rows,
    );
    add_compact_domain_data_unit(
        summary,
        20,
        "division_order_unresolved_rows",
        order_unresolved_rows,
    );
    add_compact_domain_data_unit(
        summary,
        20,
        "division_disrupted_supply_rows",
        disrupted_supply_rows,
    );
    add_compact_domain_data_unit(
        summary,
        20,
        "division_strategic_redeployment_rows",
        strategic_redeployment_rows,
    );
    add_compact_domain_data_unit(
        summary,
        20,
        "division_current_manpower_rows",
        current_manpower_rows,
    );
    add_compact_domain_data_unit(
        summary,
        20,
        "division_cumulative_losses_rows",
        cumulative_losses_rows,
    );
}

fn decode_compact_division_equipment_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let root_rows = read_u32_le(payload, 24) as usize;
    let observed_division_rows = read_u32_le(payload, 28) as usize;
    let equipment_entry_count = read_u32_le(payload, 32) as usize;
    let observed_equipment_rows = read_u32_le(payload, 36) as usize;
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    let expected_len = 40usize
        .saturating_add(row_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ROW_LEN))
        .saturating_add(
            equipment_entry_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN),
        );
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    let rowless_unknown = row_count == 0
        && (status_flags & (COMPACT_STATUS_PARTIAL_DATA | COMPACT_STATUS_SOURCE_UNAVAILABLE)) != 0
        && (observed_division_rows != 0 || observed_equipment_rows != 0);
    if !delta_data && !rowless_unknown && row_count != observed_division_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment full row_count {row_count} != observed_division_rows {observed_division_rows}"
        ));
    }
    if observed_division_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment observed_division_rows {observed_division_rows} > root_rows {root_rows}"
        ));
    }
    if !delta_data && !rowless_unknown && equipment_entry_count != observed_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment full equipment_entry_count {equipment_entry_count} != observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if equipment_entry_count > observed_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment emitted entries {equipment_entry_count} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&26) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 26, "division_equipment_rows", row_count);
    add_compact_domain_data_unit(summary, 26, "division_equipment_root_rows", root_rows);
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_skipped_root_rows",
        root_rows.saturating_sub(observed_division_rows),
    );
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_observed_division_rows",
        observed_division_rows,
    );
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_entry_rows",
        equipment_entry_count,
    );
    add_compact_domain_data_unit(
        summary,
        26,
        "division_equipment_observed_entry_rows",
        observed_equipment_rows,
    );

    let mut cursor = 40usize;
    let mut seen_divisions = BTreeSet::new();
    let mut declared_entries = 0usize;
    let mut deleted_rows = 0usize;
    let mut parent_ranges = Vec::with_capacity(row_count);
    for row_index in 0..row_count {
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let div_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let template_id = read_u32_le(payload, cursor + 12);
        let entry_first = read_u32_le(payload, cursor + 16) as usize;
        let entry_count = read_u32_le(payload, cursor + 20) as usize;
        cursor += COMPACT_DIVISION_EQUIPMENT_ROW_LEN;

        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_MAX_DIVISION_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if div_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has zero div_id"
            ));
        } else if !seen_divisions.insert((owner_tag_db_id, div_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} duplicates owner/division {owner_tag_db_id}:{div_id}"
            ));
        }
        if flags
            & !(COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE
                | COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: division equipment row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = (flags & COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED) != 0;
        let has_template = (flags & COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_TEMPLATE) != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != COMPACT_DIVISION_EQUIPMENT_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has non-delete flags"
                ));
            }
            if template_id != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has template_id"
                ));
            }
            if entry_count != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} has entries"
                ));
            }
            if entry_first > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment delete row {row_index} entry_first {entry_first} exceeds {equipment_entry_count}"
                ));
            }
        } else {
            validate_optional_compact_u32(
                summary,
                offset,
                row_index,
                has_template,
                template_id,
                1,
                COMPACT_MAX_DIVISION_TEMPLATE_ID,
                "template_id",
            );
            if entry_first.saturating_add(entry_count) > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment row {row_index} entry range {entry_first}+{entry_count} exceeds {equipment_entry_count}"
                ));
            }
        }
        declared_entries = declared_entries.saturating_add(entry_count);
        parent_ranges.push((
            row_index,
            owner_tag_db_id,
            div_id,
            entry_first,
            entry_count,
            deleted,
        ));
    }
    if declared_entries != equipment_entry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: division equipment entry_count {equipment_entry_count} != declared {declared_entries}"
        ));
    }
    add_compact_domain_data_unit(summary, 26, "division_equipment_deleted_rows", deleted_rows);

    let mut seen_entries = BTreeSet::new();
    for (row_index, owner_tag_db_id, div_id, entry_first, entry_count, deleted) in parent_ranges {
        if deleted {
            continue;
        }
        for child_index in entry_first..entry_first.saturating_add(entry_count) {
            let child_offset = 40usize
                .saturating_add(row_count.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ROW_LEN))
                .saturating_add(
                    child_index.saturating_mul(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN),
                );
            if child_offset.saturating_add(COMPACT_DIVISION_EQUIPMENT_ENTRY_ROW_LEN) > payload.len()
            {
                continue;
            }
            let variant_id = read_u32_le(payload, child_offset);
            let reserved = read_u32_le(payload, child_offset + 4);
            let amount_raw = read_i64_le(payload, child_offset + 8);
            if variant_id == 0 || variant_id > COMPACT_MAX_DIVISION_EQUIPMENT_VARIANT_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} has invalid variant_id {variant_id}"
                ));
            }
            if amount_raw <= 0 || amount_raw > COMPACT_MAX_DIVISION_EQUIPMENT_AMOUNT_RAW {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if reserved != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment entry row {child_index} reserved field is nonzero"
                ));
            }
            if !seen_entries.insert((owner_tag_db_id, div_id, variant_id)) {
                summary.parse_errors.push(format!(
                    "offset {offset}: division equipment row {row_index} duplicates variant {variant_id}"
                ));
            }
        }
    }
}

fn validate_optional_compact_u32(
    summary: &mut DomainDataDecodeSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    min_value: u32,
    max_value: u32,
    field: &str,
) {
    if present {
        if value < min_value || value > max_value {
            summary.parse_errors.push(format!(
                "offset {offset}: divisions row {row_index} {field} value {value} out of range"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: divisions row {row_index} has {field} without flag"
        ));
    }
}

fn valid_compact_province_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_MAX_DIVISION_PROVINCE_ID
}

fn decode_compact_army_orders_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: army order data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_order_rows = read_u32_le(payload, 28) as usize;
    let target_state_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    add_decoder_status(summary, status_flags);
    let expected_len = 40usize
        .saturating_add(row_count.saturating_mul(COMPACT_ARMY_ORDER_ROW_LEN))
        .saturating_add(target_state_count.saturating_mul(COMPACT_ARMY_ORDER_TARGET_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: army order data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: army order observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&27) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 27, "army_order_rows", row_count);
    add_compact_domain_data_unit(summary, 27, "army_order_root_rows", root_rows);
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_observed_armies",
        observed_army_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_observed_orders",
        observed_order_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_target_state_rows",
        target_state_count,
    );
    add_compact_domain_data_unit(summary, 27, "army_order_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_orders = BTreeSet::new();
    let mut declared_targets = 0usize;
    let mut exact_rows = 0usize;
    let mut unsupported_rows = 0usize;
    let mut overflow_rows = 0usize;
    let mut validation_failed_rows = 0usize;
    for row_index in 0..row_count {
        let army_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 12);
        let slot_index = read_u32_le(payload, cursor + 16);
        let order_type = read_i32_le(payload, cursor + 20);
        let target_first = read_u32_le(payload, cursor + 24) as usize;
        let target_count = read_u32_le(payload, cursor + 28) as usize;
        let target_state_declared_count = read_u32_le(payload, cursor + 32) as usize;
        let target_status_code = read_u32_le(payload, cursor + 36);
        let owner_kind = read_u32_le(payload, cursor + 40);
        let row_reserved1 = read_u32_le(payload, cursor + 44);
        cursor += COMPACT_ARMY_ORDER_ROW_LEN;

        let deleted = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_DELETED) != 0;
        let has_target_states = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES) != 0;
        let has_target_state_count = (flags & COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT) != 0;
        if army_id == 0 || army_id > COMPACT_MAX_ARMY_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has invalid army_id {army_id}"
            ));
        }
        if !matches!(
            owner_kind,
            COMPACT_ARMY_ORDER_OWNER_KIND_ARMY | COMPACT_ARMY_ORDER_OWNER_KIND_ARMY_GROUP
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has invalid owner_kind {owner_kind}"
            ));
        } else if !seen_orders.insert((owner_kind, army_id, generation, slot_index, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} duplicates typed owner {owner_kind}:{army_id}:{generation}:{slot_index}"
            ));
        }
        if flags
            & !(COMPACT_ARMY_ORDER_ROW_FLAG_DELETED
                | COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATES
                | COMPACT_ARMY_ORDER_ROW_FLAG_TARGET_STATE_COUNT)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has unknown flags {flags}"
            ));
        }
        if slot_index >= COMPACT_ARMY_ORDER_MAX_SLOT_INDEX {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has invalid slot_index {slot_index}"
            ));
        }
        if order_type < -1_000 || order_type > 1_000 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has suspicious order_type {order_type}"
            ));
        }
        if deleted {
            if target_first != 0
                || target_count != 0
                || target_state_declared_count != 0
                || target_status_code != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order deleted row {row_index} has live target fields"
                ));
            }
        } else {
            match target_status_code {
                0 => {}
                1 => exact_rows = exact_rows.saturating_add(1),
                2 => unsupported_rows = unsupported_rows.saturating_add(1),
                3 => overflow_rows = overflow_rows.saturating_add(1),
                4 => validation_failed_rows = validation_failed_rows.saturating_add(1),
                other => summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} has unknown target_status_code {other}"
                )),
            }
        }
        if !has_target_states && target_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_count without target-states flag"
            ));
        }
        if has_target_states {
            let Some(target_end) = target_first.checked_add(target_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} target range overflows"
                ));
                continue;
            };
            if target_end > target_state_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: army order row {row_index} target range {target_first}..{target_end} exceeds {target_state_count}"
                ));
            }
            declared_targets = declared_targets.saturating_add(target_count);
        } else if target_first != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_first without target-states flag"
            ));
        }
        if !has_target_state_count && target_state_declared_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} has target_state_count without flag"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order row {row_index} reserved field is nonzero"
            ));
        }
    }
    if declared_targets != target_state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: army order target_state_count {target_state_count} != declared {declared_targets}"
        ));
    }
    add_compact_domain_data_unit(summary, 27, "army_order_exact_target_rows", exact_rows);
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_unsupported_target_rows",
        unsupported_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_overflow_target_rows",
        overflow_rows,
    );
    add_compact_domain_data_unit(
        summary,
        27,
        "army_order_validation_failed_target_rows",
        validation_failed_rows,
    );

    let mut seen_target_states = BTreeSet::new();
    for row_index in 0..target_state_count {
        let state_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_ARMY_ORDER_TARGET_ROW_LEN;
        if !valid_compact_army_order_state_id(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !seen_target_states.insert((row_index, state_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} duplicates state_id {state_id}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: army order target row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn valid_compact_army_order_state_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_ARMY_ORDER_MAX_STATE_ID
}

fn decode_compact_oob_command_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let root_rows = read_u32_le(payload, 20) as usize;
    let observed_army_rows = read_u32_le(payload, 24) as usize;
    let observed_army_group_rows = read_u32_le(payload, 28) as usize;
    let member_count = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let observed_fleet_rows = read_u32_le(payload, 40) as usize;
    let observed_task_force_rows = read_u32_le(payload, 44) as usize;
    add_decoder_status(summary, status_flags);
    if member_count > COMPACT_OOB_MAX_MEMBER_ROWS {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} exceeds limit {COMPACT_OOB_MAX_MEMBER_ROWS}"
        ));
    }
    let expected_len = 48usize
        .saturating_add(row_count.saturating_mul(COMPACT_OOB_ROW_LEN))
        .saturating_add(member_count.saturating_mul(COMPACT_OOB_MEMBER_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if observed_army_rows > root_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command observed_army_rows {observed_army_rows} > root_rows {root_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&28) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 28, "oob_command_rows", row_count);
    add_compact_domain_data_unit(summary, 28, "oob_command_root_rows", root_rows);
    add_compact_domain_data_unit(summary, 28, "oob_observed_army_rows", observed_army_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_observed_army_group_rows",
        observed_army_group_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_rows", member_count);
    add_compact_domain_data_unit(summary, 28, "oob_deleted_rows", deleted_rows);
    add_compact_domain_data_unit(summary, 28, "oob_observed_fleet_rows", observed_fleet_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_observed_task_force_rows",
        observed_task_force_rows,
    );

    let mut cursor = 48usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_members = 0usize;
    let mut emitted_army_rows = 0usize;
    let mut emitted_army_group_rows = 0usize;
    let mut emitted_fleet_rows = 0usize;
    let mut emitted_task_force_rows = 0usize;
    let mut member_division_rows = 0usize;
    let mut member_army_rows = 0usize;
    let mut member_task_force_rows = 0usize;
    let mut member_ship_rows = 0usize;
    let mut leader_rows = 0usize;
    let mut leader_skill_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut location_rows = 0usize;
    let mut engine_count_rows = 0usize;
    let mut missing_count_rows = 0usize;
    for row_index in 0..row_count {
        let kind = read_u32_le(payload, cursor);
        let object_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let parent_id = read_u32_le(payload, cursor + 12);
        let owner_tag_db_id = read_u32_le(payload, cursor + 16);
        let leader_id = read_u32_le(payload, cursor + 20);
        let leader_skill = read_u32_le(payload, cursor + 24);
        let member_first = read_u32_le(payload, cursor + 28) as usize;
        let row_member_count = read_u32_le(payload, cursor + 32) as usize;
        let logical_member_count = read_u32_le(payload, cursor + 36) as usize;
        let member_count_engine = read_u32_le(payload, cursor + 40);
        let missing_member_count = read_u32_le(payload, cursor + 44);
        let leader_trait_count = read_u32_le(payload, cursor + 48);
        let parent_status = read_u32_le(payload, cursor + 52);
        let member_status = read_u32_le(payload, cursor + 56);
        let owner_status = read_u32_le(payload, cursor + 60);
        let leader_status = read_u32_le(payload, cursor + 64);
        let leader_trait_status = read_u32_le(payload, cursor + 68);
        let location_province_id = read_u32_le(payload, cursor + 72);
        let row_reserved1 = read_u32_le(payload, cursor + 76);
        cursor += COMPACT_OOB_ROW_LEN;

        let deleted = (flags & COMPACT_OOB_ROW_FLAG_DELETED) != 0;
        if !matches!(
            kind,
            COMPACT_OOB_KIND_ARMY
                | COMPACT_OOB_KIND_ARMY_GROUP
                | COMPACT_OOB_KIND_FLEET
                | COMPACT_OOB_KIND_TASK_FORCE
        ) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid kind {kind}"
            ));
        } else if kind == COMPACT_OOB_KIND_ARMY {
            emitted_army_rows = emitted_army_rows.saturating_add(1);
        } else if kind == COMPACT_OOB_KIND_ARMY_GROUP {
            emitted_army_group_rows = emitted_army_group_rows.saturating_add(1);
        } else if kind == COMPACT_OOB_KIND_FLEET {
            emitted_fleet_rows = emitted_fleet_rows.saturating_add(1);
        } else {
            emitted_task_force_rows = emitted_task_force_rows.saturating_add(1);
        }
        if !valid_compact_oob_id(object_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid object_id {object_id}"
            ));
        } else if !seen_rows.insert((kind, object_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} duplicates {kind}:{object_id}"
            ));
        }
        if flags
            & !(COMPACT_OOB_ROW_FLAG_DELETED
                | COMPACT_OOB_ROW_FLAG_PARENT
                | COMPACT_OOB_ROW_FLAG_OWNER
                | COMPACT_OOB_ROW_FLAG_LEADER
                | COMPACT_OOB_ROW_FLAG_LEADER_SKILL
                | COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE
                | COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT
                | COMPACT_OOB_ROW_FLAG_LOCATION)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted {
            if parent_id != 0
                || owner_tag_db_id != 0
                || leader_id != 0
                || leader_skill != 0
                || row_member_count != 0
                || logical_member_count != 0
                || member_count_engine != 0
                || missing_member_count != 0
                || leader_trait_count != 0
                || location_province_id != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command deleted row {row_index} has live fields"
                ));
            }
        }
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_PARENT) != 0,
            parent_id,
            "parent_id",
        );
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0,
            owner_tag_db_id,
            "owner_tag_db_id",
        );
        validate_compact_oob_optional_id(
            summary,
            offset,
            row_index,
            (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0,
            leader_id,
            "leader_id",
        );
        if (flags & COMPACT_OOB_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER) != 0 {
            leader_rows = leader_rows.saturating_add(1);
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LOCATION) != 0 {
            location_rows = location_rows.saturating_add(1);
            if location_province_id == 0
                || location_province_id > COMPACT_OOB_MAX_LOCATION_PROVINCE_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid location_province_id {location_province_id}"
                ));
            }
        } else if location_province_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has location_province_id without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_LEADER_SKILL) != 0 {
            leader_skill_rows = leader_skill_rows.saturating_add(1);
            if leader_skill > COMPACT_OOB_MAX_LEADER_SKILL {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid leader_skill {leader_skill}"
                ));
            }
        } else if leader_skill != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has leader_skill without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MEMBER_COUNT_ENGINE) != 0 {
            engine_count_rows = engine_count_rows.saturating_add(1);
        } else if member_count_engine != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has engine member count without flag"
            ));
        }
        if (flags & COMPACT_OOB_ROW_FLAG_MISSING_MEMBER_COUNT) != 0 {
            missing_count_rows = missing_count_rows.saturating_add(1);
        } else if missing_member_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has missing member count without flag"
            ));
        }
        for (status_name, status_value) in [
            ("parent_status", parent_status),
            ("member_status", member_status),
            ("owner_status", owner_status),
            ("leader_status", leader_status),
            ("leader_trait_status", leader_trait_status),
        ] {
            if status_value > 5 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has invalid {status_name} {status_value}"
                ));
            }
        }
        if row_member_count == 0 {
            if member_first != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} has member_first without members"
                ));
            }
        } else {
            let Some(member_end) = member_first.checked_add(row_member_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range overflows"
                ));
                continue;
            };
            if member_end > member_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: oob command row {row_index} member range {member_first}..{member_end} exceeds {member_count}"
                ));
            }
            declared_members = declared_members.saturating_add(row_member_count);
            if kind == COMPACT_OOB_KIND_ARMY {
                member_division_rows = member_division_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_ARMY_GROUP {
                member_army_rows = member_army_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_FLEET {
                member_task_force_rows = member_task_force_rows.saturating_add(row_member_count);
            } else if kind == COMPACT_OOB_KIND_TASK_FORCE {
                member_ship_rows = member_ship_rows.saturating_add(row_member_count);
            }
        }
        if member_status != 3 && row_member_count != logical_member_count {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} member_count {row_member_count} != logical_member_count {logical_member_count}"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} reserved field is nonzero"
            ));
        }
    }
    if declared_members != member_count {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command member_count {member_count} != declared {declared_members}"
        ));
    }
    add_compact_domain_data_unit(summary, 28, "oob_army_rows", emitted_army_rows);
    add_compact_domain_data_unit(summary, 28, "oob_army_group_rows", emitted_army_group_rows);
    add_compact_domain_data_unit(summary, 28, "oob_fleet_rows", emitted_fleet_rows);
    add_compact_domain_data_unit(summary, 28, "oob_task_force_rows", emitted_task_force_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_member_division_rows",
        member_division_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_army_rows", member_army_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_member_task_force_rows",
        member_task_force_rows,
    );
    add_compact_domain_data_unit(summary, 28, "oob_member_ship_rows", member_ship_rows);
    add_compact_domain_data_unit(summary, 28, "oob_leader_rows", leader_rows);
    add_compact_domain_data_unit(summary, 28, "oob_leader_skill_rows", leader_skill_rows);
    add_compact_domain_data_unit(summary, 28, "oob_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 28, "oob_location_rows", location_rows);
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_engine_member_count_rows",
        engine_count_rows,
    );
    add_compact_domain_data_unit(
        summary,
        28,
        "oob_missing_member_count_rows",
        missing_count_rows,
    );

    let mut seen_members = BTreeSet::new();
    for row_index in 0..member_count {
        let member_id = read_u32_le(payload, cursor);
        let row_reserved = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_OOB_MEMBER_ROW_LEN;
        if !valid_compact_oob_id(member_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} has invalid member_id {member_id}"
            ));
        }
        if !seen_members.insert((row_index, member_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} duplicates member_id {member_id}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command member row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn valid_compact_oob_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_OOB_MAX_ID
}

fn validate_compact_oob_optional_id(
    summary: &mut DomainDataDecodeSummary,
    offset: usize,
    row_index: usize,
    present: bool,
    value: u32,
    field: &str,
) {
    if present {
        if !valid_compact_oob_id(value) {
            summary.parse_errors.push(format!(
                "offset {offset}: oob command row {row_index} has invalid {field} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: oob command row {row_index} has {field} without flag"
        ));
    }
}

fn decode_compact_ships_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 40 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload is {} byte(s), expected at least 40",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_ship_rows = read_u32_le(payload, 20) as usize;
    let observed_task_force_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let reserved0 = read_u32_le(payload, 36);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: ships deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_ship_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships row_count {row_count} > observed_ship_rows {observed_ship_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: ships scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: ships header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&29) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 29, "ship_rows", row_count);
    add_compact_domain_data_unit(summary, 29, "ship_observed_ship_rows", observed_ship_rows);
    add_compact_domain_data_unit(
        summary,
        29,
        "ship_observed_task_force_rows",
        observed_task_force_rows,
    );
    add_compact_domain_data_unit(summary, 29, "ship_deleted_rows", deleted_rows);

    let mut cursor = 40usize;
    let mut seen_rows = BTreeSet::new();
    let mut task_force_rows = 0usize;
    let mut owner_rows = 0usize;
    let mut definition_rows = 0usize;
    for row_index in 0..row_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, COMPACT_SHIP_ROW_BASE_LEN, "ships row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let instance_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let task_force_id = read_u32_le(payload, cursor + 8);
        let category_enum = read_u32_le(payload, cursor + 12);
        let strength_raw = read_u32_le(payload, cursor + 16);
        let organisation_raw = read_u32_le(payload, cursor + 20);
        let experience_raw = read_u32_le(payload, cursor + 24);
        let sunk_convoys = read_u32_le(payload, cursor + 28);
        let owner_tag_db_id = read_u32_le(payload, cursor + 32);
        let row_reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_SHIP_ROW_BASE_LEN;
        let definition_len = if cursor.saturating_add(2) <= payload.len() {
            read_u16_le(payload, cursor) as usize
        } else {
            0
        };
        if let Err(err) = skip_compact_string(payload, &mut cursor, "ships definition") {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }

        let deleted = (flags & COMPACT_SHIP_ROW_FLAG_DELETED) != 0;
        if !valid_compact_ship_id(instance_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid instance_id {instance_id}"
            ));
        } else if !seen_rows.insert((instance_id, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} duplicates instance_id {instance_id}"
            ));
        }
        if flags
            & !(COMPACT_SHIP_ROW_FLAG_DELETED
                | COMPACT_SHIP_ROW_FLAG_TASK_FORCE
                | COMPACT_SHIP_ROW_FLAG_DEFINITION
                | COMPACT_SHIP_ROW_FLAG_OWNER)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_TASK_FORCE) != 0 {
            task_force_rows = task_force_rows.saturating_add(1);
            if !valid_compact_oob_id(task_force_id) {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid task_force_id {task_force_id}"
                ));
            }
        } else if task_force_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has task_force_id without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_DEFINITION) != 0 {
            definition_rows = definition_rows.saturating_add(1);
            if definition_len == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has definition flag without definition"
                ));
            }
        } else if definition_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has definition without flag"
            ));
        }
        if (flags & COMPACT_SHIP_ROW_FLAG_OWNER) != 0 {
            owner_rows = owner_rows.saturating_add(1);
            if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_SHIP_MAX_OWNER_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: ships row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                ));
            }
        } else if owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has owner_tag_db_id without flag"
            ));
        }
        if category_enum > COMPACT_SHIP_MAX_CATEGORY {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid category_enum {category_enum}"
            ));
        }
        if strength_raw > COMPACT_SHIP_MAX_STRENGTH_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid strength_raw {strength_raw}"
            ));
        }
        if organisation_raw > COMPACT_SHIP_MAX_ORGANISATION_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid organisation_raw {organisation_raw}"
            ));
        }
        if experience_raw > COMPACT_SHIP_MAX_EXPERIENCE_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} has invalid experience_raw {experience_raw}"
            ));
        }
        if deleted
            && (task_force_id != 0
                || category_enum != 0
                || strength_raw != 0
                || organisation_raw != 0
                || experience_raw != 0
                || sunk_convoys != 0
                || owner_tag_db_id != 0
                || definition_len != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: ships deleted row {row_index} has live fields"
            ));
        }
        if row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: ships row {row_index} reserved field is nonzero"
            ));
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: ships data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(summary, 29, "ship_task_force_rows", task_force_rows);
    add_compact_domain_data_unit(summary, 29, "ship_owner_rows", owner_rows);
    add_compact_domain_data_unit(summary, 29, "ship_definition_rows", definition_rows);
}

fn valid_compact_ship_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_SHIP_MAX_ID
}

fn decode_compact_air_wings_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let deleted_rows = read_u32_le(payload, 24) as usize;
    let scale_raw = read_u32_le(payload, 28);
    let reserved0 = read_u32_le(payload, 32);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_wing_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings row_count {row_count} > observed_air_wing_rows {observed_air_wing_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&30) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 30, "air_wing_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        30,
        "air_wing_observed_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(summary, 30, "air_wing_deleted_rows", deleted_rows);

    let mut cursor = 36usize;
    let mut seen_rows = BTreeSet::new();
    let mut mission_region_rows = 0usize;
    for row_index in 0..row_count {
        if let Err(err) =
            require_compact_remaining(payload, cursor, COMPACT_AIR_WING_ROW_LEN, "air wings row")
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let count = read_u32_le(payload, cursor + 20);
        let mission_mask = read_u32_le(payload, cursor + 24);
        let mission_region = read_u32_le(payload, cursor + 28);
        let effective_planes = read_u32_le(payload, cursor + 32);
        let reserved1 = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_AIR_WING_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_WING_ROW_FLAG_DELETED) != 0;
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > COMPACT_AIR_WING_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((owner_tag_db_id, wing_id, generation, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} duplicates owner/wing/generation"
            ));
        }
        if flags & !(COMPACT_AIR_WING_ROW_FLAG_DELETED | COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & COMPACT_AIR_WING_ROW_FLAG_MISSION_REGION) != 0 {
            mission_region_rows = mission_region_rows.saturating_add(1);
            if mission_region == 0 || mission_region > COMPACT_AIR_WING_MAX_MISSION_REGION_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wings row {row_index} has invalid mission_region {mission_region}"
                ));
            }
        } else if mission_region != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has mission_region without flag"
            ));
        }
        if count > COMPACT_AIR_WING_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid count {count}"
            ));
        }
        if effective_planes > COMPACT_AIR_WING_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} has invalid effective_planes {effective_planes}"
            ));
        }
        if deleted
            && (count != 0 || mission_mask != 0 || mission_region != 0 || effective_planes != 0)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings deleted row {row_index} has live fields"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wings row {row_index} reserved field is nonzero"
            ));
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air wings data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        30,
        "air_wing_mission_region_rows",
        mission_region_rows,
    );
}

fn decode_compact_air_region_activity_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_bucket_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let reserved0 = read_u32_le(payload, 32);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_bucket_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity row_count {row_count} > observed_bucket_rows {observed_bucket_rows}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity header reserved field is nonzero"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&31) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 31, "air_region_activity_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_observed_bucket_rows",
        observed_bucket_rows,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_deleted_rows",
        deleted_rows,
    );

    let mut cursor = 36usize;
    let mut seen_rows = BTreeSet::new();
    let mut wing_count_sum = 0usize;
    let mut plane_count_sum = 0usize;
    let mut effective_planes_sum = 0usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_REGION_ACTIVITY_ROW_LEN,
            "air region activity row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let region_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let mission_mask = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        let wing_count = read_u32_le(payload, cursor + 16);
        let plane_count = read_u32_le(payload, cursor + 20);
        let effective_planes = read_u32_le(payload, cursor + 24);
        let reserved1 = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_AIR_REGION_ACTIVITY_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED) != 0;
        if region_id == 0 || region_id > COMPACT_AIR_REGION_ACTIVITY_MAX_REGION_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid region_id {region_id}"
            ));
        }
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_REGION_ACTIVITY_MAX_OWNER_TAG_DB_ID
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if !seen_rows.insert((region_id, owner_tag_db_id, mission_mask)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} duplicates region/owner/mission"
            ));
        }
        if flags & !COMPACT_AIR_REGION_ACTIVITY_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has unknown flags {flags}"
            ));
        }
        if wing_count > COMPACT_AIR_REGION_ACTIVITY_MAX_WING_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid wing_count {wing_count}"
            ));
        }
        if plane_count > COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid plane_count {plane_count}"
            ));
        }
        if effective_planes > COMPACT_AIR_REGION_ACTIVITY_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} has invalid effective_planes {effective_planes}"
            ));
        }
        if deleted && (wing_count != 0 || plane_count != 0 || effective_planes != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity deleted row {row_index} has live counts"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air region activity row {row_index} reserved field is nonzero"
            ));
        }
        wing_count_sum = wing_count_sum.saturating_add(wing_count as usize);
        plane_count_sum = plane_count_sum.saturating_add(plane_count as usize);
        effective_planes_sum = effective_planes_sum.saturating_add(effective_planes as usize);
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: air region activity data payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_wing_count_sum",
        wing_count_sum,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_plane_count_sum",
        plane_count_sum,
    );
    add_compact_domain_data_unit(
        summary,
        31,
        "air_region_activity_effective_planes_sum",
        effective_planes_sum,
    );
}

fn decode_compact_air_wing_loadouts_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts data payload is {} byte(s), expected at least 44",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_equipment_rows = read_u32_le(payload, 24) as usize;
    let emitted_equipment_rows = read_u32_le(payload, 28) as usize;
    let deleted_rows = read_u32_le(payload, 32) as usize;
    let scale_raw = read_u32_le(payload, 36);
    let reserved0 = read_u32_le(payload, 40);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_wing_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts row_count {row_count} > observed_air_wing_rows {observed_air_wing_rows}"
        ));
    }
    if emitted_equipment_rows > observed_equipment_rows
        && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts emitted_equipment_rows {emitted_equipment_rows} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts header reserved field is nonzero"
        ));
    }
    let expected_min =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_WING_LOADOUT_ROW_LEN));
    if payload.len() < expected_min {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts data payload is {} byte(s), expected at least {expected_min}",
            payload.len()
        ));
        return;
    }
    let remaining_after_rows = payload.len().saturating_sub(expected_min);
    if remaining_after_rows % COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts equipment table has non-row trailing bytes"
        ));
        return;
    }
    let equipment_entry_count = remaining_after_rows / COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN;
    if equipment_entry_count != emitted_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts emitted_equipment_rows {emitted_equipment_rows} != payload entries {equipment_entry_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&32) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_observed_equipment_rows",
        observed_equipment_rows,
    );
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_equipment_rows",
        equipment_entry_count,
    );
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_deleted_rows", deleted_rows);

    let mut cursor = 44usize;
    let mut seen_rows = BTreeSet::new();
    let mut declared_entries = 0usize;
    let mut air_base_rows = 0usize;
    let mut equipment_rows_by_wing = BTreeMap::new();
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_WING_LOADOUT_ROW_LEN,
            "air wing loadouts row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let air_base_id = read_u32_le(payload, cursor + 20);
        let entry_first = read_u32_le(payload, cursor + 24) as usize;
        let entry_count = read_u32_le(payload, cursor + 28) as usize;
        cursor += COMPACT_AIR_WING_LOADOUT_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED) != 0;
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > COMPACT_AIR_WING_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((owner_tag_db_id, wing_id, generation, deleted)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} duplicates owner/wing/generation"
            ));
        }
        if flags & !COMPACT_AIR_WING_LOADOUT_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air wing loadouts row {row_index} has unknown flags {flags}"
            ));
        }
        if deleted {
            if air_base_id != 0 || entry_first != 0 || entry_count != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts deleted row {row_index} has live fields"
                ));
            }
        } else {
            if air_base_id == 0 || air_base_id > COMPACT_AIR_WING_LOADOUT_MAX_AIR_BASE_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} has invalid air_base_id {air_base_id}"
                ));
            } else {
                air_base_rows = air_base_rows.saturating_add(1);
            }
            let Some(entry_end) = entry_first.checked_add(entry_count) else {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} equipment range overflows"
                ));
                continue;
            };
            if entry_end > equipment_entry_count {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} equipment range {entry_first}+{entry_count} exceeds {equipment_entry_count}"
                ));
            }
            declared_entries = declared_entries.saturating_add(entry_count);
            equipment_rows_by_wing.insert(
                row_index,
                (
                    owner_tag_db_id,
                    wing_id,
                    generation,
                    entry_first,
                    entry_count,
                ),
            );
        }
    }
    if declared_entries != equipment_entry_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air wing loadouts equipment_entry_count {equipment_entry_count} != declared {declared_entries}"
        ));
    }
    add_compact_domain_data_unit(summary, 32, "air_wing_loadout_air_base_rows", air_base_rows);

    let entry_base =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_WING_LOADOUT_ROW_LEN));
    let mut seen_entries = BTreeSet::new();
    let mut amount_raw_sum = 0i128;
    for (row_index, (owner_tag_db_id, wing_id, generation, entry_first, entry_count)) in
        equipment_rows_by_wing
    {
        for child_index in entry_first..entry_first.saturating_add(entry_count) {
            let child_offset = entry_base
                .saturating_add(child_index.saturating_mul(COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN));
            if let Err(err) = require_compact_remaining(
                payload,
                child_offset,
                COMPACT_AIR_WING_LOADOUT_ENTRY_ROW_LEN,
                "air wing loadouts equipment row",
            ) {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
            let variant_id = read_u32_le(payload, child_offset);
            let row_reserved = read_u32_le(payload, child_offset + 4);
            let amount_raw = read_i64_le(payload, child_offset + 8);
            if variant_id == 0 || variant_id > COMPACT_AIR_WING_LOADOUT_MAX_VARIANT_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} has invalid variant_id {variant_id}"
                ));
            }
            if !(0..=COMPACT_AIR_WING_LOADOUT_MAX_AMOUNT_RAW).contains(&amount_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if row_reserved != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts equipment row {child_index} reserved field is nonzero"
                ));
            }
            if !seen_entries.insert((owner_tag_db_id, wing_id, generation, variant_id)) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air wing loadouts row {row_index} duplicates variant {variant_id}"
                ));
            }
            amount_raw_sum = amount_raw_sum.saturating_add(amount_raw as i128);
        }
    }
    add_compact_domain_data_unit(
        summary,
        32,
        "air_wing_loadout_amount_raw_sum",
        amount_raw_sum.max(0).min(u64::MAX as i128) as usize,
    );
}

fn decode_compact_air_combat_losses_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live data payload is {} byte(s), expected at least {COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN}",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_equipment_rows = read_u32_le(payload, 24) as usize;
    let loss_rows = read_u32_le(payload, 28) as usize;
    let reinforcement_rows = read_u32_le(payload, 32) as usize;
    let deleted_rows = read_u32_le(payload, 36) as usize;
    let scale_raw = read_u32_le(payload, 40);
    let reserved0 = read_u32_le(payload, 44);
    add_decoder_status(summary, status_flags);

    if changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live changed_rows {changed_rows} != row_count {row_count}"
        ));
    }
    if loss_rows > row_count || reinforcement_rows > row_count || deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live counters exceed row_count {row_count}"
        ));
    }
    if scale_raw != COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE as u32 {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live scale_raw {scale_raw} != {COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE}"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live header reserved field is nonzero"
        ));
    }
    let legacy_len = COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_AIR_STRENGTH_DELTA_ROW_LEN));
    if payload.len() < legacy_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live data payload is {} byte(s), expected at least {legacy_len}",
            payload.len()
        ));
        return;
    }

    if let Some(domain) = summary.per_domain.get_mut(&42) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 42, "air_strength_delta_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_observed_equipment_rows",
        observed_equipment_rows,
    );
    add_compact_domain_data_unit(summary, 42, "air_strength_loss_rows", loss_rows);
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_reinforcement_rows",
        reinforcement_rows,
    );
    add_compact_domain_data_unit(summary, 42, "air_strength_deleted_rows", deleted_rows);

    let mut cursor = COMPACT_AIR_STRENGTH_DELTA_HEADER_LEN;
    let mut seen_rows = BTreeSet::new();
    let mut decoded_loss_rows = 0usize;
    let mut decoded_reinforcement_rows = 0usize;
    let mut decoded_deleted_rows = 0usize;
    let mut mission_region_rows = 0usize;
    let mut baseline_rows = 0usize;
    let mut wing_scalar_rows = 0usize;
    let mut scalar_change_rows = 0usize;
    let max_equipment_amount = (COMPACT_AIR_WING_MAX_PLANES as i64)
        .saturating_mul(COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE);
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_STRENGTH_DELTA_ROW_LEN,
            "air combat losses live row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let owner_tag_db_id = read_u32_le(payload, cursor);
        let wing_id = read_u32_le(payload, cursor + 4);
        let generation = read_u64_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 16);
        let mission_region = read_u32_le(payload, cursor + 24);
        let old_count = read_u32_le(payload, cursor + 28);
        let new_count = read_u32_le(payload, cursor + 32);
        let count_delta = read_i32_le(payload, cursor + 36);
        let reserved1 = read_u32_le(payload, cursor + 40);
        let old_equipment_amount_sum = read_i64_le(payload, cursor + 44);
        let new_equipment_amount_sum = read_i64_le(payload, cursor + 52);
        let equipment_amount_delta = read_i64_le(payload, cursor + 60);
        let air_accidents = read_u32_le(payload, cursor + 68);
        let experience_raw = read_i32_le(payload, cursor + 72);
        cursor += COMPACT_AIR_STRENGTH_DELTA_ROW_LEN;

        let deleted = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_DELETED) != 0;
        let loss = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_LOSS) != 0;
        let reinforcement = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT) != 0;
        let baseline = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE) != 0;
        let wing_scalars = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS) != 0;
        let scalar_change = (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALAR_CHANGE) != 0;
        if baseline {
            baseline_rows = baseline_rows.saturating_add(1);
            if loss || reinforcement || deleted || scalar_change {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} marks baseline with event flags"
                ));
            }
            if count_delta != 0
                || old_count != new_count
                || equipment_amount_delta != 0
                || old_equipment_amount_sum != new_equipment_amount_sum
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live baseline row {row_index} has nonzero deltas"
                ));
            }
        }
        if wing_scalars {
            wing_scalar_rows = wing_scalar_rows.saturating_add(1);
            if air_accidents > COMPACT_AIR_WING_MAX_PLANES {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid air_accidents {air_accidents}"
                ));
            }
            if !(0..=COMPACT_AIR_WING_MAX_EXPERIENCE_RAW).contains(&experience_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid experience_raw {experience_raw}"
                ));
            }
        } else if air_accidents != 0 || experience_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has wing scalar values without measured flag"
            ));
        }
        if scalar_change {
            scalar_change_rows = scalar_change_rows.saturating_add(1);
            if !wing_scalars {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} marks scalar change without measured scalars"
                ));
            }
        }
        if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if wing_id == 0 || wing_id > COMPACT_AIR_WING_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid wing_id {wing_id}"
            ));
        } else if !seen_rows.insert((
            owner_tag_db_id,
            wing_id,
            generation,
            deleted,
            old_count,
            new_count,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} duplicates owner/wing/generation/counts"
            ));
        }
        if flags
            & !(COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_LOSS
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_REINFORCEMENT
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_DELETED
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALARS
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_STATE_BASELINE
                | COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_WING_SCALAR_CHANGE)
            != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has unknown flags {flags}"
            ));
        }
        if !loss && !reinforcement && !deleted && !baseline && !scalar_change {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has no event flag"
            ));
        }
        if (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_MISSION_REGION) != 0 {
            mission_region_rows = mission_region_rows.saturating_add(1);
            if mission_region == 0 || mission_region > COMPACT_AIR_WING_MAX_MISSION_REGION_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} has invalid mission_region {mission_region}"
                ));
            }
        } else if mission_region != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has mission_region without flag"
            ));
        }
        if old_count > COMPACT_AIR_WING_MAX_PLANES || new_count > COMPACT_AIR_WING_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid counts {old_count}->{new_count}"
            ));
        }
        if count_delta as i64 != (new_count as i64) - (old_count as i64) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} count_delta {count_delta} does not match counts {old_count}->{new_count}"
            ));
        }
        if !(0..=max_equipment_amount).contains(&old_equipment_amount_sum)
            || !(0..=max_equipment_amount).contains(&new_equipment_amount_sum)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid equipment amount sums"
            ));
        }
        if equipment_amount_delta
            != new_equipment_amount_sum.saturating_sub(old_equipment_amount_sum)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} equipment delta does not match equipment sums"
            ));
        }
        let expected_equipment_delta =
            (count_delta as i64).saturating_mul(COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE);
        if equipment_amount_delta == expected_equipment_delta {
            if (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK) == 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: air combat losses live row {row_index} missing equipment scale flag"
                ));
            }
        } else if (flags & COMPACT_AIR_STRENGTH_DELTA_ROW_FLAG_EQUIPMENT_SCALE_OK) != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} has invalid equipment scale flag"
            ));
        }
        if loss != (count_delta < 0 || equipment_amount_delta < 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} loss flag does not match deltas"
            ));
        }
        if reinforcement != (count_delta > 0 || equipment_amount_delta > 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} reinforcement flag does not match deltas"
            ));
        }
        if deleted && (new_count != 0 || new_equipment_amount_sum != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live deleted row {row_index} has live new fields"
            ));
        }
        if reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat losses live row {row_index} reserved field is nonzero"
            ));
        }
        if loss {
            decoded_loss_rows = decoded_loss_rows.saturating_add(1);
        }
        if reinforcement {
            decoded_reinforcement_rows = decoded_reinforcement_rows.saturating_add(1);
        }
        if deleted {
            decoded_deleted_rows = decoded_deleted_rows.saturating_add(1);
        }
    }
    if decoded_loss_rows != loss_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live loss_rows {loss_rows} != decoded {decoded_loss_rows}"
        ));
    }
    if decoded_reinforcement_rows != reinforcement_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live reinforcement_rows {reinforcement_rows} != decoded {decoded_reinforcement_rows}"
        ));
    }
    if decoded_deleted_rows != deleted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat losses live deleted_rows {deleted_rows} != decoded {decoded_deleted_rows}"
        ));
    }
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_mission_region_rows",
        mission_region_rows,
    );
    add_compact_domain_data_unit(summary, 42, "air_strength_baseline_rows", baseline_rows);
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_wing_scalar_rows",
        wing_scalar_rows,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_scalar_change_rows",
        scalar_change_rows,
    );

    // Historical payloads ended at the legacy wing rows. Current-build
    // payloads append the exact A4R1 regional combat-history state.
    if payload.len() == legacy_len {
        return;
    }
    let trailer = legacy_len;
    if let Err(err) = require_compact_remaining(
        payload,
        trailer,
        COMPACT_AIR_REGION_STATS_TRAILER_HEADER_LEN,
        "air combat regional stats trailer",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return;
    }
    let magic = read_u32_le(payload, trailer);
    let version = read_u16_le(payload, trailer + 4);
    let header_len = read_u16_le(payload, trailer + 6) as usize;
    let region_row_count = read_u32_le(payload, trailer + 8) as usize;
    let damage_row_count = read_u32_le(payload, trailer + 12) as usize;
    let observed_region_objects = read_u32_le(payload, trailer + 16) as usize;
    let diagnostic_invalid_wing_rows = read_u32_le(payload, trailer + 20);
    let diagnostic_scale_mismatch_rows = read_u32_le(payload, trailer + 24);
    let diagnostic_first_invalid = read_u32_le(payload, trailer + 28);
    if magic != COMPACT_AIR_REGION_STATS_TRAILER_MAGIC
        || (version != COMPACT_AIR_REGION_STATS_TRAILER_VERSION
            && version != COMPACT_AIR_REGION_STATS_TRAILER_LEGACY_VERSION)
        || header_len != COMPACT_AIR_REGION_STATS_TRAILER_HEADER_LEN
    {
        summary.parse_errors.push(format!(
            "offset {offset}: invalid air combat regional stats trailer identity"
        ));
        return;
    }
    if version == COMPACT_AIR_REGION_STATS_TRAILER_LEGACY_VERSION
        && (diagnostic_invalid_wing_rows != 0
            || diagnostic_scale_mismatch_rows != 0
            || diagnostic_first_invalid != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat regional stats trailer reserved field is nonzero"
        ));
    }
    let first_invalid_reason = diagnostic_first_invalid & 0xFFFF;
    let first_invalid_ordinal = diagnostic_first_invalid >> 16;
    if version == COMPACT_AIR_REGION_STATS_TRAILER_VERSION {
        if (diagnostic_invalid_wing_rows == 0) != (first_invalid_reason == 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat invalid-wing diagnostic count/reason mismatch"
            ));
        }
        if first_invalid_reason != 0
            && compact_air_strength_invalid_reason_unit(first_invalid_reason).is_none()
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat invalid-wing diagnostic reason {first_invalid_reason} is unknown"
            ));
        }
        if diagnostic_invalid_wing_rows > 0
            && first_invalid_ordinal > observed_air_wing_rows.saturating_add(
                diagnostic_invalid_wing_rows as usize,
            ) as u32
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air combat invalid-wing diagnostic ordinal {first_invalid_ordinal} exceeds observed source rows"
            ));
        }
    }
    let region_bytes = region_row_count.saturating_mul(COMPACT_AIR_REGION_STATS_ROW_LEN);
    let damage_bytes = damage_row_count.saturating_mul(COMPACT_AIR_REGION_DAMAGE_ROW_LEN);
    let expected_len = trailer
        .saturating_add(header_len)
        .saturating_add(region_bytes)
        .saturating_add(damage_bytes);
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat regional stats payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    let region_base = trailer + header_len;
    let damage_base = region_base + region_bytes;
    let mut expected_damage_first = 0usize;
    for index in 0..region_row_count {
        let base = region_base + index * COMPACT_AIR_REGION_STATS_ROW_LEN;
        let owner = read_u32_le(payload, base);
        let region_id = read_u32_le(payload, base + 4);
        let side_vector = payload[base + 8];
        let selection_flag = payload[base + 9];
        let classification_flag = payload[base + 10];
        let copied_flag = payload[base + 11];
        let wing_id = read_u32_le(payload, base + 12);
        let plane_count = read_u32_le(payload, base + 20);
        let attributed_country = read_u32_le(payload, base + 28);
        let damage_first = read_u32_le(payload, base + 32) as usize;
        let damage_count = read_u32_le(payload, base + 36) as usize;
        if owner == 0
            || owner > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID
            || attributed_country == 0
            || attributed_country > COMPACT_AIR_WING_MAX_OWNER_TAG_DB_ID
            || region_id > COMPACT_AIR_WING_MAX_MISSION_REGION_ID
            || wing_id == 0
            || wing_id > COMPACT_AIR_WING_MAX_ID
            || plane_count > COMPACT_AIR_WING_MAX_PLANES
            || side_vector > 1
            || selection_flag > 1
            || classification_flag > 1
            || copied_flag > 1
            || damage_first != expected_damage_first
            || damage_first.saturating_add(damage_count) > damage_row_count
        {
            summary.parse_errors.push(format!(
                "offset {offset}: invalid air combat regional stats row {index}"
            ));
        }
        expected_damage_first = damage_first.saturating_add(damage_count);
    }
    if expected_damage_first != damage_row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air combat regional stats damage coverage mismatch"
        ));
    }
    for index in 0..damage_row_count {
        let base = damage_base + index * COMPACT_AIR_REGION_DAMAGE_ROW_LEN;
        let channel = payload[base];
        let cause_type = payload[base + 1];
        let reserved16 = read_u16_le(payload, base + 2);
        let reserved32 = read_u32_le(payload, base + 4);
        let amount_raw = read_i64_le(payload, base + 8);
        if channel > 1
            || cause_type > 10
            || reserved16 != 0
            || reserved32 != 0
            || amount_raw < 0
            || amount_raw > 1_000_000_000_000_000
        {
            summary.parse_errors.push(format!(
                "offset {offset}: invalid air combat regional damage row {index}"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 42, "air_region_combat_stat_rows", region_row_count);
    add_compact_domain_data_unit(summary, 42, "air_region_damage_rows", damage_row_count);
    add_compact_domain_data_unit(
        summary,
        42,
        "air_region_combat_objects",
        observed_region_objects,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_invalid_wing_rows",
        diagnostic_invalid_wing_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_scale_mismatch_rows",
        diagnostic_scale_mismatch_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_first_invalid_ordinal",
        first_invalid_ordinal as usize,
    );
    add_compact_domain_data_unit(
        summary,
        42,
        "air_strength_first_invalid_reason",
        first_invalid_reason as usize,
    );
    if let Some(reason_unit) = compact_air_strength_invalid_reason_unit(first_invalid_reason) {
        add_compact_domain_data_unit(summary, 42, reason_unit, 1);
    }
}

fn compact_air_strength_invalid_reason_unit(reason: u32) -> Option<&'static str> {
    Some(match reason {
        1 => "air_strength_invalid_container_address_or_vtable",
        2 => "air_strength_invalid_container_owner",
        3 => "air_strength_invalid_pool_vector",
        4 => "air_strength_invalid_pool_address_or_vtable",
        5 => "air_strength_invalid_pool_owner",
        6 => "air_strength_invalid_object_vector",
        10 => "air_strength_invalid_object_address",
        11 => "air_strength_invalid_object_vtable_read",
        12 => "air_strength_invalid_object_vtable_mismatch",
        13 => "air_strength_invalid_body_address",
        14 => "air_strength_invalid_body_vtable_read",
        15 => "air_strength_invalid_body_vtable_mismatch",
        16 => "air_strength_invalid_wing_type_read",
        17 => "air_strength_invalid_wing_type_mismatch",
        18 => "air_strength_invalid_pool_read",
        19 => "air_strength_invalid_pool_mismatch",
        20 => "air_strength_invalid_owner",
        21 => "air_strength_invalid_wing_id_read",
        22 => "air_strength_invalid_wing_id_range",
        23 => "air_strength_invalid_count_read",
        24 => "air_strength_invalid_count_range",
        25 => "air_strength_invalid_equipment_bank",
        26 => "air_strength_invalid_equipment_sum_overflow",
        27 => "air_strength_invalid_equipment_sum_range",
        28 => "air_strength_invalid_air_accidents_read",
        29 => "air_strength_invalid_air_accidents_range",
        30 => "air_strength_invalid_experience_read",
        31 => "air_strength_invalid_experience_range",
        32 => "air_strength_invalid_mission_mask_read",
        _ => return None,
    })
}

fn decode_compact_air_variant_defs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    decode_compact_variant_defs_data_payload(
        offset,
        payload,
        summary,
        33,
        "air variant defs",
        "air_variant_def",
    );
}

fn decode_compact_stockpile_variant_defs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    decode_compact_variant_defs_data_payload(
        offset,
        payload,
        summary,
        37,
        "stockpile variant defs",
        "stockpile_variant_def",
    );
}

fn decode_compact_variant_defs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    label: &str,
    metric_prefix: &str,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} data payload is {} byte(s), expected at least 44",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let observed_module_rows = read_u32_le(payload, 24) as usize;
    let emitted_module_rows = read_u32_le(payload, 28) as usize;
    let invalid_variant_rows = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if emitted_module_rows > observed_module_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} emitted_module_rows {emitted_module_rows} > observed_module_rows {observed_module_rows}"
        ));
    }
    if invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} invalid_variant_rows {invalid_variant_rows} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} header reserved field is nonzero"
        ));
    }
    let expected_rows_end =
        44usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_DEF_ROW_LEN));
    if payload.len() < expected_rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} data payload is {} byte(s), expected at least {expected_rows_end}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_rows"),
        row_count,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_observed_variant_rows"),
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_observed_module_rows"),
        observed_module_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_emitted_module_rows"),
        emitted_module_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_invalid_variant_rows"),
        invalid_variant_rows,
    );

    let mut cursor = 44usize;
    let mut rows = Vec::with_capacity(row_count);
    let mut seen_variants = BTreeSet::new();
    let mut declared_modules = 0usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_VARIANT_DEF_ROW_LEN,
            &format!("{label} row"),
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let module_first = read_u32_le(payload, cursor + 8) as usize;
        let module_count = read_u32_le(payload, cursor + 12) as usize;
        let row_reserved0 = read_u32_le(payload, cursor + 16);
        let row_reserved1 = read_u32_le(payload, cursor + 20);
        cursor += COMPACT_AIR_VARIANT_DEF_ROW_LEN;
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_DEF_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        let known_flags = COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME
            | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME
            | COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved0 != 0 || row_reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} reserved fields are nonzero"
            ));
        }
        if module_count == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has modules flag without modules"
            ));
        }
        if module_count > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_MODULES == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has modules without modules flag"
            ));
        }
        let Some(module_end) = module_first.checked_add(module_count) else {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} module range overflows"
            ));
            continue;
        };
        if module_end > emitted_module_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} module range {module_first}+{module_count} exceeds {emitted_module_rows}"
            ));
        }
        declared_modules = declared_modules.saturating_add(module_count);
        rows.push((flags, module_first, module_count));
    }
    if declared_modules != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} emitted_module_rows {emitted_module_rows} != declared {declared_modules}"
        ));
    }

    for (row_index, (flags, _, _)) in rows.iter().enumerate() {
        let display_label = format!("{label} display name");
        let display_len = match skip_compact_string_len(payload, &mut cursor, &display_label) {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if display_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} display name is too long ({display_len})"
            ));
        }
        if display_len == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has display-name flag with empty string"
            ));
        }
        if display_len > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_DISPLAY_NAME == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has display name without flag"
            ));
        }
        let variant_label = format!("{label} variant name");
        let variant_len = match skip_compact_string_len(payload, &mut cursor, &variant_label) {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if variant_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} variant name is too long ({variant_len})"
            ));
        }
        if variant_len == 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has variant-name flag with empty string"
            ));
        }
        if variant_len > 0 && flags & COMPACT_AIR_VARIANT_DEF_ROW_FLAG_VARIANT_NAME == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has variant name without flag"
            ));
        }
    }

    let mut covered_modules = BTreeSet::new();
    for (row_index, (_, module_first, module_count)) in rows.iter().enumerate() {
        for module_index in *module_first..module_first.saturating_add(*module_count) {
            if !covered_modules.insert(module_index) {
                summary.parse_errors.push(format!(
                    "offset {offset}: {label} row {row_index} overlaps module row {module_index}"
                ));
            }
        }
    }

    let mut nonempty_module_tokens = 0usize;
    for module_index in 0..emitted_module_rows {
        if let Err(err) =
            require_compact_remaining(payload, cursor, 8, &format!("{label} module row"))
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let slot_handle = read_u32_le(payload, cursor);
        let module_reserved = read_u32_le(payload, cursor + 4);
        cursor += 8;
        if slot_handle == 0 || slot_handle > COMPACT_AIR_VARIANT_DEF_MAX_SLOT_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} module row {module_index} has invalid slot_handle {slot_handle}"
            ));
        }
        if module_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} module row {module_index} reserved field is nonzero"
            ));
        }
        let token_len =
            match skip_compact_string_len(payload, &mut cursor, &format!("{label} module token")) {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if token_len == 0 || token_len > COMPACT_AIR_VARIANT_DEF_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} module row {module_index} has invalid token length {token_len}"
            ));
        } else {
            nonempty_module_tokens = nonempty_module_tokens.saturating_add(1);
        }
    }
    if covered_modules.len() != emitted_module_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} covered module rows {} != emitted_module_rows {emitted_module_rows}",
            covered_modules.len()
        ));
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_module_token_rows"),
        nonempty_module_tokens,
    );
}

fn decode_compact_air_variant_stats_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let invalid_variant_rows = read_u32_le(payload, 24) as usize;
    let fixed_scale_raw = read_u32_le(payload, 28);
    let range_scale_raw = read_u32_le(payload, 32);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats invalid_variant_rows {invalid_variant_rows} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if fixed_scale_raw == 0 || range_scale_raw == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats scale fields must be nonzero"
        ));
    }
    let expected_len =
        36usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_STATS_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant stats data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&35) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 35, "air_variant_stats_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_observed_variant_rows",
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_invalid_variant_rows",
        invalid_variant_rows,
    );

    let mut seen_variants = BTreeSet::new();
    let mut speed_cache_raw_sum = 0u128;
    let mut range_raw_sum = 0u128;
    let mut air_attack_raw_sum = 0u128;
    let mut agility_raw_sum = 0u128;
    let mut max_speed_raw_sum = 0u128;
    let mut reliability_raw_sum = 0u128;
    let mut air_defence_raw_sum = 0u128;
    let mut thrust_raw_sum = 0u128;
    let mut fuel_consumption_raw_sum = 0u128;
    let mut build_cost_ic_raw_sum = 0u128;
    let mut cursor = 36usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_VARIANT_STATS_ROW_LEN,
            "air variant stats row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let equipment_token_raw = read_u32_le(payload, cursor + 8);
        let equipment_type_token_raw = read_u32_le(payload, cursor + 12);
        let _interface_type_raw = read_u32_le(payload, cursor + 16);
        let speed_cache_raw = read_i32_le(payload, cursor + 20);
        let range_raw = read_i32_le(payload, cursor + 24);
        let air_attack_raw = read_i32_le(payload, cursor + 28);
        let agility_raw = read_i32_le(payload, cursor + 32);
        let max_speed_raw = read_i32_le(payload, cursor + 36);
        let reliability_raw = read_i32_le(payload, cursor + 40);
        let air_defence_raw = read_i32_le(payload, cursor + 44);
        let thrust_raw = read_i32_le(payload, cursor + 48);
        let fuel_consumption_raw = read_i32_le(payload, cursor + 52);
        let build_cost_ic_raw = read_i32_le(payload, cursor + 56);
        let row_reserved = read_u32_le(payload, cursor + 60);
        cursor += COMPACT_AIR_VARIANT_STATS_ROW_LEN;
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_STATS_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} reserved field is nonzero"
            ));
        }
        if equipment_token_raw == 0 || equipment_type_token_raw == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant stats row {row_index} has zero equipment token/type fields"
            ));
        }
        for (label, value) in [
            ("speed_cache_raw", speed_cache_raw),
            ("range_raw", range_raw),
            ("air_attack_raw", air_attack_raw),
            ("agility_raw", agility_raw),
            ("max_speed_raw", max_speed_raw),
            ("reliability_raw", reliability_raw),
            ("air_defence_raw", air_defence_raw),
            ("thrust_raw", thrust_raw),
            ("fuel_consumption_raw", fuel_consumption_raw),
            ("build_cost_ic_raw", build_cost_ic_raw),
        ] {
            if !(0..=COMPACT_AIR_VARIANT_STATS_MAX_RAW).contains(&value) {
                summary.parse_errors.push(format!(
                    "offset {offset}: air variant stats row {row_index} has invalid {label} {value}"
                ));
            }
        }
        speed_cache_raw_sum = speed_cache_raw_sum.saturating_add(speed_cache_raw.max(0) as u128);
        range_raw_sum = range_raw_sum.saturating_add(range_raw.max(0) as u128);
        air_attack_raw_sum = air_attack_raw_sum.saturating_add(air_attack_raw.max(0) as u128);
        agility_raw_sum = agility_raw_sum.saturating_add(agility_raw.max(0) as u128);
        max_speed_raw_sum = max_speed_raw_sum.saturating_add(max_speed_raw.max(0) as u128);
        reliability_raw_sum = reliability_raw_sum.saturating_add(reliability_raw.max(0) as u128);
        air_defence_raw_sum = air_defence_raw_sum.saturating_add(air_defence_raw.max(0) as u128);
        thrust_raw_sum = thrust_raw_sum.saturating_add(thrust_raw.max(0) as u128);
        fuel_consumption_raw_sum =
            fuel_consumption_raw_sum.saturating_add(fuel_consumption_raw.max(0) as u128);
        build_cost_ic_raw_sum =
            build_cost_ic_raw_sum.saturating_add(build_cost_ic_raw.max(0) as u128);
    }
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_speed_cache_raw_sum",
        compact_sum_to_usize(speed_cache_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_range_raw_sum",
        compact_sum_to_usize(range_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_air_attack_raw_sum",
        compact_sum_to_usize(air_attack_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_agility_raw_sum",
        compact_sum_to_usize(agility_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_max_speed_raw_sum",
        compact_sum_to_usize(max_speed_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_reliability_raw_sum",
        compact_sum_to_usize(reliability_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_air_defence_raw_sum",
        compact_sum_to_usize(air_defence_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_thrust_raw_sum",
        compact_sum_to_usize(thrust_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_fuel_consumption_raw_sum",
        compact_sum_to_usize(fuel_consumption_raw_sum),
    );
    add_compact_domain_data_unit(
        summary,
        35,
        "air_variant_stats_build_cost_ic_raw_sum",
        compact_sum_to_usize(build_cost_ic_raw_sum),
    );
}

fn decode_compact_air_variant_archetypes_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = air_wings_variants_lifecycle_live_lane::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.data_changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_characters_leaders_cabinet_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = domain47::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.row_count_total);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_construction_buildings_state_damage_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match domain48::decoder_gate::decode_domain48_compact(payload) {
        Ok(decoded) => decoded,
        Err(err) => {
            summary
                .parse_errors
                .push(format!("offset {offset}: domain48 decode failed: {err}"));
            return;
        }
    };
    add_decoder_status(summary, decoded.frame.status_flags);
    for error in &decoded.gate.errors {
        add_compact_domain_problem(
            summary,
            COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
            domain48_gate_problem_name(error),
            1,
        );
    }
    if !compact_contract::compact_status_is_failure(decoded.frame.status_flags) && !decoded.gate.ok
    {
        summary.parse_errors.push(format!(
            "offset {offset}: domain48 gate failed: {}",
            decoded.gate.errors.join("; ")
        ));
    }
    let row_count = decoded
        .summary
        .construction_rows
        .saturating_add(decoded.summary.railway_construction_rows)
        .saturating_add(decoded.summary.building_rows)
        .saturating_add(decoded.summary.resource_rows);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE)
    {
        domain.status_flags_or |= decoded.frame.status_flags;
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "construction_queue_rows",
        decoded.summary.construction_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "railway_construction_rows",
        decoded.summary.railway_construction_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "construction_queue_target_state_rows",
        decoded.summary.construction_rows_with_target_state,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "building_rows",
        decoded.summary.building_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "damaged_building_rows",
        decoded.summary.damaged_building_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "resource_rows",
        decoded.summary.resource_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE,
        "target_state_unavailable_badged",
        usize::from(decoded.summary.target_state_unavailable_badged),
    );
}

fn domain48_gate_problem_name(error: &str) -> &'static str {
    if error.starts_with("blocking status_flags set") {
        "domain48_gate_blocking_status_flags_set"
    } else if error == "source_flags missing DLL-native bit" {
        "domain48_gate_missing_dll_native_source_flag"
    } else if error == "no building rows emitted" {
        "domain48_gate_no_building_rows_emitted"
    } else if error == "no state resource rows emitted" {
        "domain48_gate_no_state_resource_rows_emitted"
    } else if error == "construction target_state_id absent without target-state-unavailable source flag" {
        "domain48_gate_target_state_absent_without_unavailable_flag"
    } else if error
        == "construction target_state_id emitted while target-state-unavailable source flag is set"
    {
        "domain48_gate_target_state_emitted_while_unavailable"
    } else if error
        == "construction target_state_id partially emitted; fill every construction row or badge unavailable"
    {
        "domain48_gate_target_state_partially_emitted"
    } else if error.contains("duplicates tag/line") {
        "domain48_gate_duplicate_construction_row"
    } else if error.contains("duplicates scope/location/type") {
        "domain48_gate_duplicate_building_row"
    } else if error.contains("duplicates state/resource") {
        "domain48_gate_duplicate_resource_row"
    } else {
        "domain48_gate_unknown_error"
    }
}

fn decode_compact_scripted_state_flags_variables_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let verdict = scripted_state_flags_variables_live_lane::gate_domain51_payload(offset, payload);
    add_compact_global_status_flags(
        summary,
        COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE,
        verdict.summary.status_flags_or,
    );
    summary.parse_errors.extend(verdict.summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE)
    {
        domain.status_flags_or |= verdict.summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(verdict.summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(verdict.summary.data_changed_rows_total);
        for (name, count) in verdict.summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_occupation_resistance_compliance_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = occupation52::domain52_compact::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.data_changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_diplomacy_intel_operatives_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    let lane_summary = diplomacy53::compact::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.data_changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_capitulation_war_state_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = capwar60::compact::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.data_rows_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.data_changed_rows_total);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE,
        "capitulation_country_rows",
        lane_summary.country_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE,
        "war_rows",
        lane_summary.war_rows as usize,
    );
}

fn decode_compact_air_loss_stats_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = airloss61::decode_payload(payload);
    add_decoder_status(summary, lane_summary.status_flags);
    summary.parse_errors.extend(
        lane_summary
            .parse_errors
            .into_iter()
            .map(|error| format!("offset {offset}: air_loss_stats_live {error}")),
    );
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.row_count);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE,
        "air_loss_stat_rows",
        lane_summary.row_count as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE,
        "air_loss_binding_verified_rows",
        lane_summary.binding_verified_rows as usize,
    );
    // Each channel total is the within-frame sum over countries of that
    // country's ENGINE LIFETIME total for the channel (statistic id 23, the
    // 89-field aggregate air-loss HISTORY record). The world total is therefore
    // cumulative even though this frame built it by adding: summing it again
    // across frames multiplied the engine's lifetime figure by the frame count.
    for (label, total) in lane_summary.channel_totals {
        add_compact_domain_cumulative_unit(
            summary,
            COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE,
            &format!("air_loss_{label}_raw_sum"),
            total.max(0) as u64,
        );
    }
}

fn decode_compact_hub_motorization_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = hub62::decode_payload(payload);
    add_decoder_status(summary, lane_summary.status_flags);
    summary.parse_errors.extend(
        lane_summary
            .parse_errors
            .into_iter()
            .map(|error| format!("offset {offset}: hub_motorization_live {error}")),
    );
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.row_count);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_rows",
        lane_summary.row_count as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_verified_rows",
        lane_summary.verified_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_nodes",
        lane_summary.motorized_nodes as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_settings_nodes",
        lane_summary.total_settings_nodes as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_map_entries",
        lane_summary.total_map_entries as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_level_1_entries",
        lane_summary.level_one_entries as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE,
        "hub_motorization_level_2_entries",
        lane_summary.level_two_entries as usize,
    );
}

fn decode_compact_army_battleplans_depth_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    let lane_summary = battleplans54::decode_depth_payload(offset, payload);
    add_decoder_status(summary, lane_summary.totals.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE)
    {
        domain.status_flags_or |= lane_summary.totals.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.totals.rows);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.totals.changed_rows);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_depth_rows",
        lane_summary.totals.rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_target_state_rows",
        lane_summary.totals.target_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_path_geometry_rows",
        lane_summary.totals.path_geometry_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_non_type5_exact_path_rows",
        lane_summary.totals.non_type5_exact_path_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_fixture_blocked_rows",
        lane_summary
            .totals
            .unsupported_geometry_rows
            .saturating_add(lane_summary.totals.execution_fixture_required_rows) as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_assigned_units_exact_rows",
        lane_summary.totals.assigned_units_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_assigned_division_rows",
        lane_summary.totals.assigned_division_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_identity_exact_rows",
        lane_summary.totals.identity_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_active_exact_rows",
        lane_summary.totals.active_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_order_linkage_exact_rows",
        lane_summary.totals.order_linkage_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_omitted_non_exact_rows",
        lane_summary.totals.omitted_non_exact_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_path_geometry_omitted_rows",
        lane_summary.totals.path_geometry_omitted_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_identity_or_active_omitted_rows",
        lane_summary.totals.identity_or_active_omitted_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_complete_manager_records",
        lane_summary.totals.orders_group_extension_records as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_complete_army_rows",
        lane_summary.totals.complete_army_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_registered_orders_group_rows",
        lane_summary.totals.registered_orders_group_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE,
        "army_battleplans_registered_army_group_rows",
        lane_summary.totals.registered_army_group_rows as usize,
    );
}

fn decode_compact_resolved_tooltip_instances_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let frame = match tooltip72::decode_payload(payload) {
        Ok(frame) => frame,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: domain72 decode failed: {error:?}"
            ));
            return;
        }
    };
    add_decoder_status(summary, frame.status_flags);
    let instance_count = frame.instances.len();
    let final_blob_count = frame
        .instances
        .iter()
        .filter(|instance| !instance.final_resolved_text.is_empty())
        .count();
    if let Some(domain) = summary.per_domain.get_mut(&tooltip72::DOMAIN_ID) {
        domain.status_flags_or |= frame.status_flags;
        domain.data_rows_total = domain.data_rows_total.saturating_add(instance_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(instance_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        tooltip72::DOMAIN_ID,
        "resolved_tooltip_instances",
        instance_count,
    );
    add_compact_domain_data_unit(
        summary,
        tooltip72::DOMAIN_ID,
        "resolved_tooltip_final_blobs",
        final_blob_count,
    );
}

fn decode_compact_dynamic_collection_trace_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let Some(magic) = payload
        .get(..4)
        .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
        .map(u32::from_le_bytes)
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: domain71 payload lacks a complete DCT1/TCV1 carrier discriminator"
        ));
        return;
    };

    if magic == dynamic71::MAGIC {
        let frame = match dynamic71::decode_payload(payload) {
            Ok(frame) => frame,
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: dynamic_collection_trace DCT1 payload invalid: {error:?}"
                ));
                return;
            }
        };
        add_decoder_status(summary, frame.status_flags);
        let collection_count = frame.collections.len();
        let source_item_count = frame
            .collections
            .iter()
            .map(|collection| collection.source_items.len())
            .sum();
        let ordered_item_count = frame
            .collections
            .iter()
            .map(|collection| collection.ordered_item_ids.len())
            .sum();
        let group_count = frame
            .collections
            .iter()
            .map(|collection| collection.group_boundaries.len())
            .sum();
        let materialized_slot_count = frame
            .collections
            .iter()
            .map(|collection| collection.materialized_slots.len())
            .sum();
        if let Some(domain) = summary.per_domain.get_mut(&dynamic71::DOMAIN_ID) {
            domain.status_flags_or |= frame.status_flags;
            domain.data_rows_total = domain
                .data_rows_total
                .saturating_add(collection_count as u64);
            domain.data_changed_rows_total = domain
                .data_changed_rows_total
                .saturating_add(collection_count as u64);
        }
        for (name, count) in [
            ("dynamic_collection_trace_collections", collection_count),
            ("dynamic_collection_trace_source_items", source_item_count),
            ("dynamic_collection_trace_ordered_items", ordered_item_count),
            ("dynamic_collection_trace_groups", group_count),
            (
                "dynamic_collection_trace_materialized_slots",
                materialized_slot_count,
            ),
        ] {
            add_compact_domain_data_unit(summary, dynamic71::DOMAIN_ID, name, count);
        }
        return;
    }

    if magic == dynamic71::a729_technology_canvas::MAGIC {
        let frame = match dynamic71::a729_technology_canvas::decode_authenticated_payload(payload) {
            Ok(frame) => frame,
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: technology_canvas TCV1 payload not admitted: {error:?}"
                ));
                return;
            }
        };
        let source_item_count = frame.source_items.len();
        let ordered_item_count = frame.final_draw_order.len();
        let materialized_drawable_count = frame.materialized_drawables.len();
        if let Some(domain) = summary.per_domain.get_mut(&dynamic71::DOMAIN_ID) {
            domain.data_rows_total = domain.data_rows_total.saturating_add(1);
            domain.data_changed_rows_total = domain.data_changed_rows_total.saturating_add(1);
        }
        for (name, count) in [
            ("technology_canvas_frames", 1),
            ("technology_canvas_source_items", source_item_count),
            ("technology_canvas_ordered_items", ordered_item_count),
            (
                "technology_canvas_materialized_drawables",
                materialized_drawable_count,
            ),
        ] {
            add_compact_domain_data_unit(summary, dynamic71::DOMAIN_ID, name, count);
        }
        return;
    }

    summary.parse_errors.push(format!(
        "offset {offset}: domain71 payload has an unknown DCT1/TCV1 carrier discriminator"
    ));
}

fn decode_compact_hud_surface_activation_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let frame = match hud79::decode_activation_payload(payload) {
        Ok(frame) => frame,
        Err(error) => {
            summary
                .parse_errors
                .push(format!("offset {offset}: domain79 decode failed: {error}"));
            return;
        }
    };
    add_decoder_status(summary, frame.status_flags);
    let row_count = frame.active_surfaces.len();
    if let Some(domain) = summary.per_domain.get_mut(&hud79::DOMAIN_ID) {
        domain.status_flags_or |= frame.status_flags;
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        hud79::DOMAIN_ID,
        "hud_surface_activation_rows",
        row_count,
    );
}

fn decode_compact_naval_missions_status_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let lane_summary = naval56::decode_payload(offset, payload);
    add_decoder_status(summary, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE)
    {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count_total);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.changed_rows_total);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_naval_combat_lifecycle_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let mut lane_summary = naval4950::decode_lifecycle_data_payload(offset, payload);
    if lane_summary.parse_errors.is_empty() {
        match naval4950::decode_lifecycle_rows(payload) {
            Ok((status_flags, rows)) => {
                if let Err(reason) = validate_d49_promoted_location_region(status_flags, &rows) {
                    // Bit 11 is deliberately lane-local for d49 and therefore
                    // masked from global status interpretation. It is still
                    // fatal to d49 DATA authority. Re-promote it when the row
                    // evidence proves that the collector omitted the badge.
                    lane_summary.status_flags_or |=
                        naval4950::STATUS_LOCATION_CANDIDATE_UNVALIDATED;
                    lane_summary
                        .parse_errors
                        .push(format!("offset {offset}: {reason}"));
                }
            }
            Err(errors) => lane_summary.parse_errors.extend(errors),
        }
    }
    merge_naval_combat_lane_summary(
        summary,
        COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE,
        lane_summary,
    );
}

/// D49's promoted province/strategic-region pair is mandatory on every active
/// lifecycle row. Bit 11 is a lane-local fail-closed badge (despite overlapping
/// a global informational bit) and is therefore rejected here before global
/// status normalization. Deleted tombstones may omit both promoted fields.
pub(crate) fn validate_d49_promoted_location_region(
    status_flags: u32,
    rows: &[naval4950::LifecycleRow],
) -> Result<(), &'static str> {
    if status_flags & naval4950::STATUS_LOCATION_CANDIDATE_UNVALIDATED != 0 {
        return Err("domain 49 location/region promotion is incomplete (status bit 11 set)");
    }
    if rows
        .iter()
        .any(|row| !row.deleted && (row.location_id.is_none() || row.region_id.is_none()))
    {
        return Err(
            "domain 49 active lifecycle row lacks promoted location/region; status bit 11 re-promoted",
        );
    }
    Ok(())
}

fn decode_compact_naval_combat_members_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let lane_summary = naval4950::decode_members_data_payload(offset, payload);
    merge_naval_combat_lane_summary(
        summary,
        COMPACT_DOMAIN_NAVAL_COMBAT_MEMBERS_LIVE,
        lane_summary,
    );
}

fn merge_naval_combat_lane_summary(
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    lane_summary: naval4950::PayloadDecodeSummary,
) {
    // Domain 49 carries a lane-local payload status bit (location-candidate
    // badge); mask it from the GLOBAL status interpretation while keeping the
    // raw flags on the per-domain summary (status_flags_scope reports this).
    add_compact_global_status_flags(summary, domain_id, lane_summary.status_flags_or);
    summary.parse_errors.extend(lane_summary.parse_errors);
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        domain.status_flags_or |= lane_summary.status_flags_or;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(lane_summary.row_count);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(lane_summary.row_count);
        for (name, count) in lane_summary.data_units_total {
            fold_named_unit(&mut domain.data_units_total, &name, count);
        }
    }
}

fn decode_compact_convoy_transport_attack_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match naval4950::decode_convoy_transport_attack_batch(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: convoy_transport_attack CTA2 payload invalid: {error:?}"
            ));
            return;
        }
    };

    let witness = decoded.witness;
    let drop_counts = [
        ("convoy_transport_attack_dropped_no_original", witness.telemetry.dropped_no_original),
        ("convoy_transport_attack_dropped_teardown", witness.telemetry.dropped_teardown),
        ("convoy_transport_attack_dropped_nested", witness.telemetry.dropped_nested),
        ("convoy_transport_attack_dropped_capture_busy", witness.telemetry.dropped_capture_busy),
        ("convoy_transport_attack_dropped_hour_or_epoch", witness.telemetry.dropped_hour_or_epoch),
        ("convoy_transport_attack_dropped_resolver", witness.telemetry.dropped_resolver),
        ("convoy_transport_attack_dropped_candidate", witness.telemetry.dropped_candidate),
        ("convoy_transport_attack_dropped_d70_pre", witness.telemetry.dropped_d70_pre),
        ("convoy_transport_attack_dropped_d70_post", witness.telemetry.dropped_d70_post),
        ("convoy_transport_attack_dropped_d70_mutation", witness.telemetry.dropped_d70_mutation),
        ("convoy_transport_attack_dropped_queue_busy", witness.telemetry.dropped_queue_busy),
        ("convoy_transport_attack_dropped_queue_full", witness.telemetry.dropped_queue_full),
    ];
    let dropped_events = drop_counts
        .iter()
        .fold(0u64, |total, (_, count)| total.saturating_add(*count));

    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.rows.len() as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(decoded.rows.len() as u64);

        // PER-FRAME: computed from THIS batch.
        let mut add_batch_unit = |name: &str, count: u64| {
            if count != 0 {
                fold_named_unit(&mut domain.data_units_total, name, count);
            }
        };
        add_batch_unit("convoy_transport_attack_batches", 1);
        add_batch_unit("convoy_transport_attack_rows", decoded.rows.len() as u64);
        add_batch_unit(
            "convoy_transport_attack_exact_complete_batches",
            u64::from(decoded.exact_events_complete),
        );
        add_batch_unit(
            "convoy_transport_attack_incomplete_batches",
            u64::from(!decoded.exact_events_complete),
        );
        add_batch_unit(
            "convoy_transport_attack_hook_installed_batches",
            u64::from(witness.hook_installed),
        );
        add_batch_unit(
            "convoy_transport_attack_queue_drain_complete_batches",
            u64::from(witness.queue_drain_complete),
        );
        add_batch_unit(
            "convoy_transport_attack_drained_rows",
            witness.drained_rows as u64,
        );
    }

    // CUMULATIVE and GAUGE: `witness.telemetry` is the hook's
    // `cumulative_telemetry` -- process-global `AtomicU64` counters reset only
    // at hook install, so every batch reports the same running totals again.
    // Summing them is what published 19,470 drops for 107 events in
    // runs/live_unlock/t15. The binding/queue counts are the state of the
    // publication and queue at drain time, so they are peaks, not totals.
    //
    // These deliberately do NOT skip a zero: for a running total, zero is the
    // measurement "nothing has happened yet".
    let telemetry = witness.telemetry;
    for (name, value) in [
        (
            "convoy_transport_attack_callbacks_seen",
            telemetry.callbacks_seen,
        ),
        (
            "convoy_transport_attack_original_calls",
            telemetry.original_calls,
        ),
        (
            "convoy_transport_attack_events_emitted",
            telemetry.events_emitted,
        ),
        ("convoy_transport_attack_dropped_events", dropped_events),
        (
            "convoy_transport_attack_remaining_events",
            witness.remaining_events as u64,
        ),
        (
            "convoy_transport_attack_active_callbacks",
            witness.active_callbacks as u64,
        ),
        (
            "convoy_transport_attack_resolver_member_bindings",
            witness.resolver_member_bindings as u64,
        ),
        (
            "convoy_transport_attack_resolver_route_bindings",
            witness.resolver_route_bindings as u64,
        ),
        (
            "convoy_transport_attack_resolver_unbound_convoy_clients",
            witness.resolver_unbound_convoy_clients as u64,
        ),
        // The sites `..._dropped_hour_or_epoch` and `..._dropped_resolver` were
        // hiding. They are published HERE rather than in `drop_counts` on
        // purpose: `drop_counts` is summed into
        // `convoy_transport_attack_dropped_events`, and these are subsets of
        // two entries already in that sum, so folding them in would count the
        // same dropped callback two or three times.
        (
            "convoy_transport_attack_dropped_pre_hour_unreadable",
            telemetry.dropped_pre_hour_unreadable,
        ),
        (
            "convoy_transport_attack_dropped_receipt_hour_mismatch",
            telemetry.dropped_receipt_hour_mismatch,
        ),
        (
            "convoy_transport_attack_dropped_post_hour_unreadable",
            telemetry.dropped_post_hour_unreadable,
        ),
        (
            "convoy_transport_attack_dropped_post_hour_changed",
            telemetry.dropped_post_hour_changed,
        ),
        (
            "convoy_transport_attack_dropped_resolver_snapshot_absent",
            telemetry.dropped_resolver_snapshot_absent,
        ),
        (
            "convoy_transport_attack_dropped_resolver_unpublished",
            telemetry.dropped_resolver_unpublished,
        ),
        // Distances, not counts. Running maxima, so they fold with MAX like the
        // counters around them rather than being summed.
        (
            "convoy_transport_attack_receipt_hour_lag_max",
            telemetry.receipt_hour_lag_max,
        ),
        (
            "convoy_transport_attack_receipt_hour_lead_max",
            telemetry.receipt_hour_lead_max,
        ),
    ] {
        add_compact_domain_cumulative_unit(
            summary,
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            name,
            value,
        );
    }
    for (name, count) in drop_counts {
        add_compact_domain_cumulative_unit(
            summary,
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            name,
            count,
        );
    }

    if !decoded.exact_events_complete {
        add_compact_domain_problem(
            summary,
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            "convoy_transport_attack_incomplete_witness",
            1,
        );
    }
    // Reported as its own problem, not folded into the incomplete witness: a
    // batch can be incomplete for reasons that have nothing to do with the
    // resolver, and a consumer deciding whether these rows cover every convoy
    // member needs the resolver's own partiality named. Zero here with an
    // incomplete witness means the loss was elsewhere.
    add_compact_domain_problem(
        summary,
        COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
        "convoy_transport_attack_resolver_unbound_convoy_clients",
        witness.resolver_unbound_convoy_clients as u64,
    );
    add_compact_domain_problem(
        summary,
        COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
        "convoy_transport_attack_dropped_events",
        dropped_events,
    );
}

fn decode_compact_session_multiplayer_metadata_live_data_payload(
    offset: usize,
    sequence: u64,
    header_hour_raw: i32,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < session57::PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 payload shorter than {}-byte header",
            session57::PAYLOAD_HEADER_LEN
        ));
        return;
    }
    let header_len = read_u16_le(payload, 6) as usize;
    let player_row_len = read_u16_le(payload, 10) as usize;
    let event_row_len = read_u16_le(payload, 12) as usize;
    if header_len != session57::PAYLOAD_HEADER_LEN
        || player_row_len != session57::PLAYER_COUNTRY_ROW_LEN
        || event_row_len != session57::SESSION_EVENT_ROW_LEN
    {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 invalid row/header lengths header={header_len} player={player_row_len} event={event_row_len}"
        ));
        return;
    }
    let player_count = read_u32_le(payload, 28) as usize;
    let event_count = read_u32_le(payload, 32) as usize;
    let string_bytes = read_u32_le(payload, 36) as usize;
    let Some(event_start) = player_count
        .checked_mul(player_row_len)
        .and_then(|bytes| header_len.checked_add(bytes))
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 player rows overflow payload size"
        ));
        return;
    };
    let Some(expected_len) = event_count
        .checked_mul(event_row_len)
        .and_then(|bytes| event_start.checked_add(bytes))
        .and_then(|bytes| bytes.checked_add(string_bytes))
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 event rows overflow payload size"
        ));
        return;
    };
    if expected_len != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 payload length {} != expected {expected_len}",
            payload.len()
        ));
        return;
    }
    let gate = session57::gate_payload(payload);
    add_decoder_status(summary, gate.summary.status_flags);
    summary.parse_errors.extend(
        gate.errors
            .into_iter()
            .map(|err| format!("offset {offset}: {err}")),
    );
    let payload_hour_raw = read_i32_le(payload, 16);
    if payload_hour_raw != header_hour_raw {
        summary.parse_errors.push(format!(
            "offset {offset}: d57 payload hour {payload_hour_raw} != H4CR hour {header_hour_raw}"
        ));
    }
    let mut speed_rows = Vec::new();
    let mut pause_rows = Vec::new();
    for row_index in 0..event_count {
        let row_start = event_start + row_index * event_row_len;
        let row = &payload[row_start..row_start + event_row_len];
        let event_hour_raw = read_i32_le(row, 48);
        let value_i32 = read_i32_le(row, 32);
        match session57::SessionEventKind::from_raw(read_u16_le(row, 0)) {
            Some(session57::SessionEventKind::GameSpeed) => {
                speed_rows.push(Domain57ScalarEvidence {
                    event_hour_raw,
                    value_i32,
                })
            }
            Some(session57::SessionEventKind::Pause) => pause_rows.push(Domain57ScalarEvidence {
                event_hour_raw,
                value_i32,
            }),
            _ => {}
        }
        if event_hour_raw != header_hour_raw {
            summary.parse_errors.push(format!(
                "offset {offset}: d57 event row {row_index} hour {event_hour_raw} != H4CR hour {header_hour_raw}"
            ));
        }
    }
    summary.domain57_frames.push(Domain57FrameEvidence {
        sequence,
        header_hour_raw,
        payload_hour_raw,
        root_evidence_mask: read_u32_le(payload, 44),
        speed_rows,
        pause_rows,
    });
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE)
    {
        domain.status_flags_or |= gate.summary.status_flags;
        let player_rows = gate.summary.player_country_rows as u64;
        let event_rows = gate.summary.event_rows as u64;
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(player_rows.saturating_add(event_rows));
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(player_rows.saturating_add(event_rows));
        *domain
            .data_units_total
            .entry("session_player_country_rows".into())
            .or_default() += player_rows;
        *domain
            .data_units_total
            .entry("session_event_rows".into())
            .or_default() += event_rows;
        let pause_rows = gate
            .summary
            .event_kind_counts
            .get(&(session57::SessionEventKind::Pause as u16))
            .copied()
            .unwrap_or(0);
        *domain
            .data_units_total
            .entry("session_pause_rows".into())
            .or_default() += pause_rows;
    }
}

fn decode_compact_land_combat_detail_events_live_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match land_detail55::decode_detail_events_payload(offset, payload) {
        Ok(decoded) => decoded,
        Err(errors) => {
            summary.parse_errors.extend(errors);
            return;
        }
    };
    add_decoder_status(summary, decoded.status_flags);
    let mut phase_started_rows = 0usize;
    let mut phase_updated_rows = 0usize;
    let mut phase_ended_rows = 0usize;
    let mut participant_joined_rows = 0usize;
    let mut participant_left_unclassified_rows = 0usize;
    for row in &decoded.rows {
        match row.event_kind {
            land_detail55::EVENT_PHASE_STARTED => phase_started_rows += 1,
            land_detail55::EVENT_PHASE_UPDATED => phase_updated_rows += 1,
            land_detail55::EVENT_PHASE_ENDED => phase_ended_rows += 1,
            land_detail55::EVENT_PARTICIPANT_JOINED => participant_joined_rows += 1,
            land_detail55::EVENT_PARTICIPANT_LEFT_UNCLASSIFIED => {
                participant_left_unclassified_rows += 1
            }
            _ => {}
        }
    }
    let row_count = decoded
        .rows
        .len()
        .saturating_add(decoded.side_detail_rows.len())
        .saturating_add(decoded.attribution_rows.len())
        .saturating_add(decoded.reserve_member_rows.len())
        .saturating_add(decoded.terminal_record_rows.len())
        .saturating_add(decoded.width_observation_rows.len())
        .saturating_add(decoded.current_loss_observation_rows.len());
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE)
    {
        domain.status_flags_or |= decoded.status_flags;
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_event_rows",
        row_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_phase_started_rows",
        phase_started_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_phase_updated_rows",
        phase_updated_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_phase_ended_rows",
        phase_ended_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_participant_joined_rows",
        participant_joined_rows,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_detail_participant_left_unclassified_rows",
        participant_left_unclassified_rows,
    );
    // E2: per-side CCombatSide detail rows.
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_side_detail_rows",
        decoded.side_detail_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_attribution_rows",
        decoded.attribution_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_reserve_member_rows",
        decoded.reserve_member_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_terminal_record_rows",
        decoded.terminal_record_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_width_observation_rows",
        decoded.width_observation_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE,
        "land_combat_current_loss_observation_rows",
        decoded.current_loss_observation_rows.len(),
    );
}

fn decode_compact_air_variant_upgrades_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    decode_compact_variant_upgrades_data_payload(
        offset,
        payload,
        summary,
        39,
        "air variant upgrades",
        "air_variant_upgrade",
    );
}

fn decode_compact_stockpile_variant_upgrades_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    decode_compact_variant_upgrades_data_payload(
        offset,
        payload,
        summary,
        40,
        "stockpile variant upgrades",
        "stockpile_variant_upgrade",
    );
}

fn decode_compact_variant_upgrades_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
    domain_id: u16,
    label: &str,
    metric_prefix: &str,
) {
    if payload.len() < 44 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} data payload is {} byte(s), expected at least 44",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_variant_rows = read_u32_le(payload, 20) as usize;
    let observed_upgrade_rows = read_u32_le(payload, 24) as usize;
    let emitted_upgrade_rows = read_u32_le(payload, 28) as usize;
    let invalid_variant_rows = read_u32_le(payload, 32) as usize;
    let reserved0 = read_u32_le(payload, 36);
    let reserved1 = read_u32_le(payload, 40);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_variant_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} row_count {row_count} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if emitted_upgrade_rows > observed_upgrade_rows
        && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} emitted_upgrade_rows {emitted_upgrade_rows} > observed_upgrade_rows {observed_upgrade_rows}"
        ));
    }
    if invalid_variant_rows > observed_variant_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} invalid_variant_rows {invalid_variant_rows} > observed_variant_rows {observed_variant_rows}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} header reserved field is nonzero"
        ));
    }
    let expected_len = 44usize
        .saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_UPGRADE_ROW_LEN))
        .saturating_add(
            emitted_upgrade_rows.saturating_mul(COMPACT_AIR_VARIANT_UPGRADE_ENTRY_ROW_LEN),
        );
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&domain_id) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_rows"),
        row_count,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_observed_variant_rows"),
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_observed_upgrade_rows"),
        observed_upgrade_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_emitted_upgrade_rows"),
        emitted_upgrade_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_invalid_variant_rows"),
        invalid_variant_rows,
    );

    let mut cursor = 44usize;
    let mut seen_variants = BTreeSet::new();
    let mut rows = Vec::with_capacity(row_count);
    let mut declared_upgrades = 0usize;
    for row_index in 0..row_count {
        let variant_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let upgrade_first = read_u32_le(payload, cursor + 8) as usize;
        let upgrade_count = read_u32_le(payload, cursor + 12) as usize;
        cursor += COMPACT_AIR_VARIANT_UPGRADE_ROW_LEN;
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_UPGRADE_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has invalid variant_id {variant_id}"
            ));
        } else if !seen_variants.insert(variant_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} duplicates variant_id {variant_id}"
            ));
        }
        if flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has unknown flags {flags}"
            ));
        }
        let Some(upgrade_end) = upgrade_first.checked_add(upgrade_count) else {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} upgrade range overflows"
            ));
            continue;
        };
        if upgrade_end > emitted_upgrade_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} upgrade range {upgrade_first}+{upgrade_count} exceeds {emitted_upgrade_rows}"
            ));
        }
        declared_upgrades = declared_upgrades.saturating_add(upgrade_count);
        rows.push((upgrade_first, upgrade_count));
    }
    if declared_upgrades != emitted_upgrade_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} emitted_upgrade_rows {emitted_upgrade_rows} != declared {declared_upgrades}"
        ));
    }

    let mut entries = Vec::with_capacity(emitted_upgrade_rows);
    let mut level_sum = 0usize;
    for entry_index in 0..emitted_upgrade_rows {
        let upgrade_index = read_u32_le(payload, cursor);
        let level = read_u32_le(payload, cursor + 4);
        cursor += COMPACT_AIR_VARIANT_UPGRADE_ENTRY_ROW_LEN;
        if upgrade_index >= COMPACT_AIR_VARIANT_UPGRADE_MAX_INDEX {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} upgrade row {entry_index} has invalid upgrade_index {upgrade_index}"
            ));
        }
        if level > COMPACT_AIR_VARIANT_UPGRADE_MAX_LEVEL {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} upgrade row {entry_index} has invalid level {level}"
            ));
        }
        level_sum = level_sum.saturating_add(level as usize);
        entries.push((upgrade_index, level));
    }
    for (row_index, (upgrade_first, upgrade_count)) in rows.iter().enumerate() {
        let mut seen_indexes = BTreeSet::new();
        for entry_index in *upgrade_first..upgrade_first.saturating_add(*upgrade_count) {
            if let Some((upgrade_index, _)) = entries.get(entry_index) {
                if !seen_indexes.insert(*upgrade_index) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: {label} row {row_index} duplicates upgrade_index {upgrade_index}"
                    ));
                }
            }
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_entry_rows"),
        emitted_upgrade_rows,
    );
    add_compact_domain_data_unit(
        summary,
        domain_id,
        &format!("{metric_prefix}_level_sum"),
        level_sum,
    );
}

fn decode_compact_projects_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    if payload.len() < COMPACT_PROJECT_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: projects data payload is {} byte(s), expected at least {COMPACT_PROJECT_HEADER_LEN}",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let facility_count = read_u32_le(payload, 20) as usize;
    let project_count = read_u32_le(payload, 24) as usize;
    let scientist_count = read_u32_le(payload, 28) as usize;
    let observed_facility_rows = read_u32_le(payload, 32) as usize;
    let observed_project_rows = read_u32_le(payload, 36) as usize;
    let observed_scientist_rows = read_u32_le(payload, 40) as usize;
    let bridge_rows_supplied = read_u32_le(payload, 44) as usize;
    let source_kind = read_u32_le(payload, 48);
    let header_extension = read_u32_le(payload, 52);
    let scan_ms = read_u64_le(payload, 56);
    let mut project_state_count = 0usize;
    let mut breakthrough_count = 0usize;
    let mut observed_project_state_rows = 0usize;
    let mut observed_breakthrough_rows = 0usize;
    let mut project_header_len = COMPACT_PROJECT_HEADER_LEN;
    let mut invalid_program_status_countries = 0usize;
    let mut invalid_facility_countries = 0usize;
    let mut invalid_project_pool_countries = 0usize;
    let mut neutral_program_status_countries = 0usize;
    let mut neutral_facility_countries = 0usize;
    let mut neutral_project_pool_countries = 0usize;
    let mut invalid_facility_root_read_fail = 0usize;
    let mut invalid_facility_slot_read_fail = 0usize;
    let mut invalid_facility_candidate_vtable_read_fail = 0usize;
    let mut invalid_facility_candidate_sentinel_after_pin = 0usize;
    let mut invalid_facility_candidate_fields_invalid = 0usize;
    let mut invalid_project_pool_root_read_fail = 0usize;
    let mut invalid_project_pool_vtable_read_fail = 0usize;
    let mut invalid_project_pool_first_project_read_fail = 0usize;
    let mut invalid_project_candidate_vtable_read_fail = 0usize;
    let mut invalid_project_candidate_sentinel_after_pin = 0usize;
    let mut invalid_project_candidate_fields_invalid = 0usize;
    let mut invalid_project_short_array = 0usize;
    match header_extension {
        0 => {}
        COMPACT_PROJECT_HEADER_EXT_ROOT_COUNTERS
        | COMPACT_PROJECT_HEADER_EXT_ROOT_REASON_COUNTERS
        | COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
        | COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
        | COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1
        | COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1 => {
            project_header_len = COMPACT_PROJECT_HEADER_LEN + COMPACT_PROJECT_ROOT_COUNTERS_LEN;
            if payload.len() < project_header_len {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects data payload is {} byte(s), expected at least {project_header_len} for root counter header extension",
                    payload.len()
                ));
                return;
            }
            invalid_program_status_countries = read_u32_le(payload, 64) as usize;
            invalid_facility_countries = read_u32_le(payload, 68) as usize;
            invalid_project_pool_countries = read_u32_le(payload, 72) as usize;
            neutral_program_status_countries = read_u32_le(payload, 76) as usize;
            neutral_facility_countries = read_u32_le(payload, 80) as usize;
            neutral_project_pool_countries = read_u32_le(payload, 84) as usize;
            if header_extension == COMPACT_PROJECT_HEADER_EXT_ROOT_REASON_COUNTERS
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
                || header_extension == COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
            {
                project_header_len += COMPACT_PROJECT_ROOT_REASON_COUNTERS_LEN;
                if payload.len() < project_header_len {
                    summary.parse_errors.push(format!(
                        "offset {offset}: projects data payload is {} byte(s), expected at least {project_header_len} for root reason counter header extension",
                        payload.len()
                    ));
                    return;
                }
                invalid_facility_root_read_fail = read_u32_le(payload, 88) as usize;
                invalid_facility_slot_read_fail = read_u32_le(payload, 92) as usize;
                invalid_facility_candidate_vtable_read_fail = read_u32_le(payload, 96) as usize;
                invalid_facility_candidate_sentinel_after_pin = read_u32_le(payload, 100) as usize;
                invalid_facility_candidate_fields_invalid = read_u32_le(payload, 104) as usize;
                invalid_project_pool_root_read_fail = read_u32_le(payload, 108) as usize;
                invalid_project_pool_vtable_read_fail = read_u32_le(payload, 112) as usize;
                invalid_project_pool_first_project_read_fail = read_u32_le(payload, 116) as usize;
                invalid_project_candidate_vtable_read_fail = read_u32_le(payload, 120) as usize;
                invalid_project_candidate_sentinel_after_pin = read_u32_le(payload, 124) as usize;
                invalid_project_candidate_fields_invalid = read_u32_le(payload, 128) as usize;
                invalid_project_short_array = read_u32_le(payload, 132) as usize;
            }
            if header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
                || header_extension == COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1
                || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
            {
                project_header_len += COMPACT_PROJECT_PROGRESS_COUNTERS_LEN;
                if payload.len() < project_header_len {
                    summary.parse_errors.push(format!(
                        "offset {offset}: projects data payload is {} byte(s), expected at least {project_header_len} for progress header extension",
                        payload.len()
                    ));
                    return;
                }
                project_state_count = read_u32_le(payload, 136) as usize;
                breakthrough_count = read_u32_le(payload, 140) as usize;
                observed_project_state_rows = read_u32_le(payload, 144) as usize;
                observed_breakthrough_rows = read_u32_le(payload, 148) as usize;
            }
        }
        other => {
            summary.parse_errors.push(format!(
                "offset {offset}: projects header extension field has unknown value {other}"
            ));
            return;
        }
    }
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    if row_count
        != facility_count
            .saturating_add(project_count)
            .saturating_add(scientist_count)
            .saturating_add(project_state_count)
            .saturating_add(breakthrough_count)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: projects row_count {row_count} does not match facility/project/scientist/state/breakthrough totals"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: projects changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: projects delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    if !delta_data && facility_count > observed_facility_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: projects facility_count {facility_count} > observed_facility_rows {observed_facility_rows}"
        ));
    }
    if !delta_data && project_count > observed_project_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: projects project_count {project_count} > observed_project_rows {observed_project_rows}"
        ));
    }
    if !delta_data && scientist_count > observed_scientist_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: projects scientist_count {scientist_count} > observed_scientist_rows {observed_scientist_rows}"
        ));
    }
    if !delta_data && project_state_count > observed_project_state_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: projects project_state_count {project_state_count} > observed_project_state_rows {observed_project_state_rows}"
        ));
    }
    if !delta_data && breakthrough_count > observed_breakthrough_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: projects breakthrough_count {breakthrough_count} > observed_breakthrough_rows {observed_breakthrough_rows}"
        ));
    }
    if !matches!(
        source_kind,
        COMPACT_PROJECT_SOURCE_SCAN | COMPACT_PROJECT_SOURCE_BRIDGE
    ) {
        summary.parse_errors.push(format!(
            "offset {offset}: projects has unknown source_kind {source_kind}"
        ));
    }
    if source_kind == COMPACT_PROJECT_SOURCE_BRIDGE && bridge_rows_supplied == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: projects bridge source has zero bridge_rows_supplied"
        ));
    }
    let Some(facility_bytes) = facility_count.checked_mul(COMPACT_PROJECT_FACILITY_ROW_LEN) else {
        summary.parse_errors.push(format!(
            "offset {offset}: projects facility byte count overflow"
        ));
        return;
    };
    let Some(project_bytes) = project_count.checked_mul(COMPACT_PROJECT_ROW_LEN) else {
        summary.parse_errors.push(format!(
            "offset {offset}: projects project byte count overflow"
        ));
        return;
    };
    let Some(scientist_bytes) = scientist_count.checked_mul(COMPACT_PROJECT_SCIENTIST_ROW_LEN)
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: projects scientist byte count overflow"
        ));
        return;
    };
    let Some(project_state_bytes) = project_state_count.checked_mul(COMPACT_PROJECT_STATE_ROW_LEN)
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: projects project state byte count overflow"
        ));
        return;
    };
    let Some(breakthrough_bytes) =
        breakthrough_count.checked_mul(COMPACT_PROJECT_BREAKTHROUGH_ROW_LEN)
    else {
        summary.parse_errors.push(format!(
            "offset {offset}: projects breakthrough byte count overflow"
        ));
        return;
    };
    let fixed_end = project_header_len
        .saturating_add(facility_bytes)
        .saturating_add(project_bytes)
        .saturating_add(scientist_bytes)
        .saturating_add(project_state_bytes)
        .saturating_add(breakthrough_bytes);
    if payload.len() < fixed_end {
        summary.parse_errors.push(format!(
            "offset {offset}: projects data payload is {} byte(s), expected at least {fixed_end}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&41) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 41, "project_facility_rows", facility_count);
    add_compact_domain_data_unit(summary, 41, "project_rows", project_count);
    add_compact_domain_data_unit(summary, 41, "project_scientist_rows", scientist_count);
    add_compact_domain_data_unit(summary, 41, "project_state_rows", project_state_count);
    add_compact_domain_data_unit(summary, 41, "project_breakthrough_rows", breakthrough_count);
    let mut project_completed_state_rows = 0usize;
    add_compact_domain_data_unit(
        summary,
        41,
        "project_observed_facility_rows",
        observed_facility_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_observed_project_rows",
        observed_project_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_observed_scientist_rows",
        observed_scientist_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_observed_project_state_rows",
        observed_project_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_observed_breakthrough_rows",
        observed_breakthrough_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_bridge_rows_supplied",
        bridge_rows_supplied,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_program_status_countries",
        invalid_program_status_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_countries",
        invalid_facility_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_pool_countries",
        invalid_project_pool_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_neutral_program_status_countries",
        neutral_program_status_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_neutral_facility_countries",
        neutral_facility_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_neutral_project_pool_countries",
        neutral_project_pool_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_root_read_fail",
        invalid_facility_root_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_slot_read_fail",
        invalid_facility_slot_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_candidate_vtable_read_fail",
        invalid_facility_candidate_vtable_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_candidate_sentinel_after_pin",
        invalid_facility_candidate_sentinel_after_pin,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_facility_candidate_fields_invalid",
        invalid_facility_candidate_fields_invalid,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_pool_root_read_fail",
        invalid_project_pool_root_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_pool_vtable_read_fail",
        invalid_project_pool_vtable_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_pool_first_project_read_fail",
        invalid_project_pool_first_project_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_candidate_vtable_read_fail",
        invalid_project_candidate_vtable_read_fail,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_candidate_sentinel_after_pin",
        invalid_project_candidate_sentinel_after_pin,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_candidate_fields_invalid",
        invalid_project_candidate_fields_invalid,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_invalid_project_short_array",
        invalid_project_short_array,
    );
    add_compact_domain_data_unit(summary, 41, "project_scan_ms", scan_ms as usize);

    let mut cursor = project_header_len;
    let mut facility_flags = Vec::with_capacity(facility_count);
    let mut facility_deleted_flags = Vec::with_capacity(facility_count);
    let mut seen_facilities = BTreeSet::new();
    let mut deleted_facility_rows = 0usize;
    for row_index in 0..facility_count {
        let id = read_u32_le(payload, cursor);
        let province = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let active_project_handle = read_u32_le(payload, cursor + 12);
        let scientist_char_id = read_u32_le(payload, cursor + 16);
        let unread_rewards = read_u32_le(payload, cursor + 20);
        let reserved0 = read_u32_le(payload, cursor + 24);
        let reserved1 = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_PROJECT_FACILITY_ROW_LEN;
        if id == 0 || id > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has invalid id {id}"
            ));
        } else if !seen_facilities.insert(id) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} duplicates id {id}"
            ));
        }
        let known_flags = COMPACT_PROJECT_FACILITY_ROW_FLAG_ACTIVE
            | COMPACT_PROJECT_FACILITY_ROW_FLAG_SCIENTIST
            | COMPACT_PROJECT_FACILITY_ROW_FLAG_BUILDING
            | COMPACT_PROJECT_FACILITY_ROW_FLAG_DELETED;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_facility_rows = deleted_facility_rows.saturating_add(1);
            if flags != COMPACT_PROJECT_FACILITY_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects facility deleted row {row_index} has live flags"
                ));
            }
            if province != 0
                || active_project_handle != 0
                || scientist_char_id != 0
                || unread_rewards != 0
                || reserved0 != 0
                || reserved1 != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects facility deleted row {row_index} has live fields"
                ));
            }
            facility_flags.push(flags);
            facility_deleted_flags.push(true);
            continue;
        }
        if province == 0 || province > COMPACT_PROJECT_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has invalid province {province}"
            ));
        }
        if active_project_handle > COMPACT_PROJECT_MAX_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has invalid active_project_handle {active_project_handle}"
            ));
        }
        if scientist_char_id > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has invalid scientist_char_id {scientist_char_id}"
            ));
        }
        if scientist_char_id == 0 && flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_SCIENTIST != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has scientist flag without scientist id"
            ));
        }
        if scientist_char_id > 0 && flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_SCIENTIST == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has scientist id without scientist flag"
            ));
        }
        if unread_rewards > COMPACT_PROJECT_MAX_ITERATIONS {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has invalid unread rewards {unread_rewards}"
            ));
        }
        if reserved0 != 0 || reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} reserved fields are nonzero"
            ));
        }
        facility_flags.push(flags);
        facility_deleted_flags.push(false);
    }

    let mut seen_projects = BTreeSet::new();
    let mut deleted_project_rows = 0usize;
    for row_index in 0..project_count {
        let id = read_u32_le(payload, cursor);
        let static_def_handle = read_u32_le(payload, cursor + 4);
        let iterations = read_u32_le(payload, cursor + 8);
        let worked_at_facility = read_u32_le(payload, cursor + 12);
        let flags = read_u32_le(payload, cursor + 16);
        let reserved0 = read_u32_le(payload, cursor + 20);
        cursor += COMPACT_PROJECT_ROW_LEN;
        if id == 0 || id > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has invalid id {id}"
            ));
        } else if !seen_projects.insert(id) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} duplicates id {id}"
            ));
        }
        let known_flags =
            COMPACT_PROJECT_ROW_FLAG_WORKED_AT_FACILITY | COMPACT_PROJECT_ROW_FLAG_DELETED;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = flags & COMPACT_PROJECT_ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_project_rows = deleted_project_rows.saturating_add(1);
            if flags != COMPACT_PROJECT_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects deleted row {row_index} has live flags"
                ));
            }
            if static_def_handle != 0
                || iterations != 0
                || worked_at_facility != 0
                || reserved0 != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects deleted row {row_index} has live fields"
                ));
            }
            continue;
        }
        if static_def_handle == 0 || static_def_handle > COMPACT_PROJECT_MAX_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has invalid static_def_handle {static_def_handle}"
            ));
        }
        if iterations > COMPACT_PROJECT_MAX_ITERATIONS {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has invalid iterations {iterations}"
            ));
        }
        if worked_at_facility == 0 && flags & COMPACT_PROJECT_ROW_FLAG_WORKED_AT_FACILITY != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has worked-at flag without facility"
            ));
        }
        if worked_at_facility > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} has invalid worked_at_facility {worked_at_facility}"
            ));
        }
        if reserved0 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects row {row_index} reserved field is nonzero"
            ));
        }
    }

    let mut scientist_flags = Vec::with_capacity(scientist_count);
    let mut scientist_deleted_flags = Vec::with_capacity(scientist_count);
    let mut seen_scientists = BTreeSet::new();
    let mut deleted_scientist_rows = 0usize;
    for row_index in 0..scientist_count {
        let char_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let trait_count = read_u32_le(payload, cursor + 8);
        let skills = [
            read_i32_le(payload, cursor + 12),
            read_i32_le(payload, cursor + 16),
            read_i32_le(payload, cursor + 20),
            read_i32_le(payload, cursor + 24),
        ];
        let reserved0 = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_PROJECT_SCIENTIST_ROW_LEN;
        if char_id == 0 || char_id > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has invalid char_id {char_id}"
            ));
        } else if !seen_scientists.insert(char_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} duplicates char_id {char_id}"
            ));
        }
        let known_flags = COMPACT_PROJECT_SCIENTIST_ROW_FLAG_NAME_TOKEN
            | COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DISPLAY_NAME
            | COMPACT_PROJECT_SCIENTIST_ROW_FLAG_TRAIT_COUNT
            | COMPACT_PROJECT_SCIENTIST_ROW_FLAG_SKILLS
            | COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DELETED;
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has unknown flags {flags}"
            ));
        }
        let deleted = flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_scientist_rows = deleted_scientist_rows.saturating_add(1);
            if flags != COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects scientist deleted row {row_index} has live flags"
                ));
            }
            if trait_count != 0 || skills != [0, 0, 0, 0] || reserved0 != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects scientist deleted row {row_index} has live fields"
                ));
            }
            scientist_flags.push(flags);
            scientist_deleted_flags.push(true);
            continue;
        }
        if trait_count != u32::MAX && trait_count > COMPACT_PROJECT_MAX_TRAITS {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has invalid trait_count {trait_count}"
            ));
        }
        if trait_count == u32::MAX && flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_TRAIT_COUNT != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has trait flag without trait_count"
            ));
        }
        for skill in skills {
            if !(0..=COMPACT_PROJECT_MAX_SKILL_LEVEL).contains(&skill) {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects scientist row {row_index} has invalid skill level {skill}"
                ));
            }
        }
        if reserved0 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} reserved field is nonzero"
            ));
        }
        scientist_flags.push(flags);
        scientist_deleted_flags.push(false);
    }

    let mut seen_project_states = BTreeSet::new();
    let mut deleted_project_state_rows = 0usize;
    for row_index in 0..project_state_count {
        let id = read_u32_le(payload, cursor);
        let static_def_handle = read_u32_le(payload, cursor + 4);
        let current_state_offset = read_u32_le(payload, cursor + 8);
        let current_state_enum = read_u32_le(payload, cursor + 12);
        let prototype_state_enum = read_u32_le(payload, cursor + 16);
        let simple_state_1_enum = read_u32_le(payload, cursor + 20);
        let simple_state_2_enum = read_u32_le(payload, cursor + 24);
        let simple_state_3_enum = read_u32_le(payload, cursor + 28);
        let prototype_iterations = read_u32_le(payload, cursor + 32);
        let phase_progress_raw = read_u32_le(payload, cursor + 36);
        let phase_target_raw = read_u32_le(payload, cursor + 40);
        let state_flags_or_reserved = read_u32_le(payload, cursor + 44);
        cursor += COMPACT_PROJECT_STATE_ROW_LEN;
        if id == 0 || id > COMPACT_PROJECT_MAX_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid id {id}"
            ));
        } else if !seen_project_states.insert(id) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} duplicates id {id}"
            ));
        }
        let deleted = state_flags_or_reserved & COMPACT_PROJECT_STATE_ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_project_state_rows = deleted_project_state_rows.saturating_add(1);
            if state_flags_or_reserved != COMPACT_PROJECT_STATE_ROW_FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects state deleted row {row_index} has live flags"
                ));
            }
            if static_def_handle != 0
                || current_state_offset != 0
                || current_state_enum != 0
                || prototype_state_enum != 0
                || simple_state_1_enum != 0
                || simple_state_2_enum != 0
                || simple_state_3_enum != 0
                || prototype_iterations != 0
                || phase_progress_raw != 0
                || phase_target_raw != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects state deleted row {row_index} has live fields"
                ));
            }
            continue;
        }
        if static_def_handle == 0 || static_def_handle > COMPACT_PROJECT_MAX_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid static_def_handle {static_def_handle}"
            ));
        }
        if !matches!(current_state_offset, 0x18 | 0xE8 | 0x168 | 0x1E8) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid current_state_offset {current_state_offset:#x}"
            ));
        }
        for (name, value) in [
            ("current_state_enum", current_state_enum),
            ("prototype_state_enum", prototype_state_enum),
            ("simple_state_1_enum", simple_state_1_enum),
            ("simple_state_2_enum", simple_state_2_enum),
            ("simple_state_3_enum", simple_state_3_enum),
        ] {
            if value > COMPACT_PROJECT_MAX_STATE_ENUM {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects state row {row_index} has invalid {name} {value}"
                ));
            }
        }
        if prototype_iterations > COMPACT_PROJECT_MAX_ITERATIONS {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid prototype_iterations {prototype_iterations}"
            ));
        }
        if phase_progress_raw > COMPACT_PROJECT_MAX_PROGRESS_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid phase_progress_raw {phase_progress_raw}"
            ));
        }
        if phase_target_raw == 0 || phase_target_raw > COMPACT_PROJECT_MAX_PHASE_TARGET_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} has invalid phase_target_raw {phase_target_raw}"
            ));
        }
        if header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRESS_V2
            || header_extension == COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1
            || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
        {
            if state_flags_or_reserved
                & !(COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED
                    | COMPACT_PROJECT_STATE_ROW_FLAG_DELETED)
                != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects state row {row_index} has unknown state_flags {state_flags_or_reserved:#x}"
                ));
            }
            if state_flags_or_reserved & COMPACT_PROJECT_STATE_ROW_FLAG_COMPLETED != 0 {
                project_completed_state_rows = project_completed_state_rows.saturating_add(1);
            }
        } else if state_flags_or_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects state row {row_index} reserved field is nonzero"
            ));
        }
    }

    add_compact_domain_data_unit(
        summary,
        41,
        "project_completed_state_rows",
        project_completed_state_rows,
    );

    let mut seen_breakthrough = BTreeSet::new();
    let mut deleted_breakthrough_rows = 0usize;
    for row_index in 0..breakthrough_count {
        let tag_db_id = read_u32_le(payload, cursor);
        let specialization_handle = read_u32_le(payload, cursor + 4);
        let points_raw = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        cursor += COMPACT_PROJECT_BREAKTHROUGH_ROW_LEN;
        let program_status_index = if header_extension
            == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
        {
            (flags & COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_MASK)
                >> COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_SHIFT
        } else {
            0
        };
        if program_status_index >= 3 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} has invalid program_status_index {program_status_index}"
            ));
        }
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if specialization_handle == 0 || specialization_handle > COMPACT_PROJECT_MAX_HANDLE {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} has invalid specialization_handle {specialization_handle}"
            ));
        } else if !seen_breakthrough.insert((
            tag_db_id,
            program_status_index,
            specialization_handle,
        )) {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} duplicates tag/program-status/specialization {tag_db_id}/{program_status_index}/{specialization_handle}"
            ));
        }
        let known_flags = if header_extension
            == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
        {
            COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_KNOWN_FLAGS
        } else if header_extension == COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1 {
            COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_KNOWN_FLAGS
        } else {
            COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED
        };
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} has unknown flags {flags:#x}"
            ));
        }
        let deleted = flags & COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED != 0;
        if deleted {
            deleted_breakthrough_rows = deleted_breakthrough_rows.saturating_add(1);
            let expected_deleted_flags = COMPACT_PROJECT_BREAKTHROUGH_ROW_FLAG_DELETED
                | (program_status_index
                    << COMPACT_PROJECT_BREAKTHROUGH_PROGRAM_STATUS_INDEX_SHIFT);
            if flags != expected_deleted_flags || points_raw != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects breakthrough deleted row {row_index} has live fields"
                ));
            }
            continue;
        }
        if header_extension == COMPACT_PROJECT_HEADER_EXT_SPECIALIZATION_V1
            || header_extension == COMPACT_PROJECT_HEADER_EXT_PROGRAM_STATUS_INDEX_V1
        {
            let specialization_present =
                flags & COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_PRESENT != 0;
            let specialization_code = (flags & COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_MASK)
                >> COMPACT_PROJECT_BREAKTHROUGH_SPECIALIZATION_SHIFT;
            if (!specialization_present && specialization_code != 0)
                || (specialization_present && !(1..=4).contains(&specialization_code))
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: projects breakthrough row {row_index} has invalid specialization flags {flags:#x}"
                ));
            }
        }
        if points_raw > COMPACT_PROJECT_MAX_BREAKTHROUGH_POINTS {
            summary.parse_errors.push(format!(
                "offset {offset}: projects breakthrough row {row_index} has invalid points_raw {points_raw}"
            ));
        }
    }

    add_compact_domain_data_unit(
        summary,
        41,
        "project_facility_deleted_rows",
        deleted_facility_rows,
    );
    add_compact_domain_data_unit(summary, 41, "project_deleted_rows", deleted_project_rows);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_scientist_deleted_rows",
        deleted_scientist_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_state_deleted_rows",
        deleted_project_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_breakthrough_deleted_rows",
        deleted_breakthrough_rows,
    );

    for (row_index, (flags, deleted)) in facility_flags
        .iter()
        .copied()
        .zip(facility_deleted_flags.iter().copied())
        .enumerate()
    {
        let len = match skip_compact_string_len(payload, &mut cursor, "projects facility building")
        {
            Ok(len) => len,
            Err(err) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                return;
            }
        };
        if len > COMPACT_PROJECT_MAX_BUILDING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} building token is too long ({len})"
            ));
        }
        if deleted && len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility deleted row {row_index} has live building string"
            ));
        }
        if len == 0 && flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_BUILDING != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has building flag with empty string"
            ));
        }
        if len > 0 && flags & COMPACT_PROJECT_FACILITY_ROW_FLAG_BUILDING == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects facility row {row_index} has building string without flag"
            ));
        }
    }
    for (row_index, (flags, deleted)) in scientist_flags
        .iter()
        .copied()
        .zip(scientist_deleted_flags.iter().copied())
        .enumerate()
    {
        let token_len =
            match skip_compact_string_len(payload, &mut cursor, "projects scientist token") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if token_len > COMPACT_PROJECT_MAX_SCIENTIST_TOKEN_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} token is too long ({token_len})"
            ));
        }
        if deleted && token_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist deleted row {row_index} has live token string"
            ));
        }
        if token_len == 0 && flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_NAME_TOKEN != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has token flag with empty string"
            ));
        }
        if token_len > 0 && flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_NAME_TOKEN == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has token string without flag"
            ));
        }
        let display_len =
            match skip_compact_string_len(payload, &mut cursor, "projects scientist display") {
                Ok(len) => len,
                Err(err) => {
                    summary.parse_errors.push(format!("offset {offset}: {err}"));
                    return;
                }
            };
        if display_len > COMPACT_PROJECT_MAX_SCIENTIST_DISPLAY_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} display name is too long ({display_len})"
            ));
        }
        if deleted && display_len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist deleted row {row_index} has live display string"
            ));
        }
        if display_len == 0 && flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DISPLAY_NAME != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has display flag with empty string"
            ));
        }
        if display_len > 0 && flags & COMPACT_PROJECT_SCIENTIST_ROW_FLAG_DISPLAY_NAME == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: projects scientist row {row_index} has display string without flag"
            ));
        }
    }
    // Exact lifecycle/reward-history segments follow the self-delimiting base.
    if cursor != payload.len() {
        decode_compact_project_trailers(offset, payload, cursor, summary);
    }
}

fn decode_compact_project_trailers(
    offset: usize,
    payload: &[u8],
    mut cursor: usize,
    summary: &mut DomainDataDecodeSummary,
) {
    let mut saw_lifecycle = false;
    let mut saw_owner_progress = false;
    let mut saw_reward_history = false;
    let mut saw_authority = false;
    while cursor != payload.len() {
        let Some(magic) = payload.get(cursor..cursor.saturating_add(4)) else {
            summary.parse_errors.push(format!(
                "offset {offset}: special-project trailer has truncated magic"
            ));
            return;
        };
        if magic == b"PPA1" {
            const LEN: usize = 208;
            if saw_authority
                || payload.len().saturating_sub(cursor) != LEN
                || read_u16_le(payload, cursor + 4) != 1
                || read_u16_le(payload, cursor + 6) as usize != LEN
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: PPA1 authority trailer is duplicate, nonterminal, or malformed"
                ));
                return;
            }
            saw_authority = true;
            cursor += LEN;
            continue;
        }
        let (header_len, row_len, label) = match magic {
            b"PLCF" => {
                if saw_lifecycle || saw_owner_progress || saw_reward_history || saw_authority {
                    summary.parse_errors.push(format!(
                        "offset {offset}: PLCF trailer is duplicate or out of order"
                    ));
                    return;
                }
                saw_lifecycle = true;
                (20usize, 12usize, "project lifecycle")
            }
            b"PPGF" => {
                if saw_owner_progress || saw_reward_history || saw_authority {
                    summary.parse_errors.push(format!(
                        "offset {offset}: PPGF trailer is duplicate or out of order"
                    ));
                    return;
                }
                saw_owner_progress = true;
                (20usize, 52usize, "project owner-progress")
            }
            b"PRHF" => {
                if saw_reward_history || saw_authority {
                    summary.parse_errors.push(format!(
                        "offset {offset}: PRHF trailer is duplicate"
                    ));
                    return;
                }
                saw_reward_history = true;
                (20usize, 56usize, "project reward-history")
            }
            _ => {
                summary.parse_errors.push(format!(
                    "offset {offset}: special-project trailer magic is invalid"
                ));
                return;
            }
        };
        let Some(header) = payload.get(cursor..cursor.saturating_add(header_len)) else {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} trailer header is truncated"
            ));
            return;
        };
        let row_count = read_u32_le(header, 8) as usize;
        let Some(segment_len) = row_count
            .checked_mul(row_len)
            .and_then(|rows| header_len.checked_add(rows))
        else {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} trailer size overflow"
            ));
            return;
        };
        let Some(next) = cursor.checked_add(segment_len) else {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} trailer end overflow"
            ));
            return;
        };
        if next > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} trailer is truncated"
            ));
            return;
        }
        if magic == b"PLCF" {
            decode_compact_project_lifecycle_trailer(offset, &payload[..next], cursor, summary);
        } else if magic == b"PPGF" {
            decode_compact_project_owner_progress_trailer(offset, &payload[..next], cursor, summary);
        } else {
            decode_compact_project_reward_history_trailer(offset, &payload[..next], cursor, summary);
        }
        cursor = next;
    }
}

fn decode_compact_project_owner_progress_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 52;
    const COMPLETED: u32 = 1;
    const DELETED: u32 = 1 << 31;
    let trailer = &payload[trailer_offset..];
    if trailer.len() < HEADER_LEN || &trailer[0..4] != b"PPGF" {
        summary
            .parse_errors
            .push(format!("offset {offset}: project owner-progress trailer header is invalid"));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    let countries = read_u32_le(trailer, 12) as usize;
    let aborted_countries = read_u32_le(trailer, 16) as usize;
    if version != 1 || header_len != HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: project owner-progress header shape invalid"
        ));
        return;
    }
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if trailer.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: project owner-progress trailer len {} != {expected}",
            trailer.len()
        ));
        return;
    }
    let mut seen = BTreeSet::new();
    let mut deleted_rows = 0usize;
    for row_index in 0..row_count {
        let cursor = HEADER_LEN + row_index * ROW_LEN;
        let tag_db_id = read_u32_le(trailer, cursor);
        let local_id = read_u32_le(trailer, cursor + 4);
        let fields = (0..11)
            .map(|field| read_u32_le(trailer, cursor + 8 + field * 4))
            .collect::<Vec<_>>();
        let flags = fields[10];
        if tag_db_id == 0 || local_id == 0 || !seen.insert((tag_db_id, local_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: project owner-progress row {row_index} has invalid/duplicate key"
            ));
        }
        if flags & !(COMPLETED | DELETED) != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project owner-progress row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if flags & DELETED != 0 {
            deleted_rows += 1;
            if flags != DELETED || fields[..10].iter().any(|value| *value != 0) {
                summary.parse_errors.push(format!(
                    "offset {offset}: project owner-progress deleted row {row_index} has live fields"
                ));
            }
            continue;
        }
        let static_def = fields[0];
        let current_offset = fields[1];
        let phase_progress = fields[8];
        let phase_target = fields[9];
        if static_def == 0
            || !matches!(current_offset, 0x18 | 0xE8 | 0x168 | 0x1E8)
            || fields[2..=6].iter().any(|value| *value > 64)
            || fields[7] > 1_000_000
            || phase_progress > 1_000_000_000
            || phase_target == 0
            || phase_target > 1_000_000
        {
            summary.parse_errors.push(format!(
                "offset {offset}: project owner-progress row {row_index} violates exact bounds"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 41, "project_owner_progress_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_owner_progress_deleted_rows",
        deleted_rows,
    );
    add_compact_domain_data_unit(summary, 41, "project_owner_progress_countries", countries);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_owner_progress_aborted_countries",
        aborted_countries,
    );
}

/// Decode + gate one domain-41 `PLCF` project-lifecycle segment. The isolated
/// segment must consume the supplied payload to its exact end. Flags are STARTED bit0 /
/// COMPLETED bit1 (validated boolean-domain u8s from the collector); the
/// higher bits must be zero. Per-country (tag, local_id) uniqueness is
/// enforced.
fn decode_compact_project_lifecycle_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) {
    const MAGIC: &[u8; 4] = b"PLCF";
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 12;
    const FLAG_STARTED: u32 = 1;
    const FLAG_COMPLETED: u32 = 1 << 1;
    const FLAG_DELETED: u32 = 1 << 31;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary.parse_errors.push(format!(
            "offset {offset}: project lifecycle trailer offset {trailer_offset} beyond payload"
        ));
        return;
    };
    if trailer.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: project lifecycle trailer is {} byte(s), expected at least {HEADER_LEN}",
            trailer.len()
        ));
        return;
    }
    if &trailer[0..4] != MAGIC {
        summary.parse_errors.push(format!(
            "offset {offset}: project lifecycle trailer magic is invalid"
        ));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    let lifecycle_countries = read_u32_le(trailer, 12) as usize;
    let aborted_countries = read_u32_le(trailer, 16) as usize;
    if version != 1 || header_len != HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: project lifecycle trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return;
    }
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if trailer.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: project lifecycle trailer len {} != {expected}",
            trailer.len()
        ));
        return;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut started_rows = 0usize;
    let mut completed_rows = 0usize;
    let mut deleted_rows = 0usize;
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let local_id = read_u32_le(trailer, cursor + 4);
        let flags = read_u32_le(trailer, cursor + 8);
        cursor += ROW_LEN;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project lifecycle row {row_index} has zero tag_db_id"
            ));
        }
        if local_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project lifecycle row {row_index} has zero local_id"
            ));
        }
        if flags & !(FLAG_STARTED | FLAG_COMPLETED | FLAG_DELETED) != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project lifecycle row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if !seen.insert((tag_db_id, local_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: project lifecycle row {row_index} duplicates (tag {tag_db_id}, local_id {local_id})"
            ));
        }
        let deleted = flags & FLAG_DELETED != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != FLAG_DELETED {
                summary.parse_errors.push(format!(
                    "offset {offset}: project lifecycle deleted row {row_index} has live flags"
                ));
            }
            continue;
        }
        if flags & FLAG_STARTED != 0 {
            started_rows = started_rows.saturating_add(1);
        }
        if flags & FLAG_COMPLETED != 0 {
            completed_rows = completed_rows.saturating_add(1);
        }
    }
    add_compact_domain_data_unit(summary, 41, "project_lifecycle_rows", row_count);
    add_compact_domain_data_unit(summary, 41, "project_lifecycle_started_rows", started_rows);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_lifecycle_completed_rows",
        completed_rows,
    );
    add_compact_domain_data_unit(summary, 41, "project_lifecycle_deleted_rows", deleted_rows);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_lifecycle_countries",
        lifecycle_countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_lifecycle_aborted_countries",
        aborted_countries,
    );
}

fn decode_compact_project_reward_history_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) {
    const HEADER_LEN: usize = 20;
    const ROW_LEN: usize = 56;
    const CHARACTER_REF_NONZERO: u32 = 1;
    const COUNTRY_PRESENT: u32 = 1 << 1;
    const SCIENTIST_PRESENT: u32 = 1 << 2;
    const STATE_PRESENT: u32 = 1 << 3;
    const DELETED: u32 = 1 << 31;
    const KNOWN_FLAGS: u32 =
        CHARACTER_REF_NONZERO | COUNTRY_PRESENT | SCIENTIST_PRESENT | STATE_PRESENT | DELETED;
    let Some(trailer) = payload.get(trailer_offset..) else {
        summary.parse_errors.push(format!(
            "offset {offset}: project reward-history trailer offset beyond payload"
        ));
        return;
    };
    if trailer.len() < HEADER_LEN || &trailer[0..4] != b"PRHF" {
        summary.parse_errors.push(format!(
            "offset {offset}: project reward-history trailer header is invalid"
        ));
        return;
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    let countries = read_u32_le(trailer, 12) as usize;
    let aborted_countries = read_u32_le(trailer, 16) as usize;
    if version != 1 || header_len != HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: project reward-history trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return;
    }
    let expected = HEADER_LEN.saturating_add(row_count.saturating_mul(ROW_LEN));
    if trailer.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: project reward-history trailer len {} != {expected}",
            trailer.len()
        ));
        return;
    }
    let mut cursor = HEADER_LEN;
    let mut seen = BTreeSet::new();
    let mut deleted_rows = 0usize;
    for row_index in 0..row_count {
        let tag_db_id = read_u32_le(trailer, cursor);
        let local_id = read_u32_le(trailer, cursor + 4);
        let history_index = read_u32_le(trailer, cursor + 8);
        let prototype_reward = read_u32_le(trailer, cursor + 12);
        let option = read_u32_le(trailer, cursor + 16);
        let province = read_i32_le(trailer, cursor + 20);
        let progress = read_u32_le(trailer, cursor + 24);
        let character_type = read_u32_le(trailer, cursor + 28);
        let character_id = read_u32_le(trailer, cursor + 32);
        let country = read_u32_le(trailer, cursor + 36);
        let scientist = read_u32_le(trailer, cursor + 40);
        let state = read_u32_le(trailer, cursor + 44);
        let date = read_i32_le(trailer, cursor + 48);
        let flags = read_u32_le(trailer, cursor + 52);
        cursor += ROW_LEN;
        if tag_db_id == 0 || local_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project reward-history row {row_index} has zero tag/local key"
            ));
        }
        if !seen.insert((tag_db_id, local_id, history_index)) {
            summary.parse_errors.push(format!(
                "offset {offset}: project reward-history row {row_index} duplicates its key"
            ));
        }
        if flags & !KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: project reward-history row {row_index} has unknown flags {flags:#x}"
            ));
        }
        let deleted = flags & DELETED != 0;
        if deleted {
            deleted_rows = deleted_rows.saturating_add(1);
            if flags != DELETED
                || prototype_reward != 0
                || option != 0
                || province != 0
                || progress != 0
                || character_type != 0
                || character_id != 0
                || country != 0
                || scientist != 0
                || state != 0
                || date != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: project reward-history deleted row {row_index} has live fields"
                ));
            }
            continue;
        }
        let character_nonzero = character_type != 0 || character_id != 0;
        if character_nonzero != (flags & CHARACTER_REF_NONZERO != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: project reward-history row {row_index} character-ref flag mismatch"
            ));
        }
        for (label, raw, flag) in [
            ("country", country, COUNTRY_PRESENT),
            ("scientist", scientist, SCIENTIST_PRESENT),
            ("state", state, STATE_PRESENT),
        ] {
            if flags & flag == 0 && raw != 0 {
                summary.parse_errors.push(format!(
                    "offset {offset}: project reward-history row {row_index} has {label} ref without presence flag"
                ));
            }
        }
    }
    add_compact_domain_data_unit(summary, 41, "project_reward_history_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        41,
        "project_reward_history_deleted_rows",
        deleted_rows,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_reward_history_countries",
        countries,
    );
    add_compact_domain_data_unit(
        summary,
        41,
        "project_reward_history_aborted_countries",
        aborted_countries,
    );
}

fn decode_compact_market_trade_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_MARKET_TRADE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade data payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_MARKET_TRADE_HEADER_LEN
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_contract_rows = read_u32_le(payload, 20) as usize;
    let emitted_contract_rows = read_u32_le(payload, 24) as usize;
    let buyer_ptr_valid_rows = read_u32_le(payload, 28) as usize;
    let variant_valid_rows = read_u32_le(payload, 32) as usize;
    let amount_valid_rows = read_u32_le(payload, 36) as usize;
    let partial_field_rows = read_u32_le(payload, 40) as usize;
    let scale_raw = read_u32_le(payload, 44);
    add_decoder_status(summary, status_flags);

    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if emitted_contract_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade emitted_contract_rows {emitted_contract_rows} != row_count {row_count}"
        ));
    }
    if observed_contract_rows < row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade observed_contract_rows {observed_contract_rows} < row_count {row_count}"
        ));
    }
    let omitted_candidate_rows = observed_contract_rows.saturating_sub(emitted_contract_rows);
    for (name, count) in [
        ("buyer_ptr_valid_rows", buyer_ptr_valid_rows),
        ("variant_valid_rows", variant_valid_rows),
        ("amount_valid_rows", amount_valid_rows),
        ("partial_field_rows", partial_field_rows),
    ] {
        if count > row_count {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade {name} {count} > row_count {row_count}"
            ));
        }
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    let core_end = COMPACT_MARKET_TRADE_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_MARKET_TRADE_ROW_LEN));
    if payload.len() < core_end {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade data payload is {} byte(s), expected at least {core_end}",
            payload.len()
        ));
        return;
    }

    if let Some(domain) = summary.per_domain.get_mut(&43) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 43, "market_trade_contract_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_observed_contract_rows",
        observed_contract_rows,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_omitted_candidate_rows",
        omitted_candidate_rows,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_buyer_ptr_valid_rows",
        buyer_ptr_valid_rows,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_variant_valid_rows",
        variant_valid_rows,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_amount_valid_rows",
        amount_valid_rows,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_partial_field_rows",
        partial_field_rows,
    );

    let mut seen_contracts = BTreeSet::new();
    let mut computed_buyer_valid = 0usize;
    let mut computed_variant_valid = 0usize;
    let mut computed_amount_valid = 0usize;
    let mut computed_partial_fields = 0usize;
    let known_flags = COMPACT_MARKET_TRADE_ROW_FLAG_BUYER_PTR_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_EQUIPMENT_SUBBUF_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_DELIVERY_SUBBUF_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_VARIANT_VALID
        | COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID;
    let mut cursor = COMPACT_MARKET_TRADE_HEADER_LEN;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_MARKET_TRADE_ROW_LEN,
            "market trade row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let contract_id = read_u32_le(payload, cursor);
        let contract_type = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let seller_tag_db_id = read_u32_le(payload, cursor + 12);
        let buyer_tag_db_id = read_u32_le(payload, cursor + 16);
        let equipment_variant_id = read_u32_le(payload, cursor + 20);
        let amount_raw = read_i64_le(payload, cursor + 24);
        let equipment_subbuf_status = read_u32_le(payload, cursor + 32);
        let delivery_subbuf_status = read_u32_le(payload, cursor + 36);
        let buyer_ptr_status = read_u32_le(payload, cursor + 40);
        let reserved = read_u32_le(payload, cursor + 44);
        cursor += COMPACT_MARKET_TRADE_ROW_LEN;

        let buyer_ptr_valid = flags & COMPACT_MARKET_TRADE_ROW_FLAG_BUYER_PTR_VALID != 0;
        let equipment_subbuf_valid =
            flags & COMPACT_MARKET_TRADE_ROW_FLAG_EQUIPMENT_SUBBUF_VALID != 0;
        let delivery_subbuf_valid =
            flags & COMPACT_MARKET_TRADE_ROW_FLAG_DELIVERY_SUBBUF_VALID != 0;
        let variant_valid = flags & COMPACT_MARKET_TRADE_ROW_FLAG_VARIANT_VALID != 0;
        let amount_valid = flags & COMPACT_MARKET_TRADE_ROW_FLAG_AMOUNT_VALID != 0;
        if buyer_ptr_valid {
            computed_buyer_valid = computed_buyer_valid.saturating_add(1);
        }
        if variant_valid {
            computed_variant_valid = computed_variant_valid.saturating_add(1);
        }
        if amount_valid {
            computed_amount_valid = computed_amount_valid.saturating_add(1);
        }
        if !buyer_ptr_valid
            || !equipment_subbuf_valid
            || !delivery_subbuf_valid
            || !variant_valid
            || !amount_valid
        {
            computed_partial_fields = computed_partial_fields.saturating_add(1);
        }

        if contract_id == 0 || contract_id > COMPACT_MARKET_TRADE_MAX_CONTRACT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has invalid contract_id {contract_id}"
            ));
        } else if !seen_contracts.insert(contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} duplicates contract_id {contract_id}"
            ));
        }
        if contract_type != COMPACT_MARKET_TRADE_CONTRACT_TYPE {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has contract_type {contract_type}, expected {COMPACT_MARKET_TRADE_CONTRACT_TYPE}"
            ));
        }
        for (name, tag_db_id) in [
            ("seller_tag_db_id", seller_tag_db_id),
            ("buyer_tag_db_id", buyer_tag_db_id),
        ] {
            if tag_db_id == 0 || tag_db_id > COMPACT_MARKET_TRADE_MAX_TAG_DB_ID {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} has invalid {name} {tag_db_id}"
                ));
            }
        }
        if flags & !known_flags != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has unknown flags {flags}"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} reserved field is nonzero"
            ));
        }
        if buyer_ptr_status > 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has invalid buyer_ptr_status {buyer_ptr_status}"
            ));
        }
        if buyer_ptr_valid != (buyer_ptr_status == 1) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} buyer_ptr_valid flag disagrees with status {buyer_ptr_status}"
            ));
        }
        for (name, status) in [
            ("equipment_subbuf_status", equipment_subbuf_status),
            ("delivery_subbuf_status", delivery_subbuf_status),
        ] {
            if status > 2 {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} has invalid {name} {status}"
                ));
            }
        }
        if equipment_subbuf_valid != (equipment_subbuf_status == 1) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} equipment valid flag disagrees with status {equipment_subbuf_status}"
            ));
        }
        if delivery_subbuf_valid != (delivery_subbuf_status == 1) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} delivery valid flag disagrees with status {delivery_subbuf_status}"
            ));
        }
        if variant_valid {
            if equipment_variant_id == 0
                || equipment_variant_id > COMPACT_MARKET_TRADE_MAX_VARIANT_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} has invalid equipment_variant_id {equipment_variant_id}"
                ));
            }
            if !equipment_subbuf_valid {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} marks variant valid without valid equipment subbuffer"
                ));
            }
        } else if equipment_variant_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has equipment_variant_id {equipment_variant_id} with variant flag unset"
            ));
        }
        if amount_valid {
            if amount_raw <= 0 || amount_raw > COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} has invalid amount_raw {amount_raw}"
                ));
            }
            if !delivery_subbuf_valid {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade row {row_index} marks amount valid without valid delivery subbuffer"
                ));
            }
        } else if amount_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade row {row_index} has amount_raw {amount_raw} with amount flag unset"
            ));
        }
    }

    for (name, expected, actual) in [
        (
            "buyer_ptr_valid_rows",
            buyer_ptr_valid_rows,
            computed_buyer_valid,
        ),
        (
            "variant_valid_rows",
            variant_valid_rows,
            computed_variant_valid,
        ),
        (
            "amount_valid_rows",
            amount_valid_rows,
            computed_amount_valid,
        ),
        (
            "partial_field_rows",
            partial_field_rows,
            computed_partial_fields,
        ),
    ] {
        if expected != actual {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade header {name} {expected} != computed {actual}"
            ));
        }
    }
    // The core rows may be followed by an optional MTD1 detail trailer and/or an
    // optional MTRF resource-flow trailer. The collector appends MTD1 only when
    // there are detail rows and MTRF only when resource flow is present, and the
    // two are independent — so MTRF can appear WITHOUT MTD1. Dispatch by trailer
    // magic (mirroring the world_fold trailer walk) rather than assuming MTD1 is
    // always first; otherwise an MTD1-absent/MTRF-present record false-fails.
    let mut detail_rows = 0usize;
    let mut detail_seen = false;
    let mut stockpile_seen = false;
    let mut activation_seen = false;
    while cursor < payload.len() {
        let Some(magic) = payload.get(cursor..cursor.saturating_add(4)) else {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade trailer is too short to contain magic"
            ));
            break;
        };
        let next = if magic == COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC {
            if detail_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade has duplicate MTD1 trailer"
                ));
                break;
            }
            let (rows, end) = decode_compact_market_trade_detail_trailer(
                offset,
                payload,
                cursor,
                &seen_contracts,
                summary,
            );
            detail_rows = rows;
            detail_seen = true;
            end
        } else if magic == COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC {
            decode_compact_market_trade_resource_flow_trailer(offset, payload, cursor, summary)
        } else if magic == COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_MAGIC {
            decode_compact_market_trade_sized_trailer(
                offset,
                payload,
                cursor,
                &[COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_VERSION],
                COMPACT_MARKET_TRADE_FACTORY_HEADER_TRAILER_HEADER_LEN,
                COMPACT_MARKET_TRADE_FACTORY_HEADER_ROW_LEN,
                "MTRH",
                "market_trade_factory_header_rows",
                summary,
            )
        } else if magic == COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC {
            decode_compact_market_trade_sized_trailer(
                offset,
                payload,
                cursor,
                &[
                    COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_LEGACY_VERSION,
                    COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_COUNT_ONLY_VERSION,
                    COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_VERSION,
                    COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_NEEDED_UNAVAILABLE_VERSION,
                ],
                COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_HEADER_LEN,
                COMPACT_MARKET_TRADE_RESOURCE_ROW_LEN,
                "MTRR",
                "market_trade_resource_rows",
                summary,
            )
        } else if magic == COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC {
            decode_compact_market_trade_classic_deal_trailer(offset, payload, cursor, summary)
        } else if magic == COMPACT_MARKET_TRADE_ACTIVATION_TRAILER_MAGIC {
            if activation_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade has duplicate MTRA trailer"
                ));
                break;
            }
            activation_seen = true;
            decode_compact_market_trade_activation_trailer(offset, payload, cursor, summary)
        } else if magic == COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_MAGIC {
            if stockpile_seen {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade has duplicate MTS1 trailer"
                ));
                break;
            }
            stockpile_seen = true;
            decode_compact_market_trade_stockpile_trailer(offset, payload, cursor, summary)
        } else {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade trailing bytes have unknown trailer magic"
            ));
            break;
        };
        if next <= cursor {
            break;
        }
        cursor = next;
    }
    add_compact_domain_data_unit(summary, 43, "market_trade_detail_rows", detail_rows);
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_omitted_rows",
        row_count.saturating_sub(detail_rows),
    );
}

fn decode_compact_market_trade_detail_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    core_contracts: &BTreeSet<u32>,
    summary: &mut DomainDataDecodeSummary,
) -> (usize, usize) {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4))
        != Some(COMPACT_MARKET_TRADE_DETAIL_TRAILER_MAGIC.as_slice())
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade trailing bytes are not MTD1 detail trailer"
        ));
        return (0, payload.len());
    }
    if let Err(err) = require_compact_remaining(
        payload,
        trailer_offset,
        COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN,
        "market trade MTD1 trailer header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return (0, payload.len());
    }

    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16_le(payload, trailer_offset + 10);
    let row_count = read_u32_le(payload, trailer_offset + 12) as usize;
    let price_valid_rows = read_u32_le(payload, trailer_offset + 16) as usize;
    let speed_valid_rows = read_u32_le(payload, trailer_offset + 20) as usize;
    let delivery_route_valid_rows = read_u32_le(payload, trailer_offset + 24) as usize;
    let progress_valid_rows = read_u32_le(payload, trailer_offset + 28) as usize;

    if version != COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 version {version} != {}",
            COMPACT_MARKET_TRADE_DETAIL_TRAILER_VERSION
        ));
    }
    if header_len != COMPACT_MARKET_TRADE_DETAIL_TRAILER_HEADER_LEN
        || row_len != COMPACT_MARKET_TRADE_DETAIL_ROW_LEN
        || reserved0 != 0
    {
        // Return early: the fixed-offset row reads below assume the canonical
        // 32-byte header and 64-byte rows. A crafted header claiming a different
        // header_len/row_len (e.g. row_len=0) would otherwise let mtd1_end pass
        // the bounds check yet index past each short row and panic. The fold
        // likewise bails on a bad MTD1 header shape.
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 header shape is invalid"
        ));
        return (row_count, payload.len());
    }

    // The MTD1 detail trailer occupies exactly `header_len + row_count*row_len`
    // bytes starting at `trailer_offset`. It may be followed by an optional MTRF
    // resource-flow trailer, so the MTD1 block must fit WITHIN the payload (its
    // end is a running cursor the caller resumes from) rather than consume the
    // payload to its very end. Requiring `== payload.len()` here would false-fail
    // every record that carries the resource-flow trailer (the observed
    // "MTD1 len 10904 != 6912" is exactly this: 6912 is the MTD1 end, the extra
    // 3992 bytes are the trailing MTRF block).
    let mtd1_end = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if mtd1_end > payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 end {mtd1_end} exceeds payload len {}",
            payload.len()
        ));
        return (row_count, payload.len());
    }

    let mut seen_detail_contracts = BTreeSet::new();
    let mut computed_price = 0usize;
    let mut computed_speed = 0usize;
    let mut computed_completed = 0usize;
    let mut computed_days = 0usize;
    let mut computed_delivery_country = 0usize;
    let mut computed_convoys = 0usize;
    let mut computed_efficiency = 0usize;
    let mut computed_delivery_route = 0usize;
    let mut computed_progress = 0usize;
    let mut cursor = trailer_offset + header_len;
    for row_index in 0..row_count {
        let contract_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let price_raw = read_i64_le(payload, cursor + 8);
        let amount_raw = read_i64_le(payload, cursor + 16);
        let speed_raw = read_i32_le(payload, cursor + 24);
        let completed_raw = read_i32_le(payload, cursor + 28);
        let days = read_i32_le(payload, cursor + 32);
        let delivery_country_tag_db_id = read_u32_le(payload, cursor + 36);
        let convoys_assigned = read_i32_le(payload, cursor + 40);
        let convoys_total = read_i32_le(payload, cursor + 44);
        let efficiency_raw = read_i32_le(payload, cursor + 48);
        let factory_cic_progress_raw = read_i32_le(payload, cursor + 52);
        let equipment_transfer_progress_raw = read_i32_le(payload, cursor + 56);
        let reserved = read_u32_le(payload, cursor + 60);
        cursor += row_len;

        if contract_id == 0 || contract_id > COMPACT_MARKET_TRADE_MAX_CONTRACT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid contract_id {contract_id}"
            ));
        } else if !core_contracts.contains(&contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} references missing contract_id {contract_id}"
            ));
        } else if !seen_detail_contracts.insert(contract_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} duplicates contract_id {contract_id}"
            ));
        }
        if flags == 0 || flags & !COMPACT_MARKET_TRADE_DETAIL_FLAG_KNOWN_MASK != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid flags {flags:#x}"
            ));
        }
        if reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} reserved field is nonzero"
            ));
        }
        validate_market_trade_detail_presence_i64(
            summary,
            offset,
            row_index,
            "price_raw",
            price_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0,
            1,
            COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW,
        );
        validate_market_trade_detail_presence_i64(
            summary,
            offset,
            row_index,
            "amount_raw",
            amount_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_AMOUNT_VALID != 0,
            1,
            COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "speed_raw",
            speed_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0,
            0,
            1_000_000_000,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "completed_raw",
            completed_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0,
            0,
            1,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "days",
            days,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0,
            -1,
            100_000,
        );
        if flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0 {
            if delivery_country_tag_db_id == 0
                || delivery_country_tag_db_id > COMPACT_MARKET_TRADE_MAX_TAG_DB_ID
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTD1 row {row_index} has invalid delivery_country_tag_db_id {delivery_country_tag_db_id}"
                ));
            }
        } else if delivery_country_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has delivery_country_tag_db_id without flag"
            ));
        }
        if flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0 {
            if convoys_assigned < 0 || convoys_total < 0 || convoys_assigned > convoys_total {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTD1 row {row_index} has invalid convoy counts {convoys_assigned}/{convoys_total}"
                ));
            }
        } else if convoys_assigned != 0 || convoys_total != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has convoy counts without flag"
            ));
        }
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "efficiency_raw",
            efficiency_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "factory_cic_progress_raw",
            factory_cic_progress_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );
        validate_market_trade_detail_presence_i32(
            summary,
            offset,
            row_index,
            "equipment_transfer_progress_raw",
            equipment_transfer_progress_raw,
            flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID != 0,
            0,
            COMPACT_FIXED_SCALE_RAW as i32,
        );

        computed_price += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_PRICE_VALID != 0);
        computed_speed += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_SPEED_VALID != 0);
        computed_completed +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_COMPLETED_VALID != 0);
        computed_days += usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DAYS_VALID != 0);
        computed_delivery_country +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID != 0);
        computed_convoys +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID != 0);
        computed_efficiency +=
            usize::from(flags & COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID != 0);
        computed_delivery_route += usize::from(
            flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_DELIVERY_COUNTRY_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_CONVOYS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EFFICIENCY_VALID)
                != 0,
        );
        computed_progress += usize::from(
            flags
                & (COMPACT_MARKET_TRADE_DETAIL_FLAG_FACTORY_PROGRESS_VALID
                    | COMPACT_MARKET_TRADE_DETAIL_FLAG_EQUIPMENT_PROGRESS_VALID)
                != 0,
        );
    }

    for (name, expected, actual) in [
        ("price_valid_rows", price_valid_rows, computed_price),
        ("speed_valid_rows", speed_valid_rows, computed_speed),
        (
            "delivery_route_valid_rows",
            delivery_route_valid_rows,
            computed_delivery_route,
        ),
        (
            "progress_valid_rows",
            progress_valid_rows,
            computed_progress,
        ),
    ] {
        if expected != actual {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 {name} {expected} != computed {actual}"
            ));
        }
    }

    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_price_valid_rows",
        computed_price,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_speed_valid_rows",
        computed_speed,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_completed_valid_rows",
        computed_completed,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_days_valid_rows",
        computed_days,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_delivery_country_valid_rows",
        computed_delivery_country,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_convoy_valid_rows",
        computed_convoys,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_efficiency_valid_rows",
        computed_efficiency,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_delivery_route_valid_rows",
        computed_delivery_route,
    );
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_detail_progress_valid_rows",
        computed_progress,
    );
    (row_count, mtd1_end)
}

fn decode_compact_market_trade_activation_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) -> usize {
    if let Err(error) = require_compact_remaining(
        payload,
        trailer_offset,
        COMPACT_MARKET_TRADE_ACTIVATION_HEADER_LEN,
        "market trade MTRA activation header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {error}"));
        return payload.len();
    }
    let trailer = &payload[trailer_offset..trailer_offset + COMPACT_MARKET_TRADE_ACTIVATION_HEADER_LEN];
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let flags = read_u32_le(trailer, 8);
    let country_tag_db_id = read_u32_le(trailer, 12);
    let observation_sequence = read_u64_le(trailer, 16);
    let root_kind = read_u32_le(trailer, 24);
    let open_rva = read_u32_le(trailer, 28);
    let active = flags & COMPACT_MARKET_TRADE_ACTIVATION_FLAG_ACTIVE != 0;
    if version != COMPACT_MARKET_TRADE_ACTIVATION_TRAILER_VERSION
        || header_len != COMPACT_MARKET_TRADE_ACTIVATION_HEADER_LEN
        || flags & !COMPACT_MARKET_TRADE_ACTIVATION_FLAGS_KNOWN != 0
        || flags & 2 == 0
        || active != (country_tag_db_id != 0)
        || (active && observation_sequence == 0)
        || country_tag_db_id > COMPACT_MAX_OWNER_TAG_DB_ID
        || root_kind != COMPACT_MARKET_TRADE_ACTIVATION_ROOT_COUNTRYTRADEVIEW
        || open_rva != COMPACT_MARKET_TRADE_ACTIVATION_OPEN_RVA
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRA activation identity/shape is invalid"
        ));
    }
    add_compact_domain_data_unit(summary, 43, "market_trade_activation_rows", 1);
    trailer_offset + COMPACT_MARKET_TRADE_ACTIVATION_HEADER_LEN
}

// Decode the optional MTRF resource-flow trailer that follows the MTD1 detail
// trailer. Validates the magic, header shape, and that the trailer's fixed-width
// rows consume the payload EXACTLY to its end (any residual bytes are a torn
// payload). Structural-only: it accounts for the bytes and surfaces the row
// count as a data unit; it does not re-derive per-resource semantics (those are
// folded by world_fold, not the summarizer).
fn decode_compact_market_trade_resource_flow_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) -> usize {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4))
        != Some(COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_MAGIC.as_slice())
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade trailing bytes are not MTRF resource-flow trailer"
        ));
        return trailer_offset;
    }
    if let Err(err) = require_compact_remaining(
        payload,
        trailer_offset,
        COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN,
        "market trade MTRF trailer header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return trailer_offset;
    }

    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16_le(payload, trailer_offset + 10);
    let row_count = read_u32_le(payload, trailer_offset + 12) as usize;
    let count_or_country_count = read_u32_le(payload, trailer_offset + 16) as usize;
    let scale_raw = read_u32_le(payload, trailer_offset + 20);
    // `+24` is a seven-bit resource MASK in v1/v2 and the resource COUNT in v3.
    // The version decides which reading applies; a stream is never read under a
    // grammar it did not declare.
    let mask_or_count = read_u32_le(payload, trailer_offset + 24);
    let reserved_or_kind_mask = read_u32_le(payload, trailer_offset + 28);

    let resource_count = match version {
        COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_LEGACY_VERSION
        | COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_FIXED7_VERSION => {
            if mask_or_count != COMPACT_MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTRF resource_mask {mask_or_count:#x} != {:#x}",
                    COMPACT_MARKET_TRADE_RESOURCE_FLOW_RESOURCE_MASK
                ));
            }
            COMPACT_MARKET_TRADE_FIXED7_RESOURCE_COUNT
        }
        COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_VERSION => {
            if !(1..=COMPACT_MAX_STRATEGIC_RESOURCE_COUNT).contains(&mask_or_count) {
                // Return early: `expected_len` below multiplies by a row length
                // derived from this count, so an out-of-bound count must not be
                // trusted to size anything.
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTRF v3 resource_count {mask_or_count} is outside 1..={COMPACT_MAX_STRATEGIC_RESOURCE_COUNT}"
                ));
                return trailer_offset;
            }
            mask_or_count
        }
        _ => {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTRF version {version} is unsupported"
            ));
            return trailer_offset;
        }
    };
    let expected_row_len = if version == COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_VERSION {
        COMPACT_MARKET_TRADE_RESOURCE_FLOW_ROW_HEADER_LEN
            .saturating_add((resource_count as usize).saturating_mul(8))
    } else {
        COMPACT_MARKET_TRADE_RESOURCE_FLOW_FIXED7_ROW_LEN
    };
    if header_len != COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_HEADER_LEN
        || row_len != expected_row_len
        || reserved0 != 0
    {
        // Return early: `expected_len` below multiplies by the attacker-supplied
        // row_len, so a bad header shape must not be trusted to derive the length.
        // Mirrors the fold, which rejects a torn MTRF header (incl. nonzero
        // reserved fields).
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRF header shape is invalid"
        ));
        return trailer_offset;
    }
    if version == COMPACT_MARKET_TRADE_RESOURCE_FLOW_TRAILER_LEGACY_VERSION {
        if count_or_country_count != row_count || reserved_or_kind_mask != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade legacy MTRF count/reserved fields are invalid"
            ));
        }
    } else if row_count
        != count_or_country_count
            .saturating_mul(COMPACT_MARKET_TRADE_RESOURCE_FLOW_KIND_MASK.count_ones() as usize)
        || reserved_or_kind_mask != COMPACT_MARKET_TRADE_RESOURCE_FLOW_KIND_MASK
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRF country/kind shape is invalid"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRF scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }

    let expected_len = trailer_offset
        .saturating_add(header_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRF len {} < {expected_len}",
            payload.len()
        ));
        return trailer_offset;
    }

    add_compact_domain_data_unit(summary, 43, "market_trade_resource_flow_rows", row_count);
    expected_len
}

fn decode_compact_market_trade_sized_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    accepted_versions: &[u16],
    expected_header_len: usize,
    expected_row_len: usize,
    label: &str,
    unit_name: &str,
    summary: &mut DomainDataDecodeSummary,
) -> usize {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4)) != Some(label.as_bytes()) {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade trailing bytes are not {label} trailer"
        ));
        return trailer_offset;
    }
    if let Err(err) = require_compact_remaining(
        payload,
        trailer_offset,
        expected_header_len,
        "market trade sized trailer header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return trailer_offset;
    }

    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16_le(payload, trailer_offset + 10);
    let row_count = read_u32_le(payload, trailer_offset + 12) as usize;
    if !accepted_versions.contains(&version)
        || header_len != expected_header_len
        || row_len != expected_row_len
        || reserved0 != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade {label} header shape is invalid (version {version})"
        ));
        return trailer_offset;
    }

    let mut identity_len = 0usize;
    let shape_valid = if label == "MTRH" {
        let country_count = read_u32_le(payload, trailer_offset + 16) as usize;
        let reserved1 = read_u32_le(payload, trailer_offset + 20);
        country_count == row_count && reserved1 == 0
    } else if label == "MTRR" {
        let country_count = read_u32_le(payload, trailer_offset + 16) as usize;
        let resource_count = read_u32_le(payload, trailer_offset + 20);
        let display_scale = read_u32_le(payload, trailer_offset + 24);
        let reserved1 = read_u32_le(payload, trailer_offset + 28);
        // The count is a wire field. v1 pinned it to seven; v2 and v3 admit
        // whatever the live resource database reported, under the one shared
        // ceiling. This is the SAME predicate the fold applies -- both sides
        // bound the count by `MAX_COMPACT_STRATEGIC_RESOURCE_COUNT`, so neither
        // can admit a stream the other refuses.
        let count_admitted = if version == COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_LEGACY_VERSION
        {
            resource_count == COMPACT_MARKET_TRADE_FIXED7_RESOURCE_COUNT
        } else {
            (1..=COMPACT_MAX_STRATEGIC_RESOURCE_COUNT).contains(&resource_count)
        };
        // `+28` is reserved zero in v1/v2 and the identity stride in v3 and v4.
        // The VERSION selects the reading, never the value -- which is what
        // stops an older decoder from sizing an identity-bearing trailer as
        // though the identity block were rows.
        let carries_identity = matches!(
            version,
            COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_VERSION
                | COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_NEEDED_UNAVAILABLE_VERSION
        );
        let identity_field_valid = if carries_identity {
            reserved1 == COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE
        } else {
            reserved1 == 0
        };
        if carries_identity {
            identity_len = (resource_count as usize)
                .saturating_mul(COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE as usize);
        }
        count_admitted
            && display_scale == 1
            && identity_field_valid
            && row_count == country_count.saturating_mul(resource_count as usize)
    } else {
        false
    };
    if !shape_valid {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade {label} count/shape fields are invalid"
        ));
        return trailer_offset;
    }

    let expected_len = trailer_offset
        .saturating_add(header_len)
        .saturating_add(identity_len)
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade {label} len {} < {expected_len}",
            payload.len()
        ));
        return trailer_offset;
    }
    // Sizing the identity block is not reading it. This authority applies the
    // SAME per-entry contract the fold and the lane decoder do -- bound, UTF-8,
    // token charset, zero padding, distinct -- so a trailer one side refuses
    // cannot be accepted by another. An authority that only skipped the block
    // would report a stream as decoded that the fold then rejects.
    if identity_len > 0 {
        let mut distinct = BTreeSet::new();
        let mut cursor = trailer_offset.saturating_add(header_len);
        for slot in 0..(identity_len / COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE as usize) {
            match decode_compact_strategic_resource_identity(payload, cursor, slot) {
                Ok(token) => {
                    if !distinct.insert(token.clone()) {
                        summary.parse_errors.push(format!(
                            "offset {offset}: market trade MTRR identity {slot} repeats token {token}"
                        ));
                    }
                }
                Err(reason) => summary
                    .parse_errors
                    .push(format!("offset {offset}: market trade {reason}")),
            }
            cursor = cursor.saturating_add(COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE as usize);
        }
    }
    add_compact_domain_data_unit(summary, 43, unit_name, row_count);
    expected_len
}

/// Decode one `MTRR` v3 identity entry, fail-closed on every field.
///
/// Deliberately a charset rule and not a list of names: which resources exist
/// is the mod's to decide, and a decoder that knows the answer in advance is
/// the defect the identity block removes.
fn decode_compact_strategic_resource_identity(
    payload: &[u8],
    cursor: usize,
    slot: usize,
) -> Result<String, String> {
    let max_len = compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN;
    let token_len = read_u16_le(payload, cursor) as usize;
    if token_len == 0 || token_len > max_len {
        return Err(format!(
            "MTRR identity {slot} token length {token_len} is outside 1..={max_len}"
        ));
    }
    let body_start = cursor.saturating_add(2);
    let body = payload
        .get(body_start..body_start.saturating_add(max_len))
        .ok_or_else(|| format!("MTRR identity {slot} is truncated"))?;
    let (token_bytes, padding) = body.split_at(token_len);
    if padding.iter().any(|byte| *byte != 0) {
        return Err(format!("MTRR identity {slot} has non-zero padding"));
    }
    let token = std::str::from_utf8(token_bytes)
        .map_err(|_| format!("MTRR identity {slot} is not UTF-8"))?;
    if !token.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-' || byte == b'.' || byte == b':'
    }) {
        return Err(format!("MTRR identity {slot} is not a resource token"));
    }
    Ok(token.to_string())
}

fn decode_compact_market_trade_classic_deal_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) -> usize {
    if payload.get(trailer_offset..trailer_offset.saturating_add(4))
        != Some(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC.as_slice())
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade trailing bytes are not MTRD classic-deal trailer"
        ));
        return trailer_offset;
    }
    if let Err(err) = require_compact_remaining(
        payload,
        trailer_offset,
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_HEADER_LEN,
        "market trade MTRD trailer header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return trailer_offset;
    }

    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let reserved0 = read_u16_le(payload, trailer_offset + 10);
    let row_count = read_u32_le(payload, trailer_offset + 12) as usize;
    let country_count = read_u32_le(payload, trailer_offset + 16) as usize;
    let scale_raw = read_u32_le(payload, trailer_offset + 20);
    let witness_count = read_u32_le(payload, trailer_offset + 24) as usize;
    // v3's and v4's incomplete-owner count. v1/v2 hold this word at zero, so a
    // v3 or v4 payload that lost its version word still fails here rather than
    // reading its completeness block as deal rows.
    let per_owner = version == COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION
        || version == COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_WIDE_IDENTITY_VERSION;
    let incomplete_count = read_u32_le(payload, trailer_offset + 28) as usize;
    if ![
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_LEGACY_VERSION,
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_VERSION,
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION,
        COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_WIDE_IDENTITY_VERSION,
    ]
    .contains(&version)
        || header_len != COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_HEADER_LEN
        || row_len != COMPACT_MARKET_TRADE_CLASSIC_DEAL_ROW_LEN
        || reserved0 != 0
        || (!per_owner && incomplete_count != 0)
        || (per_owner && incomplete_count > witness_count)
        || scale_raw != COMPACT_FIXED_SCALE_RAW
        || witness_count != country_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRD header shape is invalid"
        ));
        return trailer_offset;
    }

    let expected_len = trailer_offset
        .saturating_add(header_len)
        .saturating_add(witness_count.saturating_mul(4))
        .saturating_add(incomplete_count.saturating_mul(4))
        .saturating_add(row_count.saturating_mul(row_len));
    if payload.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTRD len {} < {expected_len}",
            payload.len()
        ));
        return trailer_offset;
    }
    // v3's country blocks are READ, not merely counted.
    //
    // This decoder is structural: it advances a cursor and counts units, and it
    // deliberately does not restate the fold's ROW-level semantics. But the
    // incomplete block is a statement about which owners are exact, and it is
    // only meaningful against a witness list that is itself well formed. A
    // decoder that admits `[99]` where 99 is not a witness, `[67, 34]` out of
    // order, or witnesses `[34, 34]` -- where the duplicate collapses in the set
    // and lets `[34]` pass the subset check -- would report the payload
    // decodable while `world_fold` rejects the whole D43 record. That is three
    // decoders disagreeing about one grammar.
    //
    // Gated on v3 alone, not on a non-empty incomplete block: a v3 trailer with
    // no incomplete owners still owes a well-formed witness list, and skipping
    // the check when the block is empty was the hole review found. NO new
    // refusal is introduced for a v1/v2 payload, whose witness content this
    // decoder has never inspected and whose archived bytes must keep decoding
    // exactly as they did.
    if per_owner {
        let mut witnesses = std::collections::BTreeSet::new();
        let mut cursor = trailer_offset.saturating_add(header_len);
        for witness_index in 0..witness_count {
            let tag_db_id = read_u32_le(payload, cursor);
            cursor = cursor.saturating_add(4);
            if tag_db_id == 0
                || tag_db_id > COMPACT_MARKET_TRADE_MAX_TAG_DB_ID
                || !witnesses.insert(tag_db_id)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTRD witness {witness_index} is invalid or duplicated"
                ));
                return trailer_offset;
            }
        }
        let mut previous: Option<u32> = None;
        for incomplete_index in 0..incomplete_count {
            let tag_db_id = read_u32_le(payload, cursor);
            cursor = cursor.saturating_add(4);
            if !witnesses.contains(&tag_db_id)
                || previous.is_some_and(|previous| previous >= tag_db_id)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTRD incomplete owner {incomplete_index} is not an ascending witness"
                ));
                return trailer_offset;
            }
            previous = Some(tag_db_id);
        }
    }
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_classic_deal_country_witnesses",
        witness_count,
    );
    // Counted separately from the witnesses: this is the number of owners the
    // hour published WITHOUT an exactness claim, and it is zero for every
    // archived v1/v2 payload.
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_classic_deal_incomplete_owners",
        incomplete_count,
    );
    add_compact_domain_data_unit(summary, 43, "market_trade_classic_deal_rows", row_count);
    expected_len
}

fn decode_compact_market_trade_stockpile_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    summary: &mut DomainDataDecodeSummary,
) -> usize {
    if let Err(err) = require_compact_remaining(
        payload,
        trailer_offset,
        COMPACT_MARKET_TRADE_STOCKPILE_HEADER_LEN,
        "market trade MTS1 trailer header",
    ) {
        summary.parse_errors.push(format!("offset {offset}: {err}"));
        return trailer_offset;
    }
    let version = read_u16_le(payload, trailer_offset + 4);
    let header_len = read_u16_le(payload, trailer_offset + 6) as usize;
    let country_row_len = read_u16_le(payload, trailer_offset + 8) as usize;
    let item_row_len = read_u16_le(payload, trailer_offset + 10) as usize;
    let segment_len = read_u32_le(payload, trailer_offset + 12) as usize;
    let country_count = read_u32_le(payload, trailer_offset + 16) as usize;
    let item_count = read_u32_le(payload, trailer_offset + 20) as usize;
    let expected_country_count = read_u32_le(payload, trailer_offset + 24) as usize;
    let scale_raw = read_u32_le(payload, trailer_offset + 28);
    let flags = read_u32_le(payload, trailer_offset + 32);
    let reserved = read_u32_le(payload, trailer_offset + 36);
    let expected_segment_len = COMPACT_MARKET_TRADE_STOCKPILE_HEADER_LEN
        .saturating_add(
            country_count.saturating_mul(COMPACT_MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN),
        )
        .saturating_add(item_count.saturating_mul(COMPACT_MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN));
    if version != COMPACT_MARKET_TRADE_STOCKPILE_TRAILER_VERSION
        || header_len != COMPACT_MARKET_TRADE_STOCKPILE_HEADER_LEN
        || country_row_len != COMPACT_MARKET_TRADE_STOCKPILE_COUNTRY_ROW_LEN
        || item_row_len != COMPACT_MARKET_TRADE_STOCKPILE_ITEM_ROW_LEN
        || segment_len != expected_segment_len
        || country_count == 0
        || country_count != expected_country_count
        || scale_raw != COMPACT_FIXED_SCALE_RAW
        || flags != COMPACT_MARKET_TRADE_STOCKPILE_GLOBAL_FLAGS
        || reserved != 0
        || trailer_offset.saturating_add(segment_len) > payload.len()
    {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTS1 header/count/length is invalid"
        ));
        return trailer_offset;
    }

    let mut seen_tags = BTreeSet::new();
    let mut expected_first = 0usize;
    for row_index in 0..country_count {
        let cursor = trailer_offset + header_len + row_index * country_row_len;
        let tag_db_id = read_u32_le(payload, cursor);
        let item_first = read_u32_le(payload, cursor + 4) as usize;
        let row_item_count = read_u32_le(payload, cursor + 8) as usize;
        let row_flags = read_u32_le(payload, cursor + 12);
        let row_reserved = read_u32_le(payload, cursor + 16);
        let item_end = item_first.saturating_add(row_item_count);
        if tag_db_id == 0
            || tag_db_id > COMPACT_MARKET_TRADE_MAX_TAG_DB_ID
            || !seen_tags.insert(tag_db_id)
            || item_first != expected_first
            || item_end > item_count
            || row_flags != COMPACT_MARKET_TRADE_STOCKPILE_COUNTRY_COMPLETE
            || row_reserved != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTS1 country row {row_index} has invalid tag/range/completeness"
            ));
            return trailer_offset;
        }
        expected_first = item_end;
    }
    if expected_first != item_count {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTS1 country ranges cover {expected_first} item(s), expected {item_count}"
        ));
        return trailer_offset;
    }

    let item_base = trailer_offset + header_len + country_count * country_row_len;
    let mut seen_items = BTreeSet::new();
    for country_index in 0..country_count {
        let country_cursor = trailer_offset + header_len + country_index * country_row_len;
        let tag_db_id = read_u32_le(payload, country_cursor);
        let item_first = read_u32_le(payload, country_cursor + 4) as usize;
        let row_item_count = read_u32_le(payload, country_cursor + 8) as usize;
        for ordinal in 0..row_item_count {
            let item_index = item_first + ordinal;
            let cursor = item_base + item_index * item_row_len;
            let variant_id = read_u32_le(payload, cursor);
            let equipment_type = read_u32_le(payload, cursor + 4);
            let row_ordinal = read_u32_le(payload, cursor + 8) as usize;
            let item_reserved = read_u32_le(payload, cursor + 12);
            let amount_raw = read_i64_le(payload, cursor + 16);
            if variant_id == 0
                || variant_id > COMPACT_MARKET_TRADE_MAX_VARIANT_ID
                || equipment_type != COMPACT_MARKET_TRADE_STOCKPILE_EQUIPMENT_TYPE
                || row_ordinal != ordinal
                || item_reserved != 0
                || !(0..=COMPACT_MARKET_TRADE_MAX_AMOUNT_RAW).contains(&amount_raw)
                || !seen_items.insert((tag_db_id, variant_id, equipment_type))
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: market trade MTS1 item {item_index} has invalid identity/order/amount"
                ));
                return trailer_offset;
            }
        }
    }
    add_compact_domain_data_unit(
        summary,
        43,
        "market_trade_stockpile_country_rows",
        country_count,
    );
    add_compact_domain_data_unit(summary, 43, "market_trade_stockpile_item_rows", item_count);
    trailer_offset + segment_len
}

// --- W1 domain 63 country_core_scalars_live -------------------------------
// 20-byte header (elapsed_wall_ms u64, row_count u32, changed_rows u32,
// status_flags u32), then row_count fixed 48-byte rows. Each row: tag_db_id u32,
// flags u32, manpower_pool u32, convoys_destroyed u32, divisions_lost u32, then
// 7 x i32 experience channels. Mirrors world_fold::fold_country_core_scalars.
fn decode_compact_country_core_scalars_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_COUNTRY_CORE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: country_core_scalars payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_COUNTRY_CORE_HEADER_LEN
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    add_decoder_status(summary, status_flags);
    let expected = COMPACT_COUNTRY_CORE_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_COUNTRY_CORE_ROW_LEN));
    if payload.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: country_core_scalars len {} != {expected}",
            payload.len()
        ));
        return;
    }
    if changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: country_core_scalars changed_rows {changed_rows} != full-replace row_count {row_count}"
        ));
    }
    for i in 0..row_count {
        let base = COMPACT_COUNTRY_CORE_HEADER_LEN + i * COMPACT_COUNTRY_CORE_ROW_LEN;
        let tag_db_id = read_u32_le(payload, base);
        let flags = read_u32_le(payload, base + 4);
        let manpower_pool = read_u32_le(payload, base + 8);
        let convoys_destroyed = read_u32_le(payload, base + 12);
        let divisions_lost = read_u32_le(payload, base + 16);
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country_core_scalars row {i} has zero tag_db_id"
            ));
            continue;
        }
        if flags & !COMPACT_COUNTRY_CORE_KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country_core_scalars row {i} has unknown flags {flags:#x}"
            ));
        }
        let has_manpower = flags & COMPACT_COUNTRY_CORE_FLAG_MANPOWER_POOL != 0;
        let has_convoys = flags & COMPACT_COUNTRY_CORE_FLAG_CONVOYS_DESTROYED != 0;
        let has_divisions = flags & COMPACT_COUNTRY_CORE_FLAG_DIVISIONS_LOST != 0;
        let has_experience = flags & COMPACT_COUNTRY_CORE_FLAG_EXPERIENCE != 0;
        validate_compact_optional_u32(
            summary,
            offset,
            i,
            "country_core_scalars",
            "manpower_pool",
            manpower_pool,
            has_manpower,
            COMPACT_COUNTRY_CORE_MAX_MANPOWER_POOL,
        );
        validate_compact_optional_u32(
            summary,
            offset,
            i,
            "country_core_scalars",
            "convoys_destroyed",
            convoys_destroyed,
            has_convoys,
            COMPACT_COUNTRY_CORE_MAX_CONVOYS_DESTROYED,
        );
        validate_compact_optional_u32(
            summary,
            offset,
            i,
            "country_core_scalars",
            "divisions_lost",
            divisions_lost,
            has_divisions,
            COMPACT_COUNTRY_CORE_MAX_DIVISIONS_LOST,
        );
        // 7 experience channels (i32) at +20..+48; when the experience flag is
        // clear every channel must be zero-packed.
        if !has_experience {
            for k in 0..7 {
                if read_i32_le(payload, base + 20 + k * 4) != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: country_core_scalars row {i} experience present without flag"
                    ));
                    break;
                }
            }
        }
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_COUNTRY_CORE_SCALARS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_CORE_SCALARS_LIVE,
        "country_core_scalars_rows",
        row_count,
    );
}

// --- W3 domain 64 delayed_events_live -------------------------------------
// 20-byte header (elapsed_wall_ms u64, row_count u32, changed_rows u32,
// status_flags u32), then row_count fixed 20-byte rows: owner_tag_db_id u32,
// event_id u32, flags u32, originator u32, due_remaining_hours u32.
fn decode_compact_delayed_events_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_DELAYED_EVENTS_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_events payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_DELAYED_EVENTS_HEADER_LEN
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    add_decoder_status(summary, status_flags);
    let Some(rows_bytes) = row_count.checked_mul(COMPACT_DELAYED_EVENT_ROW_LEN) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: delayed_events row length overflow"));
        return;
    };
    let Some(rows_end) = COMPACT_DELAYED_EVENTS_HEADER_LEN.checked_add(rows_bytes) else {
        summary
            .parse_errors
            .push(format!("offset {offset}: delayed_events row length overflow"));
        return;
    };
    if payload.len() < rows_end {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_events len {} is shorter than {rows_end}",
            payload.len()
        ));
        return;
    }
    let lifecycle_rows = if payload.len() == rows_end {
        0
    } else {
        decode_compact_delayed_event_lifecycle(
            offset.saturating_add(rows_end),
            &payload[rows_end..],
            summary,
        )
        .unwrap_or(0)
    };
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_events changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    for i in 0..row_count {
        let base = COMPACT_DELAYED_EVENTS_HEADER_LEN + i * COMPACT_DELAYED_EVENT_ROW_LEN;
        let owner_tag_db_id = read_u32_le(payload, base);
        let event_id = read_u32_le(payload, base + 4);
        let flags = read_u32_le(payload, base + 8);
        let originator = read_u32_le(payload, base + 12);
        let due_remaining_hours = read_u32_le(payload, base + 16);
        if owner_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} has zero owner tag_db_id"
            ));
        }
        if event_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} has zero event_id"
            ));
        }
        if flags & !COMPACT_DELAYED_EVENT_KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} has unknown flags {flags:#x}"
            ));
        }
        let has_due = flags & COMPACT_DELAYED_EVENT_FLAG_DUE_REMAINING_HOURS != 0;
        let has_originator = flags & COMPACT_DELAYED_EVENT_FLAG_ORIGINATOR != 0;
        if !has_due && due_remaining_hours != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} due_remaining_hours without flag"
            ));
        }
        if has_due && due_remaining_hours > COMPACT_DELAYED_EVENT_MAX_DUE_REMAINING_HOURS {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} due_remaining_hours {due_remaining_hours} out of range"
            ));
        }
        if !has_originator && originator != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_events row {i} originator without flag"
            ));
        }
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_DELAYED_EVENTS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_DELAYED_EVENTS_LIVE,
        "delayed_events_rows",
        row_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_DELAYED_EVENTS_LIVE,
        "delayed_event_lifecycle_rows",
        lifecycle_rows,
    );
}

fn decode_compact_delayed_event_lifecycle(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) -> Option<usize> {
    if payload.len() < COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_event_lifecycle payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN
        ));
        return None;
    }
    if payload.get(0..4) != Some(COMPACT_DELAYED_EVENT_LIFECYCLE_MAGIC.as_slice())
        || read_u16_le(payload, 4) != COMPACT_DELAYED_EVENT_LIFECYCLE_VERSION
        || read_u16_le(payload, 6) as usize != COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN
        || read_u16_le(payload, 8) as usize != COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_LEN
        || read_u16_le(payload, 10) != 0
        || read_u32_le(payload, 44) != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_event_lifecycle header rejected"
        ));
        return None;
    }
    let flags = read_u32_le(payload, 12);
    if flags & !COMPACT_DELAYED_EVENT_LIFECYCLE_FLAG_COMPLETE != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_event_lifecycle unknown header flags {flags:#x}"
        ));
        return None;
    }
    let count = read_u32_le(payload, 16) as usize;
    let hook_epoch = read_u64_le(payload, 20);
    let first_sequence = read_u64_le(payload, 28);
    let last_sequence = read_u64_le(payload, 36);
    let expected = count
        .checked_mul(COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_LEN)
        .and_then(|rows| COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN.checked_add(rows));
    if expected != Some(payload.len())
        || hook_epoch == 0
        || (count == 0 && (first_sequence != 0 || last_sequence != 0))
        || (count != 0
            && last_sequence
                .checked_sub(first_sequence)
                .and_then(|delta| delta.checked_add(1))
                != Some(count as u64))
    {
        summary.parse_errors.push(format!(
            "offset {offset}: delayed_event_lifecycle count/sequence header rejected"
        ));
        return None;
    }
    for index in 0..count {
        let base =
            COMPACT_DELAYED_EVENT_LIFECYCLE_HEADER_LEN + index * COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_LEN;
        let sequence = read_u64_le(payload, base);
        let game_hour_raw = read_i32_le(payload, base + 8);
        let reason = read_u32_le(payload, base + 12);
        let owner_tag_db_id = read_u32_le(payload, base + 16);
        let event_id = read_u32_le(payload, base + 20);
        let originator_tag_db_id = read_u32_le(payload, base + 24);
        let row_flags = read_u32_le(payload, base + 28);
        let due_before = read_u32_le(payload, base + 40);
        let due_after = read_u32_le(payload, base + 44);
        let expected_sequence = first_sequence.checked_add(index as u64);
        if expected_sequence != Some(sequence)
            || game_hour_raw == i32::MIN
            || owner_tag_db_id == 0
            || originator_tag_db_id == 0
            || !(100_000..=100_099_999).contains(&event_id)
            || row_flags & !COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_FLAG_DUE_AFTER != 0
            || due_before > COMPACT_DELAYED_EVENT_MAX_DUE_REMAINING_HOURS
        {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_event_lifecycle row {index} rejected"
            ));
            return None;
        }
        let has_due_after =
            row_flags & COMPACT_DELAYED_EVENT_LIFECYCLE_ROW_FLAG_DUE_AFTER != 0;
        let transform_valid = match (reason, has_due_after) {
            (1, true) => {
                due_before.checked_sub(1) == Some(due_after)
                    && due_after <= COMPACT_DELAYED_EVENT_MAX_DUE_REMAINING_HOURS
            }
            (2..=4, false) => due_after == 0,
            _ => false,
        };
        if !transform_valid {
            summary.parse_errors.push(format!(
                "offset {offset}: delayed_event_lifecycle row {index} reason/due transform rejected"
            ));
            return None;
        }
    }
    Some(count)
}

// --- W3 domain 65 special_project_programs_live ---------------------------
// 24-byte header (elapsed_wall_ms u64, row_count u32, changed_rows u32,
// status_flags u32, dropped_ambiguous u32), then row_count fixed 16-byte rows:
// owner_tag_db_id u32, program_id u32, province u32, unread_prototype_rewards u32.
fn decode_compact_special_project_programs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    if payload.len() < COMPACT_SPECIAL_PROJECT_PROGRAM_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: special_project_programs payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_SPECIAL_PROJECT_PROGRAM_HEADER_LEN
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let dropped_ambiguous = read_u32_le(payload, 20);
    add_decoder_status(summary, status_flags);
    let expected = COMPACT_SPECIAL_PROJECT_PROGRAM_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_SPECIAL_PROJECT_PROGRAM_ROW_LEN));
    if payload.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: special_project_programs len {} != {expected}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: special_project_programs changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    let mut seen: BTreeSet<(u32, u32)> = BTreeSet::new();
    for i in 0..row_count {
        let base = COMPACT_SPECIAL_PROJECT_PROGRAM_HEADER_LEN
            + i * COMPACT_SPECIAL_PROJECT_PROGRAM_ROW_LEN;
        let owner_tag_db_id = read_u32_le(payload, base);
        let program_id = read_u32_le(payload, base + 4);
        if owner_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: special_project_programs row {i} has zero owner tag_db_id"
            ));
        }
        if program_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: special_project_programs row {i} has zero program_id"
            ));
        }
        if !seen.insert((owner_tag_db_id, program_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: special_project_programs row {i} duplicates (owner {owner_tag_db_id}, program_id {program_id})"
            ));
        }
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE,
        "special_project_programs_rows",
        row_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE,
        "special_project_programs_dropped_ambiguous",
        dropped_ambiguous as usize,
    );
}

// --- W4 domain 66 occupation_state_extras_live ----------------------------
// 20-byte header (elapsed_wall_ms u64, row_count u32, changed_rows u32,
// status_flags u32), then row_count fixed 24-byte rows: controller_tag u32,
// occupied_tag u32, state_id u32, flags u32, occupation_law_token_id u32,
// garrison_manpower_source_tag_db_id u32.
fn decode_compact_occupation_state_extras_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_OCCUPATION_STATE_EXTRAS_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation_state_extras payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_OCCUPATION_STATE_EXTRAS_HEADER_LEN
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    add_decoder_status(summary, status_flags);
    let expected = COMPACT_OCCUPATION_STATE_EXTRAS_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_OCCUPATION_STATE_EXTRAS_ROW_LEN));
    if payload.len() != expected {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation_state_extras len {} != {expected}",
            payload.len()
        ));
        return;
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: occupation_state_extras changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    let mut seen: BTreeSet<(u32, u32, u32)> = BTreeSet::new();
    for i in 0..row_count {
        let base = COMPACT_OCCUPATION_STATE_EXTRAS_HEADER_LEN
            + i * COMPACT_OCCUPATION_STATE_EXTRAS_ROW_LEN;
        let controller_tag_db_id = read_u32_le(payload, base);
        let occupied_tag_db_id = read_u32_le(payload, base + 4);
        let state_id = read_u32_le(payload, base + 8);
        let flags = read_u32_le(payload, base + 12);
        let occupation_law_token_id = read_u32_le(payload, base + 16);
        let garrison_manpower_source_tag_db_id = read_u32_le(payload, base + 20);
        if controller_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} has zero controller tag"
            ));
        }
        if occupied_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} has zero occupied tag"
            ));
        }
        if state_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} has zero state_id"
            ));
        }
        if flags & !COMPACT_OCCUPATION_STATE_EXTRAS_KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} has unknown flags {flags:#x}"
            ));
        }
        let has_law = flags & COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_LAW != 0;
        let has_source = flags & COMPACT_OCCUPATION_STATE_EXTRAS_FLAG_GARRISON_SOURCE != 0;
        if !has_law && occupation_law_token_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} law token without flag"
            ));
        }
        if has_law && occupation_law_token_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} law flag with zero token"
            ));
        }
        if !has_source && garrison_manpower_source_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} garrison source without flag"
            ));
        }
        if has_source && garrison_manpower_source_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} garrison flag with zero tag"
            ));
        }
        if !has_law && !has_source {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} carries no field"
            ));
        }
        if !seen.insert((controller_tag_db_id, occupied_tag_db_id, state_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: occupation_state_extras row {i} duplicates (controller {controller_tag_db_id}, occupied {occupied_tag_db_id}, state {state_id})"
            ));
        }
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE,
        "occupation_state_extras_rows",
        row_count,
    );
}

// --- W5 domain 67 country_dynamic_modifiers_live --------------------------
// 20-byte header (elapsed_wall_ms u64, country_count u32, total_id_count u32,
// status_flags u32), then country_count fixed 12-byte rows (tag_db_id u32,
// id_count u32, id_blob_offset u32 in id units), then a single concatenated id
// blob where each id is a u16 length prefix + utf8 bytes. Windows must be
// contiguous and cover the blob exactly.
fn decode_compact_country_dynamic_modifiers_data_payload(
    offset: usize,
    payload: &[u8],
    record_flags: u32,
    summary: &mut DomainDataDecodeSummary,
) {
    if record_flags & COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2 != 0 {
        if !compact_contract::dynamic_modifiers::is_v2_payload(payload) {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers schema-v2 record omitted DMI2 magic"
            ));
            return;
        }
        match compact_contract::dynamic_modifiers::decode_payload(payload) {
            Ok(frame) => {
                add_decoder_status(summary, frame.status_flags);
                let countries = frame
                    .instances
                    .iter()
                    .map(|instance| instance.owner_tag_db_id)
                    .collect::<BTreeSet<_>>()
                    .len();
                let values = frame
                    .instances
                    .iter()
                    .map(|instance| instance.values_raw.len())
                    .sum::<usize>();
                if let Some(domain) = summary
                    .per_domain
                    .get_mut(&COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE)
                {
                    domain.data_rows_total = domain
                        .data_rows_total
                        .saturating_add(frame.instances.len() as u64);
                    domain.status_flags_or |= frame.status_flags;
                }
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE,
                    "country_dynamic_modifier_instances",
                    frame.instances.len(),
                );
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE,
                    "country_dynamic_modifier_values",
                    values,
                );
                add_compact_domain_data_unit(
                    summary,
                    COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE,
                    "country_dynamic_modifiers_countries",
                    countries,
                );
            }
            Err(error) => summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers DMI2 invalid: {error:?}"
            )),
        }
        return;
    }
    if payload.len() < COMPACT_COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: country_dynamic_modifiers payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN
        ));
        return;
    }
    let country_count = read_u32_le(payload, 8) as usize;
    let total_id_count = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    add_decoder_status(summary, status_flags);
    let table_len = match country_count.checked_mul(COMPACT_COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN) {
        Some(rows) => COMPACT_COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN.saturating_add(rows),
        None => {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers row table overflow"
            ));
            return;
        }
    };
    if payload.len() < table_len {
        summary.parse_errors.push(format!(
            "offset {offset}: country_dynamic_modifiers payload is {} byte(s), expected at least {table_len}",
            payload.len()
        ));
        return;
    }
    let blob_budget = payload.len().saturating_sub(table_len);
    if total_id_count > blob_budget / 3 {
        summary.parse_errors.push(format!(
            "offset {offset}: country_dynamic_modifiers total_id_count {total_id_count} exceeds blob budget {}",
            blob_budget / 3
        ));
        return;
    }
    // Parse the id blob (u16 length prefix + bytes), validating bounds/utf8.
    let mut id_count_parsed = 0usize;
    let mut cursor = table_len;
    let mut torn = false;
    for _ in 0..total_id_count {
        if cursor + 2 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers id blob truncated (length prefix)"
            ));
            torn = true;
            break;
        }
        let len = read_u16_le(payload, cursor) as usize;
        cursor += 2;
        if len == 0 || len > COMPACT_COUNTRY_DYNAMIC_MODIFIER_MAX_ID_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers id length {len} out of range"
            ));
            torn = true;
            break;
        }
        if cursor + len > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers id blob truncated (bytes)"
            ));
            torn = true;
            break;
        }
        if std::str::from_utf8(&payload[cursor..cursor + len]).is_err() {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers id not utf8"
            ));
            torn = true;
            break;
        }
        cursor += len;
        id_count_parsed += 1;
    }
    if !torn && cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: country_dynamic_modifiers trailing bytes: consumed {cursor} of {}",
            payload.len()
        ));
    }
    // Validate the contiguous, exact-cover row windows against the parsed ids.
    let mut expected_offset = 0usize;
    let mut seen: BTreeSet<u32> = BTreeSet::new();
    for i in 0..country_count {
        let base = COMPACT_COUNTRY_DYNAMIC_MODIFIERS_HEADER_LEN
            + i * COMPACT_COUNTRY_DYNAMIC_MODIFIERS_ROW_LEN;
        let tag_db_id = read_u32_le(payload, base);
        let id_count = read_u32_le(payload, base + 4) as usize;
        let id_blob_offset = read_u32_le(payload, base + 8) as usize;
        if tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers row {i} has zero tag_db_id"
            ));
        }
        if id_count == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers row {i} has zero id_count"
            ));
        }
        if id_blob_offset != expected_offset {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers row {i} id_blob_offset {id_blob_offset} != expected {expected_offset} (non-contiguous)"
            ));
        }
        let end = id_blob_offset.saturating_add(id_count);
        if end > id_count_parsed {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers row {i} id window [{id_blob_offset},{end}) exceeds {id_count_parsed} ids"
            ));
        }
        if !seen.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: country_dynamic_modifiers duplicate tag_db_id {tag_db_id}"
            ));
        }
        expected_offset = end;
    }
    if !torn && expected_offset != id_count_parsed {
        summary.parse_errors.push(format!(
            "offset {offset}: country_dynamic_modifiers windows cover {expected_offset} of {id_count_parsed} ids (orphans)"
        ));
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(country_count as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE,
        "country_dynamic_modifiers_countries",
        country_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE,
        "country_dynamic_modifiers_ids",
        id_count_parsed,
    );
}

// --- W6 domain 68 session_header_live -------------------------------------
// 16-byte header (elapsed_wall_ms u64, presence_flags u32, status_flags u32),
// a fixed 20-byte scalar block (player_tag u32, dlcs u32, start_date u32,
// save_version u32, session u32), then three length-prefixed utf8 strings
// (ideology, cosmetic_tag, version), each a u16 length + bytes.
fn decode_compact_session_header_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    summary.session_header_identity.session_header_records_seen = summary
        .session_header_identity
        .session_header_records_seen
        .saturating_add(1);
    let errors_before = summary.parse_errors.len();
    if payload.len() < COMPACT_SESSION_HEADER_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_SESSION_HEADER_HEADER_LEN
        ));
        return;
    }
    let presence = read_u32_le(payload, 8);
    let status_flags = read_u32_le(payload, 12);
    add_decoder_status(summary, status_flags);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_SESSION_HEADER_LIVE)
    {
        domain.status_flags_or |= status_flags;
    }
    if presence & !COMPACT_SESSION_HEADER_KNOWN_FLAGS != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header unknown presence flags {presence:#x}"
        ));
    }
    let scalars_end = COMPACT_SESSION_HEADER_HEADER_LEN + COMPACT_SESSION_HEADER_SCALAR_BLOCK_LEN;
    if payload.len() < scalars_end {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header payload is {} byte(s), expected at least {scalars_end}",
            payload.len()
        ));
        return;
    }
    let scalars_base = COMPACT_SESSION_HEADER_HEADER_LEN;
    let player_tag_db_id = read_u32_le(payload, scalars_base);
    let dlcs = read_u32_le(payload, scalars_base + 4);
    let start_date_raw = read_u32_le(payload, scalars_base + 8);
    let save_version = read_u32_le(payload, scalars_base + 12);
    let session = read_u32_le(payload, scalars_base + 16);
    let has_player = presence & COMPACT_SESSION_HEADER_FLAG_PLAYER != 0;
    let has_dlcs = presence & COMPACT_SESSION_HEADER_FLAG_DLCS != 0;
    let has_start_date = presence & COMPACT_SESSION_HEADER_FLAG_START_DATE != 0;
    let has_save_version = presence & COMPACT_SESSION_HEADER_FLAG_SAVE_VERSION != 0;
    let has_session = presence & COMPACT_SESSION_HEADER_FLAG_SESSION != 0;
    let has_ideology = presence & COMPACT_SESSION_HEADER_FLAG_IDEOLOGY != 0;
    let has_cosmetic = presence & COMPACT_SESSION_HEADER_FLAG_COSMETIC_TAG != 0;
    let has_version = presence & COMPACT_SESSION_HEADER_FLAG_VERSION != 0;
    if !has_player && player_tag_db_id != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header player_tag_db_id without flag"
        ));
    }
    if has_player
        && (player_tag_db_id == 0 || player_tag_db_id > COMPACT_SESSION_HEADER_MAX_PLAYER_TAG_DB_ID)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header player_tag_db_id {player_tag_db_id} out of range"
        ));
    }
    if !has_dlcs && dlcs != 0 {
        summary
            .parse_errors
            .push(format!("offset {offset}: session_header dlcs without flag"));
    }
    if !has_start_date && start_date_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header start_date without flag"
        ));
    }
    if !has_save_version && save_version != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header save_version without flag"
        ));
    }
    if !has_session && session != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header session without flag"
        ));
    }
    // Three length-prefixed utf8 tokens must consume the payload exactly.
    let mut cursor = scalars_end;
    for (flag_present, name) in [
        (has_ideology, "ideology"),
        (has_cosmetic, "cosmetic_tag"),
        (has_version, "version"),
    ] {
        if cursor + 2 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: session_header {name} length prefix truncated"
            ));
            return;
        }
        let len = read_u16_le(payload, cursor) as usize;
        cursor += 2;
        if len > COMPACT_SESSION_HEADER_MAX_STRING_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: session_header {name} length {len} out of range"
            ));
            return;
        }
        if cursor + len > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: session_header {name} bytes truncated"
            ));
            return;
        }
        let has_bytes = if len == 0 {
            false
        } else if std::str::from_utf8(&payload[cursor..cursor + len]).is_err() {
            summary
                .parse_errors
                .push(format!("offset {offset}: session_header {name} not utf8"));
            true
        } else {
            true
        };
        cursor += len;
        if !flag_present && has_bytes {
            summary.parse_errors.push(format!(
                "offset {offset}: session_header {name} bytes without flag"
            ));
        }
        if flag_present && !has_bytes {
            summary.parse_errors.push(format!(
                "offset {offset}: session_header {name} flag set but empty"
            ));
        }
    }
    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: session_header trailing bytes: consumed {cursor} of {}",
            payload.len()
        ));
    }
    // Commit identity witnesses only after the complete payload grammar has
    // validated and only for an effective-success row. Failure/partial status
    // mirrors the canonical fold's rejection: it remains badged above but can
    // never leave stale or partial positive identity evidence behind.
    if summary.parse_errors.len() != errors_before
        || compact_contract::compact_status_is_failure(status_flags)
    {
        return;
    }
    summary.session_header_identity.session_header_rows = summary
        .session_header_identity
        .session_header_rows
        .saturating_add(1);
    if has_start_date {
        summary.session_header_identity.witnesses.start_date_raw = Some(start_date_raw);
    }
    if has_session {
        summary.session_header_identity.witnesses.session = Some(session);
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SESSION_HEADER_LIVE,
        "session_header_present_fields",
        (presence & COMPACT_SESSION_HEADER_KNOWN_FLAGS).count_ones() as usize,
    );
}

// --- W6 domain 69 tech_sharing_groups_live --------------------------------
// 16-byte header (elapsed_wall_ms u64, group_count u32, status_flags u32), then
// group_count fixed 12-byte rows (id_token u32, member_count u32,
// member_blob_offset u32 in u32 units), then a flat u32 tag_db_id member blob.
// Windows must be contiguous and cover the blob exactly.
#[cfg(any())]
fn decode_compact_tech_sharing_groups_data_payload_legacy(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_TECH_SHARING_GROUPS_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_TECH_SHARING_GROUPS_HEADER_LEN
        ));
        return;
    }
    let group_count = read_u32_le(payload, 8) as usize;
    let status_flags = read_u32_le(payload, 12);
    add_decoder_status(summary, status_flags);
    if group_count > COMPACT_TECH_SHARING_GROUP_MAX_GROUPS {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups group_count {group_count} exceeds {COMPACT_TECH_SHARING_GROUP_MAX_GROUPS} fixed slots"
        ));
        return;
    }
    let table_len = match group_count.checked_mul(COMPACT_TECH_SHARING_GROUPS_ROW_LEN) {
        Some(rows) => COMPACT_TECH_SHARING_GROUPS_HEADER_LEN.saturating_add(rows),
        None => {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row table overflow"
            ));
            return;
        }
    };
    if payload.len() < table_len {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups payload is {} byte(s), expected at least {table_len}",
            payload.len()
        ));
        return;
    }
    let blob = &payload[table_len..];
    if blob.len() % 4 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups member blob {} bytes not u32-aligned",
            blob.len()
        ));
        return;
    }
    let member_total = blob.len() / 4;
    let mut expected_offset = 0usize;
    let mut total_members = 0usize;
    for i in 0..group_count {
        let base = COMPACT_TECH_SHARING_GROUPS_HEADER_LEN + i * COMPACT_TECH_SHARING_GROUPS_ROW_LEN;
        let id_token = read_u32_le(payload, base);
        let member_count = read_u32_le(payload, base + 4) as usize;
        let member_blob_offset = read_u32_le(payload, base + 8) as usize;
        if id_token == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row {i} has zero id_token"
            ));
        }
        if member_count == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row {i} has zero member_count"
            ));
        }
        if member_count > COMPACT_TECH_SHARING_GROUP_MAX_MEMBERS {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row {i} member_count {member_count} out of range"
            ));
        }
        if member_blob_offset != expected_offset {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row {i} member_blob_offset {member_blob_offset} != expected {expected_offset} (non-contiguous)"
            ));
        }
        let end = member_blob_offset.saturating_add(member_count);
        if end > member_total {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups row {i} member window [{member_blob_offset},{end}) exceeds {member_total} members"
            ));
        } else {
            for idx in member_blob_offset..end {
                let tag = read_u32_le(blob, idx * 4);
                if tag == 0 || tag > COMPACT_TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: tech_sharing_groups row {i} member tag_db_id {tag} out of range"
                    ));
                }
            }
        }
        total_members += member_count;
        expected_offset = end;
    }
    if expected_offset != member_total {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups windows cover {expected_offset} of {member_total} members (orphans)"
        ));
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(group_count as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_groups",
        group_count,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_members",
        total_members,
    );
}

fn decode_compact_tech_sharing_groups_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    // Preserve raw/global status observability in partial authority even when
    // the v3 discriminator or body is malformed. Rows/units are committed only
    // after the complete shared decoder and host policy both succeed.
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded =
        match compact_contract::tech_sharing_groups::decode_payload_with_definition_ids(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: tech_sharing_groups TSG3 payload invalid: {error}"
            ));
            return;
        }
    };
    let mut policy_errors = Vec::new();
    if decoded.rows.len() > COMPACT_TECH_SHARING_GROUP_MAX_GROUPS {
        policy_errors.push(format!(
            "group_count {} exceeds {COMPACT_TECH_SHARING_GROUP_MAX_GROUPS}",
            decoded.rows.len()
        ));
    }
    let mut pair_total = 0usize;
    for (row_index, row) in decoded.rows.iter().enumerate() {
        if row.members.len() > COMPACT_TECH_SHARING_GROUP_MAX_MEMBERS {
            policy_errors.push(format!(
                "row {row_index} member_count {} exceeds {COMPACT_TECH_SHARING_GROUP_MAX_MEMBERS}",
                row.members.len()
            ));
        }
        pair_total = pair_total.saturating_add(row.members.len());
        for member in &row.members {
            if member.member_tag_db_id > COMPACT_TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID {
                policy_errors.push(format!(
                    "row {row_index} member tag_db_id {} exceeds {COMPACT_TECH_SHARING_GROUP_MAX_MEMBER_TAG_DB_ID}",
                    member.member_tag_db_id
                ));
            }
        }
    }
    if !policy_errors.is_empty() {
        summary.parse_errors.push(format!(
            "offset {offset}: tech_sharing_groups TSG3 policy failure: {}",
            policy_errors.join(", ")
        ));
        return;
    }
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.rows.len() as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_groups",
        decoded.rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_members",
        pair_total,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_dynamic_bonus_rows",
        pair_total,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE,
        "tech_sharing_groups_definition_id_rows",
        if decoded.definition_ids_present {
            decoded.rows.len()
        } else {
            0
        },
    );
}

fn decode_compact_sunk_convoy_history_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    // SCH1 stores its payload status at +0x0C. Preserve it in partial
    // authority even when the remaining payload is malformed.
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match compact_contract::naval_history::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: sunk_convoy_history SCH1 payload invalid: {error}"
            ));
            return;
        }
    };
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.rows.len() as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE,
        "sunk_convoy_history_rows",
        decoded.rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE,
        "sunk_convoy_history_observed_rows",
        decoded.observed_rows as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE,
        "sunk_convoy_history_convoys",
        decoded
            .rows
            .iter()
            .map(|row| row.convoys as usize)
            .sum::<usize>(),
    );
}

fn decode_compact_raid_instances_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match compact_contract::raid_instances::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: raid_instances RID1 payload invalid: {error}"
            ));
            return;
        }
    };
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_RAID_INSTANCES_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.parents.len() as u64)
            .saturating_add(decoded.children.len() as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_RAID_INSTANCES_LIVE,
        "raid_parent_rows",
        decoded.parents.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_RAID_INSTANCES_LIVE,
        "raid_instance_rows",
        decoded.children.len(),
    );
}

fn decode_compact_country_preferred_tactic_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    let decoded = match preferred92::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: country_preferred_tactic PTC1 payload invalid: {error}"
            ));
            return;
        }
    };
    let selected = decoded
        .rows
        .iter()
        .filter(|row| row.tactic_database_id > 0)
        .count();
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.rows.len() as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE,
        "country_preferred_tactic_rows",
        decoded.rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE,
        "country_preferred_tactic_selected_rows",
        selected,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE,
        "country_preferred_tactic_none_rows",
        decoded.rows.len().saturating_sub(selected),
    );
}

fn decode_compact_country_preferred_tactic_weight_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 20 {
        add_decoder_status(summary, read_u32_le(payload, 16));
    }
    let decoded = match weight96::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: country_preferred_tactic_weight PTW1 payload invalid: {error}"
            ));
            return;
        }
    };
    let accumulator_entries = decoded
        .rows
        .iter()
        .filter(|row| row.accumulator_entry_present)
        .count();
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE)
    {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.rows.len() as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE,
        "country_preferred_tactic_weight_observed_country_count",
        decoded.observed_country_count as usize,
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE,
        "country_preferred_tactic_weight_selected_rows",
        decoded.rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE,
        "country_preferred_tactic_weight_override_rows",
        accumulator_entries,
    );
}

fn decode_compact_country_army_upgrade_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match army_upgrades93::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: country_army_upgrade_requests AUR1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let (countries, groups, requests, deliveries, produced) = decoded.snapshot.as_ref().map_or(
        (0usize, 0usize, 0usize, 0usize, 0usize),
        |snapshot| {
            let groups = snapshot
                .countries
                .iter()
                .map(|country| country.groups.len())
                .sum::<usize>();
            let requests = snapshot
                .countries
                .iter()
                .flat_map(|country| &country.groups)
                .map(|group| group.requests.len())
                .sum::<usize>();
            let deliveries = snapshot
                .countries
                .iter()
                .flat_map(|country| &country.groups)
                .map(|group| group.deliveries.len())
                .sum::<usize>();
            let produced = snapshot
                .countries
                .iter()
                .flat_map(|country| &country.groups)
                .flat_map(|group| &group.deliveries)
                .map(|delivery| delivery.produced.len())
                .sum::<usize>();
            (
                snapshot.countries.len(),
                groups,
                requests,
                deliveries,
                produced,
            )
        },
    );
    // d93 recorded its status word into the per-domain copy only, never into the
    // shared funnel. `DomainDataAuthority::status_flags` was therefore `None` for
    // this domain, which is the claim "this payload authority decodes no status
    // word at all" — false, and deliberately not collapsed with "the status was
    // clean" (see `stream_fold::domain_data_payload_status`). The consequence was
    // that d93's embedded status was never cross-checked against its DOMAIN_PERF
    // status, and now also that it would have escaped `reject_unassigned_status_bits`.
    add_decoder_status(summary, decoded.status_flags);
    if let Some(domain) = summary
        .per_domain
        .get_mut(&COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE)
    {
        domain.data_rows_total = domain.data_rows_total.saturating_add(countries as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    for (name, count) in [
        ("country_army_upgrade_countries", countries),
        ("country_army_upgrade_groups", groups),
        ("country_army_upgrade_requests", requests),
        ("country_army_upgrade_deliveries", deliveries),
        ("country_army_upgrade_produced_rows", produced),
    ] {
        add_compact_domain_data_unit(
            summary,
            COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE,
            name,
            count,
        );
    }
}

fn decode_compact_rail_replay_state_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match compact_contract::rail_replay::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: rail_replay_state RRL1 payload invalid: {error}"
            ));
            return;
        }
    };
    if decoded.payload_version != compact_contract::rail_replay::PAYLOAD_VERSION_V3
        || !decoded.hub_simulation_complete
    {
        summary.parse_errors.push(format!(
            "offset {offset}: rail_replay_state RRL1 production authority requires v3 raw supply-simulation rows"
        ));
        return;
    }
    let admitted_hub_simulation_rows = decoded.hub_simulation_rows.len();
    if let Some(domain) = summary.per_domain.get_mut(&COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.provinces.len() as u64)
            .saturating_add(admitted_hub_simulation_rows as u64);
        domain.status_flags_or |= decoded.status_flags;
    }
    add_compact_domain_data_unit(summary, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE, "rail_replay_provinces", decoded.provinces.len());
    add_compact_domain_data_unit(summary, COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE, "rail_replay_directed_neighbors", decoded.observed_neighbors as usize);
    add_compact_domain_data_unit(
        summary,
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE,
        "rail_replay_hub_simulation_rows",
        admitted_hub_simulation_rows,
    );
}

fn decode_compact_fired_event_popups_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match fired75::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: fired_event_popups FEV1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let admitted_rows = if decoded.fired_events_complete {
        decoded.rows.len()
    } else {
        0
    };
    if let Some(domain) = summary.per_domain.get_mut(&fired75::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(admitted_rows as u64);
    }
    // The d75 ledger is append-only for the life of the runtime -- every frame
    // clones the WHOLE ledger -- so these are the ledger's size, not this
    // frame's fires.
    add_compact_domain_cumulative_unit(
        summary,
        fired75::DOMAIN_ID,
        "fired_event_popups_rows",
        admitted_rows as u64,
    );
    add_compact_domain_cumulative_unit(
        summary,
        fired75::DOMAIN_ID,
        "fired_event_popups_unadmitted_rows",
        decoded.rows.len().saturating_sub(admitted_rows) as u64,
    );
    add_compact_domain_cumulative_unit(
        summary,
        fired75::DOMAIN_ID,
        "fired_event_popups_dropped_rows",
        decoded
            .telemetry
            .dropped_fire_rows
            .saturating_add(decoded.telemetry.dropped_resolution_rows),
    );
}

fn decode_compact_design_catalog_context_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match design78::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: design_catalog_context DCX1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let rows = usize::from(decoded.context.is_some());
    if let Some(domain) = summary.per_domain.get_mut(&design78::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(rows as u64);
    }
    add_compact_domain_data_unit(
        summary,
        design78::DOMAIN_ID,
        "design_catalog_context_rows",
        rows,
    );
    if let Some(context) = decoded.context {
        add_compact_domain_data_unit(
            summary,
            design78::DOMAIN_ID,
            "design_catalog_context_ordered_design_ids",
            context.ordered_design_ids.len(),
        );
    }
}

fn decode_compact_character_presentation_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match character82::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: character_presentation CPR1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let (
        tree_rows,
        node_rows,
        progress_rows,
        reason_context_rows,
        ace_rows,
        ace_modifier_text_bytes,
    ) = decoded
        .snapshot
        .map(|snapshot| {
            let node_rows = snapshot
                .trait_trees
                .iter()
                .map(|tree| tree.nodes.len())
                .sum();
            let progress_rows = snapshot
                .trait_trees
                .iter()
                .flat_map(|tree| &tree.nodes)
                .filter(|node| node.progress_raw.is_some())
                .count();
            let reason_context_rows = snapshot
                .trait_trees
                .iter()
                .flat_map(|tree| &tree.nodes)
                .filter(|node| node.reason_context_id.is_some())
                .count();
            let ace_modifier_text_bytes = snapshot
                .ace_portraits
                .iter()
                .map(|ace| ace.modifier_text.len())
                .sum();
            (
                snapshot.trait_trees.len(),
                node_rows,
                progress_rows,
                reason_context_rows,
                snapshot.ace_portraits.len(),
                ace_modifier_text_bytes,
            )
        })
        .unwrap_or((0, 0, 0, 0, 0, 0));
    if let Some(domain) = summary.per_domain.get_mut(&character82::DOMAIN_ID) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(tree_rows.saturating_add(ace_rows) as u64);
    }
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_trait_trees",
        tree_rows,
    );
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_trait_nodes",
        node_rows,
    );
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_trait_progress_values",
        progress_rows,
    );
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_trait_reason_contexts",
        reason_context_rows,
    );
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_ace_portraits",
        ace_rows,
    );
    add_compact_domain_data_unit(
        summary,
        character82::DOMAIN_ID,
        "character_presentation_ace_modifier_text_bytes",
        ace_modifier_text_bytes,
    );
}

fn decode_compact_scripted_gui_runtime_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match scripted84::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: scripted_gui_runtime SGR1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let (row_count, result_count, unavailable_count, tombstone_count) = decoded
        .frame
        .as_ref()
        .map(|frame| {
            let mut result_count = 0usize;
            let mut unavailable_count = 0usize;
            for row in &frame.rows {
                result_count = result_count.saturating_add(
                    2 + row.triggers.len() + row.properties.len() + row.effect_enabled.len(),
                );
                unavailable_count = unavailable_count
                    .saturating_add(usize::from(matches!(
                        &row.visible,
                        scripted_state_flags_variables_live_lane::EngineResult::Unavailable(_)
                    )))
                    .saturating_add(usize::from(matches!(
                        &row.enabled,
                        scripted_state_flags_variables_live_lane::EngineResult::Unavailable(_)
                    )))
                    .saturating_add(
                        row.triggers
                            .values()
                            .filter(|value| matches!(value, scripted_state_flags_variables_live_lane::EngineResult::Unavailable(_)))
                            .count(),
                    )
                    .saturating_add(
                        row.properties
                            .values()
                            .filter(|value| matches!(value, scripted_state_flags_variables_live_lane::EngineResult::Unavailable(_)))
                            .count(),
                    )
                    .saturating_add(
                        row.effect_enabled
                            .values()
                            .filter(|value| matches!(value, scripted_state_flags_variables_live_lane::EngineResult::Unavailable(_)))
                            .count(),
                    );
            }
            (
                frame.rows.len(),
                result_count,
                unavailable_count,
                frame.tombstones.len(),
            )
        })
        .unwrap_or((0, 0, 0, 0));
    if let Some(domain) = summary.per_domain.get_mut(&scripted84::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    }
    for (name, count) in [
        ("scripted_gui_runtime_rows", row_count),
        ("scripted_gui_runtime_results", result_count),
        ("scripted_gui_runtime_unavailable_results", unavailable_count),
        ("scripted_gui_runtime_tombstones", tombstone_count),
    ] {
        add_compact_domain_data_unit(summary, scripted84::DOMAIN_ID, name, count);
    }
}

fn decode_compact_nuke_stockpile_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match nuke83::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: nuke_stockpile NUK1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let (country_rows, entry_rows) = decoded
        .snapshot
        .map(|snapshot| {
            let entries = snapshot
                .countries
                .iter()
                .map(|country| country.entries.len())
                .sum();
            (snapshot.countries.len(), entries)
        })
        .unwrap_or((0, 0));
    if let Some(domain) = summary.per_domain.get_mut(&nuke83::DOMAIN_ID) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(country_rows as u64);
    }
    add_compact_domain_data_unit(
        summary,
        nuke83::DOMAIN_ID,
        "nuke_stockpile_countries",
        country_rows,
    );
    add_compact_domain_data_unit(
        summary,
        nuke83::DOMAIN_ID,
        "nuke_stockpile_entries",
        entry_rows,
    );
}

fn decode_compact_selected_designer_identity_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match selected85::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: selected_designer_identity SDI1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let ordered_rows = decoded
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.ordered_designs.len())
        .unwrap_or(0);
    if let Some(domain) = summary.per_domain.get_mut(&selected85::DOMAIN_ID) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(usize::from(decoded.snapshot.is_some()) as u64);
    }
    add_compact_domain_data_unit(
        summary,
        selected85::DOMAIN_ID,
        "selected_designer_frames",
        usize::from(decoded.snapshot.is_some()),
    );
    add_compact_domain_data_unit(
        summary,
        selected85::DOMAIN_ID,
        "selected_designer_ordered_designs",
        ordered_rows,
    );
}

fn decode_compact_peace_conference_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match peace97::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: peace_conference PCS1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    // A measured zero is an ANSWER but not a ROW: the hour looked at the
    // engine's peace-conference container and there was nothing in it to
    // describe. Counting it here would inflate `data_rows_total` by one per
    // peaceful hour -- which is nearly every hour of a campaign -- and make
    // d97 look like the busiest domain in the capture.
    let frame_rows = usize::from(matches!(
        decoded.observation,
        Some(
            peace97::PeaceConferenceObservation::Inactive { .. }
                | peace97::PeaceConferenceObservation::Active(_)
        )
    ));
    let active_rows = usize::from(matches!(
        decoded.observation,
        Some(peace97::PeaceConferenceObservation::Active(_))
    ));
    let no_conference_frames = usize::from(matches!(
        decoded.observation,
        Some(peace97::PeaceConferenceObservation::NoConferenceInProgress)
    ));
    if let Some(domain) = summary.per_domain.get_mut(&peace97::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(frame_rows as u64);
    }
    add_compact_domain_data_unit(
        summary,
        peace97::DOMAIN_ID,
        "peace_conference_frames",
        frame_rows,
    );
    add_compact_domain_data_unit(
        summary,
        peace97::DOMAIN_ID,
        "peace_conference_active_frames",
        active_rows,
    );
    // Published so a capture can state the shape of d97's hours instead of
    // leaving "no rows" to be read as "no answer". Per-frame by construction:
    // its value is computed from the payload of the frame it rides in.
    add_compact_domain_data_unit(
        summary,
        peace97::DOMAIN_ID,
        "peace_conference_no_conference_frames",
        no_conference_frames,
    );
}

/// Count what domain 98 actually measured, keeping the three outcomes apart.
///
/// `theatre_selector_admitted_sections` and
/// `theatre_selector_refused_sections` must be reported separately: collapsing
/// them would make a recording in which every country failed look identical to
/// one in which every country genuinely had no theatres.
fn decode_compact_theatre_selector_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match theatre98::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: theatre_selector THS2 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let Some(snapshot) = decoded.snapshot else {
        return;
    };
    let mut admitted = 0usize;
    let mut refused = 0usize;
    let mut theatres = 0usize;
    let mut members = 0usize;
    for section in &snapshot.sections {
        match &section.admission {
            theatre98::SectionAdmission::Unavailable(_) => refused += 1,
            theatre98::SectionAdmission::Admitted(body) => {
                admitted += 1;
                theatres = theatres.saturating_add(body.theatres.len());
                members = members.saturating_add(
                    body.theatres
                        .iter()
                        .map(|row| row.members.len())
                        .sum::<usize>()
                        .saturating_add(body.free_air_wings.len()),
                );
            }
        }
    }
    if let Some(domain) = summary.per_domain.get_mut(&theatre98::DOMAIN_ID) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(snapshot.sections.len() as u64);
    }
    add_compact_domain_data_unit(
        summary,
        theatre98::DOMAIN_ID,
        "theatre_selector_admitted_sections",
        admitted,
    );
    add_compact_domain_data_unit(
        summary,
        theatre98::DOMAIN_ID,
        "theatre_selector_refused_sections",
        refused,
    );
    add_compact_domain_data_unit(
        summary,
        theatre98::DOMAIN_ID,
        "theatre_selector_theatres",
        theatres,
    );
    add_compact_domain_data_unit(
        summary,
        theatre98::DOMAIN_ID,
        "theatre_selector_members",
        members,
    );
}

fn decode_compact_sunk_ship_history_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match sunk86::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: sunk_ship_history SHP1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let event_rows = decoded
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.events.len())
        .unwrap_or(0);
    if let Some(domain) = summary.per_domain.get_mut(&sunk86::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(event_rows as u64);
    }
    // `current_complete_snapshot` clones the complete writer history every
    // frame, so this is the history's LENGTH.
    add_compact_domain_cumulative_unit(
        summary,
        sunk86::DOMAIN_ID,
        "sunk_ship_writer_events",
        event_rows as u64,
    );
}

fn decode_compact_decision_panel_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match decision87::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: decision_panel DPN1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let row_count = decoded
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.rows.len())
        .unwrap_or(0);
    if let Some(domain) = summary.per_domain.get_mut(&decision87::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        decision87::DOMAIN_ID,
        "decision_panel_rows",
        row_count,
    );
}

fn decode_compact_active_command_abilities_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match abilities88::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: active_command_abilities ABA2 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let row_count = decoded
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.rows.len())
        .unwrap_or(0);
    if let Some(domain) = summary.per_domain.get_mut(&abilities88::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        abilities88::DOMAIN_ID,
        "active_command_ability_rows",
        row_count,
    );
}

fn decode_compact_national_focus_presentation_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() >= 16 {
        add_decoder_status(summary, read_u32_le(payload, 12));
    }
    let decoded = match focus89::decode_payload(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: national_focus_presentation NFP1 payload invalid: {error:?}"
            ));
            return;
        }
    };
    let row_count = decoded
        .snapshot
        .as_ref()
        .map(|snapshot| snapshot.visible_items.len())
        .unwrap_or(0);
    if let Some(domain) = summary.per_domain.get_mut(&focus89::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        focus89::DOMAIN_ID,
        "national_focus_presentation_items",
        row_count,
    );
}

// Shared helper: an optional u32 field packed with a presence flag. When present
// the value must be within [0, max]; when absent it must be zero-packed.
fn validate_compact_optional_u32(
    summary: &mut DomainDataDecodeSummary,
    offset: usize,
    row_index: usize,
    domain: &str,
    field: &str,
    value: u32,
    present: bool,
    max: u32,
) {
    if present {
        if value > max {
            summary.parse_errors.push(format!(
                "offset {offset}: {domain} row {row_index} {field} {value} out of range"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: {domain} row {row_index} {field} present without flag"
        ));
    }
}

fn validate_market_trade_detail_presence_i64(
    summary: &mut DomainDataDecodeSummary,
    offset: usize,
    row_index: usize,
    name: &str,
    value: i64,
    present: bool,
    min: i64,
    max: i64,
) {
    if present {
        if !(min..=max).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 row {row_index} has {name} without flag"
        ));
    }
}

fn validate_market_trade_detail_presence_i32(
    summary: &mut DomainDataDecodeSummary,
    offset: usize,
    row_index: usize,
    name: &str,
    value: i32,
    present: bool,
    min: i32,
    max: i32,
) {
    if present {
        if !(min..=max).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: market trade MTD1 row {row_index} has invalid {name} {value}"
            ));
        }
    } else if value != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: market trade MTD1 row {row_index} has {name} without flag"
        ));
    }
}

fn decode_compact_supply_logistics_attrition_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < COMPACT_SUPPLY_LOGISTICS_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics data payload is {} byte(s), expected at least {}",
            payload.len(),
            COMPACT_SUPPLY_LOGISTICS_HEADER_LEN
        ));
        return;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let scale_raw = read_u32_le(payload, 20);
    let province_rows = read_u32_le(payload, 24) as usize;
    let division_rows = read_u32_le(payload, 28) as usize;
    let badge_rows = read_u32_le(payload, 32) as usize;
    let observed_province_roots = read_u32_le(payload, 36) as usize;
    let observed_division_roots = read_u32_le(payload, 40) as usize;
    let csupply_input_rows = read_u32_le(payload, 44) as usize;
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if delta_data && changed_rows != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics delta changed_rows {changed_rows} != emitted row_count {row_count}"
        ));
    }
    if province_rows
        .saturating_add(division_rows)
        .saturating_add(badge_rows)
        .saturating_add(csupply_input_rows)
        != row_count
    {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row counters {province_rows}+{division_rows}+{badge_rows}+{csupply_input_rows} != row_count {row_count}"
        ));
    }
    if !delta_data && observed_province_roots < province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics observed_province_roots {observed_province_roots} < province_rows {province_rows}"
        ));
    }
    if !delta_data && observed_division_roots < division_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics observed_division_roots {observed_division_roots} < division_rows {division_rows}"
        ));
    }
    let expected_len = COMPACT_SUPPLY_LOGISTICS_HEADER_LEN
        .saturating_add(row_count.saturating_mul(COMPACT_SUPPLY_LOGISTICS_ROW_LEN));
    if payload.len() < expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics data payload is {} byte(s), expected at least {expected_len}",
            payload.len()
        ));
        return;
    }

    if let Some(domain) = summary.per_domain.get_mut(&44) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 44, "supply_province_logistics_rows", province_rows);
    add_compact_domain_data_unit(summary, 44, "supply_division_supply_rows", division_rows);
    add_compact_domain_data_unit(summary, 44, "supply_computed_badge_rows", badge_rows);
    add_compact_domain_data_unit(summary, 44, "supply_csupply_rows", csupply_input_rows);
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_observed_province_roots",
        observed_province_roots,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_observed_division_roots",
        observed_division_roots,
    );

    let mut decoded_province_rows = 0usize;
    let mut decoded_division_rows = 0usize;
    let mut decoded_badge_rows = 0usize;
    let mut decoded_csupply_input_rows = 0usize;
    let mut decoded_country_disrupted_supply_rows = 0usize;
    let mut decoded_country_disrupted_supply_live_rows = 0usize;
    let mut decoded_country_disrupted_supply_tombstones = 0usize;
    let mut seen_provinces = BTreeSet::new();
    let mut seen_divisions = BTreeSet::new();
    let mut seen_csupply_input_provinces = BTreeSet::new();
    let mut seen_country_disrupted_supply = BTreeSet::new();
    let mut seen_badge_flags = BTreeSet::new();
    let mut has_country_disrupted_source_available = false;
    let mut division_fuel_rows = 0usize;
    let mut division_supply_deficit_rows = 0usize;

    let mut cursor = COMPACT_SUPPLY_LOGISTICS_HEADER_LEN;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_SUPPLY_LOGISTICS_ROW_LEN,
            "supply logistics row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let row_kind = read_u32_le(payload, cursor);
        let entity_id = read_u32_le(payload, cursor + 4);
        let join_id = read_u32_le(payload, cursor + 8);
        let owner_tag_db_id = read_u32_le(payload, cursor + 12);
        let flags = read_u32_le(payload, cursor + 16);
        let value_a_raw = read_i32_le(payload, cursor + 20);
        let value_b_raw = read_i32_le(payload, cursor + 24);
        let value_c_raw = read_i32_le(payload, cursor + 28);
        let value_d_raw = read_i32_le(payload, cursor + 32);
        let aux_raw = read_i32_le(payload, cursor + 36);
        cursor += COMPACT_SUPPLY_LOGISTICS_ROW_LEN;

        supply44::domain44_contract::validate_row_flags(
            offset,
            row_index,
            row_kind,
            flags,
            COMPACT_SUPPLY_ROW_VALID_FLAGS,
            &mut summary.parse_errors,
        );
        match row_kind {
            COMPACT_SUPPLY_ROW_KIND_PROVINCE_LOGISTICS => {
                decoded_province_rows = decoded_province_rows.saturating_add(1);
                let deleted_id = compact_supply_logistics_deleted_id(entity_id);
                let province_id = deleted_id.unwrap_or(entity_id);
                if province_id == 0 || province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has invalid province_id {province_id}"
                    ));
                } else if !seen_provinces.insert(province_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} duplicates province_id {province_id}"
                    ));
                }
                if deleted_id.is_some() {
                    validate_supply_logistics_tombstone_fields(
                        offset,
                        summary,
                        row_index,
                        "province",
                        join_id,
                        owner_tag_db_id,
                        flags,
                        [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                    );
                    continue;
                }
                if join_id == 0 || join_id > COMPACT_SUPPLY_MAX_STATE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has invalid state_id {join_id}"
                    ));
                }
                if owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} has nonzero owner_tag_db_id"
                    ));
                }
                if flags & COMPACT_SUPPLY_ROW_FLAG_LIVE_BUILDING_VECTOR == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics province row {row_index} missing live building vector flag"
                    ));
                }
                for (label, value, flag) in [
                    ("rail", value_a_raw, COMPACT_SUPPLY_ROW_FLAG_RAIL),
                    (
                        "supply_node",
                        value_b_raw,
                        COMPACT_SUPPLY_ROW_FLAG_SUPPLY_NODE,
                    ),
                    (
                        "naval_supply_hub",
                        value_c_raw,
                        COMPACT_SUPPLY_ROW_FLAG_NAVAL_SUPPLY_HUB,
                    ),
                    (
                        "infrastructure",
                        value_d_raw,
                        COMPACT_SUPPLY_ROW_FLAG_INFRASTRUCTURE,
                    ),
                ] {
                    validate_supply_logistics_nonnegative_value(
                        offset,
                        summary,
                        row_index,
                        label,
                        value,
                        COMPACT_SUPPLY_MAX_BUILDING_LEVEL,
                    );
                    validate_supply_logistics_presence_flag(
                        offset, summary, row_index, label, value, flags, flag,
                    );
                }
                validate_supply_logistics_nonnegative_value(
                    offset,
                    summary,
                    row_index,
                    "building_count",
                    aux_raw,
                    COMPACT_SUPPLY_MAX_BUILDING_LEVEL,
                );
            }
            COMPACT_SUPPLY_ROW_KIND_DIVISION_SUPPLY => {
                decoded_division_rows = decoded_division_rows.saturating_add(1);
                let deleted_id = compact_supply_logistics_deleted_id(entity_id);
                let division_id = deleted_id.unwrap_or(entity_id);
                if division_id == 0 || division_id > COMPACT_SUPPLY_MAX_DIVISION_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has invalid division_id {division_id}"
                    ));
                } else if !seen_divisions.insert(division_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} duplicates division_id {division_id}"
                    ));
                }
                if deleted_id.is_some() {
                    validate_supply_logistics_tombstone_fields(
                        offset,
                        summary,
                        row_index,
                        "division",
                        join_id,
                        owner_tag_db_id,
                        flags,
                        [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                    );
                    continue;
                }
                if join_id == 0 || join_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has invalid province_id {join_id}"
                    ));
                }
                if flags & COMPACT_SUPPLY_ROW_FLAG_PROXY_VALUE == 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} missing proxy flag"
                    ));
                }
                if flags & COMPACT_SUPPLY_ROW_FLAG_DIVISION_OWNER != 0 {
                    if owner_tag_db_id == 0 || owner_tag_db_id > COMPACT_SUPPLY_MAX_TAG_DB_ID {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics division row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                        ));
                    }
                } else if owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has owner without owner flag"
                    ));
                }
                for (label, value, max_value, flag) in [
                    (
                        "current_supply_raw",
                        value_a_raw,
                        COMPACT_SUPPLY_MAX_SUPPLY_RAW,
                        COMPACT_SUPPLY_ROW_FLAG_DIVISION_CURRENT_SUPPLY,
                    ),
                    (
                        "max_supply_raw",
                        value_b_raw,
                        COMPACT_SUPPLY_MAX_SUPPLY_RAW,
                        COMPACT_SUPPLY_ROW_FLAG_DIVISION_MAX_SUPPLY,
                    ),
                    (
                        "supply_gain_raw",
                        value_c_raw,
                        COMPACT_SUPPLY_MAX_SUPPLY_GAIN_RAW,
                        COMPACT_SUPPLY_ROW_FLAG_DIVISION_SUPPLY_GAIN,
                    ),
                ] {
                    validate_supply_logistics_nonnegative_value(
                        offset, summary, row_index, label, value, max_value,
                    );
                    validate_supply_logistics_presence_flag(
                        offset, summary, row_index, label, value, flags, flag,
                    );
                }
                validate_supply_logistics_nonnegative_value(
                    offset,
                    summary,
                    row_index,
                    "supply_deficit_raw",
                    value_d_raw,
                    COMPACT_SUPPLY_MAX_SUPPLY_RAW,
                );
                let expected_deficit = value_b_raw.saturating_sub(value_a_raw).max(0);
                if value_d_raw != expected_deficit {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics division row {row_index} has supply_deficit_raw {value_d_raw} but expected {expected_deficit}"
                    ));
                }
                if value_d_raw > 0 {
                    division_supply_deficit_rows = division_supply_deficit_rows.saturating_add(1);
                }
                validate_supply_logistics_presence_flag(
                    offset,
                    summary,
                    row_index,
                    "supply_deficit_raw",
                    value_d_raw,
                    flags,
                    COMPACT_SUPPLY_ROW_FLAG_DIVISION_SUPPLY_DEFICIT,
                );
                validate_supply_logistics_nonnegative_value(
                    offset,
                    summary,
                    row_index,
                    "fuel_raw",
                    aux_raw,
                    COMPACT_SUPPLY_MAX_DIVISION_FUEL_RAW,
                );
                if aux_raw > 0 {
                    division_fuel_rows = division_fuel_rows.saturating_add(1);
                }
                validate_supply_logistics_presence_flag(
                    offset,
                    summary,
                    row_index,
                    "fuel_raw",
                    aux_raw,
                    flags,
                    COMPACT_SUPPLY_ROW_FLAG_DIVISION_FUEL,
                );
            }
            COMPACT_SUPPLY_ROW_KIND_CSUPPLY_PROVINCE_INPUT => {
                decoded_csupply_input_rows = decoded_csupply_input_rows.saturating_add(1);
                let deleted_id = compact_supply_logistics_deleted_id(entity_id);
                let province_id = deleted_id.unwrap_or(entity_id);
                if province_id == 0 || province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply row {row_index} has invalid province_id {province_id}"
                    ));
                } else if !seen_csupply_input_provinces.insert(province_id) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply row {row_index} duplicates province_id {province_id}"
                    ));
                }
                if deleted_id.is_some() {
                    validate_supply_logistics_tombstone_fields(
                        offset,
                        summary,
                        row_index,
                        "csupply",
                        join_id,
                        owner_tag_db_id,
                        flags,
                        [value_a_raw, value_b_raw, value_c_raw, value_d_raw, aux_raw],
                    );
                    continue;
                }
                if join_id != 0 || owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply row {row_index} has nonzero identity join fields"
                    ));
                }
                let expected_flags = COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_EFFICIENCY
                    | COMPACT_SUPPLY_ROW_FLAG_CSUPPLY_PROVINCE_ARRAY;
                if flags != expected_flags {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply row {row_index} has invalid flags {flags}"
                    ));
                }
                validate_supply_logistics_nonnegative_value(
                    offset,
                    summary,
                    row_index,
                    "csupply_input_raw",
                    value_a_raw,
                    COMPACT_SUPPLY_MAX_CSUPPLY_INPUT_RAW,
                );
                if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics csupply row {row_index} has nonzero reserved value fields"
                    ));
                }
            }
            COMPACT_SUPPLY_ROW_KIND_COUNTRY_DISRUPTED_SUPPLY => {
                decoded_country_disrupted_supply_rows =
                    decoded_country_disrupted_supply_rows.saturating_add(1);
                let deleted_id = compact_supply_logistics_deleted_id(entity_id);
                let province_id = deleted_id.unwrap_or(entity_id);
                if province_id == 0 || province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has invalid province_id {province_id}"
                    ));
                }
                if owner_tag_db_id == 0
                    || owner_tag_db_id > COMPACT_SUPPLY_MAX_TAG_DB_ID
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
                    ));
                }
                if !seen_country_disrupted_supply.insert((
                    owner_tag_db_id,
                    province_id,
                    join_id,
                )) {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} duplicates owner/province/node_type identity"
                    ));
                }
                if deleted_id.is_some() {
                    decoded_country_disrupted_supply_tombstones =
                        decoded_country_disrupted_supply_tombstones.saturating_add(1);
                    if flags != 0
                        || value_a_raw != 0
                        || value_b_raw != 0
                        || value_c_raw != 0
                        || value_d_raw != 0
                        || aux_raw != 0
                    {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics disrupted tombstone row {row_index} has live value fields"
                        ));
                    }
                    continue;
                }
                decoded_country_disrupted_supply_live_rows =
                    decoded_country_disrupted_supply_live_rows.saturating_add(1);
                if flags != COMPACT_SUPPLY_ROW_FLAG_COUNTRY_DISRUPTED_SUPPLY_EXACT {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has invalid flags {flags:#x}"
                    ));
                }
                validate_supply_logistics_nonnegative_value(
                    offset,
                    summary,
                    row_index,
                    "country_disrupted_supply_raw",
                    value_a_raw,
                    COMPACT_SUPPLY_MAX_DISRUPTED_RAW,
                );
                if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics disrupted row {row_index} has nonzero reserved value fields"
                    ));
                }
            }
            COMPACT_SUPPLY_ROW_KIND_COMPUTED_BADGE => {
                decoded_badge_rows = decoded_badge_rows.saturating_add(1);
                if entity_id != 0 || join_id != 0 || owner_tag_db_id != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} has nonzero identity fields"
                    ));
                }
                if value_b_raw != 0 || value_c_raw != 0 || value_d_raw != 0 || aux_raw != 0 {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply logistics badge row {row_index} has nonzero reserved value fields"
                    ));
                }
                if let Some(spec) = supply44::domain44_contract::validate_badge_row(
                    offset,
                    row_index,
                    flags,
                    value_a_raw,
                    &mut summary.parse_errors,
                ) {
                    if !seen_badge_flags.insert(spec.flag) {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply logistics badge row {row_index} duplicates badge flag {:#x}",
                            spec.flag
                        ));
                    }
                    has_country_disrupted_source_available |=
                        spec.flag == COMPACT_SUPPLY_ROW_FLAG_COUNTRY_DISRUPTED_SOURCE_AVAILABLE;
                }
            }
            other => summary.parse_errors.push(format!(
                "offset {offset}: supply logistics row {row_index} has unknown row_kind {other}"
            )),
        }
    }

    // Additive D44 trailer chain: optional FHB1, optional ATR1, optional ARC1,
    // optional PST1.
    // PST1 is last because its strict codec owns EOF.
    let mut extension_cursor = expected_len;
    let mut foreign_homebase_extension_present = false;
    if payload.len().saturating_sub(extension_cursor) >= 4
        && read_u32_le(payload, extension_cursor) == u32::from_le_bytes(*b"FHB1")
    {
        foreign_homebase_extension_present = true;
        if payload.len().saturating_sub(extension_cursor) < 16 {
            summary.parse_errors.push(format!("offset {offset}: truncated supply FHB1 header"));
            return;
        }
        let row_count = read_u32_le(payload, extension_cursor + 8) as usize;
        if read_u32_le(payload, extension_cursor + 4) != 1
            || read_u32_le(payload, extension_cursor + 12) != 0
        {
            summary.parse_errors.push(format!("offset {offset}: invalid supply FHB1 header"));
            return;
        }
        extension_cursor += 16;
        if row_count > payload.len().saturating_sub(extension_cursor) / 80 {
            summary.parse_errors.push(format!(
                "offset {offset}: supply FHB1 row_count exceeds payload byte budget"
            ));
            return;
        }
        let mut previous_key: Option<(u32, u32, String, u32)> = None;
        for row_index in 0..row_count {
            if payload.len().saturating_sub(extension_cursor) < 80 {
                summary.parse_errors.push(format!(
                    "offset {offset}: truncated supply FHB1 row {row_index}"
                ));
                return;
            }
            let owner_tag_db_id = read_u32_le(payload, extension_cursor);
            let province_id = read_u32_le(payload, extension_cursor + 4);
            let occurrence = read_u32_le(payload, extension_cursor + 8);
            let flags = read_u32_le(payload, extension_cursor + 12);
            let supply_raw = read_i64_le(payload, extension_cursor + 16);
            let start_raw = read_i64_le(payload, extension_cursor + 24);
            let penalty_raw = read_i64_le(payload, extension_cursor + 32);
            let added_penalty_raw = read_i64_le(payload, extension_cursor + 40);
            let duration_raw = read_i64_le(payload, extension_cursor + 48);
            let hours_raw = read_i64_le(payload, extension_cursor + 56);
            let decay_raw = read_i64_le(payload, extension_cursor + 64);
            let base_raw = payload[extension_cursor + 72];
            if payload[extension_cursor + 73..extension_cursor + 76]
                .iter()
                .any(|byte| *byte != 0)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply FHB1 row {row_index} has nonzero reserved bytes"
                ));
                return;
            }
            let source_len = read_u32_le(payload, extension_cursor + 76) as usize;
            if owner_tag_db_id == 0
                || owner_tag_db_id > COMPACT_SUPPLY_MAX_TAG_DB_ID
                || province_id == 0
                || province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID
                || flags & !1 != 0
                || base_raw > 1
                || source_len > 4_096
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply FHB1 row {row_index} identity/flags/base/source length invalid"
                ));
                return;
            }
            let source_start = match extension_cursor.checked_add(80) {
                Some(cursor) => cursor,
                None => {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply FHB1 row {row_index} offset overflow"
                    ));
                    return;
                }
            };
            let source_end = match source_start
                .checked_add(source_len)
            {
                Some(cursor) if cursor <= payload.len() => cursor,
                _ => {
                    summary.parse_errors.push(format!(
                        "offset {offset}: invalid supply FHB1 source length at row {row_index}"
                    ));
                    return;
                }
            };
            let source = match std::str::from_utf8(&payload[source_start..source_end]) {
                Ok(source) => source.to_owned(),
                Err(_) => {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply FHB1 row {row_index} source is not utf8"
                    ));
                    return;
                }
            };
            let deleted = flags & 1 != 0;
            if deleted
                && (!delta_data
                    || supply_raw != 0
                    || start_raw != 0
                    || penalty_raw != 0
                    || added_penalty_raw != 0
                    || duration_raw != 0
                    || hours_raw != 0
                    || decay_raw != 0
                    || base_raw != 0)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply FHB1 row {row_index} has invalid tombstone"
                ));
                return;
            }
            let key = (owner_tag_db_id, province_id, source, occurrence);
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply FHB1 row {row_index} is duplicate or not canonical"
                ));
                return;
            }
            if !delta_data {
                let expected_occurrence = match previous_key.as_ref() {
                    Some(previous)
                        if previous.0 == key.0
                            && previous.1 == key.1
                            && previous.2 == key.2 =>
                    {
                        match previous.3.checked_add(1) {
                            Some(value) => value,
                            None => {
                                summary.parse_errors.push(format!(
                                    "offset {offset}: supply FHB1 row {row_index} occurrence overflow"
                                ));
                                return;
                            }
                        }
                    }
                    _ => 0,
                };
                if occurrence != expected_occurrence {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply FHB1 row {row_index} occurrence {occurrence} != canonical {expected_occurrence}"
                    ));
                    return;
                }
            }
            previous_key = Some(key);
            extension_cursor = source_end;
        }
    }

    let mut atr_rows = 0usize;
    if payload.len().saturating_sub(extension_cursor) >= 4
        && read_u32_le(payload, extension_cursor) == u32::from_le_bytes(*b"ATR1")
    {
        if payload.len().saturating_sub(extension_cursor) < 16 {
            summary.parse_errors.push(format!("offset {offset}: truncated supply ATR1 header"));
            return;
        }
        atr_rows = read_u32_le(payload, extension_cursor + 8) as usize;
        if read_u32_le(payload, extension_cursor + 4) != 1
            || read_u32_le(payload, extension_cursor + 12) != 0
        {
            summary.parse_errors.push(format!("offset {offset}: invalid supply ATR1 header"));
            return;
        }
        extension_cursor += 16;
        let mut prior_id = None;
        for row_index in 0..atr_rows {
            if payload.len().saturating_sub(extension_cursor) < 64 {
                summary.parse_errors.push(format!(
                    "offset {offset}: truncated supply ATR1 row {row_index}"
                ));
                return;
            }
            let division_id = read_u32_le(payload, extension_cursor);
            let province_id = read_u32_le(payload, extension_cursor + 4);
            let owner_tag_db_id = read_u32_le(payload, extension_cursor + 8);
            let flags = read_u32_le(payload, extension_cursor + 12);
            let total_raw = read_i64_le(payload, extension_cursor + 16);
            let threshold_raw = read_i64_le(payload, extension_cursor + 24);
            let base_raw = read_i64_le(payload, extension_cursor + 32);
            let modifier_raw = read_i64_le(payload, extension_cursor + 40);
            let reserved0 = read_i64_le(payload, extension_cursor + 48);
            let reserved1 = read_i64_le(payload, extension_cursor + 56);
            extension_cursor += 64;
            const DELETED: u32 = 1;
            const OWNER: u32 = 1 << 1;
            const TOTAL: u32 = 1 << 2;
            const THRESHOLD: u32 = 1 << 3;
            const BADGE: u32 = 1 << 4;
            const ACTIVE: u32 = 1 << 5;
            const REASONS_TRANSIENT: u32 = 1 << 6;
            const KNOWN: u32 = DELETED | OWNER | TOTAL | THRESHOLD | BADGE | ACTIVE | REASONS_TRANSIENT;
            if division_id == 0
                || division_id > COMPACT_SUPPLY_MAX_DIVISION_ID
                || prior_id.is_some_and(|prior| prior >= division_id)
                || flags & !KNOWN != 0
                || reserved0 != 0
                || reserved1 != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply ATR1 row {row_index} identity/flags invalid"
                ));
                continue;
            }
            prior_id = Some(division_id);
            if flags & DELETED != 0 {
                if !delta_data
                    || flags != DELETED
                    || province_id != 0
                    || owner_tag_db_id != 0
                    || total_raw != 0
                    || threshold_raw != 0
                    || base_raw != 0
                    || modifier_raw != 0
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply ATR1 tombstone row {row_index} invalid"
                    ));
                }
                continue;
            }
            if province_id == 0
                || province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID
                || owner_tag_db_id == 0
                || owner_tag_db_id > COMPACT_SUPPLY_MAX_TAG_DB_ID
                || flags & (OWNER | TOTAL | REASONS_TRANSIENT)
                    != (OWNER | TOTAL | REASONS_TRANSIENT)
                || (flags & THRESHOLD != 0) != (flags & BADGE != 0)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply ATR1 live row {row_index} identity/exactness invalid"
                ));
                continue;
            }
            if flags & THRESHOLD != 0 {
                let expected = (i128::from(100_000i64) + i128::from(modifier_raw))
                    .max(0)
                    .checked_mul(i128::from(base_raw))
                    .map(|value| value / i128::from(100_000i64));
                if expected != Some(i128::from(threshold_raw))
                    || (flags & ACTIVE != 0) != (total_raw > threshold_raw)
                {
                    summary.parse_errors.push(format!(
                        "offset {offset}: supply ATR1 row {row_index} fails exact badge law"
                    ));
                }
            } else if flags & ACTIVE != 0
                || threshold_raw != 0
                || base_raw != 0
                || modifier_raw != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: supply ATR1 row {row_index} has partial badge law"
                ));
            }
        }
        if !delta_data && atr_rows != observed_division_roots {
            summary.parse_errors.push(format!(
                "offset {offset}: supply ATR1 rows {atr_rows} != observed division roots {observed_division_roots}"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 44, "supply_division_attrition_i64_rows", atr_rows);

    if payload.len().saturating_sub(extension_cursor) >= 4
        && read_u32_le(payload, extension_cursor)
            == supply44::ATTRITION_COMPONENT_TRAILER_MAGIC
    {
        match supply44::decode_attrition_component_trailer_prefix(&payload[extension_cursor..]) {
            Ok((batch, consumed)) => {
                extension_cursor = match extension_cursor.checked_add(consumed) {
                    Some(cursor) => cursor,
                    None => {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply ARC1 end offset overflow"
                        ));
                        return;
                    }
                };
                if batch.dropped_rows == 0 {
                    if batch.rows.len() != observed_division_roots as usize
                        || batch.rows.iter().any(|row| {
                            row.flags == supply44::ATTRITION_COMPONENT_FLAG_DELETED
                        })
                    {
                        summary.parse_errors.push(format!(
                            "offset {offset}: supply ARC1 is not a complete live current-frame division roster"
                        ));
                        return;
                    }
                    add_compact_domain_data_unit(
                        summary,
                        44,
                        "supply_division_attrition_component_rows",
                        batch.rows.len(),
                    );
                }
            }
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: invalid supply ARC1 trailer: {error:?}"
                ));
                return;
            }
        }
    }

    if extension_cursor < payload.len() {
        if payload.len().saturating_sub(extension_cursor) < 4
            || read_u32_le(payload, extension_cursor) != supply44::TRAILER_MAGIC
        {
            summary.parse_errors.push(format!(
                "offset {offset}: unknown/truncated supply trailer at byte {extension_cursor}"
            ));
            return;
        }
        match supply44::decode_trailer(&payload[extension_cursor..]) {
            Ok(batch) => add_compact_domain_data_unit(
                summary,
                44,
                "supply_exact_tooltip_observation_rows",
                batch.rows.len(),
            ),
            Err(error) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: invalid supply PST1 trailer: {error:?}"
                ));
                return;
            }
        }
    }

    if decoded_province_rows != province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics province_rows {province_rows} != decoded {decoded_province_rows}"
        ));
    }
    if decoded_division_rows != division_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics division_rows {division_rows} != decoded {decoded_division_rows}"
        ));
    }
    if decoded_badge_rows != badge_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics badge_rows {badge_rows} != decoded {decoded_badge_rows}"
        ));
    }
    if decoded_csupply_input_rows.saturating_add(decoded_country_disrupted_supply_rows)
        != csupply_input_rows
    {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics csupply_rows {csupply_input_rows} != decoded inputs {} + disrupted {}",
            decoded_csupply_input_rows, decoded_country_disrupted_supply_rows
        ));
    }
    // Constant badges delta once and are retained by the fold. Baselines must
    // carry the shared contract's four unconditional badges plus exactly the
    // pressure badge selected by FHB1 presence. FHB1 row_count=0 remains a
    // successful exact EMPTY read, not a source failure.
    if !delta_data {
        let seen_badge_flags = seen_badge_flags
            .iter()
            .copied()
            .fold(0u32, |seen, flag| seen | flag);
        supply44::domain44_contract::validate_baseline_badge_coverage(
            offset,
            seen_badge_flags,
            foreign_homebase_extension_present,
            &mut summary.parse_errors,
        );
        if decoded_country_disrupted_supply_live_rows > 0
            && !has_country_disrupted_source_available
        {
            summary.parse_errors.push(format!(
                "offset {offset}: supply logistics disrupted rows lack exact-source availability badge"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 44, "supply_division_fuel_rows", division_fuel_rows);
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_division_deficit_rows",
        division_supply_deficit_rows,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_csupply_efficiency_rows",
        decoded_csupply_input_rows,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_csupply_input_rows",
        decoded_csupply_input_rows,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_country_disrupted_supply_rows",
        decoded_country_disrupted_supply_live_rows,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_country_disrupted_supply_tombstones",
        decoded_country_disrupted_supply_tombstones,
    );
    add_compact_domain_data_unit(
        summary,
        44,
        "supply_country_disrupted_source_available",
        usize::from(has_country_disrupted_source_available),
    );
}

fn validate_supply_logistics_nonnegative_value(
    offset: usize,
    summary: &mut DomainDataDecodeSummary,
    row_index: usize,
    label: &str,
    value: i32,
    max_value: i32,
) {
    if !(0..=max_value).contains(&value) {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} has invalid {label} {value}"
        ));
    }
}

fn validate_supply_logistics_presence_flag(
    offset: usize,
    summary: &mut DomainDataDecodeSummary,
    row_index: usize,
    label: &str,
    value: i32,
    flags: u32,
    flag: u32,
) {
    let present = flags & flag != 0;
    if value != 0 && !present {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics row {row_index} has nonzero {label} without presence flag"
        ));
    }
}

fn compact_supply_logistics_deleted_id(entity_id: u32) -> Option<u32> {
    (entity_id & COMPACT_SUPPLY_ROW_ID_DELETED != 0)
        .then_some(entity_id & !COMPACT_SUPPLY_ROW_ID_DELETED)
}

fn validate_supply_logistics_tombstone_fields(
    offset: usize,
    summary: &mut DomainDataDecodeSummary,
    row_index: usize,
    family: &str,
    join_id: u32,
    owner_tag_db_id: u32,
    flags: u32,
    values: [i32; 5],
) {
    if join_id != 0 || owner_tag_db_id != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics {family} tombstone row {row_index} has live identity fields"
        ));
    }
    if flags != 0 || values.iter().any(|value| *value != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: supply logistics {family} tombstone row {row_index} has live fields"
        ));
    }
}

fn decode_compact_air_variant_reserves_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_country_rows = read_u32_le(payload, 20) as usize;
    let observed_stockpile_rows = read_u32_le(payload, 24) as usize;
    let observed_air_variant_rows = read_u32_le(payload, 28) as usize;
    let matched_stockpile_rows = read_u32_le(payload, 32) as usize;
    let invalid_country_rows = read_u32_le(payload, 36) as usize;
    let deleted_rows = read_u32_le(payload, 40) as usize;
    let scale_raw = read_u32_le(payload, 44);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if matched_stockpile_rows > observed_stockpile_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves matched_stockpile_rows {matched_stockpile_rows} > observed_stockpile_rows {observed_stockpile_rows}"
        ));
    }
    if invalid_country_rows > observed_country_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves invalid_country_rows {invalid_country_rows} > observed_country_rows {observed_country_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if scale_raw != COMPACT_PRODUCTION_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves scale {scale_raw} != expected {COMPACT_PRODUCTION_SCALE_RAW}"
        ));
    }
    let expected_len =
        48usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_VARIANT_RESERVE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air variant reserves data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&36) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 36, "air_variant_reserve_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_observed_country_rows",
        observed_country_rows,
    );
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_observed_stockpile_rows",
        observed_stockpile_rows,
    );
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_observed_air_variant_rows",
        observed_air_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_matched_stockpile_rows",
        matched_stockpile_rows,
    );
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_invalid_country_rows",
        invalid_country_rows,
    );
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_deleted_rows",
        deleted_rows,
    );

    let mut seen_rows = BTreeSet::new();
    let mut amount_raw_sum = 0i128;
    let mut cursor = 48usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_VARIANT_RESERVE_ROW_LEN,
            "air variant reserves row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let tag_db_id = read_u32_le(payload, cursor);
        let variant_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let row_reserved = read_u32_le(payload, cursor + 12);
        let amount_raw = read_i64_le(payload, cursor + 16);
        cursor += COMPACT_AIR_VARIANT_RESERVE_ROW_LEN;
        let deleted = flags & COMPACT_AIR_VARIANT_RESERVE_ROW_FLAG_DELETED != 0;
        if tag_db_id == 0 || tag_db_id > COMPACT_AIR_VARIANT_RESERVE_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        }
        if variant_id == 0 || variant_id > COMPACT_AIR_VARIANT_RESERVE_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} has invalid variant_id {variant_id}"
            ));
        }
        if !seen_rows.insert((tag_db_id, variant_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} duplicates tag_db_id {tag_db_id} variant_id {variant_id}"
            ));
        }
        if flags & !COMPACT_AIR_VARIANT_RESERVE_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} reserved field is nonzero"
            ));
        }
        if !(-COMPACT_AIR_VARIANT_RESERVE_MAX_AMOUNT_RAW
            ..=COMPACT_AIR_VARIANT_RESERVE_MAX_AMOUNT_RAW)
            .contains(&amount_raw)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves row {row_index} has invalid amount_raw {amount_raw}"
            ));
        }
        if deleted && amount_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air variant reserves deleted row {row_index} carries nonzero amount"
            ));
        }
        if !deleted {
            amount_raw_sum = amount_raw_sum.saturating_add(amount_raw as i128);
        }
    }
    add_compact_domain_data_unit(
        summary,
        36,
        "air_variant_reserve_amount_raw_sum",
        amount_raw_sum.max(0).min(usize::MAX as i128) as usize,
    );
}

fn decode_compact_air_superiority_inputs_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 52 {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs data payload is {} byte(s), expected at least 52",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_wing_rows = read_u32_le(payload, 20) as usize;
    let observed_loadout_rows = read_u32_le(payload, 24) as usize;
    let observed_equipment_rows = read_u32_le(payload, 28) as usize;
    let observed_variant_rows = read_u32_le(payload, 32) as usize;
    let invalid_air_wing_rows = read_u32_le(payload, 36) as usize;
    let invalid_variant_rows = read_u32_le(payload, 40) as usize;
    let deleted_rows = read_u32_le(payload, 44) as usize;
    let scale_raw = read_u32_le(payload, 48);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_equipment_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs row_count {row_count} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if observed_loadout_rows > observed_air_wing_rows
        && status_flags & COMPACT_STATUS_FAILURE_MASK == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs observed_loadout_rows {observed_loadout_rows} > observed_air_wing_rows {observed_air_wing_rows}"
        ));
    }
    if invalid_variant_rows > observed_equipment_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs invalid_variant_rows {invalid_variant_rows} > observed_equipment_rows {observed_equipment_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs scale {scale_raw} != expected {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    let expected_len =
        52usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_SUPERIORITY_INPUT_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air superiority inputs data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&38) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 38, "air_superiority_input_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_observed_wing_rows",
        observed_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_observed_loadout_rows",
        observed_loadout_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_observed_equipment_rows",
        observed_equipment_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_observed_variant_rows",
        observed_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_invalid_wing_rows",
        invalid_air_wing_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_invalid_variant_rows",
        invalid_variant_rows,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_deleted_rows",
        deleted_rows,
    );

    let mut seen_rows = BTreeSet::new();
    let mut mission_bucket_rows = BTreeSet::new();
    let mut wing_count_sum = 0usize;
    let mut amount_raw_sum = 0i128;
    let mut cursor = 52usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_SUPERIORITY_INPUT_ROW_LEN,
            "air superiority inputs row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let region_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let mission_mask = read_u32_le(payload, cursor + 8);
        let variant_id = read_u32_le(payload, cursor + 12);
        let flags = read_u32_le(payload, cursor + 16);
        let equipment_token_raw = read_u32_le(payload, cursor + 20);
        let equipment_type_token_raw = read_u32_le(payload, cursor + 24);
        let interface_type_raw = read_u32_le(payload, cursor + 28);
        let wing_count = read_u32_le(payload, cursor + 32);
        let row_reserved = read_u32_le(payload, cursor + 36);
        let amount_raw = read_i64_le(payload, cursor + 40);
        cursor += COMPACT_AIR_SUPERIORITY_INPUT_ROW_LEN;

        let deleted = flags & COMPACT_AIR_SUPERIORITY_INPUT_ROW_FLAG_DELETED != 0;
        if region_id == 0 || region_id > COMPACT_AIR_SUPERIORITY_INPUT_MAX_REGION_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has invalid region_id {region_id}"
            ));
        }
        if owner_tag_db_id == 0
            || owner_tag_db_id > COMPACT_AIR_SUPERIORITY_INPUT_MAX_OWNER_TAG_DB_ID
        {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has invalid owner_tag_db_id {owner_tag_db_id}"
            ));
        }
        if mission_mask == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has zero mission_mask"
            ));
        }
        if variant_id == 0 || variant_id > COMPACT_AIR_SUPERIORITY_INPUT_MAX_VARIANT_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has invalid variant_id {variant_id}"
            ));
        }
        if !seen_rows.insert((region_id, owner_tag_db_id, mission_mask, variant_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} duplicates region/owner/mission/variant"
            ));
        }
        if flags & !COMPACT_AIR_SUPERIORITY_INPUT_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} reserved field is nonzero"
            ));
        }
        if deleted {
            if equipment_token_raw != 0
                || equipment_type_token_raw != 0
                || interface_type_raw != 0
                || wing_count != 0
                || amount_raw != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air superiority inputs deleted row {row_index} carries live fields"
                ));
            }
            continue;
        }
        if equipment_token_raw == 0 || equipment_type_token_raw == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has zero equipment token/type fields"
            ));
        }
        if wing_count == 0 || wing_count > COMPACT_AIR_SUPERIORITY_INPUT_MAX_WING_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has invalid wing_count {wing_count}"
            ));
        }
        if amount_raw <= 0 || amount_raw > COMPACT_AIR_SUPERIORITY_INPUT_MAX_AMOUNT_RAW {
            summary.parse_errors.push(format!(
                "offset {offset}: air superiority inputs row {row_index} has invalid amount_raw {amount_raw}"
            ));
        }
        mission_bucket_rows.insert((region_id, owner_tag_db_id, mission_mask));
        wing_count_sum = wing_count_sum.saturating_add(wing_count as usize);
        amount_raw_sum = amount_raw_sum.saturating_add(amount_raw as i128);
    }
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_mission_bucket_rows",
        mission_bucket_rows.len(),
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_wing_count_sum",
        wing_count_sum,
    );
    add_compact_domain_data_unit(
        summary,
        38,
        "air_superiority_input_amount_raw_sum",
        amount_raw_sum.max(0).min(usize::MAX as i128) as usize,
    );
}

fn decode_compact_air_base_status_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 36 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status data payload is {} byte(s), expected at least 36",
            payload.len()
        ));
        return;
    }
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let observed_air_base_rows = read_u32_le(payload, 20) as usize;
    let observed_pool_rows = read_u32_le(payload, 24) as usize;
    let deleted_rows = read_u32_le(payload, 28) as usize;
    let reserved = read_u32_le(payload, 32);
    add_decoder_status(summary, status_flags);
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if row_count > observed_air_base_rows && (status_flags & COMPACT_STATUS_DELTA_DATA) == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status row_count {row_count} > observed_air_base_rows {observed_air_base_rows}"
        ));
    }
    if deleted_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status deleted_rows {deleted_rows} > row_count {row_count}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status header reserved field is nonzero"
        ));
    }
    let expected_len =
        36usize.saturating_add(row_count.saturating_mul(COMPACT_AIR_BASE_STATUS_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: air base status data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if let Some(domain) = summary.per_domain.get_mut(&34) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 34, "air_base_status_rows", row_count);
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_observed_air_base_rows",
        observed_air_base_rows,
    );
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_observed_pool_rows",
        observed_pool_rows,
    );
    add_compact_domain_data_unit(summary, 34, "air_base_status_deleted_rows", deleted_rows);

    let mut seen_air_bases = BTreeSet::new();
    let mut capacity_sum = 0usize;
    let mut based_wing_sum = 0usize;
    let mut based_plane_sum = 0usize;
    let mut cursor = 36usize;
    for row_index in 0..row_count {
        if let Err(err) = require_compact_remaining(
            payload,
            cursor,
            COMPACT_AIR_BASE_STATUS_ROW_LEN,
            "air base status row",
        ) {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
            return;
        }
        let air_base_id = read_u32_le(payload, cursor);
        let state_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let capacity = read_u32_le(payload, cursor + 12);
        let building_level = read_u32_le(payload, cursor + 16);
        let based_wing_count = read_u32_le(payload, cursor + 20);
        let based_plane_count = read_u32_le(payload, cursor + 24);
        // A7 exact-a729 placement closure promoted the former reserved word to
        // the pointer-free CAirBase -> CProvince id. Keep zero compatible with
        // older/unresolved rows, but validate every present province id.
        let province_id = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_AIR_BASE_STATUS_ROW_LEN;
        let deleted = (flags & COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED) != 0;
        if air_base_id == 0 || air_base_id > COMPACT_AIR_BASE_STATUS_MAX_AIR_BASE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid air_base_id {air_base_id}"
            ));
        } else if !seen_air_bases.insert(air_base_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} duplicates air_base_id {air_base_id}"
            ));
        }
        if state_id == 0 || state_id > COMPACT_AIR_BASE_STATUS_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid state_id {state_id}"
            ));
        }
        if flags & !COMPACT_AIR_BASE_STATUS_ROW_FLAG_DELETED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has unknown flags {flags}"
            ));
        }
        if province_id > COMPACT_SUPPLY_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid province_id {province_id}"
            ));
        }
        if deleted {
            if capacity != 0
                || building_level != 0
                || based_wing_count != 0
                || based_plane_count != 0
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: air base status deleted row {row_index} carries nonzero live fields"
                ));
            }
            continue;
        }
        if capacity > COMPACT_AIR_BASE_STATUS_MAX_CAPACITY {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid capacity {capacity}"
            ));
        }
        if building_level > COMPACT_AIR_BASE_STATUS_MAX_BUILDING_LEVEL {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid building_level {building_level}"
            ));
        }
        if based_wing_count > COMPACT_AIR_BASE_STATUS_MAX_WING_COUNT {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid based_wing_count {based_wing_count}"
            ));
        }
        if based_plane_count > COMPACT_AIR_BASE_STATUS_MAX_PLANES {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has invalid based_plane_count {based_plane_count}"
            ));
        }
        if building_level == 0 && capacity != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: air base status row {row_index} has capacity {capacity} with building_level 0"
            ));
        }
        capacity_sum = capacity_sum.saturating_add(capacity as usize);
        based_wing_sum = based_wing_sum.saturating_add(based_wing_count as usize);
        based_plane_sum = based_plane_sum.saturating_add(based_plane_count as usize);
    }
    add_compact_domain_data_unit(summary, 34, "air_base_status_capacity_sum", capacity_sum);
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_based_wing_count_sum",
        based_wing_sum,
    );
    add_compact_domain_data_unit(
        summary,
        34,
        "air_base_status_based_plane_count_sum",
        based_plane_sum,
    );
}

fn decode_compact_land_combats_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let combat_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let participant_count = read_u32_le(payload, 20) as usize;
    let ended_count = read_u32_le(payload, 24) as usize;
    let reserved = read_u32_le(payload, 28);
    add_decoder_status(summary, status_flags);
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data reserved field is nonzero"
        ));
    }
    let expected_len = 32usize
        .saturating_add(combat_count.saturating_mul(COMPACT_LAND_COMBAT_ROW_LEN))
        .saturating_add(participant_count.saturating_mul(COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if changed_rows > combat_count.saturating_add(ended_count) {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat changed_rows {changed_rows} > active+ended rows {}",
            combat_count.saturating_add(ended_count)
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&21) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(combat_count as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 21, "land_combat_rows", combat_count);
    add_compact_domain_data_unit(
        summary,
        21,
        "land_combat_participant_rows",
        participant_count,
    );
    add_compact_domain_data_unit(summary, 21, "land_combat_ended_rows", ended_count);
    let mut cursor = 32usize;
    let mut seen_combats = BTreeSet::new();
    let mut declared_participants = 0usize;
    let mut attacker_rows = 0usize;
    let mut defender_rows = 0usize;
    for row_index in 0..combat_count {
        let combat_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 12);
        let location = read_u32_le(payload, cursor + 16);
        let day = read_u32_le(payload, cursor + 20);
        let duration = read_u32_le(payload, cursor + 24);
        let attacker_count = read_u32_le(payload, cursor + 28) as usize;
        let defender_count = read_u32_le(payload, cursor + 32) as usize;
        let row_reserved = read_u32_le(payload, cursor + 36);
        cursor += COMPACT_LAND_COMBAT_ROW_LEN;

        if combat_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has zero combat_id"
            ));
        } else if !seen_combats.insert((combat_id, generation)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} duplicates combat {combat_id}:{generation}"
            ));
        }
        if flags & !0b1_1111 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) != 0 && !valid_compact_land_combat_province_id(location) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid location {location}"
            ));
        }
        if (flags & 1) == 0 && location != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has location without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && day > COMPACT_LAND_COMBAT_MAX_AGE_DAYS {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid day {day}"
            ));
        }
        if (flags & (1 << 1)) == 0 && day != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has day without flag"
            ));
        }
        if (flags & (1 << 2)) != 0 && duration > COMPACT_LAND_COMBAT_MAX_AGE_DAYS {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has invalid duration {duration}"
            ));
        }
        if (flags & (1 << 2)) == 0 && duration != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has duration without flag"
            ));
        }
        if (flags & (1 << 3)) == 0 && attacker_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has attackers without exact flag"
            ));
        }
        if (flags & (1 << 4)) == 0 && defender_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} has defenders without exact flag"
            ));
        }
        declared_participants =
            declared_participants.saturating_add(attacker_count.saturating_add(defender_count));
        attacker_rows = attacker_rows.saturating_add(attacker_count);
        defender_rows = defender_rows.saturating_add(defender_count);
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat row {row_index} reserved field is nonzero"
            ));
        }
    }
    add_compact_domain_data_unit(summary, 21, "land_combat_attacker_rows", attacker_rows);
    add_compact_domain_data_unit(summary, 21, "land_combat_defender_rows", defender_rows);
    if declared_participants != participant_count {
        summary.parse_errors.push(format!(
            "offset {offset}: land combat participant_count {participant_count} != declared {declared_participants}"
        ));
    }
    let mut seen_participants = BTreeSet::new();
    for row_index in 0..participant_count {
        let combat_id = read_u32_le(payload, cursor);
        let generation = read_u64_le(payload, cursor + 4);
        let side = read_u32_le(payload, cursor + 12);
        let division_id = read_u32_le(payload, cursor + 16);
        let row_reserved = read_u32_le(payload, cursor + 20);
        cursor += COMPACT_LAND_COMBAT_PARTICIPANT_ROW_LEN;
        if !seen_combats.contains(&(combat_id, generation)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} references unknown combat {combat_id}:{generation}"
            ));
        }
        if side != 1 && side != 2 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} has invalid side {side}"
            ));
        }
        if division_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} has zero division_id"
            ));
        }
        if !seen_participants.insert((combat_id, generation, side, division_id)) {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} duplicates participant"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: land combat participant row {row_index} reserved field is nonzero"
            ));
        }
    }
}

fn decode_compact_map_control_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 32 {
        summary.parse_errors.push(format!(
            "offset {offset}: map control data payload is {} byte(s), expected at least 32",
            payload.len()
        ));
        return;
    }
    let state_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let changed_state_rows = read_u32_le(payload, 24) as usize;
    let changed_province_rows = read_u32_le(payload, 28) as usize;
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    let expected_len = 32usize
        .saturating_add(state_count.saturating_mul(COMPACT_MAP_STATE_ROW_LEN))
        .saturating_add(province_count.saturating_mul(COMPACT_MAP_PROVINCE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: map control data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    // A FULL baseline carrying no rows of EITHER kind carries no measurement,
    // and the fold refuses it too -- before it clears its retained maps. It also
    // cannot have come from this recorder, whose topology walk refuses an array
    // that authenticates no province.
    //
    // Scoped twice: an empty DELTA is ordinary, and a STATE-ONLY snapshot is
    // still a measurement. This is a consistency rule with the producer, not the
    // 10 000-province content floor lane HJ removed from this decoder.
    if !delta_data && state_count == 0 && province_count == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map control full baseline carries no rows of either kind"
        ));
    }
    if changed_state_rows > state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_state_rows {changed_state_rows} > state_count {state_count}"
        ));
    }
    if changed_province_rows > province_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_province_rows {changed_province_rows} > province_count {province_count}"
        ));
    }
    if changed_rows != changed_state_rows.saturating_add(changed_province_rows) {
        summary.parse_errors.push(format!(
            "offset {offset}: map control changed_rows {changed_rows} != changed state+province rows {}",
            changed_state_rows.saturating_add(changed_province_rows)
        ));
    }
    if delta_data && changed_state_rows != state_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control delta changed_state_rows {changed_state_rows} != emitted state_count {state_count}"
        ));
    }
    if delta_data && changed_province_rows != province_count {
        summary.parse_errors.push(format!(
            "offset {offset}: map control delta changed_province_rows {changed_province_rows} != emitted province_count {province_count}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&22) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(state_count.saturating_add(province_count) as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 22, "map_control_state_rows", state_count);
    add_compact_domain_data_unit(summary, 22, "map_control_province_rows", province_count);
    add_compact_domain_data_unit(
        summary,
        22,
        "map_control_changed_state_rows",
        changed_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        22,
        "map_control_changed_province_rows",
        changed_province_rows,
    );

    let mut cursor = 32usize;
    let mut seen_states = BTreeSet::new();
    for row_index in 0..state_count {
        let state_id = read_u32_le(payload, cursor);
        let owner_tag_db_id = read_u32_le(payload, cursor + 4);
        let controller_tag_db_id = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        cursor += COMPACT_MAP_STATE_ROW_LEN;
        if state_id == 0 || state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid state_id {state_id}"
            ));
        } else if !seen_states.insert(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} duplicates state_id {state_id}"
            ));
        }
        if owner_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid owner tag {owner_tag_db_id}"
            ));
        }
        if controller_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has invalid controller tag {controller_tag_db_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && owner_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has owner without flag"
            ));
        }
        if (flags & 1) != 0 && owner_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has owner flag without owner"
            ));
        }
        if (flags & (1 << 1)) == 0 && controller_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has controller without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && controller_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control state row {row_index} has controller flag without controller"
            ));
        }
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let controller_tag_db_id = read_u32_le(payload, cursor + 4);
        let state_id = read_u32_le(payload, cursor + 8);
        let flags = read_u32_le(payload, cursor + 12);
        cursor += COMPACT_MAP_PROVINCE_ROW_LEN;
        if province_id == 0 || province_id > COMPACT_MAP_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid province_id {province_id}"
            ));
        } else if !seen_provinces.insert(province_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if controller_tag_db_id > COMPACT_MAP_MAX_TAG_DB_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid controller tag {controller_tag_db_id}"
            ));
        }
        if state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has invalid state_id {state_id}"
            ));
        }
        if !delta_data && state_id != 0 && !seen_states.contains(&state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} references unknown state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && controller_tag_db_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has controller without flag"
            ));
        }
        if (flags & 1) != 0 && controller_tag_db_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has controller flag without controller"
            ));
        }
        if (flags & (1 << 1)) == 0 && state_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has state without flag"
            ));
        }
        if (flags & (1 << 1)) != 0 && state_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map control province row {row_index} has state flag without state"
            ));
        }
    }
}

fn decode_compact_map_detail_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let state_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let observed_state_rows = read_u32_le(payload, 24) as usize;
    let observed_province_rows = read_u32_le(payload, 28) as usize;
    let resource_count = read_u32_le(payload, 32) as usize;
    let state_building_count = read_u32_le(payload, 36) as usize;
    let province_building_count = read_u32_le(payload, 40) as usize;
    let reserved = read_u32_le(payload, 44);
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;

    let expected_len = 48usize
        .saturating_add(state_count.saturating_mul(COMPACT_MAP_DETAIL_STATE_ROW_LEN))
        .saturating_add(province_count.saturating_mul(COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN))
        .saturating_add(resource_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN))
        .saturating_add(state_building_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN))
        .saturating_add(province_building_count.saturating_mul(COMPACT_MAP_DETAIL_ENTRY_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail reserved field is {reserved}"
        ));
    }
    if state_count > observed_state_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail state_count {state_count} > observed_state_rows {observed_state_rows}"
        ));
    }
    if province_count > observed_province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail province_count {province_count} > observed_province_rows {observed_province_rows}"
        ));
    }
    let emitted_rows = state_count.saturating_add(province_count);
    if changed_rows > emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail changed_rows {changed_rows} > emitted rows {emitted_rows}"
        ));
    }
    if delta_data && changed_rows != emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail delta changed_rows {changed_rows} != emitted rows {emitted_rows}"
        ));
    }
    if let Some(domain) = summary.per_domain.get_mut(&23) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(emitted_rows as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 23, "map_detail_state_rows", state_count);
    add_compact_domain_data_unit(summary, 23, "map_detail_province_rows", province_count);
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_observed_state_rows",
        observed_state_rows,
    );
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_observed_province_rows",
        observed_province_rows,
    );
    add_compact_domain_data_unit(summary, 23, "map_detail_resource_rows", resource_count);
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_state_building_rows",
        state_building_count,
    );
    add_compact_domain_data_unit(
        summary,
        23,
        "map_detail_province_building_rows",
        province_building_count,
    );

    let mut cursor = 48usize;
    let mut seen_states = BTreeSet::new();
    for row_index in 0..state_count {
        let state_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let resource_first = read_u32_le(payload, cursor + 8) as usize;
        let resource_len = read_u32_le(payload, cursor + 12) as usize;
        let building_first = read_u32_le(payload, cursor + 16) as usize;
        let building_len = read_u32_le(payload, cursor + 20) as usize;
        let row_reserved = read_u32_le(payload, cursor + 24);
        cursor += COMPACT_MAP_DETAIL_STATE_ROW_LEN;
        if state_id == 0 || state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} has invalid state_id {state_id}"
            ));
        } else if !seen_states.insert(state_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} duplicates state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} has unknown flags {flags}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail state row {row_index} reserved field is {row_reserved}"
            ));
        }
        validate_compact_index_range(
            offset,
            summary,
            "map detail state resource",
            row_index,
            flags & 1 != 0,
            resource_first,
            resource_len,
            resource_count,
        );
        validate_compact_index_range(
            offset,
            summary,
            "map detail state building",
            row_index,
            flags & (1 << 1) != 0,
            building_first,
            building_len,
            state_building_count,
        );
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let state_id = read_u32_le(payload, cursor + 4);
        let flags = read_u32_le(payload, cursor + 8);
        let building_first = read_u32_le(payload, cursor + 12) as usize;
        let building_len = read_u32_le(payload, cursor + 16) as usize;
        cursor += COMPACT_MAP_DETAIL_PROVINCE_ROW_LEN;
        if province_id == 0 || province_id > COMPACT_MAP_MAX_PROVINCE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has invalid province_id {province_id}"
            ));
        } else if !seen_provinces.insert(province_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if state_id > COMPACT_MAP_MAX_STATE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has invalid state_id {state_id}"
            ));
        }
        if flags & !0b11 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has unknown flags {flags}"
            ));
        }
        if (flags & 1) == 0 && state_id != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has state without flag"
            ));
        }
        if (flags & 1) != 0 && state_id == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail province row {row_index} has state flag without state"
            ));
        }
        validate_compact_index_range(
            offset,
            summary,
            "map detail province building",
            row_index,
            flags & (1 << 1) != 0,
            building_first,
            building_len,
            province_building_count,
        );
    }

    for row_index in 0..resource_count {
        let resource_id = read_u32_le(payload, cursor);
        let amount_raw = read_i32_le(payload, cursor + 4);
        let entry_reserved = read_u32_le(payload, cursor + 8);
        cursor += COMPACT_MAP_DETAIL_ENTRY_ROW_LEN;
        if resource_id == 0 || resource_id == u32::MAX {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} has invalid resource_id {resource_id}"
            ));
        }
        if !(0..=COMPACT_MAP_MAX_RESOURCE_AMOUNT_RAW).contains(&amount_raw) {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} has invalid amount_raw {amount_raw}"
            ));
        }
        if entry_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: map detail resource row {row_index} reserved field is {entry_reserved}"
            ));
        }
    }
    for row_index in 0..state_building_count {
        decode_compact_map_detail_building_entry(
            offset,
            payload,
            &mut cursor,
            summary,
            "state",
            row_index,
        );
    }
    for row_index in 0..province_building_count {
        decode_compact_map_detail_building_entry(
            offset,
            payload,
            &mut cursor,
            summary,
            "province",
            row_index,
        );
    }
}

fn decode_compact_weather_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    if payload.len() < 48 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather data payload is {} byte(s), expected at least 48",
            payload.len()
        ));
        return;
    }
    let region_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let province_count = read_u32_le(payload, 20) as usize;
    let observed_region_rows = read_u32_le(payload, 24) as usize;
    let observed_province_rows = read_u32_le(payload, 28) as usize;
    let scale_raw = read_u32_le(payload, 32);
    let active_region_count = read_u32_le(payload, 36) as usize;
    // Header words `+40` / `+44`: the two ENGINE precipitation ceilings the
    // producer discovered for this record, in the slots that used to be reserved
    // zeros. See `COMPACT_WEATHER_MAX_TEMPERATURE_RAW`'s neighbourhood for why
    // the constant they replaced is gone.
    let max_water_raw_word = read_u32_le(payload, 40);
    let max_snow_raw_word = read_u32_le(payload, 44);
    add_decoder_status(summary, status_flags);
    let delta_data = (status_flags & COMPACT_STATUS_DELTA_DATA) != 0;
    let failed = compact_contract::compact_status_is_failure(
        compact_contract::compact_status_for_global_interpretation(24, status_flags),
    );
    let emitted_rows = region_count.saturating_add(province_count);

    let expected_len = 48usize
        .saturating_add(region_count.saturating_mul(COMPACT_WEATHER_REGION_ROW_LEN))
        .saturating_add(province_count.saturating_mul(COMPACT_WEATHER_PROVINCE_ROW_LEN));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: weather data payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return;
    }
    if !failed
        && !(COMPACT_WEATHER_MIN_REGION_ROWS..=COMPACT_WEATHER_MAX_REGION_ROWS)
            .contains(&observed_region_rows)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: weather observed_region_rows {observed_region_rows} outside {COMPACT_WEATHER_MIN_REGION_ROWS}..={COMPACT_WEATHER_MAX_REGION_ROWS}"
        ));
    }
    if observed_province_rows != 0
        && !(COMPACT_WEATHER_MIN_PROVINCE_ROWS..=COMPACT_WEATHER_MAX_PROVINCE_ROWS)
            .contains(&observed_province_rows)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: weather observed_province_rows {observed_province_rows} outside {COMPACT_WEATHER_MIN_PROVINCE_ROWS}..={COMPACT_WEATHER_MAX_PROVINCE_ROWS}"
        ));
    }
    if !failed && !delta_data && region_count != observed_region_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather full snapshot region_count {region_count} != observed_region_rows {observed_region_rows}"
        ));
    }
    if !delta_data && observed_province_rows != 0 && province_count != observed_province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather full snapshot province_count {province_count} != observed_province_rows {observed_province_rows}"
        ));
    }
    if region_count > observed_region_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather region_count {region_count} > observed_region_rows {observed_region_rows}"
        ));
    }
    if observed_province_rows == 0 && province_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province_count {province_count} emitted with zero observed_province_rows"
        ));
    } else if province_count > observed_province_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather province_count {province_count} > observed_province_rows {observed_province_rows}"
        ));
    }
    if changed_rows > emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather changed_rows {changed_rows} > emitted_rows {emitted_rows}"
        ));
    }
    if delta_data && changed_rows != emitted_rows {
        summary.parse_errors.push(format!(
            "offset {offset}: weather delta changed_rows {changed_rows} != emitted_rows {emitted_rows}"
        ));
    }
    if scale_raw != COMPACT_FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: weather scale_raw {scale_raw} != {COMPACT_FIXED_SCALE_RAW}"
        ));
    }
    if active_region_count > region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather active_region_count {active_region_count} > region_count {region_count}"
        ));
    }
    // The ceilings are a presence-gated pair: `0, 0` is the producer saying no
    // weather root resolved, which is legal only when the record carries no
    // province rows to bound. Anything else zero, or a value the wire's signed
    // slot cannot hold, means rows arrived without the envelope they were
    // admitted against.
    let ceilings_absent = max_water_raw_word == 0 && max_snow_raw_word == 0;
    if !ceilings_absent && (max_water_raw_word == 0 || max_snow_raw_word == 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: weather precipitation ceilings are half-present (water {max_water_raw_word}, snow {max_snow_raw_word})"
        ));
    }
    if ceilings_absent && province_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather carries {province_count} province rows with no precipitation ceilings"
        ));
    }
    if max_water_raw_word > i32::MAX as u32 || max_snow_raw_word > i32::MAX as u32 {
        summary.parse_errors.push(format!(
            "offset {offset}: weather precipitation ceilings exceed the wire's signed slot (water {max_water_raw_word}, snow {max_snow_raw_word})"
        ));
    }
    let max_water_raw = max_water_raw_word.min(i32::MAX as u32) as i32;
    let max_snow_raw = max_snow_raw_word.min(i32::MAX as u32) as i32;
    if let Some(domain) = summary.per_domain.get_mut(&24) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(emitted_rows as u64);
        domain.data_changed_rows_total = domain
            .data_changed_rows_total
            .saturating_add(changed_rows as u64);
        domain.status_flags_or |= status_flags;
    }
    add_compact_domain_data_unit(summary, 24, "weather_region_rows", region_count);
    add_compact_domain_data_unit(
        summary,
        24,
        "weather_observed_region_rows",
        observed_region_rows,
    );
    add_compact_domain_data_unit(summary, 24, "weather_province_visual_rows", province_count);
    add_compact_domain_data_unit(
        summary,
        24,
        "weather_observed_province_rows",
        observed_province_rows,
    );
    add_compact_domain_data_unit(
        summary,
        24,
        "weather_active_region_rows",
        active_region_count,
    );

    let mut cursor = 48usize;
    let mut seen_regions = BTreeSet::new();
    let mut computed_active_regions = 0usize;
    for row_index in 0..region_count {
        let region_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let temperature_raw = read_i32_le(payload, cursor + 8);
        let phenomenon = read_u32_le(payload, cursor + 12);
        let next_weather_change_raw = read_u32_le(payload, cursor + 16);
        let active_modifier_count = read_u32_le(payload, cursor + 20);
        let intensity = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_WEATHER_REGION_ROW_LEN;
        // Bounded by the region table's OWN length, which the header states.
        // This was the constant 304, i.e. the vanilla count minus one.
        if region_id as usize >= observed_region_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid region_id {region_id}"
            ));
        } else if !seen_regions.insert(region_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} duplicates region_id {region_id}"
            ));
        }
        if flags & !COMPACT_WEATHER_REGION_KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has unknown flags {flags}"
            ));
        }
        let named_weather_count = [
            COMPACT_WEATHER_REGION_FLAG_RAIN_LIGHT,
            COMPACT_WEATHER_REGION_FLAG_RAIN_HEAVY,
            COMPACT_WEATHER_REGION_FLAG_SNOW,
            COMPACT_WEATHER_REGION_FLAG_BLIZZARD,
            COMPACT_WEATHER_REGION_FLAG_SANDSTORM,
        ]
        .into_iter()
        .filter(|flag| flags & *flag != 0)
        .count();
        if named_weather_count > 1 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} overlaps mutually-exclusive named weather flags"
            ));
        }
        let active_flag = (flags & (1 << 5)) != 0;
        let phenomenon_flag = (flags & (1 << 6)) != 0;
        if active_flag != phenomenon_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has mismatched active flags"
            ));
        }
        if phenomenon >= 1 {
            computed_active_regions = computed_active_regions.saturating_add(1);
            if !active_flag {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather row {row_index} has nonzero raw +0x78 activity candidate without active flag"
                ));
            }
        } else if active_flag {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has active flag without raw +0x78 activity candidate"
            ));
        }
        if phenomenon > COMPACT_WEATHER_MAX_RAW_ACTIVITY_CANDIDATE {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid raw +0x78 activity candidate {phenomenon}"
            ));
        }
        if next_weather_change_raw > i32::MAX as u32 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid signed transition expiry {next_weather_change_raw}"
            ));
        }
        if !(-COMPACT_WEATHER_MAX_TEMPERATURE_RAW..=COMPACT_WEATHER_MAX_TEMPERATURE_RAW)
            .contains(&temperature_raw)
        {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid temperature_raw {temperature_raw}"
            ));
        }
        if active_modifier_count > COMPACT_WEATHER_MAX_ACTIVE_MODIFIERS {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid active_modifier_count {active_modifier_count}"
            ));
        }
        if flags & COMPACT_WEATHER_REGION_FLAG_SANDSTORM != 0 {
            add_compact_domain_data_unit(summary, 24, "weather_region_sandstorm_rows", 1);
        }
        if intensity > u8::MAX as u32 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} has invalid intensity {intensity}"
            ));
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather row {row_index} reserved field is {row_reserved}"
            ));
        }
    }
    if computed_active_regions != active_region_count {
        summary.parse_errors.push(format!(
            "offset {offset}: weather active_region_count {active_region_count} != computed {computed_active_regions}"
        ));
    }

    let mut seen_provinces = BTreeSet::new();
    for row_index in 0..province_count {
        let province_id = read_u32_le(payload, cursor);
        let flags = read_u32_le(payload, cursor + 4);
        let temperature_raw = read_i32_le(payload, cursor + 8);
        let temperature_change_raw = read_i32_le(payload, cursor + 12);
        let water_raw = read_i32_le(payload, cursor + 16);
        let snow_raw = read_i32_le(payload, cursor + 20);
        let custom_modifier_count = read_u32_le(payload, cursor + 24);
        let row_reserved = read_u32_le(payload, cursor + 28);
        cursor += COMPACT_WEATHER_PROVINCE_ROW_LEN;

        if observed_province_rows != 0 && province_id as usize >= observed_province_rows {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has invalid province_id {province_id}"
            ));
        } else if !seen_provinces.insert(province_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} duplicates province_id {province_id}"
            ));
        }
        if flags & !COMPACT_WEATHER_PROVINCE_KNOWN_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has unknown flags {flags}"
            ));
        }
        if flags & COMPACT_WEATHER_PROVINCE_FLAG_STRUCT_VALIDATED == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} missing struct validation flag"
            ));
        }
        let temp_present = flags & COMPACT_WEATHER_PROVINCE_FLAG_TEMP_PRESENT != 0;
        let temp_change_present = flags & COMPACT_WEATHER_PROVINCE_FLAG_TEMP_CHANGE_PRESENT != 0;
        let water_present = flags & COMPACT_WEATHER_PROVINCE_FLAG_WATER_PRESENT != 0;
        let snow_present = flags & COMPACT_WEATHER_PROVINCE_FLAG_SNOW_PRESENT != 0;
        let mud_present = flags & COMPACT_WEATHER_PROVINCE_FLAG_MUD_PRESENT != 0;
        let mud_active = flags & COMPACT_WEATHER_PROVINCE_FLAG_MUD_ACTIVE != 0;
        let custom_present =
            flags & COMPACT_WEATHER_PROVINCE_FLAG_CUSTOM_MODIFIER_COUNT_PRESENT != 0;
        let map_recolor_driver = flags & COMPACT_WEATHER_PROVINCE_FLAG_MAP_RECOLOR_DRIVER != 0;
        let dynamic_lut_driver = flags & COMPACT_WEATHER_PROVINCE_FLAG_DYNAMIC_LUT_DRIVER != 0;

        if mud_active && !mud_present {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has mud_active without mud_present"
            ));
        }
        if map_recolor_driver != (temp_present || water_present || snow_present) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has inconsistent map recolor flag"
            ));
        }
        if dynamic_lut_driver != (snow_present || mud_present) {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has inconsistent dynamic LUT flag"
            ));
        }
        if temp_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_temp_rows", 1);
            if !(-COMPACT_WEATHER_MAX_TEMPERATURE_RAW..=COMPACT_WEATHER_MAX_TEMPERATURE_RAW)
                .contains(&temperature_raw)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather province row {row_index} has invalid temperature_raw {temperature_raw}"
                ));
            }
        } else if temperature_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has temperature_raw without presence flag"
            ));
        }
        if temp_change_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_temp_change_rows", 1);
            if !(-COMPACT_WEATHER_MAX_TEMP_CHANGE_RAW..=COMPACT_WEATHER_MAX_TEMP_CHANGE_RAW)
                .contains(&temperature_change_raw)
            {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather province row {row_index} has invalid temperature_change_raw {temperature_change_raw}"
                ));
            }
        } else if temperature_change_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has temperature_change_raw without presence flag"
            ));
        }
        if water_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_water_rows", 1);
            if !(0..=max_water_raw).contains(&water_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather province row {row_index} has invalid water_raw {water_raw}"
                ));
            }
        } else if water_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has water_raw without presence flag"
            ));
        }
        if snow_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_snow_rows", 1);
            if !(0..=max_snow_raw).contains(&snow_raw) {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather province row {row_index} has invalid snow_raw {snow_raw}"
                ));
            }
        } else if snow_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has snow_raw without presence flag"
            ));
        }
        if mud_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_mud_measured_rows", 1);
        }
        if mud_active {
            add_compact_domain_data_unit(summary, 24, "weather_province_mud_active_rows", 1);
        }
        if custom_present {
            add_compact_domain_data_unit(summary, 24, "weather_province_custom_modifier_rows", 1);
            if custom_modifier_count > COMPACT_WEATHER_MAX_ACTIVE_MODIFIERS {
                summary.parse_errors.push(format!(
                    "offset {offset}: weather province row {row_index} has invalid custom_modifier_count {custom_modifier_count}"
                ));
            }
        } else if custom_modifier_count != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} has custom_modifier_count without presence flag"
            ));
        }
        if map_recolor_driver {
            add_compact_domain_data_unit(
                summary,
                24,
                "weather_province_map_recolor_driver_rows",
                1,
            );
        }
        if dynamic_lut_driver {
            add_compact_domain_data_unit(
                summary,
                24,
                "weather_province_dynamic_lut_driver_rows",
                1,
            );
        }
        if row_reserved != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: weather province row {row_index} reserved field is {row_reserved}"
            ));
        }
    }
}

fn validate_compact_index_range(
    offset: usize,
    summary: &mut DomainDataDecodeSummary,
    label: &str,
    row_index: usize,
    present: bool,
    first: usize,
    len: usize,
    total: usize,
) {
    if !present {
        if first != 0 || len != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: {label} row {row_index} has range without presence flag"
            ));
        }
        return;
    }
    if first.saturating_add(len) > total {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} row {row_index} range {first}+{len} exceeds {total}"
        ));
    }
}

fn decode_compact_map_detail_building_entry(
    offset: usize,
    payload: &[u8],
    cursor: &mut usize,
    summary: &mut DomainDataDecodeSummary,
    scope: &str,
    row_index: usize,
) {
    let type_id = read_u32_le(payload, *cursor);
    let level = read_u32_le(payload, *cursor + 4);
    let entry_reserved = read_u32_le(payload, *cursor + 8);
    *cursor += COMPACT_MAP_DETAIL_ENTRY_ROW_LEN;
    if type_id == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} has zero type_id"
        ));
    }
    if level > COMPACT_MAP_MAX_BUILDING_LEVEL {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} has invalid level {level}"
        ));
    }
    if entry_reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: map detail {scope} building row {row_index} reserved field is {entry_reserved}"
        ));
    }
}

fn valid_compact_land_combat_province_id(value: u32) -> bool {
    value > 0 && value <= COMPACT_LAND_COMBAT_MAX_PROVINCE_ID
}

fn decode_compact_pov_visibility_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match visibility94::PovVisibilitySnapshot::decode(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: POV province visibility FOW1 payload invalid: {error}"
            ));
            return;
        }
    };
    if let Some(domain) = summary.per_domain.get_mut(&visibility94::DOMAIN_ID) {
        domain.data_rows_total = domain
            .data_rows_total
            .saturating_add(decoded.province_levels.len() as u64);
    }
    add_compact_domain_data_unit(
        summary,
        visibility94::DOMAIN_ID,
        "pov_province_visibility_slots",
        decoded.province_levels.len(),
    );
    add_compact_domain_data_unit(
        summary,
        visibility94::DOMAIN_ID,
        "pov_province_visibility_complete_frames",
        1,
    );
    // How many of the frames delivered carry the hour their PRODUCER observed
    // at, rather than only the hour the collector drained them in. D94 retains
    // its frame and re-serves it until the producer replaces it, so without this
    // word a consumer cannot tell fog standing since an earlier hour from fog
    // rebuilt in the labelled one -- and a capture whose coverage of this fact
    // is partial has to be able to say so.
    add_compact_domain_data_unit(
        summary,
        visibility94::DOMAIN_ID,
        "pov_province_visibility_producer_hour_frames",
        usize::from(decoded.producer_hour_raw.is_some()),
    );
}

fn decode_compact_pov_spotted_units_data_payload(
    offset: usize,
    payload: &[u8],
    summary: &mut DomainDataDecodeSummary,
) {
    let decoded = match spotted95::SpottedUnitsSnapshot::decode(payload) {
        Ok(decoded) => decoded,
        Err(error) => {
            summary.parse_errors.push(format!(
                "offset {offset}: POV spotted-units SPU1 payload invalid: {error}"
            ));
            return;
        }
    };
    let row_count = decoded.land.len().saturating_add(decoded.taskforces.len());
    if let Some(domain) = summary.per_domain.get_mut(&spotted95::DOMAIN_ID) {
        domain.data_rows_total = domain.data_rows_total.saturating_add(row_count as u64);
    }
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_land_rows",
        decoded.land.len(),
    );
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_taskforce_rows",
        decoded.taskforces.len(),
    );
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_land_complete_frames",
        usize::from(decoded.land_frame_sequence.is_some()),
    );
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_taskforce_complete_frames",
        usize::from(decoded.taskforce_frame_sequence.is_some()),
    );
    // How many of the frames delivered carry the hour their PRODUCER observed
    // at, rather than only the hour the collector drained them in. A frame is
    // retained and re-served until its producer replaces it, so without this
    // word a consumer cannot tell state standing since an earlier hour from
    // state observed in the labelled one -- and a capture whose coverage of
    // this fact is partial has to be able to say so.
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_land_producer_hour_frames",
        usize::from(decoded.land_frame_hour_raw.is_some()),
    );
    add_compact_domain_data_unit(
        summary,
        spotted95::DOMAIN_ID,
        "pov_spotted_taskforce_producer_hour_frames",
        usize::from(decoded.taskforce_frame_hour_raw.is_some()),
    );
}

pub fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn require_compact_remaining(
    bytes: &[u8],
    cursor: usize,
    needed: usize,
    label: &str,
) -> Result<(), String> {
    if cursor.saturating_add(needed) > bytes.len() {
        Err(format!(
            "{label} overruns payload at cursor {cursor}: need {needed}, remaining {}",
            bytes.len().saturating_sub(cursor)
        ))
    } else {
        Ok(())
    }
}

fn skip_compact_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    require_compact_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_compact_remaining(bytes, *cursor, len, label)?;
    let start = *cursor;
    *cursor += len;
    Ok(start)
}

fn skip_compact_string_len(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<usize, String> {
    require_compact_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_compact_remaining(bytes, *cursor, len, label)?;
    *cursor += len;
    Ok(len)
}

fn read_compact_string(bytes: &[u8], cursor: &mut usize, label: &str) -> Result<String, String> {
    require_compact_remaining(bytes, *cursor, 2, label)?;
    let len = read_u16_le(bytes, *cursor) as usize;
    *cursor += 2;
    require_compact_remaining(bytes, *cursor, len, label)?;
    let start = *cursor;
    let end = start + len;
    *cursor = end;
    std::str::from_utf8(&bytes[start..end])
        .map(|value| value.to_string())
        .map_err(|err| format!("{label}: invalid utf-8: {err}"))
}

pub fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_i32_le(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

pub fn read_u64_le(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn read_i64_le(bytes: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
pub fn summarize_domain_data_payload(
    domain_id: u16,
    payload: &[u8],
    mut context: DomainDataContext<'_>,
) -> Result<DomainDataAuthority, DomainDataError> {
    let mut summary = DomainDataDecodeSummary::default();
    summary.domain_id = domain_id;
    summary
        .per_domain
        .insert(domain_id, DomainDataCounts::default());
    if let Some(existing) = context.session_header_identity.as_deref() {
        summary.session_header_identity = existing.clone();
    }

    let offset = context.stream_offset;
    match domain_id {
        10 => decode_compact_fuel_data_payload(offset, payload, &mut summary),
        11 => decode_compact_production_data_payload(offset, payload, &mut summary),
        12 => decode_compact_focus_data_payload(offset, payload, &mut summary),
        13 => decode_compact_tech_data_payload(offset, payload, &mut summary),
        14 => decode_compact_politics_data_payload(offset, payload, &mut summary),
        15 => decode_compact_politics_residuals_data_payload(offset, payload, &mut summary),
        16 => decode_compact_decisions_data_payload(offset, payload, &mut summary),
        17 => decode_compact_diplomacy_data_payload(offset, payload, &mut summary),
        18 => decode_compact_country_identity_data_payload(offset, payload, &mut summary),
        20 => decode_compact_divisions_data_payload(offset, payload, &mut summary),
        21 => decode_compact_land_combats_data_payload(offset, payload, &mut summary),
        22 => decode_compact_map_control_data_payload(offset, payload, &mut summary),
        23 => decode_compact_map_detail_data_payload(offset, payload, &mut summary),
        24 => decode_compact_weather_data_payload(offset, payload, &mut summary),
        26 => decode_compact_division_equipment_data_payload(offset, payload, &mut summary),
        27 => decode_compact_army_orders_data_payload(offset, payload, &mut summary),
        28 => decode_compact_oob_command_data_payload(offset, payload, &mut summary),
        29 => decode_compact_ships_data_payload(offset, payload, &mut summary),
        30 => decode_compact_air_wings_data_payload(offset, payload, &mut summary),
        31 => decode_compact_air_region_activity_data_payload(offset, payload, &mut summary),
        32 => decode_compact_air_wing_loadouts_data_payload(offset, payload, &mut summary),
        33 => decode_compact_air_variant_defs_data_payload(offset, payload, &mut summary),
        34 => decode_compact_air_base_status_data_payload(offset, payload, &mut summary),
        35 => decode_compact_air_variant_stats_data_payload(offset, payload, &mut summary),
        36 => decode_compact_air_variant_reserves_data_payload(offset, payload, &mut summary),
        37 => decode_compact_stockpile_variant_defs_data_payload(offset, payload, &mut summary),
        38 => decode_compact_air_superiority_inputs_data_payload(offset, payload, &mut summary),
        39 => decode_compact_air_variant_upgrades_data_payload(offset, payload, &mut summary),
        40 => decode_compact_stockpile_variant_upgrades_data_payload(offset, payload, &mut summary),
        41 => decode_compact_projects_data_payload(offset, payload, &mut summary),
        42 => decode_compact_air_combat_losses_live_data_payload(offset, payload, &mut summary),
        43 => decode_compact_market_trade_data_payload(offset, payload, &mut summary),
        44 => decode_compact_supply_logistics_attrition_data_payload(offset, payload, &mut summary),
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE => {
            decode_compact_frd_depth_data_payload(
                offset,
                payload,
                context.record_flags,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES => {
            decode_compact_air_variant_archetypes_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE => {
            decode_compact_characters_leaders_cabinet_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_CONSTRUCTION_BUILDINGS_STATE_DAMAGE_LIVE => {
            decode_compact_construction_buildings_state_damage_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_NAVAL_COMBAT_LIFECYCLE_LIVE => {
            decode_compact_naval_combat_lifecycle_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_NAVAL_COMBAT_MEMBERS_LIVE => {
            decode_compact_naval_combat_members_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_SCRIPTED_STATE_FLAGS_VARIABLES_LIVE => {
            decode_compact_scripted_state_flags_variables_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_OCCUPATION_RESISTANCE_COMPLIANCE_LIVE => {
            decode_compact_occupation_resistance_compliance_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE => {
            decode_compact_diplomacy_intel_operatives_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_ARMY_BATTLEPLANS_DEPTH_LIVE => {
            decode_compact_army_battleplans_depth_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_LAND_COMBAT_DETAIL_EVENTS_LIVE => {
            decode_compact_land_combat_detail_events_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_NAVAL_MISSIONS_STATUS_LIVE => {
            decode_compact_naval_missions_status_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_SESSION_MULTIPLAYER_METADATA_LIVE => {
            decode_compact_session_multiplayer_metadata_live_data_payload(
                offset,
                context.sequence,
                context.header_hour_raw,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_CAPITULATION_WAR_STATE_LIVE => {
            decode_compact_capitulation_war_state_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_AIR_LOSS_STATS_LIVE => {
            decode_compact_air_loss_stats_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_HUB_MOTORIZATION_LIVE => {
            decode_compact_hub_motorization_live_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_COUNTRY_CORE_SCALARS_LIVE => {
            decode_compact_country_core_scalars_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_DELAYED_EVENTS_LIVE => {
            decode_compact_delayed_events_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_SPECIAL_PROJECT_PROGRAMS_LIVE => {
            decode_compact_special_project_programs_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_OCCUPATION_STATE_EXTRAS_LIVE => {
            decode_compact_occupation_state_extras_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_COUNTRY_DYNAMIC_MODIFIERS_LIVE => {
            decode_compact_country_dynamic_modifiers_data_payload(
                offset,
                payload,
                context.record_flags,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_SESSION_HEADER_LIVE => {
            decode_compact_session_header_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_TECH_SHARING_GROUPS_LIVE => {
            decode_compact_tech_sharing_groups_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE => {
            decode_compact_sunk_convoy_history_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE => {
            decode_compact_rail_replay_state_data_payload(offset, payload, &mut summary)
        }
        dynamic71::DOMAIN_ID => {
            decode_compact_dynamic_collection_trace_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        tooltip72::DOMAIN_ID => decode_compact_resolved_tooltip_instances_live_data_payload(
            offset,
            payload,
            &mut summary,
        ),
        75 => decode_compact_fired_event_popups_data_payload(offset, payload, &mut summary),
        78 => decode_compact_design_catalog_context_data_payload(offset, payload, &mut summary),
        hud79::DOMAIN_ID => {
            decode_compact_hud_surface_activation_live_data_payload(offset, payload, &mut summary)
        }
        character82::DOMAIN_ID => {
            decode_compact_character_presentation_data_payload(offset, payload, &mut summary)
        }
        scripted84::DOMAIN_ID => {
            decode_compact_scripted_gui_runtime_data_payload(offset, payload, &mut summary)
        }
        nuke83::DOMAIN_ID => {
            decode_compact_nuke_stockpile_data_payload(offset, payload, &mut summary)
        }
        selected85::DOMAIN_ID => {
            decode_compact_selected_designer_identity_data_payload(offset, payload, &mut summary)
        }
        peace97::DOMAIN_ID => {
            decode_compact_peace_conference_data_payload(offset, payload, &mut summary)
        }
        theatre98::DOMAIN_ID => {
            decode_compact_theatre_selector_data_payload(offset, payload, &mut summary)
        }
        sunk86::DOMAIN_ID => {
            decode_compact_sunk_ship_history_data_payload(offset, payload, &mut summary)
        }
        decision87::DOMAIN_ID => {
            decode_compact_decision_panel_data_payload(offset, payload, &mut summary)
        }
        abilities88::DOMAIN_ID => {
            decode_compact_active_command_abilities_data_payload(offset, payload, &mut summary)
        }
        focus89::DOMAIN_ID => {
            decode_compact_national_focus_presentation_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE => {
            decode_compact_convoy_transport_attack_live_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_RAID_INSTANCES_LIVE => {
            decode_compact_raid_instances_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE => {
            decode_compact_country_preferred_tactic_data_payload(offset, payload, &mut summary)
        }
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE => {
            decode_compact_country_preferred_tactic_weight_data_payload(
                offset,
                payload,
                &mut summary,
            )
        }
        COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE => {
            decode_compact_country_army_upgrade_data_payload(offset, payload, &mut summary)
        }
        visibility94::DOMAIN_ID => {
            decode_compact_pov_visibility_data_payload(offset, payload, &mut summary)
        }
        spotted95::DOMAIN_ID => {
            decode_compact_pov_spotted_units_data_payload(offset, payload, &mut summary)
        }
        other => summary.parse_errors.push(format!(
            "offset {offset}: unsupported domain data payload for domain {other}"
        )),
    }

    reject_unassigned_status_bits(&mut summary, domain_id, offset);

    if domain_id == COMPACT_DOMAIN_SESSION_HEADER_LIVE {
        if let Some(target) = context.session_header_identity.as_deref_mut() {
            *target = summary.session_header_identity.clone();
        }
    }

    let domain = summary.per_domain.remove(&domain_id).unwrap_or_default();
    let authority = DomainDataAuthority {
        status_flags: summary.raw_status_flags_or,
        emitted_rows: domain.data_rows_total,
        changed_rows: domain.data_changed_rows_total,
        data_units: domain.data_units_total,
        problems: domain.problems,
        auxiliary: DomainDataAuxiliary {
            domain57_frame: summary.domain57_frames.pop(),
            domain68_session_header_identity: (domain_id == COMPACT_DOMAIN_SESSION_HEADER_LIVE)
                .then_some(summary.session_header_identity),
        },
    };
    if summary.parse_errors.is_empty() {
        Ok(authority)
    } else {
        Err(DomainDataError {
            errors: summary.parse_errors,
            partial_authority: authority,
        })
    }
}

/// One exact runtime token whose string bytes are eligible for the static
/// catalogue join. Numeric-only runtime ids are deliberately absent from this
/// type: callers cannot accidentally serialize them as guessed strings.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StaticJoinToken {
    pub kind: &'static str,
    pub token: String,
}

/// Validate a DATA payload with the canonical decoder, then extract only the
/// string fields approved for the post-capture static join. Failure-class
/// payloads never contribute tokens, even if they happen to contain residual
/// bytes. The returned set is deterministic and duplicate-free.
pub fn collect_static_join_tokens(
    domain_id: u16,
    payload: &[u8],
    context: DomainDataContext<'_>,
) -> Result<BTreeSet<StaticJoinToken>, DomainDataError> {
    let record_flags = context.record_flags;
    let authority = summarize_domain_data_payload(domain_id, payload, context)?;
    if authority
        .status_flags
        .is_some_and(compact_contract::compact_status_is_failure)
    {
        return Ok(BTreeSet::new());
    }

    let rows = match domain_id {
        14 => collect_politics_advisor_tokens(payload),
        15 => collect_politics_residual_advisor_tokens(payload),
        16 => collect_timed_decision_tokens(payload),
        29 => collect_ship_definition_tokens(payload),
        COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE => {
            collect_frd_decision_tokens(payload, record_flags)
        }
        COMPACT_DOMAIN_AIR_VARIANT_ARCHETYPES => collect_air_archetype_tokens(payload),
        COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE => collect_domain47_advisor_tokens(payload),
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_LIVE => {
            collect_country_preferred_tactic_tokens(payload)
        }
        COMPACT_DOMAIN_COUNTRY_PREFERRED_TACTIC_WEIGHT_LIVE => {
            collect_country_preferred_tactic_weight_tokens(payload)
        }
        _ => Ok(Vec::new()),
    }
    .map_err(|error| DomainDataError {
        errors: vec![error],
        partial_authority: authority,
    })?;
    Ok(rows.into_iter().collect())
}

fn collect_country_preferred_tactic_tokens(
    payload: &[u8],
) -> Result<Vec<StaticJoinToken>, String> {
    let decoded = preferred92::decode_payload(payload)?;
    Ok(decoded
        .rows
        .into_iter()
        .filter_map(|row| {
            row.tactic_id.map(|token| StaticJoinToken {
                kind: "localisation",
                token,
            })
        })
        .collect())
}

fn collect_country_preferred_tactic_weight_tokens(
    payload: &[u8],
) -> Result<Vec<StaticJoinToken>, String> {
    let decoded = weight96::decode_payload(payload)?;
    Ok(decoded
        .rows
        .into_iter()
        .map(|row| StaticJoinToken {
            kind: "localisation",
            token: row.tactic_id,
        })
        .collect())
}

fn collect_politics_advisor_tokens(payload: &[u8]) -> Result<Vec<StaticJoinToken>, String> {
    let country_count = read_u32_le(payload, 8) as usize;
    let popularity_count = read_u32_le(payload, 24) as usize;
    let law_count = read_u32_le(payload, 28) as usize;
    let advisor_count = read_u32_le(payload, 32) as usize;
    let spirit_count = read_u32_le(payload, 36) as usize;
    let mut cursor = 40usize;
    for _ in 0..country_count {
        cursor = cursor.saturating_add(28);
        for _ in 0..4 {
            let _ = read_compact_string(payload, &mut cursor, "politics country token")?;
        }
    }
    for _ in 0..popularity_count {
        cursor = cursor.saturating_add(4);
        let _ = read_compact_string(payload, &mut cursor, "politics popularity token")?;
        cursor = cursor.saturating_add(4);
    }
    for _ in 0..law_count {
        cursor = cursor.saturating_add(4);
        let _ = read_compact_string(payload, &mut cursor, "politics law group")?;
        let _ = read_compact_string(payload, &mut cursor, "politics law token")?;
    }
    let mut out = Vec::new();
    for _ in 0..advisor_count {
        cursor = cursor.saturating_add(4);
        let _ = read_compact_string(payload, &mut cursor, "politics advisor group")?;
        push_static_join_token(
            &mut out,
            "idea",
            read_compact_string(payload, &mut cursor, "politics advisor token")?,
        );
    }
    for _ in 0..spirit_count {
        cursor = cursor.saturating_add(4);
        let _ = read_compact_string(payload, &mut cursor, "politics spirit token")?;
    }
    Ok(out)
}

fn collect_politics_residual_advisor_tokens(
    payload: &[u8],
) -> Result<Vec<StaticJoinToken>, String> {
    let country_count = read_u32_le(payload, 8) as usize;
    let advisor_count = read_u32_le(payload, 20) as usize;
    let mut cursor = 24usize;
    for _ in 0..country_count {
        cursor = cursor.saturating_add(16);
        let _ = read_compact_string(payload, &mut cursor, "last election")?;
    }
    let mut out = Vec::new();
    for _ in 0..advisor_count {
        cursor = cursor.saturating_add(4);
        push_static_join_token(
            &mut out,
            "idea",
            read_compact_string(payload, &mut cursor, "advisor idea token")?,
        );
        for label in ["advisor slot", "advisor ledger", "advisor character"] {
            let _ = read_compact_string(payload, &mut cursor, label)?;
        }
    }
    Ok(out)
}

fn collect_timed_decision_tokens(payload: &[u8]) -> Result<Vec<StaticJoinToken>, String> {
    let country_count = read_u32_le(payload, 8) as usize;
    let timed_count = read_u32_le(payload, 20) as usize;
    let mut cursor = 24usize.saturating_add(country_count.saturating_mul(32));
    let mut out = Vec::new();
    for _ in 0..timed_count {
        cursor = cursor.saturating_add(12);
        push_static_join_token(
            &mut out,
            "decision",
            read_compact_string(payload, &mut cursor, "timed decision token")?,
        );
    }
    Ok(out)
}

fn collect_ship_definition_tokens(payload: &[u8]) -> Result<Vec<StaticJoinToken>, String> {
    let row_count = read_u32_le(payload, 8) as usize;
    let mut cursor = 40usize;
    let mut out = Vec::new();
    for _ in 0..row_count {
        cursor = cursor.saturating_add(COMPACT_SHIP_ROW_BASE_LEN);
        push_static_join_token(
            &mut out,
            "ship_hull",
            read_compact_string(payload, &mut cursor, "ship definition token")?,
        );
    }
    Ok(out)
}

fn collect_frd_decision_tokens(
    payload: &[u8],
    record_flags: u32,
) -> Result<Vec<StaticJoinToken>, String> {
    let mut summary = DomainDataDecodeSummary::default();
    let header = decode_compact_frd_depth_header(0, payload, record_flags, &mut summary)
        .ok_or_else(|| summary.parse_errors.join("; "))?;
    let mut cursor = header.header_len;
    for _ in 0..header.focus_count {
        cursor = cursor.saturating_add(36);
        for label in [
            "focus from token",
            "focus to token",
            "focus completed token",
        ] {
            let _ = read_compact_string(payload, &mut cursor, label)?;
        }
    }
    cursor = cursor.saturating_add(
        header
            .research_count
            .saturating_mul(COMPACT_FRD_RESEARCH_ETA_ROW_LEN),
    );
    cursor = cursor.saturating_add(
        header
            .decision_status_count
            .saturating_mul(COMPACT_FRD_DECISION_STATUS_ROW_LEN),
    );
    let mut out = Vec::new();
    for _ in 0..header.decision_event_count {
        cursor = cursor.saturating_add(28);
        push_static_join_token(
            &mut out,
            "decision",
            read_compact_string(payload, &mut cursor, "decision event token")?,
        );
    }
    for _ in 0..header.decision_target_count {
        cursor = cursor.saturating_add(COMPACT_FRD_DECISION_TARGET_ROW_FIXED_LEN);
        push_static_join_token(
            &mut out,
            "decision",
            read_compact_string(payload, &mut cursor, "decision target token")?,
        );
    }
    Ok(out)
}

fn collect_air_archetype_tokens(payload: &[u8]) -> Result<Vec<StaticJoinToken>, String> {
    use air_wings_variants_lifecycle_live_lane::domain46_air_variant_archetypes_live as air46;

    let row_count = read_u32_le(payload, 8) as usize;
    let rows_start = air46::HEADER_LEN;
    let mut cursor = rows_start.saturating_add(row_count.saturating_mul(air46::ROW_LEN));
    let mut out = Vec::new();
    for row_index in 0..row_count {
        let base = rows_start.saturating_add(row_index.saturating_mul(air46::ROW_LEN));
        let source_mask = read_u32_le(payload, base + 24);
        let archetype = read_compact_string(payload, &mut cursor, "air archetype token")?;
        let _ = read_compact_string(payload, &mut cursor, "air base airframe token")?;
        let _ = read_compact_string(payload, &mut cursor, "air role token")?;
        if source_mask & air46::SOURCE_LIVE_TOKEN_DICTIONARY != 0 {
            push_static_join_token(&mut out, "equipment", archetype);
        }
    }
    Ok(out)
}

fn collect_domain47_advisor_tokens(payload: &[u8]) -> Result<Vec<StaticJoinToken>, String> {
    let decoded = domain47::decode_payload_rows(0, payload)
        .map_err(|errors| format!("domain 47 token decode failed: {}", errors.join("; ")))?;
    let mut out = Vec::new();
    for row in decoded.rows {
        match row {
            domain47::DecodedDomain47Row::AdvisorRoster { idea_token, .. }
            | domain47::DecodedDomain47Row::AppointedAdvisor { idea_token, .. } => {
                push_static_join_token(&mut out, "idea", idea_token);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn push_static_join_token(out: &mut Vec<StaticJoinToken>, kind: &'static str, token: String) {
    if !token.is_empty() {
        out.push(StaticJoinToken { kind, token });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32(out: &mut Vec<u8>, value: u32) {
        out.extend_from_slice(&value.to_le_bytes());
    }

    fn push_string(out: &mut Vec<u8>, value: &str) {
        out.extend_from_slice(&(value.len() as u16).to_le_bytes());
        out.extend_from_slice(value.as_bytes());
    }

    /// A standalone `MTRD` trailer over two witnesses and no rows.
    fn mtrd_trailer_with_witnesses(version: u16, witnesses: [u32; 2], incomplete: &[u32]) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_MAGIC);
        payload.extend_from_slice(&version.to_le_bytes());
        payload
            .extend_from_slice(&(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_HEADER_LEN as u16)
                .to_le_bytes());
        payload.extend_from_slice(&(COMPACT_MARKET_TRADE_CLASSIC_DEAL_ROW_LEN as u16).to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        push_u32(&mut payload, 0);
        push_u32(&mut payload, 2);
        push_u32(&mut payload, COMPACT_FIXED_SCALE_RAW);
        push_u32(&mut payload, 2);
        push_u32(&mut payload, incomplete.len() as u32);
        push_u32(&mut payload, witnesses[0]);
        push_u32(&mut payload, witnesses[1]);
        for owner in incomplete {
            push_u32(&mut payload, *owner);
        }
        payload
    }

    fn mtrd_trailer(version: u16, incomplete: &[u32]) -> Vec<u8> {
        mtrd_trailer_with_witnesses(version, [34, 67], incomplete)
    }

    /// This decoder must refuse every `MTRD` v3 incomplete block the FOLD
    /// refuses, or the two disagree about one grammar.
    ///
    /// The decoder is structural — it advances a cursor and counts units, and
    /// does not restate the fold's row semantics. But the incomplete block is a
    /// statement about which owners are exact. Admitting `[99]` where 99 is not
    /// a witness, or `[67, 34]` out of order, would report the payload
    /// decodable and authoritative while `world_fold` rejects the whole D43
    /// record and every family in it.
    ///
    /// The witness list is checked for v3 too, and NOT only when the incomplete
    /// block is non-empty: a duplicate witness collapses in the set and lets the
    /// duplicate pass the subset check, so the block's meaning depends on the
    /// list being well formed. v1/v2 witness content stays uninspected here,
    /// exactly as it always was.
    ///
    /// What production change makes this fail: removing the ascending-witness
    /// check, removing the witness validity check, re-gating either on a
    /// non-empty incomplete block, or widening the header's version whitelist
    /// without them.
    ///
    /// MUTATION RUN (`--release`): deleted the `if per_owner { .. }` block,
    /// leaving the header/length checks alone -- which is the state review found
    /// this decoder in.
    /// OBSERVED: FAILED -- "an incomplete owner that is not a witness is refused
    /// here exactly as the fold refuses it: []"
    ///
    /// MUTATION RUN (`--release`): re-gated the block on
    /// `per_owner && incomplete_count > 0`, which is the narrower form the first
    /// round of this fix shipped.
    /// OBSERVED: FAILED -- "a duplicate witness with no incomplete owner is
    /// refused here exactly as the fold refuses it: []"
    ///
    /// MUTATION RUN (`--release`): dropped `|| !witnesses.insert(tag_db_id)` from
    /// the witness check, keeping the nonzero/range arms.
    /// OBSERVED: FAILED -- "a duplicate witness is refused here exactly as the
    /// fold refuses it: []"
    #[test]
    fn mtrd_v3_incomplete_block_is_refused_here_exactly_as_the_fold_refuses_it() {
        let decode = |version: u16, incomplete: &[u32]| {
            let payload = mtrd_trailer(version, incomplete);
            let mut summary = DomainDataDecodeSummary::default();
            let end = decode_compact_market_trade_classic_deal_trailer(0, &payload, 0, &mut summary);
            (end, summary.parse_errors)
        };

        let (end, errors) = decode(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION, &[34]);
        assert!(errors.is_empty(), "a well-formed v3 block decodes: {errors:?}");
        assert_eq!(end, 32 + 2 * 4 + 4, "and the cursor clears the whole trailer");

        let (_, errors) = decode(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION, &[99]);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("is not an ascending witness")),
            "an incomplete owner that is not a witness is refused here exactly as \
             the fold refuses it: {errors:?}"
        );

        let (_, errors) = decode(
            COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION,
            &[67, 34],
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("is not an ascending witness")),
            "and so is a descending one: {errors:?}"
        );

        let (_, errors) = decode(
            COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION,
            &[34, 34],
        );
        assert!(
            errors
                .iter()
                .any(|error| error.contains("is not an ascending witness")),
            "and a repeated one: {errors:?}"
        );

        // The version word still decides the grammar here too: a v3 body under a
        // v2 label puts a non-zero count in v2's reserved word.
        let (_, errors) = decode(COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_VERSION, &[34]);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("header shape is invalid")),
            "a v3 body labelled v2 is refused: {errors:?}"
        );

        // A v3 witness list must itself be well formed, or the incomplete block
        // has nothing meaningful to be a subset OF: duplicate witnesses collapse
        // in the set and let the duplicate pass the subset check.
        let decode_witnesses = |version: u16, witnesses: [u32; 2], incomplete: &[u32]| {
            let payload = mtrd_trailer_with_witnesses(version, witnesses, incomplete);
            let mut summary = DomainDataDecodeSummary::default();
            decode_compact_market_trade_classic_deal_trailer(0, &payload, 0, &mut summary);
            summary.parse_errors
        };
        for (label, witnesses, incomplete) in [
            ("a duplicate witness", [34u32, 34], &[34u32][..]),
            // Checked even with an EMPTY incomplete block: a v3 trailer owes a
            // well-formed witness list whether or not it names anyone short.
            ("a duplicate witness with no incomplete owner", [34, 34], &[][..]),
            ("a zero witness", [0, 67], &[][..]),
            (
                "a witness past the owner bound",
                [34, COMPACT_MARKET_TRADE_MAX_TAG_DB_ID + 1],
                &[][..],
            ),
        ] {
            let errors = decode_witnesses(
                COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_PER_OWNER_VERSION,
                witnesses,
                incomplete,
            );
            assert!(
                errors
                    .iter()
                    .any(|error| error.contains("witness") && error.contains("invalid or duplicated")),
                "{label} is refused here exactly as the fold refuses it: {errors:?}"
            );
        }

        // And the archived shapes are untouched -- including the malformed
        // witness lists this decoder has never inspected for v1/v2, which must
        // keep decoding exactly as their archived bytes always did.
        for version in [
            COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_LEGACY_VERSION,
            COMPACT_MARKET_TRADE_CLASSIC_DEAL_TRAILER_VERSION,
        ] {
            let (end, errors) = decode(version, &[]);
            assert!(errors.is_empty(), "v{version} still decodes: {errors:?}");
            assert_eq!(end, 32 + 2 * 4, "with no block between witnesses and rows");
            assert!(
                decode_witnesses(version, [34, 34], &[]).is_empty(),
                "and v{version}'s witness content is still not this decoder's business"
            );
        }
    }

    /// Minimal production-framed form of the six computed rows present at the
    /// tail of every t9 D44 payload. The first t9 payload used elapsed=2063 and
    /// carried flags/codes 0x200/1, 0x400/2, 0x4000/3, 0x10000/5,
    /// 0x100000/6, and 0x04000000/8 before FHB1. A zero-row FHB1 exercises the
    /// stricter boundary: exact EMPTY is still source-available.
    fn d44_t9_badge_shape(
        pressure_badge: supply44::domain44_contract::BadgeSpec,
        include_empty_fhb1: bool,
    ) -> Vec<u8> {
        let badges = [
            supply44::domain44_contract::EXACT_SUPPLY_WALL_BADGE,
            supply44::domain44_contract::ATTRITION_WALL_BADGE,
            supply44::domain44_contract::MOTORIZATION_WALL_BADGE,
            supply44::domain44_contract::CSUPPLY_CANDIDATE_REJECTED_BADGE,
            supply44::domain44_contract::COUNTRY_DISRUPTED_SOURCE_AVAILABLE_BADGE,
            pressure_badge,
        ];
        let mut payload = Vec::new();
        payload.extend_from_slice(&2_063u64.to_le_bytes());
        for value in [
            badges.len() as u32,
            badges.len() as u32,
            0,
            COMPACT_FIXED_SCALE_RAW,
            0,
            0,
            badges.len() as u32,
            0,
            0,
            0,
        ] {
            push_u32(&mut payload, value);
        }
        for badge in badges {
            for value in [
                COMPACT_SUPPLY_ROW_KIND_COMPUTED_BADGE,
                0,
                0,
                0,
                badge.flag,
            ] {
                push_u32(&mut payload, value);
            }
            payload.extend_from_slice(&badge.code.to_le_bytes());
            payload.extend_from_slice(&[0; 16]);
        }
        if include_empty_fhb1 {
            payload.extend_from_slice(b"FHB1");
            push_u32(&mut payload, 1); // version
            push_u32(&mut payload, 0); // exact empty row set
            push_u32(&mut payload, 0); // reserved
        }
        payload
    }

    /// Mutation proof: removing bit 26 from the shared production flag mask,
    /// removing badge code 8, restoring an unconditional convoy-wall demand,
    /// or treating FHB1 row_count=0 as unavailable makes this real-shape
    /// authority call fail.
    #[test]
    fn d44_t9_foreign_homebase_badge_and_empty_fhb1_are_certifiable() {
        let payload = d44_t9_badge_shape(
            supply44::domain44_contract::FOREIGN_HOMEBASE_SOURCE_AVAILABLE_BADGE,
            true,
        );
        let authority = summarize_domain_data_payload(
            44,
            &payload,
            DomainDataContext {
                stream_offset: 2_582_935,
                ..DomainDataContext::default()
            },
        )
        .expect("t9 D44 badge vocabulary with exact-empty FHB1 must certify");
        assert_eq!(authority.emitted_rows, 6);
        assert_eq!(authority.status_flags, Some(0));
    }

    #[test]
    fn d44_pressure_badge_still_fails_closed_on_real_mismatches() {
        let obsolete_wall = d44_t9_badge_shape(
            supply44::domain44_contract::CONVOY_PRESSURE_WALL_BADGE,
            true,
        );
        let error = summarize_domain_data_payload(
            44,
            &obsolete_wall,
            DomainDataContext::default(),
        )
        .expect_err("FHB1 plus the obsolete convoy wall is contradictory");
        assert!(error
            .errors
            .iter()
            .any(|message| message.contains("missing foreign homebase source availability")));
        assert!(error
            .errors
            .iter()
            .any(|message| message.contains("obsolete convoy pressure wall")));

        let unwitnessed_available = d44_t9_badge_shape(
            supply44::domain44_contract::FOREIGN_HOMEBASE_SOURCE_AVAILABLE_BADGE,
            false,
        );
        let error = summarize_domain_data_payload(
            44,
            &unwitnessed_available,
            DomainDataContext::default(),
        )
        .expect_err("source availability without FHB1 is not self-authenticating");
        assert!(error
            .errors
            .iter()
            .any(|message| message.contains("missing convoy pressure wall")));
        assert!(error
            .errors
            .iter()
            .any(|message| message.contains("source badge is present without FHB1")));
    }

    // --- subordinate DATA-payload statuses the fold gates on -----------------

    /// One d18 country_identity payload: base status `base_status`, one country,
    /// and a CIM1 extension carrying the three sub-statuses.
    fn country_identity_with_cim1(
        base_status: u32,
        claims_status: u32,
        cores_status: u32,
        major_status: u32,
    ) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u64.to_le_bytes());
        push_u32(&mut payload, 1); // row_count
        push_u32(&mut payload, 1); // changed_rows
        push_u32(&mut payload, base_status);
        push_u32(&mut payload, 47); // tag_db_id
        push_string(&mut payload, "GER");
        payload.push(1); // cosmetic null

        let major_available = !compact_contract::compact_status_is_failure(major_status);
        payload.extend_from_slice(b"CIM1");
        payload.extend_from_slice(&1u16.to_le_bytes());
        payload.extend_from_slice(&40u16.to_le_bytes());
        push_u32(&mut payload, claims_status);
        push_u32(&mut payload, cores_status);
        push_u32(&mut payload, major_status);
        push_u32(&mut payload, 0); // state rows
        push_u32(&mut payload, 0); // expected claim memberships
        push_u32(&mut payload, 0); // expected core memberships
        push_u32(&mut payload, u32::from(major_available)); // major rows
        push_u32(&mut payload, 0); // reserved
        if major_available {
            push_u32(&mut payload, 47);
            payload.push(1);
            payload.extend_from_slice(&[0, 0, 0]);
        }
        payload
    }

    /// d18's three CIM1 sub-statuses reach the record status.
    ///
    /// `WorldFold::fold_country_identity` gates on all three:
    /// `claims_available` / `cores_available` / `major_available` decide whether
    /// `country_claims_this_hour`, `country_cores_this_hour` and
    /// `country_major_this_hour` are `Some(map)` or `None` -- that is, whether
    /// each family is published for this hour at all. The decoder read them for
    /// its local count contract and then dropped them, so a record with a CLEAN
    /// base status and a failed membership read certified clean while the hour
    /// silently shipped with no claims. The collector writes exactly `0` or
    /// `SOURCE_UNAVAILABLE | PARTIAL_DATA` into these words, so they are the
    /// same vocabulary the classifier already speaks.
    ///
    /// Mutation proof: delete the `add_decoder_status(summary, cim1_status_or)`
    /// line in `decode_compact_country_identity_data_payload` and every failing
    /// case below reads `Some(0)`.
    #[test]
    fn d18_cim1_claims_cores_and_major_statuses_reach_the_record_status() {
        let failed = COMPACT_STATUS_SOURCE_UNAVAILABLE | COMPACT_STATUS_PARTIAL_DATA;

        // Control: everything clean stays clean, so the assertions below are
        // about the sub-statuses and not about some unrelated always-set bit.
        let clean = summarize_domain_data_payload(
            18,
            &country_identity_with_cim1(0, 0, 0, 0),
            DomainDataContext::default(),
        )
        .expect("a clean CIM1 record decodes");
        assert_eq!(clean.status_flags, Some(0));

        // Each sub-status ALONE, with a clean base status, must surface.
        for (name, claims, cores, major) in [
            ("claims", failed, 0, 0),
            ("cores", 0, failed, 0),
            ("major", 0, 0, failed),
        ] {
            let authority = summarize_domain_data_payload(
                18,
                &country_identity_with_cim1(0, claims, cores, major),
                DomainDataContext::default(),
            )
            .unwrap_or_else(|error| panic!("d18 {name}-unavailable record decodes: {error}"));
            let status = authority
                .status_flags
                .unwrap_or_else(|| panic!("d18 {name} record reports a status"));
            assert_ne!(
                status & COMPACT_STATUS_FAILURE_MASK,
                0,
                "d18 {name} sub-status failed but the record status stayed clean: {status:#010x}"
            );
            assert_eq!(
                compact_contract::compact_status_unknown_flags_v1(
                    compact_contract::compact_status_for_global_interpretation(18, status)
                ),
                0,
                "d18 {name} status must stay inside the v1 vocabulary the gates classify: {status:#010x}"
            );
        }
    }

    /// One d46 archetypes payload with an `ACE1` trailer carrying `ace_status`.
    fn air_variant_archetypes_with_ace_trailer(ace_status: u32) -> Vec<u8> {
        use air_wings_variants_lifecycle_live_lane::domain46_air_ace_roster_live as ace46;
        use air_wings_variants_lifecycle_live_lane::domain46_air_variant_archetypes_live as air46;

        let mut payload = air46::encode_payload(
            &air46::AirVariantArchetypeHeader {
                elapsed_wall_ms: 1,
                status_flags: air46::STATUS_DELTA_DATA,
                changed_rows: 0,
                observed_variant_rows: 0,
                classified_rows: 0,
                unresolved_rows: 0,
                static_catalog_hash: 0,
            },
            &[],
        );
        payload.extend_from_slice(&ace46::encode_trailer(
            &ace46::AirAceRosterHeader {
                status_flags: ace_status,
                observed_country_rows: 0,
                observed_ace_rows: 0,
                wing_assignment_rows: 0,
                dead_rows: 0,
                skipped_rows: 0,
            },
            &[],
        ));
        payload
    }

    /// d46's `ACE1` trailer status: the two bits the fold GATES on reach the
    /// record status; the one it does not stays trailer-local.
    ///
    /// `WorldFold::apply_air_ace_roster` returns early on
    /// `ACE_STATUS_SOURCE_UNAVAILABLE` or `ACE_STATUS_PARTIAL_DATA` -- it
    /// RETAINS the previous hour's ace map and per-country sidecars instead of
    /// full-replacing, and withholds `air_aces_fresh_this_hour`. The hour then
    /// publishes a roster it did not measure, and before this the record status
    /// said the domain was clean.
    ///
    /// `ACE_STATUS_WING_ASSIGNMENT_BLOCKED` must NOT cross: its value (1<<13)
    /// is `COMPACT_STATUS_WAR_SEGMENT_ABSENT` globally, an unrelated fact, and
    /// it is exactly the bit the fold does not gate on (the frame is still
    /// full-replaced). Admitting it would assert a war-segment fact about an air
    /// domain -- so this half of the test is what stops the fix from becoming
    /// "OR the whole word in".
    ///
    /// Mutation proof: widen the mask in
    /// `domain46_air_variant_archetypes_live::decode_payload` to
    /// `ACE_KNOWN_STATUS_MASK` and the blocked case fails; remove the OR
    /// entirely and both the unavailable and partial cases fail.
    #[test]
    fn d46_ace_trailer_status_crosses_exactly_where_the_fold_gates() {
        use air_wings_variants_lifecycle_live_lane::domain46_air_ace_roster_live as ace46;

        let clean = summarize_domain_data_payload(
            46,
            &air_variant_archetypes_with_ace_trailer(0),
            DomainDataContext::default(),
        )
        .expect("a clean ace trailer decodes");
        assert_eq!(
            clean
                .status_flags
                .map(|status| status & COMPACT_STATUS_FAILURE_MASK),
            Some(0),
            "a clean ace trailer must not invent a failure"
        );

        for (name, ace_status, expected_bit) in [
            (
                "source_unavailable",
                ace46::ACE_STATUS_SOURCE_UNAVAILABLE,
                COMPACT_STATUS_SOURCE_UNAVAILABLE,
            ),
            (
                "partial",
                ace46::ACE_STATUS_PARTIAL_DATA,
                COMPACT_STATUS_PARTIAL_DATA,
            ),
        ] {
            let authority = summarize_domain_data_payload(
                46,
                &air_variant_archetypes_with_ace_trailer(ace_status),
                DomainDataContext::default(),
            )
            .unwrap_or_else(|error| panic!("d46 ace {name} record decodes: {error}"));
            let status = authority
                .status_flags
                .unwrap_or_else(|| panic!("d46 ace {name} record reports a status"));
            assert_ne!(
                status & expected_bit,
                0,
                "the fold retained a stale ace roster for this hour but the d46 record status \
                 stayed clean of {name}: {status:#010x}"
            );
            assert_eq!(
                compact_contract::compact_status_unknown_flags_v1(
                    compact_contract::compact_status_for_global_interpretation(46, status)
                ),
                0,
                "d46 status must stay inside the v1 vocabulary the gates classify: {status:#010x}"
            );
        }

        // And the limit: the assignment-only anomaly is NOT a global fact.
        let blocked = summarize_domain_data_payload(
            46,
            &air_variant_archetypes_with_ace_trailer(ace46::ACE_STATUS_WING_ASSIGNMENT_BLOCKED),
            DomainDataContext::default(),
        )
        .expect("d46 wing-assignment-blocked record decodes");
        let status = blocked
            .status_flags
            .expect("d46 wing-assignment-blocked record reports a status");
        assert_eq!(
            status & ace46::ACE_STATUS_WING_ASSIGNMENT_BLOCKED,
            0,
            "trailer bit 1<<13 is COMPACT_STATUS_WAR_SEGMENT_ABSENT globally and must not be \
             asserted about an air domain: {status:#010x}"
        );
        assert_eq!(
            status & COMPACT_STATUS_FAILURE_MASK,
            0,
            "the fold still full-replaces on an assignment-only anomaly, so it is not a failure"
        );
    }
    /// Build one `MTRR` v3 trailer stating `tokens` as its resource set, with no
    /// rows. `tokens` is written verbatim, including bodies the decoder must
    /// refuse.
    fn mtrr_v3_identity_only(tokens: &[&str]) -> Vec<u8> {
        let max_len = compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN;
        let mut out = Vec::new();
        out.extend_from_slice(COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_MAGIC);
        out.extend_from_slice(&COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_VERSION.to_le_bytes());
        out.extend_from_slice(
            &(COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_HEADER_LEN as u16).to_le_bytes(),
        );
        out.extend_from_slice(&(COMPACT_MARKET_TRADE_RESOURCE_ROW_LEN as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        push_u32(&mut out, 0); // row count
        push_u32(&mut out, 0); // country count
        push_u32(&mut out, tokens.len() as u32);
        push_u32(&mut out, 1); // display scale
        push_u32(&mut out, COMPACT_STRATEGIC_RESOURCE_IDENTITY_STRIDE);
        for token in tokens {
            let bytes = token.as_bytes();
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
            let mut body = vec![0u8; max_len];
            let copied = bytes.len().min(max_len);
            body[..copied].copy_from_slice(&bytes[..copied]);
            out.extend_from_slice(&body);
        }
        out
    }

    /// The two entry points must not be interchangeable, or the split is
    /// documentation rather than enforcement.
    ///
    /// This is not hypothetical: routing domain 61's engine channel totals
    /// through the per-frame entry point is exactly the mistake this caught
    /// while the accumulation authority was being wired up.
    ///
    /// MUTATION: delete the `debug_assert!` in `add_compact_domain_data_unit`
    /// and this test stops panicking, so `#[should_panic]` fails it.
    /// A LAST-VALUE unit's zero is a measurement, and dropping it left a stale
    /// failure standing in the manifest.
    ///
    /// `air_strength_first_invalid_reason` is submitted on every frame. Frame 1
    /// fails with reason 31; frame 2 is clean and reports 0. Under the old
    /// zero-skip the manifest kept 31, so the capture reported a failure reason
    /// for a domain whose last observation was "no failure" — the
    /// `UnitPresence` mistake (absent and zero conflated) in a different place.
    ///
    /// MUTATION: restore the unconditional `if count == 0 { return; }` and the
    /// first assertion reads 31.
    #[test]
    fn a_zero_is_a_measurement_for_a_last_value_unit_and_a_no_op_for_a_sum() {
        use compact_contract::diagnostic_channel::{unit_accumulation, Accumulation};
        assert_eq!(
            unit_accumulation("air_strength_first_invalid_reason"),
            Accumulation::LastValue
        );

        let mut summary = DomainDataDecodeSummary::default();
        summary.per_domain.entry(42).or_default();
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_first_invalid_reason", 31);
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_first_invalid_reason", 0);
        assert_eq!(
            summary.per_domain[&42].data_units_total["air_strength_first_invalid_reason"],
            0,
            "the last observation was 'no failure' and the manifest must say so"
        );

        // An ordinal is a position, not a quantity: two frames at 1 then 2 must
        // read 2, never 3.
        assert_eq!(
            unit_accumulation("air_strength_first_invalid_ordinal"),
            Accumulation::LastValue
        );
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_first_invalid_ordinal", 1);
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_first_invalid_ordinal", 2);
        assert_eq!(
            summary.per_domain[&42].data_units_total["air_strength_first_invalid_ordinal"],
            2
        );

        // And a per-frame count still drops its zeroes and still sums, so the
        // fix did not turn every zero into a map entry.
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_rows", 0);
        assert!(
            !summary.per_domain[&42]
                .data_units_total
                .contains_key("air_strength_rows"),
            "zero is the identity for a sum, so it need not be stored"
        );
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_rows", 3);
        add_compact_domain_data_unit(&mut summary, 42, "air_strength_rows", 4);
        assert_eq!(
            summary.per_domain[&42].data_units_total["air_strength_rows"],
            7
        );
    }

    // The guard is a `debug_assert!`, deliberately: the release path still
    // folds by the DECLARED class, so a miscategorised call is a developer
    // error rather than a wrong figure, and the hot per-unit path pays
    // nothing for it. That means the guard does not exist in a release
    // build and this test cannot panic there -- so it is IGNORED in
    // release rather than left to fail the release gate.
    #[test]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "the guard is a debug_assert!; release compiles it out"
    )]
    #[should_panic(expected = "use add_compact_domain_cumulative_unit")]
    fn a_cumulative_unit_cannot_be_recorded_through_the_per_frame_entry_point() {
        let mut summary = DomainDataDecodeSummary::default();
        summary.per_domain.entry(61).or_default();
        add_compact_domain_data_unit(&mut summary, 61, "air_loss_our_fighters_lost_raw_sum", 7);
    }

    /// And the converse: a per-frame count must not be smuggled in as
    /// producer-retained state, which would publish a MAX where a total belongs.
    ///
    /// MUTATION: delete the `debug_assert!` in
    /// `add_compact_domain_cumulative_unit` and `#[should_panic]` fails.
    // The guard is a `debug_assert!`, deliberately: the release path still
    // folds by the DECLARED class, so a miscategorised call is a developer
    // error rather than a wrong figure, and the hot per-unit path pays
    // nothing for it. That means the guard does not exist in a release
    // build and this test cannot panic there -- so it is IGNORED in
    // release rather than left to fail the release gate.
    #[test]
    #[cfg_attr(
        not(debug_assertions),
        ignore = "the guard is a debug_assert!; release compiles it out"
    )]
    #[should_panic(expected = "is not declared cumulative")]
    fn a_per_frame_unit_cannot_be_recorded_through_the_cumulative_entry_point() {
        let mut summary = DomainDataDecodeSummary::default();
        summary.per_domain.entry(61).or_default();
        add_compact_domain_cumulative_unit(&mut summary, 61, "air_loss_stat_rows", 7);
    }

    fn decode_mtrr(payload: &[u8]) -> DomainDataDecodeSummary {
        let mut summary = DomainDataDecodeSummary::default();
        decode_compact_market_trade_sized_trailer(
            0,
            payload,
            0,
            &[
                COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_LEGACY_VERSION,
                COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_COUNT_ONLY_VERSION,
                COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_VERSION,
            ],
            COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_HEADER_LEN,
            COMPACT_MARKET_TRADE_RESOURCE_ROW_LEN,
            "MTRR",
            "market_trade_resource_rows",
            &mut summary,
        );
        summary
    }

    /// This authority must apply the SAME identity contract the fold and the
    /// lane decoder do. Sizing the block and stepping over it is not reading
    /// it: a trailer this side calls decoded and the fold then rejects is not
    /// an authority, it is a rubber stamp.
    ///
    /// Mutation: make the identity walk unreachable, and every case below is
    /// accepted with no parse error.
    #[test]
    fn x15_mtrr_identity_is_validated_by_the_domain_data_authority() {
        let max_len = compact_contract::build_profile::MAX_COMPACT_STRATEGIC_RESOURCE_TOKEN_LEN;

        // A well-formed set is accepted, so the refusals below are the contract
        // biting rather than the decoder being broken.
        let clean = decode_mtrr(&mtrr_v3_identity_only(&["oil", "steel"]));
        assert!(clean.parse_errors.is_empty(), "{:?}", clean.parse_errors);

        let over_long = "x".repeat(max_len + 1);
        for (tokens, needle) in [
            (vec![over_long.as_str(), "steel"], "token length"),
            (vec!["not a token", "steel"], "is not a resource token"),
            (vec!["oil", "oil"], "repeats token"),
            (vec!["", "steel"], "token length"),
        ] {
            let summary = decode_mtrr(&mtrr_v3_identity_only(&tokens));
            assert!(
                summary
                    .parse_errors
                    .iter()
                    .any(|error| error.contains(needle)),
                "{tokens:?} -> {:?}",
                summary.parse_errors
            );
        }

        // A non-zero padding byte carries payload nothing accounts for.
        let mut padded = mtrr_v3_identity_only(&["oil", "steel"]);
        let body = COMPACT_MARKET_TRADE_RESOURCE_ROW_TRAILER_HEADER_LEN + 2;
        padded[body + max_len - 1] = b'z';
        let summary = decode_mtrr(&padded);
        assert!(
            summary
                .parse_errors
                .iter()
                .any(|error| error.contains("non-zero padding")),
            "{:?}",
            summary.parse_errors
        );

        // And a non-UTF-8 body.
        let mut invalid = mtrr_v3_identity_only(&["oil", "steel"]);
        invalid[body] = 0xFF;
        let summary = decode_mtrr(&invalid);
        assert!(
            summary.parse_errors.iter().any(|error| {
                error.contains("is not UTF-8") || error.contains("is not a resource token")
            }),
            "{:?}",
            summary.parse_errors
        );
    }

    #[test]
    fn air_base_status_accepts_promoted_province_id() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u64.to_le_bytes());
        for value in [1u32, 1, 0, 1, 1, 0, 0] {
            payload.extend_from_slice(&value.to_le_bytes());
        }
        for value in [154u32, 2, 0, 1_600, 8, 3, 278, 321] {
            payload.extend_from_slice(&value.to_le_bytes());
        }

        let authority = summarize_domain_data_payload(
            34,
            &payload,
            DomainDataContext::default(),
        )
        .expect("A7-promoted province id is canonical domain-34 data");
        assert_eq!(authority.emitted_rows, 1);
        assert_eq!(authority.status_flags, Some(0));
    }

    #[test]
    fn weather_source_unavailable_empty_frame_is_structurally_decodable() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u64.to_le_bytes());
        for value in [
            0u32,
            0,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            0,
            0,
            0,
            COMPACT_FIXED_SCALE_RAW,
            0,
            0,
            0,
        ] {
            payload.extend_from_slice(&value.to_le_bytes());
        }

        let authority = summarize_domain_data_payload(
            24,
            &payload,
            DomainDataContext::default(),
        )
        .expect("typed source-unavailable weather frame remains decodable");
        assert_eq!(
            authority.status_flags,
            Some(COMPACT_STATUS_SOURCE_UNAVAILABLE)
        );
        assert_eq!(authority.emitted_rows, 0);
    }

    #[test]
    fn d13_summary_accepts_named_effective_ahead_extension() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        for value in [1, 1, 0, 1_000, 100_000, 1] {
            push_u32(&mut payload, value);
        }
        for value in [47, 0b111, 17, 3] {
            push_u32(&mut payload, value);
        }
        for value in [47, 1, 717, 717, 250, 0b1_1111] {
            push_u32(&mut payload, value);
        }
        payload.extend_from_slice(&125_000i64.to_le_bytes());
        payload.extend_from_slice(&300_000i64.to_le_bytes());
        push_string(&mut payload, "basic_machine_tools");
        for raw in [150_000i64, 300_000, 50_000, 200_000] {
            payload.extend_from_slice(&raw.to_le_bytes());
        }
        push_u32(&mut payload, 3); // action-gated selected-Present authority
        push_u32(&mut payload, 2); // reduced localization branch
        payload.extend_from_slice(&1u64.to_le_bytes()); // callback sequence
        push_u32(&mut payload, 2); // callback thread id
        payload.extend_from_slice(&43_800_024i32.to_le_bytes());
        payload.extend_from_slice(&3u64.to_le_bytes()); // callback QPC
        payload.extend_from_slice(&10_000_000u64.to_le_bytes()); // QPC frequency
        let candidate_set_sha256 = [0xabu8; 32];
        let selected_present = portable_render_stream::SelectedPresentReceipt::sealed(
            1,
            snapshot::sha256::to_hex(&candidate_set_sha256),
            7,
            0,
            43_800_024,
            4,
            5,
        );
        let selected_present_hex = selected_present.computed_sha256().unwrap();
        let mut selected_present_receipt_sha256 = [0u8; 32];
        for (index, byte) in selected_present_receipt_sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&selected_present_hex[index * 2..index * 2 + 2], 16)
                .unwrap();
        }
        payload.extend_from_slice(&a29_expected_source_record_sha256(
            1,
            2,
            3,
            10_000_000,
            43_800_024,
            1,
            717,
            2,
            (150_000, 300_000, 50_000, 200_000),
            1,
            7,
            0,
            43_800_024,
            4,
            5,
            &candidate_set_sha256,
            &selected_present_receipt_sha256,
        ));
        payload.extend_from_slice(&(A29_UI_YEAR_HELPER_RETURN_RVA as u32).to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&43_800_024i32.to_le_bytes());
        payload.extend_from_slice(&7u64.to_le_bytes());
        payload.extend_from_slice(&4u64.to_le_bytes());
        payload.extend_from_slice(&5u64.to_le_bytes());
        payload.extend_from_slice(&candidate_set_sha256);
        payload.extend_from_slice(&selected_present_receipt_sha256);

        let mut summary = DomainDataDecodeSummary {
            domain_id: 13,
            ..Default::default()
        };
        summary.per_domain.insert(13, DomainDataCounts::default());
        decode_compact_tech_data_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&13].data_units_total["effective_ahead_rows"],
            1
        );
        assert_eq!(
            summary.per_domain[&13].data_units_total["observed_ahead_provenance_rows"],
            1
        );

        let mut corrupted = payload.clone();
        let last = corrupted.last_mut().expect("A29 source hash byte");
        *last ^= 1;
        let mut corrupted_summary = DomainDataDecodeSummary {
            domain_id: 13,
            ..Default::default()
        };
        corrupted_summary
            .per_domain
            .insert(13, DomainDataCounts::default());
        decode_compact_tech_data_payload(0, &corrupted, &mut corrupted_summary);
        assert!(corrupted_summary
            .parse_errors
            .iter()
            .any(|error| error.contains("A29 provenance is invalid")));
    }

    #[test]
    fn d42_summary_accepts_exact_air_region_cause_trailer() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        for value in [
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            COMPACT_AIR_STRENGTH_EQUIPMENT_AMOUNT_PER_PLANE as u32,
            0,
        ] {
            push_u32(&mut payload, value);
        }
        push_u32(&mut payload, COMPACT_AIR_REGION_STATS_TRAILER_MAGIC);
        payload.extend_from_slice(&COMPACT_AIR_REGION_STATS_TRAILER_VERSION.to_le_bytes());
        payload
            .extend_from_slice(&(COMPACT_AIR_REGION_STATS_TRAILER_HEADER_LEN as u16).to_le_bytes());
        for value in [1, 1, 1, 2, 3, (1 << 16) | 31] {
            push_u32(&mut payload, value);
        }
        push_u32(&mut payload, 1); // owner
        push_u32(&mut payload, 88); // region
        payload.extend_from_slice(&[0, 1, 0, 1]);
        push_u32(&mut payload, 700); // wing
        push_u32(&mut payload, 12345); // date raw
        push_u32(&mut payload, 96); // planes
        push_u32(&mut payload, 7); // mission
        push_u32(&mut payload, 1); // attributed country
        push_u32(&mut payload, 0); // damage first
        push_u32(&mut payload, 1); // damage count
        payload.extend_from_slice(&[1, 4, 0, 0]);
        push_u32(&mut payload, 0);
        payload.extend_from_slice(&300_000i64.to_le_bytes());

        let mut summary = DomainDataDecodeSummary {
            domain_id: 42,
            ..Default::default()
        };
        summary.per_domain.insert(42, DomainDataCounts::default());
        decode_compact_air_combat_losses_live_data_payload(0, &payload, &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&42].data_units_total;
        assert_eq!(units["air_region_combat_stat_rows"], 1);
        assert_eq!(units["air_region_damage_rows"], 1);
        assert_eq!(units["air_strength_invalid_wing_rows"], 2);
        assert_eq!(units["air_strength_scale_mismatch_rows"], 3);
        assert_eq!(units["air_strength_first_invalid_ordinal"], 1);
        assert_eq!(units["air_strength_first_invalid_reason"], 31);
        assert_eq!(units["air_strength_invalid_experience_range"], 1);
    }

    #[test]
    fn x4_p6_authority_accepts_exact_pcp1_and_rejects_unknown_flags() {
        fn payload(flags: u32) -> Vec<u8> {
            let mut payload = Vec::new();
            payload.extend_from_slice(&0u64.to_le_bytes());
            for value in [1u32, 1, 0, COMPACT_PRODUCTION_SCALE_RAW, 0, 0, 0, 0] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
            payload.extend_from_slice(b"PCP1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&28u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&flags.to_le_bytes());
            payload
        }

        let mut summary = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        summary.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(0b111), &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.per_domain[&11].data_units_total["production_completeness_rows"],
            1
        );

        let mut rejected = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        rejected.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(8), &mut rejected);
        assert!(rejected
            .parse_errors
            .iter()
            .any(|error| error.contains("PCP1 row 0 has unknown flags")));

        let mut incoherent = Vec::new();
        incoherent.extend_from_slice(&0u64.to_le_bytes());
        for value in [1u32, 1, 0, COMPACT_PRODUCTION_SCALE_RAW, 1, 0, 0, 0] {
            incoherent.extend_from_slice(&value.to_le_bytes());
        }
        incoherent.extend_from_slice(&47u32.to_le_bytes());
        incoherent.extend_from_slice(&70u32.to_le_bytes());
        incoherent.extend_from_slice(&1i64.to_le_bytes());
        incoherent.extend_from_slice(&payload(0)[40..]);
        let mut rejected = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        rejected.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &incoherent, &mut rejected);
        assert!(rejected
            .parse_errors
            .iter()
            .any(|error| error.contains("marks stockpile unreadable")));
    }

    #[test]
    fn b2_authority_accepts_pmi1_pcp1_pcv1_chain_and_rejects_forged_total() {
        fn payload(available: i32) -> Vec<u8> {
            let mut payload = Vec::new();
            payload.extend_from_slice(&0u64.to_le_bytes());
            for value in [1u32, 1, 0, COMPACT_PRODUCTION_SCALE_RAW, 0, 1, 0, 0] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
            // One military production line at key (47, 1, 349).
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&349i32.to_le_bytes());
            payload.extend_from_slice(&2i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            // Exact optional-MIO join (zero is the proven-null CID).
            payload.extend_from_slice(b"PMI1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&36u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&349i32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            // Production-vector completeness.
            payload.extend_from_slice(b"PCP1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&28u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&(1u32 << 1).to_le_bytes());
            // One complete B2 country, variant, and reservation.
            payload.extend_from_slice(b"PCV1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&28u16.to_le_bytes());
            payload.extend_from_slice(&92u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&0b1_1111u32.to_le_bytes());
            payload.extend_from_slice(&36_900_000i64.to_le_bytes());
            payload.extend_from_slice(&21i32.to_le_bytes());
            payload.extend_from_slice(&available.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&7316u32.to_le_bytes());
            payload.extend_from_slice(&70u32.to_le_bytes());
            payload.extend_from_slice(&36_900_000i64.to_le_bytes());
            payload.extend_from_slice(&10i32.to_le_bytes());
            payload.extend_from_slice(&21i32.to_le_bytes());
            payload
        }

        let mut summary = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        summary.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(348), &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&11].data_units_total;
        assert_eq!(units["production_mio_join_rows"], 1);
        assert_eq!(units["country_convoy_rows"], 1);
        assert_eq!(units["country_convoy_variant_rows"], 1);
        assert_eq!(units["country_convoy_reservation_rows"], 1);

        let mut rejected = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        rejected.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(347), &mut rejected);
        assert!(rejected
            .parse_errors
            .iter()
            .any(|error| error.contains("fails the exact total law")));
    }

    #[test]
    fn xui12_authority_accepts_exact_pls1_pef1_and_rejects_incoherent_subtype() {
        fn payload(subtype: u32) -> Vec<u8> {
            let mut payload = Vec::new();
            payload.extend_from_slice(&0u64.to_le_bytes());
            for value in [1u32, 1, 0, COMPACT_PRODUCTION_SCALE_RAW, 0, 1, 0, 0] {
                payload.extend_from_slice(&value.to_le_bytes());
            }
            // One military base row at key (47, 1, 349).
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&349i32.to_le_bytes());
            payload.extend_from_slice(&2i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            payload.extend_from_slice(&0i32.to_le_bytes());
            // Exact PLS1 join.
            payload.extend_from_slice(b"PLS1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&36u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&349i32.to_le_bytes());
            payload.extend_from_slice(&subtype.to_le_bytes());
            // Exact PEF1 join.
            payload.extend_from_slice(b"PEF1");
            payload.extend_from_slice(&1u16.to_le_bytes());
            payload.extend_from_slice(&20u16.to_le_bytes());
            payload.extend_from_slice(&56u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&0u32.to_le_bytes());
            payload.extend_from_slice(&47u32.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.extend_from_slice(&349i32.to_le_bytes());
            payload.extend_from_slice(&7_500_000i64.to_le_bytes());
            payload.extend_from_slice(&10_000_000i64.to_le_bytes());
            payload.extend_from_slice(&125_000i64.to_le_bytes());
            payload
        }

        let mut summary = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        summary.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(4), &mut summary);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let units = &summary.per_domain[&11].data_units_total;
        assert_eq!(units["production_line_subtype_rows"], 1);
        assert_eq!(units["production_railway_gun_subtype_rows"], 1);
        assert_eq!(units["production_efficiency_rows"], 1);

        let mut rejected = DomainDataDecodeSummary {
            domain_id: 11,
            ..Default::default()
        };
        rejected.per_domain.insert(11, DomainDataCounts::default());
        decode_compact_production_data_payload(0, &payload(3), &mut rejected);
        assert!(rejected
            .parse_errors
            .iter()
            .any(|error| error.contains("PLS1 row 0 has invalid tag/kind/subtype (47, 1, 3)")));
    }

    #[test]
    fn static_join_inventory_extracts_only_approved_string_fields() {
        let mut politics = Vec::new();
        politics.extend_from_slice(&0u64.to_le_bytes());
        for value in [1, 1, 0, COMPACT_FIXED_SCALE_RAW, 0, 0, 1, 0] {
            push_u32(&mut politics, value);
        }
        for value in [47, 0, 1, u32::MAX, 0, 0, 0] {
            push_u32(&mut politics, value);
        }
        for _ in 0..4 {
            push_string(&mut politics, "");
        }
        push_u32(&mut politics, 47);
        push_string(&mut politics, "political_advisor");
        push_string(&mut politics, "economy_chief");
        let tokens = collect_static_join_tokens(14, &politics, DomainDataContext::default())
            .expect("valid politics payload");
        assert_eq!(
            tokens,
            BTreeSet::from([StaticJoinToken {
                kind: "idea",
                token: "economy_chief".to_string(),
            }])
        );

        use air_wings_variants_lifecycle_live_lane::domain46_air_variant_archetypes_live as air46;
        let air = air46::encode_payload(
            &air46::AirVariantArchetypeHeader {
                elapsed_wall_ms: 0,
                status_flags: air46::STATUS_DELTA_DATA,
                changed_rows: 2,
                observed_variant_rows: 2,
                classified_rows: 2,
                unresolved_rows: 0,
                static_catalog_hash: 0,
            },
            &[
                air46::AirVariantArchetypeRow {
                    variant_id: 1,
                    deleted: false,
                    equipment_token_raw: 1,
                    equipment_type_token_raw: 1,
                    interface_type_raw: 1,
                    air_superiority_weight_raw: None,
                    source_mask: air46::SOURCE_LIVE_STATS | air46::SOURCE_LIVE_TOKEN_DICTIONARY,
                    archetype_token: Some("fighter_equipment_0".to_string()),
                    base_airframe_token: None,
                    role_token: None,
                },
                air46::AirVariantArchetypeRow {
                    variant_id: 2,
                    deleted: false,
                    equipment_token_raw: 2,
                    equipment_type_token_raw: 2,
                    interface_type_raw: 2,
                    air_superiority_weight_raw: None,
                    source_mask: air46::SOURCE_LIVE_STATS | air46::SOURCE_STATIC_HEURISTIC,
                    archetype_token: Some("heuristic_must_not_join".to_string()),
                    base_airframe_token: None,
                    role_token: None,
                },
            ],
        );
        let tokens = collect_static_join_tokens(46, &air, DomainDataContext::default())
            .expect("valid air archetype payload");
        assert_eq!(
            tokens,
            BTreeSet::from([StaticJoinToken {
                kind: "equipment",
                token: "fighter_equipment_0".to_string(),
            }])
        );
    }

    fn valid_frd_depth_payload(schema_version: u16, raw_status: Option<u32>) -> Vec<u8> {
        let header_len = match schema_version {
            COMPACT_FRD_DEPTH_SCHEMA_VERSION_V2 => COMPACT_FRD_DEPTH_HEADER_LEN_V2,
            COMPACT_FRD_DEPTH_SCHEMA_VERSION => COMPACT_FRD_DEPTH_HEADER_LEN,
            COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5 => COMPACT_FRD_DEPTH_HEADER_LEN_V5,
            _ => panic!("unsupported test schema version"),
        };
        let direct_source_validated = schema_version == COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5;
        let mut out = Vec::new();
        out.extend_from_slice(COMPACT_FRD_DEPTH_SCHEMA_MAGIC);
        out.extend_from_slice(&schema_version.to_le_bytes());
        out.extend_from_slice(&(header_len as u16).to_le_bytes());
        push_u32(
            &mut out,
            COMPACT_FRD_STATUS_KNOWN_MASK
                | if direct_source_validated {
                    COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED
                } else {
                    0
                },
        );
        for _ in 0..9 {
            push_u32(&mut out, 0);
        }
        push_u32(&mut out, if direct_source_validated { 2 } else { 3 }); // badge_count
        push_u32(&mut out, 0); // decision_target_omitted_rows
        if matches!(
            schema_version,
            COMPACT_FRD_DEPTH_SCHEMA_VERSION | COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5
        ) {
            push_u32(&mut out, raw_status.expect("v3 raw status"));
        }
        if direct_source_validated {
            push_u32(&mut out, 0); // direct_transition_count
        }
        assert_eq!(out.len(), header_len);

        for (kind, flags, source_domain, evidence, detail) in [
            (
                COMPACT_FRD_BADGE_KIND_FOCUS_PAUSE_FAIL_CLOSED,
                COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED | COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED,
                12,
                COMPACT_FRD_BADGE_EVIDENCE_NO_PAUSED_TRUE_GROUND_TRUTH,
                COMPACT_FRD_BADGE_DETAIL_FOCUS_PAUSE_FAIL_CLOSED,
            ),
            (
                COMPACT_FRD_BADGE_KIND_DECISION_TARGET_FIELDS_FAIL_CLOSED,
                COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED | COMPACT_FRD_BADGE_FLAG_CURRENT_BUILD_PROBED,
                16,
                COMPACT_FRD_BADGE_EVIDENCE_TIMED_DEFINITION_LAYOUT_REJECTED,
                COMPACT_FRD_BADGE_DETAIL_DECISION_TARGET_FIELDS_FAIL_CLOSED,
            ),
            (
                COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE,
                COMPACT_FRD_BADGE_FLAG_FAIL_CLOSED | COMPACT_FRD_BADGE_FLAG_DERIVED_DETERMINISTIC,
                1000,
                COMPACT_FRD_BADGE_EVIDENCE_COMPACT_FRAME_DELTA_DERIVED,
                COMPACT_FRD_BADGE_DETAIL_DIRECT_EVENT_HOOKS_UNAVAILABLE,
            ),
        ] {
            if direct_source_validated
                && kind == COMPACT_FRD_BADGE_KIND_DIRECT_EVENT_HOOKS_UNAVAILABLE
            {
                continue;
            }
            push_u32(&mut out, kind);
            push_u32(&mut out, flags);
            push_u32(&mut out, source_domain);
            push_u32(&mut out, 0);
            push_u32(&mut out, evidence);
            push_string(&mut out, detail);
        }
        if direct_source_validated {
            out.extend_from_slice(
                &direct_frd_transitions_live::encode_trailer(&[])
                    .expect("empty direct-event trailer"),
            );
        }
        out
    }

    #[test]
    fn frd_v3_zero_raw_status_is_authoritative_while_v2_is_missing() {
        let v2 = summarize_domain_data_payload(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            &valid_frd_depth_payload(COMPACT_FRD_DEPTH_SCHEMA_VERSION_V2, None),
            DomainDataContext {
                record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                ..DomainDataContext::default()
            },
        )
        .expect("valid FRD2 v2");
        assert_eq!(v2.status_flags, None);

        let v3 = summarize_domain_data_payload(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            &valid_frd_depth_payload(COMPACT_FRD_DEPTH_SCHEMA_VERSION, Some(0)),
            DomainDataContext {
                record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                ..DomainDataContext::default()
            },
        )
        .expect("valid FRD2 v3");
        assert_eq!(v3.status_flags, Some(0));
    }

    #[test]
    fn frd_v5_direct_source_bit_is_schema_scoped_and_trailer_bound() {
        let v5 = summarize_domain_data_payload(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            &valid_frd_depth_payload(COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5, Some(0)),
            DomainDataContext {
                record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                ..DomainDataContext::default()
            },
        )
        .expect("valid FRD2 v5 with authenticated empty direct-event drain");
        assert_eq!(v5.status_flags, Some(0));
        assert_eq!(
            v5.data_units
                .get("direct_transition_rows")
                .copied()
                .unwrap_or(0),
            0
        );

        let mut legacy = valid_frd_depth_payload(COMPACT_FRD_DEPTH_SCHEMA_VERSION, Some(0));
        let legacy_status = read_u32_le(&legacy, 8)
            | COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED;
        legacy[8..12].copy_from_slice(&legacy_status.to_le_bytes());
        let error = summarize_domain_data_payload(
            COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
            &legacy,
            DomainDataContext {
                record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                ..DomainDataContext::default()
            },
        )
        .expect_err("legacy FRD2 must not claim schema-v5 direct-hook authority");
        assert!(error
            .errors
            .iter()
            .any(|item| item.contains("legacy payload claims validated direct-hook source")));
    }

    /// Bit 7 of `depth_status_flags` is an ASSIGNED bit, on every schema
    /// version, and the decoder admits it.
    ///
    /// `DELTA_WIRE_DESIGN.md` §7.3.1's D45-LOCAL direct-history reset badge.
    /// The decoder refuses any bit outside `admitted_depth_status_mask`, so a
    /// badge that was not registered there would be rejected as an unknown bit
    /// on exactly the records it exists to rescue -- the repair would announce
    /// itself as corruption.
    ///
    /// Admitted on v4 as well as v5 deliberately. One of the two ways the
    /// direct history dies is a source DOWNGRADE, and the record able to carry
    /// the badge then is an untrailered v4 one; gating the badge on v5 would
    /// leave that shape dead until the hooks came back.
    ///
    /// The negative control is bit 8, which is genuinely unassigned: without
    /// it, a decoder that had simply stopped checking unknown bits would pass
    /// the positive half.
    ///
    /// What production change makes this fail: dropping
    /// `COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET` from
    /// `COMPACT_FRD_STATUS_KNOWN_MASK`, or reassigning bit 7 to something else
    /// without moving the badge.
    #[test]
    fn frd_depth_direct_history_reset_badge_is_an_assigned_bit() {
        assert_eq!(COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET, 1 << 7);
        assert_ne!(
            COMPACT_FRD_STATUS_KNOWN_MASK & COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET,
            0,
            "the reset badge must be registered in the mask that governs depth_status_flags"
        );
        // It is a DISTINCT bit, not an alias of a neighbour that would make the
        // decoder accept it for the wrong reason.
        for (name, other) in [
            ("focus_pause_badged", COMPACT_FRD_STATUS_FOCUS_PAUSE_BADGED),
            (
                "decision_targets_badged",
                COMPACT_FRD_STATUS_DECISION_TARGETS_BADGED,
            ),
            (
                "direct_events_derived",
                COMPACT_FRD_STATUS_DIRECT_EVENTS_DERIVED,
            ),
            ("focus_pause_exact", COMPACT_FRD_STATUS_FOCUS_PAUSE_EXACT),
            (
                "decision_targets_exact",
                COMPACT_FRD_STATUS_DECISION_TARGETS_EXACT,
            ),
            (
                "decision_cooldowns_exact",
                COMPACT_FRD_STATUS_DECISION_COOLDOWNS_EXACT,
            ),
            (
                "direct_hook_source_validated",
                COMPACT_FRD_STATUS_DIRECT_HOOK_SOURCE_VALIDATED,
            ),
        ] {
            assert_eq!(
                COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET & other,
                0,
                "the reset badge collides with {name}"
            );
        }

        for (label, schema_version, raw_status) in [
            ("v4", COMPACT_FRD_DEPTH_SCHEMA_VERSION, Some(0u32)),
            ("v5", COMPACT_FRD_DEPTH_SCHEMA_VERSION_V5, Some(0u32)),
        ] {
            let mut badged = valid_frd_depth_payload(schema_version, raw_status);
            let status = read_u32_le(&badged, 8) | COMPACT_FRD_STATUS_DIRECT_HISTORY_RESET;
            badged[8..12].copy_from_slice(&status.to_le_bytes());
            summarize_domain_data_payload(
                COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
                &badged,
                DomainDataContext {
                    record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                    ..DomainDataContext::default()
                },
            )
            .unwrap_or_else(|error| {
                panic!("{label} payload carrying the reset badge was refused: {error:?}")
            });

            // NEGATIVE CONTROL: the next bit up is unassigned and must still be
            // refused, or the check above proves only that the decoder stopped
            // looking.
            let mut unassigned = valid_frd_depth_payload(schema_version, raw_status);
            let status = read_u32_le(&unassigned, 8) | (1 << 8);
            unassigned[8..12].copy_from_slice(&status.to_le_bytes());
            let error = summarize_domain_data_payload(
                COMPACT_DOMAIN_FOCUS_RESEARCH_DECISIONS_DEPTH_LIVE,
                &unassigned,
                DomainDataContext {
                    record_flags: COMPACT_RECORD_FLAG_PAYLOAD_SCHEMA_V2,
                    ..DomainDataContext::default()
                },
            )
            .expect_err("an unassigned depth status bit must still be refused");
            assert!(
                error
                    .errors
                    .iter()
                    .any(|item| item.contains("depth status_flags has unknown bits")),
                "{label}: the unassigned bit must be named as an unknown bit, not refused for \
                 some unrelated reason: {:?}",
                error.errors
            );
        }
    }

    #[test]
    fn sunk_convoy_history_summary_accepts_exact_sch1_payload() {
        let rows = [
            compact_contract::naval_history::SunkConvoyHistoryRow {
                month: 10,
                convoys: 2,
                killer_country_tag_db_id: 4,
                owner_tag_db_id: 9,
            },
            compact_contract::naval_history::SunkConvoyHistoryRow {
                month: 11,
                convoys: 3,
                killer_country_tag_db_id: 9,
                owner_tag_db_id: 4,
            },
        ];
        let payload =
            compact_contract::naval_history::encode_exact_payload(&rows, 0, 16_959_360, 2, 0, 2)
                .unwrap();
        let authority = summarize_domain_data_payload(
            COMPACT_DOMAIN_SUNK_CONVOY_HISTORY_LIVE,
            &payload,
            DomainDataContext::default(),
        )
        .expect("valid SCH1 authority");
        assert_eq!(authority.emitted_rows, 2);
        assert_eq!(authority.data_units["sunk_convoy_history_rows"], 2);
        assert_eq!(authority.data_units["sunk_convoy_history_observed_rows"], 2);
        assert_eq!(authority.data_units["sunk_convoy_history_convoys"], 5);
        assert_eq!(authority.status_flags, Some(0));
    }

    fn convoy_transport_attack_test_row() -> naval4950::ConvoyTransportAttackRow {
        let client = naval4950::ConvoyTerminalEntityHandle {
            id: 0x1234,
            type_tag: naval4950::GAME_ENTITY_TYPE_TAG,
        };
        naval4950::ConvoyTransportAttackRow {
            carrier: naval4950::ConvoyTerminalAttributionCarrier {
                producer_sequence: 1,
                game_hour_raw: 156_618,
                combat: naval4950::CombatEpoch {
                    combat_id: 77,
                    generation: 3,
                },
                convoy: naval4950::ConvoyTerminalEntityHandle {
                    id: 0x4321,
                    type_tag: naval4950::GAME_ENTITY_TYPE_TAG,
                },
                owner_tag_db_id: 47,
                killer_tag_db_id: 9,
                client,
                convoy_index: 5,
                member_state: naval4950::TERMINAL_SUNK_STATE,
                battle_slot: 2,
                route: naval4950::LiveRouteResolution {
                    route_key: client,
                    producer_kind: naval4950::RouteProducerKind::NavalTransport,
                    source: naval4950::RouteEndpoint {
                        kind: naval4950::RouteEndpointKind::Province,
                        id: 12_140,
                    },
                    destination: naval4950::RouteEndpoint {
                        kind: naval4950::RouteEndpointKind::Province,
                        id: 9_950,
                    },
                },
            },
            d70: naval4950::ConvoyD70MutationProof {
                kind: naval4950::ConvoyD70MutationKind::AppendFirstSeen,
                ordinal: 0,
                before_rows: 0,
                after_rows: 1,
                before_count: 0,
                after_count: 1,
            },
        }
    }

    fn convoy_transport_attack_test_witness(
        exact: bool,
    ) -> naval4950::ConvoyTransportAttackWitness {
        convoy_transport_attack_test_witness_with_unbound(exact, 0)
    }

    fn convoy_transport_attack_test_witness_with_unbound(
        exact: bool,
        resolver_unbound_convoy_clients: u32,
    ) -> naval4950::ConvoyTransportAttackWitness {
        naval4950::ConvoyTransportAttackWitness {
            resolver_unbound_convoy_clients,
            resolver_ever_partial: resolver_unbound_convoy_clients != 0,
            hook_installed: true,
            queue_drain_complete: true,
            producer_sequence_next: 2,
            drained_rows: 1,
            remaining_events: 0,
            active_callbacks: 0,
            resolver_publication_sequence: 1,
            resolver_game_hour_raw: 156_618,
            resolver_member_bindings: 1,
            resolver_route_bindings: 1,
            telemetry: naval4950::ConvoyTransportAttackTelemetry {
                callbacks_seen: if exact { 1 } else { 2 },
                original_calls: if exact { 1 } else { 2 },
                events_emitted: 1,
                dropped_queue_full: u64::from(!exact),
                ..naval4950::ConvoyTransportAttackTelemetry::default()
            },
        }
    }

    #[test]
    fn convoy_transport_attack_summary_accepts_exact_cta2_and_counts_witness() {
        let row = convoy_transport_attack_test_row();
        let payload = naval4950::encode_convoy_transport_attack_batch(
            &[row],
            convoy_transport_attack_test_witness(true),
        )
        .expect("valid exact CTA2 payload");
        let authority = summarize_domain_data_payload(
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            &payload,
            DomainDataContext::default(),
        )
        .expect("valid exact CTA2 authority");

        assert_eq!(authority.status_flags, None);
        assert_eq!(authority.emitted_rows, 1);
        assert_eq!(authority.changed_rows, 1);
        assert_eq!(authority.data_units["convoy_transport_attack_batches"], 1);
        assert_eq!(authority.data_units["convoy_transport_attack_rows"], 1);
        assert_eq!(
            authority.data_units["convoy_transport_attack_exact_complete_batches"],
            1
        );
        assert_eq!(
            authority.data_units["convoy_transport_attack_callbacks_seen"],
            1
        );
        assert_eq!(
            authority.data_units["convoy_transport_attack_resolver_route_bindings"],
            1
        );
        assert!(authority.problems.is_empty());
    }

    /// The t15 defect, reproduced through the real d90 decoder.
    ///
    /// The hook's counters are process-global and reported at their ABSOLUTE
    /// value on every batch, so a capture whose counter reaches 3 across three
    /// batches must publish 3 — not 1+2+3. In `runs/live_unlock/t15` the
    /// counter ran 1..107 over 396 batches and the manifest published their
    /// sum, 19,470, as an event count.
    ///
    /// MUTATION: record the telemetry through `add_compact_domain_data_unit`,
    /// or classify `convoy_transport_attack_callbacks_seen` as `PerFrame`, and
    /// the cumulative assertion reads 6 instead of 3.
    #[test]
    fn convoy_transport_attack_cumulative_telemetry_publishes_once_not_once_per_batch() {
        let mut summary = DomainDataDecodeSummary::default();
        summary
            .per_domain
            .entry(COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE)
            .or_default();

        // Three batches whose absolute counter reads 1, then 2, then 3.
        for absolute in 1u64..=3 {
            let mut witness = convoy_transport_attack_test_witness(true);
            witness.telemetry.callbacks_seen = absolute;
            witness.telemetry.original_calls = absolute;
            witness.telemetry.events_emitted = absolute;
            witness.producer_sequence_next = absolute + 1;
            let payload = naval4950::encode_convoy_transport_attack_batch(
                &[convoy_transport_attack_test_row()],
                witness,
            )
            .expect("valid CTA2 payload");
            decode_compact_convoy_transport_attack_live_data_payload(
                0,
                &payload,
                &mut summary,
            );
        }

        let domain = &summary.per_domain[&COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE];
        assert_eq!(
            domain.data_units_total["convoy_transport_attack_callbacks_seen"], 3,
            "a counter reported at its absolute value on every batch is not summed; \
             summing 1+2+3 is the same arithmetic that published 19,470 in t15"
        );
        assert_eq!(domain.data_units_total["convoy_transport_attack_original_calls"], 3);
        assert_eq!(domain.data_units_total["convoy_transport_attack_events_emitted"], 3);

        // The per-batch counters beside them must still SUM, or the fix would
        // be "aggregate everything by MAX", which loses real counts.
        assert_eq!(
            domain.data_units_total["convoy_transport_attack_batches"], 3,
            "a per-batch incidence count must still sum"
        );
        assert_eq!(domain.data_units_total["convoy_transport_attack_rows"], 3);
        assert_eq!(domain.data_units_total["convoy_transport_attack_drained_rows"], 3);

        // A gauge is the peak of the publication in force, not its total.
        assert_eq!(
            domain.data_units_total["convoy_transport_attack_resolver_route_bindings"], 1,
            "one publication observed three times is one publication"
        );
    }

    #[test]
    fn convoy_transport_attack_summary_preserves_incomplete_witness_as_problem() {
        let row = convoy_transport_attack_test_row();
        let payload = naval4950::encode_convoy_transport_attack_batch(
            &[row],
            convoy_transport_attack_test_witness(false),
        )
        .expect("structurally valid incomplete CTA2 payload");
        let authority = summarize_domain_data_payload(
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            &payload,
            DomainDataContext::default(),
        )
        .expect("incomplete witness is decoded, not promoted");

        assert_eq!(
            authority.data_units["convoy_transport_attack_incomplete_batches"],
            1
        );
        assert_eq!(
            authority.data_units["convoy_transport_attack_dropped_events"],
            1
        );
        assert_eq!(
            authority.problems["convoy_transport_attack_incomplete_witness"],
            1
        );
        assert_eq!(
            authority.problems["convoy_transport_attack_dropped_events"],
            1
        );
    }

    /// Law: rows captured under a PARTIAL resolver reach the authority, and
    /// the partiality reaches it with them as an exact count.
    ///
    /// Both halves are load-bearing. If the collector goes back to discarding
    /// a walk with any unreadable convoy member, `emitted_rows` here is 0. If
    /// the count stops being carried, the problem entry disappears and a
    /// consumer can no longer tell this batch from one that covered every
    /// convoy member the walk saw.
    #[test]
    fn convoy_transport_attack_summary_publishes_partial_resolver_rows_and_counts_them() {
        let row = convoy_transport_attack_test_row();
        let payload = naval4950::encode_convoy_transport_attack_batch(
            &[row],
            convoy_transport_attack_test_witness_with_unbound(true, 7),
        )
        .expect("a partial-resolver batch is structurally valid and must encode");
        let authority = summarize_domain_data_payload(
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            &payload,
            DomainDataContext::default(),
        )
        .expect("a partial-resolver batch is decoded, not refused");

        // The readable event survives.
        assert_eq!(authority.emitted_rows, 1);
        assert_eq!(authority.data_units["convoy_transport_attack_rows"], 1);
        // The partiality is on the record, exactly.
        assert_eq!(
            authority.data_units["convoy_transport_attack_resolver_unbound_convoy_clients"],
            7
        );
        assert_eq!(
            authority.problems["convoy_transport_attack_resolver_unbound_convoy_clients"],
            7
        );
        // And the batch does not claim to be exhaustive.
        assert_eq!(
            authority.data_units["convoy_transport_attack_incomplete_batches"],
            1
        );
        assert_eq!(
            authority.problems["convoy_transport_attack_incomplete_witness"],
            1
        );
    }

    #[test]
    fn convoy_transport_attack_summary_rejects_cta2_reserved_bytes() {
        let row = convoy_transport_attack_test_row();
        let mut payload = naval4950::encode_convoy_transport_attack_batch(
            &[row],
            convoy_transport_attack_test_witness(true),
        )
        .expect("valid exact CTA2 payload");
        // Word 188 became `resolver_unbound_convoy_clients` in CTA2 v2; word
        // 20 is still reserved-zero and is what this law is about.
        payload[20] = 1;

        let error = summarize_domain_data_payload(
            COMPACT_DOMAIN_CONVOY_TRANSPORT_ATTACK_LIVE,
            &payload,
            DomainDataContext::default(),
        )
        .expect_err("CTA2 reserved header bytes must remain zero");
        assert!(error
            .errors
            .iter()
            .any(|item| item.contains("CTA2 payload invalid: Reserved")));
    }

    /// Drive the production gate over an assembled status word without needing a
    /// well-formed payload for every one of the ~95 dispatched domains.
    ///
    /// This calls `reject_unassigned_status_bits` — the production function, not
    /// a restatement of it — after feeding the word through `add_decoder_status`,
    /// the production funnel. The end-to-end payload path is measured separately
    /// in `world_fold::tests::the_capture_boundary_rejects_an_unassigned_status_bit`.
    fn refusal_for(domain_id: u16, status_flags: u32) -> Option<String> {
        let mut summary = DomainDataDecodeSummary {
            domain_id,
            ..DomainDataDecodeSummary::default()
        };
        add_decoder_status(&mut summary, status_flags);
        reject_unassigned_status_bits(&mut summary, domain_id, 4096);
        summary.parse_errors.first().cloned()
    }

    /// A lane's DECLARED private badge is not a wire disagreement, even though
    /// it sits outside the v1 vocabulary.
    ///
    /// d53 publishes bits 16..=22 on 379 of 514 hours of batch-3a, and d60's
    /// bit 16 is MANDATORY on any frame carrying war rows. A gate spelled
    /// `compact_status_unknown_flags_v1(raw)` instead of
    /// `compact_status_unassigned_flags(domain_id, raw)` would refuse every one
    /// of those records — over-rejection that destroys real data rather than
    /// protecting it.
    ///
    /// What production change makes this fail: dropping the
    /// `compact_status_for_global_interpretation` strip from
    /// `compact_status_unassigned_flags`, or removing a domain's row from
    /// `lane_local_status_mask`.
    #[test]
    fn a_declared_lane_local_badge_is_not_refused() {
        for (domain_id, mask) in [
            (49u16, compact_contract::DOMAIN49_LANE_LOCAL_STATUS_MASK),
            (51, compact_contract::DOMAIN51_LANE_LOCAL_STATUS_MASK),
            (53, compact_contract::DOMAIN53_LANE_LOCAL_STATUS_MASK),
            (60, compact_contract::DOMAIN60_LANE_LOCAL_STATUS_MASK),
            (
                99,
                compact_contract::division_combat_stats::DOMAIN99_LANE_LOCAL_STATUS_MASK,
            ),
        ] {
            assert_ne!(mask, 0, "domain {domain_id} must declare a lane-local mask");
            // The whole mask at once, and each bit alone: a gate that tolerated
            // the union but not a single bit would pass the first form only.
            assert_eq!(
                refusal_for(domain_id, mask),
                None,
                "domain {domain_id}: its own declared badge mask {mask:#x} must not be refused"
            );
            for bit in 0..u32::BITS {
                let flag = 1_u32 << bit;
                if (mask & flag) == 0 {
                    continue;
                }
                assert_eq!(
                    refusal_for(domain_id, flag),
                    None,
                    "domain {domain_id}: declared lane-local bit {bit} must not be refused"
                );
                // The SAME bit on a domain that has not declared it is a
                // disagreement, whenever the bit is outside the v1 vocabulary.
                if compact_contract::compact_status_unknown_flags_v1(flag) != 0 {
                    assert!(
                        refusal_for(17, flag).is_some(),
                        "bit {bit} is lane-local to domain {domain_id} only; d17 has not \
                         declared it and must refuse it"
                    );
                }
            }
        }
    }

    /// d17's `WAR_SEGMENT_ABSENT` is ASSIGNED and informational, and production
    /// sets it on every diplomacy record.
    ///
    /// It is deliberately absent from `COMPACT_STATUS_FAILURE_MASK`, so a gate
    /// that asked "is this bit a known FAILURE?" rather than "is this bit
    /// DEFINED?" would refuse the entire d17 domain in every capture ever taken.
    #[test]
    fn d17_war_segment_absent_is_assigned_and_never_refused() {
        assert_eq!(
            compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT
                & compact_contract::COMPACT_STATUS_FAILURE_MASK,
            0,
            "the premise: it is informational, not a failure bit"
        );
        assert_eq!(
            refusal_for(17, compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT),
            None
        );
        assert_eq!(
            refusal_for(
                17,
                compact_contract::COMPACT_STATUS_WAR_SEGMENT_ABSENT
                    | compact_contract::COMPACT_STATUS_WAR_BRIDGE_APPLIED
                    | compact_contract::COMPACT_STATUS_PARTIAL_DATA,
            ),
            None,
            "nor in the company production actually pairs it with"
        );
    }

    /// Every unassigned bit is refused on every dispatched domain, and the
    /// reason NAMES the offending bits.
    ///
    /// The naming is the point: the refusal string is what lands verbatim in
    /// `ExcludedDomainDataRecord::validation_reason`, so it is the only place
    /// the capture ledger can say WHICH bit disagreed.
    ///
    /// What production change makes this fail: removing the
    /// `reject_unassigned_status_bits` call, or reducing its message to a bare
    /// "unknown status".
    #[test]
    fn every_unassigned_bit_is_refused_and_named() {
        for bit in 0..u32::BITS {
            let flag = 1_u32 << bit;
            let refusal = refusal_for(17, flag);
            if compact_contract::compact_status_unknown_flags_v1(flag) == 0 {
                assert_eq!(refusal, None, "assigned bit {bit} must not be refused");
                continue;
            }
            let reason = refusal.unwrap_or_else(|| panic!("unassigned bit {bit} must be refused"));
            assert!(
                reason.contains(&format!("{flag:#x}")),
                "bit {bit}: the reason must carry the offending mask: {reason}"
            );
            assert!(
                reason.contains(&format!("bit {bit}")),
                "bit {bit}: the reason must name the bit: {reason}"
            );
            assert!(
                reason.contains("offset 4096"),
                "bit {bit}: the reason must locate the record: {reason}"
            );
        }
    }

    /// Several offending bits at once are ALL named, not just the lowest.
    #[test]
    fn a_multi_bit_disagreement_names_every_offending_bit() {
        let reason = refusal_for(
            17,
            compact_contract::COMPACT_STATUS_PARTIAL_DATA | (1 << 16) | (1 << 20) | (1 << 31),
        )
        .expect("three unassigned bits must be refused");
        for bit in [16u32, 20, 31] {
            assert!(reason.contains(&format!("bit {bit}")), "{reason}");
        }
        assert!(
            reason.contains("0x80110000"),
            "the mask must be the union of exactly the offending bits: {reason}"
        );
        assert!(
            !reason.contains("bit 9"),
            "an assigned bit riding along must not be reported as offending: {reason}"
        );
    }

    /// Every decoder that reads a status word must route it through
    /// `add_decoder_status`, or the single gate in
    /// `summarize_domain_data_payload` silently stops covering that domain.
    ///
    /// This is what makes "one mechanism at the shared boundary" a property
    /// rather than a claim: the gate reads `summary.raw_status_flags_or`, and
    /// `add_decoder_status` is the only writer of it.
    ///
    /// d93 (`decode_compact_country_army_upgrade_data_payload`) was exactly the
    /// bypass this catches — it wrote `decoded.status_flags` into the per-domain
    /// copy only, leaving `DomainDataAuthority::status_flags` as `None` for the
    /// domain. Fixed in this lane; this test is what stops the next one.
    ///
    /// What production change makes this fail: adding a decoder that writes
    /// `domain.status_flags_or |= ...` without calling `add_decoder_status`.
    #[test]
    fn every_decoder_status_word_reaches_the_shared_funnel() {
        let source = include_str!("domain_data_authority.rs").replace("\r\n", "\n");
        let production = source
            .split_once("\n#[cfg(test)]\nmod tests {")
            .expect("the test module marker moved")
            .0;

        assert_eq!(
            production.matches("raw_status_flags_or = ").count(),
            1,
            "`add_decoder_status` must remain the ONLY writer of the accumulator the gate reads"
        );
        assert!(
            production.contains("reject_unassigned_status_bits(&mut summary, domain_id, offset);"),
            "the gate must still be wired into `summarize_domain_data_payload`"
        );

        let mut current: Option<&str> = None;
        let mut body = String::new();
        let mut offenders = Vec::new();
        let mut check = |name: Option<&str>, body: &str, offenders: &mut Vec<String>| {
            let Some(name) = name else { return };
            if !body.contains("status_flags_or |=") {
                return;
            }
            if body.contains("add_decoder_status(") || body.contains("add_compact_global_status_flags(") {
                return;
            }
            offenders.push(name.to_string());
        };
        for line in production.lines() {
            if line.starts_with("fn ")
                || line.starts_with("pub fn ")
                || line.starts_with("pub(crate) fn ")
            {
                check(current, &body, &mut offenders);
                current = Some(line.trim_end_matches('(').trim_end());
                body.clear();
            }
            body.push_str(line);
            body.push('\n');
        }
        check(current, &body, &mut offenders);

        assert!(
            offenders.is_empty(),
            "these decoders record a status word without routing it through the shared funnel, \
             so the unassigned-bit gate does not see it: {offenders:#?}"
        );
    }
}
