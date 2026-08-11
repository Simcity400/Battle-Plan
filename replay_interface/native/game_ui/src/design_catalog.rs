//! XUI13 design/template/variant production admission.
//!
//! Definitions may originate in resolved static artifacts or live extractor
//! rows. The legacy d78 path below retains its captured player-local display
//! order; the observer path uses an explicit deterministic replay-side order.

use crate::readonly_design_peace::DesignerViewKind;
use replay_core::world::{DesignCatalogContextView, ViewStatus};
use replay_core::{
    CurrentDesignCatalog, DesignCatalogJoinError, ObserverDesignerCandidate,
    ObserverDesignerCountryFilterRow, ObserverDesignerFamily, WorldFrame,
};
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

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedObserverDesignCatalogue {
    owner_tag: String,
    family: DesignerViewKind,
    ordered: Vec<DesignRecord>,
}

impl AdmittedObserverDesignCatalogue {
    pub fn owner_tag(&self) -> &str {
        &self.owner_tag
    }

    pub fn family(&self) -> DesignerViewKind {
        self.family
    }

    pub fn ordered(&self) -> &[DesignRecord] {
        &self.ordered
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverDesignCatalogueRejection {
    EmptyOwnerTag,
    MissingCatalogue,
    CatalogueHourUnavailable { hour: i64, reason: String },
    UnknownOwnerCompleteness { design_id: String },
    UnknownDefinitionCompleteness { design_id: String },
    /// A27 proves only runtime type 70 for equipment variants. It cannot split
    /// that census into tank, aircraft, or ship families without a separate
    /// authenticated authored-definition join.
    UnsupportedFamily { family: DesignerViewKind },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDesignerViewState {
    pub family: DesignerViewKind,
    pub anchor_design_id: String,
    /// Last replay hour at which the app proved this anchor against the exact
    /// family/country controller context. This is reconciliation state only;
    /// the compositor still performs full admission independently.
    pub anchor_catalogue_hour: i64,
    /// Owner and family paired with `anchor_catalogue_hour`. Together these
    /// fields prevent a same-hour country or family change from reusing a
    /// stale anchor-admission cache.
    pub anchor_catalogue_owner_tag: String,
    pub anchor_catalogue_family: DesignerViewKind,
    pub selected_design_id: Option<String>,
    pub our_filter_enabled: bool,
    pub foreign_filter_enabled: bool,
    pub obsolete_filter_enabled: bool,
    /// Controller +0xf418 country IDs excluded from the catalogue. Countries
    /// absent from this set are the selected/enabled strip rows.
    pub excluded_country_filter_tags: Vec<String>,
    /// Viewer-owned wheel ticks. The compositor multiplies this by the
    /// effective GUI file's authored `scroll_wheel_factor`.
    pub scroll_wheel_steps: u32,
    pub country_scroll_wheel_steps: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedObserverDesignerView {
    source_record_id: String,
    owner_tag: String,
    family: DesignerViewKind,
    anchor_design_id: String,
    ordered: Vec<ObserverDesignerCandidate>,
    country_filter_rows: Vec<ObserverDesignerCountryFilterRow>,
    excluded_country_filter_tags: Vec<String>,
    selected_index: Option<usize>,
    scroll_wheel_steps: u32,
    country_scroll_wheel_steps: u32,
}

impl AdmittedObserverDesignerView {
    pub fn source_record_id(&self) -> &str {
        &self.source_record_id
    }

    pub fn owner_tag(&self) -> &str {
        &self.owner_tag
    }

    pub fn family(&self) -> DesignerViewKind {
        self.family
    }

    pub fn anchor_design_id(&self) -> &str {
        &self.anchor_design_id
    }

    pub fn ordered(&self) -> &[ObserverDesignerCandidate] {
        &self.ordered
    }

    pub fn country_filter_rows(&self) -> &[ObserverDesignerCountryFilterRow] {
        &self.country_filter_rows
    }

    pub fn excluded_country_filter_tags(&self) -> &[String] {
        &self.excluded_country_filter_tags
    }

    pub fn selected(&self) -> Option<&ObserverDesignerCandidate> {
        self.selected_index
            .and_then(|index| self.ordered.get(index))
    }

    pub fn scroll_wheel_steps(&self) -> u32 {
        self.scroll_wheel_steps
    }

    pub fn country_scroll_wheel_steps(&self) -> u32 {
        self.country_scroll_wheel_steps
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObserverDesignerViewRejection {
    EmptyOwnerTag,
    MissingCatalogue,
    UnsupportedFamily(DesignerViewKind),
    CatalogueUnavailable { hour: i64, reason: String },
    UnknownSelection(String),
}

/// Resolve an empty viewer anchor from the complete native A27-v3 family
/// census. The first successful identity is the engine-native source-order
/// anchor, never a filename/name heuristic. If the complete family has no
/// candidates, the empty anchor is retained and admitted as measured-empty.
pub fn resolve_observer_designer_view_state(
    frame: &WorldFrame,
    owner_tag: &str,
    viewer: &ObserverDesignerViewState,
) -> Result<
    (ObserverDesignerViewState, AdmittedObserverDesignerView),
    ObserverDesignerViewRejection,
> {
    let context_is_current = viewer.anchor_catalogue_hour == frame.hour_index()
        && viewer
            .anchor_catalogue_owner_tag
            .eq_ignore_ascii_case(owner_tag)
        && viewer.anchor_catalogue_family == viewer.family;
    let mut resolved = viewer.clone();
    if !context_is_current {
        // A27 is a complete per-hour native census. A cached anchor or
        // selection is therefore evidence only for the exact hour, owner, and
        // family that admitted it. Re-enter through the empty-anchor law to
        // select the new hour's first native-order family member, and retain a
        // user selection only when the new complete membership still proves
        // it exists.
        resolved.anchor_design_id.clear();
        resolved.selected_design_id = None;
        let current = admit_observer_designer_view(frame, owner_tag, &resolved)?;
        resolved.anchor_design_id = current.anchor_design_id().to_string();
        resolved.selected_design_id = viewer.selected_design_id.as_ref().and_then(|selected| {
            current
                .ordered()
                .iter()
                .any(|candidate| candidate.record.design_id == *selected)
                .then(|| selected.clone())
        });
    }
    let admitted = admit_observer_designer_view(frame, owner_tag, &resolved)?;
    resolved.anchor_design_id = admitted.anchor_design_id().to_string();
    resolved.anchor_catalogue_hour = frame.hour_index();
    resolved.anchor_catalogue_owner_tag = owner_tag.to_string();
    resolved.anchor_catalogue_family = viewer.family;
    Ok((resolved, admitted))
}

pub fn admit_observer_designer_view(
    frame: &WorldFrame,
    owner_tag: &str,
    viewer: &ObserverDesignerViewState,
) -> Result<AdmittedObserverDesignerView, ObserverDesignerViewRejection> {
    if owner_tag.trim().is_empty() {
        return Err(ObserverDesignerViewRejection::EmptyOwnerTag);
    }
    let family = match viewer.family {
        DesignerViewKind::Ship => ObserverDesignerFamily::Ship,
        DesignerViewKind::Tank => ObserverDesignerFamily::Tank,
        DesignerViewKind::Plane => ObserverDesignerFamily::Plane,
        unsupported => {
            return Err(ObserverDesignerViewRejection::UnsupportedFamily(
                unsupported,
            ))
        }
    };
    let catalogue = frame
        .design_catalog()
        .ok_or(ObserverDesignerViewRejection::MissingCatalogue)?;
    let hour = u64::try_from(frame.hour_index()).map_err(|_| {
        ObserverDesignerViewRejection::CatalogueUnavailable {
            hour: frame.hour_index(),
            reason: "negative replay hour cannot address A27".into(),
        }
    })?;
    let admitted = catalogue
        .observer_designer_catalog_at_hour(
            hour,
            &viewer.anchor_design_id,
            owner_tag,
            family,
            viewer.our_filter_enabled,
            viewer.foreign_filter_enabled,
            viewer.obsolete_filter_enabled,
            &viewer.excluded_country_filter_tags,
        )
        .map_err(
            |reason| ObserverDesignerViewRejection::CatalogueUnavailable {
                hour: frame.hour_index(),
                reason,
            },
        )?;
    let selected_index = match viewer.selected_design_id.as_deref() {
        None => None,
        Some(selected) => Some(
            admitted
                .ordered
                .iter()
                .position(|candidate| candidate.record.design_id == selected)
                .ok_or_else(|| {
                    ObserverDesignerViewRejection::UnknownSelection(selected.to_string())
                })?,
        ),
    };
    Ok(AdmittedObserverDesignerView {
        source_record_id: admitted.source_record_id,
        owner_tag: owner_tag.to_string(),
        family: viewer.family,
        anchor_design_id: admitted.anchor_design_id,
        ordered: admitted.ordered,
        country_filter_rows: admitted.country_filter_rows,
        excluded_country_filter_tags: viewer.excluded_country_filter_tags.clone(),
        selected_index,
        scroll_wheel_steps: viewer.scroll_wheel_steps,
        country_scroll_wheel_steps: viewer.country_scroll_wheel_steps,
    })
}

/// Admit the viewer-selected country's complete A27 catalogue for one designer family.
///
/// This observer path deliberately does not consult d78 or d79. It filters the unfiltered
/// portable A27 census by the viewer-selected owner and the kinds A27 authenticates, then orders
/// matching rows by `design_id`. Division is authenticated by runtime type 52; Equipment and
/// VariantUpgrade use A27's lossless generic runtime-type-70 census. Tank, Plane, and Ship refuse
/// until a separate authenticated authored-definition join can subtype those type-70 rows.
/// That order is a deterministic replay law, not an engine display order: the
/// engine order previously captured by d78 was already filtered by one past player's local
/// designer toggles and is not authoritative for a new observer.
///
/// A catalogue with no matching rows is measured-empty and returns `Ok` with an empty slice.
/// Missing authority, an unserved hour, and unknown owner/definition completeness remain typed
/// refusals.
pub fn admit_observer_design_catalogue(
    frame: &WorldFrame,
    owner_tag: &str,
    family: DesignerViewKind,
) -> Result<AdmittedObserverDesignCatalogue, ObserverDesignCatalogueRejection> {
    if owner_tag.trim().is_empty() {
        return Err(ObserverDesignCatalogueRejection::EmptyOwnerTag);
    }
    let catalogue = frame
        .design_catalog()
        .ok_or(ObserverDesignCatalogueRejection::MissingCatalogue)?;
    let hour = u64::try_from(frame.hour_index()).map_err(|_| {
        ObserverDesignCatalogueRejection::CatalogueHourUnavailable {
            hour: frame.hour_index(),
            reason: "negative replay hour cannot address the A27 catalogue".into(),
        }
    })?;
    let records = catalogue.records_at_hour(hour).map_err(|reason| {
        ObserverDesignCatalogueRejection::CatalogueHourUnavailable {
            hour: frame.hour_index(),
            reason,
        }
    })?;
    admit_observer_design_catalogue_records(records, owner_tag, family)
}

fn admit_observer_design_catalogue_records(
    records: Vec<DesignRecord>,
    owner_tag: &str,
    family: DesignerViewKind,
) -> Result<AdmittedObserverDesignCatalogue, ObserverDesignCatalogueRejection> {
    if owner_tag.trim().is_empty() {
        return Err(ObserverDesignCatalogueRejection::EmptyOwnerTag);
    }
    let expected_kind = match family {
        DesignerViewKind::Division => DesignKind::DivisionTemplate,
        DesignerViewKind::Equipment | DesignerViewKind::VariantUpgrade => {
            DesignKind::EquipmentVariant
        }
        DesignerViewKind::Tank | DesignerViewKind::Plane | DesignerViewKind::Ship => {
            return Err(ObserverDesignCatalogueRejection::UnsupportedFamily { family });
        }
    };
    for record in &records {
        if record.owner_completeness == FieldCompleteness::Unknown {
            return Err(ObserverDesignCatalogueRejection::UnknownOwnerCompleteness {
                design_id: record.design_id.clone(),
            });
        }
        if record.definition_completeness == FieldCompleteness::Unknown {
            return Err(
                ObserverDesignCatalogueRejection::UnknownDefinitionCompleteness {
                    design_id: record.design_id.clone(),
                },
            );
        }
    }
    let mut ordered = records
        .into_iter()
        .filter(|record| record.kind == expected_kind && record.owner.as_deref() == Some(owner_tag))
        .collect::<Vec<_>>();
    ordered.sort_by(|left, right| left.design_id.cmp(&right.design_id));
    Ok(AdmittedObserverDesignCatalogue {
        owner_tag: owner_tag.into(),
        family,
        ordered,
    })
}

/// Legacy player-local handoff: admit only the A27 records selected by d78 and retain d78 as
/// ordering authority. This carries the recording player's filter state and is not the observer
/// catalogue path.
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

/// Bind d78's exact filtered equipment-designer order to the admitted A27 catalog.
///
/// This legacy admission carries the recording player's filter state and is not the observer
/// catalogue path. The stable runtime identity is `id:70`; no token/name/map order is consulted
/// and a missing d78 context is unavailable, never an empty list.
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

#[cfg(test)]
mod observer_tests {
    use super::*;

    fn record(id: &str, owner: &str, kind: DesignKind) -> DesignRecord {
        DesignRecord {
            design_id: id.into(),
            source_record_id: format!("fixture:{id}"),
            kind,
            owner: Some(owner.into()),
            name: Some(format!("name:{id}")),
            archetype: None,
            lifecycle_state: Some(DesignLifecycleState::Active),
            modules: Vec::new(),
            stats: Vec::new(),
            upgrades: Vec::new(),
            definition_completeness: FieldCompleteness::Complete,
            owner_completeness: FieldCompleteness::Complete,
            lifecycle_completeness: FieldCompleteness::Complete,
            module_completeness: FieldCompleteness::NotApplicable,
            stat_completeness: FieldCompleteness::NotApplicable,
            upgrade_completeness: FieldCompleteness::NotApplicable,
        }
    }

    #[test]
    fn observer_refusals_precede_measured_empty() {
        assert_eq!(
            admit_observer_design_catalogue_records(Vec::new(), "", DesignerViewKind::Equipment),
            Err(ObserverDesignCatalogueRejection::EmptyOwnerTag)
        );

        let mut unknown_owner = record("unknown-owner", "GER", DesignKind::EquipmentVariant);
        unknown_owner.owner_completeness = FieldCompleteness::Unknown;
        assert_eq!(
            admit_observer_design_catalogue_records(
                vec![unknown_owner],
                "GER",
                DesignerViewKind::Equipment
            ),
            Err(ObserverDesignCatalogueRejection::UnknownOwnerCompleteness {
                design_id: "unknown-owner".into()
            })
        );

        let mut unknown_definition =
            record("unknown-definition", "GER", DesignKind::EquipmentVariant);
        unknown_definition.definition_completeness = FieldCompleteness::Unknown;
        assert_eq!(
            admit_observer_design_catalogue_records(
                vec![unknown_definition],
                "GER",
                DesignerViewKind::Equipment
            ),
            Err(
                ObserverDesignCatalogueRejection::UnknownDefinitionCompleteness {
                    design_id: "unknown-definition".into()
                }
            )
        );
    }

    #[test]
    fn observer_filters_owner_and_kind_and_preserves_measured_empty() {
        let records = vec![
            record("z", "GER", DesignKind::EquipmentVariant),
            record("division", "GER", DesignKind::DivisionTemplate),
            record("foreign", "FRA", DesignKind::EquipmentVariant),
            record("a", "GER", DesignKind::EquipmentVariant),
        ];
        let admitted = admit_observer_design_catalogue_records(
            records.clone(),
            "GER",
            DesignerViewKind::Equipment,
        )
        .unwrap();
        assert_eq!(
            admitted
                .ordered()
                .iter()
                .map(|record| record.design_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "z"]
        );

        let measured_empty =
            admit_observer_design_catalogue_records(records, "ITA", DesignerViewKind::Equipment)
                .unwrap();
        assert!(measured_empty.ordered().is_empty());
    }

    #[test]
    fn observer_refuses_type_70_subclassification_without_an_authenticated_join() {
        for family in [
            DesignerViewKind::Tank,
            DesignerViewKind::Plane,
            DesignerViewKind::Ship,
        ] {
            assert_eq!(
                admit_observer_design_catalogue_records(
                    vec![record("type-70", "GER", DesignKind::EquipmentVariant)],
                    "GER",
                    family,
                ),
                Err(ObserverDesignCatalogueRejection::UnsupportedFamily { family })
            );
        }
    }
}
