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

pub const STATUS_SOURCE_UNAVAILABLE: u32 = 1 << 8;
pub const STATUS_PARTIAL_DATA: u32 = 1 << 9;
pub const STATUS_DELTA_DATA: u32 = 1 << 10;
pub const STATUS_OPERATIONS_DETAIL_RESISTED: u32 = 1 << 16;
pub const STATUS_INTEL_MAP_RESISTED: u32 = 1 << 17;
pub const STATUS_WAR_ROOT_UNVALIDATED: u32 = 1 << 18;
pub const STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED: u32 = 1 << 19;
pub const STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED: u32 = 1 << 20;

pub const KNOWN_STATUS_FLAGS: u32 = STATUS_SOURCE_UNAVAILABLE
    | STATUS_PARTIAL_DATA
    | STATUS_DELTA_DATA
    | STATUS_OPERATIONS_DETAIL_RESISTED
    | STATUS_INTEL_MAP_RESISTED
    | STATUS_WAR_ROOT_UNVALIDATED
    | STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;

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

pub const OPERATIVE_FLAG_DETECTED: u16 = 1;
pub const OPERATIVE_FLAG_SKILL_PRESENT: u16 = 1 << 1;
pub const OPERATIVE_FLAG_EXPERIENCE_PRESENT: u16 = 1 << 2;
pub const OPERATIVE_FLAG_NAME_HASH: u16 = 1 << 3;
pub const OPERATIVE_FLAG_CAPTURED: u16 = 1 << 4;

pub const CRYPTO_FLAG_DECRYPTING: u16 = 1;

pub const OPERATION_FLAG_RUNNING: u16 = 1;
pub const OPERATION_FLAG_DETAIL_RESISTED: u16 = 1 << 1;
pub const OPERATION_FLAG_TARGET_PRESENT: u16 = 1 << 2;

pub const INTEL_MAP_FLAG_RESISTED: u16 = 1;

pub const MAX_TAG_DB_ID: u32 = 2_048;
pub const MAX_CASUALTIES: i64 = 50_000_000;
pub const MAX_OPINION_ABS: i32 = 200;
pub const MAX_OPERATIVES_PER_AGENCY: u32 = 128;
pub const MAX_OPERATIVE_ID: u32 = i32::MAX as u32;
pub const MAX_OPERATIVE_SKILL: u32 = 15;
pub const MAX_FIXED_RAW: i32 = 1_000_000_000;
pub const MAX_XP_RAW: i64 = 1_000_000_000_000;
pub const MAX_CRYPTO_AMOUNT_RAW: i64 = 1_000_000_000_000;
pub const MAX_OPERATION_ID: u32 = i32::MAX as u32;
pub const MAX_OPERATION_SLOTS: u32 = 16;
pub const MAX_OPERATION_PHASES: u32 = 32;
pub const MAX_INTEL_PERCENT_RAW: i32 = 100_000;

const ROW_COUNTRY_LEN: usize = 24;
const ROW_WAR_LEN: usize = 32;
const ROW_OPINION_LEN: usize = 16;
const ROW_RELATION_FLAG_LEN: usize = 16;
const ROW_SUBJECT_LEN: usize = 24;
const ROW_AGENCY_LEN: usize = 48;
const ROW_OPERATIVE_LEN: usize = 40;
const ROW_CRYPTOLOGY_LEN: usize = 32;
const ROW_OPERATION_ANCHOR_LEN: usize = 32;
const ROW_INTEL_MAP_LEN: usize = 32;

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
    pub operatives: Vec<OperativeRow>,
    pub cryptology: Vec<CryptologyRow>,
    pub operation_anchors: Vec<OperationAnchorRow>,
    pub intel_map: Vec<IntelMapRow>,
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
            self.operatives.len(),
            self.cryptology.len(),
            self.operation_anchors.len(),
            self.intel_map.len(),
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
pub struct OperativeRow {
    pub owner_tag_db_id: u32,
    pub op_id: u32,
    pub detected: bool,
    pub captured: bool,
    pub skill: Option<u32>,
    pub experience_raw: Option<i64>,
    pub name_hash: u64,
    pub state_hash: u32,
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
    payload.extend_from_slice(&0u32.to_le_bytes());

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
        payload.extend_from_slice(&KIND_OPERATIVE.to_le_bytes());
        payload.extend_from_slice(&flags.to_le_bytes());
        payload.extend_from_slice(&row.owner_tag_db_id.to_le_bytes());
        payload.extend_from_slice(&row.op_id.to_le_bytes());
        payload.extend_from_slice(&row.skill.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.experience_raw.unwrap_or(0).to_le_bytes());
        payload.extend_from_slice(&row.name_hash.to_le_bytes());
        payload.extend_from_slice(&row.state_hash.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
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
    payload
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
    let expected_counts = ExpectedCounts {
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
    };
    let reserved0 = read_u32_le(payload, 68);

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
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel header reserved field is nonzero"
        ));
    }
    if changed_rows > row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel changed_rows {changed_rows} > row_count {row_count}"
        ));
    }
    if expected_counts.sum() != row_count {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel row_count {row_count} != sum of per-kind counts {}",
            expected_counts.sum()
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
    let mut seen_operatives = BTreeSet::new();
    let mut seen_crypto = BTreeSet::new();
    let mut seen_operations = BTreeSet::new();
    let mut seen_intel_map = BTreeSet::new();
    let mut country_war_projection_rows = 0usize;
    let mut operation_detail_resisted_rows = 0usize;
    let mut intel_map_resisted_rows = 0usize;

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
            KIND_OPERATIVE => {
                decoded_counts.operative += 1;
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
            _ => unreachable!("row_len accepted only known kinds"),
        }
        cursor += row_len;
    }

    if cursor != payload.len() {
        summary.parse_errors.push(format!(
            "offset {offset}: diplomacy/intel payload has {} trailing byte(s)",
            payload.len().saturating_sub(cursor)
        ));
    }
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
    add_unit(&mut summary, "operative_rows", decoded_counts.operative);
    add_unit(&mut summary, "cryptology_rows", decoded_counts.cryptology);
    add_unit(
        &mut summary,
        "operation_anchor_rows",
        decoded_counts.operation_anchor,
    );
    add_unit(&mut summary, "intel_map_rows", decoded_counts.intel_map);
    summary
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
    operative: usize,
    cryptology: usize,
    operation_anchor: usize,
    intel_map: usize,
}

impl ExpectedCounts {
    fn sum(self) -> usize {
        self.country
            .saturating_add(self.war)
            .saturating_add(self.opinion)
            .saturating_add(self.relation_flag)
            .saturating_add(self.subject)
            .saturating_add(self.agency)
            .saturating_add(self.operative)
            .saturating_add(self.cryptology)
            .saturating_add(self.operation_anchor)
            .saturating_add(self.intel_map)
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
        KIND_OPERATIVE => Some(ROW_OPERATIVE_LEN),
        KIND_CRYPTOLOGY => Some(ROW_CRYPTOLOGY_LEN),
        KIND_OPERATION_ANCHOR => Some(ROW_OPERATION_ANCHOR_LEN),
        KIND_INTEL_MAP => Some(ROW_INTEL_MAP_LEN),
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
    let _state_hash = read_u32_le(payload, cursor + 32);
    let reserved = read_u32_le(payload, cursor + 36);
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
            | OPERATIVE_FLAG_CAPTURED)
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
    if reserved != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: operative row {row_index} reserved field is nonzero"
        ));
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
    fn production_domain_id_is_reserved_53() {
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
        assert_eq!(summary.data_rows_total, 10);
        assert_eq!(summary.data_changed_rows_total, 10);
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
            + ROW_AGENCY_LEN;
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
            operatives: vec![OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_387,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(71_372_000),
                name_hash: stable_hash64("Johann Hein"),
                state_hash: stable_hash32("on_mission"),
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
        }
    }
}
