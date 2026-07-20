use std::collections::{BTreeMap, BTreeSet, HashSet};

pub const DOMAIN_ID_PROPOSED: u16 = 53;
pub const DOMAIN_NAME: &str = "diplomacy_intel_operatives_live";
pub const RESERVED_OCCUPIED_DOMAIN_IDS: &[(u16, &str)] = &[
    (44, "supply_logistics_attrition_live"),
    (45, "focus_research_decisions_depth_live"),
    (46, "air_live"),
    (47, "characters_leaders_cabinet_live"),
    (48, "construction_buildings_state_damage_live"),
    (49, "naval_combat_live"),
    (50, "naval_combat_live_aux"),
    (51, "scripted_state_live"),
    (52, "occupation_resistance_compliance_live"),
];
pub const SCHEMA_VERSION: u32 = 1;
pub const HEADER_LEN: usize = 72;
pub const FIXED_SCALE_RAW: u32 = 100_000;

pub const STATUS_STATE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_STATE_UNAVAILABLE;
pub const STATUS_BRIDGE_UNAVAILABLE: u32 = compact_contract::COMPACT_STATUS_BRIDGE_UNAVAILABLE;
pub const STATUS_PROFILE_MISMATCH: u32 = compact_contract::COMPACT_STATUS_PROFILE_MISMATCH;
pub const STATUS_NO_CLOCK: u32 = compact_contract::COMPACT_STATUS_NO_CLOCK;
pub const STATUS_NO_SCALAR_SOURCE: u32 = compact_contract::COMPACT_STATUS_NO_SCALAR_SOURCE;
pub const STATUS_EXTRACTION_PERF_GATE: u32 = compact_contract::COMPACT_STATUS_EXTRACTION_PERF_GATE;
pub const STATUS_WORKER_PANIC: u32 = compact_contract::COMPACT_STATUS_WORKER_PANIC;
pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_TRANSITION_BASELINE_RESET: u32 =
    compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET;
pub const STATUS_CONTROL_INVALID: u32 = compact_contract::COMPACT_STATUS_CONTROL_INVALID;
pub const STATUS_OPERATIONS_DETAIL_RESISTED: u32 = 1 << 16;
pub const STATUS_INTEL_MAP_RESISTED: u32 = 1 << 17;
pub const STATUS_WAR_ROOT_UNVALIDATED: u32 = 1 << 18;
pub const STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED: u32 = 1 << 19;
pub const STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED: u32 = 1 << 20;
pub const STATUS_FACTION_GROUP_UNVALIDATED: u32 = 1 << 21;
pub const STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED: u32 = 1 << 22;
pub const ROW_ID_DELETED: u32 = 1 << 31;

pub const KNOWN_STATUS_FLAGS: u32 = STATUS_STATE_UNAVAILABLE
    | STATUS_BRIDGE_UNAVAILABLE
    | STATUS_PROFILE_MISMATCH
    | STATUS_NO_CLOCK
    | STATUS_NO_SCALAR_SOURCE
    | STATUS_EXTRACTION_PERF_GATE
    | STATUS_WORKER_PANIC
    | STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_DELTA_DATA
    | STATUS_TRANSITION_BASELINE_RESET
    | STATUS_CONTROL_INVALID
    | STATUS_OPERATIONS_DETAIL_RESISTED
    | STATUS_INTEL_MAP_RESISTED
    | STATUS_WAR_ROOT_UNVALIDATED
    | STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED
    | STATUS_FACTION_GROUP_UNVALIDATED
    | STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;

pub const KIND_COUNTRY_DIPLOMACY: u16 = 1;
pub const KIND_WAR_RELATION: u16 = 2;
pub const KIND_OPINION: u16 = 3;
pub const KIND_RELATION_FLAG: u16 = 4;
pub const KIND_SUBJECT: u16 = 5;
pub const KIND_AGENCY: u16 = 6;
pub const KIND_OPERATIVE: u16 = 7;
pub const KIND_CRYPTOLOGY: u16 = 8;
pub const KIND_OPERATION_ANCHOR: u16 = 9;
pub const KIND_INTEL_MAP: u16 = 10;
pub const KIND_AGENCY_CAPACITY_CLUSTER: u16 = 11;
pub const KIND_COMPACT_STATE_EVENT: u16 = 12;

pub const COUNTRY_FLAG_AT_WAR: u16 = 1;
pub const COUNTRY_FLAG_CASUALTIES: u16 = 1 << 1;
pub const COUNTRY_FLAG_FACTION_GROUP: u16 = 1 << 2;

pub const REL_FLAG_DIRECTED: u32 = 1;
pub const REL_KIND_NON_AGGRESSION_PACT: u16 = 1;
pub const REL_KIND_PUPPET: u16 = 2;
pub const REL_KIND_EMBARGO: u16 = 3;
pub const REL_KIND_MILITARY_ACCESS: u16 = 4;
pub const REL_KIND_AIR_BASE_ACCESS: u16 = 5;
pub const REL_KIND_DOCKING_RIGHTS: u16 = 6;
pub const REL_KIND_NAVAL_BLOCKADE: u16 = 7;
pub const REL_KIND_MARKET_ACCESS_RIGHTS: u16 = 8;
pub const REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION: u16 = 9;
pub const REL_KIND_GUARANTEE: u16 = 10;
pub const REL_KIND_IMPROVE_RELATIONS: u16 = 11;
pub const REL_KIND_ATTACHE: u16 = 12;
pub const REL_KIND_LICENCE_PRODUCTION: u16 = 13;

pub const AGENCY_FLAG_NAME_HASH: u16 = 1;
pub const AGENCY_FLAG_ICON_HASH: u16 = 1 << 1;
pub const AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4: u16 = 1;

pub const OPERATIVE_FLAG_DETECTED: u16 = 1;
pub const OPERATIVE_FLAG_SKILL_PRESENT: u16 = 1 << 1;
pub const OPERATIVE_FLAG_EXPERIENCE_PRESENT: u16 = 1 << 2;
pub const OPERATIVE_FLAG_NAME_HASH: u16 = 1 << 3;
pub const OPERATIVE_FLAG_CAPTURED: u16 = 1 << 4;
pub const OPERATIVE_FLAG_LIFECYCLE_PRESENT: u16 = 1 << 5;

pub const CRYPTO_FLAG_DECRYPTING: u16 = 1;

pub const OPERATION_FLAG_RUNNING: u16 = 1;
pub const OPERATION_FLAG_DETAIL_RESISTED: u16 = 1 << 1;
pub const OPERATION_FLAG_TARGET_PRESENT: u16 = 1 << 2;

pub const INTEL_MAP_FLAG_RESISTED: u16 = 1;

pub const STATE_EVENT_FLAG_TARGET_PRESENT: u16 = 1;
pub const STATE_EVENT_KIND_OBSERVED: u16 = 1;
pub const STATE_EVENT_SOURCE_RELATION_FLAG: u16 = 1;
pub const STATE_EVENT_SOURCE_SUBJECT_AUTONOMY: u16 = 2;
pub const STATE_EVENT_SOURCE_AGENCY_SCALAR: u16 = 3;
pub const STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE: u16 = 4;
pub const STATE_EVENT_SOURCE_AGENCY_CAPACITY: u16 = 5;
pub const STATE_EVENT_SOURCE_CRYPTOLOGY: u16 = 6;
pub const STATE_EVENT_SOURCE_RELATION_OPINION: u16 = 7;
pub const STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP: u16 = 8;
const STATE_EVENT_OBJECT_RELATION_OPINION_HASH: u32 = 0x570E_9D68;

pub const MAX_TAG_DB_ID: u32 = compact_contract::build_profile::MAX_COMPACT_OWNER_TAG_DB_ID;
pub const MAX_CASUALTIES: i64 = 50_000_000;
pub const MAX_OPINION_ABS: i32 = 200;
pub const MAX_OPERATIVES_PER_AGENCY: u32 = 128;
pub const MAX_AGENCY_CAPACITY_CLUSTER_VALUE: u32 = 128;
pub const MAX_OPERATIVE_ID: u32 = i32::MAX as u32;
pub const MAX_OPERATIVE_SKILL: u32 = 15;
pub const MAX_OPERATIVE_LIFECYCLE_FLAG: u32 = 1024;
pub const MAX_FIXED_RAW: i32 = 1_000_000_000;
pub const MAX_XP_RAW: i64 = 1_000_000_000_000;
pub const MAX_CRYPTO_AMOUNT_RAW: i64 = 1_000_000_000_000;
pub const MAX_OPERATION_ID: u32 = i32::MAX as u32;
pub const MAX_OPERATION_SLOTS: u32 = 16;
pub const MAX_OPERATION_PHASES: u32 = 32;
pub const MAX_INTEL_PERCENT_RAW: i32 = 100_000;
pub const MAX_INTEL_NETWORK_ROWS: usize = 4_096;
pub const MAX_INTEL_NETWORK_STATES: usize = 2_048;
pub const MAX_INTEL_NETWORK_STATE_ID: u32 = compact_contract::build_profile::MAX_STATE_ID;
pub const MAX_STATE_EVENT_VALUE_RAW: i64 = 1_000_000_000_000;

const ROW_COUNTRY_LEN: usize = 24;
const ROW_WAR_LEN: usize = 32;
const ROW_OPINION_LEN: usize = 16;
const ROW_RELATION_FLAG_LEN: usize = 16;
const ROW_SUBJECT_LEN: usize = 24;
const ROW_AGENCY_LEN: usize = 48;
const ROW_AGENCY_CAPACITY_CLUSTER_LEN: usize = 24;
const ROW_OPERATIVE_LEN: usize = 40;
const ROW_CRYPTOLOGY_LEN: usize = 32;
const ROW_OPERATION_ANCHOR_LEN: usize = 32;
const ROW_INTEL_MAP_LEN: usize = 32;
const ROW_COMPACT_STATE_EVENT_LEN: usize = 32;

// Wave-5 running-operations `OPIN` trailer (tail-appended after the base
// self-delimiting payload; MTD1/ACE1/PLIC/PLCF precedent).
const OPERATION_INSTANCE_TRAILER_MAGIC: &[u8; 4] = b"OPIN";
const OPERATION_INSTANCE_TRAILER_VERSION_V1: u16 = 1;
const OPERATION_INSTANCE_TRAILER_VERSION_V2: u16 = 2;
const OPERATION_INSTANCE_TRAILER_VERSION: u16 = 3;
const OPERATION_INSTANCE_TRAILER_HEADER_LEN: usize = 12;
const OPERATION_INSTANCE_TRAILER_V1_ROW_LEN: usize = 12;
const OPERATION_INSTANCE_TRAILER_V2_ROW_HEADER_LEN: usize = 48;
const OPERATION_INSTANCE_FLAG_DEF_REF_OK: u32 = 1;
const OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT: u32 = 1 << 1;
const OPERATION_INSTANCE_V2_FLAG_STATE_TARGET: u8 = 1;
pub const MAX_OPERATION_INSTANCE_ROWS: usize = 4_096;
pub const MAX_OPERATION_DEFINITION_ID_LEN: usize = 256;
pub const MAX_OPERATION_ASSIGNED_OPERATIVES: usize = 64;

const INCOMING_ACTION_QUEUE_TRAILER_MAGIC: &[u8; 4] = b"DIAQ";
const INCOMING_ACTION_QUEUE_TRAILER_VERSION: u16 = 1;
const INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN: usize = 12;
const INCOMING_ACTION_QUEUE_TRAILER_ROW_LEN: usize = 24;
pub const MAX_INCOMING_ACTION_QUEUE_ROWS: usize = 4_096;
pub const MAX_ACTION_DATE_HOURS: u32 = 100_000_000;

const INTEL_NETWORK_GRAPH_TRAILER_MAGIC: &[u8; 4] = b"INGR";
const INTEL_NETWORK_GRAPH_TRAILER_VERSION_V1: u16 = 1;
const INTEL_NETWORK_GRAPH_TRAILER_VERSION_V2: u16 = 2;
const INTEL_NETWORK_GRAPH_TRAILER_VERSION: u16 = 3;
const INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN: usize = 12;
const INTEL_NETWORK_GRAPH_V1_ROW_HEADER_LEN: usize = 16;
const INTEL_NETWORK_GRAPH_V2_ROW_HEADER_LEN: usize = 88;
const INTEL_NETWORK_GRAPH_V3_ROW_HEADER_LEN: usize = 92;
const INTEL_NETWORK_GRAPH_V3_FLAG_COVERAGE: u32 = 1 << 0;

// `FACS` faction-roster trailer (tail-appended after the base self-delimiting
// payload; DIAQ/INGR precedent). Per faction: a fixed row header + a
// variable-length sorted member tag_db_id list. Version 1 was emitted only for
// a non-empty roster. Version 2 is an explicit completeness witness and is
// therefore emitted with row_count == 0 for a positively measured empty roster.
const FACTION_TRAILER_MAGIC: &[u8; 4] = b"FACS";
const FACTION_TRAILER_VERSION_V1_NONEMPTY: u16 = 1;
const FACTION_TRAILER_VERSION: u16 = 2;
const FACTION_TRAILER_HEADER_LEN: usize = 12;
// row header: faction_id u32, faction_type u32, name_hash u32,
//             leader_tag_db_id u32, member_count u32
const FACTION_ROW_HEADER_LEN: usize = 20;
pub const MAX_FACTION_ROWS: usize = 4_096;
pub const MAX_FACTION_MEMBERS: usize = 256;

// A16 exact live autonomy-currentness overlay. The additive trailer preserves
// the established 24-byte CSubjectRelation base row and lets current readers
// accept old recordings. Legacy strict readers do not understand AUST.
const SUBJECT_AUTONOMY_STATUS_TRAILER_MAGIC: &[u8; 4] = b"AUST";
const SUBJECT_AUTONOMY_STATUS_TRAILER_VERSION: u16 = 1;
const SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN: usize = 12;
const SUBJECT_AUTONOMY_STATUS_TRAILER_ROW_LEN: usize = 56;
const SUBJECT_AUTONOMY_STATUS_FLAG_PREVIOUS: u32 = 1;
const SUBJECT_AUTONOMY_STATUS_FLAG_NEXT: u32 = 1 << 1;
pub const MAX_SUBJECT_AUTONOMY_STATUS_ROWS: usize = 4_096;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiplomacyIntelSnapshot {
    pub elapsed_wall_ms: u64,
    pub status_flags: u32,
    pub changed_rows: Option<u32>,
    pub countries: Vec<CountryDiplomacyRow>,
    pub wars: Vec<WarRelationRow>,
    pub opinions: Vec<OpinionRow>,
    pub relation_flags: Vec<RelationFlagRow>,
    pub subjects: Vec<SubjectRow>,
    /// Exact `CCurrentAutonomyStatus` rows. On wire these are an AUST v1
    /// additive trailer and are admitted only with a complete atomic witness.
    pub subject_autonomy_statuses: Vec<SubjectAutonomyStatusRow>,
    pub subject_autonomy_status_complete: bool,
    pub agencies: Vec<AgencyRow>,
    pub agency_capacity_clusters: Vec<AgencyCapacityClusterRow>,
    pub operatives: Vec<OperativeRow>,
    pub cryptology: Vec<CryptologyRow>,
    pub operation_anchors: Vec<OperationAnchorRow>,
    pub intel_map: Vec<IntelMapRow>,
    pub state_events: Vec<StateEventRow>,
    /// Wave-5 running-operation instances (COperationInstance {instance_id,
    /// def_ref_ok, associated_country}). Carried in the `OPIN` trailer AFTER
    /// the self-delimiting base payload — NOT counted in `row_count()` (the
    /// base row loop must not see them). Absent trailer => empty (not measured).
    pub operation_instances: Vec<OperationInstanceRow>,
    pub incoming_actions: Vec<IncomingActionQueueRow>,
    pub intel_network_graphs: Vec<IntelNetworkGraphRow>,
    /// Factions from the CFactionSystem. Carried in the `FACS` trailer AFTER the
    /// self-delimiting base payload — NOT counted in `row_count()`. Absent
    /// trailer => empty.
    pub factions: Vec<FactionRow>,
    /// True only when the runtime CFactionSystem roster read was complete and
    /// double-read stable. On the wire this is FACS v2, including row_count 0.
    /// An absent FACS trailer decodes false; legacy FACS v1 with rows decodes
    /// true to preserve its established non-empty completeness semantics.
    pub faction_roster_complete: bool,
}

impl DiplomacyIntelSnapshot {
    pub fn row_count(&self) -> u32 {
        [
            self.countries.len(),
            self.wars.len(),
            self.opinions.len(),
            self.relation_flags.len(),
            self.subjects.len(),
            self.agencies.len(),
            self.agency_capacity_clusters.len(),
            self.operatives.len(),
            self.cryptology.len(),
            self.operation_anchors.len(),
            self.intel_map.len(),
            self.state_events.len(),
        ]
        .iter()
        .fold(0u32, |acc, count| {
            acc.saturating_add((*count).min(u32::MAX as usize) as u32)
        })
    }
}

fn encoded_payload_capacity(snapshot: &DiplomacyIntelSnapshot) -> usize {
    let base = HEADER_LEN
        .saturating_add(snapshot.countries.len().saturating_mul(ROW_COUNTRY_LEN))
        .saturating_add(snapshot.wars.len().saturating_mul(ROW_WAR_LEN))
        .saturating_add(snapshot.opinions.len().saturating_mul(ROW_OPINION_LEN))
        .saturating_add(
            snapshot
                .relation_flags
                .len()
                .saturating_mul(ROW_RELATION_FLAG_LEN),
        )
        .saturating_add(snapshot.subjects.len().saturating_mul(ROW_SUBJECT_LEN))
        .saturating_add(snapshot.agencies.len().saturating_mul(ROW_AGENCY_LEN))
        .saturating_add(
            snapshot
                .agency_capacity_clusters
                .len()
                .saturating_mul(ROW_AGENCY_CAPACITY_CLUSTER_LEN),
        )
        .saturating_add(snapshot.operatives.len().saturating_mul(ROW_OPERATIVE_LEN))
        .saturating_add(snapshot.cryptology.len().saturating_mul(ROW_CRYPTOLOGY_LEN))
        .saturating_add(
            snapshot
                .operation_anchors
                .len()
                .saturating_mul(ROW_OPERATION_ANCHOR_LEN),
        )
        .saturating_add(snapshot.intel_map.len().saturating_mul(ROW_INTEL_MAP_LEN))
        .saturating_add(
            snapshot
                .state_events
                .len()
                .saturating_mul(ROW_COMPACT_STATE_EVENT_LEN),
        );
    base.saturating_add(operation_instance_trailer_capacity(
        &snapshot.operation_instances,
    ))
    .saturating_add(incoming_action_queue_trailer_capacity(
        &snapshot.incoming_actions,
    ))
    .saturating_add(intel_network_graph_trailer_capacity(
        &snapshot.intel_network_graphs,
    ))
    .saturating_add(faction_trailer_capacity(
        &snapshot.factions,
        snapshot.faction_roster_complete,
    ))
    .saturating_add(subject_autonomy_status_trailer_capacity(
        &snapshot.subject_autonomy_statuses,
        snapshot.subject_autonomy_status_complete,
    ))
}

fn operation_instance_trailer_capacity(rows: &[OperationInstanceRow]) -> usize {
    if rows.is_empty() {
        0
    } else if rows.iter().all(OperationInstanceRow::is_exact_runtime) {
        OPERATION_INSTANCE_TRAILER_HEADER_LEN.saturating_add(rows.iter().fold(
            0usize,
            |sum, row| {
                sum.saturating_add(OPERATION_INSTANCE_TRAILER_V2_ROW_HEADER_LEN)
                    .saturating_add(row.definition_id.as_ref().map_or(0, String::len))
                    .saturating_add(row.assigned_operative_ids.len().saturating_mul(4))
            },
        ))
    } else {
        OPERATION_INSTANCE_TRAILER_HEADER_LEN.saturating_add(
            rows.len()
                .saturating_mul(OPERATION_INSTANCE_TRAILER_V1_ROW_LEN),
        )
    }
}

fn incoming_action_queue_trailer_capacity(rows: &[IncomingActionQueueRow]) -> usize {
    if rows.is_empty() {
        0
    } else {
        INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN.saturating_add(
            rows.len()
                .saturating_mul(INCOMING_ACTION_QUEUE_TRAILER_ROW_LEN),
        )
    }
}

fn intel_network_graph_trailer_capacity(rows: &[IntelNetworkGraphRow]) -> usize {
    if rows.is_empty() {
        0
    } else {
        rows.iter()
            .fold(INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN, |acc, row| {
                let row_header_len = if row.state_strengths.is_some() {
                    INTEL_NETWORK_GRAPH_V3_ROW_HEADER_LEN
                } else if row.coverage.is_some() {
                    INTEL_NETWORK_GRAPH_V2_ROW_HEADER_LEN
                } else {
                    INTEL_NETWORK_GRAPH_V1_ROW_HEADER_LEN
                };
                let state_len = if row.state_strengths.is_some() { 12 } else { 4 };
                acc.saturating_add(row_header_len)
                    .saturating_add(row.state_ids.len().saturating_mul(state_len))
            })
    }
}

fn faction_trailer_capacity(rows: &[FactionRow], roster_complete: bool) -> usize {
    if rows.is_empty() && !roster_complete {
        0
    } else {
        rows.iter().fold(FACTION_TRAILER_HEADER_LEN, |acc, row| {
            acc.saturating_add(FACTION_ROW_HEADER_LEN)
                .saturating_add(row.member_tag_db_ids.len().saturating_mul(4))
        })
    }
}

pub fn occupied_domain_name(domain_id: u16) -> Option<&'static str> {
    RESERVED_OCCUPIED_DOMAIN_IDS
        .iter()
        .find_map(|(reserved_id, name)| (*reserved_id == domain_id).then_some(*name))
}

pub fn domain_id_rejection_reason(domain_id: u16) -> Option<String> {
    if domain_id == DOMAIN_ID_PROPOSED {
        None
    } else if let Some(name) = occupied_domain_name(domain_id) {
        Some(format!("domain {domain_id} is occupied by {name}"))
    } else {
        Some(format!(
            "domain {domain_id} is not reserved for {DOMAIN_NAME}; expected {DOMAIN_ID_PROPOSED}"
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CountryDiplomacyRow {
    pub tag_db_id: u32,
    pub faction_group: i32,
    pub at_war_count: u32,
    pub casualties: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WarRelationRow {
    pub first_tag_db_id: u32,
    pub second_tag_db_id: u32,
    pub first_casualties: i64,
    pub second_casualties: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpinionRow {
    pub src_tag_db_id: u32,
    pub target_tag_db_id: u32,
    pub opinion: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationFlagRow {
    pub src_tag_db_id: u32,
    pub target_tag_db_id: u32,
    pub flag_kind: u16,
    pub directed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectRow {
    pub overlord_tag_db_id: u32,
    pub subject_tag_db_id: u32,
    pub autonomy_state_hash: u32,
    /// `CSubjectRelation+0x60` is the serialized i64
    /// `set_autonomy.freedom_level` scalar. The extractor validates it in
    /// `0..=100000` before lossless i32 narrowing. This legacy field is not
    /// sufficient mutable progress; A16 owns the separate
    /// `CCurrentAutonomyStatus` current-score/bounds migration.
    pub autonomy_ratio_raw: i32,
    /// `CSubjectRelation+0x20` i32 engine-hour subject start_date (RAW). The
    /// replay layer derives the display date. Live-validated 38/38 on 870ebf.
    pub start_date_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubjectAutonomyStatusRow {
    pub overlord_tag_db_id: u32,
    pub subject_tag_db_id: u32,
    pub current_state_hash: u32,
    pub previous_state_hash: Option<u32>,
    pub next_state_hash: Option<u32>,
    pub ratio_raw: i32,
    pub current_score_raw: i64,
    pub lower_score_raw: i64,
    pub upper_score_raw: i64,
    pub last_modified_date_raw: u32,
}

/// A faction from the CFactionSystem (`CGameState+0x3F8`). Members are carried
/// as a sorted list of country tag_db_ids; the leader is `members[0]` in engine
/// order (captured separately because the emitted member list is sorted).
/// Live-validated 6/6 with exact member multisets + leaders on 870ebf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactionRow {
    pub faction_id: u32,
    pub faction_type: u32,
    pub name_hash: u32,
    pub leader_tag_db_id: u32,
    pub member_tag_db_ids: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyRow {
    pub owner_tag_db_id: u32,
    pub max_operatives: u32,
    pub usable_slots: u32,
    pub operative_count: u32,
    pub cryptology_target_count: u32,
    pub defense_raw: i32,
    pub upgrade_progress_raw: i32,
    pub name_hash: u64,
    pub icon_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyCapacityClusterRow {
    pub owner_tag_db_id: u32,
    pub value_0xe4: u32,
    pub value_0xf0: u32,
    pub value_0xf4: u32,
    pub proof_hash: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperativeRow {
    pub owner_tag_db_id: u32,
    pub op_id: u32,
    pub detected: bool,
    pub captured: bool,
    pub skill: Option<u32>,
    pub experience_raw: Option<i64>,
    pub name_hash: u64,
    pub state_hash: u32,
    pub lifecycle_flag: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CryptologyRow {
    pub owner_tag_db_id: u32,
    pub target_tag_db_id: u32,
    pub amount_raw: i64,
    pub decrypting: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationAnchorRow {
    pub owner_tag_db_id: u32,
    pub operation_id: u32,
    pub target_tag_db_id: Option<u32>,
    pub slot_count: u32,
    pub phase_count: u32,
    pub detail_resisted: bool,
}

/// Exact GH03 operation-instance runtime status. OPIN v1/v2 remain decodable,
/// while OPIN v3 preserves the distinct prepared state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationInstanceStatus {
    Preparing,
    Prepared,
    Running,
    Finished,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationInstanceRow {
    pub instance_id: u32,
    /// OPIN-v1 compatibility witness. Always true on exact v2/v3 rows.
    pub def_ref_ok: bool,
    /// OPIN-v1 compatibility value. On exact v2/v3 rows this is the authored
    /// target-country CId at +0x58, never the owner pointer at +0x50.
    pub associated_country_tag_db_id: Option<u32>,
    pub definition_id: Option<String>,
    pub owner_country_tag_db_id: Option<u32>,
    pub target_country_tag_db_id: Option<u32>,
    pub target_state_id: Option<u32>,
    pub status: Option<OperationInstanceStatus>,
    pub phase_index: u32,
    pub phase_count: u32,
    pub progress_fixed: Option<i64>,
    pub start_hour: Option<u32>,
    pub duration_hours: Option<u32>,
    pub assigned_operative_ids: Vec<u32>,
}

impl OperationInstanceRow {
    pub fn legacy(
        instance_id: u32,
        def_ref_ok: bool,
        associated_country_tag_db_id: Option<u32>,
    ) -> Self {
        Self {
            instance_id,
            def_ref_ok,
            associated_country_tag_db_id,
            definition_id: None,
            owner_country_tag_db_id: None,
            target_country_tag_db_id: None,
            target_state_id: None,
            status: None,
            phase_index: 0,
            phase_count: 0,
            progress_fixed: None,
            start_hour: None,
            duration_hours: None,
            assigned_operative_ids: Vec::new(),
        }
    }

    pub fn is_exact_runtime(&self) -> bool {
        self.definition_id.is_some()
            && self.owner_country_tag_db_id.is_some()
            && self.target_country_tag_db_id.is_some()
            && self.status.is_some()
            && self.phase_count != 0
            && self.progress_fixed.is_some()
            && self.start_hour.is_some()
            && self.duration_hours.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingActionQueueRow {
    pub recipient_tag_db_id: u32,
    pub actor_tag_db_id: u32,
    pub action_id: u32,
    pub action_type: i32,
    pub date_hours: u32,
    pub last_command_date_hours: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkGraphRow {
    pub strategic_operative_index: u32,
    pub network_slot: u32,
    pub target_tag_db_id: u32,
    /// Sorted unique state ids from CCountryIntelNetwork graph vertices.
    pub state_ids: Vec<u32>,
    /// Exact sorted per-state strengths. `None` is retained only for decoding
    /// legacy INGR v1/v2 payloads; current a729 extraction emits this atomically
    /// with `state_ids` under the fixed 100000 scale.
    pub state_strengths: Option<Vec<IntelNetworkStateStrength>>,
    pub coverage: Option<IntelNetworkCoverage>,
}

fn subject_autonomy_status_trailer_capacity(
    rows: &[SubjectAutonomyStatusRow],
    complete: bool,
) -> usize {
    if !complete {
        0
    } else {
        SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN
            .saturating_add(rows.len().saturating_mul(SUBJECT_AUTONOMY_STATUS_TRAILER_ROW_LEN))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkStateStrength {
    pub state_id: u32,
    pub strength_raw: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkCoverage {
    pub covered_core_states: u32,
    pub covered_controlled_states: u32,
    pub covered_worth: u32,
    pub controlled_strength_sum_raw: i64,
    pub core_strength_sum_raw: i64,
    pub total_core_states: u32,
    pub total_controlled_states: u32,
    pub total_worth: u32,
    pub weight_controlled_raw: i64,
    pub weight_owned_raw: i64,
    pub weight_owned_worth_raw: i64,
    pub national_coverage_raw: i64,
}

fn recompute_intel_network_national_coverage(value: &IntelNetworkCoverage) -> Option<i64> {
    fn ratio(count: u32, total: u32, weight: i64) -> Option<i64> {
        if total == 0 {
            return Some(0);
        }
        i64::try_from(i128::from(count) * i128::from(weight) / i128::from(total)).ok()
    }
    fn average(sum: i64, count: u32) -> Option<i64> {
        if count == 0 {
            return Some(0);
        }
        i64::try_from(i128::from(sum) / (i128::from(count) * 100)).ok()
    }
    fn contribution(average: i64, weighted_ratio: i64) -> Option<i64> {
        i64::try_from(i128::from(average) * i128::from(weighted_ratio) / 100_000).ok()
    }
    let average_controlled = average(
        value.controlled_strength_sum_raw,
        value.covered_controlled_states,
    )?;
    let average_core = average(value.core_strength_sum_raw, value.covered_core_states)?;
    let controlled = contribution(
        average_controlled,
        ratio(
            value.covered_controlled_states,
            value.total_controlled_states,
            value.weight_controlled_raw,
        )?,
    )?;
    let owned = contribution(
        average_core,
        ratio(
            value.covered_core_states,
            value.total_core_states,
            value.weight_owned_raw,
        )?,
    )?;
    let owned_worth = contribution(
        average_controlled,
        ratio(
            value.covered_worth,
            value.total_worth,
            value.weight_owned_worth_raw,
        )?,
    )?;
    Some(
        controlled
            .saturating_add(owned)
            .saturating_add(owned_worth)
            .clamp(0, 100_000),
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelMapRow {
    pub owner_tag_db_id: u32,
    pub target_tag_db_id: u32,
    pub civilian_raw: i32,
    pub army_raw: i32,
    pub navy_raw: i32,
    pub air_raw: i32,
    pub resisted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StateEventRow {
    pub owner_tag_db_id: u32,
    pub target_tag_db_id: Option<u32>,
    pub event_kind: u16,
    pub source_kind: u16,
    pub object_hash: u32,
    pub value_hash: u32,
    pub value_raw: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53GateSummary {
    pub status_flags_or: u32,
    pub data_rows_total: u64,
    pub data_changed_rows_total: u64,
    pub data_units_total: BTreeMap<&'static str, u64>,
    pub parse_errors: Vec<String>,
}

pub fn encode_payload(snapshot: &DiplomacyIntelSnapshot) -> Vec<u8> {
    let row_count = snapshot.row_count();
    let changed_rows = snapshot.changed_rows.unwrap_or(row_count);
    let mut payload = Vec::with_capacity(encoded_payload_capacity(snapshot));
    payload.extend_from_slice(&snapshot.elapsed_wall_ms.to_le_bytes());
    payload.extend_from_slice(&row_count.to_le_bytes());
    payload.extend_from_slice(&changed_rows.to_le_bytes());
    payload.extend_from_slice(&snapshot.status_flags.to_le_bytes());
    payload.extend_from_slice(&SCHEMA_VERSION.to_le_bytes());
    payload.extend_from_slice(&FIXED_SCALE_RAW.to_le_bytes());
    payload.extend_from_slice(&(snapshot.countries.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.wars.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.opinions.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.relation_flags.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.subjects.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.agencies.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.operatives.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.cryptology.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.operation_anchors.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.intel_map.len() as u32).to_le_bytes());
    payload.extend_from_slice(&(snapshot.agency_capacity_clusters.len() as u32).to_le_bytes());

    for row in &snapshot.countries {
        let mut flags = 0u16;
        if !row_id_deleted(row.tag_db_id) {
            flags |= COUNTRY_FLAG_FACTION_GROUP;
            if row.at_war_count > 0 {
                flags |= COUNTRY_FLAG_AT_WAR;
            }
            if row.casualties != 0 {
                flags |= COUNTRY_FLAG_CASUALTIES;
            }
        }
        payload.extend_from_slice(&KIND_COUNTRY_DIPLOMACY.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.faction_group.to_le_bytes());
        payload.extend_from_slice(&row.at_war_count.to_le_bytes());
        payload.extend_from_slice(&row.casualties.to_le_bytes());
    }
    for row in &snapshot.wars {
        payload.extend_from_slice(&KIND_WAR_RELATION.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&row.first_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.second_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.first_casualties.to_le_bytes());
        payload.extend_from_slice(&row.second_casualties.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in &snapshot.opinions {
        payload.extend_from_slice(&KIND_OPINION.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&row.src_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.opinion.to_le_bytes());
    }
    for row in &snapshot.relation_flags {
        let flags = if row.directed { REL_FLAG_DIRECTED } else { 0 };
        payload.extend_from_slice(&KIND_RELATION_FLAG.to_le_bytes());
        payload.extend_from_slice(&row.flag_kind.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.src_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.to_le_bytes());
    }
    for row in &snapshot.subjects {
        payload.extend_from_slice(&KIND_SUBJECT.to_le_bytes());
        payload.extend_from_slice(&0u16.to_le_bytes());
        payload.extend_from_slice(&row.overlord_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.subject_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.autonomy_state_hash.to_le_bytes());
        payload.extend_from_slice(&row.autonomy_ratio_raw.to_le_bytes());
        // Trailing 4 bytes (formerly reserved 0) now carry the validated i32
        // engine-hour subject start_date. Row length is unchanged (24 bytes).
        payload.extend_from_slice(&row.start_date_raw.to_le_bytes());
    }
    for row in &snapshot.agencies {
        let mut flags = 0u16;
        if row.name_hash != 0 {
            flags |= AGENCY_FLAG_NAME_HASH;
        }
        if row.icon_hash != 0 {
            flags |= AGENCY_FLAG_ICON_HASH;
        }
        payload.extend_from_slice(&KIND_AGENCY.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.max_operatives.to_le_bytes());
        payload.extend_from_slice(&row.usable_slots.to_le_bytes());
        payload.extend_from_slice(&row.operative_count.to_le_bytes());
        payload.extend_from_slice(&row.cryptology_target_count.to_le_bytes());
        payload.extend_from_slice(&row.defense_raw.to_le_bytes());
        payload.extend_from_slice(&row.upgrade_progress_raw.to_le_bytes());
        payload.extend_from_slice(&row.name_hash.to_le_bytes());
        payload.extend_from_slice(&row.icon_hash.to_le_bytes());
    }
    for row in &snapshot.agency_capacity_clusters {
        let flags = if row_id_deleted(row.owner_tag_db_id) {
            0
        } else {
            AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4
        };
        payload.extend_from_slice(&KIND_AGENCY_CAPACITY_CLUSTER.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.value_0xe4.to_le_bytes());
        payload.extend_from_slice(&row.value_0xf0.to_le_bytes());
        payload.extend_from_slice(&row.value_0xf4.to_le_bytes());
        payload.extend_from_slice(&row.proof_hash.to_le_bytes());
    }
    for row in &snapshot.operatives {
        let mut flags = 0u16;
        if row.detected {
            flags |= OPERATIVE_FLAG_DETECTED;
        }
        if row.captured {
            flags |= OPERATIVE_FLAG_CAPTURED;
        }
        if row.skill.is_some() {
            flags |= OPERATIVE_FLAG_SKILL_PRESENT;
        }
        if row.experience_raw.is_some() {
            flags |= OPERATIVE_FLAG_EXPERIENCE_PRESENT;
        }
        if row.name_hash != 0 {
            flags |= OPERATIVE_FLAG_NAME_HASH;
        }
        if row.lifecycle_flag.is_some() {
            flags |= OPERATIVE_FLAG_LIFECYCLE_PRESENT;
        }
        payload.extend_from_slice(&KIND_OPERATIVE.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.op_id.to_le_bytes());
        payload.extend_from_slice(&row.skill.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.experience_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.name_hash.to_le_bytes());
        payload.extend_from_slice(&row.state_hash.to_le_bytes());
        payload.extend_from_slice(&row.lifecycle_flag.unwrap_or(0).to_le_bytes());
    }
    for row in &snapshot.cryptology {
        let flags = if row.decrypting {
            CRYPTO_FLAG_DECRYPTING
        } else {
            0
        };
        payload.extend_from_slice(&KIND_CRYPTOLOGY.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.amount_raw.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in &snapshot.operation_anchors {
        let mut flags = OPERATION_FLAG_RUNNING;
        if row.detail_resisted {
            flags |= OPERATION_FLAG_DETAIL_RESISTED;
        }
        if row.target_tag_db_id.is_some() {
            flags |= OPERATION_FLAG_TARGET_PRESENT;
        }
        payload.extend_from_slice(&KIND_OPERATION_ANCHOR.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.operation_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.slot_count.to_le_bytes());
        payload.extend_from_slice(&row.phase_count.to_le_bytes());
        payload.extend_from_slice(&0u64.to_le_bytes());
    }
    for row in &snapshot.intel_map {
        let flags = if row.resisted {
            INTEL_MAP_FLAG_RESISTED
        } else {
            0
        };
        payload.extend_from_slice(&KIND_INTEL_MAP.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.civilian_raw.to_le_bytes());
        payload.extend_from_slice(&row.army_raw.to_le_bytes());
        payload.extend_from_slice(&row.navy_raw.to_le_bytes());
        payload.extend_from_slice(&row.air_raw.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    for row in &snapshot.state_events {
        let mut flags = 0u16;
        if row.target_tag_db_id.is_some() {
            flags |= STATE_EVENT_FLAG_TARGET_PRESENT;
        }
        payload.extend_from_slice(&KIND_COMPACT_STATE_EVENT.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.event_kind.to_le_bytes());
        payload.extend_from_slice(&row.source_kind.to_le_bytes());
        payload.extend_from_slice(&row.object_hash.to_le_bytes());
        payload.extend_from_slice(&row.value_hash.to_le_bytes());
        payload.extend_from_slice(&row.value_raw.to_le_bytes());
    }
    append_operation_instance_trailer(&mut payload, &snapshot.operation_instances);
    append_incoming_action_queue_trailer(&mut payload, &snapshot.incoming_actions);
    append_intel_network_graph_trailer(&mut payload, &snapshot.intel_network_graphs);
    append_faction_trailer(
        &mut payload,
        &snapshot.factions,
        snapshot.faction_roster_complete,
    );
    append_subject_autonomy_status_trailer(
        &mut payload,
        &snapshot.subject_autonomy_statuses,
        snapshot.subject_autonomy_status_complete,
    );
    payload
}

/// Encode `snapshot` and produce the [`Domain53GateSummary`] that
/// `decode_payload(offset, &encode_payload(snapshot))` WOULD produce — WITHOUT
/// re-parsing the (potentially ~380k-row) base payload.
///
/// W2 lever: `decode_payload` re-parsing the just-encoded payload cost 29.7
/// ms/frame (29% of the d53 wall). `summarize_snapshot` replays the decoder's
/// gate/validation logic reading SNAPSHOT FIELDS for the expensive base-row loop,
/// and for the small append-only trailers RE-ENCODES only the trailer suffix and
/// runs the real [`decode_domain53_trailers`] on it — so trailer units and
/// parse-errors are byte-identical to `decode_payload` by construction (each
/// trailer decoder slices trailer-relative, so no absolute-offset strings leak,
/// and every byte-position-bearing trailer error is unreachable from
/// encoder-produced bytes).
///
/// GOVERNING INVARIANT (proven by tests + a 1/N sampled production cross-check):
/// `summarize_snapshot(offset, s) == decode_payload(offset, &encode_payload(&s))`
/// for every snapshot `s`, field-for-field including `parse_errors` order/strings
/// and the `data_units_total` map. Header/kind/truncation/count-mismatch checks in
/// `decode_payload` are UNREACHABLE on a valid encode and so never emitted here.
/// W2.1 measure-first split of `summarize_snapshot`'s internal cost (surfaced by
/// the collector as `domain53_seg_payload_summary_*_us`). Observational only —
/// populated by [`summarize_snapshot_instrumented`], never affects the gate
/// summary bytes, its equality, or its errors. These spans are SUB-components of
/// `summary_us`, NOT additive on top of it: `base_rows_us` covers the whole
/// 12-family base row loop; `opinions_us` and `state_events_us` are the two
/// ~191k families carved out of `base_rows_us` (the ones whose `seen_*` dedup
/// BTreeSets Codex/T2 bet dominate); `trailers_us` is the trailer re-encode +
/// real-decoder replay. The tail (status-vs-row block) = `summary_us` −
/// `base_rows_us` − `trailers_us` in analysis.
#[derive(Clone, Copy, Debug, Default)]
pub struct SummaryPhaseTimes {
    pub base_rows_us: u64,
    pub opinions_us: u64,
    pub state_events_us: u64,
    pub trailers_us: u64,
}

#[inline]
fn micros_u64(elapsed: std::time::Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

/// Non-instrumented entry point (production/test callers that don't need the
/// W2.1 split). Behavior-identical to [`summarize_snapshot_instrumented`].
pub fn summarize_snapshot(offset: usize, snapshot: &DiplomacyIntelSnapshot) -> Domain53GateSummary {
    summarize_snapshot_instrumented(offset, snapshot, &mut SummaryPhaseTimes::default())
}

/// Instrumented variant: identical output, additionally records the W2.1
/// sub-phase split into `phase`. The `Instant` reads are the only added work (a
/// handful per call) and never influence the returned summary.
pub fn summarize_snapshot_instrumented(
    offset: usize,
    snapshot: &DiplomacyIntelSnapshot,
    phase: &mut SummaryPhaseTimes,
) -> Domain53GateSummary {
    let mut summary = Domain53GateSummary::default();

    let row_count = snapshot.row_count() as usize;
    let changed_rows = snapshot.changed_rows.unwrap_or(snapshot.row_count()) as usize;
    let status_flags = snapshot.status_flags;

    // Header echoes (decode_payload:1289/1643/1644). schema_version / fixed_scale
    // are always the encoder constants, so those parse_errors never fire.
    summary.status_flags_or |= status_flags;
    // `changed_rows > row_count` (decode_payload:1300). `header_count_sum >
    // row_count` (decode_payload:1305) is impossible: header per-kind counts sum to
    // row_count minus state_events, always <= row_count.
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel changed_rows {changed_rows} > row_count {row_count}"
        ));
    }

    // Per-kind decoded counts (== snapshot vec lens on a valid encode) and the
    // flag-derived tallies the status-vs-row block reads.
    let mut country_war_projection_rows = 0usize;
    let mut live_country_rows = 0usize;
    let mut live_war_rows = 0usize;
    let mut operation_detail_resisted_rows = 0usize;
    let mut intel_map_resisted_rows = 0usize;
    let mut operative_lifecycle_rows = 0usize;
    let mut state_event_source_counts = StateEventSourceCounts::default();

    // Per-kind duplicate/seen sets (decode_payload:1313..).
    let mut seen_countries = BTreeSet::new();
    let mut seen_wars = BTreeSet::new();
    let mut seen_opinions = PairSeenSet::default();
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();
    let mut seen_agencies = BTreeSet::new();
    let mut seen_agency_capacity_clusters = BTreeSet::new();
    let mut seen_operatives = BTreeSet::new();
    let mut seen_crypto = BTreeSet::new();
    let mut seen_operations = BTreeSet::new();
    let mut seen_intel_map = BTreeSet::new();
    let mut seen_state_events = StateEventSeenSet::default();

    // Unknown status bits (decode_payload:1331, BEFORE the row loop).
    if status_flags & !KNOWN_STATUS_FLAGS != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel status_flags has unknown bits 0x{:X}",
            status_flags & !KNOWN_STATUS_FLAGS
        ));
    }

    // Base row loop — SAME kind order the encoder emits, SAME global row_index the
    // decoder assigns. Each block mirrors the corresponding `validate_*_row` push
    // order exactly (reading snapshot fields instead of decoded bytes). The
    // "kind/length/truncation" branches in decode_payload are impossible here.
    let base_rows_started = std::time::Instant::now();
    let mut row_index = 0usize;
    for row in &snapshot.countries {
        let deleted = row_id_deleted(row.tag_db_id);
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            deleted,
            &mut summary,
        );
        if !deleted {
            live_country_rows = live_country_rows.saturating_add(1);
        }
        if !deleted && (row.at_war_count > 0 || row.casualties != 0) {
            country_war_projection_rows = country_war_projection_rows.saturating_add(1);
        }
        summarize_country_row(offset, row_index, row, &mut seen_countries, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.wars {
        let deleted = row_id_deleted(row.first_tag_db_id);
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            deleted,
            &mut summary,
        );
        if !deleted {
            live_war_rows = live_war_rows.saturating_add(1);
        }
        summarize_war_row(offset, row_index, row, &mut seen_wars, &mut summary);
        row_index += 1;
    }
    let opinions_started = std::time::Instant::now();
    for row in &snapshot.opinions {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.src_tag_db_id),
            &mut summary,
        );
        summarize_opinion_row(offset, row_index, row, &mut seen_opinions, &mut summary);
        row_index += 1;
    }
    phase.opinions_us = micros_u64(opinions_started.elapsed());
    for row in &snapshot.relation_flags {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.src_tag_db_id),
            &mut summary,
        );
        summarize_relation_flag_row(offset, row_index, row, &mut seen_flags, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.subjects {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.overlord_tag_db_id),
            &mut summary,
        );
        summarize_subject_row(offset, row_index, row, &mut seen_subjects, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.agencies {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.owner_tag_db_id),
            &mut summary,
        );
        summarize_agency_row(offset, row_index, row, &mut seen_agencies, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.agency_capacity_clusters {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.owner_tag_db_id),
            &mut summary,
        );
        summarize_agency_capacity_cluster_row(
            offset,
            row_index,
            row,
            &mut seen_agency_capacity_clusters,
            &mut summary,
        );
        row_index += 1;
    }
    for row in &snapshot.operatives {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.op_id),
            &mut summary,
        );
        if row.lifecycle_flag.is_some() {
            operative_lifecycle_rows = operative_lifecycle_rows.saturating_add(1);
        }
        summarize_operative_row(offset, row_index, row, &mut seen_operatives, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.cryptology {
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            row_id_deleted(row.owner_tag_db_id),
            &mut summary,
        );
        summarize_crypto_row(offset, row_index, row, &mut seen_crypto, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.operation_anchors {
        if row.detail_resisted {
            operation_detail_resisted_rows = operation_detail_resisted_rows.saturating_add(1);
        }
        summarize_operation_row(offset, row_index, row, &mut seen_operations, &mut summary);
        row_index += 1;
    }
    for row in &snapshot.intel_map {
        if row.resisted {
            intel_map_resisted_rows = intel_map_resisted_rows.saturating_add(1);
        }
        summarize_intel_map_row(offset, row_index, row, &mut seen_intel_map, &mut summary);
        row_index += 1;
    }
    let state_events_started = std::time::Instant::now();
    for row in &snapshot.state_events {
        let deleted = row_id_deleted(row.owner_tag_db_id);
        reject_snapshot_tombstone_outside_delta(
            offset,
            row_index,
            status_flags,
            deleted,
            &mut summary,
        );
        if !deleted {
            state_event_source_counts.add(row.source_kind);
        }
        summarize_state_event_row(offset, row_index, row, &mut seen_state_events, &mut summary);
        row_index += 1;
    }
    phase.state_events_us = micros_u64(state_events_started.elapsed());
    phase.base_rows_us = micros_u64(base_rows_started.elapsed());

    // Trailers — re-encode ONLY the trailer suffix (emit order) and run the REAL
    // decoder trailer state machine so `consumed==0` early-abort, row_count caps,
    // INGR/FACS count gates, unique-recipient counting, and every trailer error
    // string are byte-identical by construction. Codex-confirmed: no trailer error
    // reachable from encoder-produced bytes embeds an absolute payload offset.
    let trailers_started = std::time::Instant::now();
    let mut trailer_bytes = Vec::new();
    append_operation_instance_trailer(&mut trailer_bytes, &snapshot.operation_instances);
    append_incoming_action_queue_trailer(&mut trailer_bytes, &snapshot.incoming_actions);
    append_intel_network_graph_trailer(&mut trailer_bytes, &snapshot.intel_network_graphs);
    append_faction_trailer(
        &mut trailer_bytes,
        &snapshot.factions,
        snapshot.faction_roster_complete,
    );
    append_subject_autonomy_status_trailer(
        &mut trailer_bytes,
        &snapshot.subject_autonomy_statuses,
        snapshot.subject_autonomy_status_complete,
    );

    let mut operation_instance_rows = 0usize;
    let mut operation_instance_country_rows = 0usize;
    let mut operation_instance_def_ref_ok_rows = 0usize;
    let mut incoming_action_queue_rows = 0usize;
    let mut incoming_action_queue_recipients = BTreeSet::new();
    let mut intel_network_graph_rows = 0usize;
    let mut intel_network_graph_state_rows = 0usize;
    let mut faction_rows = 0usize;
    let mut faction_member_rows = 0usize;
    if !trailer_bytes.is_empty() {
        let trailer_rows =
            decode_domain53_trailers(offset, &trailer_bytes, 0, &mut summary.parse_errors);
        operation_instance_rows = trailer_rows.operation_instances.len();
        operation_instance_country_rows = trailer_rows
            .operation_instances
            .iter()
            .filter(|row| row.associated_country_tag_db_id.is_some())
            .count();
        operation_instance_def_ref_ok_rows = trailer_rows
            .operation_instances
            .iter()
            .filter(|row| row.def_ref_ok)
            .count();
        incoming_action_queue_rows = trailer_rows.incoming_actions.len();
        incoming_action_queue_recipients.extend(
            trailer_rows
                .incoming_actions
                .iter()
                .map(|row| row.recipient_tag_db_id),
        );
        intel_network_graph_rows = trailer_rows.intel_network_graphs.len();
        intel_network_graph_state_rows = trailer_rows
            .intel_network_graphs
            .iter()
            .map(|row| row.state_ids.len())
            .sum();
        faction_rows = trailer_rows.factions.len();
        faction_member_rows = trailer_rows
            .factions
            .iter()
            .map(|row| row.member_tag_db_ids.len())
            .sum();
    }
    add_unit(
        &mut summary,
        "operation_instance_rows",
        operation_instance_rows,
    );
    add_unit(
        &mut summary,
        "operation_instance_country_rows",
        operation_instance_country_rows,
    );
    add_unit(
        &mut summary,
        "operation_instance_def_ref_ok_rows",
        operation_instance_def_ref_ok_rows,
    );
    add_unit(
        &mut summary,
        "incoming_action_queue_rows",
        incoming_action_queue_rows,
    );
    add_unit(
        &mut summary,
        "incoming_action_queue_recipient_rows",
        incoming_action_queue_recipients.len(),
    );
    add_unit(
        &mut summary,
        "intel_network_graph_rows",
        intel_network_graph_rows,
    );
    add_unit(
        &mut summary,
        "intel_network_graph_state_rows",
        intel_network_graph_state_rows,
    );
    add_unit(&mut summary, "faction_rows", faction_rows);
    add_unit(&mut summary, "faction_member_rows", faction_member_rows);

    // `decoded_counts != expected_counts` (decode_payload:1596) is impossible on a
    // valid encode (decoded counts == header counts == snapshot vec lens).

    // Status-vs-row consistency block (decode_payload:1602..) — SAME order, SAME
    // strings. Base counts come from snapshot vec lens; the graph count comes from
    // the DECODED trailer rows above.
    phase.trailers_us = micros_u64(trailers_started.elapsed());
    if status_flags & STATUS_WAR_ROOT_UNVALIDATED != 0
        && (live_war_rows != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges war root unvalidated but emits {} war relation row(s) and {} country war projection row(s)",
            live_war_rows, country_war_projection_rows
        ));
    }
    if status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED == 0
        && (live_war_rows != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel emits {} war relation row(s) and {} country war projection row(s) without STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED; schema v{SCHEMA_VERSION} has no war name/grouping/start-date fields",
            live_war_rows, country_war_projection_rows
        ));
    }
    if operation_detail_resisted_rows != 0 && status_flags & STATUS_OPERATIONS_DETAIL_RESISTED == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel has {operation_detail_resisted_rows} resisted operation row(s) without STATUS_OPERATIONS_DETAIL_RESISTED"
        ));
    }
    if intel_map_resisted_rows != 0 && status_flags & STATUS_INTEL_MAP_RESISTED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel has {intel_map_resisted_rows} resisted intel-map row(s) without STATUS_INTEL_MAP_RESISTED"
        ));
    }
    if status_flags & STATUS_FACTION_GROUP_UNVALIDATED != 0
        && (live_country_rows != 0 || state_event_source_counts.country_faction_group != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges faction group unvalidated but emits {} country diplomacy row(s) and {} faction state event row(s)",
            live_country_rows, state_event_source_counts.country_faction_group
        ));
    }
    if status_flags & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED != 0 && intel_network_graph_rows != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges intel-network graph unvalidated but emits {intel_network_graph_rows} graph row(s)"
        ));
    }

    // Base counts / units (decode_payload:1643..).
    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "diplomacy_intel_rows", row_count);
    add_unit(
        &mut summary,
        "diplomacy_country_rows",
        snapshot.countries.len(),
    );
    add_unit(&mut summary, "war_relation_rows", snapshot.wars.len());
    add_unit(&mut summary, "opinion_rows", snapshot.opinions.len());
    add_unit(
        &mut summary,
        "relation_flag_rows",
        snapshot.relation_flags.len(),
    );
    add_unit(&mut summary, "subject_rows", snapshot.subjects.len());
    add_unit(&mut summary, "intel_agency_rows", snapshot.agencies.len());
    add_unit(
        &mut summary,
        "agency_capacity_cluster_rows",
        snapshot.agency_capacity_clusters.len(),
    );
    add_unit(&mut summary, "operative_rows", snapshot.operatives.len());
    add_unit(
        &mut summary,
        "operative_lifecycle_rows",
        operative_lifecycle_rows,
    );
    add_unit(&mut summary, "cryptology_rows", snapshot.cryptology.len());
    add_unit(
        &mut summary,
        "operation_anchor_rows",
        snapshot.operation_anchors.len(),
    );
    add_unit(&mut summary, "intel_map_rows", snapshot.intel_map.len());
    add_unit(
        &mut summary,
        "compact_state_event_rows",
        snapshot.state_events.len(),
    );
    add_state_event_source_units(&mut summary, state_event_source_counts);
    summary
}

/// `(encode_payload(s), summarize_snapshot(offset, s))`. The bytes are identical
/// to `encode_payload`; the summary is identical to
/// `decode_payload(offset, &encode_payload(s))` (proven by the INV tests + the
/// sampled production cross-check).
pub fn encode_payload_with_summary(
    offset: usize,
    snapshot: &DiplomacyIntelSnapshot,
) -> (Vec<u8>, Domain53GateSummary) {
    (
        encode_payload(snapshot),
        summarize_snapshot(offset, snapshot),
    )
}

/// Append the `FACS` faction-roster trailer after the self-delimiting base
/// payload. Trailer layout (header_len = 12):
///   magic "FACS" (4) | version u16 | header_len u16 (=12) | row_count u32
/// then per faction: {faction_id u32, faction_type u32, name_hash u32,
/// leader_tag_db_id u32, member_count u32} followed by member_count u32
/// tag_db_ids (sorted ascending). Version 1 is retained for callers encoding a
/// legacy non-empty snapshot without the explicit runtime witness. Version 2
/// positively witnesses a complete roster and is emitted even with zero rows.
fn append_faction_trailer(payload: &mut Vec<u8>, rows: &[FactionRow], roster_complete: bool) {
    if rows.is_empty() && !roster_complete {
        return;
    }
    payload.extend_from_slice(FACTION_TRAILER_MAGIC);
    let version = if roster_complete {
        FACTION_TRAILER_VERSION
    } else {
        FACTION_TRAILER_VERSION_V1_NONEMPTY
    };
    payload.extend_from_slice(&version.to_le_bytes());
    payload.extend_from_slice(&(FACTION_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.faction_id.to_le_bytes());
        payload.extend_from_slice(&row.faction_type.to_le_bytes());
        payload.extend_from_slice(&row.name_hash.to_le_bytes());
        payload.extend_from_slice(&row.leader_tag_db_id.to_le_bytes());
        payload.extend_from_slice(
            &(row.member_tag_db_ids.len().min(u32::MAX as usize) as u32).to_le_bytes(),
        );
        for member in &row.member_tag_db_ids {
            payload.extend_from_slice(&member.to_le_bytes());
        }
    }
}

fn append_subject_autonomy_status_trailer(
    payload: &mut Vec<u8>,
    rows: &[SubjectAutonomyStatusRow],
    complete: bool,
) {
    if !complete {
        return;
    }
    payload.extend_from_slice(SUBJECT_AUTONOMY_STATUS_TRAILER_MAGIC);
    payload.extend_from_slice(&SUBJECT_AUTONOMY_STATUS_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in rows {
        let mut flags = 0u32;
        if row.previous_state_hash.is_some() {
            flags |= SUBJECT_AUTONOMY_STATUS_FLAG_PREVIOUS;
        }
        if row.next_state_hash.is_some() {
            flags |= SUBJECT_AUTONOMY_STATUS_FLAG_NEXT;
        }
        payload.extend_from_slice(&row.overlord_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.subject_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.current_state_hash.to_le_bytes());
        payload.extend_from_slice(&row.previous_state_hash.unwrap_or_default().to_le_bytes());
        payload.extend_from_slice(&row.next_state_hash.unwrap_or_default().to_le_bytes());
        payload.extend_from_slice(&row.ratio_raw.to_le_bytes());
        payload.extend_from_slice(&row.current_score_raw.to_le_bytes());
        payload.extend_from_slice(&row.lower_score_raw.to_le_bytes());
        payload.extend_from_slice(&row.upper_score_raw.to_le_bytes());
        payload.extend_from_slice(&row.last_modified_date_raw.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
    }
}

fn subject_autonomy_ratio_raw(
    current_score_raw: i64,
    lower_score_raw: i64,
    upper_score_raw: i64,
) -> Option<i32> {
    if lower_score_raw >= upper_score_raw {
        return None;
    }
    let forward = current_score_raw.checked_sub(lower_score_raw)?.max(0);
    let remaining = upper_score_raw.checked_sub(current_score_raw)?.max(0);
    let denominator = forward.checked_add(remaining)?;
    if denominator <= 0 {
        return None;
    }
    let ratio = forward
        .checked_mul(i64::from(FIXED_SCALE_RAW))?
        .checked_div(denominator)?;
    let ratio = i32::try_from(ratio).ok()?;
    (0..=FIXED_SCALE_RAW as i32)
        .contains(&ratio)
        .then_some(ratio)
}

/// Wave-5: append the `OPIN` running-operation-instance trailer after the
/// self-delimiting base payload. Trailer layout (header_len = 12):
///   magic "OPIN" (4) | version u16 | header_len u16 (=12) | row_count u32
/// then per instance: {instance_id u32, flags u32, associated_country u32}
/// (flags: DEF_REF_OK bit0, COUNTRY_PRESENT bit1). Emitted only when at least
/// one instance is present; no rows => no trailer (legacy-shaped record).
///
/// BLOCKED-ADDITIVE (build 870ebf): the D53 base extract is fail-closed on
/// 870ebf (unvalidated live root offsets → empty SOURCE_UNAVAILABLE record),
/// so `rows` is always empty in production today and this trailer is not
/// live-emitted. The encode/decode is fully exercised by unit tests; it lights
/// up when the d53 base root source-availability lands.
fn append_operation_instance_trailer(payload: &mut Vec<u8>, rows: &[OperationInstanceRow]) {
    if rows.is_empty() {
        return;
    }
    let exact_runtime = rows.iter().all(OperationInstanceRow::is_exact_runtime);
    payload.extend_from_slice(OPERATION_INSTANCE_TRAILER_MAGIC);
    payload.extend_from_slice(
        &(if exact_runtime {
            OPERATION_INSTANCE_TRAILER_VERSION
        } else {
            OPERATION_INSTANCE_TRAILER_VERSION_V1
        })
        .to_le_bytes(),
    );
    payload.extend_from_slice(&(OPERATION_INSTANCE_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    if exact_runtime {
        for row in rows {
            let definition_id = row.definition_id.as_deref().unwrap_or_default().as_bytes();
            let owner = row.owner_country_tag_db_id.unwrap_or_default();
            let target_country = row.target_country_tag_db_id.unwrap_or_default();
            let target_state = row.target_state_id.unwrap_or_default();
            let status = match row.status {
                Some(OperationInstanceStatus::Preparing) => 0u8,
                Some(OperationInstanceStatus::Prepared) => 1u8,
                Some(OperationInstanceStatus::Running) => 2u8,
                Some(OperationInstanceStatus::Finished) => 3u8,
                None => unreachable!("exact OPIN v3 row requires status"),
            };
            let flags = if row.target_state_id.is_some() {
                OPERATION_INSTANCE_V2_FLAG_STATE_TARGET
            } else {
                0
            };
            payload.extend_from_slice(&row.instance_id.to_le_bytes());
            payload.extend_from_slice(&owner.to_le_bytes());
            payload.extend_from_slice(&target_country.to_le_bytes());
            payload.extend_from_slice(&target_state.to_le_bytes());
            payload.push(status);
            payload.push(flags);
            payload.extend_from_slice(&(definition_id.len() as u16).to_le_bytes());
            payload.extend_from_slice(&row.phase_index.to_le_bytes());
            payload.extend_from_slice(&row.phase_count.to_le_bytes());
            payload.extend_from_slice(&row.progress_fixed.unwrap_or_default().to_le_bytes());
            payload.extend_from_slice(&row.start_hour.unwrap_or_default().to_le_bytes());
            payload.extend_from_slice(&row.duration_hours.unwrap_or_default().to_le_bytes());
            payload.extend_from_slice(&(row.assigned_operative_ids.len() as u16).to_le_bytes());
            payload.extend_from_slice(&0u16.to_le_bytes());
            payload.extend_from_slice(definition_id);
            for operative_id in &row.assigned_operative_ids {
                payload.extend_from_slice(&operative_id.to_le_bytes());
            }
        }
        return;
    }
    for row in rows {
        let mut flags = 0u32;
        if row.def_ref_ok {
            flags |= OPERATION_INSTANCE_FLAG_DEF_REF_OK;
        }
        if row.associated_country_tag_db_id.is_some() {
            flags |= OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT;
        }
        payload.extend_from_slice(&row.instance_id.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.associated_country_tag_db_id.unwrap_or(0).to_le_bytes());
    }
}

fn append_incoming_action_queue_trailer(payload: &mut Vec<u8>, rows: &[IncomingActionQueueRow]) {
    if rows.is_empty() {
        return;
    }
    payload.extend_from_slice(INCOMING_ACTION_QUEUE_TRAILER_MAGIC);
    payload.extend_from_slice(&INCOMING_ACTION_QUEUE_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.recipient_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.actor_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.action_id.to_le_bytes());
        payload.extend_from_slice(&row.action_type.to_le_bytes());
        payload.extend_from_slice(&row.date_hours.to_le_bytes());
        payload.extend_from_slice(&row.last_command_date_hours.to_le_bytes());
    }
}

fn append_intel_network_graph_trailer(payload: &mut Vec<u8>, rows: &[IntelNetworkGraphRow]) {
    if rows.is_empty() {
        return;
    }
    let exact_coverage = rows.iter().all(|row| row.coverage.is_some());
    let any_coverage = rows.iter().any(|row| row.coverage.is_some());
    let any_state_strengths = rows.iter().any(|row| row.state_strengths.is_some());
    let exact_state_strengths = rows.iter().all(|row| {
        row.state_strengths.as_ref().is_some_and(|strengths| {
            strengths.len() == row.state_ids.len()
                && strengths
                    .iter()
                    .zip(&row.state_ids)
                    .all(|(strength, state_id)| {
                        strength.state_id == *state_id
                            && (0..=compact_contract::build_profile::DOMAIN53_INTEL_NETWORK_STRENGTH_SCALE)
                                .contains(&strength.strength_raw)
                    })
        })
    });
    // Mixed or malformed v3 rows have no truthful legacy representation: do
    // not silently discard a subset of measured strengths.
    if (any_coverage && !exact_coverage) || (any_state_strengths && !exact_state_strengths) {
        return;
    }
    payload.extend_from_slice(INTEL_NETWORK_GRAPH_TRAILER_MAGIC);
    let version = if exact_state_strengths {
        INTEL_NETWORK_GRAPH_TRAILER_VERSION
    } else if exact_coverage {
        INTEL_NETWORK_GRAPH_TRAILER_VERSION_V2
    } else {
        INTEL_NETWORK_GRAPH_TRAILER_VERSION_V1
    };
    payload.extend_from_slice(&version.to_le_bytes());
    payload.extend_from_slice(&(INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    for row in rows {
        payload.extend_from_slice(&row.strategic_operative_index.to_le_bytes());
        payload.extend_from_slice(&row.network_slot.to_le_bytes());
        payload.extend_from_slice(&row.target_tag_db_id.to_le_bytes());
        payload
            .extend_from_slice(&(row.state_ids.len().min(u32::MAX as usize) as u32).to_le_bytes());
        if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
            payload.extend_from_slice(
                &(if row.coverage.is_some() {
                    INTEL_NETWORK_GRAPH_V3_FLAG_COVERAGE
                } else {
                    0
                })
                .to_le_bytes(),
            );
        }
        if let Some(coverage) = &row.coverage {
            payload.extend_from_slice(&coverage.covered_core_states.to_le_bytes());
            payload.extend_from_slice(&coverage.covered_controlled_states.to_le_bytes());
            payload.extend_from_slice(&coverage.covered_worth.to_le_bytes());
            payload.extend_from_slice(&coverage.controlled_strength_sum_raw.to_le_bytes());
            payload.extend_from_slice(&coverage.core_strength_sum_raw.to_le_bytes());
            payload.extend_from_slice(&coverage.total_core_states.to_le_bytes());
            payload.extend_from_slice(&coverage.total_controlled_states.to_le_bytes());
            payload.extend_from_slice(&coverage.total_worth.to_le_bytes());
            payload.extend_from_slice(&coverage.weight_controlled_raw.to_le_bytes());
            payload.extend_from_slice(&coverage.weight_owned_raw.to_le_bytes());
            payload.extend_from_slice(&coverage.weight_owned_worth_raw.to_le_bytes());
            payload.extend_from_slice(&coverage.national_coverage_raw.to_le_bytes());
        } else if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
            payload.extend_from_slice(&[0u8; 72]);
        }
        if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
            for strength in row
                .state_strengths
                .as_ref()
                .expect("v3 strength completeness checked above")
            {
                payload.extend_from_slice(&strength.state_id.to_le_bytes());
                payload.extend_from_slice(&strength.strength_raw.to_le_bytes());
            }
        } else {
            for state_id in &row.state_ids {
                payload.extend_from_slice(&state_id.to_le_bytes());
            }
        }
    }
}

/// Decode + gate the `OPIN` trailer at `trailer_offset`. Returns the decoded
/// rows and appends any parse errors to `errors`. Strict: exact-end
/// consumption, unique instance ids, flag/country consistency.
fn decode_operation_instance_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> (Vec<OperationInstanceRow>, usize) {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: operation-instance trailer offset {trailer_offset} beyond payload"
        ));
        return (Vec::new(), 0);
    };
    if trailer.len() < OPERATION_INSTANCE_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: operation-instance trailer is {} byte(s), expected at least {}",
            trailer.len(),
            OPERATION_INSTANCE_TRAILER_HEADER_LEN
        ));
        return (Vec::new(), 0);
    }
    if &trailer[0..4] != OPERATION_INSTANCE_TRAILER_MAGIC {
        errors.push(format!(
            "offset {offset}: operation-instance trailer magic is invalid"
        ));
        return (Vec::new(), 0);
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    if !matches!(
        version,
        OPERATION_INSTANCE_TRAILER_VERSION_V1
            | OPERATION_INSTANCE_TRAILER_VERSION_V2
            | OPERATION_INSTANCE_TRAILER_VERSION
    ) || header_len != OPERATION_INSTANCE_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: operation-instance trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return (Vec::new(), 0);
    }
    if row_count > MAX_OPERATION_INSTANCE_ROWS {
        errors.push(format!(
            "offset {offset}: operation-instance trailer row_count {row_count} exceeds {MAX_OPERATION_INSTANCE_ROWS}"
        ));
        return (Vec::new(), 0);
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    let mut cursor = OPERATION_INSTANCE_TRAILER_HEADER_LEN;
    if version == OPERATION_INSTANCE_TRAILER_VERSION_V1 {
        let expected = OPERATION_INSTANCE_TRAILER_HEADER_LEN
            .saturating_add(row_count.saturating_mul(OPERATION_INSTANCE_TRAILER_V1_ROW_LEN));
        if trailer.len() < expected {
            errors.push(format!(
                "offset {offset}: operation-instance v1 trailer len {} < {expected}",
                trailer.len()
            ));
            return (Vec::new(), 0);
        }
        for _ in 0..row_count {
            let instance_id = read_u32_le(trailer, cursor);
            let flags = read_u32_le(trailer, cursor + 4);
            let country = read_u32_le(trailer, cursor + 8);
            cursor += OPERATION_INSTANCE_TRAILER_V1_ROW_LEN;
            if instance_id == 0 {
                errors.push(format!(
                    "offset {offset}: operation-instance row has zero instance_id"
                ));
            }
            if flags
                & !(OPERATION_INSTANCE_FLAG_DEF_REF_OK | OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT)
                != 0
            {
                errors.push(format!(
                    "offset {offset}: operation-instance row has unknown flags {flags:#x}"
                ));
            }
            let country_present = flags & OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT != 0;
            if country_present && (country == 0 || country > MAX_TAG_DB_ID) {
                errors.push(format!(
                    "offset {offset}: operation-instance row has invalid associated_country {country}"
                ));
            } else if !country_present && country != 0 {
                errors.push(format!(
                    "offset {offset}: operation-instance row has country without present flag"
                ));
            }
            if !seen.insert(instance_id) {
                errors.push(format!(
                    "offset {offset}: operation-instance trailer duplicates instance_id {instance_id}"
                ));
            }
            rows.push(OperationInstanceRow::legacy(
                instance_id,
                flags & OPERATION_INSTANCE_FLAG_DEF_REF_OK != 0,
                country_present.then_some(country),
            ));
        }
        return (rows, expected);
    }
    for _ in 0..row_count {
        let Some(header_end) = cursor.checked_add(OPERATION_INSTANCE_TRAILER_V2_ROW_HEADER_LEN)
        else {
            errors.push(format!(
                "offset {offset}: operation-instance v2 row overflows"
            ));
            return (Vec::new(), 0);
        };
        if header_end > trailer.len() {
            errors.push(format!(
                "offset {offset}: operation-instance v2 row header is truncated"
            ));
            return (Vec::new(), 0);
        }
        let instance_id = read_u32_le(trailer, cursor);
        let owner = read_u32_le(trailer, cursor + 4);
        let target_country = read_u32_le(trailer, cursor + 8);
        let target_state = read_u32_le(trailer, cursor + 12);
        let status_raw = trailer[cursor + 16];
        let flags = trailer[cursor + 17];
        let definition_len = read_u16_le(trailer, cursor + 18) as usize;
        let phase_index = read_u32_le(trailer, cursor + 20);
        let phase_count = read_u32_le(trailer, cursor + 24);
        let progress_fixed = read_i64_le(trailer, cursor + 28);
        let start_hour = read_u32_le(trailer, cursor + 36);
        let duration_hours = read_u32_le(trailer, cursor + 40);
        let operative_count = read_u16_le(trailer, cursor + 44) as usize;
        let reserved = read_u16_le(trailer, cursor + 46);
        cursor = header_end;
        let row_end = cursor
            .checked_add(definition_len)
            .and_then(|value| value.checked_add(operative_count.saturating_mul(4)));
        let Some(row_end) = row_end else {
            errors.push(format!(
                "offset {offset}: operation-instance v2 row overflows"
            ));
            return (Vec::new(), 0);
        };
        if row_end > trailer.len() {
            errors.push(format!(
                "offset {offset}: operation-instance v2 variable row is truncated"
            ));
            return (Vec::new(), 0);
        }
        let definition_id = std::str::from_utf8(&trailer[cursor..cursor + definition_len])
            .ok()
            .filter(|value| !value.is_empty() && !value.chars().any(char::is_control))
            .map(ToOwned::to_owned);
        cursor += definition_len;
        let mut assigned_operative_ids = Vec::with_capacity(operative_count);
        for _ in 0..operative_count {
            assigned_operative_ids.push(read_u32_le(trailer, cursor));
            cursor += 4;
        }
        let status = match (version, status_raw) {
            (_, 0) => Some(OperationInstanceStatus::Preparing),
            (OPERATION_INSTANCE_TRAILER_VERSION_V2, 1) => {
                Some(OperationInstanceStatus::Running)
            }
            (OPERATION_INSTANCE_TRAILER_VERSION_V2, 2) => {
                Some(OperationInstanceStatus::Finished)
            }
            (OPERATION_INSTANCE_TRAILER_VERSION, 1) => {
                Some(OperationInstanceStatus::Prepared)
            }
            (OPERATION_INSTANCE_TRAILER_VERSION, 2) => {
                Some(OperationInstanceStatus::Running)
            }
            (OPERATION_INSTANCE_TRAILER_VERSION, 3) => {
                Some(OperationInstanceStatus::Finished)
            }
            _ => None,
        };
        if instance_id == 0
            || owner == 0
            || owner > MAX_TAG_DB_ID
            || target_country == 0
            || target_country > MAX_TAG_DB_ID
            || definition_len == 0
            || definition_len > MAX_OPERATION_DEFINITION_ID_LEN
            || definition_id.is_none()
            || operative_count > MAX_OPERATION_ASSIGNED_OPERATIVES
            || assigned_operative_ids
                .iter()
                .any(|id| *id == 0 || *id > MAX_OPERATIVE_ID)
            || phase_count == 0
            || phase_count > MAX_OPERATION_PHASES
            || phase_index >= phase_count
            || !(0..=compact_contract::build_profile::DOMAIN53_OPERATION_PROGRESS_SCALE)
                .contains(&progress_fixed)
            || status.is_none()
            || flags & !OPERATION_INSTANCE_V2_FLAG_STATE_TARGET != 0
            || reserved != 0
            || (flags & OPERATION_INSTANCE_V2_FLAG_STATE_TARGET != 0) != (target_state != 0)
            || (matches!(
                status,
                Some(OperationInstanceStatus::Preparing | OperationInstanceStatus::Prepared)
            )
                && (start_hour != 0 || duration_hours != 0 || phase_index != 0))
            || (matches!(status, Some(OperationInstanceStatus::Prepared))
                && progress_fixed
                    != compact_contract::build_profile::DOMAIN53_OPERATION_PROGRESS_SCALE)
            || (matches!(
                status,
                Some(OperationInstanceStatus::Running | OperationInstanceStatus::Finished)
            ) && duration_hours == 0)
            || (matches!(status, Some(OperationInstanceStatus::Finished))
                && (phase_index + 1 != phase_count
                    || progress_fixed
                        != compact_contract::build_profile::DOMAIN53_OPERATION_PROGRESS_SCALE))
        {
            errors.push(format!(
                "offset {offset}: operation-instance v2 row {instance_id} violates the exact runtime contract"
            ));
        }
        if !seen.insert(instance_id) {
            errors.push(format!(
                "offset {offset}: operation-instance trailer duplicates instance_id {instance_id}"
            ));
        }
        rows.push(OperationInstanceRow {
            instance_id,
            def_ref_ok: true,
            associated_country_tag_db_id: Some(target_country),
            definition_id,
            owner_country_tag_db_id: Some(owner),
            target_country_tag_db_id: Some(target_country),
            target_state_id: (target_state != 0).then_some(target_state),
            status,
            phase_index,
            phase_count,
            progress_fixed: Some(progress_fixed),
            start_hour: Some(start_hour),
            duration_hours: Some(duration_hours),
            assigned_operative_ids,
        });
    }
    (rows, cursor)
}

fn decode_subject_autonomy_status_trailer(
    offset: usize,
    payload: &[u8],
    start: usize,
    errors: &mut Vec<String>,
) -> (Vec<SubjectAutonomyStatusRow>, usize) {
    let Some(header_end) = start.checked_add(SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN) else {
        errors.push(format!("offset {offset}: subject-autonomy status trailer header overflow"));
        return (Vec::new(), 0);
    };
    if header_end > payload.len() {
        errors.push(format!("offset {offset}: truncated subject-autonomy status trailer header"));
        return (Vec::new(), 0);
    }
    let version = read_u16_le(payload, start + 4);
    let header_len = read_u16_le(payload, start + 6) as usize;
    let count = read_u32_le(payload, start + 8) as usize;
    if version != SUBJECT_AUTONOMY_STATUS_TRAILER_VERSION
        || header_len != SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: unsupported subject-autonomy status trailer version/header {version}/{header_len}"
        ));
        return (Vec::new(), 0);
    }
    if count > MAX_SUBJECT_AUTONOMY_STATUS_ROWS {
        errors.push(format!(
            "offset {offset}: subject-autonomy status trailer row_count {count} exceeds {MAX_SUBJECT_AUTONOMY_STATUS_ROWS}"
        ));
        return (Vec::new(), 0);
    }
    let Some(total_len) = count
        .checked_mul(SUBJECT_AUTONOMY_STATUS_TRAILER_ROW_LEN)
        .and_then(|rows| rows.checked_add(SUBJECT_AUTONOMY_STATUS_TRAILER_HEADER_LEN))
    else {
        errors.push(format!("offset {offset}: subject-autonomy status trailer length overflow"));
        return (Vec::new(), 0);
    };
    if payload.len().saturating_sub(start) < total_len {
        errors.push(format!("offset {offset}: truncated subject-autonomy status trailer rows"));
        return (Vec::new(), 0);
    }
    let mut rows = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    let mut cursor = header_end;
    for index in 0..count {
        let overlord = read_u32_le(payload, cursor);
        let subject = read_u32_le(payload, cursor + 4);
        let current_state_hash = read_u32_le(payload, cursor + 8);
        let previous_raw = read_u32_le(payload, cursor + 12);
        let next_raw = read_u32_le(payload, cursor + 16);
        let ratio_raw = read_i32_le(payload, cursor + 20);
        let current_score_raw = read_i64_le(payload, cursor + 24);
        let lower_score_raw = read_i64_le(payload, cursor + 32);
        let upper_score_raw = read_i64_le(payload, cursor + 40);
        let last_modified_date_raw = read_u32_le(payload, cursor + 48);
        let flags = read_u32_le(payload, cursor + 52);
        let known_flags = SUBJECT_AUTONOMY_STATUS_FLAG_PREVIOUS | SUBJECT_AUTONOMY_STATUS_FLAG_NEXT;
        let previous_state_hash = (flags & SUBJECT_AUTONOMY_STATUS_FLAG_PREVIOUS != 0)
            .then_some(previous_raw);
        let next_state_hash =
            (flags & SUBJECT_AUTONOMY_STATUS_FLAG_NEXT != 0).then_some(next_raw);
        let computed_ratio = subject_autonomy_ratio_raw(
            current_score_raw,
            lower_score_raw,
            upper_score_raw,
        );
        if !valid_tag(overlord)
            || !valid_tag(subject)
            || overlord == subject
            || current_state_hash == 0
            || flags & !known_flags != 0
            || previous_state_hash.is_some_and(|hash| hash == 0)
            || next_state_hash.is_some_and(|hash| hash == 0)
            || (previous_state_hash.is_none() && previous_raw != 0)
            || (next_state_hash.is_none() && next_raw != 0)
            || !(0..=FIXED_SCALE_RAW as i32).contains(&ratio_raw)
            || computed_ratio != Some(ratio_raw)
            || !seen.insert((overlord, subject))
        {
            errors.push(format!(
                "offset {offset}: invalid subject-autonomy status trailer row {index}"
            ));
            return (Vec::new(), 0);
        }
        rows.push(SubjectAutonomyStatusRow {
            overlord_tag_db_id: overlord,
            subject_tag_db_id: subject,
            current_state_hash,
            previous_state_hash,
            next_state_hash,
            ratio_raw,
            current_score_raw,
            lower_score_raw,
            upper_score_raw,
            last_modified_date_raw,
        });
        cursor += SUBJECT_AUTONOMY_STATUS_TRAILER_ROW_LEN;
    }
    (rows, total_len)
}

#[derive(Default)]
struct DecodedTrailers {
    operation_instances: Vec<OperationInstanceRow>,
    incoming_actions: Vec<IncomingActionQueueRow>,
    intel_network_graphs: Vec<IntelNetworkGraphRow>,
    factions: Vec<FactionRow>,
    faction_roster_complete: bool,
    subject_autonomy_statuses: Vec<SubjectAutonomyStatusRow>,
    subject_autonomy_status_complete: bool,
}

fn decode_domain53_trailers(
    offset: usize,
    payload: &[u8],
    mut cursor: usize,
    errors: &mut Vec<String>,
) -> DecodedTrailers {
    let mut decoded = DecodedTrailers::default();
    let mut seen_opin = false;
    let mut seen_diaq = false;
    let mut seen_ingr = false;
    let mut seen_facs = false;
    let mut seen_aust = false;
    while cursor < payload.len() {
        if payload.get(cursor..cursor + 4) == Some(OPERATION_INSTANCE_TRAILER_MAGIC) {
            if seen_opin {
                errors.push(format!(
                    "offset {offset}: duplicate operation-instance trailer"
                ));
                return decoded;
            }
            let (rows, consumed) =
                decode_operation_instance_trailer(offset, payload, cursor, errors);
            if consumed == 0 {
                return decoded;
            }
            decoded.operation_instances = rows;
            seen_opin = true;
            cursor += consumed;
        } else if payload.get(cursor..cursor + 4) == Some(INCOMING_ACTION_QUEUE_TRAILER_MAGIC) {
            if seen_diaq {
                errors.push(format!(
                    "offset {offset}: duplicate incoming-action queue trailer"
                ));
                return decoded;
            }
            let (rows, consumed) =
                decode_incoming_action_queue_trailer(offset, payload, cursor, errors);
            if consumed == 0 {
                return decoded;
            }
            decoded.incoming_actions = rows;
            seen_diaq = true;
            cursor += consumed;
        } else if payload.get(cursor..cursor + 4) == Some(INTEL_NETWORK_GRAPH_TRAILER_MAGIC) {
            if seen_ingr {
                errors.push(format!(
                    "offset {offset}: duplicate intel-network graph trailer"
                ));
                return decoded;
            }
            let (rows, consumed) =
                decode_intel_network_graph_trailer(offset, payload, cursor, errors);
            if consumed == 0 {
                return decoded;
            }
            decoded.intel_network_graphs = rows;
            seen_ingr = true;
            cursor += consumed;
        } else if payload.get(cursor..cursor + 4) == Some(FACTION_TRAILER_MAGIC) {
            if seen_facs {
                errors.push(format!("offset {offset}: duplicate faction trailer"));
                return decoded;
            }
            let (rows, roster_complete, consumed) =
                decode_faction_trailer(offset, payload, cursor, errors);
            if consumed == 0 {
                return decoded;
            }
            decoded.factions = rows;
            decoded.faction_roster_complete = roster_complete;
            seen_facs = true;
            cursor += consumed;
        } else if payload.get(cursor..cursor + 4)
            == Some(SUBJECT_AUTONOMY_STATUS_TRAILER_MAGIC)
        {
            if seen_aust {
                errors.push(format!(
                    "offset {offset}: duplicate subject-autonomy status trailer"
                ));
                return decoded;
            }
            let (rows, consumed) =
                decode_subject_autonomy_status_trailer(offset, payload, cursor, errors);
            if consumed == 0 {
                return decoded;
            }
            decoded.subject_autonomy_statuses = rows;
            decoded.subject_autonomy_status_complete = true;
            seen_aust = true;
            cursor += consumed;
        } else {
            errors.push(format!(
                "offset {offset}: diplomacy/intel unknown trailer magic at byte {cursor}"
            ));
            return decoded;
        }
    }
    decoded
}

fn decode_incoming_action_queue_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> (Vec<IncomingActionQueueRow>, usize) {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: incoming-action queue trailer offset {trailer_offset} beyond payload"
        ));
        return (Vec::new(), 0);
    };
    if trailer.len() < INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: incoming-action queue trailer is {} byte(s), expected at least {}",
            trailer.len(),
            INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN
        ));
        return (Vec::new(), 0);
    }
    if &trailer[0..4] != INCOMING_ACTION_QUEUE_TRAILER_MAGIC {
        errors.push(format!(
            "offset {offset}: incoming-action queue trailer magic is invalid"
        ));
        return (Vec::new(), 0);
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    if version != INCOMING_ACTION_QUEUE_TRAILER_VERSION
        || header_len != INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: incoming-action queue trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return (Vec::new(), 0);
    }
    if row_count > MAX_INCOMING_ACTION_QUEUE_ROWS {
        errors.push(format!(
            "offset {offset}: incoming-action queue row_count {row_count} exceeds {MAX_INCOMING_ACTION_QUEUE_ROWS}"
        ));
        return (Vec::new(), 0);
    }
    let expected = INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN
        .saturating_add(row_count.saturating_mul(INCOMING_ACTION_QUEUE_TRAILER_ROW_LEN));
    if trailer.len() < expected {
        errors.push(format!(
            "offset {offset}: incoming-action queue trailer len {} < {expected}",
            trailer.len()
        ));
        return (Vec::new(), 0);
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    let mut cursor = INCOMING_ACTION_QUEUE_TRAILER_HEADER_LEN;
    for _ in 0..row_count {
        let recipient = read_u32_le(trailer, cursor);
        let actor = read_u32_le(trailer, cursor + 4);
        let action_id = read_u32_le(trailer, cursor + 8);
        let action_type = read_i32_le(trailer, cursor + 12);
        let date_hours = read_u32_le(trailer, cursor + 16);
        let last_command_date_hours = read_u32_le(trailer, cursor + 20);
        cursor += INCOMING_ACTION_QUEUE_TRAILER_ROW_LEN;
        if !valid_tag(recipient) {
            errors.push(format!(
                "offset {offset}: incoming-action row has invalid recipient {recipient}"
            ));
        }
        if !valid_tag(actor) {
            errors.push(format!(
                "offset {offset}: incoming-action row has invalid actor {actor}"
            ));
        }
        if action_id == 0 {
            errors.push(format!(
                "offset {offset}: incoming-action row has zero action_id"
            ));
        }
        if !(0..=MAX_ACTION_DATE_HOURS).contains(&date_hours) {
            errors.push(format!(
                "offset {offset}: incoming-action row has invalid date_hours {date_hours}"
            ));
        }
        if !(0..=MAX_ACTION_DATE_HOURS).contains(&last_command_date_hours) {
            errors.push(format!(
                "offset {offset}: incoming-action row has invalid last_command_date_hours {last_command_date_hours}"
            ));
        }
        if !seen.insert(action_id) {
            errors.push(format!(
                "offset {offset}: incoming-action trailer duplicates action_id {action_id}"
            ));
        }
        rows.push(IncomingActionQueueRow {
            recipient_tag_db_id: recipient,
            actor_tag_db_id: actor,
            action_id,
            action_type,
            date_hours,
            last_command_date_hours,
        });
    }
    (rows, expected)
}

fn decode_intel_network_graph_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> (Vec<IntelNetworkGraphRow>, usize) {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: intel-network graph trailer offset {trailer_offset} beyond payload"
        ));
        return (Vec::new(), 0);
    };
    if trailer.len() < INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: intel-network graph trailer is {} byte(s), expected at least {}",
            trailer.len(),
            INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN
        ));
        return (Vec::new(), 0);
    }
    if &trailer[0..4] != INTEL_NETWORK_GRAPH_TRAILER_MAGIC {
        errors.push(format!(
            "offset {offset}: intel-network graph trailer magic is invalid"
        ));
        return (Vec::new(), 0);
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    if !matches!(
        version,
        INTEL_NETWORK_GRAPH_TRAILER_VERSION_V1
            | INTEL_NETWORK_GRAPH_TRAILER_VERSION_V2
            | INTEL_NETWORK_GRAPH_TRAILER_VERSION
    )
        || header_len != INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: intel-network graph trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return (Vec::new(), 0);
    }
    if row_count > MAX_INTEL_NETWORK_ROWS {
        errors.push(format!(
            "offset {offset}: intel-network graph row_count {row_count} exceeds {MAX_INTEL_NETWORK_ROWS}"
        ));
        return (Vec::new(), 0);
    }

    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    let mut cursor = INTEL_NETWORK_GRAPH_TRAILER_HEADER_LEN;
    for _ in 0..row_count {
        let row_header_len = match version {
            INTEL_NETWORK_GRAPH_TRAILER_VERSION_V1 => INTEL_NETWORK_GRAPH_V1_ROW_HEADER_LEN,
            INTEL_NETWORK_GRAPH_TRAILER_VERSION_V2 => INTEL_NETWORK_GRAPH_V2_ROW_HEADER_LEN,
            _ => INTEL_NETWORK_GRAPH_V3_ROW_HEADER_LEN,
        };
        if trailer.len().saturating_sub(cursor) < row_header_len {
            errors.push(format!(
                "offset {offset}: intel-network graph row header truncated at byte {cursor}"
            ));
            return (Vec::new(), 0);
        }
        let strategic_operative_index = read_u32_le(trailer, cursor);
        let network_slot = read_u32_le(trailer, cursor + 4);
        let target = read_u32_le(trailer, cursor + 8);
        let state_count = read_u32_le(trailer, cursor + 12) as usize;
        let coverage_offset = if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
            let flags = read_u32_le(trailer, cursor + 16);
            if flags & !INTEL_NETWORK_GRAPH_V3_FLAG_COVERAGE != 0 {
                errors.push(format!(
                    "offset {offset}: intel-network graph v3 row has unknown flags 0x{flags:08x}"
                ));
            }
            if flags & INTEL_NETWORK_GRAPH_V3_FLAG_COVERAGE != 0 {
                Some(cursor + 20)
            } else {
                if trailer[cursor + 20..cursor + INTEL_NETWORK_GRAPH_V3_ROW_HEADER_LEN]
                    .iter()
                    .any(|byte| *byte != 0)
                {
                    errors.push(format!(
                        "offset {offset}: intel-network graph v3 row has non-zero reserved coverage bytes"
                    ));
                }
                None
            }
        } else if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION_V2 {
            Some(cursor + 16)
        } else {
            None
        };
        let coverage = coverage_offset.map(|coverage_offset| IntelNetworkCoverage {
                covered_core_states: read_u32_le(trailer, coverage_offset),
                covered_controlled_states: read_u32_le(trailer, coverage_offset + 4),
                covered_worth: read_u32_le(trailer, coverage_offset + 8),
                controlled_strength_sum_raw: read_i64_le(trailer, coverage_offset + 12),
                core_strength_sum_raw: read_i64_le(trailer, coverage_offset + 20),
                total_core_states: read_u32_le(trailer, coverage_offset + 28),
                total_controlled_states: read_u32_le(trailer, coverage_offset + 32),
                total_worth: read_u32_le(trailer, coverage_offset + 36),
                weight_controlled_raw: read_i64_le(trailer, coverage_offset + 40),
                weight_owned_raw: read_i64_le(trailer, coverage_offset + 48),
                weight_owned_worth_raw: read_i64_le(trailer, coverage_offset + 56),
                national_coverage_raw: read_i64_le(trailer, coverage_offset + 64),
            });
        cursor += row_header_len;
        if state_count == 0 || state_count > MAX_INTEL_NETWORK_STATES {
            errors.push(format!(
                "offset {offset}: intel-network graph row has invalid state_count {state_count}"
            ));
            return (Vec::new(), 0);
        }
        let state_row_len = if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
            12
        } else {
            4
        };
        let states_len = state_count.saturating_mul(state_row_len);
        if trailer.len().saturating_sub(cursor) < states_len {
            errors.push(format!(
                "offset {offset}: intel-network graph state list truncated at byte {cursor}"
            ));
            return (Vec::new(), 0);
        }
        if !valid_tag(target) {
            errors.push(format!(
                "offset {offset}: intel-network graph row has invalid target {target}"
            ));
        }
        if !seen.insert((strategic_operative_index, network_slot)) {
            errors.push(format!(
                "offset {offset}: intel-network graph duplicates operative {strategic_operative_index} slot {network_slot}"
            ));
        }
        if let Some(coverage) = &coverage {
            if coverage.covered_core_states > coverage.total_core_states
                || coverage.covered_controlled_states > coverage.total_controlled_states
                || coverage.covered_worth > coverage.total_worth
                || coverage.controlled_strength_sum_raw < 0
                || coverage.core_strength_sum_raw < 0
                || coverage.weight_controlled_raw < 0
                || coverage.weight_owned_raw < 0
                || coverage.weight_owned_worth_raw < 0
                || !(0..=100_000).contains(&coverage.national_coverage_raw)
                || recompute_intel_network_national_coverage(coverage)
                    != Some(coverage.national_coverage_raw)
            {
                errors.push(format!(
                    "offset {offset}: intel-network graph coverage inputs are invalid"
                ));
            }
        }
        let mut state_ids = Vec::with_capacity(state_count);
        let mut state_strengths = (version == INTEL_NETWORK_GRAPH_TRAILER_VERSION)
            .then(|| Vec::with_capacity(state_count));
        let mut previous = None;
        for _ in 0..state_count {
            let state_id = read_u32_le(trailer, cursor);
            cursor += 4;
            let strength_raw = if version == INTEL_NETWORK_GRAPH_TRAILER_VERSION {
                let value = read_i64_le(trailer, cursor);
                cursor += 8;
                Some(value)
            } else {
                None
            };
            if state_id == 0 || state_id > MAX_INTEL_NETWORK_STATE_ID {
                errors.push(format!(
                    "offset {offset}: intel-network graph row has invalid state_id {state_id}"
                ));
            }
            if previous.is_some_and(|last| state_id <= last) {
                errors.push(format!(
                    "offset {offset}: intel-network graph state_ids are not strictly increasing"
                ));
            }
            previous = Some(state_id);
            state_ids.push(state_id);
            if let Some(strength_raw) = strength_raw {
                if !(0..=compact_contract::build_profile::DOMAIN53_INTEL_NETWORK_STRENGTH_SCALE)
                    .contains(&strength_raw)
                {
                    errors.push(format!(
                        "offset {offset}: intel-network graph state {state_id} has invalid strength_raw {strength_raw}"
                    ));
                }
                state_strengths
                    .as_mut()
                    .expect("v3 strength vector initialized")
                    .push(IntelNetworkStateStrength {
                        state_id,
                        strength_raw,
                    });
            }
        }
        rows.push(IntelNetworkGraphRow {
            strategic_operative_index,
            network_slot,
            target_tag_db_id: target,
            state_ids,
            state_strengths,
            coverage,
        });
    }
    (rows, cursor)
}

/// Decode + gate the `FACS` faction-roster trailer at `trailer_offset`. Strict:
/// unique faction_id, valid leader/member tag_db_ids, leader present in members,
/// sorted+deduped member list, exact-end consumption. Returns rows + bytes
/// consumed (0 on any error).
fn decode_faction_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> (Vec<FactionRow>, bool, usize) {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: faction trailer offset {trailer_offset} beyond payload"
        ));
        return (Vec::new(), false, 0);
    };
    if trailer.len() < FACTION_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: faction trailer is {} byte(s), expected at least {}",
            trailer.len(),
            FACTION_TRAILER_HEADER_LEN
        ));
        return (Vec::new(), false, 0);
    }
    if &trailer[0..4] != FACTION_TRAILER_MAGIC {
        errors.push(format!("offset {offset}: faction trailer magic is invalid"));
        return (Vec::new(), false, 0);
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    if !matches!(
        version,
        FACTION_TRAILER_VERSION_V1_NONEMPTY | FACTION_TRAILER_VERSION
    ) || header_len != FACTION_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: faction trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return (Vec::new(), false, 0);
    }
    if version == FACTION_TRAILER_VERSION_V1_NONEMPTY && row_count == 0 {
        errors.push(format!(
            "offset {offset}: legacy faction trailer cannot witness an empty roster"
        ));
        return (Vec::new(), false, 0);
    }
    if row_count > MAX_FACTION_ROWS {
        errors.push(format!(
            "offset {offset}: faction row_count {row_count} exceeds {MAX_FACTION_ROWS}"
        ));
        return (Vec::new(), false, 0);
    }

    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    let mut cursor = FACTION_TRAILER_HEADER_LEN;
    for _ in 0..row_count {
        if trailer.len().saturating_sub(cursor) < FACTION_ROW_HEADER_LEN {
            errors.push(format!(
                "offset {offset}: faction row header truncated at byte {cursor}"
            ));
            return (Vec::new(), false, 0);
        }
        let faction_id = read_u32_le(trailer, cursor);
        let faction_type = read_u32_le(trailer, cursor + 4);
        let name_hash = read_u32_le(trailer, cursor + 8);
        let leader = read_u32_le(trailer, cursor + 12);
        let member_count = read_u32_le(trailer, cursor + 16) as usize;
        cursor += FACTION_ROW_HEADER_LEN;
        if member_count == 0 || member_count > MAX_FACTION_MEMBERS {
            errors.push(format!(
                "offset {offset}: faction row has invalid member_count {member_count}"
            ));
            return (Vec::new(), false, 0);
        }
        let members_len = member_count.saturating_mul(4);
        if trailer.len().saturating_sub(cursor) < members_len {
            errors.push(format!(
                "offset {offset}: faction member list truncated at byte {cursor}"
            ));
            return (Vec::new(), false, 0);
        }
        if !seen.insert(faction_id) {
            errors.push(format!(
                "offset {offset}: faction trailer duplicates faction_id {faction_id}"
            ));
        }
        if !valid_tag(leader) {
            errors.push(format!(
                "offset {offset}: faction row {faction_id} has invalid leader {leader}"
            ));
        }
        let mut member_tag_db_ids = Vec::with_capacity(member_count);
        let mut previous = None;
        for _ in 0..member_count {
            let member = read_u32_le(trailer, cursor);
            cursor += 4;
            if !valid_tag(member) {
                errors.push(format!(
                    "offset {offset}: faction row {faction_id} has invalid member {member}"
                ));
            }
            if previous.is_some_and(|last| member <= last) {
                errors.push(format!(
                    "offset {offset}: faction row {faction_id} members are not strictly increasing"
                ));
            }
            previous = Some(member);
            member_tag_db_ids.push(member);
        }
        if !member_tag_db_ids.contains(&leader) {
            errors.push(format!(
                "offset {offset}: faction row {faction_id} leader {leader} not present in members"
            ));
        }
        rows.push(FactionRow {
            faction_id,
            faction_type,
            name_hash,
            leader_tag_db_id: leader,
            member_tag_db_ids,
        });
    }
    (rows, true, cursor)
}

pub fn decode_payload(offset: usize, payload: &[u8]) -> Domain53GateSummary {
    let mut summary = Domain53GateSummary::default();
    if payload.len() < HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12) as usize;
    let status_flags = read_u32_le(payload, 16);
    let schema_version = read_u32_le(payload, 20);
    let fixed_scale_raw = read_u32_le(payload, 24);
    let mut expected_counts = ExpectedCounts {
        country: read_u32_le(payload, 28) as usize,
        war: read_u32_le(payload, 32) as usize,
        opinion: read_u32_le(payload, 36) as usize,
        relation_flag: read_u32_le(payload, 40) as usize,
        subject: read_u32_le(payload, 44) as usize,
        agency: read_u32_le(payload, 48) as usize,
        operative: read_u32_le(payload, 52) as usize,
        cryptology: read_u32_le(payload, 56) as usize,
        operation_anchor: read_u32_le(payload, 60) as usize,
        intel_map: read_u32_le(payload, 64) as usize,
        agency_capacity_cluster: read_u32_le(payload, 68) as usize,
        compact_state_event: 0,
    };
    let header_count_sum = expected_counts.sum();
    if header_count_sum <= row_count {
        expected_counts.compact_state_event = row_count - header_count_sum;
    }

    summary.status_flags_or |= status_flags;
    if schema_version != SCHEMA_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel schema_version {schema_version} != {SCHEMA_VERSION}"
        ));
    }
    if fixed_scale_raw != FIXED_SCALE_RAW {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel fixed_scale_raw {fixed_scale_raw} != {FIXED_SCALE_RAW}"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if header_count_sum > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel row_count {row_count} is smaller than sum of header per-kind counts {header_count_sum}"
        ));
    }

    let mut cursor = HEADER_LEN;
    let mut decoded_counts = ExpectedCounts::default();
    let mut seen_countries = BTreeSet::new();
    let mut seen_wars = BTreeSet::new();
    let mut seen_opinions = PairSeenSet::default();
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();
    let mut seen_agencies = BTreeSet::new();
    let mut seen_agency_capacity_clusters = BTreeSet::new();
    let mut seen_operatives = BTreeSet::new();
    let mut seen_crypto = BTreeSet::new();
    let mut seen_operations = BTreeSet::new();
    let mut seen_intel_map = BTreeSet::new();
    let mut seen_state_events = StateEventSeenSet::default();
    let mut country_war_projection_rows = 0usize;
    let mut live_country_rows = 0usize;
    let mut live_war_rows = 0usize;
    let mut operation_detail_resisted_rows = 0usize;
    let mut intel_map_resisted_rows = 0usize;
    let mut operative_lifecycle_rows = 0usize;
    let mut state_event_source_counts = StateEventSourceCounts::default();

    if status_flags & !KNOWN_STATUS_FLAGS != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel status_flags has unknown bits 0x{:X}",
            status_flags & !KNOWN_STATUS_FLAGS
        ));
    }

    for row_index in 0..row_count {
        if cursor + 2 > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy/intel row {row_index} starts past payload end"
            ));
            return summary;
        }
        let kind = read_u16_le(payload, cursor);
        let Some(row_len) = row_len(kind) else {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy/intel row {row_index} has unknown kind {kind}"
            ));
            return summary;
        };
        if cursor + row_len > payload.len() {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy/intel row {row_index} kind {kind} truncated: need {row_len} byte(s)"
            ));
            return summary;
        }
        if domain53_base_row_deleted(kind, payload, cursor) && status_flags & STATUS_DELTA_DATA == 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: diplomacy/intel row {row_index} tombstone appears outside delta frame"
            ));
        }
        match kind {
            KIND_COUNTRY_DIPLOMACY => {
                decoded_counts.country += 1;
                let flags = read_u16_le(payload, cursor + 2);
                let deleted = domain53_base_row_deleted(kind, payload, cursor);
                if !deleted {
                    live_country_rows = live_country_rows.saturating_add(1);
                }
                if !deleted && flags & (COUNTRY_FLAG_AT_WAR | COUNTRY_FLAG_CASUALTIES) != 0 {
                    country_war_projection_rows = country_war_projection_rows.saturating_add(1);
                }
                validate_country_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_countries,
                    &mut summary,
                );
            }
            KIND_WAR_RELATION => {
                decoded_counts.war += 1;
                if !domain53_base_row_deleted(kind, payload, cursor) {
                    live_war_rows = live_war_rows.saturating_add(1);
                }
                validate_war_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_wars,
                    &mut summary,
                );
            }
            KIND_OPINION => {
                decoded_counts.opinion += 1;
                validate_opinion_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_opinions,
                    &mut summary,
                );
            }
            KIND_RELATION_FLAG => {
                decoded_counts.relation_flag += 1;
                validate_relation_flag_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_flags,
                    &mut summary,
                );
            }
            KIND_SUBJECT => {
                decoded_counts.subject += 1;
                validate_subject_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_subjects,
                    &mut summary,
                );
            }
            KIND_AGENCY => {
                decoded_counts.agency += 1;
                validate_agency_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_agencies,
                    &mut summary,
                );
            }
            KIND_AGENCY_CAPACITY_CLUSTER => {
                decoded_counts.agency_capacity_cluster += 1;
                validate_agency_capacity_cluster_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_agency_capacity_clusters,
                    &mut summary,
                );
            }
            KIND_OPERATIVE => {
                decoded_counts.operative += 1;
                let flags = read_u16_le(payload, cursor + 2);
                if flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT != 0 {
                    operative_lifecycle_rows = operative_lifecycle_rows.saturating_add(1);
                }
                validate_operative_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_operatives,
                    &mut summary,
                );
            }
            KIND_CRYPTOLOGY => {
                decoded_counts.cryptology += 1;
                validate_crypto_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_crypto,
                    &mut summary,
                );
            }
            KIND_OPERATION_ANCHOR => {
                decoded_counts.operation_anchor += 1;
                let flags = read_u16_le(payload, cursor + 2);
                if flags & OPERATION_FLAG_DETAIL_RESISTED != 0 {
                    operation_detail_resisted_rows =
                        operation_detail_resisted_rows.saturating_add(1);
                }
                validate_operation_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_operations,
                    &mut summary,
                );
            }
            KIND_INTEL_MAP => {
                decoded_counts.intel_map += 1;
                let flags = read_u16_le(payload, cursor + 2);
                if flags & INTEL_MAP_FLAG_RESISTED != 0 {
                    intel_map_resisted_rows = intel_map_resisted_rows.saturating_add(1);
                }
                validate_intel_map_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_intel_map,
                    &mut summary,
                );
            }
            KIND_COMPACT_STATE_EVENT => {
                decoded_counts.compact_state_event += 1;
                if !domain53_base_row_deleted(kind, payload, cursor) {
                    state_event_source_counts.add(read_u16_le(payload, cursor + 14));
                }
                validate_state_event_row(
                    offset,
                    row_index,
                    payload,
                    cursor,
                    &mut seen_state_events,
                    &mut summary,
                );
            }
            _ => unreachable!("row_len accepted only known kinds"),
        }
        cursor += row_len;
    }

    // Any bytes after the self-delimiting base payload are append-only
    // trailers (`OPIN`, `DIAQ`); anything else is a trailing-byte error.
    let mut operation_instance_rows = 0usize;
    let mut operation_instance_country_rows = 0usize;
    let mut operation_instance_def_ref_ok_rows = 0usize;
    let mut incoming_action_queue_rows = 0usize;
    let mut incoming_action_queue_recipients = BTreeSet::new();
    let mut intel_network_graph_rows = 0usize;
    let mut intel_network_graph_state_rows = 0usize;
    let mut faction_rows = 0usize;
    let mut faction_member_rows = 0usize;
    if cursor != payload.len() {
        let trailer_rows =
            decode_domain53_trailers(offset, payload, cursor, &mut summary.parse_errors);
        operation_instance_rows = trailer_rows.operation_instances.len();
        operation_instance_country_rows = trailer_rows
            .operation_instances
            .iter()
            .filter(|row| row.associated_country_tag_db_id.is_some())
            .count();
        operation_instance_def_ref_ok_rows = trailer_rows
            .operation_instances
            .iter()
            .filter(|row| row.def_ref_ok)
            .count();
        incoming_action_queue_rows = trailer_rows.incoming_actions.len();
        incoming_action_queue_recipients.extend(
            trailer_rows
                .incoming_actions
                .iter()
                .map(|row| row.recipient_tag_db_id),
        );
        intel_network_graph_rows = trailer_rows.intel_network_graphs.len();
        intel_network_graph_state_rows = trailer_rows
            .intel_network_graphs
            .iter()
            .map(|row| row.state_ids.len())
            .sum();
        faction_rows = trailer_rows.factions.len();
        faction_member_rows = trailer_rows
            .factions
            .iter()
            .map(|row| row.member_tag_db_ids.len())
            .sum();
    }
    add_unit(
        &mut summary,
        "operation_instance_rows",
        operation_instance_rows,
    );
    add_unit(
        &mut summary,
        "operation_instance_country_rows",
        operation_instance_country_rows,
    );
    add_unit(
        &mut summary,
        "operation_instance_def_ref_ok_rows",
        operation_instance_def_ref_ok_rows,
    );
    add_unit(
        &mut summary,
        "incoming_action_queue_rows",
        incoming_action_queue_rows,
    );
    add_unit(
        &mut summary,
        "incoming_action_queue_recipient_rows",
        incoming_action_queue_recipients.len(),
    );
    add_unit(
        &mut summary,
        "intel_network_graph_rows",
        intel_network_graph_rows,
    );
    add_unit(
        &mut summary,
        "intel_network_graph_state_rows",
        intel_network_graph_state_rows,
    );
    add_unit(&mut summary, "faction_rows", faction_rows);
    add_unit(&mut summary, "faction_member_rows", faction_member_rows);
    if decoded_counts != expected_counts {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel per-kind counts decoded {:?} != header {:?}",
            decoded_counts, expected_counts
        ));
    }
    if status_flags & STATUS_WAR_ROOT_UNVALIDATED != 0
        && (live_war_rows != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges war root unvalidated but emits {} war relation row(s) and {} country war projection row(s)",
            live_war_rows, country_war_projection_rows
        ));
    }
    if status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED == 0
        && (live_war_rows != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel emits {} war relation row(s) and {} country war projection row(s) without STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED; schema v{SCHEMA_VERSION} has no war name/grouping/start-date fields",
            live_war_rows, country_war_projection_rows
        ));
    }
    if operation_detail_resisted_rows != 0 && status_flags & STATUS_OPERATIONS_DETAIL_RESISTED == 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel has {operation_detail_resisted_rows} resisted operation row(s) without STATUS_OPERATIONS_DETAIL_RESISTED"
        ));
    }
    if intel_map_resisted_rows != 0 && status_flags & STATUS_INTEL_MAP_RESISTED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel has {intel_map_resisted_rows} resisted intel-map row(s) without STATUS_INTEL_MAP_RESISTED"
        ));
    }
    if status_flags & STATUS_FACTION_GROUP_UNVALIDATED != 0
        && (live_country_rows != 0 || state_event_source_counts.country_faction_group != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges faction group unvalidated but emits {} country diplomacy row(s) and {} faction state event row(s)",
            live_country_rows, state_event_source_counts.country_faction_group
        ));
    }
    if status_flags & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED != 0 && intel_network_graph_rows != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges intel-network graph unvalidated but emits {intel_network_graph_rows} graph row(s)"
        ));
    }

    summary.data_rows_total = row_count as u64;
    summary.data_changed_rows_total = changed_rows as u64;
    add_unit(&mut summary, "diplomacy_intel_rows", row_count);
    add_unit(
        &mut summary,
        "diplomacy_country_rows",
        decoded_counts.country,
    );
    add_unit(&mut summary, "war_relation_rows", decoded_counts.war);
    add_unit(&mut summary, "opinion_rows", decoded_counts.opinion);
    add_unit(
        &mut summary,
        "relation_flag_rows",
        decoded_counts.relation_flag,
    );
    add_unit(&mut summary, "subject_rows", decoded_counts.subject);
    add_unit(&mut summary, "intel_agency_rows", decoded_counts.agency);
    add_unit(
        &mut summary,
        "agency_capacity_cluster_rows",
        decoded_counts.agency_capacity_cluster,
    );
    add_unit(&mut summary, "operative_rows", decoded_counts.operative);
    add_unit(
        &mut summary,
        "operative_lifecycle_rows",
        operative_lifecycle_rows,
    );
    add_unit(&mut summary, "cryptology_rows", decoded_counts.cryptology);
    add_unit(
        &mut summary,
        "operation_anchor_rows",
        decoded_counts.operation_anchor,
    );
    add_unit(&mut summary, "intel_map_rows", decoded_counts.intel_map);
    add_unit(
        &mut summary,
        "compact_state_event_rows",
        decoded_counts.compact_state_event,
    );
    add_state_event_source_units(&mut summary, state_event_source_counts);
    summary
}

pub fn decode_snapshot_payload(
    offset: usize,
    payload: &[u8],
) -> Result<DiplomacyIntelSnapshot, Vec<String>> {
    let summary = decode_payload(offset, payload);
    if !summary.parse_errors.is_empty() {
        return Err(summary.parse_errors);
    }
    if payload.len() < HEADER_LEN {
        return Err(vec![format!(
            "offset {offset}: diplomacy/intel payload is {} byte(s), expected at least {HEADER_LEN}",
            payload.len()
        )]);
    }
    let elapsed_wall_ms = read_u64_le(payload, 0);
    let row_count = read_u32_le(payload, 8) as usize;
    let changed_rows = read_u32_le(payload, 12);
    let status_flags = read_u32_le(payload, 16);
    let mut snapshot = DiplomacyIntelSnapshot {
        elapsed_wall_ms,
        status_flags,
        changed_rows: Some(changed_rows),
        ..DiplomacyIntelSnapshot::default()
    };
    let mut cursor = HEADER_LEN;
    for _ in 0..row_count {
        let kind = read_u16_le(payload, cursor);
        match kind {
            KIND_COUNTRY_DIPLOMACY => {
                snapshot.countries.push(CountryDiplomacyRow {
                    tag_db_id: read_u32_le(payload, cursor + 4),
                    faction_group: read_i32_le(payload, cursor + 8),
                    at_war_count: read_u32_le(payload, cursor + 12),
                    casualties: read_i64_le(payload, cursor + 16),
                });
            }
            KIND_WAR_RELATION => {
                snapshot.wars.push(WarRelationRow {
                    first_tag_db_id: read_u32_le(payload, cursor + 4),
                    second_tag_db_id: read_u32_le(payload, cursor + 8),
                    first_casualties: read_i64_le(payload, cursor + 12),
                    second_casualties: read_i64_le(payload, cursor + 20),
                });
            }
            KIND_OPINION => {
                snapshot.opinions.push(OpinionRow {
                    src_tag_db_id: read_u32_le(payload, cursor + 4),
                    target_tag_db_id: read_u32_le(payload, cursor + 8),
                    opinion: read_i32_le(payload, cursor + 12),
                });
            }
            KIND_RELATION_FLAG => {
                let flags = read_u32_le(payload, cursor + 4);
                snapshot.relation_flags.push(RelationFlagRow {
                    src_tag_db_id: read_u32_le(payload, cursor + 8),
                    target_tag_db_id: read_u32_le(payload, cursor + 12),
                    flag_kind: read_u16_le(payload, cursor + 2),
                    directed: flags & REL_FLAG_DIRECTED != 0,
                });
            }
            KIND_SUBJECT => {
                snapshot.subjects.push(SubjectRow {
                    overlord_tag_db_id: read_u32_le(payload, cursor + 4),
                    subject_tag_db_id: read_u32_le(payload, cursor + 8),
                    autonomy_state_hash: read_u32_le(payload, cursor + 12),
                    autonomy_ratio_raw: read_i32_le(payload, cursor + 16),
                    start_date_raw: read_i32_le(payload, cursor + 20),
                });
            }
            KIND_AGENCY => {
                snapshot.agencies.push(AgencyRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    max_operatives: read_u32_le(payload, cursor + 8),
                    usable_slots: read_u32_le(payload, cursor + 12),
                    operative_count: read_u32_le(payload, cursor + 16),
                    cryptology_target_count: read_u32_le(payload, cursor + 20),
                    defense_raw: read_i32_le(payload, cursor + 24),
                    upgrade_progress_raw: read_i32_le(payload, cursor + 28),
                    name_hash: read_u64_le(payload, cursor + 32),
                    icon_hash: read_u64_le(payload, cursor + 40),
                });
            }
            KIND_AGENCY_CAPACITY_CLUSTER => {
                snapshot
                    .agency_capacity_clusters
                    .push(AgencyCapacityClusterRow {
                        owner_tag_db_id: read_u32_le(payload, cursor + 4),
                        value_0xe4: read_u32_le(payload, cursor + 8),
                        value_0xf0: read_u32_le(payload, cursor + 12),
                        value_0xf4: read_u32_le(payload, cursor + 16),
                        proof_hash: read_u32_le(payload, cursor + 20),
                    });
            }
            KIND_OPERATIVE => {
                let flags = read_u16_le(payload, cursor + 2);
                snapshot.operatives.push(OperativeRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    op_id: read_u32_le(payload, cursor + 8),
                    detected: flags & OPERATIVE_FLAG_DETECTED != 0,
                    captured: flags & OPERATIVE_FLAG_CAPTURED != 0,
                    skill: (flags & OPERATIVE_FLAG_SKILL_PRESENT != 0)
                        .then(|| read_u32_le(payload, cursor + 12)),
                    experience_raw: (flags & OPERATIVE_FLAG_EXPERIENCE_PRESENT != 0)
                        .then(|| read_i64_le(payload, cursor + 16)),
                    name_hash: if flags & OPERATIVE_FLAG_NAME_HASH != 0 {
                        read_u64_le(payload, cursor + 24)
                    } else {
                        0
                    },
                    state_hash: read_u32_le(payload, cursor + 32),
                    lifecycle_flag: (flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT != 0)
                        .then(|| read_u32_le(payload, cursor + 36)),
                });
            }
            KIND_CRYPTOLOGY => {
                let flags = read_u16_le(payload, cursor + 2);
                snapshot.cryptology.push(CryptologyRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    target_tag_db_id: read_u32_le(payload, cursor + 8),
                    amount_raw: read_i64_le(payload, cursor + 12),
                    decrypting: flags & CRYPTO_FLAG_DECRYPTING != 0,
                });
            }
            KIND_OPERATION_ANCHOR => {
                let flags = read_u16_le(payload, cursor + 2);
                snapshot.operation_anchors.push(OperationAnchorRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    operation_id: read_u32_le(payload, cursor + 8),
                    target_tag_db_id: (flags & OPERATION_FLAG_TARGET_PRESENT != 0)
                        .then(|| read_u32_le(payload, cursor + 12)),
                    slot_count: read_u32_le(payload, cursor + 16),
                    phase_count: read_u32_le(payload, cursor + 20),
                    detail_resisted: flags & OPERATION_FLAG_DETAIL_RESISTED != 0,
                });
            }
            KIND_INTEL_MAP => {
                let flags = read_u16_le(payload, cursor + 2);
                snapshot.intel_map.push(IntelMapRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    target_tag_db_id: read_u32_le(payload, cursor + 8),
                    civilian_raw: read_i32_le(payload, cursor + 12),
                    army_raw: read_i32_le(payload, cursor + 16),
                    navy_raw: read_i32_le(payload, cursor + 20),
                    air_raw: read_i32_le(payload, cursor + 24),
                    resisted: flags & INTEL_MAP_FLAG_RESISTED != 0,
                });
            }
            KIND_COMPACT_STATE_EVENT => {
                let flags = read_u16_le(payload, cursor + 2);
                snapshot.state_events.push(StateEventRow {
                    owner_tag_db_id: read_u32_le(payload, cursor + 4),
                    target_tag_db_id: (flags & STATE_EVENT_FLAG_TARGET_PRESENT != 0)
                        .then(|| read_u32_le(payload, cursor + 8)),
                    event_kind: read_u16_le(payload, cursor + 12),
                    source_kind: read_u16_le(payload, cursor + 14),
                    object_hash: read_u32_le(payload, cursor + 16),
                    value_hash: read_u32_le(payload, cursor + 20),
                    value_raw: read_i64_le(payload, cursor + 24),
                });
            }
            _ => unreachable!("decode_payload already validated row kind"),
        }
        cursor += row_len(kind).expect("decode_payload already validated row kind");
    }
    // Parse append-only trailers (validated above by decode_payload).
    if cursor != payload.len() {
        let mut trailer_errors = Vec::new();
        let trailers = decode_domain53_trailers(offset, payload, cursor, &mut trailer_errors);
        snapshot.operation_instances = trailers.operation_instances;
        snapshot.incoming_actions = trailers.incoming_actions;
        snapshot.intel_network_graphs = trailers.intel_network_graphs;
        snapshot.factions = trailers.factions;
        snapshot.faction_roster_complete = trailers.faction_roster_complete;
        snapshot.subject_autonomy_statuses = trailers.subject_autonomy_statuses;
        snapshot.subject_autonomy_status_complete =
            trailers.subject_autonomy_status_complete;
        if !trailer_errors.is_empty() {
            return Err(trailer_errors);
        }
    }
    if snapshot.subject_autonomy_status_complete
        && snapshot.status_flags & STATUS_DELTA_DATA == 0
    {
        let subject_keys = snapshot
            .subjects
            .iter()
            .map(|row| (row.overlord_tag_db_id, row.subject_tag_db_id))
            .collect::<BTreeSet<_>>();
        let status_keys = snapshot
            .subject_autonomy_statuses
            .iter()
            .map(|row| (row.overlord_tag_db_id, row.subject_tag_db_id))
            .collect::<BTreeSet<_>>();
        if subject_keys != status_keys {
            return Err(vec![format!(
                "offset {offset}: complete subject-autonomy status keys do not match subject base rows"
            )]);
        }
    }
    Ok(snapshot)
}

pub fn stable_hash64(value: &str) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in value.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn stable_hash32(value: &str) -> u32 {
    (stable_hash64(value) & 0xffff_ffff) as u32
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ExpectedCounts {
    country: usize,
    war: usize,
    opinion: usize,
    relation_flag: usize,
    subject: usize,
    agency: usize,
    agency_capacity_cluster: usize,
    operative: usize,
    cryptology: usize,
    operation_anchor: usize,
    intel_map: usize,
    compact_state_event: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct StateEventSourceCounts {
    relation_flag: usize,
    relation_opinion: usize,
    subject_autonomy: usize,
    country_faction_group: usize,
    agency_scalar: usize,
    operative_lifecycle: usize,
    agency_capacity: usize,
    cryptology: usize,
}

impl StateEventSourceCounts {
    fn add(&mut self, source_kind: u16) {
        match source_kind {
            STATE_EVENT_SOURCE_RELATION_FLAG => {
                self.relation_flag = self.relation_flag.saturating_add(1)
            }
            STATE_EVENT_SOURCE_RELATION_OPINION => {
                self.relation_opinion = self.relation_opinion.saturating_add(1)
            }
            STATE_EVENT_SOURCE_SUBJECT_AUTONOMY => {
                self.subject_autonomy = self.subject_autonomy.saturating_add(1)
            }
            STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP => {
                self.country_faction_group = self.country_faction_group.saturating_add(1)
            }
            STATE_EVENT_SOURCE_AGENCY_SCALAR => {
                self.agency_scalar = self.agency_scalar.saturating_add(1)
            }
            STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE => {
                self.operative_lifecycle = self.operative_lifecycle.saturating_add(1)
            }
            STATE_EVENT_SOURCE_AGENCY_CAPACITY => {
                self.agency_capacity = self.agency_capacity.saturating_add(1)
            }
            STATE_EVENT_SOURCE_CRYPTOLOGY => self.cryptology = self.cryptology.saturating_add(1),
            _ => {}
        }
    }
}

fn add_state_event_source_units(summary: &mut Domain53GateSummary, counts: StateEventSourceCounts) {
    add_unit(
        summary,
        "compact_state_event_relation_flag_rows",
        counts.relation_flag,
    );
    add_unit(
        summary,
        "compact_state_event_relation_opinion_rows",
        counts.relation_opinion,
    );
    add_unit(
        summary,
        "compact_state_event_subject_autonomy_rows",
        counts.subject_autonomy,
    );
    add_unit(
        summary,
        "compact_state_event_country_faction_group_rows",
        counts.country_faction_group,
    );
    add_unit(
        summary,
        "compact_state_event_agency_scalar_rows",
        counts.agency_scalar,
    );
    add_unit(
        summary,
        "compact_state_event_operative_lifecycle_rows",
        counts.operative_lifecycle,
    );
    add_unit(
        summary,
        "compact_state_event_agency_capacity_rows",
        counts.agency_capacity,
    );
    add_unit(
        summary,
        "compact_state_event_cryptology_rows",
        counts.cryptology,
    );
}

impl ExpectedCounts {
    fn sum(self) -> usize {
        self.country
            .saturating_add(self.war)
            .saturating_add(self.opinion)
            .saturating_add(self.relation_flag)
            .saturating_add(self.subject)
            .saturating_add(self.agency)
            .saturating_add(self.agency_capacity_cluster)
            .saturating_add(self.operative)
            .saturating_add(self.cryptology)
            .saturating_add(self.operation_anchor)
            .saturating_add(self.intel_map)
            .saturating_add(self.compact_state_event)
    }
}

fn row_len(kind: u16) -> Option<usize> {
    match kind {
        KIND_COUNTRY_DIPLOMACY => Some(ROW_COUNTRY_LEN),
        KIND_WAR_RELATION => Some(ROW_WAR_LEN),
        KIND_OPINION => Some(ROW_OPINION_LEN),
        KIND_RELATION_FLAG => Some(ROW_RELATION_FLAG_LEN),
        KIND_SUBJECT => Some(ROW_SUBJECT_LEN),
        KIND_AGENCY => Some(ROW_AGENCY_LEN),
        KIND_AGENCY_CAPACITY_CLUSTER => Some(ROW_AGENCY_CAPACITY_CLUSTER_LEN),
        KIND_OPERATIVE => Some(ROW_OPERATIVE_LEN),
        KIND_CRYPTOLOGY => Some(ROW_CRYPTOLOGY_LEN),
        KIND_OPERATION_ANCHOR => Some(ROW_OPERATION_ANCHOR_LEN),
        KIND_INTEL_MAP => Some(ROW_INTEL_MAP_LEN),
        KIND_COMPACT_STATE_EVENT => Some(ROW_COMPACT_STATE_EVENT_LEN),
        _ => None,
    }
}

fn validate_country_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let tag_db_id = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(tag_db_id);
    let tag_key = row_id_clear_deleted(tag_db_id);
    let faction_group = read_i32_le(payload, cursor + 8);
    let at_war_count = read_u32_le(payload, cursor + 12);
    let casualties = read_i64_le(payload, cursor + 16);
    if deleted {
        if !valid_tag(tag_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy tombstone row {row_index} has invalid tag_db_id {tag_key}"
            ));
        } else if !seen.insert(tag_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy row {row_index} duplicates tag_db_id {tag_key}"
            ));
        }
        if flags != 0 || faction_group != 0 || at_war_count != 0 || casualties != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(tag_db_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has invalid tag_db_id {tag_db_id}"
        ));
    } else if !seen.insert(tag_db_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} duplicates tag_db_id {tag_db_id}"
        ));
    }
    if flags & !(COUNTRY_FLAG_AT_WAR | COUNTRY_FLAG_CASUALTIES | COUNTRY_FLAG_FACTION_GROUP) != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & COUNTRY_FLAG_AT_WAR == 0 && at_war_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has at_war_count without flag"
        ));
    }
    if flags & COUNTRY_FLAG_CASUALTIES == 0 && casualties != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has casualties without flag"
        ));
    }
    if casualties < 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has negative casualties"
        ));
    }
    let _ = faction_group;
}

fn validate_war_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let first = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(first);
    let first_key = row_id_clear_deleted(first);
    let second = read_u32_le(payload, cursor + 8);
    let first_casualties = read_i64_le(payload, cursor + 12);
    let second_casualties = read_i64_le(payload, cursor + 20);
    let reserved = read_u32_le(payload, cursor + 28);
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(
        offset,
        "war relation",
        row_index,
        first_key,
        second,
        summary,
    );
    let key = if first_key <= second {
        (first_key, second)
    } else {
        (second, first_key)
    };
    if valid_tag(first_key) && valid_tag(second) && !seen.insert(key) {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation row {row_index} duplicates pair {first_key}/{second}"
        ));
    }
    if deleted && (first_casualties != 0 || second_casualties != 0 || reserved != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation tombstone row {row_index} has live fields"
        ));
        return;
    }
    for (label, value) in [
        ("first_casualties", first_casualties),
        ("second_casualties", second_casualties),
    ] {
        if !(0..=MAX_CASUALTIES).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: war relation row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_opinion_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut PairSeenSet,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let src = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(src);
    let src_key = row_id_clear_deleted(src);
    let target = read_u32_le(payload, cursor + 8);
    let opinion = read_i32_le(payload, cursor + 12);
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "opinion", row_index, src_key, target, summary);
    if valid_tag(src_key) && valid_tag(target) && !seen.insert(src_key, target) {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} duplicates ordered pair {src_key}/{target}"
        ));
    }
    if deleted {
        if opinion != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: opinion tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !(-MAX_OPINION_ABS..=MAX_OPINION_ABS).contains(&opinion) {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} has out-of-range opinion {opinion}"
        ));
    }
}

fn validate_relation_flag_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u32, u16)>,
    summary: &mut Domain53GateSummary,
) {
    let flag_kind = read_u16_le(payload, cursor + 2);
    let flags = read_u32_le(payload, cursor + 4);
    let src = read_u32_le(payload, cursor + 8);
    let deleted = row_id_deleted(src);
    let src_key = row_id_clear_deleted(src);
    let target = read_u32_le(payload, cursor + 12);
    validate_pair(offset, "relation flag", row_index, src_key, target, summary);
    if !known_relation_kind(flag_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} has unknown flag_kind {flag_kind}"
        ));
    }
    if flags & !REL_FLAG_DIRECTED != 0 || (deleted && flags != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} has unknown flags {flags}"
        ));
    }
    let key = if flags & REL_FLAG_DIRECTED != 0 || src_key <= target {
        (src_key, target, flag_kind)
    } else {
        (target, src_key, flag_kind)
    };
    if valid_tag(src_key) && valid_tag(target) && !seen.insert(key) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} duplicates relation flag"
        ));
    }
}

fn validate_subject_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let overlord = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(overlord);
    let overlord_key = row_id_clear_deleted(overlord);
    let subject = read_u32_le(payload, cursor + 8);
    let autonomy_state_hash = read_u32_le(payload, cursor + 12);
    let autonomy_ratio_raw = read_i32_le(payload, cursor + 16);
    let start_date_raw = read_i32_le(payload, cursor + 20);
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "subject", row_index, overlord_key, subject, summary);
    if valid_tag(overlord_key) && valid_tag(subject) && !seen.insert((overlord_key, subject)) {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} duplicates subject pair"
        ));
    }
    if deleted {
        if autonomy_state_hash != 0 || autonomy_ratio_raw != 0 || start_date_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: subject tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if autonomy_state_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has zero autonomy_state_hash"
        ));
    }
    if !(0..=FIXED_SCALE_RAW as i32).contains(&autonomy_ratio_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has invalid autonomy_ratio_raw {autonomy_ratio_raw}"
        ));
    }
    // start_date_raw is an i32 engine-hour count; a subject cannot start before
    // the epoch, so gate strictly positive. Fail-closed on a torn/zero value.
    if start_date_raw <= 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has non-positive start_date_raw {start_date_raw}"
        ));
    }
}

fn validate_agency_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(owner);
    let owner_key = row_id_clear_deleted(owner);
    let max_operatives = read_u32_le(payload, cursor + 8);
    let usable_slots = read_u32_le(payload, cursor + 12);
    let operative_count = read_u32_le(payload, cursor + 16);
    let cryptology_target_count = read_u32_le(payload, cursor + 20);
    let defense_raw = read_i32_le(payload, cursor + 24);
    let upgrade_progress_raw = read_i32_le(payload, cursor + 28);
    let name_hash = read_u64_le(payload, cursor + 32);
    let icon_hash = read_u64_le(payload, cursor + 40);
    if deleted {
        if !valid_tag(owner_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency tombstone row {row_index} has invalid owner {owner_key}"
            ));
        } else if !seen.insert(owner_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} duplicates owner {owner_key}"
            ));
        }
        if flags != 0
            || max_operatives != 0
            || usable_slots != 0
            || operative_count != 0
            || cryptology_target_count != 0
            || defense_raw != 0
            || upgrade_progress_raw != 0
            || name_hash != 0
            || icon_hash != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: agency tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has invalid owner {owner}"
        ));
    } else if !seen.insert(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} duplicates owner {owner}"
        ));
    }
    if flags & !(AGENCY_FLAG_NAME_HASH | AGENCY_FLAG_ICON_HASH) != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & AGENCY_FLAG_NAME_HASH == 0 && name_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has name_hash without flag"
        ));
    }
    if flags & AGENCY_FLAG_ICON_HASH == 0 && icon_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has icon_hash without flag"
        ));
    }
    for (label, value) in [
        ("max_operatives", max_operatives),
        ("usable_slots", usable_slots),
        ("operative_count", operative_count),
    ] {
        if value > MAX_OPERATIVES_PER_AGENCY {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if cryptology_target_count > 256 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has invalid cryptology_target_count {cryptology_target_count}"
        ));
    }
    for (label, value) in [
        ("defense_raw", defense_raw),
        ("upgrade_progress_raw", upgrade_progress_raw),
    ] {
        if !(0..=MAX_FIXED_RAW).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} has invalid {label} {value}"
            ));
        }
    }
}

fn validate_agency_capacity_cluster_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(owner);
    let owner_key = row_id_clear_deleted(owner);
    let value_0xe4 = read_u32_le(payload, cursor + 8);
    let value_0xf0 = read_u32_le(payload, cursor + 12);
    let value_0xf4 = read_u32_le(payload, cursor + 16);
    let proof_hash = read_u32_le(payload, cursor + 20);
    if deleted {
        if !valid_tag(owner_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster tombstone row {row_index} has invalid owner {owner_key}"
            ));
        } else if !seen.insert(owner_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster row {row_index} duplicates owner {owner_key}"
            ));
        }
        if flags != 0 || value_0xe4 != 0 || value_0xf0 != 0 || value_0xf4 != 0 || proof_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has invalid owner {owner}"
        ));
    } else if !seen.insert(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} duplicates owner {owner}"
        ));
    }
    if flags & !AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4 == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} lacks raw E4/F0/F4 flag"
        ));
    }
    for (label, value) in [
        ("value_0xe4", value_0xe4),
        ("value_0xf0", value_0xf0),
        ("value_0xf4", value_0xf4),
    ] {
        if value > MAX_AGENCY_CAPACITY_CLUSTER_VALUE {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if value_0xe4 != value_0xf4 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has value_0xe4 {value_0xe4} != value_0xf4 {value_0xf4}"
        ));
    }
    if proof_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has zero proof_hash"
        ));
    }
}

fn validate_operative_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let op_id = read_u32_le(payload, cursor + 8);
    let deleted = row_id_deleted(op_id);
    let op_id_key = row_id_clear_deleted(op_id);
    let skill = read_u32_le(payload, cursor + 12);
    let experience_raw = read_i64_le(payload, cursor + 16);
    let name_hash = read_u64_le(payload, cursor + 24);
    let state_hash = read_u32_le(payload, cursor + 32);
    let lifecycle_flag = read_u32_le(payload, cursor + 36);
    if deleted {
        if flags != 0
            || owner != 0
            || skill != 0
            || experience_raw != 0
            || name_hash != 0
            || state_hash != 0
            || lifecycle_flag != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: operative tombstone row {row_index} has live fields"
            ));
        }
        if op_id_key == 0 || op_id_key > MAX_OPERATIVE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: operative tombstone row {row_index} has invalid op_id {op_id_key}"
            ));
        } else if !seen.insert(op_id_key) {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} duplicates op_id {op_id_key}"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid owner {owner}"
        ));
    }
    if op_id == 0 || op_id > MAX_OPERATIVE_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid op_id {op_id}"
        ));
    } else if !seen.insert(op_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} duplicates op_id {op_id}"
        ));
    }
    if flags
        & !(OPERATIVE_FLAG_DETECTED
            | OPERATIVE_FLAG_SKILL_PRESENT
            | OPERATIVE_FLAG_EXPERIENCE_PRESENT
            | OPERATIVE_FLAG_NAME_HASH
            | OPERATIVE_FLAG_CAPTURED
            | OPERATIVE_FLAG_LIFECYCLE_PRESENT)
        != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & OPERATIVE_FLAG_SKILL_PRESENT == 0 && skill != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has skill without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_SKILL_PRESENT != 0 && skill > MAX_OPERATIVE_SKILL {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid skill {skill}"
        ));
    }
    if flags & OPERATIVE_FLAG_EXPERIENCE_PRESENT == 0 && experience_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has experience without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_EXPERIENCE_PRESENT != 0 && !(0..MAX_XP_RAW).contains(&experience_raw)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid experience_raw {experience_raw}"
        ));
    }
    if flags & OPERATIVE_FLAG_NAME_HASH == 0 && name_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has name_hash without flag"
        ));
    }
    if flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT == 0 && lifecycle_flag != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has lifecycle_flag without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT != 0 {
        if !(1..=MAX_OPERATIVE_LIFECYCLE_FLAG).contains(&lifecycle_flag) {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} has invalid lifecycle_flag {lifecycle_flag}"
            ));
        }
        if state_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} has lifecycle flag without state_hash"
            ));
        }
    }
}

fn validate_crypto_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(owner);
    let owner_key = row_id_clear_deleted(owner);
    let target = read_u32_le(payload, cursor + 8);
    let amount_raw = read_i64_le(payload, cursor + 12);
    let reserved0 = read_u64_le(payload, cursor + 20);
    let reserved1 = read_u32_le(payload, cursor + 28);
    validate_pair(offset, "cryptology", row_index, owner_key, target, summary);
    if flags & !CRYPTO_FLAG_DECRYPTING != 0 || (deleted && flags != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} has unknown flags {flags}"
        ));
    }
    if valid_tag(owner_key) && valid_tag(target) && !seen.insert((owner_key, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} duplicates owner/target"
        ));
    }
    if deleted {
        if amount_raw != 0 || reserved0 != 0 || reserved1 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: cryptology tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !(1..MAX_CRYPTO_AMOUNT_RAW).contains(&amount_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} has invalid amount_raw {amount_raw}"
        ));
    }
    if reserved0 != 0 || reserved1 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_operation_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let operation_id = read_u32_le(payload, cursor + 8);
    let target = read_u32_le(payload, cursor + 12);
    let slot_count = read_u32_le(payload, cursor + 16);
    let phase_count = read_u32_le(payload, cursor + 20);
    let reserved = read_u64_le(payload, cursor + 24);
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid owner {owner}"
        ));
    }
    if operation_id == 0 || operation_id > MAX_OPERATION_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid operation_id {operation_id}"
        ));
    } else if !seen.insert(operation_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} duplicates operation_id {operation_id}"
        ));
    }
    if flags
        & !(OPERATION_FLAG_RUNNING | OPERATION_FLAG_DETAIL_RESISTED | OPERATION_FLAG_TARGET_PRESENT)
        != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & OPERATION_FLAG_DETAIL_RESISTED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} lacks detail-resisted/source proof flag"
        ));
    }
    if flags & OPERATION_FLAG_TARGET_PRESENT != 0 {
        if !valid_tag(target) || target == owner {
            summary.parse_errors.push(format!(
                "offset {offset}: operation anchor row {row_index} has invalid target {target}"
            ));
        }
    } else if target != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has target without flag"
        ));
    }
    if slot_count > MAX_OPERATION_SLOTS || phase_count > MAX_OPERATION_PHASES {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid slot/phase counts"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_intel_map_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let target = read_u32_le(payload, cursor + 8);
    let civilian_raw = read_i32_le(payload, cursor + 12);
    let army_raw = read_i32_le(payload, cursor + 16);
    let navy_raw = read_i32_le(payload, cursor + 20);
    let air_raw = read_i32_le(payload, cursor + 24);
    let reserved = read_u32_le(payload, cursor + 28);
    validate_pair(offset, "intel map", row_index, owner, target, summary);
    if flags & !INTEL_MAP_FLAG_RESISTED != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} has unknown flags {flags}"
        ));
    }
    if valid_tag(owner) && valid_tag(target) && !seen.insert((owner, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} duplicates owner/target"
        ));
    }
    for (label, value) in [
        ("civilian_raw", civilian_raw),
        ("army_raw", army_raw),
        ("navy_raw", navy_raw),
        ("air_raw", air_raw),
    ] {
        if !(0..=MAX_INTEL_PERCENT_RAW).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: intel map row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if flags & INTEL_MAP_FLAG_RESISTED != 0
        && (civilian_raw != 0 || army_raw != 0 || navy_raw != 0 || air_raw != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} has resisted flag with emitted values"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} reserved field is nonzero"
        ));
    }
}

fn validate_state_event_row(
    offset: usize,
    row_index: usize,
    payload: &[u8],
    cursor: usize,
    seen: &mut StateEventSeenSet,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let deleted = row_id_deleted(owner);
    let owner_key = row_id_clear_deleted(owner);
    let target = read_u32_le(payload, cursor + 8);
    let event_kind = read_u16_le(payload, cursor + 12);
    let source_kind = read_u16_le(payload, cursor + 14);
    let object_hash = read_u32_le(payload, cursor + 16);
    let value_hash = read_u32_le(payload, cursor + 20);
    let value_raw = read_i64_le(payload, cursor + 24);

    if !valid_tag(owner_key) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has invalid owner {owner_key}"
        ));
    }
    if flags & !STATE_EVENT_FLAG_TARGET_PRESENT != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & STATE_EVENT_FLAG_TARGET_PRESENT != 0 {
        if !valid_tag(target) || target == owner_key {
            summary.parse_errors.push(format!(
                "offset {offset}: compact state event row {row_index} has invalid target {target}"
            ));
        }
    } else if target != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has target without flag"
        ));
    }
    if !known_state_event_kind(event_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown event_kind {event_kind}"
        ));
    }
    if !known_state_event_source(source_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown source_kind {source_kind}"
        ));
    }
    if object_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has zero object_hash"
        ));
    }
    if deleted {
        if value_hash != 0 || value_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: compact state event tombstone row {row_index} has live fields"
            ));
        }
    } else if value_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has zero value_hash"
        ));
    }
    if !deleted && !(-MAX_STATE_EVENT_VALUE_RAW..=MAX_STATE_EVENT_VALUE_RAW).contains(&value_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has invalid value_raw {value_raw}"
        ));
    }

    if object_hash != 0 && !seen.insert(event_kind, source_kind, owner_key, target, object_hash) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} duplicates state event key"
        ));
    }
}

// ---------------------------------------------------------------------------
// Snapshot-field twins of the byte `validate_*_row` functions (W2). Each mirrors
// its byte twin's parse_error push order EXACTLY, reading struct fields and
// re-deriving the encoder-set flags from those fields (same logic as
// `encode_payload`). Checks the encoder cannot violate by construction (e.g.
// "field without flag" when the encoder sets the flag iff the field is nonzero,
// reserved fields the encoder writes 0) are kept identical in shape so the two
// sides stay in lock-step; they simply never fire on a valid encode. The
// governing invariant `summarize_snapshot(o,s) == decode_payload(o,
// &encode_payload(&s))` is proven by the round-trip tests + a sampled production
// cross-check, so any drift here is caught.
// ---------------------------------------------------------------------------

fn summarize_country_row(
    offset: usize,
    row_index: usize,
    row: &CountryDiplomacyRow,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:458..): FACTION_GROUP always; AT_WAR iff
    // at_war_count>0; CASUALTIES iff casualties!=0. So the unknown-flags and
    // "without flag" checks never fire, but we keep them for lock-step.
    let deleted = row_id_deleted(row.tag_db_id);
    let mut flags = 0u16;
    if !deleted {
        flags |= COUNTRY_FLAG_FACTION_GROUP;
        if row.at_war_count > 0 {
            flags |= COUNTRY_FLAG_AT_WAR;
        }
        if row.casualties != 0 {
            flags |= COUNTRY_FLAG_CASUALTIES;
        }
    }
    let tag_db_id = row_id_clear_deleted(row.tag_db_id);
    if deleted {
        if !valid_tag(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy tombstone row {row_index} has invalid tag_db_id {tag_db_id}"
            ));
        } else if !seen.insert(tag_db_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy row {row_index} duplicates tag_db_id {tag_db_id}"
            ));
        }
        if flags != 0 || row.faction_group != 0 || row.at_war_count != 0 || row.casualties != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: country diplomacy tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(tag_db_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has invalid tag_db_id {tag_db_id}"
        ));
    } else if !seen.insert(tag_db_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} duplicates tag_db_id {tag_db_id}"
        ));
    }
    if flags & !(COUNTRY_FLAG_AT_WAR | COUNTRY_FLAG_CASUALTIES | COUNTRY_FLAG_FACTION_GROUP) != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & COUNTRY_FLAG_AT_WAR == 0 && row.at_war_count != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has at_war_count without flag"
        ));
    }
    if flags & COUNTRY_FLAG_CASUALTIES == 0 && row.casualties != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has casualties without flag"
        ));
    }
    if row.casualties < 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: country diplomacy row {row_index} has negative casualties"
        ));
    }
}

fn summarize_war_row(
    offset: usize,
    row_index: usize,
    row: &WarRelationRow,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder writes flags=0 and reserved=0 (encode_payload:474/479).
    let flags = 0u16;
    let deleted = row_id_deleted(row.first_tag_db_id);
    let first = row_id_clear_deleted(row.first_tag_db_id);
    let second = row.second_tag_db_id;
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "war relation", row_index, first, second, summary);
    let key = if first <= second {
        (first, second)
    } else {
        (second, first)
    };
    if valid_tag(first) && valid_tag(second) && !seen.insert(key) {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation row {row_index} duplicates pair {first}/{second}"
        ));
    }
    if deleted && (row.first_casualties != 0 || row.second_casualties != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: war relation tombstone row {row_index} has live fields"
        ));
        return;
    }
    for (label, value) in [
        ("first_casualties", row.first_casualties),
        ("second_casualties", row.second_casualties),
    ] {
        if !(0..=MAX_CASUALTIES).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: war relation row {row_index} has invalid {label} {value}"
            ));
        }
    }
    // reserved is always 0 from the encoder → the nonzero-reserved check never fires.
}

fn summarize_opinion_row(
    offset: usize,
    row_index: usize,
    row: &OpinionRow,
    seen: &mut PairSeenSet,
    summary: &mut Domain53GateSummary,
) {
    let flags = 0u16; // encode_payload:483
    let deleted = row_id_deleted(row.src_tag_db_id);
    let src = row_id_clear_deleted(row.src_tag_db_id);
    let target = row.target_tag_db_id;
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "opinion", row_index, src, target, summary);
    if valid_tag(src) && valid_tag(target) && !seen.insert(src, target) {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} duplicates ordered pair {src}/{target}"
        ));
    }
    if deleted {
        if row.opinion != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: opinion tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !(-MAX_OPINION_ABS..=MAX_OPINION_ABS).contains(&row.opinion) {
        let opinion = row.opinion;
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} has out-of-range opinion {opinion}"
        ));
    }
}

fn summarize_relation_flag_row(
    offset: usize,
    row_index: usize,
    row: &RelationFlagRow,
    seen: &mut BTreeSet<(u32, u32, u16)>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:489): DIRECTED iff row.directed, else 0.
    let deleted = row_id_deleted(row.src_tag_db_id);
    let flags = if !deleted && row.directed {
        REL_FLAG_DIRECTED
    } else {
        0
    };
    let flag_kind = row.flag_kind;
    let src = row_id_clear_deleted(row.src_tag_db_id);
    let target = row.target_tag_db_id;
    validate_pair(offset, "relation flag", row_index, src, target, summary);
    if !known_relation_kind(flag_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} has unknown flag_kind {flag_kind}"
        ));
    }
    if flags & !REL_FLAG_DIRECTED != 0 || (deleted && flags != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} has unknown flags {flags}"
        ));
    }
    let key = if flags & REL_FLAG_DIRECTED != 0 || src <= target {
        (src, target, flag_kind)
    } else {
        (target, src, flag_kind)
    };
    if valid_tag(src) && valid_tag(target) && !seen.insert(key) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} duplicates relation flag"
        ));
    }
}

fn summarize_subject_row(
    offset: usize,
    row_index: usize,
    row: &SubjectRow,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = 0u16; // encode_payload has no subject flags field (KIND + reserved 0)
    let deleted = row_id_deleted(row.overlord_tag_db_id);
    let overlord = row_id_clear_deleted(row.overlord_tag_db_id);
    let subject = row.subject_tag_db_id;
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "subject", row_index, overlord, subject, summary);
    if valid_tag(overlord) && valid_tag(subject) && !seen.insert((overlord, subject)) {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} duplicates subject pair"
        ));
    }
    if deleted {
        if row.autonomy_state_hash != 0 || row.autonomy_ratio_raw != 0 || row.start_date_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: subject tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if row.autonomy_state_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has zero autonomy_state_hash"
        ));
    }
    if !(0..=FIXED_SCALE_RAW as i32).contains(&row.autonomy_ratio_raw) {
        let autonomy_ratio_raw = row.autonomy_ratio_raw;
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has invalid autonomy_ratio_raw {autonomy_ratio_raw}"
        ));
    }
    if row.start_date_raw <= 0 {
        let start_date_raw = row.start_date_raw;
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has non-positive start_date_raw {start_date_raw}"
        ));
    }
}

fn summarize_agency_row(
    offset: usize,
    row_index: usize,
    row: &AgencyRow,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:508..): NAME_HASH iff name_hash!=0; ICON_HASH
    // iff icon_hash!=0.
    let deleted = row_id_deleted(row.owner_tag_db_id);
    let mut flags = 0u16;
    if !deleted {
        if row.name_hash != 0 {
            flags |= AGENCY_FLAG_NAME_HASH;
        }
        if row.icon_hash != 0 {
            flags |= AGENCY_FLAG_ICON_HASH;
        }
    }
    let owner = row_id_clear_deleted(row.owner_tag_db_id);
    if deleted {
        if !valid_tag(owner) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency tombstone row {row_index} has invalid owner {owner}"
            ));
        } else if !seen.insert(owner) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} duplicates owner {owner}"
            ));
        }
        if flags != 0
            || row.max_operatives != 0
            || row.usable_slots != 0
            || row.operative_count != 0
            || row.cryptology_target_count != 0
            || row.defense_raw != 0
            || row.upgrade_progress_raw != 0
            || row.name_hash != 0
            || row.icon_hash != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: agency tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has invalid owner {owner}"
        ));
    } else if !seen.insert(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} duplicates owner {owner}"
        ));
    }
    if flags & !(AGENCY_FLAG_NAME_HASH | AGENCY_FLAG_ICON_HASH) != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & AGENCY_FLAG_NAME_HASH == 0 && row.name_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has name_hash without flag"
        ));
    }
    if flags & AGENCY_FLAG_ICON_HASH == 0 && row.icon_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has icon_hash without flag"
        ));
    }
    for (label, value) in [
        ("max_operatives", row.max_operatives),
        ("usable_slots", row.usable_slots),
        ("operative_count", row.operative_count),
    ] {
        if value > MAX_OPERATIVES_PER_AGENCY {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if row.cryptology_target_count > 256 {
        let cryptology_target_count = row.cryptology_target_count;
        summary.parse_errors.push(format!(
            "offset {offset}: agency row {row_index} has invalid cryptology_target_count {cryptology_target_count}"
        ));
    }
    for (label, value) in [
        ("defense_raw", row.defense_raw),
        ("upgrade_progress_raw", row.upgrade_progress_raw),
    ] {
        if !(0..=MAX_FIXED_RAW).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency row {row_index} has invalid {label} {value}"
            ));
        }
    }
}

fn summarize_agency_capacity_cluster_row(
    offset: usize,
    row_index: usize,
    row: &AgencyCapacityClusterRow,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder always writes AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4 (encode_payload:529).
    let deleted = row_id_deleted(row.owner_tag_db_id);
    let flags = if deleted {
        0
    } else {
        AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4
    };
    let owner = row_id_clear_deleted(row.owner_tag_db_id);
    if deleted {
        if !valid_tag(owner) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster tombstone row {row_index} has invalid owner {owner}"
            ));
        } else if !seen.insert(owner) {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster row {row_index} duplicates owner {owner}"
            ));
        }
        if flags != 0
            || row.value_0xe4 != 0
            || row.value_0xf0 != 0
            || row.value_0xf4 != 0
            || row.proof_hash != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has invalid owner {owner}"
        ));
    } else if !seen.insert(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} duplicates owner {owner}"
        ));
    }
    if flags & !AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4 == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} lacks raw E4/F0/F4 flag"
        ));
    }
    for (label, value) in [
        ("value_0xe4", row.value_0xe4),
        ("value_0xf0", row.value_0xf0),
        ("value_0xf4", row.value_0xf4),
    ] {
        if value > MAX_AGENCY_CAPACITY_CLUSTER_VALUE {
            summary.parse_errors.push(format!(
                "offset {offset}: agency capacity cluster row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if row.value_0xe4 != row.value_0xf4 {
        let value_0xe4 = row.value_0xe4;
        let value_0xf4 = row.value_0xf4;
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has value_0xe4 {value_0xe4} != value_0xf4 {value_0xf4}"
        ));
    }
    if row.proof_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: agency capacity cluster row {row_index} has zero proof_hash"
        ));
    }
}

fn summarize_operative_row(
    offset: usize,
    row_index: usize,
    row: &OperativeRow,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:537..): DETECTED/CAPTURED from bools;
    // SKILL/EXPERIENCE/LIFECYCLE presence from Option::is_some; NAME_HASH iff
    // name_hash!=0. Encoded scalar values are the Option's unwrap_or(0), matching
    // decode's flag-gated read; validation reads the encoded scalar.
    let deleted = row_id_deleted(row.op_id);
    let mut flags = 0u16;
    if !deleted {
        if row.detected {
            flags |= OPERATIVE_FLAG_DETECTED;
        }
        if row.captured {
            flags |= OPERATIVE_FLAG_CAPTURED;
        }
        if row.skill.is_some() {
            flags |= OPERATIVE_FLAG_SKILL_PRESENT;
        }
        if row.experience_raw.is_some() {
            flags |= OPERATIVE_FLAG_EXPERIENCE_PRESENT;
        }
        if row.name_hash != 0 {
            flags |= OPERATIVE_FLAG_NAME_HASH;
        }
        if row.lifecycle_flag.is_some() {
            flags |= OPERATIVE_FLAG_LIFECYCLE_PRESENT;
        }
    }
    let owner = row.owner_tag_db_id;
    let op_id = row_id_clear_deleted(row.op_id);
    let skill = row.skill.unwrap_or(0);
    let experience_raw = row.experience_raw.unwrap_or(0);
    let name_hash = row.name_hash;
    let state_hash = row.state_hash;
    let lifecycle_flag = row.lifecycle_flag.unwrap_or(0);
    if deleted {
        if flags != 0
            || owner != 0
            || skill != 0
            || experience_raw != 0
            || name_hash != 0
            || state_hash != 0
            || lifecycle_flag != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: operative tombstone row {row_index} has live fields"
            ));
        }
        if op_id == 0 || op_id > MAX_OPERATIVE_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: operative tombstone row {row_index} has invalid op_id {op_id}"
            ));
        } else if !seen.insert(op_id) {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} duplicates op_id {op_id}"
            ));
        }
        return;
    }
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid owner {owner}"
        ));
    }
    if op_id == 0 || op_id > MAX_OPERATIVE_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid op_id {op_id}"
        ));
    } else if !seen.insert(op_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} duplicates op_id {op_id}"
        ));
    }
    if flags
        & !(OPERATIVE_FLAG_DETECTED
            | OPERATIVE_FLAG_SKILL_PRESENT
            | OPERATIVE_FLAG_EXPERIENCE_PRESENT
            | OPERATIVE_FLAG_NAME_HASH
            | OPERATIVE_FLAG_CAPTURED
            | OPERATIVE_FLAG_LIFECYCLE_PRESENT)
        != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & OPERATIVE_FLAG_SKILL_PRESENT == 0 && skill != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has skill without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_SKILL_PRESENT != 0 && skill > MAX_OPERATIVE_SKILL {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid skill {skill}"
        ));
    }
    if flags & OPERATIVE_FLAG_EXPERIENCE_PRESENT == 0 && experience_raw != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has experience without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_EXPERIENCE_PRESENT != 0 && !(0..MAX_XP_RAW).contains(&experience_raw)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has invalid experience_raw {experience_raw}"
        ));
    }
    if flags & OPERATIVE_FLAG_NAME_HASH == 0 && name_hash != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has name_hash without flag"
        ));
    }
    if flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT == 0 && lifecycle_flag != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} has lifecycle_flag without present flag"
        ));
    }
    if flags & OPERATIVE_FLAG_LIFECYCLE_PRESENT != 0 {
        if !(1..=MAX_OPERATIVE_LIFECYCLE_FLAG).contains(&lifecycle_flag) {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} has invalid lifecycle_flag {lifecycle_flag}"
            ));
        }
        if state_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: operative row {row_index} has lifecycle flag without state_hash"
            ));
        }
    }
}

fn summarize_crypto_row(
    offset: usize,
    row_index: usize,
    row: &CryptologyRow,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:567): DECRYPTING iff row.decrypting; reserved 0.
    let deleted = row_id_deleted(row.owner_tag_db_id);
    let flags = if !deleted && row.decrypting {
        CRYPTO_FLAG_DECRYPTING
    } else {
        0
    };
    let owner = row_id_clear_deleted(row.owner_tag_db_id);
    let target = row.target_tag_db_id;
    validate_pair(offset, "cryptology", row_index, owner, target, summary);
    if flags & !CRYPTO_FLAG_DECRYPTING != 0 || (deleted && flags != 0) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} has unknown flags {flags}"
        ));
    }
    if valid_tag(owner) && valid_tag(target) && !seen.insert((owner, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} duplicates owner/target"
        ));
    }
    if deleted {
        if row.amount_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: cryptology tombstone row {row_index} has live fields"
            ));
        }
        return;
    }
    if !(1..MAX_CRYPTO_AMOUNT_RAW).contains(&row.amount_raw) {
        let amount_raw = row.amount_raw;
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} has invalid amount_raw {amount_raw}"
        ));
    }
    // reserved fields are always 0 from the encoder → the nonzero check never fires.
}

fn summarize_operation_row(
    offset: usize,
    row_index: usize,
    row: &OperationAnchorRow,
    seen: &mut BTreeSet<u32>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:581..): RUNNING always; DETAIL_RESISTED iff
    // detail_resisted; TARGET_PRESENT iff target is Some. Encoded target is
    // unwrap_or(0).
    let mut flags = OPERATION_FLAG_RUNNING;
    if row.detail_resisted {
        flags |= OPERATION_FLAG_DETAIL_RESISTED;
    }
    if row.target_tag_db_id.is_some() {
        flags |= OPERATION_FLAG_TARGET_PRESENT;
    }
    let owner = row.owner_tag_db_id;
    let operation_id = row.operation_id;
    let target = row.target_tag_db_id.unwrap_or(0);
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid owner {owner}"
        ));
    }
    if operation_id == 0 || operation_id > MAX_OPERATION_ID {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid operation_id {operation_id}"
        ));
    } else if !seen.insert(operation_id) {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} duplicates operation_id {operation_id}"
        ));
    }
    if flags
        & !(OPERATION_FLAG_RUNNING | OPERATION_FLAG_DETAIL_RESISTED | OPERATION_FLAG_TARGET_PRESENT)
        != 0
    {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & OPERATION_FLAG_DETAIL_RESISTED == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} lacks detail-resisted/source proof flag"
        ));
    }
    if flags & OPERATION_FLAG_TARGET_PRESENT != 0 {
        if !valid_tag(target) || target == owner {
            summary.parse_errors.push(format!(
                "offset {offset}: operation anchor row {row_index} has invalid target {target}"
            ));
        }
    } else if target != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has target without flag"
        ));
    }
    if row.slot_count > MAX_OPERATION_SLOTS || row.phase_count > MAX_OPERATION_PHASES {
        summary.parse_errors.push(format!(
            "offset {offset}: operation anchor row {row_index} has invalid slot/phase counts"
        ));
    }
    // reserved is always 0 from the encoder → the nonzero check never fires.
}

fn summarize_intel_map_row(
    offset: usize,
    row_index: usize,
    row: &IntelMapRow,
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:598): RESISTED iff row.resisted; reserved 0.
    let flags = if row.resisted {
        INTEL_MAP_FLAG_RESISTED
    } else {
        0
    };
    let owner = row.owner_tag_db_id;
    let target = row.target_tag_db_id;
    validate_pair(offset, "intel map", row_index, owner, target, summary);
    if flags & !INTEL_MAP_FLAG_RESISTED != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} has unknown flags {flags}"
        ));
    }
    if valid_tag(owner) && valid_tag(target) && !seen.insert((owner, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} duplicates owner/target"
        ));
    }
    for (label, value) in [
        ("civilian_raw", row.civilian_raw),
        ("army_raw", row.army_raw),
        ("navy_raw", row.navy_raw),
        ("air_raw", row.air_raw),
    ] {
        if !(0..=MAX_INTEL_PERCENT_RAW).contains(&value) {
            summary.parse_errors.push(format!(
                "offset {offset}: intel map row {row_index} has invalid {label} {value}"
            ));
        }
    }
    if flags & INTEL_MAP_FLAG_RESISTED != 0
        && (row.civilian_raw != 0 || row.army_raw != 0 || row.navy_raw != 0 || row.air_raw != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: intel map row {row_index} has resisted flag with emitted values"
        ));
    }
    // reserved is always 0 from the encoder → the nonzero check never fires.
}

fn summarize_state_event_row(
    offset: usize,
    row_index: usize,
    row: &StateEventRow,
    seen: &mut StateEventSeenSet,
    summary: &mut Domain53GateSummary,
) {
    // Encoder flags (encode_payload:614): TARGET_PRESENT iff target is Some;
    // encoded target is unwrap_or(0).
    let deleted = row_id_deleted(row.owner_tag_db_id);
    let flags = if row.target_tag_db_id.is_some() {
        STATE_EVENT_FLAG_TARGET_PRESENT
    } else {
        0
    };
    let owner = row_id_clear_deleted(row.owner_tag_db_id);
    let target = row.target_tag_db_id.unwrap_or(0);
    let event_kind = row.event_kind;
    let source_kind = row.source_kind;
    let object_hash = row.object_hash;
    let value_hash = row.value_hash;
    let value_raw = row.value_raw;
    if !valid_tag(owner) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has invalid owner {owner}"
        ));
    }
    if flags & !STATE_EVENT_FLAG_TARGET_PRESENT != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown flags {flags}"
        ));
    }
    if flags & STATE_EVENT_FLAG_TARGET_PRESENT != 0 {
        if !valid_tag(target) || target == owner {
            summary.parse_errors.push(format!(
                "offset {offset}: compact state event row {row_index} has invalid target {target}"
            ));
        }
    } else if target != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has target without flag"
        ));
    }
    if !known_state_event_kind(event_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown event_kind {event_kind}"
        ));
    }
    if !known_state_event_source(source_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has unknown source_kind {source_kind}"
        ));
    }
    if object_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has zero object_hash"
        ));
    }
    if deleted {
        if value_hash != 0 || value_raw != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: compact state event tombstone row {row_index} has live fields"
            ));
        }
    } else if value_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has zero value_hash"
        ));
    }
    if !deleted && !(-MAX_STATE_EVENT_VALUE_RAW..=MAX_STATE_EVENT_VALUE_RAW).contains(&value_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has invalid value_raw {value_raw}"
        ));
    }
    if object_hash != 0 && !seen.insert(event_kind, source_kind, owner, target, object_hash) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} duplicates state event key"
        ));
    }
}

#[derive(Debug, Default)]
struct PairSeenSet {
    words: Option<Vec<u64>>,
}

impl PairSeenSet {
    fn insert(&mut self, src: u32, target: u32) -> bool {
        debug_assert!(valid_tag(src));
        debug_assert!(valid_tag(target));
        let stride = MAX_TAG_DB_ID as usize + 1;
        let bit = src as usize * stride + target as usize;
        let word_index = bit / 64;
        let mask = 1u64 << (bit % 64);
        let words = self.words.get_or_insert_with(|| {
            let bit_count = stride * stride;
            vec![0u64; (bit_count + 63) / 64]
        });
        let word = &mut words[word_index];
        let fresh = *word & mask == 0;
        *word |= mask;
        fresh
    }
}

#[derive(Debug, Default)]
struct StateEventSeenSet {
    relation_opinion_pairs: PairSeenSet,
    other: HashSet<u128>,
}

impl StateEventSeenSet {
    fn insert(
        &mut self,
        event_kind: u16,
        source_kind: u16,
        owner: u32,
        target: u32,
        object_hash: u32,
    ) -> bool {
        if event_kind == STATE_EVENT_KIND_OBSERVED
            && source_kind == STATE_EVENT_SOURCE_RELATION_OPINION
            && object_hash == STATE_EVENT_OBJECT_RELATION_OPINION_HASH
            && valid_tag(owner)
            && valid_tag(target)
            && owner != target
        {
            return self.relation_opinion_pairs.insert(owner, target);
        }
        let key = (u128::from(event_kind) << 112)
            | (u128::from(source_kind) << 96)
            | (u128::from(owner) << 64)
            | (u128::from(target) << 32)
            | u128::from(object_hash);
        self.other.insert(key)
    }
}

fn validate_pair(
    offset: usize,
    label: &str,
    row_index: usize,
    first: u32,
    second: u32,
    summary: &mut Domain53GateSummary,
) {
    if !valid_tag(first) || !valid_tag(second) || first == second {
        summary.parse_errors.push(format!(
            "offset {offset}: {label} row {row_index} has invalid tag pair {first}/{second}"
        ));
    }
}

fn reject_snapshot_tombstone_outside_delta(
    offset: usize,
    row_index: usize,
    status_flags: u32,
    deleted: bool,
    summary: &mut Domain53GateSummary,
) {
    if deleted && status_flags & STATUS_DELTA_DATA == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel row {row_index} tombstone appears outside delta frame"
        ));
    }
}

fn domain53_base_row_deleted(kind: u16, payload: &[u8], cursor: usize) -> bool {
    match kind {
        KIND_COUNTRY_DIPLOMACY
        | KIND_WAR_RELATION
        | KIND_OPINION
        | KIND_SUBJECT
        | KIND_AGENCY
        | KIND_AGENCY_CAPACITY_CLUSTER
        | KIND_CRYPTOLOGY
        | KIND_COMPACT_STATE_EVENT => row_id_deleted(read_u32_le(payload, cursor + 4)),
        KIND_RELATION_FLAG => row_id_deleted(read_u32_le(payload, cursor + 8)),
        KIND_OPERATIVE => row_id_deleted(read_u32_le(payload, cursor + 8)),
        _ => false,
    }
}

fn row_id_deleted(value: u32) -> bool {
    value & ROW_ID_DELETED != 0
}

fn row_id_clear_deleted(value: u32) -> u32 {
    value & !ROW_ID_DELETED
}

fn valid_tag(value: u32) -> bool {
    value > 0 && value <= MAX_TAG_DB_ID
}

fn known_state_event_kind(event_kind: u16) -> bool {
    matches!(event_kind, STATE_EVENT_KIND_OBSERVED)
}

fn known_state_event_source(source_kind: u16) -> bool {
    matches!(
        source_kind,
        STATE_EVENT_SOURCE_RELATION_FLAG
            | STATE_EVENT_SOURCE_SUBJECT_AUTONOMY
            | STATE_EVENT_SOURCE_AGENCY_SCALAR
            | STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE
            | STATE_EVENT_SOURCE_AGENCY_CAPACITY
            | STATE_EVENT_SOURCE_CRYPTOLOGY
            | STATE_EVENT_SOURCE_RELATION_OPINION
            | STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP
    )
}

fn known_relation_kind(flag_kind: u16) -> bool {
    matches!(
        flag_kind,
        REL_KIND_NON_AGGRESSION_PACT
            | REL_KIND_PUPPET
            | REL_KIND_EMBARGO
            | REL_KIND_MILITARY_ACCESS
            | REL_KIND_AIR_BASE_ACCESS
            | REL_KIND_DOCKING_RIGHTS
            | REL_KIND_NAVAL_BLOCKADE
            | REL_KIND_MARKET_ACCESS_RIGHTS
            | REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION
            | REL_KIND_GUARANTEE
            | REL_KIND_IMPROVE_RELATIONS
            | REL_KIND_ATTACHE
            | REL_KIND_LICENCE_PRODUCTION
    )
}

fn add_unit(summary: &mut Domain53GateSummary, key: &'static str, value: usize) {
    let entry = summary.data_units_total.entry(key).or_default();
    *entry = entry.saturating_add(value as u64);
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

fn read_i64_le(payload: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(payload[offset..offset + 8].try_into().expect("i64 range"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_domain_id_is_53() {
        assert_eq!(DOMAIN_ID_PROPOSED, 53);
        assert_eq!(domain_id_rejection_reason(DOMAIN_ID_PROPOSED), None);
    }

    #[test]
    fn relation_opinion_hot_hash_constant_matches_stable_hash() {
        assert_eq!(
            STATE_EVENT_OBJECT_RELATION_OPINION_HASH,
            stable_hash32("relation_opinion")
        );
        let mut pairs = PairSeenSet::default();
        assert!(pairs.insert(1, 3));
        assert!(!pairs.insert(1, 3));
        assert!(pairs.insert(3, 1));

        let mut events = StateEventSeenSet::default();
        assert!(events.insert(
            STATE_EVENT_KIND_OBSERVED,
            STATE_EVENT_SOURCE_RELATION_OPINION,
            1,
            3,
            STATE_EVENT_OBJECT_RELATION_OPINION_HASH
        ));
        assert!(!events.insert(
            STATE_EVENT_KIND_OBSERVED,
            STATE_EVENT_SOURCE_RELATION_OPINION,
            1,
            3,
            STATE_EVENT_OBJECT_RELATION_OPINION_HASH
        ));
        assert!(events.insert(
            STATE_EVENT_KIND_OBSERVED,
            STATE_EVENT_SOURCE_RELATION_OPINION,
            3,
            1,
            STATE_EVENT_OBJECT_RELATION_OPINION_HASH
        ));
    }

    #[test]
    fn rejects_occupied_lower_production_domain_ids() {
        for (domain_id, owner) in RESERVED_OCCUPIED_DOMAIN_IDS {
            let error = domain_id_rejection_reason(*domain_id)
                .expect("occupied lower domain id should be rejected");
            assert!(
                error.contains(&domain_id.to_string()) && error.contains(owner),
                "{error}"
            );
        }
    }

    #[test]
    fn encodes_and_decodes_full_lane_payload() {
        let snapshot = fixture_snapshot();
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(32, &payload);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary.status_flags_or,
            STATUS_PARTIAL_DATA
                | STATUS_OPERATIONS_DETAIL_RESISTED
                | STATUS_INTEL_MAP_RESISTED
                | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED
        );
        assert_eq!(summary.data_rows_total, 19);
        assert_eq!(summary.data_changed_rows_total, 19);
        assert_eq!(
            summary.data_units_total.get("relation_flag_rows").copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operation_anchor_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("agency_capacity_cluster_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operative_lifecycle_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(8)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_relation_opinion_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("compact_state_event_country_faction_group_rows")
                .copied(),
            Some(1)
        );
    }

    #[test]
    fn rejects_unknown_relation_flag_kind() {
        let mut snapshot = fixture_snapshot();
        snapshot.relation_flags[0].flag_kind = 999;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown flag_kind 999")));
    }

    #[test]
    fn rejects_fabricated_operative_skill_without_presence_flag() {
        let payload = encode_payload(&fixture_snapshot());
        let operative_offset = HEADER_LEN
            + ROW_COUNTRY_LEN
            + ROW_WAR_LEN
            + ROW_OPINION_LEN
            + ROW_RELATION_FLAG_LEN
            + ROW_SUBJECT_LEN
            + ROW_AGENCY_LEN
            + ROW_AGENCY_CAPACITY_CLUSTER_LEN;
        let mut malformed = payload;
        malformed[operative_offset + 2..operative_offset + 4].copy_from_slice(&0u16.to_le_bytes());

        let summary = decode_payload(0, &malformed);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("skill without present flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("experience without present flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("name_hash without flag")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("lifecycle_flag without present flag")));
    }

    #[test]
    fn rejects_truncated_payload() {
        let payload = encode_payload(&fixture_snapshot());
        let summary = decode_payload(11, &payload[..payload.len() - 3]);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("truncated") || error.contains("trailing")));
    }

    #[test]
    fn accepts_current_gap_badges_when_no_unvalidated_rows_emit() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |=
            STATUS_WAR_ROOT_UNVALIDATED | STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
        snapshot.countries[0].at_war_count = 0;
        snapshot.countries[0].casualties = 0;
        snapshot.wars.clear();
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert!(
            summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED != 0,
            "war root badge should survive decode"
        );
        assert!(
            summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED != 0,
            "incoming action queue badge should survive decode"
        );
        assert!(
            summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED != 0,
            "war lifecycle detail badge should survive decode"
        );
    }

    #[test]
    fn rejects_war_rows_while_war_root_is_badged_unvalidated() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("badges war root unvalidated") && error.contains("war relation")
        }));
    }

    #[test]
    fn rejects_country_war_projection_while_war_root_is_badged_unvalidated() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        snapshot.wars.clear();
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("badges war root unvalidated")
                && error.contains("country war projection")
        }));
    }

    #[test]
    fn rejects_war_rows_without_lifecycle_detail_badge() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags &= !STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED")
                && error.contains("war name/grouping/start-date")
        }));
    }

    #[test]
    fn rejects_faction_rows_while_faction_group_is_badged_unvalidated() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |= STATUS_FACTION_GROUP_UNVALIDATED;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("badges faction group unvalidated")
                && error.contains("country diplomacy")
        }));
    }

    #[test]
    fn rejects_resisted_operation_rows_without_top_level_badge() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags &= !STATUS_OPERATIONS_DETAIL_RESISTED;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("resisted operation row")
                && error.contains("STATUS_OPERATIONS_DETAIL_RESISTED")
        }));
    }

    #[test]
    fn rejects_agency_capacity_cluster_without_e4_f4_mirror() {
        let payload = encode_payload(&fixture_snapshot());
        let capacity_offset = HEADER_LEN
            + ROW_COUNTRY_LEN
            + ROW_WAR_LEN
            + ROW_OPINION_LEN
            + ROW_RELATION_FLAG_LEN
            + ROW_SUBJECT_LEN
            + ROW_AGENCY_LEN;
        let mut malformed = payload;
        malformed[capacity_offset + 16..capacity_offset + 20].copy_from_slice(&8u32.to_le_bytes());

        let summary = decode_payload(0, &malformed);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("value_0xe4") && error.contains("value_0xf4")));
    }

    #[test]
    fn rejects_resisted_intel_map_rows_without_top_level_badge() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags &= !STATUS_INTEL_MAP_RESISTED;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary.parse_errors.iter().any(|error| {
            error.contains("resisted intel-map row") && error.contains("STATUS_INTEL_MAP_RESISTED")
        }));
    }

    #[test]
    fn rejects_compact_state_event_with_unknown_source() {
        let payload = encode_payload(&fixture_snapshot());
        let state_event_offset = HEADER_LEN
            + ROW_COUNTRY_LEN
            + ROW_WAR_LEN
            + ROW_OPINION_LEN
            + ROW_RELATION_FLAG_LEN
            + ROW_SUBJECT_LEN
            + ROW_AGENCY_LEN
            + ROW_AGENCY_CAPACITY_CLUSTER_LEN
            + ROW_OPERATIVE_LEN
            + ROW_CRYPTOLOGY_LEN
            + ROW_OPERATION_ANCHOR_LEN
            + ROW_INTEL_MAP_LEN;
        let mut malformed = payload;
        malformed[state_event_offset + 14..state_event_offset + 16]
            .copy_from_slice(&999u16.to_le_bytes());

        let summary = decode_payload(0, &malformed);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown source_kind 999")));
    }

    #[test]
    fn rejects_unknown_status_bits() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |= 1 << 31;
        let payload = encode_payload(&snapshot);

        let summary = decode_payload(0, &payload);

        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unknown bits")));
    }

    #[test]
    fn operation_instance_trailer_round_trips() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![
            OperationInstanceRow::legacy(129_014, true, Some(3)),
            OperationInstanceRow::legacy(128_904, false, None),
        ];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operation_instance_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operation_instance_country_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operation_instance_def_ref_ok_rows")
                .copied(),
            Some(1)
        );
        // Snapshot decode restores the rows.
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert_eq!(decoded.operation_instances, snapshot.operation_instances);
    }

    #[test]
    fn operation_runtime_v3_round_trips_exact_contract() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![OperationInstanceRow {
            instance_id: 18_568,
            def_ref_ok: true,
            associated_country_tag_db_id: Some(1),
            definition_id: Some("operation_infiltrate_civilian_government".into()),
            owner_country_tag_db_id: Some(2),
            target_country_tag_db_id: Some(1),
            target_state_id: Some(64),
            status: Some(OperationInstanceStatus::Running),
            phase_index: 1,
            phase_count: 3,
            progress_fixed: Some(42_735),
            start_hour: Some(60_762_441),
            duration_hours: Some(2_808),
            assigned_operative_ids: vec![9_751, 17_071],
        }];
        let payload = encode_payload(&snapshot);
        let trailer = payload
            .windows(4)
            .position(|window| window == OPERATION_INSTANCE_TRAILER_MAGIC)
            .expect("OPIN trailer");
        assert_eq!(read_u16_le(&payload, trailer + 4), 3);
        let decoded = decode_snapshot_payload(0, &payload).expect("decode OPIN v3");
        assert_eq!(decoded.operation_instances, snapshot.operation_instances);
    }

    #[test]
    fn incoming_action_queue_trailer_round_trips_with_operation_instances() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![OperationInstanceRow::legacy(129_014, true, Some(3))];
        snapshot.incoming_actions = vec![
            IncomingActionQueueRow {
                recipient_tag_db_id: 9,
                actor_tag_db_id: 1,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_000,
                last_command_date_hours: 60_830_000,
            },
            IncomingActionQueueRow {
                recipient_tag_db_id: 9,
                actor_tag_db_id: 16,
                action_id: 150_608,
                action_type: 0,
                date_hours: 60_830_024,
                last_command_date_hours: 60_830_024,
            },
        ];
        let payload = encode_payload(&snapshot);
        assert!(payload
            .windows(4)
            .any(|window| window == INCOMING_ACTION_QUEUE_TRAILER_MAGIC));
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("incoming_action_queue_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("incoming_action_queue_recipient_rows")
                .copied(),
            Some(1)
        );
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert_eq!(decoded.operation_instances, snapshot.operation_instances);
        assert_eq!(decoded.incoming_actions, snapshot.incoming_actions);
    }

    #[test]
    fn incoming_action_queue_trailer_rejects_duplicate_action_id() {
        let mut snapshot = fixture_snapshot();
        snapshot.incoming_actions = vec![
            IncomingActionQueueRow {
                recipient_tag_db_id: 9,
                actor_tag_db_id: 1,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_000,
                last_command_date_hours: 60_830_000,
            },
            IncomingActionQueueRow {
                recipient_tag_db_id: 10,
                actor_tag_db_id: 16,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_024,
                last_command_date_hours: 60_830_024,
            },
        ];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates action_id 150442")));
    }

    #[test]
    fn intel_network_graph_trailer_round_trips() {
        let mut snapshot = fixture_snapshot();
        snapshot.intel_network_graphs = vec![
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 2,
                state_ids: vec![121, 122, 123],
                state_strengths: None,
                coverage: None,
            },
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 1,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
                state_strengths: None,
                coverage: None,
            },
        ];
        let payload = encode_payload(&snapshot);
        assert!(payload
            .windows(4)
            .any(|window| window == INTEL_NETWORK_GRAPH_TRAILER_MAGIC));
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("intel_network_graph_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            summary
                .data_units_total
                .get("intel_network_graph_state_rows")
                .copied(),
            Some(6)
        );
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert_eq!(decoded.intel_network_graphs, snapshot.intel_network_graphs);
    }

    // === FACS faction trailer (melt-anchored: D04 1944.2.11 had 6 factions;
    // ids {1,2,3,5,6,7}, leaders GER/ENG/SOV/JAP/CHI/ETH = members[0], member
    // counts {18,31,3,8,10,1}). Synthetic tag_db_ids stand in for the real tags
    // but the faction id/count/leader shape mirrors the melt exactly. ===
    fn melt_faction_rows() -> Vec<FactionRow> {
        // (faction_id, leader_tag_db_id, member_count) taken from the melt.
        // Member tag_db_ids are disjoint blocks of 100 within the valid tag
        // range [1, MAX_TAG_DB_ID(=2048)]; the leader is the block's first tag.
        let shape = [
            (1u32, 100u32, 18usize),
            (2, 200, 31),
            (3, 400, 3),
            (5, 600, 10),
            (6, 800, 8),
            (7, 1000, 1),
        ];
        shape
            .iter()
            .map(|(fid, leader, count)| {
                // Members are the leader plus (count-1) other distinct in-range
                // tags, sorted ascending (decoder requires strictly-increasing).
                let mut members: Vec<u32> = (0..*count as u32).map(|k| *leader + k).collect();
                members.sort_unstable();
                members.dedup();
                FactionRow {
                    faction_id: *fid,
                    faction_type: 88,
                    name_hash: stable_hash32(&format!("faction_{fid}")),
                    leader_tag_db_id: *leader,
                    member_tag_db_ids: members,
                }
            })
            .collect()
    }

    #[test]
    fn faction_trailer_round_trips_six_melt_factions() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions = melt_faction_rows();
        let payload = encode_payload(&snapshot);
        let facs_offset = payload
            .windows(4)
            .position(|window| window == FACTION_TRAILER_MAGIC)
            .expect("legacy FACS trailer");
        assert_eq!(
            read_u16_le(&payload, facs_offset + 4),
            FACTION_TRAILER_VERSION_V1_NONEMPTY
        );
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        // 6 factions, 18+31+3+10+8+1 = 71 member rows total.
        assert_eq!(
            summary.data_units_total.get("faction_rows").copied(),
            Some(6)
        );
        assert_eq!(
            summary.data_units_total.get("faction_member_rows").copied(),
            Some(71)
        );
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert_eq!(decoded.factions, snapshot.factions);
        assert!(decoded.faction_roster_complete);
        // Leader is members[0] in the melt; verify each decoded leader is a
        // member and the member counts match the melt shape.
        let counts: Vec<usize> = decoded
            .factions
            .iter()
            .map(|f| f.member_tag_db_ids.len())
            .collect();
        assert_eq!(counts, vec![18, 31, 3, 10, 8, 1]);
        for f in &decoded.factions {
            assert!(f.member_tag_db_ids.contains(&f.leader_tag_db_id));
            assert_eq!(f.faction_type, 88);
        }
    }

    #[test]
    fn faction_trailer_rejects_unsorted_members() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions = vec![FactionRow {
            faction_id: 1,
            faction_type: 88,
            name_hash: stable_hash32("Axis"),
            leader_tag_db_id: 10,
            // Not strictly increasing.
            member_tag_db_ids: vec![10, 30, 20],
        }];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("members are not strictly increasing")));
    }

    #[test]
    fn faction_trailer_rejects_leader_not_in_members() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions = vec![FactionRow {
            faction_id: 1,
            faction_type: 88,
            name_hash: stable_hash32("Axis"),
            leader_tag_db_id: 99,
            member_tag_db_ids: vec![10, 20, 30],
        }];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("leader 99 not present in members")));
    }

    #[test]
    fn faction_trailer_rejects_duplicate_faction_id() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions = vec![
            FactionRow {
                faction_id: 1,
                faction_type: 88,
                name_hash: stable_hash32("Axis"),
                leader_tag_db_id: 10,
                member_tag_db_ids: vec![10, 20],
            },
            FactionRow {
                faction_id: 1,
                faction_type: 88,
                name_hash: stable_hash32("Allies"),
                leader_tag_db_id: 30,
                member_tag_db_ids: vec![30, 40],
            },
        ];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates faction_id 1")));
    }

    #[test]
    fn no_factions_emits_no_facs_trailer() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions = Vec::new();
        let payload = encode_payload(&snapshot);
        assert!(!payload
            .windows(4)
            .any(|window| window == FACTION_TRAILER_MAGIC));
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert!(decoded.factions.is_empty());
        assert!(!decoded.faction_roster_complete);
    }

    #[test]
    fn faction_trailer_v2_witnesses_measured_empty_roster() {
        let mut snapshot = fixture_snapshot();
        snapshot.factions.clear();
        snapshot.faction_roster_complete = true;
        let payload = encode_payload(&snapshot);
        let facs_offset = payload
            .windows(4)
            .position(|window| window == FACTION_TRAILER_MAGIC)
            .expect("FACS v2 completeness witness");
        assert_eq!(
            read_u16_le(&payload, facs_offset + 4),
            FACTION_TRAILER_VERSION
        );
        assert_eq!(read_u32_le(&payload, facs_offset + 8), 0);

        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert!(decoded.factions.is_empty());
        assert!(decoded.faction_roster_complete);
    }

    #[test]
    fn subject_start_date_round_trips() {
        // Melt: GER->DEN autonomy_reichsprotectorate started 1940.2.14.20, which
        // is engine-hour raw 60_795_475 (DATE_EPOCH_CONST + calendar hours).
        let mut snapshot = fixture_snapshot();
        snapshot.subjects = vec![SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: stable_hash32("autonomy_reichsprotectorate"),
            autonomy_ratio_raw: 40_000,
            start_date_raw: 60_795_475,
        }];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        let decoded = decode_snapshot_payload(0, &payload).unwrap();
        assert_eq!(decoded.subjects.len(), 1);
        assert_eq!(decoded.subjects[0].start_date_raw, 60_795_475);
        // The shared calendar decodes the raw to the exact melt date.
        assert_eq!(
            compact_contract::calendar::date_from_raw(decoded.subjects[0].start_date_raw)
                .as_deref(),
            Some("1940.2.14.20")
        );
    }

    fn fixture_subject_autonomy_status() -> SubjectAutonomyStatusRow {
        SubjectAutonomyStatusRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            current_state_hash: stable_hash32("autonomy_puppet"),
            previous_state_hash: Some(stable_hash32("autonomy_colony")),
            next_state_hash: Some(stable_hash32("autonomy_integrated_puppet")),
            ratio_raw: 50_000,
            current_score_raw: 150,
            lower_score_raw: 100,
            upper_score_raw: 200,
            last_modified_date_raw: 60_795_475,
        }
    }

    #[test]
    fn subject_autonomy_status_trailer_round_trips_complete_set() {
        let mut snapshot = fixture_snapshot();
        snapshot.subject_autonomy_statuses = vec![fixture_subject_autonomy_status()];
        snapshot.subject_autonomy_status_complete = true;

        let decoded = decode_snapshot_payload(0, &encode_payload(&snapshot)).unwrap();
        assert!(decoded.subject_autonomy_status_complete);
        assert_eq!(
            decoded.subject_autonomy_statuses,
            snapshot.subject_autonomy_statuses
        );
    }

    #[test]
    fn subject_autonomy_status_rejects_invalid_score_bounds() {
        let mut snapshot = fixture_snapshot();
        let mut status = fixture_subject_autonomy_status();
        status.upper_score_raw = status.lower_score_raw;
        status.ratio_raw = 0;
        snapshot.subject_autonomy_statuses = vec![status];
        snapshot.subject_autonomy_status_complete = true;

        let errors = decode_snapshot_payload(0, &encode_payload(&snapshot)).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.contains("invalid subject-autonomy status trailer row")));
    }

    #[test]
    fn subject_autonomy_status_delta_defers_key_join_to_fold() {
        let mut snapshot = fixture_snapshot();
        snapshot.status_flags |= STATUS_DELTA_DATA;
        snapshot.subjects.clear();
        snapshot.subject_autonomy_statuses = vec![fixture_subject_autonomy_status()];
        snapshot.subject_autonomy_status_complete = true;

        let decoded = decode_snapshot_payload(0, &encode_payload(&snapshot)).unwrap();
        assert!(decoded.subject_autonomy_status_complete);
        assert!(decoded.subjects.is_empty());
        assert_eq!(decoded.subject_autonomy_statuses.len(), 1);
    }

    #[test]
    fn subject_rejects_non_positive_start_date() {
        let mut snapshot = fixture_snapshot();
        snapshot.subjects = vec![SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: stable_hash32("autonomy_puppet"),
            autonomy_ratio_raw: 40_000,
            start_date_raw: 0,
        }];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("non-positive start_date_raw")));
    }

    #[test]
    fn subject_rejects_ratio_above_one() {
        let mut snapshot = fixture_snapshot();
        snapshot.subjects[0].autonomy_ratio_raw = FIXED_SCALE_RAW as i32 + 1;
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("invalid autonomy_ratio_raw 100001")));
    }

    #[test]
    fn intel_network_graph_trailer_rejects_duplicate_slot() {
        let mut snapshot = fixture_snapshot();
        snapshot.intel_network_graphs = vec![
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 2,
                state_ids: vec![121, 122, 123],
                state_strengths: None,
                coverage: None,
            },
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
                state_strengths: None,
                coverage: None,
            },
        ];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates operative 1 slot 0")));
    }

    #[test]
    fn operation_instance_trailer_rejects_duplicate_instance_id() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![
            OperationInstanceRow::legacy(42, true, None),
            OperationInstanceRow::legacy(42, true, None),
        ];
        let payload = encode_payload(&snapshot);
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("duplicates instance_id 42")));
    }

    #[test]
    fn operation_instance_trailer_rejects_country_without_flag() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![OperationInstanceRow::legacy(7, true, None)];
        let mut payload = encode_payload(&snapshot);
        // Force a nonzero country field on a row whose flag says absent.
        let len = payload.len();
        payload[len - 4..len].copy_from_slice(&5u32.to_le_bytes());
        let summary = decode_payload(0, &payload);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("country without present flag")));
    }

    #[test]
    fn no_operation_instances_emits_no_trailer() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances.clear();
        let payload = encode_payload(&snapshot);
        assert!(!payload
            .windows(4)
            .any(|window| window == OPERATION_INSTANCE_TRAILER_MAGIC));
        let summary = decode_payload(0, &payload);
        assert!(
            summary.parse_errors.is_empty(),
            "{:?}",
            summary.parse_errors
        );
        assert_eq!(
            summary
                .data_units_total
                .get("operation_instance_rows")
                .copied(),
            Some(0)
        );
    }

    fn fixture_snapshot() -> DiplomacyIntelSnapshot {
        DiplomacyIntelSnapshot {
            elapsed_wall_ms: 123,
            status_flags: STATUS_PARTIAL_DATA
                | STATUS_OPERATIONS_DETAIL_RESISTED
                | STATUS_INTEL_MAP_RESISTED
                | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            changed_rows: None,
            countries: vec![CountryDiplomacyRow {
                tag_db_id: 1,
                faction_group: 42,
                at_war_count: 1,
                casualties: 100,
            }],
            wars: vec![WarRelationRow {
                first_tag_db_id: 1,
                second_tag_db_id: 3,
                first_casualties: 100,
                second_casualties: 250,
            }],
            opinions: vec![OpinionRow {
                src_tag_db_id: 1,
                target_tag_db_id: 3,
                opinion: -120,
            }],
            relation_flags: vec![RelationFlagRow {
                src_tag_db_id: 1,
                target_tag_db_id: 3,
                flag_kind: REL_KIND_MARKET_ACCESS_RIGHTS,
                directed: false,
            }],
            subjects: vec![SubjectRow {
                overlord_tag_db_id: 1,
                subject_tag_db_id: 22,
                autonomy_state_hash: stable_hash32("autonomy_puppet"),
                autonomy_ratio_raw: 40_000,
                start_date_raw: 60_795_475,
            }],
            subject_autonomy_statuses: Vec::new(),
            subject_autonomy_status_complete: false,
            agencies: vec![AgencyRow {
                owner_tag_db_id: 1,
                max_operatives: 3,
                usable_slots: 3,
                operative_count: 1,
                cryptology_target_count: 1,
                defense_raw: 600_000,
                upgrade_progress_raw: 2_400_000,
                name_hash: stable_hash64("Abwehr"),
                icon_hash: stable_hash64("GFX_intelligence_agency_logo_ger"),
            }],
            agency_capacity_clusters: vec![AgencyCapacityClusterRow {
                owner_tag_db_id: 2,
                value_0xe4: 9,
                value_0xf0: 10,
                value_0xf4: 9,
                proof_hash: stable_hash32("agency_capacity_offsets_e4_f0_f4"),
            }],
            operatives: vec![OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_387,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(71_372_000),
                name_hash: stable_hash64("Johann Hein"),
                state_hash: stable_hash32("lifeflag_47"),
                lifecycle_flag: Some(47),
            }],
            cryptology: vec![CryptologyRow {
                owner_tag_db_id: 1,
                target_tag_db_id: 3,
                amount_raw: 960_000_000,
                decrypting: true,
            }],
            operation_anchors: vec![OperationAnchorRow {
                owner_tag_db_id: 1,
                operation_id: 231_592,
                target_tag_db_id: Some(3),
                slot_count: 2,
                phase_count: 3,
                detail_resisted: true,
            }],
            intel_map: vec![IntelMapRow {
                owner_tag_db_id: 1,
                target_tag_db_id: 3,
                civilian_raw: 0,
                army_raw: 0,
                navy_raw: 0,
                air_raw: 0,
                resisted: true,
            }],
            state_events: vec![
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(3),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_RELATION_FLAG,
                    object_hash: stable_hash32("relation_flag_market_access"),
                    value_hash: stable_hash32("directed_false"),
                    value_raw: REL_KIND_MARKET_ACCESS_RIGHTS as i64,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(22),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
                    object_hash: stable_hash32("autonomy_puppet"),
                    value_hash: stable_hash32("autonomy_ratio_40000"),
                    value_raw: 40_000,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(3),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_RELATION_OPINION,
                    object_hash: stable_hash32("relation_opinion"),
                    value_hash: stable_hash32("opinion_neg120"),
                    value_raw: -120,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
                    object_hash: stable_hash32("country_faction_group"),
                    value_hash: stable_hash32("faction_group_42"),
                    value_raw: 42,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_AGENCY_SCALAR,
                    object_hash: stable_hash32("agency_upgrade_progress_raw"),
                    value_hash: stable_hash32("agency_upgrade_progress_2400000"),
                    value_raw: 2_400_000,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
                    object_hash: stable_hash32("operative_11387_lifecycle"),
                    value_hash: stable_hash32("lifeflag_47"),
                    value_raw: 47,
                },
                StateEventRow {
                    owner_tag_db_id: 2,
                    target_tag_db_id: None,
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_AGENCY_CAPACITY,
                    object_hash: stable_hash32("agency_capacity_offsets_e4_f0_f4"),
                    value_hash: stable_hash32("agency_capacity_9_10_9"),
                    value_raw: 10,
                },
                StateEventRow {
                    owner_tag_db_id: 1,
                    target_tag_db_id: Some(3),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_CRYPTOLOGY,
                    object_hash: stable_hash32("cryptology_amount_raw"),
                    value_hash: stable_hash32("cryptology_amount_960000000"),
                    value_raw: 960_000_000,
                },
            ],
            operation_instances: Vec::new(),
            incoming_actions: Vec::new(),
            intel_network_graphs: Vec::new(),
            factions: Vec::new(),
            faction_roster_complete: false,
        }
    }

    /// A snapshot with ALL FOUR trailers populated (OPIN, DIAQ, INGR, FACS), on
    /// top of the dense base fixture. Exercises the hybrid trailer path in
    /// `summarize_snapshot` (re-encode trailers + real decoder).
    fn fixture_snapshot_with_trailers() -> DiplomacyIntelSnapshot {
        let mut snapshot = fixture_snapshot();
        // Clear STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED so INGR rows are allowed,
        // and STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED (informational only).
        snapshot.status_flags &= !STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
        snapshot.operation_instances = vec![
            OperationInstanceRow::legacy(5001, true, Some(3)),
            OperationInstanceRow::legacy(5002, false, None),
        ];
        snapshot.incoming_actions = vec![
            IncomingActionQueueRow {
                recipient_tag_db_id: 1,
                actor_tag_db_id: 3,
                action_id: 900,
                action_type: 2,
                date_hours: 60_795_400,
                last_command_date_hours: 60_795_300,
            },
            IncomingActionQueueRow {
                recipient_tag_db_id: 1,
                actor_tag_db_id: 22,
                action_id: 901,
                action_type: 4,
                date_hours: 60_795_420,
                last_command_date_hours: 60_795_310,
            },
        ];
        snapshot.intel_network_graphs = vec![IntelNetworkGraphRow {
            strategic_operative_index: 7,
            network_slot: 0,
            target_tag_db_id: 3,
            state_ids: vec![11, 42, 108],
            state_strengths: None,
            coverage: None,
        }];
        snapshot.factions = vec![FactionRow {
            faction_id: 1,
            faction_type: 2,
            name_hash: stable_hash32("allies"),
            leader_tag_db_id: 1,
            member_tag_db_ids: vec![1, 3, 22],
        }];
        snapshot
    }

    /// GOVERNING INVARIANT: the encode-side `summarize_snapshot` must be
    /// FIELD-IDENTICAL to `decode_payload(0, &encode_payload(&s))` — status,
    /// counts, the full `data_units_total` map, AND the `parse_errors` vec
    /// (order + exact strings). Proven over clean AND intentionally-bad SNAPSHOTS.
    fn assert_inv(snapshot: &DiplomacyIntelSnapshot) {
        let payload = encode_payload(snapshot);
        let decode_summary = decode_payload(0, &payload);
        let encode_summary = summarize_snapshot(0, snapshot);
        assert_eq!(
            encode_summary, decode_summary,
            "summarize_snapshot must equal decode_payload; decode.parse_errors={:?}",
            decode_summary.parse_errors
        );
        // Wire bytes must be identical to the pre-change encoder (encode untouched).
        assert_eq!(
            encode_payload(snapshot),
            payload,
            "encode_payload must be deterministic / unchanged"
        );
    }

    #[test]
    fn inv_clean_dense_fixture() {
        assert_inv(&fixture_snapshot());
    }

    #[test]
    fn inv_clean_trailer_fixture() {
        assert_inv(&fixture_snapshot_with_trailers());
    }

    #[test]
    fn inv_empty_snapshot() {
        assert_inv(&DiplomacyIntelSnapshot::default());
    }

    #[test]
    fn inv_bad_changed_rows_gt_row_count() {
        let mut s = fixture_snapshot();
        s.changed_rows = Some(s.row_count() + 5);
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_unknown_status_bit() {
        let mut s = fixture_snapshot();
        s.status_flags |= 1 << 31;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_negative_casualties() {
        let mut s = fixture_snapshot();
        s.countries[0].casualties = -1;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_out_of_range_opinion() {
        let mut s = fixture_snapshot();
        s.opinions[0].opinion = MAX_OPINION_ABS + 1;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_duplicate_country_tag() {
        let mut s = fixture_snapshot();
        let dup = s.countries[0].clone();
        s.countries.push(dup);
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_invalid_country_tag() {
        let mut s = fixture_snapshot();
        s.countries[0].tag_db_id = 0;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_zero_object_hash_state_event() {
        let mut s = fixture_snapshot();
        s.state_events[0].object_hash = 0;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_unknown_state_event_source() {
        let mut s = fixture_snapshot();
        s.state_events[0].source_kind = 999;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_subject_start_date_non_positive() {
        let mut s = fixture_snapshot();
        s.subjects[0].start_date_raw = 0;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_operative_out_of_range_skill() {
        let mut s = fixture_snapshot();
        s.operatives[0].skill = Some(MAX_OPERATIVE_SKILL + 1);
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_agency_capacity_e4_f4_mismatch() {
        let mut s = fixture_snapshot();
        s.agency_capacity_clusters[0].value_0xf4 = s.agency_capacity_clusters[0].value_0xe4 + 1;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_multiple_errors_ordering() {
        // Several violations across kinds at once — the STRONGEST ordering check:
        // summarize must emit the same parse_errors in the same order as decode.
        let mut s = fixture_snapshot();
        s.status_flags |= 1 << 31; // unknown status bit (early)
        s.changed_rows = Some(s.row_count() + 1); // changed_rows>row_count (earliest)
        s.countries[0].casualties = -5; // negative casualties (country row)
        s.opinions[0].opinion = -(MAX_OPINION_ABS + 10); // opinion range (opinion row)
        s.subjects[0].autonomy_state_hash = 0; // zero hash (subject row)
        s.state_events[2].value_hash = 0; // zero value_hash (state-event row)
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_trailer_faction_members_not_increasing() {
        let mut s = fixture_snapshot_with_trailers();
        s.factions[0].member_tag_db_ids = vec![3, 1, 22]; // not strictly increasing
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_trailer_intel_network_invalid_target() {
        let mut s = fixture_snapshot_with_trailers();
        s.intel_network_graphs[0].target_tag_db_id = 0; // invalid target
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_trailer_operation_instance_zero_id() {
        let mut s = fixture_snapshot_with_trailers();
        s.operation_instances[0].instance_id = 0;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_trailer_incoming_action_dup_id() {
        let mut s = fixture_snapshot_with_trailers();
        s.incoming_actions[1].action_id = s.incoming_actions[0].action_id;
        assert_inv(&s);
    }

    #[test]
    fn inv_bad_war_root_unvalidated_with_war_rows() {
        // Status-vs-row consistency block: badge war-root-unvalidated while war
        // rows are present -> decode pushes a consistency error; summarize must too.
        let mut s = fixture_snapshot();
        s.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        assert_inv(&s);
    }

    #[test]
    fn sampled_net_detects_byte_corruption() {
        // Class-2 (post-encode byte corruption) is OUT of summarize_snapshot's
        // scope by design — it works from the snapshot, not the bytes. This is
        // exactly what the 1/N sampled decode guards: corrupt the encoded bytes and
        // confirm decode_payload(corrupted) DIFFERS from summarize_snapshot(clean),
        // so the sampled cross-check would fire (fail closed). Mirror the existing
        // byte-mutation test: overwrite a state-event source_kind with 999.
        let snapshot = fixture_snapshot();
        let clean_summary = summarize_snapshot(0, &snapshot);
        let mut corrupted = encode_payload(&snapshot);
        let state_event_offset = HEADER_LEN
            + ROW_COUNTRY_LEN
            + ROW_WAR_LEN
            + ROW_OPINION_LEN
            + ROW_RELATION_FLAG_LEN
            + ROW_SUBJECT_LEN
            + ROW_AGENCY_LEN
            + ROW_AGENCY_CAPACITY_CLUSTER_LEN
            + ROW_OPERATIVE_LEN
            + ROW_CRYPTOLOGY_LEN
            + ROW_OPERATION_ANCHOR_LEN
            + ROW_INTEL_MAP_LEN;
        corrupted[state_event_offset + 14..state_event_offset + 16]
            .copy_from_slice(&999u16.to_le_bytes());
        let decode_summary = decode_payload(0, &corrupted);
        assert_ne!(
            clean_summary, decode_summary,
            "sampled decode must detect post-encode byte corruption"
        );
    }
}
