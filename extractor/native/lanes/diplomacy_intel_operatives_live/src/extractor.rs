use crate::compact::{
    decode_payload, encode_payload, AgencyCapacityClusterRow, AgencyRow, CountryDiplomacyRow,
    CryptologyRow, DiplomacyIntelSnapshot, Domain53GateSummary, IntelMapRow, OperationAnchorRow,
    OperativeRow, OpinionRow, RelationFlagRow, StateEventRow, SubjectRow, WarRelationRow,
    DOMAIN_ID_PROPOSED, MAX_AGENCY_CAPACITY_CLUSTER_VALUE, MAX_CASUALTIES, MAX_CRYPTO_AMOUNT_RAW,
    MAX_FIXED_RAW, MAX_OPERATIVE_ID, MAX_OPERATIVE_LIFECYCLE_FLAG, MAX_OPERATIVE_SKILL,
    MAX_OPINION_ABS, MAX_TAG_DB_ID, MAX_XP_RAW, REL_KIND_AIR_BASE_ACCESS, REL_KIND_ATTACHE,
    REL_KIND_DOCKING_RIGHTS, REL_KIND_EMBARGO, REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION,
    REL_KIND_GUARANTEE, REL_KIND_IMPROVE_RELATIONS, REL_KIND_LICENCE_PRODUCTION,
    REL_KIND_MARKET_ACCESS_RIGHTS, REL_KIND_MILITARY_ACCESS, REL_KIND_NAVAL_BLOCKADE,
    REL_KIND_NON_AGGRESSION_PACT, REL_KIND_PUPPET, STATE_EVENT_KIND_OBSERVED,
    STATE_EVENT_SOURCE_AGENCY_CAPACITY, STATE_EVENT_SOURCE_AGENCY_SCALAR,
    STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP, STATE_EVENT_SOURCE_CRYPTOLOGY,
    STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE, STATE_EVENT_SOURCE_RELATION_FLAG,
    STATE_EVENT_SOURCE_RELATION_OPINION, STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
    STATUS_FACTION_GROUP_UNVALIDATED, STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
    STATUS_INTEL_MAP_RESISTED, STATUS_OPERATIONS_DETAIL_RESISTED,
    STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED, STATUS_WAR_ROOT_UNVALIDATED,
};
use compact_contract::build_profile as profile;
use std::collections::{BTreeMap, BTreeSet};

pub const EXPECTED_HOI4_SHA256_870EBF: &str =
    "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad";

pub const SOURCE_PROOF_BUILD_SHA_870EBF: u64 = 1 << 0;
pub const SOURCE_PROOF_COUNTRY_ROOTS_870EBF: u64 = 1 << 1;
pub const SOURCE_PROOF_RELATION_ROOTS_870EBF: u64 = 1 << 2;
pub const SOURCE_PROOF_AGENCY_ROOT_870EBF: u64 = 1 << 3;
pub const SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF: u64 = 1 << 4;
pub const SOURCE_PROOF_OPERATIVE_ROOT_870EBF: u64 = 1 << 5;
pub const SOURCE_PROOF_WAR_ROOT_870EBF: u64 = 1 << 6;
pub const SOURCE_PROOF_OPERATION_DETAIL_870EBF: u64 = 1 << 7;
pub const SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF: u64 = 1 << 8;
pub const SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF: u64 = 1 << 9;
pub const SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF: u64 = 1 << 10;
pub const SOURCE_PROOF_FACTION_GROUP_870EBF: u64 = 1 << 11;

pub const KNOWN_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_OPERATION_DETAIL_870EBF
    | SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF;

pub const CURRENT_VALIDATED_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF;

const RTTI_CCOUNTRY: &str = "CCountry";
const RTTI_CINTELLIGENCE_AGENCY: &str = "CIntelligenceAgency";
const DOMAIN53_RTTI_MAX_NAME_LEN: usize = 200;
const DOMAIN53_MODULE_SPAN_MAX: u64 = 0x5000_0000;
const DOMAIN53_FACTION_PROBE_COUNTRY_FLOOR: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CurrentBuildRoot {
    pub name: &'static str,
    pub evidence: &'static str,
}

pub const CURRENT_BUILD_ROOTS_870EBF: &[CurrentBuildRoot] = &[
    CurrentBuildRoot {
        name: "game_state",
        evidence: "u64[module+0x3316E90]",
    },
    CurrentBuildRoot {
        name: "country_database",
        evidence: "u64[module+0x33189C8]",
    },
    CurrentBuildRoot {
        name: "country_table",
        evidence: "game_state+0x310 table, game_state+0x31C count, country vtable module+0x27D0E90",
    },
    CurrentBuildRoot {
        name: "relations",
        evidence: "country+0xF88 -> manager, manager+0x08 -> table, relation+0x300 opinion",
    },
    CurrentBuildRoot {
        name: "subject_relation",
        evidence: "CSubjectRelation module+0x2949F18, overlord +0x10, subject +0x14, autonomy +0x58/+0x60",
    },
    CurrentBuildRoot {
        name: "agency",
        evidence: "country+0xFC0 -> CIntelligenceAgency, vtable module+0x296B338, capacity raw cluster agency+0xE4/+0xF0/+0xF4 with +0xE4==+0xF4 warm invariant",
    },
    CurrentBuildRoot {
        name: "cryptology",
        evidence: "agency+0x120 -> CCryptology module+0x29A6540; records manager+0x28 stride 0x38, state vtable module+0x29A64F0",
    },
    CurrentBuildRoot {
        name: "operative_identity",
        evidence: "COperativeLeader vtable/classdesc module+0x293F790, identity+0x110 lifecycle flag cohorts {47,31,63}, owner identity+0x118, skill/XP stats path",
    },
    CurrentBuildRoot {
        name: "war_relation_rows",
        evidence: "global row pool with tag ids +0x00/+0x04, date vtable +0x08 in {CGregorianDate module+0x2702190, CGameDate module+0x2702388}, casualties +0x40/+0x48 i32, echo +0x68/+0x6C; v13 warm validation 850/850",
    },
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtractedDomain53Rows {
    pub elapsed_wall_ms: u64,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExtractError {
    MemoryRead { address: u64, len: usize },
    InvalidRoot(&'static str),
    InvalidCount { name: &'static str, value: u32 },
    Validation(String),
}

pub trait Domain53Memory {
    fn module_base(&self) -> u64;
    fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError>;

    fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
        Vec::new()
    }

    fn read_u8(&self, address: u64) -> Result<u8, ExtractError> {
        let bytes = self.read_bytes(address, 1)?;
        Ok(bytes[0])
    }

    fn read_u32(&self, address: u64) -> Result<u32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(u32::from_le_bytes(
            bytes.try_into().expect("u32 byte count"),
        ))
    }

    fn read_i32(&self, address: u64) -> Result<i32, ExtractError> {
        let bytes = self.read_bytes(address, 4)?;
        Ok(i32::from_le_bytes(
            bytes.try_into().expect("i32 byte count"),
        ))
    }

    fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(u64::from_le_bytes(
            bytes.try_into().expect("u64 byte count"),
        ))
    }

    fn read_i64(&self, address: u64) -> Result<i64, ExtractError> {
        let bytes = self.read_bytes(address, 8)?;
        Ok(i64::from_le_bytes(
            bytes.try_into().expect("i64 byte count"),
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Domain53FamilyCounts {
    pub country_roots: u32,
    pub relation_rows: u32,
    pub agency_roots: u32,
    pub operative_scan_ranges: u32,
    pub war_scan_ranges: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedDomain53Frame {
    pub rows: ExtractedDomain53Rows,
    pub source_proofs: u64,
    pub observed_rows: u32,
    pub skipped_rows: u32,
    pub family_counts: Domain53FamilyCounts,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountryRoot {
    tag_db_id: u32,
    country_ptr: u64,
}

#[derive(Clone, Debug, Default)]
struct RowAccumulator {
    rows: ExtractedDomain53Rows,
    skipped_rows: u32,
    observed_rows: u32,
}

impl RowAccumulator {
    fn skip(&mut self) {
        self.skipped_rows = self.skipped_rows.saturating_add(1);
    }

    fn observe(&mut self) {
        self.observed_rows = self.observed_rows.saturating_add(1);
    }
}

#[derive(Clone, Debug)]
struct AgencyRootRead {
    owner_tag_db_id: u32,
    agency_ptr: u64,
}

pub fn extract_rows_from_current_roots<M: Domain53Memory>(
    mem: &M,
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let module_base = mem.module_base();
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }

    let game_state = mem.read_u64(add_addr(module_base, profile::CG_STATE_DATE_OFFSET)?)?;
    let country_database =
        mem.read_u64(add_addr(module_base, profile::COUNTRY_DATABASE_OFFSET)?)?;
    if !is_heap_ptr(game_state) || !is_heap_ptr(country_database) {
        return Err(ExtractError::InvalidRoot(
            "domain53_global_roots_unavailable",
        ));
    }
    verify_domain53_build_identity(mem, module_base)?;

    let countries = resolve_country_roots(mem, module_base, game_state)?;
    if countries.is_empty() {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_roots_empty_implausible",
        ));
    }

    let mut acc = RowAccumulator::default();
    let mut source_proofs = SOURCE_PROOF_BUILD_SHA_870EBF | SOURCE_PROOF_COUNTRY_ROOTS_870EBF;
    let mut family_counts = Domain53FamilyCounts {
        country_roots: countries.len().min(u32::MAX as usize) as u32,
        ..Domain53FamilyCounts::default()
    };

    let war_rows = read_war_rows(mem, module_base, &countries, &mut acc)?;
    family_counts.war_scan_ranges = mem
        .readable_ranges(
            profile::DOMAIN53_HEAP_LO as u64,
            profile::DOMAIN53_HEAP_HI as u64,
        )
        .len()
        .min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_WAR_ROOT_870EBF;
    let war_projection = war_projection_by_country(&war_rows);
    let faction_gate = read_faction_groups(mem, &countries)?;
    if faction_gate.validated {
        source_proofs |= SOURCE_PROOF_FACTION_GROUP_870EBF;
    }

    if faction_gate.validated {
        for country in &countries {
            let Some(&faction_group) = faction_gate.groups_by_country.get(&country.tag_db_id)
            else {
                continue;
            };
            let (at_war_count, casualties) = war_projection
                .get(&country.tag_db_id)
                .copied()
                .unwrap_or((0, 0));
            acc.rows.countries.push(CountryDiplomacyRow {
                tag_db_id: country.tag_db_id,
                faction_group,
                at_war_count,
                casualties,
            });
            acc.rows.state_events.push(state_event(
                country.tag_db_id,
                None,
                STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
                "country_faction_group",
                &format!("faction_group_{faction_group}"),
                i64::from(faction_group),
            ));
            acc.observe();
        }
    }

    read_relation_rows(mem, module_base, &countries, &mut acc)?;
    let relation_rows =
        acc.rows.opinions.len() + acc.rows.relation_flags.len() + acc.rows.subjects.len();
    if relation_rows == 0 {
        return Err(ExtractError::InvalidRoot(
            "domain53_relations_empty_implausible",
        ));
    }
    family_counts.relation_rows = relation_rows.min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_RELATION_ROOTS_870EBF;

    let agency_roots = read_agency_rows(mem, module_base, &countries, &mut acc)?;
    family_counts.agency_roots = agency_roots.len().min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_AGENCY_ROOT_870EBF;

    let operative_scan_ranges = read_operatives(mem, module_base, &countries, &mut acc)?;
    family_counts.operative_scan_ranges = operative_scan_ranges;
    source_proofs |= SOURCE_PROOF_OPERATIVE_ROOT_870EBF;

    for agency in &mut acc.rows.agencies {
        agency.operative_count = acc
            .rows
            .operatives
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }

    read_cryptology_rows(mem, module_base, &agency_roots, &mut acc)?;
    if !agency_roots.is_empty() {
        source_proofs |= SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF;
    }
    for agency in &mut acc.rows.agencies {
        agency.cryptology_target_count = acc
            .rows
            .cryptology
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }

    acc.rows.wars = war_rows;

    Ok(ExtractedDomain53Frame {
        rows: acc.rows,
        source_proofs,
        observed_rows: acc.observed_rows,
        skipped_rows: acc.skipped_rows,
        family_counts,
    })
}

fn verify_domain53_build_identity<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
) -> Result<(), ExtractError> {
    verify_vtable_rtti_name(
        mem,
        module_base,
        profile::DOMAIN53_COUNTRY_VTABLE_RVA,
        RTTI_CCOUNTRY,
    )?;
    verify_vtable_rtti_name(
        mem,
        module_base,
        profile::DOMAIN53_AGENCY_VTABLE_RVA,
        RTTI_CINTELLIGENCE_AGENCY,
    )?;
    Ok(())
}

fn verify_vtable_rtti_name<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    vtable_rva: usize,
    expected_undecorated: &str,
) -> Result<(), ExtractError> {
    let vtable = add_addr(module_base, vtable_rva)?;
    let decorated = rtti_type_name_for_vtable(mem, module_base, vtable)?;
    let Some(undecorated) = undecorate_msvc_class_name(&decorated) else {
        return Err(ExtractError::Validation(format!(
            "domain53_rtti_name_not_class:{decorated}"
        )));
    };
    if undecorated != expected_undecorated {
        return Err(ExtractError::Validation(format!(
            "domain53_rtti_name_mismatch:{undecorated}!={expected_undecorated}"
        )));
    }
    Ok(())
}

fn rtti_type_name_for_vtable<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    vtable: u64,
) -> Result<String, ExtractError> {
    if !in_module(module_base, vtable) {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_vtable_out_of_module",
        ));
    }
    let col_slot = vtable
        .checked_sub(8)
        .ok_or(ExtractError::InvalidRoot("domain53_rtti_vtable_underflow"))?;
    let col = mem.read_u64(col_slot)?;
    if !in_module(module_base, col) {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_out_of_module"));
    }
    let col_bytes = mem.read_bytes(col, 24)?;
    let signature = read_u32_from(&col_bytes, 0);
    let type_descriptor_rva = read_u32_from(&col_bytes, 0x0C);
    let class_descriptor_rva = read_u32_from(&col_bytes, 0x10);
    let self_rva = read_u32_from(&col_bytes, 0x14);
    if signature > 1 {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_signature"));
    }
    if signature == 1 && module_base.saturating_add(u64::from(self_rva)) != col {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_self_mismatch"));
    }
    if type_descriptor_rva == 0 || class_descriptor_rva == 0 {
        return Err(ExtractError::InvalidRoot("domain53_rtti_col_missing_refs"));
    }
    let type_descriptor = module_base
        .checked_add(u64::from(type_descriptor_rva))
        .ok_or(ExtractError::InvalidRoot(
            "domain53_rtti_type_descriptor_overflow",
        ))?;
    if !in_module(module_base, type_descriptor) {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_type_descriptor_out_of_module",
        ));
    }
    read_cstr(
        mem,
        add_addr(type_descriptor, 0x10)?,
        DOMAIN53_RTTI_MAX_NAME_LEN,
    )
}

fn read_u32_from(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("u32 byte count"),
    )
}

fn read_cstr<M: Domain53Memory>(
    mem: &M,
    address: u64,
    max_len: usize,
) -> Result<String, ExtractError> {
    let bytes = mem.read_bytes(address, max_len)?;
    let Some(end) = bytes.iter().position(|byte| *byte == 0) else {
        return Err(ExtractError::InvalidRoot(
            "domain53_rtti_name_not_terminated",
        ));
    };
    let value = std::str::from_utf8(&bytes[..end])
        .map_err(|_| ExtractError::InvalidRoot("domain53_rtti_name_invalid_utf8"))?;
    if !value.starts_with(".?A") || value.chars().any(char::is_control) {
        return Err(ExtractError::InvalidRoot("domain53_rtti_name_invalid"));
    }
    Ok(value.to_string())
}

fn undecorate_msvc_class_name(name: &str) -> Option<&str> {
    name.strip_prefix(".?AV")?.strip_suffix("@@")
}

#[derive(Clone, Debug, Default)]
struct FactionGroupGate {
    validated: bool,
    groups_by_country: BTreeMap<u32, i32>,
}

fn read_faction_groups<M: Domain53Memory>(
    mem: &M,
    countries: &[CountryRoot],
) -> Result<FactionGroupGate, ExtractError> {
    let mut groups_by_country = BTreeMap::new();
    let mut group_members = BTreeMap::<i32, Vec<u32>>::new();
    let max_group = profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS as i32;

    for country in countries {
        let value = match mem.read_i32(add_addr(
            country.country_ptr,
            profile::DOMAIN53_COUNTRY_FACTION_GROUP,
        )?) {
            Ok(value) if (0..=max_group).contains(&value) => value,
            _ => {
                return Ok(FactionGroupGate::default());
            }
        };
        groups_by_country.insert(country.tag_db_id, value);
        group_members
            .entry(value)
            .or_default()
            .push(country.tag_db_id);
    }

    let read_count = groups_by_country.len();
    let distinct_groups = group_members.len();
    let nonzero_groups = group_members.keys().filter(|group| **group > 0).count();
    let multi_member_nonzero_groups = group_members
        .iter()
        .filter(|(group, members)| **group > 0 && members.len() > 1)
        .count();

    let validated = read_count >= DOMAIN53_FACTION_PROBE_COUNTRY_FLOOR
        && read_count == countries.len()
        && distinct_groups > 0
        && nonzero_groups > 0
        && multi_member_nonzero_groups > 0;

    if validated {
        Ok(FactionGroupGate {
            validated: true,
            groups_by_country,
        })
    } else {
        Ok(FactionGroupGate::default())
    }
}

fn resolve_country_roots<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
) -> Result<Vec<CountryRoot>, ExtractError> {
    let country_table = mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE,
    )?)?;
    let country_count = mem.read_u32(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT,
    )?)?;
    if country_count == 0 || country_count > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS {
        return Err(ExtractError::InvalidCount {
            name: "domain53_country_count",
            value: country_count,
        });
    }
    if !is_heap_ptr(country_table) {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_table_unavailable",
        ));
    }

    let expected_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let mut roots = Vec::new();
    let mut seen = BTreeSet::new();
    for slot in 0..country_count {
        let slot_addr = country_table
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("domain53_country_table_overflow"))?;
        let Ok(country_ptr) = mem.read_u64(slot_addr) else {
            continue;
        };
        if country_ptr == 0 {
            continue;
        }
        if !is_heap_ptr(country_ptr) {
            continue;
        }
        let Ok(vtable) = mem.read_u64(country_ptr) else {
            continue;
        };
        if vtable != expected_vtable {
            continue;
        }
        let Ok(tag_db_id) = mem.read_u32(add_addr(country_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        else {
            continue;
        };
        if !valid_tag_db_id(tag_db_id) || !seen.insert(tag_db_id) {
            continue;
        }
        roots.push(CountryRoot {
            tag_db_id,
            country_ptr,
        });
    }
    Ok(roots)
}

fn read_relation_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let relation_vtables = relation_vtable_kinds(module_base)?;
    let mut seen_opinions = BTreeSet::new();
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();

    for src in countries {
        let manager_addr = add_addr(src.country_ptr, profile::DOMAIN53_RELATION_MANAGER)?;
        let manager = match mem.read_u64(manager_addr) {
            Ok(value) if is_heap_ptr(value) => value,
            Ok(0) => continue,
            _ => {
                acc.skip();
                continue;
            }
        };
        let relation_table =
            match mem.read_u64(add_addr(manager, profile::DOMAIN53_RELATION_TABLE)?) {
                Ok(value) if is_heap_ptr(value) => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        for target in countries {
            if src.tag_db_id == target.tag_db_id {
                continue;
            }
            let entry_slot = relation_table
                .checked_add(u64::from(target.tag_db_id) * 8)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_relation_table_overflow",
                ))?;
            let relation_entry = match mem.read_u64(entry_slot) {
                Ok(value) if is_heap_ptr(value) => value,
                Ok(0) => continue,
                _ => {
                    acc.skip();
                    continue;
                }
            };
            match mem.read_i32(add_addr(
                relation_entry,
                profile::DOMAIN53_RELATION_OPINION,
            )?) {
                Ok(opinion) if (-MAX_OPINION_ABS..=MAX_OPINION_ABS).contains(&opinion) => {
                    if seen_opinions.insert((src.tag_db_id, target.tag_db_id)) {
                        acc.rows.opinions.push(OpinionRow {
                            src_tag_db_id: src.tag_db_id,
                            target_tag_db_id: target.tag_db_id,
                            opinion,
                        });
                        acc.rows.state_events.push(state_event(
                            src.tag_db_id,
                            Some(target.tag_db_id),
                            STATE_EVENT_SOURCE_RELATION_OPINION,
                            "relation_opinion",
                            &format!("opinion_{opinion}"),
                            i64::from(opinion),
                        ));
                        acc.observe();
                    }
                }
                Ok(_) | Err(_) => {
                    acc.skip();
                    continue;
                }
            }

            let scan_bytes = profile::DOMAIN53_RELATION_SCAN_BYTES;
            let bytes = match mem.read_bytes(relation_entry, scan_bytes) {
                Ok(bytes) => bytes,
                Err(_) => {
                    acc.skip();
                    continue;
                }
            };
            let mut offset = 0usize;
            while offset + 8 <= bytes.len() {
                let qword = u64::from_le_bytes(
                    bytes[offset..offset + 8]
                        .try_into()
                        .expect("qword scan slice"),
                );
                if let Some(flag_kind) = relation_vtables.get(&qword).copied() {
                    let object_addr = relation_entry.checked_add(offset as u64).ok_or(
                        ExtractError::InvalidRoot("domain53_relation_object_overflow"),
                    )?;
                    if flag_kind == REL_KIND_PUPPET {
                        read_subject_relation(mem, object_addr, acc, &mut seen_subjects);
                    } else {
                        let key = (src.tag_db_id, target.tag_db_id, flag_kind);
                        if seen_flags.insert(key) {
                            acc.rows.relation_flags.push(RelationFlagRow {
                                src_tag_db_id: src.tag_db_id,
                                target_tag_db_id: target.tag_db_id,
                                flag_kind,
                                directed: true,
                            });
                            acc.rows.state_events.push(state_event(
                                src.tag_db_id,
                                Some(target.tag_db_id),
                                STATE_EVENT_SOURCE_RELATION_FLAG,
                                &format!("relation_flag_kind_{flag_kind}"),
                                "directed_true",
                                i64::from(flag_kind),
                            ));
                            acc.observe();
                        }
                    }
                }
                offset += 8;
            }
        }
    }
    Ok(())
}

fn read_subject_relation<M: Domain53Memory>(
    mem: &M,
    object_addr: u64,
    acc: &mut RowAccumulator,
    seen_subjects: &mut BTreeSet<(u32, u32)>,
) {
    let read = (|| -> Result<SubjectRow, ExtractError> {
        let overlord = mem.read_u32(add_addr(object_addr, profile::DOMAIN53_SUBJECT_OVERLORD)?)?;
        let subject = mem.read_u32(add_addr(object_addr, profile::DOMAIN53_SUBJECT_SUBJECT)?)?;
        if !valid_tag_db_id(overlord) || !valid_tag_db_id(subject) || overlord == subject {
            return Err(ExtractError::InvalidRoot("domain53_subject_tag_invalid"));
        }
        let state_db = mem.read_u64(add_addr(object_addr, profile::DOMAIN53_SUBJECT_STATE_DB)?)?;
        let state_hash = if is_heap_ptr(state_db) {
            match read_msvc_string(
                mem,
                add_addr(state_db, profile::DOMAIN53_SUBJECT_STATE_NAME)?,
                96,
            ) {
                StringRead::Value(value) => crate::compact::stable_hash32(&value),
                StringRead::Invalid => {
                    acc.skip();
                    crate::compact::stable_hash32("autonomy_state_invalid_utf8")
                }
                StringRead::Missing => crate::compact::stable_hash32("autonomy_state_unknown"),
            }
        } else {
            crate::compact::stable_hash32("autonomy_state_unknown")
        };
        let ratio = mem.read_i32(add_addr(object_addr, profile::DOMAIN53_SUBJECT_RATIO)?)?;
        if !(-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&ratio) {
            return Err(ExtractError::InvalidRoot("domain53_subject_ratio_invalid"));
        }
        Ok(SubjectRow {
            overlord_tag_db_id: overlord,
            subject_tag_db_id: subject,
            autonomy_state_hash: state_hash,
            autonomy_ratio_raw: ratio,
        })
    })();

    match read {
        Ok(row) => {
            if seen_subjects.insert((row.overlord_tag_db_id, row.subject_tag_db_id)) {
                acc.rows.state_events.push(StateEventRow {
                    owner_tag_db_id: row.overlord_tag_db_id,
                    target_tag_db_id: Some(row.subject_tag_db_id),
                    event_kind: STATE_EVENT_KIND_OBSERVED,
                    source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
                    object_hash: row.autonomy_state_hash,
                    value_hash: crate::compact::stable_hash32(&format!(
                        "autonomy_ratio_{}",
                        row.autonomy_ratio_raw
                    )),
                    value_raw: i64::from(row.autonomy_ratio_raw),
                });
                acc.rows.subjects.push(row);
                acc.observe();
            }
        }
        Err(_) => acc.skip(),
    }
}

fn read_agency_rows<M: Domain53Memory>(
    mem: &M,
    _module_base: u64,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<Vec<AgencyRootRead>, ExtractError> {
    let expected_vtable = add_addr(mem.module_base(), profile::DOMAIN53_AGENCY_VTABLE_RVA)?;
    let mut agency_roots = Vec::new();
    for country in countries {
        let agency_ptr = match mem.read_u64(add_addr(
            country.country_ptr,
            profile::DOMAIN53_COUNTRY_AGENCY_PTR,
        )?) {
            Ok(0) => continue,
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) | Err(_) => {
                acc.skip();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(agency_ptr) else {
            acc.skip();
            continue;
        };
        if vtable != expected_vtable {
            acc.skip();
            continue;
        }
        let defense_raw =
            match mem.read_i32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_DEFENSE)?) {
                Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        let upgrade_progress_raw = match mem.read_i32(add_addr(
            agency_ptr,
            profile::DOMAIN53_AGENCY_UPGRADE_PROGRESS,
        )?) {
            Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let name_hash = match read_msvc_string(
            mem,
            add_addr(agency_ptr, profile::DOMAIN53_AGENCY_NAME)?,
            128,
        ) {
            StringRead::Value(value) => crate::compact::stable_hash64(&value),
            StringRead::Invalid => {
                acc.skip();
                0
            }
            StringRead::Missing => 0,
        };
        let icon_hash = match read_msvc_string(
            mem,
            add_addr(agency_ptr, profile::DOMAIN53_AGENCY_ICON)?,
            128,
        ) {
            StringRead::Value(value) => crate::compact::stable_hash64(&value),
            StringRead::Invalid => {
                acc.skip();
                0
            }
            StringRead::Missing => 0,
        };
        // Legacy wire fields carry raw capacity candidates only. 870ebf proof
        // rejects the old E4=max/F0=usable assignment; the authoritative raw
        // row below preserves E4/F0/F4 without semantic labels.
        let max_operatives =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_E4)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        let usable_slots =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F0)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip();
                    continue;
                }
            };
        acc.rows.agencies.push(AgencyRow {
            owner_tag_db_id: country.tag_db_id,
            max_operatives,
            usable_slots,
            operative_count: 0,
            cryptology_target_count: 0,
            defense_raw,
            upgrade_progress_raw,
            name_hash,
            icon_hash,
        });
        acc.rows.state_events.push(state_event(
            country.tag_db_id,
            None,
            STATE_EVENT_SOURCE_AGENCY_SCALAR,
            "agency_upgrade_progress_raw",
            &format!("agency_upgrade_progress_{upgrade_progress_raw}"),
            i64::from(upgrade_progress_raw),
        ));
        acc.observe();

        read_agency_capacity(mem, country.tag_db_id, agency_ptr, acc)?;
        agency_roots.push(AgencyRootRead {
            owner_tag_db_id: country.tag_db_id,
            agency_ptr,
        });
    }
    Ok(agency_roots)
}

fn read_agency_capacity<M: Domain53Memory>(
    mem: &M,
    owner_tag_db_id: u32,
    agency_ptr: u64,
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let value_0xe4 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_E4)?)?;
    let value_0xf0 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F0)?)?;
    let value_0xf4 = mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F4)?)?;
    if value_0xe4 != value_0xf4
        || value_0xe4 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
        || value_0xf0 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
        || value_0xf4 > MAX_AGENCY_CAPACITY_CLUSTER_VALUE
    {
        acc.skip();
        return Ok(());
    }
    let proof_hash = crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4");
    acc.rows
        .agency_capacity_clusters
        .push(AgencyCapacityClusterRow {
            owner_tag_db_id,
            value_0xe4,
            value_0xf0,
            value_0xf4,
            proof_hash,
        });
    acc.rows.state_events.push(state_event(
        owner_tag_db_id,
        None,
        STATE_EVENT_SOURCE_AGENCY_CAPACITY,
        "agency_capacity_offsets_e4_f0_f4",
        &format!("agency_capacity_{value_0xe4}_{value_0xf0}_{value_0xf4}"),
        i64::from(value_0xf0),
    ));
    acc.observe();
    Ok(())
}

fn read_cryptology_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    agency_roots: &[AgencyRootRead],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let crypto_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_VTABLE_RVA)?;
    let record_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA)?;
    let valid_owners = agency_roots
        .iter()
        .map(|root| root.owner_tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for agency in agency_roots {
        let crypto_ptr = match mem.read_u64(add_addr(
            agency.agency_ptr,
            profile::DOMAIN53_AGENCY_CRYPTO_MGR,
        )?) {
            Ok(0) => continue,
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) | Err(_) => {
                acc.skip();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(crypto_ptr) else {
            acc.skip();
            continue;
        };
        if vtable != crypto_vtable {
            acc.skip();
            continue;
        }
        let data = match mem.read_u64(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_DATA)?)
        {
            Ok(value) if value == 0 || is_heap_ptr(value) => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let size = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_SIZE)?)
        {
            Ok(value) if value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let cap = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_CAP)?) {
            Ok(value) if value >= size && value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip();
                continue;
            }
        };
        let _ = cap;
        if size == 0 {
            continue;
        }
        if !is_heap_ptr(data) {
            acc.skip();
            continue;
        }
        for index in 0..size {
            let row_addr = data
                .checked_add(u64::from(index) * profile::DOMAIN53_CRYPTOLOGY_RECORD_STRIDE as u64)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_cryptology_record_overflow",
                ))?;
            let read = (|| -> Result<CryptologyRow, ExtractError> {
                if mem.read_u64(row_addr)? != record_vtable {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_vtable_mismatch",
                    ));
                }
                let target = mem.read_u32(add_addr(
                    row_addr,
                    profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET,
                )?)?;
                if !valid_tag_db_id(target) || !valid_owners.contains(&agency.owner_tag_db_id) {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_target_invalid",
                    ));
                }
                let decrypting = mem
                    .read_u8(add_addr(
                        row_addr,
                        profile::DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING,
                    )?)
                    .map(|value| value != 0)?;
                let amount_raw = mem.read_i64(add_addr(
                    row_addr,
                    profile::DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT,
                )?)?;
                if !(0..=MAX_CRYPTO_AMOUNT_RAW).contains(&amount_raw) {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_cryptology_amount_invalid",
                    ));
                }
                Ok(CryptologyRow {
                    owner_tag_db_id: agency.owner_tag_db_id,
                    target_tag_db_id: target,
                    amount_raw,
                    decrypting,
                })
            })();
            match read {
                Ok(row) if seen.insert((row.owner_tag_db_id, row.target_tag_db_id)) => {
                    acc.rows.state_events.push(state_event(
                        row.owner_tag_db_id,
                        Some(row.target_tag_db_id),
                        STATE_EVENT_SOURCE_CRYPTOLOGY,
                        "cryptology_amount_raw",
                        &format!("cryptology_amount_{}", row.amount_raw),
                        row.amount_raw,
                    ));
                    acc.rows.cryptology.push(row);
                    acc.observe();
                }
                Ok(_) => {}
                Err(_) => acc.skip(),
            }
        }
    }
    Ok(())
}

fn read_operatives<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<u32, ExtractError> {
    let valid_owners = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    let ranges = mem.readable_ranges(
        profile::DOMAIN53_HEAP_LO as u64,
        profile::DOMAIN53_HEAP_HI as u64,
    );
    let hits = scan_qword_hits(mem, &ranges, operative_vtable)?;
    let mut seen = BTreeSet::new();
    for hit in hits {
        let identity = hit
            .checked_add(profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64)
            .ok_or(ExtractError::InvalidRoot(
                "domain53_operative_identity_overflow",
            ))?;
        let read = read_operative_row(mem, identity, &valid_owners);
        match read {
            Ok(row) if seen.insert(row.op_id) => {
                if let Some(lifecycle) = row.lifecycle_flag {
                    acc.rows.state_events.push(state_event(
                        row.owner_tag_db_id,
                        None,
                        STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
                        &format!("operative_{}_lifecycle", row.op_id),
                        &format!("lifeflag_{lifecycle}"),
                        i64::from(lifecycle),
                    ));
                }
                acc.rows.operatives.push(row);
                acc.observe();
            }
            Ok(_) => {}
            Err(_) => acc.skip(),
        }
    }
    Ok(ranges.len().min(u32::MAX as usize) as u32)
}

fn read_operative_row<M: Domain53Memory>(
    mem: &M,
    identity: u64,
    valid_owners: &BTreeSet<u32>,
) -> Result<OperativeRow, ExtractError> {
    let handle_type = mem.read_u32(identity)?;
    if handle_type != profile::DOMAIN53_OPERATIVE_HANDLE_TYPE {
        return Err(ExtractError::InvalidRoot("domain53_operative_handle_type"));
    }
    let op_id = mem.read_u32(add_addr(identity, 4)?)?;
    if op_id == 0 || op_id > MAX_OPERATIVE_ID {
        return Err(ExtractError::InvalidRoot("domain53_operative_id_invalid"));
    }
    let owner = mem.read_u32(add_addr(identity, profile::DOMAIN53_OPERATIVE_OWNER)?)?;
    if !valid_tag_db_id(owner) || !valid_owners.contains(&owner) {
        return Err(ExtractError::InvalidRoot(
            "domain53_operative_owner_invalid",
        ));
    }
    let lifecycle = mem.read_u32(add_addr(
        identity,
        profile::DOMAIN53_OPERATIVE_LIFECYCLE_FLAG,
    )?)?;
    if lifecycle > MAX_OPERATIVE_LIFECYCLE_FLAG {
        return Err(ExtractError::InvalidRoot(
            "domain53_operative_lifecycle_invalid",
        ));
    }
    let name_a = read_msvc_string(
        mem,
        add_addr(identity, profile::DOMAIN53_OPERATIVE_NAME)?,
        128,
    );
    let name_b = read_msvc_string(
        mem,
        add_addr(identity, profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE)?,
        128,
    );
    let name_hash = match (name_a, name_b) {
        (StringRead::Value(a), StringRead::Value(b)) if a == b => crate::compact::stable_hash64(&a),
        (StringRead::Value(a), StringRead::Missing)
        | (StringRead::Missing, StringRead::Value(a)) => crate::compact::stable_hash64(&a),
        (StringRead::Missing, StringRead::Missing) => 0,
        (StringRead::Invalid, _) | (_, StringRead::Invalid) => {
            return Err(ExtractError::InvalidRoot(
                "domain53_operative_name_invalid_utf8",
            ));
        }
        _ => {
            return Err(ExtractError::InvalidRoot(
                "domain53_operative_name_mismatch",
            ))
        }
    };
    let detected = mem
        .read_u8(add_addr(identity, profile::DOMAIN53_OPERATIVE_DETECTED)?)
        .map(|value| value != 0)?;
    let leader = identity
        .checked_add_signed(profile::DOMAIN53_OPERATIVE_LEADER_FROM_IDENTITY_DELTA as i64)
        .ok_or(ExtractError::InvalidRoot(
            "domain53_operative_leader_overflow",
        ))?;
    let experience_raw = match mem.read_i64(add_addr(
        leader,
        profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW,
    )?) {
        Ok(value) if (0..=MAX_XP_RAW).contains(&value) => Some(value),
        Ok(_) => return Err(ExtractError::InvalidRoot("domain53_operative_xp_invalid")),
        Err(_) => None,
    };
    let skill = match mem.read_u64(add_addr(
        leader,
        profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT,
    )?) {
        Ok(skill_obj) if is_heap_ptr(skill_obj) => {
            match mem.read_u32(add_addr(
                skill_obj,
                profile::DOMAIN53_OPERATIVE_SKILL_LEVEL,
            )?) {
                Ok(value) if value <= MAX_OPERATIVE_SKILL => Some(value),
                Ok(_) => {
                    return Err(ExtractError::InvalidRoot(
                        "domain53_operative_skill_invalid",
                    ))
                }
                Err(_) => None,
            }
        }
        _ => None,
    };
    Ok(OperativeRow {
        owner_tag_db_id: owner,
        op_id,
        detected,
        captured: false,
        skill,
        experience_raw,
        name_hash,
        state_hash: crate::compact::stable_hash32(&format!("lifeflag_{lifecycle}")),
        lifecycle_flag: Some(lifecycle),
    })
}

fn read_war_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<Vec<WarRelationRow>, ExtractError> {
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let date_vtables = [
        add_addr(module_base, profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA)?,
        add_addr(module_base, profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA)?,
    ];
    let ranges = mem.readable_ranges(
        profile::DOMAIN53_HEAP_LO as u64,
        profile::DOMAIN53_HEAP_HI as u64,
    );
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    for date_vtable in date_vtables {
        for hit in scan_qword_hits(mem, &ranges, date_vtable)? {
            let Some(row_addr) = hit.checked_sub(profile::DOMAIN53_WAR_DATE_VTABLE as u64) else {
                continue;
            };
            let read = read_war_row(mem, row_addr, &valid_tags);
            match read {
                Ok(row) => {
                    let key = if row.first_tag_db_id <= row.second_tag_db_id {
                        (row.first_tag_db_id, row.second_tag_db_id)
                    } else {
                        (row.second_tag_db_id, row.first_tag_db_id)
                    };
                    if seen.insert(key) {
                        rows.push(row);
                        acc.observe();
                    }
                }
                Err(_) => acc.skip(),
            }
        }
    }
    Ok(rows)
}

fn read_war_row<M: Domain53Memory>(
    mem: &M,
    row_addr: u64,
    valid_tags: &BTreeSet<u32>,
) -> Result<WarRelationRow, ExtractError> {
    let first = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_FIRST_TAG)?)?;
    let second = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_SECOND_TAG)?)?;
    if first == second
        || !valid_tag_db_id(first)
        || !valid_tag_db_id(second)
        || !valid_tags.contains(&first)
        || !valid_tags.contains(&second)
    {
        return Err(ExtractError::InvalidRoot("domain53_war_pair_invalid"));
    }
    let first_casualties =
        mem.read_i32(add_addr(row_addr, profile::DOMAIN53_WAR_FIRST_CASUALTIES)?)? as i64;
    let second_casualties =
        mem.read_i32(add_addr(row_addr, profile::DOMAIN53_WAR_SECOND_CASUALTIES)?)? as i64;
    if !(0..=MAX_CASUALTIES).contains(&first_casualties)
        || !(0..=MAX_CASUALTIES).contains(&second_casualties)
    {
        return Err(ExtractError::InvalidRoot("domain53_war_casualties_invalid"));
    }
    let echo_first = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_ECHO_FIRST)?)?;
    let echo_second = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_ECHO_SECOND)?)?;
    if echo_first != first || echo_second != second {
        return Err(ExtractError::InvalidRoot("domain53_war_echo_mismatch"));
    }
    Ok(WarRelationRow {
        first_tag_db_id: first,
        second_tag_db_id: second,
        first_casualties,
        second_casualties,
    })
}

fn war_projection_by_country(rows: &[WarRelationRow]) -> BTreeMap<u32, (u32, i64)> {
    let mut out = BTreeMap::<u32, (u32, i64)>::new();
    for row in rows {
        let first = out.entry(row.first_tag_db_id).or_default();
        first.0 = first.0.saturating_add(1);
        first.1 = first.1.saturating_add(row.first_casualties);
        let second = out.entry(row.second_tag_db_id).or_default();
        second.0 = second.0.saturating_add(1);
        second.1 = second.1.saturating_add(row.second_casualties);
    }
    out
}

fn relation_vtable_kinds(module_base: u64) -> Result<BTreeMap<u64, u16>, ExtractError> {
    Ok(BTreeMap::from([
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA,
            )?,
            REL_KIND_NON_AGGRESSION_PACT,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA)?,
            REL_KIND_PUPPET,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_EMBARGO_VTABLE_RVA)?,
            REL_KIND_EMBARGO,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA,
            )?,
            REL_KIND_MILITARY_ACCESS,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA,
            )?,
            REL_KIND_AIR_BASE_ACCESS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_DOCKING_RIGHTS_VTABLE_RVA)?,
            REL_KIND_DOCKING_RIGHTS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_NAVAL_BLOCKADE_VTABLE_RVA)?,
            REL_KIND_NAVAL_BLOCKADE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_MARKET_ACCESS_RIGHTS_VTABLE_RVA,
            )?,
            REL_KIND_MARKET_ACCESS_RIGHTS,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_EQUIPMENT_PURCHASE_RELATION_VTABLE_RVA,
            )?,
            REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA)?,
            REL_KIND_GUARANTEE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_IMPROVE_RELATIONS_VTABLE_RVA,
            )?,
            REL_KIND_IMPROVE_RELATIONS,
        ),
        (
            add_addr(module_base, profile::DOMAIN53_REL_ATTACHE_VTABLE_RVA)?,
            REL_KIND_ATTACHE,
        ),
        (
            add_addr(
                module_base,
                profile::DOMAIN53_REL_LICENCE_PRODUCTION_VTABLE_RVA,
            )?,
            REL_KIND_LICENCE_PRODUCTION,
        ),
    ]))
}

fn scan_qword_hits<M: Domain53Memory>(
    mem: &M,
    ranges: &[(u64, usize)],
    needle: u64,
) -> Result<Vec<u64>, ExtractError> {
    let needle_bytes = needle.to_le_bytes();
    let mut hits = Vec::new();
    const CHUNK: usize = 1 << 20;
    const OVERLAP: usize = 16;
    for (start, len) in ranges {
        let Some(end) = start.checked_add(*len as u64) else {
            continue;
        };
        let mut pos = *start;
        while pos < end {
            let remaining = (end - pos).min(usize::MAX as u64) as usize;
            let want = remaining.min(CHUNK);
            let blob = mem.read_bytes(pos, want)?;
            let mut offset = (8usize.wrapping_sub((pos as usize) % 8)) % 8;
            while offset + 8 <= blob.len() {
                if blob[offset..offset + 8] == needle_bytes {
                    hits.push(pos + offset as u64);
                }
                offset += 8;
            }
            if blob.len() < want || blob.len() >= remaining {
                break;
            }
            pos = pos
                .checked_add(blob.len().saturating_sub(OVERLAP).max(1) as u64)
                .ok_or(ExtractError::InvalidRoot("domain53_scan_overflow"))?;
        }
    }
    Ok(hits)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StringRead {
    Value(String),
    Missing,
    Invalid,
}

fn read_msvc_string<M: Domain53Memory>(mem: &M, address: u64, max_len: usize) -> StringRead {
    let Ok(size) = mem.read_u64(add_addr(address, 0x10).unwrap_or(address)) else {
        return StringRead::Missing;
    };
    let Ok(cap) = mem.read_u64(add_addr(address, 0x18).unwrap_or(address)) else {
        return StringRead::Missing;
    };
    let size = size as usize;
    let cap = cap as usize;
    if size == 0 {
        return StringRead::Missing;
    }
    if size > max_len || cap < size || cap > (1 << 20) {
        return StringRead::Invalid;
    }
    let chars_addr = if cap < 16 {
        address
    } else {
        match mem.read_u64(address) {
            Ok(ptr) if is_heap_ptr(ptr) => ptr,
            _ => return StringRead::Invalid,
        }
    };
    let Ok(bytes) = mem.read_bytes(chars_addr, size + 1) else {
        return StringRead::Missing;
    };
    if bytes.get(size).copied() != Some(0) {
        return StringRead::Invalid;
    }
    match std::str::from_utf8(&bytes[..size]) {
        Ok(value) if !value.chars().any(char::is_control) => StringRead::Value(value.to_string()),
        _ => StringRead::Invalid,
    }
}

fn state_event(
    owner_tag_db_id: u32,
    target_tag_db_id: Option<u32>,
    source_kind: u16,
    object: &str,
    value: &str,
    value_raw: i64,
) -> StateEventRow {
    StateEventRow {
        owner_tag_db_id,
        target_tag_db_id,
        event_kind: STATE_EVENT_KIND_OBSERVED,
        source_kind,
        object_hash: crate::compact::stable_hash32(object),
        value_hash: crate::compact::stable_hash32(value),
        value_raw,
    }
}

fn add_addr(base: u64, offset: usize) -> Result<u64, ExtractError> {
    base.checked_add(offset as u64)
        .ok_or(ExtractError::InvalidRoot("domain53_address_overflow"))
}

fn is_heap_ptr(value: u64) -> bool {
    value >= profile::DOMAIN53_HEAP_LO as u64 && value < profile::DOMAIN53_HEAP_HI as u64
}

fn in_module(module_base: u64, value: u64) -> bool {
    (module_base..module_base.saturating_add(DOMAIN53_MODULE_SPAN_MAX)).contains(&value)
}

fn valid_tag_db_id(value: u32) -> bool {
    value > 0 && value <= MAX_TAG_DB_ID
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain53ExtractionReport {
    pub domain_id: u16,
    pub source_proofs: u64,
    pub snapshot: DiplomacyIntelSnapshot,
    pub payload: Vec<u8>,
    pub gate_summary: Domain53GateSummary,
    pub errors: Vec<String>,
}

impl Domain53ExtractionReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn build_domain53_payload(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
) -> Domain53ExtractionReport {
    let mut snapshot = DiplomacyIntelSnapshot {
        elapsed_wall_ms: rows.elapsed_wall_ms,
        status_flags: 0,
        changed_rows: rows.changed_rows,
        countries: rows.countries,
        wars: rows.wars,
        opinions: rows.opinions,
        relation_flags: rows.relation_flags,
        subjects: rows.subjects,
        agencies: rows.agencies,
        agency_capacity_clusters: rows.agency_capacity_clusters,
        operatives: rows.operatives,
        cryptology: rows.cryptology,
        operation_anchors: rows.operation_anchors,
        intel_map: rows.intel_map,
        state_events: rows.state_events,
    };
    let mut errors = Vec::new();

    let unknown_source_bits = source_proofs & !KNOWN_SOURCE_PROOFS;
    if unknown_source_bits != 0 {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} source_proofs has unknown bits 0x{unknown_source_bits:X}"
        ));
    }
    if snapshot.row_count() != 0 && !has(source_proofs, SOURCE_PROOF_BUILD_SHA_870EBF) {
        errors.push(format!(
            "domain{DOMAIN_ID_PROPOSED} rows require SOURCE_PROOF_BUILD_SHA_870EBF for {EXPECTED_HOI4_SHA256_870EBF}"
        ));
    }

    require_rows(
        !snapshot.countries.is_empty(),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "country diplomacy rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.countries.is_empty(),
        source_proofs,
        SOURCE_PROOF_FACTION_GROUP_870EBF,
        "country diplomacy rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.opinions.is_empty()
            || !snapshot.relation_flags.is_empty()
            || !snapshot.subjects.is_empty(),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "opinion/relation/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.agencies.is_empty() || !snapshot.agency_capacity_clusters.is_empty(),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "agency rows/capacity clusters require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.operatives.is_empty(),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "operative rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.cryptology.is_empty(),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_RELATION_FLAG
                    | STATE_EVENT_SOURCE_RELATION_OPINION
                    | STATE_EVENT_SOURCE_SUBJECT_AUTONOMY
            )
        }),
        source_proofs,
        SOURCE_PROOF_RELATION_ROOTS_870EBF,
        "compact state events sourced from relation/opinion/subject rows require SOURCE_PROOF_RELATION_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
        source_proofs,
        SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        "compact state events sourced from country faction rows require SOURCE_PROOF_COUNTRY_ROOTS_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
        source_proofs,
        SOURCE_PROOF_FACTION_GROUP_870EBF,
        "compact state events sourced from country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot.state_events.iter().any(|row| {
            matches!(
                row.source_kind,
                STATE_EVENT_SOURCE_AGENCY_SCALAR | STATE_EVENT_SOURCE_AGENCY_CAPACITY
            )
        }),
        source_proofs,
        SOURCE_PROOF_AGENCY_ROOT_870EBF,
        "compact state events sourced from agency rows require SOURCE_PROOF_AGENCY_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE),
        source_proofs,
        SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        "compact state events sourced from operative lifecycle rows require SOURCE_PROOF_OPERATIVE_ROOT_870EBF",
        &mut errors,
    );
    require_rows(
        snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_CRYPTOLOGY),
        source_proofs,
        SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF,
        "compact state events sourced from cryptology rows require SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF",
        &mut errors,
    );

    let has_country_war_projection = snapshot
        .countries
        .iter()
        .any(|row| row.at_war_count != 0 || row.casualties != 0);
    if has(source_proofs, SOURCE_PROOF_WAR_ROOT_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_ROOT_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_ROOT_UNVALIDATED;
        if !snapshot.wars.is_empty() {
            errors.push("war rows require SOURCE_PROOF_WAR_ROOT_870EBF".to_string());
        }
        if has_country_war_projection {
            errors.push(
                "country at-war/casualty projections require SOURCE_PROOF_WAR_ROOT_870EBF"
                    .to_string(),
            );
        }
    }
    if has(source_proofs, SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF) {
        snapshot.status_flags &= !STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED;
    }

    let has_faction_rows = !snapshot.countries.is_empty()
        || snapshot
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);
    if has(source_proofs, SOURCE_PROOF_FACTION_GROUP_870EBF) {
        snapshot.status_flags &= !STATUS_FACTION_GROUP_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_FACTION_GROUP_UNVALIDATED;
        if has_faction_rows {
            errors
                .push("country faction rows require SOURCE_PROOF_FACTION_GROUP_870EBF".to_string());
        }
    }

    if has(source_proofs, SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF) {
        snapshot.status_flags &= !STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED;
    }

    let has_resisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| row.detail_resisted);
    let has_unresisted_operations = snapshot
        .operation_anchors
        .iter()
        .any(|row| !row.detail_resisted);
    if has_resisted_operations || !has(source_proofs, SOURCE_PROOF_OPERATION_DETAIL_870EBF) {
        snapshot.status_flags |= STATUS_OPERATIONS_DETAIL_RESISTED;
    }
    require_rows(
        has_unresisted_operations,
        source_proofs,
        SOURCE_PROOF_OPERATION_DETAIL_870EBF,
        "unresisted operation rows require SOURCE_PROOF_OPERATION_DETAIL_870EBF",
        &mut errors,
    );

    let has_resisted_intel_map = snapshot.intel_map.iter().any(|row| row.resisted);
    let has_unresisted_intel_map = snapshot.intel_map.iter().any(|row| !row.resisted);
    if has_resisted_intel_map || !has(source_proofs, SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF) {
        snapshot.status_flags |= STATUS_INTEL_MAP_RESISTED;
    }
    require_rows(
        has_unresisted_intel_map,
        source_proofs,
        SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
        "unresisted intel-map rows require SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF",
        &mut errors,
    );

    let payload = encode_payload(&snapshot);
    let gate_summary = decode_payload(0, &payload);
    errors.extend(gate_summary.parse_errors.iter().cloned());

    Domain53ExtractionReport {
        domain_id: DOMAIN_ID_PROPOSED,
        source_proofs,
        snapshot,
        payload,
        gate_summary,
        errors,
    }
}

pub fn proof_fixture_rows() -> ExtractedDomain53Rows {
    ExtractedDomain53Rows {
        elapsed_wall_ms: 3_600_000,
        changed_rows: None,
        countries: vec![CountryDiplomacyRow {
            tag_db_id: 1,
            faction_group: 42,
            at_war_count: 0,
            casualties: 0,
        }],
        wars: Vec::new(),
        opinions: vec![OpinionRow {
            src_tag_db_id: 1,
            target_tag_db_id: 3,
            opinion: -35,
        }],
        relation_flags: vec![RelationFlagRow {
            src_tag_db_id: 1,
            target_tag_db_id: 3,
            flag_kind: REL_KIND_GUARANTEE,
            directed: true,
        }],
        subjects: vec![SubjectRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            autonomy_state_hash: crate::compact::stable_hash32("autonomy_puppet"),
            autonomy_ratio_raw: 40_000,
        }],
        agencies: vec![AgencyRow {
            owner_tag_db_id: 1,
            max_operatives: 3,
            usable_slots: 3,
            operative_count: 3,
            cryptology_target_count: 1,
            defense_raw: 600_000,
            upgrade_progress_raw: 2_400_000,
            name_hash: crate::compact::stable_hash64("Abwehr"),
            icon_hash: crate::compact::stable_hash64("GFX_intelligence_agency_logo_ger"),
        }],
        agency_capacity_clusters: vec![AgencyCapacityClusterRow {
            owner_tag_db_id: 2,
            value_0xe4: 9,
            value_0xf0: 10,
            value_0xf4: 9,
            proof_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
        }],
        operatives: vec![
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_387,
                detected: false,
                captured: false,
                skill: Some(2),
                experience_raw: Some(71_372_000),
                name_hash: crate::compact::stable_hash64("Johann Hein"),
                state_hash: crate::compact::stable_hash32("lifeflag_47"),
                lifecycle_flag: Some(47),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_388,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(12_400_000),
                name_hash: crate::compact::stable_hash64("Pool Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_31"),
                lifecycle_flag: Some(31),
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_389,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(0),
                name_hash: crate::compact::stable_hash64("Lifecycle Cohort"),
                state_hash: crate::compact::stable_hash32("lifeflag_63"),
                lifecycle_flag: Some(63),
            },
        ],
        cryptology: vec![CryptologyRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            amount_raw: 960_000_000,
            decrypting: true,
        }],
        operation_anchors: Vec::new(),
        intel_map: Vec::new(),
        state_events: validated_state_event_rows(),
    }
}

fn validated_state_event_rows() -> Vec<StateEventRow> {
    vec![
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_FLAG,
            object_hash: crate::compact::stable_hash32("relation_flag_guarantee"),
            value_hash: crate::compact::stable_hash32("directed_true"),
            value_raw: REL_KIND_GUARANTEE as i64,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(22),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_SUBJECT_AUTONOMY,
            object_hash: crate::compact::stable_hash32("autonomy_puppet"),
            value_hash: crate::compact::stable_hash32("autonomy_ratio_40000"),
            value_raw: 40_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_RELATION_OPINION,
            object_hash: crate::compact::stable_hash32("relation_opinion"),
            value_hash: crate::compact::stable_hash32("opinion_neg35"),
            value_raw: -35,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
            object_hash: crate::compact::stable_hash32("country_faction_group"),
            value_hash: crate::compact::stable_hash32("faction_group_42"),
            value_raw: 42,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_SCALAR,
            object_hash: crate::compact::stable_hash32("agency_upgrade_progress_raw"),
            value_hash: crate::compact::stable_hash32("agency_upgrade_progress_2400000"),
            value_raw: 2_400_000,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
            object_hash: crate::compact::stable_hash32("operative_11387_lifecycle"),
            value_hash: crate::compact::stable_hash32("lifeflag_47"),
            value_raw: 47,
        },
        StateEventRow {
            owner_tag_db_id: 2,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_AGENCY_CAPACITY,
            object_hash: crate::compact::stable_hash32("agency_capacity_offsets_e4_f0_f4"),
            value_hash: crate::compact::stable_hash32("agency_capacity_9_10_9"),
            value_raw: 10,
        },
        StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: Some(3),
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_CRYPTOLOGY,
            object_hash: crate::compact::stable_hash32("cryptology_amount_raw"),
            value_hash: crate::compact::stable_hash32("cryptology_amount_960000000"),
            value_raw: 960_000_000,
        },
    ]
}

fn require_rows(
    present: bool,
    source_proofs: u64,
    required: u64,
    message: &str,
    errors: &mut Vec<String>,
) {
    if present && !has(source_proofs, required) {
        errors.push(message.to_string());
    }
}

fn has(source_proofs: u64, required: u64) -> bool {
    source_proofs & required != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_COUNTRY_ROWS: u32 = 45;
    const TEST_COUNTRY_COL_RVA: u64 = 0x3600_0000;
    const TEST_COUNTRY_TYPE_DESCRIPTOR_RVA: u64 = 0x3600_1000;
    const TEST_AGENCY_COL_RVA: u64 = 0x3600_2000;
    const TEST_AGENCY_TYPE_DESCRIPTOR_RVA: u64 = 0x3600_3000;
    const TEST_RTTI_CCOUNTRY_DECORATED: &str = ".?AVCCountry@@";
    const TEST_RTTI_CINTELLIGENCE_AGENCY_DECORATED: &str = ".?AVCIntelligenceAgency@@";

    #[derive(Clone, Debug)]
    struct FakeMemory {
        module_base: u64,
        chunks: Vec<(u64, Vec<u8>)>,
        ranges: Vec<(u64, usize)>,
    }

    impl FakeMemory {
        fn new(module_base: u64) -> Self {
            Self {
                module_base,
                chunks: Vec::new(),
                ranges: Vec::new(),
            }
        }

        fn add_chunk(&mut self, address: u64, len: usize) {
            self.chunks.push((address, vec![0; len]));
        }

        fn add_range(&mut self, address: u64, len: usize) {
            self.ranges.push((address, len));
        }

        fn chunk_mut(&mut self, address: u64, len: usize) -> &mut [u8] {
            for (base, bytes) in &mut self.chunks {
                let end = *base + bytes.len() as u64;
                if address >= *base && address + len as u64 <= end {
                    let offset = (address - *base) as usize;
                    return &mut bytes[offset..offset + len];
                }
            }
            panic!("missing fake memory chunk at {address:#x} len {len}");
        }

        fn write_u8(&mut self, address: u64, value: u8) {
            self.chunk_mut(address, 1)[0] = value;
        }

        fn write_u32(&mut self, address: u64, value: u32) {
            self.chunk_mut(address, 4)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_i32(&mut self, address: u64, value: i32) {
            self.chunk_mut(address, 4)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_u64(&mut self, address: u64, value: u64) {
            self.chunk_mut(address, 8)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_i64(&mut self, address: u64, value: i64) {
            self.chunk_mut(address, 8)
                .copy_from_slice(&value.to_le_bytes());
        }

        fn write_bytes(&mut self, address: u64, value: &[u8]) {
            self.chunk_mut(address, value.len()).copy_from_slice(value);
        }

        fn write_inline_string(&mut self, address: u64, value: &str) {
            let bytes = value.as_bytes();
            assert!(bytes.len() < 16);
            self.chunk_mut(address, 16).fill(0);
            self.chunk_mut(address, bytes.len()).copy_from_slice(bytes);
            self.write_u64(address + 0x10, bytes.len() as u64);
            self.write_u64(address + 0x18, 15);
        }

        fn write_rtti(&mut self, vtable_rva: usize, col_rva: u64, td_rva: u64, name: &str) {
            let module = self.module_base;
            let vtable = module + vtable_rva as u64;
            let col = module + col_rva;
            let td = module + td_rva;
            self.add_chunk(vtable - 8, 8);
            self.add_chunk(col, 24);
            self.add_chunk(td, 0x10 + DOMAIN53_RTTI_MAX_NAME_LEN);
            self.write_u64(vtable - 8, col);
            self.write_u32(col, 1);
            self.write_u32(col + 0x0C, td_rva as u32);
            self.write_u32(col + 0x10, 0x3700_0000);
            self.write_u32(col + 0x14, col_rva as u32);
            self.write_rtti_name(td_rva, name);
        }

        fn write_rtti_name(&mut self, td_rva: u64, name: &str) {
            let td = self.module_base + td_rva;
            let len = name.len() + 1;
            self.chunk_mut(td + 0x10, len).fill(0);
            self.write_bytes(td + 0x10, name.as_bytes());
        }
    }

    impl Domain53Memory for FakeMemory {
        fn module_base(&self) -> u64 {
            self.module_base
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            for (base, bytes) in &self.chunks {
                let end = *base + bytes.len() as u64;
                if address >= *base && address + len as u64 <= end {
                    let offset = (address - *base) as usize;
                    return Ok(bytes[offset..offset + len].to_vec());
                }
            }
            Err(ExtractError::MemoryRead { address, len })
        }

        fn readable_ranges(&self, _lo: u64, _hi: u64) -> Vec<(u64, usize)> {
            self.ranges.clone()
        }
    }

    fn fake_country_addr(heap: u64, tag: u32) -> u64 {
        heap + 0x20_0000 + u64::from(tag) * 0x2000
    }

    fn fake_domain53_memory(include_relations: bool, include_operative: bool) -> FakeMemory {
        let module = 0x1400_0000_0;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let game_state = heap;
        let country_database = heap + 0x1_0000;
        let country_table = heap + 0x2_0000;
        let country1 = heap + 0x3_0000;
        let country2 = heap + 0x4_0000;
        let country3 = heap + 0x5_0000;
        let rel_manager1 = heap + 0x6_0000;
        let rel_table1 = heap + 0x7_0000;
        let rel12 = heap + 0x8_0000;
        let rel13 = heap + 0x9_0000;
        let autonomy_state = heap + 0xA_0000;
        let agency1 = heap + 0xB_0000;
        let crypto1 = heap + 0xC_0000;
        let crypto_records = heap + 0xD_0000;
        let leader1 = heap + 0xE_0000;
        let skill1 = heap + 0xF_0000;
        let war1 = heap + 0x10_0000;

        let mut mem = FakeMemory::new(module);
        mem.write_rtti(
            profile::DOMAIN53_COUNTRY_VTABLE_RVA,
            TEST_COUNTRY_COL_RVA,
            TEST_COUNTRY_TYPE_DESCRIPTOR_RVA,
            TEST_RTTI_CCOUNTRY_DECORATED,
        );
        mem.write_rtti(
            profile::DOMAIN53_AGENCY_VTABLE_RVA,
            TEST_AGENCY_COL_RVA,
            TEST_AGENCY_TYPE_DESCRIPTOR_RVA,
            TEST_RTTI_CINTELLIGENCE_AGENCY_DECORATED,
        );
        for (addr, len) in [
            (module + profile::CG_STATE_DATE_OFFSET as u64, 8),
            (module + profile::COUNTRY_DATABASE_OFFSET as u64, 8),
            (game_state, 0x400),
            (country_database, 0x100),
            (country_table, (FAKE_COUNTRY_ROWS as usize + 1) * 8),
            (country1, 0x1100),
            (country2, 0x1100),
            (country3, 0x1100),
            (rel_manager1, 0x40),
            (rel_table1, (FAKE_COUNTRY_ROWS as usize + 1) * 8),
            (rel12, profile::DOMAIN53_RELATION_SCAN_BYTES),
            (rel13, profile::DOMAIN53_RELATION_SCAN_BYTES),
            (autonomy_state, 0x80),
            (agency1, 0x180),
            (crypto1, 0x80),
            (crypto_records, profile::DOMAIN53_CRYPTOLOGY_RECORD_STRIDE),
            (skill1, 0x200),
            (war1, 0x80),
        ] {
            mem.add_chunk(addr, len);
        }
        if include_operative {
            mem.add_chunk(leader1, 0xF00);
            mem.add_range(leader1, 0xF00);
        }
        for tag in 4..=FAKE_COUNTRY_ROWS {
            mem.add_chunk(fake_country_addr(heap, tag), 0x1100);
        }
        mem.add_range(war1, 0x80);

        mem.write_u64(module + profile::CG_STATE_DATE_OFFSET as u64, game_state);
        mem.write_u64(
            module + profile::COUNTRY_DATABASE_OFFSET as u64,
            country_database,
        );
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64,
            country_table,
        );
        mem.write_u32(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64,
            FAKE_COUNTRY_ROWS + 1,
        );
        for tag in 1..=FAKE_COUNTRY_ROWS {
            let country = match tag {
                1 => country1,
                2 => country2,
                3 => country3,
                _ => fake_country_addr(heap, tag),
            };
            mem.write_u64(country_table + u64::from(tag) * 8, country);
            mem.write_u64(
                country,
                module + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64,
            );
            mem.write_u32(country + profile::DOMAIN53_OBJ_TAG_DB_ID as u64, tag);
            let faction_group = match tag {
                1..=20 => 1,
                21..=35 => 2,
                _ => 3,
            };
            mem.write_i32(
                country + profile::DOMAIN53_COUNTRY_FACTION_GROUP as u64,
                faction_group,
            );
        }

        if include_relations {
            mem.write_u64(
                country1 + profile::DOMAIN53_RELATION_MANAGER as u64,
                rel_manager1,
            );
            mem.write_u64(
                rel_manager1 + profile::DOMAIN53_RELATION_TABLE as u64,
                rel_table1,
            );
            mem.write_u64(rel_table1 + 2 * 8, rel12);
            mem.write_u64(rel_table1 + 3 * 8, rel13);
            mem.write_u64(
                rel12,
                module + profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA as u64,
            );
            mem.write_u32(rel12 + profile::DOMAIN53_SUBJECT_OVERLORD as u64, 1);
            mem.write_u32(rel12 + profile::DOMAIN53_SUBJECT_SUBJECT as u64, 2);
            mem.write_u64(
                rel12 + profile::DOMAIN53_SUBJECT_STATE_DB as u64,
                autonomy_state,
            );
            mem.write_i32(rel12 + profile::DOMAIN53_SUBJECT_RATIO as u64, 40_000);
            mem.write_inline_string(
                autonomy_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
                "aut_puppet",
            );
            mem.write_i32(rel12 + profile::DOMAIN53_RELATION_OPINION as u64, 10);

            mem.write_u64(
                rel13,
                module + profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA as u64,
            );
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_OPINION as u64, -35);
        }

        mem.write_u64(
            country1 + profile::DOMAIN53_COUNTRY_AGENCY_PTR as u64,
            agency1,
        );
        mem.write_u64(agency1, module + profile::DOMAIN53_AGENCY_VTABLE_RVA as u64);
        mem.write_inline_string(agency1 + profile::DOMAIN53_AGENCY_NAME as u64, "Abwehr");
        mem.write_inline_string(agency1 + profile::DOMAIN53_AGENCY_ICON as u64, "GFX_ger");
        mem.write_i32(
            agency1 + profile::DOMAIN53_AGENCY_UPGRADE_PROGRESS as u64,
            2_400_000,
        );
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_E4 as u64, 9);
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_F0 as u64, 10);
        mem.write_u32(agency1 + profile::DOMAIN53_AGENCY_CAPACITY_F4 as u64, 9);
        mem.write_u64(
            agency1 + profile::DOMAIN53_AGENCY_CRYPTO_MGR as u64,
            crypto1,
        );
        mem.write_i32(agency1 + profile::DOMAIN53_AGENCY_DEFENSE as u64, 600_000);

        mem.write_u64(
            crypto1,
            module + profile::DOMAIN53_CRYPTOLOGY_VTABLE_RVA as u64,
        );
        mem.write_u64(
            crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_DATA as u64,
            crypto_records,
        );
        mem.write_u32(crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_SIZE as u64, 1);
        mem.write_u32(crypto1 + profile::DOMAIN53_CRYPTOLOGY_MGR_CAP as u64, 1);
        mem.write_u64(
            crypto_records,
            module + profile::DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA as u64,
        );
        mem.write_u32(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET as u64,
            3,
        );
        mem.write_u8(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_DECRYPTING as u64,
            1,
        );
        mem.write_i64(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_AMOUNT as u64,
            960_000_000,
        );

        if include_operative {
            let identity = leader1 + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
            mem.write_u64(
                leader1,
                module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            );
            mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
            mem.write_u32(identity + 4, 11_387);
            mem.write_inline_string(identity + profile::DOMAIN53_OPERATIVE_NAME as u64, "Johann");
            mem.write_inline_string(
                identity + profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE as u64,
                "Johann",
            );
            mem.write_u8(identity + profile::DOMAIN53_OPERATIVE_DETECTED as u64, 0);
            mem.write_u32(
                identity + profile::DOMAIN53_OPERATIVE_LIFECYCLE_FLAG as u64,
                47,
            );
            mem.write_u32(identity + profile::DOMAIN53_OPERATIVE_OWNER as u64, 1);
            mem.write_u64(
                leader1 + profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT as u64,
                skill1,
            );
            mem.write_i64(
                leader1 + profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW as u64,
                71_372_000,
            );
            mem.write_u32(skill1 + profile::DOMAIN53_OPERATIVE_SKILL_LEVEL as u64, 2);
        }

        mem.write_u32(war1 + profile::DOMAIN53_WAR_FIRST_TAG as u64, 1);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_SECOND_TAG as u64, 3);
        mem.write_u64(
            war1 + profile::DOMAIN53_WAR_DATE_VTABLE as u64,
            module + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64,
        );
        mem.write_i32(war1 + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64, 100);
        mem.write_i32(war1 + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64, 200);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 1);
        mem.write_u32(war1 + profile::DOMAIN53_WAR_ECHO_SECOND as u64, 3);

        mem
    }

    #[test]
    fn reader_extracts_validated_slice_from_fake_memory() {
        let mem = fake_domain53_memory(true, true);
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");

        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.rows.countries.len(), FAKE_COUNTRY_ROWS as usize);
        assert_eq!(extracted.rows.opinions.len(), 2);
        assert_eq!(extracted.rows.relation_flags.len(), 1);
        assert_eq!(extracted.rows.subjects.len(), 1);
        assert_eq!(extracted.rows.agencies.len(), 1);
        assert_eq!(extracted.rows.agency_capacity_clusters.len(), 1);
        assert_eq!(extracted.rows.cryptology.len(), 1);
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.rows.wars.len(), 1);
        assert!(extracted.rows.operation_anchors.is_empty());
        assert!(extracted.rows.intel_map.is_empty());
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_BUILD_SHA_870EBF, 0);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_GROUP_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_RELATION_ROOTS_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
        assert_eq!(
            extracted.source_proofs
                & (SOURCE_PROOF_OPERATION_DETAIL_870EBF
                    | SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
                    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
                    | SOURCE_PROOF_WAR_LIFECYCLE_DETAIL_870EBF),
            0
        );

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_FACTION_GROUP_UNVALIDATED,
            0
        );
    }

    #[test]
    fn reader_fails_closed_before_build_proof_on_wrong_country_rtti() {
        let mut mem = fake_domain53_memory(true, true);
        mem.write_rtti_name(TEST_COUNTRY_TYPE_DESCRIPTOR_RVA, ".?AVCWrongCountry@@");

        let err = extract_rows_from_current_roots(&mem).expect_err("wrong RTTI must fail closed");
        assert!(matches!(err, ExtractError::Validation(_)), "{err:?}");

        let report = build_domain53_payload(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_BUILD_SHA_870EBF,
        );
        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("SOURCE_PROOF_BUILD_SHA_870EBF")));
    }

    #[test]
    fn reader_fails_closed_before_build_proof_on_garbled_agency_rtti() {
        let mut mem = fake_domain53_memory(true, true);
        mem.write_rtti_name(TEST_AGENCY_TYPE_DESCRIPTOR_RVA, "not_rtti");

        let err = extract_rows_from_current_roots(&mem).expect_err("garbled RTTI must fail closed");
        assert!(matches!(
            err,
            ExtractError::InvalidRoot("domain53_rtti_name_invalid")
        ));
    }

    #[test]
    fn reader_omits_faction_groups_when_structural_gate_fails() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let country1 = heap + 0x3_0000;
        mem.write_i32(
            country1 + profile::DOMAIN53_COUNTRY_FACTION_GROUP as u64,
            -1,
        );

        let extracted =
            extract_rows_from_current_roots(&mem).expect("other slices remain readable");

        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_GROUP_870EBF,
            0
        );
        assert!(extracted.rows.countries.is_empty());
        assert!(!extracted
            .rows
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP));

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_FACTION_GROUP_UNVALIDATED,
            0
        );
    }

    #[test]
    fn reader_fails_closed_on_worldwide_zero_relations() {
        let mem = fake_domain53_memory(false, true);
        let err = extract_rows_from_current_roots(&mem).expect_err("relations must fail closed");
        assert_eq!(
            err,
            ExtractError::InvalidRoot("domain53_relations_empty_implausible")
        );
    }

    #[test]
    fn reader_accepts_zero_operatives_as_resolved_empty() {
        let mem = fake_domain53_memory(true, false);
        let extracted = extract_rows_from_current_roots(&mem).expect("zero operative read");

        assert!(extracted.rows.operatives.is_empty());
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
    }

    #[test]
    fn current_validated_sources_emit_badged_domain53_payload() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(report.domain_id, 53);
        assert_eq!(report.gate_summary.data_rows_total, 18);
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("agency_capacity_cluster_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("operative_lifecycle_rows")
                .copied(),
            Some(3)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(8)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_relation_opinion_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_country_faction_group_rows")
                .copied(),
            Some(1)
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_OPERATIONS_DETAIL_RESISTED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_MAP_RESISTED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("war_relation_rows")
                .copied(),
            Some(0)
        );
    }

    #[test]
    fn rejects_country_war_projection_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.countries[0].at_war_count = 1;

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("country at-war/casualty projections")));
    }

    #[test]
    fn rejects_war_rows_without_current_war_root() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_WAR_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("war rows require SOURCE_PROOF_WAR_ROOT_870EBF")));
    }

    #[test]
    fn badges_war_lifecycle_detail_until_source_proof_exists() {
        let mut rows = proof_fixture_rows();
        rows.wars.push(WarRelationRow {
            first_tag_db_id: 1,
            second_tag_db_id: 3,
            first_casualties: 100,
            second_casualties: 200,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_unresisted_operation_without_operation_detail_root() {
        let mut rows = proof_fixture_rows();
        rows.operation_anchors.push(OperationAnchorRow {
            owner_tag_db_id: 1,
            operation_id: 231_592,
            target_tag_db_id: Some(3),
            slot_count: 2,
            phase_count: 3,
            detail_resisted: false,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted operation rows require")));
    }

    #[test]
    fn rejects_unresisted_intel_map_without_percentage_root() {
        let mut rows = proof_fixture_rows();
        rows.intel_map.push(IntelMapRow {
            owner_tag_db_id: 1,
            target_tag_db_id: 3,
            civilian_raw: 10_000,
            army_raw: 0,
            navy_raw: 0,
            air_raw: 0,
            resisted: false,
        });

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("unresisted intel-map rows require")));
    }

    #[test]
    fn rejects_state_events_without_matching_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.operatives.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from operative lifecycle")));
    }

    #[test]
    fn rejects_faction_group_state_event_without_country_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.countries.clear();
        rows.state_events
            .retain(|row| row.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP);

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_COUNTRY_ROOTS_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("compact state events sourced from country faction")));
    }

    #[test]
    fn root_manifest_pins_current_870ebf_subset() {
        let evidence = CURRENT_BUILD_ROOTS_870EBF
            .iter()
            .map(|root| root.evidence)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(evidence.contains("module+0x3316E90"));
        assert!(evidence.contains("module+0x296B338"));
        assert!(evidence.contains("module+0x29A6540"));
        assert!(evidence.contains("module+0x293F790"));
        assert!(evidence.contains("module+0x2702388"));
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_WAR_ROOT_870EBF,
            0
        );
    }

    #[test]
    fn intentional_omissions_do_not_set_partial_or_source_failure_bits() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(
            report.gate_summary.status_flags_or
                & (crate::compact::STATUS_SOURCE_UNAVAILABLE | crate::compact::STATUS_PARTIAL_DATA),
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or
                & (STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED
                    | STATUS_OPERATIONS_DETAIL_RESISTED
                    | STATUS_INTEL_MAP_RESISTED
                    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED),
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn unresolved_operation_and_intel_gaps_are_flagged_without_rows() {
        let report = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(report.ok(), "{:?}", report.errors);
        assert!(report.snapshot.operation_anchors.is_empty());
        assert!(report.snapshot.intel_map.is_empty());
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_OPERATIONS_DETAIL_RESISTED,
            0
        );
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_MAP_RESISTED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("operation_anchor_rows")
                .copied(),
            Some(0)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_map_rows")
                .copied(),
            Some(0)
        );
    }
}
