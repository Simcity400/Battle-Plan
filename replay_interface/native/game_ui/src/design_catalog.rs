//! XUI13 design/template/variant production admission.
//!
//! Definitions may originate in resolved static artifacts or live extractor
//! rows, but display order is always an explicit producer snapshot. No token,
//! map, source-file, or localisation order is promoted as engine UI order.

use replay_core::world::{DesignCatalogContextView, ViewStatus};
use replay_core::{CurrentDesignCatalog, DesignCatalogJoinError, WorldFrame};
pub use replay_core::{
    DesignKind, DesignLifecycleState, DesignModule, DesignRecord, DesignStat, DesignUpgrade,
    DisplayOrderSnapshot, FieldCompleteness,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedDesignCatalog {
    ordered: Vec<DesignRecord>,
    by_id: BTreeMap<String, usize>,
}

impl AdmittedDesignCatalog {
    pub fn ordered(&self) -> &[DesignRecord] {
        &self.ordered
    }

    pub fn get(&self, design_id: &str) -> Option<&DesignRecord> {
        self.by_id
            .get(design_id)
            .and_then(|index| self.ordered.get(*index))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignCatalogRejection {
    MissingProvenance,
    MissingDesignId,
    DuplicateDesignId,
    UnknownCompleteness,
    MissingDefinitionIdentity,
    MissingOwner,
    MissingLifecycleState,
    InvalidModule,
    DuplicateModuleSlot,
    InvalidStat,
    DuplicateStat,
    InvalidUpgrade,
    DuplicateUpgrade,
    EmptyDisplayOrder,
    DuplicateDisplayOrderId,
    DisplayOrderUnknownId,
    DisplayOrderMissingId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedDesignerView {
    pub source_record_id: String,
    pub transition_sequence: u64,
    pub active: bool,
    pub family: String,
    pub current_country_tag_id: u32,
    pub selected_country_tag_ids: Vec<u32>,
    pub our_filter_enabled: bool,
    pub foreign_filter_enabled: bool,
    pub obsolete_filter_enabled: bool,
    pub ordered: Vec<DesignRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesignerViewRejection {
    MissingContext,
    InactiveRows,
    UnknownDesignId,
    NonVariantDesign,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CurrentDesignerAdmissionError {
    PortableJoin(DesignCatalogJoinError),
    Catalog(DesignCatalogRejection),
    View(DesignerViewRejection),
}

/// Production handoff: admit only the A27 records selected by d78 and retain
/// d78 as the sole ordering authority. The full A27 heap-scan vector and d59
/// authored-definition order never reach `admit_design_catalog` as UI order.
pub fn admit_current_designer_frame(
    frame: &WorldFrame,
) -> Result<AdmittedDesignerView, CurrentDesignerAdmissionError> {
    let current = frame
        .current_design_catalog()
        .map_err(CurrentDesignerAdmissionError::PortableJoin)?;
    admit_current_designer_catalog(&current, frame.design_catalog_context())
}

pub fn admit_current_designer_catalog(
    current: &CurrentDesignCatalog,
    context: &DesignCatalogContextView,
) -> Result<AdmittedDesignerView, CurrentDesignerAdmissionError> {
    let catalog = admit_design_catalog(&current.records, &current.display_order)
        .map_err(CurrentDesignerAdmissionError::Catalog)?;
    admit_designer_view(&catalog, context).map_err(CurrentDesignerAdmissionError::View)
}

/// Bind d78's exact filtered equipment-designer order to the admitted A27
/// catalog. The stable runtime identity is `id:70`; no token/name/map order is
/// consulted and a missing d78 context is unavailable, never an empty list.
pub fn admit_designer_view(
    catalog: &AdmittedDesignCatalog,
    view: &DesignCatalogContextView,
) -> Result<AdmittedDesignerView, DesignerViewRejection> {
    if view.status != ViewStatus::Present {
        return Err(DesignerViewRejection::MissingContext);
    }
    let context = view
        .context
        .as_ref()
        .ok_or(DesignerViewRejection::MissingContext)?;
    if !context.active && !context.ordered_design_ids.is_empty() {
        return Err(DesignerViewRejection::InactiveRows);
    }
    let mut ordered = Vec::with_capacity(context.ordered_design_ids.len());
    for id in &context.ordered_design_ids {
        let stable_id = format!("{id}:70");
        let row = catalog
            .get(&stable_id)
            .ok_or(DesignerViewRejection::UnknownDesignId)?;
        if row.kind == DesignKind::DivisionTemplate {
            return Err(DesignerViewRejection::NonVariantDesign);
        }
        ordered.push(row.clone());
    }
    Ok(AdmittedDesignerView {
        source_record_id: context.completeness_record_identity.clone(),
        transition_sequence: context.transition_sequence,
        active: context.active,
        family: context.family.clone(),
        current_country_tag_id: context.current_country_tag_id,
        selected_country_tag_ids: context.selected_country_tag_ids.clone(),
        our_filter_enabled: context.our_filter_enabled,
        foreign_filter_enabled: context.foreign_filter_enabled,
        obsolete_filter_enabled: context.obsolete_filter_enabled,
        ordered,
    })
}

fn nonempty(value: &str) -> bool {
    !value.trim().is_empty()
}

fn complete_or_not_applicable(value: FieldCompleteness) -> bool {
    value != FieldCompleteness::Unknown
}

pub fn admit_design_catalog(
    records: &[DesignRecord],
    display_order: &DisplayOrderSnapshot,
) -> Result<AdmittedDesignCatalog, DesignCatalogRejection> {
    if !nonempty(&display_order.source_record_id) {
        return Err(DesignCatalogRejection::MissingProvenance);
    }
    if display_order.completeness != FieldCompleteness::Complete {
        return Err(DesignCatalogRejection::UnknownCompleteness);
    }
    if !records.is_empty() && display_order.ordered_design_ids.is_empty() {
        return Err(DesignCatalogRejection::EmptyDisplayOrder);
    }

    let mut records_by_id = BTreeMap::new();
    for record in records {
        if !nonempty(&record.source_record_id) {
            return Err(DesignCatalogRejection::MissingProvenance);
        }
        if !nonempty(&record.design_id) {
            return Err(DesignCatalogRejection::MissingDesignId);
        }
        if records_by_id
            .insert(record.design_id.clone(), record)
            .is_some()
        {
            return Err(DesignCatalogRejection::DuplicateDesignId);
        }
        if !complete_or_not_applicable(record.definition_completeness)
            || !complete_or_not_applicable(record.owner_completeness)
            || !complete_or_not_applicable(record.lifecycle_completeness)
            || !complete_or_not_applicable(record.module_completeness)
            || !complete_or_not_applicable(record.stat_completeness)
            || !complete_or_not_applicable(record.upgrade_completeness)
        {
            return Err(DesignCatalogRejection::UnknownCompleteness);
        }
        if record.definition_completeness == FieldCompleteness::Complete
            && record
                .name
                .as_deref()
                .filter(|value| nonempty(value))
                .is_none()
            && record
                .archetype
                .as_deref()
                .filter(|value| nonempty(value))
                .is_none()
        {
            return Err(DesignCatalogRejection::MissingDefinitionIdentity);
        }
        if record.owner_completeness == FieldCompleteness::Complete
            && record
                .owner
                .as_deref()
                .filter(|value| nonempty(value))
                .is_none()
        {
            return Err(DesignCatalogRejection::MissingOwner);
        }
        if record.lifecycle_completeness == FieldCompleteness::Complete
            && record.lifecycle_state.is_none()
        {
            return Err(DesignCatalogRejection::MissingLifecycleState);
        }

        let mut slots = BTreeSet::new();
        for module in &record.modules {
            if !nonempty(&module.source_record_id)
                || !nonempty(&module.slot_id)
                || !nonempty(&module.module_token)
            {
                return Err(DesignCatalogRejection::InvalidModule);
            }
            if !slots.insert(module.slot_id.clone()) {
                return Err(DesignCatalogRejection::DuplicateModuleSlot);
            }
        }

        let mut stats = BTreeSet::new();
        for stat in &record.stats {
            if !nonempty(&stat.source_record_id)
                || !nonempty(&stat.stat_id)
                || !stat.value.is_finite()
            {
                return Err(DesignCatalogRejection::InvalidStat);
            }
            if !stats.insert(stat.stat_id.clone()) {
                return Err(DesignCatalogRejection::DuplicateStat);
            }
        }

        let mut upgrades = BTreeSet::new();
        for upgrade in &record.upgrades {
            if !nonempty(&upgrade.source_record_id)
                || !nonempty(&upgrade.upgrade_id)
                || upgrade.level < 0
            {
                return Err(DesignCatalogRejection::InvalidUpgrade);
            }
            if !upgrades.insert(upgrade.upgrade_id.clone()) {
                return Err(DesignCatalogRejection::DuplicateUpgrade);
            }
        }
    }

    let mut seen = BTreeSet::new();
    let mut ordered = Vec::with_capacity(records.len());
    for design_id in &display_order.ordered_design_ids {
        if !seen.insert(design_id.clone()) {
            return Err(DesignCatalogRejection::DuplicateDisplayOrderId);
        }
        let Some(record) = records_by_id.get(design_id) else {
            return Err(DesignCatalogRejection::DisplayOrderUnknownId);
        };
        ordered.push((*record).clone());
    }
    if seen.len() != records_by_id.len() {
        return Err(DesignCatalogRejection::DisplayOrderMissingId);
    }

    let by_id = ordered
        .iter()
        .enumerate()
        .map(|(index, record)| (record.design_id.clone(), index))
        .collect();
    Ok(AdmittedDesignCatalog { ordered, by_id })
}
