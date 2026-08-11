//! Replay-owned observer routes for semantic panels.
//!
//! These models deliberately stop before pixels. They bind the viewer's local
//! country selection to complete replay domains and the effective game-file
//! registry, while preserving unavailable engine leaves as typed dispositions.
//! Packet-specific shipping compositors apply the final presentation carriers.
//! Nothing in this module reads recorder interaction state, a
//! selected-Present stream, or a recorder-owned country.

use crate::content_interpreter::Interpreted;
use crate::readonly_design_peace::{
    admit_peace_conference, peace_conference_snapshot_from_world, AdmittedPeaceConference,
    Xui08Rejection,
};
use crate::scripted_gui_runtime::{
    captured_inputs_for, evaluate_registered_gui, EvaluatedProperty, ScriptedGuiEvaluation,
    ScriptedGuiRegistry, Truth,
};
use crate::static_research::{PoliticsIdeaCatalogLabels, StaticResearchUiCatalog};
use crate::{GameUiError, Result};
use replay_core::{
    ContentLayerKind, Leaf, PoliticsDetailView, ScriptedGuiView, UsedContentKey, ViewStatus,
    WorldFrame,
};
use std::collections::{BTreeMap, BTreeSet};

/// Legacy PCS1 semantic helper. Shipping XUI08 uses the strict D97/PCS3
/// presentation carrier; this helper remains fail-closed for active PCS1 data
/// and is not product pixel authority.
pub fn admit_peace_observer_panel(
    frame: &WorldFrame,
) -> std::result::Result<AdmittedPeaceConference, Xui08Rejection> {
    let snapshot = peace_conference_snapshot_from_world(frame)?;
    admit_peace_conference(&snapshot)
}

/// Exact semantic work that the replay cannot lawfully synthesize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ObserverSemanticGap {
    /// d14: final engine-resolved politics tooltip/text atoms.
    D14PoliticsRuntimeText,
    /// d15: runtime-only politics widget state and ordered presentation leaves.
    D15PoliticsPresentation,
    /// d74: all-country power-balance presentation independent of panel-open
    /// and recorder-country state.
    D74AllCountryPowerBalance,
    /// d84: the exact post-owner current-manager snapshot or one of its
    /// required residual results is unavailable.
    D84ScriptedGuiManagerSnapshot,
}

/// A semantic route may be useful to an observer while exact presentation is
/// still blocked. Callers must never treat `Blocked` as permission to draw.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExactPresentationState {
    Ready,
    Blocked { gaps: BTreeSet<ObserverSemanticGap> },
}

impl ExactPresentationState {
    pub fn blocked(gaps: impl IntoIterator<Item = ObserverSemanticGap>) -> Self {
        Self::Blocked {
            gaps: gaps.into_iter().collect(),
        }
    }

    pub fn ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// XUI11 semantic politics route for the viewer-selected country.
///
/// `detail` retains the `Absent` / `Unobtained` / `Measured` trichotomy. The
/// static labels come from recorder-packaged, content-admitted catalogs rather
/// than a playback-machine fallback.
#[derive(Clone, Debug, PartialEq)]
pub struct PoliticsObserverPanel {
    pub country_tag: String,
    pub detail: PoliticsDetailView,
    pub idea_labels: PoliticsIdeaCatalogLabels,
    pub presentation: ExactPresentationState,
}

pub fn admit_politics_observer_panel(
    frame: &WorldFrame,
    viewer_country: &str,
    static_catalog: &StaticResearchUiCatalog,
) -> Result<PoliticsObserverPanel> {
    if viewer_country.trim().is_empty() {
        return Err(GameUiError::parse(
            "XUI11 viewer-selected country identity is empty",
        ));
    }
    if frame.country_identities_status() != ViewStatus::Present {
        return Err(GameUiError::missing(
            "XUI11 complete country identity domain is unavailable",
        ));
    }
    if frame.country_identity(viewer_country).is_none() {
        return Err(GameUiError::missing(format!(
            "XUI11 viewer-selected country {viewer_country:?} is absent from the complete identity domain"
        )));
    }
    if frame.politics_detail_status() != ViewStatus::Present {
        return Err(GameUiError::missing(
            "XUI11 complete politics-detail domain is unavailable",
        ));
    }
    let detail = frame
        .politics_detail(viewer_country)
        .ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI11 politics detail is absent for viewer-selected country {viewer_country:?}"
            ))
        })?
        .clone();
    let measured_tag_id =
        detail.tag_db_id.measured().copied().ok_or_else(|| {
            GameUiError::missing("XUI11 D18 country identity join is unavailable")
        })?;
    if measured_tag_id == 0 {
        return Err(GameUiError::parse(
            "XUI11 D18 country identity join is the invalid zero sentinel",
        ));
    }
    let idea_labels = static_catalog.bind_politics_ideas(&detail)?;
    Ok(PoliticsObserverPanel {
        country_tag: viewer_country.to_string(),
        detail,
        idea_labels,
        presentation: ExactPresentationState::blocked([
            ObserverSemanticGap::D14PoliticsRuntimeText,
            ObserverSemanticGap::D15PoliticsPresentation,
        ]),
    })
}

/// Viewer-relevant semantic row from the world-scoped power-balance carrier.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserverPowerBalanceRow {
    pub source_record_id: String,
    pub manager_ordinal: u32,
    pub template: String,
    pub value: f64,
    pub left_side: String,
    pub right_side: String,
    pub trending_side: String,
    pub active_range: String,
}

/// XUI18 observer route. A measured empty vector is a real hidden state.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerBalanceObserverPanel {
    pub country_tag: String,
    pub applicable: Vec<ObserverPowerBalanceRow>,
    pub presentation: ExactPresentationState,
}

pub fn admit_power_balance_observer_panel(
    frame: &WorldFrame,
    viewer_country: &str,
) -> Result<PowerBalanceObserverPanel> {
    if viewer_country.trim().is_empty() {
        return Err(GameUiError::parse(
            "XUI18 viewer-selected country identity is empty",
        ));
    }
    if frame.country_identities_status() != ViewStatus::Present
        || frame.country_identity(viewer_country).is_none()
    {
        return Err(GameUiError::missing(
            "XUI18 viewer-selected country is absent from the complete identity domain",
        ));
    }
    let state_view = frame.power_balance_state_exact();
    let presentation_view = frame.power_balance_presentation_exact();
    let state = match (state_view.status, state_view.state.as_ref()) {
        (ViewStatus::Present, Some(state)) => state,
        _ => {
            return Err(GameUiError::missing(
                "XUI18 complete all-country power-balance state is unavailable",
            ))
        }
    };
    let presentation = match (
        presentation_view.status,
        presentation_view.presentation.as_ref(),
    ) {
        (ViewStatus::Present, Some(presentation)) => presentation,
        _ => {
            return Err(GameUiError::missing(
                "XUI18 complete all-country power-balance presentation is unavailable",
            ))
        }
    };
    if presentation.frame_hour != state.frame_hour
        || presentation.state_semantic_receipt_sha256 != state.semantic_receipt_sha256
        || presentation.effective_content_manifest_sha256 != state.effective_content_manifest_sha256
    {
        return Err(GameUiError::parse(
            "XUI18 all-country state/presentation join disagrees",
        ));
    }
    let country = state
        .countries
        .iter()
        .find(|country| country.country_tag == viewer_country)
        .ok_or_else(|| GameUiError::missing("XUI18 selected country is absent from d74 roster"))?;
    let selected = state
        .records
        .iter()
        .filter(|record| record.member_tag_db_ids.contains(&country.tag_db_id))
        .min_by_key(|record| record.manager_ordinal);
    let presented = presentation
        .balances
        .iter()
        .find(|row| row.country_tag == viewer_country);
    if selected.is_none() && presented.is_none() {
        return Ok(PowerBalanceObserverPanel {
            country_tag: viewer_country.to_string(),
            applicable: Vec::new(),
            presentation: ExactPresentationState::Ready,
        });
    }
    let (record, presented) = selected.zip(presented).ok_or_else(|| {
        GameUiError::parse("XUI18 selected state and presentation membership disagree")
    })?;
    if record.balance_id != presented.balance_id
        || record.manager_ordinal != presented.manager_ordinal
        || record.active_range_id != presented.active_range_id
    {
        return Err(GameUiError::parse(
            "XUI18 selected state and presentation row disagree",
        ));
    }
    let value = record.value_raw_fixed5 as f64 / 100_000.0;
    if !(-1.0..=1.0).contains(&value) {
        return Err(GameUiError::parse(
            "XUI18 exact fixed5 value is out of range",
        ));
    }
    Ok(PowerBalanceObserverPanel {
        country_tag: viewer_country.to_string(),
        applicable: vec![ObserverPowerBalanceRow {
            source_record_id: record.source_record_id.clone(),
            manager_ordinal: record.manager_ordinal,
            template: record.balance_id.clone(),
            value,
            left_side: record.left_side_id.clone(),
            right_side: record.right_side_id.clone(),
            trending_side: record.trending_side_id.clone(),
            active_range: record.active_range_id.clone(),
        }],
        presentation: ExactPresentationState::Ready,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedGuiObserverRow {
    pub manager_order_index: u32,
    pub gui_id: String,
    pub source_file: String,
    pub context_type: Option<String>,
    pub window_name: Option<String>,
    pub evaluation: ScriptedGuiEvaluation,
    pub presentation: ExactPresentationState,
}

/// XUI22 route for the viewer-selected country.
///
/// Row order is the authenticated engine manager order. Source identity and
/// definitions come from the effective dynamic VFS. Individual evaluations
/// remain blocked when any runtime-only field is unavailable. A measured d84
/// envelope is the exact post-owner current-manager snapshot: its contiguous
/// manager order covers every row in that snapshot, so absence from the
/// selected-country slice is authoritative measured-empty rather than an
/// invented Cartesian country/definition product.
#[derive(Clone, Debug, PartialEq)]
pub struct ScriptedGuiObserverPanel {
    pub country_tag: String,
    pub registry_schema: String,
    pub effective_file_count: usize,
    pub runtime_authority: ScriptedGuiRuntimeAuthorityState,
    pub rows: Vec<ScriptedGuiObserverRow>,
    pub presentation: ExactPresentationState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScriptedGuiRuntimeAuthorityState {
    Absent,
    Unobtained,
    Present,
}

pub fn admit_scripted_gui_observer_panel(
    admitted_registry: &Interpreted<ScriptedGuiRegistry>,
    frame: &WorldFrame,
    viewer_country: &str,
) -> Result<ScriptedGuiObserverPanel> {
    if viewer_country.trim().is_empty() {
        return Err(GameUiError::parse(
            "XUI22 viewer-selected country identity is empty",
        ));
    }
    if frame.country_identities_status() != ViewStatus::Present
        || frame.country_identity(viewer_country).is_none()
    {
        return Err(GameUiError::missing(
            "XUI22 viewer-selected country is absent from the complete identity domain",
        ));
    }
    let registry = &admitted_registry.value;
    if !registry.production_admitted() {
        return Err(GameUiError::parse(format!(
            "XUI22 effective scripted-GUI registry is incomplete: {:?}",
            registry.source_issues
        )));
    }
    if admitted_registry.provenance.len() != registry.files {
        return Err(GameUiError::parse(
            "XUI22 effective scripted-GUI registry provenance/file count mismatch",
        ));
    }
    let mut admitted_content: BTreeMap<String, &UsedContentKey> = BTreeMap::new();
    for content in &admitted_registry.provenance {
        content.validate().map_err(|error| {
            GameUiError::parse(format!(
                "XUI22 invalid effective scripted-GUI content identity: {error}"
            ))
        })?;
        let path = content
            .normalized_game_relative_path
            .replace('\\', "/")
            .to_ascii_lowercase();
        if !path.starts_with("common/scripted_guis/")
            || !path.ends_with(".txt")
            || admitted_content.insert(path, content).is_some()
        {
            return Err(GameUiError::parse(
                "XUI22 effective scripted-GUI provenance path set is invalid",
            ));
        }
    }
    if registry.definitions.values().any(|definition| {
        !admitted_content.contains_key(
            &definition
                .source_file
                .replace('\\', "/")
                .to_ascii_lowercase(),
        )
    }) {
        return Err(GameUiError::missing(
            "XUI22 a scripted-GUI definition is not backed by admitted effective content",
        ));
    }
    let view: &ScriptedGuiView = frame.scripted_gui();
    if view.status != ViewStatus::Present {
        return Err(GameUiError::missing(
            "XUI22 scripted-GUI replay domain is unavailable",
        ));
    }
    let authority = match &view.runtime_authority {
        Leaf::Measured(authority) => authority,
        Leaf::Absent => {
            return Ok(ScriptedGuiObserverPanel {
                country_tag: viewer_country.to_string(),
                registry_schema: registry.schema.clone(),
                effective_file_count: registry.files,
                runtime_authority: ScriptedGuiRuntimeAuthorityState::Absent,
                rows: Vec::new(),
                presentation: ExactPresentationState::blocked([
                    ObserverSemanticGap::D84ScriptedGuiManagerSnapshot,
                ]),
            })
        }
        Leaf::Unobtained => {
            return Ok(ScriptedGuiObserverPanel {
                country_tag: viewer_country.to_string(),
                registry_schema: registry.schema.clone(),
                effective_file_count: registry.files,
                runtime_authority: ScriptedGuiRuntimeAuthorityState::Unobtained,
                rows: Vec::new(),
                presentation: ExactPresentationState::blocked([
                    ObserverSemanticGap::D84ScriptedGuiManagerSnapshot,
                ]),
            })
        }
    };

    let mut gui_ids = BTreeSet::new();
    let mut rows = Vec::new();
    for manager in authority
        .manager_order
        .iter()
        .filter(|row| row.country_tag == viewer_country)
    {
        if !gui_ids.insert(manager.gui_id.as_str()) {
            return Err(GameUiError::parse(format!(
                "XUI22 duplicate manager identity for country {viewer_country:?}: {:?}",
                manager.gui_id
            )));
        }
        let definition = registry.definitions.get(&manager.gui_id).ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI22 effective VFS has no definition for manager identity {:?}",
                manager.gui_id
            ))
        })?;
        let captured =
            captured_inputs_for(view, viewer_country, &manager.gui_id).ok_or_else(|| {
                GameUiError::missing(format!(
                    "XUI22 manager row has no runtime payload for {:?}",
                    manager.gui_id
                ))
            })?;
        let recorded = captured.definition.measured().ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI22 recorded definition identity unavailable for {:?}",
                manager.gui_id
            ))
        })?;
        let recorded_path = recorded
            .logical_path
            .replace('\\', "/")
            .to_ascii_lowercase();
        let content = admitted_content.get(&recorded_path).ok_or_else(|| {
            GameUiError::missing(format!(
                "XUI22 recorded definition path is not admitted for {:?}",
                manager.gui_id
            ))
        })?;
        if !recorded_path.eq_ignore_ascii_case(&definition.source_file.replace('\\', "/"))
            || recorded.winning_layer_ordinal != content.winning_layer_ordinal
            || recorded.layer_kind != content_layer_kind_token(content.layer_kind)
            || recorded.layer_id != content.layer_id
            || recorded.source_sha256 != content.source_sha256
            || recorded.payload_sha256 != content.payload_sha256
            || recorded.payload_bytes != content.payload_bytes
        {
            return Err(GameUiError::parse(format!(
                "XUI22 effective-VFS definition identity mismatch for {:?}",
                manager.gui_id
            )));
        }
        let evaluation = evaluate_registered_gui(registry, view, viewer_country, &manager.gui_id)?;
        let presentation = if evaluation_complete(&evaluation) {
            ExactPresentationState::Ready
        } else {
            ExactPresentationState::blocked([ObserverSemanticGap::D84ScriptedGuiManagerSnapshot])
        };
        rows.push(ScriptedGuiObserverRow {
            manager_order_index: manager.manager_order_index,
            gui_id: manager.gui_id.clone(),
            source_file: definition.source_file.clone(),
            context_type: definition.context_type.clone(),
            window_name: definition.window_name.clone(),
            evaluation,
            presentation,
        });
    }
    let presentation = scripted_gui_manager_presentation(&rows);
    Ok(ScriptedGuiObserverPanel {
        country_tag: viewer_country.to_string(),
        registry_schema: registry.schema.clone(),
        effective_file_count: registry.files,
        runtime_authority: ScriptedGuiRuntimeAuthorityState::Present,
        rows,
        // The authority parser already proved that manager_order is contiguous
        // and exactly covers every folded d84 row from the exact post-owner
        // current-manager snapshot. No Cartesian product is an engine object.
        presentation,
    })
}

fn scripted_gui_manager_presentation(rows: &[ScriptedGuiObserverRow]) -> ExactPresentationState {
    if rows
        .iter()
        .all(|row| row.presentation == ExactPresentationState::Ready)
    {
        // A measured empty selected-country slice is exact: d84's authority
        // already proves the full current-manager order for this hour.
        ExactPresentationState::Ready
    } else {
        ExactPresentationState::blocked([ObserverSemanticGap::D84ScriptedGuiManagerSnapshot])
    }
}

fn content_layer_kind_token(kind: ContentLayerKind) -> &'static str {
    match kind {
        ContentLayerKind::Base => "base",
        ContentLayerKind::IntegratedDlc => "integrated_dlc",
        ContentLayerKind::Dlc => "dlc",
        ContentLayerKind::Mod => "mod",
    }
}

fn evaluation_complete(evaluation: &ScriptedGuiEvaluation) -> bool {
    evaluation.visible != Truth::Unavailable
        && evaluation.enabled != Truth::Unavailable
        && evaluation
            .triggers
            .values()
            .all(|truth| *truth != Truth::Unavailable)
        && evaluation
            .properties
            .values()
            .all(|property| !matches!(property, EvaluatedProperty::Unavailable))
        && evaluation
            .effect_enabled
            .values()
            .all(|truth| *truth != Truth::Unavailable)
}

#[cfg(test)]
mod xui22_manager_snapshot_tests {
    use super::*;

    fn row(presentation: ExactPresentationState) -> ScriptedGuiObserverRow {
        ScriptedGuiObserverRow {
            manager_order_index: 0,
            gui_id: "fixture".into(),
            source_file: "common/scripted_guis/fixture.txt".into(),
            context_type: Some("decision_category".into()),
            window_name: Some("fixture_window".into()),
            evaluation: ScriptedGuiEvaluation {
                visible: Truth::True,
                enabled: Truth::True,
                triggers: BTreeMap::new(),
                properties: BTreeMap::new(),
                effect_enabled: BTreeMap::new(),
            },
            presentation,
        }
    }

    #[test]
    fn measured_empty_selected_country_is_ready() {
        assert_eq!(
            scripted_gui_manager_presentation(&[]),
            ExactPresentationState::Ready
        );
    }

    #[test]
    fn every_manager_row_must_have_complete_materialized_results() {
        assert_eq!(
            scripted_gui_manager_presentation(&[row(ExactPresentationState::Ready)]),
            ExactPresentationState::Ready
        );
        assert_eq!(
            scripted_gui_manager_presentation(&[row(ExactPresentationState::blocked([
                ObserverSemanticGap::D84ScriptedGuiManagerSnapshot,
            ]))]),
            ExactPresentationState::blocked([
                ObserverSemanticGap::D84ScriptedGuiManagerSnapshot,
            ])
        );
    }
}
