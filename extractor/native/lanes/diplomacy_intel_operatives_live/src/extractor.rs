use crate::compact::{
    decode_payload, encode_payload, summarize_snapshot_instrumented, AgencyCapacityClusterRow,
    AgencyRow, CountryDiplomacyRow, CryptologyRow, DiplomacyIntelSnapshot, Domain53GateSummary,
    FactionRow, IncomingActionQueueRow, IntelMapRow, IntelNetworkCoverage, IntelNetworkGraphRow,
    IntelNetworkStateStrength, OperationAnchorRow, OperationInstanceRow, OperationInstanceStatus,
    OperationTokenMembershipRow, OperativeRow, OpinionRow, RelationFlagRow, StateEventRow,
    SubjectAutonomyStatusRow, SubjectRow, SummaryPhaseTimes, WarRelationRow, DOMAIN_ID_PROPOSED,
    FIXED_SCALE_RAW, MAX_ACTION_DATE_HOURS, MAX_AGENCY_CAPACITY_CLUSTER_VALUE, MAX_CASUALTIES,
    MAX_CRYPTO_AMOUNT_RAW, MAX_FIXED_RAW, MAX_INTEL_NETWORK_STATE_ID, MAX_OPERATION_ID,
    MAX_OPERATIVE_ID, MAX_OPERATIVE_SKILL, MAX_OPINION_ABS, MAX_TAG_DB_ID, MAX_XP_RAW,
    REL_KIND_AIR_BASE_ACCESS, REL_KIND_ATTACHE, REL_KIND_DOCKING_RIGHTS, REL_KIND_EMBARGO,
    REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION, REL_KIND_GUARANTEE, REL_KIND_IMPROVE_RELATIONS,
    REL_KIND_LICENCE_PRODUCTION, REL_KIND_MARKET_ACCESS_RIGHTS, REL_KIND_MILITARY_ACCESS,
    REL_KIND_NAVAL_BLOCKADE, REL_KIND_NON_AGGRESSION_PACT, REL_KIND_PUPPET,
    STATE_EVENT_KIND_OBSERVED, STATE_EVENT_SOURCE_AGENCY_CAPACITY,
    STATE_EVENT_SOURCE_AGENCY_SCALAR, STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP,
    STATE_EVENT_SOURCE_CRYPTOLOGY, STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
    STATE_EVENT_SOURCE_RELATION_FLAG, STATE_EVENT_SOURCE_RELATION_OPINION,
    STATE_EVENT_SOURCE_SUBJECT_AUTONOMY, STATUS_FACTION_GROUP_UNVALIDATED,
    STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED, STATUS_INTEL_MAP_RESISTED,
    STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED, STATUS_OPERATIONS_DETAIL_RESISTED,
    STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED, STATUS_WAR_ROOT_UNVALIDATED,
};
use compact_contract::build_profile as profile;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::time::Instant;

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
pub const SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF: u64 = 1 << 12;
pub const SOURCE_PROOF_FACTION_SYSTEM_870EBF: u64 = 1 << 13;
pub const SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF: u64 = 1 << 14;
pub const SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF: u64 = 1 << 15;

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
    | SOURCE_PROOF_FACTION_GROUP_870EBF
    | SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF
    | SOURCE_PROOF_FACTION_SYSTEM_870EBF
    | SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF
    | SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF;

pub const CURRENT_VALIDATED_SOURCE_PROOFS: u64 = SOURCE_PROOF_BUILD_SHA_870EBF
    | SOURCE_PROOF_COUNTRY_ROOTS_870EBF
    | SOURCE_PROOF_RELATION_ROOTS_870EBF
    | SOURCE_PROOF_AGENCY_ROOT_870EBF
    | SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF
    | SOURCE_PROOF_OPERATIVE_ROOT_870EBF
    | SOURCE_PROOF_WAR_ROOT_870EBF
    | SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF
    | SOURCE_PROOF_FACTION_GROUP_870EBF
    | SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF
    | SOURCE_PROOF_FACTION_SYSTEM_870EBF
    | SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF;

const RTTI_CCOUNTRY: &str = "CCountry";
const RTTI_CINTELLIGENCE_AGENCY: &str = "CIntelligenceAgency";
const DOMAIN53_RTTI_MAX_NAME_LEN: usize = 200;
const DOMAIN53_MODULE_SPAN_MAX: u64 = 0x5000_0000;
const DOMAIN53_FACTION_PROBE_COUNTRY_FLOOR: usize = 40;

const OPERATION_TOKEN_DATABASE_GLOBAL_RVA: usize = 0x3316BE8;
const OPERATION_TOKEN_DATABASE_MAP_OFFSET: usize = 0x40;
const OPERATION_TOKEN_DATABASE_ENTRY_LEN: usize = 0x38;
const OPERATION_TOKEN_DEFINITION_ID_OFFSET: usize = 0x50;
const COUNTRY_OPERATION_TOKEN_MANAGER_OFFSET: usize = 0x15B0;
const OPERATION_TOKEN_MANAGER_MAP_OFFSET: usize = 0x28;
const OPERATION_TOKEN_MANAGER_ENTRY_LEN: usize = 0x28;
const MAX_OPERATION_TOKEN_DATABASE_ENTRIES: usize = 4_096;
const MAX_OPERATION_TOKENS_PER_TARGET: usize = 256;

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
        name: "current_subject_autonomy_status",
        evidence: "subject CCountry+0xF88 -> diplomacy+0x350 -> CCurrentAutonomyStatus module+0x27CD3A0; score/state/previous/lower/next/upper/subject/date at +0x08/+0x28/+0x30/+0x38/+0x40/+0x48/+0x50/+0x60",
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
        evidence: "COperativeLeader vtable/classdesc module+0x293F790, portrait GFX MSVC string identity+0xF8, exact assignment discriminator identity+0x1078, owner identity+0x118, skill/XP stats path",
    },
    CurrentBuildRoot {
        name: "war_relation_rows",
        evidence: "global row pool with tag ids +0x00/+0x04, date vtable +0x08 in {CGregorianDate module+0x2702190, CGameDate module+0x2702388}, casualties +0x40/+0x48 i32, echo +0x68/+0x6C; v13 warm validation 850/850",
    },
    CurrentBuildRoot {
        name: "incoming_diplomatic_action_queue",
        evidence: "country+0xFF8 -> queue, queue+0x08 action pointer array, queue+0x10 size/cap, queue+0x20 recipient guard, CIncomingDiplomaticAction vtable module+0x2A209A0, handle +0x08, body +0x18 actor/recipient/date fields",
    },
    CurrentBuildRoot {
        name: "intel_network_graph",
        evidence: "CStrategicOperative vtable module+0x298B448, +0x08 owner CId, +0x10 vector<CCountryIntelNetwork*>, +0x18 capacity, +0x1C size; CCountryIntelNetwork vtable module+0x298AB48, target +0x08, graph +0x18, graph+0x38 vertex array, graph+0x40 vertex count, 16-byte vertices state_id@+0; COperativeLeader action +0x1088 variants 1 build_network/2 quiet_network resolve wrapper +0x10 -> network +0x18",
    },
    CurrentBuildRoot {
        name: "operation_token_membership",
        evidence: "module+0x3316BE8 COperationTokensDatabase; database map +0x40; definition authored id +0x50; CCountry+0x15B0 COperationTokenManager; target map +0x28",
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
    pub subject_autonomy_statuses: Vec<SubjectAutonomyStatusRow>,
    pub subject_autonomy_status_complete: bool,
    pub agencies: Vec<AgencyRow>,
    pub agency_capacity_clusters: Vec<AgencyCapacityClusterRow>,
    pub operatives: Vec<OperativeRow>,
    pub cryptology: Vec<CryptologyRow>,
    pub operation_anchors: Vec<OperationAnchorRow>,
    pub intel_map: Vec<IntelMapRow>,
    pub state_events: Vec<StateEventRow>,
    pub operation_instances: Vec<OperationInstanceRow>,
    pub incoming_actions: Vec<IncomingActionQueueRow>,
    pub intel_network_graphs: Vec<IntelNetworkGraphRow>,
    pub factions: Vec<FactionRow>,
    pub faction_roster_complete: bool,
    pub operation_tokens: Vec<OperationTokenMembershipRow>,
    pub operation_tokens_complete: bool,
    pub air_disclosure_threshold_raw: Option<i64>,
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

    fn read_into(&self, address: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
        let bytes = self.read_bytes(address, buf.len())?;
        buf.copy_from_slice(&bytes);
        Ok(())
    }

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

const RELATION_HOT_FIELD_START: usize = profile::DOMAIN53_RELATION_SUBJECT_PTR;
const RELATION_HOT_FIELD_LEN: usize =
    profile::DOMAIN53_RELATION_OPINION + std::mem::size_of::<i32>() - RELATION_HOT_FIELD_START;
const RELATION_HOT_SUBJECT_PTR_OFFSET: usize =
    profile::DOMAIN53_RELATION_SUBJECT_PTR - RELATION_HOT_FIELD_START;
const RELATION_COUNT_READ_START: usize = profile::DOMAIN53_RELATION_SUBREL_COUNT;
const RELATION_COUNT_READ_LEN: usize = std::mem::size_of::<i32>();
const fn window_hint_count(len: usize) -> usize {
    1 + (len - 1).div_ceil(64)
}
const RELATION_PREFETCH_HINT_COUNT: usize =
    window_hint_count(RELATION_COUNT_READ_LEN) + window_hint_count(RELATION_HOT_FIELD_LEN);

const fn relation_prefetch_hint_offsets() -> [usize; RELATION_PREFETCH_HINT_COUNT] {
    let mut out = [0usize; RELATION_PREFETCH_HINT_COUNT];
    let mut written = 0;
    written = fill_window_hints(
        &mut out,
        written,
        RELATION_COUNT_READ_START,
        RELATION_COUNT_READ_LEN,
    );
    written = fill_window_hints(
        &mut out,
        written,
        RELATION_HOT_FIELD_START,
        RELATION_HOT_FIELD_LEN,
    );
    assert!(
        written == RELATION_PREFETCH_HINT_COUNT,
        "relation prefetch hint table under-filled"
    );
    out
}

const fn fill_window_hints(
    out: &mut [usize; RELATION_PREFETCH_HINT_COUNT],
    written: usize,
    start: usize,
    len: usize,
) -> usize {
    let last = start + len - 1;
    let hints = window_hint_count(len);
    let mut index = 0;
    while index < hints {
        let offset = start + index * 64;
        out[written + index] = if offset >= last { last } else { offset };
        index += 1;
    }
    written + hints
}

const RELATION_PREFETCH_HINT_OFFSETS: [usize; RELATION_PREFETCH_HINT_COUNT] =
    relation_prefetch_hint_offsets();

const RELATION_HOT_OPINION_OFFSET: usize =
    profile::DOMAIN53_RELATION_OPINION - RELATION_HOT_FIELD_START;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain53RelationReadMode {
    Scan,
    Offsets,
    Shadow,
}

impl Domain53RelationReadMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Offsets => "offsets",
            Self::Shadow => "shadow",
        }
    }
}

const DOMAIN53_RELATION_READ_MODE_SCAN: u8 = 0;
const DOMAIN53_RELATION_READ_MODE_OFFSETS: u8 = 1;
const DOMAIN53_RELATION_READ_MODE_SHADOW: u8 = 2;

static DOMAIN53_RELATION_READ_MODE: AtomicU8 = AtomicU8::new(DOMAIN53_RELATION_READ_MODE_OFFSETS);

pub fn parse_relation_read_mode(raw: Option<&str>) -> Domain53RelationReadMode {
    match raw.map(str::trim) {
        Some(value) if value.eq_ignore_ascii_case("scan") => Domain53RelationReadMode::Scan,
        Some(value) if value.eq_ignore_ascii_case("shadow") => Domain53RelationReadMode::Shadow,
        Some(value) if value.eq_ignore_ascii_case("offsets") => Domain53RelationReadMode::Offsets,
        _ => Domain53RelationReadMode::Offsets,
    }
}

pub fn set_relation_read_mode(mode: Domain53RelationReadMode) {
    let raw = match mode {
        Domain53RelationReadMode::Scan => DOMAIN53_RELATION_READ_MODE_SCAN,
        Domain53RelationReadMode::Offsets => DOMAIN53_RELATION_READ_MODE_OFFSETS,
        Domain53RelationReadMode::Shadow => DOMAIN53_RELATION_READ_MODE_SHADOW,
    };
    DOMAIN53_RELATION_READ_MODE.store(raw, Ordering::Relaxed);
}

pub fn relation_read_mode() -> Domain53RelationReadMode {
    match DOMAIN53_RELATION_READ_MODE.load(Ordering::Relaxed) {
        DOMAIN53_RELATION_READ_MODE_SCAN => Domain53RelationReadMode::Scan,
        DOMAIN53_RELATION_READ_MODE_SHADOW => Domain53RelationReadMode::Shadow,
        _ => Domain53RelationReadMode::Offsets,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RelationHotFields {
    opinion: i32,
    subject_ptr: u64,
}

fn read_relation_hot_fields<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    buf: &mut [u8; RELATION_HOT_FIELD_LEN],
) -> Result<RelationHotFields, ExtractError> {
    mem.read_into(add_addr(relation_entry, RELATION_HOT_FIELD_START)?, buf)?;
    Ok(RelationHotFields {
        subject_ptr: u64::from_le_bytes(
            buf[RELATION_HOT_SUBJECT_PTR_OFFSET..RELATION_HOT_SUBJECT_PTR_OFFSET + 8]
                .try_into()
                .expect("relation hot subject pointer bytes"),
        ),
        opinion: i32::from_le_bytes(
            buf[RELATION_HOT_OPINION_OFFSET..RELATION_HOT_OPINION_OFFSET + 4]
                .try_into()
                .expect("relation hot opinion bytes"),
        ),
    })
}

fn valid_relation_opinion(opinion: i32) -> bool {
    (-MAX_OPINION_ABS..=MAX_OPINION_ABS).contains(&opinion)
}

fn read_relation_opinion_with_retry<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    hot: Option<RelationHotFields>,
) -> Result<i32, ExtractError> {
    if let Some(opinion) = hot
        .map(|fields| fields.opinion)
        .filter(|value| valid_relation_opinion(*value))
    {
        return Ok(opinion);
    }
    let opinion = mem.read_i32(add_addr(
        relation_entry,
        profile::DOMAIN53_RELATION_OPINION,
    )?)?;
    if valid_relation_opinion(opinion) {
        Ok(opinion)
    } else {
        Err(ExtractError::Validation(
            "domain53_relation_opinion_out_of_range_after_retry".to_string(),
        ))
    }
}

fn valid_relation_subrel_count(count: i32) -> bool {
    (0..=profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT).contains(&count)
}

fn read_relation_subrel_count_with_retry<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
) -> Result<i32, ExtractError> {
    let count_addr = add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_COUNT)?;
    let sampled = mem.read_i32(count_addr)?;
    if valid_relation_subrel_count(sampled) {
        return Ok(sampled);
    }
    let count = mem.read_i32(count_addr)?;
    if valid_relation_subrel_count(count) {
        Ok(count)
    } else {
        Err(ExtractError::Validation(
            "domain53_relation_subrel_count_invalid_after_retry".to_string(),
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
    pub operation_candidates: u32,
    pub operation_emitted: u32,
    pub operation_stale_liveness: u32,
    pub operation_filtered: u32,
    pub strategic_operative_candidates: u32,
    pub intel_network_candidates: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OperationSemanticCause {
    DefinitionPointerUnreadable,
    DefinitionObjectInvalid,
    DefinitionTokenInvalid,
    OwnerCountryInvalid,
    TargetCountryUnreadable,
    TargetCountryUnknown,
    TargetObjectInvalid,
    PhaseCountUnreadable,
    PhaseCountInvalid,
    DurationUnreadable,
    TerminalFlagInvalid,
    ZeroDurationTerminal,
    PreparationCurrentUnreadable,
    PreparationRequiredUnreadable,
    PreparationValuesNegative,
    PreparedStateUnavailable,
    StartHourUnreadable,
    ElapsedTimeInvalid,
    RunningPastDuration,
    PhaseProgressInvalid,
    DurationHoursOverflow,
    AssignedOperativesUnavailable,
    AssignedOperativeJoinInvalid,
}

impl OperationSemanticCause {
    pub const ALL: [Self; 23] = [
        Self::DefinitionPointerUnreadable,
        Self::DefinitionObjectInvalid,
        Self::DefinitionTokenInvalid,
        Self::OwnerCountryInvalid,
        Self::TargetCountryUnreadable,
        Self::TargetCountryUnknown,
        Self::TargetObjectInvalid,
        Self::PhaseCountUnreadable,
        Self::PhaseCountInvalid,
        Self::DurationUnreadable,
        Self::TerminalFlagInvalid,
        Self::ZeroDurationTerminal,
        Self::PreparationCurrentUnreadable,
        Self::PreparationRequiredUnreadable,
        Self::PreparationValuesNegative,
        Self::PreparedStateUnavailable,
        Self::StartHourUnreadable,
        Self::ElapsedTimeInvalid,
        Self::RunningPastDuration,
        Self::PhaseProgressInvalid,
        Self::DurationHoursOverflow,
        Self::AssignedOperativesUnavailable,
        Self::AssignedOperativeJoinInvalid,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IntelMapFailureCause {
    CountriesEmpty,
    DenominatorsRootUnavailable,
    CivilianDenominatorInvalid,
    ArmyDenominatorInvalid,
    NavyDenominatorInvalid,
    AirDenominatorInvalid,
    NoFogFlagUnreadable,
    ViewerIntelTableUnavailable,
    IntelTableDataUnavailable,
    IntelTableCountInvalid,
    TargetCountryIndexInvalid,
    FullVisibilityReadFailed,
    IntelLaneValueInvalid,
    NormalizedValueOutOfRange,
    IntelLaneScaleOverflow,
}

impl IntelMapFailureCause {
    pub const ALL: [Self; 15] = [
        Self::CountriesEmpty,
        Self::DenominatorsRootUnavailable,
        Self::CivilianDenominatorInvalid,
        Self::ArmyDenominatorInvalid,
        Self::NavyDenominatorInvalid,
        Self::AirDenominatorInvalid,
        Self::NoFogFlagUnreadable,
        Self::ViewerIntelTableUnavailable,
        Self::IntelTableDataUnavailable,
        Self::IntelTableCountInvalid,
        Self::TargetCountryIndexInvalid,
        Self::FullVisibilityReadFailed,
        Self::IntelLaneValueInvalid,
        Self::NormalizedValueOutOfRange,
        Self::IntelLaneScaleOverflow,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum IntelNetworkFailureCause {
    NetworkHitsWithoutStrategicOwner,
    CountriesEmpty,
    NetworkVtableInvalid,
    NetworkTargetUnreadable,
    NetworkTargetNonPositive,
    NetworkTargetUnknown,
    NetworkGraphUnreadable,
    NetworkGraphPointerInvalid,
    GraphRowDataUnreadable,
    GraphRowEndUnreadable,
    GraphRowRangeReversed,
    GraphRowShapeInvalid,
    GraphRowCountInvalid,
    VertexDataUnreadable,
    VertexCountUnreadable,
    VertexCountInvalid,
    VertexDataPointerInvalid,
    SubnetworkDataUnreadable,
    SubnetworkSizeUnreadable,
    SubnetworkShapeInvalid,
    VertexStateIdUnreadable,
    VertexStateIdInvalid,
    VertexRowIndexUnreadable,
    VertexRowIndexOutOfRange,
    GraphRowSelectorUnreadable,
    GraphRowSelectorOutOfRange,
    SubnetworkStrengthUnreadable,
    SubnetworkStrengthOutOfRange,
    DuplicateStateId,
    DuplicateNetworkPointer,
    DuplicateOperativeIdentity,
    OperativeHandleTypeUnreadable,
    OperativeIdUnreadable,
    OperativeIdentityInvalid,
    OperativeActionVariantUnreadable,
    OperativeActionWrapperUnreadable,
    OperativeActionWrapperInvalid,
    OperativeActionNetworkUnreadable,
    OperativeActionNetworkUnknown,
    DuplicateNetworkOperative,
    StrategicOperativeVtableInvalid,
    StrategicOwnerUnreadable,
    StrategicOwnerUnknown,
    StrategicNetworkVectorUnreadable,
    StrategicNetworkSizeUnreadable,
    StrategicNetworkCapacityUnreadable,
    StrategicNetworkShapeInvalid,
    StrategicNetworkSlotUnreadable,
    StrategicNetworkSlotUnknown,
    NetworkOwnerEqualsTarget,
    NetworkReachedTwice,
    DuplicateOwnerSlot,
    NetworkOperativeOwnerMismatch,
    UnreachedNetworkCount,
}

impl IntelNetworkFailureCause {
    pub const ALL: [Self; 54] = [
        Self::NetworkHitsWithoutStrategicOwner,
        Self::CountriesEmpty,
        Self::NetworkVtableInvalid,
        Self::NetworkTargetUnreadable,
        Self::NetworkTargetNonPositive,
        Self::NetworkTargetUnknown,
        Self::NetworkGraphUnreadable,
        Self::NetworkGraphPointerInvalid,
        Self::GraphRowDataUnreadable,
        Self::GraphRowEndUnreadable,
        Self::GraphRowRangeReversed,
        Self::GraphRowShapeInvalid,
        Self::GraphRowCountInvalid,
        Self::VertexDataUnreadable,
        Self::VertexCountUnreadable,
        Self::VertexCountInvalid,
        Self::VertexDataPointerInvalid,
        Self::SubnetworkDataUnreadable,
        Self::SubnetworkSizeUnreadable,
        Self::SubnetworkShapeInvalid,
        Self::VertexStateIdUnreadable,
        Self::VertexStateIdInvalid,
        Self::VertexRowIndexUnreadable,
        Self::VertexRowIndexOutOfRange,
        Self::GraphRowSelectorUnreadable,
        Self::GraphRowSelectorOutOfRange,
        Self::SubnetworkStrengthUnreadable,
        Self::SubnetworkStrengthOutOfRange,
        Self::DuplicateStateId,
        Self::DuplicateNetworkPointer,
        Self::DuplicateOperativeIdentity,
        Self::OperativeHandleTypeUnreadable,
        Self::OperativeIdUnreadable,
        Self::OperativeIdentityInvalid,
        Self::OperativeActionVariantUnreadable,
        Self::OperativeActionWrapperUnreadable,
        Self::OperativeActionWrapperInvalid,
        Self::OperativeActionNetworkUnreadable,
        Self::OperativeActionNetworkUnknown,
        Self::DuplicateNetworkOperative,
        Self::StrategicOperativeVtableInvalid,
        Self::StrategicOwnerUnreadable,
        Self::StrategicOwnerUnknown,
        Self::StrategicNetworkVectorUnreadable,
        Self::StrategicNetworkSizeUnreadable,
        Self::StrategicNetworkCapacityUnreadable,
        Self::StrategicNetworkShapeInvalid,
        Self::StrategicNetworkSlotUnreadable,
        Self::StrategicNetworkSlotUnknown,
        Self::NetworkOwnerEqualsTarget,
        Self::NetworkReachedTwice,
        Self::DuplicateOwnerSlot,
        Self::NetworkOperativeOwnerMismatch,
        Self::UnreachedNetworkCount,
    ];
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FamilyRead<T, C> {
    Proven(T),
    Unavailable(C),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Domain53RejectionCounts {
    operation: [u32; OperationSemanticCause::ALL.len()],
    intel_map: [u32; IntelMapFailureCause::ALL.len()],
    intel_network: [u32; IntelNetworkFailureCause::ALL.len()],
}

impl Default for Domain53RejectionCounts {
    fn default() -> Self {
        Self {
            operation: [0; OperationSemanticCause::ALL.len()],
            intel_map: [0; IntelMapFailureCause::ALL.len()],
            intel_network: [0; IntelNetworkFailureCause::ALL.len()],
        }
    }
}

impl Domain53RejectionCounts {
    pub fn record_operation(&mut self, cause: OperationSemanticCause) {
        let slot = &mut self.operation[cause as usize];
        *slot = slot.saturating_add(1);
    }

    pub fn record_intel_map(&mut self, cause: IntelMapFailureCause) {
        let slot = &mut self.intel_map[cause as usize];
        *slot = slot.saturating_add(1);
    }

    pub fn record_intel_network(&mut self, cause: IntelNetworkFailureCause) {
        let slot = &mut self.intel_network[cause as usize];
        *slot = slot.saturating_add(1);
    }

    pub fn operation(&self, cause: OperationSemanticCause) -> u32 {
        self.operation[cause as usize]
    }

    pub fn intel_map(&self, cause: IntelMapFailureCause) -> u32 {
        self.intel_map[cause as usize]
    }

    pub fn intel_network(&self, cause: IntelNetworkFailureCause) -> u32 {
        self.intel_network[cause as usize]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53PhaseTimes {
    pub wars_us: u64,
    pub faction_gate_us: u64,
    pub relations_us: u64,
    pub faction_roster_us: u64,
    pub incoming_actions_us: u64,
    pub agencies_us: u64,
    pub operatives_us: u64,
    pub operations_us: u64,
    pub intel_networks_us: u64,
    pub cryptology_us: u64,
    pub roots_us: u64,
    pub war_projection_us: u64,
    pub aggregation_us: u64,
}

fn duration_micros_u64_saturating(elapsed: std::time::Duration) -> u64 {
    elapsed.as_micros().min(u128::from(u64::MAX)) as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedDomain53Frame {
    pub rows: ExtractedDomain53Rows,
    pub source_proofs: u64,
    pub observed_rows: u32,
    pub skipped_rows: u32,
    pub skip_counts: Domain53SkipCounts,
    pub stable_rejects: Domain53StableRejects,
    pub stale_dropped_total: usize,
    pub family_counts: Domain53FamilyCounts,
    pub rejection_counts: Domain53RejectionCounts,
    pub phase_times: Domain53PhaseTimes,
    pub relflag_offsets: RelationFlagOffsetHistogram,
    pub relation_shadow_mismatches: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53SkipCounts {
    pub relation_root: u32,
    pub relation_slot: u32,
    pub relation_opinion: u32,
    pub relation_subject: u32,
    pub relation_container: u32,
    pub incoming_action: u32,
    pub agency: u32,
    pub cryptology: u32,
    pub operative: u32,
    pub war: u32,
}

impl Domain53SkipCounts {
    pub fn total(self) -> u32 {
        [
            self.relation_root,
            self.relation_slot,
            self.relation_opinion,
            self.relation_subject,
            self.relation_container,
            self.incoming_action,
            self.agency,
            self.cryptology,
            self.operative,
            self.war,
        ]
        .into_iter()
        .fold(0u32, u32::saturating_add)
    }
}

/// Operative candidate observations proven to carry no row: a needle-hit
/// address whose row read failed with the IDENTICAL value-carrying error on
/// two attempts within one frame, with the vtable qword and the identity
/// head re-read AFTER the second attempt still matching their before-samples
/// (the full observation bracket in `read_operatives`).
///
/// This is discovery noise - freed heap slots still holding the needle-vtable
/// qword, unused pool entries - not an omission of data: there is no
/// publishable row at the address under this contract, and the whole bracket
/// proves the address stably holds none. Counted per frame and surfaced as a
/// PERF diagnostic unit so every frame still witnesses it, but it must never
/// feed `skipped_rows`/PARTIAL_DATA: counting a stably-empty candidate as a
/// missed data row misattributes the hour.
///
/// Deliberately LIMITED to this family: queue headers/data pointers advertise
/// rows that would become unreadable, duplicate action ids cannot be proven
/// to be the same delivery, and a repeated non-heap relation-slot value is
/// not proven emptiness - all of those stay refuse-by-name skips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Domain53StableRejects {
    pub operative: u32,
}

pub const DOMAIN53_DISCOVERY_DEGRADED: &str = "domain53_discovery_degraded";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Domain53Discovery {
    pub operative_hits: Vec<u64>,
    pub operation_instance_hits: Vec<u64>,
    pub strategic_operative_hits: Vec<u64>,
    pub country_intel_network_hits: Vec<u64>,
    pub scan_range_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountryRoot {
    tag_db_id: u32,
    country_ptr: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CountryTableSlotFingerprint {
    Unreadable,
    Vacant,
    Rejected { country_ptr: u64 },
    Accepted { country_ptr: u64, tag_db_id: u32 },
    AmbiguousDuplicateTag { country_ptr: u64, tag_db_id: u32 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CountryAcceptanceOracle {
    country_table: u64,
    country_allocation: u32,
    country_count: u32,
    header_capture_stable: bool,
    allocation_capture_complete: bool,
    slot_allocation: Vec<CountryTableSlotFingerprint>,
    accepted_countries: Vec<CountryRoot>,
    vacant_slot_count: u32,
    ambiguous_or_rejected_count: u32,
    duplicate_tag_slot_count: u32,
}

impl CountryAcceptanceOracle {
    fn supports_positive_faction_witness(&self) -> bool {
        self.slot_allocation.len() == self.country_count as usize
            && self.allocation_capture_complete
            && self.header_capture_stable
            && self.country_count <= self.country_allocation
            && self.ambiguous_or_rejected_count == 0
            && self.duplicate_tag_slot_count == 0
            && self
                .accepted_countries
                .len()
                .saturating_add(self.vacant_slot_count as usize)
                == self.country_count as usize
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelationFlagOffsetHistogram {
    pub per_kind: BTreeMap<u16, (u64, u64)>,
    pub total_matches: u64,
}

impl RelationFlagOffsetHistogram {
    fn record_kind_count(&mut self, flag_kind: u16) {
        let entry = self.per_kind.entry(flag_kind).or_default();
        entry.0 = entry.0.saturating_add(1);
        self.total_matches = self.total_matches.saturating_add(1);
    }

    fn merge_from(&mut self, other: &Self) {
        for (&flag_kind, &(count, high)) in &other.per_kind {
            let entry = self.per_kind.entry(flag_kind).or_default();
            entry.0 = entry.0.saturating_add(count);
            entry.1 = entry.1.saturating_add(high);
        }
        self.total_matches = self.total_matches.saturating_add(other.total_matches);
    }
}

#[derive(Clone, Debug, Default)]
struct RowAccumulator {
    rows: ExtractedDomain53Rows,
    skipped_rows: u32,
    skip_counts: Domain53SkipCounts,
    stable_rejects: Domain53StableRejects,
    observed_rows: u32,
    relflag_offsets: RelationFlagOffsetHistogram,
    relation_shadow_mismatches: u64,
}

impl RowAccumulator {
    fn skip(&mut self) {
        self.skipped_rows = self.skipped_rows.saturating_add(1);
    }

    fn skip_relation_root(&mut self) {
        self.skip();
        self.skip_counts.relation_root = self.skip_counts.relation_root.saturating_add(1);
    }

    fn skip_relation_slot(&mut self) {
        self.skip();
        self.skip_counts.relation_slot = self.skip_counts.relation_slot.saturating_add(1);
    }

    fn skip_relation_opinion(&mut self) {
        self.skip();
        self.skip_counts.relation_opinion = self.skip_counts.relation_opinion.saturating_add(1);
    }

    fn skip_relation_subject(&mut self) {
        self.skip();
        self.skip_counts.relation_subject = self.skip_counts.relation_subject.saturating_add(1);
    }

    fn skip_relation_container(&mut self) {
        self.skip();
        self.skip_counts.relation_container = self.skip_counts.relation_container.saturating_add(1);
    }

    fn skip_incoming_action(&mut self) {
        self.skip();
        self.skip_counts.incoming_action = self.skip_counts.incoming_action.saturating_add(1);
    }

    fn skip_agency(&mut self) {
        self.skip();
        self.skip_counts.agency = self.skip_counts.agency.saturating_add(1);
    }

    fn skip_cryptology(&mut self) {
        self.skip();
        self.skip_counts.cryptology = self.skip_counts.cryptology.saturating_add(1);
    }

    fn skip_operative(&mut self) {
        self.skip();
        self.skip_counts.operative = self.skip_counts.operative.saturating_add(1);
    }

    fn skip_war(&mut self) {
        self.skip();
        self.skip_counts.war = self.skip_counts.war.saturating_add(1);
    }

    fn observe(&mut self) {
        self.observed_rows = self.observed_rows.saturating_add(1);
    }

    fn merge_relation_rows_from(&mut self, mut other: Self) {
        self.rows.opinions.append(&mut other.rows.opinions);
        self.rows
            .relation_flags
            .append(&mut other.rows.relation_flags);
        self.rows.subjects.append(&mut other.rows.subjects);
        self.rows
            .subject_autonomy_statuses
            .append(&mut other.rows.subject_autonomy_statuses);
        self.rows.subject_autonomy_status_complete = other.rows.subject_autonomy_status_complete;
        self.rows.state_events.append(&mut other.rows.state_events);
        self.skipped_rows = self.skipped_rows.saturating_add(other.skipped_rows);
        self.skip_counts.relation_root = self
            .skip_counts
            .relation_root
            .saturating_add(other.skip_counts.relation_root);
        self.skip_counts.relation_slot = self
            .skip_counts
            .relation_slot
            .saturating_add(other.skip_counts.relation_slot);
        self.skip_counts.relation_opinion = self
            .skip_counts
            .relation_opinion
            .saturating_add(other.skip_counts.relation_opinion);
        self.skip_counts.relation_subject = self
            .skip_counts
            .relation_subject
            .saturating_add(other.skip_counts.relation_subject);
        self.skip_counts.relation_container = self
            .skip_counts
            .relation_container
            .saturating_add(other.skip_counts.relation_container);
        self.observed_rows = self.observed_rows.saturating_add(other.observed_rows);
        self.relflag_offsets.merge_from(&other.relflag_offsets);
        self.relation_shadow_mismatches = self
            .relation_shadow_mismatches
            .saturating_add(other.relation_shadow_mismatches);
    }
}

#[derive(Clone, Debug)]
struct AgencyRootRead {
    owner_tag_db_id: u32,
    agency_ptr: u64,
}

#[derive(Clone, Debug)]
struct Domain53FrameRoots {
    module_base: u64,
    game_state: u64,
    current_game_hour: u32,
    countries: Vec<CountryRoot>,
    country_acceptance_oracle: CountryAcceptanceOracle,
    roots_resolve_us: u64,
}

#[derive(Clone, Debug, Default)]
struct Domain53DiscoveryStaleness {
    operative_total: usize,
    operative_stale_dropped: usize,
    war_total: usize,
    war_stale_dropped: usize,
    operation_total: usize,
    operation_stale_dropped: usize,
    operation_filtered: usize,
}

impl Domain53DiscoveryStaleness {
    fn frame_degraded(&self) -> bool {
        stale_fraction_degraded(self.operative_total, self.operative_stale_dropped)
    }

    #[allow(dead_code)]
    fn operation_degraded(&self) -> bool {
        stale_fraction_degraded(self.operation_total, self.operation_stale_dropped)
    }

    fn war_degraded(&self) -> bool {
        stale_fraction_degraded(self.war_total, self.war_stale_dropped)
    }

    fn stale_dropped_total(&self) -> usize {
        self.operative_stale_dropped
            .saturating_add(self.war_stale_dropped)
            .saturating_add(self.operation_stale_dropped)
    }
}

fn stale_fraction_degraded(total: usize, stale: usize) -> bool {
    total >= 8 && stale.saturating_mul(2) > total
}

pub fn discover_domain53_heap_hits<M: Domain53Memory>(
    mem: &M,
) -> Result<Domain53Discovery, ExtractError> {
    let module_base = mem.module_base();
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }
    verify_domain53_build_identity(mem, module_base)?;
    discover_domain53_heap_hits_for_module(mem, module_base)
}

pub fn domain53_scan_needles(module_base: u64) -> Result<[u64; 4], ExtractError> {
    if module_base == 0 {
        return Err(ExtractError::InvalidRoot("module_base_zero"));
    }
    Ok([
        add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?,
        add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?,
        add_addr(
            module_base,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
        )?,
        add_addr(
            module_base,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
        )?,
    ])
}

pub fn domain53_discovery_from_needle_hits(
    operative_hits: Vec<u64>,
    operation_instance_hits: Vec<u64>,
    strategic_operative_hits: Vec<u64>,
    country_intel_network_hits: Vec<u64>,
) -> Domain53Discovery {
    Domain53Discovery {
        operative_hits,
        operation_instance_hits,
        strategic_operative_hits,
        country_intel_network_hits,
        scan_range_count: 0,
    }
}

fn discover_domain53_heap_hits_for_module<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
) -> Result<Domain53Discovery, ExtractError> {
    let ranges = mem.readable_ranges(
        profile::DOMAIN53_HEAP_LO as u64,
        profile::DOMAIN53_HEAP_HI as u64,
    );
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    let operation_instance_vtable =
        add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?;
    let strategic_operative_vtable = add_addr(
        module_base,
        profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
    )?;
    let country_intel_network_vtable = add_addr(
        module_base,
        profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
    )?;
    Ok(Domain53Discovery {
        operative_hits: scan_qword_hits(mem, &ranges, operative_vtable)?,
        operation_instance_hits: scan_qword_hits(mem, &ranges, operation_instance_vtable)?,
        strategic_operative_hits: scan_qword_hits(mem, &ranges, strategic_operative_vtable)?,
        country_intel_network_hits: scan_qword_hits(mem, &ranges, country_intel_network_vtable)?,
        scan_range_count: ranges.len().min(u32::MAX as usize) as u32,
    })
}

pub fn extract_rows_from_current_roots_with_validated_war_addrs<M: Domain53Memory>(
    mem: &M,
    discovery: &Domain53Discovery,
    war_row_addrs: &[u64],
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    extract_rows_from_resolved_roots_with_discovery(mem, roots, discovery, war_row_addrs)
}

fn resolve_domain53_frame_roots<M: Domain53Memory>(
    mem: &M,
) -> Result<Domain53FrameRoots, ExtractError> {
    let roots_started = Instant::now();
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

    let country_acceptance_oracle =
        resolve_country_acceptance_oracle(mem, module_base, game_state)?;
    let countries = country_acceptance_oracle.accepted_countries.clone();
    if countries.is_empty() {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_roots_empty_implausible",
        ));
    }

    let current_game_hour =
        mem.read_u32(add_addr(game_state, profile::CGAMESTATE_CURRENT_HOUR)?)?;

    Ok(Domain53FrameRoots {
        module_base,
        game_state,
        current_game_hour,
        countries,
        country_acceptance_oracle,
        roots_resolve_us: duration_micros_u64_saturating(roots_started.elapsed()),
    })
}

fn extract_rows_from_resolved_roots_with_discovery<M: Domain53Memory>(
    mem: &M,
    roots: Domain53FrameRoots,
    discovery: &Domain53Discovery,
    war_row_addrs: &[u64],
) -> Result<ExtractedDomain53Frame, ExtractError> {
    let module_base = roots.module_base;
    let game_state = roots.game_state;
    let current_game_hour = roots.current_game_hour;
    let roots_resolve_us = roots.roots_resolve_us;
    let countries = roots.countries;
    let country_acceptance_oracle = roots.country_acceptance_oracle;
    let mut acc = RowAccumulator::default();
    let mut source_proofs = SOURCE_PROOF_BUILD_SHA_870EBF | SOURCE_PROOF_COUNTRY_ROOTS_870EBF;
    let mut family_counts = Domain53FamilyCounts {
        country_roots: countries.len().min(u32::MAX as usize) as u32,
        ..Domain53FamilyCounts::default()
    };
    let mut rejection_counts = Domain53RejectionCounts::default();
    let mut staleness = Domain53DiscoveryStaleness::default();
    let mut phase_times = Domain53PhaseTimes::default();
    phase_times.roots_us = roots_resolve_us;

    if let Some(operation_tokens) = read_operation_token_memberships(mem, module_base, &countries) {
        acc.rows.operation_tokens = operation_tokens;
        acc.rows.operation_tokens_complete = true;
        source_proofs |= SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF;
    }

    let wars_started = Instant::now();
    let (war_rows, war_total, war_stale_dropped) = {
        let (rows, stale) = read_war_rows_from_validated_addrs(
            mem,
            module_base,
            &countries,
            war_row_addrs,
            &mut acc,
        )?;
        (rows, war_row_addrs.len(), stale)
    };
    phase_times.wars_us = duration_micros_u64_saturating(wars_started.elapsed());
    staleness.war_total = war_total;
    staleness.war_stale_dropped = war_stale_dropped;
    family_counts.war_scan_ranges = discovery.scan_range_count;

    let war_projection_started = Instant::now();
    let war_degraded = staleness.war_degraded();
    let (war_rows, war_projection) = if war_degraded {
        (Vec::new(), BTreeMap::new())
    } else {
        source_proofs |= SOURCE_PROOF_WAR_ROOT_870EBF;
        let projection = war_projection_by_country(&war_rows);
        (war_rows, projection)
    };
    phase_times.war_projection_us =
        duration_micros_u64_saturating(war_projection_started.elapsed());
    let faction_gate_started = Instant::now();
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
            if !war_degraded {
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
            }
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
    phase_times.faction_gate_us = duration_micros_u64_saturating(faction_gate_started.elapsed());

    let relations_started = Instant::now();
    read_relation_rows(mem, module_base, current_game_hour, &countries, &mut acc)?;
    phase_times.relations_us = duration_micros_u64_saturating(relations_started.elapsed());
    let relation_rows =
        acc.rows.opinions.len() + acc.rows.relation_flags.len() + acc.rows.subjects.len();
    if relation_rows == 0 {
        return Err(ExtractError::InvalidRoot(
            "domain53_relations_empty_implausible",
        ));
    }
    family_counts.relation_rows = relation_rows.min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_RELATION_ROOTS_870EBF;
    if acc.rows.subject_autonomy_status_complete {
        source_proofs |= SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF;
    }

    let faction_roster_started = Instant::now();
    let faction_roster =
        read_faction_rows(mem, module_base, game_state, &country_acceptance_oracle)?;
    phase_times.faction_roster_us =
        duration_micros_u64_saturating(faction_roster_started.elapsed());
    if let Some(faction_rows) = faction_roster {
        acc.rows.factions = faction_rows;
        acc.rows.faction_roster_complete = true;
        source_proofs |= SOURCE_PROOF_FACTION_SYSTEM_870EBF;
    }

    let incoming_actions_started = Instant::now();
    read_incoming_action_queue_rows(mem, module_base, current_game_hour, &countries, &mut acc)?;
    phase_times.incoming_actions_us =
        duration_micros_u64_saturating(incoming_actions_started.elapsed());
    source_proofs |= SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF;

    let agencies_started = Instant::now();
    let agency_roots = read_agency_rows(mem, module_base, &countries, &mut acc)?;
    phase_times.agencies_us = duration_micros_u64_saturating(agencies_started.elapsed());
    family_counts.agency_roots = agency_roots.len().min(u32::MAX as usize) as u32;
    source_proofs |= SOURCE_PROOF_AGENCY_ROOT_870EBF;

    let operatives_started = Instant::now();
    let operative_stale_dropped = read_operatives(
        mem,
        module_base,
        &countries,
        &discovery.operative_hits,
        &mut acc,
    )?;
    phase_times.operatives_us = duration_micros_u64_saturating(operatives_started.elapsed());
    staleness.operative_total = discovery.operative_hits.len();
    staleness.operative_stale_dropped = operative_stale_dropped;
    family_counts.operative_scan_ranges = discovery.scan_range_count;
    source_proofs |= SOURCE_PROOF_OPERATIVE_ROOT_870EBF;

    let operations_started = Instant::now();
    let operative_ids = acc
        .rows
        .operatives
        .iter()
        .map(|row| row.op_id)
        .collect::<BTreeSet<_>>();
    let (operation_instances, operation_drops) = read_operations(
        mem,
        module_base,
        current_game_hour,
        &countries,
        &operative_ids,
        &discovery.operation_instance_hits,
    )?;
    if operation_drops.semantic_incomplete == 0 {
        source_proofs |= SOURCE_PROOF_OPERATION_DETAIL_870EBF;
    }
    phase_times.operations_us = duration_micros_u64_saturating(operations_started.elapsed());
    acc.rows.operation_instances = if operation_drops.semantic_incomplete == 0 {
        operation_instances
    } else {
        Vec::new()
    };
    staleness.operation_total = discovery.operation_instance_hits.len();
    staleness.operation_stale_dropped = operation_drops.stale_liveness;
    staleness.operation_filtered = operation_drops.filtered;
    family_counts.operation_candidates = discovery
        .operation_instance_hits
        .len()
        .min(u32::MAX as usize) as u32;
    family_counts.operation_emitted =
        acc.rows.operation_instances.len().min(u32::MAX as usize) as u32;
    family_counts.operation_stale_liveness =
        operation_drops.stale_liveness.min(u32::MAX as usize) as u32;
    family_counts.operation_filtered = operation_drops.filtered.min(u32::MAX as usize) as u32;
    for cause in OperationSemanticCause::ALL {
        rejection_counts.operation[cause as usize] =
            operation_drops.semantic_causes.operation(cause);
    }

    let intel_networks_started = Instant::now();
    let intel_network_graphs = read_intel_network_graph_rows(
        mem,
        module_base,
        &countries,
        &acc.rows.operatives,
        &discovery.operative_hits,
        &discovery.strategic_operative_hits,
        &discovery.country_intel_network_hits,
    )?;
    phase_times.intel_networks_us =
        duration_micros_u64_saturating(intel_networks_started.elapsed());
    family_counts.strategic_operative_candidates = discovery
        .strategic_operative_hits
        .len()
        .min(u32::MAX as usize) as u32;
    family_counts.intel_network_candidates = discovery
        .country_intel_network_hits
        .len()
        .min(u32::MAX as usize) as u32;
    match intel_network_graphs {
        FamilyRead::Proven(intel_network_graphs) => {
            acc.rows.intel_network_graphs = intel_network_graphs;
            source_proofs |= SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF;
        }
        FamilyRead::Unavailable(cause) => rejection_counts.record_intel_network(cause),
    }

    match read_intel_map_rows(mem, module_base, &countries)? {
        FamilyRead::Proven(intel_map) => {
            acc.rows.intel_map = intel_map;
            source_proofs |= SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF;
        }
        FamilyRead::Unavailable(cause) => rejection_counts.record_intel_map(cause),
    }

    acc.rows.air_disclosure_threshold_raw = read_air_disclosure_threshold(mem, module_base);

    let aggregation_started = Instant::now();
    for agency in &mut acc.rows.agencies {
        agency.operative_count = acc
            .rows
            .operatives
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }
    phase_times.aggregation_us = duration_micros_u64_saturating(aggregation_started.elapsed());

    let cryptology_started = Instant::now();
    read_cryptology_rows(mem, module_base, &countries, &agency_roots, &mut acc)?;
    phase_times.cryptology_us = duration_micros_u64_saturating(cryptology_started.elapsed());
    if !agency_roots.is_empty() {
        source_proofs |= SOURCE_PROOF_CRYPTOLOGY_ROOT_870EBF;
    }
    let aggregation2_started = Instant::now();
    for agency in &mut acc.rows.agencies {
        agency.cryptology_target_count = acc
            .rows
            .cryptology
            .iter()
            .filter(|row| row.owner_tag_db_id == agency.owner_tag_db_id)
            .count()
            .min(u32::MAX as usize) as u32;
    }
    phase_times.aggregation_us =
        phase_times
            .aggregation_us
            .saturating_add(duration_micros_u64_saturating(
                aggregation2_started.elapsed(),
            ));

    acc.rows.wars = war_rows;

    if staleness.frame_degraded() {
        return Err(ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
    }

    debug_assert_eq!(
        acc.skip_counts.total(),
        acc.skipped_rows,
        "every D53 row skip must carry typed family attribution"
    );
    Ok(ExtractedDomain53Frame {
        rows: acc.rows,
        source_proofs,
        observed_rows: acc.observed_rows,
        skipped_rows: acc.skipped_rows,
        skip_counts: acc.skip_counts,
        stable_rejects: acc.stable_rejects,
        stale_dropped_total: staleness.stale_dropped_total(),
        family_counts,
        rejection_counts,
        phase_times,
        relflag_offsets: acc.relflag_offsets,
        relation_shadow_mismatches: acc.relation_shadow_mismatches,
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
    let max_group = profile::DOMAIN53_MAX_FACTION_GROUP_INDEX as i32;

    for country in countries {
        let value = match mem.read_i32(add_addr(
            country.country_ptr,
            profile::DOMAIN53_COUNTRY_FACTION_GROUP,
        )?) {
            Ok(value) if (0..=max_group).contains(&value) => value,
            Ok(out_of_range) => {
                compact_contract::bound_trips::record_bound_trip(
                    compact_contract::bound_trips::BOUND_TRIP_FACTION_GROUP_INDEX,
                    out_of_range as u32 as u64,
                );
                continue;
            }
            Err(_) => {
                compact_contract::bound_trips::record_bound_trip(
                    compact_contract::bound_trips::BOUND_TRIP_FACTION_GROUP_INDEX,
                    u64::MAX,
                );
                continue;
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
        Ok(FactionGroupGate {
            validated: false,
            groups_by_country: BTreeMap::new(),
        })
    }
}

fn read_faction_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
    country_acceptance_oracle: &CountryAcceptanceOracle,
) -> Result<Option<Vec<FactionRow>>, ExtractError> {
    if !country_acceptance_oracle.supports_positive_faction_witness() {
        return Ok(None);
    }
    let accepted_country_by_ptr: BTreeMap<u64, u32> = country_acceptance_oracle
        .accepted_countries
        .iter()
        .map(|country| (country.country_ptr, country.tag_db_id))
        .collect();
    if accepted_country_by_ptr.len() != country_acceptance_oracle.accepted_countries.len() {
        return Ok(None);
    }

    let faction_system = match mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM,
    )?) {
        Ok(value) if is_heap_ptr(value) => value,
        Ok(_) => return Ok(None),
        Err(_) => return Ok(None),
    };

    let array_base = match mem.read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_ARRAY)?)
    {
        Ok(value) => value,
        _ => return Ok(None),
    };
    let packed_count =
        match mem.read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_COUNT)?) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
    let size = (packed_count & 0xFFFF_FFFF) as u32;
    let capacity = ((packed_count >> 32) & 0xFFFF_FFFF) as u32;
    if size > capacity
        || compact_contract::bound_trips::bound_exceeded_u32(
            capacity,
            profile::DOMAIN53_FACTION_MAX_SLOTS,
            compact_contract::bound_trips::BOUND_TRIP_FACTION_SLOTS,
        )
        || (capacity != 0 && !is_heap_ptr(array_base))
        || (capacity == 0 && array_base != 0 && !is_heap_ptr(array_base))
    {
        return Ok(None);
    }

    let expected_country_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let mut rows = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for slot in 0..size {
        let slot_addr = array_base
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("domain53_faction_array_overflow"))?;
        match read_faction_row(
            mem,
            slot_addr,
            expected_country_vtable,
            &accepted_country_by_ptr,
            &mut seen_ids,
        )? {
            Some(row) => rows.push(row),
            None => return Ok(None),
        }
    }
    let faction_system_after = mem
        .read_u64(add_addr(
            game_state,
            profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM,
        )?)
        .ok();
    let array_base_after = mem
        .read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_ARRAY)?)
        .ok();
    let packed_count_after = mem
        .read_u64(add_addr(faction_system, profile::DOMAIN53_FACTION_COUNT)?)
        .ok();
    if faction_system_after != Some(faction_system)
        || array_base_after != Some(array_base)
        || packed_count_after != Some(packed_count)
    {
        return Ok(None);
    }

    let game_state_after = mem
        .read_u64(add_addr(module_base, profile::CG_STATE_DATE_OFFSET)?)
        .ok();
    if game_state_after != Some(game_state) {
        return Ok(None);
    }
    let country_acceptance_oracle_after =
        match resolve_country_acceptance_oracle(mem, module_base, game_state) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
    if country_acceptance_oracle_after != *country_acceptance_oracle
        || !country_acceptance_oracle_after.supports_positive_faction_witness()
    {
        return Ok(None);
    }
    Ok(Some(rows))
}

fn read_faction_row<M: Domain53Memory>(
    mem: &M,
    slot_addr: u64,
    expected_country_vtable: u64,
    accepted_country_by_ptr: &BTreeMap<u64, u32>,
    seen_ids: &mut BTreeSet<u32>,
) -> Result<Option<FactionRow>, ExtractError> {
    let faction_ptr = match mem.read_u64(slot_addr) {
        Ok(value) if is_heap_ptr(value) => value,
        _ => return Ok(None),
    };
    let raw_id = match mem.read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_ID)?) {
        Ok(value) => value,
        Err(_) => return Ok(None),
    };
    let faction_type = (raw_id & 0xFFFF_FFFF) as u32;
    let faction_id = ((raw_id >> 32) & 0xFFFF_FFFF) as u32;
    if faction_type != profile::DOMAIN53_FACTION_TYPE_EXPECTED || faction_id == 0 {
        return Ok(None);
    }
    if !seen_ids.insert(faction_id) {
        return Ok(None);
    }

    let member_count = match mem.read_u32(add_addr(
        faction_ptr,
        profile::DOMAIN53_FACTION_MEMBERS_SIZE,
    )?) {
        Ok(value)
            if value > 0
                && compact_contract::bound_trips::gate_u32(
                    value,
                    profile::DOMAIN53_FACTION_MAX_MEMBERS,
                    compact_contract::bound_trips::BOUND_TRIP_FACTION_MEMBERS,
                )
                .is_some() =>
        {
            value
        }
        _ => return Ok(None),
    };
    let members_base = match mem.read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_MEMBERS)?)
    {
        Ok(value) if is_heap_ptr(value) => value,
        _ => return Ok(None),
    };

    let mut leader_tag_db_id = 0u32;
    let mut member_tag_db_ids = Vec::with_capacity(member_count as usize);
    let mut member_values = Vec::with_capacity(member_count as usize);
    for member_slot in 0..member_count {
        let member_slot_addr = members_base.checked_add(u64::from(member_slot) * 8).ok_or(
            ExtractError::InvalidRoot("domain53_faction_member_overflow"),
        )?;
        let member_ptr = match mem.read_u64(member_slot_addr) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => return Ok(None),
        };
        let Some(&accepted_tag) = accepted_country_by_ptr.get(&member_ptr) else {
            return Ok(None);
        };
        match mem.read_u64(member_ptr) {
            Ok(vtable) if vtable == expected_country_vtable => {}
            _ => return Ok(None),
        }
        let member_tag = match mem.read_u32(add_addr(member_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        {
            Ok(value) if valid_tag_db_id(value) && value == accepted_tag => value,
            _ => return Ok(None),
        };
        if member_slot == 0 {
            leader_tag_db_id = member_tag;
        }
        member_values.push((member_ptr, member_tag));
        member_tag_db_ids.push(member_tag);
    }
    if leader_tag_db_id == 0 {
        return Ok(None);
    }

    let name_addr = add_addr(faction_ptr, profile::DOMAIN53_FACTION_NAME)?;
    let Some(name_before) = read_msvc_string_snapshot(mem, name_addr, 128) else {
        return Ok(None);
    };

    member_tag_db_ids.sort_unstable();
    member_tag_db_ids.dedup();
    if member_tag_db_ids.len() != member_count as usize
        || !member_tag_db_ids.contains(&leader_tag_db_id)
    {
        return Ok(None);
    }

    if mem.read_u64(slot_addr).ok() != Some(faction_ptr)
        || mem
            .read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_ID)?)
            .ok()
            != Some(raw_id)
        || mem
            .read_u32(add_addr(
                faction_ptr,
                profile::DOMAIN53_FACTION_MEMBERS_SIZE,
            )?)
            .ok()
            != Some(member_count)
        || mem
            .read_u64(add_addr(faction_ptr, profile::DOMAIN53_FACTION_MEMBERS)?)
            .ok()
            != Some(members_base)
    {
        return Ok(None);
    }
    for (member_slot, (expected_ptr, expected_tag)) in member_values.into_iter().enumerate() {
        let member_slot_addr =
            members_base
                .checked_add((member_slot as u64) * 8)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_faction_member_overflow",
                ))?;
        if mem.read_u64(member_slot_addr).ok() != Some(expected_ptr)
            || mem.read_u64(expected_ptr).ok() != Some(expected_country_vtable)
            || mem
                .read_u32(add_addr(expected_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
                .ok()
                != Some(expected_tag)
        {
            return Ok(None);
        }
    }
    let Some(name_after) = read_msvc_string_snapshot(mem, name_addr, 128) else {
        return Ok(None);
    };
    if name_after != name_before {
        return Ok(None);
    }
    let name_hash = match name_before.value {
        StringRead::Value(value) => crate::compact::stable_hash32(&value),
        StringRead::Missing => crate::compact::stable_hash32("faction_name_unknown"),
        StringRead::Invalid => crate::compact::stable_hash32("faction_name_invalid_utf8"),
    };

    Ok(Some(FactionRow {
        faction_id,
        faction_type,
        name_hash,
        leader_tag_db_id,
        member_tag_db_ids,
    }))
}

fn resolve_country_acceptance_oracle<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
) -> Result<CountryAcceptanceOracle, ExtractError> {
    const COUNTRY_TABLE_HEADER_LEN: usize = 16;
    let country_header_addr = add_addr(game_state, profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE)?;
    let country_header = mem.read_bytes(country_header_addr, COUNTRY_TABLE_HEADER_LEN)?;
    if country_header.len() != COUNTRY_TABLE_HEADER_LEN {
        return Err(ExtractError::MemoryRead {
            address: country_header_addr,
            len: COUNTRY_TABLE_HEADER_LEN,
        });
    }
    let country_table = u64::from_le_bytes(country_header[0..8].try_into().unwrap());
    let country_allocation = u32::from_le_bytes(country_header[8..12].try_into().unwrap());
    let country_count = u32::from_le_bytes(country_header[12..16].try_into().unwrap());
    if country_count == 0 || country_count > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS {
        return Err(ExtractError::InvalidCount {
            name: "domain53_country_count",
            value: country_count,
        });
    }
    if country_allocation < country_count
        || country_allocation > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS
    {
        return Err(ExtractError::InvalidCount {
            name: stringify!(domain53_country_allocation),
            value: country_allocation,
        });
    }
    if !is_heap_ptr(country_table) {
        return Err(ExtractError::InvalidRoot(
            "domain53_country_table_unavailable",
        ));
    }

    let expected_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let allocation_len =
        (country_count as usize)
            .checked_mul(8)
            .ok_or(ExtractError::InvalidRoot(stringify!(
                domain53_country_table_overflow
            )))?;
    let allocation_bytes = mem
        .read_bytes(country_table, allocation_len)
        .ok()
        .filter(|bytes| bytes.len() == allocation_len);
    let allocation_capture_complete = allocation_bytes.is_some();
    let mut slot_allocation = Vec::with_capacity(country_count as usize);
    let mut tag_slot_counts = BTreeMap::<u32, u32>::new();
    for slot in 0..country_count {
        let slot_addr = country_table
            .checked_add(u64::from(slot) * 8)
            .ok_or(ExtractError::InvalidRoot("domain53_country_table_overflow"))?;
        let unit_pointer = allocation_bytes.as_ref().map(|bytes| {
            let offset = slot as usize * 8;
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
        });
        let country_ptr = match unit_pointer
            .map(Ok)
            .unwrap_or_else(|| mem.read_u64(slot_addr))
        {
            Ok(value) => value,
            Err(_) => {
                slot_allocation.push(CountryTableSlotFingerprint::Unreadable);
                continue;
            }
        };
        if country_ptr == 0 {
            slot_allocation.push(CountryTableSlotFingerprint::Vacant);
            continue;
        }
        if !is_heap_ptr(country_ptr) {
            slot_allocation.push(CountryTableSlotFingerprint::Rejected { country_ptr });
            continue;
        }
        let Ok(vtable) = mem.read_u64(country_ptr) else {
            slot_allocation.push(CountryTableSlotFingerprint::Rejected { country_ptr });
            continue;
        };
        if vtable != expected_vtable {
            slot_allocation.push(CountryTableSlotFingerprint::Rejected { country_ptr });
            continue;
        }
        let Ok(tag_db_id) = mem.read_u32(add_addr(country_ptr, profile::DOMAIN53_OBJ_TAG_DB_ID)?)
        else {
            slot_allocation.push(CountryTableSlotFingerprint::Rejected { country_ptr });
            continue;
        };
        if !valid_tag_db_id(tag_db_id) {
            slot_allocation.push(CountryTableSlotFingerprint::Rejected { country_ptr });
            continue;
        }
        let count = tag_slot_counts.entry(tag_db_id).or_default();
        *count = count.saturating_add(1);
        slot_allocation.push(CountryTableSlotFingerprint::Accepted {
            country_ptr,
            tag_db_id,
        });
    }
    let mut roots = Vec::new();
    let mut vacant_slot_count = 0u32;
    let mut ambiguous_or_rejected_count = u32::from(!allocation_capture_complete);
    let mut duplicate_tag_slot_count = 0u32;
    for slot in &mut slot_allocation {
        match slot.clone() {
            CountryTableSlotFingerprint::Unreadable
            | CountryTableSlotFingerprint::Rejected { .. } => {
                ambiguous_or_rejected_count = ambiguous_or_rejected_count.saturating_add(1);
            }
            CountryTableSlotFingerprint::Vacant => {
                vacant_slot_count = vacant_slot_count.saturating_add(1);
            }
            CountryTableSlotFingerprint::Accepted {
                country_ptr,
                tag_db_id,
            } => {
                if tag_slot_counts.get(&tag_db_id).copied().unwrap_or(0) > 1 {
                    *slot = CountryTableSlotFingerprint::AmbiguousDuplicateTag {
                        country_ptr,
                        tag_db_id,
                    };
                    ambiguous_or_rejected_count = ambiguous_or_rejected_count.saturating_add(1);
                    duplicate_tag_slot_count = duplicate_tag_slot_count.saturating_add(1);
                } else {
                    roots.push(CountryRoot {
                        tag_db_id,
                        country_ptr,
                    });
                }
            }
            CountryTableSlotFingerprint::AmbiguousDuplicateTag { .. } => unreachable!(),
        }
    }
    let header_capture_stable = mem
        .read_bytes(country_header_addr, COUNTRY_TABLE_HEADER_LEN)
        .ok()
        .is_some_and(|after| after == country_header);
    if !header_capture_stable {
        ambiguous_or_rejected_count = ambiguous_or_rejected_count.saturating_add(1);
    }
    Ok(CountryAcceptanceOracle {
        country_table,
        country_allocation,
        country_count,
        header_capture_stable,
        allocation_capture_complete,
        slot_allocation,
        accepted_countries: roots,
        vacant_slot_count,
        ambiguous_or_rejected_count,
        duplicate_tag_slot_count,
    })
}

pub fn resolve_complete_country_tag_db_ids<M: Domain53Memory>(
    mem: &M,
    game_state: u64,
) -> Result<BTreeSet<u32>, ExtractError> {
    let oracle = resolve_country_acceptance_oracle(mem, mem.module_base(), game_state)?;
    if !oracle.supports_positive_faction_witness() {
        return Err(ExtractError::Validation(
            "domain53_country_identity_authority_incomplete".into(),
        ));
    }
    Ok(oracle
        .accepted_countries
        .into_iter()
        .map(|country| country.tag_db_id)
        .collect())
}

pub fn resolve_complete_country_pointer_by_tag_db_id<M: Domain53Memory>(
    mem: &M,
    game_state: u64,
    wanted_tag_db_id: u32,
) -> Result<u64, ExtractError> {
    let oracle = resolve_country_acceptance_oracle(mem, mem.module_base(), game_state)?;
    if !oracle.supports_positive_faction_witness() {
        return Err(ExtractError::Validation(
            "domain53_country_pointer_authority_incomplete".into(),
        ));
    }
    let mut matches = oracle
        .accepted_countries
        .into_iter()
        .filter(|country| country.tag_db_id == wanted_tag_db_id);
    let selected = matches.next().ok_or(ExtractError::InvalidRoot(
        "domain53_country_pointer_tag_absent",
    ))?;
    if matches.next().is_some() {
        return Err(ExtractError::Validation(
            "domain53_country_pointer_tag_ambiguous".into(),
        ));
    }
    Ok(selected.country_ptr)
}

#[cfg(test)]
fn resolve_country_roots<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    game_state: u64,
) -> Result<Vec<CountryRoot>, ExtractError> {
    Ok(resolve_country_acceptance_oracle(mem, module_base, game_state)?.accepted_countries)
}

const PREFETCH_DIST_RELATION_DEFAULT: usize = 8;

const PREFETCH_DIST_CANDIDATES_DEFAULT: usize = 32;

const ENV_PREFETCH_DIST_RELATION: &str = "BATTLE_PLAN_PREFETCH_DIST_RELATION";

const ENV_PREFETCH_DIST_CANDIDATES: &str = "BATTLE_PLAN_PREFETCH_DIST_CANDIDATES";

fn parse_prefetch_dist(raw: Option<&str>, default: usize) -> usize {
    match raw {
        Some(s) => s.trim().parse::<usize>().unwrap_or(default),
        None => default,
    }
}

const PREFETCH_OVERRIDE_UNSET: usize = usize::MAX;

static PREFETCH_DIST_RELATION_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(PREFETCH_OVERRIDE_UNSET);

static PREFETCH_DIST_CANDIDATES_OVERRIDE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(PREFETCH_OVERRIDE_UNSET);

pub fn set_prefetch_dist_relation_override(dist: Option<u32>) {
    PREFETCH_DIST_RELATION_OVERRIDE.store(
        dist.map(|dist| dist as usize)
            .unwrap_or(PREFETCH_OVERRIDE_UNSET),
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub fn set_prefetch_dist_candidates_override(dist: Option<u32>) {
    PREFETCH_DIST_CANDIDATES_OVERRIDE.store(
        dist.map(|dist| dist as usize)
            .unwrap_or(PREFETCH_OVERRIDE_UNSET),
        std::sync::atomic::Ordering::Relaxed,
    );
}

pub fn effective_prefetch_dist_relation() -> usize {
    prefetch_dist_relation()
}

pub fn effective_prefetch_dist_candidates() -> usize {
    prefetch_dist_candidates()
}

fn prefetch_dist_relation() -> usize {
    let overridden = PREFETCH_DIST_RELATION_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if overridden != PREFETCH_OVERRIDE_UNSET {
        return overridden;
    }
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        parse_prefetch_dist(
            std::env::var(ENV_PREFETCH_DIST_RELATION).ok().as_deref(),
            PREFETCH_DIST_RELATION_DEFAULT,
        )
    })
}

fn prefetch_dist_candidates() -> usize {
    let overridden = PREFETCH_DIST_CANDIDATES_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if overridden != PREFETCH_OVERRIDE_UNSET {
        return overridden;
    }
    static CACHED: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *CACHED.get_or_init(|| {
        parse_prefetch_dist(
            std::env::var(ENV_PREFETCH_DIST_CANDIDATES).ok().as_deref(),
            PREFETCH_DIST_CANDIDATES_DEFAULT,
        )
    })
}

fn relation_slot_ptr<M: Domain53Memory>(
    mem: &M,
    table_bytes: Option<&[u8]>,
    relation_table: u64,
    target_tag_db_id: u32,
) -> Result<Result<u64, ExtractError>, ExtractError> {
    let entry_slot = relation_table
        .checked_add(u64::from(target_tag_db_id) * 8)
        .ok_or(ExtractError::InvalidRoot(
            "domain53_relation_table_overflow",
        ))?;
    if let Some(bytes) = table_bytes {
        let lo = (target_tag_db_id as usize) * 8;
        if let Some(slot) = bytes.get(lo..lo + 8) {
            let value = u64::from_le_bytes(slot.try_into().expect("relation slot u64"));
            return Ok(Ok(value));
        }
    }
    Ok(mem.read_u64(entry_slot))
}

fn read_relation_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    read_relation_rows_for_mode(
        mem,
        module_base,
        current_game_hour,
        countries,
        acc,
        relation_read_mode(),
    )
}

fn read_relation_rows_for_mode<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
    mode: Domain53RelationReadMode,
) -> Result<(), ExtractError> {
    match mode {
        Domain53RelationReadMode::Offsets => read_relation_rows_impl(
            mem,
            module_base,
            current_game_hour,
            countries,
            acc,
            RelationReadPass::Offsets,
        ),
        Domain53RelationReadMode::Scan => read_relation_rows_impl(
            mem,
            module_base,
            current_game_hour,
            countries,
            acc,
            RelationReadPass::Scan,
        ),
        Domain53RelationReadMode::Shadow => {
            let mut offsets = RowAccumulator::default();
            let mut scan = RowAccumulator::default();
            read_relation_rows_impl(
                mem,
                module_base,
                current_game_hour,
                countries,
                &mut offsets,
                RelationReadPass::Offsets,
            )?;
            read_relation_rows_impl(
                mem,
                module_base,
                current_game_hour,
                countries,
                &mut scan,
                RelationReadPass::Scan,
            )?;
            let mismatches = relation_shadow_mismatch_count(&offsets, &scan);
            acc.merge_relation_rows_from(offsets);
            acc.relation_shadow_mismatches =
                acc.relation_shadow_mismatches.saturating_add(mismatches);
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RelationReadPass {
    Offsets,
    Scan,
}

fn relation_shadow_mismatch_count(offsets: &RowAccumulator, scan: &RowAccumulator) -> u64 {
    let mut count = 0u64;
    if offsets.rows != scan.rows {
        count = count.saturating_add(1);
    }
    if offsets.skipped_rows != scan.skipped_rows {
        count = count.saturating_add(1);
    }
    if offsets.observed_rows != scan.observed_rows {
        count = count.saturating_add(1);
    }
    count
}

fn read_relation_rows_impl<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
    pass: RelationReadPass,
) -> Result<(), ExtractError> {
    let relation_skipped_before = acc.skipped_rows;
    let relation_vtables = relation_vtable_kinds(module_base)?;
    if relation_vtables.is_empty() {
        return Err(ExtractError::InvalidRoot(
            "domain53_relation_vtable_map_empty",
        ));
    }
    let scan_bounds: Option<(u64, u64)> = if pass == RelationReadPass::Scan {
        relation_vtables
            .keys()
            .next()
            .copied()
            .zip(relation_vtables.keys().next_back().copied())
    } else {
        None
    };
    let mut seen_flags = BTreeSet::new();
    let mut seen_subjects = BTreeSet::new();

    let pair_estimate = countries
        .len()
        .saturating_mul(countries.len().saturating_sub(1))
        .min(1 << 20);
    acc.rows.opinions.reserve(pair_estimate);
    // over-reservation, allocated and released every frame.

    let max_target_tag = countries
        .iter()
        .map(|country| country.tag_db_id)
        .max()
        .unwrap_or(0);
    let table_len: Option<usize> = u64::from(max_target_tag)
        .checked_add(1)
        .and_then(|slots| slots.checked_mul(8))
        .and_then(|len| usize::try_from(len).ok());

    let prefetch_dist_relation = prefetch_dist_relation();
    let mut relation_hot_buf = [0u8; RELATION_HOT_FIELD_LEN];
    let mut relation_scan_buf = if pass == RelationReadPass::Scan {
        Some([0u8; profile::DOMAIN53_RELATION_SCAN_BYTES])
    } else {
        None
    };

    for src in countries {
        let manager_addr = add_addr(src.country_ptr, profile::DOMAIN53_RELATION_MANAGER)?;
        let manager = match mem.read_u64(manager_addr) {
            Ok(value) if is_heap_ptr(value) => value,
            Ok(0) => continue,
            _ => {
                acc.skip_relation_root();
                continue;
            }
        };
        let relation_table =
            match mem.read_u64(add_addr(manager, profile::DOMAIN53_RELATION_TABLE)?) {
                Ok(value) if is_heap_ptr(value) => value,
                _ => {
                    acc.skip_relation_root();
                    continue;
                }
            };
        let table_bytes = table_len.and_then(|len| mem.read_bytes(relation_table, len).ok());
        for (target_index, target) in countries.iter().enumerate() {
            if prefetch_dist_relation != 0 {
                if let Some(bytes) = table_bytes.as_deref() {
                    if let Some(future) = target_index
                        .checked_add(prefetch_dist_relation)
                        .and_then(|i| countries.get(i))
                    {
                        let lo = (future.tag_db_id as usize) * 8;
                        if let Some(slot) = bytes.get(lo..lo + 8) {
                            let future_entry =
                                u64::from_le_bytes(slot.try_into().expect("relation slot u64"));
                            for hint_offset in RELATION_PREFETCH_HINT_OFFSETS {
                                if let Some(line) = future_entry.checked_add(hint_offset as u64) {
                                    prefetch_read(line);
                                }
                            }
                        }
                    }
                }
            }
            if src.tag_db_id == target.tag_db_id {
                continue;
            }
            let relation_entry = match relation_slot_ptr(
                mem,
                table_bytes.as_deref(),
                relation_table,
                target.tag_db_id,
            )? {
                Ok(value) if is_heap_ptr(value) => value,
                Ok(0) => continue,
                // Zero is the established empty encoding; any other non-heap
                // value is not PROVEN emptiness (it may be a corrupted live
                // slot), so it refuses by name as a skip.
                _ => {
                    acc.skip_relation_slot();
                    continue;
                }
            };
            let relation_hot =
                read_relation_hot_fields(mem, relation_entry, &mut relation_hot_buf).ok();
            let opinion = read_relation_opinion_with_retry(mem, relation_entry, relation_hot);
            match opinion {
                Ok(opinion) => {
                    acc.rows.opinions.push(OpinionRow {
                        src_tag_db_id: src.tag_db_id,
                        target_tag_db_id: target.tag_db_id,
                        opinion,
                    });
                    acc.observe();
                }
                Err(_) => {
                    acc.skip_relation_opinion();
                    continue;
                }
            }

            let subject_ptr_read = match relation_hot {
                Some(fields) => Ok(fields.subject_ptr),
                None => add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBJECT_PTR)
                    .and_then(|addr| mem.read_u64(addr)),
            };
            match subject_ptr_read {
                Ok(subject_ptr) if is_heap_ptr(subject_ptr) => {
                    read_subject_relation_from_ptr(
                        mem,
                        module_base,
                        subject_ptr,
                        src.tag_db_id,
                        target.tag_db_id,
                        target.country_ptr,
                        current_game_hour,
                        acc,
                        &mut seen_subjects,
                        &mut seen_flags,
                    );
                }
                Ok(_) => {}
                Err(_) => acc.skip_relation_subject(),
            }

            let subrel_count_read = read_relation_subrel_count_with_retry(mem, relation_entry);
            match subrel_count_read {
                Ok(count) => read_relation_container_with_count(
                    mem,
                    relation_entry,
                    count,
                    &relation_vtables,
                    src.tag_db_id,
                    target.tag_db_id,
                    current_game_hour,
                    acc,
                    &mut seen_flags,
                ),
                Err(_) => acc.skip_relation_container(),
            }
            if let (Some(scan_buf), Some((vtable_min, vtable_max))) =
                (relation_scan_buf.as_mut(), scan_bounds)
            {
                legacy_scan_relation_blob(
                    mem,
                    relation_entry,
                    scan_buf,
                    &relation_vtables,
                    vtable_min,
                    vtable_max,
                    src.tag_db_id,
                    target.tag_db_id,
                    acc,
                    &mut seen_flags,
                );
            }
        }
    }
    let subject_keys = acc
        .rows
        .subjects
        .iter()
        .map(|row| (row.overlord_tag_db_id, row.subject_tag_db_id))
        .collect::<BTreeSet<_>>();
    let status_keys = acc
        .rows
        .subject_autonomy_statuses
        .iter()
        .map(|row| (row.overlord_tag_db_id, row.subject_tag_db_id))
        .collect::<BTreeSet<_>>();
    acc.rows.subject_autonomy_status_complete = acc.skipped_rows == relation_skipped_before
        && subject_keys.len() == acc.rows.subjects.len()
        && status_keys.len() == acc.rows.subject_autonomy_statuses.len()
        && subject_keys == status_keys;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn legacy_scan_relation_blob<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    scan_buf: &mut [u8; profile::DOMAIN53_RELATION_SCAN_BYTES],
    relation_vtables: &BTreeMap<u64, u16>,
    vtable_min: u64,
    vtable_max: u64,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    acc: &mut RowAccumulator,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    if mem.read_into(relation_entry, scan_buf).is_err() {
        acc.skip_relation_container();
        return;
    }
    for bytes in scan_buf.chunks_exact(8) {
        let qword = u64::from_le_bytes(bytes.try_into().expect("qword scan slice"));
        if qword < vtable_min || qword > vtable_max {
            continue;
        }
        let Some(flag_kind) = relation_vtables.get(&qword).copied() else {
            continue;
        };
        acc.relflag_offsets.record_kind_count(flag_kind);
        if flag_kind == REL_KIND_PUPPET {
            continue;
        }
        let key = (src_tag_db_id, target_tag_db_id, flag_kind);
        if seen_flags.insert(key) {
            acc.rows.relation_flags.push(RelationFlagRow {
                src_tag_db_id,
                target_tag_db_id,
                flag_kind,
                directed: true,
            });
            acc.rows.state_events.push(state_event(
                src_tag_db_id,
                Some(target_tag_db_id),
                STATE_EVENT_SOURCE_RELATION_FLAG,
                &format!("relation_flag_kind_{flag_kind}"),
                "directed_true",
                i64::from(flag_kind),
            ));
            acc.observe();
        }
    }
}

fn read_subject_autonomy_status<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    overlord_tag_db_id: u32,
    subject_tag_db_id: u32,
    subject_country_ptr: u64,
) -> Option<SubjectAutonomyStatusRow> {
    let diplomacy = mem
        .read_u64(add_addr(subject_country_ptr, profile::DOMAIN53_RELATION_MANAGER).ok()?)
        .ok()?;
    if !is_heap_ptr(diplomacy) {
        return None;
    }
    let status_slot = add_addr(
        diplomacy,
        profile::DOMAIN53_DIPLOMACY_CURRENT_AUTONOMY_STATUS,
    )
    .ok()?;
    let status_ptr = mem.read_u64(status_slot).ok()?;
    if !is_heap_ptr(status_ptr) {
        return None;
    }
    let before = mem
        .read_bytes(status_ptr, profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_SIZE)
        .ok()?;
    let after = mem
        .read_bytes(status_ptr, profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_SIZE)
        .ok()?;
    if before.len() != profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_SIZE
        || before != after
        || mem.read_u64(status_slot).ok()? != status_ptr
    {
        return None;
    }
    let u64_at = |offset: usize| {
        before
            .get(offset..offset + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u64::from_le_bytes)
    };
    let i64_at = |offset: usize| {
        before
            .get(offset..offset + 8)
            .and_then(|bytes| bytes.try_into().ok())
            .map(i64::from_le_bytes)
    };
    let u32_at = |offset: usize| {
        before
            .get(offset..offset + 4)
            .and_then(|bytes| bytes.try_into().ok())
            .map(u32::from_le_bytes)
    };
    let expected_vtable = add_addr(
        module_base,
        profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_VTABLE_RVA,
    )
    .ok()?;
    if u64_at(0)? != expected_vtable
        || u64_at(profile::DOMAIN53_CURRENT_AUTONOMY_SUBJECT_COUNTRY)? != subject_country_ptr
    {
        return None;
    }
    let current_state = u64_at(profile::DOMAIN53_CURRENT_AUTONOMY_STATE)?;
    let previous_state = u64_at(profile::DOMAIN53_CURRENT_AUTONOMY_PREVIOUS_STATE)?;
    let next_state = u64_at(profile::DOMAIN53_CURRENT_AUTONOMY_NEXT_STATE)?;
    if !is_heap_ptr(current_state)
        || (previous_state != 0 && !is_heap_ptr(previous_state))
        || (next_state != 0 && !is_heap_ptr(next_state))
        || previous_state == current_state
        || next_state == current_state
        || (previous_state != 0 && previous_state == next_state)
    {
        return None;
    }
    let hash_state = |state_ptr: u64| match read_msvc_string(
        mem,
        add_addr(state_ptr, profile::DOMAIN53_SUBJECT_STATE_NAME).ok()?,
        96,
    ) {
        StringRead::Value(value) if !value.is_empty() => {
            Some(crate::compact::stable_hash32(&value))
        }
        StringRead::Value(_) | StringRead::Invalid | StringRead::Missing => None,
    };
    let current_state_hash = hash_state(current_state)?;
    let previous_state_hash = (previous_state != 0)
        .then(|| hash_state(previous_state))
        .flatten();
    let next_state_hash = (next_state != 0).then(|| hash_state(next_state)).flatten();
    if (previous_state != 0 && previous_state_hash.is_none())
        || (next_state != 0 && next_state_hash.is_none())
    {
        return None;
    }
    let current_score_raw = i64_at(profile::DOMAIN53_CURRENT_AUTONOMY_SCORE)?;
    let lower_score_raw = i64_at(profile::DOMAIN53_CURRENT_AUTONOMY_LOWER_SCORE)?;
    let upper_score_raw = i64_at(profile::DOMAIN53_CURRENT_AUTONOMY_UPPER_SCORE)?;
    if lower_score_raw >= upper_score_raw {
        return None;
    }
    let forward = current_score_raw.checked_sub(lower_score_raw)?.max(0);
    let remaining = upper_score_raw.checked_sub(current_score_raw)?.max(0);
    let denominator = forward.checked_add(remaining)?;
    let ratio_i64 = forward
        .checked_mul(i64::from(FIXED_SCALE_RAW))?
        .checked_div(denominator)?;
    let ratio_raw = i32::try_from(ratio_i64).ok()?;
    if !(0..=FIXED_SCALE_RAW as i32).contains(&ratio_raw) {
        return None;
    }
    Some(SubjectAutonomyStatusRow {
        overlord_tag_db_id,
        subject_tag_db_id,
        current_state_hash,
        previous_state_hash,
        next_state_hash,
        ratio_raw,
        current_score_raw,
        lower_score_raw,
        upper_score_raw,
        last_modified_date_raw: u32_at(profile::DOMAIN53_CURRENT_AUTONOMY_LAST_MODIFIED_DATE)?,
    })
}

#[allow(clippy::too_many_arguments)]
fn read_subject_relation_from_ptr<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    subject_ptr: u64,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    target_country_ptr: u64,
    current_game_hour: u32,
    acc: &mut RowAccumulator,
    seen_subjects: &mut BTreeSet<(u32, u32)>,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    let expected_vtable = match add_addr(module_base, profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA) {
        Ok(value) => value,
        Err(_) => return,
    };
    match mem.read_u64(subject_ptr) {
        Ok(vtable) if vtable == expected_vtable => {}
        Ok(_) => return,
        Err(_) => {
            acc.skip_relation_subject();
            return;
        }
    }

    let read = (|| -> Result<SubjectRow, ExtractError> {
        let overlord = mem.read_u32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_OVERLORD)?)?;
        let subject = mem.read_u32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_SUBJECT)?)?;
        if !valid_tag_db_id(overlord) || !valid_tag_db_id(subject) || overlord == subject {
            return Err(ExtractError::InvalidRoot("domain53_subject_tag_invalid"));
        }
        if src_tag_db_id == subject && target_tag_db_id == overlord {
            return Err(ExtractError::InvalidRoot("domain53_subject_direction_skip"));
        }
        if src_tag_db_id != overlord || target_tag_db_id != subject {
            return Err(ExtractError::InvalidRoot(
                "domain53_subject_direction_mismatch",
            ));
        }
        let state_db = mem.read_u64(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_STATE_DB)?)?;
        let state_hash = if is_heap_ptr(state_db) {
            match read_msvc_string(
                mem,
                add_addr(state_db, profile::DOMAIN53_SUBJECT_STATE_NAME)?,
                96,
            ) {
                StringRead::Value(value) => crate::compact::stable_hash32(&value),
                StringRead::Invalid => {
                    acc.skip_relation_subject();
                    crate::compact::stable_hash32("autonomy_state_invalid_utf8")
                }
                StringRead::Missing => crate::compact::stable_hash32("autonomy_state_unknown"),
            }
        } else {
            crate::compact::stable_hash32("autonomy_state_unknown")
        };
        let ratio_i64 = mem.read_i64(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_RATIO)?)?;
        if !(0..=i64::from(crate::compact::FIXED_SCALE_RAW)).contains(&ratio_i64) {
            return Err(ExtractError::InvalidRoot("domain53_subject_ratio_invalid"));
        }
        let ratio = i32::try_from(ratio_i64)
            .map_err(|_| ExtractError::InvalidRoot("domain53_subject_ratio_invalid"))?;
        let start_date_raw =
            mem.read_i32(add_addr(subject_ptr, profile::DOMAIN53_SUBJECT_START_DATE)?)?;
        let clock_ceiling = i64::from(current_game_hour) + 24;
        if start_date_raw <= 0 || i64::from(start_date_raw) > clock_ceiling {
            return Err(ExtractError::InvalidRoot(
                "domain53_subject_start_date_invalid",
            ));
        }
        Ok(SubjectRow {
            overlord_tag_db_id: overlord,
            subject_tag_db_id: subject,
            autonomy_state_hash: state_hash,
            autonomy_ratio_raw: ratio,
            start_date_raw,
        })
    })();

    match read {
        Ok(row) => {
            if seen_subjects.insert((row.overlord_tag_db_id, row.subject_tag_db_id)) {
                if let Some(status) = read_subject_autonomy_status(
                    mem,
                    module_base,
                    row.overlord_tag_db_id,
                    row.subject_tag_db_id,
                    target_country_ptr,
                ) {
                    acc.rows.subject_autonomy_statuses.push(status);
                }
                let flag_key = (
                    row.overlord_tag_db_id,
                    row.subject_tag_db_id,
                    REL_KIND_PUPPET,
                );
                if seen_flags.insert(flag_key) {
                    acc.rows.relation_flags.push(RelationFlagRow {
                        src_tag_db_id: row.overlord_tag_db_id,
                        target_tag_db_id: row.subject_tag_db_id,
                        flag_kind: REL_KIND_PUPPET,
                        directed: true,
                    });
                    acc.rows.state_events.push(state_event(
                        row.overlord_tag_db_id,
                        Some(row.subject_tag_db_id),
                        STATE_EVENT_SOURCE_RELATION_FLAG,
                        &format!("relation_flag_kind_{REL_KIND_PUPPET}"),
                        "directed_true",
                        i64::from(REL_KIND_PUPPET),
                    ));
                    acc.observe();
                }
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
        Err(ExtractError::InvalidRoot("domain53_subject_direction_skip")) => {}
        Err(_) => acc.skip_relation_subject(),
    }
}

fn relation_kind_emitted_by_d53(kind: u16) -> bool {
    !matches!(
        kind,
        REL_KIND_PUPPET | REL_KIND_MARKET_ACCESS_RIGHTS | REL_KIND_AIR_BASE_ACCESS
    )
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
fn read_relation_container<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    relation_vtables: &BTreeMap<u64, u16>,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    current_game_hour: u32,
    acc: &mut RowAccumulator,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    let count = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_COUNT)
        .and_then(|addr| mem.read_i32(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip_relation_container();
            return;
        }
    };
    read_relation_container_with_count(
        mem,
        relation_entry,
        count,
        relation_vtables,
        src_tag_db_id,
        target_tag_db_id,
        current_game_hour,
        acc,
        seen_flags,
    );
}

#[allow(clippy::too_many_arguments)]
fn read_relation_container_with_count<M: Domain53Memory>(
    mem: &M,
    relation_entry: u64,
    count: i32,
    relation_vtables: &BTreeMap<u64, u16>,
    src_tag_db_id: u32,
    target_tag_db_id: u32,
    current_game_hour: u32,
    acc: &mut RowAccumulator,
    seen_flags: &mut BTreeSet<(u32, u32, u16)>,
) {
    if count == 0 {
        return;
    }
    if !(1..=profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT).contains(&count) {
        acc.skip_relation_container();
        return;
    }
    let begin = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_BEGIN)
        .and_then(|addr| mem.read_u64(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip_relation_container();
            return;
        }
    };
    if !is_heap_ptr(begin) {
        acc.skip_relation_container();
        return;
    }
    let cap = match add_addr(relation_entry, profile::DOMAIN53_RELATION_SUBREL_CAP)
        .and_then(|addr| mem.read_i32(addr))
    {
        Ok(value) => value,
        Err(_) => {
            acc.skip_relation_container();
            return;
        }
    };
    if cap < count || cap > profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT {
        acc.skip_relation_container();
        return;
    }
    let array_len = (count as usize) * 8;
    let mut array_buf = [0u8; profile::DOMAIN53_RELATION_SUBREL_MAX_COUNT as usize * 8];
    if mem.read_into(begin, &mut array_buf[..array_len]).is_err() {
        acc.skip_relation_container();
        return;
    }
    for chunk in array_buf[..array_len].chunks_exact(8) {
        let sub = u64::from_le_bytes(chunk.try_into().expect("8-byte pointer slot"));
        if !is_heap_ptr(sub) {
            continue;
        }
        let vptr = match mem.read_u64(sub) {
            Ok(value) => value,
            Err(_) => {
                acc.skip_relation_container();
                continue;
            }
        };
        let kind = match relation_vtables.get(&vptr).copied() {
            Some(value) => value,
            None => continue,
        };
        acc.relflag_offsets.record_kind_count(kind);
        if !relation_kind_emitted_by_d53(kind) {
            continue;
        }
        let first = match add_addr(sub, profile::DOMAIN53_REL_BASE_FIRST)
            .and_then(|addr| mem.read_u32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip_relation_container();
                continue;
            }
        };
        let second = match add_addr(sub, profile::DOMAIN53_REL_BASE_SECOND)
            .and_then(|addr| mem.read_u32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip_relation_container();
                continue;
            }
        };
        if !valid_tag_db_id(first) || !valid_tag_db_id(second) || first == second {
            acc.skip_relation_container();
            continue;
        }
        let matches_bucket = (first == src_tag_db_id && second == target_tag_db_id)
            || (first == target_tag_db_id && second == src_tag_db_id);
        if !matches_bucket {
            acc.skip_relation_container();
            continue;
        }
        let start_raw = match add_addr(sub, profile::DOMAIN53_REL_BASE_START_DATE)
            .and_then(|addr| mem.read_i32(addr))
        {
            Ok(value) => value,
            Err(_) => {
                acc.skip_relation_container();
                continue;
            }
        };
        let clock_ceiling = i64::from(current_game_hour) + 24;
        if start_raw == profile::DOMAIN53_REL_DATE_SENTINEL
            || start_raw <= 0
            || i64::from(start_raw) > clock_ceiling
        {
            acc.skip_relation_container();
            continue;
        }
        let end_raw = match add_addr(sub, profile::DOMAIN53_REL_BASE_END_DATE)
            .and_then(|addr| mem.read_i32(addr))
        {
            Ok(value) if value != profile::DOMAIN53_REL_DATE_SENTINEL && value > 0 => Some(value),
            Ok(_) => None,
            Err(_) => {
                acc.skip_relation_container();
                continue;
            }
        };
        if !seen_flags.insert((first, second, kind)) {
            continue;
        }
        acc.rows.relation_flags.push(RelationFlagRow {
            src_tag_db_id: first,
            target_tag_db_id: second,
            flag_kind: kind,
            directed: true,
        });
        acc.rows.state_events.push(state_event(
            first,
            Some(second),
            STATE_EVENT_SOURCE_RELATION_FLAG,
            &format!("relation_flag_kind_{kind}"),
            "directed_true",
            i64::from(kind),
        ));
        acc.rows.state_events.push(state_event(
            first,
            Some(second),
            STATE_EVENT_SOURCE_RELATION_FLAG,
            &format!("relation_flag_start_{kind}"),
            "start_date_raw",
            i64::from(start_raw),
        ));
        if let Some(end) = end_raw {
            acc.rows.state_events.push(state_event(
                first,
                Some(second),
                STATE_EVENT_SOURCE_RELATION_FLAG,
                &format!("relation_flag_end_{kind}"),
                "end_date_raw",
                i64::from(end),
            ));
        }
        acc.observe();
    }
}

fn read_incoming_action_queue_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let action_vtable = add_addr(module_base, profile::DOMAIN53_INCOMING_ACTION_VTABLE_RVA)?;
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut seen_actions = BTreeSet::new();

    for recipient in countries {
        let queue = match mem.read_u64(add_addr(
            recipient.country_ptr,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE,
        )?) {
            Ok(value) if is_heap_ptr(value) => value,
            Ok(_) => continue,
            Err(_) => {
                acc.skip_incoming_action();
                continue;
            }
        };
        let packed = match mem.read_u64(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP,
        )?) {
            Ok(value) => value,
            Err(_) => {
                acc.skip_incoming_action();
                continue;
            }
        };
        let size = (packed & 0xFFFF_FFFF) as u32;
        let cap = (packed >> 32) as u32;
        // A queue whose packed header is invalid advertises rows this read
        // cannot delimit - a genuine omission, refused by name (skip), never
        // reclassified as noise.
        if size > cap || cap > profile::DOMAIN53_MAX_INCOMING_ACTION_ROWS {
            acc.skip_incoming_action();
            continue;
        }
        if size == 0 {
            continue;
        }
        let owner_guard = match mem.read_u32(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD,
        )?) {
            Ok(value) => value,
            Err(_) => {
                acc.skip_incoming_action();
                continue;
            }
        };
        if owner_guard != recipient.tag_db_id {
            acc.skip_incoming_action();
            continue;
        }
        let data = match mem.read_u64(add_addr(
            queue,
            profile::DOMAIN53_INCOMING_ACTION_QUEUE_DATA,
        )?) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => {
                acc.skip_incoming_action();
                continue;
            }
        };
        for index in 0..size {
            let slot = data
                .checked_add(u64::from(index) * 8)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_incoming_action_queue_overflow",
                ))?;
            match read_incoming_action_row(
                mem,
                slot,
                recipient.tag_db_id,
                action_vtable,
                current_game_hour,
                &valid_tags,
            ) {
                Ok(row) if seen_actions.insert(row.action_id) => {
                    acc.rows.incoming_actions.push(row);
                    acc.observe();
                }
                Ok(_) => {
                    // A second sighting of an action id already emitted this
                    // frame. Suppression is load-bearing for the wire (the
                    // decoder refuses duplicate ids), but whether a colliding
                    // id is the SAME delivery or a distinct row cannot be
                    // proven offline, so this keeps its refuse-by-name skip.
                    acc.skip_incoming_action();
                    continue;
                }
                Err(_) => {
                    acc.skip_incoming_action();
                    continue;
                }
            }
        }
    }
    acc.rows
        .incoming_actions
        .sort_by_key(|row| (row.recipient_tag_db_id, row.action_id));
    Ok(())
}

fn read_incoming_action_row<M: Domain53Memory>(
    mem: &M,
    slot: u64,
    expected_recipient: u32,
    action_vtable: u64,
    current_game_hour: u32,
    valid_tags: &BTreeSet<u32>,
) -> Result<IncomingActionQueueRow, ExtractError> {
    let action = mem.read_u64(slot)?;
    if !is_heap_ptr(action) || mem.read_u64(action)? != action_vtable {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_vtable_invalid",
        ));
    }
    let handle = mem.read_u64(add_addr(action, profile::DOMAIN53_INCOMING_ACTION_HANDLE)?)?;
    let handle_type = (handle & 0xFFFF_FFFF) as u32;
    if handle_type != profile::DOMAIN53_INCOMING_ACTION_HANDLE_TYPE {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_handle_type_invalid",
        ));
    }
    let action_id = (handle >> 32) as u32;
    if action_id == 0 || action_id > MAX_OPERATION_ID {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_id_invalid",
        ));
    }
    let body = mem.read_u64(add_addr(action, profile::DOMAIN53_INCOMING_ACTION_BODY)?)?;
    if !is_heap_ptr(body) {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_body_invalid",
        ));
    }
    let action_type = mem.read_i32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_BODY_TYPE)?)?;
    let actor = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_ACTOR)?)?;
    let original_actor = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR,
    )?)?;
    let recipient = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_RECIPIENT)?)?;
    let original_recipient = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT,
    )?)?;
    let date_hours = mem.read_u32(add_addr(body, profile::DOMAIN53_INCOMING_ACTION_DATE)?)?;
    let last_command_date_hours = mem.read_u32(add_addr(
        body,
        profile::DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE,
    )?)?;
    if actor != original_actor
        || recipient != original_recipient
        || recipient != expected_recipient
        || !valid_tag_db_id(actor)
        || !valid_tag_db_id(recipient)
        || !valid_tags.contains(&actor)
        || !valid_tags.contains(&recipient)
    {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_tag_guard_invalid",
        ));
    }
    if date_hours > current_game_hour
        || last_command_date_hours > current_game_hour
        || date_hours > MAX_ACTION_DATE_HOURS
        || last_command_date_hours > MAX_ACTION_DATE_HOURS
    {
        return Err(ExtractError::InvalidRoot(
            "domain53_incoming_action_date_invalid",
        ));
    }
    Ok(IncomingActionQueueRow {
        recipient_tag_db_id: recipient,
        actor_tag_db_id: actor,
        action_id,
        action_type,
        date_hours,
        last_command_date_hours,
    })
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
                acc.skip_agency();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(agency_ptr) else {
            acc.skip_agency();
            continue;
        };
        if vtable != expected_vtable {
            acc.skip_agency();
            continue;
        }
        let defense_raw =
            match mem.read_i32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_DEFENSE)?) {
                Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
                _ => {
                    acc.skip_agency();
                    continue;
                }
            };
        let upgrade_progress_raw = match mem.read_i32(add_addr(
            agency_ptr,
            profile::DOMAIN53_AGENCY_UPGRADE_PROGRESS,
        )?) {
            Ok(value) if (-MAX_FIXED_RAW..=MAX_FIXED_RAW).contains(&value) => value,
            _ => {
                acc.skip_agency();
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
                acc.skip_agency();
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
                acc.skip_agency();
                0
            }
            StringRead::Missing => 0,
        };
        let max_operatives =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_E4)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip_agency();
                    continue;
                }
            };
        let usable_slots =
            match mem.read_u32(add_addr(agency_ptr, profile::DOMAIN53_AGENCY_CAPACITY_F0)?) {
                Ok(value) if value <= MAX_AGENCY_CAPACITY_CLUSTER_VALUE => value,
                _ => {
                    acc.skip_agency();
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
        acc.skip_agency();
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
    countries: &[CountryRoot],
    agency_roots: &[AgencyRootRead],
    acc: &mut RowAccumulator,
) -> Result<(), ExtractError> {
    let crypto_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_VTABLE_RVA)?;
    let record_vtable = add_addr(module_base, profile::DOMAIN53_CRYPTOLOGY_RECORD_VTABLE_RVA)?;
    let valid_owners = agency_roots
        .iter()
        .map(|root| root.owner_tag_db_id)
        .collect::<BTreeSet<_>>();
    let valid_targets = countries
        .iter()
        .map(|root| root.tag_db_id)
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
                acc.skip_cryptology();
                continue;
            }
        };
        let Ok(vtable) = mem.read_u64(crypto_ptr) else {
            acc.skip_cryptology();
            continue;
        };
        if vtable != crypto_vtable {
            acc.skip_cryptology();
            continue;
        }
        let data = match mem.read_u64(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_DATA)?)
        {
            Ok(value) if value == 0 || is_heap_ptr(value) => value,
            _ => {
                acc.skip_cryptology();
                continue;
            }
        };
        let size = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_SIZE)?)
        {
            Ok(value) if value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip_cryptology();
                continue;
            }
        };
        let cap = match mem.read_u32(add_addr(crypto_ptr, profile::DOMAIN53_CRYPTOLOGY_MGR_CAP)?) {
            Ok(value) if value >= size && value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                acc.skip_cryptology();
                continue;
            }
        };
        let _ = cap;
        if size == 0 {
            continue;
        }
        if !is_heap_ptr(data) {
            acc.skip_cryptology();
            continue;
        }
        for index in 0..size {
            let row_addr = data
                .checked_add(u64::from(index) * profile::DOMAIN53_CRYPTOLOGY_RECORD_STRIDE as u64)
                .ok_or(ExtractError::InvalidRoot(
                    "domain53_cryptology_record_overflow",
                ))?;
            match mem.read_u64(row_addr) {
                Ok(vtable) if vtable == record_vtable => {}
                Ok(_) => continue,
                Err(_) => {
                    acc.skip_cryptology();
                    continue;
                }
            }
            let read = (|| -> Result<CryptologyRow, ExtractError> {
                let target = mem.read_u32(add_addr(
                    row_addr,
                    profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET,
                )?)?;
                if !valid_tag_db_id(target)
                    || !valid_owners.contains(&agency.owner_tag_db_id)
                    || !valid_targets.contains(&target)
                {
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
                if !(1..MAX_CRYPTO_AMOUNT_RAW).contains(&amount_raw) {
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
                Ok(_) => acc.skip_cryptology(),
                Err(_) => acc.skip_cryptology(),
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OperationDropBreakdown {
    pub stale_liveness: usize,
    pub filtered: usize,
    pub semantic_incomplete: usize,
    pub semantic_causes: Domain53RejectionCounts,
}

impl OperationDropBreakdown {
    fn reject_semantic(&mut self, cause: OperationSemanticCause) {
        self.filtered = self.filtered.saturating_add(1);
        self.semantic_incomplete = self.semantic_incomplete.saturating_add(1);
        self.semantic_causes.record_operation(cause);
    }
}

fn read_operations<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    current_game_hour: u32,
    countries: &[CountryRoot],
    valid_operative_ids: &BTreeSet<u32>,
    hits: &[u64],
) -> Result<(Vec<OperationInstanceRow>, OperationDropBreakdown), ExtractError> {
    let instance_vtable = add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA)?;
    let def_vtable = add_addr(module_base, profile::DOMAIN53_OPERATION_DEF_VTABLE_RVA)?;
    let country_vtable = add_addr(module_base, profile::DOMAIN53_COUNTRY_VTABLE_RVA)?;
    let province_vtable = add_addr(module_base, profile::CPROVINCE_VTABLE_RVA)?;
    let state_vtable = add_addr(module_base, profile::STATE_VTABLE_RVA)?;
    let valid_countries = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut assigned_globally = BTreeSet::new();
    let mut drops = OperationDropBreakdown::default();
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (hit_index, hit) in hits.iter().enumerate() {
        let hit = *hit;
        if prefetch_dist_candidates != 0 {
            if let Some(&future_hit) = hit_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| hits.get(i))
            {
                prefetch_read(future_hit);
            }
        }
        match mem.read_u64(hit) {
            Ok(vtable) if vtable == instance_vtable => {}
            Ok(_) | Err(_) => {
                drops.stale_liveness = drops.stale_liveness.saturating_add(1);
                continue;
            }
        }
        let Ok(handle) = mem.read_u64(add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_HANDLE)?)
        else {
            drops.stale_liveness = drops.stale_liveness.saturating_add(1);
            continue;
        };
        let handle_type = (handle & 0xFFFF_FFFF) as u32;
        if handle_type != profile::DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        let instance_id = (handle >> 32) as u32;
        if instance_id == 0 || instance_id > MAX_OPERATION_ID {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        if !seen.insert(instance_id) {
            drops.filtered = drops.filtered.saturating_add(1);
            continue;
        }
        let Ok(def_ptr) = mem.read_u64(add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_DEF)?)
        else {
            drops.reject_semantic(OperationSemanticCause::DefinitionPointerUnreadable);
            continue;
        };
        if !is_heap_ptr(def_ptr) || mem.read_u64(def_ptr).ok() != Some(def_vtable) {
            drops.reject_semantic(OperationSemanticCause::DefinitionObjectInvalid);
            continue;
        }
        let definition_id = match read_msvc_string(
            mem,
            add_addr(def_ptr, profile::DOMAIN53_OPERATION_DEF_TOKEN)?,
            crate::compact::MAX_OPERATION_DEFINITION_ID_LEN,
        ) {
            StringRead::Value(value) if valid_operation_definition_id(&value) => value,
            _ => {
                drops.reject_semantic(OperationSemanticCause::DefinitionTokenInvalid);
                continue;
            }
        };
        let Some(owner_country_tag_db_id) = read_operation_country_pointer(
            mem,
            add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_OWNER_COUNTRY)?,
            country_vtable,
            &valid_countries,
        ) else {
            drops.reject_semantic(OperationSemanticCause::OwnerCountryInvalid);
            continue;
        };
        let Ok(target_country_tag_db_id) = mem.read_u32(add_addr(
            hit,
            profile::DOMAIN53_OPERATION_INSTANCE_TARGET_COUNTRY_ID,
        )?) else {
            drops.reject_semantic(OperationSemanticCause::TargetCountryUnreadable);
            continue;
        };
        if !valid_countries.contains(&target_country_tag_db_id) {
            drops.reject_semantic(OperationSemanticCause::TargetCountryUnknown);
            continue;
        }
        let target_state_id = match read_operation_target_state(
            mem,
            add_addr(hit, profile::DOMAIN53_OPERATION_INSTANCE_TARGET_OBJECT)?,
            province_vtable,
            state_vtable,
        ) {
            Ok(value) => value,
            Err(()) => {
                drops.reject_semantic(OperationSemanticCause::TargetObjectInvalid);
                continue;
            }
        };
        let Ok(phase_count) = mem.read_u32(add_addr(
            hit,
            profile::DOMAIN53_OPERATION_INSTANCE_PHASE_COUNT,
        )?) else {
            drops.reject_semantic(OperationSemanticCause::PhaseCountUnreadable);
            continue;
        };
        if phase_count == 0 || phase_count > crate::compact::MAX_OPERATION_PHASES {
            drops.reject_semantic(OperationSemanticCause::PhaseCountInvalid);
            continue;
        }
        let Ok(duration_days) = mem.read_u32(add_addr(
            hit,
            profile::DOMAIN53_OPERATION_INSTANCE_DURATION_DAYS,
        )?) else {
            drops.reject_semantic(OperationSemanticCause::DurationUnreadable);
            continue;
        };
        let terminal = match mem.read_u8(add_addr(
            hit,
            profile::DOMAIN53_OPERATION_INSTANCE_TERMINAL_FLAG,
        )?) {
            Ok(0) => false,
            Ok(1) => true,
            Ok(_) | Err(_) => {
                drops.reject_semantic(OperationSemanticCause::TerminalFlagInvalid);
                continue;
            }
        };
        let (status, phase_index, progress_fixed, start_hour, duration_hours) = if duration_days
            == 0
        {
            if terminal {
                drops.reject_semantic(OperationSemanticCause::ZeroDurationTerminal);
                continue;
            }
            let Ok(current) = mem.read_i64(add_addr(
                hit,
                profile::DOMAIN53_OPERATION_INSTANCE_PREPARATION_CURRENT,
            )?) else {
                drops.reject_semantic(OperationSemanticCause::PreparationCurrentUnreadable);
                continue;
            };
            let Ok(required) = mem.read_i64(add_addr(
                hit,
                profile::DOMAIN53_OPERATION_INSTANCE_PREPARATION_REQUIRED,
            )?) else {
                drops.reject_semantic(OperationSemanticCause::PreparationRequiredUnreadable);
                continue;
            };
            if current < 0 || required < 0 {
                drops.reject_semantic(OperationSemanticCause::PreparationValuesNegative);
                continue;
            }
            let progress = if required == 0 {
                profile::DOMAIN53_OPERATION_PROGRESS_SCALE
            } else {
                current
                    .min(required)
                    .saturating_mul(profile::DOMAIN53_OPERATION_PROGRESS_SCALE)
                    / required
            };
            let prepared = match operation_is_prepared(mem, hit, current, required) {
                Ok(Some(value)) => value,
                Ok(None) | Err(_) => {
                    drops.reject_semantic(OperationSemanticCause::PreparedStateUnavailable);
                    continue;
                }
            };
            (
                if prepared {
                    OperationInstanceStatus::Prepared
                } else {
                    OperationInstanceStatus::Preparing
                },
                0,
                progress,
                0,
                0,
            )
        } else {
            let Ok(start_hour) = mem.read_u32(add_addr(
                hit,
                profile::DOMAIN53_OPERATION_INSTANCE_START_HOUR,
            )?) else {
                drops.reject_semantic(OperationSemanticCause::StartHourUnreadable);
                continue;
            };
            let Some(elapsed_days) = operation_elapsed_days(current_game_hour, start_hour) else {
                drops.reject_semantic(OperationSemanticCause::ElapsedTimeInvalid);
                continue;
            };
            if elapsed_days > duration_days && !terminal {
                drops.reject_semantic(OperationSemanticCause::RunningPastDuration);
                continue;
            }
            let elapsed_days = elapsed_days.min(duration_days);
            let phase_index = if terminal || elapsed_days == duration_days {
                phase_count - 1
            } else {
                elapsed_days.saturating_mul(phase_count) / duration_days
            };
            let progress = if terminal || elapsed_days == duration_days {
                profile::DOMAIN53_OPERATION_PROGRESS_SCALE
            } else {
                let Some(progress) =
                    operation_current_phase_progress(elapsed_days, duration_days, phase_count)
                else {
                    drops.reject_semantic(OperationSemanticCause::PhaseProgressInvalid);
                    continue;
                };
                progress
            };
            let Some(duration_hours) = duration_days.checked_mul(24) else {
                drops.reject_semantic(OperationSemanticCause::DurationHoursOverflow);
                continue;
            };
            (
                if terminal {
                    OperationInstanceStatus::Finished
                } else {
                    OperationInstanceStatus::Running
                },
                phase_index,
                progress,
                start_hour,
                duration_hours,
            )
        };
        let assigned_operative_ids = match read_operation_assigned_operatives(mem, hit) {
            Ok(Some(ids)) => ids,
            Ok(None) | Err(_) => {
                drops.reject_semantic(OperationSemanticCause::AssignedOperativesUnavailable);
                continue;
            }
        };
        if assigned_operative_ids.iter().any(|operative_id| {
            !valid_operative_ids.contains(operative_id) || !assigned_globally.insert(*operative_id)
        }) {
            drops.reject_semantic(OperationSemanticCause::AssignedOperativeJoinInvalid);
            continue;
        }
        rows.push(OperationInstanceRow {
            instance_id,
            def_ref_ok: true,
            associated_country_tag_db_id: Some(target_country_tag_db_id),
            definition_id: Some(definition_id),
            owner_country_tag_db_id: Some(owner_country_tag_db_id),
            target_country_tag_db_id: Some(target_country_tag_db_id),
            target_state_id,
            status: Some(status),
            phase_index,
            phase_count,
            progress_fixed: Some(progress_fixed),
            start_hour: Some(start_hour),
            duration_hours: Some(duration_hours),
            assigned_operative_ids,
        });
    }
    rows.sort_by_key(|row| row.instance_id);
    Ok((rows, drops))
}

fn read_operation_country_pointer<M: Domain53Memory>(
    mem: &M,
    country_ptr_addr: u64,
    country_vtable: u64,
    valid_countries: &BTreeSet<u32>,
) -> Option<u32> {
    let country_ptr = mem.read_u64(country_ptr_addr).ok()?;
    if !is_heap_ptr(country_ptr) {
        return None;
    }
    if mem.read_u64(country_ptr).ok()? != country_vtable {
        return None;
    }
    let tag_db_id = mem
        .read_u32(country_ptr.checked_add(profile::DOMAIN53_OBJ_TAG_DB_ID as u64)?)
        .ok()?;
    if !valid_tag_db_id(tag_db_id) || !valid_countries.contains(&tag_db_id) {
        return None;
    }
    Some(tag_db_id)
}

fn valid_operation_definition_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= crate::compact::MAX_OPERATION_DEFINITION_ID_LEN
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
}

fn read_operation_target_state<M: Domain53Memory>(
    mem: &M,
    target_object_addr: u64,
    province_vtable: u64,
    state_vtable: u64,
) -> Result<Option<u32>, ()> {
    let target_object = mem.read_u64(target_object_addr).map_err(|_| ())?;
    if target_object == 0 {
        return Ok(None);
    }
    if !is_heap_ptr(target_object)
        || mem.read_u64(target_object).map_err(|_| ())? != province_vtable
    {
        return Err(());
    }
    let state = mem
        .read_u64(
            target_object
                .checked_add(profile::CPROVINCE_OFF_STATE as u64)
                .ok_or(())?,
        )
        .map_err(|_| ())?;
    if !is_heap_ptr(state) || mem.read_u64(state).map_err(|_| ())? != state_vtable {
        return Err(());
    }
    let state_id = mem
        .read_u32(
            state
                .checked_add(profile::DOMAIN66_STATE_ID_OFF as u64)
                .ok_or(())?,
        )
        .map_err(|_| ())?;
    if state_id == 0 {
        return Err(());
    }
    if compact_contract::bound_trips::bound_exceeded_u32(
        state_id,
        profile::MAX_STATE_ID,
        compact_contract::bound_trips::BOUND_TRIP_MAP_STATE_ID,
    ) {
        return Err(());
    }
    Ok(Some(state_id))
}

fn operation_elapsed_days(current_hour: u32, start_hour: u32) -> Option<u32> {
    let epoch = profile::DOMAIN53_OPERATION_RAW_HOUR_EPOCH;
    if current_hour < epoch || start_hour < epoch || current_hour < start_hour {
        return None;
    }
    Some((current_hour - epoch) / 24 - (start_hour - epoch) / 24)
}

fn read_operation_assigned_operatives<M: Domain53Memory>(
    mem: &M,
    instance: u64,
) -> Result<Option<Vec<u32>>, ExtractError> {
    let count = mem.read_u32(add_addr(
        instance,
        profile::DOMAIN53_OPERATION_INSTANCE_OPERATIVE_COUNT,
    )?)? as usize;
    if count > crate::compact::MAX_OPERATION_ASSIGNED_OPERATIVES {
        return Ok(None);
    }
    if count == 0 {
        return Ok(Some(Vec::new()));
    }
    let data = mem.read_u64(add_addr(
        instance,
        profile::DOMAIN53_OPERATION_INSTANCE_OPERATIVE_VECTOR,
    )?)?;
    if !is_heap_ptr(data) {
        return Ok(None);
    }
    let mut ids = Vec::with_capacity(count);
    let mut seen = BTreeSet::new();
    for index in 0..count {
        let slot = add_addr(
            data,
            index.saturating_mul(profile::DOMAIN53_OPERATION_INSTANCE_OPERATIVE_STRIDE),
        )?;
        let handle = mem.read_u64(slot)?;
        let handle_type = handle as u32;
        let operative_id = (handle >> 32) as u32;
        if handle_type != profile::DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE
            || operative_id == 0
            || operative_id > MAX_OPERATIVE_ID
            || !seen.insert(operative_id)
        {
            return Ok(None);
        }
        ids.push(operative_id);
    }
    Ok(Some(ids))
}

fn read_intel_network_graph_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    operatives: &[OperativeRow],
    operative_hits: &[u64],
    strategic_operative_hits: &[u64],
    country_intel_network_hits: &[u64],
) -> Result<FamilyRead<Vec<IntelNetworkGraphRow>, IntelNetworkFailureCause>, ExtractError> {
    if strategic_operative_hits.is_empty() {
        return Ok(if country_intel_network_hits.is_empty() {
            FamilyRead::Proven(Vec::new())
        } else {
            FamilyRead::Unavailable(IntelNetworkFailureCause::NetworkHitsWithoutStrategicOwner)
        });
    }
    if countries.is_empty() {
        return Ok(FamilyRead::Unavailable(
            IntelNetworkFailureCause::CountriesEmpty,
        ));
    }
    let strategic_vtable = add_addr(
        module_base,
        profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA,
    )?;
    let network_vtable = add_addr(
        module_base,
        profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA,
    )?;
    let countries_by_tag = countries
        .iter()
        .map(|country| (country.tag_db_id, country))
        .collect::<BTreeMap<_, _>>();

    let mut networks = BTreeMap::new();
    for &network in country_intel_network_hits {
        match mem.read_u64(network) {
            Ok(vtable) if vtable == network_vtable => {}
            _ => {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::NetworkVtableInvalid,
                ))
            }
        }
        let Ok(target) = mem.read_i32(add_addr(
            network,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::NetworkTargetUnreadable,
            ));
        };
        if target <= 0 {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::NetworkTargetNonPositive,
            ));
        }
        let target = target as u32;
        let Some(&target_country) = countries_by_tag.get(&target) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::NetworkTargetUnknown,
            ));
        };
        let Ok(graph) = mem.read_u64(add_addr(
            network,
            profile::DOMAIN53_COUNTRY_INTEL_NETWORK_GRAPH,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::NetworkGraphUnreadable,
            ));
        };
        if !is_heap_ptr(graph) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::NetworkGraphPointerInvalid,
            ));
        }
        let Ok(graph_row_data) = mem.read_u64(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_DATA,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::GraphRowDataUnreadable,
            ));
        };
        let Ok(graph_row_end) = mem.read_u64(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_END,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::GraphRowEndUnreadable,
            ));
        };
        let Some(graph_row_bytes) = graph_row_end.checked_sub(graph_row_data) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::GraphRowRangeReversed,
            ));
        };
        if !is_heap_ptr(graph_row_data)
            || graph_row_bytes == 0
            || graph_row_bytes % profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE as u64 != 0
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::GraphRowShapeInvalid,
            ));
        }
        let graph_row_count =
            graph_row_bytes / profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE as u64;
        if graph_row_count == 0
            || graph_row_count > u64::from(profile::DOMAIN53_MAX_INTEL_NETWORK_STATES)
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::GraphRowCountInvalid,
            ));
        }
        let Ok(vertex_data) = mem.read_u64(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_DATA,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::VertexDataUnreadable,
            ));
        };
        let Ok(vertex_count) = mem.read_u32(add_addr(
            graph,
            profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_COUNT,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::VertexCountUnreadable,
            ));
        };
        if vertex_count == 0 || vertex_count > profile::DOMAIN53_MAX_INTEL_NETWORK_STATES {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::VertexCountInvalid,
            ));
        }
        if !is_heap_ptr(vertex_data) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::VertexDataPointerInvalid,
            ));
        }
        let Ok(subnetwork_data) = mem.read_u64(add_addr(
            network,
            profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_DATA,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::SubnetworkDataUnreadable,
            ));
        };
        let Ok(subnetwork_size) = mem.read_u32(add_addr(
            network,
            profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_SIZE,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::SubnetworkSizeUnreadable,
            ));
        };
        if subnetwork_size == 0
            || subnetwork_size > profile::DOMAIN53_MAX_INTEL_NETWORK_STATES
            || !is_heap_ptr(subnetwork_data)
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::SubnetworkShapeInvalid,
            ));
        }
        let mut state_strengths = Vec::with_capacity(vertex_count as usize);
        for index in 0..vertex_count {
            let vertex = add_addr(
                vertex_data,
                (index as usize)
                    .saturating_mul(profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE),
            )?;
            let Ok(state_id) = mem.read_i32(add_addr(
                vertex,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID,
            )?) else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::VertexStateIdUnreadable,
                ));
            };
            if state_id <= 0 || state_id as u32 > MAX_INTEL_NETWORK_STATE_ID {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::VertexStateIdInvalid,
                ));
            }
            let Ok(graph_row_index) = mem.read_u64(add_addr(
                vertex,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_ROW_INDEX,
            )?) else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::VertexRowIndexUnreadable,
                ));
            };
            if graph_row_index >= graph_row_count {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::VertexRowIndexOutOfRange,
                ));
            }
            let graph_row = add_addr(
                graph_row_data,
                (graph_row_index as usize)
                    .saturating_mul(profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE),
            )?;
            let Ok(selector) = mem.read_i32(add_addr(
                graph_row,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_SUBNETWORK_SELECTOR,
            )?) else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::GraphRowSelectorUnreadable,
                ));
            };
            if selector < 0 || selector as u32 >= subnetwork_size {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::GraphRowSelectorOutOfRange,
                ));
            }
            let subnetwork = add_addr(
                subnetwork_data,
                (selector as usize)
                    .saturating_mul(profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_STRIDE),
            )?;
            let Ok(strength_raw) = mem.read_i64(add_addr(
                subnetwork,
                profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_STRENGTH,
            )?) else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::SubnetworkStrengthUnreadable,
                ));
            };
            if !(0..=profile::DOMAIN53_INTEL_NETWORK_STRENGTH_SCALE).contains(&strength_raw) {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::SubnetworkStrengthOutOfRange,
                ));
            }
            state_strengths.push(IntelNetworkStateStrength {
                state_id: state_id as u32,
                strength_raw,
            });
        }
        state_strengths.sort_by_key(|row| row.state_id);
        if state_strengths
            .windows(2)
            .any(|pair| pair[0].state_id == pair[1].state_id)
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::DuplicateStateId,
            ));
        }
        let state_ids = state_strengths
            .iter()
            .map(|row| row.state_id)
            .collect::<Vec<_>>();
        let coverage =
            match read_intel_network_coverage(mem, module_base, network, target_country, target) {
                Ok(value) => value,
                Err(ExtractError::MemoryRead { .. }) => None,
                Err(error) => return Err(error),
            };
        if networks
            .insert(network, (target, state_ids, state_strengths, coverage))
            .is_some()
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::DuplicateNetworkPointer,
            ));
        }
    }

    let operative_owners = operatives
        .iter()
        .map(|row| (row.op_id, row.owner_tag_db_id))
        .collect::<BTreeMap<_, _>>();
    if operative_owners.len() != operatives.len() {
        return Ok(FamilyRead::Unavailable(
            IntelNetworkFailureCause::DuplicateOperativeIdentity,
        ));
    }
    let mut network_operative_ids = networks
        .keys()
        .map(|network| (*network, Vec::<u32>::new()))
        .collect::<BTreeMap<_, _>>();
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    for &leader in operative_hits {
        match mem.read_u64(leader) {
            Ok(vtable) if vtable == operative_vtable => {}
            _ => continue,
        }
        let identity = add_addr(leader, profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE)?;
        let Ok(handle_type) = mem.read_u32(identity) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeHandleTypeUnreadable,
            ));
        };
        let Ok(operative_id) = mem.read_u32(add_addr(identity, 4)?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeIdUnreadable,
            ));
        };
        if handle_type != profile::DOMAIN53_OPERATIVE_HANDLE_TYPE
            || !operative_owners.contains_key(&operative_id)
        {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeIdentityInvalid,
            ));
        }
        let action = add_addr(leader, profile::DOMAIN53_OPERATIVE_ACTION)?;
        let Ok(variant) = mem.read_u32(add_addr(
            action,
            profile::DOMAIN53_OPERATIVE_ACTION_VARIANT,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeActionVariantUnreadable,
            ));
        };
        if !matches!(
            variant,
            profile::DOMAIN53_OPERATIVE_ACTION_BUILD_NETWORK
                | profile::DOMAIN53_OPERATIVE_ACTION_QUIET_NETWORK
        ) {
            continue;
        }
        let Ok(wrapper) = mem.read_u64(add_addr(
            action,
            profile::DOMAIN53_OPERATIVE_ACTION_PAYLOAD_WRAPPER,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeActionWrapperUnreadable,
            ));
        };
        if !is_heap_ptr(wrapper) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeActionWrapperInvalid,
            ));
        }
        let Ok(network) = mem.read_u64(add_addr(
            wrapper,
            profile::DOMAIN53_OPERATIVE_ACTION_NETWORK_TARGET,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeActionNetworkUnreadable,
            ));
        };
        let Some(ids) = network_operative_ids.get_mut(&network) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::OperativeActionNetworkUnknown,
            ));
        };
        if ids.contains(&operative_id) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::DuplicateNetworkOperative,
            ));
        }
        ids.push(operative_id);
    }
    for ids in network_operative_ids.values_mut() {
        ids.sort_unstable();
    }

    let mut reached = BTreeSet::new();
    let mut rows = Vec::new();
    let mut seen_slots = BTreeSet::new();
    for &operative in strategic_operative_hits {
        match mem.read_u64(operative) {
            Ok(vtable) if vtable == strategic_vtable => {}
            _ => {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::StrategicOperativeVtableInvalid,
                ))
            }
        }
        let Ok(owner_tag_db_id) = mem.read_u32(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_OWNER_TAG_DB_ID,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicOwnerUnreadable,
            ));
        };
        if !countries_by_tag.contains_key(&owner_tag_db_id) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicOwnerUnknown,
            ));
        }
        let Ok(data) = mem.read_u64(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_VECTOR,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicNetworkVectorUnreadable,
            ));
        };
        let Ok(size) = mem.read_u32(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_SIZE,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicNetworkSizeUnreadable,
            ));
        };
        let Ok(capacity) = mem.read_u32(add_addr(
            operative,
            profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_CAPACITY,
        )?) else {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicNetworkCapacityUnreadable,
            ));
        };
        if size == 0 {
            continue;
        }
        if size > capacity || size > profile::DOMAIN53_MAX_INTEL_NETWORKS || !is_heap_ptr(data) {
            return Ok(FamilyRead::Unavailable(
                IntelNetworkFailureCause::StrategicNetworkShapeInvalid,
            ));
        }
        for slot in 0..size {
            let Ok(network) = mem.read_u64(add_addr(data, (slot as usize).saturating_mul(8))?)
            else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::StrategicNetworkSlotUnreadable,
                ));
            };
            let Some((target, state_ids, state_strengths, coverage)) = networks.get(&network)
            else {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::StrategicNetworkSlotUnknown,
                ));
            };
            if *target == owner_tag_db_id {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::NetworkOwnerEqualsTarget,
                ));
            }
            if !reached.insert(network) {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::NetworkReachedTwice,
                ));
            }
            if !seen_slots.insert((owner_tag_db_id, slot)) {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::DuplicateOwnerSlot,
                ));
            }
            let operative_ids = network_operative_ids
                .get(&network)
                .expect("all authenticated network pointers initialized");
            if operative_ids
                .iter()
                .any(|operative_id| operative_owners.get(operative_id) != Some(&owner_tag_db_id))
            {
                return Ok(FamilyRead::Unavailable(
                    IntelNetworkFailureCause::NetworkOperativeOwnerMismatch,
                ));
            }
            rows.push(IntelNetworkGraphRow {
                strategic_operative_index: owner_tag_db_id,
                network_slot: slot,
                target_tag_db_id: *target,
                state_ids: state_ids.clone(),
                state_strengths: Some(state_strengths.clone()),
                coverage: coverage.clone(),
                operative_ids: Some(operative_ids.clone()),
            });
        }
    }

    if reached.len() != networks.len() {
        return Ok(FamilyRead::Unavailable(
            IntelNetworkFailureCause::UnreachedNetworkCount,
        ));
    }
    rows.sort_by_key(|row| {
        (
            row.strategic_operative_index,
            row.network_slot,
            row.target_tag_db_id,
        )
    });
    if rows.iter().any(|row| row.coverage.is_none()) {
        for row in &mut rows {
            row.coverage = None;
        }
    }
    Ok(FamilyRead::Proven(rows))
}

fn read_intel_network_coverage<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    network: u64,
    target_country: &CountryRoot,
    target_id: u32,
) -> Result<Option<IntelNetworkCoverage>, ExtractError> {
    let total_controlled_states = mem.read_u32(add_addr(
        target_country.country_ptr,
        profile::DOMAIN53_COUNTRY_CONTROLLED_STATE_COUNT,
    )?)?;
    if total_controlled_states > profile::DOMAIN53_MAX_VECTOR_ROWS {
        return Ok(None);
    }
    let state_data = if total_controlled_states == 0 {
        0
    } else {
        let value = mem.read_u64(add_addr(
            target_country.country_ptr,
            profile::DOMAIN53_COUNTRY_CONTROLLED_STATE_VECTOR,
        )?)?;
        if !is_heap_ptr(value) {
            return Ok(None);
        }
        value
    };
    let mut total_core_states = 0u32;
    for index in 0..total_controlled_states {
        let state = mem.read_u64(add_addr(state_data, index as usize * 8)?)?;
        if !is_heap_ptr(state) {
            return Ok(None);
        }
        let core_count =
            mem.read_u32(add_addr(state, profile::DOMAIN53_STATE_CORE_COUNTRY_COUNT)?)?;
        if core_count > profile::DOMAIN53_MAX_COUNTRY_TABLE_SLOTS {
            return Ok(None);
        }
        let core_data = if core_count == 0 {
            0
        } else {
            let value = mem.read_u64(add_addr(
                state,
                profile::DOMAIN53_STATE_CORE_COUNTRY_VECTOR,
            )?)?;
            if !is_heap_ptr(value) {
                return Ok(None);
            }
            value
        };
        let mut is_core = false;
        for core_index in 0..core_count {
            let core_id = mem.read_u32(add_addr(core_data, core_index as usize * 4)?)?;
            if country_ids_equivalent(mem, module_base, core_id, target_id)? {
                is_core = true;
                break;
            }
        }
        if is_core {
            total_core_states = total_core_states.saturating_add(1);
        }
    }

    let total_worth = mem.read_u32(add_addr(
        target_country.country_ptr,
        profile::DOMAIN53_COUNTRY_TOTAL_STATE_WORTH,
    )?)?;
    let covered_core_states = mem.read_u32(add_addr(
        network,
        profile::DOMAIN53_INTEL_NETWORK_COVERED_CORE_STATES,
    )?)?;
    let covered_controlled_states = mem.read_u32(add_addr(
        network,
        profile::DOMAIN53_INTEL_NETWORK_COVERED_CONTROLLED_STATES,
    )?)?;
    let covered_worth = mem.read_u32(add_addr(
        network,
        profile::DOMAIN53_INTEL_NETWORK_COVERED_WORTH,
    )?)?;
    let controlled_strength_sum_raw = mem.read_i64(add_addr(
        network,
        profile::DOMAIN53_INTEL_NETWORK_CONTROLLED_STRENGTH_SUM,
    )?)?;
    let core_strength_sum_raw = mem.read_i64(add_addr(
        network,
        profile::DOMAIN53_INTEL_NETWORK_CORE_STRENGTH_SUM,
    )?)?;
    let weight_controlled_raw = mem.read_i64(add_addr(
        module_base,
        profile::DOMAIN53_INTEL_NETWORK_WEIGHT_CONTROLLED_RVA,
    )?)?;
    let weight_owned_raw = mem.read_i64(add_addr(
        module_base,
        profile::DOMAIN53_INTEL_NETWORK_WEIGHT_OWNED_RVA,
    )?)?;
    let weight_owned_worth_raw = mem.read_i64(add_addr(
        module_base,
        profile::DOMAIN53_INTEL_NETWORK_WEIGHT_OWNED_WORTH_RVA,
    )?)?;
    if covered_core_states > total_core_states
        || covered_controlled_states > total_controlled_states
        || covered_worth > total_worth
        || controlled_strength_sum_raw < 0
        || core_strength_sum_raw < 0
        || weight_controlled_raw < 0
        || weight_owned_raw < 0
        || weight_owned_worth_raw < 0
    {
        return Ok(None);
    }
    let Some(national_coverage_raw) = intel_network_national_coverage(
        covered_core_states,
        covered_controlled_states,
        covered_worth,
        controlled_strength_sum_raw,
        core_strength_sum_raw,
        total_core_states,
        total_controlled_states,
        total_worth,
        weight_controlled_raw,
        weight_owned_raw,
        weight_owned_worth_raw,
    ) else {
        return Ok(None);
    };
    Ok(Some(IntelNetworkCoverage {
        covered_core_states,
        covered_controlled_states,
        covered_worth,
        controlled_strength_sum_raw,
        core_strength_sum_raw,
        total_core_states,
        total_controlled_states,
        total_worth,
        weight_controlled_raw,
        weight_owned_raw,
        weight_owned_worth_raw,
        national_coverage_raw,
    }))
}

fn intel_network_national_coverage(
    covered_core: u32,
    covered_controlled: u32,
    covered_worth: u32,
    controlled_strength_sum: i64,
    core_strength_sum: i64,
    total_core: u32,
    total_controlled: u32,
    total_worth: u32,
    weight_controlled: i64,
    weight_owned: i64,
    weight_owned_worth: i64,
) -> Option<i64> {
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
        i64::try_from(
            i128::from(average) * i128::from(weighted_ratio)
                / i128::from(profile::DOMAIN53_OPERATION_PROGRESS_SCALE),
        )
        .ok()
    }
    let average_controlled = average(controlled_strength_sum, covered_controlled)?;
    let average_core = average(core_strength_sum, covered_core)?;
    let controlled = contribution(
        average_controlled,
        ratio(covered_controlled, total_controlled, weight_controlled)?,
    )?;
    let owned = contribution(average_core, ratio(covered_core, total_core, weight_owned)?)?;
    let owned_worth = contribution(
        average_controlled,
        ratio(covered_worth, total_worth, weight_owned_worth)?,
    )?;
    Some(
        controlled
            .saturating_add(owned)
            .saturating_add(owned_worth)
            .clamp(0, profile::DOMAIN53_OPERATION_PROGRESS_SCALE),
    )
}

fn operation_current_phase_progress(
    elapsed_days: u32,
    duration_days: u32,
    phase_count: u32,
) -> Option<i64> {
    if duration_days == 0 || phase_count == 0 {
        return None;
    }
    let phase_index = elapsed_days.saturating_mul(phase_count) / duration_days;
    let days_per_phase = duration_days / phase_count;
    if days_per_phase == 0 {
        return None;
    }
    let days_in_phase = elapsed_days.saturating_sub(phase_index.saturating_mul(days_per_phase));
    Some(
        (i64::from(days_in_phase) * profile::DOMAIN53_OPERATION_PROGRESS_SCALE)
            / i64::from(days_per_phase),
    )
}

fn operation_is_prepared<M: Domain53Memory>(
    mem: &M,
    instance: u64,
    current: i64,
    required: i64,
) -> Result<Option<bool>, ExtractError> {
    if current < required {
        return Ok(Some(false));
    }
    let requirement_count = mem.read_u32(add_addr(
        instance,
        profile::DOMAIN53_OPERATION_INSTANCE_REQUIREMENT_COUNT,
    )?)?;
    if requirement_count > profile::DOMAIN53_MAX_OPERATION_RESOURCES {
        return Ok(None);
    }
    if requirement_count == 0 {
        return Ok(Some(true));
    }
    let requirement_data = mem.read_u64(add_addr(
        instance,
        profile::DOMAIN53_OPERATION_INSTANCE_REQUIREMENT_DATA,
    )?)?;
    if !is_heap_ptr(requirement_data) {
        return Ok(None);
    }
    let map = add_addr(instance, profile::DOMAIN53_OPERATION_INSTANCE_RESOURCE_MAP)?;
    let map_count = mem.read_u32(add_addr(
        map,
        profile::DOMAIN53_OPERATION_RESOURCE_MAP_COUNT,
    )?)?;
    if map_count > profile::DOMAIN53_MAX_OPERATION_RESOURCES {
        return Ok(None);
    }
    let map_data = if map_count == 0 {
        0
    } else {
        let value = mem.read_u64(add_addr(
            map,
            profile::DOMAIN53_OPERATION_RESOURCE_MAP_DATA,
        )?)?;
        if !is_heap_ptr(value) {
            return Ok(None);
        }
        value
    };
    for index in 0..requirement_count {
        let requirement = add_addr(
            requirement_data,
            index as usize * profile::DOMAIN53_OPERATION_REQUIREMENT_STRIDE,
        )?;
        if mem.read_u8(add_addr(
            requirement,
            profile::DOMAIN53_OPERATION_REQUIREMENT_OPTIONAL,
        )?)? != 0
        {
            continue;
        }
        let amount = mem.read_i64(add_addr(
            requirement,
            profile::DOMAIN53_OPERATION_REQUIREMENT_AMOUNT,
        )?)?;
        if amount < 0 {
            return Ok(None);
        }
        let definition = mem.read_u64(add_addr(
            requirement,
            profile::DOMAIN53_OPERATION_REQUIREMENT_DEFINITION,
        )?)?;
        if !is_heap_ptr(definition) {
            return Ok(None);
        }
        let resource = mem.read_u64(add_addr(
            definition,
            profile::DOMAIN53_OPERATION_REQUIREMENT_RESOURCE,
        )?)?;
        if !is_heap_ptr(resource) {
            return Ok(None);
        }
        let key = mem.read_u32(add_addr(
            resource,
            profile::DOMAIN53_OPERATION_RESOURCE_SORT_KEY,
        )?)?;
        let mut available = 0i64;
        for map_index in 0..map_count {
            let row = add_addr(
                map_data,
                map_index as usize * profile::DOMAIN53_OPERATION_RESOURCE_MAP_ROW_STRIDE,
            )?;
            let map_definition = mem.read_u64(add_addr(
                row,
                profile::DOMAIN53_OPERATION_RESOURCE_MAP_ROW_DEFINITION,
            )?)?;
            if !is_heap_ptr(map_definition) {
                return Ok(None);
            }
            let map_key = mem.read_u32(add_addr(
                map_definition,
                profile::DOMAIN53_OPERATION_RESOURCE_SORT_KEY,
            )?)?;
            if map_key == key {
                available = mem.read_i64(add_addr(
                    row,
                    profile::DOMAIN53_OPERATION_RESOURCE_MAP_ROW_VALUE,
                )?)?;
                break;
            }
            if map_key > key {
                break;
            }
        }
        if available < amount {
            return Ok(Some(false));
        }
    }
    Ok(Some(true))
}

fn read_intel_map_rows<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
) -> Result<FamilyRead<Vec<IntelMapRow>, IntelMapFailureCause>, ExtractError> {
    if countries.is_empty() {
        return Ok(FamilyRead::Unavailable(
            IntelMapFailureCause::CountriesEmpty,
        ));
    }
    let denominators_slot = add_addr(module_base, profile::DOMAIN53_INTEL_LEVEL_DENOMINATORS_RVA)?;
    let denominators_addr = match mem.read_u64(denominators_slot) {
        Ok(value) if is_heap_ptr(value) => value,
        _ => {
            return Ok(FamilyRead::Unavailable(
                IntelMapFailureCause::DenominatorsRootUnavailable,
            ))
        }
    };
    let denominator_causes = [
        IntelMapFailureCause::CivilianDenominatorInvalid,
        IntelMapFailureCause::ArmyDenominatorInvalid,
        IntelMapFailureCause::NavyDenominatorInvalid,
        IntelMapFailureCause::AirDenominatorInvalid,
    ];
    let mut denominators = [0i64; 4];
    for (index, denominator) in denominators.iter_mut().enumerate() {
        *denominator = match mem.read_i64(add_addr(denominators_addr, index * 8)?) {
            Ok(value) if value > 0 => value,
            _ => return Ok(FamilyRead::Unavailable(denominator_causes[index])),
        };
    }
    let no_fog = match mem.read_u8(add_addr(
        module_base,
        profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA,
    )?) {
        Ok(value) => value == 0,
        Err(_) => {
            return Ok(FamilyRead::Unavailable(
                IntelMapFailureCause::NoFogFlagUnreadable,
            ))
        }
    };

    let mut rows = Vec::with_capacity(countries.len().saturating_mul(countries.len()));
    for viewer in countries {
        let intel_table = match mem.read_u64(add_addr(
            viewer.country_ptr,
            profile::DOMAIN53_COUNTRY_INTEL_TABLE,
        )?) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => {
                return Ok(FamilyRead::Unavailable(
                    IntelMapFailureCause::ViewerIntelTableUnavailable,
                ))
            }
        };
        let data = match mem.read_u64(add_addr(intel_table, profile::DOMAIN53_INTEL_TABLE_DATA)?) {
            Ok(value) if is_heap_ptr(value) => value,
            _ => {
                return Ok(FamilyRead::Unavailable(
                    IntelMapFailureCause::IntelTableDataUnavailable,
                ))
            }
        };
        let count = match mem.read_u32(add_addr(intel_table, profile::DOMAIN53_INTEL_TABLE_COUNT)?)
        {
            Ok(value) if value > 0 && value <= profile::DOMAIN53_MAX_VECTOR_ROWS => value,
            _ => {
                return Ok(FamilyRead::Unavailable(
                    IntelMapFailureCause::IntelTableCountInvalid,
                ))
            }
        };
        for target in countries {
            let target_index = match mem.read_u32(add_addr(
                target.country_ptr,
                profile::DOMAIN53_COUNTRY_CID_INDEX,
            )?) {
                Ok(value) if value < count => value,
                _ => {
                    return Ok(FamilyRead::Unavailable(
                        IntelMapFailureCause::TargetCountryIndexInvalid,
                    ))
                }
            };
            let full = if no_fog {
                true
            } else {
                match intel_full_visibility(mem, module_base, viewer, target, target_index) {
                    Ok(value) => value,
                    Err(_) => {
                        return Ok(FamilyRead::Unavailable(
                            IntelMapFailureCause::FullVisibilityReadFailed,
                        ))
                    }
                }
            };
            let mut fixed = [FIXED_SCALE_RAW as i64; 4];
            if !full {
                let row_addr = add_addr(
                    data,
                    (target_index as usize).saturating_mul(profile::DOMAIN53_INTEL_ROW_STRIDE),
                )?;
                for index in 0..4 {
                    let raw = match mem.read_i64(add_addr(row_addr, index * 8)?) {
                        Ok(value) if value >= 0 => value,
                        _ => {
                            return Ok(FamilyRead::Unavailable(
                                IntelMapFailureCause::IntelLaneValueInvalid,
                            ))
                        }
                    };
                    let Some(scaled) = raw.checked_mul(i64::from(FIXED_SCALE_RAW)) else {
                        return Ok(FamilyRead::Unavailable(
                            IntelMapFailureCause::IntelLaneScaleOverflow,
                        ));
                    };
                    fixed[index] = scaled / denominators[index];
                    if !(0..=i64::from(FIXED_SCALE_RAW)).contains(&fixed[index]) {
                        return Ok(FamilyRead::Unavailable(
                            IntelMapFailureCause::NormalizedValueOutOfRange,
                        ));
                    }
                }
            }
            rows.push(IntelMapRow {
                owner_tag_db_id: viewer.tag_db_id,
                target_tag_db_id: target.tag_db_id,
                civilian_raw: fixed[0] as i32,
                army_raw: fixed[1] as i32,
                navy_raw: fixed[2] as i32,
                air_raw: fixed[3] as i32,
                resisted: false,
            });
        }
    }
    rows.sort_by_key(|row| (row.owner_tag_db_id, row.target_tag_db_id));
    Ok(FamilyRead::Proven(rows))
}

fn read_air_disclosure_threshold<M: Domain53Memory>(mem: &M, module_base: u64) -> Option<i64> {
    let address = add_addr(module_base, profile::DOMAIN53_AIR_DISCLOSURE_THRESHOLD_RVA).ok()?;
    mem.read_i64(address).ok()
}

fn intel_full_visibility<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    viewer: &CountryRoot,
    target: &CountryRoot,
    target_index: u32,
) -> Result<bool, ExtractError> {
    if viewer.tag_db_id == target.tag_db_id {
        return Ok(true);
    }
    let viewer_manager = mem.read_u64(add_addr(
        viewer.country_ptr,
        profile::DOMAIN53_COUNTRY_RELATION_STATUS_MANAGER,
    )?)?;
    let target_manager = mem.read_u64(add_addr(
        target.country_ptr,
        profile::DOMAIN53_COUNTRY_RELATION_STATUS_MANAGER,
    )?)?;
    if !is_heap_ptr(viewer_manager) || !is_heap_ptr(target_manager) {
        return Ok(false);
    }
    let table = mem.read_u64(add_addr(
        viewer_manager,
        profile::DOMAIN53_RELATION_STATUS_TABLE,
    )?)?;
    if !is_heap_ptr(table) {
        return Ok(false);
    }
    let relation = mem.read_u64(add_addr(table, target_index as usize * 8)?)?;
    if !is_heap_ptr(relation) {
        return Ok(false);
    }
    let visibility_block = mem.read_u64(add_addr(
        relation,
        profile::DOMAIN53_RELATION_STATUS_VISIBILITY_BLOCK,
    )?)?;
    if is_heap_ptr(visibility_block)
        && mem.read_u8(add_addr(
            visibility_block,
            profile::DOMAIN53_RELATION_STATUS_VISIBILITY_BLOCK_FLAG,
        )?)? == 0
    {
        return Ok(false);
    }
    let viewer_faction = mem.read_u64(add_addr(
        viewer_manager,
        profile::DOMAIN53_RELATION_MANAGER_FACTION,
    )?)?;
    let target_faction = mem.read_u64(add_addr(
        target_manager,
        profile::DOMAIN53_RELATION_MANAGER_FACTION,
    )?)?;
    if viewer_faction != 0 && viewer_faction == target_faction {
        return Ok(true);
    }
    let subject = mem.read_u64(add_addr(
        relation,
        profile::DOMAIN53_RELATION_STATUS_SUBJECT,
    )?)?;
    if !is_heap_ptr(subject)
        || mem.read_u32(add_addr(subject, profile::DOMAIN53_SUBJECT_RELATION_TYPE)?)?
            != profile::DOMAIN53_SUBJECT_RELATION_TYPE_VALUE
    {
        return Ok(false);
    }
    let relation_id = mem.read_u32(add_addr(
        relation,
        profile::DOMAIN53_RELATION_STATUS_FIRST_ID,
    )?)?;
    let overlord = mem.read_u32(add_addr(
        subject,
        profile::DOMAIN53_SUBJECT_RELATION_OVERLORD,
    )?)?;
    let subject_id = mem.read_u32(add_addr(
        subject,
        profile::DOMAIN53_SUBJECT_RELATION_SUBJECT,
    )?)?;
    Ok(
        country_ids_equivalent(mem, module_base, overlord, relation_id)?
            || country_ids_equivalent(mem, module_base, subject_id, relation_id)?,
    )
}

fn country_ids_equivalent<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    left: u32,
    right: u32,
) -> Result<bool, ExtractError> {
    if left == right {
        return Ok(true);
    }
    if left == 0 || right == 0 {
        return Ok(false);
    }
    let game_state = mem.read_u64(add_addr(
        module_base,
        profile::DOMAIN57_CGAMESTATE_SINGLETON_RVA,
    )?)?;
    if !is_heap_ptr(game_state) {
        return Ok(false);
    }
    let table = mem.read_u64(add_addr(
        game_state,
        profile::DOMAIN53_CGAMESTATE_CID_EQUIVALENCE_TABLE,
    )?)?;
    if !is_heap_ptr(table) {
        return Ok(false);
    }
    let left_value = mem.read_u32(add_addr(table, left as usize * 4)?)?;
    let right_value = mem.read_u32(add_addr(table, right as usize * 4)?)?;
    Ok(left_value == right_value)
}

/// Object-identity witness width for the stable-reject bracket: the first 16
/// bytes of the identity block (handle type + op id + neighbours), sampled
/// before and after the two row attempts alongside the vtable qword.
const OPERATIVE_STABLE_REJECT_HEAD_BYTES: usize = 16;

/// Stable-reject eligibility: ONLY the row-validation failures whose
/// deciding raw value lives INSIDE the bracketed identity head
/// (`OPERATIVE_STABLE_REJECT_HEAD_BYTES`): handle type (offset 0) and op id
/// (offset 4). For these, the full bracket - head byte-equal before attempt 1
/// and after attempt 2, vtable qword unchanged, identical error - proves both
/// attempts observed the same invalid bytes of the SAME object.
///
/// Deliberately NOT eligible, each refusing by name instead:
/// - `MemoryRead`: equality proves nothing about contents;
/// - the constant-string utf8 arms: distinct invalid states collapse to one
///   value;
/// - `owner_invalid`: decides from the frame's country-acceptance oracle
///   snapshot, not from the candidate's own bytes;
/// - `assignment` / `xp` / `skill` / `name` failures: their deciding fields
///   lie OUTSIDE the bracketed head (offsets up to 0x1078 and beyond, or
///   behind the leader pointer), so the bracket cannot witness them; two
///   identical observations prove only repeatability, not stability of a
///   real row's absence;
/// - InvalidRoot: value-free.
fn operative_stable_reject_error(err: &ExtractError) -> bool {
    matches!(err, ExtractError::Validation(message)
        if message.starts_with("domain53_operative_handle_type=")
            || message.starts_with("domain53_operative_id_invalid="))
}

fn read_operatives<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    hits: &[u64],
    acc: &mut RowAccumulator,
) -> Result<usize, ExtractError> {
    let valid_owners = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let operative_vtable = add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA)?;
    let mut seen = BTreeSet::new();
    let mut stale_dropped = 0usize;
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (hit_index, hit) in hits.iter().enumerate() {
        let hit = *hit;
        if prefetch_dist_candidates != 0 {
            if let Some(&future_hit) = hit_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| hits.get(i))
            {
                prefetch_read(future_hit);
            }
        }
        match mem.read_u64(hit) {
            Ok(vtable) if vtable == operative_vtable => {}
            Ok(_) | Err(_) => {
                stale_dropped = stale_dropped.saturating_add(1);
                continue;
            }
        }
        let identity = hit
            .checked_add(profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64)
            .ok_or(ExtractError::InvalidRoot(
                "domain53_operative_identity_overflow",
            ))?;
        // Object-identity witness sampled BEFORE the first row attempt so a
        // failure can be bracketed on both sides. One extra 16-byte read per
        // candidate; the row attempt reads this region anyway.
        let mut head_a = [0u8; OPERATIVE_STABLE_REJECT_HEAD_BYTES];
        let head_a_ok = mem.read_into(identity, &mut head_a).is_ok();
        let read = read_operative_row(mem, identity, &valid_owners);
        match read {
            Ok(row) if seen.insert(row.op_id) => {
                acc.rows.operatives.push(row);
                acc.observe();
            }
            Ok(_) => {}
            Err(first)
                if head_a_ok && operative_stable_reject_error(&first) =>
            {
                // kills: d53 permanent needle-hit garbage (freed operative
                // slots retaining the vtable qword) stamped PARTIAL_DATA on
                // every frame of every capture (measured 56.3-56.7 % of
                // candidates, constant across sessions: pw_gate_final
                // 63895/112622, pr_d47_run1 68410/120615, pt_d13_after
                // 66564/118232) because a single failed row read counted the
                // candidate as a skipped DATA row.
                //
                // A candidate is stably not a publishable row ONLY when the
                // whole observation bracket agrees: the vtable qword and the
                // identity head are re-read AFTER the second attempt and must
                // equal their before-samples (the object was not freed,
                // reused or rewritten mid-observation), AND both attempts
                // fail with the IDENTICAL error whose deciding bytes live
                // inside the bracketed head (handle type / op id - see
                // operative_stable_reject_error). Everything else refuses by
                // name: MemoryRead equality proves nothing about contents,
                // constant-string utf8 arms collapse distinct invalid states,
                // owner_invalid decides from the oracle snapshot, and the
                // assignment/xp/skill/name fields lie outside the bracketed
                // head. Any inequality, different error, or a success stays a
                // torn skip.
                let second = read_operative_row(mem, identity, &valid_owners);
                let vtable_b = mem.read_u64(hit);
                let mut head_b = [0u8; OPERATIVE_STABLE_REJECT_HEAD_BYTES];
                let head_b_ok = mem.read_into(identity, &mut head_b).is_ok();
                match vtable_b {
                    Ok(v) if v == operative_vtable && head_b_ok && head_b == head_a => {
                        match second {
                            Err(e) if e == first => {
                                acc.stable_rejects.operative =
                                    acc.stable_rejects.operative.saturating_add(1);
                            }
                            _ => acc.skip_operative(),
                        }
                    }
                    _ => acc.skip_operative(),
                }
            }
            Err(_) => acc.skip_operative(),
        }
    }
    Ok(stale_dropped)
}

fn read_operative_row<M: Domain53Memory>(
    mem: &M,
    identity: u64,
    valid_owners: &BTreeSet<u32>,
) -> Result<OperativeRow, ExtractError> {
    // Row-validation failures are VALUE-CARRYING on purpose: the stable-reject
    // classification in `read_operatives` re-observes a failed candidate and
    // requires the IDENTICAL error. Only the two arms whose deciding bytes
    // live inside the bracketed identity head (handle type, op id) are
    // eligible for that classification; the remaining arms carry values or
    // constant strings purely for diagnostics and refuse by name regardless.
    let handle_type = mem.read_u32(identity)?;
    if handle_type != profile::DOMAIN53_OPERATIVE_HANDLE_TYPE {
        return Err(ExtractError::Validation(format!(
            "domain53_operative_handle_type={handle_type:#x}"
        )));
    }
    let op_id = mem.read_u32(add_addr(identity, 4)?)?;
    if op_id == 0 || op_id > MAX_OPERATIVE_ID {
        return Err(ExtractError::Validation(format!(
            "domain53_operative_id_invalid={op_id}"
        )));
    }
    let owner = mem.read_u32(add_addr(identity, profile::DOMAIN53_OPERATIVE_OWNER)?)?;
    if !valid_tag_db_id(owner) || !valid_owners.contains(&owner) {
        return Err(ExtractError::Validation(format!(
            "domain53_operative_owner_invalid={owner}"
        )));
    }
    let assignment = mem.read_u32(add_addr(
        identity,
        profile::DOMAIN53_OPERATIVE_ASSIGNMENT_ENUM,
    )?)?;
    if assignment > 5 {
        return Err(ExtractError::Validation(format!(
            "domain53_operative_assignment_invalid={assignment}"
        )));
    }
    let _portrait_gfx = match read_msvc_string(
        mem,
        add_addr(identity, profile::DOMAIN53_OPERATIVE_PORTRAIT_GFX)?,
        256,
    ) {
        StringRead::Value(value) if !value.is_empty() => value,
        StringRead::Value(_) | StringRead::Missing => String::new(),
        StringRead::Invalid => {
            return Err(ExtractError::Validation(
                "domain53_operative_portrait_gfx_invalid_utf8".to_string(),
            ))
        }
    };
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
    let name_hash = match (&name_a, &name_b) {
        (StringRead::Value(a), StringRead::Value(b)) if a == b => crate::compact::stable_hash64(a),
        (StringRead::Value(a), StringRead::Missing)
        | (StringRead::Missing, StringRead::Value(a)) => crate::compact::stable_hash64(a),
        (StringRead::Missing, StringRead::Missing) => 0,
        (StringRead::Invalid, _) | (_, StringRead::Invalid) => {
            return Err(ExtractError::Validation(
                "domain53_operative_name_invalid_utf8".to_string(),
            ));
        }
        _ => {
            return Err(ExtractError::Validation(format!(
                "domain53_operative_name_mismatch:{name_a:?}:{name_b:?}"
            )))
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
        Ok(value) => {
            return Err(ExtractError::Validation(format!(
                "domain53_operative_xp_invalid={value}"
            )))
        }
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
                Ok(value) => {
                    return Err(ExtractError::Validation(format!(
                        "domain53_operative_skill_invalid={value}"
                    )))
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
        state_hash: crate::compact::stable_hash32(&format!("assignment_{assignment}")),
        lifecycle_flag: None,
    })
}

fn domain53_war_date_vtables(module_base: u64) -> Result<[u64; 2], ExtractError> {
    Ok([
        add_addr(module_base, profile::DOMAIN53_WAR_GREGORIAN_DATE_VTABLE_RVA)?,
        add_addr(module_base, profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA)?,
    ])
}

pub fn domain53_war_row_addrs_from_relation_roots<M: Domain53Memory>(
    mem: &M,
) -> Result<Vec<u64>, ExtractError> {
    let roots = resolve_domain53_frame_roots(mem)?;
    let module_base = roots.module_base;
    let valid_tags = roots
        .countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let expected_war_vtable = add_addr(module_base, profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA)?;
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let mut target_indices = BTreeSet::new();
    for target in &roots.countries {
        let target_index = mem.read_u32(add_addr(
            target.country_ptr,
            profile::DOMAIN53_COUNTRY_CID_INDEX,
        )?)?;
        if target_index > profile::DOMAIN53_MAX_COUNTRY_INDEX {
            return Err(ExtractError::InvalidRoot(
                "domain53_war_relation_target_index_invalid",
            ));
        }
        target_indices.insert(target_index);
    }

    let mut addrs = BTreeSet::new();
    let mut seen_pairs = BTreeSet::new();
    let mut manager_count = 0u32;
    let mut table_count = 0u32;
    for viewer in &roots.countries {
        let manager = mem.read_u64(add_addr(
            viewer.country_ptr,
            profile::DOMAIN53_RELATION_MANAGER,
        )?)?;
        if manager == 0 {
            continue;
        }
        if !is_heap_ptr(manager) {
            return Err(ExtractError::InvalidRoot(
                "domain53_war_relation_manager_invalid",
            ));
        }
        manager_count = manager_count.saturating_add(1);
        let table = mem.read_u64(add_addr(manager, profile::DOMAIN60_WAR_RELATION_TABLE)?)?;
        if table == 0 {
            continue;
        }
        if !is_heap_ptr(table) {
            return Err(ExtractError::InvalidRoot(
                "domain53_war_relation_table_invalid",
            ));
        }
        table_count = table_count.saturating_add(1);
        for target_index in &target_indices {
            let slot = table.checked_add(u64::from(*target_index) * 8).ok_or(
                ExtractError::InvalidRoot("domain53_war_relation_slot_overflow"),
            )?;
            let relation = mem.read_u64(slot)?;
            if relation == 0 {
                continue;
            }
            if !is_heap_ptr(relation) {
                return Err(ExtractError::InvalidRoot("domain53_war_relation_invalid"));
            }
            let war_object = mem.read_u64(add_addr(
                relation,
                profile::DOMAIN60_WAR_RELATION_OBJECT_PTR,
            )?)?;
            if war_object == 0 {
                continue;
            }
            if !is_heap_ptr(war_object) {
                return Err(ExtractError::InvalidRoot(
                    "domain53_war_relation_object_invalid",
                ));
            }
            let retired = mem.read_bytes(
                add_addr(war_object, profile::DOMAIN60_WAR_OBJECT_RETIRED)?,
                1,
            )?[0];
            if retired != 0 {
                continue;
            }
            if mem.read_u64(war_object)? != expected_war_vtable {
                return Err(ExtractError::InvalidRoot(
                    "domain53_war_relation_object_vtable_mismatch",
                ));
            }
            let row_addr = add_addr(war_object, profile::DOMAIN60_WAR_OBJECT_TO_ROW)?;
            if addrs.contains(&row_addr) {
                continue;
            }
            let row_date_vtable =
                mem.read_u64(add_addr(row_addr, profile::DOMAIN53_WAR_DATE_VTABLE)?)?;
            if !date_vtables.contains(&row_date_vtable) {
                return Err(ExtractError::InvalidRoot(
                    "domain53_war_relation_row_date_vtable_mismatch",
                ));
            }
            let row = read_war_row(mem, row_addr, &valid_tags)?;
            let key = if row.first_tag_db_id <= row.second_tag_db_id {
                (row.first_tag_db_id, row.second_tag_db_id)
            } else {
                (row.second_tag_db_id, row.first_tag_db_id)
            };
            if !seen_pairs.insert(key) {
                return Err(ExtractError::InvalidRoot(
                    "domain53_war_relation_duplicate_war_pair",
                ));
            }
            addrs.insert(row_addr);
        }
    }
    if manager_count == 0 || table_count == 0 {
        return Err(ExtractError::InvalidRoot(
            "domain53_war_relation_roots_empty_implausible",
        ));
    }
    Ok(addrs.into_iter().collect())
}

fn read_war_rows_from_validated_addrs<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
    war_row_addrs: &[u64],
    acc: &mut RowAccumulator,
) -> Result<(Vec<WarRelationRow>, usize), ExtractError> {
    let valid_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let date_vtables = domain53_war_date_vtables(module_base)?;
    let mut rows = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stale_dropped = 0usize;
    let prefetch_dist_candidates = prefetch_dist_candidates();
    for (row_index, &row_addr) in war_row_addrs.iter().enumerate() {
        if prefetch_dist_candidates != 0 {
            if let Some(&future_addr) = row_index
                .checked_add(prefetch_dist_candidates)
                .and_then(|i| war_row_addrs.get(i))
            {
                prefetch_read(future_addr);
            }
        }
        let Some(vtable_addr) = row_addr.checked_add(profile::DOMAIN53_WAR_DATE_VTABLE as u64)
        else {
            stale_dropped = stale_dropped.saturating_add(1);
            continue;
        };
        match mem.read_u64(vtable_addr) {
            Ok(vtable) if date_vtables.contains(&vtable) => {}
            Ok(_) | Err(_) => {
                stale_dropped = stale_dropped.saturating_add(1);
                continue;
            }
        }
        match read_war_row(mem, row_addr, &valid_tags) {
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
            Err(_) => acc.skip_war(),
        }
    }
    Ok((rows, stale_dropped))
}

fn read_war_row<M: Domain53Memory>(
    mem: &M,
    row_addr: u64,
    valid_tags: &BTreeSet<u32>,
) -> Result<WarRelationRow, ExtractError> {
    let first = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_FIRST_TAG)?)?;
    let second = mem.read_u32(add_addr(row_addr, profile::DOMAIN53_WAR_SECOND_TAG)?)?;
    if first == second || !valid_tags.contains(&first) || !valid_tags.contains(&second) {
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

fn stable_bytes<M: Domain53Memory>(mem: &M, address: u64, len: usize) -> Option<Vec<u8>> {
    let first = mem.read_bytes(address, len).ok()?;
    let second = mem.read_bytes(address, len).ok()?;
    (first == second).then_some(first)
}

fn operation_token_map_shape(bytes: &[u8]) -> Option<(u64, usize)> {
    if bytes.len() < 0x20 {
        return None;
    }
    let table = u64::from_le_bytes(bytes[8..16].try_into().ok()?);
    let mask = u32::from_le_bytes(bytes[0x14..0x18].try_into().ok()?) as usize;
    let overflow = bytes[0x18] as usize;
    let entries = mask.checked_add(overflow)?.checked_add(1)?;
    (is_heap_ptr(table) && entries != 0 && entries <= MAX_OPERATION_TOKEN_DATABASE_ENTRIES)
        .then_some((table, entries))
}

fn read_operation_token_memberships<M: Domain53Memory>(
    mem: &M,
    module_base: u64,
    countries: &[CountryRoot],
) -> Option<Vec<OperationTokenMembershipRow>> {
    let database_global = add_addr(module_base, OPERATION_TOKEN_DATABASE_GLOBAL_RVA).ok()?;
    let database_first = mem.read_u64(database_global).ok()?;
    let database_second = mem.read_u64(database_global).ok()?;
    if database_first != database_second || !is_heap_ptr(database_first) {
        return None;
    }
    let database_map = add_addr(database_first, OPERATION_TOKEN_DATABASE_MAP_OFFSET).ok()?;
    let database_header = stable_bytes(mem, database_map, 0x20)?;
    let (definition_table, definition_slots) = operation_token_map_shape(&database_header)?;
    let definition_len = definition_slots.checked_mul(OPERATION_TOKEN_DATABASE_ENTRY_LEN)?;
    let definition_bytes = stable_bytes(mem, definition_table, definition_len)?;
    if stable_bytes(mem, database_map, 0x20)? != database_header {
        return None;
    }

    let mut definitions = BTreeMap::new();
    let mut authored_ids = BTreeSet::new();
    for slot in 0..definition_slots {
        let at = slot.checked_mul(OPERATION_TOKEN_DATABASE_ENTRY_LEN)?;
        if definition_bytes[at + 4] == 0 {
            continue;
        }
        let numeric_id = u32::from_le_bytes(definition_bytes[at + 8..at + 12].try_into().ok()?);
        let definition =
            u64::from_le_bytes(definition_bytes[at + 0x10..at + 0x18].try_into().ok()?);
        if numeric_id == 0 || !is_heap_ptr(definition) || definitions.contains_key(&numeric_id) {
            return None;
        }
        let string_addr = add_addr(definition, OPERATION_TOKEN_DEFINITION_ID_OFFSET).ok()?;
        let first = read_msvc_string_snapshot(
            mem,
            string_addr,
            crate::compact::MAX_OPERATION_TOKEN_ID_LEN,
        )?;
        let second = read_msvc_string_snapshot(
            mem,
            string_addr,
            crate::compact::MAX_OPERATION_TOKEN_ID_LEN,
        )?;
        if first != second {
            return None;
        }
        let StringRead::Value(authored_id) = first.value else {
            return None;
        };
        if !authored_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
            || !authored_ids.insert(authored_id.clone())
        {
            return None;
        }
        definitions.insert(numeric_id, authored_id);
    }
    if definitions.is_empty()
        || stable_bytes(mem, definition_table, definition_len)? != definition_bytes
        || stable_bytes(mem, database_map, 0x20)? != database_header
    {
        return None;
    }

    let accepted_tags = countries
        .iter()
        .map(|country| country.tag_db_id)
        .collect::<BTreeSet<_>>();
    let mut memberships = BTreeSet::new();
    for country in countries {
        let manager_addr =
            add_addr(country.country_ptr, COUNTRY_OPERATION_TOKEN_MANAGER_OFFSET).ok()?;
        let manager_first = mem.read_u64(manager_addr).ok()?;
        let manager_second = mem.read_u64(manager_addr).ok()?;
        if manager_first != manager_second || !is_heap_ptr(manager_first) {
            return None;
        }
        if mem.read_u64(add_addr(manager_first, 8).ok()?).ok()? != country.country_ptr {
            return None;
        }
        let map_addr = add_addr(manager_first, OPERATION_TOKEN_MANAGER_MAP_OFFSET).ok()?;
        let manager_header = stable_bytes(mem, map_addr, 0x20)?;
        let (target_table, target_slots) = operation_token_map_shape(&manager_header)?;
        let target_len = target_slots.checked_mul(OPERATION_TOKEN_MANAGER_ENTRY_LEN)?;
        let target_bytes = stable_bytes(mem, target_table, target_len)?;
        if stable_bytes(mem, map_addr, 0x20)? != manager_header {
            return None;
        }
        for slot in 0..target_slots {
            let at = slot.checked_mul(OPERATION_TOKEN_MANAGER_ENTRY_LEN)?;
            if target_bytes[at + 4] == 0 {
                continue;
            }
            let target = u32::from_le_bytes(target_bytes[at + 8..at + 12].try_into().ok()?);
            let token_vector =
                u64::from_le_bytes(target_bytes[at + 0x10..at + 0x18].try_into().ok()?);
            let token_count =
                i32::from_le_bytes(target_bytes[at + 0x1c..at + 0x20].try_into().ok()?);
            if target == country.tag_db_id
                || !accepted_tags.contains(&target)
                || token_count <= 0
                || token_count as usize > MAX_OPERATION_TOKENS_PER_TARGET
                || !is_heap_ptr(token_vector)
            {
                return None;
            }
            let token_bytes = stable_bytes(mem, token_vector, token_count as usize * 4)?;
            let mut target_tokens = BTreeSet::new();
            for raw in token_bytes.chunks_exact(4) {
                let numeric_id = u32::from_le_bytes(raw.try_into().ok()?);
                let authored_id = definitions.get(&numeric_id)?.clone();
                if !target_tokens.insert(numeric_id)
                    || !memberships.insert(OperationTokenMembershipRow {
                        owner_tag_db_id: country.tag_db_id,
                        target_tag_db_id: target,
                        token_id: authored_id,
                    })
                {
                    return None;
                }
            }
        }
        if stable_bytes(mem, target_table, target_len)? != target_bytes
            || stable_bytes(mem, map_addr, 0x20)? != manager_header
            || mem.read_u64(manager_addr).ok()? != manager_first
        {
            return None;
        }
    }
    (memberships.len() <= crate::compact::MAX_OPERATION_TOKEN_ROWS)
        .then(|| memberships.into_iter().collect())
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum StringRead {
    Value(String),
    Missing,
    Invalid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MsvcStringSnapshot {
    size: u64,
    capacity: u64,
    chars_addr: Option<u64>,
    bytes: Vec<u8>,
    value: StringRead,
}

fn read_msvc_string_snapshot<M: Domain53Memory>(
    mem: &M,
    address: u64,
    max_len: usize,
) -> Option<MsvcStringSnapshot> {
    let size_raw = mem.read_u64(add_addr(address, 0x10).ok()?).ok()?;
    let capacity_raw = mem.read_u64(add_addr(address, 0x18).ok()?).ok()?;
    let size = size_raw as usize;
    let capacity = capacity_raw as usize;
    if size == 0 {
        return Some(MsvcStringSnapshot {
            size: size_raw,
            capacity: capacity_raw,
            chars_addr: None,
            bytes: Vec::new(),
            value: StringRead::Missing,
        });
    }
    if size > max_len || capacity < size || capacity > (1 << 20) {
        return Some(MsvcStringSnapshot {
            size: size_raw,
            capacity: capacity_raw,
            chars_addr: None,
            bytes: Vec::new(),
            value: StringRead::Invalid,
        });
    }
    let chars_addr = if capacity < 16 {
        address
    } else {
        let ptr = mem.read_u64(address).ok()?;
        if !is_heap_ptr(ptr) {
            return Some(MsvcStringSnapshot {
                size: size_raw,
                capacity: capacity_raw,
                chars_addr: Some(ptr),
                bytes: Vec::new(),
                value: StringRead::Invalid,
            });
        }
        ptr
    };
    let bytes = mem.read_bytes(chars_addr, size + 1).ok()?;
    let value = if bytes.get(size).copied() != Some(0) {
        StringRead::Invalid
    } else {
        match std::str::from_utf8(&bytes[..size]) {
            Ok(value) if !value.chars().any(char::is_control) => {
                StringRead::Value(value.to_string())
            }
            _ => StringRead::Invalid,
        }
    };
    Some(MsvcStringSnapshot {
        size: size_raw,
        capacity: capacity_raw,
        chars_addr: Some(chars_addr),
        bytes,
        value,
    })
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

#[inline(always)]
fn prefetch_read(addr: u64) {
    if !is_heap_ptr(addr) {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        use core::arch::x86_64::{_mm_prefetch, _MM_HINT_T0};
        // SAFETY: the `is_heap_ptr(addr)` guard above guarantees `addr` is inside
        unsafe {
            _mm_prefetch::<{ _MM_HINT_T0 }>(addr as *const i8);
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = addr;
    }
}

fn in_module(module_base: u64, value: u64) -> bool {
    (module_base..module_base.saturating_add(DOMAIN53_MODULE_SPAN_MAX)).contains(&value)
}

fn valid_tag_db_id(value: u32) -> bool {
    value > 0
        && compact_contract::bound_trips::gate_u32(
            value,
            MAX_TAG_DB_ID,
            compact_contract::bound_trips::BOUND_TRIP_TAG_DB_ID_1024,
        )
        .is_some()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PayloadPhaseTimes {
    pub snapshot_us: u64,
    pub validation_us: u64,
    pub encode_us: u64,
    pub summary_us: u64,
    pub decode_gate_us: u64,
    pub summary_base_rows_us: u64,
    pub summary_opinions_us: u64,
    pub summary_state_events_us: u64,
    pub summary_trailers_us: u64,
}

#[derive(Clone, Debug)]
pub struct Domain53ExtractionReport {
    pub domain_id: u16,
    pub source_proofs: u64,
    pub snapshot: DiplomacyIntelSnapshot,
    pub payload: Vec<u8>,
    pub gate_summary: Domain53GateSummary,
    pub errors: Vec<String>,
    pub payload_phase_times: PayloadPhaseTimes,
}

impl PartialEq for Domain53ExtractionReport {
    fn eq(&self, other: &Self) -> bool {
        self.domain_id == other.domain_id
            && self.source_proofs == other.source_proofs
            && self.snapshot == other.snapshot
            && self.payload == other.payload
            && self.gate_summary == other.gate_summary
            && self.errors == other.errors
    }
}
impl Eq for Domain53ExtractionReport {}

impl Domain53ExtractionReport {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

pub const DOMAIN53_ERROR_SUBJECT_AUTONOMY_COMPLETENESS: &str =
    "subject-autonomy current-status rows require a completeness witness";

const DECODE_VERIFY_SAMPLE_N: u64 = 32;

static DOMAIN53_BUILD_FRAME_COUNTER: AtomicU64 = AtomicU64::new(0);

fn should_sample_decode_verify() -> bool {
    let index = DOMAIN53_BUILD_FRAME_COUNTER.fetch_add(1, Ordering::Relaxed);
    index % DECODE_VERIFY_SAMPLE_N == 0
}

fn decode_verify_mismatch_error(
    encode_summary: &Domain53GateSummary,
    decode_summary: &Domain53GateSummary,
) -> Option<String> {
    if encode_summary == decode_summary {
        None
    } else {
        Some(format!(
            "domain{DOMAIN_ID_PROPOSED} encode-side gate summary disagrees with sampled decode (fail-closed): encode={encode_summary:?} decode={decode_summary:?}"
        ))
    }
}

pub fn build_domain53_payload(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
) -> Domain53ExtractionReport {
    build_domain53_payload_with_status(rows, source_proofs, 0)
}

pub fn build_domain53_payload_with_status(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
    extra_status_flags: u32,
) -> Domain53ExtractionReport {
    let sample_decode = should_sample_decode_verify();
    build_domain53_payload_sampled(rows, source_proofs, extra_status_flags, sample_decode)
}

fn build_domain53_payload_sampled(
    rows: ExtractedDomain53Rows,
    source_proofs: u64,
    extra_status_flags: u32,
    sample_decode: bool,
) -> Domain53ExtractionReport {
    let snapshot_started = Instant::now();
    let mut snapshot = DiplomacyIntelSnapshot {
        elapsed_wall_ms: rows.elapsed_wall_ms,
        status_flags: extra_status_flags,
        changed_rows: rows.changed_rows,
        countries: rows.countries,
        wars: rows.wars,
        opinions: rows.opinions,
        relation_flags: rows.relation_flags,
        subjects: rows.subjects,
        subject_autonomy_statuses: rows.subject_autonomy_statuses,
        subject_autonomy_status_complete: rows.subject_autonomy_status_complete,
        agencies: rows.agencies,
        agency_capacity_clusters: rows.agency_capacity_clusters,
        operatives: rows.operatives,
        cryptology: rows.cryptology,
        operation_anchors: rows.operation_anchors,
        intel_map: rows.intel_map,
        state_events: rows.state_events,
        operation_instances: rows.operation_instances,
        incoming_actions: rows.incoming_actions,
        intel_network_graphs: rows.intel_network_graphs,
        factions: rows.factions,
        faction_roster_complete: rows.faction_roster_complete,
        operation_tokens: rows.operation_tokens,
        operation_tokens_complete: rows.operation_tokens_complete,
        war_lifecycle: Vec::new(),
        war_lifecycle_complete: false,
        air_disclosure_threshold_raw: rows.air_disclosure_threshold_raw,
    };
    let snapshot_us = duration_micros_u64_saturating(snapshot_started.elapsed());

    let validation_started = Instant::now();
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
        !snapshot.incoming_actions.is_empty(),
        source_proofs,
        SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
        "incoming diplomatic action rows require SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF",
        &mut errors,
    );
    require_rows(
        !snapshot.intel_network_graphs.is_empty(),
        source_proofs,
        SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
        "intel-network graph rows require SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF",
        &mut errors,
    );
    if !snapshot.factions.is_empty() && !snapshot.faction_roster_complete {
        errors.push("faction roster rows require a completeness witness".to_string());
    }
    require_rows(
        snapshot.faction_roster_complete,
        source_proofs,
        SOURCE_PROOF_FACTION_SYSTEM_870EBF,
        "faction roster completeness witness requires SOURCE_PROOF_FACTION_SYSTEM_870EBF",
        &mut errors,
    );
    if !snapshot.subject_autonomy_statuses.is_empty() && !snapshot.subject_autonomy_status_complete
    {
        errors.push(DOMAIN53_ERROR_SUBJECT_AUTONOMY_COMPLETENESS.to_string());
    }
    require_rows(
        snapshot.subject_autonomy_status_complete
            || !snapshot.subject_autonomy_statuses.is_empty(),
        source_proofs,
        SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF,
        "subject-autonomy current-status witness requires SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF",
        &mut errors,
    );
    if !snapshot.operation_tokens.is_empty() && !snapshot.operation_tokens_complete {
        errors.push("operation-token rows require a completeness witness".to_string());
    }
    require_rows(
        snapshot.operation_tokens_complete || !snapshot.operation_tokens.is_empty(),
        source_proofs,
        SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF,
        "operation-token census requires SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF",
        &mut errors,
    );
    if has(
        source_proofs,
        SOURCE_PROOF_OPERATION_TOKEN_MEMBERSHIP_870EBF,
    ) != snapshot.operation_tokens_complete
    {
        errors.push("operation-token source proof and complete OPTK witness disagree".to_string());
    }
    if snapshot.subject_autonomy_status_complete
        && snapshot.status_flags & crate::compact::STATUS_DELTA_DATA == 0
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
            errors.push(
                "complete subject-autonomy current-status keys do not match subject base rows"
                    .to_string(),
            );
        }
    }
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

    if has(source_proofs, SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF) {
        snapshot.status_flags &= !STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
    } else {
        snapshot.status_flags |= STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED;
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

    let validation_us = duration_micros_u64_saturating(validation_started.elapsed());

    let encode_started = Instant::now();
    let payload = encode_payload(&snapshot);
    let encode_us = duration_micros_u64_saturating(encode_started.elapsed());

    let summary_started = Instant::now();
    let mut summary_phase = SummaryPhaseTimes::default();
    let gate_summary = summarize_snapshot_instrumented(0, &snapshot, &mut summary_phase);
    let summary_us = duration_micros_u64_saturating(summary_started.elapsed());

    let mut decode_gate_us = 0u64;
    if sample_decode {
        let decode_gate_started = Instant::now();
        let decode_summary = decode_payload(0, &payload);
        decode_gate_us = duration_micros_u64_saturating(decode_gate_started.elapsed());
        if let Some(mismatch) = decode_verify_mismatch_error(&gate_summary, &decode_summary) {
            errors.push(mismatch);
        }
    }

    errors.extend(gate_summary.parse_errors.iter().cloned());

    Domain53ExtractionReport {
        domain_id: DOMAIN_ID_PROPOSED,
        source_proofs,
        snapshot,
        payload,
        gate_summary,
        errors,
        payload_phase_times: PayloadPhaseTimes {
            snapshot_us,
            validation_us,
            encode_us,
            summary_us,
            decode_gate_us,
            summary_base_rows_us: summary_phase.base_rows_us,
            summary_opinions_us: summary_phase.opinions_us,
            summary_state_events_us: summary_phase.state_events_us,
            summary_trailers_us: summary_phase.trailers_us,
        },
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
            start_date_raw: 60_795_475,
        }],
        subject_autonomy_statuses: vec![SubjectAutonomyStatusRow {
            overlord_tag_db_id: 1,
            subject_tag_db_id: 22,
            current_state_hash: crate::compact::stable_hash32("autonomy_puppet"),
            previous_state_hash: Some(crate::compact::stable_hash32("autonomy_integrated_puppet")),
            next_state_hash: Some(crate::compact::stable_hash32("autonomy_colony")),
            ratio_raw: 40_000,
            current_score_raw: 40_000,
            lower_score_raw: 0,
            upper_score_raw: 100_000,
            last_modified_date_raw: 60_795_475,
        }],
        subject_autonomy_status_complete: true,
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
                state_hash: crate::compact::stable_hash32("assignment_3"),
                lifecycle_flag: None,
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_388,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(12_400_000),
                name_hash: crate::compact::stable_hash64("Pool Cohort"),
                state_hash: crate::compact::stable_hash32("assignment_0"),
                lifecycle_flag: None,
            },
            OperativeRow {
                owner_tag_db_id: 1,
                op_id: 11_389,
                detected: false,
                captured: false,
                skill: Some(1),
                experience_raw: Some(0),
                name_hash: crate::compact::stable_hash64("Assignment Cohort"),
                state_hash: crate::compact::stable_hash32("assignment_4"),
                lifecycle_flag: None,
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
        operation_instances: vec![
            OperationInstanceRow::legacy(129_014, true, Some(3)),
            OperationInstanceRow::legacy(128_904, true, None),
        ],
        incoming_actions: vec![
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
                action_id: 150_608,
                action_type: 0,
                date_hours: 60_830_024,
                last_command_date_hours: 60_830_024,
            },
        ],
        intel_network_graphs: vec![
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 2,
                state_ids: vec![
                    121, 122, 123, 125, 126, 127, 128, 129, 130, 131, 132, 133, 338, 931,
                ],
                state_strengths: None,
                coverage: None,
                operative_ids: None,
            },
            IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 1,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
                state_strengths: None,
                coverage: None,
                operative_ids: None,
            },
        ],
        factions: vec![FactionRow {
            faction_id: 1,
            faction_type: profile::DOMAIN53_FACTION_TYPE_EXPECTED,
            name_hash: crate::compact::stable_hash32("Axis"),
            leader_tag_db_id: 1,
            member_tag_db_ids: vec![1, 3, 22],
        }],
        faction_roster_complete: true,
        operation_tokens: Vec::new(),
        operation_tokens_complete: false,
        air_disclosure_threshold_raw: Some(30_000),
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

    #[test]
    fn parse_prefetch_dist_falls_back_when_absent_or_garbage() {
        assert_eq!(parse_prefetch_dist(None, 16), 16);
        assert_eq!(parse_prefetch_dist(None, 32), 32);
        assert_eq!(parse_prefetch_dist(Some(""), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("   "), 32), 32);
        assert_eq!(parse_prefetch_dist(Some("abc"), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("16x"), 16), 16);
        assert_eq!(parse_prefetch_dist(Some("-4"), 32), 32);
        assert_eq!(parse_prefetch_dist(Some("3.5"), 32), 32);
    }

    #[test]
    fn parse_prefetch_dist_returns_parsed_value() {
        assert_eq!(parse_prefetch_dist(Some("8"), 16), 8);
        assert_eq!(parse_prefetch_dist(Some("64"), 32), 64);
        assert_eq!(parse_prefetch_dist(Some("  24  "), 16), 24);
        assert_eq!(parse_prefetch_dist(Some("16"), 32), 16);
    }

    #[test]
    fn parse_prefetch_dist_honors_zero_disable() {
        assert_eq!(parse_prefetch_dist(Some("0"), 16), 0);
        assert_eq!(parse_prefetch_dist(Some("0"), 32), 0);
        assert_eq!(parse_prefetch_dist(Some(" 0 "), 32), 0);
    }

    #[test]
    fn prefetch_dist_overrides_win_reset_and_restore() {
        set_prefetch_dist_relation_override(Some(8));
        assert_eq!(prefetch_dist_relation(), 8);
        set_prefetch_dist_relation_override(Some(0));
        assert_eq!(prefetch_dist_relation(), 0);
        set_prefetch_dist_relation_override(Some(999_983));
        assert_eq!(prefetch_dist_relation(), 999_983);
        set_prefetch_dist_relation_override(None);
        assert_ne!(prefetch_dist_relation(), 999_983);
        set_prefetch_dist_candidates_override(Some(999_983));
        assert_eq!(prefetch_dist_candidates(), 999_983);
        set_prefetch_dist_candidates_override(None);
        assert_ne!(prefetch_dist_candidates(), 999_983);
        set_prefetch_dist_relation_override(Some(16));
        assert_eq!(prefetch_dist_relation(), 16);
        set_prefetch_dist_relation_override(None);
    }

    #[test]
    fn domain53_scan_needles_order_matches_discovery_fields() {
        let module_base = 0x1_4000_0000u64;
        let needles = domain53_scan_needles(module_base).expect("needles");
        assert_eq!(
            needles[0],
            add_addr(module_base, profile::DOMAIN53_OPERATIVE_VTABLE_RVA).unwrap()
        );
        assert_eq!(
            needles[1],
            add_addr(module_base, profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA).unwrap()
        );
        assert_eq!(
            needles[2],
            add_addr(
                module_base,
                profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA
            )
            .unwrap()
        );
        assert_eq!(
            needles[3],
            add_addr(
                module_base,
                profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA
            )
            .unwrap()
        );

        let discovery = domain53_discovery_from_needle_hits(vec![1], vec![4], vec![5], vec![6]);
        assert_eq!(discovery.operative_hits, vec![1]);
        assert_eq!(discovery.operation_instance_hits, vec![4]);
        assert_eq!(discovery.strategic_operative_hits, vec![5]);
        assert_eq!(discovery.country_intel_network_hits, vec![6]);
    }

    #[test]
    fn domain53_scan_needles_rejects_zero_module_base() {
        assert!(domain53_scan_needles(0).is_err());
    }

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

    struct StaleRelationHotWindowMemory {
        inner: FakeMemory,
        relation_entry: u64,
        stale_count: i32,
        stale_count_reads: usize,
        count_reads: std::cell::Cell<usize>,
        stale_opinion: i32,
    }

    impl StaleRelationHotWindowMemory {
        fn new(inner: FakeMemory, relation_entry: u64) -> Self {
            Self {
                inner,
                relation_entry,
                stale_count: -1,
                stale_count_reads: 0,
                count_reads: std::cell::Cell::new(0),
                stale_opinion: MAX_OPINION_ABS + 1,
            }
        }

        fn with_stale_count_reads(mut self, reads: usize) -> Self {
            self.stale_count_reads = reads;
            self
        }
    }

    impl Domain53Memory for StaleRelationHotWindowMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            let mut bytes = self.inner.read_bytes(address, len)?;
            if address == self.relation_entry + RELATION_COUNT_READ_START as u64
                && len == RELATION_COUNT_READ_LEN
            {
                let seen = self.count_reads.get();
                self.count_reads.set(seen + 1);
                if seen < self.stale_count_reads {
                    bytes.copy_from_slice(&self.stale_count.to_le_bytes());
                }
            }
            Ok(bytes)
        }

        fn read_into(&self, address: u64, buf: &mut [u8]) -> Result<(), ExtractError> {
            let bytes = self.inner.read_bytes(address, buf.len())?;
            buf.copy_from_slice(&bytes);
            if address == self.relation_entry + RELATION_HOT_FIELD_START as u64
                && buf.len() == RELATION_HOT_FIELD_LEN
            {
                buf[RELATION_HOT_OPINION_OFFSET..RELATION_HOT_OPINION_OFFSET + 4]
                    .copy_from_slice(&self.stale_opinion.to_le_bytes());
            }
            Ok(())
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    fn stale_relation_window_fixture() -> (FakeMemory, u64) {
        let module = 0x1_4000_0000u64;
        let relation_entry = profile::DOMAIN53_HEAP_LO as u64 + 0x50_0000;
        let mut inner = FakeMemory::new(module);
        inner.add_chunk(
            relation_entry,
            profile::DOMAIN53_RELATION_OPINION + std::mem::size_of::<i32>(),
        );
        inner.write_i32(
            relation_entry + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64,
            3,
        );
        inner.write_i32(
            relation_entry + profile::DOMAIN53_RELATION_OPINION as u64,
            42,
        );
        (inner, relation_entry)
    }

    #[test]
    fn invalid_wide_relation_sample_retries_exact_scalars_fail_closed() {
        let (inner, relation_entry) = stale_relation_window_fixture();
        let mem = StaleRelationHotWindowMemory::new(inner, relation_entry);
        let mut hot_buf = [0u8; RELATION_HOT_FIELD_LEN];
        let hot = read_relation_hot_fields(&mem, relation_entry, &mut hot_buf)
            .expect("hot-window sample remains readable");
        assert!(!valid_relation_opinion(hot.opinion));
        assert_eq!(
            read_relation_opinion_with_retry(&mem, relation_entry, Some(hot))
                .expect("valid exact opinion retry"),
            42
        );

        let mut invalid_inner = mem.inner.clone();
        invalid_inner.write_i32(
            relation_entry + profile::DOMAIN53_RELATION_OPINION as u64,
            MAX_OPINION_ABS + 1,
        );
        let persistently_invalid = StaleRelationHotWindowMemory::new(invalid_inner, relation_entry);
        assert!(
            read_relation_opinion_with_retry(&persistently_invalid, relation_entry, Some(hot))
                .is_err()
        );
    }

    #[test]
    fn separate_subrel_count_read_retries_once_and_fails_closed() {
        let (inner, relation_entry) = stale_relation_window_fixture();

        let clean = StaleRelationHotWindowMemory::new(inner.clone(), relation_entry);
        assert_eq!(
            read_relation_subrel_count_with_retry(&clean, relation_entry)
                .expect("clean count read"),
            3
        );
        assert_eq!(
            clean.count_reads.get(),
            1,
            "a valid first sample must not issue a retry read"
        );

        let transient = StaleRelationHotWindowMemory::new(inner.clone(), relation_entry)
            .with_stale_count_reads(1);
        assert_eq!(
            read_relation_subrel_count_with_retry(&transient, relation_entry)
                .expect("valid exact count retry"),
            3
        );
        assert_eq!(
            transient.count_reads.get(),
            2,
            "an out-of-range first sample must issue exactly one retry read"
        );

        let persistent = StaleRelationHotWindowMemory::new(inner, relation_entry)
            .with_stale_count_reads(usize::MAX);
        assert!(read_relation_subrel_count_with_retry(&persistent, relation_entry).is_err());
        assert_eq!(
            persistent.count_reads.get(),
            2,
            "the count retry must stay bounded at one retry"
        );
    }

    fn fake_subject_autonomy_status() -> (FakeMemory, u64, u64) {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x20_0000;
        let country = heap;
        let diplomacy = heap + 0x20_000;
        let status = heap + 0x30_000;
        let current_state = heap + 0x40_000;
        let previous_state = heap + 0x41_000;
        let next_state = heap + 0x42_000;
        let mut mem = FakeMemory::new(module);
        mem.add_chunk(country, profile::DOMAIN53_RELATION_MANAGER + 8);
        mem.add_chunk(
            diplomacy,
            profile::DOMAIN53_DIPLOMACY_CURRENT_AUTONOMY_STATUS + 8,
        );
        mem.add_chunk(status, profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_SIZE);
        for state in [current_state, previous_state, next_state] {
            mem.add_chunk(state, profile::DOMAIN53_SUBJECT_STATE_NAME + 0x20);
        }
        mem.write_u64(
            country + profile::DOMAIN53_RELATION_MANAGER as u64,
            diplomacy,
        );
        mem.write_u64(
            diplomacy + profile::DOMAIN53_DIPLOMACY_CURRENT_AUTONOMY_STATUS as u64,
            status,
        );
        mem.write_u64(
            status,
            module + profile::DOMAIN53_CURRENT_AUTONOMY_STATUS_VTABLE_RVA as u64,
        );
        mem.write_i64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_SCORE as u64,
            40_000,
        );
        mem.write_u64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_STATE as u64,
            current_state,
        );
        mem.write_u64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_PREVIOUS_STATE as u64,
            previous_state,
        );
        mem.write_i64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_LOWER_SCORE as u64,
            0,
        );
        mem.write_u64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_NEXT_STATE as u64,
            next_state,
        );
        mem.write_i64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_UPPER_SCORE as u64,
            100_000,
        );
        mem.write_u64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_SUBJECT_COUNTRY as u64,
            country,
        );
        mem.write_u32(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_LAST_MODIFIED_DATE as u64,
            60_795_475,
        );
        mem.write_inline_string(
            current_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
            "autonomy_puppet",
        );
        mem.write_inline_string(
            previous_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
            "autonomy_colony",
        );
        mem.write_inline_string(
            next_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
            "autonomy_free",
        );
        (mem, country, status)
    }

    #[test]
    fn reads_exact_current_subject_autonomy_status() {
        let (mem, country, _) = fake_subject_autonomy_status();
        let row = read_subject_autonomy_status(&mem, mem.module_base(), 1, 22, country)
            .expect("authenticated current autonomy status");
        assert_eq!(row.overlord_tag_db_id, 1);
        assert_eq!(row.subject_tag_db_id, 22);
        assert_eq!(
            row.current_state_hash,
            crate::compact::stable_hash32("autonomy_puppet")
        );
        assert_eq!(row.ratio_raw, 40_000);
        assert_eq!(row.current_score_raw, 40_000);
        assert_eq!(row.lower_score_raw, 0);
        assert_eq!(row.upper_score_raw, 100_000);
        assert_eq!(row.last_modified_date_raw, 60_795_475);
    }

    #[test]
    fn current_subject_autonomy_status_rejects_overflow_ratio_branch() {
        let (mut mem, country, status) = fake_subject_autonomy_status();
        mem.write_i64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_SCORE as u64,
            i64::MAX,
        );
        mem.write_i64(
            status + profile::DOMAIN53_CURRENT_AUTONOMY_LOWER_SCORE as u64,
            i64::MIN,
        );
        assert!(read_subject_autonomy_status(&mem, mem.module_base(), 1, 22, country).is_none());
    }

    #[test]
    fn current_subject_autonomy_source_is_listed_in_build_proof_roots() {
        let root = CURRENT_BUILD_ROOTS_870EBF
            .iter()
            .find(|root| root.name == "current_subject_autonomy_status")
            .expect("A16 source must be part of the emitted build proof");
        assert!(root.evidence.contains("diplomacy+0x350"));
        assert!(root.evidence.contains("module+0x27CD3A0"));
        assert!(root.evidence.contains("+0x50"));
    }

    struct TornFactionHeaderMemory {
        inner: FakeMemory,
        faction_count_addr: u64,
        faction_count_reads: std::cell::Cell<u32>,
    }

    impl Domain53Memory for TornFactionHeaderMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.faction_count_addr && len == 8 {
                let reads = self.faction_count_reads.get();
                self.faction_count_reads.set(reads.saturating_add(1));
                if reads != 0 {
                    return Ok((1u64 << 32).to_le_bytes().to_vec());
                }
            }
            self.inner.read_bytes(address, len)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    /// Serves a DIFFERENT invalid handle_type on every read of the watched
    /// address, so the two row attempts fail with different value-carrying
    /// errors - the signature of an object mutating mid-observation, which
    /// must stay a torn skip rather than a stable reject.
    struct FlakyHandleTypeMemory {
        inner: FakeMemory,
        handle_addr: u64,
        calls: std::cell::Cell<u32>,
    }

    impl Domain53Memory for FlakyHandleTypeMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            let mut bytes = self.inner.read_bytes(address, len)?;
            if address == self.handle_addr && len == 4 && bytes.len() == 4 {
                let call = self.calls.get();
                self.calls.set(call + 1);
                let value: u32 = if call % 2 == 0 { 0xDEAD_0001 } else { 0xDEAD_0002 };
                bytes.copy_from_slice(&value.to_le_bytes());
            }
            Ok(bytes)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    struct TornFactionValueMemory {
        inner: FakeMemory,
        address: u64,
        len: usize,
        replacement: Vec<u8>,
        reads: std::cell::Cell<u32>,
    }

    impl Domain53Memory for TornFactionValueMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            let request_end = address.checked_add(len as u64);
            let replacement_end = self.address.checked_add(self.len as u64);
            if address <= self.address
                && replacement_end.is_some_and(|end| request_end.is_some_and(|limit| end <= limit))
            {
                let reads = self.reads.get();
                self.reads.set(reads.saturating_add(1));
                if reads != 0 {
                    let mut bytes = self.inner.read_bytes(address, len)?;
                    let offset = (self.address - address) as usize;
                    bytes[offset..offset + self.len].copy_from_slice(&self.replacement);
                    return Ok(bytes);
                }
            }
            self.inner.read_bytes(address, len)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    struct CountryTableAllocationChangesAfterOuterValidationMemory {
        inner: FakeMemory,
        outer_root_addr: u64,
        country_table: u64,
        allocation_len: usize,
        changed_slot_offset: usize,
        replacement: Vec<u8>,
        outer_root_reads: std::cell::Cell<u32>,
        allocation_changed: std::cell::Cell<bool>,
    }

    impl Domain53Memory for CountryTableAllocationChangesAfterOuterValidationMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.outer_root_addr && len == 8 {
                let reads = self.outer_root_reads.get();
                self.outer_root_reads.set(reads.saturating_add(1));
                if reads != 0 {
                    self.allocation_changed.set(true);
                }
            }
            if address == self.country_table
                && len == self.allocation_len
                && self.allocation_changed.get()
            {
                let mut bytes = self.inner.read_bytes(address, len)?;
                bytes[self.changed_slot_offset..self.changed_slot_offset + 8]
                    .copy_from_slice(&self.replacement);
                return Ok(bytes);
            }
            self.inner.read_bytes(address, len)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    struct CountryTableBulkReadFailsMemory {
        inner: FakeMemory,
        country_table: u64,
        allocation_len: usize,
    }

    impl Domain53Memory for CountryTableBulkReadFailsMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.country_table && len == self.allocation_len {
                return Err(ExtractError::MemoryRead { address, len });
            }
            self.inner.read_bytes(address, len)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    fn test_war_row_addrs<M: Domain53Memory>(mem: &M) -> Result<Vec<u64>, ExtractError> {
        domain53_war_row_addrs_from_relation_roots(mem)
    }

    fn extract_rows_from_current_roots<M: Domain53Memory>(
        mem: &M,
    ) -> Result<ExtractedDomain53Frame, ExtractError> {
        let discovery = discover_domain53_heap_hits(mem)?;
        let war_addrs = test_war_row_addrs(mem)?;
        extract_rows_from_current_roots_with_validated_war_addrs(mem, &discovery, &war_addrs)
    }

    fn extract_rows_from_current_roots_bypassing_war_discovery<M: Domain53Memory>(
        mem: &M,
    ) -> Result<ExtractedDomain53Frame, ExtractError> {
        let discovery = discover_domain53_heap_hits(mem)?;
        extract_rows_from_current_roots_with_validated_war_addrs(mem, &discovery, &[])
    }

    fn extract_rows_from_current_roots_with_discovery<M: Domain53Memory>(
        mem: &M,
        discovery: &Domain53Discovery,
    ) -> Result<ExtractedDomain53Frame, ExtractError> {
        let war_addrs = test_war_row_addrs(mem)?;
        extract_rows_from_current_roots_with_validated_war_addrs(mem, discovery, &war_addrs)
    }

    fn wire_fake_war_relation(mem: &mut FakeMemory, viewer_country: u64, war_row: u64, slot: u32) {
        let module = mem.module_base;
        let aux = fake_heap_base() + 0x70_0000 + u64::from(slot) * 0x1_0000;
        let manager = aux;
        let table = aux + 0x1000;
        let relation = aux + 0x2000;
        let war_object = war_row - u64::from(profile::DOMAIN60_WAR_OBJECT_TO_ROW as u32);
        for (addr, len) in [
            (manager, 0x20usize),
            (table, (FAKE_COUNTRY_ROWS as usize + 1) * 8),
            (relation, profile::DOMAIN60_WAR_RELATION_OBJECT_PTR + 0x10),
        ] {
            mem.add_chunk(addr, len);
        }
        mem.add_chunk(war_object, 0x10);
        mem.write_u64(
            viewer_country + profile::DOMAIN53_RELATION_MANAGER as u64,
            manager,
        );
        mem.write_u64(manager + profile::DOMAIN60_WAR_RELATION_TABLE as u64, table);
        mem.write_u64(table, relation);
        mem.write_u64(
            relation + profile::DOMAIN60_WAR_RELATION_OBJECT_PTR as u64,
            war_object,
        );
        mem.write_u64(
            war_object,
            module + profile::DOMAIN60_WAR_CWARRELATION_VTABLE_RVA as u64,
        );
        mem.write_u8(war_object + profile::DOMAIN60_WAR_OBJECT_RETIRED as u64, 0);
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
        let incoming_queue2 = heap + 0x11_0000;
        let incoming_data2 = heap + 0x12_0000;
        let incoming_action2 = heap + 0x13_0000;
        let incoming_body2 = heap + 0x14_0000;
        let strategic_operative1 = heap + 0x15_0000;
        let strategic_network_vec1 = heap + 0x16_0000;
        let intel_network1 = heap + 0x17_0000;
        let intel_graph1 = heap + 0x18_0000;
        let intel_vertices1 = heap + 0x19_0000;
        let subject_object = heap + 0x1A_0000;
        let faction_system = heap + 0x1B_0000;
        let faction_array = heap + 0x1C_0000;
        let faction1 = heap + 0x1D_0000;
        let faction_members = heap + 0x1E_0000;
        let intel_graph_rows1 = heap + 0x1F_0000;
        let intel_subnetworks1 = heap + 0x1F_1000;
        let operative_action_wrapper1 = heap + 0x1F_4000;

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
            (game_state, 0x500),
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
            (incoming_queue2, 0x40),
            (incoming_data2, 0x08),
            (incoming_action2, 0x20),
            (incoming_body2, 0x50),
            (strategic_operative1, 0x40),
            (strategic_network_vec1, 0x08),
            (intel_network1, 0x100),
            (intel_graph1, 0x60),
            (
                intel_vertices1,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE * 3,
            ),
            (
                intel_graph_rows1,
                profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE * 3,
            ),
            (
                intel_subnetworks1,
                profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_STRIDE * 3,
            ),
            (operative_action_wrapper1, 0x20),
            (subject_object, 0x80),
            (faction_system, 0x40),
            (faction_array, 0x20),
            (faction1, 0x80),
            (faction_members, 0x20),
        ] {
            mem.add_chunk(addr, len);
        }
        if include_operative {
            mem.add_chunk(leader1, 0x1100);
            mem.add_range(leader1, 0x1100);
        }
        for tag in 4..=FAKE_COUNTRY_ROWS {
            mem.add_chunk(fake_country_addr(heap, tag), 0x1100);
        }
        mem.add_range(war1, 0x80);
        mem.add_range(strategic_operative1, 0x40);
        mem.add_range(intel_network1, 0x100);

        mem.write_u64(module + profile::CG_STATE_DATE_OFFSET as u64, game_state);
        mem.write_u32(
            game_state + profile::CGAMESTATE_CURRENT_HOUR as u64,
            60_830_441,
        );
        mem.write_u64(
            module + profile::COUNTRY_DATABASE_OFFSET as u64,
            country_database,
        );
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64,
            country_table,
        );
        mem.write_u32(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64 + 8,
            FAKE_COUNTRY_ROWS + 1,
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

        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64,
            faction_system,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_ARRAY as u64,
            faction_array,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_COUNT as u64,
            (1u64 << 32) | 1u64,
        );
        mem.write_u64(faction_array, faction1);
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_ID as u64,
            (1u64 << 32) | u64::from(profile::DOMAIN53_FACTION_TYPE_EXPECTED),
        );
        mem.write_inline_string(faction1 + profile::DOMAIN53_FACTION_NAME as u64, "Axis");
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS as u64,
            faction_members,
        );
        mem.write_u32(faction1 + profile::DOMAIN53_FACTION_MEMBERS_SIZE as u64, 2);
        mem.write_u64(faction_members, country1);
        mem.write_u64(faction_members + 8, country2);

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
            mem.write_i32(rel12 + profile::DOMAIN53_RELATION_OPINION as u64, 10);
            mem.write_u64(
                rel12 + profile::DOMAIN53_RELATION_SUBJECT_PTR as u64,
                subject_object,
            );
            mem.write_u64(
                subject_object,
                module + profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA as u64,
            );
            mem.write_u32(
                subject_object + profile::DOMAIN53_SUBJECT_OVERLORD as u64,
                1,
            );
            mem.write_u32(subject_object + profile::DOMAIN53_SUBJECT_SUBJECT as u64, 2);
            mem.write_u64(
                subject_object + profile::DOMAIN53_SUBJECT_STATE_DB as u64,
                autonomy_state,
            );
            mem.write_i64(
                subject_object + profile::DOMAIN53_SUBJECT_RATIO as u64,
                40_000,
            );
            mem.write_i32(
                subject_object + profile::DOMAIN53_SUBJECT_START_DATE as u64,
                60_795_475,
            );
            mem.write_inline_string(
                autonomy_state + profile::DOMAIN53_SUBJECT_STATE_NAME as u64,
                "aut_puppet",
            );

            let guarantee_arr = heap + 0x1F_0000;
            let guarantee_obj = heap + 0x1F_8000;
            mem.add_chunk(guarantee_arr, 0x40);
            mem.add_chunk(guarantee_obj, 0x40);
            mem.write_u64(
                rel13 + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64,
                guarantee_arr,
            );
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 1);
            mem.write_i32(rel13 + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(guarantee_arr, guarantee_obj);
            mem.write_u64(
                guarantee_obj,
                module + profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA as u64,
            );
            mem.write_u32(guarantee_obj + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
            mem.write_u32(guarantee_obj + profile::DOMAIN53_REL_BASE_SECOND as u64, 3);
            mem.write_i32(
                guarantee_obj + profile::DOMAIN53_REL_BASE_START_DATE as u64,
                60_000_000,
            );
            mem.write_i32(
                guarantee_obj + profile::DOMAIN53_REL_BASE_END_DATE as u64,
                profile::DOMAIN53_REL_DATE_SENTINEL,
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
            mem.write_inline_string(
                identity + profile::DOMAIN53_OPERATIVE_PORTRAIT_GFX as u64,
                "GFX_op_test",
            );
            mem.write_u32(
                identity + profile::DOMAIN53_OPERATIVE_ASSIGNMENT_ENUM as u64,
                3,
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
            let action = leader1 + profile::DOMAIN53_OPERATIVE_ACTION as u64;
            mem.write_u64(
                action + profile::DOMAIN53_OPERATIVE_ACTION_PAYLOAD_WRAPPER as u64,
                operative_action_wrapper1,
            );
            mem.write_u32(
                action + profile::DOMAIN53_OPERATIVE_ACTION_VARIANT as u64,
                profile::DOMAIN53_OPERATIVE_ACTION_BUILD_NETWORK,
            );
            mem.write_u64(
                operative_action_wrapper1
                    + profile::DOMAIN53_OPERATIVE_ACTION_NETWORK_TARGET as u64,
                intel_network1,
            );
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
        wire_fake_war_relation(&mut mem, country3, war1, 0);

        mem.write_u64(
            country2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE as u64,
            incoming_queue2,
        );
        mem.write_u64(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_DATA as u64,
            incoming_data2,
        );
        mem.write_u64(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP as u64,
            (1u64 << 32) | 1,
        );
        mem.write_u32(
            incoming_queue2 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD as u64,
            2,
        );
        mem.write_u64(incoming_data2, incoming_action2);
        mem.write_u64(
            incoming_action2,
            module + profile::DOMAIN53_INCOMING_ACTION_VTABLE_RVA as u64,
        );
        mem.write_u64(
            incoming_action2 + profile::DOMAIN53_INCOMING_ACTION_HANDLE as u64,
            (150_442u64 << 32) | u64::from(profile::DOMAIN53_INCOMING_ACTION_HANDLE_TYPE),
        );
        mem.write_u64(
            incoming_action2 + profile::DOMAIN53_INCOMING_ACTION_BODY as u64,
            incoming_body2,
        );
        mem.write_i32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_BODY_TYPE as u64,
            0,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ACTOR as u64,
            1,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR as u64,
            1,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_RECIPIENT as u64,
            2,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT as u64,
            2,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_DATE as u64,
            60_830_000,
        );
        mem.write_u32(
            incoming_body2 + profile::DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE as u64,
            60_830_000,
        );

        mem.write_u64(
            strategic_operative1,
            module + profile::DOMAIN53_STRATEGIC_OPERATIVE_VTABLE_RVA as u64,
        );
        mem.write_u64(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_INDEX as u64,
            1,
        );
        mem.write_u64(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_VECTOR as u64,
            strategic_network_vec1,
        );
        mem.write_u32(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_SIZE as u64,
            1,
        );
        mem.write_u32(
            strategic_operative1 + profile::DOMAIN53_STRATEGIC_OPERATIVE_NETWORK_CAPACITY as u64,
            1,
        );
        mem.write_u64(strategic_network_vec1, intel_network1);
        mem.write_u64(
            intel_network1,
            module + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_VTABLE_RVA as u64,
        );
        mem.write_i32(
            intel_network1 + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET as u64,
            3,
        );
        mem.write_u64(
            intel_network1 + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_GRAPH as u64,
            intel_graph1,
        );
        mem.write_u64(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_DATA as u64,
            intel_graph_rows1,
        );
        mem.write_u64(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_END as u64,
            intel_graph_rows1 + (profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE * 3) as u64,
        );
        mem.write_u64(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_DATA as u64,
            intel_vertices1,
        );
        mem.write_u32(
            intel_graph1 + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_COUNT as u64,
            3,
        );
        for (index, state_id) in [219u32, 253, 254].iter().enumerate() {
            let vertex = intel_vertices1
                + (index * profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE) as u64;
            mem.write_i32(
                vertex + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID as u64,
                *state_id as i32,
            );
            mem.write_u64(
                vertex + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_ROW_INDEX as u64,
                index as u64,
            );
            mem.write_i32(
                intel_graph_rows1
                    + (index * profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_STRIDE) as u64
                    + profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_SUBNETWORK_SELECTOR as u64,
                index as i32,
            );
            mem.write_i64(
                intel_subnetworks1
                    + (index * profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_STRIDE) as u64
                    + profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_STRENGTH as u64,
                [25_000, 50_000, 100_000][index],
            );
        }
        mem.write_u64(
            intel_network1 + profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_DATA as u64,
            intel_subnetworks1,
        );
        mem.write_u32(
            intel_network1 + profile::DOMAIN53_INTEL_NETWORK_SUBNETWORK_SIZE as u64,
            3,
        );

        mem
    }

    fn fake_heap_base() -> u64 {
        profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000
    }

    fn add_fake_operative(mem: &mut FakeMemory, index: u32) -> u64 {
        let heap = fake_heap_base();
        let leader = heap + 0x30_0000 + u64::from(index) * 0x2000;
        let skill = leader + 0x1800;
        mem.add_chunk(leader, 0x1100);
        mem.add_chunk(skill, 0x200);
        mem.add_range(leader, 0x1100);

        let identity = leader + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
        mem.write_u64(
            leader,
            mem.module_base + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
        mem.write_u32(identity + 4, 20_000 + index);
        let name = format!("Op{index}");
        mem.write_inline_string(identity + profile::DOMAIN53_OPERATIVE_NAME as u64, &name);
        mem.write_inline_string(
            identity + profile::DOMAIN53_OPERATIVE_NAME_DUPLICATE as u64,
            &name,
        );
        mem.write_u8(identity + profile::DOMAIN53_OPERATIVE_DETECTED as u64, 0);
        mem.write_inline_string(
            identity + profile::DOMAIN53_OPERATIVE_PORTRAIT_GFX as u64,
            "GFX_op_test",
        );
        mem.write_u32(
            identity + profile::DOMAIN53_OPERATIVE_ASSIGNMENT_ENUM as u64,
            3,
        );
        mem.write_u32(identity + profile::DOMAIN53_OPERATIVE_OWNER as u64, 1);
        mem.write_u64(
            leader + profile::DOMAIN53_OPERATIVE_SKILL_SUBOBJECT as u64,
            skill,
        );
        mem.write_i64(
            leader + profile::DOMAIN53_OPERATIVE_EXPERIENCE_RAW as u64,
            70_000_000 + i64::from(index),
        );
        mem.write_u32(skill + profile::DOMAIN53_OPERATIVE_SKILL_LEVEL as u64, 2);
        leader
    }

    fn add_fake_war(mem: &mut FakeMemory, index: u32) -> u64 {
        let heap = fake_heap_base();
        let war = heap + 0x40_0000 + u64::from(index) * 0x100;
        let second_tag = 10 + index;
        assert!(second_tag <= FAKE_COUNTRY_ROWS);
        mem.add_chunk(war, 0x80);
        mem.add_range(war, 0x80);

        mem.write_u32(war + profile::DOMAIN53_WAR_FIRST_TAG as u64, 1);
        mem.write_u32(war + profile::DOMAIN53_WAR_SECOND_TAG as u64, second_tag);
        mem.write_u64(
            war + profile::DOMAIN53_WAR_DATE_VTABLE as u64,
            mem.module_base + profile::DOMAIN53_WAR_GAME_DATE_VTABLE_RVA as u64,
        );
        mem.write_i32(
            war + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64,
            100 + index as i32,
        );
        mem.write_i32(
            war + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64,
            200 + index as i32,
        );
        mem.write_u32(war + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 1);
        mem.write_u32(war + profile::DOMAIN53_WAR_ECHO_SECOND as u64, second_tag);
        let viewer = fake_country_addr(heap, second_tag);
        wire_fake_war_relation(mem, viewer, war, index + 1);
        war
    }

    fn add_fake_intel_map_carriers(mem: &mut FakeMemory) {
        let module = mem.module_base;
        let heap = fake_heap_base();
        let denominators = heap + 0x60_0000;
        let intel_table = heap + 0x60_1000;
        let intel_data = heap + 0x60_2000;
        mem.add_chunk(
            module + profile::DOMAIN53_INTEL_LEVEL_DENOMINATORS_RVA as u64,
            8,
        );
        mem.add_chunk(module + profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA as u64, 1);
        mem.add_chunk(denominators, 32);
        mem.add_chunk(intel_table, 0x20);
        mem.add_chunk(intel_data, profile::DOMAIN53_INTEL_ROW_STRIDE);
        mem.write_u64(
            module + profile::DOMAIN53_INTEL_LEVEL_DENOMINATORS_RVA as u64,
            denominators,
        );
        for index in 0..4 {
            mem.write_i64(denominators + (index * 8) as u64, 100_000);
        }
        mem.write_u8(module + profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA as u64, 0);
        mem.write_u64(
            intel_table + profile::DOMAIN53_INTEL_TABLE_DATA as u64,
            intel_data,
        );
        mem.write_u32(intel_table + profile::DOMAIN53_INTEL_TABLE_COUNT as u64, 1);
        for tag in 1..=FAKE_COUNTRY_ROWS {
            let country = match tag {
                1 => heap + 0x3_0000,
                2 => heap + 0x4_0000,
                3 => heap + 0x5_0000,
                _ => fake_country_addr(heap, tag),
            };
            mem.write_u64(
                country + profile::DOMAIN53_COUNTRY_INTEL_TABLE as u64,
                intel_table,
            );
            mem.write_u32(country + profile::DOMAIN53_COUNTRY_CID_INDEX as u64, 0);
        }
    }

    #[test]
    fn typed_family_read_preserves_production_rows_and_proof_gate() {
        let mut proven = fake_domain53_memory(true, true);
        add_fake_intel_map_carriers(&mut proven);
        let extracted =
            extract_rows_from_current_roots(&proven).expect("proven intel-map extraction");
        assert_eq!(
            extracted.rows.intel_map.len(),
            (FAKE_COUNTRY_ROWS * FAKE_COUNTRY_ROWS) as usize
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
            0
        );

        let mut unavailable = proven.clone();
        unavailable.write_u64(
            fake_heap_base() + 0x3_0000 + profile::DOMAIN53_COUNTRY_INTEL_TABLE as u64,
            0,
        );
        let extracted = extract_rows_from_current_roots(&unavailable)
            .expect("unavailable intel-map extraction remains rowless");
        assert!(extracted.rows.intel_map.is_empty());
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF,
            0
        );
        assert_eq!(
            extracted
                .rejection_counts
                .intel_map(IntelMapFailureCause::ViewerIntelTableUnavailable),
            1
        );
    }

    #[test]
    fn rejection_cause_vocabulary_is_dense_and_recordable() {
        let mut counts = Domain53RejectionCounts::default();
        for (index, cause) in OperationSemanticCause::ALL.into_iter().enumerate() {
            assert_eq!(cause as usize, index);
            counts.record_operation(cause);
            assert_eq!(counts.operation(cause), 1);
        }
        for (index, cause) in IntelMapFailureCause::ALL.into_iter().enumerate() {
            assert_eq!(cause as usize, index);
            counts.record_intel_map(cause);
            assert_eq!(counts.intel_map(cause), 1);
        }
        for (index, cause) in IntelNetworkFailureCause::ALL.into_iter().enumerate() {
            assert_eq!(cause as usize, index);
            counts.record_intel_network(cause);
            assert_eq!(counts.intel_network(cause), 1);
        }
    }

    #[test]
    fn each_declared_cause_has_exactly_one_syntactic_reader_site() {
        let source = include_str!("extractor.rs");
        let operation = source
            .split_once("fn read_operations<")
            .and_then(|(_, tail)| tail.split_once("fn read_operation_country_pointer<"))
            .map(|(body, _)| body)
            .expect("read_operations source body");
        let network = source
            .split_once("fn read_intel_network_graph_rows<")
            .and_then(|(_, tail)| tail.split_once("fn read_intel_network_coverage<"))
            .map(|(body, _)| body)
            .expect("read_intel_network_graph_rows source body");
        let intel_map = source
            .split_once("fn read_intel_map_rows<")
            .and_then(|(_, tail)| tail.split_once("fn intel_full_visibility<"))
            .map(|(body, _)| body)
            .expect("read_intel_map_rows source body");

        for cause in OperationSemanticCause::ALL {
            let site = format!("reject_semantic(OperationSemanticCause::{cause:?})");
            assert_eq!(
                operation.matches(&site).count(),
                1,
                "{cause:?} must have exactly one read_operations rejection site"
            );
        }
        for cause in IntelMapFailureCause::ALL {
            let site = format!("IntelMapFailureCause::{cause:?}");
            assert_eq!(
                intel_map.matches(&site).count(),
                1,
                "{cause:?} must have exactly one read_intel_map_rows rejection site"
            );
        }
        for cause in IntelNetworkFailureCause::ALL {
            let site = format!("IntelNetworkFailureCause::{cause:?}");
            assert_eq!(
                network.matches(&site).count(),
                1,
                "{cause:?} must have exactly one read_intel_network_graph_rows rejection site"
            );
        }
    }

    #[test]
    fn intel_map_reader_reports_exact_fake_memory_causes() {
        let mem = fake_domain53_memory(true, true);
        assert_eq!(
            read_intel_map_rows(&mem, mem.module_base, &[]).unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::CountriesEmpty)
        );
        let roots = resolve_domain53_frame_roots(&mem).expect("fake country roots");
        assert_eq!(
            read_intel_map_rows(&mem, mem.module_base, &roots.countries).unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::DenominatorsRootUnavailable)
        );

        let mut valid = mem.clone();
        add_fake_intel_map_carriers(&mut valid);
        let valid_roots = resolve_domain53_frame_roots(&valid).expect("valid map roots");
        let proven =
            read_intel_map_rows(&valid, valid.module_base, &valid_roots.countries).unwrap();
        let FamilyRead::Proven(rows) = proven else {
            panic!("valid intel-map carriers must produce a proven family");
        };
        assert_eq!(rows.len(), (FAKE_COUNTRY_ROWS * FAKE_COUNTRY_ROWS) as usize);

        let denominators = fake_heap_base() + 0x60_0000;
        let denominator_causes = [
            IntelMapFailureCause::CivilianDenominatorInvalid,
            IntelMapFailureCause::ArmyDenominatorInvalid,
            IntelMapFailureCause::NavyDenominatorInvalid,
            IntelMapFailureCause::AirDenominatorInvalid,
        ];
        for (index, expected) in denominator_causes.into_iter().enumerate() {
            let mut rejected = valid.clone();
            rejected.write_i64(denominators + (index * 8) as u64, 0);
            assert_eq!(
                read_intel_map_rows(&rejected, rejected.module_base, &valid_roots.countries)
                    .unwrap(),
                FamilyRead::Unavailable(expected)
            );
        }

        let mut no_fog_unreadable = valid.clone();
        let no_fog_addr =
            no_fog_unreadable.module_base + profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA as u64;
        no_fog_unreadable
            .chunks
            .retain(|(base, _)| *base != no_fog_addr);
        assert_eq!(
            read_intel_map_rows(
                &no_fog_unreadable,
                no_fog_unreadable.module_base,
                &valid_roots.countries
            )
            .unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::NoFogFlagUnreadable)
        );

        let mut table_unavailable = valid.clone();
        table_unavailable.write_u64(
            fake_heap_base() + 0x3_0000 + profile::DOMAIN53_COUNTRY_INTEL_TABLE as u64,
            0,
        );
        assert_eq!(
            read_intel_map_rows(
                &table_unavailable,
                table_unavailable.module_base,
                &valid_roots.countries
            )
            .unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::ViewerIntelTableUnavailable)
        );

        let mut invalid_lane = valid.clone();
        let no_fog_addr = invalid_lane.module_base + profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA as u64;
        invalid_lane.write_u8(no_fog_addr, 1);
        assert_eq!(
            invalid_lane.read_u8(no_fog_addr).unwrap(),
            1,
            "M3 must disable the full-visibility no-fog shortcut"
        );
        invalid_lane.write_i64(fake_heap_base() + 0x60_2000, -1);
        assert_eq!(
            read_intel_map_rows(
                &invalid_lane,
                invalid_lane.module_base,
                &valid_roots.countries
            )
            .unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::IntelLaneValueInvalid)
        );
    }

    #[test]
    fn intel_lane_scale_overflow_refuses_the_family_instead_of_emitting_a_wrapped_value() {
        let mut valid = fake_domain53_memory(true, true);
        add_fake_intel_map_carriers(&mut valid);
        let roots = resolve_domain53_frame_roots(&valid).expect("valid map roots");
        let no_fog_addr = valid.module_base + profile::DOMAIN53_INTEL_NO_FOG_FLAG_RVA as u64;
        valid.write_u8(no_fog_addr, 1);

        let largest_exact = i64::MAX / i64::from(FIXED_SCALE_RAW);
        let denominators = fake_heap_base() + 0x60_0000;
        let mut boundary = valid.clone();
        boundary.write_i64(denominators, largest_exact);
        boundary.write_i64(fake_heap_base() + 0x60_2000, largest_exact);
        let FamilyRead::Proven(rows) =
            read_intel_map_rows(&boundary, boundary.module_base, &roots.countries).unwrap()
        else {
            panic!("a product that fits i64 must still be emitted");
        };
        assert_eq!(rows[0].civilian_raw, FIXED_SCALE_RAW as i32);

        let mut overflowing = valid.clone();
        overflowing.write_i64(denominators, largest_exact);
        overflowing.write_i64(fake_heap_base() + 0x60_2000, largest_exact + 1);
        assert_eq!(
            read_intel_map_rows(&overflowing, overflowing.module_base, &roots.countries).unwrap(),
            FamilyRead::Unavailable(IntelMapFailureCause::IntelLaneScaleOverflow)
        );
    }

    #[test]
    fn air_disclosure_threshold_is_read_from_the_image_and_absent_when_unreadable() {
        assert_eq!(profile::DOMAIN53_AIR_DISCLOSURE_THRESHOLD_RVA, 0x331B588);

        let mut mem = fake_domain53_memory(true, true);
        add_fake_intel_map_carriers(&mut mem);
        assert_eq!(
            read_air_disclosure_threshold(&mem, mem.module_base),
            None,
            "no mapped define is not a zero threshold"
        );
        let extracted =
            extract_rows_from_current_roots(&mem).expect("extraction without the define");
        assert_eq!(extracted.rows.air_disclosure_threshold_raw, None);

        let threshold_addr =
            mem.module_base + profile::DOMAIN53_AIR_DISCLOSURE_THRESHOLD_RVA as u64;
        mem.add_chunk(threshold_addr, 8);
        mem.write_i64(threshold_addr, 0x0000_0007_0000_2710);
        assert_eq!(
            read_air_disclosure_threshold(&mem, mem.module_base),
            Some(0x0000_0007_0000_2710)
        );
        let extracted = extract_rows_from_current_roots(&mem).expect("extraction with the define");
        assert_eq!(
            extracted.rows.air_disclosure_threshold_raw,
            Some(0x0000_0007_0000_2710)
        );

        for value in [i64::MIN, -1, 0, 100_000, i64::MAX] {
            mem.write_i64(threshold_addr, value);
            assert_eq!(
                read_air_disclosure_threshold(&mem, mem.module_base),
                Some(value)
            );
        }
    }

    #[test]
    fn intel_network_reader_reports_exact_fake_memory_causes() {
        let mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("network country roots");
        let discovery = discover_domain53_heap_hits(&mem).expect("network discovery");
        let extracted =
            extract_rows_from_current_roots_with_discovery(&mem, &discovery).expect("baseline");
        let read = |memory: &FakeMemory, countries: &[CountryRoot]| {
            read_intel_network_graph_rows(
                memory,
                memory.module_base,
                countries,
                &extracted.rows.operatives,
                &discovery.operative_hits,
                &discovery.strategic_operative_hits,
                &discovery.country_intel_network_hits,
            )
            .unwrap()
        };
        assert!(matches!(
            read(&mem, &roots.countries),
            FamilyRead::Proven(_)
        ));
        assert_eq!(
            read(&mem, &[]),
            FamilyRead::Unavailable(IntelNetworkFailureCause::CountriesEmpty)
        );

        let network = fake_heap_base() + 0x17_0000;
        let mut bad_vtable = mem.clone();
        bad_vtable.write_u64(network, 0);
        assert_eq!(
            read(&bad_vtable, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::NetworkVtableInvalid)
        );

        let graph = fake_heap_base() + 0x18_0000;
        let mut reversed_rows = mem.clone();
        reversed_rows.write_u64(
            graph + profile::DOMAIN53_INTEL_NETWORK_GRAPH_ROW_END as u64,
            fake_heap_base() + 0x1F_0000 - 1,
        );
        assert_eq!(
            read(&reversed_rows, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::GraphRowRangeReversed)
        );

        let mut unknown_target = mem.clone();
        unknown_target.write_i32(
            network + profile::DOMAIN53_COUNTRY_INTEL_NETWORK_TARGET as u64,
            i32::MAX,
        );
        assert_eq!(
            read(&unknown_target, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::NetworkTargetUnknown)
        );

        let vertices = fake_heap_base() + 0x19_0000;
        let mut duplicate_state = mem.clone();
        duplicate_state.write_i32(
            vertices
                + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STRIDE as u64
                + profile::DOMAIN53_INTEL_NETWORK_GRAPH_VERTEX_STATE_ID as u64,
            219,
        );
        assert_eq!(
            read(&duplicate_state, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::DuplicateStateId)
        );

        let leader = fake_heap_base() + 0xE_0000;
        let mut invalid_action_wrapper = mem.clone();
        invalid_action_wrapper.write_u64(
            leader
                + profile::DOMAIN53_OPERATIVE_ACTION as u64
                + profile::DOMAIN53_OPERATIVE_ACTION_PAYLOAD_WRAPPER as u64,
            0,
        );
        assert_eq!(
            read(&invalid_action_wrapper, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::OperativeActionWrapperInvalid)
        );

        let strategic = fake_heap_base() + 0x15_0000;
        let mut owner_equals_target = mem.clone();
        owner_equals_target.write_u32(
            strategic + profile::DOMAIN53_STRATEGIC_OPERATIVE_OWNER_TAG_DB_ID as u64,
            3,
        );
        assert_eq!(
            read(&owner_equals_target, &roots.countries),
            FamilyRead::Unavailable(IntelNetworkFailureCause::NetworkOwnerEqualsTarget)
        );
    }

    #[test]
    fn reader_extracts_validated_slice_from_fake_memory() {
        let mem = fake_domain53_memory(true, true);
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");

        assert_eq!(
            extracted.skipped_rows, 0,
            "unexpected D53 family skips: {:?}",
            extracted.skip_counts
        );
        assert_eq!(extracted.rows.countries.len(), FAKE_COUNTRY_ROWS as usize);
        assert_eq!(extracted.rows.opinions.len(), 2);
        assert_eq!(extracted.rows.relation_flags.len(), 2);
        assert!(extracted.rows.relation_flags.iter().any(|row| {
            row.src_tag_db_id == 1 && row.target_tag_db_id == 2 && row.flag_kind == REL_KIND_PUPPET
        }));
        assert!(extracted.rows.relation_flags.iter().any(|row| {
            row.src_tag_db_id == 1
                && row.target_tag_db_id == 3
                && row.flag_kind == REL_KIND_GUARANTEE
                && row.directed
        }));
        assert!(extracted.rows.state_events.iter().any(|e| e.source_kind
            == STATE_EVENT_SOURCE_RELATION_FLAG
            && e.value_raw == 60_000_000));
        assert_eq!(extracted.rows.subjects.len(), 1);
        assert_eq!(extracted.rows.subjects[0].overlord_tag_db_id, 1);
        assert_eq!(extracted.rows.subjects[0].subject_tag_db_id, 2);
        assert_eq!(extracted.rows.subjects[0].start_date_raw, 60_795_475);
        assert_eq!(extracted.rows.factions.len(), 1);
        assert_eq!(extracted.rows.factions[0].faction_id, 1);
        assert_eq!(extracted.rows.factions[0].faction_type, 88);
        assert_eq!(extracted.rows.factions[0].leader_tag_db_id, 1);
        assert_eq!(extracted.rows.factions[0].member_tag_db_ids, vec![1, 2]);
        assert!(extracted.rows.faction_roster_complete);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
        assert_eq!(extracted.rows.agencies.len(), 1);
        assert_eq!(extracted.rows.agency_capacity_clusters.len(), 1);
        assert_eq!(extracted.rows.cryptology.len(), 1);
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(
            extracted.rows.operatives[0].state_hash,
            crate::compact::stable_hash32("assignment_3")
        );
        assert_eq!(extracted.rows.operatives[0].lifecycle_flag, None);
        assert!(!extracted
            .rows
            .state_events
            .iter()
            .any(|row| row.source_kind == STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE));
        assert_eq!(extracted.rows.wars.len(), 1);
        assert_eq!(
            extracted.rows.incoming_actions,
            vec![IncomingActionQueueRow {
                recipient_tag_db_id: 2,
                actor_tag_db_id: 1,
                action_id: 150_442,
                action_type: 0,
                date_hours: 60_830_000,
                last_command_date_hours: 60_830_000,
            }]
        );
        assert!(extracted.rows.operation_anchors.is_empty());
        assert!(extracted.rows.intel_map.is_empty());
        assert_eq!(
            extracted.rows.intel_network_graphs,
            vec![IntelNetworkGraphRow {
                strategic_operative_index: 1,
                network_slot: 0,
                target_tag_db_id: 3,
                state_ids: vec![219, 253, 254],
                state_strengths: Some(vec![
                    IntelNetworkStateStrength {
                        state_id: 219,
                        strength_raw: 25_000,
                    },
                    IntelNetworkStateStrength {
                        state_id: 253,
                        strength_raw: 50_000,
                    },
                    IntelNetworkStateStrength {
                        state_id: 254,
                        strength_raw: 100_000,
                    },
                ]),
                coverage: None,
                operative_ids: Some(vec![11_387]),
            }]
        );
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_BUILD_SHA_870EBF, 0);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_GROUP_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
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
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATION_DETAIL_870EBF,
            0
        );
        assert_eq!(
            extracted.source_proofs
                & (SOURCE_PROOF_INTEL_MAP_PERCENTAGES_870EBF
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
    fn cryptology_sparse_foreign_vtable_slot_is_measured_inactive_not_partial() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let crypto_records = heap + 0xD_0000;
        mem.write_u64(crypto_records, mem.module_base + 0x12_3456);

        let extracted =
            extract_rows_from_current_roots(&mem).expect("foreign sparse slot is readable");
        assert!(extracted.rows.cryptology.is_empty());
        assert_eq!(extracted.skip_counts.cryptology, 0);
        assert_eq!(
            extracted.skipped_rows, 0,
            "unexpected D53 family skips: {:?}",
            extracted.skip_counts
        );
    }

    #[test]
    fn authenticated_cryptology_row_with_unknown_target_remains_partial() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let crypto_records = heap + 0xD_0000;
        mem.write_u32(
            crypto_records + profile::DOMAIN53_CRYPTOLOGY_RECORD_TARGET as u64,
            FAKE_COUNTRY_ROWS + 10,
        );

        let extracted = extract_rows_from_current_roots(&mem).expect("bad known row is row-local");
        assert!(extracted.rows.cryptology.is_empty());
        assert_eq!(extracted.skip_counts.cryptology, 1);
        assert_eq!(extracted.skipped_rows, 1);
    }

    #[test]
    fn operative_candidate_invalid_coherently_is_stable_reject_not_partial() {
        // kills: pw_gate_final d53 partial-everywhere - permanent needle-hit
        // garbage (freed slot retaining the operative vtable qword; identity
        // head stably invalid) counted as a skipped DATA row on every frame.
        let mut mem = fake_domain53_memory(true, true);
        let dead = fake_heap_base() + 0x40_0000;
        mem.add_chunk(dead, 0x2000);
        mem.add_range(dead, 0x2000);
        let module = mem.module_base;
        mem.write_u64(
            dead,
            module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        // Identity head left zeroed: handle_type reads 0 != the expected
        // constant, identically (same value-carrying error), on both attempts.
        let extracted = extract_rows_from_current_roots(&mem)
            .expect("a stably-invalid candidate must not fail the frame");
        assert_eq!(
            extracted.stable_rejects.operative, 1,
            "the dead candidate must classify as stable reject"
        );
        assert_eq!(
            extracted.skipped_rows, 0,
            "stable rejects are not data omissions: {:?}",
            extracted.skip_counts
        );
    }

    #[test]
    fn operative_candidate_mutating_mid_observation_stays_torn_skip() {
        // The deciding field changes between the two attempts (handle_type
        // 0xDEAD01 -> 0xDEAD02): the value-carrying errors differ, so the
        // re-observation cannot prove stability and the candidate stays a
        // torn skip - refuse-by-name preserved.
        let mut inner = fake_domain53_memory(true, true);
        let dead = fake_heap_base() + 0x40_0000;
        inner.add_chunk(dead, 0x2000);
        inner.add_range(dead, 0x2000);
        let module = inner.module_base;
        inner.write_u64(
            dead,
            module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        let identity = dead + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
        let mem = FlakyHandleTypeMemory {
            inner,
            handle_addr: identity,
            calls: std::cell::Cell::new(0),
        };
        let extracted = extract_rows_from_current_roots(&mem)
            .expect("a torn candidate refuses row-local, not the frame");
        assert_eq!(extracted.stable_rejects.operative, 0);
        assert_eq!(extracted.skip_counts.operative, 1);
        assert_eq!(extracted.skipped_rows, 1);
    }

    /// Order-sensitive tail-head guard probe. Serves the watched 16-byte
    /// identity window unchanged on its FIRST read (the before-attempt
    /// sample) and flips its first byte on the second read ONLY IF both row
    /// attempts have already run (two handle reads observed). With the R5
    /// ordering defect - the final sample taken between the attempts - the
    /// second window read sees an unflipped buffer, the bracket wrongly
    /// agrees, and the test's expectation of a skip fails.
    struct MutatingTailHeadMemory {
        inner: FakeMemory,
        window_addr: u64,
        window_len: usize,
        handle_addr: u64,
        window_reads: std::cell::Cell<u32>,
        handle_reads: std::cell::Cell<u32>,
    }

    impl Domain53Memory for MutatingTailHeadMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.handle_addr && len == 4 {
                self.handle_reads.set(self.handle_reads.get() + 1);
                return self.inner.read_bytes(address, len);
            }
            let mut bytes = self.inner.read_bytes(address, len)?;
            if address == self.window_addr && len == self.window_len {
                let reads = self.window_reads.get();
                self.window_reads.set(reads + 1);
                if reads >= 1 && self.handle_reads.get() >= 2 {
                    if let Some(slot) = bytes.first_mut() {
                        *slot ^= 0xFF;
                    }
                }
            }
            Ok(bytes)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    /// Order-sensitive vtable guard probe. Serves the operative vtable on the
    /// FIRST read of the watched address (the entry check) and the foreign
    /// value on the post-attempt re-observation ONLY IF both row attempts
    /// have already run. Under the R5 ordering defect the re-observation
    /// lands between the attempts, still sees the true vtable, and the
    /// resulting wrongful stable reject fails the test's skip expectation.
    struct VtableFlipMemory {
        inner: FakeMemory,
        hit: u64,
        handle_addr: u64,
        vtable_value: u64,
        foreign_value: u64,
        hit_reads: std::cell::Cell<u32>,
        handle_reads: std::cell::Cell<u32>,
    }

    impl Domain53Memory for VtableFlipMemory {
        fn module_base(&self) -> u64 {
            self.inner.module_base()
        }

        fn read_u64(&self, address: u64) -> Result<u64, ExtractError> {
            if address == self.hit {
                let reads = self.hit_reads.get();
                self.hit_reads.set(reads + 1);
                if reads == 0 {
                    return Ok(self.vtable_value);
                }
                return Ok(if self.handle_reads.get() >= 2 {
                    self.foreign_value
                } else {
                    self.vtable_value
                });
            }
            let bytes = self.inner.read_bytes(address, 8)?;
            Ok(u64::from_le_bytes(bytes.try_into().expect("u64 slice")))
        }

        fn read_bytes(&self, address: u64, len: usize) -> Result<Vec<u8>, ExtractError> {
            if address == self.handle_addr && len == 4 {
                self.handle_reads.set(self.handle_reads.get() + 1);
            }
            self.inner.read_bytes(address, len)
        }

        fn readable_ranges(&self, lo: u64, hi: u64) -> Vec<(u64, usize)> {
            self.inner.readable_ranges(lo, hi)
        }
    }

    #[test]
    fn operative_stable_reject_gate_refuses_memory_read_and_owner_errors() {
        // Negative regressions for the eligibility gate (Codex r2 F2 / r3 F1):
        // - an identity block that is UNREADABLE on both attempts yields two
        //   equal MemoryRead errors; equality proves nothing about contents,
        //   so the candidate must stay a refuse-by-name skip;
        // - a candidate with a well-formed handle but an owner outside the
        //   frame's accepted-country snapshot must also stay a skip: its
        //   deciding state lives in the oracle, not in its own bytes.
        let module = 0x1400_0000_0;
        let countries = vec![CountryRoot {
            tag_db_id: 1,
            country_ptr: 0,
        }];
        {
            let mut mem = FakeMemory::new(module);
            // Only the vtable slot is readable: the row validator's very
            // first field read fails with MemoryRead. MemoryRead errors are
            // NOT eligible for stable rejection (equality proves nothing
            // about contents), so the gate refuses before any re-attempt and
            // the candidate stays a refuse-by-name skip.
            let dead = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000 + 0x40_0000;
            mem.add_chunk(dead, 8);
            mem.write_u64(
                dead,
                module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            );
            let mut acc = RowAccumulator::default();
            read_operatives(&mem, module, &countries, &[dead], &mut acc)
                .expect("unreadable candidate walk succeeds");
            assert_eq!(acc.stable_rejects.operative, 0, "MemoryRead is not eligible");
            assert_eq!(acc.skip_counts.operative, 1);
            assert_eq!(acc.skipped_rows, 1);
        }
        {
            let mut mem = FakeMemory::new(module);
            let dead = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000 + 0x40_0000;
            mem.add_chunk(dead, 0x2000);
            mem.write_u64(
                dead,
                module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            );
            let identity = dead + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
            mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
            mem.write_u32(identity + 4, 11_387);
            mem.write_u32(identity + profile::DOMAIN53_OPERATIVE_OWNER as u64, 999);
            let mut acc = RowAccumulator::default();
            read_operatives(&mem, module, &countries, &[dead], &mut acc)
                .expect("foreign-owner candidate walk succeeds");
            assert_eq!(
                acc.stable_rejects.operative, 0,
                "owner_invalid decides from the oracle snapshot, not the candidate"
            );
            assert_eq!(acc.skip_counts.operative, 1);
            assert_eq!(acc.skipped_rows, 1);
        }
        {
            // assignment lives at offset 0x1078 - OUTSIDE the bracketed
            // identity head - so even an identical invalid value on both
            // attempts is mere repeatability, not witnessed stability: the
            // candidate must stay a refuse-by-name skip.
            let mut mem = FakeMemory::new(module);
            let dead = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000 + 0x40_0000;
            mem.add_chunk(dead, 0x2000);
            mem.write_u64(
                dead,
                module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            );
            let identity = dead + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
            mem.write_u32(identity, profile::DOMAIN53_OPERATIVE_HANDLE_TYPE);
            mem.write_u32(identity + 4, 11_387);
            let owner_addr = identity + profile::DOMAIN53_OPERATIVE_OWNER as u64;
            mem.write_u32(owner_addr, 1);
            let assignment_addr =
                identity + profile::DOMAIN53_OPERATIVE_ASSIGNMENT_ENUM as u64;
            mem.write_u32(assignment_addr, 6);
            let mut acc = RowAccumulator::default();
            read_operatives(&mem, module, &countries, &[dead], &mut acc)
                .expect("foreign-assignment candidate walk succeeds");
            assert_eq!(
                acc.stable_rejects.operative, 0,
                "assignment_invalid decides outside the bracketed head"
            );
            assert_eq!(acc.skip_counts.operative, 1);
            assert_eq!(acc.skipped_rows, 1);
        }
    }

    #[test]
    fn operative_stable_reject_bracket_refuses_changed_identity_head() {
        // kills: the bracket guards must classify on their own - both row
        // attempts fail with the IDENTICAL eligible error here, so only the
        // changed final identity-head sample stands between this candidate
        // and a wrongful stable reject.
        let mut inner = fake_domain53_memory(true, true);
        let dead = fake_heap_base() + 0x40_0000;
        inner.add_chunk(dead, 0x2000);
        inner.add_range(dead, 0x2000);
        let module = inner.module_base;
        inner.write_u64(
            dead,
            module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        let identity = dead + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64;
        let mem = MutatingTailHeadMemory {
            inner,
            window_addr: identity,
            window_len: OPERATIVE_STABLE_REJECT_HEAD_BYTES,
            handle_addr: identity,
            window_reads: std::cell::Cell::new(0),
            handle_reads: std::cell::Cell::new(0),
        };
        let extracted = extract_rows_from_current_roots(&mem)
            .expect("a bracket-violating candidate refuses row-local");
        assert_eq!(
            extracted.stable_rejects.operative, 0,
            "changed identity head must refuse the stable reject"
        );
        assert_eq!(extracted.skip_counts.operative, 1);
    }

    #[test]
    fn operative_stable_reject_bracket_refuses_changed_vtable() {
        // The vtable qword changes by the post-attempt-2 re-observation: the
        // object was freed or reused inside the bracket and the candidate
        // must stay a torn skip even though both attempts failed identically.
        let mut inner = fake_domain53_memory(true, true);
        let dead = fake_heap_base() + 0x40_0000;
        inner.add_chunk(dead, 0x2000);
        inner.add_range(dead, 0x2000);
        let module = inner.module_base;
        inner.write_u64(
            dead,
            module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
        );
        let mem = VtableFlipMemory {
            inner,
            hit: dead,
            handle_addr: dead + profile::DOMAIN53_OPERATIVE_IDENTITY_FROM_VTABLE as u64,
            vtable_value: module + profile::DOMAIN53_OPERATIVE_VTABLE_RVA as u64,
            foreign_value: module + profile::DOMAIN53_AGENCY_VTABLE_RVA as u64,
            hit_reads: std::cell::Cell::new(0),
            handle_reads: std::cell::Cell::new(0),
        };
        let extracted = extract_rows_from_current_roots(&mem)
            .expect("a bracket-violating candidate refuses row-local");
        assert_eq!(
            extracted.stable_rejects.operative, 0,
            "changed vtable must refuse the stable reject"
        );
        assert_eq!(extracted.skip_counts.operative, 1);
    }

    #[test]
    fn incoming_action_duplicate_id_keeps_refuse_by_name_skip() {
        // The duplicate-id suppression is load-bearing for the wire (the
        // decoder refuses duplicate ids), but a colliding id cannot be proven
        // to be the SAME delivery, so it must stay a refuse-by-name skip:
        // pw_gate_final's ia_sk population keeps its typed gap.
        let mut mem = fake_domain53_memory(true, true);
        let heap = fake_heap_base();
        let country3 = heap + 0x5_0000;
        let queue3 = heap + 0x41_0000;
        let data3 = heap + 0x41_1000;
        let action3 = heap + 0x41_2000;
        let body3 = heap + 0x41_3000;
        for (addr, len) in [
            (queue3, 0x40usize),
            (data3, 0x08),
            (action3, 0x20),
            (body3, 0x50),
        ] {
            mem.add_chunk(addr, len);
        }
        let module = mem.module_base;
        mem.write_u64(
            country3 + profile::DOMAIN53_INCOMING_ACTION_QUEUE as u64,
            queue3,
        );
        mem.write_u64(queue3 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_DATA as u64, data3);
        mem.write_u64(
            queue3 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_PACKED_SIZE_CAP as u64,
            (1u64 << 32) | 1,
        );
        mem.write_u32(queue3 + profile::DOMAIN53_INCOMING_ACTION_QUEUE_OWNER_GUARD as u64, 3);
        mem.write_u64(data3, action3);
        // SAME action handle (same action id) as the fixture's country-2
        // queue, but a body addressed to recipient 3 so the row itself would
        // validate for this recipient.
        mem.write_u64(
            action3,
            module + profile::DOMAIN53_INCOMING_ACTION_VTABLE_RVA as u64,
        );
        mem.write_u64(
            action3 + profile::DOMAIN53_INCOMING_ACTION_HANDLE as u64,
            (150_442u64 << 32) | u64::from(profile::DOMAIN53_INCOMING_ACTION_HANDLE_TYPE),
        );
        mem.write_u64(action3 + profile::DOMAIN53_INCOMING_ACTION_BODY as u64, body3);
        mem.write_i32(body3 + profile::DOMAIN53_INCOMING_ACTION_BODY_TYPE as u64, 0);
        mem.write_u32(body3 + profile::DOMAIN53_INCOMING_ACTION_ACTOR as u64, 1);
        mem.write_u32(body3 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_ACTOR as u64, 1);
        mem.write_u32(body3 + profile::DOMAIN53_INCOMING_ACTION_RECIPIENT as u64, 3);
        mem.write_u32(
            body3 + profile::DOMAIN53_INCOMING_ACTION_ORIGINAL_RECIPIENT as u64,
            3,
        );
        mem.write_u32(body3 + profile::DOMAIN53_INCOMING_ACTION_DATE as u64, 60_830_000);
        mem.write_u32(
            body3 + profile::DOMAIN53_INCOMING_ACTION_LAST_COMMAND_DATE as u64,
            60_830_000,
        );

        let extracted = extract_rows_from_current_roots(&mem)
            .expect("a duplicated delivery is not an error");
        assert_eq!(
            extracted.skip_counts.incoming_action, 1,
            "the second sighting stays a refuse-by-name skip"
        );
        assert_eq!(extracted.stable_rejects.operative, 0);
    }

    #[test]
    fn intel_network_quiet_action_uses_the_same_reciprocal_join() {
        let mut mem = fake_domain53_memory(true, true);
        let leader = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000 + 0xE_0000;
        let action = leader + profile::DOMAIN53_OPERATIVE_ACTION as u64;
        mem.write_u32(
            action + profile::DOMAIN53_OPERATIVE_ACTION_VARIANT as u64,
            profile::DOMAIN53_OPERATIVE_ACTION_QUIET_NETWORK,
        );
        let extracted = extract_rows_from_current_roots(&mem).expect("quiet-network fake read");
        assert_eq!(
            extracted.rows.intel_network_graphs[0].operative_ids,
            Some(vec![11_387])
        );
    }

    #[test]
    fn reader_rejects_subject_ratio_with_nonzero_high_dword() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let subject_object = heap + 0x1A_0000;
        mem.write_i64(
            subject_object + profile::DOMAIN53_SUBJECT_RATIO as u64,
            (1i64 << 32) | 40_000,
        );

        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");
        assert!(extracted.rows.subjects.is_empty());
        assert!(extracted.skipped_rows >= 1);
    }

    #[test]
    fn subject_reader_accepts_full_width_valid_ratio() {
        let mem = fake_domain53_memory(true, true);
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");
        assert_eq!(extracted.rows.subjects.len(), 1);
        assert_eq!(extracted.rows.subjects[0].autonomy_ratio_raw, 40_000);
    }

    #[test]
    fn intel_network_reader_rejects_unknown_owner_cid() {
        let mut mem = fake_domain53_memory(true, true);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let strategic_operative = heap + 0x15_0000;
        mem.write_u32(
            strategic_operative + profile::DOMAIN53_STRATEGIC_OPERATIVE_OWNER_TAG_DB_ID as u64,
            2_047,
        );

        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");
        assert!(extracted.rows.intel_network_graphs.is_empty());
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
            0
        );
        assert_eq!(
            extracted
                .rejection_counts
                .intel_network(IntelNetworkFailureCause::StrategicOwnerUnknown),
            1
        );
    }

    #[test]
    fn reader_witnesses_stable_empty_faction_roster() {
        let mut mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("resolve fake roots");
        let faction_system = mem
            .read_u64(roots.game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        mem.write_u64(faction_system + profile::DOMAIN53_FACTION_ARRAY as u64, 0);
        mem.write_u64(faction_system + profile::DOMAIN53_FACTION_COUNT as u64, 0);

        let extracted = extract_rows_from_current_roots(&mem).expect("stable empty roster");
        assert!(extracted.rows.factions.is_empty());
        assert!(extracted.rows.faction_roster_complete);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        let decoded = crate::compact::decode_snapshot_payload(0, &report.payload).unwrap();
        assert!(decoded.factions.is_empty());
        assert!(decoded.faction_roster_complete);
    }

    #[test]
    fn reader_suppresses_faction_witness_on_torn_header() {
        let mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("resolve fake roots");
        let faction_system = mem
            .read_u64(roots.game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        let torn = TornFactionHeaderMemory {
            inner: mem,
            faction_count_addr: faction_system + profile::DOMAIN53_FACTION_COUNT as u64,
            faction_count_reads: std::cell::Cell::new(0),
        };

        let extracted = extract_rows_from_current_roots(&torn).expect("torn roster is non-fatal");
        assert!(extracted.rows.factions.is_empty());
        assert!(!extracted.rows.faction_roster_complete);
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
    }

    #[test]
    fn reader_suppresses_faction_witness_on_torn_member_tag_value() {
        let mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("resolve fake roots");
        let faction_system = mem
            .read_u64(roots.game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        let faction_array = mem
            .read_u64(faction_system + profile::DOMAIN53_FACTION_ARRAY as u64)
            .unwrap();
        let faction = mem.read_u64(faction_array).unwrap();
        let members = mem
            .read_u64(faction + profile::DOMAIN53_FACTION_MEMBERS as u64)
            .unwrap();
        let first_member = mem.read_u64(members).unwrap();
        let torn = TornFactionValueMemory {
            inner: mem,
            address: first_member + profile::DOMAIN53_OBJ_TAG_DB_ID as u64,
            len: 4,
            replacement: 3u32.to_le_bytes().to_vec(),
            reads: std::cell::Cell::new(0),
        };

        let roster = read_faction_rows(
            &torn,
            roots.module_base,
            roots.game_state,
            &roots.country_acceptance_oracle,
        )
        .expect("torn member value is non-fatal");
        assert!(roster.is_none(), "a value tear must suppress the witness");
    }

    #[test]
    fn reader_suppresses_faction_witness_on_torn_name_bytes() {
        let mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("resolve fake roots");
        let faction_system = mem
            .read_u64(roots.game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        let faction_array = mem
            .read_u64(faction_system + profile::DOMAIN53_FACTION_ARRAY as u64)
            .unwrap();
        let faction = mem.read_u64(faction_array).unwrap();
        let name_addr = faction + profile::DOMAIN53_FACTION_NAME as u64;
        let name_len = mem.read_u64(name_addr + 0x10).unwrap() as usize;
        let torn = TornFactionValueMemory {
            inner: mem,
            address: name_addr,
            len: name_len + 1,
            replacement: b"Axiz\0".to_vec(),
            reads: std::cell::Cell::new(0),
        };

        let roster = read_faction_rows(
            &torn,
            roots.module_base,
            roots.game_state,
            &roots.country_acceptance_oracle,
        )
        .expect("torn faction name is non-fatal");
        assert!(roster.is_none(), "a name tear must suppress the witness");
    }

    #[test]
    fn reader_suppresses_faction_witness_on_changed_outer_game_state_root() {
        let mem = fake_domain53_memory(true, true);
        let game_state_root_addr = mem.module_base + profile::CG_STATE_DATE_OFFSET as u64;
        let original_game_state = mem.read_u64(game_state_root_addr).unwrap();
        let torn = TornFactionValueMemory {
            inner: mem,
            address: game_state_root_addr,
            len: 8,
            replacement: (original_game_state + 0x8000).to_le_bytes().to_vec(),
            reads: std::cell::Cell::new(0),
        };

        let extracted = extract_rows_from_current_roots_bypassing_war_discovery(&torn)
            .expect("outer-root tear is non-fatal to d53");
        assert!(extracted.rows.factions.is_empty());
        assert!(
            !extracted.rows.faction_roster_complete,
            "a witness must bind to the still-current outer game-state root"
        );
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
    }

    #[test]
    fn reader_suppresses_faction_witness_on_changed_country_acceptance_oracle() {
        let mut mem = fake_domain53_memory(true, true);
        let game_state = mem
            .read_u64(mem.module_base + profile::CG_STATE_DATE_OFFSET as u64)
            .unwrap();
        let country_table_addr = game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64;
        let country_table = mem.read_u64(country_table_addr).unwrap();
        let country_count = mem
            .read_u32(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64)
            .unwrap();
        let table_len = country_count as usize * 8;
        let alternate_country_table = country_table + 0x4000;
        let table_bytes = mem.read_bytes(country_table, table_len).unwrap();
        mem.add_chunk(alternate_country_table, table_len);
        mem.write_bytes(alternate_country_table, &table_bytes);
        let torn = TornFactionValueMemory {
            inner: mem,
            address: country_table_addr,
            len: 8,
            replacement: alternate_country_table.to_le_bytes().to_vec(),
            reads: std::cell::Cell::new(0),
        };

        let extracted = extract_rows_from_current_roots_bypassing_war_discovery(&torn)
            .expect("country-oracle tear is non-fatal to d53");
        assert!(extracted.rows.factions.is_empty());
        assert!(
            !extracted.rows.faction_roster_complete,
            "a witness must bind to the double-read-stable country acceptance oracle"
        );
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
    }

    #[test]
    fn reader_suppresses_faction_witness_when_inner_allocation_changes_after_outer_reread() {
        let mem = fake_domain53_memory(true, true);
        let outer_root_addr = mem.module_base + profile::CG_STATE_DATE_OFFSET as u64;
        let game_state = mem.read_u64(outer_root_addr).unwrap();
        let country_table = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64)
            .unwrap();
        let country_count = mem
            .read_u32(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64)
            .unwrap();
        let allocation_len = country_count as usize * 8;
        assert_ne!(mem.read_u64(country_table + 8).unwrap(), 0);
        let torn = CountryTableAllocationChangesAfterOuterValidationMemory {
            inner: mem,
            outer_root_addr,
            country_table,
            allocation_len,
            changed_slot_offset: 8,
            replacement: 0u64.to_le_bytes().to_vec(),
            outer_root_reads: std::cell::Cell::new(0),
            allocation_changed: std::cell::Cell::new(false),
        };

        let extracted = extract_rows_from_current_roots_bypassing_war_discovery(&torn).unwrap();
        assert!(torn.allocation_changed.get());
        assert!(extracted.rows.factions.is_empty());
        assert!(!extracted.rows.faction_roster_complete);
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
    }

    #[test]
    fn reader_preserves_countries_but_suppresses_witness_when_bulk_table_capture_fails() {
        let mem = fake_domain53_memory(true, true);
        let game_state = mem
            .read_u64(mem.module_base + profile::CG_STATE_DATE_OFFSET as u64)
            .unwrap();
        let country_table = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64)
            .unwrap();
        let country_count = mem
            .read_u32(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64)
            .unwrap();
        let torn = CountryTableBulkReadFailsMemory {
            inner: mem,
            country_table,
            allocation_len: country_count as usize * 8,
        };

        let oracle =
            resolve_country_acceptance_oracle(&torn, torn.module_base(), game_state).unwrap();
        assert!(!oracle.allocation_capture_complete);
        assert_eq!(oracle.ambiguous_or_rejected_count, 1);
        assert_eq!(oracle.slot_allocation.len(), country_count as usize);
        assert_eq!(oracle.accepted_countries.len(), FAKE_COUNTRY_ROWS as usize);
        assert!(!oracle.supports_positive_faction_witness());

        let extracted = extract_rows_from_current_roots_bypassing_war_discovery(&torn).unwrap();
        assert!(extracted.rows.factions.is_empty());
        assert!(!extracted.rows.faction_roster_complete);
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_FACTION_SYSTEM_870EBF,
            0
        );
    }

    #[test]
    fn phase_timers_are_measured_and_internally_consistent() {
        let mem = fake_domain53_memory(true, true);

        let outer_started = Instant::now();
        let extracted = extract_rows_from_current_roots(&mem).expect("domain53 fake read");
        let outer_total_us = duration_micros_u64_saturating(outer_started.elapsed());

        assert!(
            !extracted.rows.opinions.is_empty()
                || !extracted.rows.relation_flags.is_empty()
                || !extracted.rows.subjects.is_empty(),
            "fixture must carry relations rows for this test to exercise the relations span"
        );

        let phase = extracted.phase_times;
        let spans = [
            phase.wars_us,
            phase.faction_gate_us,
            phase.relations_us,
            phase.faction_roster_us,
            phase.incoming_actions_us,
            phase.agencies_us,
            phase.operatives_us,
            phase.operations_us,
            phase.intel_networks_us,
            phase.cryptology_us,
            phase.roots_us,
            phase.war_projection_us,
            phase.aggregation_us,
        ];

        for (idx, &span) in spans.iter().enumerate() {
            assert!(
                span <= outer_total_us,
                "phase[{idx}] = {span}us exceeds outer total {outer_total_us}us"
            );
        }

        let sum_us = spans
            .iter()
            .copied()
            .fold(0u64, |acc, span| acc.saturating_add(span));
        assert!(
            sum_us <= outer_total_us,
            "sum of phase spans {sum_us}us exceeds outer total {outer_total_us}us"
        );
    }

    fn fake_faction_system(
        members: &[(u32, bool)],
        member_count_override: Option<u32>,
    ) -> (FakeMemory, u64, Vec<CountryRoot>, CountryAcceptanceOracle) {
        let module = 0x1400_0000_0;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let game_state = heap + 0x100;
        let faction_system = heap + 0x1000;
        let faction_array = heap + 0x2000;
        let faction1 = heap + 0x3000;
        let faction_members = heap + 0x4000;
        let country_base = heap + 0x5000;
        let country_table = heap + 0x8000;

        let mut mem = FakeMemory::new(module);
        mem.add_chunk(module + profile::CG_STATE_DATE_OFFSET as u64, 8);
        mem.add_chunk(game_state, 0x500);
        mem.add_chunk(faction_system, 0x40);
        mem.add_chunk(faction_array, 0x20);
        mem.add_chunk(faction1, 0x80);
        mem.add_chunk(faction_members, 0x100);
        mem.add_chunk(country_table, members.len() * 8);

        for (idx, (tag, is_country)) in members.iter().enumerate() {
            let cptr = country_base + (idx as u64) * 0x200;
            mem.add_chunk(cptr, 0x100);
            if *is_country {
                mem.write_u64(cptr, module + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64);
                mem.write_u32(cptr + profile::DOMAIN53_OBJ_TAG_DB_ID as u64, *tag);
            } else {
                mem.write_u64(cptr, module + 0x00DE_AD00);
            }
            mem.write_u64(faction_members + (idx as u64) * 8, cptr);
            mem.write_u64(country_table + (idx as u64) * 8, cptr);
        }

        mem.write_u64(module + profile::CG_STATE_DATE_OFFSET as u64, game_state);
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64,
            country_table,
        );
        mem.write_u32(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64 + 8,
            members.len() as u32,
        );
        mem.write_u32(
            game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_COUNT as u64,
            members.len() as u32,
        );
        mem.write_u64(
            game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64,
            faction_system,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_ARRAY as u64,
            faction_array,
        );
        mem.write_u64(
            faction_system + profile::DOMAIN53_FACTION_COUNT as u64,
            (1u64 << 32) | 1u64,
        );
        mem.write_u64(faction_array, faction1);
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_ID as u64,
            (1u64 << 32) | u64::from(profile::DOMAIN53_FACTION_TYPE_EXPECTED),
        );
        mem.write_inline_string(faction1 + profile::DOMAIN53_FACTION_NAME as u64, "Axis");
        mem.write_u64(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS as u64,
            faction_members,
        );
        mem.write_u32(
            faction1 + profile::DOMAIN53_FACTION_MEMBERS_SIZE as u64,
            member_count_override.unwrap_or(members.len() as u32),
        );
        let country_acceptance_oracle =
            resolve_country_acceptance_oracle(&mem, module, game_state).unwrap();
        let countries = country_acceptance_oracle.accepted_countries.clone();
        (mem, game_state, countries, country_acceptance_oracle)
    }

    #[test]
    fn faction_reader_suppresses_duplicate_tag_country_table() {
        let (mut mem, game_state, _, _) = fake_faction_system(&[(1, true), (2, true)], None);
        let country_table = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64)
            .unwrap();
        let first_country = mem.read_u64(country_table).unwrap();
        let colliding_country = mem.read_u64(country_table + 8).unwrap();
        assert_ne!(first_country, colliding_country);
        mem.write_u32(
            colliding_country + profile::DOMAIN53_OBJ_TAG_DB_ID as u64,
            1,
        );

        let faction_system = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        let faction_array = mem
            .read_u64(faction_system + profile::DOMAIN53_FACTION_ARRAY as u64)
            .unwrap();
        let faction = mem.read_u64(faction_array).unwrap();
        let members = mem
            .read_u64(faction + profile::DOMAIN53_FACTION_MEMBERS as u64)
            .unwrap();
        mem.write_u64(members, colliding_country);
        mem.write_u32(faction + profile::DOMAIN53_FACTION_MEMBERS_SIZE as u64, 1);

        let oracle = resolve_country_acceptance_oracle(&mem, mem.module_base, game_state).unwrap();
        assert_eq!(oracle.duplicate_tag_slot_count, 2);
        assert_eq!(oracle.ambiguous_or_rejected_count, 2);
        assert!(oracle.accepted_countries.is_empty());
        assert!(!oracle.supports_positive_faction_witness());
        assert!(
            read_faction_rows(&mem, mem.module_base, game_state, &oracle)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn faction_reader_rejects_member_pointer_outside_accepted_table() {
        let (mut mem, game_state, _, oracle) = fake_faction_system(&[(1, true)], None);
        assert!(oracle.supports_positive_faction_witness());
        let unaccepted_country = profile::DOMAIN53_HEAP_LO as u64 + 0x20_0000;
        mem.add_chunk(unaccepted_country, 0x100);
        mem.write_u64(
            unaccepted_country,
            mem.module_base + profile::DOMAIN53_COUNTRY_VTABLE_RVA as u64,
        );
        mem.write_u32(
            unaccepted_country + profile::DOMAIN53_OBJ_TAG_DB_ID as u64,
            1,
        );
        let faction_system = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_FACTION_SYSTEM as u64)
            .unwrap();
        let faction_array = mem
            .read_u64(faction_system + profile::DOMAIN53_FACTION_ARRAY as u64)
            .unwrap();
        let faction = mem.read_u64(faction_array).unwrap();
        let members = mem
            .read_u64(faction + profile::DOMAIN53_FACTION_MEMBERS as u64)
            .unwrap();
        mem.write_u64(members, unaccepted_country);

        assert!(
            read_faction_rows(&mem, mem.module_base, game_state, &oracle)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn faction_reader_fails_closed_when_a_member_is_not_a_country() {
        let (mem, game_state, _countries, oracle) =
            fake_faction_system(&[(1, true), (2, false)], None);
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &oracle).expect("faction read");
        assert!(rows.is_none());
    }

    #[test]
    fn faction_reader_fails_closed_on_oversized_member_count() {
        let (mem, game_state, _countries, oracle) =
            fake_faction_system(&[(1, true), (2, true)], Some(5));
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &oracle).expect("faction read");
        assert!(rows.is_none());
    }

    #[test]
    fn faction_reader_emits_clean_roster() {
        let (mem, game_state, _countries, oracle) =
            fake_faction_system(&[(1, true), (2, true), (3, true)], None);
        let rows =
            read_faction_rows(&mem, mem.module_base, game_state, &oracle).expect("faction read");
        let rows = rows.expect("stable complete faction roster");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].faction_id, 1);
        assert_eq!(rows[0].leader_tag_db_id, 1);
        assert_eq!(rows[0].member_tag_db_ids, vec![1, 2, 3]);
    }

    #[test]
    fn discovery_extract_matches_uncached_fake_memory() {
        let mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        let mut cached = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("cached domain53 extract");
        let mut uncached =
            extract_rows_from_current_roots(&mem).expect("uncached domain53 extract");

        cached.phase_times = Domain53PhaseTimes::default();
        uncached.phase_times = Domain53PhaseTimes::default();

        assert_eq!(cached, uncached);
    }

    #[test]
    fn owner_route_war_addrs_reread_casualties_fresh() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");

        let validated =
            domain53_war_row_addrs_from_relation_roots(&mem).expect("owner-route war addrs");
        assert_eq!(validated.len(), 1);

        let via_addrs =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("validated");
        assert_eq!(via_addrs.rows.wars.len(), 1);
        assert_eq!(via_addrs.rows.wars[0].first_casualties, 100);
        assert_eq!(via_addrs.rows.wars[0].second_casualties, 200);
        assert_eq!(
            via_addrs.skipped_rows, 0,
            "unexpected D53 family skips: {:?}",
            via_addrs.skip_counts
        );

        let war_row = validated[0];
        mem.write_i32(
            war_row + profile::DOMAIN53_WAR_FIRST_CASUALTIES as u64,
            4242,
        );
        mem.write_i32(
            war_row + profile::DOMAIN53_WAR_SECOND_CASUALTIES as u64,
            9999,
        );
        let refreshed =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("refreshed");
        assert_eq!(refreshed.rows.wars.len(), 1);
        assert_eq!(refreshed.rows.wars[0].first_casualties, 4242);
        assert_eq!(refreshed.rows.wars[0].second_casualties, 9999);

        mem.write_u64(war_row + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        let ended =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &validated)
                .expect("war-ended extract");
        assert!(ended.rows.wars.is_empty());
    }

    #[test]
    fn cached_operative_stale_hit_drops_only_that_row() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 1);
        mem.write_u64(discovery.operative_hits[0], 0);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("stale operative hit below degradation threshold");

        assert!(extracted.rows.operatives.is_empty());
        assert_eq!(extracted.rows.wars.len(), 1);
        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.stale_dropped_total, 1);
        assert_ne!(
            extracted.source_proofs & SOURCE_PROOF_OPERATIVE_ROOT_870EBF,
            0
        );
    }

    #[test]
    fn cached_operative_stale_majority_fails_closed() {
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..7 {
            add_fake_operative(&mut mem, index);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 8);
        for hit in discovery.operative_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }

        let err = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect_err("stale operative majority must fail closed");

        assert_eq!(err, ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
    }

    fn add_fake_operation_instance(
        mem: &mut FakeMemory,
        index: u32,
        instance_id: u32,
        def_ok: bool,
        country_ptr: u64,
    ) -> u64 {
        let heap = fake_heap_base();
        let instance = heap + 0x50_0000 + u64::from(index) * 0x2000;
        let def = instance + 0x1000;
        mem.add_chunk(instance, 0x180);
        mem.add_chunk(def, 0x40);
        mem.add_range(instance, 0x180);
        mem.write_u64(
            instance,
            mem.module_base + profile::DOMAIN53_OPERATION_INSTANCE_VTABLE_RVA as u64,
        );
        let handle = (u64::from(instance_id) << 32)
            | u64::from(profile::DOMAIN53_OPERATION_INSTANCE_HANDLE_TYPE);
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_HANDLE as u64,
            handle,
        );
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_DEF as u64,
            def,
        );
        let def_vtable = if def_ok {
            mem.module_base + profile::DOMAIN53_OPERATION_DEF_VTABLE_RVA as u64
        } else {
            mem.module_base + 0x1234
        };
        mem.write_u64(def, def_vtable);
        if def_ok {
            mem.write_inline_string(
                def + profile::DOMAIN53_OPERATION_DEF_TOKEN as u64,
                "operation_test",
            );
        }
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_OWNER_COUNTRY as u64,
            country_ptr,
        );
        mem.write_u32(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_TARGET_COUNTRY_ID as u64,
            3,
        );
        mem.write_u64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_TARGET_OBJECT as u64,
            0,
        );
        mem.write_i64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_PREPARATION_CURRENT as u64,
            400_000,
        );
        mem.write_i64(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_PREPARATION_REQUIRED as u64,
            1_500_000,
        );
        mem.write_u32(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_DURATION_DAYS as u64,
            0,
        );
        mem.write_u32(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_OPERATIVE_COUNT as u64,
            0,
        );
        mem.write_u32(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_PHASE_COUNT as u64,
            3,
        );
        mem.write_u32(
            instance + profile::DOMAIN53_OPERATION_INSTANCE_REQUIREMENT_COUNT as u64,
            0,
        );
        instance
    }

    #[test]
    fn read_operations_validates_instances_and_resolves_country() {
        let mut mem = fake_domain53_memory(true, true);
        let module = mem.module_base;
        let country3 = fake_heap_base() + 0x5_0000;
        let inst_a = add_fake_operation_instance(&mut mem, 0, 129_014, true, country3);
        let inst_b = add_fake_operation_instance(&mut mem, 1, 128_904, false, 0);
        let inst_c = add_fake_operation_instance(&mut mem, 2, 7, true, country3);
        mem.write_u64(
            inst_c + profile::DOMAIN53_OPERATION_INSTANCE_HANDLE as u64,
            (7u64 << 32) | 9999,
        );

        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert!(discovery.operation_instance_hits.contains(&inst_a));
        assert!(discovery.operation_instance_hits.contains(&inst_b));
        assert!(discovery.operation_instance_hits.contains(&inst_c));

        let countries =
            resolve_country_roots(&mem, module, fake_heap_base()).expect("country roots");
        let (rows, drops) = read_operations(
            &mem,
            module,
            60_830_441,
            &countries,
            &BTreeSet::new(),
            &discovery.operation_instance_hits,
        )
        .expect("read_operations");

        assert_eq!(drops.stale_liveness, 0);
        assert_eq!(drops.filtered, 2);
        assert_eq!(drops.semantic_incomplete, 1);
        assert_eq!(
            drops
                .semantic_causes
                .operation(OperationSemanticCause::DefinitionObjectInvalid),
            1
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].instance_id, 129_014);
        assert_eq!(rows[0].definition_id.as_deref(), Some("operation_test"));
        assert_eq!(rows[0].owner_country_tag_db_id, Some(3));
        assert_eq!(rows[0].target_country_tag_db_id, Some(3));
        assert_eq!(rows[0].status, Some(OperationInstanceStatus::Preparing));
        assert_eq!(rows[0].phase_index, 0);
        assert_eq!(rows[0].phase_count, 3);
        assert_eq!(rows[0].progress_fixed, Some(26_666));
        assert_eq!(rows[0].start_hour, Some(0));
        assert_eq!(rows[0].duration_hours, Some(0));
    }

    #[test]
    fn semantic_operation_failure_withholds_the_complete_opin_family_only() {
        let mut mem = fake_domain53_memory(true, true);
        let country3 = fake_heap_base() + 0x5_0000;
        add_fake_operation_instance(&mut mem, 9, 200_001, false, country3);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("OPIN semantic failure must not remove unrelated d53 families");

        assert!(extracted.rows.operation_instances.is_empty());
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_OPERATION_DETAIL_870EBF,
            0
        );
        assert_eq!(
            extracted
                .rejection_counts
                .operation(OperationSemanticCause::DefinitionObjectInvalid),
            1
        );
        assert!(OperationSemanticCause::ALL
            .into_iter()
            .filter(|cause| *cause != OperationSemanticCause::DefinitionObjectInvalid)
            .all(|cause| extracted.rejection_counts.operation(cause) == 0));
        assert!(!extracted.rows.operatives.is_empty());
    }

    #[test]
    fn operation_progress_uses_exact_current_phase_law() {
        assert_eq!(operation_current_phase_progress(15, 50, 5), Some(50_000));
        assert_eq!(operation_current_phase_progress(19, 50, 5), Some(90_000));
        assert_eq!(operation_current_phase_progress(0, 50, 5), Some(0));
    }

    #[test]
    fn operations_never_nuke_the_frame_operatives_are_the_systemic_gate() {
        let ops_majority = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 5,
            operation_filtered: 0,
        };
        assert!(
            !ops_majority.frame_degraded(),
            "an operation stale majority must NOT nuke the whole frame"
        );
        assert!(
            ops_majority.operation_degraded(),
            "operation health must still be visible via the diagnostic gate"
        );

        let ops_all_filtered = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 8,
        };
        assert!(!ops_all_filtered.frame_degraded());
        assert!(
            !ops_all_filtered.operation_degraded(),
            "candidate filtering is not staleness"
        );
        assert_eq!(
            ops_all_filtered.stale_dropped_total(),
            0,
            "filtering must not count toward the visible stale total"
        );

        let operatives_majority = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 5,
            war_total: 0,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(operatives_majority.frame_degraded());
    }

    #[test]
    fn war_staleness_never_contributes_to_the_whole_frame_gate() {
        let war_only = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 0,
            war_total: 8,
            war_stale_dropped: 8,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(
            !war_only.frame_degraded(),
            "a war stale majority must never doom the whole frame"
        );
        assert!(
            war_only.war_degraded(),
            "a war stale majority must trip the war-only downgrade gate"
        );

        let op_only = Domain53DiscoveryStaleness {
            operative_total: 8,
            operative_stale_dropped: 5,
            war_total: 8,
            war_stale_dropped: 0,
            operation_total: 8,
            operation_stale_dropped: 0,
            operation_filtered: 0,
        };
        assert!(op_only.frame_degraded());
        assert!(!op_only.war_degraded());
    }

    #[test]
    fn cached_operation_stale_majority_still_emits_the_rest_of_d53() {
        let mut mem = fake_domain53_memory(true, true);
        let country3 = fake_heap_base() + 0x5_0000;
        for index in 0..8 {
            add_fake_operation_instance(&mut mem, index, 100 + index, true, country3);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operation_instance_hits.len(), 8);
        assert_eq!(discovery.operative_hits.len(), 1);
        for hit in discovery.operation_instance_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("operation stale majority must NOT fail the whole frame");
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.rows.operation_instances.len(), 3);
        assert_eq!(extracted.family_counts.operation_candidates, 8);
        assert_eq!(extracted.family_counts.operation_emitted, 3);
        assert_eq!(extracted.family_counts.operation_stale_liveness, 5);
        assert_eq!(extracted.family_counts.operation_filtered, 0);
    }

    #[test]
    fn cached_operation_stale_hit_below_majority_does_not_degrade_operatives() {
        let mut mem = fake_domain53_memory(true, true);
        let country3 = fake_heap_base() + 0x5_0000;
        for index in 0..8 {
            add_fake_operation_instance(&mut mem, index, 100 + index, true, country3);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        mem.write_u64(discovery.operation_instance_hits[0], 0);

        let extracted = extract_rows_from_current_roots_with_discovery(&mem, &discovery)
            .expect("single stale operation hit is below degradation threshold");
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.rows.operation_instances.len(), 7);
        assert_eq!(extracted.stale_dropped_total, 1);
    }

    #[test]
    fn owner_route_refuses_a_malformed_war_root_instead_of_shrinking_the_set() {
        let mut mem = fake_domain53_memory(true, true);
        let addrs = domain53_war_row_addrs_from_relation_roots(&mem).expect("baseline resolves");
        assert_eq!(addrs.len(), 1);
        let war_row = addrs[0];
        mem.write_u32(war_row + profile::DOMAIN53_WAR_ECHO_FIRST as u64, 9999);
        assert!(
            domain53_war_row_addrs_from_relation_roots(&mem).is_err(),
            "an owned war row that will not read must refuse the whole set"
        );

        let mut mem = fake_domain53_memory(true, true);
        mem.write_u64(war_row + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        assert_eq!(
            domain53_war_row_addrs_from_relation_roots(&mem),
            Err(ExtractError::InvalidRoot(
                "domain53_war_relation_row_date_vtable_mismatch"
            ))
        );

        let mut mem = fake_domain53_memory(true, true);
        let twin = add_fake_war(&mut mem, 0);
        mem.write_u32(twin + profile::DOMAIN53_WAR_SECOND_TAG as u64, 3);
        mem.write_u32(twin + profile::DOMAIN53_WAR_ECHO_SECOND as u64, 3);
        assert_eq!(
            domain53_war_row_addrs_from_relation_roots(&mem),
            Err(ExtractError::InvalidRoot(
                "domain53_war_relation_duplicate_war_pair"
            ))
        );
    }

    #[test]
    fn owner_route_refuses_an_empty_relation_world_instead_of_reporting_peace() {
        let mut mem = fake_domain53_memory(true, true);
        let game_state = mem
            .read_u64(mem.module_base + profile::CG_STATE_DATE_OFFSET as u64)
            .unwrap();
        let country_table = mem
            .read_u64(game_state + profile::DOMAIN53_CGAMESTATE_COUNTRY_TABLE as u64)
            .unwrap();
        for tag in 1..=FAKE_COUNTRY_ROWS {
            let country = mem.read_u64(country_table + u64::from(tag) * 8).unwrap();
            if country != 0 {
                mem.write_u64(country + profile::DOMAIN53_RELATION_MANAGER as u64, 0);
            }
        }
        assert_eq!(
            domain53_war_row_addrs_from_relation_roots(&mem),
            Err(ExtractError::InvalidRoot(
                "domain53_war_relation_roots_empty_implausible"
            ))
        );
    }

    #[test]
    fn cached_war_stale_hit_drops_only_that_row() {
        let mut mem = fake_domain53_memory(true, true);
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        let cached = test_war_row_addrs(&mem).expect("owner-route war addrs");
        assert_eq!(cached.len(), 1);
        mem.write_u64(cached[0] + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &cached)
                .expect("stale war hit below degradation threshold");

        assert!(extracted.rows.wars.is_empty());
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.skipped_rows, 0);
        assert_eq!(extracted.stale_dropped_total, 1);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
    }

    #[test]
    fn cached_war_stale_majority_downgrades_war_family_but_still_emits() {
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..8 {
            add_fake_war(&mut mem, index);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        let cached = test_war_row_addrs(&mem).expect("owner-route war addrs");
        assert_eq!(cached.len(), 9);
        let mut corrupted = 0usize;
        for addr in cached.iter().take(5) {
            mem.write_u64(addr + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
            corrupted += 1;
        }
        assert_eq!(corrupted, 5);

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &cached)
                .expect("war stale majority downgrades the war family, does not fail the frame");

        assert!(extracted.rows.wars.is_empty(), "war rows must be dropped");
        assert_eq!(
            extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF,
            0,
            "war-root proof must be withheld on a stale war majority"
        );
        assert!(
            extracted.rows.countries.is_empty(),
            "country diplomacy rows must be omitted when war is downgraded"
        );
        assert!(
            extracted
                .rows
                .state_events
                .iter()
                .any(|e| e.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP),
            "faction-group state events must still emit when war is downgraded"
        );
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.skipped_rows, 0, "{:?}", extracted.skip_counts);
        assert_eq!(extracted.stale_dropped_total, 5);

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0,
            "downgraded war family must badge STATUS_WAR_ROOT_UNVALIDATED"
        );
    }

    #[test]
    fn validated_war_addrs_stale_majority_downgrades_war_only_operatives_survive() {
        let mut mem = fake_domain53_memory(true, true);
        let mut addrs = Vec::new();
        for index in 0..8 {
            let war = add_fake_war(&mut mem, index);
            addrs.push(war);
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        for war in addrs.iter().take(5) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("validated-addr war stale majority must downgrade war, not fail the frame");

        assert!(extracted.rows.wars.is_empty());
        assert_eq!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert!(extracted.rows.countries.is_empty());
        assert!(extracted
            .rows
            .state_events
            .iter()
            .any(|e| e.source_kind == STATE_EVENT_SOURCE_COUNTRY_FACTION_GROUP));
        assert_eq!(extracted.rows.operatives.len(), 1);
        assert_eq!(extracted.stale_dropped_total, 5);

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn validated_war_addrs_below_majority_keeps_war_family_validated() {
        let mut mem = fake_domain53_memory(true, true);
        let mut addrs = Vec::new();
        for index in 0..8 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        for war in addrs.iter().take(3) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("minority-stale war set stays validated");

        assert_eq!(extracted.rows.wars.len(), 5);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(extracted.stale_dropped_total, 3);
        assert!(extracted
            .rows
            .countries
            .iter()
            .any(|country| country.at_war_count != 0));

        let report = build_domain53_payload(extracted.rows, extracted.source_proofs);
        assert!(report.ok(), "{:?}", report.errors);
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_WAR_ROOT_UNVALIDATED,
            0
        );
    }

    #[test]
    fn war_boundary_exactly_at_majority_threshold_downgrades() {
        let build = |stale_count: usize| {
            let mut mem = fake_domain53_memory(true, true);
            let mut addrs = Vec::new();
            for index in 0..8 {
                addrs.push(add_fake_war(&mut mem, index));
            }
            let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
            for war in addrs.iter().take(stale_count) {
                mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
            }
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("frame always extracts (war only downgrades)")
        };

        let at_four = build(4);
        assert_ne!(at_four.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(at_four.rows.wars.len(), 4);

        let at_five = build(5);
        assert_eq!(at_five.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert!(at_five.rows.wars.is_empty());
    }

    #[test]
    fn war_below_absolute_floor_never_downgrades_even_if_fraction_high() {
        let mut mem = fake_domain53_memory(true, true);
        let mut addrs = Vec::new();
        for index in 0..4 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        for war in addrs.iter().take(3) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let extracted =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect("below the absolute floor the war family is never downgraded");

        assert_eq!(extracted.rows.wars.len(), 1);
        assert_ne!(extracted.source_proofs & SOURCE_PROOF_WAR_ROOT_870EBF, 0);
        assert_eq!(extracted.stale_dropped_total, 3);
    }

    #[test]
    fn operative_stale_majority_still_fails_whole_frame_when_war_downgrades() {
        let mut mem = fake_domain53_memory(true, true);
        for index in 0..7 {
            add_fake_operative(&mut mem, index);
        }
        let mut addrs = Vec::new();
        for index in 0..8 {
            addrs.push(add_fake_war(&mut mem, index));
        }
        let discovery = discover_domain53_heap_hits(&mem).expect("domain53 discovery");
        assert_eq!(discovery.operative_hits.len(), 8);
        for hit in discovery.operative_hits.iter().take(5) {
            mem.write_u64(*hit, 0);
        }
        for war in addrs.iter().take(5) {
            mem.write_u64(war + profile::DOMAIN53_WAR_DATE_VTABLE as u64, 0);
        }

        let err =
            extract_rows_from_current_roots_with_validated_war_addrs(&mem, &discovery, &addrs)
                .expect_err("operative stale majority must still fail the whole frame closed");
        assert_eq!(err, ExtractError::InvalidRoot(DOMAIN53_DISCOVERY_DEGRADED));
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
        assert_eq!(report.gate_summary.data_rows_total, 17);
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
            Some(0)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("compact_state_event_rows")
                .copied(),
            Some(7)
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
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
            0
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("incoming_action_queue_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_network_graph_rows")
                .copied(),
            Some(2)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("intel_network_graph_state_rows")
                .copied(),
            Some(17)
        );
        assert_eq!(
            report
                .gate_summary
                .data_units_total
                .get("incoming_action_queue_recipient_rows")
                .copied(),
            Some(2)
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
    fn payload_phase_times_populated_and_bounded_by_outer_build() {
        let mut rows = proof_fixture_rows();
        rows.opinions = (1..=250u32)
            .flat_map(|src| {
                (1..=201u32)
                    .filter(move |&target| target != src)
                    .take(200)
                    .map(move |target| OpinionRow {
                        src_tag_db_id: src,
                        target_tag_db_id: target,
                        opinion: -35,
                    })
            })
            .collect();
        let outer_started = Instant::now();
        let report = build_domain53_payload_sampled(rows, CURRENT_VALIDATED_SOURCE_PROOFS, 0, true);
        let outer_us = duration_micros_u64_saturating(outer_started.elapsed());

        assert!(report.ok(), "{:?}", report.errors);

        let pt = report.payload_phase_times;
        assert!(!report.payload.is_empty(), "fixture must produce a payload");
        assert!(pt.encode_us > 0, "encode phase must be timed");
        assert!(pt.summary_us > 0, "encode-side summary phase must be timed");
        assert!(
            pt.decode_gate_us > 0,
            "sampled decode-gate phase must be timed"
        );

        let phase_sum =
            pt.snapshot_us + pt.validation_us + pt.encode_us + pt.summary_us + pt.decode_gate_us;
        assert!(
            phase_sum <= outer_us,
            "sub-phase sum {phase_sum} exceeds outer build wall {outer_us}"
        );
    }

    #[test]
    fn off_sample_frame_skips_decode_gate() {
        let off = build_domain53_payload_sampled(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS,
            0,
            false,
        );
        let on = build_domain53_payload_sampled(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS,
            0,
            true,
        );
        assert_eq!(off.payload_phase_times.decode_gate_us, 0);
        assert_eq!(off, on);
        assert_eq!(off.gate_summary, on.gate_summary);
        assert_eq!(off.errors, on.errors);
    }

    #[test]
    fn collector_emit_decision_matches_decode_derived_report() {
        for rows in [proof_fixture_rows(), {
            let mut r = proof_fixture_rows();
            r.opinions[0].opinion = MAX_OPINION_ABS + 1000;
            r
        }] {
            let report = build_domain53_payload(rows.clone(), CURRENT_VALIDATED_SOURCE_PROOFS);
            let decode_summary = decode_payload(0, &report.payload);

            assert_eq!(
                report.gate_summary.status_flags_or, decode_summary.status_flags_or,
                "status_flags_or (emit gate) must match decode"
            );
            assert_eq!(
                report.gate_summary.data_rows_total, decode_summary.data_rows_total,
                "emitted_rows source must match decode"
            );
            assert_eq!(
                report.gate_summary.data_changed_rows_total, decode_summary.data_changed_rows_total,
                "changed_rows source must match decode"
            );
            assert_eq!(
                report.gate_summary.parse_errors, decode_summary.parse_errors,
                "parse_errors (which drive report.ok()) must match decode"
            );
            assert_eq!(
                report.ok(),
                decode_summary.parse_errors.is_empty(),
                "report.ok() must equal the decode-derived ok decision"
            );
        }
    }

    #[test]
    fn sampled_decode_mismatch_is_fail_closed() {
        let a = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS)
            .gate_summary;
        assert!(
            decode_verify_mismatch_error(&a, &a).is_none(),
            "identical summaries must NOT fail closed"
        );

        let mut b = a.clone();
        b.data_rows_total += 1;
        let err = decode_verify_mismatch_error(&a, &b)
            .expect("differing summaries must produce a fail-closed error");
        assert!(err.contains("disagrees with sampled decode"));
        assert!(err.contains("fail-closed"));

        let mut report =
            build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert!(
            report.ok(),
            "baseline fixture is clean: {:?}",
            report.errors
        );
        report.errors.push(err);
        assert!(
            !report.ok(),
            "a pushed mismatch error must make the report not-ok (fail closed)"
        );
    }

    #[test]
    fn payload_phase_times_are_ignored_in_report_equality() {
        let a = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        let b = build_domain53_payload(proof_fixture_rows(), CURRENT_VALIDATED_SOURCE_PROOFS);
        assert_eq!(a, b, "identical inputs must produce equal reports");
        assert_eq!(a.payload, b.payload);
        assert_eq!(a.gate_summary, b.gate_summary);
        assert_eq!(a.errors, b.errors);
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
    fn rejects_incoming_action_rows_without_queue_source_proof() {
        let rows = proof_fixture_rows();

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_INCOMING_ACTION_QUEUE_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("incoming diplomatic action rows require")));
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_intel_network_graph_rows_without_graph_source_proof() {
        let rows = proof_fixture_rows();

        let report = build_domain53_payload(
            rows,
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| error.contains("intel-network graph rows require")));
        assert_ne!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
            0
        );
    }

    #[test]
    fn rejects_subject_autonomy_current_without_exact_source_proof() {
        let report = build_domain53_payload(
            proof_fixture_rows(),
            CURRENT_VALIDATED_SOURCE_PROOFS & !SOURCE_PROOF_CURRENT_AUTONOMY_STATUS_870EBF,
        );

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| { error.contains("subject-autonomy current-status witness requires") }));
    }

    #[test]
    fn rejects_subject_autonomy_current_without_completeness_witness() {
        let mut rows = proof_fixture_rows();
        rows.subject_autonomy_status_complete = false;

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| { error.contains("current-status rows require a completeness witness") }));
    }

    #[test]
    fn rejects_subject_autonomy_current_with_invalid_score_bounds() {
        let mut rows = proof_fixture_rows();
        rows.subject_autonomy_statuses[0].upper_score_raw =
            rows.subject_autonomy_statuses[0].lower_score_raw;
        rows.subject_autonomy_statuses[0].ratio_raw = 0;

        let report = build_domain53_payload(rows, CURRENT_VALIDATED_SOURCE_PROOFS);

        assert!(!report.ok());
        assert!(report
            .errors
            .iter()
            .any(|error| { error.contains("invalid subject-autonomy status trailer row") }));
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
    fn rejects_legacy_lifecycle_event_without_matching_source_proof() {
        let mut rows = proof_fixture_rows();
        rows.operatives.clear();
        rows.state_events.clear();
        rows.state_events.push(StateEventRow {
            owner_tag_db_id: 1,
            target_tag_db_id: None,
            event_kind: STATE_EVENT_KIND_OBSERVED,
            source_kind: STATE_EVENT_SOURCE_OPERATIVE_LIFECYCLE,
            object_hash: crate::compact::stable_hash32("legacy_operative_lifecycle"),
            value_hash: crate::compact::stable_hash32("legacy_allocator_capacity_47"),
            value_raw: 47,
        });

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
        assert!(evidence.contains("module+0x298B448"));
        assert!(evidence.contains("module+0x298AB48"));
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_WAR_ROOT_870EBF,
            0
        );
        assert_ne!(
            CURRENT_VALIDATED_SOURCE_PROOFS & SOURCE_PROOF_INTEL_NETWORK_GRAPH_870EBF,
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
                & (STATUS_OPERATIONS_DETAIL_RESISTED
                    | STATUS_INTEL_MAP_RESISTED
                    | STATUS_WAR_LIFECYCLE_DETAIL_UNVALIDATED),
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INCOMING_ACTION_QUEUE_UNVALIDATED,
            0
        );
        assert_eq!(
            report.gate_summary.status_flags_or & STATUS_INTEL_NETWORK_GRAPH_UNVALIDATED,
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

    #[test]
    fn relation_slot_batch_matches_per_field_read() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let relation_table = heap + 0x1_0000;

        let max_tag: u32 = 45;
        let slots = [
            (1u32, 0u64),
            (2, heap + 0x9_9000),
            (3, 0xDEAD_BEEFu64),
            (7, heap + 0x1234),
            (max_tag, u64::MAX),
        ];

        let mut mem = FakeMemory::new(module);
        mem.add_chunk(relation_table, (max_tag as usize + 1) * 8);
        for (tag, value) in slots {
            mem.write_u64(relation_table + u64::from(tag) * 8, value);
        }

        let batch_len = (max_tag as usize + 1) * 8;
        let table_bytes = mem.read_bytes(relation_table, batch_len).ok();
        assert!(table_bytes.is_some(), "batch read of a committed table");

        for (tag, expected) in slots {
            let entry_slot = relation_table + u64::from(tag) * 8;
            let per_field = mem.read_u64(entry_slot);

            let batched =
                relation_slot_ptr(&mem, table_bytes.as_deref(), relation_table, tag).unwrap();

            assert_eq!(
                batched, per_field,
                "batched slot {tag} diverged from per-field read"
            );
            assert_eq!(batched, Ok(expected), "slot {tag} value");
        }

        for (tag, _expected) in slots {
            let entry_slot = relation_table + u64::from(tag) * 8;
            let per_field = mem.read_u64(entry_slot);
            let fallback = relation_slot_ptr(&mem, None, relation_table, tag).unwrap();
            assert_eq!(fallback, per_field, "fallback slot {tag}");
        }

        let torn_table = heap + 0x5_0000;
        let torn = relation_slot_ptr(&mem, None, torn_table, 2).unwrap();
        assert!(
            torn.is_err(),
            "unreadable slot must fail-closed (Err) in the fallback path"
        );
        assert!(mem.read_u64(torn_table + 16).is_err());
    }

    fn relation_kind_specs() -> [(usize, u16); 13] {
        [
            (
                profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA,
                REL_KIND_NON_AGGRESSION_PACT,
            ),
            (profile::DOMAIN53_REL_SUBJECT_VTABLE_RVA, REL_KIND_PUPPET),
            (profile::DOMAIN53_REL_EMBARGO_VTABLE_RVA, REL_KIND_EMBARGO),
            (
                profile::DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA,
                REL_KIND_MILITARY_ACCESS,
            ),
            (
                profile::DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA,
                REL_KIND_AIR_BASE_ACCESS,
            ),
            (
                profile::DOMAIN53_REL_DOCKING_RIGHTS_VTABLE_RVA,
                REL_KIND_DOCKING_RIGHTS,
            ),
            (
                profile::DOMAIN53_REL_NAVAL_BLOCKADE_VTABLE_RVA,
                REL_KIND_NAVAL_BLOCKADE,
            ),
            (
                profile::DOMAIN53_REL_MARKET_ACCESS_RIGHTS_VTABLE_RVA,
                REL_KIND_MARKET_ACCESS_RIGHTS,
            ),
            (
                profile::DOMAIN53_REL_EQUIPMENT_PURCHASE_RELATION_VTABLE_RVA,
                REL_KIND_EQUIPMENT_PURCHASE_CONTRACT_RELATION,
            ),
            (
                profile::DOMAIN53_REL_GUARANTEE_VTABLE_RVA,
                REL_KIND_GUARANTEE,
            ),
            (
                profile::DOMAIN53_REL_IMPROVE_RELATIONS_VTABLE_RVA,
                REL_KIND_IMPROVE_RELATIONS,
            ),
            (profile::DOMAIN53_REL_ATTACHE_VTABLE_RVA, REL_KIND_ATTACHE),
            (
                profile::DOMAIN53_REL_LICENCE_PRODUCTION_VTABLE_RVA,
                REL_KIND_LICENCE_PRODUCTION,
            ),
        ]
    }

    fn add_all_relation_kind_container(mem: &mut FakeMemory) {
        let module = mem.module_base;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let rel12 = heap + 0x8_0000;
        let arr = heap + 0x40_0000;
        let obj_base = heap + 0x41_0000;
        let specs = relation_kind_specs();
        mem.add_chunk(arr, specs.len() * 8);
        mem.write_u64(rel12 + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
        mem.write_i32(
            rel12 + profile::DOMAIN53_RELATION_SUBREL_CAP as u64,
            specs.len() as i32,
        );
        mem.write_i32(
            rel12 + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64,
            specs.len() as i32,
        );
        for (index, (vtable_rva, _kind)) in specs.iter().copied().enumerate() {
            let obj = obj_base + (index as u64) * 0x100;
            mem.add_chunk(obj, 0x40);
            mem.write_u64(arr + (index as u64) * 8, obj);
            mem.write_u64(obj, module + vtable_rva as u64);
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_SECOND as u64, 2);
            mem.write_i32(
                obj + profile::DOMAIN53_REL_BASE_START_DATE as u64,
                60_000_000 + index as i32,
            );
            let end = if index == 0 {
                60_900_000
            } else {
                profile::DOMAIN53_REL_DATE_SENTINEL
            };
            mem.write_i32(obj + profile::DOMAIN53_REL_BASE_END_DATE as u64, end);
        }
    }

    fn relation_rows_for_mode(mem: &FakeMemory, mode: Domain53RelationReadMode) -> RowAccumulator {
        let roots = resolve_domain53_frame_roots(mem).expect("roots");
        let mut acc = RowAccumulator::default();
        read_relation_rows_for_mode(
            mem,
            roots.module_base,
            roots.current_game_hour,
            &roots.countries,
            &mut acc,
            mode,
        )
        .expect("relation rows");
        acc
    }

    fn encode_relation_rows(rows: &ExtractedDomain53Rows) -> Vec<u8> {
        encode_payload(&DiplomacyIntelSnapshot {
            opinions: rows.opinions.clone(),
            relation_flags: rows.relation_flags.clone(),
            subjects: rows.subjects.clone(),
            state_events: rows.state_events.clone(),
            ..DiplomacyIntelSnapshot::default()
        })
    }

    #[test]
    fn relation_prefetch_hints_cover_every_line_of_the_copied_hot_window() {
        let hints = RELATION_PREFETCH_HINT_OFFSETS;
        assert_eq!(
            hints,
            relation_prefetch_hint_offsets(),
            "the promoted hint table drifted from its generator"
        );
        let windows = [
            (RELATION_COUNT_READ_START, RELATION_COUNT_READ_LEN),
            (RELATION_HOT_FIELD_START, RELATION_HOT_FIELD_LEN),
        ];

        for &offset in hints.iter() {
            assert!(
                windows
                    .iter()
                    .any(|&(start, len)| (start..start + len).contains(&offset)),
                "prefetch hint 0x{offset:x} lies outside every copied window \
                 {}",
                windows
                    .iter()
                    .map(|&(start, len)| format!("[0x{start:x}, 0x{:x})", start + len))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }

        for pair in hints.windows(2) {
            assert!(
                pair[1] > pair[0],
                "prefetch hints are not strictly increasing: 0x{:x} then 0x{:x}",
                pair[0],
                pair[1]
            );
        }

        let expected: Vec<usize> = {
            let mut out = Vec::with_capacity(RELATION_PREFETCH_HINT_COUNT);
            for (start, len) in windows {
                let last = start + len - 1;
                let mut offset = start;
                while offset < last {
                    out.push(offset);
                    offset += 64;
                }
                out.push(last);
            }
            out
        };
        assert_eq!(
            hints.as_slice(),
            expected.as_slice(),
            "prefetch hints are not the per-window 64-byte-step pattern"
        );

        for align in 0..64usize {
            let base = 0x1_0000_0000usize + align;
            for (start, len) in windows {
                let last = start + len - 1;
                let mut line = (base + start) & !63usize;
                while line <= base + last {
                    assert!(
                        hints
                            .iter()
                            .any(|&offset| (base + offset) & !63usize == line),
                        "at entry alignment {align}, copied line 0x{line:x} of \
                         [0x{:x}, 0x{:x}] is never prefetched",
                        base + start,
                        base + last
                    );
                    line += 64;
                }
            }
        }

        for field in [
            profile::DOMAIN53_RELATION_SUBREL_COUNT,
            profile::DOMAIN53_RELATION_SUBJECT_PTR,
            profile::DOMAIN53_RELATION_OPINION,
        ] {
            assert!(
                hints
                    .iter()
                    .any(|&offset| offset & !63usize == field & !63usize),
                "decoded field 0x{field:x} lost its prefetch line"
            );
        }

        for &offset in hints.iter() {
            assert!(
                !(RELATION_COUNT_READ_START + RELATION_COUNT_READ_LEN..RELATION_HOT_FIELD_START)
                    .contains(&offset),
                "prefetch hint 0x{offset:x} warms the [0x{:x}, 0x{:x}) interior \
                 that no per-pair read copies",
                RELATION_COUNT_READ_START + RELATION_COUNT_READ_LEN,
                RELATION_HOT_FIELD_START
            );
        }
        assert_eq!(
            hints.len(),
            5,
            "the two narrowed windows span 5 lines at worst; a different hint \
             count means a window moved"
        );
    }

    #[test]
    fn relation_offsets_mode_rows_are_byte_identical_to_scan_mode() {
        let mut mem = fake_domain53_memory(true, true);
        add_all_relation_kind_container(&mut mem);

        let offsets = relation_rows_for_mode(&mem, Domain53RelationReadMode::Offsets);
        let scan = relation_rows_for_mode(&mem, Domain53RelationReadMode::Scan);

        assert_eq!(offsets.skipped_rows, 0);
        assert_eq!(offsets.rows, scan.rows);
        assert_eq!(offsets.skipped_rows, scan.skipped_rows);
        assert_eq!(offsets.observed_rows, scan.observed_rows);
        assert_eq!(
            encode_relation_rows(&offsets.rows),
            encode_relation_rows(&scan.rows),
            "DOMAIN_DATA relation bytes differ between offsets and scan modes"
        );
        assert!(offsets.rows.opinions.iter().any(|row| {
            row.src_tag_db_id == 1 && row.target_tag_db_id == 2 && row.opinion == 10
        }));
        assert!(offsets
            .rows
            .subjects
            .iter()
            .any(|row| { row.overlord_tag_db_id == 1 && row.subject_tag_db_id == 2 }));
        for (_vtable_rva, kind) in relation_kind_specs() {
            assert!(
                offsets.relflag_offsets.per_kind.contains_key(&kind),
                "missing relation kind {kind} from all-kind vector fixture"
            );
        }
    }

    #[test]
    fn relation_state_event_reserve_is_not_pair_scaled() {
        let mem = fake_domain53_memory(true, true);
        let roots = resolve_domain53_frame_roots(&mem).expect("roots");

        let unmapped = profile::DOMAIN53_HEAP_LO as u64 + 0x40_0000;
        let countries: Vec<CountryRoot> = (0..64u32)
            .map(|index| CountryRoot {
                tag_db_id: index + 1,
                country_ptr: unmapped + u64::from(index) * 0x1000,
            })
            .collect();
        let pair_estimate = countries.len() * (countries.len() - 1);
        assert_eq!(pair_estimate, 4032);

        let mut acc = RowAccumulator::default();
        read_relation_rows_impl(
            &mem,
            roots.module_base,
            roots.current_game_hour,
            &countries,
            &mut acc,
            RelationReadPass::Offsets,
        )
        .expect("relation rows");

        assert_eq!(
            acc.rows.state_events.len(),
            0,
            "fixture must refuse every root, or the capacity claim below is not \
             about the reserve"
        );
        assert_eq!(
            acc.rows.state_events.capacity(),
            0,
            "state_events was reserved by the ordered-pair count after the \
             per-opinion push it was sized for was removed"
        );
        assert!(
            acc.rows.opinions.capacity() >= pair_estimate,
            "the opinions reserve is still earned by a push that happens and must \
             not be removed with it (capacity {})",
            acc.rows.opinions.capacity()
        );
    }

    #[test]
    fn relation_shadow_mode_detects_legacy_scan_divergence() {
        let mut mem = fake_domain53_memory(true, true);
        add_all_relation_kind_container(&mut mem);
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let rel13 = heap + 0x9_0000;
        mem.write_u64(
            rel13 + 0x40,
            mem.module_base + profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA as u64,
        );

        let offsets = relation_rows_for_mode(&mem, Domain53RelationReadMode::Offsets);
        let shadow = relation_rows_for_mode(&mem, Domain53RelationReadMode::Shadow);

        assert_eq!(
            shadow.rows, offsets.rows,
            "shadow must emit offsets-mode rows while reporting the mismatch"
        );
        assert_eq!(shadow.skipped_rows, offsets.skipped_rows);
        assert!(shadow.relation_shadow_mismatches > 0);
    }

    #[test]
    fn relation_read_mode_parse_defaults_to_offsets() {
        assert_eq!(
            parse_relation_read_mode(None),
            Domain53RelationReadMode::Offsets
        );
        assert_eq!(
            parse_relation_read_mode(Some("scan")),
            Domain53RelationReadMode::Scan
        );
        assert_eq!(
            parse_relation_read_mode(Some("OFFSETS")),
            Domain53RelationReadMode::Offsets
        );
        assert_eq!(
            parse_relation_read_mode(Some(" shadow ")),
            Domain53RelationReadMode::Shadow
        );
        assert_eq!(
            parse_relation_read_mode(Some("bad")),
            Domain53RelationReadMode::Offsets
        );
    }

    #[test]
    fn relflag_histogram_counts_per_kind() {
        let mut hist = RelationFlagOffsetHistogram::default();
        assert_eq!(hist.total_matches, 0);
        assert!(hist.per_kind.is_empty());
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_NON_AGGRESSION_PACT);
        hist.record_kind_count(REL_KIND_EMBARGO);
        hist.record_kind_count(REL_KIND_PUPPET);
        hist.record_kind_count(REL_KIND_PUPPET);
        assert_eq!(hist.per_kind[&REL_KIND_NON_AGGRESSION_PACT], (3, 0));
        assert_eq!(hist.per_kind[&REL_KIND_EMBARGO], (1, 0));
        assert_eq!(hist.per_kind[&REL_KIND_PUPPET], (2, 0));
        assert_eq!(hist.total_matches, 6);
    }

    #[test]
    fn read_relation_container_emits_owned_kinds_and_dedups() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let vtables = relation_vtable_kinds(module).expect("vtables");
        let rel = heap + 0x1000;
        let arr = heap + 0x2000;
        let nap = heap + 0x3000;
        let milacc = heap + 0x3100;
        let airbase = heap + 0x3200;
        let unknown = heap + 0x3300;

        let mut mem = FakeMemory::new(module);
        for (addr, len) in [
            (rel, 0x100usize),
            (arr, 0x40),
            (nap, 0x40),
            (milacc, 0x40),
            (airbase, 0x40),
            (unknown, 0x40),
        ] {
            mem.add_chunk(addr, len);
        }
        mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
        mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 8);
        mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 6);
        mem.write_u64(arr, nap);
        mem.write_u64(arr + 8, milacc);
        mem.write_u64(arr + 16, airbase);
        mem.write_u64(arr + 24, unknown);
        mem.write_u64(arr + 32, 0);
        mem.write_u64(arr + 40, nap);
        mem.write_u64(
            nap,
            module + profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA as u64,
        );
        mem.write_u32(nap + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
        mem.write_u32(nap + profile::DOMAIN53_REL_BASE_SECOND as u64, 2);
        mem.write_i32(nap + profile::DOMAIN53_REL_BASE_START_DATE as u64, 100);
        mem.write_i32(nap + profile::DOMAIN53_REL_BASE_END_DATE as u64, 200);
        mem.write_u64(
            milacc,
            module + profile::DOMAIN53_REL_MILITARY_ACCESS_VTABLE_RVA as u64,
        );
        mem.write_u32(milacc + profile::DOMAIN53_REL_BASE_FIRST as u64, 2);
        mem.write_u32(milacc + profile::DOMAIN53_REL_BASE_SECOND as u64, 1);
        mem.write_i32(milacc + profile::DOMAIN53_REL_BASE_START_DATE as u64, 50);
        mem.write_i32(
            milacc + profile::DOMAIN53_REL_BASE_END_DATE as u64,
            profile::DOMAIN53_REL_DATE_SENTINEL,
        );
        mem.write_u64(
            airbase,
            module + profile::DOMAIN53_REL_AIR_BASE_ACCESS_VTABLE_RVA as u64,
        );
        mem.write_u32(airbase + profile::DOMAIN53_REL_BASE_FIRST as u64, 1);
        mem.write_u32(airbase + profile::DOMAIN53_REL_BASE_SECOND as u64, 2);
        mem.write_i32(airbase + profile::DOMAIN53_REL_BASE_START_DATE as u64, 60);
        mem.write_i32(
            airbase + profile::DOMAIN53_REL_BASE_END_DATE as u64,
            profile::DOMAIN53_REL_DATE_SENTINEL,
        );
        mem.write_u64(unknown, module + 0x0099_9999);

        let mut acc = RowAccumulator::default();
        let mut seen_flags = BTreeSet::new();
        read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen_flags);

        assert_eq!(
            acc.rows.relation_flags,
            vec![
                RelationFlagRow {
                    src_tag_db_id: 1,
                    target_tag_db_id: 2,
                    flag_kind: REL_KIND_NON_AGGRESSION_PACT,
                    directed: true,
                },
                RelationFlagRow {
                    src_tag_db_id: 2,
                    target_tag_db_id: 1,
                    flag_kind: REL_KIND_MILITARY_ACCESS,
                    directed: true,
                },
            ],
        );
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_NON_AGGRESSION_PACT],
            (2, 0)
        );
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_MILITARY_ACCESS],
            (1, 0)
        );
        assert_eq!(
            acc.relflag_offsets.per_kind[&REL_KIND_AIR_BASE_ACCESS],
            (1, 0)
        );
        assert_eq!(acc.relflag_offsets.total_matches, 4);
        assert_eq!(acc.skipped_rows, 0);
        assert!(seen_flags.contains(&(1, 2, REL_KIND_NON_AGGRESSION_PACT)));
        assert!(seen_flags.contains(&(2, 1, REL_KIND_MILITARY_ACCESS)));
        let flag_events = acc
            .rows
            .state_events
            .iter()
            .filter(|e| e.source_kind == STATE_EVENT_SOURCE_RELATION_FLAG)
            .count();
        assert_eq!(flag_events, 5);
        assert!(acc.rows.state_events.iter().any(|e| e.value_raw == 200));
        assert!(!acc
            .rows
            .state_events
            .iter()
            .any(|e| e.value_raw == i64::from(profile::DOMAIN53_REL_DATE_SENTINEL)));
    }

    #[test]
    fn read_relation_container_fail_closed() {
        let module = 0x1_4000_0000u64;
        let heap = profile::DOMAIN53_HEAP_LO as u64 + 0x10_0000;
        let vtables = relation_vtable_kinds(module).expect("vtables");
        let rel = heap + 0x1000;
        let arr = heap + 0x2000;

        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 0);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 0);
        }
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, 0);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 0);
        }
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 2);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 9999);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
        {
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(arr, heap + 0x9_0000);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
        {
            let obj = heap + 0x4000;
            let mut mem = FakeMemory::new(module);
            mem.add_chunk(rel, 0x100);
            mem.add_chunk(arr, 0x40);
            mem.add_chunk(obj, 0x40);
            mem.write_u64(rel + profile::DOMAIN53_RELATION_SUBREL_BEGIN as u64, arr);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_CAP as u64, 4);
            mem.write_i32(rel + profile::DOMAIN53_RELATION_SUBREL_COUNT as u64, 1);
            mem.write_u64(arr, obj);
            mem.write_u64(
                obj,
                module + profile::DOMAIN53_REL_NON_AGGRESSION_PACT_VTABLE_RVA as u64,
            );
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_FIRST as u64, 3);
            mem.write_u32(obj + profile::DOMAIN53_REL_BASE_SECOND as u64, 4);
            mem.write_i32(obj + profile::DOMAIN53_REL_BASE_START_DATE as u64, 100);
            mem.write_i32(obj + profile::DOMAIN53_REL_BASE_END_DATE as u64, 200);
            let mut acc = RowAccumulator::default();
            let mut seen = BTreeSet::new();
            read_relation_container(&mem, rel, &vtables, 1, 2, 1000, &mut acc, &mut seen);
            assert!(acc.rows.relation_flags.is_empty());
            assert_eq!(acc.skipped_rows, 1);
        }
    }

    #[test]
    fn opinion_value_hashes_match_formatted_labels_over_full_domain() {
        let opinion_value_hashes: Vec<u32> = (-MAX_OPINION_ABS..=MAX_OPINION_ABS)
            .map(|opinion| crate::compact::stable_hash32(&format!("opinion_{opinion}")))
            .collect();
        assert_eq!(
            opinion_value_hashes.len(),
            (2 * MAX_OPINION_ABS + 1) as usize
        );

        for opinion in -MAX_OPINION_ABS..=MAX_OPINION_ABS {
            let idx = (opinion + MAX_OPINION_ABS) as usize;
            let expected = format!("opinion_{opinion}");
            assert_eq!(
                opinion_value_hashes[idx],
                crate::compact::stable_hash32(&expected),
                "value hash for {opinion}"
            );
        }

        for opinion in [-MAX_OPINION_ABS, -1, 0, 1, MAX_OPINION_ABS] {
            let idx = (opinion + MAX_OPINION_ABS) as usize;
            assert_eq!(
                opinion_value_hashes[idx],
                crate::compact::stable_hash32(&format!("opinion_{opinion}"))
            );
        }
    }
}
