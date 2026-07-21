//! XUI14 fail-closed intelligence/operative UI admission.
//!
//! Static GUI and definition files identify chrome and catalog entries, while
//! save/domain-53 data supplies gameplay rows. Engine-authored lists carry the
//! exact XUI03 dynamic-collection trace used by their producer. Intel networks
//! are the deliberate exception: the installed a729 UI has no network
//! collection, so replay exposes a product-authored deterministic summary and
//! never labels its stable content-key order as engine UI order.

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
use replay_core::SelectedPresentReceipt;
use std::collections::{BTreeMap, BTreeSet};

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
/// upgrade rows. This does not activate the producer or replace the required
/// XUI03 materialization trace; it is the pointer-free identity/order seam the
/// future domain-53 fold must satisfy before that trace can be admitted.
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
            return Err(fail("cipher controller references an unjoined semantic row"));
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
                OperationStatus::Running
                | OperationStatus::Finished
                | OperationStatus::Aborted => {
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
            match (row.captured, row.captured_by.as_deref()) {
                (true, Some(country)) => nonempty(country, "captured_by")?,
                (true, None) => return Err(fail("captured operative lacks captured_by")),
                (false, Some(_)) => return Err(fail("uncaptured operative carries captured_by")),
                (false, None) => {}
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
        return Err(fail("selected-Present companion semantic identity mismatch"));
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
        return Err(fail("operative or operation controller order companion mismatch"));
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
        ProducerValue::Unobtained { .. }
            if !admitted.operative_mission_products().is_empty() =>
        {
            return Err(fail("unobtained operatives carry mission product rows"));
        }
        ProducerValue::Unobtained { .. } => {}
    }

    let companion_gates = exact.visibility.iter().map(|gate| {
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
    }).collect::<BTreeMap<_,_>>();
    for gate in &trace.visibility {
        let Some((disposition, reason)) = companion_gates.get(&gate.surface).copied() else {
            return Err(fail("selected-Present companion visibility surface missing"));
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
            if summary.order_authority
                != NetworkSummaryOrderAuthority::ProductStableContentKeyV1
            {
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
