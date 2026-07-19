//! XUI14 fail-closed intelligence/operative UI admission.
//!
//! Static GUI and definition files identify chrome and catalog entries, while
//! save/domain-53 data supplies gameplay rows. Neither proves controller
//! visibility or list order. Every measured list therefore carries the exact
//! XUI03 dynamic-collection trace used by the engine-facing producer.

use crate::dynamic_collection::DynamicCollectionTrace;
use crate::{ErrorKind, GameUiError, Result};
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
        target_country: Option<String>,
        target_state: Option<u32>,
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
pub struct IntelNetworkUiRow {
    pub stable_item_id: String,
    pub network_id: String,
    pub owner_country: String,
    pub target_country: String,
    pub operative_ids: Vec<String>,
    pub state_ids: Vec<u32>,
    pub strength_fixed: i64,
    pub national_coverage_fixed: i64,
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
    pub networks: ProducerValue<MeasuredList<IntelNetworkUiRow>>,
    pub visibility: Vec<VisibilityGate>,
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
    validate_family(
        &trace.networks,
        IntelligenceSurface::Networks,
        trace.frame_hour,
        &visibility,
    )?;

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
            if row.phase_count == 0 || row.phase_index >= row.phase_count || row.duration_hours == 0
            {
                return Err(fail("operation phase/duration is invalid"));
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
                    target_country,
                    target_state,
                } => {
                    nonempty(mission_id, "mission_id")?;
                    if target_country.is_none() && target_state.is_none() {
                        return Err(fail("mission target is absent"));
                    }
                    if let Some(country) = target_country {
                        nonempty(country, "mission target country")?;
                    }
                    if target_state == &Some(0) {
                        return Err(fail("mission target state is zero"));
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
            fixed(row.strength_fixed, "network strength")?;
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
        ordered(&self.trace.networks)
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
) -> Result<BTreeSet<String>> {
    match value {
        ProducerValue::Unobtained { reason } => {
            nonempty(reason, "family unobtained reason")?;
            require_available(gates, surface, false)?;
            Ok(BTreeSet::new())
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
            Ok(ids)
        }
    }
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
