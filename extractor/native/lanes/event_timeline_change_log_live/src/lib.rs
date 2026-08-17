use std::collections::{BTreeMap, BTreeSet};

use compact_contract::{
    compact_status_failure_flags_after_expected_partial_tolerance, compact_status_unknown_flags_v1,
    parse_compact_record_header, validate_compact_record_semantics, RecordFramingError,
    COMPACT_STATUS_FAILURE_MASK,
};
use recorder_host::domain_data_authority::{summarize_domain_data_payload, DomainDataContext};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const COMPACT_DOMAIN_FRAME: u16 = 1000;
pub const COMPACT_DOMAIN_RC_PROBE: u16 = 250;

/// Lane-local proof namespace. This is deliberately outside the production
/// roster so the proof wire cannot masquerade as canonical domain 44 (supply).
pub const DOMAIN_ID: u16 = 249;
pub const DOMAIN_NAME: &str = "event_timeline_change_log_live";
pub const EVENT_PAYLOAD_VERSION: u16 = 1;
pub const EVENT_PAYLOAD_HEADER_LEN: usize = 64;
pub const EVENT_ROW_LEN: usize = 112;

pub const EVENT_FLAG_SUBJECT_PRESENT: u32 = 1;
pub const EVENT_FLAG_OBJECT_PRESENT: u32 = 1 << 1;
pub const EVENT_FLAG_KEY_PRESENT: u32 = 1 << 2;
pub const EVENT_FLAG_FROM_PRESENT: u32 = 1 << 3;
pub const EVENT_FLAG_TO_PRESENT: u32 = 1 << 4;
pub const EVENT_FLAG_DETAIL_PRESENT: u32 = 1 << 5;
pub const EVENT_FLAG_DIRECT_CAPTURE: u32 = 1 << 6;
pub const EVENT_FLAG_DERIVED: u32 = 1 << 7;
pub const EVENT_FLAG_MARKER_SUPERSEDES_DIFF: u32 = 1 << 8;
pub const EVENT_FLAG_PROOF_FIXTURE: u32 = 1 << 31;

const KNOWN_EVENT_FLAGS: u32 = EVENT_FLAG_SUBJECT_PRESENT
    | EVENT_FLAG_OBJECT_PRESENT
    | EVENT_FLAG_KEY_PRESENT
    | EVENT_FLAG_FROM_PRESENT
    | EVENT_FLAG_TO_PRESENT
    | EVENT_FLAG_DETAIL_PRESENT
    | EVENT_FLAG_DIRECT_CAPTURE
    | EVENT_FLAG_DERIVED
    | EVENT_FLAG_MARKER_SUPERSEDES_DIFF
    | EVENT_FLAG_PROOF_FIXTURE;

pub const SOURCE_DOMAIN_FOCUS: u16 = 12;
pub const SOURCE_DOMAIN_TECH: u16 = 13;
pub const SOURCE_DOMAIN_DECISIONS: u16 = 16;
pub const SOURCE_DOMAIN_DIPLOMACY: u16 = 17;
pub const SOURCE_DOMAIN_DIVISIONS: u16 = 20;
pub const SOURCE_DOMAIN_LAND_COMBATS: u16 = 21;
pub const SOURCE_DOMAIN_MAP_CONTROL: u16 = 22;
pub const SOURCE_DOMAIN_MAP_DETAIL: u16 = 23;
pub const SOURCE_DOMAIN_ARMY_ORDERS: u16 = 27;
pub const SOURCE_DOMAIN_OOB_COMMAND: u16 = 28;
pub const SOURCE_DOMAIN_SHIPS: u16 = 29;
pub const SOURCE_DOMAIN_AIR_WINGS: u16 = 30;
pub const SOURCE_DOMAIN_PROJECTS: u16 = 41;
pub const SOURCE_DOMAIN_MARKET_TRADE: u16 = 43;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum EventKind {
    WarDeclared = 1,
    Peace = 2,
    Capitulated = 3,
    FactionJoined = 4,
    FactionLeft = 5,
    FocusCompleted = 6,
    TechResearched = 7,
    TechLost = 8,
    DecisionFired = 9,
    DecisionExpired = 10,
    CombatStarted = 11,
    CombatUpdated = 12,
    CombatEnded = 13,
    StateOwnerChange = 14,
    StateControllerChange = 15,
    ProvinceControllerChange = 16,
    BuildingCompleted = 17,
    ConstructionCompleted = 18,
    ShipCreated = 19,
    ShipDeleted = 20,
    PlaneCreated = 21,
    PlaneDeleted = 22,
    DivisionCreated = 23,
    DivisionDestroyed = 24,
    DivisionMoved = 25,
    DivisionTemplateChanged = 26,
    ProductionMilestone = 27,
    FactoryMilestone = 28,
    ProjectCompleted = 29,
    LendLeaseStarted = 30,
    LendLeaseEnded = 31,
    ArmyFormed = 32,
    ArmyDisbanded = 33,
    OrderChanged = 34,
    OobMembershipChanged = 35,
}

impl EventKind {
    pub fn from_raw(raw: u16) -> Option<Self> {
        Some(match raw {
            1 => Self::WarDeclared,
            2 => Self::Peace,
            3 => Self::Capitulated,
            4 => Self::FactionJoined,
            5 => Self::FactionLeft,
            6 => Self::FocusCompleted,
            7 => Self::TechResearched,
            8 => Self::TechLost,
            9 => Self::DecisionFired,
            10 => Self::DecisionExpired,
            11 => Self::CombatStarted,
            12 => Self::CombatUpdated,
            13 => Self::CombatEnded,
            14 => Self::StateOwnerChange,
            15 => Self::StateControllerChange,
            16 => Self::ProvinceControllerChange,
            17 => Self::BuildingCompleted,
            18 => Self::ConstructionCompleted,
            19 => Self::ShipCreated,
            20 => Self::ShipDeleted,
            21 => Self::PlaneCreated,
            22 => Self::PlaneDeleted,
            23 => Self::DivisionCreated,
            24 => Self::DivisionDestroyed,
            25 => Self::DivisionMoved,
            26 => Self::DivisionTemplateChanged,
            27 => Self::ProductionMilestone,
            28 => Self::FactoryMilestone,
            29 => Self::ProjectCompleted,
            30 => Self::LendLeaseStarted,
            31 => Self::LendLeaseEnded,
            32 => Self::ArmyFormed,
            33 => Self::ArmyDisbanded,
            34 => Self::OrderChanged,
            35 => Self::OobMembershipChanged,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::WarDeclared => "war_declared",
            Self::Peace => "peace",
            Self::Capitulated => "capitulated",
            Self::FactionJoined => "faction_joined",
            Self::FactionLeft => "faction_left",
            Self::FocusCompleted => "focus_completed",
            Self::TechResearched => "tech_researched",
            Self::TechLost => "tech_lost",
            Self::DecisionFired => "decision_fired",
            Self::DecisionExpired => "decision_expired",
            Self::CombatStarted => "combat_started",
            Self::CombatUpdated => "combat_updated",
            Self::CombatEnded => "combat_ended",
            Self::StateOwnerChange => "state_owner_change",
            Self::StateControllerChange => "state_controller_change",
            Self::ProvinceControllerChange => "province_controller_change",
            Self::BuildingCompleted => "building_completed",
            Self::ConstructionCompleted => "construction_completed",
            Self::ShipCreated => "ship_created",
            Self::ShipDeleted => "ship_deleted",
            Self::PlaneCreated => "plane_created",
            Self::PlaneDeleted => "plane_deleted",
            Self::DivisionCreated => "division_created",
            Self::DivisionDestroyed => "division_destroyed",
            Self::DivisionMoved => "division_moved",
            Self::DivisionTemplateChanged => "division_template_changed",
            Self::ProductionMilestone => "production_milestone",
            Self::FactoryMilestone => "factory_milestone",
            Self::ProjectCompleted => "project_completed",
            Self::LendLeaseStarted => "lend_lease_started",
            Self::LendLeaseEnded => "lend_lease_ended",
            Self::ArmyFormed => "army_formed",
            Self::ArmyDisbanded => "army_disbanded",
            Self::OrderChanged => "order_changed",
            Self::OobMembershipChanged => "oob_membership_changed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum SourceKind {
    DirectCaptured = 1,
    DerivedDelta = 2,
    Marker = 3,
    SyntheticGap = 4,
}

impl SourceKind {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::DirectCaptured,
            2 => Self::DerivedDelta,
            3 => Self::Marker,
            4 => Self::SyntheticGap,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::DirectCaptured => "direct_captured",
            Self::DerivedDelta => "derived_delta",
            Self::Marker => "marker",
            Self::SyntheticGap => "synthetic_gap",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Confidence {
    Exact = 1,
    Sampled = 2,
    Interval = 3,
    Inferred = 4,
    Gap = 5,
}

impl Confidence {
    pub fn from_raw(raw: u8) -> Option<Self> {
        Some(match raw {
            1 => Self::Exact,
            2 => Self::Sampled,
            3 => Self::Interval,
            4 => Self::Inferred,
            5 => Self::Gap,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Sampled => "sampled",
            Self::Interval => "interval",
            Self::Inferred => "inferred",
            Self::Gap => "gap",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRecord {
    pub event_kind: EventKind,
    pub source_domain_id: u16,
    pub producer_domain_id: u16,
    pub source_kind: SourceKind,
    pub confidence: Confidence,
    pub flags: u32,
    pub game_hour_raw: i32,
    pub interval_start_hour_raw: i32,
    pub source_sequence: u64,
    pub source_row_index: u32,
    pub subject_hash: u64,
    pub object_hash: u64,
    pub key_hash: u64,
    pub from_hash: u64,
    pub to_hash: u64,
    pub detail_hash: u64,
    pub provenance_hash: u64,
    pub identity_hash: u64,
    pub generation: u32,
    pub detail_len: u32,
}

impl EventRecord {
    pub fn new(
        event_kind: EventKind,
        source_domain_id: u16,
        source_kind: SourceKind,
        confidence: Confidence,
        game_hour_raw: i32,
    ) -> Self {
        Self {
            event_kind,
            source_domain_id,
            producer_domain_id: DOMAIN_ID,
            source_kind,
            confidence,
            flags: 0,
            game_hour_raw,
            interval_start_hour_raw: i32::MIN,
            source_sequence: 0,
            source_row_index: 0,
            subject_hash: 0,
            object_hash: 0,
            key_hash: 0,
            from_hash: 0,
            to_hash: 0,
            detail_hash: 0,
            provenance_hash: 0,
            identity_hash: 0,
            generation: 0,
            detail_len: 0,
        }
    }

    pub fn with_keys(
        mut self,
        subject: Option<&str>,
        object: Option<&str>,
        key: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Self {
        if let Some(value) = subject {
            self.subject_hash = stable_hash(value);
            self.flags |= EVENT_FLAG_SUBJECT_PRESENT;
        }
        if let Some(value) = object {
            self.object_hash = stable_hash(value);
            self.flags |= EVENT_FLAG_OBJECT_PRESENT;
        }
        if let Some(value) = key {
            self.key_hash = stable_hash(value);
            self.flags |= EVENT_FLAG_KEY_PRESENT;
        }
        if let Some(value) = from {
            self.from_hash = stable_hash(value);
            self.flags |= EVENT_FLAG_FROM_PRESENT;
        }
        if let Some(value) = to {
            self.to_hash = stable_hash(value);
            self.flags |= EVENT_FLAG_TO_PRESENT;
        }
        self.identity_hash = self.compute_identity_hash();
        self.provenance_hash = self.compute_provenance_hash();
        self
    }

    pub fn with_detail(mut self, detail: &str) -> Self {
        self.detail_hash = stable_hash(detail);
        self.detail_len = detail.len().min(u32::MAX as usize) as u32;
        self.flags |= EVENT_FLAG_DETAIL_PRESENT;
        self.identity_hash = self.compute_identity_hash();
        self.provenance_hash = self.compute_provenance_hash();
        self
    }

    pub fn with_source_position(mut self, source_sequence: u64, source_row_index: u32) -> Self {
        self.source_sequence = source_sequence;
        self.source_row_index = source_row_index;
        self.provenance_hash = self.compute_provenance_hash();
        self
    }

    pub fn with_generation(mut self, generation: u32) -> Self {
        self.generation = generation;
        self.identity_hash = self.compute_identity_hash();
        self.provenance_hash = self.compute_provenance_hash();
        self
    }

    pub fn mark_proof_fixture(mut self) -> Self {
        self.flags |= EVENT_FLAG_PROOF_FIXTURE;
        self
    }

    pub fn compute_identity_hash(&self) -> u64 {
        let mut bytes = Vec::with_capacity(64);
        bytes.extend_from_slice(&(self.event_kind as u16).to_le_bytes());
        bytes.extend_from_slice(&self.key_hash.to_le_bytes());
        bytes.extend_from_slice(&self.from_hash.to_le_bytes());
        bytes.extend_from_slice(&self.to_hash.to_le_bytes());
        bytes.extend_from_slice(&self.subject_hash.to_le_bytes());
        bytes.extend_from_slice(&self.object_hash.to_le_bytes());
        bytes.extend_from_slice(&self.generation.to_le_bytes());
        stable_hash_bytes(&bytes)
    }

    pub fn compute_provenance_hash(&self) -> u64 {
        let mut bytes = Vec::with_capacity(48);
        bytes.extend_from_slice(&self.source_domain_id.to_le_bytes());
        bytes.extend_from_slice(&(self.source_kind as u8).to_le_bytes());
        bytes.extend_from_slice(&(self.confidence as u8).to_le_bytes());
        bytes.extend_from_slice(&self.source_sequence.to_le_bytes());
        bytes.extend_from_slice(&self.source_row_index.to_le_bytes());
        bytes.extend_from_slice(&self.detail_hash.to_le_bytes());
        stable_hash_bytes(&bytes)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventPayloadGateSummary {
    pub ok: bool,
    pub row_count: u64,
    pub direct_count: u64,
    pub derived_count: u64,
    pub marker_count: u64,
    pub interval_count: u64,
    pub inferred_count: u64,
    pub gap_count: u64,
    pub proof_fixture_count: u64,
    pub status_flags: u32,
    pub source_domains: BTreeSet<u16>,
    pub event_kind_counts: BTreeMap<u16, u64>,
    pub source_kind_counts: BTreeMap<u8, u64>,
    pub confidence_counts: BTreeMap<u8, u64>,
    pub parse_errors: Vec<String>,
    pub source_bindings: Vec<EventSourceBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSourceBinding {
    pub event_kind: Option<EventKind>,
    pub source_domain_id: u16,
    pub source_sequence: u64,
    pub source_row_index: u32,
    pub game_hour_raw: i32,
    pub flags: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CompactSourceSummary {
    pub total_bytes: usize,
    pub record_count: u64,
    pub domain_perf_record_count: u64,
    pub domain_data_record_count: u64,
    pub frame_end_record_count: u64,
    pub total_payload_bytes: u64,
    pub max_payload_len: usize,
    pub status_flags_or: u32,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    pub unique_hours: BTreeSet<i32>,
    pub skipped_hours: Vec<i32>,
    pub domains_seen: BTreeSet<u16>,
    pub per_domain_data_record_count: BTreeMap<u16, u64>,
    pub per_domain_perf_record_count: BTreeMap<u16, u64>,
    pub per_hour_data_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub per_hour_perf_domain_counts: BTreeMap<i32, BTreeMap<u16, u64>>,
    pub frame_end_hour_counts: BTreeMap<i32, u64>,
    pub parse_errors: Vec<String>,
    pub ordering_errors: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventCompactGateSummary {
    pub source_summary: CompactSourceSummary,
    pub event_payloads: u64,
    pub event_rows_total: u64,
    pub event_kind_counts: BTreeMap<u16, u64>,
    pub source_domains: BTreeSet<u16>,
    pub payload_errors: Vec<String>,
    pub ok: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFamilyRequirement {
    pub family: &'static str,
    pub event_kinds: &'static [EventKind],
    pub required_domain_ids: &'static [u16],
    pub optional_domain_ids: &'static [u16],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventFamilyGate {
    pub family: &'static str,
    pub supported: bool,
    pub required_domain_ids: Vec<u16>,
    pub optional_domain_ids: Vec<u16>,
    pub present_domain_ids: Vec<u16>,
    pub missing_domain_ids: Vec<u16>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceUnitCheck {
    Present,
    Zero,
    Equals(&'static str),
}

impl SourceUnitCheck {
    pub fn name(&self) -> String {
        match self {
            Self::Present => "present".to_string(),
            Self::Zero => "zero".to_string(),
            Self::Equals(other) => format!("equals:{other}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceUnitRequirement {
    pub unit: &'static str,
    pub check: SourceUnitCheck,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSourceAdapterContract {
    pub family: &'static str,
    pub source_domain_id: u16,
    pub event_kinds: Vec<EventKind>,
    pub identity_fields: &'static [&'static str],
    pub provenance_fields: &'static [&'static str],
    pub unit_requirements: Vec<SourceUnitRequirement>,
    pub adapter_note: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceDomainUnitSnapshot {
    pub domain_id: u16,
    pub data_record_count: u64,
    pub parse_error_count: u64,
    pub units: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventSourceAdapterGate {
    pub family: &'static str,
    pub source_domain_id: u16,
    pub ready: bool,
    pub blockers: Vec<String>,
    pub unit_requirements: Vec<SourceUnitRequirement>,
}

pub fn event_family_requirements() -> Vec<EventFamilyRequirement> {
    vec![
        EventFamilyRequirement {
            family: "war_faction_capitulation",
            event_kinds: &[
                EventKind::WarDeclared,
                EventKind::Peace,
                EventKind::FactionJoined,
                EventKind::FactionLeft,
                EventKind::Capitulated,
            ],
            required_domain_ids: &[SOURCE_DOMAIN_DIPLOMACY],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "focus_completed",
            event_kinds: &[EventKind::FocusCompleted],
            required_domain_ids: &[SOURCE_DOMAIN_FOCUS],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "research_completed",
            event_kinds: &[EventKind::TechResearched, EventKind::TechLost],
            required_domain_ids: &[SOURCE_DOMAIN_TECH],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "decision_lifecycle",
            event_kinds: &[EventKind::DecisionFired, EventKind::DecisionExpired],
            required_domain_ids: &[SOURCE_DOMAIN_DECISIONS],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "combat_lifecycle",
            event_kinds: &[
                EventKind::CombatStarted,
                EventKind::CombatUpdated,
                EventKind::CombatEnded,
            ],
            required_domain_ids: &[SOURCE_DOMAIN_LAND_COMBATS],
            optional_domain_ids: &[SOURCE_DOMAIN_DIVISIONS],
        },
        EventFamilyRequirement {
            family: "territory_control_changes",
            event_kinds: &[
                EventKind::StateOwnerChange,
                EventKind::StateControllerChange,
                EventKind::ProvinceControllerChange,
            ],
            required_domain_ids: &[SOURCE_DOMAIN_MAP_CONTROL],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "construction_completion",
            event_kinds: &[
                EventKind::BuildingCompleted,
                EventKind::ConstructionCompleted,
            ],
            required_domain_ids: &[SOURCE_DOMAIN_MAP_DETAIL],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "division_lifecycle",
            event_kinds: &[
                EventKind::DivisionCreated,
                EventKind::DivisionDestroyed,
                EventKind::DivisionMoved,
                EventKind::DivisionTemplateChanged,
            ],
            required_domain_ids: &[SOURCE_DOMAIN_DIVISIONS],
            optional_domain_ids: &[SOURCE_DOMAIN_OOB_COMMAND, SOURCE_DOMAIN_ARMY_ORDERS],
        },
        EventFamilyRequirement {
            family: "ship_lifecycle",
            event_kinds: &[EventKind::ShipCreated, EventKind::ShipDeleted],
            required_domain_ids: &[SOURCE_DOMAIN_SHIPS],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "plane_lifecycle",
            event_kinds: &[EventKind::PlaneCreated, EventKind::PlaneDeleted],
            required_domain_ids: &[SOURCE_DOMAIN_AIR_WINGS],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "special_project_completion",
            event_kinds: &[EventKind::ProjectCompleted],
            required_domain_ids: &[SOURCE_DOMAIN_PROJECTS],
            optional_domain_ids: &[],
        },
        EventFamilyRequirement {
            family: "market_lend_lease_lifecycle",
            event_kinds: &[EventKind::LendLeaseStarted, EventKind::LendLeaseEnded],
            required_domain_ids: &[SOURCE_DOMAIN_MARKET_TRADE],
            optional_domain_ids: &[],
        },
    ]
}

pub fn gate_event_families(summary: &CompactSourceSummary) -> Vec<EventFamilyGate> {
    event_family_requirements()
        .into_iter()
        .map(|req| {
            let mut missing = Vec::new();
            let mut present = Vec::new();
            for domain_id in req.required_domain_ids {
                if summary.per_domain_data_record_count.contains_key(domain_id) {
                    present.push(*domain_id);
                } else {
                    missing.push(*domain_id);
                }
            }
            for domain_id in req.optional_domain_ids {
                if summary.per_domain_data_record_count.contains_key(domain_id) {
                    present.push(*domain_id);
                }
            }
            EventFamilyGate {
                family: req.family,
                supported: missing.is_empty(),
                required_domain_ids: req.required_domain_ids.to_vec(),
                optional_domain_ids: req.optional_domain_ids.to_vec(),
                present_domain_ids: present,
                missing_domain_ids: missing,
            }
        })
        .collect()
}

pub fn missing_source_adapter_contracts() -> Vec<EventSourceAdapterContract> {
    vec![
        EventSourceAdapterContract {
            family: "ship_lifecycle",
            source_domain_id: SOURCE_DOMAIN_SHIPS,
            event_kinds: vec![EventKind::ShipCreated, EventKind::ShipDeleted],
            identity_fields: &["instance_id"],
            provenance_fields: &[
                "source_sequence",
                "source_row_index",
                "deleted flag",
                "owner_tag_db_id when present",
                "definition token length/hash when present",
            ],
            unit_requirements: vec![
                req_present("ship_rows"),
                req_present("ship_observed_ship_rows"),
                req_present("ship_observed_task_force_rows"),
                req_present("ship_deleted_rows"),
                req_present("ship_task_force_rows"),
                req_present("ship_owner_rows"),
                req_present("ship_definition_rows"),
            ],
            adapter_note: "Consume compact ships rows only; emit create/delete from stable instance_id and deleted-row transitions, with owner/definition as optional provenance.",
        },
        EventSourceAdapterContract {
            family: "plane_lifecycle",
            source_domain_id: SOURCE_DOMAIN_AIR_WINGS,
            event_kinds: vec![EventKind::PlaneCreated, EventKind::PlaneDeleted],
            identity_fields: &["owner_tag_db_id", "wing_id", "generation"],
            provenance_fields: &[
                "source_sequence",
                "source_row_index",
                "deleted flag",
                "count",
                "effective_planes",
                "mission_region when present",
            ],
            unit_requirements: vec![
                req_present("air_wing_rows"),
                req_present("air_wing_observed_rows"),
                req_present("air_wing_deleted_rows"),
                req_present("air_wing_mission_region_rows"),
            ],
            adapter_note: "Consume compact air_wings rows only; maintain lane-local previous counts and emit plane deltas or wing deletion without reading air roots.",
        },
        EventSourceAdapterContract {
            family: "special_project_completion",
            source_domain_id: SOURCE_DOMAIN_PROJECTS,
            event_kinds: vec![EventKind::ProjectCompleted],
            identity_fields: &["project id", "static_def_handle"],
            provenance_fields: &[
                "source_sequence",
                "source_row_index",
                "current_state_enum",
                "current_state_offset",
                "phase_progress_raw",
                "phase_target_raw",
                "prototype_iterations",
            ],
            unit_requirements: vec![
                req_present("project_rows"),
                req_present("project_state_rows"),
                req_present("project_observed_project_rows"),
                req_present("project_observed_project_state_rows"),
                req_zero("project_invalid_project_pool_root_read_fail"),
                req_zero("project_invalid_project_pool_vtable_read_fail"),
                req_zero("project_invalid_project_pool_first_project_read_fail"),
                req_zero("project_invalid_project_candidate_vtable_read_fail"),
                req_zero("project_invalid_project_candidate_sentinel_after_pin"),
                req_zero("project_invalid_project_candidate_fields_invalid"),
                req_zero("project_invalid_project_short_array"),
            ],
            adapter_note: "Consume compact projects progress/state rows only; emit completion on validated state transition, fail closed for invalid project-pool counters.",
        },
        EventSourceAdapterContract {
            family: "market_lend_lease_lifecycle",
            source_domain_id: SOURCE_DOMAIN_MARKET_TRADE,
            event_kinds: vec![EventKind::LendLeaseStarted, EventKind::LendLeaseEnded],
            identity_fields: &["contract id", "direction/counterpart when supplied"],
            provenance_fields: &[
                "source_sequence",
                "source_row_index",
                "buyer pointer validity",
                "variant validity",
                "amount validity",
            ],
            unit_requirements: vec![
                req_present("market_trade_contract_rows"),
                req_present("market_trade_observed_contract_rows"),
                req_equal("market_trade_buyer_ptr_valid_rows", "market_trade_contract_rows"),
                req_equal("market_trade_variant_valid_rows", "market_trade_contract_rows"),
                req_equal("market_trade_amount_valid_rows", "market_trade_contract_rows"),
                req_zero("market_trade_partial_field_rows"),
            ],
            adapter_note: "Consume compact market_trade_live rows only; emit lend-lease lifecycle events only when all contract identity/detail fields are complete.",
        },
    ]
}

pub fn gate_missing_source_adapters(
    domains: &BTreeMap<u16, SourceDomainUnitSnapshot>,
) -> Vec<EventSourceAdapterGate> {
    missing_source_adapter_contracts()
        .into_iter()
        .map(|contract| {
            let mut blockers = Vec::new();
            let Some(domain) = domains.get(&contract.source_domain_id) else {
                blockers.push(format!(
                    "missing compact source domain {} ({})",
                    contract.source_domain_id,
                    source_domain_name(contract.source_domain_id)
                ));
                return EventSourceAdapterGate {
                    family: contract.family,
                    source_domain_id: contract.source_domain_id,
                    ready: false,
                    blockers,
                    unit_requirements: contract.unit_requirements,
                };
            };
            if domain.data_record_count == 0 {
                blockers.push("source domain has zero data records".to_string());
            }
            if domain.parse_error_count != 0 {
                blockers.push(format!(
                    "source domain has {} parser error(s)",
                    domain.parse_error_count
                ));
            }
            for requirement in &contract.unit_requirements {
                match requirement.check {
                    SourceUnitCheck::Present => {
                        if !domain.units.contains_key(requirement.unit) {
                            blockers.push(format!("missing unit counter {}", requirement.unit));
                        }
                    }
                    SourceUnitCheck::Zero => match domain.units.get(requirement.unit) {
                        Some(0) => {}
                        Some(value) => blockers.push(format!(
                            "unit {} is {value}, expected zero",
                            requirement.unit
                        )),
                        None => {
                            blockers.push(format!("missing zero-check unit {}", requirement.unit))
                        }
                    },
                    SourceUnitCheck::Equals(other) => {
                        let left = domain.units.get(requirement.unit).copied();
                        let right = domain.units.get(other).copied();
                        match (left, right) {
                            (Some(left), Some(right)) if left == right => {}
                            (Some(left), Some(right)) => blockers.push(format!(
                                "unit {} is {left}, expected to equal {other} ({right})",
                                requirement.unit
                            )),
                            (None, _) => {
                                blockers.push(format!("missing unit counter {}", requirement.unit))
                            }
                            (_, None) => {
                                blockers.push(format!("missing comparison unit counter {other}"))
                            }
                        }
                    }
                }
            }
            EventSourceAdapterGate {
                family: contract.family,
                source_domain_id: contract.source_domain_id,
                ready: blockers.is_empty(),
                blockers,
                unit_requirements: contract.unit_requirements,
            }
        })
        .collect()
}

fn req_present(unit: &'static str) -> SourceUnitRequirement {
    SourceUnitRequirement {
        unit,
        check: SourceUnitCheck::Present,
    }
}

fn req_zero(unit: &'static str) -> SourceUnitRequirement {
    SourceUnitRequirement {
        unit,
        check: SourceUnitCheck::Zero,
    }
}

fn req_equal(unit: &'static str, other: &'static str) -> SourceUnitRequirement {
    SourceUnitRequirement {
        unit,
        check: SourceUnitCheck::Equals(other),
    }
}

pub fn encode_event_payload(rows: &[EventRecord]) -> Vec<u8> {
    let mut direct_count = 0u32;
    let mut derived_count = 0u32;
    let mut marker_count = 0u32;
    let mut interval_count = 0u32;
    let mut inferred_count = 0u32;
    let mut gap_count = 0u32;
    let mut source_domains = BTreeSet::new();
    let mut source_domain_bitmap_low = 0u64;
    let mut identity_hash_xor = 0u64;

    for row in rows {
        match row.source_kind {
            SourceKind::DirectCaptured => direct_count = direct_count.saturating_add(1),
            SourceKind::DerivedDelta => derived_count = derived_count.saturating_add(1),
            SourceKind::Marker => marker_count = marker_count.saturating_add(1),
            SourceKind::SyntheticGap => gap_count = gap_count.saturating_add(1),
        }
        match row.confidence {
            Confidence::Interval => interval_count = interval_count.saturating_add(1),
            Confidence::Inferred => inferred_count = inferred_count.saturating_add(1),
            Confidence::Gap => gap_count = gap_count.saturating_add(1),
            Confidence::Exact | Confidence::Sampled => {}
        }
        source_domains.insert(row.source_domain_id);
        if row.source_domain_id < 64 {
            source_domain_bitmap_low |= 1u64 << row.source_domain_id;
        }
        identity_hash_xor ^= row.identity_hash;
    }

    let mut payload = Vec::with_capacity(EVENT_PAYLOAD_HEADER_LEN + rows.len() * EVENT_ROW_LEN);
    payload.extend_from_slice(&EVENT_PAYLOAD_VERSION.to_le_bytes());
    payload.extend_from_slice(&(EVENT_PAYLOAD_HEADER_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&(EVENT_ROW_LEN as u16).to_le_bytes());
    payload.extend_from_slice(&0u16.to_le_bytes());
    payload.extend_from_slice(&(rows.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&direct_count.to_le_bytes());
    payload.extend_from_slice(&derived_count.to_le_bytes());
    payload.extend_from_slice(&marker_count.to_le_bytes());
    payload.extend_from_slice(&interval_count.to_le_bytes());
    payload.extend_from_slice(&inferred_count.to_le_bytes());
    payload.extend_from_slice(&gap_count.to_le_bytes());
    payload.extend_from_slice(&(source_domains.len().min(u32::MAX as usize) as u32).to_le_bytes());
    payload.extend_from_slice(&source_domain_bitmap_low.to_le_bytes());
    payload.extend_from_slice(&identity_hash_xor.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());

    for row in rows {
        payload.extend_from_slice(&(row.event_kind as u16).to_le_bytes());
        payload.extend_from_slice(&row.source_domain_id.to_le_bytes());
        payload.extend_from_slice(&row.producer_domain_id.to_le_bytes());
        payload.push(row.source_kind as u8);
        payload.push(row.confidence as u8);
        payload.extend_from_slice(&row.flags.to_le_bytes());
        payload.extend_from_slice(&row.game_hour_raw.to_le_bytes());
        payload.extend_from_slice(&row.interval_start_hour_raw.to_le_bytes());
        payload.extend_from_slice(&row.source_sequence.to_le_bytes());
        payload.extend_from_slice(&row.source_row_index.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&row.subject_hash.to_le_bytes());
        payload.extend_from_slice(&row.object_hash.to_le_bytes());
        payload.extend_from_slice(&row.key_hash.to_le_bytes());
        payload.extend_from_slice(&row.from_hash.to_le_bytes());
        payload.extend_from_slice(&row.to_hash.to_le_bytes());
        payload.extend_from_slice(&row.detail_hash.to_le_bytes());
        payload.extend_from_slice(&row.provenance_hash.to_le_bytes());
        payload.extend_from_slice(&row.identity_hash.to_le_bytes());
        payload.extend_from_slice(&row.generation.to_le_bytes());
        payload.extend_from_slice(&row.detail_len.to_le_bytes());
        payload.extend_from_slice(&0u32.to_le_bytes());
    }
    payload
}

pub fn decode_event_payload(offset: usize, payload: &[u8]) -> EventPayloadGateSummary {
    let mut summary = EventPayloadGateSummary::default();
    if payload.len() < EVENT_PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: event timeline payload is {} byte(s), expected at least {EVENT_PAYLOAD_HEADER_LEN}",
            payload.len()
        ));
        return summary;
    }

    let version = read_u16_le(payload, 0);
    let header_len = read_u16_le(payload, 2) as usize;
    let row_len = read_u16_le(payload, 4) as usize;
    let header_flags = read_u16_le(payload, 6);
    let row_count = read_u32_le(payload, 8) as usize;
    let header_direct_count = read_u32_le(payload, 12) as u64;
    let header_derived_count = read_u32_le(payload, 16) as u64;
    let header_marker_count = read_u32_le(payload, 20) as u64;
    let header_interval_count = read_u32_le(payload, 24) as u64;
    let header_inferred_count = read_u32_le(payload, 28) as u64;
    let header_gap_count = read_u32_le(payload, 32) as u64;
    let header_source_domain_count = read_u32_le(payload, 36) as u64;
    let header_source_bitmap = read_u64_le(payload, 40);
    let header_identity_xor = read_u64_le(payload, 48);
    let status_flags = read_u32_le(payload, 56);
    let reserved0 = read_u32_le(payload, 60);
    summary.status_flags = status_flags;

    if version != EVENT_PAYLOAD_VERSION {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported event payload version {version}"
        ));
    }
    if header_len != EVENT_PAYLOAD_HEADER_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported event payload header length {header_len}"
        ));
    }
    if row_len != EVENT_ROW_LEN {
        summary.parse_errors.push(format!(
            "offset {offset}: unsupported event row length {row_len}"
        ));
    }
    if header_flags != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: event payload header flags are nonzero"
        ));
    }
    if reserved0 != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: event payload reserved field is nonzero"
        ));
    }
    let unknown_status = compact_status_unknown_flags_v1(status_flags);
    if unknown_status != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: event payload status flags contain unknown v1 bits {unknown_status:#010x}"
        ));
    }
    let fatal_status =
        compact_status_failure_flags_after_expected_partial_tolerance(DOMAIN_ID, status_flags);
    if fatal_status != 0 {
        summary.parse_errors.push(format!(
            "offset {offset}: event payload carries fatal status flags {fatal_status:#010x}"
        ));
    }
    if version != EVENT_PAYLOAD_VERSION
        || header_len != EVENT_PAYLOAD_HEADER_LEN
        || row_len != EVENT_ROW_LEN
    {
        return summary;
    }
    let expected_len = header_len.saturating_add(row_count.saturating_mul(row_len));
    if payload.len() != expected_len {
        summary.parse_errors.push(format!(
            "offset {offset}: event timeline payload is {} byte(s), expected {expected_len}",
            payload.len()
        ));
        return summary;
    }

    let mut seen_identity = BTreeSet::new();
    let mut identity_xor = 0u64;
    let mut cursor = header_len;
    for row_index in 0..row_count {
        let event_kind_raw = read_u16_le(payload, cursor);
        let source_domain_id = read_u16_le(payload, cursor + 2);
        let producer_domain_id = read_u16_le(payload, cursor + 4);
        let source_kind_raw = payload[cursor + 6];
        let confidence_raw = payload[cursor + 7];
        let flags = read_u32_le(payload, cursor + 8);
        let game_hour_raw = read_i32_le(payload, cursor + 12);
        let interval_start_hour_raw = read_i32_le(payload, cursor + 16);
        let source_sequence = read_u64_le(payload, cursor + 20);
        let source_row_index = read_u32_le(payload, cursor + 28);
        let reserved1 = read_u32_le(payload, cursor + 32);
        let subject_hash = read_u64_le(payload, cursor + 36);
        let object_hash = read_u64_le(payload, cursor + 44);
        let key_hash = read_u64_le(payload, cursor + 52);
        let from_hash = read_u64_le(payload, cursor + 60);
        let to_hash = read_u64_le(payload, cursor + 68);
        let detail_hash = read_u64_le(payload, cursor + 76);
        let provenance_hash = read_u64_le(payload, cursor + 84);
        let identity_hash = read_u64_le(payload, cursor + 92);
        let generation = read_u32_le(payload, cursor + 100);
        let detail_len = read_u32_le(payload, cursor + 104);
        let reserved2 = read_u32_le(payload, cursor + 108);
        cursor += row_len;

        let event_kind = EventKind::from_raw(event_kind_raw);
        let source_kind = SourceKind::from_raw(source_kind_raw);
        let confidence = Confidence::from_raw(confidence_raw);
        if event_kind.is_none() {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has unknown event kind {event_kind_raw}"
            ));
        }
        if source_kind.is_none() {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has unknown source kind {source_kind_raw}"
            ));
        }
        if confidence.is_none() {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has unknown confidence {confidence_raw}"
            ));
        }
        if source_domain_id == 0
            || source_domain_id == DOMAIN_ID
            || source_domain_id == COMPACT_DOMAIN_FRAME
        {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has invalid source_domain_id {source_domain_id}"
            ));
        }
        if producer_domain_id != DOMAIN_ID {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} producer_domain_id {producer_domain_id} != {DOMAIN_ID}"
            ));
        }
        if flags & !KNOWN_EVENT_FLAGS != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has unknown flags {flags:#x}"
            ));
        }
        if reserved1 != 0 || reserved2 != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} reserved field is nonzero"
            ));
        }
        if identity_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has zero identity_hash"
            ));
        } else if !seen_identity.insert((game_hour_raw, event_kind_raw, identity_hash)) {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} duplicates hour/kind/identity"
            ));
        }
        if provenance_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has zero provenance_hash"
            ));
        }
        if let (Some(event_kind), Some(source_kind), Some(confidence)) =
            (event_kind, source_kind, confidence)
        {
            let decoded = EventRecord {
                event_kind,
                source_domain_id,
                producer_domain_id,
                source_kind,
                confidence,
                flags,
                game_hour_raw,
                interval_start_hour_raw,
                source_sequence,
                source_row_index,
                subject_hash,
                object_hash,
                key_hash,
                from_hash,
                to_hash,
                detail_hash,
                provenance_hash,
                identity_hash,
                generation,
                detail_len,
            };
            let expected_identity = decoded.compute_identity_hash();
            if identity_hash != expected_identity {
                summary.parse_errors.push(format!(
                    "offset {offset}: event row {row_index} identity_hash {identity_hash:#x} != recomputed {expected_identity:#x}"
                ));
            }
            let expected_provenance = decoded.compute_provenance_hash();
            if provenance_hash != expected_provenance {
                summary.parse_errors.push(format!(
                    "offset {offset}: event row {row_index} provenance_hash {provenance_hash:#x} != recomputed {expected_provenance:#x}"
                ));
            }
        }
        if flags & EVENT_FLAG_KEY_PRESENT != 0 && key_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} key-present flag has zero key hash"
            ));
        }
        if flags & EVENT_FLAG_KEY_PRESENT == 0 && key_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has key hash without key-present flag"
            ));
        }
        if flags & EVENT_FLAG_SUBJECT_PRESENT != 0 && subject_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} subject-present flag has zero subject hash"
            ));
        }
        if flags & EVENT_FLAG_SUBJECT_PRESENT == 0 && subject_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has subject hash without subject-present flag"
            ));
        }
        if flags & EVENT_FLAG_OBJECT_PRESENT != 0 && object_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} object-present flag has zero object hash"
            ));
        }
        if flags & EVENT_FLAG_OBJECT_PRESENT == 0 && object_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has object hash without object-present flag"
            ));
        }
        if flags & EVENT_FLAG_FROM_PRESENT != 0 && from_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} from-present flag has zero from hash"
            ));
        }
        if flags & EVENT_FLAG_FROM_PRESENT == 0 && from_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has from hash without from-present flag"
            ));
        }
        if flags & EVENT_FLAG_TO_PRESENT != 0 && to_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} to-present flag has zero to hash"
            ));
        }
        if flags & EVENT_FLAG_TO_PRESENT == 0 && to_hash != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has to hash without to-present flag"
            ));
        }
        if flags & EVENT_FLAG_DETAIL_PRESENT != 0 && (detail_hash == 0 || detail_len == 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} detail-present flag has empty detail proof"
            ));
        }
        if flags & EVENT_FLAG_DETAIL_PRESENT == 0 && (detail_hash != 0 || detail_len != 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has detail proof without detail-present flag"
            ));
        }
        if flags & EVENT_FLAG_DIRECT_CAPTURE != 0 && flags & EVENT_FLAG_DERIVED != 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} is both direct and derived"
            ));
        }
        if source_kind == Some(SourceKind::DirectCaptured) && flags & EVENT_FLAG_DIRECT_CAPTURE == 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} direct source missing direct flag"
            ));
        }
        if source_kind == Some(SourceKind::DerivedDelta) && flags & EVENT_FLAG_DERIVED == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} derived source missing derived flag"
            ));
        }
        if matches!(
            source_kind,
            Some(SourceKind::Marker | SourceKind::SyntheticGap)
        ) && flags & (EVENT_FLAG_DIRECT_CAPTURE | EVENT_FLAG_DERIVED) != 0
        {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} non-direct/non-derived source carries a direct/derived flag"
            ));
        }
        if confidence == Some(Confidence::Interval) && interval_start_hour_raw == i32::MIN {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} interval confidence lacks interval_start_hour_raw"
            ));
        }
        if confidence != Some(Confidence::Interval) && interval_start_hour_raw != i32::MIN {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} non-interval event has interval_start_hour_raw"
            ));
        }
        if source_sequence == 0 && flags & EVENT_FLAG_PROOF_FIXTURE == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} has no source sequence"
            ));
        }

        if flags & EVENT_FLAG_PROOF_FIXTURE != 0 {
            summary.proof_fixture_count = summary.proof_fixture_count.saturating_add(1);
        }
        summary.source_bindings.push(EventSourceBinding {
            event_kind,
            source_domain_id,
            source_sequence,
            source_row_index,
            game_hour_raw,
            flags,
        });

        summary.row_count = summary.row_count.saturating_add(1);
        *summary.event_kind_counts.entry(event_kind_raw).or_default() += 1;
        *summary
            .source_kind_counts
            .entry(source_kind_raw)
            .or_default() += 1;
        *summary.confidence_counts.entry(confidence_raw).or_default() += 1;
        summary.source_domains.insert(source_domain_id);
        identity_xor ^= identity_hash;
        match source_kind {
            Some(SourceKind::DirectCaptured) => {
                summary.direct_count = summary.direct_count.saturating_add(1)
            }
            Some(SourceKind::DerivedDelta) => {
                summary.derived_count = summary.derived_count.saturating_add(1)
            }
            Some(SourceKind::Marker) => {
                summary.marker_count = summary.marker_count.saturating_add(1)
            }
            Some(SourceKind::SyntheticGap) => {
                summary.gap_count = summary.gap_count.saturating_add(1)
            }
            None => {}
        }
        match confidence {
            Some(Confidence::Interval) => {
                summary.interval_count = summary.interval_count.saturating_add(1)
            }
            Some(Confidence::Inferred) => {
                summary.inferred_count = summary.inferred_count.saturating_add(1)
            }
            Some(Confidence::Gap) => summary.gap_count = summary.gap_count.saturating_add(1),
            Some(Confidence::Exact | Confidence::Sampled) | None => {}
        }
        let _ = generation;
        let _ = source_row_index;
    }

    if summary.direct_count != header_direct_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header direct_count {header_direct_count} != decoded {}",
            summary.direct_count
        ));
    }
    if summary.derived_count != header_derived_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header derived_count {header_derived_count} != decoded {}",
            summary.derived_count
        ));
    }
    if summary.marker_count != header_marker_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header marker_count {header_marker_count} != decoded {}",
            summary.marker_count
        ));
    }
    if summary.interval_count != header_interval_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header interval_count {header_interval_count} != decoded {}",
            summary.interval_count
        ));
    }
    if summary.inferred_count != header_inferred_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header inferred_count {header_inferred_count} != decoded {}",
            summary.inferred_count
        ));
    }
    if summary.gap_count != header_gap_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header gap_count {header_gap_count} != decoded {}",
            summary.gap_count
        ));
    }
    if summary.source_domains.len() as u64 != header_source_domain_count {
        summary.parse_errors.push(format!(
            "offset {offset}: header source_domain_count {header_source_domain_count} != decoded {}",
            summary.source_domains.len()
        ));
    }
    let decoded_source_bitmap = summary
        .source_domains
        .iter()
        .filter(|domain_id| **domain_id < 64)
        .fold(0u64, |bitmap, domain_id| bitmap | (1u64 << domain_id));
    if header_source_bitmap != decoded_source_bitmap {
        summary.parse_errors.push(format!(
            "offset {offset}: source_domain_bitmap_low {header_source_bitmap:#018x} != decoded {decoded_source_bitmap:#018x}"
        ));
    }
    if identity_xor != header_identity_xor {
        summary.parse_errors.push(format!(
            "offset {offset}: header identity_hash_xor {header_identity_xor:#x} != decoded {identity_xor:#x}"
        ));
    }
    summary.ok = summary.parse_errors.is_empty();
    summary
}

fn event_kind_accepts_source_domain(event_kind: EventKind, source_domain_id: u16) -> bool {
    match event_kind {
        EventKind::WarDeclared
        | EventKind::Peace
        | EventKind::Capitulated
        | EventKind::FactionJoined
        | EventKind::FactionLeft => source_domain_id == SOURCE_DOMAIN_DIPLOMACY,
        EventKind::FocusCompleted => source_domain_id == SOURCE_DOMAIN_FOCUS,
        EventKind::TechResearched | EventKind::TechLost => source_domain_id == SOURCE_DOMAIN_TECH,
        EventKind::DecisionFired | EventKind::DecisionExpired => {
            source_domain_id == SOURCE_DOMAIN_DECISIONS
        }
        EventKind::CombatStarted | EventKind::CombatUpdated | EventKind::CombatEnded => {
            matches!(
                source_domain_id,
                SOURCE_DOMAIN_LAND_COMBATS | SOURCE_DOMAIN_DIVISIONS
            )
        }
        EventKind::StateOwnerChange
        | EventKind::StateControllerChange
        | EventKind::ProvinceControllerChange => source_domain_id == SOURCE_DOMAIN_MAP_CONTROL,
        EventKind::BuildingCompleted | EventKind::ConstructionCompleted => {
            source_domain_id == SOURCE_DOMAIN_MAP_DETAIL
        }
        EventKind::ShipCreated | EventKind::ShipDeleted => source_domain_id == SOURCE_DOMAIN_SHIPS,
        EventKind::PlaneCreated | EventKind::PlaneDeleted => {
            source_domain_id == SOURCE_DOMAIN_AIR_WINGS
        }
        EventKind::DivisionCreated
        | EventKind::DivisionDestroyed
        | EventKind::DivisionMoved
        | EventKind::DivisionTemplateChanged => matches!(
            source_domain_id,
            SOURCE_DOMAIN_DIVISIONS | SOURCE_DOMAIN_OOB_COMMAND | SOURCE_DOMAIN_ARMY_ORDERS
        ),
        EventKind::ProductionMilestone => source_domain_id == 11,
        EventKind::FactoryMilestone => matches!(source_domain_id, 11 | SOURCE_DOMAIN_MAP_DETAIL),
        EventKind::ProjectCompleted => source_domain_id == SOURCE_DOMAIN_PROJECTS,
        EventKind::LendLeaseStarted | EventKind::LendLeaseEnded => {
            source_domain_id == SOURCE_DOMAIN_MARKET_TRADE
        }
        EventKind::ArmyFormed | EventKind::ArmyDisbanded | EventKind::OobMembershipChanged => {
            source_domain_id == SOURCE_DOMAIN_OOB_COMMAND
        }
        EventKind::OrderChanged => source_domain_id == SOURCE_DOMAIN_ARMY_ORDERS,
    }
}

pub fn append_compact_record(
    out: &mut Vec<u8>,
    sequence: u64,
    game_hour_raw: i32,
    domain_id: u16,
    record_type: u16,
    flags: u32,
    payload: &[u8],
) {
    out.extend_from_slice(COMPACT_RECORD_MAGIC);
    out.extend_from_slice(&COMPACT_RECORD_VERSION.to_le_bytes());
    out.extend_from_slice(&(COMPACT_RECORD_HEADER_LEN as u16).to_le_bytes());
    out.extend_from_slice(&sequence.to_le_bytes());
    out.extend_from_slice(&game_hour_raw.to_le_bytes());
    out.extend_from_slice(&domain_id.to_le_bytes());
    out.extend_from_slice(&record_type.to_le_bytes());
    out.extend_from_slice(&(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(payload);
}

pub fn encode_event_compact_stream(game_hour_raw: i32, rows: &[EventRecord]) -> Vec<u8> {
    let mut out = Vec::new();
    let payload = encode_event_payload(rows);
    append_compact_record(
        &mut out,
        1,
        game_hour_raw,
        DOMAIN_ID,
        COMPACT_RECORD_TYPE_DOMAIN_DATA,
        0,
        &payload,
    );
    append_compact_record(
        &mut out,
        2,
        game_hour_raw,
        COMPACT_DOMAIN_FRAME,
        COMPACT_RECORD_TYPE_FRAME_END,
        0,
        &frame_end_payload(1, 0),
    );
    out
}

pub fn summarize_compact_source_stream(raw: &[u8]) -> CompactSourceSummary {
    let mut summary = CompactSourceSummary {
        total_bytes: raw.len(),
        ..CompactSourceSummary::default()
    };
    if let Err(error) = compact_contract::stream_validator::validate_freeze_census_stream_placement(raw) {
        summary.parse_errors.push(format!("stateful stream placement: {error}"));
    }
    let mut offset = 0usize;
    let mut expected_sequence = 1u64;
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(RecordFramingError::TruncatedHeader(actual)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: truncated header with {actual} byte(s) remaining"
                ));
                break;
            }
            Err(RecordFramingError::BadMagic) => {
                summary
                    .parse_errors
                    .push(format!("offset {offset}: invalid record magic"));
                break;
            }
            Err(RecordFramingError::UnsupportedVersion(version)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported record version {version}"
                ));
                break;
            }
            Err(RecordFramingError::UnsupportedHeaderLen(header_len)) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unsupported record header length {header_len}"
                ));
                break;
            }
            Err(err @ RecordFramingError::PayloadTooLarge(_)) => {
                summary.parse_errors.push(format!("offset {offset}: {err}"));
                break;
            }
        };
        let sequence = header.sequence;
        let game_hour_raw = header.game_hour_raw;
        let domain_id = header.domain_id;
        let record_type = header.record_type;
        let payload_len = header.payload_len;
        let record_len = header.record_len();
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if sequence != expected_sequence {
            summary.ordering_errors.push(format!(
                "offset {offset}: sequence {sequence} != expected {expected_sequence}"
            ));
        }
        match sequence.checked_add(1) {
            Some(next) => expected_sequence = next,
            None => summary.ordering_errors.push(format!(
                "offset {offset}: sequence {sequence} cannot advance without wrap"
            )),
        }
        let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];

        if let Err(err) =
            validate_compact_record_semantics(domain_id, record_type, header.record_flags, payload)
        {
            summary.parse_errors.push(format!("offset {offset}: {err}"));
        }

        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary
            .total_payload_bytes
            .saturating_add(payload_len as u64);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        summary.domains_seen.insert(domain_id);
        let known_record_type = match compact_contract::KnownRecordType::try_from(record_type) {
            Ok(record_type) => record_type,
            Err(unknown) => {
                summary.parse_errors.push(format!(
                    "offset {offset}: unknown record type {}",
                    unknown.0
                ));
                offset += record_len;
                continue;
            }
        };
        match known_record_type {
            compact_contract::KnownRecordType::DomainPerf => {
                summary.domain_perf_record_count =
                    summary.domain_perf_record_count.saturating_add(1);
                *summary
                    .per_domain_perf_record_count
                    .entry(domain_id)
                    .or_default() += 1;
                *summary
                    .per_hour_perf_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
                if payload.len() >= 36 {
                    summary.status_flags_or |= read_u32_le(payload, 32);
                }
            }
            compact_contract::KnownRecordType::DomainData => {
                summary.domain_data_record_count =
                    summary.domain_data_record_count.saturating_add(1);
                *summary
                    .per_domain_data_record_count
                    .entry(domain_id)
                    .or_default() += 1;
                *summary
                    .per_hour_data_domain_counts
                    .entry(game_hour_raw)
                    .or_default()
                    .entry(domain_id)
                    .or_default() += 1;
            }
            compact_contract::KnownRecordType::FrameEnd => {
                summary.frame_end_record_count = summary.frame_end_record_count.saturating_add(1);
                summary.unique_hours.insert(game_hour_raw);
                *summary
                    .frame_end_hour_counts
                    .entry(game_hour_raw)
                    .or_default() += 1;
                if payload.len() >= 28 {
                    summary.status_flags_or |= read_u32_le(payload, 24);
                }
            }
            compact_contract::KnownRecordType::FreezeCensus
            | compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader => {}
            compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {
                summary.parse_errors.push(format!(
                    "offset {offset}: record type {} is outside the event-timeline summary grammar",
                    known_record_type.canonical_name()
                ))
            }
        }
        offset += record_len;
    }
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary
}

/// Validate a production event stream for an explicitly commanded number of
/// game-hour advancements. The exact inclusive closed-frame requirement is
/// therefore `commanded_game_hour_advancements + 1` (for example, 48 advancements require
/// 49 consecutive closed frames).
pub fn gate_event_compact_stream(
    raw: &[u8],
    commanded_game_hour_advancements: i64,
) -> EventCompactGateSummary {
    let source_summary = summarize_compact_source_stream(raw);
    let mut gate = EventCompactGateSummary {
        source_summary,
        ..EventCompactGateSummary::default()
    };
    if raw.is_empty() {
        gate.payload_errors
            .push("compact event stream is empty".to_string());
        return gate;
    }
    if commanded_game_hour_advancements <= 0 {
        gate.payload_errors.push(format!(
            "commanded game-hour advancements must be explicitly positive, got {commanded_game_hour_advancements}"
        ));
    }

    #[derive(Clone, Copy)]
    struct SourceRecord {
        domain_id: u16,
        game_hour_raw: i32,
        emitted_rows: u64,
        status_flags: Option<u32>,
    }

    #[derive(Clone, Copy)]
    struct PerfRecord {
        status_flags: u32,
    }

    let mut source_records = BTreeMap::<u64, SourceRecord>::new();
    let mut source_data_counts = BTreeMap::<u16, u64>::new();
    let mut referenced_source_sequences = BTreeSet::<u64>::new();
    let mut referenced_source_domains = BTreeSet::<u16>::new();
    let mut perf_records = BTreeMap::<u16, Vec<PerfRecord>>::new();
    let mut offset = 0usize;
    let mut open_hour = None;
    let mut last_closed_hour: Option<i32> = None;
    let mut owned_data_this_hour = 0u64;
    let mut owned_perf_this_hour = 0u64;
    let mut owned_data_status_this_hour = None;
    let mut owned_perf_status_this_hour = None;
    let mut observed_perf_this_hour = 0u32;
    let mut last_record_was_frame_end = false;
    let mut release_epilogue_seen = false;
    while offset < raw.len() {
        let header = match parse_compact_record_header(&raw[offset..]) {
            Ok(header) => header,
            Err(_) => break,
        };
        let record_len = header.record_len();
        if record_len > raw.len().saturating_sub(offset) {
            break;
        }
        let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
        let previous_record_closed_world = last_record_was_frame_end;
        last_record_was_frame_end = false;
        let known_record_type = match compact_contract::KnownRecordType::try_from(header.record_type) {
            Ok(record_type) => record_type,
            Err(error) => {
                gate.payload_errors.push(format!("offset {offset}: {error}"));
                offset += record_len;
                continue;
            }
        };

        match known_record_type {
            compact_contract::KnownRecordType::DomainData
            | compact_contract::KnownRecordType::DomainPerf => {
                match open_hour {
                    Some(hour) if hour != header.game_hour_raw => {
                        gate.payload_errors.push(format!(
                            "offset {offset}: record hour {} does not match open frame hour {hour}",
                            header.game_hour_raw
                        ))
                    }
                    None => {
                        if let Some(previous) = last_closed_hour {
                            match previous.checked_add(1) {
                                Some(expected) if header.game_hour_raw != expected => {
                                    gate.payload_errors.push(format!(
                                        "offset {offset}: frame hour {} is not consecutive after {previous}",
                                        header.game_hour_raw
                                    ));
                                }
                                None => gate.payload_errors.push(format!(
                                    "offset {offset}: frame hour cannot advance after {previous}"
                                )),
                                _ => {}
                            }
                        }
                        open_hour = Some(header.game_hour_raw);
                    }
                    Some(_) => {}
                }

                if header.record_type == COMPACT_RECORD_TYPE_DOMAIN_PERF {
                    if header.domain_id != COMPACT_DOMAIN_RC_PROBE {
                        observed_perf_this_hour = observed_perf_this_hour.saturating_add(1);
                    }
                    if payload.len() >= 36 {
                        perf_records
                            .entry(header.domain_id)
                            .or_default()
                            .push(PerfRecord {
                                status_flags: read_u32_le(payload, 32),
                            });
                    }
                    if payload.len() >= 36 {
                        let status_flags = read_u32_le(payload, 32);
                        let unknown = compact_status_unknown_flags_v1(status_flags);
                        if unknown != 0 {
                            gate.payload_errors.push(format!(
                                "offset {offset}: domain {} PERF has unknown status flags {unknown:#010x}",
                                header.domain_id
                            ));
                        }
                        let fatal = compact_status_failure_flags_after_expected_partial_tolerance(
                            header.domain_id,
                            status_flags,
                        );
                        if fatal != 0 {
                            gate.payload_errors.push(format!(
                                "offset {offset}: domain {} PERF has fatal status flags {fatal:#010x}",
                                header.domain_id
                            ));
                        }
                    }
                    if header.domain_id == DOMAIN_ID {
                        owned_perf_this_hour = owned_perf_this_hour.saturating_add(1);
                        if payload.len() >= 36 {
                            owned_perf_status_this_hour = Some(read_u32_le(payload, 32));
                        }
                    }
                } else if header.domain_id == DOMAIN_ID {
                    owned_data_this_hour = owned_data_this_hour.saturating_add(1);
                    let payload_summary = decode_event_payload(offset, payload);
                    owned_data_status_this_hour = Some(payload_summary.status_flags);
                    gate.event_payloads = gate.event_payloads.saturating_add(1);
                    gate.event_rows_total = gate
                        .event_rows_total
                        .saturating_add(payload_summary.row_count);
                    for (kind, count) in payload_summary.event_kind_counts {
                        *gate.event_kind_counts.entry(kind).or_default() += count;
                    }
                    gate.source_domains
                        .extend(payload_summary.source_domains.iter().copied());
                    if payload_summary.gap_count != 0 {
                        gate.payload_errors.push(format!(
                            "offset {offset}: production event payload contains {} synthetic/gap row badge(s)",
                            payload_summary.gap_count
                        ));
                    }
                    for binding in payload_summary.source_bindings {
                        referenced_source_sequences.insert(binding.source_sequence);
                        referenced_source_domains.insert(binding.source_domain_id);
                        if binding.game_hour_raw != header.game_hour_raw {
                            gate.payload_errors.push(format!(
                                "offset {offset}: event source sequence {} row {} hour {} does not match event envelope hour {}",
                                binding.source_sequence,
                                binding.source_row_index,
                                binding.game_hour_raw,
                                header.game_hour_raw
                            ));
                        }
                        if binding.flags & EVENT_FLAG_PROOF_FIXTURE != 0 {
                            gate.payload_errors.push(format!(
                                "offset {offset}: event source sequence {} row {} is marked as a proof fixture",
                                binding.source_sequence, binding.source_row_index
                            ));
                        }
                        if binding.event_kind.is_some_and(|event_kind| {
                            !event_kind_accepts_source_domain(event_kind, binding.source_domain_id)
                        }) {
                            let event_kind = binding.event_kind.expect("checked Some");
                            gate.payload_errors.push(format!(
                                "offset {offset}: event kind {} is incompatible with source domain {} ({})",
                                event_kind.name(),
                                binding.source_domain_id,
                                source_domain_name(binding.source_domain_id)
                            ));
                        }
                        match source_records.get(&binding.source_sequence) {
                            Some(source)
                                if source.domain_id == binding.source_domain_id
                                    && source.game_hour_raw == binding.game_hour_raw => {
                                if u64::from(binding.source_row_index) >= source.emitted_rows {
                                    gate.payload_errors.push(format!(
                                        "offset {offset}: event source sequence {} row {} is outside canonical source DATA row count {}",
                                        binding.source_sequence,
                                        binding.source_row_index,
                                        source.emitted_rows
                                    ));
                                }
                            }
                            Some(source) => gate.payload_errors.push(format!(
                                "offset {offset}: event source sequence {} claims domain {}/hour {}, actual source DATA is domain {}/hour {}",
                                binding.source_sequence,
                                binding.source_domain_id,
                                binding.game_hour_raw,
                                source.domain_id,
                                source.game_hour_raw
                            )),
                            None => gate.payload_errors.push(format!(
                                "offset {offset}: event source sequence {} row {} does not identify an earlier source DOMAIN_DATA record",
                                binding.source_sequence, binding.source_row_index
                            )),
                        }
                    }
                    gate.payload_errors.extend(payload_summary.parse_errors);
                } else {
                    *source_data_counts.entry(header.domain_id).or_default() += 1;
                    let canonical_fold = recorder_host::stream_fold::validate_domain_data_payload(
                        header.domain_id,
                        payload,
                        header.record_flags,
                    );
                    if let Err(reason) = &canonical_fold {
                        gate.payload_errors.push(format!(
                            "offset {offset}: source domain {} DATA failed canonical fold decode: {reason}",
                            header.domain_id
                        ));
                    }
                    let authority = summarize_domain_data_payload(
                        header.domain_id,
                        payload,
                        DomainDataContext {
                            stream_offset: offset,
                            record_flags: header.record_flags,
                            sequence: header.sequence,
                            header_hour_raw: header.game_hour_raw,
                            session_header_identity: None,
                        },
                    );
                    match (canonical_fold, authority) {
                        (Ok(()), Ok(authority)) => {
                            source_records.insert(
                                header.sequence,
                                SourceRecord {
                                    domain_id: header.domain_id,
                                    game_hour_raw: header.game_hour_raw,
                                    emitted_rows: authority.emitted_rows,
                                    status_flags: authority.status_flags,
                                },
                            );
                        }
                        (_, Err(error)) => gate.payload_errors.push(format!(
                            "offset {offset}: source domain {} DATA failed canonical authority decode: {}",
                            header.domain_id,
                            error.errors.join("; ")
                        )),
                        (Err(_), Ok(_)) => {}
                    }
                }
            }
            compact_contract::KnownRecordType::FrameEnd => {
                match open_hour {
                    Some(hour) if hour != header.game_hour_raw => {
                        gate.payload_errors.push(format!(
                        "offset {offset}: FRAME_END hour {} does not match open frame hour {hour}",
                        header.game_hour_raw
                    ))
                    }
                    None => gate.payload_errors.push(format!(
                        "offset {offset}: FRAME_END hour {} closes no open frame",
                        header.game_hour_raw
                    )),
                    Some(_) => {}
                }
                if owned_data_this_hour != 1 {
                    gate.payload_errors.push(format!(
                        "offset {offset}: closed hour {} has {} event DOMAIN_DATA records, expected exactly 1",
                        header.game_hour_raw, owned_data_this_hour
                    ));
                }
                if owned_perf_this_hour != 1 {
                    gate.payload_errors.push(format!(
                        "offset {offset}: closed hour {} has {} event DOMAIN_PERF records, expected exactly 1",
                        header.game_hour_raw, owned_perf_this_hour
                    ));
                }
                if owned_data_this_hour == 1
                    && owned_perf_this_hour == 1
                    && owned_data_status_this_hour != owned_perf_status_this_hour
                {
                    gate.payload_errors.push(format!(
                        "offset {offset}: closed hour {} event DATA status {:?} does not match PERF status {:?}",
                        header.game_hour_raw,
                        owned_data_status_this_hour.map(|value| format!("{value:#010x}")),
                        owned_perf_status_this_hour.map(|value| format!("{value:#010x}"))
                    ));
                }
                for source_sequence in &referenced_source_sequences {
                    let Some(source) = source_records.get(source_sequence) else {
                        continue;
                    };
                    let matching_perf = perf_records
                        .get(&source.domain_id)
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    if matching_perf.len() != 1 {
                        gate.payload_errors.push(format!(
                            "offset {offset}: source DATA sequence {source_sequence} domain {} hour {} has {} matching PERF records, expected exactly 1",
                            source.domain_id,
                            source.game_hour_raw,
                            matching_perf.len()
                        ));
                    } else if let Some(data_status) = source.status_flags {
                        if matching_perf[0].status_flags != data_status {
                            gate.payload_errors.push(format!(
                                "offset {offset}: source DATA sequence {source_sequence} domain {} status {data_status:#010x} does not match PERF status {:#010x}",
                                source.domain_id,
                                matching_perf[0].status_flags
                            ));
                        }
                    }
                }
                for source_domain in &referenced_source_domains {
                    let source_data_count =
                        source_data_counts.get(source_domain).copied().unwrap_or(0);
                    if source_data_count != 1 {
                        gate.payload_errors.push(format!(
                            "offset {offset}: referenced source domain {source_domain} hour {} has {source_data_count} DOMAIN_DATA records, expected exactly 1",
                            header.game_hour_raw
                        ));
                    }
                }
                match validate_frame_end_authority(payload) {
                    Ok(emitted_domains) => {
                        if emitted_domains != observed_perf_this_hour {
                            gate.payload_errors.push(format!(
                                "offset {offset}: FRAME_END emitted domain count {emitted_domains} does not match independently observed DOMAIN_PERF count {observed_perf_this_hour}"
                            ));
                        }
                        let required_authority_domains =
                            u32::try_from(referenced_source_domains.len().saturating_add(1))
                                .unwrap_or(u32::MAX);
                        if emitted_domains < required_authority_domains {
                            gate.payload_errors.push(format!(
                                "offset {offset}: FRAME_END emitted domain count {emitted_domains} cannot account for event domain plus {} referenced source domain(s) (minimum {required_authority_domains})",
                                referenced_source_domains.len()
                            ));
                        }
                    }
                    Err(reason) => gate
                        .payload_errors
                        .push(format!("offset {offset}: {reason}")),
                }
                if open_hour == Some(header.game_hour_raw) {
                    last_closed_hour = Some(header.game_hour_raw);
                }
                open_hour = None;
                owned_data_this_hour = 0;
                owned_perf_this_hour = 0;
                owned_data_status_this_hour = None;
                owned_perf_status_this_hour = None;
                observed_perf_this_hour = 0;
                source_records.clear();
                source_data_counts.clear();
                referenced_source_sequences.clear();
                referenced_source_domains.clear();
                perf_records.clear();
                last_record_was_frame_end = true;
            }
            compact_contract::KnownRecordType::FreezeCensus => {
                if let Ok(census) = compact_contract::freeze_census_record::FreezeCensusRecordV1::decode(payload) {
                    last_record_was_frame_end = compact_contract::freeze_census_record::release_epilogue_closes_stream_policy(previous_record_closed_world, census.phase(), release_epilogue_seen);
                    release_epilogue_seen |= census.phase() == compact_contract::freeze_census_record::FreezeCensusPhase::Release;
                }
            }
            compact_contract::KnownRecordType::ContentIdentityCensus
            | compact_contract::KnownRecordType::StreamHeader
            | compact_contract::KnownRecordType::DomainGap
            | compact_contract::KnownRecordType::FrameRefused
            | compact_contract::KnownRecordType::FrameDegraded
            | compact_contract::KnownRecordType::CaptureTerminated
            | compact_contract::KnownRecordType::ClockUnreadableInterval
            | compact_contract::KnownRecordType::UntrustedClockRecovery => {}
        }
        offset += record_len;
    }
    if open_hour.is_some() {
        gate.payload_errors
            .push("compact event stream ends with an unclosed frame".to_string());
    }
    if !last_record_was_frame_end {
        gate.payload_errors
            .push("compact event stream does not end at a canonical FRAME_END".to_string());
    }
    let observed_advancements = match (
        gate.source_summary.first_hour,
        gate.source_summary.last_hour,
    ) {
        (Some(first), Some(last)) => i64::from(last) - i64::from(first),
        _ => -1,
    };
    let observed_closed_frames =
        i64::try_from(gate.source_summary.unique_hours.len()).unwrap_or(i64::MAX);
    let required_closed_frames = commanded_game_hour_advancements
        .checked_add(1)
        .unwrap_or(i64::MAX);
    if observed_advancements != commanded_game_hour_advancements
        || observed_closed_frames != required_closed_frames
        || i64::try_from(gate.source_summary.frame_end_record_count).unwrap_or(i64::MAX)
            != required_closed_frames
    {
        gate.payload_errors.push(format!(
            "production proof duration is {observed_advancements} advancement(s) across {observed_closed_frames} unique / {} total closed frame(s), expected exactly {commanded_game_hour_advancements} advancement(s) / {required_closed_frames} closed frame(s)",
            gate.source_summary.frame_end_record_count
        ));
    }
    gate.ok = gate.source_summary.parse_errors.is_empty()
        && gate.source_summary.ordering_errors.is_empty()
        && gate.payload_errors.is_empty()
        && gate.event_payloads > 0
        && gate.event_rows_total > 0
        && gate.source_summary.frame_end_record_count > 0
        && gate.source_summary.skipped_hours.is_empty();
    gate
}

fn validate_frame_end_authority(payload: &[u8]) -> Result<u32, String> {
    if payload.len() != 32 {
        return Err(format!(
            "FRAME_END payload length {} is not 32",
            payload.len()
        ));
    }
    let emitted_domains = read_u32_le(payload, 16);
    let scheduled_domains = read_u32_le(payload, 20);
    let status_flags = read_u32_le(payload, 24);
    let reserved = read_u32_le(payload, 28);
    if reserved != 0 {
        return Err(format!(
            "FRAME_END reserved word is nonzero ({reserved:#010x})"
        ));
    }
    if emitted_domains != scheduled_domains {
        return Err(format!(
            "FRAME_END emitted domain count {emitted_domains} does not match scheduled domain count {scheduled_domains}"
        ));
    }
    let unknown = compact_status_unknown_flags_v1(status_flags);
    if unknown != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain unknown bits {unknown:#010x}"
        ));
    }
    let fatal = status_flags & COMPACT_STATUS_FAILURE_MASK;
    if fatal != 0 {
        return Err(format!(
            "FRAME_END status flags {status_flags:#010x} contain fatal bits {fatal:#010x}"
        ));
    }
    Ok(emitted_domains)
}

pub fn source_domain_name(domain_id: u16) -> &'static str {
    match domain_id {
        10 => "fuel_live",
        11 => "production_live",
        12 => "focus_live",
        13 => "tech_live",
        14 => "politics_live",
        15 => "politics_residuals",
        16 => "decisions_live",
        17 => "diplomacy_live",
        20 => "divisions",
        21 => "land_combats",
        22 => "map_control",
        23 => "map_detail",
        24 => "weather_live",
        26 => "division_equipment",
        27 => "army_orders",
        28 => "oob_command",
        29 => "ships",
        30 => "air_wings",
        41 => "projects_live",
        43 => "market_trade_live",
        44 => "supply_logistics_attrition_live",
        249 => "event_timeline_change_log_live_lane_local_proof",
        1000 => "frame",
        _ => "unknown",
    }
}

pub fn stable_hash(value: &str) -> u64 {
    stable_hash_bytes(value.as_bytes())
}

pub fn stable_hash_bytes(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    if hash == 0 {
        1
    } else {
        hash
    }
}

pub fn frame_end_payload(frame_time_us: u64, status_flags: u32) -> Vec<u8> {
    frame_end_payload_with_counts(frame_time_us, status_flags, 1, 1)
}

pub fn frame_end_payload_with_counts(
    frame_time_us: u64,
    status_flags: u32,
    emitted_domains: u32,
    scheduled_domains: u32,
) -> Vec<u8> {
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&frame_time_us.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&emitted_domains.to_le_bytes());
    payload.extend_from_slice(&scheduled_domains.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn skipped_hours(hours: &BTreeSet<i32>) -> Vec<i32> {
    let mut skipped = Vec::new();
    let mut prev: Option<i32> = None;
    for hour in hours {
        if let Some(previous) = prev {
            let mut missing = previous.checked_add(1);
            while let Some(value) = missing {
                if value >= *hour {
                    break;
                }
                skipped.push(value);
                missing = value.checked_add(1);
            }
        }
        prev = Some(*hour);
    }
    skipped
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

#[cfg(test)]
mod tests {
    use super::*;

    fn perf_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = vec![0u8; 40];
        payload[32..36].copy_from_slice(&status_flags.to_le_bytes());
        payload
    }

    fn append_compact_string(payload: &mut Vec<u8>, value: &str) {
        payload.extend_from_slice(&(value.len() as u16).to_le_bytes());
        payload.extend_from_slice(value.as_bytes());
    }

    fn focus_data_payload(status_flags: u32) -> Vec<u8> {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u64.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&status_flags.to_le_bytes());
        payload.extend_from_slice(&100_000u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&47u32.to_le_bytes());
        payload.extend_from_slice(&0b11_0111u32.to_le_bytes());
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&1_350_000i32.to_le_bytes());
        append_compact_string(&mut payload, "portugal_focus");
        append_compact_string(&mut payload, "POR_industrial_focus");
        append_compact_string(&mut payload, "");
        payload.extend_from_slice(&47u32.to_le_bytes());
        append_compact_string(&mut payload, "POR_completed_focus");
        payload
    }

    fn event_row(
        hour: i32,
        source_sequence: u64,
        source_domain_id: u16,
        proof_fixture: bool,
    ) -> EventRecord {
        let mut row = EventRecord::new(
            EventKind::FocusCompleted,
            source_domain_id,
            SourceKind::DerivedDelta,
            Confidence::Exact,
            hour,
        )
        .with_keys(Some("GER"), None, Some("focus"), None, Some("completed"))
        .with_source_position(source_sequence, 0);
        row.flags |= EVENT_FLAG_DERIVED;
        if proof_fixture {
            row = row.mark_proof_fixture();
        }
        row
    }

    fn append_valid_hour(
        stream: &mut Vec<u8>,
        sequence: &mut u64,
        hour: i32,
        proof_fixture: bool,
        perf_status: u32,
        include_source_perf: bool,
    ) {
        let source_sequence = *sequence;
        append_compact_record(
            stream,
            *sequence,
            hour,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        *sequence += 1;
        if include_source_perf {
            append_compact_record(
                stream,
                *sequence,
                hour,
                SOURCE_DOMAIN_FOCUS,
                COMPACT_RECORD_TYPE_DOMAIN_PERF,
                0,
                &perf_payload(0),
            );
            *sequence += 1;
        }
        append_compact_record(
            stream,
            *sequence,
            hour,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[event_row(
                hour,
                source_sequence,
                SOURCE_DOMAIN_FOCUS,
                proof_fixture,
            )]),
        );
        *sequence += 1;
        append_compact_record(
            stream,
            *sequence,
            hour,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(perf_status),
        );
        *sequence += 1;
        append_compact_record(
            stream,
            *sequence,
            hour,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload_with_counts(1, 0, 2, 2),
        );
        *sequence += 1;
    }

    fn valid_stream(hours: &[i32]) -> Vec<u8> {
        let mut stream = Vec::new();
        let mut sequence = 1;
        for hour in hours {
            append_valid_hour(&mut stream, &mut sequence, *hour, false, 0, true);
        }
        stream
    }

    #[test]
    fn event_payload_roundtrips_direct_and_derived_rows() {
        let mut focus = EventRecord::new(
            EventKind::FocusCompleted,
            SOURCE_DOMAIN_FOCUS,
            SourceKind::DerivedDelta,
            Confidence::Exact,
            123,
        )
        .with_keys(
            Some("GER"),
            None,
            Some("GER_four_year_plan"),
            None,
            Some("completed"),
        )
        .with_source_position(10, 0)
        .mark_proof_fixture();
        focus.flags |= EVENT_FLAG_DERIVED;
        let mut combat = EventRecord::new(
            EventKind::CombatStarted,
            SOURCE_DOMAIN_LAND_COMBATS,
            SourceKind::DirectCaptured,
            Confidence::Inferred,
            123,
        )
        .with_keys(Some("GER"), Some("POL"), Some("combat:42:1"), None, None)
        .with_source_position(11, 4)
        .with_generation(1)
        .with_detail("land_combat_started pointer_free")
        .mark_proof_fixture();
        combat.flags |= EVENT_FLAG_DIRECT_CAPTURE;

        let payload = encode_event_payload(&[focus, combat]);
        let summary = decode_event_payload(32, &payload);
        assert!(summary.ok, "{:?}", summary.parse_errors);
        assert_eq!(summary.row_count, 2);
        assert_eq!(summary.direct_count, 1);
        assert_eq!(summary.derived_count, 1);
        assert_eq!(summary.inferred_count, 1);
        assert!(summary.source_domains.contains(&SOURCE_DOMAIN_FOCUS));
        assert!(summary.source_domains.contains(&SOURCE_DOMAIN_LAND_COMBATS));
    }

    #[test]
    fn event_payload_rejects_bad_identity_and_flags() {
        let mut row = EventRecord::new(
            EventKind::ProvinceControllerChange,
            SOURCE_DOMAIN_MAP_CONTROL,
            SourceKind::DerivedDelta,
            Confidence::Exact,
            200,
        )
        .with_keys(
            Some("GER"),
            None,
            Some("province:1"),
            Some("POL"),
            Some("GER"),
        );
        row.flags |= EVENT_FLAG_DERIVED | (1 << 29);
        row.identity_hash = 0;
        let payload = encode_event_payload(&[row]);
        let summary = decode_event_payload(0, &payload);
        assert!(!summary.ok);
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("unknown flags")));
        assert!(summary
            .parse_errors
            .iter()
            .any(|err| err.contains("zero identity_hash")));
    }

    #[test]
    fn event_payload_rejects_noncanonical_shape_without_panicking() {
        let row = event_row(200, 1, SOURCE_DOMAIN_FOCUS, false);
        let canonical = encode_event_payload(&[row]);

        let mut short_row = canonical.clone();
        short_row[4..6].copy_from_slice(&1u16.to_le_bytes());
        let summary = decode_event_payload(0, &short_row);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unsupported event row length 1")));

        let mut short_header = canonical;
        short_header[2..4].copy_from_slice(&1u16.to_le_bytes());
        let summary = decode_event_payload(0, &short_header);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("unsupported event payload header length 1")));
    }

    #[test]
    fn event_payload_requires_exact_bitmap_and_presence_flags() {
        let row = event_row(200, 1, SOURCE_DOMAIN_FOCUS, false);
        let canonical = encode_event_payload(&[row]);

        let mut phantom_bitmap = canonical.clone();
        let bitmap = read_u64_le(&phantom_bitmap, 40) | (1u64 << 1);
        phantom_bitmap[40..48].copy_from_slice(&bitmap.to_le_bytes());
        let summary = decode_event_payload(0, &phantom_bitmap);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("source_domain_bitmap_low") && error.contains("decoded")));

        let mut missing_presence = canonical;
        let flags_offset = EVENT_PAYLOAD_HEADER_LEN + 8;
        let flags = read_u32_le(&missing_presence, flags_offset) & !EVENT_FLAG_KEY_PRESENT;
        missing_presence[flags_offset..flags_offset + 4].copy_from_slice(&flags.to_le_bytes());
        let summary = decode_event_payload(0, &missing_presence);
        assert!(summary
            .parse_errors
            .iter()
            .any(|error| error.contains("key hash without key-present flag")));
    }

    #[test]
    fn compact_event_stream_gate_binds_rows_to_supplied_source() {
        let stream = valid_stream(&[500, 501]);
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(gate.ok, "{:?}", gate);
        assert_eq!(gate.event_payloads, 2);
        assert_eq!(gate.event_rows_total, 2);
        assert_eq!(gate.source_summary.frame_end_record_count, 2);
        assert_eq!(gate.source_summary.skipped_hours.len(), 0);
    }

    #[test]
    fn production_gate_requires_canonical_source_data_decode() {
        let mut stream = valid_stream(&[500, 501]);
        // d12 scale_raw lives at payload offset 20. The H4CR envelope remains
        // canonical, so only the production domain decoder can reject this.
        let scale_offset = COMPACT_RECORD_HEADER_LEN + 20;
        stream[scale_offset..scale_offset + 4].copy_from_slice(&42u32.to_le_bytes());

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("failed canonical")));
    }

    #[test]
    fn production_gate_requires_matching_source_perf_authority() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_valid_hour(&mut stream, &mut sequence, 500, false, 0, false);
        append_valid_hour(&mut stream, &mut sequence, 501, false, 0, true);

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("0 matching PERF records")));
    }

    #[test]
    fn production_gate_requires_exactly_one_referenced_source_data_record() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        let source_sequence = sequence;
        for _ in 0..2 {
            append_compact_record(
                &mut stream,
                sequence,
                500,
                SOURCE_DOMAIN_FOCUS,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &focus_data_payload(0),
            );
            sequence += 1;
        }
        append_compact_record(
            &mut stream,
            sequence,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[event_row(500, source_sequence, SOURCE_DOMAIN_FOCUS, false)]),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload_with_counts(1, 0, 2, 2),
        );
        sequence += 1;
        append_valid_hour(&mut stream, &mut sequence, 501, false, 0, true);

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate.payload_errors.iter().any(|error| {
            error.contains("referenced source domain 12")
                && error.contains("2 DOMAIN_DATA records, expected exactly 1")
        }));
    }

    #[test]
    fn production_gate_requires_matching_source_data_perf_status() {
        let mut stream = valid_stream(&[500, 501]);
        let source_status_offset = COMPACT_RECORD_HEADER_LEN + 16;
        stream[source_status_offset..source_status_offset + 4].copy_from_slice(
            &compact_contract::COMPACT_STATUS_TRANSITION_BASELINE_RESET.to_le_bytes(),
        );

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("does not match PERF status")));
    }

    #[test]
    fn production_gate_binds_source_row_index_to_decoded_row_count() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        append_compact_record(
            &mut stream,
            2,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        let forged_row = event_row(500, 1, SOURCE_DOMAIN_FOCUS, false).with_source_position(1, 1);
        append_compact_record(
            &mut stream,
            3,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[forged_row]),
        );
        append_compact_record(
            &mut stream,
            4,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        append_compact_record(
            &mut stream,
            5,
            500,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload_with_counts(1, 0, 2, 2),
        );
        let mut sequence = 6;
        append_valid_hour(&mut stream, &mut sequence, 501, false, 0, true);

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("outside canonical source DATA row count 1")));
    }

    #[test]
    fn production_gate_duration_is_advancements_plus_one_closed_frames() {
        let exact_hours = (1_000..=1_048).collect::<Vec<_>>();
        let exact = gate_event_compact_stream(&valid_stream(&exact_hours), 48);
        assert!(exact.ok, "{:?}", exact.payload_errors);
        assert_eq!(exact.source_summary.unique_hours.len(), 49);

        let short_hours = (1_000..=1_047).collect::<Vec<_>>();
        let short = gate_event_compact_stream(&valid_stream(&short_hours), 48);
        assert!(!short.ok);
        assert!(short
            .payload_errors
            .iter()
            .any(|error| error.contains("47 advancement(s) across 48 unique / 48 total")));

        let overlong_hours = (1_000..=1_049).collect::<Vec<_>>();
        let overlong = gate_event_compact_stream(&valid_stream(&overlong_hours), 48);
        assert!(!overlong.ok);
        assert!(overlong
            .payload_errors
            .iter()
            .any(|error| error.contains("49 advancement(s) across 50 unique / 50 total")));
    }

    #[test]
    fn production_gate_rejects_non_positive_duration_contract() {
        let gate = gate_event_compact_stream(&valid_stream(&[500, 501]), 0);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("must be explicitly positive")));
    }

    #[test]
    fn production_gate_rejects_non_authoritative_frame_end_fields() {
        let base = valid_stream(&[500, 501]);
        let payload_start = base.len() - 32;

        let mut mismatched_counts = base.clone();
        mismatched_counts[payload_start + 16..payload_start + 20]
            .copy_from_slice(&1u32.to_le_bytes());
        let gate = gate_event_compact_stream(&mismatched_counts, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("does not match scheduled domain count")));

        let mut forged_matching_counts = base.clone();
        forged_matching_counts[payload_start + 16..payload_start + 20]
            .copy_from_slice(&3u32.to_le_bytes());
        forged_matching_counts[payload_start + 20..payload_start + 24]
            .copy_from_slice(&3u32.to_le_bytes());
        let gate = gate_event_compact_stream(&forged_matching_counts, 1);
        assert!(!gate.ok);
        assert!(gate.payload_errors.iter().any(|error| {
            error.contains("emitted domain count 3")
                && error.contains("independently observed DOMAIN_PERF count 2")
        }));

        let mut reserved = base.clone();
        reserved[payload_start + 28..payload_start + 32].copy_from_slice(&7u32.to_le_bytes());
        let gate = gate_event_compact_stream(&reserved, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("reserved word is nonzero")));

        let mut unknown_status = base;
        unknown_status[payload_start + 24..payload_start + 28]
            .copy_from_slice(&(1u32 << 31).to_le_bytes());
        let gate = gate_event_compact_stream(&unknown_status, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|error| error.contains("unknown bits")));
    }

    #[test]
    fn production_gate_rejects_proof_fixture_rows() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_valid_hour(&mut stream, &mut sequence, 500, true, 0, true);
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("marked as a proof fixture")));
    }

    #[test]
    fn production_gate_rejects_synthetic_gap_rows() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        append_compact_record(
            &mut stream,
            2,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        let row = EventRecord::new(
            EventKind::FocusCompleted,
            SOURCE_DOMAIN_FOCUS,
            SourceKind::SyntheticGap,
            Confidence::Gap,
            500,
        )
        .with_keys(Some("GER"), None, Some("focus"), None, None)
        .with_source_position(1, 0);
        append_compact_record(
            &mut stream,
            3,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[row]),
        );
        append_compact_record(
            &mut stream,
            4,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        append_compact_record(
            &mut stream,
            5,
            500,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("synthetic/gap")));
    }

    #[test]
    fn production_gate_rejects_forged_source_identity_and_hour() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        append_compact_record(
            &mut stream,
            2,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        let row = event_row(501, 1, SOURCE_DOMAIN_TECH, false);
        append_compact_record(
            &mut stream,
            3,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[row]),
        );
        append_compact_record(
            &mut stream,
            4,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        append_compact_record(
            &mut stream,
            5,
            500,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("does not match event envelope hour")));
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("actual source DATA is domain")));
    }

    #[test]
    fn production_gate_rejects_event_kind_source_domain_mismatch() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        let source_sequence = sequence;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        sequence += 1;
        let mut row = EventRecord::new(
            EventKind::TechResearched,
            SOURCE_DOMAIN_FOCUS,
            SourceKind::DerivedDelta,
            Confidence::Exact,
            500,
        )
        .with_keys(Some("GER"), None, Some("tech"), None, Some("researched"))
        .with_source_position(source_sequence, 0);
        row.flags |= EVENT_FLAG_DERIVED;
        row.identity_hash = row.compute_identity_hash();
        row.provenance_hash = row.compute_provenance_hash();
        append_compact_record(
            &mut stream,
            sequence,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[row]),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut stream,
            sequence,
            500,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload_with_counts(1, 0, 2, 2),
        );
        sequence += 1;
        append_valid_hour(&mut stream, &mut sequence, 501, false, 0, true);

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate.payload_errors.iter().any(|error| {
            error.contains("event kind tech_researched is incompatible with source domain 12")
        }));
    }

    #[test]
    fn production_gate_binds_event_data_status_to_owned_perf() {
        let mut stream = valid_stream(&[500, 501]);
        let event_payload_status_offset = COMPACT_RECORD_HEADER_LEN
            + focus_data_payload(0).len()
            + COMPACT_RECORD_HEADER_LEN
            + perf_payload(0).len()
            + COMPACT_RECORD_HEADER_LEN
            + 56;
        stream[event_payload_status_offset..event_payload_status_offset + 4]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_DELTA_DATA.to_le_bytes());

        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate.payload_errors.iter().any(|error| {
            error.contains("event DATA status") && error.contains("does not match PERF status")
        }));
    }

    #[test]
    fn production_gate_rejects_empty_unrelated_gapped_unclosed_and_trailing_inputs() {
        assert!(!gate_event_compact_stream(&[], 1).ok);

        let mut unrelated = Vec::new();
        append_compact_record(
            &mut unrelated,
            1,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[1],
        );
        append_compact_record(
            &mut unrelated,
            2,
            10,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        assert!(!gate_event_compact_stream(&unrelated, 1).ok);

        assert!(!gate_event_compact_stream(&valid_stream(&[10, 12]), 1).ok);

        let mut unclosed = valid_stream(&[10]);
        unclosed.truncate(unclosed.len() - (COMPACT_RECORD_HEADER_LEN + 32));
        assert!(!gate_event_compact_stream(&unclosed, 1).ok);

        let mut trailing = valid_stream(&[10]);
        trailing.push(0xAA);
        assert!(!gate_event_compact_stream(&trailing, 1).ok);
    }

    #[test]
    fn production_gate_requires_exact_sequence_cardinality_and_canonical_envelopes() {
        let mut bad_first_sequence = valid_stream(&[10]);
        bad_first_sequence[8..16].copy_from_slice(&2u64.to_le_bytes());
        assert!(!gate_event_compact_stream(&bad_first_sequence, 1).ok);

        let mut wrapping_sequence = valid_stream(&[10]);
        wrapping_sequence[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        let gate = gate_event_compact_stream(&wrapping_sequence, 1);
        assert!(!gate.ok);
        assert!(gate
            .source_summary
            .ordering_errors
            .iter()
            .any(|err| err.contains("cannot advance without wrap")));

        let mut missing_perf = Vec::new();
        let mut sequence = 1;
        let source_sequence = sequence;
        append_compact_record(
            &mut missing_perf,
            sequence,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut missing_perf,
            sequence,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        sequence += 1;
        append_compact_record(
            &mut missing_perf,
            sequence,
            10,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &encode_event_payload(&[event_row(10, source_sequence, SOURCE_DOMAIN_FOCUS, false)]),
        );
        sequence += 1;
        append_compact_record(
            &mut missing_perf,
            sequence,
            10,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        let gate = gate_event_compact_stream(&missing_perf, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("0 event DOMAIN_PERF")));

        let mut bad_flags = valid_stream(&[10]);
        bad_flags[28..32].copy_from_slice(&1u32.to_le_bytes());
        assert!(!gate_event_compact_stream(&bad_flags, 1).ok);
    }

    #[test]
    fn production_gate_rejects_duplicate_owned_data() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &focus_data_payload(0),
        );
        append_compact_record(
            &mut stream,
            2,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        let payload = encode_event_payload(&[event_row(10, 1, SOURCE_DOMAIN_FOCUS, false)]);
        for sequence in [3, 4] {
            append_compact_record(
                &mut stream,
                sequence,
                10,
                DOMAIN_ID,
                COMPACT_RECORD_TYPE_DOMAIN_DATA,
                0,
                &payload,
            );
        }
        append_compact_record(
            &mut stream,
            5,
            10,
            DOMAIN_ID,
            COMPACT_RECORD_TYPE_DOMAIN_PERF,
            0,
            &perf_payload(0),
        );
        append_compact_record(
            &mut stream,
            6,
            10,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("2 event DOMAIN_DATA")));
    }

    #[test]
    fn production_gate_rejects_lane_fatal_status() {
        let mut stream = Vec::new();
        let mut sequence = 1;
        append_valid_hour(
            &mut stream,
            &mut sequence,
            500,
            false,
            compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
            true,
        );
        let gate = gate_event_compact_stream(&stream, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("PERF has fatal status")));

        let mut bad_frame = valid_stream(&[500]);
        let frame_payload_start = bad_frame.len() - 32;
        bad_frame[frame_payload_start + 24..frame_payload_start + 28]
            .copy_from_slice(&compact_contract::COMPACT_STATUS_WORKER_PANIC.to_le_bytes());
        let gate = gate_event_compact_stream(&bad_frame, 1);
        assert!(!gate.ok);
        assert!(gate
            .payload_errors
            .iter()
            .any(|err| err.contains("FRAME_END status flags") && err.contains("fatal bits")));
    }

    #[test]
    fn source_summary_detects_missing_event_family_domains() {
        let mut stream = Vec::new();
        append_compact_record(
            &mut stream,
            1,
            10,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[0, 1, 2, 3],
        );
        append_compact_record(
            &mut stream,
            2,
            10,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );
        append_compact_record(
            &mut stream,
            3,
            12,
            SOURCE_DOMAIN_FOCUS,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &[4, 5, 6, 7],
        );
        append_compact_record(
            &mut stream,
            4,
            12,
            COMPACT_DOMAIN_FRAME,
            COMPACT_RECORD_TYPE_FRAME_END,
            0,
            &frame_end_payload(1, 0),
        );

        let summary = summarize_compact_source_stream(&stream);
        assert_eq!(summary.skipped_hours, vec![11]);
        let gates = gate_event_families(&summary);
        let focus = gates
            .iter()
            .find(|gate| gate.family == "focus_completed")
            .unwrap();
        assert!(focus.supported);
        let ships = gates
            .iter()
            .find(|gate| gate.family == "ship_lifecycle")
            .unwrap();
        assert!(!ships.supported);
        assert_eq!(ships.missing_domain_ids, vec![SOURCE_DOMAIN_SHIPS]);
    }

    #[test]
    fn missing_source_adapter_gate_reports_absent_domains() {
        let gates = gate_missing_source_adapters(&BTreeMap::new());
        assert_eq!(gates.len(), 4);
        let ships = gates
            .iter()
            .find(|gate| gate.family == "ship_lifecycle")
            .unwrap();
        assert!(!ships.ready);
        assert!(ships
            .blockers
            .iter()
            .any(|blocker| blocker.contains("missing compact source domain 29")));
    }

    #[test]
    fn market_adapter_requires_complete_contract_fields() {
        let mut units = BTreeMap::new();
        units.insert("market_trade_contract_rows".to_string(), 5);
        units.insert("market_trade_observed_contract_rows".to_string(), 5);
        units.insert("market_trade_buyer_ptr_valid_rows".to_string(), 5);
        units.insert("market_trade_variant_valid_rows".to_string(), 5);
        units.insert("market_trade_amount_valid_rows".to_string(), 5);
        units.insert("market_trade_partial_field_rows".to_string(), 0);

        let mut domains = BTreeMap::new();
        domains.insert(
            SOURCE_DOMAIN_MARKET_TRADE,
            SourceDomainUnitSnapshot {
                domain_id: SOURCE_DOMAIN_MARKET_TRADE,
                data_record_count: 1,
                parse_error_count: 0,
                units,
            },
        );
        let gates = gate_missing_source_adapters(&domains);
        let market = gates
            .iter()
            .find(|gate| gate.family == "market_lend_lease_lifecycle")
            .unwrap();
        assert!(market.ready, "{:?}", market.blockers);

        domains
            .get_mut(&SOURCE_DOMAIN_MARKET_TRADE)
            .unwrap()
            .units
            .insert("market_trade_partial_field_rows".to_string(), 1);
        let gates = gate_missing_source_adapters(&domains);
        let market = gates
            .iter()
            .find(|gate| gate.family == "market_lend_lease_lifecycle")
            .unwrap();
        assert!(!market.ready);
        assert!(market
            .blockers
            .iter()
            .any(|blocker| blocker.contains("expected zero")));
    }
}
