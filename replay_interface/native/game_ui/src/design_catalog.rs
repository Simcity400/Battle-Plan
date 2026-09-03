
use crate::readonly_design_peace::DesignerViewKind;
use crate::stored_country_selection::StoredCountrySelection;
use replay_core::NamedCountry;
use replay_core::{
    ObserverDesignerCandidate, ObserverDesignerCountryFilterRow, ObserverDesignerFamily,
    WorldFrame,
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
    UnsupportedFamily { family: DesignerViewKind },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverDesignerViewState {
    pub family: DesignerViewKind,
    pub anchor_design_id: String,
    pub anchor_catalogue_hour: i64,
    /// The country the anchor belongs to, as a SELECTION rather than a
    /// spelling: it is written from what the user selected and read only to
    /// compare against a name this frame's census already admitted.
    pub anchor_catalogue_owner_tag: StoredCountrySelection,
    pub anchor_catalogue_family: DesignerViewKind,
    pub selected_design_id: Option<String>,
    pub our_filter_enabled: bool,
    pub foreign_filter_enabled: bool,
    pub obsolete_filter_enabled: bool,
    pub excluded_country_filter_tags: Vec<String>,
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

pub fn resolve_observer_designer_view_state(
    frame: &WorldFrame,
    owner_tag: NamedCountry<'_>,
    viewer: &ObserverDesignerViewState,
) -> Result<
    (ObserverDesignerViewState, AdmittedObserverDesignerView),
    ObserverDesignerViewRejection,
> {
    let context_is_current = viewer.anchor_catalogue_hour == frame.hour_index()
        && viewer
            .anchor_catalogue_owner_tag
            .is_ignoring_ascii_case(owner_tag.tag())
        && viewer.anchor_catalogue_family == viewer.family;
    let mut resolved = viewer.clone();
    if !context_is_current {
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
    resolved.anchor_catalogue_owner_tag = StoredCountrySelection::of(owner_tag.tag());
    resolved.anchor_catalogue_family = viewer.family;
    Ok((resolved, admitted))
}

pub fn admit_observer_designer_view(
    frame: &WorldFrame,
    owner_tag: NamedCountry<'_>,
    viewer: &ObserverDesignerViewState,
) -> Result<AdmittedObserverDesignerView, ObserverDesignerViewRejection> {
    if owner_tag.trim().is_empty() {
        return Err(ObserverDesignerViewRejection::EmptyOwnerTag);
    }
    // The viewer's own selection, which the recording never vouched for: a key
    // two ids reach anchors no catalogue, and `resolve_observer_designer_view_state`
    // reaches this on both of its paths.
    if frame.country_key_is_contended(owner_tag.tag()) {
        return Err(ObserverDesignerViewRejection::CatalogueUnavailable {
            hour: frame.hour_index(),
            reason: format!(
                "owner tag {owner_tag:?} is a contended key this hour, so it names no country"
            ),
        });
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
            owner_tag.tag(),
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
