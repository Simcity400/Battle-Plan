//! Production selected-POV view-model admission.
//!
//! This is the replay application's consumer boundary for typed late-RE
//! surfaces. It never reads raw replay JSON and never turns a missing static
//! catalog, user selection, or dormant producer into an empty/zero model.

use std::collections::BTreeMap;

use game_ui::active_abilities::{
    admit_active_abilities, load_active_ability_definitions, AbilityDefinition,
    ActiveAbilityPresentation,
};
use game_ui::army_orders_group::{
    bind_army_orders_group_controls, ArmyOrdersGroupControls,
};
use game_ui::character_ui::{AdmittedCharacterUi, CharacterPresentationInput};
use game_ui::combat_detail_losses::{
    admit_xui20_replay, combat_detail_losses_from_frame, Xui20ReplayAdmission,
};
use game_ui::decision_replay::{
    bind_decision_panel, decision_panel_snapshot_from_world, DecisionPanelBinding,
    DecisionStaticCatalog,
};
use game_ui::national_focus::{
    admit_exact_national_focus_presentation, AdmittedNationalFocusPresentation, FocusCatalog,
    FocusTreeTopology,
};
use game_ui::readonly_design_peace::{
    admit_current_selected_designer_identity, admit_peace_conference, AdmittedPeaceConference,
    AdmittedSelectedDesignerIdentity, PeaceConferenceSnapshot,
};
use game_ui::research_canvas::ResearchCanvasRequest;
use game_ui::{assets::AssetResolver, gfx::SpriteRegistry};
use replay_core::agency_upgrade_controller::admit_agency_upgrade_controller_refresh;
use replay_core::cipher_controller::admit_cipher_controller_refresh;
use replay_core::{Leaf, WorldFrame};

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureState<T> {
    pub value: Option<T>,
    pub unavailable_reason: Option<String>,
}

impl<T> FeatureState<T> {
    fn admitted(value: T) -> Self {
        Self {
            value: Some(value),
            unavailable_reason: None,
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgencyControllerSummary {
    pub owner: String,
    pub owner_tag_db_id: u32,
    pub ordered_upgrade_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CipherControllerSummary {
    pub owner: String,
    pub owner_tag_db_id: u32,
    pub selected_target: Option<String>,
    pub ordered_targets: Vec<String>,
}

/// Authenticated inputs that cannot be reconstructed from a replay frame.
/// The product passes them only after its effective VFS/UI09 layers have
/// admitted the same content identity. `None` means feature-local unavailable.
#[derive(Clone, Copy, Default)]
pub struct SelectedPovSupplementalInputs<'a> {
    pub focus: Option<(
        &'a FocusTreeTopology,
        &'a FocusCatalog,
        &'a ResearchCanvasRequest,
    )>,
    pub decisions: Option<&'a DecisionStaticCatalog>,
    pub character: Option<(u32, &'a CharacterPresentationInput)>,
    pub peace: Option<&'a PeaceConferenceSnapshot>,
    pub selected_army_id: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectedPovRuntimeSnapshot {
    pub pov_country: Option<String>,
    pub topbar: FeatureState<game_ui::topbar::TopbarBinding>,
    pub agency_controller: FeatureState<AgencyControllerSummary>,
    pub cipher_controller: FeatureState<CipherControllerSummary>,
    pub national_focus: FeatureState<AdmittedNationalFocusPresentation>,
    pub decision_panel: FeatureState<DecisionPanelBinding>,
    pub active_abilities: FeatureState<Vec<ActiveAbilityPresentation>>,
    pub characters: FeatureState<AdmittedCharacterUi>,
    pub selected_designer: FeatureState<AdmittedSelectedDesignerIdentity>,
    pub peace_conference: FeatureState<AdmittedPeaceConference>,
    pub orders_group: FeatureState<ArmyOrdersGroupControls>,
    pub combat_detail: FeatureState<Xui20ReplayAdmission>,
}

pub struct SelectedPovRuntime {
    ability_definitions: FeatureState<BTreeMap<String, AbilityDefinition>>,
    snapshot: SelectedPovRuntimeSnapshot,
}

impl SelectedPovRuntime {
    pub fn new(resolver: &AssetResolver, registry: &SpriteRegistry) -> Self {
        let ability_definitions = match load_active_ability_definitions(resolver, registry) {
            Ok(definitions) => FeatureState::admitted(definitions),
            Err(error) => FeatureState::unavailable(format!(
                "XUI23 effective ability catalog unavailable: {}",
                error.message
            )),
        };
        Self {
            ability_definitions,
            snapshot: unavailable_snapshot(None, "selected POV has not been evaluated"),
        }
    }

    pub fn snapshot(&self) -> &SelectedPovRuntimeSnapshot {
        &self.snapshot
    }

    pub fn refresh(
        &mut self,
        frame: &WorldFrame,
        pov_country: Option<&str>,
        supplemental: SelectedPovSupplementalInputs<'_>,
    ) {
        let Some(pov) = pov_country else {
            self.snapshot = unavailable_snapshot(None, "world view has no selected country POV");
            self.snapshot.combat_detail = FeatureState::admitted(admit_xui20_replay(
                &combat_detail_losses_from_frame(frame),
            ));
            self.snapshot.selected_designer = selected_designer(frame);
            return;
        };

        let diplomacy = frame.diplomacy();
        let expected_hour = match u64::try_from(frame.hour_index()) {
            Ok(hour) => hour,
            Err(_) => {
                self.snapshot = unavailable_snapshot(
                    Some(pov),
                    "negative replay hour cannot admit exact runtime carriers",
                );
                return;
            }
        };

        let agency_controller = match &diplomacy.agency_upgrade_controller_refresh {
            Leaf::Measured(refresh) if refresh.owner == pov => {
                match admit_agency_upgrade_controller_refresh(refresh, expected_hour) {
                    Ok(admitted) => FeatureState::admitted(AgencyControllerSummary {
                        owner: refresh.owner.clone(),
                        owner_tag_db_id: refresh.owner_tag_db_id,
                        ordered_upgrade_ids: admitted
                            .ordered_upgrades()
                            .into_iter()
                            .map(|(_, row)| row.upgrade_id.clone())
                            .collect(),
                    }),
                    Err(error) => FeatureState::unavailable(error.message),
                }
            }
            Leaf::Measured(_) => FeatureState::unavailable(
                "A30 agency-controller refresh belongs to a different country",
            ),
            Leaf::Absent => FeatureState::unavailable("A30 agency-controller refresh absent"),
            Leaf::Unobtained => {
                FeatureState::unavailable("A30 agency-controller refresh unavailable")
            }
        };
        let cipher_controller = match &diplomacy.cipher_controller_refresh {
            Leaf::Measured(refresh) if refresh.owner == pov => {
                match admit_cipher_controller_refresh(refresh, expected_hour) {
                    Ok(admitted) => FeatureState::admitted(CipherControllerSummary {
                        owner: refresh.owner.clone(),
                        owner_tag_db_id: refresh.owner_tag_db_id,
                        selected_target: refresh.selected_target.clone(),
                        ordered_targets: admitted
                            .ordered_targets()
                            .into_iter()
                            .map(|row| row.target.clone())
                            .collect(),
                    }),
                    Err(error) => FeatureState::unavailable(error.message),
                }
            }
            Leaf::Measured(_) => FeatureState::unavailable(
                "A30 cipher-controller refresh belongs to a different country",
            ),
            Leaf::Absent => FeatureState::unavailable("A30 cipher-controller refresh absent"),
            Leaf::Unobtained => {
                FeatureState::unavailable("A30 cipher-controller refresh unavailable")
            }
        };

        let national_focus = match supplemental.focus {
            Some((topology, catalog, canvas)) => match frame.focus(pov) {
                Some(focus) => admit_exact_national_focus_presentation(
                    focus,
                    frame.national_focus_presentation_exact(),
                    pov,
                    topology,
                    catalog,
                    canvas,
                )
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.reason)),
                None => FeatureState::unavailable("A31 selected-country focus view absent"),
            },
            None => FeatureState::unavailable(
                "A31 effective focus topology/catalog/UI09 canvas not admitted",
            ),
        };

        let decision_panel = match supplemental.decisions {
            Some(catalog) => decision_panel_snapshot_from_world(frame.decision_panel_exact())
                .and_then(|snapshot| {
                    if snapshot.country_tag != pov {
                        Err(game_ui::GameUiError::parse(
                            "A34 decision panel belongs to a different country",
                        ))
                    } else {
                        bind_decision_panel(catalog, &snapshot)
                    }
                })
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.message)),
            None => FeatureState::unavailable(
                "A34 effective decision catalog not admitted for this replay",
            ),
        };

        let active_abilities = match (
            frame.politics_detail(pov),
            self.ability_definitions.value.as_ref(),
        ) {
            (Some(view), Some(definitions)) => admit_active_abilities(view, definitions)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.0)),
            (None, _) => FeatureState::unavailable("A36 selected-country politics view absent"),
            (_, None) => FeatureState::unavailable(
                self.ability_definitions
                    .unavailable_reason
                    .clone()
                    .unwrap_or_else(|| "A36 ability catalog unavailable".into()),
            ),
        };

        let characters = match supplemental.character {
            Some((owner_tag_db_id, remaining)) =>
                crate::character_runtime::admit_country_character_ui_with_exact_a39(
                    frame,
                    pov,
                    owner_tag_db_id,
                    remaining,
                )
                .map(FeatureState::admitted)
                .unwrap_or_else(FeatureState::unavailable),
            None => FeatureState::unavailable(
                "A39 remaining assignment/name/portrait presentation inputs not admitted",
            ),
        };

        let peace_conference = match supplemental.peace {
            Some(snapshot) => admit_peace_conference(snapshot)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(format!("{error:?}"))),
            None => FeatureState::unavailable(
                "A40 replay-core peace transition carrier is not available",
            ),
        };

        self.snapshot = SelectedPovRuntimeSnapshot {
            pov_country: Some(pov.to_string()),
            topbar: FeatureState::admitted(game_ui::binding::topbar_binding_from_frame(frame, pov)),
            agency_controller,
            cipher_controller,
            national_focus,
            decision_panel,
            active_abilities,
            characters,
            selected_designer: selected_designer(frame),
            peace_conference,
            orders_group: FeatureState::admitted(bind_army_orders_group_controls(
                frame,
                supplemental.selected_army_id,
            )),
            combat_detail: FeatureState::admitted(admit_xui20_replay(
                &combat_detail_losses_from_frame(frame),
            )),
        };
    }
}

fn selected_designer(frame: &WorldFrame) -> FeatureState<AdmittedSelectedDesignerIdentity> {
    admit_current_selected_designer_identity(frame)
        .map(FeatureState::admitted)
        .unwrap_or_else(|error| FeatureState::unavailable(format!("{error:?}")))
}

fn unavailable_snapshot(
    pov: Option<&str>,
    reason: &str,
) -> SelectedPovRuntimeSnapshot {
    SelectedPovRuntimeSnapshot {
        pov_country: pov.map(str::to_string),
        topbar: FeatureState::unavailable(reason),
        agency_controller: FeatureState::unavailable(reason),
        cipher_controller: FeatureState::unavailable(reason),
        national_focus: FeatureState::unavailable(reason),
        decision_panel: FeatureState::unavailable(reason),
        active_abilities: FeatureState::unavailable(reason),
        characters: FeatureState::unavailable(reason),
        selected_designer: FeatureState::unavailable(reason),
        peace_conference: FeatureState::unavailable(reason),
        orders_group: FeatureState::unavailable(reason),
        combat_detail: FeatureState::unavailable(reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_selected_pov_is_feature_local_unavailable_not_empty_data() {
        let snapshot = unavailable_snapshot(None, "no selected POV");
        assert_eq!(snapshot.pov_country, None);
        for state in [
            snapshot.topbar.unavailable_reason.as_deref(),
            snapshot.agency_controller.unavailable_reason.as_deref(),
            snapshot.cipher_controller.unavailable_reason.as_deref(),
            snapshot.national_focus.unavailable_reason.as_deref(),
            snapshot.decision_panel.unavailable_reason.as_deref(),
            snapshot.active_abilities.unavailable_reason.as_deref(),
            snapshot.characters.unavailable_reason.as_deref(),
            snapshot.selected_designer.unavailable_reason.as_deref(),
            snapshot.peace_conference.unavailable_reason.as_deref(),
            snapshot.orders_group.unavailable_reason.as_deref(),
            snapshot.combat_detail.unavailable_reason.as_deref(),
        ] {
            assert_eq!(state, Some("no selected POV"));
        }
        assert!(snapshot.topbar.value.is_none());
        assert!(snapshot.combat_detail.value.is_none());
    }

    #[test]
    fn production_ui_runtime_refreshes_the_typed_consumer_before_rendering() {
        let source = include_str!("ui_runtime.rs");
        let refresh = source
            .find("self.selected_pov_runtime.refresh(")
            .expect("selected-POV runtime refresh");
        let selected_present = source
            .find("prepare_selected_present_ui_pass(")
            .expect("selected-Present preparation");
        assert!(refresh < selected_present);
        assert!(source.contains("pub fn selected_pov_runtime("));
    }
}
