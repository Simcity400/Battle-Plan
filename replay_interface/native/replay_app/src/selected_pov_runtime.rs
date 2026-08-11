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
use game_ui::character_ui::{
    admit_exact_cabinet, AdmittedCabinetUi, CharacterPortraitCatalog, CharacterViewerSelection,
    LeaderSkillCostCatalog, ObserverCharacterUi, UnitLeaderTraitCatalog,
};
use game_ui::combat_detail_losses::{
    admit_xui20_replay, combat_detail_losses_from_frame, Xui20FrameHour, Xui20ReplayAdmission,
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
use game_ui::loc::Localisation;
use game_ui::national_focus::{
    admit_observer_national_focus_presentation, AdmittedNationalFocusPresentation, FocusCatalog,
    FocusTreeTopology,
};
use game_ui::readonly_design_peace::{
    admit_current_selected_designer_identity, admit_peace_conference,
    peace_conference_snapshot_from_world, AdmittedPeaceConference,
    AdmittedSelectedDesignerIdentity, PeaceConferenceSnapshot,
};
use game_ui::research_canvas::TechnologyCanvasRequest;
use game_ui::research_observer::{observer_research_from_frame, ObserverResearchModel};
use game_ui::static_research::{
    PoliticsIdeaCatalogLabels, StaticResearchUiCatalog, TechnologySharingUiView,
};
use game_ui::strategic_air_details::{bind_strategic_air_details, StrategicAirDetailsPanel};
use game_ui::supply_hub_motorization::bind_supply_hub_motorization_rows;
use game_ui::text_law::RuntimeTextContext;
use game_ui::trade_state::{admit_trade_screen, AdmittedTradeScreen};
use game_ui::{assets::AssetResolver, gfx::SpriteRegistry};
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
    pub focus: Option<(&'a FocusTreeTopology, &'a FocusCatalog)>,
    /// Recorder-packaged semantic research catalogs. This supplies authored
    /// technology labels and the exact D69 runtime-definition join; it never
    /// falls back to the playback machine's installed content.
    pub static_research: Option<&'a StaticResearchUiCatalog>,
    pub decisions: Option<&'a DecisionStaticCatalog>,
    pub character: Option<&'a CharacterViewerSelection>,
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
    /// Authenticated D92 selection plus the exact D96 fixed-point multiplier
    /// when a tactic is selected. Its pixels remain owned by the matching
    /// authenticated selected-Present pass.
    pub preferred_tactic: FeatureState<game_ui::preferred_tactic::PreferredTacticUi>,
    pub agency_controller: FeatureState<AgencyControllerSummary>,
    pub cipher_controller: FeatureState<CipherControllerSummary>,
    pub national_focus: FeatureState<AdmittedNationalFocusPresentation>,
    /// Retired TCV1 compatibility slot; product XUI09 is observer-composed.
    pub technology_canvas: FeatureState<TechnologyCanvasRequest>,
    pub research: FeatureState<ObserverResearchModel>,
    /// Selected-country politics idea families joined to the authenticated
    /// recorder-packaged idea catalog.
    pub politics_ideas: FeatureState<PoliticsIdeaCatalogLabels>,
    /// Global D69 membership/bonus state joined to the authenticated
    /// technology-sharing definitions.
    pub technology_sharing: FeatureState<TechnologySharingUiView>,
    pub decision_panel: FeatureState<DecisionPanelBinding>,
    pub active_abilities: FeatureState<Vec<ActiveAbilityPresentation>>,
    pub cabinet: FeatureState<AdmittedCabinetUi>,
    pub characters: FeatureState<ObserverCharacterUi>,
    pub selected_designer: FeatureState<AdmittedSelectedDesignerIdentity>,
    pub peace_conference: FeatureState<AdmittedPeaceConference>,
    pub orders_group: FeatureState<ArmyOrdersGroupControls>,
    pub battleplan_panel: FeatureState<ArmyBattleplanPanel>,
    pub combat_detail: FeatureState<Xui20ReplayAdmission>,
    /// D61 exact 24-channel strategic-air details for the selected country.
    pub strategic_air_details: FeatureState<StrategicAirDetailsPanel>,
    /// D62 persisted supply-hub motorization settings involving the selected
    /// country. Measured empty is distinct from unavailable.
    pub supply_hub_motorization: FeatureState<Vec<replay_core::HubMotorizationRow>>,
    pub fired_events: FeatureState<Vec<AdmittedFiredEvent>>,
    pub direct_transitions: FeatureState<DirectTransitionHistorySummary>,
    /// Legacy selected-POV XUI15 semantic companion. Shipping XUI15 uses the
    /// passive all-country carrier plus viewer-owned panel state; this field
    /// does not gate product pixels or activation.
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
    character_portraits: FeatureState<CharacterPortraitCatalog>,
    character_traits: FeatureState<UnitLeaderTraitCatalog>,
    character_skill_costs: FeatureState<LeaderSkillCostCatalog>,
    snapshot: SelectedPovRuntimeSnapshot,
}

impl SelectedPovRuntime {
    /// Construct the selected-POV runtime with an optional recorder-packaged
    /// effective decision catalog. The catalog bytes must come from the
    /// authenticated replay package/VFS boundary; this crate deliberately has
    /// no compile-time dependency on machine-local `game_defs/out`.
    ///
    /// Passing `None` makes XUI19's authored catalogue feature-locally
    /// unavailable. It never falls back to an installed game tree or a
    /// developer checkout artifact.
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
                // `load_events` RECORDS byte-level damage and skips the file, so
                // a damaged mod override can simply vanish from the corpus and
                // leave an older definition uniquely admissible. Admitting a
                // registry built from a knowingly partial corpus would present
                // that as the effective content.
                if !corpus.decode_lossy_files.is_empty() {
                    return Err(format!(
                        "XUI21 event corpus decode destroyed bytes in {:?}",
                        corpus.decode_lossy_files
                    ));
                }
                event_definitions_by_id(&corpus)
                    .map_err(|error| format!("XUI21 event-definition registry rejected: {error:?}"))
            })
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| {
                FeatureState::unavailable(format!(
                    "XUI21 effective event-definition registry unavailable: {error}"
                ))
            });
        let event_localisation = Localisation::load_english(resolver)
            .map_err(|error| error.message)
            .and_then(|loc| {
                // `load_english` RECORDS byte-level decode damage; admitting it
                // unconditionally would throw that record away and present
                // mojibake as an admitted string. Line-level damage
                // (`parse_failures`) is deliberately still tolerated here — it
                // is the pre-existing behaviour and skipping a damaged line is
                // the right rendering choice — but destroyed BYTES mean the
                // strings are not the file's strings.
                if loc.decode_lossy_files.is_empty() {
                    Ok(loc)
                } else {
                    Err(format!(
                        "localisation decode destroyed bytes in {:?}",
                        loc.decode_lossy_files
                    ))
                }
            })
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| {
                FeatureState::unavailable(format!(
                    "XUI21 effective localisation unavailable: {error}"
                ))
            });
        // Prewarm the effective mod-aware character portrait declarations once
        // at product startup. Per-frame character admission then joins d47's
        // exact character/leader identities to this immutable catalog and the
        // already loaded physical sprite registry.
        let character_portraits = CharacterPortraitCatalog::load(resolver)
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| {
                FeatureState::unavailable(format!(
                    "XUI17 effective character portrait catalog unavailable: {}",
                    error.message
                ))
            });
        let character_traits = UnitLeaderTraitCatalog::load(resolver)
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| {
                FeatureState::unavailable(format!(
                    "XUI17 effective unit-leader trait catalog unavailable: {}",
                    error.message
                ))
            });
        let character_skill_costs = LeaderSkillCostCatalog::load(resolver)
            .map(FeatureState::admitted)
            .unwrap_or_else(|error| {
                FeatureState::unavailable(format!(
                    "XUI17 effective unit-leader skill-cost catalog unavailable: {}",
                    error.message
                ))
            });
        Self {
            ability_definitions,
            decision_catalog,
            event_definitions,
            event_localisation,
            character_portraits,
            character_traits,
            character_skill_costs,
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
        let technology_sharing = match supplemental.static_research {
            Some(catalog) => catalog
                .bind_technology_sharing(frame)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.message)),
            None => FeatureState::unavailable(
                "recorder-packaged static research catalog was not admitted",
            ),
        };
        let Some(pov) = pov_country else {
            self.snapshot = unavailable_snapshot(None, "world view has no selected country POV");
            self.snapshot.technology_sharing = technology_sharing;
            self.snapshot.combat_detail = FeatureState::admitted(admit_xui20_replay(
                &combat_detail_losses_from_frame(frame),
                &Xui20FrameHour::from_frame(frame),
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
        let preferred_tactic = admit_preferred_tactic(frame, pov);

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
            Some((topology, catalog)) => match frame.focus(pov) {
                Some(focus) => admit_observer_national_focus_presentation(focus, topology, catalog)
                    .map(FeatureState::admitted)
                    .unwrap_or_else(|error| FeatureState::unavailable(error.reason)),
                None => FeatureState::unavailable("A31 selected-country focus view absent"),
            },
            None => {
                FeatureState::unavailable("XUI10 effective focus topology/catalog not admitted")
            }
        };

        let technology_canvas = FeatureState::unavailable(
            "XUI09 TCV1 selected-Present canvas is retired from product replay",
        );
        let research = match (supplemental.static_research, frame.research(pov)) {
            (Some(catalog), Some(_)) => observer_research_from_frame(frame, pov, catalog)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.message)),
            (Some(_), None) => {
                FeatureState::unavailable("D3 selected-country research view is absent")
            }
            (None, _) => FeatureState::unavailable(
                "recorder-packaged static research catalog was not admitted",
            ),
        };
        let politics_ideas = match (supplemental.static_research, frame.politics_detail(pov)) {
            (Some(catalog), Some(view)) => catalog
                .bind_politics_ideas(view)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.message)),
            (Some(_), None) => {
                FeatureState::unavailable("selected-country politics detail view is absent")
            }
            (None, _) => FeatureState::unavailable(
                "recorder-packaged static research catalog was not admitted",
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

        let default_character_selection = CharacterViewerSelection::default();
        let characters = match (
            self.character_portraits.value.as_ref(),
            self.character_traits.value.as_ref(),
            self.character_skill_costs.value.as_ref(),
        ) {
            (Some(portraits), Some(traits), Some(skill_costs)) => {
                crate::character_runtime::admit_country_observer_character_ui(
                    frame,
                    pov,
                    portraits,
                    traits,
                    skill_costs,
                    sprites,
                    supplemental
                        .character
                        .unwrap_or(&default_character_selection),
                )
                .map(FeatureState::admitted)
                .unwrap_or_else(FeatureState::unavailable)
            }
            _ => FeatureState::unavailable(
                self.character_portraits
                    .unavailable_reason
                    .as_deref()
                    .or(self.character_traits.unavailable_reason.as_deref())
                    .or(self.character_skill_costs.unavailable_reason.as_deref())
                    .unwrap_or("XUI17 effective character/trait/skill-cost catalog unavailable"),
            ),
        };
        let cabinet = match (
            frame.character_roster_exact(pov),
            self.character_portraits.value.as_ref(),
        ) {
            (Some(roster), Some(portraits)) => admit_exact_cabinet(roster, portraits, sprites)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(error.message)),
            (None, _) => {
                FeatureState::unavailable("D47 selected-country character/advisor roster absent")
            }
            (_, None) => FeatureState::unavailable(
                self.character_portraits
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("XUI17 effective character portrait catalog unavailable"),
            ),
        };

        let peace_conference = match supplemental.peace {
            Some(snapshot) => admit_peace_conference(snapshot)
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| FeatureState::unavailable(format!("{error:?}"))),
            None => peace_conference_snapshot_from_world(frame)
                .and_then(|snapshot| admit_peace_conference(&snapshot))
                .map(FeatureState::admitted)
                .unwrap_or_else(|error| {
                    FeatureState::unavailable(format!(
                        "A40 recorder-owned peace snapshot unavailable: {error:?}"
                    ))
                }),
        };

        let fired_events = match (
            self.event_definitions.value.as_ref(),
            self.event_localisation.value.as_ref(),
            runtime_text,
        ) {
            (Some(definitions), Some(localisation), Some(runtime_text))
                if runtime_text.contexts_complete =>
            {
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
                .unwrap_or_else(|error| {
                    FeatureState::unavailable(format!(
                        "XUI21 fired-event history unavailable: {error:?}"
                    ))
                })
            }
            (None, _, _) => FeatureState::unavailable(
                self.event_definitions
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("XUI21 effective event-definition registry unavailable"),
            ),
            (_, None, _) => FeatureState::unavailable(
                self.event_localisation
                    .unavailable_reason
                    .as_deref()
                    .unwrap_or("XUI21 effective localisation unavailable"),
            ),
            (_, _, Some(_)) => {
                FeatureState::unavailable("XUI21 A25 runtime-text carrier is incomplete")
            }
            (_, _, None) => {
                FeatureState::unavailable("XUI21 A25 runtime-text carrier is unavailable")
            }
        };
        let direct_transitions = admit_direct_transition_history(frame);
        let deployment_queue = FeatureState::admitted(deployment_queue_state(frame, pov));
        let strategic_air_details = match bind_strategic_air_details(frame, pov) {
            Leaf::Measured(value) => FeatureState::admitted(value),
            Leaf::Absent => FeatureState::unavailable("D61 air-loss statistics domain absent"),
            Leaf::Unobtained => {
                FeatureState::unavailable("D61 selected-country air-loss statistics unavailable")
            }
        };
        let supply_hub_motorization = match bind_supply_hub_motorization_rows(frame, pov) {
            Leaf::Measured(value) => FeatureState::admitted(value),
            Leaf::Absent => FeatureState::unavailable("D62 hub-motorization domain absent"),
            Leaf::Unobtained => {
                FeatureState::unavailable("D62 selected-country hub motorization unavailable")
            }
        };

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
            preferred_tactic,
            agency_controller,
            cipher_controller,
            national_focus,
            technology_canvas,
            research,
            politics_ideas,
            technology_sharing,
            decision_panel,
            active_abilities,
            cabinet,
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
                &Xui20FrameHour::from_frame(frame),
            )),
            strategic_air_details,
            supply_hub_motorization,
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

fn admit_preferred_tactic(
    frame: &WorldFrame,
    pov: &str,
) -> FeatureState<game_ui::preferred_tactic::PreferredTacticUi> {
    if frame.country_identities_status() != ViewStatus::Present {
        return FeatureState::unavailable(
            "D92/D96 country preferred-tactic identity view is not present",
        );
    }
    let Some(identity) = frame.country_identity(pov) else {
        return FeatureState::unavailable(
            "D92/D96 selected country is absent from the complete identity view",
        );
    };
    match game_ui::preferred_tactic::preferred_tactic_ui(identity) {
        Leaf::Measured(model) => FeatureState::admitted(model),
        Leaf::Absent => FeatureState::unavailable(
            "D92 selection or its required D96 preferred-weight carrier is absent",
        ),
        Leaf::Unobtained => FeatureState::unavailable(
            "D92/D96 preferred-tactic selection or weight failed exact admission",
        ),
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

fn unavailable_snapshot(pov: Option<&str>, reason: &str) -> SelectedPovRuntimeSnapshot {
    SelectedPovRuntimeSnapshot {
        pov_country: pov.map(str::to_string),
        topbar: FeatureState::unavailable(reason),
        preferred_tactic: FeatureState::unavailable(reason),
        agency_controller: FeatureState::unavailable(reason),
        cipher_controller: FeatureState::unavailable(reason),
        national_focus: FeatureState::unavailable(reason),
        technology_canvas: FeatureState::unavailable(reason),
        research: FeatureState::unavailable(reason),
        politics_ideas: FeatureState::unavailable(reason),
        technology_sharing: FeatureState::unavailable(reason),
        decision_panel: FeatureState::unavailable(reason),
        active_abilities: FeatureState::unavailable(reason),
        cabinet: FeatureState::unavailable(reason),
        characters: FeatureState::unavailable(reason),
        selected_designer: FeatureState::unavailable(reason),
        peace_conference: FeatureState::unavailable(reason),
        orders_group: FeatureState::unavailable(reason),
        battleplan_panel: FeatureState::unavailable(reason),
        combat_detail: FeatureState::unavailable(reason),
        strategic_air_details: FeatureState::unavailable(reason),
        supply_hub_motorization: FeatureState::unavailable(reason),
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
            snapshot.preferred_tactic.unavailable_reason.as_deref(),
            snapshot.agency_controller.unavailable_reason.as_deref(),
            snapshot.cipher_controller.unavailable_reason.as_deref(),
            snapshot.national_focus.unavailable_reason.as_deref(),
            snapshot.technology_canvas.unavailable_reason.as_deref(),
            snapshot.research.unavailable_reason.as_deref(),
            snapshot.politics_ideas.unavailable_reason.as_deref(),
            snapshot.technology_sharing.unavailable_reason.as_deref(),
            snapshot.decision_panel.unavailable_reason.as_deref(),
            snapshot.active_abilities.unavailable_reason.as_deref(),
            snapshot.cabinet.unavailable_reason.as_deref(),
            snapshot.characters.unavailable_reason.as_deref(),
            snapshot.selected_designer.unavailable_reason.as_deref(),
            snapshot.peace_conference.unavailable_reason.as_deref(),
            snapshot.orders_group.unavailable_reason.as_deref(),
            snapshot.battleplan_panel.unavailable_reason.as_deref(),
            snapshot.combat_detail.unavailable_reason.as_deref(),
            snapshot.strategic_air_details.unavailable_reason.as_deref(),
            snapshot
                .supply_hub_motorization
                .unavailable_reason
                .as_deref(),
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
                delta_base_sequence: None,
                cumulative_count: None,
                // A v3 whole-history fixture: neither §7.3.1 field applies, and
                // both spell "this frame declares nothing about the raw
                // sequence space", which is what the delta path reads as an
                // ordinary continuation.
                direct_history_reset: false,
                direct_history_base_sequence: None,
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
        let compact_source = |source: &str| {
            source
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>()
        };
        let compact_product_source = compact_source(product_source);
        let refresh = compact_product_source
            .find("self.selected_pov_runtime.refresh(")
            .expect("selected-POV runtime refresh");
        let selected_present = compact_product_source
            .find("prepare_selected_present_ui_pass(")
            .expect("selected-Present preparation");
        let decision_model = compact_product_source
            .find(".decision_panel.value.as_ref()")
            .expect("A34 model consumption");
        let selected_pov_source = include_str!("selected_pov_runtime.rs");
        let selected_pov_impl = selected_pov_source
            .split_once("#[cfg(test)]")
            .expect("selected-POV production implementation")
            .0;
        let compact_selected_pov_impl = compact_source(selected_pov_impl);
        let battleplan_binding = compact_selected_pov_impl
            .find("letbattleplan_panel=admit_battleplan_panel(bind_army_battleplan_panel(")
            .expect("d54 production battleplan binding");
        let deployment_binding = compact_selected_pov_impl
            .find("letdeployment_queue=FeatureState::admitted(deployment_queue_state(frame,pov));")
            .expect("X4 production deployment-queue binding");
        let preferred_tactic_binding = compact_selected_pov_impl
            .find("letpreferred_tactic=admit_preferred_tactic(frame,pov);")
            .expect("D92/D96 production preferred-tactic binding");
        let cabinet_binding = compact_selected_pov_impl
            .find("letcabinet=match(")
            .expect("D47 production cabinet binding");
        let air_details_binding = compact_selected_pov_impl
            .find("letstrategic_air_details=matchbind_strategic_air_details(frame,pov)")
            .expect("D61 production strategic-air-details binding");
        let hub_motorization_binding = compact_selected_pov_impl
            .find("letsupply_hub_motorization=matchbind_supply_hub_motorization_rows(frame,pov)")
            .expect("D62 production hub-motorization binding");
        let snapshot_publish = compact_selected_pov_impl
            .find("self.snapshot=SelectedPovRuntimeSnapshot{")
            .expect("selected-POV typed snapshot publication");
        assert!(refresh < selected_present);
        assert!(battleplan_binding < snapshot_publish);
        assert!(deployment_binding < snapshot_publish);
        assert!(preferred_tactic_binding < snapshot_publish);
        assert!(cabinet_binding < snapshot_publish);
        assert!(air_details_binding < snapshot_publish);
        assert!(hub_motorization_binding < snapshot_publish);
        assert!(refresh < decision_model);
        assert!(decision_model < selected_present);
        assert!(!product_source.contains("pub fn selected_pov_runtime("));
    }
}
