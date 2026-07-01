use std::collections::{BTreeMap, BTreeSet};

pub const COMPACT_RECORD_MAGIC: &[u8; 4] = b"H4CR";
pub const COMPACT_RECORD_VERSION: u16 = 1;
pub const COMPACT_RECORD_HEADER_LEN: usize = 32;
pub const COMPACT_RECORD_TYPE_DOMAIN_PERF: u16 = 1;
pub const COMPACT_RECORD_TYPE_FRAME_END: u16 = 2;
pub const COMPACT_RECORD_TYPE_DOMAIN_DATA: u16 = 3;
pub const COMPACT_DOMAIN_FRAME: u16 = 1000;

pub const DOMAIN_ID: u16 = 44;
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
    pub source_domains: BTreeSet<u16>,
    pub event_kind_counts: BTreeMap<u16, u64>,
    pub source_kind_counts: BTreeMap<u8, u64>,
    pub confidence_counts: BTreeMap<u8, u64>,
    pub parse_errors: Vec<String>,
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
    let reserved0 = read_u64_le(payload, 56);

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
        if flags & EVENT_FLAG_KEY_PRESENT != 0 && key_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} key-present flag has zero key hash"
            ));
        }
        if flags & EVENT_FLAG_SUBJECT_PRESENT != 0 && subject_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} subject-present flag has zero subject hash"
            ));
        }
        if flags & EVENT_FLAG_OBJECT_PRESENT != 0 && object_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} object-present flag has zero object hash"
            ));
        }
        if flags & EVENT_FLAG_FROM_PRESENT != 0 && from_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} from-present flag has zero from hash"
            ));
        }
        if flags & EVENT_FLAG_TO_PRESENT != 0 && to_hash == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} to-present flag has zero to hash"
            ));
        }
        if flags & EVENT_FLAG_DETAIL_PRESENT != 0 && (detail_hash == 0 || detail_len == 0) {
            summary.parse_errors.push(format!(
                "offset {offset}: event row {row_index} detail-present flag has empty detail proof"
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
    for domain_id in &summary.source_domains {
        if *domain_id < 64 && header_source_bitmap & (1u64 << domain_id) == 0 {
            summary.parse_errors.push(format!(
                "offset {offset}: source_domain_bitmap_low missing domain {domain_id}"
            ));
        }
    }
    if identity_xor != header_identity_xor {
        summary.parse_errors.push(format!(
            "offset {offset}: header identity_hash_xor {header_identity_xor:#x} != decoded {identity_xor:#x}"
        ));
    }
    summary.ok = summary.parse_errors.is_empty();
    summary
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
    let mut offset = 0usize;
    let mut previous_sequence = None;
    while offset < raw.len() {
        let remaining = raw.len().saturating_sub(offset);
        if remaining < COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: truncated header with {remaining} byte(s) remaining"
            ));
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            summary
                .parse_errors
                .push(format!("offset {offset}: invalid record magic"));
            break;
        }
        let version = read_u16_le(header, 4);
        let header_len = read_u16_le(header, 6) as usize;
        let sequence = read_u64_le(header, 8);
        let game_hour_raw = read_i32_le(header, 16);
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if version != COMPACT_RECORD_VERSION {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported record version {version}"
            ));
            break;
        }
        if header_len != COMPACT_RECORD_HEADER_LEN {
            summary.parse_errors.push(format!(
                "offset {offset}: unsupported record header length {header_len}"
            ));
            break;
        }
        if record_len > remaining {
            summary.parse_errors.push(format!(
                "offset {offset}: payload length {payload_len} exceeds remaining stream bytes"
            ));
            break;
        }
        if let Some(previous) = previous_sequence {
            if sequence <= previous {
                summary.ordering_errors.push(format!(
                    "offset {offset}: sequence {sequence} did not advance past {previous}"
                ));
            }
        }
        previous_sequence = Some(sequence);
        let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];

        summary.record_count = summary.record_count.saturating_add(1);
        summary.total_payload_bytes = summary
            .total_payload_bytes
            .saturating_add(payload_len as u64);
        summary.max_payload_len = summary.max_payload_len.max(payload_len);
        summary.domains_seen.insert(domain_id);
        match record_type {
            COMPACT_RECORD_TYPE_DOMAIN_PERF => {
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
            COMPACT_RECORD_TYPE_DOMAIN_DATA => {
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
            COMPACT_RECORD_TYPE_FRAME_END => {
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
            other => summary.parse_errors.push(format!(
                "offset {offset}: unsupported compact record type {other}"
            )),
        }
        offset += record_len;
    }
    summary.first_hour = summary.unique_hours.first().copied();
    summary.last_hour = summary.unique_hours.last().copied();
    summary.skipped_hours = skipped_hours(&summary.unique_hours);
    summary
}

pub fn gate_event_compact_stream(raw: &[u8]) -> EventCompactGateSummary {
    let source_summary = summarize_compact_source_stream(raw);
    let mut gate = EventCompactGateSummary {
        source_summary,
        ..EventCompactGateSummary::default()
    };
    let mut offset = 0usize;
    while offset < raw.len() {
        if raw.len().saturating_sub(offset) < COMPACT_RECORD_HEADER_LEN {
            break;
        }
        let header = &raw[offset..offset + COMPACT_RECORD_HEADER_LEN];
        if &header[0..4] != COMPACT_RECORD_MAGIC {
            break;
        }
        let payload_len = read_u32_le(header, 24) as usize;
        let record_len = COMPACT_RECORD_HEADER_LEN.saturating_add(payload_len);
        if record_len > raw.len().saturating_sub(offset) {
            break;
        }
        let domain_id = read_u16_le(header, 20);
        let record_type = read_u16_le(header, 22);
        if domain_id == DOMAIN_ID && record_type == COMPACT_RECORD_TYPE_DOMAIN_DATA {
            let payload = &raw[offset + COMPACT_RECORD_HEADER_LEN..offset + record_len];
            let payload_summary = decode_event_payload(offset, payload);
            gate.event_payloads = gate.event_payloads.saturating_add(1);
            gate.event_rows_total = gate
                .event_rows_total
                .saturating_add(payload_summary.row_count);
            for (kind, count) in payload_summary.event_kind_counts {
                *gate.event_kind_counts.entry(kind).or_default() += count;
            }
            gate.source_domains.extend(payload_summary.source_domains);
            gate.payload_errors.extend(payload_summary.parse_errors);
        }
        offset += record_len;
    }
    gate.ok = gate.source_summary.parse_errors.is_empty()
        && gate.source_summary.ordering_errors.is_empty()
        && gate.payload_errors.is_empty()
        && gate.event_payloads > 0
        && gate.event_rows_total > 0
        && gate.source_summary.frame_end_record_count > 0;
    gate
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
        44 => "event_timeline_change_log_live",
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
    let mut payload = Vec::with_capacity(32);
    payload.extend_from_slice(&frame_time_us.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&0u64.to_le_bytes());
    payload.extend_from_slice(&status_flags.to_le_bytes());
    payload.extend_from_slice(&0u32.to_le_bytes());
    payload
}

fn skipped_hours(hours: &BTreeSet<i32>) -> Vec<i32> {
    let mut skipped = Vec::new();
    let mut prev = None;
    for hour in hours {
        if let Some(previous) = prev {
            let mut missing = previous + 1;
            while missing < *hour {
                skipped.push(missing);
                missing += 1;
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
    fn compact_event_stream_gate_decodes_event_domain() {
        let mut row = EventRecord::new(
            EventKind::TechResearched,
            SOURCE_DOMAIN_TECH,
            SourceKind::DerivedDelta,
            Confidence::Exact,
            500,
        )
        .with_keys(
            Some("USA"),
            None,
            Some("basic_machine_tools"),
            None,
            Some("1"),
        )
        .with_source_position(99, 2)
        .mark_proof_fixture();
        row.flags |= EVENT_FLAG_DERIVED;
        let stream = encode_event_compact_stream(500, &[row]);
        let gate = gate_event_compact_stream(&stream);
        assert!(gate.ok, "{:?}", gate);
        assert_eq!(gate.event_payloads, 1);
        assert_eq!(gate.event_rows_total, 1);
        assert_eq!(gate.source_summary.frame_end_record_count, 1);
        assert_eq!(gate.source_summary.skipped_hours.len(), 0);
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
