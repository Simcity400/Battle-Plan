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
    bind_army_battleplan_panel, bind_army_orders_group_controls, ArmyBattleplanPanel,
    ArmyOrdersGroupControls,
};
use game_ui::character_ui::{AdmittedCharacterUi, CharacterPresentationInput};
use game_ui::combat_detail_losses::{
    admit_xui20_replay, combat_detail_losses_from_frame, Xui20ReplayAdmission,
};
use game_ui::decision_replay::{
    bind_decision_panel, decision_panel_snapshot_from_world, DecisionPanelBinding,
    DecisionStaticCatalog,
};
use game_ui::deployment_queue_state::{deployment_queue_state, AdmittedDeploymentQueue};
use game_ui::event_chrome::{
    admit_event_history, event_definitions_by_id, load_events, AdmittedFiredEvent, EventDef,
    EventHistoryRequest,
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
use game_ui::trade_state::{admit_trade_screen, AdmittedTradeScreen};
use game_ui::{assets::AssetResolver, gfx::SpriteRegistry};
use game_ui::loc::Localisation;
use game_ui::text_law::RuntimeTextContext;
use replay_core::agency_upgrade_controller::admit_agency_upgrade_controller_refresh;
use replay_core::cipher_controller::admit_cipher_controller_refresh;
use replay_core::{
    ArmyBattleplanOrderKey, ArmyBattleplanOwnerKey, ArmyBattleplanOwnerKind,
    FrdDirectTransitionKind, Leaf, RuntimeTextCatalog, ViewStatus, WorldFrame,
};

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

/// Pointer-free X10 history state consumed by the existing selected-POV
/// event/runtime surface. Counts remain in the engine's fixed kind order and
/// the latest row keeps exact sequence/hour identity without cloning the
/// cumulative history on every selected-Present refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectTransitionHistorySummary {
    pub transition_count: usize,
    pub kind_counts: [usize; 12],
    pub last_sequence: Option<u64>,
    pub last_event_hour_raw: Option<i32>,
}

/// Authenticated inputs that cannot be reconstructed from a replay frame.
/// The product passes them only after its effective VFS/UI09 layers have
/// admitted the same content identity. Decisions use the recorder-packaged
/// effective catalog when no test/owner override is supplied, and d87's
/// catalog hash must match it before binding.
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
    pub selected_battleplan_owner: Option<ArmyBattleplanOwnerKey>,
    /// Stable replay identity selected by the product UI. A raw slot index or
    /// engine pointer is never accepted because either can alias after churn.
    pub selected_order_key: Option<&'a ArmyBattleplanOrderKey>,
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
    pub battleplan_panel: FeatureState<ArmyBattleplanPanel>,
    pub combat_detail: FeatureState<Xui20ReplayAdmission>,
    pub fired_events: FeatureState<Vec<AdmittedFiredEvent>>,
    pub direct_transitions: FeatureState<DirectTransitionHistorySummary>,
    /// XUI15 typed classic-resource trade presentation. This is consumed on
    /// every selected-POV refresh; pixel composition remains separately gated
    /// on an authenticated trade-window activation/root binding.
    pub trade: FeatureState<AdmittedTradeScreen>,
    /// Exact X4 ordered deployment hierarchy for the selected country. The
    /// inner enum preserves domain absence, unobtained capture and measured
    /// empty as three different presentation states.
    pub deployment_queue: FeatureState<AdmittedDeploymentQueue>,
}

pub struct SelectedPovRuntime {
    ability_definitions: FeatureState<BTreeMap<String, AbilityDefinition>>,
    decision_catalog: FeatureState<DecisionStaticCatalog>,
    event_definitions: FeatureState<BTreeMap<String, EventDef>>,
    event_localisation: FeatureState<Localisation>,
    snapshot: SelectedPovRuntimeSnapshot,
}

impl SelectedPovRuntime {
    /// Construct the selected-POV runtime with an optional recorder-packaged
    /// effective decision catalog. The catalog bytes must come from the
    /// authenticated replay package/VFS boundary; this crate deliberately has
    /// no compile-time dependency on machine-local `game_defs/out`.
    ///
    /// Passing `None` keeps only XUI19 unavailable. It never falls back to an
    /// installed game tree or a developer checkout artifact.
    pub fn new_with_decision_catalog_bytes(
        resolver: &AssetResolver,
        registry: &SpriteRegistry,
        decision_catalog_bytes: Option<&[u8]>,
    ) -> Self {
        let ability_definitions = match load_active_ability_definitions(resolver, registry) {
            Ok(definitions) => FeatureState::admitted(definitions),
            Err(error) => FeatureState::unavailable(format!(
                "XUI23 effective ability catalog unavailable: {}",
                error.message
            )),
        };
        let decision_catalog = match decision_catalog_bytes {
            Some(bytes) => match DecisionStaticCatalog::from_extractor_json(bytes) {
                Ok(catalog) => FeatureState::admitted(catalog),
                Err(error) => FeatureState::unavailable(format!(
                    "A34 recorder-packaged effective decision catalog rejected: {}",
                    error.message
                )),
            },
            None => FeatureState::unavailable(
                "A34 recorder-packaged effective decision catalog was not supplied",
            ),
        };
        let event_definitions = load_events(resolver)
            .map_err(|error| error.message)
            .and_then(|corpus| {
                event_definitions_by_id(&corpus)
                    .map_err(|error| format!("XUI21 event-definition registry rejected: {error:?}"))
            })
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| FeatureState::unavailable(format!(
                "XUI21 effective event-definition registry unavailable: {error}"
            )));
        let event_localisation = Localisation::load_english(resolver)
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| FeatureState::unavailable(format!(
                "XUI21 effective localisation unavailable: {}",
                error.message
            )));
        Self {
            ability_definitions,
            decision_catalog,
            event_definitions,
            event_localisation,
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
        runtime_text: Option<&RuntimeTextCatalog>,
        sprites: &SpriteRegistry,
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

        let decision_catalog = supplemental
            .decisions
            .or(self.decision_catalog.value.as_ref());
        let decision_panel = match decision_catalog {
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
                self.decision_catalog
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("A34 effective decision catalog not admitted for this replay"),
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

        let fired_events = match (
            self.event_definitions.value.as_ref(),
            self.event_localisation.value.as_ref(),
            runtime_text,
        ) {
            (Some(definitions), Some(localisation), Some(runtime_text))
                if runtime_text.contexts_complete => {
                let contexts = runtime_text
                    .contexts
                    .iter()
                    .map(|(id, row)| (id.clone(), RuntimeTextContext::from(row)))
                    .collect::<BTreeMap<_, _>>();
                admit_event_history(EventHistoryRequest {
                    source_record_id: "world_events.fired_events:d75",
                    history: frame.fired_events(),
                    definitions,
                    localisation,
                    sprites,
                    runtime_text: &contexts,
                })
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(format!(
                    "XUI21 fired-event history unavailable: {error:?}"
                )))
            }
            (None, _, _) => FeatureState::unavailable(
                self.event_definitions.unavailable_reason.as_deref().unwrap_or(
                    "XUI21 effective event-definition registry unavailable",
                ),
            ),
            (_, None, _) => FeatureState::unavailable(
                self.event_localisation
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("XUI21 effective localisation unavailable"),
            ),
            (_, _, Some(_)) => FeatureState::unavailable(
                "XUI21 A25 runtime-text carrier is incomplete",
            ),
            (_, _, None) => FeatureState::unavailable(
                "XUI21 A25 runtime-text carrier is unavailable",
            ),
        };
        let direct_transitions = admit_direct_transition_history(frame);
        let deployment_queue = FeatureState::admitted(deployment_queue_state(frame, pov));

        let battleplan_panel = admit_battleplan_panel(
            bind_army_battleplan_panel(
                frame,
                supplemental.selected_battleplan_owner,
                supplemental.selected_order_key,
            ),
            supplemental.selected_order_key,
        );

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
                supplemental.selected_battleplan_owner.and_then(|owner| {
                    (owner.kind == ArmyBattleplanOwnerKind::Army).then_some(owner.id)
                }),
            )),
            battleplan_panel,
            combat_detail: FeatureState::admitted(admit_xui20_replay(
                &combat_detail_losses_from_frame(frame),
            )),
            fired_events,
            direct_transitions,
            trade: FeatureState::admitted(admit_trade_screen(
                frame.trade_status(),
                pov,
                frame.trade(pov),
            )),
            deployment_queue,
        };
    }
}

fn admit_direct_transition_history(
    frame: &WorldFrame,
) -> FeatureState<DirectTransitionHistorySummary> {
    admit_direct_transition_history_view(frame.frd_direct_transition_history())
}

fn admit_direct_transition_history_view(
    history: &replay_core::FrdDirectTransitionHistoryView,
) -> FeatureState<DirectTransitionHistorySummary> {
    if history.status != ViewStatus::Present {
        return FeatureState::unavailable(format!(
            "X10 direct-transition history is not present: {:?}",
            history.status
        ));
    }
    let Some(authority) = history.authority.as_ref() else {
        return FeatureState::unavailable(
            "X10 direct-transition history omitted its typed authority",
        );
    };
    if authority.evidence_fixture {
        return FeatureState::unavailable(
            "X10 evidence-fixture history is not admitted to product runtime",
        );
    }
    let mut kind_counts = [0usize; 12];
    for row in &history.transitions {
        let index = match row.kind {
            FrdDirectTransitionKind::FocusCompletion => 0,
            FrdDirectTransitionKind::ResearchThresholdCrossing => 1,
            FrdDirectTransitionKind::OrdinaryDecisionTaken => 2,
            FrdDirectTransitionKind::TargetedDecisionTaken => 3,
            FrdDirectTransitionKind::TimedDecisionActivated => 4,
            FrdDirectTransitionKind::OnActionInvoked => 5,
            FrdDirectTransitionKind::DecisionCancelEffectInvoked => 6,
            FrdDirectTransitionKind::DecisionRemoveEffectInvoked => 7,
            FrdDirectTransitionKind::DecisionTimeoutEffectInvoked => 8,
            FrdDirectTransitionKind::TargetedDecisionCancelEffectInvoked => 9,
            FrdDirectTransitionKind::TargetedDecisionRemoveEffectInvoked => 10,
            FrdDirectTransitionKind::TargetedDecisionTimeoutEffectInvoked => 11,
        };
        kind_counts[index] = kind_counts[index].saturating_add(1);
    }
    FeatureState::admitted(DirectTransitionHistorySummary {
        transition_count: history.transitions.len(),
        kind_counts,
        last_sequence: history.transitions.last().map(|row| row.sequence),
        last_event_hour_raw: history.transitions.last().map(|row| row.event_hour_raw),
    })
}

fn selected_designer(frame: &WorldFrame) -> FeatureState<AdmittedSelectedDesignerIdentity> {
    admit_current_selected_designer_identity(frame)
        .map(FeatureState::admitted)
        .unwrap_or_else(|error| FeatureState::unavailable(format!("{error:?}")))
}

fn admit_battleplan_panel(
    panel: ArmyBattleplanPanel,
    selected_order_key: Option<&ArmyBattleplanOrderKey>,
) -> FeatureState<ArmyBattleplanPanel> {
    if panel.status != ViewStatus::Present {
        return FeatureState::unavailable(format!(
            "d54 exact battleplan carrier is not present: {:?}",
            panel.status
        ));
    }
    let Some(selected_owner) = panel.selected_owner else {
        return FeatureState::unavailable("no exact battleplan owner selection was supplied");
    };
    if panel.selected_owner_present != Some(true) {
        return FeatureState::unavailable(format!(
            "selected battleplan owner {:?}:{} is not present in the complete d54 manager census",
            selected_owner.kind, selected_owner.id
        ));
    }
    if let Some(key) = selected_order_key {
        if key.owner != selected_owner {
            return FeatureState::unavailable(
                "selected order identity belongs to a different selected battleplan owner",
            );
        }
        if panel.selected_order.is_none() {
            return FeatureState::unavailable(
                "selected order stable identity is absent from the exact d54 order census",
            );
        }
    }
    FeatureState::admitted(panel)
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
        battleplan_panel: FeatureState::unavailable(reason),
        combat_detail: FeatureState::unavailable(reason),
        fired_events: FeatureState::unavailable(reason),
        direct_transitions: FeatureState::unavailable(reason),
        trade: FeatureState::unavailable(reason),
        deployment_queue: FeatureState::unavailable(reason),
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
            snapshot.battleplan_panel.unavailable_reason.as_deref(),
            snapshot.combat_detail.unavailable_reason.as_deref(),
            snapshot.fired_events.unavailable_reason.as_deref(),
            snapshot.direct_transitions.unavailable_reason.as_deref(),
            snapshot.trade.unavailable_reason.as_deref(),
            snapshot.deployment_queue.unavailable_reason.as_deref(),
        ] {
            assert_eq!(state, Some("no selected POV"));
        }
        assert!(snapshot.topbar.value.is_none());
        assert!(snapshot.combat_detail.value.is_none());
        assert!(snapshot.trade.value.is_none());
        assert!(snapshot.deployment_queue.value.is_none());
    }

    #[test]
    fn battleplan_admission_rejects_missing_or_stale_exact_selection() {
        let no_army = ArmyBattleplanPanel {
            status: ViewStatus::Present,
            selected_owner: None,
            selected_owner_present: None,
            orders: Vec::new(),
            selected_order: None,
        };
        assert_eq!(
            admit_battleplan_panel(no_army, None)
                .unavailable_reason
                .as_deref(),
            Some("no exact battleplan owner selection was supplied")
        );

        let owner = ArmyBattleplanOwnerKey {
            kind: ArmyBattleplanOwnerKind::Army,
            id: 17,
        };
        let key = ArmyBattleplanOrderKey {
            owner,
            engine_order_instance_id: 2025,
            generation: 4,
        };
        let stale_order = ArmyBattleplanPanel {
            status: ViewStatus::Present,
            selected_owner: Some(owner),
            selected_owner_present: Some(true),
            orders: Vec::new(),
            selected_order: None,
        };
        assert_eq!(
            admit_battleplan_panel(stale_order, Some(&key))
                .unavailable_reason
                .as_deref(),
            Some("selected order stable identity is absent from the exact d54 order census")
        );
    }

    fn x10_history(evidence_fixture: bool) -> replay_core::FrdDirectTransitionHistoryView {
        let row = replay_core::FrdDirectTransitionRow {
            sequence: 9,
            kind: replay_core::FrdDirectTransitionKind::OnActionInvoked,
            phase: replay_core::FrdDirectTransitionPhase::Invocation,
            thread_id: 7,
            event_hour_raw: 60_787_539,
            hook_rva: 0x00a6_aae0,
            owner_instance_id: 1,
            subject_instance_id: 2,
            argument_instance_id: 0,
            owner_key: 0,
            subject_token_hash_fnv1a64: 3,
            subject_key: 0,
            input_bits: 0,
            return_bits: 0,
            before: replay_core::FrdDirectTransitionSummary {
                state_bits: 0,
                progress_raw: 0,
                remaining_raw: 0,
            },
            after: replay_core::FrdDirectTransitionSummary {
                state_bits: 2,
                progress_raw: 0,
                remaining_raw: 0,
            },
            original_returned: false,
        };
        replay_core::FrdDirectTransitionHistoryView {
            status: ViewStatus::Present,
            authority: Some(replay_core::FrdDirectTransitionAuthority {
                schema_version: 3,
                authority: "exact_a729_d45e_hook_epoch".into(),
                executable_sha256: "test".into(),
                legacy_root_manifest_sha256: "test".into(),
                on_action_root_manifest_sha256: "test".into(),
                terminal_root_manifest_sha256: "test".into(),
                cluster_exhaustion_manifest_sha256: "test".into(),
                hook_epoch: 1,
                last_drain_sequence: 1,
                installed_root_mask: (1 << 12) - 1,
                observed_kind_mask: 1 << 5,
                producer_complete: true,
                evidence_fixture,
                transitions: vec![row.clone()],
            }),
            transitions: vec![row],
        }
    }

    #[test]
    fn x10_history_feeds_runtime_summary_and_rejects_evidence_mode() {
        let admitted = admit_direct_transition_history_view(&x10_history(false));
        let summary = admitted.value.expect("production X10 summary");
        assert_eq!(summary.transition_count, 1);
        assert_eq!(summary.kind_counts[5], 1);
        assert_eq!(summary.last_sequence, Some(9));
        assert_eq!(summary.last_event_hour_raw, Some(60_787_539));

        let evidence = admit_direct_transition_history_view(&x10_history(true));
        assert!(evidence.value.is_none());
        assert_eq!(
            evidence.unavailable_reason.as_deref(),
            Some("X10 evidence-fixture history is not admitted to product runtime")
        );
    }

    #[test]
    fn production_ui_runtime_refreshes_the_typed_consumer_before_rendering() {
        let product_source = include_str!("ui_runtime.rs");
        let refresh = product_source
            .find("self.selected_pov_runtime.refresh(")
            .expect("selected-POV runtime refresh");
        let selected_present = product_source
            .find("prepare_selected_present_ui_pass(")
            .expect("selected-Present preparation");
        let decision_model = product_source
            .find(".decision_panel\n            .value")
            .expect("A34 model consumption");
        let selected_pov_source = include_str!("selected_pov_runtime.rs");
        let selected_pov_impl = selected_pov_source
            .split_once("#[cfg(test)]")
            .expect("selected-POV production implementation")
            .0;
        let battleplan_binding = selected_pov_impl
            .find("let battleplan_panel = admit_battleplan_panel(\n            bind_army_battleplan_panel(")
            .expect("d54 production battleplan binding");
        let deployment_binding = selected_pov_impl
            .find("let deployment_queue = FeatureState::admitted(deployment_queue_state(frame, pov));")
            .expect("X4 production deployment-queue binding");
        let snapshot_publish = selected_pov_impl
            .find("self.snapshot = SelectedPovRuntimeSnapshot {")
            .expect("selected-POV typed snapshot publication");
        assert!(refresh < selected_present);
        assert!(battleplan_binding < snapshot_publish);
        assert!(deployment_binding < snapshot_publish);
        assert!(refresh < decision_model);
        assert!(decision_model < selected_present);
        assert!(!product_source.contains("pub fn selected_pov_runtime("));
    }
}
