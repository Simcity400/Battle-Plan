use std::collections::{BTreeMap, BTreeSet};

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
    | STATUS_FACTION_GROUP_UNVALIDATED;

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
const OPERATION_INSTANCE_TRAILER_VERSION: u16 = 1;
const OPERATION_INSTANCE_TRAILER_HEADER_LEN: usize = 12;
const OPERATION_INSTANCE_TRAILER_ROW_LEN: usize = 12;
const OPERATION_INSTANCE_FLAG_DEF_REF_OK: u32 = 1;
const OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT: u32 = 1 << 1;
pub const MAX_OPERATION_INSTANCE_ROWS: usize = 4_096;

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
    pub autonomy_ratio_raw: i32,
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

/// Wave-5 running-operation instance row: {instance_id, def_ref_ok,
/// associated_country}. The `owner` (+0x10) field is REJECTED and NOT carried.
/// `associated_country` is PROVISIONAL — semantics are associated-country;
/// target-vs-operative-location is undistinguished (all 8 coincided on D04).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationInstanceRow {
    pub instance_id: u32,
    /// The operation-def ptr (@+0x48) passed the COperation vtable check.
    pub def_ref_ok: bool,
    /// Resolved associated-country tag_db_id (@+0x50 -> CCountry). `None` when
    /// the pointer was null/invalid (absent, never zero).
    pub associated_country_tag_db_id: Option<u32>,
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
    let mut payload = Vec::new();
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
        let mut flags = COUNTRY_FLAG_FACTION_GROUP;
        if row.at_war_count > 0 {
            flags |= COUNTRY_FLAG_AT_WAR;
        }
        if row.casualties != 0 {
            flags |= COUNTRY_FLAG_CASUALTIES;
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
        payload.extend_from_slice(&0u32.to_le_bytes());
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
        payload.extend_from_slice(&KIND_AGENCY_CAPACITY_CLUSTER.to_le_bytes());
        payload.extend_from_slice(&AGENCY_CAPACITY_FLAG_RAW_E4_F0_F4.to_le_bytes());
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
    payload
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
    payload.extend_from_slice(OPERATION_INSTANCE_TRAILER_MAGIC);
    payload.extend_from_slice(&OPERATION_INSTANCE_TRAILER_VERSION.to_le_bytes());
    payload.extend_from_slice(&(OPERATION_INSTANCE_TRAILER_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
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

/// Decode + gate the `OPIN` trailer at `trailer_offset`. Returns the decoded
/// rows and appends any parse errors to `errors`. Strict: exact-end
/// consumption, unique instance ids, flag/country consistency.
fn decode_operation_instance_trailer(
    offset: usize,
    payload: &[u8],
    trailer_offset: usize,
    errors: &mut Vec<String>,
) -> Vec<OperationInstanceRow> {
    let Some(trailer) = payload.get(trailer_offset..) else {
        errors.push(format!(
            "offset {offset}: operation-instance trailer offset {trailer_offset} beyond payload"
        ));
        return Vec::new();
    };
    if trailer.len() < OPERATION_INSTANCE_TRAILER_HEADER_LEN {
        errors.push(format!(
            "offset {offset}: operation-instance trailer is {} byte(s), expected at least {}",
            trailer.len(),
            OPERATION_INSTANCE_TRAILER_HEADER_LEN
        ));
        return Vec::new();
    }
    if &trailer[0..4] != OPERATION_INSTANCE_TRAILER_MAGIC {
        errors.push(format!(
            "offset {offset}: operation-instance trailer magic is invalid"
        ));
        return Vec::new();
    }
    let version = read_u16_le(trailer, 4);
    let header_len = read_u16_le(trailer, 6) as usize;
    let row_count = read_u32_le(trailer, 8) as usize;
    if version != OPERATION_INSTANCE_TRAILER_VERSION
        || header_len != OPERATION_INSTANCE_TRAILER_HEADER_LEN
    {
        errors.push(format!(
            "offset {offset}: operation-instance trailer header shape invalid (version {version}, header_len {header_len})"
        ));
        return Vec::new();
    }
    if row_count > MAX_OPERATION_INSTANCE_ROWS {
        errors.push(format!(
            "offset {offset}: operation-instance trailer row_count {row_count} exceeds {MAX_OPERATION_INSTANCE_ROWS}"
        ));
        return Vec::new();
    }
    let expected = OPERATION_INSTANCE_TRAILER_HEADER_LEN
        .saturating_add(row_count.saturating_mul(OPERATION_INSTANCE_TRAILER_ROW_LEN));
    if trailer.len() != expected {
        errors.push(format!(
            "offset {offset}: operation-instance trailer len {} != {expected}",
            trailer.len()
        ));
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(row_count);
    let mut seen = BTreeSet::new();
    let mut cursor = OPERATION_INSTANCE_TRAILER_HEADER_LEN;
    for _ in 0..row_count {
        let instance_id = read_u32_le(trailer, cursor);
        let flags = read_u32_le(trailer, cursor + 4);
        let country = read_u32_le(trailer, cursor + 8);
        cursor += OPERATION_INSTANCE_TRAILER_ROW_LEN;
        if instance_id == 0 {
            errors.push(format!(
                "offset {offset}: operation-instance row has zero instance_id"
            ));
        }
        if flags & !(OPERATION_INSTANCE_FLAG_DEF_REF_OK | OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT)
            != 0
        {
            errors.push(format!(
                "offset {offset}: operation-instance row has unknown flags {flags:#x}"
            ));
        }
        let country_present = flags & OPERATION_INSTANCE_FLAG_COUNTRY_PRESENT != 0;
        if country_present {
            if country == 0 || country > MAX_TAG_DB_ID {
                errors.push(format!(
                    "offset {offset}: operation-instance row has invalid associated_country {country}"
                ));
            }
        } else if country != 0 {
            errors.push(format!(
                "offset {offset}: operation-instance row has country without present flag"
            ));
        }
        if !seen.insert(instance_id) {
            errors.push(format!(
                "offset {offset}: operation-instance trailer duplicates instance_id {instance_id}"
            ));
        }
        rows.push(OperationInstanceRow {
            instance_id,
            def_ref_ok: flags & OPERATION_INSTANCE_FLAG_DEF_REF_OK != 0,
            associated_country_tag_db_id: country_present.then_some(country),
        });
    }
    rows
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
    let mut seen_opinions = BTreeSet::new();
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();
    let mut seen_agencies = BTreeSet::new();
    let mut seen_agency_capacity_clusters = BTreeSet::new();
    let mut seen_operatives = BTreeSet::new();
    let mut seen_crypto = BTreeSet::new();
    let mut seen_operations = BTreeSet::new();
    let mut seen_intel_map = BTreeSet::new();
    let mut seen_state_events = BTreeSet::new();
    let mut country_war_projection_rows = 0usize;
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
        match kind {
            KIND_COUNTRY_DIPLOMACY => {
                decoded_counts.country += 1;
                let flags = read_u16_le(payload, cursor + 2);
                if flags & (COUNTRY_FLAG_AT_WAR | COUNTRY_FLAG_CASUALTIES) != 0 {
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
                state_event_source_counts.add(read_u16_le(payload, cursor + 14));
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

    // Wave-5: any bytes after the self-delimiting base payload are the `OPIN`
    // running-operation-instance trailer; anything else is a trailing-byte
    // error exactly as before.
    let mut operation_instance_rows = 0usize;
    let mut operation_instance_country_rows = 0usize;
    let mut operation_instance_def_ref_ok_rows = 0usize;
    if cursor != payload.len() {
        let trailer_rows =
            decode_operation_instance_trailer(offset, payload, cursor, &mut summary.parse_errors);
        operation_instance_rows = trailer_rows.len();
        operation_instance_country_rows = trailer_rows
            .iter()
            .filter(|row| row.associated_country_tag_db_id.is_some())
            .count();
        operation_instance_def_ref_ok_rows =
            trailer_rows.iter().filter(|row| row.def_ref_ok).count();
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
    if decoded_counts != expected_counts {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel per-kind counts decoded {:?} != header {:?}",
            decoded_counts, expected_counts
        ));
    }
    if status_flags & STATUS_WAR_ROOT_UNVALIDATED != 0
        && (decoded_counts.war != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges war root unvalidated but emits {} war relation row(s) and {} country war projection row(s)",
            decoded_counts.war, country_war_projection_rows
        ));
    }
    if status_flags & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED == 0
        && (decoded_counts.war != 0 || country_war_projection_rows != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel emits {} war relation row(s) and {} country war projection row(s) without STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED; schema v{SCHEMA_VERSION} has no war name/grouping/start-date fields",
            decoded_counts.war, country_war_projection_rows
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
        && (decoded_counts.country != 0 || state_event_source_counts.country_faction_group != 0)
    {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel badges faction group unvalidated but emits {} country diplomacy row(s) and {} faction state event row(s)",
            decoded_counts.country, state_event_source_counts.country_faction_group
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
    // Wave-5: parse the `OPIN` running-operation-instance trailer (validated
    // above by decode_payload, whose parse errors already gated this call).
    if cursor != payload.len() {
        let mut trailer_errors = Vec::new();
        snapshot.operation_instances =
            decode_operation_instance_trailer(offset, payload, cursor, &mut trailer_errors);
        if !trailer_errors.is_empty() {
            return Err(trailer_errors);
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
    let faction_group = read_i32_le(payload, cursor + 8);
    let at_war_count = read_u32_le(payload, cursor + 12);
    let casualties = read_i64_le(payload, cursor + 16);
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
    let second = read_u32_le(payload, cursor + 8);
    let first_casualties = read_i64_le(payload, cursor + 12);
    let second_casualties = read_i64_le(payload, cursor + 20);
    let reserved = read_u32_le(payload, cursor + 28);
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
    seen: &mut BTreeSet<(u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let src = read_u32_le(payload, cursor + 4);
    let target = read_u32_le(payload, cursor + 8);
    let opinion = read_i32_le(payload, cursor + 12);
    if flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} has unknown flags {flags}"
        ));
    }
    validate_pair(offset, "opinion", row_index, src, target, summary);
    if valid_tag(src) && valid_tag(target) && !seen.insert((src, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: opinion row {row_index} duplicates ordered pair {src}/{target}"
        ));
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
    let target = read_u32_le(payload, cursor + 12);
    validate_pair(offset, "relation flag", row_index, src, target, summary);
    if !known_relation_kind(flag_kind) {
        summary.parse_errors.push(format!(
            "offset {offset}: relation flag row {row_index} has unknown flag_kind {flag_kind}"
        ));
    }
    if flags & !REL_FLAG_DIRECTED != 0 {
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
    let subject = read_u32_le(payload, cursor + 8);
    let autonomy_state_hash = read_u32_le(payload, cursor + 12);
    let autonomy_ratio_raw = read_i32_le(payload, cursor + 16);
    let reserved = read_u32_le(payload, cursor + 20);
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
    if autonomy_state_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has zero autonomy_state_hash"
        ));
    }
    if !(0..=1_000_000).contains(&autonomy_ratio_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} has invalid autonomy_ratio_raw {autonomy_ratio_raw}"
        ));
    }
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: subject row {row_index} reserved field is nonzero"
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
    let max_operatives = read_u32_le(payload, cursor + 8);
    let usable_slots = read_u32_le(payload, cursor + 12);
    let operative_count = read_u32_le(payload, cursor + 16);
    let cryptology_target_count = read_u32_le(payload, cursor + 20);
    let defense_raw = read_i32_le(payload, cursor + 24);
    let upgrade_progress_raw = read_i32_le(payload, cursor + 28);
    let name_hash = read_u64_le(payload, cursor + 32);
    let icon_hash = read_u64_le(payload, cursor + 40);
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
    let value_0xe4 = read_u32_le(payload, cursor + 8);
    let value_0xf0 = read_u32_le(payload, cursor + 12);
    let value_0xf4 = read_u32_le(payload, cursor + 16);
    let proof_hash = read_u32_le(payload, cursor + 20);
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
    let skill = read_u32_le(payload, cursor + 12);
    let experience_raw = read_i64_le(payload, cursor + 16);
    let name_hash = read_u64_le(payload, cursor + 24);
    let state_hash = read_u32_le(payload, cursor + 32);
    let lifecycle_flag = read_u32_le(payload, cursor + 36);
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
    let target = read_u32_le(payload, cursor + 8);
    let amount_raw = read_i64_le(payload, cursor + 12);
    let reserved0 = read_u64_le(payload, cursor + 20);
    let reserved1 = read_u32_le(payload, cursor + 28);
    validate_pair(offset, "cryptology", row_index, owner, target, summary);
    if flags & !CRYPTO_FLAG_DECRYPTING != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} has unknown flags {flags}"
        ));
    }
    if valid_tag(owner) && valid_tag(target) && !seen.insert((owner, target)) {
        summary.parse_errors.push(format!(
            "offset {offset}: cryptology row {row_index} duplicates owner/target"
        ));
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
    seen: &mut BTreeSet<(u16, u16, u32, u32, u32)>,
    summary: &mut Domain53GateSummary,
) {
    let flags = read_u16_le(payload, cursor + 2);
    let owner = read_u32_le(payload, cursor + 4);
    let target = read_u32_le(payload, cursor + 8);
    let event_kind = read_u16_le(payload, cursor + 12);
    let source_kind = read_u16_le(payload, cursor + 14);
    let object_hash = read_u32_le(payload, cursor + 16);
    let value_hash = read_u32_le(payload, cursor + 20);
    let value_raw = read_i64_le(payload, cursor + 24);

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
    if value_hash == 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has zero value_hash"
        ));
    }
    if !(-MAX_STATE_EVENT_VALUE_RAW..=MAX_STATE_EVENT_VALUE_RAW).contains(&value_raw) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} has invalid value_raw {value_raw}"
        ));
    }

    let key = (event_kind, source_kind, owner, target, object_hash);
    if object_hash != 0 && !seen.insert(key) {
        summary.parse_errors.push(format!(
            "offset {offset}: compact state event row {row_index} duplicates state event key"
        ));
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
            OperationInstanceRow {
                instance_id: 129_014,
                def_ref_ok: true,
                associated_country_tag_db_id: Some(3),
            },
            OperationInstanceRow {
                instance_id: 128_904,
                def_ref_ok: false,
                associated_country_tag_db_id: None,
            },
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
    fn operation_instance_trailer_rejects_duplicate_instance_id() {
        let mut snapshot = fixture_snapshot();
        snapshot.operation_instances = vec![
            OperationInstanceRow {
                instance_id: 42,
                def_ref_ok: true,
                associated_country_tag_db_id: None,
            },
            OperationInstanceRow {
                instance_id: 42,
                def_ref_ok: true,
                associated_country_tag_db_id: None,
            },
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
        snapshot.operation_instances = vec![OperationInstanceRow {
            instance_id: 7,
            def_ref_ok: true,
            associated_country_tag_db_id: None,
        }];
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
            }],
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
        }
    }
}
