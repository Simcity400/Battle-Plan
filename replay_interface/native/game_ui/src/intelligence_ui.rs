//! Legacy controller-bound intelligence diagnostics.
//!
//! Static GUI and definition files identify chrome and catalog entries, while
//! save/domain-53 data supplies gameplay rows. The selected client controller
//! and its XUI03 dynamic-collection trace are retained only as authenticated
//! diagnostics: they are recorder-POV presentation state and are not replay
//! render authority. Shipping XUI14 uses the passive all-country presentation
//! carrier and `intelligence_presentation`; this module cannot promote, demote
//! or substitute for that product route.

use crate::dynamic_collection::DynamicCollectionTrace;
use crate::{ErrorKind, GameUiError, Result};
use replay_core::agency_upgrade_controller::{
    admit_agency_upgrade_controller_refresh, AgencyUpgradeControllerRefreshExact,
};
use replay_core::cipher_controller::{
    admit_cipher_controller_refresh, CipherControllerRefreshExact,
};
use replay_core::intelligence_controller_companion::{
    admit_intelligence_controller_companion, IntelligenceControllerCompanionExact,
    IntelligenceSurfaceId, IntelligenceVisibilityExact,
};
use replay_core::{Leaf, SelectedPresentReceipt, ViewStatus, WorldFrame};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const FIXED_ONE: i64 = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntelligenceTraceAuthority {
    ExactA729ExtractorController,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntelligenceSurface {
    Agency,
    Upgrades,
    Operatives,
    Operations,
    Ciphers,
    IntelLedger,
    Networks,
}

impl IntelligenceSurface {
    pub const ALL: [Self; 7] = [
        Self::Agency,
        Self::Upgrades,
        Self::Operatives,
        Self::Operations,
        Self::Ciphers,
        Self::IntelLedger,
        Self::Networks,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VisibilityDisposition {
    Visible,
    Hidden,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibilityGate {
    pub surface: IntelligenceSurface,
    pub disposition: VisibilityDisposition,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProducerValue<T> {
    Measured(T),
    Unobtained { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MeasuredList<T> {
    /// Every producer row, including rows filtered out by the controller.
    pub rows: Vec<T>,
    pub collection: DynamicCollectionTrace,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyState {
    pub agency_instance_id: String,
    pub owner_country: String,
    pub name_text: String,
    pub icon_sprite: String,
    pub created: bool,
    pub upgrade_progress_fixed: i64,
    pub current_upgrade_id: Option<String>,
    pub max_operatives: u32,
    pub usable_slots: u32,
    pub defense_raw: i32,
    /// Exact denominator/scale attached to `defense_raw` by the producer.
    /// Domain-53 currently preserves the numerator only, so A30 cannot admit
    /// that raw sidecar as semantic agency defense until this leaf is proven.
    pub defense_denominator_raw: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyUpgradeRow {
    pub stable_item_id: String,
    pub upgrade_id: String,
    pub branch_id: String,
    pub level: u32,
    pub icon_sprite: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperativeAssignment {
    Idle,
    Mission {
        mission_id: String,
        mission_action_id: String,
        target_country: Option<String>,
        target_country_tag_db_id: u32,
        target_state: Option<u32>,
        authored_subject_id: Option<String>,
    },
    Operation {
        operation_id: String,
    },
    Network {
        network_id: String,
    },
    Captured {
        captured_by: String,
    },
    Recruitable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperativeUiRow {
    pub stable_item_id: String,
    pub operative_id: String,
    pub owner_country: String,
    pub name_text: String,
    pub portrait_sprite: String,
    pub skill: u32,
    pub experience_fixed: i64,
    pub traits: Vec<String>,
    pub nationalities: Vec<String>,
    /// Capture is an independent lifecycle leaf. It must not be inferred from
    /// the currently selected assignment presentation.
    pub captured: bool,
    pub captured_by: Option<String>,
    /// Exact country database identity from `COperativeLeader+0xFB0`.
    pub captured_by_tag_db_id: Option<u32>,
    /// Exact raw capture-end date from `COperativeLeader+0xFC0`.
    pub capture_end_date_raw: Option<i64>,
    /// Membership in the recruitable pool is likewise independent producer
    /// state, even though the visible assignment row has a Recruitable branch.
    pub recruitable_pool_member: bool,
    pub assignment: OperativeAssignment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperationTarget {
    Country(String),
    State(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationStatus {
    Preparing,
    Prepared,
    Running,
    Finished,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationUiRow {
    pub stable_item_id: String,
    pub operation_id: String,
    pub definition_id: String,
    pub owner_country: String,
    pub target: OperationTarget,
    pub status: OperationStatus,
    pub phase_index: u32,
    pub phase_count: u32,
    pub progress_fixed: i64,
    pub start_hour: i64,
    pub duration_hours: u32,
    pub assigned_operative_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CipherUiRow {
    pub stable_item_id: String,
    pub owner_country: String,
    pub target_country: String,
    pub decryption_points_raw: i64,
    /// Exact producer denominator for `decryption_points_raw`. A normalized
    /// percentage without its denominator is not an authenticated cipher law.
    pub progress_denominator_raw: i64,
    pub progress_fixed: i64,
    pub decrypting: bool,
    pub cipher_broken: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelLevelUiRow {
    pub stable_item_id: String,
    pub viewer_country: String,
    pub target_country: String,
    pub civilian_fixed: i64,
    pub army_fixed: i64,
    pub navy_fixed: i64,
    pub air_fixed: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkStateStrengthUiRow {
    pub state_id: u32,
    pub strength_fixed: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkUiRow {
    pub stable_item_id: String,
    pub network_id: String,
    pub owner_country: String,
    pub target_country: String,
    pub operative_ids: Vec<String>,
    pub state_ids: Vec<u32>,
    /// Exact state-qualified INGR v4 strengths. There is no authenticated
    /// single network-level strength scalar in A11.
    pub state_strengths: Vec<IntelNetworkStateStrengthUiRow>,
    pub national_coverage_fixed: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetworkSummaryOrderAuthority {
    /// Product-authored order by
    /// `(owner_country, target_country, network_id, stable_item_id)`.
    ProductStableContentKeyV1,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelNetworkSummary {
    pub order_authority: NetworkSummaryOrderAuthority,
    pub rows: Vec<IntelNetworkUiRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelligenceUiTrace {
    pub authority: IntelligenceTraceAuthority,
    pub frame_hour: u64,
    /// Stable identity of the same-tick extractor/controller record. Frame
    /// hour alone is not sufficient provenance when records can be retried or
    /// multiple POV/owner pairs are emitted in one hour.
    pub producer_record_identity: String,
    pub pov_country: String,
    pub owner_country: String,
    pub agency: ProducerValue<AgencyState>,
    pub upgrades: ProducerValue<MeasuredList<AgencyUpgradeRow>>,
    pub operatives: ProducerValue<MeasuredList<OperativeUiRow>>,
    pub operations: ProducerValue<MeasuredList<OperationUiRow>>,
    pub ciphers: ProducerValue<MeasuredList<CipherUiRow>>,
    pub intel_levels: ProducerValue<MeasuredList<IntelLevelUiRow>>,
    /// Exact engine-derived network values presented as a replay-product
    /// summary. There is no corresponding a729 authored list/controller order.
    pub networks: ProducerValue<IntelNetworkSummary>,
    pub visibility: Vec<VisibilityGate>,
    /// Foreign-owner rows excluded while deriving this POV-scoped authority.
    /// Keeping the audit on the admitted trace prevents a successful
    /// composition from silently discarding evidence of the filter.
    pub pov_scope_drops: Vec<PovScopedDrop>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum IntelligenceComponent {
    Agency,
    Upgrades,
    Operatives,
    Operations,
    Ciphers,
    IntelLevels,
    Networks,
}

impl IntelligenceComponent {
    pub const ALL: [Self; 7] = [
        Self::Agency,
        Self::Upgrades,
        Self::Operatives,
        Self::Operations,
        Self::Ciphers,
        Self::IntelLevels,
        Self::Networks,
    ];

    pub fn field_name(self) -> &'static str {
        match self {
            Self::Agency => "agency",
            Self::Upgrades => "upgrades",
            Self::Operatives => "operatives",
            Self::Operations => "operations",
            Self::Ciphers => "ciphers",
            Self::IntelLevels => "intel_levels",
            Self::Networks => "networks",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PovScopedDrop {
    pub component: IntelligenceComponent,
    pub row_identity: String,
    pub recorded_owner: Option<String>,
    pub pov_country: String,
}

/// The seven independently assembled semantic components retained when the
/// whole XUI14 trace cannot pass its selected-Present admission gate.
///
/// This is deliberately not an `IntelligenceUiTrace`: rejected components
/// cannot be mistaken for render authority. It exists so absence remains
/// machine-readable per component and per leaf.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelligenceComponentAssembly {
    pub agency: ProducerValue<AgencyState>,
    pub upgrades: ProducerValue<MeasuredList<AgencyUpgradeRow>>,
    pub operatives: ProducerValue<MeasuredList<OperativeUiRow>>,
    pub operations: ProducerValue<MeasuredList<OperationUiRow>>,
    pub ciphers: ProducerValue<MeasuredList<CipherUiRow>>,
    pub intel_levels: ProducerValue<MeasuredList<IntelLevelUiRow>>,
    pub networks: ProducerValue<IntelNetworkSummary>,
    /// Exact seven-surface companion visibility retained for rejection
    /// diagnostics. It is copied, never recomputed from replay-local state.
    pub visibility: Vec<VisibilityGate>,
    pub pov_scope_drops: Vec<PovScopedDrop>,
}

impl IntelligenceComponentAssembly {
    pub fn value_is_measured(&self, component: IntelligenceComponent) -> bool {
        match component {
            IntelligenceComponent::Agency => matches!(self.agency, ProducerValue::Measured(_)),
            IntelligenceComponent::Upgrades => {
                matches!(self.upgrades, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Operatives => {
                matches!(self.operatives, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Operations => {
                matches!(self.operations, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Ciphers => {
                matches!(self.ciphers, ProducerValue::Measured(_))
            }
            IntelligenceComponent::IntelLevels => {
                matches!(self.intel_levels, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Networks => {
                matches!(self.networks, ProducerValue::Measured(_))
            }
        }
    }

    pub fn unobtained_reason(&self, component: IntelligenceComponent) -> Option<&str> {
        match component {
            IntelligenceComponent::Agency => producer_value_reason(&self.agency),
            IntelligenceComponent::Upgrades => producer_value_reason(&self.upgrades),
            IntelligenceComponent::Operatives => producer_value_reason(&self.operatives),
            IntelligenceComponent::Operations => producer_value_reason(&self.operations),
            IntelligenceComponent::Ciphers => producer_value_reason(&self.ciphers),
            IntelligenceComponent::IntelLevels => producer_value_reason(&self.intel_levels),
            IntelligenceComponent::Networks => producer_value_reason(&self.networks),
        }
    }

    fn from_trace(trace: IntelligenceUiTrace) -> Self {
        Self {
            agency: trace.agency,
            upgrades: trace.upgrades,
            operatives: trace.operatives,
            operations: trace.operations,
            ciphers: trace.ciphers,
            intel_levels: trace.intel_levels,
            networks: trace.networks,
            visibility: trace.visibility,
            pov_scope_drops: trace.pov_scope_drops,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntelligenceAdapterRejection {
    PovCountryEmpty,
    CompanionAbsent,
    CompanionUnobtained,
    SnapshotPayloadIdentityAbsent,
    SnapshotPayloadIdentityUnobtained,
    SnapshotPayloadIdentityMismatch {
        snapshot_payload_sha256: String,
        producer_record_identity: String,
    },
    ProductionRenderAbsent,
    CompanionOwnerMismatch {
        companion_owner: String,
        pov_country: String,
    },
    CompanionAdmissionRejected {
        reason: String,
    },
    ComponentAdmissionRejected {
        reason: String,
        assembly: Box<IntelligenceComponentAssembly>,
    },
}

impl IntelligenceAdapterRejection {
    pub fn component_assembly(&self) -> Option<&IntelligenceComponentAssembly> {
        match self {
            Self::ComponentAdmissionRejected { assembly, .. } => Some(assembly),
            _ => None,
        }
    }
}

impl fmt::Display for IntelligenceAdapterRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PovCountryEmpty => write!(formatter, "pov_tag is empty"),
            Self::CompanionAbsent => write!(
                formatter,
                "DiplomacyView.intelligence_controller_companion is Leaf::Absent"
            ),
            Self::CompanionUnobtained => write!(
                formatter,
                "DiplomacyView.intelligence_controller_companion is Leaf::Unobtained"
            ),
            Self::SnapshotPayloadIdentityAbsent => write!(
                formatter,
                "DiplomacyView.domain53_snapshot_payload_sha256 is Leaf::Absent"
            ),
            Self::SnapshotPayloadIdentityUnobtained => write!(
                formatter,
                "DiplomacyView.domain53_snapshot_payload_sha256 is Leaf::Unobtained"
            ),
            Self::SnapshotPayloadIdentityMismatch {
                snapshot_payload_sha256,
                producer_record_identity,
            } => write!(
                formatter,
                "DiplomacyView.domain53_snapshot_payload_sha256 ({snapshot_payload_sha256}) does not match IntelligenceControllerCompanionExact.producer_record_identity ({producer_record_identity})"
            ),
            Self::ProductionRenderAbsent => {
                write!(formatter, "WorldFrame.production_render is absent")
            }
            Self::CompanionOwnerMismatch {
                companion_owner,
                pov_country,
            } => write!(
                formatter,
                "IntelligenceControllerCompanionExact.owner ({companion_owner}) does not match pov_tag ({pov_country})"
            ),
            Self::CompanionAdmissionRejected { reason } => {
                write!(formatter, "selected-Present companion rejected: {reason}")
            }
            Self::ComponentAdmissionRejected { reason, assembly } => {
                write!(formatter, "{reason}")?;
                for component in IntelligenceComponent::ALL {
                    if let Some(component_reason) = assembly.unobtained_reason(component) {
                        write!(
                            formatter,
                            "; {}: {component_reason}",
                            component.field_name()
                        )?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for IntelligenceAdapterRejection {}

const AGENCY_UPGRADES_COLLECTION_ID: &str =
    "interface/countryintelligenceagencyview.gui#collection-0008";

/// Build XUI14 semantic input from the exact current `WorldFrame`.
///
/// Every successful return is immediately checked by the selected-Present
/// admitter. A component-level failure retains the seven typed producer values
/// in `IntelligenceComponentAssembly`; it never returns a partial trace.
pub fn intelligence_trace_from_world(
    frame: &WorldFrame,
    pov_tag: &str,
) -> std::result::Result<IntelligenceUiTrace, IntelligenceAdapterRejection> {
    if pov_tag.trim().is_empty() {
        return Err(IntelligenceAdapterRejection::PovCountryEmpty);
    }

    let exact_sources = frame.intelligence_ui_exact_sources();
    let companion = match exact_sources.controller_companion {
        Leaf::Measured(companion) => companion,
        Leaf::Absent => return Err(IntelligenceAdapterRejection::CompanionAbsent),
        Leaf::Unobtained => return Err(IntelligenceAdapterRejection::CompanionUnobtained),
    };
    match exact_sources.domain53_snapshot_payload_sha256 {
        Leaf::Absent => return Err(IntelligenceAdapterRejection::SnapshotPayloadIdentityAbsent),
        Leaf::Unobtained => {
            return Err(IntelligenceAdapterRejection::SnapshotPayloadIdentityUnobtained)
        }
        Leaf::Measured(snapshot_payload_sha256)
            if exact_sources
                .snapshot_bound_controller_companion()
                .is_none() =>
        {
            return Err(
                IntelligenceAdapterRejection::SnapshotPayloadIdentityMismatch {
                    snapshot_payload_sha256: snapshot_payload_sha256.clone(),
                    producer_record_identity: companion.producer_record_identity.clone(),
                },
            )
        }
        Leaf::Measured(_) => {}
    }

    let render = frame
        .production_render()
        .ok_or(IntelligenceAdapterRejection::ProductionRenderAbsent)?;
    if companion.owner != pov_tag {
        return Err(IntelligenceAdapterRejection::CompanionOwnerMismatch {
            companion_owner: companion.owner.clone(),
            pov_country: pov_tag.to_string(),
        });
    }
    admit_intelligence_controller_companion(companion, &render.selected_present).map_err(
        |error| IntelligenceAdapterRejection::CompanionAdmissionRejected {
            reason: error.message,
        },
    )?;

    let frame_hour = companion.frame_hour;
    let mut pov_scope_drops = Vec::new();
    let agency = build_agency(frame, pov_tag);
    let upgrades = build_upgrades(frame, pov_tag, frame_hour, &mut pov_scope_drops);
    let operatives = build_operatives(frame, pov_tag);
    let operations = build_operations(frame, pov_tag, &mut pov_scope_drops);
    let ciphers = build_ciphers(frame, pov_tag, frame_hour, &mut pov_scope_drops);
    let intel_levels = build_intel_levels(frame, pov_tag, &mut pov_scope_drops);
    let networks = build_networks(frame, pov_tag, &mut pov_scope_drops);
    let visibility = companion
        .visibility
        .iter()
        .map(|gate| VisibilityGate {
            surface: map_surface(gate.surface),
            disposition: map_visibility(gate.disposition),
            reason: gate.reason.clone(),
        })
        .collect::<Vec<_>>();

    let trace = IntelligenceUiTrace {
        authority: IntelligenceTraceAuthority::ExactA729ExtractorController,
        frame_hour,
        producer_record_identity: companion.producer_record_identity.clone(),
        pov_country: pov_tag.to_string(),
        owner_country: companion.owner.clone(),
        agency,
        upgrades,
        operatives,
        operations,
        ciphers,
        intel_levels,
        networks,
        visibility,
        pov_scope_drops,
    };

    if let Some(reason) = availability_gate_mismatch(&trace) {
        return Err(IntelligenceAdapterRejection::ComponentAdmissionRejected {
            reason,
            assembly: Box::new(IntelligenceComponentAssembly::from_trace(trace)),
        });
    }

    let admission = admit_intelligence_ui_with_selected_present_companion(
        &trace,
        companion,
        &render.selected_present,
    )
    .map(|_| ());
    match admission {
        Ok(()) => Ok(trace),
        Err(error) => Err(IntelligenceAdapterRejection::ComponentAdmissionRejected {
            reason: format!(
                "assembled IntelligenceUiTrace failed selected-Present admission: {}",
                error.message
            ),
            assembly: Box::new(IntelligenceComponentAssembly::from_trace(trace)),
        }),
    }
}

fn build_agency(frame: &WorldFrame, pov_tag: &str) -> ProducerValue<AgencyState> {
    let intel_status = frame.intel(pov_tag).map_or_else(
        || "WorldFrame.intel(pov_tag) is absent".to_string(),
        |intel| {
            format!(
                "IntelView.agency_name={}, IntelView.agency_created={}, IntelView.agency_upgrade_progress={}",
                leaf_state(&intel.agency_name),
                leaf_state(&intel.agency_created),
                leaf_state(&intel.agency_upgrade_progress),
            )
        },
    );
    ProducerValue::Unobtained {
        reason: format!(
            "missing AgencyState.agency_instance_id, AgencyState.icon_sprite, AgencyState.current_upgrade_id, AgencyState.max_operatives, AgencyState.usable_slots, AgencyState.defense_raw, AgencyState.defense_denominator_raw [C-RAW]; {intel_status}; Domain53AgencyScalarsRaw and Domain53AgencyCapacityRaw are deliberately unresolved and are not semantic inputs"
        ),
    }
}

fn build_upgrades(
    frame: &WorldFrame,
    pov_tag: &str,
    frame_hour: u64,
    pov_scope_drops: &mut Vec<PovScopedDrop>,
) -> ProducerValue<MeasuredList<AgencyUpgradeRow>> {
    let refresh = match &frame.diplomacy().agency_upgrade_controller_refresh {
        Leaf::Absent => {
            return unobtained(
                "DiplomacyView.agency_upgrade_controller_refresh is Leaf::Absent [C-DORMANT]; missing AgencyUpgradeRow.level and AgencyUpgradeRow.icon_sprite",
            )
        }
        Leaf::Unobtained => {
            return unobtained(
                "DiplomacyView.agency_upgrade_controller_refresh is Leaf::Unobtained [C-DORMANT]; missing AgencyUpgradeRow.level and AgencyUpgradeRow.icon_sprite",
            )
        }
        Leaf::Measured(refresh) => refresh,
    };
    if refresh.owner != pov_tag {
        pov_scope_drops.push(PovScopedDrop {
            component: IntelligenceComponent::Upgrades,
            row_identity: refresh.producer_record_identity.clone(),
            recorded_owner: Some(refresh.owner.clone()),
            pov_country: pov_tag.to_string(),
        });
        return unobtained(
            "AgencyUpgradeControllerRefreshExact.owner does not match pov_tag; all foreign-owner upgrade rows were dropped",
        );
    }
    let admitted = match admit_agency_upgrade_controller_refresh(refresh, frame_hour) {
        Ok(admitted) => admitted,
        Err(error) => return unobtained(error.message),
    };
    let ordered = admitted.ordered_upgrades();
    if !ordered.is_empty() {
        return unobtained(
            "missing AgencyUpgradeRow.level and AgencyUpgradeRow.icon_sprite [C-DORMANT row content]",
        );
    }
    let collection = match exact_collection(frame, AGENCY_UPGRADES_COLLECTION_ID, frame_hour) {
        Ok(collection) => collection,
        Err(reason) => return unobtained(reason),
    };
    if !collection.source_items.is_empty() || !collection.ordered_item_ids.is_empty() {
        return unobtained(
            "AgencyUpgradeControllerRefreshExact is measured-empty but DynamicCollectionTrace source_items/ordered_item_ids are nonempty",
        );
    }
    ProducerValue::Measured(MeasuredList {
        rows: Vec::new(),
        collection,
    })
}

fn build_operatives(
    frame: &WorldFrame,
    pov_tag: &str,
) -> ProducerValue<MeasuredList<OperativeUiRow>> {
    let intel_status = frame.intel(pov_tag).map_or_else(
        || "WorldFrame.intel(pov_tag) is absent".to_string(),
        |intel| {
            format!(
                "IntelView.operatives={}, IntelView.domain53_operative_lifecycle_raw={}",
                leaf_state(&intel.operatives),
                leaf_state(&intel.domain53_operative_lifecycle_raw),
            )
        },
    );
    unobtained(format!(
        "missing OperativeUiRow.name_text, OperativeUiRow.traits, OperativeUiRow.nationalities [C-DROP], OperativeUiRow.experience_fixed denominator, and replay product-stable list-order authority [C-ORDER]; {intel_status}; IntelView.operatives is source S (save/oracle, development-offline only) and is not an acceptable shipped-live runtime source; runtime name/traits/nationalities must be carried by the whole-world per-country domain-53 producer, whose current production fold drops them"
    ))
}

fn build_operations(
    frame: &WorldFrame,
    pov_tag: &str,
    pov_scope_drops: &mut Vec<PovScopedDrop>,
) -> ProducerValue<MeasuredList<OperationUiRow>> {
    let rows = match &frame.diplomacy().operation_instances {
        Leaf::Absent => {
            return unobtained(
                "DiplomacyView.operation_instances is Leaf::Absent; missing MeasuredList.collection DynamicCollectionTrace [C-XUI03]",
            )
        }
        Leaf::Unobtained => {
            return unobtained(
                "DiplomacyView.operation_instances is Leaf::Unobtained; missing MeasuredList.collection DynamicCollectionTrace [C-XUI03]",
            )
        }
        Leaf::Measured(rows) => rows,
    };
    let mut semantic_issues = Vec::new();
    for row in rows {
        if row.owner_country != pov_tag {
            pov_scope_drops.push(PovScopedDrop {
                component: IntelligenceComponent::Operations,
                row_identity: format!("operation:{}", row.stable_operation_id),
                recorded_owner: Some(row.owner_country.clone()),
                pov_country: pov_tag.to_string(),
            });
            continue;
        }
        if scaled_fixed(
            row.progress_fixed,
            row.progress_scale,
            "OperationRuntimeRow.progress_fixed/OperationRuntimeRow.progress_scale",
        )
        .is_err()
        {
            semantic_issues.push(format!(
                "OperationRuntimeRow[{}].progress_fixed/progress_scale",
                row.stable_operation_id
            ));
        }
    }
    let measured_state = if rows.is_empty() {
        "DiplomacyView.operation_instances is Leaf::Measured(vec![])"
    } else {
        "DiplomacyView.operation_instances is Leaf::Measured"
    };
    let issues = if semantic_issues.is_empty() {
        String::new()
    } else {
        format!("; invalid leaves: {}", semantic_issues.join(", "))
    };
    unobtained(format!(
        "{measured_state}; missing MeasuredList.collection DynamicCollectionTrace for OperationUiRow [C-XUI03]{issues}"
    ))
}

fn build_ciphers(
    frame: &WorldFrame,
    pov_tag: &str,
    frame_hour: u64,
    pov_scope_drops: &mut Vec<PovScopedDrop>,
) -> ProducerValue<MeasuredList<CipherUiRow>> {
    let controller = match &frame.diplomacy().cipher_controller_refresh {
        Leaf::Absent => {
            return unobtained(
                "DiplomacyView.cipher_controller_refresh is Leaf::Absent [C-DORMANT]; missing CipherUiRow.progress_denominator_raw, CipherUiRow.progress_fixed, CipherUiRow.cipher_broken, and MeasuredList.collection DynamicCollectionTrace [C-RAW/C-XUI03]",
            )
        }
        Leaf::Unobtained => {
            return unobtained(
                "DiplomacyView.cipher_controller_refresh is Leaf::Unobtained [C-DORMANT]; missing CipherUiRow.progress_denominator_raw, CipherUiRow.progress_fixed, CipherUiRow.cipher_broken, and MeasuredList.collection DynamicCollectionTrace [C-RAW/C-XUI03]",
            )
        }
        Leaf::Measured(controller) => controller,
    };
    if controller.owner != pov_tag {
        pov_scope_drops.push(PovScopedDrop {
            component: IntelligenceComponent::Ciphers,
            row_identity: controller.producer_record_identity.clone(),
            recorded_owner: Some(controller.owner.clone()),
            pov_country: pov_tag.to_string(),
        });
        return unobtained(
            "CipherControllerRefreshExact.owner does not match pov_tag; all foreign-owner cipher rows were dropped",
        );
    }
    if let Err(error) = admit_cipher_controller_refresh(controller, frame_hour) {
        return unobtained(error.message);
    }
    let raw_state = frame.intel(pov_tag).map_or_else(
        || "WorldFrame.intel(pov_tag) is absent".to_string(),
        |intel| {
            format!(
                "IntelView.domain53_cryptology_targets_raw={}",
                leaf_state(&intel.domain53_cryptology_targets_raw)
            )
        },
    );
    unobtained(format!(
        "missing CipherUiRow.progress_denominator_raw, CipherUiRow.progress_fixed, CipherUiRow.cipher_broken [C-RAW] and MeasuredList.collection DynamicCollectionTrace [C-XUI03]; {raw_state}"
    ))
}

fn build_intel_levels(
    frame: &WorldFrame,
    pov_tag: &str,
    pov_scope_drops: &mut Vec<PovScopedDrop>,
) -> ProducerValue<MeasuredList<IntelLevelUiRow>> {
    let rows = match &frame.diplomacy().intel_levels {
        Leaf::Absent => {
            return unobtained(
                "DiplomacyView.intel_levels is Leaf::Absent; missing MeasuredList.collection DynamicCollectionTrace [C-XUI03]",
            )
        }
        Leaf::Unobtained => {
            return unobtained(
                "DiplomacyView.intel_levels is Leaf::Unobtained; missing MeasuredList.collection DynamicCollectionTrace [C-XUI03]",
            )
        }
        Leaf::Measured(rows) => rows,
    };
    let mut invalid = Vec::new();
    for row in rows {
        if row.owner != pov_tag {
            pov_scope_drops.push(PovScopedDrop {
                component: IntelligenceComponent::IntelLevels,
                row_identity: format!("intel:{}:{}", row.owner, row.target),
                recorded_owner: Some(row.owner.clone()),
                pov_country: pov_tag.to_string(),
            });
            continue;
        }
        for (value, field) in [
            (i64::from(row.civilian_raw), "IntelLevelRow.civilian_raw"),
            (i64::from(row.army_raw), "IntelLevelRow.army_raw"),
            (i64::from(row.navy_raw), "IntelLevelRow.navy_raw"),
            (i64::from(row.air_raw), "IntelLevelRow.air_raw"),
        ] {
            if scaled_fixed(value, row.scale, field).is_err() {
                invalid.push(format!("{} for target {}", field, row.target));
            }
        }
    }
    let measured_state = if rows.is_empty() {
        "DiplomacyView.intel_levels is Leaf::Measured(vec![])"
    } else {
        "DiplomacyView.intel_levels is Leaf::Measured"
    };
    let invalid = if invalid.is_empty() {
        String::new()
    } else {
        format!("; invalid scaled leaves: {}", invalid.join(", "))
    };
    unobtained(format!(
        "{measured_state}; missing MeasuredList.collection DynamicCollectionTrace for IntelLevelUiRow [C-XUI03]{invalid}"
    ))
}

fn build_networks(
    frame: &WorldFrame,
    pov_tag: &str,
    pov_scope_drops: &mut Vec<PovScopedDrop>,
) -> ProducerValue<IntelNetworkSummary> {
    let rows = match &frame.diplomacy().intel_network_graphs {
        Leaf::Absent => return unobtained("DiplomacyView.intel_network_graphs is Leaf::Absent"),
        Leaf::Unobtained => {
            return unobtained("DiplomacyView.intel_network_graphs is Leaf::Unobtained")
        }
        Leaf::Measured(rows) => rows,
    };
    let mut built = Vec::new();
    let mut missing = Vec::new();
    for row in rows {
        let row_identity = format!("network-slot:{}", row.network_slot);
        let owner = match row.owner.as_deref() {
            Some(owner) if owner != pov_tag => {
                pov_scope_drops.push(PovScopedDrop {
                    component: IntelligenceComponent::Networks,
                    row_identity,
                    recorded_owner: Some(owner.to_string()),
                    pov_country: pov_tag.to_string(),
                });
                continue;
            }
            Some(owner) => owner,
            None => {
                missing.push(format!(
                    "IntelNetworkGraphRow[slot={}].owner",
                    row.network_slot
                ));
                continue;
            }
        };
        let Some(owner_tag_db_id) = row.owner_tag_db_id else {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].owner_tag_db_id",
                row.network_slot
            ));
            continue;
        };
        let Some(operative_ids) = row.operative_ids.as_ref() else {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].operative_ids",
                row.network_slot
            ));
            continue;
        };
        let Some(state_strengths) = row.state_strengths.as_ref() else {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].state_strengths",
                row.network_slot
            ));
            continue;
        };
        let Some(state_strength_scale) = row.state_strength_scale else {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].state_strength_scale",
                row.network_slot
            ));
            continue;
        };
        let Some(national_coverage) = row.national_coverage.as_ref() else {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].national_coverage",
                row.network_slot
            ));
            continue;
        };
        if state_strengths.len() != row.state_ids.len()
            || !state_strengths
                .iter()
                .zip(&row.state_ids)
                .all(|(strength, state_id)| strength.state_id == *state_id)
        {
            missing.push(format!(
                "IntelNetworkGraphRow[slot={}].state_strengths exact state_ids join",
                row.network_slot
            ));
            continue;
        }
        let mut scaled_strengths = Vec::with_capacity(state_strengths.len());
        let mut row_invalid = false;
        for strength in state_strengths {
            match scaled_fixed(
                strength.strength_raw,
                state_strength_scale,
                "IntelNetworkStateStrength.strength_raw/IntelNetworkGraphRow.state_strength_scale",
            ) {
                Ok(strength_fixed) => {
                    scaled_strengths.push(IntelNetworkStateStrengthUiRow {
                        state_id: strength.state_id,
                        strength_fixed,
                    });
                }
                Err(reason) => {
                    missing.push(format!(
                        "IntelNetworkGraphRow[slot={}].{reason}",
                        row.network_slot
                    ));
                    row_invalid = true;
                    break;
                }
            }
        }
        if row_invalid {
            continue;
        }
        let national_coverage_fixed = match scaled_fixed(
            national_coverage.national_coverage_raw,
            national_coverage.scale,
            "IntelNetworkNationalCoverage.national_coverage_raw/IntelNetworkNationalCoverage.scale",
        ) {
            Ok(value) => value,
            Err(reason) => {
                missing.push(format!(
                    "IntelNetworkGraphRow[slot={}].{reason}",
                    row.network_slot
                ));
                continue;
            }
        };
        let network_id = format!("network:{owner_tag_db_id}:{}", row.network_slot);
        built.push(IntelNetworkUiRow {
            stable_item_id: network_id.clone(),
            network_id,
            owner_country: owner.to_string(),
            target_country: row.target.clone(),
            operative_ids: operative_ids
                .iter()
                .map(|operative_id| format!("operative:{operative_id}"))
                .collect(),
            state_ids: row.state_ids.clone(),
            state_strengths: scaled_strengths,
            national_coverage_fixed,
        });
    }
    if !missing.is_empty() {
        return unobtained(format!(
            "missing typed network leaves: {}",
            missing.join(", ")
        ));
    }
    ProducerValue::Measured(IntelNetworkSummary {
        order_authority: NetworkSummaryOrderAuthority::ProductStableContentKeyV1,
        rows: built,
    })
}

fn exact_collection(
    frame: &WorldFrame,
    collection_id: &str,
    frame_hour: u64,
) -> std::result::Result<DynamicCollectionTrace, String> {
    let view = frame.dynamic_collection_trace();
    match view.status {
        ViewStatus::DomainAbsent => {
            return Err(format!(
                "DynamicCollectionTraceView for {collection_id} is DomainAbsent [C-XUI03]"
            ))
        }
        ViewStatus::DomainUnavailable => {
            return Err(format!(
                "DynamicCollectionTraceView for {collection_id} is DomainUnavailable [C-XUI03]"
            ))
        }
        ViewStatus::Present => {}
    }
    if view.frame_hour != Some(frame_hour) {
        return Err(format!(
            "DynamicCollectionTraceView.frame_hour for {collection_id} does not match IntelligenceControllerCompanionExact.frame_hour [C-XUI03]"
        ));
    }
    let matches = view
        .collections
        .iter()
        .filter(|row| row.collection_instance_id == collection_id)
        .collect::<Vec<_>>();
    let [row] = matches.as_slice() else {
        return Err(format!(
            "DynamicCollectionTraceView.collections has {} rows for {collection_id}, expected exactly one [C-XUI03]",
            matches.len()
        ));
    };
    DynamicCollectionTrace::try_from(*row).map_err(|error| error.message)
}

fn availability_gate_mismatch(trace: &IntelligenceUiTrace) -> Option<String> {
    for gate in &trace.visibility {
        let component = match gate.surface {
            IntelligenceSurface::Agency => IntelligenceComponent::Agency,
            IntelligenceSurface::Upgrades => IntelligenceComponent::Upgrades,
            IntelligenceSurface::Operatives => IntelligenceComponent::Operatives,
            IntelligenceSurface::Operations => IntelligenceComponent::Operations,
            IntelligenceSurface::Ciphers => IntelligenceComponent::Ciphers,
            IntelligenceSurface::IntelLedger => IntelligenceComponent::IntelLevels,
            IntelligenceSurface::Networks => IntelligenceComponent::Networks,
        };
        let measured = match component {
            IntelligenceComponent::Agency => matches!(trace.agency, ProducerValue::Measured(_)),
            IntelligenceComponent::Upgrades => {
                matches!(trace.upgrades, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Operatives => {
                matches!(trace.operatives, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Operations => {
                matches!(trace.operations, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Ciphers => {
                matches!(trace.ciphers, ProducerValue::Measured(_))
            }
            IntelligenceComponent::IntelLevels => {
                matches!(trace.intel_levels, ProducerValue::Measured(_))
            }
            IntelligenceComponent::Networks => {
                matches!(trace.networks, ProducerValue::Measured(_))
            }
        };
        let gate_available = gate.disposition != VisibilityDisposition::Unavailable;
        if measured != gate_available {
            let detail = match component {
                IntelligenceComponent::Agency => producer_value_reason(&trace.agency),
                IntelligenceComponent::Upgrades => producer_value_reason(&trace.upgrades),
                IntelligenceComponent::Operatives => producer_value_reason(&trace.operatives),
                IntelligenceComponent::Operations => producer_value_reason(&trace.operations),
                IntelligenceComponent::Ciphers => producer_value_reason(&trace.ciphers),
                IntelligenceComponent::IntelLevels => producer_value_reason(&trace.intel_levels),
                IntelligenceComponent::Networks => producer_value_reason(&trace.networks),
            }
            .unwrap_or("component is measured while companion gate is Unavailable");
            return Some(format!(
                "{} availability disagrees with mirrored companion gate: {detail}",
                component.field_name()
            ));
        }
    }
    None
}

fn map_surface(surface: IntelligenceSurfaceId) -> IntelligenceSurface {
    match surface {
        IntelligenceSurfaceId::Agency => IntelligenceSurface::Agency,
        IntelligenceSurfaceId::Upgrades => IntelligenceSurface::Upgrades,
        IntelligenceSurfaceId::Operatives => IntelligenceSurface::Operatives,
        IntelligenceSurfaceId::Operations => IntelligenceSurface::Operations,
        IntelligenceSurfaceId::Ciphers => IntelligenceSurface::Ciphers,
        IntelligenceSurfaceId::IntelLedger => IntelligenceSurface::IntelLedger,
        IntelligenceSurfaceId::Networks => IntelligenceSurface::Networks,
    }
}

fn map_visibility(visibility: IntelligenceVisibilityExact) -> VisibilityDisposition {
    match visibility {
        IntelligenceVisibilityExact::Visible => VisibilityDisposition::Visible,
        IntelligenceVisibilityExact::Hidden => VisibilityDisposition::Hidden,
        IntelligenceVisibilityExact::Unavailable => VisibilityDisposition::Unavailable,
    }
}

fn scaled_fixed(numerator: i64, denominator: i64, field: &str) -> std::result::Result<i64, String> {
    if denominator <= 0 || numerator < 0 || numerator > denominator {
        return Err(format!(
            "{field} has invalid numerator or carried denominator"
        ));
    }
    let scaled = i128::from(numerator)
        .checked_mul(i128::from(FIXED_ONE))
        .and_then(|value| value.checked_div(i128::from(denominator)))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(|| format!("{field} fixed-point conversion overflowed"))?;
    if !(0..=FIXED_ONE).contains(&scaled) {
        return Err(format!("{field} fixed-point conversion is out of range"));
    }
    Ok(scaled)
}

fn leaf_state<T>(leaf: &Leaf<T>) -> &'static str {
    match leaf {
        Leaf::Absent => "Leaf::Absent",
        Leaf::Unobtained => "Leaf::Unobtained",
        Leaf::Measured(_) => "Leaf::Measured",
    }
}

fn producer_value_reason<T>(value: &ProducerValue<T>) -> Option<&str> {
    match value {
        ProducerValue::Measured(_) => None,
        ProducerValue::Unobtained { reason } => Some(reason),
    }
}

fn unobtained<T>(reason: impl Into<String>) -> ProducerValue<T> {
    ProducerValue::Unobtained {
        reason: reason.into(),
    }
}

/// Derive the operative -> operation relationship from the exact operation
/// producer. This deliberately does not inspect operative status text or infer
/// a relationship from shared targets. Duplicate or orphan handles fail the
/// complete projection closed.
pub fn project_operation_assignments(
    operatives: &ProducerValue<MeasuredList<OperativeUiRow>>,
    operations: &ProducerValue<MeasuredList<OperationUiRow>>,
) -> Result<ProducerValue<BTreeMap<String, String>>> {
    let (ProducerValue::Measured(operatives), ProducerValue::Measured(operations)) =
        (operatives, operations)
    else {
        let mut reasons = Vec::new();
        if let ProducerValue::Unobtained { reason } = operatives {
            reasons.push(format!("operatives: {reason}"));
        }
        if let ProducerValue::Unobtained { reason } = operations {
            reasons.push(format!("operations: {reason}"));
        }
        return Ok(ProducerValue::Unobtained {
            reason: if reasons.is_empty() {
                "operation assignment inputs unavailable".into()
            } else {
                reasons.join("; ")
            },
        });
    };
    let known_operatives = operatives
        .rows
        .iter()
        .map(|row| row.operative_id.clone())
        .collect::<BTreeSet<_>>();
    let mut projected = BTreeMap::new();
    for operation in &operations.rows {
        nonempty(&operation.operation_id, "operation_id")?;
        let mut within_operation = BTreeSet::new();
        for operative_id in &operation.assigned_operative_ids {
            nonempty(operative_id, "assigned operative id")?;
            if !within_operation.insert(operative_id.as_str()) {
                return Err(fail("operation assignment duplicates an operative"));
            }
            if !known_operatives.contains(operative_id) {
                return Err(fail("operation assignment references an unknown operative"));
            }
            if projected
                .insert(operative_id.clone(), operation.operation_id.clone())
                .is_some()
            {
                return Err(fail("operative is assigned to multiple operations"));
            }
        }
    }
    Ok(ProducerValue::Measured(projected))
}

/// Join the dormant exact-a729 selected-agency refresh carrier to semantic
/// upgrade rows for diagnostic comparison. This does not activate the producer
/// or make its recorder-POV XUI03 materialization trace render authority. This
/// diagnostic comparison requires an explicit stable row order and does
/// not infer one from the selected-controller trace.
pub fn project_agency_upgrade_refresh_order(
    refresh: &AgencyUpgradeControllerRefreshExact,
    expected_frame_hour: u64,
    expected_owner_country: &str,
    upgrades: &ProducerValue<MeasuredList<AgencyUpgradeRow>>,
) -> Result<ProducerValue<Vec<String>>> {
    let admitted = admit_agency_upgrade_controller_refresh(refresh, expected_frame_hour)
        .map_err(|error| fail(&error.message))?;
    if refresh.owner != expected_owner_country {
        return Err(fail("agency-upgrade refresh owner mismatch"));
    }
    let ProducerValue::Measured(upgrades) = upgrades else {
        let reason = match upgrades {
            ProducerValue::Unobtained { reason } => reason,
            ProducerValue::Measured(_) => unreachable!("let-else excluded measured upgrades"),
        };
        return Ok(ProducerValue::Unobtained {
            reason: format!("upgrades: {reason}"),
        });
    };
    let mut rows = BTreeMap::new();
    let mut stable_item_ids = BTreeSet::new();
    for row in &upgrades.rows {
        nonempty(&row.stable_item_id, "upgrade stable_item_id")?;
        nonempty(&row.upgrade_id, "upgrade_id")?;
        nonempty(&row.branch_id, "upgrade branch_id")?;
        if !stable_item_ids.insert(row.stable_item_id.as_str())
            || rows.insert(row.upgrade_id.as_str(), row).is_some()
        {
            return Err(fail(
                "duplicate upgrade stable_item_id or upgrade_id in semantic producer",
            ));
        }
    }
    let mut ordered = Vec::new();
    for (branch_id, controller_row) in admitted.ordered_upgrades() {
        let Some(row) = rows.get(controller_row.upgrade_id.as_str()) else {
            return Err(fail(
                "agency-upgrade controller references an unjoined semantic row",
            ));
        };
        if row.branch_id != branch_id {
            return Err(fail("agency-upgrade branch definition join mismatch"));
        }
        ordered.push(row.stable_item_id.clone());
    }
    if ordered.len() != upgrades.rows.len() {
        return Err(fail(
            "semantic upgrade producer contains rows absent from the controller refresh",
        ));
    }
    Ok(ProducerValue::Measured(ordered))
}

/// Join the activation-disabled exact cryptology-country controller vector to
/// semantic cipher rows. The final vector is captured after the engine's
/// selected-first/name sort, so replay never reimplements that comparator.
pub fn project_cipher_controller_order(
    refresh: &CipherControllerRefreshExact,
    expected_frame_hour: u64,
    expected_owner_country: &str,
    ciphers: &ProducerValue<MeasuredList<CipherUiRow>>,
) -> Result<ProducerValue<Vec<String>>> {
    let admitted = admit_cipher_controller_refresh(refresh, expected_frame_hour)
        .map_err(|error| fail(&error.message))?;
    if refresh.owner != expected_owner_country {
        return Err(fail("cipher controller owner mismatch"));
    }
    let ProducerValue::Measured(ciphers) = ciphers else {
        let reason = match ciphers {
            ProducerValue::Unobtained { reason } => reason,
            ProducerValue::Measured(_) => unreachable!("let-else excluded measured ciphers"),
        };
        return Ok(ProducerValue::Unobtained {
            reason: format!("ciphers: {reason}"),
        });
    };
    let mut rows = BTreeMap::new();
    let mut stable_item_ids = BTreeSet::new();
    for row in &ciphers.rows {
        nonempty(&row.stable_item_id, "cipher stable_item_id")?;
        nonempty(&row.target_country, "cipher target_country")?;
        if row.owner_country != expected_owner_country
            || !stable_item_ids.insert(row.stable_item_id.as_str())
            || rows.insert(row.target_country.as_str(), row).is_some()
        {
            return Err(fail(
                "cipher owner, stable identity, or target identity mismatch",
            ));
        }
    }
    let mut ordered = Vec::with_capacity(refresh.targets.len());
    for target in admitted.ordered_targets() {
        let Some(row) = rows.get(target.target.as_str()) else {
            return Err(fail(
                "cipher controller references an unjoined semantic row",
            ));
        };
        ordered.push(row.stable_item_id.clone());
    }
    if ordered.len() != ciphers.rows.len() {
        return Err(fail(
            "semantic cipher producer contains rows absent from the controller refresh",
        ));
    }
    Ok(ProducerValue::Measured(ordered))
}

#[derive(Debug)]
pub struct AdmittedIntelligenceUi<'a> {
    trace: &'a IntelligenceUiTrace,
    visibility: BTreeMap<IntelligenceSurface, VisibilityDisposition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntelligencePanelContent<'a> {
    Agency(&'a AgencyState),
    Upgrades(Vec<&'a AgencyUpgradeRow>),
    Operatives(Vec<&'a OperativeUiRow>),
    Operations(Vec<&'a OperationUiRow>),
    Ciphers(Vec<&'a CipherUiRow>),
    IntelLedger(Vec<&'a IntelLevelUiRow>),
    Networks(Vec<&'a IntelNetworkUiRow>),
}

/// Final replay-owned panel state. Hidden and unavailable surfaces never carry
/// content; unavailable surfaces additionally preserve the producer's reason.
/// A visible measured-empty collection carries `Some(<variant>(vec![]))`, so
/// it cannot be confused with missing producer data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntelligencePanelRender<'a> {
    pub surface: IntelligenceSurface,
    pub disposition: VisibilityDisposition,
    pub gate_reason: Option<&'a str>,
    pub producer_reason: Option<&'a str>,
    pub content: Option<IntelligencePanelContent<'a>>,
}

trait StableItem {
    fn stable_item_id(&self) -> &str;
}

macro_rules! stable_item {
    ($ty:ty) => {
        impl StableItem for $ty {
            fn stable_item_id(&self) -> &str {
                &self.stable_item_id
            }
        }
    };
}
stable_item!(AgencyUpgradeRow);
stable_item!(OperativeUiRow);
stable_item!(OperationUiRow);
stable_item!(CipherUiRow);
stable_item!(IntelLevelUiRow);
stable_item!(IntelNetworkUiRow);

pub fn admit_intelligence_ui(trace: &IntelligenceUiTrace) -> Result<AdmittedIntelligenceUi<'_>> {
    nonempty(&trace.producer_record_identity, "producer_record_identity")?;
    nonempty(&trace.pov_country, "pov_country")?;
    nonempty(&trace.owner_country, "owner_country")?;
    let visibility = validate_visibility(&trace.visibility)?;

    match &trace.agency {
        ProducerValue::Measured(agency) => {
            require_available(&visibility, IntelligenceSurface::Agency, true)?;
            nonempty(&agency.agency_instance_id, "agency_instance_id")?;
            nonempty(&agency.name_text, "agency name")?;
            nonempty(&agency.icon_sprite, "agency icon")?;
            owner(&agency.owner_country, &trace.owner_country, "agency")?;
            fixed(agency.upgrade_progress_fixed, "agency upgrade progress")?;
            if agency.defense_raw < 0 || agency.defense_denominator_raw <= 0 {
                return Err(fail("agency defense raw scale is invalid"));
            }
            if agency.usable_slots > agency.max_operatives {
                return Err(fail("agency usable slots exceed max operatives"));
            }
        }
        ProducerValue::Unobtained { reason } => {
            nonempty(reason, "agency unobtained reason")?;
            require_available(&visibility, IntelligenceSurface::Agency, false)?;
        }
    }

    validate_family(
        &trace.upgrades,
        IntelligenceSurface::Upgrades,
        trace.frame_hour,
        &visibility,
    )?;
    validate_family(
        &trace.operatives,
        IntelligenceSurface::Operatives,
        trace.frame_hour,
        &visibility,
    )?;
    validate_family(
        &trace.operations,
        IntelligenceSurface::Operations,
        trace.frame_hour,
        &visibility,
    )?;
    validate_family(
        &trace.ciphers,
        IntelligenceSurface::Ciphers,
        trace.frame_hour,
        &visibility,
    )?;
    validate_family(
        &trace.intel_levels,
        IntelligenceSurface::IntelLedger,
        trace.frame_hour,
        &visibility,
    )?;
    validate_network_summary(&trace.networks, &visibility)?;

    let operative_ids: BTreeSet<String> = match &trace.operatives {
        ProducerValue::Measured(rows) => rows
            .rows
            .iter()
            .map(|row| row.operative_id.clone())
            .collect(),
        ProducerValue::Unobtained { .. } => BTreeSet::new(),
    };
    let operation_ids: BTreeSet<String> = match &trace.operations {
        ProducerValue::Measured(rows) => rows
            .rows
            .iter()
            .map(|row| row.operation_id.clone())
            .collect(),
        ProducerValue::Unobtained { .. } => BTreeSet::new(),
    };
    let network_ids: BTreeSet<String> = match &trace.networks {
        ProducerValue::Measured(rows) => {
            rows.rows.iter().map(|row| row.network_id.clone()).collect()
        }
        ProducerValue::Unobtained { .. } => BTreeSet::new(),
    };
    if let ProducerValue::Measured(rows) = &trace.operatives {
        if operative_ids.len() != rows.rows.len() {
            return Err(fail("duplicate operative_id"));
        }
    }
    if let ProducerValue::Measured(rows) = &trace.operations {
        if operation_ids.len() != rows.rows.len() {
            return Err(fail("duplicate operation_id"));
        }
    }
    if let ProducerValue::Measured(rows) = &trace.networks {
        if network_ids.len() != rows.rows.len() {
            return Err(fail("duplicate network_id"));
        }
    }
    let operation_assignment_projection =
        project_operation_assignments(&trace.operatives, &trace.operations)?;
    if let ProducerValue::Measured(rows) = &trace.upgrades {
        let mut upgrade_ids = BTreeSet::new();
        for row in &rows.rows {
            nonempty(&row.upgrade_id, "upgrade_id")?;
            nonempty(&row.branch_id, "upgrade branch_id")?;
            nonempty(&row.icon_sprite, "upgrade icon")?;
            if !upgrade_ids.insert(row.upgrade_id.as_str()) {
                return Err(fail("duplicate upgrade_id"));
            }
        }
        if let ProducerValue::Measured(agency) = &trace.agency {
            if agency
                .current_upgrade_id
                .as_ref()
                .is_some_and(|id| !upgrade_ids.contains(id.as_str()))
            {
                return Err(fail("current agency upgrade is unjoined"));
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.operations {
        for row in &rows.rows {
            nonempty(&row.operation_id, "operation_id")?;
            nonempty(&row.definition_id, "operation definition_id")?;
            owner(&row.owner_country, &trace.owner_country, "operation")?;
            match &row.target {
                OperationTarget::Country(country) => nonempty(country, "operation target country")?,
                OperationTarget::State(0) => return Err(fail("operation target state is zero")),
                OperationTarget::State(_) => {}
            }
            fixed(row.progress_fixed, "operation progress")?;
            if row.phase_count == 0 || row.phase_index >= row.phase_count {
                return Err(fail("operation phase/duration is invalid"));
            }
            match row.status {
                OperationStatus::Preparing | OperationStatus::Prepared => {
                    if row.start_hour != 0 || row.duration_hours != 0 {
                        return Err(fail("pre-start operation carries runtime timing"));
                    }
                }
                OperationStatus::Running | OperationStatus::Finished | OperationStatus::Aborted => {
                    if row.start_hour <= 0 || row.duration_hours == 0 {
                        return Err(fail("started operation lacks runtime timing"));
                    }
                }
            }
            let mut assigned = BTreeSet::new();
            for id in &row.assigned_operative_ids {
                nonempty(id, "assigned operative id")?;
                if !assigned.insert(id.as_str()) || !operative_ids.contains(id) {
                    return Err(fail("operation assignment is duplicate or unjoined"));
                }
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.operatives {
        for row in &rows.rows {
            nonempty(&row.operative_id, "operative_id")?;
            nonempty(&row.name_text, "operative name")?;
            nonempty(&row.portrait_sprite, "operative portrait")?;
            owner(&row.owner_country, &trace.owner_country, "operative")?;
            fixed(row.experience_fixed, "operative experience")?;
            unique_nonempty(&row.traits, "operative traits")?;
            unique_nonempty(&row.nationalities, "operative nationalities")?;
            match (
                row.captured,
                row.captured_by.as_deref(),
                row.captured_by_tag_db_id,
                row.capture_end_date_raw,
            ) {
                (true, Some(country), Some(tag_db_id), Some(_)) if tag_db_id != 0 => {
                    nonempty(country, "captured_by")?
                }
                (true, _, _, _) => {
                    return Err(fail(
                        "captured operative lacks exact captured-by/date identity",
                    ))
                }
                (false, None, None, None) => {}
                (false, _, _, _) => {
                    return Err(fail(
                        "uncaptured operative carries captured-by/date identity",
                    ))
                }
            }
            if row.captured && row.recruitable_pool_member {
                return Err(fail("operative cannot be captured and recruitable"));
            }
            if matches!(row.assignment, OperativeAssignment::Captured { .. }) != row.captured {
                return Err(fail("captured lifecycle disagrees with assignment"));
            }
            if matches!(row.assignment, OperativeAssignment::Recruitable)
                != row.recruitable_pool_member
            {
                return Err(fail(
                    "recruitable-pool membership disagrees with assignment",
                ));
            }
            match &row.assignment {
                OperativeAssignment::Mission {
                    mission_id,
                    mission_action_id,
                    target_country,
                    target_country_tag_db_id,
                    target_state,
                    authored_subject_id,
                } => {
                    nonempty(mission_id, "mission_id")?;
                    nonempty(mission_action_id, "mission_action_id")?;
                    if target_country.is_none() || *target_country_tag_db_id == 0 {
                        return Err(fail("mission target country identity is absent"));
                    }
                    if let Some(country) = target_country {
                        nonempty(country, "mission target country")?;
                    }
                    if target_state == &Some(0) {
                        return Err(fail("mission target state is zero"));
                    }
                    if authored_subject_id.as_deref().is_some_and(str::is_empty) {
                        return Err(fail("mission authored subject identity is empty"));
                    }
                }
                OperativeAssignment::Operation { operation_id } => {
                    if !operation_ids.contains(operation_id) {
                        return Err(fail("operative operation assignment is unjoined"));
                    }
                    let reciprocal = match &trace.operations {
                        ProducerValue::Measured(operations) => {
                            operations.rows.iter().any(|operation| {
                                operation.operation_id == *operation_id
                                    && operation.assigned_operative_ids.contains(&row.operative_id)
                            })
                        }
                        _ => false,
                    };
                    if !reciprocal {
                        return Err(fail("operative/operation assignment is not reciprocal"));
                    }
                    if !matches!(
                        &operation_assignment_projection,
                        ProducerValue::Measured(projected)
                            if projected.get(&row.operative_id) == Some(operation_id)
                    ) {
                        return Err(fail(
                            "operative operation assignment contradicts the OPIN inverse",
                        ));
                    }
                }
                OperativeAssignment::Network { network_id } => {
                    if !network_ids.contains(network_id) {
                        return Err(fail("operative network assignment is unjoined"));
                    }
                    let reciprocal = match &trace.networks {
                        ProducerValue::Measured(networks) => networks.rows.iter().any(|network| {
                            network.network_id == *network_id
                                && network.operative_ids.contains(&row.operative_id)
                        }),
                        _ => false,
                    };
                    if !reciprocal {
                        return Err(fail("operative/network assignment is not reciprocal"));
                    }
                }
                OperativeAssignment::Captured { captured_by } => {
                    nonempty(captured_by, "captured_by")?;
                    if row.captured_by.as_deref() != Some(captured_by.as_str()) {
                        return Err(fail(
                            "captured assignment country disagrees with captured_by",
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.operations {
        for operation in &rows.rows {
            for operative_id in &operation.assigned_operative_ids {
                let reciprocal = match &trace.operatives {
                    ProducerValue::Measured(ops) => ops.rows.iter().any(|row| row.operative_id == *operative_id && matches!(&row.assignment, OperativeAssignment::Operation { operation_id } if operation_id == &operation.operation_id)),
                    _ => false,
                };
                if !reciprocal {
                    return Err(fail("operation/operative assignment is not reciprocal"));
                }
            }
        }
    }
    if let ProducerValue::Measured(projected) = &operation_assignment_projection {
        if let ProducerValue::Measured(operatives) = &trace.operatives {
            for (operative_id, operation_id) in projected {
                if !operatives.rows.iter().any(|row| {
                    row.operative_id == *operative_id
                        && matches!(
                            &row.assignment,
                            OperativeAssignment::Operation { operation_id: row_operation_id }
                                if row_operation_id == operation_id
                        )
                }) {
                    return Err(fail(
                        "OPIN inverse lacks the matching operative assignment projection",
                    ));
                }
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.ciphers {
        let mut targets = BTreeSet::new();
        for row in &rows.rows {
            owner(&row.owner_country, &trace.owner_country, "cipher")?;
            nonempty(&row.target_country, "cipher target")?;
            if row.decryption_points_raw < 0 || row.progress_denominator_raw <= 0 {
                return Err(fail("cipher raw progress/denominator is invalid"));
            }
            fixed(row.progress_fixed, "cipher progress")?;
            if !targets.insert(row.target_country.as_str()) {
                return Err(fail("duplicate cipher target"));
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.intel_levels {
        let mut targets = BTreeSet::new();
        for row in &rows.rows {
            owner(&row.viewer_country, &trace.pov_country, "intel viewer")?;
            nonempty(&row.target_country, "intel target")?;
            for (value, label) in [
                (row.civilian_fixed, "civilian intel"),
                (row.army_fixed, "army intel"),
                (row.navy_fixed, "navy intel"),
                (row.air_fixed, "air intel"),
            ] {
                fixed(value, label)?;
            }
            if !targets.insert(row.target_country.as_str()) {
                return Err(fail("duplicate intel target"));
            }
        }
    }
    if let ProducerValue::Measured(rows) = &trace.networks {
        for row in &rows.rows {
            nonempty(&row.network_id, "network_id")?;
            owner(&row.owner_country, &trace.owner_country, "network")?;
            nonempty(&row.target_country, "network target")?;
            fixed(row.national_coverage_fixed, "network coverage")?;
            unique_nonempty(&row.operative_ids, "network operative ids")?;
            if row
                .operative_ids
                .iter()
                .any(|id| !operative_ids.contains(id))
            {
                return Err(fail("network operative is unjoined"));
            }
            for operative_id in &row.operative_ids {
                let reciprocal = match &trace.operatives {
                    ProducerValue::Measured(operatives) => {
                        operatives.rows.iter().any(|operative| {
                            operative.operative_id == *operative_id
                                && matches!(
                                    &operative.assignment,
                                    OperativeAssignment::Network { network_id }
                                        if network_id == &row.network_id
                                )
                        })
                    }
                    _ => false,
                };
                if !reciprocal {
                    return Err(fail("network/operative assignment is not reciprocal"));
                }
            }
            if !row.state_ids.windows(2).all(|w| w[0] < w[1]) {
                return Err(fail("network state ids must be sorted unique"));
            }
            if row.state_strengths.len() != row.state_ids.len()
                || !row
                    .state_strengths
                    .iter()
                    .zip(&row.state_ids)
                    .all(|(strength, state_id)| {
                        strength.state_id == *state_id
                            && (0..=FIXED_ONE).contains(&strength.strength_fixed)
                    })
            {
                return Err(fail(
                    "network state strengths must exactly match sorted state ids",
                ));
            }
        }
    }
    Ok(AdmittedIntelligenceUi { trace, visibility })
}

impl<'a> AdmittedIntelligenceUi<'a> {
    pub fn visibility(&self, surface: IntelligenceSurface) -> VisibilityDisposition {
        self.visibility[&surface]
    }
    pub fn ordered_operatives(&self) -> Result<Vec<&'a OperativeUiRow>> {
        ordered(&self.trace.operatives)
    }
    pub fn ordered_upgrades(&self) -> Result<Vec<&'a AgencyUpgradeRow>> {
        ordered(&self.trace.upgrades)
    }
    pub fn ordered_operations(&self) -> Result<Vec<&'a OperationUiRow>> {
        ordered(&self.trace.operations)
    }
    pub fn ordered_ciphers(&self) -> Result<Vec<&'a CipherUiRow>> {
        ordered(&self.trace.ciphers)
    }
    pub fn ordered_intel_levels(&self) -> Result<Vec<&'a IntelLevelUiRow>> {
        ordered(&self.trace.intel_levels)
    }
    pub fn ordered_networks(&self) -> Result<Vec<&'a IntelNetworkUiRow>> {
        let summary = match &self.trace.networks {
            ProducerValue::Measured(summary) => summary,
            ProducerValue::Unobtained { .. } => return Ok(Vec::new()),
        };
        let mut rows = summary.rows.iter().collect::<Vec<_>>();
        rows.sort_by(|left, right| network_content_key(left).cmp(&network_content_key(right)));
        Ok(rows)
    }

    /// Builds all seven panels in the frozen surface order. The method is
    /// intentionally downstream of admission so renderer code cannot bypass
    /// controller order, availability, identity, or reciprocal-join checks.
    pub fn render_panels(&self) -> Result<Vec<IntelligencePanelRender<'a>>> {
        IntelligenceSurface::ALL
            .into_iter()
            .map(|surface| {
                let gate = self
                    .trace
                    .visibility
                    .iter()
                    .find(|gate| gate.surface == surface)
                    .ok_or_else(|| fail("admitted trace lost a visibility gate"))?;
                let producer_reason = self.producer_reason(surface);
                let content = if gate.disposition == VisibilityDisposition::Visible {
                    Some(self.visible_content(surface)?)
                } else {
                    None
                };
                Ok(IntelligencePanelRender {
                    surface,
                    disposition: gate.disposition,
                    gate_reason: gate.reason.as_deref(),
                    producer_reason,
                    content,
                })
            })
            .collect()
    }

    fn visible_content(
        &self,
        surface: IntelligenceSurface,
    ) -> Result<IntelligencePanelContent<'a>> {
        match surface {
            IntelligenceSurface::Agency => match &self.trace.agency {
                ProducerValue::Measured(agency) => Ok(IntelligencePanelContent::Agency(agency)),
                ProducerValue::Unobtained { .. } => {
                    Err(fail("visible agency has no measured producer value"))
                }
            },
            IntelligenceSurface::Upgrades => {
                Ok(IntelligencePanelContent::Upgrades(self.ordered_upgrades()?))
            }
            IntelligenceSurface::Operatives => Ok(IntelligencePanelContent::Operatives(
                self.ordered_operatives()?,
            )),
            IntelligenceSurface::Operations => Ok(IntelligencePanelContent::Operations(
                self.ordered_operations()?,
            )),
            IntelligenceSurface::Ciphers => {
                Ok(IntelligencePanelContent::Ciphers(self.ordered_ciphers()?))
            }
            IntelligenceSurface::IntelLedger => Ok(IntelligencePanelContent::IntelLedger(
                self.ordered_intel_levels()?,
            )),
            IntelligenceSurface::Networks => {
                Ok(IntelligencePanelContent::Networks(self.ordered_networks()?))
            }
        }
    }

    fn producer_reason(&self, surface: IntelligenceSurface) -> Option<&'a str> {
        match surface {
            IntelligenceSurface::Agency => unobtained_reason(&self.trace.agency),
            IntelligenceSurface::Upgrades => unobtained_reason(&self.trace.upgrades),
            IntelligenceSurface::Operatives => unobtained_reason(&self.trace.operatives),
            IntelligenceSurface::Operations => unobtained_reason(&self.trace.operations),
            IntelligenceSurface::Ciphers => unobtained_reason(&self.trace.ciphers),
            IntelligenceSurface::IntelLedger => unobtained_reason(&self.trace.intel_levels),
            IntelligenceSurface::Networks => unobtained_reason(&self.trace.networks),
        }
    }
}

fn unobtained_reason<T>(value: &ProducerValue<T>) -> Option<&str> {
    match value {
        ProducerValue::Measured(_) => None,
        ProducerValue::Unobtained { reason } => Some(reason.as_str()),
    }
}

fn validate_family<T: StableItem>(
    value: &ProducerValue<MeasuredList<T>>,
    surface: IntelligenceSurface,
    frame_hour: u64,
    gates: &BTreeMap<IntelligenceSurface, VisibilityDisposition>,
) -> Result<()> {
    match value {
        ProducerValue::Unobtained { reason } => {
            nonempty(reason, "family unobtained reason")?;
            require_available(gates, surface, false)?;
            Ok(())
        }
        ProducerValue::Measured(list) => {
            require_available(gates, surface, true)?;
            list.collection.validate()?;
            if list.collection.frame_hour != frame_hour {
                return Err(fail(
                    "collection frame_hour differs from intelligence frame",
                ));
            }
            let mut ids = BTreeSet::new();
            for row in &list.rows {
                nonempty(row.stable_item_id(), "row stable_item_id")?;
                if !ids.insert(row.stable_item_id().to_owned()) {
                    return Err(fail("duplicate producer row stable_item_id"));
                }
            }
            let source: BTreeSet<String> = list
                .collection
                .source_items
                .iter()
                .map(|r| r.stable_item_id.clone())
                .collect();
            if ids != source {
                return Err(fail("producer rows do not equal collection source items"));
            }
            Ok(())
        }
    }
}

/// Production XUI14 admission. The replay-facing semantic trace is accepted
/// only when its operative/operation controller order and all seven visibility
/// gates are joined to the same authenticated selected Present. The older
/// `admit_intelligence_ui` entry point remains useful for isolated schema tests;
/// production composition must call this companion-gated entry point.
pub fn admit_intelligence_ui_with_selected_present_companion<'a>(
    trace: &'a IntelligenceUiTrace,
    companion: &IntelligenceControllerCompanionExact,
    selected_present: &SelectedPresentReceipt,
) -> Result<AdmittedIntelligenceUi<'a>> {
    let admitted = admit_intelligence_controller_companion(companion, selected_present)
        .map_err(|error| fail(&error.message))?;
    let exact = admitted.companion();
    if trace.frame_hour != exact.frame_hour
        || trace.owner_country != exact.owner
        || trace.producer_record_identity != exact.producer_record_identity
    {
        return Err(fail(
            "selected-Present companion semantic identity mismatch",
        ));
    }

    let operative_order = match &trace.operatives {
        ProducerValue::Measured(rows) => rows.collection.ordered_item_ids.as_slice(),
        ProducerValue::Unobtained { .. } => &[],
    };
    let operation_order = match &trace.operations {
        ProducerValue::Measured(rows) => rows.collection.ordered_item_ids.as_slice(),
        ProducerValue::Unobtained { .. } => &[],
    };
    if operative_order != admitted.operative_order()
        || operation_order != admitted.operation_order()
    {
        return Err(fail(
            "operative or operation controller order companion mismatch",
        ));
    }
    match &trace.operatives {
        ProducerValue::Measured(rows) => {
            if rows.rows.len() != admitted.operative_portraits().len() {
                return Err(fail("operative portrait companion coverage mismatch"));
            }
            for (row, portrait) in rows.rows.iter().zip(admitted.operative_portraits()) {
                if row.stable_item_id != portrait.stable_item_id
                    || row.operative_id != portrait.operative_id.to_string()
                    || row.portrait_sprite != portrait.identity
                {
                    return Err(fail("operative final portrait producer join mismatch"));
                }
            }
        }
        ProducerValue::Unobtained { .. } if !admitted.operative_portraits().is_empty() => {
            return Err(fail("unobtained operatives carry final portrait rows"));
        }
        ProducerValue::Unobtained { .. } => {}
    }
    match &trace.operatives {
        ProducerValue::Measured(rows) => {
            let mission_products = admitted
                .operative_mission_products()
                .iter()
                .map(|product| (product.stable_item_id.as_str(), product))
                .collect::<BTreeMap<_, _>>();
            let mut joined = 0usize;
            for row in &rows.rows {
                match &row.assignment {
                    OperativeAssignment::Mission {
                        mission_id,
                        mission_action_id,
                        target_country: _,
                        target_country_tag_db_id,
                        target_state,
                        authored_subject_id,
                    } => {
                        let Some(product) = mission_products.get(row.stable_item_id.as_str())
                        else {
                            return Err(fail("mission assignment lacks exact product companion"));
                        };
                        if row.operative_id != product.operative_id.to_string()
                            || mission_id != &product.authored_mission_localization_key
                            || mission_action_id != &product.authored_action_localization_key
                            || target_country_tag_db_id != &product.target_country_tag_db_id
                            || target_state != &product.target_state_id
                            || authored_subject_id != &product.authored_subject_id
                        {
                            return Err(fail("operative mission producer join mismatch"));
                        }
                        joined = joined.saturating_add(1);
                    }
                    _ if mission_products.contains_key(row.stable_item_id.as_str()) => {
                        return Err(fail("non-mission operative carries mission product"));
                    }
                    _ => {}
                }
            }
            if joined != mission_products.len() {
                return Err(fail("operative mission product coverage mismatch"));
            }
        }
        ProducerValue::Unobtained { .. } if !admitted.operative_mission_products().is_empty() => {
            return Err(fail("unobtained operatives carry mission product rows"));
        }
        ProducerValue::Unobtained { .. } => {}
    }

    match &trace.operatives {
        ProducerValue::Measured(rows) => {
            let capture_join = admitted.capture_recruitable_join();
            if capture_join.rows.len() != rows.rows.len() {
                return Err(fail("capture/recruitable companion coverage mismatch"));
            }
            for (row, exact) in rows.rows.iter().zip(&capture_join.rows) {
                if row.stable_item_id != format!("operative:{}", exact.operative_id)
                    || row.operative_id != exact.operative_id.to_string()
                    || row.captured != exact.captured
                    || row.captured_by_tag_db_id != exact.captured_by_tag_db_id
                    || row.capture_end_date_raw != exact.capture_end_date_raw
                    || row.recruitable_pool_member != exact.recruitable_pool_member
                {
                    return Err(fail("operative capture/recruitable producer join mismatch"));
                }
            }
        }
        ProducerValue::Unobtained { .. } => {
            return Err(fail(
                "unobtained operatives carry capture/recruitable companion rows",
            ));
        }
    }

    let companion_gates = exact
        .visibility
        .iter()
        .map(|gate| {
            let surface = match gate.surface {
                IntelligenceSurfaceId::Agency => IntelligenceSurface::Agency,
                IntelligenceSurfaceId::Upgrades => IntelligenceSurface::Upgrades,
                IntelligenceSurfaceId::Operatives => IntelligenceSurface::Operatives,
                IntelligenceSurfaceId::Operations => IntelligenceSurface::Operations,
                IntelligenceSurfaceId::Ciphers => IntelligenceSurface::Ciphers,
                IntelligenceSurfaceId::IntelLedger => IntelligenceSurface::IntelLedger,
                IntelligenceSurfaceId::Networks => IntelligenceSurface::Networks,
            };
            let disposition = match gate.disposition {
                IntelligenceVisibilityExact::Visible => VisibilityDisposition::Visible,
                IntelligenceVisibilityExact::Hidden => VisibilityDisposition::Hidden,
                IntelligenceVisibilityExact::Unavailable => VisibilityDisposition::Unavailable,
            };
            (surface, (disposition, gate.reason.as_deref()))
        })
        .collect::<BTreeMap<_, _>>();
    for gate in &trace.visibility {
        let Some((disposition, reason)) = companion_gates.get(&gate.surface).copied() else {
            return Err(fail(
                "selected-Present companion visibility surface missing",
            ));
        };
        if gate.disposition != disposition || gate.reason.as_deref() != reason {
            return Err(fail("selected-Present companion visibility gate mismatch"));
        }
    }
    admit_intelligence_ui(trace)
}

fn validate_network_summary(
    value: &ProducerValue<IntelNetworkSummary>,
    gates: &BTreeMap<IntelligenceSurface, VisibilityDisposition>,
) -> Result<()> {
    match value {
        ProducerValue::Unobtained { reason } => {
            nonempty(reason, "network summary unobtained reason")?;
            require_available(gates, IntelligenceSurface::Networks, false)
        }
        ProducerValue::Measured(summary) => {
            require_available(gates, IntelligenceSurface::Networks, true)?;
            if summary.order_authority != NetworkSummaryOrderAuthority::ProductStableContentKeyV1 {
                return Err(fail("unknown network summary order authority"));
            }
            let mut stable_ids = BTreeSet::new();
            for row in &summary.rows {
                nonempty(&row.stable_item_id, "network stable_item_id")?;
                if !stable_ids.insert(row.stable_item_id.as_str()) {
                    return Err(fail("duplicate network summary stable_item_id"));
                }
            }
            Ok(())
        }
    }
}

fn network_content_key(row: &IntelNetworkUiRow) -> (&str, &str, &str, &str) {
    (
        row.owner_country.as_str(),
        row.target_country.as_str(),
        row.network_id.as_str(),
        row.stable_item_id.as_str(),
    )
}

fn ordered<T: StableItem>(value: &'_ ProducerValue<MeasuredList<T>>) -> Result<Vec<&T>> {
    let list = match value {
        ProducerValue::Measured(v) => v,
        ProducerValue::Unobtained { .. } => return Ok(Vec::new()),
    };
    list.collection.validate()?;
    let by_id: BTreeMap<&str, &T> = list
        .rows
        .iter()
        .map(|row| (row.stable_item_id(), row))
        .collect();
    list.collection
        .ordered_item_ids
        .iter()
        .map(|id| {
            by_id
                .get(id.as_str())
                .copied()
                .ok_or_else(|| fail("ordered id has no producer row"))
        })
        .collect()
}

fn validate_visibility(
    rows: &[VisibilityGate],
) -> Result<BTreeMap<IntelligenceSurface, VisibilityDisposition>> {
    let mut out = BTreeMap::new();
    for row in rows {
        if out.insert(row.surface, row.disposition).is_some() {
            return Err(fail("duplicate visibility surface"));
        }
        if row.disposition != VisibilityDisposition::Visible {
            nonempty(row.reason.as_deref().unwrap_or(""), "visibility reason")?;
        }
    }
    let expected: BTreeSet<_> = IntelligenceSurface::ALL.into_iter().collect();
    if out.keys().copied().collect::<BTreeSet<_>>() != expected {
        return Err(fail(
            "visibility gates do not cover every intelligence surface",
        ));
    }
    Ok(out)
}

fn require_available(
    gates: &BTreeMap<IntelligenceSurface, VisibilityDisposition>,
    surface: IntelligenceSurface,
    measured: bool,
) -> Result<()> {
    let unavailable = gates[&surface] == VisibilityDisposition::Unavailable;
    if measured == unavailable {
        return Err(fail("producer availability disagrees with visibility gate"));
    }
    Ok(())
}
fn fixed(value: i64, label: &str) -> Result<()> {
    if !(0..=FIXED_ONE).contains(&value) {
        Err(fail(format!("{label} outside fixed range")))
    } else {
        Ok(())
    }
}
fn owner(actual: &str, expected: &str, label: &str) -> Result<()> {
    if actual != expected {
        Err(fail(format!("{label} owner mismatch")))
    } else {
        Ok(())
    }
}
fn unique_nonempty(values: &[String], label: &str) -> Result<()> {
    let mut set = BTreeSet::new();
    for value in values {
        nonempty(value, label)?;
        if !set.insert(value) {
            return Err(fail(format!("duplicate {label}")));
        }
    }
    Ok(())
}
fn nonempty(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty() {
        Err(fail(format!("{label} must be nonempty")))
    } else {
        Ok(())
    }
}
fn fail(message: impl Into<String>) -> GameUiError {
    GameUiError::new(
        ErrorKind::Other,
        format!("XUI14_FAIL_CLOSED: {}", message.into()),
    )
}
