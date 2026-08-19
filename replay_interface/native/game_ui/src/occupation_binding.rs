
use crate::compose::ComposeOptions;
use crate::assets::AssetResolver;
use crate::loc::Localisation;
use crate::paradox_text::parse_file_refusing_decode_damage;
use crate::{GameUiError, Result as UiResult};
use replay_core::{Leaf, OccupationRow, ViewStatus, WorldFrame};
use std::collections::{BTreeMap, BTreeSet};

pub const COUNTRY_OCCUPATION_GUI: &str = "interface/countryoccupationview.gui";
pub const GARRISON_LOG_GUI: &str = "interface/garrisonlogview.gui";
pub const COMBAT_LOG_GUI: &str = "interface/combatlogview.gui";
pub const COUNTRY_STATE_GUI: &str = "interface/countrystateview.gui";
pub const OCCUPATION_LAWS_SOURCE: &str = "common/occupation_laws/occupation_laws.txt";
pub const OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT: u32 = 0x3f;
pub const OCCUPATION_GARRISON_EQUIPMENT_FIXED_POINT_DIVISOR: i64 = 100_000;
const MAX_EXACT_F64_INTEGER: f64 = 9_007_199_254_740_992.0;

pub const OCCUPATION_COLLECTION_STATES: &str = "xui16/occupied_states";
pub const OCCUPATION_COLLECTION_ACTIVITY: &str = "xui16/garrison_activity";
pub const OCCUPATION_COLLECTION_EQUIPMENT_LOSSES: &str = "xui16/garrison_equipment_losses";
pub const OCCUPATION_COLLECTION_ALERTS: &str = "xui16/garrison_alerts";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverSemanticCollection {
    pub collection_id: &'static str,
    pub frame_hour: u64,
    pub context_country: Option<String>,
    pub rows: Vec<ObserverSemanticRow>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObserverSemanticRow {
    pub stable_item_id: String,
    pub source_ordinal: u32,
}

impl ObserverSemanticCollection {
    pub fn validate(&self, expected_id: &str, expected_hour: u64) -> bool {
        if self.collection_id != expected_id
            || self.frame_hour != expected_hour
            || self
                .context_country
                .as_deref()
                .is_some_and(|tag| tag.trim().is_empty())
        {
            return false;
        }
        let mut ids = BTreeSet::new();
        self.rows.iter().enumerate().all(|(ordinal, row)| {
            row.source_ordinal as usize == ordinal
                && !row.stable_item_id.trim().is_empty()
                && ids.insert(row.stable_item_id.as_str())
        })
    }

    pub fn ordered_item_ids(&self) -> impl Iterator<Item = &str> {
        self.rows.iter().map(|row| row.stable_item_id.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationObserverCollections {
    pub states: ObserverSemanticCollection,
    pub activities: ObserverSemanticCollection,
    pub equipment_losses: ObserverSemanticCollection,
    pub alerts: ObserverSemanticCollection,
}

pub type OccupationD66UiSupplement = replay_core::OccupationUiSupplementExact;
pub type OccupationD66ControllerSupplement =
    replay_core::OccupationUiControllerSupplementExact;
pub type OccupationD66FormattedStateText =
    replay_core::OccupationUiFormattedStateTextExact;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationLawDefinition {
    pub token: String,
    pub display_name: String,
    pub icon_frame: usize,
    pub gui_order: i64,
}

pub type GarrisonTemplateDefinition = replay_core::OccupationUiGarrisonTemplateExact;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationVisibleText {
    pub state_name: String,
    pub occupied_country_name: String,
    pub resistance_text: String,
    pub resistance_speed_text: String,
    pub compliance_text: String,
    pub compliance_speed_text: String,
    pub garrison_manpower_need_text: String,
    pub garrison_required_text: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GarrisonEquipmentRow {
    pub equipment_id: u32,
    pub equipment_type: u32,
    pub equipment_token: String,
    pub amount: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GarrisonActivityRow {
    pub stable_id: String,
    pub controller_tag: String,
    pub date_text: String,
    pub location_text: String,
    pub state_id: u32,
    pub occupied_country_tag: String,
    pub attack_blocked: bool,
    pub resistance_activity: Option<String>,
    pub lost_manpower: i64,
    pub lost_equipment: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GarrisonEquipmentLossRow {
    pub stable_id: String,
    pub equipment_id: u32,
    pub equipment_type: u32,
    pub equipment_token: String,
    pub display_name: String,
    pub amount_lost: f64,
    pub contributing_activity_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GarrisonShortageEquipmentRow {
    pub archetype_id: u32,
    pub whole_amount: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationAlertRow {
    pub stable_id: String,
    pub controller_tag: String,
    pub kind: String,
    pub severity: String,
    pub registry_slot: u32,
    pub predicate_returned: bool,
    pub equipment_fixed_point_divisor: i64,
    pub visible_order: u32,
    pub missing_manpower: i64,
    pub missing_equipment: Vec<GarrisonShortageEquipmentRow>,
    pub resolved_text: String,
}

#[derive(Clone, Copy)]
pub struct OccupationFixtureSurfaceRequest<'a> {
    pub source_record_id: &'a str,
    pub frame: &'a WorldFrame,
    pub occupation_rows: &'a BTreeMap<u32, OccupationRow>,
    pub visible_text_by_state: &'a BTreeMap<u32, OccupationVisibleText>,
    pub law_definitions_by_hash: &'a BTreeMap<u32, OccupationLawDefinition>,
    pub garrison_templates: &'a BTreeMap<u32, GarrisonTemplateDefinition>,
    pub equipment_by_state: &'a BTreeMap<u32, Vec<GarrisonEquipmentRow>>,
    pub equipment_complete: bool,
    pub collections: &'a OccupationObserverCollections,
    pub activity_location_text: &'a BTreeMap<String, String>,
    pub equipment_display_names: &'a BTreeMap<(u32, u32), String>,
    pub alert_resolved_text: &'a BTreeMap<String, String>,
    pub controller_supplement: Option<&'a OccupationD66ControllerSupplement>,
}

pub struct OccupationSurfaceRequest<'a> {
    pub source_record_id: &'a str,
    pub occupation_rows: &'a BTreeMap<u32, OccupationRow>,
    pub visible_text_by_state: &'a BTreeMap<u32, OccupationVisibleText>,
    pub law_definitions_by_hash: &'a BTreeMap<u32, OccupationLawDefinition>,
    pub garrison_templates: &'a BTreeMap<u32, GarrisonTemplateDefinition>,
    pub equipment_by_state: &'a BTreeMap<u32, Vec<GarrisonEquipmentRow>>,
    pub equipment_complete: bool,
    pub collections: &'a OccupationObserverCollections,
    pub activities: &'a [GarrisonActivityRow],
    pub activities_complete: bool,
    pub equipment_losses: &'a [GarrisonEquipmentLossRow],
    pub equipment_losses_complete: bool,
    pub alerts: &'a [OccupationAlertRow],
    pub alerts_complete: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationStatePresentation {
    pub state_id: u32,
    pub occupier_tag: String,
    pub occupied_country_tag: String,
    pub text: OccupationVisibleText,
    pub resistance: f64,
    pub resistance_speed: f64,
    pub compliance: f64,
    pub compliance_speed: f64,
    pub law: OccupationLawDefinition,
    pub garrison_template: GarrisonTemplateDefinition,
    pub garrison_army_manpower_need: i64,
    pub garrison_strength_ratio: f64,
    pub garrison_required: f64,
    pub garrison_manpower_source_tag_db_id: Option<u32>,
    pub equipment: Vec<GarrisonEquipmentRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AdmittedOccupationSurface {
    product_authority: bool,
    source_record_id: String,
    states: Vec<OccupationStatePresentation>,
    activities: Vec<GarrisonActivityRow>,
    equipment_losses: Vec<GarrisonEquipmentLossRow>,
    alerts: Vec<OccupationAlertRow>,
}

#[derive(Clone, Debug, PartialEq)]
struct OccupationProductionInputs {
    source_record_id: String,
    occupation_rows: BTreeMap<u32, OccupationRow>,
    visible_text_by_state: BTreeMap<u32, OccupationVisibleText>,
    law_definitions_by_hash: BTreeMap<u32, OccupationLawDefinition>,
    garrison_templates: BTreeMap<u32, GarrisonTemplateDefinition>,
    equipment_by_state: BTreeMap<u32, Vec<GarrisonEquipmentRow>>,
    collections: OccupationObserverCollections,
    activity_location_text: BTreeMap<String, String>,
    equipment_display_names: BTreeMap<(u32, u32), String>,
    alert_resolved_text: BTreeMap<String, String>,
    controller_supplement: OccupationD66ControllerSupplement,
}

impl OccupationProductionInputs {
    fn as_request<'a>(
        &'a self,
        frame: &'a WorldFrame,
    ) -> OccupationFixtureSurfaceRequest<'a> {
        OccupationFixtureSurfaceRequest {
            source_record_id: &self.source_record_id,
            frame,
            occupation_rows: &self.occupation_rows,
            visible_text_by_state: &self.visible_text_by_state,
            law_definitions_by_hash: &self.law_definitions_by_hash,
            garrison_templates: &self.garrison_templates,
            equipment_by_state: &self.equipment_by_state,
            equipment_complete: true,
            collections: &self.collections,
            activity_location_text: &self.activity_location_text,
            equipment_display_names: &self.equipment_display_names,
            alert_resolved_text: &self.alert_resolved_text,
            controller_supplement: Some(&self.controller_supplement),
        }
    }

    fn admit(
        &self,
        frame: &WorldFrame,
    ) -> Result<AdmittedOccupationSurface, OccupationRejection> {
        admit_resolved_occupation_surface_from_world(self.as_request(frame), true)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OccupationAssemblyGap {
    pub code: &'static str,
    pub detail: String,
}

impl std::fmt::Display for OccupationAssemblyGap {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.detail)
    }
}

#[derive(Clone, Debug)]
pub struct OccupationSurfaceAssembler {
    law_definitions_by_hash: BTreeMap<u32, OccupationLawDefinition>,
    localisation: Localisation,
    localisation_language: String,
}

impl OccupationSurfaceAssembler {
    pub fn new(resolver: &AssetResolver, localisation_language: &str) -> UiResult<Self> {
        validate_occupation_gui_sources(resolver)?;
        let localisation =
            Localisation::load_admitted_language(resolver, localisation_language)?;
        if !localisation.parse_failures.is_empty() || !localisation.decode_lossy_files.is_empty() {
            return Err(GameUiError::parse(format!(
                "XUI16 localisation is incomplete: parse={:?} decode={:?}",
                localisation.parse_failures, localisation.decode_lossy_files
            )));
        }
        let law_definitions_by_hash =
            load_effective_occupation_law_definitions(resolver, &localisation)?;
        Ok(Self {
            law_definitions_by_hash,
            localisation,
            localisation_language: localisation_language.to_owned(),
        })
    }

    fn assemble(
        &self,
        frame: &WorldFrame,
        selected_country: Option<&str>,
    ) -> std::result::Result<OccupationProductionInputs, OccupationAssemblyGap> {
        let supplement_view = frame.occupation_ui_supplement_exact();
        if supplement_view.status != ViewStatus::Present {
            return Err(OccupationAssemblyGap {
                code: "XUI16_D66_SUPPLEMENT_UNAVAILABLE",
                detail: format!(
                    "typed WorldFrame supplement status is {:?}",
                    supplement_view.status
                ),
            });
        }
        let supplement =
            supplement_view
                .supplement
                .as_ref()
                .ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_D66_SUPPLEMENT_ABSENT",
                    detail: "Present typed supplement has no payload".into(),
                })?;
        if !supplement.complete {
            return Err(OccupationAssemblyGap {
                code: "XUI16_D66_SUPPLEMENT_INCOMPLETE",
                detail: "D66 supplement completeness is false".into(),
            });
        }
        if supplement.localisation_language != self.localisation_language {
            return Err(OccupationAssemblyGap {
                code: "XUI16_D66_LANGUAGE_MISMATCH",
                detail: format!(
                    "supplement language {} differs from admitted playback language {}",
                    supplement.localisation_language, self.localisation_language
                ),
            });
        }
        let selected_country = selected_country
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| OccupationAssemblyGap {
                code: "XUI16_VIEWER_COUNTRY_UNSELECTED",
                detail: "the observer has not selected a country".into(),
            })?;
        if !supplement.controllers.contains_key(selected_country) {
            return Err(OccupationAssemblyGap {
                code: "XUI16_D66_COUNTRY_UNAVAILABLE",
                detail: format!(
                    "the complete supplement has no controller row for viewer-selected country {selected_country}"
                ),
            });
        }
        let expected_hour = u64::try_from(frame.hour_index()).map_err(|_| OccupationAssemblyGap {
            code: "XUI16_NEGATIVE_FRAME_HOUR",
            detail: format!("frame hour {} is negative", frame.hour_index()),
        })?;
        if supplement.frame_hour != expected_hour {
            return Err(OccupationAssemblyGap {
                code: "XUI16_D66_FRAME_MISMATCH",
                detail: format!(
                    "supplement hour {} differs from WorldFrame hour {expected_hour}",
                    supplement.frame_hour
                ),
            });
        }
        validate_occupation_controller_census(frame, supplement)?;
        validate_legacy_alert_join(frame, supplement)?;
        let equipment_display_names =
            equipment_display_names(frame, &self.localisation, selected_country)?;
        let controller = &supplement.controllers[selected_country];
        self.assemble_controller_inputs(
            frame,
            &supplement.source_record_id,
            selected_country,
            controller,
            &equipment_display_names,
        )
    }

    fn assemble_controller_inputs(
        &self,
        frame: &WorldFrame,
        source_record_id: &str,
        controller_country: &str,
        controller: &OccupationD66ControllerSupplement,
        equipment_display_names: &BTreeMap<(u32, u32), String>,
    ) -> std::result::Result<OccupationProductionInputs, OccupationAssemblyGap> {
        let collections =
            occupation_observer_collections(frame, controller_country, controller)?;
        let occupation_rows = frame
            .occupation_by_state()
            .iter()
            .filter(|(_, row)| {
                row.occupier_tag.measured().map(String::as_str) == Some(controller_country)
                    && measured_u32(&row.occupation_controller_tag_db_id)
                        == Some(controller.controller_country_tag_db_id)
            })
            .map(|(&state_id, row)| (state_id, row.clone()))
            .collect::<BTreeMap<_, _>>();
        let required_template_ids = occupation_rows
            .iter()
            .map(|(&state_id, row)| {
                measured_u32(&row.garrison_template_id).ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_GARRISON_TEMPLATE_ID_UNAVAILABLE",
                    detail: format!(
                        "controller {controller_country} state {state_id} has no portable garrison template identity"
                    ),
                })
            })
            .collect::<std::result::Result<BTreeSet<_>, _>>()?;
        if controller.garrison_templates.keys().copied().collect::<BTreeSet<_>>()
            != required_template_ids
        {
            return Err(OccupationAssemblyGap {
                code: "XUI16_GARRISON_TEMPLATE_COVERAGE_MISMATCH",
                detail: format!(
                    "D66 template definitions do not exactly cover controller {controller_country} occupation rows"
                ),
            });
        }
        let equipment_by_state = equipment_by_state(&occupation_rows)?;
        let visible_text_by_state =
            occupation_visible_text(frame, &occupation_rows, controller, &self.localisation)?;
        Ok(OccupationProductionInputs {
            source_record_id: source_record_id.to_owned(),
            occupation_rows,
            visible_text_by_state,
            law_definitions_by_hash: self.law_definitions_by_hash.clone(),
            garrison_templates: controller.garrison_templates.clone(),
            equipment_by_state,
            collections,
            activity_location_text: controller.activity_location_text.clone(),
            equipment_display_names: equipment_display_names.clone(),
            alert_resolved_text: controller.alert_resolved_text.clone(),
            controller_supplement: controller.clone(),
        })
    }

    pub fn assemble_admitted(
        &self,
        frame: &WorldFrame,
        selected_country: Option<&str>,
    ) -> std::result::Result<AdmittedOccupationSurface, OccupationAssemblyGap> {
        let inputs = self.assemble(frame, selected_country)?;
        let surface = inputs.admit(frame).map_err(|error| OccupationAssemblyGap {
            code: "XUI16_PRODUCT_ADMISSION_REJECTED",
            detail: format!("{error:?}"),
        })?;
        if !surface.is_product_authority() {
            return Err(OccupationAssemblyGap {
                code: "XUI16_PRODUCT_AUTHORITY_MISSING",
                detail: "fixture-only occupation surface reached the product route".into(),
            });
        }
        Ok(surface)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationAlertProductBinding {
    pub alert: OccupationAlertRow,
    pub icon: crate::alerts::AlertFeedIconBinding,
}

fn validate_occupation_controller_census(
    frame: &WorldFrame,
    supplement: &OccupationD66UiSupplement,
) -> std::result::Result<(), OccupationAssemblyGap> {
    let roster = frame.d18_country_roster();
    if roster.status != ViewStatus::Present {
        return Err(OccupationAssemblyGap {
            code: "XUI16_D18_ROSTER_UNAVAILABLE",
            detail: roster
                .diagnostic
                .unwrap_or_else(|| format!("D18 roster status is {:?}", roster.status)),
        });
    }
    let roster = roster
        .entries
        .into_iter()
        .map(|entry| (entry.tag, entry.tag_db_id))
        .collect::<BTreeMap<_, _>>();
    validate_occupation_controller_identities(
        frame.occupation_by_state().iter().map(|(&state_id, row)| {
            (
                state_id,
                row.occupier_tag
                    .measured()
                    .map(String::as_str)
                    .filter(|tag| !tag.trim().is_empty()),
                measured_u32(&row.occupation_controller_tag_db_id).filter(|id| *id != 0),
                row.occupied_country_tag
                    .measured()
                    .map(String::as_str)
                    .filter(|tag| !tag.trim().is_empty()),
                measured_u32(&row.occupied_tag_db_id).filter(|id| *id != 0),
            )
        }),
        supplement,
        &roster,
    )
}

fn validate_occupation_controller_identities<'a>(
    rows: impl IntoIterator<
        Item = (
            u32,
            Option<&'a str>,
            Option<u32>,
            Option<&'a str>,
            Option<u32>,
        ),
    >,
    supplement: &OccupationD66UiSupplement,
    roster: &BTreeMap<String, u32>,
) -> std::result::Result<(), OccupationAssemblyGap> {
    for (
        state_id,
        controller_tag,
        controller_tag_db_id,
        occupied_tag,
        occupied_tag_db_id,
    ) in rows
    {
        let controller_tag = controller_tag.ok_or_else(|| OccupationAssemblyGap {
            code: "XUI16_STATE_CONTROLLER_UNCLASSIFIED",
            detail: format!("state {state_id} has no nonempty measured occupier tag"),
        })?;
        let controller_tag_db_id =
            controller_tag_db_id.ok_or_else(|| OccupationAssemblyGap {
                code: "XUI16_STATE_CONTROLLER_UNCLASSIFIED",
                detail: format!("state {state_id} has no nonzero measured controller D18 identity"),
            })?;
        if roster.get(controller_tag) != Some(&controller_tag_db_id) {
            return Err(OccupationAssemblyGap {
                code: "XUI16_STATE_CONTROLLER_CENSUS_MISMATCH",
                detail: format!(
                    "state {state_id} controller {controller_tag} does not join the typed D18 roster"
                ),
            });
        }
        let controller =
            supplement
                .controllers
                .get(controller_tag)
                .ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_STATE_CONTROLLER_CENSUS_MISMATCH",
                    detail: format!(
                        "state {state_id} controller {controller_tag} is absent from the complete D18 census"
                    ),
                })?;
        if controller.controller_country_tag_db_id != controller_tag_db_id {
            return Err(OccupationAssemblyGap {
                code: "XUI16_STATE_CONTROLLER_CENSUS_MISMATCH",
                detail: format!(
                    "state {state_id} controller {controller_tag} D18 identity differs from the supplement census"
                ),
            });
        }
        let occupied_tag = occupied_tag.ok_or_else(|| OccupationAssemblyGap {
            code: "XUI16_OCCUPIED_COUNTRY_UNCLASSIFIED",
            detail: format!("state {state_id} has no nonempty measured occupied-country tag"),
        })?;
        let occupied_tag_db_id =
            occupied_tag_db_id.ok_or_else(|| OccupationAssemblyGap {
                code: "XUI16_OCCUPIED_COUNTRY_UNCLASSIFIED",
                detail: format!(
                    "state {state_id} has no nonzero measured occupied-country D18 identity"
                ),
            })?;
        if roster.get(occupied_tag) != Some(&occupied_tag_db_id) {
            return Err(OccupationAssemblyGap {
                code: "XUI16_OCCUPIED_COUNTRY_CENSUS_MISMATCH",
                detail: format!(
                    "state {state_id} occupied country {occupied_tag} does not join the typed D18 roster"
                ),
            });
        }
    }
    Ok(())
}

fn validate_legacy_alert_join(
    frame: &WorldFrame,
    supplement: &OccupationD66UiSupplement,
) -> std::result::Result<(), OccupationAssemblyGap> {
    let legacy = frame.occupation_garrison_alert_exact();
    if legacy.status != ViewStatus::Present {
        return Ok(());
    }
    let alert = legacy.alert.as_ref().ok_or_else(|| OccupationAssemblyGap {
        code: "XUI16_LEGACY_ALERT_PAYLOAD_ABSENT",
        detail: "Present legacy garrison-alert carrier has no payload".into(),
    })?;
    let controller = supplement
        .controllers
        .get(&alert.controller_tag)
        .ok_or_else(|| OccupationAssemblyGap {
            code: "XUI16_LEGACY_ALERT_CONTROLLER_MISSING",
            detail: format!(
                "legacy alert controller {} is absent from the complete supplement",
                alert.controller_tag
            ),
        })?;
    if !legacy_alert_matches(alert, controller) {
        return Err(OccupationAssemblyGap {
            code: "XUI16_LEGACY_ALERT_MISMATCH",
            detail: format!(
                "controller {} supplement alert contradicts the same-hour legacy exact carrier",
                alert.controller_tag
            ),
        });
    }
    Ok(())
}

fn legacy_alert_matches(
    alert: &replay_core::OccupationGarrisonAlertExact,
    controller: &OccupationD66ControllerSupplement,
) -> bool {
    let equipment_matches = alert.missing_equipment.len()
        == controller.garrison_alert.missing_equipment.len()
        && alert
            .missing_equipment
            .iter()
            .zip(&controller.garrison_alert.missing_equipment)
            .all(|(left, right)| {
                left.archetype_id == right.archetype_id
                    && left.whole_units == right.whole_units
            });
    controller.controller_country_tag_db_id == alert.controller_tag_db_id
        && controller.controller_country_tag == alert.controller_tag
        && controller.garrison_alert.game_hour_raw == alert.game_hour_raw
        && controller.garrison_alert.predicate_returned == alert.predicate_returned
        && controller.garrison_alert.equipment_fixed_point_divisor
            == alert.equipment_fixed_point_divisor
        && controller.garrison_alert.missing_manpower == alert.missing_manpower
        && equipment_matches
}

#[cfg(test)]
mod alert_join_tests {
    use super::*;
    use std::sync::Arc;

    struct TempRecording(std::path::PathBuf);

    impl Drop for TempRecording {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn control_frame_law() -> replay_core::OccupationUiControlFrameLawExact {
        replay_core::OccupationUiControlFrameLawExact {
            territory_sort_button_frames: (0..=4)
                .map(|selected| {
                    (
                        selected,
                        std::array::from_fn(|button| {
                            if button as u8 == selected { 0 } else { 1 }
                        }),
                    )
                })
                .collect(),
            show_non_resisting_checkbox_frames: [1, 0],
            territory_metric_button_frames: [[0, 1], [1, 0]],
            garrison_tab_button_frames: [[0, 1], [1, 0]],
        }
    }

    fn controller_presentation() -> replay_core::OccupationUiControllerPresentationExact {
        replay_core::OccupationUiControllerPresentationExact {
            root: replay_core::OccupationUiControllerRootPresentationExact {
                default_garrison_template_icon_sprite: "GFX_test".into(),
                default_garrison_template_need_text: "0".into(),
                selected_default_law_text: "Test".into(),
                selected_default_law_icon_frame: 1,
                invalid_default_law_visible: false,
                foreign_support_visible: false,
                foreign_support_amount_text: "0".into(),
            },
            country_groups: BTreeMap::new(),
            territory_country_orders: Vec::new(),
            state_orders_by_occupied_country: BTreeMap::new(),
            state_presentations: BTreeMap::new(),
            activity_presentations: BTreeMap::new(),
            equipment_presentations: Vec::new(),
            month_projections: Vec::new(),
        }
    }

    fn complete_product_supplement() -> replay_core::OccupationUiSupplementExact {
        use replay_core::{
            OccupationUiActivityPresentationExact, OccupationUiControllerPresentationExact,
            OccupationUiControllerRootPresentationExact,
            OccupationUiControllerSupplementExact, OccupationUiCountryGroupPresentationExact,
            OccupationUiEquipmentPresentationExact, OccupationUiFormattedStateTextExact,
            OccupationUiGarrisonAlertEquipmentExact, OccupationUiGarrisonAlertExact,
            OccupationUiGarrisonTemplateExact, OccupationUiModifierIconExact,
            OccupationUiMonthEquipmentLossExact, OccupationUiMonthProjectionExact,
            OccupationUiStatePresentationExact, OccupationUiStateSortOrderExact,
            OccupationUiStateTemplateExact, OccupationUiStatusTargetExact,
            OccupationUiStatusWidgetExact, OccupationUiTerritoryCountryOrderExact,
        };

        let status = |text: &str, target: bool| OccupationUiStatusWidgetExact {
            progress_text: text.into(),
            progress_raw: 2_230_000,
            progress_divisor: 100_000,
            progress_frame: 22,
            target: target.then_some(OccupationUiStatusTargetExact {
                authored_local_x: 37,
                value_raw: 3_000_000,
                value_divisor: 100_000,
            }),
            modifier_icons: vec![OccupationUiModifierIconExact {
                icon_sprite: "GFX_resistance_modifier".into(),
                frame: 1,
            }],
        };
        let root = || OccupationUiControllerRootPresentationExact {
            default_garrison_template_icon_sprite: "GFX_unit_cavalry_icon_medium".into(),
            default_garrison_template_need_text: "14.0K".into(),
            selected_default_law_text: "Local Police Force".into(),
            selected_default_law_icon_frame: 11,
            invalid_default_law_visible: false,
            foreign_support_visible: false,
            foreign_support_amount_text: "0".into(),
        };
        let territory_orders = |tags: &[&str]| {
            (0..=4)
                .flat_map(|sort_mode| {
                    [false, true].into_iter().map(move |show_non_resisting| {
                        OccupationUiTerritoryCountryOrderExact {
                            sort_mode,
                            show_non_resisting,
                            occupied_country_tags: tags.iter().map(|tag| (*tag).into()).collect(),
                        }
                    })
                })
                .collect::<Vec<_>>()
        };
        let month_projections = |activities: &[&str], equipment: bool| {
            (1..=12)
                .map(|months| OccupationUiMonthProjectionExact {
                    months,
                    cutoff_game_date_raw: 43_800_041 - i32::from(months) * 720,
                    total_manpower_lost_raw: if activities.is_empty() { 0 } else { 43 },
                    total_manpower_lost_text: if activities.is_empty() {
                        "Manpower losses: 0".into()
                    } else {
                        "Manpower losses: 43".into()
                    },
                    total_equipment_lost_raw: if equipment { 300_000 } else { 0 },
                    total_equipment_lost_divisor: 100_000,
                    total_equipment_lost_text: if equipment {
                        "Equipment losses: 3".into()
                    } else {
                        "Equipment losses: 0".into()
                    },
                    equipment_tab_total_text: if equipment { "3".into() } else { "0".into() },
                    empty_activity_text: format!(
                        "No activity in the selected {months}-month window."
                    ),
                    empty_equipment_text: format!(
                        "No equipment lost in the selected {months}-month window."
                    ),
                    ordered_activity_ids: activities.iter().map(|id| (*id).into()).collect(),
                    equipment_losses: equipment
                        .then_some(OccupationUiMonthEquipmentLossExact {
                            equipment_id: 438,
                            equipment_type: 70,
                            amount_lost_raw: 300_000,
                            amount_lost_divisor: 100_000,
                            amount_lost_text: "3".into(),
                        })
                        .into_iter()
                        .collect(),
                })
                .collect::<Vec<_>>()
        };
        let first_activity = "GER|1942.8.15.1|891|0";
        let second_activity = "GER|1942.8.15.2|891|1";
        let presentation = OccupationUiControllerPresentationExact {
            root: root(),
            country_groups: BTreeMap::from([(
                "FRA".into(),
                OccupationUiCountryGroupPresentationExact {
                    occupied_country_tag_db_id: 180,
                    occupied_country_tag: "FRA".into(),
                    state_ids: vec![891],
                    country_name: "France".into(),
                    flag_texture: "gfx/flags/fra.tga".into(),
                    occupied_state_count_text: "1".into(),
                    manpower_text: "23.0K".into(),
                    industrial_capacity_text: "12".into(),
                    resources_text: "8".into(),
                    resistance_status: status("22.3%", true),
                    compliance_status: status("65.0%", false),
                    selected_law_text: "Local Police Force".into(),
                    selected_law_icon_frame: 11,
                    garrison_template_icon_sprite: "GFX_unit_cavalry_icon_medium".into(),
                    garrison_template_need_text: "14.0K".into(),
                    garrison_strength_frame: 8,
                    release_visible: true,
                    release_enabled: true,
                    return_territory_visible: false,
                    return_territory_enabled: false,
                    invalid_default_law_visible: false,
                },
            )]),
            territory_country_orders: territory_orders(&["FRA"]),
            state_orders_by_occupied_country: BTreeMap::from([(
                "FRA".into(),
                (0..=4)
                    .map(|sort_mode| OccupationUiStateSortOrderExact {
                        sort_mode,
                        state_ids: vec![891],
                    })
                    .collect(),
            )]),
            state_presentations: BTreeMap::from([(
                891,
                OccupationUiStatePresentationExact {
                    template: OccupationUiStateTemplateExact::Resisting60,
                    resistance_status: Some(status("22.3%", true)),
                    compliance_status: Some(status("65.0%", false)),
                    garrison_template_icon_sprite: Some(
                        "GFX_unit_cavalry_icon_medium".into(),
                    ),
                    garrison_template_need_text: Some("14.0K".into()),
                    garrison_strength_frame: Some(8),
                    selected_law_icon_frame: Some(11),
                },
            )]),
            activity_presentations: BTreeMap::from([
                (
                    first_activity.into(),
                    OccupationUiActivityPresentationExact {
                        attack_result_text: "Attack succeeded".into(),
                        lost_manpower_text: "33".into(),
                        lost_equipment_text: "1".into(),
                        occupied_country_flag_texture: "gfx/flags/fra.tga".into(),
                        garrison_losses_visible: true,
                        attack_result_visible: true,
                    },
                ),
                (
                    second_activity.into(),
                    OccupationUiActivityPresentationExact {
                        attack_result_text: "Attack repelled".into(),
                        lost_manpower_text: "10".into(),
                        lost_equipment_text: "2".into(),
                        occupied_country_flag_texture: "gfx/flags/fra.tga".into(),
                        garrison_losses_visible: true,
                        attack_result_visible: true,
                    },
                ),
            ]),
            equipment_presentations: vec![OccupationUiEquipmentPresentationExact {
                equipment_id: 438,
                equipment_type: 70,
                equipment_icon_sprite: "GFX_infantry_equipment_medium".into(),
            }],
            month_projections: month_projections(&[first_activity, second_activity], true),
        };
        let empty_presentation = OccupationUiControllerPresentationExact {
            root: root(),
            country_groups: BTreeMap::new(),
            territory_country_orders: territory_orders(&[]),
            state_orders_by_occupied_country: BTreeMap::new(),
            state_presentations: BTreeMap::new(),
            activity_presentations: BTreeMap::new(),
            equipment_presentations: Vec::new(),
            month_projections: month_projections(&[], false),
        };
        let controller = OccupationUiControllerSupplementExact {
            controller_country_tag_db_id: 47,
            controller_country_tag: "GER".into(),
            state_source_order: vec![891],
            activity_source_order: vec![first_activity.into(), second_activity.into()],
            equipment_loss_source_order: vec![[438, 70]],
            alert_controller_source_order: vec!["GER".into()],
            garrison_alert: OccupationUiGarrisonAlertExact {
                game_hour_raw: 43_800_041,
                predicate_returned: false,
                equipment_fixed_point_divisor: 100_000,
                missing_manpower: 4_200,
                missing_equipment: vec![OccupationUiGarrisonAlertEquipmentExact {
                    archetype_id: 12,
                    whole_units: 125,
                }],
            },
            formatted_text_by_state: BTreeMap::from([(
                891,
                OccupationUiFormattedStateTextExact {
                    resistance_text: "22.3%".into(),
                    resistance_speed_text: "+0.025".into(),
                    compliance_text: "65.0%".into(),
                    compliance_speed_text: "-0.012".into(),
                    garrison_manpower_need_text: "14.0K".into(),
                    garrison_required_text: "2.50".into(),
                },
            )]),
            garrison_templates: BTreeMap::from([(
                4183,
                OccupationUiGarrisonTemplateExact {
                    template_id: 4183,
                    display_name: "Cavalry Garrison".into(),
                    icon_sprite: "GFX_unit_cavalry_icon_medium".into(),
                },
            )]),
            activity_location_text: BTreeMap::from([
                (first_activity.into(), "Angola".into()),
                (second_activity.into(), "Angola".into()),
            ]),
            alert_resolved_text: BTreeMap::from([(
                "GER".into(),
                "Garrison reinforcement required".into(),
            )]),
            presentation,
        };
        let empty_controller = OccupationUiControllerSupplementExact {
            controller_country_tag_db_id: 180,
            controller_country_tag: "FRA".into(),
            state_source_order: Vec::new(),
            activity_source_order: Vec::new(),
            equipment_loss_source_order: Vec::new(),
            alert_controller_source_order: Vec::new(),
            garrison_alert: OccupationUiGarrisonAlertExact {
                game_hour_raw: 43_800_041,
                predicate_returned: true,
                equipment_fixed_point_divisor: 100_000,
                missing_manpower: 0,
                missing_equipment: Vec::new(),
            },
            formatted_text_by_state: BTreeMap::new(),
            garrison_templates: BTreeMap::new(),
            activity_location_text: BTreeMap::new(),
            alert_resolved_text: BTreeMap::new(),
            presentation: empty_presentation,
        };
        let mut supplement = replay_core::OccupationUiSupplementExact {
            schema_version: replay_core::OCCUPATION_UI_SUPPLEMENT_SCHEMA_VERSION,
            authority: replay_core::OCCUPATION_UI_SUPPLEMENT_AUTHORITY.into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            evidence_manifest_sha256s:
                replay_core::OCCUPATION_UI_SUPPLEMENT_EVIDENCE_MANIFEST_SHA256S
                    .iter()
                    .map(|value| (*value).into())
                    .collect(),
            source_record_id: String::new(),
            frame_hour: 42,
            localisation_language: "l_english".into(),
            control_frame_law: control_frame_law(),
            complete: true,
            controllers: BTreeMap::from([
                ("FRA".into(), empty_controller),
                ("GER".into(), controller),
            ]),
        };
        supplement.source_record_id =
            replay_core::world::compute_occupation_ui_source_record_id(&supplement).unwrap();
        supplement
    }

    fn product_frame_with_complete_supplement() -> (TempRecording, Arc<WorldFrame>) {
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/xui16_production_owner/xui16_owner");
        let mut frame_json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(fixture.join("000000000042.snapshot.json")).unwrap(),
        )
        .unwrap();
        frame_json
            .as_object_mut()
            .unwrap()
            .insert(
                "_r2_occupation_ui_supplement_live".into(),
                serde_json::to_value(complete_product_supplement()).unwrap(),
            );
        let root = std::env::temp_dir().join(format!(
            "hoi4-xui16-product-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let recording = root.join("xui16_owner");
        std::fs::create_dir_all(&recording).unwrap();
        std::fs::copy(fixture.join("index.json"), recording.join("index.json")).unwrap();
        std::fs::write(
            recording.join("000000000042.snapshot.json"),
            serde_json::to_vec_pretty(&frame_json).unwrap(),
        )
        .unwrap();
        let timeline = replay_core::WorldTimeline::open(
            replay_core::open_recording(&root).expect("open complete XUI16 recording"),
        )
        .expect("load complete XUI16 recording");
        let frame = timeline.frame(42).expect("complete XUI16 frame");
        (TempRecording(root), frame)
    }

    #[test]
    fn product_assembler_admits_complete_all_country_d52_d66_equipment_and_alert() {
        let (_recording, frame) = product_frame_with_complete_supplement();
        let mut localisation = Localisation::default();
        localisation.entries.extend([
            ("STATE_891".into(), "Angola".into()),
            ("FRA".into(), "France".into()),
            ("support_equipment_1".into(), "Support Equipment".into()),
        ]);
        let law = OccupationLawDefinition {
            token: "local_police_force_garrison".into(),
            display_name: "Local Police Force".into(),
            icon_frame: 11,
            gui_order: 0,
        };
        let assembler = OccupationSurfaceAssembler {
            law_definitions_by_hash: BTreeMap::from([(fnv1a32(&law.token), law)]),
            localisation,
            localisation_language: "l_english".into(),
        };

        let surface = assembler
            .assemble_admitted(frame.as_ref(), Some("GER"))
            .expect("complete all-country D18/D52/D66 product carrier must admit");
        assert!(surface.is_product_authority());
        assert_eq!(surface.states()[0].state_id, 891);
        assert_eq!(surface.states()[0].equipment[0].amount, 800.0);
        assert_eq!(surface.activities().len(), 2);
        assert_eq!(surface.equipment_losses()[0].amount_lost, 3.0);
        assert_eq!(surface.alerts()[0].missing_manpower, 4_200);
        assert_eq!(
            frame
                .occupation_ui_supplement_exact()
                .supplement
                .as_ref()
                .unwrap()
                .controllers
                .len(),
            2,
            "the selected-country projection must follow complete all-country admission"
        );
    }

    fn legacy_alert() -> replay_core::OccupationGarrisonAlertExact {
        replay_core::OccupationGarrisonAlertExact {
            game_hour_raw: 43_800_041,
            controller_tag_db_id: 47,
            controller_tag: "GER".into(),
            predicate_returned: false,
            equipment_fixed_point_divisor: 100_000,
            missing_manpower: 4_200,
            missing_equipment: vec![replay_core::OccupationGarrisonAlertEquipmentExact {
                archetype_id: 438,
                whole_units: 125,
            }],
        }
    }

    fn controller_alert() -> OccupationD66ControllerSupplement {
        OccupationD66ControllerSupplement {
            controller_country_tag_db_id: 47,
            controller_country_tag: "GER".into(),
            state_source_order: Vec::new(),
            activity_source_order: Vec::new(),
            equipment_loss_source_order: Vec::new(),
            alert_controller_source_order: vec!["GER".into()],
            garrison_alert: replay_core::OccupationUiGarrisonAlertExact {
                game_hour_raw: 43_800_041,
                predicate_returned: false,
                equipment_fixed_point_divisor: 100_000,
                missing_manpower: 4_200,
                missing_equipment: vec![
                    replay_core::OccupationUiGarrisonAlertEquipmentExact {
                        archetype_id: 438,
                        whole_units: 125,
                    },
                ],
            },
            formatted_text_by_state: BTreeMap::new(),
            garrison_templates: BTreeMap::new(),
            activity_location_text: BTreeMap::new(),
            alert_resolved_text: BTreeMap::from([("GER".into(), "exact text".into())]),
            presentation: controller_presentation(),
        }
    }

    #[test]
    fn legacy_alert_join_rejects_any_shortage_disagreement() {
        let legacy = legacy_alert();
        let mut supplement = controller_alert();
        assert!(legacy_alert_matches(&legacy, &supplement));

        supplement.garrison_alert.missing_manpower += 1;
        assert!(!legacy_alert_matches(&legacy, &supplement));
        supplement = controller_alert();
        supplement.garrison_alert.missing_equipment[0].whole_units += 1;
        assert!(!legacy_alert_matches(&legacy, &supplement));
        supplement = controller_alert();
        supplement.garrison_alert.predicate_returned = true;
        assert!(!legacy_alert_matches(&legacy, &supplement));
    }

    #[test]
    fn occupation_controller_census_rejects_an_unmeasured_row_beside_a_valid_row() {
        let controller = controller_alert();
        let supplement = OccupationD66UiSupplement {
            schema_version: replay_core::OCCUPATION_UI_SUPPLEMENT_SCHEMA_VERSION,
            authority: replay_core::OCCUPATION_UI_SUPPLEMENT_AUTHORITY.into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            evidence_manifest_sha256s:
                replay_core::OCCUPATION_UI_SUPPLEMENT_EVIDENCE_MANIFEST_SHA256S
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            source_record_id: "test-only".into(),
            frame_hour: 42,
            localisation_language: "l_english".into(),
            control_frame_law: control_frame_law(),
            complete: true,
            controllers: BTreeMap::from([("GER".into(), controller)]),
        };
        let roster = BTreeMap::from([("FRA".into(), 180), ("GER".into(), 47)]);
        let rejected = validate_occupation_controller_identities(
            [
                (891, Some("GER"), Some(47), Some("FRA"), Some(180)),
                (892, None, Some(47), Some("FRA"), Some(180)),
            ],
            &supplement,
            &roster,
        )
        .expect_err("an unmeasured occupier must poison the whole occupation census");
        assert_eq!(rejected.code, "XUI16_STATE_CONTROLLER_UNCLASSIFIED");
    }

    #[test]
    fn occupied_country_numeric_identity_must_join_the_typed_d18_roster() {
        let controller = controller_alert();
        let supplement = OccupationD66UiSupplement {
            schema_version: replay_core::OCCUPATION_UI_SUPPLEMENT_SCHEMA_VERSION,
            authority: replay_core::OCCUPATION_UI_SUPPLEMENT_AUTHORITY.into(),
            executable_sha256:
                "870ebf1f615262facca48a3b947775c41f0a5338f3068c40958209f5c41c65ad"
                    .into(),
            evidence_manifest_sha256s:
                replay_core::OCCUPATION_UI_SUPPLEMENT_EVIDENCE_MANIFEST_SHA256S
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            source_record_id: "test-only".into(),
            frame_hour: 42,
            localisation_language: "l_english".into(),
            control_frame_law: control_frame_law(),
            complete: true,
            controllers: BTreeMap::from([("GER".into(), controller)]),
        };
        let roster = BTreeMap::from([("FRA".into(), 180), ("GER".into(), 47)]);
        let rejected = validate_occupation_controller_identities(
            [(891, Some("GER"), Some(47), Some("FRA"), Some(181))],
            &supplement,
            &roster,
        )
        .expect_err("contradictory occupied-country D18 identity must reject");
        assert_eq!(
            rejected.code,
            "XUI16_OCCUPIED_COUNTRY_CENSUS_MISMATCH"
        );
    }

    #[test]
    fn country_name_dimensions_fall_back_only_for_explicit_absence() {
        assert_eq!(
            admitted_optional_country_name_dimension(&Leaf::Absent, "ruling ideology"),
            Ok(None)
        );
        assert!(
            admitted_optional_country_name_dimension(&Leaf::Unobtained, "ruling ideology")
                .is_err()
        );
        assert!(
            admitted_optional_country_name_dimension(
                &Leaf::Measured(String::new()),
                "ruling ideology"
            )
            .is_err()
        );

        assert_eq!(
            crate::binding::admitted_cosmetic_override(&Leaf::Measured(Some(
                "FRA_VICHY".to_string()
            ))),
            Ok(Some("FRA_VICHY"))
        );
        assert_eq!(
            crate::binding::admitted_cosmetic_override(&Leaf::Measured(None)),
            Ok(None),
            "a measured 'no cosmetic override' names the country off its base TAG"
        );
        assert!(
            crate::binding::admitted_cosmetic_override(&Leaf::Absent).is_err(),
            "a cosmetic field that was never carried cannot name a country"
        );
        assert!(crate::binding::admitted_cosmetic_override(&Leaf::Unobtained).is_err());
        assert!(
            crate::binding::admitted_cosmetic_override(&Leaf::Measured(Some(String::new())))
                .is_err()
        );
    }

    #[test]
    fn occupied_country_name_prefers_the_ideology_specific_key() {
        let mut localisation = Localisation::default();
        localisation
            .entries
            .insert("FRA_democratic".into(), "French Republic".into());
        localisation
            .entries
            .insert("FRA".into(), "France".into());
        assert_eq!(
            resolve_admitted_country_name(
                &localisation,
                "FRA",
                &Leaf::Measured("democratic".into()),
                &Leaf::Measured(None),
            ),
            Ok("French Republic")
        );
        let rejected = resolve_admitted_country_name(
            &localisation,
            "FRA",
            &Leaf::Unobtained,
            &Leaf::Measured(None),
        )
        .expect_err("unobtained ideology must reject");
        assert_eq!(rejected.0, "XUI16_IDEOLOGY_IDENTITY_UNAVAILABLE");
        let unread = resolve_admitted_country_name(
            &localisation,
            "FRA",
            &Leaf::Measured("democratic".into()),
            &Leaf::Absent,
        )
        .expect_err("an unread cosmetic field must reject");
        assert_eq!(unread.0, "XUI16_COSMETIC_IDENTITY_UNAVAILABLE");
    }

    #[test]
    fn equipment_localisation_is_scoped_to_the_selected_controller() {
        let event = |controller: &str, id: u32, token: &str| {
            replay_core::OccupationHistoryExactRow {
                stable_event_id: Leaf::Measured(format!("{controller}|event")),
                source_ordinal: Leaf::Measured(0),
                controller_tag: Leaf::Measured(controller.into()),
                state_id: Leaf::Measured(1),
                occupied_country_tag: Leaf::Measured("FRA".into()),
                event_hour_or_engine_date: Leaf::Measured("1942.1.1.1".into()),
                attack_blocked: Leaf::Measured(true),
                resistance_activity: Leaf::Absent,
                lost_manpower: Leaf::Measured(0),
                equipment_loss_rows: Leaf::Measured(vec![
                    replay_core::OccupationEquipmentExactRow {
                        source_ordinal: Leaf::Measured(0),
                        equipment_id: Leaf::Measured(id),
                        equipment_type: Leaf::Measured(70),
                        equipment_token: Leaf::Measured(token.into()),
                        amount: Leaf::Measured(1.0),
                    },
                ]),
                equipment_loss_children_complete: Leaf::Measured(true),
            }
        };
        let history = vec![
            event("GER", 1, "selected_equipment"),
            event("ITA", 2, "unselected_missing_localisation"),
        ];
        let mut localisation = Localisation::default();
        localisation
            .entries
            .insert("selected_equipment".into(), "Selected Equipment".into());

        let names = equipment_display_names_from_history(&history, &localisation, "GER")
            .expect("unselected equipment must not impose a localisation dependency");
        assert_eq!(names.len(), 1);
        assert_eq!(
            names.get(&(1, 70)).map(String::as_str),
            Some("Selected Equipment")
        );
        assert!(!names.contains_key(&(2, 70)));
    }

    #[test]
    fn resistance_activity_is_exact_optional_only_for_blocked_events() {
        assert_eq!(
            admitted_resistance_activity(
                false,
                &Leaf::Measured("sabotage_infrastructure".into())
            ),
            Ok(Some("sabotage_infrastructure".into()))
        );
        assert_eq!(
            admitted_resistance_activity(true, &Leaf::Absent),
            Ok(None)
        );
        assert!(admitted_resistance_activity(false, &Leaf::Absent).is_err());
        assert!(admitted_resistance_activity(true, &Leaf::Unobtained).is_err());
        assert!(
            admitted_resistance_activity(false, &Leaf::Measured(String::new())).is_err()
        );
    }
}

fn validate_occupation_gui_sources(resolver: &AssetResolver) -> UiResult<()> {
    for source in [
        COUNTRY_OCCUPATION_GUI,
        GARRISON_LOG_GUI,
        COMBAT_LOG_GUI,
        COUNTRY_STATE_GUI,
    ] {
        let file = resolver.resolve_file(source)?;
        let document = parse_file_refusing_decode_damage(&file.path, &file.rel_path)?;
        if !document.recoveries.is_empty() {
            return Err(GameUiError::parse(format!(
                "{source}: XUI16 effective GUI parse recovered {:?}",
                document.recoveries
            )));
        }
        if document.root.fields().is_empty() {
            return Err(GameUiError::parse(format!(
                "{source}: XUI16 effective GUI is empty"
            )));
        }
    }
    Ok(())
}

pub fn load_effective_occupation_law_definitions(
    resolver: &AssetResolver,
    localisation: &Localisation,
) -> UiResult<BTreeMap<u32, OccupationLawDefinition>> {
    let mut files =
        resolver.list_files_recursive("common/occupation_laws", Some("txt"))?;
    files.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    if files.is_empty() {
        return Err(GameUiError::missing(
            "effective common/occupation_laws catalog is empty",
        ));
    }
    let mut definitions = BTreeMap::new();
    let mut tokens = BTreeSet::new();
    for file in files {
        let document = parse_file_refusing_decode_damage(&file.path, &file.rel_path)?;
        if !document.recoveries.is_empty() {
            return Err(GameUiError::parse(format!(
                "{}: occupation-law parse recovered {:?}",
                file.rel_path, document.recoveries
            )));
        }
        for entry in document.root.fields() {
            let Some(token) = entry.key.as_deref() else {
                return Err(GameUiError::parse(format!(
                    "{}: anonymous occupation-law definition",
                    file.rel_path
                )));
            };
            if token.starts_with('@') {
                continue;
            }
            if entry.value.as_object().is_none() {
                return Err(GameUiError::parse(format!(
                    "{}: occupation law {token} is not a block",
                    file.rel_path
                )));
            }
            if !tokens.insert(token.to_string()) {
                return Err(GameUiError::parse(format!(
                    "effective occupation-law token {token} is repeated"
                )));
            }
            let icon_raw = entry.value.first_scalar_ci("icon").ok_or_else(|| {
                GameUiError::parse(format!(
                    "{}: occupation law {token} has no authored icon",
                    file.rel_path
                ))
            })?;
            let icon_frame = icon_raw
                .rsplit(':')
                .next()
                .and_then(|value| value.trim().parse::<usize>().ok())
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    GameUiError::parse(format!(
                        "{}: occupation law {token} icon {icon_raw:?} has no positive frame",
                        file.rel_path
                    ))
                })?;
            let gui_order = entry
                .value
                .first_scalar_ci("gui_order")
                .unwrap_or("0")
                .parse::<i64>()
                .map_err(|_| {
                    GameUiError::parse(format!(
                        "{}: occupation law {token} has invalid gui_order",
                        file.rel_path
                    ))
                })?;
            let display_name = localisation.get(token).unwrap_or("");
            let hash = fnv1a32(token);
            if definitions
                .insert(
                    hash,
                    OccupationLawDefinition {
                        token: token.to_string(),
                        display_name: display_name.to_string(),
                        icon_frame,
                        gui_order,
                    },
                )
                .is_some()
            {
                return Err(GameUiError::parse(format!(
                    "occupation-law FNV-1a collision at {hash}"
                )));
            }
        }
    }
    if definitions.is_empty() {
        return Err(GameUiError::missing(
            "effective occupation-law catalog has no definitions",
        ));
    }
    Ok(definitions)
}

fn occupation_observer_collections(
    frame: &WorldFrame,
    selected_country: &str,
    supplement: &OccupationD66ControllerSupplement,
) -> std::result::Result<OccupationObserverCollections, OccupationAssemblyGap> {
    if frame.occupation_status() != ViewStatus::Present {
        return Err(OccupationAssemblyGap {
            code: "XUI16_OCCUPATION_STATE_UNAVAILABLE",
            detail: format!("occupation status is {:?}", frame.occupation_status()),
        });
    }
    let frame_hour = u64::try_from(frame.hour_index()).map_err(|_| OccupationAssemblyGap {
        code: "XUI16_NEGATIVE_FRAME_HOUR",
        detail: format!("frame hour {} is negative", frame.hour_index()),
    })?;
    for (&state_id, row) in frame.occupation_by_state().iter().filter(|(_, row)| {
        row.occupier_tag.measured().map(String::as_str) == Some(selected_country)
    }) {
        if measured_u32(&row.occupation_controller_tag_db_id)
            != Some(supplement.controller_country_tag_db_id)
        {
            return Err(OccupationAssemblyGap {
                code: "XUI16_STATE_CONTROLLER_IDENTITY_MISMATCH",
                detail: format!(
                    "state {state_id} controller tag matches {selected_country} but its D18 identity does not"
                ),
            });
        }
    }
    let state_ids = frame
        .occupation_by_state()
        .iter()
        .filter(|(_, row)| {
            row.occupier_tag.measured().map(String::as_str) == Some(selected_country)
        })
        .map(|(&state_id, _)| state_id)
        .collect::<BTreeSet<_>>();
    if supplement
        .state_source_order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != state_ids
        || supplement.state_source_order.len() != state_ids.len()
    {
        return Err(OccupationAssemblyGap {
            code: "XUI16_STATE_ORDER_MISMATCH",
            detail: "D66 state source order does not exactly cover occupation state identities"
                .into(),
        });
    }
    let states = semantic_collection(
        OCCUPATION_COLLECTION_STATES,
        frame_hour,
        supplement
            .state_source_order
            .iter()
            .map(|state_id| format!("state:{state_id}")),
        Some(selected_country),
    )?;

    let Leaf::Measured(true) = frame.occupation_history_complete() else {
        return Err(OccupationAssemblyGap {
            code: "XUI16_HISTORY_INCOMPLETE",
            detail: "occupation history completeness is not measured true".into(),
        });
    };
    let Leaf::Measured(history) = frame.occupation_history() else {
        return Err(OccupationAssemblyGap {
            code: "XUI16_HISTORY_UNAVAILABLE",
            detail: "occupation history rows are not measured".into(),
        });
    };
    let mut activity_ids = Vec::with_capacity(history.len());
    let mut equipment_keys = BTreeSet::new();
    for (expected_ordinal, event) in history.iter().enumerate() {
        let source_ordinal = measured_i64(&event.source_ordinal)
            .and_then(|value| usize::try_from(value).ok());
        if source_ordinal != Some(expected_ordinal) {
            return Err(OccupationAssemblyGap {
                code: "XUI16_HISTORY_SOURCE_ORDER_INVALID",
                detail: format!("history row {expected_ordinal} has ordinal {source_ordinal:?}"),
            });
        }
        let stable_id = nonempty_leaf(&event.stable_event_id).map_err(|_| OccupationAssemblyGap {
            code: "XUI16_HISTORY_IDENTITY_UNAVAILABLE",
            detail: format!("history row {expected_ordinal} has no stable event identity"),
        })?;
        let controller_tag =
            nonempty_leaf(&event.controller_tag).map_err(|_| OccupationAssemblyGap {
                code: "XUI16_HISTORY_CONTROLLER_UNAVAILABLE",
                detail: format!("history row {expected_ordinal} has no controller identity"),
            })?;
        let selected = controller_tag == selected_country;
        if selected {
            activity_ids.push(stable_id.to_string());
        }
        let Leaf::Measured(true) = &event.equipment_loss_children_complete else {
            return Err(OccupationAssemblyGap {
                code: "XUI16_HISTORY_EQUIPMENT_INCOMPLETE",
                detail: format!("history row {expected_ordinal} equipment children incomplete"),
            });
        };
        let Leaf::Measured(rows) = &event.equipment_loss_rows else {
            return Err(OccupationAssemblyGap {
                code: "XUI16_HISTORY_EQUIPMENT_UNAVAILABLE",
                detail: format!("history row {expected_ordinal} equipment children unavailable"),
            });
        };
        for (child_ordinal, row) in rows.iter().enumerate() {
            if measured_i64(&row.source_ordinal)
                .and_then(|value| usize::try_from(value).ok())
                != Some(child_ordinal)
            {
                return Err(OccupationAssemblyGap {
                    code: "XUI16_EQUIPMENT_LOSS_SOURCE_ORDER_INVALID",
                    detail: format!(
                        "history row {expected_ordinal} equipment child {child_ordinal} has a different source ordinal"
                    ),
                });
            }
            let key = [
                measured_u32(&row.equipment_id).unwrap_or(0),
                measured_u32(&row.equipment_type).unwrap_or(0),
            ];
            if key[0] == 0 || key[1] == 0 {
                return Err(OccupationAssemblyGap {
                    code: "XUI16_EQUIPMENT_LOSS_IDENTITY_UNAVAILABLE",
                    detail: format!(
                        "history row {expected_ordinal} equipment child {child_ordinal} has no portable identity"
                    ),
                });
            }
            if selected {
                equipment_keys.insert(key);
            }
        }
    }
    let activity_id_set = activity_ids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    if supplement
        .activity_source_order
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != activity_id_set
        || supplement.activity_source_order.len() != activity_ids.len()
    {
        return Err(OccupationAssemblyGap {
            code: "XUI16_ACTIVITY_ORDER_MISMATCH",
            detail: "D66 activity order does not exactly cover selected-country history rows"
                .into(),
        });
    }
    if supplement
        .activity_location_text
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != activity_id_set
    {
        return Err(OccupationAssemblyGap {
            code: "XUI16_ACTIVITY_TEXT_COVERAGE_MISMATCH",
            detail: "D66 activity text does not exactly cover selected-country history rows"
                .into(),
        });
    }
    let activities = semantic_collection(
        OCCUPATION_COLLECTION_ACTIVITY,
        frame_hour,
        supplement.activity_source_order.iter().cloned(),
        Some(selected_country),
    )?;
    if supplement
        .equipment_loss_source_order
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        != equipment_keys
        || supplement.equipment_loss_source_order.len() != equipment_keys.len()
    {
        return Err(OccupationAssemblyGap {
            code: "XUI16_EQUIPMENT_LOSS_ORDER_MISMATCH",
            detail:
                "D66 equipment-loss order does not exactly cover aggregated equipment identities"
                    .into(),
        });
    }
    let equipment_losses = semantic_collection(
        OCCUPATION_COLLECTION_EQUIPMENT_LOSSES,
        frame_hour,
        supplement
            .equipment_loss_source_order
            .iter()
            .map(|pair| format!("equipment:{}:{}", pair[0], pair[1])),
        Some(selected_country),
    )?;

    let alert = &supplement.garrison_alert;
    let expected_alerts = if alert.predicate_returned {
        Vec::new()
    } else {
        vec![selected_country.to_owned()]
    };
    if supplement.alert_controller_source_order != expected_alerts {
        return Err(OccupationAssemblyGap {
            code: "XUI16_ALERT_ORDER_MISMATCH",
            detail: "D66 alert order differs from the exact active alert identities".into(),
        });
    }
    if supplement
        .alert_resolved_text
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>()
        != expected_alerts.iter().cloned().collect::<BTreeSet<_>>()
    {
        return Err(OccupationAssemblyGap {
            code: "XUI16_ALERT_TEXT_COVERAGE_MISMATCH",
            detail: "D66 alert text does not exactly cover selected-country active alerts".into(),
        });
    }
    let alert_ids = if alert.predicate_returned {
        Vec::new()
    } else {
        vec![format!(
            "alert_not_enough_garrison:{}:{}",
            alert.game_hour_raw, supplement.controller_country_tag_db_id
        )]
    };
    let alerts = semantic_collection(
        OCCUPATION_COLLECTION_ALERTS,
        frame_hour,
        alert_ids.into_iter(),
        Some(selected_country),
    )?;
    Ok(OccupationObserverCollections {
        states,
        activities,
        equipment_losses,
        alerts,
    })
}

fn semantic_collection(
    collection_id: &'static str,
    frame_hour: u64,
    ids: impl IntoIterator<Item = String>,
    context_country: Option<&str>,
) -> std::result::Result<ObserverSemanticCollection, OccupationAssemblyGap> {
    let rows = ids
        .into_iter()
        .enumerate()
        .map(|(ordinal, stable_item_id)| {
            u32::try_from(ordinal)
                .map(|source_ordinal| ObserverSemanticRow {
                    stable_item_id,
                    source_ordinal,
                })
                .map_err(|_| OccupationAssemblyGap {
                    code: "XUI03_COLLECTION_TOO_LARGE",
                    detail: format!("{collection_id} exceeds u32 source ordinals"),
                })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let collection = ObserverSemanticCollection {
        collection_id,
        frame_hour,
        context_country: context_country.map(str::to_owned),
        rows,
    };
    if !collection.validate(collection_id, frame_hour) {
        return Err(OccupationAssemblyGap {
            code: "XUI03_COLLECTION_INVALID",
            detail: format!("{collection_id} has duplicate or invalid semantic identities"),
        });
    }
    Ok(collection)
}

fn equipment_by_state(
    occupation_rows: &BTreeMap<u32, OccupationRow>,
) -> std::result::Result<BTreeMap<u32, Vec<GarrisonEquipmentRow>>, OccupationAssemblyGap> {
    occupation_rows
        .iter()
        .map(|(&state_id, row)| {
            typed_garrison_equipment(row)
                .map_err(|error| OccupationAssemblyGap {
                    code: "XUI16_GARRISON_EQUIPMENT_INVALID",
                    detail: format!("state {state_id}: {error:?}"),
                })?
                .ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_GARRISON_EQUIPMENT_UNAVAILABLE",
                    detail: format!("state {state_id} has no complete typed equipment rows"),
                })
                .map(|rows| (state_id, rows))
        })
        .collect()
}

fn occupation_visible_text(
    frame: &WorldFrame,
    occupation_rows: &BTreeMap<u32, OccupationRow>,
    supplement: &OccupationD66ControllerSupplement,
    localisation: &Localisation,
) -> std::result::Result<BTreeMap<u32, OccupationVisibleText>, OccupationAssemblyGap> {
    if frame.country_identities_status() != ViewStatus::Present {
        return Err(OccupationAssemblyGap {
            code: "XUI16_COUNTRY_IDENTITY_UNAVAILABLE",
            detail: format!(
                "country identity status is {:?}",
                frame.country_identities_status()
            ),
        });
    }
    if supplement.formatted_text_by_state.len() != occupation_rows.len() {
        return Err(OccupationAssemblyGap {
            code: "XUI16_STATE_TEXT_COVERAGE_MISMATCH",
            detail: "D66 formatted state text does not exactly cover occupation states".into(),
        });
    }
    occupation_rows
        .iter()
        .map(|(&state_id, row)| {
            let formatted = supplement
                .formatted_text_by_state
                .get(&state_id)
                .ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_STATE_TEXT_UNAVAILABLE",
                    detail: format!("state {state_id} has no exact D66 formatted text"),
                })?;
            let state_key = format!("STATE_{state_id}");
            let state_name = localisation.get(&state_key).ok_or_else(|| OccupationAssemblyGap {
                code: "XUI16_STATE_LOCALISATION_UNAVAILABLE",
                detail: format!("effective localisation has no {state_key}"),
            })?;
            let occupied_tag = nonempty_leaf(&row.occupied_country_tag).map_err(|_| {
                OccupationAssemblyGap {
                    code: "XUI16_OCCUPIED_COUNTRY_UNAVAILABLE",
                    detail: format!("state {state_id} has no occupied-country identity"),
                }
            })?;
            let identity = frame
                .country_identity(occupied_tag)
                .ok_or_else(|| OccupationAssemblyGap {
                    code: "XUI16_COUNTRY_IDENTITY_UNAVAILABLE",
                    detail: format!(
                        "state {state_id} occupied country {occupied_tag} has no typed identity"
                    ),
                })?;
            let occupied_country_name = resolve_admitted_country_name(
                localisation,
                occupied_tag,
                &identity.ruling_ideology,
                &identity.cosmetic_tag,
            )
            .map_err(|(code, detail)| OccupationAssemblyGap {
                code,
                detail: format!("state {state_id}: {detail}"),
            })?;
            let text = OccupationVisibleText {
                state_name: state_name.to_string(),
                occupied_country_name: occupied_country_name.to_owned(),
                resistance_text: formatted.resistance_text.clone(),
                resistance_speed_text: formatted.resistance_speed_text.clone(),
                compliance_text: formatted.compliance_text.clone(),
                compliance_speed_text: formatted.compliance_speed_text.clone(),
                garrison_manpower_need_text: formatted.garrison_manpower_need_text.clone(),
                garrison_required_text: formatted.garrison_required_text.clone(),
            };
            if !visible_text_complete(&text) {
                return Err(OccupationAssemblyGap {
                    code: "XUI16_STATE_TEXT_EMPTY",
                    detail: format!("state {state_id} has an empty final text field"),
                });
            }
            Ok((state_id, text))
        })
        .collect()
}

fn admitted_optional_country_name_dimension<'a>(
    dimension: &'a Leaf<String>,
    label: &'static str,
) -> std::result::Result<Option<&'a str>, String> {
    match dimension {
        Leaf::Absent => Ok(None),
        Leaf::Measured(value) if !value.trim().is_empty() => Ok(Some(value)),
        Leaf::Measured(_) => Err(format!("measured {label} is empty")),
        Leaf::Unobtained => Err(format!("{label} is unobtained")),
    }
}

fn resolve_admitted_country_name<'a>(
    localisation: &'a Localisation,
    base_tag: &str,
    ruling_ideology: &Leaf<String>,
    cosmetic_tag: &Leaf<Option<String>>,
) -> std::result::Result<&'a str, (&'static str, String)> {
    let cosmetic_tag = crate::binding::admitted_cosmetic_override(cosmetic_tag)
        .map_err(|detail| ("XUI16_COSMETIC_IDENTITY_UNAVAILABLE", detail.to_string()))?;
    let ruling_ideology =
        admitted_optional_country_name_dimension(ruling_ideology, "ruling ideology")
            .map_err(|detail| ("XUI16_IDEOLOGY_IDENTITY_UNAVAILABLE", detail))?;
    crate::binding::country_name_key_candidates(base_tag, ruling_ideology, cosmetic_tag)
        .into_iter()
        .find_map(|key| localisation.get(&key))
        .ok_or_else(|| {
            (
                "XUI16_COUNTRY_LOCALISATION_UNAVAILABLE",
                format!(
                    "effective localisation has no admitted occupied-country key for {base_tag}"
                ),
            )
        })
}

fn equipment_display_names(
    frame: &WorldFrame,
    localisation: &Localisation,
    selected_country: &str,
) -> std::result::Result<BTreeMap<(u32, u32), String>, OccupationAssemblyGap> {
    let Leaf::Measured(history) = frame.occupation_history() else {
        return Err(OccupationAssemblyGap {
            code: "XUI16_HISTORY_UNAVAILABLE",
            detail: "occupation history rows are not measured".into(),
        });
    };
    equipment_display_names_from_history(history, localisation, selected_country)
}

fn equipment_display_names_from_history(
    history: &[replay_core::OccupationHistoryExactRow],
    localisation: &Localisation,
    selected_country: &str,
) -> std::result::Result<BTreeMap<(u32, u32), String>, OccupationAssemblyGap> {
    let mut names = BTreeMap::new();
    for event in history {
        let controller_tag =
            nonempty_leaf(&event.controller_tag).map_err(|_| OccupationAssemblyGap {
                code: "XUI16_HISTORY_CONTROLLER_UNAVAILABLE",
                detail: "history row has no controller identity".into(),
            })?;
        if controller_tag != selected_country {
            continue;
        }
        let Leaf::Measured(rows) = &event.equipment_loss_rows else {
            return Err(OccupationAssemblyGap {
                code: "XUI16_HISTORY_EQUIPMENT_UNAVAILABLE",
                detail: "history equipment-loss rows are unavailable".into(),
            });
        };
        for row in rows {
            let equipment_id = measured_u32(&row.equipment_id).unwrap_or(0);
            let equipment_type = measured_u32(&row.equipment_type).unwrap_or(0);
            let token = nonempty_leaf(&row.equipment_token).map_err(|_| OccupationAssemblyGap {
                code: "XUI16_EQUIPMENT_TOKEN_UNAVAILABLE",
                detail: format!("equipment {equipment_id}:{equipment_type} has no token"),
            })?;
            let display_name = localisation.get(token).ok_or_else(|| OccupationAssemblyGap {
                code: "XUI16_EQUIPMENT_LOCALISATION_UNAVAILABLE",
                detail: format!("effective localisation has no equipment key {token}"),
            })?;
            if let Some(prior) =
                names.insert((equipment_id, equipment_type), display_name.to_string())
            {
                if prior != display_name {
                    return Err(OccupationAssemblyGap {
                        code: "XUI16_EQUIPMENT_IDENTITY_CONFLICT",
                        detail: format!(
                            "equipment {equipment_id}:{equipment_type} resolves to conflicting names"
                        ),
                    });
                }
            }
        }
    }
    Ok(names)
}

impl AdmittedOccupationSurface {
    pub fn is_product_authority(&self) -> bool {
        self.product_authority
    }
    pub fn source_record_id(&self) -> &str {
        &self.source_record_id
    }
    pub fn states(&self) -> &[OccupationStatePresentation] {
        &self.states
    }
    pub fn activities(&self) -> &[GarrisonActivityRow] {
        &self.activities
    }
    pub fn equipment_losses(&self) -> &[GarrisonEquipmentLossRow] {
        &self.equipment_losses
    }
    pub fn alerts(&self) -> &[OccupationAlertRow] {
        &self.alerts
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OccupationRejection {
    MissingProvenance,
    IncompleteProducer,
    InvalidSemanticCollection,
    CollectionPopulationMismatch,
    MissingVisibleText,
    MissingOrInvalidOccupationValue,
    MissingLawDefinition,
    LawHashMismatch,
    MissingGarrisonTemplate,
    InvalidGarrisonEquipment,
    InvalidGarrisonLog,
    InvalidAlertRows,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OccupationMapModeStateInput {
    pub state_id: u32,
    pub resistance: Option<f32>,
    pub compliance: Option<f32>,
}

pub fn occupation_map_mode_inputs(
    rows: &BTreeMap<u32, OccupationRow>,
) -> Vec<OccupationMapModeStateInput> {
    rows.iter()
        .map(|(&state_id, row)| OccupationMapModeStateInput {
            state_id,
            resistance: measured_f64(&row.resistance)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|v| v as f32),
            compliance: measured_f64(&row.compliance)
                .filter(|v| v.is_finite() && (0.0..=100.0).contains(v))
                .map(|v| v as f32),
        })
        .collect()
}

pub fn admit_occupation_fixture_from_world(
    request: OccupationFixtureSurfaceRequest<'_>,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    admit_resolved_occupation_surface_from_world(request, false)
}

pub fn occupation_alert_product_binding(
    frame: &WorldFrame,
    pov_tag: &str,
    surface: &AdmittedOccupationSurface,
) -> Result<Option<OccupationAlertProductBinding>, OccupationRejection> {
    if !surface.is_product_authority() || pov_tag.trim().is_empty() {
        return Err(OccupationRejection::InvalidAlertRows);
    }
    let feed = crate::alerts::alert_feed_from_frame(frame, pov_tag);
    if !feed.available {
        return Err(OccupationRejection::InvalidAlertRows);
    }
    let slot_rows = feed
        .rows
        .iter()
        .filter(|row| u32::from(row.alert_type) == OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT)
        .collect::<Vec<_>>();
    let slot_icons = feed
        .icons
        .iter()
        .filter(|icon| u32::from(icon.alert_type) == OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT)
        .collect::<Vec<_>>();
    match surface.alerts() {
        [] if slot_rows.is_empty() && slot_icons.is_empty() => Ok(None),
        [alert]
            if alert.controller_tag == pov_tag
                && alert.registry_slot == OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT
                && !alert.predicate_returned
                && slot_rows.len() == 1
                && slot_icons.len() == 1
                && slot_rows[0].source_ordinal == slot_icons[0].source_ordinal =>
        {
            Ok(Some(OccupationAlertProductBinding {
                alert: alert.clone(),
                icon: (*slot_icons[0]).clone(),
            }))
        }
        _ => Err(OccupationRejection::InvalidAlertRows),
    }
}

fn admit_resolved_occupation_surface_from_world(
    request: OccupationFixtureSurfaceRequest<'_>,
    product_authority: bool,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    let frame_hour = u64::try_from(request.frame.hour_index())
        .map_err(|_| OccupationRejection::InvalidSemanticCollection)?;
    if !request
        .collections
        .states
        .validate(OCCUPATION_COLLECTION_STATES, frame_hour)
        || !request
            .collections
            .activities
            .validate(OCCUPATION_COLLECTION_ACTIVITY, frame_hour)
        || !request
            .collections
            .equipment_losses
            .validate(OCCUPATION_COLLECTION_EQUIPMENT_LOSSES, frame_hour)
        || !request
            .collections
            .alerts
            .validate(OCCUPATION_COLLECTION_ALERTS, frame_hour)
    {
        return Err(OccupationRejection::InvalidSemanticCollection);
    }
    let (activities, equipment_losses) = production_garrison_history_rows(
        request.frame,
        request.activity_location_text,
        request.equipment_display_names,
        request.collections.states.context_country.as_deref(),
    )?;
    let alerts = match request.controller_supplement {
        Some(controller) => product_garrison_alert_rows(
            controller,
            request.alert_resolved_text,
            &request.collections.alerts,
        )?,
        None => fixture_garrison_alert_rows(
            request.frame,
            request.alert_resolved_text,
            &request.collections.alerts,
        )?,
    };
    admit_occupation_surface_with_authority(
        OccupationSurfaceRequest {
            source_record_id: request.source_record_id,
            occupation_rows: request.occupation_rows,
            visible_text_by_state: request.visible_text_by_state,
            law_definitions_by_hash: request.law_definitions_by_hash,
            garrison_templates: request.garrison_templates,
            equipment_by_state: request.equipment_by_state,
            equipment_complete: request.equipment_complete,
            collections: request.collections,
            activities: &activities,
            activities_complete: true,
            equipment_losses: &equipment_losses,
            equipment_losses_complete: true,
            alerts: &alerts,
            alerts_complete: true,
        },
        product_authority,
    )
}

fn production_garrison_history_rows(
    frame: &WorldFrame,
    activity_location_text: &BTreeMap<String, String>,
    equipment_display_names: &BTreeMap<(u32, u32), String>,
    context_country: Option<&str>,
) -> Result<(Vec<GarrisonActivityRow>, Vec<GarrisonEquipmentLossRow>), OccupationRejection> {
    let Leaf::Measured(true) = frame.occupation_history_complete() else {
        return Err(OccupationRejection::IncompleteProducer);
    };
    let Leaf::Measured(history) = frame.occupation_history() else {
        return Err(OccupationRejection::IncompleteProducer);
    };
    let mut activities = Vec::with_capacity(history.len());
    let mut losses = BTreeMap::<(u32, u32), GarrisonEquipmentLossRow>::new();
    let mut stable_ids = BTreeSet::new();
    for event in history {
        let controller_tag = nonempty_leaf(&event.controller_tag)
            .map_err(|_| OccupationRejection::InvalidGarrisonLog)?;
        if context_country.is_some_and(|selected| selected != controller_tag) {
            continue;
        }
        let stable_id = nonempty_leaf(&event.stable_event_id)
            .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
            .to_owned();
        if !stable_ids.insert(stable_id.clone()) {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
        let location_text = activity_location_text
            .get(&stable_id)
            .filter(|value| !value.trim().is_empty())
            .cloned()
            .ok_or(OccupationRejection::MissingVisibleText)?;
        let Leaf::Measured(true) = &event.equipment_loss_children_complete else {
            return Err(OccupationRejection::IncompleteProducer);
        };
        let Leaf::Measured(equipment) = &event.equipment_loss_rows else {
            return Err(OccupationRejection::IncompleteProducer);
        };
        let mut lost_equipment = 0i64;
        for (expected_ordinal, row) in equipment.iter().enumerate() {
            let ordinal = measured_i64(&row.source_ordinal)
                .filter(|value| *value >= 0)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            if usize::try_from(ordinal).ok() != Some(expected_ordinal) {
                return Err(OccupationRejection::InvalidGarrisonLog);
            }
            let equipment_id =
                measured_u32(&row.equipment_id).ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let equipment_type =
                measured_u32(&row.equipment_type).ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let equipment_token = nonempty_leaf(&row.equipment_token)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned();
            let amount = measured_f64(&row.amount)
                .filter(|value| {
                    value.is_finite()
                        && *value >= 0.0
                        && *value <= MAX_EXACT_F64_INTEGER
                        && value.fract() == 0.0
                })
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let amount_i64 = amount as i64;
            if amount_i64 as f64 != amount {
                return Err(OccupationRejection::InvalidGarrisonLog);
            }
            lost_equipment = lost_equipment
                .checked_add(amount_i64)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?;
            let display_name = equipment_display_names
                .get(&(equipment_id, equipment_type))
                .filter(|value| !value.trim().is_empty())
                .cloned()
                .ok_or(OccupationRejection::MissingVisibleText)?;
            let key = (equipment_id, equipment_type);
            match losses.entry(key) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(GarrisonEquipmentLossRow {
                        stable_id: format!("equipment:{equipment_id}:{equipment_type}"),
                        equipment_id,
                        equipment_type,
                        equipment_token,
                        display_name,
                        amount_lost: amount,
                        contributing_activity_ids: vec![stable_id.clone()],
                    });
                }
                std::collections::btree_map::Entry::Occupied(mut entry) => {
                    let aggregate = entry.get_mut();
                    if aggregate.equipment_token != equipment_token
                        || aggregate.display_name != display_name
                    {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    let aggregate_i64 = aggregate.amount_lost as i64;
                    if aggregate_i64 as f64 != aggregate.amount_lost {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    let aggregate_i64 = aggregate_i64
                        .checked_add(amount_i64)
                        .ok_or(OccupationRejection::InvalidGarrisonLog)?;
                    let aggregate_f64 = aggregate_i64 as f64;
                    if aggregate_f64 as i64 != aggregate_i64 {
                        return Err(OccupationRejection::InvalidGarrisonLog);
                    }
                    aggregate.amount_lost = aggregate_f64;
                    if aggregate.contributing_activity_ids.last() != Some(&stable_id) {
                        aggregate.contributing_activity_ids.push(stable_id.clone());
                    }
                }
            }
        }
        let attack_blocked = match &event.attack_blocked {
            Leaf::Measured(value) => *value,
            _ => return Err(OccupationRejection::InvalidGarrisonLog),
        };
        let resistance_activity =
            admitted_resistance_activity(attack_blocked, &event.resistance_activity)?;
        activities.push(GarrisonActivityRow {
            stable_id,
            controller_tag: controller_tag.to_owned(),
            date_text: nonempty_leaf(&event.event_hour_or_engine_date)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned(),
            location_text,
            state_id: measured_u32(&event.state_id)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?,
            occupied_country_tag: nonempty_leaf(&event.occupied_country_tag)
                .map_err(|_| OccupationRejection::InvalidGarrisonLog)?
                .to_owned(),
            attack_blocked,
            resistance_activity,
            lost_manpower: measured_i64(&event.lost_manpower)
                .filter(|value| *value >= 0)
                .ok_or(OccupationRejection::InvalidGarrisonLog)?,
            lost_equipment,
        });
    }
    Ok((activities, losses.into_values().collect()))
}

fn admitted_resistance_activity(
    attack_blocked: bool,
    resistance_activity: &Leaf<String>,
) -> Result<Option<String>, OccupationRejection> {
    match resistance_activity {
        Leaf::Measured(value) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Leaf::Absent if attack_blocked => Ok(None),
        Leaf::Absent | Leaf::Measured(_) | Leaf::Unobtained => {
            Err(OccupationRejection::InvalidGarrisonLog)
        }
    }
}

fn fixture_garrison_alert_rows(
    frame: &WorldFrame,
    resolved_text: &BTreeMap<String, String>,
    collection: &ObserverSemanticCollection,
) -> Result<Vec<OccupationAlertRow>, OccupationRejection> {
    let view = frame.occupation_garrison_alert_exact();
    if view.status != ViewStatus::Present {
        return Err(OccupationRejection::IncompleteProducer);
    }
    let alert = view
        .alert
        .as_ref()
        .ok_or(OccupationRejection::IncompleteProducer)?;
    if alert.predicate_returned {
        return Ok(Vec::new());
    }
    if collection
        .context_country
        .as_deref()
        .is_some_and(|country| country != alert.controller_tag)
    {
        return Ok(Vec::new());
    }
    let text = resolved_text
        .get(&alert.controller_tag)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or(OccupationRejection::MissingVisibleText)?;
    let stable_id = format!(
        "alert_not_enough_garrison:{}:{}",
        alert.game_hour_raw, alert.controller_tag_db_id
    );
    let order = collection
        .rows
        .iter()
        .find(|row| row.stable_item_id == stable_id)
        .map(|row| row.source_ordinal)
        .ok_or(OccupationRejection::InvalidAlertRows)?;
    Ok(vec![OccupationAlertRow {
        stable_id,
        controller_tag: alert.controller_tag.clone(),
        kind: "alert_not_enough_garrison".into(),
        severity: "HIGH".into(),
        registry_slot: OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT,
        predicate_returned: alert.predicate_returned,
        equipment_fixed_point_divisor: i64::from(alert.equipment_fixed_point_divisor),
        visible_order: order,
        missing_manpower: alert.missing_manpower,
        missing_equipment: alert
            .missing_equipment
            .iter()
            .map(|row| GarrisonShortageEquipmentRow {
                archetype_id: row.archetype_id,
                whole_amount: row.whole_units,
            })
            .collect(),
        resolved_text: text,
    }])
}

fn product_garrison_alert_rows(
    controller: &OccupationD66ControllerSupplement,
    resolved_text: &BTreeMap<String, String>,
    collection: &ObserverSemanticCollection,
) -> Result<Vec<OccupationAlertRow>, OccupationRejection> {
    let alert = &controller.garrison_alert;
    if alert.predicate_returned {
        return Ok(Vec::new());
    }
    let controller_tag = controller.controller_country_tag.as_str();
    let text = resolved_text
        .get(controller_tag)
        .filter(|value| !value.trim().is_empty())
        .cloned()
        .ok_or(OccupationRejection::MissingVisibleText)?;
    let stable_id = format!(
        "alert_not_enough_garrison:{}:{}",
        alert.game_hour_raw, controller.controller_country_tag_db_id
    );
    let order = collection
        .rows
        .iter()
        .find(|row| row.stable_item_id == stable_id)
        .map(|row| row.source_ordinal)
        .ok_or(OccupationRejection::InvalidAlertRows)?;
    Ok(vec![OccupationAlertRow {
        stable_id,
        controller_tag: controller_tag.to_owned(),
        kind: "alert_not_enough_garrison".into(),
        severity: "HIGH".into(),
        registry_slot: OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT,
        predicate_returned: alert.predicate_returned,
        equipment_fixed_point_divisor: i64::from(alert.equipment_fixed_point_divisor),
        visible_order: order,
        missing_manpower: alert.missing_manpower,
        missing_equipment: alert
            .missing_equipment
            .iter()
            .map(|row| GarrisonShortageEquipmentRow {
                archetype_id: row.archetype_id,
                whole_amount: row.whole_units,
            })
            .collect(),
        resolved_text: text,
    }])
}

pub fn admit_occupation_fixture_surface(
    request: OccupationSurfaceRequest<'_>,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    admit_occupation_surface_with_authority(request, false)
}

fn admit_occupation_surface_with_authority(
    request: OccupationSurfaceRequest<'_>,
    product_authority: bool,
) -> Result<AdmittedOccupationSurface, OccupationRejection> {
    if request.source_record_id.trim().is_empty() {
        return Err(OccupationRejection::MissingProvenance);
    }
    if !request.activities_complete
        || !request.equipment_losses_complete
        || !request.alerts_complete
    {
        return Err(OccupationRejection::IncompleteProducer);
    }
    let frame_hour = request.collections.states.frame_hour;
    if !request
        .collections
        .states
        .validate(OCCUPATION_COLLECTION_STATES, frame_hour)
        || !request
            .collections
            .activities
            .validate(OCCUPATION_COLLECTION_ACTIVITY, frame_hour)
        || !request
            .collections
            .equipment_losses
            .validate(OCCUPATION_COLLECTION_EQUIPMENT_LOSSES, frame_hour)
        || !request
            .collections
            .alerts
            .validate(OCCUPATION_COLLECTION_ALERTS, frame_hour)
        || [
            request.collections.activities.frame_hour,
            request.collections.equipment_losses.frame_hour,
            request.collections.alerts.frame_hour,
        ]
        .iter()
        .any(|hour| *hour != frame_hour)
        || [
            &request.collections.activities.context_country,
            &request.collections.equipment_losses.context_country,
            &request.collections.alerts.context_country,
        ]
        .iter()
        .any(|country| *country != &request.collections.states.context_country)
    {
        return Err(OccupationRejection::InvalidSemanticCollection);
    }

    let state_ids = request
        .occupation_rows
        .keys()
        .map(|id| format!("state:{id}"))
        .collect::<BTreeSet<_>>();
    if request
        .collections
        .states
        .ordered_item_ids()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        != state_ids
    {
        return Err(OccupationRejection::CollectionPopulationMismatch);
    }

    let mut states = Vec::with_capacity(request.occupation_rows.len());
    for item_id in request.collections.states.ordered_item_ids() {
        let state_id = parse_prefixed_u32(item_id, "state:")
            .ok_or(OccupationRejection::CollectionPopulationMismatch)?;
        let row = request
            .occupation_rows
            .get(&state_id)
            .ok_or(OccupationRejection::CollectionPopulationMismatch)?;
        states.push(admit_state(&request, state_id, row)?);
    }

    let activities = order_rows(request.activities, &request.collections.activities, |row| {
        row.stable_id.as_str()
    })
    .ok_or(OccupationRejection::InvalidGarrisonLog)?;
    let equipment_losses = order_rows(
        request.equipment_losses,
        &request.collections.equipment_losses,
        |row| row.stable_id.as_str(),
    )
    .ok_or(OccupationRejection::InvalidGarrisonLog)?;
    validate_activity_rows(
        &activities,
        request.collections.activities.context_country.as_deref(),
    )?;
    validate_equipment_losses(&equipment_losses, &activities)?;

    let mut alerts = request.alerts.to_vec();
    alerts.sort_by_key(|row| row.visible_order);
    if alerts
        .iter()
        .map(|row| row.stable_id.as_str())
        .collect::<Vec<_>>()
        != request
            .collections
            .alerts
            .ordered_item_ids()
            .collect::<Vec<_>>()
    {
        return Err(OccupationRejection::InvalidAlertRows);
    }
    validate_alerts(&alerts, request.occupation_rows)?;

    Ok(AdmittedOccupationSurface {
        product_authority,
        source_record_id: request.source_record_id.to_owned(),
        states,
        activities,
        equipment_losses,
        alerts,
    })
}

fn admit_state(
    request: &OccupationSurfaceRequest<'_>,
    state_id: u32,
    row: &OccupationRow,
) -> Result<OccupationStatePresentation, OccupationRejection> {
    let resistance = percent(&row.resistance)?;
    let compliance = percent(&row.compliance)?;
    let resistance_speed = finite(&row.resistance_speed)?;
    let compliance_speed = finite(&row.compliance_speed)?;
    let occupier_tag = nonempty_leaf(&row.occupier_tag)?;
    let occupied_country_tag = nonempty_leaf(&row.occupied_country_tag)?;
    let text = request
        .visible_text_by_state
        .get(&state_id)
        .filter(|text| visible_text_complete(text))
        .cloned()
        .ok_or(OccupationRejection::MissingVisibleText)?;

    if matches!(row.occupation_law_status, Leaf::Measured(_)) {
        return Err(OccupationRejection::MissingLawDefinition);
    }
    let law_hash = measured_u32(&row.occupation_law_token_id)
        .ok_or(OccupationRejection::MissingLawDefinition)?;
    let law_encoding = nonempty_leaf(&row.occupation_law_token_encoding)
        .map_err(|_| OccupationRejection::MissingLawDefinition)?;
    if !matches!(law_encoding, "fnv1a32_of_law_token" | "fnv1a32_name_hash") {
        return Err(OccupationRejection::MissingLawDefinition);
    }
    let law = request
        .law_definitions_by_hash
        .get(&law_hash)
        .filter(|law| {
            !law.token.trim().is_empty()
                && !law.display_name.trim().is_empty()
                && law.icon_frame > 0
        })
        .cloned()
        .ok_or(OccupationRejection::MissingLawDefinition)?;
    if fnv1a32(&law.token) != law_hash {
        return Err(OccupationRejection::LawHashMismatch);
    }

    let template_id = measured_u32(&row.garrison_template_id)
        .ok_or(OccupationRejection::MissingGarrisonTemplate)?;
    let garrison_template = request
        .garrison_templates
        .get(&template_id)
        .filter(|definition| {
            definition.template_id == template_id
                && !definition.display_name.trim().is_empty()
                && !definition.icon_sprite.trim().is_empty()
        })
        .cloned()
        .ok_or(OccupationRejection::MissingGarrisonTemplate)?;
    if matches!(row.garrison_signals_status, Leaf::Measured(_)) {
        return Err(OccupationRejection::MissingOrInvalidOccupationValue);
    }
    let garrison_army_manpower_need = measured_i64(&row.garrison_army_manpower_need)
        .filter(|value| *value >= 0)
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)?;
    let garrison_strength_ratio = unit_interval(&row.garrison_strength_ratio)?;
    let garrison_required = nonnegative(&row.garrison_required)?;

    let equipment = typed_garrison_equipment(row)?.unwrap_or_else(|| {
        if !request.equipment_complete {
            return Vec::new();
        }
        request
            .equipment_by_state
            .get(&state_id)
            .cloned()
            .unwrap_or_default()
    });
    if equipment.is_empty()
        && !matches!(row.garrison_equipment_complete, Leaf::Measured(true))
        && (!request.equipment_complete || !request.equipment_by_state.contains_key(&state_id))
    {
        return Err(OccupationRejection::InvalidGarrisonEquipment);
    }
    validate_equipment(&equipment)?;

    Ok(OccupationStatePresentation {
        state_id,
        occupier_tag: occupier_tag.to_owned(),
        occupied_country_tag: occupied_country_tag.to_owned(),
        text,
        resistance,
        resistance_speed,
        compliance,
        compliance_speed,
        law,
        garrison_template,
        garrison_army_manpower_need,
        garrison_strength_ratio,
        garrison_required,
        garrison_manpower_source_tag_db_id: measured_u32(&row.garrison_manpower_source_tag_db_id),
        equipment,
    })
}

fn typed_garrison_equipment(
    row: &OccupationRow,
) -> Result<Option<Vec<GarrisonEquipmentRow>>, OccupationRejection> {
    match (&row.garrison_equipment_complete, &row.garrison_equipment) {
        (Leaf::Measured(true), Leaf::Measured(rows)) => rows
            .iter()
            .map(|row| {
                Ok(GarrisonEquipmentRow {
                    equipment_id: measured_u32(&row.equipment_id)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                    equipment_type: measured_u32(&row.equipment_type)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                    equipment_token: nonempty_leaf(&row.equipment_token)
                        .map_err(|_| OccupationRejection::InvalidGarrisonEquipment)?
                        .to_owned(),
                    amount: measured_f64(&row.amount)
                        .ok_or(OccupationRejection::InvalidGarrisonEquipment)?,
                })
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
        (Leaf::Measured(true), _) | (Leaf::Measured(false), _) => {
            Err(OccupationRejection::InvalidGarrisonEquipment)
        }
        _ => Ok(None),
    }
}

pub fn apply_occupation_state_to_compose_options(
    state: &OccupationStatePresentation,
    options: &mut ComposeOptions,
) {
    options
        .text_overrides
        .insert("state_name".into(), state.text.state_name.clone());
    options
        .text_overrides
        .insert("selected_law".into(), state.law.display_name.clone());
    options.text_overrides.insert(
        "template_need_text".into(),
        state.text.garrison_required_text.clone(),
    );
    options
        .frame_overrides
        .insert("select_law_icon".into(), state.law.icon_frame);
    options
        .progress_overrides
        .insert("bar_str".into(), state.garrison_strength_ratio as f32);
}

fn validate_equipment(rows: &[GarrisonEquipmentRow]) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for row in rows {
        if row.equipment_id == 0
            || row.equipment_type == 0
            || row.equipment_token.trim().is_empty()
            || !row.amount.is_finite()
            || row.amount < 0.0
            || !ids.insert((row.equipment_id, row.equipment_type))
        {
            return Err(OccupationRejection::InvalidGarrisonEquipment);
        }
    }
    Ok(())
}

fn validate_activity_rows(
    rows: &[GarrisonActivityRow],
    context_country: Option<&str>,
) -> Result<(), OccupationRejection> {
    for row in rows {
        if row.stable_id.trim().is_empty()
            || row.controller_tag.trim().is_empty()
            || row.date_text.trim().is_empty()
            || row.location_text.trim().is_empty()
            || row.occupied_country_tag.trim().is_empty()
            || row.lost_manpower < 0
            || row.lost_equipment < 0
            || context_country.is_some_and(|country| country != row.controller_tag)
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
    }
    Ok(())
}

fn validate_equipment_losses(
    rows: &[GarrisonEquipmentLossRow],
    activities: &[GarrisonActivityRow],
) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    let activity_by_id = activities
        .iter()
        .map(|row| (row.stable_id.as_str(), row))
        .collect::<BTreeMap<_, _>>();
    let expected_total = activities.iter().try_fold(0i64, |total, row| {
        total.checked_add(row.lost_equipment)
            .ok_or(OccupationRejection::InvalidGarrisonLog)
    })?;
    let mut aggregate_total = 0i64;
    for row in rows {
        let exact_stable_id = format!("equipment:{}:{}", row.equipment_id, row.equipment_type);
        let integral_amount = row.amount_lost.is_finite()
            && row.amount_lost >= 0.0
            && row.amount_lost.fract() == 0.0
            && row.amount_lost <= MAX_EXACT_F64_INTEGER;
        if row.stable_id.trim().is_empty()
            || row.stable_id != exact_stable_id
            || row.equipment_id == 0
            || row.equipment_type == 0
            || row.equipment_token.trim().is_empty()
            || row.display_name.trim().is_empty()
            || !integral_amount
            || !ids.insert(row.stable_id.as_str())
            || row.contributing_activity_ids.is_empty()
            || row.contributing_activity_ids.iter().any(|activity_id| {
                !activity_by_id.contains_key(activity_id.as_str())
            })
            || row
                .contributing_activity_ids
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                .len()
                != row.contributing_activity_ids.len()
        {
            return Err(OccupationRejection::InvalidGarrisonLog);
        }
        let amount = row.amount_lost as i64;
        let Some(total) = aggregate_total.checked_add(amount)
        else {
            return Err(OccupationRejection::InvalidGarrisonLog);
        };
        aggregate_total = total;
    }
    if aggregate_total != expected_total {
        return Err(OccupationRejection::InvalidGarrisonLog);
    }
    Ok(())
}

fn validate_alerts(
    rows: &[OccupationAlertRow],
    occupation: &BTreeMap<u32, OccupationRow>,
) -> Result<(), OccupationRejection> {
    let mut ids = BTreeSet::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let controller_owns_occupation = occupation.values().any(|occupation_row| {
            occupation_row
                .occupier_tag
                .measured()
                .is_some_and(|tag| !tag.trim().is_empty() && tag == &row.controller_tag)
        });
        if row.stable_id.trim().is_empty()
            || row.controller_tag.trim().is_empty()
            || row.kind != "alert_not_enough_garrison"
            || row.severity != "HIGH"
            || row.registry_slot != OCCUPATION_GARRISON_ALERT_REGISTRY_SLOT
            || row.predicate_returned
            || row.equipment_fixed_point_divisor
                != OCCUPATION_GARRISON_EQUIPMENT_FIXED_POINT_DIVISOR
            || row.resolved_text.trim().is_empty()
            || row.visible_order as usize != ordinal
            || row.missing_manpower < 0
            || !controller_owns_occupation
            || !ids.insert(row.stable_id.as_str())
        {
            return Err(OccupationRejection::InvalidAlertRows);
        }
        let mut archetype_ids = BTreeSet::new();
        for equipment in &row.missing_equipment {
            if equipment.archetype_id == 0
                || equipment.whole_amount <= 0
                || !archetype_ids.insert(equipment.archetype_id)
            {
                return Err(OccupationRejection::InvalidAlertRows);
            }
        }
        if row.missing_manpower == 0 && row.missing_equipment.is_empty() {
            return Err(OccupationRejection::InvalidAlertRows);
        }
    }
    Ok(())
}

fn order_rows<T: Clone>(
    rows: &[T],
    collection: &ObserverSemanticCollection,
    id: impl Fn(&T) -> &str,
) -> Option<Vec<T>> {
    let by_id = rows
        .iter()
        .map(|row| (id(row), row))
        .collect::<BTreeMap<_, _>>();
    if by_id.len() != rows.len() || by_id.len() != collection.rows.len() {
        return None;
    }
    collection
        .ordered_item_ids()
        .map(|item| by_id.get(item).map(|row| (*row).clone()))
        .collect()
}

fn visible_text_complete(text: &OccupationVisibleText) -> bool {
    [
        &text.state_name,
        &text.occupied_country_name,
        &text.resistance_text,
        &text.resistance_speed_text,
        &text.compliance_text,
        &text.compliance_speed_text,
        &text.garrison_manpower_need_text,
        &text.garrison_required_text,
    ]
    .into_iter()
    .all(|value| !value.trim().is_empty())
}

fn parse_prefixed_u32(value: &str, prefix: &str) -> Option<u32> {
    value.strip_prefix(prefix)?.parse().ok()
}

fn measured_f64(value: &Leaf<f64>) -> Option<f64> {
    value.measured().copied()
}
fn measured_i64(value: &Leaf<i64>) -> Option<i64> {
    value.measured().copied()
}
fn measured_u32(value: &Leaf<u32>) -> Option<u32> {
    value.measured().copied()
}
fn nonempty_leaf(value: &Leaf<String>) -> Result<&str, OccupationRejection> {
    value
        .measured()
        .map(String::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn percent(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && (0.0..=100.0).contains(value))
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn finite(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite())
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn nonnegative(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}
fn unit_interval(value: &Leaf<f64>) -> Result<f64, OccupationRejection> {
    measured_f64(value)
        .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
        .ok_or(OccupationRejection::MissingOrInvalidOccupationValue)
}

pub fn fnv1a32(text: &str) -> u32 {
    let mut value = 0x811C_9DC5u32;
    for byte in text.as_bytes() {
        value ^= u32::from(*byte);
        value = value.wrapping_mul(0x0100_0193);
    }
    if value == 0 {
        1
    } else {
        value
    }
}
